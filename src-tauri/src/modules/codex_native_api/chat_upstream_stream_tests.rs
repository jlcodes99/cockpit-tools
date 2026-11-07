//! Behavior reference: CLIProxyAPI v7.2.155 at 915e7a20,
//! openai_openai-responses_response.go::finalizeOpenItems. These exercise the
//! adapter used by HTTP and WebSocket Chat-provider Responses output.
use super::*;

fn tool_chunk(arguments: &str, custom: bool, reason: Option<&str>) -> Value {
    let call = if custom {
        json!({"index":0,"id":"call","type":"custom","custom":{"name":"apply_patch","input":arguments}})
    } else {
        json!({"index":0,"id":"call","type":"function","function":{"name":"lookup","arguments":arguments}})
    };
    json!({"id":"reply","choices":[{"index":0,"delta":{"tool_calls":[call]},"finish_reason":reason}]})
}

fn close(adapter: &mut ResponsesStreamAdapter, done: bool) -> Result<Vec<Value>, ServiceError> {
    if done {
        adapter.finish_done()
    } else {
        adapter.finish()
    }
}

#[test]
fn truncated_function_json_never_emits_success_for_explicit_finish_or_done() {
    for reason in [
        None,
        Some("tool_calls"),
        Some("stop"),
        Some("function_call"),
    ] {
        for done in [false, true] {
            for arguments in ["{", "{\"query\":\"partial", "{\"query\":1} trailing", " "] {
                let mut adapter = ResponsesStreamAdapter::new("fixture".into());
                let initial = adapter.push(&tool_chunk(arguments, false, reason)).unwrap();
                assert!(initial.iter().all(|event| !event["type"]
                    .as_str()
                    .unwrap()
                    .ends_with(".done")
                    && event["type"] != "response.completed"));
                assert!(
                    close(&mut adapter, done).is_err(),
                    "{reason:?} done={done} arguments={arguments:?}"
                );
                assert!(!adapter.finished);
                assert_eq!(adapter.output[0]["status"], "in_progress");
                assert!(close(&mut adapter, done).is_err());
            }
        }
    }
}

#[test]
fn no_argument_bytes_without_finish_reason_are_not_synthesized_into_success() {
    for done in [false, true] {
        let mut adapter = ResponsesStreamAdapter::new("fixture".into());
        adapter.push(&tool_chunk("", false, None)).unwrap();
        assert!(close(&mut adapter, done).is_err());
        assert_eq!(adapter.output[0]["arguments"], "");
        assert_eq!(adapter.output[0]["status"], "in_progress");
    }
}

#[test]
fn valid_function_json_and_explicit_empty_arguments_complete_with_late_usage() {
    for reason in ["tool_calls", "stop", "function_call"] {
        for arguments in [
            "",
            "{}",
            "{\"query\":1}",
            "[1,2]",
            "null",
            "true",
            "1",
            "\"text\"",
            "1e999",
        ] {
            for done in [false, true] {
                let mut adapter = ResponsesStreamAdapter::new("fixture".into());
                adapter
                    .push(&tool_chunk(arguments, false, Some(reason)))
                    .unwrap();
                adapter.push(&json!({"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":5}})).unwrap();
                let events = close(&mut adapter, done).unwrap();
                let expected = if arguments.is_empty() {
                    "{}"
                } else {
                    arguments
                };
                let arguments_done = events
                    .iter()
                    .find(|event| event["type"] == "response.function_call_arguments.done")
                    .unwrap();
                assert_eq!(arguments_done["arguments"], expected);
                let item = &events
                    .iter()
                    .find(|event| event["type"] == "response.output_item.done")
                    .unwrap()["item"];
                assert_eq!(item["status"], "completed");
                assert_eq!(item["arguments"], expected);
                let terminal = events.last().unwrap();
                assert_eq!(terminal["type"], "response.completed");
                assert_eq!(terminal["response"]["output"][0], *item);
                assert_eq!(terminal["response"]["usage"]["total_tokens"], 5);
                assert!(close(&mut adapter, done).unwrap().is_empty());
            }
        }
    }
}

#[test]
fn valid_json_without_finish_reason_requires_done_marker() {
    let mut adapter = ResponsesStreamAdapter::new("fixture".into());
    adapter.push(&tool_chunk("{}", false, None)).unwrap();
    assert!(adapter.finish().is_err());
    let events = adapter.finish_done().unwrap();
    assert_eq!(events.last().unwrap()["type"], "response.completed");
}

#[test]
fn length_and_content_filter_keep_raw_function_and_custom_tools_incomplete() {
    for (reason, incomplete_reason) in [
        ("length", "max_output_tokens"),
        ("content_filter", "content_filter"),
    ] {
        for custom in [false, true] {
            for arguments in ["", "{\"partial\":", "{}", "freeform patch"] {
                for done in [false, true] {
                    let mut adapter = ResponsesStreamAdapter::new("fixture".into());
                    adapter
                        .push(&tool_chunk(arguments, custom, Some(reason)))
                        .unwrap();
                    let events = close(&mut adapter, done).unwrap();
                    assert!(events
                        .iter()
                        .all(|event| event["type"] != "response.completed"));
                    let item = &events
                        .iter()
                        .find(|event| event["type"] == "response.output_item.done")
                        .unwrap()["item"];
                    assert_eq!(item["status"], "incomplete");
                    assert_eq!(item[if custom { "input" } else { "arguments" }], arguments);
                    let terminal = events.last().unwrap();
                    assert_eq!(terminal["type"], "response.incomplete");
                    assert_eq!(terminal["response"]["output"][0], *item);
                    assert_eq!(
                        terminal["response"]["incomplete_details"]["reason"],
                        incomplete_reason
                    );
                }
            }
        }
    }
}

#[test]
fn explicit_custom_completion_preserves_arbitrary_or_empty_input_without_json_validation() {
    for input in ["", "*** Begin Patch\n+hello", "{unclosed", " "] {
        let mut adapter = ResponsesStreamAdapter::new("fixture".into());
        adapter
            .push(&tool_chunk(input, true, Some("tool_calls")))
            .unwrap();
        let events = adapter.finish_done().unwrap();
        let input_done = events
            .iter()
            .find(|event| event["type"] == "response.custom_tool_call_input.done")
            .unwrap();
        assert_eq!(input_done["input"], input);
        let terminal = events.last().unwrap();
        assert_eq!(terminal["type"], "response.completed");
        assert_eq!(terminal["response"]["output"][0]["input"], input);
    }
}

#[test]
fn one_invalid_parallel_function_prevents_all_successful_tool_sealing() {
    let mut adapter = ResponsesStreamAdapter::new("fixture".into());
    adapter.push(&tool_chunk("{}", false, None)).unwrap();
    adapter.push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"second","type":"function","function":{"name":"lookup","arguments":"{\"partial\":"}}]},"finish_reason":"tool_calls"}]})).unwrap();
    assert!(adapter.finish_done().is_err());
    assert!(adapter
        .output
        .iter()
        .all(|item| item["status"] == "in_progress"));
}

#[test]
fn nonstream_length_and_filter_keep_message_and_tool_items_incomplete() {
    for reason in ["length", "content_filter", "tool_calls", "stop"] {
        let status = if matches!(reason, "length" | "content_filter") {
            "incomplete"
        } else {
            "completed"
        };
        let response = json!({"id":"r","choices":[{"message":{"role":"assistant","content":"partial text","tool_calls":[{"id":"c","type":"function","function":{"name":"lookup","arguments":"{\"x\":"}},{"id":"custom","type":"custom","custom":{"name":"apply_patch","input":"freeform"}}]},"finish_reason":reason}]});
        let out = super::super::chat_to_response(&response, "m").unwrap();
        assert_eq!(out["status"], status);
        assert!(out["output"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["status"] == status));
        assert_eq!(out["output"][1]["arguments"], "{\"x\":");
        assert_eq!(out["output"][2]["input"], "freeform");
    }
}
