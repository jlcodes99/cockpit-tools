//! Preserve Claude Code's message-level reminders and tool-result adjacency.
use serde_json::{json, Value};

pub(super) fn normalize(messages: &[Value]) -> Vec<Value> {
    let mut result = Vec::new();
    let mut pending_ids: Vec<String> = Vec::new();
    let mut reminders = Vec::new();
    for original in messages {
        if original["role"] == "system" {
            let text = if let Some(text) = original["content"].as_str() {
                vec![text]
            } else {
                original["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|part| part["type"] == "text")
                    .filter_map(|part| part["text"].as_str())
                    .collect()
            }
            .into_iter()
            .filter(|text| {
                !text.is_empty() && !text.trim_start().starts_with("x-anthropic-billing-header:")
            })
            .collect::<Vec<_>>()
            .join("\n");
            if !text.trim().is_empty() {
                let reminder = json!({"role":"user","content":format!("<system-reminder>\n{text}\n</system-reminder>")});
                if pending_ids.is_empty() {
                    result.push(reminder);
                } else {
                    reminders.push(reminder);
                }
            }
            continue;
        }
        let mut message = original.clone();
        if message["role"] == "user" && !pending_ids.is_empty() {
            if let Some(parts) = message["content"].as_array() {
                let tool_results = parts
                    .iter()
                    .filter(|part| part["type"] == "tool_result")
                    .collect::<Vec<_>>();
                if tool_results.len() == pending_ids.len() {
                    let mut ordered = Vec::new();
                    let mut used = std::collections::HashSet::new();
                    for id in &pending_ids {
                        if let Some((index, part)) =
                            tool_results.iter().enumerate().find(|(index, part)| {
                                !used.contains(index) && part["tool_use_id"] == *id
                            })
                        {
                            used.insert(index);
                            ordered.push((*part).clone());
                        }
                    }
                    if ordered.len() == pending_ids.len() {
                        ordered.extend(
                            parts
                                .iter()
                                .filter(|part| part["type"] != "tool_result")
                                .cloned(),
                        );
                        message["content"] = json!(ordered);
                    }
                }
            }
        }
        pending_ids.clear();
        if message["role"] == "assistant" {
            pending_ids.extend(
                message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|part| part["type"] == "tool_use")
                    .filter_map(|part| part["id"].as_str())
                    .map(str::to_owned),
            );
        }
        // Insert reminders after the leading tool results, before ordinary content.
        if !reminders.is_empty() {
            if let Some(parts) = message["content"].as_array() {
                let split = parts
                    .iter()
                    .position(|part| part["type"] != "tool_result")
                    .unwrap_or(parts.len());
                if split > 0 {
                    result.push(json!({"role":message["role"],"content":&parts[..split]}));
                }
                result.append(&mut reminders);
                if split < parts.len() {
                    result.push(json!({"role":message["role"],"content":&parts[split..]}));
                }
                continue;
            }
        }
        result.append(&mut reminders);
        result.push(message);
    }
    result.append(&mut reminders);
    result
}
