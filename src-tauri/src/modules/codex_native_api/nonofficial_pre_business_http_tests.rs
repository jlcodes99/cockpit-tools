//! Non-official Responses recover before execution across metadata visibility.
use super::*;
use futures_util::StreamExt;

struct MatrixHost {
    inner: FixtureHost,
    oauth: bool,
}
impl Host for MatrixHost {
    fn authorize<'a>(&'a self, b: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        self.inner.authorize(b)
    }
    fn models<'a>(&'a self, c: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        self.inner.models(c)
    }
    fn route<'a>(
        &'a self,
        c: &'a Caller,
        r: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let mut route = self.inner.route(c, r).await?;
            route.chatgpt = self.oauth && route.account_id == "0";
            assert!(
                route.official_registration_id.is_none()
                    && route.official_fallback_registration_id.is_none()
            );
            Ok(route)
        })
    }
    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        self.inner.execution_policy()
    }
    fn route_failed<'a>(
        &'a self,
        id: &'a str,
        status: u16,
        retry: Option<Duration>,
    ) -> BoxFuture<'a, ()> {
        self.inner.route_failed(id, status, retry)
    }
    fn record(&self, r: RequestReport) -> BoxFuture<'_, ()> {
        self.inner.record(r)
    }
}

#[tokio::test]
async fn nonofficial_http_quota_recovery_for_oauth_and_api_key_long_history() {
    let cases = [
        ("http429", 0, "", false, true),
        ("immediate_usage", 0, "", false, true),
        ("delayed_usage", 400, "", false, true),
        (
            "delayed_codex_metadata",
            400,
            "codex.response.metadata",
            false,
            true,
        ),
        ("response_metadata", 0, "response.metadata", false, true),
        ("sharing_quota", 400, "response.metadata", false, true),
        ("json_backup", 400, "response.metadata", false, true),
        ("tool_then_quota", 400, "", true, false),
        ("reasoning_then_quota", 400, "", true, false),
        (
            "unknown_metadata",
            400,
            "response.unknown_metadata",
            false,
            false,
        ),
        ("terminal_output", 0, "", false, false),
        ("terminal_tool", 0, "", false, false),
        ("terminal_usage", 0, "", false, false),
        ("terminal_reasoning_usage", 0, "", false, false),
        ("output_then_quota", 400, "", true, false),
    ];
    for oauth in [false, true] {
        for streaming in [false, true] {
            for (name, delay, metadata, output, want_rotate) in cases {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let host = Arc::new(MatrixHost {
                    oauth,
                    inner: FixtureHost {
                        base: format!("http://{}", listener.local_addr().unwrap()),
                        policy: ExecutionPolicy {
                            credential_retries: 3,
                            status_retries: 0,
                            bootstrap_retries: 0,
                            total_timeout: Duration::from_secs(4),
                            idle_timeout: Duration::from_secs(2),
                            ..Default::default()
                        },
                        failing: 1,
                        before_error: vec![],
                        recover_on_same: false,
                        delayed_opening: 0,
                        release_error: tokio::sync::Notify::new(),
                        calls: Default::default(),
                        requests: Mutex::new(vec![]),
                        failures: Mutex::new(vec![]),
                        reports: Mutex::new(vec![]),
                    },
                });
                let upstream_host = host.clone();
                let app=Router::new().fallback(move |uri:axum::extract::OriginalUri, headers:HeaderMap, axum::Json(body):axum::Json<Value>| {
            let h=upstream_host.clone(); async move {
                let index=if uri.0.path().starts_with("/0/") {0} else {1};
                h.inner.calls[index].fetch_add(1,Ordering::SeqCst);
                h.inner.requests.lock().unwrap().push((index,headers,body));
                if index==1 && name=="json_backup" { return Response::builder().header("content-type","application/json")
                    .body(Body::from(json!({"id":"backup-json","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Synthetic recovery success"}]}],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}).to_string())).unwrap(); }
                if index==1 { return Response::builder().header("content-type","text/event-stream").body(Body::from(format!("data: {}\n\ndata: {}\n\n",json!({"type":"response.created","response":{"id":"backup-created","status":"in_progress"}}),completed()))).unwrap(); }
                let code=if name=="sharing_quota" {"subscription_sharing_usage_limit_exceeded"} else {"usage_limit_reached"};
                let error=json!({"code":code,"message":"Synthetic quota exhausted","resets_in_seconds":60});
                if name=="http429" {return Response::builder().status(429).header("content-type","application/json").body(Body::from(json!({"error":error}).to_string())).unwrap();}
                let mut prefix=format!("data: {}\n\ndata: {}\n\n",created(),json!({"type":"response.in_progress","response":{"output":[],"usage":null}}));
                if !metadata.is_empty() {prefix.push_str(&format!("data: {}\n\n",json!({"type":metadata,"metadata":{"fixture":true}})));}
                if output {prefix.push_str(&format!("data: {}\n\n",json!({"type":match name {"tool_then_quota"=>"response.function_call_arguments.delta","reasoning_then_quota"=>"response.reasoning_text.delta",_=>"response.output_text.delta"},"delta":"already generated"})));}
                let mut failed=json!({"type":"response.failed","response":{"id":"temporary","status":"failed","output":[],"usage":{"output_tokens":0},"error":error}});
                match name {
                    "terminal_output" => failed["response"]["output"]=json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"already generated"}]}]),
                    "terminal_tool" => failed["response"]["tool_calls"]=json!([{"id":"tool","function":{"name":"write_file","arguments":"{}"}}]),
                    "terminal_usage" => failed["response"]["usage"]["output_tokens"]=json!(1),
                    "terminal_reasoning_usage" => failed["response"]["usage"]["output_tokens_details"]=json!({"reasoning_tokens":1}),
                    _=>{},
                }
                let chunks=futures_util::stream::iter(vec![(0,prefix),(delay,format!("data: {failed}\n\n"))]).then(|(ms,s)| async move {tokio::time::sleep(Duration::from_millis(ms)).await;Ok::<_,std::io::Error>(Bytes::from(s))});
                Response::builder().header("content-type","text/event-stream").body(Body::from_stream(chunks)).unwrap()
            }
        });
                let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
                let temp = tempfile::tempdir().unwrap();
                let mut handle = start(
                    "127.0.0.1:0".parse().unwrap(),
                    host.clone(),
                    temp.path().join("identity.sqlite"),
                    Options::default(),
                )
                .await
                .unwrap();
                let mut input = vec![];
                for i in 0..80 {
                    input.push(json!({"role":"user","content":format!("Synthetic context {i}")}));
                    input.push(json!({"role":"assistant","content":format!("Recorded {i}")}));
                    if i % 8 == 0 {
                        input.push(json!({"type":"function_call","name":"read_file","call_id":format!("call_{i}"),"arguments":"{}"}));
                        input.push(json!({"type":"function_call_output","call_id":format!("call_{i}"),"output":"Synthetic complete result"}));
                    }
                }
                input.push(json!({"role":"user","content":"Continue safely"}));
                assert_eq!(input.len(), 181);
                let response = reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .unwrap()
                    .post(format!("http://{}/v1/responses", handle.addr))
                    .bearer_auth("fixture")
                    .json(&json!({"model":"model","stream":streaming,"input":input}))
                    .send()
                    .await
                    .unwrap();
                let status = response.status().as_u16();
                let body = response.text().await.unwrap();
                handle.shutdown(Duration::from_secs(2)).await.unwrap();
                server.abort();
                let a = host.inner.calls[0].load(Ordering::SeqCst);
                let b = host.inner.calls[1].load(Ordering::SeqCst);
                let reports = host.inner.reports.lock().unwrap();
                assert_eq!(reports.len(), 1);
                let report = &reports[0];
                let requests = host.inner.requests.lock().unwrap();
                assert_eq!(requests.len(), 1 + usize::from(want_rotate));
                for (_, _, body) in requests.iter() {
                    let items = body["input"]
                        .as_array()
                        .expect("history survives both routes");
                    assert_eq!(items.len(), 181, "{name}: complete history required");
                    assert_eq!(
                        items
                            .iter()
                            .filter(|v| v["type"] == "function_call")
                            .count(),
                        10
                    );
                    assert_eq!(
                        items
                            .iter()
                            .filter(|v| v["type"] == "function_call_output")
                            .count(),
                        10
                    );
                    for call in items.iter().filter(|v| v["type"] == "function_call") {
                        assert!(items.iter().any(|v| v["type"] == "function_call_output"
                            && v["call_id"] == call["call_id"]));
                    }
                }
                let hooks = host.inner.failures.lock().unwrap().clone();
                println!(
                    "QUOTA_RECOVERY_RUST {}",
                    json!({"oauth":oauth,"stream":streaming,"case":name,"http_status":status,"actual_a":a,"actual_b":b,"success":report.success,"report_status":report.status,"report_error":report.error,"retry_after_ms":report.retry_after.map(|v|v.as_millis()),"route_failed_hooks":hooks,"attempts":report.diagnostics.as_ref().unwrap().attempts.len(),"official_binding":false})
                );
                assert_eq!(a, 1, "{name}");
                assert_eq!(
                    b,
                    usize::from(want_rotate),
                    "{name} oauth={oauth} stream={streaming}: {body}"
                );
                assert_eq!(report.success, want_rotate, "{name}: {body}");
                if want_rotate {
                    assert!(!hooks.is_empty());
                } else {
                    assert!(body.contains("usage_limit_reached"), "{body}");
                }
            }
        }
    }
}

#[tokio::test]
async fn nonofficial_http_late_quota_does_not_restart_credential_budget() {
    for streaming in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let inner = FixtureHost {
            base: format!("http://{}", listener.local_addr().unwrap()),
            policy: ExecutionPolicy {
                credential_retries: 2,
                status_retries: 0,
                bootstrap_retries: 0,
                total_timeout: Duration::from_secs(4),
                ..Default::default()
            },
            failing: 2,
            before_error: vec![],
            recover_on_same: false,
            delayed_opening: 0,
            release_error: tokio::sync::Notify::new(),
            calls: Default::default(),
            requests: Mutex::new(vec![]),
            failures: Mutex::new(vec![]),
            reports: Mutex::new(vec![]),
        };
        let host = Arc::new(MatrixHost { inner, oauth: true });
        let upstream_host = host.clone();
        let app=Router::new().fallback(move |uri:axum::extract::OriginalUri| {
            let host=upstream_host.clone(); async move {
                let index=uri.0.path().split('/').nth(1).unwrap().parse::<usize>().unwrap();
                host.inner.calls[index].fetch_add(1,Ordering::SeqCst);
                let created=json!({"type":"response.created","response":{"id":format!("account-{index}-created"),"status":"in_progress"}});
                let quota=json!({"type":"response.failed","response":{"id":format!("account-{index}-created"),"status":"failed","output":[],"usage":{"output_tokens":0},"error":{"code":"usage_limit_reached","message":"Synthetic exhausted quota","resets_in_seconds":60}}});
                let stream=futures_util::stream::iter(vec![(0,created),(400,quota)]).then(|(delay,event)|async move{
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    Ok::<_,std::io::Error>(Bytes::from(format!("data: {event}\n\n")))
                });
                Response::builder().header("content-type","text/event-stream").body(Body::from_stream(stream)).unwrap()
            }
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let temp = tempfile::tempdir().unwrap();
        let mut handle = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            temp.path().join("history.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("fixture")
            .json(&json!({"model":"model","input":[],"stream":streaming}))
            .send()
            .await
            .unwrap();
        let body = response.text().await.unwrap();
        assert!(body.contains("usage_limit_reached"), "{body}");
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        server.abort();
        assert_eq!(
            host.inner
                .calls
                .iter()
                .map(|c| c.load(Ordering::SeqCst))
                .collect::<Vec<_>>(),
            vec![1, 1, 0],
            "only two actual credentials are allowed"
        );
        let reports = host.inner.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, 429);
        assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 2);
        assert_eq!(
            host.inner.failures.lock().unwrap().len(),
            2,
            "both actual quota rejections update health"
        );
    }
}
