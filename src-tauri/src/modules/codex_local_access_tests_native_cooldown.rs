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
            recorded_at: Instant::now(),
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
                recorded_at: Instant::now(),
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
            recorded_at: Instant::now(),
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
    states.insert(native_cooldown_key("a","m",false),NativeCooldown{recorded_at:now,until:quota_until,quota:true,backoff_level:3,reason:"quota"});
    native_upsert_transient_rate_limit(&mut states,"a","m",Some(Duration::from_secs(2)),now);
    assert_eq!(states[&native_cooldown_key("a","m",false)].until,quota_until);
    assert!(states[&native_cooldown_key("a","m",false)].quota);
    let host=NativeHostBridge::default(); let mut report=native_cooldown_report(429,"turn_rate_limit_exceeded");
    report.transient_rate_limit=true; report.retry_after=Some(Duration::from_millis(200));
    host.record_failure_cooldown(&report);
    assert!(!host.cooldowns.lock().unwrap()[&native_cooldown_key("pool-a","fixture-model",false)].quota);
}

async fn native_test_wait_for_account_queue<F: std::future::Future + Unpin>(
    host: &NativeHostBridge,
    queued: &mut F,
) {
    tokio::select! {
        _ = queued => panic!("route completed before entering the concurrency queue"),
        result = tokio::time::timeout(Duration::from_secs(2), async {
            while host.account_waiters.load(Ordering::Acquire) == 0 {
                tokio::task::yield_now().await;
            }
        }) => result.expect("route did not enter the concurrency queue"),
    }
}

#[tokio::test]
async fn native_queued_affinity_rechecks_cooling_without_blocking_legal_routes() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (case, model, disabled, confirmed_quota, expired, expected) in [
        (
            "model cooling",
            "gpt-fixture",
            false,
            false,
            false,
            "pool-b",
        ),
        (
            "quota with cooling disabled",
            "",
            true,
            true,
            false,
            "pool-b",
        ),
        (
            "other model",
            "different-model",
            false,
            false,
            false,
            "pool-a",
        ),
        ("expired", "gpt-fixture", false, false, true, "pool-a"),
        (
            "cooling disabled",
            "gpt-fixture",
            true,
            false,
            false,
            "pool-a",
        ),
    ] {
        let mut host = native_test_routing_host();
        let collection = host.collection_override.as_mut().unwrap();
        collection.account_concurrency_wait_ms = 1_000;
        collection.max_retry_interval_ms = 0;
        collection.disable_cooling = disabled;
        // Admission must check the mapped upstream model, not the public alias.
        collection.model_aliases = vec![serde_json::from_value(json!({
            "alias": "public-model", "sourceModel": "gpt-fixture"
        }))
        .unwrap()];
        native_contract::Host::set_cooling_disabled(&host, disabled);
        let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
        let mut request = native_test_route_request("queued-affinity");
        request.model = "public-model".into();
        let busy = native_select_route(&host, &caller, request.clone())
            .await
            .unwrap();
        assert_eq!(busy.account_id, "pool-a", "{case}");
        let mut queued = Box::pin(native_select_route(&host, &caller, request));
        native_test_wait_for_account_queue(&host, &mut queued).await;
        assert_eq!(host.account_waiters.load(Ordering::Acquire), 1, "{case}");
        {
            let now = Instant::now();
            let mut states = host.cooldowns.lock().unwrap();
            if confirmed_quota {
                native_mark_quota_exhausted(
                    &mut states,
                    "pool-a",
                    Some(Duration::from_secs(60)),
                    now,
                );
            } else {
                native_upsert_cooldown(&mut states, "pool-a", model, 503, None, false, now);
                if expired {
                    states
                        .get_mut(&native_cooldown_key("pool-a", model, false))
                        .unwrap()
                        .until = now;
                }
            }
        }
        drop(busy);
        let selected = tokio::time::timeout(Duration::from_millis(500), queued)
            .await
            .expect(case)
            .expect(case);
        assert_eq!(selected.account_id, expected, "{case}");
        assert_eq!(host.account_waiters.load(Ordering::Acquire), 0, "{case}");
        drop(selected);
        assert!(
            host.gates
                .lock()
                .unwrap()
                .values()
                .all(|gate| gate.available_permits() == 1),
            "{case}"
        );
    }
}

#[tokio::test]
async fn native_queued_pool_skips_newly_cooled_candidate_and_waits_for_backup() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = native_test_routing_host();
    let collection = host.collection_override.as_mut().unwrap();
    collection.session_affinity = false;
    collection.account_concurrency_wait_ms = 1_000;
    collection.max_retry_interval_ms = 0;
    let collection = host.collection().await.unwrap();
    let accounts: Vec<_> = ["pool-a", "pool-b"]
        .iter()
        .map(|id| host.account_overrides[*id].clone())
        .collect();
    let (_, busy_a) = native_acquire_account_permit(&host, &collection, &accounts[..1], true)
        .await
        .unwrap();
    let (_, busy_b) = native_acquire_account_permit(&host, &collection, &accounts[1..], true)
        .await
        .unwrap();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut queued = Box::pin(native_select_route(
        &host,
        &caller,
        native_test_route_request("queued-pool"),
    ));
    native_test_wait_for_account_queue(&host, &mut queued).await;
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "pool-a",
        "gpt-fixture",
        503,
        None,
        false,
        Instant::now(),
    );
    drop(busy_a);
    // Only A has a free slot, but it has become ineligible while queued.
    tokio::select! {
        _ = &mut queued => panic!("newly cooled A must not be selected"),
        result = tokio::time::timeout(Duration::from_secs(2), async {
            while host.gates.lock().unwrap().get("pool-a:1")
                .is_some_and(|gate| gate.available_permits() != 1)
                || host.account_waiters.load(Ordering::Acquire) != 1
            {
                tokio::task::yield_now().await;
            }
        }) => result.expect("cooled slot was not released before waiting for B"),
    }
    drop(busy_b);
    let selected = tokio::time::timeout(Duration::from_millis(500), queued)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.account_id, "pool-b");
    drop(selected);
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
    assert!(host
        .gates
        .lock()
        .unwrap()
        .values()
        .all(|gate| gate.available_permits() == 1));
}

#[tokio::test]
async fn native_queued_cooling_without_backup_returns_error_and_releases_permit() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (pinned, rebuild_available, all_cooling) in [
        (false, false, false),
        (false, false, true),
        (true, false, false),
        (true, true, false),
    ] {
        let mut host = native_test_routing_host();
        let collection = host.collection_override.as_mut().unwrap();
        collection.account_concurrency_wait_ms = 1_000;
        collection.max_retry_interval_ms = if rebuild_available { 120_000 } else { 0 };
        if !pinned && !all_cooling {
            collection.account_ids = vec!["pool-a".into()];
        }
        let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
        let mut request = native_test_route_request("queued-owner");
        if pinned {
            request.preferred_account_id = Some("pool-a".into());
        }
        let busy = native_select_route(&host, &caller, request.clone())
            .await
            .unwrap();
        let mut queued = Box::pin(native_select_route_with_owner_wait(
            &host,
            &caller,
            request,
            rebuild_available,
        ));
        native_test_wait_for_account_queue(&host, &mut queued).await;
        if rebuild_available {
            native_mark_quota_exhausted(
                &mut host.cooldowns.lock().unwrap(),
                "pool-a",
                Some(Duration::from_secs(60)),
                Instant::now(),
            );
        } else {
            native_upsert_cooldown(
                &mut host.cooldowns.lock().unwrap(),
                "pool-a",
                "gpt-fixture",
                503,
                None,
                false,
                Instant::now(),
            );
        }
        if all_cooling {
            native_upsert_cooldown(
                &mut host.cooldowns.lock().unwrap(),
                "pool-b",
                "gpt-fixture",
                503,
                None,
                false,
                Instant::now(),
            );
        }
        drop(busy);
        let error = tokio::time::timeout(Duration::from_millis(500), queued)
            .await
            .unwrap()
            .err()
            .expect("cooled account must not be selected");
        assert_eq!(error.status, 503);
        assert_eq!(
            error.code,
            if pinned {
                "account_cooling"
            } else {
                "no_available_account"
            }
        );
        if rebuild_available {
            assert!(
                error.retry_after.is_none(),
                "verified rebuild must not wait for the exhausted owner"
            );
        } else {
            assert!(error.retry_after.unwrap() > Duration::from_secs(50));
        }
        assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
        assert!(host
            .gates
            .lock()
            .unwrap()
            .values()
            .all(|gate| gate.available_permits() == 1));
    }
}

#[tokio::test]
async fn native_queued_cooling_keeps_the_remaining_concurrency_wait_budget() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = native_test_routing_host();
    let collection = host.collection_override.as_mut().unwrap();
    collection.account_concurrency_wait_ms = 1_000;
    collection.max_retry_interval_ms = 0;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let request = native_test_route_request("queued-budget");
    let busy_a = native_select_route(&host, &caller, request.clone())
        .await
        .unwrap();
    let collection = host.collection().await.unwrap();
    let backup = host.account_overrides["pool-b"].clone();
    let (_, busy_b) = native_acquire_account_permit(&host, &collection, &[backup], true)
        .await
        .unwrap();
    let mut queued = Box::pin(native_select_route(&host, &caller, request));
    native_test_wait_for_account_queue(&host, &mut queued).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "pool-a",
        "gpt-fixture",
        503,
        None,
        false,
        Instant::now(),
    );
    drop(busy_a);
    // B stays busy. Reusing a fresh one-second wait would exceed this bound.
    let error = tokio::time::timeout(Duration::from_millis(600), queued)
        .await
        .expect("switching candidates restarted the wait budget")
        .err()
        .expect("busy backup must not be selected");
    assert_eq!(error.code, "account_busy");
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
    assert!(host
        .gates
        .lock()
        .unwrap()
        .get("pool-a:1")
        .is_none_or(|gate| gate.available_permits() == 1));
    drop(busy_b);
    assert!(host
        .gates
        .lock()
        .unwrap()
        .values()
        .all(|gate| gate.available_permits() == 1));
}

#[test]
fn native_quota_refresh_clears_only_earlier_quota_cooldowns_for_this_account() {
    let old = Instant::now();
    let query = old + Duration::from_secs(1);
    let newer = query + Duration::from_secs(1);
    let mut states = HashMap::new();
    native_mark_quota_exhausted(&mut states, "a", Some(Duration::from_secs(7200)), old);
    native_upsert_cooldown(&mut states, "a", "old-quota", 429, None, false, old);
    native_upsert_cooldown(&mut states, "a", "overload", 503, None, false, old);
    native_upsert_cooldown(&mut states, "a", "new-quota", 429, None, false, newer);
    native_mark_quota_exhausted(&mut states, "b", Some(Duration::from_secs(7200)), old);
    assert!(native_clear_quota_cooldowns(&mut states, "a", query));
    assert!(!states.contains_key(&native_cooldown_key("a", "", true)));
    assert!(!states.contains_key(&native_cooldown_key("a", "old-quota", false)));
    assert!(states.contains_key(&native_cooldown_key("a", "new-quota", false)));
    assert!(states.contains_key(&native_cooldown_key("a", "overload", false)));
    assert!(states.contains_key(&native_cooldown_key("b", "", true)));
    // An account-wide rejection received during the GET also wins.
    native_mark_quota_exhausted(&mut states, "b", None, newer);
    assert!(!native_clear_quota_cooldowns(&mut states, "b", query));
}

#[test]
fn native_quota_refresh_requires_positive_authoritative_allowance() {
    let now = 100;
    let started = Instant::now();
    let mut quota = CodexQuota {
        hourly_percentage: 80,
        hourly_window_present: Some(true),
        hourly_reset_time: Some(200),
        weekly_window_present: Some(false),
        raw_data: Some(serde_json::json!({"rate_limit":{"primary_window":{"used_percent":20},"secondary_window":null}})),
        hourly_window_minutes: Some(300),
        weekly_percentage: 0,
        weekly_window_minutes: None,
        weekly_reset_time: None,
        reset_credits_available: None,
        reset_credits: vec![],
        reset_credits_next_expires_at: None,
    };
    assert!(quota_refresh_confirms_usable_allowance(&quota, started, now));
    quota.raw_data.as_mut().unwrap()["rate_limit"].as_object_mut().unwrap().remove("secondary_window");
    assert!(!quota_refresh_confirms_usable_allowance(&quota, started, now));
    quota.raw_data.as_mut().unwrap()["rate_limit"]["secondary_window"] = serde_json::Value::Null;
    quota.hourly_reset_time = Some(now);
    assert!(!quota_refresh_confirms_usable_allowance(&quota, started, now));
    quota.hourly_reset_time = Some(200);
    quota.raw_data.as_mut().unwrap()["rate_limit"]["primary_window"]["used_percent"] = serde_json::json!(100);
    assert!(!quota_refresh_confirms_usable_allowance(&quota, started, now));
}
