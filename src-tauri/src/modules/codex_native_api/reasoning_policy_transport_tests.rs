use super::*;
use crate::models::codex_local_access::CodexReasoningLevel as Level;

#[tokio::test]
async fn reasoning_policy_rejects_http_and_sse_before_upstream_dispatch() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let app = Router::new().route(
        "/v1/responses",
        post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                let id = {
                    let mut sent = capture.lock().unwrap();
                    sent.push(body);
                    format!("resp-{}", sent.len())
                };
                (
                    [("content-type", "text/event-stream")],
                    format!("{}{}", created(), completed()).replace("resp-a", &id),
                )
            }
        }),
    );
    let (addr, upstream) = mock(app).await;
    let (mut service, host, root) = native(addr, true, false, Options::default()).await;
    *host.reasoning_ceiling.lock().unwrap() = Some(Level::Medium);
    let http = client();
    for (key, endpoint, request, expected) in [
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","input":[],"reasoning":{"effort":"low"}}),
            200,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","input":[],"stream":true,"reasoning":{"effort":"medium"}}),
            200,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","previous_response_id":"resp-1","input":[{"role":"user","content":"continue"}]}),
            200,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","input":[],"stream":true,"reasoning":{"effort":"ultra"}}),
            403,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","input":[],"reasoning":{"effort":"auto"}}),
            403,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","input":[]}),
            403,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model(max)","input":[],"reasoning":{"effort":"low"}}),
            403,
        ),
        (
            "local-test-key",
            "responses",
            json!({"model":"test-model","reasoning":{"effort":"medium"},"input":[{"type":"configuration_update","reasoning":{"effort":"xhigh"}}]}),
            403,
        ),
        (
            "local-test-key",
            "chat/completions",
            json!({"model":"test-model","messages":[{"role":"user","content":"hi"}],"reasoning_effort":"high"}),
            403,
        ),
        (
            "unlimited-test-key",
            "responses",
            json!({"model":"test-model","input":[],"reasoning":{"effort":"high"}}),
            200,
        ),
    ] {
        let response = http
            .post(format!("http://{}/v1/{endpoint}", service.addr))
            .bearer_auth(key)
            .json(&request)
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let response_body = response.text().await.unwrap();
        assert_eq!(status, expected, "{request} -> {response_body}");
        if expected == 403 {
            assert!(
                response_body.contains("reasoning_effort_"),
                "{response_body}"
            );
        }
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let sent = seen.lock().unwrap();
    assert_eq!(
        sent.iter()
            .map(|body| body["reasoning"]["effort"].as_str())
            .collect::<Vec<_>>(),
        [Some("low"), Some("medium"), Some("low"), Some("high")]
    );
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn reasoning_policy_rejects_websocket_turns_without_losing_the_previous_response() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let capture = seen.clone();
    let app = Router::new().route("/v1/responses", get(move |ws: WebSocketUpgrade| {
        let capture = capture.clone();
        async move {
            ws.on_upgrade(move |mut socket| async move {
                while let Some(Ok(Message::Text(text))) = socket.recv().await {
                    let request = serde_json::from_str(text.as_str()).unwrap();
                    let id = {
                        let mut seen = capture.lock().unwrap();
                        seen.push(request);
                        format!("response-{}", seen.len())
                    };
                    for event in [
                        json!({"type":"response.created","response":{"id":id}}),
                        json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[]}}),
                    ] {
                        socket.send(Message::Text(event.to_string().into())).await.unwrap();
                    }
                }
            })
        }
    }));
    let (addr, upstream) = mock(app).await;
    let (mut service, host, root) = native(addr, true, false, Options::default()).await;
    *host.reasoning_ceiling.lock().unwrap() = Some(Level::Medium);
    let mut request = format!("ws://{}/v1/responses", service.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    request
        .headers_mut()
        .insert("session-id", "reasoning-session".parse().unwrap());
    request
        .headers_mut()
        .insert("thread-id", "reasoning-session".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let create = |effort| json!({"type":"response.create","model":"test-model","input":[],"reasoning":{"effort":effort}});
    ws.send(ClientMessage::Text(create("low").to_string().into()))
        .await
        .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    let first = ws_json(&mut ws).await;
    assert_eq!(first["type"], "response.completed");
    let previous = first["response"]["id"].clone();
    let mut above = create("high");
    above["previous_response_id"] = previous.clone();
    ws.send(ClientMessage::Text(above.to_string().into()))
        .await
        .unwrap();
    let error = ws_json(&mut ws).await;
    assert_eq!(error["type"], "error", "{error}");
    assert!(
        error
            .to_string()
            .contains("reasoning_effort_exceeds_key_limit"),
        "{error}"
    );
    let mut next = create("low");
    next["previous_response_id"] = previous;
    next["input"] = json!([{"type":"configuration_update","reasoning":{"effort":"medium"}}]);
    ws.send(ClientMessage::Text(next.to_string().into()))
        .await
        .unwrap();
    let created = ws_json(&mut ws).await;
    assert_eq!(created["type"], "response.created", "{created}");
    let second = ws_json(&mut ws).await;
    assert_eq!(second["type"], "response.completed");
    let inherited = json!({
        "type":"response.create", "model":"test-model",
        "previous_response_id":second["response"]["id"],
        "input":[{"role":"user","content":"continue"}]
    });
    ws.send(ClientMessage::Text(inherited.to_string().into()))
        .await
        .unwrap();
    assert_eq!(ws_json(&mut ws).await["type"], "response.created");
    let third = ws_json(&mut ws).await;
    assert_eq!(third["type"], "response.completed");
    *host.reasoning_ceiling.lock().unwrap() = Some(Level::Low);
    let mut now_above = inherited;
    now_above["previous_response_id"] = third["response"]["id"].clone();
    ws.send(ClientMessage::Text(now_above.to_string().into()))
        .await
        .unwrap();
    let rejected = ws_json(&mut ws).await;
    assert!(
        rejected
            .to_string()
            .contains("reasoning_effort_exceeds_key_limit"),
        "{rejected}"
    );
    ws.close(None).await.unwrap();
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    let sent = seen.lock().unwrap();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0]["reasoning"]["effort"], "low");
    assert_eq!(sent[1]["input"][0]["reasoning"]["effort"], "medium");
    assert!(sent[2].get("reasoning").is_none());
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}
