// Codex 账号测试：DeepSeek and model catalog behavior。
// 测试与生产实现共享 super 作用域，验证真实持久化和运行态行为。
    #[test]
    fn deepseek_account_normalize_defaults_to_official_responses_profile() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-pro".to_string()],
        );
        account.api_wire_api = None;
        account.api_supports_websockets = true;
        account.api_supports_vision = true;

        assert!(super::normalize_deepseek_account(&mut account));
        assert_eq!(
            account.api_base_url.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(account.api_wire_api.as_deref(), Some("responses"));
        assert!(account.api_sync_model_catalog_to_codex);
        assert!(!account.api_supports_websockets);
        assert!(!account.api_supports_vision);
        assert_eq!(
            account.api_model_catalog,
            vec!["deepseek-v4-flash", "deepseek-v4-pro"]
        );
        assert_eq!(
            account.api_model_mappings,
            super::default_deepseek_api_model_mappings()
        );
    }

    #[test]
    fn api_model_mappings_normalize_and_resolve_upstream() {
        let mappings = super::normalize_api_model_mappings(vec![
            CodexApiModelMapping {
                client_model: " gpt-5.6-sol ".to_string(),
                upstream_model: " deepseek-v4-flash ".to_string(),
            },
            CodexApiModelMapping {
                client_model: "".to_string(),
                upstream_model: "".to_string(),
            },
        ])
        .expect("normalize mappings");
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].client_model, "gpt-5.6-sol");
        assert_eq!(mappings[0].upstream_model, "deepseek-v4-flash");

        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        account.api_model_mappings = mappings;
        assert_eq!(
            super::resolve_account_upstream_model(&account, "gpt-5.6-sol"),
            "deepseek-v4-flash"
        );
        assert_eq!(
            super::resolve_account_upstream_model(&account, "deepseek-v4-flash"),
            "deepseek-v4-flash"
        );
        assert_eq!(
            super::resolve_account_upstream_model(&account, "gpt-5.4"),
            "gpt-5.4"
        );
    }

    #[test]
    fn api_model_context_windows_keep_mapping_keys_and_drop_invalid() {
        let mappings = vec![CodexApiModelMapping {
            client_model: "gpt-5.6-sol".to_string(),
            upstream_model: "custom-flash".to_string(),
        }];
        let mut windows = std::collections::HashMap::new();
        windows.insert("custom-flash".to_string(), 900_000);
        windows.insert("stale-model".to_string(), 128_000);
        windows.insert("keep-default".to_string(), 0);
        let normalized = super::normalize_api_model_context_windows(
            windows,
            &["keep-default".to_string()],
            &mappings,
        );
        assert_eq!(normalized.get("custom-flash").copied(), Some(900_000));
        assert!(!normalized.contains_key("stale-model"));
        assert!(!normalized.contains_key("keep-default"));
    }

    #[test]
    fn deepseek_account_normalize_preserves_explicit_chat_completions() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-chat".to_string()],
        );
        account.api_wire_api = Some("chat_completions".to_string());
        account.api_sync_model_catalog_to_codex = false;

        assert!(super::normalize_deepseek_account(&mut account));
        assert_eq!(
            account.api_base_url.as_deref(),
            Some("https://api.deepseek.com")
        );
        assert_eq!(account.api_wire_api.as_deref(), Some("chat_completions"));
        assert!(!account.api_sync_model_catalog_to_codex);
        assert_eq!(account.api_model_catalog, vec!["deepseek-chat".to_string()]);
    }

    #[test]
    fn deepseek_direct_provider_catalog_uses_display_whitelist_and_upstream_names() {
        let json = super::build_deepseek_direct_provider_catalog_json(&[]).expect("build catalog");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog");
        let models = value
            .get("models")
            .and_then(|item| item.as_array())
            .expect("models array");
        assert!(models.len() >= 2);
        assert_eq!(
            models[0].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            models[0].get("display_name").and_then(|item| item.as_str()),
            Some("DeepSeek-V4-Flash")
        );
        assert_eq!(
            models[0].get("description").and_then(|item| item.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            models[0].get("visibility").and_then(|item| item.as_str()),
            Some("list")
        );
        assert_eq!(
            models[0]
                .get("apply_patch_tool_type")
                .and_then(|item| item.as_str()),
            Some("freeform")
        );
        assert_eq!(
            models[1].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-v4-pro")
        );
        assert_eq!(
            models[1].get("display_name").and_then(|item| item.as_str()),
            Some("DeepSeek-V4-Pro")
        );
    }

    #[test]
    fn deepseek_official_catalog_json_prefers_flash_and_keeps_tool_metadata() {
        let json = super::build_deepseek_official_model_catalog_json(&[]).expect("build catalog");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog");
        let models = value
            .get("models")
            .and_then(|item| item.as_array())
            .expect("models array");
        assert!(models.len() >= 2);
        assert_eq!(
            models[0].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            models[0]
                .get("apply_patch_tool_type")
                .and_then(|item| item.as_str()),
            Some("freeform")
        );
        assert_eq!(
            models[0].get("shell_type").and_then(|item| item.as_str()),
            Some("shell_command")
        );
        assert!(models[0]
            .get("base_instructions")
            .and_then(|item| item.as_str())
            .is_some_and(|text| !text.trim().is_empty()));
        assert_eq!(
            models[1].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-v4-pro")
        );
    }

    #[test]
    fn deepseek_official_runtime_replaces_leftover_shell_model() {
        let base_dir = make_temp_dir("codex-deepseek-official-runtime-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model = "gpt-5.6-sol"
model_provider = "codex_local_access"
model_catalog_json = "cockpit-local-access-model-catalog.json"

[model_providers.codex_local_access]
base_url = "http://localhost:58393/v1"
wire_api = "responses"
"#,
        )
        .expect("write leftover config");

        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(
            super::sync_deepseek_shell_remap_catalog_to_dir(&base_dir, &account)
                .expect("write shell remap catalog")
        );

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(!config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        let catalog_path = super::deepseek_official_model_catalog_path(&base_dir);
        let catalog = fs::read_to_string(&catalog_path).expect("read official catalog");
        assert!(catalog.contains("\"slug\": \"gpt-5.5\""));
        assert!(catalog.contains("DeepSeek-V4-Flash"));
        assert!(catalog.contains("apply_patch_tool_type"));
        assert!(catalog.contains("shell_command"));
        assert!(!catalog.contains("\"slug\": \"deepseek-v4-flash\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_official_catalog_sync_replaces_leftover_shell_model() {
        let base_dir = make_temp_dir("codex-deepseek-official-catalog-sync-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model = "gpt-5.6-sol"
model_provider = "codex_local_access"
model_catalog_json = "cockpit-local-access-model-catalog.json"
"#,
        )
        .expect("write leftover config");

        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        assert!(
            super::sync_deepseek_shell_remap_catalog_to_dir(&base_dir, &account)
                .expect("sync shell remap catalog")
        );

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(!config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        let catalog_path = super::deepseek_official_model_catalog_path(&base_dir);

        let catalog = fs::read_to_string(&catalog_path).expect("read official catalog");
        assert!(catalog.contains("\"slug\": \"gpt-5.5\""));
        assert!(catalog.contains("\"slug\": \"gpt-5.4\""));
        assert!(catalog.contains("DeepSeek-V4-Flash"));
        assert!(catalog.contains("apply_patch_tool_type"));
        assert!(catalog.contains("shell_command"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_official_runtime_writes_extra_instance_provider_catalog_and_clears_cache() {
        let instance_dir = make_temp_dir("codex-extra-instance-deepseek-official-catalog");
        fs::write(
            instance_dir.join("config.toml"),
            r#"model = "gpt-5.6-sol"
model_provider = "codex_local_access"
model_catalog_json = "cockpit-provider-model-catalog.json"
"#,
        )
        .expect("write leftover extra-instance config");
        fs::write(
            instance_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"gpt-5.6-sol","display_name":"deepseek-v4-flash"}]}"#,
        )
        .expect("write leftover gateway catalog");
        fs::write(
            instance_dir.join("models.json"),
            r#"{"models":[{"slug":"deepseek-v4-flash"}]}"#,
        )
        .expect("write leftover models.json");
        fs::write(
            instance_dir.join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-5.4"}]}"#,
        )
        .expect("write stale extra-instance model cache");

        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&instance_dir, &account).expect("write extra instance bundle");

        let catalog_path = super::deepseek_official_model_catalog_path(&instance_dir);
        let config = fs::read_to_string(instance_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.5\""));
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert_eq!(
            catalog_path.file_name().and_then(|name| name.to_str()),
            Some("cockpit-model-catalog.json")
        );
        assert!(!instance_dir.join("models.json").exists());
        assert!(!instance_dir.join("models_cache.json").exists());

        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(&catalog_path).expect("read instance provider catalog"),
        )
        .expect("parse instance provider catalog");
        let models = catalog
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models");
        let flash = models
            .iter()
            .find(|model| model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.5"))
            .expect("flash shell slug");
        assert_eq!(
            flash
                .get("display_name")
                .and_then(serde_json::Value::as_str),
            Some("DeepSeek-V4-Flash")
        );
        assert_eq!(
            flash.get("visibility").and_then(serde_json::Value::as_str),
            Some("list")
        );
        assert_eq!(
            flash
                .get("apply_patch_tool_type")
                .and_then(serde_json::Value::as_str),
            Some("freeform")
        );
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.4")
                && model
                    .get("display_name")
                    .and_then(serde_json::Value::as_str)
                    == Some("DeepSeek-V4-Pro")
        }));

        fs::remove_dir_all(&instance_dir).expect("cleanup extra instance dir");
    }

    #[test]
    fn deepseek_direct_bundle_writes_startup_model_without_shell_catalog() {
        let instance_dir = make_temp_dir("codex-deepseek-direct-startup-model");
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;
        account.api_instance_access_mode = Some("direct".to_string());
        account.api_startup_model = Some("deepseek-v4-pro".to_string());

        write_account_bundle_to_dir(&instance_dir, &account).expect("write direct bundle");

        let config = fs::read_to_string(instance_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"deepseek-v4-pro\""));
        assert!(config.contains("model_provider = \"deepseek\""));
        assert!(config.contains("base_url = \"https://api.deepseek.com\""));
        assert!(!config.contains("model_catalog_json"));
        assert!(!instance_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&instance_dir).expect("cleanup extra instance dir");
    }

    #[test]
    fn deepseek_gateway_bundle_writes_startup_shell_model() {
        let instance_dir = make_temp_dir("codex-deepseek-gateway-startup-model");
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;
        account.api_instance_access_mode = Some("gateway".to_string());
        account.api_startup_model = Some("deepseek-v4-pro".to_string());

        write_account_bundle_to_dir(&instance_dir, &account).expect("write gateway bundle");

        let config = fs::read_to_string(instance_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"gpt-5.4\""));
        assert!(config.contains("model_catalog_json"));
        assert!(instance_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&instance_dir).expect("cleanup extra instance dir");
    }

    #[test]
    fn deepseek_cdp_bundle_writes_official_provider_and_official_catalog() {
        let instance_dir = make_temp_dir("codex-deepseek-cdp-official-picker");
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;
        account.api_instance_access_mode = Some("cdp".to_string());
        account.api_startup_model = Some("deepseek-v4-pro".to_string());

        write_account_bundle_to_dir(&instance_dir, &account).expect("write cdp bundle");

        let config = fs::read_to_string(instance_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model = \"deepseek-v4-pro\""));
        assert!(!config.contains("model = \"gpt-5.4\""));
        assert!(config.contains("model_provider = \"deepseek\""));
        assert!(config.contains("base_url = \"https://api.deepseek.com\""));
        assert!(config.contains("model_catalog_json"));
        assert!(instance_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        let catalog =
            fs::read_to_string(instance_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read cdp catalog");
        assert!(catalog.contains("\"slug\": \"deepseek-v4-pro\""));
        assert!(!catalog.contains("\"slug\": \"gpt-5.4\""));

        fs::remove_dir_all(&instance_dir).expect("cleanup extra instance dir");
    }

    #[test]
    fn update_account_instance_access_saves_deepseek_start_choice() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-deepseek-instance-access-test");
        let mut account = CodexAccount::new_api_key(
            "deepseek-access".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        save_account(&account).expect("save account");

        let updated = update_account_instance_access(
            &account.id,
            Some("direct".to_string()),
            Some("deepseek-v4-pro".to_string()),
        )
        .expect("update access");
        assert_eq!(updated.api_instance_access_mode.as_deref(), Some("direct"));
        assert_eq!(
            updated.api_startup_model.as_deref(),
            Some("deepseek-v4-pro")
        );

        account.api_wire_api = Some("chat_completions".to_string());
        save_account(&account).expect("save chat account");
        let chat_error = update_account_instance_access(
            &account.id,
            Some("direct".to_string()),
            Some("deepseek-v4-flash".to_string()),
        )
        .expect_err("chat rejects direct");
        assert!(chat_error.contains("Chat Completions"));

        let chat_updated = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-v4-pro".to_string()),
        )
        .expect("chat can save startup model");
        assert_eq!(
            chat_updated.api_instance_access_mode.as_deref(),
            Some("gateway")
        );
        assert_eq!(
            chat_updated.api_startup_model.as_deref(),
            Some("deepseek-v4-pro")
        );

        account.api_wire_api = Some("responses".to_string());
        save_account(&account).expect("save responses account");
        let cdp = update_account_instance_access(
            &account.id,
            Some("cdp".to_string()),
            Some("deepseek-v4-flash".to_string()),
        )
        .expect("responses can save cdp");
        assert_eq!(cdp.api_instance_access_mode.as_deref(), Some("cdp"));
        assert!(super::account_uses_deepseek_cdp_injection(&cdp));
    }

    #[test]
    fn responses_api_key_bundle_keeps_external_catalog_without_managed_catalog() {
        let base_dir = make_temp_dir("codex-api-key-user-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_catalog_json = "user-model-catalog.json"
"#,
        )
        .expect("write config");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn chat_completions_api_key_bundle_defers_catalog_to_provider_gateway_start() {
        let base_dir = make_temp_dir("codex-chat-api-key-model-catalog-test");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["chat-model".to_string()],
        );
        account.api_wire_api = Some("chat_completions".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("experimental_bearer_token = \"sk-custom\""));
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn builtin_openai_responses_api_key_bundle_uses_official_model_discovery() {
        let base_dir = make_temp_dir("codex-builtin-responses-model-catalog-test");
        let mut account = CodexAccount::new_api_key(
            "openai-api-key".to_string(),
            "openai@example.com".to_string(),
            "sk-openai".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            Some("https://api.openai.com/v1".to_string()),
            None,
            None,
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config_path = base_dir.join("config.toml");
        if config_path.exists() {
            let config = fs::read_to_string(&config_path).expect("read config");
            assert!(!config.contains("model_catalog_json"));
        }
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bundle_bound_to_oauth_uses_dynamic_model_discovery() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-oauth-model-catalog-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "full",
            "rt-full",
        ));

        let mut api_key_account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["provider-model".to_string()],
        );
        api_key_account.api_wire_api = Some("responses".to_string());
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(!config.contains("model_catalog_json"));
        assert!(!profile_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
    }

    #[test]
    fn api_key_config_toml_clears_builtin_url_without_touching_other_providers() {
        let base_dir = make_temp_dir("codex-config-clean-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "mimo"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "cockpit-provider-model-catalog.json"
model_context_window = 1000000

[model_providers.mimo]
name = "Mimo"
base_url = "https://mimo.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.cockpit_api]
name = "Cockpit Api"
base_url = "https://chongcodex.cn/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.openai_api_key]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.codex_local_access]
name = "Old Local Access"
base_url = "https://old-local.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-old"
custom_flag = "keep-me"

[model_providers.relay]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[features]
multi_agent = true
"#,
        )
        .expect("write legacy config");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_key_bearer_provider_override_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            false,
            true,
            "responses",
        )
        .expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"codex_local_access\""));
        assert!(content.contains("[model_providers.codex_local_access]"));
        assert!(content.contains("base_url = \"https://api.openai.com/v1\""));
        assert!(content.contains("experimental_bearer_token = \"sk-test\""));
        assert!(content.contains("custom_flag = \"keep-me\""));
        assert!(content.contains("[model_providers.mimo]"));
        assert!(content.contains("[model_providers.cockpit_api]"));
        assert!(content.contains("[model_providers.openai_api_key]"));
        assert!(content.contains("[model_providers.relay]"));
        assert!(content.contains("model_catalog_json = \"cockpit-provider-model-catalog.json\""));
        assert!(!content.contains("openai_base_url"));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }
