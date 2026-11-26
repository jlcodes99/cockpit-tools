use super::{unsupported, upstream, ServiceError};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const NAME: &str = "ToolSearch";

pub(super) fn prepare(source: &Value, response: &mut Value) -> Result<(), ServiceError> {
    let mut deferred = BTreeSet::new();
    let mut declared = false;
    for tool in source["tools"].as_array().into_iter().flatten() {
        if let Some(name) = tool["name"].as_str() {
            if tool["defer_loading"] == true {
                deferred.insert(name);
            } else if name == NAME && matches!(tool["type"].as_str(), None | Some("custom")) {
                declared = true;
            }
        }
    }
    let forced = source["tool_choice"]["type"] == "tool"
        && source["tool_choice"]["name"]
            .as_str()
            .is_some_and(|name| name == NAME || deferred.contains(name));
    let native = declared && !forced;
    let mut definitions = BTreeMap::new();
    if native {
        let tools = response["tools"]
            .as_array_mut()
            .expect("converted Claude tools");
        tools.retain_mut(|tool| {
            if tool["type"] != "function" {
                return true;
            }
            let Some(name) = tool["name"].as_str() else {
                return true;
            };
            if deferred.contains(name) {
                let name = name.to_owned();
                tool["defer_loading"] = json!(true);
                definitions.insert(name, tool.clone());
                return false;
            }
            if name == NAME {
                tool["type"] = json!("tool_search");
                tool["execution"] = json!("client");
                let fields = tool.as_object_mut().expect("converted tool object");
                fields.remove("name");
                fields.remove("strict");
            }
            true
        });
    }

    let mut calls = BTreeSet::new();
    let mut pending_content = Vec::new();
    let mut rewritten = Vec::new();
    let input = response["input"]
        .as_array_mut()
        .expect("converted Claude input");
    for mut item in std::mem::take(input) {
        if item["type"] != "function_call_output" && !pending_content.is_empty() {
            flush_content(&mut pending_content, Some(&mut item), &mut rewritten);
        }
        if item["type"] == "function_call" && item["name"] == NAME {
            if let Some(id) = item["call_id"].as_str() {
                calls.insert(id.to_owned());
            }
            if native {
                item["type"] = json!("tool_search_call");
                item["execution"] = json!("client");
                item["status"] = json!("completed");
                item["arguments"] =
                    serde_json::from_str(item["arguments"].as_str().unwrap_or("{}"))
                        .map_err(|_| unsupported("tool_use.input"))?;
                item.as_object_mut()
                    .expect("converted call object")
                    .remove("name");
            }
        } else if item["type"] == "function_call_output" {
            let search_result = item["call_id"].as_str().is_some_and(|id| calls.remove(id));
            let references = item["output"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["type"] == "tool_reference"));
            if references && !search_result {
                return Err(unsupported("tool_result.content.type"));
            }
            if search_result && native {
                let mut loaded = Vec::new();
                let mut seen = BTreeSet::new();
                let output = item
                    .as_object_mut()
                    .expect("converted result object")
                    .remove("output")
                    .unwrap_or(Value::Null);
                match output {
                    Value::Array(parts) => {
                        for part in parts {
                            if part["type"] == "tool_reference" {
                                if let Some(name) = part["tool_name"].as_str() {
                                    if seen.insert(name.to_owned()) {
                                        if let Some(tool) = definitions.get(name) {
                                            loaded.push(tool.clone());
                                        }
                                    }
                                }
                            } else {
                                pending_content.push(part);
                            }
                        }
                    }
                    Value::String(text) if !text.is_empty() => {
                        pending_content.push(json!({"type":"input_text","text":text}));
                    }
                    _ => {}
                }
                item["type"] = json!("tool_search_output");
                item["execution"] = json!("client");
                item["status"] = json!("completed");
                item["tools"] = json!(loaded);
            } else if references {
                for part in item["output"]
                    .as_array_mut()
                    .expect("tool reference content")
                {
                    if part["type"] == "tool_reference" {
                        *part = json!({"type":"input_text","text":part.to_string()});
                    }
                }
            }
        }
        rewritten.push(item);
    }
    if !pending_content.is_empty() {
        flush_content(&mut pending_content, None, &mut rewritten);
    }
    *input = rewritten;
    Ok(())
}

fn flush_content(pending: &mut Vec<Value>, next: Option<&mut Value>, input: &mut Vec<Value>) {
    let mut content = std::mem::take(pending);
    if let Some(message) = next.filter(|item| item["type"] == "message" && item["role"] == "user") {
        if let Some(parts) = message["content"].as_array_mut() {
            content.append(parts);
            *parts = content;
            return;
        }
    }
    input.push(json!({"type":"message","role":"user","content":content}));
}

pub(in crate::modules::codex_native_api) fn lower_for_function_backend(
    body: &mut Value,
) -> Result<(), ServiceError> {
    use super::super::client_tool_discovery as discovery;
    if !body["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(discovery::client_declaration))
    {
        return Ok(());
    }
    let state = discovery::promote(body)?;
    for tool in body["tools"].as_array_mut().expect("promoted tools") {
        if discovery::client_declaration(tool) {
            *tool = discovery::lower_declaration(tool, NAME)?;
        }
    }
    discovery::lower_history(body, NAME, &state)
}

pub(super) fn function_call(item: &Value) -> Result<Option<Value>, ServiceError> {
    if item["type"] != "tool_search_call"
        || !matches!(item["execution"].as_str(), None | Some("client"))
    {
        return Ok(None);
    }
    let call_id = item["call_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| upstream("Client tool search call is missing its call ID"))?;
    let arguments = match item.get("arguments") {
        Some(Value::Object(_)) => item["arguments"].clone(),
        Some(Value::String(raw)) if !raw.trim().is_empty() => {
            super::super::client_tool_discovery::decode_arguments(raw, false)?
        }
        None | Some(Value::Null | Value::String(_)) => json!({}),
        _ => return Err(upstream("Client tool search arguments must be an object")),
    };
    let mut function = json!({"type":"function_call","name":NAME,"call_id":call_id,
        "arguments":arguments.to_string()});
    if let Some(id) = item.get("id") {
        function["id"] = id.clone();
    }
    Ok(Some(function))
}
