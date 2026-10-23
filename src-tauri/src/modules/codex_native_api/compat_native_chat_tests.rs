use super::*;

#[test]
fn native_chat_preserves_only_options_its_selected_protocol_can_express() {
    let claude = prepare_native_chat(&json!({"model":"m","max_tokens":33,"messages":[{"role":"user","content":"hello"}],"stop_sequences":["END"]}), CompatFlavor::Claude, None, None).unwrap();
    assert_eq!(claude.body["stop"], json!(["END"]));
    assert_eq!(claude.body["max_tokens"], 33);
    assert!(claude.responses_error.is_some());
    assert_eq!(claude.responses["input"][0]["content"][0]["text"], "hello");
    let gemini = prepare_native_chat(&json!({"contents":[{"role":"user","parts":[{"text":"hello"}]}],"generationConfig":{"topK":4,"candidateCount":2,"stopSequences":["END"],"maxOutputTokens":44,"responseModalities":["TEXT"]}}), CompatFlavor::Gemini, Some("m"), Some(true)).unwrap();
    assert_eq!(gemini.body["top_k"], 4);
    assert_eq!(gemini.body["n"], 2);
    assert_eq!(gemini.body["stop"], json!(["END"]));
    assert_eq!(gemini.body["max_tokens"], 44);
    assert!(gemini.responses_error.is_some());
    // Native Chat cannot discard an opaque reasoning cipher to make a route work.
    assert!(prepare_native_chat(&json!({"model":"m","messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"plan","signature":"cipher"}]}]}), CompatFlavor::Claude,None,None).is_err());
}

#[test]
fn gemini_native_chat_multiple_candidates_keep_each_index_and_tool_arguments() {
    let context = CompatContext {
        flavor: CompatFlavor::Gemini,
        model: "m".into(),
        tool_names: Default::default(),
    };
    let response = response_from_chat(&json!({"id":"c","choices":[
        {"index":0,"message":{"role":"assistant","content":"one"},"finish_reason":"stop"},
        {"index":1,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call","type":"function","function":{"name":"f","arguments":"{\"x\":1}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":4}}), &context).unwrap();
    assert_eq!(response["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(response["candidates"][1]["index"], 1);
    assert_eq!(
        response["candidates"][1]["content"]["parts"][0]["functionCall"]["args"],
        json!({"x":1})
    );
    let mut stream = NativeChatStreamAdapter::new(context, 2);
    let mut frames = stream.push(&json!({"id":"c","choices":[{"index":1,"delta":{"content":"two"},"finish_reason":null},{"index":0,"delta":{"content":"one"},"finish_reason":null}]})).unwrap();
    frames.extend(stream.push(&json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"},{"index":1,"delta":{},"finish_reason":"stop"}]})).unwrap());
    stream
        .push(&json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":4}}))
        .unwrap();
    frames.extend(stream.finish_done().unwrap());
    assert!(frames
        .iter()
        .any(
            |frame| frame.data.pointer("/candidates/0/index") == Some(&json!(1))
                && frame.data.pointer("/candidates/0/content/parts/0/text") == Some(&json!("two"))
        ));
    assert!(frames
        .iter()
        .any(
            |frame| frame.data.pointer("/candidates/0/index") == Some(&json!(0))
                && frame.data.pointer("/candidates/0/content/parts/0/text") == Some(&json!("one"))
        ));
    assert_eq!(
        frames
            .iter()
            .filter(|frame| frame.data.pointer("/candidates/0/finishReason").is_some())
            .count(),
        2
    );
}

#[path = "compat_native_chat_http_tests.rs"]
mod http_tests;
