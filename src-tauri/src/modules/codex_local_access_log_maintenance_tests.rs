use super::*;

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    legacy_schema(&conn);
    conn
}

fn legacy_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE request_logs (
        id INTEGER PRIMARY KEY, success INTEGER NOT NULL,
        error_category TEXT NOT NULL, error_message TEXT NOT NULL);",
    )
    .unwrap();
}

fn seed(conn: &Connection, count: usize) {
    for index in 1..=count {
        conn.execute("INSERT INTO request_logs VALUES (?1, 0, 'request_failed', 'closed before response.completed')", [index as i64]).unwrap();
    }
}

fn cursor(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT cursor FROM request_log_maintenance WHERE name = 'error_category'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn historical_categories_are_batched_checkpointed_and_idempotent() {
    let conn = database();
    seed(&conn, ERROR_CATEGORY_MAINTENANCE_BATCH as usize + 2);
    let first = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(!first.complete);
    assert_eq!(first.visited, ERROR_CATEGORY_MAINTENANCE_BATCH as usize);
    assert_eq!(first.changed, ERROR_CATEGORY_MAINTENANCE_BATCH as usize);
    assert_eq!(cursor(&conn), ERROR_CATEGORY_MAINTENANCE_BATCH);
    let second = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(second.complete);
    assert_eq!(second.changed, 2);
    let third = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(third.complete);
    assert_eq!(third.visited, 0);
    assert_eq!(third.changed, 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        ERROR_CATEGORY_MAINTENANCE_BATCH + 2
    );
}

#[test]
fn failed_batch_rolls_back_mutations_and_checkpoint_then_retries() {
    let conn = database();
    seed(&conn, ERROR_CATEGORY_MAINTENANCE_BATCH as usize + 2);
    maintain_request_log_error_category_batch(&conn).unwrap();
    conn.execute_batch(&format!(
        "CREATE TRIGGER reject_category BEFORE UPDATE OF error_category ON request_logs
        WHEN NEW.id = {} BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
        ERROR_CATEGORY_MAINTENANCE_BATCH + 2
    ))
    .unwrap();
    assert!(maintain_request_log_error_category_batch(&conn)
        .unwrap_err()
        .contains("synthetic failure"));
    assert_eq!(cursor(&conn), ERROR_CATEGORY_MAINTENANCE_BATCH);
    assert_eq!(
        conn.query_row(
            "SELECT error_category FROM request_logs WHERE id = ?1",
            [ERROR_CATEGORY_MAINTENANCE_BATCH + 1],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "request_failed"
    );
    conn.execute_batch("DROP TRIGGER reject_category").unwrap();
    let retried = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(retried.complete);
    assert_eq!(retried.changed, 2);
}

#[test]
fn timed_out_maintenance_rolls_back_and_can_resume() {
    let conn = database();
    seed(&conn, 1);
    conn.execute_batch("CREATE TRIGGER slow_category BEFORE UPDATE OF error_category ON request_logs BEGIN
        SELECT (WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x < 1000000000) SELECT SUM(x) FROM n);
        END;").unwrap();
    let started = Instant::now();
    assert!(maintain_request_log_error_category_batch(&conn)
        .unwrap_err()
        .contains("interrupted"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(conn.is_autocommit());
    assert_eq!(
        conn.query_row("SELECT error_category FROM request_logs", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "request_failed"
    );
    conn.execute_batch("DROP TRIGGER slow_category").unwrap();
    let result = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(result.complete);
    assert_eq!(result.changed, 1);
}

#[test]
fn maintenance_resumes_after_reopen_and_keeps_initial_high_water() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resume.sqlite");
    let conn = Connection::open(&path).unwrap();
    legacy_schema(&conn);
    seed(&conn, ERROR_CATEGORY_MAINTENANCE_BATCH as usize + 2);
    maintain_request_log_error_category_batch(&conn).unwrap();
    conn.execute(
        "INSERT INTO request_logs VALUES (999, 0, 'new-row', 'last_event=response.failed')",
        [],
    )
    .unwrap();
    drop(conn);
    let conn = Connection::open(path).unwrap();
    let result = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(result.complete);
    assert_eq!(result.changed, 2);
    assert_eq!(cursor(&conn), ERROR_CATEGORY_MAINTENANCE_BATCH + 2);
    assert_eq!(
        conn.query_row(
            "SELECT error_category FROM request_logs WHERE id = 999",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "new-row"
    );
}

#[test]
fn category_precedence_success_and_oversized_messages_are_preserved() {
    let conn = database();
    let rows = [
        (
            1,
            true,
            "kept-success",
            "last_event=response.failed".to_string(),
        ),
        (
            2,
            false,
            "other",
            "closed before response.completed last_event=response.failed".into(),
        ),
        (
            3,
            false,
            "upstream_response_failed",
            "incomplete_eof".into(),
        ),
        (4, false, "other", "incomplete_eof".into()),
        (
            5,
            false,
            "unknown",
            format!(
                "{}incomplete_eof",
                "x".repeat(ERROR_CATEGORY_MESSAGE_LIMIT as usize)
            ),
        ),
    ];
    for (id, success, category, message) in rows {
        conn.execute(
            "INSERT INTO request_logs VALUES (?1, ?2, ?3, ?4)",
            params![id, success, category, message],
        )
        .unwrap();
    }
    let result = maintain_request_log_error_category_batch(&conn).unwrap();
    assert!(result.complete);
    assert_eq!(result.changed, 2);
    assert_eq!(result.oversized, 1);
    let categories = conn
        .prepare("SELECT error_category FROM request_logs ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        categories,
        vec![
            "kept-success",
            "upstream_response_failed",
            "upstream_response_failed",
            "stream_incomplete",
            "unknown"
        ]
    );
}

#[test]
fn clear_between_batches_never_recreates_deleted_rows() {
    let conn = database();
    seed(&conn, ERROR_CATEGORY_MAINTENANCE_BATCH as usize + 2);
    maintain_request_log_error_category_batch(&conn).unwrap();
    conn.execute("DELETE FROM request_logs", []).unwrap();
    assert!(
        maintain_request_log_error_category_batch(&conn)
            .unwrap()
            .complete
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn completed_future_version_is_never_downgraded() {
    let conn = database();
    seed(&conn, 1);
    maintain_request_log_error_category_batch(&conn).unwrap();
    conn.execute(
        "UPDATE request_log_maintenance SET version = 999, complete = 0",
        [],
    )
    .unwrap();
    assert!(
        maintain_request_log_error_category_batch(&conn)
            .unwrap()
            .complete
    );
    assert_eq!(
        conn.query_row("SELECT version FROM request_log_maintenance", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        999
    );
}

#[test]
fn new_writes_normalize_once_and_opening_database_never_rewrites_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("live.sqlite");
    let conn = open_local_access_logs_db_once(&path, true).unwrap();
    let event = CodexLocalAccessUsageEvent {
        request_id: "new-failure".into(),
        success: false,
        error_category: "request_failed".into(),
        error_message: "closed before response.completed last_event=response.failed".into(),
        ..Default::default()
    };
    insert_local_access_usage_event(&conn, &event).unwrap();
    assert_eq!(
        conn.query_row("SELECT error_category FROM request_logs", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "upstream_response_failed"
    );
    conn.execute("UPDATE request_logs SET error_category = 'legacy'", [])
        .unwrap();
    drop(conn);
    let conn = open_local_access_logs_db_once(&path, true).unwrap();
    assert_eq!(
        conn.query_row("SELECT error_category FROM request_logs", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "legacy"
    );
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

fn decoded_fixture_events(conn: &Connection) -> Vec<CodexLocalAccessUsageEvent> {
    let mut events = Vec::new();
    for_each_local_access_usage_event_since_from_conn(conn, 0, |event| {
        events.push(event);
        Ok(())
    })
    .unwrap();
    events
}

#[test]
fn historical_row_decoding_and_window_counters_are_stable_across_maintenance() {
    let directory = tempfile::tempdir().unwrap();
    let conn =
        open_local_access_logs_db_once(&directory.path().join("decode.sqlite"), true).unwrap();
    let now = now_ms();
    let event = CodexLocalAccessUsageEvent {
        timestamp: now,
        request_id: "legacy-decoding".into(),
        success: false,
        error_category: "request_failed".into(),
        error_message: "last_event=response.failed".into(),
        ..Default::default()
    };
    insert_local_access_usage_event(&conn, &event).unwrap();
    conn.execute(
        "UPDATE request_logs SET error_category = 'request_failed'",
        [],
    )
    .unwrap();
    let before = decoded_fixture_events(&conn);
    let (daily_before, _, _, _) =
        load_stats_windows_and_recent_events_from_conn(&conn, now).unwrap();
    assert_eq!(before[0].error_category, "upstream_response_failed");
    assert_eq!(daily_before.totals.upstream_response_failed_count, 1);
    assert!(
        maintain_request_log_error_category_batch(&conn)
            .unwrap()
            .complete
    );
    let after = decoded_fixture_events(&conn);
    let (daily_after, _, _, _) =
        load_stats_windows_and_recent_events_from_conn(&conn, now).unwrap();
    assert_eq!(after[0].error_category, before[0].error_category);
    assert_eq!(
        daily_after.totals.upstream_response_failed_count,
        daily_before.totals.upstream_response_failed_count
    );
}

#[test]
fn live_error_category_has_identical_runtime_and_database_counts() {
    let directory = tempfile::tempdir().unwrap();
    let conn =
        open_local_access_logs_db_once(&directory.path().join("runtime-counts.sqlite"), true)
            .unwrap();
    let mut stats = empty_stats_snapshot();
    for (request, message, expected) in [
        (
            "live-upstream",
            "incomplete_eof last_event=response.failed",
            "upstream_response_failed",
        ),
        ("live-stream", "incomplete_eof", "stream_incomplete"),
    ] {
        let mut event = CodexLocalAccessUsageEvent {
            timestamp: now_ms(),
            request_id: request.into(),
            success: false,
            error_category: "request_failed".into(),
            error_message: message.into(),
            ..Default::default()
        };
        // The ingestion boundary and row decoder share this bounded rule;
        // exercise real aggregation and the production writer against it.
        normalize_request_log_event_category(&mut event);
        assert_eq!(event.error_category, expected);
        apply_usage_event_to_stats(&mut stats, &event);
        insert_local_access_usage_event(&conn, &event).unwrap();
    }
    let mut decoded_stats = empty_stats_snapshot();
    for event in decoded_fixture_events(&conn) {
        apply_usage_event_to_stats(&mut decoded_stats, &event);
    }
    assert_eq!(stats.totals.upstream_response_failed_count, 1);
    assert_eq!(stats.totals.stream_incomplete_count, 1);
    assert_eq!(
        decoded_stats.totals.upstream_response_failed_count,
        stats.totals.upstream_response_failed_count
    );
    assert_eq!(
        decoded_stats.totals.stream_incomplete_count,
        stats.totals.stream_incomplete_count
    );
}

#[test]
fn json_snapshot_fallback_uses_same_bounded_classification() {
    let now = now_ms();
    let mut stats = empty_stats_snapshot();
    stats.totals.upstream_response_failed_count = 77;
    let raw = CodexLocalAccessUsageEvent {
        timestamp: now,
        request_id: "snapshot".into(),
        success: false,
        error_category: "request_failed".into(),
        error_message: "incomplete_eof".into(),
        ..Default::default()
    };
    apply_snapshot_usage_event_to_current_windows(&mut stats, raw, now);
    assert_eq!(stats.daily.totals.stream_incomplete_count, 1);
    assert_eq!(stats.weekly.totals.stream_incomplete_count, 1);
    assert_eq!(stats.monthly.totals.stream_incomplete_count, 1);
    assert_eq!(stats.events[0].error_category, "stream_incomplete");
    assert_eq!(
        stats.totals.upstream_response_failed_count, 77,
        "fallback must not rewrite lifetime counters"
    );
    let oversized = CodexLocalAccessUsageEvent {
        timestamp: now,
        request_id: "oversized-snapshot".into(),
        success: false,
        error_category: "preserved".into(),
        error_message: format!(
            "incomplete_eof{}",
            "x".repeat(ERROR_CATEGORY_MESSAGE_LIMIT as usize)
        ),
        ..Default::default()
    };
    apply_snapshot_usage_event_to_current_windows(&mut stats, oversized, now);
    assert_eq!(stats.daily.totals.stream_incomplete_count, 1);
    assert_eq!(stats.events[1].error_category, "preserved");
}
