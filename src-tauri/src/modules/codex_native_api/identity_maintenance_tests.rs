use super::*;
use serde_json::json;

fn seed_checkpoint_row(
    store: &IdentityStore,
    caller: &str,
    thread: &str,
    response: &str,
    created: i64,
    payload: &str,
) {
    let transcript = serde_json::to_string(&[json!({ "type": "message", "content": payload })]).unwrap();
    let bytes = transcript.len() as i64;
    let conn = store.connection.lock().unwrap();
    conn.execute(
        "INSERT INTO native_rebuild_checkpoints_v1
            (caller,response,thread,account,generation,transcript,bytes,created,parameters)
         VALUES(?1,?2,?3,'acc','gen',?4,?5,?6,NULL)",
        params![caller, response, thread, transcript, bytes, created],
    )
    .unwrap();
}

#[test]
fn prune_archive_batch_evicts_expired_and_superseded_checkpoints_while_preserving_identity_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();

    // Seed protected identity/affinity/opaque rows that must NEVER be deleted.
    {
        let conn = store.connection.lock().unwrap();
        conn.execute(
            "INSERT INTO native_identity_v1(caller,owner,original,mapped) VALUES('k1','o1','orig','map1')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO native_affinity_v1(caller,kind,original,account,owner) VALUES('k1','thread','t1','acc','o1')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO native_opaque_v1(caller,kind,digest,account,owner,session,thread,turn,expires)
             VALUES('k1','encrypted','abc','acc','o1','s1','t1',NULL,0)",
            [],
        ).unwrap();
    }

    let now = 1_700_000_000_i64;
    let old = now - ARCHIVE_RETENTION_SECS - 3600;
    // 2 expired rows on old thread
    seed_checkpoint_row(
        &store,
        "k1",
        "old-thread",
        "resp-old-1",
        old,
        "expired-payload-1",
    );
    seed_checkpoint_row(
        &store,
        "k1",
        "old-thread",
        "resp-old-2",
        old + 1,
        "expired-payload-2",
    );
    // 5 recent turns on active thread (policy max_history_per_thread = 2 should keep latest 2)
    for idx in 0..5 {
        seed_checkpoint_row(
            &store,
            "k1",
            "active-thread",
            &format!("resp-act-{idx}"),
            now - 100 + idx,
            &format!("active-payload-{idx}"),
        );
    }

    let policy = MaintenancePolicy {
        max_history_per_thread: 2,
        batch_size: 10,
        ..MaintenancePolicy::default()
    };
    let report = store.prune_archive_batch(now, &policy, false).unwrap();
    assert_eq!(report.deleted_checkpoint_rows, 5); // 2 expired + 3 superseded

    let conn = store.connection.lock().unwrap();
    let remaining_checkpoints: Vec<String> = conn
        .prepare("SELECT response FROM native_rebuild_checkpoints_v1 ORDER BY created ASC")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining_checkpoints, vec!["resp-act-3", "resp-act-4"]);

    // Protected metadata tables untouched
    for table in [
        "native_identity_v1",
        "native_affinity_v1",
        "native_opaque_v1",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "{table} must be preserved");
    }
}

#[test]
fn clear_archived_history_and_compact_reclaims_disk_space_and_converts_legacy_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");

    // Simulate a pre-upgrade legacy DB with auto_vacuum = NONE and legacy tables.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA auto_vacuum = NONE;
             PRAGMA journal_mode = WAL;
             CREATE TABLE native_rebuild_history_v1 (
                caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
                account TEXT NOT NULL, generation TEXT NOT NULL, manifest TEXT NOT NULL,
                parameters TEXT, PRIMARY KEY(caller,response));
             CREATE TABLE native_rebuild_items_v1 (
                caller TEXT NOT NULL, digest TEXT NOT NULL, body BLOB NOT NULL,
                PRIMARY KEY(caller,digest));",
        )
        .unwrap();
        // Insert incompressible random-ish blobs totaling ~1.5 MiB
        let mut state: u64 = 0x1234_5678_9abc_def0;
        for i in 0..24u32 {
            let mut blob = vec![0u8; 64 * 1024];
            for byte in &mut blob {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                *byte = (state >> 56) as u8;
            }
            let digest = format!("{:064x}", i);
            let manifest = serde_json::to_string(&[&digest]).unwrap();
            conn.execute(
                "INSERT INTO native_rebuild_history_v1(caller,response,thread,account,generation,manifest)
                 VALUES('k1',?1,'t1','acc','gen',?2)",
                params![format!("resp-{i}"), manifest],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO native_rebuild_items_v1(caller,digest,body) VALUES('k1',?1,?2)",
                params![digest, blob],
            )
            .unwrap();
        }
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }

    let size_before = std::fs::metadata(&path).unwrap().len();
    assert!(
        size_before > 1_000_000,
        "expected >1 MiB before compact, got {size_before}"
    );

    // Opening with IdentityStore automatically drops legacy archive tables.
    let store = IdentityStore::open(path.clone()).unwrap();
    // Seed one very fresh turn that should survive explicit clear (active conversation grace).
    let now = 1_800_000_000_i64;
    seed_checkpoint_row(
        &store,
        "k1",
        "live-thread",
        "resp-live",
        now - 30,
        "live-turn",
    );

    let report = store
        .clear_archived_history_and_compact_at(now, ACTIVE_CONVERSATION_GRACE_SECS)
        .unwrap();
    assert!(report.vacuumed);

    let size_after = std::fs::metadata(&path).unwrap().len();
    assert!(
        size_after < size_before / 4,
        "expected file size to shrink drastically: before={size_before}, after={size_after}"
    );

    let conn = store.connection.lock().unwrap();
    let auto_vacuum: i64 = conn
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        auto_vacuum, 2,
        "legacy DB should be converted to INCREMENTAL auto_vacuum"
    );
    let live_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1 WHERE response='resp-live'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        live_exists, 1,
        "active conversation turn within grace window must remain"
    );
}

#[test]
fn oversized_legacy_archive_auto_slims_on_open_and_salvages_recent_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");

    // Simulate a bloated legacy DB (>16 MiB) with legacy `native_rebuild_history_v1`,
    // `native_rebuild_items_v1`, and manifest-based `native_partial_history_v1`.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA auto_vacuum = NONE;
             PRAGMA journal_mode = WAL;
             CREATE TABLE native_identity_v1 (
                caller TEXT NOT NULL, owner TEXT NOT NULL, original TEXT NOT NULL,
                mapped TEXT NOT NULL, PRIMARY KEY(caller, owner, original));
             CREATE TABLE native_rebuild_history_v1 (
                caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
                account TEXT NOT NULL, generation TEXT NOT NULL, manifest TEXT NOT NULL,
                parameters TEXT, created INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(caller,response));
             CREATE TABLE native_rebuild_items_v1 (
                caller TEXT NOT NULL, digest TEXT NOT NULL, body BLOB NOT NULL,
                PRIMARY KEY(caller,digest));
             CREATE TABLE native_partial_history_v1 (
                caller TEXT NOT NULL, response TEXT NOT NULL, thread TEXT NOT NULL,
                session TEXT, account TEXT NOT NULL, owner TEXT NOT NULL,
                manifest TEXT NOT NULL, parameters TEXT, bytes INTEGER NOT NULL,
                created INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(caller,response));",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO native_identity_v1(caller,owner,original,mapped) VALUES('caller','owner-a','thread','mapped-thread')",
            [],
        )
        .unwrap();

        // Seed 18 MiB of legacy unreferenced bloat in `native_rebuild_items_v1`.
        let mut state: u64 = 0x9876_5432_10fe_dcba;
        for i in 0..72u32 {
            let mut blob = vec![0u8; 256 * 1024];
            for byte in &mut blob {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                *byte = (state >> 56) as u8;
            }
            conn.execute(
                "INSERT INTO native_rebuild_items_v1(caller,digest,body) VALUES('caller',?1,?2)",
                params![format!("bloat-{i:04}"), blob],
            )
            .unwrap();
        }

        // Seed one valid legacy completed turn and one valid legacy partial turn.
        let valid_item = serde_json::to_vec(&json!({"type":"message","role":"user","content":"salvaged turn"})).unwrap();
        let valid_digest = format!("{:x}", Sha256::digest(&valid_item));
        let valid_compressed = zstd::stream::encode_all(valid_item.as_slice(), 1).unwrap();
        let valid_manifest = serde_json::to_string(&[&valid_digest]).unwrap();
        conn.execute(
            "INSERT INTO native_rebuild_items_v1(caller,digest,body) VALUES('caller',?1,?2)",
            params![valid_digest, valid_compressed],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO native_rebuild_history_v1(caller,response,thread,account,generation,manifest,parameters,created)
             VALUES('caller','salvaged-resp','thread','a','gen',?1,'{\"instructions\":\"kept\"}',100)",
            params![valid_manifest],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO native_partial_history_v1(caller,response,thread,session,account,owner,manifest,parameters,bytes,created)
             VALUES('caller','salvaged-partial','thread','session','a','owner-a',?1,NULL,32,100)",
            params![valid_manifest],
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    }

    let size_before = std::fs::metadata(&path).unwrap().len();
    assert!(
        size_before > 16 * 1024 * 1024,
        "expected >16 MiB legacy DB, got {size_before}"
    );

    // Opening automatically performs side-file rebuild, salvaging the active
    // rows and discarding the 18 MiB legacy chunk tables without a slow VACUUM.
    let store = IdentityStore::open(path.clone()).unwrap();
    let size_after = std::fs::metadata(&path).unwrap().len();
    assert!(
        size_after < 1024 * 1024,
        "expected slimmed DB <1 MiB on open, got {size_after} (was {size_before})"
    );

    let conn = store.connection.lock().unwrap();
    assert!(!table_exists(&conn, "native_rebuild_history_v1").unwrap());
    assert!(!table_exists(&conn, "native_rebuild_items_v1").unwrap());
    let identity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM native_identity_v1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(identity_count, 1);
    let salvaged_transcript: String = conn
        .query_row(
            "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='caller' AND response='salvaged-resp'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(salvaged_transcript.contains("salvaged turn"));
    let salvaged_partial: String = conn
        .query_row(
            "SELECT transcript FROM native_partial_history_v1 WHERE caller='caller' AND response='salvaged-partial'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(salvaged_partial.contains("salvaged turn"));
}

#[test]
fn maintenance_guard_enforces_single_flight() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(IdentityStore::open(dir.path().join("identity.sqlite")).unwrap());
    let first = store.try_acquire_maintenance_guard();
    assert!(first.is_some());
    assert!(store.try_acquire_maintenance_guard().is_none());
    drop(first);
    assert!(store.try_acquire_maintenance_guard().is_some());
}

#[test]
fn maintenance_does_not_wait_for_a_foreground_connection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(IdentityStore::open(dir.path().join("identity.sqlite")).unwrap());
    let connection = store.connection.lock().unwrap();
    let worker_store = Arc::clone(&store);
    let (sent, received) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sent.send(worker_store.prune_archive_batch(
            1_700_000_000,
            &MaintenancePolicy::default(),
            false,
        ))
        .unwrap();
    });
    let result = received.recv_timeout(Duration::from_secs(1));
    // Release before joining even when the assertion would fail on old code.
    drop(connection);
    worker.join().unwrap();
    let report = result
        .expect("maintenance must yield to foreground work")
        .unwrap();
    assert!(report.has_more);
    assert!(!report.any_deleted());
}

#[test]
fn interrupted_maintenance_rolls_back_and_restores_foreground_connection() {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    let mut connection = store.connection.lock().unwrap();
    let result = with_batch_budget(&mut connection, Duration::ZERO, |connection| {
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO native_identity_v1(caller,owner,original,mapped)
             VALUES('caller','owner','pending','pending-mapped')",
                [],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .query_row(
                "WITH RECURSIVE numbers(n) AS (
                SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<10000
             ) SELECT SUM(n) FROM numbers",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(MaintenanceReport::default())
    });
    assert!(result.unwrap_err().contains("time budget"));
    assert!(
        connection.is_autocommit(),
        "interrupted transaction must release its write lock"
    );
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM native_identity_v1 WHERE original='pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending, 0,
        "the interrupted transaction must be rolled back"
    );
    let busy_timeout: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy_timeout, 2000);
    // Foreground reads and writes must remain usable after cancellation.
    let sum: i64 = connection
        .query_row(
            "WITH RECURSIVE numbers(n) AS (
            SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<10000
         ) SELECT SUM(n) FROM numbers",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sum, 50_005_000);
    connection
        .execute(
            "INSERT INTO native_identity_v1(caller,owner,original,mapped)
         VALUES('caller','owner','foreground','foreground-mapped')",
            [],
        )
        .unwrap();
}

#[test]
fn maintenance_does_not_wait_for_another_sqlite_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    let writer = Connection::open(path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let result = store.prune_archive_batch(1_700_000_000, &MaintenancePolicy::default(), false);
    let elapsed = started.elapsed();
    writer.execute_batch("ROLLBACK").unwrap();
    assert!(result.unwrap_err().contains("locked"));
    assert!(
        elapsed < Duration::from_secs(1),
        "background work waited {elapsed:?} for another writer"
    );
    let connection = store.connection.lock().unwrap();
    let busy_timeout: i64 = connection
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        busy_timeout, 2000,
        "foreground busy timeout must be restored on errors"
    );
}

#[test]
fn retention_keeps_newest_rowids_with_equal_timestamps_for_each_caller() {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    let now = 1_700_000_000;
    for caller in ["caller-a", "caller-b"] {
        for index in 0..4 {
            seed_checkpoint_row(
                &store,
                caller,
                "same-thread",
                &format!("response-{index}"),
                now - 10,
                "payload",
            );
        }
    }
    let policy = MaintenancePolicy {
        max_history_per_thread: 2,
        ..MaintenancePolicy::default()
    };
    let report = store.prune_archive_batch(now, &policy, false).unwrap();
    assert_eq!(report.deleted_checkpoint_rows, 4);
    let connection = store.connection.lock().unwrap();
    let rows: Vec<(String, String)> = connection
        .prepare("SELECT caller,response FROM native_rebuild_checkpoints_v1 ORDER BY caller,response")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("caller-a".to_string(), "response-2".to_string()),
            ("caller-a".to_string(), "response-3".to_string()),
            ("caller-b".to_string(), "response-2".to_string()),
            ("caller-b".to_string(), "response-3".to_string()),
        ]
    );
}

#[test]
fn successful_maintenance_removes_its_progress_handler() {
    let dir = tempfile::tempdir().unwrap();
    let store = IdentityStore::open(dir.path().join("identity.sqlite")).unwrap();
    let mut connection = store.connection.lock().unwrap();
    // Complete before invoking the handler. If it leaks, the next query will
    // hit its zero-duration deadline and be interrupted.
    with_batch_budget(&mut connection, Duration::ZERO, |_| {
        Ok(MaintenanceReport::default())
    })
    .unwrap();
    let sum: i64 = connection
        .query_row(
            "WITH RECURSIVE numbers(n) AS (
            SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<10000
         ) SELECT SUM(n) FROM numbers",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sum, 50_005_000);
}

#[test]
fn incremental_vacuum_reclaims_multiple_pages_per_batch_and_truncates_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    let store = IdentityStore::open(path.clone()).unwrap();
    let now = 1_700_000_000;
    let payload = "x".repeat(64 * 1024);
    for idx in 0..80 {
        seed_checkpoint_row(
            &store,
            "caller",
            &format!("thread-{idx}"),
            &format!("resp-{idx}"),
            now - 8 * 86_400,
            &payload,
        );
    }
    let before = read_db_size_metrics(&store.connection.lock().unwrap()).unwrap();
    assert!(
        before.total_bytes > 4 * 1024 * 1024,
        "expected seeded DB > 4 MiB, got {}",
        before.total_bytes
    );

    let report = store
        .prune_archive_batch(now, &MaintenancePolicy::default(), false)
        .unwrap();
    assert_eq!(report.deleted_checkpoint_rows, 80);

    let after = read_db_size_metrics(&store.connection.lock().unwrap()).unwrap();
    assert_eq!(
        after.freelist_bytes, 0,
        "incremental vacuum must step all rows and drain freelist_bytes to 0, got {}",
        after.freelist_bytes
    );
    assert!(
        after.total_bytes < 512 * 1024,
        "expected total_bytes < 512 KiB after incremental vacuum, got {}",
        after.total_bytes
    );
    let disk_len = std::fs::metadata(&path).unwrap().len() as i64;
    assert!(
        disk_len < 512 * 1024,
        "expected on-disk file size < 512 KiB after WAL truncate checkpoint, got {disk_len}"
    );
}
