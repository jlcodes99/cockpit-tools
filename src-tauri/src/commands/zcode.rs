use std::time::Instant;
use tauri::AppHandle;

use crate::models::zcode::ZcodeAccount;
use crate::modules::{logger, zcode_account};

#[tauri::command]
pub fn list_zcode_accounts() -> Result<Vec<ZcodeAccount>, String> {
    zcode_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_zcode_account(app: AppHandle, account_id: String) -> Result<(), String> {
    zcode_account::remove_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_zcode_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    zcode_account::remove_accounts(&account_ids)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn import_zcode_from_local(app: AppHandle) -> Result<Vec<ZcodeAccount>, String> {
    let started_at = Instant::now();
    logger::log_info("[Zcode Command] 从本机导入 ZCode 账号开始");
    let accounts = zcode_account::import_from_local()?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zcode Command] 从本机导入 ZCode 账号完成: imported={}, elapsed={}ms",
        accounts.len(),
        started_at.elapsed().as_millis()
    ));
    Ok(accounts)
}

#[tauri::command]
pub fn import_zcode_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<ZcodeAccount>, String> {
    let accounts = zcode_account::import_from_json(&json_content)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_zcode_accounts(account_ids: Vec<String>) -> Result<String, String> {
    zcode_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_zcode_quota(
    app: AppHandle,
    account_id: String,
) -> Result<ZcodeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Zcode Command] 手动刷新账号额度开始: account_id={}",
        account_id
    ));
    let account = zcode_account::refresh_account(&account_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zcode Command] 手动刷新账号额度完成: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_zcode_quotas(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Zcode Command] 批量刷新额度开始");
    let refreshed = zcode_account::refresh_all_accounts().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Zcode Command] 批量刷新额度完成: refreshed={}, elapsed={}ms",
        refreshed.len(),
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed.len() as i32)
}

#[tauri::command]
pub fn update_zcode_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<ZcodeAccount, String> {
    zcode_account::update_account_tags(&account_id, tags)
}
