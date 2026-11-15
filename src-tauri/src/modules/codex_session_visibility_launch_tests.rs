#[test]
fn desktop_api_key_launch_restores_onboarding_state_without_project_metadata() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join(CONFIG_FILE_NAME), "cli_auth_credentials_store = 'file'\n").unwrap();
    let auth = br#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-key"}"#;
    fs::write(temp.path().join("auth.json"), auth).unwrap();
    let state = json!({
        "electron-account-renderer-state-v1":{"owner":null,"inactive":{}},
        "electron-persisted-atom-state":{"electron:onboarding-projectless-completed":true}
    });
    let state_path = temp.path().join(GLOBAL_STATE_FILE);
    fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    // CLI launches keep the native renderer state intact.
    repair_session_visibility_quick_for_instance("onboarding-api-service", "Fixture", temp.path(), false)
        .unwrap();
    assert_eq!(serde_json::from_slice::<JsonValue>(&fs::read(&state_path).unwrap()).unwrap(), state);
    // The same desktop entry is exercised on macOS and Windows.
    let result = repair_session_visibility_quick_for_instance(
        "onboarding-api-service", "Fixture", temp.path(), true,
    ).unwrap();
    assert!(!result.local_project_recovery_deferred);
    let prepared: JsonValue = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    assert!(prepared.get("electron-account-renderer-state-v1").is_none());
    assert_eq!(prepared["electron-persisted-atom-state"]["electron:onboarding-projectless-completed"], true);
    assert_eq!(fs::read(temp.path().join("auth.json")).unwrap(), auth);
}

#[test]
fn launch_repairs_rollout_with_large_metadata_without_changing_contents() {
    let fixture = VisibilityMtimeFixture::new();
    let mut rollout = json!({
        "type": "session_meta",
        "payload": {"id": "mtime-thread", "model_provider": "old", "extra": "x".repeat(1024 * 1024)}
    }).to_string().into_bytes();
    rollout.push(b'\n');
    fs::write(&fixture.rollout, &rollout).unwrap();
    fs::File::open(&fixture.rollout).unwrap().set_modified(fixture.original_time).unwrap();
    let result = repair_session_visibility_quick_for_instance(
        "large-header", "Large header", &fixture.dir, false,
    ).unwrap();
    assert_eq!(result.changed_rollout_file_count, 1);
    assert_eq!(read_thread_provider(&fixture.dir, "mtime-thread"), "relay");
    fixture.assert_hash(&rollout);
    for backup in result.backup_dirs {
        fs::remove_dir_all(backup).unwrap();
    }
}

#[test]
fn session_repair_preserves_legacy_and_managed_backups() {
    let fixture = VisibilityMtimeFixture::new();
    let old = fixture
        .dir
        .join("backup-20200101-000000-session-visibility-repair");
    let newest = fixture
        .dir
        .join("backup-20200102-000000-session-visibility-repair");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&newest).unwrap();
    let scope = modules::backup_storage::scope_for_path(&fixture.dir);
    let managed = [old.file_name().unwrap(), newest.file_name().unwrap()].map(|name| {
        let path = modules::backup_storage::behavior_backup_dir(
            "codex", &scope, name.to_str().unwrap(),
        ).unwrap();
        fs::write(path.join("manifest.json"), name.to_str().unwrap()).unwrap();
        path
    });
    let summary = fixture.summary(None).unwrap();
    assert_eq!(summary.backup_dirs.len(), 1);
    assert!(Path::new(&summary.backup_dirs[0]).join("manifest.json").is_file());
    assert!(old.exists() && newest.exists());
    for path in managed {
        assert_eq!(
            fs::read_to_string(path.join("manifest.json")).unwrap(),
            path.file_name().unwrap().to_str().unwrap(),
        );
        fs::remove_dir_all(path).unwrap();
    }
    for backup in summary.backup_dirs {
        fs::remove_dir_all(backup).unwrap();
    }
}
#[test]
fn only_pre_mutation_process_snapshot_failures_can_be_deferred() {
    assert!(can_defer_launch_snapshot_error("CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE"));
    for error in [
        "CODEX_SESSION_VISIBILITY_ROLLBACK_FAILED: CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE",
        "CODEX_DESKTOP_STATE_PREPARATION_FAILED: CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE",
        "database is locked",
    ] {
        assert!(!can_defer_launch_snapshot_error(error), "{error}");
    }
}
