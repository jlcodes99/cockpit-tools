use super::*;

fn function(schema: Value) -> Value {
    json!({"type":"function","name":"exec_command","description":"Run a client-owned command","parameters":schema,"strict":true})
}
fn catalog(schema: Value) -> ToolCatalog {
    ToolCatalog::from_tools(Some(&json!([function(schema)]))).unwrap()
}
fn alias(catalog: &ToolCatalog) -> String {
    catalog.delegate_definitions()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
fn call(catalog: &ToolCatalog, input: Value) -> Result<Value, ServiceError> {
    catalog.call_item(
        &json!({"toolId":alias(catalog),"input":input}),
        "call_public_fixture",
    )
}
fn events(document: &Value) -> Vec<Value> {
    response_events(document)
        .unwrap()
        .lines()
        .filter_map(|s| s.strip_prefix("data: "))
        .map(|s| serde_json::from_str(s).unwrap())
        .collect()
}

// Provenance: tools-only extraction from the synthetic Codex CLI 0.172
// captured-request-1.json compatibility probe. Original request messages,
// developer instructions, client metadata and host paths are deliberately not
// included. Eight top-level declarations expand to twelve callable functions;
// this capture contains no custom tool and is not end-to-end agent evidence.
fn captured_codex_0172_tools() -> Value {
    serde_json::from_str(include_str!(
        "workbuddy_delegate_codex_0172_tools.fixture.json"
    ))
    .unwrap()
}

#[test]
fn captured_codex_0172_public_tools_pass_catalog_and_restore_all_identities() {
    let tools = captured_codex_0172_tools();
    assert_eq!(tools.as_array().unwrap().len(), 8);
    let catalog = ToolCatalog::from_tools(Some(&tools)).unwrap();
    let definitions = catalog.delegate_definitions();
    assert_eq!(definitions.len(), 12);
    let cases = [
        (
            "exec_command",
            json!({"cmd":"printf synthetic","workdir":"synthetic-project","yield_time_ms":1000,"max_output_tokens":128,"sandbox_permissions":"use_default"}),
        ),
        (
            "write_stdin",
            json!({"session_id":1,"chars":"synthetic\\n","yield_time_ms":1000}),
        ),
        (
            "request_user_input",
            json!({"questions":[{"id":"choice","header":"Choose","question":"Which synthetic option?","options":[{"label":"One","description":"First fixture option"},{"label":"Two","description":"Second fixture option"}]}]}),
        ),
        ("view_image", json!({"path":"synthetic-image.png"})),
        (
            "multi_agent_v1.close_agent",
            json!({"target":"synthetic-agent"}),
        ),
        (
            "multi_agent_v1.resume_agent",
            json!({"id":"synthetic-agent"}),
        ),
        (
            "multi_agent_v1.send_input",
            json!({"target":"synthetic-agent","items":[{"type":"text","text":"Synthetic message"}],"interrupt":false}),
        ),
        (
            "multi_agent_v1.spawn_agent",
            json!({"message":"Synthetic scoped task","fork_context":false}),
        ),
        (
            "multi_agent_v1.wait_agent",
            json!({"targets":["synthetic-agent"],"timeout_ms":10000}),
        ),
        ("get_goal", json!({})),
        (
            "create_goal",
            json!({"objective":"Synthetic explicit goal","token_budget":1000}),
        ),
        ("update_goal", json!({"status":"complete"})),
    ];
    for (qualified_name, input) in cases {
        let definition = definitions
            .iter()
            .find(|definition| definition["name"] == qualified_name)
            .unwrap();
        assert_eq!(definition["requiresApproval"], true);
        let item = catalog
            .call_item(
                &json!({"sessionId":"","toolId":definition["id"],"input":input}),
                "call_captured_fixture",
            )
            .unwrap();
        assert_eq!(item["type"], "function_call");
        assert_eq!(
            serde_json::from_str::<Value>(item["arguments"].as_str().unwrap()).unwrap(),
            input
        );
        if let Some((namespace, name)) = qualified_name.split_once('.') {
            assert_eq!(item["namespace"], namespace);
            assert_eq!(item["name"], name);
        } else {
            assert_eq!(item["name"], qualified_name);
            assert!(item.get("namespace").is_none());
        }
    }
}

#[test]
fn captured_codex_0172_minimal_request_shape_keeps_role_text_and_auto_parallel_policy() {
    // Preserve the captured message/part structure, not its original contents or
    // identifiers. auto + parallel=true are the policy fields observed in that
    // capture. Request-boundary acceptance is tested by the glue module.
    let developer_first = "Synthetic sandbox rule: client executes tools.\r\n  ";
    let developer_second = "Synthetic AGENTS rule: preserve bytes.\t你好";
    let user = "/synthetic literal request\n";
    let body = json!({
        "model":"synthetic-workbuddy-profile","stream":true,
        "tools":captured_codex_0172_tools(),"tool_choice":"auto","parallel_tool_calls":true,"store":false,
        "input":[
            {"type":"message","id":"msg_synthetic_developer","role":"developer","content":[{"type":"input_text","text":developer_first},{"type":"input_text","text":developer_second}]},
            {"type":"message","id":"msg_synthetic_user","role":"user","content":[{"type":"input_text","text":user}]}
        ]
    });
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(
        ToolCatalog::from_tools(body.get("tools"))
            .unwrap()
            .delegate_definitions()
            .len(),
        12
    );
    let items = input_items(&body).unwrap();
    assert_eq!(Value::Array(items.clone()), body["input"]);
    let expected = format!(
        "Message begin role developer message 0 part 0 UTF8 bytes {}\n{}\nMessage end\nMessage begin role developer message 0 part 1 UTF8 bytes {}\n{}\nMessage end\nMessage begin role user message 1 part 0 UTF8 bytes {}\n{}\nMessage end\n",
        developer_first.len(),developer_first,developer_second.len(),developer_second,user.len(),user
    );
    assert_eq!(turn_prompt(&items).unwrap(), expected);
    assert_eq!(user_prompt(&items).unwrap(), user);
    assert!(tool_results(&items).unwrap().is_empty());
}

#[test]
fn captured_codex_0172_fixture_contains_only_public_declarations_without_secrets_or_host_paths() {
    let tools = captured_codex_0172_tools();
    let forbidden = [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "access_token",
        "refresh_token",
        "id_token",
        "api_key",
        "apikey",
        "credential",
        "credentials",
        "private_key",
        "token",
    ];
    fn check(value: &Value, forbidden: &[&str]) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !forbidden.contains(&key.to_ascii_lowercase().as_str()),
                        "credential field excluded from fixture: {key}"
                    );
                    check(value, forbidden);
                }
            }
            Value::Array(values) => {
                for value in values {
                    check(value, forbidden);
                }
            }
            Value::String(value) => {
                for excluded in [
                    "/Users/",
                    "/home/",
                    "/private/",
                    "/tmp/",
                    "Bearer ",
                    "-----BEGIN ",
                ] {
                    assert!(
                        !value.contains(excluded),
                        "host path/credential value excluded from fixture"
                    );
                }
            }
            _ => {}
        }
    }
    for declaration in tools.as_array().unwrap() {
        let object = declaration.as_object().unwrap();
        assert!(object.keys().all(|key| matches!(
            key.as_str(),
            "type" | "name" | "description" | "strict" | "parameters" | "tools"
        )));
        assert!(matches!(
            declaration["type"].as_str(),
            Some("function" | "namespace")
        ));
    }
    check(&tools, &forbidden);
}

#[test]
fn absent_and_empty_catalogs_are_empty_but_malformed_tools_fail_closed() {
    assert!(ToolCatalog::from_tools(None).unwrap().is_empty());
    assert!(ToolCatalog::from_tools(Some(&json!([])))
        .unwrap()
        .is_empty());
    for value in [Value::Null, json!({}), json!("tools"), json!([null])] {
        assert!(ToolCatalog::from_tools(Some(&value)).is_err());
    }
}

#[test]
fn function_call_is_a_complete_responses_item_and_arguments_are_not_executed() {
    let c = catalog(
        json!({"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"],"additionalProperties":false}),
    );
    let input = json!({"cmd":"printf 'synthetic only'\n# preserve whitespace  "});
    let item = call(&c, input.clone()).unwrap();
    assert_eq!(item["type"], "function_call");
    assert_eq!(item["name"], "exec_command");
    assert_eq!(item["call_id"], "call_public_fixture");
    assert_eq!(item["status"], "completed");
    assert!(item["id"].as_str().unwrap().starts_with("fc_"));
    assert_eq!(
        serde_json::from_str::<Value>(item["arguments"].as_str().unwrap()).unwrap(),
        input
    );
    assert_eq!(
        item,
        call(&c, input).unwrap(),
        "public IDs must replay deterministically"
    );
}

#[test]
fn custom_freeform_patch_is_wrapped_and_restored_without_json_guessing() {
    let c = ToolCatalog::from_tools(Some(
        &json!([{"type":"custom","name":"apply_patch","format":{"type":"text"}}]),
    ))
    .unwrap();
    let patch = "*** Begin Patch\n*** Add File: synthetic.txt\n+{not JSON}\n*** End Patch\n  ";
    let item = call(&c, json!({"input":patch})).unwrap();
    assert_eq!(item["type"], "custom_tool_call");
    assert_eq!(item["input"], patch);
    assert!(item["id"].as_str().unwrap().starts_with("ctc_"));
    assert!(item.get("arguments").is_none());
    assert!(call(&c, json!(patch)).is_err());
    assert!(call(&c, json!({"input":patch,"cmd":"forbidden"})).is_err());
    assert!(call(&c, json!({"input":{"patch":patch}})).is_err());
    assert!(ToolCatalog::from_tools(Some(
        &json!([{"type":"custom","name":"patch","format":{"type":"grammar","definition":"x"}}])
    ))
    .is_err());
}

#[test]
fn namespace_identity_is_restored_and_does_not_alias_same_named_tools() {
    let c = ToolCatalog::from_tools(Some(&json!([
        {"type":"namespace","name":"functions","tools":[function(json!({"type":"object"}))]},
        {"type":"namespace","name":"other","tools":[function(json!({"type":"object"}))]},
        {"type":"custom","name":"exec_command"}
    ])))
    .unwrap();
    let defs = c.delegate_definitions();
    assert_eq!(defs.len(), 3);
    let mut ids = BTreeSet::new();
    for definition in defs {
        assert!(ids.insert(definition["id"].as_str().unwrap().to_owned()));
        let input = if definition["name"] == "exec_command" {
            json!({"input":"raw"})
        } else {
            json!({})
        };
        let item = c
            .call_item(
                &json!({"toolId":definition["id"],"input":input}),
                "public_fixture",
            )
            .unwrap();
        assert_eq!(item["name"], "exec_command");
        if definition["name"] == "functions.exec_command" {
            assert_eq!(item["namespace"], "functions");
        }
        if definition["name"] == "other.exec_command" {
            assert_eq!(item["namespace"], "other");
        }
    }
}

#[test]
fn aliases_and_fingerprints_are_order_independent_but_meaning_sensitive() {
    let a = function(
        json!({"type":"object","properties":{"a":{"type":"string"},"b":{"type":"integer"}}}),
    );
    let b = json!({"type":"custom","name":"patch"});
    let c1 = ToolCatalog::from_tools(Some(&json!([a.clone(), b.clone()]))).unwrap();
    let c2 = ToolCatalog::from_tools(Some(&json!([b, a.clone()]))).unwrap();
    assert_eq!(c1.delegate_definitions(), c2.delegate_definitions());
    assert_eq!(c1.fingerprint(), c2.fingerprint());
    let mut changed = a;
    changed["description"] = json!("Changed tool meaning");
    let c3 =
        ToolCatalog::from_tools(Some(&json!([changed,{"type":"custom","name":"patch"}]))).unwrap();
    assert_ne!(c1.fingerprint(), c3.fingerprint());
    assert_eq!(c1.fingerprint().len(), 64);
}

#[test]
fn unknown_hosted_nested_namespace_and_duplicate_identity_are_rejected() {
    for tool in [
        json!({"type":"web_search"}),
        json!({"type":"image_generation"}),
        json!({"type":"computer_use_preview"}),
        json!({"type":"function","name":"bad name"}),
        json!({"type":"namespace","name":"outer","tools":[{"type":"namespace","name":"inner","tools":[]}]}),
    ] {
        assert!(ToolCatalog::from_tools(Some(&json!([tool]))).is_err());
    }
    let tool = function(json!({}));
    assert!(ToolCatalog::from_tools(Some(&json!([tool.clone(), tool]))).is_err());
    assert!(ToolCatalog::from_tools(Some(&json!([{"type":"namespace","name":"a","tools":[{"type":"function","namespace":"b","name":"x"}]}]))).is_err());
}

#[test]
fn delegate_definitions_are_schema_bound_and_require_native_permission() {
    let c = catalog(json!({"type":"object"}));
    let d = &c.delegate_definitions()[0];
    for field in [
        "id",
        "name",
        "description",
        "inputSchema",
        "provider",
        "category",
        "requiresApproval",
    ] {
        assert!(d.get(field).is_some());
    }
    assert_eq!(d["requiresApproval"], true);
    assert_eq!(d["inputSchema"], json!({"type":"object"}));
}

#[test]
fn delegate_call_rejects_undeclared_alias_extra_fields_and_bad_public_ids() {
    let c = catalog(json!({"type":"object"}));
    for p in [
        json!({"toolId":"exec_command","input":{}}),
        json!({"toolId":alias(&c),"input":{},"cmd":"bypass"}),
        json!({"toolId":alias(&c),"input":[]}),
        json!({"toolId":alias(&c)}),
        json!({"toolId":alias(&c),"input":{},"timeout":0}),
        json!({"toolId":alias(&c),"input":{},"sessionId":42}),
    ] {
        assert!(c.call_item(&p, "call_safe").is_err());
    }
    assert!(c
        .call_item(
            &json!({"sessionId":"","toolId":alias(&c),"input":{},"timeout":1000}),
            "call_safe"
        )
        .is_ok());
    assert!(c
        .call_item(&json!({"toolId":alias(&c),"input":{}}), "../internal-rpc")
        .is_err());
}

#[test]
fn actual_acp_delegate_callback_accepts_bounded_tool_call_id_without_using_its_identity() {
    let c = catalog(
        json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
    );
    // Observed live ACP shape: empty sessionId, presentation toolCallId and
    // timeout all coexist. Only toolId is rebound to this catalog's registered
    // alias, as production delegate_definitions advertises bridge aliases.
    // No observed callback field is deleted to make this fixture pass.
    let mut actual = json!({
        "sessionId":"",
        "toolCallId":"delegate-probe_echo-1795664485847",
        "toolId":alias(&c),
        "input":{"value":"ping"},
        "timeout":30000
    });
    let item = c.call_item(&actual, "call_public_fixture").unwrap();
    assert_eq!(item["call_id"], "call_public_fixture");
    assert_eq!(item["arguments"], "{\"value\":\"ping\"}");
    assert!(item.get("toolCallId").is_none());
    actual["toolCallId"] = json!("another-independent-presentation-id");
    assert_eq!(
        c.call_item(&actual, "call_public_fixture").unwrap(),
        item,
        "presentation IDs must not affect public identity or arguments"
    );
    actual["toolCallId"] = json!("x".repeat(256));
    assert!(c.call_item(&actual, "call_public_fixture").is_ok());
    actual["toolCallId"] = json!("界".repeat(85));
    assert!(
        c.call_item(&actual, "call_public_fixture").is_ok(),
        "bound is UTF-8 bytes, 255 here"
    );
}

#[test]
fn delegate_tool_call_id_rejects_malformed_oversized_and_unknown_callback_fields() {
    let c = catalog(json!({"type":"object"}));
    let original = json!({"sessionId":"","toolCallId":"delegate-synthetic-1","toolId":alias(&c),"input":{},"timeout":30000});
    for value in [
        Value::Null,
        json!(false),
        json!(1),
        json!({}),
        json!([]),
        json!(""),
        json!("x".repeat(257)),
        json!("界".repeat(86)),
    ] {
        let mut malformed = original.clone();
        malformed["toolCallId"] = value;
        assert!(c.call_item(&malformed, "call_public_fixture").is_err());
    }
    let mut unknown = original;
    unknown["unrecognizedProtocolField"] = json!("must not silently ignore");
    assert!(c.call_item(&unknown, "call_public_fixture").is_err());
}

#[test]
fn permission_raw_input_still_rejects_delegate_callback_tool_call_id() {
    let c = catalog(
        json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"],"additionalProperties":false}),
    );
    // ACP permission carries its presentation ID on toolCall, not rawInput.
    let mut permission = json!({
        "sessionId":"synthetic-permission-session",
        "toolCall":{
            "toolCallId":"call_synthetic_permission",
            "rawInput":{"toolId":alias(&c),"input":{"value":"ping"}},
            "_meta":{"codebuddy.ai/toolName":"DelegateTool"}
        }
    });
    assert!(c.validate_permission(&permission));
    permission["toolCall"]["rawInput"]["toolCallId"] = json!("delegate-probe_echo-1795664485847");
    assert!(
        !c.validate_permission(&permission),
        "permission rawInput whitelist remains distinct from delegate callback shape"
    );
}

#[test]
fn permissions_require_exact_native_metadata_and_whitelisted_valid_input() {
    let c = catalog(
        json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}),
    );
    let good = json!({"sessionId":"","toolCall":{"title":"Not trusted for approval","_meta":{"codebuddy.ai/toolName":"DelegateTool"},"rawInput":{"toolId":alias(&c),"input":{"path":"fixture"}}}});
    assert!(c.validate_permission(&good));
    let mut bad = good.clone();
    bad["toolCall"]["_meta"] = json!({});
    bad["toolCall"]["title"] = json!("DelegateTool");
    assert!(!c.validate_permission(&bad));
    let mut bad = good.clone();
    bad["toolCall"]["_meta"]["codebuddy.ai/toolName"] = json!("delegateTool");
    assert!(!c.validate_permission(&bad));
    let mut bad = good.clone();
    bad["toolCall"]["rawInput"]["toolId"] = json!("undeclared");
    assert!(!c.validate_permission(&bad));
    let mut bad = good.clone();
    bad["toolCall"]["rawInput"]["input"] = json!({"path":false});
    assert!(!c.validate_permission(&bad));
    let mut bad = good.clone();
    bad["toolCall"]["rawInput"]["input"]["shell"] = json!("unexpected");
    assert!(!c.validate_permission(&bad));
    let mut bad = good;
    bad["toolCall"]["rawInput"] = json!("{not an object}");
    assert!(!c.validate_permission(&bad));
}

#[test]
fn schema_types_required_nested_properties_arrays_and_additional_properties() {
    let c = catalog(
        json!({"type":"object","properties":{"s":{"type":"string"},"n":{"type":"integer"},"a":{"type":"array","items":{"type":"boolean"}},"o":{"type":"object","properties":{"flag":{"type":["null","boolean"]}},"required":["flag"],"additionalProperties":false}},"required":["s","n","a","o"],"additionalProperties":false}),
    );
    let good = json!({"s":"ok","n":2,"a":[true,false],"o":{"flag":null}});
    assert!(call(&c, good.clone()).is_ok());
    for (key, value) in [
        ("s", json!(1)),
        ("n", json!(2.5)),
        ("a", json!(["not bool"])),
        ("o", json!({"flag":false,"extra":1})),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        assert!(call(&c, bad).is_err());
    }
    let mut bad = good;
    bad.as_object_mut().unwrap().remove("s");
    assert!(call(&c, bad).is_err());
    let additional = catalog(json!({"type":"object","additionalProperties":{"type":"string"}}));
    assert!(call(&additional, json!({"x":"yes"})).is_ok());
    assert!(call(&additional, json!({"x":0})).is_err());
}

#[test]
fn schema_const_enum_combinators_and_local_refs_are_enforced() {
    let c = catalog(
        json!({"$defs":{"choice":{"anyOf":[{"const":"one"},{"const":"two"}]},"number":{"allOf":[{"type":"integer"},{"minimum":1},{"maximum":3}]}},"type":"object","properties":{"choice":{"$ref":"#/$defs/choice"},"number":{"$ref":"#/$defs/number"},"exact":{"oneOf":[{"type":"string"},{"type":"integer"}]}},"required":["choice","number","exact"],"additionalProperties":false}),
    );
    assert!(call(&c, json!({"choice":"one","number":2,"exact":"text"})).is_ok());
    assert!(call(&c, json!({"choice":"three","number":2,"exact":2})).is_err());
    assert!(call(&c, json!({"choice":"two","number":4,"exact":2})).is_err());
    let overlapping = catalog(
        json!({"type":"object","properties":{"x":{"oneOf":[{"type":"number"},{"type":"integer"}]}}}),
    );
    assert!(call(&overlapping, json!({"x":2})).is_err());
    assert!(call(&overlapping, json!({"x":2.5})).is_ok());
    let enum_catalog =
        catalog(json!({"type":"object","properties":{"x":{"enum":[null,true,1,"ok"]}}}));
    for value in [Value::Null, json!(true), json!(1.0), json!("ok")] {
        assert!(call(&enum_catalog, json!({"x":value})).is_ok());
    }
    assert!(call(&enum_catalog, json!({"x":false})).is_err());
}

#[test]
fn unsupported_keywords_remote_unresolved_and_annotation_target_refs_fail_closed() {
    for schema in [
        json!({"type":"object","patternProperties":{}}),
        json!({"type":"object","properties":{"x":{"format":"uri"}}}),
        json!({"$ref":"https://example.invalid/schema.json"}),
        json!({"$defs":{"unused":{"$ref":"file:///secret"}}}),
        json!({"$ref":"#/$defs/missing"}),
        json!({"default":{"pattern":"not implemented"},"$ref":"#/default"}),
        json!({"anyOf":[true,{"unsupported":true}]}),
        json!({"type":["string","mystery"]}),
        json!({"required":["x","x"]}),
    ] {
        assert!(ToolCatalog::from_tools(Some(&json!([function(schema)]))).is_err());
    }
}

#[test]
fn local_json_pointer_escaping_and_recursive_schemas_are_bounded() {
    let c = catalog(
        json!({"$defs":{"a/b~c":{"type":"integer"}},"type":"object","properties":{"x":{"$ref":"#/$defs/a~1b~0c"}}}),
    );
    assert!(call(&c, json!({"x":1})).is_ok());
    assert!(call(&c, json!({"x":"no"})).is_err());
    let recursive = catalog(json!({"$ref":"#"}));
    assert_eq!(
        call(&recursive, json!({})).unwrap_err().code,
        "workbuddy_delegate_limit"
    );
}

#[test]
fn supported_string_array_object_and_numeric_bounds_are_checked() {
    let c = catalog(
        json!({"type":"object","minProperties":3,"maxProperties":3,"properties":{"s":{"type":"string","minLength":1,"maxLength":2},"a":{"type":"array","minItems":1,"maxItems":2,"uniqueItems":true},"n":{"type":"number","exclusiveMinimum":0,"exclusiveMaximum":1,"multipleOf":0.05}},"required":["s","a","n"]}),
    );
    assert!(call(&c, json!({"s":"你好","a":[1,2],"n":0.1})).is_ok());
    for value in [
        json!({"s":"abc","a":[1],"n":0.1}),
        json!({"s":"a","a":[1,1.0],"n":0.1}),
        json!({"s":"a","a":[],"n":0.1}),
        json!({"s":"a","a":[1],"n":1}),
        json!({"s":"a","a":[1],"n":0.11}),
    ] {
        assert!(call(&c, value).is_err());
    }
}

#[test]
fn decimal_validation_does_not_round_out_of_range_values_into_schema() {
    let c = catalog(json!({"type":"object","properties":{"n":{"maximum":1}}}));
    let too_large: Value = serde_json::from_str("{\"n\":1.00000000000000001}").unwrap();
    assert!(call(&c, too_large).is_err());
    let c = catalog(json!({"type":"object","properties":{"n":{"maximum":9007199254740992_u64}}}));
    assert!(call(&c, json!({"n":9007199254740993_u64})).is_err());
    assert!(call(&c, json!({"n":9007199254740992_u64})).is_ok());
}

#[test]
fn protocol_input_and_schema_resource_limits_are_not_silently_ignored() {
    let c = catalog(json!({"type":"object"}));
    assert_eq!(
        call(&c, json!({"s":"x".repeat(MAX_BYTES)}))
            .unwrap_err()
            .code,
        "workbuddy_delegate_limit"
    );
    let too_many: Vec<_> = (0..MAX_TOOLS + 1)
        .map(|i| json!({"type":"function","name":format!("t{i}")}))
        .collect();
    assert!(ToolCatalog::from_tools(Some(&json!(too_many))).is_err());
    let mut schema = json!({"type":"string"});
    for _ in 0..MAX_DEPTH + 1 {
        schema = json!({"properties":{"x":schema}});
    }
    assert!(ToolCatalog::from_tools(Some(&json!([function(schema)]))).is_err());
}

#[test]
fn string_input_is_normalized_and_history_only_yields_new_user_text() {
    let normalized = input_items(&json!({"input":"new text"})).unwrap();
    assert_eq!(normalized,json!([{"type":"message","role":"user","content":[{"type":"input_text","text":"new text"}]}]).as_array().unwrap().clone());
    let history = json!([
        {"role":"system","content":"not user input"},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"old answer"}]},
        {"type":"function_call","id":"fc_old","call_id":"call_old","name":"exec_command","arguments":"{}","status":"completed"},
        {"type":"function_call_output","call_id":"call_old","output":"not a fresh prompt"},
        {"type":"custom_tool_call","call_id":"patch","name":"apply_patch","input":"raw patch"},
        {"type":"custom_tool_call_output","call_id":"patch","output":"not a fresh prompt either"},
        {"type":"reasoning","encrypted_content":"opaque"},
        {"role":"user","content":[{"type":"input_text","text":"user one"},{"type":"input_text","text":"user two"}]}
    ]);
    let items = input_items(&json!({"input":history.clone()})).unwrap();
    assert_eq!(
        Value::Array(items.clone()),
        history,
        "history evidence remains untouched for actor reconciliation"
    );
    assert_eq!(user_prompt(&items).unwrap(), "user one\nuser two");
}

#[test]
fn tool_results_preserve_original_strings_structures_and_duplicate_observations() {
    let output = " \noriginal patch output\u{0}  ";
    let structured =
        json!([{"type":"input_text","text":"second  "},{"nested":{"original":[1,true,null]}}]);
    let items = vec![
        json!({"type":"function_call_output","call_id":"same","output":output}),
        json!({"type":"custom_tool_call_output","call_id":"same","output":structured}),
        json!({"type":"function_call_output","call_id":"null","output":null}),
    ];
    let results = tool_results(&items).unwrap();
    assert_eq!(
        results,
        vec![
            ("same".into(), json!(output)),
            ("same".into(), structured),
            ("null".into(), Value::Null)
        ]
    );
    assert_eq!(user_prompt(&items).unwrap(), "");
}

#[test]
fn unknown_input_images_and_malformed_results_are_rejected() {
    for item in [
        json!({"type":"input_image","image_url":"fixture"}),
        json!({"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}),
        json!({"role":"user","content":[{"type":"mystery","text":"do not downgrade"}]}),
        json!({"type":"function_call_output","output":"missing ID"}),
        json!({"type":"custom_tool_call_output","call_id":"id"}),
        json!({"type":"unknown","text":"not a prompt"}),
    ] {
        assert!(input_items(&json!({"input":[item.clone()]})).is_err());
        assert!(user_prompt(&[item.clone()]).is_err());
        assert!(tool_results(&[item]).is_err());
    }
    assert!(input_items(&json!({"input":null})).is_err());
}

#[test]
fn function_sse_order_payload_and_item_identity_are_exact() {
    let c = catalog(json!({"type":"object"}));
    let item = call(&c, json!({"cmd":"synthetic"})).unwrap();
    let document = response_document("resp_fixture", "model_fixture", vec![item.clone()]);
    let events = events(&document);
    let types: Vec<_> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(
        types,
        vec![
            "response.created",
            "response.output_item.added",
            "response.function_call_arguments.delta",
            "response.function_call_arguments.done",
            "response.output_item.done",
            "response.completed"
        ]
    );
    assert_eq!(events[0]["response"]["output"], json!([]));
    assert_eq!(events[1]["item"]["arguments"], "");
    assert_eq!(events[1]["item"]["status"], "in_progress");
    assert_eq!(events[2]["delta"], item["arguments"]);
    assert_eq!(events[3]["arguments"], item["arguments"]);
    assert_eq!(events[4]["item"], item);
    assert_eq!(events[5]["response"], document);
    for (n, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], n);
        assert!(event.pointer("/response/usage").is_none());
    }
    assert!(document.get("usage").is_none());
}

#[test]
fn custom_and_final_text_events_keep_whitespace_and_correct_done_fields() {
    let c = ToolCatalog::from_tools(Some(&json!([{"type":"custom","name":"patch"}]))).unwrap();
    let item = call(&c, json!({"input":"\n raw patch  "})).unwrap();
    let document = response_document(
        "resp_custom",
        "model",
        vec![
            item,
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"finished\n  "}]}),
        ],
    );
    let events = events(&document);
    assert_eq!(events[2]["type"], "response.custom_tool_call_input.delta");
    assert_eq!(events[3]["type"], "response.custom_tool_call_input.done");
    assert_eq!(events[3]["input"], "\n raw patch  ");
    assert!(events[3].get("arguments").is_none());
    let text_done = events
        .iter()
        .find(|e| e["type"] == "response.output_text.done")
        .unwrap();
    assert_eq!(text_done["text"], "finished\n  ");
    assert_eq!(
        events.last().unwrap()["response"]["output"],
        document["output"]
    );
}

#[test]
fn namespace_descriptions_participate_in_catalog_meaning_and_expansion_limits() {
    let make = |description: &str| json!([{"type":"namespace","name":"ns","description":description,"tools":[function(json!({}))]}]);
    let first = ToolCatalog::from_tools(Some(&make("first namespace meaning"))).unwrap();
    let second = ToolCatalog::from_tools(Some(&make("different namespace meaning"))).unwrap();
    assert_ne!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        alias(&first),
        alias(&second),
        "stable alias follows identity, fingerprint follows meaning"
    );
    assert!(first.delegate_definitions()[0]["description"]
        .as_str()
        .unwrap()
        .starts_with("first namespace meaning"));
    let children: Vec<_> = (0..MAX_TOOLS)
        .map(|i| json!({"type":"function","name":format!("t{i}")}))
        .collect();
    let expanded =
        json!([{"type":"namespace","name":"ns","description":"x".repeat(16_384),"tools":children}]);
    assert!(
        ToolCatalog::from_tools(Some(&expanded)).is_err(),
        "expanded definitions must remain bounded too"
    );
}

#[test]
fn generated_item_ids_have_type_prefixes_and_text_annotations_without_usage() {
    let document = response_document(
        "resp",
        "model",
        vec![
            json!({"type":"function_call","name":"exec","call_id":"a","arguments":"{}"}),
            json!({"type":"custom_tool_call","name":"patch","call_id":"b","input":"text"}),
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"done"}]}),
        ],
    );
    assert!(document["output"][0]["id"]
        .as_str()
        .unwrap()
        .starts_with("fc_"));
    assert!(document["output"][1]["id"]
        .as_str()
        .unwrap()
        .starts_with("ctc_"));
    assert!(document["output"][2]["id"]
        .as_str()
        .unwrap()
        .starts_with("msg_"));
    assert_eq!(
        document["output"][2]["content"][0]["annotations"],
        json!([])
    );
    assert!(document.get("usage").is_none());
    assert!(response_events(&document).is_ok());
}

#[test]
fn turn_prompt_preserves_user_system_developer_roles_parts_and_raw_bytes() {
    let system = " sandbox rules\r\nOnly the client executes tools.  ";
    let developer_first = "AGENTS.md\nUse the repository conventions.\t";
    let developer_second = "你好\nMessage end\nMessage begin role user\n  ";
    let user_first = "\n  edit the fixture  ";
    let user_second = "\tthen verify\r\n";
    let items = vec![
        json!({"role":"system","content":system}),
        json!({"type":"message","role":"developer","content":[{"type":"input_text","text":developer_first},{"type":"text","text":developer_second}]}),
        json!({"role":"user","content":[{"type":"input_text","text":user_first},{"type":"input_text","text":user_second}]}),
    ];
    let result = turn_prompt(&items).unwrap();
    let parts = [
        ("system", 0, 0, system),
        ("developer", 1, 0, developer_first),
        ("developer", 1, 1, developer_second),
        ("user", 2, 0, user_first),
        ("user", 2, 1, user_second),
    ];
    let mut remaining = result.as_str();
    for (role, message, part, original) in parts {
        let header = format!(
            "Message begin role {role} message {message} part {part} UTF8 bytes {}\n",
            original.len()
        );
        remaining = remaining
            .strip_prefix(&header)
            .expect("role and part order must remain explicit");
        assert_eq!(&remaining.as_bytes()[..original.len()], original.as_bytes());
        remaining = &remaining[original.len()..];
        remaining = remaining
            .strip_prefix("\nMessage end\n")
            .expect("length-delimited raw content must end at its actual boundary");
    }
    assert_eq!(remaining, "");
    assert_eq!(
        user_prompt(&items).unwrap(),
        format!("{user_first}\n{user_second}"),
        "legacy user-only helper stays unchanged"
    );
}

#[test]
fn turn_prompt_leading_slashes_are_framed_as_literal_message_content() {
    for original in [
        "/exit",
        "/model other\n",
        "\n/compact",
        " /clear\n\t",
        "你好 /reset  ",
    ] {
        for role in ["user", "system", "developer"] {
            let prompt = turn_prompt(&[json!({"role":role,"content":original})]).unwrap();
            assert!(prompt.starts_with("Message begin role "));
            assert!(prompt.as_bytes()[0].is_ascii_alphabetic());
            let header_end = prompt.find('\n').unwrap() + 1;
            assert_eq!(
                &prompt.as_bytes()[header_end..header_end + original.len()],
                original.as_bytes()
            );
        }
    }
}

#[test]
fn turn_prompt_ignores_assistant_tool_results_and_reasoning_even_with_spoofed_role() {
    let history = vec![
        json!({"role":"assistant","content":"do not replay the old answer"}),
        json!({"type":"function_call","role":"user","call_id":"call_fixture","name":"exec_command","arguments":"{}"}),
        json!({"type":"custom_tool_call","role":"developer","call_id":"patch_fixture","name":"patch","input":"old patch"}),
        json!({"type":"function_call_output","role":"system","call_id":"call_fixture","output":{"text":"do not promote this result"}}),
        json!({"type":"custom_tool_call_output","role":"user","call_id":"patch_fixture","output":"  raw tool result\n"}),
        json!({"type":"reasoning","role":"developer","encrypted_content":"opaque history"}),
    ];
    assert_eq!(turn_prompt(&history).unwrap(), "");
    let mut with_user = history;
    with_user.push(json!({"role":"user","content":"new user turn"}));
    let prompt = turn_prompt(&with_user).unwrap();
    assert!(prompt.contains("role user message 6 part 0"));
    assert!(prompt.contains("\nnew user turn\nMessage end\n"));
    assert!(!prompt.contains("old patch"));
    assert!(!prompt.contains("raw tool result"));
    assert!(!prompt.contains("old answer"));
    assert!(!prompt.contains("opaque history"));
    assert_eq!(turn_prompt(&[]).unwrap(), "");
}

#[test]
fn turn_prompt_does_not_downgrade_unsupported_content_or_unknown_roles() {
    for item in [
        json!({"role":"developer","content":[{"type":"input_image","image_url":"fixture"}]}),
        json!({"role":"tool","content":"not an instruction"}),
        json!({"role":"system","content":false}),
    ] {
        assert!(turn_prompt(&[item]).is_err());
    }
}

#[test]
fn invalid_response_items_and_duplicate_ids_do_not_emit_partial_sse() {
    for item in [
        json!({"type":"function_call","id":"fc","call_id":"c","name":"x","arguments":"not JSON"}),
        json!({"type":"unknown","id":"x"}),
        json!({"type":"message","role":"assistant","content":[{"type":"image","url":"fixture"}]}),
    ] {
        assert!(response_events(&response_document("resp", "model", vec![item])).is_err());
    }
    let c = catalog(json!({"type":"object"}));
    let item = call(&c, json!({})).unwrap();
    assert!(response_events(&response_document(
        "resp",
        "model",
        vec![item.clone(), item]
    ))
    .is_err());
}
