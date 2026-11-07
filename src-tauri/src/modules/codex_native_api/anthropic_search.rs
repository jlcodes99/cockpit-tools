//! CPA's Responses web-search pair and citation replay contract.
use super::contract::ServiceError;
use serde_json::{json, Value};

pub(super) fn is_tool(tool: &Value) -> bool {
    matches!(
        tool["type"].as_str(),
        Some("web_search" | "web_search_preview" | "web_search_preview_2025_03_11")
    )
}

pub(super) fn declaration(tool: &Value) -> Option<Value> {
    if tool["external_web_access"] == false {
        return None;
    }
    let mut out = json!({"type":"web_search_20250305","name":tool["name"].as_str().filter(|s|!s.is_empty()).unwrap_or("web_search")});
    for key in ["max_uses", "user_location"] {
        if let Some(value) = tool.get(key) {
            out[key] = value.clone();
        }
    }
    if let Some(domains) = tool
        .pointer("/filters/allowed_domains")
        .filter(|v| v.as_array().is_some_and(|v| !v.is_empty()))
    {
        out["allowed_domains"] = domains.clone();
    }
    Some(out)
}

fn tool_id(raw: &str) -> String {
    let body = raw.strip_prefix("ws_").unwrap_or(raw);
    let body = body.strip_prefix("srvtoolu_").unwrap_or(body);
    format!(
        "srvtoolu_{}",
        body.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            })
            .collect::<String>()
    )
}

pub(super) fn history_blocks(
    item: &Value,
    with_result: bool,
    fallback_query: Option<&str>,
) -> Result<Vec<Value>, ServiceError> {
    let raw = item["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ServiceError::new(
                400,
                "anthropic_request_invalid",
                "Web search history requires an id",
            )
        })?;
    let id = tool_id(raw);
    let query = item
        .pointer("/action/query")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| item.pointer("/action/queries/0").and_then(Value::as_str))
        .or_else(|| item.pointer("/action/url").and_then(Value::as_str))
        .or(fallback_query)
        .unwrap_or("");
    let mut out =
        vec![json!({"type":"server_tool_use","id":id,"name":"web_search","input":{"query":query}})];
    if with_result {
        let results = &item["results"];
        let content = if results.is_object() {
            results.clone()
        } else {
            json!(results
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["type"] == "web_search_tool_result_error"
                    || entry["encrypted_content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty()))
                .map(|entry| {
                    let mut entry = entry.clone();
                    if entry["type"] != "web_search_tool_result_error" {
                        entry["type"] = json!("web_search_result")
                    };
                    entry
                })
                .collect::<Vec<_>>())
        };
        out.push(json!({"type":"web_search_tool_result","tool_use_id":id,"content":content}));
    }
    Ok(out)
}

pub(super) fn query_from_history(body: &Value, id: &str) -> Option<String> {
    body["input"]
        .as_array()?
        .iter()
        .rev()
        .find(|item| item["type"] == "web_search_call" && item["id"] == id)
        .and_then(|item| item.pointer("/action/query"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(super) fn item(id: &str, query: &str) -> Value {
    json!({"id":format!("ws_{id}"),"type":"web_search_call","status":"in_progress","action":{"type":"search","query":query}})
}

pub(super) fn attach_results(item: &mut Value, content: &Value) {
    if content.is_object() {
        item["results"] = content.clone()
    } else {
        let results: Vec<_> = content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| {
                entry["type"] == "web_search_tool_result_error"
                    || entry["url"].as_str().is_some_and(|s| !s.is_empty())
            })
            .cloned()
            .collect();
        item["results"] = json!(results);
    }
}

pub(super) fn finalize(item: &mut Value, stop: &str) {
    item["status"] = json!(if item.get("results").is_none()
        && matches!(stop, "pause_turn" | "tool_use")
    {
        "in_progress"
    } else if matches!(
        stop,
        "max_tokens" | "model_context_window_exceeded" | "refusal"
    ) {
        "incomplete"
    } else {
        "completed"
    });
}

pub(super) fn output(
    blocks: &[Value],
    context: &super::chat_upstream::ToolContext,
    body: &Value,
    stop: &str,
) -> Result<Vec<Value>, ServiceError> {
    let mut out: Vec<Value> = vec![];
    for block in blocks {
        match block["type"].as_str() {
            Some("server_tool_use") if block["name"] == "web_search" => out.push(item(
                block["id"].as_str().unwrap_or(""),
                block["input"]["query"].as_str().unwrap_or(""),
            )),
            Some("web_search_tool_result") => {
                let id = format!("ws_{}", block["tool_use_id"].as_str().unwrap_or(""));
                let position = out
                    .iter()
                    .position(|item| item["type"] == "web_search_call" && item["id"] == id);
                let index = position.unwrap_or_else(|| {
                    let query = query_from_history(body, &id).unwrap_or_default();
                    out.push(item(block["tool_use_id"].as_str().unwrap_or(""), &query));
                    out.len() - 1
                });
                attach_results(&mut out[index], &block["content"]);
            }
            _ => out.push(super::anthropic::output_item(block, context)?),
        }
    }
    for item in &mut out {
        if item["type"] == "web_search_call" {
            finalize(item, stop);
        }
    }
    Ok(out)
}

pub(super) fn citations(block: &Value) -> Vec<Value> {
    block["citations"].as_array().cloned().unwrap_or_default()
}

pub(super) fn replay_citations(part: &Value, block: &mut Value) {
    let citations: Vec<_> = part["annotations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| {
            item["encrypted_index"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        })
        .cloned()
        .collect();
    if !citations.is_empty() {
        block["citations"] = json!(citations)
    }
}
