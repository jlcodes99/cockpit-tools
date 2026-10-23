use super::{incompatible, invalid, ChatContext, ChatFlavor, ChatRequest, ServiceError};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
#[path = "protocol_thinking.rs"]
mod thinking;

pub fn prepare_request(body: Value, flavor: ChatFlavor) -> Result<ChatRequest, ServiceError> {
    let object = body
        .as_object()
        .ok_or_else(|| invalid("body", "expected a JSON object"))?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("model", "a nonempty model is required"))?
        .to_string();
    if let Some(n) = body.get("n").filter(|v| !v.is_null()) {
        if n.as_u64() != Some(1) {
            return Err(incompatible(
                "n",
                "Responses generates one candidate per request",
            ));
        }
    }
    for field in [
        "stop",
        "seed",
        "logit_bias",
        "prediction",
        "audio",
        "suffix",
    ] {
        if body
            .get(field)
            .is_some_and(|v| !v.is_null() && !v.as_object().is_some_and(Map::is_empty))
        {
            return Err(incompatible(
                field,
                "this option has no equivalent Responses generation setting",
            ));
        }
    }
    for field in ["presence_penalty", "frequency_penalty"] {
        if body
            .get(field)
            .is_some_and(|v| !v.is_null() && v.as_f64() != Some(0.0))
        {
            return Err(incompatible(
                field,
                "nonzero penalties have no equivalent Responses setting",
            ));
        }
    }
    if body
        .get("modalities")
        .is_some_and(|v| !v.is_null() && v != &json!(["text"]))
    {
        return Err(incompatible(
            "modalities",
            "audio output requires a native audio upstream",
        ));
    }
    let stream = match body.get("stream") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(invalid("stream", "expected a boolean")),
    };
    let include_usage = body
        .pointer("/stream_options/include_usage")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let legacy_functions = body.get("functions").is_some() || body.get("function_call").is_some();
    let mut response = object.clone();
    for field in [
        "messages",
        "prompt",
        "n",
        "stream_options",
        "functions",
        "function_call",
        "max_tokens",
        "max_completion_tokens",
        "response_format",
        "reasoning_effort",
        "logprobs",
        "presence_penalty",
        "frequency_penalty",
        "modalities",
        "echo",
        "best_of",
        "stop",
        "seed",
        "logit_bias",
        "prediction",
        "audio",
        "suffix",
        "web_search_options",
    ] {
        response.remove(field);
    }
    let messages = if flavor == ChatFlavor::Completions {
        if body.get("echo").and_then(Value::as_bool) == Some(true) {
            return Err(incompatible(
                "echo",
                "prompt echo cannot preserve token probabilities through Responses",
            ));
        }
        if body
            .get("best_of")
            .is_some_and(|v| !v.is_null() && v.as_u64() != Some(1))
        {
            return Err(incompatible(
                "best_of",
                "multiple sampled candidates require separate requests",
            ));
        }
        if body.get("logprobs").is_some_and(|v| !v.is_null()) {
            return Err(incompatible(
                "logprobs",
                "legacy token offsets cannot be represented without the upstream tokenizer",
            ));
        }
        let prompt = match body.get("prompt") {
            Some(Value::String(value)) => value.clone(),
            Some(Value::Array(values)) if values.len() == 1 && values[0].is_string() => values[0].as_str().unwrap().to_string(),
            _ => return Err(invalid("prompt", "expected one text prompt; token IDs and prompt batches require a tokenizer or separate requests")),
        };
        vec![json!({"role":"user","content":prompt})]
    } else {
        body.get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("messages", "expected an array"))?
            .clone()
    };
    let custom_names = custom_tool_names(&body);
    response.insert(
        "input".into(),
        Value::Array(convert_messages(&messages, &custom_names)?),
    );
    response.insert("stream".into(), Value::Bool(stream));
    if let Some(limit) = body
        .get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .filter(|v| !v.is_null())
    {
        if !limit.as_u64().is_some_and(|v| v > 0) {
            return Err(invalid(
                "max_completion_tokens",
                "expected a positive integer",
            ));
        }
        response.insert("max_output_tokens".into(), limit.clone());
    }
    if let Some(effort) = body.get("reasoning_effort").filter(|v| !v.is_null()) {
        response
            .entry("reasoning")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| invalid("reasoning", "expected an object"))?
            .insert("effort".into(), effort.clone());
    }
    if let Some(format) = body.get("response_format").filter(|v| !v.is_null()) {
        let kind = format
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("response_format.type", "expected a format type"))?;
        let format = match kind {
            "text" | "json_object" => format.clone(),
            "json_schema" => {
                let mut schema = format
                    .get("json_schema")
                    .and_then(Value::as_object)
                    .ok_or_else(|| invalid("response_format.json_schema", "expected an object"))?
                    .clone();
                schema.insert("type".into(), json!("json_schema"));
                Value::Object(schema)
            }
            _ => {
                return Err(invalid(
                    "response_format.type",
                    "unsupported structured output format",
                ))
            }
        };
        response
            .entry("text")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| invalid("text", "expected an object"))?
            .insert("format".into(), format);
    }
    let mut tools = Vec::new();
    if let Some(items) = body.get("tools").filter(|v| !v.is_null()) {
        for tool in items
            .as_array()
            .ok_or_else(|| invalid("tools", "expected an array"))?
        {
            tools.push(convert_tool(tool)?);
        }
    }
    if let Some(functions) = body.get("functions").filter(|v| !v.is_null()) {
        for function in functions
            .as_array()
            .ok_or_else(|| invalid("functions", "expected an array"))?
        {
            tools.push(convert_tool(
                &json!({"type":"function","function":function}),
            )?);
        }
    }
    if let Some(search) = body.get("web_search_options").filter(|v| !v.is_null()) {
        let mut search = search
            .as_object()
            .ok_or_else(|| invalid("web_search_options", "expected an object"))?
            .clone();
        search.insert("type".into(), json!("web_search"));
        tools.push(Value::Object(search));
    }
    if !tools.is_empty() {
        response.insert("tools".into(), Value::Array(tools));
    }
    if legacy_functions {
        if body.get("parallel_tool_calls").and_then(Value::as_bool) == Some(true) {
            return Err(incompatible(
                "parallel_tool_calls",
                "deprecated function_call supports one call per response",
            ));
        }
        response.insert("parallel_tool_calls".into(), json!(false));
    }
    if let Some(verbosity) = response.remove("verbosity") {
        response
            .entry("text")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| invalid("text", "expected an object"))?
            .insert("verbosity".into(), verbosity);
    }
    if let Some(choice) = body
        .get("tool_choice")
        .or_else(|| body.get("function_call"))
        .filter(|v| !v.is_null())
    {
        let mut choice = convert_choice(choice, legacy_functions)?;
        if choice["type"] == "function"
            && choice["name"]
                .as_str()
                .is_some_and(|name| custom_names.contains(name))
        {
            choice["type"] = json!("custom");
        }
        response.insert("tool_choice".into(), choice);
    }
    if body.get("logprobs").and_then(Value::as_bool) == Some(true) {
        let includes = response
            .entry("include")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| invalid("include", "expected an array"))?;
        if !includes.iter().any(|v| v == "message.output_text.logprobs") {
            includes.push(json!("message.output_text.logprobs"));
        }
    }
    let mut responses = Value::Object(response);
    thinking::apply(&body, &mut responses, "chat");
    let tool_names = super::tools::map(&mut responses);
    Ok(ChatRequest {
        responses,
        stream,
        context: ChatContext {
            flavor,
            model,
            include_usage,
            legacy_functions,
            tool_names,
        },
    })
}

fn custom_tool_names(body: &Value) -> HashSet<String> {
    let mut custom = HashSet::new();
    let mut functions = HashSet::new();
    for tool in body["tools"].as_array().into_iter().flatten() {
        match tool["type"].as_str() {
            Some("custom") => {
                if let Some(name) = tool["name"]
                    .as_str()
                    .or_else(|| tool.pointer("/custom/name").and_then(Value::as_str))
                {
                    custom.insert(name.to_owned());
                }
            }
            Some("function") => {
                if let Some(name) = tool.pointer("/function/name").and_then(Value::as_str) {
                    functions.insert(name.to_owned());
                }
            }
            _ => {}
        }
    }
    custom.retain(|name| !functions.contains(name));
    custom
}

fn convert_messages(
    messages: &[Value],
    custom_names: &HashSet<String>,
) -> Result<Vec<Value>, ServiceError> {
    let mut input = Vec::new();
    let mut legacy_calls = HashMap::<String, String>::new();
    let mut call_types = HashMap::<String, String>::new();
    for (index, message) in messages.iter().enumerate() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid(&format!("messages[{index}].role"), "a role is required"))?;
        match role {
            "tool" | "function" => {
                let call_id = if role == "tool" {
                    message
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                } else {
                    message
                        .get("name")
                        .and_then(Value::as_str)
                        .and_then(|name| legacy_calls.get(name))
                        .cloned()
                }
                .ok_or_else(|| {
                    invalid("tool_call_id", "tool results must refer to an earlier call")
                })?;
                let output = match message.get("content") {
                    Some(Value::String(value)) => Value::String(value.clone()),
                    Some(value @ Value::Array(_)) => Value::Array(convert_content(value, false)?),
                    _ => {
                        return Err(invalid(
                            "messages.content",
                            "tool output must be text or content parts",
                        ))
                    }
                };
                let kind = if call_types.get(&call_id).is_some_and(|v| v == "custom") {
                    "custom_tool_call_output"
                } else {
                    "function_call_output"
                };
                input.push(json!({"type":kind,"call_id":call_id,"output":output}));
            }
            "system" | "developer" | "user" | "assistant" => {
                if message.get("audio").is_some_and(|v| !v.is_null()) {
                    return Err(incompatible(
                        "messages.audio",
                        "referenced audio output requires the original audio service",
                    ));
                }
                if message.get("name").is_some_and(|v| !v.is_null()) {
                    return Err(incompatible(
                        "messages.name",
                        "named participants have no typed Responses item equivalent",
                    ));
                }
                let mut content = message
                    .get("content")
                    .filter(|v| !v.is_null())
                    .map(|v| convert_content(v, role == "assistant"))
                    .transpose()?
                    .unwrap_or_default();
                if let Some(refusal) = message
                    .get("refusal")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    content.push(json!({"type":"refusal","refusal":refusal}));
                }
                if let Some(details) = message.get("reasoning_details").and_then(Value::as_array) {
                    for item in details {
                        if item.get("type").and_then(Value::as_str) != Some("reasoning") {
                            return Err(incompatible(
                                "messages.reasoning_details",
                                "expected Responses reasoning items",
                            ));
                        }
                        input.push(item.clone());
                    }
                } else if let Some(text) = message
                    .get("reasoning_content")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    input.push(
                        json!({"type":"reasoning","summary":[{"type":"summary_text","text":text}]}),
                    );
                }
                if !content.is_empty()
                    || message.get("tool_calls").is_none() && message.get("function_call").is_none()
                {
                    input.push(json!({"type":"message","role":role,"content":content}));
                }
                if let Some(calls) = message.get("tool_calls").filter(|v| !v.is_null()) {
                    for call in calls
                        .as_array()
                        .ok_or_else(|| invalid("messages.tool_calls", "expected an array"))?
                    {
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|v| !v.is_empty())
                            .ok_or_else(|| invalid("tool_calls.id", "a call ID is required"))?;
                        let mut converted = convert_call(call, id)?;
                        if converted["type"] == "function_call"
                            && converted["name"]
                                .as_str()
                                .is_some_and(|name| custom_names.contains(name))
                        {
                            converted["type"] = json!("custom_tool_call");
                            let arguments = converted
                                .as_object_mut()
                                .unwrap()
                                .remove("arguments")
                                .unwrap();
                            converted["input"] = arguments;
                        }
                        call_types.insert(
                            id.to_string(),
                            if converted["type"] == "custom_tool_call" {
                                "custom"
                            } else {
                                "function"
                            }
                            .to_owned(),
                        );
                        input.push(converted);
                    }
                }
                if let Some(call) = message.get("function_call").filter(|v| !v.is_null()) {
                    let name = call
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("function_call.name", "a name is required"))?;
                    let id = format!("call_{}", uuid::Uuid::now_v7());
                    legacy_calls.insert(name.to_string(), id.clone());
                    input.push(convert_call(
                        &json!({"type":"function","function":call}),
                        &id,
                    )?);
                }
            }
            _ => return Err(invalid("messages.role", "unsupported message role")),
        }
    }
    Ok(input)
}

fn convert_call(call: &Value, id: &str) -> Result<Value, ServiceError> {
    let kind = call
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    let details = call
        .get(kind)
        .ok_or_else(|| invalid("tool_calls", "missing call details"))?;
    let name = details
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("tool_calls.name", "a name is required"))?;
    match kind {
        "function" => Ok(
            json!({"type":"function_call","call_id":id,"name":name,"arguments":details.get("arguments").and_then(Value::as_str).ok_or_else(|| invalid("tool_calls.arguments", "arguments must remain a JSON string"))?}),
        ),
        "custom" => Ok(
            json!({"type":"custom_tool_call","call_id":id,"name":name,"input":details.get("input").and_then(Value::as_str).ok_or_else(|| invalid("tool_calls.input", "custom input must be text"))?}),
        ),
        _ => Err(invalid("tool_calls.type", "unsupported tool call type")),
    }
}

fn convert_content(content: &Value, assistant: bool) -> Result<Vec<Value>, ServiceError> {
    if let Some(text) = content.as_str() {
        return Ok(vec![
            json!({"type":if assistant {"output_text"} else {"input_text"},"text":text}),
        ]);
    }
    let parts = content
        .as_array()
        .ok_or_else(|| invalid("messages.content", "expected text or an array"))?;
    let mut output = Vec::new();
    for part in parts {
        let mut converted = match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                json!({"type":if assistant {"output_text"} else {"input_text"},"text":part.get("text").and_then(Value::as_str).ok_or_else(|| invalid("content.text", "expected text"))?})
            }
            Some("refusal") if assistant => {
                json!({"type":"refusal","refusal":part.get("refusal").and_then(Value::as_str).ok_or_else(|| invalid("content.refusal", "expected text"))?})
            }
            Some("image_url") if !assistant => {
                let image = part
                    .get("image_url")
                    .ok_or_else(|| invalid("content.image_url", "image details are required"))?;
                let mut value = json!({"type":"input_image","image_url":image.get("url").and_then(Value::as_str).ok_or_else(|| invalid("content.image_url.url", "an image URL is required"))?});
                if let Some(detail) = image.get("detail") {
                    value["detail"] = detail.clone();
                }
                value
            }
            Some("file") if !assistant => {
                let mut file = part
                    .get("file")
                    .and_then(Value::as_object)
                    .ok_or_else(|| invalid("content.file", "file details are required"))?
                    .clone();
                file.insert("type".into(), json!("input_file"));
                Value::Object(file)
            }
            Some("input_audio") if !assistant => {
                let audio = part
                    .get("input_audio")
                    .ok_or_else(|| invalid("content.input_audio", "audio details are required"))?;
                let data = audio
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("input_audio.data", "base64 audio is required"))?;
                let format = audio
                    .get("format")
                    .and_then(Value::as_str)
                    .filter(|v| matches!(*v, "mp3" | "wav"))
                    .ok_or_else(|| invalid("input_audio.format", "expected mp3 or wav"))?;
                json!({"type":"input_audio","audio_url":format!("data:audio/{format};base64,{data}")})
            }
            _ => {
                return Err(incompatible(
                    "messages.content.type",
                    "this content part has no lossless Responses representation",
                ))
            }
        };
        if let Some(boundary) = part.get("prompt_cache_breakpoint") {
            converted["prompt_cache_breakpoint"] = boundary.clone();
        }
        output.push(converted);
    }
    Ok(output)
}

fn convert_tool(tool: &Value) -> Result<Value, ServiceError> {
    let kind = tool
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("tools.type", "tool type is required"))?;
    if kind.is_empty() {
        return Err(invalid("tools.type", "tool type is required"));
    }
    // Codex's Chat edge accepts Responses built-ins directly. Custom declarations
    // also use the flat Responses envelope; keep the older nested form as an extension.
    if kind != "function" && kind != "custom" {
        return Ok(tool.clone());
    }
    if kind == "custom" && tool.get("custom").is_none() {
        return Ok(tool.clone());
    }
    let mut details = tool
        .get(kind)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("tools", "tool details are required"))?
        .clone();
    details.insert("type".into(), json!(kind));
    if kind == "function" {
        details.entry("strict").or_insert(json!(false));
    }
    Ok(Value::Object(details))
}

fn convert_choice(choice: &Value, legacy: bool) -> Result<Value, ServiceError> {
    if choice.is_string() {
        return Ok(choice.clone());
    }
    let kind = choice
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(if legacy { "function" } else { "" });
    if matches!(kind, "function" | "custom") {
        let name = choice
            .get(kind)
            .unwrap_or(choice)
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("tool_choice", "a tool name is required"))?;
        return Ok(json!({"type":kind,"name":name}));
    }
    if kind == "allowed_tools" {
        if choice.get("allowed_tools").is_none() {
            return Ok(choice.clone());
        }
        let details = choice.get("allowed_tools").unwrap_or(choice);
        let tools = details
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("tool_choice.allowed_tools", "tools are required"))?;
        let converted: Result<Vec<_>, _> = tools
            .iter()
            .map(|tool| convert_choice(tool, false))
            .collect();
        return Ok(
            json!({"type":"allowed_tools","mode":details.get("mode").cloned().unwrap_or(json!("auto")),"tools":converted?}),
        );
    }
    if !kind.is_empty() {
        Ok(choice.clone())
    } else {
        Err(invalid("tool_choice", "unsupported choice structure"))
    }
}

#[cfg(test)]
#[path = "chat_tools_parity_tests.rs"]
mod tools_parity_tests;
#[cfg(test)]
#[path = "chat_tools_roundtrip_tests.rs"]
mod tools_roundtrip_tests;
