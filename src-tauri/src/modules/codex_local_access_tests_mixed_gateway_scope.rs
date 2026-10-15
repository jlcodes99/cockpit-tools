// Mixed gateway credentials must not depend on the API Service account pool.

fn mixed_scope_fixture() -> (CodexAccount, super::CodexInstanceModelRouting) {
    let mut oauth = test_account_with_plan("free");
    oauth.tokens.refresh_token = Some("mixed-test-refresh".into());
    let mut provider = CodexAccount::new_api_key(
        "mixed-provider".into(),
        "mixed@example.test".into(),
        "sk-mixed-test".into(),
        CodexApiProviderMode::Custom,
        Some("https://example.test/v1".into()),
        Some("mixed-provider".into()),
        Some("Mixed provider".into()),
        vec!["text-model".into()],
    );
    provider.api_wire_api = Some("chat_completions".into());
    // Deliberately no index entries: load_account can succeed while an index
    // snapshot during reauthorization does not yet contain the account.
    crate::modules::codex_account::save_account(&oauth).unwrap();
    crate::modules::codex_account::save_account(&provider).unwrap();
    let routing = super::CodexInstanceModelRouting {
        enabled: true,
        version: 1,
        routes: vec![super::CodexInstanceApiRoute {
            id: "route-1".into(),
            namespace: "third-party".into(),
            provider_account_id: provider.id,
            enabled: true,
            selected_models: Some(vec!["text-model".into()]),
            extra_models: None,
        }],
    };
    (oauth, routing)
}

#[test]
fn mixed_gateway_scope_survives_free_policy_and_index_rebuild() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = LocalAccessTestDataGuard::new("mixed-gateway-scope");
    let (oauth, routing) = mixed_scope_fixture();
    let profile = env.data_dir.join("profile");
    let (collection, key) =
        super::build_mixed_model_gateway_collection_for_profile(&profile, &oauth, &routing)
            .unwrap();
    assert_eq!(collection.api_keys[0].account_ids, vec![oauth.id.clone()]);
    assert_eq!(
        collection.bound_oauth_account_id.as_deref(),
        Some(oauth.id.as_str())
    );
    assert_eq!(collection.api_keys[0].inherit_account_pool, Some(false));
    assert_eq!(
        super::sidecar_client_api_keys(&collection, &HashMap::new()),
        vec![key.clone()]
    );
    let manifest = super::sidecar_api_key_manifest_values(&collection);
    assert_eq!(manifest.len(), 1);
    assert_eq!(manifest[0]["key"], key);
    assert_eq!(manifest[0]["accountIds"], json!([oauth.id]));
    assert_eq!(
        manifest[0]["modelRouting"]["routes"][0]["namespace"],
        "third-party"
    );
    let (_, rebuilt_key) =
        super::build_mixed_model_gateway_collection_for_profile(&profile, &oauth, &routing)
            .unwrap();
    assert_eq!(
        rebuilt_key, key,
        "rebuild must reuse the profile credential"
    );
}

#[test]
fn mixed_gateway_key_survives_missing_oauth_without_broadening_its_scope() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = LocalAccessTestDataGuard::new("mixed-gateway-missing-oauth");
    let (oauth, routing) = mixed_scope_fixture();
    let (collection, key) = super::build_mixed_model_gateway_collection_for_profile(
        &env.data_dir.join("profile"),
        &oauth,
        &routing,
    )
    .unwrap();
    fs::remove_file(
        env.data_dir
            .join("codex_accounts")
            .join(format!("{}.json", oauth.id)),
    )
    .unwrap();
    assert!(crate::modules::codex_account::load_account(&oauth.id).is_none());
    let launch = super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        env.data_dir.join("sidecar"),
        HashMap::new(),
        None,
        HashMap::new(),
        false,
        None,
    )
    .unwrap();
    let config: Value = serde_json::from_slice(&fs::read(&launch.config_path).unwrap()).unwrap();
    let manifest: Value =
        serde_json::from_slice(&fs::read(&launch.manifest_path).unwrap()).unwrap();
    assert_eq!(config["api-keys"], json!([key]));
    assert_eq!(config["api-key-account-ids"][&key],
        json!([super::sidecar_auth_file_name(&oauth.id)]));
    assert_eq!(manifest["apiKeys"][0]["accountIds"], json!([oauth.id]));
    assert_eq!(
        manifest["apiKeys"][0]["modelRouting"]["routes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(manifest["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|account| account["id"] != oauth.id));
    assert!(!config["api-keys"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == super::internal_api_service_key()));
    let mut disabled = collection.clone();
    disabled.api_keys[0].enabled = false;
    assert!(super::sidecar_client_api_keys(&disabled, &HashMap::new()).is_empty());
    let mut ordinary = collection;
    ordinary.api_keys[0].model_routing = None;
    assert!(super::sidecar_client_api_keys(&ordinary, &HashMap::new()).is_empty());
}
