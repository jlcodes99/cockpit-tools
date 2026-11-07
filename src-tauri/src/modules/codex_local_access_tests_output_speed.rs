mod output_speed_tests {
    use super::super::*;

    #[test]
    fn output_speed_legacy_and_sidecar_compatibility() {
        let legacy: CodexLocalAccessUsageEvent = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.output_generation_ms, None);
        let old: SidecarUsageEvent = serde_json::from_str(r#"{"type":"usage"}"#).unwrap();
        assert_eq!(old.output_generation_ms, None);
        let new: SidecarUsageEvent =
            serde_json::from_str(r#"{"type":"usage","outputGenerationMs":10000}"#).unwrap();
        assert_eq!(new.output_generation_ms, Some(10000));
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE request_logs(request_id TEXT)")
            .unwrap();
        assert_eq!(
            request_logs_output_generation_select(&conn).unwrap(),
            "NULL AS output_generation_ms"
        );
        assert!(!request_logs_has_column(&conn, "output_generation_ms").unwrap());
    }

    #[test]
    fn output_speed_storage_adds_nullable_column_without_replaying_usage() {
        let conn = Connection::open_in_memory().unwrap();
        create_request_logs_table(&conn, true).unwrap();
        // Simulate an older schema and let the existing usage writer add the metric.
        conn.execute_batch("ALTER TABLE request_logs DROP COLUMN output_generation_ms")
            .unwrap();
        let event = CodexLocalAccessUsageEvent {
            timestamp: 10,
            request_id: "speed-storage".into(),
            success: true,
            latency_ms: 19000,
            output_tokens: 1000,
            total_tokens: 1000,
            output_generation_ms: Some(10000),
            ..Default::default()
        };
        insert_local_access_usage_event(&conn, &event).unwrap();
        insert_local_access_usage_event(&conn, &event).unwrap();
        let stored = conn
            .query_row("SELECT * FROM request_logs", [], usage_event_from_row)
            .unwrap();
        assert_eq!(stored.output_generation_ms, Some(10000));
        assert_eq!(stored.latency_ms, 19000);
        assert_eq!(stored.output_tokens, 1000);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(
            request_logs_output_generation_select(&conn).unwrap(),
            "output_generation_ms"
        );
    }
}
