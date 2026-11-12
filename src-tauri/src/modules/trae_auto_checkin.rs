use super::checkin_state_store::{validate_config, CheckinStore};
pub use super::checkin_state_store::{
    AccountSchedule as TraeAccountScheduleState, CheckinConfig as TraeAutoCheckinConfig,
};
use super::wakeup_history_store::{self, checkin_timestamp, HistoryRow, HistoryStore};
// Trae CN 自动签到：后台调度器、每账号随机分散调度与持久化。
// 结构对齐 workbuddy_auto_checkin.rs，差异点：CN 平台过滤、user_id 签到去重、
// 签到 device_id 管理、签到 API 走 trae_account（api.trae.cn checkin_credits）。
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

use crate::models::trae::TraeAccount;
use crate::modules::{config, logger, trae_account};

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
pub struct TraeAutoCheckinAccountDetail {
    pub account_id: String,
    pub email: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraeAutoCheckinLogRecord {
    pub id: String,
    pub timestamp: String,
    pub date: String,
    pub duration_ms: u64,
    pub total_accounts: usize,
    pub success_count: usize,
    pub already_checked_count: usize,
    pub failed_count: usize,
    pub status: String,
    pub details: Vec<TraeAutoCheckinAccountDetail>,
}

pub(crate) const CONFIG_FILE: &str = "trae_auto_checkin_config.json";

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join(CONFIG_FILE)
}

pub(crate) const LOGS_FILE: &str = "trae_auto_checkin_logs.json";

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
    CheckinStore::new(path, "trae-cn")
}

pub fn get_config_checked() -> Result<TraeAutoCheckinConfig, String> {
    state_store().load()
}

pub fn save_config(config: &TraeAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    state_store().save_rules(config)?;
    wake_scheduler();
    Ok(())
}

pub fn migrate_config_if_missing(
    legacy_config: &TraeAutoCheckinConfig,
) -> Result<TraeAutoCheckinConfig, String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "自动签到存储锁已损坏")?;
    let config = state_store().seed_webview(legacy_config)?;
    wake_scheduler();
    Ok(config)
}

async fn save_schedules(
    previous: Option<HashMap<String, TraeAccountScheduleState>>,
    schedules: Option<HashMap<String, TraeAccountScheduleState>>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        state_store().save_schedules(previous.as_ref(), schedules)
    })
    .await
    .map_err(|error| format!("自动签到状态任务失败: {error}"))?
}

impl HistoryRow for TraeAutoCheckinLogRecord {
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
    wakeup_history_store::daily_checkin(path, "trae-cn")
}

pub fn schedule_log_migration() {
    history_store().schedule::<TraeAutoCheckinLogRecord>();
}

pub fn get_logs_checked(
    retry: bool,
) -> Result<wakeup_history_store::HistoryRead<TraeAutoCheckinLogRecord>, String> {
    wakeup_history_store::read_available(history_store(), retry)
}

pub fn clear_logs() -> Result<(), String> {
    history_store().clear()
}

/// 同日多轮签到合并到既有记录：明细按账号覆盖、耗时累加、计数与状态重算。
/// totalAccounts 排除 inactive（签到活动不可用的账号），避免出现「成功 (0/0)」误导。
fn merge_log_record(existing: &mut TraeAutoCheckinLogRecord, record: TraeAutoCheckinLogRecord) {
    let mut details: HashMap<String, TraeAutoCheckinAccountDetail> = existing
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
    existing.total_accounts = existing
        .details
        .iter()
        .filter(|detail| detail.status != "inactive")
        .count();
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

fn add_log_record(record: TraeAutoCheckinLogRecord) -> Result<(), String> {
    const THIRTY_DAYS_SECS: i64 = 30 * 24 * 60 * 60;
    history_store().merge_day(
        record,
        Local::now().timestamp() - THIRTY_DAYS_SECS,
        merge_log_record,
    )
}

async fn persist_log_record(record: TraeAutoCheckinLogRecord) -> Result<(), String> {
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
    config: &mut TraeAutoCheckinConfig,
    accounts: &[&TraeAccount],
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
            TraeAccountScheduleState {
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

/// 归一化 user_id：trim 且非空才视为有效身份
fn normalized_user_id(account: &TraeAccount) -> Option<String> {
    account
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

/// CN 签到按 user_id 去重：同一物理账号可同时存在于 trae_cn / trae_solo_cn
/// （存储按平台隔离），但服务端签到/积分是用户级的，每组只签代表
/// （id 字典序最小，确定且代表被删后自动切换）；user_id 缺失的记录不归组。
fn dedup_checkin_accounts<'a>(accounts: &'a [&'a TraeAccount]) -> Vec<&'a TraeAccount> {
    let mut representative_ids: HashMap<String, String> = HashMap::new();
    for account in accounts {
        let Some(user_id) = normalized_user_id(account) else {
            continue;
        };
        match representative_ids.get(&user_id) {
            Some(existing) if existing < &account.id => {}
            _ => {
                representative_ids.insert(user_id, account.id.clone());
            }
        }
    }

    accounts
        .iter()
        .copied()
        .filter(|account| match normalized_user_id(account) {
            None => true,
            Some(user_id) => representative_ids
                .get(&user_id)
                .is_some_and(|id| id == &account.id),
        })
        .collect()
}

/// 风控要求 did 为 16 位纯数字
fn is_valid_device_id(device_id: &str) -> bool {
    device_id.len() == 16 && device_id.chars().all(|c| c.is_ascii_digit())
}

/// 签到接口按 did 全局判重，生成结果须由持久化存储检查并复用。
fn device_candidate() -> String {
    format!(
        "{}{:03}",
        Local::now().timestamp_millis(),
        rand::random::<u16>() % 1000
    )
}

pub async fn ensure_device_id_persisted(account_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        state_store().device_id(&account_id, is_valid_device_id, device_candidate)
    })
    .await
    .map_err(|error| format!("签到设备标识任务失败: {error}"))?
}

fn mark_schedule_checked(
    schedules: &mut HashMap<String, TraeAccountScheduleState>,
    account_id: &str,
    today: &str,
    current_minute: i32,
) {
    let schedule =
        schedules
            .entry(account_id.to_string())
            .or_insert_with(|| TraeAccountScheduleState {
                scheduled_date: today.to_string(),
                scheduled_minute: current_minute,
                last_checked_date: None,
            });
    schedule.last_checked_date = Some(today.to_string());
}

pub async fn run_trae_auto_checkin_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
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

    let all_accounts = trae_account::list_accounts_checked()?;
    // 签到接口仅存在于 Trae CN（api.trae.cn），国际站账号直接跳过
    let cn_accounts: Vec<&TraeAccount> = all_accounts
        .iter()
        .filter(|account| {
            let platform = trae_account::resolve_account_platform_kind(account);
            platform.is_cn()
                && (force
                    || crate::modules::tray_layout::is_platform_enabled(platform.provider_key()))
        })
        .collect();
    let accounts: Vec<&TraeAccount> = dedup_checkin_accounts(&cn_accounts);
    if accounts.is_empty() {
        if force {
            persist_log_record(TraeAutoCheckinLogRecord {
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
            let _ = app.emit("trae-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    let previous = config.account_schedules.clone();
    let schedule_changed = ensure_account_schedules(&mut config, &accounts);
    if schedule_changed {
        save_schedules(previous, config.account_schedules.clone()).await?;
        let _ = app.emit("trae-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;

    let target_accounts: Vec<&TraeAccount> = if force {
        accounts.iter().copied().collect()
    } else {
        accounts
            .iter()
            .copied()
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
        "[TraeAutoCheckin] 开始处理后台签到，目标账号数: {}",
        target_accounts.len()
    ));

    let start_instant = Instant::now();
    let start_timestamp_str = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut success_count = 0;
    let mut already_checked_count = 0;
    let mut failed_count = 0;
    let mut retry_needed = false;
    let mut details = Vec::new();
    let mut claimed_account_ids: Vec<String> = Vec::new();
    let mut new_schedules = config.account_schedules.clone().unwrap_or_default();

    for account in target_accounts {
        let email_display = if !account.email.trim().is_empty() {
            account.email.clone()
        } else {
            account.id.clone()
        };
        let account_checkin_time = format_time_only();
        let device_id = ensure_device_id_persisted(account.id.clone()).await?;

        match trae_account::get_trae_checkin_status(&account.id, &device_id).await {
            Ok(status) if status.checked_in => {
                already_checked_count += 1;
                details.push(TraeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "already_checked".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("今日已完成签到".to_string()),
                });
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
            }
            Ok(status) if !status.active => {
                // 签到活动可能仅在每日窗口开启瞬间不可用：保持可重试，
                // 不消耗当天调度（对齐 WorkBuddy 语义）
                retry_needed = true;
                failed_count += 1;
                details.push(TraeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "inactive".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("签到活动不可用".to_string()),
                });
            }
            Ok(_) => {
                match trae_account::claim_trae_checkin(&account.id, &device_id).await {
                    // claim 成功即已签到（9095 幂等已在 API 层归一为成功）
                    Ok(_) => {
                        success_count += 1;
                        claimed_account_ids.push(account.id.clone());
                        details.push(TraeAutoCheckinAccountDetail {
                            account_id: account.id.clone(),
                            email: email_display,
                            status: "success".to_string(),
                            time: Some(account_checkin_time),
                            message: Some("签到成功".to_string()),
                        });
                        mark_schedule_checked(
                            &mut new_schedules,
                            &account.id,
                            &today_str,
                            current_minute,
                        );
                    }
                    Err(claim_err) => {
                        // 领取报错但服务端可能已入账：复查状态兜底
                        match trae_account::get_trae_checkin_status(&account.id, &device_id).await {
                            Ok(latest) if latest.checked_in => {
                                already_checked_count += 1;
                                details.push(TraeAutoCheckinAccountDetail {
                                    account_id: account.id.clone(),
                                    email: email_display,
                                    status: "already_checked".to_string(),
                                    time: Some(account_checkin_time),
                                    message: Some("今日已完成签到".to_string()),
                                });
                                mark_schedule_checked(
                                    &mut new_schedules,
                                    &account.id,
                                    &today_str,
                                    current_minute,
                                );
                            }
                            _ => {
                                logger::log_warn(&format!(
                                    "[TraeAutoCheckin] 账号 {} 自动签到异常: {}",
                                    account.id, claim_err
                                ));
                                retry_needed = true;
                                failed_count += 1;
                                details.push(TraeAutoCheckinAccountDetail {
                                    account_id: account.id.clone(),
                                    email: email_display,
                                    status: "failed".to_string(),
                                    time: Some(account_checkin_time),
                                    message: Some(claim_err),
                                });
                            }
                        }
                    }
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[TraeAutoCheckin] 账号 {} 签到状态检查异常: {}",
                    account.id, err
                ));
                retry_needed = true;
                failed_count += 1;
                details.push(TraeAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "failed".to_string(),
                    time: Some(account_checkin_time),
                    message: Some(err),
                });
            }
        }
    }

    save_schedules(config.account_schedules, Some(new_schedules)).await?;

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let total_accounts = details.iter().filter(|d| d.status != "inactive").count();
    let overall_status = if total_accounts == 0 {
        "no_accounts"
    } else if failed_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    };

    persist_log_record(TraeAutoCheckinLogRecord {
        id: format!(
            "log_{}_{}",
            Local::now().timestamp_millis(),
            rand::random::<u16>()
        ),
        timestamp: start_timestamp_str,
        date: today_str,
        duration_ms,
        total_accounts,
        success_count,
        already_checked_count,
        failed_count,
        status: overall_status.to_string(),
        details,
    })
    .await?;

    if !claimed_account_ids.is_empty() {
        logger::log_info(&format!(
            "[TraeAutoCheckin] 本轮签到成功 {} 个账号，开始刷新额度",
            claimed_account_ids.len()
        ));
        // 签到改变的是用户级积分：同 user 的兄弟平台记录一并刷新，保持积分显示一致
        let claimed_user_ids: Vec<String> = all_accounts
            .iter()
            .filter(|account| claimed_account_ids.contains(&account.id))
            .filter_map(normalized_user_id)
            .collect();
        let refresh_ids: Vec<String> = all_accounts
            .iter()
            .filter(|account| {
                let platform = trae_account::resolve_account_platform_kind(account);
                platform.is_cn()
                    && (force
                        || crate::modules::tray_layout::is_platform_enabled(
                            platform.provider_key(),
                        ))
            })
            .filter(|account| {
                claimed_account_ids.contains(&account.id)
                    || normalized_user_id(account)
                        .is_some_and(|user_id| claimed_user_ids.contains(&user_id))
            })
            .map(|account| account.id.clone())
            .collect();
        for account_id in &refresh_ids {
            if let Err(err) = trae_account::refresh_account_usage_only_async(account_id, None).await
            {
                logger::log_warn(&format!(
                    "[TraeAutoCheckin] 账号 {} 签到后额度刷新失败: {}",
                    account_id, err
                ));
            }
        }
        let _ = crate::modules::tray::update_tray_menu(app);
        let _ = app.emit("trae-auto-checkin-accounts-changed", ());
    }

    let _ = app.emit("trae-auto-checkin-logs-changed", ());
    let _ = app.emit("trae-auto-checkin-config-changed", ());

    if retry_needed {
        Ok("retry".to_string())
    } else {
        Ok("completed".to_string())
    }
}

fn next_retry_delay(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_RETRY_DELAY)
}

pub fn start_trae_auto_checkin_scheduler(app: AppHandle) {
    let wake = scheduler_wake();
    tauri::async_runtime::spawn(async move {
        logger::log_info("[TraeAutoCheckin] 后台自动签到调度服务已启动");
        // Run once as soon as the app starts to catch schedules missed
        // while the app was closed.
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
            match run_trae_auto_checkin_cycle_if_needed(&app, false).await {
                Ok(result) if result == "retry" => {
                    next_delay = retry_delay;
                    retry_delay = next_retry_delay(retry_delay);
                    logger::log_warn(&format!(
                        "[TraeAutoCheckin] 本轮存在失败，{} 秒后重试",
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
                        "[TraeAutoCheckin] 调度异常: {}，{} 秒后重试",
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

    fn trae_account_min(id: &str, email: &str) -> TraeAccount {
        TraeAccount {
            last_switched_at: None,
            id: id.to_string(),
            email: email.to_string(),
            user_id: None,
            nickname: None,
            tags: None,
            access_token: "token".to_string(),
            refresh_token: None,
            token_type: None,
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: None,
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
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
        let mut config = TraeAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        };

        let accounts = vec![
            trae_account_min("acc_1", "acc1@example.com"),
            trae_account_min("acc_2", "acc2@example.com"),
        ];
        let account_refs: Vec<&TraeAccount> = accounts.iter().collect();

        let changed = ensure_account_schedules(&mut config, &account_refs);
        assert!(changed);

        let schedules = config.account_schedules.unwrap();
        assert_eq!(schedules.len(), 2);

        let sch1 = schedules.get("acc_1").unwrap();
        assert!(sch1.scheduled_minute >= 360 && sch1.scheduled_minute <= 720);

        let sch2 = schedules.get("acc_2").unwrap();
        assert!(sch2.scheduled_minute >= 360 && sch2.scheduled_minute <= 720);
    }

    #[test]
    fn test_dedup_picks_lexicographically_smallest_id_per_user() {
        let mut accounts = vec![
            trae_account_min("trae_b", "a@example.com"),
            trae_account_min("trae_a", "a@example.com"),
            trae_account_min("trae_c", "b@example.com"),
        ];
        accounts[0].user_id = Some("u1".to_string());
        accounts[1].user_id = Some("u1".to_string());
        accounts[2].user_id = Some("u2".to_string());

        let refs: Vec<&TraeAccount> = accounts.iter().collect();
        let deduped = dedup_checkin_accounts(&refs);
        let ids: Vec<&str> = deduped.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["trae_a", "trae_c"]);
    }

    #[test]
    fn test_dedup_keeps_accounts_without_user_id_independent() {
        let mut accounts = vec![
            trae_account_min("trae_a", "a@example.com"),
            trae_account_min("trae_b", "a@example.com"),
            trae_account_min("trae_c", "b@example.com"),
        ];
        accounts[0].user_id = Some("u1".to_string());

        let refs: Vec<&TraeAccount> = accounts.iter().collect();
        let deduped = dedup_checkin_accounts(&refs);
        let ids: Vec<&str> = deduped.iter().map(|a| a.id.as_str()).collect();
        // user_id 缺失（含空白）不归组，维持独立签到
        assert_eq!(ids, vec!["trae_a", "trae_b", "trae_c"]);

        accounts[1].user_id = Some("  ".to_string());
        let refs: Vec<&TraeAccount> = accounts.iter().collect();
        assert_eq!(dedup_checkin_accounts(&refs).len(), 3);
    }

    #[test]
    fn test_dedup_representative_switches_to_next_when_removed() {
        let make = |id: &str| {
            let mut account = trae_account_min(id, "a@example.com");
            account.user_id = Some("u1".to_string());
            account
        };
        let accounts = vec![make("trae_b"), make("trae_c")];
        let refs: Vec<&TraeAccount> = accounts.iter().collect();
        let ids: Vec<&str> = dedup_checkin_accounts(&refs)
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, vec!["trae_b"]);

        // 代表被删后组内自动切到次小 id
        let remaining = vec![accounts[1].clone()];
        let refs: Vec<&TraeAccount> = remaining.iter().collect();
        let ids: Vec<&str> = dedup_checkin_accounts(&refs)
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, vec!["trae_c"]);
    }

    #[test]
    fn test_config_serde() {
        let config = TraeAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: Some("2026-10-03".to_string()),
            account_schedules: None,
            device_ids: Some(HashMap::from([(
                "acc_1".to_string(),
                "1234567890123456".to_string(),
            )])),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"startTime\":\"08:00\""));
        assert!(json.contains("\"deviceIds\":{\"acc_1\":\"1234567890123456\"}"));

        let deserialized: TraeAutoCheckinConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.start_time, "08:00");
        assert_eq!(
            deserialized.device_ids.unwrap().get("acc_1"),
            Some(&"1234567890123456".to_string())
        );
    }

    #[test]
    fn test_config_validation_rejects_invalid_time_and_schedule() {
        let mut config = TraeAutoCheckinConfig {
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
            TraeAccountScheduleState {
                scheduled_date: "2026-10-03".to_string(),
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

    #[test]
    fn test_merge_log_record_excludes_inactive_from_total() {
        let mut existing = TraeAutoCheckinLogRecord {
            id: "log_1".to_string(),
            timestamp: "2026-10-03 09:00:00".to_string(),
            date: "2026-10-03".to_string(),
            duration_ms: 100,
            total_accounts: 1,
            success_count: 1,
            already_checked_count: 0,
            failed_count: 0,
            status: "success".to_string(),
            details: vec![TraeAutoCheckinAccountDetail {
                account_id: "acc_1".to_string(),
                email: "acc1@example.com".to_string(),
                status: "success".to_string(),
                time: None,
                message: None,
            }],
        };

        let record = TraeAutoCheckinLogRecord {
            id: "log_2".to_string(),
            timestamp: "2026-10-03 10:00:00".to_string(),
            date: "2026-10-03".to_string(),
            duration_ms: 50,
            total_accounts: 0,
            success_count: 0,
            already_checked_count: 0,
            failed_count: 0,
            status: "partial".to_string(),
            details: vec![
                TraeAutoCheckinAccountDetail {
                    account_id: "acc_1".to_string(),
                    email: "acc1@example.com".to_string(),
                    status: "already_checked".to_string(),
                    time: None,
                    message: None,
                },
                TraeAutoCheckinAccountDetail {
                    account_id: "acc_2".to_string(),
                    email: "acc2@example.com".to_string(),
                    status: "inactive".to_string(),
                    time: None,
                    message: None,
                },
            ],
        };

        merge_log_record(&mut existing, record);

        assert_eq!(existing.total_accounts, 1);
        assert_eq!(existing.already_checked_count, 1);
        assert_eq!(existing.success_count, 0);
        assert_eq!(existing.failed_count, 0);
        assert_eq!(existing.status, "success");
        assert_eq!(existing.duration_ms, 150);
        assert_eq!(existing.details.len(), 2);
    }

    #[test]
    fn test_merge_log_record_all_inactive_reports_no_accounts() {
        let mut existing = TraeAutoCheckinLogRecord {
            id: "log_1".to_string(),
            timestamp: "2026-10-03 09:00:00".to_string(),
            date: "2026-10-03".to_string(),
            duration_ms: 0,
            total_accounts: 0,
            success_count: 0,
            already_checked_count: 0,
            failed_count: 0,
            status: "no_accounts".to_string(),
            details: vec![],
        };

        let record = TraeAutoCheckinLogRecord {
            id: "log_2".to_string(),
            timestamp: "2026-10-03 10:00:00".to_string(),
            date: "2026-10-03".to_string(),
            duration_ms: 0,
            total_accounts: 0,
            success_count: 0,
            already_checked_count: 0,
            failed_count: 1,
            status: "failed".to_string(),
            details: vec![TraeAutoCheckinAccountDetail {
                account_id: "acc_1".to_string(),
                email: "acc1@example.com".to_string(),
                status: "inactive".to_string(),
                time: None,
                message: None,
            }],
        };

        merge_log_record(&mut existing, record);

        assert_eq!(existing.total_accounts, 0);
        assert_eq!(existing.status, "no_accounts");
    }
}
