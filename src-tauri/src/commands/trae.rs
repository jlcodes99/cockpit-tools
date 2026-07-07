use std::collections::BTreeMap;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::trae::{TraeAccount, TraeOAuthStartResponse};
use crate::modules::{logger, trae_account, trae_oauth};

fn resolve_trae_refresh_protection_map(
    accounts: &[TraeAccount],
) -> BTreeMap<String, Option<std::path::PathBuf>> {
    trae_account::resolve_running_account_refresh_protection_map(accounts)
}

#[tauri::command]
pub fn list_trae_accounts() -> Result<Vec<TraeAccount>, String> {
    trae_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_trae_account(account_id: String) -> Result<(), String> {
    trae_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_trae_accounts(account_ids: Vec<String>) -> Result<(), String> {
    trae_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_trae_from_json(json_content: String) -> Result<Vec<TraeAccount>, String> {
    trae_account::import_from_json(&json_content)
}

#[tauri::command]
pub async fn import_trae_from_local(
    app: AppHandle,
    platform_id: Option<String>,
) -> Result<Vec<TraeAccount>, String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    match trae_account::import_from_local_for_platform(platform)? {
        Some(mut account) => {
            match trae_account::refresh_account_async(account.id.as_str()).await {
                Ok(refreshed) => account = refreshed,
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Trae Import] 本地导入后自动刷新配额失败: {}",
                        err
                    ));
                }
            }
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err(format!("未找到本地 {} 登录信息", platform.display_name())),
    }
}

#[tauri::command]
pub async fn trae_oauth_login_start(
    platform_id: Option<String>,
) -> Result<TraeOAuthStartResponse, String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    logger::log_info(&format!(
        "[Trae OAuth] start 命令触发: platform={}",
        platform.provider_key()
    ));
    trae_oauth::start_login(platform).await
}

#[tauri::command]
pub async fn trae_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    platform_id: Option<String>,
) -> Result<TraeAccount, String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    logger::log_info(&format!(
        "[Trae OAuth] complete 命令触发: platform={}, login_id={}",
        platform.provider_key(),
        login_id
    ));

    let payload = trae_oauth::complete_login(login_id.as_str(), platform).await?;
    let mut account = trae_account::upsert_account(payload)?;

    match trae_account::refresh_account_async(account.id.as_str()).await {
        Ok(refreshed) => account = refreshed,
        Err(err) => {
            logger::log_warn(&format!("[Trae OAuth] 登录后自动刷新配额失败: {}", err));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn trae_oauth_login_cancel(
    login_id: Option<String>,
    platform_id: Option<String>,
) -> Result<(), String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    logger::log_info(&format!(
        "[Trae OAuth] cancel 命令触发: platform={}, login_id={}",
        platform.provider_key(),
        login_id.as_deref().unwrap_or("<none>")
    ));
    trae_oauth::cancel_login(login_id.as_deref(), platform)
}

#[tauri::command]
pub fn trae_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
    platform_id: Option<String>,
) -> Result<(), String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    trae_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str(), platform)
}

#[tauri::command]
pub fn export_trae_accounts(account_ids: Vec<String>) -> Result<String, String> {
    trae_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_trae_token(app: AppHandle, account_id: String) -> Result<TraeAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Command] 手动刷新账号开始: account_id={}",
        account_id
    ));

    if let Ok(accounts) = trae_account::list_accounts_checked() {
        let protection_map = resolve_trae_refresh_protection_map(&accounts);
        if let Some(storage_path) = protection_map.get(account_id.as_str()) {
            logger::log_info(&format!(
                "[Trae Command] 命中运行中实例账号，改为仅额度刷新: account_id={}, storage_path={}",
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            let result = trae_account::refresh_account_usage_only_async(
                &account_id,
                storage_path.as_deref(),
            )
            .await;
            match &result {
                Ok(account) => {
                    let _ = crate::modules::tray::update_tray_menu(&app);
                    logger::log_info(&format!(
                        "[Trae Command] 仅额度刷新完成: account_id={}, email={}, elapsed={}ms",
                        account.id,
                        account.email,
                        started_at.elapsed().as_millis()
                    ));
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Trae Command] 仅额度刷新失败: account_id={}, elapsed={}ms, error={}",
                        account_id,
                        started_at.elapsed().as_millis(),
                        err
                    ));
                }
            }
            return result;
        }
    }

    match trae_account::refresh_account_async(&account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Trae Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Trae Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_trae_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Trae Command] 批量刷新开始");

    let accounts = trae_account::list_accounts_checked()?;
    let protection_map = resolve_trae_refresh_protection_map(&accounts);
    let mut success_count = 0;

    for account in accounts {
        if let Some(storage_path) = protection_map.get(account.id.as_str()) {
            logger::log_info(&format!(
                "[Trae Command] 批量刷新命中运行中实例账号，改为仅额度刷新: account_id={}, storage_path={}",
                account.id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            match trae_account::refresh_account_usage_only_async(
                account.id.as_str(),
                storage_path.as_deref(),
            )
            .await
            {
                Ok(_) => {
                    success_count += 1;
                }
                Err(err) => {
                    logger::log_warn(&format!(
                        "[Trae Command] 批量仅额度刷新失败: account_id={}, error={}",
                        account.id, err
                    ));
                }
            }
            continue;
        }

        match trae_account::refresh_account_async(account.id.as_str()).await {
            Ok(_) => {
                success_count += 1;
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "[Trae Command] 批量刷新失败: account_id={}, error={}",
                    account.id, err
                ));
            }
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Trae Command] 批量刷新完成: success={}, elapsed={}ms",
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub async fn refresh_trae_tokens_for_platform(
    app: AppHandle,
    platform_id: Option<String>,
) -> Result<i32, String> {
    let started_at = Instant::now();
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    logger::log_info(&format!(
        "[Trae Command] platform batch refresh start: platform={}",
        platform.provider_key()
    ));

    let results = trae_account::refresh_tokens_for_platform(platform).await?;
    let success_count = results.iter().filter(|(_, result)| result.is_ok()).count();
    for (account_id, result) in results {
        if let Err(err) = result {
            logger::log_warn(&format!(
                "[Trae Command] platform batch refresh failed: platform={}, account_id={}, error={}",
                platform.provider_key(),
                account_id,
                err
            ));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);

    logger::log_info(&format!(
        "[Trae Command] platform batch refresh done: platform={}, success={}, elapsed={}ms",
        platform.provider_key(),
        success_count,
        started_at.elapsed().as_millis()
    ));
    Ok(success_count as i32)
}

#[tauri::command]
pub fn add_trae_account_with_token(
    app: AppHandle,
    access_token: String,
) -> Result<TraeAccount, String> {
    let payload = crate::models::trae::TraeImportPayload {
        email: "unknown".to_string(),
        user_id: None,
        nickname: None,
        access_token,
        refresh_token: None,
        token_type: None,
        expires_at: None,
        plan_type: None,
        plan_reset_at: None,
        trae_auth_raw: None,
        trae_profile_raw: None,
        trae_entitlement_raw: None,
        trae_usage_raw: None,
        trae_server_raw: None,
        trae_usertag_raw: None,
        status: None,
        status_reason: None,
    };
    let account = trae_account::upsert_account(payload)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn update_trae_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<TraeAccount, String> {
    trae_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_trae_accounts_index_path() -> Result<String, String> {
    trae_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn inject_trae_account(
    app: AppHandle,
    account_id: String,
    platform_id: Option<String>,
) -> Result<String, String> {
    let platform = trae_account::TraePlatformKind::parse(platform_id.as_deref())?;
    let platform_key = platform.provider_key();
    let platform_label = platform.display_name();
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Trae Switch] 开始切换账号: platform={}, account_id={}",
        platform_key, account_id
    ));

    let existing = trae_account::load_account(&account_id)
        .ok_or_else(|| format!("Trae account not found: {}", account_id))?;
    logger::log_info(&format!(
        "[Trae Switch] 切号前刷新账号: account_id={}, email={}",
        existing.id, existing.email
    ));
    let account = if let Ok(accounts) = trae_account::list_accounts_checked() {
        let protection_map = resolve_trae_refresh_protection_map(&accounts);
        if let Some(storage_path) = protection_map.get(account_id.as_str()) {
            logger::log_info(&format!(
                "[Trae Switch] 切号前命中运行中账号保护，改为仅额度刷新: platform={}, account_id={}, storage_path={}",
                platform_key,
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            trae_account::refresh_account_usage_only_async(&account_id, storage_path.as_deref())
                .await
                .map_err(|err| format!("Trae 切号前刷新失败: {}", err))?
        } else {
            trae_account::refresh_account_async(&account_id)
                .await
                .map_err(|err| format!("Trae 切号前刷新失败: {}", err))?
        }
    } else {
        trae_account::refresh_account_async(&account_id)
            .await
            .map_err(|err| format!("Trae 切号前刷新失败: {}", err))?
    };

    if let Err(err) = crate::modules::process::close_trae_platform_default(platform_key, 20) {
        logger::log_warn(&format!(
            "[Trae Switch] 关闭 {} 旧进程失败，切号中止: {}",
            platform_label, err
        ));
        return Err(format!(
            "{} 正在运行且未能正常关闭（{}）。请先关闭 {} 后重试切号。",
            platform_label, err, platform_label
        ));
    }

    trae_account::inject_to_trae_for_platform(platform, &account_id)?;
    crate::modules::provider_current_state::set_current_account_id(
        platform_key,
        Some(account_id.as_str()),
    )?;

    let launch_warning = {
        if let Err(err) = crate::modules::trae_instance::update_default_settings_for_platform(
            platform,
            Some(Some(account_id.clone())),
            None,
            Some(false),
        ) {
            logger::log_warn(&format!(
                "更新 {} 默认实例绑定账号失败: {}",
                platform_label, err
            ));
        }

        match crate::commands::trae_instance::trae_start_instance(
            Some(platform_key.to_string()),
            "__default__".to_string(),
        )
        .await
        {
            Ok(_) => None,
            Err(err) => {
                if err.starts_with("APP_PATH_NOT_FOUND:")
                    || err.contains("启动 Trae 失败")
                    || (err.contains("启动 ") && err.contains("失败"))
                {
                    logger::log_warn(&format!("{} 默认实例启动失败: {}", platform_label, err));
                    if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("APP_PATH_NOT_FOUND:")
                    {
                        let _ = app.emit(
                            "app:path_missing",
                            serde_json::json!({ "app": platform_key, "retry": { "kind": "default" } }),
                        );
                    }
                    Some(err)
                } else {
                    return Err(err);
                }
            }
        }
    };

    let _ = crate::modules::tray::update_tray_menu(&app);

    if let Some(err) = launch_warning {
        logger::log_warn(&format!(
            "[Trae Switch] 切号完成但启动失败: platform={}, account_id={}, email={}, elapsed={}ms, error={}",
            platform_key,
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 {} 启动失败: {}", platform_label, err))
    } else {
        logger::log_info(&format!(
            "[Trae Switch] 切号成功: platform={}, account_id={}, email={}, elapsed={}ms",
            platform_key,
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.email))
    }
}
