use super::*;

#[test]
fn go_limits_sse_metadata_does_not_consume_the_data_line_budget() {
    let mut decoder = native::events::SseDecoder::new(1024);
    let event = json!({"type":"response.output_text.delta","delta":"x".repeat(953)});
    let data = format!("data: {event}");
    assert!(data.len() < 1024);
    let wire = format!("event: response.output_text.delta\n{data}\n\n");
    assert!(wire.len() > 1024);
    assert_eq!(decoder.push(wire.as_bytes()).unwrap().len(), 1);
    let mut decoder = native::events::SseDecoder::new(1024);
    assert_eq!(
        decoder
            .push(format!("data: {}\n\n", "x".repeat(1024)).as_bytes())
            .unwrap_err()
            .code,
        "upstream_event_too_large"
    );
}

#[test]
fn go_limits_terminal_output_may_exceed_the_individual_event_limit() {
    let mut decoder = native::events::SseDecoder::new(1024);
    for index in 0..3 {
        let event = json!({"type":"response.output_item.done","output_index":index,"item":{"type":"reasoning","encrypted_content":"x".repeat(512)}});
        decoder
            .push(format!("data: {event}\n\n").as_bytes())
            .unwrap();
    }
    let events = decoder.push(completed().as_bytes()).unwrap();
    let terminal = events.last().unwrap();
    assert_eq!(terminal["response"]["output"].as_array().unwrap().len(), 3);
    assert!(terminal.to_string().len() > 1024);
}

#[tokio::test]
async fn go_limits_unlimited_total_allows_active_response_and_still_cancels() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async {
            let chunks = futures_util::stream::unfold(0, |index| async move {
                tokio::time::sleep(Duration::from_millis(25)).await;
                let bytes = if index < 8 {
                    " "
                } else {
                    r#"{"id":"long","status":"completed","output":[]}"#
                };
                (index <= 8).then(|| {
                    (
                        Ok::<_, std::io::Error>(Bytes::from_static(bytes.as_bytes())),
                        index + 1,
                    )
                })
            });
            Body::from_stream(chunks)
        }),
    );
    let (handle, host, task, path) = start(
        app,
        ExecutionPolicy {
            idle_timeout: Duration::from_millis(150),
            total_timeout: Duration::ZERO,
            ..Default::default()
        },
    )
    .await;
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","input":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["id"], "long");
    let response = client()
        .post(format!("http://{}/v1/responses", handle.addr))
        .bearer_auth("key")
        .json(&json!({"model":"m","stream":true,"input":[]}))
        .send();
    let pending = tokio::spawn(response);
    tokio::time::sleep(Duration::from_millis(40)).await;
    // Shutdown must interrupt an unlimited request as well as finite requests.
    shutdown(handle, task, path).await;
    let _ = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .unwrap();
    assert!(host
        .reports
        .lock()
        .unwrap()
        .iter()
        .any(|report| report.success));
}

#[tokio::test]
async fn go_limits_accepts_large_json_response_and_large_sse_event() {
    for streaming in [false, true] {
        let size = if streaming {
            9 * 1024 * 1024
        } else {
            33 * 1024 * 1024
        };
        let text = "x".repeat(size);
        let payload = if streaming {
            format!(
                "data: {}\n\n{}",
                json!({"type":"response.output_text.delta","output_index":0,"delta":text}),
                completed()
            )
        } else {
            json!({"id":"large","status":"completed","output":[],"padding":text}).to_string()
        };
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let payload = payload.clone();
                async move {
                    (
                        [(
                            "content-type",
                            if streaming {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        )],
                        payload,
                    )
                }
            }),
        );
        let (handle, _, task, path) = start(app, ExecutionPolicy::default()).await;
        let response = client()
            .post(format!("http://{}/v1/responses", handle.addr))
            .bearer_auth("key")
            .json(&json!({"model":"m","stream":streaming,"input":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let raw = response.text().await.unwrap();
        assert!(raw.len() > size);
        assert!(!raw.contains("upstream_event_too_large"));
        assert!(!raw.contains("upstream_response_too_large"));
        if streaming {
            assert!(raw.contains("response.completed"));
        } else {
            assert_eq!(
                serde_json::from_str::<Value>(&raw).unwrap()["padding"]
                    .as_str()
                    .unwrap()
                    .len(),
                size
            );
        }
        shutdown(handle, task, path).await;
    }
}
