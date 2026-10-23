use super::*;
use crate::modules::codex_native_api::chat_upstream::{self, ResponsesStreamAdapter};
use serde_json::json;
fn ctx() -> ChatContext {
    ChatContext {
        flavor: ChatFlavor::ChatCompletions,
        model: "m".into(),
        include_usage: true,
        legacy_functions: false,
        tool_names: Default::default(),
    }
}
#[test]
fn native_chat_request_multimodal_tools_and_schema() {
    let request=prepare_request(json!({"model":"m","messages":[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AA","detail":"high"}}]}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object"}}}],"response_format":{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object"},"strict":true}},"reasoning_effort":"high","max_tokens":123}),ChatFlavor::ChatCompletions).unwrap();
    assert_eq!(
        request.responses["input"][0]["content"][1]["type"],
        "input_image"
    );
    assert_eq!(request.responses["tools"][0]["strict"], false);
    assert_eq!(request.responses["text"]["format"]["name"], "answer");
    assert_eq!(request.responses["max_output_tokens"], 123);
    let wire = chat_upstream::responses_to_chat(&request.responses).unwrap();
    assert_eq!(
        wire["messages"][0]["content"][1]["image_url"]["detail"],
        "high"
    );
    assert_eq!(wire["response_format"]["json_schema"]["name"], "answer");
}
#[test]
fn native_chat_invalid_nested_parameters_do_not_panic() {
    for extra in [
        json!({"reasoning":"bad","reasoning_effort":"high"}),
        json!({"text":false,"response_format":{"type":"text"}}),
    ] {
        let mut body = json!({"model":"m","messages":[]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(prepare_request(body, ChatFlavor::ChatCompletions).is_err());
    }
}
#[test]
fn native_chat_rejects_lossy_options() {
    for extra in [
        json!({"n":2}),
        json!({"stop":["END"]}),
        json!({"seed":1}),
        json!({"frequency_penalty":1}),
    ] {
        let mut body = json!({"model":"m","messages":[]});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(prepare_request(body, ChatFlavor::ChatCompletions).is_err());
    }
}
#[test]
fn native_chat_custom_tool_roundtrip() {
    let body = json!({"model":"m","messages":[{"role":"assistant","tool_calls":[{"id":"call1","type":"custom","custom":{"name":"x","input":"foo"}}]},{"role":"tool","tool_call_id":"call1","content":"done"}]});
    let request = prepare_request(body, ChatFlavor::ChatCompletions).unwrap();
    assert_eq!(
        request.responses["input"][1]["type"],
        "custom_tool_call_output"
    );
    let wire = chat_upstream::responses_to_native_chat(&request.responses).unwrap();
    assert_eq!(
        wire["messages"][0]["tool_calls"][0]["custom"]["input"],
        "foo"
    );
}
#[test]
fn native_chat_response_usage_tools_and_length() {
    let response=chat_upstream::chat_to_response(&json!({"id":"x","choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"length"}],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8,"prompt_tokens_details":{"cached_tokens":2}}}),"m").unwrap();
    assert_eq!(response["status"], "incomplete");
    let out = response_to_chat(&response, &ctx()).unwrap();
    assert_eq!(out["choices"][0]["finish_reason"], "length");
    assert_eq!(out["usage"]["prompt_tokens_details"]["cached_tokens"], 2);
}
#[test]
fn native_chat_stream_late_usage_and_tool_identity() {
    let mut upstream = ResponsesStreamAdapter::new("m".into());
    let mut events = Vec::new();
    for chunk in [
        json!({"id":"a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"x\":"}}]},"finish_reason":null}]}),
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":"f","arguments":"1}"}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":5,"total_tokens":8}}),
    ] {
        events.extend(upstream.push(&chunk).unwrap());
    }
    assert!(!events.iter().any(|e| e["type"] == "response.completed"));
    events.extend(upstream.finish_done().unwrap());
    assert_eq!(
        events.last().unwrap()["response"]["usage"]["total_tokens"],
        8
    );
    assert_eq!(
        events.last().unwrap()["response"]["output"][0]["arguments"],
        "{\"x\":1}"
    );
    let mut edge = ChatStreamAdapter::new(ctx());
    let mut chunks = Vec::new();
    for e in events {
        chunks.extend(edge.push(&e).unwrap());
    }
    assert!(edge.finish().is_ok());
    assert_eq!(chunks.last(), Some(&StreamOutput::Done));
    assert!(chunks
        .iter()
        .any(|c| matches!(c,StreamOutput::Event(v) if v["usage"]["total_tokens"]==8)));
}
#[test]
fn native_chat_stream_eof_is_not_success() {
    let mut upstream = ResponsesStreamAdapter::new("m".into());
    upstream
        .push(&json!({"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":null}]}))
        .unwrap();
    assert!(upstream.finish().is_err());
    assert!(upstream.finish_done().is_ok());
    assert!(ChatStreamAdapter::new(ctx()).finish().is_err());
}
#[test]
fn native_chat_completed_only_stream_flushes_text() {
    let mut edge = ChatStreamAdapter::new(ctx());
    let chunks=edge.push(&json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"full"}]}],"usage":{"input_tokens":1,"output_tokens":2}}})).unwrap();
    assert!(chunks.iter().any(
        |c| matches!(c,StreamOutput::Event(v) if v["choices"][0]["delta"]["content"]=="full")
    ));
}
#[test]
fn native_chat_upstream_rejects_opaque_state() {
    assert!(
        chat_upstream::responses_to_chat(&json!({"input":[],"previous_response_id":"other"}))
            .is_err()
    );
    assert!(chat_upstream::responses_to_chat(
        &json!({"input":[{"type":"reasoning","encrypted_content":"secret"}]})
    )
    .is_err());
}
#[test]
fn native_chat_legacy_completion() {
    let req = prepare_request(
        json!({"model":"m","prompt":"hey","max_tokens":5}),
        ChatFlavor::Completions,
    )
    .unwrap();
    let out=response_to_chat(&json!({"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"yes"}]}]}),&req.context).unwrap();
    assert_eq!(out["object"], "text_completion");
    assert_eq!(out["choices"][0]["text"], "yes");
}
