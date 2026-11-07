//! Actual loopback HTTP/WS transport tests; no external upstream is used.
use super::*;
use std::io::Write;

#[tokio::test]
async fn compressed_search_rebuilds_json_without_the_original_encoding_header() {
    let seen = Arc::new(Mutex::new(None::<Value>));
    let app = Router::new()
        .route(
            "/v1/alpha/search",
            post(
                |State(seen): State<Arc<Mutex<Option<Value>>>>,
                 headers: HeaderMap,
                 Json(body): Json<Value>| async move {
                    assert!(!headers.contains_key(header::CONTENT_ENCODING));
                    *seen.lock().unwrap() = Some(body);
                    Json(json!({"results":[]}))
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(br#"{"input":"raw 250ms cipher","commands":{"search_query":[{"q":"query"}]}}"#)
        .unwrap();
    let response = client()
        .post(format!("http://{}/v1/alpha/search", handle.addr))
        .bearer_auth("local-a")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_ENCODING, "gzip")
        .body(gzip.finish().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json::<Value>().await.unwrap();
    wait_free(&host).await;
    let body = seen.lock().unwrap().clone().unwrap();
    assert_eq!(body["input"], "raw 250ms cipher");
    assert_eq!(body["commands"]["search_query"][0]["q"], "query");
    cleanup(handle, upstream, root).await;
}

#[tokio::test]
async fn oauth_audio_trims_false_but_api_key_keeps_the_original_field() {
    for oauth in [true, false] {
        let seen = Arc::new(Mutex::new(Vec::<(String, Vec<u8>)>::new()));
        let app = Router::new()
            .route(
                if oauth {
                    "/v1/transcribe"
                } else {
                    "/v1/audio/transcriptions"
                },
                post(
                    |State(seen): State<Arc<Mutex<Vec<(String, Vec<u8>)>>>>,
                     mut form: Multipart| async move {
                        while let Some(field) = form.next_field().await.unwrap() {
                            let name = field.name().unwrap().to_owned();
                            let bytes = field.bytes().await.unwrap().to_vec();
                            seen.lock().unwrap().push((name, bytes));
                        }
                        Json(json!({"text":"recognized"}))
                    },
                ),
            )
            .with_state(seen.clone());
        let (handle, host, upstream, root) = fixture(app, oauth, Options::default()).await;
        for value in [" \tfalse\r\n", "\u{2003}false\u{2003}"] {
            let response = client()
                .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
                .bearer_auth("local-a")
                .multipart(
                    reqwest::multipart::Form::new()
                        .text("model", "client-model")
                        .text("stream", value.to_owned())
                        .part(
                            "file",
                            reqwest::multipart::Part::bytes(vec![0, 255]).file_name("voice.wav"),
                        ),
                )
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 200, "oauth={oauth}");
            assert_eq!(
                response.json::<Value>().await.unwrap()["text"],
                "recognized"
            );
            wait_free(&host).await;
            let fields = seen.lock().unwrap();
            if oauth {
                assert!(!fields.iter().any(|(name, _)| name == "stream"));
            } else {
                assert!(fields.contains(&("stream".into(), value.as_bytes().to_vec())));
            }
        }
        if oauth {
            let before = seen.lock().unwrap().len();
            let response = client()
                .post(format!("http://{}/v1/audio/transcriptions", handle.addr))
                .bearer_auth("local-a")
                .multipart(
                    reqwest::multipart::Form::new()
                        .text("model", "client-model")
                        .text("stream", " true ")
                        .part(
                            "file",
                            reqwest::multipart::Part::bytes(vec![1]).file_name("voice.wav"),
                        ),
                )
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 400);
            wait_free(&host).await;
            assert_eq!(seen.lock().unwrap().len(), before);
        }
        cleanup(handle, upstream, root).await;
    }
}

#[tokio::test]
async fn realtime_binary_json_is_scoped_and_opaque_binary_is_preserved() {
    let seen = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let app = Router::new()
        .route(
            "/v1/realtime",
            get(
                |ws: WebSocketUpgrade,
                 headers: HeaderMap,
                 State(seen): State<Arc<Mutex<Vec<Vec<u8>>>>>| async move {
                    assert_eq!(headers["authorization"], "Bearer upstream-secret");
                    ws.on_upgrade(move |mut socket| async move {
                        while let Some(Ok(Message::Binary(bytes))) = socket.recv().await {
                            seen.lock().unwrap().push(bytes.to_vec());
                            if socket.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                    })
                },
            ),
        )
        .with_state(seen.clone());
    let (handle, host, upstream, root) = fixture(app, true, Options::default()).await;
    let mut request = format!("ws://{}/v1/realtime?model=gpt-realtime", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local-a"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let body = json!({"type":"session.update","session":{"id":"raw-binary-session","model":"gpt-realtime"}});
    socket
        .send(UpstreamMessage::Binary(
            body.to_string().into_bytes().into(),
        ))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let UpstreamMessage::Binary(bytes) = response else {
        panic!("Binary response frame expected")
    };
    let rewritten: Value = serde_json::from_slice(&bytes).unwrap();
    assert_ne!(rewritten["session"]["id"], "raw-binary-session");
    assert_eq!(rewritten["session"]["model"], "gpt-realtime");
    let opaque = vec![255, 0, 128, 13, 10];
    socket
        .send(UpstreamMessage::Binary(opaque.clone().into()))
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response, UpstreamMessage::Binary(opaque.clone().into()));
    socket
        .send(UpstreamMessage::Binary(
            json!({"type":"session.update","session":{"model":"forged-model"}})
                .to_string()
                .into_bytes()
                .into(),
        ))
        .await
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap();
    drop(socket);
    wait_free(&host).await;
    let forwarded = seen.lock().unwrap();
    assert_eq!(forwarded.len(), 2);
    assert_eq!(forwarded[1], opaque);
    drop(forwarded);
    assert_eq!(host.reports.lock().unwrap().last().unwrap().status, 403);
    assert_eq!(
        host.reports
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .error
            .as_deref(),
        Some("realtime_model_scope_mismatch")
    );
    cleanup(handle, upstream, root).await;
}
