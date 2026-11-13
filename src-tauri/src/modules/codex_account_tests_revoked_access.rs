#[test]
fn known_access_revocation_requires_reauth_and_new_chain_recovers() {
    let mut account = CodexAccount::new(
        "revoked".into(),
        "test@example.com".into(),
        make_codex_tokens("test@example.com", "account", "org", "access", "refresh"),
    );
    account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("token_revoked".into()),
        message: "Access token revoked".into(),
        timestamp: 1,
    });
    super::observe_known_access_token_revocation(&mut account);
    assert!(account.requires_reauth);
    assert!(super::reject_known_access_token_revocation(&account).is_err());
    assert!(super::account_has_remote_api_auth_rejection(&account));
    let generation = account.token_generation;
    account.tokens = make_codex_tokens(
        "test@example.com",
        "account",
        "org",
        "new-access",
        "new-refresh",
    );
    super::mark_token_chain_updated(&mut account);
    assert_eq!(account.token_generation, generation + 1);
    assert!(!account.requires_reauth);
    assert!(account.quota_error.is_none());
    assert!(super::reject_known_access_token_revocation(&account).is_ok());
}

#[test]
fn generic_api_errors_and_refresh_conflicts_do_not_mark_access_revoked() {
    let mut account = CodexAccount::new(
        "valid".into(),
        "test@example.com".into(),
        make_codex_tokens("test@example.com", "account", "org", "access", "refresh"),
    );
    for (code, message) in [
        ("401", "Proxy returned HTTP 401"),
        ("403", "Forbidden"),
        ("refresh_token_reused", "refresh token reused"),
        ("token_invalidated", "refresh_token invalidated"),
        ("token_invalidated", "刷新 Token 失败: token_invalidated"),
    ] {
        account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
            code: Some(code.into()),
            message: message.into(),
            timestamp: 1,
        });
        super::observe_known_access_token_revocation(&mut account);
        assert!(!account.requires_reauth, "{code}: {message}");
        assert!(super::reject_known_access_token_revocation(&account).is_ok());
    }
}

#[test]
fn switch_accepts_live_access_token_after_refresh_grant_is_invalid() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-invalid-refresh-grant-switch");
    let mut account = seed_oauth_account(make_codex_tokens(
        "invalid-grant@example.com",
        "acc-invalid-grant",
        "org-invalid-grant",
        "live-access",
        "invalid-refresh",
    ));
    account.requires_reauth = true;
    account.reauth_reason = Some(format_refresh_error_for_user(
        "Token 刷新失败: status=400 Bad Request, error_code=invalid_grant",
    ));
    save_account(&account).expect("save refresh-grant failure");
    let runtime = tokio::runtime::Runtime::new().expect("create runtime");

    let prepared = runtime
        .block_on(super::prepare_account_switch_locked(&account.id, false))
        .expect("valid access still usable");
    let super::PreparedCodexAccountSwitch::Account(prepared) = prepared else {
        panic!("expected direct OAuth account");
    };
    assert_eq!(prepared.tokens.access_token, account.tokens.access_token);
    assert_eq!(prepared.token_generation, account.token_generation);
    assert!(prepared.requires_reauth);
    assert_eq!(prepared.reauth_reason, account.reauth_reason);

    let target = env.home_dir.join("launch-profile");
    std::fs::create_dir_all(&target).unwrap();
    runtime
        .block_on(super::prepare_account_for_instance_launch_preflight(&account.id))
        .expect("valid access passes the instance preflight");
    runtime
        .block_on(super::project_preflighted_account_for_instance_launch(&account.id, &target))
        .expect("write the current access token to the selected profile");
    let auth: serde_json::Value =
        serde_json::from_slice(&std::fs::read(target.join("auth.json")).unwrap()).unwrap();
    assert_eq!(auth["tokens"]["access_token"], account.tokens.access_token);

    account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("token_invalidated".into()),
        message: "API 返回错误 401 Unauthorized [error_code:token_invalidated]".into(),
        timestamp: 1,
    });
    save_account(&account).expect("record server access-token revocation");
    assert!(runtime
        .block_on(super::prepare_account_switch_locked(&account.id, false))
        .is_err());
    assert!(runtime
        .block_on(super::prepare_account_for_instance_launch_preflight(&account.id))
        .is_err());
}
