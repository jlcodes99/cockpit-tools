//! Only an authenticated checkpoint can attest an omitted plaintext marker.
use super::*;

fn plaintext_call(name: &str) -> Value {
    json!({"type":"function_call","id":"original-item","namespace":"collaboration",
        "name":name,"call_id":"marker-call","arguments":"{\"message\":\"synthetic task\"}",
        "encrypted_function_args":[]})
}

fn checkpoint(store: &IdentityStore, call: &Value) -> Value {
    let initial = json!({"model":"fixture",
        "client_metadata":{"session_id":"root","thread_id":"root"},
        "input":[{"role":"user","content":"original task"},
            {"type":"function_call_output","namespace":"codex_app",
             "name":"send_message_to_thread","output":"original host result"}]});
    let (binding, _, _) = prepare(store, "caller", &HeaderMap::new(), &initial, "a");
    store
        .observe_event(
            &binding,
            &json!({"type":"response.output_item.done",
        "output_index":0,"item":call}),
        )
        .unwrap();
    store
        .observe_event(
            &binding,
            &json!({"type":"response.completed","response":{
        "id":"marker-checkpoint","status":"completed","output":[call]}}),
        )
        .unwrap();
    initial
}

fn continuation(initial: &Value, call: &Value, shape: &str) -> Value {
    let mut request = initial.clone();
    let result = json!({"type":"function_call_output","call_id":"marker-call","output":"done"});
    if shape == "delta" {
        request["input"] = json!([result]);
    } else {
        let mut call = call.clone();
        call["id"] = json!("round-tripped-item");
        if shape != "identical" {
            call.as_object_mut()
                .unwrap()
                .remove("encrypted_function_args");
        }
        request["input"]
            .as_array_mut()
            .unwrap()
            .extend([call, result]);
    }
    if shape != "stateless" {
        request["previous_response_id"] = json!("marker-checkpoint");
    }
    request
}

fn message_registered(store: &IdentityStore) -> bool {
    store.connection.lock().unwrap().query_row(
        "SELECT EXISTS(SELECT 1 FROM native_opaque_v1 WHERE caller='caller' AND kind='encrypted' AND digest=?1)",
        params![digest("encrypted", "synthetic task").digest], |row| row.get(0),
    ).unwrap()
}

#[test]
fn checkpoint_marker_replay_preserves_all_four_request_shapes_after_restart() {
    for name in ["spawn_agent", "send_message", "followup_task"] {
        for shape in ["stateless", "delta", "identical", "omitted"] {
            let (temp, store) = fixture();
            let call = plaintext_call(name);
            let initial = checkpoint(&store, &call);
            assert!(!message_registered(&store));
            drop(store);
            let store = IdentityStore::open(temp.path().join("identity.sqlite")).unwrap();
            let request = continuation(&initial, &call, shape);
            let saved = request.clone();
            let mut context = store
                .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
                .unwrap_or_else(|error| panic!("{name}/{shape}: {error:?}"));
            assert_eq!(
                rebuild::full_input(
                    &store.connection.lock().unwrap(),
                    "caller",
                    Some("root"),
                    &request
                )
                .unwrap(),
                shape != "delta",
                "full-input detection must use the same checkpoint comparison: {name}/{shape}",
            );
            let plan = store
                .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
                .unwrap()
                .expect("complete history must remain rebuildable");
            let expected = if shape == "delta" {
                let mut input = initial["input"].as_array().unwrap().clone();
                input.push(call.clone());
                input.extend(request["input"].as_array().unwrap().clone());
                json!(input)
            } else {
                request["input"].clone()
            };
            assert_eq!(plan.body["input"], expected);
            let mut sent = request.clone();
            let binding = store
                .rewrite(&context, "a", "owner-a", &mut HeaderMap::new(), &mut sent)
                .unwrap();
            assert_eq!(sent["input"], request["input"]);
            assert!(
                !message_registered(&store),
                "preparation cannot publish input provenance"
            );
            complete(&store, &binding, "marker-next");
            assert_eq!(
                message_registered(&store),
                matches!(shape, "stateless" | "omitted")
            );
            let original_checkpoint: String = store.connection.lock().unwrap().query_row(
                "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='marker-checkpoint'",
                [], |row| row.get(0),
            ).unwrap();
            let original_checkpoint: Vec<Value> =
                serde_json::from_str(&original_checkpoint).unwrap();
            assert_eq!(original_checkpoint.last(), Some(&call));
            assert_eq!(request, saved);
        }
    }
}

#[test]
fn checkpoint_marker_replay_does_not_relax_other_call_envelopes_or_nested_ids() {
    for (field, value) in [
        ("namespace", json!("other")),
        ("name", json!("read_file")),
        ("call_id", json!("")),
        ("call_id", json!(" ")),
    ] {
        let (_temp, store) = fixture();
        let mut call = plaintext_call("followup_task");
        call[field] = value;
        let initial = checkpoint(&store, &call);
        let request = continuation(&initial, &call, "omitted");
        assert!(
            !rebuild::full_input(
                &store.connection.lock().unwrap(),
                "caller",
                Some("root"),
                &request
            )
            .unwrap(),
            "the omission exception must not cover {field}={:?}",
            call[field]
        );
    }
    let (_temp, store) = fixture();
    let mut call = plaintext_call("followup_task");
    call["extension"] = json!({"id":"original-nested-id"});
    let initial = checkpoint(&store, &call);
    let mut request = continuation(&initial, &call, "omitted");
    request["input"][2]["extension"]["id"] = json!("changed-nested-id");
    assert!(!rebuild::full_input(
        &store.connection.lock().unwrap(),
        "caller",
        Some("root"),
        &request
    )
    .unwrap());
}

#[test]
fn checkpoint_marker_replay_requires_the_same_caller_thread_and_call() {
    let (_temp, store) = fixture();
    let call = plaintext_call("followup_task");
    let initial = checkpoint(&store, &call);
    let request = continuation(&initial, &call, "omitted");
    assert_eq!(
        store
            .inspect_for_rebuild("foreign", &HeaderMap::new(), &request)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    let mut other_thread = request.clone();
    other_thread["client_metadata"]["thread_id"] = json!("other-thread");
    assert!(store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &other_thread)
        .is_err());
    for (field, value) in [
        ("namespace", json!("other")),
        ("name", json!("send_message")),
        ("call_id", json!("other-call")),
        ("arguments", json!("{\"message\":\"changed task\"}")),
        ("encrypted_function_args", Value::Null),
        ("encrypted_function_args", json!(["cipher"])),
    ] {
        let mut changed = request.clone();
        changed["input"][2][field] = value;
        assert!(
            !rebuild::complete_input_available(
                &store.connection.lock().unwrap(),
                "caller",
                Some("root"),
                &HeaderMap::new(),
                &changed
            )
            .unwrap(),
            "changed {field} cannot match the checkpoint"
        );
        assert!(!rebuild::full_input(
            &store.connection.lock().unwrap(),
            "caller",
            Some("root"),
            &changed
        )
        .unwrap());
    }
    assert!(!message_registered(&store));
}

#[test]
fn checkpoint_marker_replay_does_not_ignore_nonempty_missing_or_reverse_markers() {
    for marker in [None, Some(Value::Null), Some(json!(["cipher"]))] {
        let (_temp, store) = fixture();
        let mut call = plaintext_call("followup_task");
        if let Some(marker) = marker {
            call["encrypted_function_args"] = marker;
        } else {
            call.as_object_mut()
                .unwrap()
                .remove("encrypted_function_args");
        }
        let initial = checkpoint(&store, &call);
        let mut request = continuation(&initial, &call, "identical");
        request["input"][2]["encrypted_function_args"] = json!([]);
        assert!(
            !rebuild::complete_input_available(
                &store.connection.lock().unwrap(),
                "caller",
                Some("root"),
                &HeaderMap::new(),
                &request
            )
            .unwrap(),
            "an incoming marker cannot reclassify checkpoint evidence"
        );
        if call.get("encrypted_function_args").is_some() {
            request["input"][2]
                .as_object_mut()
                .unwrap()
                .remove("encrypted_function_args");
            assert!(
                !rebuild::complete_input_available(
                    &store.connection.lock().unwrap(),
                    "caller",
                    Some("root"),
                    &HeaderMap::new(),
                    &request
                )
                .unwrap(),
                "only an attested empty marker may be omitted"
            );
        }
    }
}

#[test]
fn checkpoint_marker_replay_cancellation_does_not_publish_the_reinterpreted_message() {
    for failure in ["response.failed", "response.cancelled"] {
        let (_temp, store) = fixture();
        let call = plaintext_call("followup_task");
        let initial = checkpoint(&store, &call);
        let mut request = continuation(&initial, &call, "omitted");
        let mut context = store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
            .unwrap();
        store
            .configure_rebuild(&mut context, &HeaderMap::new(), &request, true)
            .unwrap()
            .unwrap();
        let binding = store
            .rewrite(
                &context,
                "a",
                "owner-a",
                &mut HeaderMap::new(),
                &mut request,
            )
            .unwrap();
        store
            .observe_event(
                &binding,
                &json!({"type":failure,"response":{
            "id":"failed-next","status":failure.strip_prefix("response.").unwrap(),"output":[]}}),
            )
            .unwrap();
        complete(&store, &binding, "late-next");
        assert!(!message_registered(&store));
    }
}

#[test]
fn checkpoint_marker_replay_can_detach_a_foreign_owner_parent_only_with_the_attested_prefix() {
    let (_temp, store) = fixture();
    let call = plaintext_call("followup_task");
    let initial = checkpoint(&store, &call);
    let mut other = initial.clone();
    other["client_metadata"] = json!({"session_id":"other-root","thread_id":"other-root"});
    let (binding, _, _) = prepare(&store, "caller", &HeaderMap::new(), &other, "b");
    complete(&store, &binding, "other-parent");
    let mut request = continuation(&initial, &call, "omitted");
    request["client_metadata"]["parent_response_id"] = json!("other-parent");
    let context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &request)
        .unwrap();
    assert!(context.detached_parent);
    let mut changed = request.clone();
    changed["input"][2]["arguments"] = json!("{\"message\":\"changed task\"}");
    assert_eq!(
        store
            .inspect_for_rebuild("caller", &HeaderMap::new(), &changed)
            .unwrap_err()
            .code,
        "mixed_upstream_state"
    );
}
