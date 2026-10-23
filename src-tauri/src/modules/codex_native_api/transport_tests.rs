use crate::modules::codex_native_api::{
    self as native,
    contract::*,
    lifecycle::{Activity, RequestTracker, StopSignal},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderMap,
    routing::post,
    Router,
};
use futures_util::{future::BoxFuture, StreamExt};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[path = "output_terminal_http_tests.rs"]
mod output_terminal_tests;
#[path = "go_limits_http_tests.rs"]
mod go_limits_tests;

struct PolicyHost {
    url: String,
    chatgpt: bool,
    policy: ExecutionPolicy,
    reports: Mutex<Vec<RequestReport>>,
    record_delay: Duration,
}

impl Host for PolicyHost {
    fn responses_websocket_passthrough<'a>(
        &'a self,
        _: &'a Caller,
        _: &'a Route,
    ) -> BoxFuture<'a, Result<bool, ServiceError>> {
        Box::pin(async { Ok(false) })
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
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
                account_id: "a".into(),
                owner_id: "a".into(),
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
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep(self.record_delay).await;
            self.reports.lock().unwrap().push(report);
        })
    }
}

async fn start(
    app: Router,
    policy: ExecutionPolicy,
) -> (
    native::Handle,
    Arc<PolicyHost>,
    tokio::task::JoinHandle<()>,
    std::path::PathBuf,
) {
    start_with_chatgpt(app, policy, false).await
}

async fn start_with_chatgpt(
    app: Router,
    policy: ExecutionPolicy,
    chatgpt: bool,
) -> (
    native::Handle,
    Arc<PolicyHost>,
    tokio::task::JoinHandle<()>,
    std::path::PathBuf,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(PolicyHost {
        url,
        chatgpt,
        policy,
        reports: Mutex::new(Vec::new()),
        record_delay: Duration::ZERO,
    });
    let path =
        std::env::temp_dir().join(format!("native-transport-{}.sqlite", uuid::Uuid::now_v7()));
    let handle = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        native::Options::default(),
    )
    .await
    .unwrap();
    (handle, host, task, path)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}
fn completed() -> &'static str {
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n"
}
async fn shutdown(
    mut handle: native::Handle,
    task: tokio::task::JoinHandle<()>,
    path: std::path::PathBuf,
) {
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn native_transport_total_timeout_bounds_active_json_stream() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            let stream = futures_util::stream::unfold(0, |n| async move {
                tokio::time::sleep(Duration::from_millis(25)).await;
                Some((Ok::<_, std::io::Error>(Bytes::from_static(b" ")), n + 1))
            });
            Body::from_stream(stream)
        }),
    );
    let policy = ExecutionPolicy {
        idle_timeout: Duration::from_millis(100),
        total_timeout: Duration::from_millis(180),
        ..Default::default()
    };
    let (handle, host, task, path) = start(app, policy).await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("upstream_total_timeout"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        host.reports.lock().unwrap()[0].error.as_deref(),
        Some("upstream_total_timeout")
    );
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn native_transport_immediate_sse_precedes_slow_upstream_headers() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            ([("content-type", "text/event-stream")], completed())
        }),
    );
    let policy = ExecutionPolicy {
        immediate_sse: true,
        keepalive: Duration::from_millis(30),
        ..Default::default()
    };
    let (handle, _, task, path) = start(app, policy).await;
    let response = tokio::time::timeout(
        Duration::from_millis(150),
        client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&json!({"model":"m","stream":true}))
            .send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    assert!(text.matches(": keep-alive").count() >= 2, "{text}");
    assert!(text.contains("response.completed"));
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn native_transport_bootstrap_retry_records_each_attempt() {
    let count = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/v1/responses",
        post({
            let count = count.clone();
            move || {
                let count = count.clone();
                async move {
                    (
                        [("content-type", "text/event-stream")],
                        if count.fetch_add(1, Ordering::SeqCst) == 0 {
                            ""
                        } else {
                            completed()
                        },
                    )
                }
            }
        }),
    );
    let policy = ExecutionPolicy {
        bootstrap_retries: 1,
        retry_base_delay: Duration::from_millis(1),
        ..Default::default()
    };
    let (handle, host, task, path) = start(app, policy).await;
    let text = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(text.contains("response.completed"));
    shutdown(handle, task, path).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].status, Some(502));
    assert_eq!(attempts[1].status, Some(200));
    assert!(attempts[1].success);
}

#[tokio::test]
async fn native_transport_terminal_record_survives_tracker_drop() {
    let host = Arc::new(PolicyHost {
        url: String::new(),
        chatgpt: false,
        policy: ExecutionPolicy::default(),
        reports: Mutex::new(Vec::new()),
        record_delay: Duration::from_millis(80),
    });
    let activity = Arc::new(Activity::default());
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let permit = Arc::new(tokio::sync::Semaphore::new(1))
        .acquire_owned()
        .await
        .unwrap();
    let mut tracker =
        RequestTracker::new(activity.clone(), host.clone(), StopSignal(receiver), permit);
    tracker.context("POST", "/v1/responses", &HeaderMap::new(), false);
    tracker.authorized(
        &Caller {
            id: "caller".into(),
            label: "test".into(),
        },
        "m",
    );
    tracker.begin_attempt("http");
    tracker.complete(200, true, None, None).await;
    drop(tracker);
    assert!(activity.tasks.load(Ordering::SeqCst) > 0);
    tokio::time::sleep(Duration::from_millis(120)).await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    assert_eq!(activity.tasks.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn go_limits_websocket_first_create_has_no_upstream_initial_deadline() {
    let policy = ExecutionPolicy {
        websocket_initial_timeout: Duration::from_millis(100),
        ..Default::default()
    };
    let (handle, _, task, path) = start(Router::new(), policy).await;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    // Go waits for the client's first create without consuming the upstream timer.
    assert!(tokio::time::timeout(Duration::from_millis(200), socket.next()).await.is_err());
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    socket.send(Message::Ping(b"still-open".to_vec().into())).await.unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await.unwrap().unwrap().unwrap();
    assert!(matches!(event, Message::Pong(value) if value.as_ref() == b"still-open"));
    drop(socket);
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn native_transport_rejects_forged_websocket_accept() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 8192];
        let _ = stream.read(&mut buffer).await.unwrap();
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: forged-value\r\n\r\n").await.unwrap();
    });
    let route = Route {
        continuity_account_id: None,
        installation_profile: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        authorization_scope_fingerprint: None,
                official_fallback_failure: None,
        transport_identity: None,
        account_id: "a".into(),
        owner_id: "a".into(),
        base_url: format!("http://{addr}"),
        model: "m".into(),
        headers: HeaderMap::new(),
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
    };
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let result = native::transport::connect_websocket(
        &route,
        &route.base_url,
        HeaderMap::new(),
        4096,
        Duration::from_secs(1),
        StopSignal(receiver),
    )
    .await;
    assert_eq!(result.unwrap_err().code, "invalid_websocket_handshake");
    task.await.unwrap();
}

struct GrokHost(Arc<PolicyHost>);
impl Host for GrokHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.0.authorize(key)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut route = self.0.route(caller, request).await?;
            route
                .headers
                .insert("x-xai-token-auth", "fixture-token".parse().unwrap());
            Ok(route)
        })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.0.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.0.record(report)
    }
}

#[tokio::test]
async fn native_transport_grok_http_and_websocket_restore_tool_namespaces() {
    use axum::{
        extract::ws::{Message, WebSocketUpgrade},
        Json,
    };
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};
    let response = |body: Value| {
        assert_eq!(body["tools"][0]["type"], "function");
        assert_ne!(body["tools"][0]["name"], "lookup");
        json!({"id":"grok-response","status":"completed","output":[{"id":"fc","call_id":"call","type":"function_call","name":body["tools"][0]["name"],"arguments":"{}"}]})
    };
    let app = Router::new().route(
        "/v1/responses",
        post(move |Json(body): Json<Value>| async move { Json(response(body)) }).get(
            move |ws: WebSocketUpgrade| async move {
                ws.on_upgrade(move |mut socket| async move {
                    let Some(Ok(Message::Text(text))) = socket.recv().await else {
                        panic!("missing create");
                    };
                    let body = serde_json::from_str(text.as_str()).unwrap();
                    socket
                        .send(Message::Text(
                            json!({"type":"response.completed","response":response(body)})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    while socket.recv().await.is_some() {}
                })
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async {
        axum::serve(listener, app).await.unwrap();
    });
    let host = Arc::new(GrokHost(Arc::new(PolicyHost {
        url,
        chatgpt: false,
        policy: ExecutionPolicy::default(),
        reports: Mutex::new(Vec::new()),
        record_delay: Duration::ZERO,
    })));
    let path = std::env::temp_dir().join(format!("grok-transport-{}.sqlite", uuid::Uuid::now_v7()));
    let handle = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        path.clone(),
        native::Options::default(),
    )
    .await
    .unwrap();
    let mut body = json!({"model":"m","input":[],"tools":[{"type":"namespace","name":"functions","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}]}]});
    let restored: Value = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["output"][0]["name"], "lookup");
    assert_eq!(restored["output"][0]["namespace"], "functions");
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    body["type"] = json!("response.create");
    socket
        .send(ClientMessage::Text(body.to_string().into()))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event: Value = serde_json::from_str(event.to_text().unwrap()).unwrap();
    assert_eq!(event["response"]["output"][0]["name"], "lookup");
    assert_eq!(event["response"]["output"][0]["namespace"], "functions");
    socket.close(None).await.unwrap();
    shutdown(handle, task, path).await;
}

#[tokio::test]
async fn native_transport_status_retries_use_their_own_budget_and_backoff() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let upstream_attempts = attempts.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let attempts = upstream_attempts.clone();
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                    (
                        axum::http::StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(json!({"error":"busy"})),
                    )
                } else {
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(json!({"id":"retry-response","status":"completed","output":[]})),
                    )
                }
            }
        }),
    );
    let policy = ExecutionPolicy {
        send_retries: 0,
        status_retries: 2,
        status_retry_base_delay: Duration::from_millis(30),
        status_retry_max_delay: Duration::from_millis(40),
        ..Default::default()
    };
    let (handle, host, task, path) = start(app, policy).await;
    let started = std::time::Instant::now();
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(started.elapsed() >= Duration::from_millis(65));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    shutdown(handle, task, path).await;
    let reports = host.reports.lock().unwrap();
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.status.unwrap())
            .collect::<Vec<_>>(),
        vec![503, 503, 200]
    );
}

#[tokio::test]
async fn native_transport_stream_open_retries_work_without_network_retry_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let upstream_attempts = attempts.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move || {
            let attempts = upstream_attempts.clone();
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(180)).await;
                }
                ([("content-type", "text/event-stream")], completed())
            }
        }),
    );
    let policy = ExecutionPolicy {
        send_retries: 0,
        open_timeout: Duration::from_millis(60),
        stream_open_max_attempts: 2,
        ..Default::default()
    };
    let (handle, host, task, path) = start(app, policy).await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("response.completed"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    shutdown(handle, task, path).await;
    let reports = host.reports.lock().unwrap();
    let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
    assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.status.unwrap())
            .collect::<Vec<_>>(),
        vec![504, 200]
    );
}

struct FailoverHost {
    inner: PolicyHost,
    alternate_url: String,
    selections: Mutex<Vec<RouteRequest>>,
}
impl Host for FailoverHost {
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(key)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            self.selections.lock().unwrap().push(request.clone());
            let alternate = request.preferred_account_id.as_deref() == Some("b")
                || request
                    .excluded_account_ids
                    .iter()
                    .any(|account| account == "a");
            let mut route = self.inner.route(caller, request).await?;
            if alternate {
                route.account_id = "b".into();
                route.owner_id = "b".into();
                route.base_url = self.alternate_url.clone();
            }
            Ok(route)
        })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
}

#[tokio::test]
async fn native_transport_exhausted_stream_open_budget_can_fail_over_but_never_replay_opaque_state()
{
    let primary_count = Arc::new(AtomicUsize::new(0));
    let primary_attempts = primary_count.clone();
    let primary = Router::new().route(
        "/v1/responses",
        post(move || {
            let count = primary_attempts.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(300)).await;
                ([("content-type", "text/event-stream")], completed())
            }
        }),
    );
    let alternate_count = Arc::new(AtomicUsize::new(0));
    let alternate_attempts = alternate_count.clone();
    let alternate = Router::new().route(
        "/v1/responses",
        post(move || {
            let count = alternate_attempts.clone();
            async move {
                if count.fetch_add(1, Ordering::SeqCst) > 0 {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                ([("content-type", "text/event-stream")], completed())
            }
        }),
    );
    let primary_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let alternate_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let primary_url = format!("http://{}/v1", primary_listener.local_addr().unwrap());
    let alternate_url = format!("http://{}/v1", alternate_listener.local_addr().unwrap());
    let primary_task = tokio::spawn(async {
        axum::serve(primary_listener, primary).await.unwrap();
    });
    let alternate_task = tokio::spawn(async {
        axum::serve(alternate_listener, alternate).await.unwrap();
    });
    let host = Arc::new(FailoverHost {
        inner: PolicyHost {
            url: primary_url,
            chatgpt: false,
            policy: ExecutionPolicy {
                open_timeout: Duration::from_millis(60),
                stream_open_max_attempts: 2,
                send_retries: 3,
                credential_retries: 2,
                ..Default::default()
            },
            reports: Mutex::new(Vec::new()),
            record_delay: Duration::ZERO,
        },
        alternate_url,
        selections: Mutex::new(Vec::new()),
    });
    let path =
        std::env::temp_dir().join(format!("native-failover-{}.sqlite", uuid::Uuid::now_v7()));
    let mut handle = native::start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        path.clone(),
        native::Options::default(),
    )
    .await
    .unwrap();
    let url = format!("http://{}/v1/responses", handle.addr);
    let response = client()
        .post(&url)
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("response.completed"));
    assert_eq!(primary_count.load(Ordering::SeqCst), 2);
    assert_eq!(alternate_count.load(Ordering::SeqCst), 1);
    let response = client()
        .post(&url)
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"previous_response_id":"r","input":"follow up"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("upstream_connect_timeout"));
    assert_eq!(
        primary_count.load(Ordering::SeqCst),
        2,
        "opaque continuation must never return to another owner"
    );
    assert_eq!(
        alternate_count.load(Ordering::SeqCst),
        3,
        "the continuation only gets its owner's two opening attempts"
    );
    assert_eq!(host.selections.lock().unwrap().len(), 3);
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    primary_task.abort();
    alternate_task.abort();
    let _ = std::fs::remove_file(path);
    let reports = host.inner.reports.lock().unwrap();
    assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 3);
    assert_eq!(reports[1].diagnostics.as_ref().unwrap().attempts.len(), 2);
}
