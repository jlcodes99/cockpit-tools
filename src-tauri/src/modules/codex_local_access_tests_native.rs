// Included in native_host_tests; no real account directories or services are used.

fn native_test_collection() -> CodexLocalAccessCollection {
    serde_json::from_value(json!({
        "enabled": true,
        "port": 1455,
        "apiKey": "fixture-primary-key",
        "accountIds": ["pool-a", "pool-b"],
        "createdAt": 1,
        "updatedAt": 2
    }))
    .expect("valid minimal collection")
}

fn native_test_key() -> CodexLocalAccessApiKey {
    serde_json::from_value(json!({
        "id": "fixture-key-id",
        "label": "Fixture key",
        "key": "fixture-secret"
    }))
    .expect("valid minimal API key")
}

fn native_test_caller(id: &str) -> native_contract::Caller {
    native_contract::Caller {
        id: id.into(),
        label: "Fixture caller".into(),
    }
}

fn native_test_account() -> CodexAccount {
    CodexAccount::new(
        "fixture-account".into(),
        "fixture@example.invalid".into(),
        crate::models::codex::CodexTokens {
            access_token: "fixture-access".into(),
            refresh_token: Some("fixture-refresh".into()),
            id_token: "fixture-id".into(),
        },
    )
}

#[test]
fn native_public_keys_are_rechecked_for_revocation_disable_and_limits() {
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    let caller = native_test_caller(&key.id);
    collection.api_keys = vec![key.clone()];
    assert_eq!(
        native_caller_key(&collection, &caller).unwrap().unwrap().id,
        key.id
    );

    collection.api_keys[0].enabled = false;
    let error = native_caller_key(&collection, &caller).unwrap_err();
    assert_eq!((error.status, error.code), (401, "invalid_api_key"));
    collection.api_keys.clear();
    assert_eq!(
        native_caller_key(&collection, &caller).unwrap_err().status,
        401
    );

    key.token_limit = Some(100);
    key.token_used = 99;
    collection.api_keys = vec![key];
    assert!(native_caller_key(&collection, &caller).is_ok());
    for used in [100, 101] {
        collection.api_keys[0].token_used = used;
        let error = native_caller_key(&collection, &caller).unwrap_err();
        assert_eq!((error.status, error.code), (429, "token_limit_reached"));
    }
    collection.api_keys[0].token_limit = None;
    assert!(native_caller_key(&collection, &caller).is_ok());
    collection.enabled = false;
    let error = native_caller_key(&collection, &caller).unwrap_err();
    assert_eq!((error.status, error.code), (503, "service_disabled"));
}

#[test]
fn native_primary_key_compatibility_does_not_bypass_configured_key_policies() {
    let mut collection = native_test_collection();
    let primary = native_test_caller(NATIVE_PRIMARY_CALLER);
    assert!(native_caller_key(&collection, &primary).unwrap().is_none());
    collection.api_keys.push(native_test_key());
    assert_eq!(
        native_caller_key(&collection, &primary).unwrap_err().status,
        401
    );

    let caller = native_test_caller("fixture-key-id");
    collection.api_keys[0].model_routing = Some(
        serde_json::from_value(json!({
            "defaultRoute": "provider-a", "failurePolicy": "fail", "routes": []
        }))
        .unwrap(),
    );
    assert!(native_caller_key(&collection, &caller).is_ok());
    assert!(native_model_route_plan(&collection, Some(&collection.api_keys[0]), "model").is_err());
    collection.api_keys[0].model_routing = None;
    collection.api_keys[0].provider_gateway = Some(
        serde_json::from_value(json!({
            "baseUrl": "https://upstream.example.invalid/v1",
            "apiKey": "fixture-upstream-key", "upstreamModel": "gpt-fixture"
        }))
        .unwrap(),
    );
    assert!(native_caller_key(&collection, &caller).is_ok());
}

#[test]
fn native_model_namespace_is_exact_and_deny_rules_override_allow_rules() {
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    key.model_prefix = Some("team/".into());
    key.allowed_models = vec!["gpt-*".into()];
    key.excluded_models = vec!["gpt-private*".into()];
    collection.excluded_models = vec!["gpt-global-denied".into()];
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/gpt-fixture").unwrap(),
        "gpt-fixture"
    );
    for model in [
        "gpt-fixture",
        "other/gpt-fixture",
        "teamish/gpt-fixture",
        "team/not-gpt",
        "team/gpt-private",
        "team/gpt-global-denied",
    ] {
        let error = native_model_for_key(&collection, Some(&key), model).unwrap_err();
        assert_eq!(
            (error.status, error.code),
            (403, "model_not_allowed"),
            "{model}"
        );
    }
    key.allowed_models.clear();
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/other-model").unwrap(),
        "other-model"
    );
}

#[test]
fn native_model_aliases_do_not_bypass_the_public_alias_allowlist() {
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    key.model_prefix = Some("team".into());
    key.allowed_models = vec!["public-model".into()];
    collection.model_aliases = vec![serde_json::from_value(json!({
        "alias": "public-model", "sourceModel": "upstream-model"
    }))
    .unwrap()];
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/public-model").unwrap(),
        "upstream-model"
    );
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/upstream-model")
            .unwrap_err()
            .status,
        403
    );
    key.excluded_models.push("public-model".into());
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/public-model")
            .unwrap_err()
            .status,
        403
    );
}

#[test]
fn native_aliases_cannot_bypass_global_or_key_canonical_model_denials() {
    let mut collection = native_test_collection();
    let mut key = native_test_key();
    key.model_prefix = Some("team".into());
    key.allowed_models = vec!["public-model".into()];
    collection.model_aliases = vec![serde_json::from_value(json!({
        "alias": "public-model", "sourceModel": "restricted-model"
    }))
    .unwrap()];
    assert_eq!(
        native_model_for_key(&collection, Some(&key), "team/public-model").unwrap(),
        "restricted-model"
    );

    for deny_pattern in ["restricted-model", "restricted-*"] {
        collection.excluded_models = vec![deny_pattern.into()];
        let error = native_model_for_key(&collection, Some(&key), "team/public-model").unwrap_err();
        assert_eq!((error.status, error.code), (403, "model_not_allowed"));
        // The primary key also respects global exclusions after resolving an alias.
        assert_eq!(
            native_model_for_key(&collection, None, "public-model")
                .unwrap_err()
                .status,
            403
        );

        collection.excluded_models.clear();
        key.excluded_models = vec![deny_pattern.into()];
        let error = native_model_for_key(&collection, Some(&key), "team/public-model").unwrap_err();
        assert_eq!((error.status, error.code), (403, "model_not_allowed"));
        key.excluded_models.clear();
    }
}

#[test]
fn native_wildcards_are_anchored_and_other_metacharacters_stay_literal() {
    for (pattern, model, expected) in [
        ("", "", false),
        ("", "a", false),
        ("*", "", false),
        ("*", "gpt-model", true),
        ("gpt-*", "gpt-", true),
        ("gpt-*", "prefix/gpt-model", false),
        ("gpt", "gpt-model", false),
        ("*gpt", "model-gpt", true),
        ("*gpt", "model-gpt-tail", false),
        ("gpt-**-x*", "gpt-model-x1", true),
        ("gpt-*-x", "gpt-model-y", false),
        ("gpt.[ab]", "gpt.[ab]", true),
        ("gpt.[ab]", "gptXa", false),
        ("gpt-?", "gpt-x", false),
        ("模型-*", "模型-甲", true),
    ] {
        assert_eq!(
            native_pattern_matches(pattern, model),
            expected,
            "{pattern:?} vs {model:?}"
        );
    }
}

#[test]
fn native_account_routes_require_responses_and_preserve_full_upstream_models() {
    let mut account = native_test_account();
    assert!(native_eligible(&account));
    account.requires_reauth = true;
    assert!(!native_eligible(&account));
    account.requires_reauth = false;
    account.upstream_grok_account_id = Some("fixture-grok".into());
    assert!(!native_eligible(&account));
    account.upstream_grok_account_id = None;
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    account.api_wire_api = Some("chat".into());
    assert!(!native_eligible(&account));
    account.api_wire_api = Some("responses".into());
    assert!(native_eligible(&account));
    account.api_model_catalog = vec!["public-model".into()];
    account.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "public-model".into(),
        upstream_model: "vendor/model-version".into(),
    }];
    assert_eq!(
        native_account_model(&account, "public-model"),
        Some("vendor/model-version".into())
    );
    assert_eq!(native_account_model(&account, "vendor/model"), None);
    account.api_model_catalog.clear();
    account.api_model_mappings.clear();
    assert_eq!(
        native_account_model(&account, "custom-model"),
        Some("custom-model".into())
    );
}

#[test]
fn native_key_account_scopes_preserve_explicit_empty_and_legacy_fixed_pools() {
    let collection = native_test_collection();
    let mut key = native_test_key();
    assert_eq!(
        native_scoped_account_ids(&collection, None),
        collection.account_ids
    );
    assert_eq!(
        native_scoped_account_ids(&collection, Some(&key)),
        collection.account_ids
    );
    key.account_ids = vec!["scoped-account".into()];
    assert_eq!(
        native_scoped_account_ids(&collection, Some(&key)),
        key.account_ids
    );
    key.inherit_account_pool = Some(true);
    assert_eq!(
        native_scoped_account_ids(&collection, Some(&key)),
        collection.account_ids
    );
    key.inherit_account_pool = Some(false);
    assert_eq!(
        native_scoped_account_ids(&collection, Some(&key)),
        key.account_ids
    );
    key.account_ids.clear();
    assert!(native_scoped_account_ids(&collection, Some(&key)).is_empty());
}

#[test]
fn native_mode_patch_preserves_latest_unrelated_fields_and_rejects_nonobjects() {
    let original = json!({
        "gatewayMode": "sidecar", "updatedAt": 1, "port": 18455,
        "enabled": false, "requestPayloadLogging": true,
        "apiKeys": [{"id": "key-latest", "tokenUsed": 456}],
        "accountIds": ["latest-account"],
        "futureField": {"preserve": [1, 2, 3]}
    });
    let patched: Value = serde_json::from_str(
        &native_patch_mode(&original.to_string(), CodexLocalAccessGatewayMode::Native).unwrap(),
    )
    .unwrap();
    assert_eq!(patched["gatewayMode"], "native");
    assert!(patched["updatedAt"].as_i64().unwrap() > 1);
    for key in [
        "port",
        "enabled",
        "requestPayloadLogging",
        "apiKeys",
        "accountIds",
        "futureField",
    ] {
        assert_eq!(patched[key], original[key], "{key}");
    }
    for invalid in ["", "{", "null", "[]", "42", "\"string\""] {
        assert!(native_patch_mode(invalid, CodexLocalAccessGatewayMode::Native).is_err());
    }
}

#[test]
fn native_engine_selection_survives_stale_stats_writes_in_both_directions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    let mut stale_sidecar = native_test_collection();
    stale_sidecar.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    std::fs::write(&path, serde_json::to_vec(&stale_sidecar).unwrap()).unwrap();
    let native = native_patch_mode(
        &std::fs::read_to_string(&path).unwrap(),
        CodexLocalAccessGatewayMode::Native,
    )
    .unwrap();
    std::fs::write(&path, native).unwrap();
    stale_sidecar.port = 15555;
    save_collection_to_path(&path, &stale_sidecar).unwrap();
    let mut stale_native: CodexLocalAccessCollection =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        stale_native.gateway_mode,
        CodexLocalAccessGatewayMode::Native
    );
    assert_eq!(stale_native.port, 15555);
    let sidecar = native_patch_mode(
        &std::fs::read_to_string(&path).unwrap(),
        CodexLocalAccessGatewayMode::Sidecar,
    )
    .unwrap();
    std::fs::write(&path, sidecar).unwrap();
    stale_native.port = 16666;
    save_collection_to_path(&path, &stale_native).unwrap();
    let latest: CodexLocalAccessCollection =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(latest.gateway_mode, CodexLocalAccessGatewayMode::Sidecar);
    assert_eq!(latest.port, 16666);
}

fn native_test_account_with_quota(hourly: i32, weekly: i32) -> CodexAccount {
    let mut account = native_test_account();
    account.usage_updated_at = Some(chrono::Utc::now().timestamp());
    account.quota = Some(
        serde_json::from_value(json!({
            "hourly_percentage": hourly,
            "weekly_percentage": weekly,
            "hourly_window_present": true,
            "weekly_window_present": true
        }))
        .unwrap(),
    );
    account
}

#[test]
fn native_quota_reservation_applies_only_to_bound_account_and_present_windows() {
    let account_with_quota = |hourly, weekly| {
        let mut account = native_test_account_with_quota(hourly, weekly);
        account.id = "quota-reservation-present-windows".into();
        account
    };
    let mut collection = native_test_collection();
    collection.bound_oauth_account_id = Some("quota-reservation-present-windows".into());
    collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 10,
        strict: false, safety_margin_percent: 3,
});
    let available = account_with_quota(24, 14);
    {
        let mut runtime = quota_reserve_coordinator().lock().unwrap();
        runtime.accounts.entry(available.id.clone()).or_default();
    }
    confirm_gateway_quota_refresh(&available.id, available.quota.as_ref().unwrap(), Instant::now());
    assert!(native_quota_allows(&collection, &available));
    let hourly_blocked = account_with_quota(23, 14);
    confirm_gateway_quota_refresh(&hourly_blocked.id, hourly_blocked.quota.as_ref().unwrap(), Instant::now());
    assert!(!native_quota_allows(&collection, &hourly_blocked));
    let weekly_blocked = account_with_quota(24, 13);
    confirm_gateway_quota_refresh(&weekly_blocked.id, weekly_blocked.quota.as_ref().unwrap(), Instant::now());
    assert!(!native_quota_allows(&collection, &weekly_blocked));
    let mut unrelated = account_with_quota(5, 5);
    unrelated.id = "unrelated-account".into();
    assert!(native_quota_allows(&collection, &unrelated));
    let mut absent_window = account_with_quota(0, 14);
    absent_window.quota.as_mut().unwrap().hourly_window_present = Some(false);
    confirm_gateway_quota_refresh(&absent_window.id, absent_window.quota.as_ref().unwrap(), Instant::now());
    assert!(native_quota_allows(&collection, &absent_window));
    collection.bound_oauth_quota_reserve = None;
    assert!(native_quota_allows(
        &collection,
        &account_with_quota(5, 5)
    ));
    assert!(!native_quota_allows(
        &collection,
        &account_with_quota(0, 5)
    ));
}

#[test]
fn native_reserved_quota_fails_closed_for_missing_stale_or_invalid_evidence() {
    let mut collection = native_test_collection();
    collection.bound_oauth_account_id = Some("fixture-account".into());
    collection.bound_oauth_quota_reserve = Some(CodexLocalAccessQuotaReserve {
        hourly_percent: 20,
        weekly_percent: 10,
        strict: false, safety_margin_percent: 3,
});
    assert!(!native_quota_allows(&collection, &native_test_account()));
    let now = chrono::Utc::now().timestamp();
    for timestamp in [None, Some(0), Some(now - 181), Some(now + 60)] {
        let mut account = native_test_account_with_quota(90, 90);
        account.usage_updated_at = timestamp;
        assert!(
            !native_quota_allows(&collection, &account),
            "timestamp={timestamp:?}"
        );
    }
    let mut error_account = native_test_account_with_quota(90, 90);
    error_account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("quota_refresh_failed".into()),
        message: "fixture failure".into(),
        timestamp: now,
    });
    assert!(!native_quota_allows(&collection, &error_account));
    for (hourly, weekly) in [(101, 90), (90, 101), (-1, 90), (90, -1)] {
        assert!(!native_quota_allows(
            &collection,
            &native_test_account_with_quota(hourly, weekly)
        ));
    }
}

#[test]
fn native_hosted_image_policy_obeys_service_and_account_controls() {
    let mut collection = native_test_collection();
    let mut account = native_test_account();
    assert!(native_image_allows(&collection, &account));
    collection.image_generation_account_policies.insert(
        account.id.clone(),
        CodexLocalAccessImageGenerationPolicy::Disabled,
    );
    assert!(!native_image_allows(&collection, &account));
    collection.image_generation_account_policies.clear();
    account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
    assert!(!native_image_allows(&collection, &account));
    collection.image_generation_account_policies.insert(
        account.id.clone(),
        CodexLocalAccessImageGenerationPolicy::Enabled,
    );
    assert!(native_image_allows(&collection, &account));
    for mode in [
        CodexLocalAccessImageGenerationMode::Disabled,
        CodexLocalAccessImageGenerationMode::ImagesOnly,
    ] {
        collection.image_generation_mode = mode;
        assert!(!native_image_allows(&collection, &account));
    }
}

#[test]
fn native_owner_tracks_api_credentials_but_keeps_oauth_refresh_stable() {
    let mut account = native_test_account();
    account.user_id = Some("user-one".into());
    account.account_id = Some("workspace-one".into());
    let endpoint = "https://example.invalid/v1";
    let oauth_owner = native_account_owner(&account, endpoint);
    account.tokens.access_token = "refreshed-access".into();
    account.tokens.refresh_token = Some("refreshed-refresh".into());
    account.token_generation += 1;
    assert_eq!(oauth_owner, native_account_owner(&account, endpoint));
    account.account_id = Some("workspace-two".into());
    assert_ne!(oauth_owner, native_account_owner(&account, endpoint));

    account.auth_mode = CodexAuthMode::Apikey;
    account.openai_api_key = Some("first-api-principal".into());
    let api_owner = native_account_owner(&account, endpoint);
    assert!(!api_owner.contains("first-api-principal"));
    account.openai_api_key = Some("second-api-principal".into());
    assert_ne!(api_owner, native_account_owner(&account, endpoint));

    account.auth_mode = CodexAuthMode::OAuth;
    account.agent_identity = Some(
        serde_json::from_value(json!({
            "agent_runtime_id": "runtime", "agent_private_key": "private-fixture",
            "task_id": "task-one", "account_id": "workspace", "chatgpt_user_id": "user"
        }))
        .unwrap(),
    );
    let agent_owner = native_account_owner(&account, endpoint);
    assert!(!agent_owner.contains("private-fixture"));
    account.agent_identity.as_mut().unwrap().task_id = Some("task-two".into());
    assert_ne!(agent_owner, native_account_owner(&account, endpoint));
}

fn native_test_route_request(session: &str) -> native_contract::RouteRequest {
    native_contract::RouteRequest {
        model: "gpt-fixture".into(),
        session_id: Some(session.into()),
        thread_id: Some(session.into()),
        parent_thread_id: None,
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: true,
        auxiliary: None,
        required_capabilities: Vec::new(),
        capability_fallback_from: None,
    }
}

fn native_test_routing_host() -> NativeHostBridge {
    let mut collection = native_test_collection();
    collection.max_account_concurrency = 1;
    collection.account_concurrency_wait_ms = 0;
    collection.session_affinity = true;
    let mut host = NativeHostBridge::for_collection(collection);
    for id in ["pool-a", "pool-b"] {
        let mut account = native_test_account();
        account.id = id.into();
        account.auth_mode = CodexAuthMode::Apikey;
        account.openai_api_key = Some(format!("fixture-key-{id}"));
        account.api_base_url = Some("https://example.invalid/v1".into());
        account.api_wire_api = Some("responses".into());
        account.api_model_catalog = vec!["gpt-fixture".into()];
        account.egress_proxy_disabled = true;
        host.account_overrides.insert(id.into(), account);
    }
    host
}

#[tokio::test]
async fn native_free_agent_identity_keeps_pool_eligibility_without_bypassing_restrictions() {
    let mut host = native_test_routing_host();
    host.collection_override.as_mut().unwrap().account_ids = vec!["pool-a".into()];
    host.collection_override
        .as_mut()
        .unwrap()
        .restrict_free_accounts = true;
    let mut agent = native_test_account();
    agent.id = "pool-a".into();
    agent.plan_type = Some("free".into());
    agent.tokens.access_token.clear();
    agent.tokens.id_token.clear();
    agent.tokens.refresh_token = None;
    agent.agent_identity = Some(serde_json::from_value(json!({
        "agent_runtime_id":"fixture-runtime","agent_private_key":"unused-fixture-key",
        "task_id":"fixture-task","account_id":"fixture-workspace","chatgpt_user_id":"fixture-user"
    })).unwrap());
    assert!(is_local_access_eligible_account(&agent, true));
    host.account_overrides
        .insert(agent.id.clone(), agent.clone());
    let collection = host.collection().await.unwrap();
    // Stop at the real concurrency gate after selection, before signing or any
    // credential I/O. The account must be busy, rather than absent from the pool.
    let (_, _busy) = native_acquire_account_permit(&host, &collection, &[agent.clone()], true)
        .await
        .unwrap();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_test_route_request("free-agent-eligibility");
    // OAuth/Agent automatic routes accept the official model catalog, rather
    // than the API-key fixture model used by native_test_routing_host.
    request.model = api_service_routable_codex_model_ids()
        .into_iter()
        .find(|model| !native_is_reserve_model(model))
        .unwrap();
    let error = native_select_route(&host, &caller, request.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (429, "account_busy"));
    let mut reauth = agent.clone();
    reauth.requires_reauth = true;
    let mut web_session = agent.clone();
    web_session.token_source_mode = "chatgpt_web_session".into();
    for denied in [reauth, web_session] {
        host.account_overrides.insert(denied.id.clone(), denied);
        assert_eq!(
            native_select_route(&host, &caller, request.clone())
                .await
                .err()
                .unwrap()
                .code,
            "no_available_account"
        );
    }
    host.account_overrides
        .insert(agent.id.clone(), agent.clone());
    let mut excluded = request.clone();
    excluded.excluded_account_ids = vec![agent.id.clone()];
    assert_eq!(
        native_select_route(&host, &caller, excluded)
            .await
            .err()
            .unwrap()
            .code,
        "no_available_account"
    );
    let mut ordinary = agent;
    ordinary.agent_identity = None;
    ordinary.tokens.access_token = "fixture-access".into();
    host.account_overrides.insert(ordinary.id.clone(), ordinary);
    assert_eq!(
        native_select_route(&host, &caller, request.clone())
            .await
            .err()
            .unwrap()
            .code,
        "no_available_account"
    );
    assert!(host.bindings.lock().await.is_empty());
}

#[tokio::test]
async fn native_new_sessions_use_free_accounts_but_bound_children_do_not_split() {
    let host = native_test_routing_host();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let first = native_select_route(&host, &caller, native_test_route_request("root-a"))
        .await
        .unwrap();
    assert_eq!(first.account_id, "pool-a");
    let second = native_select_route(&host, &caller, native_test_route_request("root-b"))
        .await
        .unwrap();
    assert_eq!(second.account_id, "pool-b");
    drop(second);

    let mut child = native_test_route_request("child-a");
    child.parent_thread_id = Some("root-a".into());
    let error = native_select_route(&host, &caller, child.clone())
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (429, "account_busy"));
    assert!(!host
        .bindings
        .lock()
        .await
        .contains_key(&format!("{}\0child-a", caller.id)));
    drop(first);
    let resumed = native_select_route(&host, &caller, child).await.unwrap();
    assert_eq!(resumed.account_id, "pool-a");
}

#[tokio::test]
async fn native_account_wait_uses_first_freed_candidate_and_releases_losing_waiters() {
    let mut host = native_test_routing_host();
    host.collection_override
        .as_mut()
        .unwrap()
        .account_concurrency_wait_ms = 500;
    let collection = host.collection().await.unwrap();
    let accounts: Vec<_> = ["pool-a", "pool-b"]
        .iter()
        .map(|id| host.account_overrides[*id].clone())
        .collect();
    let (_, busy_a) = native_acquire_account_permit(&host, &collection, &accounts[..1], true)
        .await
        .unwrap();
    let (_, busy_b) = native_acquire_account_permit(&host, &collection, &accounts[1..], true)
        .await
        .unwrap();
    let releaser = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(busy_b);
    });
    let (selected, acquired) = native_acquire_account_permit(&host, &collection, &accounts, true)
        .await
        .unwrap();
    assert_eq!(selected, 1);
    releaser.await.unwrap();
    drop(busy_a);
    let (_, available_a) = native_acquire_account_permit(&host, &collection, &accounts[..1], true)
        .await
        .unwrap();
    assert!(available_a.is_some());
    drop((available_a, acquired));
}

#[test]
fn native_account_wait_budget_preserves_saved_duration_and_host_validation_bound() {
    let mut collection = native_test_collection();
    assert_eq!(
        native_account_concurrency_wait(&collection),
        Duration::from_secs(120)
    );
    collection.account_concurrency_wait_ms = 300_000;
    assert_eq!(
        native_account_concurrency_wait(&collection),
        Duration::from_secs(300)
    );
    collection.account_concurrency_wait_ms = 25;
    assert_eq!(
        native_account_concurrency_wait(&collection),
        Duration::from_millis(25)
    );
    collection.account_concurrency_wait_ms = 0;
    assert_eq!(native_account_concurrency_wait(&collection), Duration::ZERO);
    collection.account_concurrency_wait_ms = u64::MAX;
    assert_eq!(
        native_account_concurrency_wait(&collection),
        Duration::from_secs(30 * 60)
    );
}

#[test]
fn native_selection_gate_budget_uses_execution_remaining_without_shrinking_saved_fallbacks() {
    let mut collection = native_test_collection();
    collection.timeouts.legacy_request_read_timeout_ms = 20;
    collection.account_concurrency_wait_ms = 120_000;
    assert_eq!(
        native_selection_wait_budget(&collection, None),
        Duration::from_secs(120)
    );
    collection.timeouts.legacy_request_read_timeout_ms = 300_000;
    assert_eq!(
        native_selection_wait_budget(&collection, None),
        Duration::from_secs(300)
    );
    assert_eq!(
        native_selection_wait_budget(&collection, Some(Duration::from_millis(35))),
        Duration::from_millis(35)
    );
    assert_eq!(
        native_selection_wait_budget(&collection, Some(Duration::ZERO)),
        Duration::ZERO
    );
    collection.timeouts.legacy_request_read_timeout_ms = 20;
    collection.account_concurrency_wait_ms = 45;
    assert_eq!(
        native_selection_wait_budget(&collection, None),
        Duration::from_millis(45)
    );
    collection.timeouts.legacy_request_read_timeout_ms = u64::MAX;
    collection.account_concurrency_wait_ms = 0;
    assert_eq!(
        native_selection_wait_budget(&collection, None),
        Duration::from_secs(10 * 60)
    );
}

#[tokio::test]
async fn native_failed_route_preparation_does_not_publish_affinity_or_keep_permit() {
    let mut host = native_test_routing_host();
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .api_base_url = Some("not-a-url".into());
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let error = native_select_route(&host, &caller, native_test_route_request("failed-root"))
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "invalid_upstream");
    assert!(host.bindings.lock().await.is_empty());
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .api_base_url = Some("https://example.invalid/v1".into());
    host.routing_sequence.store(0, Ordering::Relaxed);
    let route = native_select_route(&host, &caller, native_test_route_request("failed-root"))
        .await
        .unwrap();
    assert_eq!(route.account_id, "pool-a");
}

#[test]
fn native_route_order_matches_plan_quota_expiry_and_global_priority_controls() {
    let host = NativeHostBridge::default();
    let mut collection = native_test_collection();
    let mut pro = native_test_account_with_quota(20, 20);
    pro.id = "pro".into();
    pro.plan_type = Some("pro".into());
    pro.subscription_active_until = Some("2000000000".into());
    let mut plus = native_test_account_with_quota(90, 90);
    plus.id = "plus".into();
    plus.plan_type = Some("plus".into());
    plus.subscription_active_until = Some("1900000000000".into());

    for (strategy, expected) in [
        (CodexLocalAccessRoutingStrategy::Auto, "pro"),
        (CodexLocalAccessRoutingStrategy::PlanHighFirst, "pro"),
        (CodexLocalAccessRoutingStrategy::PlanLowFirst, "plus"),
        (CodexLocalAccessRoutingStrategy::QuotaHighFirst, "plus"),
        (CodexLocalAccessRoutingStrategy::QuotaLowFirst, "pro"),
        (CodexLocalAccessRoutingStrategy::ExpirySoonFirst, "plus"),
    ] {
        collection.routing_strategy = strategy;
        let mut accounts = vec![plus.clone(), pro.clone()];
        native_sort_accounts(&host, &mut accounts, &collection, None);
        assert_eq!(accounts[0].id, expected, "{strategy:?}");
        collection.custom_routing_rules = vec![serde_json::from_value(json!({
            "accountId": "plus", "priority": 0, "weight": 1, "isPreferred": true,
        }))
        .unwrap()];
        native_sort_accounts(&host, &mut accounts, &collection, None);
        assert_eq!(accounts[0].id, "plus", "preferred with {strategy:?}");
        collection.custom_routing_rules = vec![serde_json::from_value(json!({
            "accountId": "plus", "priority": 0, "weight": 1, "isBackup": true,
        }))
        .unwrap()];
        native_sort_accounts(&host, &mut accounts, &collection, None);
        assert_eq!(accounts[0].id, "pro", "backup with {strategy:?}");
        collection.custom_routing_rules.clear();
    }
}

#[test]
fn native_equal_priority_candidates_rotate_without_overriding_explicit_key_priority() {
    let host = NativeHostBridge::default();
    let collection = native_test_collection();
    let mut first = native_test_account();
    first.id = "first".into();
    let mut second = first.clone();
    second.id = "second".into();
    let base = vec![first, second];
    let mut accounts = base.clone();
    native_sort_accounts(&host, &mut accounts, &collection, None);
    assert_eq!(accounts[0].id, "first");
    let mut accounts = base.clone();
    native_sort_accounts(&host, &mut accounts, &collection, None);
    assert_eq!(accounts[0].id, "second");
    let mut key = native_test_key();
    key.priority_account_ids = vec!["first".into()];
    for _ in 0..2 {
        let mut accounts = base.clone();
        native_sort_accounts(&host, &mut accounts, &collection, Some(&key));
        assert_eq!(accounts[0].id, "first");
    }
}

#[test]
fn native_known_remote_revocation_is_filtered_but_usable_credits_remain_eligible() {
    let mut account = native_test_account_with_quota(0, 0);
    let collection = native_test_collection();
    assert!(!native_quota_allows(&collection, &account));
    account.quota.as_mut().unwrap().raw_data = Some(json!({
        "credits": {"balance": 1.5}
    }));
    assert!(native_quota_allows(&collection, &account));
    account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("token_revoked".into()),
        message: "Access token revoked".into(),
        timestamp: 1,
    });
    assert!(!native_eligible(&account));
    account.quota_error = Some(crate::models::codex::CodexQuotaErrorInfo {
        code: Some("refresh_token_reused".into()),
        message: "refresh token reused".into(),
        timestamp: 1,
    });
    assert!(native_eligible(&account));
}

#[tokio::test]
async fn native_websocket_routes_bridge_chat_and_keep_responses_model_mapping() {
    let mut host = native_test_routing_host();
    host.account_overrides
        .get_mut("pool-a")
        .unwrap()
        .api_wire_api = Some("chat_completions".into());
    let websocket_account = host.account_overrides.get_mut("pool-b").unwrap();
    websocket_account.api_supports_websockets = true;
    websocket_account.api_model_catalog = vec!["gpt-fixture".into()];
    websocket_account.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "gpt-fixture".into(),
        upstream_model: "actual-model".into(),
    }];
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_test_route_request("websocket-root");
    request.auxiliary = Some("responses.websocket".into());
    request.preferred_account_id = Some("pool-a".into());
    let route = native_select_route(&host, &caller, request.clone())
        .await
        .unwrap();
    assert_eq!(route.account_id, "pool-a");
    assert_eq!(route.wire_api, native_contract::WireApi::Chat);
    assert!(
        !native_contract::Host::responses_websocket_passthrough(&host, &caller, &route)
            .await
            .unwrap()
    );
    drop(route);
    request.preferred_account_id = Some("pool-b".into());
    let route = native_select_route(&host, &caller, request.clone())
        .await
        .unwrap();
    assert_eq!(route.model, "actual-model");
    assert!(
        native_contract::Host::responses_websocket_passthrough(&host, &caller, &route)
            .await
            .unwrap()
    );
    drop(route);
    host.account_overrides
        .get_mut("pool-b")
        .unwrap()
        .api_supports_websockets = false;
    let route = native_select_route(&host, &caller, request).await.unwrap();
    assert_eq!(route.model, "actual-model");
    assert!(
        !native_contract::Host::responses_websocket_passthrough(&host, &caller, &route)
            .await
            .unwrap()
    );
    drop(route);

    let mut key = native_test_key();
    key.provider_gateway = Some(serde_json::from_value(json!({
        "baseUrl": "https://example.invalid/v1", "apiKey": "fixture-key", "upstreamModel": "gpt-fixture"
    })).unwrap());
    host.collection_override.as_mut().unwrap().api_keys = vec![key.clone()];
    let mut request = native_test_route_request("provider-root");
    request.auxiliary = Some("responses.websocket".into());
    let provider_caller = native_test_caller(&key.id);
    let route = native_select_route(&host, &provider_caller, request)
        .await
        .unwrap();
    assert!(!native_contract::Host::responses_websocket_passthrough(
        &host,
        &provider_caller,
        &route
    )
    .await
    .unwrap());
}

#[tokio::test]
async fn native_image_first_source_uses_free_account_but_continuation_stays_pinned() {
    let mut host = native_test_routing_host();
    let collection = host.collection_override.as_mut().unwrap();
    collection.image_generation_account_ids = vec!["pool-a".into(), "pool-b".into()];
    collection.max_concurrent_image_requests = 0;
    for account in host.account_overrides.values_mut() {
        account.api_model_catalog = vec!["gpt-image-2.5".into()];
    }
    let collection = host.collection().await.unwrap();
    let account = host.account_overrides["pool-a"].clone();
    let (_, busy) = native_acquire_account_permit(&host, &collection, &[account], true)
        .await
        .unwrap();
    let caller = native_test_caller(NATIVE_PRIMARY_CALLER);
    let mut request = native_contract::ImageRouteRequest {
        model: Some("gpt-image-2.5".into()),
        responses_model: None,
        edit: false,
        target_account_id: None,
        force_refresh: false,
        preferred_account_id: None,
        excluded_account_ids: vec![],
    };
    let selected = native_image_route(&host, &caller, request.clone())
        .await
        .unwrap();
    assert_eq!(selected.route.account_id, "pool-b");
    drop(selected);
    request.preferred_account_id = Some("pool-a".into());
    let error = native_image_route(&host, &caller, request)
        .await
        .err()
        .unwrap();
    assert_eq!((error.status, error.code), (429, "account_busy"));
    drop(busy);
}

#[tokio::test]
async fn native_same_session_selection_is_serialized_without_blocking_other_callers() {
    let host = NativeHostBridge::default();
    let caller = native_test_caller("one");
    let request = native_test_route_request("session");
    let first = native_acquire_selection_guard(&host, &caller, &request, Duration::from_secs(1))
        .await
        .unwrap();
    let error = native_acquire_selection_guard(&host, &caller, &request, Duration::from_millis(20))
        .await
        .err()
        .unwrap();
    assert_eq!(error.code, "session_selection_busy");
    let other = native_acquire_selection_guard(
        &host,
        &native_test_caller("two"),
        &request,
        Duration::from_millis(20),
    )
    .await
    .unwrap();
    drop(other);
    drop(first);
    assert!(
        native_acquire_selection_guard(&host, &caller, &request, Duration::from_millis(20))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn native_context_rebuild_ignores_old_false_true_and_missing_configuration() {
    let baseline = native_test_collection();
    for old_value in [None, Some(false), Some(true)] {
        let mut raw = serde_json::to_value(&baseline).unwrap();
        if let Some(value) = old_value {
            raw["nativeAccountSwitchRebuild"] = json!(value);
        }
        let restored: CodexLocalAccessCollection = serde_json::from_value(raw).unwrap();
        assert_eq!(restored.account_ids, baseline.account_ids);
        assert_eq!(restored.api_key, baseline.api_key);
        assert_eq!(restored.gateway_mode, baseline.gateway_mode);
        assert!(serde_json::to_value(&restored).unwrap().get("nativeAccountSwitchRebuild").is_none());
        let host = NativeHostBridge::for_collection(restored);
        assert!(native_contract::Host::execution_policy(&host).await.unwrap().account_switch_rebuild);
    }
    assert!(native_contract::ExecutionPolicy::default().account_switch_rebuild);
}

#[tokio::test]
async fn native_context_rebuild_stale_false_does_not_survive_collection_save() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    for mode in [CodexLocalAccessGatewayMode::Native, CodexLocalAccessGatewayMode::Sidecar] {
        let mut stale = native_test_collection();
        stale.gateway_mode = mode;
        let mut original = serde_json::to_value(&stale).unwrap();
        original["nativeAccountSwitchRebuild"] = json!(false);
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        stale.port = 16666;
        save_collection_to_path(&path, &stale).unwrap();
        let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(persisted.get("nativeAccountSwitchRebuild").is_none());
        let restored: CodexLocalAccessCollection = serde_json::from_value(persisted).unwrap();
        assert_eq!(restored.gateway_mode, mode, "No automatic engine switch");
        assert_eq!(restored.port, 16666);
        assert_eq!(restored.account_ids, stale.account_ids);
        assert_eq!(restored.api_key, stale.api_key);
        preserve_runtime_collection_fields(&mut stale, &restored);
        let host = NativeHostBridge::for_collection(stale);
        assert!(native_contract::Host::execution_policy(&host).await.unwrap().account_switch_rebuild);
    }
}

#[tokio::test]
async fn native_context_rebuild_preserves_existing_retry_limits() {
    // The removed setting no longer starts a save worker. Automatic rebuilding
    // must still obey the existing request retry and wait budgets.
    for credentials in [0, 1, 4] {
        let mut collection = native_test_collection();
        collection.max_retry_credentials = credentials;
        collection.max_retry_interval_ms = 720;
        let host = NativeHostBridge::for_collection(collection);
        let policy = native_contract::Host::execution_policy(&host).await.unwrap();
        assert!(policy.account_switch_rebuild);
        assert_eq!(policy.credential_retries, credentials);
        assert_eq!(policy.retry_max_delay, Duration::from_millis(720));
    }
}

#[tokio::test]
async fn go_limits_host_ignores_legacy_total_timeout_and_wait_queue_releases_on_drop() {
    let mut collection = native_test_collection();
    collection.timeouts.legacy_stream_total_timeout_ms = 300_000;
    let host = NativeHostBridge::for_collection(collection);
    let policy = native_contract::Host::execution_policy(&host).await.unwrap();
    assert!(policy.total_timeout.is_zero());
    assert_eq!(policy.open_timeout, Duration::from_secs(60));
    assert_eq!(policy.idle_timeout, Duration::from_secs(120));
    let slots: Vec<_> = (0..100).map(|_| native_account_wait_slot(&host).unwrap()).collect();
    assert_eq!(native_account_wait_slot(&host).err().unwrap().code, "account_busy");
    drop(slots);
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
    let slot = native_account_wait_slot(&host).unwrap();
    drop(slot);
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
    let mut collection = native_test_collection();
    collection.max_account_concurrency = 1;
    collection.account_concurrency_wait_ms = 10_000;
    let accounts = [native_test_account()];
    let (_, busy) = native_acquire_account_concurrency_permit(&host, &collection, &accounts, true).await.unwrap();
    let mut waiting = Box::pin(native_acquire_account_concurrency_permit(&host, &collection, &accounts, true));
    tokio::select! {
        result = &mut waiting => panic!("occupied account unexpectedly resolved: {}", result.is_ok()),
        _ = tokio::time::sleep(Duration::from_millis(10)) => {}
    }
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 1);
    drop(waiting);
    assert_eq!(host.account_waiters.load(Ordering::Acquire), 0);
    drop(busy);
}
