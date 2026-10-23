use super::*;
use serde_json::json;

fn call(name: &str, message: &str) -> Value {
    json!({"type":"function_call","id":"fc-observed","call_id":"call-observed",
        "namespace":"collaboration","name":name,
        "arguments":json!({"task_name":"worker","target":"worker","message":message}).to_string()})
}

fn child(message: &str) -> Value {
    json!({"type":"agent_message","author":"/root","recipient":"/root/worker",
        "content":[{"type":"input_text","text":"Payload:\n"},
        {"type":"encrypted_content","encrypted_content":message}]})
}

fn body(thread: &str, input: Value) -> Value {
    json!({"model":"test","input":[input],
        "client_metadata":{"session_id":"root-session","thread_id":thread,
        "turn_id":format!("turn-{thread}"),"x-codex-parent-thread-id":"parent"}})
}

fn parent_binding(store: &IdentityStore, account: &str, owner: &str) -> Binding {
    let mut headers = HeaderMap::new();
    let mut body = body(
        "parent",
        json!({"type":"message","role":"user","content":"delegate"}),
    );
    let context = store.inspect("caller", &headers, &body).unwrap();
    store
        .rewrite(&context, account, owner, &mut headers, &mut body)
        .unwrap()
}

#[test]
fn agent_messages_match_official_direct_source_marker_and_keep_payload_opaque() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        for marker in [None, Some(Value::Null), Some(json!(["argument-envelope"]))] {
            let mut item = call(name, "opaque-communication");
            if let Some(marker) = marker {
                item["encrypted_function_args"] = marker;
            }
            assert_eq!(
                agent_communication::encrypted_message(&item).as_deref(),
                Some("opaque-communication")
            );
            let unchanged = item.clone();
            let mut references = Vec::new();
            collect_encrypted_item(&item, &mut references);
            assert!(references
                .iter()
                .any(|reference| reference.digest
                    == digest("encrypted", "opaque-communication").digest));
            assert_eq!(item, unchanged);
        }
        let mut plaintext = call(name, "readable task");
        plaintext["encrypted_function_args"] = json!([]);
        let mut references = Vec::new();
        collect_encrypted_item(&plaintext, &mut references);
        assert!(references.is_empty());
    }
}

#[test]
fn generic_or_malformed_tool_arguments_do_not_create_communication_evidence() {
    let valid = call("spawn_agent", "opaque-communication");
    let mut invalid = Vec::new();
    for (field, value) in [
        ("namespace", json!("other")),
        ("namespace", Value::Null),
        ("name", json!("done")),
        ("name", json!("collaboration.spawn_agent")),
        ("type", json!("custom_tool_call")),
        ("arguments", json!("broken JSON")),
        ("arguments", json!({"message":"opaque-communication"})),
        ("encrypted_function_args", json!("not an array")),
        ("encrypted_function_args", json!([false])),
    ] {
        let mut item = valid.clone();
        item[field] = value;
        invalid.push(item);
    }
    for message in [
        Value::Null,
        json!(123),
        json!([]),
        json!(""),
        json!({"nested":"opaque-communication"}),
    ] {
        let mut item = valid.clone();
        item["arguments"] = json!(json!({"message":message}).to_string());
        invalid.push(item);
    }
    for item in invalid {
        let mut references = Vec::new();
        collect_encrypted_item(&item, &mut references);
        assert!(references.is_empty(), "unsupported shape: {item}");
    }
}

#[test]
fn observed_parent_message_survives_restart_and_pins_child_to_its_account() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity.sqlite");
        let store = IdentityStore::open(path.clone()).unwrap();
        let binding = parent_binding(&store, "account-a", "owner-a");
        let message = format!("cipher-{name}");
        let item = call(name, &message);
        store
            .observe_event(
                &binding,
                &json!({"type":"response.output_item.done","item":item}),
            )
            .unwrap();
        drop(store);
        let store = IdentityStore::open(path).unwrap();
        let mut headers = HeaderMap::new();
        let mut body = body("child", child(&message));
        let original_input = body["input"].clone();
        let context = store.inspect("caller", &headers, &body).unwrap();
        assert_eq!(context.state_account_id.as_deref(), Some("account-a"));
        for (account, owner) in [("account-b", "owner-b"), ("account-a", "changed-owner")] {
            assert_eq!(
                store
                    .rewrite(&context, account, owner, &mut headers, &mut body)
                    .unwrap_err()
                    .code,
                "upstream_state_owner_changed"
            );
            assert_eq!(body["input"], original_input);
        }
        store
            .rewrite(&context, "account-a", "owner-a", &mut headers, &mut body)
            .unwrap();
        assert_eq!(body["input"], original_input);
        assert_ne!(headers["thread-id"], headers["session-id"]);
        assert_eq!(
            store
                .inspect("other-caller", &HeaderMap::new(), &body)
                .unwrap_err()
                .code,
            "upstream_state_caller_mismatch"
        );
        let other_binding = parent_binding(&store, "account-b", "owner-b");
        assert_eq!(
            store
                .observe_event(
                    &other_binding,
                    &json!({"type":"response.output_item.done","item":item})
                )
                .unwrap_err()
                .code,
            "upstream_state_owner_conflict"
        );
        let context = store.inspect("caller", &HeaderMap::new(), &body).unwrap();
        assert_eq!(context.state_account_id.as_deref(), Some("account-a"));
    }
}

#[test]
fn client_history_or_parent_identity_cannot_self_authorize_unseen_message_cipher() {
    let temp = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(temp.path().join("identity.sqlite")).unwrap();
    let _binding = parent_binding(&store, "account-a", "owner-a");
    for input in [
        call("spawn_agent", "unobserved-message"),
        child("unobserved-message"),
    ] {
        assert_eq!(
            store
                .inspect("caller", &HeaderMap::new(), &body("child", input))
                .unwrap_err()
                .code,
            "unknown_upstream_state"
        );
    }
    let count: i64 = store
        .connection
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM native_opaque_v1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn explicit_plaintext_task_does_not_pin_an_account_or_allow_cipher_replay() {
    let temp = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(temp.path().join("identity.sqlite")).unwrap();
    let binding = parent_binding(&store, "account-a", "owner-a");
    let mut plaintext = call("spawn_agent", "readable task");
    plaintext["encrypted_function_args"] = json!([]);
    store
        .observe_event(
            &binding,
            &json!({"type":"response.output_item.done","item":plaintext}),
        )
        .unwrap();
    let mut headers = HeaderMap::new();
    let mut request = body(
        "child",
        json!({"type":"agent_message","author":"/root","recipient":"/root/worker",
        "content":[{"type":"input_text","text":"readable task"}]}),
    );
    let context = store.inspect("caller", &headers, &request).unwrap();
    assert!(context.state_account_id.is_none());
    store
        .rewrite(&context, "account-b", "owner-b", &mut headers, &mut request)
        .unwrap();
    assert_eq!(
        store
            .inspect(
                "caller",
                &HeaderMap::new(),
                &body("other-child", child("readable task"))
            )
            .unwrap_err()
            .code,
        "unknown_upstream_state"
    );
}
