//! Translate Codex's inline compact control into xAI's compact operation.
use super::ServiceError;
use serde_json::{json, Value};

pub(in crate::modules::codex_native_api) fn triggered(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .any(|item| item["type"] == "compaction_trigger")
        })
}

pub(super) fn prepare(body: &mut Value) {
    if let Some(object) = body.as_object_mut() {
        for key in [
            "stream",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "max_output_tokens",
            "temperature",
            "top_p",
            "top_k",
            "stop",
        ] {
            object.remove(key);
        }
    }
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.retain(|item| item["type"] != "compaction_trigger");
    }
}

pub(in crate::modules::codex_native_api) fn events(
    mut response: Value,
    model: &str,
) -> Result<Vec<Value>, ServiceError> {
    let item = response
        .get("output")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .filter(|item| {
            matches!(
                item["type"].as_str(),
                Some("compaction" | "compaction_summary")
            ) && item["encrypted_content"]
                .as_str()
                .is_some_and(|blob| !blob.is_empty())
        })
        .ok_or_else(|| {
            ServiceError::new(
                502,
                "invalid_grok_compaction",
                "Grok compact response is missing its compaction state",
            )
        })?;
    let id = response
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            ServiceError::new(
                502,
                "invalid_grok_compaction",
                "Grok compact response is missing its response ID",
            )
        })?
        .to_owned();
    let mut item = item;
    item["type"] = json!("compaction");
    response["id"] = json!(id);
    response["object"] = json!("response");
    response["model"] = json!(model);
    response["status"] = json!("completed");
    response["output"] = json!([item]);
    let mut started = response.clone();
    started["status"] = json!("in_progress");
    started["output"] = json!([]);
    Ok(vec![
        json!({"type":"response.created","sequence_number":0,"response":started}),
        json!({"type":"response.in_progress","sequence_number":1,"response":started}),
        json!({"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":item}),
        json!({"type":"response.output_item.done","sequence_number":3,"output_index":0,"item":item}),
        json!({"type":"response.completed","sequence_number":4,"response":response}),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grok_compact_control_is_not_sent_as_history_or_executed_as_normal_turn() {
        let mut body = json!({"model":"grok","stream":true,"tools":[{"type":"function"}],"tool_choice":"required","parallel_tool_calls":true,"max_output_tokens":100,"temperature":0.5,"top_p":0.8,"top_k":40,"stop":["stop"],"input":[{"type":"message","role":"user","content":"keep"},{"type":"compaction_trigger"}],"previous_response_id":"resp"});
        assert!(triggered(&body));
        prepare(&mut body);
        assert!(!triggered(&body));
        assert_eq!(
            body,
            json!({"model":"grok","input":[{"type":"message","role":"user","content":"keep"}],"previous_response_id":"resp"})
        );
    }

    #[test]
    fn grok_compact_events_keep_blob_usage_and_id_without_fabricating_success() {
        let response = json!({"id":"cmp-id","object":"response.compaction","output":[{"type":"compaction","id":"item-id","encrypted_content":"original+/blob"}],"usage":{"input_tokens":50,"output_tokens":5}});
        let events = events(response.clone(), "grok-4.7").unwrap();
        let completed = &events.last().unwrap()["response"];
        assert_eq!(completed["id"], response["id"]);
        assert_eq!(completed["output"], response["output"]);
        assert_eq!(completed["usage"], response["usage"]);
        assert_eq!(completed["status"], "completed");
        for invalid in [
            json!({"id":"x","output":[]}),
            json!({"id":"x","output":[{"type":"message"}]}),
            json!({"id":"x","output":[{"type":"compaction","encrypted_content":""}]}),
        ] {
            assert!(super::events(invalid, "grok").is_err());
        }
    }
}
