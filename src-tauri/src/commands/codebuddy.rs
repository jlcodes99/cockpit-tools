use std::time::Instant;

use tauri::{AppHandle, Emitter};

use crate::models::codebuddy::{CodebuddyAccount, CodebuddyOAuthStartResponse};
use crate::modules::{codebuddy_account, codebuddy_oauth, logger};

async fn refresh_codebuddy_account_after_login(account: CodebuddyAccount) -> CodebuddyAccount {
    let account_id = account.id.clone();
    match codebuddy_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[CodeBuddy OAuth] 登录后刷新失败，保留原账号信息: account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

#[tauri::command]
pub fn list_codebuddy_accounts() -> Result<Vec<CodebuddyAccount>, String> {
    codebuddy_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_codebuddy_account(account_id: String) -> Result<(), String> {
    codebuddy_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_codebuddy_accounts(account_ids: Vec<String>) -> Result<(), String> {
    codebuddy_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_codebuddy_from_json(json_content: String) -> Result<Vec<CodebuddyAccount>, String> {
    codebuddy_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_codebuddy_from_local(app: AppHandle) -> Result<Vec<CodebuddyAccount>, String> {
    let mut local_payload = match codebuddy_account::import_payload_from_local()? {
        Some(payload) => payload,
        None => return Err("未在本机 CodeBuddy 客户端中找到登录信息".to_string()),
    };

    match codebuddy_oauth::build_payload_from_token(&local_payload.access_token).await {
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
                "[CodeBuddy Import Local] 拉取账号资料失败，将保留本地导入结果: {}",
                err
            ));
        }
    }

    let mut account = codebuddy_account::upsert_account(local_payload.clone())?;

    // 历史版本本地导入会先写入 unknown 占位账号；这里按同 token 清理旧占位记录。
    for existing in codebuddy_account::list_accounts() {
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
            if let Err(err) = codebuddy_account::remove_account(&existing.id) {
                logger::log_warn(&format!(
                    "[CodeBuddy Import Local] 清理占位账号失败: id={}, error={}",
                    existing.id, err
                ));
            }
        }
    }

    account = refresh_codebuddy_account_after_login(account).await;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_codebuddy_accounts(account_ids: Vec<String>) -> Result<String, String> {
    codebuddy_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_codebuddy_token(
    app: AppHandle,
    account_id: String,
) -> Result<CodebuddyAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    match codebuddy_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            if let Err(e) = codebuddy_account::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[QuotaAlert][CodeBuddy] 预警检查失败: {}", e));
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[CodeBuddy Command] 手动刷新账号完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[CodeBuddy Command] 手动刷新账号失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_codebuddy_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[CodeBuddy Command] 手动批量刷新开始");

    let results = codebuddy_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, item)| item.is_ok()).count();
    let failed_count = results.len().saturating_sub(success_count);

    logger::log_info(&format!(
        "[CodeBuddy Command] 手动批量刷新完成: success={}, failed={}, elapsed={}ms",
        success_count,
        failed_count,
        started_at.elapsed().as_millis()
    ));

    if success_count > 0 {
        if let Err(e) = codebuddy_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][CodeBuddy] 全量刷新后预警检查失败: {}",
                e
            ));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn codebuddy_oauth_login_start() -> Result<CodebuddyOAuthStartResponse, String> {
    logger::log_info("CodeBuddy OAuth start 命令触发");
    codebuddy_oauth::start_login().await
}

#[tauri::command]
pub async fn codebuddy_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<CodebuddyAccount, String> {
    logger::log_info(&format!(
        "CodeBuddy OAuth complete 命令触发: login_id={}",
        login_id
    ));

    let result: Result<CodebuddyAccount, String> = async {
        let payload = codebuddy_oauth::complete_login(&login_id).await?;
        let mut account = codebuddy_account::upsert_account(payload)?;
        account = refresh_codebuddy_account_after_login(account).await;
        Ok(account)
    }
    .await;

    if let Err(err) = codebuddy_oauth::clear_pending_oauth_login(&login_id) {
        logger::log_warn(&format!(
            "[CodeBuddy OAuth] 清理待处理登录状态失败: login_id={}, error={}",
            login_id, err
        ));
    }

    let account = result?;
    if let Err(err) = codebuddy_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!(
            "[QuotaAlert][CodeBuddy] 登录后预警检查失败: {}",
            err
        ));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "CodeBuddy OAuth complete 成功: account_id={}, email={}",
        account.id, account.email
    ));
    Ok(account)
}

#[tauri::command]
pub fn codebuddy_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "CodeBuddy OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    codebuddy_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn add_codebuddy_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<CodebuddyAccount, String> {
    let payload = codebuddy_oauth::build_payload_from_token(&access_token).await?;
    let account = codebuddy_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_codebuddy_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CodebuddyAccount, String> {
    codebuddy_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_codebuddy_accounts_index_path() -> Result<String, String> {
    codebuddy_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_codebuddy_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[CodeBuddy Switch] 开始切换账号: account_id={}",
        account_id
    ));

    let account = codebuddy_account::load_account(&account_id)
        .ok_or_else(|| format!("CodeBuddy account not found: {}", account_id))?;

    crate::modules::codebuddy_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    )?;
    crate::modules::provider_current_state::set_current_account_id(
        "codebuddy",
        Some(account_id.as_str()),
    )?;

    let launch_warning =
        match crate::commands::codebuddy_instance::codebuddy_switch_default_account_and_start()
            .await
        {
            Ok(_) => None,
            Err(err) => {
                if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 CodeBuddy 失败")
                {
                    logger::log_warn(&format!("CodeBuddy 默认实例启动失败: {}", err));
                    if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("APP_PATH_NOT_FOUND:")
                    {
                        let _ = app.emit(
                            "app:path_missing",
                            serde_json::json!({ "app": "codebuddy", "retry": { "kind": "default" } }),
                        );
                    }
                    Some(err)
                } else {
                    return Err(err);
                }
            }
        };

    let _ = crate::modules::tray::update_tray_menu(&app);

    if let Some(err) = launch_warning {
        logger::log_warn(&format!(
            "[CodeBuddy Switch] 切号完成但启动失败: account_id={}, email={}, elapsed={}ms, error={}",
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 CodeBuddy 启动失败: {}", err))
    } else {
        logger::log_info(&format!(
            "[CodeBuddy Switch] 切号成功: account_id={}, email={}, elapsed={}ms",
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.email))
    }
}
