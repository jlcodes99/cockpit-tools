// Shared transport policy for native HTTP and WebSocket requests.
const NATIVE_HTTP_CLIENT_CACHE_LIMIT: usize = 128;

struct NativeHttpClientEntry {
    account_id: String,
    client: Client,
    identity: String,
    used: u64,
}

struct NativeHttpClientCache {
    entries: HashMap<String, NativeHttpClientEntry>,
    clock: u64,
    capacity: usize,
}

impl Default for NativeHttpClientCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            clock: 0,
            capacity: NATIVE_HTTP_CLIENT_CACHE_LIMIT,
        }
    }
}

impl NativeHttpClientCache {
    fn get(&mut self, key: &str) -> Option<(Client, String)> {
        let entry = self.entries.get_mut(key)?;
        self.clock = self.clock.saturating_add(1);
        entry.used = self.clock;
        Some((entry.client.clone(), entry.identity.clone()))
    }

    fn insert(&mut self, key: String, account_id: String, client: Client) -> String {
        self.clock = self.clock.saturating_add(1);
        let identity = format!("{key}:{}", uuid::Uuid::new_v4());
        self.entries.insert(
            key,
            NativeHttpClientEntry {
                account_id,
                client,
                identity: identity.clone(),
                used: self.clock,
            },
        );
        while self.entries.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                break;
            };
            // Drop only this idle-cache reference. Requests/streams retain their
            // own reqwest Client clones; eviction never aborts an active turn.
            self.entries.remove(&oldest);
        }
        identity
    }

    fn invalidate(&mut self, identity: &str) -> bool {
        let Some((key, _)) = identity.rsplit_once(':') else {
            return false;
        };
        if self
            .entries
            .get(key)
            .is_some_and(|entry| entry.identity == identity)
        {
            self.entries.remove(key);
            return true;
        }
        // A late failure from a retired pool must not remove its replacement.
        false
    }

    fn retain_accounts(&mut self, keep: impl Fn(&str) -> bool) {
        self.entries.retain(|_, entry| keep(&entry.account_id));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

struct NativeTransportContext<'a> {
    owner: &'a str,
    profile: &'a crate::modules::codex_api_identity::InstallationProfile,
    base_url: &'a str,
    wire_api: native_contract::WireApi,
    http1_only: bool,
    headers: &'a axum::http::HeaderMap,
    compact_endpoint: Option<&'a native_contract::EndpointOverride>,
    websocket_endpoint: Option<&'a native_contract::EndpointOverride>,
}

fn native_transport_endpoint(raw: &str) -> String {
    Url::parse(raw)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| raw.to_owned())
        .trim_end_matches('/')
        .to_owned()
}

fn native_transport_header_digest(headers: &axum::http::HeaderMap, agent_identity: bool) -> String {
    let mut values: Vec<_> = headers
        .iter()
        .filter(|(name, value)| {
            // Agent assertions are signed afresh. The stable task/key material is
            // included separately, so timestamp-only signatures do not churn pools.
            !(agent_identity
                && name.as_str() == "authorization"
                && value
                    .to_str()
                    .is_ok_and(|value| value.starts_with("AgentAssertion ")))
        })
        .map(|(name, value)| (name.as_str(), value.as_bytes()))
        .collect();
    values.sort_unstable();
    format!("{:x}", Sha256::digest(json!(values).to_string().as_bytes()))
}

fn native_transport_fingerprint(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    context: &NativeTransportContext<'_>,
    proxy_source: &str,
    account_proxy: Option<&str>,
    proxy: Option<&str>,
    no_proxy: &str,
) -> String {
    let agent = account.is_agent_identity_auth();
    let endpoint = |endpoint: Option<&native_contract::EndpointOverride>| {
        endpoint.map(|endpoint| {
            json!({"base":native_transport_endpoint(&endpoint.base_url),
            "headers":native_transport_header_digest(&endpoint.headers,agent)})
        })
    };
    let agent_material = account.agent_identity.as_ref().map(|identity| {
        json!([
            identity.agent_runtime_id,
            identity.task_id,
            identity.account_id,
            identity.chatgpt_user_id,
            format!(
                "{:x}",
                Sha256::digest(identity.agent_private_key.as_bytes())
            )
        ])
    });
    let material = json!({
        "version":4,"account":account.id,"owner":context.owner,
        "profile":[context.profile.version,context.profile.scope,context.profile.installation_id,context.profile.revision],
        "base":native_transport_endpoint(context.base_url),
        "protocol":match context.wire_api { native_contract::WireApi::Responses=>"responses",native_contract::WireApi::Chat=>"chat" },
        "codex_responses_alpn":native_codex_responses_alpn(account, context),
        "http1_only":context.http1_only,
        "compact":endpoint(context.compact_endpoint),"websocket":endpoint(context.websocket_endpoint),
        "websocket_enabled":account.api_supports_websockets,
        "credentials":[account.token_generation,native_transport_header_digest(context.headers,agent),agent_material],
        "proxy":[proxy_source,account.egress_proxy_disabled,account_proxy,proxy,no_proxy],
        "connect_timeout_ms":collection.timeouts.legacy_upstream_connect_timeout_ms.clamp(1000,120_000),
        "pool":"max_idle_per_host=8;redirect=none;cookies=off;no_proxy=environment_only;codex_responses_tls=rustls_native_roots_alpn"
    });
    format!("{:x}", Sha256::digest(material.to_string().as_bytes()))
}

fn native_inherited_proxy_bypass(source: &str) -> String {
    if source == "environment" {
        // Capture once, with reqwest's uppercase-before-lowercase precedence,
        // so the cache identity and the actual client use the same policy.
        std::env::var("NO_PROXY")
            .or_else(|_| std::env::var("no_proxy"))
            .unwrap_or_default()
    } else {
        String::new()
    }
}

async fn native_http_client(
    host: &NativeHostBridge,
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    context: &NativeTransportContext<'_>,
) -> Result<(Client, Option<String>, Option<Value>, String), native_contract::ServiceError> {
    // Synchronous proxy policy reads only the prepared in-memory snapshot. A
    // cold native service must load it here, without relying on UI/startup work.
    // Reuse the bounded, single-flight loader; explicit account/direct/API-key
    // routes skip shared storage and load failures must not become a fallback.
    crate::modules::codex_proxy_runtime::ensure_account_proxy_state(account)
        .await
        .map_err(|_| native_error(503, "proxy_unavailable", "Account proxy is unavailable"))?;
    let account_proxy = if account.egress_proxy_disabled {
        None
    } else {
        crate::modules::codex_account_proxy::configured_url(account)
            .map_err(|_| native_error(503, "proxy_unavailable", "Account proxy is unavailable"))?
            .map(|v| v.into_owned())
    };
    // The desktop proxy router owns node lifetimes and a stable local entry;
    // account bindings may be node/catalog references rather than URL proxies.
    let prepared_proxy = if account_proxy.is_some() {
        Some(
            crate::modules::codex_proxy_runtime::ensure(&account.id)
                .await
                .map_err(|_| {
                    native_error(
                        503,
                        "proxy_unavailable",
                        "Account proxy could not be prepared",
                    )
                })?
                .ok_or_else(|| {
                    native_error(
                        503,
                        "proxy_unavailable",
                        "Account proxy has no active entry",
                    )
                })?,
        )
    } else {
        None
    };
    let (proxy, source) = if account.egress_proxy_disabled {
        (None, "account_direct")
    } else if let Some(proxy) = prepared_proxy {
        (Some(proxy), "account")
    } else {
        // Share Go's precedence and OS discovery while keeping system commands
        // off the async runtime. Cache and serialize only the slow auto branch.
        let mut signature = current_upstream_http_client_signature(
            collection.upstream_proxy_url.as_deref(),
            DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
        );
        if signature.proxy_source == UpstreamProxySource::SystemAuto
            && signature.proxy_url.is_none()
        {
            let mut cache = host.system_proxy_cache.lock().await;
            if let Some((at, value)) = cache
                .as_ref()
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(5))
            {
                let _ = at;
                signature.proxy_url = value.clone();
            } else {
                let permit = NATIVE_SYSTEM_PROXY_IO
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| {
                        native_error(
                            503,
                            "proxy_resolution_busy",
                            "Upstream proxy discovery is still running",
                        )
                    })?;
                let worker = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    system_proxy_url_for_target(DEFAULT_OPENAI_RESPONSES_BASE_URL)
                });
                let budget = native_api::execution::remaining_prepare_budget()
                    .unwrap_or_else(|| Duration::from_secs(5))
                    .min(Duration::from_secs(5));
                signature.proxy_url = timeout(budget, worker)
                    .await
                    .map_err(|_| {
                        native_error(
                            504,
                            "proxy_resolution_timeout",
                            "Upstream proxy discovery timed out",
                        )
                    })?
                    .map_err(|_| {
                        native_error(503, "proxy_unavailable", "Upstream proxy discovery failed")
                    })?;
                *cache = Some((Instant::now(), signature.proxy_url.clone()));
            }
        }
        let source = match signature.proxy_source {
            UpstreamProxySource::ApiService => "service",
            UpstreamProxySource::Global => "global",
            UpstreamProxySource::SystemEnv => "environment",
            UpstreamProxySource::SystemAuto if signature.proxy_url.is_some() => "system",
            UpstreamProxySource::SystemAuto => "direct",
        };
        (signature.proxy_url, source)
    };
    // Explicit account/service/global routes remain authoritative. Inherited
    // environment proxies retain their NO_PROXY exclusions, including for
    // WebSocket upgrades made by this same HTTP client.
    let no_proxy = native_inherited_proxy_bypass(source);
    let entry_name = if source == "account" {
        proxy.as_deref().and_then(|url| {
            crate::modules::codex_proxy_runtime::prepared_request_route_observer(account, url)
                .map(|observer| observer.proxy_name)
        })
    } else {
        None
    };
    // Capture the public entry label before the cache branch. An entry is not
    // proof of the final leaf selected by a proxy group for this connection.
    let proxy_route = Some(native_proxy_diagnostic(
        source,
        proxy.as_deref(),
        entry_name.as_deref(),
    ));
    let cache_key = native_transport_fingerprint(
        account,
        collection,
        context,
        source,
        account_proxy.as_deref(),
        proxy.as_deref(),
        &no_proxy,
    );
    if let Some((client, identity)) = host
        .clients
        .lock()
        .ok()
        .and_then(|mut cache| cache.get(&cache_key))
    {
        return Ok((client, proxy, proxy_route, identity));
    }
    let mut builder = native_http_client_builder(
        native_codex_responses_alpn(account, context),
        context.http1_only,
    )
    .redirect(reqwest::redirect::Policy::none())
    .connect_timeout(Duration::from_millis(
        collection
            .timeouts
            .legacy_upstream_connect_timeout_ms
            .clamp(1000, 120_000),
    ))
    .pool_max_idle_per_host(8)
    .no_proxy();
    // The current reqwest matcher checks '*' only for domain names, not IPs.
    // An inherited wildcard excludes every target, so keep the proxy list empty.
    let bypass_all = no_proxy.split(',').any(|entry| entry.trim() == "*");
    if let Some(proxy) = proxy.as_deref().filter(|_| !bypass_all) {
        let normalized = crate::modules::codex_account_proxy::normalize_direct_proxy(proxy)
            .map_err(|_| native_error(503, "proxy_unavailable", "Upstream proxy is invalid"))?;
        builder = builder.proxy(
            Proxy::all(normalized)
                .map_err(|_| native_error(503, "proxy_unavailable", "Upstream proxy is invalid"))?
                .no_proxy(reqwest::NoProxy::from_string(&no_proxy)),
        );
    }
    let client = builder.build().map_err(|_| {
        native_error(
            503,
            "transport_unavailable",
            "Unable to initialize upstream transport",
        )
    })?;
    if let Ok(mut clients) = host.clients.lock() {
        // Another preparation may have built the same client while this one was
        // outside the lock. Reuse its entry without replacing its live pool.
        if let Some((client, identity)) = clients.get(&cache_key) {
            return Ok((client, proxy, proxy_route, identity));
        }
        let identity = clients.insert(cache_key, account.id.clone(), client.clone());
        return Ok((client, proxy, proxy_route, identity));
    }
    // This uncached client has no shared pool to retire.
    Ok((client, proxy, proxy_route, cache_key))
}

fn native_codex_responses_alpn(
    account: &CodexAccount,
    context: &NativeTransportContext<'_>,
) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
        && context.wire_api == native_contract::WireApi::Responses
}

fn native_http_client_builder(codex_responses: bool, http1_only: bool) -> reqwest::ClientBuilder {
    let builder = Client::builder();
    let builder = if codex_responses {
        // The native-tls default does not advertise h2 without an additional
        // global feature. Select ALPN-capable TLS only for Codex Responses,
        // keeping native platform trust and HTTP/1.1 fallback.
        builder.use_rustls_tls()
    } else {
        builder
    };
    // Upgrade uses HTTP/1.1 end-to-end, including TLS ALPN. Request.version
    // alone does not prevent the peer from negotiating h2.
    if http1_only {
        builder.http1_only()
    } else {
        builder
    }
}

fn native_proxy_diagnostic(source: &str, proxy: Option<&str>, entry_name: Option<&str>) -> Value {
    let endpoint = proxy.and_then(|raw| {
        let url = Url::parse(raw).ok()?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
            return None;
        }
        Some(format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str()?,
            url.port_or_known_default()?
        ))
    });
    let route = if let Some(endpoint) = endpoint {
        CodexLocalAccessProxyRoute {
            kind: "proxy".into(),
            name: entry_name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(&endpoint)
                .to_owned(),
        }
    } else {
        CodexLocalAccessProxyRoute {
            kind: if proxy.is_none() && matches!(source, "direct" | "account_direct") {
                "direct"
            } else {
                "unknown"
            }
            .into(),
            name: String::new(),
        }
    };
    // Use the same DTO as Host::record and SQLite. The former mode/proxy object
    // lacked required `kind`, so every native route was silently discarded.
    json!(route)
}

#[cfg(test)]
mod native_proxy_diagnostic_tests {
    use super::*;
    #[test]
    fn route_diagnostics_show_selected_source_without_proxy_credentials() {
        for source in ["account", "service", "global", "environment", "system"] {
            let value = native_proxy_diagnostic(
                source,
                Some("http://sensitive-user:sensitive-pass@127.0.0.1:7890/private-path?token=private-query#private-fragment"),
                None,
            );
            let route: CodexLocalAccessProxyRoute = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(route.kind, "proxy");
            assert_eq!(route.name, "http://127.0.0.1:7890");
            let text = value.to_string();
            for secret in [
                "sensitive-user",
                "sensitive-pass",
                "private-path",
                "private-query",
                "private-fragment",
            ] {
                assert!(!text.contains(secret));
            }
        }
        for source in ["account_direct", "direct"] {
            assert_eq!(
                native_proxy_diagnostic(source, None, None),
                json!({"kind":"direct","name":""})
            );
        }
    }

    #[test]
    fn managed_proxy_labels_are_entries_and_invalid_routes_stay_unknown() {
        let value = native_proxy_diagnostic(
            "account",
            Some("socks5h://127.0.0.1:12345"),
            Some("東京节点"),
        );
        assert_eq!(value, json!({"kind":"proxy","name":"東京节点"}));
        assert_eq!(
            native_proxy_diagnostic("global", Some("socks5://user:secret@[::1]:1080"), Some(" ")),
            json!({"kind":"proxy","name":"socks5://[::1]:1080"})
        );
        for proxy in [
            None,
            Some("invalid private-value"),
            Some("file:///private-value"),
        ] {
            assert_eq!(
                native_proxy_diagnostic("account", proxy, Some("not-verified")),
                json!({"kind":"unknown","name":""})
            );
        }
    }

    #[tokio::test]
    async fn cached_clients_keep_recordable_proxy_snapshots_without_network_requests() {
        let collection: CodexLocalAccessCollection = serde_json::from_value(json!({
            "enabled":true,"port":1455,"apiKey":"fixture","accountIds":[],
            "createdAt":1,"updatedAt":1,
            "upstreamProxyUrl":"http://private-user:private-pass@127.0.0.1:7890"
        }))
        .unwrap();
        let mut account = CodexAccount::new(
            "proxy-fixture".into(),
            "fixture@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: "fixture".into(),
                id_token: "fixture".into(),
                refresh_token: None,
            },
        );
        // API-key accounts do not inspect real account/unified proxy bindings.
        account.auth_mode = CodexAuthMode::Apikey;
        let host = NativeHostBridge::default();
        let profile = crate::modules::codex_api_identity::InstallationProfile {
            version: 1,
            scope: "proxy-fixture".into(),
            installation_id: "00000000-0000-4000-8000-000000000001".into(),
            revision: 1,
        };
        let headers = axum::http::HeaderMap::new();
        let context = NativeTransportContext {
            owner: "proxy-fixture-owner",
            profile: &profile,
            base_url: "https://example.invalid/v1",
            wire_api: native_contract::WireApi::Responses,
            http1_only: false,
            headers: &headers,
            compact_endpoint: None,
            websocket_endpoint: None,
        };
        for _ in 0..2 {
            let (_, proxy, diagnostic, _) =
                native_http_client(&host, &collection, &account, &context)
                    .await
                    .unwrap();
            assert_eq!(proxy, collection.upstream_proxy_url);
            let route: CodexLocalAccessProxyRoute =
                serde_json::from_value(diagnostic.unwrap()).unwrap();
            assert_eq!(route.kind, "proxy");
            assert_eq!(route.name, "http://127.0.0.1:7890");
            assert_eq!(host.clients.lock().unwrap().len(), 1);
        }
    }
}

#[cfg(test)]
mod native_transport_identity_tests {
    use super::*;
    include!("codex_local_access_native_transport_tests.rs");
}
