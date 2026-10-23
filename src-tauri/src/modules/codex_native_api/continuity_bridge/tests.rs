use super::{protocol::*, session::Core, *};
use serde_json::{json, Value};

#[test]
fn legacy_session_header_is_stable_through_shared_go_identity_prepare_and_bind() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let project = |request_id: &str, account: &str| {
        let mut original = request(request_id);
        original
            .body
            .as_object_mut()
            .unwrap()
            .remove("client_metadata");
        original
            .headers
            .insert("session_id".into(), vec!["pi-session".into()]);
        let prepared = core.prepare(original).unwrap();
        let bound = core.bind(binding(&prepared, "attempt", account)).unwrap();
        assert!(!bound.headers.contains_key("session_id"));
        let session = &bound.headers["session-id"][0];
        assert_ne!(session, "pi-session");
        assert_eq!(
            bound.body["client_metadata"]["session_id"],
            session.as_str()
        );
        session.clone()
    };
    let first = project("request-1", "account-a");
    assert_eq!(first, project("request-2", "account-a"));
    assert_ne!(first, project("request-3", "account-b"));
}

fn config(path: PathBuf) -> Config {
    Config {
        identity_path: path,
        token: "private-test-token-never-an-upstream-credential".into(),
        owners: HashMap::from([
            ("account-a".into(), "owner-a".into()),
            ("account-b".into(), "owner-b".into()),
        ]),
        caller_accounts: HashMap::from([
            (
                "native:primary".into(),
                vec!["account-a".into(), "account-b".into()],
            ),
            ("restricted".into(), vec!["account-b".into()]),
        ]),
    }
}

fn request(id: &str) -> PrepareRequest {
    PrepareRequest {
        request_id: id.into(),
        caller: "native:primary".into(),
        headers: Headers::new(),
        compact: false,
        allow_rebuild: true,
        body: json!({"model":"gpt-test","input":[{"role":"user","content":"first"}],"client_metadata":{"session_id":"root","thread_id":"root"}}),
    }
}

fn binding(prepared: &PrepareResponse, attempt: &str, account: &str) -> BindRequest {
    BindRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: attempt.into(),
        account_id: account.into(),
        owner_id: None,
        expand_http_previous: false,
        headers: prepared.headers.clone(),
        body: prepared.body.clone(),
    }
}

fn observation(
    prepared: &PrepareResponse,
    attempt: &str,
    sequence: u64,
    events: Vec<Value>,
) -> ObserveRequest {
    ObserveRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: attempt.into(),
        sequence,
        headers: None,
        events,
    }
}

fn terminal(id: &str, output: Vec<Value>) -> Value {
    json!({"type":"response.completed","response":{"id":id,"status":"completed","output":output}})
}

fn finish_request(
    prepared: &PrepareResponse,
    attempt: &str,
    sequence: u64,
    status: FinishStatus,
) -> FinishRequest {
    FinishRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: attempt.into(),
        sequence,
        status,
        error_code: None,
    }
}

fn count(path: &std::path::Path, table: &str) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn active_account(path: &std::path::Path) -> Option<String> {
    use rusqlite::OptionalExtension;
    rusqlite::Connection::open(path).unwrap().query_row("SELECT account FROM native_rebuild_active_v1 WHERE caller='native:primary' AND thread='root'", [], |row| row.get(0)).optional().unwrap()
}

#[test]
fn prepare_and_bind_are_idempotent_and_scope_cannot_be_expanded() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let original = request("request-1");
    let prepared = core.prepare(original.clone()).unwrap();
    assert_eq!(
        prepared.ticket,
        core.prepare(original.clone()).unwrap().ticket
    );
    let mut changed = original;
    changed.body["input"][0]["content"] = json!("different");
    assert_eq!(
        core.prepare(changed).err().unwrap().code,
        "continuity_request_conflict"
    );
    let bound = core
        .bind(binding(&prepared, "attempt-1", "account-a"))
        .unwrap();
    assert_eq!(
        bound.body,
        core.bind(binding(&prepared, "attempt-1", "account-a"))
            .unwrap()
            .body
    );
    let mut conflicting = binding(&prepared, "attempt-1", "account-b");
    assert_eq!(
        core.bind(conflicting.clone()).err().unwrap().code,
        "continuity_attempt_conflict"
    );
    conflicting.attempt_id = "attempt-2".into();
    conflicting.owner_id = Some("forged-owner".into());
    assert_eq!(
        core.bind(conflicting).err().unwrap().code,
        "continuity_account_scope"
    );

    let mut scoped = request("restricted-request");
    scoped.caller = "restricted".into();
    let scoped = core.prepare(scoped).unwrap();
    assert_eq!(
        core.bind(binding(&scoped, "try-a", "account-a"))
            .err()
            .unwrap()
            .code,
        "continuity_account_scope"
    );
    let mut unknown = request("unknown-caller");
    unknown.caller = "unknown".into();
    assert_eq!(
        core.prepare(unknown).err().unwrap().code,
        "continuity_caller_scope"
    );
}

#[test]
fn retries_fence_old_attempts_and_observations_are_ordered_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let prepared = core.prepare(request("request")).unwrap();
    core.bind(binding(&prepared, "attempt-a", "account-a"))
        .unwrap();
    core.bind(binding(&prepared, "attempt-b", "account-b"))
        .unwrap();
    assert_eq!(
        core.observe(observation(
            &prepared,
            "attempt-a",
            1,
            vec![terminal("late", vec![])]
        ))
        .err()
        .unwrap()
        .code,
        "stale_continuity_attempt"
    );
    assert_eq!(
        core.finish(finish_request(
            &prepared,
            "attempt-a",
            1,
            FinishStatus::Failed
        ))
        .err()
        .unwrap()
        .code,
        "stale_continuity_attempt"
    );
    assert_eq!(
        core.observe(observation(&prepared, "attempt-b", 2, vec![]))
            .err()
            .unwrap()
            .code,
        "continuity_sequence_mismatch"
    );
    let event = observation(
        &prepared,
        "attempt-b",
        1,
        vec![terminal("response-b", vec![])],
    );
    assert_eq!(core.observe(event.clone()).unwrap().next_sequence, 2);
    assert!(core.observe(event).unwrap().terminal);
    assert_eq!(
        core.observe(observation(&prepared, "attempt-b", 1, vec![]))
            .err()
            .unwrap()
            .code,
        "continuity_sequence_conflict"
    );
    let finished = finish_request(&prepared, "attempt-b", 2, FinishStatus::Succeeded);
    core.finish(finished.clone()).unwrap();
    core.finish(finished).unwrap();
}

#[test]
fn complete_history_rebuild_and_checkpoint_survive_engine_and_process_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let prepared = core.prepare(request("first")).unwrap();
    core.bind(binding(&prepared, "attempt-a", "account-a"))
        .unwrap();
    let reply = json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"first reply"}]});
    core.observe(observation(
        &prepared,
        "attempt-a",
        1,
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":reply}),
            terminal("response-a", vec![reply]),
        ],
    ))
    .unwrap();
    assert_eq!(active_account(&path).as_deref(), Some("account-a"));
    drop(core);

    // A fresh bridge uses the same SQLite as a native engine; no JSONL handoff.
    let core = Core::open(config(path.clone())).unwrap();
    let mut next = request("second");
    next.body["previous_response_id"] = json!("response-a");
    next.body["input"] = json!([{"role":"user","content":"continue"}]);
    let prepared = core.prepare(next).unwrap();
    assert_eq!(prepared.preferred_account_id.as_deref(), Some("account-a"));
    assert!(prepared.self_contained);
    let bound = core
        .bind(binding(&prepared, "attempt-b", "account-b"))
        .unwrap();
    assert!(bound.rebuild_applied);
    assert!(bound.body.get("previous_response_id").is_none());
    assert_eq!(bound.body["input"].as_array().unwrap().len(), 3);
    assert_eq!(active_account(&path).as_deref(), Some("account-a"));
    core.observe(observation(
        &prepared,
        "attempt-b",
        1,
        vec![terminal("response-b", vec![])],
    ))
    .unwrap();
    assert_eq!(active_account(&path).as_deref(), Some("account-b"));
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 2);
}

#[test]
fn raw_terminal_evidence_cannot_be_replaced_by_patched_or_incomplete_output() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let prepared = core.prepare(request("first")).unwrap();
    core.bind(binding(&prepared, "attempt-a", "account-a"))
        .unwrap();
    core.observe(observation(
        &prepared,
        "attempt-a",
        1,
        vec![terminal("response-a", vec![])],
    ))
    .unwrap();
    let mut next = request("second");
    next.body["previous_response_id"] = json!("response-a");
    let prepared = core.prepare(next).unwrap();
    core.bind(binding(&prepared, "attempt-b", "account-b"))
        .unwrap();
    let output = json!({"type":"message","role":"assistant","content":"actual streamed item"});
    core.observe(observation(
        &prepared,
        "attempt-b",
        1,
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":output}),
            terminal("response-b", vec![]),
        ],
    ))
    .unwrap();
    assert_eq!(active_account(&path).as_deref(), Some("account-a"));
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 1);
}

#[test]
fn cancellation_failure_and_false_success_cannot_activate_a_candidate() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let prepared = core.prepare(request("cancelled")).unwrap();
    core.bind(binding(&prepared, "cancelled", "account-b"))
        .unwrap();
    core.finish(finish_request(
        &prepared,
        "cancelled",
        1,
        FinishStatus::Cancelled,
    ))
    .unwrap();
    assert_eq!(
        core.observe(observation(
            &prepared,
            "cancelled",
            1,
            vec![terminal("late", vec![])]
        ))
        .err()
        .unwrap()
        .code,
        "stale_continuity_attempt"
    );
    assert!(active_account(&path).is_none());

    let prepared = core.prepare(request("false-success")).unwrap();
    core.bind(binding(&prepared, "false", "account-b")).unwrap();
    assert_eq!(
        core.finish(finish_request(
            &prepared,
            "false",
            1,
            FinishStatus::Succeeded
        ))
        .err()
        .unwrap()
        .code,
        "continuity_missing_terminal"
    );
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);

    let prepared = core.prepare(request("failure-first")).unwrap();
    core.bind(binding(&prepared, "failure", "account-a"))
        .unwrap();
    core.observe(observation(
        &prepared,
        "failure",
        1,
        vec![
            json!({"type":"error","error":{"code":"upstream_failed"}}),
            terminal("late-success", vec![]),
        ],
    ))
    .unwrap();
    assert!(active_account(&path).is_none());
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
}

#[test]
fn automatic_unknown_history_retries_without_a_grant_and_failure_never_commits_input_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let mut original = request("automatic-unknown");
    original.body["input"] = json!([
        {"role":"user","content":"before"},
        {"type":"reasoning","encrypted_content":"legacy-agent-cipher"},
        {"role":"user","content":"continue"}
    ]);
    let prepared = core.prepare(original.clone()).unwrap();
    assert!(prepared.self_contained);
    assert_eq!(prepared.ticket, core.prepare(original).unwrap().ticket);
    let first = binding(&prepared, "attempt-a", "account-a");
    let bound = core.bind(first.clone()).unwrap();
    core.bind(first).unwrap();
    assert_eq!(bound.body["input"], prepared.body["input"]);
    core.finish(finish_request(
        &prepared,
        "attempt-a",
        1,
        FinishStatus::Failed,
    ))
    .unwrap();
    let second = core
        .bind(binding(&prepared, "attempt-b", "account-b"))
        .unwrap();
    assert_eq!(second.body["input"], prepared.body["input"]);
    core.finish(finish_request(
        &prepared,
        "attempt-b",
        1,
        FinishStatus::Failed,
    ))
    .unwrap();
    assert_eq!(count(&path, "native_opaque_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_active_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
    assert!(!path.with_extension("recovery").exists());
}

#[test]
fn automatic_completed_history_records_cipher_before_ack_and_rejects_another_caller() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let prepared = core.prepare(request("recovery")).unwrap();
    core.bind(binding(&prepared, "once", "account-a")).unwrap();
    let call = json!({"type":"function_call","namespace":"collaboration","name":"spawn_agent","call_id":"call-1","arguments":"{\"message\":\"child-cipher\"}"});
    core.observe(observation(
        &prepared,
        "once",
        1,
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":call}),
            terminal("recovered", vec![call]),
        ],
    ))
    .unwrap();
    assert!(count(&path, "native_opaque_v1") >= 3);
    let mut other = request("foreign-cipher");
    other.caller = "restricted".into();
    other.body["input"] = json!([{"type":"agent_message","author":"root","recipient":"child","content":[{"type":"encrypted_content","encrypted_content":"child-cipher"}]}]);
    assert!(
        core.prepare(other).is_err(),
        "another caller cannot borrow observed ciphertext"
    );
}

#[test]
fn bind_restores_client_identity_headers_without_replacing_fresh_transport_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let mut original = request("headers");
    original
        .headers
        .insert("thread-id".into(), vec!["root".into()]);
    original
        .headers
        .insert("session-id".into(), vec!["client-cache-alias".into()]);
    let prepared = core.prepare(original).unwrap();
    let mut bind = binding(&prepared, "attempt", "account-a");
    bind.headers = Headers::from([(
        "authorization".into(),
        vec!["Bearer fresh-credential".into()],
    )]);
    let bound = core.bind(bind).unwrap();
    assert_eq!(
        bound.headers["authorization"],
        vec!["Bearer fresh-credential"]
    );
    assert!(bound.headers.contains_key("thread-id"));
    assert!(bound.headers.contains_key("session-id"));
    assert_ne!(bound.headers["thread-id"], vec!["root"]);
}

#[test]
fn compact_observes_the_original_compaction_object_without_inventing_a_completed_response() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let mut original = request("compact");
    original.compact = true;
    let prepared = core.prepare(original).unwrap();
    assert!(!prepared.self_contained);
    core.bind(binding(&prepared, "compact-a", "account-a"))
        .unwrap();
    core.observe(observation(&prepared, "compact-a", 1, vec![json!({"id":"compact-response","object":"response.compaction","output":[{"type":"compaction","encrypted_content":"compact-cipher"}]})])).unwrap();
    core.finish(finish_request(
        &prepared,
        "compact-a",
        2,
        FinishStatus::Succeeded,
    ))
    .unwrap();
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_active_v1"), 0);
    assert!(count(&path, "native_opaque_v1") >= 2);
}

#[test]
fn ordinary_failed_and_cancelled_opening_attempts_keep_go_retries_and_fence_late_callbacks() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let prepared = core.prepare(request("retry")).unwrap();
    core.bind(binding(&prepared, "http-429", "account-a"))
        .unwrap();
    core.observe(observation(&prepared, "http-429", 1, vec![json!({"type":"response.created","response":{"id":"opening-only","status":"in_progress"}})])).unwrap();
    let failure = finish_request(&prepared, "http-429", 2, FinishStatus::Failed);
    core.finish(failure.clone()).unwrap();
    core.bind(binding(&prepared, "ws-timeout", "account-b"))
        .unwrap();
    core.finish(failure).unwrap();
    assert_eq!(
        core.observe(observation(
            &prepared,
            "http-429",
            2,
            vec![terminal("late", vec![])]
        ))
        .err()
        .unwrap()
        .code,
        "stale_continuity_attempt"
    );
    core.finish(finish_request(
        &prepared,
        "ws-timeout",
        1,
        FinishStatus::Cancelled,
    ))
    .unwrap();
    core.bind(binding(&prepared, "http-fallback", "account-b"))
        .unwrap();
    core.observe(observation(
        &prepared,
        "http-fallback",
        1,
        vec![terminal("successful-fallback", vec![])],
    ))
    .unwrap();
    core.finish(finish_request(
        &prepared,
        "http-fallback",
        2,
        FinishStatus::Succeeded,
    ))
    .unwrap();
}

#[test]
fn unbound_automatic_history_can_be_failed_or_released_without_publishing_ownership() {
    for explicit in [true, false] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity.sqlite");
        let core = Core::open(config(path.clone())).unwrap();
        let mut original = request("unbound");
        original.body["input"] =
            json!([{"type":"reasoning","encrypted_content":"not-yet-accepted"}]);
        let prepared = core.prepare(original).unwrap();
        if explicit {
            core.finish(finish_request(&prepared, "", 1, FinishStatus::Failed))
                .unwrap();
        } else {
            core.close();
        }
        assert_eq!(count(&path, "native_opaque_v1"), 0);
        assert_eq!(count(&path, "native_rebuild_active_v1"), 0);
        assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
    }
}

#[test]
fn rejected_observation_can_be_finished_with_the_last_acknowledged_sequence() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let prepared = core.prepare(request("invalid-terminal")).unwrap();
    core.bind(binding(&prepared, "invalid-a", "account-a"))
        .unwrap();
    let invalid = observation(
        &prepared,
        "invalid-a",
        1,
        vec![
            json!({"type":"response.completed","response":{"id":42,"status":"completed","output":[]}}),
        ],
    );
    assert_eq!(
        core.observe(invalid.clone()).err().unwrap().code,
        "invalid_protocol_identity"
    );
    assert_eq!(
        core.observe(invalid).err().unwrap().code,
        "invalid_protocol_identity"
    );
    core.finish(finish_request(
        &prepared,
        "invalid-a",
        1,
        FinishStatus::Failed,
    ))
    .unwrap();
    core.bind(binding(&prepared, "valid-b", "account-b"))
        .unwrap();
}

#[test]
fn ticket_capacity_failure_cannot_publish_unverified_automatic_history() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let mut full = false;
    for index in 0..2048 {
        match core.prepare(request(&format!("capacity-{index}"))) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.code, "continuity_capacity");
                full = true;
                break;
            }
        }
    }
    assert!(full);
    let mut original = request("over-capacity");
    original.body["input"] =
        json!([{"type":"reasoning","encrypted_content":"unaccepted-capacity-cipher"}]);
    assert_eq!(
        core.prepare(original).err().unwrap().code,
        "continuity_capacity"
    );
    assert_eq!(count(&path, "native_opaque_v1"), 0);
    assert!(!path.with_extension("recovery").exists());
}

#[test]
fn concurrent_prepare_retries_share_one_automatic_history_ticket() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Arc::new(Core::open(config(path.clone())).unwrap());
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let tickets = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let core = core.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let prepared = core.prepare(request("one-claim")).unwrap();
                    prepared.ticket
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(tickets.iter().all(|ticket| ticket == &tickets[0]));
    core.close();
}

#[test]
fn more_than_a_thousand_failed_logical_requests_release_capacity_immediately() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    for index in 0..1100 {
        let prepared = core
            .prepare(request(&format!("released-failure-{index}")))
            .unwrap();
        core.bind(binding(&prepared, "attempt", "account-a"))
            .unwrap();
        core.finish(finish_request(
            &prepared,
            "attempt",
            1,
            FinishStatus::Failed,
        ))
        .unwrap();
        let release = ReleaseRequest {
            ticket: prepared.ticket.clone(),
        };
        core.release(release.clone()).unwrap();
        core.release(release).unwrap();
    }
    core.release(ReleaseRequest {
        ticket: "already-evicted-ticket".into(),
    })
    .unwrap();
    core.prepare(request("still-available")).unwrap();
}

#[test]
fn an_active_stream_survives_idle_expiry_and_only_explicit_release_closes_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let active = core.prepare(request("active-stream")).unwrap();
    core.bind(binding(&active, "attempt", "account-a")).unwrap();
    core.elapse_for_test(3 * 60 * 60);
    core.prepare(request("trigger-cleanup")).unwrap();
    core.observe(observation(&active, "attempt", 1, vec![json!({"type":"response.created","response":{"id":"still-active","status":"in_progress"}})])).unwrap();
    core.release(ReleaseRequest {
        ticket: active.ticket.clone(),
    })
    .unwrap();
    assert_eq!(
        core.observe(observation(
            &active,
            "attempt",
            2,
            vec![terminal("late", vec![])]
        ))
        .err()
        .unwrap()
        .code,
        "continuity_attempt_finished"
    );
}

#[test]
fn release_of_unbound_automatic_history_does_not_publish_unverified_state() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let mut original = request("released-history");
    original.body["input"] = json!([{"type":"compaction","encrypted_content":"old-compaction"}]);
    let prepared = core.prepare(original).unwrap();
    core.release(ReleaseRequest {
        ticket: prepared.ticket.clone(),
    })
    .unwrap();
    assert_eq!(count(&path, "native_opaque_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_active_v1"), 0);
}

#[test]
fn large_canonical_bodies_do_not_hit_an_artificial_twenty_one_megabyte_budget_limit() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let mut large = request("large-body");
    large.body["input"][0]["content"] = Value::String("x".repeat(24 * 1024 * 1024));
    let prepared = core.prepare(large).unwrap();
    assert!(prepared.self_contained, "complete input is not limited by checkpoint cache admission");
    let mapped = core
        .bind(binding(&prepared, "large-a", "account-a"))
        .unwrap();
    assert_eq!(
        mapped.body["input"][0]["content"].as_str().unwrap().len(),
        24 * 1024 * 1024
    );
    core.release(ReleaseRequest {
        ticket: prepared.ticket,
    })
    .unwrap();
}

#[test]
fn planning_may_probe_saved_defaults_but_bind_requires_the_real_transport_parameters() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let first = core.prepare(request("defaults-first")).unwrap();
    let mut first_bind = binding(&first, "first-a", "account-a");
    first_bind.body["instructions"] = json!("");
    first_bind.body["tools"] = json!([{"type":"image_generation"}]);
    core.bind(first_bind).unwrap();
    let output = json!({"type":"message","role":"assistant","content":"first reply"});
    core.observe(observation(
        &first,
        "first-a",
        1,
        vec![terminal("defaults-response", vec![output.clone()])],
    ))
    .unwrap();
    core.finish(finish_request(
        &first,
        "first-a",
        2,
        FinishStatus::Succeeded,
    ))
    .unwrap();

    let mut next = request("defaults-next");
    next.body["previous_response_id"] = json!("defaults-response");
    next.body["input"] =
        json!([{"role":"user","content":"first"},output,{"role":"user","content":"continue"}]);
    let prepared = core.prepare(next).unwrap();
    assert!(prepared.self_contained);
    assert_eq!(prepared.preferred_account_id.as_deref(), Some("account-a"));
    assert!(prepared.body.get("instructions").is_none());
    assert!(prepared.body.get("tools").is_none());
    assert_eq!(
        core.bind(binding(&prepared, "without-defaults", "account-b"))
            .err()
            .unwrap()
            .code,
        "continuation_owner_mismatch"
    );
    core.finish(finish_request(
        &prepared,
        "without-defaults",
        1,
        FinishStatus::Failed,
    ))
    .unwrap();
    assert_eq!(active_account(&path).as_deref(), Some("account-a"));
    let mut real = binding(&prepared, "with-real-defaults", "account-b");
    real.body["instructions"] = json!("actual new transport instruction");
    real.body["tools"] = json!([{"type":"image_generation"}]);
    let rebuilt = core.bind(real).unwrap();
    assert!(rebuilt.rebuild_applied);
    assert_eq!(
        rebuilt.body["instructions"],
        "actual new transport instruction"
    );
    assert!(rebuilt.body.get("previous_response_id").is_none());
    assert_eq!(rebuilt.body["input"].as_array().unwrap().len(), 3);
    core.observe(observation(
        &prepared,
        "with-real-defaults",
        1,
        vec![terminal("defaults-b", vec![])],
    ))
    .unwrap();
    assert_eq!(active_account(&path).as_deref(), Some("account-b"));
}

#[tokio::test]
async fn raw_events_larger_than_sixteen_megabytes_are_registered_before_http_ack() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let config = config(path.clone());
    let token = config.token.clone();
    let handle = start(config).unwrap();
    let base = format!("http://{}/v1/continuity", handle.addr);
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let prepared: PrepareResponse = client
        .post(format!("{base}/prepare"))
        .bearer_auth(&token)
        .json(&request("large-event"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let _: BindResponse = client
        .post(format!("{base}/bind"))
        .bearer_auth(&token)
        .json(&binding(&prepared, "large-a", "account-a"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let event = json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","encrypted_content":"x".repeat(17 * 1024 * 1024)}});
    let ack: ObserveResponse = client
        .post(format!("{base}/observe"))
        .bearer_auth(&token)
        .json(&observation(&prepared, "large-a", 1, vec![event]))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(ack.next_sequence, 2);
    assert_eq!(count(&path, "native_opaque_v1"), 1);
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
    let released = client
        .post(format!("{base}/release"))
        .bearer_auth(&token)
        .json(&ReleaseRequest {
            ticket: prepared.ticket,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(released.status(), 200);
    handle.stop().await.unwrap();
}

#[test]
fn recovering_a_child_cannot_replace_its_roots_session_or_thread_affinity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let parent = core.prepare(request("parent")).unwrap();
    core.bind(binding(&parent, "parent-a", "account-a"))
        .unwrap();
    core.observe(observation(
        &parent,
        "parent-a",
        1,
        vec![terminal("parent-response", vec![])],
    ))
    .unwrap();
    core.finish(finish_request(
        &parent,
        "parent-a",
        2,
        FinishStatus::Succeeded,
    ))
    .unwrap();
    let mut child = request("child");
    child.body["client_metadata"] =
        json!({"session_id":"root","thread_id":"child","x-codex-parent-thread-id":"root"});
    let child = core.prepare(child).unwrap();
    core.bind(binding(&child, "child-b", "account-b")).unwrap();
    core.observe(observation(
        &child,
        "child-b",
        1,
        vec![terminal("child-response", vec![])],
    ))
    .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    for kind in ["session", "thread"] {
        let account: String = connection.query_row("SELECT account FROM native_affinity_v1 WHERE caller='native:primary' AND kind=?1 AND original='root'", [kind], |row| row.get(0)).unwrap();
        assert_eq!(account, "account-a");
    }
    let account: String = connection.query_row("SELECT account FROM native_affinity_v1 WHERE caller='native:primary' AND kind='thread' AND original='child'", [], |row| row.get(0)).unwrap();
    assert_eq!(account, "account-b");
}

#[tokio::test]
async fn loopback_http_authentication_and_shutdown_use_only_the_local_control_plane() {
    let temp = tempfile::tempdir().unwrap();
    let config = config(temp.path().join("identity.sqlite"));
    let token = config.token.clone();
    let handle = start(config).unwrap();
    assert!(handle.addr.ip().is_loopback());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("http://{}/v1/continuity/prepare", handle.addr);
    let denied = client
        .post(&url)
        .json(&request("http-request"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    assert_eq!(
        denied.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_continuity_token"
    );
    let prepared: PrepareResponse = client
        .post(&url)
        .bearer_auth(&token)
        .json(&request("http-request"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let repeated: PrepareResponse = client
        .post(&url)
        .bearer_auth(&token)
        .json(&request("http-request"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(prepared.ticket, repeated.ticket);
    handle.stop().await.unwrap();
    assert!(handle.is_finished());
}

#[test]
fn compact_automatic_unknown_history_keeps_payload_without_committing_a_generation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = Core::open(config(path.clone())).unwrap();
    let mut original = request("compact-unknown");
    original.compact = true;
    original.body["input"] = json!([{"type":"compaction","encrypted_content":"old-compact-cipher"},{"role":"user","content":"continue"}]);
    let prepared = core.prepare(original.clone()).unwrap();
    let bound = core
        .bind(binding(&prepared, "compact-attempt", "account-a"))
        .unwrap();
    assert_eq!(bound.body["input"], original.body["input"]);
    core.observe(observation(&prepared,"compact-attempt",1,vec![json!({"id":"compact-result","output":[{"type":"compaction","encrypted_content":"new-compact-cipher"}]})])).unwrap();
    core.finish(finish_request(
        &prepared,
        "compact-attempt",
        2,
        FinishStatus::Succeeded,
    ))
    .unwrap();
    assert_eq!(count(&path, "native_rebuild_active_v1"), 0);
    assert_eq!(count(&path, "native_rebuild_checkpoints_v1"), 0);
}
