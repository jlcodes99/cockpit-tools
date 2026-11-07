//! Repair only existing summary identities. Read fixed identity columns, never
//! request bodies or diagnostic JSON; old unknown identities stay unknown.
use super::*;

pub(crate) fn repair_batch(conn: &Connection) -> Result<bool, String> {
    if !table_exists(conn, "request_display_summaries")? {
        return Ok(true);
    }
    let diagnostics = columns(conn, "request_diagnostics")?;
    if ![
        "client_session_id",
        "client_thread_id",
        "client_identity_captured",
    ]
    .iter()
    .all(|name| diagnostics.contains(*name))
    {
        return Ok(true);
    }
    conn.execute_batch("CREATE TABLE IF NOT EXISTS request_summary_session_repair (
        version INTEGER PRIMARY KEY,cursor INTEGER NOT NULL,high_water INTEGER NOT NULL,complete INTEGER NOT NULL);
        INSERT OR IGNORE INTO request_summary_session_repair
        SELECT 1,0,COALESCE(MAX(log_id),0),0 FROM request_display_summaries;").map_err(|e|e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let (cursor, high_water, complete): (i64, i64, bool) = tx
        .query_row(
            "SELECT cursor,high_water,complete FROM request_summary_session_repair WHERE version=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    if complete {
        return Ok(true);
    }
    // Both joins are indexed; each batch allocates at most 32 * (2 * 4096 +
    // fixed numeric fields) bytes. Limit strings in SQLite before allocation.
    let mut stmt=tx.prepare("SELECT s.log_id,s.client_thread_id IS NOT NULL,s.client_identity_captured,
        d.client_identity_captured,
        CASE WHEN length(CAST(d.client_thread_id AS BLOB))<=4096 THEN d.client_thread_id END,
        CASE WHEN length(CAST(d.client_session_id AS BLOB))<=4096 THEN d.client_session_id END,
        length(CAST(d.client_thread_id AS BLOB))>4096
        FROM request_display_summaries s LEFT JOIN request_diagnostics d ON d.request_id=s.request_id
        WHERE s.log_id>?1 AND s.log_id<=?2 ORDER BY s.log_id LIMIT ?3").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                cursor,
                high_water,
                crate::modules::background_maintenance::BATCH_ROWS as i64
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, Option<bool>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, Option<bool>>(6)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);
    for (id, existing, captured, source_captured, thread, session, oversized_thread) in &rows {
        if *existing
            || *captured
            || *source_captured != Some(true)
            || *oversized_thread == Some(true)
        {
            continue;
        }
        if let Some(thread) = effective_thread(thread.clone(), session.clone()) {
            tx.execute("UPDATE request_display_summaries SET client_thread_id=?2,client_identity_captured=1
                WHERE log_id=?1 AND client_thread_id IS NULL AND client_identity_captured=0",params![id,thread]).map_err(|e|e.to_string())?;
        }
    }
    let next = rows.last().map(|row| row.0).unwrap_or(high_water);
    let complete = next >= high_water;
    tx.execute(
        "UPDATE request_summary_session_repair SET cursor=?1,complete=?2 WHERE version=1",
        params![next, complete],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(complete)
}
