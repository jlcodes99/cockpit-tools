use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::OnceLock;

const REASONING_ENCRYPTED_CONTENT_INCLUDE: &str = "reasoning.encrypted_content";
const CODEX_AUTO_REVIEW_MODEL_ID: &str = "codex-auto-review";
const CODEX_MODEL_CATALOG_TEMPLATE_SLUG: &str = "gpt-5.5";
const CODEX_CLIENT_MODEL_TEMPLATES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../sidecars/cockpit-cliproxy/cdk/CLIProxyAPI/internal/registry/models/codex_client_models.json"
));
const DEFAULT_CONTEXT_WINDOW: i64 = 272_000;
const DEFAULT_MAX_CONTEXT_WINDOW: i64 = 1_000_000;
const LOCAL_PROXY_BYPASS_HOSTS: [&str; 5] =
    ["127.0.0.1", "127.0.0.0/8", "localhost", "::1", "::1/128"];

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
        .enumerate()
        .map(|(index, model_id)| build_codex_client_model(model_id, index))
        .collect::<Vec<_>>();

    json!({ "models": models })
}

pub(crate) fn managed_codex_model_ids() -> Vec<String> {
    codex_client_model_catalog()
        .get("model_overrides")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| model.get("slug").and_then(Value::as_str))
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn normalize_responses_body_for_codex(body: &mut Value) -> bool {
    normalize_responses_body_for_codex_with_lite(body, false)
}

pub fn normalize_responses_body_for_codex_with_lite(
    body: &mut Value,
    force_responses_lite: bool,
) -> bool {
    let responses_lite = force_responses_lite
        || body
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(codex_model_uses_responses_lite);
    let Some(obj) = body.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    changed |= ensure_string_field(obj, "instructions", "");
    changed |= ensure_bool_field(obj, "stream", true);
    changed |= ensure_bool_field(obj, "store", false);
    changed |= ensure_bool_field(obj, "parallel_tool_calls", !responses_lite);
    changed |= ensure_reasoning_include(obj);
    changed |= normalize_responses_input(obj);
    changed |= normalize_codex_builtin_tools(obj);
    if responses_lite {
        changed |= filter_responses_lite_tools_in_object(obj);
    }
    changed |= remove_unsupported_responses_fields(obj);

    changed
}

pub(crate) fn codex_model_uses_responses_lite(model_id: &str) -> bool {
    codex_client_model_catalog()
        .get("model_overrides")
        .and_then(Value::as_array)
        .is_some_and(|models| {
            models.iter().any(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id.trim()))
                    && model
                        .get("use_responses_lite")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
        })
}

pub(crate) fn filter_responses_lite_tools(body: &mut Value) -> bool {
    body.as_object_mut()
        .is_some_and(filter_responses_lite_tools_in_object)
}

fn responses_lite_tool_allowed(tool: &Value) -> bool {
    match tool
        .get("type")
        .and_then(Value::as_str)
        .map(|tool_type| tool_type.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("function" | "custom") => true,
        Some("tool_search") => tool
            .get("execution")
            .and_then(Value::as_str)
            .is_some_and(|execution| execution.trim().eq_ignore_ascii_case("client")),
        _ => false,
    }
}

fn filter_responses_lite_tool_array(value: &mut Value) -> (bool, bool) {
    let Some(tools) = value.as_array_mut() else {
        return (false, false);
    };
    let before = tools.len();
    tools.retain(responses_lite_tool_allowed);
    (tools.len() != before, !tools.is_empty())
}

fn filter_responses_lite_tool_choice(choice: &mut Value) -> (bool, bool) {
    if let Some(choice_name) = choice.as_str() {
        let valid = matches!(
            choice_name.trim().to_ascii_lowercase().as_str(),
            "auto" | "none" | "required"
        );
        return (false, valid);
    }

    let Some(choice_object) = choice.as_object_mut() else {
        return (false, false);
    };
    let choice_type = choice_object
        .get("type")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase());

    match choice_type.as_deref() {
        Some("function" | "custom") => return (false, true),
        Some("tool_search") => {
            let client_executed = choice_object
                .get("execution")
                .and_then(Value::as_str)
                .is_some_and(|execution| execution.trim().eq_ignore_ascii_case("client"));
            return (false, client_executed);
        }
        _ => {}
    }

    if choice_type.as_deref() != Some("allowed_tools") {
        return (false, false);
    }

    let mut changed = false;
    let mut has_allowed_tools = false;
    for key in ["tools", "allowed_tools"] {
        if let Some(value) = choice_object.get_mut(key) {
            let (value_changed, value_has_allowed_tools) = filter_responses_lite_tool_array(value);
            changed |= value_changed;
            has_allowed_tools |= value_has_allowed_tools;
        }
    }
    (changed, has_allowed_tools)
}

fn filter_responses_lite_tools_in_object(object: &mut Map<String, Value>) -> bool {
    let mut changed = false;

    if let Some(tools) = object.get_mut("tools") {
        changed |= filter_responses_lite_tool_array(tools).0;
    }

    let remove_tool_choice = object
        .get_mut("tool_choice")
        .map(|choice| {
            let (choice_changed, choice_valid) = filter_responses_lite_tool_choice(choice);
            changed |= choice_changed;
            !choice_valid
        })
        .unwrap_or(false);
    if remove_tool_choice {
        object.remove("tool_choice");
        changed = true;
    }

    if let Some(Value::Array(input)) = object.get_mut("input") {
        let before = input.len();
        input.retain_mut(|item| {
            let Some(item_object) = item.as_object_mut() else {
                return true;
            };
            if !item_object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|item_type| item_type.eq_ignore_ascii_case("additional_tools"))
            {
                return true;
            }
            changed |= filter_responses_lite_tools_in_object(item_object);
            item_object
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| !tools.is_empty())
        });
        changed |= input.len() != before;
    }

    if let Some(Value::Object(response)) = object.get_mut("response") {
        changed |= filter_responses_lite_tools_in_object(response);
    }

    changed
}

fn build_codex_client_model(model_id: &str, index: usize) -> Value {
    let visibility = if matches!(
        model_id,
        CODEX_AUTO_REVIEW_MODEL_ID
            | "gpt-image-2"
            | "grok-imagine-image"
            | "grok-imagine-video"
            | "grok-imagine-image-quality"
    ) {
        "hide"
    } else {
        "list"
    };

    let (mut model, is_catalog_model) = codex_client_model_template(model_id);
    let object = model
        .as_object_mut()
        .expect("Codex client model template should be a JSON object");
    object.insert("slug".to_string(), Value::String(model_id.to_string()));
    if !is_catalog_model {
        let display_name = display_name_for_model(model_id);
        object.insert(
            "display_name".to_string(),
            Value::String(display_name.clone()),
        );
        object.insert("description".to_string(), Value::String(display_name));
        object.insert("context_window".to_string(), json!(DEFAULT_CONTEXT_WINDOW));
        object.insert(
            "max_context_window".to_string(),
            json!(DEFAULT_MAX_CONTEXT_WINDOW),
        );
        object.insert("priority".to_string(), json!(1000 + index));
        object.insert(
            "additional_speed_tiers".to_string(),
            Value::Array(Vec::new()),
        );
        object.insert("service_tiers".to_string(), Value::Array(Vec::new()));
    }
    if visibility == "hide" || !object.contains_key("visibility") {
        object.insert(
            "visibility".to_string(),
            Value::String(visibility.to_string()),
        );
    }
    object.insert("supported_in_api".to_string(), Value::Bool(true));
    object.insert("availability_nux".to_string(), Value::Null);
    object.insert("upgrade".to_string(), Value::Null);
    model
}

fn codex_client_model_catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(CODEX_CLIENT_MODEL_TEMPLATES_JSON)
            .expect("Codex client model templates JSON should be valid")
    })
}

fn codex_client_model_template(model_id: &str) -> (Value, bool) {
    let payload = codex_client_model_catalog();
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .expect("Codex client model templates should include models");
    if let Some(model) = models.iter().find(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id))
    }) {
        return (model.clone(), true);
    }

    let default_model = models
        .iter()
        .find(|model| {
            model.get("slug").and_then(Value::as_str) == Some(CODEX_MODEL_CATALOG_TEMPLATE_SLUG)
        })
        .cloned()
        .expect("Codex client model templates should include gpt-5.5");
    let Some(model_override) = payload
        .get("model_overrides")
        .and_then(Value::as_array)
        .and_then(|overrides| {
            overrides.iter().find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id))
            })
        })
    else {
        return (default_model, false);
    };

    let mut model = default_model;
    let target = model
        .as_object_mut()
        .expect("Codex client model template should be a JSON object");
    for (key, value) in model_override
        .as_object()
        .expect("Codex client model override should be a JSON object")
    {
        target.insert(key.clone(), value.clone());
    }
    (model, true)
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
        CODEX_AUTO_REVIEW_MODEL_ID => "Codex Auto Review".to_string(),
        other => other.to_string(),
    }
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

    // Provider adapters use this extension to reconstruct namespaced tool calls,
    // but the official Codex Responses endpoint rejects it on replayed input items.
    let mut changed = obj.remove("namespace").is_some();
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
            "example.com,localhost,127.0.0.1,127.0.0.0/8,::1,::1/128"
        );
        assert_eq!(
            merge_local_no_proxy(""),
            "127.0.0.1,127.0.0.0/8,localhost,::1,::1/128"
        );
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
    fn disables_parallel_tool_calls_for_responses_lite_models() {
        let mut body = json!({
            "model": "gpt-5.6-luna",
            "input": "pong",
            "parallel_tool_calls": true,
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn responses_lite_header_forces_parallel_tool_calls_off() {
        let mut body = json!({
            "model": "custom-model",
            "input": "pong",
            "parallel_tool_calls": true,
        });

        normalize_responses_body_for_codex_with_lite(&mut body, true);
        assert_eq!(
            body.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn responses_lite_keeps_only_supported_tools_in_all_declaration_locations() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                {
                    "type": "additional_tools",
                    "tools": [
                        {"type": "function", "name": "additional_function"},
                        {"type": "custom", "name": "additional_custom"},
                        {"type": "tool_search", "execution": "client"},
                        {"type": "image_generation"},
                        {"type": "web_search"},
                        {"type": "namespace", "name": "mcp__additional"}
                    ]
                },
                {
                    "type": "additional_tools",
                    "tools": [{"type": "image_generation"}]
                },
                {"role": "user", "content": "hello"}
            ],
            "tools": [
                {"type": "function", "name": "lookup"},
                {"type": "custom", "name": "apply_patch"},
                {"type": "tool_search", "execution": "client"},
                {"type": "tool_search"},
                {"type": "tool_search", "execution": "server"},
                {"type": "image_generation"},
                {"type": "web_search"},
                {"type": "namespace", "name": "mcp__root"}
            ],
            "tool_choice": {
                "type": "allowed_tools",
                "mode": "auto",
                "tools": [
                    {"type": "function", "name": "lookup"},
                    {"type": "custom", "name": "apply_patch"},
                    {"type": "tool_search", "execution": "client"},
                    {"type": "image_generation"},
                    {"type": "web_search"},
                    {"type": "namespace", "name": "mcp__root"}
                ]
            },
            "response": {
                "tools": [
                    {"type": "function", "name": "nested_function"},
                    {"type": "image_generation"},
                    {"type": "web_search"}
                ],
                "tool_choice": {"type": "image_generation"}
            }
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        for pointer in ["/tools", "/tool_choice/tools", "/input/0/tools"] {
            assert_eq!(
                body.pointer(pointer)
                    .and_then(Value::as_array)
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                    }),
                Some(vec!["function", "custom", "tool_search"]),
                "unexpected tools at {pointer}"
            );
        }
        assert_eq!(
            body.pointer("/response/tools")
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                }),
            Some(vec!["function"])
        );
        assert!(body.pointer("/response/tool_choice").is_none());
        assert_eq!(
            body.get("input").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn non_lite_responses_keep_official_hosted_tools() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [
                {"type": "function", "name": "lookup"},
                {"type": "image_generation"},
                {"type": "web_search"},
                {"type": "namespace", "name": "mcp__root"}
            ],
            "tool_choice": {"type": "image_generation"}
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.get("tools").and_then(Value::as_array).map(Vec::len),
            Some(4)
        );
        assert_eq!(
            body.pointer("/tool_choice/type").and_then(Value::as_str),
            Some("image_generation")
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
    fn removes_provider_namespace_from_replayed_input_items() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "lookup",
                "namespace": "mcp__example",
                "arguments": "{}"
            }],
            "tools": [{
                "type": "namespace",
                "name": "mcp__example"
            }]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert!(body.pointer("/input/0/namespace").is_none());
        assert_eq!(
            body.pointer("/input/0/name").and_then(Value::as_str),
            Some("lookup")
        );
        assert_eq!(
            body.pointer("/input/0/call_id").and_then(Value::as_str),
            Some("call_1")
        );
        assert_eq!(
            body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("namespace")
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
        assert_eq!(
            response
                .pointer("/models/0/prefer_websockets")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            response
                .pointer("/models/0/shell_type")
                .and_then(Value::as_str),
            Some("shell_command")
        );
        assert_eq!(
            response
                .pointer("/models/0/supported_in_api")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(response
            .pointer("/models/0/input_modalities")
            .and_then(Value::as_array)
            .is_some());
    }

    #[test]
    fn codex_spark_compatibility_model_is_visible_with_a_safe_catalog_fallback() {
        let response = build_codex_client_models_response(&[
            "gpt-5.3-codex".to_string(),
            "gpt-5.3-codex-spark".to_string(),
        ]);
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");
        let spark = models
            .iter()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.3-codex-spark"))
            .expect("Spark should be visible to Codex clients");

        assert_eq!(
            spark.get("display_name").and_then(Value::as_str),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(
            spark.get("visibility").and_then(Value::as_str),
            Some("list")
        );
        assert_eq!(
            spark.get("supported_in_api").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn codex_5_6_models_preserve_official_reasoning_and_speed_capabilities() {
        assert_eq!(
            managed_codex_model_ids(),
            vec!["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]
        );
        let response = build_codex_client_models_response(&managed_codex_model_ids());
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");

        for (slug, default_effort, supports_ultra) in [
            ("gpt-5.6-sol", "low", true),
            ("gpt-5.6-terra", "medium", true),
            ("gpt-5.6-luna", "medium", false),
        ] {
            let model = models
                .iter()
                .find(|model| model.get("slug").and_then(Value::as_str) == Some(slug))
                .expect("5.6 model should exist");
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .expect("reasoning levels should exist")
                .iter()
                .filter_map(|level| level.get("effort").and_then(Value::as_str))
                .collect::<Vec<_>>();
            assert_eq!(
                model.get("default_reasoning_level").and_then(Value::as_str),
                Some(default_effort)
            );
            assert!(efforts.contains(&"max"));
            assert_eq!(efforts.contains(&"ultra"), supports_ultra);
            assert_eq!(
                model
                    .pointer("/additional_speed_tiers/0")
                    .and_then(Value::as_str),
                Some("fast")
            );
            assert_eq!(
                model.pointer("/service_tiers/0/id").and_then(Value::as_str),
                Some("priority")
            );
            assert_eq!(
                model.get("context_window").and_then(Value::as_i64),
                Some(372_000)
            );
        }
    }

    #[test]
    fn codex_default_model_priorities_follow_official_order() {
        let model_ids = [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
        ]
        .map(str::to_string);
        let response = build_codex_client_models_response(&model_ids);
        let priorities = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array")
            .iter()
            .map(|model| model.get("priority").and_then(Value::as_i64))
            .collect::<Vec<_>>();

        assert_eq!(
            priorities,
            vec![Some(1), Some(2), Some(3), Some(7), Some(16), Some(23)]
        );
    }

    #[test]
    fn unknown_codex_models_keep_conservative_catalog_fallback() {
        let response = build_codex_client_models_response(&["custom-model".to_string()]);
        assert_eq!(
            response
                .pointer("/models/0/additional_speed_tiers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            response
                .pointer("/models/0/service_tiers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }
}
