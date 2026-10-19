use super::*;

fn host_output(name: &str) -> Value {
    json!({
        "type":"function_call_output", "namespace":"codex_app", "name":name,
        "id":format!("host-{name}"), "output":"{\"message\":\"原值\"}",
        "internal_chat_message_metadata_passthrough":{"nested":[null,1,{"retain":true}]},
        "extension":{"do_not_parse":"call_id=unrelated"}
    })
}

#[test]
fn host_handoffs_among_function_and_custom_pairs_keep_all_fields_and_order() {
    let mut body = json!({"extension":42,"input":[
        host_output("create_thread"),
        {"type":"function_call","name":"lookup","call_id":"function-1","arguments":"{}"},
        host_output("send_message_to_thread"),
        {"type":"custom_tool_call","name":"apply_patch","call_id":"custom-1","input":"synthetic"},
        host_output("create_thread"),
        {"type":"function_call_output","call_id":"function-1","output":"lookup result"},
        {"type":"custom_tool_call_output","call_id":"custom-1","output":"custom result"},
        host_output("send_message_to_thread")
    ]});
    let before = body.clone();
    normalize(&mut body);
    assert_eq!(
        body, before,
        "no host field, call ID or input position may change"
    );
    normalize(&mut body);
    assert_eq!(body, before, "wire normalization remains idempotent");
}

#[test]
fn host_handoffs_do_not_consume_pending_ids_needed_by_real_outputs() {
    let host = host_output("create_thread");
    let mut body = json!({"input":[
        {"type":"function_call","namespace":"other","name":"create_thread","call_id":"other-call","arguments":"{}"},
        {"type":"function_call","name":"lookup","call_id":"lookup-call","arguments":"{}"},
        host,
        {"type":"function_call_output","name":"lookup","output":"lookup result"},
        {"type":"function_call_output","name":"create_thread","output":"other result"}
    ]});
    normalize(&mut body);
    assert_eq!(body["input"].as_array().unwrap().len(), 5);
    assert_eq!(body["input"][2], host);
    assert!(body["input"][2].get("call_id").is_none());
    assert_eq!(body["input"][3]["call_id"], "lookup-call");
    assert_eq!(body["input"][4]["call_id"], "other-call");
}

#[test]
fn ambiguous_handoffs_are_retained_but_never_repaired_into_real_outputs() {
    for name in ["create_thread", "send_message_to_thread"] {
        for call_first in [true, false] {
            let host = host_output(name);
            let call = json!({"type":"function_call","namespace":"codex_app","name":name,"call_id":"real-call","arguments":"{}"});
            let items = if call_first {
                vec![call, host.clone()]
            } else {
                vec![host.clone(), call]
            };
            let mut body = json!({"input":items});
            let before = body.clone();
            normalize(&mut body);
            assert_eq!(body, before);
            assert!(body["input"][usize::from(call_first)]
                .get("call_id")
                .is_none());
        }
    }
}

#[test]
fn ordinary_tool_repair_still_uses_its_existing_pairing_rules() {
    let mut body = json!({"input":[
        {"type":"function_call","name":"lookup","arguments":"{}"},
        {"type":"function_call_output","name":"lookup","output":"result"}
    ]});
    normalize(&mut body);
    assert_eq!(body["input"][0]["call_id"], "call_missing_0");
    assert_eq!(body["input"][1]["call_id"], "call_missing_0");

    let mut result = host_output("create_thread");
    result["call_id"] = json!("real-host-call");
    let mut explicit = json!({"input":[
        {"type":"function_call","namespace":"codex_app","name":"create_thread","call_id":"real-host-call","arguments":"{}"},
        result
    ]});
    let before = explicit.clone();
    normalize(&mut explicit);
    assert_eq!(
        explicit, before,
        "real host tool calls with explicit result IDs remain ordinary pairs"
    );
}
