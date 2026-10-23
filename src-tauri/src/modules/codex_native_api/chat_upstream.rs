//! Translation between canonical Responses and a third-party Chat API.
use super::{
    chat::{incompatible, invalid, upstream_error},
    contract::ServiceError,
};
use serde_json::{json, Value};
#[path = "chat_upstream_stream.rs"]
mod stream;
pub use stream::ResponsesStreamAdapter;
#[path = "chat_upstream_messages.rs"]
mod messages;
#[path = "chat_upstream_tools.rs"]
mod tools;
pub(crate) use tools::ToolContext;

pub fn responses_to_chat(body: &Value) -> Result<Value, ServiceError> {
    responses_to_chat_inner(body, false)
}

pub(crate) fn responses_to_native_chat(body: &Value) -> Result<Value, ServiceError> {
    responses_to_chat_inner(body, true)
}

fn responses_to_chat_inner(body: &Value, native: bool) -> Result<Value, ServiceError> {
    let mut out = body
        .as_object()
        .ok_or_else(|| invalid("body", "expected an object"))?
        .clone();
    if body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        return Err(incompatible(
            "previous_response_id",
            "a Chat upstream requires the expanded conversation",
        ));
    }
    for key in ["conversation", "background"] {
        if body
            .get(key)
            .is_some_and(|v| !v.is_null() && v != &json!(false))
        {
            return Err(incompatible(
                key,
                "stateful Responses execution requires a Responses upstream",
            ));
        }
    }
    for key in [
        "input",
        "instructions",
        "previous_response_id",
        "conversation",
        "background",
        "include",
        "text",
        "reasoning",
        "max_output_tokens",
        "truncation",
        "store",
    ] {
        out.remove(key);
    }
    let context = ToolContext::from_responses(body);
    out.insert(
        "messages".into(),
        json!(messages::convert(body, &context, native)?),
    );
    if let Some(v) = body.get("max_output_tokens") {
        out.insert("max_completion_tokens".into(), v.clone());
    }
    if let Some(v) = body.pointer("/reasoning/effort") {
        out.insert("reasoning_effort".into(), v.clone());
    }
    if let Some(v) = body.pointer("/text/verbosity") {
        out.insert("verbosity".into(), v.clone());
    }
    if let Some(format) = body.pointer("/text/format") {
        let value = if format["type"] == "json_schema" {
            let mut schema = format
                .as_object()
                .ok_or_else(|| invalid("text.format", "expected an object"))?
                .clone();
            schema.remove("type");
            json!({"type":"json_schema","json_schema":schema})
        } else {
            format.clone()
        };
        out.insert("response_format".into(), value);
    }
    let converted = context.tools(native);
    out.remove("tools");
    if !converted.is_empty() {
        out.insert("tools".into(), json!(converted));
    }
    if let Some(choice) = body.get("tool_choice") {
        out.insert(
            "tool_choice".into(),
            choice_to_chat(choice, &context, native)?,
        );
    }
    if body
        .get("include")
        .and_then(Value::as_array)
        .is_some_and(|v| v.iter().any(|i| i == "message.output_text.logprobs"))
    {
        out.insert("logprobs".into(), json!(true));
    }
    if body["stream"].as_bool() == Some(true) {
        out.insert("stream_options".into(), json!({"include_usage":true}));
    }
    Ok(json!(out))
}
fn choice_to_chat(v: &Value, context: &ToolContext, native: bool) -> Result<Value, ServiceError> {
    if v.is_string() {
        return Ok(v.clone());
    }
    let kind = v["type"].as_str().unwrap_or("");
    if matches!(kind, "function" | "custom") {
        let name = context.canonical(
            v["name"]
                .as_str()
                .or_else(|| v.pointer(&format!("/{kind}/name")).and_then(Value::as_str))
                .unwrap_or(""),
            v["namespace"].as_str(),
        );
        let kind = if kind == "custom" && !native {
            "function"
        } else {
            kind
        };
        let mut out = json!({"type":kind});
        out[kind] = json!({"name":name});
        return Ok(out);
    }
    if kind == "allowed_tools" {
        let tools: Result<Vec<_>, _> = v["tools"]
            .as_array()
            .ok_or_else(|| invalid("tool_choice.tools", "expected an array"))?
            .iter()
            .map(|choice| choice_to_chat(choice, context, native))
            .collect();
        return Ok(
            json!({"type":"allowed_tools","allowed_tools":{"mode":v["mode"],"tools":tools?}}),
        );
    }
    Err(incompatible("tool_choice", "unsupported Chat tool choice"))
}
fn content_to_chat(content: &Value) -> Result<Value, ServiceError> {
    if content.is_string() {
        return Ok(content.clone());
    }
    let mut parts = Vec::new();
    for p in content
        .as_array()
        .ok_or_else(|| invalid("input.content", "expected text or content parts"))?
    {
        parts.push(match p["type"].as_str().unwrap_or("") {
            "input_text" | "output_text" | "text" => json!({"type":"text","text":p["text"]}),
            "refusal" => json!({"type":"refusal","refusal":p["refusal"]}),
            "input_image" => {
                let url = p.get("image_url").ok_or_else(|| {
                    incompatible("input_image.file_id", "Chat requires an image URL")
                })?;
                let mut image = json!({"url":url});
                if let Some(d) = p.get("detail") {
                    image["detail"] = d.clone();
                }
                json!({"type":"image_url","image_url":image})
            }
            "input_file" => {
                let mut f = p.as_object().unwrap().clone();
                f.remove("type");
                json!({"type":"file","file":f})
            }
            "input_audio" => {
                if let Some(audio) = p.get("input_audio") {
                    json!({"type":"input_audio","input_audio":audio})
                } else {
                    let url = p["audio_url"]
                        .as_str()
                        .ok_or_else(|| invalid("input_audio", "missing audio data"))?;
                    let (mime, data) = url
                        .strip_prefix("data:audio/")
                        .and_then(|s| s.split_once(";base64,"))
                        .ok_or_else(|| {
                            incompatible(
                                "input_audio.audio_url",
                                "Chat requires inline base64 audio",
                            )
                        })?;
                    let format = match mime {
                        "mpeg" | "mp3" => "mp3",
                        "wav" | "wave" | "x-wav" => "wav",
                        "flac" => "flac",
                        "opus" | "ogg" => "opus",
                        "pcm" | "l16" => "pcm16",
                        _ => return Err(incompatible("input_audio", "Chat accepts mp3 or wav")),
                    };
                    json!({"type":"input_audio","input_audio":{"data":data,"format":format}})
                }
            }
            _ => {
                return Err(incompatible(
                    "input.content.type",
                    "unsupported Chat content part",
                ))
            }
        });
    }
    Ok(json!(parts))
}

pub(crate) fn responses_usage(usage: &Value) -> Value {
    let mut result = json!({});
    for (from, to) in [
        ("prompt_tokens", "input_tokens"),
        ("completion_tokens", "output_tokens"),
        ("total_tokens", "total_tokens"),
        ("prompt_tokens_details", "input_tokens_details"),
        ("completion_tokens_details", "output_tokens_details"),
    ] {
        if let Some(v) = usage.get(from) {
            result[to] = v.clone();
        }
    }
    if result.get("total_tokens").is_none() {
        if let (Some(a), Some(b)) = (
            result["input_tokens"].as_u64(),
            result["output_tokens"].as_u64(),
        ) {
            result["total_tokens"] = json!(a.saturating_add(b));
        }
    }
    result
}
pub(crate) fn status_for(reason: &str) -> (&'static str, Value) {
    match reason {
        "length" => ("incomplete", json!({"reason":"max_output_tokens"})),
        "content_filter" => ("incomplete", json!({"reason":"content_filter"})),
        _ => ("completed", Value::Null),
    }
}
pub fn chat_to_response(value: &Value, model: &str) -> Result<Value, ServiceError> {
    chat_to_response_with_context(value, model, &ToolContext::default())
}

pub(crate) fn chat_to_response_with_context(
    value: &Value,
    model: &str,
    context: &ToolContext,
) -> Result<Value, ServiceError> {
    if let Some(error) = super::upstream_errors::chat_failure(value, 502) {
        return Err(error);
    }
    let choices = value["choices"]
        .as_array()
        .ok_or_else(|| upstream_error("Chat response has no choices"))?;
    if choices.len() != 1 {
        return Err(upstream_error(
            "Canonical Responses requires exactly one Chat choice",
        ));
    }
    let message = &choices[0]["message"];
    if !message.is_object() {
        return Err(upstream_error("Chat response has no assistant message"));
    }
    if message.get("audio").is_some_and(|v| !v.is_null()) {
        return Err(upstream_error(
            "Chat audio output requires an audio-capable Responses adapter",
        ));
    }
    let reason = choices[0]["finish_reason"]
        .as_str()
        .ok_or_else(|| upstream_error("Chat response has no finish_reason"))?;
    if !matches!(
        reason,
        "stop" | "length" | "tool_calls" | "content_filter" | "function_call"
    ) {
        return Err(upstream_error(
            "Chat response returned an unknown finish_reason",
        ));
    }
    let (status, details) = status_for(reason);
    let mut output = Vec::new();
    if let Some(reason) = message["reasoning_content"]
        .as_str()
        .filter(|v| !v.is_empty())
        .or_else(|| message["reasoning"].as_str().filter(|v| !v.is_empty()))
    {
        output.push(json!({"id":format!("rs_{}",uuid::Uuid::now_v7()),"type":"reasoning","summary":[{"type":"summary_text","text":reason}]}));
    }
    let mut content = Vec::new();
    if let Some(text) = message["content"].as_str() {
        content.push(json!({"type":"output_text","text":text,"annotations":message.get("annotations").cloned().unwrap_or(json!([])),"logprobs":choices[0].pointer("/logprobs/content").cloned().unwrap_or(json!([]))}));
    } else if message.get("content").is_some_and(|v| !v.is_null()) {
        return Err(upstream_error("Chat output content must be text"));
    }
    if let Some(text) = message["refusal"].as_str().filter(|v| !v.is_empty()) {
        content.push(json!({"type":"refusal","refusal":text}));
    }
    if !content.is_empty() {
        output.push(json!({"id":format!("msg_{}",uuid::Uuid::now_v7()),"type":"message","role":"assistant","status":status,"content":content}));
    }
    let mut calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(call) = message.get("function_call").filter(|v| !v.is_null()) {
        calls.push(
            json!({"id":format!("call_{}",uuid::Uuid::now_v7()),"type":"function","function":call}),
        );
    }
    for call in calls {
        let custom = call["type"] == "custom";
        let key = if custom { "custom" } else { "function" };
        let details = &call[key];
        let id = call["id"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| upstream_error("Chat tool call has no ID"))?;
        let name = details["name"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| upstream_error("Chat tool call has no name"))?;
        let mut item = json!({"id":format!("fc_{}",uuid::Uuid::now_v7()),"type":if custom{"custom_tool_call"}else{"function_call"},"call_id":id,"name":name,"status":status});
        item[if custom { "input" } else { "arguments" }] = details
            .get(if custom { "input" } else { "arguments" })
            .cloned()
            .unwrap_or(json!(""));
        context.restore_item(&mut item);
        output.push(item);
    }
    let mut response = json!({"id":format!("resp_{}",value["id"].as_str().map(str::to_string).unwrap_or_else(||uuid::Uuid::now_v7().to_string())),"object":"response","created_at":value.get("created").cloned().unwrap_or(json!(chrono::Utc::now().timestamp())),"model":value["model"].as_str().unwrap_or(model),"status":status,"error":null,"incomplete_details":details,"output":output,"usage":value.get("usage").filter(|v|!v.is_null()).map(responses_usage).unwrap_or(Value::Null)});
    if let Some(v) = value.get("service_tier") {
        response["service_tier"] = v.clone();
    }
    Ok(response)
}

#[cfg(test)]
#[path = "protocol_parity22_tests.rs"]
mod parity22_tests;

#[cfg(test)]
#[path = "chat_plaintext_tests.rs"]
mod plaintext_tests;
