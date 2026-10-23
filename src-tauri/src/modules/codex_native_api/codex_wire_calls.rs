use serde_json::{json, Value};
use std::collections::HashSet;

/// Repair only historical tool protocol items. User strings and opaque item IDs stay intact.
pub(super) fn normalize(body: &mut Value) {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    let mut used: HashSet<String> = items
        .iter()
        .filter_map(|item| item["call_id"].as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect();
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut retained = Vec::with_capacity(items.len());
    for (index, mut item) in std::mem::take(items).into_iter().enumerate() {
        let kind = item["type"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let is_call = matches!(
            kind.as_str(),
            "function_call" | "custom_tool_call" | "tool_call" | "mcp_tool_call"
        );
        let is_output = matches!(
            kind.as_str(),
            "function_call_output"
                | "custom_tool_call_output"
                | "tool_call_output"
                | "mcp_tool_call_output"
        );
        if !is_call && !is_output {
            retained.push(item);
            continue;
        }
        if super::super::responses_host_results::is_host_result(&item)
            && item["call_id"]
                .as_str()
                .is_none_or(|id| id.trim().is_empty())
        {
            // Never borrow an unrelated call ID for a host-injected result.
            // Keep malformed IDs intact too: identity validation must reject them.
            retained.push(item);
            continue;
        }
        let name = item["name"].as_str().unwrap_or("").trim().to_owned();
        let mut id = item["call_id"].as_str().unwrap_or("").trim().to_owned();
        if id.is_empty() {
            if is_call {
                let base = format!("call_missing_{index}");
                id = base.clone();
                let mut suffix = 1;
                while used.contains(&id) {
                    id = format!("{base}_{suffix}");
                    suffix += 1;
                }
                used.insert(id.clone());
            } else {
                let matched = (!name.is_empty())
                    .then(|| pending.iter().position(|(_, candidate)| candidate == &name))
                    .flatten()
                    .or_else(|| (!pending.is_empty()).then_some(0));
                if let Some(matched) = matched {
                    id = pending.remove(matched).0;
                } else {
                    if kind == "function_call_output" && !name.is_empty() {
                        retained.push(item);
                    }
                    continue;
                }
            }
            item["call_id"] = json!(id);
        }
        if is_call {
            pending.push((id, name));
        } else if let Some(index) = pending.iter().position(|(candidate, _)| candidate == &id) {
            pending.remove(index);
        }
        retained.push(item);
    }
    *items = retained;
}
