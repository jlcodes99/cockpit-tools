use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use chrono::{Local, Timelike};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::models::workbuddy::WorkbuddyAccount;
use crate::modules::{codebuddy_cn_oauth, config, logger, workbuddy_account};

static IS_CHECKIN_RUNNING: AtomicBool = AtomicBool::new(false);

struct CheckinGuard;
impl Drop for CheckinGuard {
    fn drop(&mut self) {
        IS_CHECKIN_RUNNING.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAccountScheduleState {
    pub scheduled_date: String,
    pub scheduled_minute: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyAutoCheckinConfig {
    pub enabled: bool,
    pub start_time: String,
    pub end_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checked_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_schedules: Option<HashMap<String, WorkbuddyAccountScheduleState>>,
}

impl Default for WorkbuddyAutoCheckinConfig {
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
pub struct WorkbuddyAutoCheckinAccountDetail {
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
pub struct WorkbuddyAutoCheckinLogRecord {
    pub id: String,
    pub timestamp: String,
    pub date: String,
    pub duration_ms: u64,
    pub total_accounts: usize,
    pub success_count: usize,
    pub already_checked_count: usize,
    pub failed_count: usize,
    pub status: String,
    pub details: Vec<WorkbuddyAutoCheckinAccountDetail>,
}

fn get_config_file_path() -> PathBuf {
    config::get_shared_dir().join("workbuddy_auto_checkin_config.json")
}

fn get_logs_file_path() -> PathBuf {
    config::get_shared_dir().join("workbuddy_auto_checkin_logs.json")
}

pub fn get_config() -> WorkbuddyAutoCheckinConfig {
    let path = get_config_file_path();
    if !path.exists() {
        return WorkbuddyAutoCheckinConfig::default();
    }
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => WorkbuddyAutoCheckinConfig::default(),
    }
}

pub fn save_config(config: &WorkbuddyAutoCheckinConfig) -> Result<(), String> {
    let path = get_config_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 WorkBuddy 自动签到配置失败: {}", e))?;
    fs::write(&path, content).map_err(|e| format!("保存 WorkBuddy 自动签到配置失败: {}", e))
}

pub fn get_logs() -> Vec<WorkbuddyAutoCheckinLogRecord> {
    let path = get_logs_file_path();
    if !path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save_logs(logs: &[WorkbuddyAutoCheckinLogRecord]) -> Result<(), String> {
    let path = get_logs_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(logs)
        .map_err(|e| format!("序列化 WorkBuddy 自动签到日志失败: {}", e))?;
    fs::write(&path, content).map_err(|e| format!("保存 WorkBuddy 自动签到日志失败: {}", e))
}

fn add_log_record(record: WorkbuddyAutoCheckinLogRecord) {
    let mut logs = get_logs();
    if let Some(existing) = logs.iter_mut().find(|log| log.date == record.date) {
        let mut details: HashMap<String, WorkbuddyAutoCheckinAccountDetail> = existing
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
            let local_dt = ndt.and_local_timezone(Local).unwrap();
            local_dt.timestamp() >= cutoff
        } else {
            true
        }
    });

    let _ = save_logs(&logs);
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
    config: &mut WorkbuddyAutoCheckinConfig,
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
            if e.last_checked_date.as_deref() == Some(&today_str) {
                Some(today_str.clone())
            } else {
                None
            }
        });

        schedules.insert(
            account.id.clone(),
            WorkbuddyAccountScheduleState {
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
    schedules: &mut HashMap<String, WorkbuddyAccountScheduleState>,
    account_id: &str,
    today: &str,
    current_minute: i32,
) {
    let schedule =
        schedules
            .entry(account_id.to_string())
            .or_insert_with(|| WorkbuddyAccountScheduleState {
                scheduled_date: today.to_string(),
                scheduled_minute: current_minute,
                last_checked_date: None,
            });
    schedule.last_checked_date = Some(today.to_string());
}

pub async fn run_workbuddy_auto_checkin_cycle_if_needed(
    app: &AppHandle,
    force: bool,
) -> Result<String, String> {
    if IS_CHECKIN_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok("already_running".to_string());
    }
    let _guard = CheckinGuard;

    let mut config = get_config();
    if !config.enabled && !force {
        return Ok("disabled".to_string());
    }

    let accounts = workbuddy_account::list_accounts();
    if accounts.is_empty() {
        if force {
            add_log_record(WorkbuddyAutoCheckinLogRecord {
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
            });
            let _ = app.emit("workbuddy-auto-checkin-logs-changed", ());
        }
        return Ok("no_accounts".to_string());
    }

    let schedule_changed = ensure_account_schedules(&mut config, &accounts);
    if schedule_changed {
        let _ = save_config(&config);
        let _ = app.emit("workbuddy-auto-checkin-config-changed", ());
    }

    let today_str = get_today_date_string();
    let now = Local::now();
    let current_minute = (now.hour() * 60 + now.minute()) as i32;

    let target_accounts: Vec<&WorkbuddyAccount> = if force {
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
        "[WorkbuddyAutoCheckin] 开始处理后台签到，目标账号数: {}",
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

        match codebuddy_cn_oauth::get_checkin_status(
            &account.access_token,
            account.uid.as_deref(),
            account.enterprise_id.as_deref(),
            account.domain.as_deref(),
        )
        .await
        {
            Ok(status) if status.today_checked_in => {
                already_checked_count += 1;
                details.push(WorkbuddyAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "already_checked".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("今日已完成签到".to_string()),
                    credit: Some(serde_json::json!(status.daily_credit)),
                });
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
            }
            Ok(status) if !status.active => {
                details.push(WorkbuddyAutoCheckinAccountDetail {
                    account_id: account.id.clone(),
                    email: email_display,
                    status: "inactive".to_string(),
                    time: Some(account_checkin_time),
                    message: Some("签到活动未开启或不适用".to_string()),
                    credit: None,
                });
                mark_schedule_checked(&mut new_schedules, &account.id, &today_str, current_minute);
            }
            Ok(_) => match codebuddy_cn_oauth::perform_checkin(
                &account.access_token,
                account.uid.as_deref(),
                account.enterprise_id.as_deref(),
                account.domain.as_deref(),
            )
            .await
            {
                Ok(res) if res.success => {
                    success_count += 1;
                    did_checkin_any = true;
                    let streak = res
                        .streak_days
                        .and_then(|v| i32::try_from(v).ok())
                        .unwrap_or_else(|| account.checkin_streak.unwrap_or(0).saturating_add(1));
                    let reward = res.reward.clone().or_else(|| {
                        res.credit
                            .map(|credit| serde_json::json!({ "credit": credit }))
                    });
                    let _ = workbuddy_account::update_checkin_info(
                        &account.id,
                        Some(Local::now().timestamp()),
                        streak,
                        reward,
                    );

                    details.push(WorkbuddyAutoCheckinAccountDetail {
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
                    match codebuddy_cn_oauth::get_checkin_status(
                        &account.access_token,
                        account.uid.as_deref(),
                        account.enterprise_id.as_deref(),
                        account.domain.as_deref(),
                    )
                    .await
                    {
                        Ok(latest_status) if latest_status.today_checked_in => {
                            already_checked_count += 1;
                            details.push(WorkbuddyAutoCheckinAccountDetail {
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
                            details.push(WorkbuddyAutoCheckinAccountDetail {
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
                        "[WorkbuddyAutoCheckin] 账号 {} 自动签到异常: {}",
                        account.id, err
                    ));
                    retry_needed = true;
                    failed_count += 1;
                    details.push(WorkbuddyAutoCheckinAccountDetail {
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
                    "[WorkbuddyAutoCheckin] 账号 {} 签到状态检查异常: {}",
                    account.id, err
                ));
                retry_needed = true;
                failed_count += 1;
                details.push(WorkbuddyAutoCheckinAccountDetail {
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
    let _ = save_config(&config);

    let duration_ms = start_instant.elapsed().as_millis() as u64;
    let overall_status = if failed_count == 0 {
        "success"
    } else if success_count > 0 || already_checked_count > 0 {
        "partial"
    } else {
        "failed"
    };

    add_log_record(WorkbuddyAutoCheckinLogRecord {
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
    });

    if did_checkin_any {
        let _ = crate::modules::tray::update_tray_menu(app);
    }

    let _ = app.emit("workbuddy-auto-checkin-logs-changed", ());
    let _ = app.emit("workbuddy-auto-checkin-config-changed", ());

    if retry_needed {
        Ok("retry".to_string())
    } else {
        Ok("completed".to_string())
    }
}

pub fn start_auto_checkin_scheduler(app: AppHandle) {
    tokio::spawn(async move {
        logger::log_info("[WorkbuddyAutoCheckin] 后台自动签到调度服务已启动");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
            if let Err(e) = run_workbuddy_auto_checkin_cycle_if_needed(&app, false).await {
                logger::log_warn(&format!("[WorkbuddyAutoCheckin] 调度异常: {}", e));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut config = WorkbuddyAutoCheckinConfig {
            enabled: true,
            start_time: "06:00".to_string(),
            end_time: "12:00".to_string(),
            last_checked_date: None,
            account_schedules: None,
        };

        let accounts = vec![
            WorkbuddyAccount {
                id: "acc_1".to_string(),
                email: "acc1@example.com".to_string(),
                uid: None,
                nickname: None,
                enterprise_id: None,
                enterprise_name: None,
                tags: None,
                access_token: "token1".to_string(),
                refresh_token: None,
                token_type: None,
                expires_at: None,
                domain: None,
                plan_type: None,
                dosage_notify_code: None,
                dosage_notify_zh: None,
                dosage_notify_en: None,
                payment_type: None,
                quota_raw: None,
                auth_raw: None,
                profile_raw: None,
                usage_raw: None,
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
            },
            WorkbuddyAccount {
                id: "acc_2".to_string(),
                email: "acc2@example.com".to_string(),
                uid: None,
                nickname: None,
                enterprise_id: None,
                enterprise_name: None,
                tags: None,
                access_token: "token2".to_string(),
                refresh_token: None,
                token_type: None,
                expires_at: None,
                domain: None,
                plan_type: None,
                dosage_notify_code: None,
                dosage_notify_zh: None,
                dosage_notify_en: None,
                payment_type: None,
                quota_raw: None,
                auth_raw: None,
                profile_raw: None,
                usage_raw: None,
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
            },
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
        let config = WorkbuddyAutoCheckinConfig {
            enabled: true,
            start_time: "08:00".to_string(),
            end_time: "10:00".to_string(),
            last_checked_date: Some("2026-07-28".to_string()),
            account_schedules: None,
        };

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"startTime\":\"08:00\""));

        let deserialized: WorkbuddyAutoCheckinConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.start_time, "08:00");
    }
}
