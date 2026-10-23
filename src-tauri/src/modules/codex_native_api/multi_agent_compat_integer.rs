//! CPA v8 tool-schema integer normalization for non-Codex target executors.
//! This UA gate intentionally differs from multi-agent optimization eligibility.
use axum::http::HeaderMap;
use serde_json::Value;

pub(crate) fn normalize_request(headers: &HeaderMap, body: &mut Value) {
    let codex = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().to_ascii_lowercase().contains("codex"));
    if !codex {
        return;
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        normalize_tools(tools, "");
    }
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item["type"] == "additional_tools" {
                if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                    normalize_tools(tools, "");
                }
            }
        }
    }
}

fn normalize_tools(tools: &mut [Value], namespace: &str) {
    for tool in tools {
        normalize_tool(tool, namespace);
    }
}

fn normalize_tool(tool: &mut Value, namespace: &str) {
    if tool["type"] == "namespace" {
        if !namespace.is_empty() {
            return;
        }
        let name = tool["name"].as_str().unwrap_or("").to_owned();
        if name.is_empty() {
            return;
        }
        if let Some(children) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            normalize_tools(children, &name);
        }
        return;
    }
    for key in ["function_declarations", "functionDeclarations"] {
        if let Some(declarations) = tool.get_mut(key).and_then(Value::as_array_mut) {
            normalize_tools(declarations, namespace);
            return;
        }
    }
    let mut name = tool["name"].as_str().unwrap_or("").to_owned();
    let path = [
        "/parameters",
        "/function/parameters",
        "/input_schema",
        "/parametersJsonSchema",
    ]
    .into_iter()
    .find(|path| tool.pointer(path).is_some_and(Value::is_object));
    let Some(path) = path else {
        return;
    };
    if path == "/function/parameters" && name.is_empty() {
        name = tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
    }
    if !namespace.is_empty() {
        name = format!("{namespace}__{name}");
    }
    super::integer_fields::normalize_cpa(tool.pointer_mut(path).unwrap(), &name);
}

#[cfg(test)]
#[path = "multi_agent_compat_integer_tests.rs"]
mod tests;
