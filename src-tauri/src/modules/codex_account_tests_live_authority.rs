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
