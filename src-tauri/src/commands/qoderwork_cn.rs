use std::time::Instant;
use tauri::AppHandle;

use crate::models::qoderwork_cn::{QoderworkCnAccount, QoderworkCnOAuthStartResponse};
use crate::modules::{logger, qoderwork_cn_account, qoderwork_cn_oauth};

#[tauri::command]
pub fn list_qoderwork_cn_accounts() -> Result<Vec<QoderworkCnAccount>, String> {
    qoderwork_cn_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_qoderwork_cn_account(account_id: String) -> Result<(), String> {
    qoderwork_cn_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_qoderwork_cn_accounts(account_ids: Vec<String>) -> Result<(), String> {
    qoderwork_cn_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_qoderwork_cn_from_json(json_content: String) -> Result<Vec<QoderworkCnAccount>, String> {
    qoderwork_cn_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn import_qoderwork_cn_from_local(app: AppHandle) -> Result<Vec<QoderworkCnAccount>, String> {
    match qoderwork_cn_account::import_from_local()? {
        Some(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err("未找到本地 QoderWork CN 登录信息".to_string()),
    }
}

#[tauri::command]
pub fn export_qoderwork_cn_accounts(account_ids: Vec<String>) -> Result<String, String> {
    qoderwork_cn_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn qoderwork_cn_oauth_login_start() -> Result<QoderworkCnOAuthStartResponse, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderWorkCN OAuth] start 命令触发");
    let result = qoderwork_cn_oauth::start_login().await;
    match &result {
        Ok(response) => logger::log_info(&format!(
            "[QoderWorkCN OAuth] start 完成: login_id={}, uri_len={}, elapsed={}ms",
            response.login_id,
            response.verification_uri.len(),
            started_at.elapsed().as_millis()
        )),
        Err(err) => logger::log_warn(&format!(
            "[QoderWorkCN OAuth] start 失败: elapsed={}ms, error={}",
            started_at.elapsed().as_millis(),
            err
        )),
    }
    result
}

#[tauri::command]
pub fn qoderwork_cn_oauth_login_peek() -> Option<QoderworkCnOAuthStartResponse> {
    let pending = qoderwork_cn_oauth::peek_pending_login();
    if let Some(state) = pending.as_ref() {
        logger::log_info(&format!(
            "[QoderWorkCN OAuth] peek 命中: login_id={}",
            state.login_id
        ));
    } else {
        logger::log_info("[QoderWorkCN OAuth] peek 未命中");
    }
    pending
}

#[tauri::command]
pub async fn qoderwork_cn_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<QoderworkCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderWorkCN OAuth] complete 触发: login_id={}",
        login_id
    ));
    let account = match qoderwork_cn_oauth::complete_login(&login_id).await {
        Ok(account) => account,
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderWorkCN OAuth] complete 失败: login_id={}, elapsed={}ms, error={}",
                login_id,
                started_at.elapsed().as_millis(),
                err
            ));
            return Err(err);
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[QoderWorkCN OAuth] complete 完成: login_id={}, account_id={}, elapsed={}ms",
        login_id,
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn qoderwork_cn_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[QoderWorkCN OAuth] cancel 触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    qoderwork_cn_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_qoderwork_cn_token(
    app: AppHandle,
    account_id: String,
) -> Result<QoderworkCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderWorkCN Command] 手动刷新开始: account_id={}",
        account_id
    ));
    match qoderwork_cn_oauth::refresh_account_from_openapi(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[QoderWorkCN Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderWorkCN Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_qoderwork_cn_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderWorkCN Command] 批量刷新开始");
    let refreshed = qoderwork_cn_oauth::refresh_all_accounts_from_openapi().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[QoderWorkCN Command] 批量刷新完成: refreshed={}, elapsed={}ms",
        refreshed,
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}

#[tauri::command]
pub async fn switch_qoderwork_cn_account(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderWorkCN Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result = qoderwork_cn_account::switch_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[QoderWorkCN Switch] 切换完成: account_id={}, elapsed={}ms",
        account_id,
        started_at.elapsed().as_millis()
    ));
    Ok(result)
}

#[tauri::command]
pub async fn add_qoderwork_cn_account_with_token(
    app: AppHandle,
    token: String,
) -> Result<QoderworkCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderWorkCN Command] Token 导入开始");

    let account = qoderwork_cn_oauth::build_account_from_token(&token).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[QoderWorkCN Command] Token 导入完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn update_qoderwork_cn_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderworkCnAccount, String> {
    qoderwork_cn_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_qoderwork_cn_accounts_index_path() -> Result<String, String> {
    qoderwork_cn_account::accounts_index_path_string()
}
