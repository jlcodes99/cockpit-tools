use super::*;

const SUMMARY_THREAD: &str = "019e92aa-0000-7000-8000-000000000001";

fn logs() -> (tempfile::TempDir, Connection) {
    let directory = tempfile::tempdir().unwrap();
    let conn = open_local_access_logs_db_once(&directory.path().join("logs.sqlite"), true).unwrap();
    (directory, conn)
}

fn usage(mode: CodexLocalAccessGatewayMode) -> CodexLocalAccessUsageEvent {
    CodexLocalAccessUsageEvent {
        timestamp: 1000,
        request_id: "summary-live-request".into(),
        api_key_id: "summary-key".into(),
        client_instance_id: "summary-instance".into(),
        model_id: "gpt-model".into(),
        gateway_mode: Some(mode),
        success: true,
        http_status: Some(200),
        input_tokens: 100,
        output_tokens: 300,
        total_tokens: 400,
        output_generation_ms: Some(2000),
        ..Default::default()
    }
}

fn detail() -> CodexLocalAccessRequestDetail {
    CodexLocalAccessRequestDetail {
        request_id: "summary-live-request".into(),
        captured_at_ms: 1001,
        client_identity_captured: true,
        client_thread_id: Some(SUMMARY_THREAD.into()),
        client_identity_metadata_captured: true,
        client_installation_id: Some("summary-installation".into()),
        attempts: vec![CodexLocalAccessRequestAttempt {
            sequence: 1,
            account_id: "summary-final-account".into(),
            account_email: Some("final@example.test".into()),
            transport: "http".into(),
            status: Some(200),
            success: true,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM request_display_summaries",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn live_native_and_sidecar_writer_hooks_cover_both_event_orders() {
    let epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
    let payload_epoch = REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst);
    for mode in [
        CodexLocalAccessGatewayMode::Native,
        CodexLocalAccessGatewayMode::Sidecar,
    ] {
        for diagnostic_first in [true, false] {
            let (_directory, conn) = logs();
            if diagnostic_first {
                apply_request_diagnostic_write(
                    &conn,
                    RequestDiagnosticWrite::Detail(Box::new(detail()), payload_epoch, epoch),
                )
                .unwrap();
            }
            apply_request_diagnostic_write(
                &conn,
                RequestDiagnosticWrite::Usage(Box::new(usage(mode)), epoch),
            )
            .unwrap();
            if !diagnostic_first {
                apply_request_diagnostic_write(
                    &conn,
                    RequestDiagnosticWrite::Detail(Box::new(detail()), payload_epoch, epoch),
                )
                .unwrap();
            }
            assert_eq!(count(&conn), 1);
            let (thread, encoded): (String, String) = conn
                .query_row(
                    "SELECT client_thread_id,data_json FROM request_display_summaries",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(thread, SUMMARY_THREAD);
            let value: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(value["accountEmail"], "final@example.test");
            assert_eq!(value["outputGenerationMs"], 2000);
            prune_request_diagnostics(&conn, 1001 + REQUEST_DIAGNOSTICS_RETENTION_MS + 1).unwrap();
            assert_eq!(
                count(&conn),
                1,
                "diagnostic retention must preserve request summaries"
            );
        }
    }
}

#[test]
fn historical_insert_path_does_not_create_or_backfill_summary_rows() {
    let (_directory, conn) = logs();
    let event = usage(CodexLocalAccessGatewayMode::Sidecar);
    let tx = conn.unchecked_transaction().unwrap();
    insert_local_access_usage_event(&tx, &event).unwrap();
    tx.commit().unwrap();
    assert!(!conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='request_display_summaries')",
            [],
            |row| row.get::<_, bool>(0)
        )
        .unwrap());
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    assert_eq!(count(&conn), 0);
}

#[test]
fn cleared_epochs_drop_usage_and_diagnostic_writes_without_reviving_summaries() {
    let (_directory, conn) = logs();
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    let stale = REQUEST_LOG_WRITE_EPOCH
        .load(Ordering::SeqCst)
        .wrapping_sub(1);
    let payload = REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst);
    apply_request_diagnostic_write(
        &conn,
        RequestDiagnosticWrite::Usage(Box::new(usage(CodexLocalAccessGatewayMode::Native)), stale),
    )
    .unwrap();
    apply_request_diagnostic_write(
        &conn,
        RequestDiagnosticWrite::Detail(Box::new(detail()), payload, stale),
    )
    .unwrap();
    assert_eq!(count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn failed_usage_does_not_publish_summary_and_failed_summary_keeps_usage_retryable() {
    let (_directory, conn) = logs();
    let event = usage(CodexLocalAccessGatewayMode::Native);
    conn.execute_batch(
        "CREATE TRIGGER fail_usage BEFORE INSERT ON request_logs
        BEGIN SELECT RAISE(FAIL,'simulated usage failure'); END;",
    )
    .unwrap();
    assert!(insert_live_local_access_usage_event(&conn, &event).is_err());
    assert!(!conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='request_display_summaries')",
            [],
            |row| row.get::<_, bool>(0)
        )
        .unwrap());
    conn.execute("DROP TRIGGER fail_usage", []).unwrap();
    crate::modules::codex_request_summary::ensure_schema(&conn).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_summary BEFORE INSERT ON request_display_summaries
        BEGIN SELECT RAISE(FAIL,'simulated projection failure'); END;",
    )
    .unwrap();
    insert_live_local_access_usage_event(&conn, &event).unwrap();
    assert_eq!(count(&conn), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    conn.execute("DROP TRIGGER fail_summary", []).unwrap();
    let epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
    apply_request_diagnostic_write(
        &conn,
        RequestDiagnosticWrite::Detail(
            Box::new(detail()),
            REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst),
            epoch,
        ),
    )
    .unwrap();
    assert_eq!(count(&conn), 1);
}
