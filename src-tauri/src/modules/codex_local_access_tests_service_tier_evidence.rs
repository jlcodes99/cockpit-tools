mod service_tier_evidence_tests {
    use super::super::*;

    fn read_events(conn: &Connection) -> Vec<CodexLocalAccessUsageEvent> {
        let mut events = Vec::new();
        for_each_local_access_usage_event_since_from_conn(conn, 0, |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        events
    }

    #[test]
    fn authorization_route_is_recorded_per_event_and_never_inferred_from_current_mode() {
        let directory = tempfile::tempdir().unwrap();
        let conn = open_local_access_logs_db_once(&directory.path().join("logs.sqlite"), true).unwrap();
        let mut event = CodexLocalAccessUsageEvent {
            timestamp: 1000, request_id: "official-request".into(), account_id: "original-source".into(),
            authorization_route: Some("official".into()), success: true, ..Default::default()
        };
        insert_local_access_usage_event(&conn, &event).unwrap();
        event.authorization_route = Some("local_gateway".into());
        insert_local_access_usage_event(&conn, &event).unwrap();
        let loaded = read_events(&conn);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].account_id, "original-source");
        assert_eq!(loaded[0].authorization_route.as_deref(), Some("official"));
        event.request_id = "historical-unrecorded".into();
        event.authorization_route = None;
        insert_local_access_usage_event(&conn, &event).unwrap();
        assert!(read_events(&conn).iter().find(|event| event.request_id == "historical-unrecorded").unwrap().authorization_route.is_none());
    }

    #[test]
    fn service_tier_evidence_deserializes_new_sidecar_values_without_inferring_legacy() {
        let old: SidecarUsageEvent =
            serde_json::from_value(json!({"serviceTier":"standard"})).unwrap();
        assert!(old.requested_service_tier.is_none());
        assert!(old.response_service_tier.is_none());
        let new: SidecarUsageEvent = serde_json::from_value(json!({
            "requestedServiceTier":"priority", "responseServiceTier":"default"
        }))
        .unwrap();
        assert_eq!(new.requested_service_tier.as_deref(), Some("priority"));
        assert_eq!(new.response_service_tier.as_deref(), Some("default"));
        let legacy: CodexLocalAccessUsageEvent =
            serde_json::from_value(json!({"serviceTier":"standard"})).unwrap();
        let serialized = serde_json::to_value(&legacy).unwrap();
        assert!(serialized.get("requestedServiceTier").is_none());
        assert!(serialized.get("responseServiceTier").is_none());
    }

    #[test]
    fn service_tier_evidence_round_trip_preserves_outbound_and_response_independently() {
        for (has_service_tier, has_reasoning_effort) in
            [(false, false), (true, false), (true, true)]
        {
            let conn = Connection::open_in_memory().unwrap();
            create_request_logs_table(&conn, has_service_tier).unwrap();
            if has_reasoning_effort {
                ensure_request_logs_column(
                    &conn,
                    "reasoning_effort",
                    "reasoning_effort TEXT NOT NULL DEFAULT ''",
                )
                .unwrap();
            }
            for (index, (requested, response)) in [
                (Some("priority"), Some("default")),
                (Some("default"), Some("default")),
                (Some("auto"), None),
                (None, Some("future-tier")),
                (None, None),
            ]
            .into_iter()
            .enumerate()
            {
                let mut event = CodexLocalAccessUsageEvent {
                    timestamp: index as i64 + 1,
                    request_id: format!("tier-{index}"),
                    success: true,
                    requested_service_tier: requested.map(str::to_string),
                    response_service_tier: response.map(str::to_string),
                    ..Default::default()
                };
                insert_local_access_usage_event(&conn, &event).unwrap();
                // Duplicate delivery may not replace the evidence from the original request.
                event.requested_service_tier = Some("replayed-tier".into());
                event.response_service_tier = Some("replayed-response".into());
                insert_local_access_usage_event(&conn, &event).unwrap();
                let events = read_events(&conn);
                assert_eq!(events.len(), index + 1);
                assert_eq!(events[index].requested_service_tier.as_deref(), requested);
                assert_eq!(events[index].response_service_tier.as_deref(), response);
                let json = serde_json::to_value(&events[index]).unwrap();
                assert_eq!(
                    json.get("responseServiceTier").and_then(Value::as_str),
                    response
                );
            }
        }
    }

    #[test]
    fn service_tier_evidence_legacy_reads_do_not_migrate_or_backfill() {
        let conn = Connection::open_in_memory().unwrap();
        create_request_logs_table(&conn, true).unwrap();
        conn.execute("INSERT INTO request_logs (event_key, timestamp, service_tier) VALUES ('old', 1, 'standard')", []).unwrap();
        let before = conn.total_changes();
        let events = read_events(&conn);
        assert_eq!(events[0].service_tier.as_deref(), Some("standard"));
        assert!(events[0].requested_service_tier.is_none());
        assert!(events[0].response_service_tier.is_none());
        assert_eq!(conn.total_changes(), before);
        assert!(!request_logs_has_column(&conn, "requested_service_tier").unwrap());
        assert!(!request_logs_has_column(&conn, "response_service_tier").unwrap());
        let event = CodexLocalAccessUsageEvent {
            timestamp: 2,
            request_id: "new".into(),
            requested_service_tier: Some("priority".into()),
            response_service_tier: Some("default".into()),
            ..Default::default()
        };
        insert_local_access_usage_event(&conn, &event).unwrap();
        let events = read_events(&conn);
        assert_eq!(events.len(), 2);
        assert!(events[0].requested_service_tier.is_none());
        assert!(events[0].response_service_tier.is_none());
        assert_eq!(
            events[1].requested_service_tier.as_deref(),
            Some("priority")
        );
        assert_eq!(events[1].response_service_tier.as_deref(), Some("default"));
    }
}
