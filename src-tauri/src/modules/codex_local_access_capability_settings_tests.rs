use super::*;

fn configured() -> CodexLocalAccessCapabilityFallbackConfig {
    serde_json::from_value(json!({
        "enabled":true,"accountIds":["b","a"],"overrides":[
            {"accountId":"b","model":"*","capabilities":{"function":"supported"}},
            {"accountId":"a","model":"Final-Model","capabilities":{"file_search":"unsupported"}}
        ]
    }))
    .unwrap()
}

fn collection() -> CodexLocalAccessCollection {
    let mut collection = new_empty_local_access_collection().unwrap();
    collection.account_ids = vec!["a".into(), "b".into()];
    collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
    collection
}

#[test]
fn automatic_capability_settings_ignore_legacy_opt_out_and_keep_declarations_roundtrip() {
    let legacy = serde_json::to_value(collection()).unwrap();
    let mut legacy = legacy.as_object().unwrap().clone();
    legacy.remove("capabilityFallback");
    let collection: CodexLocalAccessCollection = serde_json::from_value(legacy.into()).unwrap();
    assert_eq!(collection.capability_fallback, Default::default());
    assert!(collection.capability_fallback.enabled);
    let retired: CodexLocalAccessCapabilityFallbackConfig = serde_json::from_value(json!({"enabled":false})).unwrap();
    assert!(retired.enabled);
    let config = configured();
    let value = serde_json::to_value(&config).unwrap();
    assert_eq!(value["accountIds"], json!(["b", "a"]));
    assert_eq!(value["overrides"][0]["accountId"], "b");
    assert_eq!(
        value["overrides"][0]["capabilities"]["function"],
        "supported"
    );
    assert!(value.get("account_ids").is_none());
    assert_eq!(
        serde_json::from_value::<CodexLocalAccessCapabilityFallbackConfig>(value).unwrap(),
        config
    );
    assert_eq!(CODEX_CAPABILITY_IDS.len(), 17);
    assert!(!CODEX_CAPABILITY_IDS.contains(&"unknown_tool"));
}

#[test]
fn capability_settings_validate_members_models_duplicates_and_known_capabilities() {
    let members = collection().account_ids;
    let mut config = configured();
    config.account_ids = vec![" b ".into(), "b".into(), "a".into()];
    assert_eq!(
        normalize_capability_fallback(config, &members)
            .unwrap()
            .account_ids,
        vec!["b", "a"]
    );
    let mut config = configured();
    config.account_ids.clear();
    assert!(normalize_capability_fallback(config, &members).unwrap().enabled);
    let mut config = configured();
    config.account_ids.push("outside".into());
    assert_eq!(
        normalize_capability_fallback(config, &members).unwrap_err(),
        "CAPABILITY_FALLBACK_ACCOUNT_NOT_IN_POOL"
    );
    for model in ["", "prefix*", "bad\nmodel"] {
        let mut config = configured();
        config.overrides[0].model = model.into();
        assert_eq!(
            normalize_capability_fallback(config, &members).unwrap_err(),
            "CAPABILITY_FALLBACK_INVALID_MODEL"
        );
    }
    let mut config = configured();
    config.overrides.push(config.overrides[0].clone());
    assert_eq!(
        normalize_capability_fallback(config, &members).unwrap_err(),
        "CAPABILITY_FALLBACK_DUPLICATE_OVERRIDE"
    );
    let mut config = configured();
    config.overrides[0].capabilities.insert(
        "unknown_tool".into(),
        CodexLocalAccessCapabilitySupport::Supported,
    );
    assert_eq!(
        normalize_capability_fallback(config, &members).unwrap_err(),
        "CAPABILITY_FALLBACK_INVALID_CAPABILITY"
    );
}

#[test]
fn capability_settings_survive_stale_disk_and_runtime_snapshot_publications() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    let original = collection();
    save_collection_to_path(&path, &original).unwrap();
    let mut stale = original.clone();
    stale.debug_logs = !original.debug_logs;
    let published = persist_capability_fallback_at_path(&path, configured(), None).unwrap();
    save_collection_to_path(&path, &stale).unwrap();
    let reloaded: CodexLocalAccessCollection =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(reloaded.capability_fallback, configured());
    assert_eq!(reloaded.debug_logs, stale.debug_logs);
    preserve_runtime_collection_fields(&mut stale, &published);
    assert_eq!(stale.capability_fallback, configured());
    let mut removed = reloaded;
    removed.account_ids.retain(|id| id != "b");
    save_collection_to_path(&path, &removed).unwrap();
    let reloaded: CodexLocalAccessCollection =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(reloaded.capability_fallback.account_ids, vec!["a"]);
    assert!(reloaded
        .capability_fallback
        .overrides
        .iter()
        .all(|entry| entry.account_id == "a"));
    // Runtime publication must also keep deletion pruning, rather than resurrecting a backup.
    preserve_runtime_collection_fields(&mut removed, &published);
    assert_eq!(removed.capability_fallback.account_ids, vec!["a"]);
}

#[test]
fn capability_settings_failed_saves_preserve_data_and_legacy_opt_out_cannot_disable_routing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.json");
    let collection = collection();
    save_collection_to_path(&path, &collection).unwrap();
    persist_capability_fallback_at_path(&path, configured(), None).unwrap();
    let before = fs::read(&path).unwrap();
    let mut invalid = configured();
    invalid.account_ids = vec!["outside".into()];
    assert!(persist_capability_fallback_at_path(&path, invalid, None).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let mut retired = configured();
    retired.enabled = false;
    let published = persist_capability_fallback_at_path(&path, retired.clone(), None).unwrap();
    assert_eq!(published.capability_fallback, configured());
    let mut sidecar = published;
    sidecar.gateway_mode = CodexLocalAccessGatewayMode::Legacy;
    write_string_atomic(&path, &serde_json::to_string(&sidecar).unwrap()).unwrap();
    assert!(persist_capability_fallback_at_path(&path, retired, None).unwrap().capability_fallback.enabled);
    fs::write(&path, "invalid-json").unwrap();
    assert_eq!(persist_capability_fallback_at_path(&path, configured(), None).unwrap_err(), "CAPABILITY_FALLBACK_STORAGE");
    assert_eq!(fs::read_to_string(&path).unwrap(), "invalid-json");
}

#[test]
fn capability_settings_account_edit_and_removal_clean_all_references() {
    let mut collection = collection();
    collection.capability_fallback = configured();
    assert!(replace_collection_account_references(
        &mut collection,
        "b",
        "new-b"
    ));
    assert_eq!(
        collection.capability_fallback.account_ids,
        vec!["new-b", "a"]
    );
    assert_eq!(
        collection.capability_fallback.overrides[0].account_id,
        "new-b"
    );
    assert!(remove_account_refs_from_collection(
        &mut collection,
        &HashSet::from(["new-b".into()])
    ));
    assert_eq!(collection.capability_fallback.account_ids, vec!["a"]);
    assert!(remove_account_refs_from_collection(
        &mut collection,
        &HashSet::from(["a".into()])
    ));
    assert!(collection.capability_fallback.enabled);
    assert!(collection.capability_fallback.account_ids.is_empty());
    assert!(collection.capability_fallback.overrides.is_empty());
}

#[test]
fn capability_settings_account_reference_collision_keeps_existing_target_declarations() {
    for source_first in [true, false] {
        let mut collection = collection();
        collection.capability_fallback = serde_json::from_value(json!({
            "enabled":true,"accountIds":["a","b","a"],"overrides":[
                {"accountId":"a","model":"Final-Model","capabilities":{"function":"supported","file_search":"supported"}},
                {"accountId":"b","model":"Final-Model","capabilities":{"function":"unsupported"}},
                {"accountId":"a","model":"Other-Model","capabilities":{"custom":"supported"}},
                {"accountId":"b","model":"*","capabilities":{"custom":"unsupported"}}
            ]
        })).unwrap();
        if !source_first {
            collection.capability_fallback.overrides.swap(0, 1);
        }
        assert!(replace_collection_account_references(
            &mut collection,
            "a",
            "b"
        ));
        assert_eq!(collection.capability_fallback.account_ids, vec!["b"]);
        let config = normalize_capability_fallback(
            collection.capability_fallback.clone(),
            &collection.account_ids,
        )
        .unwrap();
        assert_eq!(config.overrides.len(), 3);
        let target = config
            .overrides
            .iter()
            .find(|entry| entry.model == "Final-Model")
            .unwrap();
        assert_eq!(target.account_id, "b");
        assert_eq!(
            target.capabilities.len(),
            1,
            "do not merge source capabilities into a target declaration"
        );
        assert_eq!(
            target.capabilities["function"],
            CodexLocalAccessCapabilitySupport::Unsupported
        );
        assert_eq!(
            config
                .overrides
                .iter()
                .find(|entry| entry.model == "Other-Model")
                .unwrap()
                .account_id,
            "b"
        );
        assert_eq!(
            config
                .overrides
                .iter()
                .find(|entry| entry.model == "*")
                .unwrap()
                .capabilities["custom"],
            CodexLocalAccessCapabilitySupport::Unsupported
        );
    }
    let mut collection = collection();
    collection.account_ids.push("c".into());
    collection.capability_fallback = configured();
    collection.capability_fallback.account_ids =
        vec!["b".into(), "c".into(), "a".into(), "b".into()];
    replace_collection_account_references(&mut collection, "a", "b");
    assert_eq!(
        collection.capability_fallback.account_ids,
        vec!["b", "c"],
        "first occurrence keeps the configured order"
    );
}

struct TestData {
    _directory: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    previous_server: Option<std::ffi::OsString>,
}
impl TestData {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        // This existing test-only switch also isolates get_codex_home. No
        // request is made to this closed loopback endpoint by the settings test.
        std::env::set_var("CHATGPT_PLAN_TEST_SERVER", "http://127.0.0.1:9");
        Self {
            _directory: directory,
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
async fn capability_settings_command_saves_reads_and_reloads_without_starting_listener() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    with_chatgpt_plan_test_runtime(|| async {
        let _data = TestData::new();
        let path = local_access_file_path().unwrap();
        let mut original = collection();
        original.enabled = true;
        save_collection_to_path(&path, &original).unwrap();
        {
            let mut runtime = gateway_runtime().lock().await;
            runtime.collection = Some(original.clone());
            runtime.loaded = true;
        }
        let state = crate::commands::codex_capability_fallback::codex_local_access_update_capability_fallback(configured()).await.unwrap();
        assert!(!state.running);
        assert_eq!(state.collection.unwrap().capability_fallback, configured());
        // Exercise the actual delayed stats publication, including its runtime merge.
        save_collection_to_path(&path, &original).unwrap();
        {
            let mut runtime = gateway_runtime().lock().await;
            sync_runtime_collection(&mut runtime, original);
            assert_eq!(runtime.collection.as_ref().unwrap().capability_fallback, configured());
            runtime.collection = None;
            runtime.loaded = false;
        }
        ensure_runtime_loaded_without_start().await.unwrap();
        let state = snapshot_state_without_gateway_reload().await.unwrap();
        assert!(!state.running);
        let mut removed = state.collection.unwrap();
        assert_eq!(removed.capability_fallback, configured());
        assert!(remove_account_refs_from_collection(&mut removed, &HashSet::from(["a".into(), "b".into()])));
        save_collection_to_path(&path, &removed).unwrap();
        let reloaded = load_collection_for_runtime_from_path(&path).unwrap().unwrap();
        assert!(reloaded.capability_fallback.enabled);
        assert!(reloaded.capability_fallback.account_ids.is_empty());
        assert!(reloaded.capability_fallback.overrides.is_empty());
        let runtime = gateway_runtime().lock().await;
        assert!(runtime.native_handle.is_none() && runtime.sidecar_child.is_none());
    }).await;
}

async fn seed_command_runtime() -> PathBuf {
    let path = local_access_file_path().unwrap();
    let mut original = collection();
    original.enabled = true;
    save_collection_to_path(&path, &original).unwrap();
    let mut runtime = gateway_runtime().lock().await;
    runtime.collection = Some(original);
    runtime.loaded = true;
    path
}

async fn queue_waiting_save<F: std::future::Future>(mut save: std::pin::Pin<&mut F>) {
    // Polling once while the dedicated mutex is held deterministically queues
    // the future. This checks FIFO without a timer or scheduler-order assumption.
    std::future::poll_fn(|context| {
        assert!(std::future::Future::poll(save.as_mut(), context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn capability_settings_concurrent_commands_publish_in_disk_transaction_order() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    with_chatgpt_plan_test_runtime(|| async {
        let _data = TestData::new();
        let path = seed_command_runtime().await;
        let first_config = configured();
        let mut last_config = configured();
        last_config.account_ids = vec!["a".into()];
        last_config.overrides.clear();
        let barrier = capability_fallback_save_lock().lock().await;
        let mut first = Box::pin(update_local_access_capability_fallback(
            first_config.clone(),
        ));
        let mut last = Box::pin(update_local_access_capability_fallback(last_config.clone()));
        queue_waiting_save(first.as_mut()).await;
        queue_waiting_save(last.as_mut()).await;
        drop(barrier);
        let (first, last) =
            tokio::time::timeout(Duration::from_secs(10), async { tokio::join!(first, last) })
                .await
                .expect("ordered settings saves must complete");
        assert_eq!(
            first.unwrap().collection.unwrap().capability_fallback,
            first_config
        );
        assert_eq!(
            last.unwrap().collection.unwrap().capability_fallback,
            last_config
        );
        let persisted: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted.capability_fallback, last_config);
        assert_eq!(
            gateway_runtime()
                .lock()
                .await
                .collection
                .as_ref()
                .unwrap()
                .capability_fallback,
            last_config
        );
    })
    .await;
}

#[tokio::test]
async fn capability_settings_failed_queued_save_releases_lock_and_preserves_last_publication() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    with_chatgpt_plan_test_runtime(|| async {
        let _data = TestData::new();
        let path = seed_command_runtime().await;
        let good = configured();
        let mut invalid = good.clone();
        invalid.account_ids = vec!["outside".into()];
        let barrier = capability_fallback_save_lock().lock().await;
        let mut first = Box::pin(update_local_access_capability_fallback(good.clone()));
        let mut failed = Box::pin(update_local_access_capability_fallback(invalid));
        queue_waiting_save(first.as_mut()).await;
        queue_waiting_save(failed.as_mut()).await;
        drop(barrier);
        let (first, failed) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(first, failed)
        })
        .await
        .expect("failed save must release the command lock");
        assert_eq!(first.unwrap().collection.unwrap().capability_fallback, good);
        assert_eq!(failed.unwrap_err(), "CAPABILITY_FALLBACK_ACCOUNT_NOT_IN_POOL");
        let before_failure = fs::read(&path).unwrap();
        assert_eq!(
            gateway_runtime()
                .lock()
                .await
                .collection
                .as_ref()
                .unwrap()
                .capability_fallback,
            good
        );
        let persisted: CodexLocalAccessCollection =
            serde_json::from_slice(&before_failure).unwrap();
        assert_eq!(persisted.capability_fallback, good);
        // Disk/parse failure must have the same recovery behavior as validation failure.
        fs::write(&path, "invalid-json").unwrap();
        assert_eq!(
            update_local_access_capability_fallback(good.clone())
                .await
                .unwrap_err(),
            "CAPABILITY_FALLBACK_STORAGE"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid-json");
        assert_eq!(
            gateway_runtime()
                .lock()
                .await
                .collection
                .as_ref()
                .unwrap()
                .capability_fallback,
            good
        );
        write_string_atomic(&path, std::str::from_utf8(&before_failure).unwrap()).unwrap();
        let mut recovered = good;
        recovered.account_ids.clear();
        let state = tokio::time::timeout(
            Duration::from_secs(10),
            update_local_access_capability_fallback(recovered.clone()),
        )
        .await
        .expect("retry must acquire the lock after failure")
        .unwrap();
        assert_eq!(state.collection.unwrap().capability_fallback, recovered);
        let persisted: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.capability_fallback, recovered);
    })
    .await;
}
