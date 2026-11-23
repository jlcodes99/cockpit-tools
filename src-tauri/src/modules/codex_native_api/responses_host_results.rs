//! Codex clients can inject these host results without a model-issued call.
//! Recognize the envelope exactly; replay validation still checks its payload.
use serde_json::Value;

pub(super) fn is_host_result(item: &Value) -> bool {
    item["type"] == "function_call_output"
        && match (item["namespace"].as_str(), item["name"].as_str()) {
            (Some("codex_app" | "codex_tui"), Some("create_thread" | "send_message_to_thread")) => {
                true
            }
            (Some("agent"), Some(name)) => {
                // Goal objective snapshots are client host results without a model call.
                matches!(
                    name,
                    "browser_tabs"
                        | "codex_apps_open_page"
                        | "codex_apps_writing_block_edits"
                        | "goal_objective"
                ) || selected_page_part(name)
            }
            _ => false,
        }
}

fn selected_page_part(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("codex_apps_selected_page_part_") else {
        return false;
    };
    let Some((part, total)) = suffix.split_once("_of_") else {
        return false;
    };
    // Accept only canonical positive ASCII indices, with checked numeric bounds.
    // Other agent operations and malformed page-part names remain ordinary results.
    let number = |raw: &str| -> Option<u32> {
        if raw.is_empty() || raw.starts_with('0') || !raw.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        raw.parse().ok()
    };
    matches!((number(part), number(total)),
        (Some(part), Some(total)) if part <= total)
}

pub(super) fn is_standalone(item: &Value) -> bool {
    is_host_result(item)
        && match item.get("call_id") {
            None => true,
            Some(Value::String(id)) => id.is_empty(),
            // Explicit malformed or stale IDs must follow normal pairing rules.
            _ => false,
        }
}

#[cfg(test)]
mod agent_browser_tests {
    use super::*;
    use serde_json::json;

    fn browser_result(name: &str) -> Value {
        json!({"type":"function_call_output","namespace":"agent","name":name,
            "output":"Synthetic browser observation\n  exact whitespace  "})
    }

    #[test]
    fn agent_browser_host_envelopes_and_page_part_boundaries_are_exact() {
        for name in [
            "browser_tabs",
            "goal_objective",
            "codex_apps_open_page",
            "codex_apps_writing_block_edits",
            "codex_apps_selected_page_part_1_of_1",
            "codex_apps_selected_page_part_1_of_2",
            "codex_apps_selected_page_part_2_of_2",
            "codex_apps_selected_page_part_4294967295_of_4294967295",
        ] {
            let mut item = browser_result(name);
            assert!(is_host_result(&item), "{name}");
            assert!(is_standalone(&item), "{name}");
            item["call_id"] = json!("");
            assert!(is_standalone(&item), "{name}: explicit empty ID");
        }
        for name in [
            "browser_tabs_extra",
            "codex_apps_selected_page_part_0_of_1",
            "codex_apps_selected_page_part_1_of_0",
            "codex_apps_selected_page_part_2_of_1",
            "codex_apps_selected_page_part_01_of_1",
            "codex_apps_selected_page_part_1_of_01",
            "codex_apps_selected_page_part_+1_of_1",
            "codex_apps_selected_page_part_-1_of_1",
            "codex_apps_selected_page_part_1_of_+1",
            "codex_apps_selected_page_part_1_of_-1",
            "codex_apps_selected_page_part_1_of_1 ",
            "codex_apps_selected_page_part_ 1_of_1",
            "codex_apps_selected_page_part_１_of_１",
            "codex_apps_selected_page_part__of_1",
            "codex_apps_selected_page_part_1_of_",
            "codex_apps_selected_page_part_1_of_1_of_1",
            "codex_apps_selected_page_part_1_of_1_extra",
            "codex_apps_selected_page_part_4294967296_of_4294967296",
            "codex_apps_open_page_extra",
            "codex_apps_writing_block_edits_extra",
            "codex_apps_selected_page",
            "create_thread",
            "send_message_to_thread",
            "unknown_browser_operation",
        ] {
            assert!(!is_host_result(&browser_result(name)), "{name}");
            assert!(!is_standalone(&browser_result(name)), "{name}");
        }
        for namespace in [
            "other",
            "Agent",
            " agent",
            "agent ",
            "codex_app",
            "codex_tui",
        ] {
            for name in ["codex_apps_open_page", "codex_apps_writing_block_edits"] {
                let mut item = browser_result(name);
                item["namespace"] = json!(namespace);
                assert!(
                    !is_host_result(&item),
                    "name={name:?} namespace={namespace:?}"
                );
            }
        }
        for name in ["codex_apps_open_page", "codex_apps_writing_block_edits"] {
            let mut wrong_kind = browser_result(name);
            wrong_kind["type"] = json!("custom_tool_call_output");
            assert!(!is_host_result(&wrong_kind), "{name}");
        }
    }

    #[test]
    fn agent_browser_host_results_keep_real_tool_pairing_and_payload_validation() {
        use crate::modules::codex_native_api::identity::rebuild::{paired_tools, replay_items};
        for name in [
            "browser_tabs",
            "goal_objective",
            "codex_apps_open_page",
            "codex_apps_writing_block_edits",
            "codex_apps_selected_page_part_1_of_1",
        ] {
            let base = browser_result(name);
            for id in [json!("stale"), json!(" "), Value::Null, json!(7)] {
                let mut item = base.clone();
                item["call_id"] = id;
                assert!(!is_standalone(&item));
                assert!(!paired_tools(&[item]));
            }
            let call = json!({"type":"function_call","namespace":"agent","name":name,
                "call_id":"real-call","arguments":"{}"});
            let mut result = base.clone();
            result["call_id"] = json!("real-call");
            assert!(paired_tools(&[call.clone(), result.clone()]));
            assert!(!paired_tools(&[
                call.clone(),
                result.clone(),
                result.clone()
            ]));
            assert!(!paired_tools(&[result, call.clone()]));
            assert!(!paired_tools(&[call, base.clone()]));
            for output in [
                Value::Null,
                json!({"unexpected":"object"}),
                json!([{"type":"input_file","file_id":"server-resource"}]),
            ] {
                let mut item = base.clone();
                item["output"] = output;
                assert!(!replay_items(&[item]));
            }
        }
        assert!(!paired_tools(&[
            json!({"type":"function_call_output","output":"anonymous"})
        ]));
        assert!(!paired_tools(&[
            json!({"type":"function_call_output","namespace":"agent",
            "name":"unknown","output":"unrecognized host result"})
        ]));
    }

    #[test]
    fn writing_block_host_result_replays_losslessly_with_real_tool_calls() {
        use crate::modules::codex_native_api::responses_custom_results;
        use std::borrow::Cow;

        let history = vec![
            json!({"type":"function_call", "name":"lookup", "call_id":"call-1",
                "arguments":"{}"}),
            json!({"type":"function_call_output", "namespace":"agent",
                "name":"codex_apps_writing_block_edits",
                "output":"{\"edits\":[{\"text\":\"Alpha β\"}]}\n  exact whitespace  "}),
            json!({"type":"function_call_output", "call_id":"call-1", "output":"done"}),
        ];
        let replay = responses_custom_results::replay(&history).expect("complete history replays");
        assert!(matches!(replay, Cow::Borrowed(_)));
        assert_eq!(replay.as_ref(), history.as_slice());
        assert!(responses_custom_results::replay(&history[..2]).is_none());

        let mut unknown_namespace = history.clone();
        unknown_namespace[1]["namespace"] = json!("other");
        assert!(responses_custom_results::replay(&unknown_namespace).is_none());
    }
}

#[cfg(test)]
mod goal_objective_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_goal_objective_recognition_requires_the_exact_host_envelope() {
        let goal = json!({"type":"function_call_output","namespace":"agent",
            "name":"goal_objective","output":"Goal snapshot\n  preserve whitespace  "});
        assert!(is_standalone(&goal));
        for name in [
            "goal_objective_extra",
            "Goal_objective",
            "goal_objective ",
            " goal_objective",
        ] {
            let mut item = goal.clone();
            item["name"] = json!(name);
            assert!(!is_host_result(&item));
        }
        for namespace in [
            "other",
            "Agent",
            " agent",
            "agent ",
            "codex_app",
            "codex_tui",
        ] {
            let mut item = goal.clone();
            item["namespace"] = json!(namespace);
            assert!(!is_host_result(&item));
        }
        for kind in ["function_call", "custom_tool_call_output", "message"] {
            let mut item = goal.clone();
            item["type"] = json!(kind);
            assert!(!is_host_result(&item));
        }
    }
}
