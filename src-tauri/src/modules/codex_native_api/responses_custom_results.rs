//! Lossless replay copies of contiguous custom-tool notifications.
//! The caller's input and durable checkpoints retain every original envelope.
use std::{borrow::Cow, collections::HashMap};

use serde_json::{json, Value};

use super::identity::rebuild::{paired_tools, replay_items};

/// A complete source remains subject to the ordinary replay checks. Only a
/// uniquely identified, preceding custom call can authorize combining adjacent
/// results. The final copy must still pass the unchanged strict pairing rule.
pub(super) fn replay(items: &[Value]) -> Option<Cow<'_, [Value]>> {
    if !replay_items(items) {
        return None;
    }
    if paired_tools(items) {
        return Some(Cow::Borrowed(items));
    }
    let mut calls = HashMap::<&str, (usize, &Value)>::new();
    for (index, item) in items.iter().enumerate() {
        if matches!(
            item["type"].as_str(),
            Some("function_call" | "custom_tool_call" | "local_shell_call")
        ) {
            let id = item["call_id"].as_str()?;
            if calls.insert(id, (index, item)).is_some() {
                return None;
            }
        }
    }
    let mut copy = Vec::with_capacity(items.len());
    let mut changed = false;
    let mut index = 0;
    while index < items.len() {
        let first = &items[index];
        let mut end = index + 1;
        if first["type"] == "custom_tool_call_output" {
            let id = first["call_id"]
                .as_str()
                .filter(|id| !id.trim().is_empty())?;
            while end < items.len()
                && items[end]["type"] == "custom_tool_call_output"
                && items[end]["call_id"].as_str() == Some(id)
            {
                end += 1;
            }
            if end > index + 1 {
                let (call_index, call) = calls.get(id)?;
                if *call_index >= index || !plain_custom_call(call) {
                    return None;
                }
                let mut parts = Vec::new();
                let mut output_ids = HashMap::<&str, &Value>::new();
                for item in &items[index..end] {
                    if !plain_custom_result(item, call) {
                        return None;
                    }
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        if output_ids.insert(id, item).is_some_and(|old| old != item) {
                            return None;
                        }
                    }
                    match &item["output"] {
                        Value::String(text) => parts.push(json!({"type":"input_text","text":text})),
                        Value::Array(values) => parts.extend(values.iter().cloned()),
                        _ => return None,
                    }
                }
                // Keep the first result's ID/envelope. Later IDs and optional
                // notification names remain in Plan::source_input, never erased
                // from the persisted provenance or treated as signatures.
                let mut merged = first.clone();
                merged["output"] = Value::Array(parts);
                copy.push(merged);
                changed = true;
                index = end;
                continue;
            }
        }
        copy.push(first.clone());
        index = end;
    }
    (changed && replay_items(&copy) && paired_tools(&copy)).then_some(Cow::Owned(copy))
}

fn optional_id(item: &Value) -> bool {
    item.get("id")
        .is_none_or(|id| id.as_str().is_some_and(|id| !id.trim().is_empty()))
}

fn plain_custom_call(item: &Value) -> bool {
    item["type"] == "custom_tool_call"
        && item["name"].as_str().is_some_and(|name| !name.is_empty())
        && item["input"].is_string()
        && optional_id(item)
        && item
            .get("status")
            .is_none_or(|status| status == "completed")
        && item.as_object().is_some_and(|object| {
            object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "type" | "call_id" | "name" | "input" | "id" | "status"
                )
            })
        })
}

fn plain_custom_result(item: &Value, call: &Value) -> bool {
    optional_id(item)
        && item.get("name").is_none_or(|name| name == &call["name"])
        && item.as_object().is_some_and(|object| {
            object
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "call_id" | "output" | "id" | "name"))
        })
}

#[cfg(test)]
#[path = "responses_custom_results_tests.rs"]
mod tests;
