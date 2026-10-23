//! Minimal final-wire compatibility for parameter-less function calls.
//! Apply only to the upstream request copy, after source/owner/checkpoint proof.
use serde_json::{json, Value};

/// Some clients serialize parameter-less historical function calls as a blank
/// arguments string. Strict Responses servers require valid JSON. Do not repair
/// nonblank malformed JSON, synthesize missing calls/results, or touch signed
/// history: those require the original caller/upstream to resolve the failure.
pub(super) fn normalize_empty_function_arguments(body: &mut Value) {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for item in input {
        if item.get("type").and_then(Value::as_str) != Some("function_call") || opaque_call(item) {
            continue;
        }
        if item
            .get("arguments")
            .and_then(Value::as_str)
            .is_some_and(|raw| raw.trim().is_empty())
        {
            item["arguments"] = json!("{}");
        }
    }
}

fn opaque_call(item: &Value) -> bool {
    // Match the native identity/capability fields and preserve common signed
    // provider extensions too. A malformed non-null marker is kept untouched.
    [
        "encrypted_content",
        "signature",
        "thought_signature",
        "thoughtSignature",
    ]
    .iter()
    .any(|key| item.get(*key).is_some_and(|value| !value.is_null()))
        || item
            .get("encrypted_function_args")
            .is_some_and(|value| !value.is_null() && !value.as_array().is_some_and(Vec::is_empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_function_arguments_are_applied_only_on_codex_and_plan_wire_copies() {
        use axum::http::HeaderMap;
        let original = json!({"model":"model","input":[
            {"type":"function_call","call_id":"plain","name":"list_files","arguments":" \n"},
            {"type":"function_call_output","call_id":"plain","output":"existing result"},
            {"type":"function_call","call_id":"opaque","name":"send","arguments":"","encrypted_function_args":["message"]},
            {"type":"function_call_output","call_id":"opaque","output":"existing encrypted result"}
        ]});
        for transport in ["codex_http", "codex_ws", "plan"] {
            let mut wire = original.clone();
            let mut headers = HeaderMap::new();
            match transport {
                "codex_http" => {
                    super::super::codex_wire::normalize(&mut headers, &mut wire, true, false)
                        .unwrap()
                }
                "codex_ws" => {
                    super::super::codex_wire::normalize_websocket(&mut headers, &mut wire, true)
                        .unwrap()
                }
                _ => {
                    super::super::chatgpt_plan_wire::prepare(&mut headers, &mut wire, false)
                        .unwrap();
                }
            }
            assert_eq!(wire["input"][0]["arguments"], "{}", "{transport}");
            assert_eq!(wire["input"][2]["arguments"], "", "{transport}");
            assert_eq!(
                wire["input"][2]["encrypted_function_args"],
                json!(["message"])
            );
            assert_eq!(wire["input"][1]["output"], "existing result");
        }
        assert_eq!(
            original["input"][0]["arguments"], " \n",
            "the replay source is never edited"
        );
        let mut third_party = original.clone();
        super::super::codex_wire::normalize(&mut HeaderMap::new(), &mut third_party, false, false)
            .unwrap();
        assert_eq!(
            third_party, original,
            "API-key provider policy stays provider-owned"
        );
    }

    #[test]
    fn empty_function_arguments_normalize_only_blank_plain_history_and_keep_identity() {
        for raw in ["", " ", "\t\r\n", "\u{2003}"] {
            let original = json!({"model":"m","input":[
                {"type":"function_call","id":"fc_saved","call_id":"call_saved","namespace":"collaboration","name":"list_agents","arguments":raw,"encrypted_function_args":[],"status":"completed"},
                {"type":"function_call_output","call_id":"call_saved","output":"real client result"}
            ]});
            let mut wire = original.clone();
            normalize_empty_function_arguments(&mut wire);
            let mut expected = original.clone();
            expected["input"][0]["arguments"] = json!("{}");
            assert_eq!(wire, expected);
            assert_eq!(original["input"][0]["arguments"], raw);
            normalize_empty_function_arguments(&mut wire);
            assert_eq!(wire, expected, "normalization must be idempotent");
        }
    }

    #[test]
    fn empty_function_arguments_preserve_nonblank_json_and_nonstring_values() {
        for arguments in [
            json!("{}"),
            json!(" {\"text\":\"  \"} "),
            json!("not-json"),
            json!("{broken"),
            json!("\"\""),
            Value::Null,
            json!({}),
            json!([]),
        ] {
            let mut body =
                json!({"input":[{"type":"function_call","call_id":"c","arguments":arguments}]});
            let expected = body.clone();
            normalize_empty_function_arguments(&mut body);
            assert_eq!(body, expected);
        }
        let mut missing = json!({"input":[{"type":"function_call","call_id":"c"}]});
        let expected = missing.clone();
        normalize_empty_function_arguments(&mut missing);
        assert_eq!(missing, expected);
    }

    #[test]
    fn empty_function_arguments_never_rewrite_opaque_or_signed_call_history() {
        for (field, value) in [
            ("encrypted_function_args", json!(["cipher"])),
            ("encrypted_function_args", json!("malformed-marker")),
            ("encrypted_function_args", json!({"cipher":"opaque"})),
            ("encrypted_content", json!("cipher")),
            ("encrypted_content", json!("")),
            ("signature", json!("signed")),
            ("thought_signature", json!("signed")),
            ("thoughtSignature", json!("signed")),
        ] {
            let mut body = json!({"input":[{"type":"function_call","call_id":"c","arguments":""}]});
            body["input"][0][field] = value;
            let expected = body.clone();
            normalize_empty_function_arguments(&mut body);
            assert_eq!(body, expected, "opaque marker {field} must prevent edits");
        }
    }

    #[test]
    fn empty_function_arguments_do_not_touch_custom_tools_results_or_user_data() {
        let mut body = json!({
            "tools":[{"type":"function","name":"f","parameters":{"default":{"type":"function_call","arguments":""}}}],
            "input":[
                {"type":"custom_tool_call","call_id":"custom","input":"","arguments":""},
                {"type":"custom_tool_call_output","call_id":"custom","output":""},
                {"type":"function_call_output","call_id":"orphan","output":"","arguments":""},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"function_call arguments=\"\""}],"arguments":""},
                {"type":"tool_search_call","arguments":""},
                {"type":"agent_message","arguments":""}
            ]
        });
        let expected = body.clone();
        normalize_empty_function_arguments(&mut body);
        assert_eq!(body, expected);
        for mut body in [json!({"input":"hello"}), json!({}), Value::Null] {
            let expected = body.clone();
            normalize_empty_function_arguments(&mut body);
            assert_eq!(body, expected);
        }
    }
}
