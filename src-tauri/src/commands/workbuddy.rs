use std::time::Instant;

use crate::modules::codebuddy_cn_oauth;
use tauri::{AppHandle, Emitter};

use crate::models::workbuddy::{WorkbuddyAccount, WorkbuddyOAuthStartResponse};
use crate::modules::{logger, workbuddy_account, workbuddy_oauth};

async fn refresh_workbuddy_account_after_login(account: WorkbuddyAccount) -> WorkbuddyAccount {
    let account_id = account.id.clone();
    match workbuddy_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[WorkBuddy OAuth] 登录后刷新失败，保留原账号信息：account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

#[tauri::command]
pub fn list_workbuddy_accounts() -> Result<Vec<WorkbuddyAccount>, String> {
    workbuddy_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_workbuddy_account(account_id: String) -> Result<(), String> {
    workbuddy_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_workbuddy_accounts(account_ids: Vec<String>) -> Result<(), String> {
    workbuddy_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_workbuddy_from_json(json_content: String) -> Result<Vec<WorkbuddyAccount>, String> {
    workbuddy_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_workbuddy_from_local(app: AppHandle) -> Result<Vec<WorkbuddyAccount>, String> {
    let mut local_payload = match workbuddy_account::import_payload_from_local()? {
        Some(payload) => payload,
        None => return Err("未在本机 WorkBuddy 客户端中找到登录信息".to_string()),
    };

    match workbuddy_oauth::build_payload_from_token(&local_payload.access_token).await {
        Ok(mut payload) => {
            if payload.uid.is_none() {
                payload.uid = local_payload.uid.clone();
            }
            if payload.nickname.is_none() {
                payload.nickname = local_payload.nickname.clone();
            }
            if payload.refresh_token.is_none() {
                payload.refresh_token = local_payload.refresh_token.clone();
            }
            if payload.domain.is_none() {
                payload.domain = local_payload.domain.clone();
            }
            if payload.token_type.is_none() {
                payload.token_type = local_payload.token_type.clone();
            }
            if payload.expires_at.is_none() {
                payload.expires_at = local_payload.expires_at;
            }
            if payload.auth_raw.is_none() {
                payload.auth_raw = local_payload.auth_raw.clone();
            }
            if payload.profile_raw.is_none() {
                payload.profile_raw = local_payload.profile_raw.clone();
            }
            if payload.email.trim().is_empty() || payload.email == "unknown" {
                payload.email = local_payload.email.clone();
            }
            local_payload = payload;
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[WorkBuddy Import Local] 拉取账号资料失败，将保留本地导入结果：{}",
                err
            ));
        }
    }

    let mut account = workbuddy_account::upsert_account(local_payload.clone())?;

    for existing in workbuddy_account::list_accounts() {
        if existing.id == account.id {
            continue;
        }
        if existing.access_token != account.access_token {
            continue;
        }
        let is_placeholder = existing.email.trim().eq_ignore_ascii_case("unknown")
            || existing.email.trim().is_empty()
            || existing
                .uid
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
        if is_placeholder {
            if let Err(err) = workbuddy_account::remove_account(&existing.id) {
                logger::log_warn(&format!(
                    "[WorkBuddy Import Local] 清理占位账号失败：id={}, error={}",
                    existing.id, err
                ));
            }
        }
    }

    account = refresh_workbuddy_account_after_login(account).await;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_workbuddy_accounts(account_ids: Vec<String>) -> Result<String, String> {
    workbuddy_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_workbuddy_token(
    app: AppHandle,
    account_id: String,
) -> Result<WorkbuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Command] 手动刷新账号开始：account_id={}",
        account_id
    ));

    match workbuddy_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            if let Err(e) = workbuddy_account::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[QuotaAlert][WorkBuddy] 预警检查失败：{}", e));
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[WorkBuddy Command] 手动刷新账号完成：account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[WorkBuddy Command] 手动刷新账号失败：account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_workbuddy_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[WorkBuddy Command] 手动批量刷新开始");

    let results = workbuddy_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);

    logger::log_info(&format!(
        "[WorkBuddy Command] 手动批量刷新完成：success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));

    if success_count > 0 {
        if let Err(e) = workbuddy_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][WorkBuddy] 全量刷新后预警检查失败：{}",
                e
            ));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn workbuddy_oauth_login_start() -> Result<WorkbuddyOAuthStartResponse, String> {
    logger::log_info("WorkBuddy OAuth start 命令触发");
    workbuddy_oauth::start_login().await
}

#[tauri::command]
pub async fn workbuddy_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<WorkbuddyAccount, String> {
    logger::log_info(&format!(
        "WorkBuddy OAuth complete 命令触发：login_id={}",
        login_id
    ));

    let result: Result<WorkbuddyAccount, String> = async {
        let payload = workbuddy_oauth::complete_login(&login_id).await?;
        let mut account = workbuddy_account::upsert_account(payload)?;
        account = refresh_workbuddy_account_after_login(account).await;
        Ok(account)
    }
    .await;

    if let Err(err) = workbuddy_oauth::clear_pending_oauth_login(&login_id) {
        logger::log_warn(&format!(
            "[WorkBuddy OAuth] 清理待处理登录状态失败：login_id={}, error={}",
            login_id, err
        ));
    }

    let account = result?;
    if let Err(err) = workbuddy_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!(
            "[QuotaAlert][WorkBuddy] 登录后预警检查失败：{}",
            err
        ));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "WorkBuddy OAuth complete 成功：account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn workbuddy_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "WorkBuddy OAuth cancel 命令触发：login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    workbuddy_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn add_workbuddy_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<WorkbuddyAccount, String> {
    let payload = workbuddy_oauth::build_payload_from_token(&access_token).await?;
    let account = workbuddy_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_workbuddy_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WorkbuddyAccount, String> {
    workbuddy_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_workbuddy_accounts_index_path() -> Result<String, String> {
    workbuddy_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_workbuddy_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Switch] 开始切换账号：account_id={}",
        account_id
    ));

    let account = workbuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("WorkBuddy account not found: {}", account_id))?;

    if let Err(err) = crate::modules::workbuddy_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        logger::log_warn(&format!("更新 WorkBuddy 默认实例绑定账号失败：{}", err));
    }
    crate::modules::provider_current_state::set_current_account_id(
        "workbuddy",
        Some(account_id.as_str()),
    )?;

    let launch_warning = match crate::commands::workbuddy_instance::workbuddy_start_instance(
        "__default__".to_string(),
    )
    .await
    {
        Ok(_) => None,
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 WorkBuddy 失败") {
                logger::log_warn(&format!("WorkBuddy 默认实例启动失败：{}", err));
                // 保持既有行为：即使应用路径异常，认证切换仍然落盘。由于此时无法
                // 确认官方进程已退出，不在这里执行可能较慢的会话目录合并。
                workbuddy_account::write_account_to_default_client(&account)?;
                if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "workbuddy", "retry": { "kind": "default" } }),
                    );
                }
                Some(err)
            } else {
                return Err(err);
            }
        }
    };

    let _ = crate::modules::tray::update_tray_menu(&app);

    if let Some(err) = launch_warning {
        logger::log_warn(&format!(
            "[WorkBuddy Switch] 切号完成但启动失败：account_id={}, email={}, elapsed={}ms, error={}",
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 WorkBuddy 启动失败：{}", err))
    } else {
        logger::log_info(&format!(
            "[WorkBuddy Switch] 切号成功：account_id={}, email={}, elapsed={}ms",
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成：{}", account.email))
    }
}

#[tauri::command]
pub async fn sync_workbuddy_to_codebuddy_cn(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[WorkBuddy -> CodeBuddy CN] 开始同步账号");

    let synced_count = workbuddy_account::sync_accounts_to_codebuddy_cn()?;

    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[WorkBuddy -> CodeBuddy CN] 同步完成: count={}, elapsed={}ms",
        synced_count,
        started_at.elapsed().as_millis()
    ));

    Ok(synced_count as i32)
}

#[tauri::command]
pub async fn get_checkin_status_workbuddy(
    account_id: String,
) -> Result<crate::modules::codebuddy_cn_oauth::CheckinStatusResponse, String> {
    let account = workbuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    codebuddy_cn_oauth::get_checkin_status(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn checkin_workbuddy(
    app: AppHandle,
    account_id: String,
) -> Result<crate::modules::codebuddy_cn_oauth::CheckinResponse, String> {
    use std::time::Instant;

    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy Checkin] 执行签到开始: account_id={}",
        account_id
    ));

    let account = workbuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;

    let response = codebuddy_cn_oauth::perform_checkin(
        &account.access_token,
        account.uid.as_deref(),
        account.enterprise_id.as_deref(),
        account.domain.as_deref(),
    )
    .await?;

    if response.success {
        let now = chrono::Utc::now().timestamp();
        let streak = response
            .streak_days
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_else(|| account.checkin_streak.unwrap_or(0).saturating_add(1));
        let reward = response.reward.clone().or_else(|| {
            response
                .credit
                .map(|credit| serde_json::json!({ "credit": credit }))
        });
        workbuddy_account::update_checkin_info(&account_id, Some(now), streak, reward.clone())
            .map_err(|e| {
                logger::log_warn(&format!(
                    "[WorkBuddy Checkin] 更新签到信息失败: account_id={}, error={}",
                    account_id, e
                ));
                format!("签到成功但更新状态失败: {}", e)
            })?;

        let _ = crate::modules::tray::update_tray_menu(&app);
        let _ = app.emit(
            "workbuddy:checkin_completed",
            serde_json::json!({
                "accountId": account_id,
                "success": true,
                "reward": reward,
                "credit": response.credit,
                "streakDays": response.streak_days,
                "isStreakDay": response.is_streak_day,
                "streak": streak,
            }),
        );
    }

    logger::log_info(&format!(
        "[WorkBuddy Checkin] 执行签到完成: account_id={}, success={}, elapsed={}ms",
        account_id,
        response.success,
        started_at.elapsed().as_millis()
    ));

    Ok(response)
}
