use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, MutexGuard, OnceLock,
};
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone, Timelike};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeAccountScheduleState {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeAutoCheckinConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, MinimaxCodeAccountScheduleState>>,
}

impl Default for MinimaxCodeAutoCheckinConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        }
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

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join("minimax_code_auto_checkin_config.json")
}

fn get_logs_file_path() -> PathBuf {
    config::get_shared_dir().join("minimax_code_auto_checkin_logs.json")
}

fn scheduler_wake() -> &'static Notify {
    SCHEDULER_WAKE.get_or_init(Notify::new)
}

fn wake_scheduler() {
    scheduler_wake().notify_one();
}

fn lock_storage() -> Result<MutexGuard<'static, ()>, String> {
    STORAGE_LOCK
        .lock()
        .map_err(|_| "MiniMax Code 自动签到存储锁已损坏".to_string())
}

fn validate_time(value: &str) -> Option<i32> {
    if value.len() != 5 || value.as_bytes().get(2) != Some(&b':') {
        return None;
    }
    let hour = value.get(0..2)?.parse::<i32>().ok()?;
    let minute = value.get(3..5)?.parse::<i32>().ok()?;
    if (0..=23).contains(&hour) && (0..=59).contains(&minute) {
        Some(hour * 60 + minute)
    } else {
        None
    }
}

fn validate_config(config: &MinimaxCodeAutoCheckinConfig) -> Result<(), String> {
    let start = validate_time(&config.start_time)
        .ok_or_else(|| format!("自动签到开始时间无效: {}", config.start_time))?;
    let end = validate_time(&config.end_time)
        .ok_or_else(|| format!("自动签到结束时间无效: {}", config.end_time))?;
    if start > end {
        return Err("自动签到开始时间不能晚于结束时间".to_string());
    }
    if let Some(schedules) = &config.account_schedules {
        for (account_id, schedule) in schedules {
            if !(0..=1439).contains(&schedule.scheduled_minute) {
                return Err(format!(
                    "账号 {} 的自动签到分钟无效: {}",
                    account_id, schedule.scheduled_minute
                ));
            }
        }
    }
    Ok(())
}

fn read_config_from_path(path: &Path) -> Result<Option<MinimaxCodeAutoCheckinConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 MiniMax Code 自动签到配置失败: {}", e))?;
    let config = crate::modules::atomic_write::parse_json_with_auto_restore(path, &content)
        .map_err(|e| format!("解析 MiniMax Code 自动签到配置失败: {}", e))?;
    validate_config(&config)?;
    Ok(Some(config))
}

fn write_config_to_path(path: &Path, config: &MinimaxCodeAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 MiniMax Code 自动签到配置失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("保存 MiniMax Code 自动签到配置失败: {}", e))
}

pub fn get_config_checked() -> Result<MinimaxCodeAutoCheckinConfig, String> {
    let _guard = lock_storage()?;
    Ok(read_config_from_path(&get_config_file_path())?.unwrap_or_default())
}

pub fn save_config(config: &MinimaxCodeAutoCheckinConfig) -> Result<(), String> {
    let result = save_config_without_wake(config);
    if result.is_ok() {
        wake_scheduler();
    }
    result
}

fn save_config_without_wake(config: &MinimaxCodeAutoCheckinConfig) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_config_to_path(&get_config_file_path(), config)
}

fn migrate_config_at_path(
    path: &Path,
    legacy_config: &MinimaxCodeAutoCheckinConfig,
) -> Result<MinimaxCodeAutoCheckinConfig, String> {
    validate_config(legacy_config)?;
    if let Some(existing) = read_config_from_path(path)? {
        return Ok(existing);
    }
    write_config_to_path(path, legacy_config)?;
    Ok(legacy_config.clone())
}

pub fn migrate_config_if_missing(
    legacy_config: &MinimaxCodeAutoCheckinConfig,
) -> Result<MinimaxCodeAutoCheckinConfig, String> {
    let (config, was_missing) = {
        let _guard = lock_storage()?;
        let path = get_config_file_path();
        let was_missing = !path.exists();
        let config = migrate_config_at_path(&path, legacy_config)?;
        (config, was_missing)
    };
    if was_missing {
        logger::log_info("[MinimaxCodeAutoCheckin] 已完成 WebView 旧配置的一次性迁移");
    }
    wake_scheduler();
    Ok(config)
}

fn read_logs_from_path(path: &Path) -> Result<Vec<MinimaxCodeAutoCheckinLogRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 MiniMax Code 自动签到日志失败: {}", e))?;
    crate::modules::atomic_write::parse_json_with_auto_restore(path, &content)
        .map_err(|e| format!("解析 MiniMax Code 自动签到日志失败: {}", e))
}

fn write_logs_to_path(path: &Path, logs: &[MinimaxCodeAutoCheckinLogRecord]) -> Result<(), String> {
    let content = serde_json::to_string_pretty(logs)
        .map_err(|e| format!("序列化 MiniMax Code 自动签到日志失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("保存 MiniMax Code 自动签到日志失败: {}", e))
}

pub fn get_logs_checked() -> Result<Vec<MinimaxCodeAutoCheckinLogRecord>, String> {
    let _guard = lock_storage()?;
    read_logs_from_path(&get_logs_file_path())
}

pub fn save_logs(logs: &[MinimaxCodeAutoCheckinLogRecord]) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_logs_to_path(&get_logs_file_path(), logs)
}

fn add_log_record(record: MinimaxCodeAutoCheckinLogRecord) -> Result<(), String> {
    let _guard = lock_storage()?;
    let path = get_logs_file_path();
    let mut logs = read_logs_from_path(&path)?;
    if let Some(existing) = logs.iter_mut().find(|log| log.date == record.date) {
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
    } else {
        logs.insert(0, record);
    }

    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    let cutoff = Local::now().timestamp() - THIRTY_DAYS_SECS;

    logs.retain(|r| {
        if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(&r.timestamp, "%Y-%m-%d %H:%M:%S") {
            if let Some(local_dt) = Local.from_local_datetime(&ndt).single() {
                local_dt.timestamp() >= cutoff
            } else {
                ndt.and_utc().timestamp() >= cutoff
            }
        } else {
            true
        }
    });

    write_logs_to_path(&path, &logs)
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
    let schedule =
        schedules
            .entry(account_id.to_string())
            .or_insert_with(|| MinimaxCodeAccountScheduleState {
                scheduled_date: today.to_string(),
                scheduled_minute: current_minute,
                last_checked_date: None,
            });
    schedule.last_checked_date = Some(today.to_string());
}

pub async fn run_minimax_code_auto_checkin_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = minimax_code_account::list_accounts();
    if accounts.is_empty() {
        if force {
            add_log_record(MinimaxCodeAutoCheckinLogRecord {
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
            })?;
            let _ = app.emit("minimax-code-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    let schedule_changed = ensure_account_schedules(&mut config, &accounts);
    if schedule_changed {
        save_config_without_wake(&config)?;
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
                    status: "inactive".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("签到活动未开启或不适用".to_string()),
                    credit: None,
                });
            }
            Ok(_) => match minimax_code_api::perform_checkin(&account.access_token, region).await {
                Ok(res) if res.success => {
                    success_count += 1;
                    did_checkin_any = true;
                    let streak = res
                        .streak_days
                        .and_then(|v| i32::try_from(v).ok())
                        .unwrap_or_else(|| account.checkin_streak.unwrap_or(0).saturating_add(1));
                    let reward = res
                        .credit
                        .map(|credit| serde_json::json!({ "credit": credit }));
                    let _ = minimax_code_account::update_checkin_info(
                        &account.id,
                        Some(Local::now().timestamp()),
                        streak,
                        reward,
                    );

                    details.push(MinimaxCodeAutoCheckinAccountDetail {
                        account_id: account.id.clone(),
                        email: email_display,
                        status: "success".to_string(),
                        time: Some(account_checkin_time),
                        message: Some("签到成功".to_string()),
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
                    match minimax_code_api::get_checkin_status(&account.access_token, region).await {
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

    config.account_schedules = Some(new_schedules);
    save_config_without_wake(&config)?;

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let overall_status = if failed_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    };

    add_log_record(MinimaxCodeAutoCheckinLogRecord {
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
    })?;

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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_test_account(id: &str, email: &str, access_token: &str) -> MinimaxCodeAccount {
        MinimaxCodeAccount {
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

    fn make_temp_config_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "cockpit-minimax_code-auto-checkin-{}-{}-{}",
                test_name,
                std::process::id(),
                unique
            ))
            .join("minimax_code_auto_checkin_config.json")
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
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"startTime\":\"08:00\""));

        let deserialized: MinimaxCodeAutoCheckinConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.start_time, "08:00");
    }

    #[test]
    fn test_migration_only_writes_when_backend_config_is_missing() {
        let path = make_temp_config_path("migration");
        let legacy = MinimaxCodeAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "09:00".to_string(),
            last_checked_date: Some("2026-07-28".to_string()),
            account_schedules: None,
        };
        let migrated = migrate_config_at_path(&path, &legacy).unwrap();
        assert_eq!(migrated, legacy);

        let backend = MinimaxCodeAutoCheckinConfig {
            enabled: false,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: Some("2026-07-29".to_string()),
            account_schedules: None,
        };
        write_config_to_path(&path, &backend).unwrap();

        let preserved = migrate_config_at_path(&path, &legacy).unwrap();
        assert_eq!(preserved, backend);
        assert_eq!(read_config_from_path(&path).unwrap(), Some(backend));

        let temp_dir = path.parent().unwrap();
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn test_config_validation_rejects_invalid_time_and_schedule() {
        let mut config = MinimaxCodeAutoCheckinConfig {
            enabled: true,
            start_time: "12:00".to_string(),
            end_time: "06:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
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
