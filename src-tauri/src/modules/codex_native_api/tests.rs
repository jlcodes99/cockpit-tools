use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    extract::{
        ws::{Message, WebSocketUpgrade},
        State,
    },
    http::{HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use futures_util::{future::BoxFuture, SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

use super::{contract::*, start, Handle, Options};

#[path = "http_parity_tests.rs"]
mod http_parity_tests;
#[path = "request_size_tests.rs"]
mod request_size_tests;
#[path = "thinking_suffix_tests.rs"]
mod thinking_suffix_tests;

struct TestHost {
    base_url: String,
    chatgpt: bool,
    proxy_url: Mutex<Option<String>>,
    ws_enabled: AtomicBool,
    key_enabled: AtomicBool,
    change_owner_on_refresh: AtomicBool,
    route_by_model: AtomicBool,
    bridge_with_policy: AtomicBool,
    reports: Mutex<Vec<RequestReport>>,
    diagnostics: Mutex<Vec<super::diagnostics::DiagnosticContext>>,
    selections: Mutex<Vec<RouteRequest>>,
    capacity: Arc<tokio::sync::Semaphore>,
}

impl Host for TestHost {
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        // This fixture exercises single-account transport/refresh behavior.
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 1,
                ..Default::default()
            })
        })
    }
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(!self.bridge_with_policy.load(Ordering::SeqCst)) })
    }
    fn prepare_body<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
        _: &'a HeaderMap,
        mut body: Value,
        _: bool,
        _: bool,
    ) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async move {
            if self.bridge_with_policy.load(Ordering::SeqCst) {
                body["max_output_tokens"] = json!(7);
            }
            Ok(body)
        })
    }
    fn websockets_enabled(&self) -> BoxFuture<'_, Result<bool, ServiceError>> {
        Box::pin(async { Ok(self.ws_enabled.load(Ordering::SeqCst)) })
    }
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if bearer != "local-test-key" || !self.key_enabled.load(Ordering::SeqCst) {
                return Err(ServiceError::new(
                    401,
                    "invalid_api_key",
                    "Invalid test key",
                ));
            }
            Ok(Caller {
                id: "caller-a".to_string(),
                label: "test".to_string(),
            })
        })
    }
    fn route<'a>(
        &'a self,
        _caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            let permit: Option<Arc<dyn Send + Sync>> = if request.model.is_empty() {
                None
            } else {
                Some(Arc::new(
                    self.capacity.clone().try_acquire_owned().map_err(|_| {
                        ServiceError::new(429, "account_busy", "Test account is busy")
                    })?,
                ))
            };
            let mut headers = HeaderMap::new();
            headers.insert(
                "authorization",
                HeaderValue::from_static("Bearer upstream-test-key"),
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
                account_id: if self.route_by_model.load(Ordering::SeqCst) {
                    format!("account-{}", request.model)
                } else {
                    "account-a".to_string()
                },
                owner_id: if request.force_refresh
                    && self.change_owner_on_refresh.load(Ordering::SeqCst)
                {
                    "owner-b"
                } else if self.route_by_model.load(Ordering::SeqCst) {
                    // Test-only routes use distinct real owners for different models.
                    &request.model
                } else {
                    "owner-a"
                }
                .to_string(),
                base_url: self.base_url.clone(),
                model: if self.bridge_with_policy.load(Ordering::SeqCst) {
                    "mapped-model".into()
                } else {
                    request.model
                },
                headers,
                client: {
                    let proxy = self.proxy_url.lock().unwrap().clone();
                    let builder = reqwest::Client::builder().no_proxy();
                    match proxy {
                        Some(proxy) => builder.proxy(reqwest::Proxy::all(proxy).unwrap()),
                        None => builder,
                    }
                    .build()
                    .unwrap()
                },
                proxy_url: self.proxy_url.lock().unwrap().clone(),
                proxy_route: None,
                chatgpt: self.chatgpt,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: (self.chatgpt
                    || self.bridge_with_policy.load(Ordering::SeqCst))
                .then(|| "priority".to_string()),
                permit,
            })
        })
    }
    fn models<'a>(&'a self, _caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async {
            Ok(json!({"object":"list","data":[{"id":"test-model","object":"model"}]}))
        })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn diagnostic(&self, context: super::diagnostics::DiagnosticContext) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.diagnostics.lock().unwrap().push(context);
        })
    }
}

async fn mock(app: Router) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (addr, task)
}

async fn native(
    addr: SocketAddr,
    chatgpt: bool,
    proxy: bool,
    options: Options,
) -> (Handle, Arc<TestHost>, PathBuf) {
    let host = Arc::new(TestHost {
        base_url: format!("http://{addr}/v1"),
        chatgpt,
        proxy_url: Mutex::new(proxy.then(|| "http://127.0.0.1:9".to_string())),
        ws_enabled: AtomicBool::new(true),
        key_enabled: AtomicBool::new(true),
        change_owner_on_refresh: AtomicBool::new(false),
        route_by_model: AtomicBool::new(false),
        bridge_with_policy: AtomicBool::new(false),
        reports: Mutex::new(Vec::new()),
        diagnostics: Mutex::new(Vec::new()),
        selections: Mutex::new(Vec::new()),
        capacity: Arc::new(tokio::sync::Semaphore::new(1)),
    });
    let path =
        std::env::temp_dir().join(format!("cockpit-native-api-test-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&path).unwrap();
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.join("identity.json"),
        options,
    )
    .await
    .unwrap();
    (handle, host, path)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn body(stream: bool) -> Value {
    json!({"model":"test-model","stream":stream,"input":[{"role":"user","content":"test"}],"unknown":{"preserve":true},"reasoning":{"effort":"high"}})
}
fn created() -> &'static str {
    "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp-a\"}}\n\n"
}
fn completed() -> &'static str {
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-a\",\"status\":\"completed\",\"output\":[],\"unknown\":true,\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}}\n\n"
}

async fn ws_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn websocket_model_routes_append_and_late_interrupt_preserve_owner_identity() {
    let seen = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let sequence = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = Router::new().route("/v1/responses", get({
        let seen = seen.clone();
        move |headers: HeaderMap, ws: WebSocketUpgrade| {
            let seen = seen.clone();
            let sequence = sequence.clone();
            async move { ws.on_upgrade(move |mut socket| async move {
                while let Some(Ok(Message::Text(text))) = socket.recv().await {
                    let request: Value = serde_json::from_str(text.as_str()).unwrap();
                    assert_eq!(request["type"], "response.create");
                    seen.lock().unwrap().push((headers.clone(), request));
                    let id = format!("model-response-{}", sequence.fetch_add(1, Ordering::SeqCst));
                    socket.send(Message::Text(json!({"type":"response.created","response":{"id":id}}).to_string().into())).await.unwrap();
                    socket.send(Message::Text(json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[]}}).to_string().into())).await.unwrap();
                }
            }) }
        }
    }));
    let (addr, task) = mock(app).await;
    let (mut service, host, _) = native(addr, true, false, Options::default()).await;
    host.route_by_model.store(true, Ordering::SeqCst);
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    request
        .headers_mut()
        .insert("session-id", "unchanged-client-root".parse().unwrap());
    request
        .headers_mut()
        .insert("thread-id", "unchanged-client-root".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(ClientMessage::Text(
        json!({"type":"response.create","model":"first","input":[]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    assert_eq!(ws_json(&mut ws).await["type"], "response.completed");
    ws.send(ClientMessage::Text(
        json!({"type":"response.interrupt","response_id":"model-response-0"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    ws.send(ClientMessage::Text(
        json!({"type":"response.append","input":[{"role":"user","content":"next"}]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    assert_eq!(ws_json(&mut ws).await["type"], "response.completed");
    ws.send(ClientMessage::Text(
        json!({"type":"response.create","model":"second","input":[]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    assert_eq!(ws_json(&mut ws).await["type"], "response.completed");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1].1["model"], "first");
    assert_eq!(seen[1].1["previous_response_id"], "model-response-0");
    assert_eq!(seen[0].0["session-id"], seen[1].0["session-id"]);
    assert_ne!(seen[0].0["session-id"], seen[2].0["session-id"]);
    assert_ne!(seen[0].0["thread-id"], seen[2].0["thread-id"]);
    assert!(seen[2].1.get("previous_response_id").is_none());
    let selections = host.selections.lock().unwrap().clone();
    assert_eq!(selections.len(), 3);
    assert!(selections
        .iter()
        .all(|selection| selection.auxiliary.as_deref() == Some("responses.websocket")));
    host.key_enabled.store(false, Ordering::SeqCst);
    ws.send(ClientMessage::Text(
        json!({"type":"response.create","model":"second","input":[]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["error"]["code"], "invalid_api_key");
    let _ = ws.close(None).await;
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}

#[tokio::test]
async fn automatic_history_websocket_interrupt_rejection_keeps_response_and_connection_active() {
    let app = Router::new().route("/v1/responses", get(|ws: WebSocketUpgrade| async {
        ws.on_upgrade(|mut socket| async move {
            let _ = socket.recv().await.unwrap().unwrap();
            socket.send(Message::Text(json!({"type":"response.created","response":{"id":"interrupt-response"}}).to_string().into())).await.unwrap();
            let interrupt = socket.recv().await.unwrap().unwrap();
            assert_eq!(serde_json::from_str::<Value>(interrupt.to_text().unwrap()).unwrap()["response_id"], "interrupt-response");
            socket.send(Message::Text(json!({"type":"error","status":400,"error":{"code":"response_interrupt_not_supported","message":"response.interrupt is not supported"}}).to_string().into())).await.unwrap();
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"interrupt-response","status":"completed","output":[]}}).to_string().into())).await.unwrap();
            let next = socket.recv().await.unwrap().unwrap();
            assert_eq!(serde_json::from_str::<Value>(next.to_text().unwrap()).unwrap()["model"], "test-model");
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"after-interrupt","status":"completed","output":[]}}).to_string().into())).await.unwrap();
            while socket.recv().await.is_some() {}
        })
    }));
    let (addr, task) = mock(app).await;
    let (mut service, host, path) = native(addr, true, false, Options::default()).await;

    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(ClientMessage::Text(
        json!({"type":"response.create","model":"test-model","client_metadata":{"thread_id":"legacy-thread"},"input":[{"type":"reasoning","encrypted_content":"legacy-cipher"}]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    // Unknown control targets fail locally without changing the active response.
    ws.send(ClientMessage::Text(
        json!({"type":"response.interrupt","response_id":"unknown"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(
        ws_json(&mut ws).await["error"]["code"],
        "unknown_interrupt_response"
    );
    ws.send(ClientMessage::Text(
        json!({"type":"response.interrupt","response_id":"interrupt-response"})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(
        ws_json(&mut ws).await["error"]["code"],
        "response_interrupt_not_supported"
    );
    assert_eq!(ws_json(&mut ws).await["type"], "response.completed");
    ws.send(ClientMessage::Text(
        json!({"type":"response.create","input":[]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.completed");
    ws.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|report| report.success));
    task.abort();
}

#[tokio::test]
async fn http_maps_identity_preserves_events_and_keeps_local_keys_private() {
    let seen = Arc::new(Mutex::new(Vec::<(HeaderMap, Value)>::new()));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(
                |State(seen): State<Arc<Mutex<Vec<(HeaderMap, Value)>>>>,
                 headers: HeaderMap,
                 bytes: Bytes| async move {
                    seen.lock()
                        .unwrap()
                        .push((headers, serde_json::from_slice(&bytes).unwrap()));
                    (
                        [("content-type", "text/event-stream")],
                        format!("{}{}", created(), completed()),
                    )
                },
            ),
        )
        .with_state(seen.clone());
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    let response = client()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("session-id", "client-root")
        .header("thread-id", "client-root")
        .header("x-cockpit-target-account-id", "account-a")
        .json(&body(true))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let output = response.text().await.unwrap();
    assert!(output.starts_with(created()));
    let terminal: Value = serde_json::from_str(
        output
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .last()
            .unwrap(),
    )
    .unwrap();
    let mut expected: Value = serde_json::from_str(
        completed()
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    expected["response"]["usage"]["input_tokens_details"] = json!({"cached_tokens":0});
    expected["response"]["usage"]["output_tokens_details"] = json!({"reasoning_tokens":0});
    assert_eq!(terminal, expected);
    let captured = seen.lock().unwrap();
    let (headers, sent) = &captured[0];
    assert_eq!(headers["authorization"], "Bearer upstream-test-key");
    assert!(headers.get("x-cockpit-target-account-id").is_none());
    assert_ne!(headers["session-id"], "client-root");
    assert_eq!(headers["session-id"], headers["thread-id"]);
    assert_eq!(sent["unknown"], json!({"preserve":true}));
    drop(captured);
    assert_eq!(
        host.selections.lock().unwrap()[0]
            .target_account_id
            .as_deref(),
        Some("account-a")
    );
    assert_eq!(host.capacity.available_permits(), 1);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn oauth_nonstream_collects_final_response_without_losing_unknown_fields() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(
                |State(seen): State<Arc<Mutex<Vec<Value>>>>, bytes: Bytes| async move {
                    seen.lock()
                        .unwrap()
                        .push(serde_json::from_slice(&bytes).unwrap());
                    (
                        [("content-type", "text/event-stream")],
                        format!("{}{}", created(), completed()),
                    )
                },
            ),
        )
        .with_state(seen.clone());
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, true, false, Options::default()).await;
    let response = client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("session-id", "root")
        .header("thread-id", "root")
        .json(&body(false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["content-type"], "application/json");
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["unknown"], true);
    assert_eq!(result["id"], "resp-a");
    assert_eq!(seen.lock().unwrap()[0]["stream"], true);
    assert_eq!(seen.lock().unwrap()[0]["store"], false);
    assert_eq!(seen.lock().unwrap()[0]["service_tier"], "priority");
    let mut explicit_tier = body(false);
    explicit_tier["service_tier"] = json!("flex");
    client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("session-id", "root")
        .header("thread-id", "root")
        .json(&explicit_tier)
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    // The Go OAuth executor omits unsupported tiers; client intent is retained
    // in diagnostics, while API-key routes preserve their own wire settings.
    assert!(seen.lock().unwrap()[1].get("service_tier").is_none());
    {
        let reports = host.reports.lock().unwrap();
        assert!(reports[0].success);
        assert_eq!(
            reports[0].requested_service_tier.as_deref(),
            Some("priority")
        );
        assert_eq!(
            reports[0].response_service_tier.as_deref(),
            Some("priority")
        );
        assert_eq!(reports[0].reasoning_effort.as_deref(), Some("high"));
        assert_eq!(reports[1].requested_service_tier.as_deref(), Some("flex"));
        assert_eq!(
            reports[1].response_service_tier.as_deref(),
            Some("priority")
        );
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn shutdown_cancels_stalled_sse_and_releases_account_permit() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            let stream = futures_util::stream::once(async {
                Ok::<_, std::io::Error>(Bytes::from_static(created().as_bytes()))
            })
            .chain(futures_util::stream::pending());
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(stream))
                .unwrap()
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    let mut response = client()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("session-id", "root")
        .header("thread-id", "root")
        .json(&body(true))
        .send()
        .await
        .unwrap();
    assert!(response.chunk().await.unwrap().is_some());
    assert_eq!(host.capacity.available_permits(), 0);
    assert!(!service.pause_if_idle());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(service.active_requests(), 0);
    assert_eq!(host.capacity.available_permits(), 1);
    upstream.abort();
}

#[tokio::test]
async fn body_limit_and_api_auth_fail_before_upstream_routing() {
    let (addr, upstream) = mock(Router::new()).await;
    let options = Options {
        max_body_bytes: 16,
        ..Options::default()
    };
    let (mut service, host, _) = native(addr, false, false, options).await;
    let response = client()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-test-key")
        .json(&body(true))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let response = client()
        .get(format!("http://{}/v1/models", service.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(host.selections.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn websocket_uses_authenticated_proxy_without_leaking_proxy_credentials() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let origin_headers = Arc::new(Mutex::new(None::<HeaderMap>));
    let app = Router::new().route("/v1/responses", get(|State(seen): State<Arc<Mutex<Option<HeaderMap>>>>, headers: HeaderMap, ws: WebSocketUpgrade| async move {
        *seen.lock().unwrap() = Some(headers);
        ws.on_upgrade(|mut socket| async move {
            socket.recv().await.unwrap().unwrap();
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"proxy-response","status":"completed","output":[]}}).to_string().into())).await.unwrap();
            while socket.recv().await.is_some() {}
        })
    })).with_state(origin_headers.clone());
    let (addr, upstream) = mock(app).await;
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let proxy_headers = Arc::new(Mutex::new(String::new()));
    let seen_proxy = proxy_headers.clone();
    let proxy_task = tokio::spawn(async move {
        let (mut downstream, _) = proxy.accept().await.unwrap();
        let mut header = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            downstream.read_exact(&mut byte).await.unwrap();
            header.push(byte[0]);
            assert!(header.len() < 16 * 1024);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header = String::from_utf8(header).unwrap();
        *seen_proxy.lock().unwrap() = header.clone();
        let mut lines = header.split("\r\n");
        let first = lines.next().unwrap();
        assert!(first.starts_with(&format!("GET http://{addr}/v1/responses ")));
        let mut forwarded = String::from("GET /v1/responses HTTP/1.1\r\n");
        for line in lines.filter(|line| !line.is_empty()) {
            if !line
                .to_ascii_lowercase()
                .starts_with("proxy-authorization:")
            {
                forwarded.push_str(line);
                forwarded.push_str("\r\n");
            }
        }
        forwarded.push_str("\r\n");
        let mut origin = tokio::net::TcpStream::connect(addr).await.unwrap();
        origin.write_all(forwarded.as_bytes()).await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut downstream, &mut origin).await;
    });
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    *host.proxy_url.lock().unwrap() = Some(format!("http://proxy-user:proxy-secret@{proxy_addr}"));
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer local-test-key"),
    );
    request.headers_mut().insert(
        "proxy-authorization",
        HeaderValue::from_static("Basic untrusted-client-secret"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"test-model","input":[]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(response.to_text().unwrap().contains("response.completed"));
    let headers = proxy_headers.lock().unwrap().to_ascii_lowercase();
    use base64::Engine;
    let expected = base64::engine::general_purpose::STANDARD
        .encode("proxy-user:proxy-secret")
        .to_ascii_lowercase();
    assert!(
        headers.contains(&format!("proxy-authorization: basic {expected}")),
        "{headers}"
    );
    assert!(!headers.contains("untrusted-client-secret"));
    let seen = origin_headers.lock().unwrap().clone().unwrap();
    assert!(!seen.contains_key("proxy-authorization"));
    assert_eq!(seen["authorization"], "Bearer upstream-test-key");
    socket.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    proxy_task.abort();
    upstream.abort();
}

#[tokio::test]
async fn websocket_forwards_create_and_interrupt_without_reassigning_account() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new().route("/v1/responses", get(|State(seen): State<Arc<Mutex<Vec<Value>>>>, ws: WebSocketUpgrade| async move {
        ws.on_upgrade(move |mut socket| async move {
            let first = socket.recv().await.unwrap().unwrap();
            if let Message::Text(text) = first { seen.lock().unwrap().push(serde_json::from_str(text.as_str()).unwrap()); }
            socket.send(Message::Text(json!({"type":"response.created","response":{"id":"resp-ws"}}).to_string().into())).await.unwrap();
            let second = socket.recv().await.unwrap().unwrap();
            if let Message::Text(text) = second { seen.lock().unwrap().push(serde_json::from_str(text.as_str()).unwrap()); }
            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"resp-ws","status":"completed","output":[]}}).to_string().into())).await.unwrap();
            while socket.recv().await.is_some() {}
        })
    })).with_state(seen.clone());
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, true, false, Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer local-test-key"),
    );
    request
        .headers_mut()
        .insert("session-id", HeaderValue::from_static("ws-root"));
    request
        .headers_mut()
        .insert("thread-id", HeaderValue::from_static("ws-root"));
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let mut create = body(false);
    create["type"] = json!("response.create");
    ws.send(ClientMessage::Text(create.to_string().into()))
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let first: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(first["type"], "response.created");
    let interrupt =
        json!({"type":"response.interrupt","response_id":"resp-ws","mode":"discard_partial_items"});
    ws.send(ClientMessage::Text(interrupt.to_string().into()))
        .await
        .unwrap();
    let done = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let done: Value = serde_json::from_str(done.to_text().unwrap()).unwrap();
    assert_eq!(done["type"], "response.completed");
    assert_eq!(seen.lock().unwrap()[1], interrupt);
    assert_eq!(seen.lock().unwrap()[0]["stream"], true);
    assert_eq!(seen.lock().unwrap()[0]["store"], false);
    assert_eq!(seen.lock().unwrap()[0]["unknown"], json!({"preserve":true}));
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.active_requests() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(service.pause_if_idle());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    drop(ws);
    upstream.abort();
}

#[tokio::test]
async fn only_explicit_401_retries_once_and_429_preserves_retry_after() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for (first, second, expected_attempts) in
        [(401u16, 200u16, 2usize), (401, 401, 2), (429, 200, 1)]
    {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let counter = counter.clone();
                async move {
                    let status = if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        first
                    } else {
                        second
                    };
                    Response::builder()
                        .status(status)
                        .header("content-type", if status == 200 { "text/event-stream" } else { "application/json" })
                        .header("retry-after", "17")
                        .body(Body::from(if status == 200 {
                            "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"test-response\",\"status\":\"completed\",\"output\":[]}}\n\n"
                        } else {
                            "{\"error\":{\"code\":\"invalid_api_key\",\"message\":\"fixture rejected\"}}"
                        }))
                        .unwrap()
                }
            }),
        );
        let (addr, upstream) = mock(app).await;
        let (mut service, host, _) = native(addr, true, false, Options::default()).await;
        let response = client()
            .post(format!("http://{}/responses", service.addr))
            .bearer_auth("local-test-key")
            .header("session-id", "retry-root")
            .header("thread-id", "retry-root")
            .json(&body(false))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            if expected_attempts == 1 {
                first
            } else {
                second
            }
        );
        assert_eq!(response.headers()["retry-after"], "17");
        response.bytes().await.unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), expected_attempts);
        assert_eq!(
            host.reports.lock().unwrap()[0].retry_after,
            Some(Duration::from_secs(17))
        );
        let selections = host.selections.lock().unwrap();
        assert_eq!(selections.len(), expected_attempts);
        if expected_attempts == 2 {
            assert!(selections[1].force_refresh);
            assert_eq!(
                selections[1].preferred_account_id.as_deref(),
                Some("account-a")
            );
        }
        drop(selections);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        upstream.abort();
    }
}

#[tokio::test]
async fn compressed_body_is_decoded_with_a_decompressed_size_limit() {
    let (addr, upstream) = mock(Router::new()).await;
    let options = Options {
        max_body_bytes: 256,
        ..Options::default()
    };
    let (mut service, host, _) = native(addr, false, false, options).await;
    let compressed = zstd::stream::encode_all(vec![b'x'; 4096].as_slice(), 1).unwrap();
    assert!(compressed.len() < 256);
    let response = client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("content-encoding", "zstd")
        .body(compressed)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(host.selections.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn websocket_disabled_handshake_is_local_and_http_fallback_is_counted_once() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(format!("{}{}", created(), completed())))
                .unwrap()
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    host.ws_enabled.store(false, Ordering::SeqCst);
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer local-test-key"),
    );
    match tokio_tungstenite::connect_async(request).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED)
        }
        _ => panic!("disabled WebSocket must reject before upgrading"),
    }
    assert!(host.selections.lock().unwrap().is_empty());
    assert!(host.diagnostics.lock().unwrap().is_empty(),
        "local protocol negotiation is not an upstream request");
    let response = client()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("local-test-key")
        .json(&body(false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response: Value = response.json().await.unwrap();
    assert_eq!(response["status"], "completed");
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let reports = host.reports.lock().unwrap();
    assert_eq!(
        reports.len(),
        1,
        "protocol fallback is not an inference failure"
    );
    assert!(reports[0].success);
    assert_eq!(reports[0].status, 200);
    assert_eq!(reports[0].account_id, "account-a");
    let diagnostics = host.diagnostics.lock().unwrap();
    assert_eq!(diagnostics.len(), 1, "only the actual HTTP send is diagnostic");
    assert_eq!(diagnostics[0].method, "POST");
    assert_eq!(diagnostics[0].attempts.len(), 1);
    let attempt = &diagnostics[0].attempts[0];
    assert_eq!(attempt.status, Some(200));
    assert_eq!(attempt.transport, "http");
    assert_eq!(attempt.account_id, "account-a");
    assert!(attempt.success);
    upstream.abort();
}

#[tokio::test]
async fn websocket_actual_upstream_401_426_and_429_remain_failures() {
    for status in [401, 426, 429] {
        let app = Router::new().route(
            "/v1/responses",
            get(move || async move {
                Response::builder()
                    .status(status)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        "{\"error\":{\"message\":\"upstream rejected the handshake\"}}",
                    ))
                    .unwrap()
            })
            .post(move || async move {
                Response::builder()
                    .status(status)
                    .header("content-type", "text/plain")
                    .body(Body::from("upstream rejected the HTTP request"))
                    .unwrap()
            }),
        );
        let (addr, upstream) = mock(app).await;
        let (mut service, host, _) = native(addr, false, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses", service.addr)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_static("Bearer local-test-key"),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let mut request = body(false);
        request["type"] = json!("response.create");
        socket
            .send(ClientMessage::Text(request.to_string().into()))
            .await
            .unwrap();
        let error = ws_json(&mut socket).await;
        assert_eq!(error["type"], "error");
        assert_eq!(error["status"], status);
        let _ = socket.close(None).await;
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].success);
        assert_eq!(reports[0].status, status);
        assert_eq!(reports[0].account_id, "account-a");
        assert_ne!(
            reports[0]
                .diagnostics
                .as_ref()
                .unwrap()
                .failure_phase
                .as_deref(),
            Some("transport_fallback")
        );
        upstream.abort();
    }
}

#[tokio::test]
async fn disabled_websocket_does_not_mask_invalid_client_authentication() {
    let (addr, upstream) = mock(Router::new()).await;
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    host.ws_enabled.store(false, Ordering::SeqCst);
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer invalid-local-key"),
    );
    match tokio_tungstenite::connect_async(request).await {
        Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        _ => panic!("invalid authentication must reject before negotiating fallback"),
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    assert!(host.selections.lock().unwrap().is_empty());
    let diagnostics = host.diagnostics.lock().unwrap();
    assert!(diagnostics.is_empty(), "local authentication rejection has no upstream diagnostic");
    assert!(host.reports.lock().unwrap().is_empty());
    upstream.abort();
}

#[tokio::test]
async fn idle_switch_seals_admission_reuses_address_and_can_restore_after_start_failure() {
    let (addr, upstream) = mock(Router::new()).await;
    let (mut first, host, path) = native(addr, false, false, Options::default()).await;
    let address = first.addr;
    assert!(first.pause_if_idle());
    let rejected = client()
        .get(format!("http://{address}/v1/models"))
        .bearer_auth("local-test-key")
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(first.active_requests(), 0);
    first.shutdown(Duration::from_secs(2)).await.unwrap();
    let mut second = start(
        address,
        host.clone(),
        path.join("second.sqlite"),
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(second.addr, address);
    // A failed competing bind cannot stop the service that owns the port.
    assert!(start(
        address,
        host.clone(),
        path.join("conflict.sqlite"),
        Options::default()
    )
    .await
    .is_err());
    assert_eq!(
        client()
            .get(format!("http://{address}/v1/models"))
            .bearer_auth("local-test-key")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(second.pause_if_idle());
    second.shutdown(Duration::from_secs(2)).await.unwrap();
    // A directory cannot be opened as a state database. The old engine can still reclaim its exact address.
    assert!(
        start(address, host.clone(), path.clone(), Options::default())
            .await
            .is_err()
    );
    let mut restored = start(
        address,
        host,
        path.join("identity.json"),
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(restored.addr, address);
    assert_eq!(
        client()
            .get(format!("http://{address}/v1/models"))
            .bearer_auth("local-test-key")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    restored.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn unauthorized_retry_cannot_cross_owner_identity() {
    use std::sync::atomic::AtomicUsize;
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                (StatusCode::UNAUTHORIZED, "invalid token")
            }
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, true, false, Options::default()).await;
    host.change_owner_on_refresh.store(true, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("session-id", "refresh-root")
        .header("thread-id", "refresh-root")
        .json(&body(false))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["code"], "refresh_owner_changed");
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(host.capacity.available_permits(), 1);
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}

#[tokio::test]
async fn websocket_http_bridge_keeps_selected_model_policy_and_explicit_tier() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                capture.lock().unwrap().push(body);
                axum::Json(json!({"id":"bridge-mapped","status":"completed","output":[]}))
            }
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, host, _) = native(addr, false, false, Options::default()).await;
    host.bridge_with_policy.store(true, Ordering::SeqCst);
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer local-test-key"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    for tier in [None, Some("flex"), Some("ultrafast")] {
        let mut request = body(true);
        request["type"] = json!("response.create");
        if let Some(tier) = tier {
            request["service_tier"] = json!(tier);
        }
        socket
            .send(ClientMessage::Text(request.to_string().into()))
            .await
            .unwrap();
        loop {
            let event = ws_json(&mut socket).await;
            if event["type"] == "response.completed" {
                break;
            }
            assert_ne!(event["type"], "error", "{event}");
        }
    }
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 3);
    for request in requests.iter() {
        assert_eq!(request["model"], "mapped-model");
        assert_eq!(request["max_output_tokens"], 7);
        assert_eq!(request["reasoning"]["effort"], "high");
        assert_eq!(request["unknown"]["preserve"], true);
        assert!(request.get("type").is_none());
        assert!(request.get("generate").is_none());
    }
    assert_eq!(requests[0]["service_tier"], "priority");
    assert_eq!(requests[1]["service_tier"], "flex");
    assert_eq!(requests[2]["service_tier"], "ultrafast");
    drop(requests);
    let _ = socket.close(None).await;
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
}
