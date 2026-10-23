use super::*;

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    schema(&conn);
    conn
}

fn schema(conn: &Connection) {
    conn.execute_batch("CREATE TABLE request_logs (
        id INTEGER PRIMARY KEY, account_id TEXT NOT NULL, timestamp INTEGER NOT NULL,
        input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
        total_tokens INTEGER NOT NULL, cached_tokens INTEGER NOT NULL,
        estimated_cost_usd REAL NOT NULL, error_category TEXT DEFAULT 'other');
        CREATE INDEX idx_codex_local_access_logs_account ON request_logs(account_id, timestamp DESC);").unwrap();
}

fn query(account: &str, window: &str, start: i64, end: i64) -> CodexLocalAccessAccountWindowQuery {
    CodexLocalAccessAccountWindowQuery {
        account_id: account.into(),
        window_key: window.into(),
        start_at: start,
        end_at: end,
    }
}

#[test]
fn window_queries_use_account_time_index_and_preserve_exact_counters() {
    let conn = database();
    let exact = 9_007_199_254_740_993i64;
    conn.execute(
        "INSERT INTO request_logs VALUES (1, 'a', 1700000000000, ?1, 3, ?2, 2, 0.25, 'other')",
        params![exact, exact + 3],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO request_logs VALUES (2, 'b', 1700000000000, 77, 4, 81, 2, 1, 'other')",
        [],
    )
    .unwrap();
    let rows = query_local_access_account_window_stats_from_conn(
        &conn,
        vec![
            query(" a ", "primary", 1_700_000_000, 1_700_000_000),
            query("b", "primary", 1_700_000_000, 1_700_000_000),
            query("a", "secondary", 1_700_000_001, 1_700_000_002),
        ],
    )
    .unwrap();
    assert_eq!(rows[0].account_id, "a");
    assert_eq!(rows[0].input_tokens, exact as u64);
    assert_eq!(rows[0].request_count, 1);
    assert_eq!(rows[0].estimated_cost_usd, 0.25);
    assert_eq!(rows[1].input_tokens, 77);
    assert_eq!(rows[2].request_count, 0);
    let plan = conn
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {ACCOUNT_WINDOW_AGGREGATE_SQL}"
        ))
        .unwrap()
        .query_map(params!["a", 0, i64::MAX], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join(" ");
    assert!(
        plan.contains("idx_codex_local_access_logs_account"),
        "{plan}"
    );
    assert!(
        plan.contains("account_id=? AND timestamp>? AND timestamp<?"),
        "{plan}"
    );
}

#[test]
fn window_query_errors_are_not_partial_or_fake_zero_results() {
    let conn = database();
    conn.execute(
        "INSERT INTO request_logs VALUES (1, 'overflow', 1700000000000, ?1, 0, 0, 0, 0, 'other')",
        [i64::MAX],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO request_logs VALUES (2, 'overflow', 1700000000000, 1, 0, 0, 0, 0, 'other')",
        [],
    )
    .unwrap();
    assert!(query_local_access_account_window_stats_from_conn(
        &conn,
        vec![
            query("empty", "primary", 0, i64::MAX),
            query("overflow", "primary", 0, i64::MAX),
        ]
    )
    .is_err());
    assert!(query_local_access_account_window_stats_from_conn(
        &Connection::open_in_memory().unwrap(),
        vec![query("a", "primary", 0, 0)]
    )
    .is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.sqlite");
    assert!(query_local_access_account_window_stats_at_path(
        &path,
        vec![query("a", "primary", 0, 0)]
    )
    .is_err());
    assert!(!path.exists(), "reader must not create an empty database");
}

#[test]
fn window_query_range_limit_rejects_without_truncation() {
    let queries = (0..=ACCOUNT_WINDOW_QUERY_LIMIT)
        .map(|index| query("a", &index.to_string(), 0, 0))
        .collect::<Vec<_>>();
    assert!(validate_account_window_queries(&queries).is_err());
    assert!(validate_account_window_queries(&queries[..ACCOUNT_WINDOW_QUERY_LIMIT]).is_ok());
    assert!(
        validate_account_window_queries(&[query("a", "same", 0, 0), query("a", "same", 0, 1)])
            .is_err()
    );
    assert!(validate_account_window_queries(&[query("a", "primary", -1, 0)]).is_err());
}

#[test]
fn window_reader_does_not_migrate_or_write_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("logs.sqlite");
    let conn = Connection::open(&path).unwrap();
    schema(&conn);
    conn.execute(
        "INSERT INTO request_logs VALUES (1, 'a', 1700000000000, 1, 2, 3, 0, 0, 'legacy')",
        [],
    )
    .unwrap();
    drop(conn);
    let before = fs::read(&path).unwrap();
    let rows = query_local_access_account_window_stats_at_path(
        &path,
        vec![query("a", "primary", 0, i64::MAX)],
    )
    .unwrap();
    assert_eq!(rows[0].total_tokens, 3);
    assert_eq!(fs::read(&path).unwrap(), before);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'request_log_maintenance'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn sql_deadline_rearms_across_gaps_between_statements() {
    let conn = Connection::open_in_memory().unwrap();
    let started = Instant::now();
    let result = with_request_log_sql_budget(&conn, Duration::from_millis(2), || {
        std::thread::sleep(Duration::from_millis(15));
        conn.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x < 1000000000) SELECT SUM(x) FROM n", [], |row| row.get::<_, i64>(0))
            .map_err(|error| error.to_string())
    });
    assert!(result.unwrap_err().contains("interrupted"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn panicking_query_releases_deadline_thread() {
    let conn = Connection::open_in_memory().unwrap();
    let started = Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_request_log_sql_budget::<()>(&conn, Duration::from_millis(5), || {
            panic!("synthetic query failure")
        })
    }));
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn cancelling_caller_keeps_physical_query_slot_until_worker_exits() {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let task = tokio::spawn(run_bounded_account_window_query(move || {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        Ok(Vec::new())
    }));
    tokio::task::spawn_blocking(move || entered_rx.recv_timeout(Duration::from_secs(2)).unwrap())
        .await
        .unwrap();
    task.abort();
    let _ = task.await;
    assert_eq!(account_window_query_slots().available_permits(), 1);
    let second = account_window_query_slots()
        .clone()
        .try_acquire_owned()
        .unwrap();
    assert!(run_bounded_account_window_query(|| Ok(Vec::new()))
        .await
        .is_err());
    release_tx.send(()).unwrap();
    for _ in 0..100 {
        if account_window_query_slots().available_permits() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(account_window_query_slots().available_permits(), 1);
    drop(second);
    assert_eq!(account_window_query_slots().available_permits(), 2);
}

#[test]
fn concurrent_writes_cannot_mix_account_window_snapshots() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snapshot.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    schema(&conn);
    conn.execute_batch(
        "INSERT INTO request_logs VALUES (1, 'a', 1700000000000, 0, 0, 0, 0, 0, 'other');
        INSERT INTO request_logs VALUES (2, 'b', 1700000000000, 0, 0, 0, 0, 0, 'other');",
    )
    .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    let worker = {
        let (stop, writes, path) = (stop.clone(), writes.clone(), path.clone());
        std::thread::spawn(move || {
            let conn = Connection::open(path).unwrap();
            while !stop.load(Ordering::Acquire) {
                conn.execute(
                    "UPDATE request_logs SET input_tokens = input_tokens + 1",
                    [],
                )
                .unwrap();
                writes.fetch_add(1, Ordering::Release);
            }
        })
    };
    while writes.load(Ordering::Acquire) == 0 {
        std::thread::yield_now();
    }
    let result = query_local_access_account_window_stats_from_conn(
        &conn,
        (0..1024)
            .map(|index| {
                query(
                    if index % 2 == 0 { "a" } else { "b" },
                    &index.to_string(),
                    0,
                    i64::MAX,
                )
            })
            .collect(),
    );
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
    let rows = result.unwrap();
    assert!(rows
        .iter()
        .all(|row| row.input_tokens == rows[0].input_tokens));
    assert!(writes.load(Ordering::Acquire) > 0);
}
