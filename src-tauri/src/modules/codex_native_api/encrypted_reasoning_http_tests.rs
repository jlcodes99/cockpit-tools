//! Real retry/mapping/storage paths with synthetic encrypted data and loopback.
use super::*;

pub(super) fn rejection(request: &Value, http: bool, prefix: Option<Value>) -> Response {
    let item = &request["input"][1];
    let error = json!({"code":"invalid_encrypted_content","type":"invalid_request_error",
        "param":"input[1].encrypted_content","message":format!("The encrypted content for item {} could not be verified. Reason: Encrypted content could not be decrypted or parsed.",item["id"].as_str().unwrap_or("unknown"))});
    if http {
        (StatusCode::BAD_REQUEST, Json(json!({"error":error}))).into_response()
    } else {
        if prefix
            .as_ref()
            .is_some_and(|event| event["type"] == "response.created")
        {
            let first = Bytes::from(format!("data: {}\n\n", prefix.unwrap()));
            let last = Bytes::from(format!(
                "data: {}\n\n",
                json!({"type":"error","status":400,"error":error})
            ));
            let frames = stream::once(async move { Ok::<_, std::io::Error>(first) }).chain(
                stream::once(async move {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok::<_, std::io::Error>(last)
                }),
            );
            return Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(frames))
                .unwrap();
        }
        let mut events = prefix.into_iter().collect::<Vec<_>>();
        events.push(json!({"type":"error","status":400,"error":error}));
        sse(events)
    }
}
async fn terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let frame = socket.next().await.expect("client socket closed").unwrap();
            if let Message::Text(text) = frame {
                let event: Value = serde_json::from_str(&text).unwrap();
                if matches!(
                    event["type"].as_str(),
                    Some("response.completed" | "response.failed" | "error")
                ) {
                    return event;
                }
            }
        }
    })
    .await
    .unwrap()
}
async fn run(mode: &str, failure: &str) {
    let websocket = mode.starts_with("ws");
    let reused = mode == "ws_reused";
    let http = mode == "http_status";
    let prefix = (failure == "after_output")
        .then(|| json!({"type":"response.output_text.delta","delta":"already generated"}));
    let prefix = if failure == "late" {
        Some(
            json!({"type":"response.created","response":{"id":"rejected-metadata","status":"in_progress","output":[]}}),
        )
    } else {
        prefix
    };
    let mut steps = vec![];
    if reused {
        steps.push(Step::Complete("root", None));
    }
    steps.push(Step::CipherReject { http, prefix });
    if failure == "twice" {
        steps.push(Step::CipherReject { http, prefix: None });
    } else {
        steps.push(Step::Complete("recovered", None));
    }
    let fixture = Fixture::new(true, steps, vec![]).await;
    fixture.host.oauth.store(true, Ordering::SeqCst);
    fixture.host.passthrough.store(websocket, Ordering::SeqCst);
    let cipher = crate::modules::codex_native_api::codex_cpa_wire::synthetic_gpt_signature(32);
    let mut input = vec![
        message("user", "remember original"),
        json!({"type":"reasoning","id":"rs_bad","encrypted_content":cipher,"summary":[{"type":"summary_text","text":"visible summary"}]}),
        json!({"type":"function_call","name":"write","call_id":"once","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"once","output":"already executed once"}),
    ];
    if failure == "compaction" {
        input[1]["type"] = json!("compaction");
    }
    let request = body(input, None, mode != "http_status");
    let original = request.clone();
    let mut socket = if websocket {
        Some(fixture.socket().await)
    } else {
        None
    };
    if reused {
        ws_send(
            socket.as_mut().unwrap(),
            body(vec![message("user", "initial")], None, true),
        )
        .await;
        assert_eq!(
            terminal(socket.as_mut().unwrap()).await["type"],
            "response.completed"
        );
    }
    let event = if let Some(socket) = &mut socket {
        ws_send(socket, request.clone()).await;
        terminal(socket).await
    } else {
        let response = fixture.post(&request, None).await;
        if mode == "http_status" || !response.status().is_success() {
            let value: Value = response.json().await.unwrap();
            if value.get("error").is_some() {
                json!({"type":"error","error":value["error"]})
            } else {
                json!({"type":"response.completed","response":value})
            }
        } else {
            let text = response.text().await.unwrap();
            text.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
                .filter(|e| {
                    matches!(
                        e["type"].as_str(),
                        Some("response.completed" | "response.failed" | "error")
                    )
                })
                .last()
                .expect("terminal SSE")
        }
    };
    let sent = fixture.sent();
    let offset = usize::from(reused);
    assert!(sent.iter().all(|s| s.account == "a"));
    assert_eq!(request, original);
    if matches!(failure, "none" | "late") {
        assert_eq!(event["type"], "response.completed", "{mode}: {event}");
        assert_eq!(sent.len(), offset + 2);
        let repaired = &sent[offset + 1].body["input"];
        assert!(!repaired.to_string().contains(&cipher));
        assert!(repaired.to_string().contains("visible summary"));
        assert_eq!(
            repaired[0]["content"],
            sent[offset].body["input"][0]["content"]
        );
        for kind in ["function_call", "function_call_output"] {
            assert_eq!(
                repaired
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|i| i["type"] == kind)
                    .count(),
                1
            );
        }
        let db = Connection::open(fixture.path()).unwrap();
        let saved: String = db
            .query_row(
                "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE response='recovered'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !saved.contains(&cipher),
            "rejected cipher must not become accepted replay history"
        );
        assert!(saved.contains("visible summary"));
        let failure_count = fixture.host.failures.lock().unwrap().len();
        assert_eq!(
            failure_count, 0,
            "request-scoped rejection must not cool the account"
        );
    } else {
        assert_eq!(event["type"], "error", "{mode}/{failure}: {event}");
        assert_eq!(event["error"]["code"], "invalid_encrypted_content");
        assert_eq!(
            sent.len(),
            offset + if failure == "twice" { 2 } else { 1 },
            "never repeat after output or remove compaction"
        );
    }
    drop(socket);
    fixture.finish().await;
}
#[tokio::test]
async fn encrypted_reasoning_http_status_recovers() {
    run("http_status", "none").await;
}
#[tokio::test]
async fn encrypted_reasoning_sse_recovers() {
    run("sse", "none").await;
}
#[tokio::test]
async fn encrypted_reasoning_websocket_opening_recovers() {
    run("ws", "none").await;
}
#[tokio::test]
async fn encrypted_reasoning_websocket_reused_recovers() {
    run("ws_reused", "none").await;
}
#[tokio::test]
async fn encrypted_reasoning_retries_only_once_on_all_transports() {
    for mode in ["http_status", "sse", "ws", "ws_reused"] {
        run(mode, "twice").await;
    }
}
#[tokio::test]
async fn encrypted_reasoning_never_replays_after_output() {
    for mode in ["sse", "ws", "ws_reused"] {
        run(mode, "after_output").await;
    }
}
#[tokio::test]
async fn encrypted_reasoning_never_discards_compaction() {
    for mode in ["http_status", "sse", "ws"] {
        run(mode, "compaction").await;
    }
}

#[tokio::test]
async fn encrypted_reasoning_late_error_after_metadata_still_recovers() {
    for mode in ["sse", "ws", "ws_reused"] {
        run(mode, "late").await;
    }
}
