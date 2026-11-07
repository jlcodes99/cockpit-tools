//! Responses usage defaults shared by HTTP, SSE and WebSocket client output.
use serde_json::{Map, Value};

/// Call only for Responses output, not raw Chat/Claude/Gemini usage objects.
/// Compaction responses deliberately retain their original usage shape.
pub(super) fn normalize_usage_details(payload: &mut Value) -> bool {
    if payload.get("object").and_then(Value::as_str) == Some("response.compaction") {
        return false;
    }
    let mut changed = false;
    for path in ["/response/usage", "/usage"] {
        if let Some(usage) = payload.pointer_mut(path).and_then(Value::as_object_mut) {
            for (field, count) in [
                ("output_tokens_details", "reasoning_tokens"),
                ("input_tokens_details", "cached_tokens"),
            ] {
                let details = usage.entry(field).or_insert(Value::Null);
                if !details.is_object() {
                    *details = Value::Object(Map::new());
                    changed = true;
                }
                let details = details.as_object_mut().unwrap();
                if details.get(count).is_none_or(Value::is_null) {
                    details.insert(count.into(), Value::from(0));
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn responses_usage_defaults_keep_measured_counts_and_extensions() {
        let mut event = json!({"type":"response.completed","response":{"usage":{"input_tokens":11,"output_tokens":7,"input_tokens_details":{"cached_tokens":4,"image_tokens":2},"output_tokens_details":{"reasoning_tokens":3,"audio_tokens":1},"extension":true}}});
        let original = event.clone();
        assert!(!normalize_usage_details(&mut event));
        assert_eq!(event, original);
        let mut response = json!({"object":"response","usage":{"input_tokens":11,"output_tokens":7,"extension":true}});
        assert!(normalize_usage_details(&mut response));
        assert_eq!(
            response["usage"],
            json!({"input_tokens":11,"output_tokens":7,"extension":true,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}})
        );
        assert!(!normalize_usage_details(&mut response));
    }

    #[test]
    fn null_or_invalid_detail_objects_receive_defaults_without_replacing_existing_fields() {
        for details in [
            Value::Null,
            json!(false),
            json!([]),
            json!("invalid"),
            json!({"reasoning_tokens":null,"extension":true}),
        ] {
            let mut event = json!({"type":"response.incomplete","response":{"usage":{"output_tokens_details":details,"input_tokens_details":{"cached_tokens":null,"image_tokens":2}}}});
            assert!(normalize_usage_details(&mut event));
            assert_eq!(
                event["response"]["usage"]["output_tokens_details"]["reasoning_tokens"],
                0
            );
            assert_eq!(
                event["response"]["usage"]["input_tokens_details"],
                json!({"cached_tokens":0,"image_tokens":2})
            );
            if details.is_object() {
                assert_eq!(
                    event["response"]["usage"]["output_tokens_details"]["extension"],
                    true
                );
            }
        }
    }

    #[test]
    fn missing_usage_and_compaction_are_not_synthesized_or_rewritten() {
        for mut event in [
            json!({"type":"response.created","response":{"status":"in_progress"}}),
            json!({"usage":null}),
            json!({"usage":false}),
            json!({"object":"response.compaction","id":"compact","output":[],"usage":{"input_tokens":3,"output_tokens":2}}),
        ] {
            let original = event.clone();
            assert!(!normalize_usage_details(&mut event));
            assert_eq!(event, original);
        }
    }
}
