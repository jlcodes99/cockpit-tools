use crate::models::codex_local_access::{
    CodexLocalAccessConnectionSource, CodexLocalAccessSourceKind,
};
use std::net::IpAddr;

fn classify_request_peer(
    ip: IpAddr,
    local_addresses: &std::collections::HashSet<IpAddr>,
) -> CodexLocalAccessConnectionSource {
    let peer_ip = ip.to_canonical();
    let kind = if peer_ip.is_loopback() || local_addresses.contains(&peer_ip) {
        CodexLocalAccessSourceKind::Local
    } else if match peer_ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
    } {
        CodexLocalAccessSourceKind::Lan
    } else {
        CodexLocalAccessSourceKind::Remote
    };
    CodexLocalAccessConnectionSource { peer_ip, kind }
}

async fn classify_native_request_peer(peer: IpAddr) -> CodexLocalAccessConnectionSource {
    type LocalAddresses = (std::time::Instant, std::collections::HashSet<IpAddr>);
    static ADDRESSES: tokio::sync::Mutex<Option<LocalAddresses>> =
        tokio::sync::Mutex::const_new(None);
    if peer.is_loopback() {
        return classify_request_peer(peer, &Default::default());
    }
    // This runs in the request record worker, after the response. A single
    // snapshot serves concurrent reports and refreshes after network changes.
    let mut snapshot = ADDRESSES.lock().await;
    if snapshot
        .as_ref()
        .is_none_or(|(at, _)| at.elapsed() >= Duration::from_secs(30))
    {
        if let Ok(addresses) = tokio::task::spawn_blocking(|| {
            sysinfo::Networks::new_with_refreshed_list()
                .list()
                .values()
                .flat_map(|network| network.ip_networks())
                .map(|network| network.addr.to_canonical())
                .collect()
        })
        .await
        {
            *snapshot = Some((std::time::Instant::now(), addresses));
        }
    }
    match snapshot.as_ref() {
        Some((_, addresses)) => classify_request_peer(peer, addresses),
        None => classify_request_peer(peer, &Default::default()),
    }
}

fn ensure_request_connection_sources(conn: &Connection) -> rusqlite::Result<()> {
    // The empty companion table avoids rebuilding indexes across old logs.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS request_connection_sources (
            log_id INTEGER PRIMARY KEY,
            peer_ip TEXT NOT NULL,
            source_kind TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS request_connection_sources_peer
            ON request_connection_sources(peer_ip, log_id);
        CREATE INDEX IF NOT EXISTS request_connection_sources_kind
            ON request_connection_sources(source_kind, log_id);
        CREATE TRIGGER IF NOT EXISTS request_connection_sources_delete
            AFTER DELETE ON request_logs BEGIN
            DELETE FROM request_connection_sources WHERE log_id = OLD.id;
        END;
        CREATE TABLE IF NOT EXISTS request_log_rejections (
            id INTEGER PRIMARY KEY CHECK (id < 0),
            request_id TEXT NOT NULL UNIQUE,
            timestamp INTEGER NOT NULL,
            api_key_id TEXT NOT NULL,
            api_key_label TEXT NOT NULL,
            client_instance_id TEXT NOT NULL,
            requested_model TEXT NOT NULL,
            request_kind TEXT NOT NULL,
            http_status INTEGER,
            error_category TEXT NOT NULL,
            error_message TEXT NOT NULL,
            latency_ms INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS request_log_rejections_time
            ON request_log_rejections(timestamp DESC, id);
        CREATE TRIGGER IF NOT EXISTS request_log_rejections_delete
            AFTER DELETE ON request_log_rejections BEGIN
            DELETE FROM request_connection_sources WHERE log_id = OLD.id;
        END;",
    )
}

fn insert_request_rejection(
    conn: &Connection,
    event: &CodexLocalAccessUsageEvent,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO request_log_rejections (
            id, request_id, timestamp, api_key_id, api_key_label, client_instance_id, requested_model,
            request_kind, http_status, error_category, error_message, latency_ms
        ) VALUES (
            COALESCE((SELECT MAX(id) + 1 FROM request_log_rejections), -9223372036854775808),
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
        )",
        params![
            event.request_id,
            event.timestamp,
            event.api_key_id,
            event.api_key_label,
            event.client_instance_id,
            event.requested_model,
            request_kind_to_db_value(event.request_kind),
            event.http_status,
            event.error_category,
            event.error_message,
            event.latency_ms.min(i64::MAX as u64) as i64
        ],
    )?;
    if let Some(source) = event.connection_source {
        conn.execute(
            "INSERT OR IGNORE INTO request_connection_sources (log_id, peer_ip, source_kind)
             SELECT id, ?1, ?2 FROM request_log_rejections WHERE request_id = ?3",
            params![
                source.peer_ip.to_string(),
                source.kind.as_str(),
                event.request_id
            ],
        )?;
    }
    Ok(())
}

fn request_logs_with_rejections_cte(conn: &Connection) -> rusqlite::Result<String> {
    if !conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM request_log_rejections)",
        [],
        |row| row.get::<_, bool>(0),
    )? {
        return Ok(String::new());
    }
    let mut raw = vec![
        "id",
        "request_id",
        "timestamp",
        "api_key_id",
        "api_key_label",
        "client_instance_id",
        "requested_model",
        "request_kind",
        "http_status",
        "error_category",
        "error_message",
        "latency_ms",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    // Rejections have negative row IDs. Both branches can probe their primary
    // keys directly after a source-index lookup.
    let mut audit = raw.clone();
    let mut columns = conn.prepare("SELECT name FROM pragma_table_info('request_logs')")?;
    let columns = columns
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
    for (name, value) in [
        ("account_id", "''"),
        ("email", "''"),
        ("gateway_mode", "'native'"),
        ("model_id", "requested_model"),
        ("upstream_model", "''"),
        ("service_tier", "''"),
        ("requested_service_tier", "''"),
        ("response_service_tier", "''"),
        ("reasoning_effort", "''"),
        ("proxy_route_json", "''"),
        ("authorization_route", "''"),
        ("first_response_ms", "NULL"),
        ("output_generation_ms", "NULL"),
        ("turn_state_length", "NULL"),
        ("turn_state_class", "''"),
        ("success", "0"),
        ("input_tokens", "0"),
        ("output_tokens", "0"),
        ("total_tokens", "0"),
        ("cached_tokens", "0"),
        ("reasoning_tokens", "0"),
        ("token_breakdown_json", "''"),
        ("estimated_cost_usd", "0.0"),
        ("model_pricing_version", "1"),
        ("input_usd_per_million", "0.0"),
        ("output_usd_per_million", "0.0"),
        ("cached_input_usd_per_million", "NULL"),
    ] {
        raw.push(if columns.contains(name) {
            name.to_owned()
        } else {
            format!("{value} AS {name}")
        });
        audit.push(value.to_owned());
    }
    Ok(format!(
        "WITH request_logs AS NOT MATERIALIZED (
             SELECT {} FROM main.request_logs
             UNION ALL
             SELECT {} FROM request_log_rejections
         ) ",
        raw.join(", "),
        audit.join(", "),
    ))
}

fn insert_request_connection_source(
    conn: &Connection,
    event: &CodexLocalAccessUsageEvent,
) -> rusqlite::Result<()> {
    if let Some(source) = event.connection_source {
        conn.execute(
            "INSERT OR IGNORE INTO request_connection_sources (log_id, peer_ip, source_kind)
             SELECT id, ?1, ?2 FROM request_logs WHERE event_key = ?3",
            params![
                source.peer_ip.to_string(),
                source.kind.as_str(),
                local_access_log_event_key(event)
            ],
        )?;
    }
    Ok(())
}

fn request_connection_source_from_row(
    row: &rusqlite::Row<'_>,
) -> Option<CodexLocalAccessConnectionSource> {
    let ip = row.get::<_, String>("peer_ip").ok()?.parse().ok()?;
    let kind = CodexLocalAccessSourceKind::from_str(&row.get::<_, String>("source_kind").ok()?)?;
    Some(CodexLocalAccessConnectionSource { peer_ip: ip, kind })
}

#[derive(Default)]
struct RequestConnectionSourceFilter {
    kind: Option<String>,
    peer_ip: Option<String>,
}

impl RequestConnectionSourceFilter {
    fn append_sql(self, clauses: &mut Vec<String>, params: &mut Vec<SqlValue>) {
        if let Some(kind) = normalize_log_filter(self.kind) {
            if kind == "unknown" {
                clauses.push("id NOT IN (SELECT log_id FROM request_connection_sources)".into());
            } else {
                clauses.push(
                    "id IN (SELECT log_id FROM request_connection_sources WHERE source_kind = ?)"
                        .into(),
                );
                params.push(SqlValue::Text(kind));
            }
        }
        if let Some(ip) = normalize_log_filter(self.peer_ip) {
            if let Ok(ip) = ip.parse::<IpAddr>() {
                clauses.push(
                    "id IN (SELECT log_id FROM request_connection_sources WHERE peer_ip = ?)"
                        .into(),
                );
                params.push(SqlValue::Text(ip.to_canonical().to_string()));
            } else {
                clauses.push("0".into());
            }
        }
    }
}

#[cfg(test)]
mod request_connection_source_tests {
    use super::*;

    #[test]
    fn classifies_peer_addresses_against_the_host_interfaces() {
        let local_addresses = ["192.168.40.10", "fd00::10"]
            .map(|ip| ip.parse::<IpAddr>().unwrap())
            .into_iter()
            .collect();
        use CodexLocalAccessSourceKind::{Lan, Local, Remote};
        for (ip, kind, canonical) in [
            ("127.0.0.1", Local, "127.0.0.1"),
            ("::1", Local, "::1"),
            ("::ffff:192.168.40.10", Local, "192.168.40.10"),
            ("fd00::10", Local, "fd00::10"),
            ("10.0.0.8", Lan, "10.0.0.8"),
            ("169.254.4.8", Lan, "169.254.4.8"),
            ("fe80::8", Lan, "fe80::8"),
            ("fd00::8", Lan, "fd00::8"),
            ("198.51.100.8", Remote, "198.51.100.8"),
            ("2001:db8::8", Remote, "2001:db8::8"),
        ] {
            let source = classify_request_peer(ip.parse().unwrap(), &local_addresses);
            assert_eq!(
                (source.kind, source.peer_ip),
                (kind, canonical.parse().unwrap())
            );
        }
    }

    #[test]
    fn request_pages_filter_live_peers_without_backfilling_historical_logs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("request-logs.sqlite");
        let conn = open_local_access_logs_db_once(&path, true).unwrap();
        let old = CodexLocalAccessUsageEvent {
            request_id: "old".into(),
            timestamp: 1,
            ..Default::default()
        };
        insert_local_access_usage_event(&conn, &old).unwrap();
        let source = classify_request_peer("192.168.40.8".parse().unwrap(), &Default::default());
        let live = CodexLocalAccessUsageEvent {
            request_id: "live".into(),
            timestamp: 2,
            connection_source: Some(source),
            ..Default::default()
        };
        insert_live_local_access_usage_event(&conn, &live).unwrap();
        insert_live_local_access_usage_event(&conn, &live).unwrap();
        let query = |kind: Option<&str>, ip: Option<&str>| {
            query_local_access_usage_events_blocking(
                &conn,
                1,
                10,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                RequestConnectionSourceFilter {
                    kind: kind.map(str::to_owned),
                    peer_ip: ip.map(str::to_owned),
                },
            )
            .unwrap()
        };
        let all = query(None, None);
        assert_eq!(all.total, 2);
        assert_eq!(all.events[0].connection_source, Some(source));
        assert_eq!(all.events[1].connection_source, None);
        let matching = query(Some("lan"), Some("::ffff:192.168.40.8"));
        assert_eq!(matching.total, 1);
        assert_eq!(matching.events[0].request_id, "live");
        assert_eq!(query(Some("unknown"), None).events[0].request_id, "old");
        assert_eq!(query(Some("local"), None).total, 0);
        assert_eq!(query(None, Some("192.168.40.80")).total, 0);
        assert_eq!(query(None, Some("192.168")).total, 0);

        let rejected = CodexLocalAccessUsageEvent {
            request_id: "rejected".into(),
            timestamp: 3,
            local_rejection: true,
            connection_source: Some(source),
            http_status: Some(403),
            error_category: "model_excluded".into(),
            ..Default::default()
        };
        insert_live_local_access_usage_event(&conn, &rejected).unwrap();
        let page = query(Some("lan"), None);
        assert_eq!(page.total, 2);
        assert_eq!(page.events[0].request_id, "rejected");
        assert_eq!(page.events[0].http_status, Some(403));
        assert_eq!(page.events[0].connection_source, Some(source));
        assert_eq!(page.events[1].request_id, "live");
        assert_eq!(query(Some("unknown"), None).events[0].request_id, "old");
        let raw_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(raw_count, 2);

        for (column, value, index) in [
            ("peer_ip", "192.168.40.8", "request_connection_sources_peer"),
            ("source_kind", "lan", "request_connection_sources_kind"),
        ] {
            let cte = request_logs_with_rejections_cte(&conn).unwrap();
            let sql = format!(
                "EXPLAIN QUERY PLAN {cte} SELECT COUNT(*) FROM request_logs
                 WHERE id IN (SELECT log_id FROM request_connection_sources WHERE {column} = ?1)",
            );
            let mut statement = conn.prepare(&sql).unwrap();
            let plan = statement
                .query_map([value], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
                .join("\n");
            assert!(plan.contains(index), "{plan}");
            assert!(
                plan.contains("SEARCH main.request_logs USING INTEGER PRIMARY KEY"),
                "{plan}"
            );
            assert!(
                plan.contains("SEARCH request_log_rejections USING INTEGER PRIMARY KEY"),
                "{plan}"
            );
        }
        conn.execute("DELETE FROM request_logs WHERE request_id = 'live'", [])
            .unwrap();
        conn.execute("DELETE FROM request_log_rejections", [])
            .unwrap();
        assert_eq!(query(Some("lan"), None).total, 0);
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM request_connection_sources",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
            0
        );
    }
}
