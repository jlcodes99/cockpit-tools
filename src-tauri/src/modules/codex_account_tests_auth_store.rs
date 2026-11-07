// Regression tests for authoritative storage selection. Never access the OS keyring.
#[test]
fn auth_store_keyring_does_not_fall_back_to_a_stale_file() {
    let dir = make_temp_dir("auth-keyring-no-stale-fallback");
    let tokens = make_codex_tokens(
        "old@example.com",
        "old-account",
        "org",
        "old",
        "old-refresh",
    );
    write_oauth_auth_file(&dir, &tokens, "old-account");
    fs::write(
        dir.join("config.toml"),
        "cli_auth_credentials_store = \"keyring\"\n",
    )
    .unwrap();
    for unavailable in [false, true] {
        assert!(
            super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(&dir, |_| {
                if unavailable {
                    Err("keyring unavailable".to_string())
                } else {
                    Ok(None)
                }
            })
            .is_none()
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn auth_store_auto_keeps_secure_identity_even_if_a_stale_file_has_api_key() {
    let dir = make_temp_dir("auth-auto-secure-before-api-file");
    fs::write(
        dir.join("auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"old-key"}"#,
    )
    .unwrap();
    fs::write(
        dir.join("config.toml"),
        "cli_auth_credentials_store = \"auto\"\n",
    )
    .unwrap();
    let tokens = make_codex_tokens(
        "new@example.com",
        "new-account",
        "org",
        "new",
        "new-refresh",
    );
    let snapshot =
        super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(&dir, |_| {
            Ok(Some(build_oauth_auth_file(&tokens, "new-account")))
        })
        .unwrap();
    assert_eq!(snapshot.tokens.access_token, tokens.access_token);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn auth_store_auto_uses_file_only_when_secure_store_is_absent_or_unreadable() {
    let dir = make_temp_dir("auth-auto-file-fallback");
    let tokens = make_codex_tokens("file@example.com", "account", "org", "file", "refresh");
    write_oauth_auth_file(&dir, &tokens, "account");
    fs::write(
        dir.join("config.toml"),
        "cli_auth_credentials_store = \"auto\"\n",
    )
    .unwrap();
    for unavailable in [false, true] {
        let snapshot =
            super::load_local_oauth_snapshot_from_official_store_with_keychain_reader(&dir, |_| {
                if unavailable {
                    Err("keyring unavailable".to_string())
                } else {
                    Ok(None)
                }
            })
            .unwrap();
        assert_eq!(snapshot.tokens.access_token, tokens.access_token);
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn auth_store_ephemeral_neither_reads_nor_overwrites_persistent_credentials() {
    let dir = make_temp_dir("auth-ephemeral-no-persistent-write");
    fs::write(
        dir.join("config.toml"),
        "cli_auth_credentials_store = \"ephemeral\"\n",
    )
    .unwrap();
    fs::write(dir.join("auth.json"), "original").unwrap();
    assert!(super::read_configured_codex_auth_value(&dir)
        .unwrap()
        .is_none());
    assert!(super::write_auth_value_to_configured_store(
        &dir,
        &dir.join("auth.json"),
        &serde_json::json!({})
    )
    .unwrap_err()
    .contains("EPHEMERAL"));
    assert_eq!(
        fs::read_to_string(dir.join("auth.json")).unwrap(),
        "original"
    );
    fs::remove_dir_all(dir).unwrap();
}
