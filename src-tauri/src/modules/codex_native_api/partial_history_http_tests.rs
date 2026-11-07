//! Real native gateway, synthetic OAuth accounts and loopback upstream only.
use super::*;

fn quota() -> Step {
    Step::RawEvents(vec![json!({"type":"error","status":429,"error":{
        "code":"usage_limit_reached","message":"Synthetic quota exhausted"}})])
}
async fn terminal(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let frame = socket
                .next()
                .await
                .expect("socket closed before terminal")
                .unwrap();
            if let Message::Text(text) = frame {
                let event: Value = serde_json::from_str(&text).unwrap();
                if matches!(
                    event["type"].as_str(),
                    Some(
                        "error" | "response.failed" | "response.completed" | "response.incomplete"
                    )
                ) {
                    return event;
                }
            }
        }
    })
    .await
    .unwrap()
}

async fn scenario(case: &str, passthrough: bool) {
    let call = json!({"type":"function_call","name":"synthetic_write","call_id":"once","arguments":"{\"value\":1}","status":"completed"});
    let result =
        json!({"type":"function_call_output","call_id":"once","output":"already executed once"});
    let output = if case.starts_with("tool") {
        vec![call.clone()]
    } else if matches!(case, "empty" | "missing_output") {
        vec![]
    } else {
        vec![message("assistant", "partial answer")]
    };
    let mut events = vec![
        json!({"type":"response.created","response":{"id":"a-partial","status":"in_progress","output":[]}}),
    ];
    if case == "conflict" {
        events.push(json!({"type":"response.output_item.done","output_index":0,"item":message("assistant","different output")}));
    } else if case.starts_with("tool") {
        events.push(json!({"type":"response.output_item.done","output_index":0,"item":call}));
    } else if case != "empty" {
        events.push(json!({"type":"response.output_text.delta","delta":"partial answer"}));
    }
    events.push(json!({"type":"response.incomplete","response":{"id":"a-partial","status":"incomplete","incomplete_details":{"reason":"interrupted"},"output":output}}));
    let failed_retry = case == "failed_retry";
    let b = if failed_retry {
        vec![
            quota(),
            Step::Complete("b-success", None),
            Step::Complete("b-next", None),
        ]
    } else {
        vec![
            Step::Complete("b-success", None),
            Step::Complete("b-next", None),
        ]
    };
    let mut fixture = Fixture::new(
        true,
        vec![
            Step::Complete("a-root", None),
            Step::RawEvents(events),
            quota(),
            quota(),
        ],
        b,
    )
    .await;
    fixture.host.oauth.store(true, Ordering::SeqCst);
    fixture
        .host
        .passthrough
        .store(passthrough, Ordering::SeqCst);
    let mut socket = fixture.socket().await;
    let mut root = body(vec![message("user", "remember ORIGINAL-731")], None, true);
    if case == "parameters" {
        root["instructions"] = json!("keep the original instructions");
        root["tools"] =
            json!([{"type":"function","name":"synthetic_write","parameters":{"type":"object"}}]);
    }
    ws_send(&mut socket, root).await;
    assert_eq!(terminal(&mut socket).await["type"], "response.completed");
    ws_send(
        &mut socket,
        body(vec![message("user", "second")], Some("a-root"), true),
    )
    .await;
    assert_eq!(terminal(&mut socket).await["type"], "response.incomplete");
    let mut input = vec![];
    if case == "tool_paired" {
        input.push(result.clone());
    }
    input.push(message("user", "continue"));
    let next = body(input, Some("a-partial"), true);
    if case == "restart" {
        drop(socket);
        fixture
            .handle
            .shutdown(Duration::from_secs(3))
            .await
            .unwrap();
        fixture.handle = start(
            "127.0.0.1:0".parse().unwrap(),
            fixture.host.clone(),
            fixture.path(),
            Options::default(),
        )
        .await
        .unwrap();
        socket = fixture.socket().await;
    } else if case == "reconnect" {
        drop(socket);
        socket = fixture.socket().await;
    }
    ws_send(&mut socket, next.clone()).await;
    let mut event = terminal(&mut socket).await;
    if failed_retry {
        assert_eq!(event["error"]["code"], "usage_limit_reached");
        let count: i64 = Connection::open(fixture.path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_partial_history_v1 WHERE response='a-partial'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "failed candidates must never consume the partial history"
        );
        drop(socket);
        socket = fixture.socket().await;
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        ws_send(&mut socket, next).await;
        event = terminal(&mut socket).await;
    }
    let recover = !matches!(case, "tool_unpaired" | "conflict" | "missing_output");
    let sent = fixture.sent();
    let backup = sent.iter().find(|s| s.account == "b");
    if recover {
        assert_eq!(
            event["type"], "response.completed",
            "{case}/{passthrough}: {event}"
        );
        let request = &backup.expect("must switch to backup").body;
        assert!(request.get("previous_response_id").is_none());
        let input = request["input"].as_array().unwrap();
        assert!(input
            .iter()
            .any(|item| item.to_string().contains("ORIGINAL-731")));
        assert!(input.iter().any(|item| item.to_string().contains("second")));
        if matches!(
            case,
            "text" | "reconnect" | "restart" | "failed_retry" | "parameters"
        ) {
            assert!(input
                .iter()
                .any(|item| item.to_string().contains("partial answer")));
        }
        if case == "tool_paired" {
            assert_eq!(
                input
                    .iter()
                    .filter(|item| item["type"] == "function_call" && item["call_id"] == "once")
                    .count(),
                1
            );
            assert_eq!(
                input
                    .iter()
                    .filter(
                        |item| item["type"] == "function_call_output" && item["call_id"] == "once"
                    )
                    .count(),
                1
            );
            assert!(input.iter().any(|item| item["output"] == result["output"]));
        }
        if case == "parameters" {
            assert_eq!(request["instructions"], "keep the original instructions");
            assert_eq!(request["tools"][0]["name"], "synthetic_write");
        }
        use sha2::Digest;
        let connection = Connection::open(fixture.path()).unwrap();
        let owner:String=connection.query_row("SELECT account FROM native_opaque_v1 WHERE caller='caller' AND kind='parent_response' AND digest=?1",[format!("{:x}",sha2::Sha256::digest(b"a-partial"))],|r|r.get(0)).unwrap();
        assert_eq!(owner, "a");
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM native_rebuild_history_v1 WHERE response='a-partial'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "partial response must not become a successful checkpoint"
        );
        ws_send(
            &mut socket,
            body(
                vec![message("user", "next after recovery")],
                Some("b-success"),
                true,
            ),
        )
        .await;
        assert_eq!(terminal(&mut socket).await["response"]["id"], "b-next");
        assert_eq!(fixture.sent().last().unwrap().account, "b");
    } else {
        assert_eq!(
            event["error"]["code"], "usage_limit_reached",
            "{case}/{passthrough}: {event}"
        );
        assert!(backup.is_none());
    }
    drop(socket);
    fixture.finish().await;
}

#[tokio::test]
async fn partial_history_text_switches_and_keeps_next_turn() {
    scenario("text", true).await;
}
#[tokio::test]
async fn partial_history_empty_interruption_recovers() {
    scenario("empty", true).await;
}
#[tokio::test]
async fn partial_history_paired_tools_are_not_duplicated() {
    scenario("tool_paired", true).await;
}
#[tokio::test]
async fn partial_history_unpaired_tools_stay_on_owner() {
    scenario("tool_unpaired", true).await;
}
#[tokio::test]
async fn partial_history_conflicting_output_stays_on_owner() {
    scenario("conflict", true).await;
}
#[tokio::test]
async fn partial_history_missing_output_stays_on_owner() {
    scenario("missing_output", true).await;
}
#[tokio::test]
async fn partial_history_reconnect_recovers() {
    scenario("reconnect", true).await;
}
#[tokio::test]
async fn partial_history_service_restart_recovers() {
    scenario("restart", true).await;
}
#[tokio::test]
async fn partial_history_failed_retry_preserves_recovery() {
    scenario("failed_retry", true).await;
}
#[tokio::test]
async fn partial_history_omitted_parameters_are_inherited() {
    scenario("parameters", true).await;
}
#[tokio::test]
async fn partial_history_http_bridge_recovers() {
    scenario("text", false).await;
}
#[tokio::test]
async fn partial_history_http_bridge_reconnect_recovers() {
    scenario("reconnect", false).await;
}
