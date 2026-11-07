use super::{content_to_chat, incompatible, invalid, tools::ToolContext, ServiceError};
use serde_json::{json, Value};
use std::collections::HashSet;

fn merge_reasoning(existing: &mut String, incoming: &str) {
    if incoming.trim().is_empty() || existing.trim() == incoming.trim() {
        return;
    }
    if existing.trim().is_empty() || existing == "[reasoning unavailable]" {
        *existing = incoming.into();
    } else if incoming != "[reasoning unavailable]" {
        existing.push_str("\n\n");
        existing.push_str(incoming);
    }
}

pub(super) fn convert(
    body: &Value,
    context: &ToolContext,
    native: bool,
) -> Result<Vec<Value>, ServiceError> {
    let mut messages = Vec::new();
    if let Some(instructions) = body["instructions"].as_str() {
        if !instructions.is_empty() {
            messages.push(json!({"role":"system","content":instructions}));
        }
    }
    let input = match body.get("input") {
        Some(Value::String(s)) => vec![json!({"role":"user","content":s})],
        Some(Value::Array(items)) => items.clone(),
        None => Vec::new(),
        _ => return Err(invalid("input", "expected text or an array")),
    };
    let outputs: HashSet<String> = input
        .iter()
        .filter(|i| {
            matches!(
                i["type"].as_str(),
                Some("function_call_output" | "custom_tool_call_output")
            )
        })
        .filter_map(|i| i["call_id"].as_str().map(str::to_owned))
        .collect();
    let mut awaiting = HashSet::<String>::new();
    let mut deferred = Vec::new();
    let mut reasoning = String::new();
    for item in input {
        match item["type"].as_str().unwrap_or("message") {
            "additional_tools" => {}
            "reasoning" => {
                if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
                    return Err(incompatible(
                        "input.encrypted_content",
                        "encrypted reasoning belongs to the original Responses upstream",
                    ));
                }
                let text = item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                merge_reasoning(
                    &mut reasoning,
                    if text.is_empty() && !native {
                        "[reasoning unavailable]"
                    } else {
                        &text
                    },
                );
            }
            "function_call" | "custom_tool_call" => {
                let custom = item["type"] == "custom_tool_call";
                let name = context.canonical(
                    item["name"].as_str().unwrap_or(""),
                    item["namespace"].as_str(),
                );
                let mut call = json!({"id":item["call_id"],"type":if custom && native {"custom"}else{"function"}});
                if custom && native {
                    call["custom"] = json!({"name":name,"input":item["input"]});
                } else {
                    call["function"] = json!({"name":name,"arguments":if custom {json!({"input":item["input"].as_str().unwrap_or("")}).to_string()}else{item["arguments"].as_str().unwrap_or("").to_owned()}});
                }
                merge_reasoning(
                    &mut reasoning,
                    item["reasoning_content"].as_str().unwrap_or(""),
                );
                if !messages
                    .last()
                    .is_some_and(|m: &Value| m["role"] == "assistant")
                {
                    messages.push(json!({"role":"assistant","tool_calls":[]}));
                }
                let last = messages.last_mut().unwrap();
                if !last["tool_calls"].is_array() {
                    last["tool_calls"] = json!([]);
                }
                last["tool_calls"].as_array_mut().unwrap().push(call);
                if !reasoning.is_empty() {
                    let mut combined = last["reasoning_content"].as_str().unwrap_or("").to_owned();
                    merge_reasoning(&mut combined, &reasoning);
                    last["reasoning_content"] = json!(combined);
                    reasoning.clear();
                }
                if let Some(id) = item["call_id"].as_str() {
                    if outputs.contains(id) {
                        awaiting.insert(id.into());
                    }
                }
            }
            "function_call_output" | "custom_tool_call_output" => {
                let output = item
                    .get("output")
                    .ok_or_else(|| invalid("input.output", "missing tool output"))?;
                messages.push(json!({"role":"tool","tool_call_id":item["call_id"],"content":tool_content(output,item["type"]=="custom_tool_call_output",native)?}));
                if let Some(id) = item["call_id"].as_str() {
                    awaiting.remove(id);
                }
                if awaiting.is_empty() {
                    messages.append(&mut deferred);
                }
            }
            "message" => {
                let role = item["role"]
                    .as_str()
                    .ok_or_else(|| invalid("input.role", "expected a role"))?;
                if role != "assistant" && !reasoning.is_empty() {
                    let message = json!({"role":"assistant","content":"","reasoning_content":std::mem::take(&mut reasoning)});
                    if awaiting.is_empty() {
                        messages.push(message);
                    } else {
                        deferred.push(message);
                    }
                }
                let role = if role == "developer" {
                    if native {
                        "system"
                    } else {
                        "user"
                    }
                } else {
                    role
                };
                let mut message = json!({"role":role,"content":content_to_chat(item.get("content").unwrap_or(&json!("")))?});
                if role == "assistant" {
                    merge_reasoning(
                        &mut reasoning,
                        item["reasoning_content"].as_str().unwrap_or(""),
                    );
                    if !reasoning.is_empty() {
                        message["reasoning_content"] = json!(std::mem::take(&mut reasoning));
                    }
                }
                if awaiting.is_empty() {
                    messages.push(message);
                } else {
                    deferred.push(message);
                }
            }
            _ => {
                return Err(incompatible(
                    "input.type",
                    "this native Responses item cannot be replayed to a Chat upstream",
                ))
            }
        }
    }
    if !reasoning.is_empty() {
        let message = json!({"role":"assistant","content":"","reasoning_content":reasoning});
        if awaiting.is_empty() {
            messages.push(message);
        } else {
            deferred.push(message);
        }
    }
    messages.append(&mut deferred);
    Ok(messages)
}

fn tool_content(output: &Value, custom: bool, native: bool) -> Result<Value, ServiceError> {
    if native {
        return content_to_chat(output);
    }
    let parsed = output
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let structured = parsed.as_ref().unwrap_or(output);
    if structured.as_array().is_some_and(|parts| {
        parts
            .iter()
            .any(|p| matches!(p["type"].as_str(), Some("input_image" | "image_url")))
    }) {
        return content_to_chat(structured);
    }
    if custom {
        if let Some(parts) = output.as_array() {
            return Ok(json!(parts
                .iter()
                .filter_map(|p| p.as_str().or_else(|| p["text"].as_str()))
                .collect::<Vec<_>>()
                .join("")));
        }
    }
    Ok(json!(output
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| output.to_string())))
}
