    #[test]
    fn responses_bound_oauth_api_key_uses_compatibility_gateway_for_http_only_provider() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.api_supports_websockets = false;

        assert!(account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account));

        account.bound_oauth_use_local_gateway = true;
        assert!(account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account));

        account.api_supports_websockets = true;
        assert!(!account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account));

        account.api_supports_websockets = false;
        account.bound_oauth_account_id = None;
        assert!(!account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
        assert!(is_local_access_eligible_account(&account));
    }

    #[test]
    fn chat_bound_oauth_api_key_keeps_provider_gateway_branch() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "chat@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;

        assert!(account_requires_provider_gateway(&account));
        assert!(!account_requires_bound_oauth_local_gateway(&account));
    }

    #[test]
    fn responses_api_key_does_not_require_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "openai@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.openai.com/v1".to_string()),
            Some("openai".to_string()),
            Some("OpenAI".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        assert!(!account_requires_provider_gateway(&account));
    }

    #[test]
    fn provider_gateway_models_prefers_account_catalog() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_model_catalog.push(" ".to_string());

        assert_eq!(
            provider_gateway_models_for_account(&account),
            vec![
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string()
            ]
        );
    }

    #[test]
    fn provider_gateway_models_fill_empty_catalog_for_exact_official_endpoints() {
        for (base_url, expected_model) in [
            ("https://ark.cn-beijing.volces.com/api/coding/v3", "ark-code-latest"),
            ("https://ark.cn-beijing.volces.com/api/v3", "doubao-seed-2-1-pro-260915"),
            ("https://api.xiaomimimo.com/v1", "mimo-v2.6-pro"),
            ("https://token-plan-cn.xiaomimimo.com/v1/", "mimo-v2.6-pro"),
        ] {
            let mut account = CodexAccount::new_api_key(
                "local-account-id".to_string(),
                "custom@example.com".to_string(),
                "sk-test".to_string(),
                CodexApiProviderMode::Custom,
                Some(base_url.to_string()),
                None,
                None,
                Vec::new(),
            );
            assert_eq!(provider_gateway_default_model_for_account(&account), expected_model);

            account.api_model_catalog = vec!["user-selected-model".to_string()];
            assert_eq!(provider_gateway_models_for_account(&account), vec!["user-selected-model"]);

            account.api_model_catalog.clear();
            account.api_base_url = Some(format!("https://relay.example/{base_url}"));
            assert!(provider_gateway_models_for_account(&account).is_empty());
        }
    }

    #[test]
    fn provider_gateway_models_empty_for_unknown_provider_without_catalog() {
        let account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "custom@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example-provider.test/v1".to_string()),
            Some("custom-provider".to_string()),
            Some("Custom Provider".to_string()),
            Vec::new(),
        );

        assert!(provider_gateway_models_for_account(&account).is_empty());
        assert!(provider_gateway_default_model_for_account(&account).is_empty());
    }

    #[test]
    fn provider_gateway_model_slots_are_stable_and_bounded() {
        let slots = provider_gateway_model_slots(&[
            "vendor-pro".to_string(),
            "vendor-flash".to_string(),
            "vendor-lite".to_string(),
            "vendor-extra".to_string(),
            "gpt-5.5".to_string(),
            "custom-overflow-a".to_string(),
            "custom-overflow-b".to_string(),
            "custom-overflow-c".to_string(),
            "custom-overflow-d".to_string(),
            "custom-overflow-e".to_string(),
            "custom-overflow-f".to_string(),
        ]);

        assert_eq!(
            slots
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                // Explicit API-key models retain identity ahead of assigned shells.
                ("gpt-5.5", "gpt-5.5"),
                ("gpt-5.6-sol", "vendor-pro"),
                ("gpt-5.6-terra", "vendor-flash"),
                ("gpt-5.6-luna", "vendor-lite"),
                ("vendor-extra", "vendor-extra"),
                ("custom-overflow-a", "custom-overflow-a"),
                ("custom-overflow-b", "custom-overflow-b"),
                ("custom-overflow-c", "custom-overflow-c"),
                ("custom-overflow-d", "custom-overflow-d"),
                // Shell pool exhausted: keep upstream IDs so all models remain listed.
                ("custom-overflow-e", "custom-overflow-e"),
                ("custom-overflow-f", "custom-overflow-f"),
            ]
        );
        assert!(provider_model_slots_need_upstream_rewrite(&slots));
    }

    #[test]
    fn provider_gateway_model_slots_keep_identity_for_official_shells() {
        let slots = provider_gateway_model_slots(&[
            "gpt-5.6-sol".to_string(),
            "gpt-5.6-luna".to_string(),
            "grok-4.5".to_string(),
        ]);
        assert_eq!(
            slots
                .iter()
                .map(|slot| (slot.client_model.as_str(), slot.upstream_model.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("gpt-5.6-sol", "gpt-5.6-sol"),
                ("gpt-5.6-luna", "gpt-5.6-luna"),
                ("gpt-5.6-terra", "grok-4.5"),
            ]
        );
    }

    #[test]
    fn provider_gateway_model_slots_keep_identity_for_gpt_6_family() {
        let slots = provider_gateway_model_slots(&[
            "gpt-6.1-sol".to_string(),
            "gpt-6-astra".to_string(),
            "gpt-6-sol".to_string(),
            "gpt-6-luna".to_string(),
        ]);
        assert_eq!(
            slots,
            vec![
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-6.1-sol".to_string(),
                    upstream_model: "gpt-6.1-sol".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-6-astra".to_string(),
                    upstream_model: "gpt-6-astra".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-6-sol".to_string(),
                    upstream_model: "gpt-6-sol".to_string(),
                },
                super::ProviderGatewayModelSlot {
                    client_model: "gpt-6-luna".to_string(),
                    upstream_model: "gpt-6-luna".to_string(),
                },
            ]
        );
    }

    #[test]
    fn responses_sync_catalog_with_custom_models_requires_instance_gateway_but_remains_local_access_eligible(
    ) {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fun/v1".to_string()),
            Some("apikey_fun".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["grok-4.5".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(account_requires_provider_gateway(&account));
        assert!(is_local_access_eligible_account(&account));
        assert_eq!(local_access_ineligible_reason(&account), None);

        let account_id = account.id.clone();
        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![account_id.clone()],
            &[account],
        );
        assert_eq!(next_ids, vec![account_id.clone()]);
        assert_eq!(synced_ids, vec![account_id.clone()]);
        assert_eq!(added_ids, vec![account_id]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn responses_sync_catalog_with_only_official_shells_skips_provider_gateway() {
        let mut account = CodexAccount::new_api_key(
            "local-account-id".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fun/v1".to_string()),
            Some("apikey_fun".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["gpt-5.6-luna".to_string(), "gpt-5.6-sol".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(!account_requires_provider_gateway(&account));
    }

    #[test]
    fn provider_gateway_writes_static_catalog_for_chat_completions_models() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-override-state-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let slots = provider_gateway_model_slots(&[
            "deepseek-v4-pro".to_string(),
            "deepseek-v4-flash".to_string(),
        ]);
        let client_models = slots
            .iter()
            .map(|slot| slot.client_model.clone())
            .collect::<Vec<_>>();
        backup_current_profile_model_before_provider_gateway(&profile_dir, &client_models)
            .expect("track provider models");
        write_local_access_profile_model_override(&profile_dir, "deepseek-v4-flash")
            .expect("write model override");
        write_provider_gateway_model_catalog(&profile_dir, &slots)
            .expect("write provider model catalog");

        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE))
                .expect("read provider model catalog"),
        )
        .expect("parse provider model catalog");
        let models = catalog
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");
        for (model_id, display_name) in [
            ("deepseek-v4-flash", "deepseek-v4-flash"),
            ("deepseek-v4-pro", "deepseek-v4-pro"),
        ] {
            assert!(models.iter().any(|model| {
                model.get("slug").and_then(Value::as_str) == Some(model_id)
                    && model.get("display_name").and_then(Value::as_str) == Some(display_name)
                    && model.get("visibility").and_then(Value::as_str) == Some("list")
            }));
        }
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        )));
        assert!(config.contains("model = \"deepseek-v4-flash\""));

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-flash\""));
        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_removes_managed_model_override() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-cleanup-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        fs::write(
            profile_dir.join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"deepseek-v4-pro"},{"slug":"deepseek-v4-flash"}]}"#,
        )
        .expect("write stale model catalog");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            format!(
                "model_catalog_json = \"{}\"\nmodel = \"deepseek-v4-pro\"\n",
                CODEX_PROVIDER_MODEL_CATALOG_FILE
            ),
        )
        .expect("write stale config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_previous_official_model() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write official model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &[
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
            ],
        )
        .expect("backup official model");
        write_local_access_profile_model_override(&profile_dir, "deepseek-v4-pro")
            .expect("write provider model");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE).exists());
        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!config.contains("gpt-5.5"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_previous_model_without_catalog_file() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-no-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write official model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &[
                "deepseek-v4-pro".to_string(),
                "deepseek-v4-flash".to_string(),
            ],
        )
        .expect("backup official model");
        write_local_access_profile_model_override(&profile_dir, "deepseek-v4-pro")
            .expect("write provider model");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let existing = fs::read_to_string(&config_path).expect("read config");
        let mut doc = existing.parse::<Document>().expect("parse config");
        doc["model_catalog_json"] = value(CODEX_PROVIDER_MODEL_CATALOG_FILE);
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        write_string_atomic(&config_path, &content).expect("write config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert!(!profile_dir.join(CODEX_PROVIDER_MODEL_BACKUP_FILE).exists());
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!config.contains("gpt-5.5"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_restores_original_model_matching_provider_shell() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-matching-shell-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write original model");
        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &["gpt-5.5".to_string(), "gpt-5.4".to_string()],
        )
        .expect("backup original model");
        write_provider_gateway_model_catalog(
            &profile_dir,
            &provider_gateway_model_slots(&[
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ]),
        )
        .expect("write provider catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!config.contains("gpt-5.5"));
        assert!(!config.contains("model_catalog_json"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_reapplies_enabled_experimental_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-restore-experimental-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            "model = \"gpt-5.6-sol\"\n",
        )
        .expect("write original config");
        crate::modules::codex_account::save_quick_config_for_base_dir(
            &profile_dir,
            None,
            None,
            Some(true),
            None,
        )
        .expect("enable experimental catalog");

        backup_current_profile_model_before_provider_gateway(
            &profile_dir,
            &["gpt-5.5".to_string(), "gpt-5.4".to_string()],
        )
        .expect("backup original model");
        write_local_access_profile_model_override(&profile_dir, "gpt-5.5")
            .expect("write provider model");
        write_provider_gateway_model_catalog(
            &profile_dir,
            &provider_gateway_model_slots(&[
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ]),
        )
        .expect("write provider catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        )));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(profile_dir
            .join(CODEX_PROVIDER_MODEL_CATALOG_FILE)
            .is_file());
        assert!(
            crate::modules::codex_account::read_quick_config_from_config_toml(&profile_dir)
                .expect("read quick config")
                .experimental_model_catalog_enabled
        );

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_keeps_non_cockpit_model_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-keep-external-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        fs::write(
            &config_path,
            r#"model_provider = "ccswitch_deepseek"
model_catalog_json = "cc-switch-model-catalog.json"
model = "deepseek-v4-pro"

[model_providers.ccswitch_deepseek]
name = "CCSwitch DeepSeek"
base_url = "https://deepseek.example.com/v1"
wire_api = "responses"
"#,
        )
        .expect("write config");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        let config = fs::read_to_string(config_path).expect("read config");
        assert!(config.contains("model_catalog_json = \"cc-switch-model-catalog.json\""));
        assert!(config.contains("model_provider = \"ccswitch_deepseek\""));
        assert!(config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("[model_providers.ccswitch_deepseek]"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_gateway_cleanup_preserves_unowned_managed_model_catalog() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-model-preserve-unowned-catalog-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let catalog_path = profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE);
        let config = format!(
            "model_catalog_json = \"{}\"\nmodel = \"custom-model\"\nmodel_context_window = 1000000\n",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        );
        let catalog = r#"{"models":[{"slug":"custom-model"}]}"#;
        fs::write(&config_path, &config).expect("write config");
        fs::write(&catalog_path, catalog).expect("write catalog");

        cleanup_provider_gateway_profile_model_overrides(&profile_dir).expect("cleanup overrides");

        assert_eq!(
            fs::read_to_string(&config_path).expect("read config"),
            config
        );
        assert_eq!(
            fs::read_to_string(&catalog_path).expect("read catalog"),
            catalog
        );

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn provider_takeover_cleanup_preserves_local_access_provider_for_history_sessions() {
        let profile_dir = std::env::temp_dir().join(format!(
            "cockpit-provider-preserve-history-provider-test-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&profile_dir).expect("create temp profile");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let config_content = r#"model_provider = "codex_local_access"
model = "gpt-5.6-sol"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "Codex Local Access"
base_url = "http://127.0.0.1:51525/v1"
wire_api = "responses"
requires_openai_auth = true
http_headers = { "x-cockpit-instance-id" = "default" }
"#;
        fs::write(&config_path, config_content).expect("write initial config");

        let cleaned = remove_codex_local_access_config(config_content).expect("cleanup config");
        assert!(!cleaned.contains("model_provider = \"codex_local_access\""));
        assert!(cleaned.contains("model_context_window = 1000000"));
        assert!(cleaned.contains("[model_providers.codex_local_access]"));
        assert!(cleaned.contains("base_url = \"http://127.0.0.1:51525/v1\""));
        assert!(cleaned.contains("wire_api = \"responses\""));
        assert!(!cleaned.contains("x-cockpit-instance-id"));

        let _ = fs::remove_dir_all(&profile_dir);
    }

    #[test]
    fn normalizes_account_model_rules_for_collection_accounts() {
        let rules = normalize_account_model_rules(
            vec![
                CodexLocalAccessAccountModelRule {
                    account_id: " account-a ".to_string(),
                    excluded_models: vec!["gpt-5.4-mini".to_string(), "GPT-5.4-MINI".to_string()],
                },
                CodexLocalAccessAccountModelRule {
                    account_id: "account-b".to_string(),
                    excluded_models: vec!["".to_string(), "gpt-5.3-*".to_string()],
                },
                CodexLocalAccessAccountModelRule {
                    account_id: "missing".to_string(),
                    excluded_models: vec!["gpt-5.2".to_string()],
                },
            ],
            &["account-a".to_string(), "account-b".to_string()],
        );

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].account_id, "account-a");
        assert_eq!(rules[0].excluded_models, vec!["gpt-5.4-mini"]);
        assert_eq!(rules[1].account_id, "account-b");
        assert_eq!(rules[1].excluded_models, vec!["gpt-5.3-*"]);
    }

    #[test]
    fn remove_account_refs_clears_all_local_access_references() {
        let mut collection = test_local_access_collection(vec![
            "account-a".to_string(),
            "account-b".to_string(),
            "account-c".to_string(),
        ]);
        let mut scoped_key = build_local_access_api_key(Some("scoped"));
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string(), "account-c".to_string()];
        collection.api_keys = vec![scoped_key];
        collection.custom_routing_rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "account-b".to_string(),
                priority: 10,
                weight: 2,
                is_backup: false,
                is_preferred: false,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "account-c".to_string(),
                priority: 5,
                weight: 1,
                is_backup: false,
                is_preferred: false,
            },
        ];
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-b".to_string(),
            excluded_models: vec!["gpt-5.4-mini".to_string()],
        }];
        collection.bound_oauth_account_id = Some("account-b".to_string());
        collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
            hourly_percent: 20,
            weekly_percent: 30,
            strict: false, safety_margin_percent: 3,
});

        let changed = remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        );

        assert!(changed);
        assert_eq!(
            collection.account_ids,
            vec!["account-a".to_string(), "account-c".to_string()]
        );
        assert_eq!(collection.api_keys[0].account_ids, vec!["account-c"]);
        assert_eq!(collection.custom_routing_rules.len(), 1);
        assert_eq!(collection.custom_routing_rules[0].account_id, "account-c");
        assert!(collection.account_model_rules.is_empty());
        assert!(collection.bound_oauth_account_id.is_none());
        assert!(collection.bound_oauth_quota_reserve.is_none());
    }

    #[test]
    fn custom_scope_does_not_broaden_when_last_account_is_removed() {
        let mut collection =
            test_local_access_collection(vec!["account-a".to_string(), "account-b".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("scoped"));
        scoped_key.key = "scoped-key".to_string();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        ));

        let api_key = &collection.api_keys[0];
        assert!(!api_key_inherits_account_pool(api_key));
        assert!(api_key.account_ids.is_empty());
        assert!(effective_api_key_account_ids(&collection, api_key).is_empty());
        assert!(sidecar_api_key_manifest_values(&collection)
            .iter()
            .all(|value| value.get("key").and_then(Value::as_str) != Some("scoped-key")));
    }

    #[test]
    fn deleted_scoped_account_does_not_fall_back_to_legacy_sidecar_pool() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("Scoped"));
        scoped_key.key = collection.api_key.clone();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["account-b".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(remove_account_refs_from_collection(
            &mut collection,
            &HashSet::from(["account-b".to_string()]),
        ));

        assert!(collection.api_keys[0].account_ids.is_empty());
        assert!(sidecar_api_key_manifest_values(&collection)
            .iter()
            .all(|value| value.get("key").and_then(Value::as_str) != Some("local-api-key")));
        assert!(!sidecar_client_api_keys(&collection, &HashMap::new())
            .iter()
            .any(|key| key == "local-api-key"));
        assert!(
            sidecar_api_key_account_scope_values(&collection, &HashMap::new())
                .get("local-api-key")
                .is_none()
        );
    }

    #[test]
    fn sidecar_excludes_key_with_unresolved_custom_scope() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        let mut scoped_key = build_local_access_api_key(Some("Scoped"));
        scoped_key.key = "scoped-key".to_string();
        scoped_key.inherit_account_pool = Some(false);
        scoped_key.account_ids = vec!["missing-account".to_string()];
        collection.api_keys = vec![scoped_key];

        assert!(!sidecar_client_api_keys(&collection, &HashMap::new())
            .iter()
            .any(|key| key == "scoped-key"));
        assert!(
            sidecar_api_key_account_scope_values(&collection, &HashMap::new())
                .get("scoped-key")
                .is_none()
        );
    }

    #[test]
    fn bound_oauth_gateway_key_rejects_inherited_or_empty_scope_updates() {
        let account_id = "api-bound-oauth-1".to_string();
        let mut collection = test_local_access_collection(Vec::new());
        collection.bound_oauth_account_id = Some("oauth-1".to_string());

        let mut api_key = build_local_access_api_key(Some("Bound OAuth Local Gateway"));
        api_key.id = provider_gateway_api_key_id(&account_id);
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![account_id.clone()];

        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[]),
            Some(true),
        )
        .is_err());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[]),
            Some(false),
        )
        .is_err());
        assert!(
            validate_api_key_account_scope_update(&collection, &api_key, None, Some(true),)
                .is_err()
        );
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[account_id.clone()]),
            Some(false),
        )
        .is_ok());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&["api-bound-oauth-2".to_string()]),
            Some(false),
        )
        .is_err());
        assert!(validate_api_key_account_scope_update(
            &collection,
            &api_key,
            Some(&[account_id.clone(), "api-bound-oauth-2".to_string()]),
            Some(false),
        )
        .is_err());

        let regular_collection = test_local_access_collection(vec!["account-a".to_string()]);
        let regular_key = build_local_access_api_key(Some("Regular Key"));
        assert!(validate_api_key_account_scope_update(
            &regular_collection,
            &regular_key,
            None,
            Some(true),
        )
        .is_ok());
    }

    #[test]
    fn custom_scope_model_visibility_uses_scoped_accounts_for_image_capacity() {
        let mut paid_account = test_account_with_plan("pro");
        paid_account.id = "account-paid".to_string();
        let mut free_account = test_account_with_plan("free");
        free_account.id = "account-free".to_string();
        let collection = test_local_access_collection(vec![paid_account.id.clone()]);
        let api_key = ResolvedLocalApiKey {
            id: "key-scoped".to_string(),
            label: "Scoped".to_string(),
            provider_gateway: None,
            inherit_account_pool: false,
            account_ids: vec![free_account.id.clone()],
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };

        let models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[paid_account, free_account],
            None,
        );

        assert!(!models.iter().any(|model| model == CODEX_IMAGE_MODEL_ID));
    }

    #[test]
    fn sidecar_fingerprint_tracks_account_local_proxy_port() {
        let config = r#"{"host":"127.0.0.1","port":12345}"#;
        let manifest = r#"{"accounts":[{"id":"account-a","email":"a@example.com"}]}"#;
        let before = vec![account_proxy_fingerprint(Some(
            "socks5h://local-user:local-secret@127.0.0.1:58887",
        ))];
        let after = vec![account_proxy_fingerprint(Some(
            "socks5h://local-user:local-secret@127.0.0.1:64202",
        ))];

        assert_ne!(
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &before),
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &after),
            "a restarted account tunnel must rebuild the sidecar instead of dialling the old port"
        );
        assert_eq!(
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &before),
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &before.clone()),
            "an unchanged tunnel must not restart active streams"
        );
        assert_eq!(
            sidecar_config_fingerprint(config, manifest),
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &[]),
            "accounts without a local proxy keep the historical fingerprint"
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_account_proxy_order_but_not_absence() {
        let config = r#"{"host":"127.0.0.1","port":12345}"#;
        let manifest = r#"{"accounts":[{"id":"account-a"},{"id":"account-b"}]}"#;
        let first = account_proxy_fingerprint(Some("http://127.0.0.1:1"));
        let second = account_proxy_fingerprint(Some("http://127.0.0.1:2"));
        let unordered = vec![first.clone(), second.clone()];
        let reversed = vec![second.clone(), first.clone()];

        assert_eq!(
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &unordered),
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &reversed),
            "pool ordering must not churn the gateway"
        );
        assert_ne!(
            sidecar_config_fingerprint_with_account_proxies(config, manifest, &unordered),
            sidecar_config_fingerprint_with_account_proxies(
                config,
                manifest,
                &[first.clone(), account_proxy_fingerprint(None)]
            ),
            "losing a bound proxy must rebuild the sidecar"
        );
        assert_ne!(
            sidecar_config_fingerprint_with_account_proxies(
                config,
                manifest,
                &[account_proxy_fingerprint(None)]
            ),
            sidecar_config_fingerprint_with_account_proxies(
                config,
                manifest,
                &[account_proxy_fingerprint(Some("http://127.0.0.1:1"))]
            ),
            "binding a proxy must rebuild the sidecar"
        );
    }

    #[test]
    fn account_proxy_fingerprint_never_reveals_credentials() {
        let digest = account_proxy_fingerprint(Some("socks5h://user:secret@127.0.0.1:58887"));
        assert_eq!(digest.len(), 40);
        for secret in ["user", "secret", "58887", "127.0.0.1"] {
            assert!(
                !digest.contains(secret),
                "digest must not embed {secret}"
            );
        }
        assert_eq!(account_proxy_fingerprint(Some("   ")), String::new());
        assert_eq!(account_proxy_fingerprint(None), String::new());
    }

    #[test]
    fn sidecar_fingerprint_ignores_remaining_quota() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_a = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 10, "planRank": 2}
          ],
          "routingStrategy": "auto"
        }"#;
        let manifest_b = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 90, "planRank": 2}
          ],
          "routingStrategy": "auto"
        }"#;
        let manifest_c = r#"{
          "accounts": [
            {"id": "account-a", "email": "a@example.com", "remainingQuota": 90, "planRank": 3}
          ],
          "routingStrategy": "auto"
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_b)
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_b),
            sidecar_config_fingerprint(config, manifest_c)
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_token_usage_but_tracks_token_limit() {
        let config = r#"{"host":"127.0.0.1","port":58393}"#;
        let manifest_a = r#"{"apiKeys":[{"id":"key-1","tokenLimit":1000,"tokenUsed":100}]}"#;
        let manifest_b = r#"{"apiKeys":[{"id":"key-1","tokenLimit":1000,"tokenUsed":900}]}"#;
        let manifest_c = r#"{"apiKeys":[{"id":"key-1","tokenLimit":2000,"tokenUsed":900}]}"#;

        assert_eq!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_b),
            "usage updates must not restart active sidecar streams"
        );
        assert_ne!(
            sidecar_config_fingerprint(config, manifest_a),
            sidecar_config_fingerprint(config, manifest_c),
            "limit changes must restart the sidecar to apply the new policy"
        );
    }

    #[test]
    fn sidecar_fingerprint_ignores_hot_reloadable_payload_defaults() {
        let manifest = r#"{"accounts":[],"routingStrategy":"auto"}"#;
        let standard = r#"{"host":"127.0.0.1","port":58393}"#;
        let priority = r#"{
          "host": "127.0.0.1",
          "port": 58393,
          "payload": {"default":[{"models":[{"name":"gpt-*","protocol":"openai/responses"}],"params":{"service_tier":"priority"}}]}
        }"#;
        let other_port = r#"{
          "host": "127.0.0.1",
          "port": 58394,
          "payload": {"default":[{"models":[{"name":"gpt-*","protocol":"openai/responses"}],"params":{"service_tier":"priority"}}]}
        }"#;

        assert_eq!(
            sidecar_config_fingerprint(standard, manifest),
            sidecar_config_fingerprint(priority, manifest),
        );
        assert_ne!(
            sidecar_config_fingerprint(priority, manifest),
            sidecar_config_fingerprint(other_port, manifest),
        );
    }






    #[test]
    fn account_model_rules_block_matching_model_only() {
        let mut collection =
            test_local_access_collection(vec!["account-a".to_string(), "account-b".to_string()]);
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-a".to_string(),
            excluded_models: vec!["gpt-5.4-*".to_string()],
        }];

        assert!(account_model_rule_blocks_model(
            &collection,
            "account-a",
            "gpt-5.4-mini"
        ));
        assert!(!account_model_rule_blocks_model(
            &collection,
            "account-a",
            "gpt-5.3-codex"
        ));
        assert!(!account_model_rule_blocks_model(
            &collection,
            "account-b",
            "gpt-5.4-mini"
        ));
    }

    #[test]
    fn sidecar_account_excluded_models_merge_global_and_account_rules() {
        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        collection.excluded_models = vec!["gpt-5.2".to_string()];
        collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
            account_id: "account-a".to_string(),
            excluded_models: vec!["gpt-5.4-mini".to_string(), "GPT-5.4-MINI".to_string()],
        }];

        assert_eq!(
            merge_collection_and_account_excluded_models(&collection, "account-a"),
            vec!["gpt-5.2".to_string(), "gpt-5.4-mini".to_string()]
        );
    }

    #[test]
    fn gpt_reserve_entitlement_requires_an_allowed_additional_rate_limit() {
        let mut account = test_account_with_plan("pro");
        account.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT-Reserve",
                    "rate_limit": { "allowed": true }
                }],
                "rate_limit": { "allowed": false },
                "rate_limit_upsell": { "banner_type": "luna_reserve" }
            })),
        });
        assert!(account_has_gpt_reserve_entitlement(&account));
        let valid = account.quota.as_ref().unwrap().raw_data.clone().unwrap();
        for (regular_allowed, reserve_allowed, banner) in [
            (true, true, "luna_reserve"),
            (false, false, "luna_reserve"),
            (false, true, "upgrade"),
        ] {
            let mut raw = valid.clone();
            raw["rate_limit"]["allowed"] = json!(regular_allowed);
            raw["additional_rate_limits"][0]["rate_limit"]["allowed"] = json!(reserve_allowed);
            raw["rate_limit_upsell"]["banner_type"] = json!(banner);
            account.quota.as_mut().unwrap().raw_data = Some(raw);
            assert!(!account_has_gpt_reserve_entitlement(&account),
                "must reject regular={regular_allowed}, reserve={reserve_allowed}, banner={banner}");
        }
        account.quota.as_mut().unwrap().raw_data = Some(valid.clone());
        account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
        assert!(!account_has_gpt_reserve_entitlement(&account));
        account.quota.as_mut().unwrap().raw_data = None;
        assert!(!account_has_gpt_reserve_entitlement(&account));
    }

    #[test]
    fn api_service_model_list_keeps_reserve_without_eligible_accounts() {
        let mut entitled = test_account_with_plan("pro");
        entitled.id = "reserve-entitled".to_string();
        entitled.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT reserve",
                    "rate_limit": { "allowed": true }
                }],
                "rate_limit": { "allowed": false },
                "rate_limit_upsell": { "banner_type": "luna_reserve" }
            })),
        });
        let mut ordinary = entitled.clone();
        ordinary.id = "reserve-ordinary".to_string();
        ordinary.quota.as_mut().unwrap().raw_data = Some(json!({
            "additional_rate_limits": []
        }));

        let mut collection = test_local_access_collection(vec![entitled.id.clone()]);
        let api_key = ResolvedLocalApiKey {
            id: "reserve-key".to_string(),
            label: "Reserve".to_string(),
            provider_gateway: None,
            inherit_account_pool: true,
            account_ids: Vec::new(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
        };
        let entitled_models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[entitled.clone(), ordinary.clone()],
            None,
        );
        assert!(entitled_models
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)));

        collection.account_ids = vec![ordinary.id.clone()];
        let ordinary_models = visible_codex_model_ids_for_api_key_with_accounts(
            &collection,
            &api_key,
            &[entitled, ordinary],
            None,
        );
        assert!(ordinary_models
            .iter()
            .any(|model| model.eq_ignore_ascii_case(CODEX_GPT_RESERVE_MODEL_ID)));
    }

    #[test]
    fn scoped_api_key_pool_discovers_spark_entitlement_from_effective_accounts() {
        let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
        shared_state.prepare_disabled();
        let mut plus = test_account_with_plan("plus");
        plus.id = "scoped-plus".to_string();
        plus.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({ "additional_rate_limits": [] })),
        });

        let mut pro = test_account_with_plan("pro");
        pro.id = "scoped-pro".to_string();
        pro.quota = Some(CodexQuota {
            hourly_percentage: 100,
            hourly_reset_time: None,
            hourly_window_minutes: Some(300),
            hourly_window_present: Some(true),
            weekly_percentage: 100,
            weekly_reset_time: None,
            weekly_window_minutes: Some(10_080),
            weekly_window_present: Some(true),
            reset_credits_available: None,
            reset_credits: Vec::new(),
            reset_credits_next_expires_at: None,
            raw_data: Some(json!({
                "additional_rate_limits": [{
                    "limit_name": "GPT-5.3-Codex-Spark",
                    "metered_feature": "codex_spark",
                    "rate_limit": { "allowed": true }
                }]
            })),
        });

        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key.clear();
        let mut api_key = build_local_access_api_key(Some("Scoped Spark"));
        api_key.key = "scoped-spark-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![plus.id.clone(), pro.id.clone()];
        collection.api_keys = vec![api_key];

        let dir = make_temp_dir("codex-scoped-spark-entitlement");
        let overrides = HashMap::from([(plus.id.clone(), plus.clone()), (pro.id.clone(), pro)]);
        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            overrides,
            true,
            None,
        )
        .expect("prepare scoped sidecar config");

        let auth_path = sidecar_auths_dir(&dir).join(sidecar_auth_file_name(&plus.id));
        let auth: Value =
            serde_json::from_str(&fs::read_to_string(auth_path).expect("read scoped Plus auth"))
                .expect("parse scoped Plus auth");
        let excluded = auth
            .get("excluded_models")
            .and_then(Value::as_array)
            .expect("excluded_models should be an array");
        assert!(
            excluded
                .iter()
                .filter_map(Value::as_str)
                .any(|model| model.eq_ignore_ascii_case("gpt-5.3-codex-spark")),
            "Plus account should exclude Spark discovered from its scoped Pro peer"
        );

        fs::remove_dir_all(dir).expect("cleanup scoped sidecar config");
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        for _ in 0..10 {
            let dir = std::env::temp_dir().join(format!(
                "{}-{}-{}",
                prefix,
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            if fs::create_dir(&dir).is_ok() {
                return dir;
            }
        }
        panic!("create temp dir failed");
    }

    struct LocalAccessTestDataGuard {
        data_dir: PathBuf,
        previous_test_data_dir: Option<String>,
        previous_data_dir: Option<String>,
        takeover_backup_path: PathBuf,
        previous_takeover_backup: Option<Vec<u8>>,
    }

    impl LocalAccessTestDataGuard {
        fn new(prefix: &str) -> Self {
            let data_dir = make_temp_dir(prefix);
            let previous_test_data_dir = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR").ok();
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_DATA_DIR").ok();
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &data_dir);

            let takeover_backup_path =
                super::local_access_takeover_backups_path().expect("resolve takeover backup path");
            let previous_takeover_backup = fs::read(&takeover_backup_path).ok();
            if takeover_backup_path.exists() {
                fs::remove_file(&takeover_backup_path).expect("clear takeover backup for test");
            }

            Self {
                data_dir,
                previous_test_data_dir,
                previous_data_dir,
                takeover_backup_path,
                previous_takeover_backup,
            }
        }
    }

    impl Drop for LocalAccessTestDataGuard {
        fn drop(&mut self) {
            match self.previous_test_data_dir.as_deref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
            }
            match self.previous_data_dir.as_deref() {
                Some(value) => std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value),
                None => std::env::remove_var("COCKPIT_TOOLS_DATA_DIR"),
            }
            match self.previous_takeover_backup.as_deref() {
                Some(content) => {
                    if let Some(parent) = self.takeover_backup_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&self.takeover_backup_path, content);
                }
                None => {
                    let _ = fs::remove_file(&self.takeover_backup_path);
                }
            }
            let _ = fs::remove_dir_all(&self.data_dir);
        }
    }

    fn test_account_with_plan(plan_type: &str) -> CodexAccount {
        let mut account = CodexAccount::new(
            format!("acc-{}", plan_type),
            format!("{}@example.com", plan_type),
            CodexTokens {
                id_token: String::new(),
                access_token: "access-token".to_string(),
                refresh_token: None,
            },
        );
        account.plan_type = Some(plan_type.to_string());
        account
    }

    #[test]
    fn append_local_access_accounts_is_incremental_and_reports_skips() {
        let existing = test_account_with_plan("plus");
        let mut added = test_account_with_plan("team");
        added.id = "added".to_string();
        let free = test_account_with_plan("free");
        let mut chat = CodexAccount::new_api_key(
            "chat".to_string(),
            "chat@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example.com/v1".to_string()),
            None,
            None,
            Vec::new(),
        );
        chat.api_wire_api = Some("chat_completions".to_string());

        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[existing.id.clone(), "preserved".to_string()],
            vec![
                existing.id.clone(),
                "added".to_string(),
                "added".to_string(),
                free.id.clone(),
                "chat".to_string(),
                "missing".to_string(),
            ],
            &[existing.clone(), added, free.clone(), chat],
        );

        assert_eq!(
            next_ids,
            vec![
                existing.id.clone(),
                "preserved".to_string(),
                "added".to_string(),
                free.id.clone(),
                "chat".to_string()
            ]
        );
        assert_eq!(
            synced_ids,
            vec![existing.id, "added".to_string(), free.id.clone(), "chat".to_string()]
        );
        assert_eq!(added_ids, vec!["added".to_string(), free.id, "chat".to_string()]);
        assert_eq!(
            skipped
                .iter()
                .map(|item| (item.account_id.as_str(), item.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![("missing", "not_found")]
        );
    }

    #[test]
    fn pending_oauth_accounts_are_not_eligible_for_local_access_pool() {
        let mut pending = test_account_with_plan("plus");
        pending.id = "pending".to_string();
        pending.authorization_status = Some("pending".to_string());
        pending.tokens.access_token = String::new();
        pending.tokens.id_token = String::new();
        pending.tokens.refresh_token = None;

        assert!(!is_local_access_eligible_account(&pending));
        assert_eq!(
            local_access_ineligible_reason(&pending),
            Some("pending_oauth")
        );

        let (next_ids, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![pending.id.clone()],
            &[pending.clone()],
        );
        assert!(next_ids.is_empty());
        assert!(synced_ids.is_empty());
        assert!(added_ids.is_empty());
        assert_eq!(
            skipped
                .iter()
                .map(|item| (item.account_id.as_str(), item.reason.as_str()))
                .collect::<Vec<_>>(),
            vec![("pending", "pending_oauth")]
        );
    }

    #[test]
    fn free_agent_identity_remains_eligible_for_local_access_pool() {
        let mut account = test_account_with_plan("free");
        account.tokens.access_token.clear();
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-free-agent".to_string(),
            agent_private_key: "private-key".to_string(),
            task_id: Some("task-free-agent".to_string()),
            account_id: "account-free-agent".to_string(),
            chatgpt_user_id: "user-free-agent".to_string(),
            email: Some("free-agent@example.com".to_string()),
            plan_type: Some("free".to_string()),
            chatgpt_account_is_fedramp: false,
        });

        assert!(is_local_access_eligible_account(&account));
        let (_, synced_ids, added_ids, skipped) = append_eligible_local_access_account_ids(
            &[],
            vec![account.id.clone()],
            &[account.clone()],
        );
        assert_eq!(synced_ids, vec![account.id.clone()]);
        assert_eq!(added_ids, vec![account.id]);
        assert!(skipped.is_empty());
    }

    #[test]
    fn agent_identity_cannot_be_used_as_local_access_oauth_binding() {
        let mut account = test_account_with_plan("plus");
        account.tokens.refresh_token = Some("refresh-token".to_string());
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-oauth-binding".to_string(),
            agent_private_key: "private-key".to_string(),
            task_id: Some("task-oauth-binding".to_string()),
            account_id: "account-oauth-binding".to_string(),
            chatgpt_user_id: "user-oauth-binding".to_string(),
            email: Some("agent-oauth-binding@example.com".to_string()),
            plan_type: Some("plus".to_string()),
            chatgpt_account_is_fedramp: false,
        });

        let error = validate_loaded_local_access_bound_oauth_account(account)
            .expect_err("Agent Identity must not be accepted as an OAuth binding");

        assert!(error.contains("不能作为 OAuth 绑定账号"));
    }

    fn make_test_jwt(payload: Value) -> String {
        let header = json!({ "alg": "none", "typ": "JWT" });
        format!(
            "{}.{}.sig",
            general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&header).expect("serialize jwt header")),
            general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize jwt payload"))
        )
    }

    #[test]
    fn write_string_atomic_if_changed_skips_identical_content() {
        let dir = make_temp_dir("codex-sidecar-write-if-changed");
        let path = dir.join("auth.json");

        assert!(write_string_atomic_if_changed(&path, "{\"token\":\"a\"}")
            .expect("initial write should succeed"));
        assert!(!write_string_atomic_if_changed(&path, "{\"token\":\"a\"}")
            .expect("unchanged write should succeed"));
        assert!(write_string_atomic_if_changed(&path, "{\"token\":\"b\"}")
            .expect("changed write should succeed"));
        assert_eq!(
            fs::read_to_string(&path).expect("read updated content"),
            "{\"token\":\"b\"}"
        );

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidecar_oauth_auth_json_includes_access_token_expiry() {
        let account = CodexAccount::new(
            "account-exp".to_string(),
            "exp@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-exp",
                    "exp": 4_102_444_800i64,
                })),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        let mut collection = test_local_access_collection(vec![account.id.clone()]);

        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);

        assert_eq!(
            auth_json.get("last_refresh").and_then(Value::as_str),
            account
                .token_updated_at
                .map(|value| value.to_string())
                .as_deref()
        );
        assert_eq!(
            auth_json.get("expired").and_then(Value::as_i64),
            Some(4_102_444_800i64)
        );
        assert_eq!(
            auth_json.get("refresh_token").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            auth_json.get("refresh_owner").and_then(Value::as_str),
            Some("cockpit_token_authority")
        );
        assert_eq!(
            auth_json.get("websockets").and_then(Value::as_bool),
            Some(false)
        );

        collection.responses_websockets_enabled = true;
        let enabled_auth_json = sidecar_auth_json_for_account(&account, &collection, None);
        assert_eq!(
            enabled_auth_json.get("websockets").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn sidecar_projection_ignores_legacy_fingerprint_without_mutating_account() {
        for mode in [None, Some("off"), Some("device"), Some("session"), Some("full")] {
            let mut account = CodexAccount::new(
                "legacy-fingerprint".into(),
                "legacy@example.com".into(),
                CodexTokens {
                    id_token: String::new(),
                    access_token: "access-token".into(),
                    refresh_token: Some("refresh-token".into()),
                },
            );
            account.codex_fingerprint_mode = mode.map(str::to_string);
            account.codex_cli_only = true;
            account.codex_cli_only_allow_app_server = true;
            let collection = test_local_access_collection(vec![account.id.clone()]);
            let projected = sidecar_auth_json_for_account(&account, &collection, None);
            assert!(projected.get("codex_fingerprint_mode").is_none());
            assert!(projected.get("codex_cli_only").is_none());
            assert!(projected.get("codex_cli_only_allow_app_server").is_none());
            assert!(projected.get("codex_cli_only_allow_app_server_clients").is_none());
            assert_eq!(account.codex_fingerprint_mode.as_deref(), mode);
            assert!(account.codex_cli_only);
            assert!(account.codex_cli_only_allow_app_server);
            assert_eq!(account.tokens.refresh_token.as_deref(), Some("refresh-token"));
            assert_eq!(projected["refresh_owner"], "cockpit_token_authority");
        }
    }

    #[test]
    fn provider_gateway_runtime_auth_sync_rewrites_access_token_without_refresh_token() {
        let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
        shared_state.prepare_disabled();
        let sidecar_dir = make_temp_dir("codex-provider-runtime-auth-sync");
        let auths_dir = sidecar_auths_dir(&sidecar_dir);
        fs::create_dir_all(&auths_dir).expect("create auths dir");
        let mut account = test_account_with_plan("plus");
        account.tokens.access_token = "access-before".to_string();
        account.tokens.refresh_token = Some("refresh-must-not-leak".to_string());
        let collection = test_local_access_collection(vec![account.id.clone()]);
        let auth_path = auths_dir.join(sidecar_auth_file_name(&account.id));
        fs::write(&auth_path, "{}").expect("write existing auth file");

        account.tokens.access_token = "access-after".to_string();
        assert!(
            sync_provider_gateway_runtime_auth_file(&account, &collection, &sidecar_dir)
                .expect("sync provider runtime auth")
        );

        let auth: Value = serde_json::from_str(
            &fs::read_to_string(&auth_path).expect("read provider runtime auth"),
        )
        .expect("parse provider runtime auth");
        assert_eq!(
            auth.get("access_token").and_then(Value::as_str),
            Some("access-after")
        );
        assert_eq!(auth.get("refresh_token").and_then(Value::as_str), Some(""));
        assert_eq!(
            auth.get("refresh_owner").and_then(Value::as_str),
            Some("cockpit_token_authority")
        );

        fs::remove_dir_all(sidecar_dir).expect("cleanup provider runtime auth sync");
    }

    #[test]
    fn sidecar_agent_identity_auth_json_preserves_signing_credentials_without_tokens() {
        let mut account = CodexAccount::new(
            "agent-account".to_string(),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: "private-key-test".to_string(),
            task_id: Some("task-test".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("plus".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        account.account_id = Some("team-test".to_string());
        account.plan_type = Some("plus".to_string());
        let collection = test_local_access_collection(vec![account.id.clone()]);

        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);
        assert_eq!(
            auth_json.get("auth_mode").and_then(Value::as_str),
            Some("agentIdentity")
        );
        assert_eq!(
            auth_json.get("agent_runtime_id").and_then(Value::as_str),
            Some("runtime-test")
        );
        assert_eq!(
            auth_json.get("task_id").and_then(Value::as_str),
            Some("task-test")
        );
        assert!(auth_json.get("access_token").is_none());
        assert!(sidecar_local_account_usable_for_start(&account));

        let manifest =
            sidecar_account_manifest_value(&account, Some("agent-account.json"), &collection);
        assert_eq!(
            manifest.get("authKind").and_then(Value::as_str),
            Some("agent_identity")
        );
    }

    #[test]
    fn sidecar_recovered_agent_identity_task_is_adopted_only_for_matching_credentials() {
        let dir = make_temp_dir("codex-sidecar-agent-task-adoption");
        let auth_path = dir.join("agent.json");
        let mut account = CodexAccount::new(
            "agent-task-adoption".to_string(),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: "private-key-test".to_string(),
            task_id: Some("task-old".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some(account.email.clone()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        fs::write(
            &auth_path,
            serde_json::to_vec(&json!({
                "auth_mode": "agentIdentity",
                "agent_runtime_id": "runtime-test",
                "agent_private_key": "private-key-test",
                "task_id": "task-recovered",
            }))
            .expect("serialize auth"),
        )
        .expect("write auth");

        assert!(
            super::adopt_sidecar_agent_identity_task(&mut account, &auth_path)
                .expect("adopt recovered task")
        );
        assert_eq!(
            account
                .agent_identity
                .as_ref()
                .and_then(|identity| identity.task_id.as_deref()),
            Some("task-recovered")
        );

        account
            .agent_identity
            .as_mut()
            .expect("identity")
            .agent_private_key = "different-private-key".to_string();
        account.agent_identity.as_mut().expect("identity").task_id =
            Some("task-current".to_string());
        assert!(
            !super::adopt_sidecar_agent_identity_task(&mut account, &auth_path)
                .expect("reject mismatched credentials")
        );
        assert_eq!(
            account
                .agent_identity
                .as_ref()
                .and_then(|identity| identity.task_id.as_deref()),
            Some("task-current")
        );

        fs::remove_dir_all(dir).expect("cleanup temp dir");
    }

    #[test]
    fn build_runtime_account_enables_websockets_for_local_api_service() {
        let account = build_runtime_account(
            "http://127.0.0.1:1455/v1".to_string(),
            "agt_codex_test".to_string(),
            None,
            true,
        );
        assert!(account.api_supports_websockets);
        assert_eq!(account.api_wire_api.as_deref(), Some("responses"));
        assert_eq!(
            account.api_provider_name.as_deref(),
            Some("Codex API Service")
        );
        assert_eq!(
            account.api_provider_id.as_deref(),
            Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
        );
    }

    #[test]
    fn provider_gateway_api_key_disables_profile_websockets() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_keys.push(CodexLocalAccessApiKey {
            workbuddy_delegate: None,
            max_reasoning_effort: None,
            id: "provider_gateway_deepseek".to_string(),
            label: "Provider Gateway: DeepSeek".to_string(),
            key: "deepseek-local-key".to_string(),
            provider_gateway: Some(CodexLocalAccessProviderGateway {
        auth_scheme: None,
                base_url: "https://api.deepseek.com/v1".to_string(),
                api_key: "sk-deepseek".to_string(),
                upstream_model: "deepseek-v4-pro".to_string(),
                upstream_models: vec!["deepseek-v4-pro".to_string()],
                wire_api: Some("chat_completions".to_string()),
                supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        }),
            model_routing: None,
            inherit_account_pool: Some(false),
            account_pool_id: None,
            account_ids: vec!["deepseek-account".to_string()],
            priority_account_ids: Vec::new(),
            preferred_account_id: None,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
            budget: Default::default(),
            budget_usage: Default::default(),
            enabled: true,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        });
        assert!(!profile_api_key_supports_websockets(&collection, "deepseek-local-key"));
        assert!(!profile_api_key_supports_websockets(&collection, &collection.api_key));
        collection.responses_websockets_enabled = true;
        assert!(!profile_api_key_supports_websockets(&collection, "deepseek-local-key"));
        assert!(profile_api_key_supports_websockets(&collection, &collection.api_key));
    }

    #[test]
    fn mixed_model_routing_manifest_keeps_oauth_default_and_disables_websockets() {
        let mut collection = test_local_access_collection(vec!["oauth-account".to_string()]);
        collection.bound_oauth_account_id = Some("oauth-account".to_string());
        collection.responses_websockets_enabled = true;
        let mut api_key = build_local_access_api_key(Some("Mixed"));
        api_key.key = "mixed-local-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec!["oauth-account".to_string()];
        api_key.provider_gateway = None;
        api_key.model_routing = Some(CodexLocalAccessModelRouting {
            default_route: "oauth".to_string(),
            failure_policy: "strict".to_string(),
            routes: vec![CodexLocalAccessModelRoute {
                id: "route-cpa".to_string(),
                namespace: "cpa".to_string(),
                provider_account_id: "cpa-account".to_string(),
                provider_gateway: CodexLocalAccessProviderGateway {
        auth_scheme: None,
                    base_url: "https://cpa.example.com/v1".to_string(),
                    api_key: "sk-cpa".to_string(),
                    upstream_model: "gpt-5.5".to_string(),
                    upstream_models: vec!["gpt-5.5".to_string(), "grok-4.6".to_string()],
                    wire_api: Some("responses".to_string()),
                    supports_vision: false,
                    model_capabilities: HashMap::new(),
                    vision_routing_model: None,
                },
                native_provider: None,
            }],
        });
        collection.api_keys = vec![api_key];

        let mixed = sidecar_api_key_manifest_values(&collection)
            .into_iter()
            .find(|value| value.get("key").and_then(Value::as_str) == Some("mixed-local-key"))
            .expect("mixed routing key should be emitted");
        assert!(mixed.get("providerGateway").and_then(Value::as_object).is_none());
        assert_eq!(
            mixed
                .pointer("/modelRouting/defaultRoute")
                .and_then(Value::as_str),
            Some("oauth")
        );
        assert_eq!(
            mixed
                .pointer("/modelRouting/failurePolicy")
                .and_then(Value::as_str),
            Some("strict")
        );
        assert_eq!(
            mixed
                .pointer("/modelRouting/routes/0/namespace")
                .and_then(Value::as_str),
            Some("cpa")
        );
        assert_eq!(
            mixed
                .get("responsesWebsockets")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(!profile_api_key_supports_websockets(
            &collection,
            "mixed-local-key"
        ));
    }

    /// 混合路由的可见模型清单由路由配置推导，因此不依赖用户是否开启「模型管理」。
    #[test]
    fn mixed_model_catalog_definitions_include_official_models_without_routes() {
        let routing = crate::models::CodexInstanceModelRouting {
            enabled: true,
            version: 1,
            routes: Vec::new(),
        };
        let definitions = super::mixed_model_catalog_definitions(&routing)
            .expect("build mixed catalog definitions");
        assert!(!definitions.is_empty(), "至少包含官方模型");
        assert!(
            definitions.iter().all(|(model_id, _)| !model_id.contains('/')),
            "没有路由配置时不应出现带命名空间前缀的模型"
        );
        assert!(
            definitions
                .iter()
                .any(|(model_id, display_name)| model_id.starts_with("gpt-")
                    && !display_name.trim().is_empty()),
            "官方模型需要带展示名"
        );
    }

    /// 只加了 DeepSeek + Grok 等第三方账号时，客户端目录里不能出现 GPT-5.6 Reserve。
    #[test]
    fn profile_catalog_only_offers_reserve_when_official_gpt_models_are_present() {
        let third_party_only = serde_json::json!({
            "models": [
                {"slug": "deepseek-flash"},
                {"slug": "deepseek-v4-pro"},
                {"slug": "grok-4.6"},
                {"slug": "codex-auto-review", "visibility": "hide"},
            ]
        });
        assert!(!super::profile_catalog_allows_reserve(&third_party_only));

        let with_official = serde_json::json!({
            "models": [{"slug": "gpt-5.6-sol"}, {"slug": "grok-4.6"}]
        });
        assert!(super::profile_catalog_allows_reserve(&with_official));

        // 用户显式列出 gpt-reserve 时保持原样，不做二次隐藏。
        let explicit_reserve = serde_json::json!({"models": [{"slug": "gpt-reserve"}]});
        assert!(super::profile_catalog_allows_reserve(&explicit_reserve));
    }

    /// 模型管理目录必须等于客户端实际会渲染的模型：无 GPT 能力时去掉官方 GPT，并补入账号池 extras。
    #[tokio::test]
    async fn experimental_catalog_overlays_pool_models_and_hides_gpt_without_capability() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("catalog-overlay-pool");
        let profile_dir = make_temp_dir("catalog-overlay-profile");

        let mut deepseek = CodexAccount::new_api_key(
            "deepseek-overlay".to_string(),
            "deepseek-overlay@example.com".to_string(),
            "sk-deepseek-overlay".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()],
        );
        deepseek.api_wire_api = Some("responses".to_string());
        let mut grok = CodexAccount::new_api_key(
            "grok-overlay".to_string(),
            "grok-overlay@example.com".to_string(),
            String::new(),
            CodexApiProviderMode::Custom,
            None,
            Some("grok".to_string()),
            Some("Grok".to_string()),
            vec![
                "grok-4.6".to_string(),
                "grok-4.5".to_string(),
                "grok-4.3".to_string(),
            ],
        );
        grok.api_wire_api = Some("responses".to_string());
        grok.upstream_grok_account_id = Some("grok-source".to_string());
        grok.openai_api_key = None;
        crate::modules::codex_account::save_account(&deepseek).expect("save deepseek");
        crate::modules::codex_account::save_account(&grok).expect("save grok");

        let mut collection = test_local_access_collection(vec![deepseek.id.clone(), grok.id.clone()]);
        collection.restrict_free_accounts = false;
        super::save_collection_to_disk(&collection).expect("save collection");
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("attach profile");

        let official = vec![
            crate::models::codex::CodexExperimentalModelDefinition {
                model_id: "gpt-5.6-luna".to_string(),
                display_name: "5.6 Luna".to_string(),
                reasoning_efforts: None,
                default_reasoning_effort: None,
                ultra_reasoning_effort: None,
                context_window: Some(516000),
                auto_compact_token_limit: Some(460000),
            },
            crate::models::codex::CodexExperimentalModelDefinition {
                model_id: "gpt-reserve".to_string(),
                display_name: "GPT-5.6 Reserve".to_string(),
                reasoning_efforts: None,
                default_reasoning_effort: None,
                ultra_reasoning_effort: None,
                context_window: None,
                auto_compact_token_limit: None,
            },
        ];
        let overlaid = super::overlay_rendered_pool_models_on_experimental_catalog(
            &profile_dir,
            official,
        );
        let ids = overlaid
            .iter()
            .map(|model| model.model_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "deepseek-flash",
                "deepseek-v4-pro",
                "grok-4.7",
                "grok-4.6",
                "grok-4.5",
                "grok-4.3",
            ],
            "无 GPT 能力时应只保留账号池可见模型: {ids:?}"
        );
        assert_eq!(
            overlaid[0].display_name,
            crate::modules::codex_account::provider_model_display_name("deepseek-flash")
        );

        let mut with_context = overlaid.clone();
        let grok_46 = with_context.iter_mut()
            .find(|model| model.model_id == "grok-4.6")
            .expect("grok-4.6 remains in the upgraded default catalog");
        grok_46.context_window = Some(1_000_000);
        grok_46.auto_compact_token_limit = Some(900_000);
        let preserved = super::overlay_rendered_pool_models_on_experimental_catalog(
            &profile_dir,
            with_context,
        );
        let grok_46 = preserved
            .iter()
            .find(|model| model.model_id == "grok-4.6")
            .expect("grok-4.6");
        assert_eq!(grok_46.context_window, Some(1_000_000));
        assert_eq!(grok_46.auto_compact_token_limit, Some(900_000));

        fs::remove_dir_all(profile_dir).expect("cleanup overlay profile");
    }

    include!("codex_local_access_tests_sidecar_reserve.rs");
