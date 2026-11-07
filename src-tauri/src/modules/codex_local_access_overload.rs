// Capacity settings and attempt history belong to the global native API service.
static OVERLOAD_SETTING_WRITE_LOCK: TokioMutex<()> = TokioMutex::const_new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccessOverloadStatus {
    enabled: bool,
    active_requests: usize,
}

pub async fn get_local_access_overload_status() -> Result<LocalAccessOverloadStatus, String> {
    ensure_runtime_loaded_without_start_with_profile_restore(false).await?;
    let runtime = gateway_runtime().lock().await;
    Ok(LocalAccessOverloadStatus {
        enabled: runtime
            .collection
            .as_ref()
            .is_some_and(|value| value.continuous_overload_retry),
        active_requests: native_api::overload::active_recoveries(),
    })
}

pub async fn update_local_access_overload_retry(enabled: bool) -> Result<bool, String> {
    let _save = OVERLOAD_SETTING_WRITE_LOCK.lock().await;
    ensure_runtime_loaded_without_start_with_profile_restore(false).await?;
    let published = tauri::async_runtime::spawn_blocking(move || {
        let path = local_access_file_path()?;
        let mut published = None;
        update_string_atomic(&path, |current| {
            let mut collection: CodexLocalAccessCollection = match current {
                Some(json) => serde_json::from_str(json).map_err(|error| error.to_string())?,
                None => new_local_access_collection()?,
            };
            collection.continuous_overload_retry = enabled;
            collection.updated_at = now_ms().max(collection.updated_at);
            let value =
                serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())?;
            published = Some(collection);
            Ok(value)
        })?;
        published.ok_or_else(|| "OVERLOAD_SETTING_STORAGE".to_owned())
    })
    .await
    .map_err(|error| error.to_string())??;
    {
        let mut runtime = gateway_runtime().lock().await;
        if let Some(collection) = &mut runtime.collection {
            collection.continuous_overload_retry = enabled;
            collection.updated_at = collection.updated_at.max(published.updated_at);
        } else {
            sync_runtime_collection(&mut runtime, published);
        }
    }
    emit_local_access_state_updated();
    Ok(enabled)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccessOverloadAttempt {
    id: i64,
    request_id: String,
    account_id: String,
    model: String,
    sequence: u32,
    timestamp: i64,
    outcome: Option<String>,
    final_account_id: Option<String>,
}

fn create_overload_attempt_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS overload_attempts_v1 (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL,
            sequence INTEGER NOT NULL,
            account_id TEXT NOT NULL,
            model TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            outcome TEXT,
            final_account_id TEXT,
            UNIQUE(request_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_overload_attempts_time ON overload_attempts_v1(timestamp, account_id);
        CREATE INDEX IF NOT EXISTS idx_overload_attempts_account ON overload_attempts_v1(account_id, timestamp, id);
    ")
}

fn queue_overload_attempt(request_id: &str, attempt: &native_api::diagnostics::DiagnosticAttempt) {
    let epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
    queue_request_diagnostic(RequestDiagnosticWrite::OverloadAttempt(
        Box::new(LocalAccessOverloadAttempt {
            id: 0,
            request_id: request_id.to_owned(),
            account_id: attempt.account_id.clone(),
            model: attempt.model_id.clone(),
            sequence: attempt.sequence,
            timestamp: now_ms(),
            outcome: None,
            final_account_id: None,
        }),
        epoch,
    ));
}

fn queue_overload_outcome(request_id: &str, outcome: &str, account_id: Option<&str>) {
    queue_request_diagnostic(RequestDiagnosticWrite::OverloadOutcome(
        request_id.to_owned(),
        outcome.to_owned(),
        account_id.map(str::to_owned),
        REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst),
    ));
}

fn write_overload_attempt(
    conn: &Connection,
    attempt: &LocalAccessOverloadAttempt,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO overload_attempts_v1
        (request_id, sequence, account_id, model, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            attempt.request_id,
            attempt.sequence,
            attempt.account_id,
            attempt.model,
            attempt.timestamp
        ],
    )?;
    Ok(())
}

fn prune_overload_attempts(conn: &Connection, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM overload_attempts_v1 WHERE id IN (
        SELECT id FROM overload_attempts_v1 WHERE timestamp < ?1 ORDER BY timestamp LIMIT ?2
    )",
        params![
            now.saturating_sub(REQUEST_DIAGNOSTICS_RETENTION_MS),
            REQUEST_DIAGNOSTICS_PRUNE_BATCH
        ],
    )?;
    Ok(())
}

pub async fn query_local_access_overload_counts(
    start_at: i64,
    end_at: i64,
) -> Result<HashMap<String, u64>, String> {
    let permit = report_query_slots()
        .clone()
        .try_acquire_owned()
        .map_err(|_| crate::modules::background_maintenance::busy_message())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let conn = open_local_access_logs_db()?;
        let start_at = start_at.max(now_ms().saturating_sub(REQUEST_DIAGNOSTICS_RETENTION_MS));
        with_request_log_sql_budget(&conn, ACCOUNT_WINDOW_QUERY_BUDGET, || {
            let mut stmt = conn
                .prepare(
                    "SELECT account_id, COUNT(*) FROM overload_attempts_v1
                WHERE timestamp BETWEEN ?1 AND ?2 GROUP BY account_id",
                )
                .map_err(|error| error.to_string())?;
            let rows = stmt
                .query_map(params![start_at, end_at], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|error| error.to_string())?;
            rows.collect::<rusqlite::Result<HashMap<_, _>>>()
                .map_err(|error| error.to_string())
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

pub async fn query_local_access_overload_attempts(
    account_id: String,
    start_at: i64,
    end_at: i64,
    before_timestamp: Option<i64>,
    before_id: Option<i64>,
) -> Result<Vec<LocalAccessOverloadAttempt>, String> {
    let permit = report_query_slots()
        .clone()
        .try_acquire_owned()
        .map_err(|_| crate::modules::background_maintenance::busy_message())?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let conn = open_local_access_logs_db()?;
        let start_at = start_at.max(now_ms().saturating_sub(REQUEST_DIAGNOSTICS_RETENTION_MS));
        let mut stmt = conn.prepare("SELECT id, request_id, account_id, model, sequence, timestamp, outcome, final_account_id
            FROM overload_attempts_v1 WHERE account_id = ?1 AND timestamp BETWEEN ?2 AND ?3
              AND (timestamp, id) < (?4, ?5) ORDER BY timestamp DESC, id DESC LIMIT 51")
            .map_err(|error| error.to_string())?;
        let rows = stmt.query_map(params![account_id, start_at, end_at, before_timestamp.unwrap_or(i64::MAX), before_id.unwrap_or(i64::MAX)], |row| {
            Ok(LocalAccessOverloadAttempt {
                id: row.get(0)?, request_id: row.get(1)?, account_id: row.get(2)?, model: row.get(3)?,
                sequence: row.get(4)?, timestamp: row.get(5)?, outcome: row.get(6)?, final_account_id: row.get(7)?,
            })
        }).map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[cfg(test)]
#[test]
fn overload_history_counts_attempts_once_per_account_and_prunes_in_time_order() {
    let conn = Connection::open_in_memory().unwrap();
    create_overload_attempt_table(&conn).unwrap();
    let mut attempt = LocalAccessOverloadAttempt {
        id: 0,
        request_id: "turn".into(),
        account_id: "a".into(),
        model: "model".into(),
        sequence: 1,
        timestamp: 1,
        outcome: None,
        final_account_id: None,
    };
    write_overload_attempt(&conn, &attempt).unwrap();
    write_overload_attempt(&conn, &attempt).unwrap();
    attempt.sequence = 2;
    attempt.timestamp = 2;
    write_overload_attempt(&conn, &attempt).unwrap();
    attempt.sequence = 3;
    attempt.account_id = "b".into();
    write_overload_attempt(&conn, &attempt).unwrap();
    let count = |account| {
        conn.query_row(
            "SELECT COUNT(*) FROM overload_attempts_v1 WHERE account_id=?1",
            [account],
            |row| row.get::<_, u64>(0),
        )
        .unwrap()
    };
    assert_eq!(count("a"), 2);
    assert_eq!(count("b"), 1);
    apply_request_diagnostic_write(
        &conn,
        RequestDiagnosticWrite::OverloadOutcome(
            "turn".into(),
            "recovered".into(),
            Some("b".into()),
            REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst),
        ),
    )
    .unwrap();
    assert_eq!(conn.query_row(
        "SELECT COUNT(*) FROM overload_attempts_v1 WHERE outcome='recovered' AND final_account_id='b'",
        [], |row| row.get::<_, u64>(0),
    ).unwrap(), 3);
    prune_overload_attempts(&conn, REQUEST_DIAGNOSTICS_RETENTION_MS + 2).unwrap();
    assert_eq!(count("a"), 1);
    assert_eq!(count("b"), 1);
}
