//! Shared pure lowering of client-side tool discovery. No provider transport,
//! history cache, encrypted-content conversion, or hosted tool execution lives here.
use super::contract::ServiceError;
use serde_json::{json, Value};
use std::collections::HashSet;
const MAX_BINDINGS: usize = 4096;
fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_client_tool_discovery", message)
}
fn upstream(message: &str) -> ServiceError {
    ServiceError::new(502, "invalid_client_tool_search_arguments", message)
}
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}
fn namespace(value: &Value) -> Option<String> {
    text(value, "namespace")
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}
#[derive(Default)]
pub(super) struct Discovery {
    declared: bool,
    client_ids: HashSet<String>,
    needed: bool,
}
impl Discovery {
    pub(super) fn active(&self) -> bool {
        self.declared || !self.client_ids.is_empty()
    }
}
pub(super) fn client_declaration(tool: &Value) -> bool {
    text(tool, "type") == Some("tool_search")
        && matches!(text(tool, "execution"), None | Some("client"))
}

pub(super) fn lower_declaration(tool: &Value, wire: &str) -> Result<Value, ServiceError> {
    let mut lowered = tool.clone();
    lowered["type"] = json!("function");
    lowered["name"] = json!(wire);
    lowered.as_object_mut().unwrap().remove("execution");
    if lowered.get("parameters").is_none_or(Value::is_null) {
        lowered["parameters"] = json!({"type":"object","properties":{"query":{"type":"string"},"limit":{"type":"integer"}},"required":["query"]});
    }
    if lowered.get("description").is_none_or(Value::is_null) {
        lowered["description"] = json!("Search and load client tools for the current task.");
    }
    Ok(lowered)
}

fn search_item(item: &Value, declared: bool) -> bool {
    matches!(
        text(item, "type"),
        Some("tool_search_call" | "tool_search_output")
    ) && (text(item, "execution") == Some("client")
        || (text(item, "execution").is_none() && declared))
}

fn declaration_identity(tool: &Value, parent: &str) -> String {
    json!([
        text(tool, "type").unwrap_or(""),
        namespace(tool).unwrap_or_else(|| parent.to_owned()),
        text(tool, "name").unwrap_or("")
    ])
    .to_string()
}

fn declaration_keys(
    tools: &[Value],
    parent: &str,
    keys: &mut HashSet<String>,
    depth: usize,
) -> Result<(), ServiceError> {
    if depth > 16 {
        return Err(invalid("Tool namespaces exceed their nesting limit"));
    }
    for tool in tools {
        let name = text(tool, "name").unwrap_or("");
        let name = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}.{name}")
        };
        if text(tool, "type") == Some("namespace") {
            if let Some(children) = tool.get("tools").and_then(Value::as_array) {
                declaration_keys(children, &name, keys, depth + 1)?;
            }
        } else {
            keys.insert(declaration_identity(tool, parent));
            if keys.len() > MAX_BINDINGS {
                return Err(invalid("Too many client tool identities"));
            }
        }
    }
    Ok(())
}

fn novel_tool(
    tool: &Value,
    parent: &str,
    keys: &mut HashSet<String>,
    depth: usize,
) -> Result<Option<Value>, ServiceError> {
    if depth > 16 {
        return Err(invalid("Tool namespaces exceed their nesting limit"));
    }
    let short = text(tool, "name").unwrap_or("");
    let name = if parent.is_empty() {
        short.to_owned()
    } else {
        format!("{parent}.{short}")
    };
    if text(tool, "type") == Some("namespace") {
        let children = tool
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Discovered namespace requires a tools array"))?;
        let mut kept = vec![];
        for child in children {
            if let Some(child) = novel_tool(child, &name, keys, depth + 1)? {
                kept.push(child);
            }
        }
        if kept.is_empty() {
            return Ok(None);
        }
        let mut result = tool.clone();
        result["tools"] = json!(kept);
        return Ok(Some(result));
    }
    if !keys.insert(declaration_identity(tool, parent)) {
        return Ok(None);
    }
    if keys.len() > MAX_BINDINGS {
        return Err(invalid("Too many client tool identities"));
    }
    Ok(Some(tool.clone()))
}

fn completed_discoveries(item: &Value) -> Result<Option<Vec<Value>>, ServiceError> {
    if text(item, "type") != Some("tool_search_output")
        || text(item, "execution") == Some("server")
        || text(item, "status").is_some_and(|status| status != "completed")
    {
        return Ok(None);
    }
    if let Some(tools) = item.get("tools") {
        return tools
            .as_array()
            .cloned()
            .map(Some)
            .ok_or_else(|| invalid("Discovered tools must be an array"));
    }
    let output = match item.get("output") {
        Some(Value::String(raw)) => serde_json::from_str::<Value>(raw).ok(),
        output => output.cloned(),
    };
    Ok(output.and_then(|output| {
        output
            .as_array()
            .cloned()
            .or_else(|| output.get("tools").and_then(Value::as_array).cloned())
    }))
}

pub(super) fn promote(body: &mut Value) -> Result<Discovery, ServiceError> {
    let mut declared = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(client_declaration));
    let input = body
        .get("input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    declared |= input.iter().any(|item| {
        text(item, "type") == Some("additional_tools")
            && item
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| tools.iter().any(client_declaration))
    });
    let mut client_ids = HashSet::new();
    for item in &input {
        if search_item(item, declared) {
            if let Some(id) = text(item, "call_id").or_else(|| text(item, "id")) {
                client_ids.insert(id.to_owned());
            }
        }
    }
    let additional_tools = input
        .iter()
        .any(|item| text(item, "type") == Some("additional_tools"));
    if !declared && client_ids.is_empty() && !additional_tools {
        return Ok(Discovery {
            declared,
            client_ids,
            needed: false,
        });
    }
    // An explicit empty list disables tools. Missing declarations can occur on
    // follow-up requests, whose completed discoveries still define callable tools.
    let may_promote = body.get("tools").is_none()
        || body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| !tools.is_empty());
    if may_promote {
        let mut tools = body
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut known = HashSet::new();
        declaration_keys(&tools, "", &mut known, 0)?;
        for item in &input {
            if text(item, "type") == Some("additional_tools") {
                let additional = item
                    .get("tools")
                    .and_then(Value::as_array)
                    .ok_or_else(|| invalid("Additional tools must be an array"))?;
                for tool in additional {
                    if let Some(tool) = novel_tool(tool, "", &mut known, 0)? {
                        tools.push(tool);
                    }
                }
            }
        }
        // Current declarations win; among historical discoveries the latest
        // completed definition wins, so a stale schema cannot replace it.
        for item in input.iter().rev() {
            if !search_item(item, declared)
                && !text(item, "call_id").is_some_and(|id| client_ids.contains(id))
            {
                continue;
            }
            if let Some(discovered) = completed_discoveries(item)? {
                for tool in &discovered {
                    if let Some(tool) = novel_tool(tool, "", &mut known, 0)? {
                        tools.push(tool);
                    }
                }
            }
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }
    Ok(Discovery {
        declared,
        client_ids,
        needed: true,
    })
}

pub(super) fn lower_history(
    body: &mut Value,
    wire: &str,
    discovery: &Discovery,
) -> Result<(), ServiceError> {
    if !discovery.needed {
        return Ok(());
    }
    let declared = discovery.declared;
    let client_ids = &discovery.client_ids;
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        // Declarations are promoted once; each upstream adapter chooses its final
        // supported representation. Explicit tools=[] still disables new tools.
        items.retain(|item| text(item, "type") != Some("additional_tools"));
        for item in items {
            let paired = text(item, "execution") != Some("server")
                && text(item, "call_id").is_some_and(|id| client_ids.contains(id));
            if !search_item(item, declared) && !paired {
                continue;
            }
            match text(item, "type") {
                Some("tool_search_call") => {
                    let id = text(item, "call_id")
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| invalid("Client tool search requires call_id"))?
                        .to_owned();
                    let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    let arguments = if let Some(raw) = arguments.as_str() {
                        decode_arguments(raw, false)
                            .map_err(|_| invalid("Invalid client tool search arguments"))?
                    } else {
                        arguments
                    };
                    if !arguments.is_object() {
                        return Err(invalid("Client tool search arguments must be an object"));
                    }
                    item["type"] = json!("function_call");
                    item["name"] = json!(wire);
                    item["call_id"] = json!(id);
                    item["arguments"] = json!(arguments.to_string());
                    if let Some(id) = text(item, "id").map(str::to_owned) {
                        item["id"] = json!(wire_item_id(&id));
                    }
                    item.as_object_mut().unwrap().remove("execution");
                    item.as_object_mut().unwrap().remove("status");
                }
                Some("tool_search_output") => {
                    if text(item, "call_id").is_none_or(str::is_empty) {
                        return Err(invalid("Client tool search output requires call_id"));
                    }
                    let output = item
                        .get("output")
                        .or_else(|| item.get("tools"))
                        .cloned()
                        .ok_or_else(|| {
                            invalid("Client tool search output requires output or tools")
                        })?;
                    let output = match output {
                        Value::String(s) => s,
                        Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    item["type"] = json!("function_call_output");
                    item["output"] = json!(output);
                    let object = item.as_object_mut().unwrap();
                    for field in ["tools", "execution", "status", "id"] {
                        object.remove(field);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if !tools.iter().any(|t| {
            text(t, "type") == Some("tool_search") && text(t, "execution") == Some("server")
        }) {
            remove_deferred(tools, 0)?;
        }
    }
    Ok(())
}

fn remove_deferred(tools: &mut [Value], depth: usize) -> Result<(), ServiceError> {
    if depth > 16 {
        return Err(invalid("Tool namespaces exceed their nesting limit"));
    }
    for tool in tools {
        if let Some(object) = tool.as_object_mut() {
            object.remove("defer_loading");
        }
        if let Some(children) = tool.get_mut("tools").and_then(Value::as_array_mut) {
            remove_deferred(children, depth + 1)?;
        }
    }
    Ok(())
}

pub(super) fn wire_item_id(id: &str) -> String {
    id.strip_prefix("tsc_")
        .map(|suffix| format!("fc_{suffix}"))
        .unwrap_or_else(|| id.to_owned())
}
pub(super) fn client_item_id(id: &str) -> String {
    id.strip_prefix("fc_")
        .map(|suffix| format!("tsc_{suffix}"))
        .unwrap_or_else(|| id.to_owned())
}
pub(super) fn decode_arguments(raw: &str, partial: bool) -> Result<Value, ServiceError> {
    if raw.is_empty() && partial {
        return Ok(json!({}));
    }
    let arguments: Value = serde_json::from_str(raw)
        .map_err(|_| upstream("Upstream returned invalid client tool search arguments"))?;
    if !arguments.is_object() {
        return Err(upstream("Client tool search arguments must be an object"));
    }
    Ok(arguments)
}
