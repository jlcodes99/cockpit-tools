use super::super::{prepare_request, response_from_responses, CompatFlavor, CompatStreamAdapter};
use super::*;

#[test]
fn long_utf8_tool_names_map_schema_calls_choices_and_restore_outputs_without_touching_history() {
    let first = format!("mcp__{}__first", "工具".repeat(20));
    let second = format!("mcp__{}__second", "工具".repeat(20));
    let arguments = json!({"name":first,"encrypted_content":"keep-this-value"});
    let request=prepare_request(json!({"model":"m","messages":[
        {"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":first,"input":arguments}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"original text"}]}],
        "tools":[{"name":first,"input_schema":{"type":"object","properties":{"name":{"type":"string","description":first}}}},{"name":second,"input_schema":{"type":"object"}}],
        "tool_choice":{"type":"tool","name":first}}),CompatFlavor::Claude,None,None).unwrap();
    let alias = request.responses["tools"][0]["name"].as_str().unwrap();
    let other = request.responses["tools"][1]["name"].as_str().unwrap();
    assert!(alias.len() <= 64 && legal_name(alias));
    assert!(other.len() <= 64 && legal_name(other));
    assert_ne!(alias, other);
    assert_eq!(request.responses["input"][0]["name"], alias);
    assert_eq!(request.responses["tool_choice"]["name"], alias);
    assert_eq!(
        request.responses["input"][0]["arguments"],
        arguments.to_string()
    );
    assert_eq!(request.responses["input"][1]["call_id"], "call-1");
    assert_eq!(request.responses["input"][1]["output"], "original text");
    assert_eq!(
        request.responses["tools"][0]["parameters"]["properties"]["name"]["description"],
        first
    );
    // The scalar is the Go-compatible wire form of the same text. Verify the
    // native Chat adapter keeps it and the long-name/call identity unchanged.
    let wire = crate::modules::codex_native_api::chat_upstream::responses_to_native_chat(
        &request.responses,
    )
    .unwrap();
    assert_eq!(wire["messages"][1]["content"], "original text");
    assert_eq!(wire["messages"][1]["tool_call_id"], "call-1");
    assert_eq!(
        wire["messages"][0]["tool_calls"][0]["function"]["name"],
        alias
    );
    assert_eq!(
        wire["messages"][0]["tool_calls"][0]["function"]["arguments"],
        arguments.to_string()
    );
    assert_eq!(wire["tool_choice"]["function"]["name"], alias);
    let item =
        json!({"type":"function_call","call_id":"call-2","name":alias,"arguments":"{\"x\":1}"});
    let output = json!({"id":"r","status":"completed","output":[item],"usage":{}});
    assert_eq!(
        response_from_responses(&output, &request.context).unwrap()["content"][0]["name"],
        first
    );
    let mut stream = CompatStreamAdapter::new(request.context);
    let mut frames = stream
        .push(&json!({"type":"response.output_item.added","output_index":0,"item":item}))
        .unwrap();
    frames.extend(
        stream
            .push(&json!({"type":"response.completed","response":output}))
            .unwrap(),
    );
    assert!(frames
        .iter()
        .any(|frame| frame.data.pointer("/content_block/name") == Some(&json!(first))));
}

#[test]
fn generated_alias_never_collides_with_existing_short_tool_names() {
    let long = "f".repeat(90);
    let mut first = json!({"tools":[{"type":"function","name":long}]});
    map(&mut first);
    let reserved = first["tools"][0]["name"].as_str().unwrap().to_owned();
    let mut body = json!({"tools":[{"type":"function","name":long},{"type":"function","name":reserved}],"tool_choice":{"type":"allowed_tools","tools":[{"type":"function","name":long}]}});
    let reverse = map(&mut body);
    let alias = body["tools"][0]["name"].as_str().unwrap();
    assert_ne!(alias, reserved);
    assert!(alias.len() <= 64);
    assert_eq!(body["tools"][1]["name"], reserved);
    assert_eq!(body["tool_choice"]["tools"][0]["name"], alias);
    assert_eq!(reverse[alias], long);
}

#[test]
fn claude_search_history_cannot_drop_unrepresentable_encrypted_result() {
    let error=prepare_request(json!({"model":"m","messages":[{"role":"assistant","content":[
        {"type":"server_tool_use","id":"search","name":"web_search","input":{"query":"test"}},
        {"type":"web_search_tool_result","tool_use_id":"search","content":[{"type":"web_search_result","url":"https://example.test","title":"test","encrypted_content":"opaque"}]}]}]}),CompatFlavor::Claude,None,None).err().unwrap();
    assert_eq!(error.code, "compat_parameter_not_representable");
    assert!(error.message.contains("encrypted_content"));
}

#[test]
fn invalid_compat_tool_names_reserve_legal_names_and_restore_short_calls() {
    let long = "工具🙂".repeat(50);
    let originals = ["a.b", "a/b", "a_b", "读取📁", long.as_str()];
    let tools: Vec<Value> = originals.iter().map(|name| json!({
        "name":name,"input_schema":{"type":"object","properties":{"original":{"type":"string","description":name}}}
    })).collect();
    let arguments = json!({"original":"a.b", "text":"读取📁", "nested":{"name":"a_b"}});
    let request = prepare_request(json!({"model":"m", "messages":[
        {"role":"assistant","content":[{"type":"tool_use","id":"saved-call","name":"a.b","input":arguments}]},
        {"role":"user","content":[{"type":"tool_result","tool_use_id":"saved-call","content":"a.b result 读取📁"}]}
    ],"tools":tools,"tool_choice":{"type":"tool","name":"a.b"}}), CompatFlavor::Claude, None, None).unwrap();
    let aliases: Vec<&str> = request.responses["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(aliases
        .iter()
        .all(|name| name.len() <= 64 && legal_name(name)));
    assert_eq!(
        aliases[2], "a_b",
        "legal user-owned names must never be renamed"
    );
    assert_ne!(aliases[0], "a_b");
    assert_ne!(aliases[1], "a_b");
    assert_eq!(
        aliases.iter().copied().collect::<BTreeSet<_>>().len(),
        aliases.len()
    );
    let alias = aliases[0].to_owned();
    assert_eq!(request.responses["input"][0]["name"], alias);
    assert_eq!(request.responses["tool_choice"]["name"], alias);
    assert_eq!(
        request.responses["input"][0]["arguments"],
        arguments.to_string()
    );
    assert_eq!(request.responses["input"][1]["call_id"], "saved-call");
    assert_eq!(request.responses["input"][1]["output"], "a.b result 读取📁");
    assert_eq!(
        request.responses["tools"][0]["parameters"]["properties"]["original"]["description"],
        "a.b"
    );
    let item = json!({"type":"function_call","id":"fc_new","call_id":"new-call","name":alias,"arguments":"{}"});
    let response = json!({"id":"r","status":"completed","output":[item],"usage":{}});
    assert_eq!(
        response_from_responses(&response, &request.context).unwrap()["content"][0]["name"],
        "a.b"
    );
    let mut adapter = CompatStreamAdapter::new(request.context);
    let mut frames = adapter
        .push(&json!({"type":"response.output_item.added","output_index":0,"item":item}))
        .unwrap();
    frames.extend(
        adapter
            .push(&json!({"type":"response.completed","response":response}))
            .unwrap(),
    );
    assert!(frames
        .iter()
        .any(|frame| frame.data.pointer("/content_block/name") == Some(&json!("a.b"))));
}

#[test]
fn compat_name_mapping_is_order_independent_and_preserves_native_namespace_shapes() {
    let mut forward = json!({"tools":[
        {"type":"function","name":"a.b"},
        {"type":"function","name":"a/b"},
        {"type":"function","name":"a_b"},
        {"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"leave.native"}]},
        {"type":"custom","name":"leave.custom","format":{"type":"text"}}
    ],"tool_choice":{"type":"allowed_tools","tools":[{"type":"function","name":"a.b"}]}});
    let namespace = forward["tools"][3].clone();
    let custom = forward["tools"][4].clone();
    let mut reversed = forward.clone();
    reversed["tools"].as_array_mut().unwrap().reverse();
    let first = map(&mut forward);
    let second = map(&mut reversed);
    assert_eq!(first, second);
    assert_eq!(forward["tools"][3], namespace);
    assert_eq!(forward["tools"][4], custom);
    assert_eq!(
        forward["tools"][0]["name"],
        forward["tool_choice"]["tools"][0]["name"]
    );
    assert_eq!(forward["tools"][2]["name"], "a_b");
}
