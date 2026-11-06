// Included in native_host_tests; exercises policy without writing request logs.

fn native_cooldown_report(status: u16, error: &str) -> native_contract::RequestReport {
    native_contract::RequestReport {
        transient_rate_limit: false,
        official_access_fingerprint: None,
        official_authorization_revision: None,
        official_registration_id: None,
        official_fallback_registration_id: None,
        official_fallback_authorization_revision: None,
        official_authorization_error: None,
        operation: "text",
        diagnostics: None,
        caller: native_test_caller("fixture-key-id"),
        account_id: "pool-a".into(),
        requested_model: "fixture-model".into(),
        upstream_model: "fixture-model".into(),
        status,
        success: false,
        elapsed: Duration::ZERO,
        usage: None,
        error: Some(error.into()),
        error_message: None,
        credential_scope: false,
        retry_after: None,
        requested_service_tier: None,
        response_service_tier: None,
        reasoning_effort: None,
    }
}

#[test]
fn native_connection_failures_do_not_cool_accounts_or_shorten_existing_cooldown() {
    let host = NativeHostBridge::default();
    let codes = [
        (503, "account_cooling"),
        (502, "upstream_unavailable"),
        (502, "upstream_stream_error"),
        (502, "upstream_websocket_lost"),
        (502, "incomplete_upstream_stream"),
        (504, "upstream_connect_timeout"),
        (504, "upstream_idle_timeout"),
        (504, "upstream_total_timeout"),
        (504, "upstream_timeout"),
        (502, "websocket_connect_failed"),
        (504, "websocket_connect_timeout"),
        (504, "websocket_initial_timeout"),
        (502, "websocket_send_failed"),
        (504, "websocket_send_timeout"),
        (502, "realtime_upstream_disconnected"),
        (503, "service_stopping"),
        (503, "blocking_capacity_timeout"),
        (500, "identity_store_unavailable"),
        (504, "request_policy_timeout"),
        (504, "image_routing_timeout"),
    ];
    for (status, code) in codes {
        host.record_failure_cooldown(&native_cooldown_report(status, code));
    }
    let mut request_fault = native_cooldown_report(502, "upstream_error");
    request_fault.error_message = Some("Request contains an invalid argument.".into());
    request_fault.diagnostics = Some(native_api::diagnostics::DiagnosticContext {
        upstream_request_scoped: true,
        ..Default::default()
    });
    host.record_failure_cooldown(&request_fault);
    assert!(host.cooldowns.lock().unwrap().is_empty());

    let until = Instant::now() + Duration::from_secs(3600);
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", "fixture-model", false),
        NativeCooldown {
            until,
            quota: true,
            backoff_level: 1,
            reason: "quota",
        },
    );
    for (status, code) in codes {
        host.record_failure_cooldown(&native_cooldown_report(status, code));
    }
    assert_eq!(
        host.cooldowns.lock().unwrap()[&native_cooldown_key("pool-a", "fixture-model", false)]
            .until,
        until
    );
}

#[test]
fn native_actual_upstream_failures_keep_auth_quota_and_server_cooldowns() {
    for (status, code, mandatory) in [
        (401, "upstream_http_401", false),
        (401, "upstream_stream_error", false),
        (403, "upstream_http_403", false),
        (429, "upstream_http_429", true),
        (500, "upstream_http_500", false),
        (502, "upstream_http_502", false),
        (503, "upstream_http_503", false),
        (504, "websocket_upstream_rejected", false),
        (503, "image_upstream_error", false),
        (503, "realtime_upstream_error", false),
        (503, "server_error", false),
        (503, "server_is_overloaded", false),
    ] {
        let host = NativeHostBridge::default();
        let before = Instant::now();
        host.record_failure_cooldown(&native_cooldown_report(status, code));
        let cooldowns = host.cooldowns.lock().unwrap();
        let state = &cooldowns[&native_cooldown_key("pool-a", "fixture-model", false)];
        let (until, actual_mandatory) = (state.until, state.quota);
        assert_eq!(actual_mandatory, mandatory, "{code}");
        assert!(
            until
                >= before
                    + Duration::from_secs(match status {
                        401 | 403 => 1800,
                        429 => 1,
                        _ => 60,
                    })
        );
    }
    let host = NativeHostBridge::default();
    let mut report = native_cooldown_report(503, "upstream_http_503");
    report.retry_after = Some(Duration::from_secs(123));
    let before = Instant::now();
    host.record_failure_cooldown(&report);
    let cooldowns = host.cooldowns.lock().unwrap();
    assert!(
        cooldowns[&native_cooldown_key("pool-a", "fixture-model", false)].until
            >= before + Duration::from_secs(60)
    );
    assert!(!cooldowns[&native_cooldown_key("pool-a", "fixture-model", false)].quota);
}

#[tokio::test]
async fn native_cloudflare_challenge_cools_briefly_without_marking_credentials_invalid() {
    let host = NativeHostBridge::default();
    let before = Instant::now();
    native_contract::Host::route_failed_with_context(
        &host,
        "pool-a",
        "fixture-model",
        403,
        None,
        false,
        Some("cloudflare challenge"),
    )
    .await;
    let key = native_cooldown_key("pool-a", "fixture-model", false);
    let until = host.cooldowns.lock().unwrap()[&key].until;
    assert!(until >= before + Duration::from_secs(10));
    assert!(until < before + Duration::from_secs(11));
    let mut report = native_cooldown_report(403, "upstream_http_error");
    report.error_message = Some("cloudflare challenge".into());
    host.record_failure_cooldown(&report);
    let states = host.cooldowns.lock().unwrap();
    assert_eq!(states[&key].until, until);
    assert!(native_cooldown_wait(&states, "pool-a", "fixture-model", true, before).is_none());
    assert!(native_cooldown_wait(&states, "pool-a", "another-model", false, before).is_none());
}

#[test]
fn native_unrouted_and_successful_reports_do_not_cool_accounts() {
    let host = NativeHostBridge::default();
    let mut report = native_cooldown_report(503, "no_available_account");
    report.account_id.clear();
    host.record_failure_cooldown(&report);
    let mut report = native_cooldown_report(200, "unused");
    report.success = true;
    host.record_failure_cooldown(&report);
    host.record_failure_cooldown(&native_cooldown_report(409, "continuation_owner_mismatch"));
    assert!(host.cooldowns.lock().unwrap().is_empty());
}

#[test]
fn native_cooldown_models_are_independent_but_confirmed_credentials_cover_all_models() {
    let host = NativeHostBridge::default();
    host.record_failure_cooldown(&native_cooldown_report(503, "upstream_http_503"));
    let now = Instant::now();
    assert!(native_cooldown_wait(
        &host.cooldowns.lock().unwrap(),
        "pool-a",
        "fixture-model",
        false,
        now
    )
    .is_some());
    assert!(native_cooldown_wait(
        &host.cooldowns.lock().unwrap(),
        "pool-a",
        "other-model",
        false,
        now
    )
    .is_none());
    let mut report = native_cooldown_report(429, "usage_limit_reached");
    report.credential_scope = true;
    report.retry_after = Some(Duration::from_secs(15));
    host.record_failure_cooldown(&report);
    assert!(native_cooldown_wait(
        &host.cooldowns.lock().unwrap(),
        "pool-a",
        "other-model",
        true,
        now
    )
    .is_none());
    assert!(native_cooldown_wait(
        &host.cooldowns.lock().unwrap(),
        "pool-b",
        "other-model",
        false,
        now
    )
    .is_none());
}

#[test]
fn native_quota_backoff_escalates_once_per_window_and_hints_never_shorten_it() {
    let mut states = HashMap::new();
    let now = Instant::now();
    let key = native_cooldown_key("a", "model", false);
    native_upsert_cooldown(&mut states, "a", "model", 429, None, false, now);
    assert_eq!(states[&key].until, now + Duration::from_secs(1));
    assert_eq!(states[&key].backoff_level, 1);
    native_upsert_cooldown(
        &mut states,
        "a",
        "model",
        429,
        None,
        false,
        now + Duration::from_millis(100),
    );
    assert_eq!(states[&key].until, now + Duration::from_secs(1));
    assert_eq!(states[&key].backoff_level, 1);
    native_upsert_cooldown(
        &mut states,
        "a",
        "model",
        429,
        None,
        false,
        now + Duration::from_secs(2),
    );
    assert_eq!(states[&key].until, now + Duration::from_secs(4));
    assert_eq!(states[&key].backoff_level, 2);
    native_upsert_cooldown(
        &mut states,
        "a",
        "model",
        429,
        Some(Duration::ZERO),
        false,
        now + Duration::from_secs(3),
    );
    assert_eq!(states[&key].until, now + Duration::from_secs(13));
    native_upsert_cooldown(
        &mut states,
        "a",
        "model",
        503,
        None,
        false,
        now + Duration::from_secs(4),
    );
    assert!(states[&key].quota);
    assert!(states[&key].until >= now + Duration::from_secs(13));
}

#[tokio::test]
async fn native_cooldown_wait_is_scoped_and_initial_selection_waits_only_within_limit() {
    let mut host = native_test_routing_host();
    let collection = host.collection_override.as_mut().unwrap();
    collection.max_retry_interval_ms = 200;
    collection.max_account_concurrency = 0;
    let until = Instant::now() + Duration::from_millis(60);
    for account in ["pool-a", "pool-b"] {
        host.cooldowns.lock().unwrap().insert(
            native_cooldown_key(account, "gpt-fixture", false),
            NativeCooldown {
                until,
                quota: false,
                backoff_level: 0,
                reason: "transient_upstream",
            },
        );
    }
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_test_route_request("cooldown-root");
    request.preferred_account_id = Some("pool-a".into());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::from_millis(1))
            .await
            .unwrap()
            .is_none()
    );
    let start = Instant::now();
    let selected = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(selected.account_id, "pool-a");
    assert!(start.elapsed() >= Duration::from_millis(30));
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn native_cooldown_wait_can_be_cancelled_without_holding_selection_or_state_locks() {
    let mut host = native_test_routing_host();
    host.collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 1000;
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", "gpt-fixture", false),
        NativeCooldown {
            until: Instant::now() + Duration::from_millis(500),
            quota: false,
            backoff_level: 0,
            reason: "transient_upstream",
        },
    );
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_test_route_request("cancel-wait");
    request.preferred_account_id = Some("pool-a".into());
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        native_select_route(&host, &caller, request)
    )
    .await
    .is_err());
    assert!(host.cooldowns.try_lock().is_ok());
    let selected = native_select_route(&host, &caller, native_test_route_request("other-session"))
        .await
        .unwrap();
    assert_eq!(selected.account_id, "pool-b");
}

#[test]
fn disabling_cooling_clears_errors_without_clearing_account_quota_data() {
    let host = native_test_routing_host();
    let quota_before = host.account_overrides.clone();
    for status in [401, 403, 404, 429, 503] {
        host.record_failure_cooldown(&native_cooldown_report(status, "upstream_http_error"));
    }
    assert!(!host.cooldowns.lock().unwrap().is_empty());
    native_contract::Host::set_cooling_disabled(&host, true);
    assert!(host.cooldowns.lock().unwrap().is_empty());
    host.record_failure_cooldown(&native_cooldown_report(429, "usage_limit_reached"));
    assert!(host.cooldowns.lock().unwrap().is_empty());
    native_contract::Host::set_cooling_disabled(&host, false);
    assert!(host.cooldowns.lock().unwrap().is_empty());
    assert_eq!(
        serde_json::to_value(&host.account_overrides).unwrap(),
        serde_json::to_value(&quota_before).unwrap()
    );
    host.record_failure_cooldown(&native_cooldown_report(429, "usage_limit_reached"));
    assert!(!host.cooldowns.lock().unwrap().is_empty());
}

#[test]
fn model_errors_cool_only_the_model_and_parameters_do_not_cool() {
    let host = NativeHostBridge::default();
    let mut report = native_cooldown_report(400, "upstream_model_unavailable");
    report.error_message = Some("Requested model is not supported".into());
    let before = Instant::now();
    host.record_failure_cooldown(&report);
    let states = host.cooldowns.lock().unwrap();
    assert!(
        states[&native_cooldown_key("pool-a", "fixture-model", false)].until
            >= before + Duration::from_secs(12 * 3600)
    );
    assert!(native_cooldown_wait(&states, "pool-a", "another-model", false, before).is_none());
    drop(states);
    let host = NativeHostBridge::default();
    report.error = Some("upstream_http_error".into());
    report.error_message = Some("invalid parameter".into());
    host.record_failure_cooldown(&report);
    assert!(host.cooldowns.lock().unwrap().is_empty());
    report.error_message = Some("unsupported model".into());
    host.record_failure_cooldown(&report);
    assert!(
        host.cooldowns.lock().unwrap().is_empty(),
        "ambiguous text must not override the original error classification"
    );
}

#[test]
fn transient_admission_cools_only_the_model_without_quota_or_backoff() {
    let now=Instant::now(); let mut states=HashMap::new();
    native_upsert_transient_rate_limit(&mut states,"a","m",Some(Duration::from_millis(200)),now);
    let state=&states[&native_cooldown_key("a","m",false)];
    assert_eq!(state.until,now+Duration::from_millis(200)); assert!(!state.quota); assert_eq!(state.backoff_level,0);
    assert!(native_cooldown_wait(&states,"a","other",false,now).is_none());
    native_upsert_transient_rate_limit(&mut states,"a","m",None,now);
    assert_eq!(states[&native_cooldown_key("a","m",false)].until,now+Duration::from_secs(1));
    let quota_until=now+Duration::from_secs(3600);
    states.insert(native_cooldown_key("a","m",false),NativeCooldown{until:quota_until,quota:true,backoff_level:3,reason:"quota"});
    native_upsert_transient_rate_limit(&mut states,"a","m",Some(Duration::from_secs(2)),now);
    assert_eq!(states[&native_cooldown_key("a","m",false)].until,quota_until);
    assert!(states[&native_cooldown_key("a","m",false)].quota);
    let host=NativeHostBridge::default(); let mut report=native_cooldown_report(429,"turn_rate_limit_exceeded");
    report.transient_rate_limit=true; report.retry_after=Some(Duration::from_millis(200));
    host.record_failure_cooldown(&report);
    assert!(!host.cooldowns.lock().unwrap()[&native_cooldown_key("pool-a","fixture-model",false)].quota);
}

#[test]
fn native_clear_quota_cooldowns_unblocks_only_quota_state_for_that_account() {
    let now = Instant::now();
    let mut states = HashMap::new();
    native_mark_quota_exhausted(&mut states, "pool-a", Some(Duration::from_secs(7989)), now);
    native_upsert_cooldown(&mut states, "pool-a", "m", 429, Some(Duration::from_secs(600)), false, now);
    native_upsert_cooldown(&mut states, "pool-a", "m2", 503, None, false, now);
    native_mark_quota_exhausted(&mut states, "pool-b", Some(Duration::from_secs(7989)), now);
    assert!(native_cooldown_wait(&states, "pool-a", "m", false, now).is_some());

    assert!(native_clear_quota_cooldowns(&mut states, "pool-a"));
    // Quota barriers for the refreshed account are gone.
    assert!(native_cooldown_wait(&states, "pool-a", "m", false, now).is_none());
    // A transient (non-quota) cooldown is left to expire on its own.
    assert!(native_cooldown_wait(&states, "pool-a", "m2", false, now).is_some());
    // Other accounts are untouched.
    assert!(native_cooldown_wait(&states, "pool-b", "m", false, now).is_some());
    // Nothing left to clear.
    assert!(!native_clear_quota_cooldowns(&mut states, "pool-a"));
}
