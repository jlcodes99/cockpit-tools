use super::*;
use serde_json::json;
use std::{cell::Cell, fs};

const ROOT: &str = "11111111-1111-4111-8111-111111111111";
const LOGICAL: &str = "22222222-2222-4222-8222-222222222222";
const CHILD: &str = "33333333-3333-4333-8333-333333333333";
const OTHER: &str = "44444444-4444-4444-8444-444444444444";

const SCHEMA: &str = "
CREATE TABLE thread_items(thread_id TEXT NOT NULL,turn_id TEXT NOT NULL,item_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,item_json TEXT NOT NULL,PRIMARY KEY(thread_id,turn_id,item_id));
CREATE TABLE thread_realtime_items(thread_id TEXT NOT NULL,item_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,item_type TEXT NOT NULL,item_json TEXT NOT NULL,PRIMARY KEY(thread_id,item_id));
CREATE TABLE thread_turns(thread_id TEXT NOT NULL,turn_id TEXT NOT NULL,rollout_ordinal INTEGER NOT NULL,status TEXT NOT NULL,error_json TEXT,started_at INTEGER,completed_at INTEGER,duration_ms INTEGER,first_user_item_id TEXT,final_agent_item_id TEXT,PRIMARY KEY(thread_id,turn_id));
CREATE TABLE thread_history_projection_state(thread_id TEXT PRIMARY KEY,next_rollout_byte_offset INTEGER NOT NULL,next_rollout_ordinal INTEGER NOT NULL);
CREATE TRIGGER thread_realtime_items_projection_cleanup AFTER DELETE ON thread_history_projection_state BEGIN DELETE FROM thread_realtime_items WHERE thread_id=OLD.thread_id; END;
";

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    parent: PathBuf,
    child: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::create_dir(root.join("sessions")).unwrap();
        let parent = root.join(format!(
            "sessions/rollout-2026-10-24T00-00-00-{ROOT}.jsonl.zst"
        ));
        let child = root.join(format!(
            "sessions/rollout-2026-10-24T00-00-01-{LOGICAL}_{CHILD}.jsonl"
        ));
        let parent_bytes = records(ROOT, None, 0);
        fs::write(
            &parent,
            zstd::stream::encode_all(parent_bytes.as_slice(), 0).unwrap(),
        )
        .unwrap();
        let child_bytes = records(
            LOGICAL,
            Some(
                json!({"thread_id":ROOT,"end_byte_offset":parent_bytes.len()-1,"end_ordinal_exclusive":3}),
            ),
            3,
        );
        fs::write(&child, &child_bytes).unwrap();
        let db = Connection::open(root.join(HISTORY_DB)).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        cache(&db, ROOT, parent_bytes.len() as i64 + 1950, 3);
        cache(&db, CHILD, child_bytes.len() as i64, 6);
        let state = Connection::open(root.join(STATE_DB)).unwrap();
        state
            .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY,rollout_path TEXT);")
            .unwrap();
        state
            .execute(
                "INSERT INTO threads VALUES (?1,?2)",
                [LOGICAL, child.to_str().unwrap()],
            )
            .unwrap();
        Self {
            _temp: temp,
            root,
            parent,
            child,
        }
    }
    fn db(&self) -> Connection {
        Connection::open(self.root.join(HISTORY_DB)).unwrap()
    }
    fn plan(&self) -> Plan {
        scan(
            &self.root,
            Some(&HashSet::from([LOGICAL.to_owned()])),
            &|_, _| {},
        )
        .unwrap()
    }
    fn count(&self, id: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT COUNT(*) FROM thread_items WHERE thread_id=?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }
}

fn records(id: &str, base: Option<serde_json::Value>, ordinal: u64) -> Vec<u8> {
    let mut payload = json!({"id":id,"model_provider":"offline_audit"});
    if let Some(base) = base {
        payload["history_base"] = base;
    }
    let mut result = Vec::new();
    for item in [
        json!({"ordinal":ordinal,"type":"session_meta","payload":payload}),
        json!({"ordinal":ordinal+1,"type":"event_msg","payload":{"type":"user_message","message":"Preserve 原始🙂"}}),
        json!({"ordinal":ordinal+2,"type":"response_item","payload":{"type":"message","role":"assistant","content":[]}}),
    ] {
        result.extend(serde_json::to_vec(&item).unwrap());
        result.push(b'\n');
    }
    result
}

fn cache(db: &Connection, id: &str, offset: i64, ordinal: i64) {
    db.execute(
        "INSERT INTO thread_history_projection_state VALUES (?1,?2,?3)",
        rusqlite::params![id, offset, ordinal],
    )
    .unwrap();
    db.execute(
        "INSERT INTO thread_items VALUES (?1,'turn','item',1,10,?2)",
        rusqlite::params![id, "{\"type\":\"userMessage\"}"],
    )
    .unwrap();
    db.execute("INSERT INTO thread_realtime_items VALUES (?1,'realtime',2,11,'realtime_session_closed','{}')",[id]).unwrap();
    db.execute(
        "INSERT INTO thread_turns VALUES (?1,'turn',1,'completed',NULL,10,11,1,'item',NULL)",
        [id],
    )
    .unwrap();
}

#[test]
fn history_projection_repair_zstd_ancestor_uses_physical_ids_and_keeps_original_bytes() {
    let f = Fixture::new();
    let before_parent = fs::read(&f.parent).unwrap();
    let before_child = fs::read(&f.child).unwrap();
    let plan = f.plan();
    assert_eq!(plan.candidate_count(), 1);
    assert_eq!(plan.scanned_files(), 2);
    let backup = f.root.join("repair-backup");
    assert_eq!(apply(&f.root, &plan, &backup).unwrap(), 1);
    assert_eq!(f.count(ROOT), 0);
    assert_eq!(f.count(CHILD), 1);
    for table in storage::TABLES {
        let n: i64 = f
            .db()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE thread_id=?1"),
                [ROOT],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0);
    }
    assert_eq!(fs::read(&f.parent).unwrap(), before_parent);
    assert_eq!(fs::read(&f.child).unwrap(), before_child);
    let copy = Connection::open(backup.join("history-projection.sqlite")).unwrap();
    assert_eq!(
        copy.query_row("SELECT COUNT(*) FROM thread_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(backup.join("history-projection.sqlite.sha256").is_file());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(backup.join("history-projection.sqlite"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let again = f.plan();
    assert_eq!(again.candidate_count(), 0);
    assert_eq!(
        apply(&f.root, &again, &f.root.join("unneeded-backup")).unwrap(),
        0
    );
    assert!(!f.root.join("unneeded-backup").exists());
}

#[test]
fn history_projection_repair_backup_preserves_sqlite_storage_types() {
    let f = Fixture::new();
    f.db()
        .execute(
            "UPDATE thread_items SET item_json=?1 WHERE thread_id=?2",
            rusqlite::params![vec![0_u8, 255, 1, 0], ROOT],
        )
        .unwrap();
    let backup = f.root.join("typed-backup");
    apply(&f.root, &f.plan(), &backup).unwrap();
    let copy = Connection::open(backup.join("history-projection.sqlite")).unwrap();
    let (kind, blob): (String, Vec<u8>) = copy
        .query_row(
            "SELECT typeof(item_json),item_json FROM thread_items",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(kind, "blob");
    assert_eq!(blob, [0, 255, 1, 0]);
    let types:String=copy.query_row("SELECT typeof(error_json)||','||typeof(started_at)||','||typeof(status) FROM thread_turns",[],|r|r.get(0)).unwrap();
    assert_eq!(types, "null,integer,text");
}

#[test]
fn history_projection_repair_missing_records_never_become_a_cache_reset() {
    let f = Fixture::new();
    f.db()
        .execute(
            "UPDATE thread_history_projection_state SET next_rollout_ordinal=4 WHERE thread_id=?1",
            [ROOT],
        )
        .unwrap();
    let failure = scan(&f.root, None, &|_, _| {}).unwrap_err();
    assert!(failure.contains("MISSING_RECORDS"));
    assert_eq!(f.count(ROOT), 1);
}

#[test]
fn history_projection_repair_rejects_truncated_compression_and_partial_json() {
    for compressed in [true, false] {
        let f = Fixture::new();
        let path = if compressed { &f.parent } else { &f.child };
        let mut bytes = fs::read(path).unwrap();
        bytes.truncate(bytes.len() / 2);
        fs::write(path, bytes).unwrap();
        assert!(scan(&f.root, None, &|_, _| {})
            .unwrap_err()
            .contains("UNSAFE_HISTORY"));
        assert_eq!(f.count(ROOT), 1);
    }
}

#[test]
fn history_projection_repair_rejects_unknown_schema_and_trigger() {
    for change in ["ALTER TABLE thread_items ADD COLUMN future_payload TEXT", "CREATE TRIGGER foreign_cleanup AFTER DELETE ON thread_turns BEGIN DELETE FROM thread_items; END", "ALTER TABLE thread_items ADD COLUMN hidden_payload TEXT AS (item_json)", "CREATE UNIQUE INDEX unknown_item_constraint ON thread_items(thread_id,item_json)", "CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY, success BOOLEAN NOT NULL); INSERT INTO _sqlx_migrations VALUES (1,0)", "CREATE TABLE _sqlx_migrations(version BIGINT PRIMARY KEY, success BOOLEAN NOT NULL); INSERT INTO _sqlx_migrations VALUES (9,1)"] {
        let f=Fixture::new();f.db().execute_batch(change).unwrap();
        assert!(scan(&f.root,None,&|_,_|{}).unwrap_err().contains("SCHEMA"));assert_eq!(f.count(ROOT),1);
    }
}

#[test]
fn history_projection_repair_selected_sessions_leave_unrelated_projections_untouched() {
    let f = Fixture::new();
    let other = f.root.join(format!(
        "sessions/rollout-2026-10-24T00-00-02-{OTHER}.jsonl"
    ));
    let bytes = records(OTHER, None, 0);
    fs::write(&other, &bytes).unwrap();
    cache(&f.db(), OTHER, bytes.len() as i64 + 2000, 3);
    apply(&f.root, &f.plan(), &f.root.join("backup")).unwrap();
    assert_eq!(f.count(ROOT), 0);
    assert_eq!(f.count(OTHER), 1);
}

#[test]
fn history_projection_repair_detects_source_and_cursor_changes_before_deletion() {
    for source_change in [true, false] {
        let f = Fixture::new();
        let plan = f.plan();
        if source_change {
            let mut bytes = fs::read(&f.child).unwrap();
            bytes.push(b'\n');
            fs::write(&f.child, bytes).unwrap();
        } else {
            f.db().execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=1 WHERE thread_id=?1",[ROOT]).unwrap();
        }
        assert!(apply(&f.root, &plan, &f.root.join("backup"))
            .unwrap_err()
            .contains("CHANGED"));
        assert_eq!(f.count(ROOT), 1);
    }
}

#[test]
fn history_projection_repair_detects_changed_derived_rows_after_the_backup() {
    let f = Fixture::new();
    let plan = f.plan();
    let calls = Cell::new(0);
    let result = apply_with_checkpoints(&f.root, &plan, &f.root.join("backup"), &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            f.db()
                .execute(
                    "UPDATE thread_items SET item_json='new concurrent value' WHERE thread_id=?1",
                    [ROOT],
                )
                .unwrap();
        }
    });
    assert!(result.unwrap_err().contains("CHANGED"));
    assert_eq!(f.count(ROOT), 1);
    let value: String = f
        .db()
        .query_row(
            "SELECT item_json FROM thread_items WHERE thread_id=?1",
            [ROOT],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(value, "new concurrent value");
}

#[test]
fn history_projection_repair_allows_open_client_connections_and_retains_unrelated_writes() {
    let f = Fixture::new();
    let client = f.db();
    client.pragma_update(None, "journal_mode", "WAL").unwrap();
    cache(&client, OTHER, 20, 1);
    let plan = f.plan();
    let calls = Cell::new(0);
    let backup = f.root.join("live-backup");
    let repaired = apply_with_checkpoints(&f.root, &plan, &backup, &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 1 {
            client
                .execute(
                    "UPDATE thread_items SET item_json='new unrelated message' WHERE thread_id=?1",
                    [OTHER],
                )
                .unwrap();
        }
    })
    .unwrap();
    assert_eq!(repaired, 1);
    assert_eq!(f.count(ROOT), 0);
    assert_eq!(f.count(CHILD), 1);
    let value: String = client
        .query_row(
            "SELECT item_json FROM thread_items WHERE thread_id=?1",
            [OTHER],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "new unrelated message");
    assert!(backup.join("history-projection.sqlite").is_file());
}

#[test]
fn history_projection_repair_busy_client_fails_promptly_without_deleting_history() {
    for writer in [true, false] {
        let f = Fixture::new();
        let client = f.db();
        if writer {
            client.pragma_update(None, "journal_mode", "WAL").unwrap();
        }
        let plan = f.plan();
        client
            .execute_batch(if writer {
                "BEGIN IMMEDIATE"
            } else {
                "BEGIN; SELECT * FROM thread_items"
            })
            .unwrap();
        let started = Instant::now();
        let result = apply(&f.root, &plan, &f.root.join("busy-backup"));
        assert!(result.unwrap_err().contains("DATABASE"));
        assert!(started.elapsed() < Duration::from_secs(2));
        client.execute_batch("ROLLBACK").unwrap();
        for table in storage::TABLES {
            let count: i64 = client
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE thread_id=?1"),
                    [ROOT],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        assert_eq!(
            apply(&f.root, &f.plan(), &f.root.join("retry-backup")).unwrap(),
            1
        );
    }
}

#[test]
fn history_projection_repair_missing_or_duplicate_ancestors_are_not_guessed() {
    for duplicate in [true, false] {
        let f = Fixture::new();
        if duplicate {
            fs::write(f.parent.with_extension(""), records(ROOT, None, 0)).unwrap();
        } else {
            fs::remove_file(&f.parent).unwrap();
        }
        let error = scan(
            &f.root,
            Some(&HashSet::from([LOGICAL.to_owned()])),
            &|_, _| {},
        )
        .unwrap_err();
        assert!(error.contains(if duplicate {
            "AMBIGUOUS_SOURCE"
        } else {
            "SOURCE_MISSING"
        }));
        assert_eq!(f.count(ROOT), 1);
    }
}

#[test]
fn history_projection_repair_rejects_header_identity_mismatch_and_backup_failure() {
    let f = Fixture::new();
    fs::write(&f.child, records(OTHER, None, 0)).unwrap();
    assert!(scan(
        &f.root,
        Some(&HashSet::from([LOGICAL.to_owned()])),
        &|_, _| {}
    )
    .unwrap_err()
    .contains("UNSAFE_HISTORY"));
    let f = Fixture::new();
    let blocked = f.root.join("blocked-backup");
    fs::write(&blocked, b"existing user file").unwrap();
    assert!(apply(&f.root, &f.plan(), &blocked)
        .unwrap_err()
        .contains("BACKUP"));
    assert_eq!(f.count(ROOT), 1);
    assert_eq!(fs::read(&blocked).unwrap(), b"existing user file");
}

#[test]
fn history_projection_repair_source_change_before_commit_rolls_back() {
    let f = Fixture::new();
    let calls = Cell::new(0);
    let plan = f.plan();
    let result = apply_with_checkpoints(&f.root, &plan, &f.root.join("backup"), &|| {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            let mut bytes = fs::read(&f.child).unwrap();
            bytes.push(b'\n');
            fs::write(&f.child, bytes).unwrap();
        }
    });
    assert!(result.unwrap_err().contains("CHANGED"));
    assert_eq!(f.count(ROOT), 1);
}

#[test]
fn history_projection_repair_budget_and_profile_scope_are_enforced() {
    let budget = Budget {
        started: Instant::now() - Duration::from_secs(46),
        bytes: 0,
    };
    assert!(budget.check().unwrap_err().contains("LIMIT"));
    let f = Fixture::new();
    let other = Fixture::new();
    assert!(apply(&other.root, &f.plan(), &other.root.join("backup"))
        .unwrap_err()
        .contains("CHANGED"));
    f.db().execute("UPDATE thread_history_projection_state SET next_rollout_byte_offset=-1 WHERE thread_id=?1", [ROOT]).unwrap();
    assert!(scan(&f.root, None, &|_, _| {})
        .unwrap_err()
        .contains("UNSAFE_HISTORY"));
}
