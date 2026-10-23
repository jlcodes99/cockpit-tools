#[derive(Clone)]
struct Inputs {
    account: CodexAccount,
    collection: CodexLocalAccessCollection,
    profile: crate::modules::codex_api_identity::InstallationProfile,
    headers: axum::http::HeaderMap,
    owner: String,
    base: String,
    wire: native_contract::WireApi,
    http1_only: bool,
    endpoint: Option<native_contract::EndpointOverride>,
}
impl Inputs {
    fn new() -> Self {
        let mut account = CodexAccount::new(
            "transport-account".into(),
            "fixture@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: "fixture-access".into(),
                id_token: "fixture-id".into(),
                refresh_token: None,
            },
        );
        account.auth_mode = CodexAuthMode::Apikey;
        account.openai_api_key = Some("fixture-key".into());
        Self {
            account,
            collection: serde_json::from_value(json!({"enabled":true,"port":1455,"apiKey":"fixture","accountIds":[],"createdAt":1,"updatedAt":1,"upstreamProxyUrl":"http://proxy-user:proxy-secret@127.0.0.1:7890"})).unwrap(),
            profile: crate::modules::codex_api_identity::InstallationProfile {version:1,scope:"fixture-scope".into(),installation_id:"00000000-0000-4000-8000-000000000001".into(),revision:1},
            headers: axum::http::HeaderMap::from_iter([(AUTHORIZATION,HeaderValue::from_static("Bearer fixture-key"))]),
            owner:"fixture-owner".into(),base:"https://example.invalid/v1".into(),wire:native_contract::WireApi::Responses,http1_only:false,endpoint:None,
        }
    }
    fn context(&self) -> NativeTransportContext<'_> {
        NativeTransportContext {
            owner: &self.owner,
            profile: &self.profile,
            base_url: &self.base,
            wire_api: self.wire,
            http1_only: self.http1_only,
            headers: &self.headers,
            compact_endpoint: self.endpoint.as_ref(),
            websocket_endpoint: self.endpoint.as_ref(),
        }
    }
    fn key(&self) -> String {
        native_transport_fingerprint(
            &self.account,
            &self.collection,
            &self.context(),
            "account",
            Some("catalog-node-a"),
            Some("http://127.0.0.1:7890"),
            "",
        )
    }
}

#[test]
fn transport_identity_covers_owner_installation_realm_credentials_and_configuration() {
    let baseline = Inputs::new();
    let key = baseline.key();
    let mutations: Vec<Box<dyn Fn(&mut Inputs)>> = vec![
        Box::new(|v| v.account.id = "another-account".into()),
        Box::new(|v| v.owner = "another-owner".into()),
        Box::new(|v| v.profile.scope = "another-scope".into()),
        Box::new(|v| v.profile.installation_id = "00000000-0000-4000-8000-000000000002".into()),
        Box::new(|v| v.profile.revision += 1),
        Box::new(|v| v.base = "https://example.invalid/other-realm/v1".into()),
        Box::new(|v| v.http1_only = true),
        Box::new(|v| v.wire = native_contract::WireApi::Chat),
        Box::new(|v| {
            v.account.auth_mode = CodexAuthMode::OAuth;
            v.account.openai_api_key = None;
        }),
        Box::new(|v| v.account.token_generation += 1),
        Box::new(|v| {
            v.headers.insert(
                AUTHORIZATION,
                HeaderValue::from_static("Bearer rotated-key"),
            );
        }),
        Box::new(|v| v.account.api_supports_websockets = true),
        Box::new(|v| v.collection.timeouts.legacy_upstream_connect_timeout_ms = 17_000),
        Box::new(|v| {
            v.endpoint = Some(native_contract::EndpointOverride {
                base_url: "https://override.invalid/v1".into(),
                headers: axum::http::HeaderMap::new(),
            })
        }),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = baseline.clone();
        mutate(&mut changed);
        assert_ne!(changed.key(), key, "mutation {index} retained an old pool");
    }
    let mut equivalent = baseline.clone();
    equivalent.base.push('/');
    assert_eq!(equivalent.key(), key);
    let changed_proxy = native_transport_fingerprint(
        &baseline.account,
        &baseline.collection,
        &baseline.context(),
        "account",
        Some("catalog-node-b"),
        Some("http://127.0.0.1:7890"),
        "",
    );
    assert_ne!(
        changed_proxy, key,
        "stable local proxy entry must not hide a changed proxy binding"
    );
    assert_eq!(key.len(), 64);
    assert!(!key.contains("fixture-key") && !key.contains("proxy-secret"));
}

#[test]
fn transport_identity_uses_agent_task_key_generation_but_not_timestamp_assertions() {
    let mut inputs = Inputs::new();
    inputs.account.agent_identity=Some(serde_json::from_value(json!({"agent_runtime_id":"runtime","agent_private_key":"private-key","task_id":"task","account_id":"workspace","chatgpt_user_id":"principal"})).unwrap());
    inputs.headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("AgentAssertion signed-at-one"),
    );
    let key = inputs.key();
    inputs.headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("AgentAssertion signed-at-two"),
    );
    assert_eq!(inputs.key(), key);
    for field in ["task", "private_key"] {
        let mut changed = inputs.clone();
        let agent = changed.account.agent_identity.as_mut().unwrap();
        if field == "task" {
            agent.task_id = Some("another-task".into());
        } else {
            agent.agent_private_key = "another-key".into();
        }
        assert_ne!(changed.key(), key);
    }
    inputs.account.agent_identity = None;
    inputs.account.upstream_grok_account_id = Some("grok-source".into());
    inputs.headers.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer grok-before"),
    );
    let before = inputs.key();
    inputs
        .headers
        .insert(AUTHORIZATION, HeaderValue::from_static("Bearer grok-after"));
    assert_ne!(
        inputs.key(),
        before,
        "Grok token refresh may retain the Codex record's token_generation"
    );
}

#[tokio::test]
async fn native_client_cache_separates_accounts_on_the_same_proxy_and_reuses_exact_identity() {
    let host = NativeHostBridge::default();
    let first = Inputs::new();
    let mut second = first.clone();
    second.account.id = "another-account".into();
    second.owner = "another-owner".into();
    second.profile.scope = "another-scope".into();
    let (_, proxy_a, _, key_a) =
        native_http_client(&host, &first.collection, &first.account, &first.context())
            .await
            .unwrap();
    let (_, proxy_b, _, key_b) = native_http_client(
        &host,
        &second.collection,
        &second.account,
        &second.context(),
    )
    .await
    .unwrap();
    assert_eq!(proxy_a, proxy_b);
    assert_ne!(key_a, key_b);
    assert_eq!(host.clients.lock().unwrap().len(), 2);
    let (_, _, _, reused) =
        native_http_client(&host, &first.collection, &first.account, &first.context())
            .await
            .unwrap();
    assert_eq!(reused, key_a);
    assert_eq!(host.clients.lock().unwrap().len(), 2);
    host.clients
        .lock()
        .unwrap()
        .retain_accounts(|account| account != first.account.id);
    assert_eq!(host.clients.lock().unwrap().len(), 1);
    assert!(host.clients.lock().unwrap().entries.values().any(|entry| entry.identity == key_b));
}

#[test]
fn native_client_cache_evicts_only_the_least_recently_used_entry() {
    let mut cache = NativeHttpClientCache {
        capacity: 2,
        ..Default::default()
    };
    let client = Client::builder().no_proxy().build().unwrap();
    for key in ["a", "b"] {
        cache.insert(key.into(), key.into(), client.clone());
    }
    assert!(cache.get("a").is_some());
    cache.insert("c".into(), "c".into(), client);
    assert_eq!(cache.len(), 2);
    assert!(cache.get("a").is_some());
    assert!(cache.get("b").is_none());
    assert!(cache.get("c").is_some());
}

#[tokio::test]
async fn native_tls_invalidation_rebuilds_only_the_failed_generation() {
    let host = NativeHostBridge::default();
    let first = Inputs::new();
    let mut other = first.clone();
    other.account.id = "other-account".into();
    let mut websocket = first.clone();
    websocket.http1_only = true;
    let (_, _, _, original) = native_http_client(&host, &first.collection, &first.account, &first.context()).await.unwrap();
    let (_, _, _, other_identity) = native_http_client(&host, &other.collection, &other.account, &other.context()).await.unwrap();
    let (_, _, _, websocket_identity) = native_http_client(&host, &websocket.collection, &websocket.account, &websocket.context()).await.unwrap();
    assert!(native_contract::Host::invalidate_transport(&host, &original));
    assert!(!native_contract::Host::invalidate_transport(&host, &original));
    let (_, _, _, replacement) = native_http_client(&host, &first.collection, &first.account, &first.context()).await.unwrap();
    assert_ne!(original, replacement);
    assert!(!native_contract::Host::invalidate_transport(&host, &original), "late failure must not retire a newer pool");
    let (_, _, _, current) = native_http_client(&host, &first.collection, &first.account, &first.context()).await.unwrap();
    assert_eq!(current, replacement);
    for (input, expected) in [(&other, other_identity), (&websocket, websocket_identity)] {
        let (_, _, _, actual) = native_http_client(&host, &input.collection, &input.account, &input.context()).await.unwrap();
        assert_eq!(actual, expected, "another account or protocol must keep its pool");
    }
    assert_eq!(host.clients.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn native_client_eviction_keeps_an_active_response_alive_and_never_retains_cookies() {
    use axum::{
        body::{Body, Bytes},
        http::HeaderMap,
        response::Response,
        routing::get,
        Router,
    };
    use futures_util::{stream, StreamExt};
    let release = Arc::new(tokio::sync::Notify::new());
    let signal = release.clone();
    let cookies = Arc::new(Mutex::new(Vec::new()));
    let capture = cookies.clone();
    let app = Router::new()
        .route(
            "/held",
            get(move || {
                let signal = signal.clone();
                async move {
                    let chunks = stream::once(async {
                        Ok::<_, std::io::Error>(Bytes::from_static(b"first"))
                    })
                    .chain(stream::once(async move {
                        signal.notified().await;
                        Ok(Bytes::from_static(b"last"))
                    }));
                    Response::new(Body::from_stream(chunks))
                }
            }),
        )
        .route(
            "/cookie",
            get(move |headers: HeaderMap| {
                let capture = capture.clone();
                async move {
                    capture.lock().unwrap().push(headers.get("cookie").cloned());
                    Response::builder()
                        .header("set-cookie", "session=account-a; Path=/")
                        .body(Body::from("ok"))
                        .unwrap()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = NativeHostBridge::default();
    host.clients.lock().unwrap().capacity = 1;
    let mut first = Inputs::new();
    first.account.egress_proxy_disabled = true;
    let (client_a, _, _, key_a) =
        native_http_client(&host, &first.collection, &first.account, &first.context())
            .await
            .unwrap();
    let mut response = client_a
        .get(format!("http://{address}/held"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap(), "first");
    let mut second = first.clone();
    second.account.id = "account-b".into();
    second.profile.scope = "profile-b".into();
    second.owner = "owner-b".into();
    let (client_b, _, _, _) = native_http_client(
        &host,
        &second.collection,
        &second.account,
        &second.context(),
    )
    .await
    .unwrap();
    assert!(!host.clients.lock().unwrap().entries.values().any(|entry| entry.identity == key_a));
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), response.chunk())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "last"
    );
    for client in [&client_a, &client_a, &client_b] {
        client
            .get(format!("http://{address}/cookie"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
    }
    assert!(cookies.lock().unwrap().iter().all(Option::is_none));
    task.abort();
}

include!("codex_local_access_native_proxy_bypass_tests.rs");
include!("codex_local_access_native_proxy_startup_tests.rs");
include!("codex_local_access_native_tls_lifecycle_tests.rs");

const NATIVE_ALPN_DEADLINE: Duration = Duration::from_secs(5);

/// A TLS-only peer records the negotiated protocol, then closes deliberately.
/// The certificate is generated per fixture and trusted only by the test client.
async fn native_alpn_fixture(
    protocols: Vec<Vec<u8>>,
) -> (
    std::net::SocketAddr,
    reqwest::Certificate,
    tokio::task::JoinHandle<std::io::Result<Option<Vec<u8>>>>,
) {
    let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let root = reqwest::Certificate::from_der(certificate.cert.der()).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.cert.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der()).into(),
    )
    .unwrap();
    config.alpn_protocols = protocols;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        tokio::time::timeout(NATIVE_ALPN_DEADLINE, async move {
            let (socket, peer) = listener.accept().await?;
            assert!(peer.ip().is_loopback());
            let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await?;
            Ok(tls.get_ref().1.alpn_protocol().map(<[u8]>::to_vec))
        })
        .await
        .expect("native ALPN loopback handshake deadline")
    });
    (address, root, server)
}

#[test]
fn native_alpn_is_scoped_to_oauth_responses_and_http1_pools_are_separate() {
    let mut input = Inputs::new();
    assert!(!native_codex_responses_alpn(
        &input.account,
        &input.context()
    ));
    input.account.auth_mode = CodexAuthMode::OAuth;
    input.account.openai_api_key = None;
    assert!(native_codex_responses_alpn(
        &input.account,
        &input.context()
    ));
    let http_key = input.key();
    input.http1_only = true;
    assert_ne!(input.key(), http_key);
    input.wire = native_contract::WireApi::Chat;
    assert!(!native_codex_responses_alpn(
        &input.account,
        &input.context()
    ));
}

#[tokio::test]
async fn native_codex_responses_tls_negotiates_h2_with_the_production_client_builder() {
    let (address, root, server) =
        native_alpn_fixture(vec![b"h2".to_vec(), b"http/1.1".to_vec()]).await;
    let client = native_http_client_builder(true, false)
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(root)
        .timeout(NATIVE_ALPN_DEADLINE)
        .build()
        .unwrap();
    // HTTP failure is intentional: this fixture serves only a TLS handshake.
    assert!(client
        .get(format!("https://{address}/responses"))
        .send()
        .await
        .is_err());
    assert_eq!(
        server.await.unwrap().unwrap().as_deref(),
        Some(b"h2".as_slice())
    );
}

#[tokio::test]
async fn native_codex_responses_tls_falls_back_for_http1_only_and_no_alpn_peers() {
    for protocols in [vec![b"http/1.1".to_vec()], vec![]] {
        let expected = protocols.first().cloned();
        let (address, root, server) = native_alpn_fixture(protocols).await;
        let client = native_http_client_builder(true, false)
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(root)
            .timeout(NATIVE_ALPN_DEADLINE)
            .build()
            .unwrap();
        assert!(client
            .get(format!("https://{address}/responses"))
            .send()
            .await
            .is_err());
        assert_eq!(server.await.unwrap().unwrap(), expected);
    }
}

#[tokio::test]
async fn native_codex_responses_tls_rejects_an_untrusted_certificate() {
    let (address, _untrusted_root, server) =
        native_alpn_fixture(vec![b"h2".to_vec(), b"http/1.1".to_vec()]).await;
    let client = native_http_client_builder(true, false)
        .no_proxy()
        .tls_built_in_root_certs(false)
        .timeout(NATIVE_ALPN_DEADLINE)
        .build()
        .unwrap();
    let error = client
        .get(format!("https://{address}/responses"))
        .send()
        .await
        .unwrap_err();
    fn unknown_issuer(error: &(dyn std::error::Error + 'static)) -> bool {
        if matches!(
            error.downcast_ref::<rustls::Error>(),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer
            ))
        ) {
            return true;
        }
        // io::Error::source may skip the wrapped TLS error's own type.
        // Inspect its concrete payload as well as the ordinary source chain.
        if error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .is_some_and(|inner| unknown_issuer(inner))
        {
            return true;
        }
        error.source().is_some_and(unknown_issuer)
    }
    assert!(
        unknown_issuer(&error),
        "expected certificate validation failure: {error:?}"
    );
    assert!(
        server.await.unwrap().is_err(),
        "untrusted TLS must not complete a handshake"
    );
}

#[tokio::test]
async fn native_codex_responses_tls_http11_websocket_upgrade_does_not_negotiate_h2() {
    let (address, root, server) =
        native_alpn_fixture(vec![b"h2".to_vec(), b"http/1.1".to_vec()]).await;
    let client = native_http_client_builder(true, true)
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(root)
        .timeout(NATIVE_ALPN_DEADLINE)
        .build()
        .unwrap();
    let result = client
        .get(format!("https://{address}/responses"))
        .version(reqwest::Version::HTTP_11)
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await;
    assert!(
        result.is_err(),
        "TLS-only fixture intentionally closes before HTTP upgrade"
    );
    assert_eq!(
        server.await.unwrap().unwrap().as_deref(),
        Some(b"http/1.1".as_slice()),
        "WebSocket Upgrade must use an HTTP/1-only TLS client even when its peer prefers h2"
    );
}

#[tokio::test]
async fn native_codex_responses_h2_reads_large_chunked_sse_through_the_completed_event() {
    use futures_util::StreamExt;
    use hyper::{server::conn::http2, service::service_fn};
    use hyper_util::rt::{TokioExecutor, TokioIo};

    // A temporary CA signs only this fixture's loopback certificate. It is
    // passed to this client explicitly and never installed in global trust.
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let server_key = rcgen::KeyPair::generate().unwrap();
    let mut server_params = rcgen::CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
    server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let certificate = server_params.signed_by(&server_key, &ca, &ca_key).unwrap();
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![certificate.der().clone(), ca.der().clone()],
        rustls::pki_types::PrivatePkcs8KeyDer::from(server_key.serialize_der()).into(),
    )
    .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let mut frames = vec![axum::body::Bytes::from(format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":"large-response","status":"in_progress"}})
    ))];
    for index in 0..48 {
        frames.push(axum::body::Bytes::from(format!("data: {}\n\n",
            json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"chunk_index":index,"delta":"A".repeat(32 * 1024)}))));
    }
    frames.push(axum::body::Bytes::from(format!("data: {}\n\n",
        json!({"type":"response.completed","response":{"id":"large-response","status":"completed","output":[]}}))));
    let expected: Vec<u8> = frames
        .iter()
        .flat_map(|frame| frame.iter().copied())
        .collect();
    assert!(expected.len() > 1280 * 1024, "fixture must exceed 1.2 MiB");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let deadline = Duration::from_secs(10);
    let server = tokio::spawn(async move {
        tokio::time::timeout(deadline, async move {
            let (socket, peer) = listener.accept().await.unwrap();
            assert!(peer.ip().is_loopback());
            let tls = tokio_rustls::TlsAcceptor::from(Arc::new(config))
                .accept(socket)
                .await
                .unwrap();
            assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
            let service = service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                assert_eq!(request.version(), hyper::Version::HTTP_2);
                assert_eq!(request.uri().path(), "/responses");
                let frames = frames.clone();
                async move {
                    let chunks = futures_util::stream::iter(frames).then(|frame| async move {
                        tokio::task::yield_now().await;
                        Ok::<_, std::convert::Infallible>(frame)
                    });
                    Ok::<_, std::convert::Infallible>(
                        hyper::Response::builder()
                            .header("content-type", "text/event-stream")
                            .body(axum::body::Body::from_stream(chunks))
                            .unwrap(),
                    )
                }
            });
            http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tls), service)
                .await
                .unwrap();
        })
        .await
        .expect("large H2 SSE fixture deadline");
    });
    let client = native_http_client_builder(true, false)
        .no_proxy()
        .tls_built_in_root_certs(false)
        .add_root_certificate(reqwest::Certificate::from_der(ca.der()).unwrap())
        .timeout(deadline)
        .build()
        .unwrap();
    let response = client
        .get(format!("https://{address}/responses"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    let mut chunks = response.bytes_stream();
    let mut bytes = Vec::new();
    let mut count = 0;
    while let Some(chunk) = chunks.next().await {
        bytes.extend_from_slice(&chunk.expect("H2 SSE body must not be truncated"));
        count += 1;
    }
    assert!(count > 1, "large SSE must arrive in multiple body chunks");
    assert_eq!(
        bytes, expected,
        "all SSE bytes must survive H2 flow control"
    );
    let events: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .map(|frame| serde_json::from_str(frame).unwrap())
        .collect();
    assert_eq!(events.len(), 50);
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    assert_eq!(events.last().unwrap()["response"]["status"], "completed");
    drop(chunks);
    drop(client);
    server.await.unwrap();
}
