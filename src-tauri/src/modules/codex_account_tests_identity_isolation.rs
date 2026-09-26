// 同邮箱跨平台身份隔离：使用临时账号库验证真实导入与持久化路径。
fn seed_isolation_provider(mixed: bool) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        super::build_grok_provider_account_id("isolation-grok"),
        "shared@example.com".into(),
        String::new(),
        CodexApiProviderMode::Custom,
        Some(super::GROK_CLI_CHAT_PROXY_BASE_URL.into()),
        Some("grok".into()),
        Some("Grok".into()),
        vec!["grok-4.6".into()],
    );
    account.upstream_grok_account_id = Some("isolation-grok".into());
    if mixed {
        account.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
        account.tokens = make_codex_tokens(
            "shared@example.com",
            "acc-shared",
            "org-shared",
            "old",
            "rt-old",
        );
        account.account_id = Some("acc-shared".into());
        account.organization_id = Some("org-shared".into());
    }
    save_account(&account).unwrap();
    save_account_index(&build_test_account_index(&account)).unwrap();
    account
}

#[test]
fn identity_isolation_oauth_and_access_import_leave_grok_unchanged() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    for mixed in [false, true] {
        let _env = TestEnvGuard::new("identity-isolation-import");
        let provider = seed_isolation_provider(mixed);
        let before = serde_json::to_value(load_account(&provider.id).unwrap()).unwrap();
        let tokens = make_codex_tokens(
            "shared@example.com",
            "acc-shared",
            "org-shared",
            "new",
            "rt-new",
        );
        // Access-token-only 导入先执行，防止 OAuth 已创建正确账号掩盖另一入口的问题。
        let imported = upsert_account_from_access_token_with_hints(
            tokens.access_token.clone(),
            CodexAccessTokenImportHints {
                email: Some("shared@example.com".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let authorized = upsert_account(tokens).unwrap();
        assert_eq!(imported.id, authorized.id);
        assert_eq!(
            authorized.id,
            build_account_storage_id("shared@example.com", Some("acc-shared"), Some("org-shared"),)
        );
        assert_ne!(authorized.id, provider.id);
        assert!(authorized.upstream_grok_account_id.is_none());
        assert_eq!(
            serde_json::to_value(load_account(&provider.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(load_account_index().accounts.len(), 2);
    }
}

#[test]
fn identity_isolation_reauth_of_mixed_record_preserves_original() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("identity-isolation-reauth");
    let provider = seed_isolation_provider(true);
    let before = serde_json::to_value(load_account(&provider.id).unwrap()).unwrap();
    let account = upsert_account_for_reauth(
        make_codex_tokens(
            "shared@example.com",
            "acc-shared",
            "org-shared",
            "reauth",
            "rt-new",
        ),
        &provider.id,
    )
    .unwrap();
    assert_ne!(account.id, provider.id);
    assert!(account.upstream_grok_account_id.is_none());
    assert_eq!(
        serde_json::to_value(load_account(&provider.id).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        load_account_index().current_account_id.as_deref(),
        Some(provider.id.as_str())
    );
    assert_eq!(load_account_index().accounts.len(), 2);
}

#[test]
fn identity_isolation_all_email_fallbacks_exclude_provider_records() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("identity-isolation-fallbacks");
    let mut provider = seed_isolation_provider(false);
    // Grok、普通 API Key、历史污染的 Grok ID、非保留 ID 的残留标记。
    for kind in 0..4 {
        provider.id = if kind == 1 || kind == 3 {
            "other-provider".into()
        } else {
            super::build_grok_provider_account_id("isolation-grok")
        };
        provider.auth_mode = if kind < 2 {
            crate::models::codex::CodexAuthMode::Apikey
        } else {
            crate::models::codex::CodexAuthMode::OAuth
        };
        provider.upstream_grok_account_id = if kind == 1 || kind == 2 {
            None
        } else {
            Some("isolation-grok".into())
        };
        for identity in [false, true] {
            provider.account_id = identity.then(|| "acc-shared".into());
            provider.organization_id = identity.then(|| "org-shared".into());
            save_account(&provider).unwrap();
            let index = build_test_account_index(&provider);
            for (account_id, org) in [
                (None, None),
                (Some("acc-shared"), None),
                (Some("acc-shared"), Some("org-shared")),
            ] {
                assert_eq!(
                    super::find_existing_account_id(&index, "SHARED@example.com", account_id, org,),
                    None,
                    "kind={kind}, identity={identity}"
                );
            }
        }
    }
}

#[test]
fn identity_isolation_grok_does_not_make_legacy_oauth_ambiguous() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("identity-isolation-legacy");
    let provider = seed_isolation_provider(false);
    let legacy = CodexAccount::new(
        "legacy-oauth".into(),
        "shared@example.com".into(),
        make_codex_tokens("shared@example.com", "", "", "legacy", "rt-old"),
    );
    save_account(&legacy).unwrap();
    let mut index = build_test_account_index(&provider);
    index
        .accounts
        .extend(build_test_account_index(&legacy).accounts);
    save_account_index(&index).unwrap();
    let account = upsert_account(make_codex_tokens(
        "shared@example.com",
        "acc-shared",
        "org-shared",
        "new",
        "rt-new",
    ))
    .unwrap();
    assert_eq!(account.id, legacy.id);
    assert_eq!(load_account_index().accounts.len(), 2);
}

#[test]
fn identity_isolation_readding_grok_preserves_oauth_in_both_orders() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    for grok_first in [false, true] {
        let _env = TestEnvGuard::new("identity-isolation-both-orders");
        let payload = crate::models::grok::GrokOAuthCompletePayload {
            email: "shared@example.com".into(),
            access_token: "grok-access".into(),
            token_endpoint: "https://example.test/token".into(),
            auth_raw: serde_json::json!({}),
            first_name: None,
            last_name: None,
            user_id: None,
            principal_id: None,
            principal_type: None,
            team_id: None,
            profile_image_asset_id: None,
            coding_data_retention_opt_out: None,
            refresh_token: None,
            id_token: None,
            token_type: None,
            expires_at: None,
        };
        let grok = crate::modules::grok_account::upsert_oauth(payload).unwrap();
        if grok_first {
            super::upsert_grok_provider_account(&grok.id, &grok.email, None, None).unwrap();
        }
        let oauth = upsert_account(make_codex_tokens(
            "shared@example.com",
            "acc-shared",
            "org-shared",
            "new",
            "rt-new",
        ))
        .unwrap();
        let before = serde_json::to_value(load_account(&oauth.id).unwrap()).unwrap();
        let provider =
            super::upsert_grok_provider_account(&grok.id, &grok.email, None, None).unwrap();
        let repeated =
            super::upsert_grok_provider_account(&grok.id, &grok.email, None, None).unwrap();
        assert_ne!(provider.id, oauth.id);
        assert_eq!(provider.id, repeated.id);
        assert_eq!(
            serde_json::to_value(load_account(&oauth.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(load_account_index().accounts.len(), 2);
        assert!(load_account(&oauth.id)
            .unwrap()
            .upstream_grok_account_id
            .is_none());
    }
}
