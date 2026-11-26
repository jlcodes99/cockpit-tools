// Shared transport policy for native HTTP and WebSocket requests.
const NATIVE_HTTP_CLIENT_CACHE_LIMIT: usize = 128;
const NATIVE_TLS_ROOTS_TTL: Duration = Duration::from_secs(60);
const NATIVE_SYSTEM_PROXY_CACHE_TTL: Duration = Duration::from_secs(15);
const NATIVE_SYSTEM_PROXY_FALLBACK_TTL: Duration = Duration::from_secs(5);

struct NativeHttpClientEntry {
    account_id: String,
    client: Client,
    identity: String,
    used: u64,
    http1_fallback: bool,
}

struct NativeHttpClientCache {
    entries: HashMap<String, NativeHttpClientEntry>,
    recovery: HashMap<String, NativeTransportRecovery>,
    clock: u64,
    capacity: usize,
}

impl Default for NativeHttpClientCache {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            recovery: HashMap::new(),
            clock: 0,
            capacity: NATIVE_HTTP_CLIENT_CACHE_LIMIT,
        }
    }
}

struct NativeTransportRecovery {
    first_failure: Instant,
    fallback_until: Option<Instant>,
}

impl NativeHttpClientCache {
    fn http1_fallback(&mut self, key: &str, now: Instant) -> bool {
        self.recovery.retain(|_, entry| entry.fallback_until
            .map_or(now.saturating_duration_since(entry.first_failure) < Duration::from_secs(60), |until| now < until));
        self.recovery.get(key).is_some_and(|entry| entry.fallback_until.is_some())
    }

    fn invalidate_http2(&mut self, identity: &str, now: Instant) -> bool {
        if !self.invalidate(identity) { return false; }
        let (key, _) = identity.rsplit_once(':').unwrap();
        self.http1_fallback(key, now);
        if let Some(entry) = self.recovery.get_mut(key) {
            // Two distinct failed pool generations, not concurrent failures
            // of one multiplexed connection. Never extend an active cooldown.
            if entry.fallback_until.is_none() {
                entry.fallback_until = Some(now + Duration::from_secs(300));
                crate::modules::logger::log_info("[Codex API] Repeated HTTP/2 TLS failure: scoped HTTP/1.1 fallback for 300s");
            }
        } else {
            if self.recovery.len() >= self.capacity {
                if let Some(oldest) = self.recovery.iter().min_by_key(|(_, e)| e.first_failure).map(|(k, _)| k.clone()) {
                    self.recovery.remove(&oldest);
                }
            }
            self.recovery.insert(key.to_owned(), NativeTransportRecovery { first_failure: now, fallback_until: None });
        }
        true
    }

    fn get(&mut self, key: &str) -> Option<(Client, String)> {
        let fallback = self.http1_fallback(key, Instant::now());
        if self.entries.get(key).is_some_and(|entry| entry.http1_fallback != fallback) {
            self.entries.remove(key);
        }
        let entry = self.entries.get_mut(key)?;
        self.clock = self.clock.saturating_add(1);
        entry.used = self.clock;
        Some((entry.client.clone(), entry.identity.clone()))
    }

    #[cfg(test)]
    fn insert(&mut self, key: String, account_id: String, client: Client) -> String {
        let fallback = self.http1_fallback(&key, Instant::now());
        self.insert_with_policy(key, account_id, client, fallback)
    }

    fn insert_with_policy(&mut self, key: String, account_id: String, client: Client, http1_fallback: bool) -> String {
        self.clock = self.clock.saturating_add(1);
        let identity = format!("{key}:{}", uuid::Uuid::new_v4());
        self.entries.insert(
            key,
            NativeHttpClientEntry {
                account_id,
                client,
                identity: identity.clone(),
                used: self.clock,
                http1_fallback,
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
        "version":6,"account":account.id,"owner":context.owner,
        "profile":[context.profile.version,context.profile.scope,context.profile.installation_id,context.profile.revision],
        "base":native_transport_endpoint(context.base_url),
        "protocol":match context.wire_api { native_contract::WireApi::Responses=>"responses",native_contract::WireApi::Chat=>"chat",native_contract::WireApi::Anthropic=>"anthropic" },
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
    let api_proxy_account;
    let proxy_account = if account.is_api_key_auth() && !account.egress_proxy_disabled {
        api_proxy_account = crate::modules::codex_client_proxy::upstream_route(account)
            .await
            .map_err(|_| native_error(503, "proxy_unavailable", "Account proxy is unavailable"))?;
        &api_proxy_account
    } else {
        account
    };
    // Synchronous proxy policy reads only the prepared in-memory snapshot. A
    // cold native service must load it here, without relying on UI/startup work.
    // Reuse the bounded, single-flight loader; explicit account/direct
    // routes skip shared storage and load failures must not become a fallback.
    crate::modules::codex_proxy_runtime::ensure_account_proxy_state(proxy_account)
        .await
        .map_err(|_| native_error(503, "proxy_unavailable", "Account proxy is unavailable"))?;
    let account_proxy = if proxy_account.egress_proxy_disabled {
        None
    } else {
        crate::modules::codex_account_proxy::configured_url(proxy_account)
            .map_err(|_| native_error(503, "proxy_unavailable", "Account proxy is unavailable"))?
            .map(|v| v.into_owned())
    };
    // The desktop proxy router owns node lifetimes and a stable local entry;
    // account bindings may be node/catalog references rather than URL proxies.
    let prepared_proxy = if account_proxy.is_some() {
        Some(
            crate::modules::codex_proxy_runtime::ensure(&proxy_account.id)
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
    let (proxy, source) = if proxy_account.egress_proxy_disabled {
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
            signature.proxy_url = resolve_native_system_proxy(host).await;
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
            crate::modules::codex_proxy_runtime::prepared_request_route_observer(proxy_account, url)
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
    loop {
        let (cached, fallback) = host.clients.lock().map(|mut cache| {
            let fallback = cache.http1_fallback(&cache_key, Instant::now());
            (cache.get(&cache_key), fallback)
        }).unwrap_or((None, false));
        let mut proxy_route = proxy_route.clone();
        if fallback {
            if let Some(route) = proxy_route.as_mut().and_then(Value::as_object_mut) {
                route.insert("transportHttp1Fallback".into(), Value::Bool(true));
            }
        }
        if let Some((client, identity)) = cached {
            return Ok((client, proxy, proxy_route, identity));
        }
        let http1_only = context.http1_only || fallback;
        let mut builder = if native_codex_responses_alpn(account, context) {
            let roots = native_codex_tls_roots(host).await?;
            native_codex_http_client_builder(roots, http1_only)?
        } else {
            native_http_client_builder(http1_only)
        }
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(
            collection
                .timeouts
                .legacy_upstream_connect_timeout_ms
                .clamp(1000, 120_000),
        ))
        // Decode at the protocol boundary so repeated Content-Encoding headers and
        // encoding chains are handled together, before SSE/bootstrap inspection.
        .no_gzip().no_brotli().no_deflate().no_zstd()
        .default_headers(axum::http::HeaderMap::from_iter([(
            axum::http::header::ACCEPT_ENCODING,
            axum::http::HeaderValue::from_static("gzip, deflate, br, zstd"),
        )]))
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
            if clients.http1_fallback(&cache_key, Instant::now()) != fallback {
                continue;
            }
            // Another preparation may have built the same client outside the lock.
            if let Some((client, identity)) = clients.get(&cache_key) {
                return Ok((client, proxy, proxy_route, identity));
            }
            let identity = clients.insert_with_policy(cache_key.clone(), account.id.clone(), client.clone(), fallback);
            return Ok((client, proxy, proxy_route, identity));
        }
        // This uncached client has no shared pool to retire.
        return Ok((client, proxy, proxy_route, cache_key));
    }
}

async fn resolve_native_system_proxy_with<F>(
    host: &NativeHostBridge,
    discover: F,
) -> Option<String>
where
    F: FnOnce() -> Option<String> + Send + 'static,
{
    let stale_fallback = {
        let cache = host.system_proxy_cache.lock().await;
        if let Some((at, value)) = cache
            .as_ref()
            .filter(|(at, _)| at.elapsed() < NATIVE_SYSTEM_PROXY_CACHE_TTL)
        {
            let _ = at;
            return value.clone();
        }
        cache.as_ref().and_then(|(_, value)| value.clone())
    };

    let fallback_label = if stale_fallback.is_some() {
        "cached_proxy"
    } else {
        "direct"
    };

    let Ok(permit) = NATIVE_SYSTEM_PROXY_IO.clone().try_acquire_owned() else {
        crate::modules::logger::log_codex_api_warn(&format!(
            "[Codex API] System proxy discovery is already running; degrading to {fallback_label}"
        ));
        return stale_fallback;
    };

    let budget = native_api::execution::remaining_prepare_budget()
        .unwrap_or_else(|| Duration::from_secs(2))
        .min(Duration::from_secs(2));
    if budget < Duration::from_millis(25) {
        drop(permit);
        crate::modules::logger::log_codex_api_warn(&format!(
            "[Codex API] Remaining prepare budget ({}ms) is too small for system proxy discovery; degrading to {fallback_label}",
            budget.as_millis()
        ));
        return stale_fallback;
    }

    let started = Instant::now();
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        discover()
    });

    match timeout(budget, worker).await {
        Ok(Ok(discovered)) => {
            let mut cache = host.system_proxy_cache.lock().await;
            *cache = Some((Instant::now(), discovered.clone()));
            discovered
        }
        Ok(Err(join_error)) => {
            let cooldown_anchor = Instant::now()
                .checked_sub(
                    NATIVE_SYSTEM_PROXY_CACHE_TTL
                        .saturating_sub(NATIVE_SYSTEM_PROXY_FALLBACK_TTL),
                )
                .unwrap_or_else(Instant::now);
            let mut cache = host.system_proxy_cache.lock().await;
            *cache = Some((cooldown_anchor, stale_fallback.clone()));
            crate::modules::logger::log_codex_api_warn(&format!(
                "[Codex API] System proxy discovery worker failed after {}ms ({join_error}); degrading to {fallback_label}",
                started.elapsed().as_millis()
            ));
            stale_fallback
        }
        Err(_) => {
            let cooldown_anchor = Instant::now()
                .checked_sub(
                    NATIVE_SYSTEM_PROXY_CACHE_TTL
                        .saturating_sub(NATIVE_SYSTEM_PROXY_FALLBACK_TTL),
                )
                .unwrap_or_else(Instant::now);
            let mut cache = host.system_proxy_cache.lock().await;
            *cache = Some((cooldown_anchor, stale_fallback.clone()));
            crate::modules::logger::log_codex_api_warn(&format!(
                "[Codex API] System proxy discovery timed out after {}ms; degrading to {fallback_label}",
                started.elapsed().as_millis()
            ));
            stale_fallback
        }
    }
}

async fn resolve_native_system_proxy(host: &NativeHostBridge) -> Option<String> {
    resolve_native_system_proxy_with(host, || {
        system_proxy_url_for_target(DEFAULT_OPENAI_RESPONSES_BASE_URL)
    })
    .await
}

async fn native_codex_tls_roots(
    host: &NativeHostBridge,
) -> Result<Arc<rustls::RootCertStore>, native_contract::ServiceError> {
    let mut cache = host.tls_roots.clone().lock_owned().await;
    if let Some((loaded_at, roots)) = cache.as_ref() {
        if loaded_at.elapsed() < NATIVE_TLS_ROOTS_TTL {
            return Ok(roots.clone());
        }
    }
    // A cancelled request must not start a second trust-store load while this
    // worker still owns the lock. Never hold the transport cache lock here.
    tokio::task::spawn_blocking(move || {
        let loaded = rustls_native_certs::load_native_certs();
        let mut roots = rustls::RootCertStore::empty();
        let (added, ignored) = roots.add_parsable_certificates(loaded.certs);
        if added == 0 && (ignored > 0 || !loaded.errors.is_empty()) {
            return Err(native_error(
                503,
                "transport_unavailable",
                "Unable to load upstream TLS trust roots",
            ));
        }
        let roots = Arc::new(roots);
        *cache = Some((Instant::now(), roots.clone()));
        Ok(roots)
    })
    .await
    .map_err(|_| native_error(
        503,
        "transport_unavailable",
        "Unable to load upstream TLS trust roots",
    ))?
}

fn native_codex_http_client_builder(
    roots: Arc<rustls::RootCertStore>,
    http1_only: bool,
) -> Result<reqwest::ClientBuilder, native_contract::ServiceError> {
    let provider = rustls::crypto::CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| native_error(503, "transport_unavailable", "Unable to initialize upstream TLS"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    // Match reqwest's protocol negotiation while building a fresh per-client
    // TLS configuration and resumption store from shared trust roots only.
    tls.alpn_protocols = if http1_only {
        vec![b"http/1.1".to_vec()]
    } else {
        vec![b"h2".to_vec(), b"http/1.1".to_vec()]
    };
    Ok(native_http_client_builder(http1_only).use_preconfigured_tls(tls))
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

fn native_http_client_builder(http1_only: bool) -> reqwest::ClientBuilder {
    // The local proxy's TCP socket can stay open after its upstream path dies.
    // HTTP/2 PING checks that path even while waiting for response headers or
    // model output. Healthy PONGs do not impose a total generation deadline.
    let builder = Client::builder()
        .http2_keep_alive_interval(Duration::from_secs(30))
        .http2_keep_alive_timeout(Duration::from_secs(15))
        .http2_keep_alive_while_idle(false)
        .tcp_keepalive(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(30));
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
        let _proxy_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
        _proxy_state.prepare_disabled();
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
        // With no account/unified binding, API keys use the service proxy.
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

    #[tokio::test]
    async fn system_proxy_discovery_busy_or_timeout_degrades_to_cached_or_direct_without_failing() {
        let host = NativeHostBridge::default();
        // When the single-flight permit is held by a slow worker, concurrent
        // requests must degrade immediately to direct/cached instead of 503/504.
        {
            let _held = NATIVE_SYSTEM_PROXY_IO.clone().acquire_owned().await.unwrap();
            let direct = resolve_native_system_proxy_with(&host, || {
                Some("http://127.0.0.1:7890".to_string())
            })
            .await;
            assert_eq!(direct, None);

            *host.system_proxy_cache.lock().await = Some((
                Instant::now() - NATIVE_SYSTEM_PROXY_CACHE_TTL - Duration::from_secs(1),
                Some("http://127.0.0.1:10808".to_string()),
            ));
            let stale = resolve_native_system_proxy_with(&host, || None).await;
            assert_eq!(stale.as_deref(), Some("http://127.0.0.1:10808"));
        }

        // Normal discovery updates the cache and serves fresh reads within TTL.
        *host.system_proxy_cache.lock().await = None;
        let discovered = resolve_native_system_proxy_with(&host, || {
            Some("http://127.0.0.1:7890".to_string())
        })
        .await;
        assert_eq!(discovered.as_deref(), Some("http://127.0.0.1:7890"));
        let cached = resolve_native_system_proxy_with(&host, || None).await;
        assert_eq!(cached.as_deref(), Some("http://127.0.0.1:7890"));
    }

    #[test]
    fn effective_billing_service_tier_preserves_outbound_fast_when_oauth_reports_default() {
        assert_eq!(
            effective_billing_service_tier(Some("priority"), Some("default")),
            Some("priority")
        );
        assert_eq!(
            effective_billing_service_tier(Some("ultrafast"), Some("default")),
            Some("ultrafast")
        );
        assert_eq!(
            effective_billing_service_tier(Some("default"), Some("default")),
            Some("default")
        );
        assert_eq!(
            effective_billing_service_tier(Some("auto"), Some("priority")),
            Some("priority")
        );
        assert_eq!(
            effective_billing_service_tier(None, Some("default")),
            Some("default")
        );
    }
}

#[cfg(test)]
mod native_transport_identity_tests {
    use super::*;
    include!("codex_local_access_native_transport_tests.rs");
}
