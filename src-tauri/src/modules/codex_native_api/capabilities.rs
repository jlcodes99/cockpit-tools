//! Request feature inspection and conservative, provider-independent replay gates.
//! Inspection never mutates client schemas, arguments, history or tool choices.
use super::contract::{Route, ServiceError, WireApi};
use axum::http::HeaderMap;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(super) const MAX_REPLACEMENTS: usize = 4;
pub(super) const IDS: &[&str] = &[
    "function",
    "custom",
    "namespace",
    "tool_search_client",
    "tool_search",
    "web_search",
    "image_generation",
    "file_search",
    "code_interpreter",
    "computer_use",
    "mcp",
    "programmatic_tool_calling",
    "image_input",
    "file_input",
    "audio_input",
    "video_input",
    "compact",
];

fn tool_id(tool: &Value) -> &str {
    match tool["type"].as_str().unwrap_or("") {
        "function" | "function_call" | "function_call_output" => "function",
        "custom" | "custom_tool_call" | "custom_tool_call_output" => "custom",
        "namespace" => "namespace",
        "tool_search" | "tool_search_call" | "tool_search_output" => {
            if tool["execution"] == "client" {
                "tool_search_client"
            } else {
                "tool_search"
            }
        }
        "web_search"
        | "web_search_preview"
        | "web_search_preview_2025_03_11"
        | "web_search_call" => "web_search",
        "image_generation" | "image_generation_call" => "image_generation",
        "file_search" | "file_search_call" => "file_search",
        "code_interpreter" | "code_interpreter_call" => "code_interpreter",
        "computer" | "computer_use" | "computer_use_preview" | "computer_call" => "computer_use",
        "mcp"
        | "mcp_call"
        | "mcp_list_tools"
        | "mcp_approval_request"
        | "mcp_approval_response" => "mcp",
        "programmatic_tool_calling" => "programmatic_tool_calling",
        _ => "unknown_tool",
    }
}

pub(super) fn collect(body: &Value, compact: bool) -> Vec<String> {
    fn tools(value: Option<&Value>, ids: &mut BTreeSet<String>, depth: usize) {
        let Some(value) = value else { return };
        if depth > 16 {
            ids.insert("unknown_tool".into());
            return;
        }
        let Some(values) = value.as_array() else {
            ids.insert("unknown_tool".into());
            return;
        };
        for tool in values {
            ids.insert(tool_id(tool).into());
            if tool["allowed_callers"]
                .as_array()
                .is_some_and(|v| v.iter().any(|v| v == "programmatic"))
            {
                ids.insert("programmatic_tool_calling".into());
            }
            if tool["type"] == "namespace" {
                tools(tool.get("tools"), ids, depth + 1);
            }
            if tool
                .get("namespace")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
            {
                ids.insert("namespace".into());
            }
        }
    }
    fn choice(
        value: Option<&Value>,
        ids: &mut BTreeSet<String>,
        depth: usize,
        client_search: bool,
    ) {
        let Some(value) = value.filter(|v| v.is_object()) else {
            return;
        };
        if depth > 16 {
            ids.insert("unknown_tool".into());
            return;
        }
        if value["type"] == "allowed_tools" {
            if let Some(values) = value["tools"].as_array() {
                for value in values {
                    choice(Some(value), ids, depth + 1, client_search);
                }
            }
        } else {
            ids.insert(
                if value["type"] == "tool_search"
                    && value.get("execution").is_none()
                    && client_search
                {
                    "tool_search_client"
                } else {
                    tool_id(value)
                }
                .into(),
            );
        }
    }
    fn item(value: &Value, ids: &mut BTreeSet<String>, depth: usize) {
        if depth > 32 {
            ids.insert("unknown_tool".into());
            return;
        }
        let kind = value["type"].as_str().unwrap_or("");
        match kind {
            "input_image" | "image_url" => {
                ids.insert("image_input".into());
            }
            "input_file" => {
                ids.insert("file_input".into());
            }
            "input_audio" | "audio" | "audio_url" => {
                ids.insert("audio_input".into());
            }
            "input_video" | "video" | "video_url" => {
                ids.insert("video_input".into());
            }
            "additional_tools" => tools(value.get("tools"), ids, 0),
            "tool_search_call" | "tool_search_output" => {
                ids.insert(tool_id(value).into());
                tools(value.get("tools"), ids, 0);
                if kind == "tool_search_output" {
                    let decoded = value
                        .get("output")
                        .and_then(Value::as_str)
                        .and_then(|v| serde_json::from_str::<Value>(v).ok());
                    if let Some(output) = decoded.as_ref().or_else(|| value.get("output")) {
                        tools(
                            if output.is_array() {
                                Some(output)
                            } else {
                                output.get("tools")
                            },
                            ids,
                            0,
                        );
                    }
                }
            }
            kind if kind.ends_with("_call")
                || kind.ends_with("_call_output")
                || kind.starts_with("mcp_") =>
            {
                ids.insert(tool_id(value).into());
                if value
                    .get("namespace")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty())
                {
                    ids.insert("namespace".into());
                }
            }
            _ => {}
        }
        for key in ["content", "output"] {
            if let Some(parts) = value.get(key).and_then(Value::as_array) {
                for part in parts {
                    item(part, ids, depth + 1);
                }
            }
        }
    }
    let mut ids = BTreeSet::new();
    tools(body.get("tools").filter(|v| !v.is_null()), &mut ids, 0);
    choice(
        body.get("tool_choice"),
        &mut ids,
        0,
        super::chatgpt_plan_tools::has_client_declaration(body),
    );
    if let Some(input) = body["input"].as_array() {
        for value in input {
            item(value, &mut ids, 0);
        }
    }
    if compact {
        ids.insert("compact".into());
    }
    ids.into_iter().collect()
}

pub(super) fn failure(code: &'static str, message: &str) -> ServiceError {
    ServiceError::new(409, code, message)
}

pub(super) fn validate_replay(headers: &HeaderMap, body: &Value) -> Result<(), ServiceError> {
    let unsafe_history = || {
        failure("capability_fallback_history_unsafe", "Capability fallback requires complete portable history with matching tool calls and results; existing history was preserved")
    };
    if headers.contains_key("x-codex-turn-state")
        || headers.contains_key("conversation_id")
        || ["previous_response_id", "conversation", "conversation_id"]
            .iter()
            .any(|key| body.get(key).is_some_and(|v| !v.is_null()))
        || !super::identity::rebuild::replay_tools(body.get("tools"))
    {
        return Err(unsafe_history());
    }
    let input = body["input"].as_array().ok_or_else(unsafe_history)?;
    if !super::identity::rebuild::replay_items(input)
        || !super::identity::rebuild::paired_tools(input)
        || !paired_searches(input)
    {
        return Err(unsafe_history());
    }
    Ok(())
}

fn paired_searches(input: &[Value]) -> bool {
    let mut pending = HashMap::new();
    let mut seen = HashSet::new();
    for item in input {
        if !matches!(
            item["type"].as_str(),
            Some("tool_search_call" | "tool_search_output")
        ) {
            continue;
        }
        if item["execution"] != "client" {
            return false;
        }
        let Some(id) = item["call_id"].as_str().filter(|id| !id.is_empty()) else {
            return false;
        };
        if item["type"] == "tool_search_call" {
            if !seen.insert(id) || pending.insert(id, ()).is_some() {
                return false;
            }
        } else if pending.remove(id).is_none() {
            return false;
        }
    }
    pending.is_empty()
}

pub(super) fn encrypted(value: &Value) -> bool {
    if super::agent_communication::encrypted_message(value).is_some() {
        return true;
    }
    match value {
        Value::Object(values) => values.iter().any(|(key, value)| {
            if matches!(key.as_str(), "parameters" | "input_schema" | "schema") {
                return false;
            }
            (matches!(key.as_str(), "encrypted_content" | "signature") && !value.is_null())
                || (key == "encrypted_function_args"
                    && !value.is_null()
                    && value.as_array().is_none_or(|v| !v.is_empty()))
                || encrypted(value)
        }),
        Value::Array(values) => values.iter().any(encrypted),
        _ => false,
    }
}

pub(super) fn encrypted_compatible(source: &Route, target: &Route, body: &Value) -> bool {
    fn family(route: &Route) -> Option<u8> {
        if route.wire_api != WireApi::Responses {
            return None;
        }
        let url = reqwest::Url::parse(&route.base_url).ok()?;
        if url.scheme() != "https"
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
        {
            return None;
        }
        match (url.host_str()?, url.path().trim_end_matches('/')) {
            ("api.openai.com", "/v1") => Some(1),
            ("chatgpt.com", "/backend-api/codex") => Some(2),
            _ => None,
        }
    }
    !encrypted(body)
        || (source.owner_id == target.owner_id
            && family(source).is_some()
            && family(source) == family(target))
}

/// Only an explicit structured schema rejection before response creation is
/// eligible. Permission/runtime/network failures and arbitrary 400s never are.
pub(super) fn pre_execution_rejection(status: u16, value: &Value) -> bool {
    let Some(error) = value.get("error").filter(|v| v.is_object()) else {
        return false;
    };
    status == 400
        && value.get("type").is_none_or(|v| v == "error")
        && error
            .get("type")
            .is_none_or(|v| v == "invalid_request_error")
        && error.get("execution_started").is_none_or(|v| v == false)
        && error.get("executed").is_none_or(|v| v == false)
        && matches!(
            error["code"].as_str(),
            Some(
                "unsupported_tool"
                    | "tool_not_supported"
                    | "unsupported_capability"
                    | "subscription_sharing_unsupported_capability"
            )
        )
        && error["param"].as_str().is_some_and(|v| {
            v == "tools"
                || v.starts_with("tools[")
                || v == "tool_choice"
                || v.starts_with("tool_choice.")
                || (v.starts_with("input[") && v.contains(".tools"))
        })
        && ["response", "output", "choices", "usage", "id"]
            .iter()
            .all(|key| value.get(key).is_none_or(Value::is_null))
        && ["output", "result", "usage"]
            .iter()
            .all(|key| error.get(key).is_none_or(Value::is_null))
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
