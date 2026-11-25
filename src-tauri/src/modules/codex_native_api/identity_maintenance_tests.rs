use super::*;
use serde_json::json;

fn seed_history_row(
    store: &IdentityStore,
    caller: &str,
    thread: &str,
    response: &str,
    created: i64,
    payload: &str,
) {
    let raw = serde_json::to_vec(&json!({ "type": "message", "content": payload })).unwrap();
    let digest = format!("{:x}", Sha256::digest(&raw));
    let compressed = zstd::stream::encode_all(raw.as_slice(), 1).unwrap();
    let manifest = serde_json::to_string(&[&digest]).unwrap();
    let conn = store.connection.lock().unwrap();
    conn.execute(
        "INSERT INTO native_rebuild_history_v1(caller,response,thread,account,generation,manifest,parameters,created)
         VALUES(?1,?2,?3,'acc','gen',?4,NULL,?5)",
        params![caller, response, thread, manifest, created],
    )
    .unwrap();
    conn.execute(
        "INSERT OR IGNORE INTO native_rebuild_items_v1(caller,digest,body) VALUES(?1,?2,?3)",
        params![caller, digest, compressed],
    )
    .unwrap();
}

#[test]
fn prune_archive_batch_evicts_expired_and_superseded_history_while_preserving_identity_tables() {
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
    seed_history_row(
        &store,
        "k1",
        "old-thread",
        "resp-old-1",
        old,
        "expired-payload-1",
    );
    seed_history_row(
        &store,
        "k1",
        "old-thread",
        "resp-old-2",
        old + 1,
        "expired-payload-2",
    );
    // 5 recent turns on active thread (policy max_history_per_thread = 2 should keep latest 2)
    for idx in 0..5 {
        seed_history_row(
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
    assert_eq!(report.deleted_history_rows, 5); // 2 expired + 3 superseded
    assert_eq!(report.deleted_item_rows, 5);

    let conn = store.connection.lock().unwrap();
    let remaining_history: Vec<String> = conn
        .prepare("SELECT response FROM native_rebuild_history_v1 ORDER BY created ASC")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(remaining_history, vec!["resp-act-3", "resp-act-4"]);

    let remaining_items: i64 = conn
        .query_row("SELECT COUNT(*) FROM native_rebuild_items_v1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(remaining_items, 2);

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

    // Simulate a pre-upgrade legacy DB with auto_vacuum = NONE and no `created` column.
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

    // Opening with IdentityStore adds `created` column without rewriting legacy rows.
    let store = IdentityStore::open(path.clone()).unwrap();
    // Seed one very fresh turn that should survive explicit clear (active conversation grace).
    let now = 1_800_000_000_i64;
    seed_history_row(
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
    assert_eq!(report.deleted_history_rows, 24);
    assert_eq!(report.deleted_item_rows, 24);
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
            "SELECT COUNT(*) FROM native_rebuild_history_v1 WHERE response='resp-live'",
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
            seed_history_row(
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
    assert_eq!(report.deleted_history_rows, 4);
    let connection = store.connection.lock().unwrap();
    let rows: Vec<(String, String)> = connection
        .prepare("SELECT caller,response FROM native_rebuild_history_v1 ORDER BY caller,response")
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
