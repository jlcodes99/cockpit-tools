use crate::modules::codex_native_api::{self as native, contract::*};
use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, protocol::Message},
    MaybeTlsStream, WebSocketStream,
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[path = "chat_tool_integrity_http_tests.rs"]
mod tool_integrity_tests;

struct BridgeHost {
    base: String,
    wire: WireApi,
    passthrough: bool,
    honor_preferred: AtomicBool,
    initial_timeout_ms: std::sync::atomic::AtomicU64,
    reports: Mutex<Vec<RequestReport>>,
    changed: tokio::sync::Notify,
}
impl Host for BridgeHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if bearer != "key" {
                return Err(ServiceError::new(401, "invalid_api_key", "invalid"));
            }
            Ok(Caller {
                id: "bridge-caller".into(),
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
            let account = if self.honor_preferred.load(Ordering::SeqCst) {
                request
                    .preferred_account_id
                    .as_deref()
                    .unwrap_or(if request.model == "other" { "b" } else { "a" })
            } else {
                if request.model == "other" {
                    "b"
                } else {
                    "a"
                }
            };
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
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: false,
                wire_api: self.wire,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(self.passthrough) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                websocket_initial_timeout: Duration::from_millis(
                    self.initial_timeout_ms.load(Ordering::SeqCst),
                ),
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
            self.changed.notify_waiters();
        })
    }
}
async fn fixture(
    app: Router,
    wire: WireApi,
    passthrough: bool,
    options: native::Options,
) -> (
    native::Handle,
    Arc<BridgeHost>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(BridgeHost {
        base,
        wire,
        passthrough,
        honor_preferred: AtomicBool::new(true),
        initial_timeout_ms: std::sync::atomic::AtomicU64::new(30_000),
        reports: Mutex::new(Vec::new()),
        changed: tokio::sync::Notify::new(),
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        dir.path().join("identity.sqlite"),
        options,
    )
    .await
    .unwrap();
    (handle, host, task, dir)
}
async fn socket(handle: &native::Handle) -> Socket {
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn send(socket: &mut Socket, body: Value) {
    socket
        .send(Message::Text(body.to_string().into()))
        .await
        .unwrap();
}
async fn terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            match message {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if matches!(
                        event["type"].as_str(),
                        Some(
                            "response.completed"
                                | "response.done"
                                | "response.cancelled"
                                | "response.failed"
                                | "response.incomplete"
                                | "error"
                        )
                    ) {
                        return event;
                    }
                }
                Message::Ping(v) => socket.send(Message::Pong(v)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}
async fn reports(host: &BridgeHost, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let changed = host.changed.notified();
            if host.reports.lock().unwrap().len() >= count {
                return;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn websocket_chat_bridge_prewarms_locally_then_replays_tools_with_stable_identity() {
    let requests = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    async fn chat(
        State(requests): State<Arc<Mutex<Vec<(HeaderMap, Value)>>>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let n = {
            let mut requests = requests.lock().unwrap();
            requests.push((headers, body));
            requests.len()
        };
        let message = if n == 1 {
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"call_tool","type":"function","function":{"name":"tool","arguments":"{}"}}]})
        } else {
            json!({"role":"assistant","content":"done"})
        };
        Json(
            json!({"id":format!("chat-{n}"),"object":"chat.completion","model":"m","choices":[{"index":0,"message":message,"finish_reason":if n==1{"tool_calls"}else{"stop"}}],"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}),
        )
    }
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(requests.clone());
    let (mut handle, host, task, dir) =
        fixture(app, WireApi::Chat, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    let tools = json!([{"type":"function","name":"tool","parameters":{"type":"object","properties":{}},"strict":false}]);
    send(&mut ws,json!({"type":"response.create","model":"m","generate":false,"tools":tools,"input":[{"role":"user","content":"warm context"}]})).await;
    let warm = terminal(&mut ws).await;
    let warm_id = warm["response"]["id"].as_str().unwrap();
    assert!(warm_id.starts_with("resp_prewarm_"));
    assert!(requests.lock().unwrap().is_empty());
    assert!(host.reports.lock().unwrap().is_empty());
    let c = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(c);
    send(&mut ws,json!({"type":"response.create","model":"m","tools":tools,"previous_response_id":warm_id,"input":[{"role":"user","content":"use tool"}]})).await;
    let first = terminal(&mut ws).await;
    assert_eq!(first["type"], "response.completed");
    assert_eq!(first["response"]["output"][0]["call_id"], "call_tool");
    send(&mut ws,json!({"type":"response.append","input":[{"type":"function_call_output","call_id":"call_tool","output":"result"}]})).await;
    let second = terminal(&mut ws).await;
    assert_eq!(second["type"], "response.completed");
    reports(&host, 2).await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].0["session-id"], requests[1].0["session-id"]);
    assert_eq!(requests[0].0["thread-id"], requests[1].0["thread-id"]);
    assert!(requests[0].1["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["content"] == "warm context"));
    assert!(requests[1].1["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "tool" && m["tool_call_id"] == "call_tool"));
    assert!(requests[1].1["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["tool_calls"][0]["id"] == "call_tool"));
    drop(requests);
    assert!(host
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|report| report.success && report.account_id == "a"));
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_http_cipher_continuation_cannot_switch_to_a_different_owner() {
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let requests_copy = requests.clone();
    let app=Router::new().route("/v1/responses",post(move |Json(body):Json<Value>|{let requests=requests_copy.clone();async move{requests.lock().unwrap().push(body);Response::builder().header("content-type","text/event-stream").body(Body::from("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"owned-response\",\"status\":\"completed\",\"output\":[{\"type\":\"reasoning\",\"encrypted_content\":\"owned-cipher\"}]}}\n\n")).unwrap()}}));
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, false, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(
        &mut ws,
        json!({"type":"response.create","model":"m","input":[{"role":"user","content":"hi"}]}),
    )
    .await;
    let first = terminal(&mut ws).await;
    assert_eq!(first["type"], "response.completed");
    host.honor_preferred.store(false, Ordering::SeqCst);
    send(&mut ws,json!({"type":"response.append","model":"other","input":[{"type":"reasoning","encrypted_content":"owned-cipher"}]})).await;
    let failure = terminal(&mut ws).await;
    assert_eq!(failure["type"], "error");
    assert_eq!(failure["status"], 409);
    assert_eq!(requests.lock().unwrap().len(), 1);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_upstream_upgrade_426_falls_back_to_http_without_failed_model_record() {
    let app = Router::new().route(
        "/v1/responses",
        get(|| async { StatusCode::UPGRADE_REQUIRED }).post(|| async {
            Json(json!({"id":"fallback-response","status":"completed","output":[]}))
        }),
    );
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    send(
        &mut ws,
        json!({"type":"response.create","model":"m","input":[]}),
    )
    .await;
    let event = terminal(&mut ws).await;
    assert_eq!(event["type"], "response.completed");
    reports(&host, 1).await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(
        attempts.iter().filter_map(|a| a.status).collect::<Vec<_>>(),
        vec![426, 200]
    );
    drop(reports);
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn idle_websockets_do_not_consume_explicit_request_capacity() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async { Json(json!({"id":"http-response","status":"completed","output":[]})) }),
    );
    let options = native::Options {
        max_in_flight: 1,
        ..Default::default()
    };
    let (mut handle, _host, task, _dir) = fixture(app, WireApi::Responses, false, options).await;
    let mut first = socket(&handle).await;
    let mut second = socket(&handle).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","input":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    first.close(None).await.unwrap();
    second.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

include!("websocket_interrupt_bridge_tests.rs");

include!("websocket_recoverable_control_tests.rs");

include!("websocket_natural_incomplete_tests.rs");

include!("websocket_output_parity_tests.rs");
