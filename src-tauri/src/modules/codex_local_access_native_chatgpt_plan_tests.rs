use super::*;
include!("codex_local_access_native_chatgpt_plan_cold_tests.rs");

fn account() -> CodexAccount {
    CodexAccount::new_api_key(
        "chatgpt_plan_test-registration".into(),
        "plan@example.invalid".into(),
        "chatgpt-plan-managed".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://api.openai.com/v1".into()),
        Some("cockpit_chatgpt_plan".into()),
        Some("ChatGPT".into()),
        vec!["gpt-account-exclusive".into(), "gpt-6.1-sol".into()],
    )
}

fn collection() -> CodexLocalAccessCollection {
    serde_json::from_value(json!({"enabled":true,"port":19001,"apiKey":"local-test-key","accountIds":["chatgpt_plan_test-registration"],"createdAt":0,"updatedAt":0})).unwrap()
}

fn mixed_oauth_host(projection: CodexAccount, scope: Vec<String>) -> NativeHostBridge {
    let collection = serde_json::from_value(json!({
        "enabled":true,"port":19009,"apiKey":"synthetic-mixed-key",
        "accountIds":[projection.id,"ordinary-key"],"createdAt":1,"updatedAt":1,
        "sessionAffinity":false,
        "apiKeys":[{"id":"mixed-caller","label":"Mixed","key":"synthetic-mixed-key",
            "inheritAccountPool":false,"accountIds":scope,
            "modelRouting":{"defaultRoute":"oauth","failurePolicy":"strict","routes":[]}}]
    }))
    .unwrap();
    let mut ordinary = projection.clone();
    ordinary.id = "ordinary-key".into();
    ordinary.api_provider_id = Some("ordinary-provider".into());
    ordinary.openai_api_key = Some("synthetic-ordinary-provider-key".into());
    let mut host = NativeHostBridge::for_collection(collection);
    host.account_overrides
        .insert(projection.id.clone(), projection);
    host.account_overrides.insert(ordinary.id.clone(), ordinary);
    host
}

fn mixed_request(model: &str) -> native_contract::RouteRequest {
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

/// Called by the real loopback authorization/service test after verification.
/// This uses the production Host and independent credential store, not a fake
/// routing host or credentials stub; it sends no inference request.
pub(crate) async fn assert_chatgpt_plan_mixed_key_real_host(projection_id: &str) {
    let source = codex_account::load_account(projection_id).unwrap();
    let bindings = chatgpt_plan_selected_bindings().await.unwrap();
    let registration = bindings[projection_id].clone();
    let overlay = crate::modules::chatgpt_plan::official_runtime_overlay(source.clone(), &registration);
    let model = overlay.api_model_catalog[0].clone();
    let mut host = mixed_oauth_host(source, vec![projection_id.into()]);
    host.collection_override.as_mut().unwrap().official_account_bindings = bindings;
    let caller = native_contract::Host::authorize(&host, "synthetic-mixed-key")
        .await
        .unwrap();
    let catalog = native_contract::Host::models(&host, &caller).await.unwrap();
    assert!(catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["id"] == model));
    let route = native_contract::Host::route(&host, &caller, mixed_request(&model))
        .await
        .unwrap();
    let policy = native_contract::Host::multi_agent_policy(&host, &caller, None, &model).await.unwrap();
    assert!(policy.models.iter().any(|entry| entry.id == model));
    assert!(!policy.options.is_compat);
    let selected = native_contract::Host::multi_agent_policy(&host, &caller, Some(&route), &model).await.unwrap();
    assert!(selected.options.optimize);
    assert!(!selected.options.is_compat);
    assert_eq!(route.account_id, projection_id);
    assert_eq!(route.continuity_id(), format!("chatgpt_plan_{registration}"));
    assert_eq!(route.official_registration_id.as_deref(), Some(registration.as_str()));
    assert!(route.chatgpt_plan);
    assert!(!route.chatgpt);
    assert_eq!(route.base_url, "https://api.openai.com/v1");
    assert!(route.headers[AUTHORIZATION].is_sensitive());
    assert!(!route.headers[AUTHORIZATION]
        .to_str()
        .unwrap()
        .contains("chatgpt-plan-managed"));
    // Old checkpoints keep the technical grant namespace while every caller
    // scope and retry budget continues to refer to the original member.
    let mut pinned = mixed_request(&model);
    pinned.preferred_account_id = Some(route.continuity_id().to_owned());
    let resumed = native_contract::Host::route(&host, &caller, pinned.clone()).await.unwrap();
    assert_eq!(resumed.account_id, projection_id);
    assert_eq!(resumed.continuity_id(), route.continuity_id());
    assert_eq!(resumed.owner_id, route.owner_id);
    for excluded in [projection_id.to_owned(), route.continuity_id().to_owned()] {
        let mut request = mixed_request(&model); request.excluded_account_ids.push(excluded);
        assert!(native_contract::Host::route(&host, &caller, request).await.is_err());
    }
    host.collection_override.as_mut().unwrap().paused_account_ids.push(projection_id.into());
    assert_eq!(native_contract::Host::route(&host, &caller, pinned.clone()).await.err().unwrap().code, "account_binding_unavailable");
    host.collection_override.as_mut().unwrap().paused_account_ids.clear();
    host.collection_override.as_mut().unwrap().official_account_bindings.clear();
    assert_eq!(native_contract::Host::route(&host, &caller, pinned).await.err().unwrap().code, "account_binding_unavailable");
    assert!(host.load_route_account(&route).await.is_err());

}

#[tokio::test]
async fn chatgpt_plan_mixed_default_oauth_real_host_excludes_ordinary_keys_and_keeps_scope() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // A valid plan account is present in the service, but this key explicitly
    // authorizes only an ordinary provider. It must not escape that scope.
    let host = mixed_oauth_host(account(), vec!["ordinary-key".into()]);
    let caller = native_contract::Host::authorize(&host, "synthetic-mixed-key")
        .await
        .unwrap();
    let catalog = native_contract::Host::models(&host, &caller).await.unwrap();
    assert!(catalog["data"].as_array().unwrap().is_empty());
    let policy = native_contract::Host::multi_agent_policy(&host, &caller, None, "gpt-account-exclusive").await.unwrap();
    assert!(policy.models.is_empty());
    let error =
        native_contract::Host::route(&host, &caller, mixed_request("gpt-account-exclusive"))
            .await
            .err()
            .unwrap();
    assert_eq!(error.code, "no_available_account");
    assert_eq!(error.status, 503);
}

#[test]
fn chatgpt_plan_catalog_preserves_server_models_and_never_falls_back_to_codex_models() {
    let collection = collection();
    let mut account = account();
    let catalog = native_catalog(&collection, None, &[account.clone()]).unwrap();
    let ids = catalog["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| model["id"].as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"gpt-account-exclusive"));
    assert!(ids.contains(&"gpt-6.1-sol"));
    assert!(!ids.contains(&"gpt-reserve"));
    for id in ids {
        assert!(account.api_model_catalog.iter().any(|model| model == id));
    }
    let plan = native_model_route_plan(&collection, None, "unavailable-model").unwrap();
    assert!(native_plan_account_model(&plan, &account).is_none());
    account.api_model_catalog.clear();
    assert!(native_plan_account_model(&plan, &account).is_none());
}

#[test]
fn chatgpt_plan_explicit_provider_routes_cannot_broaden_the_official_model_catalog() {
    let collection = collection();
    let account = account();
    let mut plan = native_model_route_plan(&collection, None, "unavailable-model").unwrap();
    plan.provider = true;
    assert!(native_plan_account_model(&plan, &account).is_none());
    plan.model = "gpt-6.1-sol".into();
    assert_eq!(
        native_plan_account_model(&plan, &account).as_deref(),
        Some("gpt-6.1-sol")
    );
}

#[test]
fn chatgpt_plan_accounts_admit_only_responses_transport_and_never_hosted_auxiliary() {
    let mut account = account();
    let mut collection = collection();
    account.api_supports_websockets = true;
    account.api_wire_api = Some("chat_completions".into());
    assert!(native_auxiliary_eligible(&account, None));
    assert!(native_auxiliary_eligible(
        &account,
        Some("responses.websocket")
    ));
    for operation in ["audio.transcriptions", "audio.speech", "search", "realtime"] {
        assert!(
            !native_auxiliary_eligible(&account, Some(operation)),
            "{operation}"
        );
    }
    assert_eq!(
        native_wire_api(&account).unwrap(),
        native_contract::WireApi::Responses
    );
    let plan = native_model_route_plan(&collection, None, "gpt-6.1-sol").unwrap();
    assert!(!native_catalog_prefers_websockets(
        &collection,
        None,
        &plan,
        &account
    ));
    collection
        .image_generation_account_ids
        .push(account.id.clone());
    collection.image_generation_account_policies.insert(
        account.id.clone(),
        CodexLocalAccessImageGenerationPolicy::Enabled,
    );
    assert!(!native_image_source_allowed(
        &collection,
        &account,
        "gpt-image-2"
    ));
    // A forged policy or catalog must not turn a plan registration into an
    // image source in native routing, provider settings, or sidecar manifests.
    account.api_model_catalog.push("gpt-image-2".into());
    assert!(!native_image_allows(&collection, &account));
    assert!(!is_image_generation_source_for_model(
        &account,
        "gpt-image-2"
    ));
    assert_eq!(
        image_generation_manifest_policy(&account, &collection),
        "disabled"
    );
    assert!(image_generation_upstream_manifest_value(&account, &collection, None).is_none());
}

#[tokio::test]
async fn chatgpt_plan_standalone_search_rejection_identifies_the_gateway_endpoint_limit() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let projection = account();
    let host = mixed_oauth_host(projection.clone(), vec![projection.id]);
    let caller = native_contract::Host::authorize(&host, "synthetic-mixed-key").await.unwrap();
    let mut request = mixed_request("gpt-account-exclusive");
    request.auxiliary = Some("search".into());
    let error = native_contract::Host::route(&host, &caller, request).await.err().expect("standalone search must remain rejected");
    assert_eq!(error.status, 400);
    assert_eq!(error.code, "subscription_sharing_unsupported_capability");
    assert_eq!(error.response_error.as_ref().unwrap()["param"], "endpoint");
    assert!(error.message.contains("gateway's official-authorization route"));
    assert!(error.message.contains("/alpha/search"));
    assert!(error.message.contains("Responses web_search is a separate capability"));
}

#[test]
fn chatgpt_plan_image_policy_is_disabled_on_cold_read_and_stale_snapshot_save() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("collection.json");
    let mut stale = collection();
    let id = stale.account_ids[0].clone();
    stale
        .image_generation_account_policies
        .insert(id.clone(), CodexLocalAccessImageGenerationPolicy::Enabled);
    stale.image_generation_account_ids.push(id.clone());
    stale
        .image_generation_account_models
        .insert(id.clone(), "gpt-image-2".into());
    std::fs::write(&path, serde_json::to_vec(&stale).unwrap()).unwrap();
    let mut loaded = load_collection_from_path(&path).unwrap().unwrap();
    // Cold-start initialization uses structure-only normalization before
    // publishing and persisting runtime, without loading account secrets.
    assert!(sanitize_collection_structure(&mut loaded).unwrap());
    assert_eq!(
        loaded.image_generation_account_policies[&id],
        CodexLocalAccessImageGenerationPolicy::Disabled
    );
    assert!(loaded.image_generation_account_ids.is_empty());
    assert!(loaded.image_generation_account_models.is_empty());
    save_collection_to_path(&path, &stale).unwrap();
    let saved: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved.image_generation_account_policies[&id],
        CodexLocalAccessImageGenerationPolicy::Disabled
    );
    assert!(saved.image_generation_account_ids.is_empty());
    assert!(saved.image_generation_account_models.is_empty());
}

#[test]
fn chatgpt_plan_image_policy_uses_account_type_and_preserves_other_accounts() {
    let mut account = account();
    account.id = "legacy-plan-reference".into();
    let mut collection = collection();
    collection.account_ids = vec![account.id.clone(), "regular-api".into()];
    collection.image_generation_account_policies.insert(
        "regular-api".into(),
        CodexLocalAccessImageGenerationPolicy::Enabled,
    );
    assert!(enforce_chatgpt_plan_image_policy(
        &mut collection,
        &[account.clone()]
    ));
    assert_eq!(
        collection.image_generation_account_policies[&account.id],
        CodexLocalAccessImageGenerationPolicy::Disabled
    );
    assert_eq!(
        collection.image_generation_account_policies["regular-api"],
        CodexLocalAccessImageGenerationPolicy::Enabled
    );
    assert!(!enforce_chatgpt_plan_image_policy(
        &mut collection,
        &[account]
    ));
}

#[test]
fn chatgpt_plan_image_health_is_disabled_before_any_request() {
    let mut runtime = GatewayRuntime {
        collection: Some(collection()),
        ..Default::default()
    };
    let id = runtime.collection.as_ref().unwrap().account_ids[0].clone();
    runtime
        .account_health
        .entry(id)
        .or_default()
        .image_generation_status = CodexLocalAccessImageGenerationStatus::Available;
    let health = build_account_health_snapshot(&runtime);
    assert_eq!(
        health[0].image_generation_status,
        CodexLocalAccessImageGenerationStatus::Disabled
    );
    assert!(health[0].image_generation_checked_at.is_none());
    assert!(health[0].available);
}

#[test]
fn chatgpt_plan_sidecar_refuses_before_exporting_the_placeholder_credential() {
    let root = tempfile::tempdir().unwrap();
    let account = account();
    let error = prepare_sidecar_launch_config_in_dir_sync(
        &collection(),
        root.path().to_path_buf(),
        HashMap::new(),
        None,
        HashMap::from([(account.id.clone(), account)]),
        true,
        None,
    )
    .err()
    .expect("plan grants need native engine");
    assert_eq!(error, "CHATGPT_PLAN_NATIVE_REQUIRED");
    assert!(!sidecar_auths_dir(root.path()).exists());
}

#[test]
fn chatgpt_plan_refresh_network_failures_do_not_require_reauthorization() {
    for code in [
        "CHATGPT_PLAN_NETWORK",
        "CHATGPT_PLAN_STORAGE",
        "CHATGPT_PLAN_BUSY",
        "CHATGPT_PLAN_TIMEOUT",
    ] {
        let error = native_credential_error(code);
        assert!(matches!(error.status, 503 | 504));
        assert!(!error.credential_scope);
    }
    let error = native_credential_error("CHATGPT_PLAN_REAUTH_REQUIRED");
    assert_eq!(error.status, 401);
    assert!(error.credential_scope);
}

#[test]
fn chatgpt_plan_engine_mode_guards_preserve_disk_config_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("collection.json");
    let mut collection = collection();
    collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
    let original = serde_json::to_string(&collection).unwrap();
    std::fs::write(&path, &original).unwrap();
    for mode in [
        CodexLocalAccessGatewayMode::Sidecar,
        CodexLocalAccessGatewayMode::Legacy,
    ] {
        assert_eq!(
            native_patch_mode(&original, mode).unwrap_err(),
            "CHATGPT_PLAN_NATIVE_REQUIRED"
        );
    }
    assert!(native_patch_mode(&original, CodexLocalAccessGatewayMode::Native).is_ok());
    // Joining with a stale Native snapshot after Sidecar was persisted cannot
    // write an unsupported grant into that engine's collection.
    let mut existing = collection.clone();
    existing.account_ids.clear();
    existing.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
    let existing = serde_json::to_string(&existing).unwrap();
    std::fs::write(&path, &existing).unwrap();
    assert_eq!(
        save_collection_to_path(&path, &collection).unwrap_err(),
        "CHATGPT_PLAN_NATIVE_REQUIRED"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), existing);
}

#[test]
fn chatgpt_plan_engine_guard_checks_key_specific_accounts_too() {
    let mut collection = collection();
    let account_id = collection.account_ids.remove(0);
    collection.api_keys = vec![serde_json::from_value(json!({"id":"plan-key", "label":"plan", "key":"local-test", "inheritAccountPool":false,"accountIds":[account_id]})).unwrap()];
    assert!(
        native_validate_chatgpt_plan_mode(&collection, CodexLocalAccessGatewayMode::Sidecar)
            .is_err()
    );
}

struct PlanAccountDataDir {
    root: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
}

impl PlanAccountDataDir {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", root.path());
        Self { root, previous }
    }
}

impl Drop for PlanAccountDataDir {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", previous);
        } else {
            std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn chatgpt_plan_same_native_host_reads_updated_catalog_without_listener_reload() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let directory = PlanAccountDataDir::new();
    let mut account = account();
    let accounts = directory.root.path().join("codex_accounts");
    std::fs::create_dir_all(&accounts).unwrap();
    let path = accounts.join(format!("{}.json", account.id));
    std::fs::write(&path, serde_json::to_vec(&account).unwrap()).unwrap();
    let host = NativeHostBridge::default();
    let first = host.load_accounts(vec![account.id.clone()]).await.unwrap();
    assert_eq!(first[0].api_model_catalog, account.api_model_catalog);
    account.api_model_catalog = vec!["fresh-account-model".into()];
    std::fs::write(&path, serde_json::to_vec(&account).unwrap()).unwrap();
    let next = host.load_accounts(vec![account.id.clone()]).await.unwrap();
    assert_eq!(next[0].api_model_catalog, vec!["fresh-account-model"]);
}
