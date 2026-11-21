// Monthly reminders are independent of existing request admission and lifetime limits.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexMonthlyTokenAlert {
    threshold: Option<u64>,
    used: u64,
}

fn ensure_monthly_token_alerts_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS monthly_token_alerts (
            api_key_id TEXT PRIMARY KEY,
            threshold INTEGER,
            notified_month TEXT NOT NULL DEFAULT ''
         );",
    )
}

fn local_token_month(conn: &Connection, now: i64) -> rusqlite::Result<(String, i64)> {
    conn.query_row(
        "SELECT strftime('%Y-%m', ?1 / 1000, 'unixepoch', 'localtime'),
                unixepoch(?1 / 1000, 'unixepoch', 'localtime', 'start of month', 'utc') * 1000",
        [now],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

fn monthly_key_tokens(conn: &Connection, id: &str, start: i64, end: i64) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE(SUM(MAX(total_tokens, 0)), 0) FROM request_logs
         WHERE api_key_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3",
        params![id, start, end],
        |row| row.get(0),
    )
}

fn monthly_token_alert(
    conn: &Connection,
    id: &str,
    now: i64,
) -> rusqlite::Result<CodexMonthlyTokenAlert> {
    use rusqlite::OptionalExtension;
    let threshold = conn
        .query_row(
            "SELECT threshold FROM monthly_token_alerts WHERE api_key_id = ?1",
            [id],
            |row| row.get::<_, Option<u64>>(0),
        )
        .optional()?
        .flatten();
    let (_, start) = local_token_month(conn, now)?;
    let used = monthly_key_tokens(conn, id, start, now)?;
    Ok(CodexMonthlyTokenAlert { threshold, used })
}

fn save_monthly_token_threshold(
    conn: &Connection,
    id: &str,
    threshold: u64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO monthly_token_alerts (api_key_id, threshold) VALUES (?1, ?2)
         ON CONFLICT(api_key_id) DO UPDATE SET threshold = excluded.threshold",
        params![id, (threshold != 0).then_some(threshold)],
    )?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct MonthlyTokenNotice {
    id: String,
    label: String,
    threshold: u64,
    used: u64,
}

fn claim_monthly_token_alerts(
    conn: &Connection,
    keys: &std::collections::HashMap<String, String>,
    now: i64,
) -> rusqlite::Result<(String, Vec<MonthlyTokenNotice>)> {
    let (month, start) = local_token_month(conn, now)?;
    let mut query = conn.prepare(
        "SELECT api_key_id, threshold FROM monthly_token_alerts
         WHERE threshold IS NOT NULL AND notified_month != ?1",
    )?;
    let rows = query.query_map([&month], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
    })?;
    let rules = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut due = Vec::new();
    for (id, threshold) in rules {
        let Some(label) = keys.get(&id) else { continue };
        let used = monthly_key_tokens(conn, &id, start, now)?;
        if used >= threshold {
            due.push(MonthlyTokenNotice {
                id,
                label: label.clone(),
                threshold,
                used,
            });
        }
    }
    if due.is_empty() {
        return Ok((month, due));
    }
    let tx = conn.unchecked_transaction()?;
    let mut alerts = Vec::new();
    for notice in due {
        // A settings edit or another check can win between the read and the claim.
        let updated = tx.execute(
            "UPDATE monthly_token_alerts SET notified_month = ?1
             WHERE api_key_id = ?2 AND threshold = ?3 AND notified_month != ?1",
            params![month, notice.id, notice.threshold],
        )?;
        if updated != 0 {
            alerts.push(notice);
        }
    }
    tx.commit()?;
    Ok((month, alerts))
}

async fn token_alert_key_labels() -> std::collections::HashMap<String, String> {
    gateway_runtime()
        .lock()
        .await
        .collection
        .as_ref()
        .map(|collection| {
            collection
                .api_keys
                .iter()
                .map(|key| (key.id.clone(), key.label.clone()))
                .collect()
        })
        .unwrap_or_default()
}

fn release_monthly_token_alert_claim(
    conn: &Connection,
    id: &str,
    month: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE monthly_token_alerts SET notified_month = ''
         WHERE api_key_id = ?1 AND notified_month = ?2",
        params![id, month],
    )?;
    Ok(())
}

async fn dispatch_monthly_token_alerts(app: &AppHandle) -> Result<(), String> {
    let keys = token_alert_key_labels().await;
    if keys.is_empty() {
        return Ok(());
    }
    let (month, alerts) = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_local_access_logs_db()?;
        ensure_monthly_token_alerts_schema(&conn).map_err(|error| error.to_string())?;
        claim_monthly_token_alerts(&conn, &keys, now_ms()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if alerts.is_empty() {
        return Ok(());
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let locale = crate::modules::config::get_user_config().language;
        let title = crate::modules::i18n::translate(
            &locale,
            "codex.apiService.keys.monthlyTokens.title",
            &[],
        );
        #[cfg(target_os = "macos")]
        {
            // The native library shares an application identity with other notices.
            let _ = notify_rust::set_application(if tauri::is_dev() {
                "com.apple.Terminal"
            } else {
                &app.config().identifier
            });
        }
        let mut failed = Vec::new();
        for notice in alerts {
            let message = crate::modules::i18n::translate(
                &locale,
                "codex.apiService.keys.monthlyTokens.notification",
                &[
                    ("label", &notice.label),
                    ("used", &notice.used.to_string()),
                    ("threshold", &notice.threshold.to_string()),
                ],
            );
            // The Tauri plugin returns before the native send and discards its error.
            let mut notification = notify_rust::Notification::new();
            notification.summary(&title).body(&message).auto_icon().appname(
                app.config().product_name.as_deref().unwrap_or("Cockpit Tools"),
            );
            #[cfg(target_os = "windows")]
            if !tauri::is_dev() {
                notification.app_id(&app.config().identifier);
            }
            if let Err(error) = notification.show() {
                logger::log_codex_api_warn(&format!("Could not show monthly token notice: {error}"));
                failed.push(notice.id);
            }
        }
        if !failed.is_empty() {
            let conn = open_local_access_logs_db()?;
            for id in failed {
                release_monthly_token_alert_claim(&conn, &id, &month)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn schedule_monthly_token_alert_check() {
    static PENDING: AtomicBool = AtomicBool::new(false);
    let Some(app) = crate::get_app_handle().cloned() else {
        return;
    };
    if PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // Settle bursts of completed events without querying history on each request.
        tokio::time::sleep(Duration::from_secs(10)).await;
        PENDING.store(false, Ordering::Release);
        if let Err(error) = dispatch_monthly_token_alerts(&app).await {
            logger::log_codex_api_warn(&format!("Monthly token alert check failed: {error}"));
        }
    });
}

pub async fn local_access_monthly_token_alert(
    key_id: String,
    threshold: Option<u64>,
) -> Result<CodexMonthlyTokenAlert, String> {
    ensure_runtime_loaded_without_start().await?;
    if !token_alert_key_labels().await.contains_key(&key_id) {
        return Err("Codex API Service client key no longer exists.".into());
    }
    if threshold.is_some_and(|value| value > i64::MAX as u64) {
        return Err("Token threshold exceeds the supported integer range.".into());
    }
    let status = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_local_access_logs_db()?;
        ensure_monthly_token_alerts_schema(&conn).map_err(|error| error.to_string())?;
        if let Some(value) = threshold {
            let tx = conn
                .unchecked_transaction()
                .map_err(|error| error.to_string())?;
            save_monthly_token_threshold(&tx, &key_id, value).map_err(|error| error.to_string())?;
            let status =
                monthly_token_alert(&tx, &key_id, now_ms()).map_err(|error| error.to_string())?;
            tx.commit().map_err(|error| error.to_string())?;
            return Ok(status);
        }
        monthly_token_alert(&conn, &key_id, now_ms()).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    if threshold.is_some() {
        schedule_monthly_token_alert_check();
    }
    Ok(status)
}

#[cfg(test)]
mod monthly_token_alert_tests {
    use super::*;

    #[test]
    fn monthly_token_alerts_count_exact_boundaries_and_claim_once_per_key() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("usage.sqlite");
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch(
            "CREATE TABLE request_logs (api_key_id TEXT, timestamp INTEGER, total_tokens INTEGER);
             CREATE INDEX key_time ON request_logs(api_key_id, timestamp);",
        )
        .unwrap();
        ensure_monthly_token_alerts_schema(&conn).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-12T12:00:00Z")
            .unwrap()
            .timestamp_millis();
        let (month, start) = local_token_month(&conn, now).unwrap();
        assert_eq!(month, "2026-08");
        let add = |key: &str, time: i64, tokens: i64| {
            conn.execute(
                "INSERT INTO request_logs VALUES (?1, ?2, ?3)",
                params![key, time, tokens],
            )
            .unwrap();
        };
        save_monthly_token_threshold(&conn, "a", 100).unwrap();
        save_monthly_token_threshold(&conn, "b", 120).unwrap();
        add("a", start - 1, 1000);
        add("a", start, 30);
        add("a", now - 1, 69);
        add("a", now + 1, 500);
        add("b", now, 109);
        let keys = [
            ("a".to_owned(), "Key A".to_owned()),
            ("b".to_owned(), "Key B".to_owned()),
        ]
        .into_iter()
        .collect();
        assert_eq!(monthly_token_alert(&conn, "a", now).unwrap().used, 99);
        assert!(claim_monthly_token_alerts(&conn, &keys, now)
            .unwrap()
            .1
            .is_empty());
        add("a", now, 1);
        assert_eq!(
            claim_monthly_token_alerts(&conn, &keys, now).unwrap().1,
            vec![MonthlyTokenNotice {
                id: "a".to_owned(),
                label: "Key A".to_owned(),
                threshold: 100,
                used: 100,
            }]
        );
        assert!(claim_monthly_token_alerts(&conn, &keys, now)
            .unwrap()
            .1
            .is_empty());
        save_monthly_token_threshold(&conn, "b", 0).unwrap();
        assert_eq!(
            monthly_token_alert(&conn, "b", now).unwrap().threshold,
            None
        );
        drop(conn);

        let conn = Connection::open(&file).unwrap();
        assert!(claim_monthly_token_alerts(&conn, &keys, now)
            .unwrap()
            .1
            .is_empty());
        let next = now + 31 * 24 * 60 * 60 * 1000;
        assert_eq!(monthly_token_alert(&conn, "a", next).unwrap().used, 0);
        conn.execute("INSERT INTO request_logs VALUES ('a', ?1, 101)", [next])
            .unwrap();
        let alerts = claim_monthly_token_alerts(&conn, &keys, next).unwrap().1;
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].used, 101);
    }

    #[test]
    fn a_failed_notification_releases_only_its_own_monthly_claim() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE request_logs (api_key_id TEXT, timestamp INTEGER, total_tokens INTEGER);",
        )
        .unwrap();
        ensure_monthly_token_alerts_schema(&conn).unwrap();
        let now = now_ms();
        let keys = HashMap::from([
            ("first".to_owned(), "First".to_owned()),
            ("second".to_owned(), "Second".to_owned()),
        ]);
        for id in keys.keys() {
            save_monthly_token_threshold(&conn, id, 100).unwrap();
            conn.execute("INSERT INTO request_logs VALUES (?1, ?2, 100)", params![id, now])
                .unwrap();
        }
        let (month, notices) = claim_monthly_token_alerts(&conn, &keys, now).unwrap();
        assert_eq!(notices.len(), 2);
        release_monthly_token_alert_claim(&conn, "first", "2020-01").unwrap();
        assert!(claim_monthly_token_alerts(&conn, &keys, now).unwrap().1.is_empty());

        release_monthly_token_alert_claim(&conn, "first", &month).unwrap();
        let (_, notices) = claim_monthly_token_alerts(&conn, &keys, now).unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].id, "first");
    }
}
