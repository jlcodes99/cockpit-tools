use std::time::Instant;

use tauri::AppHandle;

use crate::models::workbuddy::{WorkbuddyAccount, WorkbuddyOAuthStartResponse};
use crate::modules::{logger, workbuddy_ai_account, workbuddy_ai_oauth};

async fn refresh_workbuddy_ai_account_after_login(account: WorkbuddyAccount) -> WorkbuddyAccount {
    let account_id = account.id.clone();
    match workbuddy_ai_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[WorkBuddy AI OAuth] 登录后刷新失败，保留原账号信息：account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

#[tauri::command]
pub fn list_workbuddy_ai_accounts() -> Result<Vec<WorkbuddyAccount>, String> {
    workbuddy_ai_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_workbuddy_ai_account(account_id: String) -> Result<(), String> {
    workbuddy_ai_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_workbuddy_ai_accounts(account_ids: Vec<String>) -> Result<(), String> {
    workbuddy_ai_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_workbuddy_ai_from_json(json_content: String) -> Result<Vec<WorkbuddyAccount>, String> {
    workbuddy_ai_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_workbuddy_ai_from_local(app: AppHandle) -> Result<Vec<WorkbuddyAccount>, String> {
    let mut local_payload = match workbuddy_ai_account::import_payload_from_local()? {
        Some(payload) => payload,
        None => return Err("未在本机 WorkBuddy AI 客户端中找到登录信息".to_string()),
    };

    match workbuddy_ai_oauth::build_payload_from_token(&local_payload.access_token).await {
        Ok(mut payload) => {
            if payload.uid.is_none() {
                payload.uid = local_payload.uid.clone();
            }
            if payload.nickname.is_none() {
                payload.nickname = local_payload.nickname.clone();
            }
            if payload.refresh_token.is_none() {
                payload.refresh_token = local_payload.refresh_token.clone();
            }
            if payload.domain.is_none() {
                payload.domain = local_payload.domain.clone();
            }
            if payload.token_type.is_none() {
                payload.token_type = local_payload.token_type.clone();
            }
            if payload.expires_at.is_none() {
                payload.expires_at = local_payload.expires_at;
            }
            if payload.auth_raw.is_none() {
                payload.auth_raw = local_payload.auth_raw.clone();
            }
            if payload.profile_raw.is_none() {
                payload.profile_raw = local_payload.profile_raw.clone();
            }
            if payload.email.trim().is_empty() || payload.email == "unknown" {
                payload.email = local_payload.email.clone();
            }
            local_payload = payload;
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[WorkBuddy AI Import Local] 拉取账号资料失败，将保留本地导入结果：{}",
                err
            ));
        }
    }

    let mut account = workbuddy_ai_account::upsert_account(local_payload.clone())?;

    for existing in workbuddy_ai_account::list_accounts() {
        if existing.id == account.id {
            continue;
        }
        if existing.access_token != account.access_token {
            continue;
        }
        let is_placeholder = existing.email.trim().eq_ignore_ascii_case("unknown")
            || existing.email.trim().is_empty()
            || existing
                .uid
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true);
        if is_placeholder {
            if let Err(err) = workbuddy_ai_account::remove_account(&existing.id) {
                logger::log_warn(&format!(
                    "[WorkBuddy AI Import Local] 清理占位账号失败：id={}, error={}",
                    existing.id, err
                ));
            }
        }
    }

    account = refresh_workbuddy_ai_account_after_login(account).await;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_workbuddy_ai_accounts(account_ids: Vec<String>) -> Result<String, String> {
    workbuddy_ai_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_workbuddy_ai_token(
    app: AppHandle,
    account_id: String,
) -> Result<WorkbuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[WorkBuddy AI Command] 手动刷新账号开始：account_id={}",
        account_id
    ));

    match workbuddy_ai_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            if let Err(e) = workbuddy_ai_account::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[QuotaAlert][WorkBuddy AI] 预警检查失败：{}", e));
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[WorkBuddy AI Command] 手动刷新账号完成：account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[WorkBuddy AI Command] 手动刷新账号失败：account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_workbuddy_ai_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[WorkBuddy AI Command] 手动批量刷新开始");

    let results = workbuddy_ai_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);

    logger::log_info(&format!(
        "[WorkBuddy AI Command] 手动批量刷新完成：success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));

    if success_count > 0 {
        if let Err(e) = workbuddy_ai_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][WorkBuddy AI] 全量刷新后预警检查失败：{}",
                e
            ));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn workbuddy_ai_oauth_login_start() -> Result<WorkbuddyOAuthStartResponse, String> {
    logger::log_info("WorkBuddy AI OAuth start 命令触发");
    workbuddy_ai_oauth::start_login().await
}

#[tauri::command]
pub async fn workbuddy_ai_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<WorkbuddyAccount, String> {
    logger::log_info(&format!(
        "WorkBuddy AI OAuth complete 命令触发：login_id={}",
        login_id
    ));

    let result: Result<WorkbuddyAccount, String> = async {
        let payload = workbuddy_ai_oauth::complete_login(&login_id).await?;
        let mut account = workbuddy_ai_account::upsert_account(payload)?;
        account = refresh_workbuddy_ai_account_after_login(account).await;
        Ok(account)
    }
    .await;

    if let Err(err) = workbuddy_ai_oauth::clear_pending_oauth_login(&login_id) {
        logger::log_warn(&format!(
            "[WorkBuddy AI OAuth] 清理待处理登录状态失败：login_id={}, error={}",
            login_id, err
        ));
    }

    let account = result?;
    if let Err(err) = workbuddy_ai_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!(
            "[QuotaAlert][WorkBuddy AI] 登录后预警检查失败：{}",
            err
        ));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "WorkBuddy AI OAuth complete 成功：account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn workbuddy_ai_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "WorkBuddy AI OAuth cancel 命令触发：login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    workbuddy_ai_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn add_workbuddy_ai_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<WorkbuddyAccount, String> {
    let payload = workbuddy_ai_oauth::build_payload_from_token(&access_token).await?;
    let account = workbuddy_ai_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_workbuddy_ai_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WorkbuddyAccount, String> {
    workbuddy_ai_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_workbuddy_ai_accounts_index_path() -> Result<String, String> {
    workbuddy_ai_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_workbuddy_ai_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let account = workbuddy_ai_account::load_account(&account_id)
        .ok_or_else(|| format!("WorkBuddy AI account not found: {}", account_id))?;
    workbuddy_ai_account::set_current_account_id(Some(account_id.as_str()))?;
    workbuddy_ai_account::write_account_to_default_client(&account)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(format!("切换完成：{}", account.email))
}
