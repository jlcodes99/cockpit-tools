use super::*;
use serde_json::json;

fn context() -> CompatContext {
    CompatContext {
        flavor: CompatFlavor::Claude,
        model: "model".into(),
        tool_names: Default::default(),
    }
}
fn output() -> Value {
    json!({"id":"response-claude","status":"completed","output":[
        {"id":"reasoning-1","type":"reasoning","summary":[{"type":"summary_text","text":"plan"}],"encrypted_content":"cipher-1"},
        {"id":"search-1","type":"web_search_call","status":"completed","action":{"type":"search","query":"latest"},"results":[{"url":"https://example.test/result","title":"Result"}]},
        {"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}
    ],"usage":{"input_tokens":2,"output_tokens":3}})
}

#[test]
fn claude_signature_and_hosted_search_roundtrip_preserves_opaque_payload() {
    let response = response_from_responses(&output(), &context()).unwrap();
    assert_eq!(response["content"][0]["signature"], "cipher-1");
    assert_eq!(response["content"][1]["type"], "server_tool_use");
    assert_eq!(response["content"][2]["type"], "web_search_tool_result");
    let request = prepare_request(json!({"model":"model","messages":[{"role":"assistant","content":response["content"]}],
        "tools":[{"type":"web_search_20260209","name":"lookup","allowed_domains":["example.test"],"user_location":{"type":"approximate","country":"US"}}],
        "tool_choice":{"type":"tool","name":"lookup"}}), CompatFlavor::Claude, None, None).unwrap();
    assert_eq!(
        request.responses["input"][0]["encrypted_content"],
        "cipher-1"
    );
    assert_eq!(request.responses["input"][1]["type"], "web_search_call");
    assert_eq!(
        request.responses["input"][1]["results"][0]["url"],
        "https://example.test/result"
    );
    assert_eq!(request.responses["tools"][0]["type"], "web_search");
    assert_eq!(
        request.responses["tools"][0]["filters"]["allowed_domains"],
        json!(["example.test"])
    );
    assert_eq!(
        request.responses["tool_choice"],
        json!({"type":"web_search"})
    );
    for prefix in ["gpt#", "openai#", "codex#", ""] {
        assert_eq!(
            signature::input(Some(&json!(format!("{prefix}cipher-1"))))
                .unwrap()
                .as_deref(),
            Some("cipher-1")
        );
    }
    for signature in ["claude#cipher-1", "gemini#cipher-1", "gpt#"] {
        assert!(super::signature::input(Some(&json!(signature))).is_err());
    }
}

#[test]
fn claude_stream_emits_only_final_signatures_per_item_and_search_once() {
    let mut adapter = CompatStreamAdapter::new(context());
    let mut frames = adapter
        .push(&json!({"type":"response.created","response":{"id":"r","model":"model"}}))
        .unwrap();
    for (index, (id, text, cipher)) in [
        ("reasoning-1", "one", "final-1"),
        ("reasoning-2", "two", "final-2"),
    ]
    .into_iter()
    .enumerate()
    {
        frames.extend(adapter.push(&json!({"type":"response.output_item.added","output_index":index,"item":{"id":id,"type":"reasoning","summary":[],"encrypted_content":"initial"}})).unwrap());
        frames.extend(adapter.push(&json!({"type":"response.reasoning_summary_text.delta","output_index":index,"item_id":id,"delta":text})).unwrap());
        frames.extend(adapter.push(&json!({"type":"response.output_item.done","output_index":index,"item":{"id":id,"type":"reasoning","summary":[{"text":text}],"encrypted_content":cipher}})).unwrap());
    }
    let search = output()["output"][1].clone();
    frames.extend(
        adapter
            .push(&json!({"type":"response.output_item.done","item":search}))
            .unwrap(),
    );
    frames.extend(adapter.push(&json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[
        {"id":"reasoning-1","type":"reasoning","summary":[],"encrypted_content":"final-1"},
        {"id":"reasoning-2","type":"reasoning","summary":[],"encrypted_content":"final-2"},search],"usage":{}}})).unwrap());
    let signatures = frames
        .iter()
        .filter_map(|frame| {
            frame
                .data
                .pointer("/delta/signature")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(signatures, vec!["final-1", "final-2"]);
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame
                .data
                .pointer("/content_block/type")
                .and_then(Value::as_str)
                == Some("server_tool_use"))
            .count(),
        1
    );
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame
                .data
                .pointer("/content_block/type")
                .and_then(Value::as_str)
                == Some("web_search_tool_result"))
            .count(),
        1
    );
    for start in frames
        .iter()
        .filter(|frame| frame.data["type"] == "content_block_start")
    {
        assert_eq!(
            frames
                .iter()
                .filter(|frame| frame.data["type"] == "content_block_stop"
                    && frame.data["index"] == start.data["index"])
                .count(),
            1
        );
    }
    adapter.finish().unwrap();
}

#[test]
fn gemini_sdk_reasoning_aliases_match_current_go_semantics() {
    for config in [
        json!({"thinkingConfig":{"thinking_level":"HIGH"}}),
        json!({"thinking_level":"HIGH"}),
        json!({"thinkingConfig":{"thinking_budget":9000}}),
    ] {
        let converted = prepare_request(
            json!({"contents":[],"generationConfig":config}),
            CompatFlavor::Gemini,
            Some("model"),
            None,
        )
        .unwrap();
        assert_eq!(converted.responses["reasoning"]["effort"], "high");
    }
}

#[path = "compat_hosted_http_tests.rs"]
mod http_tests;

#[test]
fn claude_stream_uses_output_index_when_reasoning_ids_are_absent() {
    let mut stream = CompatStreamAdapter::new(context());
    let mut frames = stream
        .push(&json!({"type":"response.created","response":{"id":"no-item-ids"}}))
        .unwrap();
    for index in 0..2 {
        frames.extend(stream.push(&json!({"type":"response.reasoning_summary_text.delta","output_index":index,"delta":format!("plan-{index}")})).unwrap());
        frames.extend(stream.push(&json!({"type":"response.output_item.done","output_index":index,"item":{"type":"reasoning","summary":[{"text":format!("plan-{index}")}],"encrypted_content":format!("cipher-{index}")}})).unwrap());
    }
    frames.extend(stream.push(&json!({"type":"response.completed","response":{"id":"no-item-ids","status":"completed","output":[
        {"type":"reasoning","summary":[],"encrypted_content":"cipher-0"},
        {"type":"reasoning","summary":[],"encrypted_content":"cipher-1"}],"usage":{}}})).unwrap());
    let signatures = frames
        .iter()
        .filter_map(|frame| {
            frame
                .data
                .pointer("/delta/signature")
                .and_then(Value::as_str)
        })
        .collect::<Vec<_>>();
    assert_eq!(signatures, vec!["cipher-0", "cipher-1"]);
    let signature_blocks = frames
        .iter()
        .filter_map(|frame| {
            frame
                .data
                .get("delta")
                .and_then(|delta| delta.get("signature"))
                .map(|_| frame.data["index"].clone())
        })
        .collect::<Vec<_>>();
    assert_ne!(signature_blocks[0], signature_blocks[1]);
}
