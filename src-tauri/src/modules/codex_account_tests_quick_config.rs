// Codex 账号测试：Quick config, provider validation and index repair behavior。
// 测试与生产实现共享 super 作用域，验证真实持久化和运行态行为。
    #[test]
    fn managed_catalog_lists_reserve_without_changing_defaults_and_cleans_up_when_disabled() {
        let base_dir = make_temp_dir("codex-reserve-managed-catalog");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write original model");
        let definitions = super::default_experimental_model_definitions(&base_dir);
        assert!(definitions.iter().any(|model| model.model_id == "gpt-reserve"));
        let saved = super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir, true, definitions.clone(), Some("gpt-5.6-sol".to_string()),
        ).expect("save managed catalog");
        assert!(saved.experimental_model_catalog_models.iter()
            .any(|model| model.model_id == "gpt-reserve"));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap()
        ).unwrap();
        let models = catalog["models"].as_array().unwrap();
        let reserve = models.iter().find(|model| model["slug"] == "gpt-reserve").unwrap();
        let luna = models.iter().find(|model| model["slug"] == "gpt-5.6-luna").unwrap();
        assert_eq!(reserve["visibility"], "list");
        assert_eq!(reserve["display_name"], "GPT-5.6 Reserve");
        assert_eq!(reserve["context_window"], luna["context_window"]);
        assert_eq!(reserve["supported_reasoning_levels"], luna["supported_reasoning_levels"]);
        // 统一口径：Reserve 与 Luna 共享上下文窗口，压缩阈值必须是窗口的 90%。
        assert_eq!(
            reserve["auto_compact_token_limit"],
            luna["context_window"].as_i64().unwrap() * 9 / 10
        );
        let config = fs::read_to_string(base_dir.join("config.toml")).unwrap();
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!config.contains("model_context_window"));
        assert!(!config.contains("model_auto_compact_token_limit"));

        super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir, false, definitions, None,
        ).expect("disable managed catalog");
        let config = fs::read_to_string(base_dir.join("config.toml")).unwrap();
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE).exists());
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_prefixes_builtin_display_names_in_managed_catalog_and_cache() {
        let base_dir = make_temp_dir("codex-display-name-prefix-migration-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-model-catalog.json\"\nmodel = \"gpt-6-astra\"\n",
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE),
            "enabled\n",
        )
        .expect("enable managed catalog");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"gpt-6-astra","display_name":"6 Astra","description":"6 Astra","visibility":"list"},{"slug":"gpt-5.6-sol","display_name":"5.6 Sol","description":"5.6 Sol","visibility":"list"},{"slug":"custom-model","display_name":"6 Astra","description":"6 Astra","visibility":"list"}]}"#,
        )
        .expect("write legacy managed catalog");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            r#"{"version":4,"models":[{"model_id":"gpt-6-astra","display_name":"6 Astra"},{"model_id":"gpt-5.6-sol","display_name":"5.6 Sol"}],"migrations":["add-gpt-6-astra-model"]}"#,
        )
        .expect("write legacy model cache");

        let quick_config =
            read_quick_config_from_config_toml(&base_dir).expect("read migrated quick config");
        for (model_id, display_name) in [
            ("gpt-6-astra", "GPT-6 Astra"),
            ("gpt-5.6-sol", "GPT-5.6 Sol"),
        ] {
            assert_eq!(
                quick_config
                    .experimental_model_catalog_models
                    .iter()
                    .find(|model| model.model_id == model_id)
                    .map(|model| model.display_name.as_str()),
                Some(display_name)
            );
        }

        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read migrated managed catalog"),
        )
        .expect("parse migrated managed catalog");
        for (slug, display_name) in [
            ("gpt-6-astra", "GPT-6 Astra"),
            ("gpt-5.6-sol", "GPT-5.6 Sol"),
        ] {
            let model = catalog["models"]
                .as_array()
                .expect("models")
                .iter()
                .find(|model| model["slug"] == slug)
                .unwrap_or_else(|| panic!("{slug} model"));
            assert_eq!(model["display_name"], display_name);
            assert_eq!(model["description"], display_name);
        }
        // 非内建模型即使叫同样的短名也不改名。
        let custom = catalog["models"]
            .as_array()
            .expect("models")
            .iter()
            .find(|model| model["slug"] == "custom-model")
            .expect("custom model");
        assert_eq!(custom["display_name"], "6 Astra");

        let cache: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE))
                .expect("read migrated model cache"),
        )
        .expect("parse migrated model cache");
        assert_eq!(cache["models"][0]["display_name"], "GPT-6 Astra");
        assert_eq!(cache["models"][1]["display_name"], "GPT-5.6 Sol");
        assert!(cache["migrations"]
            .as_array()
            .expect("migrations")
            .iter()
            .any(|migration| migration == super::BUILTIN_MODEL_DISPLAY_NAME_MIGRATION_ID));

        read_quick_config_from_config_toml(&base_dir).expect("read migrated config again");
        let cache_after = fs::read_to_string(base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE))
            .expect("read stable model cache");
        assert_eq!(cache_after, serde_json::to_string_pretty(&cache).unwrap() + "\n");

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn context_management_defaults_to_off_without_creating_config() {
        let base_dir = make_temp_dir("codex-context-management-default-test");

        let config = super::read_quick_config_from_config_toml(&base_dir)
            .expect("read missing config");
        assert!(!config.context_management_experimental_mode);
        super::save_context_management_for_base_dir(&base_dir, false)
            .expect("keep the official default disabled");
        assert!(!base_dir.join("config.toml").exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn context_management_only_changes_its_own_official_feature_key() {
        let base_dir = make_temp_dir("codex-context-management-write-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model = \"gpt-5.6-sol\"\nmodel_context_window = 516000\n\n[features]\nmemories = true\n\n[other]\nvalue = \"keep\"\n",
        )
        .expect("write config");

        let enabled = super::save_context_management_for_base_dir(&base_dir, true)
            .expect("enable context management");
        assert!(enabled.context_management_experimental_mode);
        let content = fs::read_to_string(&config_path).expect("read enabled config");
        assert!(content.contains("model = \"gpt-5.6-sol\""));
        assert!(content.contains("model_context_window = 516000"));
        assert!(content.contains("memories = true"));
        assert!(content.contains("value = \"keep\""));
        assert!(content.contains("experimental_mode = true"));

        let disabled = super::save_context_management_for_base_dir(&base_dir, false)
            .expect("disable context management");
        assert!(!disabled.context_management_experimental_mode);
        let content = fs::read_to_string(&config_path).expect("read disabled config");
        assert!(!content.contains("experimental_mode"));
        assert!(content.contains("memories = true"));
        assert!(content.contains("value = \"keep\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn context_management_preserves_inline_features_when_toggled() {
        let base_dir = make_temp_dir("codex-context-management-inline-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\nfeatures = { memories = true }\n")
            .expect("write inline config");

        super::save_context_management_for_base_dir(&base_dir, true)
            .expect("enable inline context management");
        let content = fs::read_to_string(&config_path).expect("read inline config");
        let document = content.parse::<toml_edit::Document>().expect("parse inline config");
        assert_eq!(document["features"]["memories"].as_bool(), Some(true));
        assert_eq!(
            document["features"]["context_management"]["experimental_mode"].as_bool(),
            Some(true)
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn context_management_rejects_invalid_feature_shape_without_overwriting_it() {
        let base_dir = make_temp_dir("codex-context-management-invalid-shape-test");
        let config_path = base_dir.join("config.toml");
        let original = "model = \"gpt-5\"\nfeatures = false\n";
        fs::write(&config_path, original).expect("write invalid config");

        let error = super::save_context_management_for_base_dir(&base_dir, true)
            .expect_err("invalid features should be rejected");
        assert!(error.contains("features 不是合法表结构"));
        assert_eq!(fs::read_to_string(&config_path).expect("read invalid config"), original);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

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
    fn quick_config_catalog_only_context_preservation_keeps_latest_values() {
        let base_dir = make_temp_dir("codex-catalog-context-preservation-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 750000\nmodel_auto_compact_token_limit = 640000\nmodel = \"gpt-5\"\n",
        )
        .expect("write config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        }];

        let result = super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            true,
            models,
            None,
        )
        .expect("save model catalog");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 750000"));
        assert!(content.contains("model_auto_compact_token_limit = 640000"));
        assert_eq!(result.detected_model_context_window, Some(750_000));
        assert_eq!(result.detected_auto_compact_token_limit, Some(640_000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_write_custom_context_window_and_compact_limit() {
        let base_dir = make_temp_dir("codex-quick-config-custom-write-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result =
            write_quick_config_to_config_toml(&base_dir, Some(516_000), Some(464_400), None, None)
                .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 516000"));
        assert!(content.contains("model_auto_compact_token_limit = 464400"));
        assert!(!result.context_window_1m);
        assert_eq!(result.auto_compact_token_limit, 464_400);
        assert_eq!(result.detected_model_context_window, Some(516_000));
        assert_eq!(result.detected_auto_compact_token_limit, Some(464_400));

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
        assert_eq!(
            result.experimental_model_catalog_reset_default_model_id.as_deref(),
            Some("gpt-5.6-sol")
        );
        let expected_shipped_models = super::SHIPPED_VISIBLE_CODEX_MODEL_IDS
            .iter()
            .filter(|model_id| !crate::modules::codex_wakeup::is_codex_model_before_5_5(model_id))
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(
            result
                .experimental_model_catalog_reset_models
                .iter()
                .map(|model| model.model_id.as_str())
                .collect::<Vec<_>>(),
            expected_shipped_models
        );
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
        assert_eq!(
            models[0].get("slug").and_then(serde_json::Value::as_str),
            Some("gpt-6-astra")
        );
        for expected in [
            "gpt-6-astra",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
        ] {
            assert!(models.iter().any(|model| {
                model.get("slug").and_then(serde_json::Value::as_str) == Some(expected)
            }));
        }
        for legacy in [
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex",
            "gpt-5.3-codex-spark",
        ] {
            assert!(!models.iter().any(|model| {
                model.get("slug").and_then(serde_json::Value::as_str) == Some(legacy)
            }));
        }
        assert!(!models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.6-sol-wm")
        }));
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-6-astra")
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
        assert!(!model_ids.contains(&"gpt-5.3-codex"));
        assert!(!model_ids.contains(&"gpt-5.6-sol-wm"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn account_switch_adds_astra_to_previous_shipped_catalog_without_overwriting_curated_lists() {
        let base_dir = make_temp_dir("codex-astra-visible-model-migration-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE),
            "enabled\n",
        )
        .expect("enable experimental catalog");

        let mut previous_defaults = super::default_experimental_model_definitions(&base_dir);
        previous_defaults.retain(|model| model.model_id != "gpt-6-astra");
        let previous_config = serde_json::json!({
            "version": super::EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
            "models": previous_defaults,
        });
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            serde_json::to_vec_pretty(&previous_config).expect("serialize previous catalog"),
        )
        .expect("write previous catalog");

        let account = CodexAccount::new(
            "oauth-astra-migration".to_string(),
            "astra@example.com".to_string(),
            CodexTokens {
                id_token: "test-id-token".to_string(),
                access_token: "test-access-token".to_string(),
                refresh_token: Some("test-refresh-token".to_string()),
            },
        );
        super::sync_or_cleanup_managed_model_catalog_for_dir(&base_dir, &account)
            .expect("switch account with previous catalog");
        let generated: serde_json::Value = serde_json::from_slice(
            &fs::read(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read switched catalog"),
        )
        .expect("parse switched catalog");
        assert!(generated["models"]
            .as_array()
            .expect("models")
            .first()
            .is_some_and(|model| model["slug"] == "gpt-6-astra"));
        assert!(generated["models"]
            .as_array()
            .expect("models")
            .iter()
            .any(|model| model["slug"] == "gpt-6-astra"));

        let mut curated = super::default_experimental_model_definitions(&base_dir);
        curated.retain(|model| model.model_id != "gpt-6-astra");
        super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            true,
            curated,
            None,
        )
        .expect("persist curated catalog");
        assert!(!read_experimental_model_definitions(&base_dir)
            .iter()
            .any(|model| model.model_id == "gpt-6-astra"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn account_switch_adds_gpt_6_sol_luna_only_to_unmodified_shipped_catalog() {
        let base_dir = make_temp_dir("codex-gpt-6-sol-luna-visible-model-migration-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE),
            "enabled\n",
        )
        .expect("enable experimental catalog");

        // 上一版随包发布的自动清单：只有 astra，没有 gpt-6-sol / gpt-6-luna。
        let mut previous_defaults = super::default_experimental_model_definitions(&base_dir);
        previous_defaults.retain(|model| {
            model.model_id != "gpt-6-sol" && model.model_id != "gpt-6-luna"
        });
        let previous_config = serde_json::json!({
            "version": super::EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
            "models": previous_defaults,
            "migrations": [super::GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID],
        });
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            serde_json::to_vec_pretty(&previous_config).expect("serialize previous catalog"),
        )
        .expect("write previous catalog");

        let account = CodexAccount::new(
            "oauth-sol-luna-migration".to_string(),
            "sol-luna@example.com".to_string(),
            CodexTokens {
                id_token: "test-id-token".to_string(),
                access_token: "test-access-token".to_string(),
                refresh_token: Some("test-refresh-token".to_string()),
            },
        );
        super::sync_or_cleanup_managed_model_catalog_for_dir(&base_dir, &account)
            .expect("switch account with previous catalog");
        let generated: serde_json::Value = serde_json::from_slice(
            &fs::read(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read switched catalog"),
        )
        .expect("parse switched catalog");
        let slugs = generated["models"]
            .as_array()
            .expect("models")
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            slugs.iter().take(3).copied().collect::<Vec<_>>(),
            vec!["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"]
        );
        let luna_index = slugs.iter().position(|slug| *slug == "gpt-6-luna");
        let legacy_sol_index = slugs.iter().position(|slug| *slug == "gpt-5.6-sol");
        assert!(luna_index
            .zip(legacy_sol_index)
            .is_some_and(|(luna, legacy_sol)| luna < legacy_sol));

        // 用户手工删掉这两个模型的精修清单不再被自动补回。
        let mut curated = super::default_experimental_model_definitions(&base_dir);
        curated.retain(|model| model.model_id != "gpt-6-sol" && model.model_id != "gpt-6-luna");
        super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            true,
            curated,
            None,
        )
        .expect("persist curated catalog");
        let curated_ids = read_experimental_model_definitions(&base_dir)
            .into_iter()
            .map(|model| model.model_id)
            .collect::<Vec<_>>();
        assert!(!curated_ids.iter().any(|model| model == "gpt-6-sol"));
        assert!(!curated_ids.iter().any(|model| model == "gpt-6-luna"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_repairs_astra_default_and_preserves_order_once() {
        let base_dir = make_temp_dir("codex-astra-default-repair-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"cockpit-model-catalog.json\"\nmodel = \"gpt-6-astra\"\n",
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE),
            "enabled\n",
        )
        .expect("enable experimental catalog");
        let models = vec![
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-6-astra",
        ]
        .into_iter()
        .map(|model_id| CodexExperimentalModelDefinition {
            model_id: model_id.to_string(),
            display_name: model_id.to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        })
        .collect::<Vec<_>>();
        let saved = serde_json::json!({
            "version": super::EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
            "models": models,
            "default_model_id": "gpt-6-astra",
            "migrations": [super::GPT_6_ASTRA_MODEL_CATALOG_MIGRATION_ID]
        });
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            serde_json::to_vec_pretty(&saved).expect("serialize saved catalog"),
        )
        .expect("write saved catalog");

        super::enforce_experimental_model_policy_for_dir(&base_dir)
            .expect("repair experimental catalog");

        let quick_config = read_quick_config_from_config_toml(&base_dir).expect("read repaired config");
        assert_eq!(
            quick_config.experimental_model_catalog_default_model_id.as_deref(),
            Some("gpt-5.6-sol")
        );
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        let catalog_config: serde_json::Value = serde_json::from_slice(
            &fs::read(base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE))
                .expect("read repaired catalog"),
        )
        .expect("parse repaired catalog");
        assert_eq!(catalog_config["models"][0]["model_id"], "gpt-6-astra");
        assert_eq!(catalog_config["default_model_id"], "gpt-5.6-sol");
        assert!(catalog_config["migrations"]
            .as_array()
            .expect("migrations")
            .iter()
            .any(|migration| migration == super::GPT_6_ASTRA_DEFAULT_REPAIR_MIGRATION_ID));

        let repaired_models = quick_config.experimental_model_catalog_models.clone();
        let selected_astra = write_quick_config_to_config_toml_with_default(
            &base_dir,
            None,
            None,
            Some(true),
            Some(repaired_models),
            Some("gpt-6-astra".to_string()),
        )
        .expect("persist explicit Astra selection");
        assert_eq!(
            selected_astra.experimental_model_catalog_default_model_id.as_deref(),
            Some("gpt-6-astra")
        );

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
            .any(|model| model.model_id == "gpt-5.6-sol"));
        assert!(!before_save
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
            .any(|model| {
                model.model_id == "gpt-5.6-sol" && model.display_name == "GPT-5.6 Sol"
            }));
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

        let mut expected = models.clone();
        expected.push(CodexExperimentalModelDefinition {
            model_id: "gpt-reserve".to_string(),
            display_name: "GPT-5.6 Reserve".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        });
        assert_eq!(result.experimental_model_catalog_models, expected);
        assert_eq!(
            result
                .experimental_model_catalog_reset_models
                .first()
                .map(|model| model.model_id.as_str()),
            Some("gpt-6-astra")
        );
        assert!(!result
            .experimental_model_catalog_reset_models
            .iter()
            .any(|model| model.model_id.starts_with("custom-model")));
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
    fn quick_config_without_model_overrides_keeps_official_catalog_context() {
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
            context_window: None,
            auto_compact_token_limit: None,
        }];

        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), Some(models))
            .expect("write model configuration without per-model context overrides");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(!config.contains("model_context_window"));
        assert!(!config.contains("model_auto_compact_token_limit"));
        let saved_models = fs::read_to_string(base_dir.join(
            super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE,
        ))
        .expect("read saved model definitions");
        assert!(!saved_models.contains("context_window"));
        assert!(!saved_models.contains("auto_compact_token_limit"));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read unified catalog"),
        )
        .expect("parse unified catalog");
        let model = catalog["models"]
            .as_array()
            .and_then(|models| models.iter().find(|model| model["slug"] == "gpt-5.6-sol"))
            .expect("find configured model");
        let official = crate::modules::codex_protocol::build_codex_client_models_response(&[
            "gpt-5.6-sol".to_string(),
        ]);
        let official = &official["models"][0];
        assert_eq!(model["context_window"], official["context_window"]);
        assert_eq!(model["max_context_window"], official["max_context_window"]);
        assert_eq!(
            model["auto_compact_token_limit"],
            official["auto_compact_token_limit"]
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn reading_model_management_preserves_saved_context_overrides_without_writing() {
        let base_dir = make_temp_dir("codex-model-context-migration-test");
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            format!(
                r#"{{
  "version": {},
  "models": [{{
    "model_id": "custom-model",
    "display_name": "Custom Model",
    "context_window": 1000000,
    "max_context_window": 1000000,
    "auto_compact_token_limit": 900000
  }}]
}}
"#,
                super::EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION
            ),
        )
        .expect("write legacy model configuration");

        let original = fs::read_to_string(base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)).unwrap();
        let models = super::read_experimental_model_definitions(&base_dir);
        let model = models
            .iter()
            .find(|model| model.model_id == "custom-model")
            .expect("find migrated model");
        assert_eq!(model.display_name, "Custom Model");
        assert_eq!(model.context_window, Some(1_000_000));
        assert_eq!(model.auto_compact_token_limit, Some(900_000));

        let migrated = fs::read_to_string(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
        )
        .expect("read migrated model configuration");
        assert_eq!(migrated, original, "reading must not rewrite the model configuration");
        assert!(migrated.contains("\"context_window\": 1000000"));
        assert!(migrated.contains("\"auto_compact_token_limit\": 900000"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn model_context_overrides_round_trip_and_decorate_only_enabled_profiles() {
        let base_dir = make_temp_dir("codex-model-context-round-trip");
        let original = "model_context_window = 516000\nmodel_auto_compact_token_limit = 460000\n";
        fs::write(base_dir.join("config.toml"), original).unwrap();
        let definition = CodexExperimentalModelDefinition {
            model_id: "gpt-5.6-sol".into(),
            display_name: "Sol".into(),
            reasoning_efforts: None,
            context_window: Some(800_000),
            auto_compact_token_limit: Some(700_000),
        };
        let saved = super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir, true, vec![definition.clone()], Some(definition.model_id.clone()),
        ).unwrap();
        assert!(saved.experimental_model_catalog_models.contains(&definition));
        let config = fs::read_to_string(base_dir.join("config.toml")).unwrap();
        assert!(config.contains("model_context_window = 516000"));
        assert!(config.contains("model_auto_compact_token_limit = 460000"));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap(),
        ).unwrap();
        let model = catalog["models"].as_array().unwrap().iter()
            .find(|model| model["slug"] == definition.model_id).unwrap();
        assert_eq!(model["context_window"], 800_000);
        assert_eq!(model["max_context_window"], 800_000);
        assert_eq!(model["auto_compact_token_limit"], 700_000);

        let temporary = r#"{"models":[{"slug":"GPT-5.6-SOL","context_window":516000},{"slug":"other","context_window":100000}]}"#;
        let decorated: serde_json::Value = serde_json::from_str(
            &super::decorate_managed_model_catalog_for_profile(&base_dir, temporary).unwrap(),
        ).unwrap();
        assert_eq!(decorated["models"][0]["context_window"], 800_000);
        assert_eq!(decorated["models"][0]["auto_compact_token_limit"], 700_000);
        assert_eq!(decorated["models"][1]["context_window"], 100_000);
        super::persist_experimental_model_policy(&base_dir, false).unwrap();
        assert_eq!(super::decorate_managed_model_catalog_for_profile(&base_dir, temporary).unwrap(), temporary);
        fs::remove_dir_all(&base_dir).unwrap();
    }

    #[test]
    fn model_context_overrides_require_positive_pairs_and_strict_compact_limit() {
        for (window, compact, error) in [
            (Some(0), Some(1), "EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID"),
            (None, Some(1), "EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID"),
            (Some(100), Some(0), "EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID"),
            (Some(100), Some(-1), "EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID"),
            (Some(100), Some(100), "EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID"),
            (Some(100), Some(101), "EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID"),
        ] {
            let definition = CodexExperimentalModelDefinition {
                model_id: "custom-model".into(), display_name: "Custom".into(),
                reasoning_efforts: None, context_window: window, auto_compact_token_limit: compact,
            };
            assert_eq!(super::normalize_experimental_model_definitions(vec![definition]).unwrap_err(), error);
        }

        // 统一口径：只给上下文窗口时按 90% 派生压缩阈值，不再视为非法配置。
        let derived = super::normalize_experimental_model_definitions(vec![
            CodexExperimentalModelDefinition {
                model_id: "custom-model".into(), display_name: "Custom".into(),
                reasoning_efforts: None, context_window: Some(516_000),
                auto_compact_token_limit: None,
            },
        ])
        .expect("context-only definition should derive the compact limit");
        assert_eq!(derived[0].context_window, Some(516_000));
        assert_eq!(derived[0].auto_compact_token_limit, Some(464_400));
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

        let mut expected = models.clone();
        expected.push(CodexExperimentalModelDefinition {
            model_id: "gpt-reserve".to_string(),
            display_name: "GPT-5.6 Reserve".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        });
        assert_eq!(result.experimental_model_catalog_models, expected);
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
    fn quick_config_restores_model_selected_before_experimental_default() {
        let base_dir = make_temp_dir("codex-experimental-restore-selected-model-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-original\"\n")
            .expect("write config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "custom-model".to_string(),
            display_name: "Custom Model".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        }];

        write_quick_config_to_config_toml_with_default(
            &base_dir,
            None,
            None,
            Some(true),
            Some(models),
            Some("custom-model".to_string()),
        )
        .expect("enable experimental catalog");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("disable experimental catalog");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-original\""));
        assert!(!config.contains("model = \"custom-model\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_removes_experimental_default_when_model_was_unset() {
        let base_dir = make_temp_dir("codex-experimental-restore-unset-model-test");
        fs::write(base_dir.join("config.toml"), "approval_policy = \"on-request\"\n")
            .expect("write config");
        let models = vec![CodexExperimentalModelDefinition {
            model_id: "custom-model".to_string(),
            display_name: "Custom Model".to_string(),
            reasoning_efforts: None,
            context_window: None,
            auto_compact_token_limit: None,
        }];

        write_quick_config_to_config_toml_with_default(
            &base_dir,
            None,
            None,
            Some(true),
            Some(models),
            Some("custom-model".to_string()),
        )
        .expect("enable experimental catalog");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("disable experimental catalog");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("approval_policy = \"on-request\""));
        assert!(!config.contains("model = "));

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
        assert_eq!(model.display_name, "GPT-5.6 Sol");
        // 统一口径：旧目录声明的上下文窗口照常迁移，压缩阈值缺失时按 90% 派生。
        assert_eq!(model.context_window, Some(1_000_000));
        assert_eq!(model.auto_compact_token_limit, Some(900_000));
        let saved_models = fs::read_to_string(base_dir.join(
            super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE,
        ))
        .expect("read migrated model definitions");
        let saved_models: serde_json::Value =
            serde_json::from_str(&saved_models).expect("parse migrated model definitions");
        let saved_sol = saved_models["models"]
            .as_array()
            .expect("models array")
            .iter()
            .find(|model| model["model_id"] == "gpt-5.6-sol")
            .expect("saved Sol model");
        assert_eq!(saved_sol["context_window"], 1_000_000);
        assert_eq!(saved_sol["auto_compact_token_limit"], 900_000);

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
            .expect("disable and restore official catalog");
        let restored_config = fs::read_to_string(&config_path).expect("read restored config");
        assert!(!restored_config.contains("model_catalog_json"));
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
    fn ordinary_oauth_account_switch_preserves_model_policy_and_global_context() {
        let base_dir = make_temp_dir("codex-experimental-oauth-switch-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(
            &base_dir,
            Some(1_000_000),
            Some(900_000),
            Some(true),
            None,
        )
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
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("model_context_window = 1000000"));
        assert!(config.contains("model_auto_compact_token_limit = 900000"));
        assert!(base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .is_file());
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .is_file());
        let catalog = fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
            .expect("read OAuth switched catalog");
        assert!(catalog.contains("\"gpt-6-astra\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn account_switch_does_not_recreate_disabled_experimental_catalog() {
        let base_dir = make_temp_dir("codex-experimental-disabled-switch-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(false), None)
            .expect("disable experimental catalog");
        // Simulate a stale file left by an older build; account switching must clean it too.
        fs::write(
            base_dir.join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE),
            r#"{"version":4,"models":[{"model_id":"gpt-6-astra","display_name":"6 Astra"}]}"#,
        )
        .expect("write stale catalog state");

        let account = CodexAccount::new(
            "oauth-disabled-catalog".to_string(),
            "disabled@example.com".to_string(),
            CodexTokens {
                id_token: "test-id-token".to_string(),
                access_token: "test-access-token".to_string(),
                refresh_token: Some("test-refresh-token".to_string()),
            },
        );
        super::sync_or_cleanup_managed_model_catalog_for_dir(&base_dir, &account)
            .expect("switch with disabled catalog");

        let status = read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(!status.experimental_model_catalog_enabled);
        assert!(status.experimental_model_catalog_default_model_id.is_none());
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_account_switch_preserves_model_policy_and_global_context() {
        let base_dir = make_temp_dir("codex-experimental-api-key-switch-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n").expect("write config");
        write_quick_config_to_config_toml(
            &base_dir,
            Some(516_000),
            Some(460_000),
            Some(true),
            None,
        )
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
        // API Key / 第三方账号不参与模型管理：开关状态保持用户原值。
        assert!(status.experimental_model_catalog_enabled);
        assert!(base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .is_file());
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        // 该账号没有自己的模型目录，因此不能留下模型管理的受管目录引用或文件。
        assert!(!config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("model_context_window = 516000"));
        assert!(config.contains("model_auto_compact_token_limit = 460000"));

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
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_PREVIOUS_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn disabling_catalog_ignores_stale_invalid_editor_draft_and_cleans_control_state() {
        let base_dir = make_temp_dir("codex-experimental-disable-stale-draft-test");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable catalog");

        let invalid_draft = vec![CodexExperimentalModelDefinition {
            model_id: "bad model id".to_string(),
            display_name: String::new(),
            reasoning_efforts: Some(vec!["not-a-real-effort".to_string()]),
            context_window: None,
            auto_compact_token_limit: None,
        }];
        write_quick_config_to_config_toml_with_default(
            &base_dir,
            None,
            None,
            Some(false),
            Some(invalid_draft),
            Some("bad model id".to_string()),
        )
        .expect("disable should ignore stale draft");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_PREVIOUS_CATALOG_FILE)
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
    fn quick_config_removes_provider_catalog_reference_when_switch_is_off() {
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
        assert!(!config.contains("model_catalog_json"));
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

    #[test]
    fn auto_restore_on_launch_reapplies_catalog_and_preserves_1m_context_window() {
        let base_dir = make_temp_dir("codex-auto-restore-launch-test");
        let initial_config = "model = \"gpt-5.6-sol\"\nmodel_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\n";
        fs::write(base_dir.join("config.toml"), initial_config).expect("write initial config");
        write_quick_config_to_config_toml(&base_dir, None, None, Some(true), None)
            .expect("enable experimental catalog");

        // 模拟退出接管后
        fs::write(
            base_dir.join("config.toml"),
            "model = \"gpt-5.6-sol\"\nmodel_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\n",
        )
        .expect("write unattached config");

        // 模拟启动自动恢复
        assert!(
            super::reapply_experimental_model_policy_if_enabled(&base_dir)
                .expect("reapply experimental policy")
        );

        let restored_config = fs::read_to_string(base_dir.join("config.toml")).expect("read restored config");
        assert!(restored_config.contains("model_context_window = 1000000"));
        assert!(restored_config.contains("model_auto_compact_token_limit = 900000"));
        assert!(restored_config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    /// 一次性迁移：把历史遗留的「模型管理」关闭并恢复跟随官方模型目录，
    /// 只执行一次，且保留用户已保存的模型清单（重新开启后仍可用）。
    #[test]
    fn model_management_default_off_migration_runs_once_and_keeps_definitions() {
        let base_dir = make_temp_dir("codex-model-management-default-off-migration");
        fs::write(base_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write base config");
        let definitions = super::default_experimental_model_definitions(&base_dir);
        super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            true,
            definitions.clone(),
            Some("gpt-5.6-sol".to_string()),
        )
        .expect("enable managed catalog");
        assert!(super::experimental_model_policy_enabled(&base_dir));

        assert!(
            super::migrate_model_management_default_off_once(&base_dir).expect("run migration"),
            "首次执行必须生效"
        );
        assert!(!super::experimental_model_policy_enabled(&base_dir));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert_eq!(
            super::read_experimental_model_definitions(&base_dir).len(),
            definitions.len(),
            "用户模型清单必须保留"
        );

        // 迁移只执行一次：之后用户自己再开启模型管理不再被关闭。
        super::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            true,
            definitions,
            None,
        )
        .expect("re-enable managed catalog");
        assert!(
            !super::migrate_model_management_default_off_once(&base_dir).expect("run migration"),
            "已迁移过的 profile 不能再次执行"
        );
        assert!(super::experimental_model_policy_enabled(&base_dir));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }
