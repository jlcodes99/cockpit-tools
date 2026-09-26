// Codex Local Access 测试：Takeover reconciliation, gateway configuration and remaining integration cases。
// 测试与生产实现共享 super 作用域，验证真实网关、持久化和请求协议行为。
    fn realtime_mixed_test_collection() -> CodexLocalAccessCollection {
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "mixed-realtime-test-key".to_string();
        let mut key = build_local_access_api_key(Some("Mixed realtime"));
        key.key = collection.api_key.clone();
        key.model_routing = Some(CodexLocalAccessModelRouting {
            default_route: "oauth".to_string(),
            failure_policy: "strict".to_string(),
            routes: Vec::new(),
        });
        collection.api_keys = vec![key];
        collection
    }

    #[tokio::test]
    async fn mixed_realtime_takeover_preserves_bound_oauth_and_local_bearer() {
        let _lock = crate::modules::test_support::env_lock()
            .lock().unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("mixed-realtime-bound-oauth");
        let profile_dir = make_temp_dir("mixed-realtime-bound-profile");
        let tokens = CodexTokens {
            id_token: make_test_jwt(json!({
                "email": "voice@example.test", "exp": 4_102_444_800i64,
                "https://api.openai.com/auth": { "chatgpt_account_id": "voice-test-account" }
            })),
            access_token: make_test_jwt(json!({"sub": "voice-test", "exp": 4_102_444_800i64})),
            refresh_token: Some("voice-test-refresh".to_string()),
        };
        let account = CodexAccount::new(
            "voice-oauth-test".to_string(), "voice@example.test".to_string(), tokens
        );
        crate::modules::codex_account::save_account(&account).expect("save OAuth fixture");
        let mut collection = realtime_mixed_test_collection();
        collection.bound_oauth_account_id = Some(account.id.clone());
        collection.account_ids = vec![account.id.clone()];
        collection.api_keys[0].account_ids = vec![account.id.clone()];
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await.expect("write bound mixed takeover");
        let auth: Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join(CODEX_PROFILE_AUTH_FILE)).expect("read auth")
        ).expect("parse auth");
        assert_eq!(auth.pointer("/tokens/id_token").and_then(Value::as_str), Some(account.tokens.id_token.as_str()));
        assert_eq!(auth.pointer("/tokens/refresh_token").and_then(Value::as_str), Some("voice-test-refresh"));
        let config = fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        let doc = config.parse::<Document>().expect("parse config");
        assert_eq!(doc["model_providers"]["codex_local_access"]["requires_openai_auth"].as_bool(), Some(true));
        assert_eq!(doc["model_providers"]["codex_local_access"]["experimental_bearer_token"].as_str(), Some(collection.api_key.as_str()));
        assert_eq!(doc["experimental_realtime_ws_base_url"].as_str(), Some(build_collection_base_url(&collection).as_str()));
        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    #[tokio::test]
    async fn mixed_realtime_takeover_routes_sideband_without_enabling_responses_websocket() {
        let profile_dir = make_temp_dir("mixed-realtime-sideband");
        let collection = realtime_mixed_test_collection();
        let original = "model_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\n";
        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original).expect("write original");
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write mixed takeover");
        let config = fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        let doc = config.parse::<Document>().expect("parse config");
        assert_eq!(
            doc["experimental_realtime_ws_base_url"].as_str(),
            Some(build_collection_base_url(&collection).as_str())
        );
        assert_eq!(doc["model_providers"]["codex_local_access"]["supports_websockets"].as_bool(), Some(false));
        assert_eq!(doc["model_context_window"].as_integer(), Some(1_000_000));

        let restored = restore_config_toml_from_takeover_backup(Some(&config), Some(original))
            .expect("restore config").expect("config exists");
        let restored = restored.parse::<Document>().expect("parse restored");
        assert!(restored.get("experimental_realtime_ws_base_url").is_none());
        assert_eq!(restored["model_auto_compact_token_limit"].as_integer(), Some(900_000));
        let cleaned = remove_codex_local_access_config(&config).expect("cleanup without backup");
        assert!(!cleaned.contains("experimental_realtime_ws_base_url"));
        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    #[tokio::test]
    async fn mixed_realtime_takeover_preserves_explicit_user_sideband_override() {
        let profile_dir = make_temp_dir("mixed-realtime-user-override");
        let collection = realtime_mixed_test_collection();
        let original = "experimental_realtime_ws_base_url = \"https://voice.example.test/v1\"\n";
        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original).expect("write original");
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await.expect("write mixed takeover");
        let config = fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert_eq!(
            config.parse::<Document>().expect("parse")["experimental_realtime_ws_base_url"].as_str(),
            Some("https://voice.example.test/v1")
        );
        let cleaned = remove_codex_local_access_config(&config).expect("cleanup");
        assert!(cleaned.contains("https://voice.example.test/v1"));
        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    #[test]
    fn mixed_realtime_cleanup_restores_backup_and_preserves_later_user_edits() {
        let base = "http://localhost:14998/v1";
        let config = format!(
            "model_provider = \"codex_local_access\"\nexperimental_realtime_ws_base_url = \"{base}\"\n\
             [model_providers.codex_local_access]\nbase_url = \"{base}\"\n"
        );
        let original = "experimental_realtime_ws_base_url = \"https://original.example.test/v1\"\n";
        let restored = restore_config_toml_from_takeover_backup(Some(&config), Some(original))
            .expect("restore").expect("config");
        assert_eq!(
            restored.parse::<Document>().expect("parse")["experimental_realtime_ws_base_url"].as_str(),
            Some("https://original.example.test/v1")
        );
        let mut edited = config.parse::<Document>().expect("parse config");
        edited["experimental_realtime_ws_base_url"] = value("https://edited.example.test/v1");
        let restored = restore_config_toml_from_takeover_backup(Some(&edited.to_string()), Some(original))
            .expect("restore edited").expect("config");
        assert!(restored.contains("https://edited.example.test/v1"));
    }

    #[tokio::test]
    async fn takeover_cleanup_restores_managed_local_compaction_fallback() {
        let profile_dir = make_temp_dir("local-compaction-takeover-restore");
        // 接管前用户没有任何压缩相关设置。
        let original = "model = \"gpt-6-astra\"\n\n[features]\njs_repl = false\n";
        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original).expect("write original");

        let mut collection = test_local_access_collection(Vec::new());
        collection.enabled = true;
        let mut account_key = build_local_access_api_key(Some("DeepSeek pool"));
        account_key.id = "provider_gateway_deepseek".to_string();
        account_key.provider_gateway = Some(CodexLocalAccessProviderGateway {
            base_url: "https://api.deepseek.com/v1".to_string(),
            api_key: "sk-deepseek".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
            upstream_models: vec!["deepseek-v4-pro".to_string()],
            wire_api: Some("chat_completions".to_string()),
            supports_vision: false,
            model_capabilities: HashMap::new(),
            vision_routing_model: None,
        });
        collection.api_keys = vec![account_key];
        collection.api_key = collection.api_keys[0].key.clone();
        collection.port = 15_991;

        super::write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write takeover");
        let config = fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE))
            .expect("read config");
        assert!(config.contains("remote_compaction_v2 = false"));
        // 只有远端压缩被关闭；`token_budget` 会把压缩换成不产摘要的窗口重置，不能写入。
        assert!(!config.contains("token_budget"));

        let restored = restore_config_toml_from_takeover_backup(Some(&config), Some(original))
            .expect("restore")
            .expect("config exists");
        assert!(!restored.contains("remote_compaction_v2"));
        assert!(!restored.contains("token_budget"));
        assert!(restored.contains("js_repl = false"));

        // 用户原本就设过这两个键时按原值还原。
        let user_config =
            "model = \"gpt-6-astra\"\n\n[features]\nremote_compaction_v2 = true\ntoken_budget = false\n";
        let restored = restore_config_toml_from_takeover_backup(Some(&config), Some(user_config))
            .expect("restore")
            .expect("config exists");
        assert!(restored.contains("remote_compaction_v2 = true"));
        assert!(restored.contains("token_budget = false"));

        // 接管前的配置里没有 profile 时不应凭空生成 features 段。
        let no_backup = restore_config_toml_from_takeover_backup(Some(&config), None)
            .expect("restore without backup")
            .expect("config exists");
        assert!(!no_backup.contains("remote_compaction_v2"));
        assert!(!no_backup.contains("token_budget"));

        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    #[test]
    fn mixed_realtime_override_ignores_non_mixed_or_wrong_keys() {
        let profile_dir = make_temp_dir("mixed-realtime-key-scope");
        let mut collection = realtime_mixed_test_collection();
        let original = "model = \"gpt-6-astra\"\n";
        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original).expect("write original");
        super::write_mixed_model_realtime_sideband_override(&profile_dir, &collection, "different-key")
            .expect("wrong key no-op");
        collection.api_keys[0].model_routing = None;
        super::write_mixed_model_realtime_sideband_override(&profile_dir, &collection, &collection.api_key)
            .expect("non-mixed no-op");
        assert_eq!(fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read"), original);
        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    #[tokio::test]
    async fn sidecar_uses_upstream_policy_with_api_service_bootstrap_buffering() {
        let collection = test_local_access_collection(Vec::new());
        for api_service in [false, true] {
            let dir = make_temp_dir("codex-sidecar-capacity-scope");
            let launch = if api_service {
                super::prepare_sidecar_launch_config_in_dir_sync(
                    &collection,
                    dir.clone(),
                    HashMap::new(),
                    None,
                    HashMap::new(),
                    true,
                    None,
                )
            } else {
                prepare_sidecar_launch_config_in_dir(
                    &collection,
                    dir.clone(),
                    HashMap::new(),
                    None,
                    HashMap::new(),
                )
                .await
            }
            .expect("prepare scoped sidecar config");
            let config: Value = serde_json::from_str(
                &fs::read_to_string(launch.config_path).expect("read config"),
            )
            .expect("parse config");
            assert_eq!(config["codex"]["stream-bootstrap-buffering"], json!(api_service));
            assert_eq!(config["request-retry"], json!(super::MAX_REQUEST_RETRY_ATTEMPTS));
            assert_eq!(config["disable-cooling"], json!(collection.disable_cooling));
            fs::remove_dir_all(dir).expect("cleanup test config");
        }
    }

    #[tokio::test]
    async fn local_access_takeover_writes_a_complete_model_catalog() {
        let profile_dir = make_temp_dir("codex-local-access-model-catalog-test");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write local access takeover");

        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("name = \"Codex API Service\""));
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
        assert!(!config.contains("model_context_window"));
        assert!(!config.contains("model_auto_compact_token_limit"));
        assert!(!config.contains("model = \"gpt-reserve\""));
        // 空账号池没有任何账号能承接官方模型，官方推荐 GPT 集与额度兜底条目
        // （gpt-reserve）都不应进入客户端模型目录，只留客户端内部需要的隐藏条目。
        let catalog_models = catalog["models"].as_array().expect("catalog models");
        let listed_gpt_slugs = catalog_models
            .iter()
            .filter_map(|model| model.get("slug").and_then(Value::as_str))
            .filter(|slug| slug.starts_with("gpt-") && !slug.starts_with("gpt-image"))
            .collect::<Vec<_>>();
        assert!(
            listed_gpt_slugs.is_empty(),
            "空账号池不应展示任何 GPT 条目: {listed_gpt_slugs:?}"
        );
        for hidden in ["gpt-5.3-codex-spark", "gpt-5.4", "gpt-5.4-mini"] {
            assert!(
                !catalog_models
                    .iter()
                    .any(|model| model["slug"].as_str() == Some(hidden)),
                "历史模型 {hidden} 不应出现在客户端模型目录里"
            );
        }
        assert!(!profile_dir
            .join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
            .exists());
        assert!(!profile_dir
            .join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    fn profile_reasoning_efforts(profile_dir: &std::path::Path) -> Vec<String> {
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        let doc = config.parse::<Document>().expect("parse config");
        doc.get("desktop")
            .and_then(|desktop| desktop.get("enabled-reasoning-efforts"))
            .and_then(|item| item.as_value())
            .and_then(|value| value.as_array())
            .map(|efforts| {
                efforts
                    .iter()
                    .filter_map(|effort| effort.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn local_access_takeover_appends_max_reasoning_effort() {
        let profile_dir = make_temp_dir("codex-local-access-max-effort");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();
        // 用户已自定义推理强度：只补 max，保留原有档位与顺序。
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            "[desktop]\nenabled-reasoning-efforts = [\"low\", \"high\"]\n",
        )
        .expect("write initial config");

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write local access takeover");
        assert_eq!(
            profile_reasoning_efforts(&profile_dir),
            vec!["low", "high", "max"]
        );

        // 幂等：重复接管不会重复追加，也不会改写已存在的档位。
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("repeat local access takeover");
        assert_eq!(
            profile_reasoning_efforts(&profile_dir),
            vec!["low", "high", "max"]
        );

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn local_access_takeover_fills_client_default_reasoning_efforts() {
        let profile_dir = make_temp_dir("codex-local-access-max-effort-default");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write local access takeover");

        assert_eq!(
            profile_reasoning_efforts(&profile_dir),
            vec!["low", "medium", "high", "xhigh", "ultra", "persistent", "max"]
        );

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn provider_gateway_takeover_leaves_reasoning_effort_untouched() {
        let profile_dir = make_temp_dir("codex-provider-gateway-effort-scope");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "provider-gateway-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None, false)
            .await
            .expect("write provider gateway takeover");

        assert!(profile_reasoning_efforts(&profile_dir).is_empty());

        fs::remove_dir_all(&profile_dir).expect("cleanup temp dir");
    }

    #[tokio::test]
    async fn local_access_context_overrides_survive_takeover_and_maintenance() {
        let profile = make_temp_dir("local-access-context-overrides");
        let definitions = vec![
            crate::models::codex::CodexExperimentalModelDefinition {
                model_id: "gpt-5.5".into(),
                display_name: "GPT-5.5".into(),
                reasoning_efforts: None,
                context_window: Some(516_000),
                auto_compact_token_limit: Some(460_000),
            },
            crate::models::codex::CodexExperimentalModelDefinition {
                model_id: "custom-third-party".into(),
                display_name: "Custom".into(),
                reasoning_efforts: None,
                context_window: Some(123_456),
                auto_compact_token_limit: Some(111_111),
            },
        ];
        codex_account::save_model_catalog_for_base_dir_preserving_context(
            &profile, true, definitions.clone(), None,
        ).unwrap();
        let mut collection = realtime_mixed_test_collection();
        collection.enabled = true;
        write_local_access_profile_takeover(&profile, &collection, None, true).await.unwrap();
        for pass in 0..3 {
            if pass > 0 {
                super::maintain_local_access_profile(&profile, &collection).unwrap();
            }
            let catalog: Value = serde_json::from_str(
                &fs::read_to_string(profile.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap(),
            ).unwrap();
            for definition in &definitions {
                let model = catalog["models"].as_array().unwrap().iter()
                    .find(|model| model["slug"] == definition.model_id).unwrap();
                assert_eq!(model["context_window"].as_i64(), definition.context_window);
                assert_eq!(model["max_context_window"].as_i64(), definition.context_window);
                assert_eq!(model["auto_compact_token_limit"].as_i64(), definition.auto_compact_token_limit);
                assert_eq!(model["comp_hash"], "3000");
            }
        }
        // The mixed-route writer shares the final sink, including template defaults
        // for models without an override.
        super::write_local_access_profile_model_catalog_with_definitions(
            &profile, false, Some(vec![
                ("gpt-5.5".into(), "GPT-5.5".into()),
                ("gpt-6-astra".into(), "GPT-6 Astra".into()),
            ]),
        ).unwrap();
        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap(),
        ).unwrap();
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.iter().find(|model| model["slug"] == "gpt-5.5").unwrap()["context_window"], 516_000);
        let defaults = super::codex_protocol::build_codex_client_models_response(&["gpt-6-astra".into()]);
        assert_eq!(models.iter().find(|model| model["slug"] == "gpt-6-astra").unwrap()["context_window"], defaults["models"][0]["context_window"]);
        // Disabling model management must stop applying persisted overrides.
        codex_account::save_model_catalog_for_base_dir_preserving_context(
            &profile, false, Vec::new(), None,
        ).unwrap();
        super::write_local_access_profile_model_catalog_with_definitions(
            &profile, false, Some(vec![("gpt-5.5".into(), "GPT-5.5".into())]),
        ).unwrap();
        let catalog: Value = serde_json::from_str(
            &fs::read_to_string(profile.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap(),
        ).unwrap();
        let defaults = super::codex_protocol::build_codex_client_models_response(&["gpt-5.5".into()]);
        assert_eq!(catalog["models"][0]["context_window"], defaults["models"][0]["context_window"]);
        fs::remove_dir_all(profile).unwrap();
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

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
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

        write_local_access_profile_takeover(&profile_dir, &collection, Some(&key), true)
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

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write local access takeover");
        assert!(!super::local_access_profile_takeover_needs_sync(
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

        assert!(super::local_access_profile_takeover_needs_sync(
            &profile_dir,
            &collection
        ));
        super::ensure_profile_takeover(&profile_dir, &collection)
            .await
            .expect("reconcile stale local access takeover");
        assert!(!super::local_access_profile_takeover_needs_sync(
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

    #[tokio::test]
    async fn legacy_profile_provider_name_triggers_reconciliation() {
        let profile_dir = make_temp_dir("codex-local-access-legacy-provider-name");
        let mut collection = test_local_access_collection(Vec::new());
        collection.api_key = "local-service-key".to_string();

        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write local access takeover");

        let config_path = profile_dir.join(CODEX_PROFILE_CONFIG_FILE);
        let config = fs::read_to_string(&config_path).expect("read config");
        // 历史版本曾把托管 provider 显示名写成 `OpenAI`，那会让客户端误判上游支持远端压缩；
        // 这里模拟这种旧 profile，确认启动自愈会把它改回受管名字。
        assert!(config.contains("name = \"Codex API Service\""));
        fs::write(
            &config_path,
            config.replace("name = \"Codex API Service\"", "name = \"OpenAI\""),
        )
        .expect("write legacy provider name");

        assert!(super::local_access_profile_takeover_needs_sync(
            &profile_dir,
            &collection
        ));
        super::ensure_profile_takeover(&profile_dir, &collection)
            .await
            .expect("reconcile legacy provider name");
        assert!(!super::local_access_profile_takeover_needs_sync(
            &profile_dir,
            &collection
        ));

        let repaired_config = fs::read_to_string(&config_path).expect("read repaired config");
        assert!(repaired_config.contains("name = \"Codex API Service\""));
        assert!(!repaired_config.contains("name = \"OpenAI\""));
        fs::remove_dir_all(profile_dir).expect("cleanup temp dir");
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

    /// API 服务 profile 即使开着「模型管理」，GPT 推荐集也必须用官方命名与顺序。
    #[tokio::test]
    async fn api_service_catalog_keeps_official_gpt_names_with_model_management_enabled() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("api-service-official-gpt-names");
        let profile_dir = make_temp_dir("api-service-official-gpt-names-profile");

        // 打开「模型管理」，并写入一组自定义显示名（旧版本默认的“6 Astra/5.6 Sol”）。
        fs::write(
            profile_dir.join(".cockpit-experimental-model-catalog-enabled"),
            "enabled\n",
        )
        .expect("enable model management");
        fs::write(
            profile_dir.join(".cockpit-experimental-model-catalog-config.json"),
            serde_json::to_string_pretty(&json!({
                "version": 4,
                "migrations": ["add-gpt-6-astra-model"],
                "models": [
                    {"model_id": "gpt-6-astra", "display_name": "6 Astra"},
                    {"model_id": "gpt-6-sol", "display_name": "6 Sol"},
                    {"model_id": "gpt-6-luna", "display_name": "6 Luna"},
                    {"model_id": "gpt-5.6-sol", "display_name": "5.6 Sol"},
                    {"model_id": "gpt-5.6-terra", "display_name": "5.6 Terra"},
                    {"model_id": "gpt-5.6-luna", "display_name": "5.6 Luna"},
                    {"model_id": "gpt-5.5", "display_name": "5.5"}
                ]
            }))
            .expect("serialize custom catalog"),
        )
        .expect("write custom catalog");
        fs::write(
            profile_dir.join(".cockpit-experimental-model-catalog-user-customized"),
            "customized\n",
        )
        .expect("mark user-customized catalog");

        let collection = test_local_access_collection(Vec::new());
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write API service takeover");

        let catalog = read_profile_model_catalog(&profile_dir);
        let names = catalog["models"]
            .as_array()
            .expect("catalog models")
            .iter()
            .filter_map(|model| {
                let object = model.as_object()?;
                Some((
                    object.get("slug")?.as_str()?.to_string(),
                    object.get("display_name")?.as_str()?.to_string(),
                ))
            })
            .collect::<HashMap<_, _>>();
        for (slug, expected_name) in [
            ("gpt-6-astra", "GPT-6 Astra"),
            ("gpt-6-sol", "GPT-6 Sol"),
            ("gpt-6-luna", "GPT-6 Luna"),
            ("gpt-5.6-sol", "GPT-5.6 Sol"),
            ("gpt-5.6-terra", "GPT-5.6 Terra"),
            ("gpt-5.6-luna", "GPT-5.6 Luna"),
            ("gpt-5.5", "GPT-5.5"),
        ] {
            assert_eq!(
                names.get(slug).map(String::as_str),
                Some(expected_name),
                "开启模型管理时 GPT 显示名仍必须带官方前缀: {names:?}"
            );
        }
        assert_eq!(
            names.get("gpt-reserve").map(String::as_str),
            Some("GPT-5.6 Reserve")
        );

        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
    }

    /// DeepSeek 账号在 API 服务里与「DeepSeek 网关模式」一致：只列出账号模型，
    /// 并允许把图片自动转到识图模型，因此走 provider 路由而不是原生账号池。
    #[test]
    fn api_service_deepseek_account_routes_images_to_vision_model() {
        let dir = make_temp_dir("api-service-deepseek-vision-route");
        let mut deepseek = CodexAccount::new_api_key(
            "deepseek-vision-account".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()],
        );
        deepseek.api_wire_api = Some("responses".to_string());
        deepseek.api_model_mappings =
            crate::modules::codex_account::default_deepseek_api_model_mappings();
        deepseek.api_model_vision_support = HashMap::from([
            ("deepseek-flash".to_string(), true),
            ("deepseek-v4-pro".to_string(), false),
        ]);
        let collection = test_local_access_collection(vec![deepseek.id.clone()]);

        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::from([(deepseek.id.clone(), deepseek.clone())]),
            true,
            None,
        )
        .expect("prepare API service sidecar config");

        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(super::sidecar_manifest_path(&dir)).expect("read manifest"),
        )
        .expect("parse manifest");
        let api_key = manifest
            .get("apiKeys")
            .and_then(Value::as_array)
            .and_then(|keys| keys.first())
            .expect("client API key");
        let routes = api_key
            .pointer("/modelRouting/routes")
            .and_then(Value::as_array)
            .expect("automatic routes");
        assert_eq!(routes.len(), 1, "DeepSeek 账号必须走 provider 路由: {api_key}");
        let route = &routes[0];
        assert_eq!(
            route
                .pointer("/providerGateway/visionRoutingModel")
                .and_then(Value::as_str),
            Some("deepseek-flash"),
            "带图片的文本模型请求必须自动转到识图模型"
        );
        let route_models = route
            .get("models")
            .and_then(Value::as_array)
            .expect("route models");
        assert_eq!(
            route_models
                .iter()
                .filter_map(|model| model.get("clientModel").and_then(Value::as_str))
                .collect::<Vec<_>>(),
            vec!["deepseek-flash", "deepseek-v4-pro"]
        );
        // 原生账号池不得再注册该账号，否则请求会走原生通道、图片无法改道。
        let config: Value = serde_json::from_str(
            &fs::read_to_string(super::sidecar_config_path(&dir)).expect("read config"),
        )
        .expect("parse config");
        assert!(
            config.get("codex-api-key").is_none(),
            "DeepSeek 账号不应再写入原生 codex-api-key: {config}"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn sidecar_config_disables_chat_image_generation_for_oauth_pool() {
        let _shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
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
        let _shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
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

        // Chat 协议账号现在属于 API 服务成员，既保留在账号池，也保留在 API Key 作用域。
        assert_eq!(collection.account_ids, vec![account_id.clone()]);
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
        // Provider Gateway 账号同样属于 API 服务成员，账号池与 API Key 作用域都保留。
        assert_eq!(collection.account_ids, vec![account_id.clone()]);
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

    #[tokio::test]
    async fn occupied_persisted_gateway_port_is_released_before_start() {
        let _lock = crate::modules::test_support::env_lock()
            .lock().unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("gateway-port-conflict");
        let profile_dir = make_temp_dir("gateway-port-conflict-profile");
        let runtime_id = "codex_gateway_port_conflict";

        // 先占住一个端口，模拟持久端口被其它实例或进程占用。
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind occupied port");
        let occupied_port = occupied.local_addr().expect("occupied addr").port();
        super::save_provider_gateway_profile_state(
            &profile_dir,
            runtime_id,
            &super::ProviderGatewayProfileState {
                api_key: "persisted-gateway-key".to_string(),
                port: Some(occupied_port),
                created_at: 0,
                updated_at: 0,
            },
        )
        .expect("save gateway state");

        assert!(
            super::provider_gateway_profile_port_occupied_by_others(&profile_dir, runtime_id).await,
            "被其它进程占用的持久端口必须判为冲突"
        );

        super::release_occupied_provider_gateway_profile_port(&profile_dir, runtime_id).await;

        let stored = super::load_provider_gateway_profile_state(&profile_dir, runtime_id)
            .expect("load gateway state")
            .expect("gateway state exists");
        assert_eq!(stored.port, None, "冲突端口必须被放弃，改由重新分配取空闲端口");
        assert_eq!(
            stored.api_key, "persisted-gateway-key",
            "放弃端口不能改动网关密钥等其它状态"
        );

        drop(occupied);
    }

    fn read_profile_model_catalog(profile_dir: &std::path::Path) -> Value {
        let config =
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        let doc = config.parse::<Document>().expect("parse config");
        let catalog_file = doc["model_catalog_json"]
            .as_str()
            .expect("profile must point at a model catalog");
        let content = fs::read_to_string(profile_dir.join(catalog_file)).expect("read catalog");
        serde_json::from_str(&content).expect("parse catalog")
    }

    fn catalog_model_slugs(catalog: &Value) -> Vec<String> {
        catalog
            .get("models")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| model.get("slug").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn api_service_takeover_catalog_lists_account_pool_models() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("api-service-pool-catalog");
        let profile_dir = make_temp_dir("api-service-pool-catalog-profile");

        let mut deepseek = CodexAccount::new_api_key(
            "deepseek-pool-account".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-flash".to_string(),
                "deepseek-v4-pro".to_string(),
            ],
        );
        // 真实环境里 DeepSeek 这类账号既可能按原生 Responses 接入，也可能按 Chat 协议转发。
        deepseek.api_wire_api = Some("responses".to_string());
        // 规范化后的 DeepSeek 账号带有逐模型识图开关。
        deepseek.api_model_vision_support = HashMap::from([
            ("deepseek-flash".to_string(), true),
            ("deepseek-v4-pro".to_string(), false),
        ]);
        crate::modules::codex_account::save_account(&deepseek).expect("save DeepSeek fixture");

        let mut chat_account = CodexAccount::new_api_key(
            "chat-pool-account".to_string(),
            "chat@example.com".to_string(),
            "sk-chat".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://example.com/v1".to_string()),
            None,
            None,
            vec!["vendor-model".to_string()],
        );
        chat_account.api_wire_api = Some("chat_completions".to_string());
        crate::modules::codex_account::save_account(&chat_account).expect("save chat fixture");

        let collection = test_local_access_collection(vec![
            deepseek.id.clone(),
            chat_account.id.clone(),
        ]);
        write_local_access_profile_takeover(&profile_dir, &collection, None, true)
            .await
            .expect("write API service takeover");

        let catalog = read_profile_model_catalog(&profile_dir);
        let slugs = catalog_model_slugs(&catalog);
        // 客户端按 priority 升序展示：账号池里没有官方 GPT 能力时目录里只剩账号模型。
        let ordered: Vec<(String, i64)> = catalog["models"]
            .as_array()
            .expect("catalog models")
            .iter()
            .filter_map(|model| {
                let object = model.as_object()?;
                let slug = object.get("slug")?.as_str()?.to_string();
                if object.get("visibility")?.as_str()? == "hide" {
                    return None;
                }
                Some((slug, object.get("priority")?.as_i64()?))
            })
            .collect::<Vec<_>>();
        let mut ordered_slugs = ordered.clone();
        ordered_slugs.sort_by_key(|(_, priority)| *priority);
        let listed = ordered_slugs
            .iter()
            .map(|(slug, _)| slug.as_str())
            .collect::<Vec<_>>();
        // 账号池里没有能承接官方 GPT 模型的账号（DeepSeek 目录是自己的模型、chat 账号是 vendor 模型），
        // 因此官方推荐 GPT 集不再进入客户端选择器。
        assert_eq!(
            listed
                .iter()
                .filter(|slug| slug.starts_with("deepseek"))
                .copied()
                .collect::<Vec<_>>(),
            vec!["deepseek-flash", "deepseek-v4-pro"],
            "账号模型必须全部出现在客户端目录里: {listed:?}"
        );
        assert!(
            !listed.iter().any(|slug| slug.starts_with("gpt-5")
                || slug.starts_with("gpt-6")
                || *slug == "gpt-5.5"),
            "账号池无 GPT 能力时不应展示官方 GPT 模型: {listed:?}"
        );
        // 账号池里没有任何能承接官方模型的账号：连额度兜底条目也不再保留。
        let gpt_slugs = slugs
            .iter()
            .filter(|slug| slug.starts_with("gpt-") && !slug.starts_with("gpt-image"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert!(
            gpt_slugs.is_empty(),
            "账号池无 GPT 能力时不应保留任何 GPT 条目: {slugs:?}"
        );
        let catalog_display_names = catalog
            .get("models")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| {
                        let object = model.as_object()?;
                        Some((
                            object.get("slug")?.as_str()?.to_string(),
                            object.get("display_name")?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        for (slug, expected_name) in [
            ("deepseek-flash", "DeepSeek-V4.1-Flash"),
            ("deepseek-v4-pro", "DeepSeek-V4-Pro"),
        ] {
            assert_eq!(
                catalog_display_names.get(slug).map(String::as_str),
                Some(expected_name),
                "账号模型 {slug} 的显示名必须来自账号目录"
            );
        }
        assert!(
            catalog_display_names.contains_key("codex-auto-review"),
            "客户端内部使用的隐藏模型元数据必须保留: {slugs:?}"
        );
        // DeepSeek 与网关模式一致：只展示账号模型列表里的两个模型。
        let deepseek_slugs = slugs
            .iter()
            .filter(|slug| slug.starts_with("deepseek"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            deepseek_slugs,
            vec!["deepseek-flash", "deepseek-v4-pro"],
            "DeepSeek 只展示账号模型列表里的模型: {slugs:?}"
        );
        // 存在识图兜底模型时，两个模型都声明可发送图片（图片由网关转到识图模型）。
        for model in catalog["models"].as_array().expect("catalog models") {
            let slug = model.get("slug").and_then(Value::as_str).unwrap_or_default();
            if !slug.starts_with("deepseek") {
                continue;
            }
            let modalities = model
                .get("input_modalities")
                .and_then(Value::as_array)
                .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            assert!(
                modalities.contains(&"image"),
                "{slug} 必须声明可发送图片（与 DeepSeek 网关模式一致）: {modalities:?}"
            );
            // 推理档位必须与 DeepSeek 网关模式一致：low / high / max 三档，且包含最高档。
            let levels = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|level| {
                            level
                                .get("effort")
                                .and_then(Value::as_str)
                                .or_else(|| level.as_str())
                                .map(str::to_string)
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            assert_eq!(
                levels,
                vec!["low", "high", "max"],
                "{slug} 的推理档位必须与 DeepSeek 网关模式一致（含最高档）"
            );
            assert_eq!(
                model.get("default_reasoning_level").and_then(Value::as_str),
                Some("max"),
                "{slug} 的默认档位应为最高档"
            );
        }
        for expected in ["deepseek-flash", "deepseek-v4-pro", "vendor-model"] {
            assert!(
                slugs.iter().any(|slug| slug == expected),
                "账号模型 {expected} 必须写入客户端模型目录，实际: {slugs:?}"
            );
        }
        let display_names = catalog
            .get("models")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| {
                        let object = model.as_object()?;
                        Some((
                            object.get("slug")?.as_str()?.to_string(),
                            object.get("display_name")?.as_str()?.to_string(),
                        ))
                    })
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        // 显示名与 DeepSeek 网关模式一致。
        assert_eq!(
            display_names.get("deepseek-flash").map(String::as_str),
            Some("DeepSeek-V4.1-Flash"),
            "DeepSeek 默认模型应使用官方显示名"
        );
        assert_eq!(
            display_names.get("deepseek-v4-pro").map(String::as_str),
            Some("DeepSeek-V4-Pro"),
            "DeepSeek Pro 应使用官方显示名"
        );

        // 供应商网关 / 实例接管仍然自己写模型目录，不能被账号池模型覆盖。
        let gateway_profile_dir = make_temp_dir("api-service-pool-catalog-gateway-profile");
        write_local_access_profile_takeover(&gateway_profile_dir, &collection, None, false)
            .await
            .expect("write provider gateway takeover");
        let gateway_slugs = catalog_model_slugs(&read_profile_model_catalog(&gateway_profile_dir));
        assert!(
            gateway_slugs.iter().all(|slug| !slug.starts_with("deepseek")),
            "非 API 服务接管不应注入账号池模型: {gateway_slugs:?}"
        );
        assert!(
            gateway_slugs.iter().all(|slug| slug != "vendor-model"),
            "非 API 服务接管不应注入 Chat 账号模型: {gateway_slugs:?}"
        );

        fs::remove_dir_all(profile_dir).expect("cleanup fixture");
        fs::remove_dir_all(gateway_profile_dir).expect("cleanup fixture");
    }
