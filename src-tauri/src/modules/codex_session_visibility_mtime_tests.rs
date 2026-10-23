struct VisibilityMtimeFixture {
    dir: PathBuf,
    rollout: PathBuf,
    original: Vec<u8>,
    original_time: SystemTime,
    target_time: SystemTime,
}

impl VisibilityMtimeFixture {
    fn new() -> Self {
        let dir = make_temp_dir("visibility-mtime-transaction");
        let content = concat!(
            "{ \"type\":\"session_meta\",\"payload\":{\"id\":\"mtime-thread\",\"model_provider\":\"old\"}}\r\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"id\":\"raw-cursor\",\"content\":\"中文🙂\"}}\r\n"
        );
        let rollout = write_quick_repair_rollout_reference(
            &dir,
            "mtime-thread",
            Path::new("sessions/2026/10/11/rollout-01-mtime.jsonl"),
            content.as_bytes(),
        );
        fs::File::open(&rollout)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789))
            .unwrap();
        let original_time = fs::metadata(&rollout).unwrap().modified().unwrap();
        let target_time = UNIX_EPOCH + Duration::from_millis(1_780_362_123_234);
        let db = Connection::open(dir.join(STATE_DB_FILE)).unwrap();
        db.execute_batch("ALTER TABLE threads ADD COLUMN updated_at INTEGER; ALTER TABLE threads ADD COLUMN updated_at_ms INTEGER;").unwrap();
        db.execute(
            "UPDATE threads SET updated_at=1780362123, updated_at_ms=1780362123234",
            [],
        )
        .unwrap();
        drop(db);
        Self {
            dir,
            rollout,
            original: content.as_bytes().to_vec(),
            original_time,
            target_time,
        }
    }

    fn summary(
        &self,
        reporter: ProgressReporter<'_>,
    ) -> Result<CodexSessionVisibilityRepairSummary, String> {
        repair_session_visibility_for_instances_with_options(
            CodexSessionVisibilityRepairOptions::for_auto_repair_mode(
                CodexSessionVisibilityAutoRepairMode::Current,
            ),
            None,
            reporter,
            RepairTargetSelection::default(),
            vec![CodexSyncInstance {
                id: "mtime-fixture".into(),
                name: "Mtime fixture".into(),
                data_dir: self.dir.clone(),
                last_pid: None,
            }],
        )
    }

    fn time(&self) -> SystemTime {
        fs::metadata(&self.rollout).unwrap().modified().unwrap()
    }

    fn assert_hash(&self, expected: &[u8]) {
        use sha2::{Digest, Sha256};
        assert_eq!(
            Sha256::digest(fs::read(&self.rollout).unwrap()),
            Sha256::digest(expected)
        );
    }
}

impl Drop for VisibilityMtimeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn quick_entry_applies_actual_mtime_once_and_metadata_backup_restores_without_copying_history() {
    let fixture = VisibilityMtimeFixture::new();
    let first = repair_session_visibility_quick_for_instance(
        "mtime-fixture",
        "Mtime fixture",
        &fixture.dir,
    )
    .unwrap();
    assert_eq!(first.changed_rollout_file_count, 1);
    assert_eq!(first.items[0].changed_rollout_file_count, 1);
    assert_eq!(first.skipped_rollout_file_count, 0);
    assert_eq!(
        fixture.time(),
        fixture.target_time,
        "scan/report success must correspond to a real filesystem change"
    );
    fixture.assert_hash(&fixture.original);
    let backup = PathBuf::from(&first.backup_dirs[0]);
    let manifest: JsonValue =
        serde_json::from_slice(&fs::read(backup.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["rolloutTimestamps"].as_array().unwrap().len(), 1);
    assert!(manifest["rolloutFiles"].as_array().unwrap().is_empty());
    assert!(
        !backup.join("files/sessions").exists(),
        "mtime-only backup must never copy large append-only rollout contents"
    );
    let second = repair_session_visibility_quick_for_instance(
        "mtime-fixture",
        "Mtime fixture",
        &fixture.dir,
    )
    .unwrap();
    assert_eq!(second.changed_rollout_file_count, 0);
    assert_eq!(second.mutated_instance_count, 0);
    // Provider filtering alone can also make the second result zero. Check
    // the actual time and hash to catch the original discarded-plan defect.
    assert_eq!(fixture.time(), fixture.target_time);
    fixture.assert_hash(&fixture.original);
    restore_instance_files_from_backup(&fixture.dir, &backup, false).unwrap();
    assert_eq!(
        fixture.time(),
        fixture.original_time,
        "original nanosecond precision is recoverable"
    );
    fixture.assert_hash(&fixture.original);
    fs::remove_dir_all(backup).unwrap();
}

#[test]
fn quick_entry_skips_scan_then_append_and_never_reports_a_timestamp_change() {
    use std::io::Write;
    let fixture = VisibilityMtimeFixture::new();
    let appended_time = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let appended = b"{\"type\":\"event_msg\",\"text\":\"concurrent append\"}\r\n";
    let did_append = std::cell::Cell::new(false);
    let reporter = |progress: CodexSessionVisibilityRepairProgress| {
        if progress.stage == "write_rollout_files" && !did_append.replace(true) {
            fs::OpenOptions::new()
                .append(true)
                .open(&fixture.rollout)
                .unwrap()
                .write_all(appended)
                .unwrap();
            fs::File::open(&fixture.rollout)
                .unwrap()
                .set_modified(appended_time)
                .unwrap();
        }
    };
    let result = fixture.summary(Some(&reporter)).unwrap();
    assert!(did_append.get());
    assert_eq!(result.changed_rollout_file_count, 0);
    assert_eq!(result.items[0].changed_rollout_file_count, 0);
    assert_eq!(result.skipped_rollout_file_count, 1);
    assert_eq!(fixture.time(), appended_time);
    let mut full = fixture.original.clone();
    full.extend_from_slice(appended);
    fixture.assert_hash(&full);
    let backup = PathBuf::from(&result.backup_dirs[0]);
    restore_instance_files_from_backup(&fixture.dir, &backup, false).unwrap();
    assert_eq!(
        fixture.time(),
        appended_time,
        "rollback cannot alter the time of a newer append"
    );
    fixture.assert_hash(&full);
    fs::remove_dir_all(backup).unwrap();
}

#[test]
fn timestamp_backup_restore_cannot_change_replaced_or_appended_rollouts() {
    use std::io::Write;
    let fixture = VisibilityMtimeFixture::new();
    let result = fixture.summary(None).unwrap();
    let backup = PathBuf::from(&result.backup_dirs[0]);
    fs::OpenOptions::new()
        .append(true)
        .open(&fixture.rollout)
        .unwrap()
        .write_all(b"{\"type\":\"event_msg\"}\r\n")
        .unwrap();
    let appended_bytes = fs::read(&fixture.rollout).unwrap();
    let appended_time = fixture.time();
    restore_instance_files_from_backup(&fixture.dir, &backup, false).unwrap();
    fixture.assert_hash(&appended_bytes);
    assert_eq!(fixture.time(), appended_time);
    let replacement = fixture.dir.join("replacement.jsonl");
    fs::write(&replacement, &fixture.original).unwrap();
    fs::File::open(&replacement)
        .unwrap()
        .set_modified(fixture.target_time)
        .unwrap();
    fs::rename(&replacement, &fixture.rollout).unwrap();
    restore_instance_files_from_backup(&fixture.dir, &backup, false).unwrap();
    assert_eq!(
        fixture.time(),
        fixture.target_time,
        "same size/time replacement must retain its own metadata"
    );
    fixture.assert_hash(&fixture.original);
    fs::remove_dir_all(backup).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn quick_entry_real_mtime_setter_failure_rolls_back_prior_times_and_sqlite() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = VisibilityMtimeFixture::new();
    let second = fixture.rollout.with_file_name("rollout-02-immutable.jsonl");
    fs::write(
        &second,
        b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"immutable-thread\"}}\r\n",
    )
    .unwrap();
    fs::File::open(&second)
        .unwrap()
        .set_modified(fixture.original_time)
        .unwrap();
    let second_bytes = fs::read(&second).unwrap();
    let relative = second.strip_prefix(&fixture.dir).unwrap().to_string_lossy();
    let db = Connection::open(fixture.dir.join(STATE_DB_FILE)).unwrap();
    db.execute("INSERT INTO threads (id,rollout_path,model_provider,has_user_event,first_user_message,thread_source,updated_at_ms) VALUES ('immutable-thread',?1,'openai',0,'hello','',1780362123234)",[relative.as_ref()]).unwrap();
    drop(db);
    struct ImmutableGuard(std::ffi::CString);
    impl Drop for ImmutableGuard {
        fn drop(&mut self) {
            assert_eq!(unsafe { libc::chflags(self.0.as_ptr(), 0) }, 0);
        }
    }
    let guard = ImmutableGuard(std::ffi::CString::new(second.as_os_str().as_bytes()).unwrap());
    let reached = std::cell::Cell::new(false);
    let reporter = |progress: CodexSessionVisibilityRepairProgress| {
        if progress.stage == "write_rollout_files" && progress.current == 2 {
            assert_eq!(
                fixture.time(),
                fixture.target_time,
                "first planned time was actually committed"
            );
            assert_eq!(
                unsafe { libc::chflags(guard.0.as_ptr(), libc::UF_IMMUTABLE) },
                0
            );
            reached.set(true);
        }
    };
    let error = fixture.summary(Some(&reporter)).unwrap_err();
    assert!(reached.get());
    assert!(error.contains("已自动回滚"));
    assert_eq!(fixture.time(), fixture.original_time);
    fixture.assert_hash(&fixture.original);
    assert_eq!(fs::read(&second).unwrap(), second_bytes);
    assert_eq!(read_thread_provider(&fixture.dir, "mtime-thread"), "openai");
    assert_eq!(
        read_thread_provider(&fixture.dir, "immutable-thread"),
        "openai"
    );
    drop(guard);
    let retry = fixture.summary(None).unwrap();
    assert_eq!(retry.changed_rollout_file_count, 2);
    assert_eq!(fixture.time(), fixture.target_time);
    for path in retry.backup_dirs {
        fs::remove_dir_all(path).unwrap();
    }
}
