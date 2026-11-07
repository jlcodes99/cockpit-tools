//! Exercise legacy wire history through native HTTP and the actual Go bridge protocol.
use super::*;

fn legacy_history(large: bool) -> Value {
    let mut input = history();
    let text = if large {
        "x".repeat(9 * 1024 * 1024)
    } else {
        "continue".into()
    };
    input.as_array_mut().unwrap().extend([
        json!({"type":"local_shell_call","id":"legacy-shell","call_id":"shell-call","status":"completed","action":{"type":"exec","command":["echo","ok"]}}),
        json!({"type":"function_call_output","call_id":"shell-call","output":"ok"}),
        json!({"type":"function_call","id":"fc_bad-arguments","namespace":"collaboration","name":"spawn_agent","call_id":"error-call","arguments":"{\"message\":"}),
        json!({"type":"function_call_output","call_id":"error-call","output":"Error parsing function arguments: EOF while parsing an object"}),
        json!({"type":"agent_message","id":"amsg-parent","author":"/root/worker","recipient":"/root","content":[{"type":"encrypted_content","encrypted_content":"synthetic-agent-cipher"},{"type":"input_text","text":"Delivered by the worker"}]}),
        message("user", &text),
    ]);
    input
}

#[tokio::test]
async fn legacy_history_survives_native_go_restart_account_switch_and_parent_child_followups() {
    let mut fixture = Fixture::new(true).await;
    let mut original = initial();
    original["input"] = legacy_history(true);
    let saved = original.clone();
    let first = fixture.complete(&original).await;
    let parent_a = fixture.sent()[0].0.clone();
    let mut child = request(json!([
        original["input"][10].clone(),
        message("user", "continue the child task")
    ]));
    child["client_metadata"] =
        json!({"session_id":"root","thread_id":"child","x-codex-parent-thread-id":"root"});
    let child_saved = child.clone();
    let child_first = fixture.complete(&child).await;
    let child_a = fixture.sent()[1].0.clone();
    assert_eq!(parent_a["session-id"], child_a["session-id"]);
    assert_ne!(parent_a["thread-id"], child_a["thread-id"]);

    fixture.stop_native().await;
    let bridge = fixture.bridge();
    let source = delta(
        first["id"].as_str().unwrap(),
        "tool error handled; Rust to Go",
    );
    let prepared = bridge_prepare(&bridge, "legacy-go", source.clone()).await;
    let bound = bridge_bind(&bridge, &prepared, true, source.clone()).await;
    let expected_go = append(
        append(
            original["input"].clone(),
            first["output"].as_array().unwrap().clone(),
        ),
        source["input"].as_array().unwrap().clone(),
    );
    assert_eq!(bound.body["input"], expected_go);
    assert!(bound.body.get("previous_response_id").is_none());
    for key in ["session-id", "thread-id"] {
        assert_eq!(bound.headers[key], vec![parent_a[key].to_str().unwrap()]);
    }
    // Exercise a real HTTP transmission and observation, rather than inserting
    // a checkpoint row. The Go SDK executor is covered separately by Go tests.
    let raw = client()
        .post(format!("{}/responses", fixture.host.base))
        .json(&bound.body)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let event: Value = serde_json::from_str(raw.trim().strip_prefix("data: ").unwrap()).unwrap();
    let go_response = event["response"].clone();
    bridge_complete(&bridge, &prepared, event).await;
    bridge.stop().await.unwrap();

    fixture.host.removed_a.store(true, Ordering::SeqCst);
    fixture.start(Options::default()).await;
    let back = delta(go_response["id"].as_str().unwrap(), "Go to Rust on B");
    let b = fixture.complete(&back).await;
    let expected_b = append(
        append(
            expected_go,
            go_response["output"].as_array().unwrap().clone(),
        ),
        back["input"].as_array().unwrap().clone(),
    );
    assert_eq!(fixture.sent()[3].1["input"], expected_b);
    let parent_b = fixture.sent()[3].0.clone();
    for key in ["session-id", "thread-id"] {
        assert_ne!(parent_a[key], parent_b[key]);
    }
    assert!(fixture.sent()[3].1.get("previous_response_id").is_none());

    let mut child_next = delta(
        child_first["id"].as_str().unwrap(),
        "child follows parent onto B",
    );
    child_next["client_metadata"] = child["client_metadata"].clone();
    let child_second = fixture.complete(&child_next).await;
    let child_b = fixture.sent()[4].0.clone();
    // Parent and child rebuild independent upstream conversations on B. Their
    // generations may have different sessions; the parent reference must resolve
    // to B's actual parent thread rather than the child's mapping namespace.
    assert_ne!(child_a["session-id"], child_b["session-id"]);
    assert_ne!(parent_b["thread-id"], child_b["thread-id"]);
    assert_ne!(child_a["thread-id"], child_b["thread-id"]);
    assert_eq!(
        fixture.sent()[4].1["client_metadata"]["x-codex-parent-thread-id"],
        parent_b["thread-id"].to_str().unwrap()
    );
    assert_eq!(
        fixture.sent()[4].1["input"],
        append(
            append(
                child["input"].clone(),
                child_first["output"].as_array().unwrap().clone()
            ),
            child_next["input"].as_array().unwrap().clone(),
        )
    );

    fixture.stop_native().await;
    fixture.start(Options::default()).await;
    let restarted = delta(b["id"].as_str().unwrap(), "B after another restart");
    fixture.complete(&restarted).await;
    assert_eq!(
        fixture.sent()[5].1["input"],
        append(
            append(expected_b, b["output"].as_array().unwrap().clone()),
            restarted["input"].as_array().unwrap().clone(),
        )
    );
    for key in ["session-id", "thread-id"] {
        assert_eq!(parent_b[key], fixture.sent()[5].0[key]);
    }
    let mut child_restarted = delta(child_second["id"].as_str().unwrap(), "child after restart");
    child_restarted["client_metadata"] = child["client_metadata"].clone();
    fixture.complete(&child_restarted).await;
    for key in ["session-id", "thread-id"] {
        assert_eq!(child_b[key], fixture.sent()[6].0[key]);
    }
    assert_eq!(
        fixture.sent()[6].1["client_metadata"]["x-codex-parent-thread-id"],
        parent_b["thread-id"].to_str().unwrap()
    );
    assert_eq!(
        original, saved,
        "the caller's saved history must remain unchanged"
    );
    assert_eq!(child, child_saved);

    let count = fixture.sent().len();
    let foreign = fixture.send(&original, "foreign").await;
    assert_eq!(foreign.status(), 409);
    assert_eq!(
        foreign.json::<Value>().await.unwrap()["error"]["code"],
        "upstream_state_caller_mismatch"
    );
    assert_eq!(
        fixture.sent().len(),
        count,
        "foreign history must never reach the upstream"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn legacy_tool_history_leaves_missing_wrong_and_duplicate_results_to_upstream() {
    let upstream_error = json!({
        "type":"invalid_request_error",
        "code":"invalid_request_error",
        "message":"Synthetic upstream rejected the legacy tool result pairing",
        "param":"input",
        "upstream_marker":"original-legacy-tool-diagnostic"
    });
    let source = Arc::new(Upstream::default());
    let error = upstream_error.clone();
    let app = Router::new()
        .route(
            "/responses",
            post(move |State(source): State<Arc<Upstream>>, headers: HeaderMap, bytes: Bytes| {
                let error = error.clone();
                async move {
                    let request: Value = serde_json::from_slice(&bytes).unwrap();
                    source.sent.lock().unwrap().push((headers, request));
                    Response::builder()
                        .status(400)
                        .header("content-type", "application/json")
                        .body(Body::from(json!({"error":error}).to_string()))
                        .unwrap()
                }
            }),
        )
        .with_state(source.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = Arc::new(TestHost {
        base: format!("http://{}", listener.local_addr().unwrap()),
        oauth: true,
        removed_a: AtomicBool::new(false),
    });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut fixture = Fixture {
        root: tempfile::tempdir().unwrap(),
        host,
        source,
        gateway: None,
        task,
    };
    fixture.start(Options::default()).await;
    let mut attempts = 0;
    for index in [6, 8] {
        for defect in ["missing", "wrong-id", "duplicate"] {
            let mut body = request(legacy_history(false));
            body["stream"] = json!(false);
            let input = body["input"].as_array_mut().unwrap();
            match defect {
                "missing" => {
                    input.remove(index + 1);
                }
                "wrong-id" => input[index + 1]["call_id"] = json!("orphan-call"),
                _ => {
                    input.push(input[index + 1].clone());
                }
            }
            let saved = body.clone();
            let response = fixture.send(&body, "caller").await;
            assert_eq!(response.status(), 400, "{index}: {defect}");
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"],
                upstream_error,
                "{index}: {defect}"
            );
            let sent = fixture.sent();
            attempts += 1;
            assert_eq!(sent.len(), attempts, "ordinary forwarding must not retry");
            assert_eq!(sent[attempts - 1].1["input"], saved["input"]);
            assert_eq!(body, saved);
            let db = Connection::open(fixture.path()).unwrap();
            for table in [
                "native_opaque_v1",
                "native_rebuild_active_v1",
                "native_rebuild_checkpoints_v1",
                "native_rebuild_history_v1",
            ] {
                let count: i64 = db
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
                    .unwrap();
                assert_eq!(count, 0, "rejected {index}: {defect} published {table}");
            }
        }
    }
    fixture.finish().await;
}

#[tokio::test]
async fn client_error_results_make_raw_tool_history_replayable_after_restart() {
    let mut fixture = Fixture::new(true).await;
    let call = json!({"type":"function_call","id":"fc_bad-call-item","namespace":"collaboration","name":"spawn_agent","call_id":"bad-call","arguments":"{\"message\":"});
    *fixture.source.next_output.lock().unwrap() = Some(vec![call.clone()]);
    let original = initial();
    let first = fixture.complete(&original).await;
    assert_eq!(first["output"], json!([call.clone()]));
    // Raw unparsable output alone must not publish a continuation checkpoint.
    let archived: i64 = Connection::open(fixture.path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_history_v1 WHERE response=?1",
            [first["id"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 0);
    // A reconnect supplies full history after the client has handled the parser
    // error. That matching result makes the old call replayable without editing it.
    let mut follow = delta(first["id"].as_str().unwrap(), "continue after tool error");
    follow["input"].as_array_mut().unwrap().insert(0,
        json!({"type":"function_call_output","call_id":"bad-call","output":"Error parsing function arguments: EOF while parsing an object"}));
    follow["input"] = append(
        append(original["input"].clone(), [call.clone()]),
        follow["input"].as_array().unwrap().clone(),
    );
    follow
        .as_object_mut()
        .unwrap()
        .remove("previous_response_id");
    fixture.stop_native().await;
    fixture.host.removed_a.store(true, Ordering::SeqCst);
    fixture.start(Options::default()).await;
    let next = fixture.complete(&follow).await;
    let expected = follow["input"].clone();
    assert_eq!(fixture.sent()[1].1["input"], expected);
    assert!(fixture.sent()[1].1.get("previous_response_id").is_none());
    fixture
        .complete(&delta(next["id"].as_str().unwrap(), "one more followup"))
        .await;
    assert_eq!(fixture.generation().0, "b");
    fixture.finish().await;
}
