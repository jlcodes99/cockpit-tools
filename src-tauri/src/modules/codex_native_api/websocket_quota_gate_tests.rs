//! Real socket regressions: published handshake IDs do not establish execution.
use super::*;

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) async fn quota_gate_events(
    socket: &mut WebSocket,
    metadata: bool,
    business: Option<&Value>,
    wait_interrupt: bool,
    response_id: &str,
) {
    socket.send(Message::Text(json!({"type":"response.created","response":{"id":response_id,"status":"in_progress","output":[],"usage":null,"text":{"format":{"type":"text"}}}}).to_string().into())).await.unwrap();
    if metadata {
        socket
            .send(Message::Text(
                json!({"type":"response.metadata","metadata":{"request_id":"quota-test"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    }
    if let Some(event) = business {
        let events = event
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![event.clone()]);
        for event in events {
            socket
                .send(Message::Text(event.to_string().into()))
                .await
                .unwrap();
        }
    }
    if wait_interrupt {
        loop {
            match socket.recv().await {
                Some(Ok(Message::Text(text))) => {
                    let event: Value = serde_json::from_str(text.as_str()).unwrap();
                    assert_eq!(event["type"], "response.interrupt");
                    assert_eq!(event["response_id"], response_id);
                    break;
                }
                Some(Ok(Message::Ping(value))) => {
                    socket.send(Message::Pong(value)).await.unwrap();
                }
                _ => return,
            }
        }
    } else {
        // Longer than both the old 250ms opening hold and the client-visible
        // handshake flush. The rejection must still hit the execution fence.
        tokio::time::sleep(Duration::from_millis(650)).await;
    }
    let _ = socket.send(Message::Text(json!({"type":"error","status":429,"error":{"code":"subscription_sharing_usage_limit_exceeded","type":"usage_limit_reached","message":"Synthetic quota exhausted","resets_in_seconds":60}}).to_string().into())).await;
    while socket.recv().await.is_some() {}
}

async fn connect(fixture: &Fixture) -> ClientSocket {
    let mut request = format!("ws://{}/v1/responses", fixture.handle.addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", HeaderValue::from_static("Bearer local"));
    tokio_tungstenite::connect_async(request).await.unwrap().0
}
async fn next(socket: &mut ClientSocket) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            ClientMessage::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
            ClientMessage::Ping(value) => socket.send(ClientMessage::Pong(value)).await.unwrap(),
            _ => {}
        }
    }
}
async fn terminal(socket: &mut ClientSocket) -> Vec<Value> {
    let mut events = Vec::new();
    loop {
        let event = next(socket).await;
        let done = super::super::super::events::response_terminal_kind(&event).is_some();
        events.push(event);
        if done {
            return events;
        }
    }
}
async fn send(socket: &mut ClientSocket, body: Value) {
    socket
        .send(ClientMessage::Text(body.to_string().into()))
        .await
        .unwrap();
}
fn long_conversation() -> Value {
    let mut body = request();
    let mut input = Vec::new();
    for turn in 0..80 {
        input.push(json!({"role":"user","content":format!("history question {turn}")}));
        input.push(json!({"role":"assistant","content":format!("history answer {turn}")}));
        if turn % 8 == 0 {
            let id = format!("call-{turn}");
            input.push(json!({"type":"function_call","call_id":id,"name":"read_file","arguments":"{\"path\":\"fixture.txt\"}"}));
            input.push(
                json!({"type":"function_call_output","call_id":id,"output":"fixture contents"}),
            );
        }
    }
    input.push(json!({"role":"user","content":"continue"}));
    body["input"] = Value::Array(input);
    body["tools"] = json!([{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]);
    body
}
fn gate(metadata: bool, business: Option<Value>, reused: bool, wait_interrupt: bool) -> Behavior {
    Behavior::QuotaGate {
        metadata,
        business,
        reused,
        wait_interrupt,
    }
}

#[tokio::test]
async fn websocket_delayed_quota_after_published_handshake_switches_for_plain_oauth_long_history() {
    for reused in [false, true] {
        for metadata in [false, true] {
            let fixture =
                Fixture::start(gate(metadata, None, reused, false), policy(), false).await;
            let mut socket = connect(&fixture).await;
            if reused {
                send(&mut socket, request()).await;
                assert_eq!(
                    terminal(&mut socket).await.last().unwrap()["type"],
                    "response.completed"
                );
            }
            let body = long_conversation();
            send(&mut socket, body.clone()).await;
            let events = terminal(&mut socket).await;
            assert_eq!(
                events.last().unwrap()["type"],
                "response.completed",
                "reused={reused}, metadata={metadata}: {events:?}"
            );
            assert!(events.iter().all(|event| event["type"] != "error"));
            assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(
                *fixture.host.failures.lock().unwrap(),
                vec![("a".into(), 429, Some(Duration::from_secs(60)))]
            );
            {
                let captures = fixture.second.requests.lock().unwrap();
                let input = captures[0].1["input"].as_array().unwrap();
                assert_eq!(input.len(), 181);
                assert_eq!(input.last().unwrap()["content"], "continue");
                assert_eq!(
                    input
                        .iter()
                        .filter(|item| item["type"] == "function_call")
                        .count(),
                    10
                );
                assert_eq!(
                    input
                        .iter()
                        .filter(|item| item["type"] == "function_call_output")
                        .count(),
                    10
                );
            }
            let _ = socket.close(None).await;
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn websocket_delayed_quota_never_replays_business_or_ambiguous_metadata() {
    for business in [
        json!({"type":"response.output_text.delta","delta":"already sent"}),
        json!({"type":"response.output_item.added","item":{"type":"function_call","call_id":"exec","name":"write_file","arguments":"{}"}}),
        json!({"type":"response.metadata","metadata":{"delta":"already generated"}}),
        json!({"type":"response.in_progress","response":{"usage":{"output_tokens":1}}}),
    ] {
        for reused in [false, true] {
            let fixture = Fixture::start(
                gate(true, Some(business.clone()), reused, false),
                policy(),
                false,
            )
            .await;
            let mut socket = connect(&fixture).await;
            if reused {
                send(&mut socket, request()).await;
                terminal(&mut socket).await;
            }
            send(&mut socket, long_conversation()).await;
            let events = terminal(&mut socket).await;
            assert_eq!(
                events.last().unwrap()["error"]["code"],
                "subscription_sharing_usage_limit_exceeded"
            );
            assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
            let _ = socket.close(None).await;
            fixture.finish().await;
        }
    }
}

#[tokio::test]
async fn websocket_published_handshake_accepts_interrupt_and_never_replays_after_it() {
    for reused in [false, true] {
        let fixture = Fixture::start(gate(false, None, reused, true), policy(), false).await;
        let mut socket = connect(&fixture).await;
        if reused {
            send(&mut socket, request()).await;
            terminal(&mut socket).await;
        }
        send(&mut socket, request()).await;
        assert_eq!(next(&mut socket).await["type"], "response.created");
        send(
            &mut socket,
            json!({"type":"response.interrupt","response_id":"quota-active"}),
        )
        .await;
        let events = terminal(&mut socket).await;
        assert_eq!(events.last().unwrap()["status"], 429);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        let _ = socket.close(None).await;
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_late_rejections_retain_actual_credential_budget_after_reopening() {
    let fixture = Fixture::start_with_second(
        gate(false, None, false, false),
        gate(false, None, false, false),
        ExecutionPolicy {
            credential_retries: 2,
            send_retries: 0,
            status_retries: 0,
            bootstrap_retries: 0,
            ..policy()
        },
        false,
    )
    .await;
    let mut socket = connect(&fixture).await;
    send(&mut socket, request()).await;
    let events = terminal(&mut socket).await;
    assert_eq!(events.last().unwrap()["status"], 429);
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .host
            .selections
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.excluded_account_ids.contains(&"b".to_owned()))
            .count(),
        0,
        "no extra candidate lookup once actual-send credential budget is exhausted"
    );
    let host = fixture.host.clone();
    let _ = socket.close(None).await;
    fixture.finish().await;
    assert_eq!(
        host.reports.lock().unwrap()[0]
            .diagnostics
            .as_ref()
            .unwrap()
            .attempts
            .len(),
        2
    );
}

#[tokio::test]
async fn websocket_old_published_id_cannot_interrupt_replacement_response() {
    let fixture = Fixture::start_with_second(
        gate(false, None, false, false),
        Behavior::WaitForInterrupt,
        policy(),
        false,
    )
    .await;
    let mut socket = connect(&fixture).await;
    send(&mut socket, request()).await;
    assert_eq!(next(&mut socket).await["response"]["id"], "quota-active");
    assert_eq!(next(&mut socket).await["response"]["id"], "backup-active");
    send(
        &mut socket,
        json!({"type":"response.interrupt","response_id":"quota-active"}),
    )
    .await;
    assert_eq!(
        next(&mut socket).await["error"]["code"],
        "unknown_interrupt_response"
    );
    send(
        &mut socket,
        json!({"type":"response.interrupt","response_id":"backup-active"}),
    )
    .await;
    assert_eq!(
        terminal(&mut socket).await.last().unwrap()["type"],
        "response.completed"
    );
    assert_eq!(
        fixture.second.requests.lock().unwrap().len(),
        2,
        "only create and valid interrupt reach replacement"
    );
    let _ = socket.close(None).await;
    fixture.finish().await;
}

#[tokio::test]
async fn websocket_delayed_quota_does_not_move_opaque_continuation_or_remote_tool_owner() {
    for remote_tool in [false, true] {
        let fixture = Fixture::start(gate(true, None, false, false), policy(), false).await;
        let mut body = request();
        if remote_tool {
            body["tools"] =
                json!([{"type":"file_search","vector_store_ids":["account-owned-store"]}]);
        } else {
            let store = IdentityStore::open(fixture.path.join("identity.sqlite")).unwrap();
            let mut seed = request();
            let mut headers = HeaderMap::new();
            let context = store.inspect("caller", &headers, &seed).unwrap();
            let binding = store
                .rewrite(&context, "a", "owner-a", &mut headers, &mut seed)
                .unwrap();
            store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"opaque-previous","status":"completed","output":[]}})).unwrap();
            body["previous_response_id"] = json!("opaque-previous");
        }
        let mut socket = connect(&fixture).await;
        send(&mut socket, body).await;
        let events = terminal(&mut socket).await;
        assert_eq!(events.last().unwrap()["status"], 429);
        assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
        let _ = socket.close(None).await;
        fixture.finish().await;
    }
}

#[tokio::test]
async fn websocket_candidate_with_no_upstream_send_has_no_failure_or_attempt_budget() {
    let fixture = Fixture::start(
        Behavior::Success,
        ExecutionPolicy {
            credential_retries: 1,
            send_retries: 0,
            status_retries: 0,
            bootstrap_retries: 0,
            ..policy()
        },
        true,
    )
    .await;
    let mut socket = connect(&fixture).await;
    send(&mut socket, long_conversation()).await;
    assert_eq!(
        terminal(&mut socket).await.last().unwrap()["type"],
        "response.completed"
    );
    assert_eq!(fixture.first.attempts.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
    assert!(fixture.host.failures.lock().unwrap().is_empty());
    let host = fixture.host.clone();
    let _ = socket.close(None).await;
    fixture.finish().await;
    let reports = host.reports.lock().unwrap();
    assert_eq!(reports.len(), 1);
    assert!(reports[0].success);
    assert_eq!(reports[0].diagnostics.as_ref().unwrap().attempts.len(), 1);
}

#[tokio::test]
async fn websocket_metadata_fence_stays_bounded_after_visible_handshake() {
    for business in [
        json!({"type":"response.metadata","metadata":{"padding":"x".repeat(super::super::super::bootstrap::MAX_BYTES)}}),
        Value::Array(
            (0..17)
                .map(|_| json!({"type":"response.metadata","metadata":{}}))
                .collect(),
        ),
    ] {
        for reused in [false, true] {
            let fixture = Fixture::start(
                gate(false, Some(business.clone()), reused, false),
                policy(),
                false,
            )
            .await;
            let mut socket = connect(&fixture).await;
            if reused {
                send(&mut socket, request()).await;
                terminal(&mut socket).await;
            }
            send(&mut socket, request()).await;
            let events = terminal(&mut socket).await;
            assert_eq!(events.last().unwrap()["status"], 429);
            assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
            let _ = socket.close(None).await;
            fixture.finish().await;
        }
    }
}

pub(super) async fn http_endpoint(
    state: Arc<Upstream>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst);
    state.requests.lock().unwrap().push((headers, body));
    let response_id = format!(
        "http-{}-{attempt}",
        if matches!(state.behavior, Behavior::Success) {
            "backup"
        } else {
            "source"
        }
    );
    let mut events =
        vec![json!({"type":"response.created","response":{"id":response_id,"output":[]}})];
    let mut rejected = false;
    if let Behavior::QuotaGate {
        metadata,
        business,
        reused,
        ..
    } = &state.behavior
    {
        if !reused || attempt > 0 {
            rejected = true;
            if *metadata {
                events
                    .push(json!({"type":"response.metadata","metadata":{"request_id":"fixture"}}));
            }
            if let Some(business) = business {
                events.push(business.clone());
            }
        }
    }
    let final_event = if rejected {
        json!({"type":"error","status":429,"error":{"code":"subscription_sharing_usage_limit_exceeded","type":"usage_limit_reached","message":"Synthetic quota exhausted","resets_in_seconds":60}})
    } else {
        json!({"type":"response.completed","response":{"id":response_id,"status":"completed","output":[]}})
    };
    let (sender, receiver) =
        tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(8);
    tokio::spawn(async move {
        for event in events {
            if sender
                .send(Ok(format!("data: {event}\n\n").into()))
                .await
                .is_err()
            {
                return;
            }
        }
        if rejected {
            tokio::time::sleep(Duration::from_millis(650)).await;
        }
        let _ = sender
            .send(Ok(format!("data: {final_event}\n\n").into()))
            .await;
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

#[tokio::test]
async fn websocket_http_bridge_delayed_quota_switches_but_never_replays_business() {
    for reused in [false, true] {
        for business in [
            None,
            Some(json!({"type":"response.output_text.delta","delta":"visible"})),
            Some(
                json!({"type":"response.output_item.added","item":{"type":"function_call","name":"write_file","call_id":"tool","arguments":"{}"}}),
            ),
        ] {
            let fixture =
                Fixture::start(gate(true, business.clone(), reused, false), policy(), false).await;
            fixture.host.passthrough.store(false, Ordering::Release);
            let mut socket = connect(&fixture).await;
            if reused {
                send(&mut socket, request()).await;
                terminal(&mut socket).await;
            }
            send(&mut socket, long_conversation()).await;
            let events = terminal(&mut socket).await;
            if business.is_none() {
                assert_eq!(
                    events.last().unwrap()["type"],
                    "response.completed",
                    "{events:?}"
                );
                assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 1);
                assert_eq!(
                    fixture.second.requests.lock().unwrap()[0].1["input"]
                        .as_array()
                        .unwrap()
                        .len(),
                    181
                );
            } else {
                assert_eq!(events.last().unwrap()["status"], 429);
                assert_eq!(fixture.second.attempts.load(Ordering::SeqCst), 0);
            }
            assert_eq!(
                fixture.first.attempts.load(Ordering::SeqCst),
                if reused { 2 } else { 1 }
            );
            let _ = socket.close(None).await;
            fixture.finish().await;
        }
    }
}
