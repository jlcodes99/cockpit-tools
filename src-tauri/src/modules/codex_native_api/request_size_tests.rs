//! Exercise real requests beyond the former gateway cap, not a larger test-only cap.
use super::*;
use std::io::Write;

const LARGE_TEXT_BYTES: usize = 17 * 1024 * 1024;

fn large_body() -> Value {
    let mut request = body(false);
    request["input"][0]["content"] = json!("x".repeat(LARGE_TEXT_BYTES));
    request
}

fn completed_response() -> Value {
    json!({"id":"large-request-response","status":"completed","output":[]})
}

#[tokio::test]
async fn default_http_accepts_large_plain_chunked_and_compressed_conversations() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    let handler = move |headers: HeaderMap, axum::Json(request): axum::Json<Value>| {
        let capture = capture.clone();
        async move {
            assert!(!headers.contains_key("content-encoding"));
            let text = request["input"][0]["content"].as_str().unwrap();
            assert_eq!(text.len(), LARGE_TEXT_BYTES);
            assert!(text.bytes().all(|b| b == b'x'));
            capture.lock().unwrap().push(text.len());
            axum::Json(completed_response())
        }
    };
    let app = Router::new()
        .route("/v1/responses", post(handler.clone()))
        .route("/v1/responses/compact", post(handler))
        .layer(axum::extract::DefaultBodyLimit::disable());
    let (addr, upstream) = mock(app).await;
    let (mut service, _, root) = native(addr, false, false, Options::default()).await;
    let raw = serde_json::to_vec(&large_body()).unwrap();
    let gzip = {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap()
    };
    for (path, encoding, payload, chunked) in [
        ("/v1/responses", "identity", raw.clone(), false),
        ("/v1/responses", "identity", raw.clone(), true),
        ("/v1/responses", "gzip", gzip.clone(), false),
        (
            "/v1/responses",
            "zstd",
            zstd::stream::encode_all(raw.as_slice(), 1).unwrap(),
            false,
        ),
        (
            "/v1/responses/compact",
            "gzip, zstd",
            zstd::stream::encode_all(gzip.as_slice(), 1).unwrap(),
            false,
        ),
    ] {
        let request_body = if chunked {
            let chunks = payload
                .chunks(256 * 1024)
                .map(|chunk| Ok::<_, std::io::Error>(Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>();
            reqwest::Body::wrap_stream(futures_util::stream::iter(chunks))
        } else {
            reqwest::Body::from(payload)
        };
        let response = client()
            .post(format!("http://{}{path}", service.addr))
            .bearer_auth("local-test-key")
            .header("content-type", "application/json")
            .header("content-encoding", encoding)
            .body(request_body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            200,
            "{path}/{encoding}/chunked={chunked}: {}",
            response.text().await.unwrap()
        );
    }
    assert_eq!(seen.lock().unwrap().as_slice(), &[LARGE_TEXT_BYTES; 5]);
    service.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn default_websocket_accepts_large_conversation_frames() {
    let seen = Arc::new(Mutex::new(None));
    let capture = seen.clone();
    let app = Router::new().route("/v1/responses", get(move |ws: WebSocketUpgrade| {
        let capture = capture.clone();
        async move {
            ws.max_message_size(usize::MAX).max_frame_size(usize::MAX)
                .on_upgrade(move |mut socket| async move {
                    let raw = socket.recv().await.unwrap().unwrap();
                    let request: Value = serde_json::from_str(raw.to_text().unwrap()).unwrap();
                    let text = request["input"][0]["content"].as_str().unwrap();
                    assert_eq!(text.len(), LARGE_TEXT_BYTES);
                    assert!(text.bytes().all(|b| b == b'x'));
                    *capture.lock().unwrap() = Some(text.len());
                    socket.send(Message::Text(json!({"type":"response.completed","response":completed_response()}).to_string().into())).await.unwrap();
                    while socket.recv().await.is_some() {}
                })
        }
    }));
    let (addr, upstream) = mock(app).await;
    let (mut service, _, root) = native(addr, true, false, Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let mut request = large_body();
    request["type"] = json!("response.create");
    socket
        .send(ClientMessage::Text(request.to_string().into()))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(20), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event: Value = serde_json::from_str(event.to_text().unwrap()).unwrap();
    assert_eq!(event["type"], "response.completed", "{event}");
    assert_eq!(*seen.lock().unwrap(), Some(LARGE_TEXT_BYTES));
    socket.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn transport_failure_is_not_reported_as_a_size_limit_and_explicit_limits_still_work() {
    use super::super::request_body::read_error;
    let failed = Body::from_stream(futures_util::stream::iter([
        Ok(Bytes::from_static(b"partial body")),
        Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "request body ended unexpectedly",
        )),
    ]));
    let error = axum::body::to_bytes(failed, Options::default().max_body_bytes)
        .await
        .unwrap_err();
    let error = read_error(error);
    assert_eq!(error.status, 400);
    assert_eq!(error.code, "request_body_read_error");
    assert_eq!(error.message, "request body ended unexpectedly");
    let error = axum::body::to_bytes(Body::from("too large"), 4)
        .await
        .unwrap_err();
    let error = read_error(error);
    assert_eq!(error.status, 413);
    assert_eq!(error.code, "request_too_large");
}

#[tokio::test]
async fn unfinished_http_upload_still_times_out_without_reaching_account_selection() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (addr, upstream) = mock(Router::new()).await;
    let (mut service, host, root) = native(
        addr,
        false,
        false,
        Options {
            request_timeout: Duration::from_millis(100),
            ..Default::default()
        },
    )
    .await;
    let mut socket = tokio::net::TcpStream::connect(service.addr).await.unwrap();
    socket.write_all(b"POST /v1/responses HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer local-test-key\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{").await.unwrap();
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(3), socket.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    assert!(response.contains("request_body_timeout"), "{response}");
    assert!(host.selections.lock().unwrap().is_empty());
    service.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}
