//! Real HTTP admission/retry and final reporting, with payload logging disabled.
use super::*;

struct FaultHost {
    inner: CooldownHost,
    chat: bool,
    reports: Mutex<Vec<RequestReport>>,
}
impl Host for FaultHost {
    fn authorize<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(key)
    }
    fn route<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut route = self.inner.route(caller, request).await?;
            if self.chat {
                route.chatgpt = false;
                route.wire_api = WireApi::Chat;
            }
            Ok(route)
        })
    }
    fn models<'a>(&'a self, caller: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(caller)
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                credential_retries: 2,
                status_retries: 2,
                status_retry_base_delay: Duration::from_millis(1),
                total_timeout: Duration::from_secs(3),
                ..Default::default()
            })
        })
    }
    fn route_failed<'a>(
        &'a self,
        account: &'a str,
        status: u16,
        wait: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_failed(account, status, wait)
    }
    fn record(&self, report: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reports.lock().unwrap().push(report);
        })
    }
    fn retry_cooldown_wait<'a>(
        &'a self,
        caller: &'a Caller,
        request: RouteRequest,
        maximum: Duration,
    ) -> BoxFuture<'a, Result<Option<Duration>, ServiceError>> {
        self.inner.retry_cooldown_wait(caller, request, maximum)
    }
}

#[tokio::test]
async fn request_fault_http_and_native_chat_keep_body_without_refresh_rotation_or_cooling() {
    for chat in [false, true] {
        for (status,raw) in [
            (403,r#"{"error":{"code":"cyber_policy","type":"invalid_request_error","message":"Rejected prompt"}}"#),
            (400,r#"{"error":{"code":"invalid_prompt","type":"invalid_request_error","message":"Invalid prompt: your prompt was flagged as potentially violating our usage policy."}}"#),
            (401,r#"{"error":{"code":"invalid_request_error","message":"Malformed request"}}"#),
            (503,r#"{"error":{"type":"invalid_request_error","message":"Cannot represent request"}}"#),
            (404,"Item with id fixture-item not found. Items are not persisted when `store` is set to false."),
        ] {
            let operation=if chat{"chat/completions"}else{"responses"};
            let app=Router::new().route(&format!("/a/{operation}"),post(move||async move{(StatusCode::from_u16(status).unwrap(),raw)}))
                .route(&format!("/b/{operation}"),post(||async{Json(json!({"id":"unexpected","status":"completed","output":[],"choices":[{"message":{"role":"assistant","content":"unexpected"}}]}))}));
            let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let addr=listener.local_addr().unwrap();
            let upstream=tokio::spawn(async move{axum::serve(listener,app).await.unwrap();});
            let host=Arc::new(FaultHost{inner:CooldownHost{bases:[format!("http://{addr}/a"),format!("http://{addr}/b")],failures:Mutex::new(vec![]),selections:Mutex::new(vec![]),round_wait:Some(Duration::ZERO),credential_retries:2,oauth:true,fail_refresh:true},chat,reports:Mutex::new(vec![])});
            let root=tempfile::tempdir().unwrap();
            let mut service=start("127.0.0.1:0".parse().unwrap(),host.clone(),root.path().join("identity.sqlite"),Options::default()).await.unwrap();
            let request=if chat{json!({"model":"model","messages":[{"role":"user","content":"raw prompt"}]})}else{json!({"model":"model","input":[]})};
            let response=reqwest::Client::builder().no_proxy().build().unwrap().post(format!("http://{}/v1/{operation}",service.addr)).bearer_auth("fixture").json(&request).send().await.unwrap();
            assert_eq!(response.status().as_u16(),status,"chat={chat}");
            assert_eq!(response.text().await.unwrap(),raw);
            service.shutdown(Duration::from_secs(2)).await.unwrap();
            assert_eq!(host.inner.selections.lock().unwrap().len(),1,"request fault must not refresh or rotate");
            assert!(host.inner.failures.lock().unwrap().is_empty());
            let reports=host.reports.lock().unwrap();assert_eq!(reports.len(),1);
            assert_eq!(reports[0].status,status);
            assert!(reports[0].diagnostics.as_ref().unwrap().upstream_request_scoped);
            assert!(!reports[0].credential_scope);
            let serialized=serde_json::to_value(reports[0].diagnostics.as_ref().unwrap()).unwrap();
            assert!(serialized.get("upstreamRequestScoped").is_none());
            drop(reports);upstream.abort();
        }
    }
}

#[tokio::test]
async fn generic_responses_bad_request_records_only_history_shape_and_provider_trace() {
    let received = Arc::new(Mutex::new(None::<Value>));
    let observed = received.clone();
    let app = Router::new().route("/a/responses", post(move |Json(body): Json<Value>| {
        let observed = observed.clone();
        async move {
            *observed.lock().unwrap() = Some(body);
            (
                StatusCode::BAD_REQUEST,
                [("x-request-id", "req_history-fixture")],
                Json(json!({"detail":"Bad Request"})),
            )
        }
    }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let host = Arc::new(FaultHost {
        inner: CooldownHost {
            bases: [format!("http://{addr}/a"), format!("http://{addr}/b")],
            failures: Mutex::new(vec![]),
            selections: Mutex::new(vec![]),
            round_wait: Some(Duration::ZERO),
            credential_retries: 2,
            oauth: true,
            fail_refresh: true,
        },
        chat: false,
        reports: Mutex::new(vec![]),
    });
    let root = tempfile::tempdir().unwrap();
    let mut service = start(
        "127.0.0.1:0".parse().unwrap(), host.clone(), root.path().join("identity.sqlite"),
        Options::default(),
    ).await.unwrap();
    let mut history = vec![json!({"role":"user","content":"private-long-history".repeat(4000)})];
    history.extend([
        json!({"type":"function_call","name":"fixture","call_id":"private-call-id","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"private-call-id","output":"private-tool-output"}),
        json!({"role":"user","content":"continue"}),
    ]);
    let response = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(format!("http://{}/v1/responses", service.addr))
        .bearer_auth("fixture").json(&json!({"model":"model","input":history}))
        .send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.json::<Value>().await.unwrap(), json!({"detail":"Bad Request"}));
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    assert_eq!(host.inner.selections.lock().unwrap().len(), 1);
    let reports = host.reports.lock().unwrap();
    let diagnostic = reports[0].diagnostics.as_ref().unwrap();
    assert!(diagnostic.payloads.is_empty());
    assert_eq!(diagnostic.attempts.len(), 1);
    let attempt = &diagnostic.attempts[0];
    assert_eq!(attempt.upstream_request_id.as_deref(), Some("req_history-fixture"));
    let rejection = attempt.responses_history_rejection.as_ref().unwrap();
    assert_eq!(rejection.client.input_items, 4);
    assert_eq!(rejection.client.messages, 2);
    assert_eq!(rejection.client.tool_calls, 1);
    assert_eq!(rejection.client.tool_outputs, 1);
    assert_eq!(rejection.upstream.input_items, received.lock().unwrap().as_ref().unwrap()["input"].as_array().unwrap().len());
    assert!(!rejection.compaction_request);
    let json = serde_json::to_string(diagnostic).unwrap();
    assert!(!json.contains("private-"));
    upstream.abort();
}
