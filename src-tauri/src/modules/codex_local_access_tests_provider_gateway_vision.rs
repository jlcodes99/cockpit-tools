// Codex Local Access 测试：供应商识图能力兜底（issue #2535）。
// 旧版本把识图开关放在供应商上，账号逐模型表可能为空；网关配置必须仍然发送图片。

fn provider_vision_test_account(
    provider_id: &str,
    base_url: &str,
    models: &[&str],
    account_supports_vision: bool,
    account_vision_support: &[(&str, bool)],
) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "acc-provider-vision".to_string(),
        "vendor@example.com".to_string(),
        "sk-provider-vision".to_string(),
        CodexApiProviderMode::Custom,
        Some(base_url.to_string()),
        Some(provider_id.to_string()),
        Some("Vendor".to_string()),
        models.iter().map(|model| model.to_string()).collect(),
    );
    account.api_supports_vision = account_supports_vision;
    account.api_model_vision_support = account_vision_support
        .iter()
        .map(|(model, value)| (model.to_string(), *value))
        .collect();
    account
}

#[test]
fn same_endpoint_selects_model_settings_by_api_key() {
    let providers = serde_json::json!([{
        "baseUrl": "https://relay.example.com/v1",
        "apiKeys": [
            {"apiKey": "key-oai", "modelCatalog": ["gpt-6-sol"], "compactionMode": "remote"},
            {"apiKey": "key-third", "modelCatalog": ["deepseek-v4", "kimi-k3"], "compactionMode": "local"}
        ]
    }]);
    let mut account = provider_vision_test_account(
        "provider-1", "https://relay.example.com/v1", &[], false, &[],
    );
    account.openai_api_key = Some("key-oai".to_string());
    let oai = super::select_model_provider_key_config(&providers, &account).unwrap();
    assert_eq!(oai["modelCatalog"], serde_json::json!(["gpt-6-sol"]));
    account.openai_api_key = Some("key-third".to_string());
    let third = super::select_model_provider_key_config(&providers, &account).unwrap();
    assert_eq!(third["modelCatalog"], serde_json::json!(["deepseek-v4", "kimi-k3"]));
    account.openai_api_key = Some("unknown".to_string());
    assert!(super::select_model_provider_key_config(&providers, &account).is_none());
}

#[test]
fn auto_compact_limits_apply_only_to_the_selected_catalog() {
    let catalog = r#"{"models":[{"slug":"model-x","context_window":256000}]}"#;
    let local_limits = serde_json::json!({"model-x": 220000});
    let remote_limits = serde_json::json!({"model-x": 90000});
    let local = super::apply_auto_compact_limits_to_catalog(
        catalog, &[], local_limits.as_object().unwrap(),
    ).unwrap();
    let remote = super::apply_auto_compact_limits_to_catalog(
        catalog, &[], remote_limits.as_object().unwrap(),
    ).unwrap();
    let local: serde_json::Value = serde_json::from_str(&local).unwrap();
    let remote: serde_json::Value = serde_json::from_str(&remote).unwrap();
    assert_eq!(local["models"][0]["auto_compact_token_limit"], 220000);
    assert_eq!(remote["models"][0]["auto_compact_token_limit"], 90000);
}

#[test]
fn explicit_key_window_overrides_shared_one_million_fallback() {
    let slot = super::ProviderGatewayModelSlot {
        client_model: "model-x".to_string(),
        upstream_model: "model-x".to_string(),
    };
    let windows = std::collections::HashMap::from([("model-x".to_string(), 200_000)]);
    let catalog = r#"{"models":[{"slug":"model-x","context_window":1000000}]}"#;
    let decorated = super::decorate_catalog_context_windows(
        catalog,
        &[slot],
        &windows,
        Some(1_000_000),
    ).unwrap();
    let decorated: serde_json::Value = serde_json::from_str(&decorated).unwrap();
    assert_eq!(decorated["models"][0]["context_window"], 200_000);
    assert_eq!(decorated["models"][0]["max_context_window"], 200_000);
}

#[test]
fn provider_vision_flag_fills_missing_model_capabilities() {
    let models = vec!["openai/gpt-5.6-sol".to_string()];
    let account = provider_vision_test_account(
        "cmp_legacy",
        "https://relay.example.com/v1",
        &["openai/gpt-5.6-sol"],
        false,
        &[],
    );
    let provider = super::CodexModelProviderVisionRecord {
        supports_vision: true,
        model_capabilities: std::collections::HashMap::new(),
    };
    let mut capabilities = std::collections::HashMap::new();

    let gateway_supports_vision = super::merge_provider_vision_capabilities(
        &mut capabilities,
        &account,
        Some(&provider),
        &models,
    );

    assert!(gateway_supports_vision);
    assert_eq!(
        capabilities
            .get("openai/gpt-5.6-sol")
            .map(|capability| capability.supports_vision),
        Some(true)
    );
}

#[test]
fn provider_per_model_entry_wins_over_account_entry() {
    let models = vec!["gpt-5.6-luna".to_string()];
    let account = provider_vision_test_account(
        "cmp_legacy",
        "https://relay.example.com/v1",
        &["gpt-5.6-luna"],
        true,
        &[("gpt-5.6-luna", false)],
    );
    let mut provider_capabilities = std::collections::HashMap::new();
    provider_capabilities.insert("gpt-5.6-luna".to_string(), true);
    let provider = super::CodexModelProviderVisionRecord {
        supports_vision: true,
        model_capabilities: provider_capabilities,
    };
    let mut capabilities = std::collections::HashMap::new();
    capabilities.insert(
        "gpt-5.6-luna".to_string(),
        super::CodexLocalAccessProviderGatewayModelCapability {
            supports_vision: false,
        },
    );

    super::merge_provider_vision_capabilities(&mut capabilities, &account, Some(&provider), &models);

    assert_eq!(
        capabilities
            .get("gpt-5.6-luna")
            .map(|capability| capability.supports_vision),
        Some(true)
    );
}

#[test]
fn account_explicit_disable_still_wins_over_provider_flag() {
    let models = vec!["gpt-5.5".to_string()];
    let account = provider_vision_test_account(
        "cmp_legacy",
        "https://relay.example.com/v1",
        &["gpt-5.5"],
        false,
        &[("gpt-5.5", false)],
    );
    let provider = super::CodexModelProviderVisionRecord {
        supports_vision: true,
        model_capabilities: std::collections::HashMap::new(),
    };
    let mut capabilities = std::collections::HashMap::new();
    capabilities.insert(
        "gpt-5.5".to_string(),
        super::CodexLocalAccessProviderGatewayModelCapability {
            supports_vision: false,
        },
    );

    super::merge_provider_vision_capabilities(&mut capabilities, &account, Some(&provider), &models);

    assert_eq!(
        capabilities
            .get("gpt-5.5")
            .map(|capability| capability.supports_vision),
        Some(false)
    );
}

#[test]
fn provider_vision_record_matches_by_id_or_base_url() {
    let account = provider_vision_test_account(
        "cmp_legacy",
        "https://relay.example.com/v1/",
        &["gpt-5.5"],
        false,
        &[],
    );
    let entries = vec![
        super::CodexModelProviderVisionEntry {
            id: "cmp_other".to_string(),
            base_url: Some("https://other.example.com/v1".to_string()),
            record: super::CodexModelProviderVisionRecord {
                supports_vision: false,
                model_capabilities: std::collections::HashMap::new(),
            },
        },
        super::CodexModelProviderVisionEntry {
            id: "cmp_legacy".to_string(),
            base_url: Some("https://relay.example.com/v1".to_string()),
            record: super::CodexModelProviderVisionRecord {
                supports_vision: true,
                model_capabilities: std::collections::HashMap::new(),
            },
        },
    ];

    let matched =
        super::codex_model_provider_vision_record_for_account(&account, &entries).expect("match");

    assert!(matched.supports_vision);
}

#[test]
fn gpt_5_5_and_later_default_to_vision_without_any_configuration() {
    let models = vec![
        "openai/gpt-5.6-sol".to_string(),
        "gpt-5.5".to_string(),
        "gpt-5.4".to_string(),
    ];
    let account = provider_vision_test_account(
        "cmp_unset",
        "https://relay.example.com/v1",
        &models.iter().map(String::as_str).collect::<Vec<_>>(),
        false,
        &[],
    );
    let mut capabilities = std::collections::HashMap::new();

    let gateway_supports_vision =
        super::merge_provider_vision_capabilities(&mut capabilities, &account, None, &models);

    // 供应商/账号都没有声明：gpt-5.5+ 默认支持，5.5 以下维持原样。
    assert!(!gateway_supports_vision);
    assert_eq!(
        capabilities
            .get("openai/gpt-5.6-sol")
            .map(|capability| capability.supports_vision),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("gpt-5.5")
            .map(|capability| capability.supports_vision),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("gpt-5.4")
            .map(|capability| capability.supports_vision),
        Some(false)
    );
}

#[test]
fn explicit_per_model_disable_beats_gpt_5_5_default() {
    let models = vec!["gpt-5.6-sol".to_string()];
    let account = provider_vision_test_account(
        "cmp_unset",
        "https://relay.example.com/v1",
        &["gpt-5.6-sol"],
        false,
        &[],
    );
    let mut provider_capabilities = std::collections::HashMap::new();
    provider_capabilities.insert("gpt-5.6-sol".to_string(), false);
    let provider = super::CodexModelProviderVisionRecord {
        supports_vision: false,
        model_capabilities: provider_capabilities,
    };
    let mut capabilities = std::collections::HashMap::new();

    super::merge_provider_vision_capabilities(
        &mut capabilities,
        &account,
        Some(&provider),
        &models,
    );

    assert_eq!(
        capabilities
            .get("gpt-5.6-sol")
            .map(|capability| capability.supports_vision),
        Some(false)
    );
}
