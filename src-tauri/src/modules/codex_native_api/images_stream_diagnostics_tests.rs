//! Loopback-only Images read failures and the bounded pre-output replay fence.
use super::*;
use crate::modules::codex_native_api::{
    contract::{
        ExecutionPolicy, Host, ImageRouteRequest, RequestReport, Route, RouteRequest, WireApi,
    },
    start, Handle, Options,
};
use axum::{extract::State, routing::post, Json, Router};
use futures_util::future::BoxFuture;
use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};

struct ImageReadHost {
    base_url: String,
    policy: ExecutionPolicy,
    selections: AtomicUsize,
    capacity: Arc<tokio::sync::Semaphore>,
    reports: Mutex<Vec<RequestReport>>,
}

impl Host for ImageReadHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            assert_eq!(key, "image-read-fixture");
            Ok(Caller {
                id: "image-read-caller".into(),
                label: "fixture".into(),
            })
        })
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }

    fn route<'a>(
        &'a self,
        _: &'a Caller,
        _: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async {
            Err(ServiceError::new(
                500,
                "unexpected_route",
                "Images use their own route",
            ))
        })
    }

    fn image_route<'a>(
        &'a self,
        _: &'a Caller,
        _: ImageRouteRequest,
    ) -> BoxFuture<'a, Result<ImageRoute, ServiceError>> {
        Box::pin(async move {
            self.selections.fetch_add(1, Ordering::SeqCst);
            let permit = self.capacity.clone().try_acquire_owned().unwrap();
            Ok(ImageRoute {
                route: Route {
                    account_id: "image-read-account".into(),
                    owner_id: "image-read-owner".into(),
                    continuity_account_id: None,
                    installation_profile: None,
                    official_authorization_revision: None,
                    official_registration_id: None,
                    official_fallback_registration_id: None,
                    official_fallback_authorization_revision: None,
                    authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                    transport_identity: None,
                    base_url: self.base_url.clone(),
                    model: "image-main-fixture".into(),
                    headers: HeaderMap::new(),
                    client: reqwest::Client::builder().no_proxy().build().unwrap(),
                    proxy_url: None,
                    proxy_route: None,
                    chatgpt: true,
                    chatgpt_plan: false,
                    wire_api: WireApi::Responses,
                    compact_endpoint: None,
                    websocket_endpoint: None,
                    default_service_tier: None,
                    permit: Some(Arc::new(permit)),
                },
                image_model: "gpt-image-2.5-flare".into(),
                main_model: Some("image-main-fixture".into()),
            })
        })
    }

    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[]})) })
    }

    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
}

struct ImageReadFixture {
    handle: Handle,
    host: Arc<ImageReadHost>,
    upstream: JoinHandle<()>,
    _root: tempfile::TempDir,
}

impl ImageReadFixture {
    async fn new(app: Router, policy: ExecutionPolicy) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let host = Arc::new(ImageReadHost {
            base_url: format!("http://{addr}/v1"),
            policy,
            selections: AtomicUsize::new(0),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
            reports: Mutex::new(Vec::new()),
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
            _root: root,
        }
    }

    async fn send(&self, streaming: bool) -> reqwest::Response {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .post(format!("http://{}/v1/images/generations", self.handle.addr))
            .bearer_auth("image-read-fixture")
            .json(&json!({"model":"gpt-image-2.5","prompt":"synthetic image","stream":streaming}))
            .send()
            .await
            .unwrap()
    }

    async fn report(&self) -> RequestReport {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(report) = self.host.reports.lock().unwrap().first().cloned() {
                    return report;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    async fn close(mut self) {
        self.handle.shutdown(Duration::from_secs(3)).await.unwrap();
        self.upstream.abort();
        let _ = self.upstream.await;
    }
}

fn policy(retries: u8) -> ExecutionPolicy {
    ExecutionPolicy {
        bootstrap_retries: retries,
        retry_base_delay: Duration::from_millis(1),
        image_idle_timeout: Duration::from_secs(3),
        ..ExecutionPolicy::default()
    }
}

fn frame(event: Value) -> String {
    format!("data: {event}\n\n")
}

fn completed() -> Value {
    json!({"type":"response.completed","response":{"id":"image-read-response","status":"completed",
        "output":[{"type":"image_generation_call","id":"image-read-item","result":"AA=="}]}})
}

fn broken_sse(prefix: String) -> Response {
    let source = stream::once(async move { Ok::<_, io::Error>(Bytes::from(prefix)) }).chain(
        stream::once(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "synthetic upstream reset",
            ))
        }),
    );
    (
        [(header::CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(source),
    )
        .into_response()
}

#[tokio::test]
async fn image_read_failure_keeps_sanitized_cause_and_typed_timeout() {
    for (kind, status, code) in [
        (io::ErrorKind::ConnectionReset, 502, "upstream_stream_error"),
        (io::ErrorKind::TimedOut, 504, "upstream_timeout"),
    ] {
        let source =
            stream::once(async move {
                Err::<Bytes, _>(io::Error::new(kind,
                "synthetic read cause https://private.invalid/path?key=hidden token=image-secret"))
            });
        let response = reqwest::Response::from(axum::http::Response::new(
            reqwest::Body::wrap_stream(source),
        ));
        let error = image_read_failure(response.bytes_stream().next().await.unwrap().unwrap_err());
        assert_eq!((error.status, error.code), (status, code));
        assert!(
            error.message.contains("synthetic read cause"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("[redacted URL]"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("private.invalid"));
        assert!(!error.message.contains("image-secret"));
    }
}

#[test]
fn image_stream_progress_never_records_event_payloads_or_untrusted_types() {
    let mut progress = ImageStreamProgress::new(true);
    progress.chunk(23);
    progress.event(&json!({"type":"response.created"}));
    progress.event(&json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"private-image"}));
    progress.event(&json!({"type":"private-event-token","text":"private-prompt"}));
    let error = progress.failure(ServiceError::new(
        502,
        "upstream_stream_error",
        "synthetic failure",
    ));
    for expected in [
        "stage=image_activity",
        "bytes=23",
        "chunks=1",
        "events=3",
        "handshake_events=1",
        "business_events=2",
        "image_events=1",
        "completion_seen=false",
    ] {
        assert!(error.message.contains(expected), "{}", error.message);
    }
    for secret in ["private-image", "private-event-token", "private-prompt"] {
        assert!(!error.message.contains(secret));
    }
}

#[tokio::test]
async fn image_bootstrap_disconnect_retries_only_within_its_budget_on_the_same_owner() {
    let bodies = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let app = Router::new().route(
        "/v1/responses",
        post({
            let bodies = bodies.clone();
            move |Json(body): Json<Value>| {
                let bodies = bodies.clone();
                async move {
                    let attempt = {
                        let mut bodies = bodies.lock().unwrap();
                        bodies.push(body);
                        bodies.len()
                    };
                    if attempt == 1 {
                        broken_sse(frame(
                            json!({"type":"response.created","response":{"id":"opening-fixture"}}),
                        ))
                    } else {
                        (
                            [(header::CONTENT_TYPE, "text/event-stream")],
                            frame(completed()),
                        )
                            .into_response()
                    }
                }
            }
        }),
    );
    let fixture = ImageReadFixture::new(app, policy(1)).await;
    let response = fixture.send(false).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["data"][0]["b64_json"],
        "AA=="
    );
    assert_eq!(fixture.host.selections.load(Ordering::SeqCst), 1);
    let report = fixture.report().await;
    assert!(report.success);
    assert_eq!(report.account_id, "image-read-account");
    let diagnostics = report.diagnostics.unwrap();
    assert_eq!(diagnostics.attempts.len(), 2);
    assert_eq!(
        diagnostics.attempts[0].error_category.as_deref(),
        Some("upstream_stream_error")
    );
    assert!(diagnostics.attempts[0]
        .error_message
        .as_ref()
        .unwrap()
        .contains("stage=bootstrap"));
    assert!(diagnostics.attempts[1].success);
    let bodies = bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2);
    for body in bodies {
        assert_eq!(body["model"], "image-main-fixture");
        assert_eq!(body["tools"][0]["model"], "gpt-image-2.5-flare");
        assert_eq!(body["input"][0]["content"][0]["text"], "synthetic image");
    }
    assert_eq!(fixture.host.capacity.available_permits(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn image_bootstrap_disconnect_honors_zero_and_exhausted_retry_budgets() {
    for retries in [0, 2] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/v1/responses",
                post(|State(calls): State<Arc<AtomicUsize>>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    broken_sse(frame(json!({"type":"response.created"})))
                }),
            )
            .with_state(calls.clone());
        let fixture = ImageReadFixture::new(app, policy(retries)).await;
        let response = fixture.send(false).await;
        assert_eq!(response.status(), 502);
        let error = response.json::<Value>().await.unwrap();
        assert_eq!(error["error"]["code"], "upstream_stream_error");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stage=bootstrap"));
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(retries) + 1);
        assert_eq!(fixture.host.selections.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.host.capacity.available_permits(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn image_disconnect_after_business_or_image_activity_never_replays() {
    for (event, stage) in [
        (
            json!({"type":"response.output_text.delta","delta":"synthetic output"}),
            "business_output",
        ),
        (
            json!({"type":"response.image_generation_call.in_progress","item_id":"image-read-item"}),
            "image_activity",
        ),
        (
            json!({"type":"response.image_generation_call.partial_image","partial_image_b64":"AA==","partial_image_index":0}),
            "image_activity",
        ),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/v1/responses",
            post({
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    let event = event.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        broken_sse(frame(event))
                    }
                }
            }),
        );
        let fixture = ImageReadFixture::new(app, policy(3)).await;
        let response = fixture.send(false).await;
        assert_eq!(response.status(), 502);
        let error = response.json::<Value>().await.unwrap();
        assert_eq!(error["error"]["code"], "upstream_stream_error");
        let message = error["error"]["message"].as_str().unwrap();
        assert!(message.contains(&format!("stage={stage}")), "{message}");
        assert!(message.contains("events=1"), "{message}");
        assert!(message.contains("completion_seen=false"), "{message}");
        assert_ne!(message, "Image upstream disconnected");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let report = fixture.report().await;
        assert_eq!(report.diagnostics.unwrap().attempts.len(), 1);
        assert!(report
            .error_message
            .unwrap()
            .contains(&format!("stage={stage}")));
        fixture.close().await;
    }
}

#[tokio::test]
async fn image_eof_and_idle_timeout_keep_distinct_codes() {
    for idle in [false, true] {
        let event = frame(json!({"type":"response.output_text.delta","delta":"synthetic output"}));
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let event = event.clone();
                async move {
                    let source =
                        stream::once(async move { Ok::<_, io::Error>(Bytes::from(event)) });
                    let source = if idle {
                        source.chain(stream::pending()).boxed()
                    } else {
                        source.boxed()
                    };
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        Body::from_stream(source),
                    )
                }
            }),
        );
        let mut execution_policy = policy(3);
        execution_policy.image_idle_timeout = Duration::from_millis(30);
        let fixture = ImageReadFixture::new(app, execution_policy).await;
        let response = fixture.send(false).await;
        assert_eq!(response.status().as_u16(), if idle { 504 } else { 502 });
        let error = response.json::<Value>().await.unwrap();
        assert_eq!(
            error["error"]["code"],
            if idle {
                "upstream_idle_timeout"
            } else {
                "incomplete_upstream_stream"
            }
        );
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("stage=business_output"));
        let report = fixture.report().await;
        assert_eq!(report.diagnostics.unwrap().attempts.len(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn image_handshake_hold_releases_stream_and_later_disconnect_does_not_replay() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let app = Router::new().route(
        "/v1/responses",
        post({
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                let wait = wait.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let wait = wait.lock().unwrap().take().expect("one upstream attempt");
                    let source = stream::once(async {
                        Ok::<_, io::Error>(Bytes::from(frame(json!({"type":"response.created"}))))
                    })
                    .chain(stream::once(async move {
                        wait.await.unwrap();
                        Err(io::Error::new(
                            io::ErrorKind::ConnectionReset,
                            "late synthetic disconnect",
                        ))
                    }));
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        Body::from_stream(source),
                    )
                }
            }
        }),
    );
    let fixture = ImageReadFixture::new(app, policy(3)).await;
    // The common 250ms guard must release response headers while the peer is
    // still waiting. Releasing the fault only now makes this test deterministic.
    let response = tokio::time::timeout(Duration::from_secs(2), fixture.send(true))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    release.send(()).unwrap();
    let wire = response.text().await.unwrap();
    assert!(wire.contains("upstream_stream_error"), "{wire}");
    assert!(wire.contains("stage=handshake"), "{wire}");
    assert!(wire.contains("handshake_events=1"), "{wire}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    fixture.close().await;
}
