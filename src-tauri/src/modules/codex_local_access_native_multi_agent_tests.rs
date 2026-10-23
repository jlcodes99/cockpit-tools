use super::*;

#[test]
fn multi_agent_catalog_sorting_preserves_profiles_and_no_synthetic_tiers() {
    let profiles = native_multi_agent_profiles(&json!({"models":[
        {"slug":"z-unknown","display_name":"Zoo","service_tiers":[{"id":"priority"}],"supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"}]},
        {"slug":"a-unknown","display_name":"Alpha","description":"Visible custom model"}
    ]}));
    assert_eq!(
        profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-unknown", "z-unknown"]
    );
    assert!(profiles[1].service_tiers.is_empty());
    assert_eq!(profiles[1].default_reasoning_effort, "medium");
}

#[test]
fn multi_agent_sidecar_compat_is_automatic_without_narrowing_model_catalog() {
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.enabled = true;
    collection.multi_agent =
        serde_json::from_value(json!({"compatModels":[{"accountId":"a","model":"custom"}]}))
            .unwrap();
    let mut account = CodexAccount::new_api_key(
        "a".into(),
        "synthetic@example.invalid".into(),
        "synthetic-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        None,
        None,
        vec![],
    );
    account.api_wire_api = Some("responses".into());
    let generated = sidecar_codex_key_config_value(&account, &collection, None).unwrap();
    assert!(generated.get("cockpit-model-compat").is_none());
    assert_eq!(generated["models"], json!([]));
    collection.model_aliases = serde_json::from_value(json!([{
        "sourceModel":"upstream","alias":"visible","fork":false
    }]))
    .unwrap();
    let aliased = sidecar_codex_key_config_value(&account, &collection, None).unwrap();
    assert_eq!(
        aliased["models"],
        json!([{
            "name":"upstream","alias":"visible","is-compat":true
        }])
    );
    let auth = sidecar_auth_json_for_account(&account, &collection, None);
    assert!(auth.get("cockpit_model_compat").is_none());
    account.id = "another".into();
    let auth = sidecar_auth_json_for_account(&account, &collection, None);
    assert!(auth.get("cockpit_model_compat").is_none());
}

#[tokio::test]
async fn multi_agent_host_policy_ignores_legacy_switches_and_uses_selected_protocol() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.enabled = true;
    collection.account_ids = vec!["visible".into()];
    collection.multi_agent = serde_json::from_value(
        json!({"optimize":false,"orphanDelegation":false,"compatModels":[]}),
    )
    .unwrap();
    collection.multi_agent.optimize = false;
    collection.multi_agent.orphan_delegation = false;
    let mut host = NativeHostBridge::for_collection(collection);
    for id in ["visible", "hidden"] {
        let mut account = CodexAccount::new_api_key(
            id.into(),
            "synthetic@example.invalid".into(),
            "synthetic-key".into(),
            crate::models::codex::CodexApiProviderMode::Custom,
            Some("https://example.invalid/v1".into()),
            None,
            None,
            vec!["custom".into()],
        );
        account.api_wire_api = Some("responses".into());
        account.egress_proxy_disabled = true;
        host.account_overrides.insert(id.into(), account);
    }
    let caller = native_contract::Caller {
        id: NATIVE_PRIMARY_CALLER.into(),
        label: "fixture".into(),
    };
    let policy = native_contract::Host::multi_agent_policy(&host, &caller, None, "custom")
        .await
        .unwrap();
    assert!(policy.options.optimize && policy.options.orphan_delegation);
    assert!(!policy.options.is_compat);
    assert!(policy.models.iter().any(|model| model.id == "custom"));
    let request = native_contract::RouteRequest {
        model: "custom".into(),
        session_id: None,
        thread_id: None,
        parent_thread_id: None,
        preferred_account_id: Some("visible".into()),
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: false,
        auxiliary: None,
        required_capabilities: vec![],
        capability_fallback_from: None,
    };
    let mut route = native_contract::Host::route(&host, &caller, request.clone())
        .await
        .unwrap();
    let policy = native_contract::Host::multi_agent_policy(&host, &caller, Some(&route), "custom")
        .await
        .unwrap();
    assert!(policy.options.is_compat);
    let mut denied = request;
    denied.preferred_account_id = Some("hidden".into());
    assert!(native_contract::Host::route(&host, &caller, denied)
        .await
        .is_err());
    // A selected official route must retain the private Codex envelope even
    // when the model is also available through a public provider.
    route.chatgpt = true;
    assert!(
        !native_contract::Host::multi_agent_policy(&host, &caller, Some(&route), "custom")
            .await
            .unwrap()
            .options
            .is_compat
    );
}

fn request(model: &str) -> native_contract::RouteRequest {
    native_contract::RouteRequest {
        model: model.into(),
        session_id: None,
        thread_id: None,
        parent_thread_id: None,
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: vec![],
        requires_permit: false,
        auxiliary: None,
        required_capabilities: vec![],
        capability_fallback_from: None,
    }
}

fn key_account(id: &str, models: &[&str]) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        id.into(),
        "fixture@example.invalid".into(),
        "synthetic-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        None,
        None,
        models.iter().map(|model| model.to_string()).collect(),
    );
    account.api_wire_api = Some("responses".into());
    account.egress_proxy_disabled = true;
    account
}

#[tokio::test]
async fn multi_agent_policy_preserves_provider_snapshot_grants_and_namespaced_catalog() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.enabled = true;
    collection.account_ids = vec!["conversation".into(), "provider".into()];
    collection.api_keys = serde_json::from_value(json!([{
        "id":"scoped", "label":"scoped", "key":"synthetic-scoped", "inheritAccountPool":false,
        "accountIds":["conversation"], "modelPrefix":"team",
        "modelRouting":{"defaultRoute":"oauth","failurePolicy":"strict","routes":[{
            "id":"provider-route","namespace":"vendor","providerAccountId":"provider",
            "providerGateway":{"baseUrl":"https://snapshot.invalid/v1","apiKey":"snapshot-key","upstreamModel":"custom","upstreamModels":["custom"],"wireApi":"responses"}
        }]}
    }])).unwrap();
    collection.multi_agent =
        serde_json::from_value(json!({"compatModels":[{"accountId":"provider","model":"custom"}]}))
            .unwrap();
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides.insert(
        "conversation".into(),
        key_account("conversation", &["conversation-only"]),
    );
    host.account_overrides
        .insert("provider".into(), key_account("provider", &["custom"]));
    let caller = native_contract::Host::authorize(&host, "synthetic-scoped")
        .await
        .unwrap();
    let public = "team/vendor/custom";
    let boundary = native_contract::Host::multi_agent_policy(&host, &caller, None, public)
        .await
        .unwrap();
    assert!(boundary.models.iter().any(|entry| entry.id == public));
    assert!(!boundary.models.iter().any(|entry| entry.id == "custom"));
    let route = native_contract::Host::route(&host, &caller, request(public))
        .await
        .unwrap();
    assert_eq!(route.account_id, "provider");
    // Provider snapshots define the exposed model route; credentials/base URL
    // continue to come from the currently bound account.
    assert_eq!(route.base_url, "https://example.invalid/v1");
    let selected = native_contract::Host::multi_agent_policy(&host, &caller, Some(&route), public)
        .await
        .unwrap();
    assert!(selected.options.is_compat);
    assert!(
        native_contract::Host::route(&host, &caller, request("team/custom"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn multi_agent_policy_does_not_reject_independent_image_source_pool() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.enabled = true;
    collection.account_ids = vec!["conversation".into()];
    collection.image_generation_account_ids = vec!["image".into()];
    collection
        .image_generation_account_models
        .insert("image".into(), "gpt-image-2.5".into());
    collection.image_generation_account_policies.insert(
        "image".into(),
        CodexLocalAccessImageGenerationPolicy::Enabled,
    );
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides.insert(
        "conversation".into(),
        key_account("conversation", &["custom"]),
    );
    host.account_overrides
        .insert("image".into(), key_account("image", &["gpt-image-2.5"]));
    let caller = native_contract::Caller {
        id: NATIVE_PRIMARY_CALLER.into(),
        label: "fixture".into(),
    };
    let image = native_contract::Host::image_route(
        &host,
        &caller,
        native_contract::ImageRouteRequest {
            model: Some("gpt-image-2.5".into()),
            responses_model: Some("custom".into()),
            edit: false,
            target_account_id: None,
            force_refresh: false,
            preferred_account_id: None,
            excluded_account_ids: vec![],
        },
    )
    .await
    .unwrap();
    assert_eq!(image.route.account_id, "image");
    let policy =
        native_contract::Host::multi_agent_policy(&host, &caller, Some(&image.route), "custom")
            .await
            .unwrap();
    assert!(policy.options.optimize);
    assert!(policy.options.is_compat);
}

#[tokio::test]
async fn multi_agent_sidecar_generation_ignores_saved_disabled_flags_and_model_list() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = tempfile::tempdir().unwrap();
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.enabled = true;
    collection.account_ids = vec!["public".into()];
    collection.multi_agent = serde_json::from_value(json!({
        "optimize":false,"orphanDelegation":false,"compatModels":[]
    }))
    .unwrap();
    collection.multi_agent.optimize = false;
    collection.multi_agent.orphan_delegation = false;
    let mut account = key_account("public", &["custom"]);
    // Instance/provider gateway generation exports explicit model mappings, not
    // the API-service catalog expansion. Keep a real mapping in this fixture so
    // its automatic protocol flag is exercised without a compatibility list.
    account.api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "visible-custom".into(),
        upstream_model: "custom".into(),
    }];
    let launch = prepare_sidecar_launch_config_in_dir(
        &collection,
        directory.path().to_path_buf(),
        HashMap::new(),
        None,
        HashMap::from([("public".into(), account)]),
    )
    .await
    .unwrap();
    let generated: Value =
        serde_json::from_str(&fs::read_to_string(launch.config_path).unwrap()).unwrap();
    assert_eq!(generated["codex"]["optimize-multi-agent-v2"], true);
    assert_eq!(generated["codex"]["orphan-delegation-compatibility"], true);
    assert_eq!(
        generated["codex-api-key"][0]["models"],
        json!([{"name":"custom","alias":"visible-custom","is-compat":true}])
    );
    assert!(generated["codex-api-key"][0]
        .get("cockpit-model-compat")
        .is_none());
}
