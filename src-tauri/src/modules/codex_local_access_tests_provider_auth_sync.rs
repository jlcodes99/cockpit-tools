#[test]
fn provider_auth_sync_coalesces_updates_without_losing_inflight_changes() {
    let queue = super::ProviderAuthSyncQueue::default();
    assert!(queue.request("account"));
    assert!(!queue.request("account"));
    queue.begin("account");
    assert!(!queue.request("account"));
    assert!(!queue.finish("account"));
    queue.begin("account");
    assert!(queue.finish("account"));
    assert!(queue.request("account"));
}

#[tokio::test]
async fn provider_auth_sync_prepares_cold_proxy_state_after_unbind() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let guard = LocalAccessTestDataGuard::new("provider-auth-cold-proxy");
    crate::modules::codex_unified_proxy::reset_cache();
    let mut account = test_account_with_plan("plus");
    account.tokens.access_token = "newest-access-token".into();
    account.egress_proxy_url = None;
    codex_account::save_account(&account).expect("save unbound account");
    let collection = test_local_access_collection(vec![account.id.clone()]);
    let sidecar_dir = guard.data_dir.join("active-instance");
    let auths_dir = super::sidecar_auths_dir(&sidecar_dir);
    fs::create_dir_all(&auths_dir).unwrap();
    let auth_path = auths_dir.join(super::sidecar_auth_file_name(&account.id));
    let previous = r#"{"access_token":"obsolete","proxy_url":"http://127.0.0.1:9"}"#;
    fs::write(&auth_path, previous).unwrap();
    let other_path = auths_dir.join("unrelated.json");
    fs::write(&other_path, "keep unrelated credential").unwrap();

    // Persisted files alone must never activate a stopped instance or even load
    // its proxy state. Only an existing runtime entry is eligible for writing.
    super::sync_provider_gateway_auth_files_for_account_once(&account.id)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&auth_path).unwrap(), previous);
    assert!(crate::modules::codex_unified_proxy::current().is_err());

    let runtime_key = format!("provider-auth-sync-{}", uuid::Uuid::new_v4());
    super::provider_gateway_runtime_store().lock().await.insert(
        runtime_key.clone(),
        super::ProviderGatewayRuntime {
            sidecar_dir: Some(sidecar_dir),
            collection: Some(collection),
            oauth_account_ids: vec![account.id.clone()],
            ..Default::default()
        },
    );
    let result = super::sync_provider_gateway_auth_files_for_account_once(&account.id).await;
    let runtime = super::provider_gateway_runtime_store()
        .lock()
        .await
        .remove(&runtime_key)
        .unwrap();
    let proxy_state = crate::modules::codex_unified_proxy::current();
    crate::modules::codex_unified_proxy::reset_cache();
    result.expect("prepare cold state and write existing instance auth");
    assert!(proxy_state.is_ok());
    assert!(
        runtime.sidecar_child.is_none(),
        "auth synchronization must not launch a sidecar"
    );
    let auth: Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
    assert_eq!(auth["access_token"], "newest-access-token");
    assert!(
        auth.get("proxy_url").is_none(),
        "unbinding removes the retired proxy port"
    );
    assert_eq!(
        fs::read_to_string(&other_path).unwrap(),
        "keep unrelated credential"
    );
}
