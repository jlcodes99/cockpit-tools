// A small query projection independent of diagnostic bodies. New and updated
// diagnostics maintain it in the same SQLite statement; deletes retain the
// previous retention semantics. Historical rows are filled in bounded batches.
fn ensure_session_query_index(conn: &Connection) -> Result<(), SqliteError> {
    if !diagnostics_table_exists(conn)? {
        return Ok(());
    }
    let present: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('request_diagnostics') WHERE name='client_thread_id')", [], |r| r.get(0))?;
    if !present {
        return Ok(());
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS request_session_lookup (
        request_id TEXT PRIMARY KEY, effective_session_id TEXT COLLATE NOCASE NOT NULL);
        CREATE INDEX IF NOT EXISTS idx_request_session_lookup ON request_session_lookup(effective_session_id COLLATE NOCASE,request_id);
        CREATE TABLE IF NOT EXISTS request_session_migration (id INTEGER PRIMARY KEY CHECK(id=1),cursor INTEGER NOT NULL DEFAULT 0,complete INTEGER NOT NULL DEFAULT 0);
        INSERT OR IGNORE INTO request_session_migration(id) VALUES(1);")?;
    for (name, operation) in [
        ("insert", "INSERT"),
        (
            "update",
            "UPDATE OF client_identity_captured,client_thread_id,client_session_id",
        ),
    ] {
        conn.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS request_session_{name} AFTER {operation} ON request_diagnostics BEGIN
            DELETE FROM request_session_lookup WHERE request_id=NEW.request_id;
            INSERT INTO request_session_lookup(request_id,effective_session_id)
            SELECT NEW.request_id,COALESCE(NULLIF(NEW.client_thread_id,''),NULLIF(NEW.client_session_id,''))
            WHERE NEW.client_identity_captured=1 AND COALESCE(NULLIF(NEW.client_thread_id,''),NULLIF(NEW.client_session_id,'')) IS NOT NULL;
            END;"))?;
    }
    conn.execute_batch("CREATE TRIGGER IF NOT EXISTS request_session_delete AFTER DELETE ON request_diagnostics BEGIN
        DELETE FROM request_session_lookup WHERE request_id=OLD.request_id;
        END;")?;
    Ok(())
}

fn session_query_index_ready(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT complete FROM request_session_migration WHERE id=1",
        [],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

fn backfill_session_query_index(conn: &Connection) -> Result<bool, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let (cursor, complete): (i64, bool) = tx
        .query_row(
            "SELECT cursor,complete FROM request_session_migration WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    if complete {
        return Ok(true);
    }
    let next: Option<i64> = tx.query_row("SELECT MAX(rowid) FROM (SELECT rowid FROM request_diagnostics WHERE rowid>?1 ORDER BY rowid LIMIT 32)", [cursor], |r| r.get(0)).map_err(|e| e.to_string())?;
    if let Some(next) = next {
        tx.execute("INSERT OR REPLACE INTO request_session_lookup(request_id,effective_session_id)
            SELECT request_id,COALESCE(NULLIF(client_thread_id,''),NULLIF(client_session_id,'')) FROM request_diagnostics
            WHERE rowid>?1 AND rowid<=?2 AND client_identity_captured=1
            AND COALESCE(NULLIF(client_thread_id,''),NULLIF(client_session_id,'')) IS NOT NULL", params![cursor,next]).map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE request_session_migration SET cursor=?1 WHERE id=1",
            [next],
        )
        .map_err(|e| e.to_string())?;
    } else {
        tx.execute(
            "UPDATE request_session_migration SET complete=1 WHERE id=1",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(next.is_none())
}

fn schedule_session_query_backfill() {
    let Ok(path) = local_access_logs_db_path() else {
        return;
    };
    crate::modules::background_maintenance::schedule(
        format!("request-sessions:{}", path.display()),
        move || {
            let Ok(_guard) = local_access_logs_db_write_lock().try_lock() else {
                return Ok(false);
            };
            let conn =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|e| e.to_string())?;
            conn.busy_timeout(Duration::from_millis(10))
                .map_err(|e| e.to_string())?;
            with_request_log_sql_budget(
                &conn,
                crate::modules::background_maintenance::BATCH_TIME,
                || backfill_session_query_index(&conn),
            )
        },
    );
}

#[cfg(test)]
mod query_session_index_tests {
    use super::*;
    #[test]
    fn backfill_and_live_changes_preserve_thread_precedence_and_retention() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE request_diagnostics(request_id TEXT PRIMARY KEY,client_identity_captured INTEGER,client_thread_id TEXT,client_session_id TEXT);
            INSERT INTO request_diagnostics VALUES('old',1,'Chat-A','shared'),('unknown',0,'Chat-A',NULL);").unwrap();
        ensure_session_query_index(&conn).unwrap();
        assert!(!session_query_index_ready(&conn));
        assert!(!backfill_session_query_index(&conn).unwrap());
        conn.execute("UPDATE request_diagnostics SET client_thread_id=NULL,client_session_id='Chat-B' WHERE request_id='old'", []).unwrap();
        assert!(backfill_session_query_index(&conn).unwrap());
        let id: String = conn.query_row("SELECT request_id FROM request_session_lookup WHERE effective_session_id='chat-b' COLLATE NOCASE", [], |r| r.get(0)).unwrap();
        assert_eq!(id, "old");
        conn.execute("DELETE FROM request_diagnostics", []).unwrap();
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM request_session_lookup", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(backfill_session_query_index(&conn).unwrap());
    }

    #[test]
    fn completed_session_filter_uses_both_indexes_without_correlated_scan() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE request_logs(request_id TEXT,timestamp INTEGER);CREATE INDEX log_request ON request_logs(request_id,timestamp);
            CREATE TABLE request_diagnostics(request_id TEXT PRIMARY KEY,client_identity_captured INTEGER,client_thread_id TEXT,client_session_id TEXT);").unwrap();
        ensure_session_query_index(&conn).unwrap();
        assert!(backfill_session_query_index(&conn).unwrap());
        let mut clauses = Vec::new();
        let mut values = Vec::new();
        push_session_filter(&mut clauses, &mut values, Some("CHAT".into()), true);
        let sql = format!(
            "EXPLAIN QUERY PLAN SELECT COUNT(*) FROM request_logs WHERE {}",
            clauses[0]
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let plan = stmt
            .query_map(params_from_iter(values), |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(
            plan.contains("SEARCH request_session_lookup USING COVERING INDEX"),
            "{plan}"
        );
        assert!(
            plan.contains("SEARCH request_logs USING COVERING INDEX log_request"),
            "{plan}"
        );
        assert!(!plan.contains("CORRELATED"), "{plan}");
    }
}
