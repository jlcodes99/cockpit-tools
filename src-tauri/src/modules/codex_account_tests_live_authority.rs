#[test]
fn keepalive_leaves_default_profile_refresh_chain_to_the_official_client() {
    let profile = tempfile::tempdir().unwrap();
    let profile_dir = profile.path();
    let tokens = |refresh_token| {
        make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            refresh_token,
            refresh_token,
        )
    };
    assert!(!super::default_profile_uses_keepalive_token(profile_dir, "rt-current"));

    write_oauth_auth_file(profile_dir, &tokens("rt-current"), "acc-current");
    let path = profile_dir.join("auth.json");
    let original = fs::read(&path).unwrap();
    assert!(super::default_profile_uses_keepalive_token(profile_dir, "rt-current"));
    assert!(!super::default_profile_uses_keepalive_token(profile_dir, "rt-unrelated"));
    assert_eq!(fs::read(&path).unwrap(), original);

    write_oauth_auth_file(profile_dir, &tokens("rt-renewed"), "acc-current");
    assert!(!super::default_profile_uses_keepalive_token(profile_dir, "rt-current"));
    assert!(super::default_profile_uses_keepalive_token(profile_dir, "rt-renewed"));

    fs::write(
        profile_dir.join("config.toml"),
        "cli_auth_credentials_store = 'ephemeral'\n",
    )
    .unwrap();
    assert!(!super::default_profile_uses_keepalive_token(profile_dir, "rt-renewed"));
}

#[test]
fn account_switch_roundtrip_preserves_official_rotation_while_stored_access_is_valid() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let env = TestEnvGuard::new("codex-switch-preserves-official-rotation");
    let outgoing = seed_oauth_account(make_codex_tokens(
        "demo@example.com", "acc-current", "org-current", "stored", "rt-stored",
    ));
    let incoming = upsert_account(make_codex_tokens(
        "second@example.com", "acc-second", "org-second", "second", "rt-second",
    )).unwrap();
    let mut rotated = make_codex_tokens(
        "demo@example.com", "acc-current", "org-current", "official", "rt-official",
    );
    rotated.access_token = make_jwt(serde_json::json!({
        "exp": 4_102_448_400i64,
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acc-current", "organization_id": "org-current"
        }
    }));
    let profile = env.codex_home();
    let mut auth = build_oauth_auth_file(&rotated, "acc-current");
    auth.last_refresh = Some(serde_json::json!(chrono::Utc::now().timestamp()));
    fs::write(profile.join("auth.json"), serde_json::to_vec(&auth).unwrap()).unwrap();
    assert!(!super::managed_account_tokens_need_refresh(&outgoing));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(super::switch_account_managed(&incoming.id)).unwrap();
    let saved = load_account(&outgoing.id).unwrap();
    assert_eq!(saved.tokens.refresh_token, rotated.refresh_token);
    assert_eq!(saved.token_generation, outgoing.token_generation + 1);
    assert_eq!(super::read_official_oauth_tokens(&profile).unwrap().access_token, incoming.tokens.access_token);

    runtime.block_on(super::switch_account_managed(&outgoing.id)).unwrap();
    let restored = super::read_official_oauth_tokens(&profile).unwrap();
    assert_eq!(restored.access_token, rotated.access_token);
    assert_eq!(restored.refresh_token, rotated.refresh_token);
}

#[test]
fn live_authority_sync_finds_default_auth_with_electron_process_entries() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-live-authority-electron-profile");
    let mut account = seed_oauth_account(make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "stored",
        "rt-stored",
    ));
    let generation = account.token_generation;
    let electron_dir = env.home_dir.join("AppData/Roaming/Codex/web/Codex");
    fs::create_dir_all(&electron_dir).unwrap();
    let entries = [(123, Some(electron_dir.to_string_lossy().into_owned()))];

    let official_tokens = |email, expires| {
        let mut tokens = make_codex_tokens(
            email,
            "acc-current",
            "org-current",
            "official",
            "rt-official",
        );
        tokens.access_token = make_jwt(serde_json::json!({
            "exp": expires,
            "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
        }));
        tokens
    };
    for (email, expires) in [
        ("other@example.com", 4_102_448_400i64),
        ("demo@example.com", 4_102_441_200),
    ] {
        write_oauth_auth_file(
            &env.codex_home(),
            &official_tokens(email, expires),
            "acc-current",
        );
        assert!(
            !super::sync_account_from_live_authority_sources_with_entries(&mut account, &entries)
                .unwrap()
        );
        let unchanged = load_account(&account.id).unwrap();
        assert_eq!(unchanged.token_generation, generation);
        assert_eq!(unchanged.tokens.refresh_token, account.tokens.refresh_token);
    }

    let fresh = official_tokens("demo@example.com", 4_102_448_400);
    write_oauth_auth_file(&env.codex_home(), &fresh, "acc-current");
    let auth_path = env.codex_home().join("auth.json");
    let auth_before = fs::read(&auth_path).unwrap();
    assert!(
        !super::sync_account_from_live_authority_sources_with_entries(&mut account, &[]).unwrap()
    );
    assert!(
        super::sync_account_from_live_authority_sources_with_entries(&mut account, &entries)
            .unwrap()
    );

    let persisted = load_account(&account.id).unwrap();
    assert_eq!(account.tokens.access_token, fresh.access_token);
    assert_eq!(persisted.tokens.access_token, fresh.access_token);
    assert_eq!(persisted.tokens.refresh_token, fresh.refresh_token);
    assert_eq!(persisted.token_generation, generation + 1);
    assert_eq!(fs::read(auth_path).unwrap(), auth_before);
}

#[test]
fn authority_snapshot_requires_newer_refresh_marker() {
    let mut account = CodexAccount::new(
        "codex_test".to_string(),
        "demo@example.com".to_string(),
        make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-old",
        ),
    );
    account.account_id = Some("acc-current".to_string());
    account.organization_id = Some("org-current".to_string());
    account.token_updated_at = Some(2000);

    let snapshot = LocalCodexOAuthSnapshot {
        tokens: make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "new",
            "rt-new",
        ),
        email: "demo@example.com".to_string(),
        user_id: Some("user-current".to_string()),
        subscription_active_until: None,
        account_id: Some("acc-current".to_string()),
        organization_id: Some("org-current".to_string()),
        last_refresh_at: Some(1000),
    };
    assert!(!should_accept_authority_snapshot(&account, &snapshot));

    let newer_snapshot = LocalCodexOAuthSnapshot {
        last_refresh_at: Some(3000),
        ..snapshot
    };
    assert!(should_accept_authority_snapshot(&account, &newer_snapshot));
}

#[test]
fn authority_snapshot_with_newer_marker_but_older_access_token_is_rejected() {
    let mut account = CodexAccount::new(
        "codex_test_monotonic".to_string(),
        "demo@example.com".to_string(),
        make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "current",
            "rt-current",
        ),
    );
    account.account_id = Some("acc-current".to_string());
    account.organization_id = Some("org-current".to_string());
    account.tokens.access_token = make_jwt(serde_json::json!({
        "exp": 20_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    account.token_updated_at = Some(2_000);

    let mut snapshot_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "older",
        "rt-older",
    );
    snapshot_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 10_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let snapshot = LocalCodexOAuthSnapshot {
        tokens: snapshot_tokens,
        email: "demo@example.com".to_string(),
        user_id: Some("user-current".to_string()),
        subscription_active_until: None,
        account_id: Some("acc-current".to_string()),
        organization_id: Some("org-current".to_string()),
        last_refresh_at: Some(3_000),
    };

    assert!(!should_accept_authority_snapshot(&account, &snapshot));
}

#[test]
fn runtime_snapshot_freshness_prefers_latest_official_refresh() {
    let mut older_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "older-runtime",
        "rt-older-runtime",
    );
    older_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 10_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let older = LocalCodexOAuthSnapshot {
        tokens: older_tokens,
        email: "demo@example.com".to_string(),
        user_id: Some("user-current".to_string()),
        subscription_active_until: None,
        account_id: Some("acc-current".to_string()),
        organization_id: Some("org-current".to_string()),
        last_refresh_at: Some(2_000),
    };
    let mut newer_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "newer-runtime",
        "rt-newer-runtime",
    );
    newer_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 20_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let newer = LocalCodexOAuthSnapshot {
        tokens: newer_tokens,
        // 即使运行态文件的 last_refresh 标记较旧，也优先采用有效期更晚的 access_token。
        last_refresh_at: Some(1_000),
        ..older.clone()
    };

    assert!(
        super::local_oauth_snapshot_freshness_key(&newer)
            > super::local_oauth_snapshot_freshness_key(&older)
    );
}

#[test]
fn runtime_authority_sync_writes_only_the_newest_running_profile() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-runtime-authority-selection-test");
    let mut stored_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "stored",
        "rt-stored",
    );
    stored_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 5_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let account = seed_oauth_account(stored_tokens);

    let mut older_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "older-runtime",
        "rt-older-runtime",
    );
    older_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 10_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let older_dir = env.codex_home().join("instance-older");
    write_oauth_auth_file(&older_dir, &older_tokens, "acc-current");

    let mut newer_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "newer-runtime",
        "rt-newer-runtime",
    );
    newer_tokens.access_token = make_jwt(serde_json::json!({
        "exp": 20_000i64,
        "https://api.openai.com/auth": { "chatgpt_account_id": "acc-current" }
    }));
    let newer_dir = env.codex_home().join("instance-newer");
    write_oauth_auth_file(&newer_dir, &newer_tokens, "acc-current");

    assert!(
        super::sync_account_from_runtime_authority_dirs(&account.id, &[older_dir, newer_dir])
            .expect("sync newest runtime authority")
    );
    let persisted = load_account(&account.id).expect("load synced account");
    assert_eq!(persisted.tokens.access_token, newer_tokens.access_token);
    assert_eq!(
        persisted.tokens.refresh_token.as_deref(),
        newer_tokens.refresh_token.as_deref()
    );
}

#[test]
fn reauth_consumer_sync_keeps_running_official_profile_unchanged() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-reauth-local-store-only-test");
    let account = seed_oauth_account(make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "reauth-new",
        "rt-reauth-new",
    ));
    let runtime_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "runtime-current",
        "rt-runtime-current",
    );
    write_oauth_auth_file(&env.codex_home(), &runtime_tokens, "acc-current");

    tokio::runtime::Runtime::new()
        .expect("create runtime")
        .block_on(super::sync_bound_oauth_consumers_after_reauth(&account.id))
        .expect("sync reauthorized local consumers");

    let persisted_runtime = super::load_local_oauth_snapshot_from_official_store(&env.codex_home())
        .expect("official runtime snapshot");
    assert_eq!(
        persisted_runtime.tokens.access_token,
        runtime_tokens.access_token
    );
    assert_eq!(
        persisted_runtime.tokens.refresh_token,
        runtime_tokens.refresh_token
    );
    let persisted_account = load_account(&account.id).expect("local Cockpit account");
    assert_eq!(
        persisted_account.tokens.access_token,
        account.tokens.access_token
    );
}

#[test]
fn default_auth_store_prefers_auth_json_over_keychain() {
    let base_dir = make_temp_dir("codex-auth-store-file-priority-test");
    let file_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "file",
        "rt-file",
    );
    let keychain_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "keychain",
        "rt-keychain",
    );
    write_oauth_auth_file(&base_dir, &file_tokens, "acc-current");

    let snapshot = super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(
        &base_dir,
        |_| Ok(Some(build_oauth_auth_file(&keychain_tokens, "acc-current"))),
    )
    .expect("file auth snapshot");

    assert_eq!(snapshot.tokens.access_token, file_tokens.access_token);
    assert_eq!(snapshot.tokens.refresh_token.as_deref(), Some("rt-file"));
    fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
}

#[test]
fn explicit_keyring_auth_store_prefers_keychain() {
    let base_dir = make_temp_dir("codex-auth-store-keyring-priority-test");
    let file_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "file",
        "rt-file",
    );
    let keychain_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "keychain",
        "rt-keychain",
    );
    write_oauth_auth_file(&base_dir, &file_tokens, "acc-current");
    fs::write(
        base_dir.join("config.toml"),
        "cli_auth_credentials_store = \"keyring\"\n",
    )
    .expect("write keyring config");

    let snapshot = super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(
        &base_dir,
        |_| Ok(Some(build_oauth_auth_file(&keychain_tokens, "acc-current"))),
    )
    .expect("keychain auth snapshot");

    assert_eq!(snapshot.tokens.access_token, keychain_tokens.access_token);
    assert_eq!(
        snapshot.tokens.refresh_token.as_deref(),
        Some("rt-keychain")
    );
    fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
}

#[test]
fn switch_presync_persists_current_rotated_refresh_token_before_overwrite() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let env = TestEnvGuard::new("codex-switch-presync-current-auth-test");
    let mut current = seed_oauth_account(make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "stored",
        "rt-stored",
    ));
    current.token_updated_at = Some(1);
    save_account(&current).expect("make stored credential older than official refresh");
    let target = upsert_account(make_codex_tokens(
        "target@example.com",
        "acc-target",
        "org-target",
        "target",
        "rt-target",
    ))
    .expect("seed target account");
    assert_ne!(target.id, current.id);
    assert_eq!(
        load_account_index().current_account_id.as_deref(),
        Some(current.id.as_str())
    );

    let rotated_tokens = make_codex_tokens(
        "demo@example.com",
        "acc-current",
        "org-current",
        "rotated",
        "rt-rotated",
    );
    write_oauth_auth_file(&env.codex_home(), &rotated_tokens, "acc-current");

    assert!(
        super::sync_account_from_runtime_authority_dirs(&current.id, &[env.codex_home()])
            .expect("sync active official account")
    );
    let persisted = load_account(&current.id).expect("load current account after presync");
    assert_eq!(persisted.tokens.access_token, rotated_tokens.access_token);
    assert_eq!(
        persisted.tokens.refresh_token.as_deref(),
        Some("rt-rotated")
    );
}
