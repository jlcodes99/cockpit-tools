use super::*;
use std::io::Write;

#[tokio::test]
async fn request_decodes_mixed_case_chained_content_encoding() {
    let seen = Arc::new(Mutex::new(None));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                axum::Json(json!({"id":"compressed-response","status":"completed","output":[]}))
            }
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, _, root) = native(addr, false, false, Options::default()).await;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(body(false).to_string().as_bytes()).unwrap();
    let gzip = gzip.finish().unwrap();
    let encoded = zstd::stream::encode_all(gzip.as_slice(), 1).unwrap();
    let response = client()
        .post(format!("http://{}/responses", service.addr))
        .bearer_auth("local-test-key")
        .header("content-encoding", " GZip, Identity, ZSTD ")
        .body(encoded)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    assert_eq!(
        seen.lock().unwrap().as_ref().unwrap()["input"],
        body(false)["input"]
    );
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn response_headers_match_go_host_sanitizer() {
    let mut headers = HeaderMap::new();
    headers.append(
        "connection",
        HeaderValue::from_static("X-Upstream-Secret, upgrade"),
    );
    headers.append("connection", HeaderValue::from_static("X-Second"));
    for name in [
        "x-upstream-secret",
        "x-second",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "x-litellm-cost",
        "helicone-id",
        "x-portkey-request",
        "cf-aig-id",
        "x-kong-token",
        "x-bt-id",
        "set-cookie",
    ] {
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static("private"),
        );
    }
    headers.insert("x-request-id", HeaderValue::from_static("public-id"));
    headers.insert("retry-after", HeaderValue::from_static("3"));
    let response =
        super::super::server::downstream_response(StatusCode::OK, &headers, Body::empty());
    assert_eq!(response.headers().len(), 2);
    assert_eq!(response.headers()["x-request-id"], "public-id");
    assert_eq!(response.headers()["retry-after"], "3");
}

#[test]
fn overload_does_not_invent_a_retry_after_header() {
    use axum::response::IntoResponse;
    for status in [429, 503] {
        let response = ServiceError::new(status, "upstream_overloaded", "Busy").into_response();
        assert!(!response.headers().contains_key("retry-after"));
        let response = ServiceError::new(status, "upstream_overloaded", "Busy")
            .with_retry_after(Some(Duration::from_secs(7)))
            .into_response();
        assert_eq!(response.headers()["retry-after"], "7");
    }
}
