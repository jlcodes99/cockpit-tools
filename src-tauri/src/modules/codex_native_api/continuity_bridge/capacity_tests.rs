//! Shared-budget protection must not impose a single-conversation size ceiling.
use super::*;
use serde_json::{json, Value};

fn config(path: std::path::PathBuf) -> Config {
    Config {
        identity_path: path,
        token: "capacity-test-token-never-an-upstream-credential".into(),
        session_identity_mapping_enabled: true,
        owners: HashMap::from([("account-a".into(), "owner-a".into())]),
        caller_accounts: HashMap::from([("caller".into(), vec!["account-a".into()])]),
    }
}

fn request(id: &str) -> PrepareRequest {
    PrepareRequest {
        request_id: id.into(),
        caller: "caller".into(),
        headers: Headers::new(),
        body: json!({"model":"gpt-test","input":[{"role":"user","content":"first"}],"client_metadata":{"session_id":"root","thread_id":"root"}}),
        compact: false,
        allow_rebuild: true,
    }
}

fn bind(prepared: &PrepareResponse, attempt: &str) -> BindRequest {
    BindRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: attempt.into(),
        account_id: "account-a".into(),
        owner_id: Some("owner-a".into()),
        expand_http_previous: false,
        headers: prepared.headers.clone(),
        body: prepared.body.clone(),
    }
}

fn release(core: &Core, prepared: &PrepareResponse) {
    core.release(ReleaseRequest {
        ticket: prepared.ticket.clone(),
    })
    .unwrap();
}

fn observe(core: &Core, prepared: &PrepareResponse, event: Value) {
    core.observe(ObserveRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: "attempt".into(),
        sequence: 1,
        headers: None,
        events: vec![event],
    })
    .unwrap();
}

#[test]
fn oversized_ticket_excludes_new_requests_but_not_its_idempotent_retries() {
    let temp = tempfile::tempdir().unwrap();
    let core = Arc::new(Core::open(config(temp.path().join("identity.sqlite"))).unwrap());
    let tombstone = core.prepare(request("finished")).unwrap();
    release(&core, &tombstone);
    let prepared = core.prepare(request("exclusive")).unwrap();
    let ticket = core.lookup(&prepared.ticket).unwrap();
    core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
    assert!(core.registry.lock().unwrap().bytes > MAX_RESERVED_BYTES);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for index in 0..8 {
            let core = core.clone();
            let barrier = barrier.clone();
            let expected_ticket = prepared.ticket.clone();
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(
                    core.prepare(request(&format!("contender-{index}")))
                        .err()
                        .unwrap()
                        .code,
                    "continuity_capacity"
                );
                assert_eq!(
                    core.prepare(request("exclusive")).unwrap().ticket,
                    expected_ticket
                );
            });
        }
    });
    let first = core.bind(bind(&prepared, "attempt")).unwrap();
    assert_eq!(
        first.body,
        core.bind(bind(&prepared, "attempt")).unwrap().body
    );
    assert!(ticket.reserved.load(Ordering::Relaxed) > MAX_RESERVED_BYTES);
    release(&core, &prepared);
    release(&core, &prepared);
    assert!(core.registry.lock().unwrap().bytes < MAX_RESERVED_BYTES);
    core.prepare(request("after-release")).unwrap();
}

#[test]
fn ordinary_concurrent_requests_still_share_the_existing_budget() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let first = core.prepare(request("first")).unwrap();
    let second = core.prepare(request("second")).unwrap();
    let first_ticket = core.lookup(&first.ticket).unwrap();
    let second_ticket = core.lookup(&second.ticket).unwrap();
    core.reserve(&first_ticket, MAX_RESERVED_BYTES / 2).unwrap();
    core.reserve(&second_ticket, MAX_RESERVED_BYTES / 2)
        .unwrap();
    assert_eq!(
        core.reserve(&first_ticket, MAX_RESERVED_BYTES + 1)
            .err()
            .unwrap()
            .code,
        "continuity_capacity"
    );
    assert_eq!(core.registry.lock().unwrap().bytes, MAX_RESERVED_BYTES);
    assert_eq!(
        core.prepare(request("third")).err().unwrap().code,
        "continuity_capacity"
    );
    release(&core, &second);
    core.reserve(&first_ticket, MAX_RESERVED_BYTES + 1).unwrap();
    // Giving capacity back remains possible even while the total is still above
    // the normal budget; a finished tombstone must not make that operation fail.
    core.reserve(&first_ticket, MAX_RESERVED_BYTES).unwrap();
    release(&core, &first);
    core.prepare(request("after-contention")).unwrap();
}

#[test]
fn completed_failed_and_cancelled_tickets_release_exclusive_capacity() {
    for status in [
        FinishStatus::Succeeded,
        FinishStatus::Failed,
        FinishStatus::Cancelled,
    ] {
        let temp = tempfile::tempdir().unwrap();
        let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
        let tombstone = core.prepare(request("old-finished")).unwrap();
        release(&core, &tombstone);
        let prepared = core.prepare(request("exclusive")).unwrap();
        let ticket = core.lookup(&prepared.ticket).unwrap();
        core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
        core.bind(bind(&prepared, "attempt")).unwrap();
        let event = if status == FinishStatus::Succeeded {
            json!({"type":"response.completed","response":{"id":"capacity-result","status":"completed","output":[]}})
        } else {
            json!({"type":"response.output_text.delta","delta":"partial"})
        };
        observe(&core, &prepared, event);
        let finish = FinishRequest {
            ticket: prepared.ticket.clone(),
            attempt_id: "attempt".into(),
            sequence: 2,
            status,
            error_code: None,
        };
        core.finish(finish.clone()).unwrap();
        core.finish(finish).unwrap();
        assert!(ticket.finished.load(Ordering::Acquire));
        assert!(core.registry.lock().unwrap().bytes < MAX_RESERVED_BYTES);
        core.prepare(request("after-finish")).unwrap();
    }
}

#[test]
fn retryable_failure_and_unbound_cancellation_keep_existing_lifecycle_rules() {
    let temp = tempfile::tempdir().unwrap();
    let core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let prepared = core.prepare(request("exclusive")).unwrap();
    let ticket = core.lookup(&prepared.ticket).unwrap();
    core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
    core.bind(bind(&prepared, "attempt")).unwrap();
    core.finish(FinishRequest {
        ticket: prepared.ticket.clone(),
        attempt_id: "attempt".into(),
        sequence: 1,
        status: FinishStatus::Failed,
        error_code: None,
    })
    .unwrap();
    assert!(!ticket.finished.load(Ordering::Acquire));
    core.bind(bind(&prepared, "retry")).unwrap();
    release(&core, &prepared);
    let cancelled = core.prepare(request("cancelled-before-bind")).unwrap();
    let ticket = core.lookup(&cancelled.ticket).unwrap();
    core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
    core.finish(FinishRequest {
        ticket: cancelled.ticket.clone(),
        attempt_id: String::new(),
        sequence: 1,
        status: FinishStatus::Cancelled,
        error_code: None,
    })
    .unwrap();
    assert!(ticket.finished.load(Ordering::Acquire));
    core.prepare(request("after-cancellation")).unwrap();
}

#[test]
fn expired_unbound_exclusive_tickets_are_reclaimed_without_expiring_active_streams() {
    let temp = tempfile::tempdir().unwrap();
    let mut core = Core::open(config(temp.path().join("identity.sqlite"))).unwrap();
    let prepared = core.prepare(request("expired")).unwrap();
    let ticket = core.lookup(&prepared.ticket).unwrap();
    core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
    core.elapse_for_test(IDLE_SECONDS + 1);
    let current = core.prepare(request("current")).unwrap();
    assert!(!core
        .registry
        .lock()
        .unwrap()
        .tickets
        .contains_key(&prepared.ticket));
    let ticket = core.lookup(&current.ticket).unwrap();
    core.reserve(&ticket, MAX_RESERVED_BYTES + 1).unwrap();
    core.bind(bind(&current, "attempt")).unwrap();
    core.elapse_for_test(IDLE_SECONDS + 1);
    assert_eq!(
        core.prepare(request("cannot-evict-active"))
            .err()
            .unwrap()
            .code,
        "continuity_capacity"
    );
    release(&core, &current);
    core.prepare(request("after-active-release")).unwrap();
}

async fn wire<T: serde::Serialize, R: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    bridge: &super::super::Handle,
    method: &str,
    request: &T,
) -> R {
    let response = client
        .post(format!("http://{}/v1/continuity/{method}", bridge.addr))
        .bearer_auth("capacity-test-token-never-an-upstream-credential")
        .json(request)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        200,
        "{method}: {}",
        response.text().await.unwrap()
    );
    response.json().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_sixty_five_megabyte_history_prepares_and_binds_without_losing_cipher_or_scope() {
    const LARGE: usize = 65 * 1024 * 1024;
    let temp = tempfile::tempdir().unwrap();
    let bridge = super::super::start(config(temp.path().join("identity.sqlite"))).unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut original = request("large-actual-history");
    original.body["input"][0]["content"] = Value::String("x".repeat(LARGE));
    original.body["input"].as_array_mut().unwrap().push(json!({"type":"reasoning","id":"reasoning-capacity","summary":[],"encrypted_content":"gAAAA-original-capacity-cipher"}));
    let prepared: PrepareResponse = wire(&client, &bridge, "prepare", &original).await;
    assert!(prepared.self_contained);
    assert_eq!(
        prepared.body["input"][0]["content"].as_str().unwrap().len(),
        LARGE
    );
    let bound: BindResponse =
        wire(&client, &bridge, "bind", &bind(&prepared, "large-attempt")).await;
    let text = bound.body["input"][0]["content"].as_str().unwrap();
    assert_eq!(text.len(), LARGE);
    assert!(text.bytes().all(|byte| byte == b'x'));
    assert_eq!(bound.body["input"][1]["id"], "reasoning-capacity");
    assert_eq!(
        bound.body["input"][1]["encrypted_content"],
        "gAAAA-original-capacity-cipher"
    );
    let mut forged = bind(&prepared, "forged-owner");
    forged.body = Value::Null;
    forged.owner_id = Some("wrong-owner".into());
    assert_eq!(
        bridge.core.bind(forged).err().unwrap().code,
        "continuity_account_scope"
    );
    let mut foreign = request("foreign-caller");
    foreign.caller = "unconfigured-caller".into();
    assert_eq!(
        bridge.core.prepare(foreign).err().unwrap().code,
        "continuity_caller_scope"
    );
    let _: ReleaseResponse = wire(
        &client,
        &bridge,
        "release",
        &ReleaseRequest {
            ticket: prepared.ticket.clone(),
        },
    )
    .await;
    drop(bound);
    drop(prepared);
    original.request_id = "large-invalid-header".into();
    original
        .headers
        .insert("invalid header".into(), vec!["value".into()]);
    assert_eq!(
        bridge.core.prepare(original).err().unwrap().code,
        "invalid_continuity_headers"
    );
    let next: PrepareResponse =
        wire(&client, &bridge, "prepare", &request("after-large-failure")).await;
    let _: ReleaseResponse = wire(
        &client,
        &bridge,
        "release",
        &ReleaseRequest {
            ticket: next.ticket,
        },
    )
    .await;
    bridge.stop().await.unwrap();
}
