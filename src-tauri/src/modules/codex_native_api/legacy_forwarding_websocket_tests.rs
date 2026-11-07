use super::*;

#[tokio::test]
async fn legacy_forwarding_native_websocket_keeps_unregistered_and_unknown_history() {
    for chatgpt in [false, true] {
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app = Router::new().route(
            "/v1/responses",
            get({
                let seen = seen.clone();
                move |ws: WebSocketUpgrade| {
                    let seen = seen.clone();
                    async move {
                        ws.on_upgrade(move |mut socket| async move {
                            while let Some(Ok(Message::Text(text))) = socket.recv().await {
                                let request: Value = serde_json::from_str(text.as_str()).unwrap();
                                seen.lock().unwrap().push(request);
                                for event in [
                                    json!({"type":"response.created","response":{"id":"legacy-forwarded"}}),
                                    json!({"type":"response.completed","response":{"id":"legacy-forwarded","status":"completed","output":[]}}),
                                ] {
                                    socket
                                        .send(Message::Text(event.to_string().into()))
                                        .await
                                        .unwrap();
                                }
                            }
                        })
                    }
                }
            }),
        );
        let (addr, upstream) = mock(app).await;
        let (mut service, host, _) = native(addr, chatgpt, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses", service.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer local-test-key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let original = json!({
            "type":"response.create",
            "model":"test-model",
            "extension":{"preserve":"exact extension"},
            "input":[
                {"role":"user","content":"continue the original history"},
                {"type":"future_history_item","payload":{"keep":[1,"two",true]}},
                {"type":"reasoning","encrypted_content":super::super::codex_cpa_wire::synthetic_gpt_signature(43),"summary":[]},
                {"type":"function_call_output","call_id":"unregistered-orphan","output":"Original tool output\nwith trailing whitespace.  "}
            ]
        });
        socket
            .send(ClientMessage::Text(original.to_string().into()))
            .await
            .unwrap();
        assert_eq!(ws_json(&mut socket).await["type"], "response.created");
        assert_eq!(ws_json(&mut socket).await["type"], "response.completed");
        {
            let captured = seen.lock().unwrap();
            assert_eq!(captured.len(), 1, "chatgpt={chatgpt}");
            assert_eq!(captured[0]["input"], original["input"], "chatgpt={chatgpt}");
            assert_eq!(captured[0]["extension"], original["extension"]);
        }
        assert_eq!(host.selections.lock().unwrap().len(), 1);
        let _ = socket.close(None).await;
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        upstream.abort();
    }
}

#[tokio::test]
async fn legacy_forwarding_native_websocket_leaves_unknown_previous_response_to_upstream() {
    for chatgpt in [false, true] {
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let upstream_error = json!({
            "type":"error",
            "status":400,
            "error":{
                "type":"invalid_request_error",
                "code":"previous_response_not_found",
                "message":"Only the real upstream rejected this unknown previous response",
                "param":"previous_response_id",
                "upstream_marker":"preserve-this-diagnostic"
            }
        });
        let app = Router::new().route(
            "/v1/responses",
            get({
                let seen = seen.clone();
                let event = upstream_error.clone();
                move |ws: WebSocketUpgrade| {
                    let seen = seen.clone();
                    let event = event.clone();
                    async move {
                        ws.on_upgrade(move |mut socket| async move {
                            while let Some(Ok(Message::Text(text))) = socket.recv().await {
                                let request: Value = serde_json::from_str(text.as_str()).unwrap();
                                seen.lock().unwrap().push(request);
                                socket
                                    .send(Message::Text(event.to_string().into()))
                                    .await
                                    .unwrap();
                            }
                        })
                    }
                }
            }),
        );
        let (addr, upstream) = mock(app).await;
        let (mut service, host, _) = native(addr, chatgpt, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses", service.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer local-test-key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let original = json!({
            "type":"response.create",
            "model":"test-model",
            "previous_response_id":"response-known-only-to-upstream",
            "input":[{"role":"user","content":"continue"}]
        });
        socket
            .send(ClientMessage::Text(original.to_string().into()))
            .await
            .unwrap();
        assert_eq!(
            ws_json(&mut socket).await,
            upstream_error,
            "chatgpt={chatgpt}"
        );
        {
            let captured = seen.lock().unwrap();
            assert_eq!(captured.len(), 1, "chatgpt={chatgpt}");
            assert_eq!(
                captured[0]["previous_response_id"], original["previous_response_id"],
                "chatgpt={chatgpt}"
            );
            assert_eq!(captured[0]["input"], original["input"]);
        }
        assert_eq!(host.selections.lock().unwrap().len(), 1);
        let _ = socket.close(None).await;
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        upstream.abort();
    }
}
