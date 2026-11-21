// Synthetic OAuth metadata and local fixtures only.

fn native_quota_continuation_fixture(
    reset_at: Option<i64>,
) -> (
    tempfile::TempDir,
    NativeHostBridge,
    native_contract::RouteRequest,
) {
    use base64::Engine;
    let root = tempfile::tempdir().unwrap();
    let mut host = native_test_routing_host();
    // Keep speed/profile lookups inside this test's own empty directory.
    host.profile_dir = Some(root.path().to_path_buf());
    let collection = host.collection_override.as_mut().unwrap();
    collection.max_retry_interval_ms = 0;
    collection.bound_oauth_quota_reserve = None;
    let model = api_service_routable_codex_model_ids()
        .into_iter()
        .find(|model| !native_is_reserve_model(model))
        .unwrap();
    let mut owner = native_test_account_with_quota(0, 70);
    owner.id = "pool-a".into();
    owner.plan_type = Some("plus".into());
    owner.egress_proxy_disabled = true;
    owner.tokens.refresh_token = None;
    owner.tokens.access_token = format!(
        "e30.{}.fixture",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"exp": chrono::Utc::now().timestamp() + 86400}).to_string())
    );
    owner.quota.as_mut().unwrap().hourly_reset_time = reset_at;
    host.account_overrides.insert(owner.id.clone(), owner);
    host.account_overrides
        .get_mut("pool-b")
        .unwrap()
        .api_model_catalog = vec![model.clone()];
    let mut request = native_test_route_request("quota-snapshot-thread");
    request.model = model;
    request.preferred_account_id = Some("pool-a".into());
    (root, host, request)
}

#[tokio::test]
async fn native_pool_quota_floor_keeps_other_members_available_and_reports_a_protected_owner() {
    let (_root, mut host, request) = native_quota_continuation_fixture(None);
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    host.collection_override.as_mut().unwrap().account_quota_floors.insert("pool-a".into(), 5);
    let quota = host.account_overrides.get_mut("pool-a").unwrap().quota.as_mut().unwrap();
    quota.hourly_percentage = 6;
    quota.raw_data = Some(json!({"credits": {"balance": 8}}));
    assert_eq!(native_select_route(&host, &caller, request.clone()).await.unwrap().account_id, "pool-a");

    host.account_overrides.get_mut("pool-a").unwrap().quota.as_mut().unwrap().hourly_percentage = 5;
    let error = native_select_route(&host, &caller, request.clone()).await.err().unwrap();
    assert_eq!((error.status, error.code), (429, "quota_reserved"));
    assert!(error.retry_after.is_none());
    let mut unpinned = request.clone();
    unpinned.preferred_account_id = None;
    assert_eq!(native_select_route(&host, &caller, unpinned).await.unwrap().account_id, "pool-b");

    host.collection_override.as_mut().unwrap().account_quota_floors.clear();
    assert_eq!(native_select_route(&host, &caller, request).await.unwrap().account_id, "pool-a");
}

#[test]
fn native_pool_quota_floor_uses_present_active_windows_and_ignores_unknown_values() {
    let (_root, mut host, _) = native_quota_continuation_fixture(None);
    let collection = host.collection_override.as_mut().unwrap();
    collection.account_quota_floors.insert("pool-a".into(), 5);
    let account = host.account_overrides.get_mut("pool-a").unwrap();
    let now = now_ms();
    let quota = account.quota.as_mut().unwrap();
    quota.hourly_percentage = 4;
    quota.hourly_window_present = Some(false);
    quota.weekly_window_present = Some(true);
    assert!(!pool_account_quota_cooldown(collection, account, now).unwrap().active(now));

    let quota = account.quota.as_mut().unwrap();
    quota.weekly_percentage = 4;
    quota.weekly_reset_time = Some(now / 1000 + 60);
    assert!(pool_account_quota_cooldown(collection, account, now).unwrap().active(now));
    assert!(!pool_account_quota_cooldown(collection, account, now + 60_000).unwrap().active(now + 60_000));
    assert!(native_pool_quota_floor_error(collection, account, CODEX_GPT_RESERVE_MODEL_ID).is_none());

    account.quota = None;
    assert!(pool_account_quota_cooldown(collection, account, now).is_none());
}

#[tokio::test]
async fn native_quota_snapshot_owner_naturally_recovers_within_wait_budget() {
    let reset_at = chrono::Utc::now().timestamp() + 2;
    let (_root, mut host, request) = native_quota_continuation_fixture(Some(reset_at));
    host.collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 3_000;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    assert!(now_ms() < reset_at * 1000);
    assert!(host.cooldowns.lock().unwrap().is_empty());
    let selected = tokio::time::timeout(
        Duration::from_secs(5),
        native_select_route(&host, &caller, request),
    )
    .await
    .expect("the existing preparation wait remains bounded")
    .expect("a known near-term quota reset must not become a terminal binding error");
    assert_eq!(selected.account_id, "pool-a");
    assert!(
        now_ms() >= reset_at * 1000,
        "never route before the quota reset"
    );
    assert_eq!(
        host.account_overrides["pool-a"]
            .quota
            .as_ref()
            .unwrap()
            .hourly_percentage,
        0,
        "the test must not manually mark the owner available"
    );
    assert!(host.cooldowns.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_snapshot_long_reset_is_retryable_without_selecting_another_owner() {
    let reset_at = chrono::Utc::now().timestamp() + 60;
    let (_root, host, request) = native_quota_continuation_fixture(Some(reset_at));
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (503, "account_cooling"));
    let wait = error
        .retry_after
        .expect("known quota reset supplies its delay");
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    assert!(host.gates.lock().unwrap().is_empty());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    let retry_wait = native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(61))
        .await
        .unwrap()
        .expect("credential rounds observe the same known quota window");
    assert!(retry_wait > Duration::from_secs(58));
}

#[tokio::test]
async fn native_quota_snapshot_unknown_reset_does_not_invent_a_recovery_time() {
    let (_root, mut host, request) = native_quota_continuation_fixture(None);
    host.collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 3_000;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(
        (error.status, error.code),
        (409, "account_binding_unavailable")
    );
    assert!(error.retry_after.is_none());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(120))
            .await
            .unwrap()
            .is_none()
    );
    assert!(host.gates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_snapshot_does_not_hide_static_owner_ineligibility() {
    for denial in ["reauthorize", "removed", "model", "excluded", "paused"] {
        let (_root, mut host, mut request) =
            native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
        match denial {
            "reauthorize" => {
                host.account_overrides
                    .get_mut("pool-a")
                    .unwrap()
                    .requires_reauth = true
            }
            "removed" => {
                host.collection_override.as_mut().unwrap().account_ids = vec!["pool-b".into()]
            }
            "model" => {
                host.collection_override
                    .as_mut()
                    .unwrap()
                    .account_model_rules = serde_json::from_value(json!([{
                    "accountId":"pool-a", "excludedModels":[request.model.clone()]
                }]))
                .unwrap();
            }
            "excluded" => request.excluded_account_ids = vec!["pool-a".into()],
            "paused" => {
                host.collection_override
                    .as_mut()
                    .unwrap()
                    .paused_account_ids = vec!["pool-a".into()];
            }
            _ => unreachable!(),
        }
        let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
        let error = native_select_route(&host, &caller, request.clone())
            .await
            .err()
            .unwrap();
        let expected = if denial == "paused" {
            (503, "service_member_paused")
        } else {
            (409, "account_binding_unavailable")
        };
        assert_eq!((error.status, error.code), expected, "{denial}");
        assert!(error.retry_after.is_none(), "{denial}");
        assert!(host.gates.lock().unwrap().is_empty(), "{denial}");
        assert!(
            native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(120))
                .await
                .unwrap()
                .is_none(),
            "ineligible or excluded owners must not supply retry delays: {denial}"
        );
    }
}

#[tokio::test]
async fn native_quota_snapshot_wait_covers_both_quota_windows_and_memory_cooling() {
    let now = chrono::Utc::now().timestamp();
    let (_root, mut host, request) = native_quota_continuation_fixture(Some(now + 60));
    let quota = host
        .account_overrides
        .get_mut("pool-a")
        .unwrap()
        .quota
        .as_mut()
        .unwrap();
    quota.weekly_percentage = 0;
    quota.weekly_reset_time = Some(now + 120);
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "account_cooling");
    assert!(error.retry_after.unwrap() > Duration::from_secs(118));
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", &request.model, false),
        NativeCooldown {
            non_quota_failures: 0,
            recorded_at: Instant::now(),
            until: Instant::now() + Duration::from_secs(180),
            quota: true,
            backoff_level: 0,
            reason: "quota",
        },
    );
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "account_cooling");
    assert!(error.retry_after.unwrap() > Duration::from_secs(178));
    assert!(
        native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::from_secs(150))
            .await
            .unwrap()
            .is_none()
    );
    let wait = native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(181))
        .await
        .unwrap()
        .unwrap();
    assert!(wait > Duration::from_secs(178));
}

#[tokio::test]
async fn native_quota_snapshot_wait_is_cancelable_without_holding_selection_locks() {
    let (_root, mut host, request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 5));
    host.collection_override
        .as_mut()
        .unwrap()
        .max_retry_interval_ms = 6_000;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    assert!(tokio::time::timeout(
        Duration::from_millis(20),
        native_select_route(&host, &caller, request.clone()),
    )
    .await
    .is_err());
    assert!(host.cooldowns.try_lock().is_ok());
    let mut unrelated = request;
    unrelated.preferred_account_id = None;
    unrelated.session_id = Some("other-quota-thread".into());
    unrelated.thread_id = unrelated.session_id.clone();
    let selected = native_select_route(&host, &caller, unrelated)
        .await
        .unwrap();
    assert_eq!(selected.account_id, "pool-b");
}

#[tokio::test]
async fn native_quota_snapshot_keeps_reserve_model_and_usable_credit_exceptions() {
    for reserve_model in [false, true] {
        let (_root, mut host, mut request) =
            native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
        let owner = host.account_overrides.get_mut("pool-a").unwrap();
        if reserve_model {
            request.model = "gpt-reserve".into();
            owner.quota.as_mut().unwrap().raw_data =
                native_reserve_account(true).quota.unwrap().raw_data;
        } else {
            owner.quota.as_mut().unwrap().raw_data = Some(json!({"credits":{"remaining":10}}));
        }
        let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
        let wait = native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(wait, Some(Duration::ZERO));
        let selected = native_select_route(&host, &caller, request).await.unwrap();
        assert_eq!(selected.account_id, "pool-a");
    }
}

#[tokio::test]
async fn native_oauth_binding_keeps_eligible_pool_member_and_continuation() {
    let (_root, mut host, continuation) = native_quota_continuation_fixture(None);
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .quota
        .as_mut()
        .unwrap()
        .hourly_percentage = 75;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut fresh = continuation.clone();
    fresh.preferred_account_id = None;
    fresh.session_id = Some("new-bound-oauth-thread".into());
    fresh.thread_id = fresh.session_id.clone();
    // Exclude the relay for this attempt so it cannot hide a dropped OAuth member.
    fresh.excluded_account_ids.push("pool-b".into());

    for bound_account in [None, Some("pool-a".to_string())] {
        let collection = host.collection_override.as_mut().unwrap();
        collection.bound_oauth_account_id = bound_account;
        collection.session_affinity = false;

        let route = native_select_route(&host, &caller, fresh.clone()).await.unwrap();
        assert_eq!(route.account_id, "pool-a");
        drop(route);
        let route = native_select_route(&host, &caller, continuation.clone()).await.unwrap();
        assert_eq!(route.account_id, "pool-a");
    }
}

#[tokio::test]
async fn native_quota_snapshot_does_not_bypass_bound_oauth_reservation() {
    let (_root, mut host, mut request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    let owner_id = "quota-snapshot-reserved-continuation-owner";
    let mut owner = host.account_overrides.remove("pool-a").unwrap();
    owner.id = owner_id.into();
    quota_reserve_coordinator()
        .lock()
        .unwrap()
        .accounts
        .insert(owner_id.into(), QuotaAccountRuntime::default());
    confirm_gateway_quota_refresh(owner_id, owner.quota.as_ref().unwrap(), Instant::now());
    host.account_overrides.insert(owner_id.into(), owner);
    request.preferred_account_id = Some(owner_id.into());
    let collection = host.collection_override.as_mut().unwrap();
    collection.account_ids = vec![owner_id.into(), "pool-b".into()];
    collection.bound_oauth_account_id = Some(owner_id.into());
    collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 10,
        strict: false,
        safety_margin_percent: 3,
    });
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route_with_owner_wait(&host, &caller, request.clone(), true)
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (429, "quota_reserved"));
    assert_eq!(error.response_error.unwrap()["reason"], "reserve_threshold");
    assert_eq!(error.local_rejected_account_id.as_deref(), Some(owner_id));
    assert!(error.upstream_not_sent);
    assert!(host.gates.lock().unwrap().is_empty());

    let owner = host.account_overrides.get_mut(owner_id).unwrap();
    let quota = owner.quota.as_mut().unwrap();
    quota.hourly_percentage = 75;
    confirm_gateway_quota_refresh(owner_id, quota, Instant::now());
    let route = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(route.account_id, owner_id);
}

#[tokio::test]
async fn native_quota_snapshot_disable_cooling_does_not_disable_real_quota() {
    let (_root, mut host, request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    host.collection_override.as_mut().unwrap().disable_cooling = true;
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", &request.model, false),
        NativeCooldown {
            non_quota_failures: 0,
            recorded_at: Instant::now(),
            until: Instant::now() + Duration::from_secs(180),
            quota: true,
            backoff_level: 0,
            reason: "quota",
        },
    );
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (503, "account_cooling"));
    let wait = error.retry_after.unwrap();
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    let wait = native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(61))
        .await
        .unwrap()
        .unwrap();
    assert!(wait > Duration::from_secs(58));
    assert!(host.gates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_snapshot_unknown_reset_is_not_replaced_by_finite_memory_cooling() {
    let (_root, host, request) = native_quota_continuation_fixture(None);
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", &request.model, false),
        NativeCooldown {
            non_quota_failures: 0,
            recorded_at: Instant::now(),
            until: Instant::now() + Duration::from_secs(60),
            quota: false,
            backoff_level: 0,
            reason: "transient_upstream",
        },
    );
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(
        (error.status, error.code),
        (409, "account_binding_unavailable")
    );
    assert!(error.retry_after.is_none());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(120))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn native_quota_snapshot_unpinned_pool_wait_uses_earliest_eligible_account() {
    let now = chrono::Utc::now().timestamp();
    let (_root, mut host, mut request) = native_quota_continuation_fixture(Some(now + 120));
    let mut other = host.account_overrides["pool-a"].clone();
    other.id = "pool-b".into();
    other.quota.as_mut().unwrap().hourly_reset_time = Some(now + 60);
    host.account_overrides.insert(other.id.clone(), other);
    request.preferred_account_id = None;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (429, "usage_limit_reached"));
    let wait = error.retry_after.unwrap();
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    let wait = native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(61))
        .await
        .unwrap()
        .unwrap();
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    assert!(host.gates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_error_uses_the_accounts_mapped_model_window() {
    let reset = chrono::Utc::now().timestamp() + 60;
    let (_root, mut host, mut request) = native_quota_continuation_fixture(Some(reset));
    let account = host.account_overrides.get_mut("pool-a").unwrap();
    account.api_model_mappings = serde_json::from_value(json!([
        {"client_model":"reserve-mapped","upstream_model":"gpt-reserve"}
    ])).unwrap();
    let quota = account.quota.as_mut().unwrap();
    quota.raw_data = native_reserve_account(true).quota.unwrap().raw_data;
    quota.hourly_percentage = 75;
    let mut key = native_test_key();
    key.enabled = true;
    key.inherit_account_pool = Some(false);
    key.account_ids = vec!["pool-a".into()];
    key.model_routing = Some(serde_json::from_value(json!({
        "defaultRoute":"oauth", "failurePolicy":"strict", "routes":[]
    })).unwrap());
    let caller = native_test_caller(&key.id);
    host.collection_override.as_mut().unwrap().api_keys = vec![key];
    request.model = "reserve-mapped".into();
    request.preferred_account_id = None;
    request.requires_permit = false;
    let route = native_select_route(&host, &caller, request.clone()).await.unwrap();
    assert_eq!(route.model, "gpt-reserve");

    native_mark_quota_exhausted_at_key(
        &mut host.cooldowns.lock().unwrap(),
        native_oauth_quota_key("pool-a", "gpt-reserve"),
        Some(Duration::from_secs(60)),
        Instant::now(),
    );
    let error = native_select_route(&host, &caller, request.clone()).await.err().unwrap();
    assert_eq!((error.status, error.code), (429, "usage_limit_reached"));
    assert!(error.retry_after.is_some_and(|wait| wait > Duration::from_secs(58)));

    host.cooldowns.lock().unwrap().clear();
    host.account_overrides.get_mut("pool-a").unwrap().quota.as_mut().unwrap().hourly_percentage = 0;
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(), "pool-a", "gpt-reserve", 503, None, false, Instant::now(),
    );
    let error = native_select_route(&host, &caller, request).await.err().unwrap();
    assert_eq!((error.status, error.code), (503, "no_available_account"));
}

#[tokio::test]
async fn native_quota_snapshot_unknown_pool_reset_is_explicit_without_invented_retry() {
    let (_root, mut host, mut request) = native_quota_continuation_fixture(None);
    let mut second = host.account_overrides["pool-a"].clone();
    second.id = "pool-b".into();
    host.account_overrides.insert(second.id.clone(), second);
    request.preferred_account_id = None;
    let error = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await.err().unwrap();
    assert_eq!((error.status, error.code), (429, "usage_limit_reached"));
    assert!(error.retry_after.is_none());
    assert_eq!(error.response_error.as_ref().unwrap()["code"], "usage_limit_reached");
    assert!(host.gates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_snapshot_mixed_transient_pool_does_not_claim_every_account_exhausted() {
    let (_root, host, mut request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    request.preferred_account_id = None;
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(), "pool-b", &request.model,
        503, None, false, Instant::now(),
    );
    let error = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await.err().unwrap();
    assert_eq!((error.status, error.code), (503, "no_available_account"));
    assert!(error.retry_after.is_some());
}

#[tokio::test]
async fn native_quota_snapshot_unpinned_known_reset_keeps_existing_bounded_wait() {
    let reset = chrono::Utc::now().timestamp() + 2;
    let (_root, mut host, mut request) = native_quota_continuation_fixture(Some(reset));
    let mut second = host.account_overrides["pool-a"].clone();
    second.id = "pool-b".into();
    host.account_overrides.insert(second.id.clone(), second);
    request.preferred_account_id = None;
    host.collection_override.as_mut().unwrap().max_retry_interval_ms = 3000;
    let route = tokio::time::timeout(
        Duration::from_secs(5),
        native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request),
    ).await.unwrap().unwrap();
    assert!(["pool-a", "pool-b"].contains(&route.account_id.as_str()));
    assert!(now_ms() >= reset * 1000);
}

#[test]
fn native_quota_snapshot_wait_preserves_api_key_and_official_plan_exceptions() {
    let (_root, host, request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    let collection = host.collection_override.as_ref().unwrap();
    let mut api = host.account_overrides["pool-a"].clone();
    api.auth_mode = CodexAuthMode::Apikey;
    api.openai_api_key = Some("synthetic-unused-key".into());
    assert_eq!(
        native_model_quota_retry_wait(collection, &api, &request.model),
        Some(Duration::ZERO)
    );
    let mut official = host.account_overrides["pool-a"].clone();
    official.api_provider_id = Some("cockpit_chatgpt_plan".into());
    official.official_registration_id = Some("synthetic-unused-registration".into());
    assert!(native_is_chatgpt_plan_account(&official));
    assert_eq!(
        native_model_quota_retry_wait(collection, &official, &request.model),
        Some(Duration::ZERO)
    );
}

#[tokio::test]
async fn rebuildable_quota_owner_yields_to_ready_backup_before_its_reset() {
    let (_root, mut host, mut request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    host.collection_override.as_mut().unwrap().max_retry_interval_ms = 120_000;
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        native_contract::Host::route_for_rebuild(&host, &caller, request.clone()),
    )
    .await
    .expect("a ready backup must be tried before waiting for quota reset")
    .err()
    .unwrap();
    assert_eq!(outcome.code, "account_cooling");
    assert!(outcome.retry_after.is_none());

    request.excluded_account_ids.push(request.preferred_account_id.take().unwrap());
    let replacement = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(replacement.account_id, "pool-b");
}

#[tokio::test]
async fn native_business_accounts_remain_eligible_when_selected_together() {
    let (_root, mut host, mut request) = native_quota_continuation_fixture(None);
    let template = host.account_overrides["pool-a"].clone();
    for (id, hourly, weekly) in [("pool-a", 75, 98), ("pool-b", 25, 24)] {
        let mut account = template.clone();
        account.id = id.into();
        account.account_id = Some(format!("workspace-{id}"));
        account.plan_type = Some("business".into());
        let quota = account.quota.as_mut().unwrap();
        quota.hourly_percentage = hourly;
        quota.weekly_percentage = weekly;
        host.account_overrides.insert(id.into(), account);
    }
    host.collection_override.as_mut().unwrap().restrict_free_accounts = true;
    request.preferred_account_id = None;
    request.model = "gpt-6.1-sol".into();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    for (index, members) in [
        vec!["pool-a"],
        vec!["pool-b"],
        vec!["pool-a", "pool-b"],
        vec!["pool-a", "pool-b"],
    ].into_iter().enumerate() {
        host.collection_override.as_mut().unwrap().account_ids =
            members.iter().copied().map(str::to_owned).collect();
        request.session_id = Some(format!("business-{index}"));
        request.thread_id = request.session_id.clone();
        let route = native_select_route(&host, &caller, request.clone()).await.unwrap();
        assert!(members.contains(&route.account_id.as_str()));
    }
}

#[tokio::test]
async fn free_member_picker_filter_preserves_native_models_and_routing() {
    let (root, mut host, mut request) = native_quota_continuation_fixture(None);
    let free = host.account_overrides.get_mut("pool-a").unwrap();
    free.plan_type = Some("free".into());
    free.quota.as_mut().unwrap().hourly_percentage = 80;
    let free = free.clone();

    let collection = host.collection_override.as_mut().unwrap();
    collection.account_ids = vec![free.id.clone()];
    collection.restrict_free_accounts = true;
    request.preferred_account_id = None;
    request.model = "gpt-6.1-sol".into();

    let profile_models = local_access_profile_model_definitions_with_accounts(
        root.path(),
        collection,
        &collection.api_key,
        true,
        &[free.clone()],
    )
    .unwrap();
    assert!(profile_models.iter().any(|model| model.model_id == request.model));

    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let catalog = native_models(&host, &caller).await.unwrap();
    assert!(catalog["data"].as_array().unwrap().iter().any(|model| model["id"] == request.model));
    let route = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(route.account_id, free.id);
}

#[tokio::test]
async fn native_business_accounts_share_http_load_and_fail_over_after_quota_rejection() {
    use axum::{extract::Path as AxumPath, http::StatusCode, response::IntoResponse, routing::post, Json, Router};

    let received = Arc::new(Mutex::new(Vec::new()));
    let rate_limit_a = Arc::new(AtomicBool::new(false));
    let app = Router::new().route("/stream/{account}/responses", post({
        let received = received.clone();
        let rate_limit_a = rate_limit_a.clone();
        move |AxumPath(account): AxumPath<String>, headers: axum::http::HeaderMap, Json(body): Json<Value>| {
            let received = received.clone();
            let rate_limit_a = rate_limit_a.clone();
            async move {
                assert_eq!(headers["chatgpt-account-id"], format!("workspace-{account}"));
                assert_eq!(body["model"], "gpt-6.1-sol");
                received.lock().unwrap().push(account.clone());
                if account == "pool-a" && rate_limit_a.swap(false, Ordering::SeqCst) {
                    return (StatusCode::TOO_MANY_REQUESTS, Json(json!({
                        "error":{"code":"usage_limit_reached","message":"Fixture quota exhausted","resets_in_seconds":60}
                    }))).into_response();
                }
                Json(json!({"id":format!("resp-{account}"),"status":"completed","output":[]})).into_response()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/stream", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (root, mut bridge, _) = native_quota_continuation_fixture(None);
    let template = bridge.account_overrides["pool-a"].clone();
    for id in ["pool-a", "pool-b"] {
        let mut account = template.clone();
        account.id = id.into();
        account.account_id = Some(format!("workspace-{id}"));
        account.plan_type = Some("business".into());
        account.quota.as_mut().unwrap().hourly_percentage = 75;
        bridge.account_overrides.insert(id.into(), account);
    }
    let collection = bridge.collection_override.as_mut().unwrap();
    collection.session_affinity = false;
    collection.restrict_free_accounts = true;
    collection.max_retry_credentials = 2;
    let mut service = native_api::start(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(LoopbackUpstream { bridge, origin }),
        root.path().join("identity.sqlite"),
        native_api::Options::default(),
    ).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap();
    let send = |index: u8| {
        client
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("fixture-primary-key")
            .header("session-id", format!("business-{index}"))
            .json(&json!({"model":"gpt-6.1-sol","input":"Ping","stream":false}))
            .send()
    };
    for index in 0..2 {
        let response = send(index).await.unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let mut balanced = received.lock().unwrap().clone();
    balanced.sort();
    assert_eq!(balanced, ["pool-a", "pool-b"]);
    rate_limit_a.store(true, Ordering::SeqCst);
    for index in 2..4 {
        let response = send(index).await.unwrap();
        assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    }
    let calls = received.lock().unwrap().clone();
    assert_eq!(&calls[2..], ["pool-a", "pool-b", "pool-b"]);
    service.shutdown(Duration::from_secs(3)).await.unwrap();
    upstream.abort();
}
