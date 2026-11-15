use super::*;

#[test]
fn cold_seed_input_configuration_and_zero_tool_counters_are_not_model_output() {
    let event = json!({"type":"response.completed","response":{
        "generate":false,"status":"completed","output":[],
        "image":{"context_compression":"enabled"},
        "reasoning":{"effort":"low","context":"automatic","mode":"adaptive","summary":null},
        "tool_usage":{"image_gen":{"input_tokens":0,"output_tokens":0,"input_tokens_details":{"image_tokens":0,"text_tokens":0}},"web_search":{"num_requests":0}},
        "usage":{"input_tokens":1000,"output_tokens":0,"total_tokens":1000}
    }});
    let normalized = preload_event(event);
    assert!(super::super::super::bootstrap::pre_execution(&normalized));
    assert!(normalized["response"].get("image").is_none());
    assert_eq!(normalized["response"]["usage"]["input_tokens"], 1000);
}

#[test]
fn cold_seed_real_output_and_nonzero_hosted_tool_work_still_fail_closed() {
    for patch in [
        json!({"tool_usage":{"web_search":{"num_requests":1}}}),
        json!({"image":{"url":"opaque-output"}}),
        json!({"reasoning":{"effort":"low","context":"actual generated text outside an enum"}}),
        json!({"output":[{"type":"function_call","call_id":"a","name":"side_effect","arguments":"{}"}]}),
        json!({"usage":{"input_tokens":100,"output_tokens":1}}),
    ] {
        let mut response = json!({"generate":false,"status":"completed","output":[]});
        response
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let normalized = preload_event(json!({"type":"response.completed","response":response}));
        assert!(!super::super::super::bootstrap::pre_execution(&normalized));
    }
}

#[test]
fn cold_seed_plan_preserves_all_items_images_and_opaque_fields() {
    let input: Vec<Value> = (0..10)
        .map(|n| {
            json!({"role":"user","content":[
        {"type":"input_text","text":format!("marker-{n}")},
        {"type":"input_image","image_url":format!("data:image/png;base64,{}", "A".repeat(2048))}
    ],"signature":format!("opaque-{n}")})
        })
        .collect();
    let source = json!({"type":"response.create","model":"m","instructions":"same rules",
        "reasoning":{"effort":"high"},"tools":[],"input":input});
    let original = source.clone();
    let plan = Plan::build(&source, 8192).unwrap().unwrap();
    assert!(plan.ranges.len() > 2);
    let mut reconstructed = Vec::new();
    for n in 0..plan.ranges.len() {
        let body = if n + 1 == plan.ranges.len() {
            plan.final_body(&source, "last-seed")
        } else {
            plan.body(&source, n, (n > 0).then_some("seed"), true)
        };
        assert!(json_bytes(&body) < 8192);
        assert_eq!(body["instructions"], source["instructions"]);
        assert_eq!(body["reasoning"], source["reasoning"]);
        assert_eq!(body["tools"], source["tools"]);
        assert_eq!(
            body.get("generate"),
            if n + 1 == plan.ranges.len() {
                None
            } else {
                Some(&Value::Bool(false))
            }
        );
        reconstructed.extend(body["input"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(Value::Array(reconstructed), source["input"]);
    assert_eq!(source, original);
}

#[test]
fn cold_seed_plan_keeps_parallel_calls_with_their_results() {
    let mut input = vec![json!({"role":"user","content":"A".repeat(2500)})];
    input.extend([
        json!({"type":"function_call","call_id":"a","name":"tool","arguments":"{}","encrypted_function_args":[]}),
        json!({"type":"custom_tool_call","call_id":"b","name":"other","input":"opaque"}),
        json!({"type":"function_call_output","namespace":"agent","name":"codex_apps_selected_page_part_1_of_1","output":"preserve external page observation"}),
        json!({"type":"function_call_output","call_id":"a","output":"A".repeat(700)}),
        json!({"type":"custom_tool_call_output","call_id":"b","output":"B".repeat(700)}),
        json!({"role":"user","content":"Z".repeat(2500)}),
    ]);
    let source = json!({"type":"response.create","model":"m","input":input});
    let plan = Plan::build(&source, 5000).unwrap().unwrap();
    assert!(plan.ranges.iter().any(|r| r.start <= 1 && r.end >= 6));
    assert!(plan.ranges.iter().all(|r| !(2..6).contains(&r.end)));
}

#[test]
fn cold_seed_plan_falls_back_for_an_oversized_item_or_tool_group() {
    assert!(Plan::build(
        &json!({"input":[{"role":"user","content":"A".repeat(20000)}]}),
        8192
    )
    .is_err());
    let source = json!({"input":[
        {"type":"function_call","call_id":"a","name":"x","arguments":"A".repeat(5000)},
        {"type":"function_call_output","call_id":"a","output":"B".repeat(5000)}
    ]});
    assert_eq!(
        Plan::build(&source, 8192).err().unwrap().code,
        "websocket_context_seed_unavailable"
    );
}

#[test]
fn cold_seed_plan_does_not_seed_small_requests_or_incomplete_tool_history() {
    assert!(Plan::build(
        &json!({"model":"m","input":[{"role":"user","content":"hello"}]}),
        8192
    )
    .unwrap()
    .is_none());
    let source = json!({"input":[
        {"role":"user","content":"A".repeat(5000)},
        {"type":"function_call","call_id":"a","name":"x","arguments":"A".repeat(4000)}
    ]});
    assert!(Plan::build(&source, 8192).is_err());
}
