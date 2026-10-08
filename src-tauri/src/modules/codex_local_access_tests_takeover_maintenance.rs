#[tokio::test]
async fn passive_takeover_preserves_user_selection_and_settings() {
    let profile = make_temp_dir("passive-takeover-settings");
    let mut collection = realtime_mixed_test_collection();
    collection.enabled = true;
    write_local_access_profile_takeover(&profile, &collection, None, true)
        .await
        .unwrap();
    let config_path = profile.join(CODEX_PROFILE_CONFIG_FILE);
    let mut doc = fs::read_to_string(&config_path)
        .unwrap()
        .parse::<Document>()
        .unwrap();
    doc["model"] = toml_edit::value("user-model");
    doc["model_context_window"] = toml_edit::value(123456);
    doc["model_auto_compact_token_limit"] = toml_edit::value(111111);
    fs::write(&config_path, doc.to_string()).unwrap();
    collection.port += 1;
    collection.api_key = "rotated-test-key".into();
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    let doc = fs::read_to_string(&config_path)
        .unwrap()
        .parse::<Document>()
        .unwrap();
    assert_eq!(doc["model"].as_str(), Some("user-model"));
    assert_eq!(doc["model_context_window"].as_integer(), Some(123456));
    assert_eq!(
        doc["model_auto_compact_token_limit"].as_integer(),
        Some(111111)
    );
    assert_eq!(
        doc["model_providers"]["codex_local_access"]["base_url"].as_str(),
        Some(super::build_collection_base_url(&collection).as_str())
    );
    assert_eq!(
        doc["model_providers"]["codex_local_access"]["experimental_bearer_token"].as_str(),
        Some("rotated-test-key")
    );
    let auth: Value =
        serde_json::from_str(&fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap())
            .unwrap();
    assert_eq!(auth["OPENAI_API_KEY"].as_str(), Some("rotated-test-key"));
    // An unchanged reconciliation must not invalidate an official cache.
    fs::write(profile.join(CODEX_MODEL_CACHE_FILE), "cache sentinel").unwrap();
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_MODEL_CACHE_FILE)).unwrap(),
        "cache sentinel"
    );
    fs::remove_dir_all(profile).unwrap();
}

#[tokio::test]
async fn passive_takeover_never_reclaims_user_edited_connection() {
    let mut collection = realtime_mixed_test_collection();
    collection.enabled = true;
    for edit in ["provider", "base_url", "key", "catalog", "removed"] {
        let profile = make_temp_dir(&format!("passive-takeover-{edit}"));
        write_local_access_profile_takeover(&profile, &collection, None, true)
            .await
            .unwrap();
        let config_path = profile.join(CODEX_PROFILE_CONFIG_FILE);
        let mut doc = fs::read_to_string(&config_path)
            .unwrap()
            .parse::<Document>()
            .unwrap();
        match edit {
            "provider" => doc["model_provider"] = toml_edit::value("user-provider"),
            "base_url" => {
                doc["model_providers"]["codex_local_access"]["base_url"] =
                    toml_edit::value("https://user.example/v1")
            }
            "key" => {
                doc["model_providers"]["codex_local_access"]["experimental_bearer_token"] =
                    toml_edit::value("user-key")
            }
            "catalog" => doc["model_catalog_json"] = toml_edit::value("user-models.json"),
            _ => {
                doc.remove("model_provider");
            }
        }
        let changed = doc.to_string();
        fs::write(&config_path, &changed).unwrap();
        super::maintain_local_access_profile(&profile, &collection).unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), changed, "{edit}");
        assert!(!super::cleanup_profile_takeover_without_backup(
            &profile,
            &collection.api_key,
            true
        )
        .unwrap());
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            changed,
            "disable {edit}"
        );
        fs::remove_dir_all(profile).unwrap();
    }
}

#[test]
fn passive_takeover_does_not_claim_missing_or_unbound_profile() {
    let profile = make_temp_dir("passive-takeover-unbound");
    let mut collection = realtime_mixed_test_collection();
    collection.enabled = true;
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    assert!(!profile.join(CODEX_PROFILE_CONFIG_FILE).exists());
    let config = "model_provider = \"user-provider\"\n";
    fs::write(profile.join(CODEX_PROFILE_CONFIG_FILE), config).unwrap();
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        config
    );
    assert!(!profile.join(super::TAKEOVER_OWNERSHIP_FILE).exists());
    fs::remove_dir_all(profile).unwrap();
}

#[tokio::test]
async fn passive_takeover_disabled_service_and_other_profiles_are_unchanged() {
    let first = make_temp_dir("passive-takeover-first");
    let second = make_temp_dir("passive-takeover-second");
    let mut collection = realtime_mixed_test_collection();
    write_local_access_profile_takeover(&first, &collection, None, true)
        .await
        .unwrap();
    write_local_access_profile_takeover(&second, &collection, None, true)
        .await
        .unwrap();
    let first_before = fs::read_to_string(first.join(CODEX_PROFILE_CONFIG_FILE)).unwrap();
    let second_before = fs::read_to_string(second.join(CODEX_PROFILE_CONFIG_FILE)).unwrap();
    collection.port += 1;
    collection.enabled = false;
    super::maintain_local_access_profile(&first, &collection).unwrap();
    assert_eq!(
        fs::read_to_string(first.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        first_before
    );
    collection.enabled = true;
    super::maintain_local_access_profile(&first, &collection).unwrap();
    assert_ne!(
        fs::read_to_string(first.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        first_before
    );
    assert_eq!(
        fs::read_to_string(second.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        second_before
    );
    fs::remove_dir_all(first).unwrap();
    fs::remove_dir_all(second).unwrap();
}

#[test]
fn passive_takeover_marker_failure_rolls_back_connection() {
    let profile = make_temp_dir("passive-takeover-rollback");
    let config = "model = \"original\"\n";
    let auth = "{\"OPENAI_API_KEY\":\"original-key\"}";
    fs::write(profile.join(CODEX_PROFILE_CONFIG_FILE), config).unwrap();
    fs::write(profile.join(CODEX_PROFILE_AUTH_FILE), auth).unwrap();
    let error = super::persist_takeover_connection(
        &profile,
        config,
        Some(auth),
        "model = \"changed\"\n",
        Some("{}"),
        || Err("injected marker failure".to_string()),
    )
    .unwrap_err();
    assert!(error.contains("injected marker failure"));
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap(),
        auth
    );
    fs::remove_dir_all(profile).unwrap();
}

#[tokio::test]
async fn passive_takeover_legacy_profile_requires_exact_connection_before_adoption() {
    let profile = make_temp_dir("passive-takeover-legacy");
    let mut collection = realtime_mixed_test_collection();
    collection.enabled = true;
    write_local_access_profile_takeover(&profile, &collection, None, true)
        .await
        .unwrap();
    fs::remove_file(profile.join(super::TAKEOVER_OWNERSHIP_FILE)).unwrap();
    collection.port += 1;
    let before = fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap();
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        before
    );
    assert!(!profile.join(super::TAKEOVER_OWNERSHIP_FILE).exists());
    collection.port -= 1;
    super::maintain_local_access_profile(&profile, &collection).unwrap();
    assert!(profile.join(super::TAKEOVER_OWNERSHIP_FILE).exists());
    fs::remove_dir_all(profile).unwrap();
}

#[tokio::test]
async fn passive_takeover_disable_does_not_restore_over_user_auth() {
    let profile = make_temp_dir("passive-takeover-user-auth");
    let collection = realtime_mixed_test_collection();
    write_local_access_profile_takeover(&profile, &collection, None, true)
        .await
        .unwrap();
    let user_auth = "{\"auth_mode\":\"apikey\",\"OPENAI_API_KEY\":\"user-new-key\"}";
    fs::write(profile.join(CODEX_PROFILE_AUTH_FILE), user_auth).unwrap();
    let backup = CodexLocalAccessProfileTakeoverBackup {
        profile_dir: profile.to_string_lossy().to_string(),
        auth_json: Some("{\"OPENAI_API_KEY\":\"old-key\"}".into()),
        config_toml: Some("model_provider = \"old-provider\"\n".into()),
        created_at: 0,
        updated_at: 0,
    };
    assert!(restore_profile_takeover_backup(&backup, &collection.api_key, true).unwrap());
    assert_eq!(
        fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap(),
        user_auth
    );
    fs::remove_dir_all(profile).unwrap();
}

fn takeover_restore_oauth_account(label: &str) -> CodexAccount {
    let account_id = format!("takeover-{label}");
    let email = format!("{label}@takeover.example.test");
    let mut account = CodexAccount::new(
        account_id.clone(),
        email.clone(),
        CodexTokens {
            id_token: make_test_jwt(json!({
                "email": email, "exp": 4_102_444_800i64,
                "https://api.openai.com/auth": { "chatgpt_account_id": account_id }
            })),
            access_token: make_test_jwt(json!({"sub": label, "exp": 4_102_444_800i64})),
            refresh_token: Some(format!("refresh-{label}")),
        },
    );
    account.account_id = Some(account_id);
    account
}

#[tokio::test]
async fn passive_takeover_restores_owned_oauth_without_replaying_old_refresh_tokens() {
    let _lock = crate::modules::test_support::env_lock().lock().unwrap();
    let _env = LocalAccessTestDataGuard::new("takeover-oauth-restore");
    for scenario in [
        "different",
        "rotated",
        "same",
        "newer-backup-owner",
        "legacy-marker",
        "external",
        "unknown-owner",
    ] {
        let profile = make_temp_dir(&format!("takeover-oauth-{scenario}"));
        let mut original = takeover_restore_oauth_account(&format!("original-{scenario}"));
        let owner = if scenario == "same" {
            original.clone()
        } else {
            takeover_restore_oauth_account(&format!("owner-{scenario}"))
        };
        for account in [&original, &owner] {
            codex_account::save_account(account).unwrap();
        }
        // Built-in OAuth does not need a provider table and may leave a fresh
        // profile without config.toml. Model an existing user's configured profile.
        fs::write(
            profile.join(CODEX_PROFILE_CONFIG_FILE),
            "model = \"gpt-5.5\"\n",
        )
        .unwrap();
        codex_account::write_account_bundle_to_dir(&profile, &original).unwrap();
        let backup = CodexLocalAccessProfileTakeoverBackup {
            profile_dir: profile.to_string_lossy().to_string(),
            auth_json: Some(fs::read_to_string(profile.join(CODEX_PROFILE_AUTH_FILE)).unwrap()),
            config_toml: Some(fs::read_to_string(profile.join(CODEX_PROFILE_CONFIG_FILE)).unwrap()),
            created_at: 0,
            updated_at: 0,
        };
        let mut collection = realtime_mixed_test_collection();
        collection.bound_oauth_account_id = Some(owner.id.clone());
        collection.account_ids = vec![owner.id.clone()];
        write_local_access_profile_takeover(&profile, &collection, None, true)
            .await
            .unwrap();
        let auth_path = profile.join(CODEX_PROFILE_AUTH_FILE);
        let mut live: Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        if matches!(scenario, "rotated" | "same") {
            live["tokens"]["refresh_token"] = json!("rotated-live-refresh");
            live["tokens"]["access_token"] = json!(make_test_jwt(
                json!({"sub": "rotated", "exp": 4_102_448_400i64})
            ));
            fs::write(&auth_path, serde_json::to_string_pretty(&live).unwrap()).unwrap();
        }
        if scenario == "external" {
            let external = takeover_restore_oauth_account("external-login");
            // Simulate official login: replace only auth.json, leaving the old projection stale.
            live["tokens"]["id_token"] = json!(external.tokens.id_token);
            live["tokens"]["access_token"] = json!(external.tokens.access_token);
            live["tokens"]["refresh_token"] = json!(external.tokens.refresh_token);
            live["tokens"]["account_id"] = json!(external.account_id);
            fs::write(&auth_path, serde_json::to_string_pretty(&live).unwrap()).unwrap();
        }
        if scenario == "newer-backup-owner" {
            original.token_generation += 1;
            original.token_updated_at = Some(chrono::Utc::now().timestamp() + 1);
            original.tokens.refresh_token = Some("newer-stored-refresh".to_string());
            original.tokens.access_token =
                make_test_jwt(json!({"sub": "newer-original", "exp": 4_102_452_000i64}));
            codex_account::save_account(&original).unwrap();
        }
        if scenario == "legacy-marker" {
            fs::remove_file(profile.join(super::TAKEOVER_OWNERSHIP_FILE)).unwrap();
        }
        if scenario == "unknown-owner" {
            let projection_path = profile.join(super::CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE);
            let mut projection: Value =
                serde_json::from_str(&fs::read_to_string(&projection_path).unwrap()).unwrap();
            projection
                .as_object_mut()
                .unwrap()
                .remove("credential_account_id");
            fs::write(projection_path, projection.to_string()).unwrap();
        }
        // Ensure the fixture's original identity is discoverable without relying on index repair.
        let mut index = codex_account::load_account_index();
        if !index.accounts.iter().any(|entry| entry.id == original.id) {
            index
                .accounts
                .push(crate::models::codex::CodexAccountSummary {
                    id: original.id.clone(),
                    email: original.email.clone(),
                    plan_type: None,
                    subscription_active_until: None,
                    created_at: 0,
                    last_used: 0,
                });
            codex_account::save_account_index(&index).unwrap();
        }
        let before_restore = fs::read_to_string(&auth_path).unwrap();
        assert!(restore_profile_takeover_backup(&backup, &collection.api_key, true).unwrap());
        let after: Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        let expected_refresh = match scenario {
            "same" => "rotated-live-refresh",
            "newer-backup-owner" => "newer-stored-refresh",
            "external" | "unknown-owner" => {
                assert_eq!(
                    fs::read_to_string(&auth_path).unwrap(),
                    before_restore,
                    "{scenario}"
                );
                fs::remove_dir_all(profile).unwrap();
                continue;
            }
            _ => original.tokens.refresh_token.as_deref().unwrap(),
        };
        assert_eq!(
            after["tokens"]["refresh_token"], expected_refresh,
            "{scenario}"
        );
        assert_eq!(
            after["tokens"]["account_id"],
            original.account_id.as_deref().unwrap(),
            "{scenario}"
        );
        if matches!(scenario, "rotated" | "same") {
            assert_eq!(
                codex_account::load_account(&owner.id)
                    .unwrap()
                    .tokens
                    .refresh_token
                    .as_deref(),
                Some("rotated-live-refresh")
            );
        }
        assert!(!profile.join(super::TAKEOVER_OWNERSHIP_FILE).exists());
        fs::remove_dir_all(profile).unwrap();
    }
}

#[tokio::test]
async fn passive_takeover_explicit_oauth_binding_refreshes_only_owned_profiles() {
    let owned = make_temp_dir("passive-takeover-explicit-owned");
    let user = make_temp_dir("passive-takeover-explicit-user");
    let mut previous = realtime_mixed_test_collection();
    previous.enabled = true;
    write_local_access_profile_takeover(&owned, &previous, None, true)
        .await
        .unwrap();
    let custom_config = "model_provider = \"user-provider\"\n";
    fs::write(user.join(CODEX_PROFILE_CONFIG_FILE), custom_config).unwrap();
    let mut next = previous.clone();
    next.port += 1;
    super::refresh_owned_takeovers_after_explicit_oauth_binding(
        vec![owned.clone(), user.clone()],
        &previous,
        &next,
    )
    .await
    .unwrap();
    assert!(super::inspect_local_access_profile_attachment(&owned, Some(&next)).attached);
    assert_eq!(
        fs::read_to_string(user.join(CODEX_PROFILE_CONFIG_FILE)).unwrap(),
        custom_config
    );
    assert!(!user.join(CODEX_PROFILE_AUTH_FILE).exists());
    fs::remove_dir_all(owned).unwrap();
    fs::remove_dir_all(user).unwrap();
}
