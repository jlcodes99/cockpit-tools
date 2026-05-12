use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::kiro::{KiroAccount, KiroOAuthStartResponse};
use crate::modules::{kiro_account, kiro_oauth, logger};

async fn refresh_kiro_account_after_login(account: KiroAccount) -> KiroAccount {
    let account_id = account.id.clone();
    match kiro_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[Kiro OAuth] 登录后自动刷新失败: account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

#[tauri::command]
pub fn list_kiro_accounts() -> Result<Vec<KiroAccount>, String> {
    kiro_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_kiro_account(account_id: String) -> Result<(), String> {
    kiro_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_kiro_accounts(account_ids: Vec<String>) -> Result<(), String> {
    kiro_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_kiro_from_json(json_content: String) -> Result<Vec<KiroAccount>, String> {
    kiro_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_kiro_from_local(app: AppHandle) -> Result<Vec<KiroAccount>, String> {
    let payload = kiro_oauth::build_payload_from_local_files()?;
    let payload = kiro_oauth::enrich_payload_with_runtime_usage(payload).await;
    let account = kiro_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_kiro_accounts(account_ids: Vec<String>) -> Result<String, String> {
    kiro_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_kiro_token(app: AppHandle, account_id: String) -> Result<KiroAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Kiro Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    match kiro_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            if let Err(e) = kiro_account::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[QuotaAlert][Kiro] 预警检查失败: {}", e));
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Kiro Command] 手动刷新账号完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Kiro Command] 手动刷新账号失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_kiro_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Kiro Command] 手动批量刷新开始");

    let results = kiro_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);

    logger::log_info(&format!(
        "[Kiro Command] 手动批量刷新完成: success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));

    if success_count > 0 {
        if let Err(e) = kiro_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!("[QuotaAlert][Kiro] 全量刷新后预警检查失败: {}", e));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn kiro_oauth_login_start() -> Result<KiroOAuthStartResponse, String> {
    logger::log_info("Kiro OAuth start 命令触发");
    kiro_oauth::start_login().await
}

#[tauri::command]
pub async fn kiro_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<KiroAccount, String> {
    logger::log_info(&format!(
        "Kiro OAuth complete 命令触发: login_id={}",
        login_id
    ));
    let payload = kiro_oauth::complete_login(&login_id).await?;
    let account = kiro_account::upsert_account(payload)?;
    let account = refresh_kiro_account_after_login(account).await;
    logger::log_info(&format!(
        "Kiro OAuth complete 成功: account_id={}, email={}",
        account.id, account.email
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn kiro_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Kiro OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    kiro_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn kiro_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    kiro_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub async fn add_kiro_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<KiroAccount, String> {
    let payload = kiro_oauth::build_payload_from_token(&access_token).await?;
    let account = kiro_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_kiro_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<KiroAccount, String> {
    kiro_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_kiro_accounts_index_path() -> Result<String, String> {
    kiro_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_kiro_to_vscode(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Kiro Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = kiro_account::load_account(&account_id)
        .ok_or_else(|| format!("Kiro account not found: {}", account_id))?;

    if let Err(err) = crate::modules::kiro_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        logger::log_warn(&format!("更新 Kiro 默认实例绑定账号失败: {}", err));
    }
    crate::modules::provider_current_state::set_current_account_id(
        "kiro",
        Some(account_id.as_str()),
    )?;

    let launch_warning = match crate::commands::kiro_instance::kiro_start_instance(
        "__default__".to_string(),
    )
    .await
    {
        Ok(_) => None,
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 Kiro 失败") {
                logger::log_warn(&format!("Kiro 默认实例启动失败: {}", err));
                if err.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "kiro", "retry": { "kind": "default" } }),
                    );
                }
                Some(err)
            } else {
                return Err(err);
            }
        }
    };

    let _ = crate::modules::tray::update_tray_menu(&app);
    let notification_note = launch_warning
        .as_ref()
        .map(|_| "账号已切换，但 Kiro 启动失败");
    crate::modules::wecom_switch_notify::notify_switch(
        "Kiro",
        &account.id,
        &account.email,
        "manual",
        "tools.kiro.switch",
        notification_note,
    );

    if let Some(err) = launch_warning {
        logger::log_warn(&format!(
            "[Kiro Switch] 切号完成但启动失败: account_id={}, email={}, elapsed={}ms, error={}",
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 Kiro 启动失败: {}", err))
    } else {
        logger::log_info(&format!(
            "[Kiro Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.email))
    }
}
