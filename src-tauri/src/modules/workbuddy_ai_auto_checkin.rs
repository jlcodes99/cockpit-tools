use super::checkin_state_store::{validate_config, CheckinStore};
pub use super::checkin_state_store::{
    AccountSchedule as WorkbuddyAiAccountScheduleState,
    CheckinConfig as WorkbuddyAiAutoCheckinConfig,
};
use super::wakeup_history_store::{self, checkin_timestamp, HistoryRow, HistoryStore};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    LazyLock, Mutex, OnceLock,
};
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{
    config, logger, workbuddy_ai_account,
    workbuddy_ai_checkin::{self, GuardedCheckinOutcome},
};

static IS_CHECKIN_RUNNING: AtomicBool = AtomicBool::new(false);
static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);
static STORAGE_LOCK: Mutex<()> = Mutex::new(());
static SCHEDULER_WAKE: OnceLock<Notify> = OnceLock::new();
static ACCOUNT_RETRY_STATES: LazyLock<Mutex<HashMap<String, AccountRetryState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PENDING_LOCAL_WRITES: LazyLock<Mutex<PendingLocalWrites>> =
    LazyLock::new(|| Mutex::new(PendingLocalWrites::default()));
static PENDING_FLUSH_LOCK: Mutex<()> = Mutex::new(());

const SCHEDULER_POLL_DELAY: Duration = Duration::from_secs(30);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone)]
struct AccountRetryState {
    date: String,
    next_retry_at: Instant,
    next_retry_wall_time: DateTime<Local>,
    retry_delay: Duration,
    permanent_failed: bool,
}

#[derive(Debug, Default)]
struct PendingLocalWrites {
    schedules: HashMap<String, (String, i32)>,
    in_flight_schedules: HashMap<String, (String, i32)>,
    logs: BTreeMap<String, WorkbuddyAiAutoCheckinLogRecord>,
}

struct CheckinGuard;
impl Drop for CheckinGuard {
    fn drop(&mut self) {
        IS_CHECKIN_RUNNING.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAiAutoCheckinAccountDetail {
    pub account_id: String,
    pub email: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAiAutoCheckinLogRecord {
    pub id: String,
    pub timestamp: String,
    pub date: String,
    pub duration_ms: u64,
    pub total_accounts: usize,
    pub success_count: usize,
    pub already_checked_count: usize,
    #[serde(default)]
    pub inactive_count: usize,
    #[serde(default)]
    pub pending_count: usize,
    pub failed_count: usize,
    pub status: String,
    pub details: Vec<WorkbuddyAiAutoCheckinAccountDetail>,
}

pub(crate) const CONFIG_FILE: &str = "workbuddy_ai_auto_checkin_config.json";

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join(CONFIG_FILE)
}

pub(crate) const LOGS_FILE: &str = "workbuddy_ai_auto_checkin_logs.json";

fn get_logs_file_path() -> PathBuf {
    config::get_shared_dir().join(LOGS_FILE)
}

fn scheduler_wake() -> &'static Notify {
    SCHEDULER_WAKE.get_or_init(Notify::new)
}

fn wake_scheduler() {
    scheduler_wake().notify_one();
}

fn clear_all_retry_states() {
    if let Ok(mut map) = ACCOUNT_RETRY_STATES.lock() {
        map.clear();
    }
}

pub fn clear_account_retry_state(account_id: &str) {
    if let Ok(mut map) = ACCOUNT_RETRY_STATES.lock() {
        map.remove(account_id);
    }
}

fn record_account_retry_failure(account_id: &str, today: &str, permanent: bool) {
    if let Ok(mut map) = ACCOUNT_RETRY_STATES.lock() {
        let now = Instant::now();
        let now_local = Local::now();
        let next_delay = match map.get(account_id) {
            Some(prev) if prev.date == today && !prev.permanent_failed => {
                next_retry_delay(prev.retry_delay)
            }
            _ => INITIAL_RETRY_DELAY,
        };
        let next_retry_wall_time = now_local
            + ChronoDuration::from_std(next_delay)
                .unwrap_or_else(|_| ChronoDuration::seconds(next_delay.as_secs() as i64));
        map.insert(
            account_id.to_string(),
            AccountRetryState {
                date: today.to_string(),
                next_retry_at: now + next_delay,
                next_retry_wall_time,
                retry_delay: next_delay,
                permanent_failed: permanent,
            },
        );
    }
}

fn queue_pending_schedule_mark_in(
    queue: &Mutex<PendingLocalWrites>,
    account_id: &str,
    date: &str,
    minute: i32,
) {
    if let Ok(mut pending) = queue.lock() {
        let should_insert = pending
            .schedules
            .get(account_id)
            .is_none_or(|(existing_date, _)| existing_date.as_str() <= date);
        if should_insert {
            pending
                .schedules
                .insert(account_id.to_string(), (date.to_string(), minute));
        }
    }
}

fn queue_pending_schedule_mark(account_id: &str, date: &str, minute: i32) {
    queue_pending_schedule_mark_in(&PENDING_LOCAL_WRITES, account_id, date, minute);
}

fn queue_pending_log_record_in(
    queue: &Mutex<PendingLocalWrites>,
    record: WorkbuddyAiAutoCheckinLogRecord,
) {
    if let Ok(mut pending) = queue.lock() {
        let date = record.date.clone();
        if let Some(existing) = pending.logs.get_mut(&date) {
            merge_log_record(existing, record);
        } else {
            pending.logs.insert(date, record);
        }
        while pending.logs.len() > 30 {
            let Some(oldest) = pending.logs.keys().next().cloned() else {
                break;
            };
            pending.logs.remove(&oldest);
        }
    }
}

fn queue_pending_log_record(record: WorkbuddyAiAutoCheckinLogRecord) {
    queue_pending_log_record_in(&PENDING_LOCAL_WRITES, record);
}

fn requeue_pending_log_record(
    pending: &mut PendingLocalWrites,
    failed_older: WorkbuddyAiAutoCheckinLogRecord,
) {
    let date = failed_older.date.clone();
    if let Some(newer) = pending.logs.remove(&date) {
        let mut combined = failed_older;
        merge_log_record(&mut combined, newer);
        pending.logs.insert(date, combined);
    } else {
        pending.logs.insert(date, failed_older);
    }
    while pending.logs.len() > 30 {
        let Some(oldest) = pending.logs.keys().next().cloned() else {
            break;
        };
        pending.logs.remove(&oldest);
    }
}

fn pending_schedule_snapshot_from(
    queue: &Mutex<PendingLocalWrites>,
) -> HashMap<String, (String, i32)> {
    let Ok(pending) = queue.lock() else {
        return HashMap::new();
    };
    let mut combined = pending.in_flight_schedules.clone();
    for (account_id, (date, minute)) in &pending.schedules {
        let should_overwrite = combined
            .get(account_id)
            .is_none_or(|(in_flight_date, _)| date.as_str() >= in_flight_date.as_str());
        if should_overwrite {
            combined.insert(account_id.clone(), (date.clone(), *minute));
        }
    }
    combined
}

fn pending_schedule_snapshot() -> HashMap<String, (String, i32)> {
    pending_schedule_snapshot_from(&PENDING_LOCAL_WRITES)
}

fn flush_pending_local_writes_with_ops<FS, FL>(
    flush_lock: &Mutex<()>,
    queue: &Mutex<PendingLocalWrites>,
    mut flush_schedules: FS,
    mut flush_log: FL,
) -> bool
where
    FS: FnMut(&HashMap<String, (String, i32)>) -> Result<(), String>,
    FL: FnMut(WorkbuddyAiAutoCheckinLogRecord) -> Result<(), String>,
{
    // Serialize backfill consumers so concurrent callers can never flush the same snapshot twice.
    let Ok(_flush_guard) = flush_lock.lock() else {
        return false;
    };

    // Atomically drain the current batch under queue lock so records enqueued concurrently
    // (even within the same second) remain untouched in `queue` and can never be deleted.
    let (drained_schedules, drained_logs) = {
        let Ok(mut pending) = queue.lock() else {
            return false;
        };
        if pending.schedules.is_empty() && pending.logs.is_empty() {
            return false;
        }
        let schedules = std::mem::take(&mut pending.schedules);
        pending.in_flight_schedules = schedules.clone();
        let logs = std::mem::take(&mut pending.logs);
        (schedules, logs)
    };

    let mut flushed_any = false;

    if !drained_schedules.is_empty() {
        match flush_schedules(&drained_schedules) {
            Ok(()) => {
                flushed_any = true;
                if let Ok(mut pending) = queue.lock() {
                    pending.in_flight_schedules.clear();
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[WorkbuddyAiAutoCheckin] 补写待落盘调度标记失败（稍后重试）: {err}"
                ));
                if let Ok(mut pending) = queue.lock() {
                    pending.in_flight_schedules.clear();
                    for (account_id, (date, minute)) in drained_schedules {
                        let should_restore = pending
                            .schedules
                            .get(&account_id)
                            .is_none_or(|(newer_date, _)| newer_date.as_str() < date.as_str());
                        if should_restore {
                            pending.schedules.insert(account_id, (date, minute));
                        }
                    }
                }
            }
        }
    }

    if !drained_logs.is_empty() {
        for (_date, record) in drained_logs {
            match flush_log(record.clone()) {
                Ok(()) => {
                    flushed_any = true;
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[WorkbuddyAiAutoCheckin] 补写待落盘签到历史失败（稍后重试）: {err}"
                    ));
                    if let Ok(mut pending) = queue.lock() {
                        requeue_pending_log_record(&mut pending, record);
                    }
                }
            }
        }
    }

    flushed_any
}

fn flush_pending_local_writes_in(
    flush_lock: &Mutex<()>,
    queue: &Mutex<PendingLocalWrites>,
    store: &CheckinStore,
    history: &HistoryStore,
) -> bool {
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    flush_pending_local_writes_with_ops(
        flush_lock,
        queue,
        |schedules_batch| {
            let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
            let config = store.load()?;
            let previous = config.account_schedules.clone();
            let mut schedules = previous.clone().unwrap_or_default();
            for (account_id, (today, minute)) in schedules_batch {
                mark_schedule_checked(&mut schedules, account_id, today, *minute);
            }
            store.save_schedules(previous.as_ref(), Some(schedules))
        },
        |record| {
            let cutoff = Local::now().timestamp() - THIRTY_DAYS_SECS;
            history.merge_day(record, cutoff, merge_log_record)
        },
    )
}

fn flush_pending_local_writes() -> bool {
    flush_pending_local_writes_in(
        &PENDING_FLUSH_LOCK,
        &PENDING_LOCAL_WRITES,
        &state_store(),
        &history_store(),
    )
}

fn state_store() -> CheckinStore {
    state_store_at(get_config_file_path())
}

pub(super) fn state_store_at(path: PathBuf) -> CheckinStore {
    CheckinStore::new(path, "workbuddy_ai")
}

pub fn get_config_checked() -> Result<WorkbuddyAiAutoCheckinConfig, String> {
    state_store().load()
}

pub fn save_config(config: &WorkbuddyAiAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    state_store().save_rules(config)?;
    clear_all_retry_states();
    wake_scheduler();
    Ok(())
}

fn ensure_and_save_account_schedules_in(
    store: &CheckinStore,
    accounts: &[WorkbuddyAccount],
    allow_when_disabled: bool,
) -> Result<(WorkbuddyAiAutoCheckinConfig, bool), String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    let mut config = store.load()?;
    if !config.enabled && !allow_when_disabled {
        return Ok((config, false));
    }
    let previous = config.account_schedules.clone();
    let changed = ensure_account_schedules(&mut config, accounts);
    if changed {
        store.save_schedules(previous.as_ref(), config.account_schedules.clone())?;
    }
    Ok((config, changed))
}

impl HistoryRow for WorkbuddyAiAutoCheckinLogRecord {
    fn id(&self) -> &str {
        &self.date
    }
    fn timestamp(&self) -> i64 {
        checkin_timestamp(&self.timestamp)
    }
}

fn history_store() -> HistoryStore {
    history_store_at(get_logs_file_path())
}

pub(crate) fn history_store_at(path: PathBuf) -> HistoryStore {
    wakeup_history_store::daily_checkin(path, "workbuddy_ai")
}

pub fn schedule_log_migration() {
    history_store().schedule::<WorkbuddyAiAutoCheckinLogRecord>();
}

pub fn get_logs_checked(
    retry: bool,
) -> Result<wakeup_history_store::HistoryRead<WorkbuddyAiAutoCheckinLogRecord>, String> {
    let _ = flush_pending_local_writes();
    wakeup_history_store::read_available(history_store(), retry)
}

fn clear_logs_in(
    flush_lock: &Mutex<()>,
    queue: &Mutex<PendingLocalWrites>,
    history: &HistoryStore,
) -> Result<(), String> {
    let _flush_guard = flush_lock
        .lock()
        .map_err(|_| "自动签到历史补写锁已损坏".to_string())?;
    history.clear()?;
    if let Ok(mut pending) = queue.lock() {
        pending.logs.clear();
    }
    Ok(())
}

pub fn clear_logs() -> Result<(), String> {
    clear_logs_in(&PENDING_FLUSH_LOCK, &PENDING_LOCAL_WRITES, &history_store())
}

struct BatchSummary {
    total_accounts: usize,
    success_count: usize,
    already_checked_count: usize,
    inactive_count: usize,
    pending_count: usize,
    failed_count: usize,
    status: &'static str,
}

fn summarize_details(details: &[WorkbuddyAiAutoCheckinAccountDetail]) -> BatchSummary {
    let total_accounts = details.len();
    let success_count = details.iter().filter(|d| d.status == "success").count();
    let already_checked_count = details
        .iter()
        .filter(|d| d.status == "already_checked")
        .count();
    let inactive_count = details.iter().filter(|d| d.status == "inactive").count();
    let pending_count = details.iter().filter(|d| d.status == "pending").count();
    let failed_count = details.iter().filter(|d| d.status == "failed").count();
    let status = if total_accounts == 0 {
        "no_accounts"
    } else if inactive_count == total_accounts {
        "inactive"
    } else if failed_count == 0 && pending_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else if pending_count > 0 && failed_count == 0 {
        "pending"
    } else {
        "failed"
    };
    BatchSummary {
        total_accounts,
        success_count,
        already_checked_count,
        inactive_count,
        pending_count,
        failed_count,
        status,
    }
}

fn is_confirmed_detail_status(status: &str) -> bool {
    matches!(status, "success" | "already_checked")
}

fn detail_status_rank(status: &str) -> u8 {
    if is_confirmed_detail_status(status) {
        2
    } else if status == "pending" {
        1
    } else {
        0
    }
}

fn merge_log_record(
    existing: &mut WorkbuddyAiAutoCheckinLogRecord,
    record: WorkbuddyAiAutoCheckinLogRecord,
) {
    let incoming_is_older = record.timestamp < existing.timestamp;
    let mut details: HashMap<String, WorkbuddyAiAutoCheckinAccountDetail> = existing
        .details
        .drain(..)
        .map(|detail| (detail.account_id.clone(), detail))
        .collect();
    for mut incoming in record.details {
        if let Some(prev) = details.get(&incoming.account_id) {
            let prev_rank = detail_status_rank(&prev.status);
            let incoming_rank = detail_status_rank(&incoming.status);
            // Enforce status non-downgrade precedence:
            // confirmed ("success" | "already_checked") > "pending" > other ("failed" | "inactive"),
            // and never let an older same-rank backfill overwrite a newer detail.
            if prev_rank > incoming_rank || (prev_rank == incoming_rank && incoming_is_older) {
                let mut preserved = prev.clone();
                if !incoming_is_older {
                    preserved.time = incoming.time.or(prev.time.clone());
                } else if preserved.time.is_none() {
                    preserved.time = incoming.time;
                }
                if preserved.model_id.is_none() && incoming.model_id.is_some() {
                    preserved.model_id = incoming.model_id;
                }
                if preserved.credit.is_none() && incoming.credit.is_some() {
                    preserved.credit = incoming.credit;
                }
                details.insert(preserved.account_id.clone(), preserved);
                continue;
            }
            if incoming.model_id.is_none() && prev.model_id.is_some() {
                incoming.model_id = prev.model_id.clone();
            }
            if incoming.credit.is_none() && prev.credit.is_some() {
                incoming.credit = prev.credit.clone();
            }
        }
        details.insert(incoming.account_id.clone(), incoming);
    }

    if existing.timestamp < record.timestamp {
        existing.timestamp = record.timestamp;
    }
    existing.duration_ms = existing.duration_ms.saturating_add(record.duration_ms);
    existing.details = details.into_values().collect();
    let summary = summarize_details(&existing.details);
    existing.total_accounts = summary.total_accounts;
    existing.success_count = summary.success_count;
    existing.already_checked_count = summary.already_checked_count;
    existing.inactive_count = summary.inactive_count;
    existing.pending_count = summary.pending_count;
    existing.failed_count = summary.failed_count;
    existing.status = summary.status.to_string();
}

fn add_log_record(record: WorkbuddyAiAutoCheckinLogRecord) -> Result<(), String> {
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    history_store().merge_day(
        record,
        Local::now().timestamp() - THIRTY_DAYS_SECS,
        merge_log_record,
    )
}

async fn persist_log_record(record: WorkbuddyAiAutoCheckinLogRecord) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || add_log_record(record))
        .await
        .map_err(|error| format!("保存签到历史任务失败: {error}"))?
}

pub fn parse_time_to_minutes(time_str: &str) -> i32 {
    let parts: Vec<&str> = time_str.split(':').collect();
    let h = parts
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let m = parts
        .get(1)
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    h * 60 + m
}

pub fn get_today_date_string() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn format_time_only() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

pub fn ensure_account_schedules(
    config: &mut WorkbuddyAiAutoCheckinConfig,
    accounts: &[WorkbuddyAccount],
) -> bool {
    let today_str = get_today_date_string();
    let start_min = parse_time_to_minutes(&config.start_time);
    let mut end_min = parse_time_to_minutes(&config.end_time);
    if end_min < start_min {
        end_min = start_min;
    }
    let min_range = (end_min - start_min).max(0);

    let mut schedules = config.account_schedules.clone().unwrap_or_default();
    let mut changed = false;

    for account in accounts {
        let existing = schedules.get(&account.id);
        if let Some(sch) = existing {
            if sch.scheduled_date == today_str
                && sch.scheduled_minute >= start_min
                && sch.scheduled_minute <= end_min
            {
                continue;
            }
        }

        let random_offset = if min_range > 0 {
            (rand::random::<u32>() % (min_range as u32 + 1)) as i32
        } else {
            0
        };
        let scheduled_minute = start_min + random_offset;

        let last_checked = existing.and_then(|e| {
            e.last_checked_date
                .as_deref()
                .filter(|d| *d >= today_str.as_str())
                .map(str::to_owned)
        });

        schedules.insert(
            account.id.clone(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today_str.clone(),
                scheduled_minute,
                last_checked_date: last_checked,
            },
        );
        changed = true;
    }

    if changed {
        config.account_schedules = Some(schedules);
    }
    changed
}

fn mark_schedule_checked(
    schedules: &mut HashMap<String, WorkbuddyAiAccountScheduleState>,
    account_id: &str,
    date: &str,
    current_minute: i32,
) {
    let schedule = schedules.entry(account_id.to_string()).or_insert_with(|| {
        WorkbuddyAiAccountScheduleState {
            scheduled_date: date.to_string(),
            scheduled_minute: current_minute,
            last_checked_date: None,
        }
    });
    // Advance scheduled_date alongside Day-2+ last_checked_date updates so SQLite `put_schedule`
    // (which preserves old last_checked_date when excluded.scheduled_date == schedules.scheduled_date
    // and schedules.last_checked_date == excluded.scheduled_date) never silently drops the mark.
    if schedule.scheduled_date.as_str() < date {
        schedule.scheduled_date = date.to_string();
    }
    if schedule
        .last_checked_date
        .as_deref()
        .is_none_or(|existing| existing < date)
    {
        schedule.last_checked_date = Some(date.to_string());
    }
}

fn is_account_due_at(
    platform_enabled: bool,
    config: &WorkbuddyAiAutoCheckinConfig,
    account_id: &str,
    today_str: &str,
    current_minute: i32,
    now_instant: Instant,
    retry_states: &HashMap<String, AccountRetryState>,
    pending_schedules: &HashMap<String, (String, i32)>,
) -> bool {
    if !platform_enabled || !config.enabled {
        return false;
    }
    let start_min = parse_time_to_minutes(&config.start_time);
    let end_min = parse_time_to_minutes(&config.end_time).max(start_min);
    if current_minute < start_min || current_minute > end_min {
        return false;
    }
    if pending_schedules
        .get(account_id)
        .is_some_and(|(date, _)| date.as_str() >= today_str)
    {
        return false;
    }
    let Some(s) = config
        .account_schedules
        .as_ref()
        .and_then(|map| map.get(account_id))
    else {
        return false;
    };
    if s.last_checked_date
        .as_deref()
        .is_some_and(|date| date >= today_str)
    {
        return false;
    }
    if s.scheduled_date != today_str || current_minute < s.scheduled_minute {
        return false;
    }
    if let Some(retry) = retry_states.get(account_id) {
        if retry.date == today_str && (retry.permanent_failed || now_instant < retry.next_retry_at)
        {
            return false;
        }
    }
    true
}

fn select_due_accounts<'a>(
    config: &WorkbuddyAiAutoCheckinConfig,
    accounts: &'a [WorkbuddyAccount],
    today_str: &str,
    current_minute: i32,
    now_instant: Instant,
    retry_states: &HashMap<String, AccountRetryState>,
    pending_schedules: &HashMap<String, (String, i32)>,
    force: bool,
) -> Vec<&'a WorkbuddyAccount> {
    if force {
        return accounts.iter().collect();
    }
    accounts
        .iter()
        .filter(|account| {
            is_account_due_at(
                true,
                config,
                &account.id,
                today_str,
                current_minute,
                now_instant,
                retry_states,
                pending_schedules,
            )
        })
        .collect()
}

fn account_display_label(account: &WorkbuddyAccount) -> String {
    if !account.email.trim().is_empty() {
        account.email.clone()
    } else if let Some(nickname) = account
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        nickname.to_string()
    } else {
        account.id.clone()
    }
}

fn detail_from_result(
    account_id: &str,
    email: String,
    time: String,
    result: &Result<workbuddy_ai_checkin::WorkbuddyAiCheckinResult, String>,
) -> (WorkbuddyAiAutoCheckinAccountDetail, bool, Option<String>) {
    match result {
        Ok(res) if res.success => {
            let status = if res.already_claimed {
                "already_checked"
            } else {
                "success"
            };
            (
                WorkbuddyAiAutoCheckinAccountDetail {
                    account_id: account_id.to_string(),
                    email,
                    status: status.to_string(),
                    time: Some(time),
                    result_code: res.result_code.clone(),
                    model_id: res.model_id.clone(),
                    message: res.warning.clone(),
                    credit: res.credit.map(|c| serde_json::json!(c)),
                },
                true,
                res.activity_date.clone(),
            )
        }
        Ok(res) if res.inactive => (
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: account_id.to_string(),
                email,
                status: "inactive".to_string(),
                time: Some(time),
                result_code: res
                    .result_code
                    .clone()
                    .or_else(|| Some("inactive".to_string())),
                model_id: None,
                message: None,
                credit: None,
            },
            true,
            res.activity_date.clone(),
        ),
        Ok(res) if res.pending_confirmation => (
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: account_id.to_string(),
                email,
                status: "pending".to_string(),
                time: Some(time),
                result_code: res
                    .result_code
                    .clone()
                    .or_else(|| Some("pending_confirmation".to_string())),
                model_id: res.model_id.clone(),
                message: res.warning.clone(),
                credit: None,
            },
            true,
            res.activity_date.clone(),
        ),
        Ok(res) => (
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: account_id.to_string(),
                email,
                status: "failed".to_string(),
                time: Some(time),
                result_code: res.result_code.clone(),
                model_id: res.model_id.clone(),
                message: res.message.clone().or_else(|| res.warning.clone()),
                credit: None,
            },
            false,
            res.activity_date.clone(),
        ),
        Err(err) => (
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: account_id.to_string(),
                email,
                status: "failed".to_string(),
                time: Some(time),
                result_code: Some(classify_error_code(err).to_string()),
                model_id: None,
                message: Some(err.clone()),
                credit: None,
            },
            false,
            None,
        ),
    }
}

fn classify_error_code(err: &str) -> &'static str {
    let trimmed = err.trim();
    if trimmed == "WORKBUDDY_AI_NO_FREE_MODEL" {
        "no_free_model"
    } else if workbuddy_ai_checkin::is_permanent_error(trimmed) {
        "token_expired"
    } else {
        "error"
    }
}

fn classify_cycle_outcome(
    summary: &BatchSummary,
    retry_needed: bool,
    can_background_retry: bool,
) -> &'static str {
    if retry_needed && can_background_retry {
        "retry_scheduled"
    } else if summary.failed_count > 0 {
        if summary.success_count > 0 || summary.already_checked_count > 0 {
            "partial_failed"
        } else {
            "failed"
        }
    } else {
        "completed"
    }
}

fn group_details_by_date(
    items: Vec<(String, WorkbuddyAiAutoCheckinAccountDetail)>,
) -> BTreeMap<String, Vec<WorkbuddyAiAutoCheckinAccountDetail>> {
    let mut grouped: BTreeMap<String, Vec<WorkbuddyAiAutoCheckinAccountDetail>> = BTreeMap::new();
    for (date, detail) in items {
        grouped.entry(date).or_default().push(detail);
    }
    grouped
}

fn resolve_effective_date(activity_date: Option<&str>, local_today: &str) -> String {
    activity_date
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or(local_today)
        .to_string()
}

/// Clamp local-calendar schedule marks to `min(effective_date, local_today)`:
/// - If server `activity_date < local_today` (cross-midnight D -> D+1 or server-behind-local),
///   keep `effective_date` so local `D+1` is NOT falsely skipped.
/// - If server `activity_date > local_today` (western timezones where server is already D+1),
///   stamp `local_today` (D) so local D is done while local D+1 still runs normally for server D+2.
fn resolve_schedule_mark_date<'a>(effective_date: &'a str, local_today: &'a str) -> &'a str {
    if effective_date < local_today {
        effective_date
    } else {
        local_today
    }
}

fn mark_accounts_checked_in(
    store: &CheckinStore,
    marks: &[(String, String, i32)],
) -> Result<(), String> {
    if marks.is_empty() {
        return Ok(());
    }
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    let config = store.load()?;
    let previous = config.account_schedules.clone();
    let mut schedules = previous.clone().unwrap_or_default();
    for (account_id, date, minute) in marks {
        mark_schedule_checked(&mut schedules, account_id, date, *minute);
    }
    store.save_schedules(previous.as_ref(), Some(schedules))
}

fn mark_single_account_checked_in(
    store: &CheckinStore,
    account_id: &str,
    date: &str,
    current_minute: i32,
) -> Result<(), String> {
    let mark = [(account_id.to_string(), date.to_string(), current_minute)];
    mark_accounts_checked_in(store, &mark)
}

fn mark_single_account_checked_sync(account_id: &str, date: &str) -> Result<(), String> {
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    mark_single_account_checked_in(&state_store(), account_id, date, current_minute)
}

pub(crate) fn record_manual_result(
    account_id: &str,
    result: &Result<workbuddy_ai_checkin::WorkbuddyAiCheckinResult, String>,
    duration_ms: u64,
) -> Result<(), String> {
    let _ = flush_pending_local_writes();
    let email = workbuddy_ai_account::load(account_id)
        .map(|account| account_display_label(&account))
        .unwrap_or_else(|_| account_id.to_string());
    let now = Local::now();
    let local_today = now.format("%Y-%m-%d").to_string();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let (detail, should_mark_schedule, activity_date) =
        detail_from_result(account_id, email, format_time_only(), result);
    let log_date = resolve_effective_date(activity_date.as_deref(), &local_today);
    let schedule_mark_date = resolve_schedule_mark_date(&log_date, &local_today);

    let mut errors = Vec::new();
    if should_mark_schedule {
        clear_account_retry_state(account_id);
        if let Err(err) = mark_single_account_checked_sync(account_id, schedule_mark_date) {
            queue_pending_schedule_mark(account_id, schedule_mark_date, current_minute);
            logger::log_warn(&format!(
                "[WorkbuddyAiAutoCheckin] 手动打卡后更新调度标记失败（已入队等待补写）: {err}"
            ));
            errors.push(format!("schedule: {err}"));
        }
    }

    let details = vec![detail];
    let summary = summarize_details(&details);
    let log_record = WorkbuddyAiAutoCheckinLogRecord {
        id: format!(
            "manual_{}_{}",
            Local::now().timestamp_millis(),
            rand::random::<u16>()
        ),
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        date: log_date,
        duration_ms,
        total_accounts: summary.total_accounts,
        success_count: summary.success_count,
        already_checked_count: summary.already_checked_count,
        inactive_count: summary.inactive_count,
        pending_count: summary.pending_count,
        failed_count: summary.failed_count,
        status: summary.status.into(),
        details,
    };
    if let Err(err) = add_log_record(log_record.clone()) {
        queue_pending_log_record(log_record);
        errors.push(format!("history: {err}"));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn reconcile_confirmed_status(
    account_id: &str,
    activity_date: &str,
    model_id: Option<String>,
) -> Result<(), String> {
    let _ = flush_pending_local_writes();
    let email = workbuddy_ai_account::load(account_id)
        .map(|account| account_display_label(&account))
        .unwrap_or_else(|_| account_id.to_string());
    let now = Local::now();
    let local_today = now.format("%Y-%m-%d").to_string();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let log_date = resolve_effective_date(Some(activity_date), &local_today);
    let schedule_mark_date = resolve_schedule_mark_date(&log_date, &local_today);

    if log_date.as_str() >= local_today.as_str() {
        clear_account_retry_state(account_id);
    }
    let mut errors = Vec::new();
    if let Err(err) = mark_single_account_checked_sync(account_id, schedule_mark_date) {
        queue_pending_schedule_mark(account_id, schedule_mark_date, current_minute);
        logger::log_warn(&format!(
            "[WorkbuddyAiAutoCheckin] 状态对账更新调度标记失败（已入队等待补写）: {err}"
        ));
        errors.push(format!("schedule: {err}"));
    }

    let detail = WorkbuddyAiAutoCheckinAccountDetail {
        account_id: account_id.to_string(),
        email,
        status: "already_checked".to_string(),
        time: Some(format_time_only()),
        result_code: Some("reconciled_active".to_string()),
        model_id,
        message: None,
        credit: None,
    };
    let details = vec![detail];
    let summary = summarize_details(&details);
    let log_record = WorkbuddyAiAutoCheckinLogRecord {
        id: format!(
            "reconcile_{}_{}",
            Local::now().timestamp_millis(),
            rand::random::<u16>()
        ),
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        date: log_date,
        duration_ms: 0,
        total_accounts: summary.total_accounts,
        success_count: summary.success_count,
        already_checked_count: summary.already_checked_count,
        inactive_count: summary.inactive_count,
        pending_count: summary.pending_count,
        failed_count: summary.failed_count,
        status: summary.status.into(),
        details,
    };
    if let Err(err) = add_log_record(log_record.clone()) {
        queue_pending_log_record(log_record);
        logger::log_warn(&format!(
            "[WorkbuddyAiAutoCheckin] 状态对账更新历史失败（已入队等待补写）: {err}"
        ));
        errors.push(format!("history: {err}"));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn is_account_still_due_locked(account_id: &str) -> bool {
    let platform_enabled = crate::modules::tray_layout::is_platform_enabled("workbuddy_ai");
    if !platform_enabled {
        return false;
    }
    // Capture pending/in-flight schedule snapshot BEFORE reading SQLite so items moving
    // one-way from queue/in_flight -> SQLite -> clear cannot slip between the two reads.
    let pending_schedules = pending_schedule_snapshot();
    let Ok(latest_cfg) = get_config_checked() else {
        return false;
    };
    let now = Local::now();
    let fresh_today = now.format("%Y-%m-%d").to_string();
    let fresh_minute = (now.hour() * 60 + now.minute()) as i32;
    let now_instant = Instant::now();
    let retry_snapshot = ACCOUNT_RETRY_STATES
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();
    is_account_due_at(
        platform_enabled,
        &latest_cfg,
        account_id,
        &fresh_today,
        fresh_minute,
        now_instant,
        &retry_snapshot,
        &pending_schedules,
    )
}

fn can_any_account_retry_in_window(
    transient_failed_ids: &[String],
    retry_states: &HashMap<String, AccountRetryState>,
    config: &WorkbuddyAiAutoCheckinConfig,
    platform_enabled: bool,
    pending_schedules: &HashMap<String, (String, i32)>,
    now_local: DateTime<Local>,
) -> bool {
    if !platform_enabled || !config.enabled || transient_failed_ids.is_empty() {
        return false;
    }
    let today = now_local.format("%Y-%m-%d").to_string();

    transient_failed_ids.iter().any(|account_id| {
        let Some(state) = retry_states.get(account_id) else {
            return false;
        };
        if state.permanent_failed || state.date != today {
            return false;
        }
        let retry_date = state.next_retry_wall_time.format("%Y-%m-%d").to_string();
        if retry_date != today {
            return false;
        }
        let retry_min =
            (state.next_retry_wall_time.hour() * 60 + state.next_retry_wall_time.minute()) as i32;
        let eval_minute = config
            .account_schedules
            .as_ref()
            .and_then(|map| map.get(account_id))
            .filter(|sch| sch.scheduled_date == today)
            .map(|sch| retry_min.max(sch.scheduled_minute))
            .unwrap_or(retry_min);
        is_account_due_at(
            platform_enabled,
            config,
            account_id,
            &today,
            eval_minute,
            state.next_retry_at,
            retry_states,
            pending_schedules,
        )
    })
}

pub async fn run_workbuddy_ai_auto_checkin_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if let Ok(true) = tauri::async_runtime::spawn_blocking(flush_pending_local_writes).await {
        let _ = app.emit("workbuddy-ai-auto-checkin-logs-changed", ());
        let _ = app.emit("workbuddy-ai-auto-checkin-config-changed", ());
    }

    if !force && !crate::modules::tray_layout::is_platform_enabled("workbuddy_ai") {
        return Ok("platform_disabled".to_string());
    }
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let initial_config = tauri::async_runtime::spawn_blocking(get_config_checked)
        .await
        .map_err(|error| format!("自动签到状态任务失败: {error}"))??;
    if !initial_config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = tauri::async_runtime::spawn_blocking(workbuddy_ai_account::list)
        .await
        .map_err(|error| format!("加载 WorkBuddy AI 账号失败: {error}"))??;
    if accounts.is_empty() {
        if force {
            let empty_record = WorkbuddyAiAutoCheckinLogRecord {
                id: format!(
                    "log_{}_{}",
                    Local::now().timestamp_millis(),
                    rand::random::<u16>()
                ),
                timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                date: get_today_date_string(),
                duration_ms: 0,
                total_accounts: 0,
                success_count: 0,
                already_checked_count: 0,
                inactive_count: 0,
                pending_count: 0,
                failed_count: 0,
                status: "no_accounts".to_string(),
                details: Vec::new(),
            };
            if let Err(err) = persist_log_record(empty_record.clone()).await {
                queue_pending_log_record(empty_record);
                logger::log_warn(&format!(
                    "[WorkbuddyAiAutoCheckin] 记录无账号历史失败（已入队等待补写）: {err}"
                ));
            }
            let _ = app.emit("workbuddy-ai-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    // Capture pending/in-flight schedule snapshot BEFORE reloading SQLite under STORAGE_LOCK so
    // concurrent flushes or manual marks cannot race with schedule rollover or slip through reads.
    let pending_schedules = pending_schedule_snapshot();
    let accounts_for_schedules = accounts.clone();
    let (config, schedule_changed) = tauri::async_runtime::spawn_blocking(move || {
        ensure_and_save_account_schedules_in(&state_store(), &accounts_for_schedules, force)
    })
    .await
    .map_err(|error| format!("自动签到状态任务失败: {error}"))??;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }
    if schedule_changed {
        let _ = app.emit("workbuddy-ai-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;
    let now_instant = Instant::now();
    let retry_snapshot = ACCOUNT_RETRY_STATES
        .lock()
        .map(|m| m.clone())
        .unwrap_or_default();

    let target_accounts = select_due_accounts(
        &config,
        &accounts,
        &today_str,
        current_minute,
        now_instant,
        &retry_snapshot,
        &pending_schedules,
        force,
    );

    if target_accounts.is_empty() {
        return Ok("waiting".to_string());
    }

    logger::log_info(&format!(
        "[WorkbuddyAiAutoCheckin] 开始处理后台每日活跃打卡，目标账号数: {}",
        target_accounts.len()
    ));

    let start_instant = Instant::now();
    let start_timestamp_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut transient_failed_ids: Vec<String> = Vec::new();
    let mut checked_schedule_marks: Vec<(String, String, i32)> = Vec::new();
    let mut dated_details: Vec<(String, WorkbuddyAiAutoCheckinAccountDetail)> = Vec::new();
    let mut did_checkin_any = false;

    for account in target_accounts {
        let email_display = account_display_label(account);
        let account_checkin_time = format_time_only();
        let acc_id = account.id.clone();

        // Check platform/config/full-window/latest-schedule BOTH after acquiring the per-account
        // lock AND again right before writing the reservation + sending chat POST.
        let guarded = workbuddy_ai_checkin::checkin_account_with_precheck(&account.id, move || {
            if force {
                true
            } else {
                is_account_still_due_locked(&acc_id)
            }
        })
        .await;

        let outcome = match guarded {
            Ok(GuardedCheckinOutcome::Skipped) => {
                logger::log_info(&format!(
                    "[WorkbuddyAiAutoCheckin] 账号 {} 在拿锁或写前复检后跳过（已越窗/未到点/已关闭/已完成）",
                    account.id
                ));
                continue;
            }
            Ok(GuardedCheckinOutcome::Executed(res)) => Ok(res),
            Err(err) => Err(err),
        };

        let fresh_now = Local::now();
        let fresh_local_today = fresh_now.format("%Y-%m-%d").to_string();
        let fresh_minute = (fresh_now.hour() * 60 + fresh_now.minute()) as i32;

        if let Ok(res) = &outcome {
            if res.success && !res.already_claimed {
                did_checkin_any = true;
            }
        } else if let Err(err) = &outcome {
            logger::log_warn(&format!(
                "[WorkbuddyAiAutoCheckin] 账号 {} 每日活跃打卡异常: {}",
                account.id, err
            ));
        }

        let (detail, should_mark_schedule, activity_date) =
            detail_from_result(&account.id, email_display, account_checkin_time, &outcome);
        let log_date = resolve_effective_date(activity_date.as_deref(), &fresh_local_today);
        let schedule_mark_date = resolve_schedule_mark_date(&log_date, &fresh_local_today);

        if should_mark_schedule {
            checked_schedule_marks.push((
                account.id.clone(),
                schedule_mark_date.to_string(),
                fresh_minute,
            ));
            if log_date.as_str() < fresh_local_today.as_str() {
                // Server activity day lags local calendar day (e.g. UTC+9 local D+1 while server is D,
                // or cross-midnight completion D -> D+1). Stamp `log_date` (NOT local_today) and
                // apply a normal transient cooldown so the 30s tick waits for server rollover without spinning.
                record_account_retry_failure(&account.id, &fresh_local_today, false);
            } else {
                clear_account_retry_state(&account.id);
            }
        } else {
            let err_text = detail.message.as_deref().unwrap_or("");
            let permanent = workbuddy_ai_checkin::is_permanent_error(err_text);
            record_account_retry_failure(&account.id, &fresh_local_today, permanent);
            if !permanent {
                transient_failed_ids.push(account.id.clone());
            }
        }
        dated_details.push((log_date, detail));
    }

    if dated_details.is_empty() {
        return Ok("waiting".to_string());
    }

    if !checked_schedule_marks.is_empty() {
        let marks_to_save = checked_schedule_marks.clone();
        let save_result = tauri::async_runtime::spawn_blocking(move || {
            mark_accounts_checked_in(&state_store(), &marks_to_save)
        })
        .await
        .map_err(|error| format!("自动签到状态任务失败: {error}"))
        .and_then(|inner| inner);
        if let Err(err) = save_result {
            for (acc_id, date, minute) in checked_schedule_marks {
                queue_pending_schedule_mark(&acc_id, &date, minute);
            }
            logger::log_warn(&format!(
                "[WorkbuddyAiAutoCheckin] 保存调度状态失败（已入队等待补写，已完成的远端操作不会重放）: {err}"
            ));
        }
    }

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let all_details: Vec<WorkbuddyAiAutoCheckinAccountDetail> =
        dated_details.iter().map(|(_, d)| d.clone()).collect();
    let overall_summary = summarize_details(&all_details);

    // Group log records by each account's actual activity date so midnight-crossing batches
    // never merge two distinct days into one record.
    for (log_date, group_details) in group_details_by_date(dated_details) {
        let group_summary = summarize_details(&group_details);
        let record = WorkbuddyAiAutoCheckinLogRecord {
            id: format!(
                "log_{}_{}",
                Local::now().timestamp_millis(),
                rand::random::<u16>()
            ),
            timestamp: start_timestamp_str.clone(),
            date: log_date,
            duration_ms,
            total_accounts: group_summary.total_accounts,
            success_count: group_summary.success_count,
            already_checked_count: group_summary.already_checked_count,
            inactive_count: group_summary.inactive_count,
            pending_count: group_summary.pending_count,
            failed_count: group_summary.failed_count,
            status: group_summary.status.to_string(),
            details: group_details,
        };
        if let Err(err) = persist_log_record(record.clone()).await {
            queue_pending_log_record(record);
            logger::log_warn(&format!(
                "[WorkbuddyAiAutoCheckin] 保存签到历史失败（已入队等待补写，不影响已完成远端打卡）: {err}"
            ));
        }
    }

    if did_checkin_any {
        let _ = crate::modules::tray::update_tray_menu(app);
    }

    let _ = app.emit("workbuddy-ai-auto-checkin-logs-changed", ());
    let _ = app.emit("workbuddy-ai-auto-checkin-config-changed", ());

    let retry_needed = !transient_failed_ids.is_empty();
    let can_background_retry = if retry_needed {
        let platform_on = crate::modules::tray_layout::is_platform_enabled("workbuddy_ai");
        // Capture pending/in-flight schedule snapshot BEFORE reading SQLite to close TOCTOU.
        let latest_pending_schedules = pending_schedule_snapshot();
        let latest_cfg = tauri::async_runtime::spawn_blocking(get_config_checked)
            .await
            .ok()
            .and_then(Result::ok);
        let latest_retries = ACCOUNT_RETRY_STATES
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        latest_cfg.is_some_and(|cfg| {
            can_any_account_retry_in_window(
                &transient_failed_ids,
                &latest_retries,
                &cfg,
                platform_on,
                &latest_pending_schedules,
                Local::now(),
            )
        })
    } else {
        false
    };

    Ok(classify_cycle_outcome(&overall_summary, retry_needed, can_background_retry).to_string())
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

pub fn start_auto_checkin_scheduler(app: AppHandle) {
    if SCHEDULER_STARTED.swap(true, Ordering::SeqCst) {
        logger::log_warn("[WorkbuddyAiAutoCheckin] 后台自动活跃打卡调度服务已运行，忽略重复启动");
        return;
    }
    let wake = scheduler_wake();
    tauri::async_runtime::spawn(async move {
        logger::log_info("[WorkbuddyAiAutoCheckin] 后台自动活跃打卡调度服务已启动");
        let mut next_delay = Duration::ZERO;
        let mut scheduler_err_delay = INITIAL_RETRY_DELAY;
        loop {
            if !next_delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(next_delay) => {}
                    _ = wake.notified() => {
                        next_delay = Duration::ZERO;
                        scheduler_err_delay = INITIAL_RETRY_DELAY;
                        continue;
                    }
                }
            }
            match run_workbuddy_ai_auto_checkin_cycle_if_needed(&app, false).await {
                Ok(_) => {
                    next_delay = SCHEDULER_POLL_DELAY;
                    scheduler_err_delay = INITIAL_RETRY_DELAY;
                }
                Err(err) => {
                    next_delay = scheduler_err_delay;
                    scheduler_err_delay = next_retry_delay(scheduler_err_delay);
                    logger::log_warn(&format!(
                        "[WorkbuddyAiAutoCheckin] 调度异常: {}，{} 秒后重试",
                        err,
                        next_delay.as_secs()
                    ));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_account(id: &str, email: &str) -> WorkbuddyAccount {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "email": email,
            "access_token": "tok",
            "created_at": 0,
            "last_used": 0,
        }))
        .unwrap()
    }

    #[test]
    fn ensure_schedules_assigns_within_window() {
        let mut config = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "11:30".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        };
        let accounts = vec![
            make_test_account("acc1", "a@example.com"),
            make_test_account("acc2", "b@example.com"),
        ];
        assert!(ensure_account_schedules(&mut config, &accounts));
        let schedules = config.account_schedules.as_ref().unwrap();
        assert_eq!(schedules.len(), 2);
        for acc in &accounts {
            let s = schedules.get(&acc.id).unwrap();
            assert!((480..=690).contains(&s.scheduled_minute));
        }
        assert!(!ensure_account_schedules(&mut config, &accounts));
    }

    #[test]
    fn select_due_accounts_enforces_window_cutoff_and_per_account_backoff() {
        let today = "2026-11-24";
        let accounts = vec![
            make_test_account("acc1", "a@example.com"),
            make_test_account("acc2", "b@example.com"),
            make_test_account("acc3", "c@example.com"),
        ];
        let mut schedules = HashMap::new();
        schedules.insert(
            "acc1".into(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.into(),
                scheduled_minute: 400,
                last_checked_date: None,
            },
        );
        schedules.insert(
            "acc2".into(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.into(),
                scheduled_minute: 410,
                last_checked_date: None,
            },
        );
        schedules.insert(
            "acc3".into(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.into(),
                scheduled_minute: 420,
                last_checked_date: None,
            },
        );
        let config = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".into(),
            end_time: "12:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules),
            device_ids: None,
        };

        let now = Instant::now();
        let now_local = Local::now();
        let mut retry_states = HashMap::new();
        retry_states.insert(
            "acc1".into(),
            AccountRetryState {
                date: today.into(),
                next_retry_at: now + Duration::from_secs(300),
                next_retry_wall_time: now_local + ChronoDuration::seconds(300),
                retry_delay: Duration::from_secs(300),
                permanent_failed: false,
            },
        );
        retry_states.insert(
            "acc3".into(),
            AccountRetryState {
                date: today.into(),
                next_retry_at: now,
                next_retry_wall_time: now_local,
                retry_delay: Duration::from_secs(300),
                permanent_failed: true,
            },
        );
        let empty_pending = HashMap::new();

        let due = select_due_accounts(
            &config,
            &accounts,
            today,
            500,
            now,
            &retry_states,
            &empty_pending,
            false,
        );
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "acc2");

        let past_window = select_due_accounts(
            &config,
            &accounts,
            today,
            721,
            now,
            &retry_states,
            &empty_pending,
            false,
        );
        assert!(past_window.is_empty());

        let forced = select_due_accounts(
            &config,
            &accounts,
            today,
            721,
            now,
            &retry_states,
            &empty_pending,
            true,
        );
        assert_eq!(forced.len(), 3);
    }

    #[test]
    fn is_account_due_at_enforces_start_end_updated_schedule_and_pending_backfill() {
        let today = "2026-11-24";
        let mut schedules = HashMap::new();
        schedules.insert(
            "acc1".into(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.into(),
                scheduled_minute: 540, // 09:00
                last_checked_date: None,
            },
        );
        let config = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".into(), // 480
            end_time: "12:00".into(),   // 720
            last_checked_date: None,
            account_schedules: Some(schedules),
            device_ids: None,
        };
        let now = Instant::now();
        let retries = HashMap::new();
        let empty_pending = HashMap::new();

        // Before window start_time -> false.
        assert!(!is_account_due_at(
            true,
            &config,
            "acc1",
            today,
            470,
            now,
            &retries,
            &empty_pending
        ));
        // Inside window [start, end] but before updated scheduled_minute (540) -> false.
        assert!(!is_account_due_at(
            true,
            &config,
            "acc1",
            today,
            510,
            now,
            &retries,
            &empty_pending
        ));
        // At scheduled_minute -> true.
        assert!(is_account_due_at(
            true,
            &config,
            "acc1",
            today,
            540,
            now,
            &retries,
            &empty_pending
        ));
        // Past window end_time -> false.
        assert!(!is_account_due_at(
            true,
            &config,
            "acc1",
            today,
            721,
            now,
            &retries,
            &empty_pending
        ));
        // Platform disabled -> false.
        assert!(!is_account_due_at(
            false,
            &config,
            "acc1",
            today,
            540,
            now,
            &retries,
            &empty_pending
        ));
        // Unflushed local schedule backfill for today -> false (prevents duplicate replay).
        let mut pending_schedules = HashMap::new();
        pending_schedules.insert("acc1".into(), (today.into(), 540));
        assert!(!is_account_due_at(
            true,
            &config,
            "acc1",
            today,
            540,
            now,
            &retries,
            &pending_schedules
        ));
    }

    #[test]
    fn can_any_account_retry_in_window_checks_actual_next_retry_time_and_scheduler_eligibility() {
        use chrono::TimeZone;
        let now_local = Local
            .with_ymd_and_hms(2026, 11, 24, 23, 52, 0)
            .single()
            .unwrap();
        let today = now_local.format("%Y-%m-%d").to_string();
        let make_config = |last_checked: Option<String>, scheduled_minute: i32| {
            let mut schedules = HashMap::new();
            schedules.insert(
                "acc1".into(),
                WorkbuddyAiAccountScheduleState {
                    scheduled_date: today.clone(),
                    scheduled_minute,
                    last_checked_date: last_checked,
                },
            );
            WorkbuddyAiAutoCheckinConfig {
                enabled: true,
                start_time: "06:00".into(),
                end_time: "23:55".into(),
                last_checked_date: None,
                account_schedules: Some(schedules),
                device_ids: None,
            }
        };
        let config_due = make_config(None, 540);
        let empty_pending = HashMap::new();
        let now = Instant::now();

        // 5-minute backoff from 23:52 lands at 23:57 > 23:55 -> must NOT report retry_scheduled.
        let mut retries_late = HashMap::new();
        retries_late.insert(
            "acc1".into(),
            AccountRetryState {
                date: today.clone(),
                next_retry_at: now + Duration::from_secs(300),
                next_retry_wall_time: now_local + ChronoDuration::minutes(5),
                retry_delay: Duration::from_secs(300),
                permanent_failed: false,
            },
        );
        assert!(!can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_late,
            &config_due,
            true,
            &empty_pending,
            now_local
        ));

        // 2-minute backoff from 23:52 lands at 23:54 <= 23:55 -> eligible for retry_scheduled.
        let mut retries_in_window = HashMap::new();
        retries_in_window.insert(
            "acc1".into(),
            AccountRetryState {
                date: today.clone(),
                next_retry_at: now + Duration::from_secs(120),
                next_retry_wall_time: now_local + ChronoDuration::minutes(2),
                retry_delay: Duration::from_secs(120),
                permanent_failed: false,
            },
        );
        assert!(can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_in_window,
            &config_due,
            true,
            &empty_pending,
            now_local
        ));

        // Force-test on an account ALREADY checked today must NOT report retry_scheduled.
        let config_checked_today = make_config(Some(today.clone()), 540);
        assert!(!can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_in_window,
            &config_checked_today,
            true,
            &empty_pending,
            now_local
        ));

        // Pending backfill schedule for today must ALSO suppress retry_scheduled.
        let mut pending_today = HashMap::new();
        pending_today.insert("acc1".into(), (today.clone(), 540));
        assert!(!can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_in_window,
            &config_due,
            true,
            &pending_today,
            now_local
        ));

        // Scheduled minute after retry_min but still within window (23:55 <= 23:55) -> WILL retry at 23:55.
        let config_later_schedule_in_window = make_config(None, 23 * 60 + 55);
        assert!(can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_in_window,
            &config_later_schedule_in_window,
            true,
            &empty_pending,
            now_local
        ));

        // Scheduled minute past end_time (23:56 > 23:55) -> cannot retry in window.
        let config_later_schedule_past_window = make_config(None, 23 * 60 + 56);
        assert!(!can_any_account_retry_in_window(
            &["acc1".into()],
            &retries_in_window,
            &config_later_schedule_past_window,
            true,
            &empty_pending,
            now_local
        ));
    }

    #[test]
    fn pending_local_writes_flush_recovers_failed_schedule_and_reconciles_pending_history() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_store = state_store_at(dir.path().join("auto_cfg.json"));
        let hist_store = history_store_at(dir.path().join("auto_logs.json"));
        let today = get_today_date_string();
        let now_ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // Seed an initial pending history record.
        let pending_rec = WorkbuddyAiAutoCheckinLogRecord {
            id: "seed_1".into(),
            timestamp: now_ts.clone(),
            date: today.clone(),
            duration_ms: 100,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 1,
            failed_count: 0,
            status: "pending".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "pending".into(),
                time: Some("09:00:00".into()),
                result_code: Some("pending_confirmation".into()),
                model_id: Some("hy4flash-free".into()),
                message: None,
                credit: None,
            }],
        };
        hist_store
            .merge_day(
                pending_rec,
                Local::now().timestamp() - 86_400,
                merge_log_record,
            )
            .unwrap();

        // Simulate queued schedule & reconciled history write awaiting retryable backfill.
        let queue = Mutex::new(PendingLocalWrites::default());
        {
            let mut q = queue.lock().unwrap();
            q.schedules.insert("acc1".into(), (today.clone(), 550));
            q.logs.insert(
                today.clone(),
                WorkbuddyAiAutoCheckinLogRecord {
                    id: "reconcile_1".into(),
                    timestamp: now_ts,
                    date: today.clone(),
                    duration_ms: 0,
                    total_accounts: 1,
                    success_count: 0,
                    already_checked_count: 1,
                    inactive_count: 0,
                    pending_count: 0,
                    failed_count: 0,
                    status: "success".into(),
                    details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                        account_id: "acc1".into(),
                        email: "a@example.com".into(),
                        status: "already_checked".into(),
                        time: Some("09:05:00".into()),
                        result_code: Some("reconciled_active".into()),
                        model_id: None,
                        message: None,
                        credit: None,
                    }],
                },
            );
        }

        let flush_lock = Mutex::new(());
        assert!(flush_pending_local_writes_in(
            &flush_lock,
            &queue,
            &cfg_store,
            &hist_store
        ));
        assert!(queue.lock().unwrap().schedules.is_empty());
        assert!(queue.lock().unwrap().in_flight_schedules.is_empty());
        assert!(queue.lock().unwrap().logs.is_empty());

        let loaded_cfg = cfg_store.load().unwrap();
        let sch = loaded_cfg
            .account_schedules
            .as_ref()
            .and_then(|m| m.get("acc1"))
            .unwrap();
        assert_eq!(sch.last_checked_date.as_deref(), Some(today.as_str()));

        let read = wakeup_history_store::read_available::<WorkbuddyAiAutoCheckinLogRecord>(
            hist_store, false,
        )
        .unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.records[0].status, "success");
        assert_eq!(read.records[0].pending_count, 0);
        assert_eq!(read.records[0].already_checked_count, 1);
        assert_eq!(read.records[0].details[0].status, "already_checked");
        assert_eq!(
            read.records[0].details[0].model_id.as_deref(),
            Some("hy4flash-free")
        );
    }

    #[test]
    fn same_second_interleaved_backfill_preserves_all_accounts_without_loss() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_store = state_store_at(dir.path().join("auto_cfg_interleave.json"));
        let hist_store = history_store_at(dir.path().join("auto_logs_interleave.json"));
        let today = get_today_date_string();
        let same_second_ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let cutoff = Local::now().timestamp() - 86_400;

        let make_log = |id: &str, acc_id: &str, email: &str| WorkbuddyAiAutoCheckinLogRecord {
            id: id.into(),
            timestamp: same_second_ts.clone(),
            date: today.clone(),
            duration_ms: 10,
            total_accounts: 1,
            success_count: 1,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 0,
            status: "success".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: acc_id.into(),
                email: email.into(),
                status: "success".into(),
                time: Some("14:00:00".into()),
                result_code: Some("activated".into()),
                model_id: Some("hy4flash-free".into()),
                message: None,
                credit: None,
            }],
        };

        let flush_lock = Mutex::new(());
        let queue = Mutex::new(PendingLocalWrites::default());
        queue
            .lock()
            .unwrap()
            .schedules
            .insert("acc_a".into(), (today.clone(), 500));
        queue_pending_log_record_in(&queue, make_log("log_a", "acc_a", "a@example.com"));

        // Case 1: While A is being flushed, B enqueues in the EXACT SAME SECOND.
        // Also verify in_flight_schedules stays visible during schedule flush.
        let mut saw_in_flight = false;
        let flushed1 = flush_pending_local_writes_with_ops(
            &flush_lock,
            &queue,
            |schedules_batch| {
                saw_in_flight = pending_schedule_snapshot_from(&queue).contains_key("acc_a");
                let config = cfg_store.load()?;
                let previous = config.account_schedules.clone();
                let mut schedules = previous.clone().unwrap_or_default();
                for (account_id, (d, m)) in schedules_batch {
                    mark_schedule_checked(&mut schedules, account_id, d, *m);
                }
                cfg_store.save_schedules(previous.as_ref(), Some(schedules))
            },
            |record| {
                // Interleave Account B into queue at the exact same second while A is flushing.
                queue_pending_log_record_in(&queue, make_log("log_b", "acc_b", "b@example.com"));
                hist_store.merge_day(record, cutoff, merge_log_record)
            },
        );
        assert!(flushed1);
        assert!(saw_in_flight);
        // B must STILL be present in queue (NOT deleted by A's completion!).
        assert!(queue.lock().unwrap().logs.contains_key(&today));

        // Second flush persists B; disk now contains BOTH A and B, and queue is empty.
        assert!(flush_pending_local_writes_in(
            &flush_lock,
            &queue,
            &cfg_store,
            &hist_store
        ));
        assert!(queue.lock().unwrap().logs.is_empty());

        let read = wakeup_history_store::read_available::<WorkbuddyAiAutoCheckinLogRecord>(
            history_store_at(dir.path().join("auto_logs_interleave.json")),
            false,
        )
        .unwrap();
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.records[0].total_accounts, 2);
        let mut ids: Vec<&str> = read.records[0]
            .details
            .iter()
            .map(|d| d.account_id.as_str())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["acc_a", "acc_b"]);

        // Case 2: Flush fails for C while D enqueues in the exact same second -> merge-back preserves both.
        queue_pending_log_record_in(&queue, make_log("log_c", "acc_c", "c@example.com"));
        let flushed_fail = flush_pending_local_writes_with_ops(
            &flush_lock,
            &queue,
            |_| Ok(()),
            |_record| {
                queue_pending_log_record_in(&queue, make_log("log_d", "acc_d", "d@example.com"));
                Err("transient disk busy".into())
            },
        );
        assert!(!flushed_fail);
        assert!(flush_pending_local_writes_in(
            &flush_lock,
            &queue,
            &cfg_store,
            &hist_store
        ));
        assert!(queue.lock().unwrap().logs.is_empty());

        let read2 = wakeup_history_store::read_available::<WorkbuddyAiAutoCheckinLogRecord>(
            hist_store, false,
        )
        .unwrap();
        assert_eq!(read2.records[0].total_accounts, 4);
        let mut ids2: Vec<&str> = read2.records[0]
            .details
            .iter()
            .map(|d| d.account_id.as_str())
            .collect();
        ids2.sort();
        assert_eq!(ids2, vec!["acc_a", "acc_b", "acc_c", "acc_d"]);
    }

    #[test]
    fn summarize_details_handles_all_enterprise_inactive_and_no_accounts() {
        let empty = summarize_details(&[]);
        assert_eq!(empty.status, "no_accounts");

        let enterprise_only = vec![
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "ent1".into(),
                email: "ent1@corp.com".into(),
                status: "inactive".into(),
                time: Some("09:00:00".into()),
                result_code: Some("inactive".into()),
                model_id: None,
                message: None,
                credit: None,
            },
            WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "ent2".into(),
                email: "ent2@corp.com".into(),
                status: "inactive".into(),
                time: Some("09:00:01".into()),
                result_code: Some("inactive".into()),
                model_id: None,
                message: None,
                credit: None,
            },
        ];
        let summary = summarize_details(&enterprise_only);
        assert_eq!(summary.status, "inactive");
        assert_eq!(summary.inactive_count, 2);
        assert_eq!(summary.failed_count, 0);
    }

    #[test]
    fn classify_cycle_outcome_distinguishes_permanent_failure_from_scheduled_retry() {
        let all_failed = summarize_details(&[WorkbuddyAiAutoCheckinAccountDetail {
            account_id: "acc1".into(),
            email: "a@example.com".into(),
            status: "failed".into(),
            time: None,
            result_code: Some("token_expired".into()),
            model_id: None,
            message: Some("WORKBUDDY_AI_TOKEN_EXPIRED".into()),
            credit: None,
        }]);
        assert_eq!(classify_cycle_outcome(&all_failed, false, false), "failed");
        assert_eq!(classify_cycle_outcome(&all_failed, true, false), "failed");
        assert_eq!(
            classify_cycle_outcome(&all_failed, true, true),
            "retry_scheduled"
        );
    }

    #[test]
    fn group_details_by_date_separates_midnight_crossing_accounts() {
        let detail = |id: &str| WorkbuddyAiAutoCheckinAccountDetail {
            account_id: id.into(),
            email: format!("{id}@example.com"),
            status: "success".into(),
            time: Some("23:59:59".into()),
            result_code: Some("activated".into()),
            model_id: Some("hy4flash-free".into()),
            message: None,
            credit: None,
        };
        let grouped = group_details_by_date(vec![
            ("2026-11-24".into(), detail("acc1")),
            ("2026-11-25".into(), detail("acc2")),
        ]);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["2026-11-24"].len(), 1);
        assert_eq!(grouped["2026-11-25"].len(), 1);
    }

    #[test]
    fn merge_log_records_never_downgrades_confirmed_same_day_status_to_failed() {
        let mut existing = WorkbuddyAiAutoCheckinLogRecord {
            id: "1".into(),
            timestamp: "2026-11-24 09:00:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 100,
            total_accounts: 1,
            success_count: 1,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 0,
            status: "success".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "success".into(),
                time: Some("09:00:00".into()),
                result_code: Some("activated".into()),
                model_id: Some("hy4flash-free".into()),
                message: None,
                credit: None,
            }],
        };
        let later_failure = WorkbuddyAiAutoCheckinLogRecord {
            id: "2".into(),
            timestamp: "2026-11-24 10:00:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 50,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 1,
            status: "failed".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "failed".into(),
                time: Some("10:00:00".into()),
                result_code: Some("error".into()),
                model_id: None,
                message: Some("temporary network error".into()),
                credit: None,
            }],
        };
        merge_log_record(&mut existing, later_failure);
        assert_eq!(existing.status, "success");
        assert_eq!(existing.failed_count, 0);
        assert_eq!(existing.details[0].status, "success");
        assert_eq!(
            existing.details[0].model_id.as_deref(),
            Some("hy4flash-free")
        );
        assert_eq!(existing.details[0].message, None);
    }

    #[test]
    fn day2_schedule_marks_persist_through_sqlite_direct_and_backfill_paths() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_store = state_store_at(dir.path().join("day2_cfg.json"));
        let hist_store = history_store_at(dir.path().join("day2_logs.json"));
        let day1 = "2026-11-24";
        let day2 = "2026-11-25";

        // Seed Day 1 already-checked rows in real SQLite so Day 2 hits ON CONFLICT DO UPDATE.
        let mut seeded = HashMap::new();
        seeded.insert(
            "acc_direct".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: day1.to_string(),
                scheduled_minute: 540,
                last_checked_date: Some(day1.to_string()),
            },
        );
        seeded.insert(
            "acc_backfill".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: day1.to_string(),
                scheduled_minute: 550,
                last_checked_date: Some(day1.to_string()),
            },
        );
        cfg_store.save_schedules(None, Some(seeded)).unwrap();
        let initial = cfg_store.load().unwrap();
        assert_eq!(
            initial
                .account_schedules
                .as_ref()
                .unwrap()
                .get("acc_direct")
                .unwrap()
                .last_checked_date
                .as_deref(),
            Some(day1)
        );

        // 1. Direct path (same locked implementation as mark_single_account_checked_sync):
        //    must advance scheduled_date and persist last_checked_date = Some(day2).
        mark_single_account_checked_in(&cfg_store, "acc_direct", day2, 600).unwrap();
        let after_direct = cfg_store.load().unwrap();
        let direct_sch = after_direct
            .account_schedules
            .as_ref()
            .unwrap()
            .get("acc_direct")
            .unwrap();
        assert_eq!(direct_sch.scheduled_date, day2);
        assert_eq!(direct_sch.last_checked_date.as_deref(), Some(day2));

        // Reconciling an older day (day1) must never regress scheduled_date or last_checked_date.
        mark_single_account_checked_in(&cfg_store, "acc_direct", day1, 610).unwrap();
        let after_older = cfg_store.load().unwrap();
        let direct_sch_after_older = after_older
            .account_schedules
            .as_ref()
            .unwrap()
            .get("acc_direct")
            .unwrap();
        assert_eq!(direct_sch_after_older.scheduled_date, day2);
        assert_eq!(
            direct_sch_after_older.last_checked_date.as_deref(),
            Some(day2)
        );

        // 2. Backfill path via flush_pending_local_writes_in:
        //    also verify queue_pending_schedule_mark_in never overwrites a newer queued date.
        let flush_lock = Mutex::new(());
        let queue = Mutex::new(PendingLocalWrites::default());
        queue_pending_schedule_mark_in(&queue, "acc_backfill", day2, 605);
        queue_pending_schedule_mark_in(&queue, "acc_backfill", day1, 615);
        assert_eq!(
            queue
                .lock()
                .unwrap()
                .schedules
                .get("acc_backfill")
                .map(|(d, _)| d.as_str()),
            Some(day2)
        );

        assert!(flush_pending_local_writes_in(
            &flush_lock,
            &queue,
            &cfg_store,
            &hist_store
        ));
        assert!(queue.lock().unwrap().schedules.is_empty());
        assert!(queue.lock().unwrap().in_flight_schedules.is_empty());

        let after_backfill = cfg_store.load().unwrap();
        let backfill_sch = after_backfill
            .account_schedules
            .as_ref()
            .unwrap()
            .get("acc_backfill")
            .unwrap();
        assert_eq!(backfill_sch.scheduled_date, day2);
        assert_eq!(backfill_sch.last_checked_date.as_deref(), Some(day2));

        // 3. Atomic locked ensure_and_save_account_schedules_in preserves last_checked_date >= today
        //    (same-day marks and defensive clock-skew >= today) while resetting yesterday to None.
        let today = get_today_date_string();
        let tomorrow = (Local::now() + ChronoDuration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let yesterday = (Local::now() - ChronoDuration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        cfg_store
            .save_rules(&WorkbuddyAiAutoCheckinConfig {
                enabled: true,
                start_time: "08:00".into(),
                end_time: "12:00".into(),
                ..Default::default()
            })
            .unwrap();
        let mut rollover_seed = HashMap::new();
        rollover_seed.insert(
            "acc_today".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: yesterday.clone(),
                scheduled_minute: 540,
                last_checked_date: Some(today.clone()),
            },
        );
        rollover_seed.insert(
            "acc_ahead".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: yesterday.clone(),
                scheduled_minute: 540,
                last_checked_date: Some(tomorrow.clone()),
            },
        );
        rollover_seed.insert(
            "acc_yesterday".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: yesterday.clone(),
                scheduled_minute: 540,
                last_checked_date: Some(yesterday),
            },
        );
        cfg_store.save_schedules(None, Some(rollover_seed)).unwrap();

        let accounts = vec![
            make_test_account("acc_today", "today@example.com"),
            make_test_account("acc_ahead", "ahead@example.com"),
            make_test_account("acc_yesterday", "yesterday@example.com"),
        ];
        let (ensured, changed) =
            ensure_and_save_account_schedules_in(&cfg_store, &accounts, false).unwrap();
        assert!(changed);
        let ensured_map = ensured.account_schedules.as_ref().unwrap();
        assert_eq!(ensured_map["acc_today"].scheduled_date, today);
        assert_eq!(
            ensured_map["acc_today"].last_checked_date.as_deref(),
            Some(today.as_str())
        );
        assert_eq!(ensured_map["acc_ahead"].scheduled_date, today);
        assert_eq!(
            ensured_map["acc_ahead"].last_checked_date.as_deref(),
            Some(tomorrow.as_str())
        );
        assert_eq!(ensured_map["acc_yesterday"].scheduled_date, today);
        assert_eq!(ensured_map["acc_yesterday"].last_checked_date, None);
    }

    #[test]
    fn is_account_due_at_handles_server_ahead_and_behind_dates() {
        let local_d1 = "2026-11-25";
        let local_d2 = "2026-11-26";
        let server_d2 = "2026-11-26";
        let server_d3 = "2026-11-27";
        let yesterday = "2026-11-24";
        let now = Instant::now();
        let retries = HashMap::new();
        let empty_pending = HashMap::new();

        // 1. Western timezone consecutive days (e.g., UTC-8 evening window 18:00-22:00):
        //    Local D (2026-11-25) checks in server D+1 (2026-11-26):
        //    - history log_date preserves server "2026-11-26"
        //    - schedule_mark_date clamps to local "2026-11-25" so D is done and D+1 is NOT skipped!
        let log_day1 = resolve_effective_date(Some(server_d2), local_d1);
        assert_eq!(log_day1, server_d2);
        let mark_day1 = resolve_schedule_mark_date(&log_day1, local_d1);
        assert_eq!(mark_day1, local_d1);

        let mut schedules = HashMap::new();
        schedules.insert(
            "acc1".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: local_d1.to_string(),
                scheduled_minute: 19 * 60, // 19:00
                last_checked_date: None,
            },
        );
        mark_schedule_checked(&mut schedules, "acc1", mark_day1, 19 * 60);
        assert_eq!(
            schedules["acc1"].last_checked_date.as_deref(),
            Some(local_d1)
        );

        let cfg_day1 = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "18:00".into(),
            end_time: "22:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules.clone()),
            device_ids: None,
        };
        // Rest of local D -> already checked for local D (no re-trigger).
        assert!(!is_account_due_at(
            true,
            &cfg_day1,
            "acc1",
            local_d1,
            19 * 60 + 5,
            now,
            &retries,
            &empty_pending
        ));

        // Next local evening D+1 (2026-11-26, server is now D+2 = 2026-11-27):
        // schedule rolls to local D+1 and account MUST be due (no alternating-day skip!).
        schedules.insert(
            "acc1".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: local_d2.to_string(),
                scheduled_minute: 19 * 60,
                last_checked_date: Some(local_d1.to_string()),
            },
        );
        let cfg_day2 = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "18:00".into(),
            end_time: "22:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules.clone()),
            device_ids: None,
        };
        assert!(is_account_due_at(
            true,
            &cfg_day2,
            "acc1",
            local_d2,
            19 * 60,
            now,
            &retries,
            &empty_pending
        ));

        // Completing local D+1 for server D+2 stamps local D+1 and records server D+2 in history.
        let log_day2 = resolve_effective_date(Some(server_d3), local_d2);
        assert_eq!(log_day2, server_d3);
        let mark_day2 = resolve_schedule_mark_date(&log_day2, local_d2);
        assert_eq!(mark_day2, local_d2);
        mark_schedule_checked(&mut schedules, "acc1", mark_day2, 19 * 60);
        let cfg_day2_done = WorkbuddyAiAutoCheckinConfig {
            account_schedules: Some(schedules),
            ..cfg_day2
        };
        assert!(!is_account_due_at(
            true,
            &cfg_day2_done,
            "acc1",
            local_d2,
            19 * 60 + 5,
            now,
            &retries,
            &empty_pending
        ));

        // 2. Server-behind-local / cross-midnight (local D = 2026-11-25, server = 2026-11-24):
        //    schedule_mark_date keeps server date (2026-11-24 < 2026-11-25) so local D remains due.
        let log_behind = resolve_effective_date(Some(yesterday), local_d1);
        assert_eq!(log_behind, yesterday);
        assert_eq!(resolve_schedule_mark_date(&log_behind, local_d1), yesterday);
        assert_eq!(resolve_effective_date(Some("  "), local_d1), local_d1);
        assert_eq!(resolve_effective_date(None, local_d1), local_d1);

        let mut schedules_behind = HashMap::new();
        schedules_behind.insert(
            "acc1".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: local_d1.to_string(),
                scheduled_minute: 540,
                last_checked_date: Some(yesterday.to_string()),
            },
        );
        let cfg_behind = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".into(),
            end_time: "12:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules_behind),
            device_ids: None,
        };
        assert!(is_account_due_at(
            true,
            &cfg_behind,
            "acc1",
            local_d1,
            540,
            now,
            &retries,
            &empty_pending
        ));

        // 3. Defensive >= today_str guard (e.g., system clock shifted backward):
        //    pending schedule mark >= today suppresses re-trigger.
        let mut pending_ahead = HashMap::new();
        pending_ahead.insert("acc1".to_string(), (local_d2.to_string(), 540));
        assert!(!is_account_due_at(
            true,
            &cfg_behind,
            "acc1",
            local_d1,
            540,
            now,
            &retries,
            &pending_ahead
        ));
    }

    #[test]
    fn merge_log_record_preserves_precedence_and_monotonic_fields_on_out_of_order_backfill() {
        // Case 1: Older "failed" backfill onto newer "pending" keeps "pending", message, time, and timestamp.
        let mut newer_pending = WorkbuddyAiAutoCheckinLogRecord {
            id: "pending_1".into(),
            timestamp: "2026-11-24 09:05:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 120,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 1,
            failed_count: 0,
            status: "pending".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "pending".into(),
                time: Some("09:05:00".into()),
                result_code: Some("pending_confirmation".into()),
                model_id: Some("hy4flash-free".into()),
                message: Some("WORKBUDDY_AI_PENDING_CONFIRMATION".into()),
                credit: None,
            }],
        };
        let older_failed = WorkbuddyAiAutoCheckinLogRecord {
            id: "failed_1".into(),
            timestamp: "2026-11-24 09:00:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 80,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 1,
            status: "failed".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "failed".into(),
                time: Some("09:00:00".into()),
                result_code: Some("error".into()),
                model_id: None,
                message: Some("temporary network error".into()),
                credit: None,
            }],
        };
        merge_log_record(&mut newer_pending, older_failed);
        assert_eq!(newer_pending.status, "pending");
        assert_eq!(newer_pending.pending_count, 1);
        assert_eq!(newer_pending.failed_count, 0);
        assert_eq!(newer_pending.timestamp, "2026-11-24 09:05:00");
        assert_eq!(newer_pending.details[0].status, "pending");
        assert_eq!(newer_pending.details[0].time.as_deref(), Some("09:05:00"));
        assert_eq!(
            newer_pending.details[0].message.as_deref(),
            Some("WORKBUDDY_AI_PENDING_CONFIRMATION")
        );

        // Case 2: Older "pending" backfill onto newer reconciled "already_checked" keeps
        // "already_checked", clean message: None, newer time & timestamp, and backfills model_id.
        let mut newer_confirmed = WorkbuddyAiAutoCheckinLogRecord {
            id: "reconcile_1".into(),
            timestamp: "2026-11-24 09:10:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 0,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 1,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 0,
            status: "success".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "already_checked".into(),
                time: Some("09:10:00".into()),
                result_code: Some("reconciled_active".into()),
                model_id: None,
                message: None,
                credit: None,
            }],
        };
        let older_pending = WorkbuddyAiAutoCheckinLogRecord {
            id: "pending_old".into(),
            timestamp: "2026-11-24 09:02:00".into(),
            date: "2026-11-24".into(),
            duration_ms: 90,
            total_accounts: 1,
            success_count: 0,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 1,
            failed_count: 0,
            status: "pending".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: "acc1".into(),
                email: "a@example.com".into(),
                status: "pending".into(),
                time: Some("09:02:00".into()),
                result_code: Some("pending_confirmation".into()),
                model_id: Some("hy4flash-free".into()),
                message: Some("WORKBUDDY_AI_PENDING_CONFIRMATION".into()),
                credit: None,
            }],
        };
        merge_log_record(&mut newer_confirmed, older_pending);
        assert_eq!(newer_confirmed.status, "success");
        assert_eq!(newer_confirmed.already_checked_count, 1);
        assert_eq!(newer_confirmed.pending_count, 0);
        assert_eq!(newer_confirmed.timestamp, "2026-11-24 09:10:00");
        assert_eq!(newer_confirmed.details[0].status, "already_checked");
        assert_eq!(newer_confirmed.details[0].time.as_deref(), Some("09:10:00"));
        assert_eq!(newer_confirmed.details[0].message, None);
        assert_eq!(
            newer_confirmed.details[0].model_id.as_deref(),
            Some("hy4flash-free")
        );
    }

    #[test]
    fn can_any_account_retry_in_window_returns_true_when_retry_precedes_scheduled_minute_in_window()
    {
        use chrono::TimeZone;
        let now_local = Local
            .with_ymd_and_hms(2026, 11, 24, 8, 0, 0)
            .single()
            .unwrap();
        let today = now_local.format("%Y-%m-%d").to_string();
        let mut schedules = HashMap::new();
        schedules.insert(
            "acc1".to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.clone(),
                scheduled_minute: 10 * 60, // 10:00
                last_checked_date: None,
            },
        );
        let config = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".into(),
            end_time: "12:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules),
            device_ids: None,
        };
        let now = Instant::now();
        let mut retries = HashMap::new();
        retries.insert(
            "acc1".to_string(),
            AccountRetryState {
                date: today,
                next_retry_at: now + Duration::from_secs(300),
                next_retry_wall_time: now_local + ChronoDuration::minutes(5), // 08:05
                retry_delay: Duration::from_secs(300),
                permanent_failed: false,
            },
        );
        let empty_pending = HashMap::new();

        assert!(can_any_account_retry_in_window(
            &["acc1".to_string()],
            &retries,
            &config,
            true,
            &empty_pending,
            now_local
        ));
    }

    #[test]
    fn clear_account_retry_state_clears_permanent_failed_and_restores_eligibility() {
        let account_id = "test_clear_permanent_failed_acc";
        let today = "2026-11-25";
        let mut schedules = HashMap::new();
        schedules.insert(
            account_id.to_string(),
            WorkbuddyAiAccountScheduleState {
                scheduled_date: today.to_string(),
                scheduled_minute: 540,
                last_checked_date: None,
            },
        );
        let config = WorkbuddyAiAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".into(),
            end_time: "12:00".into(),
            last_checked_date: None,
            account_schedules: Some(schedules),
            device_ids: None,
        };
        let empty_pending = HashMap::new();

        clear_account_retry_state(account_id);
        record_account_retry_failure(account_id, today, true);
        let locked_snapshot = ACCOUNT_RETRY_STATES.lock().unwrap().clone();
        assert!(locked_snapshot
            .get(account_id)
            .is_some_and(|s| s.permanent_failed));
        assert!(!is_account_due_at(
            true,
            &config,
            account_id,
            today,
            600,
            Instant::now(),
            &locked_snapshot,
            &empty_pending
        ));

        clear_account_retry_state(account_id);
        let cleared_snapshot = ACCOUNT_RETRY_STATES.lock().unwrap().clone();
        assert!(!cleared_snapshot.contains_key(account_id));
        assert!(is_account_due_at(
            true,
            &config,
            account_id,
            today,
            600,
            Instant::now(),
            &cleared_snapshot,
            &empty_pending
        ));
    }

    #[test]
    fn clear_logs_drops_pending_queued_logs_so_flush_cannot_resurrect_history() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_store = state_store_at(dir.path().join("clear_logs_cfg.json"));
        let hist_store = history_store_at(dir.path().join("clear_logs_hist.json"));
        let today = get_today_date_string();
        let now_ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let make_rec = |id: &str, acc: &str| WorkbuddyAiAutoCheckinLogRecord {
            id: id.into(),
            timestamp: now_ts.clone(),
            date: today.clone(),
            duration_ms: 10,
            total_accounts: 1,
            success_count: 1,
            already_checked_count: 0,
            inactive_count: 0,
            pending_count: 0,
            failed_count: 0,
            status: "success".into(),
            details: vec![WorkbuddyAiAutoCheckinAccountDetail {
                account_id: acc.into(),
                email: format!("{acc}@example.com"),
                status: "success".into(),
                time: Some("09:00:00".into()),
                result_code: Some("activated".into()),
                model_id: Some("hy4flash-free".into()),
                message: None,
                credit: None,
            }],
        };

        // Seed both SQLite history and queued pending logs.
        hist_store
            .merge_day(
                make_rec("disk_1", "acc_disk"),
                Local::now().timestamp() - 86_400,
                merge_log_record,
            )
            .unwrap();
        let flush_lock = Mutex::new(());
        let queue = Mutex::new(PendingLocalWrites::default());
        queue_pending_log_record_in(&queue, make_rec("queued_1", "acc_queued"));
        assert!(!queue.lock().unwrap().logs.is_empty());

        clear_logs_in(&flush_lock, &queue, &hist_store).unwrap();
        assert!(queue.lock().unwrap().logs.is_empty());

        // Subsequent flush must NOT resurrect cleared logs.
        assert!(!flush_pending_local_writes_in(
            &flush_lock,
            &queue,
            &cfg_store,
            &hist_store
        ));
        let read = wakeup_history_store::read_available::<WorkbuddyAiAutoCheckinLogRecord>(
            hist_store, false,
        )
        .unwrap();
        assert!(read.records.is_empty());
    }
}
