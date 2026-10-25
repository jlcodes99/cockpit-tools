#[test]
fn compressed_session_heads_are_listed_once_and_state_selects_current_rollout() {
    let base = make_temp_dir("codex-session-compressed-list");
    let (instance, mut snapshot, _) = isolated_export_fixture(&base, "compressed-head");
    let plain = fs::read(&snapshot.rollout_path).unwrap();
    let compressed = snapshot.rollout_path.with_extension("jsonl.zst");
    fs::write(
        &compressed,
        zstd::stream::encode_all(plain.as_slice(), 0).unwrap(),
    )
    .unwrap();
    fs::remove_file(&snapshot.rollout_path).unwrap();
    snapshot.rollout_path = compressed.clone();
    assert_eq!(
        read_rollout_session_meta(&compressed).unwrap().unwrap()["payload"]["id"],
        "compressed-head"
    );
    assert!(rollout_conversation_contains_query_uncached(&compressed, "round trip").unwrap());
    let listed = load_thread_snapshots(&instance).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].rollout_path, compressed);

    let ancestor = instance
        .data_dir
        .join("archived_sessions/rollout-old.jsonl.zst");
    fs::create_dir_all(ancestor.parent().unwrap()).unwrap();
    fs::copy(&compressed, &ancestor).unwrap();
    let database = rusqlite::Connection::open(instance.data_dir.join("state_5.sqlite")).unwrap();
    database
        .execute_batch("CREATE TABLE threads(id TEXT, rollout_path TEXT)")
        .unwrap();
    database
        .execute(
            "INSERT INTO threads VALUES (?1,?2)",
            rusqlite::params![
                "compressed-head",
                compressed.with_extension("").to_string_lossy()
            ],
        )
        .unwrap();
    let listed = load_thread_snapshots(&instance).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].rollout_path, compressed);
    // Official materialization may briefly leave both representations present.
    fs::write(compressed.with_extension(""), &plain).unwrap();
    let listed = load_thread_snapshots(&instance).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].rollout_path, compressed.with_extension(""));
    drop(database);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn compressed_trash_restore_validates_tmp_magic_and_preserves_encoded_bytes() {
    let base = make_temp_dir("codex-session-compressed-restore");
    let target = base.join("profile/sessions/2026/10/24/rollout-compressed.jsonl.zst");
    let mut entry = make_trash_entry(&base, "compressed", target.clone());
    let plain = fs::read(&entry.trashed_rollout_path).unwrap();
    let encoded = zstd::stream::encode_all(plain.as_slice(), 0).unwrap();
    let source = entry.trashed_rollout_path.with_extension("jsonl.zst");
    fs::write(&source, &encoded).unwrap();
    fs::remove_file(&entry.trashed_rollout_path).unwrap();
    entry.trashed_rollout_path = source;
    restore_trashed_session_entry_with_metadata_rebuild(&entry, false).unwrap();
    assert_eq!(fs::read(&target).unwrap(), encoded);
    assert_eq!(
        rollout_session_id(&target).unwrap().as_deref(),
        Some("compressed")
    );
    assert!(!entry.entry_dir.exists());
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn compressed_standalone_export_import_roundtrip_uses_decoded_hash_and_plain_entry() {
    let base = make_temp_dir("codex-session-compressed-export");
    let (instance, mut snapshot, manifest) = isolated_export_fixture(&base, "compressed-export");
    let original = fs::read(&snapshot.rollout_path).unwrap();
    let encoded = zstd::stream::encode_all(original.as_slice(), 0).unwrap();
    let compressed = snapshot.rollout_path.with_extension("jsonl.zst");
    fs::write(&compressed, &encoded).unwrap();
    fs::remove_file(&snapshot.rollout_path).unwrap();
    snapshot.rollout_path = compressed.clone();
    let (_staging, entries) = prepare_export_session_entries(vec![(instance, snapshot)]).unwrap();
    assert_eq!(fs::read(&entries[0].1.rollout_path).unwrap(), original);
    assert!(snapshot_relative_rollout_path(&entries[0].1).ends_with(".jsonl"));
    assert_eq!(
        sha256_file(&entries[0].1.rollout_path).unwrap(),
        (
            manifest.sessions[0].size_bytes,
            manifest.sessions[0].sha256.clone()
        )
    );
    let package = base.join("package.zip");
    write_session_export_package(&package, &manifest, &entries, None, None).unwrap();
    let mut archive = ZipArchive::new(File::open(package).unwrap()).unwrap();
    let imported = base.join("target/rollout-imported.jsonl");
    write_imported_rollout_from_archive(&mut archive, &manifest.sessions[0], &imported, None)
        .unwrap();
    assert_eq!(fs::read(imported).unwrap(), original);
    assert_eq!(fs::read(compressed).unwrap(), encoded);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn compressed_metadata_is_bounded_and_export_rejects_missing_history_chain() {
    let base = make_temp_dir("codex-session-compressed-boundaries");
    let too_large = base.join("large.tmp");
    let bytes = vec![b' '; 1024 * 1024 + 2];
    fs::write(
        &too_large,
        zstd::stream::encode_all(bytes.as_slice(), 0).unwrap(),
    )
    .unwrap();
    assert!(read_rollout_session_meta(&too_large).is_err());
    let (instance, mut snapshot, _) = isolated_export_fixture(&base, "chain");
    let bytes = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"chain\",\"history_base\":{\"thread_id\":\"parent\"}}}\n";
    let compressed = snapshot.rollout_path.with_extension("jsonl.zst");
    let encoded = zstd::stream::encode_all(bytes.as_slice(), 0).unwrap();
    fs::write(&compressed, &encoded).unwrap();
    snapshot.rollout_path = compressed.clone();
    assert!(prepare_export_session_entries(vec![(instance, snapshot)])
        .unwrap_err()
        .contains("分页会话"));
    assert_eq!(fs::read(compressed).unwrap(), encoded);
    fs::remove_dir_all(base).unwrap();
}
