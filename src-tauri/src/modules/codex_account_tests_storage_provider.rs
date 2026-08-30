// Codex 账号测试：Storage synchronization and API provider projection behavior。
// 测试与生产实现共享 super 作用域，验证真实持久化和运行态行为。
    #[test]
    fn current_account_does_not_sync_tokens_from_official_store() {
        let data_dir = make_temp_dir("codex-current-account-sync-test");
        let codex_home = data_dir.join(".codex");

        let stored = build_test_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-old",
        ));
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "latest",
            "rt-latest",
        );
        write_oauth_auth_file(&codex_home, &latest_tokens, "acc-current");

        let index = build_test_account_index(&stored);
        write_test_account(&data_dir, &stored);
        assert_eq!(
            index.current_account_id.as_deref(),
            Some(stored.id.as_str())
        );

        let current = get_current_account_from_loaded(
            index,
            |account_id| Some(load_test_account(&data_dir, account_id)),
            &codex_home,
        )
        .expect("current account");
        assert_eq!(current.id, stored.id);
        assert_eq!(current.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            current.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );

        let persisted = load_test_account(&data_dir, &stored.id);
        assert_eq!(persisted.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sync_account_from_auth_dir_updates_store_for_managed_home() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-auth-dir-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_account_from_auth_dir(&stored.id, &managed_home).expect("sync account");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );

        let persisted = load_account(&stored.id).expect("persisted account");
        assert_eq!(persisted.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn managed_projection_sync_requires_projection_marker() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-managed-projection-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        write_oauth_auth_file(&managed_home, &stored.tokens, "acc-current");
        write_managed_projection_to_dir(&managed_home, &stored).expect("write managed projection");

        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_managed_projection_from_auth_dir(&stored.id, &managed_home)
            .expect("sync managed projection");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
        assert!(synced.token_generation > stored.token_generation);
    }

    #[test]
    fn config_toml_uses_openai_base_url_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-openai-base-url-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.example.com/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("openai_base_url = \"https://api.example.com\""));
        #[cfg(target_os = "windows")]
        assert!(content.contains("model_provider = \"openai\""));
        #[cfg(not(target_os = "windows"))]
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("codex_local_access"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: Some("https://api.example.com".to_string()),
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_skips_default_official_endpoint_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-openai-default-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        assert!(!config_path.exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn mixed_model_routing_validation_keeps_oauth_identity_and_api_route_separate() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-mixed-model-routing-validation-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "subscription@example.com",
            "acc-subscription",
            "org-subscription",
            "subscription",
            "rt-subscription",
        ));
        let api_account = CodexAccount::new_api_key(
            "cpa-api".to_string(),
            "CPA".to_string(),
            "sk-cpa-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://cpa.example.com/v1".to_string()),
            Some("cpa".to_string()),
            Some("CPA".to_string()),
            vec!["gpt-5.5".to_string(), "grok-4.6".to_string()],
        );
        save_account(&api_account).expect("save API account");
        let routing = crate::models::CodexInstanceModelRouting {
            enabled: true,
            version: 1,
            routes: vec![crate::models::CodexInstanceApiRoute {
                id: "route-cpa".to_string(),
                namespace: "CPA".to_string(),
                provider_account_id: api_account.id.clone(),
                enabled: true,
                selected_models: None,
                extra_models: None,
            }],
        };

        let normalized = crate::modules::codex_local_access::validate_mixed_model_routing_config(
            Some(&oauth_account.id),
            &routing,
        )
        .expect("validate mixed routing");

        assert_eq!(normalized.routes[0].namespace, "cpa");
        assert_eq!(normalized.routes[0].provider_account_id, api_account.id);

        let empty_selection = crate::models::CodexInstanceModelRouting {
            routes: vec![crate::models::CodexInstanceApiRoute {
                selected_models: Some(Vec::new()),
                ..routing.routes[0].clone()
            }],
            ..routing.clone()
        };
        let error = crate::modules::codex_local_access::validate_mixed_model_routing_config(
            Some(&oauth_account.id),
            &empty_selection,
        )
        .expect_err("an enabled route must not accept an empty model allowlist");
        assert!(error.contains("至少需要选择一个上游模型"));

        let oauth_as_provider = crate::models::CodexInstanceModelRouting {
            routes: vec![crate::models::CodexInstanceApiRoute {
                provider_account_id: oauth_account.id.clone(),
                ..routing.routes[0].clone()
            }],
            ..routing
        };
        let error = crate::modules::codex_local_access::validate_mixed_model_routing_config(
            Some(&oauth_account.id),
            &oauth_as_provider,
        )
        .expect_err("OAuth account must not be accepted as an API route");
        assert!(error.contains("不能使用订阅 OAuth 账号"));
    }

    #[test]
    fn config_toml_removes_runtime_provider_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-clean-managed-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "codex_local_access"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "cockpit-provider-model-catalog.json"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-history"

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

[model_providers.user_manual_provider_not_managed]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false
"#,
        )
        .expect("write managed provider config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        #[cfg(target_os = "windows")]
        assert!(content.contains("model_provider = \"openai\""));
        #[cfg(not(target_os = "windows"))]
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("[model_providers.codex_local_access]"));
        assert!(!content.contains("experimental_bearer_token = \"sk-history\""));
        assert!(!content.contains("[model_providers.cockpit_api]"));
        assert!(!content.contains("[model_providers.openai_api_key]"));
        assert!(content.contains("[model_providers.user_manual_provider_not_managed]"));
        assert!(!content.contains("model_catalog_json"));
        assert!(!content.contains("openai_base_url"));
        assert!(content.contains("model_context_window = 1000000"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_removes_local_access_catalog_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-clean-local-access-catalog-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "openai"
model_catalog_json = "cockpit-local-access-model-catalog.json"
model_context_window = 1000000
"#,
        )
        .expect("write stale local access config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_catalog_json"));
        assert!(content.contains("model_context_window = 1000000"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_preserves_user_model_catalog_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-preserve-user-catalog-builtin-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "user_manual_provider"
model_catalog_json = "user-model-catalog.json"
model_context_window = 1000000

[model_providers.user_manual_provider]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false

[features]
multi_agent = true
"#,
        )
        .expect("write user provider config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"user_manual_provider\""));
        assert!(content.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(content.contains("[model_providers.user_manual_provider]"));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_preserves_openai_http_provider_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-preserve-openai-http-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "openai_http"
openai_base_url = "https://legacy.example.com/v1"

[model_providers.openai_http]
name = "OpenAI HTTP"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.codex_local_access]
name = "Managed Local Access"
base_url = "https://managed.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.cockpit_api]
name = "Managed Cockpit API"
base_url = "https://managed.example.com/api"
wire_api = "responses"
requires_openai_auth = false
"#,
        )
        .expect("write user provider config");
        let provider_config = resolve_api_provider_config(
            Some("https://api.example.com/v1"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"openai_http\""));
        assert!(content.contains("[model_providers.openai_http]"));
        assert!(content.contains("openai_base_url = \"https://api.example.com/v1\""));
        assert!(!content.contains("[model_providers.codex_local_access]"));
        assert!(!content.contains("[model_providers.cockpit_api]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_preserves_user_model_catalog_when_switching_to_custom_provider() {
        let base_dir = make_temp_dir("codex-config-preserve-user-catalog-custom-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "user_manual_provider"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "user-model-catalog.json"
model_context_window = 1000000

[model_providers.user_manual_provider]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false

[features]
multi_agent = true
"#,
        )
        .expect("write user provider config");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"relay\""));
        assert!(content.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(content.contains("[model_providers.relay]"));
        assert!(content.contains("[model_providers.user_manual_provider]"));
        assert!(!content.contains("openai_base_url"));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_uses_model_provider_section_for_custom_provider() {
        let base_dir = make_temp_dir("codex-config-custom-provider-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"relay\""));
        assert!(content.contains("[model_providers.relay]"));
        assert!(!content.contains("codex_local_access"));
        assert!(content.contains("name = \"Relay\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = false"));
        assert!(content.contains("supports_websockets = false"));
        assert!(!content.contains("openai_base_url"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: Some("relay".to_string()),
                provider_name: Some("Relay".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_keeps_builtin_openai_for_default_official_endpoint() {
        let base_dir = make_temp_dir("codex-api-key-config-openai-default-test");
        let account = CodexAccount::new_api_key(
            "openai-api-key".to_string(),
            "openai@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            Some("https://api.openai.com/v1/".to_string()),
            None,
            None,
            Vec::new(),
        );

        write_auth_file_to_dir(&base_dir, &account).expect("write auth bundle");

        let config_path = base_dir.join("config.toml");
        assert!(!config_path.exists());
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_uses_http_only_provider_for_relay_without_websocket_support() {
        let base_dir = make_temp_dir("codex-api-key-config-custom-provider-test");
        let mut account = CodexAccount::new_api_key(
            "relay".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1/".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        write_auth_file_to_dir(&base_dir, &account).expect("write relay auth bundle");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"codex_local_access\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("supports_websockets = false"));
        assert!(content.contains("requires_openai_auth = true"));
        assert!(!content.contains("openai_base_url"));
        assert!(!content.contains("[model_providers.relay]"));
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read relay auth"),
        )
        .expect("parse relay auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-test");
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: Some(CODEX_RUNTIME_MODEL_PROVIDER_ID.to_string()),
                provider_name: Some("Relay".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_account_switch_updates_relay_key_and_base_url_together() {
        let base_dir = make_temp_dir("codex-api-key-relay-switch-test");
        let mut first = CodexAccount::new_api_key(
            "relay-a".to_string(),
            "relay-a@example.com".to_string(),
            "sk-relay-a".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay-a.example.com/v1".to_string()),
            Some("relay_a".to_string()),
            Some("Relay A".to_string()),
            Vec::new(),
        );
        first.api_wire_api = Some("responses".to_string());

        write_auth_file_to_dir(&base_dir, &first).expect("write first relay account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read first auth"),
        )
        .expect("parse first auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-relay-a");
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read first config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("base_url = \"https://relay-a.example.com/v1\""));
        assert!(config.contains("supports_websockets = false"));
        assert!(!config.contains("openai_base_url"));

        sync_api_key_account_from_local_state(&mut first, &base_dir);
        assert_eq!(first.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(first.api_provider_id.as_deref(), Some("relay_a"));
        assert_eq!(first.api_provider_name.as_deref(), Some("Relay A"));

        let mut second = CodexAccount::new_api_key(
            "relay-b".to_string(),
            "relay-b@example.com".to_string(),
            "sk-relay-b".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay-b.example.com/v1".to_string()),
            Some("relay_b".to_string()),
            Some("Relay B".to_string()),
            Vec::new(),
        );
        second.api_wire_api = Some("responses".to_string());

        write_auth_file_to_dir(&base_dir, &second).expect("write second relay account");
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read second auth"),
        )
        .expect("parse second auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-relay-b");
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read second config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("base_url = \"https://relay-b.example.com/v1\""));
        assert!(config.contains("supports_websockets = false"));
        assert!(!config.contains("relay-a.example.com"));
        assert!(!config.contains("openai_base_url"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn editing_current_api_key_account_rewrites_relay_key_and_base_url() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-edit-runtime-test");
        let mut account = CodexAccount::new_api_key(
            "relay-before-edit".to_string(),
            "relay-before@example.com".to_string(),
            "sk-before-edit".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://before.example.com/v1".to_string()),
            Some("before_relay".to_string()),
            Some("Before Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        save_account(&account).expect("save API key account");
        let mut index = CodexAccountIndex::new();
        index.current_account_id = Some(account.id.clone());
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
        save_account_index(&index).expect("mark account current");
        write_account_bundle_to_dir(&env.codex_home(), &account).expect("write initial account");

        let updated = update_api_key_credentials(
            &account.id,
            "sk-after-edit".to_string(),
            Some("https://after.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("after_relay".to_string()),
            Some("After Relay".to_string()),
            Vec::new(),
            Some(false),
            Some("responses".to_string()),
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
            None,
        )
        .expect("update API key account");

        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(env.codex_home().join("auth.json")).expect("read edited auth"),
        )
        .expect("parse edited auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-after-edit");
        let config =
            fs::read_to_string(env.codex_home().join("config.toml")).expect("read edited config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("base_url = \"https://after.example.com/v1\""));
        assert!(config.contains("supports_websockets = false"));
        assert!(!config.contains("before.example.com"));
        assert!(!config.contains("openai_base_url"));
        assert_eq!(updated.api_provider_mode, CodexApiProviderMode::Custom);
        assert_eq!(updated.api_provider_id.as_deref(), Some("after_relay"));
        assert_eq!(updated.api_provider_name.as_deref(), Some("After Relay"));
    }

    #[test]
    fn api_key_config_toml_enables_imagegen_for_capable_provider() {
        let base_dir = make_temp_dir("codex-api-key-config-imagegen-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"[model_providers.codex_local_access.http_headers]
X-Custom = "keep-me"
"#,
        )
        .expect("write existing headers");
        let provider_config = resolve_api_provider_config(
            Some("http://127.0.0.1:14998/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("codex_local_access"),
            Some("Codex API Service"),
        )
        .expect("resolve provider config");

        write_api_key_bearer_provider_override_to_config_toml(
            &base_dir,
            &provider_config,
            "agt_codex_test",
            false,
            true,
            false,
            "responses",
        )
        .expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        let parsed = content.parse::<Document>().expect("parse config");
        let provider = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex_local_access"))
            .and_then(|item| item.as_table())
            .expect("codex_local_access provider");
        assert_eq!(
            provider
                .get("requires_openai_auth")
                .and_then(|item| item.as_bool()),
            Some(false)
        );
        let headers = provider
            .get("http_headers")
            .and_then(|item| item.as_table())
            .expect("http_headers table");
        assert_eq!(
            headers
                .get(CODEX_IMAGEGEN_ACTOR_HEADER)
                .and_then(|item| item.as_str()),
            Some(CODEX_IMAGEGEN_ACTOR_HEADER_VALUE)
        );
        assert_eq!(
            headers
                .get(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER)
                .and_then(|item| item.as_str()),
            Some(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE)
        );
        assert_eq!(
            headers.get("X-Custom").and_then(|item| item.as_str()),
            Some("keep-me")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn remote_api_key_imagegen_does_not_disable_hosted_chat_tool() {
        let base_dir = make_temp_dir("codex-remote-api-key-imagegen-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.apikey.fun/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("apikey_fun"),
            Some("APIKey.fun"),
        )
        .expect("resolve provider config");

        write_api_key_bearer_provider_override_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            true,
            false,
            "responses",
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(!content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_removes_imagegen_header_but_keeps_custom_headers() {
        let base_dir = make_temp_dir("codex-api-key-config-imagegen-cleanup-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"[model_providers.codex_local_access]
http_headers = { "x-openai-actor-authorization" = "legacy", "X-Custom" = "keep-me" }
"#,
        )
        .expect("write existing headers");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
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
        let parsed = content.parse::<Document>().expect("parse config");
        let provider = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex_local_access"))
            .and_then(|item| item.as_table())
            .expect("codex_local_access provider");
        assert_eq!(
            provider
                .get("requires_openai_auth")
                .and_then(|item| item.as_bool()),
            Some(true)
        );
        let headers = provider
            .get("http_headers")
            .and_then(|item| item.as_inline_table())
            .expect("http_headers inline table");
        assert!(headers
            .iter()
            .all(|(name, _)| { !name.eq_ignore_ascii_case(CODEX_IMAGEGEN_ACTOR_HEADER) }));
        assert_eq!(
            headers.get("X-Custom").and_then(|item| item.as_str()),
            Some("keep-me")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bundle_enables_imagegen_when_catalog_contains_image_model() {
        let base_dir = make_temp_dir("codex-api-key-bundle-imagegen-test");
        let account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "agt_codex_test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("requires_openai_auth = false"));
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER_VALUE));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn pure_responses_relay_without_image_catalog_uses_builtin_openai() {
        let base_dir = make_temp_dir("codex-third-party-clear-stale-actor");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "sk-old"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools" }
supports_websockets = false
"#,
        )
        .expect("seed stale imagegen config");

        let account = CodexAccount::new_api_key(
            "relay-no-image".to_string(),
            "relay@example.com".to_string(),
            "sk-new".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );

        write_account_bundle_to_dir(&base_dir, &account).expect("rewrite without image catalog");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(
            !content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "stale actor must be cleared when catalog has no gpt-image-2: {content}"
        );
        assert!(content.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!content.contains("experimental_bearer_token"));
        assert!(!content.contains("codex_local_access"));
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join("auth.json")).expect("read auth"),
        )
        .expect("parse auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-new");

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn pure_api_key_local_access_writes_imagegen_takeover_shape() {
        let base_dir = make_temp_dir("codex-local-access-pure-api-key-takeover-shape");
        let provider_config = resolve_api_provider_config(
            Some("http://localhost:12345/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("codex_local_access"),
            Some("Codex API Service"),
        )
        .expect("resolve provider config");

        write_api_key_bearer_provider_override_to_config_toml(
            &base_dir,
            &provider_config,
            "agt_codex_test",
            false,
            true,
            false,
            "responses",
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(
            content.contains("requires_openai_auth = false"),
            "pure API Key local-access must disable openai auth gate: {content}"
        );
        assert!(
            content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "pure API Key local-access must write actor header: {content}"
        );
        assert!(
            content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER),
            "pure API Key local-access should keep chat images-only header: {content}"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bound_oauth_keeps_oauth_login_and_imagegen_when_catalog_has_image() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-api-key-bound-oauth-auth-test");
        let base_dir = make_temp_dir("codex-api-key-bound-oauth-auth-test");
        let mut oauth = CodexAccount::new(
            "oauth-bound-auth-test".to_string(),
            "oauth@example.com".to_string(),
            make_codex_tokens(
                "oauth@example.com",
                "acc-bound-auth-test",
                "org-bound-auth-test",
                "bound-auth-test",
                "refresh.token",
            ),
        );
        oauth.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
        save_account(&oauth).expect("save oauth");

        let mut api_key = CodexAccount::new_api_key(
            "api-key-bound-auth-test".to_string(),
            "api@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string(), "gpt-5.5".to_string()],
        );
        api_key.bound_oauth_account_id = Some(oauth.id.clone());
        save_account(&api_key).expect("save api key");

        write_account_bundle_to_dir(&base_dir, &api_key).expect("write bound oauth bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(
            content.contains("requires_openai_auth = true"),
            "bound OAuth must enable openai auth gate so Codex uses OAuth login: {content}"
        );
        assert!(
            content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "third-party bound OAuth with image catalog must write actor for imagegen: {content}"
        );
        // 非 loopback 不写 chat disable
        assert!(
            !content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER),
            "third-party should not set chat-only image disable: {content}"
        );

        let auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(base_dir.join("auth.json")).expect("auth"))
                .expect("parse auth");
        assert!(
            auth.get("tokens").is_some(),
            "auth should keep oauth tokens"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
        let _ = remove_accounts(&[oauth.id, api_key.id]);
    }

    #[test]
    fn api_key_bound_oauth_without_image_catalog_skips_actor() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-api-key-bound-oauth-no-image-test");
        let base_dir = make_temp_dir("codex-api-key-bound-oauth-no-image-test");
        let mut previous_relay = CodexAccount::new_api_key(
            "previous-relay".to_string(),
            "previous-relay@example.com".to_string(),
            "sk-previous".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://previous-relay.example.com/v1".to_string()),
            Some("previous_relay".to_string()),
            Some("Previous Relay".to_string()),
            Vec::new(),
        );
        previous_relay.api_wire_api = Some("responses".to_string());
        previous_relay.api_supports_websockets = true;
        write_account_bundle_to_dir(&base_dir, &previous_relay)
            .expect("write previous built-in relay bundle");
        let previous_config =
            fs::read_to_string(base_dir.join("config.toml")).expect("read previous config");
        assert!(
            previous_config.contains("openai_base_url = \"https://previous-relay.example.com/v1\"")
        );

        let mut oauth = CodexAccount::new(
            "oauth-bound-no-image-test".to_string(),
            "oauth-no-image@example.com".to_string(),
            make_codex_tokens(
                "oauth-no-image@example.com",
                "acc-bound-no-image-test",
                "org-bound-no-image-test",
                "bound-no-image-test",
                "refresh.token",
            ),
        );
        oauth.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
        save_account(&oauth).expect("save oauth");

        let mut api_key = CodexAccount::new_api_key(
            "api-key-bound-no-image-test".to_string(),
            "api-no-image@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key.bound_oauth_account_id = Some(oauth.id.clone());
        save_account(&api_key).expect("save api key");

        write_account_bundle_to_dir(&base_dir, &api_key).expect("write bound oauth bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("requires_openai_auth = true"));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(!content.contains("previous-relay.example.com"));
        assert!(!content.contains("openai_base_url"));
        assert!(
            !content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "no image model in catalog → no actor: {content}"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
        let _ = remove_accounts(&[oauth.id, api_key.id]);
    }

    #[test]
    fn api_key_config_toml_enables_websockets_when_account_supports_them() {
        let base_dir = make_temp_dir("codex-api-key-config-websocket-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_key_bearer_provider_override_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            true,
            false,
            true,
            "responses",
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("supports_websockets = true"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_snapshot_sync_updates_account_and_current_config_without_touching_last_used() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-provider-snapshot-sync-test");
        let mut account = CodexAccount::new_api_key(
            "relay-account".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        account.last_used = 123;
        save_account(&account).expect("save account");

        let mut index = CodexAccountIndex::new();
        index.current_account_id = Some(account.id.clone());
        save_account_index(&index).expect("save account index");

        let updated = sync_api_key_provider_accounts(
            vec![account.id.clone(), account.id.clone()],
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5".to_string()],
            Some("responses".to_string()),
            true,
            false,
            Default::default(),
            None,
            None,
        )
        .expect("sync provider snapshot");

        assert_eq!(updated, 1);
        let saved = load_account(&account.id).expect("load updated account");
        assert!(saved.api_supports_websockets);
        assert_eq!(saved.api_wire_api.as_deref(), Some("responses"));
        assert_eq!(saved.api_model_catalog, vec!["gpt-5".to_string()]);
        assert_eq!(saved.last_used, 123);

        let config =
            fs::read_to_string(env.codex_home().join("config.toml")).expect("read current config");
        assert!(config.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!config.contains("codex_local_access"));
        assert!(!config.contains("supports_websockets = "));
    }

    #[test]
    fn api_key_bundle_bound_to_empty_id_token_oauth_writes_api_key_auth_file() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-oauth-auth-file-test");
        let mut oauth_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "empty-id-token",
            "rt-empty-id-token",
        );
        oauth_tokens.id_token = String::new();
        let oauth_account = seed_oauth_account(oauth_tokens);

        let mut api_key_account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "local-service-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            Vec::new(),
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth file"),
        )
        .expect("parse auth file");
        assert_eq!(
            auth_file.get("auth_mode").and_then(|value| value.as_str()),
            Some("apikey")
        );
        assert_eq!(
            auth_file
                .get("OPENAI_API_KEY")
                .and_then(|value| value.as_str()),
            Some("local-service-key")
        );
        assert!(
            auth_file.get("tokens").is_none(),
            "API-key local access profile should not write OAuth tokens: {}",
            auth_file
        );

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("base_url = \"http://127.0.0.1:14998/v1\""));
        assert!(config.contains("experimental_bearer_token = \"local-service-key\""));
    }

    #[test]
    fn api_key_bundle_bound_to_full_oauth_keeps_oauth_auth_file() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-full-oauth-auth-file-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "full",
            "rt-full",
        ));

        let mut api_key_account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "local-service-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth file"),
        )
        .expect("parse auth file");
        assert!(auth_file.get("auth_mode").is_none());
        assert_eq!(
            auth_file.get("OPENAI_API_KEY"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            auth_file
                .get("tokens")
                .and_then(|value| value.get("id_token"))
                .and_then(|value| value.as_str()),
            Some(oauth_account.tokens.id_token.as_str())
        );

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains("experimental_bearer_token = \"local-service-key\""));
        // local-access loopback + bound OAuth → also write imagegen headers
        assert!(config.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));
    }

    #[test]
    fn api_key_bound_oauth_projection_tracks_runtime_and_credential_owners() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-oauth-projection-owner-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "projection-owner",
            "rt-projection-owner",
        ));
        let mut api_key_account = CodexAccount::new_api_key(
            "projection-runtime".to_string(),
            "projection-runtime@example.com".to_string(),
            "sk-projection-runtime".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("bound-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account)
            .expect("write bound OAuth bundle");

        let projection =
            read_managed_projection_from_dir(&profile_dir).expect("read managed projection");
        assert_eq!(projection.version, CODEX_AUTH_PROJECTION_VERSION);
        assert_eq!(projection.account_id, api_key_account.id);
        assert_eq!(
            projection.credential_account_id.as_deref(),
            Some(oauth_account.id.as_str())
        );
        assert_eq!(
            projection.credential_token_generation,
            Some(oauth_account.token_generation)
        );
    }

    #[test]
    fn bound_oauth_rotation_sync_preserves_api_key_provider_config() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-bound-oauth-rotation-sync-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "before-rotation",
            "rt-before-rotation",
        ));
        let mut api_key_account = CodexAccount::new_api_key(
            "rotation-runtime".to_string(),
            "rotation-runtime@example.com".to_string(),
            "sk-rotation-runtime".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("rotation-profile");
        write_account_bundle_to_dir(&profile_dir, &api_key_account)
            .expect("write bound OAuth bundle");
        let config_before =
            fs::read_to_string(profile_dir.join("config.toml")).expect("read provider config");

        let mut rotated_account = oauth_account.clone();
        rotated_account.tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "after-rotation",
            "rt-after-rotation",
        );
        let rotated_auth = build_auth_file_value(&rotated_account).expect("build rotated auth");
        fs::write(
            profile_dir.join("auth.json"),
            serde_json::to_string_pretty(&rotated_auth).expect("serialize rotated auth"),
        )
        .expect("write rotated auth");

        let synced = sync_managed_projection_from_auth_dir(&oauth_account.id, &profile_dir)
            .expect("sync rotated OAuth tokens");

        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            Some("rt-after-rotation")
        );
        assert!(synced.token_generation > oauth_account.token_generation);
        let config_after =
            fs::read_to_string(profile_dir.join("config.toml")).expect("read preserved config");
        assert_eq!(config_after, config_before);
        assert!(config_after.contains("base_url = \"https://relay.example.com/v1\""));
        let projection =
            read_managed_projection_from_dir(&profile_dir).expect("read updated projection");
        assert_eq!(projection.account_id, api_key_account.id);
        assert_eq!(
            projection.credential_account_id.as_deref(),
            Some(oauth_account.id.as_str())
        );
        assert_eq!(
            projection.credential_token_generation,
            Some(synced.token_generation)
        );
    }

    #[test]
    fn managed_bound_oauth_accepts_rotated_rt_without_last_refresh() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-bound-oauth-no-last-refresh-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "authority-before",
            "rt-authority-before",
        ));
        let mut api_key_account = CodexAccount::new_api_key(
            "authority-runtime".to_string(),
            "authority-runtime@example.com".to_string(),
            "sk-authority-runtime".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("authority-profile");
        write_account_bundle_to_dir(&profile_dir, &api_key_account)
            .expect("write bound OAuth bundle");

        let mut rotated_account = oauth_account.clone();
        rotated_account.tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "authority-after",
            "rt-authority-after",
        );
        let mut rotated_auth = build_auth_file_value(&rotated_account).expect("build rotated auth");
        rotated_auth
            .as_object_mut()
            .expect("auth object")
            .remove("last_refresh");
        fs::write(
            profile_dir.join("auth.json"),
            serde_json::to_string_pretty(&rotated_auth).expect("serialize rotated auth"),
        )
        .expect("write rotated auth");

        let mut stored = load_account(&oauth_account.id).expect("load stored OAuth account");
        let changed = sync_account_from_authority_dir_if_current(&mut stored, &profile_dir)
            .expect("adopt managed authority rotation");

        assert!(changed);
        assert_eq!(
            stored.tokens.refresh_token.as_deref(),
            Some("rt-authority-after")
        );
        assert!(stored.token_generation > oauth_account.token_generation);
    }

    #[test]
    fn persisted_credential_owner_survives_api_key_unbind_for_later_oauth_sync() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-bound-oauth-unbind-owner-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "unbind-owner",
            "rt-unbind-owner",
        ));
        let mut api_key_account = CodexAccount::new_api_key(
            "unbind-runtime".to_string(),
            "unbind-runtime@example.com".to_string(),
            "sk-unbind-runtime".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        save_account(&api_key_account).expect("save bound API Key account");
        let profile_dir = env.home_dir.join("unbound-profile");
        write_account_bundle_to_dir(&profile_dir, &api_key_account)
            .expect("write bound OAuth bundle");

        let mut store = InstanceStore::new();
        store.instances.push(InstanceProfile {
            id: "unbound-instance".to_string(),
            name: "Unbound instance".to_string(),
            user_data_dir: profile_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: crate::models::codex::CodexAppSpeed::Standard,
            created_at: now_timestamp(),
            last_launched_at: None,
            last_pid: None,
        });
        crate::modules::codex_instance::save_instance_store(&store)
            .expect("save unbound instance store");
        api_key_account.bound_oauth_account_id = None;
        save_account(&api_key_account).expect("save unbound API Key account");

        let authority_dirs = authority_projection_dirs_for_account(&oauth_account);
        assert!(
            authority_dirs.iter().any(|dir| dir == &profile_dir),
            "persisted credential owner should keep the old combined profile discoverable"
        );
    }

    #[test]
    fn legacy_combined_projection_is_recovered_and_upgraded_after_unbind() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-legacy-bound-oauth-owner-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "legacy-owner",
            "rt-legacy-owner",
        ));
        let mut api_key_account = CodexAccount::new_api_key(
            "legacy-runtime".to_string(),
            "legacy-runtime@example.com".to_string(),
            "sk-legacy-runtime".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        save_account(&api_key_account).expect("save bound API Key account");
        let profile_dir = env.home_dir.join("legacy-profile");
        write_account_bundle_to_dir(&profile_dir, &api_key_account)
            .expect("write bound OAuth bundle");

        let mut legacy_projection =
            read_managed_projection_from_dir(&profile_dir).expect("read projection");
        legacy_projection.version = 1;
        legacy_projection.credential_account_id = None;
        legacy_projection.credential_email = None;
        legacy_projection.credential_token_generation = None;
        super::write_managed_projection_value_to_dir(&profile_dir, &legacy_projection)
            .expect("write legacy projection");

        let mut store = InstanceStore::new();
        store.instances.push(InstanceProfile {
            id: "legacy-instance".to_string(),
            name: "Legacy instance".to_string(),
            user_data_dir: profile_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: crate::models::codex::CodexAppSpeed::Standard,
            created_at: now_timestamp(),
            last_launched_at: None,
            last_pid: None,
        });
        crate::modules::codex_instance::save_instance_store(&store)
            .expect("save legacy instance store");
        api_key_account.bound_oauth_account_id = None;
        save_account(&api_key_account).expect("save unbound API Key account");

        let authority_dirs = authority_projection_dirs_for_account(&oauth_account);
        assert!(authority_dirs.iter().any(|dir| dir == &profile_dir));
        let mut stored = load_account(&oauth_account.id).expect("load stored OAuth account");
        assert!(
            !sync_account_from_authority_dir_if_current(&mut stored, &profile_dir)
                .expect("upgrade legacy projection without token delta")
        );
        let upgraded =
            read_managed_projection_from_dir(&profile_dir).expect("read upgraded projection");
        assert_eq!(upgraded.version, CODEX_AUTH_PROJECTION_VERSION);
        assert_eq!(
            upgraded.credential_account_id.as_deref(),
            Some(oauth_account.id.as_str())
        );
    }

    #[test]
    fn local_access_runtime_bound_oauth_keeps_oauth_login_and_imagegen() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-local-access-bound-oauth-takeover-shape");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "bound@example.com",
            "acc-bound",
            "org-bound",
            "bound-oauth",
            "rt-bound-oauth",
        ));

        let mut runtime = CodexAccount::new_api_key(
            "codex_local_access_runtime".to_string(),
            "api-service-local".to_string(),
            "agt_codex_takeover".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:12345/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );
        runtime.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("api-service-profile");

        write_account_bundle_to_dir(&profile_dir, &runtime).expect("write bound oauth takeover");

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(
            config.contains("requires_openai_auth = true"),
            "bound OAuth local-access must enable openai auth gate: {config}"
        );
        assert!(
            config.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "bound OAuth local-access must write actor for imagegen: {config}"
        );
        assert!(
            config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER)
                && config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE),
            "bound OAuth local-access must disable hosted chat imagegen: {config}"
        );
        assert!(config.contains("experimental_bearer_token = \"agt_codex_takeover\""));
        assert!(config.contains("base_url = \"http://localhost:12345/v1\""));

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth"),
        )
        .expect("parse auth");
        assert!(
            auth_file.get("tokens").is_some(),
            "auth.json should keep bound OAuth tokens"
        );
        assert!(auth_file.get("auth_mode").is_none());

        let _ = remove_accounts(&[oauth_account.id]);
    }

    #[test]
    fn responses_api_key_bundle_syncs_saved_model_catalog_when_enabled() {
        let base_dir = make_temp_dir("codex-api-key-managed-model-catalog-test");
        fs::write(base_dir.join("config.toml"), "model = \"legacy-model\"\n")
            .expect("write stale selected model");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![
                " custom-a ".to_string(),
                "custom-b".to_string(),
                "CUSTOM-A".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        // Catalog sync maps custom display models onto official slugs; relays use openai_base_url.
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!config.contains("codex_local_access"));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read managed catalog"),
        )
        .expect("parse managed catalog");
        let models = catalog
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models should be an array");
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.6-sol")
                && model
                    .get("display_name")
                    .and_then(serde_json::Value::as_str)
                    == Some("custom-a")
                && model.get("visibility").and_then(serde_json::Value::as_str) == Some("list")
        }));
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(serde_json::Value::as_str) == Some("gpt-5.6-terra")
                && model
                    .get("display_name")
                    .and_then(serde_json::Value::as_str)
                    == Some("custom-b")
                && model.get("visibility").and_then(serde_json::Value::as_str) == Some("list")
        }));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn responses_api_key_bundle_replaces_stale_local_access_catalog() {
        let base_dir = make_temp_dir("codex-api-key-replace-local-access-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_catalog_json = "cockpit-local-access-model-catalog.json"
"#,
        )
        .expect("write stale local access catalog config");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["custom-a".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        assert!(!config.contains("cockpit-local-access-model-catalog.json"));
        assert!(base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_upsert_without_sync_preference_preserves_instance_model_catalog() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let env = TestEnvGuard::new("codex-api-key-upsert-model-catalog-test");
        let api_key = "sk-upsert-model-catalog".to_string();

        let created = upsert_api_key_account(
            api_key.clone(),
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["custom-a".to_string()],
            Some(true),
            Some("responses".to_string()),
            false,
            false,
            std::collections::HashMap::new(),
            None,
            Some("Relay Key".to_string()),
            None,
        )
        .expect("create API key account");
        assert!(created.api_sync_model_catalog_to_codex);

        let updated = upsert_api_key_account(
            api_key,
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["custom-b".to_string()],
            None,
            Some("responses".to_string()),
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
            None,
        )
        .expect("upsert API key account without sync preference");
        assert!(updated.api_sync_model_catalog_to_codex);

        let profile_dir = env.home_dir.join("instance-profile");
        write_account_bundle_to_dir(&profile_dir, &updated)
            .expect("write multi-instance account projection");
        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        // Catalog sync maps custom display models onto official slugs; relays use openai_base_url.
        assert!(config.contains("model = \"gpt-5.6-sol\""));
        assert!(config.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!config.contains("codex_local_access"));
        let auth: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read instance auth"),
        )
        .expect("parse instance auth");
        assert_eq!(auth["OPENAI_API_KEY"], "sk-upsert-model-catalog");
        assert!(profile_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
    }

    #[test]
    fn responses_api_key_bundle_preserves_user_model_catalog() {
        let base_dir = make_temp_dir("codex-api-key-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_catalog_json = "user-model-catalog.json"
"#,
        )
        .expect("write user catalog config");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![
                " custom-a ".to_string(),
                "custom-b".to_string(),
                "CUSTOM-A".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(!base_dir
            .join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn responses_api_key_bundle_removes_stale_managed_model_catalog() {
        let base_dir = make_temp_dir("codex-api-key-empty-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[]}"#,
        )
        .expect("write managed catalog");
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
        account.api_supports_websockets = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("openai_base_url = \"https://relay.example.com/v1\""));
        assert!(!config.contains("codex_local_access"));
        assert!(!config.contains("supports_websockets = "));
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn cleanup_removes_existing_managed_model_catalog() {
        let base_dir = make_temp_dir("codex-managed-model-catalog-cleanup-test");
        fs::write(
            base_dir.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[]}"#,
        )
        .expect("write stale catalog");

        assert!(super::cleanup_managed_model_catalog_for_dir(&base_dir)
            .expect("cleanup managed catalog"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        let config_path = base_dir.join("config.toml");
        if config_path.exists() {
            let config = fs::read_to_string(&config_path).expect("read config");
            assert!(!config.contains("model_catalog_json"));
        }

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn managed_catalog_cleanup_preserves_custom_model_catalog() {
        let base_dir = make_temp_dir("codex-custom-model-catalog-cleanup-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"user-model-catalog.json\"\n",
        )
        .expect("write custom config");
        fs::write(
            base_dir.join("user-model-catalog.json"),
            r#"{"models":[{"slug":"user-model"}]}"#,
        )
        .expect("write custom catalog");

        assert!(!super::cleanup_managed_model_catalog_for_dir(&base_dir)
            .expect("preserve custom catalog"));
        assert_eq!(
            fs::read_to_string(base_dir.join("user-model-catalog.json"))
                .expect("read custom catalog"),
            r#"{"models":[{"slug":"user-model"}]}"#
        );
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn startup_cleanup_preserves_active_chat_completions_provider_catalog() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-chat-provider-startup-catalog-test");
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
        account.api_wire_api = Some("chat_completions".to_string());
        save_account(&account).expect("save chat completions account");
        save_account_index(&build_test_account_index(&account))
            .expect("save current account index");

        let codex_home = env.codex_home();
        fs::write(
            codex_home.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write provider catalog config");
        fs::write(
            codex_home.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"deepseek-v4-pro"}]}"#,
        )
        .expect("write provider catalog");

        assert_eq!(
            super::cleanup_managed_model_catalogs_on_startup().expect("startup cleanup"),
            0
        );
        assert!(codex_home
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(fs::read_to_string(codex_home.join("config.toml"))
            .expect("read provider config")
            .contains("model_catalog_json"));
    }
