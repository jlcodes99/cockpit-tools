use super::*;
use serde_json::json;

fn noise(length: usize) -> Vec<u8> {
    (0..length).map(|i| ((i * 41 + 17) % 251) as u8).collect()
}

fn put_varint(out: &mut Vec<u8>, mut value: usize) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn wrapped(number: usize, bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    put_varint(&mut result, (number << 3) | 2);
    put_varint(&mut result, bytes.len());
    result.extend_from_slice(bytes);
    result
}

// Mirrors testClaudeThinkingSignatureWithOpaqueLen in CPA's grok_validation_test.go.
fn classic(opaque_length: usize) -> String {
    let mut channel = vec![8, 12, 16, 2];
    channel.extend(wrapped(6, b"claude-sonnet-4-6"));
    let mut payload = wrapped(2, &wrapped(1, &channel));
    payload.extend([24, 1]);
    payload.extend(wrapped(4, &noise(opaque_length)));
    PADDED.encode(payload)
}

fn cais_fixture() -> String {
    for suffix in 0..8 {
        let mut channel = vec![8, 16, 24, 2];
        channel.extend(wrapped(5, &noise(64)));
        channel.extend(wrapped(
            6,
            format!("claude-opus-5{}", "x".repeat(suffix)).as_bytes(),
        ));
        channel.extend([56, 1]);
        channel.extend(wrapped(8, b"thinking"));
        channel.extend(wrapped(11, b"d9743975-4bb0-4936-9e28-d5b0d21bdc48"));
        let mut container = wrapped(1, &channel);
        container.extend(wrapped(9, &noise(256)));
        let mut payload = vec![8, 2];
        payload.extend(wrapped(2, &container));
        payload.extend([24, 1]);
        let encoded = PADDED.encode(payload);
        if !encoded.contains('=') {
            return encoded;
        }
    }
    panic!("CAIS fixture must reach the unpadded foreign-envelope check")
}

#[test]
fn grok_shape_accepts_opaque_state_without_claiming_decryptability() {
    for length in [32, 43, 48, 50, 255, 256, 3254, 3256, 9708, 9710] {
        assert!(valid(&RAW.encode(noise(length))), "length {length}");
    }
    // An envelope-looking prefix alone is not foreign provenance.
    for prefix in [0x08, 0x12, 0x45, 0x80] {
        let mut bytes = noise(256);
        bytes[0] = prefix;
        assert!(valid(&RAW.encode(bytes)));
    }
    let mut bytes = noise(128);
    assert!(valid(&RAW.encode(&bytes)));
    bytes[90] ^= 1;
    assert!(
        valid(&RAW.encode(bytes)),
        "local shape validation cannot detect cipher mutation"
    );
}

#[test]
fn grok_shape_rejects_invalid_transport() {
    for raw in [
        "",
        "bad",
        " opaque",
        "AAAAA",
        "abcd_efg",
        "gAAAAABinvalid-gpt-shape",
    ] {
        assert!(!valid(raw), "{raw}");
    }
    let good = RAW.encode(noise(64));
    for raw in [
        format!(" {good}"),
        format!("{good}\n"),
        format!("{good}="),
        format!("claude#{good}"),
        "A".repeat(MAX_ENCODED_BYTES + 1),
    ] {
        assert!(!valid(&raw));
    }
    assert!(!valid(&RAW.encode(noise(31))));
    assert!(!valid(&RAW.encode(vec![0xa5; 64])));
}

#[test]
fn grok_shape_matches_cpa_foreign_fixture_categories() {
    let e = classic(35);
    let r = PADDED.encode(classic(41));
    let cais = cais_fixture();
    // Same Gemini 3 field-2 -> field-1 fixture as CPA's testGeminiThoughtSignatureEnvelope.
    let mut gemini_payload = vec![1, 12];
    gemini_payload.extend(0..97);
    let gemini = RAW.encode(wrapped(2, &wrapped(1, &gemini_payload)));
    for (name, raw) in [
        ("classic Claude E", e),
        ("classic Claude R", r),
        ("Claude CAIS", cais),
        ("Gemini 3", gemini),
    ] {
        assert!(
            !raw.contains('='),
            "{name} fixture must bypass the padding rejection"
        );
        assert!(!valid(&raw), "{name}");
    }
    for length in [3255, 9709] {
        assert!(!valid(&RAW.encode(noise(length))), "Kimi length {length}");
    }
    // CPA explicitly no longer identifies the retired Gemini 2.5 field-1 envelope.
    let mut retired = vec![1];
    retired.extend(0..127);
    assert!(valid(&RAW.encode(wrapped(1, &retired))));
}

#[test]
fn grok_shape_keeps_go_noncanonical_trailing_bit_compatibility() {
    let canonical = RAW.encode(noise(32));
    let mut bytes = canonical.into_bytes();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let last = alphabet
        .iter()
        .position(|b| *b == *bytes.last().unwrap())
        .unwrap();
    *bytes.last_mut().unwrap() = alphabet[last | 1];
    assert!(valid(std::str::from_utf8(&bytes).unwrap()));
}

#[test]
fn grok_sanitize_changes_only_invalid_direct_cipher_fields() {
    let good = RAW.encode(noise(128));
    let original = json!({"input":[
        {"type":"reasoning","id":"rs_bad","encrypted_content":"bad","summary":[{"type":"summary_text","text":"keep"}],"vendor":true},
        {"type":"compaction","encrypted_content":null},
        {"type":"reasoning","id":"rs_good","encrypted_content":good,"summary":[],"vendor":123},
        {"type":"compaction","id":"cmp_good","encrypted_content":good},
        {"type":"reasoning","summary":[]},
        {"type":"function_call_output","output":{"type":"compaction","encrypted_content":"bad"}},
        {"type":"other","encrypted_content":false}
    ],"tools":[{"type":"function","parameters":{"type":"reasoning","encrypted_content":"bad"}}],"vendor":{"input":[{"type":"compaction","encrypted_content":"bad"}]}});
    let mut body = original.clone();
    sanitize(&mut body);
    assert_eq!(body["input"].as_array().unwrap().len(), 6);
    assert!(body["input"][0].get("encrypted_content").is_none());
    assert_eq!(body["input"][0]["summary"], original["input"][0]["summary"]);
    assert_eq!(body["input"][0]["id"], "rs_bad");
    assert_eq!(body["input"][0]["vendor"], true);
    for (after, before) in [(1, 2), (2, 3), (3, 4), (4, 5), (5, 6)] {
        assert_eq!(body["input"][after], original["input"][before]);
    }
    assert_eq!(body["tools"], original["tools"]);
    assert_eq!(body["vendor"], original["vendor"]);
    assert_eq!(original["input"][0]["encrypted_content"], "bad");
}

#[test]
fn grok_sanitize_rejects_nonstring_ciphers_but_keeps_missing_fields() {
    for cipher in [Value::Null, json!(false), json!(42), json!({}), json!([])] {
        let mut body = json!({"input":[{"type":"reasoning","encrypted_content":cipher,"summary":[]},{"type":"compaction","encrypted_content":cipher},{"type":"compaction"}]});
        sanitize(&mut body);
        assert_eq!(
            body,
            json!({"input":[{"type":"reasoning","summary":[]},{"type":"compaction"}]})
        );
    }
    for mut body in [Value::Null, json!({}), json!({"input":"text"})] {
        let before = body.clone();
        sanitize(&mut body);
        assert_eq!(body, before);
    }
}
