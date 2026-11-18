#[test]
fn automatic_session_repair_modes_apply_distinct_changes_to_the_selected_profile() {
    use modules::config::CodexAutomaticSessionRepair::{IndexOnly, Off, Quick};

    for mode in [Off, IndexOnly, Quick] {
        let fixture = VisibilityMtimeFixture::new();
        let summary = repair_session_visibility_for_instance(
            "selected-profile", "Fixture", &fixture.dir, false, mode,
        ).unwrap();
        let provider = read_thread_provider(&fixture.dir, "mtime-thread");
        assert_eq!(provider, if mode == Off { "openai" } else { "relay" });
        assert_eq!(fixture.time(), if mode == Quick { fixture.target_time } else { fixture.original_time });
        assert_eq!(summary.changed_rollout_file_count, usize::from(mode == Quick));
        fixture.assert_hash(&fixture.original);
        if mode == Off {
            assert!(summary.backup_dirs.is_empty());
        }
        for backup in summary.backup_dirs {
            fs::remove_dir_all(backup).unwrap();
        }
    }
}

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
    let result = repair_session_visibility_for_instance(
        "onboarding-api-service", "Fixture", temp.path(), true,
        modules::config::CodexAutomaticSessionRepair::Off,
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
fn session_repair_removes_only_the_failed_backup_attempt() {
    let fixture = VisibilityMtimeFixture::new();
    let data_dir = &fixture.dir;
    let provider_before = read_thread_provider(data_dir, "mtime-thread");
    let scope = modules::backup_storage::scope_for_path(data_dir);
    let previous = modules::backup_storage::behavior_backup_dir(
        "codex", &scope, "backup-20200101-000000-session-visibility-repair",
    ).unwrap();
    fs::write(previous.join("manifest.json"), "previous backup").unwrap();
    let scope_dir = previous.parent().unwrap();

    // The SQLite copy completes, then the session-index copy fails.
    let index_path = data_dir.join(SESSION_INDEX_FILE);
    fs::create_dir(&index_path).unwrap();
    let result = backup_instance_files(
        data_dir, &[], true, true, false, "mtime-fixture", "relay",
        repair_options(CodexSessionVisibilityRepairMode::Quick),
    );
    assert!(result.unwrap_err().contains("备份 session_index.jsonl 失败"));
    let entries: Vec<_> = fs::read_dir(scope_dir).unwrap()
        .map(|entry| entry.unwrap().path()).collect();
    assert_eq!(entries, vec![previous.clone()]);
    assert_eq!(fs::read_to_string(previous.join("manifest.json")).unwrap(), "previous backup");
    assert_eq!(read_thread_provider(data_dir, "mtime-thread"), provider_before);

    fs::remove_dir(&index_path).unwrap();
    fs::write(&index_path, "fixture index").unwrap();
    let backup = backup_instance_files(
        data_dir, &[], true, true, false, "mtime-fixture", "relay",
        repair_options(CodexSessionVisibilityRepairMode::Quick),
    ).unwrap();
    assert!(backup.join("manifest.json").is_file());
    assert!(backup.join("db").join(STATE_DB_FILE).is_file());
    assert_eq!(fs::read_to_string(backup.join("files").join(SESSION_INDEX_FILE)).unwrap(), "fixture index");
    fs::remove_dir_all(scope_dir).unwrap();
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

#[test]
fn referenced_rollout_paths_stay_under_the_instance_root() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("instance");
    fs::create_dir(&root).unwrap();
    let relative = Path::new("sessions/rollout-inside.jsonl");
    assert_eq!(resolve_rollout_path(&root, relative.to_str().unwrap()), root.join(relative));
    assert_eq!(referenced_rollout_relative_path(&root, &root.join(relative)).as_deref(), Some(relative));
    let outside = fixture.path().join("rollout-outside.jsonl");
    fs::write(&outside, "fixture").unwrap();
    assert!(referenced_rollout_relative_path(&root, &outside).is_none());
    assert!(referenced_rollout_relative_path(&root, &root.join("../rollout-outside.jsonl")).is_none());
}

#[cfg(windows)]
#[test]
fn extended_unc_rollout_reference_remains_absolute() {
    let data_dir = Path::new(r"\\?\UNC\server\share\instance");
    let reference = r"\\?\UNC\server\share\instance\sessions\rollout-latest.jsonl";
    let resolved = resolve_rollout_path(data_dir, reference);
    assert!(resolved.is_absolute());
    assert_eq!(resolved, Path::new(reference));
    assert_eq!(
        referenced_rollout_relative_path(data_dir, &resolved),
        Some(PathBuf::from(r"sessions\rollout-latest.jsonl")),
    );
}

#[cfg(windows)]
#[test]
fn launch_repairs_a_verbatim_rollout_reference_under_a_regular_drive_path() {
    let fixture = VisibilityMtimeFixture::new();
    let canonical_root = fixture.dir.canonicalize().unwrap();
    let root = Path::new(canonical_root.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    let canonical_rollout = fixture.rollout.canonicalize().unwrap();
    let db = Connection::open(root.join(STATE_DB_FILE)).unwrap();
    db.execute("UPDATE threads SET rollout_path = ?1", [canonical_rollout.to_str().unwrap()]).unwrap();
    drop(db);

    let summary = repair_session_visibility_quick_for_instance("windows-reference", "Fixture", root, false).unwrap();
    assert_eq!(summary.changed_rollout_file_count, 1);
    assert_eq!(fixture.time(), fixture.target_time);
    fixture.assert_hash(&fixture.original);
    for backup in summary.backup_dirs {
        fs::remove_dir_all(backup).unwrap();
    }
}
