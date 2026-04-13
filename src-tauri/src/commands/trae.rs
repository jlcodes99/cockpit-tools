use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::trae::{TraeAccount, TraeOAuthStartResponse};
use crate::modules::{logger, trae_account, trae_oauth};

#[tauri::command]
pub fn list_trae_accounts() -> Result<Vec<TraeAccount>, String> {
    trae_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_trae_account(account_id: String) -> Result<(), String> {
    trae_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_trae_accounts(account_ids: Vec<String>) -> Result<(), String> {
    trae_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_trae_from_json(json_content: String) -> Result<Vec<TraeAccount>, String> {
    trae_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_trae_from_local(app: AppHandle) -> Result<Vec<TraeAccount>, String> {
    match trae_account::import_from_local()? {
        Some(mut account) => {
            match trae_account::refresh_account_async(account.id.as_str()).await {
                Ok(refreshed) => account = refreshed,
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Trae Import] 本地导入后自动刷新配额失败: {}",
                        err
                    ));
                }
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err("未找到本地 Trae 登录信息".to_string()),
    }
}

#[tauri::command]
pub async fn trae_oauth_login_start() -> Result<TraeOAuthStartResponse, String> {
    logger::log_info("[Trae OAuth] start 命令触发");
    trae_oauth::start_login().await
}

#[tauri::command]
pub async fn trae_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<TraeAccount, String> {
    logger::log_info(&format!(
        "[Trae OAuth] complete 命令触发: login_id={}",
        login_id
    ));

    let payload = trae_oauth::complete_login(login_id.as_str()).await?;
    let mut account = trae_account::upsert_account(payload)?;

    match trae_account::refresh_account_async(account.id.as_str()).await {
        Ok(refreshed) => account = refreshed,
        Err(err) => {
            logger::log_warn(&format!("[Trae OAuth] 登录后自动刷新配额失败: {}", err));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn trae_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Trae OAuth] cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    trae_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn trae_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    trae_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub fn export_trae_accounts(account_ids: Vec<String>) -> Result<String, String> {
    trae_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_trae_token(app: AppHandle, account_id: String) -> Result<TraeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    match trae_account::refresh_account_async(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Trae Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_trae_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Trae Command] 批量刷新开始");

    let results = trae_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, result)| result.is_ok()).count();
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Trae Command] 批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub fn add_trae_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<TraeAccount, String> {
    let payload = crate::models::trae::TraeImportPayload {
        email: "unknown".to_string(),
        user_id: None,
        nickname: None,
        access_token,
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
    };
    let account = trae_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_trae_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<TraeAccount, String> {
    trae_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_trae_accounts_index_path() -> Result<String, String> {
    trae_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_trae_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = trae_account::load_account(&account_id)
        .ok_or_else(|| format!("Trae account not found: {}", account_id))?;

    if let Err(err) = crate::modules::process::close_trae(20) {
        logger::log_warn(&format!(
            "[Trae Switch] 关闭 Trae 旧进程失败，将继续注入: {}",
            err
        ));
    }

    trae_account::inject_to_trae(&account_id)?;

    if let Err(err) = crate::modules::trae_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        logger::log_warn(&format!("更新 Trae 默认实例绑定账号失败: {}", err));
    }

    let launch_warning = match crate::commands::trae_instance::trae_start_instance(
        "__default__".to_string(),
    )
    .await
    {
        Ok(_) => None,
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 Trae 失败") {
                logger::log_warn(&format!("Trae 默认实例启动失败: {}", err));
                if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "trae", "retry": { "kind": "default" } }),
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
            "[Trae Switch] 切号完成但启动失败: account_id={}, email={}, elapsed={}ms, error={}",
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 Trae 启动失败: {}", err))
    } else {
        logger::log_info(&format!(
            "[Trae Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.email))
    }
}
