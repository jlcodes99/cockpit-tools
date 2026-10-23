use super::super::{prepare, restore_event};
use super::*;

fn declaration() -> Value {
    json!({"type":"tool_search","execution":"client","description":"Find tools by goal","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"],"additionalProperties":false}})
}

#[test]
fn client_search_keeps_schema_and_restores_complete_response() {
    let original = declaration();
    let (body, mut context) =
        prepare(json!({"tools":[original],"tool_choice":{"type":"tool_search"}})).unwrap();
    let tool = &body["tools"][0];
    assert_eq!(tool["type"], "function");
    assert_eq!(tool["parameters"], original["parameters"]);
    assert_eq!(tool["description"], original["description"]);
    assert!(tool.get("execution").is_none());
    assert_eq!(
        body["tool_choice"],
        json!({"type":"function","name":tool["name"]})
    );
    let reply = json!({"id":"response-1","status":"completed","tools":body["tools"],"output":[{"type":"function_call","name":tool["name"],"id":"fc_search1","call_id":"call_search1","arguments":"{\"goal\":\"find child agent\"}","status":"completed","vendor":"keep"}]});
    let out = restore_event(&mut context, reply).unwrap().remove(0);
    assert_eq!(out["tools"], json!([original]));
    let item = &out["output"][0];
    assert_eq!(item["type"], "tool_search_call");
    assert_eq!(item["execution"], "client");
    assert_eq!(item["arguments"], json!({"goal":"find child agent"}));
    assert_eq!(item["id"], "tsc_search1");
    assert_eq!(item["call_id"], "call_search1");
    assert_eq!(item["vendor"], "keep");
    assert!(item.get("name").is_none());
}

#[test]
fn search_stream_buffers_arguments_and_keeps_call_identity() {
    let (body, mut context) = prepare(json!({"tools":[declaration()]})).unwrap();
    let added=restore_event(&mut context,json!({"type":"response.output_item.added","output_index":3,"item":{"type":"function_call","id":"fc_123","call_id":"call_123","name":body["tools"][0]["name"],"arguments":""}})).unwrap();
    assert_eq!(added[0]["item"]["id"], "tsc_123");
    assert_eq!(added[0]["item"]["arguments"], json!({}));
    for delta in ["{\"goal\":\"查", "找\\n工具\"}"] {
        assert!(restore_event(&mut context,json!({"type":"response.function_call_arguments.delta","item_id":"fc_123","delta":delta})).unwrap().is_empty());
    }
    assert!(restore_event(
        &mut context,
        json!({"type":"response.function_call_arguments.done","output_index":3})
    )
    .unwrap()
    .is_empty());
    let done=restore_event(&mut context,json!({"type":"response.output_item.done","output_index":3,"item":{"type":"function_call","id":"fc_123","call_id":"call_123","name":body["tools"][0]["name"],"arguments":"","status":"completed"}})).unwrap();
    assert_eq!(done[0]["item"]["arguments"], json!({"goal":"查找\n工具"}));
    assert_eq!(done[0]["item"]["execution"], "client");
    assert_eq!(done[0]["item"]["call_id"], "call_123");
    assert_eq!(done[0]["item"]["id"], "tsc_123");
}

#[test]
fn discovery_promotes_agent_tools_and_preserves_opaque_history() {
    let discovered = json!([{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"spawn_agent","defer_loading":true,"parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true}}}}]}]);
    let history = json!([
        {"type":"reasoning","encrypted_content":test_cipher(),"summary":[]},
        {"type":"tool_search_call","execution":"client","id":"tsc_123","call_id":"call_123","arguments":{"goal":"agent"},"status":"completed"},
        {"type":"tool_search_output","execution":"client","call_id":"call_123","status":"completed","tools":discovered},
        {"type":"function_call","name":"spawn_agent","namespace":"collaboration","call_id":"child_123","arguments":"{\"message\":\"opaque-agent\"}"}
    ]);
    let (body, mut context) = prepare(json!({"tools":[declaration()],"input":history})).unwrap();
    assert_eq!(body["input"][0], history[0]);
    assert_eq!(body["input"][1]["id"], "fc_123");
    assert_eq!(body["input"][1]["call_id"], "call_123");
    assert_eq!(body["input"][1]["arguments"], "{\"goal\":\"agent\"}");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(
        serde_json::from_str::<Value>(body["input"][2]["output"].as_str().unwrap()).unwrap(),
        discovered
    );
    assert_eq!(body["tools"].as_array().unwrap().len(), 2);
    assert!(body["tools"][1].get("defer_loading").is_none());
    assert_eq!(body["input"][3]["name"], body["tools"][1]["name"]);
    assert_eq!(body["input"][3]["arguments"], history[3]["arguments"]);
    let restored=restore_event(&mut context,json!({"output":[{"type":"function_call","name":body["tools"][1]["name"],"call_id":"new_child","arguments":"{\"message\":\"do work\"}"}]})).unwrap();
    assert_eq!(restored[0]["output"][0]["name"], "spawn_agent");
    assert_eq!(restored[0]["output"][0]["namespace"], "collaboration");
}

#[test]
fn explicit_server_search_and_deferred_schema_are_preserved() {
    let original = json!({"tools":[{"type":"tool_search","execution":"server"},{"type":"function","name":"lookup","defer_loading":true,"parameters":{"type":"object"}}],"tool_choice":{"type":"tool_search","execution":"server"},"input":[{"type":"tool_search_call","execution":"server","id":"tsc_native","arguments":{"query":"native"}}]});
    assert_eq!(prepare(original.clone()).unwrap().0, original);
}

#[test]
fn legacy_search_and_colliding_function_names_remain_distinct() {
    let (first, _) = prepare(json!({"tools":[{"type":"tool_search"}]})).unwrap();
    let alias = first["tools"][0]["name"].as_str().unwrap();
    let (body,mut context)=prepare(json!({"tools":[{"type":"tool_search"},{"type":"function","name":alias,"parameters":{"type":"object"}}]})).unwrap();
    assert_ne!(body["tools"][0]["name"], alias);
    let native = json!({"output":[{"type":"function_call","name":alias,"call_id":"native","arguments":"{}"}]});
    assert_eq!(
        restore_event(&mut context, native.clone()).unwrap(),
        vec![native]
    );
}

#[test]
fn discovery_does_not_override_current_schema_or_explicitly_disabled_tools() {
    let current = json!({"type":"function","name":"lookup","parameters":{"type":"object","properties":{"new":{"type":"boolean"}}}});
    let old = json!({"type":"function","name":"lookup","parameters":{"type":"object","properties":{"old":{"type":"boolean"}}}});
    let history = json!([{"type":"tool_search_output","execution":"client","call_id":"search","status":"completed","tools":[old]}]);
    let (body, _) = prepare(json!({"tools":[current],"input":history})).unwrap();
    assert_eq!(body["tools"], json!([current]));
    let (body, _) = prepare(json!({"tools":[],"input":history})).unwrap();
    assert!(body.get("tools").is_none());
}

#[test]
fn search_history_errors_fail_without_fabricating_results() {
    for history in [
        json!([{"type":"tool_search_output","execution":"client","call_id":"x"}]),
        json!([{"type":"tool_search_output","execution":"client","tools":[]}]),
        json!([{"type":"tool_search_call","execution":"client","call_id":"x","arguments":"{"}]),
        json!([{"type":"tool_search_call","execution":"client","call_id":"x","arguments":[]}]),
    ] {
        assert!(prepare(json!({"tools":[declaration()],"input":history})).is_err());
    }
    let (body, mut context) = prepare(json!({"tools":[declaration()]})).unwrap();
    assert!(restore_event(&mut context,json!({"output":[{"type":"function_call","name":body["tools"][0]["name"],"arguments":"[]"}]})).is_err());
}

#[test]
fn search_stream_obeys_existing_resource_limit() {
    let (body, mut context) = prepare(json!({"tools":[declaration()]})).unwrap();
    restore_event(&mut context,json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","name":body["tools"][0]["name"],"id":"fc_big","call_id":"big","arguments":""}})).unwrap();
    let error=restore_event(&mut context,json!({"type":"response.function_call_arguments.delta","item_id":"fc_big","delta":"x".repeat(MAX_STATE_BYTES+1)})).unwrap_err();
    assert_eq!(error.code, "invalid_grok_tool_arguments");
}

#[test]
fn additional_tools_and_legacy_outputs_preserve_discovered_definitions() {
    let input = json!([
        {"type":"additional_tools","tools":[declaration()]},
        {"type":"tool_search_call","execution":"client","id":"tsc_known","call_id":"known","arguments":{"goal":"tools"}},
        {"type":"tool_search_output","call_id":"known","tools":[{"type":"function","name":"lookup","defer_loading":true,"parameters":{"type":"object"}}]}
    ]);
    let (body, _) = prepare(json!({"input":input})).unwrap();
    assert_eq!(body["tools"].as_array().unwrap().len(), 2);
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][1]["name"], "lookup");
    assert!(body["tools"][1].get("defer_loading").is_none());
    assert_eq!(body["input"].as_array().unwrap().len(), 2);
    assert_eq!(body["input"][1]["type"], "function_call_output");
}

#[test]
fn legacy_output_discovers_latest_schema_without_overwriting_current_tools() {
    let old = json!({"type":"function","name":"lookup","parameters":{"type":"object","properties":{"old":{"type":"string"}}}});
    let new = json!({"type":"function","name":"lookup","parameters":{"type":"object","properties":{"new":{"type":"string"}}}});
    let input = json!([
        {"type":"tool_search_output","execution":"client","call_id":"first","output":json!({"tools":[old]}).to_string()},
        {"type":"tool_search_output","execution":"client","call_id":"second","output":[new]}
    ]);
    let (body, _) = prepare(json!({"tools":[declaration()],"input":input})).unwrap();
    assert_eq!(body["tools"][1]["parameters"], new["parameters"]);
}

#[test]
fn discovered_namespaced_function_does_not_alias_an_unqualified_function() {
    let body = json!({"tools":[declaration(),{"type":"function","name":"tools.lookup","parameters":{"type":"object"}}],"input":[{"type":"tool_search_output","execution":"client","call_id":"search","tools":[{"type":"function","namespace":"tools","name":"lookup","parameters":{"type":"object"}}]}]});
    let (body, _) = prepare(body).unwrap();
    assert_eq!(body["tools"].as_array().unwrap().len(), 3);
    assert_eq!(body["tools"][1]["name"], "tools.lookup");
    assert_ne!(body["tools"][2]["name"], "tools.lookup");
}
