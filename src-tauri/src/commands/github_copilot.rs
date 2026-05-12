use tauri::{AppHandle, Emitter};

use crate::models::github_copilot::{GitHubCopilotAccount, GitHubCopilotOAuthStartResponse};
use crate::modules::{
    config, github_copilot_account, github_copilot_oauth, logger, opencode_auth, process,
};

async fn refresh_github_copilot_account_after_login(
    account: GitHubCopilotAccount,
) -> GitHubCopilotAccount {
    let account_id = account.id.clone();
    match github_copilot_account::refresh_account_token(&account_id).await {
        Ok(refreshed) => refreshed,
        Err(e) => {
            logger::log_warn(&format!(
                "[GitHub Copilot OAuth] 登录后自动刷新失败: account_id={}, error={}",
                account_id, e
            ));
            account
        }
    }
}

/// 列出所有 GitHub Copilot 账号
#[tauri::command]
pub fn list_github_copilot_accounts() -> Result<Vec<GitHubCopilotAccount>, String> {
    github_copilot_account::list_accounts_checked()
}

/// 删除 GitHub Copilot 账号
#[tauri::command]
pub fn delete_github_copilot_account(account_id: String) -> Result<(), String> {
    github_copilot_account::remove_account(&account_id)
}

/// 批量删除 GitHub Copilot 账号
#[tauri::command]
pub fn delete_github_copilot_accounts(account_ids: Vec<String>) -> Result<(), String> {
    github_copilot_account::remove_accounts(&account_ids)
}

/// 从 JSON 字符串导入 GitHub Copilot 账号
#[tauri::command]
pub fn import_github_copilot_from_json(
    json_content: String,
) -> Result<Vec<GitHubCopilotAccount>, String> {
    github_copilot_account::import_from_json(&json_content)
}

/// 从本机 VS Code 导入当前 GitHub Copilot 登录账号
#[tauri::command]
pub async fn import_github_copilot_from_local(
    app: AppHandle,
) -> Result<Vec<GitHubCopilotAccount>, String> {
    match github_copilot_account::import_from_local().await? {
        Some(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err("未找到本机 VS Code GitHub Copilot 登录信息".to_string()),
    }
}

/// 导出 GitHub Copilot 账号为 JSON
#[tauri::command]
pub fn export_github_copilot_accounts(account_ids: Vec<String>) -> Result<String, String> {
    github_copilot_account::export_accounts(&account_ids)
}

/// 刷新单个账号 Copilot token/配额信息（GitHub API）
#[tauri::command]
pub async fn refresh_github_copilot_token(
    app: AppHandle,
    account_id: String,
) -> Result<GitHubCopilotAccount, String> {
    let result = github_copilot_account::refresh_account_token(&account_id).await;
    if result.is_ok() {
        if let Err(e) = github_copilot_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!("[QuotaAlert][GitHubCopilot] 预警检查失败: {}", e));
        }
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

/// 刷新所有账号 Copilot token/配额信息（GitHub API）
#[tauri::command]
pub async fn refresh_all_github_copilot_tokens(app: AppHandle) -> Result<i32, String> {
    let results = github_copilot_account::refresh_all_tokens().await?;
    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    if success_count > 0 {
        if let Err(e) = github_copilot_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!(
                "[QuotaAlert][GitHubCopilot] 全量刷新后预警检查失败: {}",
                e
            ));
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

/// OAuth（Device Flow）：开始登录（返回 user_code + verification_uri 等）
#[tauri::command]
pub async fn github_copilot_oauth_login_start() -> Result<GitHubCopilotOAuthStartResponse, String> {
    logger::log_info("GitHub Copilot OAuth start 命令触发");
    let response = github_copilot_oauth::start_login().await?;
    logger::log_info(&format!(
        "GitHub Copilot OAuth start 命令成功: login_id={}",
        response.login_id
    ));
    Ok(response)
}

/// OAuth（Device Flow）：轮询并完成登录（返回保存后的账号）
#[tauri::command]
pub async fn github_copilot_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<GitHubCopilotAccount, String> {
    logger::log_info(&format!(
        "GitHub Copilot OAuth complete 命令触发: login_id={}",
        login_id
    ));
    let payload = github_copilot_oauth::complete_login(&login_id).await?;
    let account = github_copilot_account::upsert_account(payload)?;
    let account = refresh_github_copilot_account_after_login(account).await;
    logger::log_info(&format!(
        "GitHub Copilot OAuth complete 成功: account_id={}, login={}",
        account.id, account.github_login
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

/// OAuth（Device Flow）：取消登录（login_id 为空时取消当前流程）
#[tauri::command]
pub fn github_copilot_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "GitHub Copilot OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    github_copilot_oauth::cancel_login(login_id.as_deref())
}

/// 通过 GitHub access token 添加账号（会自动拉取 Copilot token/user 信息）
#[tauri::command]
pub async fn add_github_copilot_account_with_token(
    app: AppHandle,
    github_access_token: String,
) -> Result<GitHubCopilotAccount, String> {
    let payload =
        github_copilot_oauth::build_payload_from_github_access_token(&github_access_token).await?;
    let account = github_copilot_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

/// 更新账号标签
#[tauri::command]
pub async fn update_github_copilot_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<GitHubCopilotAccount, String> {
    github_copilot_account::update_account_tags(&account_id, tags)
}

/// 返回 GitHub Copilot 账号索引文件路径（便于排障/查看）
#[tauri::command]
pub fn get_github_copilot_accounts_index_path() -> Result<String, String> {
    github_copilot_account::accounts_index_path_string()
}

/// 切换 GitHub Copilot 账号并按默认实例启动流程生效（PID 精准关闭 + 注入 + 启动）。
#[tauri::command]
pub async fn inject_github_copilot_to_vscode(
    app: AppHandle,
    account_id: String,
) -> Result<String, String> {
    logger::log_info(&format!("开始切换 GitHub Copilot 账号: {}", account_id));
    let account = github_copilot_account::load_account(&account_id)
        .ok_or_else(|| format!("GitHub Copilot account not found: {}", account_id))?;
    logger::log_info(&format!(
        "正在切换到 GitHub Copilot 账号: {} (ID: {})",
        account.github_login, account.id
    ));

    // 同步更新 VS Code 默认实例绑定账号，确保后续走默认实例启动链路时注入目标明确。
    if let Err(e) = crate::modules::github_copilot_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        logger::log_warn(&format!("更新 GitHub Copilot 默认实例绑定账号失败: {}", e));
    } else {
        logger::log_info(&format!(
            "已同步更新 GitHub Copilot 默认实例绑定账号: {}",
            account_id
        ));
    }
    crate::modules::provider_current_state::set_current_account_id(
        "github_copilot",
        Some(account_id.as_str()),
    )?;

    let user_config = config::get_user_config();
    let mut opencode_updated = false;
    if user_config.ghcp_opencode_auth_overwrite_on_switch {
        match opencode_auth::replace_github_copilot_entry_from_account(&account) {
            Ok(()) => {
                opencode_updated = true;
            }
            Err(e) => {
                logger::log_warn(&format!("OpenCode auth.json 更新跳过: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 GitHub Copilot 时覆盖 OpenCode 登录信息");
    }

    if user_config.ghcp_opencode_sync_on_switch {
        if user_config.ghcp_opencode_auth_overwrite_on_switch && opencode_updated {
            if process::is_opencode_running() {
                if let Err(e) = process::close_opencode(20) {
                    logger::log_warn(&format!("OpenCode 关闭失败: {}", e));
                }
            } else {
                logger::log_info("OpenCode 未在运行，准备启动");
            }
            if let Err(e) = process::start_opencode_with_path(Some(&user_config.opencode_app_path))
            {
                logger::log_warn(&format!("OpenCode 启动失败: {}", e));
            }
        } else if !user_config.ghcp_opencode_auth_overwrite_on_switch {
            logger::log_info("OpenCode 登录覆盖已关闭，跳过自动重启");
        } else {
            logger::log_info("OpenCode 未更新 auth.json，跳过启动/重启");
        }
    } else {
        logger::log_info("已关闭 OpenCode 自动重启");
    }

    let launch_warning = if user_config.ghcp_launch_on_switch {
        match crate::commands::github_copilot_instance::github_copilot_start_instance(
            "__default__".to_string(),
        )
        .await
        {
            Ok(_) => None,
            Err(e) => {
                if e.starts_with("APP_PATH_NOT_FOUND:") || e.contains("启动 VS Code 失败") {
                    logger::log_warn(&format!("GitHub Copilot 默认实例启动失败: {}", e));
                    if e.starts_with("APP_PATH_NOT_FOUND:") || e.contains("APP_PATH_NOT_FOUND:") {
                        let _ = app.emit(
                            "app:path_missing",
                            serde_json::json!({ "app": "vscode", "retry": { "kind": "default" } }),
                        );
                    }
                    Some(e)
                } else {
                    return Err(e);
                }
            }
        }
    } else {
        logger::log_info("已关闭切换 GitHub Copilot 时自动启动 GitHub Copilot");
        None
    };

    logger::log_info(&format!(
        "GitHub Copilot 账号切换完成: {}",
        account.github_login
    ));
    let _ = crate::modules::tray::update_tray_menu(&app);
    let notification_note = launch_warning
        .as_ref()
        .map(|_| "账号已切换，但 VS Code 启动失败");
    let account_label = account
        .github_email
        .as_deref()
        .unwrap_or(account.github_login.as_str());
    crate::modules::wecom_switch_notify::notify_switch(
        "GitHub Copilot",
        &account.id,
        account_label,
        "manual",
        "tools.github_copilot.switch",
        notification_note,
    );
    if let Some(err) = launch_warning {
        Ok(format!("切换完成，但 VS Code 启动失败: {}", err))
    } else {
        Ok("切换完成".to_string())
    }
}
