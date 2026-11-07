use super::{upstream_error, ChatContext, ChatFlavor, ServiceError};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn response_to_chat(response: &Value, context: &ChatContext) -> Result<Value, ServiceError> {
    if let Some(error) = super::super::upstream_errors::chat_failure(response, 502) {
        return Err(error);
    }
    if response.get("status").and_then(Value::as_str) == Some("failed") {
        return Err(upstream_error(
            response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("The upstream response failed"),
        ));
    }
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| upstream_error("Responses output must be an array"))?;
    let mut text = String::new();
    let mut refusal = String::new();
    let mut reasoning = String::new();
    let mut reasoning_details = Vec::new();
    let mut calls = Vec::new();
    let mut annotations = Vec::new();
    let mut logprobs = Vec::new();
    let mut images = Vec::new();
    let mut citation_keys = BTreeSet::new();
    let mut text_offset = 0i64;
    for item in output {
        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match part.get("type").and_then(Value::as_str) {
                    Some("output_text") | Some("text") => text.push_str(part.get("text").and_then(Value::as_str).unwrap_or_default()),
                    Some("refusal") => refusal.push_str(part.get("refusal").and_then(Value::as_str).unwrap_or_default()),
                    _ => return Err(upstream_error("This assistant output content cannot be represented as a Chat text completion")),
                }
                    annotations.extend(url_citations(
                        part.get("annotations"),
                        text_offset,
                        &mut citation_keys,
                    ));
                    text_offset += part
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .chars()
                        .count() as i64;
                    logprobs.extend(
                        part.get("logprobs")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                }
            }
            Some("function_call") | Some("custom_tool_call") => {
                let mut call = tool_call(item)?;
                let kind = if call["type"] == "custom" {
                    "custom"
                } else {
                    "function"
                };
                if let Some(name) = call[kind]["name"].as_str() {
                    let name = context.tool_name(name).to_owned();
                    call[kind]["name"] = json!(name);
                }
                calls.push(call);
            }
            Some("reasoning") => {
                for part in item
                    .get("summary")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    reasoning
                        .push_str(part.get("text").and_then(Value::as_str).unwrap_or_default());
                }
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if part["type"] == "reasoning_text" {
                        reasoning.push_str(part["text"].as_str().unwrap_or_default());
                    }
                }
                reasoning_details.push(item.clone());
            }
            Some("image_generation_call") => {
                if let Some(data) = item["result"].as_str().filter(|data| !data.is_empty()) {
                    images.push(chat_image(
                        data,
                        item["output_format"].as_str(),
                        images.len(),
                    ));
                }
            }
            // Native tool execution results are retained as an extension rather than recast as client function calls.
            _ => {}
        }
    }
    let reason = finish_reason(response, !calls.is_empty(), context.legacy_functions);
    let mut message = json!({"role":"assistant","content":if text.is_empty() && !calls.is_empty() {Value::Null} else {json!(text)},"refusal":if refusal.is_empty() {Value::Null} else {json!(refusal)}});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !reasoning_details.is_empty() {
        message["reasoning_details"] = json!(reasoning_details);
    }
    if !annotations.is_empty() {
        message["annotations"] = json!(annotations);
    }
    if !images.is_empty() {
        message["images"] = json!(images);
    }
    if context.legacy_functions && !calls.is_empty() {
        if calls.len() != 1 || calls[0].get("type").and_then(Value::as_str) != Some("function") {
            return Err(upstream_error(
                "Legacy function_call cannot represent multiple or custom tool calls",
            ));
        }
        message["function_call"] = calls[0]["function"].clone();
    } else if !calls.is_empty() {
        message["tool_calls"] = json!(calls);
    }
    let mut choice = if context.flavor == ChatFlavor::Completions {
        if message.get("tool_calls").is_some() || message.get("function_call").is_some() {
            return Err(upstream_error(
                "Legacy Completions cannot represent tool calls",
            ));
        }
        json!({"index":0,"text":text,"finish_reason":reason,"logprobs":if logprobs.is_empty() {Value::Null} else {json!({"content":logprobs,"refusal":null})}})
    } else {
        json!({"index":0,"message":message,"finish_reason":reason,"logprobs":if logprobs.is_empty() {Value::Null} else {json!({"content":logprobs,"refusal":null})}})
    };
    choice["native_finish_reason"] = json!(native_finish_reason(response, reason));
    let mut result = json!({"id":completion_id(response, context.flavor),"object":if context.flavor == ChatFlavor::Completions {"text_completion"} else {"chat.completion"},"created":response.get("created_at").cloned().unwrap_or(json!(chrono::Utc::now().timestamp())),"model":response.get("model").cloned().unwrap_or(json!(context.model)),"choices":[choice]});
    if let Some(usage) = response.get("usage").filter(|v| !v.is_null()) {
        result["usage"] = chat_usage(usage);
    }
    for key in ["service_tier", "system_fingerprint"] {
        if let Some(value) = response.get(key).filter(|value| {
            key != "service_tier" || value.as_str().is_some_and(|tier| !tier.trim().is_empty())
        }) {
            result[key] = value.clone();
        }
    }
    let native: Vec<_> = output
        .iter()
        .filter(|item| {
            !matches!(
                item.get("type").and_then(Value::as_str),
                Some("message" | "reasoning" | "function_call" | "custom_tool_call")
            )
        })
        .cloned()
        .collect();
    if !native.is_empty() {
        result["responses_output"] = json!(native);
    }
    Ok(result)
}

pub(crate) fn completion_id(response: &Value, flavor: ChatFlavor) -> String {
    let raw = response
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
    let prefix = if flavor == ChatFlavor::Completions {
        "cmpl-"
    } else {
        "chatcmpl-"
    };
    if raw.starts_with(prefix) {
        raw
    } else {
        format!("{prefix}{raw}")
    }
}

pub(crate) fn tool_call(item: &Value) -> Result<Value, ServiceError> {
    let id = item
        .get("call_id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| upstream_error("Tool output is missing call_id"))?;
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| upstream_error("Tool output is missing name"))?;
    if item.get("type").and_then(Value::as_str) == Some("custom_tool_call") {
        Ok(
            json!({"id":id,"type":"custom","custom":{"name":name,"input":item.get("input").cloned().unwrap_or(json!(""))}}),
        )
    } else {
        Ok(
            json!({"id":id,"type":"function","function":{"name":name,"arguments":item.get("arguments").cloned().unwrap_or(json!(""))}}),
        )
    }
}

pub(crate) fn finish_reason(response: &Value, tools: bool, legacy: bool) -> &'static str {
    match response
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
    {
        Some("max_output_tokens" | "max_tokens") => "length",
        Some("content_filter") => "content_filter",
        _ if response["status"] == "incomplete" => "stop",
        _ if tools => {
            if legacy {
                "function_call"
            } else {
                "tool_calls"
            }
        }
        _ => "stop",
    }
}

pub(crate) fn chat_usage(usage: &Value) -> Value {
    let mut result = json!({});
    for (from, to) in [
        ("input_tokens", "prompt_tokens"),
        ("output_tokens", "completion_tokens"),
        ("total_tokens", "total_tokens"),
        ("input_tokens_details", "prompt_tokens_details"),
        ("output_tokens_details", "completion_tokens_details"),
    ] {
        if let Some(value) = usage.get(from) {
            result[to] = value.clone();
        }
    }
    if let Some(n) = usage
        .pointer("/input_tokens_details/cache_write_tokens")
        .and_then(Value::as_u64)
    {
        result["prompt_tokens_details"]["cached_creation_tokens"] = json!(n);
    } else if let Some(details) = result
        .get_mut("prompt_tokens_details")
        .and_then(Value::as_object_mut)
    {
        details.remove("cache_write_tokens");
    }
    if result.get("total_tokens").is_none() {
        if let (Some(input), Some(output)) = (
            usage.get("input_tokens").and_then(Value::as_u64),
            usage.get("output_tokens").and_then(Value::as_u64),
        ) {
            result["total_tokens"] = json!(input.saturating_add(output));
        }
    }
    result
}

/// Preserve the upstream reason alongside the OpenAI normalized reason.
pub(crate) fn native_finish_reason<'a>(response: &'a Value, normalized: &'a str) -> &'a str {
    if response["status"] == "incomplete" {
        response
            .pointer("/incomplete_details/reason")
            .and_then(Value::as_str)
            .unwrap_or("")
    } else {
        normalized
    }
}

pub(crate) fn image_mime(format: Option<&str>) -> String {
    let format = format.unwrap_or("");
    if format.contains('/') {
        return format.to_owned();
    }
    match format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
    .to_owned()
}

pub(crate) fn chat_image(data: &str, format: Option<&str>, index: usize) -> Value {
    json!({"type":"image_url","index":index,"image_url":{"url":format!("data:{};base64,{data}", image_mime(format))}})
}

/// Responses citations use part-local Unicode scalar offsets; Chat citations
/// use offsets into the concatenated assistant text and a nested envelope.
pub(crate) fn url_citations(
    value: Option<&Value>,
    offset: i64,
    seen: &mut BTreeSet<String>,
) -> Vec<Value> {
    let Some(value) = value else {
        return Vec::new();
    };
    let annotations = value
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_else(|| std::slice::from_ref(value));
    let mut out = Vec::new();
    for annotation in annotations {
        if annotation["type"] != "url_citation" {
            continue;
        }
        let citation = annotation.get("url_citation").unwrap_or(annotation);
        let start = citation["start_index"]
            .as_i64()
            .unwrap_or(0)
            .saturating_add(offset);
        let end = citation["end_index"]
            .as_i64()
            .unwrap_or(0)
            .saturating_add(offset);
        if start < 0 || end < start {
            continue;
        }
        let url = citation["url"].as_str().unwrap_or("");
        let id = annotation["id"].as_str().unwrap_or("");
        let key = format!("{}\0{start}", if url.is_empty() { id } else { url });
        let id_key = format!("id\0{id}");
        if seen.contains(&key) || (!id.is_empty() && seen.contains(&id_key)) {
            continue;
        }
        seen.insert(key);
        if !id.is_empty() {
            seen.insert(id_key);
        }
        out.push(json!({"type":"url_citation","url_citation":{"url":url,"title":citation["title"].as_str().unwrap_or(""),"start_index":start,"end_index":end}}));
    }
    out
}
