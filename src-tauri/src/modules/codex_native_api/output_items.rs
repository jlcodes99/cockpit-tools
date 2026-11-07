//! Restore terminal output only from complete items observed in this response.
use std::collections::BTreeMap;

use serde_json::Value;

use super::contract::ServiceError;

pub(super) struct OutputCollector {
    indexed: BTreeMap<u64, Value>,
    unindexed: Vec<Value>,
    identity_invalid: bool,
}

impl OutputCollector {
    pub(super) fn new() -> Self {
        Self {
            indexed: BTreeMap::new(),
            unindexed: Vec::new(),
            identity_invalid: false,
        }
    }

    /// Only complete, contiguous items from this upstream attempt may fill an
    /// omitted successful terminal output for identity persistence. Other public
    /// repairs remain presentation-only and must retain the original evidence.
    pub(super) fn can_restore_identity(&self, event: &Value) -> bool {
        if self.identity_invalid
            || self.indexed.is_empty()
            || !self.unindexed.is_empty()
            || !matches!(
                event["type"].as_str(),
                Some("response.completed" | "response.done")
            )
            || event.get("error").is_some_and(|error| !error.is_null())
        {
            return false;
        }
        let Some(response) = event.get("response").and_then(Value::as_object) else {
            return false;
        };
        response.get("status").and_then(Value::as_str) == Some("completed")
            && response
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty())
            && response.get("error").is_none_or(Value::is_null)
            && response
                .get("output")
                .is_none_or(|output| output.as_array().is_some_and(Vec::is_empty))
            && self
                .indexed
                .keys()
                .enumerate()
                .all(|(expected, index)| *index == expected as u64)
    }

    pub(super) fn observe(&mut self, event: &mut Value) -> Result<bool, ServiceError> {
        if matches!(
            super::events::response_terminal_kind(event),
            Some("error" | "response.failed" | "response.incomplete" | "response.cancelled")
        ) || event.get("error").is_some_and(|error| !error.is_null())
            || event
                .pointer("/response/error")
                .is_some_and(|error| !error.is_null())
        {
            self.identity_invalid = true;
        }
        match event.get("type").and_then(Value::as_str) {
            Some("response.output_item.done") => {
                let Some(item) = event.get("item").filter(|item| {
                    item.is_object()
                        && item
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|v| !v.is_empty())
                }) else {
                    self.identity_invalid = true;
                    return Ok(false);
                };
                let index = event.get("output_index").and_then(Value::as_u64);
                self.identity_invalid |= index.is_none()
                    || !complete_identity_item(item)
                    || index
                        .and_then(|index| self.indexed.get(&index))
                        .is_some_and(|previous| previous != item);
                if let Some(index) = index {
                    self.indexed.insert(index, item.clone());
                } else {
                    self.unindexed.push(item.clone());
                }
                Ok(false)
            }
            Some("response.completed" | "response.done" | "response.incomplete") => {
                let reconcile_calls = event["type"] != "response.incomplete";
                let Some(response) = event.get_mut("response").and_then(Value::as_object_mut)
                else {
                    return Ok(false);
                };
                let collected = || {
                    self.indexed
                        .values()
                        .chain(self.unindexed.iter())
                };
                let changed = match response.get_mut("output") {
                    Some(Value::Array(output)) if !output.is_empty() => {
                        let calls: BTreeMap<_, _> = collected()
                            .filter(|item| complete_tool_call(item))
                            .map(|item| (item["call_id"].as_str().unwrap().trim(), item))
                            .collect();
                        let mut changed = false;
                        for (index, item) in output.iter_mut().enumerate() {
                            let missing_id = item.get("id").is_none_or(|id| {
                                id.is_null() || id.as_str().is_some_and(|id| id.trim().is_empty())
                            });
                            if missing_id {
                                if let Some(id) = self
                                    .indexed
                                    .get(&(index as u64))
                                    .and_then(|item| item.get("id"))
                                    .and_then(Value::as_str)
                                    .filter(|id| !id.trim().is_empty())
                                {
                                    if let Some(object) = item.as_object_mut() {
                                        object.insert("id".into(), Value::String(id.into()));
                                        changed = true;
                                    }
                                }
                            }
                            if reconcile_calls && is_tool_call(item) {
                                if let Some(call) =
                                    item["call_id"].as_str().and_then(|id| calls.get(id.trim()))
                                {
                                    if &*item != *call {
                                        *item = (*call).clone();
                                        changed = true;
                                    }
                                }
                            }
                        }
                        changed
                    }
                    _ if !self.indexed.is_empty() || !self.unindexed.is_empty() => {
                        let output = collected()
                            .filter(|item| {
                                !reconcile_calls || !is_tool_call(item) || complete_tool_call(item)
                            })
                            .cloned()
                            .collect();
                        let restored = Value::Array(output);
                        let changed = response.get("output") != Some(&restored);
                        response.insert("output".into(), restored);
                        changed
                    }
                    _ => false,
                };
                Ok(changed)
            }
            _ => Ok(false),
        }
    }
}

fn is_tool_call(item: &Value) -> bool {
    matches!(
        item.get("type").and_then(Value::as_str),
        Some("function_call" | "custom_tool_call")
    )
}

fn complete_identity_item(item: &Value) -> bool {
    if !item
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
        || item
            .get("status")
            .is_some_and(|status| status.as_str() != Some("completed"))
    {
        return false;
    }
    match item["type"].as_str() {
        Some("message") => {
            matches!(
                item["role"].as_str(),
                Some("assistant" | "user" | "system" | "developer")
            ) && complete_content(&item["content"], false)
        }
        Some("agent_message") => {
            item["author"].is_string()
                && item["recipient"].is_string()
                && complete_content(&item["content"], true)
        }
        Some("function_call") => {
            complete_tool_call(item)
                && item["arguments"]
                    .as_str()
                    .is_some_and(|arguments| serde_json::from_str::<Value>(arguments).is_ok())
        }
        Some("custom_tool_call") => complete_tool_call(item),
        Some("reasoning") => {
            item["encrypted_content"]
                .as_str()
                .is_some_and(|cipher| !cipher.is_empty())
                || item["summary"].is_array()
                || item["content"].is_array()
        }
        Some("compaction" | "compaction_summary" | "context_compaction") => item
            ["encrypted_content"]
            .as_str()
            .is_some_and(|cipher| !cipher.is_empty()),
        Some("function_call_output" | "custom_tool_call_output") => {
            complete_content(&item["output"], false)
        }
        Some("additional_tools" | "tool_search_output") => item["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().all(Value::is_object)),
        Some("local_shell_call" | "web_search_call") => item["action"].is_object(),
        Some("image_generation_call") => item["result"].is_string(),
        Some("tool_search_call") => item.get("arguments").is_some(),
        Some("configuration_update") => item["reasoning"].is_object(),
        _ => false,
    }
}

fn complete_content(content: &Value, agent: bool) -> bool {
    if !agent && content.is_string() {
        return true;
    }
    content.as_array().is_some_and(|parts| {
        !parts.is_empty()
            && parts.iter().all(|part| match part["type"].as_str() {
                Some("input_text" | "output_text" | "text") => part["text"].is_string(),
                Some("encrypted_content") if agent => part["encrypted_content"]
                    .as_str()
                    .is_some_and(|cipher| !cipher.is_empty()),
                Some("refusal") if !agent => part["refusal"].is_string(),
                _ => false,
            })
    })
}

pub(super) fn complete_tool_call(item: &Value) -> bool {
    if !["call_id", "name"].into_iter().all(|field| {
        item.get(field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return false;
    }
    let field = match item.get("type").and_then(Value::as_str) {
        Some("function_call") => "arguments",
        Some("custom_tool_call") => "input",
        _ => return false,
    };
    item.get(field).is_some_and(Value::is_string)
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completion_restores_sorted_output_without_incomplete_tools_or_losing_ciphertext() {
        for kind in ["response.completed", "response.done"] {
            let mut collector = OutputCollector::new();
            let reasoning = json!({"type":"reasoning","encrypted_content":"cipher","summary":[]});
            let custom = json!({"type":"custom_tool_call","call_id":"c","name":"exec","input":"pwd","extension":true});
            for mut event in [
                json!({"type":"response.output_item.done","output_index":2,"item":custom}),
                json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
                json!({"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"incomplete","name":"exec"}}),
                json!({"type":"response.output_item.done","item":{"type":"message","content":[]}}),
            ] {
                assert!(!collector.observe(&mut event).unwrap());
            }
            let mut terminal = json!({"type":kind,"extension":true,"response":{"id":"r","status":"completed","output":[],"usage":{"input_tokens":3,"output_tokens":5}}});
            assert!(collector.observe(&mut terminal).unwrap());
            assert_eq!(
                terminal["response"]["output"],
                json!([reasoning,custom,{"type":"message","content":[]}])
            );
            assert_eq!(terminal["response"]["usage"]["output_tokens"], 5);
            assert_eq!(terminal["extension"], true);
        }
    }

    #[test]
    fn completion_reconciles_tool_by_call_id_without_replacing_unrelated_output() {
        let mut collector = OutputCollector::new();
        let custom = json!({"type":"custom_tool_call","call_id":"c","name":"exec","input":"pwd","encrypted_function_args":["input"]});
        collector
            .observe(&mut json!({"type":"response.output_item.done","item":custom}))
            .unwrap();
        let message = json!({"type":"message","id":"m","content":[]});
        let other =
            json!({"type":"function_call","call_id":"other","name":"exec","arguments":"{}"});
        let mut terminal = json!({"type":"response.completed","response":{"status":"completed","id":"r","output":[message,{"type":"function_call","call_id":"c","name":"exec"},other]}});
        assert!(collector.observe(&mut terminal).unwrap());
        assert_eq!(
            terminal["response"]["output"],
            json!([message, custom, other])
        );
    }

    #[test]
    fn failed_output_and_unknown_fields_are_preserved_without_inventing_success() {
        let mut collector = OutputCollector::new();
        collector.observe(&mut json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"c","name":"tool"}})).unwrap();
        for mut event in [
            json!({"type":"response.failed","response":{"status":"failed","output":[]},"extension":true}),
            json!({"type":"response.cancelled","response":{"status":"cancelled","output":[]}}),
            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"custom_tool_call","call_id":"c","name":"exec","input":"real"}]}}),
        ] {
            let original = event.clone();
            assert!(!collector.observe(&mut event).unwrap());
            assert_eq!(event, original);
        }
    }

    #[test]
    fn collector_keeps_large_aggregate_output_and_replaces_indexes_like_go() {
        let mut collector = OutputCollector::new();
        let mut event = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","encrypted_content":"x".repeat(5 * 1024 * 1024)}});
        collector.observe(&mut event).unwrap();
        collector.observe(&mut event).unwrap();
        event["output_index"] = json!(1);
        collector.observe(&mut event).unwrap();
        let mut terminal = json!({"type":"response.completed","response":{"status":"completed","output":[]}});
        assert!(collector.observe(&mut terminal).unwrap());
        assert_eq!(terminal["response"]["output"].as_array().unwrap().len(), 2);
        assert_eq!(terminal["response"]["output"][0]["encrypted_content"].as_str().unwrap().len(), 5 * 1024 * 1024);
    }
    #[test]
    fn output_ids_and_incomplete_output_are_restored_without_changing_status() {
        for kind in ["response.completed", "response.incomplete"] {
            let mut collector = OutputCollector::new();
            let message = json!({"type":"message","id":"observed-message","content":[]});
            collector.observe(&mut json!({"type":"response.output_item.done","output_index":0,"item":message})).unwrap();
            let status = if kind == "response.incomplete" {
                "incomplete"
            } else {
                "completed"
            };
            for id in [Value::Null, json!(" ")] {
                let mut event = json!({"type":kind,"response":{"id":"r","status":status,"output":[{"type":"message","id":id,"content":[],"extension":true}]}});
                assert!(collector.observe(&mut event).unwrap());
                assert_eq!(event["response"]["output"][0]["id"], "observed-message");
                assert_eq!(event["response"]["output"][0]["extension"], true);
                assert_eq!(event["response"]["status"], status);
            }
            let mut original = json!({"type":kind,"response":{"id":"r","status":status,"output":[{"type":"message","id":"final-id"}]}});
            assert!(!collector.observe(&mut original).unwrap());
            assert_eq!(original["response"]["output"][0]["id"], "final-id");
            let mut empty = json!({"type":kind,"response":{"id":"r","status":status,"output":[]}});
            assert!(collector.observe(&mut empty).unwrap());
            assert_eq!(empty["response"]["output"], json!([message]));
            assert_eq!(empty["type"], kind);
        }
    }
    #[test]
    fn incomplete_preserves_its_nonempty_terminal_tool_fields() {
        let mut collector = OutputCollector::new();
        collector.observe(&mut json!({"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","call_id":"c","name":"tool","arguments":"old"}})).unwrap();
        let mut terminal = json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","output":[{"type":"function_call","call_id":"c","name":"tool","arguments":"new","extension":"terminal"}],"incomplete_details":{"reason":"max_output_tokens"}}});
        let original = terminal.clone();
        assert!(!collector.observe(&mut terminal).unwrap());
        assert_eq!(terminal, original);
    }

    #[test]
    fn null_and_nonarray_terminal_output_are_restored_from_observed_items() {
        for kind in ["response.completed", "response.done", "response.incomplete"] {
            for output in [Value::Null, json!(false), json!("invalid"), json!({})] {
                let mut collector = OutputCollector::new();
                let reasoning =
                    json!({"type":"reasoning","id":"reasoning","encrypted_content":"retained"});
                collector.observe(&mut json!({"type":"response.output_item.done","output_index":0,"item":reasoning})).unwrap();
                let status = if kind == "response.incomplete" {
                    "incomplete"
                } else {
                    "completed"
                };
                let mut terminal =
                    json!({"type":kind,"response":{"id":"r","status":status,"output":output}});
                assert!(collector.observe(&mut terminal).unwrap());
                assert_eq!(terminal["response"]["output"], json!([reasoning]));
                assert_eq!(terminal["response"]["status"], status);
            }
        }
    }
    #[test]
    fn tool_reconciliation_runs_after_id_hydration_without_borrowing_another_items_id() {
        let mut collector = OutputCollector::new();
        collector.observe(&mut json!({"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"message-id","content":[]}})).unwrap();
        let call = json!({"type":"function_call","call_id":"c","name":"tool","arguments":"{}"});
        collector
            .observe(&mut json!({"type":"response.output_item.done","output_index":1,"item":call}))
            .unwrap();
        let mut terminal = json!({"type":"response.completed","response":{"id":"r","status":"completed","output":[call]}});
        collector.observe(&mut terminal).unwrap();
        assert_eq!(terminal["response"]["output"], json!([call]));
        assert!(terminal["response"]["output"][0].get("id").is_none());
    }

    #[test]
    fn incomplete_fallback_keeps_observed_partial_tool_fields_for_the_client() {
        let mut collector = OutputCollector::new();
        let partial = json!({"type":"function_call","call_id":"partial","name":"tool","extension":"partial-output"});
        collector
            .observe(
                &mut json!({"type":"response.output_item.done","output_index":0,"item":partial}),
            )
            .unwrap();
        let mut terminal = json!({"type":"response.incomplete","response":{"id":"r","status":"incomplete","output":[]}});
        assert!(collector.observe(&mut terminal).unwrap());
        assert_eq!(terminal["response"]["output"], json!([partial]));
        assert_eq!(terminal["response"]["status"], "incomplete");
    }

    fn identity_message() -> Value {
        json!({"id":"message","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"synthetic answer"}],"unknown":{"preserve":true}})
    }

    #[test]
    fn identity_restoration_uses_only_complete_contiguous_original_done_items() {
        for kind in ["response.completed", "response.done"] {
            for missing_output in [false, true] {
                let mut collector = OutputCollector::new();
                let output = vec![
                    identity_message(),
                    json!({"id":"reasoning","type":"reasoning","summary":[],"encrypted_content":"synthetic-cipher","signature":"synthetic-signature"}),
                    json!({"id":"tool","type":"function_call","call_id":"call","name":"send_message","namespace":"collaboration","arguments":"{\"message\":\"synthetic task\"}","encrypted_function_args":[],"extra":[1,true]}),
                ];
                for index in [2, 0, 1, 0] {
                    let mut done = json!({"type":"response.output_item.done","output_index":index,"item":output[index]});
                    let original = done.clone();
                    assert!(!collector.observe(&mut done).unwrap());
                    assert_eq!(done, original);
                }
                let mut terminal = json!({"type":kind,"extra":{"keep":true},"response":{"id":"response","status":"completed","error":null,"output":[]}});
                if missing_output {
                    terminal["response"]
                        .as_object_mut()
                        .unwrap()
                        .remove("output");
                }
                assert!(collector.can_restore_identity(&terminal));
                assert!(collector.observe(&mut terminal).unwrap());
                assert_eq!(terminal["response"]["output"], json!(output));
                assert_eq!(terminal["extra"], json!({"keep":true}));
            }
        }
    }

    #[test]
    fn identity_restoration_rejects_bad_indexes_conflicts_and_incomplete_items() {
        let good =
            json!({"type":"response.output_item.done","output_index":0,"item":identity_message()});
        let cases = [
            vec![json!({"type":"response.output_item.done","item":identity_message()})],
            vec![
                json!({"type":"response.output_item.done","output_index":-1,"item":identity_message()}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":4096,"item":identity_message()}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":"0","item":identity_message()}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":0.5,"item":identity_message()}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":1,"item":identity_message()}),
            ],
            vec![
                good.clone(),
                json!({"type":"response.output_item.done","output_index":2,"item":identity_message()}),
            ],
            vec![
                good.clone(),
                json!({"type":"response.output_item.done","output_index":0,"item":{"id":"changed","type":"message","role":"assistant","content":"different"}}),
                good.clone(),
            ],
            vec![json!({"type":"response.output_item.done","output_index":0,"item":null})],
            vec![
                json!({"type":"response.output_item.done","output_index":0,"item":{"id":"partial","type":"message","role":"assistant"}}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":0,"item":{"id":"partial","type":"function_call","call_id":"call","name":"tool","arguments":"{"}}),
            ],
            vec![
                json!({"type":"response.output_item.done","output_index":0,"item":{"id":"partial","type":"custom_tool_call","call_id":"call","name":"tool"}}),
            ],
            vec![
                good.clone(),
                json!({"type":"error","error":{"code":"synthetic_failure"}}),
            ],
        ];
        for (index, events) in cases.into_iter().enumerate() {
            let mut collector = OutputCollector::new();
            for mut event in events {
                collector.observe(&mut event).unwrap();
            }
            let terminal = json!({"type":"response.completed","response":{"id":"response","status":"completed","output":[]}});
            assert!(!collector.can_restore_identity(&terminal), "case {index}");
        }
    }

    #[test]
    fn identity_restoration_never_authenticates_failed_or_nonempty_terminals() {
        let mut collector = OutputCollector::new();
        collector.observe(&mut json!({"type":"response.output_item.done","output_index":0,"item":identity_message()})).unwrap();
        for event in [
            json!({"type":"response.failed","response":{"id":"response","status":"failed","output":[]}}),
            json!({"type":"response.incomplete","response":{"id":"response","status":"incomplete","output":[]}}),
            json!({"type":"response.cancelled","response":{"id":"response","status":"cancelled","output":[]}}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
            json!({"type":"response.completed","response":{"id":" ","status":"completed","output":[]}}),
            json!({"type":"response.completed","response":{"id":"response","status":"completed","error":{},"output":[]}}),
            json!({"type":"response.completed","error":{"code":"synthetic_failure"},"response":{"id":"response","status":"completed","output":[]}}),
            json!({"type":"response.completed","response":{"id":"response","status":"completed","output":null}}),
            json!({"type":"response.completed","response":{"id":"response","status":"completed","output":[{"id":"other","type":"message","role":"assistant","content":"contradictory"}]}}),
        ] {
            assert!(!collector.can_restore_identity(&event), "{event}");
        }
    }
}
