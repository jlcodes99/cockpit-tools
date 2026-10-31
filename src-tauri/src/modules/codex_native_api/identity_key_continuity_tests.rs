use super::*;
use serde_json::json;

fn fixture(incomplete: bool) -> (tempfile::TempDir, IdentityStore, Value) {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    let request = json!({"model":"fixture","instructions":"Keep the marker PINE","tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}],
        "client_metadata":{"session_id":"session","thread_id":"thread"},"input":[
        {"role":"user","content":"Remember PINE"},
        {"type":"function_call","name":"lookup","call_id":"call-1","arguments":"{}"},
        {"type":"function_call_output","call_id":"call-1","output":"PINE"}]});
    let mut context = store
        .inspect_for_rebuild("old-key", &HeaderMap::new(), &request)
        .unwrap();
    store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .unwrap();
    let binding = store
        .rewrite(
            &context,
            "account-a",
            "owner-a",
            &mut HeaderMap::new(),
            &mut request.clone(),
        )
        .unwrap();
    let status = if incomplete {
        "incomplete"
    } else {
        "completed"
    };
    store.observe_event(&binding,&json!({"type":format!("response.{status}"),"response":{"id":"saved-response","status":status,"output":[
        {"type":"reasoning","id":"reasoning-1","encrypted_content":"opaque-PINE","summary":[]},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"PINE"}]}]}})).unwrap();
    (dir, store, request)
}
fn next() -> Value {
    json!({"model":"fixture","previous_response_id":"saved-response","client_metadata":{"session_id":"session","thread_id":"thread"},"input":[{"role":"user","content":"continue"}]})
}

#[test]
fn key_rotation_preserves_checkpoint_tools_and_supports_account_switch() {
    let (_dir, store, original) = fixture(false);
    store.set_shared_continuation_references(true);
    let request = next();
    let mut context = store
        .inspect_for_forwarding("new-key", &HeaderMap::new(), &request, true)
        .unwrap();
    assert_eq!(context.caller, "new-key");
    let plan = store
        .expand_http_previous(&mut context, &HeaderMap::new(), &request, &request)
        .unwrap();
    assert_eq!(plan.body["instructions"], original["instructions"]);
    assert_eq!(plan.body["tools"], original["tools"]);
    let input = plan.body["input"].as_array().unwrap();
    assert_eq!(
        input
            .iter()
            .filter(|v| v["type"] == "function_call")
            .count(),
        1
    );
    assert_eq!(
        input
            .iter()
            .filter(|v| v["type"] == "function_call_output")
            .count(),
        1
    );
    assert_eq!(input[0]["content"], "Remember PINE");
    assert!(plan.body.get("previous_response_id").is_none());
    let mut context = store
        .inspect_for_rebuild("new-key", &plan.headers, &plan.body)
        .unwrap();
    store
        .configure_rebuild(&mut context, &plan.headers, &plan.body, true)
        .unwrap();
    context.start_rebuild_generation(&plan);
    let mut replay = plan.body.clone();
    let binding = store
        .rewrite(
            &context,
            "account-b",
            "owner-b",
            &mut plan.headers.clone(),
            &mut replay,
        )
        .unwrap();
    assert_eq!(binding.caller, "new-key");
    store.observe_event(&binding,&json!({"type":"response.completed","response":{"id":"next-response","status":"completed","output":[]}})).unwrap();
    assert_eq!(
        store
            .inspect_for_forwarding(
                "new-key",
                &HeaderMap::new(),
                &json!({"previous_response_id":"next-response","input":[]}),
                true
            )
            .unwrap()
            .state_account_id
            .as_deref(),
        Some("account-b")
    );
}

#[test]
fn full_client_history_with_old_ciphertext_does_not_need_server_checkpoint() {
    let (_dir, store, mut request) = fixture(false);
    store.set_shared_continuation_references(true);
    request["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"reasoning","encrypted_content":"opaque-PINE","summary":[]}));
    let mut context = store
        .inspect_for_forwarding("new-key", &HeaderMap::new(), &request, true)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .unwrap();
    assert_eq!(plan.body["input"], request["input"]);
    let c = store.connection.lock().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM native_rebuild_history_v1 WHERE caller='new-key'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn partial_key_rotation_survives_restart_and_preserves_history_on_repeated_retry() {
    let (dir, store, _) = fixture(true);
    store.set_shared_continuation_references(true);
    let first = store
        .expand_partial_history("new-key", &HeaderMap::new(), &next(), usize::MAX)
        .unwrap()
        .unwrap();
    assert_eq!(first.body["input"][0]["content"], "Remember PINE");
    drop(store);
    let reopened = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    reopened.set_shared_continuation_references(true);
    for _ in 0..2 {
        assert_eq!(
            reopened
                .expand_partial_history("new-key", &HeaderMap::new(), &next(), usize::MAX)
                .unwrap()
                .unwrap()
                .body,
            first.body
        );
    }
}

#[test]
fn key_rotation_does_not_grant_thread_only_access_or_copy_unrelated_records() {
    let (_dir, store, _) = fixture(false);
    store.set_shared_continuation_references(true);
    store.inspect_for_forwarding("new-key",&HeaderMap::new(),&json!({"client_metadata":{"thread_id":"thread"},"input":[{"role":"user","content":"hi"}]}),true).unwrap();
    let c = store.connection.lock().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM native_opaque_v1 WHERE caller='new-key'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    let mut request = next();
    request["client_metadata"]["thread_id"] = json!("another-thread");
    assert_eq!(
        store
            .inspect_for_forwarding("new-key", &HeaderMap::new(), &request, true)
            .unwrap_err()
            .code,
        "upstream_state_thread_mismatch"
    );
}

#[test]
fn strict_hosts_still_isolate_keys_and_missing_history_does_not_become_empty() {
    let (_dir, store, _) = fixture(false);
    assert_eq!(
        store
            .inspect_for_forwarding("new-key", &HeaderMap::new(), &next(), true)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    store.set_shared_continuation_references(true);
    store
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM native_rebuild_items_v1", [])
        .unwrap();
    assert!(store
        .inspect_for_forwarding("new-key", &HeaderMap::new(), &next(), true)
        .is_err());
    let c = store.connection.lock().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM native_opaque_v1 WHERE caller='new-key'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn key_rotation_uses_legacy_session_alias_for_turn_token_scope() {
    let (_dir, store, _) = fixture(false);
    let reference = turn_state_reference("turn-token", Some("session"), None);
    store.connection.lock().unwrap().execute("INSERT INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires) VALUES('old-key','turn_state',?1,'account-a','owner-a','session','session',NULL,?2)",params![reference.digest,timestamp()+3600]).unwrap();
    store.set_shared_continuation_references(true);
    let headers = HeaderMap::from_iter([
        ("session_id".parse().unwrap(), "session".parse().unwrap()),
        (
            "x-codex-turn-state".parse().unwrap(),
            "turn-token".parse().unwrap(),
        ),
    ]);
    let context = store
        .inspect_for_forwarding(
            "new-key",
            &headers,
            &json!({"input":[{"role":"user","content":"next"}]}),
            true,
        )
        .unwrap();
    assert_eq!(context.thread_id.as_deref(), Some("session"));
    assert_eq!(context.state_account_id.as_deref(), Some("account-a"));
}
