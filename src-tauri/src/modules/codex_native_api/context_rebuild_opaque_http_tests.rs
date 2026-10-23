//! Explicit client history is replayed byte-for-value; it never mints unknown ciphertext.
use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
enum Transport {
    Json,
    Sse,
    WebSocket,
}
impl Transport {
    fn streaming(self) -> bool {
        !matches!(self, Self::Json)
    }
}

fn opaque_output(account: &str) -> Vec<Value> {
    let mut output = vec![
        message("assistant", &format!("answer-{account}")),
        json!({"type":"reasoning","id":format!("reasoning-item-{account}"),"summary":[{"type":"summary_text","text":"Synthetic visible summary"}],"encrypted_content":format!("synthetic-reasoning-{account}"),"status":"completed"}),
        json!({"type":"compaction","id":format!("compaction-item-{account}"),"encrypted_content":format!("synthetic-compaction-{account}")}),
    ];
    for tool in ["spawn_agent", "send_message", "followup_task"] {
        output.push(json!({"type":"function_call","id":format!("communication-item-{account}-{tool}"),"namespace":"collaboration","name":tool,"call_id":format!("call-{account}-{tool}"),"arguments":json!({"message":format!("synthetic-message-{account}-{tool}"),"task_name":"worker","target":"/root/worker","extension":{"exact":"keep"}}).to_string(),"encrypted_function_args":[format!("synthetic-args-{account}-{tool}")],"status":"completed"}));
    }
    output
}
fn root_request(streaming: bool) -> Value {
    let mut first = message("user", "first");
    first["id"] = json!("synthetic-client-first-item");
    body(vec![first], None, streaming)
}
fn append_full(previous: &Value, output: &[Value], turn: &str) -> Vec<Value> {
    let mut input = previous["input"].as_array().unwrap().clone();
    input.extend_from_slice(output);
    for item in output.iter().filter(|item| item["type"] == "function_call") {
        input.push(json!({"type":"function_call_output","id":format!("result-{}",item["call_id"].as_str().unwrap()),"call_id":item["call_id"],"output":"Synthetic delivered","extension":42}));
    }
    input.push(json!({"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"Payload:\n"},{"type":"encrypted_content","encrypted_content":"synthetic-message-a-spawn_agent"}],"extension":"agent-kept"}));
    input.push(message("user", turn));
    input
}
async fn turn(
    fixture: &Fixture,
    socket: &mut Option<Socket>,
    mode: Transport,
    request: &Value,
) -> Value {
    if matches!(mode, Transport::WebSocket) {
        ws_send(socket.as_mut().unwrap(), request.clone()).await;
        return ws_event(socket.as_mut().unwrap(), "response.completed").await["response"].clone();
    }
    let response = fixture.post(request, None).await;
    assert_eq!(response.status(), 200);
    if matches!(mode, Transport::Json) {
        return response.json().await.unwrap();
    }
    let text = response.text().await.unwrap();
    text.split("\n\n")
        .filter_map(|frame| frame.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<Value>(data).ok())
        .find(|event| event["type"] == "response.completed")
        .expect("Missing completed SSE checkpoint")["response"]
        .clone()
}
fn assert_input_exact(sent: &Sent, original: &Value) {
    assert_eq!(
        sent.body["input"], original["input"],
        "No history item, ID, tool argument or cipher may be trimmed/replaced"
    );
    assert!(sent.body.get("previous_response_id").is_none());
    assert!(!sent.headers.contains_key("x-codex-turn-state"));
    assert_eq!(
        sent.body["client_metadata"]["extension"],
        original["client_metadata"]["extension"]
    );
}
fn cipher_owner(fixture: &Fixture, item: Value, account: &str) {
    // Routing custody can legitimately be B after an accepted replay. Check
    // original provenance directly, independently of the active conversation.
    let mut blobs = vec![];
    if let Some(blob) = item["encrypted_content"].as_str() {
        blobs.push(blob.to_owned());
    }
    if let Some(args) = item["encrypted_function_args"].as_array() {
        blobs.extend(args.iter().map(|value| value.as_str().unwrap().to_owned()));
    }
    if item["namespace"] == "collaboration" {
        let args: Value = serde_json::from_str(item["arguments"].as_str().unwrap()).unwrap();
        blobs.push(args["message"].as_str().unwrap().to_owned());
    }
    assert!(!blobs.is_empty());
    let connection = Connection::open(fixture.path()).unwrap();
    for blob in blobs {
        let digest = format!("{:x}", Sha256::digest(blob.as_bytes()));
        let source:(String,String)=connection.query_row("SELECT account,owner FROM native_opaque_v1 WHERE caller='caller' AND kind='encrypted' AND digest=?1",[digest],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(source, (account.to_owned(), format!("owner-{account}")));
    }
}

#[tokio::test]
async fn context_rebuild_default_removed_a_full_history_and_mixed_a_b_continue_b() {
    let policy = ExecutionPolicy::default();
    assert!(
        policy.account_switch_rebuild,
        "Native defaults must rebuild without a setting"
    );
    for mode in [Transport::Json, Transport::Sse, Transport::WebSocket] {
        let a_output = opaque_output("a");
        let mut b_output = vec![
            message("assistant", "answer-b"),
            json!({"type":"reasoning","id":"reasoning-item-b","encrypted_content":"synthetic-reasoning-b","summary":[],"status":"completed"}),
        ];
        // Re-observation on B must not transfer the original A blob's provenance.
        b_output.push(json!({"type":"reasoning","id":"reasoning-a-observed-on-b","encrypted_content":"synthetic-reasoning-a","summary":[],"status":"completed"}));
        let fixture = Fixture::new(
            policy.account_switch_rebuild,
            vec![Step::CompleteItems("a-root", a_output.clone())],
            vec![
                Step::CompleteItems("b-replayed", b_output.clone()),
                Step::Complete("b-next", None),
                Step::Complete("b-after-off", None),
            ],
        )
        .await;
        fixture
            .host
            .passthrough
            .store(matches!(mode, Transport::WebSocket), Ordering::SeqCst);
        let mut socket = if matches!(mode, Transport::WebSocket) {
            Some(fixture.socket().await)
        } else {
            None
        };
        let root = root_request(mode.streaming());
        let root_before = root.clone();
        assert_eq!(
            turn(&fixture, &mut socket, mode, &root).await["output"],
            json!(a_output)
        );
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let original = body(
            append_full(&root, &a_output, "second"),
            Some("a-root"),
            mode.streaming(),
        );
        let unchanged = original.clone();
        let response = turn(&fixture, &mut socket, mode, &original).await;
        assert_eq!(response["id"], "b-replayed");
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].account, "b");
        assert_input_exact(&sent[1], &original);
        assert_ne!(identity(&sent[0]), identity(&sent[1]));
        let b_active = fixture.active().unwrap();
        assert_eq!(b_active.0, "b");
        let mut mixed = original["input"].as_array().unwrap().clone();
        mixed.extend_from_slice(&b_output);
        mixed.push(message("user", "third"));
        let next = body(mixed, Some("b-replayed"), mode.streaming());
        let next_before = next.clone();
        assert_eq!(
            turn(&fixture, &mut socket, mode, &next).await["id"],
            "b-next"
        );
        let sent = fixture.sent();
        assert_eq!(
            sent.iter()
                .map(|request| request.account.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "b"]
        );
        assert_eq!(sent[2].body["input"], next["input"]);
        assert_eq!(sent[2].body["previous_response_id"], "b-replayed");
        assert_eq!(identity(&sent[1]), identity(&sent[2]));
        assert_eq!(fixture.active().unwrap(), b_active);
        fixture.assert_owner("a-root", "a");
        fixture.assert_owner("b-replayed", "b");
        fixture.assert_owner("b-next", "b");
        for item in &a_output[1..] {
            cipher_owner(&fixture, item.clone(), "a");
        }
        cipher_owner(&fixture, b_output[1].clone(), "b");
        assert_eq!(root, root_before);
        assert_eq!(original, unchanged);
        assert_eq!(next, next_before);
        // Closing the option must retain established replay custody. Use a new
        // downstream socket so its execution policy really reads the disabled flag.
        fixture.host.enabled.store(false, Ordering::SeqCst);
        if let Some(mut old_socket) = socket.take() {
            old_socket.close(None).await.unwrap();
            drop(old_socket);
            socket = Some(fixture.socket().await);
        }
        let mut final_input = next["input"].as_array().unwrap().clone();
        final_input.push(message("assistant", "answer-b-next"));
        final_input.push(message("user", "after option off"));
        let after_off = body(final_input, Some("b-next"), mode.streaming());
        let after_off_before = after_off.clone();
        assert_eq!(
            turn(&fixture, &mut socket, mode, &after_off).await["id"],
            "b-after-off"
        );
        let sent = fixture.sent();
        assert_eq!(sent.len(), 4);
        assert_eq!(sent[3].account, "b");
        assert_eq!(sent[3].body["input"], after_off["input"]);
        assert_eq!(sent[3].body["previous_response_id"], "b-next");
        assert_eq!(identity(&sent[2]), identity(&sent[3]));
        assert_eq!(fixture.active().unwrap(), b_active);
        assert_eq!(after_off, after_off_before);
        fixture.assert_owner("b-after-off", "b");
        if let Some(mut socket) = socket {
            socket.close(None).await.unwrap();
            drop(socket);
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_opaque_preexisting_uncached_history_replays_after_opt_in_without_previous()
{
    let a_output = opaque_output("a");
    let fixture = Fixture::new(
        false,
        vec![Step::CompleteItems("a-root", a_output.clone())],
        vec![Step::Complete("b-replayed", None)],
    )
    .await;
    let root = root_request(false);
    let response = fixture.post(&root, None).await;
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    assert!(fixture.active().is_none());
    let count: i64 = Connection::open(fixture.path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    fixture.host.enabled.store(true, Ordering::SeqCst);
    fixture.host.removed_a.store(true, Ordering::SeqCst);
    let original = body(append_full(&root, &a_output, "second"), None, false);
    let unchanged = original.clone();
    let response = fixture.post(&original, None).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["id"], "b-replayed");
    let sent = fixture.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].account, "b");
    assert_input_exact(&sent[1], &original);
    assert_ne!(identity(&sent[0]), identity(&sent[1]));
    assert_eq!(original, unchanged);
    fixture.assert_owner("a-root", "a");
    for item in &a_output[1..] {
        cipher_owner(&fixture, item.clone(), "a");
    }
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_opaque_child_agent_parent_a_does_not_pin_child_b() {
    let a_output = opaque_output("a");
    let fixture = Fixture::new(
        true,
        vec![Step::CompleteItems("a-root", a_output.clone())],
        vec![
            Step::Complete("child-b", None),
            Step::Complete("child-b-next", None),
        ],
    )
    .await;
    let root = root_request(false);
    let response = fixture.post(&root, None).await;
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    let parent_active = fixture.active().unwrap();
    fixture.host.removed_a.store(true, Ordering::SeqCst);
    let mut original = body(
        vec![
            json!({"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"input_text","text":"Payload:\n"},{"type":"encrypted_content","encrypted_content":"synthetic-message-a-spawn_agent"}]}),
            message("user", "child instruction"),
        ],
        None,
        false,
    );
    original["client_metadata"]["thread_id"] = json!("child-thread");
    original["client_metadata"]["x-codex-parent-thread-id"] = json!("client-thread");
    original["client_metadata"]["parent_response_id"] = json!("a-root");
    original["client_metadata"]["x-codex-turn-metadata"]=json!(json!({"session_id":"client-session","thread_id":"child-thread","parent_thread_id":"client-thread","parent_response_id":"a-root","extension":"canonical-kept"}).to_string());
    let unchanged = original.clone();
    let response = fixture.post(&original, None).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()["id"], "child-b");
    let sent = fixture.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].account, "b");
    assert_input_exact(&sent[1], &original);
    assert_eq!(
        fixture.active().unwrap(),
        parent_active,
        "Child replay must not activate a new parent generation"
    );
    assert_eq!(
        fixture
            .host
            .selections
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .parent_thread_id
            .as_deref(),
        Some("client-thread")
    );
    assert_eq!(original, unchanged);
    fixture.assert_owner("a-root", "a");
    cipher_owner(&fixture, a_output[3].clone(), "a");
    let child_active: (String,String,i64)=Connection::open(fixture.path()).unwrap().query_row("SELECT account,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='child-thread'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(child_active.0, "b");
    let mut next = original.clone();
    next["previous_response_id"] = json!("child-b");
    next["input"].as_array_mut().unwrap().extend([
        message("assistant", "answer-child-b"),
        message("user", "child second"),
    ]);
    let next_before = next.clone();
    let response = fixture.post(&next, Some("synthetic-state-child-b")).await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["id"],
        "child-b-next"
    );
    let sent = fixture.sent();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[2].account, "b");
    assert_eq!(sent[2].body["input"], next["input"]);
    assert_eq!(sent[2].body["previous_response_id"], "child-b");
    assert_eq!(
        sent[2].headers["x-codex-turn-state"],
        "synthetic-state-child-b"
    );
    assert_eq!(identity(&sent[1]), identity(&sent[2]));
    assert!(sent[2].body["client_metadata"]
        .get("parent_response_id")
        .is_none());
    let canonical: Value = serde_json::from_str(
        sent[2].body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(canonical.get("parent_response_id").is_none());
    let child_after:(String,String,i64)=Connection::open(fixture.path()).unwrap().query_row("SELECT account,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='child-thread'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(child_after, child_active);
    assert_eq!(fixture.active().unwrap(), parent_active);
    assert_eq!(next, next_before);
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_opaque_failed_b_keeps_active_a_and_original_cipher_history() {
    for failure in [Step::HttpOverload, Step::SseOverload] {
        let streaming = matches!(failure, Step::SseOverload);
        let a_output = opaque_output("a");
        let fixture = Fixture::new(
            true,
            vec![Step::CompleteItems("a-root", a_output.clone())],
            vec![failure],
        )
        .await;
        let root = root_request(false);
        let response = fixture.post(&root, None).await;
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        let active = fixture.active().unwrap();
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let original = body(
            append_full(&root, &a_output, "second"),
            Some("a-root"),
            streaming,
        );
        let unchanged = original.clone();
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), 503);
        response.bytes().await.unwrap();
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].account, "b");
        assert_input_exact(&sent[1], &original);
        assert_eq!(fixture.active().unwrap(), active);
        assert_eq!(original, unchanged);
        fixture.assert_owner("a-root", "a");
        for item in &a_output[1..] {
            cipher_owner(&fixture, item.clone(), "a");
        }
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_opaque_disabled_policy_keeps_removed_known_owner_pinned() {
    for _ in [false] {
        let a_output = opaque_output("a");
        let fixture = Fixture::new(
            false,
            vec![Step::CompleteItems("a-root", a_output.clone())],
            vec![],
        )
        .await;
        let root = root_request(false);
        let response = fixture.post(&root, None).await;
        assert_eq!(response.status(), 200);
        response.bytes().await.unwrap();
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let original = body(
            append_full(&root, &a_output, "second"),
            Some("a-root"),
            false,
        );
        let unchanged = original.clone();
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), 503);
        response.bytes().await.unwrap();
        assert_eq!(
            fixture.sent().len(),
            1,
            "Removed A cannot be sent and B must never be attempted"
        );
        let selected = fixture.host.selections.lock().unwrap();
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[1].preferred_account_id.as_deref(), Some("a"));
        assert!(selected[1].excluded_account_ids.is_empty());
        drop(selected);
        assert_eq!(original, unchanged);
        fixture.assert_owner("a-root", "a");
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_opaque_unknown_self_contained_history_is_sent_unchanged_and_caller_scoped()
{
    let a_output = opaque_output("a");
    let fixture = Fixture::new(
        true,
        vec![Step::CompleteItems("a-root", a_output.clone())],
        (0..5).map(|_| Step::Complete("b-accepted", None)).collect(),
    )
    .await;
    let root = root_request(false);
    let response = fixture.post(&root, None).await;
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    fixture.host.removed_a.store(true, Ordering::SeqCst);
    for (index, unknown) in [
        json!({"type":"reasoning","encrypted_content":"synthetic-never-observed","summary":[]}),
        json!({"type":"compaction","encrypted_content":"synthetic-never-observed"}),
        json!({"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"encrypted_content","encrypted_content":"synthetic-never-observed"}]}),
        json!({"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"unknown-call","arguments":"{\"message\":\"synthetic-never-observed\"}"}),
        json!({"type":"function_call","name":"lookup","call_id":"unknown-call","arguments":"{}","encrypted_function_args":["synthetic-never-observed"]}),
    ].into_iter().enumerate() {
        let mut input = append_full(&root, &a_output, "second"); input.push(unknown.clone());
        if unknown["type"] == "function_call" { input.push(json!({"type":"function_call_output","call_id":"unknown-call","output":"ok"})); }
        let original = body(input, None, false);
        let response = fixture.post(&original, None).await;
        assert_eq!(response.status(), 200, "case {index}"); response.bytes().await.unwrap();
        assert_eq!(fixture.sent().last().unwrap().body["input"], original["input"]);
        assert!(IdentityStore::open(fixture.path()).unwrap().inspect_for_rebuild("other-caller", &HeaderMap::new(), &original).is_err());
    }
    assert_eq!(fixture.sent().len(), 6);
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_opaque_cancelled_b_candidate_keeps_active_and_cipher_provenance_a() {
    let a_output = opaque_output("a");
    let fixture = Fixture::new(
        true,
        vec![Step::CompleteItems("a-root", a_output.clone())],
        vec![Step::Hold("b-pending")],
    )
    .await;
    let root = root_request(false);
    let response = fixture.post(&root, None).await;
    assert_eq!(response.status(), 200);
    response.bytes().await.unwrap();
    let active = fixture.active().unwrap();
    fixture.host.removed_a.store(true, Ordering::SeqCst);
    let original = body(
        append_full(&root, &a_output, "second"),
        Some("a-root"),
        true,
    );
    let unchanged = original.clone();
    let mut socket = fixture.socket().await;
    ws_send(&mut socket, original.clone()).await;
    assert_eq!(
        ws_event(&mut socket, "response.created").await["response"]["id"],
        "b-pending"
    );
    assert_eq!(fixture.active().unwrap(), active);
    socket.close(None).await.unwrap();
    drop(socket);
    tokio::time::timeout(Duration::from_secs(4), async {
        while fixture.host.reports.lock().unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let sent = fixture.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].account, "b");
    assert_input_exact(&sent[1], &original);
    assert_eq!(fixture.active().unwrap(), active);
    assert_eq!(original, unchanged);
    fixture.assert_owner("a-root", "a");
    for item in &a_output[1..] {
        cipher_owner(&fixture, item.clone(), "a");
    }
    let count: i64 = Connection::open(fixture.path())
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1 WHERE account='b'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    fixture.finish().await;
}

#[tokio::test]
async fn context_rebuild_opaque_same_socket_opt_in_joins_observed_uncached_history_with_delta() {
    for passthrough in [false, true] {
        let a_output = opaque_output("a");
        let fixture = Fixture::new(
            false,
            vec![Step::CompleteItems("a-root", a_output.clone())],
            vec![Step::Complete("b-socket-replayed", None)],
        )
        .await;
        fixture
            .host
            .passthrough
            .store(passthrough, Ordering::SeqCst);
        let mut socket = fixture.socket().await;
        let root = root_request(true);
        ws_send(&mut socket, root.clone()).await;
        let completed = ws_event(&mut socket, "response.completed").await;
        assert_eq!(completed["response"]["id"], "a-root");
        assert_eq!(completed["response"]["output"], json!(a_output));
        assert!(fixture.active().is_none());
        let checkpoints: i64 = Connection::open(fixture.path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            checkpoints, 0,
            "Root completed before opt-in, so only socket history can fill the delta"
        );
        fixture.host.enabled.store(true, Ordering::SeqCst);
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let full = append_full(&root, &a_output, "second");
        let delta = full[root["input"].as_array().unwrap().len() + a_output.len()..].to_vec();
        let request = body(delta, Some("a-root"), true);
        let unchanged = request.clone();
        assert_ne!(
            request["input"],
            json!(full),
            "Client sends only tool results/Agent envelope/new turn, not the root history"
        );
        ws_send(&mut socket, request.clone()).await;
        assert_eq!(
            ws_event(&mut socket, "response.completed").await["response"]["id"],
            "b-socket-replayed"
        );
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].account, "b");
        assert_eq!(sent[1].body["input"], json!(full));
        assert!(sent[1].body.get("previous_response_id").is_none());
        assert!(!sent[1].headers.contains_key("x-codex-turn-state"));
        assert_ne!(identity(&sent[0]), identity(&sent[1]));
        assert_eq!(request, unchanged);
        fixture.assert_owner("a-root", "a");
        fixture.assert_owner("b-socket-replayed", "b");
        for item in &a_output[1..] {
            cipher_owner(&fixture, item.clone(), "a");
        }
        if passthrough {
            assert_eq!(
                *fixture.upstream.ws_connections.lock().unwrap(),
                vec!["a".to_owned(), "b".to_owned()]
            );
        } else {
            assert!(fixture.upstream.ws_connections.lock().unwrap().is_empty());
        }
        socket.close(None).await.unwrap();
        drop(socket);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn context_rebuild_opaque_observed_a_sticky_token_never_relocks_completed_b_generation() {
    for mode in [Transport::Json, Transport::Sse] {
        for full_history in [false, true] {
            for keep_enabled in [false, true] {
                let a_reasoning = json!({"type":"reasoning","id":"reasoning-a-sticky","encrypted_content":"synthetic-reasoning-a-sticky","summary":[],"status":"completed"});
                let b_reasoning = json!({"type":"reasoning","id":"reasoning-b-sticky","encrypted_content":"synthetic-reasoning-b-sticky","summary":[],"status":"completed"});
                let fixture = Fixture::new(
                    true,
                    vec![Step::Complete("a-root", Some(a_reasoning.clone()))],
                    vec![
                        Step::Complete("b-replayed", Some(b_reasoning.clone())),
                        Step::Complete("b-after-sticky", None),
                    ],
                )
                .await;
                let root = root_request(false);
                let response = fixture.post(&root, None).await;
                assert_eq!(response.status(), 200);
                let sticky = response
                    .headers()
                    .get("x-codex-turn-state")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                assert_eq!(sticky, "synthetic-state-a-root");
                let a_response: Value = response.json().await.unwrap();
                fixture.host.removed_a.store(true, Ordering::SeqCst);
                let mut replay_input = root["input"].as_array().unwrap().clone();
                replay_input.extend(a_response["output"].as_array().unwrap().iter().cloned());
                replay_input.push(message("user", "second"));
                let replay = body(replay_input, Some("a-root"), mode.streaming());
                let response = fixture.post(&replay, Some(&sticky)).await;
                assert_eq!(response.status(), 200);
                assert!(response.text().await.unwrap().contains("b-replayed"));
                let b_active = fixture.active().unwrap();
                assert_eq!(b_active.0, "b");
                fixture.host.enabled.store(keep_enabled, Ordering::SeqCst);
                let input = if full_history {
                    let mut full = replay["input"].as_array().unwrap().clone();
                    full.extend([
                        message("assistant", "answer-b-replayed"),
                        b_reasoning.clone(),
                        message("user", "third"),
                    ]);
                    full
                } else {
                    vec![message("user", "third")]
                };
                let mut original = body(input, Some("b-replayed"), mode.streaming());
                original["client_metadata"]["x-codex-turn-state"] = json!(sticky);
                original["client_metadata"]["x-codex-turn-metadata"]=json!(json!({"session_id":"client-session","thread_id":"client-thread","x-codex-turn-state":sticky,"extension":"sticky-canonical-kept"}).to_string());
                let unchanged = original.clone();
                // The client OnceLock still carries A's observed sticky token.
                // Current B continuation and its accepted generation take precedence.
                let response = fixture.post(&original, Some(&sticky)).await;
                assert_eq!(
                    response.status(),
                    200,
                    "full={full_history},enabled={keep_enabled}"
                );
                assert!(response.text().await.unwrap().contains("b-after-sticky"));
                let sent = fixture.sent();
                assert_eq!(
                    sent.iter()
                        .map(|request| request.account.as_str())
                        .collect::<Vec<_>>(),
                    vec!["a", "b", "b"]
                );
                assert_eq!(sent[2].body["input"], original["input"]);
                assert_eq!(sent[2].body["previous_response_id"], "b-replayed");
                assert_eq!(identity(&sent[1]), identity(&sent[2]));
                assert_ne!(
                    sent[2]
                        .headers
                        .get("x-codex-turn-state")
                        .and_then(|value| value.to_str().ok()),
                    Some(sticky.as_str())
                );
                assert_ne!(
                    sent[2].body["client_metadata"]["x-codex-turn-state"].as_str(),
                    Some(sticky.as_str())
                );
                let canonical: Value = serde_json::from_str(
                    sent[2].body["client_metadata"]["x-codex-turn-metadata"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                assert_ne!(
                    canonical["x-codex-turn-state"].as_str(),
                    Some(sticky.as_str())
                );
                assert_eq!(canonical["extension"], "sticky-canonical-kept");
                assert_eq!(
                    fixture.active().unwrap(),
                    b_active,
                    "Sticky A cannot restart the completed B generation"
                );
                assert_eq!(original, unchanged);
                cipher_owner(&fixture, a_reasoning, "a");
                cipher_owner(&fixture, b_reasoning, "b");
                fixture.assert_owner("a-root", "a");
                fixture.assert_owner("b-replayed", "b");
                fixture.assert_owner("b-after-sticky", "b");
                let old_token_rows:i64=Connection::open(fixture.path()).unwrap().query_row("SELECT COUNT(*) FROM native_opaque_v1 WHERE caller='caller' AND kind='turn_state' AND account='a' AND owner='owner-a'",[],|row|row.get(0)).unwrap();
                assert_eq!(
                    old_token_rows, 1,
                    "The original sticky token retains A provenance"
                );
                fixture.finish().await;
            }
        }
    }
}

#[tokio::test]
async fn context_rebuild_opaque_same_socket_automatically_replays_observed_history_when_owner_is_removed(
) {
    for passthrough in [false, true] {
        let a_output = opaque_output("a");
        let fixture = Fixture::new(
            true,
            vec![Step::CompleteItems("a-root", a_output.clone())],
            vec![Step::Complete("b-after-removal", None)],
        )
        .await;
        fixture
            .host
            .passthrough
            .store(passthrough, Ordering::SeqCst);
        let mut socket = fixture.socket().await;
        ws_send(&mut socket, root_request(true)).await;
        assert_eq!(
            ws_event(&mut socket, "response.completed").await["response"]["id"],
            "a-root"
        );
        fixture.host.removed_a.store(true, Ordering::SeqCst);
        let mut continuation = a_output.iter().filter(|item| item["type"] == "function_call")
            .map(|item| json!({"type":"function_call_output","call_id":item["call_id"],"output":"completed child task"}))
            .collect::<Vec<_>>();
        continuation.push(message("user", "continue"));
        ws_send(
            &mut socket,
            body(continuation, Some("a-root"), true),
        )
        .await;
        assert_eq!(
            ws_event(&mut socket, "response.completed").await["response"]["id"],
            "b-after-removal"
        );
        let sent = fixture.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].account, "b");
        assert!(sent[1].body.get("previous_response_id").is_none());
        for item in &a_output[1..] {
            cipher_owner(&fixture, item.clone(), "a");
        }
        fixture.assert_owner("a-root", "a");
        assert_eq!(fixture.active().unwrap().0, "b");
        socket.close(None).await.unwrap();
        fixture.finish().await;
    }
}
