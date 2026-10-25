use tauri::Emitter;

use crate::models::minimax_code::{MinimaxCodeAccount, MinimaxCodeOAuthStartResponse};
use crate::modules::{
    logger, minimax_code_account, minimax_code_api, minimax_code_auto_checkin, minimax_code_oauth,
};

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

    // The client's auth.json carries credentials but no identity, so the payload
    // used to be named after the record-key digest ("MiniMax Code 本机账号
    // (grC2d5…)"). Probe user/info for the real name, and take the quota while we
    // are online so the card is populated on the first render.
    let client = minimax_code_oauth::build_client()?;
    let mut enriched = Vec::with_capacity(payloads.len());
    for mut payload in payloads {
        let region = payload.region.clone().unwrap_or_else(|| "cn".to_string());
        match minimax_code_oauth::fetch_user_info(&client, &payload.access_token, &region).await {
            Ok((uid, nickname, email)) => {
                if uid.is_some() {
                    payload.uid = uid;
                }
                payload.nickname = nickname;
                if !email.trim().is_empty() {
                    payload.email = email;
                }
                // No email on this endpoint: fall back to the username.
                if let Some(nickname) = payload
                    .nickname
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    payload.email = nickname.to_string();
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[MiniMax Code Import] 本机导入时探测账号身份失败，沿用本地记录:{}",
                    err
                ));
            }
        }
        payload.quota_raw =
            minimax_code_oauth::fetch_credit_details(&client, &payload.access_token, &region)
                .await
                .ok();
        enriched.push(payload);
    }

    let mut accounts = Vec::new();
    for payload in enriched {
        accounts.push(minimax_code_account::upsert_account(payload)?);
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub async fn inject_minimax_code_to_client(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<String, String> {
    logger::log_info(&format!(
        "[MiniMax Code Switch] 开始切换账号: account_id={}",
        account_id
    ));
    let account = minimax_code_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {account_id}"))?;
    let email = account.email.clone();

    // Switch first and publish the new current account only on success: a failed
    // switch must not leave the app pointing at an account the client is not using.
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::modules::minimax_code_account::switch_and_launch(&account)
    })
    .await
    .map_err(|e| format!("MiniMax Code 切号后台任务失败: {e}"))??;

    crate::modules::provider_current_state::set_current_account_id(
        "minimaxcode",
        Some(account_id.as_str()),
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[MiniMax Code Switch] 切换完成: account_id={}, email={}, client={}",
        account_id,
        email,
        result.display()
    ));
    Ok(format!("切换完成: {} ({})", email, result.display()))
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

#[tauri::command]
pub async fn get_checkin_status_minimax_code(
    account_id: String,
) -> Result<minimax_code_api::MinimaxCodeCheckinStatus, String> {
    let account = minimax_code_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {account_id}"))?;
    minimax_code_api::get_checkin_status(
        &account.access_token,
        account.region.as_deref().unwrap_or("cn"),
    ).await
}

#[tauri::command]
pub async fn checkin_minimax_code(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<minimax_code_api::MinimaxCodeCheckinResult, String> {
    let account = minimax_code_account::load_account(&account_id)
        .ok_or_else(|| format!("账号不存在: {account_id}"))?;
    // MiniMax tokens are ~1h and the client renews them locally; refresh first so a
    // stale token does not burn the retry budget.
    let account = match minimax_code_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(_) => account,
    };
    let region = account.region.as_deref().unwrap_or("cn").to_string();
    let response = minimax_code_api::perform_checkin(&account.access_token, &region).await?;
    if response.success {
        let now = chrono::Utc::now().timestamp();
        let streak = if response.already_claimed {
            account.checkin_streak.unwrap_or(0)
        } else {
            response.streak_days
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or_else(|| account.checkin_streak.unwrap_or(0).saturating_add(1))
        };
        let reward = response.credit.map(|credit| serde_json::json!({ "credit": credit }));
        minimax_code_account::update_checkin_info(&account_id, Some(now), streak, reward)?;
        let _ = crate::modules::tray::update_tray_menu(&app);
        let _ = app.emit("minimaxcode:checkin_completed", serde_json::json!({
            "accountId": account_id, "success": true, "credit": response.credit, "streakDays": response.streak_days,
        }));
    }
    Ok(response)
}

#[tauri::command]
pub fn get_minimax_code_auto_checkin_config(
) -> Result<minimax_code_auto_checkin::MinimaxCodeAutoCheckinConfig, String> {
    minimax_code_auto_checkin::get_config_checked()
}
#[tauri::command]
pub fn save_minimax_code_auto_checkin_config(
    config: minimax_code_auto_checkin::MinimaxCodeAutoCheckinConfig,
) -> Result<(), String> {
    minimax_code_auto_checkin::save_config(&config)
}
#[tauri::command]
pub fn get_minimax_code_auto_checkin_logs(
) -> Result<Vec<minimax_code_auto_checkin::MinimaxCodeAutoCheckinLogRecord>, String> {
    minimax_code_auto_checkin::get_logs_checked()
}
#[tauri::command]
pub fn clear_minimax_code_auto_checkin_logs() -> Result<(), String> {
    minimax_code_auto_checkin::save_logs(&[])
}
#[tauri::command]
pub async fn run_minimax_code_auto_checkin_now(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<String, String> {
    minimax_code_auto_checkin::run_minimax_code_auto_checkin_cycle_if_needed(&app, force.unwrap_or(false)).await
}
