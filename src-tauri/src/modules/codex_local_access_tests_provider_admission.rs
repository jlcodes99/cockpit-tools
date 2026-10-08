#[test]
fn provider_gateway_template_preserves_enabled_and_disabled_concurrency() {
    let mut instance = test_local_access_collection(vec!["bound".into()]);
    let mut template = test_local_access_collection(vec!["global".into()]);
    for (limit, wait_ms) in [(3, 1500), (0, 0)] {
        template.max_account_concurrency = limit;
        template.account_concurrency_wait_ms = wait_ms;
        super::apply_provider_gateway_template_settings(&mut instance, &template);
        assert_eq!(instance.max_account_concurrency, limit);
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
