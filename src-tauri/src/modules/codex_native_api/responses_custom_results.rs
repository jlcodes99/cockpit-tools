//! Lossless replay copies of contiguous custom-tool notifications.
//! The caller's input and durable checkpoints retain every original envelope.
use std::{borrow::Cow, collections::HashMap, ops::Range};

use serde_json::{json, Value};

use super::identity::rebuild::{paired_tool_items, paired_tools, replay_items};

/// A complete source remains subject to the ordinary replay checks. Only a
/// uniquely identified, preceding custom call can authorize combining adjacent
/// results. The final copy must still pass the unchanged strict pairing rule.
pub(super) fn replay(items: &[Value]) -> Option<Cow<'_, [Value]>> {
    let groups = merge_groups(items)?;
    if groups.is_empty() {
        return Some(Cow::Borrowed(items));
    }

    let mut copy = Vec::with_capacity(items.len());
    let mut start = 0;
    for group in groups {
        copy.extend_from_slice(&items[start..group.start]);
        let mut parts = Vec::new();
        for item in &items[group.clone()] {
            match &item["output"] {
                Value::String(text) => parts.push(json!({"type":"input_text","text":text})),
                Value::Array(values) => parts.extend(values.iter().cloned()),
                _ => unreachable!("merge groups only contain validated result outputs"),
            }
        }
        // Preserve the first envelope on the wire and all envelopes in the checkpoint.
        let mut merged = items[group.start]
            .as_object()?
            .iter()
            .filter(|(field, _)| field.as_str() != "output")
            .map(|(field, value)| (field.clone(), value.clone()))
            .collect::<serde_json::Map<_, _>>();
        merged.insert("output".into(), Value::Array(parts));
        copy.push(Value::Object(merged));
        start = group.end;
    }
    copy.extend_from_slice(&items[start..]);
    Some(Cow::Owned(copy))
}

pub(super) fn replayable(items: &[Value]) -> bool {
    merge_groups(items).is_some()
}

fn merge_groups(items: &[Value]) -> Option<Vec<Range<usize>>> {
    if !replay_items(items) {
        return None;
    }
    if paired_tools(items) {
        return Some(Vec::new());
    }

    let mut calls = HashMap::<&str, &Value>::new();
    let mut groups = Vec::new();
    let mut index = 0;
    while let Some(first) = items.get(index) {
        match first["type"].as_str() {
            Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                let id = first["call_id"].as_str()?;
                if calls.insert(id, first).is_some() {
                    return None;
                }
            }
            Some("custom_tool_call_output") => {
                let id = first["call_id"]
                    .as_str()
                    .filter(|id| !id.trim().is_empty())?;
                let end = index
                    + 1
                    + items[index + 1..]
                        .iter()
                        .take_while(|item| {
                            item["type"] == "custom_tool_call_output"
                                && item["call_id"].as_str() == Some(id)
                        })
                        .count();
                if end > index + 1 {
                    let call = calls.get(id)?;
                    if !plain_custom_call(call) {
                        return None;
                    }
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
                    }
                    groups.push(index..end);
                    index = end;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    if groups.is_empty() {
        return None;
    }

    // Pair against borrowed group representatives. Admission never clones the payload.
    let mut index = 0;
    let mut ranges = groups.iter().peekable();
    let representatives = std::iter::from_fn(|| {
        let item = items.get(index)?;
        index = ranges
            .next_if(|range| range.start == index)
            .map_or(index + 1, |range| range.end);
        Some(item)
    });
    paired_tool_items(representatives).then_some(groups)
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
        && match &item["output"] {
            Value::String(_) => true,
            Value::Array(parts) => parts.iter().all(plain_result_part),
            _ => false,
        }
        && item.as_object().is_some_and(|object| {
            object
                .keys()
                .all(|key| matches!(key.as_str(), "type" | "call_id" | "output" | "id" | "name"))
        })
}

fn plain_result_part(part: &Value) -> bool {
    let (content, optional_detail) = match part["type"].as_str() {
        Some("input_text" | "output_text" | "text") => ("text", false),
        Some("input_image") => ("image_url", true),
        Some("input_audio") => ("audio_url", false),
        Some("refusal") => ("refusal", false),
        _ => return false,
    };
    part[content].is_string()
        && part.as_object().is_some_and(|fields| {
            fields.iter().all(|(field, value)| {
                field == "type"
                    || field == content
                    || (optional_detail && field == "detail" && value.is_string())
            })
        })
}

#[cfg(test)]
#[path = "responses_custom_results_tests.rs"]
mod tests;
