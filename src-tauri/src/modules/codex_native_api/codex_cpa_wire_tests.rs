use super::*;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

fn gpt() -> String {
    let mut bytes = vec![0; 73];
    bytes[0] = 0x80;
    for (index, value) in bytes.iter_mut().enumerate().skip(9) {
        *value = index as u8;
    }
    URL_SAFE_NO_PAD.encode(bytes)
}
fn wrap(number: u8, bytes: &[u8]) -> Vec<u8> {
    let mut output = vec![(number << 3) | 2];
    let mut length = bytes.len();
    while length >= 128 {
        output.push((length as u8 & 127) | 128);
        length >>= 7;
    }
    output.push(length as u8);
    output.extend_from_slice(bytes);
    output
}
fn caqs(wrapped: bool) -> String {
    let mut channel = vec![8, 18, 16, 2];
    channel.extend(wrap(8, b"thinking"));
    let mut container = wrap(1, &channel);
    container.extend(wrap(5, &[1; 64]));
    let mut payload = vec![8, 4];
    payload.extend(wrap(2, &container));
    let raw = STANDARD.encode(payload);
    if wrapped {
        STANDARD.encode(raw)
    } else {
        raw
    }
}
fn foreign_samples() -> Vec<String> {
    let mut channel = vec![8, 12, 16, 2];
    channel.extend(wrap(6, b"claude-sonnet-4-6"));
    let classic = STANDARD.encode(wrap(2, &wrap(1, &channel)));
    let gemini = STANDARD_NO_PAD.encode(wrap(2, &wrap(1, &[1, 9, 10, 11])));
    let mut invocation = vec![8, 1];
    invocation.extend(wrap(2, &[1, 19, 20]));
    let gemini_tool = STANDARD_NO_PAD.encode(wrap(2, &wrap(1, &invocation)));
    let kimi = STANDARD_NO_PAD.encode(
        (0..3255)
            .map(|index| ((index * 41 + 17) % 251) as u8)
            .collect::<Vec<_>>(),
    );
    vec![
        classic.clone(),
        STANDARD.encode(&classic),
        caqs(false),
        caqs(true),
        gemini.clone(),
        gemini_tool,
        kimi,
        format!("claude#{classic}"),
        format!("google#{gemini}"),
        format!("gpt#{}", gpt()),
        "sealed.v1.opaque".into(),
        "swe#sealed.v1.opaque".into(),
        "skip_thought_signature_validator".into(),
        "context_engineering_is_the_way_to_go".into(),
    ]
}

#[test]
fn cpa_wire_gpt_shape_is_bounded_and_keeps_go_base64_padding_behavior() {
    let signature = gpt();
    assert!(gpt_signature_valid(&signature));
    let bytes = URL_SAFE_NO_PAD.decode(&signature).unwrap();
    assert!(gpt_signature_valid(&URL_SAFE.encode(&bytes)));
    let mut malformed = bytes.clone();
    malformed.pop();
    assert!(!gpt_signature_valid(&URL_SAFE_NO_PAD.encode(malformed)));
    assert!(!gpt_signature_valid("gAAAAinvalid"));
    assert!(!gpt_signature_valid(&format!(
        "gAAAA{}",
        "A".repeat(GPT_MAX_BYTES)
    )));
    let mut noncanonical = signature.into_bytes();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let position = alphabet
        .iter()
        .position(|byte| *byte == *noncanonical.last().unwrap())
        .unwrap();
    *noncanonical.last_mut().unwrap() = alphabet[position | 1];
    assert!(gpt_signature_valid(
        std::str::from_utf8(&noncanonical).unwrap()
    ));
}

#[test]
fn cpa_wire_reasoning_keeps_valid_cipher_and_promotes_summary_only_when_empty() {
    let original = json!({"store":false,"input":[
        {"type":"reasoning","id":"rs_valid","encrypted_content":gpt(),"summary":[],"content":[{"type":"reasoning_text","text":"exact thinking\n"},{"type":"other","text":"skip"}]},
        {"type":"reasoning","id":"rs_keep_summary","summary":[{"type":"summary_text","text":"existing"}],"content":[{"type":"reasoning_text","text":"not replacement"}]},
        {"type":"function_call_output","output":{"type":"reasoning","encrypted_content":"bad"}},
        {"type":"agent_message","content":[{"type":"encrypted_content","encrypted_content":"agent-message"}]}
    ],"tools":[{"parameters":{"type":"reasoning","encrypted_content":"user schema"}}]});
    let mut body = original.clone();
    sanitize_reasoning(&mut body, false);
    assert_eq!(
        body["input"][0]["encrypted_content"],
        original["input"][0]["encrypted_content"]
    );
    assert_eq!(
        body["input"][0]["summary"],
        json!([{"type":"summary_text","text":"exact thinking\n"}])
    );
    assert_eq!(body["input"][0]["content"], json!([]));
    assert_eq!(body["input"][1]["summary"], original["input"][1]["summary"]);
    assert!(body["input"][1].get("id").is_none());
    assert_eq!(body["input"][2], original["input"][2]);
    assert_eq!(body["input"][3], original["input"][3]);
    assert_eq!(body["tools"], original["tools"]);
    assert_eq!(
        original["input"][0]["content"][0]["text"],
        "exact thinking\n"
    );
}

#[test]
fn cpa_wire_reasoning_is_compat_and_store_are_independent_gates() {
    for compatible in [false, true] {
        for stored in [false, true] {
            let original = json!({"store":stored,"input":[
                {"type":"reasoning","id":"rs_custom","encrypted_content":"unknown-third-party-state","content":[{"type":"reasoning_text","text":"reason"}]},
                {"type":"reasoning","id":"rs_missing"}, {"type":"reasoning","id":"rs_null","encrypted_content":null},
                {"type":"reasoning","id":"rs_whitespace","encrypted_content":format!(" {}",gpt())}
            ]});
            let mut body = original.clone();
            sanitize_reasoning(&mut body, compatible);
            assert_eq!(
                body["input"][0].get("encrypted_content").is_some(),
                compatible
            );
            assert_eq!(
                body["input"][0]["content"] == original["input"][0]["content"],
                compatible
            );
            for index in 0..4 {
                assert_eq!(
                    body["input"][index].get("id").is_some(),
                    stored || compatible
                );
            }
            for index in [2, 3] {
                assert!(body["input"][index].get("encrypted_content").is_none());
            }
        }
    }
}

#[test]
fn cpa_wire_compat_keeps_unknown_and_rejects_all_recognized_foreign_shapes() {
    for raw in foreign_samples() {
        assert!(
            super::super::grok_wire::known_signature_provider(&raw, gpt_signature_valid),
            "unrecognized fixture {raw}"
        );
        let mut body =
            json!({"input":[{"type":"reasoning","id":"retained","encrypted_content":raw}]});
        sanitize_reasoning(&mut body, true);
        assert!(body["input"][0].get("encrypted_content").is_none());
        assert_eq!(body["input"][0]["id"], "retained");
    }
    for raw in [
        "unknown opaque",
        "gAAAAbad",
        "claude#claude#bad",
        "unknown#sealed.v1.blob",
        "sealed#invalid",
        "C-not-envelope",
    ] {
        let mut body = json!({"input":[{"type":"reasoning","encrypted_content":raw}]});
        sanitize_reasoning(&mut body, true);
        assert_eq!(body["input"][0]["encrypted_content"], raw);
    }
}

#[test]
fn cpa_wire_reasoning_never_touches_nonarray_input_or_foreign_data() {
    for mut body in [
        Value::Null,
        json!({}),
        json!({"input":"reasoning"}),
        json!({"input":[2,{"type":"reasoning","summary":"nonarray","content":[]}]} ),
    ] {
        let before = body.clone();
        sanitize_reasoning(&mut body, false);
        assert_eq!(body, before);
    }
    for invalid in [
        Value::Null,
        json!(false),
        json!(23),
        json!([]),
        json!({}),
        json!(""),
    ] {
        let mut body =
            json!({"input":[{"type":"reasoning","encrypted_content":invalid,"id":"rs_bad"}]});
        sanitize_reasoning(&mut body, false);
        assert_eq!(body, json!({"input":[{"type":"reasoning"}]}));
    }
}

#[test]
fn cpa_wire_input_ids_cover_every_prefix_without_changing_call_ids_or_data() {
    let kinds = [
        ("message", "msg"),
        ("reasoning", "rs"),
        ("function_call", "fc"),
        ("custom_tool_call", "ctc"),
        ("custom_tool_call_output", "ctco"),
    ];
    for (kind, prefix) in kinds {
        let original = json!({"input":[{"type":kind,"id":"amsg_original","call_id":"unchanged-call","arguments":"exact","output":{"id":"nested"}}]});
        let mut body = original.clone();
        sanitize_input_ids(&mut body);
        assert_eq!(body["input"][0]["id"], format!("{prefix}_amsg_original"));
        assert_eq!(body["input"][0]["call_id"], original["input"][0]["call_id"]);
        assert_eq!(body["input"][0]["output"], original["input"][0]["output"]);
        let once = body.clone();
        sanitize_input_ids(&mut body);
        assert_eq!(body, once);
    }
}

#[test]
fn cpa_wire_input_id_collisions_are_order_independent_and_idempotent() {
    for kind in [
        "message",
        "reasoning",
        "function_call",
        "custom_tool_call",
        "custom_tool_call_output",
    ] {
        for original in ["item_collision".to_owned(), "界".repeat(65)] {
            let prefixed = ids::normalize(&json!({"type":kind}), &original);
            for order in [
                [original.clone(), prefixed.clone()],
                [prefixed.clone(), original.clone()],
            ] {
                let mut body =
                    json!({"input":[{"type":kind,"id":order[0]},{"type":kind,"id":order[1]}]});
                sanitize_input_ids(&mut body);
                assert_ne!(body["input"][0]["id"], body["input"][1]["id"]);
                for index in 0..2 {
                    assert!(body["input"][index]["id"].as_str().unwrap().chars().count() <= 64);
                }
                let once = body.clone();
                sanitize_input_ids(&mut body);
                assert_eq!(body, once);
            }
        }
    }
}

#[test]
fn cpa_wire_input_id_hash_collisions_and_unicode_limit_match_cpa() {
    let long = "grok-item-".repeat(10);
    let reserved = ids::hashed(&long, 0);
    let mut body =
        json!({"input":[{"id":long},{"id":reserved},{"id":"a".repeat(64)},{"id":"界".repeat(65)}]});
    sanitize_input_ids(&mut body);
    assert_eq!(body["input"][0]["id"], ids::hashed(&long, 1));
    assert_eq!(body["input"][1]["id"], reserved);
    assert_eq!(body["input"][2]["id"], "a".repeat(64));
    assert_eq!(body["input"][3]["id"].as_str().unwrap().chars().count(), 64);
}

#[test]
fn cpa_wire_only_drops_long_original_id_reasoning_with_nonempty_cipher() {
    let original = json!({"input":[
        {"type":"reasoning","id":"rs_".to_string()+&"a".repeat(64),"encrypted_content":gpt(),"summary":[{"type":"summary_text","text":"source stays"}]},
        {"type":"reasoning","id":"rs_".to_string()+&"b".repeat(64)},
        {"type":"reasoning","id":"x".repeat(64),"encrypted_content":gpt()},
        {"type":"agent_message","id":"x".repeat(65),"encrypted_content":"agent"}
    ]});
    let mut body = original.clone();
    sanitize_input_ids(&mut body);
    assert_eq!(body["input"].as_array().unwrap().len(), 3);
    assert_eq!(original["input"].as_array().unwrap().len(), 4);
    assert_eq!(original["input"][0]["summary"][0]["text"], "source stays");
    assert_eq!(body["input"][1]["encrypted_content"], gpt());
    assert_eq!(body["input"][2]["encrypted_content"], "agent");
    for empty in [Value::Null, json!(""), json!({}), json!(true)] {
        let mut body =
            json!({"input":[{"type":"reasoning","id":"x".repeat(65),"encrypted_content":empty}]});
        sanitize_input_ids(&mut body);
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
    }
}
