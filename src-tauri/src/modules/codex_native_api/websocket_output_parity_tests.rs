#[tokio::test]
async fn websocket_json_failure_terminals_reach_client_without_committing_success() {
    for status in ["failed", "incomplete", "cancelled"] {
        let original = json!({"id":format!("json-{status}"),"status":status,"output":[{"type":"reasoning","encrypted_content":"opaque-json","summary":[]}],"error":if status=="failed" {json!({"code":"server_error","message":"Failed upstream"})} else {Value::Null},"incomplete_details":if status=="incomplete" {json!({"reason":"max_output_tokens"})} else {Value::Null},"usage":{"input_tokens":3,"output_tokens":2},"extension":{"unchanged":true}});
        let body = original.clone();
        let app = Router::new().route(
            "/v1/responses",
            post(move || {
                let body = body.clone();
                async move { Json(body) }
            }),
        );
        let (mut handle, host, task, dir) =
            fixture(app, WireApi::Responses, false, native::Options::default()).await;
        let mut ws = socket(&handle).await;
        send(
            &mut ws,
            json!({"type":"response.create","model":"m","input":[]}),
        )
        .await;
        let mut events = Vec::new();
        loop {
            let event = next_output_event(&mut ws).await;
            let done = event["type"] == format!("response.{status}");
            events.push(event);
            if done {
                break;
            }
            assert!(
                events.len() < 4,
                "failed to deliver {status} terminal: {events:?}"
            );
        }
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["type"], "response.created");
        assert_eq!(events[0]["response"]["status"], "in_progress");
        assert!(native::events::response_terminal_kind(&events[0]).is_none());
        assert_eq!(events[1]["item"], original["output"][0]);
        let mut expected = original;
        expected["usage"]["input_tokens_details"] = json!({"cached_tokens":0});
        expected["usage"]["output_tokens_details"] = json!({"reasoning_tokens":0});
        assert_eq!(events[2]["response"], expected);
        reports(&host, 1).await;
        {
            let reports = host.reports.lock().unwrap();
            assert!(!reports[0].success);
            assert_eq!(reports[0].usage.as_ref().unwrap()["output_tokens"], 2);
        }
        let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
        let ids: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ids, 0, "{status} must not become a successful checkpoint");
        drop(connection);
        ws.close(None).await.unwrap();
        handle.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

async fn next_output_event(ws: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(value) => ws.send(Message::Pong(value)).await.unwrap(),
                other => panic!("unexpected downstream frame: {other:?}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn websocket_completion_output_matches_observed_tools_and_resets_between_turns() {
    use axum::extract::ws::{Message as AxumMessage, WebSocketUpgrade};
    let app = Router::new().route("/v1/responses", get(|upgrade: WebSocketUpgrade| async {
        upgrade.on_upgrade(|mut ws| async move {
            let call = json!({"type":"custom_tool_call","call_id":"call-1","name":"exec","input":"pwd","extension":true});
            let reasoning = json!({"type":"reasoning","encrypted_content":"opaque-ws","summary":[]});
            for turn in 0..3 {
                let Some(Ok(AxumMessage::Text(_))) = ws.next().await else { return; };
                let id = format!("ws-completed-{turn}");
                let mut events = vec![json!({"type":"response.created","response":{"id":id,"status":"in_progress","output":[]}})];
                if turn < 2 {
                    events.extend([
                        json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
                        json!({"type":"response.output_item.done","output_index":1,"item":call}),
                    ]);
                }
                let output = if turn == 1 { json!([reasoning,{"type":"function_call","call_id":"call-1","name":"exec"}]) } else { json!([]) };
                events.push(json!({"type":if turn==1 {"response.done"} else {"response.completed"},"response":{"id":id,"status":"completed","output":output,"usage":{"input_tokens":3,"output_tokens":2},"extension":true}}));
                for event in events {
                    ws.send(AxumMessage::Text(event.to_string().into())).await.unwrap();
                }
            }
            let _ = ws.next().await;
        })
    }));
    let (mut handle, host, task, _dir) =
        fixture(app, WireApi::Responses, true, native::Options::default()).await;
    let mut ws = socket(&handle).await;
    for turn in 0..3 {
        send(
            &mut ws,
            json!({"type":"response.create","model":"m","input":[]}),
        )
        .await;
        let response = terminal(&mut ws).await;
        assert_eq!(response["response"]["id"], format!("ws-completed-{turn}"));
        assert_eq!(response["response"]["usage"]["output_tokens"], 2);
        assert_eq!(
            response["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
            0
        );
        assert_eq!(
            response["response"]["usage"]["input_tokens_details"]["cached_tokens"],
            0
        );
        assert_eq!(response["response"]["extension"], true);
        if turn < 2 {
            assert_eq!(response["response"]["output"].as_array().unwrap().len(), 2);
            assert_eq!(
                response["response"]["output"][0]["encrypted_content"],
                "opaque-ws"
            );
            assert_eq!(
                response["response"]["output"][1]["type"],
                "custom_tool_call"
            );
            assert_eq!(response["response"]["output"][1]["input"], "pwd");
        } else {
            assert_eq!(response["response"]["output"], json!([]));
        }
    }
    reports(&host, 3).await;
    assert!(host
        .reports
        .lock()
        .unwrap()
        .iter()
        .all(|report| report.success));
    ws.close(None).await.unwrap();
    handle.shutdown(Duration::from_secs(2)).await.unwrap();
    task.abort();
}
