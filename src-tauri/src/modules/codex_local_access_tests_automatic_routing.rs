// 自动混合路由测试：同模型多候选、映射 fork、账号级排除、显式路由保留。
fn automatic_routing_account(id: &str, wire_api: &str, models: &[&str]) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        id.to_string(),
        format!("{id}@example.com"),
        format!("sk-{id}"),
        CodexApiProviderMode::Custom,
        Some("https://example.com/v1".to_string()),
        None,
        None,
        models.iter().map(|model| model.to_string()).collect(),
    );
    account.api_wire_api = Some(wire_api.to_string());
    account
}

fn automatic_routing_collection() -> CodexLocalAccessCollection {
    test_local_access_collection(Vec::new())
}

#[test]
fn automatic_routing_keeps_overlapping_candidates_and_scopes_native_models() {
    let collection = automatic_routing_collection();
    let accounts: HashMap<String, CodexAccount> = [
        automatic_routing_account("native", "responses", &["shared", "native-only"]),
        automatic_routing_account("chat-a", "chat_completions", &["shared", "vendor/model"]),
        automatic_routing_account("chat-b", "chat_completions", &["SHARED"]),
    ]
    .into_iter()
    .map(|account| (account.id.clone(), account))
    .collect();

    let routing = super::automatic_api_service_model_routing_value(
        &collection,
        &["native".into(), "chat-a".into(), "chat-b".into()],
        &accounts,
    );

    assert_eq!(routing["automatic"], json!(true));
    assert_eq!(routing["nativeModels"], json!(["shared", "native-only"]));
    let routes = routing["routes"].as_array().expect("routes array");
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0]["models"][0]["clientModel"], json!("shared"));
    assert_eq!(routes[1]["models"][0]["clientModel"], json!("SHARED"));

    let scoped =
        super::automatic_api_service_model_routing_value(&collection, &["chat-a".into()], &accounts);
    assert_eq!(scoped["nativeModels"], json!([]));
    assert_eq!(scoped["routes"].as_array().map(Vec::len), Some(1));
}

#[test]
fn automatic_routing_preserves_mapping_fork_and_model_exclusions() {
    let mut collection = automatic_routing_collection();
    // 账号模型列表是唯一可见来源，映射只决定该模型发给上游的名字。
    let mut account = automatic_routing_account("chat", "chat_completions", &["shared"]);
    account.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "shared".into(),
        upstream_model: "vendor/real".into(),
    }];
    collection.model_aliases = vec![crate::models::codex_local_access::CodexLocalAccessModelAlias {
        source_model: "shared".into(),
        alias: "short".into(),
        fork: true,
    }];
    let accounts = HashMap::from([(account.id.clone(), account)]);

    let routing =
        super::automatic_api_service_model_routing_value(&collection, &["chat".into()], &accounts);
    assert_eq!(
        routing["routes"][0]["models"],
        json!([
            {"clientModel": "shared", "upstreamModel": "vendor/real"},
            {"clientModel": "short", "upstreamModel": "vendor/real"},
        ])
    );
    assert_eq!(
        routing["routes"][0]["providerGateway"]["upstreamModels"],
        json!(["vendor/real"])
    );

    collection.excluded_models = vec!["short".into()];
    collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
        account_id: "chat".into(),
        excluded_models: vec!["vendor/*".into()],
    }];
    let filtered =
        super::automatic_api_service_model_routing_value(&collection, &["chat".into()], &accounts);
    assert_eq!(filtered["routes"], json!([]));
}

#[test]
fn automatic_routing_is_runtime_only_and_preserves_explicit_instance_routes() {
    let collection = automatic_routing_collection();
    let accounts = HashMap::from([(
        "chat".to_string(),
        automatic_routing_account("chat", "chat_completions", &["deepseek-chat"]),
    )]);
    let explicit = json!({"accountIds": ["chat"], "modelRouting": {"routes": []}});
    let internal = json!({"accountIds": ["chat"], "internal": true});
    let mut values = vec![
        json!({"accountIds": ["chat"]}),
        explicit.clone(),
        internal.clone(),
    ];
    let saved = serde_json::to_value(&collection).expect("serialize collection");

    super::apply_automatic_api_service_model_routing(&mut values, &collection, &accounts);

    assert_eq!(values[0]["responsesWebsockets"], json!(false));
    assert_eq!(values[0]["modelRouting"]["automatic"], json!(true));
    assert_eq!(values[0]["modelRouting"]["nativeModels"], json!([]));
    assert_eq!(values[1], explicit);
    // 内部 Key 固定落到指定账号：保留原生路由，不产生任何 provider 候选。
    assert_eq!(values[2]["modelRouting"]["routes"], json!([]));
    assert_eq!(values[2]["modelRouting"]["automatic"], json!(true));
    assert_eq!(values[2]["internal"], json!(true));
    assert_eq!(
        serde_json::to_value(&collection).expect("serialize collection"),
        saved
    );
}

/// 选择器只展示官方推荐集里的 GPT 模型，历史模型仍然可路由。
#[test]
fn automatic_routing_trims_visible_gpt_models_and_keeps_history_routable() {
    // 只断言纯函数：清单不依赖全局实验目录（其它用例可能开启「模型管理」）。
    let trimmed = super::automatic_api_service_visible_model_ids(vec![
        "gpt-6-astra".into(),
        "gpt-6-sol".into(),
        "gpt-6-luna".into(),
        "gpt-5.6-sol".into(),
        "gpt-5.4".into(),
        "gpt-5.4-mini".into(),
        "gpt-5.3-codex".into(),
        "gpt-5.3-codex-spark".into(),
        "gpt-image-2.5".into(),
        "codex-auto-review".into(),
        "gpt-reserve".into(),
        "deepseek-flash".into(),
        "deepseek-v4-pro".into(),
    ]);
    assert_eq!(
        trimmed,
        vec![
            "gpt-6-astra",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-5.6-sol",
            "gpt-image-2.5",
            "codex-auto-review",
            "gpt-reserve",
            "deepseek-flash",
            "deepseek-v4-pro"
        ]
    );

    let routable = super::api_service_routable_codex_model_ids();
    for history_model in ["gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex", "gpt-5.3-codex-spark"] {
        assert!(
            routable.iter().any(|model| model.eq_ignore_ascii_case(history_model)),
            "历史模型 {history_model} 必须保留可路由能力: {routable:?}"
        );
        assert!(
            !trimmed
                .iter()
                .any(|model| model.eq_ignore_ascii_case(history_model)),
            "历史模型 {history_model} 不应出现在客户端展示清单里"
        );
    }
}

fn deepseek_routing_account(vision: &[(&str, bool)]) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "deepseek-account".to_string(),
        "deepseek@example.com".to_string(),
        "sk-deepseek".to_string(),
        CodexApiProviderMode::Custom,
        Some("https://api.deepseek.com".to_string()),
        Some("deepseek".to_string()),
        Some("DeepSeek".to_string()),
        vec!["deepseek-flash".to_string(), "deepseek-v4-pro".to_string()],
    );
    account.api_wire_api = Some("responses".to_string());
    account.api_model_mappings = super::codex_account::default_deepseek_api_model_mappings();
    account.api_model_vision_support = vision
        .iter()
        .map(|(model, supports)| ((*model).to_string(), *supports))
        .collect();
    account
}

/// DeepSeek 账号与「DeepSeek 网关模式」保持一致：只展示账号模型列表里的模型，
/// 并把带图片的请求自动转到识图模型。
#[test]
fn automatic_routing_matches_deepseek_gateway_rules() {
    let collection = automatic_routing_collection();
    let account = deepseek_routing_account(&[
        ("deepseek-flash", true),
        ("deepseek-v4-pro", false),
    ]);
    let account_id = account.id.clone();
    let accounts = HashMap::from([(account_id.clone(), account)]);

    let routing = super::automatic_api_service_model_routing_value(
        &collection,
        &[account_id.clone()],
        &accounts,
    );

    // 只保留账号模型列表里的两个模型，映射表里的 shell 别名不再单独展示。
    assert_eq!(routing["nativeModels"], json!([]));
    let routes = routing["routes"].as_array().expect("routes");
    assert_eq!(routes.len(), 1, "具备识图能力的账号必须走 provider 路由: {routing}");
    assert_eq!(
        routes[0]["models"],
        json!([
            {
                "clientModel": "deepseek-flash",
                "upstreamModel": "deepseek-flash",
                "displayName": "DeepSeek-V4.1-Flash",
                "reasoningLevels": [
                    {"effort": "low", "description": "Fast responses with lighter reasoning"},
                    {"effort": "high", "description": "Extra high reasoning depth for complex problems"},
                    {"effort": "max", "description": "Maximum reasoning depth for the hardest problems"},
                ],
                "defaultReasoningLevel": "max",
            },
            {
                "clientModel": "deepseek-v4-pro",
                "upstreamModel": "deepseek-v4-pro",
                "displayName": "DeepSeek-V4-Pro",
                "reasoningLevels": [
                    {"effort": "low", "description": "Fast responses with lighter reasoning"},
                    {"effort": "high", "description": "Extra high reasoning depth for complex problems"},
                    {"effort": "max", "description": "Maximum reasoning depth for the hardest problems"},
                ],
                "defaultReasoningLevel": "max",
            },
        ])
    );
    // 官方模型名与官方显示名保持一致。
    assert_eq!(
        super::codex_account::provider_model_display_name("deepseek-flash"),
        "DeepSeek-V4.1-Flash"
    );
    assert_eq!(
        super::codex_account::provider_model_display_name("deepseek-v4-pro"),
        "DeepSeek-V4-Pro"
    );
    assert_eq!(
        super::codex_account::provider_model_display_name("deepseek-v4-flash-vision-exp"),
        "DeepSeek-V4-Flash-Vision-Exp"
    );
    // 与 DeepSeek 网关模式一致：文本模型上的图片会自动转到识图模型。
    assert_eq!(
        routes[0]["providerGateway"]["visionRoutingModel"],
        json!("deepseek-flash")
    );
    assert_eq!(
        routes[0]["providerGateway"]["upstreamModels"],
        json!(["deepseek-flash", "deepseek-v4-pro"])
    );
}

/// 客户端目录里，DeepSeek 的文本模型也要声明可发送图片（图片会由网关转到识图模型）。
#[test]
fn automatic_routing_marks_deepseek_models_image_capable() {
    let collection = automatic_routing_collection();
    let account = deepseek_routing_account(&[
        ("deepseek-flash", true),
        ("deepseek-v4-pro", false),
    ]);
    let account_id = account.id.clone();
    let accounts = vec![account];
    let api_key = super::ResolvedLocalApiKey {
        id: "legacy".to_string(),
        label: "Default".to_string(),
        provider_gateway: None,
        inherit_account_pool: true,
        account_ids: Vec::new(),
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
    };
    let mut collection_with_account = collection.clone();
    collection_with_account.account_ids = vec![account_id];

    let models = super::automatic_api_service_profile_extra_models(
        &collection_with_account,
        &api_key,
        &accounts,
    );

    assert_eq!(
        models,
        vec![
            ("deepseek-flash".to_string(), true),
            ("deepseek-v4-pro".to_string(), true)
        ],
        "存在识图兜底模型时，两个模型都应声明可发送图片（与 DeepSeek 网关模式一致）"
    );
}

#[test]
fn automatic_routing_pool_catalog_is_union_without_phantom_oauth_models() {
    let mut mapped = automatic_routing_account("mapped", "chat_completions", &["mapped-model"]);
    mapped.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "mapped-model".into(),
        upstream_model: "vendor/real".into(),
    }];
    let accounts = vec![
        automatic_routing_account("chat", "chat_completions", &["shared", "deepseek-chat"]),
        automatic_routing_account("native", "responses", &["SHARED", "custom"]),
        mapped,
    ];

    let models = super::automatic_api_service_pool_model_ids(&accounts, vec!["gpt-only".into()]);

    // 客户端可见清单只包含客户端模型名；上游名（vendor/real）不参与展示。
    assert_eq!(models, vec!["shared", "deepseek-chat", "custom", "mapped-model"]);
}

/// 账号池里没有能承接官方 GPT 模型的账号时，官方推荐集与额度兜底条目都不应展示。
#[test]
fn automatic_routing_hides_official_gpt_and_reserve_without_gpt_capability() {
    let fallback = vec![
        "gpt-6-astra".to_string(),
        "gpt-5.6-sol".to_string(),
        "gpt-reserve".to_string(),
        "gpt-image-2.5".to_string(),
        "codex-auto-review".to_string(),
    ];

    // 空账号池：官方推荐集与 gpt-reserve 都不展示，图片/内部条目保留。
    let empty_pool = super::automatic_api_service_pool_model_ids(&[], fallback.clone());
    assert!(
        !empty_pool
            .iter()
            .any(|model| model.eq_ignore_ascii_case("gpt-reserve")),
        "空账号池不应显示额度兜底条目: {empty_pool:?}"
    );
    assert!(
        !empty_pool
            .iter()
            .any(|model| model.eq_ignore_ascii_case("gpt-5.6-sol")),
        "空账号池不应显示官方推荐集: {empty_pool:?}"
    );
    assert!(
        empty_pool
            .iter()
            .any(|model| model.eq_ignore_ascii_case("gpt-image-2.5")),
        "图片模型与内部条目仍需保留: {empty_pool:?}"
    );
}

#[test]
fn gpt_pool_capability_ignores_grok_and_api_key_accounts() {
    // 普通 API Key（DeepSeek）不算官方 GPT 能力。
    let deepseek = automatic_routing_account("deepseek", "responses", &["deepseek-flash"]);
    // Grok 供应商账号（API Key + 绑定 Grok 平台账号）同样不算。
    let mut grok = automatic_routing_account("grok", "responses", &["grok-4.6"]);
    grok.upstream_grok_account_id = Some("grok-platform-1".to_string());

    assert!(
        !super::pool_provides_gpt_models(&[deepseek.clone(), grok.clone()]),
        "只包含 DeepSeek 与 Grok 的账号池不应被判定为具备官方 GPT 能力"
    );

    // 官方 OAuth 账号仍然提供官方 GPT 能力。
    let official = CodexAccount::new(
        "official".to_string(),
        "official@example.com".to_string(),
        CodexTokens {
            id_token: "id-token".to_string(),
            access_token: "access-token".to_string(),
            refresh_token: None,
        },
    );
    assert!(super::pool_provides_gpt_models(&[official]));
}

/// 生图转发账号不承接对话模型：只加 DeepSeek / Grok 的池即使绑定了 OAuth 生图账号，
/// 也不能判定为具备官方 GPT 能力。
#[test]
fn conversation_pool_excludes_image_generation_only_accounts() {
    let mut collection = automatic_routing_collection();
    collection.account_ids = vec!["deepseek".to_string(), "grok".to_string()];
    collection.image_generation_account_ids = vec!["gpt-image".to_string()];

    assert_eq!(
        super::conversation_sidecar_account_ids(&collection),
        vec!["deepseek", "grok"],
        "生图账号不能进入对话账号范围"
    );
    // sidecar 账号清单仍需包含生图账号，否则拿不到转发凭据。
    assert!(
        super::effective_sidecar_account_ids(&collection)
            .iter()
            .any(|account_id| account_id == "gpt-image"),
        "生图账号必须保留在 sidecar 账号清单里"
    );

    // 每个 API Key 自己的账号范围属于对话账号。
    let mut scoped_key = build_local_access_api_key(Some("Scoped"));
    scoped_key.inherit_account_pool = Some(false);
    scoped_key.account_ids = vec!["key-only".to_string()];
    collection.api_keys = vec![scoped_key];
    assert!(
        super::conversation_sidecar_account_ids(&collection)
            .iter()
            .any(|account_id| account_id == "key-only"),
        "API Key 自定义账号池属于对话账号范围"
    );

    // 同一账号同时出现在对话池与生图池时，仍按对话账号参与判断。
    collection.account_ids.push("gpt-image".to_string());
    assert!(
        super::conversation_sidecar_account_ids(&collection)
            .iter()
            .any(|account_id| account_id == "gpt-image"),
        "同时在对话池里的账号不能因为出现在生图池而被排除"
    );
}

/// 第三方 GPT 中转向账号（客户端名与上游名都是 GPT 家族）要按官方 GPT 能力处理，
/// 官方 DeepSeek 的目录壳位（客户端名是 GPT、上游是 deepseek-*）不能。
#[test]
fn gpt_relay_api_key_accounts_provide_gpt_models() {
    let relay = automatic_routing_account(
        "gpt-relay",
        "responses",
        &["gpt-5.5", "gpt-5.6-luna", "gpt-image-2", "custom-model"],
    );
    assert!(
        super::pool_provides_gpt_models(std::slice::from_ref(&relay)),
        "第三方 GPT 中转账号应提供官方 GPT 能力"
    );

    // 带命名空间前缀的上游名同样按 GPT 家族识别。
    let namespaced = automatic_routing_account("namespaced", "chat_completions", &["openai/gpt-5.5"]);
    assert!(super::pool_provides_gpt_models(std::slice::from_ref(&namespaced)));

    // 目录为空 + Responses 直通：仍按官方名称透传。
    let passthrough = automatic_routing_account("passthrough", "responses", &[]);
    assert!(super::pool_provides_gpt_models(std::slice::from_ref(&passthrough)));

    // DeepSeek 壳位：客户端名是 gpt-5.5、上游是 deepseek-flash，不算 GPT 能力。
    let mut shell = automatic_routing_account("deepseek-shell", "responses", &["gpt-5.5"]);
    shell.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "gpt-5.5".into(),
        upstream_model: "deepseek-flash".into(),
    }];
    assert!(
        !super::pool_provides_gpt_models(std::slice::from_ref(&shell)),
        "DeepSeek 目录壳位不应被当成官方 GPT 能力"
    );
}

/// 账号自己的 GPT 模型清单要追加进 profile 目录，壳位别名仍然不展示。
#[test]
fn profile_extra_models_include_account_owned_gpt_models() {
    let mut collection = automatic_routing_collection();
    let mut relay = automatic_routing_account(
        "gpt-relay",
        "responses",
        &["gpt-5.5", "gpt-5.6-luna", "gpt-4o", "custom-model"],
    );
    relay.api_provider_id = Some("custom".to_string());
    let mut shell = automatic_routing_account("deepseek-shell", "responses", &["gpt-5.6-sol"]);
    shell.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "gpt-5.6-sol".into(),
        upstream_model: "deepseek-v4-pro".into(),
    }];
    let accounts = vec![relay, shell];
    collection.account_ids = accounts.iter().map(|account| account.id.clone()).collect();
    let api_key = super::ResolvedLocalApiKey {
        id: "default".to_string(),
        label: "Default".to_string(),
        provider_gateway: None,
        inherit_account_pool: true,
        account_ids: collection.account_ids.clone(),
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        token_limit: None,
        token_used: 0,
    };

    let models = super::automatic_api_service_profile_extra_models(&collection, &api_key, &accounts)
        .into_iter()
        .map(|(model, _)| model)
        .collect::<Vec<_>>();

    assert!(models.iter().any(|model| model == "gpt-5.5"));
    assert!(models.iter().any(|model| model == "gpt-5.6-luna"));
    assert!(models.iter().any(|model| model == "custom-model"));
    assert!(
        !models.iter().any(|model| model == "gpt-4o"),
        "不在官方推荐集里的 GPT 名字不展示: {models:?}"
    );
    assert!(
        !models.iter().any(|model| model == "gpt-5.6-sol"),
        "壳位别名不应进入目录: {models:?}"
    );
}
