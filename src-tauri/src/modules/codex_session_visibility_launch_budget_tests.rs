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
fn launch_budget_interrupts_sql_and_rolls_back_its_transaction() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE sample(value INTEGER); INSERT INTO sample VALUES(0)")
        .unwrap();
    let started = std::time::Instant::now();
    let error = with_launch_visibility_budget(Duration::from_millis(30), || {
        configure_launch_visibility_connection(&connection)?;
        let transaction = connection.transaction().map_err(|error| error.to_string())?;
        transaction.execute("UPDATE sample SET value=1", []).unwrap();
        transaction.query_row(
            "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT sum(x) FROM n",
            [],
            |row| row.get::<_, i64>(0),
        ).map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }).unwrap_err();
    assert!(error.contains(LAUNCH_VISIBILITY_TIMEOUT), "{error}");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        connection
            .query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    with_launch_visibility_budget(Duration::from_secs(1), || {
        configure_launch_visibility_connection(&connection)?;
        connection
            .execute("UPDATE sample SET value=2", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT value FROM sample", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(launch_visibility_budget().is_none());
}

#[test]
fn expired_launch_budget_is_paused_for_existing_and_fresh_rollback_connections() {
    let existing = Connection::open_in_memory().unwrap();
    with_launch_visibility_budget(Duration::from_millis(10), || {
        configure_launch_visibility_connection(&existing)?;
        std::thread::sleep(Duration::from_millis(25));
        without_launch_visibility_budget(|| {
            assert!(launch_visibility_budget().is_none());
            // Keep recovery active over several watcher ticks. Previously
            // registered handles must be suspended, not merely hidden in TLS.
            std::thread::sleep(Duration::from_millis(30));
            let sql = "WITH RECURSIVE n(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM n WHERE x<200000) SELECT sum(x) FROM n";
            let expected = 20_000_100_000i64;
            assert_eq!(existing.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap(), expected);
            let fresh = Connection::open_in_memory().unwrap();
            configure_launch_visibility_connection(&fresh).unwrap();
            assert_eq!(fresh.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap(), expected);
        });
        Ok(())
    }).unwrap();
    assert!(launch_visibility_budget().is_none());
    existing
        .execute_batch("CREATE TABLE after_budget(id INTEGER)")
        .unwrap();
}

#[test]
fn launch_budget_does_not_detach_a_slow_read_or_leave_deadlines_on_next_worker() {
    let finished = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let error = with_launch_visibility_budget(Duration::from_millis(5), || {
        // Model an OS read that cannot be preempted. The repair API must wait
        // for it to return, then fail at a safe boundary before any mutation.
        std::thread::sleep(Duration::from_millis(40));
        finished.store(true, std::sync::atomic::Ordering::Release);
        check_launch_visibility_budget("slow_read_finished")
    })
    .unwrap_err();
    assert!(error.contains(LAUNCH_VISIBILITY_TIMEOUT));
    assert!(started.elapsed() >= Duration::from_millis(40));
    assert!(finished.load(std::sync::atomic::Ordering::Acquire));
    with_launch_visibility_budget(Duration::from_secs(1), || {
        check_launch_visibility_budget("next_launch")
    })
    .unwrap();
}

#[test]
fn oversized_launch_rollout_header_fails_before_database_or_history_mutation() {
    let fixture = VisibilityMtimeFixture::new();
    let oversized = vec![b'x'; MAX_LAUNCH_ROLLOUT_FIRST_LINE_BYTES as usize + 2];
    fs::write(&fixture.rollout, &oversized).unwrap();
    let original_time = fs::metadata(&fixture.rollout).unwrap().modified().unwrap();
    let error =
        repair_session_visibility_quick_for_instance("header-limit", "Header limit", &fixture.dir, false)
            .unwrap_err();
    assert!(error.contains(LAUNCH_VISIBILITY_LINE_TOO_LARGE), "{error}");
    assert_eq!(read_thread_provider(&fixture.dir, "mtime-thread"), "openai");
    assert_eq!(fs::read(&fixture.rollout).unwrap(), oversized);
    assert_eq!(
        fs::metadata(&fixture.rollout).unwrap().modified().unwrap(),
        original_time
    );
    // Non-launch readers retain their full explicit repair behavior.
    assert_eq!(
        read_first_line(&fixture.rollout).unwrap().unwrap().0.len(),
        oversized.len()
    );
}

#[test]
fn expired_launch_repair_restores_committed_changes_across_both_official_databases() {
    let fixture = VisibilityMtimeFixture::new();
    let second = fixture
        .dir
        .join(SQLITE_DIR_NAME)
        .join(OFFICIAL_STATE_DB_FILE);
    fs::create_dir_all(second.parent().unwrap()).unwrap();
    fs::copy(fixture.dir.join(STATE_DB_FILE), &second).unwrap();
    let reached = std::cell::Cell::new(false);
    let reporter = |progress: CodexSessionVisibilityRepairProgress| {
        if progress.stage == "write_rollout_files" && !reached.replace(true) {
            for path in [fixture.dir.join(STATE_DB_FILE), second.clone()] {
                assert_eq!(
                    Connection::open(path)
                        .unwrap()
                        .query_row(
                            "SELECT model_provider FROM threads WHERE id='mtime-thread'",
                            [],
                            |row| row.get::<_, String>(0)
                        )
                        .unwrap(),
                    "relay",
                    "both database transactions committed before timeout"
                );
            }
            let budget = launch_visibility_budget().unwrap();
            std::thread::sleep(
                budget
                    .deadline
                    .saturating_duration_since(std::time::Instant::now())
                    + Duration::from_millis(20),
            );
        }
    };
    let error =
        with_launch_visibility_budget(Duration::from_secs(1), || fixture.summary(Some(&reporter)))
            .unwrap_err();
    assert!(reached.get());
    assert!(error.contains(LAUNCH_VISIBILITY_TIMEOUT), "{error}");
    assert!(error.contains("已自动回滚"), "{error}");
    for path in [fixture.dir.join(STATE_DB_FILE), second] {
        assert_eq!(
            Connection::open(path)
                .unwrap()
                .query_row(
                    "SELECT model_provider FROM threads WHERE id='mtime-thread'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "openai"
        );
    }
    fixture.assert_hash(&fixture.original);
    assert_eq!(fixture.time(), fixture.original_time);
    let retry =
        repair_session_visibility_quick_for_instance("retry-after-timeout", "Retry", &fixture.dir, false)
            .unwrap();
    assert_eq!(retry.updated_sqlite_row_count, 2);
    assert_eq!(fixture.time(), fixture.target_time);
    for backup in retry.backup_dirs {
        fs::remove_dir_all(backup).unwrap();
    }
}

#[test]
fn rollback_failure_keeps_its_specific_error_after_launch_deadline() {
    let fixture = VisibilityMtimeFixture::new();
    let blocked = std::cell::Cell::new(false);
    let reporter = |progress: CodexSessionVisibilityRepairProgress| {
        if progress.stage == "write_rollout_files" && !blocked.replace(true) {
            let database = fixture.dir.join(STATE_DB_FILE);
            fs::rename(&database, fixture.dir.join("original-database.sqlite")).unwrap();
            fs::create_dir(&database).unwrap();
            let budget = launch_visibility_budget().unwrap();
            std::thread::sleep(
                budget
                    .deadline
                    .saturating_duration_since(std::time::Instant::now())
                    + Duration::from_millis(20),
            );
        }
    };
    let error =
        with_launch_visibility_budget(Duration::from_secs(1), || fixture.summary(Some(&reporter)))
            .unwrap_err();
    assert!(blocked.get());
    assert!(error.starts_with(LAUNCH_VISIBILITY_ROLLBACK_FAILED));
    assert!(error.contains("自动回滚也失败"));
    let backup = error.rsplit("备份目录: ").next().unwrap();
    assert!(Path::new(backup).join("manifest.json").is_file());
    assert!(Path::new(backup).join("db").join(STATE_DB_FILE).is_file());
    fs::remove_dir_all(backup).unwrap();
}

#[test]
fn launch_quick_repair_leaves_backup_retention_outside_critical_path() {
    let fixture = VisibilityMtimeFixture::new();
    let old = fixture
        .dir
        .join("backup-20200101-000000-session-visibility-repair");
    let newest = fixture
        .dir
        .join("backup-20200102-000000-session-visibility-repair");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&newest).unwrap();
    let summary =
        repair_session_visibility_quick_for_instance("retention", "Retention", &fixture.dir, false)
            .unwrap();
    assert!(old.exists() && newest.exists());
    for backup in summary.backup_dirs {
        fs::remove_dir_all(backup).unwrap();
    }
}
#[test]
fn only_pre_mutation_process_snapshot_failures_can_be_deferred() {
    assert!(can_defer_launch_snapshot_error("CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE"));
    assert!(can_defer_launch_snapshot_error("CODEX_SESSION_VISIBILITY_TIMEOUT: CODEX_SESSION_VISIBILITY_TIMEOUT: stage=process_snapshot"));
    for error in [
        "CODEX_SESSION_VISIBILITY_TIMEOUT: stage=sqlite",
        "CODEX_SESSION_VISIBILITY_TIMEOUT: stage=rollout_write",
        "CODEX_SESSION_VISIBILITY_ROLLBACK_FAILED: CODEX_SESSION_VISIBILITY_TIMEOUT: stage=process_snapshot",
        "database is locked",
    ] {
        assert!(!can_defer_launch_snapshot_error(error), "{error}");
    }
}
