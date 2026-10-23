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
    upsert_session_index_with_entry(&target_root, &None, "qa-roundtrip", &index).unwrap();
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
        |_, _| Err("synthetic offline app-server".into()),
        |_| Ok(()),
    )
    .unwrap();
    assert!(outcome.official_delete_failed);
    assert!(!outcome.metadata_rebuild_failed);
    assert!(!target_path.exists());
    assert!(!read_session_index_map(&target_root)
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
fn isolated_import_second_index_failure_keeps_first_commit_and_removes_only_second_rollout() {
    let base = make_temp_dir("codex-session-import-checkpoint");
    let first = base.join("sessions/first.jsonl");
    let second = base.join("sessions/second.jsonl");
    write_rollout(&first, "first", "first successful import");
    let initial = Some("{\"id\":\"existing\",\"pinned\":true}\n".to_owned());
    restore_session_index_content(&base, initial.as_deref()).unwrap();
    let first_entry = json!({"id":"first", "thread_name":"First import"});
    commit_imported_session_index_with(&base, &initial, &first, || {
        upsert_session_index_with_entry(&base, &initial, "first", &first_entry)
    })
    .unwrap();
    let checkpoint = read_session_index_content(&base).unwrap();
    write_rollout(&second, "second", "second staged import");
    let index_path = base.join(SESSION_INDEX_FILE);
    let error = commit_imported_session_index_with(&base, &checkpoint, &second, || {
        // Inject a real, transient filesystem failure after the first commit:
        // a directory occupies the index filename during atomic publication.
        fs::remove_file(&index_path).unwrap();
        fs::create_dir(&index_path).unwrap();
        let result =
            upsert_session_index_with_entry(&base, &checkpoint, "second", &json!({"id":"second"}));
        assert!(result.is_err());
        fs::remove_dir(&index_path).unwrap();
        result
    })
    .unwrap_err();
    assert!(error.contains("写入 session_index.jsonl 失败"));
    assert_eq!(read_session_index_content(&base).unwrap(), checkpoint);
    let index = read_session_index_map(&base).unwrap();
    assert!(index.contains_key("existing"));
    assert!(index.contains_key("first"));
    assert!(!index.contains_key("second"));
    assert!(first.exists());
    assert!(!second.exists());
    fs::remove_dir_all(base).unwrap();
}
