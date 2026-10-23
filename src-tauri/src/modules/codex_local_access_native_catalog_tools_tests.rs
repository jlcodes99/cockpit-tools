use super::*;

fn collection() -> CodexLocalAccessCollection {
    serde_json::from_value(json!({"enabled":true,"gatewayMode":"native","port":19001,"apiKey":"synthetic-key","accountIds":["responses","chat"],"createdAt":0,"updatedAt":0})).unwrap()
}
fn account(id: &str, wire: &str) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        id.into(),
        "synthetic@example.invalid".into(),
        "synthetic-upstream".into(),
        CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        None,
        None,
        vec!["custom-agent".into(), "gpt-6.1-sol".into()],
    );
    account.api_wire_api = Some(wire.into());
    account
}
fn entry(catalog: &Value, id: &str) -> Value {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["slug"] == id)
        .unwrap()
        .clone()
}

#[test]
fn native_catalog_registers_visible_model_agents_despite_legacy_disabled_setting() {
    let mut collection = collection();
    collection.multi_agent.optimize = false;
    let account = account("responses", "responses");
    let catalog = native_catalog(&collection, None, &[account]).unwrap();
    for id in ["custom-agent", "gpt-6.1-sol"] {
        assert_eq!(entry(&catalog, id)["multi_agent_version"], "v2");
    }
    let plan = native_model_route_plan(&collection, None, "custom-agent").unwrap();
    let mut hidden = json!({"slug":"custom-agent","visibility":"hide","multi_agent_version":null});
    native_apply_catalog_tool_capabilities(&mut hidden, &collection, None, &plan, &[]);
    assert!(hidden["multi_agent_version"].is_null());
}

#[test]
fn native_catalog_search_considers_every_eligible_pool_member_not_first_metadata_source() {
    let collection = collection();
    for accounts in [
        vec![
            account("responses", "responses"),
            account("chat", "chat_completions"),
        ],
        vec![
            account("chat", "chat_completions"),
            account("responses", "responses"),
        ],
    ] {
        let catalog = native_catalog(&collection, None, &accounts).unwrap();
        assert_eq!(
            entry(&catalog, "custom-agent")["supports_search_tool"],
            false
        );
        assert_eq!(
            entry(&catalog, "gpt-6.1-sol")["supports_search_tool"],
            false
        );
    }
}

#[test]
fn native_catalog_search_respects_key_account_grants_and_model_prefix() {
    let collection = collection();
    let key:CodexLocalAccessApiKey=serde_json::from_value(json!({"id":"scoped","label":"scope","key":"synthetic-key","inheritAccountPool":false,"accountIds":["responses"],"modelPrefix":"team"})).unwrap();
    let accounts = vec![
        account("responses", "responses"),
        account("chat", "chat_completions"),
    ];
    let mut model = json!({"slug":"team/custom-agent","supports_search_tool":true});
    let public = native_model_for_key(&collection, Some(&key), "team/custom-agent").unwrap();
    let plan = native_model_route_plan(&collection, Some(&key), &public).unwrap();
    native_apply_catalog_tool_capabilities(&mut model, &collection, Some(&key), &plan, &accounts);
    assert_eq!(model["supports_search_tool"], true);
    let catalog = native_catalog(&collection, Some(&key), &accounts).unwrap();
    assert_eq!(
        entry(&catalog, "team/custom-agent")["multi_agent_version"],
        "v2"
    );
    assert!(catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .all(|model| model["slug"].as_str().unwrap().starts_with("team/")));
}

#[test]
fn native_catalog_search_keeps_plan_and_grok_adapters_and_ignores_other_models() {
    let mut collection = collection();
    let mut plan_account = account("plan", "chat_completions");
    plan_account.api_provider_id = Some("cockpit_chatgpt_plan".into());
    let mut unrelated = account("chat", "chat_completions");
    unrelated.api_model_catalog = vec!["unrelated".into()];
    collection.account_ids = vec!["plan".into(), "chat".into()];
    let plan = native_model_route_plan(&collection, None, "custom-agent").unwrap();
    let mut model = json!({"slug":"custom-agent","supports_search_tool":true});
    native_apply_catalog_tool_capabilities(
        &mut model,
        &collection,
        None,
        &plan,
        &[plan_account, unrelated],
    );
    assert_eq!(model["supports_search_tool"], true);
    let mut grok = account("grok", "chat_completions");
    grok.upstream_grok_account_id = Some("synthetic-grok".into());
    assert_eq!(
        native_wire_api(&grok).unwrap(),
        native_contract::WireApi::Responses
    );
    collection.account_ids = vec!["grok".into()];
    let plan = native_model_route_plan(&collection, None, "custom-agent").unwrap();
    native_apply_catalog_tool_capabilities(&mut model, &collection, None, &plan, &[grok]);
    assert_eq!(model["supports_search_tool"], true);
}

#[test]
fn native_catalog_search_follows_explicit_provider_and_namespaced_mixed_routes() {
    let collection = collection();
    let gateway = json!({"baseUrl":"https://example.invalid/v1","apiKey":"synthetic","upstreamModel":"custom-agent","upstreamModels":["custom-agent"],"wireApi":"chat_completions","supportsVision":false});
    let provider:CodexLocalAccessApiKey=serde_json::from_value(json!({"id":"provider","key":"synthetic","label":"provider","inheritAccountPool":false,"accountIds":["chat"],"providerGateway":gateway.clone()})).unwrap();
    let mixed:CodexLocalAccessApiKey=serde_json::from_value(json!({"id":"mixed","key":"synthetic","label":"mixed","inheritAccountPool":false,"accountIds":["responses"],"modelRouting":{"defaultRoute":"oauth","failurePolicy":"strict","routes":[{"id":"chat-route","namespace":"third","providerAccountId":"chat","providerGateway":gateway}]}})).unwrap();
    let accounts = vec![
        account("responses", "responses"),
        account("chat", "chat_completions"),
    ];
    assert_eq!(
        entry(
            &native_catalog(&collection, Some(&provider), &accounts).unwrap(),
            "custom-agent"
        )["supports_search_tool"],
        false
    );
    assert_eq!(
        entry(
            &native_catalog(&collection, Some(&mixed), &accounts).unwrap(),
            "third/custom-agent"
        )["supports_search_tool"],
        false
    );
}

#[tokio::test]
async fn native_profile_catalog_and_models_endpoint_share_identical_tool_capability_gates() {
    let mut collection = collection();
    collection.api_keys=vec![serde_json::from_value(json!({"id":"scoped","label":"scope","key":"synthetic-key","inheritAccountPool":false,"accountIds":["responses","chat"]})).unwrap()];
    let accounts = vec![
        account("responses", "responses"),
        account("chat", "chat_completions"),
    ];
    let directory =
        std::env::temp_dir().join(format!("cockpit-catalog-tools-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&directory).unwrap();
    let original = json!({"models":[{"slug":"custom-agent","display_name":"keep display","context_window":12345,"multi_agent_version":null,"supports_search_tool":true}]});
    fs::write(
        directory.join(CODEX_MANAGED_MODEL_CATALOG_FILE),
        original.to_string(),
    )
    .unwrap();
    native_sync_profile_catalog_tool_capabilities(
        &directory,
        &collection,
        "synthetic-key",
        &accounts,
    )
    .await
    .unwrap();
    let saved: Value = serde_json::from_str(
        &fs::read_to_string(directory.join(CODEX_MANAGED_MODEL_CATALOG_FILE)).unwrap(),
    )
    .unwrap();
    let served = native_catalog(&collection, collection.api_keys.first(), &accounts).unwrap();
    for field in ["multi_agent_version", "supports_search_tool"] {
        assert_eq!(
            saved["models"][0][field],
            entry(&served, "custom-agent")[field]
        );
    }
    assert_eq!(saved["models"][0]["display_name"], "keep display");
    assert_eq!(saved["models"][0]["context_window"], 12345);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn instance_gateway_catalog_ignores_retired_template_switches() {
    let mut template = collection();
    template.multi_agent.optimize = false;
    template.multi_agent.orphan_delegation = true;
    let mut profile = collection();
    apply_provider_gateway_template_settings(&mut profile, &template);
    let catalog = native_catalog(&profile, None, &[account("responses", "responses")]).unwrap();
    assert_eq!(entry(&catalog, "custom-agent")["multi_agent_version"], "v2");
}
