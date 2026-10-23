use super::*;
use crate::modules::codex_native_api::{contract::*, identity::IdentityStore, Options};
use axum::{
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::future::BoxFuture;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};
#[path = "request_fault_websocket_tests.rs"]
mod request_fault_websocket_tests;

#[derive(Clone)]
enum Behavior {
    QuotaGate { metadata: bool, business: Option<Value>, reused: bool, wait_interrupt: bool },
    WaitForInterrupt,
    Success,
    DropHandshakeAfterRequest,
    Handshake(u16),
    HandshakeBody(u16, Value),
    First401UntilRefresh,
    CreatedThen401,
    CreatedThenOverload,
    OutputThenOverload,
    SuccessThenOverload,
    SuccessThenAlwaysOverload,
    FirstError(Value),
    Close1009,
    CreatedThenClose1009,
}

struct Upstream {
    behavior: Behavior,
    response_id: &'static str,
    attempts: AtomicUsize,
    handshakes: Mutex<Vec<HeaderMap>>,
    requests: Mutex<Vec<(HeaderMap, Value)>>,
}

async fn endpoint(state: Arc<Upstream>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    state.attempts.fetch_add(1, Ordering::SeqCst);
    state.handshakes.lock().unwrap().push(headers.clone());
    if let Behavior::Handshake(status) = state.behavior {
        return (
            StatusCode::from_u16(status).unwrap(),
            [("retry-after", "17")],
            "upstream rejection",
        )
            .into_response();
    }
    if let Behavior::HandshakeBody(status, ref body) = state.behavior {
        return (
            StatusCode::from_u16(status).unwrap(),
            axum::Json(body.clone()),
        )
            .into_response();
    }
    ws.on_upgrade(move |mut socket| async move {
        let Some(Ok(Message::Text(request))) = socket.recv().await else { return; };
        let body = serde_json::from_str(request.as_str()).unwrap();
        state.requests.lock().unwrap().push((headers.clone(), body));
        if matches!(state.behavior,Behavior::Close1009 | Behavior::CreatedThenClose1009) {
            if matches!(state.behavior,Behavior::CreatedThenClose1009) {
                socket.send(Message::Text(json!({"type":"response.created","response":{"id":"oversized"}}).to_string().into())).await.unwrap();
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let _=socket.send(Message::Close(Some(axum::extract::ws::CloseFrame{code:1009,reason:"message too big".into()}))).await;
            return;
        }
        if let Behavior::QuotaGate { metadata, business, reused: false, wait_interrupt } = &state.behavior {
            quota_gate_events(&mut socket, *metadata, business.as_ref(), *wait_interrupt, if state.response_id == "completed" {"quota-active"} else {"quota-backup-active"}).await;
            return;
        }
        if matches!(state.behavior, Behavior::WaitForInterrupt) {
            socket.send(Message::Text(json!({"type":"response.created","response":{"id":"backup-active","output":[]}}).to_string().into())).await.unwrap();
            loop {
                match socket.recv().await {
                    Some(Ok(Message::Text(text))) => {
                        let control: Value = serde_json::from_str(text.as_str()).unwrap();
                        state.requests.lock().unwrap().push((headers.clone(), control.clone()));
                        assert_eq!(control["response_id"], "backup-active");
                        break;
                    }
                    Some(Ok(Message::Ping(value))) => { socket.send(Message::Pong(value)).await.unwrap(); }
                    _ => return,
                }
            }
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"backup-active","status":"completed","output":[]}}).to_string().into())).await.unwrap();
            while socket.recv().await.is_some() {}
            return;
        }
        let error = json!({"type":"error","status":401,"error":{"code":"invalid_api_key","message":"expired"}});
        let event = match &state.behavior {
            Behavior::First401UntilRefresh
                if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer fresh") => error,
            Behavior::CreatedThen401 => {
                socket.send(Message::Text(json!({"type":"response.created","response":{"id":"active"}}).to_string().into())).await.unwrap();
                error
            }
            Behavior::CreatedThenOverload | Behavior::OutputThenOverload => {
                socket.send(Message::Text(json!({"type":"response.created","response":{"id":"active"}}).to_string().into())).await.unwrap();
                if matches!(state.behavior, Behavior::OutputThenOverload) {
                    socket.send(Message::Text(json!({"type":"response.output_text.delta","delta":"hello"}).to_string().into())).await.unwrap();
                }
                json!({"type":"error","error":{"code":"server_is_overloaded","type":"service_unavailable_error","message":"overloaded"}})
            }
            Behavior::FirstError(error) => error.clone(),
            Behavior::SuccessThenAlwaysOverload if state.attempts.load(Ordering::SeqCst) > 1 => {
                json!({"type":"error","error":{"code":"server_is_overloaded","type":"service_unavailable_error"}})
            }
            _ => json!({"type":"response.completed","response":{"id":state.response_id,"status":"completed","output":[]}}),
        };
        let _ = socket.send(Message::Text(event.to_string().into())).await;
        while let Some(Ok(message)) = socket.recv().await {
            if matches!(message, Message::Close(_)) { break; }
            if let Behavior::QuotaGate { metadata, business, reused: true, wait_interrupt } = &state.behavior {
                if let Message::Text(text) = &message {
                    state.requests.lock().unwrap().push((headers.clone(), serde_json::from_str(text.as_str()).unwrap()));
                    quota_gate_events(&mut socket, *metadata, business.as_ref(), *wait_interrupt, if state.response_id == "completed" {"quota-active"} else {"quota-backup-active"}).await;
                    return;
                }
            }
            if matches!(state.behavior, Behavior::SuccessThenOverload | Behavior::SuccessThenAlwaysOverload) && matches!(message, Message::Text(_)) {
                socket.send(Message::Text(json!({"type":"response.created","response":{"id":"second"}}).to_string().into())).await.unwrap();
                socket.send(Message::Text(json!({"type":"error","error":{"code":"server_is_overloaded","type":"service_unavailable_error"}}).to_string().into())).await.unwrap();
            }
        }
    })
}

#[tokio::test]
async fn parity22_runtime_websocket_lite_wire_matches_go_executor() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/parity22_runtime_go.json")).unwrap();
    let fixture = Fixture::start(Behavior::Success, policy(), false).await;
    let result = fixture
        .request(reference["websocket_wire"]["input"].clone(), 1)
        .await;
    assert_eq!(result[0]["type"], "response.completed");
    {
        let captures = fixture.first.requests.lock().unwrap();
        let (headers, body) = &captures[0];
        let actual = json!({"stream_options":body.get("stream_options"),"lite_header":headers.get(super::super::codex_wire::LITE_HEADER).and_then(|value|value.to_str().ok()).unwrap_or(""),"parallel_tool_calls":body.get("parallel_tool_calls"),"tools":body.get("tools")});
        assert_eq!(actual, reference["websocket_wire"]["expected"]);
    }
    fixture.finish().await;
}

#[tokio::test]
async fn parity22_runtime_websocket_1009_and_structured_413_match_go_close_frames() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/parity22_runtime_close_go.json")).unwrap();
    for (behavior, index) in [
        (Behavior::Close1009, 0),
        (Behavior::CreatedThenClose1009, 0),
        (
            Behavior::HandshakeBody(
                413,
                json!({"error":{"code":"message_too_big","message":"fixture frame too large"}}),
            ),
            1,
        ),
        (
            Behavior::FirstError(
                json!({"type":"error","status":413,"body":{"error":{"code":"message_too_big","message":"fixture frame too large"}}}),
            ),
            1,
        ),
    ] {
        let mut policy = policy();
        policy.credential_retries = 1;
        policy.send_retries = 0;
        policy.status_retries = 0;
        policy.bootstrap_retries = 0;
        let fixture = Fixture::start(behavior, policy, false).await;
        let mut outbound = format!("ws://{}/v1/responses", fixture.handle.addr)
            .into_client_request()
            .unwrap();
        outbound.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Bearer fixture-only"),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(outbound).await.unwrap();
        socket
            .send(ClientMessage::Text(request().to_string().into()))
            .await
            .unwrap();
        let frame = loop {
            let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let ClientMessage::Close(Some(frame)) = message {
                break frame;
            }
        };
        assert_eq!(
            u16::from(frame.code),
            reference[index]["code"].as_u64().unwrap() as u16
        );
        assert_eq!(
            frame.reason.as_str(),
            reference[index]["reason"].as_str().unwrap()
        );
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        drop(socket);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn parity22_runtime_websocket_generic_413_stays_an_error_without_1009() {
    let fixture = Fixture::start(
        Behavior::HandshakeBody(
            413,
            json!({"error":{"code":"request_too_large","message":"fixture request too large"}}),
        ),
        policy(),
        false,
    )
    .await;
    let result = fixture.request(request(), 1).await;
    assert_eq!(result[0]["status"], 413);
    assert_eq!(result[0]["error"]["code"], "request_too_large");
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn parity22_runtime_wrapped_quota_preserves_go_reset_and_scope_before_failover() {
    let reference: Value =
        serde_json::from_str(include_str!("fixtures/parity22_runtime_go.json")).unwrap();
    for case in reference["wrapped_errors"].as_array().unwrap() {
        let event = &case["input"];
        let error =
            first_event_rejection(&UpstreamMessage::Text(event.to_string().into()), false).unwrap();
        assert_eq!(
            error.0.status,
            u16::try_from(case["status"].as_u64().unwrap()).unwrap()
        );
        assert_eq!(
            error.0.credential_scope,
            case["credential_scope"].as_bool().unwrap()
        );
        let delay = Duration::from_millis(case["retry_after_ms"].as_u64().unwrap());
        assert_eq!(error.1, Some(delay));
        let fixture = Fixture::start(Behavior::FirstError(event.clone()), policy(), false).await;
        let result = fixture.request(request(), 1).await;
        assert_eq!(result[0]["type"], "response.completed");
        assert_eq!(
            *fixture.host.failures.lock().unwrap(),
            vec![("a".into(), 429, Some(delay))]
        );
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
        fixture.finish().await;
    }
}

struct RetryHost {
    bases: [String; 2],
    policy: ExecutionPolicy,
    selections: Mutex<Vec<RouteRequest>>,
    failures: Mutex<Vec<(String, u16, Option<Duration>)>>,
    reports: Mutex<Vec<RequestReport>>,
    capacity: Arc<tokio::sync::Semaphore>,
    refreshable: AtomicBool,
    refresh_fails: AtomicBool,
    passthrough: AtomicBool,
}

impl Host for RetryHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "caller".into(),
                label: "test".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            if request.force_refresh && self.refresh_fails.load(Ordering::Acquire) {
                return Err(ServiceError::new(
                    401,
                    "reauthorization_required",
                    "Fixture refresh rejected",
                ));
            }
            let account = request.preferred_account_id.as_deref().unwrap_or_else(|| {
                if request.excluded_account_ids.iter().any(|id| id == "a") {
                    "b"
                } else {
                    "a"
                }
            });
            if request.excluded_account_ids.iter().any(|id| id == account) {
                return Err(ServiceError::new(
                    503,
                    "no_available_account",
                    "No alternate account",
                ));
            }
            let permit: Option<Arc<dyn Send + Sync>> = if request.requires_permit {
                Some(Arc::new(
                    self.capacity.clone().try_acquire_owned().map_err(|_| {
                        ServiceError::new(429, "account_busy", "Permit was retained during retry")
                    })?,
                ))
            } else {
                None
            };
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                HeaderValue::from_static(if request.force_refresh {
                    "Bearer fresh"
                } else {
                    "Bearer stale"
                }),
            );
            Ok(Route {
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: self.bases[usize::from(account == "b")].clone(),
                model: request.model,
                headers,
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.refreshable.load(Ordering::Acquire),
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({})) })
    }
    fn responses_websocket_passthrough<'a>(&'a self, _: &'a Caller, _: &'a Route) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(self.passthrough.load(Ordering::Acquire)) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        delay: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures
                .lock()
                .unwrap()
                .push((account.into(), status, delay));
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct Fixture {
    handle: crate::modules::codex_native_api::Handle,
    host: Arc<RetryHost>,
    first: Arc<Upstream>,
    second: Arc<Upstream>,
    task: tokio::task::JoinHandle<()>,
    raw_task: Option<tokio::task::JoinHandle<()>>,
    path: PathBuf,
}

impl Fixture {
    async fn start(first: Behavior, policy: ExecutionPolicy, broken_first: bool) -> Self {
        Self::start_with_second(first, Behavior::Success, policy, broken_first).await
    }
    async fn start_with_second(first: Behavior, second: Behavior, policy: ExecutionPolicy, broken_first: bool) -> Self {
        let first = Arc::new(Upstream {
            behavior: first,
            response_id: "completed",
            attempts: AtomicUsize::new(0),
            handshakes: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        });
        let second = Arc::new(Upstream {
            behavior: second,
            response_id: "completed-b",
            attempts: AtomicUsize::new(0),
            handshakes: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        });
        let app = Router::new()
            .route(
                "/a/responses",
                get({
                    let state = first.clone();
                    move |headers, ws| endpoint(state.clone(), headers, ws)
                }).post({
                    let state = first.clone();
                    move |headers, body| quota_gate_tests::http_endpoint(state.clone(), headers, body)
                }),
            )
            .route(
                "/b/responses",
                get({
                    let state = second.clone();
                    move |headers, ws| endpoint(state.clone(), headers, ws)
                }).post({
                    let state = second.clone();
                    move |headers, body| quota_gate_tests::http_endpoint(state.clone(), headers, body)
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut raw_task = None;
        let first_base = if matches!(first.behavior, Behavior::DropHandshakeAfterRequest) {
            let raw = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/a", raw.local_addr().unwrap());
            let source = first.clone();
            raw_task = Some(tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                loop {
                    let (mut stream, _) = raw.accept().await.unwrap();
                    let mut bytes = [0; 4096];
                    assert!(stream.read(&mut bytes).await.unwrap() > 0);
                    source.attempts.fetch_add(1, Ordering::SeqCst);
                    // The request reached upstream; losing its response is
                    // ambiguous and must still consume the send retry budget.
                    drop(stream);
                }
            }));
            base
        } else if broken_first {
            let unused = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/a", unused.local_addr().unwrap());
            drop(unused);
            base
        } else {
            format!("http://{address}/a")
        };
        let host = Arc::new(RetryHost {
            bases: [first_base, format!("http://{address}/b")],
            policy,
            selections: Mutex::new(Vec::new()),
            failures: Mutex::new(Vec::new()),
            reports: Mutex::new(Vec::new()),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
            refreshable: AtomicBool::new(true),
            refresh_fails: AtomicBool::new(false),
            passthrough: AtomicBool::new(true),
        });
        let path = std::env::temp_dir().join(format!("native-ws-retry-{}", uuid::Uuid::now_v7()));
        let handle = crate::modules::codex_native_api::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            first,
            second,
            task,
            raw_task,
            path,
        }
    }
    async fn request(&self, body: Value, count: usize) -> Vec<Value> {
        let mut request = format!("ws://{}/v1/responses", self.handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer local"));
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(ClientMessage::Text(body.to_string().into()))
            .await
            .unwrap();
        let mut result = Vec::new();
        while result.len() < count {
            let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let ClientMessage::Text(text) = message {
                result.push(serde_json::from_str(text.as_str()).unwrap());
            }
        }
        let _ = socket.close(None).await;
        result
    }
    async fn finish(mut self) {
        self.handle.shutdown(Duration::from_secs(2)).await.unwrap();
        assert_eq!(self.host.capacity.available_permits(), 1);
        self.task.abort();
        if let Some(task) = self.raw_task { task.abort(); }
        let _ = std::fs::remove_dir_all(self.path);
    }
}

fn policy() -> ExecutionPolicy {
    ExecutionPolicy {
        credential_retries: 2,
        send_retries: 2,
        status_retries: 1,
        retry_base_delay: Duration::from_millis(1),
        status_retry_base_delay: Duration::from_millis(1),
        status_retry_max_delay: Duration::from_millis(1),
        total_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}
fn request() -> Value {
    json!({"type":"response.create","model":"model","input":[],"client_metadata":{"session_id":"root","thread_id":"root"}})
}

#[tokio::test]
async fn reused_websocket_turn_is_probed_before_output_and_retains_opaque_owner() {
    for permanent in [false, true] {
        let fixture = Fixture::start(
            if permanent {
                Behavior::SuccessThenAlwaysOverload
            } else {
                Behavior::SuccessThenOverload
            },
            ExecutionPolicy {
                credential_retries: 0,
                status_retries: 1,
                ..policy()
            },
            false,
        )
        .await;
        let mut url = format!("ws://{}/v1/responses", fixture.handle.addr)
            .into_client_request()
            .unwrap();
        url.headers_mut()
            .insert("authorization", HeaderValue::from_static("Bearer local"));
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        for turn in 0..2 {
            let mut body = request();
            // This account-owned resource cannot move with the otherwise
            // complete socket history, so retries must retain the original A.
            body["tools"] = json!([{"type":"file_search","vector_store_ids":["fixture-store-a"]}]);
            if turn > 0 {
                body["previous_response_id"] = json!("completed");
            }
            socket
                .send(ClientMessage::Text(body.to_string().into()))
                .await
                .unwrap();
            let event = tokio::time::timeout(Duration::from_secs(3), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let ClientMessage::Text(text) = event else {
                panic!("expected response event")
            };
            let event: Value = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(
                event["type"],
                if permanent && turn == 1 {
                    "error"
                } else {
                    "response.completed"
                },
                "turn {turn}: {event}"
            );
        }
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        socket.close(None).await.unwrap();
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_buffered_overload_retries_but_never_after_output() {
    for after_output in [false, true] {
        let fixture = Fixture::start(
            if after_output {
                Behavior::OutputThenOverload
            } else {
                Behavior::CreatedThenOverload
            },
            ExecutionPolicy {
                credential_retries: 0,
                status_retries: 0,
                ..policy()
            },
            false,
        )
        .await;
        let events = fixture
            .request(request(), if after_output { 3 } else { 1 })
            .await;
        if after_output {
            assert_eq!(events[0]["type"], "response.created");
            assert_eq!(events[1]["delta"], "hello");
            assert_eq!(events[2]["error"]["code"], "server_is_overloaded");
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(events[0]["type"], "response.completed");
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.host.failures.lock().unwrap()[0].1, 503);
        }
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_retry_429_switches_once_without_spending_send_budget() {
    let fixture = Fixture::start(Behavior::Handshake(429), policy(), false).await;
    assert_eq!(
        fixture.request(request(), 1).await[0]["type"],
        "response.completed"
    );
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    let first_headers = fixture.first.handshakes.lock().unwrap()[0].clone();
    let second_headers = fixture.second.handshakes.lock().unwrap()[0].clone();
    assert_ne!(first_headers["session-id"], second_headers["session-id"]);
    assert_ne!(first_headers["thread-id"], second_headers["thread-id"]);
    assert_eq!(
        second_headers["thread-id"],
        second_headers["x-client-request-id"]
    );
    assert_eq!(
        fixture.host.failures.lock().unwrap().as_slice(),
        &[("a".into(), 429, Some(Duration::from_secs(17)))]
    );
    let selections = fixture.host.selections.lock().unwrap();
    assert!(selections
        .last()
        .unwrap()
        .excluded_account_ids
        .contains(&"a".into()));
    drop(selections);
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_retry_server_status_has_its_own_budget_before_account_switch() {
    let fixture = Fixture::start(Behavior::Handshake(503), policy(), false).await;
    assert_eq!(
        fixture.request(request(), 1).await[0]["type"],
        "response.completed"
    );
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.host.failures.lock().unwrap().as_slice(),
        &[("a".into(), 503, Some(Duration::from_secs(17)))]
    );
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_retry_network_send_budget_is_independent_of_status_budget() {
    let fixture = Fixture::start(
        Behavior::DropHandshakeAfterRequest,
        ExecutionPolicy {
            send_retries: 1,
            ..policy()
        },
        false,
    )
    .await;
    assert_eq!(
        fixture.request(request(), 1).await[0]["type"],
        "response.completed"
    );
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 2);
    assert!(fixture.host.failures.lock().unwrap().is_empty());
    // Two network attempts plus the alternate account's successful attempt.
    let host = fixture.host.clone();
    fixture.finish().await;
    assert_eq!(
        host.reports.lock().unwrap()[0]
            .diagnostics
            .as_ref()
            .unwrap()
            .attempts
            .len(),
        3
    );
}

#[tokio::test]
async fn websocket_retry_first_error_refreshes_same_owner_without_remapping_identity() {
    let fixture = Fixture::start(Behavior::First401UntilRefresh, policy(), false).await;
    assert_eq!(
        fixture.request(request(), 1).await[0]["type"],
        "response.completed"
    );
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    let requests = fixture.first.requests.lock().unwrap();
    assert_eq!(requests[0].0["session-id"], requests[1].0["session-id"]);
    assert_eq!(requests[0].0["thread-id"], requests[1].0["thread-id"]);
    assert_eq!(requests[1].0["authorization"], "Bearer fresh");
    drop(requests);
    assert_eq!(
        fixture
            .host
            .selections
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.force_refresh)
            .count(),
        1
    );
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_retry_never_replays_after_response_created() {
    let fixture = Fixture::start(Behavior::CreatedThen401, policy(), false).await;
    let output = fixture.request(request(), 2).await;
    assert_eq!(output[0]["type"], "response.created");
    assert_eq!(output[1]["error"]["code"], "invalid_api_key");
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    assert!(!fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.force_refresh));
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_retry_opaque_continuation_does_not_switch_accounts() {
    let fixture = Fixture::start(Behavior::Handshake(429), policy(), false).await;
    let store = IdentityStore::open(fixture.path.join("identity.sqlite")).unwrap();
    let mut body = request();
    let mut headers = HeaderMap::new();
    let context = store.inspect("caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    store.observe_event(&binding,&json!({"type":"response.completed","response":{"id":"previous","status":"completed","output":[]}})).unwrap();
    drop(store);
    let mut body = request();
    body["previous_response_id"] = json!("previous");
    let output = fixture.request(body, 1).await;
    assert_eq!(output[0]["status"], 429);
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_retry_returns_original_unknown_error_without_retrying() {
    let original = json!({"type":"error","status":422,"error":{"code":"unknown_request","message":"original message"},"extension":{"unchanged":true}});
    let fixture = Fixture::start(Behavior::FirstError(original.clone()), policy(), false).await;
    assert_eq!(fixture.request(request(), 1).await[0], original);
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn automatic_history_websocket_error_is_terminal_while_connection_stays_open() {
    let fixture = Fixture::start(Behavior::CreatedThen401, policy(), false).await;
    let path = fixture.path.join("identity.sqlite");

    let mut upgrade = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    upgrade
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local"));
    let (mut socket, _) = tokio_tungstenite::connect_async(upgrade).await.unwrap();
    let mut body = request();
    body["input"] = json!([{"type":"reasoning","encrypted_content":super::super::codex_cpa_wire::synthetic_gpt_signature(11)}]);
    socket
        .send(ClientMessage::Text(body.to_string().into()))
        .await
        .unwrap();
    for kind in ["response.created", "error"] {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if let ClientMessage::Text(text) = message {
                let event: Value = serde_json::from_str(text.as_str()).unwrap();
                assert_eq!(event["type"], kind);
                break;
            }
        }
    }
    // Output already reached the client: an upstream error must not replay another account.
    assert!(!path.with_extension("recovery").exists());
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    assert!(!fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .any(|selection| selection.force_refresh));
    let requests = fixture.first.requests.lock().unwrap();
    assert_ne!(requests[0].0["thread-id"], "root");
    assert_eq!(requests[0].1["input"], body["input"]);
    drop(requests);
    let connection = rusqlite::Connection::open(&path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    let _ = socket.close(None).await;
    fixture.finish().await;
}

#[test]
fn first_event_retry_detection_requires_an_error_without_any_output() {
    for event in [
        json!({"type":"response.created","status":401,"response":{"id":"active"}}),
        json!({"type":"response.output_text.delta","status":401,"delta":"visible"}),
        json!({"type":"response.failed","status":401,"response":{"output":[{"type":"message","content":[]}]}}),
        json!({"type":"error","error":{"code":"unknown"}}),
    ] {
        assert!(
            first_event_rejection(&UpstreamMessage::Text(event.to_string().into()), false)
                .is_none()
        );
    }
}

#[tokio::test]
async fn websocket_refresh_failure_and_payment_can_use_safe_alternate_but_api_keys_do_not_refresh()
{
    for (behavior, refreshable, refresh_fails, refresh_count) in [
        (Behavior::First401UntilRefresh, true, true, 1),
        (Behavior::First401UntilRefresh, false, false, 0),
        (Behavior::Handshake(402), false, false, 0),
    ] {
        let fixture = Fixture::start(behavior, policy(), false).await;
        fixture
            .host
            .refreshable
            .store(refreshable, Ordering::Release);
        fixture
            .host
            .refresh_fails
            .store(refresh_fails, Ordering::Release);
        assert_eq!(
            fixture.request(request(), 1).await[0]["type"],
            "response.completed"
        );
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture
                .host
                .selections
                .lock()
                .unwrap()
                .iter()
                .filter(|request| request.force_refresh)
                .count(),
            refresh_count
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_refresh_failure_cannot_transfer_known_opaque_owner() {
    let fixture = Fixture::start(Behavior::First401UntilRefresh, policy(), false).await;
    fixture.host.refresh_fails.store(true, Ordering::Release);
    let store = IdentityStore::open(fixture.path.join("identity.sqlite")).unwrap();
    let mut headers = HeaderMap::new();
    let mut body = request();
    let context = store.inspect("caller", &headers, &body).unwrap();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"known","status":"completed","output":[]}})).unwrap();
    let mut body = request();
    body["previous_response_id"] = json!("known");
    let events = fixture.request(body, 1).await;
    assert_eq!(events[0]["error"]["code"], "invalid_api_key");
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    assert!(fixture
        .host
        .selections
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.excluded_account_ids.is_empty()));
    fixture.finish().await;
}

#[path = "websocket_quota_gate_tests.rs"]
mod quota_gate_tests;
use quota_gate_tests::quota_gate_events;
