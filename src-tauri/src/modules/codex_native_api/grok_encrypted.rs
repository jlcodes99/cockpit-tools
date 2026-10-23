//! Transport-shape checks matching CPA's `InspectGrokEncryptedContent`.
//!
//! Grok state has no identifying envelope. Passing these checks establishes
//! neither its provider nor its owner or decryptability. Call only after the
//! route has established Grok provenance; never decode/re-encode accepted state.

use base64::{
    alphabet,
    engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
    Engine,
};
use serde_json::Value;

#[path = "codex_cpa_wire_foreign.rs"]
mod cpa_foreign;

/// Shared transport-shape probes; no plaintext, provenance or replay grant is inferred.
pub(super) fn known_signature_provider(raw: &str, gpt: fn(&str) -> bool) -> bool {
    cpa_foreign::known_signature_provider(raw, gpt)
}

const MAX_ENCODED_BYTES: usize = 8 * 1024 * 1024;
const MIN_DECODED_BYTES: usize = 32;
const MIN_ENTROPY_RATIO: f64 = 0.85;
// Go's base64 decoders permit nonzero unused trailing bits unless Strict() is
// requested. Keep that compatibility, while forbidding padding on Grok state.
const RAW: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);
const PADDED: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

pub(super) fn valid(raw: &str) -> bool {
    if raw.is_empty()
        || raw.len() > MAX_ENCODED_BYTES
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return false;
    }
    let Ok(decoded) = RAW.decode(raw) else {
        return false;
    };
    if decoded.len() < MIN_DECODED_BYTES {
        return false;
    }
    if matches!(raw.as_bytes()[0], b'C' | b'E' | b'R' | b'g')
        && (raw.starts_with("gAAAA")
            || classic_claude(raw, &decoded)
            || (raw.len() % 4 == 0 && cais(&decoded))
            || gemini(&decoded))
    {
        return false;
    }
    let entropy = entropy_ratio(&decoded);
    // Kimi has no magic envelope either. CPA recognizes these two observed
    // lengths, with the same entropy floor. Do not generalize them to buckets.
    if matches!(raw.len(), 4340 | 12946) && entropy >= MIN_ENTROPY_RATIO {
        return false;
    }
    entropy >= MIN_ENTROPY_RATIO
}

/// Sanitize only direct protocol-owned input items on the caller's cloned body.
/// Missing encrypted_content remains untouched, as in CPA. A rejected reasoning
/// cipher loses only that field; a rejected compaction cannot be replayed at all.
pub(super) fn sanitize(body: &mut Value) {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    items.retain_mut(|item| {
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if !matches!(kind, "reasoning" | "compaction") {
            return true;
        }
        let Some(cipher) = item.get("encrypted_content") else {
            return true;
        };
        if cipher.as_str().is_some_and(valid) {
            return true;
        }
        if kind == "compaction" {
            return false;
        }
        if let Some(item) = item.as_object_mut() {
            item.remove("encrypted_content");
        }
        true
    });
}

fn entropy_ratio(bytes: &[u8]) -> f64 {
    if bytes.len() <= 1 {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &byte in bytes {
        counts[byte as usize] += 1;
    }
    let n = bytes.len() as f64;
    let entropy = counts
        .iter()
        .filter(|&&count| count != 0)
        .map(|&count| {
            let p = count as f64 / n;
            -p * p.log2()
        })
        .sum::<f64>();
    entropy / (bytes.len().min(256) as f64).log2()
}

// Foreign-envelope parsing is bounded and allocation-free. Unknown protobuf
// fields are skipped; malformed trees do not establish foreign provenance.
#[derive(Clone, Copy)]
struct Field<'a> {
    number: u64,
    wire: u8,
    bytes: &'a [u8],
}

fn varint(input: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *input.get(*offset)?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

fn field<'a>(input: &'a [u8], offset: &mut usize, depth: usize) -> Option<Field<'a>> {
    if depth > 64 {
        return None;
    }
    let tag = varint(input, offset)?;
    let number = tag >> 3;
    let wire = (tag & 7) as u8;
    if number == 0 || number > 0x1fff_ffff {
        return None;
    }
    let mut bytes = &input[*offset..*offset];
    match wire {
        0 => {
            varint(input, offset)?;
        }
        1 | 5 => {
            let length = if wire == 1 { 8 } else { 4 };
            *offset = offset.checked_add(length)?;
            if *offset > input.len() {
                return None;
            }
        }
        2 => {
            let length = usize::try_from(varint(input, offset)?).ok()?;
            let end = offset.checked_add(length)?;
            bytes = input.get(*offset..end)?;
            *offset = end;
        }
        3 => loop {
            let start = *offset;
            let child_tag = varint(input, offset)?;
            if child_tag & 7 == 4 {
                if child_tag >> 3 != number {
                    return None;
                }
                break;
            }
            *offset = start;
            field(input, offset, depth + 1)?;
        },
        _ => return None,
    }
    Some(Field {
        number,
        wire,
        bytes,
    })
}

fn walk<'a>(input: &'a [u8], mut visit: impl FnMut(Field<'a>) -> bool) -> bool {
    let mut offset = 0;
    while offset < input.len() {
        let Some(next) = field(input, &mut offset, 0) else {
            return false;
        };
        if !visit(next) {
            return false;
        }
    }
    true
}

fn bytes_field(input: &[u8], number: u64) -> Option<&[u8]> {
    let mut result = None;
    if !walk(input, |f| {
        if f.number == number {
            if f.wire != 2 {
                return false;
            }
            result = Some(f.bytes);
        }
        true
    }) {
        return None;
    }
    result
}

fn only_bytes_field(input: &[u8], number: u64) -> Option<&[u8]> {
    let mut offset = 0;
    let f = field(input, &mut offset, 0)?;
    (offset == input.len() && f.number == number && f.wire == 2).then_some(f.bytes)
}

fn decode_standard(input: &[u8]) -> Option<Vec<u8>> {
    if input.iter().any(|b| matches!(b, b'\r' | b'\n')) {
        PADDED
            .decode(
                input
                    .iter()
                    .copied()
                    .filter(|b| !matches!(b, b'\r' | b'\n'))
                    .collect::<Vec<_>>(),
            )
            .ok()
    } else {
        PADDED.decode(input).ok()
    }
}

fn classic_claude(raw: &str, decoded: &[u8]) -> bool {
    if raw.len() % 4 != 0 {
        return false;
    }
    let inner;
    let payload = match raw.as_bytes()[0] {
        b'E' => decoded,
        b'R' if decoded.first() == Some(&b'E') => {
            let Some(value) = decode_standard(decoded) else {
                return false;
            };
            inner = value;
            &inner
        }
        _ => return false,
    };
    if payload.first() != Some(&0x12) {
        return false;
    }
    let Some(container) = bytes_field(payload, 2) else {
        return false;
    };
    let Some(channel) = bytes_field(container, 1) else {
        return false;
    };
    let mut have_channel = false;
    walk(channel, |f| match f.number {
        1 => {
            have_channel = f.wire == 0;
            have_channel
        }
        2 | 7 => f.wire == 0,
        6 => f.wire == 2 && std::str::from_utf8(f.bytes).is_ok(),
        _ => true,
    }) && have_channel
}

fn cais(decoded: &[u8]) -> bool {
    if decoded.first() != Some(&0x08) {
        return false;
    }
    if !walk(decoded, |f| !matches!(f.number, 1 | 3) || f.wire == 0) {
        return false;
    }
    let Some(container) = bytes_field(decoded, 2) else {
        return false;
    };
    let Some(channel) = bytes_field(container, 1) else {
        return false;
    };
    let (mut have_channel, mut have_signature, mut have_model) = (false, false, false);
    walk(channel, |f| match f.number {
        1 => {
            have_channel = f.wire == 0;
            have_channel
        }
        3 | 7 => f.wire == 0,
        5 => {
            have_signature = f.wire == 2 && !f.bytes.is_empty();
            have_signature
        }
        6 => {
            have_model =
                f.wire == 2 && std::str::from_utf8(f.bytes).is_ok_and(|s| s.starts_with("claude-"));
            have_model
        }
        8 => f.wire == 2 && std::str::from_utf8(f.bytes).is_ok(),
        11 => f.wire == 2 && uuid(f.bytes),
        _ => true,
    }) && have_channel
        && have_signature
        && have_model
}

fn gemini(decoded: &[u8]) -> bool {
    let Some(container) = only_bytes_field(decoded, 2) else {
        return false;
    };
    let Some(payload) = only_bytes_field(container, 1) else {
        return false;
    };
    payload.first() == Some(&1) || uuid(payload)
}

fn uuid(bytes: &[u8]) -> bool {
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
#[path = "grok_encrypted_tests.rs"]
mod tests;
