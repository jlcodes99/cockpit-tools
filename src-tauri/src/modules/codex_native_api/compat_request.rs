use super::{
    invalid, search, signature, unsupported, CompatContext, CompatFlavor, CompatRequest,
    ServiceError,
};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
#[path = "compat_claude_history.rs"]
mod claude_history;
#[path = "compat_schema.rs"]
mod schema;
#[path = "protocol_thinking.rs"]
mod thinking;

pub fn prepare_request(
    body: Value,
    flavor: CompatFlavor,
    model: Option<&str>,
    stream: Option<bool>,
) -> Result<CompatRequest, ServiceError> {
    if !body.is_object() {
        return Err(invalid("body", "expected an object"));
    }
    let model = model
        .or_else(|| body["model"].as_str())
        .filter(|m| !m.is_empty())
        .ok_or_else(|| invalid("model", "a model is required"))?
        .trim_start_matches("models/")
        .to_string();
    let is_stream = match stream {
        Some(s) => s,
        None => match body.get("stream") {
            Some(Value::Bool(s)) => *s,
            None | Some(Value::Null) => flavor == CompatFlavor::Ollama,
            _ => return Err(invalid("stream", "expected a boolean")),
        },
    };
    let mut response = match flavor {
        CompatFlavor::Claude => claude(&body)?,
        CompatFlavor::Gemini => gemini(&body)?,
        CompatFlavor::Ollama => ollama(&body)?,
    };
    thinking::apply(
        &body,
        &mut response,
        match flavor {
            CompatFlavor::Claude => "claude",
            CompatFlavor::Gemini => "gemini",
            CompatFlavor::Ollama => "responses",
        },
    );
    response["model"] = json!(model);
    response["stream"] = json!(is_stream);
    let tool_names = super::tools::map(&mut response);
    Ok(CompatRequest {
        responses: response,
        context: CompatContext {
            flavor,
            model,
            tool_names,
        },
        stream: is_stream,
    })
}
pub fn count_tokens_request(
    mut body: Value,
    flavor: CompatFlavor,
    model: Option<&str>,
) -> Result<Value, ServiceError> {
    // A local text estimate has no generation policy. Stop/candidate/sampling
    // options accepted by native providers must not prevent counting input.
    if let Some(object) = body.as_object_mut() {
        match flavor {
            CompatFlavor::Claude => {
                for key in [
                    "stop_sequences",
                    "top_k",
                    "container",
                    "context_management",
                    "mcp_servers",
                ] {
                    object.remove(key);
                }
            }
            CompatFlavor::Gemini => {
                object.remove("generationConfig");
            }
            CompatFlavor::Ollama => {}
        }
    }
    let mut out = prepare_request(body, flavor, model, Some(false))?.responses;
    let fields = [
        "model",
        "input",
        "instructions",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "text",
        "reasoning",
    ];
    out.as_object_mut()
        .unwrap()
        .retain(|k, _| fields.contains(&k.as_str()));
    Ok(out)
}
fn reject_nonempty(body: &Value, key: &str) -> Result<(), ServiceError> {
    if body
        .get(key)
        .is_some_and(|v| !v.is_null() && !v.as_array().is_some_and(Vec::is_empty))
    {
        Err(unsupported(key))
    } else {
        Ok(())
    }
}
fn text_item(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role=="assistant"{"output_text"}else{"input_text"},"text":text}]})
}
fn message_parts(role: &str, parts: &mut Vec<Value>, input: &mut Vec<Value>) {
    if !parts.is_empty() {
        input.push(json!({"type":"message","role":role,"content":std::mem::take(parts)}));
    }
}
fn required<'a>(body: &'a Value, key: &str) -> Result<&'a str, ServiceError> {
    body[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid(key, "expected nonempty text"))
}

// An explicit result ID may arrive out of order. Consume that exact pending
// call rather than shifting the name queue and binding a later result to it.
fn consume_call(
    calls: &mut HashMap<String, VecDeque<String>>,
    name: Option<&str>,
    explicit_id: Option<&str>,
) -> Option<String> {
    if let Some(id) = explicit_id {
        for queue in calls.values_mut() {
            if let Some(index) = queue.iter().position(|candidate| candidate == id) {
                queue.remove(index);
                break;
            }
        }
        return Some(id.to_owned());
    }
    if let Some(name) = name {
        return calls.get_mut(name).and_then(VecDeque::pop_front);
    }
    if calls.values().map(VecDeque::len).sum::<usize>() == 1 {
        return calls.values_mut().find_map(VecDeque::pop_front);
    }
    None
}
fn source_part(block: &Value, image: bool) -> Result<Value, ServiceError> {
    let source = &block["source"];
    let kind = required(source, "type")?;
    let url = match kind {
        "base64" => format!(
            "data:{};base64,{}",
            required(source, "media_type")?,
            required(source, "data")?
        ),
        "url" => required(source, "url")?.to_string(),
        "text" if !image => {
            return Ok(json!({"type":"input_text","text":required(source,"data")?}))
        }
        _ => return Err(unsupported("content.source.type")),
    };
    if image {
        Ok(json!({"type":"input_image","image_url":url}))
    } else {
        Ok(if kind == "base64" {
            json!({"type":"input_file","file_data":url,"filename":block["title"].as_str().unwrap_or("document.pdf")})
        } else {
            json!({"type":"input_file","file_url":url})
        })
    }
}
fn claude_parts(content: &Value, role: &str, input: &mut Vec<Value>) -> Result<(), ServiceError> {
    if let Some(text) = content.as_str() {
        input.push(text_item(role, text));
        return Ok(());
    }
    let mut parts = Vec::new();
    for block in content
        .as_array()
        .ok_or_else(|| invalid("content", "expected text or content blocks"))?
    {
        match required(block,"type")?{
            "text"=>parts.push(json!({"type":if role=="assistant"{"output_text"}else{"input_text"},"text":block["text"].as_str().ok_or_else(||invalid("text","expected text"))?})),
            "image" if role!="assistant"=>parts.push(source_part(block,true)?),
            "document" if role!="assistant"=>{if block.pointer("/citations/enabled").and_then(Value::as_bool)==Some(true){return Err(unsupported("document.citations"));}parts.push(source_part(block,false)?);},
            "tool_use"=>{message_parts(role,&mut parts,input);let args=block.get("input").filter(|v|v.is_object()).ok_or_else(||invalid("tool_use.input","expected an object"))?;input.push(json!({"type":"function_call","call_id":required(block,"id")?,"name":required(block,"name")?,"arguments":serde_json::to_string(args).unwrap()}));},
            "tool_result"=>{message_parts(role,&mut parts,input);let mut output=Vec::new();match block.get("content"){None=>{},Some(Value::String(s))=>output.push(json!({"type":"input_text","text":s})),Some(Value::Array(blocks))=>for b in blocks{match b["type"].as_str(){Some("text")=>output.push(json!({"type":"input_text","text":b["text"]})),Some("image")=>output.push(source_part(b,true)?),Some("document")=>output.push(source_part(b,false)?),_=>return Err(unsupported("tool_result.content.type"))}},_=>return Err(invalid("tool_result.content","expected text or content blocks"))};if block["is_error"].as_bool()==Some(true){output.insert(0,json!({"type":"input_text","text":"Tool execution failed:"}));}let output=if block["is_error"].as_bool()!=Some(true) { match block.get("content") { None=>json!(""),Some(Value::String(text))=>json!(text),_=>json!(output) } }else{json!(output)};input.push(json!({"type":"function_call_output","call_id":required(block,"tool_use_id")?,"output":output}));},
            "thinking"=>{if role!="assistant"{return Err(invalid("thinking","expected assistant history"));}message_parts(role,&mut parts,input);let mut item=json!({"type":"reasoning","summary":[{"type":"summary_text","text":block["thinking"].as_str().unwrap_or("")} ]});if let Some(value)=signature::input(block.get("signature"))?{item["encrypted_content"]=json!(value);}input.push(item);},
            "server_tool_use"|"web_search_tool_result" if role=="assistant"=>{message_parts(role,&mut parts,input);search::history(block,input)?;},
            _=>return Err(unsupported("content.type")),
        }
    }
    message_parts(role, &mut parts, input);
    Ok(())
}
fn claude(body: &Value) -> Result<Value, ServiceError> {
    for k in [
        "stop_sequences",
        "top_k",
        "container",
        "context_management",
        "mcp_servers",
    ] {
        reject_nonempty(body, k)?;
    }
    let mut input = Vec::new();
    if let Some(system) = body.get("system").filter(|v| !v.is_null()) {
        claude_parts(system, "developer", &mut input)?;
    }
    let messages = claude_history::normalize(
        body["messages"]
            .as_array()
            .ok_or_else(|| invalid("messages", "expected an array"))?,
    );
    for message in &messages {
        let role = required(message, "role")?;
        if !matches!(role, "user" | "assistant") {
            return Err(invalid("messages.role", "expected user or assistant"));
        }
        claude_parts(&message["content"], role, &mut input)?;
    }
    let mut out = json!({"input":input});
    for (from, to) in [
        ("max_tokens", "max_output_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
    ] {
        if let Some(v) = body.get(from) {
            out[to] = v.clone();
        }
    }
    if let Some(tools) = body.get("tools") {
        let mut mapped = Vec::new();
        for tool in tools
            .as_array()
            .ok_or_else(|| invalid("tools", "expected an array"))?
        {
            if search::is_claude_tool(tool) {
                mapped.push(search::claude_tool(tool)?);
                continue;
            }
            if tool["type"].as_str().is_some_and(|s| s != "custom") {
                return Err(unsupported("tools.type"));
            }
            let mut t = json!({"type":"function","name":required(tool,"name")?,"parameters":tool.get("input_schema").cloned().unwrap_or(json!({"type":"object","properties":{}})),"strict":false});
            if let Some(d) = tool.get("description") {
                t["description"] = d.clone();
            }
            mapped.push(t);
        }
        out["tools"] = json!(mapped);
    }
    if let Some(choice) = body.get("tool_choice") {
        out["tool_choice"] = match choice["type"].as_str() {
            Some("auto") => json!("auto"),
            Some("any") => json!("required"),
            Some("none") => json!("none"),
            Some("tool") => {
                let name = required(choice, "name")?;
                if body["tools"].as_array().is_some_and(|tools| {
                    tools
                        .iter()
                        .any(|tool| search::is_claude_tool(tool) && tool["name"] == name)
                }) {
                    json!({"type":"web_search"})
                } else {
                    json!({"type":"function","name":name})
                }
            }
            _ => return Err(invalid("tool_choice.type", "unsupported tool choice")),
        };
        if let Some(v) = choice
            .get("disable_parallel_tool_use")
            .and_then(Value::as_bool)
        {
            out["parallel_tool_calls"] = json!(!v);
        }
    }
    if let Some(thinking) = body.get("thinking") {
        match thinking["type"].as_str() {
            Some("disabled") => out["reasoning"] = json!({"effort":"none"}),
            Some("adaptive" | "auto") => out["reasoning"] = json!({"effort":"xhigh"}),
            Some("enabled") => {
                let effort = thinking
                    .get("budget_tokens")
                    .map(reasoning_budget)
                    .transpose()?;
                out["reasoning"] = json!({"effort":"medium"});
                if let Some(effort) = effort {
                    out["reasoning"]["effort"] = json!(effort);
                }
            }
            _ => return Err(invalid("thinking.type", "unsupported thinking mode")),
        }
    }
    if let Some(effort) = body.pointer("/output_config/effort") {
        if out.get("reasoning").is_none() {
            out["reasoning"] = json!({});
        }
        out["reasoning"]["effort"] = effort
            .as_str()
            .map(|value| json!(value.trim().to_ascii_lowercase()))
            .unwrap_or_else(|| effort.clone());
    }
    if let Some(format) = body
        .pointer("/output_config/format")
        .or_else(|| body.get("output_format"))
    {
        if format["type"] != "json_schema" {
            return Err(unsupported("output_config.format"));
        }
        out["text"] = json!({"format":{"type":"json_schema","name":format["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("cli_proxy_structured_output"),"schema":format["schema"],"strict":format["strict"] != false && !schema::misses_required(&format["schema"])}});
    }
    let tier = if body["speed"] == "fast" {
        Some("priority")
    } else {
        normalized_tier(body.get("service_tier"))
    };
    if let Some(tier) = tier {
        out["service_tier"] = json!(tier);
    }
    Ok(out)
}
fn normalized_tier(value: Option<&Value>) -> Option<&'static str> {
    match value
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("priority" | "fast") => Some("priority"),
        _ => None,
    }
}
fn media_part(mime: &str, data: &str, inline: bool) -> Result<Value, ServiceError> {
    let url = if inline {
        format!("data:{mime};base64,{data}")
    } else {
        data.to_string()
    };
    if mime.starts_with("image/") {
        Ok(json!({"type":"input_image","image_url":url}))
    } else if mime.starts_with("audio/") {
        Ok(json!({"type":"input_audio","audio_url":url}))
    } else if mime == "application/pdf" || mime.starts_with("text/") {
        Ok(if inline {
            json!({"type":"input_file","file_data":url,"filename":if mime=="application/pdf"{"document.pdf"}else{"document.txt"}})
        } else {
            json!({"type":"input_file","file_url":url})
        })
    } else {
        Err(unsupported("contents.parts.media_type"))
    }
}
fn gemini(body: &Value) -> Result<Value, ServiceError> {
    for key in ["safetySettings", "cachedContent"] {
        reject_nonempty(body, key)?;
    }
    let mut input = Vec::new();
    let mut calls: HashMap<String, VecDeque<String>> = HashMap::new();
    if let Some(system) = body
        .get("systemInstruction")
        .or_else(|| body.get("system_instruction"))
    {
        let text = system["parts"]
            .as_array()
            .ok_or_else(|| invalid("systemInstruction.parts", "expected an array"))?
            .iter()
            .map(|p| {
                p["text"]
                    .as_str()
                    .ok_or_else(|| invalid("systemInstruction.parts", "expected text"))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        input.push(text_item("developer", &text));
    }
    for content in body["contents"]
        .as_array()
        .ok_or_else(|| invalid("contents", "expected an array"))?
    {
        let role = match content["role"].as_str().unwrap_or("user") {
            "user" => "user",
            "model" => "assistant",
            _ => return Err(invalid("contents.role", "expected user or model")),
        };
        let mut parts = Vec::new();
        for part in content["parts"]
            .as_array()
            .ok_or_else(|| invalid("contents.parts", "expected an array"))?
        {
            // Gemini's replay signature is not Codex encrypted continuation state.
            // The Go converter excludes historical thoughts and ignores their signature.
            if part["thought"].as_bool() == Some(true) {
                continue;
            }
            if let Some(text) = part["text"].as_str() {
                parts.push(json!({"type":if role=="assistant"{"output_text"}else{"input_text"},"text":text}));
            } else if let Some(media) = part.get("inlineData").or_else(|| part.get("inline_data")) {
                parts.push(media_part(
                    media["mimeType"]
                        .as_str()
                        .or_else(|| media["mime_type"].as_str())
                        .ok_or_else(|| invalid("mimeType", "missing MIME type"))?,
                    required(media, "data")?,
                    true,
                )?);
            } else if let Some(media) = part.get("fileData").or_else(|| part.get("file_data")) {
                parts.push(media_part(
                    media
                        .get("mimeType")
                        .or_else(|| media.get("mime_type"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("mimeType", "missing MIME type"))?,
                    media
                        .get("fileUri")
                        .or_else(|| media.get("file_uri"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| invalid("fileUri", "missing file URI"))?,
                    false,
                )?);
            } else if let Some(call) = part.get("functionCall") {
                message_parts(role, &mut parts, &mut input);
                let name = required(call, "name")?;
                let id = call["id"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| call["call_id"].as_str().filter(|s| !s.trim().is_empty()))
                    .map(str::trim)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::now_v7()));
                calls
                    .entry(name.to_string())
                    .or_default()
                    .push_back(id.clone());
                let args = call.get("args").cloned().unwrap_or(json!({}));
                if !args.is_object() {
                    return Err(invalid("functionCall.args", "expected an object"));
                }
                input.push(json!({"type":"function_call","call_id":id,"name":name,"arguments":args.to_string()}));
            } else if let Some(result) = part.get("functionResponse") {
                message_parts(role, &mut parts, &mut input);
                let name = required(result, "name")?;
                let id = consume_call(
                    &mut calls,
                    Some(name),
                    result["id"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| result["call_id"].as_str().filter(|s| !s.trim().is_empty()))
                        .map(str::trim),
                )
                .ok_or_else(|| {
                    invalid("functionResponse", "cannot match a preceding function call")
                })?;
                input.push(json!({"type":"function_call_output","call_id":id,"output":result.get("response").cloned().unwrap_or(json!({})).to_string()}));
            } else {
                return Err(unsupported("contents.parts"));
            }
        }
        message_parts(role, &mut parts, &mut input);
    }
    let mut out = json!({"input":input});
    let config = &body["generationConfig"];
    for key in [
        "stopSequences",
        "topK",
        "responseLogprobs",
        "logprobs",
        "audioTimestamp",
        "mediaResolution",
        "speechConfig",
    ] {
        reject_nonempty(config, key)?;
    }
    if config
        .get("candidateCount")
        .is_some_and(|v| v.as_u64() != Some(1))
    {
        return Err(unsupported("candidateCount"));
    }
    if config
        .get("responseModalities")
        .is_some_and(|v| v != &json!(["TEXT"]))
    {
        return Err(unsupported("responseModalities"));
    }
    for (from, to) in [
        ("temperature", "temperature"),
        ("topP", "top_p"),
        ("maxOutputTokens", "max_output_tokens"),
    ] {
        if let Some(v) = config.get(from) {
            out[to] = v.clone();
        }
    }
    if config["responseMimeType"] == "application/json" {
        let schema = config
            .get("responseJsonSchema")
            .or_else(|| config.get("responseSchema"));
        out["text"] = if let Some(schema) = schema {
            json!({"format":{"type":"json_schema","name":"response","schema":normalize_schema(schema),"strict":true}})
        } else {
            json!({"format":{"type":"json_object"}})
        };
    } else if config
        .get("responseMimeType")
        .is_some_and(|v| v != "text/plain")
    {
        return Err(unsupported("responseMimeType"));
    }
    if let Some(thinking) = config
        .get("thinkingConfig")
        .or_else(|| config.get("thinking_config"))
    {
        if let Some(budget) = thinking
            .get("thinkingBudget")
            .or_else(|| thinking.get("thinking_budget"))
        {
            match budget.as_i64() {
                Some(0) => out["reasoning"] = json!({"effort":"none"}),
                Some(-1) => out["reasoning"] = json!({"effort":"auto"}),
                Some(value) if value > 0 => {
                    out["reasoning"] = json!({"effort":reasoning_budget(budget)?})
                }
                _ => {
                    return Err(invalid(
                        "thinkingBudget",
                        "expected -1 or a nonnegative integer",
                    ))
                }
            }
        }
        if let Some(level) = thinking
            .get("thinkingLevel")
            .or_else(|| thinking.get("thinking_level"))
            .and_then(Value::as_str)
        {
            out["reasoning"] = json!({"effort":level.trim().to_ascii_lowercase()});
        }
        if thinking["includeThoughts"].as_bool() == Some(true) {
            if out.get("reasoning").is_none() {
                out["reasoning"] = json!({});
            }
            out["reasoning"]["summary"] = json!("auto");
        }
    }
    if let Some(level) = config
        .get("thinkingLevel")
        .or_else(|| config.get("thinking_level"))
        .and_then(Value::as_str)
    {
        out["reasoning"] = json!({"effort":level.trim().to_ascii_lowercase()});
    }
    if let Some(tier) = normalized_tier(body.get("service_tier")) {
        out["service_tier"] = json!(tier);
    }
    let mut tools = Vec::new();
    if let Some(items) = body["tools"].as_array() {
        for t in items {
            if let Some(functions) = t["functionDeclarations"].as_array() {
                for f in functions {
                    let mut tool = json!({"type":"function","name":required(f,"name")?,"parameters":normalize_schema(f.get("parametersJsonSchema").or_else(||f.get("parameters")).unwrap_or(&json!({"type":"object","properties":{}}))),"strict":false});
                    if let Some(d) = f.get("description") {
                        tool["description"] = d.clone();
                    }
                    tools.push(tool);
                }
            } else if t.get("googleSearch").is_some() {
                tools.push(json!({"type":"web_search"}));
            } else {
                return Err(unsupported("tools"));
            }
        }
    }
    if !tools.is_empty() {
        out["tools"] = json!(tools);
    }
    if let Some(choice) = body.pointer("/toolConfig/functionCallingConfig") {
        let mode = match choice["mode"].as_str().unwrap_or("AUTO") {
            "AUTO" | "MODE_UNSPECIFIED" => "auto",
            "NONE" => "none",
            "ANY" => "required",
            _ => return Err(unsupported("functionCallingConfig.mode")),
        };
        out["tool_choice"] = json!(mode);
        if let Some(names) = choice["allowedFunctionNames"]
            .as_array()
            .filter(|v| !v.is_empty())
        {
            out["tool_choice"] = json!({"type":"allowed_tools","mode":mode,"tools":names.iter().map(|n|json!({"type":"function","name":n})).collect::<Vec<_>>()});
        }
    }
    Ok(out)
}
fn normalize_schema(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = map.clone();
            for (k, val) in &mut out {
                *val = if k == "type" {
                    val.as_str()
                        .map(|s| json!(s.to_lowercase()))
                        .unwrap_or_else(|| normalize_schema(val))
                } else {
                    normalize_schema(val)
                };
            }
            if out.remove("nullable") == Some(json!(true)) {
                if let Some(kind) = out.get("type").cloned() {
                    out.insert("type".into(), json!([kind, "null"]));
                }
            }
            out.remove("propertyOrdering");
            json!(out)
        }
        Value::Array(a) => json!(a.iter().map(normalize_schema).collect::<Vec<_>>()),
        _ => v.clone(),
    }
}
fn ollama(body: &Value) -> Result<Value, ServiceError> {
    let mut chat = json!({"model":body["model"],"messages":[],"stream":body.get("stream").cloned().unwrap_or(json!(true))});
    let mut messages = Vec::new();
    let mut calls: HashMap<String, VecDeque<String>> = HashMap::new();
    for message in body["messages"]
        .as_array()
        .ok_or_else(|| invalid("messages", "expected an array"))?
    {
        let mut m = message.clone();
        if let Some(images) = message.get("images") {
            let mut content =
                vec![json!({"type":"text","text":message["content"].as_str().unwrap_or("")})];
            for image in images
                .as_array()
                .ok_or_else(|| invalid("images", "expected an array"))?
            {
                let data = image
                    .as_str()
                    .ok_or_else(|| invalid("images", "expected base64 strings"))?;
                let mime = if data.starts_with("iVBOR") {
                    "image/png"
                } else if data.starts_with("R0lG") {
                    "image/gif"
                } else if data.starts_with("UklG") {
                    "image/webp"
                } else {
                    "image/jpeg"
                };
                content.push(json!({"type":"image_url","image_url":{"url":format!("data:{mime};base64,{data}")}}));
            }
            m["content"] = json!(content);
        }
        if let Some(thinking) = message.get("thinking") {
            m["reasoning_content"] = thinking.clone();
        }
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            let mut mapped = Vec::new();
            for call in tool_calls {
                let name = required(&call["function"], "name")?;
                let id = call["id"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::now_v7()));
                calls
                    .entry(name.to_string())
                    .or_default()
                    .push_back(id.clone());
                mapped.push(json!({"id":id,"type":"function","function":{"name":name,"arguments":match call["function"].get("arguments") { None | Some(Value::Null) => "{}".to_owned(), Some(Value::String(s)) => s.clone(), Some(value) => value.to_string() }}}));
            }
            m["tool_calls"] = json!(mapped);
        }
        if message["role"] == "tool" {
            let name = message["tool_name"]
                .as_str()
                .or_else(|| message["name"].as_str());
            let id = consume_call(&mut calls, name, message["tool_call_id"].as_str())
                .ok_or_else(|| invalid("tool_name", "cannot match a preceding tool call"))?;
            m["tool_call_id"] = json!(id);
            m.as_object_mut().unwrap().remove("name");
        }
        messages.push(m);
    }
    chat["messages"] = json!(messages);
    if let Some(tools) = body.get("tools") {
        chat["tools"] = tools.clone();
    }
    if let Some(format) = body
        .get("format")
        .filter(|v| !v.is_null() && v != &&json!(""))
    {
        chat["response_format"] = if format == "json" {
            json!({"type":"json_object"})
        } else if format.is_object() {
            json!({"type":"json_schema","json_schema":{"name":"response","schema":format,"strict":true}})
        } else {
            return Err(invalid("format", "expected json or a JSON schema"));
        };
    }
    if let Some(options) = body.get("options") {
        for (k, v) in options
            .as_object()
            .ok_or_else(|| invalid("options", "expected an object"))?
        {
            match k.as_str() {
                "temperature" | "top_p" => chat[k] = v.clone(),
                "num_predict" if v.as_i64().is_some_and(|n| n > 0) => {
                    chat["max_tokens"] = v.clone()
                }
                "num_predict" if v == &json!(-1) => {}
                _ => return Err(unsupported(&format!("options.{k}"))),
            }
        }
    }
    if let Some(think) = body.get("think").filter(|v| !v.is_null()) {
        chat["reasoning_effort"] = match think {
            Value::Bool(false) => json!("none"),
            Value::Bool(true) => json!("medium"),
            Value::String(s)
                if matches!(
                    s.as_str(),
                    "low" | "medium" | "high" | "minimal" | "none" | "xhigh" | "max"
                ) =>
            {
                json!(s)
            }
            _ => return Err(invalid("think", "expected boolean or reasoning effort")),
        };
    }
    for key in ["logprobs", "top_logprobs"] {
        if let Some(v) = body.get(key) {
            chat[key] = v.clone();
        }
    }
    Ok(
        super::super::chat::prepare_request(chat, super::super::chat::ChatFlavor::ChatCompletions)?
            .responses,
    )
}

// Responses exposes effort, not an exact thinking-token cap. Match the intent
// of compatibility clients without claiming an exact upstream token budget.
fn reasoning_budget(value: &Value) -> Result<&'static str, ServiceError> {
    value
        .as_i64()
        .and_then(thinking::budget)
        .ok_or_else(|| invalid("thinking budget", "expected -1 or a nonnegative integer"))
}
