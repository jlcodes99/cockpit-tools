use super::{invalid, ServiceError};
use serde_json::{json, Value};

pub(super) fn is_claude_tool(value: &Value) -> bool {
    matches!(
        value["type"].as_str(),
        Some("web_search_20250305" | "web_search_20260209")
    )
}

pub(super) fn claude_tool(value: &Value) -> Result<Value, ServiceError> {
    let mut tool = json!({"type":"web_search"});
    if let Some(domains) = value.get("allowed_domains") {
        if !domains
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string))
        {
            return Err(invalid("allowed_domains", "expected a string array"));
        }
        tool["filters"] = json!({"allowed_domains":domains});
    }
    if let Some(location) = value.get("user_location") {
        if !location.is_object() {
            return Err(invalid("user_location", "expected an object"));
        }
        tool["user_location"] = location.clone();
    }
    Ok(tool)
}

pub(super) fn claude_blocks(item: &Value) -> Vec<Value> {
    let Some(id) = item["id"].as_str().filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    let query = item
        .pointer("/action/query")
        .or_else(|| item.get("query"))
        .or_else(|| item.pointer("/input/query"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let results = item.get("results").and_then(Value::as_array);
    if query.is_empty() && results.is_none() {
        return Vec::new();
    }
    let results = results
        .into_iter()
        .flatten()
        .filter_map(|result| {
            let url = result["url"].as_str().filter(|url| !url.is_empty())?;
            Some(json!({"type":"web_search_result", "url":url,
            "title":result["title"].as_str().filter(|title| !title.is_empty()).unwrap_or(url),
            "page_age":result.get("page_age").cloned().unwrap_or(Value::Null)}))
        })
        .collect::<Vec<_>>();
    vec![
        json!({"type":"server_tool_use","id":id,"name":"web_search","input":if query.is_empty(){json!({})}else{json!({"query":query})}}),
        json!({"type":"web_search_tool_result","tool_use_id":id,"content":results}),
    ]
}

pub(super) fn history(block: &Value, input: &mut Vec<Value>) -> Result<(), ServiceError> {
    if block["type"] == "server_tool_use" {
        if block["name"] != "web_search" {
            return Err(invalid("server_tool_use.name", "expected web_search"));
        }
        let id = block["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| invalid("server_tool_use.id", "expected an identifier"))?;
        let query = block
            .pointer("/input/query")
            .and_then(Value::as_str)
            .unwrap_or("");
        input.push(json!({"type":"web_search_call","id":id,"status":"completed","action":{"type":"search","query":query}}));
    } else {
        let id = block["tool_use_id"]
            .as_str()
            .ok_or_else(|| invalid("tool_use_id", "expected an identifier"))?;
        let item = input
            .iter_mut()
            .rev()
            .find(|item| item["type"] == "web_search_call" && item["id"] == id)
            .ok_or_else(|| invalid("web_search_tool_result", "cannot match its server tool use"))?;
        let content = block["content"].as_array().ok_or_else(|| {
            invalid(
                "web_search_tool_result.content",
                "expected search result blocks",
            )
        })?;
        if content.iter().any(|result| {
            ["encrypted_content", "encrypted_index"]
                .iter()
                .any(|key| result.get(*key).is_some_and(|value| !value.is_null()))
        }) {
            return Err(super::unsupported("web_search_result.encrypted_content"));
        }
        item["results"] = json!(content
            .iter()
            .filter(|result| result["type"] == "web_search_result")
            .map(|result| json!({"url":result["url"],"title":result["title"]}))
            .collect::<Vec<_>>());
    }
    Ok(())
}
