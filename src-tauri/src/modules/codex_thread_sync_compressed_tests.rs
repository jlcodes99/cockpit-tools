use super::*;

fn standalone() -> Vec<u8> {
    " {\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"cwd\":\"/synthetic/project\",\"model_provider\":\"供应商🙂\"}}\r\n{\"type\":\"event_msg\",\"timestamp\":\"2026-10-24T08:00:00Z\",\"payload\":{\"type\":\"user_message\",\"message\":\"原始字节\"}}\r\n".as_bytes().to_vec()
}

fn compressed(path: &Path, bytes: &[u8]) -> Vec<u8> {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let encoded = zstd::stream::encode_all(bytes, 1).unwrap();
    fs::write(path, &encoded).unwrap();
    encoded
}

fn snapshot(path: &Path, root: &Path) -> ThreadSnapshot {
    ThreadSnapshot {
        id: "s1".into(),
        rollout_path: path.into(),
        rollout_actual_modified_at: None,
        rollout_modified_at: None,
        merged_rollout_content: None,
        session_index_entry: json!({"id":"s1","thread_name":"Synthetic"}),
        workspace_root: None,
        source_root: root.into(),
        freshness: ThreadFreshness {
            activity_ms: 0,
            rollout_len: 0,
            rollout_modified_ms: 0,
        },
    }
}

fn instance(root: &Path) -> CodexSyncInstance {
    CodexSyncInstance {
        id: "synthetic-compressed-sync".into(),
        name: "Synthetic".into(),
        data_dir: root.into(),
        last_pid: None,
    }
}

#[test]
fn compressed_snapshot_is_discovered_with_real_metadata_and_decoded_freshness() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sessions/rollout-current.jsonl.zst");
    let bytes = standalone();
    let raw = compressed(&source, &bytes);
    let found = list_rollout_files(&root.path().join("sessions")).unwrap();
    assert_eq!(found, [source.clone()]);
    let meta = read_rollout_session_meta(&source).unwrap().unwrap();
    assert_eq!(session_meta_id(&meta).as_deref(), Some("s1"));
    let snapshots = load_thread_snapshots(&instance(root.path())).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].freshness.rollout_len, bytes.len() as u64);
    assert_eq!(snapshots[0].freshness.activity_ms, 1792828800000);
    assert_eq!(rollout_snapshot_bytes(&snapshots[0]).unwrap(), bytes);
    assert_eq!(fs::read(source).unwrap(), raw);
}

#[test]
fn compressed_standalone_sync_materializes_plain_extension_without_changing_source() {
    let root = tempfile::tempdir().unwrap();
    let source_root = root.path().join("source");
    let target_root = root.path().join("target");
    fs::create_dir_all(&target_root).unwrap();
    let source = source_root.join("sessions/2026/10/24/rollout-current.jsonl.zst");
    let bytes = standalone();
    let raw = compressed(&source, &bytes);
    let snapshot = snapshot(&source, &source_root);
    let plan = [ThreadSyncPlanItem {
        snapshot: snapshot.clone(),
        existing_rollout_path: None,
        is_update: false,
    }];
    let result =
        sync_thread_plan_to_instance_with_rebuild(&instance(&target_root), &plan, &[], |_| true)
            .unwrap();
    let target = target_root.join("sessions/2026/10/24/rollout-current.jsonl");
    assert_eq!(fs::read(&target).unwrap(), bytes);
    assert!(!target.with_extension("jsonl.zst").exists());
    assert_eq!(fs::read(&source).unwrap(), raw);
    assert!(result.backup_dir.exists());
    assert!(fs::read_to_string(target_root.join(SESSION_INDEX_FILE))
        .unwrap()
        .contains("s1"));
    copy_rollout_file_to_path(&snapshot, &target).unwrap();
    assert_eq!(fs::read(target).unwrap(), bytes);
}

#[test]
fn mixed_compressed_and_plain_prefix_merge_retains_verbatim_decoded_bytes() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("rollout-a.jsonl.zst");
    let second = root.path().join("rollout-b.jsonl");
    let prefix = standalone();
    let mut full = prefix.clone();
    full.extend_from_slice(b"{\"type\":\"event_msg\"}\r\n");
    compressed(&first, &prefix);
    fs::write(&second, &full).unwrap();
    let snapshots = [
        snapshot(&first, root.path()),
        snapshot(&second, root.path()),
    ];
    let merged = merge_thread_snapshots(&snapshots).unwrap();
    assert_eq!(rollout_snapshot_bytes(&merged).unwrap(), full);
    assert_eq!(
        resolve_target_rollout_path(&merged, &root.path().join("target"), None)
            .unwrap()
            .extension()
            .unwrap(),
        "jsonl"
    );
    assert!(snapshot_rollout_matches(&snapshots[1], &merged));
    fs::write(&second, b"different history").unwrap();
    assert!(merge_thread_snapshots(&snapshots).is_err());
}

#[test]
fn compressed_existing_target_is_only_reused_when_decoded_bytes_match() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source/rollout-a.jsonl.zst");
    let target = root.path().join("target/rollout-a.jsonl.zst");
    let bytes = standalone();
    compressed(&source, &bytes);
    let raw = compressed(&target, &bytes);
    let snap = snapshot(&source, &root.path().join("source"));
    assert!(matches!(
        validate_rollout_copy_target(&snap, &target, &bytes).unwrap(),
        RolloutCopyAction::Unchanged
    ));
    let mut extended = bytes.clone();
    extended.extend_from_slice(b"{\"type\":\"event_msg\"}\n");
    assert!(validate_rollout_copy_target(&snap, &target, &extended).is_err());
    assert_eq!(fs::read(&target).unwrap(), raw);
    let misnamed = root.path().join("target/rollout-magic.jsonl");
    fs::write(&misnamed, &raw).unwrap();
    assert!(validate_rollout_copy_target(&snap, &misnamed, &extended).is_err());
    assert_eq!(fs::read(misnamed).unwrap(), raw);
}

#[test]
fn compressed_source_can_append_verbatim_to_stopped_plain_target() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.jsonl.zst");
    let target = root.path().join("target.jsonl");
    let prefix = standalone();
    let mut full = prefix.clone();
    full.extend_from_slice(b"{\"type\":\"event_msg\"}\r\n");
    compressed(&source, &full);
    fs::write(&target, &prefix).unwrap();
    let snap = snapshot(&source, root.path());
    let bytes = rollout_snapshot_bytes(&snap).unwrap();
    let action = validate_rollout_copy_target(&snap, &target, &bytes).unwrap();
    assert!(matches!(action, RolloutCopyAction::Append(_)));
    copy_prepared_rollout(&snap, &target, &bytes, action, || Ok(())).unwrap();
    assert_eq!(fs::read(target).unwrap(), full);
}

#[test]
fn compressed_paginated_or_truncated_input_never_publishes() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.jsonl.zst");
    let target = root.path().join("target.jsonl");
    let mut paginated = standalone();
    paginated.extend_from_slice(b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"history_base\":{\"thread_id\":\"parent\",\"end_byte_offset\":1}}}\r\n");
    compressed(&source, &paginated);
    assert!(copy_rollout_file_to_path(&snapshot(&source, root.path()), &target).is_err());
    assert!(!target.exists());
    let mut broken = compressed(&source, &standalone());
    broken.truncate(broken.len() - 3);
    fs::write(&source, broken).unwrap();
    assert!(copy_rollout_file_to_path(&snapshot(&source, root.path()), &target).is_err());
    assert!(!target.exists());
}

#[test]
fn current_head_catalogue_ignores_old_same_id_segments_and_accepts_zst_storage_suffix() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("sessions/rollout-old.jsonl");
    let current = root.path().join("sessions/rollout-new.jsonl.zst");
    compressed(&current, &standalone());
    fs::write(&old, standalone()).unwrap();
    let db = rusqlite::Connection::open(root.path().join("state_5.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE threads(id TEXT, rollout_path TEXT)")
        .unwrap();
    db.execute(
        "INSERT INTO threads VALUES (?1,?2)",
        rusqlite::params!["s1", "sessions/rollout-new.jsonl"],
    )
    .unwrap();
    let snapshots = load_thread_snapshots(&instance(root.path())).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].rollout_path, current);
}

#[test]
fn compressed_alias_and_concurrent_plain_creation_are_never_overwritten() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.jsonl.zst");
    let target = root.path().join("target.jsonl");
    let bytes = standalone();
    compressed(&source, &bytes);
    let snap = snapshot(&source, root.path());
    let action = validate_rollout_copy_target(&snap, &target, &bytes).unwrap();
    fs::write(&target, b"concurrent writer").unwrap();
    assert!(copy_prepared_rollout(&snap, &target, &bytes, action, || Ok(())).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"concurrent writer");
    fs::remove_file(&target).unwrap();
    let action = validate_rollout_copy_target(&snap, &target, &bytes).unwrap();
    let alias = alternate_rollout_path(&target);
    let raw = compressed(&alias, &bytes);
    assert!(copy_prepared_rollout(&snap, &target, &bytes, action, || Ok(())).is_err());
    assert!(!target.exists());
    assert_eq!(fs::read(&alias).unwrap(), raw);
    assert!(validate_rollout_copy_target(&snap, &target, &bytes).is_err());
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".cockpit-rollout-copy-")));
}

#[test]
fn zstd_magic_is_decoded_even_when_the_source_is_named_jsonl() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.jsonl");
    let target = root.path().join("target.jsonl");
    let bytes = standalone();
    let raw = compressed(&source, &bytes);
    copy_rollout_file_to_path(&snapshot(&source, root.path()), &target).unwrap();
    assert_eq!(fs::read(target).unwrap(), bytes);
    assert_eq!(fs::read(source).unwrap(), raw);
}
