use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::zed::{ZedAccount, ZedOAuthStartResponse, ZedRuntimeStatus};
use crate::modules::{logger, zed_account, zed_instance, zed_oauth};

#[tauri::command]
pub fn list_zed_accounts() -> Result<Vec<ZedAccount>, String> {
    zed_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_zed_account(app: AppHandle, account_id: String) -> Result<(), String> {
    zed_account::remove_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_zed_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    zed_account::remove_accounts(&account_ids)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn import_zed_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<ZedAccount>, String> {
    let accounts = zed_account::import_from_json(&json_content)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_zed_from_local(app: AppHandle) -> Result<Vec<ZedAccount>, String> {
    let account = zed_account::import_from_local().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_zed_accounts(account_ids: Vec<String>) -> Result<String, String> {
    zed_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_zed_token(app: AppHandle, account_id: String) -> Result<ZedAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Zed Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    let account = zed_account::refresh_account(&account_id).await?;
    if let Err(err) = zed_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[QuotaAlert][Zed] 刷新后预警检查失败: {}", err));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zed Command] 手动刷新账号完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_zed_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Zed Command] 批量刷新开始");
    let refreshed = zed_account::refresh_all_accounts().await?;
    if !refreshed.is_empty() {
        if let Err(err) = zed_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][Zed] 全量刷新后预警检查失败: {}",
                err
            ));
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zed Command] 批量刷新完成: refreshed={}, elapsed={}ms",
        refreshed.len(),
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed.len() as i32)
}

#[tauri::command]
pub fn update_zed_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<ZedAccount, String> {
    zed_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub async fn zed_oauth_login_start() -> Result<ZedOAuthStartResponse, String> {
    let started_at = Instant::now();
    logger::log_info("[Zed OAuth] start 命令触发");
    let result = zed_oauth::start_login().await;
    match &result {
        Ok(response) => logger::log_info(&format!(
            "[Zed OAuth] start 命令完成: login_id={}, elapsed={}ms",
            response.login_id,
            started_at.elapsed().as_millis()
        )),
        Err(err) => logger::log_warn(&format!(
            "[Zed OAuth] start 命令失败: elapsed={}ms, error={}",
            started_at.elapsed().as_millis(),
            err
        )),
    }
    result
}

#[tauri::command]
pub fn zed_oauth_login_peek() -> Option<ZedOAuthStartResponse> {
    zed_oauth::peek_pending_login()
}

#[tauri::command]
pub async fn zed_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<ZedAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Zed OAuth] complete 命令触发: login_id={}",
        login_id
    ));
    let account = zed_oauth::complete_login(&login_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zed OAuth] complete 命令完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn zed_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    zed_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn zed_oauth_submit_callback_url(login_id: String, callback_url: String) -> Result<(), String> {
    zed_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub async fn inject_zed_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Zed Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = zed_account::inject_account(&account_id)?;
    let restart_result = zed_instance::restart_default_session();
    let _ = crate::modules::tray::update_tray_menu(&app);

    match restart_result {
        Ok(_) => {
            logger::log_info(&format!(
                "[Zed Switch] 切号成功: account_id={}, github_login={}, elapsed={}ms",
                account.id,
                account.github_login,
                started_at.elapsed().as_millis()
            ));
            crate::modules::wecom_switch_notify::notify_switch(
                "Zed",
                &account.id,
                &account.github_login,
                "manual",
                "tools.zed.switch",
                None,
            );
            Ok(format!("切换完成: {}", account.github_login))
        }
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 Zed 失败") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "zed",
                        "retry": { "kind": "switchAccount", "accountId": account_id }
                    }),
                );
                logger::log_warn(&format!(
                    "[Zed Switch] 切号完成但重启失败: account_id={}, err={}",
                    account.id, err
                ));
                crate::modules::wecom_switch_notify::notify_switch(
                    "Zed",
                    &account.id,
                    &account.github_login,
                    "manual",
                    "tools.zed.switch",
                    Some("账号已切换，但 Zed 重启失败"),
                );
                return Ok(format!("切换完成，但 Zed 重启失败: {}", err));
            }
            crate::modules::wecom_switch_notify::notify_switch(
                "Zed",
                &account.id,
                &account.github_login,
                "manual",
                "tools.zed.switch",
                Some("账号已切换，但 Zed 重启失败"),
            );
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn zed_logout_current_account(app: AppHandle) -> Result<String, String> {
    zed_account::clear_current_runtime_account()?;
    let restart_result = zed_instance::restart_default_session();
    let _ = crate::modules::tray::update_tray_menu(&app);

    match restart_result {
        Ok(_) => Ok("已退出当前 Zed 账号".to_string()),
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 Zed 失败") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "zed",
                        "retry": { "kind": "default" }
                    }),
                );
                return Ok(format!("已退出当前 Zed 账号，但 Zed 重启失败: {}", err));
            }
            Err(err)
        }
    }
}

#[tauri::command]
pub fn zed_get_runtime_status() -> Result<ZedRuntimeStatus, String> {
    Ok(zed_instance::get_runtime_status())
}

#[tauri::command]
pub fn zed_start_default_session() -> Result<ZedRuntimeStatus, String> {
    zed_instance::start_default_session()
}

#[tauri::command]
pub fn zed_stop_default_session() -> Result<ZedRuntimeStatus, String> {
    zed_instance::stop_default_session()
}

#[tauri::command]
pub fn zed_restart_default_session() -> Result<ZedRuntimeStatus, String> {
    zed_instance::restart_default_session()
}

#[tauri::command]
pub fn zed_focus_default_session() -> Result<ZedRuntimeStatus, String> {
    zed_instance::focus_default_session()
}
