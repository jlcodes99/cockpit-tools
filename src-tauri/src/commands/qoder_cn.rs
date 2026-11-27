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
        qoder_native::import(qoder_native::NativeRegion::Cn, client).map(|item| vec![item])
    })
    .await
    .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())?
}

#[tauri::command]
pub async fn import_qoder_cn_from_local(app: AppHandle) -> Result<Vec<QoderAccount>, String> {
    let found = tauri::async_runtime::spawn_blocking(|| {
        qoder_native::import_from_local(qoder_native::NativeRegion::Cn)
    })
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
pub async fn get_qoder_cn_checkin_status(
    account_id: String,
) -> Result<qoder_oauth::QoderCheckinStatus, String> {
    qoder_oauth::get_qoder_checkin_status(&account_id).await
}

#[tauri::command]
pub async fn claim_qoder_cn_checkin(
    account_id: String,
) -> Result<qoder_oauth::QoderCheckinStatus, String> {
    qoder_oauth::claim_qoder_checkin(&account_id).await
}

/// Result of a CN switch. `relaunched` reports whether the client that was
/// closed before the write has been started again, so the UI never claims a
/// relaunch that did not happen.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderCnSwitchOutcome {
    account_id: String,
    relaunched: bool,
}

#[tauri::command]
pub async fn inject_qoder_cn_native(
    account_id: String,
    client: String,
) -> Result<QoderCnSwitchOutcome, String> {
    let target = qoder_native::NativeClient::parse(&client)?;
    qoder_oauth::prepare_cn_account_for_switch(&account_id).await?;
    let id = account_id.clone();
    let started_at = Instant::now();
    let relaunched = tauri::async_runtime::spawn_blocking(move || switch_qoder_cn(&id, target))
        .await
        .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())??;
    logger::log_info(&format!(
        "[Qoder CN Switch] 写入客户端完成: account_id={account_id}, client={client}, relaunched={relaunched}, elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(QoderCnSwitchOutcome {
        account_id,
        relaunched,
    })
}

/// Closes the desktop client, replaces its credentials and starts it again.
/// Returns whether the client was actually started again.
#[cfg(target_os = "windows")]
fn switch_qoder_cn(
    account_id: &str,
    target: qoder_native::NativeClient,
) -> Result<bool, String> {
    if target != qoder_native::NativeClient::Desktop {
        qoder_native::switch(qoder_native::NativeRegion::Cn, account_id, target)?;
        return Ok(false);
    }

    // Resolve the binary before closing it. A running client is the most
    // reliable source of its own path; the stored/detected path covers the case
    // where the client is not running yet, so the switch can still bring it up.
    // Failing to resolve is not fatal: writing credentials is useful on its own,
    // and the UI falls back to asking the user to open the client manually.
    let launch_path = match crate::modules::process::resolve_qoder_cn_launch_path() {
        Ok(path) => Some(path),
        Err(error) => {
            logger::log_warn(&format!(
                "[Qoder CN Switch] launch path unresolved, skipping relaunch: {error}"
            ));
            None
        }
    };
    let system = sysinfo::System::new_all();
    let pids = qoder_native::qoder_cn_desktop_pids(&system);
    if !pids.is_empty() {
        // A close timeout means the client is still running. Starting another copy
        // here would leave the user with two instances holding the same profile,
        // so the failure is reported as-is.
        crate::modules::process::close_qoder_cn_desktop(&pids, 20)?;
    }
    if let Err(error) = qoder_native::switch(qoder_native::NativeRegion::Cn, account_id, target) {
        // Credentials were rolled back and the client is closed, so the real
        // state depends on whether it comes back, not on the original error.
        return Err(restore_after_failure(launch_path.as_deref(), error));
    }
    let Some(path) = launch_path else {
        return Ok(false);
    };
    crate::modules::process::start_qoder_cn_desktop(&path)
        .map_err(|error| format!("QODER_CN_RELAUNCH_FAILED: {error}"))?;
    Ok(true)
}

/// Last resort after a failed switch: put the client back. When that also fails
/// the caller must report both facts instead of an unrelated error.
#[cfg(target_os = "windows")]
fn restore_after_failure(executable: Option<&std::path::Path>, cause: String) -> String {
    let Some(path) = executable else {
        return cause;
    };
    // The launcher may have brought the client back on its own while the write was
    // failing; only start one when nothing is running.
    if !qoder_native::qoder_cn_desktop_pids(&sysinfo::System::new_all()).is_empty() {
        return cause;
    }
    match crate::modules::process::start_qoder_cn_desktop(path) {
        Ok(_) => cause,
        Err(relaunch_error) => {
            logger::log_warn(&format!(
                "[Qoder CN Switch] restore launch failed after failure: cause={cause}, relaunch={relaunch_error}"
            ));
            format!("QODER_CN_RELAUNCH_FAILED: {relaunch_error} (cause: {cause})")
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn switch_qoder_cn(
    account_id: &str,
    target: qoder_native::NativeClient,
) -> Result<bool, String> {
    qoder_native::switch(qoder_native::NativeRegion::Cn, account_id, target)?;
    Ok(false)
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

/// Switch used by the dashboard, the floating card, automation and the native
/// menu. It runs the same close → write → relaunch sequence as the CN page: a
/// live desktop client would reject the write (`QODER_CLIENT_RUNNING`) and undo it
/// with its own next save, so every entry point has to go through one path.
#[tauri::command]
pub async fn inject_qoder_cn_account(app: AppHandle, account_id: String) -> Result<String, String> {
    qoder_oauth::prepare_cn_account_for_switch(&account_id).await?;
    let id = account_id.clone();
    let client = default_cn_client();
    let relaunched = tauri::async_runtime::spawn_blocking(move || switch_qoder_cn(&id, client))
        .await
        .map_err(|_| "QODER_NATIVE_WORKER_FAILED".to_string())??;
    // This command only answers with the account id, so a desktop client that was
    // closed and could not be started again would otherwise look like a plain
    // success. The page path reports it to the user; here at least leave a trace.
    if !relaunched && default_cn_client() == qoder_native::NativeClient::Desktop {
        logger::log_warn(&format!(
            "[Qoder CN Switch] 凭据已写入但客户端未自动重启: account_id={account_id}"
        ));
    }
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
            if let Err(error) = qoder_native::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[Qoder CN Command] 配额预警检查失败: {error}"));
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
    if let Err(error) = qoder_native::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[Qoder CN Command] 配额预警检查失败: {error}"));
    }
    logger::log_info(&format!(
        "[Qoder CN Command] 批量刷新完成: refreshed={refreshed}, elapsed={}ms",
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}
