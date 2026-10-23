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
        .entry(id.into())
        .or_default()
        .verified = true;
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
        native_acquire_account_permit(&host_b, &collection, &[account.clone(), other], true)
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
    assert!(native_bound_quota_reserve_allows(&collection, &account));
    confirm_gateway_quota_refresh(&account.id, account.quota.as_ref().unwrap(), Instant::now());
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
