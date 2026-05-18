use serde_json::{json, Map, Value};
use std::collections::HashSet;

const REASONING_ENCRYPTED_CONTENT_INCLUDE: &str = "reasoning.encrypted_content";
const DEFAULT_CONTEXT_WINDOW: i64 = 272_000;
const DEFAULT_MAX_CONTEXT_WINDOW: i64 = 1_000_000;
const LOCAL_PROXY_BYPASS_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "::1"];

pub fn merge_local_no_proxy(raw: &str) -> String {
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for item in raw.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            items.push(trimmed.to_string());
        }
    }

    for host in LOCAL_PROXY_BYPASS_HOSTS {
        if seen.insert(host.to_ascii_lowercase()) {
            items.push(host.to_string());
        }
    }

    items.join(",")
}

pub fn is_codex_client_models_request(target: &str) -> bool {
    let Some(query) = target.split_once('?').map(|(_, query)| query) else {
        return false;
    };

    query.split('&').any(|pair| {
        pair.split_once('=')
            .map(|(key, _)| key)
            .unwrap_or(pair)
            .eq_ignore_ascii_case("client_version")
    })
}

pub fn build_codex_client_models_response(model_ids: &[String]) -> Value {
    let models = model_ids
        .iter()
        .map(|model_id| build_codex_client_model(model_id))
        .collect::<Vec<_>>();

    json!({ "models": models })
}

pub fn normalize_responses_body_for_codex(body: &mut Value) -> bool {
    let Some(obj) = body.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    changed |= ensure_string_field(obj, "instructions", "");
    changed |= ensure_bool_field(obj, "stream", true);
    changed |= ensure_bool_field(obj, "store", false);
    changed |= ensure_bool_field(obj, "parallel_tool_calls", true);
    changed |= ensure_reasoning_include(obj);
    changed |= normalize_responses_input(obj);
    changed |= normalize_codex_builtin_tools(obj);
    changed |= remove_unsupported_responses_fields(obj);

    changed
}

fn build_codex_client_model(model_id: &str) -> Value {
    let display_name = display_name_for_model(model_id);
    let visibility = if matches!(
        model_id,
        "gpt-image-2" | "grok-imagine-image" | "grok-imagine-video" | "grok-imagine-image-quality"
    ) {
        "hide"
    } else {
        "show"
    };

    json!({
        "slug": model_id,
        "display_name": display_name.clone(),
        "description": display_name,
        "context_window": DEFAULT_CONTEXT_WINDOW,
        "max_context_window": DEFAULT_MAX_CONTEXT_WINDOW,
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": reasoning_levels(),
        "prefer_websockets": false,
        "visibility": visibility,
    })
}

fn display_name_for_model(model_id: &str) -> String {
    match model_id {
        "gpt-5-codex" => "GPT-5 Codex".to_string(),
        "gpt-5-codex-mini" => "GPT-5 Codex Mini".to_string(),
        "gpt-5.4" => "GPT-5.4".to_string(),
        "gpt-5.4-mini" => "GPT-5.4 Mini".to_string(),
        "gpt-5.3-codex" => "GPT-5.3 Codex".to_string(),
        "gpt-5.3-codex-spark" => "GPT-5.3 Codex Spark".to_string(),
        "gpt-5.2" => "GPT-5.2".to_string(),
        "gpt-5.2-codex" => "GPT-5.2 Codex".to_string(),
        "gpt-5.1-codex-max" => "GPT-5.1 Codex Max".to_string(),
        "gpt-5.1-codex-mini" => "GPT-5.1 Codex Mini".to_string(),
        "gpt-image-2" => "GPT Image 2".to_string(),
        other => other.to_string(),
    }
}

fn reasoning_levels() -> Value {
    json!([
        {
            "effort": "minimal",
            "description": "Fastest responses with minimal reasoning",
        },
        {
            "effort": "low",
            "description": "Fast responses with lighter reasoning",
        },
        {
            "effort": "medium",
            "description": "Balances speed and reasoning depth for everyday tasks",
        },
        {
            "effort": "high",
            "description": "Greater reasoning depth for complex problems",
        },
        {
            "effort": "xhigh",
            "description": "Extra high reasoning depth for complex problems",
        },
    ])
}

fn ensure_string_field(obj: &mut Map<String, Value>, key: &str, value: &str) -> bool {
    if obj.get(key).and_then(Value::as_str) == Some(value) {
        return false;
    }
    if obj.get(key).is_some_and(Value::is_string) {
        return false;
    }
    obj.insert(key.to_string(), Value::String(value.to_string()));
    true
}

fn ensure_bool_field(obj: &mut Map<String, Value>, key: &str, value: bool) -> bool {
    if obj.get(key).and_then(Value::as_bool) == Some(value) {
        return false;
    }
    obj.insert(key.to_string(), Value::Bool(value));
    true
}

fn ensure_reasoning_include(obj: &mut Map<String, Value>) -> bool {
    match obj.get_mut("include") {
        Some(Value::Array(items)) => {
            if items
                .iter()
                .any(|item| item.as_str() == Some(REASONING_ENCRYPTED_CONTENT_INCLUDE))
            {
                false
            } else {
                items.push(Value::String(
                    REASONING_ENCRYPTED_CONTENT_INCLUDE.to_string(),
                ));
                true
            }
        }
        _ => {
            obj.insert(
                "include".to_string(),
                Value::Array(vec![Value::String(
                    REASONING_ENCRYPTED_CONTENT_INCLUDE.to_string(),
                )]),
            );
            true
        }
    }
}

fn normalize_responses_input(obj: &mut Map<String, Value>) -> bool {
    let Some(input) = obj.get_mut("input") else {
        return false;
    };

    match input {
        Value::String(text) => {
            let text = text.clone();
            *input = Value::Array(vec![message_item("user", &text)]);
            true
        }
        Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= normalize_responses_input_item(item);
            }
            changed
        }
        Value::Object(_) => {
            let mut item = input.clone();
            normalize_responses_input_item(&mut item);
            *input = Value::Array(vec![item]);
            true
        }
        _ => false,
    }
}

fn normalize_responses_input_item(item: &mut Value) -> bool {
    let Some(obj) = item.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_ascii_lowercase();

    if role == "system" {
        obj.insert("role".to_string(), Value::String("developer".to_string()));
        changed = true;
    }

    if !obj.contains_key("type") && (obj.contains_key("role") || obj.contains_key("content")) {
        obj.insert("type".to_string(), Value::String("message".to_string()));
        changed = true;
    }

    let normalized_role = obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_ascii_lowercase();
    if let Some(content) = obj.get_mut("content") {
        changed |= normalize_message_content(content, &normalized_role);
    }

    changed
}

fn normalize_message_content(content: &mut Value, role: &str) -> bool {
    match content {
        Value::String(text) => {
            let text = text.clone();
            *content = Value::Array(vec![text_part(role, &text)]);
            true
        }
        Value::Array(parts) => {
            let mut changed = false;
            for part in parts {
                changed |= normalize_content_part(part, role);
            }
            changed
        }
        _ => false,
    }
}

fn normalize_content_part(part: &mut Value, role: &str) -> bool {
    let Some(obj) = part.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    if !obj.contains_key("text") {
        if let Some(text) = obj
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            obj.insert("text".to_string(), Value::String(text));
            changed = true;
        }
    }

    let desired_type = response_text_type_for_role(role);
    match obj.get("type").and_then(Value::as_str) {
        Some("text") | None => {
            if obj.contains_key("text") {
                obj.insert("type".to_string(), Value::String(desired_type.to_string()));
                changed = true;
            }
        }
        Some("input_text") if role == "assistant" => {
            obj.insert("type".to_string(), Value::String("output_text".to_string()));
            changed = true;
        }
        Some("output_text") if role != "assistant" => {
            obj.insert("type".to_string(), Value::String("input_text".to_string()));
            changed = true;
        }
        _ => {}
    }

    changed
}

fn message_item(role: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [text_part(role, text)],
    })
}

fn text_part(role: &str, text: &str) -> Value {
    json!({
        "type": response_text_type_for_role(role),
        "text": text,
    })
}

fn response_text_type_for_role(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("assistant") {
        "output_text"
    } else {
        "input_text"
    }
}

fn normalize_codex_builtin_tools(obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;

    if let Some(Value::Array(tools)) = obj.get_mut("tools") {
        for tool in tools {
            changed |= normalize_builtin_tool_value(tool);
        }
    }

    if let Some(tool_choice) = obj.get_mut("tool_choice") {
        changed |= normalize_builtin_tool_value(tool_choice);
        if let Some(Value::Array(tools)) = tool_choice.get_mut("tools") {
            for tool in tools {
                changed |= normalize_builtin_tool_value(tool);
            }
        }
    }

    changed
}

fn normalize_builtin_tool_value(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let Some(tool_type) = obj.get("type").and_then(Value::as_str) else {
        return false;
    };
    let normalized = match tool_type {
        "web_search_preview" | "web_search_preview_2025_03_11" => "web_search",
        _ => return false,
    };

    obj.insert("type".to_string(), Value::String(normalized.to_string()));
    true
}

fn remove_unsupported_responses_fields(obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    for key in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "truncation",
        "context_management",
        "user",
        "prompt_cache_retention",
        "safety_identifier",
        "stream_options",
    ] {
        changed |= obj.remove(key).is_some();
    }

    if obj.get("service_tier").is_some()
        && obj.get("service_tier").and_then(Value::as_str) != Some("priority")
    {
        obj.remove("service_tier");
        changed = true;
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_local_no_proxy_hosts() {
        assert_eq!(
            merge_local_no_proxy("example.com, localhost"),
            "example.com,localhost,127.0.0.1,::1"
        );
        assert_eq!(merge_local_no_proxy(""), "127.0.0.1,localhost,::1");
    }

    #[test]
    fn normalizes_string_input_for_codex_responses() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": "pong",
            "stream": false,
            "store": true,
            "temperature": 0.1,
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(body.get("store").and_then(Value::as_bool), Some(false));
        assert_eq!(body.get("instructions").and_then(Value::as_str), Some(""));
        assert!(body.get("temperature").is_none());
        assert_eq!(
            body.pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            body.pointer("/input/0/content/0/text")
                .and_then(Value::as_str),
            Some("pong")
        );
    }

    #[test]
    fn normalizes_system_role_and_builtin_tool_aliases() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": [{
                "type": "message",
                "role": "system",
                "content": "be concise"
            }],
            "tools": [{"type": "web_search_preview"}],
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.pointer("/input/0/role").and_then(Value::as_str),
            Some("developer")
        );
        assert_eq!(
            body.pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("web_search")
        );
    }

    #[test]
    fn codex_client_models_use_models_field_only() {
        let response = build_codex_client_models_response(&["gpt-5.4".to_string()]);
        assert!(response.get("models").and_then(Value::as_array).is_some());
        assert!(response.get("object").is_none());
        assert!(response.get("data").is_none());
        assert_eq!(
            response.pointer("/models/0/slug").and_then(Value::as_str),
            Some("gpt-5.4")
        );
    }
}
