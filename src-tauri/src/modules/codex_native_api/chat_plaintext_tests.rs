//! Plaintext provenance belongs to the Chat boundary, never an inferred cipher format.
use super::*;
use crate::modules::codex_native_api::{agent_communication, codex_wire};
use axum::http::HeaderMap;

fn request(namespace: Option<&str>, name: &str, kind: &str) -> Value {
    let tool = json!({"type":kind,"name":name,"parameters":{"type":"object",
        "properties":{"message":{"type":"string","encrypted":true}}}});
    json!({"model":"fixture","input":[],"tools":match namespace {
        Some(namespace) => json!([{"type":"namespace","name":namespace,"tools":[tool]}]),
        None => json!([tool]),
    }})
}

#[test]
fn chat_plaintext_collaboration_is_explicit_in_json_and_all_stream_item_events() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        let request = request(Some("collaboration"), name, "function");
        let context = ToolContext::from_responses(&request);
        let arguments = "{\"message\":\"请检查代码，保留原始文本。\"}";
        let call = json!({"id":"call-1","type":"function","function":{
            "name":format!("collaboration__{name}"),"arguments":arguments}});
        let raw = json!({"choices":[{"message":{"role":"assistant","tool_calls":[call]},
            "finish_reason":"tool_calls"}]});
        let response = chat_to_response_with_context(&raw, "fixture", &context).unwrap();
        let item = &response["output"][0];
        assert_eq!(item["encrypted_function_args"], json!([]));
        assert_eq!(item["namespace"], "collaboration");
        assert_eq!(item["arguments"], arguments);
        assert!(agent_communication::encrypted_message(item).is_none());

        let mut adapter = ResponsesStreamAdapter::with_context("fixture".into(), context);
        let mut events = adapter
            .push(&json!({"choices":[{"index":0,"delta":{"tool_calls":[{
                "index":0,"id":"call-1","type":"function","function":{
                "name":format!("collaboration__{name}"),"arguments":arguments}}]},
                "finish_reason":"tool_calls"}]}))
            .unwrap();
        events.extend(adapter.finish_done().unwrap());
        let mut seen = Vec::new();
        for event in events {
            let kind = event["type"].as_str().unwrap();
            let item = match kind {
                "response.output_item.added" | "response.output_item.done" => &event["item"],
                "response.completed" => &event["response"]["output"][0],
                _ => continue,
            };
            assert_eq!(item["encrypted_function_args"], json!([]), "{name}: {kind}");
            assert_eq!(item["namespace"], "collaboration");
            assert_eq!(item["name"], name);
            if kind != "response.output_item.added" {
                assert_eq!(item["arguments"], arguments);
                assert!(agent_communication::encrypted_message(item).is_none());
            }
            seen.push(kind.to_owned());
        }
        assert_eq!(
            seen,
            [
                "response.output_item.added",
                "response.output_item.done",
                "response.completed"
            ]
        );
    }
}

#[test]
fn plaintext_marker_does_not_reclassify_other_tools_or_explicit_encryption() {
    for (namespace, name, kind) in [
        (Some("other"), "spawn_agent", "function"),
        (None, "send_message", "function"),
        (Some("collaboration"), "exec_command", "function"),
        (Some("collaboration"), "spawn_agent", "custom"),
    ] {
        let request = request(namespace, name, kind);
        let context = ToolContext::from_responses(&request);
        let qualified = tools::qualify(namespace.unwrap_or(""), name);
        let mut item = json!({"type":"function_call","name":qualified,
            "arguments":"{\"input\":\"task\",\"message\":\"plain text\"}"});
        context.restore_item(&mut item);
        assert!(item.get("encrypted_function_args").is_none());
    }
    for (field, value) in [
        ("encrypted_function_args", json!(["message"])),
        ("encrypted_content", json!("opaque-cipher")),
        ("signature", json!("opaque-signature")),
        ("encrypted_function_args", json!("invalid-marker")),
    ] {
        let context = ToolContext::from_responses(&request(
            Some("collaboration"),
            "send_message",
            "function",
        ));
        let mut item = json!({"type":"function_call","name":"collaboration__send_message",
            "arguments":"{\"message\":\"opaque-cipher\"}"});
        item[field] = value.clone();
        context.restore_item(&mut item);
        assert_eq!(item[field], value);
        if field != "encrypted_function_args" {
            assert!(item.get("encrypted_function_args").is_none());
        }
    }
}

#[test]
fn official_responses_keep_encryption_schema_and_do_not_repair_legacy_history() {
    for mode in ["http", "lite", "websocket", "compact"] {
        let mut body = request(Some("collaboration"), "send_message", "function");
        body["input"] = json!([
            {"type":"function_call","namespace":"collaboration","name":"send_message",
                "call_id":"call-1","arguments":"{\"message\":\"gAAAA-original-cipher\"}",
                "encrypted_function_args":["message"]},
            {"type":"agent_message","id":"amsg-original","author":"/root/worker",
                "recipient":"/root","content":[{"type":"encrypted_content",
                "encrypted_content":"gAAAA-original-cipher"}]},
            {"type":"agent_message","id":"amsg-legacy","author":"/root/worker",
                "recipient":"/root","content":[{"type":"encrypted_content",
                "encrypted_content":"旧历史中误标记的明文保持原样"}]}
        ]);
        let original = body.clone();
        let mut headers = HeaderMap::new();
        if mode == "lite" {
            headers.insert(codex_wire::LITE_HEADER, "true".parse().unwrap());
        }
        if mode == "websocket" {
            codex_wire::normalize_websocket(&mut headers, &mut body, true).unwrap();
        } else {
            codex_wire::normalize(&mut headers, &mut body, true, mode == "compact").unwrap();
        }
        assert_eq!(body["tools"], original["tools"], "{mode}");
        assert_eq!(body["input"], original["input"], "{mode}");
    }
}
