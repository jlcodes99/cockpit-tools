use super::*;

#[test]
fn chat_tool_envelopes_match_shared_go_golden() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/chat_tools_go.json")).unwrap();
    for case in cases {
        let converted =
            prepare_request(case["input"].clone(), ChatFlavor::ChatCompletions).unwrap();
        for field in ["tools", "tool_choice", "input"] {
            if field == "input" && case["expected"].get(field).is_none() {
                continue;
            }
            assert_eq!(
                converted.responses.get(field),
                case["expected"].get(field),
                "{} {field}",
                case["name"]
            );
        }
    }
}

#[test]
fn legacy_nested_custom_envelope_still_works() {
    let converted = prepare_request(json!({"model":"fixture","messages":[{"role":"user","content":"text"}],"tools":[{"type":"custom","custom":{"name":"legacy","format":{"type":"text"}}}],"tool_choice":{"type":"custom","custom":{"name":"legacy"}}}), ChatFlavor::ChatCompletions).unwrap();
    assert_eq!(
        converted.responses["tools"][0],
        json!({"type":"custom","name":"legacy","format":{"type":"text"}})
    );
    assert_eq!(
        converted.responses["tool_choice"],
        json!({"type":"custom","name":"legacy"})
    );
}

#[test]
fn empty_tool_type_remains_invalid() {
    assert!(prepare_request(
        json!({"model":"fixture","messages":[],"tools":[{"type":""}]}),
        ChatFlavor::ChatCompletions
    )
    .is_err());
}
