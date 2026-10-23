//! Only the official, provably pre-business handshake may outlive MAX_HOLD.
use super::*;
use futures_util::StreamExt;

struct GateHost {
    inner: FallbackHost,
    order: Arc<Mutex<Vec<String>>>,
    idle: Duration,
}
impl Host for GateHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(bearer)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        self.inner.route(caller, request)
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(report)
    }
    fn capability_fallback_decision<'a>(
        &'a self,
        caller: &'a Caller,
        route: &'a Route,
        request: &'a RouteRequest,
    ) -> BoxFuture<'a, Result<CapabilityFallbackDecision, ServiceError>> {
        self.inner
            .capability_fallback_decision(caller, route, request)
    }
    fn observe_official_failure<'a>(
        &'a self,
        route: &'a Route,
        model: &'a str,
        failure: &'a OfficialFallbackFailure,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.inner
                .observe_official_failure(route, model, failure)
                .await;
            self.order.lock().unwrap().push("cooling".into());
        })
    }
    fn fallback_to_local<'a>(
        &'a self,
        caller: &'a Caller,
        failed: &'a Route,
        request: RouteRequest,
        failure: OfficialFallbackFailure,
    ) -> BoxFuture<'a, Result<Option<Route>, ServiceError>> {
        Box::pin(async move {
            assert!(
                !self.inner.official_failures.lock().unwrap().is_empty(),
                "health must be recorded before local replay"
            );
            self.order.lock().unwrap().push("local_candidate".into());
            self.inner
                .fallback_to_local(caller, failed, request, failure)
                .await
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                idle_timeout: self.idle,
                total_timeout: Duration::from_secs(3),
                status_retries: 0,
                bootstrap_retries: 2,
                credential_retries: 3,
                ..Default::default()
            })
        })
    }
}

struct GateFixture {
    host: Arc<GateHost>,
    handle: Handle,
    order: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
    _root: tempfile::TempDir,
}
impl GateFixture {
    async fn new(prefix: String, local_failure: bool, idle: Duration, generic: bool) -> Self {
        let order = Arc::new(Mutex::new(Vec::new()));
        let wire_order = order.clone();
        let app=Router::new().fallback(move |OriginalUri(uri):OriginalUri| {
            let prefix=prefix.clone(); let order=wire_order.clone();
            async move {
                let path=uri.path().to_owned(); order.lock().unwrap().push(path.clone());
                if path.starts_with("/official/") || generic && path.starts_with("/local/") {
                    let tail=json!({"type":"response.failed","response":{"status":"failed","output":[],"usage":{"output_tokens":0},"error":{"code":"subscription_sharing_usage_limit_exceeded","message":"Synthetic sharing quota exhausted"}},"retry_after":17});
                    let stream=futures_util::stream::iter(vec![
                        (Duration::ZERO,prefix),
                        (Duration::from_millis(400),format!("data: {tail}\n\n")),
                    ]).then(|(delay,data)| async move {
                        tokio::time::sleep(delay).await;
                        Ok::<_,std::io::Error>(axum::body::Bytes::from(data))
                    });
                    return Response::builder().header("content-type","text/event-stream").body(Body::from_stream(stream)).unwrap();
                }
                if local_failure && path.starts_with("/local/") {
                    return Response::builder().status(429).header("content-type","application/json")
                        .body(Body::from(json!({"error":{"code":"usage_limit_reached","message":"Synthetic local quota exhausted"}}).to_string())).unwrap();
                }
                Response::builder().header("content-type","text/event-stream").body(Body::from(COMPLETED)).unwrap()
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let inner = FallbackHost {
            base: format!("http://{}", listener.local_addr().unwrap()),
            fallback_calls: AtomicUsize::new(0),
            foreign_fallback: AtomicBool::new(false),
            fallback_disabled: AtomicBool::new(false),
            connect_failure: AtomicBool::new(false),
            local_connect_failure: AtomicBool::new(false),
            local_preflight_error: AtomicUsize::new(0),
            pool_disabled: AtomicBool::new(false),
            official_header_delay: Arc::new(AtomicBool::new(false)),
            direct_local: AtomicBool::new(generic),
            local_oauth: AtomicBool::new(false),
            local_unauthorized_once: Arc::new(AtomicBool::new(false)),
            refresh_authorization_changed: AtomicBool::new(false),
            refresh_requests: Mutex::new(vec![]),
            official_failures: Mutex::new(vec![]),
            credential_limit: AtomicUsize::new(3),
            reports: Mutex::new(vec![]),
            requests: Mutex::new(vec![]),
        };
        let host = Arc::new(GateHost {
            inner,
            order: order.clone(),
            idle,
        });
        let task = tokio::spawn(async {
            axum::serve(listener, app).await.unwrap();
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
            host,
            handle,
            order,
            task,
            _root: root,
        }
    }
    async fn send(&self, stream: bool) -> reqwest::Response {
        let mut body = request();
        body["stream"] = json!(stream);
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", self.handle.addr))
            .bearer_auth("fixture")
            .json(&body)
            .send()
            .await
            .unwrap()
    }
    async fn close(mut self) -> Vec<RequestReport> {
        self.handle.shutdown(Duration::from_secs(3)).await.unwrap();
        self.task.abort();
        let reports = self.host.inner.reports.lock().unwrap().clone();
        reports
    }
}

fn handshake_prefix() -> String {
    format!(
        ": header comment\n\ndata: {}\n\ndata: {}\n\ndata: {}\n\n",
        json!({"type":"response.created","sequence_number":0,"response":{
            "id":"synthetic","object":"response","created_at":0,"status":"in_progress","background":false,
            "error":null,"incomplete_details":null,"instructions":"Synthetic fixture instructions",
            "max_output_tokens":null,"max_tool_calls":null,"model":"fixture-model","output":[],
            "parallel_tool_calls":true,"previous_response_id":null,"reasoning":{"effort":"medium","summary":null},
            "service_tier":"default","store":true,"temperature":1.0,
            "text":{"format":{"type":"text"},"verbosity":"medium"},"audio":null,"image":null,
            "tool_choice":"auto","tools":[{"type":"function","name":"read_file","description":"Synthetic tool",
                "parameters":{"type":"object","properties":{"content":{"type":"string"},"text":{"type":"string"},"arguments":{"type":"string"}},"required":[],"additionalProperties":false},"strict":false}],
            "top_p":1.0,"truncation":"disabled","usage":null,"user":null,"metadata":{}
        }}),
        json!({"type":"response.in_progress","response":{"output":[],"usage":{"output_tokens":0}}}),
        json!({"type":"response.metadata","metadata":{"synthetic":true}})
    )
}

#[tokio::test]
async fn official_pre_business_gate_waits_past_hold_for_quota_then_cools_and_falls_back() {
    for local_failure in [false, true] {
        let fixture = GateFixture::new(
            handshake_prefix(),
            local_failure,
            Duration::from_secs(1),
            false,
        )
        .await;
        let response = fixture.send(false).await;
        assert_eq!(response.status(), 200);
        assert!(response
            .json::<Value>()
            .await
            .unwrap()
            .get("error")
            .is_none());
        let mut expected = vec![
            "/official/responses",
            "cooling",
            "local_candidate",
            "/local/responses",
        ];
        if local_failure {
            expected.push("/backup/responses");
        }
        assert_eq!(*fixture.order.lock().unwrap(), expected);
        let reports = fixture.close().await;
        assert_eq!(reports.len(), 1);
        let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
        assert_eq!(attempts.len(), if local_failure { 3 } else { 2 });
        assert_eq!(attempts[0].status, Some(429));
        assert!(attempts.last().unwrap().success);
    }
}

#[tokio::test]
async fn official_pre_business_gate_never_replays_after_text_tools_or_output_usage() {
    for business in [
        json!({"type":"response.output_text.delta","delta":"already generated"}),
        json!({"type":"response.reasoning_summary_text.delta","delta":"already reasoning"}),
        json!({"type":"provider.unknown_execution_event"}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","id":"synthetic-call","call_id":"call-one","name":"read_file","arguments":"{}"}}),
        json!({"type":"response.in_progress","response":{"output":[],"usage":{"output_tokens":1}}}),
        json!({"type":"response.metadata","delta":"already generated"}),
        json!({"type":"response.metadata","response":{"content":[]}}),
    ] {
        let prefix = format!("{}data: {business}\n\n", handshake_prefix());
        let fixture = GateFixture::new(prefix, false, Duration::from_secs(1), false).await;
        let response = fixture.send(true).await;
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        assert!(
            fixture
                .order
                .lock()
                .unwrap()
                .iter()
                .all(|stage| stage == "/official/responses" || stage == "cooling"),
            "business output forbids replay even when its later quota is observed"
        );
        let reports = fixture.close().await;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 1);
    }
}

#[tokio::test]
async fn official_pre_business_gate_timeout_does_not_replay_unknown_execution() {
    let fixture =
        GateFixture::new(handshake_prefix(), false, Duration::from_millis(100), false).await;
    let response = fixture.send(false).await;
    assert_eq!(response.status(), 504);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "upstream_idle_timeout"
    );
    assert_eq!(*fixture.order.lock().unwrap(), vec!["/official/responses"]);
    assert!(fixture
        .host
        .inner
        .official_failures
        .lock()
        .unwrap()
        .is_empty());
    let reports = fixture.close().await;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 1);
}

#[tokio::test]
async fn official_gate_limits_stop_replay_but_generic_visibility_timer_does_not() {
    let events = (0..super::super::bootstrap::MAX_EVENTS)
        .map(|_| format!("data: {}\n\n", json!({"type":"response.in_progress"})))
        .collect::<String>();
    let bytes = format!(": {}\n\n", "x".repeat(super::super::bootstrap::MAX_BYTES));
    for (prefix, generic) in [(events, false), (bytes, false), (handshake_prefix(), true)] {
        let fixture = GateFixture::new(prefix, false, Duration::from_secs(1), generic).await;
        let response = fixture.send(true).await;
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        let stages = fixture.order.lock().unwrap().clone();
        assert_eq!(
            stages.iter().filter(|stage| stage.starts_with('/')).count(),
            if generic { 2 } else { 1 },
            "metadata bounds stop replay; a visibility timer alone does not"
        );
        assert!(!stages.iter().any(|stage| stage == "local_candidate"));
        if generic {
            assert!(fixture
                .host
                .inner
                .official_failures
                .lock()
                .unwrap()
                .is_empty());
        }
        let reports = fixture.close().await;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), if generic {2} else {1});
    }
}
