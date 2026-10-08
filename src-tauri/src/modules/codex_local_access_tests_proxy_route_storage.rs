use super::*;

fn snapshot(kind: &str, name: &str) -> CodexLocalAccessProxyRoute {
    CodexLocalAccessProxyRoute {
        kind: kind.to_string(),
        name: name.to_string(),
    }
}

fn event_with_snapshot(route: Option<CodexLocalAccessProxyRoute>) -> CodexLocalAccessUsageEvent {
    CodexLocalAccessUsageEvent {
        timestamp: 123,
        request_id: "proxy-route-request".to_string(),
        model_id: "gpt-test".to_string(),
        proxy_route: route,
        success: true,
        ..Default::default()
    }
}

fn stored_events(conn: &Connection) -> Vec<CodexLocalAccessUsageEvent> {
    let mut events = Vec::new();
    for_each_local_access_usage_event_since_from_conn(conn, 0, |event| {
        events.push(event);
        Ok(())
    })
    .expect("read persisted events");
    events
}

#[test]
fn proxy_route_storage_round_trip_preserves_all_route_kinds_and_insert_schemas() {
    for (has_service_tier, has_reasoning_effort) in [(false, false), (true, false), (true, true)] {
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
        for (index, route) in [
            Some(snapshot("node", "东京节点 A")),
            Some(snapshot("proxy", "proxy.example:8080")),
            Some(snapshot("direct", "")),
            Some(snapshot("unknown", "")),
            None,
        ]
        .into_iter()
        .enumerate()
        {
            let mut event = event_with_snapshot(route.clone());
            event.timestamp += index as i64;
            insert_local_access_usage_event(&conn, &event).unwrap();
            assert_eq!(stored_events(&conn)[index].proxy_route, route);
        }
    }
}

#[test]
fn proxy_route_storage_reads_legacy_schema_without_migration_or_backfill() {
    let current = Connection::open_in_memory().unwrap();
    create_request_logs_table(&current, true).unwrap();
    let schema: String = current
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'request_logs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let legacy = Connection::open_in_memory().unwrap();
    legacy
        .execute_batch(&schema.replace("proxy_route_json TEXT NOT NULL DEFAULT '',", ""))
        .unwrap();
    legacy
        .execute(
            "INSERT INTO request_logs (event_key, timestamp, request_id) VALUES ('old', 1, 'old-request')",
            [],
        )
        .unwrap();
    let changes_before_read = legacy.total_changes();
    assert_eq!(stored_events(&legacy)[0].proxy_route, None);
    assert_eq!(legacy.total_changes(), changes_before_read);
    assert!(!request_logs_has_column(&legacy, "proxy_route_json").unwrap());

    let new_route = snapshot("node", "新节点");
    insert_local_access_usage_event(&legacy, &event_with_snapshot(Some(new_route.clone())))
        .unwrap();
    let events = stored_events(&legacy);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].proxy_route, None, "old requests stay unrecorded");
    assert_eq!(events[1].proxy_route.as_ref(), Some(&new_route));
}

#[test]
fn proxy_route_storage_duplicate_requests_cannot_rewrite_recorded_snapshot() {
    let conn = Connection::open_in_memory().unwrap();
    create_request_logs_table(&conn, true).unwrap();
    let original = snapshot("node", "original node");
    let mut event = event_with_snapshot(Some(original.clone()));
    insert_local_access_usage_event(&conn, &event).unwrap();

    event.proxy_route = Some(snapshot("node", "renamed or replacement node"));
    insert_local_access_usage_event(&conn, &event).unwrap();
    let events = stored_events(&conn);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].proxy_route.as_ref(), Some(&original));
}

#[test]
fn proxy_route_storage_json_round_trip_and_historical_events() {
    let old_event: CodexLocalAccessUsageEvent = serde_json::from_str("{}").unwrap();
    assert_eq!(old_event.proxy_route, None);
    assert!(serde_json::to_value(&old_event)
        .unwrap()
        .get("proxyRoute")
        .is_none());

    let route = snapshot("node", "节点 \"A\" / 東京");
    let event = event_with_snapshot(Some(route.clone()));
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["proxyRoute"]["kind"], "node");
    assert_eq!(json["proxyRoute"]["name"], route.name);
    assert!(json.get("proxy_route").is_none());
    let restored: CodexLocalAccessUsageEvent = serde_json::from_value(json).unwrap();
    assert_eq!(restored.proxy_route.as_ref(), Some(&route));
}

#[test]
fn proxy_route_sidecar_snapshot_is_cloned_into_recent_events() {
    let mut sidecar: SidecarUsageEvent = serde_json::from_value(json!({
        "requestId": "captured-connection", "success": true,
        "proxyRoute": {"kind": "node", "name": "original node"}
    }))
    .unwrap();
    let mut recent = Vec::new();
    let event = append_usage_event_with_meta(
        &mut recent,
        123,
        Some(&sidecar.request_id),
        None,
        None,
        None,
        None,
        None,
        Some("model"),
        Some(CodexLocalAccessGatewayMode::Sidecar),
        CodexLocalAccessRequestKind::Text,
        None,
        None,
        None,
        None,
        true,
        Some(200),
        None,
        None,
        10,
        None,
        None,
        1,
        0.0,
        sidecar.proxy_route.as_ref(),
    );
    sidecar.proxy_route.as_mut().unwrap().name = "replacement node".into();
    assert_eq!(event.proxy_route.as_ref().unwrap().name, "original node");
    assert_eq!(recent[0].proxy_route, event.proxy_route);
    let old: SidecarUsageEvent = serde_json::from_value(json!({"success": true})).unwrap();
    assert!(old.proxy_route.is_none());
    let direct: SidecarUsageEvent = serde_json::from_value(json!({
        "success": true, "proxyRoute": {"kind": "direct"}
    }))
    .unwrap();
    assert_eq!(direct.proxy_route.unwrap().name, "");
}
