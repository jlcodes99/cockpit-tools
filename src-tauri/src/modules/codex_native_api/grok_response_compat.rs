//! Normalize xAI reasoning events to the Responses summary shape before identity observation.
use serde_json::{json, Value};

fn summary_index(event: &mut Value) {
    let Some(object) = event.as_object_mut() else {
        return;
    };
    if let Some(index) = object.remove("content_index") {
        object.entry("summary_index").or_insert(index);
    }
}

fn item(item: &mut Value) {
    if item.get("type").and_then(Value::as_str) != Some("reasoning") {
        return;
    }
    if let Some(summary) = item.get_mut("summary").and_then(Value::as_array_mut) {
        for part in summary {
            normalize_part(part);
        }
    }
    let mut parts: Vec<Value> = item
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("reasoning_text"))
        .cloned()
        .collect();
    if !parts.is_empty() {
        for part in &mut parts {
            normalize_part(part);
        }
        let mut summary = item
            .get("summary")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for part in parts {
            if !summary.contains(&part) {
                summary.push(part);
            }
        }
        item["summary"] = json!(summary);
        // Opaque or future content parts are not reasoning text and must survive.
        let content = item
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .unwrap();
        content.retain(|part| part.get("type").and_then(Value::as_str) != Some("reasoning_text"));
        if content.is_empty() {
            item.as_object_mut().unwrap().remove("content");
        }
    }
}

fn normalize_part(part: &mut Value) {
    if part.get("type").and_then(Value::as_str) == Some("reasoning_text") {
        part["type"] = json!("summary_text");
    }
}

fn response(response: &mut Value) {
    if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) {
        for value in output {
            item(value);
        }
    }
}

pub(super) fn normalize(event: &mut Value) -> Option<Value> {
    let kind = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut done = None;
    match kind.as_str() {
        "response.reasoning_text.delta" => {
            event["type"] = json!("response.reasoning_summary_text.delta");
            summary_index(event);
        }
        "response.reasoning_text.done" => {
            let mut text_done = event.clone();
            text_done["type"] = json!("response.reasoning_summary_text.done");
            summary_index(&mut text_done);
            done = Some(text_done);
            event["type"] = json!("response.reasoning_summary_part.done");
            let text = event.get("text").cloned();
            if !event.get("part").is_some_and(Value::is_object) {
                event["part"] = json!({});
            }
            event["part"]["type"] = json!("summary_text");
            if let Some(text) = text {
                event["part"]["text"] = text;
            }
            event.as_object_mut().unwrap().remove("text");
            summary_index(event);
        }
        "response.content_part.added" | "response.content_part.done"
            if event.pointer("/part/type").and_then(Value::as_str) == Some("reasoning_text") =>
        {
            event["type"] = json!(if kind.ends_with("added") {
                "response.reasoning_summary_part.added"
            } else {
                "response.reasoning_summary_part.done"
            });
            normalize_part(&mut event["part"]);
            summary_index(event);
        }
        _ => {}
    }
    if let Some(value) = event.get_mut("item") {
        item(value);
    }
    if let Some(value) = event.get_mut("response") {
        response(value);
    }
    response(event);
    done
}

#[cfg(test)]
#[path = "grok_response_compat_tests.rs"]
mod tests;
