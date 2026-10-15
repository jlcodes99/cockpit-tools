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
