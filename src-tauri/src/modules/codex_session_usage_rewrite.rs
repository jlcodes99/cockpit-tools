// Rewritten rollouts keep their previous numeric snapshot visible until the
// replacement parser reaches EOF. Backup and removal both use bounded batches.
const SESSION_REWRITE_ROLLUP: crate::modules::usage_rollup::Rollup =
    crate::modules::usage_rollup::Rollup {
        source: "session_rewrite_events",
        prefix: "session_rewrite_v1",
        width: 60,
        dimensions: &["instance_id", "instance_name", "model", "file_path"],
        metrics: SESSION_USAGE_ROLLUP.metrics,
    };

fn ensure_rewrite_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS session_rewrite_state(file_path TEXT PRIMARY KEY,cursor INTEGER NOT NULL DEFAULT 0,phase INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS session_rewrite_events(request_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,instance_name TEXT NOT NULL,session_id TEXT NOT NULL,model TEXT NOT NULL,timestamp INTEGER NOT NULL,input_tokens INTEGER NOT NULL,cached_input_tokens INTEGER NOT NULL,output_tokens INTEGER NOT NULL,file_path TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS idx_rewrite_file ON session_rewrite_events(file_path);
        CREATE INDEX IF NOT EXISTS idx_rewrite_time ON session_rewrite_events(timestamp);").map_err(|e|e.to_string())?;
    SESSION_REWRITE_ROLLUP
        .ensure(conn)
        .map_err(|e| e.to_string())?;
    // This table is created empty and every insert is covered by its triggers.
    conn.execute(
        "UPDATE session_rewrite_v1_state SET complete=1 WHERE id=1 AND complete=0",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn prepare_rewrite_batch(conn: &Connection, path: &str) -> Result<bool, String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let active: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_rewrite_state WHERE file_path=?1)",
            [path],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !active {
        let removed=tx.execute("DELETE FROM session_rewrite_events WHERE rowid IN (SELECT rowid FROM session_rewrite_events WHERE file_path=?1 LIMIT 32)",[path]).map_err(|e|e.to_string())?;
        if removed > 0 {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT OR IGNORE INTO session_rewrite_state(file_path) VALUES(?1)",
        [path],
    )
    .map_err(|e| e.to_string())?;
    let (cursor, phase): (i64, i64) = tx
        .query_row(
            "SELECT cursor,phase FROM session_rewrite_state WHERE file_path=?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let ready = match phase {
        0 => {
            let next:Option<i64>=tx.query_row("SELECT MAX(source_id) FROM (SELECT rowid AS source_id FROM session_usage_events WHERE file_path=?1 AND rowid>?2 ORDER BY rowid LIMIT 32)",params![path,cursor],|r|r.get(0)).map_err(|e|e.to_string())?;
            if let Some(next) = next {
                tx.execute("INSERT OR IGNORE INTO session_rewrite_events SELECT * FROM session_usage_events WHERE file_path=?1 AND rowid>?2 AND rowid<=?3",params![path,cursor,next]).map_err(|e|e.to_string())?;
                tx.execute(
                    "UPDATE session_rewrite_state SET cursor=?1 WHERE file_path=?2",
                    params![next, path],
                )
                .map_err(|e| e.to_string())?;
            } else {
                tx.execute(
                    "UPDATE session_rewrite_state SET phase=1 WHERE file_path=?1",
                    [path],
                )
                .map_err(|e| e.to_string())?;
            }
            false
        }
        1 => {
            let removed=tx.execute("DELETE FROM session_usage_events WHERE rowid IN (SELECT rowid FROM session_usage_events WHERE file_path=?1 LIMIT 32)",[path]).map_err(|e|e.to_string())?;
            if removed == 0 {
                tx.execute(
                    "UPDATE session_rewrite_state SET phase=2 WHERE file_path=?1",
                    [path],
                )
                .map_err(|e| e.to_string())?;
            }
            removed == 0
        }
        _ => true,
    };
    tx.commit().map_err(|e| e.to_string())?;
    Ok(ready)
}

fn publish_rewrite(conn: &Connection, path: &str, complete: bool) -> Result<(), String> {
    if complete {
        conn.execute(
            "DELETE FROM session_rewrite_state WHERE file_path=?1 AND phase=2",
            [path],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn cleanup_rewrite_batch(conn: &Connection) -> Result<bool, String> {
    let removed=conn.execute("DELETE FROM session_rewrite_events WHERE rowid IN (SELECT e.rowid FROM session_rewrite_events e WHERE NOT EXISTS(SELECT 1 FROM session_rewrite_state s WHERE s.file_path=e.file_path) LIMIT 32)",[]).map_err(|e|e.to_string())?;
    Ok(removed == 0)
}

fn session_report_source(conn: &Connection, start: i64, end: i64) -> String {
    format!("SELECT * FROM ({live}) WHERE file_path NOT IN (SELECT file_path FROM session_rewrite_state WHERE phase>=1)
        UNION ALL SELECT * FROM ({backup}) WHERE file_path IN (SELECT file_path FROM session_rewrite_state WHERE phase>=1)",
        live=SESSION_USAGE_ROLLUP.select(conn,start,end),backup=SESSION_REWRITE_ROLLUP.select(conn,start,end))
}
