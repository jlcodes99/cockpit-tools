//! Synthetic upstreams consume the complete POST before delaying response headers.
use super::*;

fn native_request(edit: bool, n: u32) -> Value {
    let mut body = json!({"model":"gpt-image-2.6-flare","prompt":"draw","n":n});
    if edit {
        body["image"] = json!("data:image/png;base64,AA==");
    }
    body
}

fn allow_native_retries(host: &ImageHost, opening: Duration) {
    host.direct_images.store(1, Ordering::SeqCst);
    host.retries.store(2, Ordering::SeqCst);
    host.credential_retries.store(2, Ordering::SeqCst);
    host.alternate_sources.store(1, Ordering::SeqCst);
    let mut policy = host.policy.lock().unwrap();
    policy.image_open_timeout = opening;
    policy.open_timeout = Duration::from_millis(20);
    policy.total_timeout = Duration::from_secs(3);
    policy.retry_base_delay = Duration::from_millis(1);
    policy.retry_max_delay = Duration::from_millis(1);
    policy.stream_open_max_attempts = 3;
    policy.status_retries = 2;
}

#[tokio::test]
async fn native_image_header_timeout_never_replays_or_cools_an_uncertain_post() {
    for edit in [false, true] {
        for n in [1, 2] {
            let path = if edit {
                "/v1/images/edits"
            } else {
                "/v1/images/generations"
            };
            let received = Arc::new(Mutex::new(Vec::<Value>::new()));
            let app = Router::new()
                .route(
                    path,
                    post(
                        |State(received): State<Arc<Mutex<Vec<Value>>>>,
                         Json(body): Json<Value>| async move {
                            // Json has read the entire body: this request might have
                            // already started generation when the opening budget ends.
                            received.lock().unwrap().push(body);
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            Json(json!({"data":[{"b64_json":"AA=="}]}))
                        },
                    ),
                )
                .with_state(received.clone());
            let (mut handle, host, upstream, root) = fixture(app, true, false).await;
            allow_native_retries(&host, Duration::from_millis(150));
            let response = client()
                .post(format!("http://{}{path}", handle.addr))
                .bearer_auth("local-image-key")
                .timeout(Duration::from_secs(4))
                .json(&native_request(edit, n))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 504);
            let error: Value = response.json().await.unwrap();
            assert_eq!(error["error"]["code"], "upstream_connect_timeout");
            assert!(error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("response headers timed out"));
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            assert_eq!(received.lock().unwrap().len(), 1, "edit={edit}, n={n}");
            assert_eq!(host.selections.lock().unwrap().len(), 1);
            assert!(host.failures.lock().unwrap().is_empty());
            assert_eq!(host.capacity.available_permits(), 1);
            {
                let reports = host.reports.lock().unwrap();
                assert_eq!(reports.len(), 1);
                assert!(!reports[0].success);
                assert_eq!(reports[0].status, 504);
                assert_eq!(
                    reports[0].error.as_deref(),
                    Some("upstream_connect_timeout")
                );
                assert!(reports[0].usage.is_none());
                let attempts = &reports[0].diagnostics.as_ref().unwrap().attempts;
                assert_eq!(attempts.len(), 1);
                assert_eq!(
                    attempts[0].error_category.as_deref(),
                    Some("upstream_connect_timeout")
                );
                assert!(!attempts[0].failure_cooldown_recorded);
            }
            cleanup(handle, upstream, root).await;
        }
    }
}

#[tokio::test]
async fn native_image_headers_can_arrive_after_text_opening_within_image_budget() {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/v1/images/generations",
            post(
                |State(calls): State<Arc<AtomicUsize>>, Json(_body): Json<Value>| async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Json(json!({"data":[{"b64_json":"AA=="}]}))
                },
            ),
        )
        .with_state(calls.clone());
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    allow_native_retries(&host, Duration::from_secs(1));
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .timeout(Duration::from_secs(4))
        .json(&native_request(false, 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"][0]["b64_json"], "AA==");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    assert!(host.failures.lock().unwrap().is_empty());
    assert_eq!(host.capacity.available_permits(), 1);
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn native_oauth_image_batch_delivers_all_requested_images_and_usage() {
    let counts = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = counts.clone();
    let app = Router::new().route("/v1/images/generations", post(move |Json(body): Json<Value>| {
        let capture = capture.clone();
        async move {
            let number = {
                let mut counts = capture.lock().unwrap();
                counts.push(body["n"].clone());
                counts.len()
            };
            Json(json!({"data":[{"b64_json": if number == 1 { "AA==" } else { "AQ==" }}],
                "usage":{"total_tokens": 3}}))
        }
    }));
    let (handle, host, upstream, root) = fixture(app, true, false).await;
    host.direct_images.store(1, Ordering::SeqCst);
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&native_request(false, 2))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"], json!([{"b64_json":"AA=="},{"b64_json":"AQ=="}]));
    assert_eq!(body["usage"]["total_tokens"], 6);
    assert_eq!(*counts.lock().unwrap(), vec![json!(2), json!(1)]);
    assert_eq!(host.selections.lock().unwrap().len(), 1);
    cleanup(handle, upstream, root).await;
}
