//! Complete wire history is replayed, never decrypted or relabelled as plaintext.
use super::*;
use serde_json::json;

fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{"type":if role=="assistant" {"output_text"} else {"input_text"},"text":text}]})
}
fn body(input: Vec<Value>) -> Value {
    json!({"model":"fixture","client_metadata":{"session_id":"root","thread_id":"root"},"input":input})
}
fn fixture() -> (tempfile::TempDir, PathBuf, IdentityStore) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    (temp, path, store)
}
fn opaque() -> Vec<Value> {
    vec![
        json!({"type":"reasoning","id":"reasoning-a","summary":[{"type":"summary_text","text":"visible summary"}],"encrypted_content":"reasoning-a-cipher","extension":{"kept":true}}),
        json!({"type":"compaction","id":"compaction-a","encrypted_content":"compaction-a-cipher"}),
        json!({"type":"function_call","id":"call-a","namespace":"collaboration","name":"send_message","arguments":"{\"id\":\"worker\",\"message\":\"agent-message-a-cipher\"}","encrypted_function_args":["argument-a-cipher"],"call_id":"call-id-a"}),
        json!({"type":"agent_message","id":"agent-a","author":"root","recipient":"worker","content":[{"type":"input_text","text":"Payload:"},{"type":"encrypted_content","encrypted_content":"agent-message-a-cipher"}]}),
        json!({"type":"custom_tool_call","id":"custom-a","name":"apply_patch","input":"*** Begin Patch\n*** End Patch","call_id":"custom-id-a"}),
    ]
}
fn complete(store: &IdentityStore, binding: &Binding, id: &str, output: &[Value]) {
    for (index, item) in output.iter().enumerate() {
        store
            .observe_event(
                binding,
                &json!({"type":"response.output_item.done","output_index":index,"item":item}),
            )
            .unwrap();
    }
    store.observe_event(binding, &json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output}})).unwrap();
}
fn seed(store: &IdentityStore) -> Vec<Value> {
    let mut request = body(vec![message("user", "first")]);
    let mut headers = HeaderMap::new();
    let context = store.inspect("caller", &headers, &request).unwrap();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut request)
        .unwrap();
    let output = opaque();
    complete(store, &binding, "a-response", &output);
    let mut full = vec![message("user", "first")];
    full.extend(output);
    full.push(json!({"type":"function_call_output","id":"result-a","call_id":"call-id-a","output":"delivered"}));
    full.push(json!({"type":"custom_tool_call_output","id":"custom-result-a","call_id":"custom-id-a","output":"patch complete"}));
    full.push(message("user", "second"));
    full
}
fn prepare(store: &IdentityStore, request: &Value, account: &str) -> (Binding, Value) {
    let mut headers = HeaderMap::new();
    let mut context = store
        .inspect_for_rebuild("caller", &headers, request)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &headers, request, true)
        .unwrap()
        .unwrap();
    assert_eq!(plan.body["input"], request["input"]);
    let mut upstream = request.clone();
    let binding = store
        .rewrite(
            &context,
            account,
            &format!("owner-{account}"),
            &mut headers,
            &mut upstream,
        )
        .unwrap();
    (binding, upstream)
}
fn source(path: &PathBuf, cipher: &str) -> (String, String) {
    Connection::open(path).unwrap().query_row("SELECT account,owner FROM native_opaque_v1 WHERE caller='caller' AND kind='encrypted' AND digest=?1",[digest("encrypted",cipher).digest],|row|Ok((row.get(0)?,row.get(1)?))).unwrap()
}
fn active(path: &PathBuf, thread: &str) -> Option<(String, String, i64)> {
    Connection::open(path).unwrap().query_row("SELECT account,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread=?1",[thread],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().unwrap()
}

#[test]
fn complete_opaque_history_preserves_ids_markers_and_original_provenance_after_restart_and_disable()
{
    let (_temp, path, store) = fixture();
    let request = body(seed(&store));
    let (binding, upstream) = prepare(&store, &request, "b");
    assert_eq!(upstream["input"], request["input"]);
    assert!(!binding.generation.as_ref().unwrap().id.is_empty());
    assert!(active(&path, "root").is_none());
    let output = vec![
        opaque()[0].clone(),
        json!({"type":"reasoning","id":"reasoning-b","encrypted_content":"reasoning-b-cipher","summary":[]}),
        message("assistant", "b answer"),
    ];
    complete(&store, &binding, "b-response", &output);
    assert_eq!(
        source(&path, "reasoning-a-cipher"),
        ("a".into(), "owner-a".into())
    );
    assert_eq!(
        source(&path, "reasoning-b-cipher"),
        ("b".into(), "owner-b".into())
    );
    let before = active(&path, "root").unwrap();
    drop(store);
    let store = IdentityStore::open(path.clone()).unwrap();
    let mut next = request.clone();
    next["input"].as_array_mut().unwrap().extend(output);
    next["input"]
        .as_array_mut()
        .unwrap()
        .push(message("user", "third"));
    next["previous_response_id"] = json!("b-response");
    let mut headers = HeaderMap::new();
    let mut context = store.inspect("caller", &headers, &next).unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("b"));
    assert!(store
        .configure_rebuild(&mut context, &headers, &next, false)
        .unwrap()
        .is_none());
    let mut upstream = next.clone();
    let resumed = store
        .rewrite(&context, "b", "owner-b", &mut headers, &mut upstream)
        .unwrap();
    assert_eq!(resumed.generation.as_ref().unwrap().id, before.1);
    assert_eq!(upstream["input"], next["input"]);
    complete(&store, &resumed, "b-next", &[message("assistant", "next")]);
    assert_eq!(active(&path, "root").unwrap(), before);
    assert_eq!(source(&path, "reasoning-a-cipher").0, "a");
    assert_eq!(
        store
            .inspect_for_rebuild("other-caller", &HeaderMap::new(), &request)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
}

#[test]
fn failed_or_cancelled_replay_cannot_authorize_disabled_mixed_history() {
    for cancelled in [false, true] {
        let (_temp, path, store) = fixture();
        let request = body(seed(&store));
        let (binding, _) = prepare(&store, &request, "b");
        if cancelled {
            binding.abort_rebuild();
        } else {
            store.observe_event(&binding,&json!({"type":"response.failed","response":{"id":"failed-b","status":"failed","output":[]}})).unwrap();
        }
        complete(
            &store,
            &binding,
            "late-b",
            &[json!({"type":"reasoning","encrypted_content":"late-b-cipher","summary":[]})],
        );
        assert!(active(&path, "root").is_none());
        let permissions: i64 = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_rebuild_portable_v1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(permissions, 0);
        let mut mixed = request.clone();
        mixed["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"reasoning","encrypted_content":"late-b-cipher","summary":[]}));
        assert_eq!(
            store
                .inspect("caller", &HeaderMap::new(), &mixed)
                .unwrap_err()
                .code,
            "mixed_upstream_state"
        );
        assert_eq!(source(&path, "reasoning-a-cipher").0, "a");
    }
}

#[test]
fn incomplete_or_resource_history_restores_original_pin_before_first_active_generation() {
    let (_temp, _path, store) = fixture();
    let full = seed(&store);
    for control in [
        json!({"type":"item_reference","id":"only-a-server-item"}),
        json!({"type":"message","role":"user","content":[{"type":"input_image","file_id":"only-a-file"}]}),
    ] {
        let mut request = body(full.clone());
        request["input"].as_array_mut().unwrap().push(control);
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
            .unwrap();
        assert!(store
            .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
            .unwrap()
            .is_none());
        store.protect_unrebuildable(&mut context).unwrap();
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
        assert!(store
            .rewrite(
                &context,
                "b",
                "owner-b",
                &mut HeaderMap::new(),
                &mut request
            )
            .is_err());
    }
    for parameters in [
        json!({"conversation":"a-conversation"}),
        json!({"tools":[{"type":"file_search","vector_store_ids":["a-vector"]}]}),
    ] {
        let mut request = body(full.clone());
        request
            .as_object_mut()
            .unwrap()
            .extend(parameters.as_object().unwrap().clone());
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
            .unwrap();
        assert!(store
            .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
            .unwrap()
            .is_none());
        store.protect_unrebuildable(&mut context).unwrap();
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
    }
    let mut request = body(full);
    request["tools"] = json!([{"type":"namespace","name":"collaboration","tools":[{"type":"function","name":"send_message","parameters":{}}]},{"type":"custom","name":"apply_patch"},{"type":"web_search"}]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    assert!(store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .is_some());
}

#[test]
fn child_full_replay_detaches_old_parent_anchor_without_overwriting_root_or_restarting_b() {
    let (_temp, path, store) = fixture();
    seed(&store);
    let mut original = body(vec![opaque()[3].clone(), message("user", "child task")]);
    original["client_metadata"]["thread_id"] = json!("child");
    original["client_metadata"]["x-codex-parent-thread-id"] = json!("root");
    original["client_metadata"]["parent_response_id"] = json!("a-response");
    let headers = HeaderMap::new();
    let mut context = store
        .inspect_for_rebuild("caller", &headers, &original)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &headers, &original, true)
        .unwrap()
        .unwrap();
    let mut context = store
        .inspect_for_rebuild("caller", &plan.headers, &plan.body)
        .unwrap();
    store
        .configure_rebuild(&mut context, &plan.headers, &plan.body, true)
        .unwrap();
    context.start_rebuild_generation(&plan);
    let mut upstream = plan.body.clone();
    let binding = store
        .rewrite(
            &context,
            "b",
            "owner-b",
            &mut HeaderMap::new(),
            &mut upstream,
        )
        .unwrap();
    let output = [message("assistant", "child answer")];
    complete(&store, &binding, "child-b", &output);
    assert!(active(&path, "root").is_none());
    let before = active(&path, "child").unwrap();
    let root_affinity:String=Connection::open(&path).unwrap().query_row("SELECT account FROM native_affinity_v1 WHERE caller='caller' AND kind='session' AND original='root'",[],|row|row.get(0)).unwrap();
    assert_eq!(root_affinity, "a");
    let mut full = original.clone();
    full["previous_response_id"] = json!("child-b");
    full["input"].as_array_mut().unwrap().extend(output);
    full["input"]
        .as_array_mut()
        .unwrap()
        .push(message("user", "child next"));
    // The switch may be closed: completion permissions persist for this generation.
    let context = store.inspect("caller", &HeaderMap::new(), &full).unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("b"));
    let mut upstream = full.clone();
    let resumed = store
        .rewrite(
            &context,
            "b",
            "owner-b",
            &mut HeaderMap::new(),
            &mut upstream,
        )
        .unwrap();
    assert!(upstream["client_metadata"]
        .get("parent_response_id")
        .is_none());
    assert_eq!(upstream["input"], full["input"]);
    complete(
        &store,
        &resumed,
        "child-b-next",
        &[message("assistant", "next")],
    );
    assert_eq!(active(&path, "child").unwrap(), before);
    let mut delta = original;
    delta["previous_response_id"] = json!("child-b");
    delta["input"] = json!([message("user", "only delta")]);
    assert_eq!(
        store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &delta)
            .unwrap_err()
            .code,
        "mixed_upstream_state"
    );
}

#[test]
fn unknown_history_retries_automatically_and_publishes_only_successful_provenance() {
    let (_temp, path, store) = fixture();
    let request = body(vec![
        message("user", "old turn"),
        json!({"type":"reasoning","encrypted_content":"old-unknown-cipher","summary":[]}),
        message("assistant", "old answer"),
        message("user", "continue"),
    ]);
    let (failed, upstream) = prepare(&store, &request, "a");
    assert_eq!(upstream["input"], request["input"]);
    store.observe_event(&failed, &json!({"type":"response.output_item.done","output_index":0,"item":request["input"][1]})).unwrap();
    store.observe_event(&failed, &json!({"type":"response.failed","response":{"id":"failed","status":"failed","output":[]}})).unwrap();
    complete(&store, &failed, "late", &[request["input"][1].clone()]);
    assert!(active(&path, "root").is_none());
    let count: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "failed and late terminal frames cannot claim input"
    );
    let (accepted, retried) = prepare(&store, &request, "b");
    assert_eq!(retried["input"], request["input"]);
    complete(
        &store,
        &accepted,
        "accepted-b",
        &[message("assistant", "answer")],
    );
    assert_eq!(
        source(&path, "old-unknown-cipher"),
        ("b".into(), "owner-b".into())
    );
    assert_eq!(active(&path, "root").unwrap().0, "b");
    drop(store);
    let store = IdentityStore::open(path).unwrap();
    let (_, resumed) = prepare(&store, &request, "b");
    assert_eq!(
        resumed["client_metadata"]["thread_id"],
        retried["client_metadata"]["thread_id"]
    );
}

#[test]
fn completed_b_generation_discards_only_observed_old_a_sticky_tokens_in_full_and_delta_requests() {
    for enabled in [false, true] {
        for kind in ["header", "flat", "canonical-body", "canonical-header"] {
            for full in [false, true] {
                let (_temp, path, store) = fixture();
                let input = seed(&store);
                let mut a_headers = HeaderMap::new();
                let mut a_body = body(vec![message("user", "token observation")]);
                let a_context = store.inspect("caller", &a_headers, &a_body).unwrap();
                let a_binding = store
                    .rewrite(&a_context, "a", "owner-a", &mut a_headers, &mut a_body)
                    .unwrap();
                let mut observed = HeaderMap::new();
                observed.insert(
                    "x-codex-turn-state",
                    HeaderValue::from_static("old-a-token"),
                );
                store.observe_headers(&a_binding, &observed).unwrap();
                let original = body(input.clone());
                let (b_binding, _) = prepare(&store, &original, "b");
                let output = [message("assistant", "b answer")];
                complete(&store, &b_binding, "b-sticky-response", &output);
                observed.insert(
                    "x-codex-turn-state",
                    HeaderValue::from_static("new-b-token"),
                );
                store.observe_headers(&b_binding, &observed).unwrap();
                let before = active(&path, "root").unwrap();
                let mut next = body(if full {
                    let mut input = input;
                    input.extend(output);
                    input.push(message("user", "next"));
                    input
                } else {
                    vec![message("user", "next")]
                });
                next["previous_response_id"] = json!("b-sticky-response");
                let mut headers = HeaderMap::new();
                let canonical=json!({"session_id":"root","thread_id":"root","x-codex-turn-state":"old-a-token","extension":"kept"}).to_string();
                match kind {
                    "header" => {
                        headers.insert(
                            "x-codex-turn-state",
                            HeaderValue::from_static("old-a-token"),
                        );
                    }
                    "flat" => {
                        next["client_metadata"]["x-codex-turn-state"] = json!("old-a-token");
                        headers.insert(
                            "x-codex-turn-state",
                            HeaderValue::from_static("new-b-token"),
                        );
                    }
                    "canonical-body" => {
                        next["client_metadata"]["x-codex-turn-metadata"] = json!(canonical)
                    }
                    _ => {
                        headers.insert(
                            "x-codex-turn-metadata",
                            HeaderValue::from_str(&canonical).unwrap(),
                        );
                    }
                }
                let unchanged = next.clone();
                let mut context = if enabled {
                    store.inspect_for_rebuild("caller", &headers, &next)
                } else {
                    store.inspect("caller", &headers, &next)
                }
                .unwrap();
                assert_eq!(context.state_account_id.as_deref(), Some("b"));
                store
                    .configure_rebuild(&mut context, &headers, &next, enabled)
                    .unwrap();
                let mut upstream = next.clone();
                let binding = store
                    .rewrite(&context, "b", "owner-b", &mut headers, &mut upstream)
                    .unwrap();
                assert_eq!(binding.generation.as_ref().unwrap().id, before.1);
                assert_eq!(upstream["previous_response_id"], "b-sticky-response");
                assert_eq!(upstream["input"], next["input"]);
                assert!(upstream["client_metadata"]
                    .get("x-codex-turn-state")
                    .is_none());
                if kind == "flat" {
                    assert_eq!(headers.get("x-codex-turn-state").unwrap(), "new-b-token");
                } else {
                    assert!(!headers.contains_key("x-codex-turn-state"));
                }
                for raw in [
                    upstream
                        .pointer("/client_metadata/x-codex-turn-metadata")
                        .and_then(Value::as_str),
                    headers
                        .get("x-codex-turn-metadata")
                        .and_then(|v| v.to_str().ok()),
                ]
                .into_iter()
                .flatten()
                {
                    let canonical: Value = serde_json::from_str(raw).unwrap();
                    assert!(canonical.get("x-codex-turn-state").is_none());
                    assert_eq!(canonical["extension"], "kept");
                }
                complete(
                    &store,
                    &binding,
                    "next-b-sticky-response",
                    &[message("assistant", "continued")],
                );
                assert_eq!(active(&path, "root").unwrap(), before);
                assert_eq!(next, unchanged);
                let reference = turn_state_reference("old-a-token", Some("root"), None);
                let origin:String=Connection::open(&path).unwrap().query_row("SELECT account FROM native_opaque_v1 WHERE caller='caller' AND kind='turn_state' AND digest=?1",[reference.digest],|row|row.get(0)).unwrap();
                assert_eq!(origin, "a");
                let mut unknown_headers = HeaderMap::new();
                unknown_headers.insert(
                    "x-codex-turn-state",
                    HeaderValue::from_static("unknown-token"),
                );
                let automatic = store
                    .inspect_for_rebuild("caller", &unknown_headers, &next)
                    .unwrap();
                let mut automatic_body = next.clone();
                store
                    .rewrite(
                        &automatic,
                        "b",
                        "owner-b",
                        &mut unknown_headers,
                        &mut automatic_body,
                    )
                    .unwrap();
                assert!(!unknown_headers.contains_key("x-codex-turn-state"));
                assert_eq!(automatic_body["input"], next["input"]);
                assert_eq!(
                    automatic_body["previous_response_id"],
                    next["previous_response_id"]
                );
                let mut wrong_previous = next.clone();
                wrong_previous["previous_response_id"] = json!("a-response");
                unknown_headers.insert(
                    "x-codex-turn-state",
                    HeaderValue::from_static("new-b-token"),
                );
                assert_eq!(
                    store
                        .inspect_for_rebuild("caller", &unknown_headers, &wrong_previous)
                        .unwrap_err()
                        .code,
                    "mixed_upstream_state"
                );
            }
        }
    }
}

#[test]
fn missing_plan_cannot_use_a_legacy_affinity_or_server_pin_to_authorize_unaccepted_mixed_blobs() {
    let (_temp, _path, store) = fixture();
    let full = seed(&store);
    let mut other = body(vec![message("user", "independent B")]);
    other["client_metadata"]["thread_id"] = json!("other");
    other["client_metadata"]["session_id"] = json!("other");
    let context = store.inspect("caller", &HeaderMap::new(), &other).unwrap();
    let binding = store
        .rewrite(&context, "b", "owner-b", &mut HeaderMap::new(), &mut other)
        .unwrap();
    complete(
        &store,
        &binding,
        "other-b",
        &[json!({"type":"reasoning","encrypted_content":"unaccepted-b-blob","summary":[]})],
    );
    for server_pin in [false, true] {
        let mut request = body(full.clone());
        request["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"reasoning","encrypted_content":"unaccepted-b-blob","summary":[]}));
        request["input"].as_array_mut().unwrap().push(
            json!({"type":"function_call","call_id":"pending","name":"lookup","arguments":"{}"}),
        );
        if server_pin {
            request["previous_response_id"] = json!("a-response");
        }
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
            .unwrap();
        assert!(store
            .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
            .unwrap()
            .is_none());
        assert_eq!(
            store.protect_unrebuildable(&mut context).unwrap_err().code,
            "mixed_upstream_state"
        );
    }
}

#[test]
fn unknown_structured_output_blob_cannot_hide_behind_an_unrelated_content_extension() {
    let (_temp, _path, store) = fixture();
    for item in [
        json!({"type":"function_call_output","call_id":"call","content":null,"output":[{"type":"encrypted_content","encrypted_content":"unknown-tool-output"}]}),
        json!({"type":"custom_tool_call_output","call_id":"call","content":"unrelated extension","output":[{"type":"encrypted_content","encrypted_content":"unknown-custom-output"}]}),
    ] {
        let request = body(vec![item]);
        assert_eq!(
            store
                .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
                .unwrap_err()
                .code,
            "continuation_history_incomplete"
        );
    }
    let request = body(vec![
        json!({"type":"message","role":"assistant","content":[{"type":"encrypted_content","encrypted_content":"unknown-message-part"}]}),
    ]);
    let (_, sent) = prepare(&store, &request, "a");
    assert_eq!(sent["input"], request["input"]);
}

#[test]
fn tool_schema_resource_property_names_are_replayable_but_real_resource_configuration_is_not() {
    let (_temp, _path, store) = fixture();
    let full = seed(&store);
    let schema = json!({"type":"object","properties":{"file_id":{"type":"string"},"vector_store_ids":{"type":"array","items":{"type":"string"}},"asset_pointer":{"type":"string"},"container_id":{"type":"string"}}});
    let tools = json!([
        {"type":"function","name":"read_file","parameters":schema},
        {"type":"function","function":{"name":"old_wire_function","parameters":schema}},
        {"type":"namespace","name":"workspace","tools":[{"type":"function","name":"search","parameters":schema}]},
        {"type":"custom","name":"structured_custom","input_schema":schema},
    ]);
    let mut request = body(full);
    request["tools"] = tools.clone();
    request["input"].as_array_mut().unwrap().extend([
        json!({"type":"additional_tools","role":"developer","tools":tools}),
        json!({"type":"tool_search_output","tools":[{"name":"inventory_function","parameters":schema}]}),
    ]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    let plan = store
        .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
        .unwrap()
        .unwrap();
    assert_eq!(plan.body["tools"], request["tools"]);
    assert_eq!(plan.body["input"], request["input"]);
    for actual in [
        json!({"type":"file_search","vector_store_ids":["actual-vector"]}),
        json!({"type":"code_interpreter","container":{"file_ids":["actual-file"]}}),
    ] {
        let mut configured = request.clone();
        configured["tools"] = json!([actual]);
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &configured)
            .unwrap();
        assert!(store
            .configure_rebuild(&mut context, &HeaderMap::new(), &configured, true)
            .unwrap()
            .is_none());
        for item_type in ["additional_tools", "tool_search_output"] {
            let mut inventory = request.clone();
            inventory["input"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":item_type,"role":"developer","tools":[actual]}));
            let mut context = store
                .inspect_for_rebuild("caller", &HeaderMap::new(), &inventory)
                .unwrap();
            assert!(store
                .configure_rebuild(&mut context, &HeaderMap::new(), &inventory, true)
                .unwrap()
                .is_none());
        }
    }
}
