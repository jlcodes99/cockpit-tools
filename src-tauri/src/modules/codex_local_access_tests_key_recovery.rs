#[test]
fn key_recovery_partial_listing_preserves_pool_and_bound_references() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-partial-listing");
    let mut available = test_account_with_plan("plus");
    available.tokens.refresh_token = Some("refresh".into());
    let mut ineligible = available.clone();
    ineligible.id = "ineligible".into();
    ineligible.tokens.access_token.clear();
    ineligible.tokens.refresh_token = None;
    let mut collection = test_local_access_collection(vec![
        available.id.clone(),
        "missing".into(),
        ineligible.id.clone(),
    ]);
    collection.bound_oauth_account_id = Some("missing".into());
    collection.image_generation_account_ids = vec!["missing".into()];
    sanitize_collection_with_accounts(&mut collection, &[available.clone(), ineligible]).unwrap();
    assert_eq!(collection.account_ids, vec![available.id, "missing".into()]);
    assert_eq!(
        collection.bound_oauth_account_id.as_deref(),
        Some("missing")
    );
    assert_eq!(collection.image_generation_account_ids, vec!["missing"]);
    assert!(api_key_inherits_account_pool(&collection.api_keys[0]));
    assert!(
        effective_api_key_account_ids(&collection, &collection.api_keys[0])
            .contains(&"missing".to_string())
    );
    assert!(remove_account_refs_from_collection(
        &mut collection,
        &HashSet::from(["missing".into()])
    ));
    assert!(!collection.account_ids.contains(&"missing".into()));
    assert!(collection.bound_oauth_account_id.is_none());
}

#[test]
fn key_recovery_partial_sources_keep_each_key_and_its_declared_scope() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    shared_state.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("key-recovery-partial-keys");
    let mut available = test_account_with_plan("plus");
    available.id = "available".into();
    available.tokens.refresh_token = Some("synthetic-refresh".into());
    let mut collection = test_local_access_collection(vec![available.id.clone(), "missing".into()]);
    let mut first = build_local_access_api_key(Some("available"));
    first.inherit_account_pool = Some(false);
    first.account_ids = vec![available.id.clone()];
    let mut second = build_local_access_api_key(Some("pending"));
    second.inherit_account_pool = Some(false);
    second.account_ids = vec!["missing".into()];
    let mut empty_scope = build_local_access_api_key(Some("empty"));
    empty_scope.inherit_account_pool = Some(false);
    empty_scope.account_ids.clear();
    collection.api_key = first.key.clone();
    collection.api_keys = vec![first.clone(), second.clone(), empty_scope.clone()];
    let dir = make_temp_dir("key-recovery-partial-keys");
    let overrides = HashMap::from([(available.id.clone(), available)]);
    let launch = super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        overrides,
        false,
        None,
    )
    .unwrap();
    assert!(launch.pending_public_accounts);
    let config: Value =
        serde_json::from_str(&fs::read_to_string(&launch.config_path).unwrap()).unwrap();
    assert_eq!(config["api-keys"], json!([first.key, second.key]));
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(&launch.manifest_path).unwrap()).unwrap();
    let keys = manifest["apiKeys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(
        keys.iter().find(|key| key["key"] == second.key).unwrap()["accountIds"],
        json!(["missing"])
    );
    assert!(!keys.iter().any(|key| key["key"] == empty_scope.key));
    assert_eq!(manifest["accounts"].as_array().unwrap().len(), 1);
    // The listener remains usable for the available key and enters background recovery.
    let mut runtime = super::GatewayRuntime::default();
    runtime.running = true;
    assert_eq!(
        super::finish_public_key_preparation(&mut runtime, true).unwrap_err(),
        super::UNRESOLVED_PUBLIC_API_KEYS
    );
    assert!(runtime.running);
    super::finish_public_key_preparation(&mut runtime, false).unwrap();
    assert!(runtime.last_error.is_none());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn key_recovery_partial_source_detection_ignores_disabled_and_deleted_accounts() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-pending-sources");
    let mut collection = test_local_access_collection(vec!["missing".into()]);
    let mut key = build_local_access_api_key(Some("pending"));
    collection.api_keys = vec![key.clone()];
    assert!(super::public_key_has_missing_sources(
        &collection,
        &HashMap::new()
    ));
    key.enabled = false;
    collection.api_keys = vec![key];
    assert!(!super::public_key_has_missing_sources(
        &collection,
        &HashMap::new()
    ));
    collection.api_keys[0].enabled = true;
    let dir = crate::modules::account::get_data_dir()
        .unwrap()
        .join("codex_account_tombstones");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("missing.json"),
        r#"{"deleted":true,"generation":1}"#,
    )
    .unwrap();
    assert!(!super::public_key_has_missing_sources(
        &collection,
        &HashMap::new()
    ));
}

#[test]
fn key_recovery_display_edits_do_not_revoke_service_but_proxy_credentials_do() {
    let mut account = test_account_with_plan("plus");
    account.egress_proxy_url = Some("http://user:first@127.0.0.1:12345".into());
    let original = super::recovery_account_signature(&account);
    account.account_note = Some("display note".into());
    account.tags = Some(vec!["display tag".into()]);
    assert_eq!(super::recovery_account_signature(&account), original);
    account.egress_proxy_url = Some("http://user:second@127.0.0.1:12345".into());
    assert_ne!(super::recovery_account_signature(&account), original);
}

#[tokio::test]
async fn key_recovery_deferred_import_result_respects_stop_generation() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, false);
    super::defer_gateway_key_preparation_error(
        Err(super::UNRESOLVED_PUBLIC_API_KEYS.into()),
        ticket.generation,
    )
    .await
    .unwrap();
    assert!(super::gateway_key_recovery_snapshot().0);
    super::advance_gateway_lifecycle_generation();
    assert!(super::defer_gateway_key_preparation_error(
        Err(super::UNRESOLVED_PUBLIC_API_KEYS.into()),
        ticket.generation,
    )
    .await
    .is_err());
    super::gateway_runtime().lock().await.collection = None;
    tokio::time::timeout(Duration::from_millis(200), async {
        while super::key_recovery_control()
            .lock()
            .unwrap()
            .active
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn key_recovery_stop_during_deferred_runtime_lock_cannot_start_new_ticket() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, false);
    let mut runtime = super::gateway_runtime().lock().await;
    let task = tokio::spawn(async move {
        super::defer_gateway_key_preparation_error(
            Err(super::UNRESOLVED_PUBLIC_API_KEYS.into()),
            ticket.generation,
        )
        .await
    });
    tokio::task::yield_now().await;
    super::advance_gateway_lifecycle_generation();
    runtime.running = false;
    drop(runtime);
    assert!(task.await.unwrap().is_err());
    assert!(super::key_recovery_control()
        .lock()
        .unwrap()
        .active
        .is_none());
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_stop_during_ticket_runtime_lock_prevents_preparation() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    let runtime = super::gateway_runtime().lock().await;
    let checked_ticket = ticket.clone();
    let task =
        tokio::spawn(async move { super::key_recovery_ticket_is_current(&checked_ticket).await });
    tokio::task::yield_now().await;
    super::advance_gateway_lifecycle_generation();
    drop(runtime);
    assert!(!task.await.unwrap());
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, false);
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_stop_during_saved_account_notification_cannot_restart_service() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-stop-during-save");
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, true);
    let mut account = test_account_with_plan("plus");
    account.id = "temporarily-missing".into();
    account.tokens.refresh_token = Some("synthetic-refresh".into());
    let mut runtime = super::gateway_runtime().lock().await;
    codex_account::save_account(&account).unwrap();
    tokio::task::yield_now().await;
    super::advance_gateway_lifecycle_generation();
    runtime.running = false;
    drop(runtime);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let control = super::key_recovery_control().lock().unwrap();
    assert!(control.active.is_none());
    assert_eq!(control.exhausted.as_ref(), Some(&ticket));
    drop(control);
    super::gateway_runtime().lock().await.collection = None;
}

#[test]
fn key_recovery_deleted_account_is_pruned_even_when_listing_is_empty() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-deleted-reference");
    let data_dir = crate::modules::account::get_data_dir().unwrap();
    let dir = data_dir.join("codex_account_tombstones");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("removed.json"),
        r#"{"deleted":true,"generation":1}"#,
    )
    .unwrap();
    let mut collection = test_local_access_collection(vec!["removed".into(), "unreadable".into()]);
    let mut key = build_local_access_api_key(Some("scoped"));
    key.inherit_account_pool = Some(false);
    key.account_ids = vec!["removed".into(), "unreadable".into()];
    collection.api_keys = vec![key];
    sanitize_collection_with_accounts(&mut collection, &[]).unwrap();
    assert_eq!(collection.account_ids, vec!["unreadable"]);
    assert_eq!(collection.api_keys[0].account_ids, vec!["unreadable"]);
}

#[test]
fn key_recovery_snapshot_reads_each_source_once_and_overrides_are_authoritative() {
    let account = test_account_with_plan("plus");
    let mut calls = HashMap::new();
    let overrides = HashMap::from([(account.id.clone(), account.clone())]);
    let snapshot = super::sidecar_account_snapshot(
        vec![
            account.id.clone(),
            "missing".into(),
            "missing".into(),
            " loaded ".into(),
            "loaded".into(),
        ],
        &overrides,
        |id| {
            *calls.entry(id.to_string()).or_insert(0) += 1;
            (id == "loaded").then(|| account.clone())
        },
    );
    assert!(!calls.contains_key(&account.id));
    assert_eq!(calls.get("missing"), Some(&1));
    assert_eq!(calls.get("loaded"), Some(&1));
    assert!(snapshot.contains_key(&account.id));
    assert!(!snapshot.contains_key("missing"));
    let mut collection = test_local_access_collection(vec!["missing".into()]);
    collection.api_keys = vec![build_local_access_api_key(Some("inherited"))];
    assert!(super::sidecar_client_api_keys_from_snapshot(&collection, &snapshot, false).is_empty());
    assert!(
        super::sidecar_api_key_scopes_from_snapshot(&collection, &snapshot, false)
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn key_recovery_old_service_signature_rejects_revocations_and_scope_changes() {
    let mut base = test_local_access_collection(vec!["account-a".into(), "account-b".into()]);
    base.api_keys = vec![build_local_access_api_key(Some("key"))];
    let signature = super::recovery_collection_signature(&base).unwrap();
    let mut same = base.clone();
    same.updated_at += 1;
    assert_eq!(
        super::recovery_collection_signature(&same).unwrap(),
        signature
    );
    let mut variants = Vec::new();
    let mut next = base.clone();
    next.api_keys[0].enabled = false;
    variants.push(next);
    let mut next = base.clone();
    next.account_ids.remove(0);
    variants.push(next);
    let mut next = base.clone();
    next.api_keys[0].allowed_models = vec!["gpt-5.5".into()];
    variants.push(next);
    let mut next = base.clone();
    next.api_keys[0].token_limit = Some(100);
    variants.push(next);
    let mut next = base.clone();
    next.api_keys[0].key = "rotated-key".into();
    variants.push(next);
    let mut next = base.clone();
    next.excluded_models = vec!["gpt-5.5".into()];
    variants.push(next);
    let mut next = base.clone();
    next.port += 1;
    variants.push(next);
    for next in variants {
        assert_ne!(
            super::recovery_collection_signature(&next).unwrap(),
            signature
        );
    }
}

#[test]
fn key_recovery_old_service_rejects_changed_or_missing_credentials() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-credentials");
    let account = CodexAccount::new_api_key(
        "recovery-key-account".into(),
        "test@example.com".into(),
        "sk-first".into(),
        CodexApiProviderMode::Custom,
        Some("https://example.com/v1".into()),
        None,
        None,
        Vec::new(),
    );
    codex_account::save_account(&account).unwrap();
    let signatures = HashMap::from([(
        account.id.clone(),
        super::recovery_account_signature(&account),
    )]);
    assert!(super::can_preserve_gateway_accounts(&signatures));
    let mut edited = account.clone();
    edited.openai_api_key = Some("sk-second".into());
    codex_account::save_account(&edited).unwrap();
    assert!(!super::can_preserve_gateway_accounts(&signatures));
    assert!(!super::can_preserve_gateway_accounts(&HashMap::from([(
        "missing".into(),
        "unused".into()
    )])));
}

#[test]
fn key_recovery_control_deduplicates_and_new_configuration_can_retry() {
    let first = super::KeyRecoveryTicket {
        generation: 1,
        collection_fingerprint: vec![1],
    };
    let second = super::KeyRecoveryTicket {
        generation: 1,
        collection_fingerprint: vec![2],
    };
    let mut control = super::KeyRecoveryControl::default();
    assert!(control.begin(first.clone()));
    control.attempt = 2;
    assert!(control.begin(first.clone()));
    assert_eq!(control.attempt, 2);
    control.finish(&first, true);
    assert!(!control.begin(first.clone()));
    assert!(control.begin(second.clone()));
    control.finish(&first, false);
    assert_eq!(control.active.as_ref(), Some(&second));
}

// The caller must hold the shared fixture lock until assertions and cleanup end.
// This helper resets process-global runtime/control state; it never acquires the
// non-reentrant mutex itself, including for callers that already own a data guard.
async fn key_recovery_test_ticket(
    _lock: &std::sync::MutexGuard<'static, ()>,
) -> super::KeyRecoveryTicket {
    let mut collection = test_local_access_collection(vec!["temporarily-missing".into()]);
    collection.api_keys = vec![build_local_access_api_key(Some("inherited"))];
    let ticket = super::KeyRecoveryTicket {
        generation: super::current_gateway_lifecycle_generation(),
        collection_fingerprint: super::collection_content_fingerprint(&collection).unwrap(),
    };
    {
        let mut runtime = super::gateway_runtime().lock().await;
        runtime.collection = Some(collection);
        runtime.applied_account_signatures.clear();
        runtime.last_error = Some(super::UNRESOLVED_PUBLIC_API_KEYS.into());
    }
    let mut control = super::key_recovery_control().lock().unwrap();
    *control = super::KeyRecoveryControl::default();
    assert!(control.begin(ticket.clone()));
    ticket
}

#[tokio::test]
async fn key_recovery_retries_until_success_and_clears_error() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    let mut attempts = 0;
    super::run_gateway_key_recovery_with(ticket, &[Duration::ZERO; 5], || {
        attempts += 1;
        let result = if attempts < 3 {
            Err(super::UNRESOLVED_PUBLIC_API_KEYS.into())
        } else {
            Ok(())
        };
        async move { result }
    })
    .await;
    assert_eq!(attempts, 3);
    assert_eq!(super::gateway_key_recovery_snapshot(), (false, 0));
    assert!(super::gateway_runtime().lock().await.last_error.is_none());
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_exhaustion_does_not_restart_same_retry_budget() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    let mut attempts = 0;
    super::run_gateway_key_recovery_with(ticket.clone(), &[Duration::ZERO; 5], || {
        attempts += 1;
        async { Err(super::UNRESOLVED_PUBLIC_API_KEYS.into()) }
    })
    .await;
    assert_eq!(attempts, 5);
    assert!(!super::key_recovery_control().lock().unwrap().begin(ticket));
    assert!(super::gateway_runtime().lock().await.last_error.is_some());
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_stop_cancels_wait_without_preparing_again() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    let task = tokio::spawn(async move {
        super::run_gateway_key_recovery_with(ticket, &[Duration::from_secs(30)], || async {
            panic!("stop must cancel pending preparation");
            #[allow(unreachable_code)]
            Ok(())
        })
        .await;
    });
    tokio::task::yield_now().await;
    super::advance_gateway_lifecycle_generation();
    tokio::time::timeout(Duration::from_millis(200), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(super::gateway_key_recovery_snapshot(), (false, 0));
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_new_scope_cancels_old_work() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::gateway_runtime()
        .lock()
        .await
        .collection
        .as_mut()
        .unwrap()
        .account_ids
        .clear();
    super::run_gateway_key_recovery_with(ticket, &[Duration::ZERO], || async {
        panic!("changed scope must cancel stale recovery");
        #[allow(unreachable_code)]
        Ok(())
    })
    .await;
    assert_eq!(super::gateway_key_recovery_snapshot(), (false, 0));
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_lock_timeout_exhausts_the_retry_budget() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let ticket = key_recovery_test_ticket(&_lock).await;
    let guard = super::gateway_lifecycle_lock().lock().await;
    super::run_gateway_key_recovery_with(ticket.clone(), &[Duration::ZERO], || async {
        panic!("busy lifecycle must not prepare concurrently");
        #[allow(unreachable_code)]
        Ok(())
    })
    .await;
    drop(guard);
    assert!(!super::key_recovery_control().lock().unwrap().begin(ticket));
    assert_eq!(super::gateway_key_recovery_snapshot(), (false, 0));
    super::gateway_runtime().lock().await.collection = None;
}

#[test]
fn key_recovery_reappearing_account_restores_original_key_and_scope() {
    let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
    shared_state.prepare_disabled();
    let _env = LocalAccessTestDataGuard::new("key-recovery-account-returns");
    let account = CodexAccount::new_api_key(
        "returning-account".into(),
        "test@example.com".into(),
        "sk-synthetic".into(),
        CodexApiProviderMode::Custom,
        Some("https://example.com/v1".into()),
        None,
        None,
        Vec::new(),
    );
    let mut collection = test_local_access_collection(vec![account.id.clone()]);
    collection.api_keys = vec![build_local_access_api_key(Some("keep-this-key"))];
    let original_key = collection.api_keys[0].key.clone();
    sanitize_collection_with_accounts(&mut collection, &[]).unwrap();
    assert_eq!(collection.account_ids, vec![account.id.clone()]);
    let dir = make_temp_dir("key-recovery-account-returns");
    fs::create_dir_all(&dir).unwrap();
    let config_path = super::sidecar_config_path(&dir);
    fs::write(&config_path, "last-good-config").unwrap();
    assert_eq!(
        super::prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            HashMap::new(),
            false,
            None,
        )
        .unwrap_err(),
        super::UNRESOLVED_PUBLIC_API_KEYS
    );
    assert_eq!(
        fs::read_to_string(&config_path).unwrap(),
        "last-good-config"
    );
    codex_account::save_account(&account).unwrap();
    super::prepare_sidecar_launch_config_in_dir_sync(
        &collection,
        dir.clone(),
        HashMap::new(),
        None,
        HashMap::new(),
        false,
        None,
    )
    .unwrap();
    let config: Value = serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(config["api-keys"], json!([original_key.clone()]));
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(super::sidecar_manifest_path(&dir)).unwrap())
            .unwrap();
    assert_eq!(manifest["apiKeys"][0]["key"], original_key);
    assert_eq!(manifest["apiKeys"][0]["accountIds"], json!([account.id]));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn key_recovery_restored_source_must_match_enabled_key_scope() {
    let mut account = test_account_with_plan("plus");
    account.tokens.refresh_token = Some("synthetic-refresh".into());
    let mut collection = test_local_access_collection(vec![account.id.clone()]);
    let mut key = build_local_access_api_key(Some("scoped"));
    key.inherit_account_pool = Some(false);
    key.account_ids = vec!["another-account".into()];
    collection.api_keys = vec![key];
    assert!(!super::account_can_restore_public_key(
        &collection,
        &account
    ));
    collection.api_keys[0].inherit_account_pool = Some(true);
    assert!(super::account_can_restore_public_key(&collection, &account));
    collection.api_keys[0].enabled = false;
    assert!(!super::account_can_restore_public_key(
        &collection,
        &account
    ));
    collection.api_keys[0].enabled = true;
    let mut rejected = account;
    rejected.requires_reauth = true;
    assert!(!super::account_can_restore_public_key(
        &collection,
        &rejected
    ));
}

#[tokio::test]
async fn key_recovery_saved_usable_account_resumes_after_exhaustion() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-saved-account");
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, true);
    assert!(!super::key_recovery_control().lock().unwrap().begin(ticket));
    let mut account = test_account_with_plan("plus");
    account.id = "temporarily-missing".into();
    account.tokens.refresh_token = Some("synthetic-refresh".into());
    codex_account::save_account(&account).unwrap();
    tokio::time::timeout(Duration::from_millis(200), async {
        while !super::gateway_key_recovery_snapshot().0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("saved usable source should restart recovery without a manual click");
    // Cancel before any filesystem preparation; no client or sidecar is launched.
    super::advance_gateway_lifecycle_generation();
    tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            if super::key_recovery_control()
                .lock()
                .unwrap()
                .active
                .is_none()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_source_saved_during_last_attempt_is_not_lost() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-last-attempt-save");
    let ticket = key_recovery_test_ticket(&_lock).await;
    let mut account = test_account_with_plan("plus");
    account.id = "temporarily-missing".into();
    account.tokens.refresh_token = Some("synthetic-refresh".into());
    super::run_gateway_key_recovery_with(ticket, &[Duration::ZERO], || {
        let account = account.clone();
        async move {
            // The preparation snapshot was taken before this durable save.
            codex_account::save_account(&account).unwrap();
            tokio::time::timeout(Duration::from_millis(200), async {
                while !super::key_recovery_control().lock().unwrap().source_updated {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            Err(super::UNRESOLVED_PUBLIC_API_KEYS.into())
        }
    })
    .await;
    assert!(super::gateway_key_recovery_snapshot().0);
    super::advance_gateway_lifecycle_generation();
    tokio::time::timeout(Duration::from_millis(200), async {
        while super::key_recovery_control()
            .lock()
            .unwrap()
            .active
            .is_some()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    super::gateway_runtime().lock().await.collection = None;
}

#[tokio::test]
async fn key_recovery_loaded_account_maintenance_does_not_repeat_missing_source_retries() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("key-recovery-loaded-account-save");
    let ticket = key_recovery_test_ticket(&_lock).await;
    super::key_recovery_control()
        .lock()
        .unwrap()
        .finish(&ticket, true);
    let mut account = test_account_with_plan("plus");
    account.id = "temporarily-missing".into();
    account.tokens.refresh_token = Some("synthetic-refresh".into());
    super::gateway_runtime()
        .lock()
        .await
        .applied_account_signatures
        .insert(
            account.id.clone(),
            super::recovery_account_signature(&account),
        );
    codex_account::save_account(&account).unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!super::gateway_key_recovery_snapshot().0);
    assert!(super::key_recovery_control()
        .lock()
        .unwrap()
        .exhausted
        .is_some());
    let mut runtime = super::gateway_runtime().lock().await;
    runtime.collection = None;
    runtime.applied_account_signatures.clear();
}

#[test]
fn key_recovery_account_signature_is_stable_across_map_reload_order() {
    let mut account = test_account_with_plan("plus");
    let entries: Vec<_> = (0..16)
        .map(|index| (format!("model-{index}"), 1000 + index))
        .collect();
    account.api_model_context_windows = entries.clone().into_iter().collect();
    let signature = super::recovery_account_signature(&account);
    for _ in 0..30 {
        account.api_model_context_windows = entries.iter().rev().cloned().collect();
        assert_eq!(super::recovery_account_signature(&account), signature);
    }
    account
        .api_model_context_windows
        .insert("model-0".into(), 2000);
    assert_ne!(super::recovery_account_signature(&account), signature);
}
