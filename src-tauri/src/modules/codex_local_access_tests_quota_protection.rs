// Included by native_host_tests. Fixtures never read real credentials or send requests.
fn quota_protection_fixture(
    id: &str,
    remaining: i32,
    strict: bool,
) -> (CodexLocalAccessCollection, CodexAccount) {
    let mut account = native_test_account_with_quota(remaining, 90);
    account.id = id.into();
    quota_reserve_coordinator()
        .lock()
        .unwrap()
        .accounts
        .insert(id.into(), QuotaAccountRuntime::default());
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    let mut collection = native_test_collection();
    collection.max_account_concurrency = 0;
    collection.account_concurrency_wait_ms = 0;
    collection.bound_oauth_account_id = Some(account.id.clone());
    collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 10,
        weekly_percent: 10,
        strict,
        safety_margin_percent: 3,
    });
    (collection, account)
}

fn quota_protection_snapshot(remaining: f64, reset: i64) -> QuotaSnapshot {
    QuotaSnapshot {
        primary: Some(QuotaWindow {
            remaining_percent: remaining,
            window_minutes: Some(300),
            resets_at: Some(reset),
        }),
        secondary: None,
    }
}

#[test]
fn quota_protection_legacy_settings_always_apply_the_safety_margin() {
    let reserve: CodexLocalAccessQuotaReserve = serde_json::from_value(json!({
        "hourlyPercent":10,"weeklyPercent":10}))
    .unwrap();
    assert!(reserve.strict);
    assert_eq!(reserve.safety_margin_percent, 3);
    assert!(validate_bound_oauth_quota_reserve(
        Some(CodexLocalAccessQuotaReserve {
            safety_margin_percent: 0,
            ..reserve.clone()
        }),
        true
    )
    .is_err());
    assert_eq!(quota_reserve_cutoff(&reserve, 10), 13.0);
    assert_eq!(
        quota_reserve_cutoff(
            &CodexLocalAccessQuotaReserve {
                strict: false,
                ..reserve
            },
            10
        ),
        13.0
    );
}

#[test]
fn quota_protection_partial_fractional_snapshots_and_old_replies_are_safe() {
    let (collection, account) = quota_protection_fixture("quota-partial", 80, false);
    let earlier = Instant::now();
    observe_gateway_quota(&account.id, quota_protection_snapshot(13.25, 5000), earlier);
    assert!(native_bound_quota_reserve_allows(&collection, &account));
    observe_gateway_quota(
        &account.id,
        quota_protection_snapshot(12.75, 5000),
        Instant::now(),
    );
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    observe_gateway_quota(&account.id, quota_protection_snapshot(90.0, 5000), earlier);
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    // A slow query must not erase newer evidence received while it was running.
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), earlier);
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    // A later snapshot from a genuinely newer reset window may restore allowance.
    observe_gateway_quota(
        &account.id,
        quota_protection_snapshot(90.0, 6000),
        Instant::now(),
    );
    assert!(native_bound_quota_reserve_allows(&collection, &account));
}

#[test]
fn quota_protection_missing_reset_cannot_restore_allowance() {
    let (collection, account) = quota_protection_fixture("quota-unknown-reset", 80, false);
    observe_gateway_quota(&account.id, quota_protection_snapshot(9.0, 5000), Instant::now());
    let mut incomplete = quota_protection_snapshot(90.0, 5000);
    incomplete.primary.as_mut().unwrap().resets_at = None;
    observe_gateway_quota(&account.id, incomplete, Instant::now());
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(native_bound_quota_reserve_allows(&collection, &account));
}

#[tokio::test]
async fn quota_protection_is_atomic_across_native_hosts_and_prefers_another_account() {
    let (collection, account) = quota_protection_fixture("quota-cross-host", 18, false);
    let host_a = NativeHostBridge::default();
    let host_b = NativeHostBridge::default();
    let (_, first) =
        native_acquire_account_permit(&host_a, &collection, std::slice::from_ref(&account), true)
            .await
            .unwrap();
    let second =
        native_acquire_account_permit(&host_b, &collection, std::slice::from_ref(&account), true)
            .await;
    assert!(second.is_err_and(|error| error.code == "quota_reserved"));
    let mut other = account.clone();
    other.id = "quota-cross-host-alternative".into();
    let (index, permit) =
        native_acquire_affinity_permit(&host_b, &collection, &[account.clone(), other], true)
            .await
            .unwrap();
    assert_eq!(index, 1);
    drop(permit);
    // Without dispatch, dropping preparation must not invent pending consumption.
    drop(first);
    assert!(native_bound_quota_reserve_allows(&collection, &account));
}

#[test]
fn quota_protection_completion_requires_a_later_confirmed_query() {
    let (collection, account) = quota_protection_fixture("quota-settlement", 18, true);
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    let stale_query = Instant::now();
    note_gateway_quota_request(&account.id);
    drop(permit);
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), stale_query);
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    // Simulate settlement grace without sleeping or using a real network.
    quota_reserve_coordinator()
        .lock()
        .unwrap()
        .accounts
        .get_mut(&account.id)
        .unwrap()
        .completed_at = Some(Instant::now() - Duration::from_secs(2));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(native_bound_quota_reserve_allows(&collection, &account));
}

#[tokio::test]
async fn quota_protection_rechecks_after_waiting_for_a_concurrency_slot() {
    let (mut collection, account) = quota_protection_fixture("quota-queued", 80, false);
    collection.max_account_concurrency = 1;
    collection.account_concurrency_wait_ms = 500;
    let host = Arc::new(NativeHostBridge::default());
    let (_, held) =
        native_acquire_account_permit(&host, &collection, std::slice::from_ref(&account), true)
            .await
            .unwrap();
    let (queued_host, queued_collection, queued_account) =
        (host.clone(), collection.clone(), account.clone());
    let queued = tokio::spawn(async move {
        native_acquire_account_permit(&queued_host, &queued_collection, &[queued_account], true)
            .await
    });
    tokio::task::yield_now().await;
    observe_gateway_quota(
        &account.id,
        quota_protection_snapshot(9.0, 5000),
        Instant::now(),
    );
    drop(held);
    assert!(queued
        .await
        .unwrap()
        .is_err_and(|error| error.code == "quota_reserved"));
}

#[test]
fn quota_protection_adaptive_polling_uses_the_tighter_window() {
    let (_, mut account) = quota_protection_fixture("quota-polling", 100, false);
    {
        let mut runtime = quota_reserve_coordinator().lock().unwrap();
        let state = runtime.accounts.get_mut(&account.id).unwrap();
        state.primary = None;
        state.secondary = None;
    }
    account.quota.as_mut().unwrap().weekly_percentage = 100;
    assert_eq!(
        bound_oauth_dynamic_refresh_interval(Some(&account)),
        Duration::from_secs(30)
    );
    account.quota.as_mut().unwrap().weekly_percentage = 20;
    assert_eq!(
        bound_oauth_dynamic_refresh_interval(Some(&account)),
        Duration::from_secs(6)
    );
    account.quota.as_mut().unwrap().hourly_percentage = 10;
    assert_eq!(
        bound_oauth_dynamic_refresh_interval(Some(&account)),
        Duration::from_secs(5)
    );
    account.usage_updated_at = None;
    assert_eq!(
        bound_oauth_dynamic_refresh_interval(Some(&account)),
        Duration::from_secs(5)
    );
}

#[test]
fn quota_protection_final_send_fence_rechecks_after_preparation() {
    let (collection, account) = quota_protection_fixture("quota-final-fence", 18, true);
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    let reserve = collection.bound_oauth_quota_reserve.as_ref();
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_ok());
    observe_gateway_quota(
        &account.id,
        quota_protection_snapshot(12.5, 5000),
        Instant::now(),
    );
    assert!(check_gateway_quota_before_send(&account.id, false, reserve)
        .is_err_and(|error| error.code == "quota_reserved"));
    drop(permit);
}

fn quota_protection_expire_runtime_windows(account: &CodexAccount, primary: bool, secondary: bool) {
    let expired = Instant::now() - Duration::from_secs(181);
    let mut runtime = quota_reserve_coordinator().lock().unwrap();
    let state = runtime.accounts.get_mut(&account.id).unwrap();
    if primary {
        let window = state.primary.as_mut().unwrap();
        window.observed_at = expired;
        window.request_started = expired;
    }
    if secondary {
        let window = state.secondary.as_mut().unwrap();
        window.observed_at = expired;
        window.request_started = expired;
    }
}

#[test]
fn quota_protection_fresh_display_cannot_replace_expired_live_evidence() {
    for (id, primary_expired, secondary_expired) in [
        ("quota-expired-primary", true, false),
        ("quota-expired-secondary", false, true),
        ("quota-expired-both", true, true),
    ] {
        let (collection, account) = quota_protection_fixture(id, 80, true);
        quota_protection_expire_runtime_windows(&account, primary_expired, secondary_expired);
        let reserve = collection.bound_oauth_quota_reserve.as_ref();
        assert!(!native_bound_quota_reserve_allows(&collection, &account));
        assert!(acquire_quota_admission(&collection, &account)
            .is_err_and(|error| error.code == "quota_reserved"));
        assert!(check_gateway_quota_before_send(&account.id, false, reserve)
            .is_err_and(|error| error.code == "quota_reserved"));

        confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
        let permit = acquire_quota_admission(&collection, &account).unwrap();
        assert!(
            check_gateway_quota_before_send(&account.id, false, reserve).is_ok(),
            "confirmed live quota for {id} must be available to the send fence"
        );
        observe_gateway_quota(&account.id, quota_protection_snapshot(12.0, 5000), Instant::now());
        assert!(check_gateway_quota_before_send(&account.id, false, reserve)
            .is_err_and(|error| error.code == "quota_reserved"));
        drop(permit);
    }
}

#[test]
fn quota_protection_slow_query_cannot_freshen_an_empty_cache() {
    let (collection, account) = quota_protection_fixture("quota-slow-empty", 80, true);
    {
        let mut runtime = quota_reserve_coordinator().lock().unwrap();
        *runtime.accounts.get_mut(&account.id).unwrap() = QuotaAccountRuntime::default();
    }
    let slow_started = Instant::now() - Duration::from_secs(181);
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), slow_started);
    let reserve = collection.bound_oauth_quota_reserve.as_ref();
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));
    assert!(check_gateway_quota_before_send(&account.id, false, reserve)
        .is_err_and(|error| error.code == "quota_reserved"));
    let status = build_quota_reserve_status(&collection).unwrap();
    assert!(!status.snapshot_fresh && status.blocked);

    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_ok());
    let status = build_quota_reserve_status(&collection).unwrap();
    assert!(status.snapshot_fresh && !status.blocked);
    drop(permit);
}

#[test]
fn quota_protection_slow_query_and_imported_display_cannot_override_newer_evidence() {
    let (collection, mut account) = quota_protection_fixture("quota-slow-import", 80, true);
    let slow_started = Instant::now() - Duration::from_secs(182);
    quota_reserve_coordinator().lock().unwrap().accounts.get_mut(&account.id).unwrap().primary = None;
    observe_gateway_quota(&account.id, quota_protection_snapshot(12.0, 5000), slow_started);
    quota_protection_expire_runtime_windows(&account, true, false);
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), slow_started);
    // Simulate an import re-stamping the display value without a live query.
    account.usage_updated_at = Some(chrono::Utc::now().timestamp());
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));
    let reserve = collection.bound_oauth_quota_reserve.as_ref();
    assert!(check_gateway_quota_before_send(&account.id, false, reserve)
        .is_err_and(|error| error.code == "quota_reserved"));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_ok());
    drop(permit);
}

fn quota_protection_reversed_reset_fixture(
    id: &str,
    primary: bool,
) -> (CodexLocalAccessCollection, CodexAccount) {
    let (collection, mut account) = quota_protection_fixture(id, 80, true);
    let quota = account.quota.as_mut().unwrap();
    if primary {
        quota.hourly_reset_time = Some(5000);
    } else {
        quota.weekly_reset_time = Some(5000);
    }
    let mut runtime = quota_reserve_coordinator().lock().unwrap();
    let state = runtime.accounts.get_mut(&account.id).unwrap();
    state.completed_at = Some(Instant::now() - Duration::from_secs(2));
    state.awaiting_confirmation = true;
    let window = if primary { &mut state.primary } else { &mut state.secondary };
    let window = window.as_mut().unwrap();
    window.observed_at = Instant::now() - Duration::from_secs(181);
    window.request_started = window.observed_at;
    let value = window.value.as_mut().unwrap();
    value.remaining_percent = 12.0;
    value.resets_at = Some(8000);
    drop(runtime);
    (collection, account)
}

#[test]
fn quota_protection_complete_settled_query_recovers_stale_reversed_reset() {
    for (id, primary) in [("quota-reset-primary", true), ("quota-reset-secondary", false)] {
        let (collection, account) = quota_protection_reversed_reset_fixture(id, primary);
        assert!(acquire_quota_admission(&collection, &account).is_err());
        confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
        {
            let runtime = quota_reserve_coordinator().lock().unwrap();
            let state = &runtime.accounts[&account.id];
            let window = if primary { &state.primary } else { &state.secondary };
            assert_eq!(window.as_ref().unwrap().value.as_ref().unwrap().resets_at, Some(5000));
            assert!(!state.awaiting_confirmation);
        }
        let permit = acquire_quota_admission(&collection, &account).unwrap();
        assert!(check_gateway_quota_before_send(&account.id, false, collection.bound_oauth_quota_reserve.as_ref()).is_ok());
        drop(permit);
    }
}

#[test]
fn quota_protection_reversed_reset_keeps_newer_or_unsettled_evidence() {
    for case in ["fresh", "newer-observation", "in-flight", "unsettled", "partial", "late", "event"] {
        let id = format!("quota-reset-guard-{case}");
        let (collection, account) = quota_protection_reversed_reset_fixture(&id, true);
        let mut started = Instant::now();
        {
            let mut runtime = quota_reserve_coordinator().lock().unwrap();
            let state = runtime.accounts.get_mut(&account.id).unwrap();
            match case {
                "fresh" => {
                    let window = state.primary.as_mut().unwrap();
                    window.observed_at = Instant::now();
                    window.request_started = window.observed_at;
                    started = Instant::now();
                }
                "newer-observation" => {
                    state.primary.as_mut().unwrap().observed_at = Instant::now() - Duration::from_secs(1);
                    state.completed_at = None;
                    started = Instant::now() - Duration::from_secs(2);
                }
                "in-flight" => state.in_flight = 1,
                "unsettled" => state.completed_at = Some(Instant::now()),
                "late" => started = Instant::now() - Duration::from_secs(181),
                _ => {}
            }
        }
        if case == "event" {
            observe_gateway_quota(&account.id, quota_protection_snapshot(80.0, 5000), started);
        } else {
            let mut quota = account.quota.as_ref().unwrap().clone();
            if case == "partial" {
                quota.weekly_window_present = Some(false);
                quota.raw_data = Some(json!({"rate_limit":{
                    "primary_window":{"used_percent":20.0}}}));
            }
            confirm_gateway_quota_refresh(&account.id, &quota, started);
        }
        {
            let runtime = quota_reserve_coordinator().lock().unwrap();
            let window = runtime.accounts[&account.id].primary.as_ref().unwrap();
            assert_eq!(window.value.as_ref().unwrap().resets_at, Some(8000), "case={case}");
            assert_eq!(window.value.as_ref().unwrap().remaining_percent, 12.0, "case={case}");
        }
        assert!(acquire_quota_admission(&collection, &account).is_err(), "case={case}");
    }
}

#[test]
fn quota_protection_partial_query_cannot_settle_a_near_window() {
    let (collection, account) = quota_protection_fixture("quota-partial-settlement", 18, true);
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    note_gateway_quota_request(&account.id);
    drop(permit);
    let (verified_at, verified_started) = {
        let mut runtime = quota_reserve_coordinator().lock().unwrap();
        let state = runtime.accounts.get_mut(&account.id).unwrap();
        assert!(state.awaiting_confirmation);
        state.completed_at = Some(Instant::now() - Duration::from_secs(2));
        (state.verified_at, state.verified_query_started_at)
    };

    // The parser normalizes a missing primary window to Some(false), even
    // though the original response contains no explicit absence marker.
    let mut partial = account.quota.as_ref().unwrap().clone();
    partial.hourly_percentage = 100;
    partial.hourly_window_present = Some(false);
    partial.raw_data = Some(json!({"rate_limit":{
        "secondary_window":{"used_percent":10.0}}}));
    confirm_gateway_quota_refresh(&account.id, &partial, Instant::now());
    {
        let runtime = quota_reserve_coordinator().lock().unwrap();
        let state = &runtime.accounts[&account.id];
        assert!(state.awaiting_confirmation);
        assert_eq!(state.verified_at, verified_at);
        assert_eq!(state.verified_query_started_at, verified_started);
        assert_eq!(state.primary.as_ref().unwrap().value.as_ref().unwrap().remaining_percent, 18.0);
    }
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));
    assert!(check_gateway_quota_before_send(&account.id, false, collection.bound_oauth_quota_reserve.as_ref())
        .is_err_and(|error| error.code == "quota_reserved"));

    // A partial high primary value also cannot relax the older near-window
    // evidence while settlement awaits a complete result.
    partial.hourly_window_present = Some(true);
    partial.hourly_percentage = 80;
    partial.weekly_window_present = Some(false);
    partial.raw_data = Some(json!({"rate_limit":{
        "primary_window":{"used_percent":20.0}}}));
    confirm_gateway_quota_refresh(&account.id, &partial, Instant::now());
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));

    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    assert!(check_gateway_quota_before_send(&account.id, false, collection.bound_oauth_quota_reserve.as_ref()).is_ok());
    drop(permit);
}

#[test]
fn quota_protection_explicit_null_is_complete_absent_evidence() {
    let (collection, account) = quota_protection_fixture("quota-explicit-absent", 80, true);
    *quota_reserve_coordinator().lock().unwrap().accounts.get_mut(&account.id).unwrap() =
        QuotaAccountRuntime::default();
    let mut quota = account.quota.as_ref().unwrap().clone();
    quota.hourly_percentage = 100;
    quota.hourly_window_present = Some(false);
    quota.raw_data = Some(json!({"rate_limit":{
        "primary_window":null,"secondary_window":{"used_percent":10.0}}}));
    confirm_gateway_quota_refresh(&account.id, &quota, Instant::now());
    let permit = acquire_quota_admission(&collection, &account).unwrap();
    assert!(check_gateway_quota_before_send(&account.id, false, collection.bound_oauth_quota_reserve.as_ref()).is_ok());
    drop(permit);
}

#[test]
fn quota_protection_missing_window_and_pending_confirmation_fail_closed() {
    let (collection, mut account) = quota_protection_fixture("quota-live-missing-window", 80, true);
    quota_reserve_coordinator().lock().unwrap().accounts.get_mut(&account.id).unwrap().primary = None;
    account.quota.as_mut().unwrap().hourly_percentage = -1;
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));

    let (collection, account) = quota_protection_fixture("quota-live-unverified", 80, true);
    quota_reserve_coordinator()
        .lock().unwrap().accounts.get_mut(&account.id).unwrap().verified = false;
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));

    let (collection, account) = quota_protection_fixture("quota-live-awaiting", 18, true);
    quota_reserve_coordinator()
        .lock().unwrap().accounts.get_mut(&account.id).unwrap().awaiting_confirmation = true;
    assert!(acquire_quota_admission(&collection, &account)
        .is_err_and(|error| error.code == "quota_reserved"));
}

#[test]
fn quota_protection_failure_backoff_respects_upstream_retry_after() {
    assert_eq!(
        quota_refresh_failure_backoff(1, None),
        Duration::from_secs(10)
    );
    assert_eq!(
        quota_refresh_failure_backoff(10, None),
        Duration::from_secs(120)
    );
    assert_eq!(
        quota_refresh_failure_backoff(1, Some("HTTP 429 [retry_after_seconds:300]")),
        Duration::from_secs(300)
    );
    assert_eq!(
        quota_refresh_failure_backoff(1, Some("[retry_after_seconds:invalid]")),
        Duration::from_secs(10)
    );
}

#[test]
fn quota_protection_legacy_false_cold_start_requires_live_confirmation() {
    let (collection, account) = quota_protection_fixture("quota-cold-start", 80, false);
    quota_reserve_coordinator()
        .lock()
        .unwrap()
        .accounts
        .get_mut(&account.id)
        .unwrap()
        .verified = false;
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    assert!(acquire_quota_admission(&collection, &account).is_err());
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(native_bound_quota_reserve_allows(&collection, &account));
    assert!(acquire_quota_admission(&collection, &account).is_ok());
}

#[test]
fn quota_protection_query_keeps_precision_beyond_the_display_integer() {
    let (collection, mut account) = quota_protection_fixture("quota-query-precision", 13, false);
    account.quota.as_mut().unwrap().raw_data = Some(json!({"rate_limit":{
        "primary_window":{"used_percent":86.75}}}));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(native_bound_quota_reserve_allows(&collection, &account));
    let runtime = quota_reserve_coordinator().lock().unwrap();
    let primary = runtime.accounts[&account.id].primary.as_ref().unwrap();
    assert_eq!(primary.value.as_ref().unwrap().remaining_percent, 13.25);
}

#[test]
fn quota_protection_critical_zone_stops_already_prepared_parallel_requests() {
    let (collection, account) = quota_protection_fixture("quota-prepared-parallel", 80, true);
    let first = acquire_quota_admission(&collection, &account).unwrap();
    let second = acquire_quota_admission(&collection, &account).unwrap();
    let reserve = collection.bound_oauth_quota_reserve.as_ref();
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_ok());
    observe_gateway_quota(
        &account.id,
        quota_protection_snapshot(18.0, 5000),
        Instant::now(),
    );
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_err());
    drop(second);
    assert!(check_gateway_quota_before_send(&account.id, false, reserve).is_ok());
    drop(first);
}

#[test]
fn quota_protection_timeout_blocks_old_evidence_and_recovers_on_verification() {
    let (collection, account) = quota_protection_fixture("quota-timeout", 80, true);
    let started = Instant::now();
    note_gateway_quota_refresh_failure(&account.id, started);
    assert!(!native_bound_quota_reserve_allows(&collection, &account));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    assert!(native_bound_quota_reserve_allows(&collection, &account));
    note_gateway_quota_refresh_failure(&account.id, started);
    assert!(
        native_bound_quota_reserve_allows(&collection, &account),
        "slow metadata must not invalidate verified quota"
    );
}

#[test]
fn quota_protection_status_uses_live_memory_without_requiring_an_account_file() {
    let (collection, account) = quota_protection_fixture("quota-status-without-file", 80, true);
    {
        let mut runtime = quota_reserve_coordinator().lock().unwrap();
        let state = runtime.accounts.get_mut(&account.id).unwrap();
        *state = QuotaAccountRuntime::default();
    }
    let cold = build_quota_reserve_status(&collection).unwrap();
    assert!(cold.blocked && cold.awaiting_confirmation);
    assert!(!cold.snapshot_fresh);
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
    let ready = build_quota_reserve_status(&collection).unwrap();
    assert!(ready.snapshot_fresh);
    assert!(!ready.blocked && !ready.awaiting_confirmation);
    observe_gateway_quota(&account.id, quota_protection_snapshot(12.0, 5000), Instant::now());
    assert!(build_quota_reserve_status(&collection).unwrap().blocked);
}
