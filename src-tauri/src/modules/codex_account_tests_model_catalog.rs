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
        // 识图默认值跟随账号模型列表：列表里没有的模型不再被强制写入。
        assert!(account
            .api_model_vision_support
            .get("deepseek-v4-flash-vision-exp")
            .is_none());
        // 模型列表以用户数据为准：非空列表不再被官方默认三条覆盖。
        assert_eq!(
            account.api_model_catalog,
            vec!["deepseek-v4-pro".to_string()]
        );
        assert_eq!(
            account.api_model_mappings,
            super::default_deepseek_api_model_mappings()
        );
    }

    #[test]
    fn third_party_deepseek_preset_keeps_its_own_endpoint() {
        // 供应商可以选择 DeepSeek 预设（身份）却把地址指向第三方中转：此时不能被当成
        // 官方账号，地址、协议与模型列表必须原样保留（模型名按上游真实 ID）。
        let mut account = CodexAccount::new_api_key(
            "third-party-deepseek-key".to_string(),
            "third-party@example.com".to_string(),
            "sk-third-party".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fan/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek 中转".to_string()),
            vec!["deepseek-chat".to_string()],
        );
        account.api_wire_api = Some("chat_completions".to_string());

        assert!(!super::is_deepseek_account(&account));
        assert!(!super::normalize_deepseek_account(&mut account));
        assert_eq!(
            account.api_base_url.as_deref(),
            Some("https://api.apikey.fan/v1")
        );
        assert_eq!(account.api_wire_api.as_deref(), Some("chat_completions"));
        assert_eq!(
            account.api_model_catalog,
            vec!["deepseek-chat".to_string()]
        );
    }

    #[test]
    fn provider_account_access_modes_are_generic() {
        // 第三方供应商账号也能选择直连上游 / CDP 注入，并按选择决定是否做壳位改写。
        let mut account = CodexAccount::new_api_key(
            "third-party-provider-key".to_string(),
            "third-party@example.com".to_string(),
            "sk-provider".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fan/v1".to_string()),
            Some("cmp_provider".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["deepseek-chat".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());

        assert!(!super::account_uses_cdp_model_injection(&account));
        assert!(!super::account_uses_provider_direct_access(&account));
        assert!(!super::account_uses_raw_provider_model_ids(&account));

        account.api_instance_access_mode = Some("cdp".to_string());
        assert!(super::account_uses_cdp_model_injection(&account));
        assert!(super::account_uses_raw_provider_model_ids(&account));

        account.api_instance_access_mode = Some("direct".to_string());
        assert!(super::account_uses_provider_direct_access(&account));
        assert!(!super::account_uses_cdp_model_injection(&account));

        // Chat Completions 不能直连上游（Codex 端只支持 Responses 直连）。
        account.api_wire_api = Some("chat_completions".to_string());
        assert!(!super::account_wire_api_is_responses(&account));
    }

    /// 历史遗留的 Codex/GPT 内置 id（例如 gpt-5.6-sol / gpt-5.6-terra）不能再作为 DeepSeek
    /// 的客户端可见模型名：客户端会用内置 GPT 元数据生成工具定义，DeepSeek 上游只能把工具调用
    /// 写成文本标记（DSML）返回，链路无法解析，正文里就会直接出现原始标记。
    #[test]
    fn deepseek_account_mappings_drop_legacy_gpt_shell_ids() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-legacy-mappings".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-flash".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_model_mappings = vec![
            CodexApiModelMapping {
                client_model: "gpt-5.6-sol".to_string(),
                upstream_model: "deepseek-v4-flash".to_string(),
            },
            CodexApiModelMapping {
                client_model: "gpt-5.6-terra".to_string(),
                upstream_model: "deepseek-v4-pro".to_string(),
            },
            CodexApiModelMapping {
                client_model: "gpt-5.6-luna".to_string(),
                upstream_model: "deepseek-v4-flash".to_string(),
            },
            CodexApiModelMapping {
                client_model: "custom-alias".to_string(),
                upstream_model: "deepseek-v4-flash".to_string(),
            },
        ];

        assert!(super::normalize_deepseek_account(&mut account));

        let client_models: Vec<&str> = account
            .api_model_mappings
            .iter()
            .map(|mapping| mapping.client_model.as_str())
            .collect();
        for legacy in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
            assert!(
                !client_models
                    .iter()
                    .any(|model| model.eq_ignore_ascii_case(legacy)),
                "历史 Codex/GPT 模型 id 不能继续作为 DeepSeek 的客户端可见名: {legacy}"
            );
        }
        assert!(
            client_models
                .iter()
                .any(|model| model.eq_ignore_ascii_case("custom-alias")),
            "用户自定义别名不能被清理"
        );
        for expected in [
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
            "deepseek-flash",
            "deepseek-v4-flash",
            "deepseek-v4-pro",
            "deepseek-v4-flash-vision-exp",
        ] {
            assert!(
                client_models
                    .iter()
                    .any(|model| model.eq_ignore_ascii_case(expected)),
                "缺少默认模型映射: {expected}"
            );
        }
        // 目录壳位仍指向 DeepSeek 上游；历史 GPT 模型 id 不再被解析成 DeepSeek 模型。
        assert_eq!(
            super::resolve_account_upstream_model(&account, "gpt-5.5"),
            "deepseek-flash"
        );
        assert_eq!(
            super::resolve_account_upstream_model(&account, "gpt-5.6-sol"),
            "gpt-5.6-sol"
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
    fn deepseek_account_normalize_backfills_new_vision_mapping_without_overwriting_custom() {
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
        account.api_wire_api = Some("responses".to_string());
        account.api_model_mappings = vec![CodexApiModelMapping {
            client_model: "gpt-5.4-mini".to_string(),
            upstream_model: "custom-vision".to_string(),
        }];

        assert!(super::normalize_deepseek_account(&mut account));
        assert_eq!(
            account
                .api_model_mappings
                .iter()
                .find(|mapping| mapping.client_model.eq_ignore_ascii_case("gpt-5.4-mini"))
                .map(|mapping| mapping.upstream_model.as_str()),
            Some("custom-vision")
        );
        assert!(account.api_model_mappings.iter().any(|mapping| {
            mapping.client_model == "deepseek-v4-flash-vision-exp"
                && mapping.upstream_model == "deepseek-v4-flash-vision-exp"
        }));
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
    fn deepseek_official_catalog_keeps_upstream_names_and_official_metadata() {
        let account = CodexAccount::new_api_key(
            "deepseek-catalog".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        let json = super::build_deepseek_official_model_catalog_json(&account)
            .expect("build catalog");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog");
        let models = value
            .get("models")
            .and_then(|item| item.as_array())
            .expect("models array");
        assert!(models.len() >= 2);
        assert_eq!(
            models[0].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-flash")
        );
        assert_eq!(
            models[0].get("display_name").and_then(|item| item.as_str()),
            Some("DeepSeek-V4.1-Flash")
        );
        assert_eq!(
            models[0].get("description").and_then(|item| item.as_str()),
            Some("deepseek-flash")
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
        let vision = models
            .iter()
            .find(|model| {
                model.get("slug").and_then(|item| item.as_str())
                    == Some("deepseek-flash")
            })
            .expect("vision model");
        assert_eq!(
            vision.get("input_modalities"),
            Some(&serde_json::json!(["text", "image"]))
        );
        // 官方完整条目：官方声明的多 agent / 客户端版本要求必须原样带过去，
        // 不能再从 Codex 内置模型壳继承计费档位与套餐门控。
        assert_eq!(
            models[0]
                .get("multi_agent_version")
                .and_then(|item| item.as_str()),
            Some("v2")
        );
        assert_eq!(
            models[0]
                .get("minimal_client_version")
                .and_then(|item| item.as_str()),
            Some("0.144.0")
        );
        assert_eq!(
            models[0]
                .get("effective_context_window_percent")
                .and_then(|item| item.as_i64()),
            Some(95)
        );
        for model in models {
            for shell_field in [
                "service_tiers",
                "additional_speed_tiers",
                "available_in_plans",
                "include_apps_usage_instructions",
                "include_plugin_usage_instructions",
            ] {
                assert!(
                    model.get(shell_field).is_none(),
                    "{shell_field} 不应来自 Codex 内置模型壳: {model}"
                );
            }
        }
    }

    #[test]
    fn deepseek_official_catalog_json_prefers_flash_and_keeps_tool_metadata() {
        let account = CodexAccount::new_api_key(
            "deepseek-catalog".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        let json =
            super::build_deepseek_official_model_catalog_json(&account).expect("build catalog");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog");
        let models = value
            .get("models")
            .and_then(|item| item.as_array())
            .expect("models array");
        assert!(models.len() >= 2);
        assert_eq!(
            models[0].get("slug").and_then(|item| item.as_str()),
            Some("deepseek-flash")
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
    fn deepseek_compaction_fallback_round_trips_user_values() {
        let base_dir = make_temp_dir("codex-deepseek-compaction-fallback");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\n\n[features]\njs_repl = false\n",
        )
        .expect("parse config");
        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        let applied = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(applied.contains("remote_compaction_v2 = false"));
        // `token_budget = true` 会让客户端把压缩换成不产摘要的窗口重置，必须保持清除状态。
        assert!(!applied.contains("token_budget"));
        assert!(applied.contains("js_repl = false"));

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(!restored.contains("remote_compaction_v2"));
        assert!(!restored.contains("token_budget"));
        assert!(restored.contains("js_repl = false"));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_compaction_fallback_restores_original_user_settings() {
        let base_dir = make_temp_dir("codex-deepseek-compaction-restore");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "[features]\nremote_compaction_v2 = true\ntoken_budget = false\n",
        )
        .expect("parse config");
        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        let applied = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(applied.contains("remote_compaction_v2 = false"));
        assert!(!applied.contains("token_budget"));

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(restored.contains("remote_compaction_v2 = true"));
        assert!(restored.contains("token_budget = false"));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_profile_overrides_reapply_after_gateway_takeover() {
        let base_dir = make_temp_dir("codex-deepseek-gateway-fallback");
        // 接管流程还没写入 profile 配置时不生成残缺配置。
        assert!(!super::reapply_deepseek_config_overrides_for_dir(&base_dir).expect("skip missing"));
        assert!(!super::get_config_toml_path(&base_dir).exists());

        let config_path = super::get_config_toml_path(&base_dir);
        // 模拟实例网关接管后的 profile：网关运行账号 + 已被接管流程清掉的 DeepSeek 兜底。
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &config_path,
            "model = \"gpt-5.6-sol\"\n\n[features]\njs_repl = false\n\n[model_providers.codex_local_access]\nname = \"OpenAI\"\n",
        )
        .expect("write config");

        assert!(super::reapply_deepseek_config_overrides_for_dir(&base_dir).expect("reapply"));
        let applied = fs::read_to_string(&config_path).expect("read config");
        assert!(applied.contains("remote_compaction_v2 = false"));
        assert!(!applied.contains("token_budget"));
        assert!(applied.contains("js_repl = false"));
        assert!(applied.contains("name = \"OpenAI\""));
        assert!(base_dir.join(super::DEEPSEEK_COMPACTION_BACKUP_FILE).exists());

        // 已经补写过时不重复改写。
        assert!(!super::reapply_deepseek_config_overrides_for_dir(&base_dir).expect("idempotent"));

        // 切走时仍按备份还原：兜底键被清掉，用户原有设置保留。
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&applied)
            .expect("parse config");
        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(!restored.contains("remote_compaction_v2"));
        assert!(!restored.contains("token_budget"));
        assert!(restored.contains("js_repl = false"));
        assert!(restored.contains("name = \"OpenAI\""));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn local_compaction_fallback_forwards_profile_and_is_idempotent() {
        let base_dir = make_temp_dir("codex-local-compaction-fallback");
        // 接管还没写入 profile 配置时跳过，避免生成没有 provider 的残缺配置。
        assert!(!super::ensure_local_compaction_fallback_for_dir(&base_dir).expect("skip missing"));

        let config_path = super::get_config_toml_path(&base_dir);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &config_path,
            "model = \"gpt-5.6-sol\"\nservice_tier = \"priority\"\nweb_search = \"live\"\n\n[features]\njs_repl = false\n\n[model_providers.codex_local_access]\nname = \"OpenAI\"\n",
        )
        .expect("write config");

        assert!(super::ensure_local_compaction_fallback_for_dir(&base_dir).expect("apply"));
        let applied = fs::read_to_string(&config_path).expect("read config");
        assert!(applied.contains("remote_compaction_v2 = false"));
        assert!(!applied.contains("token_budget"));
        // 只动压缩键：账号池里的官方账号仍要保留 service_tier / web_search 等原有设置。
        assert!(applied.contains("service_tier = \"priority\""));
        assert!(applied.contains("web_search = \"live\""));
        assert!(!applied.contains("web_search = \"disabled\""));

        assert!(!super::ensure_local_compaction_fallback_for_dir(&base_dir).expect("idempotent"));
        assert_eq!(
            fs::read_to_string(&config_path).expect("read config"),
            applied
        );
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn local_compaction_fallback_clears_legacy_token_budget_override() {
        let base_dir = make_temp_dir("codex-local-compaction-legacy-cleanup");
        let config_path = super::get_config_toml_path(&base_dir);
        // 复现 1.3.54 / 1.3.55 写下的受管状态：远端压缩关闭 + token_budget 打开（换窗口模式）。
        crate::modules::codex_config_format::write_codex_config_toml_atomic(
            &config_path,
            "model = \"gpt-5.6-sol\"\n\n[features]\nremote_compaction_v2 = false\ntoken_budget = true\njs_repl = false\n",
        )
        .expect("write config");

        assert!(super::ensure_local_compaction_fallback_for_dir(&base_dir).expect("apply"));
        let applied = fs::read_to_string(&config_path).expect("read config");
        assert!(applied.contains("remote_compaction_v2 = false"));
        assert!(!applied.contains("token_budget"));
        assert!(applied.contains("js_repl = false"));

        // 清理后保持幂等，避免每次启动都改写配置。
        assert!(!super::ensure_local_compaction_fallback_for_dir(&base_dir).expect("idempotent"));
        assert_eq!(
            fs::read_to_string(&config_path).expect("read config"),
            applied
        );
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_overrides_disable_web_search_and_clean_conflict_keys() {
        let base_dir = make_temp_dir("codex-deepseek-config-overrides");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\nweb_search = \"live\"\nmodel_verbosity = \"low\"\nplan_mode_reasoning_effort = \"xhigh\"\nbase_instructions = \"custom\"\nservice_tier = \"priority\"\nmodel_context_window = 1000000\n",
        )
        .expect("parse config");

        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        let applied = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(applied.contains("web_search = \"disabled\""));
        for removed in [
            "model_verbosity",
            "plan_mode_reasoning_effort",
            "base_instructions",
            "service_tier",
        ] {
            assert!(!applied.contains(removed), "{removed} 应在切到 DeepSeek 时被移除");
        }
        // 上下文窗口属于用户在「上下文管理」里的显式设置，不在清理范围内。
        assert!(applied.contains("model_context_window = 1000000"));

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(restored.contains("web_search = \"live\""));
        assert!(restored.contains("model_verbosity = \"low\""));
        assert!(restored.contains("plan_mode_reasoning_effort = \"xhigh\""));
        assert!(restored.contains("base_instructions = \"custom\""));
        assert!(restored.contains("service_tier = \"priority\""));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_switch_restores_preferred_auth_method_written_by_runtime() {
        // 用户原本手工设置过：DeepSeek 运行态会改写成 apikey，切走必须还原原值。
        let base_dir = make_temp_dir("codex-deepseek-preferred-auth-method-restore");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\npreferred_auth_method = \"chatgpt\"\n",
        )
        .expect("parse config");
        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        doc["preferred_auth_method"] = toml_edit::value("apikey");

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(restored.contains("preferred_auth_method = \"chatgpt\""));
        assert!(!restored.contains("apikey"));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");

        // 用户原本没有该键：切走时要把 DeepSeek 运行态写入的值清掉。
        let empty_dir = make_temp_dir("codex-deepseek-preferred-auth-method-absent");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\n",
        )
        .expect("parse config");
        super::apply_deepseek_config_overrides(&mut doc, &empty_dir);
        doc["preferred_auth_method"] = toml_edit::value("apikey");

        assert!(super::restore_deepseek_config_overrides(&mut doc, &empty_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(!restored.contains("preferred_auth_method"));
        assert!(restored.contains("model = \"gpt-5\""));
        fs::remove_dir_all(&empty_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_overrides_remove_web_search_when_user_had_none() {
        let base_dir = make_temp_dir("codex-deepseek-config-overrides-empty");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\n",
        )
        .expect("parse config");

        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        let applied = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(applied.contains("web_search = \"disabled\""));

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(!restored.contains("web_search"));
        assert!(restored.contains("model = \"gpt-5\""));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deepseek_overrides_leave_table_valued_keys_untouched() {
        let base_dir = make_temp_dir("codex-deepseek-config-overrides-table");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\n\n[web_search]\nmode = \"live\"\n\n[base_instructions]\nvalue = \"custom\"\n",
        )
        .expect("parse config");

        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        let applied = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(applied.contains("[web_search]"));
        assert!(applied.contains("[base_instructions]"));

        assert!(super::restore_deepseek_config_overrides(&mut doc, &base_dir));
        let restored = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        assert!(restored.contains("[web_search]"));
        assert!(restored.contains("[base_instructions]"));
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn switching_away_from_deepseek_restores_overrides_and_catalog() {
        let base_dir = make_temp_dir("codex-deepseek-switch-away-restore");
        let config_path = base_dir.join("config.toml");
        let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(
            "model = \"gpt-5\"\nweb_search = \"live\"\nmodel_verbosity = \"low\"\n",
        )
        .expect("parse config");
        super::apply_deepseek_config_overrides(&mut doc, &base_dir);
        doc["model_catalog_json"] = toml_edit::value(super::CODEX_MANAGED_MODEL_CATALOG_FILE);
        fs::write(
            &config_path,
            crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc),
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"deepseek-flash","apply_patch_tool_type":"freeform"}]}"#,
        )
        .expect("write deepseek catalog");

        assert!(super::cleanup_deepseek_official_model_catalog_for_dir(&base_dir).expect("cleanup"));
        let restored = fs::read_to_string(&config_path).expect("read config");
        assert!(restored.contains("web_search = \"live\""));
        assert!(restored.contains("model_verbosity = \"low\""));
        assert!(!restored.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    fn assert_builtin_provider_config(content: &str) {
        let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(content)
            .expect("parse builtin provider config");
        let expected = if cfg!(target_os = "windows") {
            Some("openai")
        } else {
            None
        };
        assert_eq!(
            doc.get("model_provider").and_then(|item| item.as_str()),
            expected
        );
        assert!(doc.get("model_providers").is_none(), "config: {content}");
    }

    /// 通用回归：工具写入的第三方 provider 引用必须在切回官方账号时还原。
    ///
    /// 官方账号投影只清理受管 provider（`openai` / `codex_local_access` / ...），并把其余 id 当作
    /// 用户自定义 provider 保留——工具自己写给供应商账号的 `model_provider` 因此会残留，
    /// 表现为「账号已经切回普通账号，客户端仍按上一个供应商发请求」。这里用非 DeepSeek 的自建
    /// 中转验证机制是通用的，而不是按供应商 id 写特判。
    #[test]
    fn official_account_switch_restores_supplier_provider_snapshot() {
        let base_dir = make_temp_dir("codex-provider-override-snapshot");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5.5\"\n").expect("write official config");

        let relay_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve relay config");
        super::write_api_provider_to_config_toml_with_options(&base_dir, &relay_config, false)
            .expect("write relay provider");

        let relay_content = fs::read_to_string(&config_path).expect("read relay config");
        assert!(relay_content.contains("model_provider = \"relay\""));
        assert!(relay_content.contains("[model_providers.relay]"));
        assert!(base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).is_file());

        let official_config =
            resolve_api_provider_config(None, Some(CodexApiProviderMode::OpenaiBuiltin), None, None)
                .expect("resolve official config");
        super::write_api_provider_to_config_toml_with_options(&base_dir, &official_config, false)
            .expect("write official provider");

        let content = fs::read_to_string(&config_path).expect("read official config");
        assert_builtin_provider_config(&content);
        assert!(!content.contains("relay.example.com"), "config: {content}");
        assert!(content.contains("model = \"gpt-5.5\""), "config: {content}");
        assert!(
            !base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).exists(),
            "快照只服务一次还原"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_snapshot_preserves_manual_provider_and_model_changes() {
        for manual_provider in [Some("myrelay"), None] {
            let base_dir = make_temp_dir("codex-provider-snapshot-manual-change");
            let config_path = base_dir.join("config.toml");
            fs::write(&config_path, "model = \"gpt-5.5\"\n").expect("write original config");
            let relay = resolve_api_provider_config(
                Some("https://relay.example.com/v1"),
                Some(CodexApiProviderMode::Custom),
                Some("relay"),
                Some("Relay"),
            )
            .expect("resolve relay");
            super::write_api_provider_to_config_toml_with_options(&base_dir, &relay, false)
                .expect("write relay config and snapshot");
            assert!(base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).is_file());

            // Simulate a later user edit, including selecting the implicit builtin provider.
            let current = fs::read_to_string(&config_path).expect("read relay config");
            let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&current)
                .expect("parse relay config");
            doc["model"] = toml_edit::value("gpt-6-sol");
            if let Some(provider) = manual_provider {
                doc["model_provider"] = toml_edit::value(provider);
                doc["model_providers"][provider] = toml_edit::table();
                doc["model_providers"][provider]["name"] = toml_edit::value("My Relay");
                doc["model_providers"][provider]["base_url"] =
                    toml_edit::value("https://myrelay.example.com/v1");
                doc["model_providers"][provider]["wire_api"] = toml_edit::value("responses");
            } else {
                doc.remove("model_provider");
            }
            fs::write(
                &config_path,
                crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc),
            )
            .expect("write user's provider and model");

            let official = resolve_api_provider_config(
                None,
                Some(CodexApiProviderMode::OpenaiBuiltin),
                None,
                None,
            )
            .expect("resolve builtin");
            super::write_api_provider_to_config_toml_with_options(&base_dir, &official, false)
                .expect("switch to official account");
            let content = fs::read_to_string(&config_path).expect("read final config");
            let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
                .expect("parse final config");
            let expected_provider = manual_provider.or_else(|| {
                if cfg!(target_os = "windows") {
                    Some("openai")
                } else {
                    None
                }
            });
            assert_eq!(
                doc.get("model_provider").and_then(|item| item.as_str()),
                expected_provider
            );
            assert_eq!(doc["model"].as_str(), Some("gpt-6-sol"));
            if let Some(provider) = manual_provider {
                assert_eq!(
                    doc["model_providers"][provider]["base_url"].as_str(),
                    Some("https://myrelay.example.com/v1"),
                );
            }
            assert!(!base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).exists());
            fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
        }
    }

    #[test]
    fn provider_snapshot_restores_original_user_provider_from_legacy_snapshot() {
        let base_dir = make_temp_dir("codex-provider-snapshot-legacy-user-provider");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model = "deepseek-v4-pro"
model_provider = "deepseek"
[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
wire_api = "responses"
[model_providers.myrelay]
name = "My Relay"
base_url = "https://myrelay.example.com/v1"
wire_api = "responses"
"#,
        )
        .expect("write supplier config");
        fs::write(
            base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE),
            r#"{
  "providerId": "deepseek",
  "model": "gpt-5.5",
  "modelProvider": "myrelay",
  "providerTable": null
}"#,
        )
        .expect("write existing snapshot format");
        let official =
            resolve_api_provider_config(None, Some(CodexApiProviderMode::OpenaiBuiltin), None, None)
                .expect("resolve builtin");

        super::write_api_provider_to_config_toml_with_options(&base_dir, &official, false)
            .expect("restore original user provider");
        let content = fs::read_to_string(&config_path).expect("read restored config");
        let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
            .expect("parse restored config");
        assert_eq!(doc["model_provider"].as_str(), Some("myrelay"));
        assert_eq!(doc["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(
            doc["model_providers"]["myrelay"]["base_url"].as_str(),
            Some("https://myrelay.example.com/v1")
        );
        assert!(doc["model_providers"].get("deepseek").is_none());
        assert!(!base_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).exists());
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    /// 没有工具快照时，用户自己配置的 provider 必须原样保留（不能被通用还原误删）。
    #[test]
    fn official_account_switch_keeps_user_owned_provider_without_snapshot() {
        let base_dir = make_temp_dir("codex-provider-override-user-owned");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model = "gpt-5.5"
model_provider = "myrelay"

[model_providers.myrelay]
name = "My Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
        )
        .expect("write custom provider config");

        let official_config =
            resolve_api_provider_config(None, Some(CodexApiProviderMode::OpenaiBuiltin), None, None)
                .expect("resolve official config");
        super::write_api_provider_to_config_toml_with_options(&base_dir, &official_config, false)
            .expect("write official provider");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"myrelay\""));
        assert!(content.contains("[model_providers.myrelay]"));
        assert!(content.contains("model = \"gpt-5.5\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    /// 连续切号回归：受管 API Key → DeepSeek → OAuth 不得恢复已删除的运行时 provider。
    #[test]
    fn provider_snapshot_oauth_bundle_after_deepseek_drops_managed_providers() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-deepseek-switch-back-oauth");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "deepseek-switch-back",
            "rt-deepseek-switch-back",
        ));

        let mut deepseek_account = CodexAccount::new_api_key(
            "deepseek-switch-back-key".to_string(),
            "deepseek-switch-back@example.com".to_string(),
            "sk-deepseek-switch-back".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-pro".to_string()],
        );
        deepseek_account.api_wire_api = Some("responses".to_string());
        deepseek_account.api_sync_model_catalog_to_codex = true;
        deepseek_account.api_instance_access_mode = Some("cdp".to_string());
        deepseek_account.api_startup_model = Some("deepseek-v4-pro".to_string());

        for (access_mode, start_with_managed_provider) in
            [("direct", false), ("direct", true), ("cdp", false), ("cdp", true)]
        {
            deepseek_account.api_instance_access_mode = Some(access_mode.to_string());
            let profile_dir = env
                .home_dir
                .join(format!("managed-profile-{access_mode}-{start_with_managed_provider}"));
            if start_with_managed_provider {
                let mut relay_account = CodexAccount::new_api_key(
                    "relay-before-deepseek".to_string(),
                    "relay@example.com".to_string(),
                    "sk-relay-before-deepseek".to_string(),
                    CodexApiProviderMode::Custom,
                    Some("https://relay.example.com/v1".to_string()),
                    Some("relay".to_string()),
                    Some("Relay".to_string()),
                    vec!["gpt-5.5".to_string()],
                );
                relay_account.api_wire_api = Some("responses".to_string());
                relay_account.api_supports_websockets = false;
                write_account_bundle_to_dir(&profile_dir, &relay_account).expect("write relay bundle");
                let relay_config = fs::read_to_string(profile_dir.join("config.toml")).unwrap();
                assert!(relay_config.contains("model_provider = \"codex_local_access\""));
            }
            write_account_bundle_to_dir(&profile_dir, &deepseek_account)
                .expect("write deepseek bundle");
            let deepseek_config = fs::read_to_string(profile_dir.join("config.toml")).unwrap();
            assert!(deepseek_config.contains("model_provider = \"deepseek\""));
            assert!(deepseek_config.contains("[model_providers.deepseek]"));
            if start_with_managed_provider {
                let snapshot: serde_json::Value = serde_json::from_str(
                    &fs::read_to_string(profile_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE)).unwrap(),
                )
                .expect("read provider snapshot");
                assert_eq!(snapshot["modelProvider"], "codex_local_access");
            }

            write_account_bundle_to_dir(&profile_dir, &oauth_account).expect("write oauth bundle");
            let content = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
            assert_builtin_provider_config(&content);
            assert!(
                !content.contains("experimental_bearer_token"),
                "config: {content}"
            );
            assert!(!content.contains("deepseek"), "config: {content}");
            assert!(
                !content.contains("preferred_auth_method"),
                "config: {content}"
            );
            assert!(!profile_dir
                .join(super::CODEX_PROVIDER_OVERRIDE_FILE)
                .exists());
        }
    }

    #[test]
    fn provider_snapshot_oauth_bundle_preserves_manual_provider_and_model_changes() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-provider-snapshot-manual-bundle");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "provider-manual-bundle",
            "rt-provider-manual-bundle",
        ));
        let mut deepseek_account = CodexAccount::new_api_key(
            "deepseek-manual-bundle-key".to_string(),
            "deepseek-manual-bundle@example.com".to_string(),
            "sk-deepseek-manual-bundle".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-pro".to_string()],
        );
        deepseek_account.api_wire_api = Some("responses".to_string());
        deepseek_account.api_sync_model_catalog_to_codex = true;
        deepseek_account.api_startup_model = Some("deepseek-v4-pro".to_string());

        for (access_mode, manual_provider) in [
            ("direct", Some("myrelay")),
            ("direct", None),
            ("cdp", Some("myrelay")),
            ("cdp", None),
        ] {
            deepseek_account.api_instance_access_mode = Some(access_mode.to_string());
            let profile_dir = env.home_dir.join(format!(
                "manual-profile-{access_mode}-{}",
                manual_provider.unwrap_or("builtin")
            ));
            fs::create_dir_all(&profile_dir).expect("create profile");
            let config_path = profile_dir.join("config.toml");
            fs::write(&config_path, "model = \"gpt-5.5\"\n").expect("write initial config");
            write_account_bundle_to_dir(&profile_dir, &deepseek_account)
                .expect("write deepseek bundle");
            assert!(profile_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).is_file());

            let content = fs::read_to_string(&config_path).expect("read deepseek config");
            let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
                .expect("parse deepseek config");
            doc["model"] = toml_edit::value("gpt-6-sol");
            if let Some(provider) = manual_provider {
                doc["model_provider"] = toml_edit::value(provider);
                doc["model_providers"][provider] = toml_edit::table();
                doc["model_providers"][provider]["name"] = toml_edit::value("My Relay");
                doc["model_providers"][provider]["base_url"] =
                    toml_edit::value("https://myrelay.example.com/v1");
                doc["model_providers"][provider]["wire_api"] = toml_edit::value("responses");
            } else {
                doc.remove("model_provider");
            }
            fs::write(
                &config_path,
                crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc),
            )
            .expect("write manual provider selection");

            // Exercise auth projection and both catalog cleanup paths, then repeat the same
            // projection to ensure a consumed snapshot cannot overwrite the user's selection.
            for _ in 0..2 {
                write_account_bundle_to_dir(&profile_dir, &oauth_account)
                    .expect("write oauth bundle");
                let content = fs::read_to_string(&config_path).expect("read oauth config");
                let doc =
                    crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
                        .expect("parse oauth config");
                let expected_provider = manual_provider.or_else(|| {
                    if cfg!(target_os = "windows") {
                        Some("openai")
                    } else {
                        None
                    }
                });
                assert_eq!(
                    doc.get("model_provider").and_then(|item| item.as_str()),
                    expected_provider,
                );
                assert_eq!(doc["model"].as_str(), Some("gpt-6-sol"));
                if let Some(provider) = manual_provider {
                    assert_eq!(
                        doc["model_providers"][provider]["base_url"].as_str(),
                        Some("https://myrelay.example.com/v1"),
                    );
                }
                assert!(!profile_dir.join(super::CODEX_PROVIDER_OVERRIDE_FILE).exists());
            }
        }
    }

    #[test]
    fn deepseek_catalog_keeps_custom_models_and_honors_vision_override() {
        let mut account = CodexAccount::new_api_key(
            "deepseek-custom-catalog".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec![
                "deepseek-v4-flash".to_string(),
                "deepseek-flash".to_string(),
                "deepseek-v4-pro".to_string(),
                "my-custom-model".to_string(),
            ],
        );
        // 用户手动关掉官方 Flash 的识图，并给自定义模型打开识图。
        account
            .api_model_vision_support
            .insert("deepseek-v4-flash".to_string(), false);
        account
            .api_model_vision_support
            .insert("my-custom-model".to_string(), true);

        let json =
            super::build_deepseek_official_model_catalog_json(&account).expect("build catalog");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse catalog");
        let models = value
            .get("models")
            .and_then(|item| item.as_array())
            .expect("models array");

        let find = |slug: &str| {
            models
                .iter()
                .find(|model| model.get("slug").and_then(|item| item.as_str()) == Some(slug))
                .expect("model present")
        };
        assert_eq!(
            find("deepseek-v4-flash").get("input_modalities"),
            Some(&serde_json::json!(["text"]))
        );
        // 官方新名沿用 Flash 模板：识图默认开启，工具元数据齐全。
        assert_eq!(
            find("deepseek-flash").get("input_modalities"),
            Some(&serde_json::json!(["text", "image"]))
        );
        assert_eq!(
            find("deepseek-flash").get("display_name").and_then(|item| item.as_str()),
            Some("DeepSeek-V4.1-Flash")
        );
        assert_eq!(
            find("deepseek-flash")
                .get("apply_patch_tool_type")
                .and_then(|item| item.as_str()),
            Some("freeform")
        );
        assert_eq!(
            find("my-custom-model").get("input_modalities"),
            Some(&serde_json::json!(["text", "image"]))
        );
        assert_eq!(
            find("deepseek-v4-pro").get("input_modalities"),
            Some(&serde_json::json!(["text"]))
        );
        assert_eq!(
            find("deepseek-flash")
                .get("supported_reasoning_levels")
                .and_then(|item| item.as_array())
                .map(|levels| levels.len()),
            Some(3)
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
                "deepseek-v4-flash-vision-exp".to_string(),
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
    fn deepseek_direct_bundle_writes_startup_model_with_native_catalog() {
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
                "deepseek-v4-flash-vision-exp".to_string(),
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
        assert!(config.contains("model_catalog_json = \"cockpit-model-catalog.json\""));
        let catalog = fs::read_to_string(
            instance_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
        )
        .expect("read direct catalog");
        assert!(catalog.contains("deepseek-v4-flash-vision-exp"));

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
            None,
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
            None,
        )
        .expect_err("chat rejects direct");
        assert!(chat_error.contains("Chat Completions"));

        let chat_updated = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-v4-pro".to_string()),
            None,
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
            None,
        )
        .expect("responses can save cdp");
        assert_eq!(cdp.api_instance_access_mode.as_deref(), Some("cdp"));
        assert!(super::account_uses_deepseek_cdp_injection(&cdp));
    }

    #[test]
    fn deepseek_image_generation_accounts_round_trip_and_validate() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-deepseek-image-accounts-test");

        let mut oauth = CodexAccount::new(
            "image-oauth".to_string(),
            "image@example.com".to_string(),
            CodexTokens {
                id_token: "id-token".to_string(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        oauth.plan_type = Some("plus".to_string());
        save_account(&oauth).expect("save oauth account");

        let other_api_key = CodexAccount::new_api_key(
            "other-api-key".to_string(),
            "other@example.com".to_string(),
            "sk-other".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            Vec::new(),
        );
        save_account(&other_api_key).expect("save other api key account");

        let mut account = CodexAccount::new_api_key(
            "deepseek-image-router".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-flash".to_string()],
        );
        account.api_wire_api = Some("responses".to_string());
        save_account(&account).expect("save deepseek account");

        let updated = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-flash".to_string()),
            Some(vec![oauth.id.clone(), oauth.id.clone()]),
        )
        .expect("image accounts accepted");
        assert_eq!(
            updated.api_image_generation_account_ids,
            vec![oauth.id.clone()]
        );
        let reloaded = load_account(&account.id).expect("reload account");
        assert_eq!(
            reloaded.api_image_generation_account_ids,
            vec![oauth.id.clone()]
        );

        let api_key_error = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-flash".to_string()),
            Some(vec![other_api_key.id.clone()]),
        )
        .expect_err("api key accounts cannot host images");
        assert!(api_key_error.contains("OAuth"));

        let self_error = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-flash".to_string()),
            Some(vec![account.id.clone()]),
        )
        .expect_err("self binding rejected");
        assert!(self_error.contains("自身"));

        let missing_error = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-flash".to_string()),
            Some(vec!["missing-account".to_string()]),
        )
        .expect_err("missing account rejected");
        assert!(missing_error.contains("不存在"));

        let cleared = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            Some("deepseek-flash".to_string()),
            Some(Vec::new()),
        )
        .expect("clear image accounts");
        assert!(cleared.api_image_generation_account_ids.is_empty());
    }

    #[test]
    fn non_deepseek_api_key_account_can_store_image_generation_accounts() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-provider-image-accounts-test");

        let mut oauth = CodexAccount::new(
            "provider-image-oauth".to_string(),
            "provider-image@example.com".to_string(),
            CodexTokens {
                id_token: "id-token".to_string(),
                access_token: "access-token".to_string(),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        oauth.plan_type = Some("plus".to_string());
        save_account(&oauth).expect("save oauth account");

        let account = CodexAccount::new_api_key(
            "apikey-fun-like".to_string(),
            "apikey@example.com".to_string(),
            "sk-relay".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.apikey.fan/v1".to_string()),
            Some("cockpit_api".to_string()),
            Some("APIKEY.FUN".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        save_account(&account).expect("save provider account");

        let updated = update_account_instance_access(
            &account.id,
            None,
            None,
            Some(vec![oauth.id.clone()]),
        )
        .expect("non-DeepSeek account can store image accounts");
        assert_eq!(
            updated.api_image_generation_account_ids,
            vec![oauth.id.clone()]
        );
        assert!(updated.api_instance_access_mode.is_none());

        let access_error = update_account_instance_access(
            &account.id,
            Some("gateway".to_string()),
            None,
            None,
        )
        .expect("第三方供应商账号同样支持接入方式");
        assert_eq!(
            access_error.api_instance_access_mode.as_deref(),
            Some("gateway")
        );

        // 直连上游需要 Responses；Chat Completions 账号会被拒绝。
        let mut chat_account = CodexAccount::new_api_key(
            "chat-only-provider".to_string(),
            "chat-only@example.com".to_string(),
            "sk-chat-only".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("cmp_chat_only".to_string()),
            Some("Relay Chat".to_string()),
            vec!["kimi-k2.6".to_string()],
        );
        chat_account.api_wire_api = Some("chat_completions".to_string());
        save_account(&chat_account).expect("save chat account");
        let direct_error = update_account_instance_access(
            &chat_account.id,
            Some("direct".to_string()),
            None,
            None,
        )
        .expect_err("Chat Completions 账号拒绝直连上游");
        assert!(direct_error.contains("Chat Completions"));
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

    /// 第三方（API Key）账号必须使用自己的模型目录：既不能被「模型管理」的受管目录覆盖，
    /// 也不能反过来改动用户的模型管理开关与模型清单。
    #[test]
    fn api_key_account_catalog_is_independent_from_model_management() {
        let managed_dir = make_temp_dir("codex-api-key-managed-catalog-isolation");
        let baseline_dir = make_temp_dir("codex-api-key-managed-catalog-baseline");

        fs::write(managed_dir.join("config.toml"), "model = \"gpt-5.6-sol\"\n")
            .expect("write base config");
        let definitions = super::default_experimental_model_definitions(&managed_dir);
        super::save_model_catalog_for_base_dir_preserving_context(
            &managed_dir,
            true,
            definitions.clone(),
            None,
        )
        .expect("enable managed catalog");
        let policy_path = managed_dir.join(super::CODEX_EXPERIMENTAL_MODEL_POLICY_FILE);
        assert!(policy_path.is_file(), "前置条件：模型管理已开启");

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

        write_account_bundle_to_dir(&managed_dir, &account).expect("write managed dir bundle");
        write_account_bundle_to_dir(&baseline_dir, &account).expect("write baseline bundle");

        let catalog_file = super::CODEX_MANAGED_MODEL_CATALOG_FILE;
        let managed_catalog =
            fs::read_to_string(managed_dir.join(catalog_file)).expect("read managed catalog");
        let baseline_catalog =
            fs::read_to_string(baseline_dir.join(catalog_file)).expect("read baseline catalog");
        assert_eq!(
            managed_catalog, baseline_catalog,
            "第三方账号的模型目录不能受模型管理影响"
        );
        assert!(policy_path.is_file(), "API Key 账号不能改动模型管理开关");
        let definitions_after = super::read_experimental_model_definitions(&managed_dir);
        assert_eq!(
            definitions_after.len(),
            definitions.len(),
            "用户的模型清单必须保留"
        );

        fs::remove_dir_all(&managed_dir).expect("cleanup temp dir");
        fs::remove_dir_all(&baseline_dir).expect("cleanup temp dir");
    }
