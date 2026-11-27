#[test]
fn account_concurrency_defaults_to_two_and_preserves_saved_limits() {
    let mut value = serde_json::to_value(test_local_access_collection(Vec::new())).unwrap();
    value.as_object_mut().unwrap().remove("maxAccountConcurrency");
    let collection: super::CodexLocalAccessCollection =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(collection.max_account_concurrency, 2);

    for limit in [0, 2, 5] {
        value["maxAccountConcurrency"] = serde_json::json!(limit);
        let collection: super::CodexLocalAccessCollection =
            serde_json::from_value(value.clone()).unwrap();
        assert_eq!(collection.max_account_concurrency, limit);
    }

    // Unlike an explicit zero, an absent legacy field adopts the new default.
    value.as_object_mut().unwrap().remove("maxAccountConcurrency");
    value.as_object_mut().unwrap().remove("accountConcurrencyDefaultMigrated");
    let mut collection: super::CodexLocalAccessCollection = serde_json::from_value(value).unwrap();
    super::sanitize_collection_structure(&mut collection).unwrap();
    assert_eq!(collection.max_account_concurrency, 2);
    assert!(collection.account_concurrency_default_migrated);
}

#[test]
fn legacy_account_concurrency_preserves_explicit_zero_and_other_saved_limits() {
    for (previous, expected) in [(0, 0), (1, 1), (5, 5)] {
        let mut value = serde_json::to_value(test_local_access_collection(vec!["bound".into()])).unwrap();
        value.as_object_mut().unwrap().remove("accountConcurrencyDefaultMigrated");
        value["maxAccountConcurrency"] = serde_json::json!(previous);
        let mut collection: super::CodexLocalAccessCollection =
            serde_json::from_value(value).unwrap();
        assert!(!collection.account_concurrency_default_migrated);
        assert!(super::sanitize_collection_structure(&mut collection).unwrap());
        assert_eq!(collection.max_account_concurrency, expected);
        assert!(collection.account_concurrency_default_migrated);
        assert_eq!(collection.account_ids, vec!["bound".to_string()]);
        assert!(!super::sanitize_collection_structure(&mut collection).unwrap());

        // A later explicit choice of unlimited concurrency survives save/reload.
        collection.max_account_concurrency = 0;
        let mut reloaded: super::CodexLocalAccessCollection =
            serde_json::from_value(serde_json::to_value(collection).unwrap()).unwrap();
        assert!(!super::sanitize_collection_structure(&mut reloaded).unwrap());
        assert_eq!(reloaded.max_account_concurrency, 0);
    }
}

#[test]
fn new_collections_account_concurrency_defaults_to_two() {
    assert_eq!(
        super::new_local_access_collection().unwrap().max_account_concurrency,
        2
    );
    assert_eq!(
        super::new_empty_local_access_collection().unwrap().max_account_concurrency,
        2
    );
}

#[test]
fn provider_gateway_template_preserves_enabled_and_disabled_concurrency() {
    let mut instance = test_local_access_collection(vec!["bound".into()]);
    let mut template = test_local_access_collection(vec!["global".into()]);
    for (limit, wait_ms) in [(3, 1500), (0, 0)] {
        template.max_account_concurrency = limit;
        template.account_concurrency_wait_ms = wait_ms;
        super::apply_provider_gateway_template_settings(&mut instance, &template);
        assert_eq!(instance.max_account_concurrency, limit);
        assert_eq!(instance.account_concurrency_default_migrated, template.account_concurrency_default_migrated);
        assert_eq!(instance.account_concurrency_wait_ms, wait_ms);
        assert_eq!(instance.account_ids, vec!["bound".to_string()]);
    }
}

#[test]
fn per_account_concurrency_overrides_clamp_prune_and_persist() {
    let dir = make_temp_dir("codex-account-concurrency-limits");
    let path = dir.join("local_access.json");
    let mut collection = test_local_access_collection(vec!["acct-a".into(), "acct-b".into()]);
    collection.max_account_concurrency = 2;
    collection
        .account_concurrency_limits
        .insert("acct-a".into(), 99);
    collection
        .account_concurrency_limits
        .insert("removed".into(), 4);
    assert!(super::prune_account_concurrency_limits(&mut collection));
    assert_eq!(collection.account_concurrency_limits.get("acct-a"), Some(&64));
    assert!(!collection.account_concurrency_limits.contains_key("removed"));
    assert_eq!(
        super::effective_account_concurrency_limit(&collection, "acct-a"),
        64
    );
    assert_eq!(
        super::effective_account_concurrency_limit(&collection, "acct-b"),
        2
    );

    std::fs::write(&path, serde_json::to_string_pretty(&collection).unwrap()).unwrap();
    let updated = super::persist_account_concurrency_limit(&path, "acct-a", Some(1)).unwrap();
    assert_eq!(updated.account_concurrency_limits.get("acct-a"), Some(&1));
    let updated = super::persist_account_concurrency_limit(&path, "acct-b", Some(0)).unwrap();
    assert_eq!(updated.account_concurrency_limits.get("acct-b"), Some(&0));
    assert_eq!(
        super::effective_account_concurrency_limit(&updated, "acct-b"),
        0
    );
    assert!(super::persist_account_concurrency_limit(&path, "missing", Some(3)).is_err());
    let cleared = super::persist_account_concurrency_limit(&path, "acct-a", None).unwrap();
    assert!(!cleared.account_concurrency_limits.contains_key("acct-a"));
    assert_eq!(
        super::effective_account_concurrency_limit(&cleared, "acct-a"),
        2
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn native_concurrency_permit_enforces_distinct_per_account_limits_and_zero_unlimited() {
    let mut collection = test_local_access_collection(vec![
        "acct-one".into(),
        "acct-three".into(),
        "acct-unlimited".into(),
    ]);
    collection.max_account_concurrency = 2;
    collection.account_concurrency_wait_ms = 0;
    collection
        .account_concurrency_limits
        .insert("acct-one".into(), 1);
    collection
        .account_concurrency_limits
        .insert("acct-three".into(), 3);
    collection
        .account_concurrency_limits
        .insert("acct-unlimited".into(), 0);

    let host = super::NativeHostBridge::for_collection(collection.clone());
    let acct_one = test_oauth_account_with_quota("acct-one", 90, 90, Some(true), Some(true));
    let acct_three = test_oauth_account_with_quota("acct-three", 90, 90, Some(true), Some(true));
    let acct_unlimited =
        test_oauth_account_with_quota("acct-unlimited", 90, 90, Some(true), Some(true));

    // acct-one has limit 1: first acquires, second fails immediately.
    let (idx1, permit1) = super::native_acquire_account_concurrency_permit(
        &host,
        &collection,
        std::slice::from_ref(&acct_one),
        true,
    )
    .await
    .unwrap();
    assert_eq!(idx1, 0);
    assert!(permit1.is_some());
    assert!(super::native_acquire_account_concurrency_permit(
        &host,
        &collection,
        std::slice::from_ref(&acct_one),
        true,
    )
    .await
    .is_err());

    // Mixed pool [acct-one (full), acct-three (limit 3)] spills over to acct-three up to 3 times.
    let pool = vec![acct_one.clone(), acct_three.clone()];
    let mut three_permits = Vec::new();
    for _ in 0..3 {
        let (idx, permit) = super::native_acquire_account_concurrency_permit(
            &host,
            &collection,
            &pool,
            true,
        )
        .await
        .unwrap();
        assert_eq!(idx, 1);
        assert!(permit.is_some());
        three_permits.push(permit);
    }
    assert!(super::native_acquire_account_concurrency_permit(
        &host,
        &collection,
        &pool,
        true,
    )
    .await
    .is_err());

    // acct-unlimited overrides global limit 2 to 0 (unlimited): never blocks even when others are full.
    let mixed_with_unlimited = vec![acct_one.clone(), acct_three.clone(), acct_unlimited.clone()];
    let (idx_u, permit_u) = super::native_acquire_account_concurrency_permit(
        &host,
        &collection,
        &mixed_with_unlimited,
        true,
    )
    .await
    .unwrap();
    assert_eq!(idx_u, 2);
    assert!(permit_u.is_none());

    drop(permit1);
    drop(three_permits);
}

#[test]
fn automatic_catalog_keeps_explicit_future_models_without_guessing() {
    let models = vec![
        "gpt-future-test".into(),
        "gpt-5.4".into(),
        "gpt-6-sol".into(),
    ];
    assert_eq!(
        super::automatic_api_service_visible_model_ids_with_explicit_catalog(models.clone(), &[]),
        vec!["gpt-6-sol"]
    );
    assert_eq!(
        super::automatic_api_service_visible_model_ids_with_explicit_catalog(
            models,
            &["GPT-FUTURE-TEST".into()]
        ),
        vec!["gpt-future-test", "gpt-6-sol"]
    );
}
