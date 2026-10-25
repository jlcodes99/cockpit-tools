//! Real loopback coverage for stateless OAuth HTTP continuation and shared Go binding.
use super::{
    continuity_bridge::{self, protocol::*},
    contract::*,
    identity::IdentityStore,
    Handle, Options,
};
use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message as WsMessage, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, HeaderValue},
    response::Response,
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const CONTROL_TOKEN: &str = "synthetic-http-checkpoint-control-token-never-upstream";

#[path = "legacy_history_continuation_tests.rs"]
mod legacy_history_continuation_tests;

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role=="assistant" {"output_text"} else {"input_text"},"text":text}]})
}
fn history() -> Value {
    json!([
        message("user", "initial synthetic request"),
        {"type":"reasoning","id":"rs_old-reasoning","summary":[],"encrypted_content":super::codex_cpa_wire::synthetic_gpt_signature(17)},
        {"type":"compaction","encrypted_content":"synthetic-compaction"},
        {"type":"function_call","id":"fc_old-call-item","namespace":"collaboration","name":"send_message","call_id":"agent-call","arguments":"{\"message\":\"synthetic-agent-cipher\"}","encrypted_function_args":["message"]},
        {"type":"function_call_output","call_id":"agent-call","output":"synthetic delivery result"},
        message("user", "continue with full tool evidence")
    ])
}
fn request(input: Value) -> Value {
    json!({"model":"gpt-5.4","stream":true,"store":false,"client_metadata":{"session_id":"root","thread_id":"root"},"input":input})
}
fn initial() -> Value {
    let mut body = request(history());
    body["instructions"] = json!("synthetic instruction");
    body["tools"] = json!([{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"query":{"type":"string"}}}}]);
    body["tool_choice"] = json!("auto");
    body["parallel_tool_calls"] = json!(true);
    body["text"] = json!({"format":{"type":"text"}});
    body["reasoning"] = json!({"effort":"low"});
    body
}
fn delta(previous: &str, text: &str) -> Value {
    let mut body = request(json!([message("user", text)]));
    body["previous_response_id"] = json!(previous);
    body
}
fn completion(id: &str) -> Value {
    json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[message("assistant","synthetic answer")]}})
}
#[derive(Default)]
struct Upstream {
    sent: Mutex<Vec<(HeaderMap, Value)>>,
    next_output: Mutex<Option<Vec<Value>>>,
}
async fn upstream(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let body = serde_json::from_slice(&bytes).unwrap();
    let mut sent = state.sent.lock().unwrap();
    sent.push((headers, body));
    let id = format!("response-{}", sent.len());
    let mut event = completion(&id);
    if let Some(output) = state.next_output.lock().unwrap().take() {
        event["response"]["output"] = json!(output);
    }
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(format!("data: {event}\n\n")))
        .unwrap()
}
async fn websocket(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        while let Some(Ok(WsMessage::Text(raw))) = socket.recv().await {
            let body = serde_json::from_str(&raw).unwrap();
            let id = {
                let mut sent = state.sent.lock().unwrap();
                sent.push((headers.clone(), body));
                format!("response-{}", sent.len())
            };
            if socket
                .send(WsMessage::Text(completion(&id).to_string().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    })
}
struct TestHost {
    base: String,
    oauth: bool,
    removed_a: AtomicBool,
}
impl Host for TestHost {
    fn authorize<'a>(&'a self, credential: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: if credential == "foreign" {
                    "foreign"
                } else {
                    "caller"
                }
                .into(),
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
            if request.preferred_account_id.as_deref() == Some("a")
                && self.removed_a.load(Ordering::SeqCst)
            {
                return Err(ServiceError::new(
                    503,
                    "account_binding_unavailable",
                    "Synthetic account A removed",
                ));
            }
            let account = request.preferred_account_id.as_deref().unwrap_or_else(|| {
                if self.removed_a.load(Ordering::SeqCst)
                    || request.excluded_account_ids.iter().any(|id| id == "a")
                {
                    "b"
                } else {
                    "a"
                }
            });
            Ok(Route {
                continuity_account_id: None,
                account_id: account.into(),
                owner_id: format!("owner-{account}"),
                base_url: self.base.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: client(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
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
                credential_retries: 1,
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
        Box::pin(async { Ok(true) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
struct Fixture {
    root: tempfile::TempDir,
    host: Arc<TestHost>,
    source: Arc<Upstream>,
    gateway: Option<Handle>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn new(oauth: bool) -> Self {
        let source = Arc::new(Upstream::default());
        let app = Router::new()
            .route("/responses", post(upstream).get(websocket))
            .layer(axum::extract::DefaultBodyLimit::disable())
            .with_state(source.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = Arc::new(TestHost {
            base: format!("http://{}", listener.local_addr().unwrap()),
            oauth,
            removed_a: AtomicBool::new(false),
        });
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut result = Self {
            root: tempfile::tempdir().unwrap(),
            host,
            source,
            gateway: None,
            task,
        };
        result.start(Options::default()).await;
        result
    }
    fn path(&self) -> PathBuf {
        self.root.path().join("identity.sqlite")
    }
    async fn start(&mut self, options: Options) {
        self.gateway = Some(
            super::start(
                "127.0.0.1:0".parse().unwrap(),
                self.host.clone(),
                self.path(),
                options,
            )
            .await
            .unwrap(),
        );
    }
    async fn stop_native(&mut self) {
        if let Some(mut handle) = self.gateway.take() {
            handle
                .shutdown(std::time::Duration::from_secs(3))
                .await
                .unwrap();
        }
    }
    async fn finish(mut self) {
        self.stop_native().await;
        self.task.abort();
    }
    async fn send(&self, body: &Value, caller: &str) -> reqwest::Response {
        client()
            .post(format!(
                "http://{}/v1/responses",
                self.gateway.as_ref().unwrap().addr
            ))
            .bearer_auth(caller)
            .json(body)
            .send()
            .await
            .unwrap()
    }
    async fn complete(&self, body: &Value) -> Value {
        let response = self.send(body, "caller").await;
        assert_eq!(response.status(), 200);
        let raw = response.text().await.unwrap();
        raw.split("\n\n")
            .filter_map(|frame| frame.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find(|event| event["type"] == "response.completed")
            .expect("real terminal event")["response"]
            .clone()
    }
    fn sent(&self) -> Vec<(HeaderMap, Value)> {
        self.source.sent.lock().unwrap().clone()
    }
    fn generation(&self) -> (String, String, i64) {
        Connection::open(self.path()).unwrap().query_row("SELECT account,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='root'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
    }
    fn bridge(&self) -> Arc<continuity_bridge::Handle> {
        continuity_bridge::start(continuity_bridge::Config {
            identity_path: self.path(),
            token: CONTROL_TOKEN.into(),
            session_identity_mapping_enabled: true,
            owners: HashMap::from([
                ("a".into(), "owner-a".into()),
                ("b".into(), "owner-b".into()),
            ]),
            caller_accounts: HashMap::from([
                ("caller".into(), vec!["a".into(), "b".into()]),
                ("foreign".into(), vec!["a".into()]),
            ]),
        })
        .unwrap()
    }
}
fn append(mut input: Value, items: impl IntoIterator<Item = Value>) -> Value {
    input.as_array_mut().unwrap().extend(items);
    input
}
fn checkpoint(path: &Path, id: &str) -> Value {
    let raw:String=Connection::open(path).unwrap().query_row("SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response=?1",[id],|r|r.get(0)).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[tokio::test]
async fn large_http_delta_replays_complete_history_on_same_and_replacement_accounts() {
    const LARGE: usize = 17 * 1024 * 1024;
    for replacement in [false, true] {
        let mut fixture = Fixture::new(true).await;
        let first = fixture.complete(&initial()).await;
        fixture.host.removed_a.store(replacement, Ordering::SeqCst);
        let next = delta(first["id"].as_str().unwrap(), &"x".repeat(LARGE));
        let second = fixture.complete(&next).await;
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        let expected = append(
            append(history(), first["output"].as_array().unwrap().clone()),
            next["input"].as_array().unwrap().clone(),
        );
        assert_eq!(sent[1].1["input"], expected, "replacement={replacement}");
        assert!(sent[1].1.get("previous_response_id").is_none());
        let owner = if replacement { "b" } else { "a" };
        assert_eq!(fixture.generation().0, owner);
        for header in ["session-id", "thread-id"] {
            assert_eq!(sent[0].0.get(header) != sent[1].0.get(header), replacement);
        }
        assert!(checkpoint(&fixture.path(), first["id"].as_str().unwrap()).is_array());
        // The large checkpoint is not admitted to the 8 MiB hot cache. A new
        // process must still accept a pure-ID delta and inherit exact history.
        fixture.stop_native().await;
        fixture.start(Options::default()).await;
        let third = fixture
            .complete(&delta(second["id"].as_str().unwrap(), "after restart"))
            .await;
        let expected = append(
            append(expected, second["output"].as_array().unwrap().clone()),
            [message("user", "after restart")],
        );
        assert_eq!(fixture.sent().last().unwrap().1["input"], expected);
        assert_eq!(fixture.generation().0, owner);
        // The other engine reads the same durable record without requiring the
        // client to resend the large prefix; its result is readable by Rust.
        fixture.stop_native().await;
        let bridge = fixture.bridge();
        let source = delta(third["id"].as_str().unwrap(), "large native to Go");
        let prepared = bridge_prepare(&bridge, "large-checkpoint-handoff", source.clone()).await;
        let bound: BindResponse = bridge_call(
            &bridge,
            "bind",
            &BindRequest {
                ticket: prepared.ticket.clone(),
                attempt_id: "attempt".into(),
                account_id: owner.into(),
                owner_id: Some(format!("owner-{owner}")),
                expand_http_previous: true,
                headers: prepared.headers.clone(),
                body: source,
            },
        )
        .await;
        let expected = append(
            append(expected, third["output"].as_array().unwrap().clone()),
            [message("user", "large native to Go")],
        );
        assert_eq!(bound.body["input"], expected);
        assert_eq!(bound.body["tools"], initial()["tools"]);
        bridge_complete(&bridge, &prepared, completion("large-go-result")).await;
        bridge.stop().await.unwrap();
        fixture.start(Options::default()).await;
        fixture
            .complete(&delta("large-go-result", "Go back to Rust"))
            .await;
        assert_eq!(
            fixture.sent().last().unwrap().1["input"],
            append(
                append(expected, [message("assistant", "synthetic answer")]),
                [message("user", "Go back to Rust")],
            )
        );
        fixture.finish().await;
    }
}

#[tokio::test]
async fn continuity_bridge_large_wire_requests_reach_scope_checks_and_valid_history_binds() {
    const LARGE: usize = 65 * 1024 * 1024;
    let fixture = Fixture::new(true).await;
    let bridge = fixture.bridge();
    let source = request(json!([message("user", &"x".repeat(LARGE))]));
    let response = client()
        .post(format!("http://{}/v1/continuity/prepare", bridge.addr))
        .bearer_auth(CONTROL_TOKEN)
        .json(&PrepareRequest {
            request_id: "large-wire-scope-check".into(),
            caller: "unconfigured-caller".into(),
            headers: Headers::new(),
            body: source,
            compact: false,
            allow_rebuild: true,
        })
        .send()
        .await
        .unwrap();
    // A real body beyond the former 64 MiB cap reaches its caller-scope check.
    // It cannot allocate a ticket or bypass the independent state memory budget.
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "continuity_caller_scope"
    );
    let source = request(json!([message("user", &"x".repeat(17 * 1024 * 1024))]));
    let prepared = bridge_prepare(&bridge, "large-envelope", source.clone()).await;
    assert_eq!(prepared.body, source);
    let bound = bridge_bind(&bridge, &prepared, false, source.clone()).await;
    assert_eq!(bound.body["input"], source["input"]);
    for header in ["session-id", "thread-id"] {
        assert!(!bound.headers.get(header).unwrap().is_empty());
    }
    let _: ReleaseResponse = bridge_call(
        &bridge,
        "release",
        &ReleaseRequest {
            ticket: prepared.ticket,
        },
    )
    .await;
    bridge.stop().await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn continuity_bridge_rejects_invalid_tokens_before_waiting_for_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::new(true).await;
    let bridge = fixture.bridge();
    let mut socket = tokio::net::TcpStream::connect(bridge.addr).await.unwrap();
    socket.write_all(b"POST /v1/continuity/prepare HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer invalid-token\r\nContent-Type: application/json\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n{").await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        socket.read_to_string(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(response.starts_with("HTTP/1.1 401"), "{response}");
    assert!(response.contains("invalid_continuity_token"), "{response}");
    for (mime, body, status) in [
        ("application/foo/bar+json", "{}", 415),
        ("application/json; invalid", "{}", 415),
        ("text/json", "{}", 415),
        ("application/problem+json; charset=utf-8", "{}", 422),
        ("application/json", "{", 400),
        ("application/json", "{\"request_id\":false}", 422),
    ] {
        let response = client()
            .post(format!("http://{}/v1/continuity/prepare", bridge.addr))
            .bearer_auth(CONTROL_TOKEN)
            .header("content-type", mime)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            status,
            "{mime}: {}",
            response.text().await.unwrap()
        );
    }
    bridge.stop().await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_http_previous_expands_same_account_twice_without_rotating_identity_or_losing_cipher()
{
    let fixture = Fixture::new(true).await;
    let first = fixture.complete(&initial()).await;
    let active = fixture.generation();
    let mut next = delta(first["id"].as_str().unwrap(), "second delta");
    next["reasoning"] = json!({"effort":"medium"});
    let second = fixture.complete(&next).await;
    let third = fixture
        .complete(&delta(second["id"].as_str().unwrap(), "third delta"))
        .await;
    let sent = fixture.sent();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].1["input"], history());
    let expected_second = append(
        append(history(), first["output"].as_array().unwrap().clone()),
        next["input"].as_array().unwrap().clone(),
    );
    assert_eq!(sent[1].1["input"], expected_second);
    assert_eq!(
        sent[2].1["input"],
        append(
            append(
                expected_second,
                second["output"].as_array().unwrap().clone()
            ),
            [message("user", "third delta")]
        )
    );
    for index in [1, 2] {
        assert!(sent[index].1.get("previous_response_id").is_none());
        for key in ["instructions", "tools", "tool_choice", "text"] {
            assert_eq!(sent[index].1[key], sent[0].1[key], "inherited {key}");
        }
        assert_eq!(sent[index].1["reasoning"]["effort"], "medium");
        for header in ["session-id", "thread-id"] {
            assert_eq!(
                sent[index].0.get(header),
                sent[0].0.get(header),
                "same-account {header}"
            );
        }
    }
    assert_eq!(
        fixture.generation(),
        active,
        "HTTP expansion must not rotate the generation"
    );
    assert_eq!(
        checkpoint(&fixture.path(), third["id"].as_str().unwrap()),
        append(
            sent[2].1["input"].clone(),
            third["output"].as_array().unwrap().clone()
        )
    );
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_http_full_history_with_previous_is_not_duplicated_and_explicit_clears_win() {
    let fixture = Fixture::new(true).await;
    let first = fixture.complete(&initial()).await;
    let input = append(
        append(history(), first["output"].as_array().unwrap().clone()),
        [message("user", "full replay")],
    );
    let mut next = delta(first["id"].as_str().unwrap(), "unused");
    next["input"] = input.clone();
    next["instructions"] = json!("");
    next["tools"] = json!([]);
    next["tool_choice"] = json!("none");
    next["text"] = json!({"format":{"type":"json_object"}});
    next["reasoning"] = json!({"effort":"high"});
    fixture.complete(&next).await;
    let sent = fixture.sent();
    assert_eq!(sent[1].1["input"], input);
    for key in ["instructions", "tools", "tool_choice", "text", "reasoning"] {
        assert_eq!(sent[1].1[key], next[key], "explicit {key}");
    }
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_http_missing_checkpoint_foreign_caller_and_size_limit_fail_before_upstream() {
    for mode in ["missing", "manifest", "foreign", "size"] {
        let mut fixture = Fixture::new(true).await;
        let first = fixture.complete(&initial()).await;
        let id = first["id"].as_str().unwrap();
        let active = fixture.generation();
        if mode == "missing" {
            Connection::open(fixture.path())
                .unwrap()
                .execute(
                    "DELETE FROM native_rebuild_checkpoints_v1 WHERE response=?1",
                    [id],
                )
                .unwrap();
            Connection::open(fixture.path())
                .unwrap()
                .execute(
                    "DELETE FROM native_rebuild_history_v1 WHERE response=?1",
                    [id],
                )
                .unwrap();
        }
        if mode == "manifest" {
            Connection::open(fixture.path())
                .unwrap()
                .execute(
                    "UPDATE native_rebuild_checkpoints_v1 SET parameters=NULL WHERE response=?1",
                    [id],
                )
                .unwrap();
        }
        if mode == "size" {
            fixture.stop_native().await;
            fixture
                .start(Options {
                    max_body_bytes: 1024,
                    ..Default::default()
                })
                .await;
        }
        let response = fixture
            .send(
                &delta(id, "synthetic delta"),
                if mode == "foreign" {
                    "foreign"
                } else {
                    "caller"
                },
            )
            .await;
        assert_eq!(
            response.status(),
            if mode == "size" { 413 } else { 409 },
            "{mode}"
        );
        let error: Value = response.json().await.unwrap();
        assert_eq!(
            error["error"]["code"],
            match mode {
                "foreign" => "upstream_state_caller_mismatch",
                "size" => "continuation_history_too_large",
                _ => "continuation_history_incomplete",
            },
            "{mode}: {error}"
        );
        assert_eq!(
            fixture.sent().len(),
            1,
            "{mode} cannot forward an incomplete prompt"
        );
        assert_eq!(fixture.generation(), active);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn api_key_http_and_oauth_websocket_keep_supported_previous_response_state() {
    for oauth in [false, true] {
        let fixture = Fixture::new(oauth).await;
        let first = fixture.complete(&initial()).await;
        let body = delta(
            first["id"].as_str().unwrap(),
            "supported incremental transport",
        );
        if oauth {
            let mut request = format!(
                "ws://{}/v1/responses",
                fixture.gateway.as_ref().unwrap().addr
            )
            .into_client_request()
            .unwrap();
            request
                .headers_mut()
                .insert("authorization", HeaderValue::from_static("Bearer caller"));
            let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            let mut create = body.clone();
            create["type"] = json!("response.create");
            socket
                .send(Message::Text(create.to_string().into()))
                .await
                .unwrap();
            loop {
                let event = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                if let Message::Text(raw) = event {
                    let value: Value = serde_json::from_str(&raw).unwrap();
                    assert_ne!(value["type"], "error", "{value}");
                    if value["type"] == "response.completed" {
                        break;
                    }
                }
            }
            socket.close(None).await.unwrap();
        } else {
            fixture.complete(&body).await;
        }
        let sent = fixture.sent();
        assert_eq!(sent[1].1["previous_response_id"], first["id"]);
        assert_eq!(sent[1].1["input"], body["input"]);
        fixture.finish().await;
    }
}

async fn bridge_call<T: serde::Serialize, R: serde::de::DeserializeOwned>(
    handle: &continuity_bridge::Handle,
    operation: &str,
    request: &T,
) -> R {
    let response = client()
        .post(format!("http://{}/v1/continuity/{operation}", handle.addr))
        .bearer_auth(CONTROL_TOKEN)
        .json(request)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, 200, "{operation}: {body}");
    serde_json::from_str(&body).unwrap()
}
async fn bridge_prepare(
    handle: &continuity_bridge::Handle,
    id: &str,
    body: Value,
) -> PrepareResponse {
    bridge_call(
        handle,
        "prepare",
        &PrepareRequest {
            request_id: id.into(),
            caller: "caller".into(),
            headers: Headers::new(),
            body,
            compact: false,
            allow_rebuild: true,
        },
    )
    .await
}
async fn bridge_bind(
    handle: &continuity_bridge::Handle,
    prepared: &PrepareResponse,
    expand: bool,
    body: Value,
) -> BindResponse {
    bridge_call(
        handle,
        "bind",
        &BindRequest {
            ticket: prepared.ticket.clone(),
            attempt_id: "attempt".into(),
            account_id: "a".into(),
            owner_id: Some("owner-a".into()),
            expand_http_previous: expand,
            headers: prepared.headers.clone(),
            body,
        },
    )
    .await
}
async fn bridge_complete(
    handle: &continuity_bridge::Handle,
    prepared: &PrepareResponse,
    event: Value,
) {
    let _: ObserveResponse = bridge_call(
        handle,
        "observe",
        &ObserveRequest {
            ticket: prepared.ticket.clone(),
            attempt_id: "attempt".into(),
            sequence: 1,
            headers: None,
            events: vec![event],
        },
    )
    .await;
    let _: FinishResponse = bridge_call(
        handle,
        "finish",
        &FinishRequest {
            ticket: prepared.ticket.clone(),
            attempt_id: "attempt".into(),
            sequence: 2,
            status: FinishStatus::Succeeded,
            error_code: None,
        },
    )
    .await;
    let _: ReleaseResponse = bridge_call(
        handle,
        "release",
        &ReleaseRequest {
            ticket: prepared.ticket.clone(),
        },
    )
    .await;
}

#[tokio::test]
async fn native_and_go_bridge_http_incremental_checkpoints_roundtrip_without_account_rotation() {
    let mut fixture = Fixture::new(true).await;
    let native = fixture.complete(&initial()).await;
    let active = fixture.generation();
    fixture.stop_native().await;
    let bridge = fixture.bridge();
    let source = delta(native["id"].as_str().unwrap(), "native to Go delta");
    let prepared = bridge_prepare(&bridge, "go-incremental", source.clone()).await;
    let mut normalized = source.clone();
    normalized["instructions"] = json!("");
    normalized["tools"] = json!([]);
    normalized["reasoning"] = json!({"effort":"high"});
    let bound = bridge_bind(&bridge, &prepared, true, normalized).await;
    assert!(bound.rebuild_applied);
    assert!(bound.body.get("previous_response_id").is_none());
    assert_eq!(
        bound.body["tools"],
        initial()["tools"],
        "normalizer defaults must not override inherited tools"
    );
    assert_eq!(bound.body["instructions"], initial()["instructions"]);
    assert_eq!(bound.body["reasoning"], initial()["reasoning"]);
    let response = client()
        .post(format!("{}/responses", fixture.host.base))
        .json(&bound.body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    let terminal = raw
        .split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .expect("raw mock terminal");
    let go_id = terminal["response"]["id"].as_str().unwrap().to_owned();
    bridge_complete(&bridge, &prepared, terminal).await;
    bridge.stop().await.unwrap();
    assert_eq!(fixture.generation(), active);
    fixture.start(Options::default()).await;
    let returned = fixture
        .complete(&delta(&go_id, "Go back to native delta"))
        .await;
    let sent = fixture.sent();
    let expected = append(
        append(
            bound.body["input"].clone(),
            [message("assistant", "synthetic answer")],
        ),
        [message("user", "Go back to native delta")],
    );
    assert_eq!(sent.last().unwrap().1["input"], expected);
    assert!(sent.last().unwrap().1.get("previous_response_id").is_none());
    assert_eq!(fixture.generation(), active);
    assert_eq!(
        checkpoint(&fixture.path(), returned["id"].as_str().unwrap()),
        append(expected, returned["output"].as_array().unwrap().clone())
    );
    fixture.finish().await;
}

#[tokio::test]
async fn bridge_http_capability_is_optional_and_explicit_null_parameters_are_not_inherited() {
    let mut fixture = Fixture::new(true).await;
    let native = fixture.complete(&initial()).await;
    fixture.stop_native().await;
    let bridge = fixture.bridge();
    let source = delta(native["id"].as_str().unwrap(), "old worker or WS");
    let prepared = bridge_prepare(&bridge, "default-false", source.clone()).await;
    let old_wire = json!({"ticket":prepared.ticket,"attempt_id":"old-attempt","account_id":"a","body":source,"headers":{}});
    let bound: BindResponse = bridge_call(&bridge, "bind", &old_wire).await;
    assert!(!bound.rebuild_applied);
    assert_eq!(bound.body["previous_response_id"], native["id"]);
    let _: FinishResponse = bridge_call(
        &bridge,
        "finish",
        &FinishRequest {
            ticket: prepared.ticket.clone(),
            attempt_id: "old-attempt".into(),
            sequence: 1,
            status: FinishStatus::Failed,
            error_code: None,
        },
    )
    .await;
    let mut cleared = delta(native["id"].as_str().unwrap(), "explicit clearing");
    for key in [
        "instructions",
        "tools",
        "tool_choice",
        "text",
        "reasoning",
        "parallel_tool_calls",
    ] {
        cleared[key] = Value::Null;
    }
    let prepared = bridge_prepare(&bridge, "explicit-null", cleared.clone()).await;
    let bound = bridge_bind(&bridge, &prepared, true, cleared).await;
    for key in [
        "instructions",
        "tools",
        "tool_choice",
        "text",
        "reasoning",
        "parallel_tool_calls",
    ] {
        assert_eq!(bound.body.get(key), Some(&Value::Null), "explicit {key}");
    }
    bridge.stop().await.unwrap();
    fixture.finish().await;
}

#[tokio::test]
async fn oauth_http_checkpoint_expansion_preserves_cross_account_rebuild_and_cipher_history() {
    for string_delta in [false, true] {
        let fixture = Fixture::new(true).await;
        let first = fixture.complete(&initial()).await;
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let mut next = delta(first["id"].as_str().unwrap(), "account A removed");
        if string_delta {
            next["input"] = json!("account A removed");
        }
        fixture.complete(&next).await;
        let sent = fixture.sent();
        assert!(sent[1].1.get("previous_response_id").is_none());
        for key in ["instructions", "tools", "tool_choice", "text", "reasoning"] {
            assert_eq!(
                sent[1].1[key], sent[0].1[key],
                "cross-account inherited {key}"
            );
        }
        assert_eq!(
            sent[1].1["input"],
            append(
                append(history(), first["output"].as_array().unwrap().clone()),
                [message("user", "account A removed")]
            )
        );
        assert_eq!(fixture.generation().0, "b");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn oauth_http_string_input_checkpoints_the_first_turn_and_two_incremental_turns() {
    for metadata in [true, false] {
        let fixture = Fixture::new(true).await;
        let mut first = initial();
        first["input"] = json!("string first turn");
        if !metadata {
            first.as_object_mut().unwrap().remove("client_metadata");
        }
        let mut response = fixture.complete(&first).await;
        let mut expected = json!([message("user", "string first turn")]);
        assert_eq!(fixture.sent()[0].1["input"], expected);
        assert_eq!(
            checkpoint(&fixture.path(), response["id"].as_str().unwrap()),
            append(
                expected.clone(),
                response["output"].as_array().unwrap().clone()
            )
        );
        for text in ["string second turn", "string third turn"] {
            expected = append(
                append(expected, response["output"].as_array().unwrap().clone()),
                [message("user", text)],
            );
            let mut next = delta(response["id"].as_str().unwrap(), text);
            next["input"] = json!(text);
            if !metadata {
                next.as_object_mut().unwrap().remove("client_metadata");
            }
            response = fixture.complete(&next).await;
            let sent = fixture.sent();
            let actual = sent.last().unwrap();
            assert_eq!(actual.1["input"], expected);
            assert!(actual.1.get("previous_response_id").is_none());
            for name in ["session-id", "thread-id"] {
                assert_eq!(
                    actual.0.get(name),
                    sent[0].0.get(name),
                    "generic SDK identity must remain stable"
                );
            }
            assert_eq!(
                checkpoint(&fixture.path(), response["id"].as_str().unwrap()),
                append(
                    expected.clone(),
                    response["output"].as_array().unwrap().clone()
                )
            );
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn go_bridge_oauth_http_string_first_turn_and_delta_capture_complete_checkpoints() {
    let mut fixture = Fixture::new(true).await;
    fixture.stop_native().await;
    let bridge = fixture.bridge();
    let mut source = request(json!("bridge string first turn"));
    let mut expected = json!([message("user", "bridge string first turn")]);
    for (index, text) in ["bridge string first turn", "bridge string delta"]
        .into_iter()
        .enumerate()
    {
        let prepared = bridge_prepare(&bridge, &format!("string-{index}"), source.clone()).await;
        let bound = bridge_bind(&bridge, &prepared, true, source.clone()).await;
        assert_eq!(bound.body["input"], expected);
        assert!(bound.body.get("previous_response_id").is_none());
        assert_eq!(bound.rebuild_applied, index > 0);
        let response = client()
            .post(format!("{}/responses", fixture.host.base))
            .json(&bound.body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let raw = response.text().await.unwrap();
        let event: Value = raw
            .split("\n\n")
            .filter_map(|frame| frame.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find(|event| event["type"] == "response.completed")
            .unwrap();
        let id = event["response"]["id"].as_str().unwrap().to_owned();
        let output = event["response"]["output"].as_array().unwrap().clone();
        bridge_complete(&bridge, &prepared, event).await;
        assert_eq!(
            checkpoint(&fixture.path(), &id),
            append(expected.clone(), output.clone())
        );
        if index == 0 {
            expected = append(
                append(expected, output),
                [message("user", "bridge string delta")],
            );
            source = delta(&id, text);
            source["input"] = json!("bridge string delta");
        }
    }
    bridge.stop().await.unwrap();
    fixture.finish().await;
}
