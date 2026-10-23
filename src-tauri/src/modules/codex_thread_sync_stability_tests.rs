use super::*;

struct AppendFixture {
    root: PathBuf,
    source: PathBuf,
    target: PathBuf,
    prefix: Vec<u8>,
    full: Vec<u8>,
}

impl AppendFixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("thread-sync-append-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("source/sessions")).unwrap();
        fs::create_dir_all(root.join("target/sessions")).unwrap();
        let source = root.join("source/sessions/rollout-s1.jsonl");
        let target = root.join("target/sessions/rollout-s1.jsonl");
        let prefix = "{ \"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"中文🙂\"}}\r\n".as_bytes().to_vec();
        let mut full = prefix.clone();
        full.extend_from_slice("{\"type\":\"event\",\"text\":\"新增对话🙂\"}\r\n".as_bytes());
        fs::write(&source, &full).unwrap();
        fs::write(&target, &prefix).unwrap();
        Self {
            root,
            source,
            target,
            prefix,
            full,
        }
    }

    fn snapshot(&self) -> ThreadSnapshot {
        snapshot(&self.source, &self.root.join("source"))
    }

    fn action(&self) -> RolloutCopyAction {
        validate_rollout_copy_target(&self.snapshot(), &self.target, &self.full).unwrap()
    }
}

impl Drop for AppendFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn stopped_sync_appends_complete_prefix_verbatim_and_preserves_original_backup() {
    let fixture = AppendFixture::new();
    let target = CodexSyncInstance {
        id: "synthetic-target".into(),
        name: "Synthetic target".into(),
        data_dir: fixture.root.join("target"),
        last_pid: None,
    };
    let source = CodexSyncInstance {
        id: "synthetic-source".into(),
        name: "Synthetic source".into(),
        data_dir: fixture.root.join("source"),
        last_pid: None,
    };
    let plans = [ThreadSyncPlanItem {
        snapshot: fixture.snapshot(),
        existing_rollout_path: Some(fixture.target.clone()),
        is_update: true,
    }];
    let result = run_thread_sync_when_stopped(&[source, target.clone()], &[], || {
        // Only the external app-server rebuild is replaced; the stopped gate,
        // preparation, backup, real append and index persistence all execute.
        sync_thread_plan_to_instance_with_rebuild(&target, &plans, &[], |_| true)
    })
    .unwrap()
    .unwrap();
    assert_eq!(fs::read(&fixture.target).unwrap(), fixture.full);
    assert_eq!(
        &fs::read(&fixture.target).unwrap()[..fixture.prefix.len()],
        fixture.prefix
    );
    assert_eq!(
        fs::read(result.backup_dir.join("rollouts/sessions/rollout-s1.jsonl")).unwrap(),
        fixture.prefix
    );
    assert!(fs::read_to_string(target.data_dir.join(SESSION_INDEX_FILE))
        .unwrap()
        .contains("s1"));
    assert!(matches!(fixture.action(), RolloutCopyAction::Unchanged));
    fs::remove_dir_all(result.backup_dir).unwrap();
}

#[test]
fn interrupted_append_resumes_partial_source_bytes_without_duplicate_or_truncation() {
    let fixture = AppendFixture::new();
    let interrupted = fixture.prefix.len() + 11;
    // Simulate an interrupted append halfway through a record, on fixture files.
    fs::write(&fixture.target, &fixture.full[..interrupted]).unwrap();
    copy_prepared_rollout(
        &fixture.snapshot(),
        &fixture.target,
        &fixture.full,
        fixture.action(),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(fs::read(&fixture.target).unwrap(), fixture.full);
    assert_eq!(
        &fs::read(&fixture.target).unwrap()[..interrupted],
        &fixture.full[..interrupted]
    );
    assert!(matches!(fixture.action(), RolloutCopyAction::Unchanged));
}

#[test]
fn append_rejects_target_changes_after_preparation_and_during_process_recheck() {
    use std::io::Write;
    for during_recheck in [false, true] {
        let fixture = AppendFixture::new();
        let action = fixture.action();
        let changed = || {
            fs::OpenOptions::new()
                .append(true)
                .open(&fixture.target)
                .unwrap()
                .write_all(b"{\"type\":\"event\",\"text\":\"target edit\"}\r\n")
                .unwrap();
        };
        if !during_recheck {
            changed();
        }
        let checks = std::cell::Cell::new(0);
        let result = copy_prepared_rollout(
            &fixture.snapshot(),
            &fixture.target,
            &fixture.full,
            action,
            || {
                checks.set(checks.get() + 1);
                if during_recheck && checks.get() == 2 {
                    changed();
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        let mut expected = fixture.prefix.clone();
        expected.extend_from_slice(b"{\"type\":\"event\",\"text\":\"target edit\"}\r\n");
        assert_eq!(fs::read(&fixture.target).unwrap(), expected);
    }
}

#[test]
fn running_target_rejects_append_and_default_process_is_detected_without_codex_home() {
    let fixture = AppendFixture::new();
    assert!(copy_prepared_rollout(
        &fixture.snapshot(),
        &fixture.target,
        &fixture.full,
        fixture.action(),
        || { Err("synthetic running target".into()) }
    )
    .is_err());
    assert_eq!(fs::read(&fixture.target).unwrap(), fixture.prefix);
    let default = CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.into(),
        name: "Synthetic default".into(),
        data_dir: fixture.root.join("default"),
        last_pid: None,
    };
    let managed = CodexSyncInstance {
        id: "synthetic-target".into(),
        name: "Synthetic target".into(),
        data_dir: fixture.root.join("target"),
        last_pid: None,
    };
    let entries = [(std::process::id(), None)];
    assert!(is_instance_running(&default, &entries));
    assert!(!is_instance_running(&managed, &entries));
    let result: Option<()> = run_thread_sync_when_stopped(&[default, managed], &entries, || {
        panic!("running default must not dispatch synchronization");
    })
    .unwrap();
    assert!(result.is_none());
}

#[test]
fn merged_source_same_as_target_path_still_appends_longer_verbatim_snapshot() {
    let fixture = AppendFixture::new();
    let mut target_snapshot = snapshot(&fixture.target, &fixture.root.join("target"));
    target_snapshot.merged_rollout_content = Some(String::from_utf8(fixture.full.clone()).unwrap());
    let bytes = rollout_snapshot_bytes(&target_snapshot).unwrap();
    let action = validate_rollout_copy_target(&target_snapshot, &fixture.target, &bytes).unwrap();
    copy_prepared_rollout(&target_snapshot, &fixture.target, &bytes, action, || Ok(())).unwrap();
    assert_eq!(fs::read(&fixture.target).unwrap(), fixture.full);
}

#[test]
fn competing_rollout_lock_rejects_append_without_writing_bytes() {
    let fixture = AppendFixture::new();
    let locked = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&fixture.target)
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&locked).unwrap();
    assert!(copy_prepared_rollout(
        &fixture.snapshot(),
        &fixture.target,
        &fixture.full,
        fixture.action(),
        || Ok(())
    )
    .is_err());
    assert_eq!(fs::read(&fixture.target).unwrap(), fixture.prefix);
}

#[test]
fn paginated_append_and_target_fork_or_newer_content_remain_rejected() {
    let fixture = AppendFixture::new();
    let mut paginated = fixture.prefix.clone();
    paginated.extend_from_slice(b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"history_base\":{\"thread_id\":\"parent\",\"end_byte_offset\":178}}}\r\n");
    assert!(
        validate_rollout_copy_target(&fixture.snapshot(), &fixture.target, &paginated).is_err()
    );
    for extra in [b"different target edit".as_slice(), fixture.full.as_slice()] {
        let mut changed = fixture.prefix.clone();
        changed.extend_from_slice(extra);
        fs::write(&fixture.target, &changed).unwrap();
        assert!(
            validate_rollout_copy_target(&fixture.snapshot(), &fixture.target, &fixture.full)
                .is_err()
        );
        assert_eq!(fs::read(&fixture.target).unwrap(), changed);
    }
}

#[cfg(unix)]
#[test]
fn replacing_target_during_process_recheck_never_writes_detached_or_new_file() {
    let fixture = AppendFixture::new();
    let checks = std::cell::Cell::new(0);
    let detached = fixture.root.join("original.jsonl");
    let result = copy_prepared_rollout(
        &fixture.snapshot(),
        &fixture.target,
        &fixture.full,
        fixture.action(),
        || {
            checks.set(checks.get() + 1);
            if checks.get() == 2 {
                fs::rename(&fixture.target, &detached).unwrap();
                fs::write(&fixture.target, b"new target content").unwrap();
            }
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(fs::read(&fixture.target).unwrap(), b"new target content");
    assert_eq!(fs::read(detached).unwrap(), fixture.prefix);
}

fn snapshot(path: &Path, root: &Path) -> ThreadSnapshot {
    ThreadSnapshot {
        id: "s1".into(),
        rollout_path: path.into(),
        rollout_actual_modified_at: None,
        rollout_modified_at: None,
        merged_rollout_content: None,
        session_index_entry: json!({"id":"s1"}),
        workspace_root: None,
        source_root: root.into(),
        freshness: ThreadFreshness {
            activity_ms: 0,
            rollout_len: 0,
            rollout_modified_ms: 0,
        },
    }
}

#[test]
fn stability_merge_selects_verbatim_prefix_and_rejects_forks() {
    let root = std::env::temp_dir().join(format!("merge-prefix-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let a = root.join("a.jsonl");
    let b = root.join("b.jsonl");
    let prefix = "{ \"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"长供应商🙂\"}}\r\n";
    let long = format!("{prefix}{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"s1\",\"history_base\":{{\"thread_id\":\"parent\",\"end_byte_offset\":178}}}}}}\r\n");
    fs::write(&a, prefix).unwrap();
    fs::write(&b, &long).unwrap();
    let snapshots = [snapshot(&a, &root), snapshot(&b, &root)];
    assert_eq!(merge_rollout_contents(&snapshots).unwrap(), long);
    assert_eq!(merge_rollout_contents(&snapshots).unwrap(), long);
    fs::write(&a, format!("{prefix}{{\"type\":\"fork\"}}\r\n")).unwrap();
    assert!(merge_rollout_contents(&snapshots).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stability_copy_preserves_bytes_and_never_overwrites_concurrent_append() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("copy-append-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.jsonl");
    let target = root.join("target.jsonl");
    let original = "{ \"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old\"}}\r\n{\"type\":\"event\",\"text\":\"中文🙂\"}\r\n";
    fs::write(&source, original).unwrap();
    let mut snapshot = snapshot(&source, &root);
    copy_rollout_file_to_path(&snapshot, &target).unwrap();
    copy_rollout_file_to_path(&snapshot, &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), original.as_bytes());
    snapshot.merged_rollout_content = Some(original.into());
    let mut appender = fs::OpenOptions::new().append(true).open(&target).unwrap();
    appender.write_all(b"{\"type\":\"event\"}\r\n").unwrap();
    let appended = fs::read(&target).unwrap();
    assert!(copy_rollout_file_to_path(&snapshot, &target).is_err());
    assert_eq!(fs::read(&target).unwrap(), appended);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stability_partial_paginated_copy_is_rejected_without_publishing() {
    let root = std::env::temp_dir().join(format!("copy-lineage-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.jsonl");
    let target = root.join("target.jsonl");
    let content = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"history_base\":{\"thread_id\":\"parent\",\"end_byte_offset\":178}}}\r\n";
    fs::write(&source, content).unwrap();
    assert!(copy_rollout_file_to_path(&snapshot(&source, &root), &target).is_err());
    assert!(!target.exists());
    assert_eq!(fs::read(&source).unwrap(), content);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stability_batch_rejects_unsafe_child_before_publishing_ordinary_session() {
    let root = std::env::temp_dir().join(format!("batch-lineage-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    let ordinary = source.join("ordinary.jsonl");
    let child = source.join("child.jsonl");
    fs::write(
        &ordinary,
        b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\"}}\r\n",
    )
    .unwrap();
    fs::write(&child, b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"history_base\":{\"thread_id\":\"parent\",\"end_byte_offset\":178}}}\r\n").unwrap();
    let target = CodexSyncInstance {
        id: "test".into(),
        name: "Test".into(),
        data_dir: root.join("target"),
        last_pid: None,
    };
    let plans = [&ordinary, &child]
        .into_iter()
        .map(|path| ThreadSyncPlanItem {
            snapshot: snapshot(path, &source),
            existing_rollout_path: None,
            is_update: false,
        })
        .collect::<Vec<_>>();
    assert!(sync_thread_plan_to_instance(&target, &plans, &[]).is_err());
    assert!(!target.data_dir.exists());
    fs::remove_dir_all(root).unwrap();
}
