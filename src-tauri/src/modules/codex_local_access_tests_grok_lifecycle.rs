// Grok sources and the Codex proxy records have independent lifecycles. Verify that source
// removal revokes runtime credentials without deleting user-maintained provider settings.
use crate::modules::codex_account;

#[test]
fn grok_auth_sync_queue_coalesces_updates_without_losing_inflight_changes() {
    let queue = super::GrokAuthSyncQueue::default();
    assert!(queue.request("source"));
    assert!(!queue.request("source"));
    queue.begin_attempt("source");
    assert!(
        !queue.request("source"),
        "an in-flight update must reuse the worker"
    );
    assert!(
        !queue.finish_attempt("source", true),
        "a later update needs another pass"
    );
    queue.begin_attempt("source");
    assert!(
        !queue.finish_attempt("source", false),
        "failure must retain ownership"
    );
    assert!(!queue.request("source"));
    assert!(
        queue.request("other"),
        "different accounts have independent workers"
    );
    queue.begin_attempt("source");
    assert!(queue.finish_attempt("source", true));
    assert!(
        queue.request("source"),
        "a completed worker must release its slot"
    );
}

#[tokio::test]
async fn grok_auth_sync_retries_lock_contention_and_reads_latest_credentials() {
    grok_auth_sync_lock_contention_scenario(false).await;
}

#[tokio::test]
async fn grok_auth_sync_retry_revokes_a_source_deleted_during_backoff() {
    grok_auth_sync_lock_contention_scenario(true).await;
}

async fn grok_auth_sync_lock_contention_scenario(delete_source: bool) {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let guard = LocalAccessTestDataGuard::new("grok-sync-retry");
    let source = crate::modules::grok_account::import_from_json(
        &json!({"id":"retry-source", "email":"retry@example.test", "auth_mode":"oauth",
            "access_token":"token-old", "created_at":1, "last_used":1})
        .to_string(),
    )
    .unwrap()
    .remove(0);
    let proxy = grok_lifecycle_proxy(&source.id);
    codex_account::save_account(&proxy).unwrap();
    let collection = test_local_access_collection(vec![proxy.id.clone()]);
    super::save_collection_to_disk(&collection).unwrap();
    let auths = super::sidecar_auths_dir(&super::local_access_sidecar_dir().unwrap());
    fs::create_dir_all(&auths).unwrap();
    let auth_path = auths.join(codex_account::grok_sidecar_auth_file_name(&proxy.id));
    assert!(super::prepare_grok_sidecar_auth_file(&proxy, &auth_path, None).unwrap());
    let auth_backup = auth_path.with_file_name(format!(
        "{}.bak",
        auth_path.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&auth_backup, "stale credential backup").unwrap();
    assert!(super::prepare_grok_sidecar_auth_file(&proxy, &auth_path, None).unwrap());
    assert!(
        !auth_backup.exists(),
        "sidecar auth backups must be removed"
    );

    let lifecycle = super::provider_gateway_lifecycle_lock().lock().await;
    let wait = std::time::Duration::from_millis(5);
    assert!(
        super::sync_grok_upstream_auth_files_once(source.id.clone(), wait)
            .await
            .is_err()
    );
    let queue = super::GrokAuthSyncQueue::default();
    assert!(queue.request(&source.id));
    let mut worker = Box::pin(super::run_grok_auth_sync_worker(
        &queue, &source.id, wait, wait,
    ));
    // Borrow the future so this test timeout does not cancel the worker or its pending retry.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(35), worker.as_mut())
            .await
            .is_err()
    );
    assert!(
        !queue.request(&source.id),
        "lock timeouts must not drop the queued update"
    );
    assert!(fs::read_to_string(&auth_path)
        .unwrap()
        .contains("token-old"));

    if delete_source {
        crate::modules::grok_account::remove_account(&source.id).unwrap();
    } else {
        let mut current = crate::modules::grok_account::load_account(&source.id).unwrap();
        current.access_token = "token-new".to_string();
        current.expires_at = Some(chrono::Utc::now().timestamp() + 3600);
        crate::modules::grok_account::import_from_json(&serde_json::to_string(&current).unwrap())
            .unwrap();
    }
    drop(lifecycle);
    tokio::time::timeout(std::time::Duration::from_secs(2), worker.as_mut())
        .await
        .expect("retry should complete after the lifecycle lock is released");
    if delete_source {
        assert!(
            !auth_path.exists(),
            "retry must revoke, never restore, a deleted source"
        );
    } else {
        let auth: Value = serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(auth["access_token"], "token-new");
    }
    assert!(queue.pending.lock().unwrap().is_empty());
    assert!(guard.data_dir.exists());
}

fn grok_lifecycle_proxy(source_id: &str) -> CodexAccount {
    let mut proxy = automatic_routing_account("grok-proxy", "responses", &["grok-test-only"]);
    proxy.upstream_grok_account_id = Some(source_id.to_string());
    proxy.openai_api_key = None;
    proxy
}

#[test]
fn grok_missing_source_is_isolated_and_stale_auth_is_removed() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let guard = LocalAccessTestDataGuard::new("grok-missing-source");
    let dir = guard.data_dir.join("sidecar");
    let broken = grok_lifecycle_proxy("missing-source");
    let healthy = automatic_routing_account("healthy", "responses", &["healthy-model"]);
    codex_account::save_account(&broken).expect("persist orphan provider settings");
    codex_account::save_account(&healthy).expect("persist healthy account");
    let mut collection = test_local_access_collection(vec![broken.id.clone(), healthy.id.clone()]);
    let auths = super::sidecar_auths_dir(&dir);
    fs::create_dir_all(&auths).unwrap();
    let revoked_path = auths.join(codex_account::grok_sidecar_auth_file_name(&broken.id));
    fs::write(&revoked_path, r#"{"type":"xai","access_token":"revoked"}"#).unwrap();
    let overrides = HashMap::from([
        (broken.id.clone(), broken.clone()),
        (healthy.id.clone(), healthy.clone()),
    ]);

    // Cover the main pool, an API key's independent scope, and an explicit native route. Keep
    // the same stale installation to verify subsequent preparations remain safe and idempotent.
    for scenario in ["main", "scoped", "explicit"] {
        if scenario != "main" {
            collection.account_ids = vec![healthy.id.clone()];
            let mut key = build_local_access_api_key(Some(scenario));
            key.inherit_account_pool = Some(false);
            key.account_ids = vec![healthy.id.clone()];
            if scenario == "scoped" {
                key.account_ids.push(broken.id.clone());
            } else {
                key.model_routing = Some(CodexLocalAccessModelRouting {
                    default_route: "oauth".to_string(),
                    failure_policy: "strict".to_string(),
                    routes: vec![CodexLocalAccessModelRoute {
                        id: "grok-route".to_string(),
                        namespace: "grok".to_string(),
                        provider_account_id: broken.id.clone(),
                        native_provider: Some("xai".to_string()),
                        provider_gateway: CodexLocalAccessProviderGateway {
                            base_url: "https://cli-chat-proxy.grok.com/v1".to_string(),
                            api_key: String::new(),
                            upstream_model: "grok-test-only".to_string(),
                            upstream_models: vec!["grok-test-only".to_string()],
                            wire_api: Some("responses".to_string()),
                            supports_vision: true,
                            model_capabilities: HashMap::new(),
                            vision_routing_model: None,
                        },
                    }],
                });
            }
            collection.api_keys = vec![key];
        }
        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            overrides.clone(),
            true,
            None,
        )
        .expect("a broken Grok member must not prevent a healthy pool from starting");
        assert!(!revoked_path.exists());
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(super::sidecar_manifest_path(&dir)).unwrap())
                .unwrap();
        let manifest_accounts = manifest["accounts"].as_array().unwrap();
        assert!(manifest_accounts
            .iter()
            .any(|account| account["id"] == healthy.id));
        assert!(!manifest_accounts
            .iter()
            .any(|account| account["id"] == broken.id));
        assert!(!manifest["modelIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model == "grok-test-only"));
        assert!(!manifest["apiKeys"].to_string().contains("grok-test-only"));
    }
    assert!(
        codex_account::load_account(&broken.id).is_some(),
        "keep recoverable provider settings"
    );
}

#[test]
fn grok_revocation_reports_auth_cleanup_failures() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let guard = LocalAccessTestDataGuard::new("grok-revoke-failure");
    let auth_path = guard.data_dir.join("xai-blocked.json");
    // A directory where the auth file should be is a portable, deterministic unlink failure.
    fs::create_dir(&auth_path).unwrap();
    let error = super::prepare_grok_sidecar_auth_file(
        &grok_lifecycle_proxy("missing-source"),
        &auth_path,
        None,
    )
    .expect_err("cleanup failures must remain visible and retryable");
    assert!(error.contains("清理失效 Grok sidecar 凭据失败"));
    assert!(auth_path.is_dir());
}

#[tokio::test]
async fn grok_delete_revokes_global_and_instance_auth_before_returning() {
    let _env = crate::modules::test_support::env_lock().lock().unwrap();
    let guard = LocalAccessTestDataGuard::new("grok-delete-runtime");
    let imported = crate::modules::grok_account::import_from_json(
        &json!({
            "id": "source-to-delete", "email": "grok-delete@example.com", "auth_mode": "oauth",
            "access_token": "test-source-token", "created_at": 1, "last_used": 1,
        })
        .to_string(),
    )
    .expect("import source fixture");
    let source_id = imported[0].id.clone();
    let proxy = grok_lifecycle_proxy(&source_id);
    codex_account::save_account(&proxy).expect("persist linked proxy");
    let collection = test_local_access_collection(vec![proxy.id.clone()]);
    super::save_collection_to_disk(&collection).unwrap();
    let global_dir = super::local_access_sidecar_dir().unwrap();
    let instance_dir = guard.data_dir.join("instance-sidecar");
    let mut paths = Vec::new();
    for dir in [&global_dir, &instance_dir] {
        let auths = super::sidecar_auths_dir(dir);
        fs::create_dir_all(&auths).unwrap();
        let path = auths.join(codex_account::grok_sidecar_auth_file_name(&proxy.id));
        assert!(super::prepare_grok_sidecar_auth_file(&proxy, &path, None).unwrap());
        fs::write(auths.join("codex-unrelated.json"), "keep").unwrap();
        paths.push(path);
    }
    let runtime_key = format!("test-grok-delete-{}", uuid::Uuid::new_v4());
    super::provider_gateway_runtime_store().lock().await.insert(
        runtime_key.clone(),
        super::ProviderGatewayRuntime {
            collection: Some(collection),
            sidecar_dir: Some(instance_dir),
            ..Default::default()
        },
    );

    let result = crate::commands::grok::delete_grok_account(source_id.clone()).await;
    super::provider_gateway_runtime_store()
        .lock()
        .await
        .remove(&runtime_key);
    result.expect("delete must finish runtime revocation");
    assert!(crate::modules::grok_account::load_account(&source_id).is_none());
    assert!(codex_account::load_account(&proxy.id).is_some());
    for path in paths {
        assert!(
            !path.exists(),
            "deleted source token must not remain in the sidecar"
        );
        assert!(path.parent().unwrap().join("codex-unrelated.json").exists());
        // A late token-write attempt observes deletion and cannot resurrect the old credential.
        assert!(!super::prepare_grok_sidecar_auth_file(&proxy, &path, None).unwrap());
        assert!(!path.exists());
    }
    crate::commands::grok::delete_grok_accounts(vec![source_id])
        .await
        .expect("retry is idempotent");
}
