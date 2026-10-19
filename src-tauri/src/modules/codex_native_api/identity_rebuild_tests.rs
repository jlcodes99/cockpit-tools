//! Checkpoints authorize a new upstream generation, never transfer ciphertext ownership.
use super::*;
use serde_json::json;
use std::path::Path;

fn fixture() -> (tempfile::TempDir, PathBuf, IdentityStore) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    (temp, path, store)
}
fn message(role: &str, text: &str) -> Value {
    json!({"type":"message","role":role,"content":[{
        "type":if role=="assistant" {"output_text"} else {"input_text"},"text":text}]})
}
fn request(input: Vec<Value>) -> Value {
    json!({"model":"fixture","client_metadata":{"session_id":"root","thread_id":"root"},"input":input})
}
fn continuation(response: &str, input: Vec<Value>) -> Value {
    let mut body = request(input);
    body["previous_response_id"] = json!(response);
    body
}
fn owner(account: &str) -> String {
    format!("owner-{account}")
}
fn prepare(
    store: &IdentityStore,
    caller: &str,
    account: &str,
    body: &Value,
    enabled: bool,
) -> (Binding, HeaderMap, Value) {
    let mut headers = HeaderMap::new();
    let mut upstream = body.clone();
    let mut context = store.inspect(caller, &headers, body).unwrap();
    let candidate = store
        .configure_rebuild(&mut context, &headers, body, enabled)
        .unwrap();
    if enabled && candidate.is_none() {
        store.protect_unrebuildable(&mut context).unwrap();
    }
    let binding = store
        .rewrite(
            &context,
            account,
            &owner(account),
            &mut headers,
            &mut upstream,
        )
        .unwrap();
    (binding, headers, upstream)
}
fn rebuilt(store: &IdentityStore, account: &str, original: &Value) -> (Binding, HeaderMap, Value) {
    let headers = HeaderMap::new();
    let mut original_context = store.inspect("caller", &headers, original).unwrap();
    let plan = store
        .configure_rebuild(&mut original_context, &headers, original, true)
        .unwrap()
        .expect("complete plaintext checkpoint");
    rebuilt_plan(store, account, plan)
}
fn rebuilt_plan(store: &IdentityStore, account: &str, plan: Plan) -> (Binding, HeaderMap, Value) {
    // Match mapping::prepare_rebuilt: inspect the candidate instead of changing
    // the original context's opaque references or its hard owner by hand.
    let mut context = store.inspect("caller", &plan.headers, &plan.body).unwrap();
    store
        .configure_rebuild(&mut context, &plan.headers, &plan.body, true)
        .unwrap();
    context.start_rebuild_generation(&plan);
    let mut headers = plan.headers;
    let mut body = plan.body;
    let binding = store
        .rewrite(&context, account, &owner(account), &mut headers, &mut body)
        .unwrap();
    (binding, headers, body)
}
fn complete(store: &IdentityStore, binding: &Binding, id: &str, output: Vec<Value>) {
    for (index, item) in output.iter().enumerate() {
        store
            .observe_event(
                binding,
                &json!({"type":"response.output_item.done","output_index":index,"item":item}),
            )
            .unwrap();
    }
    store
        .observe_event(
            binding,
            &json!({"type":"response.completed","response":{
        "id":id,"status":"completed","output":output}}),
        )
        .unwrap();
}
fn root(store: &IdentityStore) -> (Binding, HeaderMap, Value) {
    let prepared = prepare(
        store,
        "caller",
        "a",
        &request(vec![message("user", "first")]),
        true,
    );
    complete(
        store,
        &prepared.0,
        "a-root",
        vec![message("assistant", "answer-a")],
    );
    prepared
}
fn active_row(path: &Path) -> Option<(String, String, String, i64)> {
    Connection::open(path).unwrap().query_row(
        "SELECT account,owner,generation,revision FROM native_rebuild_active_v1 WHERE caller='caller' AND thread='root'",
        [],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).optional().unwrap()
}
fn checkpoint(path: &Path, id: &str) -> Option<(String, String, Vec<Value>)> {
    Connection::open(path).unwrap().query_row(
        "SELECT account,generation,transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response=?1",
        [id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)),
    ).optional().unwrap().map(|(account,generation,raw)|(account,generation,serde_json::from_str(&raw).unwrap()))
}
fn rows(path: &Path, table: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
fn affinity(path: &Path) -> Vec<(String, String, String)> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection.prepare("SELECT kind,account,owner FROM native_affinity_v1 WHERE caller='caller' AND original='root' ORDER BY kind").unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
fn assert_affinity(path: &Path, account: &str) {
    assert_eq!(
        affinity(path),
        vec![
            ("session".into(), account.into(), owner(account)),
            ("thread".into(), account.into(), owner(account))
        ]
    );
}
fn plan(store: &IdentityStore, body: &Value, enabled: bool) -> (RequestIdentity, Option<Plan>) {
    let headers = HeaderMap::new();
    let mut context = store.inspect("caller", &headers, body).unwrap();
    let plan = store
        .configure_rebuild(&mut context, &headers, body, enabled)
        .unwrap();
    if enabled && plan.is_none() {
        store.protect_unrebuildable(&mut context).unwrap();
    }
    (context, plan)
}

#[test]
fn disabled_checkpoint_capture_and_missing_or_unknown_checkpoint_keep_owner_pins() {
    let (_temp, path, store) = fixture();
    let input = request(vec![message("user", "first")]);
    let prepared = prepare(&store, "caller", "a", &input, false);
    complete(
        &store,
        &prepared.0,
        "uncached-a",
        vec![message("assistant", "reply")],
    );
    assert!(active_row(&path).is_none());
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 0);
    let followup = continuation("uncached-a", vec![message("user", "next")]);
    for enabled in [false, true] {
        let (context, candidate) = plan(&store, &followup, enabled);
        assert!(candidate.is_none());
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
    }
    let unknown = continuation(
        "unknown-response",
        vec![
            message("assistant", "looks complete"),
            message("user", "next"),
        ],
    );
    assert_eq!(
        store
            .inspect("caller", &HeaderMap::new(), &unknown)
            .unwrap_err()
            .code,
        "previous_response_not_found"
    );
    root(&store);
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DELETE FROM native_rebuild_checkpoints_v1; DELETE FROM native_rebuild_history_v1;",
        )
        .unwrap();
    let (context, candidate) = plan(
        &store,
        &continuation("a-root", vec![message("user", "next")]),
        true,
    );
    assert!(candidate.is_none());
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
}

#[test]
fn complete_plaintext_checkpoint_joins_delta_and_preserves_original_request() {
    let (_temp, path, store) = fixture();
    root(&store);
    let mut original = continuation("a-root", vec![message("user", "second")]);
    original["client_metadata"]["extension"] = json!("kept");
    let before = original.clone();
    let (context, candidate) = plan(&store, &original, true);
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    let candidate = candidate.unwrap();
    assert_eq!(
        candidate.body["input"],
        json!([
            message("user", "first"),
            message("assistant", "answer-a"),
            message("user", "second")
        ])
    );
    assert!(candidate.body.get("previous_response_id").is_none());
    assert_eq!(candidate.body["client_metadata"]["thread_id"], "root");
    assert_eq!(candidate.body["client_metadata"]["extension"], "kept");
    assert_eq!(original, before);
    let (binding, _, upstream) = rebuilt(&store, "b", &original);
    assert_eq!(binding.owner_id, "owner-b");
    assert_eq!(upstream["input"], candidate.body["input"]);
    assert_eq!(
        active_row(&path).unwrap().0,
        "a",
        "preparation must not activate b"
    );
    complete(
        &store,
        &binding,
        "b-rebuilt",
        vec![message("assistant", "answer-b")],
    );
    let row = active_row(&path).unwrap();
    assert_eq!((row.0, row.1), ("b".into(), "owner-b".into()));
    assert_eq!(checkpoint(&path, "b-rebuilt").unwrap().2.len(), 4);
    let old = store
        .inspect("caller", &HeaderMap::new(), &original)
        .unwrap();
    assert_eq!(
        old.state_account_id.as_deref(),
        Some("a"),
        "old response ownership never transfers"
    );
}

#[test]
fn verified_checkpoint_removes_old_turn_state_only_from_the_new_upstream_copy() {
    let (_temp, path, store) = fixture();
    let parent = root(&store);
    let mut token_headers = HeaderMap::new();
    token_headers.insert(
        "x-codex-turn-state",
        HeaderValue::from_static("owned-turn-state"),
    );
    store.observe_headers(&parent.0, &token_headers).unwrap();
    let mut original = continuation("a-root", vec![message("user", "next")]);
    original["client_metadata"]["x-codex-turn-state"] = json!("owned-turn-state");
    original["client_metadata"]["parent_response_id"] = json!("a-root");
    let canonical = json!({"session_id":"root","thread_id":"root","x-codex-turn-state":"owned-turn-state","parent_response_id":"a-root"}).to_string();
    original["client_metadata"]["x-codex-turn-metadata"] = json!(canonical);
    token_headers.insert(
        "x-codex-turn-metadata",
        HeaderValue::from_str(&canonical).unwrap(),
    );
    let unchanged = original.clone();
    let mut context = store.inspect("caller", &token_headers, &original).unwrap();
    let candidate = store
        .configure_rebuild(&mut context, &token_headers, &original, true)
        .unwrap()
        .unwrap();
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert!(!candidate.headers.contains_key("x-codex-turn-state"));
    assert!(candidate.body.get("previous_response_id").is_none());
    assert!(candidate.body["client_metadata"]
        .get("x-codex-turn-state")
        .is_none());
    assert!(candidate.body["client_metadata"]
        .get("parent_response_id")
        .is_none());
    let metadata: Value = serde_json::from_str(
        candidate.body["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert!(metadata.get("x-codex-turn-state").is_none());
    assert!(metadata.get("parent_response_id").is_none());
    assert_eq!(original, unchanged);
    assert_eq!(token_headers["x-codex-turn-state"], "owned-turn-state");
    let owner: String = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT account FROM native_opaque_v1 WHERE kind='turn_state'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(owner, "a");
}

#[test]
fn opaque_reasoning_summary_agent_messages_and_compaction_are_not_decrypted() {
    for item in [
        json!({"type":"reasoning","summary":[{"type":"summary_text","text":"visible summary"}],"encrypted_content":"reasoning-cipher"}),
        json!({"type":"agent_message","author":"/parent","recipient":"/child","content":[{"type":"input_text","text":"visible envelope"},{"type":"encrypted_content","encrypted_content":"agent-cipher"}]}),
        json!({"type":"compaction","encrypted_content":"compaction-cipher","summary":"visible compacted summary"}),
        json!({"type":"function_call","namespace":"collaboration","name":"send_message","call_id":"call","arguments":"{\"message\":\"message-cipher\"}"}),
    ] {
        let (_temp, path, store) = fixture();
        let parent = root(&store);
        store
            .observe_event(
                &parent.0,
                &json!({"type":"response.output_item.done","item":item}),
            )
            .unwrap();
        let body = continuation("a-root", vec![item.clone(), message("user", "next")]);
        let unchanged = body.clone();
        let (context, candidate) = plan(&store, &body, true);
        assert!(candidate.is_none(), "{item}");
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
        let mut headers = HeaderMap::new();
        let mut upstream = body.clone();
        assert_eq!(
            store
                .rewrite(&context, "b", "owner-b", &mut headers, &mut upstream)
                .unwrap_err()
                .code,
            "upstream_state_owner_changed"
        );
        assert_eq!(upstream, unchanged);
        assert_eq!(active_row(&path).unwrap().0, "a");
    }
    let (_temp, path, store) = fixture();
    let parent = root(&store);
    complete(
        &store,
        &parent.0,
        "opaque-output",
        vec![
            json!({"type":"reasoning","encrypted_content":"output-cipher","summary":[{"text":"summary"}]}),
        ],
    );
    assert!(checkpoint(&path, "opaque-output").is_none());
    assert!(plan(
        &store,
        &continuation("opaque-output", vec![message("user", "next")]),
        true
    )
    .1
    .is_none());
}

#[test]
fn plaintext_tools_require_ordered_complete_pairs_and_checkpoint_delta_can_finish_a_call() {
    let call = json!({"type":"function_call","call_id":"call-1","name":"lookup","arguments":"{\"query\":\"plain\"}"});
    let output = json!({"type":"function_call_output","call_id":"call-1","output":"plain result"});
    let (_temp, path, store) = fixture();
    let root_body = request(vec![
        message("user", "lookup"),
        call.clone(),
        output.clone(),
    ]);
    assert!(plan(&store, &root_body, true).1.is_some());
    for items in [
        vec![call.clone()],
        vec![output.clone()],
        vec![output.clone(), call.clone()],
        vec![call.clone(), call.clone(), output.clone()],
    ] {
        assert!(plan(&store, &request(items), true).1.is_none());
    }
    let parent = prepare(
        &store,
        "caller",
        "a",
        &request(vec![message("user", "lookup")]),
        true,
    );
    complete(&store, &parent.0, "tool-pending", vec![call]);
    assert!(checkpoint(&path, "tool-pending").is_some());
    assert!(plan(
        &store,
        &continuation(
            "tool-pending",
            vec![output.clone(), message("user", "next")]
        ),
        true
    )
    .1
    .is_some());
    assert!(plan(
        &store,
        &continuation("tool-pending", vec![message("user", "next")]),
        true
    )
    .1
    .is_none());
    let mut wrong = output;
    wrong["call_id"] = json!("another-call");
    assert!(
        plan(&store, &continuation("tool-pending", vec![wrong]), true)
            .1
            .is_none()
    );
}

fn codex_app_host_output(name: &str) -> Value {
    json!({
        "type":"function_call_output", "id":format!("host-{name}"),
        "namespace":"codex_app", "name":name, "output":"host handoff result",
        "internal_chat_message_metadata_passthrough":{"keep":[1,null,{"text":"原值"}]},
        "extension":{"retained":true}
    })
}

#[test]
fn codex_app_handoffs_are_independent_of_ordered_function_and_custom_pairs() {
    let function =
        json!({"type":"function_call","name":"lookup","call_id":"function-1","arguments":"{}"});
    let custom =
        json!({"type":"custom_tool_call","name":"patch","call_id":"custom-1","input":"synthetic"});
    let function_result =
        json!({"type":"function_call_output","call_id":"function-1","output":"found"});
    let custom_result =
        json!({"type":"custom_tool_call_output","call_id":"custom-1","output":"done"});
    let host = codex_app_host_output("create_thread");
    let items = vec![
        host.clone(),
        function.clone(),
        codex_app_host_output("send_message_to_thread"),
        custom.clone(),
        function_result.clone(),
        host.clone(),
        custom_result,
    ];
    let before = items.clone();
    assert!(paired_tools(&items));
    assert_eq!(items, before);
    for incomplete in [
        vec![function.clone(), host.clone()],
        vec![host.clone(), function_result.clone()],
        vec![function_result.clone(), host.clone(), function.clone()],
        vec![
            function.clone(),
            host.clone(),
            function.clone(),
            function_result.clone(),
        ],
        vec![custom, host.clone()],
    ] {
        assert!(
            !paired_tools(&incomplete),
            "a host output cannot finish or hide a pending call: {incomplete:?}"
        );
    }
}

#[test]
fn codex_app_handoff_exemption_never_accepts_unknown_or_invalid_tool_results() {
    for (field, value) in [
        ("type", json!("custom_tool_call_output")),
        ("namespace", json!("other")),
        ("name", json!("list_threads")),
        ("name", json!("codex_app.create_thread")),
        ("call_id", Value::Null),
        ("call_id", json!("")),
        ("call_id", json!("unknown-call")),
        ("output", Value::Null),
        ("output", json!([{"type":"input_text","text":"result"}])),
    ] {
        let mut host = codex_app_host_output("create_thread");
        host[field] = value;
        assert!(!paired_tools(&[host.clone()]), "{host}");
    }
    let mut resource = codex_app_host_output("create_thread");
    resource["extension"]["file_id"] = json!("server-only-file");
    assert!(paired_tools(&[resource.clone()]));
    assert!(
        !replay_items(&[resource]),
        "pairing exemption must not bypass resource checks"
    );
}

#[test]
fn same_qualified_real_calls_make_idless_host_results_ambiguous() {
    for name in ["create_thread", "send_message_to_thread"] {
        let host = codex_app_host_output(name);
        for call in [
            json!({"type":"function_call","namespace":"codex_app","name":name,"call_id":"real-call","arguments":"{}"}),
            json!({"type":"function_call","name":format!("codex_app.{name}"),"call_id":"real-call","arguments":"{}"}),
        ] {
            let result =
                json!({"type":"function_call_output","call_id":"real-call","output":"real result"});
            assert!(paired_tools(&[call.clone(), result.clone()]));
            let mut explicit_host = host.clone();
            explicit_host["call_id"] = json!("real-call");
            assert!(paired_tools(&[call.clone(), explicit_host]));
            assert!(!paired_tools(&[host.clone(), call.clone(), result.clone()]));
            assert!(!paired_tools(&[call.clone(), host.clone(), result.clone()]));
            assert!(!paired_tools(&[call, result, host.clone()]));
        }
    }
}

#[test]
fn codex_app_host_records_survive_portable_checkpoint_and_cached_continuation_exactly() {
    let (_temp, path, store) = fixture();
    let input = vec![
        codex_app_host_output("create_thread"),
        message("user", "first"),
        codex_app_host_output("send_message_to_thread"),
    ];
    let original = request(input.clone());
    let before = original.clone();
    let headers = HeaderMap::new();
    let mut context = store
        .inspect_for_rebuild("caller", &headers, &original)
        .unwrap();
    let candidate = store
        .configure_rebuild(&mut context, &headers, &original, true)
        .unwrap()
        .unwrap();
    assert_eq!(candidate.body["input"], json!(input));
    assert_eq!(original, before);
    let mut upstream_headers = headers;
    let mut upstream = original;
    let binding = store
        .rewrite(
            &context,
            "a",
            &owner("a"),
            &mut upstream_headers,
            &mut upstream,
        )
        .unwrap();
    complete(
        &store,
        &binding,
        "host-a",
        vec![message("assistant", "answer")],
    );
    assert_eq!(
        &checkpoint(&path, "host-a").unwrap().2[..input.len()],
        input.as_slice()
    );
    assert_eq!(rows(&path, "native_rebuild_portable_v1"), 1);
    let next = continuation("host-a", vec![message("user", "next")]);
    let headers = HeaderMap::new();
    let mut context = store
        .inspect_for_rebuild("caller", &headers, &next)
        .unwrap();
    let candidate = store
        .configure_rebuild(&mut context, &headers, &next, true)
        .unwrap()
        .unwrap();
    let replay = candidate.body["input"].as_array().unwrap();
    assert_eq!(&replay[..input.len()], input.as_slice());
    assert!(candidate.body.get("previous_response_id").is_none());
    assert!(replay[0].get("call_id").is_none());
    assert!(replay[2].get("call_id").is_none());
}

#[test]
fn truncated_history_declines_but_large_requests_keep_owner_and_cache_budget() {
    let (_temp, path, store) = fixture();
    root(&store);
    let truncated = continuation(
        "a-root",
        vec![
            message("assistant", "truncated answer"),
            message("user", "next"),
        ],
    );
    assert!(plan(&store, &truncated, true).1.is_none());
    for field in ["conversation", "conversation_id"] {
        let mut body = request(vec![message("user", "incremental only")]);
        body[field] = json!("unobserved-upstream-conversation");
        assert!(
            plan(&store, &body, true).1.is_none(),
            "{field} cannot prove a root"
        );
    }
    let oversized = continuation(
        "a-root",
        vec![message("user", &"x".repeat(MAX_CHECKPOINT_BYTES))],
    );
    let (context, candidate) = plan(&store, &oversized, true);
    let candidate = candidate.expect("cache size must not cap an in-memory replay request");
    assert_eq!(
        candidate.body["input"].as_array().unwrap().last().unwrap(),
        &oversized["input"][0]
    );
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    let prepared = prepare(
        &store,
        "caller",
        "a",
        &request(vec![message("user", "short")]),
        true,
    );
    complete(
        &store,
        &prepared.0,
        "large-output",
        vec![message("assistant", &"x".repeat(MAX_CHECKPOINT_BYTES))],
    );
    assert!(checkpoint(&path, "large-output").is_none());
    assert!(plan(
        &store,
        &continuation("large-output", vec![message("user", "next")]),
        true
    )
    .1
    .is_some());
}

#[test]
fn rejected_stateless_rebuild_keeps_known_active_account_and_child_parent_scope() {
    let (_temp, path, store) = fixture();
    root(&store);
    let initial = active_row(&path).unwrap();
    for child in [false, true] {
        let mut body = request(vec![
            json!({"type":"compaction","summary":"incomplete visible summary"}),
        ]);
        if child {
            body["client_metadata"]["thread_id"] = json!("child");
            body["client_metadata"]["x-codex-parent-thread-id"] = json!("root");
        }
        let (context, candidate) = plan(&store, &body, true);
        assert!(candidate.is_none());
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
        assert!(!context.rebuild_enabled);
        assert_eq!(context.generation.as_deref(), Some(initial.2.as_str()));
        assert_eq!(active_row(&path).unwrap(), initial);
    }
    let body = request(vec![
        json!({"type":"compaction","summary":"incomplete visible summary"}),
    ]);
    let mut other = store
        .inspect("other-caller", &HeaderMap::new(), &body)
        .unwrap();
    assert!(store
        .configure_rebuild(&mut other, &HeaderMap::new(), &body, true)
        .unwrap()
        .is_none());
    assert!(other.state_account_id.is_none());
    assert!(other.generation.is_none());
}

#[test]
fn a_to_b_to_a_uses_new_mapping_but_old_response_restores_its_exact_generation() {
    let (_temp, path, store) = fixture();
    let initial = root(&store);
    let from_a = continuation("a-root", vec![message("user", "second")]);
    let next_b = rebuilt(&store, "b", &from_a);
    complete(
        &store,
        &next_b.0,
        "b-response",
        vec![message("assistant", "b answer")],
    );
    let from_b = continuation("b-response", vec![message("user", "third")]);
    let next_a = rebuilt(&store, "a", &from_b);
    complete(
        &store,
        &next_a.0,
        "a-new-response",
        vec![message("assistant", "new a answer")],
    );
    assert_ne!(initial.1["thread-id"], next_b.1["thread-id"]);
    assert_ne!(initial.1["thread-id"], next_a.1["thread-id"]);
    assert_ne!(next_b.1["thread-id"], next_a.1["thread-id"]);
    assert_eq!(active_row(&path).unwrap().3, 3);
    for (id, account, expected) in [
        ("a-root", "a", &initial),
        ("b-response", "b", &next_b),
        ("a-new-response", "a", &next_a),
    ] {
        let resumed = prepare(
            &store,
            "caller",
            account,
            &continuation(id, vec![message("user", "resume")]),
            false,
        );
        assert_eq!(resumed.1["thread-id"], expected.1["thread-id"], "{id}");
        assert_eq!(
            resumed.0.generation.as_ref().unwrap().id,
            expected.0.generation.as_ref().unwrap().id
        );
    }
}

#[test]
fn concurrent_rebuilds_and_late_old_owner_success_cannot_overwrite_active_cas_winner() {
    let (_temp, path, store) = fixture();
    root(&store);
    let late_a = prepare(
        &store,
        "caller",
        "a",
        &request(vec![message("user", "old in-flight")]),
        true,
    );
    let original = continuation("a-root", vec![message("user", "next")]);
    let next_b = rebuilt(&store, "b", &original);
    let next_c = rebuilt(&store, "c", &original);
    assert_ne!(
        next_b.0.generation.as_ref().unwrap().id,
        next_c.0.generation.as_ref().unwrap().id
    );
    complete(
        &store,
        &next_b.0,
        "winner-b",
        vec![message("assistant", "b")],
    );
    let winner = active_row(&path).unwrap();
    complete(&store, &late_a.0, "late-a", vec![message("assistant", "a")]);
    assert_eq!(active_row(&path).unwrap(), winner);
    complete(&store, &next_c.0, "late-c", vec![message("assistant", "c")]);
    assert_eq!(active_row(&path).unwrap(), winner);
    let late = checkpoint(&path, "late-c").unwrap();
    assert_eq!(late.0, "c");
    assert_eq!(late.1, next_c.0.generation.as_ref().unwrap().id);
    let resumed = prepare(
        &store,
        "caller",
        "c",
        &continuation("late-c", vec![message("user", "resume c branch")]),
        false,
    );
    assert_eq!(resumed.1["thread-id"], next_c.1["thread-id"]);
    assert_eq!(active_row(&path).unwrap(), winner);
}

#[test]
fn failed_or_cancelled_candidate_does_not_activate_or_create_a_success_checkpoint() {
    for status in ["failed", "incomplete", "cancelled"] {
        let (_temp, path, store) = fixture();
        root(&store);
        let before = active_row(&path).unwrap();
        let target = rebuilt(
            &store,
            "b",
            &continuation("a-root", vec![message("user", "next")]),
        );
        store
            .observe_event(
                &target.0,
                &json!({"type":format!("response.{status}"),"response":{
            "id":"failed-target","status":status,"output":[]}}),
            )
            .unwrap();
        assert_eq!(active_row(&path).unwrap(), before);
        assert!(checkpoint(&path, "failed-target").is_none());
        complete(
            &store,
            &target.0,
            "late-target",
            vec![message("assistant", "late")],
        );
        assert_eq!(active_row(&path).unwrap(), before);
        assert!(checkpoint(&path, "late-target").is_none());
        assert_affinity(&path, "a");
    }
}

#[test]
fn checkpoints_survive_restart_but_are_not_borrowed_by_another_caller_or_child_scope() {
    let (_temp, path, store) = fixture();
    root(&store);
    drop(store);
    let restarted = IdentityStore::open(path.clone()).unwrap();
    let original = continuation("a-root", vec![message("user", "next")]);
    assert!(plan(&restarted, &original, true).1.is_some());
    assert_eq!(
        restarted
            .inspect("other-caller", &HeaderMap::new(), &original)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
    let mut other_thread = original.clone();
    other_thread["client_metadata"]["thread_id"] = json!("unrelated");
    let (context, candidate) = plan(&restarted, &other_thread, true);
    assert!(candidate.is_none());
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    let mut child = original.clone();
    child["client_metadata"]["x-codex-parent-thread-id"] = json!("parent");
    assert!(plan(&restarted, &child, true).1.is_none());
    let plain_child = request(vec![message("user", "child request")]);
    let mut child = plain_child;
    child["client_metadata"]["thread_id"] = json!("child");
    child["client_metadata"]["x-codex-parent-thread-id"] = json!("root");
    let prepared = prepare(&restarted, "caller", "a", &child, true);
    assert_eq!(
        prepared.0.generation.as_ref().unwrap().id,
        active_row(&path).unwrap().2
    );
    assert!(
        prepared.0.generation.as_ref().unwrap().input.is_none(),
        "inheriting a parent mapping is not a full child history proof"
    );
}

#[test]
fn unregistered_complete_history_is_admitted_and_records_ownership_only_after_success() {
    let (_temp, path, store) = fixture();
    let original = request(vec![
        message("user", "complete historical task"),
        json!({"type":"reasoning","encrypted_content":"legacy-cipher"}),
    ]);
    let mut context = store
        .inspect_for_rebuild("caller", &HeaderMap::new(), &original)
        .unwrap();
    assert!(context.state_account_id.is_none());
    assert!(store
        .configure_rebuild(&mut context, &HeaderMap::new(), &original, true)
        .unwrap()
        .is_some());
    assert!(context.rebuild_enabled);
    let mut headers = HeaderMap::new();
    let mut body = original.clone();
    let binding = store
        .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
        .unwrap();
    assert!(binding.generation.is_some());
    assert_ne!(headers["thread-id"], "root");
    assert_eq!(body["input"], original["input"]);
    assert!(active_row(&path).is_none());
    assert!(affinity(&path).is_empty());
    assert_eq!(rows(&path, "native_opaque_v1"), 0);
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 0);
    complete(&store, &binding, "accepted-history", vec![]);
    assert_eq!(active_row(&path).unwrap().0, "a");
    assert_affinity(&path, "a");
    assert_eq!(
        checkpoint(&path, "accepted-history").unwrap().2,
        original["input"].as_array().unwrap().clone()
    );
    assert_eq!(rows(&path, "native_rebuild_blobs_v1"), 1);
    let accepted = store
        .inspect("caller", &HeaderMap::new(), &original)
        .unwrap();
    assert_eq!(accepted.state_account_id.as_deref(), Some("a"));
    assert_eq!(
        store
            .inspect_for_rebuild("other-caller", &HeaderMap::new(), &original)
            .unwrap_err()
            .code,
        "upstream_state_caller_mismatch"
    );
}

#[test]
fn failed_unregistered_history_never_activates_or_prevents_a_fresh_automatic_attempt() {
    for status in ["error", "failed", "incomplete", "cancelled"] {
        let (_temp, path, store) = fixture();
        let original = request(vec![
            message("user", "complete historical task"),
            json!({"type":"reasoning","encrypted_content":"legacy-cipher"}),
        ]);
        let mut headers = HeaderMap::new();
        let mut body = original.clone();
        let mut context = store
            .inspect_for_rebuild("caller", &headers, &body)
            .unwrap();
        assert!(store
            .configure_rebuild(&mut context, &headers, &body, true)
            .unwrap()
            .is_some());
        let binding = store
            .rewrite(&context, "a", "owner-a", &mut headers, &mut body)
            .unwrap();
        assert_eq!(body["input"], original["input"]);
        let failure = if status == "error" {
            json!({"type":"error","status":503,"error":{"message":"failed"}})
        } else {
            json!({"type":format!("response.{status}"),"response":{"id":"failed-history","status":status,"output":[]}})
        };
        store.observe_event(&binding, &failure).unwrap();
        complete(&store, &binding, "late-history", vec![]);
        assert!(active_row(&path).is_none(), "{status}");
        assert!(affinity(&path).is_empty(), "{status}");
        assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 0);
        assert_eq!(rows(&path, "native_rebuild_blobs_v1"), 0);
        let accepted_blobs: i64 = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(accepted_blobs, 0, "{status}");
        let mut retry_headers = HeaderMap::new();
        let mut retry_body = original.clone();
        let mut retry = store
            .inspect_for_rebuild("caller", &retry_headers, &retry_body)
            .unwrap();
        assert!(retry.state_account_id.is_none());
        assert!(store
            .configure_rebuild(&mut retry, &retry_headers, &retry_body, true)
            .unwrap()
            .is_some());
        store
            .rewrite(&retry, "b", "owner-b", &mut retry_headers, &mut retry_body)
            .unwrap();
        assert_eq!(retry_body["input"], original["input"]);
        assert!(active_row(&path).is_none());
    }
}

#[test]
fn disabling_rebuild_still_resumes_b_response_in_its_existing_generation() {
    let (_temp, path, store) = fixture();
    let initial = root(&store);
    let target = rebuilt(
        &store,
        "b",
        &continuation("a-root", vec![message("user", "move to b")]),
    );
    complete(
        &store,
        &target.0,
        "b-before-disable",
        vec![message("assistant", "b answer")],
    );
    let current = active_row(&path).unwrap();
    let original = continuation(
        "b-before-disable",
        vec![message("user", "continue with option off")],
    );
    let (context, candidate) = plan(&store, &original, false);
    assert!(candidate.is_none());
    assert!(!context.rebuild_enabled);
    assert_eq!(context.state_account_id.as_deref(), Some("b"));
    assert_eq!(
        context.generation.as_deref(),
        Some(target.0.generation.as_ref().unwrap().id.as_str())
    );
    let resumed = prepare(&store, "caller", "b", &original, false);
    assert_eq!(resumed.1["thread-id"], target.1["thread-id"]);
    assert_eq!(resumed.1["session-id"], target.1["session-id"]);
    assert_ne!(resumed.1["thread-id"], initial.1["thread-id"]);
    assert_eq!(resumed.2["previous_response_id"], "b-before-disable");
    complete(
        &store,
        &resumed.0,
        "b-after-disable",
        vec![message("assistant", "b followup")],
    );
    assert_eq!(active_row(&path).unwrap(), current);
    assert!(
        checkpoint(&path, "b-after-disable").is_none(),
        "disabled capture does not erase the existing generation"
    );
    assert_affinity(&path, "b");
    let next = prepare(
        &store,
        "caller",
        "b",
        &continuation("b-after-disable", vec![message("user", "another")]),
        false,
    );
    assert_eq!(next.1["thread-id"], target.1["thread-id"]);
}

#[test]
fn checkpoint_eviction_only_removes_optional_cache_not_old_response_owner_or_generation() {
    let (_temp, path, store) = fixture();
    for index in 0..129 {
        let mut body = request(vec![message("user", &format!("root {index}"))]);
        let thread = format!("thread-{index:03}");
        body["client_metadata"]["session_id"] = json!(thread);
        body["client_metadata"]["thread_id"] = json!(thread);
        let prepared = prepare(&store, "caller", "a", &body, true);
        complete(
            &store,
            &prepared.0,
            &format!("root-response-{index:03}"),
            vec![message("assistant", "answer")],
        );
    }
    assert_eq!(rows(&path, "native_rebuild_checkpoints_v1"), 128);
    assert!(checkpoint(&path, "root-response-000").is_none());
    assert!(checkpoint(&path, "root-response-128").is_some());
    let reference = digest("response", "root-response-000");
    let evidence:(String,String,String)=Connection::open(&path).unwrap().query_row(
        "SELECT o.account,o.owner,r.generation FROM native_opaque_v1 o JOIN native_rebuild_refs_v1 r ON r.caller=o.caller AND r.kind=o.kind AND r.digest=o.digest WHERE o.caller='caller' AND o.kind='response' AND o.digest=?1",
        [reference.digest],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!((evidence.0, evidence.1), ("a".into(), "owner-a".into()));
    let mut original = continuation("root-response-000", vec![message("user", "next")]);
    original["client_metadata"]["session_id"] = json!("thread-000");
    original["client_metadata"]["thread_id"] = json!("thread-000");
    let (context, candidate) = plan(&store, &original, true);
    assert!(
        candidate.is_some(),
        "cache eviction must not erase accepted history"
    );
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert_eq!(context.generation.as_deref(), Some(evidence.2.as_str()));
    let mut headers = HeaderMap::new();
    let mut body = original.clone();
    assert_eq!(
        store
            .rewrite(&context, "b", "owner-b", &mut headers, &mut body)
            .unwrap_err()
            .code,
        "upstream_state_owner_changed"
    );
    assert_eq!(body, original);
}

#[test]
fn checkpoint_total_byte_budget_evicts_optional_history_without_erasing_ownership() {
    let (_temp, path, store) = fixture();
    // Ten valid checkpoints fit individually below 8 MiB but exceed the shared
    // 64 MiB budget. Exercise actual serialized history, not synthetic SQL sizes.
    let payload = "x".repeat(7 * 1024 * 1024);
    for index in 0..10 {
        let mut body = request(vec![message("user", &payload)]);
        let thread = format!("large-thread-{index}");
        body["client_metadata"]["session_id"] = json!(thread);
        body["client_metadata"]["thread_id"] = json!(thread);
        let prepared = prepare(&store, "caller", "a", &body, true);
        complete(
            &store,
            &prepared.0,
            &format!("large-response-{index}"),
            vec![],
        );
    }
    assert!(rows(&path, "native_rebuild_checkpoints_v1") < 10);
    assert!(checkpoint(&path, "large-response-0").is_none());
    assert!(checkpoint(&path, "large-response-9").is_some());
    let connection = Connection::open(&path).unwrap();
    let total_bytes: i64 = connection
        .query_row(
            "SELECT COALESCE(SUM(bytes),0) FROM native_rebuild_checkpoints_v1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(total_bytes <= MAX_CACHE_BYTES);
    let mut original = continuation("large-response-0", vec![message("user", "next")]);
    original["client_metadata"]["session_id"] = json!("large-thread-0");
    original["client_metadata"]["thread_id"] = json!("large-thread-0");
    let (context, candidate) = plan(&store, &original, true);
    assert!(
        candidate.is_some(),
        "durable replay survives the hot-cache byte budget"
    );
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert_eq!(original["previous_response_id"], "large-response-0");
}

#[test]
fn root_parameters_must_be_explicit_in_delta_requests_and_current_values_are_preserved() {
    let (_temp, path, store) = fixture();
    let mut original = request(vec![message("user", "root prompt")]);
    original["instructions"] = json!("root instruction");
    original["tools"] = json!([{"type":"function","name":"lookup","parameters":{"type":"object","properties":{"query":{"type":"string"}}}}]);
    original["tool_choice"] = json!("auto");
    original["parallel_tool_calls"] = json!(true);
    original["text"] = json!({"format":{"type":"text"}});
    original["reasoning"] = json!({"effort":"low"});
    let parent = prepare(&store, "caller", "a", &original, true);
    complete(
        &store,
        &parent.0,
        "parameterized-root",
        vec![message("assistant", "root answer")],
    );
    let missing = continuation("parameterized-root", vec![message("user", "delta")]);
    let (context, candidate) = plan(&store, &missing, true);
    assert!(candidate.is_none());
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    let keys = [
        "instructions",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "text",
        "reasoning",
    ];
    let mut explicit = missing.clone();
    for key in keys {
        explicit[key] = original[key].clone();
    }
    explicit["instructions"] = json!("current explicit instruction");
    explicit["reasoning"] = json!({"effort":"medium"});
    for omitted in keys {
        let mut body = explicit.clone();
        body.as_object_mut().unwrap().remove(omitted);
        let (context, candidate) = plan(&store, &body, true);
        assert!(
            candidate.is_none(),
            "omitted {omitted} cannot inherit root parameters"
        );
        assert_eq!(context.state_account_id.as_deref(), Some("a"));
    }
    let candidate = plan(&store, &explicit, true).1.unwrap();
    for key in keys {
        assert_eq!(candidate.body[key], explicit[key], "{key}");
    }
    assert_eq!(
        candidate.body["input"],
        json!([
            message("user", "root prompt"),
            message("assistant", "root answer"),
            message("user", "delta")
        ])
    );
    // A pre-migration row with no parameter manifest cannot attest omitted or
    // inherited instructions even when its old transcript is still readable.
    Connection::open(&path).unwrap().execute("UPDATE native_rebuild_checkpoints_v1 SET parameters=NULL WHERE response='parameterized-root'",[]).unwrap();
    let (context, candidate) = plan(&store, &explicit, true);
    assert!(candidate.is_none());
    assert_eq!(context.state_account_id.as_deref(), Some("a"));
    assert!(checkpoint(&path, "parameterized-root").is_some());
}

#[test]
fn streamed_output_missing_or_changed_at_terminal_cannot_checkpoint_or_activate_target() {
    for mode in ["empty", "different", "missing"] {
        let (_temp, path, store) = fixture();
        root(&store);
        let before = active_row(&path).unwrap();
        assert_affinity(&path, "a");
        let target = rebuilt(
            &store,
            "b",
            &continuation("a-root", vec![message("user", "switch")]),
        );
        // Preparing b cannot publish its affinity before successful activation.
        assert_affinity(&path, "a");
        let sent = message("assistant", "already delivered output");
        store
            .observe_event(
                &target.0,
                &json!({"type":"response.output_item.done","output_index":0,"item":sent}),
            )
            .unwrap();
        let mut terminal = json!({"type":"response.completed","response":{"id":format!("bad-terminal-{mode}"),"status":"completed","output":[]}});
        if mode == "different" {
            terminal["response"]["output"] = json!([message("assistant", "different output")]);
        }
        if mode == "missing" {
            terminal["response"]
                .as_object_mut()
                .unwrap()
                .remove("output");
        }
        store.observe_event(&target.0, &terminal).unwrap();
        assert!(checkpoint(&path, &format!("bad-terminal-{mode}")).is_none());
        assert_eq!(active_row(&path).unwrap(), before, "{mode}");
        assert_affinity(&path, "a");
    }
}

#[test]
fn preparation_and_failed_b_keep_a_affinity_and_late_cas_loser_cannot_restore_it() {
    let (_temp, path, store) = fixture();
    root(&store);
    let initial = active_row(&path).unwrap();
    assert_affinity(&path, "a");
    let failed = rebuilt(
        &store,
        "b",
        &continuation("a-root", vec![message("user", "failed b")]),
    );
    assert_affinity(&path, "a");
    store.observe_event(&failed.0,&json!({"type":"response.failed","response":{"id":"failed-b","status":"failed","output":[]}})).unwrap();
    assert_eq!(active_row(&path).unwrap(), initial);
    assert_affinity(&path, "a");
    let stale_original = continuation("a-root", vec![message("user", "stale a rebuild")]);
    let frozen = plan(&store, &stale_original, true).1.unwrap();
    let winner = rebuilt(
        &store,
        "b",
        &continuation("a-root", vec![message("user", "winning b")]),
    );
    let late_existing_a = prepare(
        &store,
        "caller",
        "a",
        &continuation("a-root", vec![message("user", "old a")]),
        false,
    );
    assert_affinity(&path, "a");
    complete(
        &store,
        &winner.0,
        "winning-b",
        vec![message("assistant", "b won")],
    );
    let active_b = active_row(&path).unwrap();
    assert_affinity(&path, "b");
    let late_new_a = rebuilt_plan(&store, "a", frozen);
    assert_affinity(&path, "b");
    complete(
        &store,
        &late_new_a.0,
        "late-new-a",
        vec![message("assistant", "a branch")],
    );
    assert_eq!(active_row(&path).unwrap(), active_b);
    assert_affinity(&path, "b");
    complete(
        &store,
        &late_existing_a.0,
        "late-existing-a",
        vec![message("assistant", "old a branch")],
    );
    assert_eq!(active_row(&path).unwrap(), active_b);
    assert_affinity(&path, "b");
    let stored = checkpoint(&path, "late-new-a").unwrap();
    assert_eq!(stored.0, "a");
    assert_eq!(stored.1, late_new_a.0.generation.as_ref().unwrap().id);
}

#[test]
fn error_or_cancel_fences_late_completion_and_plain_tool_inventory_can_be_cached() {
    for kind in ["error", "response.error", "cancel", "invalid-terminal"] {
        let (_temp, path, store) = fixture();
        let mut original = request(vec![
            json!({"type":"additional_tools","role":"developer","tools":[{"name":"lookup","parameters":{"type":"object"}}]}),
            message("user", "first"),
        ]);
        original["tools"] =
            json!([{"type":"function","name":"lookup","parameters":{"type":"object"}}]);
        let first = prepare(&store, "caller", "a", &original, true);
        complete(
            &store,
            &first.0,
            "inventory-a",
            vec![message("assistant", "a answer")],
        );
        assert!(checkpoint(&path, "inventory-a").is_some());
        let before = active_row(&path).unwrap();
        let mut next = continuation("inventory-a", vec![message("user", "next")]);
        next["tools"] = original["tools"].clone();
        let target = rebuilt(&store, "b", &next);
        match kind {
            "cancel" => target.0.abort_rebuild(),
            "invalid-terminal" => {
                store.observe_event(&target.0, &json!({"type":"response.completed","response":{"status":"completed","output":[]}})).unwrap();
            }
            kind => {
                store
                    .observe_event(
                        &target.0,
                        &json!({"type":kind,"error":{"code":"synthetic_error"}}),
                    )
                    .unwrap();
            }
        }
        complete(
            &store,
            &target.0,
            "late-b",
            vec![message("assistant", "late")],
        );
        assert_eq!(active_row(&path).unwrap(), before);
        assert!(checkpoint(&path, "late-b").is_none());
        assert_affinity(&path, "a");
    }
}

#[test]
fn conflicting_duplicate_output_index_cannot_replace_delivered_manifest() {
    for conflicting in [false, true] {
        let (_temp, path, store) = fixture();
        root(&store);
        let before = active_row(&path).unwrap();
        let target = rebuilt(
            &store,
            "b",
            &continuation("a-root", vec![message("user", "next")]),
        );
        let first = message("assistant", "delivered first");
        let second = if conflicting {
            message("assistant", "different delivered second")
        } else {
            first.clone()
        };
        for item in [first, second.clone()] {
            store
                .observe_event(
                    &target.0,
                    &json!({"type":"response.output_item.done","output_index":0,"item":item}),
                )
                .unwrap();
        }
        complete(&store, &target.0, "duplicate-target", vec![second]);
        if conflicting {
            assert_eq!(active_row(&path).unwrap(), before);
            assert!(checkpoint(&path, "duplicate-target").is_none());
            assert_affinity(&path, "a");
        } else {
            assert_eq!(active_row(&path).unwrap().0, "b");
            assert!(checkpoint(&path, "duplicate-target").is_some());
            assert_affinity(&path, "b");
        }
    }
}

include!("identity_checkpoint_tests.rs");
