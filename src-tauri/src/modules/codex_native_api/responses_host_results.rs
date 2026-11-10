//! Codex clients can inject these host results without a model-issued call.
//! Recognize the envelope exactly; replay validation still checks its payload.
use serde_json::Value;

pub(super) fn is_host_result(item: &Value) -> bool {
    if item["type"] != "function_call_output" {
        return false;
    }
    match (item["namespace"].as_str(), item["name"].as_str()) {
        (Some("codex_app" | "codex_tui"), Some("create_thread" | "send_message_to_thread")) => true,
        // Companion Page snapshots are injected by the client in the agent
        // namespace. They have no preceding call and must not consume a pending
        // tool result or prevent a complete history from rebuilding its owner.
        (Some("agent"), Some(name)) => is_page_context(name),
        _ => false,
    }
}

fn is_page_context(name: &str) -> bool {
    if name == "codex_apps_open_page" {
        return true;
    }
    let Some((part, total)) = name
        .strip_prefix("codex_apps_selected_page_part_")
        .and_then(|suffix| suffix.split_once("_of_"))
    else {
        return false;
    };
    let (Ok(index), Ok(count)) = (part.parse::<usize>(), total.parse::<usize>()) else {
        return false;
    };
    index > 0 && index <= count && index.to_string() == part && count.to_string() == total
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
