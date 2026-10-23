//! CPA DetectSignatureProvider's known-provider gate, sharing bounded protobuf
//! parsing with the Grok adapter. Existing Grok validation remains unchanged.
use super::{
    classic_claude, decode_standard, entropy_ratio, field, gemini, only_bytes_field, uuid, varint,
    walk, Engine, RAW,
};

const MAX_BYTES: usize = 32 * 1024 * 1024;
fn bypass(raw: &str) -> bool {
    matches!(
        raw,
        "skip_thought_signature_validator" | "context_engineering_is_the_way_to_go"
    )
}

pub(super) fn known_signature_provider(raw: &str, gpt: fn(&str) -> bool) -> bool {
    let raw = raw.trim();
    if raw.is_empty() {
        return false;
    }
    if let Some((prefix, payload)) = raw.split_once('#') {
        let payload = payload.trim();
        if payload.contains('#') {
            return false;
        }
        return match prefix.trim().to_ascii_lowercase().as_str() {
            "openai" | "gpt" | "codex" => gpt(payload),
            "swe" | "sealed" => payload.starts_with("sealed.v1."),
            "gemini" | "google" => bypass(payload) || gemini_signature(payload),
            "claude" | "anthropic" | "cais" | "claude-cais" | "claude_cais" | "ccmax"
            | "claude-code-max" | "claude_code_max" => claude_signature(payload),
            _ => false,
        };
    }
    if bypass(raw) || raw.starts_with("sealed.v1.") {
        return true;
    }
    if raw.len() <= MAX_BYTES
        && matches!(raw.as_bytes()[0], b'C' | b'E' | b'Q' | b'R' | b'g')
        && (gpt(raw) || claude_signature(raw) || gemini_signature(raw))
    {
        return true;
    }
    // Kimi has no self-describing envelope. These exact observed lengths and
    // entropy threshold are CPA's final probe, never a general ciphertext rule.
    if !matches!(raw.len(), 4340 | 12946)
        || raw.contains('=')
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/'))
    {
        return false;
    }
    if raw.starts_with("gAAAA") {
        return false;
    }
    RAW.decode(raw)
        .is_ok_and(|decoded| entropy_ratio(&decoded) >= 0.85)
}

fn gemini_signature(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_BYTES {
        return false;
    }
    let stripped = raw.replace(['\r', '\n'], "");
    decode_standard(stripped.as_bytes())
        .or_else(|| RAW.decode(&stripped).ok())
        .is_some_and(|decoded| {
            if gemini(&decoded) {
                return true;
            }
            let Some(container) = only_bytes_field(&decoded, 2) else {
                return false;
            };
            let Some(payload) = only_bytes_field(container, 1) else {
                return false;
            };
            let mut tink = false;
            walk(payload, |field| {
                if field.wire == 2 && field.bytes.first() == Some(&1) {
                    tink = true;
                }
                matches!(field.wire, 0 | 1 | 2 | 5)
            }) && tink
        })
}

fn claude_signature(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_BYTES {
        return false;
    }
    match raw.as_bytes()[0] {
        b'Q' => {
            if raw.contains(['\r', '\n']) {
                return false;
            }
            let Ok(inner) = base64::engine::general_purpose::STANDARD.decode(raw) else {
                return false;
            };
            if inner.first() != Some(&b'C')
                || inner
                    .iter()
                    .any(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b'#'))
            {
                return false;
            }
            let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&inner) else {
                return false;
            };
            cais_payload(&decoded, true)
        }
        b'C' => {
            decode_standard(raw.as_bytes()).is_some_and(|decoded| cais_payload(&decoded, false))
        }
        b'E' | b'R' => {
            let clean = raw.replace(['\r', '\n'], "");
            decode_standard(clean.as_bytes())
                .is_some_and(|decoded| classic_claude(&clean, &decoded))
        }
        _ => false,
    }
}

fn number(input: &[u8], wanted: u64) -> Option<u64> {
    let mut offset = 0;
    let mut result = None;
    while offset < input.len() {
        let start = offset;
        let parsed = field(input, &mut offset, 0)?;
        if parsed.number == wanted {
            if parsed.wire != 0 {
                return None;
            }
            let mut cursor = start;
            varint(input, &mut cursor)?;
            result = varint(input, &mut cursor);
        }
    }
    result
}

fn message(input: &[u8], wanted: u64, unique: bool) -> Option<&[u8]> {
    let mut result = None;
    if !walk(input, |parsed| {
        if parsed.number == wanted {
            if parsed.wire != 2 || (unique && result.is_some()) {
                return false;
            }
            result = Some(parsed.bytes);
        }
        true
    }) {
        return None;
    }
    result
}

fn cais_payload(decoded: &[u8], wrapped: bool) -> bool {
    if decoded.first() != Some(&8)
        || !walk(decoded, |parsed| {
            !matches!(parsed.number, 1 | 3) || parsed.wire == 0
        })
    {
        return false;
    }
    let version = number(decoded, 1).unwrap_or_default();
    let Some(container) = message(decoded, 2, wrapped) else {
        return false;
    };
    if !walk(container, |parsed| {
        !matches!(parsed.number, 1 | 5) || parsed.wire == 2
    }) {
        return false;
    }
    let Some(channel) = message(container, 1, wrapped) else {
        return false;
    };
    let (mut have_channel, mut have_signature, mut have_model) = (false, false, false);
    let mut block = "";
    if !walk(channel, |parsed| match parsed.number {
        1 => {
            have_channel = parsed.wire == 0;
            have_channel
        }
        2 | 3 | 7 => parsed.wire == 0,
        5 => {
            have_signature = parsed.wire == 2 && !parsed.bytes.is_empty();
            have_signature
        }
        6 => {
            have_model = parsed.wire == 2
                && std::str::from_utf8(parsed.bytes).is_ok_and(|text| text.starts_with("claude-"));
            have_model
        }
        8 => {
            if parsed.wire != 2 {
                return false;
            }
            let Ok(text) = std::str::from_utf8(parsed.bytes) else {
                return false;
            };
            block = text;
            true
        }
        11 => parsed.wire == 2 && uuid(parsed.bytes),
        _ => true,
    }) {
        return false;
    }
    let container_signature = !have_signature
        && version >= 4
        && message(container, 5, false).is_some_and(|bytes| !bytes.is_empty());
    if !have_channel
        || (!have_signature && !container_signature)
        || (version < 4 && !have_model)
        || (version >= 4 && !matches!(block, "thinking" | "narration"))
    {
        return false;
    }
    !wrapped
        || (version == 4
            && number(channel, 1) == Some(18)
            && number(channel, 2) == Some(2)
            && block == "thinking"
            && container_signature)
}
