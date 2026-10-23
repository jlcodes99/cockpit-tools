//! Grok's ordinary JSON message arguments are plaintext, not official ciphertext.
use crate::modules::codex_native_api::{grok_wire, identity::IdentityStore};
use axum::http::HeaderMap;
use serde_json::{json, Value};

fn prepared(namespace: &str, name: &str) -> (Value, grok_wire::Context) {
    grok_wire::prepare(json!({"tools":[{"type":"namespace","name":namespace,"tools":[{"type":"function","name":name,"parameters":{"type":"object","properties":{"message":{"type":"string"}}}}]}],"input":[]})).unwrap()
}
fn item(wire_name: &Value, message: &str) -> Value {
    json!({"type":"function_call","id":"item","call_id":"call","name":wire_name,"arguments":json!({"message":message,"target":"/root/worker"}).to_string(),"status":"completed","extension":"preserved"})
}
fn restored_completed(context: &mut grok_wire::Context, item: Value) -> Value {
    grok_wire::restore_event(context,json!({"type":"response.completed","response":{"id":"parent","status":"completed","output":[item]}})).unwrap().remove(0)
}
fn child(cipher: &str) -> Value {
    json!({"client_metadata":{"session_id":"session","thread_id":"child","parent_thread_id":"parent"},"input":[{"type":"agent_message","author":"/root","recipient":"/root/worker","content":[{"type":"encrypted_content","encrypted_content":cipher}]}]})
}
fn observe_and_inspect(event: &Value, cipher: &str, expected_owned: bool) {
    let directory = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(directory.path().join("identity.sqlite")).unwrap();
    let mut source =
        json!({"client_metadata":{"session_id":"session","thread_id":"parent"},"input":[]});
    let mut headers = HeaderMap::new();
    let context = store.inspect("caller", &headers, &source).unwrap();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut source)
        .unwrap();
    store.observe_event(&binding, event).unwrap();
    let context = store.inspect("caller", &HeaderMap::new(), &child(cipher));
    if expected_owned {
        assert_eq!(context.unwrap().state_account_id.as_deref(), Some("a"));
    } else {
        assert_eq!(context.unwrap_err().code, "unknown_upstream_state");
    }
}

#[test]
fn grok_collaboration_plaintext_is_explicit_and_never_issues_cipher_ownership() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        let (wire, mut context) = prepared("collaboration", name);
        let original = item(&wire["tools"][0]["name"], "ordinary message 250ms");
        let restored = restored_completed(&mut context, original.clone());
        let tool = &restored["response"]["output"][0];
        assert_eq!(tool["namespace"], "collaboration");
        assert_eq!(tool["name"], name);
        assert_eq!(tool["encrypted_function_args"], json!([]));
        for field in ["arguments", "id", "call_id", "extension"] {
            assert_eq!(tool[field], original[field]);
        }
        observe_and_inspect(&restored, "ordinary message 250ms", false);
        let (wire, mut context) = prepared("collaboration", name);
        let original = item(&wire["tools"][0]["name"], "ordinary stream message");
        let restored = grok_wire::restore_event(
            &mut context,
            json!({"type":"response.output_item.done","output_index":0,"item":original}),
        )
        .unwrap()
        .remove(0);
        assert_eq!(restored["item"]["encrypted_function_args"], json!([]));
        observe_and_inspect(&restored, "ordinary stream message", false);
    }
}

#[test]
fn grok_collaboration_real_opaque_metadata_is_preserved_and_owned() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        for top_cipher in [false, true] {
            let (wire, mut context) = prepared("collaboration", name);
            let mut original = item(&wire["tools"][0]["name"], "synthetic-message-cipher");
            if top_cipher {
                original["encrypted_content"] = json!("synthetic-top-cipher");
            } else {
                original["encrypted_function_args"] = json!(["synthetic-args-cipher"]);
            }
            let restored = restored_completed(&mut context, original.clone());
            let tool = &restored["response"]["output"][0];
            assert_eq!(tool["arguments"], original["arguments"]);
            if top_cipher {
                assert_eq!(tool["encrypted_content"], original["encrypted_content"]);
                assert!(tool.get("encrypted_function_args").is_none());
            } else {
                assert_eq!(
                    tool["encrypted_function_args"],
                    original["encrypted_function_args"]
                );
            }
            observe_and_inspect(&restored, "synthetic-message-cipher", true);
            observe_and_inspect(
                &restored,
                if top_cipher {
                    "synthetic-top-cipher"
                } else {
                    "synthetic-args-cipher"
                },
                true,
            );
        }
    }
}

#[test]
fn grok_unknown_tool_namespace_and_partial_items_do_not_acquire_plaintext_markers() {
    for (namespace, name) in [
        ("collaboration", "unknown_tool"),
        ("functions", "spawn_agent"),
    ] {
        let (wire, mut context) = prepared(namespace, name);
        let original = item(&wire["tools"][0]["name"], "ordinary message");
        let restored = restored_completed(&mut context, original.clone());
        let tool = &restored["response"]["output"][0];
        assert_eq!(tool["namespace"], namespace);
        assert_eq!(tool["name"], name);
        assert!(tool.get("encrypted_function_args").is_none());
        assert_eq!(tool["arguments"], original["arguments"]);
        observe_and_inspect(&restored, "ordinary message", false);
    }
    let (wire, mut context) = prepared("collaboration", "spawn_agent");
    let mut partial = item(&wire["tools"][0]["name"], "ordinary message");
    partial["status"] = json!("in_progress");
    partial["arguments"] = json!("");
    let event = grok_wire::restore_event(
        &mut context,
        json!({"type":"response.output_item.added","output_index":0,"item":partial}),
    )
    .unwrap()
    .remove(0);
    assert!(event["item"].get("encrypted_function_args").is_none());
    let original = item(&json!("unregistered_wire_name"), "ordinary message");
    let restored = restored_completed(&mut context, original.clone());
    assert_eq!(restored["response"]["output"][0], original);
}
