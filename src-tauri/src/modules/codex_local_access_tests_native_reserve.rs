fn native_reserve_account(allowed: bool) -> CodexAccount {
    let mut account = native_test_account_with_quota(0, 0);
    account.id = "pool-a".into();
    account.quota.as_mut().unwrap().raw_data = Some(json!({
        "rate_limit":{"allowed":false},
        "rate_limit_upsell":{"banner_type":"luna_reserve"},
        "additional_rate_limits":[{"limit_name":"GPT-Reserve","rate_limit":{"allowed":allowed}}]
    }));
    account
}

#[test]
fn native_reserve_dispatch_requires_confirmed_entitlement_and_keeps_other_quota_guards() {
    let collection = native_test_collection();
    let account = native_reserve_account(true);
    assert!(!native_quota_allows(&collection, &account));
    assert!(native_model_quota_allows(
        &collection,
        &account,
        "gpt-reserve"
    ));
    assert!(!native_model_quota_allows(
        &collection,
        &account,
        "gpt-5.6-luna"
    ));
    let plan = native_model_route_plan(&collection, None, "gpt-reserve").unwrap();
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, false).as_deref(),
        Some("gpt-reserve")
    );
    assert!(native_operation_model(
        &collection,
        &plan,
        &native_reserve_account(false),
        None,
        false
    )
    .is_none());
    let mut api = account.clone();
    api.auth_mode = CodexAuthMode::Apikey;
    api.openai_api_key = Some("fixture-key".into());
    assert!(native_operation_model(&collection, &plan, &api, None, false).is_none());

    let mut reserved = collection;
    reserved.bound_oauth_account_id = Some(account.id.clone());
    reserved.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 10,
        strict: false, safety_margin_percent: 3,
});
    assert!(!native_model_quota_allows(
        &reserved,
        &account,
        "gpt-reserve"
    ));
    let mut states = HashMap::new();
    native_upsert_cooldown(
        &mut states,
        &account.id,
        "gpt-5.6-luna",
        429,
        Some(Duration::from_secs(90)),
        true,
        Instant::now(),
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-reserve", false, Instant::now()).is_some()
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-reserve", true, Instant::now()).is_none()
    );
}

#[test]
fn native_reserve_catalog_lists_explicit_capability_independently_of_current_entitlement() {
    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    for allowed in [true, false] {
        let account = native_reserve_account(allowed);
        let catalog = native_catalog(&collection, None, &[account]).unwrap();
        assert_eq!(
            catalog["data"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|entry| entry["id"] == "gpt-reserve")
                .count(),
            1
        );
        assert!(catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["slug"] == "gpt-reserve" && entry["visibility"] == "list"));
    }
}

#[test]
fn native_reserve_catalog_and_aliases_keep_key_scope_prefix_and_model_acl() {
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    key.inherit_account_pool = Some(false);
    key.account_ids = vec!["pool-a".into()];
    key.model_prefix = Some("team".into());
    key.allowed_models = vec!["gpt-reserve".into()];
    let account = native_reserve_account(true);
    let catalog = native_catalog(&collection, Some(&key), &[account.clone()]).unwrap();
    assert!(catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "team/gpt-reserve"));
    key.excluded_models = vec!["gpt-reserve".into()];
    let catalog = native_catalog(&collection, Some(&key), &[account.clone()]).unwrap();
    assert!(!catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "team/gpt-reserve"));
    key.excluded_models.clear();
    key.account_ids = vec!["pool-b".into()];
    assert!(
        !native_catalog(&collection, Some(&key), &[account.clone()]).unwrap()["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "team/gpt-reserve")
    );
    collection.model_aliases =
        serde_json::from_value(json!([{"alias":"reserve-alias","sourceModel":"gpt-reserve"}]))
            .unwrap();
    key.account_ids = vec!["pool-a".into()];
    key.allowed_models = vec!["reserve-alias".into()];
    let canonical = native_model_for_key(&collection, Some(&key), "team/reserve-alias").unwrap();
    let plan = native_model_route_plan(&collection, Some(&key), &canonical).unwrap();
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, false).as_deref(),
        Some("gpt-reserve")
    );
}

#[tokio::test]
async fn native_reserve_unqualified_accounts_fail_before_credential_preparation() {
    let mut host = NativeHostBridge::for_collection(native_test_collection());
    host.account_overrides
        .insert("pool-a".into(), native_reserve_account(false));
    host.collection_override.as_mut().unwrap().account_ids = vec!["pool-a".into()];
    let mut request = native_test_route_request("reserve-thread");
    request.model = "gpt-reserve".into();
    let error = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "no_available_account");
    assert!(host.bindings.lock().await.is_empty());
}
