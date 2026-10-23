// Synthetic scheduler state only; no account files, GUI or live network.
fn native_health_fixture() -> (Arc<NativeHostBridge>, GatewayRuntime) {
    let mut collection = native_test_collection();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
    collection.account_ids = vec!["pool-a".into(), "pool-b".into()];
    let host = Arc::new(NativeHostBridge::for_collection(collection.clone()));
    let runtime = GatewayRuntime {
        collection: Some(collection),
        running: true,
        native_health_source: Some(Arc::downgrade(&host)),
        ..Default::default()
    };
    (host, runtime)
}

#[test]
fn native_health_snapshot_uses_live_model_and_credential_cooldowns() {
    let (host, mut runtime) = native_health_fixture();
    let now = now_ms();
    // A stale Go cooldown must not leak into the active native scheduler view.
    runtime.model_cooldowns.insert(
        build_cooldown_key("pool-b", "other").unwrap(),
        AccountModelCooldown {
            model_key: "other".into(),
            next_retry_at_ms: now + 90_000,
            reason: "stale".into(),
        },
    );
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    let health = build_account_health_snapshot(&runtime);
    let a = health
        .iter()
        .find(|item| item.account_id == "pool-a")
        .unwrap();
    assert!(!a.available);
    assert_eq!(a.cooldowns.len(), 1);
    assert_eq!(a.cooldowns[0].model_id, "fixture-model");
    assert_eq!(a.cooldowns[0].reason, "transient_upstream");
    assert!((58_000..=60_000).contains(&a.cooldowns[0].remaining_ms));
    assert_eq!(
        a.scheduler_next_retry_at,
        Some(a.cooldowns[0].next_retry_at)
    );
    let b = health
        .iter()
        .find(|item| item.account_id == "pool-b")
        .unwrap();
    assert!(b.available);
    assert!(b.cooldowns.is_empty());
    // Both full service state and request state use the same authoritative view.
    assert!(!build_request_state_snapshot(&runtime).account_health[0].available);

    let mut failure = native_cooldown_report(401, "upstream_http_401");
    failure.account_id = "pool-b".into();
    failure.credential_scope = true;
    host.record_failure_cooldown(&failure);
    let health = build_account_health_snapshot(&runtime);
    assert_eq!(health[1].cooldowns[0].model_id, "*");
    assert_eq!(health[1].cooldowns[0].reason, "unauthorized");
}

#[test]
fn native_health_snapshot_tracks_expiry_success_reset_and_cooling_disable() {
    let (host, mut runtime) = native_health_fixture();
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    host.cooldowns
        .lock()
        .unwrap()
        .values_mut()
        .for_each(|state| state.until = Instant::now());
    assert!(build_account_health_snapshot(&runtime)[0].available);
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    let mut success = native_cooldown_report(200, "");
    success.success = true;
    host.record_failure_cooldown(&success);
    assert!(build_account_health_snapshot(&runtime)[0]
        .cooldowns
        .is_empty());
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    native_contract::Host::set_cooling_disabled(host.as_ref(), true);
    assert!(build_account_health_snapshot(&runtime)[0].available);
    assert!(host.cooldowns.lock().unwrap().is_empty());
    native_contract::Host::set_cooling_disabled(host.as_ref(), false);
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    host.cooldowns.lock().unwrap().clear();
    assert!(build_account_health_snapshot(&runtime)[0].available);
    runtime.running = false;
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    assert!(build_account_health_snapshot(&runtime)[0]
        .cooldowns
        .is_empty());
    runtime.running = true;
    runtime.collection.as_mut().unwrap().gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    assert!(build_account_health_snapshot(&runtime)[0]
        .cooldowns
        .is_empty());
}

#[test]
fn native_health_snapshot_does_not_wait_for_scheduler_or_claim_ready_on_contention() {
    let (host, runtime) = native_health_fixture();
    let _held = host.cooldowns.lock().unwrap();
    let health = build_account_health_snapshot(&runtime);
    assert!(health.iter().all(|item| !item.available));
    assert!(health
        .iter()
        .all(|item| item.scheduler_reason.as_deref() == Some("state_refreshing")));
}

#[test]
fn native_health_keeps_authoritative_quota_when_soft_cooling_is_disabled() {
    let (host, mut runtime) = native_health_fixture();
    let now = now_ms();
    runtime.account_quota_cooldowns.insert(
        "pool-a".into(),
        AccountQuotaCooldown {
            exhausted: true,
            reset_at_ms: Some(now + 3600_000),
            updated_at_ms: now,
        },
    );
    host.record_failure_cooldown(&native_cooldown_report(503, "server_is_overloaded"));
    let before = build_account_health_snapshot(&runtime);
    assert_eq!(before[0].cooldowns.len(), 2);
    native_contract::Host::set_cooling_disabled(host.as_ref(), true);
    let after = build_account_health_snapshot(&runtime);
    assert!(!after[0].available);
    assert_eq!(after[0].cooldowns.len(), 1);
    assert_eq!(after[0].cooldowns[0].reason, "quota_exhausted");
    assert!(after[1].available);
}
