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
