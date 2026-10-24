use super::*;
use axum::http::HeaderValue;

fn headers(user_agent: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("user-agent", HeaderValue::from_str(user_agent).unwrap());
    headers
}

fn enabled() -> Options {
    Options {
        optimize: true,
        ..Options::default()
    }
}

fn tools() -> Value {
    json!([{"type":"namespace","name":"collaboration","tools":[
        {"type":"function","name":"spawn_agent","description":"Spawns an agent to work on a task.","parameters":{"type":"object","properties":{"message":{"type":"string","encrypted":true},"id":{"type":"string","encrypted":true}}}},
        {"type":"function","name":"send_message","parameters":{"properties":{"message":{"encrypted":true}}}},
        {"type":"function","name":"followup_task","parameters":{"properties":{"message":{"encrypted":true}}}},
        {"type":"function","name":"wait_agent","parameters":{"properties":{"message":{"encrypted":true}}}}
    ]}])
}

fn agent_message() -> Value {
    json!({"type":"agent_message","id":"agent-1","author":"/root","recipient":"/root/worker",
        "content":[{"type":"input_text","text":"Payload:\n"},{"type":"encrypted_content","encrypted_content":"task verbatim\nline two","extension":"keep"}],
        "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"},"extension":{"keep":true}})
}

fn profile() -> ModelProfile {
    ModelProfile {
        id: "fixture-model".into(),
        description: "Reasoning model".into(),
        reasoning_efforts: vec!["low".into(), "high".into()],
        default_reasoning_effort: "high".into(),
        service_tiers: vec!["priority".into()],
    }
}

#[test]
fn cpa_user_agent_matching_is_case_sensitive_and_requires_official_prefix() {
    for agent in [
        "Codex Desktop/1",
        "codex-tui/1",
        "codex_cli_rs",
        "codex_cli_rs/1",
        "codex_exec/1",
        " codex-tui/1 ",
    ] {
        assert!(is_codex_client(agent), "{agent}");
    }
    for agent in [
        "",
        "Codex",
        "codex_cli_rs-fake",
        "curl/8",
        "codex desktop/1",
        "evil Codex Desktop/1",
        "codex-tui",
    ] {
        assert!(!is_codex_client(agent), "{agent}");
    }
}

#[test]
fn cpa_optimization_requires_both_flag_and_official_client() {
    let original = json!({"tools":tools(),"input":[agent_message()]});
    for (user_agent, options) in [
        ("curl/8", enabled()),
        ("Codex Desktop/1", Options::default()),
    ] {
        let mut body = original.clone();
        let context = prepare(&headers(user_agent), &mut body, options, &[profile()]);
        assert_eq!(body, original);
        assert!(!context.optimized_namespace);
    }
}

#[test]
fn cpa_executor_optimization_preserves_native_agent_type_metadata_and_exact_text() {
    let original_message = agent_message();
    let mut body = json!({"tools":tools(),"input":[original_message.clone()]});
    let context = prepare(
        &headers("Codex Desktop/1"),
        &mut body,
        enabled(),
        &[profile()],
    );
    assert!(context.optimized_namespace);
    assert_eq!(body["tools"][0]["name"], OPTIMIZED);
    assert_eq!(body["input"][0]["type"], "agent_message");
    for key in [
        "id",
        "author",
        "recipient",
        "internal_chat_message_metadata_passthrough",
        "extension",
    ] {
        assert_eq!(body["input"][0][key], original_message[key]);
    }
    assert_eq!(
        body["input"][0]["content"][1],
        json!({"type":"input_text","text":"task verbatim\nline two","extension":"keep"})
    );
    for index in 0..3 {
        assert!(
            body["tools"][0]["tools"][index]["parameters"]["properties"]["message"]
                .get("encrypted")
                .is_none()
        );
    }
    assert_eq!(
        body["tools"][0]["tools"][0]["parameters"]["properties"]["id"]["encrypted"],
        true
    );
    assert_eq!(
        body["tools"][0]["tools"][3]["parameters"]["properties"]["message"]["encrypted"],
        true
    );
}

#[test]
fn cpa_compatibility_is_independent_of_client_and_optimization() {
    let mut body = json!({"tools":tools(),"input":[agent_message(),{"type":"message","role":"assistant","author":"/root","recipient":"/child","internal_chat_message_metadata_passthrough":{},"content":"reply","id":"keep"}]});
    let context = prepare(
        &headers("curl/8"),
        &mut body,
        Options {
            is_compat: true,
            ..Options::default()
        },
        &[profile()],
    );
    assert!(!context.optimized_namespace);
    assert_eq!(body["tools"], tools());
    assert_eq!(body["input"][0]["type"], "message");
    assert_eq!(body["input"][0]["role"], "user");
    for item in body["input"].as_array().unwrap() {
        for key in [
            "author",
            "recipient",
            "internal_chat_message_metadata_passthrough",
        ] {
            assert!(item.get(key).is_none());
        }
        assert!(item.get("id").is_some());
    }
    assert_eq!(body["input"][1]["role"], "assistant");
    assert_eq!(body["input"][0]["extension"], json!({"keep":true}));
}

#[test]
fn cpa_translation_optimization_converts_agent_type_but_keeps_native_metadata() {
    let mut body = json!({"input":[agent_message()]});
    prepare_translated_input(&headers("codex_cli_rs"), &mut body, enabled());
    assert_eq!(body["input"][0]["type"], "message");
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["author"], "/root");
    assert!(body["input"][0]
        .get("internal_chat_message_metadata_passthrough")
        .is_some());
}

#[test]
fn portable_input_converts_only_agent_items_and_never_interprets_or_decrypts_strings() {
    let mut body = json!({"input":[agent_message(),
        {"type":"reasoning","encrypted_content":"reasoning-cipher","signature":"sig"},
        {"type":"function_call","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"cipher\"}","encrypted_function_args":["message"]},
        {"type":"agent_message","content":[{"type":"encrypted_content","encrypted_content":42}, {"type":"input_text","text":"plain"}]}]});
    let reasoning = body["input"][1].clone();
    let call = body["input"][2].clone();
    portable_input(&mut body);
    assert_eq!(body["input"][1], reasoning);
    assert_eq!(body["input"][2], call);
    assert_eq!(body["input"][3]["content"][0]["encrypted_content"], 42);
    assert_eq!(
        body["input"][0]["content"][1]["text"],
        "task verbatim\nline two"
    );
}

#[test]
fn cpa_tools_prepare_at_boundary_and_executor_preserves_first_catalog() {
    let mut body = json!({"tools":tools()});
    let prepared = prepare_boundary(&headers("codex-tui/1"), &mut body, enabled(), &[profile()]);
    assert!(prepared);
    assert_eq!(body["tools"][0]["name"], COLLABORATION);
    let description = body["tools"][0]["tools"][0]["description"].clone();
    assert!(description.as_str().unwrap().contains("fixture-model"));
    let other = ModelProfile {
        id: "must-not-replace".into(),
        ..Default::default()
    };
    let context = prepare(
        &headers("codex-tui/1"),
        &mut body,
        Options {
            tools_prepared: prepared,
            ..enabled()
        },
        &[other],
    );
    assert!(context.optimized_namespace);
    assert_eq!(body["tools"][0]["tools"][0]["description"], description);
}

#[test]
fn cpa_additional_tools_and_nested_namespaces_are_prepared_without_rewriting_schemas() {
    let mut body = json!({"input":[{"type":"additional_tools","role":"developer","tools":[{"type":"namespace","name":"outer","tools":tools()}]}]});
    let context = prepare(&headers("codex_cli_rs/1"), &mut body, enabled(), &[]);
    assert!(context.optimized_namespace);
    assert_eq!(body["input"][0]["tools"][0]["tools"][0]["name"], OPTIMIZED);
    assert!(
        body["input"][0]["tools"][0]["tools"][0]["tools"][0]["parameters"]["properties"]["message"]
            .get("encrypted")
            .is_none()
    );
}

#[test]
fn cpa_message_tool_preparation_is_by_function_name_not_namespace() {
    let mut body = json!({"tools":[{"type":"function","name":"send_message","parameters":{"properties":{"message":{"encrypted":false}}}}, {"type":"custom","name":"spawn_agent","parameters":{"properties":{"message":{"encrypted":true}}}}]});
    let context = prepare(&headers("codex_exec/1"), &mut body, enabled(), &[]);
    assert!(!context.optimized_namespace);
    assert!(body["tools"][0]["parameters"]["properties"]["message"]
        .get("encrypted")
        .is_none());
    assert_eq!(
        body["tools"][1]["parameters"]["properties"]["message"]["encrypted"],
        true
    );
}

#[test]
fn cpa_namespace_renaming_requires_spawn_directly_inside_collaboration() {
    for names in [
        json!([{"type":"function","name":"send_message"}]),
        json!([{"type":"namespace","name":"nested","tools":[{"type":"function","name":"spawn_agent"}]}]),
    ] {
        let mut body = json!({"tools":[{"type":"namespace","name":"collaboration","tools":names}]});
        let context = prepare(&headers("Codex Desktop/1"), &mut body, enabled(), &[]);
        assert!(!context.optimized_namespace);
        assert_eq!(body["tools"][0]["name"], COLLABORATION);
    }
}

#[test]
fn cpa_namespace_conflicts_skip_renaming_and_model_rewrite_but_strip_encryption() {
    for name in [
        "collaboration-optimize",
        "collaboration-optimize__send_message",
        "collaboration-optimize.send_message",
    ] {
        let mut body = json!({"tools":tools(),"input":[{"type":"additional_tools","tools":[{"type":"function","name":name}]}]});
        let original_description = body["tools"][0]["tools"][0]["description"].clone();
        assert!(has_namespace_conflict(&body));
        let context = prepare(
            &headers("Codex Desktop/1"),
            &mut body,
            enabled(),
            &[profile()],
        );
        assert!(!context.optimized_namespace);
        assert_eq!(body["tools"][0]["name"], COLLABORATION);
        assert_eq!(
            body["tools"][0]["tools"][0]["description"],
            original_description
        );
        assert!(
            body["tools"][0]["tools"][0]["parameters"]["properties"]["message"]
                .get("encrypted")
                .is_none()
        );
    }
}

#[test]
fn cpa_response_restore_changes_all_wire_forms_without_touching_opaque_payloads() {
    let protected = json!({"type":"function_call","namespace":OPTIMIZED,"name":"collaboration-optimize.send_message"});
    let mut event = json!({"type":"response.completed","response":{"output":[
        {"type":"function_call","namespace":OPTIMIZED,"name":"spawn_agent","call_id":"one","arguments":protected.clone(),"encrypted_function_args":["message"],"signature":"keep"},
        {"type":"custom_tool_call","name":"collaboration-optimize.send_message","input":protected.clone()},
        {"type":"function_call","name":"collaboration-optimize__followup_task"},
        {"type":"function_call_output","output":protected.clone()},
        {"type":"custom_tool_call_output","output":protected.clone()},
        {"type":"namespace","name":OPTIMIZED}
    ]},"input":[protected.clone()]});
    Context {
        optimized_namespace: true,
        ..Default::default()
    }
    .restore(&mut event);
    let output = &event["response"]["output"];
    assert_eq!(output[0]["namespace"], COLLABORATION);
    assert_eq!(output[0]["arguments"], protected);
    assert_eq!(output[0]["signature"], "keep");
    assert_eq!(output[0]["encrypted_function_args"], json!(["message"]));
    assert_eq!(output[1]["namespace"], COLLABORATION);
    assert_eq!(output[1]["name"], "send_message");
    assert_eq!(output[1]["input"], protected);
    assert_eq!(output[2]["namespace"], COLLABORATION);
    assert_eq!(output[2]["name"], "followup_task");
    assert_eq!(output[3]["output"], protected);
    assert_eq!(output[4]["output"], protected);
    assert_eq!(output[5]["name"], COLLABORATION);
    assert_eq!(event["input"][0], protected);
}

#[test]
fn cpa_response_restore_requires_request_context_and_supports_sse_item_events() {
    for event_type in ["response.output_item.added", "response.output_item.done"] {
        for tool in ["spawn_agent", "wait_agent", "followup_task", "send_message"] {
            let original = json!({"type":event_type,"item":{
                "type":"function_call","name":format!("collaboration-optimize__{tool}"),
                "arguments":"opaque collaboration-optimize__wait_agent","call_id":"preserved"
            }});
            let mut event = original.clone();
            Context::default().restore(&mut event);
            assert_eq!(event, original);
            Context {
                optimized_namespace: true,
                ..Default::default()
            }
            .restore(&mut event);
            assert_eq!(event["item"]["namespace"], COLLABORATION);
            assert_eq!(event["item"]["name"], tool);
            assert_eq!(event["item"]["arguments"], original["item"]["arguments"]);
            assert_eq!(event["item"]["call_id"], "preserved");
        }
    }
}

fn orphan(name: &str, call_id: &str, output: Value) -> Value {
    json!({"type":"function_call_output","namespace":"codex_app","name":name,"call_id":call_id,"output":output})
}

#[test]
fn chained_cpa_flat_calls_restore_only_unambiguous_request_declarations() {
    let mut body = json!({"tools":tools(),"input":[]});
    let context = prepare(&headers("codex_cli_rs"), &mut body, enabled(), &[]);
    let protected = json!({"type":"function_call","name":"collaboration__wait_agent"});
    for event_type in ["response.output_item.added", "response.output_item.done"] {
        let mut event = json!({"type":event_type,"item":{
            "type":"function_call","name":"collaboration__wait_agent",
            "call_id":"unchanged","arguments":protected.clone()
        }});
        context.restore(&mut event);
        assert_eq!(event["item"]["namespace"], COLLABORATION);
        assert_eq!(event["item"]["name"], "wait_agent");
        assert_eq!(event["item"]["call_id"], "unchanged");
        assert_eq!(event["item"]["arguments"], protected);
    }
    for mut call in [
        json!({"type":"function_call","name":"collaboration__unknown"}),
        json!({"type":"function_call","namespace":"other","name":"collaboration__wait_agent"}),
        json!({"type":"message","name":"collaboration__wait_agent"}),
        json!({"type":"function_call_output","output":protected}),
    ] {
        let original = call.clone();
        context.restore(&mut call);
        assert_eq!(call, original);
    }
    for additional in [false, true] {
        let conflict = json!({"type":"function","name":"collaboration__wait_agent"});
        let mut body = json!({"tools":tools(),"input":[]});
        if additional {
            body["input"] = json!([{"type":"additional_tools","tools":[conflict]}]);
        } else {
            body["tools"].as_array_mut().unwrap().push(conflict);
        }
        let context = prepare(&headers("codex_cli_rs"), &mut body, enabled(), &[]);
        let mut call = json!({"type":"function_call","name":"collaboration__wait_agent"});
        let original = call.clone();
        context.restore(&mut call);
        assert_eq!(call, original);
    }
    let mut event = json!({"type":"function_call","name":"collaboration__wait_agent"});
    let original = event.clone();
    Context::default().restore(&mut event);
    assert_eq!(event, original);
}

fn child_headers() -> HeaderMap {
    let mut headers = headers("curl/8");
    headers.insert(
        "x-openai-subagent",
        HeaderValue::from_static(" COLLAb_SPAWN "),
    );
    headers
}

#[test]
fn cpa_orphan_requires_flag_and_exact_subagent_marker_independently_of_user_agent() {
    let original = json!({"input":[orphan("create_thread","",json!("child task"))]});
    for (headers, options) in [
        (
            headers("Codex Desktop/1"),
            Options {
                orphan_delegation: true,
                ..Default::default()
            },
        ),
        (child_headers(), Options::default()),
    ] {
        let mut body = original.clone();
        prepare(&headers, &mut body, options, &[]);
        assert_eq!(body, original);
    }
    let mut body = original;
    let prepared = prepare_boundary(
        &child_headers(),
        &mut body,
        Options {
            orphan_delegation: true,
            ..Default::default()
        },
        &[],
    );
    assert!(!prepared);
    assert_eq!(body["input"][0]["type"], "message");
    assert_eq!(
        body["input"][0]["content"][0]["text"],
        "Tool output from codex_app__create_thread:\nchild task"
    );
}

#[test]
fn cpa_orphan_counts_all_calls_then_consumes_pairings_once_without_trimming_ids() {
    let mut body = json!({"input":[
        orphan("create_thread","later",json!("paired later")),
        {"type":"function_call","call_id":"later","name":"anything"},
        orphan("create_thread","later",json!("duplicate output")),
        {"type":"function_call","call_id":" spaced ","name":"anything"},
        orphan("create_thread","spaced",json!("ID mismatch")),
        orphan("create_thread"," spaced ",json!("exact ID"))
    ]});
    let original = body.clone();
    prepare(
        &child_headers(),
        &mut body,
        Options {
            orphan_delegation: true,
            ..Default::default()
        },
        &[],
    );
    for index in [0, 1, 3, 5] {
        assert_eq!(body["input"][index], original["input"][index]);
    }
    for index in [2, 4] {
        assert_eq!(body["input"][index]["type"], "message");
    }
}

#[test]
fn cpa_orphan_matches_only_exact_codex_app_function_result_names() {
    let mut body = json!({"input":[
        orphan("send_message_to_thread","stale",json!({"child":"result"})),
        {"type":"function_call_output","namespace":"collaboration","name":"spawn_agent","output":"untouched"},
        orphan("codex_app__create_thread","",json!("untouched")),
        {"type":"custom_tool_call_output","namespace":"codex_app","name":"create_thread","output":"untouched"},
        orphan(" create_thread","",json!("untouched"))
    ]});
    let original = body.clone();
    prepare(
        &child_headers(),
        &mut body,
        Options {
            orphan_delegation: true,
            ..Default::default()
        },
        &[],
    );
    assert_eq!(
        body["input"][0]["content"][0]["text"],
        "Tool output from codex_app__send_message_to_thread:\n{\"child\":\"result\"}"
    );
    for index in 1..5 {
        assert_eq!(body["input"][index], original["input"][index]);
    }
}

#[test]
fn cpa_orphan_preserves_exact_string_output_and_null_or_missing_semantics() {
    let mut body = json!({"input":[orphan("create_thread","",json!("\n  exact \"quotes\"  \n")),orphan("create_thread","",Value::Null),{"type":"function_call_output","namespace":"codex_app","name":"create_thread"}]});
    prepare(
        &child_headers(),
        &mut body,
        Options {
            orphan_delegation: true,
            ..Default::default()
        },
        &[],
    );
    for (index, text) in ["\n  exact \"quotes\"  \n", "null", ""].iter().enumerate() {
        assert_eq!(
            body["input"][index]["content"][0]["text"],
            format!("Tool output from codex_app__create_thread:\n{text}")
        );
    }
}

#[test]
fn cpa_model_list_replaces_all_old_sections_preserves_instructions_and_is_idempotent() {
    let description = concat!("Preamble\n  Available model overrides (optional; inherited parent model is preferred):\n- `old`: obsolete\nSpawns an agent to work.\nKeep this instruction.\nAvailable model overrides (optional; inherited parent model is preferred):\n- `stale`: stale\nTrailing note.");
    let rendered = format_models(&[profile()]);
    let result = models::replace_model_sections(description, &rendered);
    assert_eq!(result.matches("Available model overrides").count(), 1);
    assert!(!result.contains("`old`") && !result.contains("`stale`"));
    assert!(result.starts_with("Preamble\n  Available model overrides"));
    assert!(result.contains("Spawns an agent to work.\nKeep this instruction.\nTrailing note."));
    assert!(result.contains("Reasoning efforts: low, high (default). Service tiers: priority."));
    assert_eq!(models::replace_model_sections(&result, &rendered), result);
    assert_eq!(models::replace_model_sections(description, ""), description);
}

#[test]
fn cpa_model_rendering_normalizes_lines_code_and_catalog_reasoning_metadata() {
    let metadata = json!({"description":"  Good\nmodel  ","supported_reasoning_levels":[{"effort":" HIGH "},{"effort":"unsupported"},{"effort":"low"}],"default_reasoning_level":"medium","service_tiers":[{"id":"priority"},{"id":"priority"},{"id":"ultrafast"}]});
    let mut profile = ModelProfile::from_metadata("fixture`id", &metadata);
    assert_eq!(profile.reasoning_efforts, vec!["high", "low"]);
    assert_eq!(profile.default_reasoning_effort, "high");
    assert_eq!(profile.service_tiers, vec!["priority", "ultrafast"]);
    assert!(format_models(&[profile.clone()]).starts_with("- `` fixture`id ``: Good model."));
    profile.apply_thinking(&[
        "none".into(),
        "high".into(),
        "medium".into(),
        "ultra".into(),
    ]);
    assert_eq!(profile.default_reasoning_effort, "medium");
    assert_eq!(
        profile.reasoning_efforts,
        vec!["none", "high", "medium", "ultra"]
    );
}

fn plaintext_request(name: &str, schema: Value) -> Value {
    json!({"tools":[{"type":"namespace","name":"collaboration","tools":[{
        "type":"function","name":name,"parameters":{"properties":{"message":schema}}
    }]}]})
}

fn plaintext_call(name: &str) -> Value {
    json!({"type":"function_call","namespace":"collaboration","name":name,"arguments":"{\"message\":\"task\"}"})
}

#[test]
fn native_plaintext_policy_requires_each_final_string_message_declaration() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        for (schema, expected) in [
            (json!({"type":"string"}), true),
            (json!({"type":"string","encrypted":false}), true),
            (json!({"type":"string","encrypted":true}), false),
            (json!({"type":"string","encrypted":"false"}), false),
            (json!({"type":"string","encrypted":null}), false),
            (json!({"type":"object"}), false),
            (json!({}), false),
        ] {
            let mut context = Context {
                plaintext_enabled: true,
                ..Default::default()
            };
            context.capture_final(&plaintext_request(name, schema.clone()));
            let mut event = json!({"type":"response.output_item.done","item":plaintext_call(name)});
            context.restore(&mut event);
            assert_eq!(
                event["item"].get("encrypted_function_args") == Some(&json!([])),
                expected,
                "{name} {schema}"
            );
        }
    }
}

#[test]
fn native_plaintext_policy_preserves_crypto_evidence_even_null_signatures() {
    let mut context = Context {
        plaintext_enabled: true,
        ..Default::default()
    };
    context.capture_final(&plaintext_request("send_message", json!({"type":"string"})));
    for (key, value) in [
        ("encrypted_function_args", json!(["message"])),
        ("encrypted_function_args", json!([])),
        ("encrypted_function_args", json!("unknown")),
        ("encrypted_content", json!("opaque")),
        ("encrypted_content", Value::Null),
        ("signature", json!("signature")),
        ("signature", Value::Null),
    ] {
        let mut item = plaintext_call("send_message");
        item[key] = value;
        let original = item.clone();
        context.restore(&mut item);
        assert_eq!(item, original);
    }
}

#[test]
fn native_plaintext_policy_restores_optimizer_alias_only_with_real_rename() {
    let request = json!({"tools":[{"type":"namespace","name":OPTIMIZED,"tools":[{"type":"function","name":"spawn_agent","parameters":{"properties":{"message":{"type":"string"}}}}]}]});
    for renamed in [false, true] {
        let mut context = Context {
            optimized_namespace: renamed,
            plaintext_enabled: true,
            ..Default::default()
        };
        context.capture_final(&request);
        let mut item = json!({"type":"function_call","namespace":OPTIMIZED,"name":"spawn_agent","arguments":"{}"});
        let mut wire = item.clone();
        context.mark_wire_calls(&mut wire);
        assert_eq!(wire["namespace"], OPTIMIZED);
        assert_eq!(
            wire.get("encrypted_function_args") == Some(&json!([])),
            renamed
        );
        context.restore(&mut item);
        assert_eq!(
            item.get("encrypted_function_args") == Some(&json!([])),
            renamed
        );
    }
}

#[test]
fn native_plaintext_policy_inherits_omitted_tools_but_explicit_empty_revokes() {
    let mut initial = Context {
        plaintext_enabled: true,
        ..Default::default()
    };
    initial.capture_final(&plaintext_request("send_message", json!({"type":"string"})));
    for (body, expected) in [
        (json!({"input":[]}), true),
        (json!({"tools":[]}), false),
        (json!({"tools":null}), false),
    ] {
        let mut next = Context {
            plaintext_enabled: true,
            ..Default::default()
        };
        next.capture_final(&body);
        next.inherit_plaintext(&initial);
        let mut call = plaintext_call("send_message");
        next.restore(&mut call);
        assert_eq!(
            call.get("encrypted_function_args") == Some(&json!([])),
            expected,
            "{body}"
        );
    }
}

#[test]
fn native_plaintext_policy_conflict_remains_denied_and_malformed_addition_revokes() {
    let mut initial = Context {
        plaintext_enabled: true,
        ..Default::default()
    };
    let tools = plaintext_request("send_message", json!({"type":"string"}))["tools"].clone();
    initial.capture_final(&json!({"input":[{"type":"additional_tools","tools":tools}]}));
    for body in [
        plaintext_request("send_message", json!({"type":"string","encrypted":true})),
        json!({"input":[{"type":"additional_tools","tools":null}]}),
    ] {
        let mut conflict = Context {
            plaintext_enabled: true,
            ..Default::default()
        };
        conflict.capture_final(&body);
        conflict.inherit_plaintext(&initial);
        let mut call = plaintext_call("send_message");
        conflict.restore(&mut call);
        assert!(call.get("encrypted_function_args").is_none());
        let mut next = Context {
            plaintext_enabled: true,
            ..Default::default()
        };
        next.capture_final(&json!({}));
        next.inherit_plaintext(&conflict);
        next.restore(&mut call);
        assert!(call.get("encrypted_function_args").is_none());
    }
}

#[test]
fn native_plaintext_policy_final_capture_reflects_actual_adapter_declarations() {
    let mut request = json!({"tools":tools()});
    let mut context = prepare(&headers("Codex Desktop/1"), &mut request, enabled(), &[]);
    let mut call = plaintext_call("spawn_agent");
    context.restore(&mut call);
    assert_eq!(call["encrypted_function_args"], json!([]));
    context.capture_final(&json!({"tools":[]}));
    let mut revoked = plaintext_call("spawn_agent");
    context.restore(&mut revoked);
    assert!(revoked.get("encrypted_function_args").is_none());
}
