//! Synthetic loopback regression: control metadata is already public before a
//! transport disconnect. No production credentials, images or history are used.
use super::{contract::*, start, Handle, Options};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::Response,
    routing::post,
    Router,
};
use futures_util::{future::BoxFuture, stream, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

const HOLD: Duration = Duration::from_millis(350);

struct Source {
    requests: Mutex<Vec<Vec<u8>>>,
    suffix: String,
    fail_again: bool,
    clean_eof: AtomicBool,
    delay_ms: AtomicU64,
    recovery_failure: Mutex<Option<(u16, Value)>>,
    early_failure: AtomicBool,
    concurrent: AtomicBool,
    arrivals: AtomicUsize,
    started: Notify,
    successful_output: Mutex<Value>,
}

async fn upstream(State(source): State<Arc<Source>>, bytes: Bytes) -> Response {
    let attempt = {
        let mut requests = source.requests.lock().unwrap();
        requests.push(bytes.to_vec());
        requests.len()
    };
    let id = format!("resp_fixture_{attempt}");
    if source.concurrent.load(Ordering::SeqCst) {
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let name = body["input"][0]["content"].as_str().unwrap();
        let occurrence = source
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|bytes| {
                let request: Value = serde_json::from_slice(bytes).unwrap();
                request["input"][0]["content"] == name
            })
            .count();
        if occurrence == 1 {
            source.arrivals.fetch_add(1, Ordering::SeqCst);
            loop {
                let ready = source.started.notified();
                if source.arrivals.load(Ordering::SeqCst) >= 3 {
                    source.started.notify_waiters();
                    break;
                }
                ready.await;
            }
        }
        let prefix = format!(
            "data: {}\n\n",
            json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}})
        );
        let should_fail = name == "fail" && occurrence == 1;
        let ending = if should_fail {
            None
        } else {
            Some(format!(
                "data: {}\n\n",
                json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[],"usage":{"output_tokens":1}}})
            ))
        };
        let stream = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(prefix)) })
            .chain(stream::once(async move {
                tokio::time::sleep(if should_fail {
                    HOLD
                } else {
                    Duration::from_millis(700)
                })
                .await;
                match ending {
                    Some(value) => Ok(Bytes::from(value)),
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "one concurrent fixture failed",
                    )),
                }
            }));
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(stream))
            .unwrap();
    }
    if attempt == 1 && source.early_failure.load(Ordering::SeqCst) {
        let stream = stream::once(async {
            Ok::<_, std::io::Error>(Bytes::from_static(b": pre-event comment\n\n"))
        })
        .chain(stream::once(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "synthetic bootstrap disconnect",
            ))
        }));
        return Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(stream))
            .unwrap();
    }
    let created = format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}})
    );
    if attempt > 1 {
        if let Some((status, error)) = source.recovery_failure.lock().unwrap().clone() {
            let (content_type, body) = if status == 200 {
                ("text/event-stream", format!("{created}data: {error}\n\n"))
            } else {
                ("application/json", error.to_string())
            };
            return Response::builder()
                .status(status)
                .header("content-type", content_type)
                .body(Body::from(body))
                .unwrap();
        }
    }
    if attempt > 1 && !source.fail_again {
        let output = source.successful_output.lock().unwrap().clone();
        return Response::builder().header("content-type", "text/event-stream")
            .body(Body::from(format!("{created}data: {}\n\n", json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output,"usage":{"output_tokens":1}}})))).unwrap();
    }
    let suffix = source.suffix.clone();
    let delay = Duration::from_millis(source.delay_ms.load(Ordering::SeqCst));
    let clean_eof = source.clean_eof.load(Ordering::SeqCst);
    let body = stream::once(async move { Ok::<_, std::io::Error>(Bytes::from(created)) })
        .chain(stream::once(async move {
            tokio::time::sleep(delay).await;
            Ok(Bytes::from(suffix))
        }))
        .chain(
            stream::once(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                (!clean_eof).then(|| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "synthetic body disconnect",
                    ))
                })
            })
            .filter_map(futures_util::future::ready),
        );
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(body))
        .unwrap()
}

struct TestHost {
    base: String,
    routes: Mutex<Vec<RouteRequest>>,
    reports: Mutex<Vec<RequestReport>>,
    policy: ExecutionPolicy,
    change_owner: bool,
    change_headers: AtomicBool,
    chatgpt: bool,
    permits: Arc<Semaphore>,
    block_refresh: AtomicBool,
    refresh_entered: Notify,
    refresh_cancelled: AtomicBool,
    client: Mutex<(usize, reqwest::Client)>,
    retired: Mutex<Vec<String>>,
    optimize: AtomicBool,
}

struct Cancelled<'a>(&'a AtomicBool);
impl Drop for Cancelled<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Host for TestHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "fixture".into(),
                label: "synthetic".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let permit = self.permits.clone().acquire_owned().await.unwrap();
            let (refresh, changed, changed_headers) = {
                let mut routes = self.routes.lock().unwrap();
                routes.push(request.clone());
                (
                    routes.len() > 1,
                    self.change_owner && routes.len() > 1,
                    self.change_headers.load(Ordering::SeqCst) && routes.len() > 1,
                )
            };
            if refresh && self.block_refresh.load(Ordering::SeqCst) {
                let _cancelled = Cancelled(&self.refresh_cancelled);
                self.refresh_entered.notify_one();
                futures_util::future::pending::<()>().await;
            }
            let mut headers = HeaderMap::new();
            if changed_headers {
                headers.insert("authorization", "Bearer synthetic-changed".parse().unwrap());
            }
            let (generation, client) = self.client.lock().unwrap().clone();
            Ok(Route {
                account_id: "account-a".into(),
                continuity_account_id: None,
                owner_id: if changed { "owner-b" } else { "owner-a" }.into(),
                installation_profile: None,
                transport_identity: Some(format!("fixture-pool-{generation}")),
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                base_url: self.base.clone(),
                model: request.model,
                headers,
                client,
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.chatgpt,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: Some(Arc::new(permit)),
            })
        })
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        let mut client = self.client.lock().unwrap();
        if identity != format!("fixture-pool-{}", client.0) {
            return false;
        }
        self.retired.lock().unwrap().push(identity.to_owned());
        *client = (
            client.0 + 1,
            reqwest::Client::builder().no_proxy().build().unwrap(),
        );
        true
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn multi_agent_policy<'a>(
        &'a self,
        _: &'a Caller,
        _: Option<&'a Route>,
        _: &'a str,
    ) -> BoxFuture<'a, Result<MultiAgentPolicy, ServiceError>> {
        Box::pin(async {
            Ok(MultiAgentPolicy {
                options: MultiAgentOptions {
                    optimize: self.optimize.load(Ordering::SeqCst),
                    ..Default::default()
                },
                ..Default::default()
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Fixture {
    _directory: tempfile::TempDir,
    handle: Handle,
    host: Arc<TestHost>,
    source: Arc<Source>,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new(
        suffix: String,
        fail_again: bool,
        policy: ExecutionPolicy,
        change_owner: bool,
        chatgpt: bool,
    ) -> Self {
        let source = Arc::new(Source {
            requests: Mutex::new(vec![]),
            suffix,
            fail_again,
            clean_eof: AtomicBool::new(false),
            delay_ms: AtomicU64::new(HOLD.as_millis() as u64),
            recovery_failure: Mutex::new(None),
            early_failure: AtomicBool::new(false),
            concurrent: AtomicBool::new(false),
            arrivals: AtomicUsize::new(0),
            started: Notify::new(),
            successful_output: Mutex::new(json!([])),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(TestHost {
            base: format!("http://{}/v1", listener.local_addr().unwrap()),
            routes: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
            policy,
            change_owner,
            change_headers: AtomicBool::new(false),
            chatgpt,
            permits: Arc::new(Semaphore::new(1)),
            block_refresh: AtomicBool::new(false),
            refresh_entered: Notify::new(),
            refresh_cancelled: AtomicBool::new(false),
            client: Mutex::new((0, reqwest::Client::builder().no_proxy().build().unwrap())),
            retired: Mutex::new(vec![]),
            optimize: AtomicBool::new(false),
        });
        let app = Router::new()
            .route("/v1/responses", post(upstream))
            .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024))
            .with_state(source.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let directory = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            directory.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            _directory: directory,
            handle,
            host,
            source,
            task,
        }
    }
    async fn request(&self, body: &Value) -> String {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth("fixture")
            .header("user-agent", "codex_cli_rs/0.171.0")
            .json(body)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }
    async fn shutdown(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        self.task.abort();
    }
}

fn policy() -> ExecutionPolicy {
    ExecutionPolicy {
        bootstrap_retries: 0,
        send_retries: 0,
        status_retries: 0,
        credential_retries: 1,
        open_timeout: Duration::from_secs(3),
        idle_timeout: Duration::from_secs(3),
        total_timeout: Duration::from_secs(8),
        retry_base_delay: Duration::from_millis(1),
        ..Default::default()
    }
}

fn body() -> Value {
    json!({"model":"fixture","stream":true,"input":[{"role":"user","content":"synthetic analysis"}],
        "tools":[{"type":"function","name":"fixture_read","parameters":{"type":"object","properties":{}}},
            {"type":"namespace","name":"fixture","tools":[{"type":"custom","name":"inspect"}]}]})
}

#[tokio::test]
async fn published_metadata_disconnect_replays_once_with_identical_large_input() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    let mut request = body();
    // An opaque synthetic data URL tests preservation/large upload, not image inference.
    let image = format!("data:image/png;base64,{}", "QUJD".repeat(2 * 1024 * 1024));
    request["input"] = json!([
        {"role":"user","content":[{"type":"input_image","image_url":image,"detail":"original"}]},
        {"type":"function_call","name":"fixture_read","call_id":"synthetic_call","arguments":"{}"},
        {"type":"function_call_output","call_id":"synthetic_call","output":"synthetic_result"},
        {"role":"user","content":"continue without changing the frame"}
    ]);
    let output = fixture.request(&request).await;
    assert!(
        output.contains("response.completed"),
        "metadata-only disconnect should recover: {output}"
    );
    assert!(!output.contains("response.failed"), "{output}");
    {
        let requests = fixture.source.requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "one bounded replay");
        assert_eq!(
            requests[0], requests[1],
            "complete upstream body is byte-identical"
        );
        let first: Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(
            first["input"], request["input"],
            "image, order, tool IDs/results stay intact"
        );
    }
    {
        let routes = fixture.host.routes.lock().unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[1].preferred_account_id.as_deref(), Some("account-a"));
        assert!(routes
            .iter()
            .all(|request| !request.force_refresh && request.excluded_account_ids.is_empty()));
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_recovery_never_replenishes_after_second_disconnect() {
    let fixture = Fixture::new(String::new(), true, policy(), false, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("upstream_stream_error"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 2);
    assert_eq!(output.matches("event: response.failed").count(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_recovery_refuses_any_business_or_partial_frame() {
    let events = [
        json!({"type":"response.output_text.delta","delta":"partial"}),
        json!({"type":"response.reasoning_text.delta","delta":"reason"}),
        json!({"type":"response.function_call_arguments.delta","delta":"{"}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","id":"item","call_id":"call","name":"fixture_read","arguments":""}}),
        json!({"type":"response.output_item.added","item":{"type":"message","content":[]}}),
        json!({"type":"response.unknown_metadata"}),
    ];
    let mut suffixes: Vec<String> = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    suffixes
        .push("data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{".into());
    suffixes.push("event: response.function_call_arguments.delta\ndata: {\"type\":\"response.created\",\"response\":{\"output\":[]}}\n\n".into());
    suffixes.push("event: response.output_item.added\n\n".into());
    for suffix in suffixes {
        let fixture = Fixture::new(suffix.clone(), false, policy(), false, true).await;
        let output = fixture.request(&body()).await;
        assert!(!output.contains("response.completed"), "{suffix}: {output}");
        assert_eq!(fixture.source.requests.lock().unwrap().len(), 1, "{suffix}");
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn published_metadata_recovery_refuses_hosted_unknown_and_non_native_tools() {
    for tool in [
        json!({"type":"image_generation"}),
        json!({"type":"mcp","server_url":"https://example.invalid"}),
        json!({"type":"code_interpreter"}),
        json!({"type":"unknown"}),
        json!({"type":"function","name":"fixture_read","execution":"server"}),
        json!({"type":"namespace","name":"unsafe","tools":[{"type":"web_search"}]}),
    ] {
        let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
        let mut request = body();
        request["tools"] = json!([tool]);
        let output = fixture.request(&request).await;
        assert!(!output.contains("response.completed"), "{output}");
        assert!(fixture.source.requests.lock().unwrap().len() <= 1);
        fixture.shutdown().await;
    }
    let fixture = Fixture::new(String::new(), false, policy(), false, false).await;
    fixture.request(&body()).await;
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_recovery_preserves_total_network_and_owner_fences() {
    let mut limited = policy();
    limited.network_recovery_max_failures = 1;
    let fixture = Fixture::new(String::new(), false, limited, false, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("network_recovery_exhausted"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
    let mut limited = policy();
    limited.network_recovery_timeout = Duration::from_millis(30);
    limited.retry_base_delay = Duration::from_millis(200);
    let fixture = Fixture::new(String::new(), false, limited, false, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("network_recovery_exhausted"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
    let mut limited = policy();
    limited.total_timeout = Duration::from_millis(300);
    let fixture = Fixture::new(String::new(), false, limited, false, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("upstream_total_timeout"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
    let fixture = Fixture::new(String::new(), false, policy(), true, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("refresh_owner_changed"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_recovery_does_not_retry_its_http_or_sse_rejection() {
    for (status, error) in [
        (
            401,
            json!({"error":{"code":"invalid_api_key","message":"fixture unauthorized"}}),
        ),
        (
            503,
            json!({"error":{"code":"server_is_overloaded","message":"fixture overload"}}),
        ),
        (
            200,
            json!({"type":"response.failed","response":{"id":"resp_fixture_2","status":"failed","output":[],"error":{"code":"server_is_overloaded","message":"fixture overload"}}}),
        ),
    ] {
        let mut generous = policy();
        generous.status_retries = 2;
        generous.send_retries = 2;
        generous.bootstrap_retries = 2;
        let fixture = Fixture::new(String::new(), false, generous, false, true).await;
        *fixture.source.recovery_failure.lock().unwrap() = Some((status, error));
        let output = fixture.request(&body()).await;
        assert!(output.contains("response.failed"), "{output}");
        assert_eq!(fixture.source.requests.lock().unwrap().len(), 2);
        assert_eq!(
            fixture.host.routes.lock().unwrap().len(),
            2,
            "no OAuth/account retry after recovery"
        );
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn published_metadata_eof_idle_timeout_and_completed_response_are_not_replayed() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    fixture.source.clean_eof.store(true, Ordering::SeqCst);
    let output = fixture.request(&body()).await;
    assert!(output.contains("incomplete_upstream_stream"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
    let mut limited = policy();
    limited.idle_timeout = Duration::from_millis(300);
    let fixture = Fixture::new(String::new(), false, limited, false, true).await;
    fixture.source.delay_ms.store(800, Ordering::SeqCst);
    let output = fixture.request(&body()).await;
    assert!(output.contains("upstream_idle_timeout"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
    let terminal = format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":{"id":"resp_fixture_1","status":"completed","output":[],"usage":{"output_tokens":1}}})
    );
    let fixture = Fixture::new(terminal, false, policy(), false, true).await;
    let output = fixture.request(&body()).await;
    assert!(output.contains("response.completed"), "{output}");
    assert!(!output.contains("response.failed"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_client_disconnect_drops_the_recovery_future() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("fixture")
        .header("user-agent", "codex_cli_rs/0.171.0")
        .json(&body())
        .send()
        .await
        .unwrap();
    let mut bytes = response.bytes_stream();
    let first = bytes.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("response.created"));
    drop(bytes);
    tokio::time::sleep(HOLD + Duration::from_millis(100)).await;
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn published_metadata_recovery_renews_only_opening_and_keeps_authorization() {
    let mut short_open = policy();
    short_open.open_timeout = Duration::from_millis(100);
    let fixture = Fixture::new(String::new(), false, short_open, false, true).await;
    let output = fixture.request(&body()).await;
    assert!(
        output.contains("response.completed"),
        "the original opening already expired: {output}"
    );
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 2);
    fixture.shutdown().await;
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    fixture.host.change_headers.store(true, Ordering::SeqCst);
    let output = fixture.request(&body()).await;
    assert!(output.contains("transport_route_changed"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn metadata_disconnect_nonstreaming_behavior_is_unchanged() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    let mut request = body();
    request["stream"] = json!(false);
    let output = fixture.request(&request).await;
    assert!(output.contains("upstream_stream_error"), "{output}");
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn cancelling_an_entered_recovery_releases_its_only_route_permit() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    fixture.host.block_refresh.store(true, Ordering::SeqCst);
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth("fixture")
        .header("user-agent", "codex_cli_rs/0.171.0")
        .json(&body())
        .send()
        .await
        .unwrap();
    let mut bytes = response.bytes_stream();
    assert!(bytes.next().await.unwrap().is_ok());
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.host.refresh_entered.notified(),
    )
    .await
    .unwrap();
    drop(bytes);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !fixture.host.refresh_cancelled.load(Ordering::SeqCst)
            || fixture.host.permits.available_permits() != 1
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.source.requests.lock().unwrap().len(), 1);
    fixture.shutdown().await;
}

#[tokio::test]
async fn existing_bootstrap_retry_cannot_rearm_metadata_body_recovery() {
    let mut bootstrap = policy();
    bootstrap.bootstrap_retries = 1;
    let fixture = Fixture::new(String::new(), true, bootstrap, false, true).await;
    fixture.source.early_failure.store(true, Ordering::SeqCst);
    let output = fixture.request(&body()).await;
    assert!(output.contains("upstream_stream_error"), "{output}");
    assert_eq!(
        fixture.source.requests.lock().unwrap().len(),
        2,
        "the new layer cannot add a third dispatch"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn one_concurrent_stream_recovers_without_interrupting_other_shared_client_streams() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    fixture.host.permits.add_permits(2);
    fixture.source.concurrent.store(true, Ordering::SeqCst);
    let requests: Vec<Value> = ["fail", "healthy-a", "healthy-b"]
        .into_iter()
        .map(|name| {
            let mut request = body();
            request["input"][0]["content"] = json!(name);
            request
        })
        .collect();
    let outputs =
        futures_util::future::join_all(requests.iter().map(|body| fixture.request(body))).await;
    assert!(
        outputs
            .iter()
            .all(|output| output.contains("response.completed")
                && !output.contains("response.failed")),
        "{outputs:?}"
    );
    {
        let seen = fixture.source.requests.lock().unwrap();
        assert_eq!(seen.len(), 4);
        for name in ["fail", "healthy-a", "healthy-b"] {
            let matching: Vec<_> = seen
                .iter()
                .filter(|bytes| {
                    let request: Value = serde_json::from_slice(bytes).unwrap();
                    request["input"][0]["content"] == name
                })
                .collect();
            assert_eq!(matching.len(), if name == "fail" { 2 } else { 1 }, "{name}");
            if matching.len() == 2 {
                assert_eq!(matching[0], matching[1]);
            }
        }
    }
    assert_eq!(
        *fixture.host.retired.lock().unwrap(),
        vec!["fixture-pool-0"]
    );
    assert_eq!(fixture.host.permits.available_permits(), 3);
    fixture.shutdown().await;
}

fn namespace_body() -> Value {
    let mut request = body();
    request["tools"] = json!([{"type":"namespace","name":"collaboration","tools":[
        {"type":"function","name":"spawn_agent","parameters":{"type":"object","properties":{"message":{"type":"string"}}}}
    ]}]);
    request
}

#[tokio::test]
async fn native_namespace_conversion_recovers_with_the_frozen_restore_context() {
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    fixture.host.optimize.store(true, Ordering::SeqCst);
    *fixture.source.successful_output.lock().unwrap() = json!([{
        "type":"function_call","namespace":"collaboration-optimize","name":"spawn_agent",
        "call_id":"synthetic_call","arguments":"{\"message\":\"synthetic agent task\"}"
    }]);
    let output = fixture.request(&namespace_body()).await;
    let terminal = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .expect(&output);
    let call = &terminal["response"]["output"][0];
    assert_eq!(call["namespace"], "collaboration");
    assert_eq!(call["encrypted_function_args"], json!([]));
    assert_eq!(call["call_id"], "synthetic_call");
    assert_eq!(call["arguments"], "{\"message\":\"synthetic agent task\"}");
    {
        let requests = fixture.source.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0], requests[1]);
        let wire: Value = serde_json::from_slice(&requests[0]).unwrap();
        assert_eq!(wire["tools"][0]["name"], "collaboration-optimize");
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn native_conversion_cannot_hide_partial_or_conflicting_raw_business_frames() {
    for suffix in [
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{",
        "event: response.output_item.added\n\n",
        "event: response.function_call_arguments.delta\ndata: {\"type\":\"response.created\",\"response\":{\"output\":[]}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"arguments\":\"\"}}\n\n",
    ] {
        let fixture = Fixture::new(suffix.into(), false, policy(), false, true).await;
        fixture.host.optimize.store(true, Ordering::SeqCst);
        let output = fixture.request(&namespace_body()).await;
        assert!(!output.contains("response.completed"), "{suffix}: {output}");
        assert_eq!(fixture.source.requests.lock().unwrap().len(), 1, "{suffix}");
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn recovered_final_response_continues_on_the_same_owner_with_complete_history() {
    use sha2::Digest;
    let fixture = Fixture::new(String::new(), false, policy(), false, true).await;
    let answer = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"synthetic recovered answer"}]});
    *fixture.source.successful_output.lock().unwrap() = json!([answer.clone()]);
    let mut original = body();
    original["client_metadata"] =
        json!({"session_id":"synthetic-recovery-session","thread_id":"synthetic-recovery-thread"});
    let output = fixture.request(&original).await;
    let terminal = output
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|value| serde_json::from_str::<Value>(value).ok())
        .find(|event| event["type"] == "response.completed")
        .expect(&output);
    let final_id = terminal["response"]["id"].as_str().unwrap();
    assert_eq!(final_id, "resp_fixture_2");
    assert_ne!(final_id, "resp_fixture_1");
    let next_input = json!({"type":"message","role":"user","content":[{"type":"input_text","text":"synthetic follow-up"}]});
    let next = json!({"model":"fixture","stream":true,"previous_response_id":final_id,
        "client_metadata":original["client_metadata"],"input":[next_input.clone()]});
    let output = fixture.request(&next).await;
    assert!(output.contains("response.completed"), "{output}");
    {
        let requests = fixture.source.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            3,
            "two attempts in the recovered turn and one independent continuation"
        );
        let recovered: Value = serde_json::from_slice(&requests[1]).unwrap();
        let continued: Value = serde_json::from_slice(&requests[2]).unwrap();
        let mut expected = recovered["input"].as_array().unwrap().clone();
        expected.push(answer);
        expected.push(next_input);
        assert_eq!(
            continued["input"],
            json!(expected),
            "complete successful history is expanded exactly once"
        );
        assert!(
            continued.get("previous_response_id").is_none(),
            "native stateless HTTP must not resend the interrupted ID"
        );
    }
    {
        let routes = fixture.host.routes.lock().unwrap();
        assert_eq!(routes.len(), 3);
        assert_eq!(routes[2].preferred_account_id.as_deref(), Some("account-a"));
    }
    let connection =
        rusqlite::Connection::open(fixture._directory.path().join("identity.sqlite")).unwrap();
    let (account, owner): (String, String) = connection.query_row(
        "SELECT account,owner FROM native_opaque_v1 WHERE caller='fixture' AND kind='response' AND digest=?1",
        [format!("{:x}", sha2::Sha256::digest(final_id.as_bytes()))], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    assert_eq!((account.as_str(), owner.as_str()), ("account-a", "owner-a"));
    let incomplete_checkpoint: i64 = connection.query_row(
        "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1 WHERE caller='fixture' AND response='resp_fixture_1'",
        [], |row| row.get(0)).unwrap();
    assert_eq!(
        incomplete_checkpoint, 0,
        "the interrupted metadata ID is not a completed history checkpoint"
    );
    drop(connection);
    fixture.shutdown().await;
}
