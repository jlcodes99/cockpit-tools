//! Commands for the public Qoder CN client. The account store and the OAuth
//! flow are shared with `cockpit-core`; this layer only exposes them.

use std::time::Instant;

use tauri::AppHandle;

use crate::models::qoder::{QoderAccount, QoderOAuthStartResponse};
use crate::modules::{logger, qoder_cn_account, qoder_oauth};

fn store() -> Result<qoder_cn_account::QoderCnAccountStore, String> {
    qoder_cn_account::QoderCnAccountStore::default_store()
}

#[tauri::command]
pub fn list_qoder_cn_accounts() -> Result<Vec<QoderAccount>, String> {
    store()?.list_accounts()
}

#[tauri::command]
pub fn delete_qoder_cn_account(account_id: String) -> Result<(), String> {
    store()?.remove_account(&account_id)
}

#[tauri::command]
pub fn delete_qoder_cn_accounts(account_ids: Vec<String>) -> Result<(), String> {
    store()?.remove_accounts(&account_ids)
}

#[tauri::command]
pub fn update_qoder_cn_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderAccount, String> {
    store()?.update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_qoder_cn_accounts_index_path() -> Result<String, String> {
    store()?.accounts_index_path_string()
}

#[tauri::command]
pub async fn qoder_cn_oauth_login_start() -> Result<QoderOAuthStartResponse, String> {
    let started_at = Instant::now();
    logger::log_info("[Qoder CN OAuth] start 命令触发");
    let result = qoder_oauth::start_cn_login().await;
    match &result {
        Ok(response) => logger::log_info(&format!(
            "[Qoder CN OAuth] start 命令完成: login_id={}, verification_uri_len={}, elapsed={}ms",
            response.login_id,
            response.verification_uri.len(),
            started_at.elapsed().as_millis()
        )),
        Err(error) => logger::log_warn(&format!(
            "[Qoder CN OAuth] start 命令失败: elapsed={}ms, error={error}",
            started_at.elapsed().as_millis()
        )),
    }
    result
}

#[tauri::command]
pub fn qoder_cn_oauth_login_peek() -> Option<QoderOAuthStartResponse> {
    qoder_oauth::peek_pending_cn_login()
}

#[tauri::command]
pub async fn qoder_cn_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder CN OAuth] complete 命令触发: login_id={login_id}"
    ));
    let account = match qoder_oauth::complete_login(&login_id).await {
        Ok(account) => account,
        Err(error) => {
            logger::log_warn(&format!(
                "[Qoder CN OAuth] complete 命令失败: login_id={login_id}, elapsed={}ms, error={error}",
                started_at.elapsed().as_millis()
            ));
            return Err(error);
        }
    };
    if let Err(error) = crate::modules::tray::update_tray_menu(&app) {
        logger::log_warn(&format!("[Qoder CN OAuth] 刷新托盘失败: {error}"));
    }
    logger::log_info(&format!(
        "[Qoder CN OAuth] complete 命令完成: login_id={login_id}, account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn qoder_cn_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    qoder_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_qoder_cn_token(
    app: AppHandle,
    account_id: String,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder CN Command] 手动刷新账号开始: account_id={account_id}"
    ));
    match qoder_oauth::refresh_account_from_openapi(&account_id).await {
        Ok(account) => {
            if let Err(error) = crate::modules::tray::update_tray_menu(&app) {
                logger::log_warn(&format!("[Qoder CN Command] 刷新托盘失败: {error}"));
            }
            logger::log_info(&format!(
                "[Qoder CN Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            if let Some(error) = &account.quota_query_last_error {
                return Err(error.clone());
            }
            Ok(account)
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Qoder CN Command] 刷新失败: account_id={account_id}, elapsed={}ms, error={error}",
                started_at.elapsed().as_millis()
            ));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_qoder_cn_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Qoder CN Command] 批量刷新开始");
    let refreshed = qoder_oauth::refresh_all_cn_accounts_from_openapi().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder CN Command] 批量刷新完成: refreshed={refreshed}, elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}
