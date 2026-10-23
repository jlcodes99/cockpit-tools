//! Real loopback TLS corruption before or after visible tool output. The only trusted CA
//! is generated per test; no user credentials, proxy settings or external hosts.
use super::{contract::*, start, Handle, Options};
use axum::http::HeaderMap;
use futures_util::{future::BoxFuture, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{Notify, Semaphore},
};

const DEADLINE: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Debug)]
enum Mode {
    Responses,
    Codex,
    Plan,
    Chat,
}

struct Generation {
    number: usize,
    client: reqwest::Client,
}

struct TlsHost {
    mode: Mode,
    base: String,
    root: reqwest::Certificate,
    generation: Mutex<Generation>,
    invalidated: Mutex<Vec<String>>,
    routed: Mutex<Vec<(RouteRequest, String)>>,
    reports: Mutex<Vec<RequestReport>>,
    recorded: Notify,
    bootstrap_retries: u8,
    permits: Option<Arc<Semaphore>>,
    headers_observed: Arc<Notify>,
    local_fallback: AtomicBool,
    fallback_refreshes: Mutex<Vec<bool>>,
}

fn trusted_client(root: &reqwest::Certificate) -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .use_rustls_tls()
        .http1_only()
        .tls_built_in_root_certs(false)
        .add_root_certificate(root.clone())
        .timeout(DEADLINE)
        .build()
        .unwrap()
}

impl Host for TlsHost {
    fn observe_rate_limits(
        &self,
        _: &str,
        _: super::rate_limits::QuotaSnapshot,
        _: std::time::Instant,
    ) {
        self.headers_observed.notify_one();
    }

    fn invalidate_transport(&self, identity: &str) -> bool {
        let mut current = self.generation.lock().unwrap();
        if identity != format!("tls-generation-{}", current.number) {
            return false;
        }
        self.invalidated.lock().unwrap().push(identity.to_owned());
        current.number += 1;
        current.client = trusted_client(&self.root);
        true
    }
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "tls-caller".into(),
                label: "Synthetic TLS client".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            // The bootstrap case deliberately allows only one live route lease.
            // Transport refresh must release its old lease before routing again.
            let permit = match &self.permits {
                Some(permits) => Some(Arc::new(permits.clone().acquire_owned().await.unwrap())
                    as Arc<dyn Send + Sync>),
                None => None,
            };
            let current = self.generation.lock().unwrap();
            let local_fallback = self.local_fallback.load(Ordering::SeqCst);
            let reselected_official = local_fallback && current.number > 0;
            let identity = format!("tls-generation-{}", current.number);
            self.routed
                .lock()
                .unwrap()
                .push((request.clone(), identity.clone()));
            Ok(Route {
                continuity_account_id: reselected_official.then(|| "chatgpt_plan_grant".into()),
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: reselected_official.then(|| "grant".into()),
                official_fallback_registration_id: (local_fallback && !reselected_official).then(|| "grant".into()),
                official_fallback_authorization_revision: (local_fallback && !reselected_official).then(|| "epoch".into()),
                authorization_scope_fingerprint: local_fallback.then(|| "scope".into()),
                official_fallback_failure: (local_fallback && !reselected_official).then(|| OfficialFallbackFailure {
                    trigger_stage: "health_pause", status: 0, code: "official_unavailable".into(), message: None, retry_after: None,
                }),
                transport_identity: Some(identity),
                account_id: "account-a".into(),
                owner_id: if reselected_official { "official-owner" } else { "owner-a" }.into(),
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: current.client.clone(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: matches!(self.mode, Mode::Codex) && !reselected_official,
                chatgpt_plan: matches!(self.mode, Mode::Plan) || reselected_official,
                wire_api: if matches!(self.mode, Mode::Chat) {
                    WireApi::Chat
                } else {
                    WireApi::Responses
                },
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit,
            })
        })
    }
    fn refresh_route<'a>(
        &'a self,
        caller: &'a Caller,
        previous: &'a Route,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            if previous.official_fallback_registration_id.is_none() {
                return self.route(caller, request).await;
            }
            self.fallback_refreshes.lock().unwrap().push(request.force_refresh);
            let mut refreshed = self.route(caller, request).await?;
            // Default selection has recovered to official. The refresh hook
            // retains this turn's local namespace while renewing its client.
            refreshed.continuity_account_id = previous.continuity_account_id.clone();
            refreshed.owner_id = previous.owner_id.clone();
            refreshed.official_registration_id = previous.official_registration_id.clone();
            refreshed.official_fallback_registration_id = previous.official_fallback_registration_id.clone();
            refreshed.official_fallback_authorization_revision = previous.official_fallback_authorization_revision.clone();
            refreshed.authorization_scope_fingerprint = previous.authorization_scope_fingerprint.clone();
            refreshed.official_fallback_failure = previous.official_fallback_failure.clone();
            refreshed.chatgpt = previous.chatgpt;
            refreshed.chatgpt_plan = previous.chatgpt_plan;
            Ok(refreshed)
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async move {
            Ok(ExecutionPolicy {
                open_timeout: DEADLINE,
                idle_timeout: DEADLINE,
                total_timeout: DEADLINE,
                // A visible stream must fence all replay paths despite retry budgets.
                credential_retries: 3,
                send_retries: 2,
                status_retries: 2,
                bootstrap_retries: self.bootstrap_retries,
                stream_open_max_attempts: 3,
                ..ExecutionPolicy::default()
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
            self.recorded.notify_one();
        })
    }
}

async fn read_request(reader: &mut (impl AsyncRead + Unpin)) -> (String, Value) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(reader.read_u8().await.unwrap());
        assert!(
            head.len() < 32 * 1024,
            "unexpected loopback request header size"
        );
    }
    let head = String::from_utf8(head).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .expect("JSON requests have a fixed content length");
    assert!(length < 256 * 1024);
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await.unwrap();
    (head, serde_json::from_slice(&body).unwrap())
}

fn response_events(healthy: bool) -> String {
    let id = if healthy {
        "resp_healthy"
    } else {
        "resp_corrupt"
    };
    let mut values = vec![
        json!({"type":"response.created","response":{"id":id,"object":"response","status":"in_progress","output":[]}}),
    ];
    if healthy {
        values.push(json!({"type":"response.completed","response":{"id":id,"object":"response","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":1}}}));
    } else {
        let item = json!({"type":"function_call","id":"fc_action","call_id":"call_action","name":"fixture_read","arguments":"{}","status":"completed"});
        values.push(
            json!({"type":"response.output_item.added","output_index":0,"item":item.clone()}),
        );
        values.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
    }
    values
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

fn chat_events(healthy: bool) -> String {
    let delta = if healthy {
        json!({"role":"assistant","content":"healthy"})
    } else {
        json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_action","type":"function","function":{"name":"fixture_read","arguments":"{}"}}]})
    };
    let first = json!({"id":"chat_tls","object":"chat.completion.chunk","model":"fixture","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let mut result = format!("data: {first}\n\n");
    if healthy {
        let done = json!({"id":"chat_tls","object":"chat.completion.chunk","model":"fixture","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
        result.push_str(&format!("data: {done}\n\ndata: [DONE]\n\n"));
    }
    result
}

struct Fixture {
    handle: Handle,
    host: Arc<TlsHost>,
    server: tokio::task::JoinHandle<()>,
    release: Arc<Notify>,
    requests: Arc<AtomicUsize>,
    path: std::path::PathBuf,
}

impl Fixture {
    async fn start(mode: Mode) -> Self {
        Self::start_with_phase(mode, false).await
    }

    async fn start_with_phase(mode: Mode, before_output: bool) -> Self {
        let certificate = rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
        let root = reqwest::Certificate::from_der(certificate.cert.der()).unwrap();
        let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der())
                .into(),
        )
        .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let release = Arc::new(Notify::new());
        let requests = Arc::new(AtomicUsize::new(0));
        let headers_observed = Arc::new(Notify::new());
        let server_headers_observed = headers_observed.clone();
        let server_release = release.clone();
        let server_requests = requests.clone();
        let server = tokio::spawn(async move {
            tokio::time::timeout(DEADLINE,async move {
                let acceptor=tokio_rustls::TlsAcceptor::from(Arc::new(config));
                for index in 0..2 {
                    let (socket,peer)=listener.accept().await.unwrap();
                    assert!(peer.ip().is_loopback());
                    let mut tls=acceptor.accept(socket).await.unwrap();
                    let (head,body)=read_request(&mut tls).await;
                    let endpoint=if matches!(mode,Mode::Chat) {"/v1/chat/completions"} else {"/v1/responses"};
                    assert!(head.starts_with(&format!("POST {endpoint} HTTP/1.1")),"{head}");
                    assert_eq!(body["model"],"fixture");
                    server_requests.fetch_add(1,Ordering::SeqCst);
                    let healthy=index==1;
                    let events=if matches!(mode,Mode::Chat) {chat_events(healthy)} else {response_events(healthy)};
                    let header=b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nx-codex-primary-used-percent: 1\r\n\r\n";
                    tls.write_all(header).await.unwrap();
                    if healthy || !before_output {
                        tls.write_all(format!("{:x}\r\n{events}\r\n",events.len()).as_bytes()).await.unwrap();
                    }
                    tls.flush().await.unwrap();
                    if healthy {
                        tls.write_all(b"0\r\n\r\n").await.unwrap();
                        tls.shutdown().await.unwrap();
                    } else {
                        // Wait for the actual gateway/client to expose a tool
                        // event, so bootstrap buffering cannot make this replayable.
                        if before_output {
                            // This hook runs after reqwest returned the actual
                            // HTTP response. Corruption therefore reaches the
                            // bootstrap body probe, not request.send().
                            server_headers_observed.notified().await;
                        } else {
                            server_release.notified().await;
                        }
                        // TLS 1.3 application-data record with a valid envelope
                        // and a deliberately invalid AEAD tag. This bypasses the
                        // server encoder only for this record, after real TLS.
                        let mut invalid_record=vec![0x17,0x03,0x03,0x00,0x20];
                        invalid_record.extend_from_slice(&[0;32]);
                        let (tcp,_)=tls.get_mut();
                        tcp.write_all(&invalid_record).await.unwrap();
                        tcp.flush().await.unwrap();
                        tcp.shutdown().await.unwrap();
                    }
                }
            }).await.expect("loopback TLS fixture deadline");
        });
        let host = Arc::new(TlsHost {
            mode,
            base: format!("https://{address}/v1"),
            generation: Mutex::new(Generation {
                number: 0,
                client: trusted_client(&root),
            }),
            root,
            invalidated: Mutex::new(Vec::new()),
            routed: Mutex::new(Vec::new()),
            reports: Mutex::new(Vec::new()),
            recorded: Notify::new(),
            bootstrap_retries: if before_output { 1 } else { 2 },
            permits: before_output.then(|| Arc::new(Semaphore::new(1))),
            headers_observed,
            local_fallback: AtomicBool::new(false),
            fallback_refreshes: Mutex::new(Vec::new()),
        });
        let path = std::env::temp_dir().join(format!(
            "native-tls-recovery-{}.sqlite",
            uuid::Uuid::now_v7()
        ));
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.clone(),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            server,
            release,
            requests,
            path,
        }
    }

    async fn reports(&self, count: usize) {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let ready = self.host.recorded.notified();
                if self.host.reports.lock().unwrap().len() >= count {
                    break;
                }
                ready.await;
            }
        })
        .await
        .expect("request report deadline");
    }

    async fn shutdown(mut self) {
        self.handle.shutdown(Duration::from_secs(3)).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.server)
            .await
            .unwrap()
            .unwrap();
        for path in [
            self.path.clone(),
            self.path.with_extension("sqlite-wal"),
            self.path.with_extension("sqlite-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}

async fn assert_tls_recovery(mode: Mode) {
    let fixture = Fixture::start(mode).await;
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(DEADLINE)
        .build()
        .unwrap();
    let url = format!("http://{}/v1/responses", fixture.handle.addr);
    let body = json!({"model":"fixture","stream":true,"input":[{"role":"user","content":"synthetic TLS test"}],"tools":[{"type":"function","name":"fixture_read","parameters":{"type":"object","properties":{}}}]});
    let first = client
        .post(&url)
        .bearer_auth("synthetic")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let mut stream = first.bytes_stream();
    let mut text = String::new();
    tokio::time::timeout(DEADLINE, async {
        while !(text.contains("response.output_item.added") && text.contains("call_action")) {
            let chunk = stream
                .next()
                .await
                .expect("upstream must expose a tool item before corruption")
                .unwrap();
            text.push_str(&String::from_utf8_lossy(&chunk));
        }
    })
    .await
    .expect("visible tool event deadline");
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 1);
    assert!(fixture.host.invalidated.lock().unwrap().is_empty());
    fixture.release.notify_one();
    tokio::time::timeout(DEADLINE, async {
        while let Some(chunk) = stream.next().await {
            text.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
    })
    .await
    .expect("corrupt TLS stream must terminate");
    fixture.reports(1).await;
    assert!(
        !text.contains("response.completed"),
        "a partial tool stream must not be replayed: {text}"
    );
    assert!(
        text.contains("upstream_stream_error"),
        "transport error code must remain visible: {text}"
    );
    assert_eq!(
        *fixture.host.invalidated.lock().unwrap(),
        vec!["tls-generation-0"]
    );
    assert_eq!(
        fixture.requests.load(Ordering::SeqCst),
        1,
        "no automatic upstream replay after output"
    );
    {
        let reports = fixture.host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].success);
        assert_eq!(reports[0].status, 502);
        assert_eq!(reports[0].error.as_deref(), Some("upstream_stream_error"));
        assert!(
            !reports[0].credential_scope,
            "TLS corruption is not an OAuth failure"
        );
        let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].transport_error.as_deref(), Some("tls_decrypt"));
        assert_eq!(attempts[0].transport_pool_retired, Some(true));
        assert_eq!(
            attempts[0].upstream_http_version.as_deref(),
            Some("HTTP/1.1")
        );
    }
    let healthy = client
        .post(&url)
        .bearer_auth("synthetic")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(healthy.status(), 200);
    let healthy = healthy.text().await.unwrap();
    assert!(healthy.contains("response.completed"), "{healthy}");
    fixture.reports(2).await;
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 2);
    {
        let routes = fixture.host.routed.lock().unwrap();
        assert_eq!(
            routes.len(),
            2,
            "one selection for each independent client request"
        );
        assert_eq!(routes[0].1, "tls-generation-0");
        assert_eq!(routes[1].1, "tls-generation-1");
        for (request, _) in routes.iter() {
            assert!(
                !request.force_refresh,
                "TLS errors must not refresh OAuth credentials"
            );
            assert!(
                request.excluded_account_ids.is_empty(),
                "TLS errors must not fail over credentials"
            );
        }
    }
    {
        let reports = fixture.host.reports.lock().unwrap();
        assert!(reports[1].success);
        assert!(reports
            .iter()
            .all(|report| report.account_id == "account-a"));
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn tls_decrypt_after_native_tool_output_retires_only_transport_without_replaying() {
    assert_tls_recovery(Mode::Responses).await;
}

#[tokio::test]
async fn tls_decrypt_survives_plan_canonical_stream_and_next_request_recovers() {
    assert_tls_recovery(Mode::Plan).await;
}

#[tokio::test]
async fn tls_decrypt_survives_chat_canonical_stream_and_next_request_recovers() {
    assert_tls_recovery(Mode::Chat).await;
}

async fn assert_bootstrap_tls_recovery(local_fallback: bool) {
    let fixture = Fixture::start_with_phase(Mode::Codex, true).await;
    fixture.host.local_fallback.store(local_fallback, Ordering::SeqCst);
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(DEADLINE)
        .build()
        .unwrap();
    let response = client
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("synthetic")
        .json(&json!({"model":"fixture","stream":true,"input":[{"role":"user","content":"synthetic bootstrap TLS test"}]}))
        .send()
        .await
        .expect("bootstrap retry must release the only old route permit");
    assert_eq!(response.status(), 200);
    let output = response.text().await.unwrap();
    assert!(output.contains("response.completed"), "{output}");
    assert!(!output.contains("upstream_stream_error"), "{output}");
    fixture.reports(1).await;
    assert_eq!(fixture.requests.load(Ordering::SeqCst), 2);
    assert_eq!(
        *fixture.host.invalidated.lock().unwrap(),
        vec!["tls-generation-0"]
    );
    {
        let routes = fixture.host.routed.lock().unwrap();
        assert_eq!(routes.len(), 2, "one bootstrap retry with a fresh client");
        assert_eq!(routes[0].1, "tls-generation-0");
        assert_eq!(routes[1].1, "tls-generation-1");
        for (request, _) in routes.iter() {
            assert!(
                !request.force_refresh,
                "TLS recovery must not refresh tokens"
            );
            assert!(request.excluded_account_ids.is_empty());
            assert_eq!(request.model, "fixture");
        }
    }
    {
        let reports = fixture.host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].success);
        assert_eq!(reports[0].account_id, "account-a");
        assert_eq!(reports[0].official_fallback_registration_id.as_deref(), local_fallback.then_some("grant"));
        assert!(!reports[0].credential_scope);
        let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 2);
        assert!(attempts
            .iter()
            .all(|attempt| attempt.account_id == "account-a"));
        assert_eq!(attempts[0].transport_error.as_deref(), Some("tls_decrypt"));
        assert_eq!(attempts[0].transport_pool_retired, Some(true));
        assert_eq!(
            attempts[0].upstream_http_version.as_deref(),
            Some("HTTP/1.1")
        );
        assert!(!attempts[0].success);
        assert!(attempts[1].success);
        assert!(attempts[1].transport_error.is_none());
        assert!(attempts[1].transport_pool_retired.is_none());
    }
    assert_eq!(
        fixture.host.permits.as_ref().unwrap().available_permits(),
        1
    );
    assert_eq!(*fixture.host.fallback_refreshes.lock().unwrap(), if local_fallback { vec![false] } else { vec![] });
    fixture.shutdown().await;
}

#[tokio::test]
async fn tls_decrypt_before_first_event_refreshes_same_transport_with_single_route_permit() {
    assert_bootstrap_tls_recovery(false).await;
}

#[tokio::test]
async fn official_fallback_tls_decrypt_refresh_keeps_local_namespace_when_official_recovers() {
    assert_bootstrap_tls_recovery(true).await;
}
