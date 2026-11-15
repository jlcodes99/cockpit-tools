fn full_replay_fixture() -> (Prepared, History, Value) {
    let mut prepared = prepared_history();
    prepared.source_body["input"] = json!([{"role":"user","content":[
        {"type":"input_text","text":"original context"},
        {"type":"input_image","image_url":"data:image/png;base64,c3ludGhldGlj"}
    ]}]);
    prepared.body = prepared.source_body.clone();
    let call = json!({"type":"function_call","id":"tool-item","call_id":"tool-call",
        "namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"synthetic\"}",
        "encrypted_function_args":[],"metadata":{"internal":true},
        "internal_chat_message_metadata_passthrough":{"content_token_count":5}});
    let mut history = History::default();
    history.begin(&prepared);
    history.observe(
        &prepared,
        &json!({"type":"response.output_item.done","output_index":0,"item":call}),
    );
    history.observe(&prepared, &json!({"type":"response.completed","response":{"id":"complete","status":"completed","output":[call]}}));
    let mut replay_call = call;
    for field in [
        "encrypted_function_args",
        "metadata",
        "internal_chat_message_metadata_passthrough",
    ] {
        replay_call.as_object_mut().unwrap().remove(field);
    }
    prepared.source_body["input"]
        .as_array_mut()
        .unwrap()
        .extend([
            replay_call,
            json!({"type":"function_call_output","call_id":"tool-call","output":"new result"}),
        ]);
    prepared.body = prepared.source_body.clone();
    let wire = prepared.body.clone();
    (prepared, history, wire)
}

#[test]
fn full_replay_reuses_complete_prefix_without_removing_history_or_plaintext_marker() {
    let (prepared, history, mut wire) = full_replay_fixture();
    let original = prepared.source_body.clone();
    let reduced = history
        .reduce_full_replay(&prepared, Some("complete"), &mut wire)
        .unwrap();
    assert_eq!((reduced.prefix_items, reduced.delta_items), (2, 1));
    assert_eq!(wire["previous_response_id"], "complete");
    assert_eq!(
        wire["input"],
        json!([{"type":"function_call_output","call_id":"tool-call","output":"new result"}])
    );
    assert_eq!(prepared.source_body, original);
    assert_eq!(
        history.checkpoint.as_ref().unwrap().output[0]["encrypted_function_args"],
        json!([])
    );
    assert_eq!(
        history.checkpoint.as_ref().unwrap().input[0]["content"][1]["image_url"],
        "data:image/png;base64,c3ludGhldGlj"
    );
}

#[test]
fn full_replay_rejects_semantic_crypto_and_ownership_changes() {
    for mutation in 0..15 {
        let (mut prepared, mut history, mut wire) = full_replay_fixture();
        let mut completed = Some("complete");
        match mutation {
            0 => {
                prepared.source_body["input"][1]["arguments"] = json!("{\"message\":\"different\"}")
            }
            1 => prepared.source_body["input"][1]["call_id"] = json!("different-call"),
            2 => {
                prepared.source_body["input"][0]["content"][1]["image_url"] =
                    json!("data:image/png;base64,ZGlmZmVyZW50")
            }
            3 => prepared.route.account_id = "different-account".into(),
            4 => prepared.route.owner_id = "different-owner".into(),
            5 => prepared.caller.id = "different-caller".into(),
            6 => {
                prepared
                    .headers
                    .insert("thread-id", "different-thread".parse().unwrap());
            }
            7 => history.checkpoint.as_mut().unwrap().complete_for_rebuild = false,
            8 => history.checkpoint.as_mut().unwrap().connection_local = true,
            9 => {
                history.checkpoint.as_mut().unwrap().output[0]["encrypted_function_args"] =
                    json!(["message"])
            }
            10 => completed = None,
            11 => completed = Some("another-response"),
            12 => prepared.source_body["previous_response_id"] = json!("explicit-other-parent"),
            13 => {
                prepared.source_body["client_metadata"]["parent_response_id"] =
                    json!("explicit-other-parent")
            }
            14 => prepared.continuation_recovered = true,
            _ => unreachable!(),
        }
        // Wire transformations must never make altered client history eligible.
        if mutation <= 2 || mutation == 12 || mutation == 13 {
            wire = prepared.source_body.clone();
        }
        let original = wire.clone();
        assert!(
            history
                .reduce_full_replay(&prepared, completed, &mut wire)
                .is_none(),
            "mutation {mutation}"
        );
        assert_eq!(wire, original, "mutation {mutation}");
    }
}

#[test]
fn full_replay_keeps_modified_tool_results_and_unencrypted_reasoning_content() {
    let old_result = json!({"type":"function_call_output","call_id":"call","output":"old","metadata":{"cost":1}});
    let mut changed = old_result.clone();
    changed.as_object_mut().unwrap().remove("metadata");
    assert!(!same_replay_item(&old_result, &changed));
    let reasoning = json!({"type":"reasoning","id":"r","content":[{"type":"reasoning_text","text":"visible"}],"summary":[]});
    let mut without = reasoning.clone();
    without.as_object_mut().unwrap().remove("content");
    assert!(!same_replay_item(&reasoning, &without));
    let mut encrypted = reasoning;
    encrypted["encrypted_content"] = json!("opaque-unmodified");
    without["encrypted_content"] = json!("opaque-unmodified");
    assert!(same_replay_item(&encrypted, &without));
    without["encrypted_content"] = json!("different-opaque");
    assert!(!same_replay_item(&encrypted, &without));
}
