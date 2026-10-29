// Synthetic OAuth metadata only. No account files or upstream requests are used.

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
        let error = native_select_route(&host, &caller, request)
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
async fn native_quota_snapshot_does_not_bypass_bound_oauth_reservation() {
    let (_root, mut host, request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    let collection = host.collection_override.as_mut().unwrap();
    collection.bound_oauth_account_id = Some("pool-a".into());
    collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 10,
        strict: false,
        safety_margin_percent: 3,
    });
    let error = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await
        .err()
        .unwrap();
    assert_eq!(
        (error.status, error.code),
        (409, "account_binding_unavailable")
    );
    assert!(error.retry_after.is_none());
    assert!(host.gates.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_quota_snapshot_disable_cooling_does_not_disable_real_quota() {
    let (_root, mut host, request) =
        native_quota_continuation_fixture(Some(chrono::Utc::now().timestamp() + 60));
    host.collection_override.as_mut().unwrap().disable_cooling = true;
    host.cooldowns.lock().unwrap().insert(
        native_cooldown_key("pool-a", &request.model, false),
        NativeCooldown {
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
    assert_eq!((error.status, error.code), (503, "no_available_account"));
    let wait = error.retry_after.unwrap();
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    let wait = native_retry_cooldown_wait(&host, &caller, request, Duration::from_secs(61))
        .await
        .unwrap()
        .unwrap();
    assert!(wait > Duration::from_secs(58) && wait <= Duration::from_secs(60));
    assert!(host.gates.lock().unwrap().is_empty());
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
