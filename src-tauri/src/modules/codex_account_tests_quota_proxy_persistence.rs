use super::*;

struct QuotaProxyStorage {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl QuotaProxyStorage {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("codex-quota-proxy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &path);
        assert!(get_accounts_dir().starts_with(&path));
        Self { path, previous }
    }
}

impl Drop for QuotaProxyStorage {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn seed_quota_proxy_account() -> CodexAccount {
    let account = CodexAccount::new(
        "quota-proxy-fixture".into(),
        "quota-proxy@example.invalid".into(),
        CodexTokens {
            id_token: "fixture-id".into(),
            access_token: "fixture-access".into(),
            refresh_token: Some("fixture-refresh".into()),
        },
    );
    save_account(&account).unwrap();
    account
}

#[test]
fn delayed_quota_response_preserves_independent_proxy_saved_after_disabled_snapshot() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let account = seed_quota_proxy_account();
    let mut snapshot = update_account_egress_proxy(&account.id, None, true).unwrap();
    snapshot.quota = Some(
        serde_json::from_value(serde_json::json!({
            "hourly_percentage": 41,
            "weekly_percentage": 63
        }))
        .unwrap(),
    );
    snapshot.usage_updated_at = Some(123);
    let response_ready = Arc::new(std::sync::Barrier::new(2));
    let delayed_response = response_ready.clone();
    let response = std::thread::spawn(move || {
        delayed_response.wait();
        save_account_preserving_egress_proxy(&snapshot)
    });
    let edited =
        update_account_egress_proxy(&account.id, Some("http://127.0.0.1:8081".into()), false)
            .unwrap();
    response_ready.wait();
    let returned = response.join().unwrap().unwrap();
    let stored = load_account(&account.id).unwrap();
    for current in [&returned, &stored] {
        assert_eq!(current.egress_proxy_url, edited.egress_proxy_url);
        assert!(!current.egress_proxy_disabled);
        assert_eq!(current.quota.as_ref().unwrap().hourly_percentage, 41);
        assert_eq!(current.usage_updated_at, Some(123));
    }
}

#[test]
fn stale_quota_snapshot_cannot_restore_replaced_cleared_or_disabled_proxy_modes() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let account = seed_quota_proxy_account();
    for (before, before_disabled, after, after_disabled) in [
        (
            Some("http://127.0.0.1:8081"),
            false,
            Some("http://127.0.0.1:8082"),
            false,
        ),
        (Some("http://127.0.0.1:8081"), false, None, false),
        (Some("http://127.0.0.1:8081"), false, None, true),
        (None, false, None, true),
        (None, true, None, false),
    ] {
        let mut snapshot =
            update_account_egress_proxy(&account.id, before.map(str::to_owned), before_disabled)
                .unwrap();
        snapshot.subscription_query_last_success_at = Some(456);
        let edited =
            update_account_egress_proxy(&account.id, after.map(str::to_owned), after_disabled)
                .unwrap();
        let returned = save_account_preserving_egress_proxy(&snapshot).unwrap();
        let stored = load_account(&account.id).unwrap();
        for current in [&returned, &stored] {
            assert_eq!(current.egress_proxy_url, edited.egress_proxy_url);
            assert_eq!(current.egress_proxy_disabled, edited.egress_proxy_disabled);
            assert_eq!(current.subscription_query_last_success_at, Some(456));
        }
    }
}

#[test]
fn failed_quota_response_updates_error_without_overwriting_latest_proxy() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let mut snapshot = seed_quota_proxy_account();
    snapshot.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("fixture_upstream_failure".into()),
        message: "fixture upstream failure".into(),
        timestamp: 789,
    });
    let edited =
        update_account_egress_proxy(&snapshot.id, Some("http://127.0.0.1:8082".into()), false)
            .unwrap();
    let returned = save_account_preserving_egress_proxy(&snapshot).unwrap();
    let stored = load_account(&snapshot.id).unwrap();
    for current in [&returned, &stored] {
        assert_eq!(current.egress_proxy_url, edited.egress_proxy_url);
        assert_eq!(current.quota_error.as_ref().unwrap().timestamp, 789);
        assert_eq!(
            current.quota_error.as_ref().unwrap().code.as_deref(),
            Some("fixture_upstream_failure")
        );
    }
}

#[test]
fn quota_proxy_merge_fails_closed_when_latest_account_is_unreadable_or_missing() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let snapshot = seed_quota_proxy_account();
    let path = get_accounts_dir().join(format!("{}.json", snapshot.id));
    fs::write(&path, "invalid fixture JSON").unwrap();
    assert!(save_account_preserving_egress_proxy(&snapshot).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "invalid fixture JSON");
    fs::remove_file(&path).unwrap();
    assert!(save_account_preserving_egress_proxy(&snapshot).is_err());
    assert!(!path.exists());
}

#[test]
fn quota_proxy_merge_keeps_deleted_and_newer_credential_tombstone_guards() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let snapshot = seed_quota_proxy_account();
    let mut latest = snapshot.clone();
    latest.token_generation += 1;
    latest.tokens.access_token = "fixture-newer-access".into();
    latest.egress_proxy_url = Some("http://127.0.0.1:8082".into());
    save_account(&latest).unwrap();
    write_account_tombstone(
        &latest.id,
        false,
        latest.token_generation,
        account_credential_hash(&latest),
    )
    .unwrap();
    assert!(save_account_preserving_egress_proxy(&snapshot).is_err());
    let persisted = load_account(&latest.id).unwrap();
    assert_eq!(persisted.token_generation, latest.token_generation);
    assert_eq!(persisted.egress_proxy_url, latest.egress_proxy_url);

    write_account_tombstone(&latest.id, true, latest.token_generation, String::new()).unwrap();
    delete_account_file(&latest.id).unwrap();
    assert!(save_account_preserving_egress_proxy(&snapshot).is_err());
    assert!(load_account(&latest.id).is_none());
    assert!(!get_accounts_dir()
        .join(format!("{}.json", latest.id))
        .exists());
}

#[test]
fn reauth_state_helper_preserves_latest_proxy_in_storage_and_mutable_snapshot() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _storage = QuotaProxyStorage::new();
    let mut snapshot = seed_quota_proxy_account();
    let edited =
        update_account_egress_proxy(&snapshot.id, Some("http://127.0.0.1:8082".into()), false)
            .unwrap();
    mark_account_requires_reauth(&mut snapshot, "fixture expired authorization").unwrap();
    let stored = load_account(&snapshot.id).unwrap();
    for current in [&snapshot, &stored] {
        assert_eq!(current.egress_proxy_url, edited.egress_proxy_url);
        assert!(!current.egress_proxy_disabled);
        assert!(current.requires_reauth);
        assert_eq!(
            current.reauth_reason.as_deref(),
            Some("fixture expired authorization")
        );
    }
    let mut stale_cleanup = snapshot;
    stale_cleanup.reauth_reason = Some("refresh_token_reused".into());
    let disabled = update_account_egress_proxy(&stale_cleanup.id, None, true).unwrap();
    clear_refresh_token_reused_state(&mut stale_cleanup).unwrap();
    let stored = load_account(&stale_cleanup.id).unwrap();
    for current in [&stale_cleanup, &stored] {
        assert_eq!(current.egress_proxy_url, disabled.egress_proxy_url);
        assert!(current.egress_proxy_disabled);
        assert!(!current.requires_reauth);
    }
}

#[test]
fn explicit_credential_sync_preserves_latest_proxy_in_storage_and_returned_snapshot() {
    use base64::Engine as _;
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let storage = QuotaProxyStorage::new();
    let mut snapshot = seed_quota_proxy_account();
    let source = storage.path.join("official-fixture");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("config.toml"),
        "cli_auth_credentials_store = \"file\"\n",
    )
    .unwrap();
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::json!({"email":snapshot.email}).to_string());
    fs::write(
        source.join("auth.json"),
        serde_json::json!({
            "tokens": {
                "id_token": format!("e30.{payload}.fixture"),
                "access_token": "fixture-official-access",
                "refresh_token": "fixture-official-refresh"
            }
        })
        .to_string(),
    )
    .unwrap();
    let edited =
        update_account_egress_proxy(&snapshot.id, Some("http://127.0.0.1:8082".into()), false)
            .unwrap();
    assert!(sync_account_from_auth_dir_if_current(&mut snapshot, &source).unwrap());
    let stored = load_account(&snapshot.id).unwrap();
    for current in [&snapshot, &stored] {
        assert_eq!(current.egress_proxy_url, edited.egress_proxy_url);
        assert!(!current.egress_proxy_disabled);
        assert_eq!(current.tokens.access_token, "fixture-official-access");
    }
}

#[test]
fn optional_api_local_state_failure_keeps_original_current_account_and_provider() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let storage = QuotaProxyStorage::new();
    let mut account = CodexAccount::new_api_key(
        "quota-api-fixture".into(),
        "quota-api@example.invalid".into(),
        "fixture-api-key".into(),
        CodexApiProviderMode::Custom,
        Some("https://original.example.invalid/v1".into()),
        Some("original-provider".into()),
        Some("Original Provider".into()),
        Vec::new(),
    );
    account.egress_proxy_url = Some("http://127.0.0.1:8082".into());
    save_account(&account).unwrap();
    let detail = get_accounts_dir().join(format!("{}.json", account.id));
    let persisted_before = fs::read(&detail).unwrap();
    let source = storage.path.join("optional-local-state");
    fs::create_dir_all(&source).unwrap();
    let auth = source.join("auth.json");
    for unreadable in [false, true] {
        if unreadable {
            fs::remove_file(&auth).unwrap();
            // A directory exists at the expected file path but cannot be read as auth.json.
            fs::create_dir(&auth).unwrap();
        } else {
            fs::write(&auth, "invalid fixture auth JSON").unwrap();
        }
        let mut index = CodexAccountIndex::new();
        index.current_account_id = Some(account.id.clone());
        let current = get_current_account_from_loaded(index, |_| Some(account.clone()), &source)
            .expect("optional synchronization failure must not hide the current account");
        assert_eq!(current.egress_proxy_url, account.egress_proxy_url);
        assert_eq!(current.api_base_url, account.api_base_url);
        assert_eq!(current.api_provider_id, account.api_provider_id);
        assert_eq!(fs::read(&detail).unwrap(), persisted_before);
    }

    fs::remove_dir(&auth).unwrap();
    fs::write(
        &auth,
        serde_json::json!({
            "auth_mode": "apikey",
            "OPENAI_API_KEY": "fixture-api-key",
            "base_url": "https://updated.example.invalid/v1"
        })
        .to_string(),
    )
    .unwrap();
    fs::write(&detail, "unreadable current fixture").unwrap();
    let mut unsaved = account.clone();
    assert!(sync_api_key_account_from_local_state(&mut unsaved, &source).is_err());
    assert_eq!(
        unsaved.api_base_url.as_deref(),
        Some("https://updated.example.invalid/v1")
    );
    let mut index = CodexAccountIndex::new();
    index.current_account_id = Some(account.id.clone());
    let current =
        get_current_account_from_loaded(index, |_| Some(account.clone()), &source).unwrap();
    assert_eq!(current.api_base_url, account.api_base_url);
    assert_eq!(current.egress_proxy_url, account.egress_proxy_url);
    assert_eq!(
        fs::read_to_string(&detail).unwrap(),
        "unreadable current fixture"
    );

    // No disk read occurs for an ordinary OAuth account even with an unreadable local file.
    fs::remove_file(&auth).unwrap();
    fs::create_dir(&auth).unwrap();
    let oauth = seed_quota_proxy_account();
    let mut index = CodexAccountIndex::new();
    index.current_account_id = Some(oauth.id.clone());
    let current = get_current_account_from_loaded(index, |_| Some(oauth.clone()), &source).unwrap();
    assert_eq!(current.id, oauth.id);
}
