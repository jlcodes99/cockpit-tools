use super::*;
fn body() -> Value {
    json!({"input":[{"type":"message","role":"user","content":"original"},
 {"type":"reasoning","id":"rs_bad","encrypted_content":"bad-cipher","summary":[{"type":"summary_text","text":"visible summary"}]},
 {"type":"function_call","name":"f","call_id":"once","arguments":"{}"},
 {"type":"function_call_output","call_id":"once","output":"already executed"}]})
}
fn error() -> Value {
    json!({"type":"error","error":{"code":"invalid_encrypted_content","message":"The encrypted content for item rs_bad could not be verified. Reason: Encrypted content could not be decrypted or parsed."}})
}
#[test]
fn encrypted_reasoning_repair_preserves_messages_summary_and_tool_pairs() {
    let original = body();
    let repair = Repair::rejected(&original, &error()).unwrap();
    let mut wire = original.clone();
    assert!(repair.apply(&mut wire));
    assert_eq!(wire["input"][0], original["input"][0]);
    assert_eq!(wire["input"][2], original["input"][2]);
    assert_eq!(wire["input"][3], original["input"][3]);
    assert_eq!(wire["input"][1]["content"][0]["text"], "visible summary");
    assert!(wire["input"][1].get("encrypted_content").is_none());
    assert!(!repair.apply(&mut wire));
    assert_eq!(original, body());
}
#[test]
fn encrypted_reasoning_repair_requires_exact_unambiguous_reasoning_target() {
    for case in [
        "unknown",
        "code",
        "compaction",
        "previous",
        "duplicate",
        "missing_result",
        "unknown_summary",
        "conflicting_param",
    ] {
        let mut b = body();
        let mut e = error();
        match case {
            "unknown" => {
                e["error"]["message"] =
                    json!("The encrypted content for item other could not be verified.")
            }
            "code" => e["error"]["code"] = json!("invalid_request_error"),
            "compaction" => b["input"][1]["type"] = json!("compaction"),
            "previous" => b["previous_response_id"] = json!("upstream-owned"),
            "duplicate" => {
                let duplicate = b["input"][1].clone();
                b["input"].as_array_mut().unwrap().push(duplicate);
            }
            "missing_result" => {
                b["input"].as_array_mut().unwrap().pop();
            }
            "unknown_summary" => {
                b["input"][1]["summary"] = json!([{"type":"unknown","data":"must not drop"}])
            }
            "conflicting_param" => e["error"]["param"] = json!("input[0].encrypted_content"),
            _ => unreachable!(),
        }
        assert!(Repair::rejected(&b, &e).is_none(), "{case}");
    }
}
#[test]
fn encrypted_reasoning_repair_supports_index_and_empty_summary_without_dropping_other_items() {
    let mut b = body();
    b["input"][1]["summary"] = json!([]);
    let original = b.clone();
    let e =
        json!({"error":{"code":"invalid_encrypted_content","param":"input[1].encrypted_content"}});
    assert!(Repair::rejected(&b, &e).unwrap().apply(&mut b));
    assert_eq!(
        b["input"],
        json!([
            original["input"][0],
            original["input"][2],
            original["input"][3]
        ])
    );
}
