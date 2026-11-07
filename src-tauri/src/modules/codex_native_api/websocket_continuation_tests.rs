use super::*;

fn invalid_previous() -> Value {
    json!({"type":"error","status":400,"error":{"type":"invalid_request_error","message":"Invalid `previous_response_id`."}})
}

fn root() -> Value {
    let mut body = request();
    body["input"] = json!([{"role":"user","content":"original context"}]);
    body["instructions"] = json!("preserve instructions");
    body["tools"] = json!([]);
    body
}

fn next() -> Value {
    json!({"type":"response.create","model":"model","previous_response_id":"completed",
        "input":[{"role":"user","content":"next turn"}],
        "client_metadata":{"session_id":"root","thread_id":"root"}})
}

pub(super) async fn serve(
    state: &Upstream,
    headers: &HeaderMap,
    socket: &mut axum::extract::ws::WebSocket,
    mut body: Value,
) {
    let Behavior::Continuation {
        error,
        delayed,
        business,
        always,
    } = &state.behavior
    else {
        unreachable!()
    };
    loop {
        if body.get("previous_response_id").is_some()
            || (*always && state.attempts.load(Ordering::SeqCst) > 1)
        {
            if *delayed {
                if socket
                    .send(Message::Text(
                        json!({"type":"response.created","response":{"id":"rejected","output":[]}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(350)).await;
            }
            if let Some(event) = business {
                if socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            if socket
                .send(Message::Text(error.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        } else {
            let id = if state.attempts.load(Ordering::SeqCst) > 1 {
                "recovered"
            } else {
                "completed"
            };
            let event = json!({"type":"response.completed","response":{"id":id,"status":"completed",
                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"previous answer"}]}]}});
            if socket
                .send(Message::Text(event.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }
        loop {
            match socket.recv().await {
                Some(Ok(Message::Text(text))) => {
                    body = serde_json::from_str(text.as_str()).unwrap();
                    state
                        .requests
                        .lock()
                        .unwrap()
                        .push((headers.clone(), body.clone()));
                    break;
                }
                Some(Ok(Message::Ping(data))) => {
                    let _ = socket.send(Message::Pong(data)).await;
                }
                Some(Ok(Message::Pong(_))) => {}
                _ => return,
            }
        }
    }
}

async fn connect(
    fixture: &Fixture,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local"));
    tokio_tungstenite::connect_async(request).await.unwrap().0
}

async fn terminal(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let ClientMessage::Text(text) = frame {
            let event: Value = serde_json::from_str(text.as_str()).unwrap();
            let terminal = matches!(
                event["type"].as_str(),
                Some("error" | "response.completed" | "response.failed")
            );
            events.push(event);
            if terminal {
                return events;
            }
        }
    }
}

#[tokio::test]
async fn invalid_continuation_recovers_complete_history_on_new_and_reused_sockets() {
    for reconnect in [false, true] {
        for delayed in [false, true] {
            let fixture = Fixture::start(
                Behavior::Continuation {
                    error: invalid_previous(),
                    delayed,
                    business: None,
                    always: false,
                },
                policy(),
                false,
            )
            .await;
            let mut socket = connect(&fixture).await;
            socket
                .send(ClientMessage::Text(root().to_string().into()))
                .await
                .unwrap();
            assert_eq!(
                terminal(&mut socket).await.last().unwrap()["type"],
                "response.completed"
            );
            if reconnect {
                let _ = socket.close(None).await;
                socket = connect(&fixture).await;
            }
            socket
                .send(ClientMessage::Text(next().to_string().into()))
                .await
                .unwrap();
            let events = terminal(&mut socket).await;
            assert_eq!(
                events.last().unwrap()["response"]["id"],
                "recovered",
                "{events:?}"
            );
            assert!(!events.iter().any(|event| event["type"] == "error"));
            {
                let sent = fixture.first.requests.lock().unwrap();
                assert_eq!(sent.len(), 3);
                let original = &sent[0].1;
                let replay = &sent[2].1;
                assert!(replay.get("previous_response_id").is_none());
                assert_eq!(replay["input"].as_array().unwrap().len(), 3);
                assert_eq!(replay["input"][0]["content"], "original context");
                assert_eq!(replay["input"][1]["role"], "assistant");
                assert_eq!(replay["input"][2]["content"], "next turn");
                assert_eq!(replay["instructions"], original["instructions"]);
                assert_eq!(replay["tools"], original["tools"]);
                assert_eq!(sent[0].0.get("thread-id"), sent[2].0.get("thread-id"));
                assert_eq!(
                    sent[0].0.get("authorization"),
                    sent[2].0.get("authorization")
                );
            }
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
            assert!(fixture.host.failures.lock().unwrap().is_empty());
            {
                let reports = fixture.host.reports.lock().unwrap();
                let report = reports.last().unwrap();
                assert!(report.success);
                let diagnostics = report.diagnostics.as_ref().unwrap();
                assert_eq!(diagnostics.attempts.len(), 2);
                assert!(diagnostics
                    .attempts
                    .iter()
                    .all(|attempt| attempt.account_id == "a"));
                assert!(diagnostics.payloads.is_empty());
            }
            let _ = socket.close(None).await;
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn invalid_continuation_with_missing_history_preserves_error() {
    let error = invalid_previous();
    let fixture = Fixture::start(
        Behavior::Continuation {
            error: error.clone(),
            delayed: false,
            business: None,
            always: false,
        },
        policy(),
        false,
    )
    .await;
    fixture.request(root(), 1).await;
    {
        let db = rusqlite::Connection::open(fixture.path.join("identity.sqlite")).unwrap();
        db.execute(
            "DELETE FROM native_rebuild_checkpoints_v1 WHERE response='completed'",
            [],
        )
        .unwrap();
        db.execute(
            "DELETE FROM native_rebuild_history_v1 WHERE response='completed'",
            [],
        )
        .unwrap();
    }
    let events = fixture.request(next(), 1).await;
    assert_eq!(events[0], error);
    assert_eq!(fixture.first.requests.lock().unwrap().len(), 2);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}

#[tokio::test]
async fn invalid_continuation_retry_is_bounded_and_does_not_fail_over() {
    let error = invalid_previous();
    let fixture = Fixture::start(
        Behavior::Continuation {
            error: error.clone(),
            delayed: false,
            business: None,
            always: true,
        },
        policy(),
        false,
    )
    .await;
    fixture.request(root(), 1).await;
    let events = fixture.request(next(), 1).await;
    assert_eq!(events[0], error);
    assert_eq!(fixture.first.requests.lock().unwrap().len(), 3);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    assert!(fixture.host.failures.lock().unwrap().is_empty());
    fixture.finish().await;
}

#[tokio::test]
async fn invalid_continuation_after_text_or_tool_output_is_never_replayed() {
    for business in [
        json!({"type":"response.output_text.delta","delta":"already generated"}),
        json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call-1","name":"run","arguments":"{}"}}),
    ] {
        let error = invalid_previous();
        let fixture = Fixture::start(
            Behavior::Continuation {
                error: error.clone(),
                delayed: false,
                business: Some(business.clone()),
                always: false,
            },
            policy(),
            false,
        )
        .await;
        fixture.request(root(), 1).await;
        let events = fixture.request(next(), 2).await;
        assert_eq!(events, vec![business, error]);
        assert_eq!(fixture.first.requests.lock().unwrap().len(), 2);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        fixture.finish().await;
    }
}

#[test]
fn continuation_error_matching_requires_an_exact_unexecuted_rejection() {
    assert!(continuation::rejection(&invalid_previous()));
    assert!(continuation::rejection(
        &json!({"type":"response.failed","response":{"output":[],"error":{"code":"previous_response_not_found"}}})
    ));
    for event in [
        json!({"type":"error","error":{"code":"invalid_value","message":"Invalid `previous_response_id`."}}),
        json!({"type":"error","error":{"message":"Invalid previous_response_id format elsewhere"}}),
        json!({"type":"response.failed","response":{"error":{"code":"previous_response_not_found"},"output":[{"type":"function_call","name":"run"}]}}),
        json!({"type":"response.output_text.delta","delta":"Invalid `previous_response_id`."}),
    ] {
        assert!(!continuation::rejection(&event), "{event}");
    }
}

#[tokio::test]
async fn invalid_continuation_recovery_preserves_history_for_the_following_turn() {
    let fixture = Fixture::start(
        Behavior::Continuation {
            error: invalid_previous(),
            delayed: false,
            business: None,
            always: false,
        },
        policy(),
        false,
    )
    .await;
    let mut socket = connect(&fixture).await;
    socket
        .send(ClientMessage::Text(root().to_string().into()))
        .await
        .unwrap();
    terminal(&mut socket).await;
    socket
        .send(ClientMessage::Text(next().to_string().into()))
        .await
        .unwrap();
    assert_eq!(
        terminal(&mut socket).await.last().unwrap()["response"]["id"],
        "recovered"
    );
    let mut following = next();
    following["previous_response_id"] = json!("recovered");
    following["input"][0]["content"] = json!("after recovery");
    socket
        .send(ClientMessage::Text(following.to_string().into()))
        .await
        .unwrap();
    assert_eq!(
        terminal(&mut socket).await.last().unwrap()["type"],
        "response.completed"
    );
    {
        let sent = fixture.first.requests.lock().unwrap();
        assert_eq!(sent.len(), 5);
        let replay = &sent[4].1;
        assert_eq!(replay["input"].as_array().unwrap().len(), 5);
        assert_eq!(replay["input"][0]["content"], "original context");
        assert_eq!(replay["input"][4]["content"], "after recovery");
    }
    let _ = socket.close(None).await;
    fixture.finish().await;
}

#[tokio::test]
async fn cancelled_continuation_does_not_start_recovery() {
    let mut fixture = Fixture::start(
        Behavior::Continuation {
            error: invalid_previous(),
            delayed: true,
            business: None,
            always: false,
        },
        policy(),
        false,
    )
    .await;
    let mut socket = connect(&fixture).await;
    socket
        .send(ClientMessage::Text(root().to_string().into()))
        .await
        .unwrap();
    terminal(&mut socket).await;
    socket
        .send(ClientMessage::Text(next().to_string().into()))
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(first, ClientMessage::Text(ref text) if text.contains("response.created")));
    let _ = socket.close(None).await;
    fixture
        .handle
        .shutdown(Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(fixture.first.requests.lock().unwrap().len(), 2);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
    fixture.finish().await;
}
