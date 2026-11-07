//! CPA's marker denotes public commentary text, never encrypted reasoning.
use serde_json::{json, Value};
use std::collections::HashSet;
pub(super) const MARKER: &str = "cpa:commentary:v1";

pub(super) fn item(item: &Value) -> Option<Value> {
    if item["type"] != "message" || item["phase"] != "commentary" {
        return None;
    }
    let summary: Vec<_> = item["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str().or_else(|| part["refusal"].as_str()))
        .map(|text| json!({"type":"summary_text","text":text}))
        .collect();
    let mut value = json!({"type":"reasoning","summary":summary,"encrypted_content":MARKER});
    for key in ["id", "status"] {
        if let Some(v) = item.get(key) {
            value[key] = v.clone();
        }
    }
    Some(value)
}

#[derive(Default)]
pub(super) struct Commentary {
    ids: HashSet<String>,
    indices: HashSet<u64>,
}
impl Commentary {
    pub fn event(&mut self, event: &Value) -> Option<Value> {
        let mut event = event.clone();
        let mut changed = false;
        if let Some(mapped) = event.get("item").and_then(item) {
            if let Some(id) = event.pointer("/item/id").and_then(Value::as_str) {
                self.ids.insert(id.to_owned());
            }
            if let Some(index) = event["output_index"].as_u64() {
                self.indices.insert(index);
            }
            event["item"] = mapped;
            changed = true;
        }
        if event["item_id"]
            .as_str()
            .is_some_and(|id| self.ids.contains(id))
            || event["output_index"]
                .as_u64()
                .is_some_and(|index| self.indices.contains(&index))
        {
            let mapped = match event["type"].as_str() {
                Some("response.output_text.delta") => Some("response.reasoning_summary_text.delta"),
                Some("response.output_text.done") => Some("response.reasoning_summary_text.done"),
                Some("response.content_part.added") => {
                    Some("response.reasoning_summary_part.added")
                }
                Some("response.content_part.done") => Some("response.reasoning_summary_part.done"),
                _ => None,
            };
            if let Some(kind) = mapped {
                event["type"] = json!(kind);
                if event["part"].is_object() {
                    event["part"]["type"] = json!("summary_text");
                }
                if let Some(index) = event.get("content_index").cloned() {
                    event["summary_index"] = index;
                }
                changed = true;
            }
        }
        if let Some(output) = event
            .pointer_mut("/response/output")
            .and_then(Value::as_array_mut)
        {
            for value in output {
                if let Some(mapped) = item(value) {
                    *value = mapped;
                    changed = true;
                }
            }
        }
        changed.then_some(event)
    }
}
