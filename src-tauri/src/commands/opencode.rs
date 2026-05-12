use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::opencode::OpenCodeAccount;
use crate::modules::{logger, opencode_account};

#[tauri::command]
pub fn list_opencode_accounts() -> Result<Vec<OpenCodeAccount>, String> {
    opencode_account::list_accounts_checked()
}

#[tauri::command]
pub fn add_opencode_account(api_key: String, tier: String) -> Result<OpenCodeAccount, String> {
    let email = "unknown".to_string();
    let normalized_tier = tier.trim().to_lowercase();
    let tier_enum = match normalized_tier.as_str() {
        "go" => crate::models::opencode::OpenCodeTier::Go,
        "zen" => crate::models::opencode::OpenCodeTier::Zen,
        "free" => crate::models::opencode::OpenCodeTier::Free,
        _ => return Err(format!("Invalid OpenCode tier: {}", tier)),
    };

    let payload = crate::models::opencode::OpenCodeImportPayload {
        email,
        name: None,
        access_token: api_key,
        tier: tier_enum,
        plan_name: None,
        subscription_status: None,
        usage_raw: None,
        status: None,
        status_reason: None,
    };
    let account = opencode_account::upsert_account(payload)?;
    Ok(account)
}

#[tauri::command]
pub fn delete_opencode_account(account_id: String) -> Result<(), String> {
    opencode_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_opencode_accounts(account_ids: Vec<String>) -> Result<(), String> {
    opencode_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_opencode_from_json(json_content: String) -> Result<Vec<OpenCodeAccount>, String> {
    opencode_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn export_opencode_accounts(account_ids: Vec<String>) -> Result<String, String> {
    opencode_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_opencode_token(
    app: AppHandle,
    account_id: String,
) -> Result<OpenCodeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[OpenCode Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    match opencode_account::refresh_account_async(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[OpenCode Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[OpenCode Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_opencode_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[OpenCode Command] 批量刷新开始");

    let results = opencode_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();

    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[OpenCode Command] 批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn update_opencode_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<OpenCodeAccount, String> {
    opencode_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_opencode_accounts_index_path() -> Result<String, String> {
    opencode_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_opencode_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[OpenCode Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = opencode_account::load_account(&account_id)
        .ok_or_else(|| format!("OpenCode account not found: {}", account_id))?;

    opencode_account::inject_to_opencode(&account_id)?;
    crate::modules::provider_current_state::set_current_account_id(
        "opencode",
        Some(account_id.as_str()),
    )?;

    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[OpenCode Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
        account.id,
        account.email,
        started_at.elapsed().as_millis()
    ));
    Ok(format!("切换完成: {}", account.email))
}
