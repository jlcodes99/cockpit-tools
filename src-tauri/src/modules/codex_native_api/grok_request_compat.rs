//! xAI wire compatibility. Only known protocol fields and JSON schema nodes are rewritten.
use super::{invalid, text, ServiceError};
use serde_json::{json, Value};
use std::collections::HashSet;

use super::super::multi_agent_compat::integer_fields as integer_schema;

pub(super) fn normalize_search_aliases(body: &mut Value) {
    fn normalize(tool: &mut Value) {
        if matches!(
            text(tool, "type"),
            Some("web_search_preview" | "web_search_preview_2025_03_11")
        ) {
            tool["type"] = json!("web_search");
        }
        if let Some(tools) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            for tool in tools {
                normalize(tool);
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            normalize(tool);
        }
    }
    if let Some(choice) = body.get_mut("tool_choice") {
        normalize(choice);
    }
}

pub(super) fn normalize_web_search(tool: &mut Value) {
    if let Some(tool) = tool.as_object_mut() {
        for field in ["external_web_access", "search_context_size"] {
            tool.remove(field);
        }
    }
}

pub(super) fn normalize_choices(body: &mut Value) {
    if let Some(body) = body.as_object_mut() {
        // These OpenAI options are not accepted by the xAI Responses endpoint.
        // Continuation and opaque history remain owned by the host identity layer.
        for field in [
            "prompt_cache_retention",
            "safety_identifier",
            "stream_options",
            "stop",
        ] {
            body.remove(field);
        }
    }
    if let Some(kind @ ("web_search" | "image_generation")) = body
        .get("tool_choice")
        .and_then(|choice| text(choice, "type"))
        .map(str::to_owned)
        .as_deref()
    {
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            tools.retain(|tool| text(tool, "type") == Some(kind));
        }
        body["tool_choice"] = json!("required");
    }
    if !body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| !tools.is_empty())
    {
        if let Some(body) = body.as_object_mut() {
            for field in ["tools", "tool_choice", "parallel_tool_calls"] {
                body.remove(field);
            }
        }
    }
}

pub(super) fn normalize_function(
    tool: &mut Value,
    name: &str,
    namespace: Option<&str>,
) -> Result<(), ServiceError> {
    if tool.get("parameters").is_none_or(Value::is_null) {
        tool["parameters"] = json!({"type":"object","properties":{}});
    }
    let parameters = &mut tool["parameters"];
    if namespace == Some("collaboration")
        && matches!(name, "spawn_agent" | "send_message" | "followup_task")
    {
        if let Some(message) = parameters
            .pointer_mut("/properties/message")
            .and_then(Value::as_object_mut)
        {
            message.remove("encrypted");
        }
    }
    integer_schema::normalize(parameters, name, namespace);
    let source = parameters.clone();
    let mut expanded = false;
    let mut nodes = 0usize;
    inline_refs(
        parameters,
        &source,
        &mut HashSet::new(),
        0,
        &mut nodes,
        &mut expanded,
    )?;
    if expanded && !has_ref(parameters) {
        if let Some(parameters) = parameters.as_object_mut() {
            parameters.remove("$defs");
            parameters.remove("definitions");
        }
    }
    if parameters.get("type").and_then(Value::as_str) == Some("object") {
        for union in ["anyOf", "oneOf"] {
            if let Some(branches) = parameters.get_mut(union).and_then(Value::as_array_mut) {
                for branch in branches {
                    if branch.is_object()
                        && branch.get("type").is_none()
                        && branch.get("$ref").is_none()
                    {
                        branch["type"] = json!("object");
                    }
                }
            }
        }
    }
    let clean_name = name.strip_prefix("mcp__").unwrap_or(name);
    let clean_ns = namespace
        .unwrap_or("")
        .strip_prefix("mcp__")
        .unwrap_or(namespace.unwrap_or(""));
    let automation = (clean_name.eq_ignore_ascii_case("automation_update")
        && ["codex_app", "codex_apps"]
            .iter()
            .any(|ns| clean_ns.eq_ignore_ascii_case(ns)))
        || [
            "codex_app__automation_update",
            "codex_apps__automation_update",
        ]
        .iter()
        .any(|n| clean_name.eq_ignore_ascii_case(n));
    let unsupported_union = ["anyOf", "oneOf"].iter().any(|key| {
        parameters
            .get(*key)
            .and_then(Value::as_array)
            .is_some_and(|branches| {
                branches
                    .iter()
                    .any(|branch| branch.get("$ref").is_some() || !object_only(branch.get("type")))
            })
    });
    if automation || unsupported_union {
        tool["parameters"] = json!({"type":"object","properties":{},"additionalProperties":true});
        if tool.get("strict") == Some(&Value::Bool(true)) {
            tool["strict"] = json!(false);
        }
    }
    Ok(())
}

fn object_only(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(kind)) => kind.trim().eq_ignore_ascii_case("object"),
        Some(Value::Array(types)) => {
            !types.is_empty() && types.iter().all(|kind| object_only(Some(kind)))
        }
        _ => false,
    }
}

fn schema_children_mut(
    schema: &mut Value,
    mut visit: impl FnMut(&mut Value) -> Result<(), ServiceError>,
) -> Result<(), ServiceError> {
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(children) = schema.get_mut(key).and_then(Value::as_object_mut) {
            for child in children.values_mut() {
                visit(child)?;
            }
        }
    }
    for key in [
        "items",
        "prefixItems",
        "contains",
        "additionalProperties",
        "additionalItems",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contentSchema",
        "anyOf",
        "oneOf",
        "allOf",
        "not",
        "if",
        "then",
        "else",
    ] {
        if let Some(child) = schema.get_mut(key) {
            if let Some(children) = child.as_array_mut() {
                for child in children {
                    visit(child)?;
                }
            } else {
                visit(child)?;
            }
        }
    }
    Ok(())
}

fn inline_refs(
    schema: &mut Value,
    root: &Value,
    stack: &mut HashSet<String>,
    depth: usize,
    nodes: &mut usize,
    expanded: &mut bool,
) -> Result<(), ServiceError> {
    *nodes += 1;
    if depth > 64 || *nodes > 100_000 {
        return Err(invalid("Grok function schema exceeds its expansion limit"));
    }
    if !schema.is_object() {
        return Ok(());
    }
    if let Some(reference) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with("#/"))
        .map(str::to_owned)
    {
        if !stack.contains(&reference) {
            if let Some(target) = root
                .pointer(&reference[1..])
                .filter(|target| target.is_object() || target.is_boolean())
            {
                stack.insert(reference.clone());
                let mut replacement = target.clone();
                if let (Some(source), Some(target)) =
                    (schema.as_object(), replacement.as_object_mut())
                {
                    for (key, value) in source {
                        if key != "$ref" {
                            target.insert(key.clone(), value.clone());
                        }
                    }
                }
                inline_refs(&mut replacement, root, stack, depth + 1, nodes, expanded)?;
                stack.remove(&reference);
                *schema = replacement;
                *expanded = true;
                return Ok(());
            }
        }
    }
    schema_children_mut(schema, |child| {
        inline_refs(child, root, stack, depth + 1, nodes, expanded)
    })
}

fn has_ref(schema: &Value) -> bool {
    // Inspect only schema nodes; literal defaults/examples may legitimately contain "$ref".
    let mut copy = schema.clone();
    let mut found = schema.get("$ref").is_some();
    let _ = schema_children_mut(&mut copy, |child| {
        found |= has_ref(child);
        Ok(())
    });
    found
}

#[cfg(test)]
#[path = "grok_request_compat_tests.rs"]
mod tests;
