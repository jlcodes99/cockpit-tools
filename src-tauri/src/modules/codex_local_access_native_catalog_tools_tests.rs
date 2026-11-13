use super::*;
use crate::models::codex::CodexTokens;

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
fn anthropic_catalog_exposes_only_configured_claude_models_and_respects_key_scope() {
    let mut collection = collection();
    collection.account_ids = vec!["claude".into()];
    let mut claude = account("claude", "anthropic");
    claude.api_model_catalog = vec!["claude-sonnet-4-6".into(), "claude-opus-4-6".into()];
    claude.api_auth_scheme = Some("bearer".into());
    assert_eq!(
        native_wire_api(&claude).unwrap(),
        native_contract::WireApi::Anthropic
    );
    let catalog = native_catalog(&collection, None, &[claude.clone()]).unwrap();
    let models = catalog["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert!(models
        .iter()
        .all(|m| m["slug"].as_str().unwrap().starts_with("claude-")));
    assert_eq!(
        entry(&catalog, "claude-sonnet-4-6")["supports_search_tool"],
        true
    );
    let gateway = provider_gateway_for_account(&claude).unwrap();
    assert_eq!(gateway.wire_api.as_deref(), Some("anthropic"));
    assert_eq!(gateway.auth_scheme.as_deref(), Some("bearer"));
    let key: CodexLocalAccessApiKey = serde_json::from_value(json!({"id":"key","key":"fixture","label":"scope","inheritAccountPool":false,"accountIds":["unrelated"]})).unwrap();
    let denied = native_catalog(&collection, Some(&key), &[claude]).unwrap();
    assert!(!denied["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["slug"] == "claude-sonnet-4-6"));
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
    let mut accounts = vec![
        account("responses", "responses"),
        account("chat", "chat_completions"),
    ];
    accounts[0]
        .api_model_vision_support
        .insert("custom-agent".into(), true);
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
    for field in [
        "multi_agent_version",
        "supports_search_tool",
        "input_modalities",
        "supports_image_detail_original",
    ] {
        assert_eq!(
            saved["models"][0][field],
            entry(&served, "custom-agent")[field]
        );
    }
    assert_eq!(saved["models"][0]["display_name"], "keep display");
    assert_eq!(saved["models"][0]["context_window"], 12345);
    fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn mixed_profile_catalog_keeps_each_provider_context_and_saved_profile_override() {
    let mut collection = collection();
    let gateway = json!({"baseUrl":"https://example.invalid/v1","apiKey":"synthetic","upstreamModel":"custom-agent","upstreamModels":["custom-agent","unconfigured"],"wireApi":"responses"});
    collection.account_ids = vec!["official".into()];
    collection.api_keys = vec![serde_json::from_value(json!({
        "id":"mixed", "key":"synthetic-key", "label":"mixed", "inheritAccountPool":false,
        "accountIds":["official"],
        "modelRouting":{"defaultRoute":"oauth","failurePolicy":"strict","routes":[
            {"id":"first","namespace":"first","providerAccountId":"responses","providerGateway":gateway.clone()},
            {"id":"second","namespace":"second","providerAccountId":"chat","providerGateway":gateway}
        ]}
    })).unwrap()];
    let mut accounts = vec![
        account("responses", "responses"),
        account("chat", "responses"),
        CodexAccount::new("official".into(), "fixture@example.invalid".into(), CodexTokens {
            id_token: "fixture-id".into(), access_token: "fixture-access".into(), refresh_token: None,
        }),
    ];
    accounts[0].api_model_context_windows.insert("Custom-Agent".into(), 204_800);
    accounts[1].api_model_context_windows.insert("custom-agent".into(), 1_000_000);
    let profile = tempfile::tempdir().unwrap();
    let path = profile.path().join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    let ids = ["first/custom-agent", "second/custom-agent", "gpt-6.1-sol", "first/unconfigured"];
    let original = codex_protocol::build_codex_client_models_response(&ids.map(String::from));
    fs::write(&path, original.to_string()).unwrap();

    native_sync_profile_catalog_tool_capabilities(profile.path(), &collection, "synthetic-key", &accounts).await.unwrap();
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let served = native_catalog(&collection, collection.api_keys.first(), &accounts).unwrap();
    for (id, window) in [(ids[0], 204_800), (ids[1], 1_000_000)] {
        let written = entry(&saved, id);
        assert_eq!(written["context_window"], window);
        assert_eq!(written["max_context_window"], window);
        assert_eq!(written["auto_compact_token_limit"], window * 9 / 10);
        assert_eq!(written["context_window"], entry(&served, id)["context_window"]);
    }
    assert_eq!(entry(&saved, ids[2])["context_window"], entry(&original, ids[2])["context_window"]);
    assert_eq!(entry(&saved, ids[3])["context_window"], entry(&original, ids[3])["context_window"]);

    let definitions = serde_json::from_value(json!([
        {"model_id": ids[0], "display_name": "First", "context_window": 262144, "auto_compact_token_limit": 200000},
        {"model_id": ids[1], "display_name": "Second"}
    ])).unwrap();
    codex_account::save_quick_config_for_base_dir_with_default(profile.path(), None, None, Some(true), Some(definitions), None).unwrap();
    accounts[1].api_model_context_windows.insert("custom-agent".into(), 500_000);
    native_sync_profile_catalog_tool_capabilities(profile.path(), &collection, "synthetic-key", &accounts).await.unwrap();
    let rebuilt: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(entry(&rebuilt, ids[0])["context_window"], 262_144);
    assert_eq!(entry(&rebuilt, ids[0])["auto_compact_token_limit"], 200_000);
    assert_eq!(entry(&rebuilt, ids[1])["context_window"], 500_000);
}

#[tokio::test]
async fn model_tool_contract_tracks_eligible_routes_on_refresh() {
    let mut collection = collection();
    let official = CodexAccount::new(
        "official".into(),
        "official@example.invalid".into(),
        CodexTokens {
            id_token: "fixture-id".into(),
            access_token: "fixture-access".into(),
            refresh_token: None,
        },
    );
    let mut provider = account("responses", "responses");
    provider.api_model_catalog.push("gpt-5.6-sol".into());
    let directory = tempfile::tempdir().unwrap();
    let catalog_path = directory.path().join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    let model_id = "gpt-5.6-sol";
    let mut original = codex_protocol::build_codex_client_models_response(&[model_id.into()]);
    original["models"][0]["context_window"] = json!(12345);
    fs::write(&catalog_path, original.to_string()).unwrap();

    let accounts = [official, provider];
    for (ids, expects_lite, tool_mode) in [
        (vec!["responses"], false, "direct"),
        (vec!["official", "responses"], false, "direct"),
        (vec!["official"], true, "code_mode_only"),
        (vec!["official", "responses"], false, "direct"),
    ] {
        collection.account_ids = ids.into_iter().map(str::to_owned).collect();
        let served = native_catalog(&collection, None, &accounts).unwrap();
        let served_model = entry(&served, model_id);
        assert_eq!(served_model["use_responses_lite"], expects_lite);
        assert_eq!(served_model["tool_mode"], tool_mode);
        assert_eq!(served_model["shell_type"], "shell_command");
        native_sync_profile_catalog_tool_capabilities(
            directory.path(),
            &collection,
            &collection.api_key,
            &accounts,
        )
        .await
        .unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
        let saved_model = entry(&saved, model_id);
        assert_eq!(saved_model["use_responses_lite"], expects_lite);
        assert_eq!(saved_model["tool_mode"], tool_mode);
        assert_eq!(saved_model["shell_type"], "shell_command");
        assert_eq!(saved_model["context_window"], 12345);
    }
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

#[test]
fn kimi_k3_uses_standard_shell_tools_in_the_provider_catalog() {
    let mut kimi = account("chat", "chat_completions");
    kimi.api_sync_model_catalog_to_codex = true;
    kimi.api_model_catalog = vec!["kimi-k3".into()];
    let slots = provider_model_slots_for_account(&kimi, &kimi.api_model_catalog);
    assert_eq!(slots[0].upstream_model, "kimi-k3");

    let written: Value =
        serde_json::from_str(&build_provider_model_catalog_json(&slots).unwrap()).unwrap();
    let slot = entry(&written, &slots[0].client_model);
    assert_eq!(slot["display_name"], "kimi-k3");
    assert_eq!(slot["tool_mode"], "direct");
    assert_eq!(slot["shell_type"], "shell_command");
    assert_eq!(slot["use_responses_lite"], false);

    let served = native_catalog(&collection(), None, &[kimi]).unwrap();
    let model = entry(&served, "kimi-k3");
    assert_ne!(model["tool_mode"], "code_mode_only");
    assert_eq!(model["shell_type"], "shell_command");
    assert_eq!(model["use_responses_lite"], false);
}

#[test]
fn provider_gateway_sol_and_older_shells_keep_standard_terminal_tools() {
    let slots = [
        super::ProviderGatewayModelSlot {
            client_model: "gpt-5.6-sol".into(),
            upstream_model: "third-party-a".into(),
        },
        super::ProviderGatewayModelSlot {
            client_model: "gpt-5.4-mini".into(),
            upstream_model: "third-party-b".into(),
        },
    ];
    let catalog: Value = serde_json::from_str(
        &super::build_provider_model_catalog_json(&slots).unwrap(),
    ).unwrap();
    let models = catalog["models"].as_array().unwrap();
    let official = super::codex_protocol::known_codex_model_template("gpt-5.6-sol").unwrap();
    let sol = models.iter().find(|model| model["slug"] == "gpt-5.6-sol").unwrap();
    assert_eq!(sol["display_name"], "third-party-a");
    assert_eq!(sol["tool_mode"], "direct");
    assert_eq!(sol["shell_type"], "shell_command");
    assert_eq!(sol["use_responses_lite"], false);
    assert_eq!(sol["multi_agent_version"], official["multi_agent_version"]);
    assert_eq!(sol["context_window"], official["context_window"]);

    let older = models.iter().find(|model| model["slug"] == "gpt-5.4-mini").unwrap();
    let baseline = super::codex_protocol::build_codex_client_models_response(&["gpt-5.4-mini".into()]);
    assert_eq!(older["tool_mode"], baseline["models"][0]["tool_mode"]);
    assert_eq!(older["shell_type"], baseline["models"][0]["shell_type"]);
}
