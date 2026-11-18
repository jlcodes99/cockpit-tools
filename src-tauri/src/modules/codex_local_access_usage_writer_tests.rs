use super::*;

fn logs() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_local_access_logs_db_once(&dir.path().join("logs.sqlite"), true).unwrap();
    ensure_remote_analytics_schema(&conn).unwrap();
    (dir, conn)
}
fn write(id: &str, epoch: u64) -> UsageLogWrite {
    let event = CodexLocalAccessUsageEvent {
        timestamp: REMOTE_HOUR,
        request_id: id.into(),
        api_key_id: "writer-key".into(),
        requested_model: "public".into(),
        success: true,
        total_tokens: 12,
        budget_settlement: Some(
            crate::models::codex_local_access::CodexRequestBudgetSettlement {
                amount_microusd: 123,
                metered: true,
                settled_at: REMOTE_HOUR,
            },
        ),
        ..Default::default()
    };
    UsageLogWrite {
        bytes: usage_log_encoded_bytes(&event).unwrap(),
        event,
        epoch,
        official: None,
    }
}

fn finish_writes(conn: &Connection, batch: &[UsageLogWrite], epoch: u64) {
    let mut cursor = 0;
    let mut retries = 0;
    while cursor < batch.len() {
        let consumed = match write_usage_log_batch(conn, &batch[cursor..], epoch) {
            Ok(consumed) => consumed,
            Err(error) if error.to_lowercase().contains("interrupted") && retries < 8 => {
                retries += 1;
                continue;
            }
            Err(error) => panic!("batch failed: {error}"),
        };
        assert!(consumed > 0 && consumed <= batch.len() - cursor);
        cursor += consumed;
    }
}

#[test]
fn committed_usage_is_visible_in_the_account_window_at_millisecond_precision() {
    let (dir, conn) = logs();
    let second = 1_800_000_000_000;
    let mut latest = write("latest", 1);
    latest.event.account_id = "account-a".into();
    latest.event.timestamp = second + 431;
    latest.event.estimated_cost_usd = 0.125;
    let mut other = write("other", 1);
    other.event.account_id = "account-b".into();
    other.event.timestamp = latest.event.timestamp;
    finish_writes(&conn, &[latest, other], 1);

    let database = dir.path().join("logs.sqlite");
    let read = |end_at| {
        query_local_access_account_window_stats_at_path(
            &database,
            vec![CodexLocalAccessAccountWindowQuery {
                account_id: "account-a".into(),
                window_key: "primary".into(),
                start_at: second,
                end_at,
            }],
        )
        .unwrap()
    };
    assert_eq!(read(second)[0].request_count, 0);
    let stats = read(second + 431);
    assert_eq!(stats[0].request_count, 1);
    assert_eq!(stats[0].total_tokens, 12);
    assert_eq!(stats[0].estimated_cost_usd, 0.125);
}

#[test]
fn summary_transaction_rollback_never_commits_later_rows_outside_the_batch() {
    let (_dir, conn) = logs();
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    conn.execute_batch("CREATE TRIGGER rollback_summary BEFORE INSERT ON request_display_summaries BEGIN SELECT RAISE(ROLLBACK,'interrupted summary'); END;").unwrap();
    let batch = vec![write("one", 1), write("two", 1)];
    assert!(write_usage_log_batch(&conn, &batch, 1).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER rollback_summary;")
        .unwrap();
    finish_writes(&conn, &batch, 1);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT SUM(settled_microusd) FROM request_logs", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        246
    );
}

#[test]
fn settlement_and_raw_log_roll_back_together_and_retry_exactly_once() {
    let (_dir, conn) = logs();
    conn.execute_batch(
        "CREATE TRIGGER fail_charge BEFORE UPDATE OF settled_microusd ON request_logs
        BEGIN SELECT RAISE(ABORT,'storage failure'); END;",
    )
    .unwrap();
    let item = write("one", 1);
    assert!(insert_live_local_access_usage_event(&conn, &item.event).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        query_remote_key_stats(&conn, "writer-key", 0, 2 * REMOTE_HOUR).unwrap()["summary"]
            ["requestCount"],
        0
    );
    conn.execute_batch("DROP TRIGGER fail_charge;").unwrap();
    insert_live_local_access_usage_event(&conn, &item.event).unwrap();
    insert_live_local_access_usage_event(&conn, &item.event).unwrap();
    let stats = query_remote_key_stats(&conn, "writer-key", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 1);
    assert_eq!(stats["summary"]["settledMicrousd"], 123);
}

#[test]
fn batch_failure_preserves_cursor_and_clear_epoch_does_not_resurrect_old_requests() {
    let (_dir, conn) = logs();
    conn.execute_batch(
        "CREATE TRIGGER fail_second BEFORE INSERT ON request_logs WHEN NEW.request_id='one'
        BEGIN SELECT RAISE(ABORT,'storage failure'); END;",
    )
    .unwrap();
    let batch = vec![write("one", 2), write("two", 2)];
    assert!(write_usage_log_batch(&conn, &batch, 2).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER fail_second;").unwrap();
    // A batch may yield after any row when its time budget expires. Continue
    // from the returned cursor, exactly as the production writer does.
    finish_writes(&conn, &batch, 2);
    finish_writes(&conn, &batch, 2);
    conn.execute("DELETE FROM request_logs", []).unwrap();
    finish_writes(&conn, &batch, 3);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        write_usage_log_batch(&conn, &[write("new", 3)], 3).unwrap(),
        1
    );
    let stats = query_remote_key_stats(&conn, "writer-key", 0, 2 * REMOTE_HOUR).unwrap();
    assert_eq!(stats["summary"]["requestCount"], 1);
    assert_eq!(stats["summary"]["settledMicrousd"], 123);
}

#[tokio::test]
async fn bounded_queue_applies_backpressure_without_starting_tasks_or_blocking_runtime() {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
    sender.try_send(write("one", 1)).unwrap();
    sender.try_send(write("two", 1)).unwrap();
    assert!(sender.try_send(write("three", 1)).is_err());
    let send = sender.send(write("three", 1));
    assert!(tokio::time::timeout(Duration::from_millis(10), send)
        .await
        .is_err());
    assert_eq!(receiver.recv().await.unwrap().event.request_id, "one");
    assert_eq!(receiver.recv().await.unwrap().event.request_id, "two");
    assert!(receiver.try_recv().is_err());
}

#[test]
fn one_oversized_record_has_a_byte_boundary_before_it_enters_the_queue() {
    let mut item = write("one", 1);
    item.event.error_message = "x".repeat(USAGE_WRITER_ROW_BYTES + 1);
    assert!(usage_log_encoded_bytes(&item.event).is_err());
    assert!(USAGE_WRITER_CAPACITY * USAGE_WRITER_ROW_BYTES <= 32 * 1024 * 1024);
    assert!(USAGE_WRITER_BATCH_BYTES <= 512 * 1024);
}

#[test]
fn slow_sql_batch_is_interrupted_and_same_rows_can_be_retried_without_partial_charge() {
    let (_dir, conn) = logs();
    conn.execute_batch("CREATE TRIGGER slow_usage BEFORE INSERT ON request_logs BEGIN
        SELECT SUM(x) FROM (WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000000) SELECT x FROM n);
        END;").unwrap();
    let rows = vec![write("slow", 1)];
    let started = Instant::now();
    let budget_expired = AtomicBool::new(false);
    assert!(write_usage_log_batch_limited(&conn, &rows, 1, 1, &budget_expired).is_err());
    assert!(budget_expired.load(Ordering::Relaxed));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER slow_usage").unwrap();
    finish_writes(&conn, &rows, 1);
    assert_eq!(
        query_remote_key_stats(&conn, "writer-key", 0, 2 * REMOTE_HOUR).unwrap()["summary"]
            ["settledMicrousd"],
        123
    );
}

#[test]
fn monthly_token_alerts_use_committed_and_deduplicated_gateway_usage() {
    let (_dir, conn) = logs();
    ensure_monthly_token_alerts_schema(&conn).unwrap();
    save_monthly_token_threshold(&conn, "writer-key", 24).unwrap();
    let now = 1_800_000_000_431;
    let mut first = write("monthly-one", 1);
    first.event.timestamp = now;
    let mut second = write("monthly-two", 1);
    second.event.timestamp = now;
    let mut other = write("monthly-other", 1);
    other.event.timestamp = now;
    other.event.api_key_id = "another-key".into();
    other.event.total_tokens = 10_000;
    let keys = [("writer-key".to_owned(), "Writer key".to_owned())]
        .into_iter()
        .collect();
    let persist = |writes: &[&UsageLogWrite]| {
        let tx = conn.unchecked_transaction().unwrap();
        for write in writes {
            insert_live_usage_event_in_transaction(&tx, &write.event, None).unwrap();
        }
        tx.commit().unwrap();
    };
    persist(&[&other, &first]);
    persist(&[&first]);
    assert_eq!(monthly_token_alert(&conn, "writer-key", now).unwrap().used, 12);
    assert!(claim_monthly_token_alerts(&conn, &keys, now).unwrap().1.is_empty());
    persist(&[&second]);
    let notices = claim_monthly_token_alerts(&conn, &keys, now).unwrap().1;
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].used, 24);
}
