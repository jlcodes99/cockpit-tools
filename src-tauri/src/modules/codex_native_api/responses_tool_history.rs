//! Final-wire compatibility for historical Codex input items.
//! Apply only to the upstream request copy, after source/owner/checkpoint proof.
use std::collections::{HashMap, HashSet};

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

/// DeepSeek requires results immediately after the entire batch of calls.
/// Repair inline history only; continuation references may own results outside this request.
pub(super) fn normalize_deepseek_tool_history(body: &mut Value) {
    if !body["previous_response_id"].is_null() || !body["conversation"].is_null() {
        return;
    }
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    if input.iter().any(|item| item["type"] == "item_reference") {
        return;
    }
    let mut occurrences = HashMap::new();
    for (index, item) in input.iter().enumerate() {
        if let Some(id) = item["call_id"].as_str() {
            let (count, last) = occurrences.entry(id).or_insert((0usize, index));
            *count += 1;
            *last = index;
        }
    }
    let mut batches = HashMap::new();
    let mut moved_outputs = HashSet::new();
    let mut batch_start = 0;
    while batch_start < input.len() {
        let batch_len = input[batch_start..]
            .iter()
            .take_while(|item| {
                matches!(item["type"].as_str(), Some("function_call" | "custom_tool_call"))
            })
            .count();
        if batch_len == 0 {
            batch_start += 1;
            continue;
        }
        let batch_end = batch_start + batch_len;
        let pairs = (batch_start..batch_end)
            .map(|index| {
                let call = &input[index];
                let output_type = match call["type"].as_str() {
                    Some("function_call") if call["arguments"].is_string() => "function_call_output",
                    Some("custom_tool_call") if call["input"].is_string() => "custom_tool_call_output",
                    _ => return None,
                };
                let id = call["call_id"].as_str().filter(|id| !id.trim().is_empty())?;
                if !call["name"].as_str().is_some_and(|name| !name.is_empty()) || opaque_call(call) {
                    return None;
                }
                match occurrences[id] {
                    (1, _) if output_type == "function_call_output" => Some((id, None)),
                    (2, output) if output >= batch_end
                        && input[output]["type"] == output_type
                        && !opaque_call(&input[output]) =>
                    {
                        Some((id, Some(output)))
                    }
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>();
        batch_start = batch_end;
        let Some(pairs) = pairs else {
            continue;
        };
        let mut outputs: Vec<_> = pairs.iter().filter_map(|(_, output)| *output).collect();
        outputs.sort_unstable();
        let missing: Vec<_> = pairs
            .into_iter()
            .filter_map(|(id, output)| output.is_none().then(|| id.to_owned()))
            .collect();
        if missing.is_empty() && outputs.iter().copied().eq(batch_end..batch_end + outputs.len()) {
            continue;
        }
        moved_outputs.extend(outputs.iter().copied());
        batches.insert(batch_end - 1, (outputs, missing));
    }
    if batches.is_empty() {
        return;
    }
    let mut original = std::mem::take(input);
    input.reserve(original.len());
    for index in 0..original.len() {
        if moved_outputs.contains(&index) {
            continue;
        }
        input.push(original[index].take());
        if let Some((outputs, missing)) = batches.remove(&index) {
            input.extend(outputs.into_iter().map(|output| original[output].take()));
            for call_id in missing {
                // Let the model decide how to proceed with the incomplete call.
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": "Tool result unavailable: the client did not record a result for this call."
                }));
            }
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

/// The old relay used generic IDs for messages, custom calls and reasoning.
/// Remove them from complete inline items while preserving references, signed
/// items and the saved replay source.
pub(super) fn strip_legacy_codex_item_ids(body: &mut Value) {
    let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    let referenced: HashSet<String> = input
        .iter()
        .flat_map(|item| {
            [
                item.get("item_id"),
                (item["type"] == "item_reference")
                    .then(|| item.get("id"))
                    .flatten(),
            ]
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| id.starts_with("item_"))
            .map(str::to_owned)
        })
        .collect();
    for item in input {
        let inline = match item["type"].as_str() {
            Some("message") => {
                item["role"].is_string()
                    && (item["content"].is_string() || item["content"].is_array())
            }
            Some("custom_tool_call") => {
                item["call_id"].as_str().is_some_and(|id| !id.is_empty())
                    && item["name"].is_string()
                    && item["input"].is_string()
            }
            Some("reasoning") => {
                (item["content"].is_null()
                    || item["content"].as_array().is_some_and(Vec::is_empty))
                    && item["summary"].as_array().is_some_and(|parts| {
                        !parts.is_empty()
                            && parts.iter().all(|part| {
                                part["type"] == "summary_text" && part["text"].is_string()
                            })
                    })
            }
            _ => false,
        };
        if inline
            && !opaque_call(item)
            && item["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("item_") && !referenced.contains(id))
        {
            if let Some(object) = item.as_object_mut() {
                object.remove("id");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_history_leaves_opaque_ambiguous_and_server_owned_results_untouched() {
        let call = json!({"type":"function_call","call_id":"one","name":"read","arguments":"{}"});
        let mut signed = call.clone();
        signed["signature"] = json!("opaque");
        for mut body in [
            json!({"input":[call],"previous_response_id":"stored"}),
            json!({"input":[call],"conversation":{"id":"stored"}}),
            json!({"input":[call,{"type":"item_reference","id":"stored"}]}),
            json!({"input":[call,call]}),
            json!({"input":[signed]}),
            json!({"input":[call,{"type":"function_call_output","call_id":"one","output":"recorded"}]}),
        ] {
            let original = body.clone();
            normalize_deepseek_tool_history(&mut body);
            assert_eq!(body, original);
        }
    }

    #[test]
    fn deepseek_history_moves_custom_output_without_changing_other_items() {
        let call = json!({"type":"custom_tool_call","call_id":"edit","name":"edit","input":"fixture"});
        let output = json!({"type":"custom_tool_call_output","call_id":"edit","output":"result"});
        let hook = json!({"type":"message","role":"developer","content":"hook"});
        let note = json!({"type":"message","role":"user","content":"next"});
        let mut body = json!({"input":[call,hook,output,note]});
        let expected = json!({"input":[call,output,hook,note]});
        normalize_deepseek_tool_history(&mut body);
        assert_eq!(body, expected);
        normalize_deepseek_tool_history(&mut body);
        assert_eq!(body, expected);
    }

    #[test]
    fn official_replay_strips_complete_inline_legacy_ids_on_every_transport() {
        use super::super::codex_wire;
        use axum::http::HeaderMap;
        let original = json!({"model":"model","input":[
            {"type":"message","id":"item_message","role":"assistant","content":"previous message"},
            {"type":"custom_tool_call","id":"item_custom","call_id":"call_patch","name":"apply_patch","input":"fixture"},
            {"type":"custom_tool_call_output","call_id":"call_patch","output":{"id":"item_nested"}},
            {"type":"message","id":"msg_valid","role":"user","content":"continue"},
            {"type":"reasoning","id":"item_signed","encrypted_content":"opaque"},
            {"type":"reasoning","id":"item_summary","summary":[{"type":"summary_text","text":"previous summary"}]}
        ]});
        for mode in ["http", "sse", "compact", "websocket"] {
            let mut request = original.clone();
            let mut headers = HeaderMap::new();
            if mode == "sse" {
                request["stream"] = json!(true);
            }
            if mode == "websocket" {
                codex_wire::normalize_websocket(&mut headers, &mut request, true).unwrap();
            } else {
                codex_wire::normalize(&mut headers, &mut request, true, mode == "compact").unwrap();
            }
            let input = &request["input"];
            assert!(input[0].get("id").is_none(), "{mode}");
            assert!(input[1].get("id").is_none(), "{mode}");
            assert_eq!(input[1]["call_id"], "call_patch");
            assert_eq!(input[2]["call_id"], "call_patch");
            assert_eq!(input[2]["output"]["id"], "item_nested");
            assert_eq!(input[3]["id"], "msg_valid");
            assert_eq!(input[4], original["input"][4]);
            assert!(input[5].get("id").is_none(), "{mode}");
            assert_eq!(input[5]["summary"], original["input"][5]["summary"]);
        }
        let mut third_party = original.clone();
        codex_wire::normalize(&mut HeaderMap::new(), &mut third_party, false, false).unwrap();
        assert_eq!(third_party, original);
    }

    #[test]
    fn legacy_wire_repair_does_not_orphan_references_or_remove_an_opaque_or_incomplete_id() {
        let mut request = json!({"input":[
            {"type":"message","id":"item_shared","role":"assistant","content":"complete"},
            {"type":"item_reference","id":"item_shared"},
            {"type":"item_reference","id":"item_server_only"},
            {"type":"message","id":"item_missing_content","role":"assistant"},
            {"type":"message","id":"item_signed","role":"assistant","content":"complete","signature":"opaque"},
            {"type":"reasoning","id":"item_reasoning","summary":[{"type":"summary_text","text":"referenced"}]},
            {"type":"item_reference","id":"item_reasoning"},
            {"type":"reasoning","id":"item_empty","summary":[]},
            {"type":"reasoning","id":"item_malformed","summary":[{"type":"summary_text"}]},
            {"type":"reasoning","id":"item_native_content","summary":[{"type":"summary_text","text":"summary"}],
                "content":[{"type":"reasoning_text","text":"provider-specific reasoning"}]},
            {"type":"reasoning","id":"item_encrypted","summary":[{"type":"summary_text","text":"summary"}],
                "encrypted_content":"opaque"},
            {"type":"reasoning","id":"rs_official","summary":[{"type":"summary_text","text":"valid official ID"}]},
            {"type":"custom_tool_call","id":"item_call","call_id":"call-1","name":"apply_patch","input":"fixture"},
            {"type":"custom_tool_call_output","call_id":"call-1","item_id":"item_call","output":"done"}
        ]});
        let original = request.clone();
        strip_legacy_codex_item_ids(&mut request);
        assert_eq!(request, original);
    }

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
