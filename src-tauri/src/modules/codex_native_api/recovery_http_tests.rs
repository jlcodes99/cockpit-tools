//! Real loopback transports accept complete legacy history without manual controls.
use super::super::{contract::*, identity::IdentityStore, Options};
use axum::{
    body::{Body, Bytes},
    extract::State,
    http::HeaderMap,
    response::Response,
    routing::post,
    Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[path = "multi_result_history_tests.rs"]
mod multi_result_history_tests;

struct HostFixture {
    url: String,
    chatgpt: bool,
    selections: Mutex<Vec<RouteRequest>>,
}
impl Host for HostFixture {
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn authorize<'a>(&'a self, credential: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: if credential == "foreign" {
                    "other-caller"
                } else {
                    "caller"
                }
                .into(),
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
                account_id: "account".into(),
                owner_id: "owner".into(),
                base_url: self.url.clone(),
                model: request.model,
                headers: HeaderMap::new(),
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.chatgpt,
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
        Box::pin(async { Ok(json!({})) })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                send_retries: 0,
                status_retries: 0,
                credential_retries: 1,
                bootstrap_retries: 0,
                stream_open_max_attempts: 1,
                account_switch_rebuild: true,
                ..Default::default()
            })
        })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
struct Upstream {
    path: PathBuf,
    mode: u16,
    seen: Mutex<Vec<(HeaderMap, Value)>>,
}
fn input() -> Value {
    json!([
        {"role":"user","content":"first turn"},
        {"type":"reasoning","id":"old-item","encrypted_content":"legacy-cipher"},
        {"type":"compaction","encrypted_content":"legacy-summary"},
        {"type":"function_call","namespace":"collaboration","name":"send_message","call_id":"old-call","arguments":"{\"message\":\"legacy-agent-cipher\"}","encrypted_function_args":["message"]},
        {"type":"function_call_output","call_id":"old-call","output":"ok"},
        {"role":"user","content":"continue"}
    ])
}
fn body(streaming: bool) -> Value {
    json!({"model":"test","stream":streaming,"client_metadata":{"session_id":"client-root","thread_id":"client-child"},"input":input()})
}
fn input_owners(path: &Path) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(b"legacy-cipher"));
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted' AND digest=?1",
            [digest],
            |row| row.get(0),
        )
        .unwrap()
}
async fn upstream(
    State(source): State<Arc<Upstream>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        input_owners(&source.path),
        0,
        "unverified input must not acquire ownership before completion"
    );
    let previous = request.get("previous_response_id").is_some();
    source.seen.lock().unwrap().push((headers, request));
    if previous {
        return Response::builder().status(400).body(Body::from(
            r#"{"error":{"code":"previous_response_not_found","message":"fixture upstream checkpoint unavailable","param":"previous_response_id"}}"#,
        )).unwrap();
    }
    if source.mode == 400 {
        return Response::builder()
            .status(400)
            .body(Body::from(
                r#"{"error":{"code":"invalid_request_error","message":"fixture failure","param":"input"}}"#,
            ))
            .unwrap();
    }
    let output = json!([{"type":"reasoning","encrypted_content":"new-cipher"}]);
    let event = if source.mode == 299 {
        json!({"type":"response.failed","response":{"id":"auto-response","status":"failed","error":{"code":"invalid_request_error","message":"fixture failure"},"output":[]}})
    } else {
        json!({"type":"response.completed","response":{"id":"auto-response","status":"completed","output":output}})
    };
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(format!("data: {event}\n\n")))
        .unwrap()
}
async fn fixture(
    mode: u16,
) -> (
    tempfile::TempDir,
    super::super::Handle,
    Arc<Upstream>,
    tokio::task::JoinHandle<()>,
) {
    fixture_with_route(mode, false).await
}

async fn fixture_with_route(
    mode: u16,
    chatgpt: bool,
) -> (
    tempfile::TempDir,
    super::super::Handle,
    Arc<Upstream>,
    tokio::task::JoinHandle<()>,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("identity.sqlite");
    let source = Arc::new(Upstream {
        path: path.clone(),
        mode,
        seen: Mutex::new(vec![]),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(HostFixture {
        url: format!("http://{}/v1", listener.local_addr().unwrap()),
        chatgpt,
        selections: Mutex::new(vec![]),
    });
    let app = Router::new()
        .route("/v1/responses", post(upstream))
        .route("/v1/responses/compact", post(compact_upstream))
        .with_state(source.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let service = super::super::start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        path,
        Options::default(),
    )
    .await
    .unwrap();
    (directory, service, source, task)
}

#[tokio::test]
async fn legacy_forwarding_http_and_sse_let_upstream_judge_incomplete_or_unknown_history() {
    for streaming in [false, true] {
        for mode in [200, 400, 299] {
            for item in [
                json!({"type":"function_call_output","call_id":"orphan","output":"original result"}),
                json!({"type":"function_call","name":"legacy_tool","arguments":"{}"}),
                json!({"type":"future_history_item","payload":{"keep":"exact"}}),
            ] {
                let (_directory, mut service, source, task) = fixture(mode).await;
                let mut original = body(streaming);
                original["input"] = json!([
                    {"type":"reasoning","encrypted_content":"legacy-cipher","summary":[]},
                    item
                ]);
                original["tools"] = json!([{"type":"future_tool","extension":"keep"}]);
                let response = reqwest::Client::builder().no_proxy().build().unwrap()
                    .post(format!("http://{}/v1/responses", service.addr))
                    .bearer_auth("test")
                    .json(&original)
                    .send().await.unwrap();
                let status = response.status();
                let payload = response.text().await.unwrap();
                assert!(!payload.contains("continuation_history_incomplete"), "{payload}");
                if mode == 200 {
                    assert!(status.is_success(), "{payload}");
                    assert!(payload.contains("auto-response"), "{payload}");
                } else {
                    assert!(payload.contains("fixture failure"), "{payload}");
                    if mode == 400 {
                        assert!(payload.contains("invalid_request_error"), "{payload}");
                        assert!(payload.contains("\"param\":\"input\""), "{payload}");
                    }
                    assert_eq!(input_owners(&source.path), 0);
                }
                let seen = source.seen.lock().unwrap();
                assert_eq!(seen.len(), 1, "ordinary forwarding must not silently retry");
                assert_eq!(seen[0].1["input"], original["input"]);
                assert_eq!(seen[0].1["tools"], original["tools"]);
                drop(seen);
                service.shutdown(Duration::from_secs(2)).await.unwrap();
                task.abort();
            }
        }
    }
}

#[tokio::test]
async fn automatic_history_http_and_sse_preserve_unknown_cipher_and_commit_only_complete_success() {
    for streaming in [false, true] {
        for mode in [200, 400, 299] {
            let (directory, mut service, source, task) = fixture(mode).await;
            let original = body(streaming);
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let response = client
                .post(format!("http://{}/v1/responses", service.addr))
                .bearer_auth("test")
                .json(&original)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let payload = response.text().await.unwrap();
            if mode == 200 {
                assert!(status.is_success(), "{payload}");
                assert!(payload.contains("auto-response"), "{payload}");
            } else {
                assert!(payload.contains("fixture failure"), "{payload}");
            }
            let seen = source.seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].1["input"], original["input"]);
            assert_ne!(seen[0].0["thread-id"], "client-child");
            drop(seen);
            assert_eq!(input_owners(&source.path), if mode == 200 { 1 } else { 0 });
            assert!(!source.path.with_extension("recovery").exists());
            if mode == 200 {
                let foreign = client
                    .post(format!("http://{}/v1/responses", service.addr))
                    .bearer_auth("foreign")
                    .json(&original)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(foreign.status(), 409);
                assert_eq!(source.seen.lock().unwrap().len(), 1);
                let store = IdentityStore::open(source.path.clone()).unwrap();
                assert!(store
                    .inspect_for_rebuild("caller", &HeaderMap::new(), &original)
                    .is_ok());
            }
            service.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
            drop(directory);
        }
    }
}

#[tokio::test]
async fn automatic_history_missing_checkpoint_fails_before_upstream_and_full_resend_succeeds() {
    let (_directory, mut service, source, task) = fixture_with_route(200, true).await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/responses", service.addr);
    let delta = json!({"model":"test","previous_response_id":"unknown-response","client_metadata":{"session_id":"client-root","thread_id":"client-child"},"input":[{"role":"user","content":"next"}]});
    let response = client
        .post(&url)
        .bearer_auth("test")
        .json(&delta)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("continuation_history_incomplete"));
    assert!(source.seen.lock().unwrap().is_empty());
    let mut full = body(false);
    full["input"][1] = json!({
        "type":"reasoning",
        "summary":[],
        "encrypted_content":super::super::codex_cpa_wire::synthetic_gpt_signature(71)
    });
    let response = client
        .post(&url)
        .bearer_auth("test")
        .json(&full)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    assert_eq!(source.seen.lock().unwrap()[0].1["input"], full["input"]);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn legacy_forwarding_websocket_http_bridge_preserves_unverified_history_and_upstream_error() {
    for mode in [200, 400] {
        let (_directory, mut service, source, task) = fixture(mode).await;
        let mut request = format!("ws://{}/v1/responses", service.addr)
            .into_client_request().unwrap();
        request.headers_mut().insert("authorization", "Bearer test".parse().unwrap());
        let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
        let mut original = body(true);
        original["type"] = json!("response.create");
        original["input"] = json!([
            {"type":"reasoning","encrypted_content":"legacy-cipher","summary":[]},
            {"type":"function_call_output","call_id":"orphan","output":"original result"},
            {"type":"future_history_item","payload":"keep"}
        ]);
        socket.send(Message::Text(original.to_string().into())).await.unwrap();
        let event = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match socket.next().await.unwrap().unwrap() {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text).unwrap();
                        if value["type"] == "response.completed" || value["type"] == "error" {
                            break value;
                        }
                    }
                    Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                    _ => {}
                }
            }
        }).await.unwrap();
        if mode == 200 {
            assert_eq!(event["type"], "response.completed", "{event}");
        } else {
            assert_eq!(event["error"]["code"], "invalid_request_error", "{event}");
            assert_eq!(event["error"]["message"], "fixture failure", "{event}");
            assert_eq!(event["error"]["param"], "input", "{event}");
            assert_eq!(input_owners(&source.path), 0);
        }
        let seen = source.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].1["input"], original["input"]);
        drop(seen);
        socket.close(None).await.unwrap();
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn automatic_history_websocket_http_bridge_accepts_full_history_without_a_control_file() {
    let (_directory, mut service, source, task) = fixture(200).await;
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer test".parse().unwrap());
    let mut socket = tokio_tungstenite::connect_async(request).await.unwrap().0;
    let mut original = body(true);
    original["type"] = json!("response.create");
    socket
        .send(Message::Text(original.to_string().into()))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    if value["type"] == "response.completed" || value["type"] == "error" {
                        break value;
                    }
                }
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(event["type"], "response.completed", "{event}");
    assert_eq!(source.seen.lock().unwrap()[0].1["input"], original["input"]);
    assert_eq!(input_owners(&source.path), 1);
    assert!(!source.path.with_extension("recovery").exists());
    socket.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

async fn compact_upstream(
    State(source): State<Arc<Upstream>>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    source.seen.lock().unwrap().push((headers, body));
    Response::builder().header("content-type","application/json").body(Body::from(json!({"id":"compact-result","output":[{"type":"compaction","encrypted_content":"compact-output-cipher"}]}).to_string())).unwrap()
}

#[tokio::test]
async fn automatic_history_compact_accepts_complete_unknown_cipher_without_a_generation_commit() {
    let (_directory, mut service, source, task) = fixture(200).await;
    let original = body(false);
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses/compact", service.addr))
        .bearer_auth("test")
        .json(&original)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["output"][0]["encrypted_content"],
        "compact-output-cipher"
    );
    assert_eq!(source.seen.lock().unwrap()[0].1["input"], original["input"]);
    assert_eq!(input_owners(&source.path), 0);
    let connection = rusqlite::Connection::open(&source.path).unwrap();
    for table in ["native_rebuild_active_v1", "native_rebuild_checkpoints_v1"] {
        let rows: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 0);
    }
    drop(connection);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
