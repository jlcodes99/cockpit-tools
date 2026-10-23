//! Host-injected results are history, not anonymous results of another tool call.
use super::*;

fn host_result(name: &str) -> Value {
    json!({"type":"function_call_output","namespace":"codex_app","name":name,"output":"Original host result\nwith exact whitespace.  "})
}

#[test]
fn host_results_survive_admission_wire_normalization_restart_and_account_switch() {
    for name in ["create_thread", "send_message_to_thread"] {
        for empty_id in [false, true] {
            for output in [
                json!("Original host result\nwith exact whitespace.  "),
                json!([{"type":"input_text","text":"Host payload:"},{"type":"encrypted_content","encrypted_content":"host-result-cipher"}]),
            ] {
                let (_temp, store) = fixture();
                let mut original = history();
                let mut result = host_result(name);
                result["output"] = output;
                if empty_id {
                    result["call_id"] = json!("");
                }
                // The unrelated ordinary call is pending at this position.
                original["input"].as_array_mut().unwrap().insert(4, result);
                let saved = original.clone();
                let (binding, headers_a, sent) =
                    prepare(&store, "caller", &HeaderMap::new(), &original, "a");
                let mut wire = sent.clone();
                let mut headers = headers_a.clone();
                crate::modules::codex_native_api::codex_wire::normalize(
                    &mut headers,
                    &mut wire,
                    true,
                    false,
                )
                .unwrap();
                assert_eq!(
                    wire["input"], original["input"],
                    "host results must never consume a pending tool call"
                );
                complete(&store, &binding, "host-response-a");
                drop(store);

                let restarted = IdentityStore::open(_temp.path().join("identity.sqlite")).unwrap();
                let (binding_b, headers_b, sent_b) =
                    prepare(&restarted, "caller", &HeaderMap::new(), &original, "b");
                assert_ne!(headers_a["thread-id"], headers_b["thread-id"]);
                assert_eq!(sent_b["input"], saved["input"]);
                complete(&restarted, &binding_b, "host-response-b");
                let (_, next_headers, next) =
                    prepare(&restarted, "caller", &HeaderMap::new(), &original, "b");
                assert_eq!(headers_b["thread-id"], next_headers["thread-id"]);
                assert_eq!(next["input"], saved["input"]);
                assert_eq!(original, saved);
                assert_eq!(
                    restarted
                        .inspect_for_rebuild("foreign", &HeaderMap::new(), &original)
                        .unwrap_err()
                        .code,
                    "upstream_state_caller_mismatch"
                );
            }
        }
    }
}

#[test]
fn host_results_do_not_admit_unknown_malformed_or_conflicting_history() {
    let base = host_result("create_thread");
    let mut invalid = Vec::new();
    for (field, value) in [
        ("type", json!("custom_tool_call_output")),
        ("namespace", json!("other")),
        ("namespace", json!(" codex_app ")),
        ("name", json!("unknown")),
        ("name", json!("codex_app__create_thread")),
        ("call_id", json!("stale-call")),
        ("call_id", json!(" ")),
        ("call_id", json!(null)),
        ("call_id", json!(123)),
        ("output", json!(null)),
        ("output", json!({"result":"unknown envelope"})),
        (
            "output",
            json!([{"type":"encrypted_content","encrypted_content":123}]),
        ),
        (
            "output",
            json!([{"type":"input_file","file_id":"server-only"}]),
        ),
    ] {
        let mut item = base.clone();
        item[field] = value;
        invalid.push(vec![item]);
    }
    let mut missing_output = base.clone();
    missing_output.as_object_mut().unwrap().remove("output");
    invalid.push(vec![missing_output]);
    invalid.push(vec![
        json!({"type":"function_call_output","output":"ordinary anonymous result"}),
    ]);
    let call = json!({"type":"function_call","namespace":"codex_app","name":"create_thread","call_id":"real-call","arguments":"{}"});
    let mut paired = base;
    paired["call_id"] = json!("real-call");
    invalid.push(vec![call.clone(), paired.clone(), paired.clone()]);
    invalid.push(vec![paired.clone(), call.clone()]);
    invalid.push(vec![call.clone(), host_result("create_thread")]);
    assert!(super::super::rebuild::paired_tools(&[call, paired]));
    for items in invalid {
        let body = json!({"input":items});
        assert!(
            !super::super::automatic_history::complete_input(&HeaderMap::new(), &body),
            "invalid history was admitted: {body}"
        );
    }
}
