mod request_diagnostics_tests {
    use super::super::*;

    #[test]
    fn request_payload_choice_survives_inflight_collection_flush_and_reload() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-request-payload-setting-{}", uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("collection.json");
        for (initial, enabled) in [(true, false), (false, true)] {
            // Set up the exact full collection snapshot captured by the stats worker.
            let mut collection = new_empty_local_access_collection().unwrap();
            collection.request_payload_logging = initial;
            write_string_atomic(&path, &serde_json::to_string(&collection).unwrap()).unwrap();
            let (captured, captured_rx) = std::sync::mpsc::channel();
            let (resume, resume_rx) = std::sync::mpsc::channel();
            let worker_path = path.clone();
            let worker = std::thread::spawn(move || {
                let mut snapshot: CodexLocalAccessCollection =
                    serde_json::from_slice(&fs::read(&worker_path).unwrap()).unwrap();
                snapshot.debug_logs = false;
                captured.send(()).unwrap();
                resume_rx.recv().unwrap();
                save_collection_to_path(&worker_path, &snapshot).unwrap();
                snapshot
            });
            captured_rx.recv().unwrap();
            persist_request_payload_logging_at_path(&path, enabled).unwrap();
            resume.send(()).unwrap();
            let mut late_snapshot = worker.join().unwrap();
            let reloaded: CodexLocalAccessCollection =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(reloaded.request_payload_logging, enabled);
            assert!(!reloaded.debug_logs, "unrelated settings must still be saved");
            // The same stale writer may publish its snapshot back into the runtime.
            preserve_runtime_collection_fields(&mut late_snapshot, &reloaded);
            assert_eq!(late_snapshot.request_payload_logging, enabled);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn request_payload_choice_merges_latest_configuration_and_preserves_failed_writes() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-request-payload-merge-{}", uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("collection.json");
        let mut collection = new_empty_local_access_collection().unwrap();
        collection.account_ids = vec!["newly-added-account".into()];
        collection.request_payload_logging = true;
        save_collection_to_path(&path, &collection).unwrap();
        persist_request_payload_logging_at_path(&path, false).unwrap();
        let current: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(current.account_ids, collection.account_ids);
        assert!(!current.request_payload_logging);

        fs::write(&path, "invalid-config").unwrap();
        assert!(persist_request_payload_logging_at_path(&path, true).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid-config");
        // Recovery with an existing valid snapshot remains available after a failed toggle.
        let published_choice = REQUEST_PAYLOAD_LOGGING_ENABLED.load(Ordering::SeqCst);
        save_collection_to_path(&path, &collection).unwrap();
        let restored: CodexLocalAccessCollection =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.account_ids, collection.account_ids);
        assert_eq!(restored.request_payload_logging, published_choice);
        let missing = directory.join("missing.json");
        assert!(persist_request_payload_logging_at_path(&missing, true).is_err());
        assert!(!missing.exists(), "a failed toggle must not create a collection");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn payload_setting_sync_status_does_not_reuse_an_older_failure() {
        let failure = (3, "connection refused".to_string());
        let current = request_payload_logging_status_for(3, false, Some(&failure));
        assert_eq!(current.error.as_deref(), Some("connection refused"));
        assert!(!current.pending);
        let retried = request_payload_logging_status_for(4, true, Some(&failure));
        assert_eq!(retried.error, None);
        assert!(retried.pending);
    }

    fn diagnostic(request_id: &str, captured_at_ms: i64) -> CodexLocalAccessRequestDetail {
        CodexLocalAccessRequestDetail {
            request_id: request_id.into(), first_response_ms: Some(125), captured_at_ms,
            attempts: vec![CodexLocalAccessRequestAttempt {
                sequence: 1, account_id: "account-a".into(), model_id: "gpt-5.5".into(),
                transport: "sse".into(), started_at_ms: captured_at_ms - 200, latency_ms: 200,
                status: Some(200), success: true, ..Default::default()
            }],
            payloads: vec![CodexLocalAccessRequestPayload {
                stage: "client".into(), transport: "http".into(), content_type: "application/json".into(),
                body: r#"{"model":"gpt-5.5","api_key":"secret","messages":[{"content":"private user prompt","refresh_token":"refresh-secret"}]}"#.into(),
                headers: HashMap::from([("authorization".into(),"Bearer secret".into()),("content-type".into(),"application/json".into())]),
                original_bytes: 123, ..Default::default()
            }], ..Default::default()
        }
    }

    fn memory_logs() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        create_request_logs_table(&conn, true).unwrap();
        create_request_diagnostics_table(&conn).unwrap();
        conn
    }

    #[test]
    fn request_diagnostics_legacy_records_remain_unrecorded() {
        let event: CodexLocalAccessUsageEvent = serde_json::from_str("{}").unwrap();
        assert_eq!(event.first_response_ms, None);
        let conn = memory_logs();
        insert_local_access_usage_event(
            &conn,
            &CodexLocalAccessUsageEvent {
                timestamp: 1,
                request_id: "old-request".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(read_request_diagnostic(&conn, "old-request")
            .unwrap()
            .is_none());
        let stored = conn
            .query_row("SELECT * FROM request_logs", [], usage_event_from_row)
            .unwrap();
        assert!(stored.first_response_ms.is_none());
        let legacy = Connection::open_in_memory().unwrap();
        legacy
            .execute_batch("CREATE TABLE request_logs(request_id TEXT)")
            .unwrap();
        assert_eq!(
            request_logs_first_response_select(&legacy).unwrap(),
            "NULL AS first_response_ms"
        );
    }

    #[test]
    fn request_diagnostics_late_detail_updates_metadata_without_usage_duplicates() {
        for diagnostic_first in [true, false] {
            let conn = memory_logs();
            let event = CodexLocalAccessUsageEvent {
                timestamp: 10,
                request_id: "late".into(),
                input_tokens: 12,
                output_tokens: 3,
                total_tokens: 15,
                ..Default::default()
            };
            if diagnostic_first {
                insert_request_diagnostic(&conn, diagnostic("late", 11)).unwrap();
            }
            insert_local_access_usage_event(&conn, &event).unwrap();
            insert_request_diagnostic(&conn, diagnostic("late", 12)).unwrap();
            insert_request_diagnostic(&conn, diagnostic("late", 12)).unwrap();
            let (count, tokens, first): (i64, i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*),SUM(total_tokens),first_response_ms FROM request_logs",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!((count, tokens, first), (1, 15, 125));
            let detail = read_request_diagnostic(&conn, "late").unwrap().unwrap();
            assert_eq!(detail.attempts.len(), 1);
            assert_eq!(detail.payloads.len(), 1);
        }
    }

    #[test]
    fn request_diagnostics_out_of_order_attempts_preserve_latest_terminal_state() {
        let conn = memory_logs();
        let mut latest = diagnostic("ordered", 20);
        latest.attempts[0].sequence = 2;
        insert_request_diagnostic(&conn, latest).unwrap();
        let mut old = diagnostic("ordered", 10);
        old.failure_phase = Some("connect".into());
        old.attempts[0].success = false;
        old.attempts[0].status = Some(429);
        insert_request_diagnostic(&conn, old).unwrap();
        let detail = read_request_diagnostic(&conn, "ordered").unwrap().unwrap();
        assert_eq!(detail.captured_at_ms, 20);
        assert_eq!(detail.failure_phase, None);
        assert_eq!(
            detail
                .attempts
                .iter()
                .map(|attempt| attempt.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(detail.attempts[1].success);
    }

    #[test]
    fn request_diagnostics_redacts_named_fields_and_headers_but_preserves_prompt() {
        let normalized = normalize_request_diagnostics(diagnostic("redaction", 1));
        let payload = &normalized.payloads[0];
        let json: Value = serde_json::from_str(&payload.body).unwrap();
        assert_eq!(json["api_key"], "[REDACTED]");
        assert_eq!(json["messages"][0]["refresh_token"], "[REDACTED]");
        assert_eq!(json["messages"][0]["content"], "private user prompt");
        assert!(!payload.headers.contains_key("authorization"));
        assert_eq!(payload.headers.len(), 1);
        assert_eq!(
            payload.sha256,
            format!("{:x}", Sha256::digest(payload.body.as_bytes()))
        );
        assert_ne!(
            payload.sha256,
            format!(
                "{:x}",
                Sha256::digest(diagnostic("redaction", 1).payloads[0].body.as_bytes())
            )
        );
    }

    #[test]
    fn request_diagnostics_snapshot_and_trace_size_limits_are_explicit() {
        let mut detail = diagnostic("bounded", 1);
        let payload = CodexLocalAccessRequestPayload {
            stage: "upstream".into(),
            body: json!({"input":"汉".repeat(20000)}).to_string(),
            ..Default::default()
        };
        detail.payloads = (0..12)
            .map(|index| CodexLocalAccessRequestPayload {
                attempt_sequence: Some(index),
                ..payload.clone()
            })
            .collect();
        detail.attempts = (1..=45)
            .map(|sequence| CodexLocalAccessRequestAttempt {
                sequence,
                ..Default::default()
            })
            .collect();
        let normalized = normalize_request_diagnostics(detail);
        assert_eq!(normalized.attempts.len(), REQUEST_DIAGNOSTICS_MAX_ATTEMPTS);
        assert_eq!(normalized.payloads.len(), REQUEST_DIAGNOSTICS_MAX_PAYLOADS);
        assert!(normalized.truncated);
        assert!(normalized
            .payloads
            .iter()
            .all(|payload| payload.body.len() <= REQUEST_DIAGNOSTICS_PAYLOAD_BYTES));
        assert!(
            normalized
                .payloads
                .iter()
                .map(|payload| payload.body.len())
                .sum::<usize>()
                <= REQUEST_DIAGNOSTICS_REQUEST_BYTES
        );
    }

    #[test]
    fn request_diagnostics_rejects_non_json_body_when_not_pre_redacted_truncation() {
        let mut detail = diagnostic("binary", 1);
        detail.payloads[0].body = "refresh_token=secret".into();
        let detail = normalize_request_diagnostics(detail);
        assert!(detail.payloads[0].truncated);
        assert_eq!(detail.payloads[0].body, "[omitted: non-JSON request body]");
    }

    #[test]
    fn request_diagnostics_clear_bodies_keeps_attempts_first_response_and_tokens() {
        let conn = memory_logs();
        insert_request_diagnostic(&conn, diagnostic("clear", 1)).unwrap();
        insert_local_access_usage_event(
            &conn,
            &CodexLocalAccessUsageEvent {
                timestamp: 1,
                request_id: "clear".into(),
                total_tokens: 123,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(clear_request_payloads_from_db(&conn).unwrap(), 1);
        assert_eq!(clear_request_payloads_from_db(&conn).unwrap(), 0);
        let detail = read_request_diagnostic(&conn, "clear").unwrap().unwrap();
        assert!(detail.payloads.is_empty());
        assert_eq!(detail.attempts.len(), 1);
        assert_eq!(detail.first_response_ms, Some(125));
        let tokens: i64 = conn
            .query_row("SELECT total_tokens FROM request_logs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(tokens, 123);
    }

    #[test]
    fn request_diagnostics_prunes_expired_rows_in_bounded_batches() {
        let conn = memory_logs();
        for index in 0..300 {
            insert_request_diagnostic(&conn, diagnostic(&format!("old-{index}"), 1)).unwrap();
        }
        insert_request_diagnostic(
            &conn,
            diagnostic("recent", REQUEST_DIAGNOSTICS_RETENTION_MS),
        )
        .unwrap();
        assert_eq!(
            prune_request_diagnostics(&conn, REQUEST_DIAGNOSTICS_RETENTION_MS + 2).unwrap(),
            REQUEST_DIAGNOSTICS_PRUNE_BATCH as usize
        );
        assert!(read_request_diagnostic(&conn, "recent").unwrap().is_some());
        assert_eq!(
            prune_request_diagnostics(&conn, REQUEST_DIAGNOSTICS_RETENTION_MS + 2).unwrap(),
            44
        );
    }

    #[test]
    fn request_diagnostics_global_capacity_evicts_body_before_trace() {
        let conn = memory_logs();
        let now = now_ms();
        insert_request_diagnostic(&conn, diagnostic("old-body", now - 1)).unwrap();
        insert_request_diagnostic(&conn, diagnostic("new-body", now)).unwrap();
        conn.execute(
            "UPDATE request_diagnostics SET payload_bytes=?1 WHERE request_id='old-body'",
            params![REQUEST_DIAGNOSTICS_TOTAL_PAYLOAD_BYTES],
        )
        .unwrap();
        prune_request_diagnostics(&conn, now).unwrap();
        let old = read_request_diagnostic(&conn, "old-body").unwrap().unwrap();
        assert!(old.payloads.is_empty());
        assert_eq!(old.attempts.len(), 1);
        assert!(old.truncated);
        assert!(!read_request_diagnostic(&conn, "new-body")
            .unwrap()
            .unwrap()
            .payloads
            .is_empty());
    }

    #[test]
    fn request_diagnostics_full_queue_does_not_wait_for_slow_writer() {
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        let write = || RequestDiagnosticWrite::Detail(Box::new(diagnostic("queue", 1)), 0, 0);
        assert!(try_queue_request_diagnostic_to(&sender, write()));
        let before = Instant::now();
        assert!(!try_queue_request_diagnostic_to(&sender, write()));
        assert!(before.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn request_diagnostics_cleared_epochs_drop_queued_bodies_and_old_events() {
        let conn = memory_logs();
        let payload_epoch = REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst);
        let log_epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
        apply_request_diagnostic_write(
            &conn,
            RequestDiagnosticWrite::Detail(
                Box::new(diagnostic("queued", 1)),
                payload_epoch.wrapping_sub(1),
                log_epoch,
            ),
        )
        .unwrap();
        assert!(read_request_diagnostic(&conn, "queued")
            .unwrap()
            .unwrap()
            .payloads
            .is_empty());
        apply_request_diagnostic_write(
            &conn,
            RequestDiagnosticWrite::Usage(
                Box::new(CodexLocalAccessUsageEvent {
                    timestamp: 1,
                    request_id: "stale".into(),
                    ..Default::default()
                }),
                log_epoch.wrapping_sub(1),
            ),
        )
        .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
