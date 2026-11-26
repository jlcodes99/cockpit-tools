//! Exercise retry preparation through the real HTTP gateway and zstd path.
//! The ChatGPT hostname is resolved only to this test's loopback TLS listener;
//! certificates, credentials, accounts, payloads and identity databases are synthetic.
use super::*;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

const OPEN: Duration = Duration::from_secs(1);
const WATCHDOG: Duration = Duration::from_secs(12);

struct ScopeHost {
    inner: Arc<TestHost>,
    client: reqwest::Client,
    url: String,
    policy: ExecutionPolicy,
    delayed_reasoning: AtomicBool,
    reasoning_calls: std::sync::atomic::AtomicUsize,
}

impl Host for ScopeHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(key)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
    fn reasoning_ceiling<'a>(
        &'a self,
        caller: &'a Caller,
    ) -> BoxFuture<
        'a,
        Result<Option<crate::models::codex_local_access::CodexReasoningLevel>, ServiceError>,
    > {
        Box::pin(async move {
            if self.delayed_reasoning.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            self.reasoning_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.reasoning_ceiling(caller).await
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let account = ["account-a", "account-b"]
                .into_iter()
                .find(|id| {
                    !request
                        .excluded_account_ids
                        .iter()
                        .any(|excluded| excluded.as_str() == *id)
                        && request
                            .preferred_account_id
                            .as_deref()
                            .is_none_or(|p| p == *id)
                })
                .ok_or_else(|| {
                    ServiceError::new(503, "no_available_account", "Synthetic pool exhausted")
                })?;
            let mut route = self.inner.route(caller, request).await?;
            route.account_id = account.into();
            route.owner_id = format!("owner-{account}");
            route.base_url = self.url.clone();
            route.client = self.client.clone();
            route.chatgpt = true;
            route
                .headers
                .insert("x-synthetic-account", HeaderValue::from_static(account));
            Ok(route)
        })
    }
}

#[derive(Clone, Debug)]
struct Captured {
    account: String,
    compressed: bool,
    wire_bytes: usize,
    plain_bytes: usize,
    body_hash: Vec<u8>,
    input_hash: Vec<u8>,
}

struct Fixture {
    handle: Handle,
    host: Arc<ScopeHost>,
    captures: Arc<Mutex<Vec<Captured>>>,
    arrived: Arc<tokio::sync::Notify>,
    upstream: JoinHandle<()>,
    _temp: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Dropping the listener task also drops its JoinSet, cancelling every
        // owned pending TLS/request task if a test assertion unwinds early.
        self.upstream.abort();
    }
}

impl Fixture {
    async fn start(policy: ExecutionPolicy, succeed_after_first: bool) -> Self {
        let cert = rcgen::generate_simple_self_signed(vec!["chatgpt.com".into()]).unwrap();
        let root = reqwest::Certificate::from_der(cert.cert.der()).unwrap();
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()).into(),
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
        let captures = Arc::new(Mutex::new(Vec::<Captured>::new()));
        let arrived = Arc::new(tokio::sync::Notify::new());
        let capture = captures.clone();
        let notify = arrived.clone();
        let upstream = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, peer) = accepted.unwrap();
                        assert!(peer.ip().is_loopback());
                        let tls = acceptor.clone();
                        let capture = capture.clone();
                        let notify = notify.clone();
                        connections.spawn(async move {
                            let socket = tls.accept(socket).await.unwrap();
                            let service = hyper::service::service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                                let capture = capture.clone();
                                let notify = notify.clone();
                                async move {
                                    assert_eq!(request.uri().path(), "/backend-api/codex/responses");
                                    let (parts, body) = request.into_parts();
                                    let bytes = body.collect().await.unwrap().to_bytes();
                                    let compressed = parts.headers.get("content-encoding").is_some_and(|v| v == "zstd");
                                    let plain = if compressed {
                                        zstd::stream::decode_all(bytes.as_ref()).unwrap()
                                    } else {
                                        bytes.to_vec()
                                    };
                                    let value: Value = serde_json::from_slice(&plain).unwrap();
                                    let entry = Captured {
                                        account: parts.headers["x-synthetic-account"].to_str().unwrap().into(),
                                        compressed,
                                        wire_bytes: bytes.len(),
                                        plain_bytes: plain.len(),
                                        body_hash: Sha256::digest(&plain).to_vec(),
                                        input_hash: Sha256::digest(serde_json::to_vec(&value["input"]).unwrap()).to_vec(),
                                    };
                                    let sequence = {
                                        let mut seen = capture.lock().unwrap();
                                        seen.push(entry);
                                        seen.len()
                                    };
                                    notify.notify_waiters();
                                    if sequence == 1 || !succeed_after_first {
                                        // Consume the entire real TLS request, but send no HTTP
                                        // headers. The native opening deadline must expire.
                                        std::future::pending::<()>().await;
                                    }
                                    Ok::<_, std::convert::Infallible>(Response::builder()
                                        .header("content-type", "text/event-stream")
                                        .body(Body::from(format!("{}{}", created(), completed())))
                                        .unwrap())
                                }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                                .await;
                        });
                    }
                    joined = connections.join_next(), if !connections.is_empty() => {
                        if let Some(result) = joined { result.unwrap(); }
                    }
                }
            }
        });
        let inner = Arc::new(TestHost {
            base_url: format!("http://{addr}/v1"),
            chatgpt: true,
            proxy_url: Mutex::new(None),
            ws_enabled: AtomicBool::new(false),
            key_enabled: AtomicBool::new(true),
            change_owner_on_refresh: AtomicBool::new(false),
            route_by_model: AtomicBool::new(false),
            bridge_with_policy: AtomicBool::new(false),
            reports: Mutex::new(Vec::new()),
            reasoning_ceiling: Mutex::new(None),
            rejections: Mutex::new(Vec::new()),
            diagnostics: Mutex::new(Vec::new()),
            selections: Mutex::new(Vec::new()),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        let host = Arc::new(ScopeHost {
            inner,
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .add_root_certificate(root)
                .resolve("chatgpt.com", addr)
                .build()
                .unwrap(),
            url: format!("https://chatgpt.com:{}/backend-api/codex", addr.port()),
            policy,
            delayed_reasoning: AtomicBool::new(false),
            reasoning_calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let temp = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            temp.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            captures,
            arrived,
            upstream,
            _temp: temp,
        }
    }

    async fn wait_captures(&self, count: usize) {
        tokio::time::timeout(WATCHDOG, async {
            loop {
                let next = self.arrived.notified();
                if self.captures.lock().unwrap().len() >= count {
                    break;
                }
                next.await;
            }
        })
        .await
        .expect("expected real loopback upstream attempts");
    }

    async fn finish(mut self) {
        self.handle.shutdown(Duration::from_secs(3)).await.unwrap();
        assert_eq!(self.handle.active_requests(), 0);
        assert_eq!(
            self.host.inner.capacity.available_permits(),
            1,
            "account permit must be released"
        );
        self.upstream.abort();
        match (&mut self.upstream).await {
            Err(error) if error.is_cancelled() => {}
            Err(error) => panic!("Synthetic upstream task failed: {error}"),
            Ok(()) => panic!("Synthetic upstream task ended before fixture cleanup"),
        }
    }
}

fn policy() -> ExecutionPolicy {
    ExecutionPolicy {
        open_timeout: OPEN,
        idle_timeout: Duration::from_secs(3),
        total_timeout: Duration::from_secs(8),
        stream_open_max_attempts: 2,
        credential_retries: 1,
        network_recovery_timeout: Duration::from_secs(5),
        network_recovery_max_failures: 8,
        send_retries: 0,
        status_retries: 0,
        bootstrap_retries: 0,
        ..Default::default()
    }
}

fn request_body(duplicate_images: bool) -> Value {
    // Deterministic, low-repetition ASCII makes compression non-trivial even
    // on fast machines. No randomness, external assets, or production history.
    let mut state = 0x1234_5678_u32;
    let text: String = (0..4 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            char::from(b'a' + (state % 26) as u8)
        })
        .collect();
    let mut content = vec![json!({"type":"input_text","text":text})];
    if duplicate_images {
        let image = json!({"type":"input_image","image_url":"data:image/png;base64,QUFBQQ=="});
        content.extend([image.clone(), image]);
    }
    json!({"model":"test-model","stream":false,"input":[{"role":"user","content":content}]})
}

async fn request(addr: SocketAddr, body: Value) -> (u16, Value) {
    tokio::time::timeout(WATCHDOG, async {
        let response = client()
            .post(format!("http://{addr}/v1/responses"))
            .bearer_auth("local-test-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap())
    })
    .await
    .expect("native request exceeded independent watchdog")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_same_account_recompresses_after_first_open_timeout() {
    let fixture = Fixture::start(policy(), true).await;
    let body = request_body(false);
    let input_hash = Sha256::digest(serde_json::to_vec(&body["input"]).unwrap()).to_vec();
    let started = tokio::time::Instant::now();
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    let selections = fixture.host.inner.selections.lock().unwrap().clone();
    fixture.finish().await;
    eprintln!(
        "synthetic retry_scope_same_account status={status} error_code={} captures={} selections={}",
        value["error"]["code"].as_str().unwrap_or("none"),
        seen.len(),
        selections.len()
    );
    assert!(
        !seen.is_empty(),
        "the first real upstream request must arrive"
    );
    assert!(
        seen.iter()
            .all(|r| r.compressed && r.wire_bytes < r.plain_bytes && r.input_hash == input_hash),
        "captured attempts must use production zstd and preserve synthetic input"
    );
    assert!(
        selections.len() >= 2,
        "retry route preparation must occur before evaluating the outcome"
    );
    if status != 200 {
        assert_eq!(
            status, 503,
            "unexpected failure before target assertion: {value}"
        );
        assert_eq!(
            value["error"]["code"], "blocking_operation_timeout",
            "RED must expose the stale preparation scope: {value}"
        );
        assert_eq!(
            seen.len(),
            1,
            "the stale scope must fail before a second upstream dispatch"
        );
    }
    assert_eq!(status, 200, "{value}");
    assert!(
        started.elapsed() >= OPEN,
        "first opening deadline must actually expire"
    );
    assert_eq!(seen.len(), 2);
    assert!(seen
        .iter()
        .all(|r| r.compressed && r.wire_bytes < r.plain_bytes));
    assert!(seen
        .iter()
        .all(|r| r.account == "account-a" && r.input_hash == input_hash));
    assert_eq!(
        seen[0].body_hash, seen[1].body_hash,
        "retry must preserve canonical body"
    );
    assert_eq!(
        selections.len(),
        2,
        "same-account transport must be freshly prepared"
    );
    assert!(selections[1].preferred_account_id.as_deref() == Some("account-a"));
    assert!(
        !selections[1].force_refresh,
        "network timeout must not force credential refresh"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_recompresses_after_account_replacement() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            stream_open_max_attempts: 1,
            credential_retries: 2,
            ..policy()
        },
        true,
    )
    .await;
    let body = request_body(false);
    let input_hash = Sha256::digest(serde_json::to_vec(&body["input"]).unwrap()).to_vec();
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    let selections = fixture.host.inner.selections.lock().unwrap().clone();
    fixture.finish().await;
    eprintln!(
        "synthetic retry_scope_account_replacement status={status} error_code={} captures={} selections={}",
        value["error"]["code"].as_str().unwrap_or("none"),
        seen.len(),
        selections.len()
    );
    assert!(
        !seen.is_empty(),
        "the first real upstream request must arrive"
    );
    assert!(
        seen.iter()
            .all(|r| r.compressed && r.wire_bytes < r.plain_bytes && r.input_hash == input_hash),
        "captured attempts must use production zstd and preserve synthetic input"
    );
    assert!(
        selections.len() >= 2,
        "replacement route preparation must occur before evaluating the outcome"
    );
    if status != 200 {
        assert_eq!(
            status, 503,
            "unexpected failure before target assertion: {value}"
        );
        assert_eq!(
            value["error"]["code"], "blocking_operation_timeout",
            "RED must expose the stale preparation scope: {value}"
        );
        assert_eq!(
            seen.len(),
            1,
            "the stale scope must fail before a second upstream dispatch"
        );
    }
    assert_eq!(status, 200, "{value}");
    assert_eq!(
        seen.iter().map(|r| r.account.as_str()).collect::<Vec<_>>(),
        vec!["account-a", "account-b"]
    );
    assert!(seen
        .iter()
        .all(|r| r.compressed && r.input_hash == input_hash));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_duplicate_images_keep_compression_disabled_and_input_intact() {
    let fixture = Fixture::start(policy(), true).await;
    let body = request_body(true);
    let input_hash = Sha256::digest(serde_json::to_vec(&body["input"]).unwrap()).to_vec();
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    fixture.finish().await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(seen.len(), 2);
    assert!(seen
        .iter()
        .all(|r| !r.compressed && r.wire_bytes == r.plain_bytes && r.input_hash == input_hash));
    assert_eq!(seen[0].body_hash, seen[1].body_hash);
}

fn guard_body(duplicate_images: bool) -> Value {
    let mut content =
        vec![json!({"type":"input_text","text":"synthetic guard history ".repeat(8192)})];
    if duplicate_images {
        let image = json!({"type":"input_image","image_url":"data:image/png;base64,QUFBQQ=="});
        content.extend([image.clone(), image]);
    }
    json!({"model":"test-model","stream":false,"input":[{"role":"user","content":content}]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_compaction_capsule_expands_again_after_open_timeout() {
    let fixture = Fixture::start(policy(), true).await;
    let summary = "Synthetic retained summary, preserving the user's current plan. ".repeat(8192);
    let capsule = super::super::anthropic_compact::seal(&summary, "test-model").unwrap();
    let mut body = guard_body(true);
    let history = body["input"].as_array_mut().unwrap();
    history.insert(0, json!({"type":"compaction","id":"cmp_ag_compact_scope_fixture","encrypted_content":capsule}));
    let mut expected = body["input"].clone();
    expected[0] = json!({"type":"message","role":"developer","content":[{
        "type":"input_text","text":format!("Context summary from previous turns:\n{summary}")
    }]});
    let input_hash = Sha256::digest(serde_json::to_vec(&expected).unwrap()).to_vec();
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    fixture.finish().await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter()
            .all(|r| !r.compressed && r.input_hash == input_hash),
        "both attempts must unseal the capsule without altering retained history"
    );
    assert_eq!(seen[0].body_hash, seen[1].body_hash);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_async_reasoning_policy_check_uses_new_attempt_budget() {
    use crate::models::codex_local_access::CodexReasoningLevel as Level;
    let fixture = Fixture::start(policy(), true).await;
    fixture.host.delayed_reasoning.store(true, Ordering::SeqCst);
    *fixture.host.inner.reasoning_ceiling.lock().unwrap() = Some(Level::Medium);
    // Duplicate inline images bypass zstd, so this case isolates the actual
    // async reasoning policy lookup from the compression regression.
    let mut body = guard_body(true);
    body["reasoning"] = json!({"effort":"low"});
    let input_hash = Sha256::digest(serde_json::to_vec(&body["input"]).unwrap()).to_vec();
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    let checks = fixture.host.reasoning_calls.load(Ordering::SeqCst);
    fixture.finish().await;
    assert_eq!(status, 200, "{value}");
    assert_eq!(seen.len(), 2);
    assert!(seen
        .iter()
        .all(|r| !r.compressed && r.input_hash == input_hash));
    assert_eq!(checks, 2, "both real dispatch policy lookups must complete");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_renewal_does_not_extend_total_deadline() {
    let total = Duration::from_millis(1750);
    let fixture = Fixture::start(
        ExecutionPolicy {
            total_timeout: total,
            stream_open_max_attempts: 8,
            ..policy()
        },
        false,
    )
    .await;
    let body = guard_body(false);
    let started = tokio::time::Instant::now();
    let (status, value) = request(fixture.handle.addr, body).await;
    let elapsed = started.elapsed();
    let seen = fixture.captures.lock().unwrap().clone();
    fixture.finish().await;
    assert_eq!(status, 504, "{value}");
    assert_eq!(value["error"]["code"], "upstream_total_timeout", "{value}");
    assert_eq!(
        seen.len(),
        2,
        "a second compressed request must start inside the original total budget"
    );
    assert!(seen.iter().all(|r| r.compressed));
    assert!(elapsed >= total);
    assert!(
        elapsed < total + OPEN,
        "renewing the opening deadline must not renew the whole request: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_renewal_preserves_armed_network_recovery_deadline() {
    let recovery = Duration::from_millis(700);
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_timeout: recovery,
            stream_open_max_attempts: 8,
            ..policy()
        },
        false,
    )
    .await;
    let body = guard_body(false);
    let started = tokio::time::Instant::now();
    let (status, value) = request(fixture.handle.addr, body).await;
    let elapsed = started.elapsed();
    let seen = fixture.captures.lock().unwrap().clone();
    fixture.finish().await;
    assert_eq!(status, 504, "{value}");
    assert_eq!(
        value["error"]["code"], "network_recovery_exhausted",
        "{value}"
    );
    assert_eq!(value["error"]["transport_failures"], 1, "{value}");
    assert_eq!(
        seen.len(),
        2,
        "watcher must interrupt the second real compressed request"
    );
    assert!(seen.iter().all(|r| r.compressed));
    assert!(elapsed >= OPEN + recovery);
    assert!(
        elapsed < OPEN + recovery + Duration::from_millis(700),
        "fresh preparation must not reset continuous recovery: {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_renewal_preserves_network_failure_cap() {
    let fixture = Fixture::start(
        ExecutionPolicy {
            network_recovery_max_failures: 2,
            stream_open_max_attempts: 8,
            ..policy()
        },
        false,
    )
    .await;
    let body = guard_body(false);
    let (status, value) = request(fixture.handle.addr, body).await;
    let seen = fixture.captures.lock().unwrap().clone();
    fixture.finish().await;
    assert_eq!(status, 504, "{value}");
    assert_eq!(
        value["error"]["code"], "network_recovery_exhausted",
        "{value}"
    );
    assert_eq!(value["error"]["transport_failures"], 2, "{value}");
    assert_eq!(
        seen.len(),
        2,
        "fresh opening scopes must not refill the failure allowance"
    );
    assert!(seen.iter().all(|r| r.compressed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_scope_shutdown_cancels_second_attempt_and_releases_account_permit() {
    let mut fixture = Fixture::start(
        ExecutionPolicy {
            stream_open_max_attempts: 8,
            ..policy()
        },
        false,
    )
    .await;
    let body = guard_body(false);
    let addr = fixture.handle.addr;
    let pending = tokio::spawn(async move {
        client()
            .post(format!("http://{addr}/v1/responses"))
            .bearer_auth("local-test-key")
            .json(&body)
            .send()
            .await
    });
    fixture.wait_captures(2).await;
    let started = tokio::time::Instant::now();
    fixture
        .handle
        .shutdown(Duration::from_secs(3))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(3), pending)
        .await
        .unwrap()
        .unwrap();
    match response {
        Ok(response) => {
            let status = response.status().as_u16();
            let value: Value = response.json().await.unwrap();
            assert_eq!(status, 503, "{value}");
            assert_eq!(value["error"]["code"], "service_stopping", "{value}");
        }
        Err(error) => {
            assert!(
                error.is_request() || error.is_body(),
                "only a shutdown transport close is acceptable: {error}"
            );
            assert!(
                !error.is_timeout() && !error.is_builder(),
                "shutdown must complete promptly: {error}"
            );
        }
    }
    assert_eq!(fixture.handle.active_requests(), 0);
    assert_eq!(fixture.host.inner.capacity.available_permits(), 1);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(
        fixture.captures.lock().unwrap().len(),
        2,
        "shutdown must not launch another retry"
    );
    fixture.finish().await;
}
