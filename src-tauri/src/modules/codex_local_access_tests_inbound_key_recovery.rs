#[test]
fn mixed_model_gateway_keeps_client_api_key_when_bound_account_unlisted() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    shared_state.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("mixed-model-key-survives");
    let mut oauth_account = test_account_with_plan("plus");
    oauth_account.id = "oauth-main".to_string();
    oauth_account.tokens.refresh_token = Some("refresh-token".to_string());

    let mut collection = test_local_access_collection(vec![oauth_account.id.clone()]);
    collection.bound_oauth_account_id = Some(oauth_account.id.clone());
    let mut api_key = build_local_access_api_key(Some("Mixed Model Routing"));
    api_key.id = "mixed_model_routing".to_string();
    api_key.inherit_account_pool = Some(false);
    api_key.account_ids = vec![oauth_account.id.clone()];
    api_key.model_routing = Some(CodexLocalAccessModelRouting {
        default_route: "oauth".to_string(),
        failure_policy: "strict".to_string(),
        routes: Vec::new(),
    });
    collection.api_key = api_key.key.clone();
    collection.api_keys = vec![api_key];

    // An interrupted re-auth can leave the bound OAuth account temporarily
    // unresolvable in the account listing. Sanitize must not empty the only
    // client key's scope, and the writer must still resolve the key through
    // account overrides instead of persisting `"api-keys": []`.
    sanitize_collection_with_accounts(&mut collection, &[]).expect("sanitize collection");
    assert_eq!(
        collection.api_keys[0].account_ids,
        vec![oauth_account.id.clone()],
        "the only client key must keep its declared scope"
    );

    let dir = make_temp_dir("mixed-model-key-survives");
    let overrides = HashMap::from([(oauth_account.id.clone(), oauth_account.clone())]);
    super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        overrides,
        false,
        None,
    )
    .expect("prepare instance gateway sidecar config");

    let config: Value = serde_json::from_str(
        &fs::read_to_string(super::sidecar_config_path(&dir)).expect("read sidecar config"),
    )
    .expect("parse sidecar config");
    let api_keys = config
        .get("api-keys")
        .and_then(Value::as_array)
        .expect("api-keys should be an array");
    assert_eq!(
        api_keys.len(),
        1,
        "instance gateway must keep its client key: {config}"
    );
    assert_eq!(
        api_keys[0].as_str(),
        Some(collection.api_keys[0].key.as_str())
    );
    let scopes = config
        .get("api-key-account-ids")
        .and_then(Value::as_object)
        .expect("api-key-account-ids should be an object");
    assert_eq!(
        scopes.len(),
        1,
        "the client key must keep its auth scope mapping: {config}"
    );

    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(super::sidecar_manifest_path(&dir)).expect("read sidecar manifest"),
    )
    .expect("parse sidecar manifest");
    assert_eq!(
        manifest
            .get("apiKeys")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1),
        "manifest must keep advertising the client key: {manifest}"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn sidecar_preparation_refuses_to_persist_empty_api_keys() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    shared_state.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("refuse-empty-api-keys");
    let mut collection = test_local_access_collection(Vec::new());
    let mut api_key = build_local_access_api_key(Some("Mixed Model Routing"));
    api_key.inherit_account_pool = Some(false);
    api_key.account_ids = vec!["vanished-account".to_string()];
    collection.api_keys = vec![api_key];

    let dir = make_temp_dir("refuse-empty-api-keys");
    let error = super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        HashMap::new(),
        false,
        None,
    )
    .expect_err("a config without resolvable client keys must not be persisted");

    assert!(
        error.contains("API Key"),
        "error should explain the missing inbound keys: {error}"
    );
    assert!(
        !super::sidecar_config_path(&dir).exists(),
        "no sidecar config may be written when every declared client key is unresolved"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn inbound_key_keeps_partial_missing_scope_until_explicit_removal() {
    let mut eligible = test_account_with_plan("plus");
    eligible.id = "available-account".to_string();
    let mut ineligible = eligible.clone();
    ineligible.id = "known-ineligible".to_string();
    ineligible.tokens.access_token.clear();
    ineligible.tokens.refresh_token = None;
    let mut collection = test_local_access_collection(vec![eligible.id.clone()]);
    let mut api_key = build_local_access_api_key(Some("Recovery"));
    api_key.inherit_account_pool = Some(false);
    api_key.account_ids = vec![
        eligible.id.clone(),
        "temporarily-missing".to_string(),
        ineligible.id.clone(),
    ];
    collection.api_keys = vec![api_key];
    sanitize_collection_with_accounts(&mut collection, &[eligible, ineligible]).unwrap();
    assert_eq!(
        collection.api_keys[0].account_ids,
        vec!["available-account", "temporarily-missing"]
    );
    assert!(remove_account_refs_from_collection(
        &mut collection,
        &HashSet::from(["temporarily-missing".to_string()])
    ));
    assert_eq!(
        collection.api_keys[0].account_ids,
        vec!["available-account"]
    );
}

#[test]
fn unresolved_public_key_preserves_last_good_files_even_with_internal_key() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _internal_accounts = InternalApiAccountIdsGuard::new();
    shared_state.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("inbound-key-preserve-files");
    super::register_internal_api_account(INTERNAL_SERVICE_TEST_ACCOUNT_ID).unwrap();
    let overrides = HashMap::from([(
        INTERNAL_SERVICE_TEST_ACCOUNT_ID.to_string(),
        internal_service_test_account(),
    )]);
    let mut collection = test_local_access_collection(Vec::new());
    let mut api_key = build_local_access_api_key(Some("Recovery"));
    api_key.inherit_account_pool = Some(false);
    api_key.account_ids = vec!["temporarily-missing".to_string()];
    collection.api_keys = vec![api_key];
    // A resolvable internal key cannot stand in for public authentication.
    assert!(
        !super::sidecar_client_api_keys_with_internal(&collection, &overrides, true).is_empty()
    );
    let dir = make_temp_dir("inbound-key-preserve-files");
    let config_path = super::sidecar_config_path(&dir);
    let auth_dir = dir.join("auths");
    fs::create_dir_all(&auth_dir).unwrap();
    let auth_path = auth_dir.join("last-good.json");
    fs::write(&config_path, b"last good synthetic config").unwrap();
    fs::write(&auth_path, b"synthetic auth must survive").unwrap();
    let error = super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        overrides,
        true,
        None,
    )
    .expect_err("unresolved public key must fail before any writes or cleanup");
    assert!(error.contains("API Key"));
    assert_eq!(
        fs::read(&config_path).unwrap(),
        b"last good synthetic config"
    );
    assert_eq!(
        fs::read(&auth_path).unwrap(),
        b"synthetic auth must survive"
    );
    assert!(!super::sidecar_manifest_path(&dir).exists());
    let _ = fs::remove_dir_all(dir);
}

#[tokio::test]
async fn mixed_gateway_restart_reuses_key_and_publishes_bound_port_after_reauth() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("mixed-native-reauth");
    let root = tempfile::tempdir().unwrap();
    let profile = root.path();
    let oauth_id = "oauth-mixed-fixture";
    let key = "mixed-profile-fixture-key";
    let runtime_id = super::MIXED_MODEL_ROUTING_RUNTIME_ID;
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let old_port = occupied.local_addr().unwrap().port();
    super::save_provider_gateway_profile_state(
        profile,
        runtime_id,
        &super::ProviderGatewayProfileState {
            api_key: key.into(),
            port: Some(old_port),
            created_at: 1,
            updated_at: 1,
        },
    )
    .unwrap();
    let config_path = super::profile_config_path(profile);
    let original = b"model_provider = 'openai'\n";
    fs::write(&config_path, original).unwrap();
    let provider = CodexAccount::new_api_key(
        "mixed-provider-fixture".into(),
        "relay@example.test".into(),
        "fixture-upstream-key".into(),
        CodexApiProviderMode::Custom,
        Some("https://relay.example.test/v1".into()),
        None,
        Some("Fixture relay".into()),
        vec!["fixture-model".into()],
    );
    crate::modules::codex_account::save_account(&provider).unwrap();
    let routing = super::CodexInstanceModelRouting {
        enabled: true,
        version: 1,
        routes: vec![super::CodexInstanceApiRoute {
            id: "fixture-route".into(),
            namespace: "relay".into(),
            provider_account_id: provider.id,
            enabled: true,
            selected_models: None,
            extra_models: None,
        }],
    };
    let error = super::ensure_mixed_model_gateway_for_dir(profile, oauth_id, &routing)
        .await
        .unwrap_err();
    assert!(error.contains(oauth_id), "{error}");
    assert_eq!(fs::read(&config_path).unwrap(), original);

    let oauth = CodexAccount::new(
        oauth_id.into(),
        "oauth@example.test".into(),
        CodexTokens {
            id_token: make_test_jwt(json!({"email":"oauth@example.test","exp":4_102_444_800i64})),
            access_token: make_test_jwt(json!({"sub":"oauth-fixture","exp":4_102_444_800i64})),
            refresh_token: Some("fixture-refresh".into()),
        },
    );
    crate::modules::codex_account::save_account(&oauth).unwrap();
    super::ensure_mixed_model_gateway_for_dir(profile, oauth_id, &routing)
        .await
        .unwrap();

    let runtime_key = super::provider_gateway_runtime_key(profile, runtime_id);
    let port = {
        let runtimes = super::provider_gateway_runtime_store().lock().await;
        let runtime = &runtimes[&runtime_key];
        assert_eq!(runtime.oauth_account_ids, [oauth_id]);
        let collection = runtime.collection.as_ref().unwrap();
        assert_eq!(collection.api_keys[0].account_ids, [oauth_id]);
        assert_eq!(collection.api_key, key);
        runtime.actual_port.unwrap()
    };
    let persisted = super::load_provider_gateway_profile_state(profile, runtime_id)
        .unwrap()
        .unwrap();
    assert_ne!(port, old_port);
    assert_eq!(persisted.port, Some(port));
    assert_eq!(persisted.api_key, key);
    let doc = fs::read_to_string(&config_path)
        .unwrap()
        .parse::<Document>()
        .unwrap();
    let provider_config = &doc["model_providers"]["codex_local_access"];
    let base_url = provider_config["base_url"].as_str().unwrap();
    assert_eq!(url::Url::parse(base_url).unwrap().port(), Some(port));
    assert_eq!(provider_config["experimental_bearer_token"].as_str(), Some(key));

    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{base_url}/models"))
        .bearer_auth(key)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let catalog: Value = response.json().await.unwrap();
    assert!(catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|model| model["id"] == "relay/fixture-model"));
    super::stop_provider_gateway_runtime(&runtime_key).await.unwrap();
}
