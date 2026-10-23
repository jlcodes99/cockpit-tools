mod request_diagnostics_tests {
    use super::super::*;

    #[test]
    fn request_payload_choice_survives_inflight_collection_flush_and_reload() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-request-payload-setting-{}",
            uuid::Uuid::new_v4()
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
            assert!(
                !reloaded.debug_logs,
                "unrelated settings must still be saved"
            );
            // The same stale writer may publish its snapshot back into the runtime.
            preserve_runtime_collection_fields(&mut late_snapshot, &reloaded);
            assert_eq!(late_snapshot.request_payload_logging, enabled);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn request_payload_choice_merges_latest_configuration_and_preserves_failed_writes() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-request-payload-merge-{}",
            uuid::Uuid::new_v4()
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
        assert!(
            !missing.exists(),
            "a failed toggle must not create a collection"
        );
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

    fn authorization_fallback_attempt() -> CodexLocalAccessRequestAttempt {
        CodexLocalAccessRequestAttempt {
            sequence: 2, account_id: "account-a".into(), model_id: "gpt-5.5".into(),
            authorization_route: Some("local_gateway".into()),
            upstream_request_id: Some("req_local-success".into()),
            authorization_fallback: Some(crate::models::codex_local_access::CodexLocalAccessAuthorizationFallbackTrace {
                reason: "eligibility_rejected".into(), trigger_stage: "upstream".into(), from_account_id: "account-a".into(),
                to_account_id: "account-a".into(), from_route: "official".into(), to_route: "local_gateway".into(),
                status: Some(403), error_code: Some("subscription_sharing_user_not_eligible".into()),
                error_message: Some("The ChatGPT user is not eligible for subscription sharing.".into()),
            }),
            status: Some(200), success: true, ..Default::default()
        }
    }

    #[test]
    fn authorization_fallback_metadata_survives_without_payloads_and_late_snapshots() {
        let conn = memory_logs();
        let mut detail = diagnostic("authorization-fallback", 100);
        detail.payloads.clear();
        detail.attempts[0].authorization_route = Some("official".into());
        detail.attempts[0].success = false;
        detail.attempts[0].status = Some(403);
        detail.attempts[0].error_message = Some("not eligible".into());
        detail.attempts[0].upstream_request_id = Some("req_official-rejected".into());
        detail.attempts.push(authorization_fallback_attempt());
        insert_request_diagnostic(&conn, detail.clone()).unwrap();
        detail.captured_at_ms += 10;
        detail.attempts[1].authorization_fallback = None;
        detail.attempts[1].upstream_request_id = None;
        insert_request_diagnostic(&conn, detail).unwrap();
        let stored = read_request_diagnostic(&conn, "authorization-fallback").unwrap().unwrap();
        assert!(stored.payloads.is_empty());
        assert_eq!(stored.attempts.len(), 2);
        assert_eq!(stored.attempts[0].status, Some(403));
        assert_eq!(stored.attempts[0].error_message.as_deref(), Some("not eligible"));
        assert_eq!(stored.attempts[1].authorization_fallback, authorization_fallback_attempt().authorization_fallback);
        assert_eq!(stored.attempts[1].upstream_request_id.as_deref(), Some("req_local-success"));
    }

    #[test]
    fn authorization_fallback_rejects_unrelated_accounts_routes_and_unbounded_metadata() {
        for mutation in 0..7 {
            let mut attempt = authorization_fallback_attempt();
            let trace = attempt.authorization_fallback.as_mut().unwrap();
            match mutation {
                0 => trace.to_account_id = "other-account".into(),
                1 => trace.from_route = "local_gateway".into(),
                2 => trace.to_route = "official".into(),
                3 => attempt.authorization_route = Some("official".into()),
                4 => trace.reason = "untrusted.key".into(),
                5 => trace.from_account_id = "x".repeat(257),
                _ => trace.trigger_stage = "unknown".into(),
            }
            assert!(normalize_authorization_fallback_trace(&mut attempt));
            assert!(attempt.authorization_fallback.is_none());
        }
        let mut detail = diagnostic("invalid-upstream-id", 100);
        detail.attempts[0].upstream_request_id = Some("https://secret.example/token".into());
        assert!(normalize_request_diagnostics(detail).attempts[0].upstream_request_id.is_none());
        assert!(authorization_fallback_from_attempts_json(&"x".repeat(65537)).is_none());
        assert!(authorization_fallback_from_attempts_json("not-json").is_none());
        assert!(authorization_fallback_from_attempts_json("[]").is_none());
        let mut legacy = serde_json::to_value(authorization_fallback_attempt()).unwrap();
        legacy["authorizationFallback"].as_object_mut().unwrap().remove("triggerStage");
        let stored = authorization_fallback_from_attempts_json(&serde_json::to_string(&vec![legacy]).unwrap()).unwrap();
        assert_eq!(stored.trigger_stage, "upstream", "old traces retain their upstream meaning");
    }

    #[test]
    fn authorization_fallback_summary_is_paged_optional_and_handles_late_diagnostic_writes() {
        let conn = memory_logs();
        insert_local_access_usage_event(&conn, &CodexLocalAccessUsageEvent {
            request_id: "fallback-summary".into(), account_id: "account-a".into(),
            success: true, ..Default::default()
        }).unwrap();
        let projection = request_logs_authorization_fallback_select(&conn).unwrap();
        let sql = format!("SELECT {projection} FROM request_logs ORDER BY id DESC LIMIT 1");
        let summary = || {
            conn.query_row(&sql, [], |row| row.get::<_, Option<String>>(0)).unwrap()
                .as_deref().and_then(authorization_fallback_from_attempts_json)
        };
        // A read before optional diagnostics arrive returns immediately with the usage row.
        assert!(summary().is_none());
        let detail = CodexLocalAccessRequestDetail {
            request_id: "fallback-summary".into(), captured_at_ms: 100,
            attempts: vec![authorization_fallback_attempt()], ..Default::default()
        };
        insert_request_diagnostic(&conn, detail).unwrap();
        assert_eq!(summary(), authorization_fallback_attempt().authorization_fallback);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        conn.execute("UPDATE request_diagnostics SET attempts_json=?1", [&"x".repeat(65537)]).unwrap();
        assert!(summary().is_none());
        conn.execute_batch("DROP TABLE request_diagnostics").unwrap();
        assert_eq!(request_logs_authorization_fallback_select(&conn).unwrap(), "'' AS authorization_attempts_json");
    }

    fn capability_trace(
    ) -> crate::models::codex_local_access::CodexLocalAccessCapabilityFallbackTrace {
        crate::models::codex_local_access::CodexLocalAccessCapabilityFallbackTrace {
            reason: "preflight".into(),
            capabilities: vec!["file_search".into()],
            from_account_id: "plan-primary".into(),
            from_model: "gpt-5.5".into(),
            to_account_id: "account-a".into(),
            to_model: "gpt-5.5".into(),
        }
    }

    #[test]
    fn capability_fallback_trace_persists_without_bodies_and_survives_late_completion() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("request-logs.sqlite");
        let mut detail = diagnostic("capability-route", 10);
        detail.payloads.clear();
        detail.attempts[0].capability_fallback = Some(capability_trace());
        {
            let conn = Connection::open(&path).unwrap();
            create_request_logs_table(&conn, true).unwrap();
            create_request_diagnostics_table(&conn).unwrap();
            insert_request_diagnostic(&conn, detail.clone()).unwrap();
            let mut late = detail;
            late.captured_at_ms += 10;
            late.attempts[0].capability_fallback = None;
            insert_request_diagnostic(&conn, late).unwrap();
        }
        let conn = Connection::open(&path).unwrap();
        let stored = read_request_diagnostic(&conn, "capability-route")
            .unwrap()
            .unwrap();
        assert!(stored.payloads.is_empty());
        assert_eq!(
            stored.attempts.len(),
            1,
            "preflight does not invent a network attempt"
        );
        assert_eq!(
            stored.attempts[0].capability_fallback,
            Some(capability_trace())
        );
        let serialized = serde_json::to_value(&stored).unwrap();
        assert_eq!(
            serialized["attempts"][0]["capabilityFallback"]["fromAccountId"],
            "plan-primary"
        );
        assert_eq!(
            serialized["attempts"][0]["capabilityFallback"]["toModel"],
            "gpt-5.5"
        );
    }

    #[test]
    fn tls_recovery_evidence_survives_storage_without_request_bodies() {
        let conn = memory_logs();
        let mut detail = diagnostic("tls-recovery", 10);
        detail.payloads.clear();
        detail.attempts[0].transport_error = Some("tls_decrypt".into());
        detail.attempts[0].transport_pool_retired = Some(true);
        detail.attempts[0].upstream_http_version = Some("HTTP/2.0".into());
        insert_request_diagnostic(&conn, detail.clone()).unwrap();
        detail.captured_at_ms += 1;
        detail.attempts[0].transport_error = None;
        detail.attempts[0].transport_pool_retired = None;
        detail.attempts[0].upstream_http_version = None;
        insert_request_diagnostic(&conn, detail.clone()).unwrap();
        let stored = read_request_diagnostic(&conn, "tls-recovery").unwrap().unwrap();
        assert!(stored.payloads.is_empty());
        assert_eq!(stored.attempts[0].transport_error.as_deref(), Some("tls_decrypt"));
        assert_eq!(stored.attempts[0].transport_pool_retired, Some(true));
        assert_eq!(stored.attempts[0].upstream_http_version.as_deref(), Some("HTTP/2.0"));
        detail.captured_at_ms += 1;
        detail.attempts[0].account_id = "different-account".into();
        insert_request_diagnostic(&conn, detail).unwrap();
        let stored = read_request_diagnostic(&conn, "tls-recovery").unwrap().unwrap();
        assert!(stored.attempts[0].transport_error.is_none());
        assert!(stored.attempts[0].transport_pool_retired.is_none());
        assert!(stored.attempts[0].upstream_http_version.is_none());
    }

    #[test]
    fn capability_fallback_trace_never_labels_legacy_or_unrelated_attempts() {
        let legacy: CodexLocalAccessRequestAttempt =
            serde_json::from_str(r#"{"sequence":1}"#).unwrap();
        assert!(legacy.capability_fallback.is_none());
        assert!(serde_json::to_value(legacy)
            .unwrap()
            .get("capabilityFallback")
            .is_none());

        let mut original = diagnostic("capability-route", 10);
        original.attempts[0].capability_fallback = Some(capability_trace());
        let mut replaced = original.clone();
        replaced.captured_at_ms += 1;
        replaced.attempts[0].account_id = "another-target".into();
        replaced.attempts[0].capability_fallback = None;
        assert!(merge_request_diagnostics(original, replaced).attempts[0]
            .capability_fallback
            .is_none());

        for invalid in ["reason", "account", "model", "url", "capability"] {
            let mut detail = diagnostic("invalid-trace", 1);
            let mut trace = capability_trace();
            match invalid {
                "reason" => trace.reason = "arbitrary upstream text".into(),
                "account" => trace.to_account_id = "another-target".into(),
                "model" => trace.to_model = "another-model".into(),
                "url" => trace.from_model = "https://example.invalid/secret".into(),
                _ => trace.capabilities = vec!["schema or arguments must never be logged".into()],
            }
            detail.attempts[0].capability_fallback = Some(trace);
            let normalized = normalize_request_diagnostics(detail);
            assert!(
                normalized.attempts[0].capability_fallback.is_none(),
                "{invalid}"
            );
            assert!(normalized.truncated);
        }
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
