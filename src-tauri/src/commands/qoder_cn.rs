//! Commands for the public Qoder CN client. The account store and the OAuth
//! flow are shared with `cockpit-core`; this layer only exposes them.

use std::time::Instant;

use tauri::AppHandle;

use crate::models::qoder::{QoderAccount, QoderOAuthStartResponse};
use crate::modules::{logger, qoder_cn_account, qoder_native, qoder_oauth};

fn store() -> Result<qoder_cn_account::QoderCnAccountStore, String> {
    qoder_cn_account::QoderCnAccountStore::default_store()
}

/// The CN commands only ever act on the public CN client; an explicit region is
/// accepted for frontend compatibility and rejected when it names the other one.
fn ensure_cn_region(region: Option<&str>) -> Result<(), String> {
    match region.unwrap_or("cn") {
        "cn" => Ok(()),
        _ => Err("QODER_PLATFORM_MISMATCH".into()),
    }
}

#[tauri::command]
pub fn list_qoder_cn_accounts() -> Result<Vec<QoderAccount>, String> {
    store()?.list_accounts()
}

#[tauri::command]
pub async fn import_qoder_cn_native(
    region: Option<String>,
    client: String,
) -> Result<Vec<QoderAccount>, String> {
    ensure_cn_region(region.as_deref())?;
    let client = qoder_native::NativeClient::parse(&client)?;
    tauri::async_runtime::spawn_blocking(move || {
        qoder_native::import(client).map(|item| vec![item])
    })
    .await
    .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())?
}

#[tauri::command]
pub async fn import_qoder_cn_from_local(app: AppHandle) -> Result<Vec<QoderAccount>, String> {
    let found = tauri::async_runtime::spawn_blocking(qoder_native::import_from_local)
        .await
        .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())??;
    if !found {
        return Err("未找到本地 Qoder CN 登录信息".to_string());
    }
    if let Err(error) = crate::modules::tray::update_tray_menu(&app) {
        logger::log_warn(&format!("[Qoder CN Native] 刷新托盘失败: {error}"));
    }
    store()?.list_accounts()
}

#[tauri::command]
pub fn import_qoder_cn_from_json(
    json_content: String,
    region: Option<String>,
) -> Result<Vec<QoderAccount>, String> {
    ensure_cn_region(region.as_deref())?;
    let mut value: serde_json::Value =
        serde_json::from_str(&json_content).map_err(|error| format!("Qoder JSON: {error}"))?;
    qoder_native::prepare_cn_import(&mut value)?;
    let content = serde_json::to_string(&value).map_err(|error| error.to_string())?;
    store()?.import_json(&content)
}

#[tauri::command]
pub fn export_qoder_cn_accounts(account_ids: Vec<String>) -> Result<String, String> {
    store()?.export_accounts(&account_ids)
}

#[tauri::command]
pub async fn inject_qoder_cn_native(account_id: String, client: String) -> Result<String, String> {
    let target = qoder_native::NativeClient::parse(&client)?;
    qoder_oauth::prepare_cn_account_for_switch(&account_id).await?;
    let id = account_id.clone();
    let started_at = Instant::now();
    tauri::async_runtime::spawn_blocking(move || qoder_native::switch(&id, target))
        .await
        .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())??;
    logger::log_info(&format!(
        "[Qoder CN Switch] 写入客户端完成: account_id={account_id}, client={client}, elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(account_id)
}

/// The CN desktop client only ships for Windows, so other platforms fall back to
/// the CLI client instead of failing on a missing OS key store.
fn default_cn_client() -> qoder_native::NativeClient {
    if cfg!(target_os = "windows") {
        qoder_native::NativeClient::Desktop
    } else {
        qoder_native::NativeClient::Cli
    }
}

#[tauri::command]
pub async fn inject_qoder_cn_account(app: AppHandle, account_id: String) -> Result<String, String> {
    qoder_oauth::prepare_cn_account_for_switch(&account_id).await?;
    let id = account_id.clone();
    tauri::async_runtime::spawn_blocking(move || qoder_native::switch(&id, default_cn_client()))
        .await
        .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())??;
    if let Err(error) = crate::modules::tray::update_tray_menu(&app) {
        logger::log_warn(&format!("[Qoder CN Switch] 刷新托盘失败: {error}"));
    }
    Ok(account_id)
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
