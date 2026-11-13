use super::*;

fn call(id: &str) -> Value {
    json!({"type":"custom_tool_call","call_id":id,"id":format!("call-{id}"),
        "name":"exec","input":"original custom input\n  ","status":"completed"})
}

fn result(id: &str, item: &str, output: Value) -> Value {
    json!({"type":"custom_tool_call_output","call_id":id,"id":item,"output":output})
}

fn source() -> Vec<Value> {
    let mut notification = result("c", "notice", json!("\n  second \"exact\"  "));
    notification["name"] = json!("exec");
    vec![
        call("c"),
        result(
            "c",
            "first",
            json!([
                {"type":"input_text","text":"first"},
                {"type":"input_image","image_url":"data:image/png;base64,fixture","detail":"original"}
            ]),
        ),
        notification.clone(),
        {
            let mut last = notification;
            last["id"] = json!("last");
            last
        },
    ]
}

#[test]
fn custom_results_copy_keeps_content_order_whitespace_images_and_duplicates() {
    let original = source();
    let saved = original.clone();
    let merged = replay(&original).unwrap();
    assert!(matches!(merged, Cow::Owned(_)));
    assert!(paired_tools(&merged));
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0], original[0]);
    assert_eq!(merged[1]["id"], "first");
    assert!(merged[1].get("name").is_none());
    assert_eq!(
        merged[1]["output"],
        json!([
            {"type":"input_text","text":"first"},
            {"type":"input_image","image_url":"data:image/png;base64,fixture","detail":"original"},
            {"type":"input_text","text":"\n  second \"exact\"  "},
            {"type":"input_text","text":"\n  second \"exact\"  "}
        ])
    );
    assert_eq!(original, saved, "source envelopes must stay exact");
    assert!(
        matches!(replay(&merged).unwrap(), Cow::Borrowed(_)),
        "idempotent replay"
    );
}

#[test]
fn custom_results_multiple_groups_and_empty_strings_keep_every_part() {
    let mut input = source();
    input.extend([
        call("d"),
        result("d", "d1", json!("")),
        result("d", "d2", json!([{ "type":"input_text", "text":" " }])),
        result("d", "d3", json!("")),
    ]);
    let copy = replay(&input).unwrap();
    assert_eq!(copy.len(), 4);
    assert_eq!(
        copy[3]["output"],
        json!([
            {"type":"input_text","text":""}, {"type":"input_text","text":" "},
            {"type":"input_text","text":""}
        ])
    );
}

#[test]
fn custom_results_refuses_orphans_conflicts_wrong_kind_and_noncontiguous_groups() {
    let original = source();
    let mut cases = Vec::new();
    cases.push(original[1..].to_vec());
    let mut after = original.clone();
    after.rotate_left(1);
    cases.push(after);
    let mut duplicate_call = original.clone();
    duplicate_call.insert(1, original[0].clone());
    cases.push(duplicate_call);
    let mut wrong_kind = original.clone();
    wrong_kind[0] = json!({"type":"function_call","call_id":"c","name":"exec","arguments":"{}"});
    cases.push(wrong_kind);
    let mut interrupted = original.clone();
    interrupted.insert(
        2,
        json!({"role":"assistant","content":"intervening output"}),
    );
    cases.push(interrupted);
    let mut wrong_name = original.clone();
    wrong_name[2]["name"] = json!("other");
    cases.push(wrong_name);
    let mut conflicting_id = original.clone();
    conflicting_id[2]["id"] = original[1]["id"].clone();
    cases.push(conflicting_id);
    for input in cases {
        let saved = input.clone();
        assert!(
            replay(&input).is_none(),
            "unsafe source was admitted: {input:?}"
        );
        assert_eq!(input, saved);
    }
}

#[test]
fn custom_results_unknown_signed_encrypted_and_malformed_envelopes_stay_rejected() {
    for index in [0, 1, 2, 3] {
        for (key, value) in [
            ("extension", json!(7)),
            ("signature", json!("signed")),
            ("encrypted_content", json!("cipher")),
            ("encrypted_function_args", json!([])),
            ("thought_signature", json!("signed")),
            ("status", json!("in_progress")),
        ] {
            let mut input = source();
            input[index][key] = value;
            assert!(replay(&input).is_none(), "accepted {index}:{key}");
        }
        let mut input = source();
        input[index]["id"] = Value::Null;
        assert!(replay(&input).is_none());
    }
    let mut input = source();
    input[1]["output"][1]["file_id"] = json!("server-only-image");
    assert!(
        replay(&input).is_none(),
        "original replay safety runs first"
    );
    let mut input = source();
    input[0]["input"] = json!({"not":"custom input text"});
    assert!(replay(&input).is_none());
}

#[test]
fn custom_results_does_not_relax_function_results_or_change_strict_pairs() {
    let input = vec![
        json!({"type":"function_call","call_id":"f","name":"lookup","arguments":"{}"}),
        json!({"type":"function_call_output","call_id":"f","output":"first"}),
        json!({"type":"function_call_output","call_id":"f","output":"second"}),
    ];
    assert!(replay(&input).is_none());
    let mut valid = source();
    valid.truncate(2);
    valid[0]["extension"] = json!("existing valid history is not rewritten");
    assert!(paired_tools(&valid));
    assert!(matches!(replay(&valid).unwrap(), Cow::Borrowed(_)));
}

// Synthetic values reproduce only the verified envelope/part structure: three
// completed exec calls, seven initial parts, then two distinct notifications.
fn three_notification_groups() -> (Vec<Value>, Vec<Value>) {
    let mut source = Vec::new();
    let mut expected = Vec::new();
    for id in ["group-a", "group-b", "group-c"] {
        let parts = json!([
            {"type":"input_text","text":format!("{id}: first\n  ")},
            {"type":"input_image","image_url":format!("data:image/png;base64,{id}-one")},
            {"type":"input_text","text":format!("{id}: second")},
            {"type":"input_image","image_url":format!("data:image/png;base64,{id}-two")},
            {"type":"input_text","text":format!("{id}: third")},
            {"type":"input_image","image_url":format!("data:image/png;base64,{id}-three")},
            {"type":"input_text","text":format!("{id}: final initial part")}
        ]);
        let first_notice = format!("notify-{id}-one\n  ");
        let second_notice = format!("\t notify-{id}-two \"quoted\"\n");
        let initial = result(id, &format!("{id}-initial"), parts.clone());
        let mut first = result(id, &format!("{id}-notice-one"), json!(first_notice));
        first["name"] = json!("exec");
        let mut second = result(id, &format!("{id}-notice-two"), json!(second_notice));
        second["name"] = json!("exec");
        let between = json!({"role":"assistant","content":format!("After {id}")});
        source.extend([call(id), initial, first, second, between.clone()]);

        let mut expected_parts = parts.as_array().unwrap().clone();
        expected_parts.push(json!({"type":"input_text","text":first_notice}));
        expected_parts.push(json!({"type":"input_text","text":second_notice}));
        expected.extend([
            call(id),
            result(id, &format!("{id}-initial"), Value::Array(expected_parts)),
            between,
        ]);
    }
    (source, expected)
}

#[test]
fn custom_results_three_observed_envelope_groups_keep_distinct_notifications() {
    let (source, expected) = three_notification_groups();
    let saved = source.clone();
    assert!(replay_items(&source));
    assert!(!paired_tools(&source));
    let merged = replay(&source).unwrap();
    assert_eq!(merged.as_ref(), expected.as_slice());
    assert!(paired_tools(&merged));
    for result_index in [1, 4, 7] {
        let parts = merged[result_index]["output"].as_array().unwrap();
        assert_eq!(parts.len(), 9);
        assert_ne!(parts[7]["text"], parts[8]["text"]);
    }
    assert_eq!(source, saved);
}

#[test]
fn custom_results_three_groups_require_tail_pending_call_result_before_replay() {
    let (mut source, mut expected) = three_notification_groups();
    source.push(call("pending-tail"));
    expected.push(call("pending-tail"));
    let waiting = source.clone();
    assert!(replay_items(&source));
    assert!(
        replay(&source).is_none(),
        "completed history cannot hide a pending call"
    );
    assert_eq!(source, waiting);

    let returned = result(
        "pending-tail",
        "pending-tail-result",
        json!("\n  actual tail result  "),
    );
    source.push(returned.clone());
    expected.push(returned);
    let completed = source.clone();
    let merged = replay(&source).unwrap();
    assert_eq!(merged.as_ref(), expected.as_slice());
    assert!(paired_tools(&merged));
    assert_eq!(source, completed);
    assert!(waiting.starts_with(&source[..source.len() - 1]));
}
