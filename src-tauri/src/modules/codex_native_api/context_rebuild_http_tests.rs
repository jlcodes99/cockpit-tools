//! Synthetic loopback coverage: rebuilding creates a new generation, never transfers state.
use crate::modules::codex_native_api::{
    contract::*, identity::IdentityStore, start, Handle, Options,
};
use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message as UpstreamMessage, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, stream, SinkExt, StreamExt};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[derive(Clone)]
enum Step {
    Complete(&'static str, Option<Value>),
    CompleteItems(&'static str, Vec<Value>),
    RawEvents(Vec<Value>),
    HttpOverload,
    HttpRateLimit,
    SseOverload,
    Hold(&'static str),
}
#[derive(Clone, Debug)]
struct Sent {
    account: String,
    headers: HeaderMap,
    body: Value,
}
struct Upstream {
    steps: Mutex<[VecDeque<Step>; 2]>,
    sent: Mutex<Vec<Sent>>,
    ws_connections: Mutex<Vec<String>>,
}
struct TestHost {
    base: String,
    enabled: AtomicBool,
    oauth: AtomicBool,
    passthrough: AtomicBool,
    removed_a: AtomicBool,
    selections: Mutex<Vec<RouteRequest>>,
    failures: Mutex<Vec<(String, u16)>>,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for TestHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "caller".into(),
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
            self.selections.lock().unwrap().push(request.clone());
            let account = if let Some(preferred) = request.preferred_account_id.as_deref() {
                if preferred == "a" && self.removed_a.load(Ordering::SeqCst) {
                    return Err(ServiceError::new(
                        503,
                        "account_binding_unavailable",
                        "Synthetic account A removed",
                    ));
                }
                if request
                    .excluded_account_ids
                    .iter()
                    .any(|id| id == preferred)
                {
                    return Err(ServiceError::new(
                        503,
                        "account_cooling",
                        "Synthetic pinned owner unavailable",
                    ));
                }
                preferred
            } else {
                ["a", "b"]
                    .into_iter()
                    .find(|candidate| {
                        !(*candidate == "a" && self.removed_a.load(Ordering::SeqCst))
                            && !request
                                .excluded_account_ids
                                .iter()
                                .any(|id| id == *candidate)
                    })
                    .ok_or_else(|| {
                        ServiceError::new(503, "no_available_account", "Synthetic pool exhausted")
                    })?
            };
            Ok(Route {
                continuity_account_id: None,
                installation_profile: Some(installation_profile(account)),
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: format!("{}/{account}", self.base),
                model: request.model,
                headers: HeaderMap::from_iter([(
                    axum::http::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer synthetic-{account}")).unwrap(),
                )]),
                client: client(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth.load(Ordering::SeqCst),
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                account_switch_rebuild: self.enabled.load(Ordering::SeqCst),
                credential_retries: 0,
                status_retries: 0,
                send_retries: 0,
                bootstrap_retries: 0,
                total_timeout: Duration::from_secs(6),
                open_timeout: Duration::from_secs(3),
                idle_timeout: Duration::from_secs(3),
                websocket_initial_timeout: Duration::from_secs(3),
                websocket_idle_timeout: Duration::from_secs(3),
                ..Default::default()
            })
        })
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(self.passthrough.load(Ordering::SeqCst)) })
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        _: &'a Caller,
        _: RouteRequest,
        _: Duration,
    ) -> BoxFuture<'a, Result<Option<Duration>, ServiceError>> {
        Box::pin(async { Ok(None) })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        _: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.failures.lock().unwrap().push((account.into(), status));
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}
fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role=="assistant" {"output_text"} else {"input_text"},"text":text}]})
}
fn body(input: Vec<Value>, previous: Option<&str>, streaming: bool) -> Value {
    let mut value = json!({"model":"fixture-model","stream":streaming,"client_metadata":{"session_id":"client-session","thread_id":"client-thread","extension":"kept"},"input":input});
    if let Some(previous) = previous {
        value["previous_response_id"] = json!(previous);
    }
    value
}
fn completed(id: &str, extra: Option<Value>) -> Value {
    let mut output = vec![message("assistant", &format!("answer-{id}"))];
    if let Some(extra) = extra {
        output.push(extra);
    }
    json!({"id":id,"status":"completed","output":output})
}
fn sse(frames: Vec<Value>) -> Response {
    let chunks = frames
        .into_iter()
        .map(|event| Ok::<_, std::io::Error>(Bytes::from(format!("data: {event}\n\n"))));
    axum::http::Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream::iter(chunks)))
        .unwrap()
}
async fn upstream_http(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Response {
    let account = headers["authorization"]
        .to_str()
        .unwrap()
        .trim_start_matches("Bearer synthetic-")
        .to_owned();
    state.sent.lock().unwrap().push(Sent {
        account: account.clone(),
        headers,
        body: request.clone(),
    });
    let index = usize::from(account == "b");
    let step = state.steps.lock().unwrap()[index]
        .pop_front()
        .expect("Unexpected synthetic upstream attempt");
    match step {
        Step::RawEvents(events) => sse(events),
        Step::HttpOverload => (StatusCode::SERVICE_UNAVAILABLE,Json(json!({"error":{"type":"server_error","code":"server_error","message":"Synthetic overloaded"}}))).into_response(),
        Step::HttpRateLimit => (StatusCode::TOO_MANY_REQUESTS,Json(json!({"error":{"type":"usage_limit_reached","code":"usage_limit_reached","message":"Synthetic quota exhausted"}}))).into_response(),
        Step::SseOverload => sse(vec![json!({"type":"error","status_code":503,"error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"Synthetic overloaded"}})]),
        Step::CompleteItems(id,items) => {
            let response=json!({"id":id,"status":"completed","output":items});
            if request["stream"]==true {
                let mut frames=vec![json!({"type":"response.created","response":{"id":id,"status":"in_progress"}})];
                for (index,item) in response["output"].as_array().unwrap().iter().enumerate() {
                    frames.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
                }
                frames.push(json!({"type":"response.completed","response":response}));
                sse(frames)
            }else {Json(response).into_response()}
        }
        Step::Complete(id,extra) => {
            let response=completed(id,extra);
            let mut result=if request["stream"]==true {sse(vec![json!({"type":"response.created","response":{"id":id,"status":"in_progress"}}),json!({"type":"response.completed","response":response})])} else {Json(response).into_response()};
            result.headers_mut().insert("x-codex-turn-state",HeaderValue::from_str(&format!("synthetic-state-{id}")).unwrap());
            result
        }
        Step::Hold(id) => {
            let start=Bytes::from(format!("data: {}\n\n",json!({"type":"response.created","response":{"id":id,"status":"in_progress"}})));
            let source=stream::once(async move{Ok::<_,std::io::Error>(start)}).chain(stream::pending());
            axum::http::Response::builder().header("content-type","text/event-stream").body(Body::from_stream(source)).unwrap()
        }
    }
}
async fn upstream_ws(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    let account = headers["authorization"]
        .to_str()
        .unwrap()
        .trim_start_matches("Bearer synthetic-")
        .to_owned();
    state.ws_connections.lock().unwrap().push(account);
    upgrade.on_upgrade(move|mut socket|async move {
        while let Some(Ok(frame))=socket.recv().await {
            match frame {
                UpstreamMessage::Text(text)=> {
                    let body:Value=serde_json::from_str(&text).unwrap();
                    let response=upstream_http(State(state.clone()),headers.clone(),Json(body)).await;
                    let success=response.status().is_success();
                    let is_sse=response.headers().get("content-type").and_then(|value|value.to_str().ok()).is_some_and(|value|value.starts_with("text/event-stream"));
                    let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
                    let events=if !success {
                        vec![json!({"type":"error","status_code":503,"error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"Synthetic overloaded"}})]
                    }else if is_sse {
                        std::str::from_utf8(&bytes).unwrap().split("\n\n").filter_map(|frame|frame.strip_prefix("data: ")).map(|data|serde_json::from_str::<Value>(data).unwrap()).collect()
                    }else {
                        vec![json!({"type":"response.completed","response":serde_json::from_slice::<Value>(&bytes).unwrap()})]
                    };
                    for event in events {
                        if socket.send(UpstreamMessage::Text(event.to_string().into())).await.is_err(){return;}
                    }
                }
                UpstreamMessage::Ping(value)=>{if socket.send(UpstreamMessage::Pong(value)).await.is_err(){return;}}
                UpstreamMessage::Close(_)=>break,
                _=>{}
            }
        }
    })
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn installation_profile(account: &str) -> crate::modules::codex_api_identity::InstallationProfile {
    crate::modules::codex_api_identity::InstallationProfile {
        version: 1,
        scope: if account == "a" { "a" } else { "b" }.repeat(64),
        installation_id: if account == "a" {
            "d5a2e458-7e4a-456a-bf68-f690dcf3c256"
        } else {
            "32902d87-7d4d-454b-9a9a-d00e14db6c71"
        }
        .into(),
        revision: 1,
    }
}
struct Fixture {
    handle: Handle,
    host: Arc<TestHost>,
    upstream: Arc<Upstream>,
    task: JoinHandle<()>,
    root: tempfile::TempDir,
}
impl Fixture {
    async fn new(enabled: bool, a: Vec<Step>, b: Vec<Step>) -> Self {
        let upstream = Arc::new(Upstream {
            steps: Mutex::new([a.into(), b.into()]),
            sent: Mutex::new(vec![]),
            ws_connections: Mutex::new(vec![]),
        });
        let app = Router::new()
            .route("/a/responses", post(upstream_http).get(upstream_ws))
            .route("/b/responses", post(upstream_http).get(upstream_ws))
            .with_state(upstream.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(TestHost {
            base: format!("http://{address}"),
            enabled: AtomicBool::new(enabled),
            oauth: AtomicBool::new(false),
            passthrough: AtomicBool::new(false),
            removed_a: AtomicBool::new(false),
            selections: Mutex::new(vec![]),
            failures: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
        });
        let root = tempfile::tempdir().unwrap();
        let handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        Self {
            handle,
            host,
            upstream,
            task,
            root,
        }
    }
    fn path(&self) -> PathBuf {
        self.root.path().join("identity.sqlite")
    }
    async fn post(&self, request: &Value, turn_state: Option<&str>) -> reqwest::Response {
        let mut builder = client()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth("synthetic-caller")
            .json(request);
        if let Some(state) = turn_state {
            builder = builder.header("x-codex-turn-state", state);
        }
        builder.send().await.unwrap()
    }
    async fn root(&self) -> Value {
        let response = self
            .post(&body(vec![message("user", "first")], None, false), None)
            .await;
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }
    fn sent(&self) -> Vec<Sent> {
        self.upstream.sent.lock().unwrap().clone()
    }
    fn active(&self) -> Option<(String, String, String, i64)> {
        Connection::open(self.path()).unwrap().query_row("SELECT account,owner,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='client-thread'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().unwrap()
    }
    fn assert_owner(&self, previous: &str, account: &str) {
        let identity = IdentityStore::open(self.path())
            .unwrap()
            .inspect(
                "caller",
                &HeaderMap::new(),
                &body(vec![message("user", "inspect")], Some(previous), false),
            )
            .unwrap();
        assert_eq!(identity.state_account_id.as_deref(), Some(account));
    }
    async fn socket(
        &self,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let mut request = format!("ws://{}/v1/responses", self.handle.addr)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Bearer synthetic-caller"),
        );
        tokio_tungstenite::connect_async(request).await.unwrap().0
    }
    async fn finish(mut self) {
        self.handle.shutdown(Duration::from_secs(3)).await.unwrap();
        self.task.abort();
    }
}
fn identity(sent: &Sent) -> (Value, Value) {
    (
        sent.body["client_metadata"]["session_id"].clone(),
        sent.body["client_metadata"]["thread_id"].clone(),
    )
}
fn assert_fresh(source: &Sent, rebuilt: &Sent, expected: Value) {
    assert_ne!(identity(source), identity(rebuilt));
    assert_ne!(
        source.headers.get("session-id"),
        rebuilt.headers.get("session-id")
    );
    assert_ne!(
        source.headers.get("thread-id"),
        rebuilt.headers.get("thread-id")
    );
    assert!(rebuilt.body.get("previous_response_id").is_none());
    assert!(!rebuilt.headers.contains_key("x-codex-turn-state"));
    assert!(rebuilt.body["client_metadata"]
        .get("x-codex-turn-state")
        .is_none());
    assert!(rebuilt.body["client_metadata"]
        .get("parent_response_id")
        .is_none());
    assert_eq!(rebuilt.body["input"], expected);
    assert_eq!(rebuilt.body["client_metadata"]["extension"], "kept");
}

#[tokio::test]
async fn context_rebuild_defaults_off_and_previous_response_is_still_hard_pinned() {
    let fixture = Fixture::new(
        false,
        vec![Step::Complete("a-root", None), Step::HttpOverload],
        vec![],
    )
    .await;
    fixture.root().await;
    let original = body(vec![message("user", "second")], Some("a-root"), false);
    let response = fixture
        .post(&original, Some("synthetic-state-a-root"))
        .await;
    assert_eq!(response.status(), 503);
    response.bytes().await.unwrap();
    let sent = fixture.sent();
    assert_eq!(sent.len(), 2);
    assert!(sent.iter().all(|request| request.account == "a"));
    assert_eq!(sent[1].body["input"], original["input"]);
    assert_eq!(sent[1].body["previous_response_id"], "a-root");
    assert_eq!(identity(&sent[0]), identity(&sent[1]));
    assert!(fixture.active().is_none());
    fixture.assert_owner("a-root", "a");
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_http_and_sse_overload_replay_complete_plaintext_and_keep_client_input() {
    for failure in [Step::HttpOverload, Step::SseOverload] {
        let streaming = matches!(failure, Step::SseOverload);
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("a-root", None), failure],
            vec![Step::Complete("b-rebuilt", None)],
        )
        .await;
        fixture.root().await;
        let mut original = body(vec![message("user", "second")], Some("a-root"), streaming);
        original["client_metadata"]["x-codex-turn-state"] = json!("synthetic-state-a-root");
        original["client_metadata"]["parent_response_id"] = json!("a-root");
        let unchanged = original.clone();
        let response = fixture
            .post(&original, Some("synthetic-state-a-root"))
            .await;
        assert_eq!(response.status(), 200);
        let output = response.text().await.unwrap();
        assert!(output.contains("b-rebuilt"));
        let sent = fixture.sent();
        assert_eq!(
            sent.iter()
                .map(|request| request.account.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "a", "b"]
        );
        assert_fresh(
            &sent[1],
            &sent[2],
            json!([
                message("user", "first"),
                message("assistant", "answer-a-root"),
                message("user", "second")
            ]),
        );
        assert_eq!(sent[1].body["input"], original["input"]);
        assert_eq!(original, unchanged);
        assert_eq!(fixture.active().unwrap().0, "b");
        fixture.assert_owner("a-root", "a");
        fixture.assert_owner("b-rebuilt", "b");
        let selections = fixture.host.selections.lock().unwrap();
        assert_eq!(selections[1].preferred_account_id.as_deref(), Some("a"));
        assert_eq!(selections[2].excluded_account_ids, vec!["a"]);
        drop(selections);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_a_b_a_uses_new_a_generation_and_does_not_reassign_old_checkpoints() {
    let fixture = Fixture::new(
        true,
        vec![
            Step::Complete("a-root", None),
            Step::HttpOverload,
            Step::Complete("a-again", None),
        ],
        vec![Step::Complete("b-rebuilt", None), Step::HttpOverload],
    )
    .await;
    fixture.root().await;
    for (previous, turn, completed) in [
        ("a-root", "second", "b-rebuilt"),
        ("b-rebuilt", "third", "a-again"),
    ] {
        let response = fixture
            .post(
                &body(vec![message("user", turn)], Some(previous), false),
                None,
            )
            .await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap()["id"], completed);
    }
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|request| request.account.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b", "b", "a"]
    );
    assert_ne!(identity(&sent[0]), identity(&sent[4]));
    assert_ne!(identity(&sent[2]), identity(&sent[4]));
    assert_fresh(
        &sent[3],
        &sent[4],
        json!([
            message("user", "first"),
            message("assistant", "answer-a-root"),
            message("user", "second"),
            message("assistant", "answer-b-rebuilt"),
            message("user", "third")
        ]),
    );
    let active = fixture.active().unwrap();
    assert_eq!(
        (active.0, active.1, active.3),
        ("a".into(), "owner-a".into(), 3)
    );
    for (previous, account) in [("a-root", "a"), ("b-rebuilt", "b"), ("a-again", "a")] {
        fixture.assert_owner(previous, account);
    }
    fixture.finish().await;
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn ws_send(socket: &mut Socket, mut request: Value) {
    request["type"] = json!("response.create");
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();
}
async fn ws_event(socket: &mut Socket, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = socket
                .next()
                .await
                .expect("Synthetic socket ended before expected event")
                .unwrap();
            if let Message::Text(text) = frame {
                let event: Value = serde_json::from_str(&text).unwrap();
                if kind != "error" {
                    assert_ne!(event["type"], "error", "Unexpected WS error: {event}");
                }
                if event["type"] == kind {
                    return event;
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn context_rebuild_candidate_failure_or_client_cancel_never_activates_b() {
    for cancel in [false, true] {
        let b = if cancel {
            Step::Hold("b-pending")
        } else {
            Step::HttpOverload
        };
        let fixture = Fixture::new(
            true,
            vec![Step::Complete("a-root", None), Step::HttpOverload],
            vec![b],
        )
        .await;
        fixture.root().await;
        let before = fixture.active().unwrap();
        if cancel {
            let mut socket = fixture.socket().await;
            ws_send(
                &mut socket,
                body(vec![message("user", "second")], Some("a-root"), true),
            )
            .await;
            let event = ws_event(&mut socket, "response.created").await;
            assert_eq!(event["response"]["id"], "b-pending");
            assert_eq!(
                fixture.active().unwrap(),
                before,
                "Created is not a completed checkpoint"
            );
            socket.close(None).await.unwrap();
            drop(socket);
            tokio::time::timeout(Duration::from_secs(4), async {
                while fixture.host.reports.lock().unwrap().len() < 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        } else {
            let response = fixture
                .post(
                    &body(vec![message("user", "second")], Some("a-root"), false),
                    None,
                )
                .await;
            assert_eq!(response.status(), 503);
            response.bytes().await.unwrap();
        }
        let sent = fixture.sent();
        assert_eq!(
            sent.iter()
                .map(|request| request.account.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "a", "b"]
        );
        assert_eq!(fixture.active().unwrap(), before);
        fixture.assert_owner("a-root", "a");
        let count: i64 = Connection::open(fixture.path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1 WHERE account='b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "Failed/cancelled candidate must not persist a replay checkpoint"
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_cache_miss_reasoning_and_agent_cipher_keep_original_a_payload() {
    let reasoning =
        json!({"type":"reasoning","summary":[],"encrypted_content":"synthetic-reasoning-cipher"});
    let communication = json!({"type":"function_call","namespace":"collaboration","name":"send_message","call_id":"synthetic-call","arguments":json!({"target":"/root/worker","message":"synthetic-agent-cipher"}).to_string()});
    let agent_input = json!({"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"Payload:\n"},{"type":"encrypted_content","encrypted_content":"synthetic-agent-cipher"}]});
    for extra in [None, Some(reasoning.clone()), Some(communication)] {
        let cache_miss = extra.is_none();
        let fixture = Fixture::new(
            !cache_miss,
            vec![Step::Complete("a-root", extra.clone()), Step::HttpOverload],
            vec![],
        )
        .await;
        fixture.root().await;
        fixture.host.enabled.store(true, Ordering::SeqCst);
        // This case protects genuinely missing history. Opaque checkpoints are
        // now supported, so remove both the hot cache and accepted replay history explicitly;
        // durable response/cipher ownership must remain available on A.
        Connection::open(fixture.path())
            .unwrap()
            .execute_batch(
                "DELETE FROM native_rebuild_checkpoints_v1; DELETE FROM native_rebuild_history_v1;",
            )
            .unwrap();
        let checkpoints: i64 = Connection::open(fixture.path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(checkpoints, 0);
        let input = match extra.as_ref().and_then(|value| value["type"].as_str()) {
            Some("reasoning") => vec![reasoning.clone(), message("user", "second")],
            Some("function_call") => vec![agent_input.clone()],
            _ => vec![message("user", "second")],
        };
        let original = body(input, Some("a-root"), false);
        let unchanged = original.clone();
        let response = fixture
            .post(&original, Some("synthetic-state-a-root"))
            .await;
        assert_eq!(response.status(), 503);
        response.bytes().await.unwrap();
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|request| request.account == "a"));
        assert_eq!(sent[1].body["input"], original["input"]);
        assert_eq!(sent[1].body["previous_response_id"], "a-root");
        assert_eq!(
            sent[1].headers["x-codex-turn-state"],
            "synthetic-state-a-root"
        );
        assert_eq!(identity(&sent[0]), identity(&sent[1]));
        assert_eq!(original, unchanged);
        fixture.assert_owner("a-root", "a");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_unknown_complete_history_automatically_retries_an_available_account() {
    for failure in [Step::HttpOverload, Step::SseOverload] {
        let streaming = matches!(failure, Step::SseOverload);
        let fixture =
            Fixture::new(true, vec![failure], vec![Step::Complete("b-first", None)]).await;
        let original = body(
            vec![
                message("user", "before"),
                json!({"type":"reasoning","encrypted_content":"preexisting-cipher"}),
                message("user", "continue"),
            ],
            None,
            streaming,
        );
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("b-first"));
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].account, "a");
        assert_eq!(sent[1].account, "b");
        for request in &sent {
            assert_eq!(request.body["input"], original["input"]);
            assert!(request.body.get("previous_response_id").is_none());
        }
        assert_eq!(fixture.active().unwrap().0, "b");
        assert!(!fixture.path().with_extension("recovery").exists());
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_real_response_socket_delta_overload_bridges_with_fresh_full_history() {
    let fixture = Fixture::new(
        true,
        vec![Step::Complete("a-root", None), Step::SseOverload],
        vec![Step::Complete("b-rebuilt", None)],
    )
    .await;
    let mut socket = fixture.socket().await;
    let root = body(vec![message("user", "first")], None, true);
    ws_send(&mut socket, root).await;
    let event = ws_event(&mut socket, "response.completed").await;
    assert_eq!(event["response"]["id"], "a-root");
    let mut original = body(vec![message("user", "second")], Some("a-root"), true);
    original["client_metadata"]["x-codex-turn-state"] = json!("synthetic-state-a-root");
    original["client_metadata"]["parent_response_id"] = json!("a-root");
    let unchanged = original.clone();
    ws_send(&mut socket, original.clone()).await;
    let event = ws_event(&mut socket, "response.completed").await;
    assert_eq!(event["response"]["id"], "b-rebuilt");
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|request| request.account.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b"]
    );
    assert_fresh(
        &sent[1],
        &sent[2],
        json!([
            message("user", "first"),
            message("assistant", "answer-a-root"),
            message("user", "second")
        ]),
    );
    // The existing WS-to-HTTP bridge expands the checkpoint on A as well.
    assert!(sent[1].body.get("previous_response_id").is_none());
    assert_eq!(
        sent[1].body["input"],
        json!([
            message("user", "first"),
            message("assistant", "answer-a-root"),
            message("user", "second")
        ])
    );
    assert_eq!(original, unchanged);
    assert_eq!(fixture.active().unwrap().0, "b");
    fixture.assert_owner("a-root", "a");
    fixture.assert_owner("b-rebuilt", "b");
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_only_replays_tool_history_after_call_and_result_are_paired() {
    let call = json!({"type":"function_call","name":"lookup","call_id":"synthetic-lookup","arguments":"{\"key\":\"synthetic\"}","status":"completed"});
    let result = json!({"type":"function_call_output","call_id":"synthetic-lookup","output":"synthetic result"});
    for paired in [false, true] {
        let fixture = Fixture::new(
            true,
            vec![
                Step::Complete("a-root", Some(call.clone())),
                Step::HttpOverload,
            ],
            if paired {
                vec![Step::Complete("b-rebuilt", None)]
            } else {
                vec![]
            },
        )
        .await;
        fixture.root().await;
        let input = if paired {
            vec![result.clone(), message("user", "second")]
        } else {
            vec![message("user", "second")]
        };
        let original = body(input, Some("a-root"), false);
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), if paired { 200 } else { 503 });
        response.bytes().await.unwrap();
        let sent = fixture.sent();
        if paired {
            assert_eq!(sent.len(), 3);
            assert_eq!(sent[2].account, "b");
            assert_fresh(
                &sent[1],
                &sent[2],
                json!([
                    message("user", "first"),
                    message("assistant", "answer-a-root"),
                    call,
                    result,
                    message("user", "second")
                ]),
            );
        } else {
            assert_eq!(sent.len(), 2);
            assert!(sent.iter().all(|request| request.account == "a"));
            assert_eq!(sent[1].body["input"], original["input"]);
        }
        fixture.assert_owner("a-root", "a");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_upstream_websocket_overload_rebuilds_then_reuses_b_generation() {
    let fixture = Fixture::new(
        true,
        vec![Step::Complete("a-root", None), Step::SseOverload],
        vec![
            Step::Complete("b-rebuilt", None),
            Step::Complete("b-next", None),
            Step::SseOverload,
        ],
    )
    .await;
    fixture.host.passthrough.store(true, Ordering::SeqCst);
    let mut socket = fixture.socket().await;
    ws_send(
        &mut socket,
        body(vec![message("user", "first")], None, true),
    )
    .await;
    assert_eq!(
        ws_event(&mut socket, "response.completed").await["response"]["id"],
        "a-root"
    );
    let original = body(vec![message("user", "second")], Some("a-root"), true);
    let unchanged = original.clone();
    // The upstream handshake did not issue a turn-state token. Only use the
    // checkpoint actually observed on this client socket, never an invented token.
    ws_send(&mut socket, original.clone()).await;
    assert_eq!(
        ws_event(&mut socket, "response.completed").await["response"]["id"],
        "b-rebuilt"
    );
    let b_active = fixture.active().unwrap();
    assert_eq!(b_active.0, "b");
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|request| request.account.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b"]
    );
    assert_fresh(
        &sent[1],
        &sent[2],
        json!([
            message("user", "first"),
            message("assistant", "answer-a-root"),
            message("user", "second")
        ]),
    );
    assert_eq!(sent[1].body["previous_response_id"], "a-root");
    assert_eq!(original, unchanged);
    fixture.assert_owner("a-root", "a");
    fixture.assert_owner("b-rebuilt", "b");
    let next = body(vec![message("user", "third")], Some("b-rebuilt"), true);
    let unchanged_next = next.clone();
    ws_send(&mut socket, next.clone()).await;
    assert_eq!(
        ws_event(&mut socket, "response.completed").await["response"]["id"],
        "b-next"
    );
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|request| request.account.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b", "b"]
    );
    assert_eq!(
        identity(&sent[2]),
        identity(&sent[3]),
        "Successful B continuation must keep its upstream generation"
    );
    assert_eq!(
        sent[2].headers.get("session-id"),
        sent[3].headers.get("session-id")
    );
    assert_eq!(
        sent[2].headers.get("thread-id"),
        sent[3].headers.get("thread-id")
    );
    assert_eq!(sent[3].body["previous_response_id"], "b-rebuilt");
    assert_eq!(sent[3].body["input"], next["input"]);
    assert_eq!(
        fixture.active().unwrap(),
        b_active,
        "Same-owner continuation must not activate another generation"
    );
    assert_eq!(
        *fixture.upstream.ws_connections.lock().unwrap(),
        vec!["a".to_owned(), "b".to_owned()],
        "Three turns use A then one persistent B upstream socket"
    );
    fixture.assert_owner("b-next", "b");
    assert_eq!(next, unchanged_next);
    fixture.host.enabled.store(false, Ordering::SeqCst);
    ws_send(
        &mut socket,
        body(vec![message("user", "fourth")], Some("b-next"), true),
    )
    .await;
    let error = ws_event(&mut socket, "error").await;
    assert_eq!(
        error
            .get("status")
            .or_else(|| error.get("status_code"))
            .unwrap(),
        503,
        "The open socket must honor disabling rebuild: {error}"
    );
    let sent = fixture.sent();
    assert_eq!(
        sent.iter()
            .map(|request| request.account.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "a", "b", "b", "b"]
    );
    assert_eq!(identity(&sent[3]), identity(&sent[4]));
    assert_eq!(fixture.active().unwrap(), b_active);
    socket.close(None).await.unwrap();
    drop(socket);
    fixture.finish().await;
}

#[path = "context_rebuild_opaque_http_tests.rs"]
mod opaque_tests;

#[path = "context_rebuild_host_results_tests.rs"]
mod host_results_tests;

#[path = "context_rebuild_codex_app_failover_tests.rs"]
mod codex_app_failover_tests;

#[path = "context_rebuild_output_evidence_tests.rs"]
mod output_evidence_tests;

#[path = "account_installation_http_tests.rs"]
mod installation_tests;
