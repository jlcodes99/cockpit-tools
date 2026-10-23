use super::*;

struct DelayedBatchSource {
    requests: Mutex<Vec<(HeaderMap, Value)>>,
    second_delay: Duration,
}
async fn delayed_batch_response(
    State(source): State<Arc<DelayedBatchSource>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let index = {
        let mut requests = source.requests.lock().unwrap();
        let index = requests.len();
        requests.push((headers, body));
        index
    };
    let id = format!("batch-response-{index}");
    let created = Bytes::from(format!(
        "data: {}\n\n",
        json!({"type":"response.created","response":{"id":id,"status":"in_progress"}})
    ));
    // Response headers and the first event arrive within the opening budget;
    // generation itself intentionally lasts longer than that budget.
    let delay = if index == 0 {
        Duration::from_millis(450)
    } else {
        source.second_delay
    };
    let completed = stream::once(async move {
        tokio::time::sleep(delay).await;
        Ok::<_, std::io::Error>(Bytes::from(format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[{"type":"image_generation_call","id":format!("image-{index}"),"result":"AA=="}],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}})
        )))
    });
    let bytes = stream::once(async move { Ok::<_, std::io::Error>(created) }).chain(completed);
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(bytes))
        .unwrap()
}

#[tokio::test]
async fn oauth_image_batch_renews_each_opening_without_extending_request_total_deadline() {
    for total_expires in [false, true] {
        let source = Arc::new(DelayedBatchSource {
            requests: Mutex::new(vec![]),
            second_delay: if total_expires {
                Duration::from_millis(450)
            } else {
                Duration::from_millis(10)
            },
        });
        let app = Router::new()
            .route("/v1/responses", post(delayed_batch_response))
            .with_state(source.clone());
        let (mut handle, host, upstream, root) = fixture(app, true, false).await;
        {
            let mut policy = host.policy.lock().unwrap();
            policy.image_open_timeout = Duration::from_millis(200);
            policy.image_idle_timeout = Duration::from_secs(2);
            policy.total_timeout = if total_expires {
                Duration::from_millis(650)
            } else {
                Duration::from_secs(2)
            };
        }
        let started = tokio::time::Instant::now();
        let response = client()
            .post(format!("http://{}/v1/images/generations", handle.addr))
            .bearer_auth("local-image-key")
            .header("session-id", "batch-thread")
            .header("thread-id", "batch-thread")
            .json(&json!({"prompt":"draw","n":2}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), if total_expires { 504 } else { 200 });
        let value: Value = response.json().await.unwrap();
        if total_expires {
            assert_eq!(value["error"]["code"], "upstream_total_timeout");
            assert!(started.elapsed() < Duration::from_millis(850));
        } else {
            assert_eq!(value["data"].as_array().unwrap().len(), 2);
            assert_eq!(value["usage"]["total_tokens"], 10);
        }
        // Shutdown settles persistence before testing the single outcome and
        // route permit, including cancellation of the second operation.
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        let requests = source.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for name in ["authorization", "session-id", "thread-id"] {
            assert_eq!(requests[0].0.get(name), requests[1].0.get(name), "{name}");
        }
        drop(requests);
        assert_eq!(host.selections.lock().unwrap().len(), 1);
        assert_eq!(host.capacity.available_permits(), 1);
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].account_id, "image-account");
        assert_eq!(reports[0].success, !total_expires);
        assert_eq!(
            reports[0].usage.as_ref().unwrap()["total_tokens"],
            if total_expires { 5 } else { 10 }
        );
        drop(reports);
        cleanup(handle, upstream, root).await;
    }
}
