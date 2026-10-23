use super::*;

fn request() -> Value {
    let mut tools:Vec<Value>=(0..198).map(|i|json!({"type":"function","name":format!("root_{i}"),"parameters":{"type":"object"}})).collect();
    tools.push(json!({"type":"namespace","name":"collaboration","tools":[
        {"type":"function","name":"spawn_agent","description":"Spawn a worker","parameters":{"type":"object","properties":{"message":{"type":"string"},"number":{"type":"integer"}},"required":["message"]}},
        {"type":"custom","name":"apply_patch","description":"Apply exact patch text","format":{"type":"grammar","definition":"*** Begin Patch"}},
        {"type":"function","name":"wait_agent","parameters":{"type":"object","properties":{"ids":{"type":"array","items":{"type":"string"}}}}}
    ]}));
    tools.push(json!({"type":"tool_search","execution":"client"}));
    json!({"tools":tools,"input":[]})
}
fn dispatcher(body: &Value) -> &Value {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| {
            tool["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("grok_namespace_"))
        })
        .unwrap()
}

#[test]
fn over_limit_folds_namespaces_without_discarding_tools_or_client_search() {
    let (body, context) = super::super::prepare(request()).unwrap();
    assert_eq!(body["tools"].as_array().unwrap().len(), 200);
    let tool = dispatcher(&body);
    assert_eq!(
        tool["parameters"]["properties"]["name"]["enum"],
        json!(["spawn_agent", "apply_patch", "wait_agent"])
    );
    let description = tool["description"].as_str().unwrap();
    assert!(description.contains("Spawn a worker"));
    assert!(description.contains("*** Begin Patch"));
    assert!(description.contains("integer"));
    assert!(description.contains("required"));
    assert!(context.by_wire.values().any(|binding| binding.key.search));
    for i in 0..198 {
        assert!(body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == format!("root_{i}")));
    }
    let mut within = request();
    within["tools"].as_array_mut().unwrap().remove(0);
    within["tools"].as_array_mut().unwrap().remove(0);
    let (within, _) = super::super::prepare(within).unwrap();
    assert_eq!(within["tools"].as_array().unwrap().len(), 200);
    assert!(!within["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("grok_namespace_"))));
}

#[test]
fn dispatcher_preserves_history_and_restores_original_custom_function_calls() {
    let mut original = request();
    let exact = "{\"number\":123456789012345678901234567890,\"message\":\"call\"}";
    original["input"] = json!([
        {"type":"reasoning","encrypted_content":test_cipher()},
        {"type":"function_call","name":"spawn_agent","namespace":"collaboration","call_id":"old-function","arguments":exact},
        {"type":"custom_tool_call","name":"apply_patch","namespace":"collaboration","call_id":"old-patch","input":"*** Begin Patch\nexact"},
        {"type":"custom_tool_call_output","call_id":"old-patch","output":"ok"}
    ]);
    let (body, mut context) = super::super::prepare(original).unwrap();
    let wire = dispatcher(&body)["name"].clone();
    assert_eq!(body["input"][0]["encrypted_content"], test_cipher());
    for index in [1, 2] {
        assert_eq!(body["input"][index]["name"], wire);
    }
    let envelope: Value =
        serde_json::from_str(body["input"][1]["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["name"], "spawn_agent");
    assert_eq!(envelope["arguments"], exact);
    let response = json!({"output":[
        {"type":"function_call","name":wire,"call_id":"new-function","arguments":format!("{{\"name\":\"spawn_agent\",\"arguments\":{exact}}}")},
        {"type":"function_call","name":wire,"call_id":"new-patch","arguments":json!({"name":"apply_patch","arguments":{"input":"*** Begin Patch\nexact"}}).to_string()}
    ]});
    let restored = super::super::restore_event(&mut context, response).unwrap();
    assert_eq!(restored[0]["output"][0]["name"], "spawn_agent");
    assert_eq!(restored[0]["output"][0]["namespace"], "collaboration");
    assert_eq!(restored[0]["output"][0]["arguments"], exact);
    assert_eq!(restored[0]["output"][1]["type"], "custom_tool_call");
    assert_eq!(restored[0]["output"][1]["input"], "*** Begin Patch\nexact");
}

#[test]
fn dispatcher_stream_waits_for_child_then_restores_tool_sequence_once() {
    for custom in [false, true] {
        let (body, mut context) = super::super::prepare(request()).unwrap();
        let wire = dispatcher(&body)["name"].clone();
        assert!(super::super::restore_event(&mut context,json!({"type":"response.output_item.added","output_index":4,"item":{"type":"function_call","id":"fc_dispatch","call_id":"call_dispatch","name":wire,"arguments":"","vendor":true}})).unwrap().is_empty());
        let (name, args) = if custom {
            ("apply_patch", json!({"input":"line1\nline2"}))
        } else {
            ("spawn_agent", json!({"message":"go"}))
        };
        let wrapped = json!({"name":name,"arguments":args}).to_string();
        for chunk in wrapped.as_bytes().chunks(5) {
            assert!(super::super::restore_event(&mut context,json!({"type":"response.function_call_arguments.delta","item_id":"fc_dispatch","delta":std::str::from_utf8(chunk).unwrap()})).unwrap().is_empty());
        }
        let events = super::super::restore_event(
            &mut context,
            json!({"type":"response.function_call_arguments.done","output_index":4}),
        )
        .unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["item"]["name"], name);
        assert_eq!(events[0]["item"]["namespace"], "collaboration");
        assert_eq!(events[0]["item"]["vendor"], true);
        assert_eq!(
            events[1]["delta"],
            if custom {
                "line1\nline2".to_owned()
            } else {
                args.to_string()
            }
        );
        assert_eq!(
            events[2]["type"],
            if custom {
                "response.custom_tool_call_input.done"
            } else {
                "response.function_call_arguments.done"
            }
        );
        let done=super::super::restore_event(&mut context,json!({"type":"response.output_item.done","output_index":4,"item":{"type":"function_call","id":"fc_dispatch","call_id":"call_dispatch","name":wire,"arguments":"","status":"completed"}})).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0]["item"]["name"], name);
        assert_eq!(done[0]["item"]["call_id"], "call_dispatch");
    }
}

#[test]
fn forced_choice_cannot_dispatch_a_different_child_and_allowed_choice_stays_narrow() {
    let mut original = request();
    original["tool_choice"] =
        json!({"type":"function","name":"spawn_agent","namespace":"collaboration"});
    let (body, mut context) = super::super::prepare(original).unwrap();
    assert_eq!(body["tool_choice"]["name"], dispatcher(&body)["name"]);
    assert_eq!(
        dispatcher(&body)["parameters"]["properties"]["name"]["enum"],
        json!(["spawn_agent"])
    );
    assert!(super::super::restore_event(&mut context,json!({"output":[{"type":"function_call","name":dispatcher(&body)["name"],"arguments":"{\"name\":\"apply_patch\",\"arguments\":{\"input\":\"bad\"}}"}]})).is_err());
    let mut narrow = request();
    narrow["tool_choice"] = json!({"type":"allowed_tools","mode":"required","tools":[{"type":"function","name":"spawn_agent","namespace":"collaboration"}]});
    let (narrow, _) = super::super::prepare(narrow).unwrap();
    assert_eq!(narrow["tools"].as_array().unwrap().len(), 1);
    assert_eq!(narrow["tool_choice"], "required");
}

#[test]
fn dispatcher_rejects_unknown_children_and_raw_parser_keeps_escaped_json() {
    let group = Group {
        children: BTreeMap::from([("lookup".into(), "wire".into())]),
    };
    let raw = r#"{"name":"lookup","arguments":{"large":123456789012345678901234567890,"text":"quoted \" , } \\"}}"#;
    let (_, args) = unwrap(&group, raw).unwrap();
    assert_eq!(
        args,
        r#"{"large":123456789012345678901234567890,"text":"quoted \" , } \\"}"#
    );
    assert_eq!(
        unwrap(
            &group,
            r#"{"name":"lookup","count":123456789012345678901234567890}"#
        )
        .unwrap()
        .1,
        r#"{"count":123456789012345678901234567890}"#
    );
    for raw in [
        r#"{"name":"unknown","arguments":{}}"#,
        r#"{"name":"lookup","arguments":[]}"#,
        "{",
    ] {
        assert!(unwrap(&group, raw).is_err());
    }
}

#[test]
fn sparse_or_repeated_dispatcher_outputs_require_one_consistent_completed_call() {
    let (body, mut context) = super::super::prepare(request()).unwrap();
    let wire = dispatcher(&body)["name"].clone();
    let added = json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc_one","call_id":"one","name":wire,"arguments":""}});
    assert!(super::super::restore_event(&mut context, added.clone())
        .unwrap()
        .is_empty());
    let sparse = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc_one","call_id":"one","name":wire}});
    assert!(super::super::restore_event(&mut context, sparse.clone()).is_err());
    assert!(super::super::restore_event(&mut context, json!({"output":[sparse["item"]]})).is_err());
    let wrapped = json!({"name":"spawn_agent","arguments":{"message":"once"}}).to_string();
    let arguments_done = json!({"type":"response.function_call_arguments.done","item_id":"fc_one","arguments":wrapped});
    assert_eq!(
        super::super::restore_event(&mut context, arguments_done.clone())
            .unwrap()
            .len(),
        3
    );
    assert!(super::super::restore_event(&mut context, arguments_done)
        .unwrap()
        .is_empty());
    let restored = super::super::restore_event(&mut context, sparse.clone()).unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0]["item"]["name"], "spawn_agent");
    assert!(super::super::restore_event(&mut context, sparse.clone())
        .unwrap()
        .is_empty());
    let terminal =
        super::super::restore_event(&mut context, json!({"output":[sparse["item"]]})).unwrap();
    assert_eq!(
        terminal[0]["output"][0]["arguments"],
        json!({"message":"once"}).to_string()
    );
    for bad in [
        json!({"name":"spawn_agent","arguments":{"message":"changed"}}),
        json!({"name":"wait_agent","arguments":{}}),
    ] {
        assert!(super::super::restore_event(&mut context,json!({"type":"response.function_call_arguments.done","item_id":"fc_one","arguments":bad.to_string()})).is_err());
        let mut wrong = sparse["item"].clone();
        wrong["arguments"] = json!(bad.to_string());
        assert!(super::super::restore_event(&mut context, json!({"output":[wrong]})).is_err());
    }
    let mut wrong = sparse["item"].clone();
    wrong["name"] = json!("native_0");
    assert!(super::super::restore_event(&mut context, json!({"output":[wrong]})).is_err());
}

#[test]
fn dispatcher_distinguishes_function_and_custom_tools_with_the_same_short_name() {
    let mut original = request();
    original["tools"][198]["tools"] = json!([
        {"type":"function","name":"same","parameters":{"type":"object"}},
        {"type":"custom","name":"same","format":{"type":"text"}},
        {"type":"function","name":"other","parameters":{"type":"object"}}
    ]);
    let (body, mut context) = super::super::prepare(original).unwrap();
    let wire = dispatcher(&body)["name"].clone();
    let group = &context.dispatchers.groups[wire.as_str().unwrap()];
    let function = group
        .children
        .iter()
        .find(|(_, wire)| {
            let binding = &context.by_wire[*wire];
            binding.key.name == "same" && !binding.key.custom
        })
        .unwrap()
        .0
        .clone();
    let custom = group
        .children
        .iter()
        .find(|(_, wire)| context.by_wire[*wire].key.custom)
        .unwrap()
        .0
        .clone();
    assert_ne!(function, custom);
    for (selector, args, expected) in [
        (function, json!({"value":1}), "function_call"),
        (custom, json!({"input":"exact"}), "custom_tool_call"),
    ] {
        let result=super::super::restore_event(&mut context,json!({"output":[{"type":"function_call","name":wire,"arguments":json!({"name":selector,"arguments":args}).to_string()}]})).unwrap();
        assert_eq!(result[0]["output"][0]["name"], "same");
        assert_eq!(result[0]["output"][0]["namespace"], "collaboration");
        assert_eq!(result[0]["output"][0]["type"], expected);
    }
}

#[test]
fn dispatcher_catalog_obeys_the_request_local_resource_limit() {
    let mut original = request();
    original["tools"][198]["tools"][0]["description"] = json!("x".repeat(MAX_STATE_BYTES));
    let error = super::super::prepare(original).unwrap_err();
    assert_eq!(error.status, 400);
    assert!(error.message.contains("catalog"));
}
