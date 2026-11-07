use super::*;

#[test]
fn grok_compact_lowers_discovered_history_before_removing_all_tool_declarations() {
    let request = json!({"model":"grok-4.7","stream":true,"tools":[{"type":"tool_search","execution":"client"}],"input":[
        {"type":"tool_search_call","call_id":"search","id":"ts_1","arguments":{"query":"find"}},
        {"type":"tool_search_output","call_id":"search","tools":[{"type":"function","name":"found","parameters":{"type":"object"}}]},
        {"type":"additional_tools","tools":[{"type":"function","name":"extra"}]},
        {"type":"compaction_trigger"}
    ]});
    let (wire, _) = prepare_compact(request).unwrap();
    for field in ["tools", "tool_choice", "parallel_tool_calls", "stream"] {
        assert!(wire.get(field).is_none(), "compact field {field}");
    }
    assert_eq!(wire["input"][0]["type"], "function_call");
    assert_eq!(wire["input"][1]["type"], "function_call_output");
    assert_eq!(wire["input"].as_array().unwrap().len(), 2);
}

fn prepared() -> (Value, Context) {
    prepare(json!({"model":"grok-build-latest","tools":[
    {"type":"namespace","name":"functions","vendor_ns":true,"tools":[
        {"type":"custom","name":"apply_patch","description":"Apply a patch","format":{"type":"grammar","definition":"patch"},"vendor":123},
        {"type":"function","name":"exec","parameters":{"type":"object"},"vendor":456}
    ]},{"type":"function","name":"native","parameters":{"type":"object"}},
    {"type":"x_search","allowed_x_handles":["xai"]}
],"tool_choice":{"type":"function","name":"apply_patch","namespace":"functions","vendor":"keep"},"input":[]})).unwrap()
}

#[test]
fn grok_wire_native_tools_and_reasoning_remain_exact() {
    let original = json!({"tools":[{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"one":{"type":"string","pattern":"keep"}}},"vendor":true},{"type":"web_search"}],"input":[{"type":"reasoning","id":"rs_1","encrypted_content":test_cipher(),"summary":[],"vendor":true},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}],"previous_response_id":"resp_1","include":["reasoning.encrypted_content"],"vendor_root":"keep"});
    let (body, mut context) = prepare(original.clone()).unwrap();
    assert_eq!(body, original);
    let event =
        json!({"type":"response.completed","response":{"output":original["input"],"vendor":true}});
    assert_eq!(
        restore_event(&mut context, event.clone()).unwrap(),
        vec![event]
    );
}
#[test]
fn grok_wire_namespaces_and_custom_declarations_are_reversible_and_collision_free() {
    let (body, mut context) = prepared();
    assert_eq!(body["tools"].as_array().unwrap().len(), 4);
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(
        body["tools"][0]["parameters"]["properties"]["input"]["type"],
        "string"
    );
    assert_eq!(body["tools"][0]["vendor"], 123);
    assert_eq!(body["tools"][1]["vendor"], 456);
    assert_eq!(body["tools"][2]["name"], "native");
    assert_eq!(body["tools"][3]["type"], "x_search");
    assert_eq!(body["tool_choice"]["name"], body["tools"][0]["name"]);
    assert_eq!(body["tool_choice"]["vendor"], "keep");
    let call = json!({"type":"response.completed","vendor":true,"response":{"id":"resp_1","output":[{"type":"function_call","id":"fc_1","call_id":"call_1","name":body["tools"][1]["name"],"arguments":"{}","vendor":true}],"tools":body["tools"]}});
    let out = restore_event(&mut context, call).unwrap();
    assert_eq!(out[0]["response"]["output"][0]["name"], "exec");
    assert_eq!(out[0]["response"]["output"][0]["namespace"], "functions");
    assert_eq!(out[0]["response"]["output"][0]["id"], "fc_1");
    assert_eq!(out[0]["response"]["output"][0]["call_id"], "call_1");
    assert_eq!(out[0]["response"]["output"][0]["vendor"], true);
    assert_eq!(out[0]["response"]["tools"][0]["type"], "namespace");
}
#[test]
fn grok_wire_custom_history_preserves_exact_patch_and_call_ids() {
    let patch =
        "*** Begin Patch ***\n*** Add File: quoted file.txt\n+\\\"text\\\"\n*** End Patch ***";
    let original = json!({"tools":[{"type":"custom","name":"apply_patch","format":{"type":"text"}}],"input":[{"type":"custom_tool_call","id":"ctc_old","call_id":"old","name":"apply_patch","input":patch,"vendor":true},{"type":"custom_tool_call_output","call_id":"old","output":[{"type":"input_text","text":"done"}],"vendor":true}]});
    let (body, mut context) = prepare(original).unwrap();
    assert_eq!(body["input"][0]["type"], "function_call");
    assert_eq!(body["input"][0]["id"], "ctc_old");
    assert_eq!(body["input"][0]["call_id"], "old");
    assert_eq!(body["input"][1]["type"], "function_call_output");
    assert_eq!(body["input"][1]["output"][0]["text"], "done");
    let out = restore_event(&mut context, json!({"output":[body["input"][0]]})).unwrap();
    assert_eq!(out[0]["output"][0]["input"], patch);
    assert_eq!(out[0]["output"][0]["type"], "custom_tool_call");
    assert_eq!(out[0]["output"][0]["vendor"], true);
}
#[test]
fn grok_wire_custom_stream_buffers_split_json_escapes_and_restores_delta_and_done() {
    let (body, mut context) = prepared();
    let wire = body["tools"][0]["name"].clone();
    let added=restore_event(&mut context,json!({"type":"response.output_item.added","output_index":2,"item":{"id":"fc_2","call_id":"call_2","type":"function_call","name":wire,"arguments":""}})).unwrap();
    assert_eq!(added[0]["item"]["type"], "custom_tool_call");
    assert_eq!(added[0]["item"]["namespace"], "functions");
    for delta in ["{\"in", "put\":\"line1\\n", "line2\\\"quoted\\\"\"}"] {
        assert!(restore_event(&mut context,json!({"type":"response.function_call_arguments.delta","item_id":"fc_2","output_index":2,"delta":delta})).unwrap().is_empty());
    }
    let out=restore_event(&mut context,json!({"type":"response.function_call_arguments.done","item_id":"fc_2","output_index":2,"sequence_number":9,"vendor":"keep"})).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0]["type"], "response.custom_tool_call_input.delta");
    assert_eq!(out[0]["delta"], "line1\nline2\"quoted\"");
    assert_eq!(out[1]["input"], out[0]["delta"]);
    assert_eq!(out[1]["sequence_number"], 9);
    assert_eq!(out[1]["item_id"], "fc_2");
    assert_eq!(out[1]["vendor"], "keep");
}
#[test]
fn grok_wire_unknown_hosted_calls_are_not_misidentified_as_client_tools() {
    let (_, mut context) = prepared();
    for event in [
        json!({"type":"response.output_item.done","item":{"type":"custom_tool_call","call_id":"xs_1","name":"x_user_search","input":"{}","vendor":true}}),
        json!({"type":"response.future_event","future":{"nested":true}}),
    ] {
        assert_eq!(
            restore_event(&mut context, event.clone()).unwrap(),
            vec![event]
        );
    }
}
#[test]
fn grok_wire_rejects_excess_active_tools_without_silently_truncating() {
    let tools:Vec<Value>=(0..200).map(|i|json!({"type":"function","name":format!("tool_{i}"),"parameters":{"oneOf":[{"type":"object","properties":{"x":{"type":"string"}}}]}})).collect();
    let original = json!({"tools":tools});
    assert_eq!(prepare(original.clone()).unwrap().0, original);
    let mut too_many = original;
    too_many["tools"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"function","name":"extra"}));
    assert_eq!(prepare(too_many.clone()).unwrap_err().status, 400);
    too_many["tool_choice"] =
        json!({"type":"allowed_tools","mode":"auto","tools":[{"type":"function","name":"tool_0"}]});
    assert_eq!(
        prepare(too_many).unwrap().0["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn grok_wire_invalid_custom_payload_does_not_invent_an_executable_patch() {
    let (body, mut context) = prepared();
    let error=restore_event(&mut context,json!({"output":[{"type":"function_call","name":body["tools"][0]["name"],"arguments":"{\"not_input\":\"value\"}"}]})).unwrap_err();
    assert_eq!(error.status, 502);
    assert!(restore_event(
        &mut context,
        json!({"output":[{"type":"function_call","name":body["tools"][0]["name"],"arguments":""}]})
    )
    .is_err());
}

#[test]
fn grok_wire_native_hosted_history_and_opaque_reasoning_are_not_replayed_or_projected() {
    let original = json!({"input":[
        {"type":"reasoning","encrypted_content":test_cipher(),"summary":[],"id":"rs_keep"},
        {"type":"custom_tool_call","name":"x_user_search","call_id":"xs_1","input":"{}","encrypted_content":"opaque-search"},
        {"type":"custom_tool_call_output","call_id":"xs_1","output":"keep","vendor":1},
        {"type":"custom_tool_call","name":"attachment_search","call_id":"as_1","input":"{}"}
    ],"previous_response_id":"resp_previous"});
    assert_eq!(prepare(original.clone()).unwrap().0, original);
}
#[test]
fn grok_wire_allowed_tools_preserves_policy_when_flattening_namespaces() {
    let (mut original, _) = prepared();
    original["tools"] = json!([
        {"type":"function","name":"native","parameters":{"type":"object"}},
        {"type":"namespace","name":"functions","tools":[{"type":"custom","name":"apply_patch"},{"type":"function","name":"exec","parameters":{"type":"object"}}]}
    ]);
    original["tool_choice"] = json!({"type":"allowed_tools","mode":"required","tools":[{"type":"custom","namespace":"functions","name":"apply_patch"}]});
    let (body, _) = prepare(original).unwrap();
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(body["tool_choice"], "required");
    assert_eq!(
        body["tools"][0]["parameters"]["properties"]["input"]["type"],
        "string"
    );
}
#[test]
fn grok_wire_stream_item_id_and_output_index_share_one_argument_buffer() {
    let (body, mut context) = prepared();
    restore_event(&mut context,json!({"type":"response.output_item.added","output_index":0,"item":{"id":"item-one","type":"function_call","call_id":"call-one","name":body["tools"][0]["name"],"arguments":""}})).unwrap();
    restore_event(&mut context,json!({"type":"response.function_call_arguments.delta","item_id":"item-one","delta":"{\"input\":\"safe\"}"})).unwrap();
    let out = restore_event(
        &mut context,
        json!({"type":"response.function_call_arguments.done","output_index":0}),
    )
    .unwrap();
    assert_eq!(out[0]["delta"], "safe");
}
#[test]
fn grok_wire_aliases_cannot_shadow_native_tool_names() {
    let (first, _) = prepare(json!({"tools":[{"type":"custom","name":"tool"}]})).unwrap();
    let alias = first["tools"][0]["name"].as_str().unwrap();
    let (second,mut context)=prepare(json!({"tools":[{"type":"custom","name":"tool"},{"type":"function","name":alias,"parameters":{"type":"object"}}]})).unwrap();
    assert_ne!(second["tools"][0]["name"], alias);
    let event = json!({"output":[{"type":"function_call","name":alias,"arguments":"{}","call_id":"native-id"}]});
    assert_eq!(
        restore_event(&mut context, event.clone()).unwrap(),
        vec![event]
    );
}
