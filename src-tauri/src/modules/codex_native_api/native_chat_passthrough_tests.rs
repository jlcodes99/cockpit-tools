use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn native_chat_mislabeled_multi_choice_sse_keeps_parameters_and_eof_usage_once() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let router = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
        let capture = capture.clone();
        async move {
            capture.lock().unwrap().push(body);
            ([("content-type", "application/octet-stream")], concat!(
                "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"},\"finish_reason\":\"stop\"},{\"index\":1,\"delta\":{\"content\":\"second\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2,\"total_tokens\":9},\"extension\":true}"
            ))
        }
    }));
    let mut fixture = Fixture::new(WireApi::Chat, router).await;
    let response = reqwest::Client::builder().no_proxy().build().unwrap()
        .post(fixture.url("/v1/chat/completions")).bearer_auth("local-key")
        .json(&json!({"model":"public-model","messages":[{"role":"user","content":"hello"}],"stream":true,"n":2,"top_k":5,"stop":["END"],"seed":9,"stream_options":{"include_usage":true}}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let text = response.text().await.unwrap();
    assert_eq!(text.matches("\"content\":\"first\"").count(), 1);
    assert_eq!(text.matches("\"content\":\"second\"").count(), 1);
    assert!(text.contains("\"extension\":true"));
    assert!(text.ends_with("\n\n"));
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["model"], "upstream-model");
    assert_eq!(seen[0]["n"], 2);
    assert_eq!(seen[0]["top_k"], 5);
    assert_eq!(seen[0]["stop"], json!(["END"]));
    assert_eq!(seen[0]["seed"], 9);
    assert_eq!(seen[0]["messages"][0]["content"], "hello");
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 7);
    assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 2);
}

#[tokio::test]
async fn native_chat_done_without_all_requested_choices_never_reports_success() {
    let router = Router::new().route("/v1/chat/completions", post(|| async {
        ([("content-type", "text/event-stream")], concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        ))
    }));
    let mut fixture = Fixture::new(WireApi::Chat, router).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(fixture.url("/v1/chat/completions"))
        .bearer_auth("local-key")
        .json(&json!({"model":"public-model","messages":[],"stream":true,"n":2}))
        .send()
        .await
        .unwrap();
    let body = response.text().await.unwrap();
    assert!(body.contains("first"));
    assert!(body.contains("incomplete_upstream_stream"));
    assert!(!body.contains("[DONE]"));
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    let reports = fixture.host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(!reports[0].success);
    assert_eq!(
        reports[0].error.as_deref(),
        Some("incomplete_upstream_stream")
    );
}

#[tokio::test]
async fn native_chat_json_and_sse_credential_errors_keep_failure_scope_and_details() {
    for streaming in [false, true] {
        let router = Router::new().route("/v1/chat/completions", post(move || async move {
            let payload = "{\"error\":{\"code\":\"invalid_grant\",\"message\":\"fixture expired https://private.invalid/?token=secret Bearer hidden-token\"}}";
            if streaming {
                ([("content-type", "text/event-stream")], format!("data: {payload}\n\n"))
            } else { ([("content-type", "application/json")], payload.to_string()) }
        }));
        let mut fixture = Fixture::new(WireApi::Chat, router).await;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(fixture.url("/v1/chat/completions"))
            .bearer_auth("local-key")
            .json(&json!({"model":"public-model","messages":[],"stream":streaming}))
            .send()
            .await
            .unwrap();
        if !streaming {
            assert_eq!(response.status(), 401);
        }
        let text = response.text().await.unwrap();
        assert!(text.contains("fixture expired"));
        fixture
            .handle
            .shutdown(Duration::from_secs(2))
            .await
            .unwrap();
        let reports = fixture.host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].success);
        assert_eq!(reports[0].status, 401);
        assert!(reports[0].credential_scope);
        let detail = reports[0].error_message.as_ref().unwrap();
        assert!(detail.contains("fixture expired"));
        assert!(!detail.contains("private.invalid"));
        assert!(!detail.contains("hidden-token"));
    }
}

#[tokio::test]
async fn native_chat_stream_error_shapes_preserve_status_and_never_replay() {
    for (wire, status) in [
        (
            "data: {\"error\":{\"status_code\":429,\"message\":\"limited\"}}\n\n",
            429,
        ),
        (
            "data: {\"response\":{\"error\":{\"status\":403,\"message\":\"denied\"}}}\n\n",
            403,
        ),
        (
            "event: error\ndata: {\"status_code\":503,\"message\":\"busy\"}\n\n",
            503,
        ),
        (
            "data: {\"code\":\"busy\",\"message\":\"overloaded\",\"status\":503}\n\n",
            503,
        ),
        ("event: error\n\n", 502),
        ("event: error\ndata: [DONE]\n\n", 502),
    ] {
        for path in ["/v1/chat/completions", "/v1/responses"] {
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let app = Router::new().route("/v1/chat/completions",post(move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1,Ordering::SeqCst);
                ([("content-type","text/event-stream")], format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"kept\"}},\"finish_reason\":null}}]}}\n\n{wire}"))
            }
        }));
            let mut fixture = Fixture::new(WireApi::Chat, app).await;
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(fixture.url(path))
                .bearer_auth("local-key")
                .json(&if path.ends_with("/responses") {
                    json!({"model":"public-model","input":[],"stream":true})
                } else {
                    json!({"model":"public-model","messages":[],"stream":true})
                })
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let body = response.text().await.unwrap();
            assert_eq!(body.matches("kept").count(), 1);
            assert!(!body.contains("[DONE]"), "{path}: {wire}: {body}");
            assert!(body.contains("\"error\""), "{path}: {wire}: {body}");
            fixture
                .handle
                .shutdown(Duration::from_secs(2))
                .await
                .unwrap();
            let reports = fixture.host.reports.lock().unwrap();
            assert!(!reports[0].success, "{wire}");
            assert_eq!(reports[0].status, status, "{wire}");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
        }
    }
}
