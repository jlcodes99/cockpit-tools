use super::*;

#[tokio::test]
async fn go_limits_image_upload_bounds_each_file_instead_of_the_request_total() {
    let seen = Arc::new(AtomicUsize::new(0));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/images/edits",
        post(move |mut form: Multipart| {
            let seen = capture.clone();
            async move {
                while let Some(field) = form.next_field().await.unwrap() {
                    if field.file_name().is_some() {
                        assert_eq!(field.bytes().await.unwrap().len(), 8);
                        seen.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Json(json!({"data":[{"b64_json":"AA=="}]}))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = fixture_host(format!("http://{addr}/v1"), false, false);
    let root = std::env::temp_dir().join(format!("go-limits-image-{}", uuid::Uuid::now_v7()));
    let handle = start(
        "127.0.0.1:0".parse().unwrap(),
        host,
        root.join("identity.sqlite"),
        Options {
            max_upload_bytes: 12,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for (size, expected) in [(8, 200), (13, 413)] {
        let mut form = reqwest::multipart::Form::new()
            .text("prompt", "edit")
            .text("model", "image-model");
        for name in ["one.png", "two.png"] {
            form = form.part(
                "image[]",
                reqwest::multipart::Part::bytes(vec![1; size])
                    .file_name(name)
                    .mime_str("image/png")
                    .unwrap(),
            );
        }
        let response = client()
            .post(format!("http://{}/v1/images/edits", handle.addr))
            .bearer_auth("local-image-key")
            .multipart(form)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status().as_u16(),
            expected,
            "{}",
            response.text().await.unwrap()
        );
    }
    assert_eq!(seen.load(Ordering::SeqCst), 2);
    let response = client().post(format!("http://{}/v1/images/edits", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"model":"image-model","prompt":"edit","image":"data:image/png;base64,AQEBAQEBAQEBAQEBAQEBAQE="}))
        .send().await.unwrap();
    assert_eq!(response.status(), 413, "{}", response.text().await.unwrap());
    cleanup(handle, upstream, root).await;
}

#[test]
fn go_limits_image_response_defaults_match_go() {
    let options = Options::default();
    assert_eq!(options.max_image_response_bytes, 96 * 1024 * 1024);
    assert_eq!(options.max_upload_bytes, 64 * 1024 * 1024);
    assert_eq!(options.max_response_bytes, usize::MAX);
}

#[tokio::test]
async fn go_limits_image_api_uses_its_own_generation_budget() {
    let app = Router::new().route(
        "/v1/images/generations",
        post(|| async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Json(json!({"data":[{"b64_json":"AA=="}]}))
        }),
    );
    let (handle, host, upstream, root) = fixture(app, false, false).await;
    {
        let mut policy = host.policy.lock().unwrap();
        policy.image_open_timeout = Duration::from_millis(100);
        policy.image_idle_timeout = Duration::from_millis(100);
    }
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"model":"image-model","prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn go_limits_image_api_accepts_a_response_larger_than_32_mib() {
    let size = 33 * 1024 * 1024;
    let app = Router::new().route(
        "/v1/images/generations",
        post(move || async move { Json(json!({"data":[{"b64_json":"A".repeat(size)}]})) }),
    );
    let (handle, _, upstream, root) = fixture(app, false, false).await;
    let response = client()
        .post(format!("http://{}/v1/images/generations", handle.addr))
        .bearer_auth("local-image-key")
        .json(&json!({"model":"image-model","prompt":"draw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response: Value = response.json().await.unwrap();
    assert_eq!(
        response["data"][0]["b64_json"].as_str().unwrap().len(),
        size
    );
    cleanup(handle, upstream, root).await;
}
