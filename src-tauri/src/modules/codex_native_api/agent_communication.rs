//! Official MultiAgentV2 message arguments become encrypted agent_message input.
use serde_json::{json, Map, Value};

/// Match Codex ToolCall::direct_source and agent_message_from_tool. The absence
/// of encrypted_function_args is not a plaintext signal: only an explicit empty
/// array selects DirectPlaintextMessage for these three collaboration tools.
/// This extracts a reference, never authorizes it. Input references still need
/// an observed owner or a validated complete-history replay.
pub(super) fn encrypted_message(item: &Value) -> Option<String> {
    if !message_call(item) {
        return None;
    }
    match item.get("encrypted_function_args") {
        None | Some(Value::Null) => {}
        Some(Value::Array(values)) if !values.is_empty() && values.iter().all(Value::is_string) => {
        }
        // Invalid wire types cannot supply evidence of an encrypted message.
        _ => return None,
    }
    let arguments: Value = serde_json::from_str(item.get("arguments")?.as_str()?).ok()?;
    arguments
        .get("message")?
        .as_str()
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
}

fn message_call(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("function_call")
        && item.get("namespace").and_then(Value::as_str) == Some("collaboration")
        && matches!(
            item.get("name").and_then(Value::as_str),
            Some("spawn_agent" | "send_message" | "followup_task")
        )
}

/// Only call at a transport boundary known to return plaintext tool arguments.
/// An upstream's explicit encryption evidence remains untouched. This marker
/// makes the official client deliver a plaintext task rather than ciphertext.
pub(super) fn declare_plaintext_message(item: &mut Value) {
    if message_call(item)
        && item
            .get("encrypted_function_args")
            .is_none_or(Value::is_null)
        && item.get("encrypted_content").is_none_or(Value::is_null)
        && item.get("signature").is_none_or(Value::is_null)
    {
        item["encrypted_function_args"] = serde_json::json!([]);
    }
}

/// Match the pre-Rust Go provider gateway's portable message projection.
/// This retains literal bytes and routing information; it never decrypts data.
/// Only call after ownership admission, on a provider's per-attempt wire copy.
pub(super) fn portable_message(item: &mut Value) -> Result<(), &'static str> {
    let Some(parts) = item.get_mut("content").and_then(Value::as_array_mut) else {
        return Err("Agent message content must be an array");
    };
    for part in parts.iter_mut() {
        if part.get("type").and_then(Value::as_str) == Some("encrypted_content") {
            let content = part
                .get("encrypted_content")
                .and_then(Value::as_str)
                .ok_or("Agent message encrypted content must be text")?
                .to_owned();
            part["type"] = json!("input_text");
            part["text"] = json!(content);
            part.as_object_mut().unwrap().remove("encrypted_content");
        }
    }
    let object = item.as_object_mut().unwrap();
    let mut metadata = Map::new();
    for key in [
        "author",
        "recipient",
        "id",
        "internal_chat_message_metadata_passthrough",
    ] {
        if let Some(value) = object.remove(key) {
            metadata.insert(key.to_owned(), value);
        }
    }
    if !metadata.is_empty() {
        object.get_mut("content").unwrap().as_array_mut().unwrap().push(json!({
            "type":"input_text", "text":format!("Agent routing metadata: {}", Value::Object(metadata))
        }));
    }
    object.insert("type".into(), json!("message"));
    object.insert("role".into(), json!("user"));
    Ok(())
}
