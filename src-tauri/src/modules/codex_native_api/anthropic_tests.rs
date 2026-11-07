use super::*;

#[test]
fn deepseek_anthropic_effort_uses_output_config() {
    for (effort, expected) in [("low", "low"), ("xhigh", "high"), ("ultra", "max")] {
        let wire = responses_to_messages(&json!({"model":"vendor/deepseek-v5-pro", "input":"hi", "reasoning":{"effort":effort}})).unwrap();
        assert_eq!(wire["thinking"]["type"], "enabled");
        assert_eq!(wire["output_config"]["effort"], expected);
    }
}

#[test]
fn anthropic_agent_calls_keep_integer_arguments_in_json_and_streams() {
    let request = json!({"model":"claude-sonnet-4-6","input":"wait","tools":[{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"wait_agent","parameters":{"type":"object","properties":{"timeout_ms":{"type":"integer"}}}}]}]});
    let wire = responses_to_messages(&request).unwrap();
    let block = json!({"type":"tool_use","id":"toolu_wait","name":wire["tools"][0]["name"],"input":{"timeout_ms":10000.0,"text":"10000.0"}});
    let response = messages_to_response(
        &json!({"id":"msg_wait","content":[block.clone()],"stop_reason":"tool_use","usage":{}}),
        "claude-sonnet-4-6",
        &request,
    )
    .unwrap();
    let check = |item: &Value| {
        assert_eq!(item["name"], "wait_agent");
        assert_eq!(item["namespace"], "collaboration");
        let args: Value = serde_json::from_str(item["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["timeout_ms"].as_i64(), Some(10000));
        assert_eq!(args["text"], "10000.0");
    };
    check(&response["output"][0]);
    let mut adapter = ResponsesStreamAdapter::new("claude-sonnet-4-6".into(), &request).unwrap();
    let mut events = vec![];
    for event in [
        json!({"type":"message_start","message":{"id":"msg_wait","usage":{}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_wait","name":block["name"],"input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"timeout_ms\":1e4,\"text\":\"10000.0\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{}}),
        json!({"type":"message_stop"}),
    ] {
        events.extend(adapter.push(&event).unwrap());
    }
    check(&events.last().unwrap()["response"]["output"][0]);
    let done = events
        .iter()
        .find(|event| event["type"] == "response.function_call_arguments.done")
        .unwrap();
    assert!(done["arguments"]
        .as_str()
        .unwrap()
        .contains("\"timeout_ms\":10000"));
}

#[test]
fn anthropic_cpa_model_capabilities_preserve_max_effort_and_disable_modes() {
    for (model, requested, expected) in [
        ("claude-sonnet-4-5", None, 64000),
        ("claude-sonnet-4-5", Some(100000), 64000),
        ("claude-sonnet-4-5", Some(4000), 4000),
        ("claude-unknown", None, 32000),
    ] {
        let mut request = json!({"model":model,"input":"work"});
        if let Some(requested) = requested {
            request["max_output_tokens"] = json!(requested);
        }
        assert_eq!(
            responses_to_messages(&request).unwrap()["max_tokens"],
            expected
        );
    }
    for model in [
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        "claude-opus-5-5",
        "claude-sonnet-5-5-20261001[1m]",
    ] {
        let request = json!({"model":model,"input":"work","reasoning":{"effort":"max"}});
        assert_eq!(
            responses_to_messages(&request).unwrap()["output_config"]["effort"],
            "max"
        );
        let mut template = json!({});
        customize_model_template(model, &mut template);
        assert!(template["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|level| level["effort"] == "max"));
    }
    let disabled = responses_to_messages(
        &json!({"model":"claude-sonnet-5-5","input":"work","reasoning":{"effort":"none"}}),
    )
    .unwrap();
    assert_eq!(disabled["thinking"]["type"], "between_tools");
    assert!(responses_to_messages(
        &json!({"model":"claude-opus-5-5","input":"work","reasoning":{"effort":"none"}})
    )
    .is_err());
    let legacy = responses_to_messages(
        &json!({"model":"claude-haiku-4-5","input":"work","reasoning":{"effort":"medium"}}),
    )
    .unwrap();
    assert_eq!(legacy["thinking"]["budget_tokens"], 8192);
}

#[test]
fn anthropic_cpa_compaction_capsules_roundtrip_without_accepting_incomplete_summaries() {
    let fixture: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/anthropic-cpa-compaction.json"
    )))
    .unwrap();
    let capsule = fixture["capsule"].as_str().unwrap();
    assert_eq!(
        super::super::anthropic_compact::unseal(capsule).unwrap(),
        fixture["summary"]
    );
    let body = json!({"model":"claude-sonnet-4-6","input":[{"type":"compaction","encrypted_content":capsule},{"role":"user","content":"continue"}]});
    let replay = responses_to_messages(&body).unwrap();
    assert!(replay["system"][0]["text"]
        .as_str()
        .unwrap()
        .contains(fixture["summary"].as_str().unwrap()));
    let completed = json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"keep the plan"}]}],"usage":{"input_tokens":22,"output_tokens":4,"total_tokens":26}});
    let compact = super::super::anthropic_compact::finish(completed.clone(), "claude").unwrap();
    assert_eq!(compact["object"], "response.compaction");
    assert_eq!(compact["usage"], completed["usage"]);
    assert_eq!(
        super::super::anthropic_compact::unseal(
            compact["output"][0]["encrypted_content"].as_str().unwrap()
        )
        .unwrap(),
        "keep the plan"
    );
    for (field, value) in [
        ("status", json!("incomplete")),
        ("error", json!({"message":"failed"})),
        ("incomplete_details", json!({"reason":"max_output_tokens"})),
    ] {
        let mut summary = completed.clone();
        summary[field] = value;
        assert!(super::super::anthropic_compact::finish(summary, "claude").is_err());
    }
    assert!(super::super::anthropic_compact::unseal("cpa-ag-compact-v1:AAAA").is_err());
    assert!(super::super::anthropic_compact::finish(
        json!({"status":"completed","output":[]}),
        "claude"
    )
    .is_err());
}

#[test]
fn anthropic_search_preserves_results_citations_pause_and_session_isolation() {
    let result = json!({"type":"web_search_result","url":"https://example.test/docs","title":"Docs","encrypted_content":"opaque-search-result"});
    let citation = json!({"type":"web_search_result_location","url":"https://example.test/docs","title":"Docs","encrypted_index":"opaque-citation"});
    let request = json!({"model":"claude-sonnet-4-6","input":"search","tools":[{"type":"web_search","filters":{"allowed_domains":["example.test"]}}]});
    let wire = responses_to_messages(&request).unwrap();
    assert_eq!(wire["tools"][0]["type"], "web_search_20250305");
    assert_eq!(wire["tools"][0]["allowed_domains"], json!(["example.test"]));
    let response=messages_to_response(&json!({"id":"search","content":[{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search","input":{"query":"docs"}},{"type":"web_search_tool_result","tool_use_id":"srvtoolu_1","content":[result.clone()]},{"type":"text","text":"found","citations":[citation.clone()]}],"stop_reason":"end_turn","usage":{}}),"claude-sonnet-4-6",&request).unwrap();
    assert_eq!(response["output"].as_array().unwrap().len(), 2);
    assert_eq!(response["output"][0]["results"][0], result);
    assert_eq!(
        response["output"][1]["content"][0]["annotations"][0],
        citation
    );
    let mut replay = request.clone();
    replay["input"] = response["output"].clone();
    let messages = responses_to_messages(&replay).unwrap();
    assert_eq!(messages["messages"][0]["content"][1]["content"][0], result);
    assert_eq!(
        messages["messages"][0]["content"][2]["citations"][0],
        citation
    );
    let cache = super::super::anthropic_search_cache::Cache::default();
    cache.record(Some("caller-a/account-a/thread-a"), &response);
    replay["input"][0]
        .as_object_mut()
        .unwrap()
        .remove("results");
    let mut other = replay.clone();
    cache.restore(Some("caller-b/account-a/thread-a"), &mut other);
    assert!(other["input"][0].get("results").is_none());
    cache.restore(Some("caller-a/account-a/thread-a"), &mut replay);
    assert_eq!(replay["input"][0]["results"][0], result);
    let paused=messages_to_response(&json!({"id":"paused","content":[{"type":"server_tool_use","id":"srvtoolu_2","name":"web_search","input":{"query":"next"}}],"stop_reason":"pause_turn","usage":{}}),"claude",&request).unwrap();
    assert_eq!(paused["end_turn"], false);
    assert_eq!(paused["output"][0]["status"], "in_progress");
    replay["input"] = paused["output"].clone();
    assert_eq!(
        responses_to_messages(&replay).unwrap()["messages"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let refusal=messages_to_response(&json!({"id":"refusal","content":[{"type":"text","text":"declined"}],"stop_reason":"refusal","usage":{}}),"claude",&request).unwrap();
    assert_eq!(refusal["status"], "incomplete");
    assert_eq!(refusal["incomplete_details"]["reason"], "content_filter");
}

#[test]
fn anthropic_structured_output_and_forced_tools_follow_cpa_contract() {
    let schema =
        json!({"type":"object","properties":{"answer":{"type":"integer"}},"required":["answer"]});
    let body = json!({"model":"claude-sonnet-4-6","input":"work","text":{"format":{"type":"json_schema","schema":schema}},"tools":[{"type":"function","name":"calculate","parameters":{"type":"object"}}],"tool_choice":"required","reasoning":{"effort":"high"}});
    let wire = responses_to_messages(&body).unwrap();
    assert!(wire["system"][0]["text"]
        .as_str()
        .unwrap()
        .contains("answer"));
    assert_eq!(wire["tool_choice"]["type"], "any");
    assert!(wire.get("thinking").is_none());
    let patch =
        json!({"model":"claude","input":"work","tools":[{"type":"custom","name":"apply_patch"}]});
    for input in [
        json!({"input":42}),
        json!({"input":"patch","other":"unexpected"}),
    ] {
        assert!(messages_to_response(&json!({"id":"bad","content":[{"type":"tool_use","id":"c","name":"apply_patch","input":input}],"stop_reason":"tool_use","usage":{}}),"claude",&patch).is_err());
    }
}

#[test]
fn anthropic_codex_client_tools_preserve_arguments_results_and_freeform_grammars() {
    let args = json!({"command":"printf '中文'","nested":{"timeout":1234},"items":[1,true,null]});
    let patch = "*** Begin Patch\n*** Add File: 中文.txt\n+hello\n*** End Patch";
    let mut tools = Vec::new();
    let mut input = vec![json!({"role":"user","content":"work"})];
    for (index, (namespace, name)) in [
        ("functions", "exec_command"),
        ("functions", "write_stdin"),
        ("functions", "update_plan"),
        ("functions", "view_image"),
        ("functions", "request_user_input"),
        ("collaboration", "spawn_agent"),
        ("collaboration", "send_message"),
        ("collaboration", "wait"),
        ("collaboration", "close_agent"),
        ("mcp__workspace", "read_file"),
    ]
    .iter()
    .enumerate()
    {
        tools.push(json!({"type":"namespace","name":namespace,"tools":[{"type":"function","name":name,"parameters":{"type":"object","properties":{"nested":{"type":"object"}}}}]}));
        input.push(json!({"type":"function_call","namespace":namespace,"name":name,"call_id":format!("c_{index}"),"arguments":args.to_string()}));
        input.push(json!({"type":"function_call_output","call_id":format!("c_{index}"),"output":format!("result_{index}")}));
    }
    for name in ["apply_patch", "exec"] {
        tools.push(json!({"type":"custom","name":name,"format":{"type":"grammar","syntax":"lark","definition":"start: /[\\s\\S]+/"}}));
        input.push(json!({"type":"custom_tool_call","name":name,"call_id":name,"input":patch}));
        input.push(json!({"type":"custom_tool_call_output","call_id":name,"output":"ok"}));
    }
    let request = json!({"model":"claude-sonnet-4-6","input":input,"tools":tools});
    let wire = responses_to_messages(&request).unwrap();
    assert_eq!(wire["tools"].as_array().unwrap().len(), 12);
    let blocks: Vec<_> = wire["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().unwrap())
        .collect();
    assert_eq!(
        blocks
            .iter()
            .filter(|block| block["type"] == "tool_result")
            .count(),
        12
    );
    let calls: Vec<_> = blocks
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| (*block).clone())
        .collect();
    for call in &calls[..10] {
        assert_eq!(call["input"], args);
    }
    for call in &calls[10..] {
        assert_eq!(call["input"]["input"], patch);
    }
    for tool in &wire["tools"].as_array().unwrap()[10..] {
        assert!(tool["description"].as_str().unwrap().contains("start: /"));
        assert_eq!(tool["input_schema"]["required"], json!(["input"]));
    }
    let response = messages_to_response(
        &json!({"id":"msg_tools","content":calls,"stop_reason":"tool_use","usage":{}}),
        "claude-sonnet-4-6",
        &request,
    )
    .unwrap();
    for (index, item) in response["output"].as_array().unwrap().iter().enumerate() {
        if index < 10 {
            assert_eq!(
                serde_json::from_str::<Value>(item["arguments"].as_str().unwrap()).unwrap(),
                args
            );
            assert_eq!(item["name"], input[1 + index * 2]["name"]);
            assert_eq!(item["namespace"], input[1 + index * 2]["namespace"]);
        } else {
            assert_eq!(item["type"], "custom_tool_call");
            assert_eq!(item["input"], patch);
        }
    }
}

#[test]
fn anthropic_urls_preserve_gateway_prefix_and_avoid_duplicate_v1() {
    for (base, expected) in [
        (
            "https://example.invalid",
            "https://example.invalid/v1/messages",
        ),
        (
            "https://example.invalid/anthropic",
            "https://example.invalid/anthropic/v1/messages",
        ),
        (
            "https://example.invalid/anthropic/v1/",
            "https://example.invalid/anthropic/v1/messages",
        ),
        (
            "https://example.invalid/anthropic/v1/messages",
            "https://example.invalid/anthropic/v1/messages",
        ),
    ] {
        assert_eq!(endpoint(base, "messages").unwrap(), expected);
    }
    assert_eq!(
        endpoint("https://example.invalid/v1/messages", "models").unwrap(),
        "https://example.invalid/v1/models"
    );
    assert!(endpoint("file:///tmp/local", "messages").is_err());
}

#[test]
fn anthropic_long_tool_names_restore_the_original_namespace_and_name() {
    let name = "very_long_tool_name_".repeat(8);
    let body = json!({"model":"claude-sonnet-4-6","input":"question","tools":[{"type":"namespace","name":"mcp","tools":[{"type":"function","name":name,"parameters":{"type":"object"}}]}]});
    let wire = responses_to_messages(&body).unwrap();
    let wire_name = wire["tools"][0]["name"].as_str().unwrap();
    assert!(wire_name.len() <= 64);
    let response=messages_to_response(&json!({"id":"m","content":[{"type":"tool_use","id":"c","name":wire_name,"input":{}}],"stop_reason":"tool_use","usage":{}}),"claude-sonnet-4-6",&body).unwrap();
    assert_eq!(response["output"][0]["name"], name);
    assert_eq!(response["output"][0]["namespace"], "mcp");
}

#[test]
fn anthropic_maps_system_images_namespace_custom_tools_and_results_directly() {
    let request = json!({"model":"claude-sonnet-4-6","instructions":"system-a","input":[
        {"role":"developer","content":"system-b"},
        {"role":"user","content":[{"type":"input_text","text":"inspect"},{"type":"input_image","image_url":"data:image/png;base64,YQ=="}]},
        {"type":"custom_tool_call","call_id":"toolu_1","namespace":"functions","name":"apply_patch","input":"*** Begin Patch"},
        {"type":"custom_tool_call_output","call_id":"toolu_1","output":"done"}
    ],"tools":[{"type":"namespace","name":"functions","tools":[{"type":"custom","name":"apply_patch"}]}],"stream":false,"max_output_tokens":1234});
    let wire = responses_to_messages(&request).unwrap();
    assert_eq!(wire["system"][0]["text"], "system-a");
    assert_eq!(wire["system"][1]["text"], "system-b");
    assert_eq!(wire["max_tokens"], 1234);
    assert_eq!(
        wire["messages"][0]["content"][1]["source"]["media_type"],
        "image/png"
    );
    assert_eq!(
        wire["messages"][1]["content"][0]["name"],
        "functions__apply_patch"
    );
    assert_eq!(
        wire["messages"][1]["content"][0]["input"]["input"],
        "*** Begin Patch"
    );
    assert_eq!(wire["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
    let response=messages_to_response(&json!({"id":"msg_fixture","content":[{"type":"tool_use","id":"toolu_2","name":"functions__apply_patch","input":{"input":"patch-2"}}],"stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":20,"cache_creation_input_tokens":4}}),"claude-sonnet-4-6",&request).unwrap();
    assert_eq!(response["output"][0]["type"], "custom_tool_call");
    assert_eq!(response["output"][0]["namespace"], "functions");
    assert_eq!(response["output"][0]["input"], "patch-2");
    assert_eq!(response["usage"]["input_tokens"], 34);
    assert_eq!(response["usage"]["total_tokens"], 37);
}

#[test]
fn anthropic_reasoning_signatures_roundtrip_and_foreign_cipher_is_rejected() {
    let body = json!({"model":"claude-sonnet-4-6","input":"question"});
    let response=messages_to_response(&json!({"id":"m","content":[{"type":"thinking","thinking":"plan","signature":"signature-fixture"},{"type":"text","text":"reply"}],"stop_reason":"end_turn","usage":{}}),"claude-sonnet-4-6",&body).unwrap();
    let next = json!({"model":"claude-sonnet-4-6","input":response["output"]});
    assert_eq!(
        responses_to_messages(&next).unwrap()["messages"][0]["content"][0],
        json!({"type":"thinking","thinking":"plan","signature":"signature-fixture"})
    );
    assert!(responses_to_messages(&json!({"model":"claude","input":[{"type":"reasoning","summary":[],"encrypted_content":"gpt#private"}]})).is_err());
}

#[test]
fn anthropic_stream_keeps_tool_ids_signatures_usage_and_rejects_truncation() {
    let body = json!({"model":"claude-sonnet-4-6","input":"question","tools":[{"type":"custom","name":"patch"}]});
    let mut adapter = ResponsesStreamAdapter::new("claude-sonnet-4-6".into(), &body).unwrap();
    let mut events = vec![];
    for frame in [
        json!({"type":"message_start","message":{"id":"msg_stream","usage":{"input_tokens":12,"cache_read_input_tokens":7}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"patch","input":{}}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"input\":\"补"}}),
        json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"丁\"}"}}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":5}}),
        json!({"type":"message_stop"}),
    ] {
        events.extend(adapter.push(&frame).unwrap());
    }
    assert!(adapter.finish().unwrap().is_empty());
    let response = &events.last().unwrap()["response"];
    assert_eq!(response["output"][0]["encrypted_content"], "claude#signed");
    assert_eq!(response["output"][1]["type"], "custom_tool_call");
    assert_eq!(response["output"][1]["call_id"], "toolu_1");
    assert_eq!(response["output"][1]["input"], "补丁");
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "response.custom_tool_call_input.delta")
            .filter_map(|event| event["delta"].as_str())
            .collect::<String>(),
        "补丁"
    );
    assert_eq!(response["usage"]["total_tokens"], 24);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], index);
    }
    assert!(ResponsesStreamAdapter::new("claude".into(), &body)
        .unwrap()
        .finish()
        .is_err());
}

#[test]
fn anthropic_thinking_limits_and_unsupported_tools_fail_before_sending() {
    let body = json!({"model":"claude-sonnet-4-5","input":"q","max_output_tokens":4000,"reasoning":{"effort":"high"}});
    assert_eq!(
        responses_to_messages(&body).unwrap()["thinking"]["budget_tokens"],
        3999
    );
    assert!(responses_to_messages(
        &json!({"model":"claude","input":"q","max_output_tokens":500,"reasoning":{"effort":"high"}})
    )
    .is_err());
    assert!(responses_to_messages(
        &json!({"model":"claude","input":"q","tools":[{"type":"image_generation"}]})
    )
    .is_err());
}
