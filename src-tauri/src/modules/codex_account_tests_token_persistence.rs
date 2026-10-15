use super::*;

struct StorageGuard {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl StorageGuard {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("codex-refresh-cas-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &path);
        assert!(get_accounts_dir().starts_with(&path));
        Self { path, previous }
    }
}

impl Drop for StorageGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
            None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn tokens(label: &str) -> CodexTokens {
    CodexTokens {
        id_token: format!("id-{label}"),
        access_token: format!("access-{label}"),
        refresh_token: Some(format!("refresh-{label}")),
    }
}

fn seed() -> CodexAccount {
    let account = CodexAccount::new(
        "refresh-cas".into(),
        "test@example.com".into(),
        tokens("old"),
    );
    save_account(&account).unwrap();
    account
}

#[test]
fn refresh_response_preserves_concurrent_metadata_and_quota_write() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = StorageGuard::new();
    let original = seed();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let response_barrier = barrier.clone();
    let response_account = original.clone();
    let response = std::thread::spawn(move || {
        // The refresh request already captured its credentials; release the
        // simulated response only after a separate writer updates storage.
        response_barrier.wait();
        persist_refreshed_account(&response_account, tokens("rotated"))
    });
    let mut latest = original.clone();
    latest.account_note = Some("edited during refresh".into());
    latest.tags = Some(vec!["keep".into()]);
    latest.last_client_auth_observed_at = Some(123);
    latest.quota = Some(
        serde_json::from_value(serde_json::json!({
            "hourly_percentage": 70,
            "hourly_reset_time": 12345,
            "weekly_percentage": 80,
            "weekly_reset_time": 56789
        }))
        .unwrap(),
    );
    save_account(&latest).unwrap();
    barrier.wait();
    let (returned, saved) = response.join().unwrap().unwrap();
    assert!(saved);
    let persisted = load_account(&original.id).unwrap();
    for account in [&returned, &persisted] {
        assert_eq!(account.account_note, latest.account_note);
        assert_eq!(account.tags, latest.tags);
        assert_eq!(account.last_client_auth_observed_at, Some(123));
        assert_eq!(account.quota.as_ref().unwrap().hourly_percentage, 70);
        assert_eq!(account.quota.as_ref().unwrap().weekly_percentage, 80);
        assert_eq!(
            account.tokens.refresh_token,
            tokens("rotated").refresh_token
        );
        assert_eq!(account.token_generation, original.token_generation + 1);
    }
}

#[test]
fn refresh_response_cannot_overwrite_reauthorized_credentials_at_same_generation() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = StorageGuard::new();
    let original = seed();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let response_barrier = barrier.clone();
    let response_account = original.clone();
    let response = std::thread::spawn(move || {
        response_barrier.wait();
        persist_refreshed_account(&response_account, tokens("stale-response"))
    });
    let mut reauthorized = original.clone();
    reauthorized.tokens = tokens("reauthorized");
    save_account(&reauthorized).unwrap();
    barrier.wait();
    let (returned, saved) = response.join().unwrap().unwrap();
    assert!(!saved);
    assert_eq!(
        returned.tokens.access_token,
        reauthorized.tokens.access_token
    );
    let persisted = load_account(&original.id).unwrap();
    assert_eq!(
        persisted.tokens.refresh_token,
        reauthorized.tokens.refresh_token
    );
    assert_eq!(persisted.token_generation, original.token_generation);
}

#[test]
fn failed_old_refresh_cannot_mark_new_credentials_as_reauth_required() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = StorageGuard::new();
    let original = seed();
    let mut reauthorized = original.clone();
    reauthorized.tokens = tokens("reauthorized");
    reauthorized.token_generation += 1;
    save_account(&reauthorized).unwrap();
    let (returned, marked) = update_account_after_refresh_if_current(&original, |current| {
        current.requires_reauth = true;
        current.reauth_reason = Some("invalid_grant".into());
    })
    .unwrap();
    assert!(!marked);
    assert!(!returned.requires_reauth);
    assert!(!load_account(&original.id).unwrap().requires_reauth);
}

#[test]
fn failed_current_refresh_preserves_concurrent_user_edits() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = StorageGuard::new();
    let original = seed();
    let mut edited = original.clone();
    edited.account_note = Some("keep after failure".into());
    save_account(&edited).unwrap();
    let (returned, marked) = update_account_after_refresh_if_current(&original, |current| {
        current.requires_reauth = true;
        current.reauth_reason = Some("invalid_grant".into());
    })
    .unwrap();
    assert!(marked);
    assert!(returned.requires_reauth);
    let persisted = load_account(&original.id).unwrap();
    assert!(persisted.requires_reauth);
    assert_eq!(persisted.account_note, edited.account_note);
    assert_eq!(persisted.tokens.access_token, original.tokens.access_token);
}

#[test]
fn refresh_response_cannot_recreate_deleted_account() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _storage = StorageGuard::new();
    let original = seed();
    delete_account_file(&original.id).unwrap();
    assert!(persist_refreshed_account(&original, tokens("rotated")).is_err());
    assert!(load_account(&original.id).is_none());
    assert!(!get_accounts_dir()
        .join(format!("{}.json", original.id))
        .exists());
}

#[test]
fn force_refresh_transport_failures_do_not_claim_expiration_or_require_reauth() {
    for raw in [
        "Token 刷新请求失败: error sending request for url (https://auth.openai.com/oauth/token)",
        "Token 刷新请求失败: operation timed out",
        "Token 刷新失败: status=503 Service Unavailable, body_len=42",
        "解析 Token 响应失败: invalid JSON",
    ] {
        assert!(matches!(
            super::classify_refresh_error(raw),
            super::CodexRefreshErrorKind::Other
        ));
        assert!(!super::is_reauth_required_refresh_error(raw));
        let formatted = format_refresh_error_for_user(raw);
        assert!(formatted.contains(raw));
        for misleading in ["已过期", "expired", "请重新登录", "sign in again"] {
            assert!(!formatted.contains(misleading), "{formatted}");
        }
    }

    for code in [
        "PROXY_RUNTIME_BUSY",
        "PROXY_ENGINE_MISSING",
        "PROXY_RUNTIME_READ_TIMEOUT",
    ] {
        assert_eq!(format_refresh_error_for_user(code), code);
        assert!(!super::is_reauth_required_refresh_error(code));
    }
}
