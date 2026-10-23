use super::super::chat::image_mime;
use super::{search, upstream, CompatContext, CompatFlavor, ServiceError};
use serde_json::{json, Value};
pub(crate) fn usage(usage: &Value, flavor: CompatFlavor) -> Value {
    let mut out = json!({});
    let input = usage["input_tokens"].as_u64();
    let output = usage["output_tokens"].as_u64();
    let cache = usage
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64);
    let reasoning = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64);
    match flavor {
        CompatFlavor::Claude => {
            if let Some(n) = input {
                out["input_tokens"] = json!(n.saturating_sub(cache.unwrap_or(0)));
            }
            if let Some(n) = output {
                out["output_tokens"] = json!(n);
            }
            if let Some(n) = cache {
                out["cache_read_input_tokens"] = json!(n);
            }
            if let Some(n) = usage
                .pointer("/input_tokens_details/cache_write_tokens")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .or_else(|| {
                    usage
                        .pointer("/input_tokens_details/cache_creation_tokens")
                        .and_then(Value::as_u64)
                        .filter(|n| *n > 0)
                })
            {
                out["cache_creation_input_tokens"] = json!(n);
            }
        }
        CompatFlavor::Gemini => {
            if let Some(n) = input {
                out["promptTokenCount"] = json!(n);
            }
            if let Some(n) = output {
                out["candidatesTokenCount"] = json!(n);
            }
            if let Some(n) = reasoning {
                out["thoughtsTokenCount"] = json!(n);
            }
            if let Some(n) = cache {
                out["cachedContentTokenCount"] = json!(n);
            }
            if let Some(total) = usage.get("total_tokens") {
                out["totalTokenCount"] = total.clone();
            } else if let (Some(a), Some(b)) = (input, output) {
                out["totalTokenCount"] = json!(a.saturating_add(b));
            }
        }
        CompatFlavor::Ollama => {
            out["prompt_eval_count"] = json!(input.unwrap_or(0));
            out["eval_count"] = json!(output.unwrap_or(0));
            if let Some(n) = input {
                out["prompt_eval_count"] = json!(n);
            }
            if let Some(n) = output {
                out["eval_count"] = json!(n);
            }
            if let Some(n) = cache {
                out["prompt_eval_cached_count"] = json!(n);
            }
        }
    }
    out
}
pub(crate) fn stop_reason(response: &Value, flavor: CompatFlavor, tools: bool) -> &'static str {
    let sequence = response["stop_sequence"]
        .as_str()
        .filter(|sequence| !sequence.is_empty());
    let reason = response["stop_reason"]
        .as_str()
        .filter(|reason| !reason.is_empty())
        .or_else(|| {
            response
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    match flavor {
        CompatFlavor::Claude => match reason {
            _ if tools => "tool_use",
            "max_output_tokens" | "max_tokens" => "max_tokens",
            "content_filter" | "refusal" => "refusal",
            "stop_sequence" => "stop_sequence",
            "pause_turn" => "pause_turn",
            "model_context_window_exceeded" => "model_context_window_exceeded",
            "" | "stop" if sequence.is_some() => "stop_sequence",
            _ => "end_turn",
        },
        CompatFlavor::Gemini => match reason {
            "max_output_tokens" | "max_tokens" => "MAX_TOKENS",
            "content_filter" => "SAFETY",
            _ => "STOP",
        },
        CompatFlavor::Ollama => match reason {
            "max_output_tokens" | "max_tokens" => "length",
            "content_filter" => "content_filter",
            _ if tools => "tool_calls",
            _ => "stop",
        },
    }
}
pub(crate) fn args(item: &Value) -> Result<Value, ServiceError> {
    let raw = item["arguments"]
        .as_str()
        .ok_or_else(|| upstream("Function call arguments must be a JSON string"))?;
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| upstream("Function call arguments are not valid JSON"))?;
    if !value.is_object() {
        return Err(upstream("Function call arguments must be a JSON object"));
    }
    Ok(value)
}
pub fn response_from_responses(
    response: &Value,
    context: &CompatContext,
) -> Result<Value, ServiceError> {
    if let Some(error) = super::super::upstream_errors::chat_failure(response, 502) {
        return Err(error);
    }
    if response["status"] == "failed" {
        return Err(upstream(
            response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Upstream response failed"),
        ));
    }
    let items = response["output"]
        .as_array()
        .ok_or_else(|| upstream("Responses output must be an array"))?;
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut thinking = String::new();
    let mut calls = Vec::new();
    let mut images = Vec::new();
    let mut has_tools = false;
    for item in items {
        match item["type"].as_str().unwrap_or("") {
            "message" => {
                for content in item["content"].as_array().into_iter().flatten() {
                    let value =
                        match content["type"].as_str() {
                            Some("output_text") => content["text"].as_str(),
                            Some("refusal") => content["refusal"].as_str(),
                            _ => return Err(upstream(
                                "This output content is not representable in the client protocol",
                            )),
                        }
                        .ok_or_else(|| upstream("Output text must be a string"))?;
                    text.push_str(value);
                    parts.push(match context.flavor {
                        CompatFlavor::Claude => json!({"type":"text","text":value}),
                        _ => json!({"text":value}),
                    });
                }
            }
            "reasoning" => {
                let mut summary = item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("");
                if summary.is_empty() {
                    summary = item["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("");
                }
                let signature = item["encrypted_content"].as_str().unwrap_or("");
                if !summary.is_empty()
                    || (context.flavor == CompatFlavor::Claude && !signature.is_empty())
                {
                    thinking.push_str(&summary);
                    parts.push(match context.flavor {
                        CompatFlavor::Claude => {
                            json!({"type":"thinking","thinking":summary,"signature":signature})
                        }
                        _ => json!({"text":summary,"thought":true}),
                    });
                }
            }
            "function_call" => {
                has_tools = true;
                let args = args(item)?;
                let name = item["name"]
                    .as_str()
                    .ok_or_else(|| upstream("Function call name is missing"))?;
                let name = context.tool_name(name);
                let id = item["call_id"]
                    .as_str()
                    .ok_or_else(|| upstream("Function call ID is missing"))?;
                parts.push(match context.flavor {
                    CompatFlavor::Claude => {
                        json!({"type":"tool_use","id":id,"name":name,"input":args})
                    }
                    _ => json!({"functionCall":{"id":id,"name":name,"args":args}}),
                });
                calls.push(
                    json!({"id":id,"type":"function","function":{"name":name,"arguments":args}}),
                );
            }
            "custom_tool_call" => {
                return Err(upstream(
                    "The client protocol only supports JSON function tools",
                ))
            }
            "image_generation_call" => {
                if let Some(data) = item["result"].as_str() {
                    let mime = image_mime(item["output_format"].as_str());
                    parts.push(match context.flavor{CompatFlavor::Claude=>json!({"type":"image","source":{"type":"base64","media_type":mime,"data":data}}),_=>json!({"inlineData":{"mimeType":mime,"data":data}})});
                    images.push(json!(data));
                }
            }
            // Hosted tool traces have no client invocation; only their message/image results are rendered.
            "web_search_call" => {
                if context.flavor == CompatFlavor::Claude {
                    parts.extend(search::claude_blocks(item));
                }
            }
            "file_search_call" | "code_interpreter_call" | "mcp_list_tools" | "mcp_call" => {}
            _ => {
                return Err(upstream(
                    "This Responses output item is not representable in the client protocol",
                ))
            }
        }
    }
    let model = response["model"].as_str().unwrap_or(&context.model);
    let id = response["id"].as_str().unwrap_or("");
    let usage = usage(&response["usage"], context.flavor);
    let reason = stop_reason(response, context.flavor, has_tools);
    let stop_sequence = response
        .get("stop_sequence")
        .filter(|value| value.as_str().is_some_and(|s| !s.is_empty()))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(match context.flavor {
        CompatFlavor::Claude => {
            json!({"id":format!("msg_{id}"),"type":"message","role":"assistant","model":model,"content":parts,"stop_reason":reason,"stop_sequence":stop_sequence,"usage":usage})
        }
        CompatFlavor::Gemini => {
            json!({"candidates":[{"index":0,"content":{"role":"model","parts":parts},"finishReason":reason}],"usageMetadata":usage,"modelVersion":model,"responseId":id,"createTime":timestamp(response)})
        }
        CompatFlavor::Ollama => {
            let mut message = json!({"role":"assistant","content":text});
            if !thinking.is_empty() {
                message["thinking"] = json!(thinking);
            }
            if !calls.is_empty() {
                message["tool_calls"] = json!(calls);
            }
            if !images.is_empty() {
                message["images"] = json!(images);
            }
            let mut result = json!({"model":model,"created_at":timestamp(response),"message":message,"done":true,"done_reason":reason,"total_duration":0,"load_duration":0,"prompt_eval_duration":0,"eval_duration":0});
            result
                .as_object_mut()
                .unwrap()
                .extend(usage.as_object().unwrap().clone());
            result
        }
    })
}
pub(crate) fn timestamp(response: &Value) -> String {
    response["created_at"]
        .as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}
pub fn models(value: &Value, flavor: CompatFlavor) -> Result<Value, ServiceError> {
    let data = value["data"]
        .as_array()
        .or_else(|| value.as_array())
        .ok_or_else(|| upstream("Models list must contain data"))?;
    let mut result = Vec::new();
    for model in data {
        let id = model["id"]
            .as_str()
            .ok_or_else(|| upstream("Model ID is missing"))?;
        let name = model["display_name"].as_str().unwrap_or(id);
        result.push(match flavor{
        CompatFlavor::Claude=>{let mut entry=json!({"id":id,"type":"model","display_name":name});if let Some(created)=model.get("created").and_then(Value::as_i64).and_then(|t|chrono::DateTime::from_timestamp(t,0)){entry["created_at"]=json!(created.to_rfc3339());}entry},
        CompatFlavor::Gemini=>{let mut entry=json!({"name":format!("models/{id}"),"displayName":name,"supportedGenerationMethods":["generateContent","streamGenerateContent","countTokens"]});for(from,to)in[("context_length","inputTokenLimit"),("max_output_tokens","outputTokenLimit")]{if let Some(v)=model.get(from){entry[to]=v.clone();}}entry},
        CompatFlavor::Ollama=>{let mut entry=json!({"name":id,"model":id,"remote_model":id});if let Some(v)=model.get("description"){entry["description"]=v.clone();}entry},
    });
    }
    Ok(match flavor {
        CompatFlavor::Claude => {
            json!({"data":result,"has_more":false,"first_id":data.first().and_then(|v|v.get("id")),"last_id":data.last().and_then(|v|v.get("id"))})
        }
        _ => json!({"models":result}),
    })
}
#[cfg(test)]
pub fn count_tokens_response(value: &Value, flavor: CompatFlavor) -> Result<Value, ServiceError> {
    let tokens = value["input_tokens"]
        .as_u64()
        .ok_or_else(|| upstream("Upstream token count is missing input_tokens"))?;
    Ok(match flavor {
        CompatFlavor::Claude => json!({"input_tokens":tokens}),
        CompatFlavor::Gemini => json!({"totalTokens":tokens}),
        CompatFlavor::Ollama => json!({"prompt_eval_count":tokens}),
    })
}
pub fn ollama_show(model: &Value) -> Result<Value, ServiceError> {
    let id = model["id"]
        .as_str()
        .ok_or_else(|| upstream("Model ID is missing"))?;
    let mut result = json!({"remote_model":id,"capabilities":["completion"],"model_info":{}});
    for key in ["details", "capabilities", "model_info"] {
        if let Some(v) = model.get(key) {
            result[key] = v.clone();
        }
    }
    if let Some(v) = model.get("context_length") {
        result["model_info"]["context_length"] = v.clone();
    }
    Ok(result)
}
