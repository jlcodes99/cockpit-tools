// Codex Local Access 测试：Takeover reconciliation, gateway configuration and remaining integration cases。
// 测试与生产实现共享 super 作用域，验证真实网关、持久化和请求协议行为。
    #[tokio::test]
    async fn local_access_takeover_writes_a_complete_model_catalog() {
        let profile_dir = make_temp_dir("codex-local-access-model-catalog-test");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None)
            .await
            .expect("write local access takeover");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("requires_openai_auth = false"));
        assert!(config.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(config.contains(CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));
        assert!(config.contains(CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE));
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE
        )));
        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE))
                .expect("read local access model catalog"),
        )
        .expect("parse local access model catalog");
        let spark = catalog
            .get("models")
            .and_then(Value::as_array)
            .and_then(|models| {
                models.iter().find(|model| {
                    model.get("slug").and_then(Value::as_str) == Some("gpt-5.3-codex-spark")
                })
            })
            .expect("Spark should be present in the local access model catalog");
        assert_eq!(
            spark.get("display_name").and_then(Value::as_str),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(
            spark.get("prefer_websockets").and_then(Value::as_bool),
            Some(false)
        );
        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn local_access_takeover_preserves_enabled_model_catalog() {
        let profile_dir = make_temp_dir("codex-local-access-model-catalog-test");
        fs::write(
            profile_dir.join(".cockpit-experimental-model-catalog-enabled"),
            "enabled\n",
        )
        .expect("write model catalog policy marker");
        fs::write(
            profile_dir.join(".cockpit-experimental-model-catalog-config.json"),
            serde_json::to_string_pretty(&json!({
                "version": 4,
                "models": [{
                    "model_id": CODEX_TEST_MODEL_ID,
                    "display_name": CODEX_TEST_MODEL_ID
                }]
            }))
            .expect("serialize model catalog definitions"),
        )
        .expect("write model catalog definitions");
        fs::write(
            profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE),
            serde_json::to_string_pretty(&json!({
                "models": [{ "slug": CODEX_TEST_MODEL_ID }]
            }))
            .expect("serialize initial model catalog"),
        )
        .expect("write initial model catalog");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            format!(
                "model_catalog_json = \"{}\"\nmodel = \"{}\"\n",
                CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE, CODEX_TEST_MODEL_ID
            ),
        )
        .expect("write initial model config");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None)
            .await
            .expect("write local access takeover");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains(&format!(
            "model_catalog_json = \"{}\"",
            CODEX_PROVIDER_MODEL_CATALOG_FILE
        )));
        assert!(!config.contains("model = "));
        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join(CODEX_PROVIDER_MODEL_CATALOG_FILE))
                .expect("read model catalog"),
        )
        .expect("parse model catalog");
        let model = catalog
            .get("models")
            .and_then(Value::as_array)
            .and_then(|models| {
                models.iter().find(|model| {
                    model.get("slug").and_then(Value::as_str) == Some(CODEX_TEST_MODEL_ID)
                })
            })
            .expect("model should be present in the managed catalog");
        assert_eq!(
            model.get("display_name").and_then(Value::as_str),
            Some(CODEX_TEST_MODEL_ID)
        );
        assert_eq!(
            model.get("prefer_websockets").and_then(Value::as_bool),
            Some(false)
        );
        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn provider_gateway_takeover_disables_websockets_in_profile() {
        let profile_dir = make_temp_dir("codex-provider-gateway-websocket-test");
        let mut collection = test_local_access_collection(Vec::new());
        let key = "deepseek-local-key".to_string();
        collection.api_keys.push(CodexLocalAccessApiKey {
            id: "provider_gateway_deepseek".to_string(),
            label: "Provider Gateway: DeepSeek".to_string(),
            key: key.clone(),
            provider_gateway: Some(CodexLocalAccessProviderGateway {
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
            account_ids: vec!["deepseek-account".to_string()],
            priority_account_ids: Vec::new(),
            preferred_account_id: None,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            token_limit: None,
            token_used: 0,
            enabled: true,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
        });

        write_local_access_profile_takeover(&profile_dir, &collection, Some(&key))
            .await
            .expect("write provider gateway takeover");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("supports_websockets = false"));
        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn stale_profile_websocket_capabilities_trigger_reconciliation() {
        let profile_dir = make_temp_dir("codex-local-access-stale-websocket-test");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None)
            .await
            .expect("write local access takeover");
        assert!(!super::local_access_profile_takeover_needs_websocket_sync(
            &profile_dir,
            &collection
        ));

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let config = fs::read_to_string(&config_path).expect("read config");
        fs::write(
            &config_path,
            config.replace("supports_websockets = false", "supports_websockets = true"),
        )
        .expect("write stale config");

        let catalog_path = profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE);
        let mut catalog: Value =
            serde_json::from_str(&fs::read_to_string(&catalog_path).expect("read model catalog"))
                .expect("parse model catalog");
        for model in catalog
            .get_mut("models")
            .and_then(Value::as_array_mut)
            .expect("model catalog array")
        {
            model["prefer_websockets"] = json!(true);
        }
        fs::write(
            &catalog_path,
            serde_json::to_string_pretty(&catalog).expect("serialize model catalog"),
        )
        .expect("write stale model catalog");

        assert!(super::local_access_profile_takeover_needs_websocket_sync(
            &profile_dir,
            &collection
        ));
        super::ensure_profile_takeover(&profile_dir, &collection)
            .await
            .expect("reconcile stale local access takeover");
        assert!(!super::local_access_profile_takeover_needs_websocket_sync(
            &profile_dir,
            &collection
        ));

        let repaired_config = fs::read_to_string(&config_path).expect("read repaired config");
        assert!(repaired_config.contains("supports_websockets = false"));
        let repaired_catalog: Value = serde_json::from_str(
            &fs::read_to_string(&catalog_path).expect("read repaired model catalog"),
        )
        .expect("parse repaired model catalog");
        assert!(repaired_catalog
            .get("models")
            .and_then(Value::as_array)
            .is_some_and(|models| {
                !models.is_empty()
                    && models.iter().all(|model| {
                        model.get("prefer_websockets").and_then(Value::as_bool) == Some(false)
                    })
            }));

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[test]
    fn model_provider_chat_test_collection_uses_images_only() {
        let request = model_provider_chat_test_request("responses");
        let account = CodexAccount::new_api_key(
            "api-test-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["upstream-model".to_string()],
        );
        let direct_collection = build_model_provider_gateway_test_collection(
            &request,
            &account,
            None,
            &model_provider_direct_test_client_model(),
        )
        .expect("direct collection should build");

        assert_eq!(
            direct_collection.image_generation_mode,
            CodexLocalAccessImageGenerationMode::Enabled
        );

        let provider_gateway = CodexLocalAccessProviderGateway {
            base_url: "https://relay.example/v1".to_string(),
            api_key: "sk-test".to_string(),
            upstream_model: "upstream-model".to_string(),
            upstream_models: vec!["upstream-model".to_string()],
            wire_api: Some("chat_completions".to_string()),
            supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        };
        let chat_request = model_provider_chat_test_request("chat_completions");
        let chat_collection = build_model_provider_gateway_test_collection(
            &chat_request,
            &account,
            Some(provider_gateway),
            "upstream-model",
        )
        .expect("chat collection should build");

        assert_eq!(
            chat_collection.image_generation_mode,
            CodexLocalAccessImageGenerationMode::Enabled
        );
    }

    #[tokio::test]
    async fn sidecar_config_disables_chat_image_generation_for_bound_oauth_api_key_pool() {
        let dir = make_temp_dir("codex-sidecar-bound-oauth-image-generation");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;

        let collection = test_local_access_collection(vec![account.id.clone()]);
        let launch_config = prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::from([(account.id.clone(), account)]),
        )
        .await
        .expect("sidecar config should build");
        let config: Value = serde_json::from_str(
            &fs::read_to_string(&launch_config.config_path).expect("read sidecar config"),
        )
        .expect("parse sidecar config");

        assert_eq!(config.get("disable-auth-auto-refresh"), Some(&json!(true)));

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn sidecar_config_disables_chat_image_generation_for_oauth_pool() {
        let dir = make_temp_dir("codex-sidecar-oauth-image-generation");
        let account = CodexAccount::new(
            "oauth-image-generation-1".to_string(),
            "oauth@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );

        let collection = test_local_access_collection(vec![account.id.clone()]);
        let launch_config = prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::from([(account.id.clone(), account)]),
        )
        .await
        .expect("sidecar config should build");
        let _config: Value = serde_json::from_str(
            &fs::read_to_string(&launch_config.config_path).expect("read sidecar config"),
        )
        .expect("parse sidecar config");

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn sidecar_config_uses_streaming_bootstrap_retry_setting() {
        let dir = make_temp_dir("codex-sidecar-streaming-bootstrap-retries");
        let account = CodexAccount::new(
            "oauth-streaming-retries-1".to_string(),
            "oauth@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        let mut collection = test_local_access_collection(vec![account.id.clone()]);
        collection.timeouts.single_account_status_retry_attempts = 4;
        collection.timeouts.sidecar_streaming_bootstrap_retries = 2;

        let launch_config = prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::from([(account.id.clone(), account)]),
        )
        .await
        .expect("sidecar config should build");
        let config: Value = serde_json::from_str(
            &fs::read_to_string(&launch_config.config_path).expect("read sidecar config"),
        )
        .expect("parse sidecar config");

        assert_eq!(
            config
                .get("streaming")
                .and_then(|streaming| streaming.get("bootstrap-retries")),
            Some(&json!(2))
        );
        assert_eq!(
            config
                .get("codex")
                .and_then(|codex| codex.get("optimize-multi-agent-v2")),
            Some(&json!(true))
        );

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn bound_oauth_local_gateway_config_uses_direct_codex_api_key_scope() {
        let dir = make_temp_dir("codex-sidecar-bound-oauth-direct-scope");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-direct-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;
        let expected_auth_id = sidecar_codex_api_key_auth_id(&account).expect("auth id");

        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-profile-key".to_string();
        collection.image_generation_mode = provider_gateway_image_generation_mode_for_account(
            &account,
            collection.image_generation_mode,
        );
        collection.bound_oauth_account_id =
            provider_gateway_bound_oauth_account_id_for_account(&account);
        let mut api_key = build_local_access_api_key(Some("Bound OAuth Local Gateway"));
        api_key.key = collection.api_key.clone();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![account.id.clone()];
        collection.api_keys = vec![api_key];

        let launch_config = prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::from([(account.id.clone(), account)]),
        )
        .await
        .expect("sidecar config should build");
        let config: Value = serde_json::from_str(
            &fs::read_to_string(&launch_config.config_path).expect("read sidecar config"),
        )
        .expect("parse sidecar config");
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(&launch_config.manifest_path).expect("read sidecar manifest"),
        )
        .expect("parse sidecar manifest");

        assert_eq!(
            config
                .get("api-key-account-ids")
                .and_then(|value| value.get("local-profile-key"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_str),
            Some(expected_auth_id.as_str())
        );
        assert_eq!(
            config
                .get("codex-api-key")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("base-url"))
                .and_then(Value::as_str),
            Some("https://relay.example/v1")
        );
        assert!(
            manifest
                .get("apiKeys")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("providerGateway"))
                .map(Value::is_null)
                .unwrap_or(true),
            "Responses bound OAuth local gateway should not use providerGateway"
        );

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn treats_collection_client_url_as_local_gateway_not_upstream() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 53549;
        collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::Localhost;
        assert!(is_local_access_gateway_base_url(
            "http://localhost:53549/v1",
            &collection
        ));
        assert!(is_local_access_gateway_base_url(
            "http://127.0.0.1:53549/v1",
            &collection
        ));
        assert!(!is_local_access_gateway_base_url(
            "https://relay.example/v1",
            &collection
        ));
        assert!(!is_local_access_gateway_base_url(
            "http://127.0.0.1:11434/v1",
            &collection
        ));
    }

    #[test]
    fn resolves_sidecar_upstream_from_model_provider_when_account_holds_gateway_url() {
        let data_dir = make_temp_dir("codex-sidecar-upstream-providers");
        fs::write(
            data_dir.join("codex_model_providers.json"),
            r#"[{"id":"relay","name":"Relay","baseUrl":"https://relay.example/v1"}]"#,
        )
        .expect("write providers");

        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 53549;
        let account = CodexAccount::new_api_key(
            "api-polluted-1".to_string(),
            "polluted@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:53549/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![],
        );

        assert_eq!(
            lookup_codex_model_provider_base_url_in_dir(&data_dir, Some("relay"), None).as_deref(),
            Some("https://relay.example/v1")
        );
        // Avoid mutating process-global COCKPIT_TOOLS_DATA_DIR (races other tests).
        let resolved = resolve_sidecar_upstream_base_url_with(&account, &collection, |id, name| {
            lookup_codex_model_provider_base_url_in_dir(&data_dir, id, name)
        });
        assert_eq!(resolved.as_deref(), Some("https://relay.example/v1"));

        // sidecar_codex_key_config_value uses the same resolve rules with production lookup;
        // with a safe recovered URL injected via resolve_with, the written base-url matches.
        // When account already has a non-gateway URL, config writes it directly:
        let direct = CodexAccount::new_api_key(
            "api-direct-1".to_string(),
            "direct@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![],
        );
        let config = sidecar_codex_key_config_value(&direct, &collection, None)
            .expect("sidecar key for real upstream");
        assert_eq!(
            config.get("base-url").and_then(Value::as_str),
            Some("https://relay.example/v1")
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidecar_codex_key_uses_account_model_mappings() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-map-1".to_string(),
            "deepseek@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
            client_model: "gpt-5.6-sol".to_string(),
            upstream_model: "deepseek-v4-flash".to_string(),
        }];
        let collection = test_local_access_collection(vec![account.id.clone()]);
        let config =
            sidecar_codex_key_config_value(&account, &collection, None).expect("sidecar key");
        let models = config
            .get("models")
            .and_then(Value::as_array)
            .expect("models");
        assert_eq!(models.len(), 1);
        assert_eq!(
            models[0].get("alias").and_then(Value::as_str),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            models[0].get("name").and_then(Value::as_str),
            Some("deepseek-v4-flash")
        );
        let excluded = config
            .get("excluded-models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(excluded.iter().any(|item| item.as_str() == Some("gpt-5.4")));
        assert!(!excluded
            .iter()
            .any(|item| item.as_str() == Some("gpt-5.6-sol")));
    }

    #[test]
    fn skips_sidecar_key_when_gateway_url_cannot_be_recovered() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 53549;
        let account = CodexAccount::new_api_key(
            "api-polluted-2".to_string(),
            "polluted2@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:53549/v1".to_string()),
            Some("missing-provider".to_string()),
            Some("Missing".to_string()),
            vec![],
        );
        assert!(resolve_sidecar_upstream_base_url(&account, &collection).is_none());
        assert!(sidecar_codex_key_config_value(&account, &collection, None).is_none());
    }

    #[test]
    fn sidecar_codex_key_skips_same_port_gateway_loopback_without_provider_recovery() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 53549;
        let account = CodexAccount::new_api_key(
            "api-loopback-self-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:53549/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        // Same port as the running API Service ⇒ self-reference, still rejected.
        assert!(resolve_sidecar_upstream_base_url(&account, &collection).is_none());
        assert!(sidecar_codex_key_config_value(&account, &collection, None).is_none());
    }

    #[test]
    fn sidecar_codex_key_allows_loopback_upstream_on_different_port() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 63266;
        let account = CodexAccount::new_api_key(
            "api-loopback-other-1".to_string(),
            "local-upstream@example.com".to_string(),
            "sk-local-upstream".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:8317/v1".to_string()),
            Some("local-compat".to_string()),
            Some("Local Compat".to_string()),
            Vec::new(),
        );
        assert_eq!(
            resolve_sidecar_upstream_base_url(&account, &collection).as_deref(),
            Some("http://127.0.0.1:8317/v1")
        );
        let config = sidecar_codex_key_config_value(&account, &collection, None)
            .expect("different-port loopback upstream should be accepted");
        assert_eq!(
            config.get("base-url").and_then(Value::as_str),
            Some("http://127.0.0.1:8317/v1")
        );
        assert_eq!(
            config.get("api-key").and_then(Value::as_str),
            Some("sk-local-upstream")
        );
    }

    #[test]
    fn sidecar_codex_key_syncs_account_supports_websockets() {
        let collection = test_local_access_collection(Vec::new());
        let mut account = CodexAccount::new_api_key(
            "api-ws-1".to_string(),
            "ws-upstream@example.com".to_string(),
            "sk-ws-upstream".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://sub2api.example/v1".to_string()),
            Some("sub2api".to_string()),
            Some("Sub2API".to_string()),
            Vec::new(),
        );

        // Default API Key accounts do not advertise upstream WebSocket.
        let disabled =
            sidecar_codex_key_config_value(&account, &collection, None).expect("api key config");
        assert_eq!(
            disabled.get("websockets").and_then(Value::as_bool),
            Some(false),
            "missing supportsWebsockets must serialize as websockets=false so cliproxy stays on HTTP"
        );

        // Provider supportsWebsockets=true must flow into codex-api-key.websockets so the
        // second hop (Cockpit → OpenAI-compatible upstream) can keep Responses WebSocket.
        account.api_supports_websockets = true;
        let enabled = sidecar_codex_key_config_value(&account, &collection, None)
            .expect("api key config with websockets");
        assert_eq!(
            enabled.get("websockets").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            enabled.get("base-url").and_then(Value::as_str),
            Some("https://sub2api.example/v1")
        );
    }

    #[test]
    fn sidecar_codex_key_allows_localhost_upstream_on_different_port() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 63266;
        let account = CodexAccount::new_api_key(
            "api-loopback-other-2".to_string(),
            "local-upstream2@example.com".to_string(),
            "sk-local-upstream-2".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:8317/v1".to_string()),
            None,
            None,
            Vec::new(),
        );
        assert_eq!(
            resolve_sidecar_upstream_base_url(&account, &collection).as_deref(),
            Some("http://localhost:8317/v1")
        );
        let config = sidecar_codex_key_config_value(&account, &collection, None)
            .expect("localhost different-port upstream should be accepted");
        assert_eq!(
            config.get("base-url").and_then(Value::as_str),
            Some("http://localhost:8317/v1")
        );
    }

    #[test]
    fn resolves_sidecar_upstream_from_provider_when_account_holds_different_port_loopback() {
        let data_dir = make_temp_dir("codex-sidecar-loopback-provider");
        fs::write(
            data_dir.join("codex_model_providers.json"),
            r#"[{"id":"local","name":"Local","baseUrl":"http://127.0.0.1:8317/v1"}]"#,
        )
        .expect("write providers");

        let mut collection = test_local_access_collection(Vec::new());
        collection.port = 63266;
        // Account base is polluted to the gateway; provider still has the real local upstream.
        let account = CodexAccount::new_api_key(
            "api-polluted-loopback".to_string(),
            "polluted-local@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:63266/v1".to_string()),
            Some("local".to_string()),
            Some("Local".to_string()),
            vec![],
        );

        let resolved = resolve_sidecar_upstream_base_url_with(&account, &collection, |id, name| {
            lookup_codex_model_provider_base_url_in_dir(&data_dir, id, name)
        });
        assert_eq!(resolved.as_deref(), Some("http://127.0.0.1:8317/v1"));
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidecar_api_key_scope_uses_account_overrides_for_temporary_api_key() {
        let account = CodexAccount::new_api_key(
            "api-override-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["upstream-model".to_string()],
        );
        let expected_auth_id = sidecar_codex_api_key_auth_id(&account).expect("auth id");
        let mut collection = test_local_access_collection(Vec::new());
        collection.account_ids.clear();
        collection.api_keys.clear();
        let mut api_key = build_local_access_api_key(Some("Temporary"));
        api_key.key = "local-test-key".to_string();
        api_key.inherit_account_pool = Some(false);
        api_key.account_ids = vec![account.id.clone()];
        collection.api_keys.push(api_key);

        let scopes = sidecar_api_key_account_scope_values(
            &collection,
            &HashMap::from([(account.id.clone(), account)]),
        );
        let actual = scopes
            .get("local-test-key")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        assert_eq!(actual, vec![expected_auth_id]);
    }

    #[test]
    fn provider_gateway_inherits_api_key_bound_oauth_account() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());
        account.bound_oauth_account_id = Some(" oauth-1 ".to_string());

        assert_eq!(
            provider_gateway_bound_oauth_account_id_for_account(&account).as_deref(),
            Some("oauth-1")
        );
    }

    #[test]
    fn provider_gateway_oauth_binding_keeps_image_generation_enabled() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.4".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;

        assert_eq!(
            provider_gateway_image_generation_mode_for_account(
                &account,
                CodexLocalAccessImageGenerationMode::Enabled,
            ),
            CodexLocalAccessImageGenerationMode::Enabled
        );

        assert_eq!(
            provider_gateway_image_generation_mode_for_account(
                &account,
                CodexLocalAccessImageGenerationMode::Disabled,
            ),
            CodexLocalAccessImageGenerationMode::Enabled
        );

        account.bound_oauth_use_local_gateway = false;
        assert_eq!(
            provider_gateway_image_generation_mode_for_account(
                &account,
                CodexLocalAccessImageGenerationMode::Disabled,
            ),
            CodexLocalAccessImageGenerationMode::Enabled
        );
    }

    #[test]
    fn sanitize_collection_enables_session_affinity_once_for_existing_config() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.session_affinity = false;
        collection.session_affinity_default_enabled_migrated = false;

        let (changed, _) = sanitize_collection_with_accounts(&mut collection, &[])
            .expect("collection should sanitize");

        assert!(changed);
        assert!(collection.session_affinity);
        assert!(collection.session_affinity_default_enabled_migrated);
    }

    #[test]
    fn sanitize_collection_respects_session_affinity_disabled_after_migration() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.session_affinity = false;
        collection.session_affinity_default_enabled_migrated = true;

        let (_changed, _) = sanitize_collection_with_accounts(&mut collection, &[])
            .expect("collection should sanitize");

        assert!(!collection.session_affinity);
        assert!(collection.session_affinity_default_enabled_migrated);
    }

    #[test]
    fn sanitize_collection_migrates_legacy_image_generation_mode_to_enabled() {
        for mode in [
            CodexLocalAccessImageGenerationMode::ImagesOnly,
            CodexLocalAccessImageGenerationMode::Disabled,
        ] {
            let mut collection = test_local_access_collection(Vec::new());
            collection.image_generation_mode = mode;

            let (changed, _) = sanitize_collection_with_accounts(&mut collection, &[])
                .expect("collection should sanitize");

            assert!(
                changed,
                "legacy image generation mode {mode:?} should be migrated"
            );
            assert_eq!(
                collection.image_generation_mode,
                CodexLocalAccessImageGenerationMode::Enabled
            );
        }
    }

    #[test]
    fn sanitize_collection_migrates_legacy_gateway_to_sidecar() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.gateway_mode = CodexLocalAccessGatewayMode::Legacy;

        let (changed, _) = sanitize_collection_with_accounts(&mut collection, &[])
            .expect("collection should sanitize");

        assert!(changed);
        assert_eq!(
            collection.gateway_mode,
            CodexLocalAccessGatewayMode::Sidecar
        );
    }

    #[test]
    fn legacy_disabled_image_mode_no_longer_blocks_image_capacity_after_sanitize() {
        let mut paid = test_account_with_plan("plus");
        paid.id = "oauth-plus".to_string();
        let accounts = vec![paid.clone()];

        let mut collection = test_local_access_collection(vec![paid.id.clone()]);
        collection.image_generation_mode = CodexLocalAccessImageGenerationMode::Disabled;

        assert!(
            !selected_account_ids_have_image_generation_capacity(
                &collection.account_ids,
                collection.image_generation_mode,
                Some(accounts.as_slice()),
                None,
            ),
            "disabled mode should hide image capacity before migration"
        );

        let (changed, _) = sanitize_collection_with_accounts(&mut collection, &accounts)
            .expect("collection should sanitize");
        assert!(changed);
        assert_eq!(
            collection.image_generation_mode,
            CodexLocalAccessImageGenerationMode::Enabled
        );
        assert!(
            selected_account_ids_have_image_generation_capacity(
                &collection.account_ids,
                collection.image_generation_mode,
                Some(accounts.as_slice()),
                None,
            ),
            "plus OAuth pool should expose image capacity after migration"
        );
    }

    #[test]
    fn sanitize_collection_keeps_provider_gateway_account_scope() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());
        let account_id = account.id.clone();

        let mut collection = test_local_access_collection(vec![account_id.clone()]);
        let mut api_key = build_local_access_api_key(Some("Provider Gateway"));
        api_key.inherit_account_pool = Some(false);
        api_key.provider_gateway = Some(CodexLocalAccessProviderGateway {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
            upstream_models: vec!["deepseek-v4-pro".to_string()],
            wire_api: Some("chat_completions".to_string()),
            supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        });
        api_key.account_ids = vec![account_id.clone()];
        collection.api_keys = vec![api_key];

        sanitize_collection_with_accounts(&mut collection, &[account])
            .expect("collection should sanitize");

        assert!(collection.account_ids.is_empty());
        assert_eq!(collection.api_keys.len(), 1);
        assert_eq!(collection.api_keys[0].account_ids, vec![account_id]);
    }

    #[test]
    fn sanitize_collection_keeps_provider_gateway_bound_oauth_account() {
        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("chat_completions".to_string());
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        let account_id = account.id.clone();
        let oauth_account = CodexAccount::new(
            "oauth-1".to_string(),
            "oauth@example.com".to_string(),
            CodexTokens {
                id_token: "id-token".to_string(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );

        let mut collection = test_local_access_collection(vec![account_id.clone()]);
        collection.bound_oauth_account_id =
            provider_gateway_bound_oauth_account_id_for_account(&account);
        let mut api_key = build_local_access_api_key(Some("Provider Gateway"));
        api_key.inherit_account_pool = Some(false);
        api_key.provider_gateway = Some(CodexLocalAccessProviderGateway {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: "sk-test".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
            upstream_models: vec!["deepseek-v4-pro".to_string()],
            wire_api: Some("chat_completions".to_string()),
            supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        });
        api_key.account_ids = vec![account_id.clone()];
        collection.api_keys = vec![api_key];

        sanitize_collection_with_accounts(&mut collection, &[account, oauth_account])
            .expect("collection should sanitize");

        assert_eq!(
            collection.bound_oauth_account_id.as_deref(),
            Some("oauth-1")
        );
        assert!(collection.account_ids.is_empty());
        assert_eq!(collection.api_keys.len(), 1);
        assert_eq!(collection.api_keys[0].account_ids, vec![account_id]);
    }

    #[test]
    fn sanitize_collection_removes_agent_identity_oauth_binding() {
        let mut agent_identity = test_account_with_plan("plus");
        agent_identity.id = "agent-identity-binding".to_string();
        agent_identity.tokens.refresh_token = Some("refresh-token".to_string());
        agent_identity.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-binding".to_string(),
            agent_private_key: "private-key".to_string(),
            task_id: Some("task-binding".to_string()),
            account_id: "account-binding".to_string(),
            chatgpt_user_id: "user-binding".to_string(),
            email: Some("agent-binding@example.com".to_string()),
            plan_type: Some("plus".to_string()),
            chatgpt_account_is_fedramp: false,
        });

        let mut collection = test_local_access_collection(vec![agent_identity.id.clone()]);
        collection.bound_oauth_account_id = Some(agent_identity.id.clone());

        let (changed, valid_account_ids) =
            sanitize_collection_with_accounts(&mut collection, &[agent_identity.clone()])
                .expect("collection should sanitize");

        assert!(changed);
        assert!(collection.bound_oauth_account_id.is_none());
        assert!(valid_account_ids.contains(&agent_identity.id));
        assert!(collection.account_ids.contains(&agent_identity.id));
    }

    #[test]
    fn builds_upstream_websocket_url_from_custom_base_url() {
        let https_account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        let http_account = CodexAccount::new_api_key(
            "api-2".to_string(),
            "local@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:8080/v1".to_string()),
            Some("local".to_string()),
            Some("Local".to_string()),
            Vec::new(),
        );

        assert_eq!(
            build_upstream_websocket_url(&https_account, "/responses").unwrap(),
            "wss://relay.example/v1/responses"
        );
        assert_eq!(
            build_upstream_websocket_url(&http_account, "/responses").unwrap(),
            "ws://127.0.0.1:8080/v1/responses"
        );
    }

    #[test]
    fn request_log_time_bounds_accept_unix_seconds_and_millis() {
        assert_eq!(
            super::normalize_request_log_time_bound(1_800_000_000),
            1_800_000_000_000
        );
        assert_eq!(
            super::normalize_request_log_time_bound(1_800_000_000_000),
            1_800_000_000_000
        );
        assert_eq!(super::normalize_request_log_time_bound(0), 0);
    }
