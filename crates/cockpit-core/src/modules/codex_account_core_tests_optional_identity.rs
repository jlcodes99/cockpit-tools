#[test]
fn healthy_access_token_does_not_refresh_for_expired_missing_or_due_identity() {
    let now = chrono::Utc::now().timestamp();
    for id_token in [
        make_jwt(serde_json::json!({"exp": now - 3600})),
        make_jwt(serde_json::json!({"exp": now + 30})),
        String::new(),
        "malformed-identity".to_string(),
    ] {
        let account = CodexAccount::new(
            "optional-id".to_string(),
            "test@example.com".to_string(),
            CodexTokens {
                access_token: make_jwt(serde_json::json!({"exp": now + 7200})),
                id_token,
                refresh_token: Some("synthetic-refresh".to_string()),
            },
        );
        assert!(!super::managed_account_tokens_need_refresh(&account));
    }
}

#[test]
fn expired_access_token_still_requires_refresh_with_fresh_identity() {
    let now = chrono::Utc::now().timestamp();
    let account = CodexAccount::new(
        "expired-access".to_string(),
        "test@example.com".to_string(),
        CodexTokens {
            access_token: make_jwt(serde_json::json!({"exp": now - 3600})),
            id_token: make_jwt(serde_json::json!({"exp": now + 7200})),
            refresh_token: Some("synthetic-refresh".to_string()),
        },
    );
    assert!(super::managed_account_tokens_need_refresh(&account));
}
