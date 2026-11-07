//! Query only future write-time summaries. No diagnostic JSON or historical
//! rebuild is required on the dialog's interactive path.
use super::*;

pub(super) fn available(conn: &Connection) -> Result<bool, String> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='request_display_summaries')", [], |row| row.get(0))
        .map_err(|error| error.to_string())
}

pub(super) fn read_page(
    conn: &Connection,
    scope: &RequestScope,
    query: &DialogQuery,
    snapshot_id: i64,
    limit: usize,
    started: Instant,
) -> Result<Value, String> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    cursor_binding(scope, query).hash(&mut hash);
    "write-time-summary-v1".hash(&mut hash);
    let binding = format!("{:016x}", hash.finish());
    let cursor = query
        .cursor
        .as_deref()
        .map(|raw| {
            decode_cursor(raw)
                .filter(|cursor| cursor.binding == binding)
                .ok_or_else(|| "dialog cursor scope changed".to_string())
        })
        .transpose()?;
    let snapshot_id = cursor
        .as_ref()
        .map_or(snapshot_id, |cursor| cursor.snapshot_id);
    let timestamp = cursor
        .as_ref()
        .map_or(MAX_SAFE_INTEGER, |cursor| cursor.timestamp);
    let id = cursor.as_ref().map_or(i64::MAX, |cursor| cursor.id);
    let (index, identity) = if query.scope == "chat" {
        ("idx_request_display_chat", "AND api_key_id=:key AND client_instance_id=:instance AND client_thread_id=:thread AND client_identity_captured=1 AND client_identity_metadata_captured=1 AND (:installation IS NULL OR client_installation_id=:installation)")
    } else {
        ("idx_request_display_timestamp", "AND (:key IS NOT NULL) AND (:instance IS NULL OR 1) AND (:thread IS NULL OR 1) AND (:installation IS NULL OR 1)")
    };
    let sql = format!("SELECT log_id,timestamp,CASE WHEN length(CAST(data_json AS BLOB))<=16384 THEN data_json ELSE NULL END FROM request_display_summaries INDEXED BY {index} WHERE dispatched=1 AND log_id<=:snapshot AND timestamp<=:timestamp AND (timestamp<:timestamp OR log_id<:id) {identity} ORDER BY timestamp DESC,log_id DESC LIMIT :limit");
    let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut rows = statement.query(rusqlite::named_params! {
        ":snapshot":snapshot_id,":timestamp":timestamp,":id":id,":key":scope.key_id,
        ":instance":scope.instance_id,":thread":query.thread_id,":installation":scope.installation_id,
        ":limit":limit as i64+1,
    }).map_err(|error| error.to_string())?;
    let mut values = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        check_budget(started)?;
        let raw: String = row
            .get(2)
            .map_err(|_| "request summary is missing or too large".to_string())?;
        let mut value: Value =
            serde_json::from_str(&raw).map_err(|_| "invalid request summary".to_string())?;
        if !value.is_object() {
            return Err("invalid request summary".into());
        }
        value["id"] = json!(row
            .get::<_, i64>(0)
            .map_err(|error| error.to_string())?
            .to_string());
        value["timestamp"] = json!(row.get::<_, i64>(1).map_err(|error| error.to_string())?);
        values.push(value);
    }
    let more = values.len() > limit;
    values.truncate(limit);
    let next_cursor = if more {
        values
            .last()
            .map(|last| {
                serde_json::to_string(&Cursor {
                    timestamp: last["timestamp"].as_i64().unwrap_or_default(),
                    id: last["id"]
                        .as_str()
                        .and_then(|id| id.parse().ok())
                        .unwrap_or_default(),
                    snapshot_id,
                    binding,
                })
                .map_err(|error| error.to_string())
            })
            .transpose()?
    } else {
        None
    };
    Ok(json!({"rows":values,"nextCursor":next_cursor}))
}
