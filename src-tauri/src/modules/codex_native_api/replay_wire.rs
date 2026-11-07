//! Wire copies of upstream output; client input and persisted provenance stay intact.
use serde_json::Value;

pub(super) fn upstream_item(item: &Value) -> Value {
    let mut item = item.clone();
    if let Some(object) = item.as_object_mut() {
        if let Some(metadata) = object
            .get_mut("internal_chat_message_metadata_passthrough")
            .and_then(Value::as_object_mut)
        {
            metadata.remove("content_token_count");
            if metadata.is_empty() {
                object.remove("internal_chat_message_metadata_passthrough");
            }
        }
    }
    item
}

/// Gateway-injected instructions are internal; echo the caller's original JSON value.
pub(super) fn instructions(source: &Value, response: &mut Value) -> bool {
    let target = if response.get("response").is_some_and(Value::is_object) {
        &mut response["response"]
    } else {
        response
    };
    let Some(object) = target.as_object_mut() else {
        return false;
    };
    if !object.contains_key("instructions") {
        return false;
    }
    let original = source.get("instructions").unwrap_or(&Value::Null);
    if object.get("instructions") == Some(original) {
        return false;
    }
    object.insert("instructions".into(), original.clone());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn cleans_only_the_upstream_copy_and_preserves_other_metadata() {
        let source = json!({"type":"function_call","arguments":"exact","internal_chat_message_metadata_passthrough":{"content_token_count":3,"turn_id":"t"}});
        let clean = upstream_item(&source);
        assert_eq!(
            source["internal_chat_message_metadata_passthrough"]["content_token_count"],
            3
        );
        assert_eq!(
            clean["internal_chat_message_metadata_passthrough"],
            json!({"turn_id":"t"})
        );
        assert_eq!(clean["arguments"], "exact");
    }
    #[test]
    fn instruction_echo_retains_missing_null_empty_and_explicit_values() {
        for source in [
            json!({}),
            json!({"instructions":null}),
            json!({"instructions":""}),
            json!({"instructions":"original"}),
        ] {
            let mut event =
                json!({"type":"response.completed","response":{"instructions":"internal"}});
            assert!(instructions(&source, &mut event));
            assert_eq!(event["response"]["instructions"], source["instructions"]);
        }
    }
}
