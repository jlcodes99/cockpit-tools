#[test]
fn image_catalog_discovery_tombstones_restart_and_manual_readd_are_durable() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("image-model-catalog-durable");
    let mut collection = test_local_access_collection(Vec::new());
    collection.enabled = false;
    let variant = "gpt-image-2.5-flare".to_string();
    assert!(super::update_image_model_catalog(
        &mut collection,
        Some(vec![
            variant.clone(),
            variant.clone(),
            " custom-image-v3 ".into(),
            "invalid space".into()
        ]),
        None,
        None
    )
    .unwrap());
    assert_eq!(
        collection
            .image_generation_models
            .iter()
            .filter(|model| **model == variant)
            .count(),
        1
    );
    assert!(collection
        .image_generation_models
        .contains(&"custom-image-v3".into()));
    assert!(!collection
        .image_generation_models
        .contains(&"invalid space".into()));
    assert!(
        super::update_image_model_catalog(&mut collection, None, None, Some(variant.clone()))
            .unwrap()
    );
    super::save_collection_to_disk(&collection).unwrap();
    let mut restored = super::load_collection_from_disk().unwrap().unwrap();
    restored
        .image_generation_account_models
        .insert("legacy-source".into(), variant.clone());
    super::sanitize_image_model_catalog(&mut restored);
    assert!(!super::update_image_model_catalog(
        &mut restored,
        Some(vec![variant.clone()]),
        None,
        None
    )
    .unwrap());
    assert!(!restored.image_generation_models.contains(&variant));
    assert!(restored.image_generation_deleted_models.contains(&variant));
    assert!(
        super::update_image_model_catalog(&mut restored, None, Some(variant.clone()), None)
            .unwrap()
    );
    assert!(restored.image_generation_models.contains(&variant));
    assert!(!restored.image_generation_deleted_models.contains(&variant));
    let path = super::local_access_file_path().unwrap();
    super::persist_image_model_settings(
        &path,
        &restored,
        None,
        None,
        Some((None, Some(variant.clone()), None)),
    )
    .unwrap();
    let restarted = super::load_collection_from_disk().unwrap().unwrap();
    assert_eq!(
        restarted.image_generation_models,
        restored.image_generation_models
    );
    assert_eq!(
        restarted.image_generation_deleted_models,
        restored.image_generation_deleted_models
    );
}

#[test]
fn image_catalog_selected_delete_is_rejected_and_family_selection_preserves_tombstone() {
    let mut collection = test_local_access_collection(Vec::new());
    let before = serde_json::to_value(&collection).unwrap();
    assert_eq!(
        super::update_image_model_catalog(
            &mut collection,
            None,
            None,
            Some("gpt-image-2.5".into())
        )
        .unwrap_err(),
        "codex.localAccess.imageGenerationModel.deleteSelected"
    );
    assert_eq!(serde_json::to_value(&collection).unwrap(), before);
    assert!(super::update_image_model_catalog(
        &mut collection,
        None,
        None,
        Some("gpt-image-2".into())
    )
    .unwrap());
    assert!(super::apply_selected_image_model(&mut collection, "gpt-image-2").unwrap());
    assert_eq!(collection.image_generation_model, "gpt-image-2");
    assert!(!collection
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(collection
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    super::update_image_model_catalog(&mut collection, None, Some("gpt-image-2".into()), None)
        .unwrap();
    assert!(collection
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(!collection
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    for invalid in ["", "model with space", "model\0bad"] {
        assert!(super::apply_selected_image_model(&mut collection, invalid).is_err());
        assert!(super::update_image_model_catalog(
            &mut collection,
            None,
            Some(invalid.into()),
            None
        )
        .is_err());
    }
    assert!(super::apply_selected_image_model(&mut collection, &"a".repeat(201)).is_err());
    assert!(
        super::apply_selected_image_model(&mut collection, " provider/custom-image-v3 ").unwrap()
    );
    assert_eq!(
        collection.image_generation_model,
        "provider/custom-image-v3"
    );
}

#[test]
fn image_catalog_legacy_defaults_migrate_candidates_without_changing_selection_or_deleted_models() {
    let mut legacy = serde_json::to_value(test_local_access_collection(Vec::new())).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("imageGenerationModels");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("imageGenerationDeletedModels");
    let mut collection: super::CodexLocalAccessCollection = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        collection.image_generation_models,
        vec!["gpt-image-2.5", "gpt-image-2"]
    );
    collection.image_generation_account_models =
        HashMap::from([("legacy-source".into(), "gpt-image-2.5-vip".into())]);
    assert!(super::sanitize_image_model_catalog(&mut collection));
    assert_eq!(collection.image_generation_model, "gpt-image-2.5");
    assert!(collection
        .image_generation_models
        .contains(&"gpt-image-2.5-vip".into()));
    assert!(super::update_image_model_catalog(
        &mut collection,
        None,
        None,
        Some("gpt-image-2.5-vip".into())
    )
    .unwrap());
    assert!(!super::sanitize_image_model_catalog(&mut collection));
    let mut profile = test_local_access_collection(Vec::new());
    super::apply_provider_gateway_template_settings(&mut profile, &collection);
    assert_eq!(
        profile.image_generation_models,
        collection.image_generation_models
    );
    assert_eq!(
        profile.image_generation_deleted_models,
        collection.image_generation_deleted_models
    );
    assert_eq!(
        profile.image_generation_model,
        collection.image_generation_model
    );
}

#[test]
fn image_full_custom_model_excludes_oauth_and_keeps_legacy_declared_source_scope() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("image-model-legacy-scope");
    let account = test_image_api_source(&["gpt-image-2.5-flare"]);
    let mut collection = test_local_access_collection(Vec::new());
    collection.image_generation_account_ids = vec![account.id.clone()];
    collection.image_generation_account_models =
        HashMap::from([(account.id.clone(), "gpt-image-2.5-flare".into())]);
    super::sanitize_collection_with_accounts(&mut collection, std::slice::from_ref(&account))
        .unwrap();
    assert_eq!(
        collection.image_generation_account_ids,
        vec![account.id.clone()]
    );
    assert!(collection
        .image_generation_models
        .contains(&"gpt-image-2.5-flare".into()));
    assert_eq!(
        super::sidecar_account_manifest_value(&account, None, &collection)
            ["imageGenerationUpstream"]["model"],
        "gpt-image-2.5-flare"
    );
    let oauth = CodexAccount::new(
        "image-oauth".into(),
        "oauth@example.com".into(),
        CodexTokens {
            id_token: "id".into(),
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
        },
    );
    assert!(!super::is_image_generation_source_for_model(
        &oauth,
        "gpt-image-2.5-flare"
    ));
    assert!(!super::is_image_generation_source_for_model(
        &oauth,
        "custom-image-v3"
    ));
    assert!(super::is_image_generation_source_for_model(
        &oauth,
        "gpt-image-2.5"
    ));
    super::apply_selected_image_model(&mut collection, "gpt-image-2.5-flare").unwrap();
    let manifest = super::sidecar_account_manifest_value(&account, None, &collection);
    assert_eq!(
        manifest["imageGenerationUpstream"]["model"],
        "gpt-image-2.5-flare"
    );
}

#[test]
fn image_catalog_field_transactions_preserve_newer_disk_and_runtime_against_old_snapshots() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("image-catalog-concurrency");
    let mut original = test_local_access_collection(Vec::new());
    original.enabled = false;
    super::save_collection_to_disk(&original).unwrap();
    let path = super::local_access_file_path().unwrap();
    let newer = super::persist_image_model_settings(
        &path,
        &original,
        None,
        None,
        Some((
            Some(vec!["custom-image-v3".into()]),
            None,
            Some("gpt-image-2".into()),
        )),
    )
    .unwrap();
    assert!(
        newer.image_generation_model_catalog_revision
            > original.image_generation_model_catalog_revision
    );
    let selected = super::persist_image_model_settings(
        &path,
        &original,
        Some("custom-image-v3".into()),
        None,
        None,
    )
    .unwrap();
    let mut old_snapshot = original.clone();
    old_snapshot.debug_logs = false;
    super::save_collection_to_disk(&old_snapshot).unwrap();
    let persisted = super::load_collection_from_disk().unwrap().unwrap();
    assert_eq!(persisted.image_generation_model, "custom-image-v3");
    assert!(persisted
        .image_generation_models
        .contains(&"custom-image-v3".into()));
    assert!(!persisted
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(persisted
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    assert!(!persisted.debug_logs);
    // A delayed ordinary runtime publisher cannot roll the selected ID or tombstone back.
    let mut runtime = super::GatewayRuntime::default();
    runtime.loaded = true;
    runtime.collection = Some(selected);
    super::sync_runtime_collection(&mut runtime, old_snapshot.clone());
    let current = runtime.collection.as_ref().unwrap();
    assert_eq!(current.image_generation_model, "custom-image-v3");
    assert!(!current
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(current
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    assert!(!current.debug_logs);
    // Discovery starts from latest disk state instead of restoring fallback's old settings.
    let discovered = super::persist_image_model_settings(
        &path,
        &original,
        None,
        None,
        Some((
            Some(vec!["gpt-image-2".into(), "another-image".into()]),
            None,
            None,
        )),
    )
    .unwrap();
    assert!(!discovered.debug_logs);
    assert_eq!(discovered.image_generation_model, "custom-image-v3");
    assert!(!discovered
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(discovered
        .image_generation_models
        .contains(&"another-image".into()));
    super::publish_image_model_settings(&mut runtime, &discovered, false);
    assert!(!runtime.collection.as_ref().unwrap().debug_logs);
    assert!(runtime
        .collection
        .as_ref()
        .unwrap()
        .image_generation_models
        .contains(&"another-image".into()));
    // Selecting a family does not restore a deleted bare actual. Manual add does.
    let restored = super::persist_image_model_settings(
        &path,
        &original,
        Some("gpt-image-2".into()),
        None,
        None,
    )
    .unwrap();
    assert!(restored
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    assert_eq!(restored.image_generation_model, "gpt-image-2");
    let added = super::persist_image_model_settings(
        &path,
        &original,
        None,
        None,
        Some((None, Some("gpt-image-2".into()), None)),
    )
    .unwrap();
    assert!(!added
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
}

#[tokio::test]
async fn image_catalog_command_creates_disabled_local_settings_without_accounts_or_gateway() {
    use futures_util::FutureExt;

    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _internal_accounts = InternalApiAccountIdsGuard::new();
    let _env = LocalAccessTestDataGuard::new("image-catalog-no-service");
    let previous_runtime = {
        let mut runtime = super::gateway_runtime().lock().await;
        let previous = std::mem::take(&mut *runtime);
        runtime.loaded = true;
        previous
    };
    // Restore every runtime field even if an assertion fails. Re-raise the
    // original panic after cleanup so the test retains its full failure signal.
    let result = std::panic::AssertUnwindSafe(async {
        assert!(super::internal_api_account_ids().is_empty());
        let generation = super::current_gateway_lifecycle_generation();
        let state = super::update_local_access_image_generation_model_catalog(
            None,
            Some("custom-image-v3".into()),
            None,
        )
        .await
        .unwrap();
        assert!(!state.running);
        let collection = state.collection.unwrap();
        assert!(!collection.enabled);
        assert!(collection.account_ids.is_empty());
        assert!(collection
            .image_generation_models
            .contains(&"custom-image-v3".into()));
        assert_eq!(super::current_gateway_lifecycle_generation(), generation);
        assert!(super::load_collection_from_disk()
            .unwrap()
            .unwrap()
            .image_generation_models
            .contains(&"custom-image-v3".into()));
        // Selected-model persistence has the same independent, disabled initialization path.
        std::fs::remove_file(super::local_access_file_path().unwrap()).unwrap();
        super::gateway_runtime().lock().await.collection = None;
        let state = super::update_local_access_image_generation_model("other-image-v2".into())
            .await
            .unwrap();
        assert!(!state.running);
        assert!(!state.collection.as_ref().unwrap().enabled);
        assert_eq!(
            state.collection.as_ref().unwrap().image_generation_model,
            "other-image-v2"
        );
        assert_eq!(super::current_gateway_lifecycle_generation(), generation);
    })
    .catch_unwind()
    .await;
    *super::gateway_runtime().lock().await = previous_runtime;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[test]
fn image_catalog_concurrent_discovery_and_other_settings_do_not_overwrite_each_other() {
    let directory = make_temp_dir("image-model-concurrent-writers");
    let path = directory.join("collection.json");
    let mut original = test_local_access_collection(Vec::new());
    original.enabled = false;
    super::save_collection_to_path(&path, &original).unwrap();
    let catalog_path = path.clone();
    let fallback = original.clone();
    let discovery = std::thread::spawn(move || {
        for index in 0..20 {
            super::persist_image_model_settings(
                &catalog_path,
                &fallback,
                None,
                None,
                Some((
                    Some(vec![format!("test-image-{}", index)]),
                    None,
                    (index == 0).then(|| "gpt-image-2".to_string()),
                )),
            )
            .unwrap();
            std::thread::yield_now();
        }
    });
    let settings_path = path.clone();
    let settings = std::thread::spawn(move || {
        let mut stale = original;
        stale.debug_logs = false;
        for _ in 0..20 {
            super::save_collection_to_path(&settings_path, &stale).unwrap();
            std::thread::yield_now();
        }
    });
    discovery.join().unwrap();
    settings.join().unwrap();
    let collection: super::CodexLocalAccessCollection =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(!collection.debug_logs);
    assert!(!collection
        .image_generation_models
        .contains(&"gpt-image-2".into()));
    assert!(collection
        .image_generation_deleted_models
        .contains(&"gpt-image-2".into()));
    for index in 0..20 {
        assert!(collection
            .image_generation_models
            .contains(&format!("test-image-{}", index)));
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn image_source_variant_registration_is_atomic_durable_and_never_restores_deleted_actual() {
    let directory = make_temp_dir("image-source-registry-transaction");
    let path = directory.join("collection.json");
    let mut original = test_local_access_collection(Vec::new());
    original.enabled = false;
    super::save_collection_to_path(&path, &original).unwrap();
    let actual = "gpt-image-2.5-vip".to_string();
    let registered = super::persist_image_model_settings_with_sources(
        &path,
        &original,
        None,
        None,
        None,
        Some(vec![actual.clone(), actual.clone()]),
    )
    .unwrap();
    assert_eq!(
        registered
            .image_generation_models
            .iter()
            .filter(|id| *id == &actual)
            .count(),
        1
    );
    let mut stale = original.clone();
    stale.debug_logs = false;
    super::save_collection_to_path(&path, &stale).unwrap();
    let restarted: super::CodexLocalAccessCollection =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(restarted.image_generation_models.contains(&actual));
    assert!(!restarted.debug_logs);
    let deleted = super::persist_image_model_settings(
        &path,
        &original,
        None,
        None,
        Some((None, None, Some(actual.clone()))),
    )
    .unwrap();
    assert_eq!(
        super::persist_image_model_settings_with_sources(
            &path,
            &original,
            None,
            None,
            None,
            Some(vec![actual.clone()]),
        )
        .unwrap_err(),
        "codex.localAccess.imageAccounts.modelRequired"
    );
    let restarted: super::CodexLocalAccessCollection =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        restarted.image_generation_deleted_models,
        deleted.image_generation_deleted_models
    );
    assert!(!restarted.image_generation_models.contains(&actual));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn image_deleted_actual_blocks_api_oauth_and_profile_without_fallback_or_erasing_scope() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("image-deleted-source-profile");
    let actual = "gpt-image-2.5-vip".to_string();
    let mut account = test_image_api_source(&["chat-model", "gpt-image-2.5", &actual]);
    account.api_image_generation_account_ids = vec![account.id.clone()];
    account.api_image_generation_account_models =
        HashMap::from([(account.id.clone(), actual.clone())]);
    let mut template = test_local_access_collection(Vec::new());
    template.enabled = false;
    template.image_generation_deleted_models = vec![actual.clone(), "gpt-image-2.5".into()];
    super::save_collection_to_disk(&template).unwrap();
    let profile = make_temp_dir("image-deleted-source-profile-dir");
    let (collection, _, _) =
        super::build_provider_gateway_collection_for_profile(&profile, &account).unwrap();
    assert_eq!(collection.image_generation_model, "gpt-image-2.5");
    assert_eq!(
        collection.image_generation_account_ids,
        vec![account.id.clone()]
    );
    assert_eq!(
        collection.image_generation_account_models.get(&account.id),
        Some(&actual)
    );
    assert!(super::image_generation_accounts_for_account(&account, &collection).is_empty());
    assert!(
        super::resolve_image_generation_source_model(&account, &collection, Some(&actual))
            .is_none()
    );
    assert!(
        super::sidecar_account_manifest_value(&account, None, &collection)
            .get("imageGenerationUpstream")
            .is_none()
    );
    let oauth = CodexAccount::new(
        "deleted-oauth".into(),
        "oauth@example.com".into(),
        CodexTokens {
            id_token: "id".into(),
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
        },
    );
    assert!(super::resolve_image_generation_source_model(&oauth, &collection, None).is_none());
    assert_eq!(
        super::sidecar_account_manifest_value(&oauth, None, &collection)["imageGenerationPolicy"],
        "disabled"
    );
    fs::remove_dir_all(profile).unwrap();
}

#[test]
fn image_catalog_in_use_row_actual_delete_is_rejected_without_changing_selection() {
    let mut collection = test_local_access_collection(Vec::new());
    collection.image_generation_account_ids = vec!["source".into()];
    collection
        .image_generation_account_models
        .insert("source".into(), "gpt-image-2.5-vip".into());
    let before = serde_json::to_value(&collection).unwrap();
    assert_eq!(
        super::update_image_model_catalog(
            &mut collection,
            None,
            None,
            Some("gpt-image-2.5-vip".into())
        )
        .unwrap_err(),
        "codex.localAccess.imageGenerationModel.deleteSelected"
    );
    assert_eq!(serde_json::to_value(&collection).unwrap(), before);
}

#[tokio::test]
async fn image_catalog_delete_rejects_pending_profile_without_blocking_add_or_discovery() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("image-delete-pending-profile");
    let actual = "gpt-image-2.5-vip".to_string();
    let mut account = test_image_api_source(&["chat-model", &actual]);
    account.api_image_generation_account_ids = vec![account.id.clone()];
    account.api_image_generation_account_models =
        HashMap::from([(account.id.clone(), actual.clone())]);
    let mut original = test_local_access_collection(Vec::new());
    original.enabled = false;
    original.image_generation_models.push(actual.clone());
    super::save_collection_to_disk(&original).unwrap();
    let previous = {
        let mut runtime = super::gateway_runtime().lock().await;
        let previous = (runtime.collection.take(), runtime.loaded, runtime.running);
        runtime.collection = Some(original.clone());
        runtime.loaded = true;
        runtime.running = false;
        previous
    };
    let profile = make_temp_dir("image-delete-pending-profile-dir");
    let key = format!("image-delete-pending-{}", uuid::Uuid::new_v4());
    // Match the real launch: hold its lifecycle lock, read the template and
    // prepare the private source, then pause before publication to the store.
    let lifecycle = super::provider_gateway_lifecycle_lock().lock().await;
    let (prepared, _, _) =
        super::build_provider_gateway_collection_for_profile(&profile, &account).unwrap();
    assert_eq!(
        super::sidecar_account_manifest_value(&account, None, &prepared)["imageGenerationUpstream"]
            ["model"],
        actual
    );
    assert!(!super::provider_gateway_runtime_store()
        .lock()
        .await
        .contains_key(&key));
    let path = super::local_access_file_path().unwrap();
    let before = fs::read(&path).unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::update_local_access_image_generation_model_catalog(None, None, Some(actual.clone())),
    )
    .await
    .expect("pending gateway removal must fail promptly")
    .unwrap_err();
    assert_eq!(error, "codex.modelConfig.errors.busy");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(super::image_configuration_update_lock().try_lock().is_ok());
    // Ordinary discovery/manual add must still finish while the gateway waits.
    let updated = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        super::update_local_access_image_generation_model_catalog(
            Some(vec!["gpt-image-2.5-flare".into()]),
            Some("custom-image-v3".into()),
            None,
        ),
    )
    .await
    .expect("discovery and add must not wait for a provider launch")
    .unwrap();
    let collection = updated.collection.unwrap();
    assert!(collection
        .image_generation_models
        .contains(&"gpt-image-2.5-flare".into()));
    assert!(collection
        .image_generation_models
        .contains(&"custom-image-v3".into()));
    assert!(!collection.image_generation_deleted_models.contains(&actual));
    // Completing publication under the same lock makes the existing in-use
    // guard authoritative before a subsequent delete can acquire that lock.
    super::provider_gateway_runtime_store().lock().await.insert(
        key.clone(),
        super::ProviderGatewayRuntime {
            collection: Some(prepared),
            ..Default::default()
        },
    );
    drop(lifecycle);
    assert_eq!(
        super::update_local_access_image_generation_model_catalog(None, None, Some(actual.clone()))
            .await
            .unwrap_err(),
        "codex.localAccess.imageGenerationModel.deleteSelected"
    );
    super::provider_gateway_runtime_store()
        .lock()
        .await
        .remove(&key);
    let deleted =
        super::update_local_access_image_generation_model_catalog(None, None, Some(actual.clone()))
            .await
            .unwrap();
    assert!(deleted
        .collection
        .unwrap()
        .image_generation_deleted_models
        .contains(&actual));
    let (next_profile, _, _) =
        super::build_provider_gateway_collection_for_profile(&profile, &account).unwrap();
    assert_eq!(
        next_profile.image_generation_account_ids,
        vec![account.id.clone()]
    );
    assert!(
        super::sidecar_account_manifest_value(&account, None, &next_profile)
            .get("imageGenerationUpstream")
            .is_none()
    );
    let mut runtime = super::gateway_runtime().lock().await;
    runtime.collection = previous.0;
    runtime.loaded = previous.1;
    runtime.running = previous.2;
    drop(runtime);
    fs::remove_dir_all(profile).unwrap();
}
