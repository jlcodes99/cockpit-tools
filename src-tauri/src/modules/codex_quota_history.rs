//! Local time series of committed Codex quota observations.
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::Emitter;

use crate::models::codex::{CodexAccount, CodexQuota};
use crate::modules::{account, codex_account, logger};

const MINUTE: i64 = 60_000;
const DAY: i64 = 24 * 60 * MINUTE;
const MAX_QUERY_BUCKETS: i64 = 32 * DAY / MINUTE;
const BATCH_SIZE: usize = 32;
const PRUNE_SIZE: usize = 256;
static WRITER: OnceLock<Option<SyncSender<Sample>>> = OnceLock::new();
static CLEANUP: Mutex<()> = Mutex::new(());
const CLEANED_EVENT: &str = "codex:quota-history-cleaned";
const SETTINGS_EVENT: &str = "codex:quota-history-settings-changed";

#[path = "codex_quota_history_management.rs"]
pub(crate) mod management;
use management::{auto_prune, read_settings, HistorySettings};

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaHistoryPoint {
    observed_at: i64,
    bucket_start: i64,
    short_remaining: Option<i32>,
    weekly_remaining: Option<i32>,
}

#[derive(Default)]
struct QuotaConsumption {
    previous: Option<i32>,
    total: Option<i64>,
}

impl QuotaConsumption {
    fn observe(&mut self, remaining: Option<i32>) {
        if let (Some(previous), Some(current)) = (self.previous, remaining) {
            let decrease = i64::from(previous) - i64::from(current);
            *self.total.get_or_insert(0) += decrease.max(0);
        }
        self.previous = remaining;
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuotaHistory {
    points: Vec<QuotaHistoryPoint>,
    short_consumed: Option<i64>,
    weekly_consumed: Option<i64>,
    interval_minutes: i64,
}

struct Sample {
    account_id: String,
    identity: String,
    observed_at: i64,
    short_remaining: Option<i32>,
    weekly_remaining: Option<i32>,
}

fn account_identity(account: &CodexAccount) -> String {
    let mut hash = Sha256::new();
    hash.update(account.created_at.to_le_bytes());
    for value in [
        &account.user_id,
        &account.account_id,
        &account.organization_id,
    ] {
        let bytes = value.as_deref().unwrap_or_default().as_bytes();
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

fn window_values(quota: &CodexQuota) -> (Option<i32>, Option<i32>) {
    let mut short = None;
    let mut weekly = None;
    for (present, minutes, percentage, primary) in [
        (
            quota.hourly_window_present,
            quota.hourly_window_minutes,
            quota.hourly_percentage,
            true,
        ),
        (
            quota.weekly_window_present,
            quota.weekly_window_minutes,
            quota.weekly_percentage,
            false,
        ),
    ] {
        if present != Some(true) || !(0..=100).contains(&percentage) {
            continue;
        }
        // Some plans put the sole weekly allowance in the primary slot.
        let is_weekly = minutes.map_or(!primary, |duration| duration >= 7 * 24 * 60);
        if is_weekly {
            weekly = Some(percentage);
        } else if short.is_none() {
            short = Some(percentage);
        }
    }
    (short, weekly)
}

pub(crate) fn record(account: &CodexAccount, observed_at: i64) {
    if crate::get_app_handle().is_none() || account.is_api_key_auth() {
        return;
    }
    let Some(quota) = &account.quota else { return };
    let (short_remaining, weekly_remaining) = window_values(quota);
    if let Some(writer) = start_worker() {
        if let Err(error) = writer.try_send(Sample {
            account_id: account.id.clone(),
            identity: account_identity(account),
            observed_at,
            short_remaining,
            weekly_remaining,
        }) {
            logger::log_warn(&format!(
                "[Codex quota history] Could not queue observation: {error}"
            ));
        }
    }
}

pub(crate) fn start_maintenance() {
    start_worker();
}

fn start_worker() -> Option<&'static SyncSender<Sample>> {
    WRITER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel(256);
            match std::thread::Builder::new()
                .name("codex-quota-history".into())
                .spawn(move || write_history(receiver))
            {
                Ok(_) => Some(sender),
                Err(error) => {
                    logger::log_warn(&format!(
                        "[Codex quota history] Could not start writer: {error}"
                    ));
                    None
                }
            }
        })
        .as_ref()
}

fn history_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join("codex-quota-history-v1.sqlite"))
}

fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(Duration::from_millis(500))?;
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version >= 2 {
        return Ok(());
    }
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == 0 {
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS quota_samples (
             account_id TEXT NOT NULL,
             identity TEXT NOT NULL,
             minute INTEGER NOT NULL,
             observed_at INTEGER NOT NULL,
             short_remaining INTEGER,
             weekly_remaining INTEGER,
             PRIMARY KEY(account_id, identity, minute)
         );
         CREATE INDEX IF NOT EXISTS quota_samples_observation
             ON quota_samples(account_id, identity, observed_at);
         CREATE INDEX IF NOT EXISTS quota_samples_expiry ON quota_samples(observed_at);
         CREATE TABLE IF NOT EXISTS quota_history_settings (
             id INTEGER PRIMARY KEY CHECK(id = 1),
             settings TEXT NOT NULL DEFAULT '{}',
             last_pruned_at INTEGER
         );",
        )?;
    }
    if version == 1 {
        tx.execute(
            "UPDATE quota_history_settings SET settings=json_set(settings,'$.onlyChanges',json('true')) WHERE id=1",
            [],
        )?;
    }
    if version < 2 {
        tx.pragma_update(None, "user_version", 2)?;
    }
    tx.commit()
}

fn store_batch(conn: &mut Connection, batch: &[Sample]) -> Result<Vec<String>, String> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_error)?;
    let settings = read_settings(&tx)?;
    let ids = store_samples(&tx, batch, &settings).map_err(db_error)?;
    tx.commit().map_err(db_error)?;
    Ok(ids)
}

fn store_samples(
    conn: &Connection,
    batch: &[Sample],
    settings: &HistorySettings,
) -> rusqlite::Result<Vec<String>> {
    let mut ids = Vec::new();
    if !settings.enabled {
        return Ok(ids);
    }
    let mut latest = conn.prepare_cached(
        "SELECT observed_at, short_remaining, weekly_remaining FROM quota_samples
         WHERE account_id=?1 AND identity=?2 ORDER BY observed_at DESC LIMIT 1",
    )?;
    {
        let mut statement = conn.prepare_cached(
            "INSERT INTO quota_samples VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(account_id,identity,minute) DO UPDATE SET
                observed_at=excluded.observed_at,
                short_remaining=excluded.short_remaining,
                weekly_remaining=excluded.weekly_remaining
             WHERE excluded.observed_at >= quota_samples.observed_at",
        )?;
        for row in batch {
            if settings
                .account_ids
                .as_ref()
                .is_some_and(|ids| !ids.contains(&row.account_id))
            {
                continue;
            }
            if settings.interval_minutes > 1 || settings.only_changes {
                let previous = latest
                    .query_row(params![row.account_id, row.identity], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<i32>>(1)?,
                            row.get::<_, Option<i32>>(2)?,
                        ))
                    })
                    .optional()?;
                if previous.is_some_and(|(at, short, weekly)| {
                    row.observed_at < at
                        || (settings.interval_minutes > 1
                            && row.observed_at - at < settings.interval_minutes * MINUTE)
                        || (settings.only_changes
                            && short == row.short_remaining
                            && weekly == row.weekly_remaining)
                }) {
                    continue;
                }
            }
            if statement.execute(params![
                row.account_id,
                row.identity,
                row.observed_at.div_euclid(MINUTE),
                row.observed_at,
                row.short_remaining,
                row.weekly_remaining,
            ])? > 0
            {
                ids.push(row.account_id.clone());
            }
        }
    }
    Ok(ids)
}

fn write_history(receiver: Receiver<Sample>) {
    let mut conn = None;
    let mut next_check = Instant::now();
    let mut first_check = true;
    loop {
        let batch: Vec<_> =
            match receiver.recv_timeout(next_check.saturating_duration_since(Instant::now())) {
                Ok(first) => std::iter::once(first)
                    .chain(receiver.try_iter().take(BATCH_SIZE - 1))
                    .collect(),
                Err(RecvTimeoutError::Timeout) => Vec::new(),
                Err(RecvTimeoutError::Disconnected) => break,
            };
        let check_due = Instant::now() >= next_check;
        if check_due {
            next_check = Instant::now() + Duration::from_secs(30);
        }
        let result = (|| -> Result<(), String> {
            if conn.is_none() {
                let path = history_path()?;
                if batch.is_empty() && !path.try_exists().map_err(db_error)? {
                    return Ok(());
                }
                let opened = Connection::open(path).map_err(db_error)?;
                initialize(&opened).map_err(db_error)?;
                conn = Some(opened);
            }
            let db = conn.as_mut().expect("initialized quota history database");
            if !batch.is_empty() {
                let mut ids = store_batch(db, &batch)?;
                ids.sort_unstable();
                ids.dedup();
                if !ids.is_empty() {
                    emit_history_event("codex:quota-history-recorded", ids);
                }
            }
            if check_due || first_check {
                first_check = false;
                match auto_prune(db, chrono::Utc::now().timestamp_millis()) {
                    Ok(count) if count == PRUNE_SIZE => {
                        next_check = Instant::now() + Duration::from_millis(100)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        logger::log_warn(&format!("[Codex quota history] Prune failed: {error}"))
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            logger::log_warn(&format!("[Codex quota history] Write failed: {error}"));
        }
    }
}

fn db_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn emit_history_event(event: &str, payload: impl Serialize + Clone) {
    if let Some(app) = crate::get_app_handle() {
        if let Err(error) = app.emit(event, payload) {
            logger::log_warn(&format!(
                "[Codex quota history] Update event failed: {error}"
            ));
        }
    }
}

fn read_history(
    conn: &Connection,
    account_id: &str,
    identity: &str,
    from: i64,
    to: i64,
    interval: i64,
) -> rusqlite::Result<QuotaHistory> {
    let mut query = conn.prepare(
        "SELECT observed_at, short_remaining, weekly_remaining
         FROM quota_samples WHERE account_id=?1 AND identity=?2
           AND observed_at>=?3 AND observed_at<?4 ORDER BY observed_at",
    )?;
    let mut rows = query.query(params![account_id, identity, from, to])?;
    let mut points: Vec<QuotaHistoryPoint> = Vec::new();
    let mut short = QuotaConsumption::default();
    let mut weekly = QuotaConsumption::default();
    while let Some(row) = rows.next()? {
        let observed_at: i64 = row.get(0)?;
        let point = QuotaHistoryPoint {
            observed_at,
            bucket_start: from + (observed_at - from) / interval * interval,
            short_remaining: row.get(1)?,
            weekly_remaining: row.get(2)?,
        };
        short.observe(point.short_remaining);
        weekly.observe(point.weekly_remaining);
        if let Some(previous) = points
            .last_mut()
            .filter(|previous| previous.bucket_start == point.bucket_start)
        {
            *previous = point;
        } else {
            points.push(point);
        }
    }
    Ok(QuotaHistory {
        points,
        short_consumed: short.total,
        weekly_consumed: weekly.total,
        interval_minutes: interval / MINUTE,
    })
}

#[tauri::command]
pub(crate) async fn get_codex_quota_history(
    account_id: String,
    from: i64,
    to: i64,
    interval_minutes: i64,
) -> Result<QuotaHistory, String> {
    if from < 0 || to <= from || !(1..=24 * 60).contains(&interval_minutes) {
        return Err("CODEX_QUOTA_HISTORY_INVALID_RANGE".into());
    }
    // Preserve the existing maximum number of display points for long custom ranges.
    let interval_minutes = interval_minutes.max(1 + (to - from - 1) / (MAX_QUERY_BUCKETS * MINUTE));
    let empty = move || QuotaHistory {
        interval_minutes,
        ..QuotaHistory::default()
    };
    tokio::task::spawn_blocking(move || {
        let account = codex_account::load_account_checked(&account_id)?
            .ok_or("CODEX_QUOTA_HISTORY_ACCOUNT_NOT_FOUND")?;
        let path = history_path()?;
        if !path.try_exists().map_err(|error| error.to_string())? {
            return Ok(empty());
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        conn.busy_timeout(Duration::from_millis(500))
            .map_err(|error| error.to_string())?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if version == 0 {
            // A newly opened file may still be waiting on its first schema commit.
            return Ok(empty());
        }
        read_history(
            &conn,
            &account_id,
            &account_identity(&account),
            from,
            to,
            interval_minutes * MINUTE,
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
#[path = "codex_quota_history_tests.rs"]
mod tests;
