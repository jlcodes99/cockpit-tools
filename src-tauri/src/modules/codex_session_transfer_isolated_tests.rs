// Real ZIP/file/index round trips. Only the external official app-server RPC
// is replaced, so no installed client or real instance is started.
fn isolated_export_fixture(
    base: &Path,
    session_id: &str,
) -> (CodexSyncInstance, ThreadSnapshot, SessionExportManifest) {
    let root = base.join("source");
    let rollout = root.join(format!("sessions/2026/10/11/rollout-{session_id}.jsonl"));
    write_rollout(&rollout, session_id, "中文🙂 round trip");
    let index = json!({"id": session_id, "thread_name": "QA export", "pinned": true, "cwd": "/tmp/project"});
    fs::write(root.join(SESSION_INDEX_FILE), format!("{index}\n")).unwrap();
    let instance = CodexSyncInstance {
        id: "qa-source".into(),
        name: "QA source".into(),
        data_dir: root.clone(),
        last_pid: None,
    };
    let snapshot = ThreadSnapshot {
        id: session_id.into(),
        title: "QA export".into(),
        cwd: "/tmp/project".into(),
        project_name: None,
        updated_at: Some(1_780_362_123),
        rollout_path: rollout.clone(),
        session_index_entry: index.clone(),
        source_root: root,
    };
    let (size_bytes, sha256) = sha256_file(&rollout).unwrap();
    let manifest = SessionExportManifest {
        kind: SESSION_EXPORT_KIND.into(),
        package_version: SESSION_EXPORT_VERSION,
        exported_at: "2026-10-11T00:00:00Z".into(),
        sessions: vec![SessionExportManifestItem {
            session_id: session_id.into(),
            title: "QA export".into(),
            cwd: "/tmp/project".into(),
            updated_at: snapshot.updated_at,
            relative_rollout_path: "sessions/2026/10/11/rollout-imported.jsonl".into(),
            file_entry: format!("files/{session_id}.jsonl"),
            size_bytes,
            sha256,
            session_index_entry: index,
            source_instance: SessionExportInstance {
                id: instance.id.clone(),
                name: instance.name.clone(),
            },
        }],
    };
    (instance, snapshot, manifest)
}

#[test]
fn isolated_export_import_trash_restore_roundtrip_keeps_bytes_index_and_package() {
    let base = make_temp_dir("codex-session-transfer-roundtrip");
    let (instance, snapshot, manifest) = isolated_export_fixture(&base, "qa-roundtrip");
    let original_bytes = fs::read(&snapshot.rollout_path).unwrap();
    let package = base.join("sessions.zip");
    write_session_export_package(&package, &manifest, &[(instance, snapshot)], None, None).unwrap();
    let original_package_hash = sha256_file(&package).unwrap();
    let disk_manifest = read_session_export_manifest_from_path(&package).unwrap();
    assert_eq!(disk_manifest.sessions[0].session_id, "qa-roundtrip");
    validate_manifest_item(&disk_manifest.sessions[0]).unwrap();
    let target_root = base.join("target");
    let target_path = resolve_import_target_rollout_path(&target_root, &disk_manifest.sessions[0]);
    let mut archive = ZipArchive::new(File::open(&package).unwrap()).unwrap();
    write_imported_rollout_from_archive(
        &mut archive,
        &disk_manifest.sessions[0],
        &target_path,
        None,
    )
    .unwrap();
    assert_eq!(fs::read(&target_path).unwrap(), original_bytes);
    let index = build_imported_session_index_entry(&disk_manifest.sessions[0], &target_path);
    upsert_session_index_with_entry(&target_root, "qa-roundtrip", &index).unwrap();
    let target = CodexSyncInstance {
        id: "qa-target".into(),
        name: "QA target".into(),
        data_dir: target_root.clone(),
        last_pid: None,
    };
    let imported = ThreadSnapshot {
        id: "qa-roundtrip".into(),
        title: "QA export".into(),
        cwd: "/tmp/project".into(),
        project_name: None,
        updated_at: Some(1_780_362_123),
        rollout_path: target_path.clone(),
        session_index_entry: index,
        source_root: target_root.clone(),
    };
    let trash_root = base.join("trash");
    let outcome = trash_snapshots_for_instance_with_app_server(
        &target,
        &trash_root.join("20261011-000000"),
        &[imported],
        |_, ids| { fs::remove_file(&target_path).unwrap(); Ok(ids.len()) },
        |_| Ok(()),
    )
    .unwrap();
    assert!(!outcome.official_delete_failed);
    assert!(!outcome.metadata_rebuild_failed);
    assert!(!target_path.exists());
    assert!(read_session_index_map(&target_root)
        .unwrap()
        .contains_key("qa-roundtrip"));
    let entries = load_trash_entries_from_roots(&[TrashRoot {
        path: trash_root,
        optional: false,
    }])
    .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        fs::read(&entries[0].trashed_rollout_path).unwrap(),
        original_bytes
    );
    restore_trashed_session_entry_with_metadata_rebuild(&entries[0], false).unwrap();
    assert_eq!(fs::read(&target_path).unwrap(), original_bytes);
    assert_eq!(
        read_session_index_map(&target_root).unwrap()["qa-roundtrip"]["pinned"],
        true
    );
    assert!(!entries[0].entry_dir.exists());
    assert_eq!(sha256_file(&package).unwrap(), original_package_hash);
    drop(archive);
    fs::remove_dir_all(base).unwrap();
}

#[test]
#[ignore = "requires disposable HOME, official app-server and OS-denied networking"]
fn offline_zip_import_is_visible_to_official_client_in_clean_profile() {
    assert_eq!(
        std::env::var("COCKPIT_CODEX_OFFLINE_INTEGRATION").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var_os("COCKPIT_CODEX_OFFLINE_ROOT").unwrap());
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let target = modules::codex_instance::get_default_codex_home().unwrap();
    assert!(root.is_absolute() && home.starts_with(&root) && target.starts_with(&root));
    assert!(modules::account::get_data_dir().unwrap().starts_with(&root));
    let denied = std::net::TcpStream::connect_timeout(
        &"127.0.0.1:9".parse().unwrap(),
        std::time::Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(matches!(denied.raw_os_error(), Some(1 | 13)), "{denied}");

    let source = root.join("previous-install");
    let project = source.join("original-project");
    fs::create_dir_all(&project).unwrap();
    let session_id = uuid::Uuid::new_v4().to_string();
    let relative = format!("sessions/2026/10/11/rollout-2026-10-11T03-00-00-{session_id}.jsonl");
    let rollout = source.join(&relative);
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let original = [
        json!({"timestamp":"2026-10-11T03:00:00Z","type":"session_meta","payload":{
            "id":session_id,"timestamp":"2026-10-11T03:00:00Z","cwd":project,
            "originator":"codex_cli_rs","cli_version":"0.162.0-alpha.17.2","source":"cli","model_provider":"openai"
        }}),
        json!({"timestamp":"2026-10-11T03:00:01Z","type":"event_msg","payload":{
            "type":"user_message","message":"Synthetic restored session","images":[],"local_images":[],"text_elements":[]
        }}),
    ].map(|row| format!("{row}\n")).concat();
    fs::write(&rollout, &original).unwrap();
    fs::write(source.join(SESSION_INDEX_FILE), format!("{}\n", json!({
        "id":session_id,"thread_name":"Synthetic restored session","updated_at":"2026-10-11T03:00:01Z"
    }))).unwrap();

    let mut store = modules::codex_instance::load_instance_store().unwrap();
    store.instances.push(crate::models::InstanceProfile {
        id: "previous-install".into(),
        name: "Previous install".into(),
        user_data_dir: source.to_string_lossy().into_owned(),
        working_dir: None,
        extra_args: String::new(),
        bind_account_id: None,
        model_routing: None,
        launch_mode: crate::models::InstanceLaunchMode::App,
        app_speed: crate::models::codex::CodexAppSpeed::Standard,
        created_at: 1,
        last_launched_at: None,
        last_pid: None,
    });
    modules::codex_instance::save_instance_store(&store).unwrap();
    let package = root
        .join("previous-sessions.zip")
        .to_string_lossy()
        .into_owned();
    let selected = vec![session_id.clone()];
    let export = export_sessions(selected.clone(), package.clone(), None, None).unwrap();
    assert_eq!(export.exported_session_count, 1);

    let mut store = modules::codex_instance::load_instance_store().unwrap();
    store.instances.clear();
    modules::codex_instance::save_instance_store(&store).unwrap();
    fs::remove_dir_all(source).unwrap();
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("config.toml"), "cli_auth_credentials_store = 'file'\n[analytics]\nenabled = false\n[feedback]\nenabled = false\n").unwrap();
    assert!(!target.join("state_5.sqlite").exists());
    let imported = import_sessions(package, None, selected, HashMap::new(), None, None).unwrap();
    assert_eq!(imported.target_instance_id, DEFAULT_INSTANCE_ID);
    assert_eq!(imported.imported_session_count, 1, "{}", imported.message);
    assert!(
        !imported.message.contains("索引或项目归属更新未完成"),
        "{}",
        imported.message
    );
    modules::codex_session_visibility::repair_session_visibility_quick_for_instance(
        DEFAULT_INSTANCE_ID,
        "Clean install",
        &target,
        true,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(target.join(relative)).unwrap(), original);
    assert!(!target.join("auth.json").exists());
    let db = rusqlite::Connection::open(target.join("state_5.sqlite")).unwrap();
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id=?1",
            [&session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    println!("IMPORTED_FIXTURE_THREAD_ID={session_id}");
}

#[test]
fn isolated_import_rejects_manifest_id_mismatch_without_publishing_file() {
    let base = make_temp_dir("codex-session-import-id-mismatch");
    let (instance, snapshot, mut manifest) = isolated_export_fixture(&base, "actual-id");
    manifest.sessions[0].session_id = "wrong-manifest-id".into();
    let package = base.join("sessions.zip");
    write_session_export_package(&package, &manifest, &[(instance, snapshot)], None, None).unwrap();
    let target = base.join("target/rollout.jsonl");
    let mut archive = ZipArchive::new(File::open(&package).unwrap()).unwrap();
    assert!(
        write_imported_rollout_from_archive(&mut archive, &manifest.sessions[0], &target, None)
            .is_err(),
        "matching hash must not allow manifest/rollout identity mismatch"
    );
    assert!(!target.exists());
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 0);
    drop(archive);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn isolated_import_disk_parent_failure_preserves_existing_file_and_package() {
    let base = make_temp_dir("codex-session-import-parent-failure");
    let (instance, snapshot, manifest) = isolated_export_fixture(&base, "qa-disk-failure");
    let package = base.join("sessions.zip");
    write_session_export_package(&package, &manifest, &[(instance, snapshot)], None, None).unwrap();
    let hash = sha256_file(&package).unwrap();
    let blocker = base.join("target-file");
    fs::write(&blocker, b"keep existing data").unwrap();
    let mut archive = ZipArchive::new(File::open(&package).unwrap()).unwrap();
    assert!(write_imported_rollout_from_archive(
        &mut archive,
        &manifest.sessions[0],
        &blocker.join("rollout.jsonl"),
        None
    )
    .is_err());
    assert_eq!(fs::read(&blocker).unwrap(), b"keep existing data");
    assert_eq!(sha256_file(&package).unwrap(), hash);
    drop(archive);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn isolated_import_index_failure_preserves_published_history_and_concurrent_index_entries() {
    let base = make_temp_dir("codex-session-import-checkpoint");
    let first = base.join("sessions/first.jsonl");
    let second = base.join("sessions/second.jsonl");
    write_rollout(&first, "first", "first successful import");
    let index_path = base.join(SESSION_INDEX_FILE);
    fs::write(&index_path, "{\"id\":\"existing\",\"pinned\":true}\n").unwrap();
    commit_imported_session_index_with(|| {
        upsert_session_index_with_entry(&base, "first", &json!({"id":"first", "thread_name":"First import"}))
    }).unwrap();
    write_rollout(&second, "second", "second published import");
    let held_index = base.join("held-index");
    let mut after_failure = String::new();
    let error = commit_imported_session_index_with(|| {
        // A real filesystem publication failure overlaps another writer's update.
        fs::rename(&index_path, &held_index).unwrap();
        fs::create_dir(&index_path).unwrap();
        let result = upsert_session_index_with_entry(&base, "second", &json!({"id":"second"}));
        assert!(result.is_err());
        fs::remove_dir(&index_path).unwrap();
        fs::rename(&held_index, &index_path).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&index_path).unwrap();
        file.write_all(b"{\"id\":\"concurrent\",\"thread_name\":\"Newer client record\"}\n").unwrap();
        after_failure = fs::read_to_string(&index_path).unwrap();
        result
    }).unwrap_err();
    assert!(error.contains("session_index"));
    assert_eq!(read_session_index_content(&base).unwrap().as_deref(), Some(after_failure.as_str()));
    let index = read_session_index_map(&base).unwrap();
    assert!(index.contains_key("existing"));
    assert!(index.contains_key("first"));
    assert!(index.contains_key("concurrent"));
    assert!(!index.contains_key("second"));
    assert!(first.exists());
    assert!(second.exists(), "published history must remain recoverable after index failure");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn isolated_import_retry_repairs_only_identical_unindexed_history() {
    let base = make_temp_dir("codex-session-import-index-retry");
    let (instance, snapshot, manifest) = isolated_export_fixture(&base, "retry-index");
    let package = base.join("package.zip");
    write_session_export_package(&package, &manifest, &[(instance, snapshot.clone())], None, None).unwrap();
    let mut archive = ZipArchive::new(File::open(&package).unwrap()).unwrap();
    let item = &manifest.sessions[0];
    let root = base.join("target");
    fs::create_dir_all(&root).unwrap();
    let rollout = root.join("rollout-retry-index.jsonl");
    fs::copy(&snapshot.rollout_path, &rollout).unwrap();
    let before = fs::read(&rollout).unwrap();
    assert!(repair_imported_session_index(&root, &mut archive, item, &rollout, None).unwrap());
    assert_eq!(fs::read(&rollout).unwrap(), before);
    assert_eq!(read_session_index_map(&root).unwrap()["retry-index"]["thread_name"], "QA export");
    assert!(!repair_imported_session_index(&root, &mut archive, item, &rollout, None).unwrap());
    fs::remove_file(root.join(SESSION_INDEX_FILE)).unwrap();
    fs::OpenOptions::new().append(true).open(&rollout).unwrap().write_all(b"new client history\n").unwrap();
    assert!(!repair_imported_session_index(&root, &mut archive, item, &rollout, None).unwrap());
    assert!(!root.join(SESSION_INDEX_FILE).exists());
    let mapped = root.join("mapped.jsonl");
    modules::codex_session_import_paths::rewrite_rollout_workspace_paths(&snapshot.rollout_path, &mapped, &item.cwd, "/mapped/project").unwrap();
    assert!(repair_imported_session_index(&root, &mut archive, item, &mapped, Some("/mapped/project")).unwrap());
    assert_eq!(read_session_index_map(&root).unwrap()["retry-index"]["cwd"], "/mapped/project");
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn paginated_session_transfer_keeps_only_the_selected_branch_history() {
    let base = tempfile::tempdir().unwrap();
    let session_id = "3a2cfad6-6e2c-4637-9570-8022128b3c03";
    let source_id = "3a2cfad6-6e2c-4637-9570-8022128b3c01";
    let physical_id = "3a2cfad6-6e2c-4637-9570-8022128b3c02";
    let child_physical = "3a2cfad6-6e2c-4637-9570-8022128b3c04";
    let (instance, mut snapshot, mut manifest) = isolated_export_fixture(base.path(), session_id);
    let source_path = instance.data_dir.join(format!(
        "archived_sessions/rollout-2026-10-11T00-00-00-{source_id}_{physical_id}.jsonl.zst"
    ));
    fs::create_dir_all(source_path.parent().unwrap()).unwrap();
    let line = |ordinal: u64, kind: &str, payload: JsonValue| {
        format!(
            "{}\n",
            json!({
                "timestamp": "2026-10-11T00:00:00Z", "ordinal": ordinal, "type": kind, "payload": payload,
            })
        )
    };
    let message = |ordinal, text: &str| {
        line(
            ordinal,
            "event_msg",
            json!({
                "type": "item_completed", "item": {"type":"UserMessage", "id": "message",
                    "content": [{"type": "text", "text": text}]},
            }),
        )
    };
    let prefix = line(
        0,
        "session_meta",
        json!({"id": source_id, "history_mode": "paginated"}),
    ) + &message(1, "ancestor included 中文🙂");
    let source = prefix.clone() + &message(2, "unselected ancestor tail");
    let compressed = zstd::stream::encode_all(source.as_bytes(), 0).unwrap();
    fs::write(&source_path, &compressed).unwrap();

    fs::remove_file(&snapshot.rollout_path).unwrap();
    snapshot.rollout_path.set_file_name(format!(
        "rollout-2026-10-11T00-01-00-{session_id}_{child_physical}.jsonl"
    ));
    let child = line(
        2,
        "session_meta",
        json!({
            "id": session_id, "cwd": "/tmp/project", "history_mode": "paginated",
            "history_base": {"thread_id": physical_id, "end_byte_offset": prefix.len(), "end_ordinal_exclusive": 2},
        }),
    ) + &message(3, "selected branch");
    fs::write(&snapshot.rollout_path, &child).unwrap();
    let original_child = snapshot.rollout_path.clone();

    let (_staging, prepared) = prepare_export_session_entries(vec![(instance, snapshot)]).unwrap();
    let item = &mut manifest.sessions[0];
    let staged = &prepared[0].1;
    (item.size_bytes, item.sha256) = sha256_file(&staged.rollout_path).unwrap();
    item.relative_rollout_path = snapshot_relative_rollout_path(staged);
    assert!(item
        .relative_rollout_path
        .ends_with(&format!("-{session_id}.jsonl")));
    let package = base.path().join("portable.zip");
    write_session_export_package(&package, &manifest, &prepared, None, None).unwrap();

    let target = base.path().join("different-computer");
    let item = &manifest.sessions[0];
    let destination = resolve_import_target_rollout_path(&target, item);
    let mut archive = ZipArchive::new(File::open(package).unwrap()).unwrap();
    write_imported_rollout_from_archive(&mut archive, item, &destination, Some("/tmp/new-project"))
        .unwrap();
    let imported = fs::read_to_string(destination).unwrap();
    let records = imported
        .lines()
        .map(|line| serde_json::from_str::<JsonValue>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records[0]["payload"]["cwd"], "/tmp/new-project");
    assert_eq!(records[0]["payload"]["history_mode"], "legacy");
    assert!(records[0]["payload"].get("history_base").is_none());
    let messages = records
        .iter()
        .filter(|record| record["payload"]["type"] == "user_message")
        .map(|record| record["payload"]["message"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(messages, ["ancestor included 中文🙂", "selected branch"]);
    assert!(!imported.contains("unselected ancestor tail"));
    assert_eq!(fs::read(source_path).unwrap(), compressed);
    assert_eq!(fs::read_to_string(original_child).unwrap(), child);
}
