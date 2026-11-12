use super::*;

fn relay_history_is_portable(body: &Value) -> bool {
    let input = &body["input"];
    input[0].get("id").is_none()
        && input[0]["role"] == "assistant"
        && input[0]["content"][0]["text"] == "Previous answer from relay"
        && input[1].get("id").is_none()
        && input[1]["call_id"] == "call_legacy_patch"
        && input[2]["call_id"] == "call_legacy_patch"
        && input[2]["output"] == "patch applied"
        && input[3].get("id").is_none()
        && input[3]["type"] == "reasoning"
        && input[3]["summary"][0]["text"] == "Previous reasoning summary from relay"
}

#[tokio::test]
async fn legacy_relay_item_ids_are_stripped_on_official_http_sse_and_websocket_wire() {
    let app = Router::new().route(
        "/v1/responses",
        post(|axum::Json(body): axum::Json<Value>| async move {
            use axum::response::IntoResponse;
            if relay_history_is_portable(&body) {
                (
                    [("content-type", "text/event-stream")],
                    format!("{}{}", created(), completed()),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    axum::Json(json!({"error":{"message":"Invalid 'input[3].id'. Expected an ID that begins with 'rs'."}})),
                )
                    .into_response()
            }
        })
        .get(|ws: WebSocketUpgrade| async {
            ws.on_upgrade(|mut socket| async move {
                let text = socket.recv().await.unwrap().unwrap();
                let body: Value = serde_json::from_str(text.to_text().unwrap()).unwrap();
                let event = if relay_history_is_portable(&body) {
                    json!({"type":"response.completed","response":{"id":"resp-legacy","status":"completed","output":[]}})
                } else {
                    json!({"type":"error","status":400,"error":{"message":"Invalid 'input[3].id'. Expected an ID that begins with 'rs'."}})
                };
                socket.send(Message::Text(event.to_string().into())).await.unwrap();
                while socket.recv().await.is_some() {}
            })
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, _, root) = native(addr, true, false, Options::default()).await;
    let history = json!([
        {"type":"message","id":"item_db645dXXXXXXf7e51","role":"assistant","content":[{"type":"output_text","text":"Previous answer from relay"}]},
        {"type":"custom_tool_call","id":"item_patch","call_id":"call_legacy_patch","name":"apply_patch","input":"fixture"},
        {"type":"custom_tool_call_output","call_id":"call_legacy_patch","output":"patch applied"},
        {"type":"reasoning","id":"item_32855e1b58e547dca07ba920","summary":[{"type":"summary_text","text":"Previous reasoning summary from relay"}]},
        {"type":"message","role":"user","content":[{"type":"input_text","text":"Continue with my subscription"}]}
    ]);
    for stream in [false, true] {
        let mut body = body(stream);
        body["input"] = history.clone();
        let response = client()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("local-test-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = response.status();
        let output = response.text().await.unwrap();
        assert_eq!(status, StatusCode::OK, "{output}");
        assert!(output.contains("resp-a"), "{output}");
    }
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(ClientMessage::Text(
            json!({
                "type":"response.create", "model":"test-model", "input":history
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let event = ws_json(&mut socket).await;
    assert_eq!(event["type"], "response.completed", "{event}");
    socket.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
    std::fs::remove_dir_all(root).unwrap();
}
