use super::*;
use serde_json::json;

#[test]
fn native_xai_suffix_uses_real_model_capabilities_and_go_budget_thresholds() {
    for (model, suffix, expected) in [
        ("grok-4.3", "none", "none"),
        ("grok-4.3", "0", "none"),
        ("grok-4.3", "auto", "medium"),
        ("grok-4.3", "-1", "medium"),
        ("grok-4.3", "1", "low"),
        ("grok-4.3", "512", "low"),
        ("grok-4.3", "513", "low"),
        ("grok-4.3", "1024", "low"),
        ("grok-4.3", "1025", "medium"),
        ("grok-4.3", "8192", "medium"),
        ("grok-4.3", "08192", "medium"),
        ("grok-4.3", "+8192", "medium"),
        ("grok-4.3", "8193", "high"),
        ("grok-4.3", "24576", "high"),
        ("grok-4.3", "24577", "high"),
        ("grok-4.3", "9223372036854775807", "high"),
        ("grok-4.3", "minimal", "low"),
        ("grok-4.3", "xhigh", "high"),
        ("grok-4.3", "max", "high"),
        ("grok-4.5", "none", "low"),
        ("grok-4.5", "AUTO", "medium"),
        ("grok-4.6", "none", "low"),
        ("grok-4.6", "24577", "xhigh"),
        ("grok-4.6", "XHIGH", "xhigh"),
        ("grok-4.6", "max", "xhigh"),
        ("grok-4.7", "none", "low"),
        ("grok-4.7", "max", "xhigh"),
        ("grok-4.7", "24577", "xhigh"),
        ("grok-4.7-latest", "minimal", "low"),
        ("grok-4.7-build-fast", "max", "xhigh"),
        ("grok-build-0.1", "none", "none"),
        ("grok-4.20-multi-agent-0309", "auto", "medium"),
    ] {
        let source = format!("team/grok/{model}({suffix})");
        assert_eq!(
            parse_model(&source).unwrap(),
            (format!("team/grok/{model}"), Some(expected.into())),
            "{source}"
        );
    }
}

#[test]
fn suffix_parser_keeps_go_last_parenthesis_and_unknown_value_semantics() {
    for (source, base, expected) in [
        ("grok-4.3", "grok-4.3", None),
        ("grok-4.3(high", "grok-4.3(high", None),
        ("grok-4.3(high)-v2", "grok-4.3(high)-v2", None),
        ("grok-4.3()", "grok-4.3", None),
        ("grok-4.3( high)", "grok-4.3", None),
        ("grok-4.3(unknown)", "grok-4.3", None),
        ("grok-4.3(-2)", "grok-4.3", None),
        ("grok-4.3(9223372036854775808)", "grok-4.3", None),
        ("custom(v2)(high)", "custom(v2)", Some("high")),
        ("custom(auto)", "custom", Some("auto")),
        ("custom(1)", "custom", Some("minimal")),
        ("custom(24577)", "custom", Some("xhigh")),
        (
            "grok-4.20-0309-reasoning(high)",
            "grok-4.20-0309-reasoning",
            None,
        ),
        (
            "grok-4.20-0309-non-reasoning(high)",
            "grok-4.20-0309-non-reasoning",
            None,
        ),
    ] {
        assert_eq!(
            parse_model(source).unwrap(),
            (base.into(), expected.map(str::to_owned)),
            "{source}"
        );
    }
    assert_eq!(parse_model("(high)").unwrap_err().code, "model_required");
}

#[test]
fn suffix_overrides_effort_only_and_keeps_signed_history_and_summary_byte_values() {
    let mut body = json!({
        "model":"grok-4.3",
        "previous_response_id":"owner-response",
        "client_metadata":{"session_id":"mapped-root","thread_id":"mapped-child"},
        "reasoning":{"effort":"low","summary":"detailed","encrypted_content":"reasoning-config-cipher"},
        "input":[
            {"type":"reasoning","encrypted_content":"signed-owner-cipher","summary":[{"type":"summary_text","text":"history"}]},
            {"type":"function_call","call_id":"call-original","name":"tool","arguments":"{\"model\":\"grok-4.3(none)\"}","encrypted_function_args":"signed-tool-cipher"}
        ]
    });
    let mut expected = body.clone();
    expected["reasoning"]["effort"] = json!("medium");
    apply(&mut body, "namespace/public-alias(8192)", false).unwrap();
    assert_eq!(body, expected);
}

#[test]
fn responses_and_raw_chat_share_selected_alias_capabilities_without_cross_field_mutation() {
    for (selected, source, effort) in [
        ("grok-4.3", "public(none)", "none"),
        ("grok-4.5", "public(none)", "low"),
        ("grok-4.3", "public(32768)", "high"),
        ("grok-4.6", "public(32768)", "xhigh"),
        ("grok-4.6", "public(auto)", "medium"),
        ("grok-4.7", "public(none)", "low"),
        ("grok-4.7", "public(max)", "xhigh"),
    ] {
        let history = json!([{"role":"assistant","content":"original","reasoning_signature":"signed-owner-cipher"}]);
        let mut responses =
            json!({"model":selected,"reasoning":{"effort":"low","summary":"auto"},"input":history});
        apply(&mut responses, source, false).unwrap();
        assert_eq!(responses["reasoning"]["effort"], effort);
        assert_eq!(responses["reasoning"]["summary"], "auto");
        assert_eq!(responses["input"], history);
        let mut chat = json!({"model":selected,"reasoning_effort":"high","reasoning":{"summary":"auto"},"messages":history,"n":2});
        let mut expected = chat.clone();
        expected["reasoning_effort"] = json!(effort);
        apply(&mut chat, source, true).unwrap();
        assert_eq!(chat, expected);
    }
}

#[test]
fn absent_unknown_or_unsupported_suffix_preserves_existing_body_exactly() {
    for (selected, source) in [
        ("grok-4.3", "grok-4.3"),
        ("grok-4.3", "grok-4.3(unknown)"),
        ("grok-4.3", "grok-4.3(-2)"),
    ] {
        let original = json!({"model":selected,"reasoning":{"effort":"high","summary":"detailed"},"input":[{"type":"reasoning","encrypted_content":"signed-owner"}]});
        let mut body = original.clone();
        apply(&mut body, source, false).unwrap();
        assert_eq!(body, original, "{source}");
    }
}

#[test]
fn body_effort_without_suffix_clamps_codex_settings_for_each_xai_model() {
    for (selected, effort, expected) in [
        ("grok-4.3", "xhigh", "high"),
        ("grok-4.3", "max", "high"),
        ("grok-4.3", "minimal", "low"),
        ("grok-4.3", "auto", "medium"),
        ("grok-4.3", "none", "none"),
        ("grok-4.5", "none", "low"),
        ("grok-4.6", "xhigh", "xhigh"),
        ("grok-4.6", "max", "xhigh"),
        ("grok-4.6", "auto", "medium"),
        ("grok-4.7", "none", "low"),
        ("grok-4.7", "max", "xhigh"),
    ] {
        for chat in [false, true] {
            let mut body = if chat {
                json!({"model":selected,"reasoning_effort":effort,"messages":[{"role":"assistant","reasoning_signature":"signed-owner"}],"n":2})
            } else {
                json!({"model":selected,"reasoning":{"effort":effort,"summary":"detailed","encrypted_content":"signed-config"},"input":[{"type":"reasoning","encrypted_content":"signed-owner"}]})
            };
            let mut expected_body = body.clone();
            if chat {
                expected_body["reasoning_effort"] = json!(expected);
            } else {
                expected_body["reasoning"]["effort"] = json!(expected);
            }
            apply(&mut body, "public-alias", chat).unwrap();
            assert_eq!(body, expected_body, "{selected}/{effort}/chat={chat}");
        }
    }
}

#[test]
fn nonthinking_models_strip_only_generation_controls_even_without_suffix() {
    for source in [
        "public-alias",
        "public-alias(high)",
        "public-alias(unknown)",
    ] {
        for chat in [false, true] {
            let mut body = json!({
                "model":"grok-composer-2.5-fast",
                "reasoning_effort":"max",
                "reasoning":{"effort":"high","budget_tokens":8192,"budget":1024,"summary":"detailed","encrypted_content":"signed-config","encrypted_custom":"opaque-custom"},
                "input":[{"type":"reasoning","encrypted_content":"signed-owner","effort":"max","budget_tokens":16384}],
                "messages":[{"role":"assistant","reasoning_signature":"signed-owner"}]
            });
            let mut expected = body.clone();
            expected.as_object_mut().unwrap().remove("reasoning_effort");
            for key in ["effort", "budget_tokens", "budget"] {
                expected["reasoning"].as_object_mut().unwrap().remove(key);
            }
            apply(&mut body, source, chat).unwrap();
            assert_eq!(body, expected);
        }
    }
    for (key, value) in [
        ("effort", json!({"encrypted_content":"signed-owner"})),
        ("budget_tokens", json!({"encrypted_content":"signed-owner"})),
    ] {
        let mut body = json!({"model":"grok-composer-2.5-fast","reasoning":{"summary":"auto"}});
        body["reasoning"][key] = value;
        let original = body.clone();
        assert!(apply(&mut body, "public", false).is_err());
        assert_eq!(
            body, original,
            "invalid controls cannot hide ciphertext that cleanup removes"
        );
    }
}

#[test]
fn unsupported_body_effort_fails_before_mutation_but_valid_suffix_has_priority() {
    for effort in [
        json!("ultra"),
        json!("8192"),
        json!(8192),
        json!({"encrypted_content":"opaque"}),
    ] {
        let mut body = json!({"model":"grok-4.3","reasoning":{"effort":effort,"summary":"auto"},"input":[{"type":"reasoning","encrypted_content":"signed-owner"}]});
        let original = body.clone();
        assert_eq!(
            apply(&mut body, "public", false).unwrap_err().code,
            "invalid_reasoning_effort"
        );
        assert_eq!(body, original);
        apply(&mut body, "public(8192)", false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["input"], original["input"]);
    }
    let mut body =
        json!({"model":"custom-grok-model","reasoning":{"effort":"ultra","summary":"auto"}});
    let original = body.clone();
    apply(&mut body, "public", false).unwrap();
    assert_eq!(
        body, original,
        "unknown models retain user-defined effort semantics"
    );
}

#[test]
fn malformed_reasoning_errors_without_overwriting_existing_payload() {
    for reasoning in [
        json!("opaque-signed-value"),
        json!([{"encrypted_content":"cipher"}]),
    ] {
        let mut body = json!({"model":"grok-4.3","reasoning":reasoning,"input":[]});
        let original = body.clone();
        assert_eq!(
            apply(&mut body, "grok-4.3(high)", false).unwrap_err().code,
            "invalid_request"
        );
        assert_eq!(body, original);
    }
    let mut body = json!({"model":"grok-4.3","reasoning":null,"input":[]});
    apply(&mut body, "grok-4.3(high)", false).unwrap();
    assert_eq!(body["reasoning"], json!({"effort":"high"}));
}
