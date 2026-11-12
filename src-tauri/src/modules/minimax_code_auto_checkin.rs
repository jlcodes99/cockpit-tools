use super::checkin_state_store::{validate_config, CheckinStore};
pub use super::checkin_state_store::{
    AccountSchedule as MinimaxCodeAccountScheduleState,
    CheckinConfig as MinimaxCodeAutoCheckinConfig,
};
use super::wakeup_history_store::{self, checkin_timestamp, HistoryRow, HistoryStore};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};
use std::time::{Duration, Instant};

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::models::minimax_code::MinimaxCodeAccount;
use crate::modules::{config, logger, minimax_code_account, minimax_code_api};

static IS_CHECKIN_RUNNING: AtomicBool = AtomicBool::new(false);
static STORAGE_LOCK: Mutex<()> = Mutex::new(());
static SCHEDULER_WAKE: OnceLock<Notify> = OnceLock::new();

const SCHEDULER_POLL_DELAY: Duration = Duration::from_secs(30);
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(5 * 60);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60 * 60);

struct CheckinGuard;
impl Drop for CheckinGuard {
    fn drop(&mut self) {
        IS_CHECKIN_RUNNING.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeAutoCheckinAccountDetail {
    pub account_id: String,
    pub email: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeAutoCheckinLogRecord {
    pub id: String,
    pub timestamp: String,
    pub date: String,
    pub duration_ms: u64,
    pub total_accounts: usize,
    pub success_count: usize,
    pub already_checked_count: usize,
    pub failed_count: usize,
    pub status: String,
    pub details: Vec<MinimaxCodeAutoCheckinAccountDetail>,
}

pub(crate) const CONFIG_FILE: &str = "minimax_code_auto_checkin_config.json";

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join(CONFIG_FILE)
}

pub(crate) const LOGS_FILE: &str = "minimax_code_auto_checkin_logs.json";

fn get_logs_file_path() -> PathBuf {
    config::get_shared_dir().join(LOGS_FILE)
}

fn scheduler_wake() -> &'static Notify {
    SCHEDULER_WAKE.get_or_init(Notify::new)
}

fn wake_scheduler() {
    scheduler_wake().notify_one();
}

fn state_store() -> CheckinStore {
    state_store_at(get_config_file_path())
}

pub(super) fn state_store_at(path: PathBuf) -> CheckinStore {
    CheckinStore::new(path, "minimaxcode")
}

pub fn get_config_checked() -> Result<MinimaxCodeAutoCheckinConfig, String> {
    state_store().load()
}

pub fn save_config(config: &MinimaxCodeAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    state_store().save_rules(config)?;
    wake_scheduler();
    Ok(())
}

pub fn migrate_config_if_missing(
    legacy_config: &MinimaxCodeAutoCheckinConfig,
) -> Result<MinimaxCodeAutoCheckinConfig, String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    let config = state_store().seed_webview(legacy_config)?;
    wake_scheduler();
    Ok(config)
}

async fn save_schedules(
    previous: Option<HashMap<String, MinimaxCodeAccountScheduleState>>,
    schedules: Option<HashMap<String, MinimaxCodeAccountScheduleState>>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        state_store().save_schedules(previous.as_ref(), schedules)
    })
    .await
    .map_err(|error| format!("自动签到状态任务失败: {error}"))?
}

impl HistoryRow for MinimaxCodeAutoCheckinLogRecord {
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
    wakeup_history_store::daily_checkin(path, "minimaxcode")
}

pub fn schedule_log_migration() {
    history_store().schedule::<MinimaxCodeAutoCheckinLogRecord>();
}

pub fn get_logs_checked(
    retry: bool,
) -> Result<wakeup_history_store::HistoryRead<MinimaxCodeAutoCheckinLogRecord>, String> {
    wakeup_history_store::read_available(history_store(), retry)
}

pub fn clear_logs() -> Result<(), String> {
    history_store().clear()
}

fn merge_log_record(
    existing: &mut MinimaxCodeAutoCheckinLogRecord,
    record: MinimaxCodeAutoCheckinLogRecord,
) {
    let mut details: HashMap<String, MinimaxCodeAutoCheckinAccountDetail> = existing
        .details
        .drain(..)
        .map(|detail| (detail.account_id.clone(), detail))
        .collect();
    for detail in record.details {
        details.insert(detail.account_id.clone(), detail);
    }

    existing.timestamp = record.timestamp;
    existing.duration_ms = existing.duration_ms.saturating_add(record.duration_ms);
    existing.details = details.into_values().collect();
    existing.success_count = existing
        .details
        .iter()
        .filter(|detail| detail.status == "success")
        .count();
    existing.already_checked_count = existing
        .details
        .iter()
        .filter(|detail| detail.status == "already_checked")
        .count();
    existing.failed_count = existing
        .details
        .iter()
        .filter(|detail| detail.status == "failed")
        .count();
    existing.total_accounts = existing.details.len();
    existing.status = if existing.total_accounts == 0 {
        "no_accounts"
    } else if existing.failed_count == 0 {
        "success"
    } else if existing.success_count > 0 || existing.already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    }
    .to_string();
}

fn add_log_record(record: MinimaxCodeAutoCheckinLogRecord) -> Result<(), String> {
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    history_store().merge_day(
        record,
        Local::now().timestamp() - THIRTY_DAYS_SECS,
        merge_log_record,
    )
}

async fn persist_log_record(record: MinimaxCodeAutoCheckinLogRecord) -> Result<(), String> {
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
    config: &mut MinimaxCodeAutoCheckinConfig,
    accounts: &[MinimaxCodeAccount],
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
            if e.last_checked_date.as_deref() == Some(&today_str) {
                Some(today_str.clone())
            } else {
                None
            }
        });

        schedules.insert(
            account.id.clone(),
            MinimaxCodeAccountScheduleState {
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
    schedules: &mut HashMap<String, MinimaxCodeAccountScheduleState>,
    account_id: &str,
    today: &str,
    current_minute: i32,
) {
    let schedule = schedules.entry(account_id.to_string()).or_insert_with(|| {
        MinimaxCodeAccountScheduleState {
            scheduled_date: today.to_string(),
            scheduled_minute: current_minute,
            last_checked_date: None,
        }
    });
    schedule.last_checked_date = Some(today.to_string());
}

pub(crate) fn record_manual_result(
    account_id: &str,
    result: &Result<minimax_code_api::MinimaxCodeCheckinResult, String>,
    duration_ms: u64,
) -> Result<(), String> {
    let email = minimax_code_account::load_account(account_id)
        .map(|account| account.nickname.unwrap_or(account.email))
        .unwrap_or_else(|| account_id.to_string());
    let (status, message, credit) = match result {
        Ok(value) if value.success => (
            if value.already_claimed {
                "already_checked"
            } else {
                "success"
            },
            value.message.clone(),
            value.credit,
        ),
        Ok(value) => ("failed", value.message.clone(), None),
        Err(error) => ("failed", Some(error.clone()), None),
    };
    add_log_record(MinimaxCodeAutoCheckinLogRecord {
        id: format!(
            "manual_{}_{}",
            Local::now().timestamp_millis(),
            rand::random::<u16>()
        ),
        timestamp: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        date: get_today_date_string(),
        duration_ms,
        total_accounts: 1,
        success_count: usize::from(status == "success"),
        already_checked_count: usize::from(status == "already_checked"),
        failed_count: usize::from(status == "failed"),
        status: if status == "failed" {
            "failed"
        } else {
            "success"
        }
        .into(),
        details: vec![MinimaxCodeAutoCheckinAccountDetail {
            account_id: account_id.into(),
            email,
            status: status.into(),
            time: Some(format_time_only()),
            message,
            credit: credit.map(|value| serde_json::json!(value)),
        }],
    })
}

pub async fn run_minimax_code_auto_checkin_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if !force && !crate::modules::tray_layout::is_platform_enabled("minimaxcode") {
        return Ok("platform_disabled".to_string());
    }
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let mut config = tauri::async_runtime::spawn_blocking(get_config_checked)
        .await
        .map_err(|error| format!("自动签到状态任务失败: {error}"))??;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = minimax_code_account::list_accounts();
    if accounts.is_empty() {
        if force {
            persist_log_record(MinimaxCodeAutoCheckinLogRecord {
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
                failed_count: 0,
                status: "no_accounts".to_string(),
                details: Vec::new(),
            })
            .await?;
            let _ = app.emit("minimax-code-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    let previous = config.account_schedules.clone();
    let schedule_changed = ensure_account_schedules(&mut config, &accounts);
    if schedule_changed {
        save_schedules(previous, config.account_schedules.clone()).await?;
        let _ = app.emit("minimax-code-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;

    let target_accounts: Vec<&MinimaxCodeAccount> = if force {
        accounts.iter().collect()
    } else {
        accounts
            .iter()
            .filter(|account| {
                let sch = config
                    .account_schedules
                    .as_ref()
                    .and_then(|s| s.get(&account.id));
                match sch {
                    Some(s) => {
                        if s.last_checked_date.as_deref() == Some(&today_str) {
                            return false;
                        }
                        s.scheduled_date == today_str && current_minute >= s.scheduled_minute
                    }
                    None => false,
                }
            })
            .collect()
    };

    if target_accounts.is_empty() {
        return Ok("waiting".to_string());
    }

    logger::log_info(&format!(
        "[MinimaxCodeAutoCheckin] 开始处理后台签到，目标账号数: {}",
        target_accounts.len()
    ));

    let start_instant = Instant::now();
    let start_timestamp_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut success_count = 0;
    let mut already_checked_count = 0;
    let mut failed_count = 0;
    let mut retry_needed = false;
    let mut details = Vec::new();
    let mut did_checkin_any = false;
    let mut new_schedules = config.account_schedules.clone().unwrap_or_default();
    let target_account_count = target_accounts.len();

    for account in target_accounts {
        // MiniMax tokens are ~1h and the client renews them locally; reload before
        // check-in so a stale in-memory token does not burn the retry budget.
        let account =
            match crate::modules::minimax_code_account::refresh_account_token(&account.id).await {
                Ok(refreshed) => refreshed,
                Err(err) => {
                    logger::log_warn(&format!(
                        "[MinimaxCodeAutoCheckin] 账号 {} 刷新失败: {}",
                        account.id, err
                    ));
                    failed_count += 1;
                    retry_needed = true;
                    details.push(MinimaxCodeAutoCheckinAccountDetail {
                        account_id: account.id.clone(),
                        email: account.email.clone(),
                        status: "failed".into(),
                        time: Some(format_time_only()),
                        message: Some(err),
                        credit: None,
                    });
                    continue;
                }
            };
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };
        let account_checkin_time = format_time_only();

        let region = account.region.as_deref().unwrap_or("cn");
        match minimax_code_api::get_checkin_status(&account.access_token, region).await {
            Ok(status) if status.today_checked_in => {
                already_checked_count += 1;
                details.push(MinimaxCodeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "already_checked".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("今日已完成签到".to_string()),
                    credit: Some(serde_json::json!(status.points)),
                });
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
            }
            Ok(status) if !status.active => {
                // The activity can be temporarily unavailable while the daily
                // window is opening. Keep this account eligible for a retry
                // instead of consuming today's schedule.
                retry_needed = true;
                failed_count += 1;
                details.push(MinimaxCodeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "failed".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("签到活动未开启或不适用".to_string()),
                    credit: None,
                });
            }
            Ok(_) => match minimax_code_api::checkin_account(&account.id).await {
                Ok(res) if res.success => {
                    if res.already_claimed {
                        already_checked_count += 1;
                    } else {
                        success_count += 1;
                        did_checkin_any = true;
                    }
                    details.push(MinimaxCodeAutoCheckinAccountDetail {
                        account_id: account.id.clone(),
                        email: email_display,
                        status: if res.already_claimed {
                            "already_checked"
                        } else {
                            "success"
                        }
                        .to_string(),
                        time: Some(account_checkin_time),
                        message: res.message.clone(),
                        credit: res.credit.map(|c| serde_json::json!(c)),
                    });

                    mark_schedule_checked(
                        &mut new_schedules,
                        &account.id,
                        &today_str,
                        current_minute,
                    );
                }
                Ok(res) => {
                    match minimax_code_api::get_checkin_status(&account.access_token, region).await
                    {
                        Ok(latest_status) if latest_status.today_checked_in => {
                            already_checked_count += 1;
                            details.push(MinimaxCodeAutoCheckinAccountDetail {
                                account_id: account.id.clone(),
                                email: email_display,
                                status: "already_checked".to_string(),
                                time: Some(account_checkin_time),
                                message: Some("今日已完成签到".to_string()),
                                credit: None,
                            });
                            mark_schedule_checked(
                                &mut new_schedules,
                                &account.id,
                                &today_str,
                                current_minute,
                            );
                        }
                        _ => {
                            retry_needed = true;
                            failed_count += 1;
                            details.push(MinimaxCodeAutoCheckinAccountDetail {
                                account_id: account.id.clone(),
                                email: email_display,
                                status: "failed".to_string(),
                                time: Some(account_checkin_time),
                                message: Some(
                                    res.message
                                        .clone()
                                        .unwrap_or_else(|| "签到失败".to_string()),
                                ),
                                credit: None,
                            });
                        }
                    }
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[MinimaxCodeAutoCheckin] 账号 {} 自动签到异常: {}",
                        account.id, err
                    ));
                    retry_needed = true;
                    failed_count += 1;
                    details.push(MinimaxCodeAutoCheckinAccountDetail {
                        account_id: account.id.clone(),
                        email: email_display,
                        status: "failed".to_string(),
                        time: Some(account_checkin_time),
                        message: Some(err),
                        credit: None,
                    });
                }
            },
            Err(err) => {
                logger::log_warn(&format!(
                    "[MinimaxCodeAutoCheckin] 账号 {} 签到状态检查异常: {}",
                    account.id, err
                ));
                retry_needed = true;
                failed_count += 1;
                details.push(MinimaxCodeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "failed".to_string(),
                    time: Some(account_checkin_time),
                    message: Some(err),
                    credit: None,
                });
            }
        }
    }

    save_schedules(config.account_schedules, Some(new_schedules)).await?;

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let overall_status = if failed_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    };

    persist_log_record(MinimaxCodeAutoCheckinLogRecord {
        id: format!(
            "log_{}_{}",
            Local::now().timestamp_millis(),
            rand::random::<u16>()
        ),
        timestamp: start_timestamp_str,
        date: today_str,
        duration_ms,
        total_accounts: target_account_count,
        success_count,
        already_checked_count,
        failed_count,
        status: overall_status.to_string(),
        details,
    })
    .await?;

    if did_checkin_any {
        let _ = crate::modules::tray::update_tray_menu(app);
    }

    let _ = app.emit("minimax-code-auto-checkin-logs-changed", ());
    let _ = app.emit("minimax-code-auto-checkin-config-changed", ());

    if retry_needed {
        Ok("retry".to_string())
    } else {
        Ok("completed".to_string())
    }
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

pub fn start_auto_checkin_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    tauri::async_runtime::spawn(async move {
        logger::log_info("[MinimaxCodeAutoCheckin] 后台自动签到调度服务已启动");
        // Run once as soon as the app starts. This picks up a migrated config
        // immediately and catches schedules missed while the app was closed.
        let mut next_delay = Duration::ZERO;
        let mut retry_delay = INITIAL_RETRY_DELAY;
        loop {
            if !next_delay.is_zero() {
                tokio::select! {
                    _ = tokio::time::sleep(next_delay) => {}
                    _ = wake.notified() => {
                        next_delay = Duration::ZERO;
                        retry_delay = INITIAL_RETRY_DELAY;
                        continue;
                    }
                }
            }
            match run_minimax_code_auto_checkin_cycle_if_needed(&app, false).await {
                Ok(result) if result == "retry" => {
                    next_delay = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                    logger::log_warn(&format!(
                        "[MinimaxCodeAutoCheckin] 本轮存在失败，{} 秒后重试",
                        next_delay.as_secs()
                    ));
                }
                Ok(_) => {
                    next_delay = SCHEDULER_POLL_DELAY;
                    retry_delay = INITIAL_RETRY_DELAY;
                }
                Err(err) => {
                    next_delay = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                    logger::log_warn(&format!(
                        "[MinimaxCodeAutoCheckin] 调度异常: {}，{} 秒后重试",
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

    fn make_test_account(id: &str, email: &str, access_token: &str) -> MinimaxCodeAccount {
        MinimaxCodeAccount {
            last_switched_at: None,
            id: id.to_string(),
            email: email.to_string(),
            uid: None,
            nickname: None,
            tags: None,
            access_token: access_token.to_string(),
            refresh_token: None,
            token_type: None,
            expires_at: None,
            region: Some("cn".to_string()),
            auth_file: None,
            plan_type: None,
            quota_raw: None,
            auth_raw: None,
            profile_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            last_checkin_time: None,
            checkin_streak: None,
            checkin_rewards: None,
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn test_parse_time_to_minutes() {
        assert_eq!(parse_time_to_minutes("06:00"), 360);
        assert_eq!(parse_time_to_minutes("12:30"), 750);
        assert_eq!(parse_time_to_minutes("00:00"), 0);
        assert_eq!(parse_time_to_minutes("23:59"), 1439);
        assert_eq!(parse_time_to_minutes("invalid"), 0);
    }

    #[test]
    fn test_ensure_account_schedules() {
        let mut config = MinimaxCodeAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        };

        let accounts = vec![
            make_test_account("acc_1", "acc1@example.com", "token1"),
            make_test_account("acc_2", "acc2@example.com", "token2"),
        ];

        let changed = ensure_account_schedules(&mut config, &accounts);
        assert!(changed);

        let schedules = config.account_schedules.unwrap();
        assert_eq!(schedules.len(), 2);

        let sch1 = schedules.get("acc_1").unwrap();
        assert!(sch1.scheduled_minute >= 360 && sch1.scheduled_minute <= 720);

        let sch2 = schedules.get("acc_2").unwrap();
        assert!(sch2.scheduled_minute >= 360 && sch2.scheduled_minute <= 720);
    }

    #[test]
    fn test_config_serde() {
        let config = MinimaxCodeAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: Some("2026-07-28".to_string()),
            account_schedules: None,
            device_ids: None,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"startTime\":\"08:00\""));

        let deserialized: MinimaxCodeAutoCheckinConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.start_time, "08:00");
    }

    #[test]
    fn test_config_validation_rejects_invalid_time_and_schedule() {
        let mut config = MinimaxCodeAutoCheckinConfig {
            enabled: true,
            start_time: "12:00".to_string(),
            end_time: "06:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        };
        assert!(validate_config(&config).is_err());

        config.start_time = "06:00".to_string();
        config.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            MinimaxCodeAccountScheduleState {
                scheduled_date: "2026-07-29".to_string(),
                scheduled_minute: 1440,
                last_checked_date: None,
            },
        )]));
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn test_retry_delay_uses_bounded_exponential_backoff() {
        assert_eq!(
            next_retry_delay(INITIAL_RETRY_DELAY),
            Duration::from_secs(10 * 60)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(45 * 60)),
            MAX_RETRY_DELAY
        );
        assert_eq!(next_retry_delay(MAX_RETRY_DELAY), MAX_RETRY_DELAY);
    }
}
