use super::*;
use std::sync::atomic::AtomicUsize;

struct SlowHost {
    inner: TestHost,
    policy: ExecutionPolicy,
    route_delay: Duration,
}
impl Host for SlowHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(bearer)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            tokio::time::sleep(self.route_delay).await;
            self.inner.route(caller, request).await
        })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async { Ok(self.policy.clone()) })
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
}

async fn slow_fixture(
    app: Router,
    oauth: bool,
    route_delay: Duration,
    total: Duration,
) -> (Handle, Arc<SlowHost>, JoinHandle<()>, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = Arc::new(SlowHost {
        inner: TestHost {
            base: format!("http://{address}/v1"),
            oauth,
            owner_changed: AtomicBool::new(false),
            capacity: Arc::new(tokio::sync::Semaphore::new(1)),
            selections: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
            execution_policy: Mutex::new(ExecutionPolicy::default()),
        },
        policy: ExecutionPolicy {
            total_timeout: total,
            open_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        route_delay,
    });
    let root = std::env::temp_dir().join(format!("native-aux-execution-{}", uuid::Uuid::new_v4()));
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host.clone(),
        root.join("identity.sqlite"),
        Options {
            request_timeout: Duration::from_millis(100),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    (handle, host, upstream, root)
}

fn search(delay: Duration, calls: Arc<AtomicUsize>) -> Router {
    Router::new().route(
        "/v1/alpha/search",
        post(move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                Json(json!({"results":[],"output":"search completed"}))
            }
        }),
    )
}
async fn search_request(handle: &Handle) -> reqwest::Response {
    client()
        .post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("local-a")
        .json(&json!({"model":"client-model","id":"session-original","input":"hello"}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn auxiliary_slow_account_preparation_uses_open_policy_instead_of_body_read_cap() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (handle, host, upstream, root) = slow_fixture(
        search(Duration::ZERO, calls.clone()),
        false,
        Duration::from_millis(250),
        Duration::from_secs(2),
    )
    .await;
    let response = search_request(&handle).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["output"],
        "search completed"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    wait_free(&host.inner).await;
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_total_budget_includes_account_selection_and_upstream_opening() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (handle, host, upstream, root) = slow_fixture(
        search(Duration::from_millis(250), calls.clone()),
        false,
        Duration::from_millis(200),
        Duration::from_millis(350),
    )
    .await;
    let started = Instant::now();
    let response = search_request(&handle).await;
    assert_eq!(response.status(), 504);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "upstream_timeout"
    );
    assert!(started.elapsed() < Duration::from_millis(700));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    wait_free(&host.inner).await;
    assert_eq!(
        host.inner.reports.lock().unwrap().last().unwrap().status,
        504
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_route_exhausting_total_budget_never_sends_upstream() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (handle, host, upstream, root) = slow_fixture(
        search(Duration::ZERO, calls.clone()),
        false,
        Duration::from_millis(600),
        Duration::from_millis(220),
    )
    .await;
    let response = search_request(&handle).await;
    assert_eq!(response.status(), 504);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(host.inner.selections.lock().unwrap().is_empty());
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_downstream_stream_retains_total_deadline_after_preparation_scope_ends() {
    let app = Router::new().route(
        "/v1/alpha/search",
        post(|| async {
            Body::from_stream(stream::unfold(0usize, |next| async move {
                if next == 0 {
                    return Some((
                        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"first")),
                        1,
                    ));
                }
                tokio::time::sleep(Duration::from_millis(90)).await;
                Some((Ok(axum::body::Bytes::from_static(b"continued")), next + 1))
            }))
        }),
    );
    let (handle, host, upstream, root) = slow_fixture(
        app,
        false,
        Duration::from_millis(180),
        Duration::from_millis(370),
    )
    .await;
    let response = search_request(&handle).await;
    assert_eq!(response.status(), 200);
    let started = Instant::now();
    assert!(response.bytes().await.is_err());
    assert!(started.elapsed() < Duration::from_millis(500));
    wait_free(&host.inner).await;
    assert_eq!(
        host.inner.reports.lock().unwrap().last().unwrap().status,
        504
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_audio_budget_keeps_time_spent_preparing_the_account() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let app = Router::new().route(
        "/v1/transcribe",
        post(move || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(250)).await;
                Json(json!({"text":"too late"}))
            }
        }),
    );
    let (handle, host, upstream, root) = slow_fixture(
        app,
        true,
        Duration::from_millis(200),
        Duration::from_millis(350),
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
        .bearer_auth("local-a")
        .multipart(
            reqwest::multipart::Form::new()
                .text("model", "client-model")
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(vec![1, 2, 3]).file_name("audio.wav"),
                ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 504);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    wait_free(&host.inner).await;
    assert_eq!(
        host.inner.reports.lock().unwrap().last().unwrap().status,
        504
    );
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn auxiliary_realtime_websocket_preparation_uses_configured_open_budget() {
    let app = Router::new().route(
        "/v1/realtime",
        get(|ws: WebSocketUpgrade| async {
            ws.on_upgrade(|mut socket| async move {
                socket
                    .send(Message::Text(
                        json!({"type":"session.created"}).to_string().into(),
                    ))
                    .await
                    .unwrap();
                let _ = socket.close().await;
            })
        }),
    );
    let (handle, host, upstream, root) = slow_fixture(
        app,
        false,
        Duration::from_millis(250),
        Duration::from_secs(2),
    )
    .await;
    let mut request = format!("ws://{}/v1/realtime?model=client-model", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-a".parse().unwrap());
    let (mut socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
    let message = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(message, UpstreamMessage::Text(_)));
    let _ = socket.close(None).await;
    drop(socket);
    wait_free(&host.inner).await;
    cleanup(handle, upstream, root).await;
}
