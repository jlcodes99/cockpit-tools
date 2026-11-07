// Compatibility for a caller that supplies only a historical session ID or
// an instance ID. Full client chat queries use the direct summary index.
fn remote_request_scope(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    query: &crate::modules::codex_remote_service::KeyQuery,
) -> Result<(), String> {
    if let Some(thread) = &query.thread_id {
        let thread = uuid::Uuid::parse_str(thread)
            .map_err(|_| "invalid thread")?
            .hyphenated()
            .to_string();
        clauses.push("request_id IN (SELECT request_id FROM request_session_lookup WHERE effective_session_id=? COLLATE NOCASE)".into());
        values.push(SqlValue::Text(thread));
    }
    if let Some(instance) = &query.instance_id {
        clauses.push("client_instance_id=?".into());
        values.push(SqlValue::Text(instance.clone()));
    }
    Ok(())
}

fn query_remote_key_session(
    conn: &Connection,
    key: &str,
    query: &crate::modules::codex_remote_service::KeyQuery,
) -> Result<Value, String> {
    use rusqlite::OptionalExtension;
    let thread = query
        .thread_id
        .as_deref()
        .ok_or("session requires threadId")?;
    // Write-time summaries are usable without waiting for historical migrations.
    // Unknown old identities remain unknown.
    let present: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='request_display_summaries')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !present {
        return Ok(json!({"ready":false}));
    }
    let session = conn
        .query_row(
            "SELECT substr(r.requested_model,1,256),r.first_response_ms,s.timestamp
        FROM request_display_summaries s INDEXED BY idx_request_display_chat
        CROSS JOIN request_logs r ON r.id=s.log_id
        WHERE s.api_key_id=?1 AND s.client_instance_id=?2 AND s.client_thread_id=?3
            AND s.client_identity_captured=1 AND s.client_identity_metadata_captured=1
            AND (?4 IS NULL OR s.client_installation_id=?4)
        ORDER BY s.timestamp DESC,s.log_id DESC LIMIT 1",
            params![key, query.instance_id, thread, query.installation_id],
            |r| {
                Ok(json!({
                    "model":r.get::<_,String>(0)?,"requestedModel":r.get::<_,String>(0)?,
                    "firstResponseMs":r.get::<_,Option<i64>>(1)?,"timestamp":r.get::<_,i64>(2)?
                }))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let metrics =
        crate::modules::codex_app_injection::remote_key_session_metrics(conn, key, query)?;
    Ok(json!({"ready":true,"session":session,"metrics":metrics}))
}
