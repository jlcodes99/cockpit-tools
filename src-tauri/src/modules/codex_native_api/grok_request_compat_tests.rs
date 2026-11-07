use super::super::{prepare, restore_event};
use super::*;

#[test]
fn additional_tools_promote_without_client_search_and_respect_disabled_tools() {
    let additional = json!({"type":"additional_tools","tools":[{"type":"namespace","name":"research","tools":[{"type":"function","name":"lookup"}]}]});
    let (body, _) = prepare(json!({"input":[additional,{"role":"user","content":"go"}]})).unwrap();
    assert_eq!(body["input"], json!([{ "role":"user", "content":"go" }]));
    assert_eq!(
        body["tools"][0]["parameters"],
        json!({"type":"object","properties":{}})
    );
    assert!(body["tools"][0]["name"]
        .as_str()
        .unwrap()
        .starts_with("grok_tool_"));
    let (disabled, _) = prepare(json!({"tools":[],"tool_choice":"required","parallel_tool_calls":true,"input":[additional]})).unwrap();
    for field in ["tools", "tool_choice", "parallel_tool_calls"] {
        assert!(disabled.get(field).is_none());
    }
    assert_eq!(disabled["input"], json!([]));
}

#[test]
fn search_options_and_aliases_are_normalized_only_in_tool_declarations() {
    let (body, _) = prepare(json!({"tools":[
        {"type":"web_search_preview","external_web_access":true,"search_context_size":"high","filters":{"allowed_domains":["example.test"]},"user_location":{"country":"NL"},"search_content_types":["text","image"],"vendor":{"external_web_access":true}},
        {"type":"namespace","name":"research","tools":[{"type":"web_search_preview_2025_03_11","search_context_size":"low"}]}
    ],"input":[{"type":"reasoning","encrypted_content":super::super::test_cipher(),"summary":[]}],"vendor":{"tools":[{"type":"web_search_preview"}],"stop":"keep"},"stop":["stop"],"stream_options":{"include_usage":true},"safety_identifier":"x","prompt_cache_retention":"24h","temperature":0.5,"max_output_tokens":100})).unwrap();
    for tool in body["tools"].as_array().unwrap() {
        assert_eq!(tool["type"], "web_search");
        assert!(tool.get("external_web_access").is_none());
        assert!(tool.get("search_context_size").is_none());
    }
    assert_eq!(
        body["tools"][0]["filters"]["allowed_domains"],
        json!(["example.test"])
    );
    assert_eq!(body["tools"][0]["user_location"]["country"], "NL");
    assert_eq!(
        body["tools"][0]["search_content_types"],
        json!(["text", "image"])
    );
    assert_eq!(body["tools"][0]["vendor"]["external_web_access"], true);
    assert_eq!(body["vendor"]["tools"][0]["type"], "web_search_preview");
    assert_eq!(body["vendor"]["stop"], "keep");
    assert_eq!(
        body["input"][0]["encrypted_content"],
        super::super::test_cipher()
    );
    assert_eq!(body["temperature"], 0.5);
    assert_eq!(body["max_output_tokens"], 100);
    for field in [
        "stop",
        "stream_options",
        "safety_identifier",
        "prompt_cache_retention",
    ] {
        assert!(body.get(field).is_none());
    }
}

#[test]
fn forced_hosted_and_allowed_tools_choices_preserve_restrictions() {
    for kind in ["web_search", "image_generation"] {
        let tools = json!([{"type":kind},{"type":"function","name":"other","parameters":{"type":"object"}}]);
        let (body, _) = prepare(json!({"tools":tools,"tool_choice":{"type":kind}})).unwrap();
        assert_eq!(body["tool_choice"], "required");
        assert_eq!(body["tools"], json!([{ "type":kind }]));
        let (body, _) = prepare(json!({"tools":tools,"tool_choice":{"type":"allowed_tools","mode":"auto","tools":[{"type":kind}]}})).unwrap();
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["tools"], json!([{ "type":kind }]));
    }
    let (body, _) = prepare(json!({"tools":[{"type":"web_search_preview"}],"tool_choice":{"type":"web_search_preview_2025_03_11"}})).unwrap();
    assert_eq!(body["tools"], json!([{"type":"web_search"}]));
    assert_eq!(body["tool_choice"], "required");
}

#[test]
fn client_web_search_aliases_declarations_choices_history_and_responses() {
    let (initial, _) = prepare(json!({"tools":[{"type":"function","name":"web_search"}]})).unwrap();
    let collision = initial["tools"][0]["name"].as_str().unwrap();
    let request = json!({"tools":[{"type":"function","name":"web_search"},{"type":"function","name":collision},{"type":"web_search"}],"tool_choice":{"type":"function","name":"web_search"},"input":[{"type":"function_call","name":"web_search","call_id":"prior","arguments":"{}"},{"type":"function_call_output","call_id":"prior","output":"ok"}]});
    let (body, mut context) = prepare(request).unwrap();
    let alias = &body["tools"][0]["name"];
    assert_ne!(alias, "web_search");
    assert_ne!(alias, collision);
    assert_eq!(&body["tool_choice"]["name"], alias);
    assert_eq!(&body["input"][0]["name"], alias);
    assert_eq!(body["tools"][2]["type"], "web_search");
    for event in [
        json!({"type":"response.output_item.done","item":{"type":"function_call","name":alias,"call_id":"next","arguments":"{}"}}),
        json!({"output":[{"type":"function_call","name":alias,"call_id":"next","arguments":"{}"}]}),
    ] {
        let restored = restore_event(&mut context, event).unwrap().remove(0);
        let item = restored.get("item").unwrap_or(&restored["output"][0]);
        assert_eq!(item["name"], "web_search");
        assert!(item.get("namespace").is_none());
    }
    let event = restore_event(
        &mut context,
        json!({"type":"response.function_call_arguments.done","name":alias,"arguments":"{}"}),
    )
    .unwrap();
    assert_eq!(event[0]["name"], "web_search");
}

#[test]
fn schema_inlining_keeps_constraints_literal_data_and_numbers() {
    let schema = json!({"type":"object","properties":{"value":{"$ref":"#/$defs/Thing","description":"override"},"example":{"type":"object","default":{"$ref":"#/not-a-schema"}},"count":{"type":"number","minimum":2}},"$defs":{"Thing":{"type":"string","description":"original","enum":["a","b"]}},"vendor":{"$ref":"#/$defs/Thing"}});
    let mut tool = json!({"parameters":schema});
    normalize_function(&mut tool, "lookup", None).unwrap();
    assert_eq!(
        tool["parameters"]["properties"]["value"],
        json!({"type":"string","description":"override","enum":["a","b"]})
    );
    assert_eq!(
        tool["parameters"]["properties"]["example"]["default"],
        schema["properties"]["example"]["default"]
    );
    assert_eq!(
        tool["parameters"]["properties"]["count"],
        schema["properties"]["count"]
    );
    assert_eq!(tool["parameters"]["vendor"], schema["vendor"]);
    assert!(tool["parameters"].get("$defs").is_none());
}

#[test]
fn root_union_types_are_repaired_or_simplified_and_automation_does_not_hang() {
    let mut object_union = json!({"parameters":{"type":"object","oneOf":[{"required":["a"],"properties":{"a":{"type":"number"}}},{"type":"object","required":["b"]}]},"strict":true});
    normalize_function(&mut object_union, "lookup", None).unwrap();
    assert_eq!(object_union["parameters"]["oneOf"][0]["type"], "object");
    assert_eq!(
        object_union["parameters"]["oneOf"][0]["properties"]["a"]["type"],
        "number"
    );
    assert_eq!(object_union["strict"], true);
    for (name, namespace, schema) in [
        (
            "lookup",
            None,
            json!({"oneOf":[{"type":"object"},{"type":"string"}]}),
        ),
        (
            "automation_update",
            Some("codex_app"),
            json!({"type":"object","properties":{"unchanged":{"type":"string"}}}),
        ),
        (
            "mcp__codex_apps__automation_update",
            None,
            json!({"type":"object"}),
        ),
    ] {
        let mut tool = json!({"parameters":schema,"strict":true});
        normalize_function(&mut tool, name, namespace).unwrap();
        assert_eq!(
            tool["parameters"],
            json!({"type":"object","properties":{},"additionalProperties":true})
        );
        assert_eq!(tool["strict"], false);
    }
}

#[test]
fn recursive_references_remain_bounded_and_keep_definitions() {
    let schema = json!({"type":"object","properties":{"tree":{"$ref":"#/$defs/Tree"}},"$defs":{"Tree":{"type":"object","properties":{"next":{"$ref":"#/$defs/Tree"}}}}});
    let mut tool = json!({"parameters":schema});
    normalize_function(&mut tool, "lookup", None).unwrap();
    assert!(tool["parameters"]["$defs"].is_object());
    assert_eq!(
        tool["parameters"]["properties"]["tree"]["properties"]["next"]["$ref"],
        "#/$defs/Tree"
    );
}
