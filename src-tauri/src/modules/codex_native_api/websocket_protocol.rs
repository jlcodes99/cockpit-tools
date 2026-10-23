//! Narrow compatibility for older downstream WebSocket request envelopes.
//! Official Responses v2 creates remain complete requests; only `response.append`
//! and a create that omits its model opt into the legacy shorthand defaults.
use serde_json::Value;

use super::ServiceError;

pub(super) fn normalize_create(
    mut body: Value,
    previous: Option<&Value>,
    last_response_id: Option<&str>,
) -> Result<Value, ServiceError> {
    let object = body.as_object_mut().ok_or_else(|| {
        invalid(
            "invalid_websocket_message",
            "WebSocket request must be a JSON object",
        )
    })?;
    let append = match object.get("type").and_then(Value::as_str) {
        Some("response.create") => false,
        Some("response.append")
            if previous.is_some()
                || object
                    .get("previous_response_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.trim().is_empty()) =>
        {
            true
        }
        Some("response.append") => {
            return Err(invalid(
                "websocket_create_required",
                "Send response.create before response.append",
            ));
        }
        _ => {
            return Err(invalid(
                "unsupported_websocket_event",
                "Expected response.create or response.append",
            ));
        }
    };
    let shorthand = append || !object.contains_key("model");
    if !object.contains_key("model") {
        if let Some(model) = previous.and_then(|previous| previous.get("model")) {
            object.insert("model".to_owned(), model.clone());
        }
    }
    if !object
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|model| !model.trim().is_empty())
    {
        return Err(invalid(
            "model_required",
            "A nonempty model is required in the first response.create",
        ));
    }
    match object.get("input") {
        Some(Value::Array(_)) => {}
        None if previous.is_none() && !append => {
            object.insert("input".to_owned(), Value::Array(Vec::new()));
        }
        _ => {
            return Err(invalid(
                "invalid_websocket_input",
                "WebSocket response input must be an array",
            ));
        }
    }
    if shorthand && !object.contains_key("instructions") {
        if let Some(instructions) = previous.and_then(|previous| previous.get("instructions")) {
            object.insert("instructions".to_owned(), instructions.clone());
        }
    }
    if shorthand {
        for field in [
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "reasoning",
            "text",
            "max_output_tokens",
            "temperature",
            "top_p",
        ] {
            if !object.contains_key(field) {
                if let Some(value) = previous.and_then(|previous| previous.get(field)) {
                    object.insert(field.to_owned(), value.clone());
                }
            }
        }
    }
    if let Some(value) = object.get("previous_response_id") {
        if !value.is_null() && !value.is_string() {
            return Err(invalid(
                "invalid_previous_response_id",
                "previous_response_id must be a string or null",
            ));
        }
    }
    if append {
        // Never reconstruct history or infer a checkpoint from the previous
        // request. The caller supplies only an actually completed response ID.
        let explicit = object
            .get("previous_response_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty());
        let checkpoint = explicit
            .or_else(|| last_response_id.filter(|id| !id.trim().is_empty()))
            .ok_or_else(|| {
                ServiceError::new(
                    409,
                    "append_checkpoint_required",
                    "response.append requires a completed response checkpoint",
                )
            })?;
        object.insert(
            "previous_response_id".to_owned(),
            Value::String(checkpoint.to_owned()),
        );
    }
    object.insert(
        "type".to_owned(),
        Value::String("response.create".to_owned()),
    );
    object.insert("stream".to_owned(), Value::Bool(true));
    Ok(body)
}

fn invalid(code: &'static str, message: &str) -> ServiceError {
    ServiceError::new(400, code, message)
}

/// Identifies a rejected interrupt control message, never a response failure or
/// ordinary inference error. A generic `error` needs an explicit interrupt marker.
pub(super) fn is_interrupt_rejection(event: &Value) -> bool {
    match event.get("type").and_then(Value::as_str) {
        Some("response.interrupt.failed") => return true,
        Some("error") => {}
        _ => return false,
    }
    let Some(error) = event.get("error").filter(|error| error.is_object()) else {
        return false;
    };
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let code = code.to_ascii_lowercase();
    matches!(
        code.as_str(),
        "interrupt_not_supported"
            | "unsupported_interrupt"
            | "invalid_interrupt"
            | "interrupt_failed"
            | "interrupt_rejected"
            | "response_interrupt_not_supported"
    ) || ["code", "param", "message"]
        .into_iter()
        .filter_map(|field| error.get(field).and_then(Value::as_str))
        .any(|value| value.to_ascii_lowercase().contains("response.interrupt"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_create_requires_model_and_supplies_empty_input() {
        let first = normalize_create(
            json!({"type":"response.create","model":"model","extension":{"v":1},"stream":false}),
            None,
            None,
        )
        .unwrap();
        assert_eq!(first["input"], json!([]));
        assert_eq!(first["stream"], true);
        assert_eq!(first["extension"], json!({"v":1}));
        for request in [
            json!({"type":"response.create"}),
            json!({"type":"response.create","model":" "}),
            json!({"type":"response.create","model":null}),
            json!({"type":"response.append","model":"model","input":[]}),
        ] {
            assert!(normalize_create(request, None, Some("real-id")).is_err());
        }
        let append = normalize_create(json!({"type":"response.append","model":"model","input":[],"previous_response_id":"real-id"}), None, None).unwrap();
        assert_eq!(append["previous_response_id"], "real-id");
        assert_eq!(append["type"], "response.create");
    }

    #[test]
    fn shorthand_inherits_execution_settings_without_inventing_continuation() {
        let previous = json!({"type":"response.create","model":"model","instructions":"rules","input":["old"],"generate":false,"client_metadata":{"turn_id":"old-turn"},"tools":["old-tool"],"previous_response_id":"older-id","extension":"old"});
        let current = json!({"type":"response.create","input":[],"extension":"new"});
        let normalized = normalize_create(current, Some(&previous), Some("completed-id")).unwrap();
        assert_eq!(normalized["model"], "model");
        assert_eq!(normalized["instructions"], "rules");
        assert_eq!(normalized["extension"], "new");
        assert_eq!(normalized["input"], json!([]));
        assert_eq!(normalized["tools"], json!(["old-tool"]));
        for field in ["generate", "client_metadata", "previous_response_id"] {
            assert!(normalized.get(field).is_none(), "inherited {field}");
        }
    }

    #[test]
    fn append_uses_only_explicit_or_completed_checkpoint_and_keeps_delta() {
        let previous = json!({"model":"model","instructions":"rules","input":["old"],"previous_response_id":"stale"});
        let request = json!({"type":"response.append","input":[{"type":"function_call_output","call_id":"call","output":"result"}],"extension":true});
        let appended =
            normalize_create(request.clone(), Some(&previous), Some("completed")).unwrap();
        assert_eq!(appended["type"], "response.create");
        assert_eq!(appended["previous_response_id"], "completed");
        assert_eq!(appended["input"], request["input"]);
        assert_eq!(appended["instructions"], "rules");
        let mut explicit = request.clone();
        explicit["previous_response_id"] = json!("explicit");
        assert_eq!(
            normalize_create(explicit, Some(&previous), Some("completed")).unwrap()
                ["previous_response_id"],
            "explicit"
        );
        assert_eq!(
            normalize_create(request, Some(&previous), None)
                .unwrap_err()
                .code,
            "append_checkpoint_required"
        );
    }

    #[test]
    fn full_official_create_is_not_merged_or_reinterpreted_as_append() {
        let previous = json!({"model":"model","instructions":"old instructions","input":["old"],"client_metadata":{"turn_id":"old"},"generate":false});
        let request = json!({"type":"response.create","model":"model","stream":true,"input":[{"type":"context_compaction","encrypted_content":"cipher"}],"client_metadata":{"turn_id":"new"},"generate":true});
        assert_eq!(
            normalize_create(request.clone(), Some(&previous), Some("completed")).unwrap(),
            request
        );
        let mut cleared = request;
        cleared["previous_response_id"] = Value::Null;
        assert!(
            normalize_create(cleared, Some(&previous), Some("completed")).unwrap()
                ["previous_response_id"]
                .is_null()
        );
    }

    #[test]
    fn invalid_input_and_explicit_invalid_fields_are_not_hidden_by_defaults() {
        let previous = json!({"model":"model","instructions":"rules"});
        for request in [
            json!({"type":"response.create","input":"text"}),
            json!({"type":"response.create"}),
            json!({"type":"response.create","model":null,"input":[]}),
            json!({"type":"response.append","input":[],"previous_response_id":5}),
            json!({"type":"response.interrupt","input":[]}),
            Value::Null,
        ] {
            assert!(normalize_create(request, Some(&previous), Some("completed")).is_err());
        }
        let request = json!({"type":"response.append","input":[],"instructions":null});
        assert!(
            normalize_create(request, Some(&previous), Some("completed")).unwrap()["instructions"]
                .is_null()
        );
    }

    #[test]
    fn interrupt_rejections_need_explicit_control_evidence() {
        for event in [
            json!({"type":"response.interrupt.failed","response_id":"r"}),
            json!({"type":"error","error":{"code":"interrupt_not_supported"}}),
            json!({"type":"error","error":{"code":"unsupported_event","param":"response.interrupt"}}),
            json!({"type":"error","error":{"code":"invalid_value","param":"type","message":"Unsupported event type: response.interrupt"}}),
        ] {
            assert!(is_interrupt_rejection(&event), "{event}");
        }
        for event in [
            json!({"type":"error","error":{"code":"rate_limit_exceeded","message":"Try again later"}}),
            json!({"type":"error","error":{"code":"generation_interrupted","param":"response_id"}}),
            json!({"type":"error","error":{"code":"invalid_value","param":"type"}}),
            json!({"type":"response.failed","response":{"error":{"code":"server_error","message":"Unexpected response.interrupt"}}}),
            json!({"type":"response.output_text.delta","delta":"response.interrupt"}),
        ] {
            assert!(!is_interrupt_rejection(&event), "{event}");
        }
    }
}
