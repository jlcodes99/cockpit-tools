use super::*;

struct Storage {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}
impl Storage {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("quota-sync-storage-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &path);
        Self { path, previous }
    }
}
impl Drop for Storage {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}
fn seed() -> CodexAccount {
    let mut account = CodexAccount::new(
        format!("sync-fixture-{}", uuid::Uuid::new_v4()),
        "quota@example.invalid".into(),
        CodexTokens {
            access_token: "local-fixture-access".into(),
            id_token: "local-fixture-id".into(),
            refresh_token: None,
        },
    );
    account.account_id = Some("workspace-a".into());
    save_account(&account).unwrap();
    account
}
fn quota(value: i32) -> crate::models::codex::CodexQuota {
    serde_json::from_value(serde_json::json!({"hourly_percentage":value,"weekly_percentage":70}))
        .unwrap()
}

#[test]
fn provider_quota_snapshot_rejects_old_credentials_and_late_failures() {
    let _lock = crate::modules::test_support::env_lock().lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = Storage::new();
    let mut account = seed();
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    account.openai_api_key = Some("fixture-key".into());
    account.api_base_url = Some("https://relay.example.invalid/v1".into());
    save_account(&account).unwrap();
    let older = Instant::now() - Duration::from_secs(1);
    let summary = serde_json::json!({ "mode": "new_api", "quotaRemaining": 94, "unit": "%" });
    save_provider_usage_snapshot(&account, Some(summary.clone()), Instant::now()).unwrap();
    save_provider_usage_snapshot(&account, None, older).unwrap();
    let read = || load_account(&account.id).unwrap();
    let latest = read();
    let raw = latest.quota.unwrap().raw_data.unwrap();
    assert_eq!(raw["provider_usage"], summary);
    assert!(raw.get("provider_usage_query_failed").is_none());
    let last_success = latest.usage_updated_at;
    assert!(last_success.is_some());

    save_provider_usage_snapshot(&account, None, Instant::now()).unwrap();
    let mut latest = read();
    assert_eq!(latest.usage_updated_at, last_success);
    assert_eq!(latest.quota.as_ref().unwrap().raw_data.as_ref().unwrap()["provider_usage_query_failed"], true);
    latest.api_base_url = Some("https://new-relay.example.invalid/v1".into());
    latest.quota = None;
    save_account(&latest).unwrap();
    save_provider_usage_snapshot(&account, Some(summary), Instant::now()).unwrap();
    assert!(read().quota.is_none());
}

#[test]
fn quota_and_subscription_writes_preserve_each_others_results() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    let mut edited = original.clone();
    edited.account_note = Some("edited while request running".into());
    edited.egress_proxy_disabled = true;
    save_account(&edited).unwrap();
    let committed =
        save_refreshed_quota(&original, quota(40), Some("plus".into()), Instant::now()).unwrap();
    assert_eq!(committed.account_note, edited.account_note);
    assert!(committed.egress_proxy_disabled);
    let mut maintenance = committed.clone();
    maintenance.subscription_active_until = Some("2030-01-01T00:00:00Z".into());
    maintenance.subscription_query_last_attempt_at = Some(100);
    maintenance.subscription_query_last_success_at = Some(101);
    let next = save_refreshed_quota(&committed, quota(25), None, Instant::now()).unwrap();
    save_subscription_refresh(&committed, &maintenance).unwrap();
    let stored = load_account(&original.id).unwrap();
    assert_eq!(stored.quota.unwrap().hourly_percentage, 25);
    assert_eq!(stored.usage_updated_at, next.usage_updated_at);
    assert_eq!(
        stored.subscription_active_until,
        maintenance.subscription_active_until
    );
    assert_eq!(stored.tokens.access_token, original.tokens.access_token);

    // A credential or quota path writing a snapshot loaded before the lookup
    // must keep the newer subscription date and its source metadata.
    let saved = save_account_preserving_egress_proxy(&next).unwrap();
    assert_eq!(saved.subscription_active_until, maintenance.subscription_active_until);
    assert_eq!(saved.subscription_query_last_success_at, Some(101));
}

#[test]
fn confirmed_free_subscription_survives_a_stale_maintenance_write() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = Storage::new();
    let mut paid = seed();
    paid.plan_type = Some("plus".into());
    paid.subscription_active_until = Some("2000-01-01T00:00:00Z".into());
    save_account(&paid).unwrap();
    let mut free = paid.clone();
    free.plan_type = Some("free".into());
    free.subscription_active_until = None;
    free.subscription_query_last_attempt_at = Some(100);
    free.subscription_query_last_success_at = Some(101);
    save_subscription_refresh(&paid, &free).unwrap();

    let stored = save_account_preserving_egress_proxy(&paid).unwrap();
    assert_eq!(stored.plan_type.as_deref(), Some("free"));
    assert!(stored.subscription_active_until.is_none());
    assert_eq!(stored.subscription_query_last_success_at, Some(101));
    assert_eq!(stored.tokens.access_token, paid.tokens.access_token);
    assert!(!stored.requires_reauth);
}

#[test]
fn confirmed_free_subscription_does_not_clear_date_after_a_concurrent_plan_change() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = Storage::new();
    let mut paid = seed();
    paid.plan_type = Some("plus".into());
    paid.subscription_active_until = Some("2000-01-01T00:00:00Z".into());
    save_account(&paid).unwrap();
    let mut free = paid.clone();
    free.plan_type = Some("free".into());
    free.subscription_active_until = None;
    let updated =
        save_refreshed_quota(&paid, quota(60), Some("pro".into()), Instant::now()).unwrap();

    assert_eq!(
        save_subscription_refresh(&paid, &free).unwrap_err(),
        "CODEX_STALE_QUOTA_REQUEST"
    );
    let stored = load_account(&paid.id).unwrap();
    assert_eq!(stored.plan_type, updated.plan_type);
    assert_eq!(stored.subscription_active_until, paid.subscription_active_until);
}

#[test]
fn changed_token_or_workspace_invalidates_delayed_quota_response() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    for mutation in 0..5 {
        let mut changed = original.clone();
        match mutation {
            0 => changed.token_generation += 1,
            1 => changed.tokens.access_token = "rotated-fixture".into(),
            2 => changed.account_id = Some("workspace-b".into()),
            3 => changed.organization_id = Some("other-org".into()),
            _ => {
                changed.agent_identity = Some(serde_json::from_value(serde_json::json!({
                    "agent_runtime_id": "fixture-runtime",
                    "agent_private_key": "fixture-key",
                    "account_id": "workspace-a",
                    "chatgpt_user_id": "fixture-user",
                })).unwrap());
            }
        }
        save_account(&changed).unwrap();
        assert!(save_refreshed_quota(&original, quota(1), None, Instant::now()).is_err());
        assert!(load_account(&original.id).unwrap().quota.is_none());
    }
}

#[test]
fn deleted_account_is_not_resurrected_by_quota_response() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    fs::remove_file(get_accounts_dir().join(format!("{}.json", original.id))).unwrap();
    assert!(save_refreshed_quota(&original, quota(1), None, Instant::now()).is_err());
    assert!(load_account(&original.id).is_none());
}

#[test]
fn older_in_flight_request_cannot_overwrite_newer_committed_quota() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    let old_request = Instant::now();
    let new_request = old_request + Duration::from_millis(1);
    save_refreshed_quota(&original, quota(15), None, new_request).unwrap();
    assert!(save_refreshed_quota(&original, quota(30), None, old_request).is_err());
    assert_eq!(
        load_account(&original.id)
            .unwrap()
            .quota
            .unwrap()
            .hourly_percentage,
        15
    );
}

#[test]
fn refresh_failure_preserves_user_edits_and_cannot_replace_newer_success() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    let started = Instant::now();
    let mut edited = original.clone();
    edited.account_note = Some("edited during failed query".into());
    save_account(&edited).unwrap();
    save_quota_sync_failure(&original, "network timeout".into(), started).unwrap();
    let failed = load_account(&original.id).unwrap();
    assert_eq!(failed.account_note, edited.account_note);
    assert!(failed.quota.is_none());
    assert!(failed.quota_error.is_some());
    save_refreshed_quota(
        &original,
        quota(20),
        None,
        started + Duration::from_millis(1),
    )
    .unwrap();
    save_quota_sync_failure(&original, "old query failed".into(), started).unwrap();
    let stored = load_account(&original.id).unwrap();
    assert!(stored.quota_error.is_none());
    assert_eq!(stored.quota.unwrap().hourly_percentage, 20);
}
