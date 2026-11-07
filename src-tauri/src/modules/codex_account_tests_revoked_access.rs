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
