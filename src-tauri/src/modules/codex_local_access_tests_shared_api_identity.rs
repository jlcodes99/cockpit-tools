#[test]
fn shared_api_installation_manifest_and_auth_projection_survive_engine_switch_and_refresh() {
    let proxy_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    proxy_state.prepare_disabled();
    let mut first = test_account_with_plan("plus");
    first.id = format!("shared-identity-{}", uuid::Uuid::new_v4());
    first.user_id = Some("fixture-principal".into());
    first.account_id = Some("fixture-workspace".into());
    first.egress_proxy_disabled = true;
    let mut second = first.clone();
    second.id = format!("shared-identity-{}", uuid::Uuid::new_v4());
    second.user_id = Some("other-principal".into());
    let collection = test_local_access_collection(vec![first.id.clone(), second.id.clone()]);
    let dir = tempfile::tempdir().unwrap();
    let prepare = |first: &CodexAccount| {
        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.path().to_path_buf(),
            HashMap::new(),
            None,
            HashMap::from([
                (first.id.clone(), first.clone()),
                (second.id.clone(), second.clone()),
            ]),
            false,
            None,
        )
        .unwrap()
    };
    let launch = prepare(&first);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(&launch.manifest_path).unwrap()).unwrap();
    let entries = manifest["accounts"].as_array().unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry["id"] == first.id)
        .unwrap();
    let native_profile = crate::modules::codex_api_identity::resolve(&first).unwrap();
    assert_eq!(
        entry["gatewayIdentity"],
        serde_json::to_value(&native_profile).unwrap()
    );
    assert_eq!(entry["chatgptUserId"], "fixture-principal");
    assert_eq!(entry["chatgptAccountId"], "fixture-workspace");
    assert_eq!(entry["upstreamProxyUrl"], "direct");
    let auth_path =
        super::sidecar_auths_dir(dir.path()).join(super::sidecar_auth_file_name(&first.id));
    let auth: Value = serde_json::from_slice(&std::fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["gatewayIdentity"], entry["gatewayIdentity"]);
    assert_ne!(
        entries[0]["gatewayIdentity"]["installationId"],
        entries[1]["gatewayIdentity"]["installationId"]
    );
    first.tokens.access_token = "new-fixture-access".into();
    first.token_generation += 1;
    let refreshed = prepare(&first);
    let new_manifest: Value =
        serde_json::from_slice(&std::fs::read(&refreshed.manifest_path).unwrap()).unwrap();
    let new_entry = new_manifest["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == first.id)
        .unwrap();
    assert_eq!(new_entry["gatewayIdentity"], entry["gatewayIdentity"]);
    assert_eq!(
        crate::modules::codex_api_identity::resolve(&first).unwrap(),
        native_profile
    );
    first.tokens.access_token = "write-through-fixture-access".into();
    assert!(
        super::sync_provider_gateway_runtime_auth_file(&first, &collection, dir.path()).unwrap()
    );
    let written: Value = serde_json::from_slice(&std::fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(written["gatewayIdentity"], entry["gatewayIdentity"]);
    assert_eq!(written["access_token"], "write-through-fixture-access");
    first.account_id = Some("new-workspace".into());
    let replaced = prepare(&first);
    let changed: Value =
        serde_json::from_slice(&std::fs::read(&replaced.manifest_path).unwrap()).unwrap();
    let changed = changed["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == first.id)
        .unwrap();
    assert_ne!(changed["gatewayIdentity"], entry["gatewayIdentity"]);
    assert_ne!(replaced.fingerprint, refreshed.fingerprint);
}

#[test]
fn shared_api_installation_direct_provider_preserves_environment_bypass_and_explicit_overrides() {
    let proxy_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    proxy_state.prepare_disabled();
    let mut account = test_account_with_plan("plus");
    assert!(super::direct_provider_inherits_proxy_environment(
        &account,
        super::UpstreamProxySource::SystemEnv
    )
    .unwrap());
    assert!(!super::direct_provider_inherits_proxy_environment(
        &account,
        super::UpstreamProxySource::ApiService
    )
    .unwrap());
    account.egress_proxy_disabled = true;
    assert!(!super::direct_provider_inherits_proxy_environment(
        &account,
        super::UpstreamProxySource::SystemEnv
    )
    .unwrap());
    account.egress_proxy_disabled = false;
    account.egress_proxy_url = Some("http://127.0.0.1:7777".into());
    assert!(!super::direct_provider_inherits_proxy_environment(
        &account,
        super::UpstreamProxySource::SystemEnv
    )
    .unwrap());
}

#[test]
fn shared_api_installation_manifest_declares_recovered_sdk_endpoint() {
    let proxy_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _data = LocalAccessTestDataGuard::new("api-identity-recovered-endpoint");
    proxy_state.prepare_disabled();
    let mut account = CodexAccount::new_api_key(
        "api-recovered-endpoint".into(),
        "fixture@example.invalid".into(),
        "fixture-api-key".into(),
        CodexApiProviderMode::OpenaiBuiltin,
        None,
        None,
        None,
        vec![],
    );
    account.egress_proxy_disabled = true;
    let collection = test_local_access_collection(vec![account.id.clone()]);
    // The original account contains the gateway URL. The existing resolver
    // restores the official endpoint before generating the Go config.
    account.api_base_url = Some(format!("http://127.0.0.1:{}/v1", collection.port));
    let directory = tempfile::tempdir().unwrap();
    let launch = super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        directory.path().to_path_buf(),
        HashMap::new(),
        None,
        HashMap::from([(account.id.clone(), account.clone())]),
        false,
        None,
    )
    .unwrap();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(launch.manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["accounts"][0]["upstreamBaseUrl"],
        super::DEFAULT_OPENAI_RESPONSES_BASE_URL
    );
    let config: Value =
        serde_json::from_slice(&std::fs::read(launch.config_path).unwrap()).unwrap();
    assert_eq!(
        config["codex-api-key"][0]["base-url"],
        manifest["accounts"][0]["upstreamBaseUrl"],
        "manifest and actual SDK endpoint must agree"
    );
    assert_eq!(
        manifest["accounts"][0]["gatewayIdentity"],
        serde_json::to_value(crate::modules::codex_api_identity::resolve(&account).unwrap())
            .unwrap()
    );
}

#[test]
fn shared_api_installation_global_write_through_keeps_profile_and_preserves_auth_on_store_failure()
{
    let proxy_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    let _data = LocalAccessTestDataGuard::new("global-api-identity-write-through");
    proxy_state.prepare_disabled();
    let mut account = test_account_with_plan("plus");
    account.user_id = Some("fixture-principal".into());
    account.account_id = Some("fixture-workspace".into());
    account.egress_proxy_disabled = true;
    let collection = test_local_access_collection(vec![account.id.clone()]);
    super::save_collection_to_disk(&collection).unwrap();
    let directory = super::local_access_sidecar_dir().unwrap();
    let auths = super::sidecar_auths_dir(&directory);
    std::fs::create_dir_all(&auths).unwrap();
    let path = auths.join(super::sidecar_auth_file_name(&account.id));
    std::fs::write(&path, "{}").unwrap();
    let profile = crate::modules::codex_api_identity::resolve(&account).unwrap();
    for access in ["fixture-before", "fixture-after"] {
        account.tokens.access_token = access.into();
        super::sync_sidecar_auth_file_for_account_with_task_source(&account, false).unwrap();
        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            written["gatewayIdentity"],
            serde_json::to_value(&profile).unwrap()
        );
        assert_eq!(written["access_token"], access);
    }
    let original = std::fs::read(&path).unwrap();
    // Installation stores legitimately contain multiple accounts. Fault
    // injection must target this account, not assign one UNIQUE value to every
    // row (which fails before exercising the auth-preservation path).
    let mut other = account.clone();
    other.id = format!("unrelated-installation-{}", uuid::Uuid::new_v4());
    let other_profile = crate::modules::codex_api_identity::resolve(&other).unwrap();
    let store = crate::modules::account::get_data_dir()
        .unwrap()
        .join("codex_api_account_identity.sqlite");
    let affected = rusqlite::Connection::open(store)
        .unwrap()
        .execute(
            "UPDATE account_installation_v1 SET installation_id='invalid-installation' WHERE installation_id=?1",
            [&profile.installation_id],
        )
        .unwrap();
    assert_eq!(affected, 1);
    account.tokens.access_token = "must-not-replace-auth-on-failure".into();
    assert!(super::sync_sidecar_auth_file_for_account_with_task_source(&account, false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(crate::modules::codex_api_identity::resolve(&other).unwrap(), other_profile);
}
