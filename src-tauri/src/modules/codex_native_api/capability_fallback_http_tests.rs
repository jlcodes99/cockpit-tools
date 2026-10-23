//! Capability routing through real loopback HTTP/SSE and downstream WebSocket.
use super::{contract::*, start, Handle, Options};
use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
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

#[derive(Clone, Copy)]
enum Rejection {
    None,
    Unsupported,
    Invalid,
    UnsupportedSse,
    UnsupportedHandshakeSse,
    InvalidSse,
    AfterOutput,
    Incomplete,
}
struct FallbackHost {
    base: String,
    enabled: bool,
    preflight: bool,
    backup: bool,
    incompatible_first: AtomicBool,
    all_incompatible: AtomicBool,
    permit_releases: Arc<AtomicUsize>,
    seen: Mutex<Vec<(String, RouteRequest)>>,
    reports: Mutex<Vec<RequestReport>>,
}
struct CandidatePermit(Arc<AtomicUsize>);
impl Drop for CandidatePermit {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Host for FallbackHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            Ok(Caller {
                id: bearer.into(),
                label: "test".into(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((caller.id.clone(), request.clone()));
            let replacement = request.capability_fallback_from.is_some();
            if replacement
                && (!self.backup
                    || caller.id == "restricted"
                    || request.excluded_account_ids.iter().any(|id| id == "backup"))
            {
                return Err(ServiceError::new(
                    503,
                    "capability_fallback_unavailable",
                    "No authorized capable backup",
                ));
            }
            let id = if replacement {
                if self.incompatible_first.load(Ordering::SeqCst)
                    && !request
                        .excluded_account_ids
                        .iter()
                        .any(|id| id == "incompatible")
                {
                    "incompatible"
                } else {
                    "backup"
                }
            } else {
                "primary"
            };
            if id == "backup" && self.incompatible_first.load(Ordering::SeqCst) {
                assert!(
                    self.permit_releases.load(Ordering::SeqCst) > 0,
                    "Rejected candidate must release capacity before the next selection"
                );
            }
            let mut headers = HeaderMap::new();
            headers.insert("x-test-account", id.parse().unwrap());
            Ok(Route {
                account_id: id.into(),
                owner_id: format!("owner-{id}"),
                continuity_account_id: None,
                installation_profile: None,
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                base_url: self.base.clone(),
                model: request.model,
                headers,
                client: reqwest::Client::builder().no_proxy().build().unwrap(),
                proxy_url: None,
                proxy_route: None,
                chatgpt: false,
                chatgpt_plan: self.preflight && !replacement,
                wire_api: if id == "incompatible"
                    || (replacement && self.all_incompatible.load(Ordering::SeqCst))
                {
                    WireApi::Chat
                } else {
                    WireApi::Responses
                },
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: (id == "incompatible").then(|| {
                    Arc::new(CandidatePermit(self.permit_releases.clone())) as Arc<dyn Send + Sync>
                }),
            })
        })
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        _: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        Box::pin(async move {
            Ok(CapabilityFallbackDecision {
                enabled: self.enabled,
                unsupported: if self.preflight && route.account_id == "primary" {
                    request
                        .required_capabilities
                        .iter()
                        .filter(|id| id.as_str() == "code_interpreter")
                        .cloned()
                        .collect()
                } else {
                    Vec::new()
                },
            })
        })
    }
    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"same-model"}]})) })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}
type Captures = Arc<Mutex<Vec<(String, Value)>>>;
async fn fixture(
    enabled: bool,
    preflight: bool,
    backup: bool,
    rejection: Rejection,
) -> (
    Arc<FallbackHost>,
    Captures,
    Handle,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    let captures: Captures = Default::default();
    let copy = captures.clone();
    let app = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let copy = copy.clone();
        async move {
            let account = headers["x-test-account"].to_str().unwrap().to_owned();
            copy.lock().unwrap().push((account.clone(), body));
            if account == "primary" && matches!(rejection, Rejection::Unsupported | Rejection::Invalid) {
                let code = if matches!(rejection, Rejection::Unsupported) { "unsupported_tool" } else { "missing_required_parameter" };
                return Response::builder().status(StatusCode::BAD_REQUEST).header("content-type", "application/json")
                    .body(Body::from(json!({"error":{"code":code,"param":"tools[0].type","message":"Synthetic rejection"}}).to_string())).unwrap();
            }
            let events = if account == "primary" && matches!(rejection, Rejection::UnsupportedSse | Rejection::UnsupportedHandshakeSse | Rejection::InvalidSse) {
                let code = if matches!(rejection, Rejection::InvalidSse) { "missing_required_parameter" } else { "unsupported_tool" };
                let error = json!({"type":"error","error":{"type":"invalid_request_error","code":code,"param":"tools[0].type","message":"Synthetic SSE rejection","execution_started":false}});
                if matches!(rejection, Rejection::UnsupportedHandshakeSse) {
                    vec![json!({"type":"response.created","response":{"id":"resp_unpublished","status":"in_progress","output":[]}}), error]
                } else {
                    vec![error]
                }
            } else if account == "primary" && matches!(rejection, Rejection::AfterOutput) {
                vec![json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_write","call_id":"write","name":"write","arguments":"{}","status":"completed"}}),
                    json!({"type":"error","error":{"code":"unsupported_tool","param":"tools[0].type","message":"Already generated output"}})]
            } else if account == "primary" && matches!(rejection, Rejection::Incomplete) {
                let item = json!({"type":"message","id":"msg_partial","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Partial local anchor","annotations":[]}]});
                vec![
                    json!({"type":"response.created","response":{"id":"resp_partial","status":"in_progress","output":[]}}),
                    json!({"type":"response.output_item.done","output_index":0,"item":item}),
                    json!({"type":"response.incomplete","response":{"id":"resp_partial","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[item],"usage":{"input_tokens":1,"output_tokens":1}}}),
                ]
            } else {
                let id = if account == "primary" { "resp_fallback" } else { "resp_fallback_backup" };
                vec![json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[{"type":"message","id":"msg_ok","role":"assistant","status":"completed","content":[{"type":"output_text","text":"OK","annotations":[]}]}],"usage":{"input_tokens":1,"output_tokens":1}}})]
            };
            let payload: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
            Response::builder().header("content-type", "text/event-stream").body(Body::from(payload)).unwrap()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = Arc::new(FallbackHost {
        base,
        enabled,
        preflight,
        backup,
        incompatible_first: AtomicBool::new(false),
        all_incompatible: AtomicBool::new(false),
        permit_releases: Arc::new(AtomicUsize::new(0)),
        seen: Mutex::new(Vec::new()),
        reports: Mutex::new(Vec::new()),
    });
    let dir = tempfile::tempdir().unwrap();
    let service = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        dir.path().join("identity.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    (host, captures, service, dir, task)
}
fn request() -> Value {
    json!({"model":"same-model","stream":false,"client_metadata":{"thread_id":"root","session_id":"root"},
        "input":[{"role":"user","content":"Use the requested capability."}],
        "tools":[{"type":"code_interpreter","container":{"type":"auto"}},{"type":"function","name":"write","parameters":{"type":"object","properties":{}}}]})
}
async fn post_request(service: &Handle, body: &Value, key: &str) -> reqwest::Response {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth(key)
        .json(body)
        .send()
        .await
        .unwrap()
}
async fn report(host: &FallbackHost) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while host.reports.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn capability_fallback_preflight_preserves_schema_model_and_records_only_target_attempt() {
    for input in [
        Some(json!("original string")),
        None,
        Some(json!([])),
        Some(request()["input"].clone()),
    ] {
        let (host, captures, mut service, _dir, task) =
            fixture(true, true, true, Rejection::None).await;
        let mut body = request();
        match input {
            Some(input) => body["input"] = input,
            None => {
                body.as_object_mut().unwrap().remove("input");
            }
        }
        let response = post_request(&service, &body, "caller").await;
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        report(&host).await;
        let captures = captures.lock().unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].0, "backup");
        assert_eq!(captures[0].1["tools"], body["tools"]);
        assert_eq!(captures[0].1["model"], "same-model");
        drop(captures);
        let reports = host.reports.lock().unwrap();
        let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), 1);
        let trace = attempts[0].capability_fallback.as_ref().unwrap();
        assert_eq!(trace.reason, "preflight");
        assert_eq!(trace.from_account_id, "primary");
        assert_eq!(trace.to_account_id, "backup");
        drop(reports);
        assert!(host
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|(caller, _)| caller == "caller"));
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn capability_fallback_disabled_acl_no_backup_and_unsafe_history_do_not_send_to_backups() {
    for (enabled, backup, key, unsafe_history) in [
        (false, true, "caller", false),
        (true, false, "caller", false),
        (true, true, "restricted", false),
        (true, true, "caller", true),
    ] {
        let (host, captures, mut service, _dir, task) =
            fixture(enabled, true, backup, Rejection::None).await;
        let mut body = request();
        if unsafe_history {
            body["input"] = json!([{"type":"function_call","id":"fc_pending","call_id":"pending","name":"write","arguments":"{}"}]);
        }
        let response = post_request(&service, &body, key).await;
        assert!(!response.status().is_success());
        let error: Value = response.json().await.unwrap();
        if !enabled {
            assert_eq!(
                error["error"]["code"],
                "subscription_sharing_unsupported_capability"
            );
        }
        assert!(captures.lock().unwrap().is_empty());
        assert!(host
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|(caller, _)| caller == key));
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn capability_fallback_only_retries_precise_pre_execution_errors() {
    for (mode, expected) in [
        (Rejection::Unsupported, 2),
        (Rejection::Invalid, 1),
        (Rejection::AfterOutput, 1),
    ] {
        let (host, captures, mut service, _dir, task) = fixture(true, false, true, mode).await;
        let mut body = request();
        body["stream"] = json!(true);
        let response = post_request(&service, &body, "caller").await;
        let text = response.text().await.unwrap();
        assert_eq!(captures.lock().unwrap().len(), expected, "{text}");
        if expected == 2 {
            assert!(text.contains("response.completed"));
            report(&host).await;
            let reports = host.reports.lock().unwrap();
            let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
            assert_eq!(attempts.len(), 2);
            assert!(attempts[0].capability_fallback.is_none());
            assert_eq!(
                attempts[1].capability_fallback.as_ref().unwrap().reason,
                "upstream_unsupported"
            );
        } else {
            assert!(!text.contains("response.completed"));
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn capability_fallback_handles_precise_sse_rejections_before_output_for_both_client_modes() {
    for mode in [Rejection::UnsupportedSse, Rejection::UnsupportedHandshakeSse] {
        for streaming in [false, true] {
            let (host, captures, mut service, _dir, task) = fixture(true, false, true, mode).await;
            let mut body = request();
            body["stream"] = json!(streaming);
            let response = post_request(&service, &body, "caller").await;
            assert_eq!(response.status(), 200);
            let text = response.text().await.unwrap();
            assert!(text.contains("resp_fallback_backup"), "{text}");
            assert!(!text.contains("resp_unpublished"), "a buffered source handshake must not escape before fallback: {text}");
            let sent = captures.lock().unwrap();
            assert_eq!(sent.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["primary", "backup"]);
            assert_eq!(sent[1].1["tools"], body["tools"]);
            assert_eq!(sent[1].1["model"], body["model"]);
            drop(sent);
            report(&host).await;
            let reports = host.reports.lock().unwrap();
            let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
            assert_eq!(attempts.len(), 2);
            assert_eq!(attempts[0].status, Some(400));
            assert_eq!(attempts[1].capability_fallback.as_ref().unwrap().reason, "upstream_unsupported");
            drop(reports);
            service.shutdown(Duration::from_secs(2)).await.unwrap();
            task.abort();
        }
    }
}

#[tokio::test]
async fn capability_fallback_sse_parameter_faults_and_generated_output_never_replay() {
    for mode in [Rejection::InvalidSse, Rejection::AfterOutput] {
        let (_host, captures, mut service, _dir, task) = fixture(true, false, true, mode).await;
        let mut body = request();
        body["stream"] = json!(true);
        let response = post_request(&service, &body, "caller").await;
        let text = response.text().await.unwrap();
        let sent = captures.lock().unwrap();
        assert_eq!(sent.len(), 1, "{text}");
        assert_eq!(sent[0].0, "primary");
        assert!(!text.contains("resp_fallback_backup"));
        if matches!(mode, Rejection::AfterOutput) {
            assert!(text.contains("fc_write"), "the existing tool output remains visible: {text}");
        }
        drop(sent);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn capability_fallback_rejects_unportable_resources_unknown_previous_and_incompatible_ciphertext(
) {
    for input in [
        json!([{"role":"user","content":[{"type":"input_file","file_id":"file_owned"}]}]),
        json!([{"type":"reasoning","id":"rs_private","summary":[],"encrypted_content":"synthetic_cipher"},{"role":"user","content":"continue"}]),
        json!([{"type":"additional_tools","role":"developer","tools":[{"type":"file_search","vector_store_ids":["vs_owned"]}]}]),
        json!([{"type":"tool_search_call","execution":"client","call_id":"pending","arguments":{}}]),
    ] {
        let (_host, captures, mut service, _dir, task) =
            fixture(true, true, true, Rejection::None).await;
        let mut body = request();
        body["input"] = input;
        let response = post_request(&service, &body, "caller").await;
        assert!(!response.status().is_success());
        assert!(captures.lock().unwrap().is_empty());
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
    let (_host, captures, mut service, _dir, task) =
        fixture(true, true, true, Rejection::None).await;
    let mut body = request();
    body["previous_response_id"] = json!("never-observed");
    assert!(!post_request(&service, &body, "caller")
        .await
        .status()
        .is_success());
    assert!(captures.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn capability_fallback_never_restarts_an_exhausted_backup_pool() {
    let (host, captures, mut service, _dir, task) =
        fixture(true, false, false, Rejection::Unsupported).await;
    let response = post_request(&service, &request(), "caller").await;
    assert!(!response.status().is_success());
    assert_eq!(captures.lock().unwrap().len(), 1);
    assert_eq!(host.seen.lock().unwrap().len(), 2);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn capability_fallback_skips_incompatible_candidate_before_sending_and_preserves_source_trace(
) {
    let (host, captures, mut service, _dir, task) =
        fixture(true, true, true, Rejection::None).await;
    host.incompatible_first.store(true, Ordering::SeqCst);
    let response = post_request(&service, &request(), "caller").await;
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    report(&host).await;
    let captures = captures.lock().unwrap();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].0, "backup");
    drop(captures);
    let seen = host.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen[2].1.capability_fallback_from.as_deref(),
        Some("primary")
    );
    assert!(seen[2]
        .1
        .excluded_account_ids
        .iter()
        .any(|id| id == "incompatible"));
    drop(seen);
    let reports = host.reports.lock().unwrap();
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(attempts.len(), 1);
    let trace = attempts[0].capability_fallback.as_ref().unwrap();
    assert_eq!(trace.from_account_id, "primary");
    assert_eq!(trace.to_account_id, "backup");
    drop(reports);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn capability_fallback_all_incompatible_candidates_send_nothing_and_keep_accepted_history() {
    let (host, captures, mut service, dir, task) = fixture(true, true, true, Rejection::None).await;
    let mut initial = request();
    initial["tools"] = json!([]);
    let first = post_request(&service, &initial, "caller").await;
    assert_eq!(first.status(), 200);
    let first: Value = first.json().await.unwrap();
    let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
    let accepted = || {
        connection.query_row(
        "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='resp_fallback'", [], |row| row.get::<_,String>(0)).unwrap()
    };
    let before = accepted();
    host.incompatible_first.store(true, Ordering::SeqCst);
    host.all_incompatible.store(true, Ordering::SeqCst);
    let mut body = request();
    body["input"] = json!([initial["input"][0], first["output"][0], {"role":"user","content":"Now use the required capability."}]);
    let source = body.clone();
    let response = post_request(&service, &body, "caller").await;
    assert_eq!(response.status(), 409);
    let error: Value = response.json().await.unwrap();
    assert_eq!(
        error["error"]["code"],
        "capability_fallback_protocol_unsafe"
    );
    assert_eq!(
        captures.lock().unwrap().len(),
        1,
        "Only the earlier accepted turn was sent"
    );
    assert_eq!(accepted(), before);
    assert_eq!(body, source);
    assert!(host
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, request)| request.capability_fallback_from.is_some())
        .all(|(_, request)| request.capability_fallback_from.as_deref() == Some("primary")));
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn capability_fallback_downstream_websocket_uses_same_preflight_and_full_http_request() {
    let (_host, captures, mut service, _dir, task) =
        fixture(true, true, true, Rejection::None).await;
    let mut handshake = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    handshake
        .headers_mut()
        .insert("authorization", "Bearer caller".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(handshake).await.unwrap();
    let mut body = request();
    body["type"] = json!("response.create");
    body["stream"] = json!(true);
    ws.send(Message::Text(body.to_string().into()))
        .await
        .unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                let event: Value = serde_json::from_str(&text).unwrap();
                if matches!(
                    event["type"].as_str(),
                    Some("response.completed" | "error" | "response.failed")
                ) {
                    break event;
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal["type"], "response.completed", "{terminal}");
    let captures = captures.lock().unwrap();
    assert_eq!(captures.len(), 1);
    assert_eq!(captures[0].0, "backup");
    assert_eq!(captures[0].1["tools"], body["tools"]);
    drop(captures);
    ws.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn socket_terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket
                .next()
                .await
                .expect("gateway remains connected")
                .unwrap()
            {
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if matches!(
                        event["type"].as_str(),
                        Some(
                            "response.completed"
                                | "response.incomplete"
                                | "response.failed"
                                | "error"
                        )
                    ) {
                        return event;
                    }
                }
                Message::Ping(value) => socket.send(Message::Pong(value)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn capability_fallback_websocket_continuation_rebuilds_only_completed_owned_history() {
    for mode in ["completed", "unknown", "partial"] {
        let first_rejection = if mode == "partial" {
            Rejection::Incomplete
        } else {
            Rejection::None
        };
        let (_host, captures, mut service, dir, task) =
            fixture(true, true, true, first_rejection).await;
        let mut handshake = format!("ws://{}/v1/responses", service.addr)
            .into_client_request()
            .unwrap();
        handshake
            .headers_mut()
            .insert("authorization", "Bearer caller".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(handshake).await.unwrap();
        let mut first = request();
        first["type"] = json!("response.create");
        first["tools"] = json!([]);
        first["stream"] = json!(true);
        socket
            .send(Message::Text(first.to_string().into()))
            .await
            .unwrap();
        let first_result = socket_terminal(&mut socket).await;
        assert_eq!(
            first_result["type"],
            if mode == "partial" {
                "response.incomplete"
            } else {
                "response.completed"
            },
            "{mode}: {first_result}"
        );
        let mut next = request();
        next["type"] = json!("response.create");
        next["stream"] = json!(true);
        next["previous_response_id"] = if mode == "unknown" {
            json!("never_observed")
        } else {
            first_result["response"]["id"].clone()
        };
        next["input"] = json!([{"role":"user","content":"Now use the new capability."}]);
        socket
            .send(Message::Text(next.to_string().into()))
            .await
            .unwrap();
        let second = socket_terminal(&mut socket).await;
        if mode == "completed" {
            assert_eq!(second["type"], "response.completed", "{second}");
            let captures = captures.lock().unwrap();
            assert_eq!(captures.len(), 2);
            assert_eq!(captures[0].0, "primary");
            assert_eq!(captures[1].0, "backup");
            let replay = &captures[1].1;
            assert!(replay.get("previous_response_id").is_none());
            assert_eq!(replay["input"].as_array().unwrap().len(), 3);
            assert_eq!(replay["input"][1]["content"][0]["text"], "OK");
            assert_eq!(replay["input"][2]["content"], "Now use the new capability.");
            assert_eq!(replay["tools"], next["tools"]);
            drop(captures);
            let connection =
                rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
            let active: String = connection.query_row("SELECT account FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='root'", [], |row| row.get(0)).unwrap();
            assert_eq!(active, "backup");
        } else {
            assert_eq!(second["type"], "error", "{mode}: {second}");
            if mode == "partial" {
                assert_eq!(second["status"], 409);
            } else {
                assert!(matches!(second["status"].as_u64(), Some(400 | 409)));
            }
            assert_eq!(
                captures.lock().unwrap().len(),
                1,
                "{mode}: no cross-owner send"
            );
        }
        let _ = socket.close(None).await;
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}
