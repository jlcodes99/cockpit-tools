use super::*;
use serde_json::json;

#[test]
fn capability_collection_includes_effective_declarations_choices_and_history_without_mutation() {
    let body = json!({"tools":[
        {"type":"tool_search","execution":"client"},
        {"type":"namespace","name":"local","tools":[{"type":"function","name":"shell","allowed_callers":["programmatic"],"parameters":{"properties":{"encrypted_content":{"type":"string"},"tools":{"type":"string"}}}}]}
    ],"tool_choice":{"type":"allowed_tools","tools":[{"type":"tool_search"}]},"input":[
        {"type":"additional_tools","tools":[{"type":"custom","name":"patch"}]},
        {"type":"tool_search_output","execution":"client","call_id":"search","output":"{\"tools\":[{\"type\":\"mcp\"}]}"},
        {"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AA=="},{"type":"input_file","file_data":"data:text/plain;base64,AA=="}]}
    ]});
    let original = body.clone();
    let ids = collect(&body, true);
    for id in [
        "function",
        "custom",
        "namespace",
        "programmatic_tool_calling",
        "tool_search_client",
        "mcp",
        "image_input",
        "file_input",
        "compact",
    ] {
        assert!(ids.iter().any(|v| v == id), "{id}: {ids:?}");
    }
    assert!(!ids
        .iter()
        .any(|v| v == "tool_search" || v == "unknown_tool"));
    assert_eq!(body, original);
    assert!(!encrypted(&body));
    assert!(encrypted(
        &json!({"input":[{"type":"reasoning","encrypted_content":"cipher"}]})
    ));
}

#[test]
fn capability_search_history_requires_complete_unambiguous_pairing() {
    let call = json!({"type":"tool_search_call","execution":"client","call_id":"s","arguments":{}});
    let result = json!({"type":"tool_search_output","execution":"client","call_id":"s","tools":[]});
    assert!(validate_replay(&HeaderMap::new(), &json!({"input":[call,result]})).is_ok());
    for input in [
        json!([call]),
        json!([result]),
        json!([call, result, result]),
        json!([call, call, result]),
    ] {
        assert!(validate_replay(&HeaderMap::new(), &json!({"input":input})).is_err());
    }
    for body in [
        json!({"input":[],"previous_response_id":"unknown"}),
        json!({"input":[{"type":"additional_tools","tools":[{"type":"file_search","vector_store_ids":["vs_private"]}]}]}),
        json!({"input":[{"role":"user","content":[{"type":"input_file","file_id":"file_private"}]}]}),
        json!({"input":[],"tools":[{"type":"code_interpreter","container":"container_private"}]}),
    ] {
        assert!(validate_replay(&HeaderMap::new(), &body).is_err());
    }
}

#[test]
fn only_precise_pre_execution_capability_errors_are_eligible() {
    let error = json!({"error":{"code":"unsupported_tool","param":"tools[0].type","message":"Unavailable"}});
    assert!(pre_execution_rejection(400, &error));
    assert!(!pre_execution_rejection(403, &error));
    for value in [
        json!({"error":{"code":"invalid_request_error","param":"tools[0]"}}),
        json!({"error":{"code":"missing_required_parameter","param":"input[1].role"}}),
        json!({"error":{"code":"permission_denied","param":"tools"}}),
        json!({"error":{"code":"unsupported_tool","param":"model"}}),
        json!({"error":{"code":"unsupported_tool","param":"tools"},"usage":{"output_tokens":1}}),
        json!({"error":{"code":"unsupported_tool","param":"tools"},"response":{"id":"r"}}),
        json!({"error":{"code":"unsupported_tool","param":"tools","type":"tool_execution_error"}}),
        json!({"error":{"code":"unsupported_tool","param":"tools","executed":true}}),
    ] {
        assert!(!pre_execution_rejection(400, &value), "{value}");
    }
}
