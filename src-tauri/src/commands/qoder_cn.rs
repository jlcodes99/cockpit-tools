use std::time::Instant;
use tauri::AppHandle;

use crate::models::qoder_cn::{QoderCnAccount, QoderCnOAuthStartResponse};
use crate::modules::{logger, qoder_cn_account, qoder_cn_oauth};

#[tauri::command]
pub fn list_qoder_cn_accounts() -> Result<Vec<QoderCnAccount>, String> {
    qoder_cn_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_qoder_cn_account(account_id: String) -> Result<(), String> {
    qoder_cn_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_qoder_cn_accounts(account_ids: Vec<String>) -> Result<(), String> {
    qoder_cn_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_qoder_cn_from_json(json_content: String) -> Result<Vec<QoderCnAccount>, String> {
    qoder_cn_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn import_qoder_cn_from_local(app: AppHandle) -> Result<Vec<QoderCnAccount>, String> {
    match qoder_cn_account::import_from_local()? {
        Some(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err("未找到本地 Qoder CN 登录信息".to_string()),
    }
}

#[tauri::command]
pub fn export_qoder_cn_accounts(account_ids: Vec<String>) -> Result<String, String> {
    qoder_cn_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn qoder_cn_oauth_login_start() -> Result<QoderCnOAuthStartResponse, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderCN OAuth] start 命令触发");
    let result = qoder_cn_oauth::start_login().await;
    match &result {
        Ok(response) => logger::log_info(&format!(
            "[QoderCN OAuth] start 完成: login_id={}, uri_len={}, elapsed={}ms",
            response.login_id,
            response.verification_uri.len(),
            started_at.elapsed().as_millis()
        )),
        Err(err) => logger::log_warn(&format!(
            "[QoderCN OAuth] start 失败: elapsed={}ms, error={}",
            started_at.elapsed().as_millis(),
            err
        )),
    }
    result
}

#[tauri::command]
pub fn qoder_cn_oauth_login_peek() -> Option<QoderCnOAuthStartResponse> {
    let pending = qoder_cn_oauth::peek_pending_login();
    if let Some(state) = pending.as_ref() {
        logger::log_info(&format!(
            "[QoderCN OAuth] peek 命中: login_id={}",
            state.login_id
        ));
    } else {
        logger::log_info("[QoderCN OAuth] peek 未命中");
    }
    pending
}

#[tauri::command]
pub async fn qoder_cn_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<QoderCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderCN OAuth] complete 触发: login_id={}",
        login_id
    ));
    let account = match qoder_cn_oauth::complete_login(&login_id).await {
        Ok(account) => account,
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderCN OAuth] complete 失败: login_id={}, elapsed={}ms, error={}",
                login_id,
                started_at.elapsed().as_millis(),
                err
            ));
            return Err(err);
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[QoderCN OAuth] complete 完成: login_id={}, account_id={}, elapsed={}ms",
        login_id,
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn qoder_cn_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[QoderCN OAuth] cancel 触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    qoder_cn_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_qoder_cn_token(
    app: AppHandle,
    account_id: String,
) -> Result<QoderCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderCN Command] 手动刷新开始: account_id={}",
        account_id
    ));
    match qoder_cn_oauth::refresh_account_from_openapi(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[QoderCN Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[QoderCN Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_qoder_cn_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderCN Command] 批量刷新开始");
    let refreshed = qoder_cn_oauth::refresh_all_accounts_from_openapi().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[QoderCN Command] 批量刷新完成: refreshed={}, elapsed={}ms",
        refreshed,
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}

#[tauri::command]
pub async fn switch_qoder_cn_account(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[QoderCN Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let result = qoder_cn_account::switch_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[QoderCN Switch] 切换完成: account_id={}, elapsed={}ms",
        account_id,
        started_at.elapsed().as_millis()
    ));
    Ok(result)
}

#[tauri::command]
pub async fn add_qoder_cn_account_with_token(
    app: AppHandle,
    token: String,
) -> Result<QoderCnAccount, String> {
    let started_at = Instant::now();
    logger::log_info("[QoderCN Command] Token 导入开始");

    let account = qoder_cn_oauth::build_account_from_token(&token).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[QoderCN Command] Token 导入完成: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn update_qoder_cn_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderCnAccount, String> {
    qoder_cn_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_qoder_cn_accounts_index_path() -> Result<String, String> {
    qoder_cn_account::accounts_index_path_string()
}
