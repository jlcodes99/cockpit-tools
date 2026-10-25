#[test]
fn restore_index_failure_keeps_durable_target_and_trash_for_retry() {
    let base = make_temp_dir("codex-session-restore-index-retry");
    let root = base.join("codex-home");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("sessions/2026/10/24/rollout-session-1.jsonl");
    let entry = make_trash_entry(&base, "session-1", target.clone());
    let source = fs::read(&entry.trashed_rollout_path).unwrap();
    fs::create_dir(root.join(SESSION_INDEX_FILE)).unwrap();
    assert!(restore_trashed_session_entry_with_metadata_rebuild(&entry, false).is_err());
    assert_eq!(fs::read(&target).unwrap(), source);
    assert_eq!(fs::read(&entry.trashed_rollout_path).unwrap(), source);
    fs::remove_dir(root.join(SESSION_INDEX_FILE)).unwrap();
    fs::write(
        root.join(SESSION_INDEX_FILE),
        b"{\"id\":\"other\",\"thread_name\":\"Concurrent record\"}\n",
    )
    .unwrap();
    restore_trashed_session_entry_with_metadata_rebuild(&entry, false).unwrap();
    assert_eq!(fs::read(&target).unwrap(), source);
    assert!(read_session_index_map(&root).unwrap().contains_key("other"));
    assert!(!entry.entry_dir.exists());
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn restored_session_index_is_append_only_and_uses_the_latest_existing_record() {
    let root = make_temp_dir("codex-session-restore-index-append");
    let before = b"{ \"id\":\"session\",\"pinned\":false }\r\n{\"id\":\"other\",\"thread_name\":\"Keep me\"}\n{\"id\":\"session\",\"pinned\":true}\n{unfinished old tail";
    fs::write(root.join(SESSION_INDEX_FILE), before).unwrap();
    upsert_session_index_with_entry(
        &root,
        "session",
        &json!({"id":"session", "thread_name":"Restored"}),
    )
    .unwrap();
    assert!(fs::read(root.join(SESSION_INDEX_FILE))
        .unwrap()
        .starts_with(before));
    let index = read_session_index_map(&root).unwrap();
    assert_eq!(index["session"]["pinned"], true);
    assert_eq!(index["session"]["thread_name"], "Restored");
    assert_eq!(index["other"]["thread_name"], "Keep me");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restored_session_index_concurrent_appends_keep_every_entry() {
    let root = make_temp_dir("codex-session-restore-index-concurrent");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers = (0..8)
        .map(|index| {
            let root = root.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let id = format!("session-{index}");
                upsert_session_index_with_entry(&root, &id, &json!({"id":id,"thread_name":index}))
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    let index = read_session_index_map(&root).unwrap();
    for number in 0..8 {
        assert_eq!(index[&format!("session-{number}")]["thread_name"], number);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn official_fork_dependency_refusal_preserves_parent_child_and_index() {
    let base = make_temp_dir("codex-session-trash-fork-dependency");
    let root = base.join("profile");
    let parent = root.join("sessions/rollout-parent.jsonl");
    let child = root.join("sessions/rollout-child.jsonl");
    write_rollout(&parent, "parent", "original history");
    fs::write(&child, b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"history_base\":{\"thread_id\":\"parent\",\"end_ordinal\":1,\"end_byte_offset\":100}}}\n").unwrap();
    let index = b"{\"id\":\"parent\",\"thread_name\":\"Parent\"}\n{\"id\":\"child\",\"thread_name\":\"Child\"}\n";
    fs::write(root.join(SESSION_INDEX_FILE), index).unwrap();
    let instance = CodexSyncInstance {
        id: "default".into(),
        name: "Fixture".into(),
        data_dir: root.clone(),
        last_pid: None,
    };
    let snapshot = ThreadSnapshot {
        id: "parent".into(),
        title: "Parent".into(),
        cwd: "/tmp".into(),
        project_name: None,
        updated_at: None,
        rollout_path: parent.clone(),
        session_index_entry: json!({"id":"parent"}),
        source_root: root.clone(),
    };
    let parent_bytes = fs::read(&parent).unwrap();
    let child_bytes = fs::read(&child).unwrap();
    let result = trash_snapshots_for_instance_with_app_server(
        &instance,
        &base.join("trash"),
        &[snapshot],
        |_, _| Err("cannot delete thread parent: forked history still references it".into()),
        |_| panic!("dependency refusal must never rebuild or remove history"),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&parent).unwrap(), parent_bytes);
    assert_eq!(fs::read(&child).unwrap(), child_bytes);
    assert_eq!(fs::read(root.join(SESSION_INDEX_FILE)).unwrap(), index);
    fs::remove_dir_all(base).unwrap();
}
