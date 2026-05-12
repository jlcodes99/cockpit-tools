use std::time::Instant;
use tauri::AppHandle;

use crate::models::gemini::{GeminiAccount, GeminiOAuthCompletePayload, GeminiOAuthStartResponse};
use crate::modules::{gemini_account, gemini_oauth, logger};

#[tauri::command]
pub fn list_gemini_accounts() -> Result<Vec<GeminiAccount>, String> {
    gemini_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_gemini_account(account_id: String) -> Result<(), String> {
    gemini_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_gemini_accounts(account_ids: Vec<String>) -> Result<(), String> {
    gemini_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub async fn import_gemini_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<GeminiAccount>, String> {
    let mut accounts = gemini_account::import_from_json(&json_content)?;

    for account in accounts.iter_mut() {
        match gemini_account::refresh_account_token(&account.id).await {
            Ok(refreshed) => *account = refreshed,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Gemini Command] JSON 导入后刷新失败: account_id={}, error={}",
                    account.id, error
                ));
                let _ =
                    gemini_account::set_account_status(&account.id, Some("error"), Some(&error));
                account.status = Some("error".to_string());
                account.status_reason = Some(error);
            }
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn import_gemini_from_local(app: AppHandle) -> Result<Vec<GeminiAccount>, String> {
    let mut account = match gemini_account::import_from_local()? {
        Some(a) => a,
        None => return Err("未找到本地 Gemini 登录信息".to_string()),
    };

    match gemini_account::refresh_account_token(&account.id).await {
        Ok(refreshed) => account = refreshed,
        Err(error) => {
            logger::log_warn(&format!(
                "[Gemini Command] 本地导入后刷新失败: account_id={}, error={}",
                account.id, error
            ));
            let _ = gemini_account::set_account_status(&account.id, Some("error"), Some(&error));
            account.status = Some("error".to_string());
            account.status_reason = Some(error);
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_gemini_accounts(account_ids: Vec<String>) -> Result<String, String> {
    gemini_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_gemini_token(
    app: AppHandle,
    account_id: String,
) -> Result<GeminiAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Gemini Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    match gemini_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Gemini Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Gemini Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            let _ = gemini_account::set_account_status(&account_id, Some("error"), Some(&err));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_gemini_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Gemini Command] 批量刷新开始");

    let results = gemini_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);

    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Gemini Command] 批量刷新完成: success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn gemini_oauth_login_start() -> Result<GeminiOAuthStartResponse, String> {
    logger::log_info("[Gemini Command] OAuth 登录开始");
    gemini_oauth::start_login().await
}

#[tauri::command]
pub async fn gemini_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<GeminiAccount, String> {
    logger::log_info(&format!(
        "[Gemini Command] OAuth 等待完成: login_id={}",
        login_id
    ));

    let payload = gemini_oauth::complete_login(&login_id).await?;
    let mut account = gemini_account::upsert_account(payload)?;

    match gemini_account::refresh_account_token(&account.id).await {
        Ok(refreshed) => {
            account = refreshed;
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Gemini OAuth] 登录后自动刷新配额失败: account_id={}, error={}",
                account.id, error
            ));
            let _ = gemini_account::set_account_status(&account.id, Some("error"), Some(&error));
            account.status = Some("error".to_string());
            account.status_reason = Some(error);
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Gemini Command] OAuth 登录完成: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn gemini_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Gemini Command] OAuth 取消: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    gemini_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn gemini_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    gemini_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub async fn add_gemini_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<GeminiAccount, String> {
    let payload = GeminiOAuthCompletePayload {
        email: "unknown@gmail.com".to_string(),
        auth_id: None,
        name: None,
        access_token,
        refresh_token: None,
        id_token: None,
        token_type: None,
        scope: None,
        expiry_date: None,
        selected_auth_type: Some("oauth-personal".to_string()),
        project_id: None,
        tier_id: None,
        plan_name: None,
        gemini_auth_raw: None,
        gemini_usage_raw: None,
        status: None,
        status_reason: None,
    };

    let mut account = gemini_account::upsert_account(payload)?;
    match gemini_account::refresh_account_token(&account.id).await {
        Ok(refreshed) => account = refreshed,
        Err(error) => {
            logger::log_warn(&format!(
                "[Gemini Command] Token 导入后刷新失败: account_id={}, error={}",
                account.id, error
            ));
            let _ = gemini_account::set_account_status(&account.id, Some("error"), Some(&error));
            account.status = Some("error".to_string());
            account.status_reason = Some(error);
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn update_gemini_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<GeminiAccount, String> {
    gemini_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_gemini_accounts_index_path() -> Result<String, String> {
    gemini_account::accounts_index_path_string()
}

#[tauri::command]
pub fn inject_gemini_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Gemini Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = gemini_account::load_account(&account_id)
        .ok_or_else(|| format!("Gemini account not found: {}", account_id))?;
    gemini_account::inject_to_gemini(&account_id)?;
    crate::modules::provider_current_state::set_current_account_id(
        "gemini",
        Some(account_id.as_str()),
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Gemini Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    crate::modules::wecom_switch_notify::notify_switch(
        "Gemini",
        &account.id,
        &account.email,
        "manual",
        "tools.gemini.switch",
        None,
    );
    Ok(format!("切换完成: {}", account.email))
}
