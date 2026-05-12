use std::time::Instant;

use crate::models::openrouter::OpenRouterAccount;
use crate::modules::{logger, openrouter_account};

#[tauri::command]
pub fn list_openrouter_accounts() -> Result<Vec<OpenRouterAccount>, String> {
    openrouter_account::list_accounts_checked()
}

#[tauri::command]
pub async fn add_openrouter_account(api_key: String) -> Result<OpenRouterAccount, String> {
    let started_at = Instant::now();
    logger::log_info("[OpenRouter Command] 添加账号开始");
    match openrouter_account::add_account_with_key(&api_key).await {
        Ok(account) => {
            logger::log_info(&format!(
                "[OpenRouter Command] 账号添加成功: id={}, key_type={}, is_free={}, elapsed={}ms",
                account.id,
                account.key_type.as_str(),
                account.is_free_tier,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[OpenRouter Command] 添加账号失败: elapsed={}ms, error={}",
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub fn delete_openrouter_account(account_id: String) -> Result<(), String> {
    openrouter_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_openrouter_accounts(account_ids: Vec<String>) -> Result<(), String> {
    openrouter_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_openrouter_from_json(json_content: String) -> Result<Vec<OpenRouterAccount>, String> {
    openrouter_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn export_openrouter_accounts(account_ids: Vec<String>) -> Result<String, String> {
    openrouter_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_openrouter_token(account_id: String) -> Result<OpenRouterAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[OpenRouter Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    match openrouter_account::refresh_account_from_api(&account_id).await {
        Ok(account) => {
            logger::log_info(&format!(
                "[OpenRouter Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[OpenRouter Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_openrouter_tokens() -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[OpenRouter Command] 批量刷新开始");
    let refreshed = openrouter_account::refresh_all_accounts_from_api().await?;
    logger::log_info(&format!(
        "[OpenRouter Command] 批量刷新完成: refreshed={}, elapsed={}ms",
        refreshed,
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}

#[tauri::command]
pub async fn inject_openrouter_account(account_id: String) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[OpenRouter Command] 开始注入账号: account_id={}",
        account_id
    ));

    let account = openrouter_account::load_account(&account_id)
        .ok_or_else(|| format!("OpenRouter 账号不存在: {}", account_id))?;

    openrouter_account::inject_to_openrouter(&account_id)?;
    crate::modules::provider_current_state::set_current_account_id(
        "openrouter",
        Some(account_id.as_str()),
    )?;

    logger::log_info(&format!(
        "[OpenRouter Command] 注入成功: account_id={}, elapsed={}ms",
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(format!("注入完成: {}", account.email))
}

#[tauri::command]
pub fn update_openrouter_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<OpenRouterAccount, String> {
    openrouter_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub async fn fetch_openrouter_credits(account_id: String) -> Result<serde_json::Value, String> {
    openrouter_account::fetch_credits_from_api(&account_id).await
}

#[tauri::command]
pub async fn fetch_openrouter_models(
) -> Result<Vec<crate::models::openrouter::OpenRouterModel>, String> {
    let started_at = Instant::now();
    logger::log_info("[OpenRouter Command] 获取模型列表开始");
    match openrouter_account::fetch_models_from_api().await {
        Ok(models) => {
            logger::log_info(&format!(
                "[OpenRouter Command] 获取模型列表完成: count={}, elapsed={}ms",
                models.len(),
                started_at.elapsed().as_millis()
            ));
            Ok(models)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[OpenRouter Command] 获取模型列表失败: elapsed={}ms, error={}",
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn fetch_openrouter_activity(
    account_id: String,
    days: Option<u32>,
) -> Result<serde_json::Value, String> {
    openrouter_account::fetch_activity_from_api(&account_id, days.unwrap_or(30)).await
}
