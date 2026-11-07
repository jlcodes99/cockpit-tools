//! Synthetic-only loopback coverage for collaboration tool ciphertext ownership.
//! Go sources, real accounts, rollout files and real ciphertext are never used.
use crate::modules::codex_native_api::{
    contract::*, identity::IdentityStore, start, Handle, Options,
};
use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, stream, SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

#[derive(Clone, Copy)]
enum ParentTransport {
    Sse,
    WebSocket,
}

struct TestHost {
    base: String,
    default_b: AtomicBool,
    force_b: AtomicBool,
    selections: Mutex<Vec<(String, RouteRequest)>>,
}
impl Host for TestHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if !matches!(key, "caller-a" | "caller-b") {
                return Err(ServiceError::new(
                    401,
                    "invalid_api_key",
                    "Unknown synthetic key",
                ));
            }
            Ok(Caller {
                id: key.into(),
                label: "synthetic".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.selections
                .lock()
                .unwrap()
                .push((caller.id.clone(), request.clone()));
            let account = if self.force_b.load(Ordering::SeqCst) {
                "b"
            } else {
                request.preferred_account_id.as_deref().unwrap_or(
                    if self.default_b.load(Ordering::SeqCst) {
                        "b"
                    } else {
                        "a"
                    },
                )
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
                base_url: format!("{}/{account}", self.base),
                model: "fixture-model".into(),
                headers: HeaderMap::from_iter([(
                    axum::http::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer upstream-{account}")).unwrap(),
                )]),
                client: client(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
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
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(true) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

struct Upstream {
    tool: Value,
    completed: Value,
    release: watch::Sender<bool>,
    requests: Mutex<Vec<(String, HeaderMap, Value)>>,
    http_failure: AtomicBool,
    terminal_failure: AtomicBool,
}
fn is_child(body: &Value) -> bool {
    body["input"]
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item["type"] == "agent_message"))
}
async fn wait_release(mut release: watch::Receiver<bool>) {
    while !*release.borrow() {
        if release.changed().await.is_err() {
            break;
        }
    }
}
async fn upstream_http(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let account = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim_start_matches("Bearer upstream-")
        .to_owned();
    state
        .requests
        .lock()
        .unwrap()
        .push((account, headers, body.clone()));
    if is_child(&body) {
        return Json(json!({"id":"child-response","status":"completed","output":[]}))
            .into_response();
    }
    if state.http_failure.load(Ordering::SeqCst) {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"code":"server_error","message":"synthetic recovery failure"}})),
        )
            .into_response();
    }
    if body["stream"] != true {
        return Json(state.completed["response"].clone()).into_response();
    }
    let frames = vec![
        json!({"type":"response.created","response":{"id":"parent-response","status":"in_progress"}}),
        json!({"type":"response.output_item.done","output_index":0,"item":state.tool}),
        if state.terminal_failure.load(Ordering::SeqCst) {
            json!({"type":"error","status":503,"error":{"code":"server_error","message":"synthetic recovery failure"}})
        } else {
            state.completed.clone()
        },
    ];
    let release = state.release.subscribe();
    let source = stream::unfold(
        (0usize, frames, release),
        |(index, frames, release)| async move {
            if index == frames.len() {
                return None;
            }
            if index == 2 {
                wait_release(release.clone()).await;
            }
            let frame = Bytes::from(format!("data: {}\n\n", frames[index]));
            Some((Ok::<_, std::io::Error>(frame), (index + 1, frames, release)))
        },
    );
    axum::http::Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(source))
        .unwrap()
}
async fn upstream_ws(
    State(state): State<Arc<Upstream>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move|mut socket|async move{
        if let Some(Ok(Message::Text(text)))=socket.recv().await {
            let body:Value=serde_json::from_str(&text).unwrap();
            let account=headers.get("authorization").and_then(|value|value.to_str().ok()).unwrap_or("").trim_start_matches("Bearer upstream-").to_owned();
            state.requests.lock().unwrap().push((account,headers,body));
            for frame in [json!({"type":"response.created","response":{"id":"parent-response","status":"in_progress"}}),json!({"type":"response.output_item.done","output_index":0,"item":state.tool})] {
                if socket.send(Message::Text(frame.to_string().into())).await.is_err(){return;}
            }
            wait_release(state.release.subscribe()).await;
            let _=socket.send(Message::Text(state.completed.to_string().into())).await;
            while let Some(Ok(message))=socket.recv().await {if matches!(message,Message::Close(_)){break;}}
        }
    })
}

struct Fixture {
    handle: Option<Handle>,
    host: Arc<TestHost>,
    upstream: Arc<Upstream>,
    task: JoinHandle<()>,
    root: tempfile::TempDir,
    cipher: String,
}
impl Fixture {
    async fn new(tool: &str, plaintext: bool) -> Self {
        let cipher = format!("synthetic-agent-cipher-{tool}");
        let arguments = if tool == "spawn_agent" {
            json!({"task_name":"worker","message":cipher})
        } else {
            json!({"target":"/root/worker","message":cipher})
        };
        let mut item = json!({"type":"function_call","id":"communication-item","call_id":"communication-call","namespace":"collaboration","name":tool,"arguments":arguments.to_string(),"status":"completed"});
        if plaintext {
            item["encrypted_function_args"] = json!([]);
        }
        let (release, _) = watch::channel(false);
        let upstream = Arc::new(Upstream {
            completed: json!({"type":"response.completed","response":{"id":"parent-response","status":"completed","output":[item]}}),
            tool: item,
            release,
            requests: Mutex::new(vec![]),
            http_failure: AtomicBool::new(false),
            terminal_failure: AtomicBool::new(false),
        });
        let app = Router::new()
            .route("/a/responses", post(upstream_http).get(upstream_ws))
            .route("/b/responses", post(upstream_http).get(upstream_ws))
            .with_state(upstream.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(TestHost {
            base: format!("http://{addr}"),
            default_b: AtomicBool::new(false),
            force_b: AtomicBool::new(false),
            selections: Mutex::new(vec![]),
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
            handle: Some(handle),
            host,
            upstream,
            task,
            root,
            cipher,
        }
    }
    fn address(&self) -> String {
        format!("http://{}", self.handle.as_ref().unwrap().addr)
    }
    fn parent_body(&self, stream: bool) -> Value {
        json!({"model":"fixture-model","stream":stream,"client_metadata":{"session_id":"session","thread_id":"parent"},"input":[{"role":"user","content":"Synthetic agent communication fixture"}]})
    }
    fn child_body(&self, cipher: &str) -> Value {
        json!({"model":"fixture-model","client_metadata":{"session_id":"session","thread_id":"child","parent_thread_id":"parent"},"input":[{"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"Payload:\n"},{"type":"encrypted_content","encrypted_content":cipher}]}]})
    }
    async fn child(&self, key: &str, body: &Value) -> reqwest::Response {
        client()
            .post(format!("{}/v1/responses", self.address()))
            .bearer_auth(key)
            .json(body)
            .send()
            .await
            .unwrap()
    }
    fn assert_persisted(&self, body: &Value) {
        let store = IdentityStore::open(self.root.path().join("identity.sqlite")).unwrap();
        let identity = store.inspect("caller-a", &HeaderMap::new(), body).unwrap();
        assert_eq!(identity.state_account_id.as_deref(), Some("a"));
    }
    fn assert_original_ownership(&self) {
        let connection =
            rusqlite::Connection::open(self.root.path().join("identity.sqlite")).unwrap();
        for (kind, value) in [
            ("encrypted", self.cipher.as_str()),
            ("response", "parent-response"),
        ] {
            let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
            let owner: (String, String) = connection
                .query_row(
                    "SELECT account,owner FROM native_opaque_v1 WHERE caller='caller-a' AND kind=?1 AND digest=?2",
                    rusqlite::params![kind, digest],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(owner, ("a".into(), "owner-a".into()));
        }
    }
    async fn restart(&mut self) {
        self.handle
            .as_mut()
            .unwrap()
            .shutdown(Duration::from_secs(3))
            .await
            .unwrap();
        self.handle = None;
        self.handle = Some(
            start(
                "127.0.0.1:0".parse().unwrap(),
                self.host.clone(),
                self.root.path().join("identity.sqlite"),
                Options::default(),
            )
            .await
            .unwrap(),
        );
    }
    async fn finish(mut self) {
        self.upstream.release.send_replace(true);
        if let Some(handle) = &mut self.handle {
            handle.shutdown(Duration::from_secs(3)).await.unwrap();
        }
        self.task.abort();
    }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn agent_communication_json_tool_messages_survive_restart_and_replay_without_transferring_ownership(
) {
    for tool in ["spawn_agent", "send_message", "followup_task"] {
        let mut fixture = Fixture::new(tool, false).await;
        let parent = client()
            .post(format!("{}/v1/responses", fixture.address()))
            .bearer_auth("caller-a")
            .json(&fixture.parent_body(false))
            .send()
            .await
            .unwrap();
        assert_eq!(parent.status(), 200);
        let response: Value = parent.json().await.unwrap();
        assert_eq!(response["output"][0], fixture.upstream.tool);
        assert!(response["output"][0]
            .get("encrypted_function_args")
            .is_none());
        let child = fixture.child_body(&fixture.cipher);
        fixture.assert_persisted(&child);
        fixture.restart().await;
        fixture.host.default_b.store(true, Ordering::SeqCst);
        let result = fixture.child("caller-a", &child).await;
        assert_eq!(result.status(), 200);
        result.bytes().await.unwrap();
        let seen = fixture.upstream.requests.lock().unwrap();
        let sent = seen.last().unwrap();
        assert_eq!(sent.0, "b");
        assert_eq!(sent.2["input"], child["input"]);
        let parent = &seen[0];
        for field in ["session_id", "thread_id"] {
            assert_ne!(
                sent.2["client_metadata"][field],
                parent.2["client_metadata"][field]
            );
            assert_ne!(
                sent.2["client_metadata"][field],
                child["client_metadata"][field]
            );
        }
        assert!(sent.2.get("previous_response_id").is_none());
        drop(seen);
        // A full child payload can run on B; the original ciphertext and parent
        // response still belong to A, including after the service restarts.
        fixture.assert_original_ownership();
        let rejected = fixture.child("caller-b", &child).await;
        assert_eq!(rejected.status(), 409);
        assert_eq!(
            rejected.json::<Value>().await.unwrap()["error"]["code"],
            "upstream_state_caller_mismatch"
        );
        let unknown = fixture.child_body("synthetic-unknown-cipher");
        let accepted = fixture.child("caller-a", &unknown).await;
        assert_eq!(accepted.status(), 200);
        accepted.bytes().await.unwrap();
        assert_eq!(
            fixture.upstream.requests.lock().unwrap().last().unwrap().2["input"],
            unknown["input"]
        );
        fixture.assert_original_ownership();
        let count = fixture.upstream.requests.lock().unwrap().len();
        let rejected = fixture.child("caller-b", &unknown).await;
        assert_eq!(rejected.status(), 409);
        assert_eq!(
            rejected.json::<Value>().await.unwrap()["error"]["code"],
            "upstream_state_caller_mismatch"
        );
        fixture.host.force_b.store(true, Ordering::SeqCst);
        // A server-only item reference is not a portable child payload. Its
        // original response owner remains mandatory even with rebuilding on.
        let mut bound = fixture.parent_body(false);
        bound["previous_response_id"] = json!("parent-response");
        bound["input"] = json!([{"type":"item_reference","id":"communication-item"}]);
        let rejected = fixture.child("caller-a", &bound).await;
        assert_eq!(rejected.status(), 409);
        assert_eq!(
            rejected.json::<Value>().await.unwrap()["error"]["code"],
            "upstream_state_owner_changed"
        );
        assert_eq!(fixture.upstream.requests.lock().unwrap().len(), count);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn agent_communication_tool_done_is_persisted_before_http_sse_and_websocket_delivery() {
    for transport in [ParentTransport::Sse, ParentTransport::WebSocket] {
        for tool in ["spawn_agent", "send_message", "followup_task"] {
            let fixture = Fixture::new(tool, false).await;
            let child = fixture.child_body(&fixture.cipher);
            let mut http_source = None;
            let mut ws_source = None;
            match transport {
                ParentTransport::Sse => {
                    let parent = client()
                        .post(format!("{}/v1/responses", fixture.address()))
                        .bearer_auth("caller-a")
                        .json(&fixture.parent_body(true))
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(parent.status(), 200);
                    let mut source = parent.bytes_stream();
                    let mut received = String::new();
                    tokio::time::timeout(Duration::from_secs(3), async {
                        while !received.contains("response.output_item.done") {
                            let chunk = source.next().await.unwrap().unwrap();
                            received.push_str(std::str::from_utf8(&chunk).unwrap());
                        }
                    })
                    .await
                    .unwrap();
                    assert!(received.contains(&fixture.cipher));
                    http_source = Some(source);
                }
                ParentTransport::WebSocket => {
                    let mut request = format!(
                        "ws://{}/v1/responses",
                        fixture.handle.as_ref().unwrap().addr
                    )
                    .into_client_request()
                    .unwrap();
                    request
                        .headers_mut()
                        .insert("authorization", HeaderValue::from_static("Bearer caller-a"));
                    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
                    let mut parent = fixture.parent_body(true);
                    parent["type"] = json!("response.create");
                    socket
                        .send(ClientMessage::Text(parent.to_string().into()))
                        .await
                        .unwrap();
                    tokio::time::timeout(Duration::from_secs(3), async {
                        loop {
                            let message = socket.next().await.unwrap().unwrap();
                            if let ClientMessage::Text(text) = message {
                                let value: Value = serde_json::from_str(&text).unwrap();
                                if value["type"] == "response.output_item.done" {
                                    assert_eq!(value["item"], fixture.upstream.tool);
                                    break;
                                }
                            }
                        }
                    })
                    .await
                    .unwrap();
                    ws_source = Some(socket);
                }
            }
            // A separately opened SQLite reader observes the grant before the
            // parent's terminal frame is allowed to leave the fake upstream.
            fixture.assert_persisted(&child);
            let result = fixture.child("caller-a", &child).await;
            assert_eq!(result.status(), 200);
            result.bytes().await.unwrap();
            let sent = fixture
                .upstream
                .requests
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .clone();
            assert_eq!(sent.0, "a");
            assert_eq!(sent.2["input"], child["input"]);
            fixture.upstream.release.send_replace(true);
            if let Some(mut source) = http_source {
                while let Some(chunk) = source.next().await {
                    chunk.unwrap();
                }
            }
            if let Some(mut socket) = ws_source {
                tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        let message = socket.next().await.unwrap().unwrap();
                        if let ClientMessage::Text(text) = message {
                            let value: Value = serde_json::from_str(&text).unwrap();
                            if value["type"] == "response.completed" {
                                break;
                            }
                        }
                    }
                })
                .await
                .unwrap();
                socket.close(None).await.unwrap();
            }
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn agent_communication_explicit_plaintext_does_not_issue_cipher_ownership() {
    let fixture = Fixture::new("spawn_agent", true).await;
    let response = client()
        .post(format!("{}/v1/responses", fixture.address()))
        .bearer_auth("caller-a")
        .json(&fixture.parent_body(false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    let path = fixture.root.path().join("identity.sqlite");
    let body = fixture.child_body(&fixture.cipher);
    // Explicit plaintext output grants no ciphertext provenance by itself.
    assert_eq!(
        IdentityStore::open(path.clone())
            .unwrap()
            .inspect("caller-a", &HeaderMap::new(), &body)
            .unwrap_err()
            .code,
        "unknown_upstream_state"
    );
    let result = fixture.child("caller-a", &body).await;
    assert_eq!(result.status(), 200);
    result.bytes().await.unwrap();
    // The independently supplied complete input becomes accepted only after its own success.
    assert!(IdentityStore::open(path)
        .unwrap()
        .inspect("caller-a", &HeaderMap::new(), &body)
        .is_ok());
    let foreign = fixture.child("caller-b", &body).await;
    assert_eq!(foreign.status(), 409);
    assert_eq!(
        foreign.json::<Value>().await.unwrap()["error"]["code"],
        "upstream_state_caller_mismatch"
    );
    assert_eq!(fixture.upstream.requests.lock().unwrap().len(), 2);
    fixture.finish().await;
}

#[tokio::test]
async fn agent_communication_observed_cipher_is_usable_during_automatic_parent_history() {
    let fixture = Fixture::new("spawn_agent", false).await;
    let parent = client()
        .post(format!("{}/v1/responses", fixture.address()))
        .bearer_auth("caller-a")
        .json(&fixture.parent_body(true))
        .send()
        .await
        .unwrap();
    assert_eq!(parent.status(), 200);
    let mut source = parent.bytes_stream();
    let mut received = String::new();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !received.contains("response.output_item.done") {
            let chunk = source.next().await.unwrap().unwrap();
            received.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    })
    .await
    .unwrap();
    let child = fixture.child_body(&fixture.cipher);
    let result = fixture.child("caller-a", &child).await;
    assert_eq!(result.status(), 200);
    result.bytes().await.unwrap();
    assert_eq!(fixture.upstream.requests.lock().unwrap().len(), 2);
    fixture.upstream.release.send_replace(true);
    while let Some(chunk) = source.next().await {
        chunk.unwrap();
    }
    fixture.assert_persisted(&child);
    let result = fixture.child("caller-a", &child).await;
    assert_eq!(result.status(), 200);
    result.bytes().await.unwrap();
    assert!(fixture
        .upstream
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|(account, _, _)| account == "a"));
    fixture.finish().await;
}

#[tokio::test]
async fn agent_communication_failed_automatic_history_does_not_adopt_input_cipher() {
    for sse_error in [false, true] {
        let fixture = Fixture::new("spawn_agent", false).await;
        fixture
            .upstream
            .http_failure
            .store(!sse_error, Ordering::SeqCst);
        fixture
            .upstream
            .terminal_failure
            .store(sse_error, Ordering::SeqCst);
        let path = fixture.root.path().join("identity.sqlite");
        let mut original = fixture.parent_body(sse_error);
        original["input"] =
            json!([{"type":"reasoning","encrypted_content":"synthetic-legacy-cipher"}]);
        let response = client()
            .post(format!("{}/v1/responses", fixture.address()))
            .bearer_auth("caller-a")
            .json(&original)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if sse_error { 200 } else { 503 }
        );
        if sse_error {
            let mut source = response.bytes_stream();
            let mut observed = String::new();
            tokio::time::timeout(Duration::from_secs(3), async {
                while !observed.contains("response.output_item.done") {
                    let chunk = source.next().await.unwrap().unwrap();
                    observed.push_str(std::str::from_utf8(&chunk).unwrap());
                }
            })
            .await
            .unwrap();
            assert!(observed.contains(&fixture.cipher));
            let store = IdentityStore::open(path.clone()).unwrap();
            let unknown = json!({"model":"fixture-model","input":[{"type":"reasoning","encrypted_content":"synthetic-legacy-cipher"}]});
            assert!(store
                .inspect("caller-a", &HeaderMap::new(), &unknown)
                .is_err());
            drop(store);
            fixture.upstream.release.send_replace(true);
            while let Some(chunk) = source.next().await {
                observed.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
            }
            assert!(observed.contains("synthetic recovery failure"));
        } else {
            assert!(response
                .text()
                .await
                .unwrap()
                .contains("synthetic recovery failure"));
        }
        let seen = fixture.upstream.requests.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "a");
        assert_eq!(seen[0].2["input"], original["input"]);
        assert!(seen[0].2.get("previous_response_id").is_none());
        drop(seen);
        let selections = fixture.host.selections.lock().unwrap();
        assert_eq!(selections.len(), if sse_error { 1 } else { 2 });
        for (_, selection) in selections.iter() {
            assert_eq!(selection.thread_id.as_deref(), Some("parent"));
            assert_eq!(selection.preferred_account_id.as_deref(), None);
            assert!(!selection.force_refresh);
        }
        assert!(selections[0].1.excluded_account_ids.is_empty());
        if !sse_error {
            // A retry selection is allowed before any output, but this fixture
            // deliberately returns the already excluded account, so it is never sent.
            assert_eq!(selections[1].1.excluded_account_ids, vec!["a"]);
        }
        drop(selections);
        let store = IdentityStore::open(path).unwrap();
        assert_eq!(
            store
                .inspect(
                    "caller-a",
                    &HeaderMap::new(),
                    &json!({"model":"fixture-model","input":[{"type":"reasoning","encrypted_content":"synthetic-legacy-cipher"}]})
                )
                .unwrap_err()
                .code,
            "unknown_upstream_state"
        );
        let clean = fixture.parent_body(false);
        assert_eq!(
            store
                .inspect("caller-a", &HeaderMap::new(), &clean)
                .unwrap()
                .thread_id
                .as_deref(),
            Some("parent")
        );
        drop(store);
        // Failed complete input remains retryable; it does not consume a hidden grant
        // or acquire ownership merely because an upstream attempt was made.
        let retry = client()
            .post(format!("{}/v1/responses", fixture.address()))
            .bearer_auth("caller-a")
            .json(&original)
            .send()
            .await
            .unwrap();
        assert_eq!(retry.status().as_u16(), if sse_error { 200 } else { 503 });
        assert!(retry
            .text()
            .await
            .unwrap()
            .contains("synthetic recovery failure"));
        assert_eq!(fixture.upstream.requests.lock().unwrap().len(), 2);
        assert!(IdentityStore::open(fixture.root.path().join("identity.sqlite")).unwrap()
            .inspect("caller-a", &HeaderMap::new(), &json!({"input":[{"type":"reasoning","encrypted_content":"synthetic-legacy-cipher"}]})).is_err());
        fixture.finish().await;
    }
}
