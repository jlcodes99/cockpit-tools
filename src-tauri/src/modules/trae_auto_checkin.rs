// Trae CN 自动签到：后台调度器、每账号随机分散调度与文件持久化。
// 结构对齐 workbuddy_auto_checkin.rs，差异点：CN 平台过滤、user_id 签到去重、
// 签到 device_id 管理、签到 API 走 trae_account（api.trae.cn checkin_credits）。
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraeAccountScheduleState {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraeAutoCheckinConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, TraeAccountScheduleState>>,
    // 账号 → 签到 device_id（16 位纯数字）。接口按 did 全局判重，必须稳定复用
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_ids: Option<HashMap<String, String>>,
}

impl Default for TraeAutoCheckinConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        }
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

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join("trae_auto_checkin_config.json")
}

fn get_logs_file_path() -> PathBuf {
    config::get_shared_dir().join("trae_auto_checkin_logs.json")
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
        .map_err(|_| "Trae 自动签到存储锁已损坏".to_string())
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

fn validate_config(config: &TraeAutoCheckinConfig) -> Result<(), String> {
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

fn read_config_from_path(path: &Path) -> Result<Option<TraeAutoCheckinConfig>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 Trae 自动签到配置失败: {}", e))?;
    let config = crate::modules::atomic_write::parse_json_with_auto_restore(path, &content)
        .map_err(|e| format!("解析 Trae 自动签到配置失败: {}", e))?;
    validate_config(&config)?;
    Ok(Some(config))
}

fn write_config_to_path(path: &Path, config: &TraeAutoCheckinConfig) -> Result<(), String> {
    validate_config(config)?;
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 Trae 自动签到配置失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("保存 Trae 自动签到配置失败: {}", e))
}

pub fn get_config_checked() -> Result<TraeAutoCheckinConfig, String> {
    let _guard = lock_storage()?;
    Ok(read_config_from_path(&get_config_file_path())?.unwrap_or_default())
}

pub fn save_config(config: &TraeAutoCheckinConfig) -> Result<(), String> {
    let result = save_config_without_wake(config);
    if result.is_ok() {
        wake_scheduler();
    }
    result
}

fn save_config_without_wake(config: &TraeAutoCheckinConfig) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_config_merging_disk(&get_config_file_path(), config, false)
}

/// 后台 cycle 写回调度状态：enabled/start/end 以磁盘为准，避免 cycle 持快照
/// 执行期间用户在弹窗保存的新配置被旧快照覆盖。
fn save_cycle_state_without_wake(config: &TraeAutoCheckinConfig) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_config_merging_disk(&get_config_file_path(), config, true)
}

/// 写盘前并入磁盘状态：did 双向合并（调用方可能持有旧快照，直接覆盖会丢掉
/// 期间产生的 did）；preserve_user_fields 时用户字段（enabled/start/end）以磁盘为准。
fn write_config_merging_disk(
    path: &Path,
    config: &TraeAutoCheckinConfig,
    preserve_user_fields: bool,
) -> Result<(), String> {
    let mut merged = config.clone();
    // 磁盘配置不可读/损坏时按无存量处理，不阻断保存
    if let Some(disk) = read_config_from_path(path).ok().flatten() {
        if preserve_user_fields {
            merged.enabled = disk.enabled;
            merged.start_time = disk.start_time;
            merged.end_time = disk.end_time;
        }
        if let Some(disk_ids) = disk.device_ids {
            let mut ids = disk_ids;
            if let Some(memory_ids) = config.device_ids.clone() {
                ids.extend(memory_ids);
            }
            merged.device_ids = Some(ids);
        }
    }
    write_config_to_path(path, &merged)
}

fn read_logs_from_path(path: &Path) -> Result<Vec<TraeAutoCheckinLogRecord>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("读取 Trae 自动签到日志失败: {}", e))?;
    crate::modules::atomic_write::parse_json_with_auto_restore(path, &content)
        .map_err(|e| format!("解析 Trae 自动签到日志失败: {}", e))
}

fn write_logs_to_path(path: &Path, logs: &[TraeAutoCheckinLogRecord]) -> Result<(), String> {
    let content = serde_json::to_string_pretty(logs)
        .map_err(|e| format!("序列化 Trae 自动签到日志失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("保存 Trae 自动签到日志失败: {}", e))
}

pub fn get_logs_checked() -> Result<Vec<TraeAutoCheckinLogRecord>, String> {
    let _guard = lock_storage()?;
    read_logs_from_path(&get_logs_file_path())
}

pub fn save_logs(logs: &[TraeAutoCheckinLogRecord]) -> Result<(), String> {
    let _guard = lock_storage()?;
    write_logs_to_path(&get_logs_file_path(), logs)
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
    let _guard = lock_storage()?;
    let path = get_logs_file_path();
    let mut logs = read_logs_from_path(&path)?;
    if let Some(existing) = logs.iter_mut().find(|log| log.date == record.date) {
        merge_log_record(existing, record);
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

/// 生成未被占用的 did：13 位毫秒时间戳 + 3 位随机数。
/// 同一毫秒内为多账号连续生成会撞号（接口按 did 全局判重），必须对齐既有 did 去重。
fn generate_device_id(existing: &HashMap<String, String>) -> String {
    loop {
        let candidate = format!(
            "{}{:03}",
            Local::now().timestamp_millis(),
            rand::random::<u16>() % 1000
        );
        if !existing.values().any(|value| value == &candidate) {
            return candidate;
        }
    }
}

/// 获取账号签到 device_id，无合法存量则生成并写回内存配置。
pub fn ensure_device_id(config: &mut TraeAutoCheckinConfig, account_id: &str) -> String {
    let mut ids = config.device_ids.clone().unwrap_or_default();
    if let Some(existing) = ids.get(account_id) {
        if is_valid_device_id(existing) {
            return existing.clone();
        }
    }
    let device_id = generate_device_id(&ids);
    ids.insert(account_id.to_string(), device_id.clone());
    config.device_ids = Some(ids);
    device_id
}

/// 取账号 did，仅在没有合法存量时落盘。读改写全程持存储锁，
/// 避免与后台 cycle 的整份配置写回互相覆盖；生成 did 不需要唤醒调度器。
pub fn ensure_device_id_persisted(account_id: &str) -> Result<String, String> {
    let _guard = lock_storage()?;
    ensure_device_id_persisted_at_path(&get_config_file_path(), account_id)
}

fn ensure_device_id_persisted_at_path(path: &Path, account_id: &str) -> Result<String, String> {
    let mut config = read_config_from_path(path)?.unwrap_or_default();
    if let Some(existing) = config
        .device_ids
        .as_ref()
        .and_then(|ids| ids.get(account_id))
    {
        if is_valid_device_id(existing) {
            return Ok(existing.clone());
        }
    }
    let device_id = ensure_device_id(&mut config, account_id);
    write_config_to_path(path, &config)?;
    Ok(device_id)
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

    let mut config = get_config_checked()?;
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let all_accounts = trae_account::list_accounts_checked()?;
    // 签到接口仅存在于 Trae CN（api.trae.cn），国际站账号直接跳过
    let cn_accounts: Vec<&TraeAccount> = all_accounts
        .iter()
        .filter(|account| trae_account::resolve_account_platform_kind(account).is_cn())
        .collect();
    let accounts: Vec<&TraeAccount> = dedup_checkin_accounts(&cn_accounts);
    if accounts.is_empty() {
        if force {
            add_log_record(TraeAutoCheckinLogRecord {
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
            let _ = app.emit("trae-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    let schedule_changed = ensure_account_schedules(&mut config, &accounts);
    if schedule_changed {
        save_cycle_state_without_wake(&config)?;
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
        let device_id = ensure_device_id(&mut config, &account.id);

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

    config.account_schedules = Some(new_schedules);
    save_cycle_state_without_wake(&config)?;

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let total_accounts = details
        .iter()
        .filter(|d| d.status != "inactive")
        .count();
    let overall_status = if total_accounts == 0 {
        "no_accounts"
    } else if failed_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    };

    add_log_record(TraeAutoCheckinLogRecord {
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
    })?;

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
            .filter(|account| trae_account::resolve_account_platform_kind(account).is_cn())
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

    fn make_temp_config_path(test_name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!(
                "cockpit-trae-auto-checkin-{}-{}-{}",
                test_name,
                std::process::id(),
                unique
            ))
            .join("trae_auto_checkin_config.json")
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
    fn test_ensure_device_id_generates_16_digit_and_reuses() {
        let mut config = TraeAutoCheckinConfig::default();
        let first = ensure_device_id(&mut config, "acc_1");
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|c| c.is_ascii_digit()));

        let second = ensure_device_id(&mut config, "acc_1");
        assert_eq!(first, second);

        // 非法存量值重新生成
        config.device_ids = Some(HashMap::from([(
            "acc_2".to_string(),
            "bad".to_string(),
        )]));
        let regenerated = ensure_device_id(&mut config, "acc_2");
        assert_eq!(regenerated.len(), 16);
        assert_ne!(regenerated, "bad");
    }

    #[test]
    fn test_ensure_device_id_does_not_reuse_taken_value() {
        let mut config = TraeAutoCheckinConfig::default();
        // 同一毫秒内连续为多账号生成，did 不能重复（接口按 did 全局判重）
        for index in 0..200 {
            let account_id = format!("acc_{}", index);
            let device_id = ensure_device_id(&mut config, &account_id);
            assert_eq!(device_id.len(), 16);
            assert!(device_id.chars().all(|c| c.is_ascii_digit()));
        }

        let ids = config.device_ids.unwrap();
        let unique: std::collections::HashSet<&String> = ids.values().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn test_ensure_device_id_persisted_reuses_stored_value() {
        let config_path = make_temp_config_path("device-id-persisted");

        let generated = ensure_device_id_persisted_at_path(&config_path, "acc_1").unwrap();
        assert_eq!(generated.len(), 16);
        assert!(generated.chars().all(|c| c.is_ascii_digit()));

        // 二次调用沿用已落盘的值，且不改动其他字段
        let stored = TraeAutoCheckinConfig {
            enabled: true,
            start_time: "07:00".to_string(),
            end_time: "09:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: Some(HashMap::from([("acc_1".to_string(), generated.clone())])),
        };
        write_config_to_path(&config_path, &stored).unwrap();

        assert_eq!(
            ensure_device_id_persisted_at_path(&config_path, "acc_1").unwrap(),
            generated
        );
        let reloaded = read_config_from_path(&config_path).unwrap().unwrap();
        assert_eq!(reloaded.start_time, "07:00");
        assert_eq!(reloaded.device_ids.unwrap().len(), 1);

        let temp_dir = config_path.parent().unwrap();
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn test_save_config_keeps_device_ids_written_by_other_caller() {
        let config_path = make_temp_config_path("device-id-merge");
        let mut disk_config = TraeAutoCheckinConfig::default();
        ensure_device_id(&mut disk_config, "acc_1");
        write_config_to_path(&config_path, &disk_config).unwrap();

        // 前端保存设置时可能持有不含 deviceIds 的旧快照
        let stale = TraeAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
            device_ids: None,
        };
        write_config_merging_disk(&config_path, &stale, false).unwrap();

        let reloaded = read_config_from_path(&config_path).unwrap().unwrap();
        assert!(reloaded.enabled);
        assert_eq!(reloaded.device_ids.unwrap().len(), 1);

        let temp_dir = config_path.parent().unwrap();
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn test_cycle_state_write_preserves_user_fields_from_disk() {
        let config_path = make_temp_config_path("cycle-state-merge");

        // 用户在 cycle 执行期间保存了新配置
        let user_config = TraeAutoCheckinConfig {
            enabled: false,
            start_time: "20:00".to_string(),
            end_time: "23:00".to_string(),
            ..TraeAutoCheckinConfig::default()
        };
        write_config_to_path(&config_path, &user_config).unwrap();

        // cycle 持旧快照写回调度状态：用户字段不被回滚，schedules/did 以 cycle 为准
        let mut cycle_snapshot = TraeAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            ..TraeAutoCheckinConfig::default()
        };
        ensure_device_id(&mut cycle_snapshot, "acc_1");
        cycle_snapshot.account_schedules = Some(HashMap::from([(
            "acc_1".to_string(),
            TraeAccountScheduleState {
                scheduled_date: "2026-10-03".to_string(),
                scheduled_minute: 400,
                last_checked_date: Some("2026-10-03".to_string()),
            },
        )]));
        write_config_merging_disk(&config_path, &cycle_snapshot, true).unwrap();

        let reloaded = read_config_from_path(&config_path).unwrap().unwrap();
        assert!(!reloaded.enabled);
        assert_eq!(reloaded.start_time, "20:00");
        assert_eq!(reloaded.end_time, "23:00");
        assert!(reloaded.account_schedules.unwrap().contains_key("acc_1"));
        assert_eq!(reloaded.device_ids.unwrap().len(), 1);

        let temp_dir = config_path.parent().unwrap();
        std::fs::remove_dir_all(temp_dir).unwrap();
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
