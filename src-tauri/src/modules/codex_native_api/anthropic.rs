//! Direct Responses <-> Anthropic Messages adaptation. Credentials and routing
//! remain with the host; tools are executed by the requesting client.
use super::{chat_upstream::ToolContext, contract::ServiceError};
use serde_json::{json, Value};

#[path = "anthropic_stream.rs"]
mod stream;
pub(super) use stream::ResponsesStreamAdapter;

pub(crate) const VERSION: &str = "2023-06-01";

fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "anthropic_request_invalid", message)
}
fn upstream(message: &str) -> ServiceError {
    ServiceError::new(502, "anthropic_response_invalid", message)
}

/// Accept root, /v1, and explicit Messages URLs, preserving gateway prefixes.
pub(crate) fn endpoint(base: &str, operation: &str) -> Result<String, ServiceError> {
    let mut url =
        reqwest::Url::parse(base.trim()).map_err(|_| invalid("Invalid Anthropic Base URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(invalid("Anthropic Base URL must use HTTP or HTTPS"));
    }
    let path = url.path().trim_end_matches('/');
    let root = path.strip_suffix("/messages").unwrap_or(path);
    let root = if root.ends_with("/v1") {
        root.to_owned()
    } else {
        format!("{root}/v1")
    };
    url.set_path(&format!("{root}/{operation}"));
    Ok(url.to_string())
}

fn parts(value: &Value) -> Result<Vec<Value>, ServiceError> {
    if let Some(text) = value.as_str() {
        return Ok(vec![json!({"type":"text","text":text})]);
    }
    let values = value
        .as_array()
        .ok_or_else(|| invalid("Message content must be text or an array"))?;
    values
        .iter()
        .map(|part| match part["type"].as_str().unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                let mut block = json!({"type":"text","text":part["text"].as_str().unwrap_or("")});
                super::anthropic_search::replay_citations(part, &mut block);
                Ok(block)
            }
            "input_image" => {
                let raw = part["image_url"]
                    .as_str()
                    .ok_or_else(|| invalid("Image URL is required"))?;
                let source = if let Some(data) = raw.strip_prefix("data:") {
                    let (mime, data) = data
                        .split_once(";base64,")
                        .ok_or_else(|| invalid("Images must use a base64 data URL"))?;
                    json!({"type":"base64","media_type":mime,"data":data})
                } else {
                    json!({"type":"url","url":raw})
                };
                Ok(json!({"type":"image","source":source}))
            }
            "input_file" => {
                let source = if let Some(raw) = part["file_data"].as_str() {
                    let (mime, data) = raw
                        .strip_prefix("data:")
                        .and_then(|v| v.split_once(";base64,"))
                        .ok_or_else(|| invalid("Files must use a base64 data URL"))?;
                    json!({"type":"base64","media_type":mime,"data":data})
                } else if let Some(url) = part["file_url"].as_str() {
                    json!({"type":"url","url":url})
                } else {
                    return Err(invalid(
                        "Anthropic file input requires file_data or file_url",
                    ));
                };
                Ok(json!({"type":"document","source":source}))
            }
            "refusal" => Ok(json!({"type":"text","text":part["refusal"].as_str().unwrap_or("")})),
            _ => Err(invalid("Unsupported content block for Anthropic")),
        })
        .collect()
}

fn append(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut().filter(|v| v["role"] == role) {
        last["content"].as_array_mut().unwrap().extend(blocks);
    } else {
        messages.push(json!({"role":role,"content":blocks}));
    }
}

// Reuse the client discovery protocol, independently of ChatGPT plan transport.
// Rebuilding this request-local context also restores names on buffered replies.
fn prepare_client_tools(
    body: &Value,
) -> Result<(Value, super::chatgpt_plan_tools::Context), ServiceError> {
    let mut body = body.clone();
    super::anthropic_compact::expand(&mut body)?;
    let context = super::chatgpt_plan_tools::prepare(&mut body)?;
    Ok((body, context))
}

pub(super) fn responses_to_messages(body: &Value) -> Result<Value, ServiceError> {
    let (body, _) = prepare_client_tools(body)?;
    let body = &body;
    if body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        return Err(invalid(
            "Anthropic requires the expanded conversation for previous_response_id",
        ));
    }
    let context = ToolContext::for_anthropic(body);
    if body["background"] == true || body.get("conversation").is_some_and(|v| !v.is_null()) {
        return Err(invalid(
            "Anthropic providers require a synchronous request with complete history",
        ));
    }
    let mut system = Vec::new();
    if let Some(instructions) = body["instructions"].as_str().filter(|s| !s.is_empty()) {
        system.push(json!({"type":"text","text":instructions}));
    }
    if let Some(format) = body
        .pointer("/text/format")
        .or_else(|| body.get("response_format"))
    {
        match format["type"].as_str(){
            Some("json_object")=>system.push(json!({"type":"text","text":"You must format your entire response as a valid JSON object. Do not include any explanations, markdown code blocks, or any text outside of the JSON object."})),
            Some("json_schema")=>{let schema=format.pointer("/json_schema/schema").or_else(||format.get("schema")).cloned().unwrap_or_else(||json!({"type":"object"}));system.push(json!({"type":"text","text":format!("You must format your entire response as valid JSON that conforms strictly to the following JSON schema:\n{schema}\nDo not include any explanations, markdown code blocks, or any text outside of the JSON.")}));},
            Some("text")|None=>{},
            _=>return Err(invalid("Unsupported Claude output format")),
        }
    }
    let mut messages = Vec::new();
    let input = if let Some(text) = body["input"].as_str() {
        vec![json!({"role":"user","content":text})]
    } else {
        body["input"]
            .as_array()
            .cloned()
            .ok_or_else(|| invalid("Responses input must be text or an array"))?
    };
    for (index, item) in input.iter().enumerate() {
        match item["type"].as_str().unwrap_or("message") {
            "message" => {
                let blocks = parts(&item["content"])?;
                match item["role"].as_str().unwrap_or("user") {
                    "system" | "developer" => system.extend(blocks),
                    "assistant" => append(&mut messages, "assistant", blocks),
                    "user" => append(&mut messages, "user", blocks),
                    _ => return Err(invalid("Unsupported message role for Anthropic")),
                }
            }
            "function_call" | "custom_tool_call" => {
                let name = item["name"]
                    .as_str()
                    .ok_or_else(|| invalid("Tool call name is required"))?;
                let name = context.canonical(name, item["namespace"].as_str());
                let input = if item["type"] == "custom_tool_call" {
                    json!({"input":item["input"].as_str().unwrap_or("")})
                } else {
                    serde_json::from_str::<Value>(item["arguments"].as_str().unwrap_or("{}"))
                        .map_err(|_| invalid("Invalid tool arguments JSON"))?
                };
                if !input.is_object() {
                    return Err(invalid("Anthropic tool arguments must be an object"));
                }
                append(
                    &mut messages,
                    "assistant",
                    vec![json!({"type":"tool_use","id":item["call_id"],"name":name,"input":input})],
                );
            }
            "function_call_output" | "custom_tool_call_output" => {
                let output = &item["output"];
                let content = if output.is_array() {
                    json!(parts(output)?)
                } else if output.is_string() {
                    output.clone()
                } else {
                    json!(output.to_string())
                };
                append(
                    &mut messages,
                    "user",
                    vec![
                        json!({"type":"tool_result","tool_use_id":item["call_id"],"content":content}),
                    ],
                );
            }
            "reasoning" => {
                if let Some(signature) =
                    item["encrypted_content"].as_str().filter(|s| !s.is_empty())
                {
                    if let Some(data) = signature
                        .strip_prefix("claude-redacted#")
                        .or_else(|| signature.strip_prefix("claude-redacted-thinking:"))
                    {
                        append(
                            &mut messages,
                            "assistant",
                            vec![json!({"type":"redacted_thinking","data":data})],
                        );
                        continue;
                    }
                    let signature = if let Some(raw) = signature.strip_prefix("claude#") {
                        raw
                    } else if super::grok_wire::claude_signature(signature) {
                        signature
                    } else {
                        return Err(invalid("This reasoning signature belongs to another provider; start a new conversation for Claude"));
                    };
                    let thinking: String = item["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["text"].as_str())
                        .collect();
                    append(
                        &mut messages,
                        "assistant",
                        vec![json!({"type":"thinking","thinking":thinking,"signature":signature})],
                    );
                }
            }
            "additional_tools" => {}
            "web_search_call" => {
                // Merge a paused search with its later result only within the
                // same assistant turn, matching CPA's replay normalization.
                let later = input
                    .iter()
                    .skip(index + 1)
                    .take_while(|next| {
                        !matches!(next["role"].as_str(), Some("user" | "developer" | "system"))
                            && !matches!(
                                next["type"].as_str(),
                                Some("function_call_output" | "custom_tool_call_output")
                            )
                    })
                    .find(|next| next["type"] == "web_search_call" && next["id"] == item["id"]);
                if item["status"] == "in_progress"
                    && item.get("results").is_none()
                    && later.is_some()
                {
                    continue;
                }
                let tail = !input.iter().skip(index + 1).any(|next| {
                    matches!(next["role"].as_str(), Some("user" | "developer" | "system"))
                        || matches!(
                            next["type"].as_str(),
                            Some("function_call_output" | "custom_tool_call_output")
                        )
                });
                let fallback = input[..index]
                    .iter()
                    .rev()
                    .find(|prev| prev["type"] == "web_search_call" && prev["id"] == item["id"])
                    .and_then(|prev| prev.pointer("/action/query"))
                    .and_then(Value::as_str);
                let blocks = super::anthropic_search::history_blocks(
                    item,
                    !(tail && item["status"] == "in_progress" && item.get("results").is_none()),
                    fallback,
                )?;
                append(&mut messages, "assistant", blocks);
            }
            _ => return Err(invalid("Unsupported Responses history item for Anthropic")),
        }
    }
    if messages.is_empty() {
        return Err(invalid("Anthropic requires at least one message"));
    }
    for message in &mut messages {
        // Tool results must precede any accompanying user text.
        if message["role"] == "user" {
            message["content"]
                .as_array_mut()
                .unwrap()
                .sort_by_key(|part| part["type"] != "tool_result");
        }
    }
    let max_tokens = super::anthropic_models::output_limit(
        body["model"].as_str().unwrap_or(""),
        body["max_output_tokens"].as_u64(),
    );
    let mut out = json!({"model":body["model"],"messages":messages,"max_tokens":max_tokens,"stream":body["stream"].as_bool().unwrap_or(false)});
    let mut tools: Vec<_> = context.tools(false).into_iter().map(|tool|json!({"name":tool["function"]["name"],"description":tool["function"]["description"],"input_schema":tool["function"]["parameters"]})).collect();
    for tool in body["tools"].as_array().into_iter().flatten() {
        if super::anthropic_search::is_tool(tool) {
            if let Some(tool) = super::anthropic_search::declaration(tool) {
                tools.push(tool)
            };
            continue;
        }
        if !matches!(
            tool["type"].as_str(),
            Some("function" | "custom" | "namespace")
        ) {
            return Err(invalid(
                "This Anthropic route supports client function, custom and namespace tools only",
            ));
        }
    }
    if let Some(guide) = super::anthropic_code_mode::instructions(&tools) {
        system.push(json!({"type":"text","text":guide}));
    }
    if !system.is_empty() {
        out["system"] = json!(system);
    }
    if !tools.is_empty() {
        out["tools"] = json!(tools);
    }
    if let Some(choice) = body.get("tool_choice").filter(|v| !v.is_null()) {
        out["tool_choice"] = match choice
            .as_str()
            .or_else(|| choice["type"].as_str())
            .unwrap_or("")
        {
            "auto" => json!({"type":"auto"}),
            "none" => json!({"type":"none"}),
            "required" => json!({"type":"any"}),
            "web_search" | "web_search_preview" | "web_search_preview_2025_03_11" => {
                json!({"type":"tool","name":"web_search"})
            }
            "function" | "custom" => {
                json!({"type":"tool","name":context.canonical(choice["name"].as_str().unwrap_or(""),choice["namespace"].as_str())})
            }
            _ => return Err(invalid("Unsupported Anthropic tool_choice")),
        };
    }
    if body["parallel_tool_calls"] == false && out.get("tools").is_some() {
        if out.get("tool_choice").is_none() {
            out["tool_choice"] = json!({"type":"auto"});
        }
        out["tool_choice"]["disable_parallel_tool_use"] = json!(true);
    }
    for key in ["temperature", "top_p"] {
        if let Some(value) = body.get(key) {
            out[key] = value.clone();
        }
    }
    apply_thinking(body, &mut out)?;
    if matches!(out["tool_choice"]["type"].as_str(), Some("any" | "tool"))
        && !body["model"].as_str().is_some_and(crate::modules::codex_protocol::is_deepseek_model) {
        out.as_object_mut().unwrap().remove("thinking");
        if let Some(config) = out["output_config"].as_object_mut() {
            config.remove("effort");
        }
    }
    Ok(out)
}

fn apply_thinking(body: &Value, out: &mut Value) -> Result<(), ServiceError> {
    let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return Ok(());
    };
    let model = body["model"].as_str().unwrap_or("");
    if crate::modules::codex_protocol::is_deepseek_model(model) {
        if effort == "none" {
            out["thinking"] = json!({"type":"disabled"});
        } else {
            let effort = match effort {
                "minimal" | "low" => "low",
                "medium" | "high" | "xhigh" => "high",
                "max" | "ultra" => "max",
                _ => return Err(invalid("Unsupported DeepSeek reasoning effort")),
            };
            out["thinking"] = json!({"type":"enabled"});
            out["output_config"] = json!({"effort":effort});
        }
        return Ok(());
    }
    let capability = super::anthropic_models::get(model).and_then(|model| model.thinking.as_ref());
    if effort == "none" {
        if capability.is_some_and(|thinking| !thinking.zero_allowed) {
            return Err(invalid(
                "This Claude model does not support disabling thinking",
            ));
        }
        out["thinking"] = json!({"type":capability.and_then(|thinking|thinking.disable_type.as_deref()).unwrap_or("disabled")});
        return Ok(());
    }
    if let Some(thinking) = capability.filter(|thinking| !thinking.levels.is_empty()) {
        out["thinking"] = json!({"type":"adaptive"});
        if effort == "auto" {
            return Ok(());
        }
        let effort = if thinking.levels.iter().any(|level| level == effort) {
            effort
        } else {
            match effort {
                "minimal" => "low",
                "xhigh" | "max" if thinking.levels.iter().any(|level| level == "max") => "max",
                "xhigh" | "max" => "high",
                _ => return Err(invalid("Unsupported Claude reasoning effort")),
            }
        };
        out["output_config"] = json!({"effort":effort});
    } else {
        let budget = match effort {
            "minimal" => 512,
            "low" => 1024,
            "medium" => 8192,
            "high" => 24576,
            "xhigh" => 32768,
            "max" => 128000,
            "auto" => capability
                .map(|thinking| thinking.max)
                .filter(|max| *max > 0)
                .unwrap_or(24576),
            _ => return Err(invalid("Unsupported Claude reasoning effort")),
        };
        let limit = out["max_tokens"].as_u64().unwrap_or(32000);
        let minimum = capability
            .map(|thinking| thinking.min)
            .filter(|min| *min > 0)
            .unwrap_or(1024);
        if limit <= minimum {
            return Err(invalid(
                "Thinking requires max_output_tokens above the model's minimum thinking budget",
            ));
        }
        let maximum = capability
            .map(|thinking| thinking.max)
            .filter(|max| *max > 0)
            .unwrap_or(u64::MAX);
        out["thinking"] =
            json!({"type":"enabled","budget_tokens":budget.max(minimum).min(maximum).min(limit-1)});
    }
    Ok(())
}

/// Advertise the selected Claude family's real effort tiers rather than GPT defaults.
pub(crate) fn customize_model_template(model: &str, template: &mut Value) {
    let capability = super::anthropic_models::get(model);
    let thinking = capability.and_then(|model| model.thinking.as_ref());
    let efforts: Vec<&str> = thinking
        .map(|thinking| {
            if thinking.levels.is_empty() {
                vec!["low", "medium", "high", "xhigh"]
            } else {
                thinking.levels.iter().map(String::as_str).collect()
            }
        })
        .unwrap_or_default();
    let catalog = crate::modules::codex_protocol::build_codex_client_models_response(&[
        "gpt-6.1-sol".to_owned(),
    ]);
    let defaults = catalog["models"][0]["supported_reasoning_levels"].as_array();
    template["supported_reasoning_levels"] = json!(efforts
        .iter()
        .map(|effort| defaults
            .and_then(|levels| levels.iter().find(|entry| entry["effort"] == *effort))
            .cloned()
            .unwrap_or_else(|| json!({"effort":effort,"description":effort})))
        .collect::<Vec<_>>());
    template["default_reasoning_level"] = if efforts.contains(&"medium") {
        json!("medium")
    } else {
        efforts.first().map_or(Value::Null, |effort| json!(effort))
    };
    if let Some(context) = capability.and_then(|model| model.context_length) {
        template["context_window"] = json!(context);
    }
    template["supports_search_tool"] = json!(true);
    template["prefer_websockets"] = json!(false);
}

pub(super) fn usage(value: &Value) -> Value {
    let cache_read = value["cache_read_input_tokens"].as_u64().unwrap_or(0);
    let cache_write = value["cache_creation_input_tokens"].as_u64().unwrap_or(0);
    let input = value["input_tokens"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(cache_read)
        .saturating_add(cache_write);
    let output = value["output_tokens"].as_u64().unwrap_or(0);
    json!({"input_tokens":input,"output_tokens":output,"total_tokens":input.saturating_add(output),"input_tokens_details":{"cached_tokens":cache_read,"cache_write_tokens":cache_write},"output_tokens_details":{"reasoning_tokens":0}})
}

pub(super) fn output_item(block: &Value, context: &ToolContext) -> Result<Value, ServiceError> {
    output_item_partial(block, context, false)
}

fn output_item_partial(
    block: &Value,
    context: &ToolContext,
    partial: bool,
) -> Result<Value, ServiceError> {
    let item = match block["type"].as_str().unwrap_or("") {
        "text" => {
            json!({"id":format!("msg_{}",uuid::Uuid::now_v7()),"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":block["text"].as_str().unwrap_or(""),"annotations":super::anthropic_search::citations(block)}]})
        }
        "tool_use" => {
            if !block["input"].is_object() {
                return Err(upstream("Tool input is not a JSON object"));
            }
            if !partial
                && context.is_apply_patch(block["name"].as_str().unwrap_or(""))
                && (block["input"].as_object().unwrap().len() != 1
                    || !block["input"]["input"].is_string())
            {
                return Err(upstream(
                    "Claude returned invalid apply_patch input; expected one input string",
                ));
            }
            let mut item = json!({"id":format!("fc_{}",uuid::Uuid::now_v7()),"type":"function_call","status":"completed","call_id":block["id"],"name":block["name"],"arguments":block["input"].to_string()});
            context.restore_item(&mut item);
            if item["type"] == "function_call" {
                super::grok_wire::normalize_tool_arguments(&mut item);
            }
            item
        }
        "thinking" => {
            let signature = block["signature"].as_str().unwrap_or("");
            let signature = if super::grok_wire::claude_signature(signature) {
                signature.to_owned()
            } else {
                format!("claude#{signature}")
            };
            let mut item = json!({"id":format!("rs_{}",uuid::Uuid::now_v7()),"type":"reasoning","summary":[{"type":"summary_text","text":block["thinking"].as_str().unwrap_or("")}],"encrypted_content":signature});
            if block["signature"].as_str().is_none_or(str::is_empty) {
                item.as_object_mut().unwrap().remove("encrypted_content");
            }
            item
        }
        "redacted_thinking" => {
            json!({"id":format!("rs_{}",uuid::Uuid::now_v7()),"type":"reasoning","summary":[],"encrypted_content":format!("claude-redacted-thinking:{}",block["data"].as_str().unwrap_or(""))})
        }
        _ => return Err(upstream("Unsupported Anthropic response content block")),
    };
    Ok(item)
}

pub(super) fn messages_to_response(
    value: &Value,
    model: &str,
    body: &Value,
) -> Result<Value, ServiceError> {
    let (body, mut discovery) = prepare_client_tools(body)?;
    if value["type"] == "error" {
        return Err(upstream(
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Anthropic returned an error"),
        ));
    }
    if value["id"].as_str().is_none_or(str::is_empty)
        || value["stop_reason"].as_str().is_none_or(str::is_empty)
    {
        return Err(upstream(
            "Anthropic response is missing its id or stop reason",
        ));
    }
    let context = ToolContext::for_anthropic(&body);
    let blocks = value["content"]
        .as_array()
        .ok_or_else(|| upstream("Anthropic response has no content array"))?;
    let output = super::anthropic_search::output(
        blocks,
        &context,
        &body,
        value["stop_reason"].as_str().unwrap_or(""),
    )?;
    let incomplete = matches!(
        value["stop_reason"].as_str(),
        Some("max_tokens" | "model_context_window_exceeded" | "refusal")
    );
    let mut response = json!({"id":format!("resp_{}",value["id"].as_str().unwrap_or("anthropic")),"object":"response","created_at":chrono::Utc::now().timestamp(),"model":model,"status":if incomplete {"incomplete"}else{"completed"},"error":null,"incomplete_details":if incomplete {json!({"reason":if value["stop_reason"]=="refusal"{"content_filter"}else{"max_output_tokens"}})}else{Value::Null},"output":output,"usage":usage(&value["usage"])});
    if value["stop_reason"] == "pause_turn" {
        response["end_turn"] = json!(false)
    }
    super::chatgpt_plan_tools::restore_event(&mut discovery, response)?
        .pop()
        .ok_or_else(|| upstream("Anthropic returned no response"))
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
