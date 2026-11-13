//! Project client-injected receipts onto the public HTTP input schema.
//! Only mutate an outbound copy, never the saved history or a native socket frame.
use serde_json::{json, Value};

pub(super) fn normalize_external_results(body: &mut Value) {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    for item in items {
        if item["type"] != "function_call_output"
            || !match item.get("call_id") {
                None | Some(Value::Null) => true,
                Some(Value::String(id)) => id.trim().is_empty(),
                _ => false,
            }
        {
            continue;
        }
        let label = ["namespace", "name"]
            .into_iter()
            .filter_map(|key| item[key].as_str().filter(|text| !text.is_empty()))
            .collect::<Vec<_>>()
            .join(".");
        // This is protocol context, not a UI label or a model-issued tool call.
        let marker = if label.is_empty() {
            "[External tool result]".to_owned()
        } else {
            format!("[External tool result: {label}]")
        };
        let content = match item.get("output") {
            Some(Value::String(output)) => {
                vec![json!({"type":"input_text","text":format!("{marker}\n{output}")})]
            }
            Some(Value::Array(parts))
                if parts.iter().all(|part| {
                    matches!(
                        part["type"].as_str(),
                        Some("input_text" | "input_image" | "input_file")
                    )
                }) =>
            {
                let mut content = Vec::with_capacity(parts.len() + 1);
                content.push(json!({"type":"input_text","text":marker}));
                content.extend(parts.iter().cloned());
                content
            }
            // Malformed payloads and private encrypted parts retain their
            // validation errors; never turn missing data into a successful result.
            _ => continue,
        };
        *item = json!({"type":"message","role":"user","content":content});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_results_preserve_content_pairings_and_response_continuation() {
        for id in [None, Some(Value::Null), Some(json!("")), Some(json!(" \t"))] {
            for output in [
                json!("page context\n  exact whitespace  "),
                json!([]),
                json!([
                    {"type":"input_text","text":"selection"},
                    {"type":"input_image","image_url":"data:image/png;base64,AAAA","detail":"low"},
                    {"type":"input_file","file_data":"data:application/pdf;base64,AAAA","filename":"fixture.pdf"}
                ]),
            ] {
                let mut receipt = json!({"type":"function_call_output","namespace":"agent",
                    "name":"codex_apps_open_page","id":"fco_receipt","output":output});
                if let Some(id) = &id {
                    receipt["call_id"] = id.clone();
                }
                let mut body = json!({"model":"m","previous_response_id":"stored-response","input":[
                    {"type":"function_call","call_id":"real","name":"read","arguments":"{}"},
                    receipt,
                    {"type":"function_call_output","call_id":"real","output":"real output"},
                    {"type":"function_call_output","call_id":"from-previous-response","output":"prior output"}
                ]});
                let original = body.clone();
                normalize_external_results(&mut body);
                assert_eq!(
                    body["previous_response_id"],
                    original["previous_response_id"]
                );
                for index in [0, 2, 3] {
                    assert_eq!(body["input"][index], original["input"][index]);
                }
                let message = &body["input"][1];
                assert_eq!(message["type"], "message");
                assert_eq!(message["role"], "user");
                assert!(message.get("call_id").is_none());
                assert!(message.get("id").is_none());
                if let Some(parts) = output.as_array() {
                    assert_eq!(&message["content"].as_array().unwrap()[1..], parts);
                } else {
                    assert_eq!(
                        message["content"][0]["text"],
                        format!(
                            "[External tool result: agent.codex_apps_open_page]\n{}",
                            output.as_str().unwrap()
                        )
                    );
                }
                let once = body.clone();
                normalize_external_results(&mut body);
                assert_eq!(body, once);
            }
        }
    }

    #[test]
    fn external_results_do_not_reinterpret_malformed_or_opaque_payloads() {
        let mut body = json!({"input":[
            {"type":"function_call_output","output":null},
            {"type":"function_call_output"},
            {"type":"function_call_output","output":{"unexpected":"object"}},
            {"type":"function_call_output","call_id":42,"output":"invalid ID"},
            {"type":"function_call_output","call_id":" real ","output":"exact ID"},
            {"type":"function_call_output","output":[{"type":"encrypted_content","encrypted_content":"opaque"}]},
            {"type":"custom_tool_call_output","output":"custom result"},
            {"type":"message","role":"user","content":"unchanged"}
        ]});
        let original = body.clone();
        normalize_external_results(&mut body);
        assert_eq!(body, original);
    }
}
