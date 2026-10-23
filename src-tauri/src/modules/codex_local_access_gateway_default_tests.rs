use super::*;

fn old_collection(mode: Option<&str>) -> Value {
    let mut value = json!({
        "enabled": true,
        "port": 18555,
        "apiKey": "fixture-api-key",
        "accountIds": ["account-a", "account-b"],
        "createdAt": 1,
        "updatedAt": 2,
        "upstreamProxyUrl": "http://127.0.0.1:18111",
        "futureField": {"historyReference": "keep-original", "opaque": [1, 2, 3]}
    });
    if let Some(mode) = mode {
        value["gatewayMode"] = json!(mode);
    }
    value
}

#[test]
fn native_gateway_default_new_collections_use_native_without_starting_service() {
    assert_eq!(
        CodexLocalAccessGatewayMode::default(),
        CodexLocalAccessGatewayMode::Native
    );
    for collection in [
        new_local_access_collection().unwrap(),
        new_empty_local_access_collection().unwrap(),
    ] {
        assert_eq!(collection.gateway_mode, CodexLocalAccessGatewayMode::Native);
        assert!(collection.native_gateway_default_migrated);
        assert!(!collection.enabled);
    }
}

#[test]
fn native_gateway_default_first_runtime_load_migrates_every_old_mode_and_preserves_data() {
    for mode in [None, Some("legacy"), Some("sidecar"), Some("native")] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("collection.json");
        let mut original = old_collection(mode);
        original["enabled"] = json!(mode != Some("sidecar"));
        std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        let loaded = load_collection_for_runtime_from_path(&path)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.gateway_mode, CodexLocalAccessGatewayMode::Native);
        assert!(loaded.native_gateway_default_migrated);
        let persisted: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["gatewayMode"], "native");
        assert_eq!(persisted["nativeGatewayDefaultMigrated"], true);
        for key in [
            "enabled",
            "port",
            "apiKey",
            "accountIds",
            "createdAt",
            "upstreamProxyUrl",
            "futureField",
        ] {
            assert_eq!(persisted[key], original[key], "mode={mode:?}, key={key}");
        }
        let saved = std::fs::read(&path).unwrap();
        load_collection_for_runtime_from_path(&path).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            saved,
            "migration must not rewrite on restart"
        );
    }
}

#[test]
fn native_gateway_default_keeps_later_go_choice_after_stale_save_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("collection.json");
    std::fs::write(&path, old_collection(Some("sidecar")).to_string()).unwrap();
    let mut stale = load_collection_for_runtime_from_path(&path)
        .unwrap()
        .unwrap();
    let selected_go = native_patch_mode(
        &std::fs::read_to_string(&path).unwrap(),
        CodexLocalAccessGatewayMode::Sidecar,
    )
    .unwrap();
    std::fs::write(&path, selected_go).unwrap();
    // A pre-upgrade settings/statistics snapshot does not own the selection or marker.
    stale.native_gateway_default_migrated = false;
    stale.debug_logs = !stale.debug_logs;
    save_collection_to_path(&path, &stale).unwrap();
    let loaded = load_collection_for_runtime_from_path(&path)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.gateway_mode, CodexLocalAccessGatewayMode::Sidecar);
    assert!(loaded.native_gateway_default_migrated);
    assert_eq!(loaded.debug_logs, stale.debug_logs);
    assert_eq!(loaded.port, 18555);
    assert_eq!(loaded.api_key, "fixture-api-key");
    assert_eq!(loaded.account_ids, vec!["account-a", "account-b"]);
}

#[test]
fn native_gateway_default_migration_and_explicit_go_choice_are_serialized() {
    for _ in 0..8 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("collection.json");
        std::fs::write(&path, old_collection(Some("sidecar")).to_string()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            let barrier = &barrier;
            let path = &path;
            scope.spawn(move || {
                barrier.wait();
                migrate_native_gateway_default_on_disk(path).unwrap();
            });
            scope.spawn(move || {
                barrier.wait();
                update_string_atomic(path, |current| {
                    native_patch_mode(current.unwrap(), CodexLocalAccessGatewayMode::Sidecar)
                })
                .unwrap();
            });
        });
        let loaded = load_collection_for_runtime_from_path(&path)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.gateway_mode, CodexLocalAccessGatewayMode::Sidecar);
        assert!(loaded.native_gateway_default_migrated);
    }
}

#[test]
fn native_gateway_default_migration_failure_keeps_original_file_retryable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("collection.json");
    assert!(migrate_native_gateway_default_on_disk(&path).is_err());
    assert!(!path.exists());
    let invalid = "{ incomplete configuration";
    std::fs::write(&path, invalid).unwrap();
    assert!(migrate_native_gateway_default_on_disk(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), invalid);
    std::fs::write(&path, old_collection(None).to_string()).unwrap();
    assert_eq!(
        migrate_native_gateway_default_on_disk(&path)
            .unwrap()
            .gateway_mode,
        CodexLocalAccessGatewayMode::Native
    );
}

#[test]
fn native_gateway_default_catalog_reads_do_not_migrate_revision_snapshots() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("collection.json");
    let original = old_collection(Some("sidecar")).to_string();
    std::fs::write(&path, &original).unwrap();
    let loaded = load_collection_from_path(&path).unwrap().unwrap();
    assert_eq!(loaded.gateway_mode, CodexLocalAccessGatewayMode::Sidecar);
    assert!(!loaded.native_gateway_default_migrated);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    let initialized = load_collection_for_runtime_from_path(&path)
        .unwrap()
        .unwrap();
    assert_eq!(
        initialized.gateway_mode,
        CodexLocalAccessGatewayMode::Native
    );
    assert!(initialized.native_gateway_default_migrated);
}
