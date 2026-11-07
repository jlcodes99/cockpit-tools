use super::*;
use axum::http::HeaderMap;

fn search() -> Value {
    json!({"type":"tool_search","execution":"client","description":"Search available local tools","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"],"additionalProperties":false}})
}

fn prepare(mut body: Value) -> (Value, Context) {
    let context =
        super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut body, false).unwrap();
    (body, context)
}

fn local_tools(body: &Value) -> Vec<Value> {
    body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "additional_tools")
        .flat_map(|item| item["tools"].as_array().unwrap().iter().cloned())
        .collect()
}

#[test]
fn chatgpt_plan_discovery_lowers_only_client_search_and_preserves_native_tools() {
    let shell = json!({"type":"function","name":"shell_command","description":"Run local commands","parameters":{"type":"object","properties":{"command":{"type":"string"}}}});
    let custom = json!({"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":"start: /.+/"}});
    let namespace = json!({"type":"namespace","name":"mcp__workspace","tools":[{"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}]});
    let history = json!([{"type":"reasoning","encrypted_content":"opaque-openai-cipher","summary":[]},{"type":"function_call","name":"shell_command","call_id":"call_shell","arguments":"{\"command\":\"echo hello\"}"},{"type":"function_call_output","call_id":"call_shell","output":"hello"},{"type":"custom_tool_call","name":"apply_patch","call_id":"call_patch","input":"*** Begin Patch\n*** End Patch"},{"type":"custom_tool_call_output","call_id":"call_patch","output":"done"}]);
    let (body, mut context) = prepare(
        json!({"tools":[search(),shell.clone(),custom.clone(),namespace.clone()],"input":history}),
    );
    let tools = local_tools(&body);
    assert!(tools.contains(&shell));
    assert!(tools.contains(&custom));
    assert_eq!(body["tools"], json!([namespace]));
    for (index, item) in history.as_array().unwrap().iter().enumerate() {
        assert_eq!(&body["input"][index], item);
    }
    let hidden = tools
        .iter()
        .find(|tool| tool["name"] == context.wire_name.as_ref().unwrap().as_str())
        .unwrap();
    assert_eq!(hidden["type"], "function");
    assert_eq!(hidden["parameters"], search()["parameters"]);
    assert!(hidden.get("execution").is_none());
    let call = json!({"type":"function_call","name":hidden["name"],"id":"fc_discovery","call_id":"call_discovery","arguments":"{\"goal\":\"files\"}","status":"completed"});
    let output = restore_event(
        &mut context,
        json!({"type":"response.completed","response":{"output":[call]}}),
    )
    .unwrap();
    assert_eq!(
        output[0]["response"]["output"][0]["type"],
        "tool_search_call"
    );
    assert_eq!(
        output[0]["response"]["output"][0]["arguments"],
        json!({"goal":"files"})
    );
}

#[test]
fn chatgpt_plan_discovered_namespace_and_tool_results_survive_next_round() {
    let discovered = json!([{"type":"namespace","name":"collaboration","description":"Local agents","tools":[{"type":"function","name":"spawn_agent","defer_loading":true,"parameters":{"type":"object","properties":{"message":{"type":"string"}}}}]}]);
    let history = json!([
        {"type":"tool_search_call","execution":"client","id":"tsc_search","call_id":"search","arguments":{"goal":"agents"},"status":"completed"},
        {"type":"tool_search_output","execution":"client","call_id":"search","status":"completed","tools":discovered},
        {"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"child","arguments":"{\"message\":\"do work\"}","encrypted_function_args":[]},
        {"type":"function_call_output","call_id":"child","output":"{\"agent_id\":\"worker\"}"}
    ]);
    let (body, mut context) = prepare(json!({"tools":[search()],"input":history}));
    assert_eq!(body["input"][0]["type"], "function_call");
    assert_eq!(body["input"][0]["id"], "fc_search");
    assert_eq!(body["input"][0]["call_id"], "search");
    assert_eq!(body["input"][1]["type"], "function_call_output");
    assert_eq!(
        serde_json::from_str::<Value>(body["input"][1]["output"].as_str().unwrap()).unwrap(),
        discovered
    );
    assert_eq!(body["tools"][0]["name"], "collaboration");
    assert_eq!(body["tools"][0]["tools"][0]["name"], "spawn_agent");
    assert!(body["tools"][0]["tools"][0].get("defer_loading").is_none());
    assert_eq!(body["input"][2], history[2]);
    assert_eq!(body["input"][3], history[3]);
    let response = restore_event(&mut context, json!({"output":[{"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"new-child","arguments":"{\"message\":\"plain task\"}"}]})).unwrap();
    assert_eq!(
        response[0]["output"][0]["encrypted_function_args"],
        json!([])
    );
}

#[test]
fn chatgpt_plan_search_sse_hides_private_function_events_and_keeps_call_identity() {
    let (_body, mut context) = prepare(json!({"tools":[search()],"input":[]}));
    let wire = context.wire_name.clone().unwrap();
    let added = restore_event(&mut context, json!({"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","name":wire,"id":"fc_search","call_id":"call_search","arguments":""}})).unwrap();
    assert_eq!(added[0]["item"]["id"], "tsc_search");
    assert_eq!(added[0]["item"]["arguments"], json!({}));
    for delta in ["{\"goal\":\"查", "找工具\"}"] {
        assert!(restore_event(&mut context, json!({"type":"response.function_call_arguments.delta","item_id":"fc_search","delta":delta})).unwrap().is_empty());
    }
    assert!(restore_event(
        &mut context,
        json!({"type":"response.function_call_arguments.done","output_index":2})
    )
    .unwrap()
    .is_empty());
    let done = restore_event(&mut context, json!({"type":"response.output_item.done","output_index":2,"item":{"type":"function_call","name":wire,"id":"fc_search","call_id":"call_search","arguments":"","status":"completed"}})).unwrap();
    assert_eq!(done[0]["item"]["arguments"], json!({"goal":"查找工具"}));
    assert_eq!(done[0]["item"]["call_id"], "call_search");
    assert!(done[0]["item"].get("name").is_none());
}

#[test]
fn chatgpt_plan_discovery_names_do_not_shadow_real_functions_or_other_request_contexts() {
    let (body, mut context) = prepare(
        json!({"tools":[search(),{"type":"function","name":"cockpit_client_tool_search","parameters":{"type":"object"}}],"input":[]}),
    );
    assert_ne!(
        context.wire_name.as_deref(),
        Some("cockpit_client_tool_search")
    );
    let native = json!({"output":[{"type":"function_call","name":"cockpit_client_tool_search","call_id":"ordinary","arguments":"{}"}]});
    assert_eq!(
        restore_event(&mut context, native.clone()).unwrap(),
        vec![native]
    );
    let (_body, mut other) = prepare(
        json!({"tools":[{"type":"function","name":"shell_command","parameters":{"type":"object"}}],"input":[]}),
    );
    let search_call = json!({"output":[{"type":"function_call","name":context.wire_name,"call_id":"foreign","arguments":"{}"}]});
    assert_eq!(
        restore_event(&mut other, search_call.clone()).unwrap(),
        vec![search_call]
    );
    assert!(body["input"].is_array());
}

#[test]
fn chatgpt_plan_hosted_tools_report_type_and_safe_param_without_echoing_schema() {
    for (body, path) in [
        (
            json!({"tools":[{"type":"tool_search","execution":"server","description":"private schema secret"}],"input":[]}),
            "tools[0]",
        ),
        (
            json!({"input":[{"type":"tool_search_output","execution":"client","call_id":"c","tools":[{"type":"file_search","description":"private schema secret"}]}]}),
            "input[0].tools[0]",
        ),
        (
            json!({"input":[{"type":"additional_tools","tools":[{"type":"mcp","description":"private schema secret"}]}]}),
            "input[0].tools[0]",
        ),
    ] {
        let error = super::super::chatgpt_plan_wire::validate(&body, false).unwrap_err();
        assert_eq!(error.response_error.as_ref().unwrap()["param"], path);
        assert!(!error.message.contains("private schema secret"));
        assert!(
            error.message.contains("tool_search")
                || error.message.contains("file_search")
                || error.message.contains("mcp")
        );
    }
}

#[test]
fn chatgpt_plan_real_codex_cli_local_tool_schemas_and_multi_agent_v1_are_preserved() {
    // Captured only public local-tool declarations from a loopback Codex CLI
    // request. The fixture contains no prompt, request headers, or credentials.
    let tools: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/chatgpt_plan_codex_local_tools.json")).unwrap();
    let (body, mut context) = prepare(json!({"tools":tools,"input":[]}));
    let local = local_tools(&body);
    for tool in tools.iter().filter(|tool| tool["type"] == "function") {
        assert!(local.contains(tool), "{}", tool["name"]);
    }
    assert_eq!(
        body["tools"],
        json!(tools
            .iter()
            .filter(|tool| tool["type"] == "namespace")
            .collect::<Vec<_>>())
    );
    let args = "{\"message\":\"Review local files\",\"agent_type\":\"default\"}";
    let call = json!({"type":"function_call","namespace":"multi_agent_v1","name":"spawn_agent","call_id":"real-cli-shape","arguments":args});
    assert_eq!(
        restore_event(&mut context, json!({"output":[call.clone()]})).unwrap()[0]["output"][0],
        call
    );
}

#[test]
fn chatgpt_plan_qualified_discovered_functions_use_native_namespace_containers() {
    let declaration = json!({"type":"function","namespace":"mcp__local","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}});
    let (body, _) = prepare(
        json!({"tools":[search()],"input":[{"type":"tool_search_output","execution":"client","call_id":"lookup","tools":[declaration]}]}),
    );
    let namespace = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "mcp__local")
        .unwrap();
    assert_eq!(namespace["tools"][0]["name"], "read_file");
    assert_eq!(
        namespace["tools"][0]["parameters"],
        declaration["parameters"]
    );
    assert!(namespace["tools"][0].get("namespace").is_none());
}

#[test]
fn chatgpt_plan_ambiguous_search_history_never_leaks_native_search_items_upstream() {
    let mut body =
        json!({"input":[{"type":"tool_search_output","call_id":"unknown-search","tools":[]} ]});
    let error = super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut body, false)
        .unwrap_err();
    assert_eq!(error.response_error.unwrap()["param"], "input[0]");
    assert!(error.message.contains("tool_search_output"));
}

#[test]
fn chatgpt_plan_nested_namespaces_and_qualified_tools_merge_without_losing_schemas() {
    let (body, _) = prepare(json!({"tools":[
        {"type":"function","namespace":"mcp","name":"read","parameters":{"type":"object"}},
        {"type":"namespace","name":"mcp","tools":[{"type":"function","name":"list","parameters":{"type":"object"}},{"type":"namespace","name":"nested","tools":[{"type":"custom","name":"patch","format":{"type":"text"}}]}]}
    ],"input":[]}));
    assert_eq!(body["tools"].as_array().unwrap().len(), 2);
    assert_eq!(body["tools"][0]["name"], "mcp");
    assert_eq!(body["tools"][0]["tools"].as_array().unwrap().len(), 2);
    assert_eq!(body["tools"][1]["name"], "mcp.nested");
    assert_eq!(body["tools"][1]["tools"][0]["type"], "custom");
}

#[test]
fn chatgpt_plan_bare_or_server_search_is_hosted_and_never_reinterpreted_as_client_search() {
    for tool in [
        json!({"type":"tool_search"}),
        json!({"type":"tool_search","execution":"server"}),
    ] {
        let mut body = json!({"tools":[tool],"input":[]});
        let error =
            super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut body, false)
                .unwrap_err();
        assert_eq!(error.response_error.unwrap()["param"], "tools[0]");
        assert!(error.message.contains("tool_search"));
    }
    let mut additional =
        json!({"input":[{"type":"additional_tools","tools":[{"type":"tool_search"}]}]});
    let error =
        super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut additional, false)
            .unwrap_err();
    assert_eq!(error.response_error.unwrap()["param"], "input[0].tools[0]");
    for item in [
        json!({"type":"tool_search_call","call_id":"bare","arguments":{}}),
        json!({"type":"tool_search_output","call_id":"bare","tools":[]}),
    ] {
        let mut body = json!({"tools":[search()],"input":[item]});
        let error =
            super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut body, false)
                .unwrap_err();
        assert_eq!(error.response_error.unwrap()["param"], "input[0]");
    }
}

#[test]
fn chatgpt_plan_search_choice_requires_a_matching_explicit_client_declaration() {
    for choice in [
        json!({"type":"tool_search"}),
        json!({"type":"tool_search","execution":"client"}),
    ] {
        let mut absent = json!({"input":[],"tool_choice":choice});
        let error =
            super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut absent, false)
                .unwrap_err();
        assert_eq!(error.response_error.unwrap()["param"], "tool_choice");
        let (body, context) = prepare(json!({"tools":[search()],"input":[],"tool_choice":choice}));
        assert_eq!(body["tool_choice"]["type"], "function");
        assert_eq!(body["tool_choice"]["name"], context.wire_name.unwrap());
        assert!(body["tool_choice"].get("execution").is_none());
    }
    let (body, _) = prepare(
        json!({"input":[{"type":"additional_tools","tools":[search()]}],"tool_choice":{"type":"tool_search"}}),
    );
    assert_eq!(body["tool_choice"]["type"], "function");
    let mut disabled = json!({"tools":[],"input":[{"type":"additional_tools","tools":[search()]}],"tool_choice":{"type":"tool_search"}});
    assert!(
        super::super::chatgpt_plan_wire::prepare(&mut HeaderMap::new(), &mut disabled, false)
            .is_err()
    );
}

#[test]
fn chatgpt_plan_search_done_and_completed_must_preserve_the_same_delivered_call() {
    let (_body, mut context) = prepare(json!({"tools":[search()],"input":[]}));
    let item = json!({"type":"function_call","name":context.wire_name.clone(),"id":"fc_search","call_id":"call_search","arguments":"{\"goal\":\"files\"}","status":"completed"});
    let done = json!({"type":"response.output_item.done","output_index":0,"item":item});
    let restored = restore_event(&mut context, done.clone()).unwrap();
    assert_eq!(restored[0]["item"]["arguments"], json!({"goal":"files"}));
    let delivered = context.clone();
    let terminal =
        json!({"type":"response.completed","response":{"status":"completed","output":[item]}});
    assert!(restore_event(&mut context, terminal.clone()).is_ok());
    let mut contradiction = terminal.clone();
    contradiction["response"]["output"][0]["arguments"] =
        json!("{\"goal\":\"private replacement\"}");
    let error = restore_event(&mut delivered.clone(), contradiction).unwrap_err();
    assert_eq!(error.status, 502);
    assert!(!error.message.contains("private replacement"));
    let mut duplicate_done = done;
    duplicate_done["item"]["id"] = json!("fc_changed");
    assert_eq!(
        restore_event(&mut delivered.clone(), duplicate_done)
            .unwrap_err()
            .status,
        502
    );
    let omitted = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[]}]}});
    assert_eq!(
        restore_event(&mut delivered.clone(), omitted)
            .unwrap_err()
            .status,
        502
    );
}

#[test]
fn chatgpt_plan_search_done_can_fill_an_omitted_or_empty_terminal_snapshot() {
    for output in [None, Some(json!([]))] {
        let (_body, mut context) = prepare(json!({"tools":[search()],"input":[]}));
        let item = json!({"type":"function_call","name":context.wire_name.clone(),"id":"fc_search","call_id":"call_search","arguments":"{\"goal\":\"files\"}","status":"completed"});
        let mut done = restore_event(
            &mut context,
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
        )
        .unwrap()
        .pop()
        .unwrap();
        let restored = done["item"].clone();
        let mut collector = super::super::output_items::OutputCollector::new();
        collector.observe(&mut done).unwrap();
        let mut terminal = json!({"type":"response.completed","response":{"id":"resp_search","status":"completed"}});
        if let Some(output) = output {
            terminal["response"]["output"] = output;
        }
        let mut terminal = restore_event(&mut context, terminal)
            .unwrap()
            .pop()
            .unwrap();
        assert!(collector.can_restore_identity(&terminal));
        assert!(collector.observe(&mut terminal).unwrap());
        assert_eq!(terminal["response"]["output"], json!([restored]));
        assert_eq!(
            terminal["response"]["output"][0]["type"],
            "tool_search_call"
        );
        assert_eq!(
            terminal["response"]["output"][0]["arguments"],
            json!({"goal":"files"})
        );
    }
}
