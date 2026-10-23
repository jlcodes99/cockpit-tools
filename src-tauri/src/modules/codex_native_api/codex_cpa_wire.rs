//! CPA executor wire-only reasoning and input-ID rules. Call on a cloned body
//! after ownership admission; these helpers never authorize opaque state.
use base64::{
    alphabet,
    engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
    Engine,
};
use serde_json::{json, Value};

#[path = "codex_cpa_wire_ids.rs"]
mod ids;
pub(super) fn sanitize_input_ids(body: &mut Value) {
    ids::sanitize_input_ids(body);
}

const GPT_MAX_BYTES: usize = 32 * 1024 * 1024;

#[cfg(test)]
pub(super) fn synthetic_gpt_signature(seed: u8) -> String {
    let mut bytes = vec![seed; 73];
    bytes[0] = 0x80;
    bytes[1..9].fill(0);
    base64::engine::general_purpose::URL_SAFE.encode(bytes)
}
const GPT_RAW: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);
const GPT_PADDED: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

/// Fernet transport shape only, not cryptographic validity or ownership.
pub(super) fn gpt_signature_valid(raw: &str) -> bool {
    let raw = raw.trim();
    if raw.len() > GPT_MAX_BYTES
        || !raw.starts_with("gAAAA")
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'='))
    {
        return false;
    }
    let Ok(decoded) = GPT_RAW.decode(raw).or_else(|_| GPT_PADDED.decode(raw)) else {
        return false;
    };
    decoded.len() >= 73 && decoded[0] == 0x80 && (decoded.len() - 57) % 16 == 0
}

fn bool_value(value: &Value) -> bool {
    match value {
        Value::Bool(value) => *value,
        Value::String(value) => {
            matches!(value.to_ascii_lowercase().as_str(), "1" | "t" | "true")
        }
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        _ => false,
    }
}
fn wire_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}

/// Mirrors sanitizeOpenAIResponsesReasoningEncryptedContentWithCompat. Run
/// before multi-agent optimization on Codex/SIWC executor bodies. Generic
/// Responses passthrough must opt into this executor policy explicitly.
pub(super) fn sanitize_reasoning(body: &mut Value, is_compat: bool) {
    let strip_orphan_ids = !bool_value(&body["store"]);
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for item in input {
        if item["type"].as_str().map(str::trim) != Some("reasoning") {
            continue;
        }
        if !is_compat {
            if let Some(content) = item
                .get("content")
                .and_then(Value::as_array)
                .filter(|parts| !parts.is_empty())
            {
                if item.get("summary").is_none_or(|summary| {
                    summary.is_null() || summary.as_array().is_some_and(Vec::is_empty)
                }) {
                    let summary: Vec<_> = content
                        .iter()
                        .filter(|part| {
                            part["type"].as_str().map(str::trim) == Some("reasoning_text")
                        })
                        .filter_map(|part| {
                            let text = wire_text(&part["text"]);
                            (!text.is_empty()).then(|| json!({"type":"summary_text","text":text}))
                        })
                        .collect();
                    if !summary.is_empty() {
                        item["summary"] = json!(summary);
                    }
                }
                item["content"] = json!([]);
            }
        }
        let keep = item.get("encrypted_content").is_some_and(|cipher| {
            cipher.as_str().is_some_and(|raw| {
                raw == raw.trim()
                    && (gpt_signature_valid(raw)
                        || (is_compat
                            && !raw.is_empty()
                            && !super::grok_wire::known_signature_provider(
                                raw,
                                gpt_signature_valid,
                            )))
            })
        });
        let Some(object) = item.as_object_mut() else {
            continue;
        };
        if !keep {
            object.remove("encrypted_content");
            if !is_compat && strip_orphan_ids {
                object.remove("id");
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_cpa_wire_tests.rs"]
mod tests;
