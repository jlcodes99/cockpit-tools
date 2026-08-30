// Codex Session Visibility 测试：跨实例修复、SQLite 迁移和备份恢复。
// 测试作为原 tests 模块内容被 include，super 引用保持不变。
use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }

    fn repair_options(
        mode: CodexSessionVisibilityRepairMode,
    ) -> CodexSessionVisibilityRepairOptions {
        CodexSessionVisibilityRepairOptions::for_mode(mode)
    }

    #[test]
    fn provider_discovery_uses_config_and_official_state_db_without_scanning_rollouts() {
        let data_dir = make_temp_dir("codex-session-provider-discovery-test");
        fs::write(
            data_dir.join(CONFIG_FILE_NAME),
            "model_provider = \"relay\"\n[model_providers.alt]\nname = \"alternate\"\n",
        )
        .expect("write config");

        let rollout_path = data_dir.join("sessions/2026/08/24/rollout-only.jsonl");
        fs::create_dir_all(rollout_path.parent().expect("rollout parent"))
            .expect("create rollout dir");
        fs::write(
            rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"model_provider\":\"rollout-only\"}}\n",
        )
        .expect("write rollout");

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('legacy-thread', 'legacy')",
                [],
            )
            .expect("insert provider");
        drop(connection);

        let instances = vec![CodexSyncInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: data_dir.clone(),
            last_pid: None,
        }];
        let result = collect_session_visibility_repair_providers_for_instances(&instances)
            .expect("discover providers");
        let ids = result
            .providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<HashSet<_>>();

        assert!(ids.contains("relay"));
        assert!(ids.contains("alt"));
        assert!(ids.contains("legacy"));
        assert!(!ids.contains("rollout-only"));
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    fn write_quick_repair_rollout_reference(
        data_dir: &Path,
        thread_id: &str,
        relative_path: &Path,
        content: &[u8],
    ) -> PathBuf {
        fs::write(
            data_dir.join(CONFIG_FILE_NAME),
            "model_provider = \"relay\"\n",
        )
        .expect("write config");

        let rollout_path = data_dir.join(relative_path);
        fs::create_dir_all(rollout_path.parent().expect("rollout parent"))
            .expect("create rollout dir");
        fs::write(&rollout_path, content).expect("write rollout");

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");
        let rollout_relative = relative_path.to_string_lossy().replace('\\', "/");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES (?1, ?2, 'openai', 0, 'hello', '')",
                (thread_id, rollout_relative.as_str()),
            )
            .expect("insert thread");
        drop(connection);
        rollout_path
    }

    fn read_thread_provider(data_dir: &Path, thread_id: &str) -> String {
        Connection::open(data_dir.join(STATE_DB_FILE))
            .expect("open sqlite for read")
            .query_row(
                "SELECT model_provider FROM threads WHERE id = ?1",
                [thread_id],
                |row| row.get(0),
            )
            .expect("read provider")
    }

    #[test]
    fn quick_repair_skips_compressed_referenced_rollout() {
        let data_dir = make_temp_dir("codex-session-compressed-reference-test");
        let rollout_bytes = [0x28, 0xb5, 0x2f, 0xfd, 0xff, 0x00];
        let rollout_path = write_quick_repair_rollout_reference(
            &data_dir,
            "compressed-thread",
            Path::new("archived_sessions/rollout-compressed-thread.jsonl.zst"),
            &rollout_bytes,
        );

        let summary = repair_session_visibility_quick_for_instance(
            "compressed-instance",
            "Compressed instance",
            &data_dir,
        )
        .expect("compressed rollout should not abort quick repair");

        assert_eq!(summary.instance_count, 1);
        assert_eq!(
            read_thread_provider(&data_dir, "compressed-thread"),
            "relay"
        );
        assert_eq!(
            fs::read(&rollout_path).expect("read rollout"),
            rollout_bytes
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_repair_skips_non_utf8_plain_rollout() {
        let data_dir = make_temp_dir("codex-session-non-utf8-reference-test");
        let rollout_bytes = [0xff, 0xfe, 0xfd, b'\n'];
        let rollout_path = write_quick_repair_rollout_reference(
            &data_dir,
            "non-utf8-thread",
            Path::new("sessions/2026/07/17/rollout-non-utf8-thread.jsonl"),
            &rollout_bytes,
        );

        let summary = repair_session_visibility_quick_for_instance(
            "non-utf8-instance",
            "Non UTF-8 instance",
            &data_dir,
        )
        .expect("non UTF-8 rollout should not abort quick repair");

        assert_eq!(summary.instance_count, 1);
        assert_eq!(read_thread_provider(&data_dir, "non-utf8-thread"), "relay");
        assert_eq!(
            fs::read(&rollout_path).expect("read rollout"),
            rollout_bytes
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn desktop_workspace_path_normalizes_windows_extended_paths() {
        assert_eq!(
            to_desktop_workspace_path(r"\\?\D:\Andrew\Code\pxread\").as_deref(),
            Some(r"D:\Andrew\Code\pxread")
        );
        assert_eq!(
            to_desktop_workspace_path(r"\\?\UNC\server\share\repo\").as_deref(),
            Some(r"\\server\share\repo")
        );
        assert_eq!(
            to_desktop_workspace_path(r"\\?\C:\").as_deref(),
            Some(r"C:\")
        );
        assert_eq!(
            to_desktop_workspace_path("/Users/demo/project/").as_deref(),
            Some("/Users/demo/project/")
        );
    }

    #[test]
    fn quick_sqlite_repair_normalizes_existing_windows_extended_cwds() {
        let data_dir = make_temp_dir("codex-session-cwd-normalization-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    cwd TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, cwd) VALUES
                 ('drive', 'relay', '\\\\?\\D:\\Andrew\\Code\\pxread\\'),
                 ('unc', 'relay', '\\\\?\\UNC\\server\\share\\repo\\'),
                 ('posix', 'relay', '/Users/demo/project/')",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 2);

        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
        assert_eq!(updated_rows, 2);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let drive = connection
            .query_row("SELECT cwd FROM threads WHERE id = 'drive'", [], |row| {
                row.get::<usize, String>(0)
            })
            .expect("read drive cwd");
        let unc = connection
            .query_row("SELECT cwd FROM threads WHERE id = 'unc'", [], |row| {
                row.get::<usize, String>(0)
            })
            .expect("read unc cwd");
        let posix = connection
            .query_row("SELECT cwd FROM threads WHERE id = 'posix'", [], |row| {
                row.get::<usize, String>(0)
            })
            .expect("read posix cwd");
        assert_eq!(drive, r"D:\Andrew\Code\pxread");
        assert_eq!(unc, r"\\server\share\repo");
        assert_eq!(posix, "/Users/demo/project/");
        connection
            .execute(
                "UPDATE threads SET cwd = '\\\\?\\D:\\Andrew\\Code\\pxread\\' WHERE id = 'drive'",
                [],
            )
            .expect("simulate metadata rebuild");
        drop(connection);

        assert_eq!(
            normalize_official_thread_cwds(&data_dir).expect("normalize rebuilt metadata"),
            1
        );
        let connection = Connection::open(&db_path).expect("reopen normalized sqlite");
        let rebuilt_drive = connection
            .query_row("SELECT cwd FROM threads WHERE id = 'drive'", [], |row| {
                row.get::<usize, String>(0)
            })
            .expect("read rebuilt drive cwd");
        assert_eq!(rebuilt_drive, r"D:\Andrew\Code\pxread");
        drop(connection);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn relay_openai_base_url_keeps_history_visibility_on_builtin_openai() {
        let data_dir = make_temp_dir("codex-session-visibility-relay-provider-test");
        fs::write(
            data_dir.join(CONFIG_FILE_NAME),
            "openai_base_url = \"https://relay.example.com/v1\"\n",
        )
        .expect("write relay config");

        assert_eq!(
            read_history_visibility_provider_for_dir(&data_dir)
                .expect("read history visibility provider"),
            DEFAULT_PROVIDER_ID
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_marks_threads_with_first_user_message_visible() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES
                 ('matched-invisible', 'relay', 0, 'hello', ''),
                 ('old-invisible', 'old', 0, 'hi', NULL),
                 ('already-visible', 'relay', 1, 'visible', 'user'),
                 ('provider-only', '', 0, '', NULL)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 3);
        assert!(!scan.skipped_unusable_database);

        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
        assert_eq!(updated_rows, 3);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let matched_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'matched-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read matched row");
        assert_eq!(
            matched_invisible,
            ("relay".to_string(), 1, "user".to_string())
        );

        let old_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'old-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read old row");
        assert_eq!(old_invisible, ("relay".to_string(), 1, "user".to_string()));

        let provider_only = connection
            .query_row(
                "SELECT model_provider, has_user_event FROM threads WHERE id = 'provider-only'",
                [],
                |row| Ok((row.get::<usize, String>(0)?, row.get::<usize, i64>(1)?)),
            )
            .expect("read provider-only row");
        assert_eq!(provider_only, ("relay".to_string(), 0));

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_sqlite_repair_targets_official_sidebar_rows_only() {
        let data_dir = make_temp_dir("codex-session-sidebar-visible-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT,
                    archived INTEGER,
                    preview TEXT,
                    source TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (
                    id, rollout_path, model_provider, has_user_event,
                    first_user_message, thread_source, archived, preview, source
                 ) VALUES
                 ('visible-old', 'sessions/visible.jsonl', 'old', 1, 'visible', 'user', 0, 'visible', 'vscode'),
                 ('empty-preview', 'sessions/preview.jsonl', 'relay', 1, 'preview fallback', 'user', 0, '', 'vscode'),
                 ('archived-old', 'archived_sessions/archived.jsonl', 'old', 1, 'archived', 'user', 1, 'archived', 'vscode'),
                 ('missing-rollout', '', 'old', 1, 'missing', 'user', 0, 'missing', 'vscode'),
                 ('subagent-old', 'sessions/subagent.jsonl', 'old', 1, 'subagent', NULL, 0, 'subagent', '{\"subagent\":{\"other\":\"guardian\"}}')",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 2);

        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
        assert_eq!(updated_rows, 2);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let visible_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'visible-old'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read visible provider");
        let repaired_preview = connection
            .query_row(
                "SELECT preview FROM threads WHERE id = 'empty-preview'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read repaired preview");
        assert_eq!(visible_provider, "relay");
        assert_eq!(repaired_preview, "preview fallback");
        for hidden_id in ["archived-old", "missing-rollout", "subagent-old"] {
            let provider = connection
                .query_row(
                    "SELECT model_provider FROM threads WHERE id = ?1",
                    [hidden_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("read hidden provider");
            assert_eq!(provider, "old");
        }

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_keeps_provider_only_schema_working() {
        let data_dir = make_temp_dir("codex-session-provider-only-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('old', 'old'), ('same', 'relay')",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 1);
        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
        assert_eq!(updated_rows, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let old_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'old'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read old provider");
        assert_eq!(old_provider, "relay");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_repair_uses_official_state_dbs_without_touching_rollouts() {
        let data_dir = make_temp_dir("codex-session-quick-official-state-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let legacy_db_path = data_dir.join(STATE_DB_FILE);
        let unrelated_db_path = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        for db_path in [&official_db_path, &legacy_db_path, &unrelated_db_path] {
            let connection = Connection::open(db_path).expect("open sqlite");
            connection
                .execute(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        model_provider TEXT,
                        has_user_event INTEGER,
                        first_user_message TEXT,
                        thread_source TEXT
                    )",
                    [],
                )
                .expect("create threads table");
            connection
                .execute(
                    "INSERT INTO threads (id, model_provider, has_user_event, first_user_message, thread_source)
                     VALUES ('thread-1', 'old', 0, 'hello', '')",
                    [],
                )
                .expect("insert row");
        }

        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("16");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-thread-1.jsonl");
        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan quick sqlite");
        assert_eq!(scan.rows_to_update, 2);
        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &[],
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("quick repair");
        assert_eq!(repaired.updated_sqlite_rows, 2);
        assert_eq!(repaired.updated_sqlite_timestamp_rows, 0);
        assert_eq!(repaired.added_session_index_entries, 0);
        assert_eq!(repaired.updated_session_index_entries, 0);

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read rollout"),
            rollout_content
        );

        for db_path in [&official_db_path, &legacy_db_path] {
            let connection = Connection::open(db_path).expect("reopen sqlite");
            let row = connection
                .query_row(
                    "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'thread-1'",
                    [],
                    |row| {
                        Ok((
                            row.get::<usize, String>(0)?,
                            row.get::<usize, i64>(1)?,
                            row.get::<usize, String>(2)?,
                        ))
                    },
                )
                .expect("read repaired row");
            assert_eq!(row, ("relay".to_string(), 1, "user".to_string()));
        }

        let connection = Connection::open(&unrelated_db_path).expect("reopen unrelated sqlite");
        let unrelated_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read unrelated provider");
        assert_eq!(unrelated_provider, "old");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_repair_updates_rollouts_referenced_by_official_state_dbs() {
        let data_dir = make_temp_dir("codex-session-quick-referenced-rollout-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let connection = Connection::open(&official_db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");

        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("17");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let referenced_rollout = rollout_dir.join("rollout-thread-1.jsonl");
        let unreferenced_rollout = rollout_dir.join("rollout-thread-2.jsonl");
        let referenced_relative = referenced_rollout
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                [referenced_relative.as_str()],
            )
            .expect("insert thread");
        drop(connection);

        let old_line = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old-later\"}}\n"
        );
        let unreferenced_line =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-2\",\"model_provider\":\"old\"}}\n";
        fs::write(&referenced_rollout, old_line).expect("write referenced rollout");
        fs::write(&unreferenced_rollout, unreferenced_line).expect("write unreferenced rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let rollout_changes =
            collect_referenced_rollout_provider_changes(&data_dir, "relay", options, &selection)
                .expect("collect referenced rollout changes");
        assert_eq!(rollout_changes.len(), 1);
        assert_eq!(rollout_changes[0].absolute_path, referenced_rollout);

        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &rollout_changes,
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("quick repair");
        assert_eq!(repaired.updated_sqlite_rows, 1);

        let referenced_content =
            fs::read_to_string(&referenced_rollout).expect("read referenced rollout");
        assert!(referenced_content.contains("\"model_provider\":\"relay\""));
        assert!(referenced_content.contains("\"model_provider\":\"old-later\""));
        assert_eq!(
            fs::read_to_string(&unreferenced_rollout).expect("read unreferenced rollout"),
            unreferenced_line
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deep_mode_repairs_all_session_databases_and_rollout_metadata() {
        let data_dir = make_temp_dir("codex-session-deep-compat-official-state-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let unrelated_db_path = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("17");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let referenced_rollout = rollout_dir.join("rollout-thread-1.jsonl");
        let referenced_relative = referenced_rollout
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");

        for db_path in [&official_db_path, &unrelated_db_path] {
            let connection = Connection::open(db_path).expect("open sqlite");
            connection
                .execute(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        rollout_path TEXT,
                        model_provider TEXT,
                        has_user_event INTEGER,
                        first_user_message TEXT,
                        thread_source TEXT
                    )",
                    [],
                )
                .expect("create threads table");
            connection
                .execute(
                    "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                     VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                    [referenced_relative.as_str()],
                )
                .expect("insert row");
        }
        fs::write(
            &referenced_rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n",
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old-later\"}}\n"
            ),
        )
        .expect("write referenced rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Deep);
        assert_eq!(options.mode, CodexSessionVisibilityRepairMode::Deep);
        assert_eq!(options.sqlite_scope, SqliteRepairScope::AllSessionDbs);
        assert!(options.repair_rollout);
        assert!(!options.repair_referenced_rollouts);
        assert!(options.rewrite_all_session_meta);
        assert!(options.collect_rollout_thread_facts);
        assert!(options.repair_local_thread_catalog);
        assert!(options.normalize_global_state);
        assert!(!options.repair_session_index);
        assert!(options.rebuild_metadata);
        assert!(options.require_stopped_instances);

        let selection = RepairTargetSelection::default();
        let rollout_changes =
            collect_rollout_provider_changes(&data_dir, "relay", options, &selection)
                .expect("collect deep rollout changes");
        assert_eq!(rollout_changes.len(), 1);
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan compatibility sqlite");
        assert_eq!(scan.rows_to_update, 2);

        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &rollout_changes,
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("compatibility repair");
        assert_eq!(repaired.updated_sqlite_rows, 2);

        let connection = Connection::open(&official_db_path).expect("reopen official sqlite");
        let official_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read official provider");
        assert_eq!(official_provider, "relay");

        let connection = Connection::open(&unrelated_db_path).expect("reopen unrelated sqlite");
        let unrelated_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read unrelated provider");
        assert_eq!(unrelated_provider, "relay");

        let referenced_content =
            fs::read_to_string(&referenced_rollout).expect("read deep repaired rollout");
        assert!(referenced_content.contains("\"model_provider\":\"relay\""));
        assert!(!referenced_content.contains("old-later"));

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn auto_repair_mode_stays_on_official_state_db_only() {
        let options = CodexSessionVisibilityRepairOptions::for_auto_repair_mode(
            CodexSessionVisibilityAutoRepairMode::Current,
        );
        assert_eq!(options.mode, CodexSessionVisibilityRepairMode::Quick);
        assert_eq!(options.sqlite_scope, SqliteRepairScope::OfficialStateDbs);
        assert!(!options.repair_rollout);
        assert!(options.repair_referenced_rollouts);
        assert!(!options.rewrite_all_session_meta);
        assert!(!options.repair_session_index);
        assert!(!options.rebuild_metadata);
    }

    #[test]
    fn sqlite_backup_restore_replaces_db_and_clears_sidecars() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-backup-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('thread-1', 'old')",
                [],
            )
            .expect("insert old row");
        drop(connection);

        let backup_dir = backup_instance_files(
            &data_dir,
            &[],
            true,
            false,
            false,
            "default",
            "relay",
            repair_options(CodexSessionVisibilityRepairMode::Quick),
        )
        .expect("backup db");

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        connection
            .execute(
                "UPDATE threads SET model_provider = 'new' WHERE id = 'thread-1'",
                [],
            )
            .expect("mutate db after backup");
        drop(connection);
        for path in sqlite_sidecar_paths(&db_path) {
            fs::write(path, b"stale wal/shm").expect("write stale sidecar");
        }

        restore_instance_files_from_backup(&data_dir, &backup_dir, true).expect("restore db");
        for path in sqlite_sidecar_paths(&db_path) {
            assert!(
                !path.exists(),
                "stale sidecar should be removed: {:?}",
                path
            );
        }

        let connection = Connection::open(&db_path).expect("open restored sqlite");
        let provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read restored provider");
        assert_eq!(provider, "old");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deep_repair_rebuilds_local_catalog_and_reports_encrypted_history() {
        let data_dir = make_temp_dir("codex-session-deep-catalog-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let state_db = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let connection = Connection::open(&state_db).expect("open state db");
        connection
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    created_at INTEGER,
                    updated_at INTEGER,
                    title TEXT,
                    cwd TEXT,
                    source TEXT,
                    model_provider TEXT,
                    git_branch TEXT,
                    thread_source TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT
                );
                CREATE TABLE thread_spawn_edges (child_thread_id TEXT);",
            )
            .expect("create state schema");
        connection
            .execute(
                "INSERT INTO threads VALUES ('user-1', '', 10, 20, 'User chat', 'C:\\\\repo', 'cli', 'old', 'main', 'user', 0, 'hello')",
                [],
            )
            .expect("insert user thread");
        connection
            .execute(
                "INSERT INTO threads VALUES ('child-1', '', 11, 21, 'Child', 'C:\\\\repo', '{\"subagent\":{}}', 'old', NULL, 'subagent', 0, '')",
                [],
            )
            .expect("insert child thread");
        connection
            .execute("INSERT INTO thread_spawn_edges VALUES ('child-1')", [])
            .expect("insert spawn edge");
        drop(connection);

        let catalog_db = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        let connection = Connection::open(&catalog_db).expect("open catalog db");
        connection
            .execute_batch(
                "CREATE TABLE local_thread_catalog_hosts (host_id TEXT PRIMARY KEY, host_kind TEXT);
                 INSERT INTO local_thread_catalog_hosts VALUES ('local', 'local');
                 CREATE TABLE local_thread_catalog (
                    host_id TEXT NOT NULL,
                    thread_id TEXT NOT NULL,
                    display_title TEXT NOT NULL,
                    source_created_at REAL NOT NULL,
                    source_updated_at REAL NOT NULL,
                    cwd TEXT,
                    source_kind TEXT NOT NULL,
                    source_detail TEXT,
                    model_provider TEXT,
                    git_branch TEXT,
                    observation_sequence INTEGER NOT NULL,
                    missing_candidate INTEGER NOT NULL DEFAULT 0,
                    thread_source TEXT,
                    source_recency_at REAL NOT NULL DEFAULT 0,
                    pending_observed_title INTEGER NOT NULL DEFAULT 0,
                    project_id TEXT,
                    PRIMARY KEY (host_id, thread_id)
                 );
                 CREATE TABLE local_thread_catalog_metadata (id INTEGER PRIMARY KEY, catalog_revision INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO local_thread_catalog_metadata VALUES (1, 0);
                 CREATE TABLE local_thread_catalog_sync_state (
                    host_id TEXT PRIMARY KEY,
                    watermark_updated_at REAL,
                    initial_build_complete INTEGER NOT NULL DEFAULT 0,
                    observation_sequence INTEGER NOT NULL DEFAULT 0,
                    last_full_reconciled_at INTEGER
                 );
                 INSERT INTO local_thread_catalog_sync_state VALUES ('local', 0, 0, 0, 0);
                 INSERT INTO local_thread_catalog (
                    host_id, thread_id, display_title, source_created_at, source_updated_at,
                    cwd, source_kind, model_provider, observation_sequence, thread_source
                 ) VALUES ('local', 'child-1', 'Child', 11, 21, 'C:\\repo', 'subagent', 'old', 1, 'subagent');",
            )
            .expect("create catalog schema");
        drop(connection);

        let rollout_dir = data_dir.join("sessions").join("2026").join("08").join("24");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        fs::write(
            rollout_dir.join("rollout-user-1.jsonl"),
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"user-1\",\"model_provider\":\"old\",\"cwd\":\"C:\\\\\\\\repo\",\"source\":\"cli\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"user_message\",\"encrypted_content\":\"secret\"}}\n"
            ),
        )
        .expect("write user rollout");
        fs::write(
            rollout_dir.join("rollout-child-1.jsonl"),
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"child-1\",\"model_provider\":\"old\",\"source\":{\"subagent\":{}}}}\n",
        )
        .expect("write child rollout");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            "{\"project-order\":[\"\\\\\\\\?\\\\C:\\\\repo\\\\\",\"C:\\\\repo\"]}",
        )
        .expect("write global state");

        let selection = RepairTargetSelection::default();
        let facts = collect_rollout_thread_facts(&data_dir, &selection).expect("collect facts");
        assert!(facts.user_event_thread_ids.contains("user-1"));
        assert!(facts.subagent_thread_ids.contains("child-1"));
        assert_eq!(facts.encrypted_content_counts.get("old"), Some(&1));

        let preview =
            repair_local_thread_catalog_for_options(&data_dir, "relay", &selection, &facts, true)
                .expect("preview catalog repair");
        assert_eq!(preview.inserted_rows, 1);
        assert_eq!(preview.removed_rows, 1);

        let repaired =
            repair_local_thread_catalog_for_options(&data_dir, "relay", &selection, &facts, false)
                .expect("repair catalog");
        assert_eq!(repaired.inserted_rows, 1);
        assert_eq!(repaired.removed_rows, 1);
        let connection = Connection::open(&catalog_db).expect("reopen catalog db");
        let user_provider = connection
            .query_row(
                "SELECT model_provider FROM local_thread_catalog WHERE thread_id = 'user-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("read repaired catalog row");
        assert_eq!(user_provider, "relay");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM local_thread_catalog WHERE thread_id = 'child-1'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count child catalog row"),
            0
        );

        assert_eq!(
            normalize_global_state(&data_dir, true).expect("preview state"),
            1
        );
        assert_eq!(
            normalize_global_state(&data_dir, false).expect("repair state"),
            1
        );
        assert!(
            build_encrypted_content_warning(&facts.encrypted_content_counts, "relay").is_some()
        );

        let changes = collect_rollout_provider_changes(
            &data_dir,
            "relay",
            repair_options(CodexSessionVisibilityRepairMode::Deep),
            &selection,
        )
        .expect("collect rollout changes");
        assert_eq!(changes.len(), 1);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn dry_run_reports_planned_changes_without_writing_files_or_backups() {
        let data_dir = make_temp_dir("codex-session-dry-run-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let connection = Connection::open(&official_db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");

        let rollout_dir = data_dir.join("sessions").join("2026").join("07").join("03");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-thread-1.jsonl");
        let rollout_relative = rollout_path
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                [rollout_relative.as_str()],
            )
            .expect("insert thread");
        drop(connection);

        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick).with_dry_run(true);
        let selection = RepairTargetSelection::from_inputs(Some("relay".to_string()), None, None)
            .expect("selection");
        let summary = repair_session_visibility_for_instances_with_options(
            options,
            None,
            None,
            selection,
            vec![CodexSyncInstance {
                id: "dry-run-instance".to_string(),
                name: "Dry run instance".to_string(),
                data_dir: data_dir.clone(),
                last_pid: None,
            }],
        )
        .expect("dry run summary");

        assert_eq!(summary.instance_count, 1);
        assert_eq!(summary.mutated_instance_count, 1);
        assert_eq!(summary.changed_rollout_file_count, 1);
        assert_eq!(summary.updated_sqlite_row_count, 1);
        assert!(summary.backup_dirs.is_empty());
        assert_eq!(summary.items.len(), 1);
        assert!(summary.items[0].backup_dir.is_none());
        assert!(summary.message.contains("预览将"));

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read rollout after dry run"),
            rollout_content
        );
        let connection = Connection::open(&official_db_path).expect("reopen sqlite");
        let row = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read unchanged row");
        assert_eq!(row, ("old".to_string(), 0, "".to_string()));

        let backup_count = fs::read_dir(&data_dir)
            .expect("read data dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)
            })
            .count();
        assert_eq!(backup_count, 0);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn launch_target_quick_repair_is_bidirectional_idempotent_and_instance_scoped() {
        fn write_fixture(
            data_dir: &Path,
            thread_id: &str,
            configured_provider: &str,
            history_provider: &str,
        ) -> (PathBuf, PathBuf) {
            fs::write(
                data_dir.join(CONFIG_FILE_NAME),
                format!("model_provider = \"{}\"\n", configured_provider),
            )
            .expect("write config");

            let rollout_dir = data_dir.join("sessions").join("2026").join("07").join("14");
            fs::create_dir_all(&rollout_dir).expect("create rollout dir");
            let rollout_path = rollout_dir.join(format!("rollout-{}.jsonl", thread_id));
            let rollout_relative = rollout_path
                .strip_prefix(data_dir)
                .expect("relative rollout")
                .to_string_lossy()
                .replace('\\', "/");
            fs::write(
                &rollout_path,
                format!(
                    "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"model_provider\":\"{}\"}}}}\n",
                    thread_id, history_provider
                ),
            )
            .expect("write rollout");

            let db_path = data_dir.join(STATE_DB_FILE);
            let connection = Connection::open(&db_path).expect("open sqlite");
            connection
                .execute(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        rollout_path TEXT,
                        model_provider TEXT,
                        has_user_event INTEGER,
                        first_user_message TEXT,
                        thread_source TEXT
                    )",
                    [],
                )
                .expect("create threads table");
            connection
                .execute(
                    "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                     VALUES (?1, ?2, ?3, 0, 'hello', '')",
                    (thread_id, rollout_relative.as_str(), history_provider),
                )
                .expect("insert thread");
            drop(connection);
            (db_path, rollout_path)
        }

        fn read_provider(db_path: &Path, thread_id: &str) -> String {
            Connection::open(db_path)
                .expect("open sqlite for read")
                .query_row(
                    "SELECT model_provider FROM threads WHERE id = ?1",
                    [thread_id],
                    |row| row.get(0),
                )
                .expect("read provider")
        }

        let target_dir = make_temp_dir("codex-launch-target-repair-test");
        let other_dir = make_temp_dir("codex-launch-other-instance-test");
        let (target_db, target_rollout) =
            write_fixture(&target_dir, "target-thread", "relay", "openai");
        let (other_db, other_rollout) =
            write_fixture(&other_dir, "other-thread", "openai", "other-provider");
        let other_rollout_before = fs::read(&other_rollout).expect("read other rollout before");

        let first = repair_session_visibility_quick_for_instance(
            "target-instance",
            "Target instance",
            &target_dir,
        )
        .expect("repair target instance");
        assert_eq!(first.instance_count, 1);
        assert_eq!(first.mutated_instance_count, 1);
        assert_eq!(first.items[0].instance_id, "target-instance");
        assert_eq!(read_provider(&target_db, "target-thread"), "relay");
        assert!(fs::read_to_string(&target_rollout)
            .expect("read target rollout")
            .contains("\"model_provider\":\"relay\""));

        assert_eq!(read_provider(&other_db, "other-thread"), "other-provider");
        assert_eq!(
            fs::read(&other_rollout).expect("read other rollout after"),
            other_rollout_before
        );
        assert!(fs::read_dir(&other_dir)
            .expect("read other instance dir")
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)));

        let second = repair_session_visibility_quick_for_instance(
            "target-instance",
            "Target instance",
            &target_dir,
        )
        .expect("repeat target repair");
        assert_eq!(second.mutated_instance_count, 0);
        assert_eq!(second.changed_rollout_file_count, 0);
        assert_eq!(second.updated_sqlite_row_count, 0);

        fs::write(
            target_dir.join(CONFIG_FILE_NAME),
            "model_provider = \"openai\"\n",
        )
        .expect("switch target back to account provider");
        let switched_back = repair_session_visibility_quick_for_instance(
            "target-instance",
            "Target instance",
            &target_dir,
        )
        .expect("repair target after provider switch");
        assert_eq!(switched_back.mutated_instance_count, 1);
        assert_eq!(read_provider(&target_db, "target-thread"), "openai");
        assert!(fs::read_to_string(&target_rollout)
            .expect("read switched-back rollout")
            .contains("\"model_provider\":\"openai\""));

        fs::remove_dir_all(&target_dir).expect("cleanup target dir");
        fs::remove_dir_all(&other_dir).expect("cleanup other dir");
    }
