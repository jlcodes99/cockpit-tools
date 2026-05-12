use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::windsurf::{WindsurfAccount, WindsurfOAuthStartResponse};
use crate::modules::{logger, windsurf_account, windsurf_oauth};

#[derive(Debug, Deserialize)]
pub struct WindsurfPasswordCredentialInput {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub source_line: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct WindsurfPasswordCredentialFailure {
    pub email: String,
    pub error: String,
    pub source_line: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct WindsurfPasswordBatchResult {
    pub accounts: Vec<WindsurfAccount>,
    pub success_count: usize,
    pub failed_count: usize,
    pub failures: Vec<WindsurfPasswordCredentialFailure>,
}

async fn refresh_windsurf_account_after_login(account: WindsurfAccount) -> WindsurfAccount {
    let account_id = account.id.clone();
    match windsurf_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[Windsurf OAuth] 登录后自动刷新失败: account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

async fn add_windsurf_account_from_password(
    email: &str,
    password: &str,
) -> Result<WindsurfAccount, String> {
    let payload = windsurf_oauth::build_payload_from_password(email, password).await?;
    windsurf_account::upsert_account(payload)
}

#[tauri::command]
pub fn list_windsurf_accounts() -> Result<Vec<WindsurfAccount>, String> {
    windsurf_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_windsurf_account(account_id: String) -> Result<(), String> {
    windsurf_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_windsurf_accounts(account_ids: Vec<String>) -> Result<(), String> {
    windsurf_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_windsurf_from_json(json_content: String) -> Result<Vec<WindsurfAccount>, String> {
    windsurf_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_windsurf_from_local(app: AppHandle) -> Result<Vec<WindsurfAccount>, String> {
    let auth_status = windsurf_account::read_local_auth_status()?.ok_or_else(|| {
        "未在本机 Windsurf 客户端中找到登录信息（windsurfAuthStatus）".to_string()
    })?;
    let mut payload = windsurf_oauth::build_payload_from_local_auth_status(auth_status).await?;
    if payload.github_login.trim().is_empty() {
        if let Some(hint) = windsurf_account::read_local_login_hint() {
            payload.github_login = hint;
        }
    }
    let account = windsurf_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(vec![account])
}

#[tauri::command]
pub fn export_windsurf_accounts(account_ids: Vec<String>) -> Result<String, String> {
    windsurf_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_windsurf_token(
    app: AppHandle,
    account_id: String,
) -> Result<WindsurfAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Windsurf Command] 手动刷新账号开始: account_id={}",
        account_id
    ));
    match windsurf_account::refresh_account_token(&account_id).await {
        Ok(account) => {
            if let Err(e) = windsurf_account::run_quota_alert_if_needed() {
                logger::log_warn(&format!("[QuotaAlert][Windsurf] 预警检查失败: {}", e));
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Windsurf Command] 手动刷新账号完成: account_id={}, login={}, elapsed={}ms",
                account.id,
                account.github_login,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Windsurf Command] 手动刷新账号失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_windsurf_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Windsurf Command] 手动批量刷新开始");
    let results = windsurf_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed: Vec<String> = results
        .iter()
        .filter_map(|(id, result)| result.as_ref().err().map(|err| format!("{}:{}", id, err)))
        .collect();
    if failed.is_empty() {
        logger::log_info(&format!(
            "[Windsurf Command] 手动批量刷新完成: success={}, elapsed={}ms",
            success_count,
            started_at.elapsed().as_millis()
        ));
    } else {
        logger::log_warn(&format!(
            "[Windsurf Command] 手动批量刷新完成(部分失败): success={}, failed={}, elapsed={}ms, details={}",
            success_count,
            failed.len(),
            started_at.elapsed().as_millis(),
            failed.join(" | ")
        ));
    }
    if success_count > 0 {
        if let Err(e) = windsurf_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][Windsurf] 全量刷新后预警检查失败: {}",
                e
            ));
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn windsurf_oauth_login_start() -> Result<WindsurfOAuthStartResponse, String> {
    logger::log_info("Windsurf OAuth start 命令触发");
    windsurf_oauth::start_login().await
}

#[tauri::command]
pub async fn windsurf_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<WindsurfAccount, String> {
    logger::log_info(&format!(
        "Windsurf OAuth complete 命令触发: login_id={}",
        login_id
    ));
    let payload = windsurf_oauth::complete_login(&login_id).await?;
    let account = windsurf_account::upsert_account(payload)?;
    let account = refresh_windsurf_account_after_login(account).await;
    logger::log_info(&format!(
        "Windsurf OAuth complete 成功: account_id={}, login={}",
        account.id, account.github_login
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn windsurf_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Windsurf OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    windsurf_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn windsurf_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    windsurf_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub async fn add_windsurf_account_with_token(
    app: AppHandle,
    github_access_token: String,
) -> Result<WindsurfAccount, String> {
    let payload = windsurf_oauth::build_payload_from_token(&github_access_token).await?;
    let account = windsurf_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn add_windsurf_account_with_password(
    app: AppHandle,
    email: String,
    password: String,
) -> Result<WindsurfAccount, String> {
    logger::log_info("[Windsurf Command] 邮箱密码登录开始");
    let account = add_windsurf_account_from_password(&email, &password).await?;
    logger::log_info(&format!(
        "[Windsurf Command] 邮箱密码登录成功: account_id={}, login={}",
        account.id, account.github_login
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn add_windsurf_accounts_with_password(
    app: AppHandle,
    credentials: Vec<WindsurfPasswordCredentialInput>,
) -> Result<WindsurfPasswordBatchResult, String> {
    if credentials.is_empty() {
        return Err("请先提供至少一组邮箱和密码".to_string());
    }

    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Windsurf Command] 批量邮箱密码登录开始: count={}",
        credentials.len()
    ));

    let mut accounts = Vec::new();
    let mut failures = Vec::new();

    for item in credentials {
        let email = item.email.trim().to_string();
        let password = item.password;
        if email.is_empty() || password.is_empty() {
            failures.push(WindsurfPasswordCredentialFailure {
                email,
                error: "邮箱和密码不能为空".to_string(),
                source_line: item.source_line,
            });
            continue;
        }

        match add_windsurf_account_from_password(&email, &password).await {
            Ok(account) => {
                logger::log_info(&format!(
                    "[Windsurf Command] 批量邮箱密码登录成功: account_id={}, login={}",
                    account.id, account.github_login
                ));
                accounts.push(account);
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "[Windsurf Command] 批量邮箱密码登录失败: email={}, error={}",
                    email, error
                ));
                failures.push(WindsurfPasswordCredentialFailure {
                    email,
                    error,
                    source_line: item.source_line,
                });
            }
        }
    }

    if !accounts.is_empty() {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }

    let success_count = accounts.len();
    let failed_count = failures.len();
    if failed_count == 0 {
        logger::log_info(&format!(
            "[Windsurf Command] 批量邮箱密码登录完成: success={}, elapsed={}ms",
            success_count,
            started_at.elapsed().as_millis()
        ));
    } else {
        logger::log_warn(&format!(
            "[Windsurf Command] 批量邮箱密码登录完成(部分失败): success={}, failed={}, elapsed={}ms",
            success_count,
            failed_count,
            started_at.elapsed().as_millis()
        ));
    }

    Ok(WindsurfPasswordBatchResult {
        accounts,
        success_count,
        failed_count,
        failures,
    })
}

#[tauri::command]
pub async fn update_windsurf_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WindsurfAccount, String> {
    windsurf_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_windsurf_accounts_index_path() -> Result<String, String> {
    windsurf_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_windsurf_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Windsurf Switch] 开始切换账号: account_id={}",
        account_id
    ));
    let account = windsurf_account::load_account(&account_id)
        .ok_or_else(|| format!("Windsurf account not found: {}", account_id))?;
    logger::log_info(&format!(
        "[Windsurf Switch] 目标账号信息: login={}, email={}",
        account.github_login,
        account.github_email.as_deref().unwrap_or("-")
    ));

    if let Err(e) = crate::modules::windsurf_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        logger::log_warn(&format!("更新 Windsurf 默认实例绑定账号失败: {}", e));
    }
    crate::modules::provider_current_state::set_current_account_id(
        "windsurf",
        Some(account_id.as_str()),
    )?;

    let launch_warning = match crate::commands::windsurf_instance::windsurf_start_instance(
        "__default__".to_string(),
    )
    .await
    {
        Ok(_) => None,
        Err(e) => {
            if e.starts_with("APP_PATH_NOT_FOUND:") || e.contains("启动 Windsurf 失败") {
                logger::log_warn(&format!("Windsurf 默认实例启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") || e.contains("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "windsurf", "retry": { "kind": "default" } }),
                    );
                }
                Some(e)
            } else {
                return Err(e);
            }
        }
    };

    if let Some(err) = launch_warning {
        let _ = crate::modules::tray::update_tray_menu(&app);
        let account_label = account
            .github_email
            .as_deref()
            .unwrap_or(account.github_login.as_str());
        crate::modules::wecom_switch_notify::notify_switch(
            "Windsurf",
            &account.id,
            account_label,
            "manual",
            "tools.windsurf.switch",
            Some("账号已切换，但 Windsurf 启动失败"),
        );
        logger::log_warn(&format!(
            "[Windsurf Switch] 切号完成但启动失败: account_id={}, login={}, elapsed={}ms, error={}",
            account.id,
            account.github_login,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 Windsurf 启动失败: {}", err))
    } else {
        let _ = crate::modules::tray::update_tray_menu(&app);
        let account_label = account
            .github_email
            .as_deref()
            .unwrap_or(account.github_login.as_str());
        crate::modules::wecom_switch_notify::notify_switch(
            "Windsurf",
            &account.id,
            account_label,
            "manual",
            "tools.windsurf.switch",
            None,
        );
        logger::log_info(&format!(
            "[Windsurf Switch] 切号成功: account_id={}, login={}, elapsed={}ms",
            account.id,
            account.github_login,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.github_login))
    }
}
