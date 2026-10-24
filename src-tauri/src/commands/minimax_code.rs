use crate::models::minimax_code::{MinimaxCodeAccount, MinimaxCodeOAuthStartResponse};
use crate::modules::{logger, minimax_code_account, minimax_code_oauth};

#[tauri::command]
pub fn list_minimax_code_accounts() -> Result<Vec<MinimaxCodeAccount>, String> {
    minimax_code_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_minimax_code_account(account_id: String) -> Result<(), String> {
    minimax_code_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_minimax_code_accounts(account_ids: Vec<String>) -> Result<(), String> {
    minimax_code_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_minimax_code_from_json(json_content: String) -> Result<Vec<MinimaxCodeAccount>, String> {
    minimax_code_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn export_minimax_code_accounts(account_ids: Vec<String>) -> Result<String, String> {
    minimax_code_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_minimax_code_token(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<MinimaxCodeAccount, String> {
    let account = minimax_code_account::refresh_account_token(&account_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_minimax_code_tokens(app: tauri::AppHandle) -> Result<i32, String> {
    let results = minimax_code_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn add_minimax_code_account_with_token(
    app: tauri::AppHandle,
    access_token: String,
    region: Option<String>,
) -> Result<MinimaxCodeAccount, String> {
    let payload = minimax_code_oauth::build_payload_from_token(&access_token, region.as_deref()).await?;
    let account = minimax_code_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_minimax_code_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<MinimaxCodeAccount, String> {
    minimax_code_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_minimax_code_accounts_index_path() -> Result<String, String> {
    minimax_code_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn import_minimax_code_from_local(app: tauri::AppHandle) -> Result<Vec<MinimaxCodeAccount>, String> {
    let payloads = tauri::async_runtime::spawn_blocking(minimax_code_account::import_payload_from_local)
        .await
        .map_err(|e| format!("MiniMax Code 本机导入后台任务失败: {e}"))??;
    let mut accounts = Vec::new();
    for payload in payloads {
        accounts.push(minimax_code_account::upsert_account(payload)?);
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn minimax_code_oauth_login_start(
    region: Option<String>,
) -> Result<MinimaxCodeOAuthStartResponse, String> {
    let region = region.unwrap_or_else(|| crate::modules::config::get_user_config().minimax_code_region);
    logger::log_info("MiniMax Code OAuth start 命令触发");
    minimax_code_oauth::start_login(&region).await
}

#[tauri::command]
pub fn minimax_code_oauth_login_peek() -> Option<MinimaxCodeOAuthStartResponse> {
    minimax_code_oauth::peek_pending_login()
}

#[tauri::command]
pub async fn minimax_code_oauth_login_complete(
    app: tauri::AppHandle,
    login_id: String,
) -> Result<MinimaxCodeAccount, String> {
    logger::log_info(&format!("MiniMax Code OAuth complete 命令触发：login_id={login_id}"));
    let payload = minimax_code_oauth::complete_login(&login_id).await?;
    let mut account = minimax_code_account::upsert_account(payload)?;
    if let Ok(refreshed) = minimax_code_account::refresh_account_token(&account.id).await {
        account = refreshed;
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn minimax_code_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    minimax_code_oauth::cancel_login(login_id.as_deref())
}
