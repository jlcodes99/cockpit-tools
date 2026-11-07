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
fn quota_merge_preserves_concurrent_user_edits_and_subscription_work_cannot_restore_old_quota() {
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
    maintenance.subscription_active_until = Some("2030-01-01".into());
    let next = save_refreshed_quota(&committed, quota(25), None, Instant::now()).unwrap();
    save_quota_subscription_maintenance(&committed, &maintenance).unwrap();
    let stored = load_account(&original.id).unwrap();
    assert_eq!(stored.quota.unwrap().hourly_percentage, 25);
    assert_eq!(stored.usage_updated_at, next.usage_updated_at);
    assert_eq!(
        stored.subscription_active_until,
        maintenance.subscription_active_until
    );
    assert_eq!(stored.tokens.access_token, original.tokens.access_token);
}

#[test]
fn changed_token_or_workspace_invalidates_delayed_quota_response() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = Storage::new();
    let original = seed();
    for mutation in 0..4 {
        let mut changed = original.clone();
        match mutation {
            0 => changed.token_generation += 1,
            1 => changed.tokens.access_token = "rotated-fixture".into(),
            2 => changed.account_id = Some("workspace-b".into()),
            _ => changed.organization_id = Some("other-org".into()),
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
