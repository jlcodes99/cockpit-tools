use super::*;

async fn send(fixture: &Fixture, key: &str, request: Value) -> reqwest::Response {
    client()
        .post(format!("http://{}/v1/responses", fixture.handle.addr))
        .bearer_auth(key)
        .json(&request)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn key_continuity_http_keeps_new_caller_accounting_and_failover_history() {
    let fixture = Fixture::with_key_continuity(
        true,
        vec![Step::Complete("old-root", None), Step::HttpRateLimit],
        vec![Step::Complete("new-root", None)],
        true,
    )
    .await;
    let first = send(
        &fixture,
        "old-key",
        body(vec![message("user", "KEY-ROTATION-MARKER")], None, false),
    )
    .await;
    assert_eq!(first.status(), 200);
    let _: Value = first.json().await.unwrap();
    let next = send(
        &fixture,
        "new-key",
        body(vec![message("user", "continue")], Some("old-root"), false),
    )
    .await;
    let status = next.status();
    let result: Value = next.json().await.unwrap();
    assert_eq!(status, 200, "{result}");
    let sent = fixture.sent();
    assert_eq!(sent.last().unwrap().account, "b");
    assert!(sent.last().unwrap().body["input"]
        .to_string()
        .contains("KEY-ROTATION-MARKER"));
    assert!(sent
        .last()
        .unwrap()
        .body
        .get("previous_response_id")
        .is_none());
    let reports = fixture.host.reports.lock().unwrap().clone();
    assert!(reports.iter().any(|report| report.caller.id == "new-key"));
    fixture.finish().await;
}

#[tokio::test]
async fn key_continuity_websocket_reconnects_with_new_key_and_old_response() {
    let fixture = Fixture::with_key_continuity(
        true,
        vec![
            Step::Complete("old-root", None),
            Step::Complete("new-root", None),
        ],
        vec![],
        true,
    )
    .await;
    let first = send(
        &fixture,
        "old-key",
        body(vec![message("user", "WS-ROTATION-MARKER")], None, false),
    )
    .await;
    assert_eq!(first.status(), 200);
    let _: Value = first.json().await.unwrap();
    let mut handshake = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    handshake
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer new-key"));
    let (mut socket, _) = tokio_tungstenite::connect_async(handshake).await.unwrap();
    let mut request = body(vec![message("user", "continue")], Some("old-root"), true);
    request["type"] = json!("response.create");
    socket
        .send(Message::Text(request.to_string().into()))
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(frame) = socket.next().await {
            if let Message::Text(text) = frame.unwrap() {
                let event: Value = serde_json::from_str(&text).unwrap();
                assert_ne!(event["type"], "error", "{event}");
                if event["type"] == "response.completed" {
                    return event;
                }
            }
        }
        panic!("socket closed before completion")
    })
    .await
    .unwrap();
    assert_eq!(result["response"]["id"], "new-root");
    let _ = socket.close(None).await;
    fixture.finish().await;
}

#[tokio::test]
async fn key_continuity_partial_retry_retains_executed_tool_result() {
    let call = json!({"type":"function_call","name":"write_once","call_id":"once","arguments":"{}","status":"completed"});
    let fixture=Fixture::with_key_continuity(true,
        vec![Step::RawEvents(vec![json!({"type":"response.incomplete","response":{"id":"partial-root","status":"incomplete","output":[call]}})]),Step::HttpRateLimit,Step::HttpRateLimit],
        vec![Step::HttpRateLimit,Step::Complete("recovered",None)],true).await;
    fixture.host.oauth.store(true, Ordering::SeqCst);
    let first = send(
        &fixture,
        "old-key",
        body(
            vec![message("user", "PARTIAL-ROTATION-MARKER")],
            None,
            false,
        ),
    )
    .await;
    let first_status = first.status();
    let first: Value = first.json().await.unwrap();
    assert_eq!(first_status, 200, "{first}");
    let request = body(
        vec![
            json!({"type":"function_call_output","call_id":"once","output":"already executed"}),
            message("user", "continue"),
        ],
        Some("partial-root"),
        false,
    );
    let failed = send(&fixture, "new-key", request.clone()).await;
    assert!(!failed.status().is_success());
    let _ = failed.bytes().await.unwrap();
    let recovered = send(&fixture, "new-key", request).await;
    let status = recovered.status();
    let recovered: Value = recovered.json().await.unwrap();
    assert_eq!(status, 200, "{recovered}");
    let sent = fixture.sent();
    let input = sent.last().unwrap().body["input"].as_array().unwrap();
    assert_eq!(
        input
            .iter()
            .filter(|v| v["type"] == "function_call")
            .count(),
        1
    );
    assert_eq!(
        input
            .iter()
            .filter(|v| v["type"] == "function_call_output")
            .count(),
        1
    );
    assert!(serde_json::to_string(input)
        .unwrap()
        .contains("PARTIAL-ROTATION-MARKER"));
    fixture.finish().await;
}
