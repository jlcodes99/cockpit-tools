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

    super::merge_provider_vision_capabilities(
        &mut capabilities,
        &account,
        Some(&provider),
        &models,
    );

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

    super::merge_provider_vision_capabilities(
        &mut capabilities,
        &account,
        Some(&provider),
        &models,
    );

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
            credential_records: Vec::new(),
            record: super::CodexModelProviderVisionRecord {
                supports_vision: false,
                model_capabilities: std::collections::HashMap::new(),
            },
        },
        super::CodexModelProviderVisionEntry {
            id: "cmp_legacy".to_string(),
            base_url: Some("https://relay.example.com/v1".to_string()),
            credential_records: Vec::new(),
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
fn catalog_image_declarations_determine_vision_without_any_configuration() {
    let models = vec![
        "openai/gpt-5.6-sol".to_string(),
        "gpt-6-luna".to_string(),
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

    // 供应商/账号都没有声明：采用 models.json 的图片输入声明；目录缺少的 gpt-5.4 默认 false。
    assert!(!gateway_supports_vision);
    assert_eq!(
        capabilities
            .get("openai/gpt-5.6-sol")
            .map(|capability| capability.supports_vision),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("gpt-6-luna")
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
fn explicit_per_model_disable_beats_catalog_image_declaration() {
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

#[test]
fn provider_vision_resolves_each_credential_before_overriding_account_capabilities() {
    let entries = super::parse_codex_model_provider_vision_entries(
        r#"[{
        "id": "cmp_keys", "baseUrl": "https://relay.example.com/v1",
        "supportsVision": false,
        "modelCapabilities": {"qwen-vl": {"supportsVision": false}},
        "apiKeys": [
            {"apiKey": " sk-enabled ", "supportsVision": true,
             "modelCapabilities": {"QWEN-VL": {"supportsVision": true}}},
            {"apiKey": "sk-disabled", "supportsVision": false,
             "modelCapabilities": {"qwen-vl": {"supportsVision": false}}},
            {"apiKey": "sk-inherited"}
        ]
    }]"#,
    );
    let models = vec!["qwen-vl".to_string()];
    for (api_key, expected) in [
        (" sk-enabled ", true),
        ("sk-disabled", false),
        ("sk-inherited", false),
        ("sk-not-listed", false),
    ] {
        // A stale account snapshot must not override the selected credential's map.
        let mut account = provider_vision_test_account(
            "cmp_keys",
            "https://relay.example.com/v1",
            &["qwen-vl"],
            !expected,
            &[("qwen-vl", !expected)],
        );
        account.openai_api_key = Some(api_key.to_string());
        let provider = super::codex_model_provider_vision_record_for_account(&account, &entries)
            .expect("credential or provider record");
        assert_eq!(provider.supports_vision, expected, "key={api_key}");
        let mut capabilities = std::collections::HashMap::from([(
            "qwen-vl".to_string(),
            super::CodexLocalAccessProviderGatewayModelCapability {
                supports_vision: !expected,
            },
        )]);
        super::merge_provider_vision_capabilities(
            &mut capabilities,
            &account,
            Some(provider),
            &models,
        );
        assert_eq!(
            capabilities["qwen-vl"].supports_vision, expected,
            "key={api_key}"
        );
    }
}

#[test]
fn provider_vision_credential_catalog_filters_only_inherited_metadata() {
    let entries = super::parse_codex_model_provider_vision_entries(
        r#"[{
        "id": "cmp_keys", "supportsVision": true,
        "modelCapabilities": {
            "qwen-vl": {"supportsVision": false},
            "other-model": {"supportsVision": true}
        },
        "apiKeys": [
            {"apiKey": "sk-inherited"},
            {"apiKey": "sk-catalog", "modelCatalog": ["QWEN-VL"]},
            {"apiKey": "sk-empty-catalog", "modelCatalog": []},
            {"apiKey": "sk-empty-map", "modelCapabilities": {}},
            {"apiKey": "sk-own-map", "modelCatalog": ["qwen-vl"],
             "modelCapabilities": {"other-model": {"supportsVision": false}}}
        ]
    }]"#,
    );
    for (api_key, expected) in [
        (
            "sk-inherited",
            vec![("qwen-vl", false), ("other-model", true)],
        ),
        ("sk-catalog", vec![("qwen-vl", false)]),
        ("sk-empty-catalog", vec![]),
        ("sk-empty-map", vec![]),
        ("sk-own-map", vec![("other-model", false)]),
    ] {
        let mut account = provider_vision_test_account("cmp_keys", "", &["qwen-vl"], false, &[]);
        account.openai_api_key = Some(api_key.to_string());
        let record = super::codex_model_provider_vision_record_for_account(&account, &entries)
            .expect("credential record");
        assert!(record.supports_vision, "key={api_key}");
        assert_eq!(
            record.model_capabilities,
            expected
                .into_iter()
                .map(|(model, flag)| (model.to_string(), flag))
                .collect(),
            "key={api_key}"
        );
    }
}

#[test]
fn provider_vision_credential_flags_inherit_only_when_missing() {
    let entries = super::parse_codex_model_provider_vision_entries(
        r#"[{
        "id": "cmp_keys", "supportsVision": true,
        "apiKeys": [
            {"apiKey": "sk-enabled", "supportsVision": true},
            {"apiKey": "sk-disabled", "supportsVision": false},
            {"apiKey": "sk-inherited"}
        ]
    }]"#,
    );
    for (api_key, expected) in [
        (Some("sk-enabled"), true),
        (Some("sk-disabled"), false),
        (Some("sk-inherited"), true),
        (Some("sk-unlisted"), true),
        (Some("  "), true),
        (None, true),
    ] {
        let mut account = provider_vision_test_account("cmp_keys", "", &["qwen-vl"], false, &[]);
        account.openai_api_key = api_key.map(str::to_string);
        let record = super::codex_model_provider_vision_record_for_account(&account, &entries)
            .expect("credential or provider record");
        assert_eq!(record.supports_vision, expected, "key={api_key:?}");
    }
}

#[test]
fn provider_vision_parser_preserves_legacy_records_and_handles_missing_providers() {
    for content in ["not JSON", "null", "{}", "[{}]"] {
        assert!(super::parse_codex_model_provider_vision_entries(content).is_empty());
    }
    let entries = super::parse_codex_model_provider_vision_entries(
        r#"[
        {"id": "cmp_legacy", "supportsVision": true, "apiKey": "legacy-key"},
        {"baseUrl": " HTTPS://OTHER.EXAMPLE.COM/v1/ ",
         "modelCapabilities": {" QWEN-VL ": {"supportsVision": false}, "invalid": {}}}
    ]"#,
    );
    let legacy = provider_vision_test_account("cmp_legacy", "", &["qwen-vl"], false, &[]);
    assert!(
        super::codex_model_provider_vision_record_for_account(&legacy, &entries)
            .expect("legacy provider record")
            .supports_vision
    );
    let url_match = provider_vision_test_account(
        "cmp_old_id",
        "https://other.example.com/v1/",
        &["qwen-vl"],
        false,
        &[],
    );
    let record = super::codex_model_provider_vision_record_for_account(&url_match, &entries)
        .expect("base URL match");
    assert_eq!(record.model_capabilities.get("qwen-vl"), Some(&false));
    assert_eq!(record.model_capabilities.len(), 1);
    let missing = provider_vision_test_account("cmp_missing", "", &["qwen-vl"], false, &[]);
    assert!(super::codex_model_provider_vision_record_for_account(&missing, &entries).is_none());
}
