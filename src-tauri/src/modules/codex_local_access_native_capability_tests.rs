use super::*;

fn account(id: &str) -> CodexAccount {
    let mut account = CodexAccount::new_api_key(
        id.into(),
        "fixture@example.invalid".into(),
        "synthetic-key".into(),
        crate::models::codex::CodexApiProviderMode::Custom,
        Some("https://example.invalid/v1".into()),
        None,
        None,
        vec!["gpt-fixture".into()],
    );
    account.api_wire_api = Some("responses".into());
    account.egress_proxy_disabled = true;
    account
}

fn host() -> NativeHostBridge {
    let collection: CodexLocalAccessCollection = serde_json::from_value(json!({
        "enabled":true,"port":1455,"apiKey":"synthetic-key","accountIds":["a","b","c"],
        "createdAt":1,"updatedAt":1,"gatewayMode":"native","sessionAffinity":false,
        "capabilityFallback":{"enabled":true,"accountIds":["c","b"],"overrides":[
            {"accountId":"a","model":"*","capabilities":{"function":"unsupported"}},
            {"accountId":"b","model":"*","capabilities":{"function":"supported"}},
            {"accountId":"c","model":"*","capabilities":{"function":"supported"}}
        ]}
    }))
    .unwrap();
    let mut host = NativeHostBridge::for_collection(collection);
    for id in ["a", "b", "c"] {
        host.account_overrides.insert(id.into(), account(id));
    }
    host
}

fn caller() -> native_contract::Caller {
    native_contract::Caller {
        id: NATIVE_PRIMARY_CALLER.into(),
        label: "fixture".into(),
    }
}

fn request() -> native_contract::RouteRequest {
    native_contract::RouteRequest {
        model: "gpt-fixture".into(),
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
        required_capabilities: vec!["function".into()],
        capability_fallback_from: None,
    }
}

async fn select(
    host: &NativeHostBridge,
    request: native_contract::RouteRequest,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    native_contract::Host::route(host, &caller(), request).await
}

#[test]
fn capability_knowledge_is_conservative_exact_per_capability_and_non_overridable() {
    let mut host = host();
    let mut collection = host.collection_override.take().unwrap();
    let account = &host.account_overrides["b"];
    assert_eq!(
        native_capability_support(&collection, account, "gpt-fixture", "function"),
        Some(CodexLocalAccessCapabilitySupport::Supported)
    );
    assert_eq!(
        native_capability_support(&collection, account, "gpt-fixture", "web_search"),
        None
    );
    collection.capability_fallback.overrides.push(serde_json::from_value(json!({
        "accountId":"b","model":"gpt-fixture","capabilities":{"function":"unsupported","file_search":"supported"}
    })).unwrap());
    assert_eq!(
        native_capability_support(&collection, account, "gpt-fixture", "function"),
        Some(CodexLocalAccessCapabilitySupport::Unsupported)
    );
    assert_eq!(
        native_capability_support(&collection, account, "GPT-FIXTURE", "function"),
        Some(CodexLocalAccessCapabilitySupport::Supported)
    );
    let mut chat = account.clone();
    chat.api_wire_api = Some("chat_completions".into());
    assert_eq!(
        native_capability_support(&collection, &chat, "gpt-fixture", "file_search"),
        Some(CodexLocalAccessCapabilitySupport::Unsupported)
    );
    let mut plan = account.clone();
    plan.api_provider_id = Some("cockpit_chatgpt_plan".into());
    assert_eq!(
        native_capability_support(&collection, &plan, "gpt-fixture", "file_search"),
        Some(CodexLocalAccessCapabilitySupport::Unsupported)
    );
    assert_eq!(
        native_capability_support(&collection, &plan, "gpt-fixture", "unknown_tool"),
        None
    );
    assert_eq!(
        native_capability_support(&collection, &plan, "gpt-fixture", "web_search"),
        None
    );
}

#[tokio::test]
async fn real_native_host_automatically_uses_scoped_backups_despite_legacy_opt_out_or_missing_declarations() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    let initial = select(&host, request()).await.unwrap();
    assert_eq!(initial.account_id, "a");
    let decision =
        native_contract::Host::capability_fallback_decision(&host, &caller(), &initial, &request())
            .await
            .unwrap();
    assert!(decision.enabled);
    assert_eq!(decision.unsupported, vec!["function"]);
    host.collection_override
        .as_mut()
        .unwrap()
        .capability_fallback
        .enabled = false;
    let decision =
        native_contract::Host::capability_fallback_decision(&host, &caller(), &initial, &request())
            .await
            .unwrap();
    assert!(decision.enabled);
    assert_eq!(decision.unsupported, vec!["function"]);
    host.collection_override
        .as_mut()
        .unwrap()
        .capability_fallback
        .enabled = true;
    let mut backup = request();
    backup.capability_fallback_from = Some("a".into());
    assert_eq!(select(&host, backup.clone()).await.unwrap().account_id, "c");
    backup.excluded_account_ids.push("c".into());
    assert_eq!(select(&host, backup.clone()).await.unwrap().account_id, "b");
    host.collection_override
        .as_mut()
        .unwrap()
        .capability_fallback
        .overrides
        .retain(|entry| entry.account_id != "b");
    assert_eq!(select(&host, backup.clone()).await.unwrap().account_id, "b");
    // Unknown primary capability leaves normal selection available.
    let mut unknown = request();
    unknown.required_capabilities = vec!["web_search".into()];
    let route = select(&host, unknown.clone()).await.unwrap();
    let decision =
        native_contract::Host::capability_fallback_decision(&host, &caller(), &route, &unknown)
            .await
            .unwrap();
    assert!(decision.unsupported.is_empty());
    unknown.capability_fallback_from = Some("a".into());
    assert!(matches!(select(&host, unknown).await.unwrap().account_id.as_str(), "b" | "c"));
}

#[tokio::test]
async fn automatic_capability_backups_need_no_secondary_account_list_and_never_expand_key_scope() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    let collection = host.collection_override.as_mut().unwrap();
    collection.capability_fallback = Default::default();
    collection.capability_fallback.enabled = false;
    collection.api_keys = vec![serde_json::from_value(json!({
        "id":"scoped","label":"fixture","key":"synthetic","accountIds":["a","b"]
    })).unwrap()];
    let scoped = native_contract::Caller { id: "scoped".into(), label: "fixture".into() };
    let mut request = request();
    request.capability_fallback_from = Some("a".into());
    let route = native_contract::Host::route(&host, &scoped, request.clone()).await.unwrap();
    assert_eq!(route.account_id, "b");
    request.excluded_account_ids.push("b".into());
    assert_eq!(native_contract::Host::route(&host, &scoped, request).await.err().unwrap().code,
        "capability_fallback_no_compatible_account");
}

#[tokio::test]
async fn real_native_backup_selection_preserves_key_scope_catalog_model_rules_and_hard_pin() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    let mut backup = request();
    backup.capability_fallback_from = Some("a".into());
    host.collection_override.as_mut().unwrap().api_keys = vec![serde_json::from_value(json!({
        "id":"scoped","label":"fixture","key":"synthetic","accountIds":["a","b"]
    }))
    .unwrap()];
    let scoped = native_contract::Caller {
        id: "scoped".into(),
        label: "fixture".into(),
    };
    assert_eq!(
        native_contract::Host::route(&host, &scoped, backup.clone())
            .await
            .unwrap()
            .account_id,
        "b"
    );
    host.collection_override
        .as_mut()
        .unwrap()
        .account_model_rules =
        vec![serde_json::from_value(json!({"accountId":"b","excludedModels":["gpt-*"]})).unwrap()];
    assert_eq!(
        native_contract::Host::route(&host, &scoped, backup.clone())
            .await
            .err()
            .unwrap()
            .code,
        "capability_fallback_no_compatible_account"
    );
    host.collection_override
        .as_mut()
        .unwrap()
        .account_model_rules
        .clear();
    host.account_overrides
        .get_mut("b")
        .unwrap()
        .api_model_catalog = vec!["other".into()];
    assert_eq!(
        native_contract::Host::route(&host, &scoped, backup.clone())
            .await
            .err()
            .unwrap()
            .code,
        "capability_fallback_no_compatible_account"
    );
    host.account_overrides
        .get_mut("b")
        .unwrap()
        .api_model_catalog = vec!["gpt-fixture".into()];
    backup.preferred_account_id = Some("a".into());
    assert_eq!(
        native_contract::Host::route(&host, &scoped, backup)
            .await
            .err()
            .unwrap()
            .code,
        "account_binding_unavailable"
    );
}

#[tokio::test]
async fn real_native_backup_capability_uses_final_mapped_model_and_respects_cooldowns() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut host = host();
    host.collection_override
        .as_mut()
        .unwrap()
        .capability_fallback
        .account_ids = vec!["b".into()];
    host.collection_override
        .as_mut()
        .unwrap()
        .capability_fallback
        .overrides = vec![serde_json::from_value(json!({
        "accountId":"b","model":"Final-Model","capabilities":{"function":"supported"}
    }))
    .unwrap()];
    host.account_overrides
        .get_mut("b")
        .unwrap()
        .api_model_mappings = vec![crate::models::codex::CodexApiModelMapping {
        client_model: "gpt-fixture".into(),
        upstream_model: "Final-Model".into(),
    }];
    let mut backup = request();
    backup.capability_fallback_from = Some("a".into());
    let route = select(&host, backup.clone()).await.unwrap();
    assert_eq!(
        (route.account_id.as_str(), route.model.as_str()),
        ("b", "Final-Model")
    );
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "b",
        "Final-Model",
        503,
        None,
        false,
        Instant::now(),
    );
    // The retired backup list only orders candidates. A healthy account in the
    // existing authorized pool remains eligible when the preferred one cools down.
    let route = select(&host, backup.clone()).await.unwrap();
    assert_eq!(
        (route.account_id.as_str(), route.model.as_str()),
        ("c", "gpt-fixture")
    );
    native_upsert_cooldown(
        &mut host.cooldowns.lock().unwrap(),
        "c",
        "gpt-fixture",
        503,
        None,
        false,
        Instant::now(),
    );
    assert_eq!(
        select(&host, backup).await.err().unwrap().code,
        "capability_fallback_no_compatible_account"
    );
}

struct TestData {
    directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    previous_server: Option<std::ffi::OsString>,
}
impl TestData {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        std::env::set_var("CHATGPT_PLAN_TEST_SERVER", "http://127.0.0.1:9");
        Self {
            directory,
            previous,
            previous_server,
        }
    }
}
impl Drop for TestData {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", previous);
        } else {
            std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        }
        if let Some(previous) = &self.previous_server {
            std::env::set_var("CHATGPT_PLAN_TEST_SERVER", previous);
        } else {
            std::env::remove_var("CHATGPT_PLAN_TEST_SERVER");
        }
    }
}

#[tokio::test]
async fn real_native_plan_only_pool_accepts_responses_websocket_and_rejects_true_auxiliaries() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let data = TestData::new();
    let id = uuid::Uuid::new_v4().to_string();
    let projection = format!("chatgpt_plan_{id}");
    let now = chrono::Utc::now().timestamp();
    let encrypted = crate::modules::secure_account_storage::serialize_account_file("chatgpt_plan", &json!({
        "client_id":"synthetic-client","subject":"synthetic-subject","access_token":"synthetic-access",
        "refresh_token":"synthetic-refresh","id_token":"synthetic-id","expires_at":now+3600,
        "scopes":["offline_access","resource.invoke","chatgpt.tokens.use.direct"],"earliest_refresh_at":null
    })).unwrap();
    let directory = data.directory.path().join("chatgpt-plan");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join(format!("{id}.json")), serde_json::to_vec(&json!({
        "summary":{"id":id,"label":"Fixture","email":"fixture@example.invalid","subject":"synthetic-subject","client_id":"synthetic-client",
        "connected":true,"needs_reauth":false,"verified_at":now,"verified_model":"gpt-fixture","expires_at":now+3600,"created_at":now,"updated_at":now},
        "revision":"synthetic-revision","authorization_revision":"synthetic-authorization","encrypted_credentials":encrypted,"revocation_confirmed":false,"model_catalog":["gpt-fixture"]
    })).unwrap()).unwrap();
    let mut source = CodexAccount::new("canonical-plan-source".into(), "fixture@example.invalid".into(), crate::models::codex::CodexTokens {
        access_token: "original-source-access".into(), id_token: "original-source-id".into(), refresh_token: Some("original-source-refresh".into()),
    });
    source.user_id = Some("original-user".into()); source.account_id = Some("original-workspace".into());
    source.plan_type = Some("plus".into()); source.egress_proxy_disabled = true;
    codex_account::save_account(&source).unwrap();
    crate::modules::chatgpt_plan::bind_account(&id, &source.id).await.unwrap();
    let mut host = host();
    host.collection_override.as_mut().unwrap().account_ids = vec![source.id.clone()];
    host.collection_override.as_mut().unwrap().official_account_bindings.insert(source.id.clone(), id.clone());
    host.collection_override.as_mut().unwrap().capability_fallback = Default::default();
    host.account_overrides.clear();
    host.account_overrides.insert(source.id.clone(), source.clone());
    let mut websocket = request();
    websocket.auxiliary = Some("responses.websocket".into());
    let route = select(&host, websocket).await.unwrap();
    assert_eq!(route.account_id, source.id);
    assert_eq!(route.continuity_id(), projection);
    assert!(route.chatgpt_plan);
    assert_eq!(route.base_url, "https://api.openai.com/v1");
    assert_eq!(route.wire_api, native_contract::WireApi::Responses);
    assert!(route.websocket_endpoint.is_none());
    assert!(
        !native_contract::Host::responses_websocket_passthrough(&host, &caller(), &route)
            .await
            .unwrap()
    );
    for operation in ["realtime", "search", "audio.transcriptions"] {
        let mut unsupported = request();
        unsupported.auxiliary = Some(operation.into());
        let error = select(&host, unsupported).await.err().unwrap();
        assert_eq!(error.status, 400);
    }
}
