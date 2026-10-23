//! Codex Desktop can inject these host results without a model-issued call.
//! Recognize the envelope exactly; replay validation still checks its payload.
use serde_json::Value;

pub(super) fn is_host_result(item: &Value) -> bool {
    item["type"] == "function_call_output"
        && item["namespace"] == "codex_app"
        && matches!(
            item["name"].as_str(),
            Some("create_thread" | "send_message_to_thread")
        )
}

pub(super) fn is_standalone(item: &Value) -> bool {
    is_host_result(item)
        && match item.get("call_id") {
            None => true,
            Some(Value::String(id)) => id.is_empty(),
            // Explicit malformed or stale IDs must follow normal pairing rules.
            _ => false,
        }
}
