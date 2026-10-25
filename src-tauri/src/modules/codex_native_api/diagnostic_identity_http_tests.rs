use super::{
    contract::*,
    diagnostics::{capture_header_identity, DiagnosticContext},
    start, Handle, Options,
};
use axum::{
    body::to_bytes,
    extract::{
        ws::{Message as AxumMessage, WebSocketUpgrade},
        Request, State,
    },
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[derive(Clone)]
struct WireRecord {
    path: String,
    headers: HeaderMap,
    body_thread: Option<String>,
    body: Value,
}
struct IdentitySource {
    seen: Mutex<Vec<WireRecord>>,
    connections: AtomicUsize,
    fail_first: bool,
}
async fn identity_http(State(source): State<Arc<IdentitySource>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let account = parts
        .headers
        .get("x-test-account")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let reject = source.fail_first && account == "a";
    let id = {
        let mut seen = source.seen.lock().unwrap();
        let id = seen.len();
        seen.push(WireRecord {
            path: parts.uri.path().into(),
            headers: parts.headers.clone(),
            body_thread: value
                .pointer("/client_metadata/thread_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            body: value.clone(),
        });
        id
    };
    if reject {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"code":"unavailable","message":"fixture unavailable"}})),
        )
            .into_response();
    }
    if parts.uri.path().contains("images/") {
        return Json(json!({"created":1,"data":[{"b64_json":"AA=="}]})).into_response();
    }
    if parts.uri.path().contains("chat/completions") {
        return Json(json!({"id":format!("chat-{id}"),"object":"chat.completion","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}})).into_response();
    }
    if parts.uri.path().contains("alpha/search") {
        return Json(json!({"results":[],"output":"ok"})).into_response();
    }
    if parts.uri.path().contains("audio/") || parts.uri.path().ends_with("/transcribe") {
        return Json(json!({"text":"ok"})).into_response();
    }
    if value["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|tool| tool["type"] == "image_generation"))
    {
        return Json(json!({"id":format!("image-{id}"),"status":"completed","output":[{"type":"image_generation_call","result":"AA=="}],"usage":{"total_tokens":2}})).into_response();
    }
    Json(json!({"id":format!("response-{id}"),"status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":1}})).into_response()
}
async fn identity_ws(
    State(source): State<Arc<IdentitySource>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    source.connections.fetch_add(1, Ordering::SeqCst);
    ws.on_upgrade(move |mut socket| async move {
        while let Some(Ok(message))=socket.recv().await {
            if let AxumMessage::Text(text)=message {
                let body:Value=serde_json::from_str(&text).unwrap();
                let id={
                    let mut seen=source.seen.lock().unwrap();
                    let id=seen.len();
                    seen.push(WireRecord{path:"/v1/responses".into(),headers:headers.clone(),body_thread:body.pointer("/client_metadata/thread_id").and_then(Value::as_str).map(str::to_owned),body:body.clone()});
                    id
                };
                let event=json!({"type":"response.completed","response":{"id":format!("ws-{id}"),"status":"completed","output":[]}});
                if socket.send(AxumMessage::Text(event.to_string().into())).await.is_err(){break;}
            } else if matches!(message,AxumMessage::Close(_)) {break;}
        }
    })
}

#[path = "account_installation_auxiliary_tests.rs"]
mod installation_auxiliary_tests;
#[path = "diagnostic_identity_mapping_tests.rs"]
mod session_identity_mapping_tests;
struct IdentityHost {
    base: String,
    wire: WireApi,
    native_socket: bool,
    retry: bool,
    oauth: AtomicBool,
    diagnostics: Mutex<Vec<DiagnosticContext>>,
}
impl Host for IdentityHost {
    fn authorize<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async {
            Ok(Caller {
                id: "identity-caller".into(),
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
            let account = request.preferred_account_id.as_deref().unwrap_or(
                if request
                    .excluded_account_ids
                    .iter()
                    .any(|account| account == "a")
                {
                    "b"
                } else {
                    "a"
                },
            );
            let mut headers = HeaderMap::new();
            headers.insert("x-test-account", account.parse().unwrap());
            Ok(Route {
                continuity_account_id: None,
                installation_profile: Some(test_profile(account)),
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
                headers,
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: self.oauth.load(Ordering::SeqCst),
                chatgpt_plan: false,
                wire_api: self.wire,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }
    fn image_route<'a>(
        &'a self,
        caller: &'a Caller,
        request: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            let route = self
                .route(
                    caller,
                    RouteRequest {
                        model: request.model.unwrap_or_else(|| "image".into()),
                        session_id: None,
                        thread_id: None,
                        parent_thread_id: None,
                        preferred_account_id: request.preferred_account_id,
                        soft_preferred_account_id: None,
                        target_account_id: request.target_account_id,
                        force_refresh: request.force_refresh,
                        image_generation_requested: true,
                        image_input: false,
                        excluded_account_ids: request.excluded_account_ids,
                        requires_permit: true,
                        required_capabilities: Vec::new(),
                        capability_fallback_from: None,
                        auxiliary: Some("images.generations".into()),
                    },
                )
                .await?;
            Ok(ImageRoute {
                route,
                image_model: "image".into(),
                main_model: self.oauth.load(Ordering::SeqCst).then(|| "m".into()),
            })
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: if self.retry { 2 } else { 1 },
                ..Default::default()
            })
        })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(self.native_socket) })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }
    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn diagnostic(&self, context: DiagnosticContext) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.diagnostics.lock().unwrap().push(context);
        })
    }
}
async fn identity_fixture(
    wire: WireApi,
    native_socket: bool,
    retry: bool,
) -> (
    Handle,
    Arc<IdentityHost>,
    Arc<IdentitySource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    identity_fixture_with_options(wire, native_socket, retry, Options::default()).await
}
async fn identity_fixture_with_options(
    wire: WireApi,
    native_socket: bool,
    retry: bool,
    options: Options,
) -> (
    Handle,
    Arc<IdentityHost>,
    Arc<IdentitySource>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let source = Arc::new(IdentitySource {
        seen: Mutex::new(vec![]),
        connections: AtomicUsize::new(0),
        fail_first: retry,
    });
    let app = Router::new()
        .route("/v1/responses", post(identity_http).get(identity_ws))
        .route("/v1/responses/compact", post(identity_http))
        .route("/v1/chat/completions", post(identity_http))
        .route("/v1/alpha/search", post(identity_http))
        .route("/v1/audio/transcriptions", post(identity_http))
        .route("/v1/transcribe", post(identity_http))
        .route("/v1/images/generations", post(identity_http))
        .route("/v1/images/edits", post(identity_http))
        .with_state(source.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(IdentityHost {
        base,
        wire,
        native_socket,
        retry,
        oauth: AtomicBool::new(false),
        diagnostics: Mutex::new(vec![]),
    });
    let dir = tempfile::tempdir().unwrap();
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        dir.path().join("identity.sqlite"),
        options,
    )
    .await
    .unwrap();
    (handle, host, source, task, dir)
}
fn test_profile(account: &str) -> crate::modules::codex_api_identity::InstallationProfile {
    crate::modules::codex_api_identity::InstallationProfile {
        version: 1,
        scope: account.repeat(64),
        installation_id: if account == "a" {
            "d5a2e458-7e4a-456a-bf68-f690dcf3c256"
        } else {
            "32902d87-7d4d-454b-9a9a-d00e14db6c71"
        }
        .into(),
        revision: 1,
    }
}
async fn captured(host: &IdentityHost, count: usize) -> Vec<DiagnosticContext> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let values = host.diagnostics.lock().unwrap().clone();
            if values.len() >= count {
                return values;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn canonical_routing_feedback_does_not_replace_original_header_diagnostics() {
    let (mut handle, host, _, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    let response = http_client().post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key").header("session-id", "header-session").header("thread-id", "header-thread")
        .json(&json!({"model":"m","input":[],"client_metadata":{"x-codex-turn-metadata":json!({"session_id":"body-session","thread_id":"body-child","parent_thread_id":"body-parent"}).to_string()}}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let contexts = captured(&host, 1).await;
    let context = contexts.last().unwrap();
    assert_eq!(context.client_session_id.as_deref(), Some("header-session"));
    assert_eq!(context.client_thread_id.as_deref(), Some("header-thread"));
    let routing = context.routing_identity.as_ref().unwrap();
    assert_eq!(routing.session_id.as_deref(), Some("body-session"));
    assert_eq!(routing.thread_id.as_deref(), Some("body-child"));
    assert_eq!(routing.parent_thread_id.as_deref(), Some("body-parent"));
    let public = serde_json::to_value(context).unwrap();
    assert!(public.get("routingIdentity").is_none());
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
fn assert_sent(
    context: &DiagnosticContext,
    record: &WireRecord,
    client_session: Option<&str>,
    client_thread: Option<&str>,
) {
    assert!(context.client_identity_captured);
    assert_eq!(context.client_session_id.as_deref(), client_session);
    assert_eq!(context.client_thread_id.as_deref(), client_thread);
    assert!(
        context.payloads.is_empty(),
        "identity collection must not enable payload logging"
    );
    let attempt = context.attempts.last().unwrap();
    let expected = capture_header_identity(&record.headers);
    assert!(attempt.upstream_identity_captured);
    assert_eq!(attempt.upstream_session_id, expected.session);
    assert_eq!(attempt.upstream_thread_id, expected.thread);
    assert!(context.client_identity_metadata_captured);
    assert!(attempt.upstream_identity_metadata_captured);
    assert_eq!(attempt.upstream_installation_id, expected.installation);
    assert_eq!(attempt.upstream_chatgpt_account_id, expected.account);
    assert_eq!(
        attempt.account_identity,
        Some(test_profile(&attempt.account_id))
    );
}
fn http_client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn http_identity_metadata_matches_wire_with_payload_logging_disabled_and_no_body_backfill() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, false).await;
    for (index, path) in ["responses", "responses/compact"].iter().enumerate() {
        let response = http_client()
            .post(format!("http://{}/v1/{path}", handle.addr))
            .bearer_auth("key")
            .header("session-id", "raw-session")
            .header("session_id", "alias-must-not-win")
            .header("thread-id", "raw-thread")
            .header("x-codex-installation-id", "client-installation")
            .header("chatgpt-account-id", "client-workspace")
            .json(&json!({"model":"m","input":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let _: Value = response.json().await.unwrap();
        let values = captured(&host, index + 1).await;
        assert_eq!(
            values[index].client_installation_id.as_deref(),
            Some("client-installation")
        );
        assert_eq!(
            values[index].client_chatgpt_account_id.as_deref(),
            Some("client-workspace")
        );
        let seen = source.seen.lock().unwrap();
        assert_eq!(seen[index].path, format!("/v1/{path}"));
        assert_sent(
            &values[index],
            &seen[index],
            Some("raw-session"),
            Some("raw-thread"),
        );
        assert_ne!(
            values[index]
                .attempts
                .last()
                .unwrap()
                .upstream_session_id
                .as_deref(),
            Some("raw-session")
        );
    }
    let response=http_client().post(format!("http://{}/v1/responses",handle.addr)).bearer_auth("key")
        .json(&json!({"model":"m","input":[],"client_metadata":{"session_id":"body-session","thread_id":"body-thread"}})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let values = captured(&host, 3).await;
    let seen = source.seen.lock().unwrap();
    assert_sent(&values[2], &seen[2], None, None);
    assert!(values[2].attempts[0].upstream_session_id.is_some());
    assert!(values[2].attempts[0].upstream_thread_id.is_some());
    drop(seen);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn retry_identity_metadata_keeps_each_account_attempt_separate() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, false, true).await;
    let response = http_client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .header("session_id", "alias-session")
        .header("thread-id", "raw-thread")
        .json(&json!({"model":"m","input":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let _: Value = response.json().await.unwrap();
    let values = captured(&host, 1).await;
    let context = &values[0];
    let seen = source.seen.lock().unwrap();
    assert_eq!(context.client_session_id.as_deref(), Some("alias-session"));
    assert_eq!(seen.len(), 2);
    assert_eq!(context.attempts.len(), 2);
    for (attempt, record) in context.attempts.iter().zip(seen.iter()) {
        let expected = capture_header_identity(&record.headers);
        assert!(attempt.upstream_identity_metadata_captured);
        assert_eq!(attempt.upstream_installation_id, expected.installation);
        assert_eq!(
            attempt.account_identity,
            Some(test_profile(&attempt.account_id))
        );
        assert!(attempt.upstream_identity_captured);
        assert_eq!(attempt.upstream_session_id, expected.session);
        assert_eq!(attempt.upstream_thread_id, expected.thread);
        assert_eq!(attempt.account_id, record.headers["x-test-account"]);
    }
    assert_ne!(
        context.attempts[0].upstream_session_id,
        context.attempts[1].upstream_session_id
    );
    assert_ne!(
        context.attempts[0].upstream_thread_id,
        context.attempts[1].upstream_thread_id
    );
    drop(seen);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn native_chat_auxiliary_and_multipart_images_capture_the_final_send_headers() {
    for wire in [WireApi::Responses, WireApi::Chat] {
        let (mut handle, host, source, task, _dir) = identity_fixture(wire, false, false).await;
        let mut count = 0;
        if wire == WireApi::Chat {
            let response=http_client().post(format!("http://{}/v1/chat/completions",handle.addr)).bearer_auth("key").header("session_id","alias-session").header("thread-id","raw-thread")
                .json(&json!({"model":"m","messages":[{"role":"user","content":"hello"}],"stop":["END"]})).send().await.unwrap();
            assert_eq!(response.status(), 200);
            let _: Value = response.json().await.unwrap();
            count += 1;
        } else {
            let response = http_client()
                .post(format!("http://{}/v1/alpha/search", handle.addr))
                .bearer_auth("key")
                .header("session_id", "alias-session")
                .header("thread-id", "raw-thread")
                .json(&json!({"model":"m","id":"search-session","input":"hello"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let _: Value = response.json().await.unwrap();
            count += 1;
            let form = reqwest::multipart::Form::new().text("model", "m").part(
                "file",
                reqwest::multipart::Part::bytes(vec![0, 1, 2]).file_name("audio.wav"),
            );
            let response = http_client()
                .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
                .bearer_auth("key")
                .header("session_id", "alias-session")
                .header("thread-id", "raw-thread")
                .multipart(form)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let _: Value = response.json().await.unwrap();
            count += 1;
            let form = reqwest::multipart::Form::new()
                .text("model", "image")
                .text("prompt", "draw")
                .part(
                    "image",
                    reqwest::multipart::Part::bytes(vec![0, 1, 2])
                        .file_name("image.png")
                        .mime_str("image/png")
                        .unwrap(),
                );
            let response = http_client()
                .post(format!("http://{}/v1/images/edits", handle.addr))
                .bearer_auth("key")
                .header("session_id", "alias-session")
                .header("thread-id", "raw-thread")
                .multipart(form)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let _: Value = response.json().await.unwrap();
            count += 1;
        }
        let values = captured(&host, count).await;
        let seen = source.seen.lock().unwrap();
        assert_eq!(seen.len(), count);
        for (context, record) in values.iter().zip(seen.iter()) {
            assert_sent(context, record, Some("alias-session"), Some("raw-thread"));
        }
        drop(seen);
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

async fn downstream_socket(
    handle: &Handle,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn complete_socket(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let event: Value = serde_json::from_str(&text).unwrap();
            assert_ne!(event["type"], "error");
            if event["type"] == "response.completed" {
                return;
            }
        }
        panic!("socket ended without completion");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn websocket_changed_identifiers_reconnect_and_capture_actual_handshake_ids() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, true, false).await;
    let mut socket = downstream_socket(&handle).await;
    for index in 0..2 {
        socket.send(Message::Text(json!({"type":"response.create","model":"m","previous_response_id":null,"input":[],"client_metadata":{"session_id":format!("body-session-{index}"),"thread_id":format!("body-thread-{index}")}}).to_string().into())).await.unwrap();
        complete_socket(&mut socket).await;
    }
    let values = captured(&host, 2).await;
    let seen = source.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(source.connections.load(Ordering::SeqCst), 2);
    for (context, record) in values.iter().zip(seen.iter()) {
        assert_sent(context, record, None, None);
        assert_eq!(context.attempts[0].transport, "websocket");
    }
    // The downstream socket retains its initial session header even when body
    // metadata changes. A new thread still requires a fresh upstream handshake.
    assert_eq!(
        values[0].attempts[0].upstream_session_id,
        values[1].attempts[0].upstream_session_id
    );
    assert_ne!(
        values[0].attempts[0].upstream_thread_id,
        values[1].attempts[0].upstream_thread_id
    );
    assert_eq!(
        seen[1].body_thread, values[1].attempts[0].upstream_thread_id,
        "the new connection must use the new mapped thread"
    );
    drop(seen);
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_stable_identifiers_reuse_and_record_actual_handshake_headers() {
    let (mut handle, host, source, task, _dir) =
        identity_fixture(WireApi::Responses, true, false).await;
    let mut socket = downstream_socket(&handle).await;
    for _ in 0..2 {
        socket
            .send(Message::Text(
                json!({"type":"response.create","model":"m","input":[],
            "client_metadata":{"session_id":"session","thread_id":"thread"}})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        complete_socket(&mut socket).await;
    }
    let values = captured(&host, 2).await;
    assert_eq!(source.connections.load(Ordering::SeqCst), 1);
    let seen = source.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (context, record) in values.iter().zip(seen.iter()) {
        assert_sent(context, record, None, None);
        assert_eq!(context.attempts.last().unwrap().transport, "websocket");
    }
    drop(seen);
    assert_eq!(
        values[0].attempts[0].upstream_session_id,
        values[1].attempts[0].upstream_session_id
    );
    assert_eq!(
        values[0].attempts[0].upstream_thread_id,
        values[1].attempts[0].upstream_thread_id
    );
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn websocket_http_chat_bridge_and_local_prewarm_capture_only_real_sends() {
    for wire in [WireApi::Responses, WireApi::Chat] {
        let (mut handle, host, source, task, _dir) = identity_fixture(wire, false, false).await;
        let mut socket = downstream_socket(&handle).await;
        socket
            .send(Message::Text(
                json!({"type":"response.create","model":"m","generate":false,"input":[]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        complete_socket(&mut socket).await;
        assert!(host.diagnostics.lock().unwrap().is_empty(),
            "local prewarm does not represent an upstream attempt");
        assert!(source.seen.lock().unwrap().is_empty());
        socket
            .send(Message::Text(
                json!({"type":"response.create","model":"m","input":[]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        complete_socket(&mut socket).await;
        let values = captured(&host, 1).await;
        let seen = source.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_sent(&values[0], &seen[0], None, None);
        assert_eq!(values[0].attempts.last().unwrap().transport, "http");
        drop(seen);
        socket.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        assert_eq!(host.diagnostics.lock().unwrap().len(), 1,
            "only the real send may leave a diagnostic after all work settles");
        task.abort();
    }
}
