//! Ordinary Go forwarding retains legacy history without relaxing trusted scope or retries.
use std::{collections::HashMap, path::Path};

use serde_json::{json, Value};

use super::{protocol::*, session::Core, Config};

fn core(path: &Path) -> Core {
    Core::open(Config {
        identity_path: path.to_owned(),
        token: "legacy-forwarding-private-test-token".into(),
        session_identity_mapping_enabled: true,
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
    })
    .unwrap()
}

fn request(id: &str, caller: &str, input: Value) -> PrepareRequest {
    PrepareRequest {
        request_id: id.into(),
        caller: caller.into(),
        headers: Headers::new(),
        body: json!({
            "model":"gpt-test",
            "input":input,
            "client_metadata":{"session_id":id,"thread_id":id}
        }),
        compact: false,
        allow_rebuild: true,
    }
}

fn bind(prepared: &PrepareResponse, attempt: &str, account: &str) -> BindRequest {
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

fn observe(core: &Core, prepared: &PrepareResponse, event: Value) -> ObserveResponse {
    core.observe(ObserveRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: "attempt".into(),
        sequence: 1,
        headers: None,
        events: vec![event],
    })
    .unwrap()
}

fn incomplete_history(cipher: &str) -> Value {
    json!([
        {"role":"user","content":"Continue the original conversation."},
        {"type":"reasoning","id":"original-reasoning","summary":[],"encrypted_content":cipher},
        {"type":"function_call_output","call_id":"unavailable-original-call","output":"Original result\nwith exact whitespace.  "}
    ])
}

#[test]
fn multi_result_history_go_bridge_preserves_both_results_after_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let original = request("multi-result", "native:primary", json!([
        {"role":"user","content":"Original task"},
        {"type":"reasoning","summary":[],"encrypted_content":"unregistered-multi-result-cipher"},
        {"type":"function_call","name":"read_file","call_id":"same-call","arguments":"{}"},
        {"type":"function_call_output","call_id":"same-call","output":"First result\n  keep whitespace  "},
        {"type":"function_call_output","call_id":"same-call","output":"Second distinct result"},
        {"role":"user","content":"Continue the same conversation"}
    ]));
    let saved = original.body.clone();
    for _ in 0..2 {
        let core = core(&path);
        let prepared = core.prepare(original.clone()).unwrap();
        assert_eq!(prepared.body, saved);
        assert!(!prepared.self_contained, "unverified history must not authorize account switching");
        let bound = core.bind(bind(&prepared, "attempt", "account-a")).unwrap();
        assert_eq!(bound.body["input"], saved["input"]);
        assert!(!bound.rebuild_applied);
        let completed = observe(&core, &prepared, json!({
            "type":"response.completed",
            "response":{"id":"multi-result-completed","status":"completed","output":[]}
        }));
        assert!(completed.terminal);
        core.close();
    }
    assert_eq!(original.body, saved);
}

#[test]
fn legacy_forwarding_binds_unknown_cipher_and_unverified_history_without_rebuild_claims() {
    let histories = [
        incomplete_history("unregistered-orphan-history"),
        json!([
            {"type":"reasoning","summary":[],"encrypted_content":"unregistered-pending-history"},
            {"type":"function_call","call_id":"pending-call","name":"existing_tool","arguments":"{}"}
        ]),
        json!([
            {"type":"reasoning","summary":[],"encrypted_content":"unregistered-future-history"},
            {"type":"future_history_item","id":"future-original-id","payload":{"text":"Original future payload","flag":true}}
        ]),
    ];
    for (index, history) in histories.into_iter().enumerate() {
        let temp = tempfile::tempdir().unwrap();
        let core = core(&temp.path().join("identity.sqlite"));
        let original = request(&format!("legacy-{index}"), "native:primary", history);
        let prepared = core.prepare(original.clone()).unwrap();
        assert_eq!(prepared.body, original.body);
        assert!(
            !prepared.self_contained,
            "ordinary forwarding must not authorize automatic cross-account replay"
        );
        let bound = core.bind(bind(&prepared, "attempt", "account-a")).unwrap();
        assert_eq!(bound.body["input"], original.body["input"]);
        assert!(!bound.rebuild_applied);
    }
}

#[test]
fn legacy_forwarding_upstream_failure_does_not_publish_input_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.sqlite");
    let core = core(&path);
    let history = incomplete_history("failed-forwarding-cipher");
    let prepared = core
        .prepare(request("failed-import", "native:primary", history.clone()))
        .unwrap();
    core.bind(bind(&prepared, "attempt", "account-a")).unwrap();
    let result = observe(
        &core,
        &prepared,
        json!({
            "type":"response.failed",
            "response":{
                "id":"failed-upstream-response",
                "status":"failed",
                "error":{"code":"invalid_encrypted_content","message":"Original upstream rejection"}
            }
        }),
    );
    assert!(result.terminal);
    core.finish(FinishRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: "attempt".into(),
        sequence: result.next_sequence,
        status: FinishStatus::Failed,
        error_code: Some("invalid_encrypted_content".into()),
    })
    .unwrap();

    let published: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(published, 0, "a rejected input must remain unclaimed");

    let retried = core
        .prepare(request(
            "different-caller-retry",
            "restricted",
            history.clone(),
        ))
        .unwrap();
    let bound = core.bind(bind(&retried, "attempt", "account-b")).unwrap();
    assert_eq!(bound.body["input"], history);
}

#[test]
fn legacy_forwarding_preserves_known_foreign_caller_and_account_scope_rejections() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp.path().join("identity.sqlite"));
    let prepared = core
        .prepare(request(
            "known-owner",
            "native:primary",
            json!([{"role":"user","content":"Initial request"}]),
        ))
        .unwrap();
    core.bind(bind(&prepared, "attempt", "account-a")).unwrap();
    observe(
        &core,
        &prepared,
        json!({
            "type":"response.completed",
            "response":{
                "id":"owned-response",
                "status":"completed",
                "output":[{"type":"reasoning","summary":[],"encrypted_content":"known-foreign-cipher"}]
            }
        }),
    );
    let foreign = core
        .prepare(request(
            "foreign-caller",
            "restricted",
            incomplete_history("known-foreign-cipher"),
        ))
        .err()
        .expect("known ownership cannot be bypassed with unverified input");
    assert_eq!(foreign.code, "upstream_state_caller_mismatch");

    let restricted = core
        .prepare(request(
            "restricted-account",
            "restricted",
            incomplete_history("unregistered-restricted-cipher"),
        ))
        .unwrap();
    assert_eq!(
        core.bind(bind(&restricted, "attempt", "account-a"))
            .err()
            .unwrap()
            .code,
        "continuity_account_scope"
    );
    let mut forged_owner = bind(&restricted, "attempt-forged", "account-b");
    forged_owner.owner_id = Some("forged-owner".into());
    assert_eq!(
        core.bind(forged_owner).err().unwrap().code,
        "continuity_account_scope"
    );
}

#[test]
fn legacy_forwarding_observed_output_still_prevents_a_second_upstream_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp.path().join("identity.sqlite"));
    let prepared = core
        .prepare(request(
            "output-started",
            "native:primary",
            incomplete_history("unregistered-output-started-cipher"),
        ))
        .unwrap();
    let initial = bind(&prepared, "attempt", "account-a");
    let bound = core.bind(initial.clone()).unwrap();
    let result = observe(
        &core,
        &prepared,
        json!({"type":"response.output_text.delta","item_id":"message-a","output_index":0,"content_index":0,"delta":"Already executing"}),
    );
    assert!(!result.terminal);
    assert_eq!(core.bind(initial).unwrap().body, bound.body);
    for account in ["account-a", "account-b"] {
        assert_eq!(
            core.bind(bind(&prepared, &format!("retry-{account}"), account))
                .err()
                .unwrap()
                .code,
            "continuity_output_started"
        );
    }
}

fn fail_attempt(core: &Core, prepared: &PrepareResponse, attempt: &str, sequence: u64, code: &str) {
    core.finish(FinishRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: attempt.into(),
        sequence,
        status: FinishStatus::Failed,
        error_code: Some(code.into()),
    })
    .unwrap();
}

#[test]
fn legacy_forwarding_unverified_history_retries_remain_on_the_first_account() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp.path().join("identity.sqlite"));
    let prepared = core
        .prepare(request(
            "unverified-retry",
            "native:primary",
            incomplete_history("unverified-retry-cipher"),
        ))
        .unwrap();
    assert!(!prepared.self_contained);
    core.bind(bind(&prepared, "first", "account-a")).unwrap();
    assert_eq!(
        core.bind(bind(&prepared, "different-account", "account-b"))
            .err()
            .unwrap()
            .code,
        "continuity_unverified_account_retry"
    );
    fail_attempt(&core, &prepared, "first", 1, "continuity_upstream_failed");
    assert_eq!(
        core.bind(bind(&prepared, "after-upstream-failure", "account-b"))
            .err()
            .unwrap()
            .code,
        "continuity_unverified_account_retry"
    );
    core.bind(bind(&prepared, "same-account", "account-a"))
        .unwrap();
    // A later unsent candidate must not erase the earlier execution's pin.
    fail_attempt(
        &core,
        &prepared,
        "same-account",
        1,
        "continuity_upstream_not_sent",
    );
    assert_eq!(
        core.bind(bind(&prepared, "after-later-unsent", "account-b"))
            .err()
            .unwrap()
            .code,
        "continuity_unverified_account_retry"
    );
}

#[test]
fn legacy_forwarding_proven_unsent_candidate_can_use_another_account() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp.path().join("identity.sqlite"));
    let prepared = core
        .prepare(request(
            "unsent-retry",
            "native:primary",
            incomplete_history("unsent-retry-cipher"),
        ))
        .unwrap();
    core.bind(bind(&prepared, "first", "account-a")).unwrap();
    fail_attempt(&core, &prepared, "first", 1, "continuity_upstream_not_sent");
    let next = core.bind(bind(&prepared, "next", "account-b")).unwrap();
    assert_eq!(next.body["input"], prepared.body["input"]);
    assert!(!next.rebuild_applied);
}

#[test]
fn legacy_forwarding_observed_headers_prevent_an_unsent_claim_from_releasing_the_pin() {
    let temp = tempfile::tempdir().unwrap();
    let core = core(&temp.path().join("identity.sqlite"));
    let prepared = core
        .prepare(request(
            "observed-retry",
            "native:primary",
            incomplete_history("observed-retry-cipher"),
        ))
        .unwrap();
    core.bind(bind(&prepared, "attempt", "account-a")).unwrap();
    core.observe(ObserveRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: "attempt".into(),
        sequence: 1,
        headers: Some(Headers::new()),
        events: vec![],
    })
    .unwrap();
    fail_attempt(
        &core,
        &prepared,
        "attempt",
        2,
        "continuity_upstream_not_sent",
    );
    assert_eq!(
        core.bind(bind(&prepared, "next", "account-b"))
            .err()
            .unwrap()
            .code,
        "continuity_unverified_account_retry"
    );
}

#[test]
fn legacy_forwarding_stateless_requests_and_proven_complete_history_can_change_accounts() {
    for (id, input, allow_rebuild) in [
        (
            "stateless-retry",
            json!([{"role":"user","content":"Ordinary stateless request"}]),
            false,
        ),
        (
            "complete-retry",
            json!([
                {"role":"user","content":"Complete history"},
                {"type":"reasoning","summary":[],"encrypted_content":"complete-history-cipher"}
            ]),
            true,
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let core = core(&temp.path().join("identity.sqlite"));
        let mut original = request(id, "native:primary", input);
        original.allow_rebuild = allow_rebuild;
        let prepared = core.prepare(original).unwrap();
        assert_eq!(prepared.self_contained, allow_rebuild);
        core.bind(bind(&prepared, "first", "account-a")).unwrap();
        fail_attempt(&core, &prepared, "first", 1, "continuity_upstream_failed");
        core.bind(bind(&prepared, "next", "account-b")).unwrap();
    }
}
