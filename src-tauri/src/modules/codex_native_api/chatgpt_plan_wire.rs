//! Public Responses contract for independently authorized ChatGPT plan usage.
//! Keep this separate from Codex backend compatibility and arbitrary API keys.
use axum::http::HeaderMap;
use serde_json::{json, Value};

use super::contract::ServiceError;

fn unsupported(message: impl Into<String>) -> ServiceError {
    ServiceError::new(400, "subscription_sharing_unsupported_capability", message)
}

/// Official structured error codes may arrive only after SSE has begun and
/// carry no numeric HTTP status. Preserve their documented recovery semantics.
pub(super) fn error_status(code: &str) -> Option<u16> {
    match code {
        "subscription_sharing_usage_limit_exceeded" => Some(429),
        "subscription_sharing_usage_unavailable" | "subscription_sharing_user_unavailable" => {
            Some(503)
        }
        "subscription_sharing_invalid_user" => Some(401),
        "subscription_sharing_user_not_eligible"
        | "subscription_sharing_route_not_supported"
        | "chatpass_v2_scope_not_authorized"
        | "chatpass_v2_invalid_authorization_context" => Some(403),
        "subscription_sharing_unsupported_capability" => Some(400),
        _ => None,
    }
}

pub(super) fn validate_event(event: &Value) -> Result<(), ServiceError> {
    if matches!(
        super::events::response_terminal_kind(event),
        Some("response.completed" | "response.done")
    ) && event["type"] != "response.completed"
    {
        return Err(ServiceError::new(
            502,
            "incomplete_upstream_stream",
            "ChatGPT plan inference requires a response.completed event",
        ));
    }
    Ok(())
}

/// Plan usage has no public speed override. Adapt only the per-route copy;
/// the logical source remains unchanged for a later local fallback.
pub(super) fn adapt_service_tier(body: &mut Value) {
    if body.get("service_tier").and_then(Value::as_str)
        .is_some_and(|tier| !matches!(tier, "" | "auto" | "default"))
    {
        body.as_object_mut().expect("field implies object").remove("service_tier");
    }
}

/// Validate the original request before host policies can remove tools. HTTP
/// continuation IDs are rejected explicitly; discarding one would lose history.
pub(super) fn validate(body: &Value, compact: bool) -> Result<(), ServiceError> {
    validate_before_transport(body, compact, false)
}

pub(super) fn validate_before_transport(
    body: &Value,
    compact: bool,
    websocket: bool,
) -> Result<(), ServiceError> {
    if compact {
        return Err(unsupported(
            "ChatGPT plan usage does not support Responses compaction",
        ));
    }
    if !body.is_object() {
        return Err(ServiceError::new(
            400,
            "invalid_json",
            "Request body must be a JSON object",
        ));
    }
    super::chatgpt_plan_request::validate_configuration_updates(body)?;
    if !websocket
        && body
            .get("previous_response_id")
            .is_some_and(|value| !value.is_null())
    {
        return Err(unsupported(
            "ChatGPT plan HTTP requests require full history in input; omit previous_response_id",
        ));
    }
    if let Some(tier) = body.get("service_tier").and_then(Value::as_str) {
        if !matches!(tier, "" | "auto" | "default") {
            return Err(unsupported(
                "ChatGPT plan usage does not support a service-tier override in this version",
            ));
        }
    }
    validate_tools_at(body.get("tools"), "tools", 0)?;
    validate_choice(
        body.get("tool_choice"),
        "tool_choice",
        super::chatgpt_plan_tools::has_client_declaration(body),
    )?;
    if let Some(input) = body.get("input").and_then(Value::as_array) {
        for (index, item) in input.iter().enumerate() {
            let path = format!("input[{index}]");
            if item["type"] == "additional_tools" {
                validate_tools_at(item.get("tools"), &format!("{path}.tools"), 0)?;
            }
            if matches!(
                item["type"].as_str(),
                Some("tool_search_call" | "tool_search_output")
            ) {
                if item["execution"] != "client" {
                    return Err(unsupported_tool(item, &path));
                }
                validate_tools_at(item.get("tools"), &format!("{path}.tools"), 0)?;
                if item["type"] == "tool_search_output" {
                    let decoded = item
                        .get("output")
                        .and_then(Value::as_str)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
                    if let Some(output) = decoded.as_ref().or_else(|| item.get("output")) {
                        let tools = if output.is_array() {
                            Some(output)
                        } else {
                            output.get("tools")
                        };
                        validate_tools_at(tools, &format!("{path}.output"), 0)?;
                    }
                }
            }
            validate_input(item)?;
        }
    }
    Ok(())
}

pub(super) fn unsupported_tool(tool: &Value, path: &str) -> ServiceError {
    let kind = tool["type"]
        .as_str()
        .filter(|kind| {
            kind.len() <= 64
                && kind
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
        .unwrap_or("unknown");
    let message = format!("Unsupported ChatGPT plan tool type '{kind}' at {path}");
    unsupported(&message).with_error_details(json!({"code":"subscription_sharing_unsupported_capability","message":message,"param":path}))
}

pub(super) fn validate_tools_at(
    value: Option<&Value>,
    path: &str,
    depth: usize,
) -> Result<(), ServiceError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(());
    };
    if depth > 16 {
        return Err(unsupported("Tool namespaces exceed their nesting limit"));
    }
    let tools = value
        .as_array()
        .ok_or_else(|| unsupported("tools must be an array"))?;
    for (index, tool) in tools.iter().enumerate() {
        let path = format!("{path}[{index}]");
        match tool["type"].as_str() {
            Some("function" | "custom") => {
                if tool
                    .get("allowed_callers")
                    .and_then(Value::as_array)
                    .is_some_and(|callers| callers.iter().any(|caller| caller == "programmatic"))
                {
                    return Err(unsupported_tool(tool, &format!("{path}.allowed_callers")));
                }
            }
            Some("namespace") => {
                if tool["name"].as_str().is_none_or(str::is_empty) || !tool["tools"].is_array() {
                    return Err(unsupported_tool(tool, &path));
                }
                validate_tools_at(tool.get("tools"), &format!("{path}.tools"), depth + 1)?;
            }
            Some("tool_search")
                if depth == 0 && super::chatgpt_plan_tools::client_declaration(tool) => {}
            Some("web_search" | "web_search_preview" | "web_search_preview_2025_03_11")
                if depth == 0 => {}
            _ => return Err(unsupported_tool(tool, &path)),
        }
    }
    Ok(())
}

fn validate_choice(
    value: Option<&Value>,
    path: &str,
    client_search_declared: bool,
) -> Result<(), ServiceError> {
    let Some(value) = value.filter(|value| value.is_object()) else {
        return Ok(());
    };
    match value["type"].as_str() {
        Some(
            "function"
            | "custom"
            | "web_search"
            | "web_search_preview"
            | "web_search_preview_2025_03_11",
        ) => Ok(()),
        Some("tool_search")
            if client_search_declared
                && matches!(
                    value.get("execution").and_then(Value::as_str),
                    None | Some("client")
                ) =>
        {
            Ok(())
        }
        Some("allowed_tools") => {
            if let Some(tools) = value.get("tools").and_then(Value::as_array) {
                for (index, tool) in tools.iter().enumerate() {
                    validate_choice(
                        Some(tool),
                        &format!("{path}.tools[{index}]"),
                        client_search_declared,
                    )?;
                }
            }
            Ok(())
        }
        _ => Err(unsupported_tool(value, path)),
    }
}

fn validate_input(item: &Value) -> Result<(), ServiceError> {
    if matches!(
        item["type"].as_str(),
        Some("input_audio" | "audio" | "input_video" | "video")
    ) {
        return Err(unsupported(
            "ChatGPT plan usage does not support audio or video input",
        ));
    }
    if item["type"] == "input_file" && item.get("file_id").is_some() {
        return Err(unsupported("ChatGPT plan file inputs require inline data or a URL; uploaded file IDs are unsupported"));
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for part in content {
            validate_input(part)?;
        }
    }
    if let Some(output) = item.get("output").and_then(Value::as_array) {
        for part in output {
            validate_input(part)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn prepare(
    headers: &mut HeaderMap,
    body: &mut Value,
    compact: bool,
) -> Result<super::chatgpt_plan_tools::Context, ServiceError> {
    prepare_with_effort_override(headers, body, compact, false)
}

pub(super) fn prepare_with_effort_override(
    headers: &mut HeaderMap,
    body: &mut Value,
    compact: bool,
    keep_top_level_effort: bool,
) -> Result<super::chatgpt_plan_tools::Context, ServiceError> {
    validate(body, compact)?;
    let responses_lite = super::codex_wire::responses_lite(headers, body);
    super::chatgpt_plan_request::prepare_headers(headers);
    let object = body.as_object_mut().expect("object validated above");
    if let Some(Value::String(text)) = object.get("input") {
        object.insert("input".into(), json!([{"role":"user","content":text}]));
    }
    if object.get("input").is_none_or(Value::is_null) {
        object.insert("input".into(), json!([]));
    }
    if !object.get("input").is_some_and(Value::is_array) {
        return Err(ServiceError::new(
            400,
            "invalid_input",
            "ChatGPT plan input must be an array or text",
        ));
    }
    object.insert("store".into(), json!(false));
    object.insert("stream".into(), json!(true));
    for field in [
        "background",
        "conversation",
        "max_output_tokens",
        "max_tool_calls",
        "metadata",
        "moderation",
        "multi_agent",
        "prompt",
        "prompt_cache_retention",
        "safety_identifier",
        "temperature",
        "top_logprobs",
        "top_p",
        "truncation",
        "user",
        // Client/Codex extensions that have no public plan-route semantics.
        "max_completion_tokens",
        "prompt_cache_options",
        "context_management",
        "generate",
        "client_metadata",
        "stream_options",
        "service_tier",
        "previous_response_id",
    ] {
        object.remove(field);
    }
    super::chatgpt_plan_request::collapse_configuration_updates(body, keep_top_level_effort)?;
    // SIWC is a standard Responses endpoint. CPA model compatibility performs
    // this same projection; it is required here independently of its optional
    // namespace optimization. Source/checkpoint history remains untouched.
    super::multi_agent_compat::portable_input(body);
    super::responses_tool_history::normalize_empty_function_arguments(body);
    let context = super::chatgpt_plan_tools::prepare(body)?;
    // Tool discovery may have promoted additional declarations. Normalize the
    // final native function schemas before moving standalone tools to input.
    // Original capability validation above always runs before this helper.
    super::codex_wire::normalize_plan_tools(body, responses_lite);
    let object = body.as_object_mut().expect("validated object");
    let mut additional = Vec::new();
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        let mut retained = Vec::new();
        for mut tool in std::mem::take(tools) {
            match tool["type"].as_str() {
                Some("function" | "custom") => additional.push(tool),
                Some("web_search_preview" | "web_search_preview_2025_03_11") => {
                    tool["type"] = json!("web_search");
                    retained.push(tool);
                }
                _ => retained.push(tool),
            }
        }
        *tools = retained;
    }
    let input = object
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .unwrap();
    for item in input.iter_mut() {
        if item["role"] == "system" {
            item["role"] = json!("developer");
        }
        if let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) {
            for part in parts {
                if let Some(part) = part.as_object_mut() {
                    part.remove("prompt_cache_breakpoint");
                }
            }
        }
    }
    if !additional.is_empty() {
        // Public Responses AdditionalTools requires the declaring role; unlike
        // ordinary messages, this item supports only the developer role.
        input.push(json!({"type":"additional_tools","role":"developer","tools":additional}));
    }
    Ok(context)
}

#[cfg(test)]
#[path = "chatgpt_plan_protocol_parity_tests.rs"]
mod protocol_parity_tests;

#[cfg(test)]
fn normalize(headers: &mut HeaderMap, body: &mut Value, compact: bool) -> Result<(), ServiceError> {
    prepare(headers, body, compact).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_plan_errors_keep_documented_status_and_recovery_scope() {
        for (code, status, scoped) in [
            ("subscription_sharing_usage_limit_exceeded", 429, true),
            ("subscription_sharing_usage_unavailable", 503, false),
            ("subscription_sharing_user_not_eligible", 403, true),
            ("subscription_sharing_unsupported_capability", 400, false),
            ("subscription_sharing_invalid_user", 401, true),
        ] {
            let event = json!({"type":"response.failed","response":{"status":"failed","error":{"code":code,"param":"tools","message":"policy result"}}});
            let mut observation = super::super::events::ResponseObservation::default();
            observation.observe(&event);
            assert_eq!(observation.http_status, Some(status), "{code}");
            assert_eq!(observation.credential_scope, scoped, "{code}");
            let error = super::super::upstream_errors::chat_failure(&event, 502).unwrap();
            assert_eq!(error.status, status);
            assert_eq!(error.response_error.unwrap()["param"], "tools");
        }
    }

    #[test]
    fn plan_wire_uses_public_http_requirements_and_preserves_text() {
        let mut body = json!({"model":"account-model","input":[{"type":"message","role":"system","content":"system is user data"}],"stream":false,"store":true,"metadata":{"private":"x"},"multi_agent":true,"temperature":0.4,"top_p":0.8,"max_output_tokens":10,"client_metadata":{"session_id":"local"},"include":["reasoning.encrypted_content"]});
        let mut headers = HeaderMap::new();
        headers.insert(
            "openai-beta",
            "responses_websockets=2025-02-01".parse().unwrap(),
        );
        normalize(&mut headers, &mut body, false).unwrap();
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["content"], "system is user data");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        for field in [
            "metadata",
            "multi_agent",
            "temperature",
            "top_p",
            "max_output_tokens",
            "client_metadata",
        ] {
            assert!(body.get(field).is_none(), "{field}");
        }
        assert!(!headers.contains_key("openai-beta"));
    }

    #[test]
    fn standalone_local_tools_move_into_input_without_changing_names_or_schemas() {
        let tool = json!({"type":"function","name":"read_file","parameters":{"type":"object","properties":{"path":{"type":"string"}}}});
        let mut body = json!({"input":"hello","tools":[tool.clone(),{"type":"web_search_preview"}],"tool_choice":{"type":"function","name":"read_file"}});
        normalize(&mut HeaderMap::new(), &mut body, false).unwrap();
        assert_eq!(body["input"][0], json!({"role":"user","content":"hello"}));
        assert_eq!(
            body["input"][1],
            json!({"type":"additional_tools","role":"developer","tools":[tool]})
        );
        assert_eq!(body["tools"], json!([{"type":"web_search"}]));
        assert_eq!(body["tool_choice"]["name"], "read_file");
    }

    #[test]
    fn incoming_additional_tools_are_rebuilt_with_the_required_developer_role() {
        let tools = json!([
            {"type":"function","name":"shell_command","parameters":{"type":"object","properties":{"command":{"type":"string"}}}},
            {"type":"custom","name":"apply_patch","format":{"type":"text"}},
        ]);
        for role in [None, Some("developer")] {
            let mut item = json!({"type":"additional_tools","tools":tools});
            if let Some(role) = role {
                item["role"] = json!(role);
            }
            let mut body = json!({"input":[item]});
            normalize(&mut HeaderMap::new(), &mut body, false).unwrap();
            assert_eq!(body["tools"], json!([]));
            assert_eq!(
                body["input"],
                json!([{"type":"additional_tools","role":"developer","tools":tools}])
            );
        }
    }

    #[test]
    fn unsupported_capabilities_fail_instead_of_silently_losing_context_or_tools() {
        for body in [
            json!({"input":[],"previous_response_id":"remote-id"}),
            json!({"input":[],"tools":[{"type":"image_generation"}]}),
            json!({"input":[],"tools":[{"type":"namespace","tools":[{"type":"tool_search","execution":"client"}]}]}),
            json!({"input":[{"type":"additional_tools","tools":[{"type":"file_search"}]}]}),
            json!({"input":[{"role":"user","content":[{"type":"input_audio","data":"sample"}]}]}),
            json!({"input":[],"service_tier":"priority"}),
        ] {
            assert_eq!(
                validate(&body, false).unwrap_err().code,
                "subscription_sharing_unsupported_capability"
            );
        }
        assert!(validate(&json!({"input":[]}), true).is_err());
    }
}
