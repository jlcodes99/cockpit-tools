use super::*;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, IdentityStore, Binding, Value) {
    let root = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(root.path().join("identity.sqlite")).unwrap();
    let request = json!({"model":"fixture","instructions":"original instructions",
        "client_metadata":{"session_id":"session","thread_id":"thread"},
        "input":[{"role":"user","content":"original prompt"}]});
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .unwrap();
    let binding = store
        .rewrite(
            &context,
            "a",
            "owner-a",
            &mut HeaderMap::new(),
            &mut request.clone(),
        )
        .unwrap();
    (root, store, binding, request)
}
fn partial(output: Value) -> Value {
    json!({"type":"response.incomplete","response":{"id":"partial","status":"incomplete","output":output}})
}
fn output() -> Value {
    json!([{"type":"message","role":"assistant","content":"partial text"}])
}
fn next() -> Value {
    json!({"model":"fixture","client_metadata":{"session_id":"session","thread_id":"thread"},
        "previous_response_id":"partial","input":[{"role":"user","content":"continue"}]})
}
fn expand(store: &IdentityStore, request: &Value) -> Option<Value> {
    store
        .expand_partial_history("caller", &HeaderMap::new(), request, usize::MAX)
        .unwrap()
        .map(|partial| partial.body)
}
fn count(store: &IdentityStore) -> i64 {
    store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM native_partial_history_v1", [], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn partial_history_persists_without_claiming_completed_response_or_consuming_it() {
    let (root, store, binding, _) = fixture();
    store.observe_event(&binding, &partial(output())).unwrap();
    let full = expand(&store, &next()).unwrap();
    assert_eq!(full["input"].as_array().unwrap().len(), 3);
    assert_eq!(full["instructions"], "original instructions");
    assert!(full.get("previous_response_id").is_none());
    assert_eq!(expand(&store, &next()).unwrap(), full);
    drop(store);
    let reopened = IdentityStore::open(root.path().join("identity.sqlite")).unwrap();
    assert_eq!(expand(&reopened, &next()).unwrap(), full);
    let c = reopened.connection.lock().unwrap();
    for table in [
        "native_rebuild_checkpoints_v1",
        "native_rebuild_history_v1",
        "native_rebuild_active_v1",
        "native_rebuild_portable_v1",
    ] {
        let count: i64 = c
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "partial must not publish success/active ownership: {table}"
        );
    }
    let count: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn partial_history_restores_omitted_identity_and_rejects_other_callers_threads() {
    let (_root, store, binding, _) = fixture();
    store.observe_event(&binding, &partial(output())).unwrap();
    let error = store
        .expand_partial_history("other", &HeaderMap::new(), &next(), usize::MAX)
        .unwrap_err();
    assert_eq!(error.code, "upstream_state_caller_mismatch");
    for key in ["thread_id", "session_id"] {
        let mut request = next();
        request["client_metadata"][key] = json!("foreign");
        assert_eq!(
            store
                .expand_partial_history("caller", &HeaderMap::new(), &request, usize::MAX)
                .unwrap_err()
                .code,
            "upstream_state_thread_mismatch"
        );
    }
    let mut no_identity = next();
    no_identity
        .as_object_mut()
        .unwrap()
        .remove("client_metadata");
    let full = expand(&store, &no_identity).unwrap();
    assert_eq!(full["client_metadata"]["thread_id"], "thread");
    assert_eq!(full["client_metadata"]["session_id"], "session");
    let mut unknown = next();
    unknown["previous_response_id"] = json!("unknown");
    assert!(expand(&store, &unknown).is_none());
}

#[test]
fn partial_history_tool_result_can_arrive_later_without_duplicate_calls() {
    let (_root, store, binding, _) = fixture();
    let call = json!({"type":"function_call","name":"write","call_id":"once","arguments":"{}","status":"completed"});
    store
        .observe_event(&binding, &partial(json!([call.clone()])))
        .unwrap();
    assert!(expand(&store, &next()).is_none());
    let mut request = next();
    request["input"] = json!([{"type":"function_call_output","call_id":"once","output":"already executed"},{"role":"user","content":"continue"}]);
    let full = expand(&store, &request).unwrap();
    assert_eq!(
        full["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["call_id"] == "once")
            .count(),
        2
    );
    assert_eq!(expand(&store, &request).unwrap(), full);
    let mut duplicate = request.clone();
    duplicate["input"].as_array_mut().unwrap().insert(0, call);
    assert!(expand(&store, &duplicate).is_none());
}

#[test]
fn partial_history_invalid_terminal_never_becomes_a_recovery_anchor() {
    for case in [
        "cancelled",
        "conflict",
        "missing",
        "invalid_tool",
        "error",
        "unknown_input",
    ] {
        let (_root, store, mut binding, _) = fixture();
        let mut event = partial(output());
        match case {
            "cancelled"=>binding.abort_rebuild(),
            "conflict"=>store.observe_event(&binding,&json!({"type":"response.output_item.done","output_index":0,"item":{"role":"assistant","content":"different"}})).unwrap(),
            "missing"=>{binding.note_partial_content(&json!({"type":"response.output_text.delta","delta":"seen"}));event["response"]["output"]=json!([]);},
            "invalid_tool"=>event["response"]["output"]=json!([{"type":"function_call","call_id":"unfinished","name":"write","arguments":"{","status":"in_progress"}]),
            "error"=>event["response"]["error"]=json!({"code":"unknown"}),
            "unknown_input"=>binding.generation.as_mut().unwrap().input=None,
            _=>unreachable!(),
        }
        store.observe_event(&binding, &event).unwrap();
        assert_eq!(count(&store), 0, "{case}");
    }
}

#[test]
fn partial_history_size_failure_and_explicit_parameter_overrides_preserve_original() {
    let (_root, store, binding, _) = fixture();
    store.observe_event(&binding, &partial(output())).unwrap();
    let error = store
        .expand_partial_history("caller", &HeaderMap::new(), &next(), 16)
        .unwrap_err();
    assert_eq!(error.code, "continuation_history_too_large");
    assert_eq!(count(&store), 1);
    let mut request = next();
    request["instructions"] = Value::Null;
    assert!(expand(&store, &request).unwrap()["instructions"].is_null());
    assert_eq!(
        expand(&store, &next()).unwrap()["instructions"],
        "original instructions"
    );
    let mut larger = next();
    larger["input"][0]["content"] = json!("x".repeat(2048));
    assert_eq!(
        store
            .expand_partial_history("caller", &HeaderMap::new(), &larger, 1024)
            .unwrap_err()
            .code,
        "continuation_history_too_large"
    );
    assert!(expand(&store, &next()).is_some());
}

#[test]
fn partial_history_concurrent_duplicate_publication_is_idempotent() {
    let (_root, store, binding, _) = fixture();
    let store = Arc::new(store);
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let store = store.clone();
            let binding = binding.clone();
            std::thread::spawn(move || store.observe_event(&binding, &partial(output())).unwrap())
        })
        .collect();
    for task in tasks {
        task.join().unwrap();
    }
    assert_eq!(count(&store), 1);
    assert!(expand(&store, &next()).is_some());
    store
        .observe_event(
            &binding,
            &partial(json!([{"role":"assistant","content":"late different content"}])),
        )
        .unwrap();
    assert_eq!(
        expand(&store, &next()).unwrap()["input"][1]["content"],
        "partial text"
    );
}

#[test]
fn partial_history_cancellation_while_waiting_for_writer_cannot_publish_late() {
    let (_root, store, binding, _) = fixture();
    let store = Arc::new(store);
    let guard = store.connection.lock().unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let worker = {
        let store = store.clone();
        let binding = binding.clone();
        std::thread::spawn(move || {
            started.send(()).unwrap();
            store.observe_event(&binding, &partial(output())).unwrap();
        })
    };
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    binding.abort_rebuild();
    drop(guard);
    worker.join().unwrap();
    assert_eq!(count(&store), 0);
}

#[test]
fn partial_history_corrupt_chunk_reports_error_without_erasing_original() {
    let (_root, store, binding, _) = fixture();
    store.observe_event(&binding, &partial(output())).unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute("UPDATE native_rebuild_items_v1 SET body=x'010203'", [])
        .unwrap();
    assert!(store
        .expand_partial_history("caller", &HeaderMap::new(), &next(), usize::MAX)
        .is_err());
    assert_eq!(count(&store), 1);
}

#[test]
fn partial_history_upgrade_preserves_existing_completed_history() {
    let (root, store, binding, _) = fixture();
    store.observe_event(&binding,&json!({"type":"response.completed","response":{"id":"completed","status":"completed","output":[]}})).unwrap();
    store
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TABLE native_partial_history_v1;")
        .unwrap();
    drop(store);
    let store = IdentityStore::open(root.path().join("identity.sqlite")).unwrap();
    assert_eq!(count(&store), 0);
    let records: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_history_v1 WHERE response='completed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(records, 1);
}

#[test]
fn partial_history_large_input_round_trips_and_string_continuations_work() {
    let (_root, store, mut binding, _) = fixture();
    binding.generation.as_mut().unwrap().input =
        Some(vec![json!({"role":"user","content":"x".repeat(1_000_000)})]);
    store.observe_event(&binding, &partial(output())).unwrap();
    let mut request = next();
    request["input"] = json!("continue as text");
    let full = expand(&store, &request).unwrap();
    assert_eq!(
        full["input"][0]["content"].as_str().unwrap().len(),
        1_000_000
    );
    assert_eq!(full["input"][2]["content"], "continue as text");
    let stored_bytes: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT SUM(LENGTH(body)) FROM native_rebuild_items_v1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        stored_bytes < 10_000,
        "partial history must use compressed chunks"
    );
}

#[test]
fn partial_history_cannot_override_a_different_resource_owner() {
    let (_root, store, _binding, request) = fixture();
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    context.state_account_id = Some("other".into());
    assert_eq!(
        context.prefer_partial_owner("a").unwrap_err().code,
        "continuation_owner_mismatch"
    );
}

#[test]
fn partial_history_disk_failure_rolls_back_and_can_retry_without_losing_history() {
    let (_root, store, binding, _) = fixture();
    store.connection.lock().unwrap().execute_batch("CREATE TRIGGER fail_partial_chunk BEFORE INSERT ON native_rebuild_items_v1 BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(store.observe_event(&binding, &partial(output())).is_err());
    assert_eq!(count(&store), 0);
    let parents: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='parent_response'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        parents, 0,
        "parent and history publication must roll back together"
    );
    store
        .connection
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_partial_chunk;")
        .unwrap();
    store.observe_event(&binding, &partial(output())).unwrap();
    assert_eq!(count(&store), 1);
    assert!(expand(&store, &next()).is_some());
}

#[test]
fn partial_history_reads_do_not_wait_for_the_shared_identity_writer_mutex() {
    let (_root, store, binding, _) = fixture();
    store.observe_event(&binding, &partial(output())).unwrap();
    let store = Arc::new(store);
    let guard = store.connection.lock().unwrap();
    let (done, result) = std::sync::mpsc::channel();
    let worker = {
        let store = store.clone();
        std::thread::spawn(move || {
            done.send(expand(&store, &next()).is_some()).unwrap();
        })
    };
    let recovered = result.recv_timeout(Duration::from_secs(1));
    drop(guard);
    worker.join().unwrap();
    assert_eq!(recovered.unwrap(), true);
}
