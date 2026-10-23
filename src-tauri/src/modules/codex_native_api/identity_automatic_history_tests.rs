use super::*;
use serde_json::json;

#[path = "identity_host_results_tests.rs"]
mod host_results;

fn fixture() -> (tempfile::TempDir, IdentityStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(temp.path().join("identity.sqlite")).unwrap();
    (temp, store)
}

#[test]
fn provider_projection_never_admits_ciphertext_omitted_from_upstream() {
    let (_temp, store) = fixture();
    let original = history();
    let (mut binding, _, mut forwarded) =
        prepare(&store, "caller", &HeaderMap::new(), &original, "b");
    assert!(!binding.input_blobs.is_empty());
    forwarded["input"] = json!([{ "role":"user", "content":"projected text" }]);
    binding.retain_transmitted_input_blobs(&forwarded);
    assert!(binding.input_blobs.is_empty());
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"projected-success","status":"completed","output":[]}})).unwrap();
    let count: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(original, history(), "saved caller input must stay intact");
}
fn history() -> Value {
    json!({"model":"fixture","tools":[{"type":"function","name":"spawn_agent","parameters":{"type":"object"}}],
        "client_metadata":{"session_id":"root","thread_id":"root"},"input":[
        {"role":"user","content":"old prompt"},
        {"type":"reasoning","id":"old-reasoning","encrypted_content":"legacy-reasoning","summary":[]},
        {"type":"compaction","id":"old-compaction","encrypted_content":"legacy-compaction"},
        {"type":"function_call","id":"old-call","namespace":"collaboration","name":"spawn_agent","call_id":"call-old","arguments":"{\"message\":\"legacy-agent-message\"}","encrypted_function_args":["legacy-arguments"]},
        {"type":"function_call_output","call_id":"call-old","output":[{"type":"encrypted_content","encrypted_content":"legacy-tool-output"}]},
        {"type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"encrypted_content","encrypted_content":"legacy-agent-message"}]},
        {"role":"user","content":"continue"}]})
}
fn prepare(
    store: &IdentityStore,
    caller: &str,
    headers: &HeaderMap,
    request: &Value,
    account: &str,
) -> (Binding, HeaderMap, Value) {
    let mut context = store.inspect_for_rebuild(caller, headers, request).unwrap();
    let plan = store
        .configure_rebuild(&mut context, headers, request, true)
        .unwrap()
        .unwrap();
    assert_eq!(plan.body["input"], request["input"]);
    let mut body = request.clone();
    let mut headers = headers.clone();
    let binding = store
        .rewrite(
            &context,
            account,
            &format!("owner-{account}"),
            &mut headers,
            &mut body,
        )
        .unwrap();
    (binding, headers, body)
}
fn complete(store: &IdentityStore, binding: &Binding, response: &str) {
    store.observe_event(binding, &json!({"type":"response.completed","response":{"id":response,"status":"completed","output":[{"role":"assistant","content":"answer"}]}})).unwrap();
}

#[test]
fn complete_legacy_shell_and_handled_argument_errors_are_replayable_without_changing_history() {
    for (call, output) in [
        (
            json!({"type":"local_shell_call","id":"legacy-shell","call_id":"legacy-call","status":"completed","action":{"type":"exec","command":["echo","ok"]}}),
            json!({"type":"function_call_output","call_id":"legacy-call","output":"ok"}),
        ),
        (
            json!({"type":"function_call","name":"spawn_agent","namespace":"collaboration","call_id":"legacy-call","arguments":"{\"message\":"}),
            json!({"type":"function_call_output","call_id":"legacy-call","output":"Error parsing function arguments: EOF while parsing an object"}),
        ),
    ] {
        let (_temp, store) = fixture();
        let mut original = history();
        let input = original["input"].as_array_mut().unwrap();
        input.insert(1, call.clone());
        input.insert(2, output.clone());
        let saved = original.clone();
        let (binding, headers_a, sent) =
            prepare(&store, "caller", &HeaderMap::new(), &original, "a");
        assert_eq!(sent["input"], original["input"]);
        complete(&store, &binding, "legacy-response");
        drop(store);

        let restarted = IdentityStore::open(_temp.path().join("identity.sqlite")).unwrap();
        let (_, headers_b, sent) = prepare(&restarted, "caller", &HeaderMap::new(), &original, "b");
        assert_ne!(headers_a["session-id"], headers_b["session-id"]);
        assert_ne!(headers_a["thread-id"], headers_b["thread-id"]);
        assert_eq!(sent["input"], original["input"]);
        assert_eq!(original, saved);
        assert_eq!(
            restarted
                .inspect_for_rebuild("foreign", &HeaderMap::new(), &original)
                .unwrap_err()
                .code,
            "upstream_state_caller_mismatch"
        );

        for invalid in [
            vec![call.clone()],
            vec![output.clone()],
            vec![output.clone(), call.clone()],
            vec![call.clone(), call.clone(), output.clone()],
            vec![
                call.clone(),
                json!({"type":"function_call_output","call_id":"wrong-call","output":"ok"}),
            ],
            vec![
                call.clone(),
                json!({"type":"custom_tool_call_output","call_id":"legacy-call","output":"wrong output kind"}),
            ],
        ] {
            assert!(
                !super::rebuild::paired_tools(&invalid),
                "must reject incomplete or conflicting history: {invalid:?}"
            );
        }
    }
}

#[test]
fn legacy_full_history_preserves_all_cipher_kinds_and_child_scope_across_engines() {
    let (_temp, store) = fixture();
    let original = history();
    let (binding, first_headers, sent) =
        prepare(&store, "caller", &HeaderMap::new(), &original, "a");
    assert_eq!(sent["input"], original["input"]);
    assert_ne!(first_headers["thread-id"], "root");
    assert_ne!(first_headers["session-id"], "root");
    complete(&store, &binding, "response-a");
    // Opening the same durable store models a Go worker / Rust native handoff.
    let next_engine = IdentityStore::open(store.path.clone()).unwrap();
    let (same, resumed_headers, resumed) =
        prepare(&next_engine, "caller", &HeaderMap::new(), &original, "a");
    assert_eq!(first_headers["thread-id"], resumed_headers["thread-id"]);
    assert_eq!(resumed["input"], original["input"]);
    complete(&next_engine, &same, "response-a2");
    let mut child = json!({"model":"fixture","client_metadata":{"session_id":"root","thread_id":"child","x-codex-parent-thread-id":"root"},"input":[original["input"][5].clone()]});
    let (child_binding, child_headers, child_sent) =
        prepare(&next_engine, "caller", &HeaderMap::new(), &child, "a");
    assert_eq!(child_headers["session-id"], first_headers["session-id"]);
    assert_ne!(child_headers["thread-id"], first_headers["thread-id"]);
    assert_eq!(child_sent["input"], child["input"]);
    complete(&next_engine, &child_binding, "response-child");
    let (changed, b_headers, b_sent) =
        prepare(&next_engine, "caller", &HeaderMap::new(), &original, "b");
    assert_ne!(first_headers["session-id"], b_headers["session-id"]);
    assert_ne!(first_headers["thread-id"], b_headers["thread-id"]);
    assert_eq!(b_sent["input"], original["input"]);
    complete(&next_engine, &changed, "response-b");
    child["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role":"user","content":"continue child"}));
    let (_, still_child, _) = prepare(&next_engine, "caller", &HeaderMap::new(), &child, "a");
    assert_eq!(still_child["thread-id"], child_headers["thread-id"]);
}

#[test]
fn unregistered_sticky_routing_is_removed_from_every_projection_without_touching_history() {
    let (_temp, store) = fixture();
    let mut request = history();
    let canonical = json!({"session_id":"root","thread_id":"root","turn_id":"turn","x-codex-turn-state":"old-token","parent_response_id":"old-parent"}).to_string();
    request["client_metadata"]["x-codex-turn-state"] = json!("old-token");
    request["client_metadata"]["parent_response_id"] = json!("old-parent");
    request["client_metadata"]["x-codex-turn-metadata"] = json!(canonical);
    let mut headers = HeaderMap::new();
    headers.insert("x-codex-turn-state", "old-token".parse().unwrap());
    headers.insert("x-codex-turn-metadata", canonical.parse().unwrap());
    let (_, headers, sent) = prepare(&store, "caller", &headers, &request, "b");
    assert_eq!(sent["input"], request["input"]);
    assert!(headers.get("x-codex-turn-state").is_none());
    assert!(sent["client_metadata"].get("x-codex-turn-state").is_none());
    assert!(sent["client_metadata"].get("parent_response_id").is_none());
    for raw in [
        headers["x-codex-turn-metadata"].to_str().unwrap(),
        sent["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    ] {
        let meta: Value = serde_json::from_str(raw).unwrap();
        assert!(meta.get("x-codex-turn-state").is_none());
        assert!(meta.get("parent_response_id").is_none());
    }
}

#[test]
fn automatic_history_rechecks_caller_provenance_before_binding() {
    let (_temp, store) = fixture();
    let request = history();
    let mut earlier = store
        .inspect_for_rebuild("caller-b", &HeaderMap::new(), &request)
        .unwrap();
    store
        .configure_rebuild(&mut earlier, &HeaderMap::new(), &request, true)
        .unwrap()
        .unwrap();
    let (owner, _, _) = prepare(&store, "caller-a", &HeaderMap::new(), &request, "a");
    complete(&store, &owner, "accepted");
    assert_eq!(
        store
            .inspect_for_rebuild("caller-b", &HeaderMap::new(), &request)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    let mut body = request.clone();
    assert_eq!(
        store
            .rewrite(&earlier, "b", "owner-b", &mut HeaderMap::new(), &mut body)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    assert_eq!(body, request);
}

#[test]
fn unknown_incremental_state_requests_full_input_then_resumes_without_manual_recovery() {
    let (_temp, store) = fixture();
    let request = history();
    let mut delta = json!({"model":"fixture","previous_response_id":"unrecorded-response","input":[{"role":"user","content":"next"}]});
    let error = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &delta)
        .unwrap_err();
    assert_eq!(error.code, "previous_response_not_found");
    delta["input"] = request["input"].clone();
    delta
        .as_object_mut()
        .unwrap()
        .remove("previous_response_id");
    let (_, _, sent) = prepare(&store, "caller", &HeaderMap::new(), &delta, "b");
    assert_eq!(sent["input"], request["input"]);
}

#[test]
fn incomplete_tool_or_resource_history_is_not_silently_truncated() {
    let (_temp, store) = fixture();
    for mutate in 0..3 {
        let mut request = history();
        match mutate {
            0 => {
                request["input"].as_array_mut().unwrap().remove(4);
            }
            1 => {
                request["conversation"] = json!("server-only-conversation");
            }
            _ => {
                request["tools"] = json!([{"type":"file_search","vector_store_ids":["old-store"]}]);
            }
        }
        let original = request.clone();
        assert_eq!(
            store
                .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
                .unwrap_err()
                .code,
            "continuation_history_incomplete"
        );
        assert_eq!(request, original);
    }
}

#[test]
fn remote_compaction_trigger_keeps_unknown_history_and_legacy_compact_can_disable_checkpoints() {
    let (_temp, store) = fixture();
    let mut request = history();
    request["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"compaction_trigger"}));
    let (_, _, sent) = prepare(&store, "caller", &HeaderMap::new(), &request, "b");
    assert_eq!(sent["input"], request["input"]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    context.disable_checkpoints();
    let mut compact = request.clone();
    store
        .rewrite(
            &context,
            "b",
            "owner-b",
            &mut HeaderMap::new(),
            &mut compact,
        )
        .unwrap();
    assert_eq!(compact["input"], request["input"]);
}

#[test]
fn cached_history_completes_incremental_input_containing_new_unregistered_agent_cipher() {
    let (_temp, store) = fixture();
    let original = history();
    let (first, _, _) = prepare(&store, "caller", &HeaderMap::new(), &original, "a");
    complete(&store, &first, "checkpoint-a");
    let delta = json!({"model":"fixture","tools":original["tools"],"client_metadata":original["client_metadata"],"previous_response_id":"checkpoint-a","input":[
        {"type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"encrypted_content","encrypted_content":"previously-unseen-child-message"}]},
        {"role":"user","content":"continue"}]});
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &delta)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &delta, true)
        .unwrap()
        .unwrap();
    assert_eq!(plan.body["input"][0], original["input"][0]);
    assert!(plan.body["input"]
        .as_array()
        .unwrap()
        .ends_with(delta["input"].as_array().unwrap()));
    let mut upstream = delta.clone();
    store
        .rewrite(
            &context,
            "a",
            "owner-a",
            &mut HeaderMap::new(),
            &mut upstream,
        )
        .unwrap();
    assert_eq!(upstream["input"], delta["input"]);
    assert_eq!(upstream["previous_response_id"], "checkpoint-a");
    let mut switched = store
        .inspect_for_rebuild("caller", &plan.headers, &plan.body)
        .unwrap();
    store
        .configure_rebuild(&mut switched, &plan.headers, &plan.body, true)
        .unwrap()
        .unwrap();
    switched.start_rebuild_generation(&plan);
    let mut full = plan.body.clone();
    let binding = store
        .rewrite(
            &switched,
            "b",
            "owner-b",
            &mut plan.headers.clone(),
            &mut full,
        )
        .unwrap();
    assert!(full.get("previous_response_id").is_none());
    assert_eq!(full["input"], plan.body["input"]);
    complete(&store, &binding, "checkpoint-b");
}

#[test]
fn invalid_completed_tool_output_cannot_publish_imported_input_or_generation() {
    let (_temp, store) = fixture();
    let (binding, _, _) = prepare(&store, "caller", &HeaderMap::new(), &history(), "a");
    store.observe_event(&binding, &json!({"type":"response.completed","response":{"id":"invalid-terminal","status":"completed","output":[{"type":"function_call","name":"shell","call_id":"cut","arguments":"{\"truncated\":"}]}})).unwrap();
    let db = store.connection.lock().unwrap();
    for table in [
        "native_rebuild_active_v1",
        "native_rebuild_portable_v1",
        "native_rebuild_checkpoints_v1",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "invalid output committed {table}");
    }
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn cancellation_while_sqlite_is_busy_cannot_late_commit_unknown_history() {
    for checkpoints in [false, true] {
        let (_temp, store) = fixture();
        let store = Arc::new(store);
        let request = history();
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
            .unwrap();
        if checkpoints {
            store
                .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
                .unwrap()
                .unwrap();
        } else {
            context.disable_checkpoints();
        }
        let binding = store
            .rewrite(
                &context,
                "a",
                "owner-a",
                &mut HeaderMap::new(),
                &mut request.clone(),
            )
            .unwrap();
        let cancel = binding.rebuild_abort_handle().unwrap();
        let mut competing_writer = Connection::open(&store.path).unwrap();
        let transaction = competing_writer
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let work_store = store.clone();
        let worker = std::thread::spawn(move || {
            let event = json!({"type":"response.completed","response":{"id":"contended-terminal","status":"completed","output":[]}});
            if checkpoints {
                work_store.commit_rebuild(&binding, &event)
            } else {
                work_store.commit_automatic_input(&binding, &event)
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        // The separate SQLite writer holds BEGIN IMMEDIATE. Seeing this mutex
        // held proves the worker passed its first cancellation check and is now
        // waiting inside transaction creation, rather than not being scheduled.
        loop {
            if store.connection.try_lock().is_err() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "commit did not reach SQLite"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        cancel.store(true, std::sync::atomic::Ordering::Release);
        transaction.rollback().unwrap();
        worker.join().unwrap().unwrap();
        let db = store.connection.lock().unwrap();
        for table in [
            "native_rebuild_active_v1",
            "native_rebuild_portable_v1",
            "native_rebuild_checkpoints_v1",
            "native_rebuild_history_v1",
            "native_rebuild_items_v1",
            "native_opaque_v1",
        ] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "cancelled admission committed {table}");
        }
    }
}
