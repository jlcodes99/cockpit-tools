use super::super::{prepare, restore_event};
use super::*;

#[test]
fn grok_agent_message_replays_as_standard_message_with_routing_metadata() {
    let original = json!({"input":[{"type":"agent_message","id":"agent-1","author":"/root","recipient":"/root/child","internal_chat_message_metadata_passthrough":{"key":"value"},"content":[{"type":"input_text","text":"请继续任务"},{"type":"encrypted_content","encrypted_content":"legacy-literal-message","extension":7}]}]});
    let (wire, _) = prepare(original.clone()).unwrap();
    let item = &wire["input"][0];
    assert_eq!(item["type"], "message");
    assert_eq!(item["role"], "user");
    assert_eq!(item["content"][0], original["input"][0]["content"][0]);
    assert_eq!(
        item["content"][1],
        json!({"type":"input_text","text":"legacy-literal-message","extension":7})
    );
    let metadata = item["content"][2]["text"]
        .as_str()
        .unwrap()
        .strip_prefix("Agent routing metadata: ")
        .unwrap();
    let metadata: Value = serde_json::from_str(metadata).unwrap();
    for key in [
        "author",
        "recipient",
        "id",
        "internal_chat_message_metadata_passthrough",
    ] {
        assert!(item.get(key).is_none());
        assert_eq!(metadata[key], original["input"][0][key]);
    }
    assert_eq!(original["input"][0]["type"], "agent_message");
    assert_eq!(prepare(wire.clone()).unwrap().0, wire);
}

#[test]
fn grok_plaintext_parent_call_and_child_followup_roundtrip() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        let tools = json!([{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":name,"parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]}]);
        let (wire, mut context) = prepare(json!({"tools":tools,"input":[]})).unwrap();
        assert!(wire["tools"][0]["parameters"]["properties"]["message"]
            .get("encrypted")
            .is_none());
        let response = restore_event(&mut context, json!({"type":"response.completed","response":{"id":"parent-response","output":[{"type":"function_call","id":"fc-1","call_id":"call-1","name":wire["tools"][0]["name"],"arguments":"{\"message\":\"continue\"}"}]}})).unwrap().remove(0);
        assert_eq!(
            response["response"]["output"][0]["encrypted_function_args"],
            json!([])
        );
        let (child, _) = prepare(json!({"input":[{"type":"agent_message","author":"/root","recipient":"/root/child","content":[{"type":"input_text","text":"continue"}]},{"type":"message","role":"user","content":"下一步"}]})).unwrap();
        assert_eq!(child["input"][0]["type"], "message");
        assert_eq!(child["input"][0]["content"][0]["text"], "continue");
        assert_eq!(child["input"][1]["content"], "下一步");
    }
}

#[test]
fn grok_history_preserves_search_state_while_filling_required_query_fields() {
    for action in [
        json!({"type":"search","query":"query one"}),
        json!({"type":"search","queries":["query one","query two"]}),
    ] {
        let (wire, _) = prepare(json!({"input":[{"type":"web_search_call","encrypted_content":"opaque-search","action":action}]})).unwrap();
        assert_eq!(wire["input"][0]["action"]["query"], "query one");
        assert_eq!(wire["input"][0]["action"]["queries"][0], "query one");
        assert_eq!(wire["input"][0]["encrypted_content"], "opaque-search");
    }
    let unchanged = json!({"input":[{"type":"web_search_call","action":{"type":"open_page","url":"https://example.test"}},{"type":"function_call_output","call_id":"c","output":"{\"type\":\"agent_message\"}"}]});
    assert_eq!(prepare(unchanged.clone()).unwrap().0, unchanged);
}

#[test]
fn grok_compaction_alias_preserves_blob_exactly() {
    let (wire, _) = prepare(json!({"input":[{"type":"compaction_summary","encrypted_content":super::super::test_cipher(),"id":"cmp-1"}]})).unwrap();
    assert_eq!(
        wire["input"][0],
        json!({"type":"compaction","encrypted_content":super::super::test_cipher(),"id":"cmp-1"})
    );
}

#[test]
fn grok_compact_projection_replaces_only_the_completed_prefix() {
    let original = json!({"input":[
        {"type":"message","role":"user","content":"old"},
        {"type":"compaction","encrypted_content":super::super::test_cipher()},
        {"type":"message","role":"user","content":"continue"}
    ]});
    let (wire, _) = prepare(original.clone()).unwrap();
    assert_eq!(
        wire["input"],
        json!([original["input"][1], original["input"][2]])
    );
    assert_eq!(original["input"].as_array().unwrap().len(), 3);
    let original = json!({"input":[
        {"type":"message","role":"user","content":"recoverable old history"},
        {"type":"compaction","encrypted_content":"not-a-valid-grok-state"},
        {"type":"message","role":"user","content":"continue"}
    ]});
    let (wire, _) = prepare(original.clone()).unwrap();
    assert_eq!(
        wire["input"],
        json!([original["input"][0], original["input"][2]])
    );
}

#[test]
fn grok_empty_history_arguments_are_repaired_without_fabricating_current_output() {
    let (wire, _) = prepare(json!({"input":[
        {"type":"function_call","name":"no_args","call_id":"c","arguments":"  "},
        {"type":"function_call_output","call_id":"c","output":"done"},
        {"type":"function_call","name":"partial","call_id":"p","arguments":"{\"value\":"}
    ]}))
    .unwrap();
    assert_eq!(wire["input"][0]["arguments"], "{}");
    assert_eq!(wire["input"][2]["arguments"], "{\"value\":");
    let mut current = json!({"arguments":""});
    normalize_arguments(&mut current);
    assert_eq!(current["arguments"], "");
}

#[test]
fn grok_reasoning_replay_merges_only_unidentified_summary_fragments() {
    let original = json!({"input":[
        {"type":"reasoning","id":"rs-1","encrypted_content":super::super::test_cipher(),"summary":[],"content":null},
        {"type":"reasoning","summary":[{"type":"summary_text","text":"fragment"}]},
        {"type":"reasoning","id":"rs-2","summary":[{"type":"summary_text","text":"identified"}]}
    ]});
    let (wire, _) = prepare(original.clone()).unwrap();
    assert_eq!(wire["input"].as_array().unwrap().len(), 2);
    assert_eq!(wire["input"][0]["id"], "rs-1");
    assert_eq!(
        wire["input"][0]["encrypted_content"],
        original["input"][0]["encrypted_content"]
    );
    assert!(wire["input"][0].get("content").is_none());
    assert_eq!(wire["input"][0]["summary"][0]["text"], "fragment");
    assert_eq!(wire["input"][1], original["input"][2]);
}
