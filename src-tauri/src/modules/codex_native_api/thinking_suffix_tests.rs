//! Real loopback HTTP, Chat and WS paths, with no external provider calls.
use super::*;

#[tokio::test]
async fn non_gpt_suffix_reaches_http_and_chat_responses_with_original_tool_arguments() {
    let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/responses",
            post(
                |State(seen): State<Arc<Mutex<Vec<Value>>>>,
                 axum::Json(body): axum::Json<Value>| async move {
                    let id = {
                        let mut values = seen.lock().unwrap();
                        values.push(body);
                        format!("suffix-response-{}", values.len())
                    };
                    axum::Json(
                        json!({"id":id,"status":"completed","output":[],"model":"mapped-model"}),
                    )
                },
            ),
        )
        .with_state(seen.clone());
    let (addr, upstream) = mock(app).await;
    let (mut service, host, root) = native(addr, false, false, Options::default()).await;
    // An aliased selected model cannot be used to recover the client's suffix.
    host.bridge_with_policy.store(true, Ordering::SeqCst);
    for (path, model, expected) in [
        ("responses", "o3(high)", "high"),
        ("responses", "custom(8192)", "medium"),
        ("chat/completions", "o3(high)", "high"),
        ("chat/completions", "custom(8192)", "medium"),
    ] {
        let body = if path == "responses" {
            json!({"model":model,"reasoning":{"effort":"low","summary":"detailed"},"input":[{"type":"function_call","call_id":"raw-call","name":"tool","arguments":"raw 250ms cipher"}]})
        } else {
            json!({"model":model,"reasoning_effort":"low","messages":[{"role":"assistant","tool_calls":[{"id":"raw-call","type":"function","function":{"name":"tool","arguments":"raw 250ms cipher"}}]}]})
        };
        let response = client()
            .post(format!("http://{}/v1/{path}", service.addr))
            .bearer_auth("local-test-key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
        let sent = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(sent["model"], "mapped-model");
        assert_eq!(sent["reasoning"]["effort"], expected);
        assert_eq!(sent["input"][0]["arguments"], "raw 250ms cipher");
        assert_eq!(sent["input"][0]["call_id"], "raw-call");
    }
    service.shutdown(Duration::from_secs(2)).await.unwrap();
    upstream.abort();
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn non_gpt_suffix_reaches_real_responses_websocket() {
    for (model, expected) in [("o3(high)", "high"), ("custom(8192)", "medium")] {
        let seen = Arc::new(Mutex::new(None::<Value>));
        let app=Router::new().route("/v1/responses",get(|State(seen):State<Arc<Mutex<Option<Value>>>>,ws:WebSocketUpgrade|async move {
            ws.on_upgrade(move |mut socket|async move {
                if let Some(Ok(Message::Text(text)))=socket.recv().await {
                    *seen.lock().unwrap()=Some(serde_json::from_str(&text).unwrap());
                    socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"suffix-ws","status":"completed","output":[]}}).to_string().into())).await.unwrap();
                    while socket.recv().await.is_some(){}
                }
            })
        })).with_state(seen.clone());
        let (addr, upstream) = mock(app).await;
        let (mut service, _host, root) = native(addr, false, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses", service.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer local-test-key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(ClientMessage::Text(json!({"type":"response.create","model":model,"reasoning":{"effort":"low"},"input":[{"type":"function_call","call_id":"raw-call","name":"tool","arguments":"raw 250ms cipher"}]}).to_string().into())).await.unwrap();
        assert_eq!(ws_json(&mut socket).await["type"], "response.completed");
        let sent = seen.lock().unwrap().clone().unwrap();
        assert_eq!(sent["reasoning"]["effort"], expected);
        assert_eq!(sent["input"][0]["arguments"], "raw 250ms cipher");
        assert_eq!(sent["input"][0]["call_id"], "raw-call");
        socket.close(None).await.unwrap();
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        upstream.abort();
        let _ = std::fs::remove_dir_all(root);
    }
}
