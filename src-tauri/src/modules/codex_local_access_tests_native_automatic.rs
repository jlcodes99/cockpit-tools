fn native_automatic_fixture() -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        "pool-a".into(),
        "fixture@example.invalid".into(),
        "fixture-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        Some("fixture".into()),
        Some("Fixture".into()),
        vec!["client-text".into(), "client-vision".into()],
    );
    account.api_wire_api = Some("chat_completions".into());
    account.egress_proxy_disabled = true;
    account.api_model_mappings = vec![
        crate::models::codex::CodexApiModelMapping {
            client_model: "client-text".into(),
            upstream_model: "upstream-text".into(),
        },
        crate::models::codex::CodexApiModelMapping {
            client_model: "client-vision".into(),
            upstream_model: "upstream-vision".into(),
        },
    ];
    account
        .api_model_vision_support
        .insert("upstream-text".into(), false);
    account
        .api_model_vision_support
        .insert("upstream-vision".into(), true);
    account
}

#[test]
fn native_automatic_catalog_resolves_declared_client_slots_without_requiring_upstream_in_catalog() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account
        .api_model_mappings
        .push(crate::models::codex::CodexApiModelMapping {
            client_model: "undeclared-client".into(),
            upstream_model: "upstream-text".into(),
        });
    assert_eq!(
        native_account_model(&account, "CLIENT-TEXT").as_deref(),
        Some("upstream-text")
    );
    assert!(native_account_model(&account, "upstream-text").is_none());
    assert!(native_account_model(&account, "undeclared-client").is_none());
    let catalog = native_catalog(&collection, None, &[account]).unwrap();
    let entries = catalog["data"].as_array().unwrap();
    assert!(entries.iter().any(|entry| entry["id"] == "client-text"));
    assert!(!entries
        .iter()
        .any(|entry| entry["id"] == "upstream-text" || entry["id"] == "undeclared-client"));
    assert!(catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["slug"] == "client-text"
            && entry["input_modalities"] == json!(["text", "image"])));
}

#[test]
fn native_automatic_vision_routes_use_actual_upstream_model_and_fail_closed_on_ambiguity() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    let plan = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, false).as_deref(),
        Some("upstream-text")
    );
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-vision")
    );
    account.api_model_catalog.push("other-vision".into());
    account
        .api_model_vision_support
        .insert("other-vision".into(), true);
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-text")
    );
    account.api_vision_routing_model = Some("upstream-vision".into());
    assert_eq!(
        native_operation_model(&collection, &plan, &account, None, true).as_deref(),
        Some("upstream-vision")
    );
}

#[tokio::test]
async fn native_automatic_vision_selection_applies_target_acl_and_cooldown_before_credentials() {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let mut collection = native_test_collection();
    collection.account_ids = vec!["pool-a".into()];
    collection.max_retry_interval_ms = 0;
    let mut key = native_test_key();
    key.excluded_models = vec!["upstream-vision".into()];
    collection.api_keys = vec![key.clone()];
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides
        .insert("pool-a".into(), native_automatic_fixture());
    let caller = native_test_caller(&key.id);
    let mut request = native_test_route_request("vision-policy");
    request.model = "client-text".into();
    request.image_input = true;
    assert_eq!(
        native_select_route(&host, &caller, request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "no_available_account"
    );
    assert!(host.bindings.lock().await.is_empty());
    host.collection_override.as_mut().unwrap().api_keys[0]
        .excluded_models
        .clear();
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "pool-a",
        "upstream-vision",
        429,
        Some(Duration::from_secs(90)),
        false,
        Instant::now(),
    );
    assert!(native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap()
        .retry_after
        .is_some());
    assert!(
        native_retry_cooldown_wait(&host, &caller, request.clone(), Duration::from_secs(1))
            .await
            .unwrap()
            .is_none()
    );
    request.image_input = false;
    let route = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(route.model, "upstream-text");
}

#[test]
fn native_provider_vision_omission_changes_only_content_and_preserves_opaque_and_tool_data() {
    let collection = native_test_collection();
    let account = native_automatic_fixture();
    for (body, text_type, pointer) in [
        (
            json!({"model":"upstream-text","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}],"reasoning":{"encrypted_content":"cipher"},"tools":[{"type":"function","parameters":{"type":"input_image"}}]}),
            "input_text",
            "/input/0/content/0",
        ),
        (
            json!({"model":"upstream-text","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"fixture"}}]}],"reasoning":{"encrypted_content":"cipher"},"tools":[{"type":"function","parameters":{"type":"input_image"}}]}),
            "text",
            "/messages/0/content/0",
        ),
    ] {
        let result = native_prepare_provider_vision_body(&collection, None, &account, body.clone());
        assert_eq!(result.pointer(pointer).unwrap()["type"], text_type);
        assert!(result.pointer(pointer).unwrap().get("image_url").is_none());
        assert_eq!(result["reasoning"], body["reasoning"]);
        assert_eq!(result["tools"], body["tools"]);
    }
    let body = json!({"model":"upstream-vision","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, None, &account, body.clone()),
        body
    );
}

#[test]
fn native_gateway_vision_uses_current_account_capabilities_without_overriding_explicit_model_values(
) {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    let mut key = native_test_key();
    key.provider_gateway = Some(serde_json::from_value(json!({"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"upstream-text","upstreamModels":["upstream-text","upstream-vision"]})).unwrap());
    let plan = native_model_route_plan(&collection, Some(&key), "upstream-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &plan, &account, "upstream-text"),
        "upstream-vision"
    );
    assert!(native_catalog_vision(
        &collection,
        &plan,
        &account,
        "upstream-text"
    ));
    let images = json!({"model":"upstream-vision","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, Some(&key), &account, images.clone()),
        images
    );
    key.provider_gateway
        .as_mut()
        .unwrap()
        .model_capabilities
        .insert(
            "upstream-vision".into(),
            CodexLocalAccessProviderGatewayModelCapability {
                supports_vision: false,
            },
        );
    let plan = native_model_route_plan(&collection, Some(&key), "upstream-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &plan, &account, "upstream-text"),
        "upstream-text"
    );
    assert!(!native_catalog_vision(
        &collection,
        &plan,
        &account,
        "upstream-text"
    ));
    let result =
        native_prepare_provider_vision_body(&collection, Some(&key), &account, images.clone());
    assert_eq!(result["input"][0]["content"][0]["type"], "input_text");
    account.api_model_vision_support.clear();
    account.api_supports_vision = true;
    let automatic = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert_eq!(
        native_plan_vision_model(&collection, &automatic, &account, "upstream-text"),
        "upstream-text"
    );
    assert!(native_catalog_vision(
        &collection,
        &automatic,
        &account,
        "upstream-text"
    ));
    let images = json!({"model":"upstream-text","input":[{"role":"user","content":[{"type":"input_image","image_url":"fixture"}]}]});
    assert_eq!(
        native_prepare_provider_vision_body(&collection, None, &account, images.clone()),
        images
    );
}

#[test]
fn native_automatic_catalog_hides_retired_gpt_names_but_keeps_request_routing() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account.api_model_catalog = vec!["gpt-4.1".into(), "client-text".into()];
    let catalog = native_catalog(&collection, None, &[account.clone()]).unwrap();
    assert!(!catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == "gpt-4.1"));
    assert!(native_account_model(&account, "gpt-4.1").is_some());
}

#[tokio::test]
async fn native_automatic_third_party_names_skip_oauth_pool_and_preserve_native_fallback_models() {
    let _env = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _data = super::native_capability_integration_tests::NativeInstallationTestData::new();
    let mut collection = native_test_collection();
    collection.session_affinity = false;
    let account = native_automatic_fixture();
    let mut oauth = native_test_account();
    oauth.id = "pool-b".into();
    let provider_plan = native_model_route_plan(&collection, None, "client-text").unwrap();
    assert!(native_plan_account_model(&provider_plan, &oauth).is_none());
    assert_eq!(
        native_plan_account_model(&provider_plan, &account).as_deref(),
        Some("upstream-text")
    );
    let native_model = api_service_routable_codex_model_ids()
        .into_iter()
        .next()
        .unwrap();
    let native_plan = native_model_route_plan(&collection, None, &native_model).unwrap();
    assert_eq!(
        native_plan_account_model(&native_plan, &oauth).as_deref(),
        Some(native_model.as_str())
    );
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides.insert(account.id.clone(), account);
    host.account_overrides.insert(oauth.id.clone(), oauth);
    let mut request = native_test_route_request("mixed-provider-only");
    request.model = "client-text".into();
    let route = native_select_route(&host, &native_test_caller(NATIVE_PRIMARY_CALLER), request)
        .await
        .unwrap();
    assert_eq!(route.account_id, "pool-a");
    assert_eq!(route.model, "upstream-text");
}

#[test]
fn native_automatic_empty_responses_pool_does_not_claim_undeclared_provider_models() {
    let collection = native_test_collection();
    let mut account = native_automatic_fixture();
    account.api_wire_api = Some("responses".into());
    account.api_model_catalog.clear();
    account.api_model_mappings.clear();
    let plan = native_model_route_plan(&collection, None, "other-provider-only").unwrap();
    assert!(native_plan_account_model(&plan, &account).is_none());
    let mut key = native_test_key();
    key.provider_gateway = Some(serde_json::from_value(json!({"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"other-provider-only"})).unwrap());
    let explicit = native_model_route_plan(&collection, Some(&key), "other-provider-only").unwrap();
    assert_eq!(
        native_plan_account_model(&explicit, &account).as_deref(),
        Some("other-provider-only")
    );
}

#[test]
fn native_official_quota_reset_reopens_elapsed_windows_but_keeps_other_windows_and_429() {
    let collection = native_test_collection();
    let now = chrono::Utc::now().timestamp();
    let mut account = native_test_account_with_quota(0, 70);
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now - 1);
    assert!(native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now + 60);
    assert!(!native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().hourly_reset_time = Some(now - 1);
    account.quota.as_mut().unwrap().weekly_percentage = 0;
    account.quota.as_mut().unwrap().weekly_reset_time = Some(now + 60);
    assert!(!native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().weekly_reset_time = Some(now - 1);
    assert!(native_quota_allows(&collection, &account));
    let mut states = HashMap::new();
    native_upsert_cooldown(
        &mut states,
        &account.id,
        "gpt-fixture",
        429,
        Some(Duration::from_secs(90)),
        true,
        Instant::now(),
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-fixture", false, Instant::now()).is_some()
    );
    assert!(
        native_cooldown_wait(&states, &account.id, "gpt-fixture", true, Instant::now()).is_none()
    );
    let mut reserved = collection;
    reserved.bound_oauth_account_id = Some(account.id.clone());
    reserved.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 10,
        weekly_percent: 10,
        strict: false, safety_margin_percent: 3,
});
    assert!(!native_quota_allows(&reserved, &account));
}

#[test]
fn native_xai_suffix_is_route_scoped_and_checks_full_and_base_model_acl() {
    let collection = native_test_collection();
    let mut key = native_test_key();
    key.model_routing = Some(serde_json::from_value(json!({"defaultRoute":"oauth","failurePolicy":"strict","routes":[{"id":"xai","namespace":"grok","providerAccountId":"pool-a","nativeProvider":"xai","providerGateway":{"baseUrl":"https://example.invalid/v1","apiKey":"fixture","upstreamModel":"grok-4.3","upstreamModels":["grok-4.3"]}}]})).unwrap());
    let plan = native_model_route_plan(&collection, Some(&key), "grok/grok-4.3(8192)").unwrap();
    let account = native_automatic_fixture();
    assert_eq!(
        native_plan_account_model(&plan, &account).as_deref(),
        Some("grok-4.3")
    );
    assert!(!native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.excluded_models = vec!["grok-4.3".into()];
    assert!(native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.excluded_models.clear();
    key.allowed_models = vec!["grok/grok-4.3(8192)".into()];
    assert!(native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.allowed_models.push("grok/grok-4.3".into());
    assert!(!native_xai_base_denied(
        &collection,
        Some(&key),
        &plan,
        &account
    ));
    key.model_routing.as_mut().unwrap().routes[0].native_provider = None;
    assert!(native_model_route_plan(&collection, Some(&key), "grok/grok-4.3(8192)").is_err());
}
