//! Fixtures mirror CLIProxyAPI v7.2.155's Codex Chat translator at 915e7a20:
//! codex_openai_response.go and codex_citations_followup_test.go. IDs/timestamps
//! are deliberately not golden-tested; content, offsets and terminal semantics are.
use super::*;
use serde_json::json;

fn context() -> ChatContext {
    ChatContext {
        flavor: ChatFlavor::ChatCompletions,
        model: "fixture".into(),
        include_usage: true,
        legacy_functions: false,
        tool_names: Default::default(),
    }
}
fn values(chunks: Vec<StreamOutput>) -> Vec<Value> {
    chunks
        .into_iter()
        .filter_map(|chunk| {
            if let StreamOutput::Event(value) = chunk {
                Some(value)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn go_output_nonstream_preserves_reasoning_images_unicode_citations_and_cache_writes() {
    let response = json!({"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[
        {"type":"reasoning","summary":[{"type":"summary_text","text":"summary"}],"content":[{"type":"reasoning_text","text":"details"}],"encrypted_content":"cipher-kept"},
        {"type":"message","content":[{"type":"output_text","text":"前文"},{"type":"output_text","text":"引用","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}]}]},
        {"type":"image_generation_call","result":"AAAA","output_format":"JPG"}
    ],"usage":{"input_tokens":9,"output_tokens":3,"input_tokens_details":{"cache_write_tokens":9007199254740993u64}}});
    let out = response_to_chat(&response, &context()).unwrap();
    let message = &out["choices"][0]["message"];
    assert_eq!(message["content"], "前文引用");
    assert_eq!(message["reasoning_content"], "summarydetails");
    assert_eq!(
        message["reasoning_details"][0]["encrypted_content"],
        "cipher-kept"
    );
    assert_eq!(
        message["images"][0]["image_url"]["url"],
        "data:image/jpeg;base64,AAAA"
    );
    assert_eq!(
        message["annotations"][0],
        json!({"type":"url_citation","url_citation":{"url":"https://example.com","title":"Example","start_index":2,"end_index":4}})
    );
    assert_eq!(out["choices"][0]["finish_reason"], "length");
    assert_eq!(
        out["choices"][0]["native_finish_reason"],
        "max_output_tokens"
    );
    assert_eq!(
        out["usage"]["prompt_tokens_details"]["cached_creation_tokens"],
        9007199254740993u64
    );
    assert_eq!(out["responses_output"][0]["type"], "image_generation_call");
}

#[test]
fn go_output_citation_events_deduplicate_and_use_unicode_offsets() {
    let mut stream = ChatStreamAdapter::new(context());
    stream.push(&json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"前文"})).unwrap();
    stream.push(&json!({"type":"response.output_text.delta","output_index":0,"content_index":1,"delta":"引用"})).unwrap();
    let annotation = json!({"type":"response.output_text.annotation.added","output_index":0,"content_index":1,"annotation":{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}});
    let chunks = values(stream.push(&annotation).unwrap());
    assert_eq!(
        chunks[0]["choices"][0]["delta"]["annotations"][0]["url_citation"]["start_index"],
        2
    );
    assert!(stream.push(&annotation).unwrap().is_empty());
    let chunks = values(stream.push(&json!({"type":"response.output_text.done","output_index":0,"content_index":0,"text":"前文","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example","start_index":0,"end_index":2}]})).unwrap());
    assert_eq!(
        chunks[0]["choices"][0]["delta"]["annotations"][0]["url_citation"]["start_index"],
        0
    );
}

#[test]
fn go_output_done_only_tool_arguments_resolve_interleaved_item_ids() {
    let mut stream = ChatStreamAdapter::new(context());
    for (index, id, name) in [(0, "a", "first"), (1, "b", "second")] {
        stream.push(&json!({"type":"response.output_item.added","output_index":index,"item":{"id":id,"type":"function_call","call_id":id,"name":name,"arguments":""}})).unwrap();
    }
    for (id, index, args) in [("b", 1, "{\"b\":2}"), ("a", 0, "{\"a\":1}")] {
        let chunks = values(stream.push(&json!({"type":"response.function_call_arguments.done","item_id":id,"arguments":args})).unwrap());
        assert_eq!(
            chunks[0]["choices"][0]["delta"]["tool_calls"][0]["index"],
            index
        );
        assert_eq!(
            chunks[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            args
        );
    }
    let chunks = stream
        .push(&json!({"type":"response.completed","response":{"status":"completed","output":[]}}))
        .unwrap();
    assert_eq!(chunks.last(), Some(&StreamOutput::Done));
    assert!(stream
        .push(&json!({"type":"response.failed","error":{"message":"late"}}))
        .unwrap()
        .is_empty());
}

#[test]
fn go_output_images_stream_immediately_and_deduplicate_final_image() {
    let mut stream = ChatStreamAdapter::new(context());
    let first = values(stream.push(&json!({"type":"response.image_generation_call.partial_image","item_id":"ig","output_index":0,"partial_image_b64":"AAAA","output_format":"image/webp"})).unwrap());
    assert_eq!(
        first.last().unwrap()["choices"][0]["delta"]["images"][0]["image_url"]["url"],
        "data:image/webp;base64,AAAA"
    );
    assert!(stream.push(&json!({"type":"response.output_item.done","output_index":0,"item":{"id":"ig","type":"image_generation_call","result":"AAAA","output_format":"webp"}})).unwrap().is_empty());
    let last = values(stream.push(&json!({"type":"response.output_item.done","output_index":0,"item":{"id":"ig","type":"image_generation_call","result":"BBBB","output_format":"gif"}})).unwrap());
    assert_eq!(
        last[0]["choices"][0]["delta"]["images"][0]["image_url"]["url"],
        "data:image/gif;base64,BBBB"
    );
}

#[test]
fn go_output_stream_preserves_raw_reasoning_done_text_and_failure_status() {
    let mut stream = ChatStreamAdapter::new(context());
    let chunks = values(stream.push(&json!({"type":"response.reasoning_text.done","output_index":0,"content_index":0,"text":"details"})).unwrap());
    assert_eq!(
        chunks.last().unwrap()["choices"][0]["delta"]["reasoning_content"],
        "details"
    );
    let error = stream.push(&json!({"type":"response.failed","response":{"error":{"status":429,"code":"rate_limit_exceeded","message":"limited"}}})).unwrap_err();
    assert_eq!(error.status, 429);
    assert!(stream.finish().is_err());
}

#[test]
fn go_output_unknown_incomplete_reason_is_not_tool_success() {
    let out = response_to_chat(&json!({"status":"incomplete","incomplete_details":{"reason":"unrecognized_limit"},"output":[{"type":"function_call","call_id":"a","name":"f","arguments":"{}"}]}), &context()).unwrap();
    assert_eq!(out["choices"][0]["finish_reason"], "stop");
    assert_eq!(
        out["choices"][0]["native_finish_reason"],
        "unrecognized_limit"
    );
}

#[test]
fn go_terminal_usage_remains_available_without_stream_options() {
    let mut context = context();
    context.include_usage = false;
    let mut stream = ChatStreamAdapter::new(context);
    let chunks = values(stream.push(&json!({"type":"response.completed","response":{"status":"completed","output":[],"usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}})).unwrap());
    assert_eq!(chunks.last().unwrap()["usage"]["total_tokens"], 5);
}

#[test]
fn go_limits_chat_stream_accepts_more_than_32_mib_and_4096_items() {
    let mut stream = ChatStreamAdapter::new(context());
    let delta = "x".repeat(4 * 1024 * 1024);
    for _ in 0..9 {
        stream.push(&json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":delta})).unwrap();
    }
    for index in 1..=4096 {
        stream.push(&json!({"type":"response.output_text.delta","output_index":index,"content_index":0,"item_id":format!("item-{index}"),"delta":"x"})).unwrap();
    }
    let terminal = stream.push(&json!({"type":"response.completed","response":{"id":"large-chat","status":"completed","output":[]}})).unwrap();
    assert!(!terminal.is_empty());
    assert!(stream.is_finished());
}

#[test]
fn go_limits_chat_upstream_stream_accepts_large_aggregate_and_tool_indexes() {
    let mut stream = crate::modules::codex_native_api::chat_upstream::ResponsesStreamAdapter::new("m".into());
    let delta = "x".repeat(3 * 1024 * 1024);
    for _ in 0..12 {
        stream.push(&json!({"choices":[{"index":0,"delta":{"content":delta}}]})).unwrap();
    }
    stream.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":5000,"id":"call-large-index","type":"function","function":{"name":"tool","arguments":"{}"}}]}}]})).unwrap();
    stream.push(&json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})).unwrap();
    let events = stream.finish().unwrap();
    assert!(events.iter().any(|event| event["type"] == "response.completed"));
    assert!(stream.finish().unwrap().is_empty());
}
