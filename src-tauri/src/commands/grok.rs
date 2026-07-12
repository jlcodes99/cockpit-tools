use std::time::Instant;
use tauri::AppHandle;

use crate::models::grok::{GrokAccount, GrokOAuthStartResponse};
use crate::modules::{grok_account, logger};

#[tauri::command]
pub fn list_grok_accounts() -> Result<Vec<GrokAccount>, String> {
    grok_account::list_accounts_checked()
}

#[tauri::command]
pub fn get_current_grok_account() -> Result<Option<GrokAccount>, String> {
    Ok(grok_account::get_current_account())
}

#[tauri::command]
pub fn delete_grok_account(app: AppHandle, account_id: String) -> Result<(), String> {
    grok_account::remove_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_grok_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    grok_account::remove_accounts(&account_ids)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub async fn import_grok_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<GrokAccount>, String> {
    let mut accounts = grok_account::import_from_json(&json_content)?;
    for account in accounts.iter_mut() {
        match grok_account::refresh_account_token(&account.id).await {
            Ok(refreshed) => *account = refreshed,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Grok] JSON 导入后刷新失败: account_id={}, error={}",
                    account.id, error
                ));
                let _ = grok_account::set_account_status(&account.id, Some("error"), Some(&error));
                account.status = Some("error".into());
                account.status_reason = Some(error);
            }
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_grok_from_local(app: AppHandle) -> Result<Vec<GrokAccount>, String> {
    let mut accounts = grok_account::import_from_local()?;
    for account in accounts.iter_mut() {
        match grok_account::refresh_account_token(&account.id).await {
            Ok(refreshed) => *account = refreshed,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Grok] 本地导入后刷新失败: account_id={}, error={}",
                    account.id, error
                ));
                let _ = grok_account::set_account_status(&account.id, Some("error"), Some(&error));
                account.status = Some("error".into());
                account.status_reason = Some(error);
            }
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub fn export_grok_accounts(account_ids: Vec<String>) -> Result<String, String> {
    grok_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_grok_token(
    app: AppHandle,
    account_id: String,
) -> Result<GrokAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Grok] 手动刷新开始: account_id={}",
        account_id
    ));
    match grok_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Grok] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Grok] 刷新失败: account_id={}, error={}",
                account_id, err
            ));
            let _ = grok_account::set_account_status(&account_id, Some("error"), Some(&err));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_grok_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Grok] 批量刷新开始");
    let results = grok_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Grok] 批量刷新完成: success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub fn update_grok_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<GrokAccount, String> {
    grok_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_grok_accounts_index_path() -> Result<String, String> {
    grok_account::accounts_index_path_string()
}

#[tauri::command]
pub fn get_grok_auth_json_path() -> Result<String, String> {
    Ok(grok_account::get_auth_json_path()?.to_string_lossy().to_string())
}

/// 切换账号（写入 ~/.grok/auth.json）
#[tauri::command]
pub async fn switch_grok_account(
    app: AppHandle,
    account_id: String,
) -> Result<GrokAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Grok Switch] 开始: account_id={}",
        account_id
    ));

    // 切号前刷新 token（refresh_token 单次有效）；失败且 token 已过期则禁止写入坏凭据
    match grok_account::refresh_account_token(&account_id).await {
        Ok(_) => {}
        Err(err) => {
            let acc = grok_account::load_account(&account_id)
                .ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
            if grok_account::needs_token_refresh(&acc) {
                return Err(format!("切号前刷新失败，已阻止写入过期凭据: {}", err));
            }
            logger::log_warn(&format!(
                "[Grok Switch] 刷新失败但 access 仍可用，继续切号: account_id={}, error={}",
                account_id, err
            ));
        }
    }
    let account = grok_account::inject_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Grok Switch] 成功: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

/// 兼容旧调用：与 switch 相同，必须先 refresh 再写 auth.json（禁止裸写过期凭据）
#[tauri::command]
pub async fn inject_grok_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let account = switch_grok_account(app, account_id).await?;
    Ok(format!("切换完成: {}", account.email))
}

#[tauri::command]
pub async fn grok_oauth_login_start() -> Result<GrokOAuthStartResponse, String> {
    logger::log_info("[Grok] Device OAuth 登录开始");
    grok_account::oauth_login_start().await
}

#[tauri::command]
pub async fn grok_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<GrokAccount, String> {
    logger::log_info(&format!(
        "[Grok] Device OAuth 等待完成: login_id={}",
        login_id
    ));
    let account = grok_account::oauth_login_complete(&login_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn grok_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    grok_account::oauth_login_cancel(login_id.as_deref())
}
