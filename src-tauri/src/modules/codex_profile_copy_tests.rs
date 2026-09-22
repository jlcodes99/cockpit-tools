use super::*;
use serde_json::json;

const ROOT: &str = "11111111-1111-4111-8111-111111111111";
const CHILD: &str = "22222222-2222-4222-8222-222222222222";
const LEAF: &str = "33333333-3333-4333-8333-333333333333";

struct Fixture {
    dir: PathBuf,
    source: PathBuf,
    target: PathBuf,
    root_rollout: PathBuf,
    child_rollout: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir =
            std::env::temp_dir().join(format!("codex-profile-copy-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let source = dir.join("source");
        let target = dir.join("target");
        fs::create_dir_all(source.join("sessions/2026/01/01")).unwrap();
        let root_rollout = source.join(format!(
            "sessions/2026/01/01/rollout-2026-01-01T00-00-00-{ROOT}.jsonl"
        ));
        let child_rollout = source.join(format!(
            "sessions/2026/01/01/rollout-2026-01-01T00-01-00-{ROOT}_{CHILD}.jsonl"
        ));
        // Whitespace and non-ASCII content exercise byte preservation, not just parsed JSON equality.
        let root = format!("{{ \"type\": \"session_meta\", \"payload\": {{\"id\":\"{ROOT}\",\"history_mode\":\"paginated\"}} }}\n{{\"type\":\"event_msg\",\"payload\":{{\"text\":\"合成数据\"}}}}\n");
        fs::write(&root_rollout, &root).unwrap();
        fs::write(&child_rollout, format!("{}\n{{\"type\":\"event_msg\"}}\n", json!({
            "type": "session_meta", "payload": {"id": ROOT, "history_mode": "paginated", "history_base": {
                "thread_id": ROOT, "end_byte_offset": root.len(), "end_ordinal_exclusive": 2
            }}
        }))).unwrap();
        let db = Connection::open(source.join(STATE_DB)).unwrap();
        db.execute_batch("CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT); CREATE TABLE project_roots (project_id TEXT, path TEXT); CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, project_id TEXT, cwd TEXT); INSERT INTO projects VALUES ('project-new', 'Synthetic project');").unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, NULL, ?3)",
            rusqlite::params![ROOT, child_rollout.to_str().unwrap(), "/workspace/shared"],
        )
        .unwrap();
        let state = json!({
            "local-projects": {"project-old": {"name": "Synthetic project", "rootPaths": []}},
            "thread-project-assignments": {ROOT: {"projectId": "project-old", "projectKind": "local"}},
            HOST_MAPS[0]: {format!("local:{}", source.display()): {"project-old": "project-new"}, "remote:keep": {"other": "unchanged"}},
            HOST_MAPS[1]: {format!("local:{}", source.display()): {"projectsMigrated": true, "threadAssignmentsMigrated": false}},
            HOST_MAPS[2]: {format!("local:{}", source.display()): [ROOT]},
            "unrelated-setting": true
        });
        fs::write(
            source.join(GLOBAL_STATE),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        Self {
            dir,
            source,
            target,
            root_rollout,
            child_rollout,
        }
    }

    fn target_path(&self, path: &Path) -> PathBuf {
        self.target.join(path.strip_prefix(&self.source).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn copied_instance_uses_its_own_history_after_source_grows() {
    let f = Fixture::new();
    let original_root = fs::read(&f.root_rollout).unwrap();
    let original_child = fs::read(&f.child_rollout).unwrap();
    let source_state = fs::read(f.source.join(GLOBAL_STATE)).unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    let db = Connection::open(f.target.join(STATE_DB)).unwrap();
    let (path, project, cwd): (String, Option<String>, String) = db
        .query_row(
            "SELECT rollout_path, project_id, cwd FROM threads WHERE id = ?1",
            [ROOT],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(Path::new(&path), f.target_path(&f.child_rollout));
    assert_eq!(project.as_deref(), Some("project-new"));
    assert_eq!(cwd, "/workspace/shared");
    assert_eq!(
        fs::read(f.target_path(&f.root_rollout)).unwrap(),
        original_root
    );
    assert_eq!(fs::read(&path).unwrap(), original_child);
    // Appending to the original instance must no longer change the copied thread.
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(&f.child_rollout)
        .unwrap()
        .write_all(b"{\"type\":\"event_msg\",\"new\":true}\n")
        .unwrap();
    assert_eq!(fs::read(&path).unwrap(), original_child);
    validate_lineage(&f.target).unwrap();
    assert_eq!(fs::read(f.source.join(GLOBAL_STATE)).unwrap(), source_state);
    let source_db = Connection::open(f.source.join(STATE_DB)).unwrap();
    assert_eq!(
        source_db
            .query_row("SELECT rollout_path FROM threads", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        f.child_rollout.to_str().unwrap()
    );
    assert!(source_db
        .query_row("SELECT project_id FROM threads", [], |r| r
            .get::<_, Option<String>>(0))
        .unwrap()
        .is_none());
}

#[test]
fn rekeys_local_project_and_pinned_maps_without_changing_remote_hosts() {
    let f = Fixture::new();
    copy_profile(&f.source, &f.target).unwrap();
    let state = read_state(&f.target).unwrap();
    for key in HOST_MAPS {
        assert!(state[key]
            .get(format!("local:{}", f.source.display()))
            .is_none());
        assert!(state[key]
            .get(format!("local:{}", f.target.display()))
            .is_some());
    }
    assert_eq!(state[HOST_MAPS[0]]["remote:keep"]["other"], "unchanged");
    assert_eq!(
        state["local-projects"]["project-old"]["rootPaths"],
        json!([])
    );
    assert_eq!(state["unrelated-setting"], true);
}

#[test]
fn sqlite_snapshot_includes_committed_wal_rows() {
    let f = Fixture::new();
    let db = Connection::open(f.source.join(STATE_DB)).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; INSERT INTO projects VALUES ('wal-only', 'WAL project');").unwrap();
    assert!(
        f.source
            .join("state_5.sqlite-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    copy_profile(&f.source, &f.target).unwrap();
    let copied = Connection::open(f.target.join(STATE_DB)).unwrap();
    assert_eq!(
        copied
            .query_row("SELECT name FROM projects WHERE id='wal-only'", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "WAL project"
    );
    assert_eq!(
        copied
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
}

#[cfg(unix)]
#[test]
fn preserves_existing_target_directory_permissions() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o700, 0o750, 0o500] {
        let f = Fixture::new();
        fs::create_dir(&f.target).unwrap();
        fs::set_permissions(&f.target, fs::Permissions::from_mode(mode)).unwrap();
        let result = copy_profile(&f.source, &f.target);
        let copied_mode = fs::metadata(&f.target).unwrap().permissions().mode() & 0o777;
        // Restore owner write permission so the read-only fixture can be removed.
        fs::set_permissions(&f.target, fs::Permissions::from_mode(0o700)).unwrap();
        result.unwrap();
        assert_eq!(copied_mode, mode);
        assert!(f.target.join(STATE_DB).is_file());
    }
}

#[cfg(unix)]
#[test]
fn new_target_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::set_permissions(&f.source, fs::Permissions::from_mode(0o755)).unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    assert_eq!(
        fs::metadata(&f.target).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn staging_directory_is_private_and_cleaned_up_on_publish_failure() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    for (mode, conflict) in [(0o755, false), (0o500, true)] {
        let f = Fixture::new();
        // Even a shared target must have private staging. A read-only target
        // additionally exercises cleanup if another writer prevents publishing.
        fs::create_dir(&f.target).unwrap();
        fs::set_permissions(&f.target, fs::Permissions::from_mode(mode)).unwrap();
        let db = Connection::open(f.source.join(STATE_DB)).unwrap();
        db.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let (observed_mode, result) = std::thread::scope(|scope| {
            let copy = scope.spawn(|| copy_profile(&f.source, &f.target));
            let deadline = Instant::now() + Duration::from_secs(4);
            let observed = loop {
                let staging = fs::read_dir(&f.dir).unwrap().find_map(|entry| {
                    let entry = entry.unwrap();
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".codex-profile-copy-")
                        .then(|| entry.path())
                });
                if let Some(staging) = staging {
                    break Some(fs::metadata(staging).unwrap().permissions().mode() & 0o777);
                }
                if copy.is_finished() || Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if conflict {
                fs::set_permissions(&f.target, fs::Permissions::from_mode(0o700)).unwrap();
                fs::write(f.target.join("keep"), b"another writer").unwrap();
                fs::set_permissions(&f.target, fs::Permissions::from_mode(mode)).unwrap();
            }
            // Always release the lock and join before asserting the copy result.
            db.execute_batch("ROLLBACK").unwrap();
            (observed, copy.join().unwrap())
        });
        let final_mode = fs::metadata(&f.target).unwrap().permissions().mode() & 0o777;
        fs::set_permissions(&f.target, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(observed_mode, Some(0o700));
        assert_eq!(final_mode, mode);
        if conflict {
            assert!(result.unwrap_err().contains("目标实例目录已被使用"));
            assert_eq!(fs::read(f.target.join("keep")).unwrap(), b"another writer");
            assert_eq!(fs::read_dir(&f.target).unwrap().count(), 1);
        } else {
            result.unwrap();
        }
        assert_eq!(
            fs::read_dir(&f.dir).unwrap().count(),
            2,
            "no staging remains"
        );
    }
}

#[test]
fn rejects_truncated_parent_before_publishing_target() {
    let f = Fixture::new();
    fs::write(
        &f.root_rollout,
        format!("{}\n", json!({"type":"session_meta","payload":{"id":ROOT}})),
    )
    .unwrap();
    fs::create_dir(&f.target).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&f.target, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let error = copy_profile(&f.source, &f.target).unwrap_err();
    assert!(error.contains("字节边界超出"), "{error}");
    assert!(fs::read_dir(&f.target).unwrap().next().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&f.target).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        fs::read_dir(&f.dir).unwrap().count(),
        2,
        "staging is cleaned up"
    );
}

#[test]
fn rejects_missing_parent_and_preserves_source() {
    let f = Fixture::new();
    fs::remove_file(&f.root_rollout).unwrap();
    let child = fs::read(&f.child_rollout).unwrap();
    assert!(copy_profile(&f.source, &f.target)
        .unwrap_err()
        .contains("缺少来源文件"));
    assert!(!f.target.exists());
    assert_eq!(fs::read(&f.child_rollout).unwrap(), child);
}

#[test]
fn validates_multiple_segments_including_archived_ancestor() {
    let f = Fixture::new();
    fs::create_dir_all(f.source.join("archived_sessions")).unwrap();
    fs::rename(
        &f.root_rollout,
        f.source
            .join("archived_sessions")
            .join(f.root_rollout.file_name().unwrap()),
    )
    .unwrap();
    let leaf = f.source.join(format!(
        "sessions/rollout-2026-01-01T00-02-00-{ROOT}_{LEAF}.jsonl"
    ));
    fs::write(&leaf, format!("{}\n", json!({"type":"session_meta", "payload":{"id":ROOT,"history_base":{"thread_id":CHILD,"end_byte_offset":fs::metadata(&f.child_rollout).unwrap().len(),"end_ordinal_exclusive":4}}}))).unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    assert!(f.target_path(&leaf).exists());
    validate_lineage(&f.target).unwrap();
}

#[test]
fn preserves_existing_project_membership() {
    let f = Fixture::new();
    let db = Connection::open(f.source.join(STATE_DB)).unwrap();
    db.execute_batch("INSERT INTO projects VALUES ('explicit', 'Existing'); UPDATE threads SET project_id='explicit';").unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    let copied = Connection::open(f.target.join(STATE_DB)).unwrap();
    assert_eq!(
        copied
            .query_row("SELECT project_id FROM threads", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "explicit"
    );
}

#[test]
fn supports_old_profile_without_project_tables_or_global_state() {
    let f = Fixture::new();
    let db = Connection::open(f.source.join(STATE_DB)).unwrap();
    db.execute_batch("DROP TABLE projects; ALTER TABLE threads DROP COLUMN project_id;")
        .unwrap();
    fs::remove_file(f.source.join(GLOBAL_STATE)).unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    assert!(!f.target.join(GLOBAL_STATE).exists());
}

#[test]
fn refuses_external_rollout_paths_and_nonempty_targets() {
    let f = Fixture::new();
    let db = Connection::open(f.source.join(STATE_DB)).unwrap();
    db.execute(
        "UPDATE threads SET rollout_path=?1",
        [f.dir.join("external.jsonl").to_str().unwrap()],
    )
    .unwrap();
    assert!(copy_profile(&f.source, &f.target)
        .unwrap_err()
        .contains("不在来源实例"));
    assert!(!f.target.exists());
    fs::create_dir(&f.target).unwrap();
    fs::write(f.target.join("keep"), b"keep").unwrap();
    assert!(copy_profile(&f.source, &f.target)
        .unwrap_err()
        .contains("为空"));
    assert_eq!(fs::read(f.target.join("keep")).unwrap(), b"keep");
}

#[test]
fn rejects_non_record_cutoff_and_cycles() {
    for cycle in [false, true] {
        let f = Fixture::new();
        let base = if cycle { CHILD } else { ROOT };
        fs::write(&f.child_rollout, format!("{}\n", json!({"type":"session_meta","payload":{"id":ROOT,"history_base":{"thread_id":base,"end_byte_offset":if cycle {0} else {1},"end_ordinal_exclusive":0}}}))).unwrap();
        let error = copy_profile(&f.source, &f.target).unwrap_err();
        assert!(
            error.contains(if cycle {
                "循环引用"
            } else {
                "不在记录末尾"
            }),
            "{error}"
        );
        assert!(!f.target.exists());
    }
}

#[test]
fn rejects_overlapping_directories_without_changing_source() {
    let f = Fixture::new();
    let nested = f.source.join("nested/target");
    assert!(copy_profile(&f.source, &nested)
        .unwrap_err()
        .contains("不能重叠"));
    assert!(!f.source.join("nested").exists());
}

#[cfg(unix)]
#[test]
fn supports_source_directory_alias_and_preserves_database_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = Fixture::new();
    let alias = f.dir.join("source-alias");
    symlink(&f.source, &alias).unwrap();
    let db = Connection::open(f.source.join(STATE_DB)).unwrap();
    let alias_rollout = alias.join(f.child_rollout.strip_prefix(&f.source).unwrap());
    db.execute(
        "UPDATE threads SET rollout_path=?1",
        [alias_rollout.to_str().unwrap()],
    )
    .unwrap();
    let mut state = read_state(&f.source).unwrap();
    for key in HOST_MAPS {
        let old = state[key]
            .as_object_mut()
            .unwrap()
            .remove(&format!("local:{}", f.source.display()))
            .unwrap();
        state[key][format!("local:{}", alias.display())] = old;
    }
    fs::write(
        f.source.join(GLOBAL_STATE),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    fs::set_permissions(f.source.join(STATE_DB), fs::Permissions::from_mode(0o600)).unwrap();
    copy_profile(&alias, &f.target).unwrap();
    let copied = Connection::open(f.target.join(STATE_DB)).unwrap();
    assert_eq!(
        copied
            .query_row("SELECT rollout_path FROM threads", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        f.target_path(&f.child_rollout).to_str().unwrap()
    );
    assert!(read_state(&f.target).unwrap()[HOST_MAPS[0]]
        .get(format!("local:{}", f.target.display()))
        .is_some());
    assert_eq!(
        fs::metadata(f.target.join(STATE_DB))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[test]
fn keeps_destination_host_identity_when_parent_is_an_alias() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let real_parent = f.dir.join("real-parent");
    let alias_parent = f.dir.join("alias-parent");
    fs::create_dir(&real_parent).unwrap();
    symlink(&real_parent, &alias_parent).unwrap();
    let target = alias_parent.join("new-instance");
    copy_profile(&f.source, &target).unwrap();
    let state = read_state(&target).unwrap();
    assert!(state[HOST_MAPS[0]]
        .get(format!("local:{}", target.display()))
        .is_some());
    let copied = Connection::open(target.join(STATE_DB)).unwrap();
    let rollout: String = copied
        .query_row("SELECT rollout_path FROM threads", [], |r| r.get(0))
        .unwrap();
    assert!(Path::new(&rollout).starts_with(&target));
    assert!(Path::new(&rollout).is_file());
}

#[test]
fn rejects_projection_captured_ahead_of_rollout_snapshot() {
    let f = Fixture::new();
    let db = Connection::open(f.source.join("thread_history_1.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER, next_rollout_ordinal INTEGER);").unwrap();
    let bytes = fs::metadata(&f.child_rollout).unwrap().len() as i64;
    db.execute(
        "INSERT INTO thread_history_projection_state VALUES (?1, ?2, 4)",
        rusqlite::params![ROOT, bytes + 10],
    )
    .unwrap();
    assert!(copy_profile(&f.source, &f.target)
        .unwrap_err()
        .contains("历史索引超出"));
    assert!(!f.target.exists());
    db.execute(
        "UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1",
        [bytes],
    )
    .unwrap();
    copy_profile(&f.source, &f.target).unwrap();
    let copied = Connection::open(f.target.join("thread_history_1.sqlite")).unwrap();
    assert_eq!(
        copied
            .query_row(
                "SELECT next_rollout_byte_offset FROM thread_history_projection_state",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        bytes
    );
}
