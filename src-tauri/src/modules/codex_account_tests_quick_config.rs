// Codex 账号测试：Quick config, provider validation and index repair behavior。
// 测试与生产实现共享 super 作用域，验证真实持久化和运行态行为。
    #[test]
    fn quick_config_reads_custom_context_window_without_hiding_it() {
        let base_dir = make_temp_dir("codex-quick-config-custom-window-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 200000\nmodel_auto_compact_token_limit = 180000\n",
        )
        .expect("write config");

        let quick_config =
            read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(!quick_config.context_window_1m);
        assert_eq!(quick_config.auto_compact_token_limit, 180000);
        assert_eq!(quick_config.detected_model_context_window, Some(200000));
        assert_eq!(quick_config.detected_auto_compact_token_limit, Some(180000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_enable_1m_context_window() {
        let base_dir = make_temp_dir("codex-quick-config-enable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result =
            write_quick_config_to_config_toml(&base_dir, Some(1_000_000), Some(880000), None, None)
                .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("model_auto_compact_token_limit = 880000"));
        assert_eq!(result.context_window_1m, true);
        assert_eq!(result.auto_compact_token_limit, 880000);
        assert_eq!(
            result.detected_model_context_window,
            Some(CODEX_CONTEXT_WINDOW_1M_VALUE)
        );
        assert_eq!(result.detected_auto_compact_token_limit, Some(880000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_remove_managed_fields() {
        let base_dir = make_temp_dir("codex-quick-config-disable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\nmodel = \"gpt-5\"\n",
        )
        .expect("write config");

        let result = write_quick_config_to_config_toml(&base_dir, None, None, None, None)
            .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_context_window"));
        assert!(!content.contains("model_auto_compact_token_limit"));
        assert!(content.contains("model = \"gpt-5\""));
        assert!(!result.context_window_1m);
        assert_eq!(
            result.auto_compact_token_limit,
            CODEX_AUTO_COMPACT_DEFAULT_LIMIT
        );
        assert_eq!(result.detected_model_context_window, None);
        assert_eq!(result.detected_auto_compact_token_limit, None);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_write_custom_context_window_and_compact_limit() {
        let base_dir = make_temp_dir("codex-quick-config-custom-write-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result =
            write_quick_config_to_config_toml(&base_dir, Some(516_000), Some(460_000), None, None)
                .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 516000"));
        assert!(content.contains("model_auto_compact_token_limit = 460000"));
        assert!(!result.context_window_1m);
        assert_eq!(result.auto_compact_token_limit, 460_000);
        assert_eq!(result.detected_model_context_window, Some(516_000));
        assert_eq!(result.detected_auto_compact_token_limit, Some(460_000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_rejects_non_positive_context_window() {
        let base_dir = make_temp_dir("codex-quick-config-invalid-context-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let err = write_quick_config_to_config_toml(&base_dir, Some(0), Some(100_000), None, None)
            .expect_err("context window should be rejected");
        assert!(err.contains("上下文窗口必须大于 0"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_reports_managed_experimental_catalog_available_without_model_cache() {
        let base_dir = make_temp_dir("codex-experimental-managed-available-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_context_window = 516000\n",
        )
        .expect("write config");

        let result = read_quick_config_from_config_toml(&base_dir).expect("read quick config");

        assert_eq!(result.detected_model_context_window, Some(516_000));
        assert!(!result.experimental_model_catalog_enabled);
        assert!(result.experimental_model_catalog_available);
        assert!(result
            .experimental_model_catalog_unavailable_reason
            .is_none());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_initializes_full_visible_model_catalog() {
        let base_dir = make_temp_dir("codex-experimental-enable-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");

        let result = write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");

        assert!(result.experimental_model_catalog_enabled);
        assert!(result.experimental_model_catalog_available);
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .is_file());
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        let generated: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read generated catalog"),
        )
        .expect("parse generated catalog");
        let models = generated["models"].as_array().expect("models array");
        for expected in [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
        ] {
            assert!(models.iter().any(|model| {
                model.get("slug").and_then(serde_json::Value::as_str) == Some(expected)
            }));
        }
        assert!(!models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.6-sol-wm")
        }));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_migrates_pre_release_catalog_to_shipped_visible_models() {
        let base_dir = make_temp_dir("codex-experimental-v2-migration-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol-wm\"\n")
            .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            r#"{"version":2,"models":[{"model_id":"gpt-5.6-sol-wm","display_name":"GPT-5.6 Sol WM"}]}"#,
        )
        .expect("write v2 model definitions");

        let result = read_quick_config_from_config_toml(&base_dir).expect("read migrated config");
        let model_ids = result
            .experimental_model_catalog_models
            .iter()
            .map(|model| model.model_id.as_str())
            .collect::<Vec<_>>();
        assert!(model_ids.contains(&"gpt-5.6-sol"));
        assert!(model_ids.contains(&"gpt-5.3-codex"));
        assert!(!model_ids.contains(&"gpt-5.6-sol-wm"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_matches_existing_provider_picker_models_and_labels() {
        let base_dir = make_temp_dir("codex-model-catalog-picker-models-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-provider-model-catalog.json\"\nmodel = \"gpt-5.6-sol\"\n",
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[
                {"slug":"gpt-5.6-sol","display_name":"GPT-5.6-Sol","visibility":"list"},
                {"slug":"gpt-5.6-sol-wm","display_name":"GPT-5.6 Sol WM","visibility":"list"},
                {"slug":"gpt-image-2","display_name":"GPT Image 2","visibility":"hide"}
            ]}"#,
        )
        .expect("write existing provider catalog");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            r#"{"models":[{"model_id":"gpt-5.6-sol","display_name":"GPT-5.6-Sol"}]}"#,
        )
        .expect("write legacy model definitions");

        let before_save =
            read_quick_config_from_config_toml(&base_dir).expect("read legacy model definitions");
        assert!(before_save
            .experimental_model_catalog_models
            .iter()
            .any(|model| model.model_id == "gpt-5.3-codex"));
        assert!(!before_save
            .experimental_model_catalog_models
            .iter()
            .any(|model| model.model_id == "gpt-5.6-sol-wm"));

        let result = write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable model catalog");
        assert!(result
            .experimental_model_catalog_models
            .iter()
            .any(|model| model.model_id == "gpt-5.6-sol" && model.display_name == "5.6 Sol"));
        assert!(!result
            .experimental_model_catalog_models
            .iter()
            .any(|model| model.model_id == "gpt-5.6-sol-wm"));
        assert!(!result
            .experimental_model_catalog_models
            .iter()
            .any(|model| model.model_id == "gpt-image-2"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_persists_dynamic_visible_models_without_default() {
        let base_dir = make_temp_dir("codex-experimental-dynamic-models-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        let models = vec![
            CodexExperimentalModelDefinition {
                model_id: "custom-model-a".to_string(),
                display_name: "Custom Model A".to_string(),
                reasoning_efforts: None,
                context_window: None,
                auto_compact_token_limit: None,
            },
            CodexExperimentalModelDefinition {
                model_id: "custom-model-b".to_string(),
                display_name: "Custom Model B".to_string(),
                reasoning_efforts: None,
                context_window: None,
                auto_compact_token_limit: None,
            },
        ];

        let result = write_quick_config_to_config_toml(
            &base_dir,
            None,
            None,
            Some(true),
            Some(models.clone()),
        )
        .expect("enable dynamic experimental catalog");

        assert_eq!(result.experimental_model_catalog_models, models);
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model = \"custom-model-a\""));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read catalog"),
        )
        .expect("parse catalog");
        let catalog_models = catalog["models"].as_array().expect("models array");
        let custom = catalog_models
            .iter()
            .find(|model| model["slug"] == "custom-model-a")
            .expect("custom model");
        assert_eq!(custom["display_name"], "Custom Model A");
        assert!(custom.get("context_window").is_some());
        assert!(catalog_models
            .iter()
            .any(|model| model["slug"] == "custom-model-b"));
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
            .is_file());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_writes_custom_reasoning_efforts_per_model() {
        let base_dir = make_temp_dir("codex-experimental-reasoning-efforts-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "custom-reasoning-model".to_string(),
            display_name: "Custom Reasoning Model".to_string(),
            reasoning_efforts: Some(vec!["low".to_string(), "high".to_string()]),
            context_window: None,
            auto_compact_token_limit: None,
        }];

        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), Some(models))
            .expect("write reasoning configuration");

        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read catalog"),
        )
        .expect("parse catalog");
        let model = catalog["models"]
            .as_array()
            .expect("models array")
            .iter()
            .find(|model| model["slug"] == "custom-reasoning-model")
            .expect("custom model");
        let efforts = model["supported_reasoning_levels"]
            .as_array()
            .expect("reasoning levels")
            .iter()
            .filter_map(|level| level["effort"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(efforts, vec!["low", "high"]);
        assert_eq!(model["default_reasoning_level"], "low");

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_writes_context_settings_per_visible_model() {
        let base_dir = make_temp_dir("codex-visible-model-context-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_context_window = 516000\nmodel_auto_compact_token_limit = 460000\n",
        )
        .expect("write legacy global context config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "gpt-5.6-sol".to_string(),
            display_name: "5.6 Sol".to_string(),
            reasoning_efforts: None,
            context_window: Some(1_000_000),
            auto_compact_token_limit: Some(900_000),
        }];

        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), Some(models))
            .expect("write per-model context configuration");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(!config.contains("model_context_window"));
        assert!(!config.contains("model_auto_compact_token_limit"));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read unified catalog"),
        )
        .expect("parse unified catalog");
        let model = catalog["models"]
            .as_array()
            .and_then(|models| models.iter().find(|model| model["slug"] == "gpt-5.6-sol"))
            .expect("find configured model");
        assert_eq!(model["context_window"], 1_000_000);
        assert_eq!(model["max_context_window"], 1_000_000);
        assert_eq!(model["auto_compact_token_limit"], 900_000);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_persists_selected_default_model() {
        let base_dir = make_temp_dir("codex-experimental-explicit-default-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "custom-model".to_string(),
            display_name: "Custom Model".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        }];

        let result = write_quick_config_to_config_toml_with_default(
            &base_dir,
            None,
            None,
            Some(true),
            Some(models.clone()),
            Some("custom-model".to_string()),
        )
        .expect("persist visible model list");

        assert_eq!(result.experimental_model_catalog_models, models);
        assert_eq!(
            result
                .experimental_model_catalog_default_model_id
                .as_deref(),
            Some("custom-model")
        );
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"custom-model\""));
        let catalog_config: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE))
                .expect("read model config"),
        )
        .expect("parse model config");
        assert_eq!(catalog_config["default_model_id"], "custom-model");

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_enable_experimental_catalog_from_local_access_catalog() {
        let base_dir = make_temp_dir("codex-experimental-local-access-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_provider = \"codex_local_access\"\nmodel_catalog_json = \"cockpit-local-access-model-catalog.json\"\n",
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"gpt-5.6-sol","context_window":1000000,"max_context_window":1000000,"auto_compact_token_limit":null}]}"#,
        )
        .expect("write local access catalog");

        let initial = read_quick_config_from_config_toml(&base_dir).expect("read initial status");
        assert!(!initial.experimental_model_catalog_enabled);
        assert!(initial.experimental_model_catalog_available);
        assert!(initial
            .experimental_model_catalog_unavailable_reason
            .is_none());

        let result = write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");

        assert!(result.experimental_model_catalog_enabled);
        assert!(result.experimental_model_catalog_available);
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(!config.contains("model = "));
        assert!(!base_dir
            .join(super::CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());
        let model = result
            .experimental_model_catalog_models
            .iter()
            .find(|model| model.model_id == "gpt-5.6-sol")
            .expect("migrated Sol model");
        assert_eq!(model.context_window, Some(1_000_000));
        assert_eq!(model.auto_compact_token_limit, Some(900_000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_merges_existing_user_catalog_without_overwriting_it() {
        let base_dir = make_temp_dir("codex-experimental-conflict-test");
        let config_path = base_dir.join("config.toml");
        let existing = "model_catalog_json = \"user-model-catalog.json\"\nmodel = \"gpt-5\"\n";
        fs::write(&config_path, existing).expect("write config");
        let user_catalog =
            r#"{"models":[{"slug":"user-custom-model","display_name":"User Custom"}]}"#;
        fs::write(base_dir.join("user-model-catalog.json"), user_catalog)
            .expect("write user catalog");
        let status = read_quick_config_from_config_toml(&base_dir).expect("read status");
        assert!(status.experimental_model_catalog_available);
        assert!(status
            .experimental_model_catalog_unavailable_reason
            .is_none());
        assert_eq!(
            status.experimental_model_catalog_conflict.as_deref(),
            Some("user-model-catalog.json")
        );
        let result = write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("merge conflicting catalog");
        assert!(result.experimental_model_catalog_enabled);
        let config = fs::read_to_string(&config_path).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(config.contains("model = \"gpt-5\""));
        let managed_catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read managed catalog"),
        )
        .expect("parse managed catalog");
        assert!(managed_catalog["models"]
            .as_array()
            .expect("managed models")
            .iter()
            .any(|model| model["slug"] == "user-custom-model"));
        assert_eq!(
            fs::read_to_string(base_dir.join("user-model-catalog.json"))
                .expect("read original catalog"),
            user_catalog
        );

        write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("disable and restore original catalog");
        let restored_config = fs::read_to_string(&config_path).expect("read restored config");
        assert!(restored_config.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(restored_config.contains("model = \"gpt-5\""));
        assert_eq!(
            fs::read_to_string(base_dir.join("user-model-catalog.json"))
                .expect("read original catalog after disable"),
            user_catalog
        );
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn ordinary_oauth_account_switch_preserves_experimental_model_policy() {
        let base_dir = make_temp_dir("codex-experimental-oauth-switch-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");
        let account = CodexAccount::new(
            "oauth-account".to_string(),
            "oauth@example.com".to_string(),
            CodexTokens {
                id_token: "test-id-token".to_string(),
                access_token: "test-access-token".to_string(),
                refresh_token: Some("test-refresh-token".to_string()),
            },
        );

        super::sync_or_cleanup_managed_model_catalog_for_dir(&base_dir, &account)
            .expect("switch ordinary OAuth account");

        let status = read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(status.experimental_model_catalog_enabled);
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        let default_model = read_experimental_model_definitions(&base_dir)
            .first()
            .expect("initial model")
            .model_id
            .clone();
        assert!(config.contains(&format!("model = \"{}\"", default_model)));
        assert!(base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .is_file());
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .is_file());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_account_switch_preserves_experimental_model_policy() {
        let base_dir = make_temp_dir("codex-experimental-api-key-switch-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");
        let account = CodexAccount::new_api_key(
            "api-key-account".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.example.com/v1".to_string()),
            Some("example_provider".to_string()),
            Some("Example Provider".to_string()),
            Vec::new(),
        );

        super::sync_or_cleanup_managed_model_catalog_for_dir(&base_dir, &account)
            .expect("switch API Key account");

        let status = read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(status.experimental_model_catalog_enabled);
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        let default_model = read_experimental_model_definitions(&base_dir)
            .first()
            .expect("initial model")
            .model_id
            .clone();
        assert!(config.contains(&format!("model = \"{}\"", default_model)));
        assert!(base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .is_file());
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .is_file());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_gateway_final_catalog_write_reapplies_experimental_policy() {
        let base_dir = make_temp_dir("codex-experimental-provider-final-write-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"provider-model"}]}"#,
        )
        .expect("simulate provider gateway catalog write");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-provider-model-catalog.json\"\nmodel = \"provider-model\"\n",
        )
        .expect("simulate provider gateway config write");

        assert!(
            super::reapply_experimental_model_policy_if_enabled(&base_dir)
                .expect("reapply experimental policy")
        );

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"provider-model\""));
        assert!(!config.contains("model = \"gpt-5.6-sol-wm\""));
        let first_model = read_experimental_model_definitions(&base_dir)
            .first()
            .expect("initial model")
            .model_id
            .clone();
        let catalog = fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
            .expect("read catalog");
        assert!(catalog.contains(&first_model));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_disables_only_its_experimental_catalog() {
        let base_dir = make_temp_dir("codex-experimental-disable-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable catalog");

        let result = write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("disable catalog");

        assert!(!result.experimental_model_catalog_enabled);
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_cleanup_recognizes_managed_model_catalog() {
        let mut doc = "model_catalog_json = \"cockpit-provider-model-catalog.json\"\n"
            .parse::<toml_edit::Document>()
            .expect("parse config");

        assert!(super::remove_provider_managed_model_catalog_from_doc(
            &mut doc
        ));
        assert!(doc.get("model_catalog_json").is_none());
    }

    #[test]
    fn quick_config_preserves_provider_catalog_when_switch_is_off() {
        let base_dir = make_temp_dir("codex-provider-catalog-disabled-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-provider-model-catalog.json\"\n",
        )
        .expect("write config");
        let catalog = r#"{"models":[{"slug":"gpt-5.6-sol","visibility":"list"}]}"#;
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            catalog,
        )
        .expect("write provider catalog");

        let status = read_quick_config_from_config_toml(&base_dir).expect("read status");
        assert!(!status.experimental_model_catalog_enabled);
        assert!(status.experimental_model_catalog_available);
        write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("keep switch disabled");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert_eq!(
            fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read provider catalog"),
            catalog
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_cleanup_removes_managed_catalog_reference_and_file() {
        let base_dir = make_temp_dir("codex-experimental-api-key-cleanup-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-provider-model-catalog.json\"\nmodel = \"gpt-5.6-sol\"\n",
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"gpt-5.6-sol"}]}"#,
        )
        .expect("write managed catalog");

        super::cleanup_experimental_model_catalog_for_dir(&base_dir)
            .expect("cleanup experimental catalog");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_cleanup_preserves_selected_model_after_provider_removed_catalog_reference() {
        let base_dir = make_temp_dir("codex-experimental-api-key-late-cleanup-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"gpt-5.6-sol"}]}"#,
        )
        .expect("write managed catalog");

        super::cleanup_experimental_model_catalog_for_dir(&base_dir)
            .expect("cleanup experimental catalog");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn validate_api_key_credentials_rejects_url_api_key() {
        let err = validate_api_key_credentials("http://127.0.0.1:3000/v1", None)
            .expect_err("url should be rejected as api key");
        assert!(err.contains("API Key 不能是 URL"));
    }

    #[test]
    fn validate_api_key_credentials_rejects_invalid_base_url() {
        let err = validate_api_key_credentials("sk-test-key", Some("not-a-url"))
            .expect_err("invalid base url should be rejected");
        assert!(err.contains("Base URL 格式无效"));
    }

    #[test]
    fn validate_api_key_credentials_accepts_valid_values() {
        let (api_key, api_base_url) =
            validate_api_key_credentials("  sk-test-key  ", Some("https://relay.local/v1/"))
                .expect("valid api key + base url should pass");
        assert_eq!(api_key, "sk-test-key");
        assert_eq!(api_base_url.as_deref(), Some("https://relay.local/v1"));
    }

    #[test]
    fn loopback_http_base_url_detection() {
        assert!(is_loopback_http_base_url(Some("http://localhost:53549/v1")));
        assert!(is_loopback_http_base_url(Some("http://127.0.0.1:53549/v1")));
        assert!(is_loopback_http_base_url(Some("http://[::1]:53549/v1")));
        assert!(!is_loopback_http_base_url(Some("https://relay.example/v1")));
        assert!(!is_loopback_http_base_url(None));
    }

    #[test]
    fn sync_api_key_account_skips_local_access_loopback_provider() {
        let base_dir = make_temp_dir("codex-sync-api-key-local-access");
        fs::write(
            base_dir.join("auth.json"),
            r#"{
              "auth_mode": "apikey",
              "OPENAI_API_KEY": "sk-test-key"
            }"#,
        )
        .expect("write auth");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex Local Access"
base_url = "http://localhost:53549/v1"
wire_api = "responses"
"#,
        )
        .expect("write config");

        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        let original_base = account.api_base_url.clone();
        let original_provider_id = account.api_provider_id.clone();

        sync_api_key_account_from_local_state(&mut account, &base_dir);

        assert_eq!(account.api_base_url, original_base);
        assert_eq!(account.api_provider_id, original_provider_id);
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    #[ignore = "manual local Codex repair smoke test"]
    fn local_codex_index_repair_smoke() {
        crate::modules::logger::init_logger();

        let index_path = get_accounts_storage_path();
        let accounts_dir = get_accounts_dir();
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 索引路径: {}",
            index_path.display()
        );
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 详情目录: {}",
            accounts_dir.display()
        );

        let accounts = list_accounts_checked().expect("local Codex repair should succeed");
        let index = load_account_index();
        eprintln!(
            "[LocalCodexRepairTest] 修复/读取完成: accounts={}, current_account_id={}",
            accounts.len(),
            index.current_account_id.as_deref().unwrap_or("-")
        );

        if let Ok(log_file) = crate::modules::logger::get_latest_app_log_file() {
            eprintln!(
                "[LocalCodexRepairTest] 应用日志文件: {}",
                log_file.display()
            );
        }
    }

    #[test]
    fn codex_group_quota_policy_defaults_to_inherit() {
        let groups: Vec<CodexAccountGroupRecord> =
            serde_json::from_str(r#"[{"accountIds":["a1"]}]"#).expect("parse");
        assert_eq!(groups[0].policy(), CodexGroupQuotaRefreshPolicy::Inherit);
    }

    #[test]
    fn codex_group_quota_policy_supports_disabled_and_custom() {
        let groups: Vec<CodexAccountGroupRecord> = serde_json::from_str(
            r#"[
              {"accountIds":["a1"],"quotaAutoRefreshMinutes":-1},
              {"accountIds":["a2"],"quotaAutoRefreshMinutes":5},
              {"accountIds":["a3"],"quotaRefreshEnabled":false}
            ]"#,
        )
        .expect("parse");
        assert_eq!(groups[0].policy(), CodexGroupQuotaRefreshPolicy::Disabled);
        assert_eq!(groups[1].policy(), CodexGroupQuotaRefreshPolicy::Minutes(5));
        assert_eq!(groups[2].policy(), CodexGroupQuotaRefreshPolicy::Disabled);
    }
