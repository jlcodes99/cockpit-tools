//! Local quota observations from successful account refreshes.
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::models::quota::QuotaData;
use crate::modules::{account, cursor_account, logger};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Platform {
    Antigravity,
    Cursor,
}

impl Platform {
    fn key(self) -> &'static str {
        match self {
            Self::Antigravity => "antigravity",
            Self::Cursor => "cursor",
        }
    }
}

pub(crate) struct Metric {
    pub key: String,
    pub label: String,
    pub remaining: i32,
    pub reset_at: Option<i64>,
}

#[derive(Serialize)]
struct Series {
    key: String,
    label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Point {
    observed_at: i64,
    remaining: i32,
    reset_at: Option<i64>,
    change: Option<i32>,
}

#[derive(Default, Serialize)]
pub(crate) struct History {
    series: Vec<Series>,
    selected: Option<String>,
    points: Vec<Point>,
}

pub(crate) fn antigravity_metrics(quota: &QuotaData) -> Vec<Metric> {
    if quota.is_forbidden {
        return Vec::new();
    }
    quota
        .models
        .iter()
        .filter(|model| !model.name.is_empty() && (0..=100).contains(&model.percentage))
        .map(|model| Metric {
            key: model.name.clone(),
            label: model
                .display_name
                .clone()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| model.name.clone()),
            remaining: model.percentage,
            reset_at: chrono::DateTime::parse_from_rfc3339(&model.reset_time)
                .ok()
                .map(|time| time.timestamp_millis()),
        })
        .collect()
}

fn history_path() -> Result<std::path::PathBuf, String> {
    Ok(account::get_data_dir()?.join("platform-quota-history-v1.sqlite"))
}

fn open_writer(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(500))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS quota_observations (
            platform TEXT NOT NULL,
            account_id TEXT NOT NULL,
            account_created_at INTEGER NOT NULL,
            metric TEXT NOT NULL,
            label TEXT NOT NULL,
            observed_at INTEGER NOT NULL,
            remaining INTEGER NOT NULL,
            reset_at INTEGER,
            PRIMARY KEY(platform, account_id, account_created_at, metric, observed_at)
        ) WITHOUT ROWID;",
    )?;
    Ok(conn)
}

fn store(
    conn: &mut Connection,
    platform: Platform,
    account_id: &str,
    created_at: i64,
    observed_at: i64,
    metrics: &[Metric],
) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO quota_observations VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for metric in metrics {
            insert.execute(params![
                platform.key(),
                account_id,
                created_at,
                metric.key,
                metric.label,
                observed_at,
                metric.remaining,
                metric.reset_at
            ])?;
        }
    }
    tx.commit()
}

pub(crate) fn record(
    platform: Platform,
    account_id: &str,
    created_at: i64,
    observed_at: i64,
    metrics: &[Metric],
) {
    if metrics.is_empty() || observed_at <= 0 || crate::get_app_handle().is_none() {
        return;
    }
    let result = history_path().and_then(|path| {
        open_writer(&path)
            .and_then(|mut conn| {
                store(
                    &mut conn,
                    platform,
                    account_id,
                    created_at,
                    observed_at,
                    metrics,
                )
            })
            .map_err(|error| error.to_string())
    });
    if let Err(error) = result {
        logger::log_warn(&format!(
            "[Quota history] {} observations could not be saved: {error}",
            platform.key()
        ));
    }
}

fn read(
    conn: &Connection,
    platform: Platform,
    account_id: &str,
    created_at: i64,
    from: i64,
    to: i64,
    metric: Option<&str>,
) -> rusqlite::Result<History> {
    let mut available = conn.prepare(
        "SELECT metric, label, MAX(observed_at) FROM quota_observations
         WHERE platform=?1 AND account_id=?2 AND account_created_at=?3
           AND observed_at>=?4 AND observed_at<?5
         GROUP BY metric ORDER BY metric != 'total', metric",
    )?;
    let series = available
        .query_map(
            params![platform.key(), account_id, created_at, from, to],
            |row| {
                Ok(Series {
                    key: row.get(0)?,
                    label: row.get(1)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let Some(selected) = series
        .iter()
        .find(|series| Some(series.key.as_str()) == metric)
        .or_else(|| series.first())
    else {
        return Ok(History::default());
    };
    // Include the last observation before the interval for the first row's change.
    let mut query = conn.prepare(
        "SELECT observed_at, remaining, reset_at FROM quota_observations
         WHERE platform=?1 AND account_id=?2 AND account_created_at=?3 AND metric=?4
           AND observed_at>=COALESCE((SELECT MAX(observed_at) FROM quota_observations
               WHERE platform=?1 AND account_id=?2 AND account_created_at=?3 AND metric=?4 AND observed_at<?5), ?5)
           AND observed_at<?6 ORDER BY observed_at",
    )?;
    let mut points = Vec::new();
    let mut previous = None;
    let mut rows = query.query(params![
        platform.key(),
        account_id,
        created_at,
        selected.key,
        from,
        to
    ])?;
    while let Some(row) = rows.next()? {
        let observed_at: i64 = row.get(0)?;
        let remaining: i32 = row.get(1)?;
        let reset_at: Option<i64> = row.get(2)?;
        let change = previous
            .filter(|(_, prior_reset)| *prior_reset == reset_at)
            .map(|(prior_remaining, _)| remaining - prior_remaining);
        previous = Some((remaining, reset_at));
        if observed_at >= from {
            points.push(Point {
                observed_at,
                remaining,
                reset_at,
                change,
            });
        }
    }
    Ok(History {
        selected: Some(selected.key.clone()),
        series,
        points,
    })
}

#[tauri::command]
pub(crate) async fn get_platform_quota_history(
    platform: Platform,
    account_id: String,
    from: i64,
    to: i64,
    metric: Option<String>,
) -> Result<History, String> {
    if from < 0 || from >= to {
        return Err("invalid quota history date range".into());
    }
    // Account readers resolve the ID as a filename in the platform's account store.
    if account_id.is_empty()
        || account_id.contains(['/', '\\', ':'])
        || account_id == "."
        || account_id == ".."
    {
        return Err("invalid quota history account id".into());
    }
    tokio::task::spawn_blocking(move || {
        let created_at = match platform {
            Platform::Antigravity => account::load_account(&account_id)?.created_at,
            Platform::Cursor => cursor_account::load_account(&account_id)
                .ok_or("quota history account not found")?.created_at,
        };
        let path = history_path()?;
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Ok(History::default());
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        conn.busy_timeout(Duration::from_millis(500)).map_err(|error| error.to_string())?;
        let initialized: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='quota_observations')",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !initialized {
            return Ok(History::default());
        }
        read(&conn, platform, &account_id, created_at, from, to, metric.as_deref())
            .map_err(|error| error.to_string())
    }).await.map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_the_requested_account_generation_and_tracks_reset_boundaries() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let mut conn = open_writer(&path).unwrap();
        let save = |conn: &mut Connection,
                    platform,
                    account_id,
                    created_at,
                    observed_at,
                    key: &str,
                    remaining,
                    reset_at| {
            store(
                conn,
                platform,
                account_id,
                created_at,
                observed_at,
                &[Metric {
                    key: key.into(),
                    label: key.into(),
                    remaining,
                    reset_at,
                }],
            )
            .unwrap();
        };
        for (observed_at, remaining, reset_at) in
            [(10, 90, 35), (20, 70, 35), (40, 100, 80), (50, 96, 80)]
        {
            save(
                &mut conn,
                Platform::Cursor,
                "account",
                1,
                observed_at,
                "total",
                remaining,
                Some(reset_at),
            );
        }
        save(
            &mut conn,
            Platform::Cursor,
            "account",
            1,
            20,
            "total",
            5,
            Some(35),
        );
        save(
            &mut conn,
            Platform::Cursor,
            "account",
            1,
            30,
            "sand",
            20,
            None,
        );
        save(
            &mut conn,
            Platform::Antigravity,
            "account",
            1,
            30,
            "total",
            8,
            None,
        );
        save(
            &mut conn,
            Platform::Cursor,
            "other",
            1,
            30,
            "total",
            8,
            None,
        );
        save(
            &mut conn,
            Platform::Cursor,
            "account",
            2,
            30,
            "total",
            8,
            None,
        );
        drop(conn);

        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let result = read(&conn, Platform::Cursor, "account", 1, 20, 50, None).unwrap();
        assert_eq!(result.selected.as_deref(), Some("total"));
        assert_eq!(result.series.len(), 2);
        assert_eq!(
            result
                .points
                .iter()
                .map(|point| (
                    point.observed_at,
                    point.remaining,
                    point.reset_at,
                    point.change
                ))
                .collect::<Vec<_>>(),
            [(20, 70, Some(35), Some(-20)), (40, 100, Some(80), None)]
        );
        let sand = read(&conn, Platform::Cursor, "account", 1, 20, 50, Some("sand")).unwrap();
        assert_eq!(sand.points[0].remaining, 20);
        let empty = read(&conn, Platform::Cursor, "account", 1, 60, 100, None).unwrap();
        assert!(empty.points.is_empty());
    }

    #[test]
    fn antigravity_extracts_current_model_values_even_if_the_window_summary_failed() {
        let mut quota = QuotaData::new();
        quota.quota_summary_stale = true;
        quota.add_model(
            "model".into(),
            Some("Model".into()),
            24,
            "2026-11-16T06:00:00+08:00".into(),
        );
        quota.add_model("unknown".into(), None, -1, String::new());
        let metrics = antigravity_metrics(&quota);
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].key, "model");
        assert_eq!(metrics[0].label, "Model");
        assert_eq!(metrics[0].remaining, 24);
        assert_eq!(
            metrics[0].reset_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-11-15T22:00:00Z")
                    .unwrap()
                    .timestamp_millis()
            )
        );
        quota.is_forbidden = true;
        assert!(antigravity_metrics(&quota).is_empty());
    }
}
