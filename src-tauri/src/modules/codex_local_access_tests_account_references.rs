#[test]
fn key_rotation_preserves_public_key_scopes_and_pool_settings() {
    let mut collection = test_local_access_collection(vec!["old".into(), "other".into()]);
    collection.enabled = false;
    let mut key = build_local_access_api_key(Some("scope"));
    key.account_ids = vec!["old".into(), "other".into()];
    key.priority_account_ids = vec!["old".into()];
    key.preferred_account_id = Some("old".into());
    key.inherit_account_pool = Some(false);
    let bearer = key.key.clone();
    collection.api_keys = vec![key];
    collection.custom_routing_rules = vec![CodexLocalAccessCustomRoutingRule {
        account_id: "old".into(),
        priority: 3,
        weight: 2,
        is_backup: true,
        is_preferred: false,
    }];
    collection.account_model_rules = vec![CodexLocalAccessAccountModelRule {
        account_id: "old".into(),
        excluded_models: vec!["hidden".into()],
    }];
    assert!(super::replace_collection_account_references(
        &mut collection,
        "old",
        "new"
    ));
    assert_eq!(collection.account_ids, vec!["new", "other"]);
    assert_eq!(collection.api_keys[0].account_ids, vec!["new", "other"]);
    assert_eq!(collection.api_keys[0].priority_account_ids, vec!["new"]);
    assert_eq!(
        collection.api_keys[0].preferred_account_id.as_deref(),
        Some("new")
    );
    assert_eq!(collection.api_keys[0].key, bearer);
    assert!(!collection.enabled);
    assert_eq!(collection.api_keys[0].inherit_account_pool, Some(false));
    assert_eq!(collection.custom_routing_rules[0].account_id, "new");
    assert_eq!(collection.account_model_rules[0].account_id, "new");
    assert_eq!(
        collection.account_model_rules[0].excluded_models,
        vec!["hidden"]
    );
    assert!(!super::replace_collection_account_references(
        &mut collection,
        "old",
        "new"
    ));
}

#[test]
fn full_key_edit_migrates_persisted_references_and_failed_edits_remain_retryable() {
    let shared = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    shared.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("key-edit-reference-integration");
    let account = codex_account::upsert_api_key_account(
        "old-test-key".into(),
        Some("https://relay.example/v1".into()),
        Some(CodexApiProviderMode::Custom),
        Some("relay".into()),
        Some("Relay".into()),
        vec!["model-a".into()],
        Some(true),
        Some("responses".into()),
        false,
        false,
        HashMap::new(),
        None,
        None,
        None,
    )
    .unwrap();
    let mut store = crate::models::InstanceStore::new();
    store.default_settings.bind_account_id =
        crate::modules::codex_instance::provider_gateway_bind_account_id(&account.id);
    store.default_settings.model_routing = Some(crate::models::CodexInstanceModelRouting {
        enabled: false,
        routes: vec![crate::models::CodexInstanceApiRoute {
            id: "route".into(),
            namespace: "relay".into(),
            provider_account_id: account.id.clone(),
            enabled: true,
            selected_models: None,
            extra_models: None,
        }],
        ..Default::default()
    });
    crate::modules::codex_instance::save_instance_store(&store).unwrap();
    let mut collection = test_local_access_collection(vec![account.id.clone()]);
    collection.enabled = false;
    collection.api_keys = vec![build_local_access_api_key(Some("integration scope"))];
    collection.api_keys[0].account_ids = vec![account.id.clone()];
    collection.api_keys[0].priority_account_ids = vec![account.id.clone()];
    collection.api_keys[0].inherit_account_pool = Some(false);
    let path = super::local_access_file_path().unwrap();
    fs::write(&path, "invalid interrupted config").unwrap();
    let edit = || {
        codex_account::update_api_key_credentials(
            &account.id,
            "new-test-key".into(),
            account.api_base_url.clone(),
            Some(CodexApiProviderMode::Custom),
            account.api_provider_id.clone(),
            account.api_provider_name.clone(),
            account.api_model_catalog.clone(),
            Some(true),
            Some("responses".into()),
            false,
            false,
            HashMap::new(),
            None,
            None,
            None,
        )
    };
    assert!(edit().unwrap_err().contains("账号池引用"));
    assert!(
        codex_account::load_account(&account.id).is_some(),
        "failed migration keeps old credentials readable"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "invalid interrupted config"
    );
    super::save_collection_to_disk(&collection).unwrap();
    let updated = edit().expect("retry after restoring account-pool config");
    assert_ne!(updated.id, account.id);
    assert!(codex_account::load_account(&account.id).is_none());
    let store = crate::modules::codex_instance::load_instance_store().unwrap();
    assert_eq!(
        store.default_settings.bind_account_id,
        crate::modules::codex_instance::provider_gateway_bind_account_id(&updated.id)
    );
    assert_eq!(
        store.default_settings.model_routing.unwrap().routes[0].provider_account_id,
        updated.id
    );
    let collection = super::load_collection_from_disk().unwrap().unwrap();
    assert_eq!(collection.account_ids, vec![updated.id.clone()]);
    assert_eq!(collection.api_keys[0].account_ids, vec![updated.id.clone()]);
    assert_eq!(
        collection.api_keys[0].priority_account_ids,
        vec![updated.id]
    );
    assert!(
        !collection.enabled,
        "credential edits cannot enable the API service"
    );
}
