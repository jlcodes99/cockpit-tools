use tauri::{AppHandle, Emitter};

use crate::models::zcode::{ZcodeAccount, ZcodeOAuthStartResponse};
use crate::modules::{logger, process, tray, zcode_account, zcode_instance, zcode_oauth};

#[tauri::command]
pub fn list_zcode_accounts() -> Result<Vec<ZcodeAccount>, String> {
    zcode_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_zcode_account(account_id: String) -> Result<(), String> {
    zcode_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_zcode_accounts(account_ids: Vec<String>) -> Result<(), String> {
    zcode_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_zcode_from_json(json_content: String) -> Result<Vec<ZcodeAccount>, String> {
    zcode_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_zcode_from_local(app: AppHandle) -> Result<Vec<ZcodeAccount>, String> {
    let accounts = zcode_account::import_from_local().await?;
    let _ = tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub fn import_zcode_api_key(
    app: AppHandle,
    api_key: String,
    provider: String,
    account_name: Option<String>,
) -> Result<ZcodeAccount, String> {
    let account = zcode_account::import_api_key(&api_key, &provider, account_name.as_deref())?;
    let _ = tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn export_zcode_accounts(account_ids: Vec<String>) -> Result<String, String> {
    zcode_account::export_accounts(&account_ids)
}

#[tauri::command]
pub fn zcode_oauth_login_start(provider: String) -> Result<ZcodeOAuthStartResponse, String> {
    zcode_oauth::start_oauth_login(&provider)
}

#[tauri::command]
pub async fn zcode_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<ZcodeAccount, String> {
    let account = zcode_oauth::complete_oauth_login(&login_id).await?;
    let _ = tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn zcode_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    zcode_oauth::submit_callback_url(&login_id, &callback_url).await
}

#[tauri::command]
pub async fn zcode_oauth_open_window(
    app: AppHandle,
    auth_url: String,
    incognito: Option<bool>,
) -> Result<(), String> {
    zcode_oauth::open_oauth_window(&app, &auth_url, incognito.unwrap_or(false))
}

#[tauri::command]
pub fn zcode_oauth_login_cancel(app: AppHandle, login_id: Option<String>) -> Result<(), String> {
    zcode_oauth::cancel_oauth_login(login_id.as_deref())?;
    zcode_oauth::close_oauth_window(&app)
}

#[tauri::command]
pub async fn refresh_zcode_account(
    app: AppHandle,
    account_id: String,
) -> Result<ZcodeAccount, String> {
    let account = zcode_account::refresh_account_quota(&account_id).await?;
    let _ = tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_zcode_accounts(app: AppHandle) -> Result<i32, String> {
    let count = zcode_account::refresh_all_accounts().await?;
    let _ = tray::update_tray_menu(&app);
    Ok(count)
}

#[tauri::command]
pub fn inject_zcode_account(app: AppHandle, account_id: String) -> Result<String, String> {
    if let Err(error) = zcode_instance::resolve_executable() {
        if error == "APP_PATH_NOT_FOUND:zcode" {
            let _ = app.emit("app:path_missing", serde_json::json!({
                "app": "zcode", "retry": { "kind": "switchAccount", "accountId": account_id }
            }));
        }
        return Err(error);
    }
    let settings = zcode_instance::load_default_settings()?;
    if let Some(pid) = zcode_instance::resolve_pid(None, settings.last_pid) {
        process::close_pid(pid, 20)?;
    }
    zcode_instance::mark_stopped(None)?;
    let account = zcode_account::inject_to_default(&account_id)?;
    if let Err(error) =
        zcode_instance::update_default_settings(Some(Some(account_id)), None, Some(false))
    {
        logger::log_warn(&format!("更新 ZCode 默认实例绑定账号失败: {}", error));
    }
    let args = process::parse_extra_args(&settings.extra_args);
    match zcode_instance::start_default(&args) {
        Ok(pid) => zcode_instance::mark_started(None, pid)?,
        Err(error) if error == "APP_PATH_NOT_FOUND:zcode" => {
            let _ = app.emit("app:path_missing", serde_json::json!({
                "app": "zcode", "retry": { "kind": "launchOnly" }
            }));
        }
        Err(error) => {
            return Err(format!("ZCode 账号已切换，但默认实例启动失败: {error}"));
        }
    }
    let _ = tray::update_tray_menu(&app);
    Ok(account.email)
}

#[tauri::command]
pub async fn zcode_launch_current() -> Result<(), String> {
    tokio::task::spawn_blocking(|| {
        let settings = zcode_instance::load_default_settings()?;
        let args = process::parse_extra_args(&settings.extra_args);
        let pid = zcode_instance::start_default(&args)?;
        zcode_instance::mark_started(None, pid)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn update_zcode_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<ZcodeAccount, String> {
    zcode_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_zcode_current_account_id() -> Result<Option<String>, String> {
    zcode_account::current_account_id()
}

#[tauri::command]
pub fn get_zcode_accounts_index_path() -> Result<String, String> {
    zcode_account::accounts_index_path_string()
}
