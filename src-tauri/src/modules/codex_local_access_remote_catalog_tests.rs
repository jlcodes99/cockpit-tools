use super::*;

fn remote(catalog: Value, models: Vec<String>) -> crate::modules::codex_remote_service::Connection {
    serde_json::from_value(json!({"product":"cockpit-tools","protocolVersion":1,"serviceId":"catalog-fixture",
        "serviceName":"Cockpit Tools","keyLabel":"B","status":"active","models":models,"imageModels":[],
        "modelCatalog":catalog,"keyViewsVersion":1,
        "capabilities":{"responses":true,"images":false,"imageEdits":false,"hostedImages":false,"usage":true,"websockets":true},
        "tokenLimit":null,"tokenUsed":0,"budget":{"policy":{"period":"lifetime"},"usedMicrousd":0},"updatedAt":1})).unwrap()
}

#[test]
fn remote_catalog_roundtrip_preserves_host_names_order_and_reasoning_with_key_prefix() {
    let mut definitions = local_gateway_visible_gpt_model_definitions()
        .into_iter()
        .map(|(id, name)| ProfileModelDefinition::plain(&id, &name))
        .collect::<Vec<_>>();
    definitions.push(ProfileModelDefinition {
        model_id: "deepseek-flash".into(),
        display_name: "DeepSeek-V4.1-Flash".into(),
        template: codex_protocol::deepseek_model_template("deepseek-flash"),
        image_capable: false,
    });
    let mut host = build_local_access_profile_catalog(&definitions);
    for model in host["models"].as_array_mut().unwrap() {
        model["base_instructions"] = json!("PRIVATE INSTRUCTIONS");
        model["provider_credentials"] = json!("SECRET CREDENTIALS");
    }
    let allowed = vec![
        "team/deepseek-flash".into(),
        "team/gpt-6.1-sol".into(),
        "team/gpt-reserve".into(),
    ];
    let public = project_public_model_catalog(&host, &allowed, "team/");
    let mut connection = remote(public.clone(), allowed);
    // Metadata cannot add unauthorized models or executable configuration.
    connection.model_catalog.as_mut().unwrap()["models"]
        .as_array_mut()
        .unwrap()
        .push(json!({"slug":"private-model","display_name":"SECRET ACCOUNT"}));
    let client = project_remote_model_catalog(&connection).unwrap();
    assert_eq!(client["models"].as_array().unwrap().len(), 3);
    assert_eq!(client["models"][0]["slug"], "team/gpt-6.1-sol");
    for model in public["models"].as_array().unwrap() {
        let reconstructed = client["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["slug"] == model["slug"])
            .unwrap();
        for field in REMOTE_MODEL_FIELDS {
            if let Some(expected) = model.get(*field) {
                assert_eq!(&reconstructed[*field], expected, "{field}");
            }
        }
    }
    let flash = client["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["slug"] == "team/deepseek-flash")
        .unwrap();
    assert_eq!(flash["display_name"], "DeepSeek-V4.1-Flash");
    for secret in [
        "PRIVATE INSTRUCTIONS",
        "SECRET CREDENTIALS",
        "SECRET ACCOUNT",
        "private-model",
    ] {
        assert!(!public.to_string().contains(secret));
        assert!(!client.to_string().contains(secret));
    }
}

#[test]
fn remote_catalog_replaces_stale_account_catalog_and_survives_profile_and_http_views() {
    let base = format!("http://192.0.2.10/{}/v1", uuid::Uuid::new_v4());
    let mut account = CodexAccount::new_api_key(
        "remote-catalog".into(),
        "fixture@invalid.test".into(),
        "remote-fixture".into(),
        CodexApiProviderMode::Custom,
        Some(base.clone()),
        None,
        None,
        vec!["outdated-model".into()],
    );
    account.api_wire_api = Some("responses".into());
    let connection = remote(
        json!({"models":[{"slug":"deepseek-flash","display_name":"DeepSeek-V4.1-Flash","priority":17,
        "context_window":123456,"supported_reasoning_levels":[{"effort":"max","description":"Maximum"}],"default_reasoning_level":"max"}]}),
        vec!["deepseek-flash".into()],
    );
    crate::modules::codex_remote_service::remember_test_connection(
        &base,
        "remote-fixture",
        connection,
    );
    assert_eq!(
        provider_gateway_models_for_account(&account),
        vec!["deepseek-flash"]
    );
    let dir = tempfile::tempdir().unwrap();
    let slots =
        provider_model_slots_for_account(&account, &provider_gateway_models_for_account(&account));
    write_provider_gateway_model_catalog_with_templates(dir.path(), &slots, None, Some(&account))
        .unwrap();
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(CODEX_PROVIDER_MODEL_CATALOG_FILE)).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["models"].as_array().unwrap().len(), 1);
    assert_eq!(saved["models"][0]["display_name"], "DeepSeek-V4.1-Flash");
    assert_eq!(saved["models"][0]["priority"], 17);
    assert_eq!(
        account_model_template(&account, "deepseek-flash").unwrap()["default_reasoning_level"],
        "max"
    );
    assert!(account_model_template(&account, "outdated-model").is_none());
    let collection: CodexLocalAccessCollection =
        serde_json::from_value(json!({"enabled":true,"port":0,"apiKey":"local-key",
        "accountIds":[account.id],"createdAt":1,"updatedAt":1}))
        .unwrap();
    let gateway = provider_gateway_for_account(&account).unwrap();
    assert!(native_gateway_model(&gateway, "gpt-5.6-luna", true).is_err());
    let key: CodexLocalAccessApiKey =
        serde_json::from_value(json!({"id":"B","key":"B-key","label":"B",
        "inheritAccountPool":false,"accountIds":[account.id],"providerGateway":gateway}))
        .unwrap();
    let gateway_catalog = native_catalog(&collection, Some(&key), &[account.clone()]).unwrap();
    assert_eq!(gateway_catalog["models"].as_array().unwrap().len(), 1);
    assert_eq!(gateway_catalog["models"][0]["slug"], "deepseek-flash");
    let catalog = native_catalog(&collection, None, &[account]).unwrap();
    let entry = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == "deepseek-flash")
        .unwrap();
    assert_eq!(entry["display_name"], "DeepSeek-V4.1-Flash");
    assert_eq!(entry["priority"], 17);
    assert_eq!(entry["context_window"], 123456);
    let data = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "deepseek-flash")
        .unwrap();
    assert_eq!(data["display_name"], "DeepSeek-V4.1-Flash");
}

#[tokio::test]
async fn remote_route_defers_installation_rewriting_to_the_final_host() {
    let base = format!("http://192.0.2.10/{}/v1", uuid::Uuid::new_v4());
    let account = CodexAccount::new_api_key(
        "remote-installation".into(),
        "fixture@invalid.test".into(),
        "remote-fixture".into(),
        CodexApiProviderMode::Custom,
        Some(base.clone()),
        None,
        None,
        vec!["gpt-6.1-sol".into()],
    );
    crate::modules::codex_remote_service::remember_test_connection(
        &base,
        "remote-fixture",
        remote(json!({"models":[]}), vec![]),
    );
    let collection: CodexLocalAccessCollection =
        serde_json::from_value(json!({"enabled":true,"port":0,"apiKey":"local-key",
        "accountIds":[account.id],"createdAt":1,"updatedAt":1}))
        .unwrap();
    let host = NativeHostBridge::for_collection(collection.clone());
    let route = native_route_for_reserved_account(
        &host,
        &collection,
        account,
        "gpt-6.1-sol".into(),
        false,
        None,
        false,
    )
    .await
    .unwrap();
    assert!(route.installation_profile.is_none());
    assert!(!route.headers.contains_key("x-codex-installation-id"));
}
