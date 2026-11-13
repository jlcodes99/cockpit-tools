use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct HistorySettings {
    pub(super) enabled: bool,
    pub(super) interval_minutes: i64,
    retention_days: i64,
    auto_cleanup: bool,
    cleanup_every_days: i64,
    pub(super) account_ids: Option<Vec<String>>,
    pub(super) only_changes: bool,
    default_account_id: Option<String>,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 1,
            retention_days: 90,
            auto_cleanup: true,
            cleanup_every_days: 1,
            account_ids: None,
            only_changes: true,
            default_account_id: None,
        }
    }
}

fn settings_row(conn: &Connection) -> Result<(HistorySettings, Option<i64>), String> {
    let row = conn
        .query_row(
            "SELECT settings, last_pruned_at FROM quota_history_settings WHERE id=1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(db_error)?;
    match row {
        Some((json, last)) => Ok((serde_json::from_str(&json).map_err(db_error)?, last)),
        None => Ok((HistorySettings::default(), None)),
    }
}

pub(super) fn read_settings(conn: &Connection) -> Result<HistorySettings, String> {
    Ok(settings_row(conn)?.0)
}

fn save_settings(conn: &Connection, settings: &HistorySettings) -> Result<(), String> {
    if !(1..=1440).contains(&settings.interval_minutes)
        || !(1..=3650).contains(&settings.retention_days)
        || !(1..=3650).contains(&settings.cleanup_every_days)
    {
        return Err("CODEX_QUOTA_HISTORY_INVALID_SETTINGS".into());
    }
    conn.execute(
        "INSERT INTO quota_history_settings(id,settings) VALUES(1,?1)
         ON CONFLICT(id) DO UPDATE SET settings=excluded.settings",
        [serde_json::to_string(settings).map_err(db_error)?],
    )
    .map_err(db_error)?;
    Ok(())
}

fn open_writer(path: &std::path::Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(db_error)?;
    initialize(&conn).map_err(db_error)?;
    Ok(conn)
}

fn open_reader(path: &std::path::Path) -> Result<Option<Connection>, String> {
    if !path.try_exists().map_err(db_error)? {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(db_error)?;
    conn.busy_timeout(Duration::from_millis(500))
        .map_err(db_error)?;
    let ready = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(db_error)?
        > 0;
    Ok(ready.then_some(conn))
}

async fn background<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(task).await.map_err(db_error)?
}

#[tauri::command]
pub(crate) async fn codex_quota_history_settings() -> Result<HistorySettings, String> {
    background(|| {
        let path = history_path()?;
        if !path.try_exists().map_err(db_error)? {
            return Ok(HistorySettings::default());
        }
        read_settings(&open_writer(&path)?)
    })
    .await
}

#[tauri::command]
pub(crate) async fn set_codex_quota_history_settings(
    settings: HistorySettings,
) -> Result<HistorySettings, String> {
    let saved = background(move || {
        save_settings(&open_writer(&history_path()?)?, &settings)?;
        Ok(settings)
    })
    .await?;
    emit_history_event(SETTINGS_EVENT, ());
    Ok(saved)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryStats {
    count: u64,
    bytes: u64,
}

#[tauri::command]
pub(crate) async fn codex_quota_history_stats() -> Result<HistoryStats, String> {
    background(|| {
        let path = history_path()?;
        let count = match open_reader(&path)? {
            Some(conn) => conn
                .query_row("SELECT COUNT(*) FROM quota_samples", [], |row| row.get(0))
                .map_err(db_error)?,
            None => 0,
        };
        let mut bytes = 0;
        for suffix in ["", "-wal", "-shm"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            match std::fs::metadata(name) {
                Ok(metadata) => bytes += metadata.len(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(db_error(error)),
            }
        }
        Ok(HistoryStats { count, bytes })
    })
    .await
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum CleanupSelection {
    Expired,
    Dates { from: i64, to: i64 },
    All,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupRange {
    account_id: Option<String>,
    identity: Option<String>,
    from: i64,
    to: i64,
}

impl CleanupRange {
    fn checked(&self) -> Result<(), String> {
        if self.from < 0 || self.to <= self.from || self.to > chrono::Utc::now().timestamp_millis()
        {
            return Err("CODEX_QUOTA_HISTORY_INVALID_RANGE".into());
        }
        if account_identity_for_cleanup(self.account_id.as_deref())? != self.identity {
            return Err("CODEX_QUOTA_HISTORY_ACCOUNT_CHANGED".into());
        }
        Ok(())
    }
}

fn account_identity_for_cleanup(account_id: Option<&str>) -> Result<Option<String>, String> {
    account_id
        .map(|id| {
            let account = codex_account::load_account_checked(id)?
                .filter(|account| !account.is_api_key_auth())
                .ok_or("CODEX_QUOTA_HISTORY_ACCOUNT_NOT_FOUND")?;
            Ok(account_identity(&account))
        })
        .transpose()
}

#[derive(Serialize)]
pub(crate) struct CleanupPreview {
    range: CleanupRange,
    count: usize,
}

#[tauri::command]
pub(crate) async fn codex_quota_history_cleanup_preview(
    account_id: Option<String>,
    selection: CleanupSelection,
) -> Result<CleanupPreview, String> {
    background(move || {
        let now = chrono::Utc::now().timestamp_millis();
        let path = history_path()?;
        let (from, to) = match selection {
            CleanupSelection::Expired => {
                let settings = if path.try_exists().map_err(db_error)? {
                    read_settings(&open_writer(&path)?)?
                } else {
                    HistorySettings::default()
                };
                (0, now - settings.retention_days * DAY)
            }
            CleanupSelection::Dates { from, to } => (from, to.min(now)),
            CleanupSelection::All => (0, now),
        };
        let identity = account_identity_for_cleanup(account_id.as_deref())?;
        let range = CleanupRange {
            account_id,
            identity,
            from,
            to,
        };
        if from < 0 || to <= from {
            return Err("CODEX_QUOTA_HISTORY_INVALID_RANGE".into());
        }
        let count = match open_reader(&path)? {
            Some(conn) => range_statement(&conn, &range, false).map_err(db_error)?,
            None => 0,
        };
        Ok(CleanupPreview { range, count })
    })
    .await
}

fn range_statement(
    conn: &Connection,
    range: &CleanupRange,
    delete: bool,
) -> rusqlite::Result<usize> {
    let scope = if range.account_id.is_some() {
        " AND account_id=?3 AND identity=?4"
    } else {
        ""
    };
    let predicate = format!("FROM quota_samples WHERE observed_at>=?1 AND observed_at<?2{scope}");
    let values = params![range.from, range.to, range.account_id, range.identity];
    let values = if range.account_id.is_some() {
        values
    } else {
        &values[..2]
    };
    if delete {
        conn.execute(
            &format!(
                "DELETE FROM quota_samples WHERE rowid IN (
               SELECT rowid {predicate} ORDER BY observed_at LIMIT {PRUNE_SIZE}
             )"
            ),
            values,
        )
    } else {
        conn.query_row(&format!("SELECT COUNT(*) {predicate}"), values, |row| {
            row.get(0)
        })
    }
}

#[tauri::command]
pub(crate) async fn codex_quota_history_cleanup(range: CleanupRange) -> Result<usize, String> {
    background(move || {
        range.checked()?;
        let _guard = CLEANUP
            .try_lock()
            .map_err(|_| "CODEX_QUOTA_HISTORY_CLEANUP_BUSY".to_string())?;
        let path = history_path()?;
        if !path.try_exists().map_err(db_error)? {
            return Ok(0);
        }
        let conn = open_writer(&path)?;
        let mut removed = 0;
        let result = loop {
            match range_statement(&conn, &range, true) {
                Ok(count) => {
                    removed += count;
                    if count < PRUNE_SIZE {
                        break Ok(removed);
                    }
                }
                Err(error) => break Err(db_error(error)),
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        if removed > 0 {
            emit_history_event(CLEANED_EVENT, ());
        }
        result
    })
    .await
}

fn cleanup_due(settings: &HistorySettings, last: Option<i64>, now: i64) -> bool {
    settings.auto_cleanup
        && last.is_none_or(|at| at > now || now - at >= settings.cleanup_every_days * DAY)
}

pub(super) fn auto_prune(conn: &mut Connection, now: i64) -> Result<usize, String> {
    let (settings, last) = settings_row(conn)?;
    if !cleanup_due(&settings, last, now) {
        return Ok(0);
    }
    let Ok(_guard) = CLEANUP.try_lock() else {
        return Ok(0);
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let (settings, last) = settings_row(&tx)?;
    if !cleanup_due(&settings, last, now) {
        return Ok(0);
    }
    let range = CleanupRange {
        account_id: None,
        identity: None,
        from: 0,
        to: now - settings.retention_days * DAY,
    };
    let count = range_statement(&tx, &range, true).map_err(db_error)?;
    if count < PRUNE_SIZE {
        tx.execute(
            "INSERT INTO quota_history_settings(id,last_pruned_at) VALUES(1,?1)
             ON CONFLICT(id) DO UPDATE SET last_pruned_at=excluded.last_pruned_at",
            [now],
        )
        .map_err(db_error)?;
    }
    tx.commit().map_err(db_error)?;
    if count > 0 {
        emit_history_event(CLEANED_EVENT, ());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(conn: &Connection, account: &str, identity: &str, at: i64) {
        conn.execute(
            "INSERT INTO quota_samples VALUES(?1,?2,?3,?4,80,50)",
            params![account, identity, at / MINUTE, at],
        )
        .unwrap();
    }

    #[test]
    fn cleanup_preview_matches_scoped_deletion_and_leaves_newer_and_other_identity_rows() {
        let conn = Connection::open_in_memory().unwrap();
        initialize(&conn).unwrap();
        insert(&conn, "a", "current", MINUTE);
        insert(&conn, "a", "current", 2 * MINUTE);
        insert(&conn, "a", "previous", MINUTE);
        insert(&conn, "b", "current", MINUTE);
        let range = CleanupRange {
            account_id: Some("a".into()),
            identity: Some("current".into()),
            from: 0,
            to: 2 * MINUTE,
        };
        assert_eq!(range_statement(&conn, &range, false).unwrap(), 1);
        insert(&conn, "a", "current", 3 * MINUTE);
        assert_eq!(range_statement(&conn, &range, true).unwrap(), 1);
        assert_eq!(range_statement(&conn, &range, false).unwrap(), 0);
        let all = CleanupRange {
            account_id: None,
            identity: None,
            from: 0,
            to: 4 * MINUTE,
        };
        assert_eq!(range_statement(&conn, &all, false).unwrap(), 4);
    }

    #[test]
    fn upgrade_enables_changes_only_once_and_keeps_other_settings_and_samples() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("history.sqlite");
        let conn = open_writer(&path).unwrap();
        let old = HistorySettings {
            only_changes: false,
            interval_minutes: 15,
            retention_days: 45,
            account_ids: Some(vec!["a".into()]),
            ..HistorySettings::default()
        };
        save_settings(&conn, &old).unwrap();
        insert(&conn, "a", "identity", DAY);
        conn.pragma_update(None, "user_version", 1).unwrap();
        drop(conn);

        let upgraded = open_writer(&path).unwrap();
        assert_eq!(read_settings(&upgraded).unwrap(), HistorySettings { only_changes: true, ..old.clone() });
        let count: i64 = upgraded.query_row("SELECT COUNT(*) FROM quota_samples", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        let opted_out = HistorySettings { default_account_id: Some("a".into()), ..old };
        save_settings(&upgraded, &opted_out).unwrap();
        drop(upgraded);
        assert_eq!(read_settings(&open_writer(&path).unwrap()).unwrap(), opted_out);
    }

    #[test]
    fn retention_and_cleanup_schedule_survive_reopen() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("history.sqlite");
        let mut conn = open_writer(&path).unwrap();
        let settings = HistorySettings {
            retention_days: 3,
            cleanup_every_days: 2,
            ..HistorySettings::default()
        };
        save_settings(&conn, &settings).unwrap();
        insert(&conn, "a", "identity", DAY);
        insert(&conn, "a", "identity", 3 * DAY);
        assert_eq!(auto_prune(&mut conn, 5 * DAY).unwrap(), 1);
        drop(conn);
        let mut reopened = open_writer(&path).unwrap();
        assert_eq!(read_settings(&reopened).unwrap(), settings);
        assert_eq!(auto_prune(&mut reopened, 6 * DAY).unwrap(), 0);
        assert_eq!(auto_prune(&mut reopened, 7 * DAY).unwrap(), 1);
    }
}
