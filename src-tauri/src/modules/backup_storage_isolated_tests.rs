// Exercise the full production migration transaction, using only disposable
// Cockpit/client homes. External legacy-backup discovery must also be isolated.
struct MigrationFixture {
    root: PathBuf,
    source: PathBuf,
    target: PathBuf,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl MigrationFixture {
    fn new() -> Self {
        let root = make_temp_dir("isolated-migration");
        let home = root.join("home");
        let data = root.join("data");
        let codex = home.join(".codex");
        for path in [&home, &data, &codex] {
            fs::create_dir_all(path).unwrap();
        }
        let mut previous = Vec::new();
        for (key, value) in [
            ("HOME", home),
            ("CODEX_HOME", codex),
            ("COCKPIT_TOOLS_TEST_DATA_DIR", data),
        ] {
            previous.push((key, std::env::var_os(key)));
            std::env::set_var(key, value);
        }
        let source = root.join("source");
        let target = root.join("target");
        backup_directory_recovery::save_override(&source).unwrap();
        Self {
            root,
            source,
            target,
            previous,
        }
    }

    fn seed(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut entries = Vec::new();
        // A real SQLite snapshot and a large scheduled backup exercise the
        // nested manifest, chunked copier, verification and publication paths.
        let sqlite_path = self
            .source
            .join("behavior/codex/scope/operation/state.sqlite");
        fs::create_dir_all(sqlite_path.parent().unwrap()).unwrap();
        let db = rusqlite::Connection::open(&sqlite_path).unwrap();
        db.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT); INSERT INTO threads VALUES ('qa-thread', '中文🙂');").unwrap();
        drop(db);
        entries.push((
            sqlite_path
                .strip_prefix(&self.source)
                .unwrap()
                .to_path_buf(),
            fs::read(&sqlite_path).unwrap(),
        ));
        let scheduled = PathBuf::from("cockpit_auto_backup.json");
        let content = serde_json::to_vec(&serde_json::json!({
            "schema": "qa-isolated-backup",
            "accounts": (0..1500).map(|id| serde_json::json!({"id": id, "email": format!("qa-{id}@example.invalid"), "notes": "测试🙂".repeat(30)})).collect::<Vec<_>>()
        })).unwrap();
        assert!(content.len() > 512 * 1024);
        fs::write(self.source.join(&scheduled), &content).unwrap();
        entries.push((scheduled, content));
        entries
    }

    fn assert_sources(&self, entries: &[(PathBuf, Vec<u8>)]) {
        for (relative, bytes) in entries {
            assert_eq!(fs::read(self.source.join(relative)).unwrap(), *bytes);
        }
        assert_eq!(get_backup_root_dir().unwrap(), self.source);
        assert!(!fs::read_dir(&self.target)
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(MIGRATION_STAGING_PREFIX)));
    }
}

impl Drop for MigrationFixture {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn isolated_migration_moves_sqlite_and_large_scheduled_backup_and_rereads_preference() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = MigrationFixture::new();
    let entries = fixture.seed();
    let preview = preview_backup_root_dir_change(&fixture.target.to_string_lossy()).unwrap();
    assert!(preview.source_error.is_none());
    assert_eq!(preview.file_count, entries.len() as u64);
    let mut phases = Vec::new();
    let result = change_backup_root_dir_with_progress(
        &fixture.target.to_string_lossy(),
        true,
        "isolated-success",
        |progress| phases.push(progress.phase),
    )
    .unwrap();
    assert!(result.migrated);
    assert_eq!(result.migrated_file_count, entries.len() as u64);
    assert_eq!(
        result.migrated_size_bytes,
        entries
            .iter()
            .map(|(_, bytes)| bytes.len() as u64)
            .sum::<u64>()
    );
    assert!(result.remaining_paths.is_empty());
    assert_eq!(get_backup_root_dir().unwrap(), fixture.target);
    assert_eq!(
        backup_directory_recovery::read_override().unwrap(),
        Some(fixture.target.clone())
    );
    for (relative, bytes) in &entries {
        assert_eq!(fs::read(fixture.target.join(relative)).unwrap(), *bytes);
        assert!(!fixture.source.join(relative).exists());
    }
    let db = rusqlite::Connection::open(fixture.target.join(&entries[0].0)).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT title FROM threads WHERE id='qa-thread'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "中文🙂"
    );
    assert!(phases.contains(&"copying".to_owned()));
    assert!(phases.contains(&"verifying".to_owned()));
    assert_eq!(phases.last().unwrap(), "completed");
}

#[test]
fn isolated_migration_cancel_mid_copy_preserves_sources_and_can_retry() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = MigrationFixture::new();
    let entries = fixture.seed();
    let mut requested = false;
    let error = change_backup_root_dir_with_progress(
        &fixture.target.to_string_lossy(),
        true,
        "isolated-cancel",
        |progress| {
            if !requested && progress.phase == "copying" && progress.processed_size_bytes > 0 {
                requested = true;
                assert!(cancel_backup_root_dir_change("isolated-cancel").unwrap());
            }
        },
    )
    .unwrap_err();
    assert!(requested);
    assert_eq!(error, "backup_migration_cancelled");
    fixture.assert_sources(&entries);
    assert!(fs::read_dir(&fixture.target).unwrap().next().is_none());
    assert!(ensure_backup_write_available().is_ok());
    assert!(
        change_backup_root_dir_with_progress(
            &fixture.target.to_string_lossy(),
            true,
            "isolated-retry",
            |_| {}
        )
        .unwrap()
        .migrated
    );
}

#[test]
fn isolated_migration_rolls_back_publication_on_concurrent_destination_collision() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = MigrationFixture::new();
    let entries = fixture.seed();
    // Sorting publishes behavior/ SQLite first; creating the later JSON
    // destination exercises rollback of an already-published file.
    let error = change_backup_root_dir_with_progress(
        &fixture.target.to_string_lossy(),
        true,
        "isolated-write-failure",
        |progress| {
            if progress.phase == "switching" {
                fs::write(
                    fixture.target.join("cockpit_auto_backup.json"),
                    b"external concurrent file",
                )
                .unwrap();
            }
        },
    )
    .unwrap_err();
    assert!(error.contains("迁移目标文件已存在"));
    fixture.assert_sources(&entries);
    assert!(!fixture.target.join(&entries[0].0).exists());
    assert_eq!(
        fs::read(fixture.target.join("cockpit_auto_backup.json")).unwrap(),
        b"external concurrent file"
    );
    assert!(ensure_backup_write_available().is_ok());
}

#[test]
fn isolated_migration_rejects_reentry_and_preserves_new_source_writes_during_cleanup() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let fixture = MigrationFixture::new();
    let entries = fixture.seed();
    let mut blocked = false;
    let result = change_backup_root_dir_with_progress(
        &fixture.target.to_string_lossy(),
        true,
        "isolated-concurrency",
        |progress| {
            if progress.phase == "copying" && !blocked {
                blocked = true;
                assert!(ensure_backup_write_available().is_err());
                assert!(change_backup_root_dir_with_progress(
                    &fixture.root.join("other-target").to_string_lossy(),
                    false,
                    "isolated-reentry",
                    |_| {}
                )
                .is_err());
            }
            if progress.phase == "cleaning" {
                fs::write(
                    fixture.source.join("cockpit_auto_backup.json"),
                    b"newer backup from in-flight old writer",
                )
                .unwrap();
                fs::write(
                    fixture.source.join("created-during-migration.json"),
                    b"new file",
                )
                .unwrap();
            }
        },
    )
    .unwrap();
    assert!(blocked);
    assert_eq!(
        fs::read(fixture.source.join("cockpit_auto_backup.json")).unwrap(),
        b"newer backup from in-flight old writer"
    );
    assert_eq!(
        fs::read(fixture.source.join("created-during-migration.json")).unwrap(),
        b"new file"
    );
    assert!(result
        .remaining_paths
        .iter()
        .any(|path| path.ends_with("cockpit_auto_backup.json")));
    assert_eq!(
        fs::read(fixture.target.join("cockpit_auto_backup.json")).unwrap(),
        entries[1].1
    );
    assert!(ensure_backup_write_available().is_ok());
}
