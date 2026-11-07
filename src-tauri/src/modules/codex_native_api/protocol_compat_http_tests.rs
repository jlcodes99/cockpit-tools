use super::*;

#[tokio::test]
async fn websocket_empty_pool_falls_back_only_before_any_event_and_preserves_close_details() {
    use std::sync::atomic::AtomicUsize;
    for (prior, reason, expected_http) in [
        (None, "no available account", true),
        (None, "try again later", false),
        (Some("response.created"), "no available account", false),
        (Some("response.output_text.delta"), "no available account", false),
    ] {
        let posts = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route("/v1/responses", get(move |ws: WebSocketUpgrade| async move {
            ws.on_upgrade(move |mut socket| async move {
                let _ = socket.recv().await;
                if let Some(kind) = prior {
                    socket.send(Message::Text(json!({"type":kind,"delta":"partial","response":{"id":"begun"}}).to_string().into())).await.unwrap();
                }
                socket.send(Message::Close(Some(axum::extract::ws::CloseFrame {code:1013, reason:reason.into()}))).await.unwrap();
            })
        }).post({let posts = posts.clone(); move |headers: HeaderMap, axum::Json(body): axum::Json<Value>| {
            let posts = posts.clone(); async move {
                assert_eq!(headers["authorization"], "Bearer upstream-test-key");
                assert_eq!(body["input"], json!([]));
                posts.fetch_add(1, Ordering::SeqCst);
                ([("content-type","text/event-stream")], "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"http\",\"status\":\"completed\",\"output\":[]}}\n\n")
            }
        }}));
        let (addr, task) = mock(app).await;
        let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses",handle.addr).into_client_request().unwrap();
        request.headers_mut().insert("authorization","Bearer local-test-key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(ClientMessage::Text(json!({"type":"response.create","model":"m","input":[]}).to_string().into())).await.unwrap();
        let mut event = ws_json(&mut socket).await;
        if prior.is_some() && event["type"] != "error" { event = ws_json(&mut socket).await; }
        if expected_http {
            assert_eq!(event["type"],"response.completed","{event}");
            assert_eq!(host.selections.lock().unwrap().len(),1);
        } else {
            assert_eq!(event["error"]["code"],"upstream_websocket_lost","{event}");
            assert_eq!(event["error"]["upstream_close_code"],1013);
            assert_eq!(event["error"]["upstream_close_reason"],reason);
        }
        assert_eq!(posts.load(Ordering::SeqCst),usize::from(expected_http));
        socket.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}

fn wire_stream(failed: bool) -> String {
    let mut events = vec![
        json!({"type":"responsesapi.timing","private_marker":"timing"}),
        json!({"type":"response.subscription_usage","private_marker":"billing"}),
        json!({"type":"codex.rate_limits","private_marker":"quota"}),
        json!({"type":"codex.response.metadata","codex_marker":true}),
        json!({"type":"response.metadata","moderation_marker":true}),
        json!({"type":"response.created","sequence_number":7,"response":{"id":"resp_wire","created_at":123,"model":"m"}}),
    ];
    events.push(if failed { json!({"type":"response.failed","sequence_number":8,"response":{"status":"failed","error":{"code":"server_error","message":"test failure"}}}) }
        else {json!({"type":"response.completed","sequence_number":8,"response":{"id":"resp_wire","status":"completed","output":[]}})});
    events
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[tokio::test]
async fn responses_public_wire_filters_private_events_and_completes_failure_objects() {
    for failed in [false, true] {
        let stream = wire_stream(failed);
        let (addr, task) = mock(Router::new().route(
            "/v1/responses",
            post(move || {
                let stream = stream.clone();
                async move { ([("content-type", "text/event-stream")], stream) }
            }),
        ))
        .await;
        let (mut handle, _host, path) = native(addr, false, false, Options::default()).await;
        for codex in [false, true] {
            let request = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("local-test-key")
                .header(
                    "user-agent",
                    if codex {
                        "codex_cli_rs/1.0"
                    } else {
                        "standard-sdk/1.0"
                    },
                )
                .json(&body(true));
            let text = request.send().await.unwrap().text().await.unwrap();
            assert!(!text.contains("private_marker"), "{text}");
            assert_eq!(text.contains("codex_marker"), codex, "{text}");
            assert_eq!(text.contains("moderation_marker"), codex, "{text}");
            let events = text
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .collect::<Vec<_>>();
            let last = events.last().unwrap();
            if !failed || codex {
                assert_eq!(last["response"]["created_at"], 123, "{text}");
                assert_eq!(last["sequence_number"], 8);
            }
            if failed && codex {
                assert_eq!(last["response"]["id"], "resp_wire");
                assert_eq!(last["response"]["object"], "response");
                assert_eq!(last["response"]["output"], json!([]));
            }
        }
        if !failed {
            let value: Value = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("local-test-key")
                .json(&body(false))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(value["created_at"], 123);
            assert_eq!(value["id"], "resp_wire");
        }
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tokio::test]
async fn websocket_loss_keeps_client_open_fences_queue_and_reconnects_same_owner() {
    use std::sync::atomic::AtomicUsize;
    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    let app=Router::new().route("/v1/responses",get({
        let connections=connections.clone(); let requests=requests.clone(); let release=release.clone();
        move |ws:WebSocketUpgrade| {
            let index=connections.fetch_add(1,Ordering::SeqCst); let requests=requests.clone(); let release=release.clone();
            async move {ws.on_upgrade(move |mut socket|async move {
                let Some(Ok(Message::Text(text)))=socket.recv().await else {return;};
                requests.lock().unwrap().push(serde_json::from_str(&text).unwrap());
                if index==0 {
                    socket.send(Message::Text(json!({"type":"response.created","response":{"id":"lost","created_at":123}}).to_string().into())).await.unwrap();
                    socket.send(Message::Text(json!({"type":"response.output_text.delta","delta":"partial"}).to_string().into())).await.unwrap();
                    release.notified().await;
                } else {
                    socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"restored","status":"completed","output":[]}}).to_string().into())).await.unwrap();
                    let _=socket.recv().await;
                }
            })}
        }
    }));
    let (addr, task) = mock(app).await;
    let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
    let mut request = format!("ws://{}/v1/responses", handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer local-test-key".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"test-model","input":[],"event_id":"active"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(ws_json(&mut socket).await["type"], "response.created");
    assert_eq!(ws_json(&mut socket).await["delta"], "partial");
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"test-model","input":[],"event_id":"queued"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    release.notify_one();
    let loss = ws_json(&mut socket).await;
    assert_eq!(loss["error"]["code"], "upstream_websocket_lost");
    assert_eq!(loss["event_id"], "active");
    let queued = ws_json(&mut socket).await;
    assert_eq!(queued["event_id"], "queued");
    assert_eq!(queued["error"]["not_sent"], true);
    assert_eq!(connections.load(Ordering::SeqCst), 1);
    assert_eq!(requests.lock().unwrap().len(), 1);
    socket.send(ClientMessage::Text(json!({"type":"response.create","model":"test-model","input":[],"previous_response_id":"lost"}).to_string().into())).await.unwrap();
    assert_eq!(
        ws_json(&mut socket).await["error"]["code"],
        "previous_response_not_found"
    );
    // Key revocation is still checked for the new turn, even on the surviving socket.
    host.key_enabled.store(false, Ordering::SeqCst);
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"test-model","input":[]})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        ws_json(&mut socket).await["error"]["code"],
        "invalid_api_key"
    );
    host.key_enabled.store(true, Ordering::SeqCst);
    socket
        .send(ClientMessage::Text(
            json!({"type":"response.create","model":"test-model","input":"fresh independent turn"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    assert_eq!(ws_json(&mut socket).await["response"]["id"], "restored");
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(
        host.selections
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .preferred_account_id
            .as_deref(),
        Some("account-a")
    );
    socket.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
    let _ = std::fs::remove_dir_all(path);
}

#[tokio::test]
async fn opening_and_idle_disconnects_keep_client_socket_without_replaying_turns() {
    use std::sync::atomic::AtomicUsize;
    for idle in [false, true] {
        let count = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let app=Router::new().route("/v1/responses",get({
            let count=count.clone(); let release=release.clone();
            move |ws:WebSocketUpgrade| {
                let index=count.fetch_add(1,Ordering::SeqCst); let release=release.clone();
                async move {ws.on_upgrade(move |mut socket|async move {
                    let _=socket.recv().await;
                    if index==0 {
                        if idle {
                            socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"completed-before-loss","status":"completed","output":[]}}).to_string().into())).await.unwrap();
                            release.notified().await;
                        }
                    } else {
                        socket.send(Message::Text(json!({"type":"response.completed","response":{"id":"reconnected","status":"completed","output":[]}}).to_string().into())).await.unwrap();
                        let _=socket.recv().await;
                    }
                })}
            }
        }));
        let (addr, task) = mock(app).await;
        let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
        let mut request = format!("ws://{}/v1/responses", handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer local-test-key".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let create = json!({"type":"response.create","model":"test-model","input":[]});
        socket
            .send(ClientMessage::Text(create.to_string().into()))
            .await
            .unwrap();
        if idle {
            assert_eq!(ws_json(&mut socket).await["type"], "response.completed");
            release.notify_one();
        }
        assert_eq!(
            ws_json(&mut socket).await["error"]["code"],
            "upstream_websocket_lost"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        socket
            .send(ClientMessage::Text(create.to_string().into()))
            .await
            .unwrap();
        assert_eq!(ws_json(&mut socket).await["response"]["id"], "reconnected");
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(
            host.selections
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .preferred_account_id
                .as_deref(),
            Some("account-a")
        );
        socket.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}

#[tokio::test]
async fn admission_limit_and_quota_keep_distinct_reports_and_retry_hints() {
    for (code, transient) in [
        ("turn_rate_limit_exceeded", true),
        ("usage_limit_reached", false),
    ] {
        for sse in [false, true] {
            let error = json!({"code":code,"message":"limited","type":if transient {"rate_limit_error"}else{"usage_limit_reached"}});
            let payload = if sse {
                format!("data: {{\"type\":\"response.queued\"}}\n\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":\"\"}}\n\ndata: {}\n\n",json!({"type":"response.failed","response":{"status":"failed","error":error}}))
            } else {
                json!({"error":error}).to_string()
            };
            let app = Router::new().route(
                "/v1/responses",
                post(move || {
                    let payload = payload.clone();
                    async move {
                        (
                            if sse {
                                StatusCode::OK
                            } else {
                                StatusCode::TOO_MANY_REQUESTS
                            },
                            [
                                (
                                    "content-type",
                                    if sse {
                                        "text/event-stream"
                                    } else {
                                        "application/json"
                                    },
                                ),
                                ("retry-after", "2"),
                            ],
                            payload,
                        )
                    }
                }),
            );
            let (addr, task) = mock(app).await;
            let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
            let response = client()
                .post(format!("http://{}/v1/responses", handle.addr))
                .bearer_auth("local-test-key")
                .header("user-agent", "codex_cli_rs/1")
                .json(&body(sse))
                .send()
                .await
                .unwrap();
            let text = response.text().await.unwrap();
            assert!(text.contains(code), "{text}");
            handle.shutdown(Duration::from_secs(2)).await.unwrap();
            let reports = host.reports.lock().unwrap();
            let report = reports.last().unwrap();
            assert_eq!(report.status, 429, "{text}");
            assert_eq!(report.transient_rate_limit, transient);
            assert_eq!(report.retry_after, Some(Duration::from_secs(2)));
            task.abort();
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

#[tokio::test]
async fn websocket_http_bridge_applies_public_event_filter_and_response_timestamp() {
    for codex in [false, true] {
        let app = Router::new().route(
            "/v1/responses",
            post(|| async { ([("content-type", "text/event-stream")], wire_stream(false)) }),
        );
        let (addr, task) = mock(app).await;
        let (mut handle, host, path) = native(addr, false, false, Options::default()).await;
        host.bridge_with_policy.store(true, Ordering::SeqCst);
        let mut request = format!("ws://{}/v1/responses", handle.addr)
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer local-test-key".parse().unwrap());
        request.headers_mut().insert(
            "user-agent",
            if codex {
                "codex_cli_rs/1"
            } else {
                "standard-sdk/1"
            }
            .parse()
            .unwrap(),
        );
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(ClientMessage::Text(
                json!({"type":"response.create","model":"test-model","input":[]})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let mut events = Vec::new();
        loop {
            let event = ws_json(&mut socket).await;
            let done = event["type"] == "response.completed";
            events.push(event);
            if done {
                break;
            }
        }
        assert!(!events
            .iter()
            .any(|event| event.get("private_marker").is_some()));
        assert_eq!(
            events
                .iter()
                .any(|event| event.get("codex_marker").is_some()),
            codex
        );
        assert_eq!(events.last().unwrap()["response"]["created_at"], 123);
        socket.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
        let _ = std::fs::remove_dir_all(path);
    }
}
