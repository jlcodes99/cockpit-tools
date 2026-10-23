//! A protocol envelope is not ownership evidence. The decoded opaque value still
//! passes through the shared IdentityStore before any account is selected.
use super::{invalid, unsupported, ServiceError};
use serde_json::Value;

pub(super) fn input(value: Option<&Value>) -> Result<Option<String>, ServiceError> {
    let raw = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(raw)) if raw.is_empty() => return Ok(None),
        Some(Value::String(raw)) => raw,
        _ => return Err(invalid("thinking.signature", "expected a string")),
    };
    let payload = if let Some((prefix, rest)) = raw.split_once('#') {
        match prefix.to_ascii_lowercase().as_str() {
            "gpt" | "openai" | "codex" => rest,
            "claude" | "anthropic" | "gemini" | "google" | "cais" | "claude-cais"
            | "claude_cais" | "ccmax" | "claude-code-max" | "claude_code_max" => {
                return Err(unsupported("thinking.signature provider"));
            }
            _ => raw,
        }
    } else {
        raw
    };
    if payload.is_empty()
        || payload.len() > 32 * 1024 * 1024
        || payload.chars().any(char::is_control)
    {
        return Err(invalid("thinking.signature", "invalid opaque signature"));
    }
    // Preserve raw GPT and Grok encrypted_content exactly. Unknown values are
    // rejected by ownership admission, rather than dropping a reasoning block.
    Ok(Some(payload.to_owned()))
}
