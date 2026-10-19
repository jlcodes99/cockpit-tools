use super::*;
use serde_json::json;

fn output(name: &str) -> Value {
    json!({"type":"function_call_output","namespace":"codex_app","name":name,"output":"host result"})
}

#[test]
fn exact_handoff_envelopes_need_no_metadata_marker_and_are_not_mutated() {
    for name in ["create_thread", "send_message_to_thread"] {
        for metadata in [
            None,
            Some(Value::Null),
            Some(json!({"extension":[1,"原值"]})),
        ] {
            let mut item = output(name);
            item["id"] = json!("host-item");
            item["extension"] = json!({"nested":[true,null,{"a":"b"}]});
            if let Some(metadata) = metadata {
                item["internal_chat_message_metadata_passthrough"] = metadata;
            }
            let items = vec![item];
            let before = items.clone();
            assert_eq!(
                Classifier::new(&items).classify(&items[0]),
                Classification::Standalone
            );
            assert_eq!(items, before);
        }
    }
}

#[test]
fn only_exact_shapes_qualify_and_a_present_invalid_call_id_is_not_missing() {
    for (field, value) in [
        ("type", json!("custom_tool_call_output")),
        ("type", json!(" FUNCTION_CALL_OUTPUT ")),
        ("namespace", json!("other")),
        ("namespace", json!(" codex_app ")),
        ("namespace", Value::Null),
        ("name", json!("delete_thread")),
        ("name", json!("codex_app.create_thread")),
        ("name", json!(" create_thread ")),
        ("name", json!("Create_Thread")),
        ("name", Value::Null),
        ("output", Value::Null),
        ("output", json!([{"type":"input_text","text":"result"}])),
        ("output", json!({"result":"text"})),
        ("call_id", Value::Null),
        ("call_id", json!("")),
        ("call_id", json!(" ")),
        ("call_id", json!(42)),
        ("call_id", json!("call-1")),
    ] {
        let mut item = output("create_thread");
        item[field] = value;
        item["internal_chat_message_metadata_passthrough"] = json!({"host":true});
        assert_eq!(
            Classifier::new(&[item.clone()]).classify(&item),
            Classification::Ordinary,
            "{item}"
        );
    }
    for field in ["type", "namespace", "name", "output"] {
        let mut item = output("create_thread");
        item.as_object_mut().unwrap().remove(field);
        assert_eq!(
            Classifier::new(&[item.clone()]).classify(&item),
            Classification::Ordinary,
            "missing {field}"
        );
    }
}

#[test]
fn a_same_qualified_call_anywhere_makes_only_that_handoff_name_ambiguous() {
    for name in ["create_thread", "send_message_to_thread"] {
        for call in [
            json!({"type":"function_call","namespace":"codex_app","name":name,"call_id":"real-call","arguments":"{}"}),
            json!({"type":"function_call","name":format!("codex_app.{name}"),"call_id":"real-call","arguments":"{}"}),
            json!({"type":"function_call","name":format!("codex_app__{name}"),"call_id":"real-call","arguments":"{}"}),
            json!({"type":"function_call","name":format!("mcp__codex_app__{name}"),"call_id":"real-call","arguments":"{}"}),
            json!({"type":" FUNCTION_CALL ","namespace":"codex_app","name":format!(" {name} "),"arguments":"{}"}),
        ] {
            for call_first in [false, true] {
                let item = output(name);
                let items = if call_first {
                    vec![call.clone(), item.clone()]
                } else {
                    vec![item.clone(), call.clone()]
                };
                let classifier = Classifier::new(&items);
                assert_eq!(
                    classifier.classify(&item),
                    Classification::Ambiguous,
                    "{items:?}"
                );
                let other_name = if name == "create_thread" {
                    "send_message_to_thread"
                } else {
                    "create_thread"
                };
                assert_eq!(
                    classifier.classify(&output(other_name)),
                    Classification::Standalone
                );
            }
        }
    }
}

#[test]
fn flattened_call_aliases_never_expand_the_allowed_output_names() {
    for name in ["create_thread", "send_message_to_thread"] {
        for prefix in ["codex_app.", "codex_app__", "mcp__codex_app__"] {
            for namespace in [Some(json!("codex_app")), Some(Value::Null), None] {
                let mut item = output(&format!("{prefix}{name}"));
                if let Some(namespace) = namespace {
                    item["namespace"] = namespace;
                } else {
                    item.as_object_mut().unwrap().remove("namespace");
                }
                assert_eq!(
                    Classifier::new(&[item.clone()]).classify(&item),
                    Classification::Ordinary,
                    "qualified aliases are conflicts for real calls, never standalone output names: {item}"
                );
            }
        }
    }
}

#[test]
fn unrelated_calls_cannot_claim_a_host_notification() {
    let items = vec![
        json!({"type":"function_call","namespace":"other","name":"create_thread","call_id":"other-call","arguments":"{}"}),
        json!({"type":"function_call","name":"create_thread","call_id":"unnamespaced-call","arguments":"{}"}),
        json!({"type":"function_call","namespace":"codex_app","name":"list_threads","call_id":"list-call","arguments":"{}"}),
        output("create_thread"),
    ];
    assert_eq!(
        Classifier::new(&items).classify(&items[3]),
        Classification::Standalone
    );
}
