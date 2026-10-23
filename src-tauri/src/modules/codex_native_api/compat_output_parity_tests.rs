//! Fixed Go behavior references: codex_claude_response.go,
//! codex_gemini_response.go and cockpit-cliproxy/ollama_bridge.go at 915e7a20.
use super::*;
use serde_json::json;
fn context(flavor: CompatFlavor) -> CompatContext {
    CompatContext {
        flavor,
        model: "fixture".into(),
        tool_names: Default::default(),
    }
}

#[test]
fn go_claude_stop_reasons_cache_creation_and_content_reasoning_are_preserved() {
    for (reason, sequence, expected) in [
        ("stop", json!("END"), "stop_sequence"),
        ("pause_turn", Value::Null, "pause_turn"),
        (
            "model_context_window_exceeded",
            Value::Null,
            "model_context_window_exceeded",
        ),
        ("refusal", Value::Null, "refusal"),
    ] {
        let response = json!({"status":"completed","stop_reason":reason,"stop_sequence":sequence,"output":[{"type":"reasoning","content":[{"type":"reasoning_text","text":"plan"}],"encrypted_content":"preserved-cipher"}],"usage":{"input_tokens":20,"output_tokens":5,"input_tokens_details":{"cached_tokens":10,"cache_creation_tokens":7}}});
        let value = response_from_responses(&response, &context(CompatFlavor::Claude)).unwrap();
        assert_eq!(value["stop_reason"], expected);
        assert_eq!(value["stop_sequence"], sequence);
        assert_eq!(value["content"][0]["thinking"], "plan");
        assert_eq!(value["content"][0]["signature"], "preserved-cipher");
        assert_eq!(value["usage"]["input_tokens"], 10);
        assert_eq!(value["usage"]["cache_creation_input_tokens"], 7);
        let mut stream = CompatStreamAdapter::new(context(CompatFlavor::Claude));
        let frames = stream
            .push(&json!({"type":"response.completed","response":response}))
            .unwrap();
        let delta = frames
            .iter()
            .find(|frame| frame.data["type"] == "message_delta")
            .unwrap();
        assert_eq!(delta.data["delta"]["stop_reason"], expected);
        assert_eq!(delta.data["delta"]["stop_sequence"], sequence);
        assert_eq!(delta.data["usage"]["cache_creation_input_tokens"], 7);
    }
}

#[test]
fn go_gemini_token_counts_include_reasoning_and_metadata_has_creation_time() {
    let response = json!({"id":"r","created_at":1700000000,"status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":8,"output_tokens_details":{"reasoning_tokens":3}}});
    let out = response_from_responses(&response, &context(CompatFlavor::Gemini)).unwrap();
    assert_eq!(out["usageMetadata"]["candidatesTokenCount"], 8);
    assert_eq!(out["usageMetadata"]["thoughtsTokenCount"], 3);
    assert_eq!(out["usageMetadata"]["totalTokenCount"], 18);
    assert_eq!(out["responseId"], "r");
    assert_eq!(out["createTime"], "2023-11-14T22:13:20+00:00");
}

#[test]
fn compat_partial_images_are_immediate_with_correct_mime_for_every_client() {
    for flavor in [
        CompatFlavor::Claude,
        CompatFlavor::Gemini,
        CompatFlavor::Ollama,
    ] {
        let mut stream = CompatStreamAdapter::new(context(flavor));
        let frames = stream.push(&json!({"type":"response.image_generation_call.partial_image","item_id":"ig","partial_image_b64":"AAAA","output_format":"jpg"})).unwrap();
        assert!(
            frames
                .iter()
                .any(|frame| frame.data.to_string().contains("AAAA")),
            "{flavor:?}"
        );
        if flavor != CompatFlavor::Ollama {
            assert!(frames
                .iter()
                .any(|frame| frame.data.to_string().contains("image/jpeg")));
        }
        let done = stream.push(&json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"id":"ig","type":"image_generation_call","result":"AAAA","output_format":"jpg"}]}})).unwrap();
        assert!(!done
            .iter()
            .any(|frame| frame.data.to_string().contains("AAAA")));
        assert!(stream.is_finished());
    }
}

#[test]
fn go_ollama_final_chunk_contains_aggregate_text_thinking_and_duration_fields() {
    let mut stream = CompatStreamAdapter::new(context(CompatFlavor::Ollama));
    for event in [
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"delta":"plan"}),
        json!({"type":"response.output_text.delta","output_index":1,"delta":"answer"}),
    ] {
        stream.push(&event).unwrap();
    }
    let frames = stream.push(&json!({"type":"response.completed","response":{"status":"completed","output":[],"usage":{"input_tokens":2,"output_tokens":3}}})).unwrap();
    let final_frame = &frames.last().unwrap().data;
    assert_eq!(final_frame["message"]["content"], "answer");
    assert_eq!(final_frame["message"]["thinking"], "plan");
    for key in [
        "total_duration",
        "load_duration",
        "prompt_eval_duration",
        "eval_duration",
    ] {
        assert_eq!(final_frame[key], 0);
    }
    assert_eq!(final_frame["done"], true);
    assert_eq!(final_frame["prompt_eval_count"], 2);
    assert_eq!(final_frame["eval_count"], 3);
}

#[test]
fn go_claude_summary_parts_keep_one_final_signature_and_separate_paragraphs() {
    let mut stream = CompatStreamAdapter::new(context(CompatFlavor::Claude));
    let mut frames = Vec::new();
    for event in [
        json!({"type":"response.output_item.added","output_index":0,"item":{"id":"r","type":"reasoning","encrypted_content":"initial"}}),
        json!({"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":0}),
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"delta":"First part"}),
        json!({"type":"response.reasoning_summary_part.done","output_index":0,"summary_index":0}),
        json!({"type":"response.reasoning_summary_part.added","output_index":0,"summary_index":1}),
        json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":1,"delta":"Second part"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"r","type":"reasoning","encrypted_content":"final"}}),
    ] {
        frames.extend(stream.push(&event).unwrap());
    }
    let text: String = frames
        .iter()
        .filter_map(|frame| {
            frame
                .data
                .pointer("/delta/thinking")
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(text, "First part\n\nSecond part");
    let signatures: Vec<_> = frames
        .iter()
        .filter_map(|frame| {
            frame
                .data
                .pointer("/delta/signature")
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(signatures, ["final"]);
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.data["type"] == "content_block_stop")
            .count(),
        1
    );
}

#[test]
fn go_claude_terminal_only_reasoning_items_keep_distinct_signed_blocks() {
    let mut stream = CompatStreamAdapter::new(context(CompatFlavor::Claude));
    let frames = stream.push(&json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[
        {"id":"a","type":"reasoning","summary":[{"text":"first"}],"encrypted_content":"cipher-a"},
        {"id":"b","type":"reasoning","summary":[{"text":"second"}],"encrypted_content":"cipher-b"}
    ]}})).unwrap();
    let thinking: Vec<_> = frames
        .iter()
        .filter(|frame| frame.data.pointer("/delta/thinking").is_some())
        .map(|frame| {
            (
                frame.data["index"].as_u64().unwrap(),
                frame.data["delta"]["thinking"].as_str().unwrap(),
            )
        })
        .collect();
    let signatures: Vec<_> = frames
        .iter()
        .filter(|frame| frame.data.pointer("/delta/signature").is_some())
        .map(|frame| {
            (
                frame.data["index"].as_u64().unwrap(),
                frame.data["delta"]["signature"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(thinking, [(0, "first"), (1, "second")]);
    assert_eq!(signatures, [(0, "cipher-a"), (1, "cipher-b")]);
}

#[test]
fn go_claude_initial_signature_is_emitted_only_after_done_when_final_is_missing() {
    let mut stream = CompatStreamAdapter::new(context(CompatFlavor::Claude));
    let initial = stream.push(&json!({"type":"response.output_item.added","output_index":0,"item":{"id":"a","type":"reasoning","encrypted_content":"fallback-cipher"}})).unwrap();
    assert!(!initial
        .iter()
        .any(|frame| frame.data.to_string().contains("fallback-cipher")));
    let done = stream.push(&json!({"type":"response.output_item.done","output_index":0,"item":{"id":"a","type":"reasoning"}})).unwrap();
    assert!(done
        .iter()
        .any(|frame| frame.data["delta"]["signature"] == "fallback-cipher"));
}

#[test]
fn go_claude_error_types_cover_status_and_preserve_upstream_contract() {
    for (status, kind) in [
        (401, "authentication_error"),
        (402, "billing_error"),
        (403, "permission_error"),
        (404, "not_found_error"),
        (413, "request_too_large"),
        (429, "rate_limit_error"),
        (504, "timeout_error"),
        (529, "overloaded_error"),
        (500, "api_error"),
    ] {
        let failure = ServiceError::new(status, "native-code", "failed");
        assert_eq!(error(&failure, CompatFlavor::Claude)["error"]["type"], kind);
    }
    let failure = ServiceError::new(503,"upstream_error","fallback").with_error_details(json!({"type":"overloaded_error","code":"overloaded","message":"busy","param":"model","retryable":true}));
    let output = error(&failure, CompatFlavor::Claude);
    assert_eq!(output["error"]["type"], "overloaded_error");
    assert_eq!(output["error"]["message"], "busy");
    assert_eq!(output["error"]["retryable"], true);
    let policy = ServiceError::new(400, "upstream_error", "denied").with_error_details(
        json!({"type":"invalid_request","code":"cyber_policy","message":"denied"}),
    );
    assert_eq!(
        error(&policy, CompatFlavor::Claude)["error"]["type"],
        "invalid_request_error"
    );
}

#[test]
fn all_compat_clients_use_safe_structured_upstream_error_messages() {
    let raw = json!({"error":{"type":"rate_limit_error","code":"limited","message":"busy Bearer hidden-token https://private.invalid/?token=secret","retryable":true,"api_key":"secret-key"}});
    let failure = super::super::upstream_errors::chat_failure(&raw, 429).unwrap();
    assert_eq!(failure.response_error.as_ref().unwrap()["retryable"], true);
    for flavor in [
        CompatFlavor::Claude,
        CompatFlavor::Gemini,
        CompatFlavor::Ollama,
    ] {
        let output = error(&failure, flavor);
        let message = if flavor == CompatFlavor::Ollama {
            output["error"].as_str()
        } else {
            output["error"]["message"].as_str()
        }
        .unwrap();
        assert!(message.contains("busy"));
        let text = output.to_string();
        assert!(!text.contains("hidden-token"), "{flavor:?}: {text}");
        assert!(!text.contains("private.invalid"), "{flavor:?}: {text}");
        assert!(!text.contains("secret-key"), "{flavor:?}: {text}");
    }
}
