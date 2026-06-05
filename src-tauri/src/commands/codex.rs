use crate::models::codex::{
    CodexAccount, CodexApiProviderMode, CodexAppSpeed, CodexAppSpeedConfig, CodexQuickConfig,
    CodexQuota, CodexTokens,
};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountModelRule, CodexLocalAccessChatMessage, CodexLocalAccessChatResult,
    CodexLocalAccessClientBaseUrlHost, CodexLocalAccessCustomRoutingRule,
    CodexLocalAccessGatewayMode, CodexLocalAccessModelAlias, CodexLocalAccessModelPricing,
    CodexLocalAccessPortCleanupResult, CodexLocalAccessRequestKind,
    CodexLocalAccessRoutingStrategy, CodexLocalAccessScope, CodexLocalAccessState,
    CodexLocalAccessTestFailure, CodexLocalAccessTestResult, CodexLocalAccessTimeoutPreset,
    CodexLocalAccessTimeouts, CodexLocalAccessUsageEventPage,
};
use crate::modules::{
    account, codex_account, codex_local_access, codex_oauth, codex_quota, codex_speed,
    codex_wakeup, codex_wakeup_scheduler, config, logger, openclaw_auth, opencode_auth, process,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

static CODEX_POST_REFRESH_CHECK_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

fn restart_codex_specified_app_if_enabled(user_config: &config::UserConfig) {
    if !user_config.codex_restart_specified_app_on_switch {
        logger::log_info("已关闭切换 Codex 时自动重启指定应用");
        return;
    }

    let path = user_config.codex_specified_app_path.trim();
    if path.is_empty() {
        logger::log_warn("已开启切换 Codex 时自动重启指定应用，但未配置应用路径，已跳过");
        return;
    }

    match process::restart_specified_app_by_path(path, 20) {
        Ok(()) => {
            logger::log_info(&format!("已重启指定应用: {}", path));
        }
        Err(error) => {
            logger::log_warn(&format!("重启指定应用失败（path={}）：{}", path, error));
        }
    }
}

/// 列出所有 Codex 账号
#[tauri::command]
pub fn list_codex_accounts() -> Result<Vec<CodexAccount>, String> {
    codex_account::list_accounts_checked()
}

/// 获取当前激活的 Codex 账号
#[tauri::command]
pub fn get_current_codex_account() -> Result<Option<CodexAccount>, String> {
    Ok(codex_account::get_current_account())
}

#[tauri::command]
pub fn get_codex_config_toml_path() -> Result<String, String> {
    let path = codex_account::get_codex_home().join("config.toml");
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_codex_config_toml(app: AppHandle) -> Result<(), String> {
    let path = codex_account::get_codex_home().join("config.toml");
    if !path.exists() {
        return Err(format!("未找到 Codex config.toml 文件: {}", path.display()));
    }

    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开 Codex config.toml 失败: {}", e))
}

#[tauri::command]
pub fn get_codex_quick_config() -> Result<CodexQuickConfig, String> {
    codex_account::load_current_quick_config()
}

#[tauri::command]
pub fn save_codex_quick_config(
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    codex_account::save_current_quick_config(model_context_window, auto_compact_token_limit)
}

#[tauri::command]
pub fn get_codex_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    codex_speed::get_app_speed_config()
}

#[tauri::command]
pub fn save_codex_app_speed(speed: CodexAppSpeed) -> Result<CodexAppSpeedConfig, String> {
    codex_speed::save_api_service_app_speed(speed)
}

#[tauri::command]
pub fn get_codex_api_service_app_speed_config() -> Result<CodexAppSpeedConfig, String> {
    codex_speed::get_api_service_app_speed_config()
}

#[tauri::command]
pub fn save_codex_api_service_app_speed(
    speed: CodexAppSpeed,
) -> Result<CodexAppSpeedConfig, String> {
    let saved = codex_speed::save_api_service_app_speed(speed.clone())?;
    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if settings.bind_account_id.as_deref()
            == Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID)
        {
            let _ = crate::modules::codex_instance::update_default_app_speed(speed);
        }
    }
    Ok(saved)
}

#[tauri::command]
pub fn update_codex_account_app_speed(
    account_id: String,
    speed: CodexAppSpeed,
) -> Result<CodexAccount, String> {
    let account = codex_account::update_account_app_speed(&account_id, speed)?;
    let account_speed = account.app_speed.clone();
    let current_account_id = codex_account::load_account_index().current_account_id;
    let default_bind_account_id = crate::modules::codex_instance::load_default_settings()
        .ok()
        .and_then(|settings| settings.bind_account_id);
    if current_account_id.as_deref() == Some(account_id.as_str())
        || default_bind_account_id.as_deref() == Some(account_id.as_str())
    {
        codex_speed::write_official_app_speed(account_speed.clone())?;
        let _ = crate::modules::codex_instance::update_default_app_speed(account_speed.clone());
    }

    let bound_instances = crate::modules::codex_instance::update_bound_instances_app_speed(
        &account_id,
        account_speed.clone(),
    )?;
    for instance in bound_instances {
        codex_speed::write_app_speed_for_dir(
            std::path::Path::new(&instance.user_data_dir),
            account_speed.clone(),
        )?;
    }
    Ok(account)
}

/// 刷新账号资料（团队名/结构）
#[tauri::command]
pub async fn refresh_codex_account_profile(account_id: String) -> Result<CodexAccount, String> {
    codex_account::refresh_account_profile(&account_id).await
}

/// 切换 Codex 账号（包含 token 刷新检查）
#[tauri::command]
pub async fn switch_codex_account(
    app: AppHandle,
    account_id: String,
) -> Result<CodexAccount, String> {
    // 切换账号（写入 auth.json）
    let account = codex_account::switch_account_managed(&account_id).await?;
    let account_speed = account.app_speed.clone();
    codex_speed::write_official_app_speed(account_speed.clone())?;

    // 同步更新 Codex 默认实例的绑定账号（不同步到 Antigravity，因为账号体系不同）
    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
        None,
        None,
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例绑定账号失败: {}", e));
    } else {
        logger::log_info(&format!(
            "已同步更新 Codex 默认实例绑定账号: {}",
            account_id
        ));
    }
    if let Err(e) = crate::modules::codex_instance::update_default_app_speed(account_speed) {
        logger::log_warn(&format!("更新 Codex 默认实例速度失败: {}", e));
    }

    let user_config = config::get_user_config();

    let mut opencode_updated = false;
    if user_config.opencode_auth_overwrite_on_switch {
        match opencode_auth::replace_openai_entry_from_codex(&account) {
            Ok(()) => {
                opencode_updated = true;
            }
            Err(e) => {
                logger::log_warn(&format!("OpenCode auth.json 更新跳过: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 OpenCode 登录信息");
    }

    if user_config.opencode_sync_on_switch {
        if user_config.opencode_auth_overwrite_on_switch && opencode_updated {
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
        } else if !user_config.opencode_auth_overwrite_on_switch {
            logger::log_info("OpenCode 登录覆盖已关闭，跳过自动重启");
        } else {
            logger::log_info("OpenCode 未更新 auth.json，跳过启动/重启");
        }
    } else {
        logger::log_info("已关闭 OpenCode 自动重启");
    }

    if user_config.openclaw_auth_overwrite_on_switch {
        match openclaw_auth::replace_openai_codex_entry_from_codex(&account) {
            Ok(()) => {}
            Err(e) => {
                logger::log_warn(&format!("OpenClaw auth 同步失败: {}", e));
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时覆盖 OpenClaw 登录信息");
    }

    if user_config.codex_launch_on_switch {
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        match crate::commands::codex_instance::codex_start_default_with_prepared_profile().await {
            Ok(_) => {}
            Err(e) => {
                logger::log_warn(&format!("Codex 启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                    );
                }
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
    }

    restart_codex_specified_app_if_enabled(&user_config);

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

async fn run_codex_post_refresh_checks(app: &AppHandle) {
    if CODEX_POST_REFRESH_CHECK_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        logger::log_info("[AutoSwitch][Codex] 后置检查进行中，跳过本次执行");
        return;
    }

    let mut switched = false;

    match codex_account::pick_auto_switch_target_if_needed() {
        Ok(Some(target)) => {
            let target_id = target.id.clone();
            match switch_codex_account(app.clone(), target_id.clone()).await {
                Ok(switched_account) => {
                    logger::log_info(&format!(
                        "[AutoSwitch][Codex] 自动切号完成: target_id={}, email={}",
                        switched_account.id, switched_account.email
                    ));
                    switched = true;
                }
                Err(e) => {
                    logger::log_warn(&format!(
                        "[AutoSwitch][Codex] 自动切号失败: target_id={}, error={}",
                        target_id, e
                    ));
                }
            }
        }
        Ok(None) => {}
        Err(e) => {
            logger::log_warn(&format!("[AutoSwitch][Codex] 自动切号检查失败: {}", e));
        }
    }

    if !switched {
        if let Err(e) = codex_account::run_quota_alert_if_needed() {
            logger::log_warn(&format!("[QuotaAlert][Codex] 预警检查失败: {}", e));
        }
    }

    CODEX_POST_REFRESH_CHECK_IN_PROGRESS.store(false, Ordering::SeqCst);
}

/// 删除 Codex 账号
#[tauri::command]
pub async fn delete_codex_account(account_id: String) -> Result<(), String> {
    codex_account::remove_account(&account_id)?;
    codex_local_access::remove_local_access_accounts(&[account_id]).await?;
    Ok(())
}

/// 批量删除 Codex 账号
#[tauri::command]
pub async fn delete_codex_accounts(account_ids: Vec<String>) -> Result<(), String> {
    codex_account::remove_accounts(&account_ids)?;
    codex_local_access::remove_local_access_accounts(&account_ids).await?;
    Ok(())
}

async fn refresh_imported_codex_accounts(
    app: &AppHandle,
    accounts: Vec<CodexAccount>,
) -> Vec<CodexAccount> {
    let mut result = Vec::with_capacity(accounts.len());
    let mut success_count = 0;
    let mut attempted = false;

    for account in accounts {
        if account.is_api_key_auth() {
            result.push(account);
            continue;
        }

        attempted = true;
        match codex_quota::refresh_account_quota(&account.id).await {
            Ok(_) => {
                success_count += 1;
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "Codex 导入后刷新配额失败: account_id={}, email={}, error={}",
                    account.id, account.email, error
                ));
            }
        }

        result.push(codex_account::load_account(&account.id).unwrap_or(account));
    }

    if success_count > 0 {
        run_codex_post_refresh_checks(app).await;
    }
    if attempted || !result.is_empty() {
        let _ = crate::modules::tray::update_tray_menu(app);
    }

    result
}

/// 从本地 auth.json 导入账号
#[tauri::command]
pub async fn import_codex_from_local(app: AppHandle) -> Result<CodexAccount, String> {
    let account = codex_account::import_from_local()?;
    let mut accounts = refresh_imported_codex_accounts(&app, vec![account]).await;
    accounts
        .pop()
        .ok_or_else(|| "账号导入后无法读取".to_string())
}

/// 从 JSON 字符串导入账号
#[tauri::command]
pub async fn import_codex_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<CodexAccount>, String> {
    let accounts = codex_account::import_from_json(&json_content).await?;
    Ok(refresh_imported_codex_accounts(&app, accounts).await)
}

/// 导出 Codex 账号
#[tauri::command]
pub fn export_codex_accounts(account_ids: Vec<String>) -> Result<String, String> {
    codex_account::export_accounts(&account_ids)
}

/// 从本地文件导入 Codex 账号
#[tauri::command]
pub async fn import_codex_from_files(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<codex_account::CodexFileImportResult, String> {
    let result = codex_account::import_from_files(file_paths).await?;
    let imported = refresh_imported_codex_accounts(&app, result.imported).await;
    Ok(codex_account::CodexFileImportResult {
        imported,
        failed: result.failed,
    })
}

/// 刷新单个账号配额
#[tauri::command]
pub async fn refresh_codex_quota(app: AppHandle, account_id: String) -> Result<CodexQuota, String> {
    let result = codex_quota::refresh_account_quota(&account_id).await;
    if result.is_ok() {
        run_codex_post_refresh_checks(&app).await;
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn refresh_codex_subscription_info(
    app: AppHandle,
    account_id: String,
) -> Result<CodexAccount, String> {
    let result = codex_quota::refresh_account_subscription_info(&account_id, true).await;
    if result.is_ok() {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn refresh_current_codex_quota(app: AppHandle) -> Result<(), String> {
    let Some(account) = codex_account::get_current_account() else {
        return Err("未找到当前 Codex 账号".to_string());
    };
    if account.is_api_key_auth() {
        return Ok(());
    }

    let result = codex_quota::refresh_account_quota(&account.id).await;
    if result.is_ok() {
        run_codex_post_refresh_checks(&app).await;
        let _ = crate::modules::tray::update_tray_menu(&app);
        Ok(())
    } else {
        Err(result
            .err()
            .unwrap_or_else(|| "刷新 Codex 配额失败".to_string()))
    }
}

/// 刷新所有账号配额
#[tauri::command]
pub async fn refresh_all_codex_quotas(app: AppHandle) -> Result<i32, String> {
    let results = codex_quota::refresh_all_quotas().await?;
    let success_count = results.iter().filter(|(_, r)| r.is_ok()).count();
    if success_count > 0 {
        run_codex_post_refresh_checks(&app).await;
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success_count as i32)
}

async fn save_codex_oauth_tokens(tokens: CodexTokens) -> Result<CodexAccount, String> {
    let account = codex_account::upsert_account(tokens)?;

    if let Err(e) = codex_quota::refresh_account_quota(&account.id).await {
        logger::log_error(&format!("刷新配额失败: {}", e));
    }

    let loaded =
        codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())?;
    logger::log_info(&format!(
        "Codex OAuth 账号已保存: account_id={}, email={}",
        loaded.id, loaded.email
    ));
    Ok(loaded)
}

/// OAuth：开始登录（返回 loginId + authUrl）
#[tauri::command]
pub async fn codex_oauth_login_start(
    app_handle: AppHandle,
) -> Result<codex_oauth::CodexOAuthLoginStartResponse, String> {
    logger::log_info("Codex OAuth start 命令触发");
    let response = codex_oauth::start_oauth_login(app_handle).await?;
    logger::log_info(&format!(
        "Codex OAuth start 命令成功: login_id={}",
        response.login_id
    ));
    Ok(response)
}

/// OAuth：浏览器授权完成后按 loginId 完成登录
#[tauri::command]
pub async fn codex_oauth_login_completed(login_id: String) -> Result<CodexAccount, String> {
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    logger::log_info(&format!(
        "Codex OAuth completed 命令开始: login_id={}, started_at_ms={}",
        login_id, started_at_ms
    ));
    let tokens = match codex_oauth::complete_oauth_login(&login_id).await {
        Ok(tokens) => tokens,
        Err(e) => {
            logger::log_error(&format!(
                "Codex OAuth completed 命令失败: login_id={}, duration_ms={}, error={}",
                login_id,
                chrono::Utc::now().timestamp_millis() - started_at_ms,
                e
            ));
            return Err(e);
        }
    };
    let account = save_codex_oauth_tokens(tokens).await?;
    logger::log_info(&format!(
        "Codex OAuth completed 命令成功: login_id={}, duration_ms={}, account_id={}, account_email={}",
        login_id,
        chrono::Utc::now().timestamp_millis() - started_at_ms,
        account.id,
        account.email
    ));
    Ok(account)
}

/// OAuth：按 loginId 取消登录（login_id 为空时取消当前流程）
#[tauri::command]
pub fn codex_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "Codex OAuth cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    let result = codex_oauth::cancel_oauth_flow_for(login_id.as_deref());
    logger::log_info(&format!(
        "Codex OAuth cancel 命令返回: {:?}",
        result.as_ref().map(|_| "ok").map_err(|e| e)
    ));
    result
}

/// OAuth：手动提交回调链接（用于本地端口不可达时）
#[tauri::command]
pub fn codex_oauth_submit_callback_url(
    app_handle: AppHandle,
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    codex_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())?;
    let payload = serde_json::json!({ "loginId": login_id });
    let _ = app_handle.emit("codex-oauth-login-completed", payload.clone());
    let _ = app_handle.emit("ghcp-oauth-login-completed", payload);
    Ok(())
}

/// 通过 Token 添加账号
#[tauri::command]
pub async fn add_codex_account_with_token(
    id_token: String,
    access_token: String,
    refresh_token: Option<String>,
) -> Result<CodexAccount, String> {
    let tokens = CodexTokens {
        id_token,
        access_token,
        refresh_token,
    };

    let account = codex_account::upsert_account(tokens)?;

    // 刷新配额
    if let Err(e) = codex_quota::refresh_account_quota(&account.id).await {
        logger::log_error(&format!("刷新配额失败: {}", e));
    }

    codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())
}

/// 通过 API Key 添加账号
#[tauri::command]
pub fn add_codex_account_with_api_key(
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let account = codex_account::upsert_api_key_account(
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
        account_name,
    )?;
    codex_account::load_account(&account.id).ok_or_else(|| "账号保存后无法读取".to_string())
}

#[tauri::command]
pub fn update_codex_account_name(account_id: String, name: String) -> Result<CodexAccount, String> {
    codex_account::update_account_name(&account_id, name)
}

#[tauri::command]
pub fn update_codex_api_key_credentials(
    account_id: String,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Option<Vec<String>>,
    api_supports_vision: Option<bool>,
    api_model_vision_support: Option<std::collections::HashMap<String, bool>>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_credentials(
        &account_id,
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
        api_model_catalog.unwrap_or_default(),
        api_supports_vision.unwrap_or(false),
        api_model_vision_support.unwrap_or_default(),
    )
}

#[tauri::command]
pub async fn update_codex_api_key_bound_oauth_account(
    account_id: String,
    bound_oauth_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_bound_oauth_account(&account_id, bound_oauth_account_id).await
}

#[tauri::command]
pub async fn update_codex_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<CodexAccount, String> {
    codex_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub async fn update_codex_account_note(
    account_id: String,
    note: String,
) -> Result<CodexAccount, String> {
    codex_account::update_account_note(&account_id, note)
}

/// 检查 Codex OAuth 端口是否被占用
#[tauri::command]
pub fn is_codex_oauth_port_in_use() -> Result<bool, String> {
    let port = codex_oauth::get_callback_port();
    process::is_port_in_use(port)
}

/// 关闭占用 Codex OAuth 端口的进程
#[tauri::command]
pub fn close_codex_oauth_port() -> Result<u32, String> {
    let port = codex_oauth::get_callback_port();
    let killed = process::kill_port_processes(port)?;
    Ok(killed as u32)
}

#[tauri::command]
pub fn codex_wakeup_get_cli_status() -> Result<codex_wakeup::CodexCliStatus, String> {
    Ok(codex_wakeup::get_cli_status())
}

#[tauri::command]
pub fn codex_wakeup_update_runtime_config(
    codex_cli_path: Option<String>,
    node_path: Option<String>,
) -> Result<codex_wakeup::CodexCliStatus, String> {
    codex_wakeup::save_runtime_config(&codex_wakeup::CodexWakeupRuntimeConfig {
        codex_cli_path,
        node_path,
    })?;
    Ok(codex_wakeup::get_cli_status())
}

#[tauri::command]
pub fn codex_wakeup_get_overview() -> Result<codex_wakeup::CodexWakeupOverview, String> {
    codex_wakeup::load_overview()
}

#[tauri::command]
pub fn codex_wakeup_get_state() -> Result<codex_wakeup::CodexWakeupState, String> {
    codex_wakeup::load_state()
}

#[tauri::command]
pub fn codex_wakeup_save_state(
    enabled: bool,
    tasks: Vec<codex_wakeup::CodexWakeupTask>,
    model_presets: Vec<codex_wakeup::CodexWakeupModelPreset>,
    model_preset_migrations: Vec<String>,
) -> Result<codex_wakeup::CodexWakeupState, String> {
    codex_wakeup::save_state(&codex_wakeup::CodexWakeupState {
        enabled,
        tasks,
        model_presets,
        model_preset_migrations,
    })
}

#[tauri::command]
pub fn codex_wakeup_load_history() -> Result<Vec<codex_wakeup::CodexWakeupHistoryItem>, String> {
    codex_wakeup::load_history()
}

#[tauri::command]
pub fn codex_wakeup_clear_history() -> Result<(), String> {
    codex_wakeup::clear_history()
}

#[tauri::command]
pub fn codex_wakeup_cancel_scope(cancel_scope_id: String) -> Result<(), String> {
    codex_wakeup::cancel_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub fn codex_wakeup_release_scope(cancel_scope_id: String) -> Result<(), String> {
    codex_wakeup::release_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub async fn codex_wakeup_test(
    app: AppHandle,
    account_ids: Vec<String>,
    prompt: Option<String>,
    model: Option<String>,
    model_display_name: Option<String>,
    model_reasoning_effort: Option<String>,
    run_id: Option<String>,
    cancel_scope_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    codex_wakeup::run_batch(
        Some(&app),
        account_ids,
        prompt,
        codex_wakeup::CodexWakeupExecutionConfig {
            model,
            model_display_name,
            model_reasoning_effort,
        },
        codex_wakeup::TaskRunContext {
            trigger_type: "test".to_string(),
            task_id: None,
            task_name: None,
        },
        run_id,
        cancel_scope_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn codex_wakeup_run_task(
    app: AppHandle,
    task_id: String,
    run_id: Option<String>,
) -> Result<codex_wakeup::CodexWakeupBatchResult, String> {
    codex_wakeup_scheduler::run_task_now(Some(&app), &task_id, "manual_task", run_id).await
}

#[tauri::command]
pub async fn codex_wakeup_run_enabled_tasks(
    app: AppHandle,
    trigger_type: Option<String>,
) -> Result<u32, String> {
    let trigger = trigger_type.unwrap_or_else(|| "startup".to_string());
    codex_wakeup_scheduler::run_enabled_tasks_now(Some(&app), &trigger).await
}

// ─── Codex 账号分组持久化 ────────────────────────────────────────────

const CODEX_GROUPS_FILE: &str = "codex_account_groups.json";
const CODEX_MODEL_PROVIDERS_FILE: &str = "codex_model_providers.json";
const CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS: u64 = 20;

#[tauri::command]
pub async fn load_codex_account_groups() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read codex groups: {}", e))
}

#[tauri::command]
pub async fn save_codex_account_groups(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex groups: {}", e))
}

#[tauri::command]
pub async fn load_codex_model_providers() -> Result<String, String> {
    let path = account::get_data_dir()?.join(CODEX_MODEL_PROVIDERS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read codex model providers: {}", e))
}

#[tauri::command]
pub async fn save_codex_model_providers(data: String) -> Result<(), String> {
    let dir = account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_MODEL_PROVIDERS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex model providers: {}", e))
}

fn codex_model_provider_models_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/models".to_string()
    } else {
        format!("{}/models", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_usage_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/usage".to_string()
    } else {
        format!("{}/usage", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_new_api_billing_url(
    base_url: &str,
    endpoint: &str,
) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let base_path = url.path().trim_end_matches('/');
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_new_api_api_url(base_url: &str, endpoint: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let mut base_path = url.path().trim_end_matches('/').to_string();
    if base_path == "/v1" {
        base_path.clear();
    }
    let next_path = if base_path.is_empty() {
        format!("/{}", endpoint.trim_start_matches('/'))
    } else {
        format!("{}/{}", base_path, endpoint.trim_start_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn codex_model_provider_failure(
    title: &str,
    stage: &str,
    cause: String,
    suggestion: &str,
    status: Option<u16>,
    detail: Option<String>,
) -> CodexLocalAccessTestResult {
    CodexLocalAccessTestResult {
        model_id: None,
        latency_ms: None,
        output: None,
        failure: Some(CodexLocalAccessTestFailure {
            title: title.to_string(),
            stage: stage.to_string(),
            cause,
            suggestion: suggestion.to_string(),
            status,
            model_id: None,
            detail,
            gateway_output: None,
        }),
    }
}

fn summarize_model_provider_models(body: &serde_json::Value) -> (Option<String>, Option<String>) {
    let ids: Vec<String> = body
        .get("data")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
                .take(8)
                .map(|id| id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let first = ids.first().cloned();
    let output = if ids.is_empty() {
        None
    } else {
        Some(ids.join(", "))
    };
    (first, output)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageDetail {
    pub key: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderUsageSummary {
    pub mode: Option<String>,
    pub is_valid: Option<bool>,
    pub status: Option<String>,
    pub plan_name: Option<String>,
    pub remaining: Option<f64>,
    pub balance: Option<f64>,
    pub unit: Option<String>,
    pub quota_unlimited: Option<bool>,
    pub quota_limit: Option<f64>,
    pub quota_used: Option<f64>,
    pub quota_remaining: Option<f64>,
    pub today_requests: Option<i64>,
    pub today_total_tokens: Option<i64>,
    pub today_cost: Option<f64>,
    pub total_requests: Option<i64>,
    pub total_total_tokens: Option<i64>,
    pub total_cost: Option<f64>,
    pub model_stats_count: usize,
    pub latency_ms: u64,
    pub details: Vec<CodexModelProviderUsageDetail>,
}

fn json_f64_at(value: &serde_json::Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_f64()
}

fn json_i64_at(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_i64()
}

fn json_string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|item| item.to_string())
}

fn json_bool_at(value: &serde_json::Value, path: &[&str]) -> Option<bool> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_bool()
}

fn summarize_model_provider_usage(
    body: &serde_json::Value,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let model_stats_count = body
        .get("model_stats")
        .and_then(|value| value.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "mode",
        "Mode",
        json_string_at(body, &["mode"]),
    );
    push_usage_detail(
        &mut details,
        "status",
        "Status",
        json_string_at(body, &["status"]),
    );
    push_usage_detail(
        &mut details,
        "planName",
        "Plan",
        json_string_at(body, &["planName"]),
    );
    push_usage_detail(
        &mut details,
        "remaining",
        "Remaining",
        json_f64_at(body, &["remaining"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "balance",
        "Balance",
        json_f64_at(body, &["balance"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "todayRequests",
        "Today Requests",
        json_i64_at(body, &["usage", "today", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayTokens",
        "Today Tokens",
        json_i64_at(body, &["usage", "today", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "todayCost",
        "Today Cost",
        json_f64_at(body, &["usage", "today", "cost"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "totalRequests",
        "Total Requests",
        json_i64_at(body, &["usage", "total", "requests"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalTokens",
        "Total Tokens",
        json_i64_at(body, &["usage", "total", "total_tokens"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "totalCost",
        "Total Cost",
        json_f64_at(body, &["usage", "total", "cost"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: json_string_at(body, &["mode"]),
        is_valid: json_bool_at(body, &["isValid"]),
        status: json_string_at(body, &["status"]),
        plan_name: json_string_at(body, &["planName"]),
        remaining: json_f64_at(body, &["remaining"]),
        balance: json_f64_at(body, &["balance"]),
        unit: json_string_at(body, &["unit"]).or_else(|| json_string_at(body, &["quota", "unit"])),
        quota_unlimited: json_bool_at(body, &["quota", "unlimited"]),
        quota_limit: json_f64_at(body, &["quota", "limit"]),
        quota_used: json_f64_at(body, &["quota", "used"]),
        quota_remaining: json_f64_at(body, &["quota", "remaining"]),
        today_requests: json_i64_at(body, &["usage", "today", "requests"]),
        today_total_tokens: json_i64_at(body, &["usage", "today", "total_tokens"]),
        today_cost: json_f64_at(body, &["usage", "today", "cost"]),
        total_requests: json_i64_at(body, &["usage", "total", "requests"]),
        total_total_tokens: json_i64_at(body, &["usage", "total", "total_tokens"]),
        total_cost: json_f64_at(body, &["usage", "total", "cost"]),
        model_stats_count,
        latency_ms,
        details,
    }
}

fn format_usage_number(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{:.0}", value)
    } else {
        format!("{:.4}", value)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

fn push_usage_detail(
    details: &mut Vec<CodexModelProviderUsageDetail>,
    key: &str,
    label: &str,
    value: Option<String>,
) {
    let Some(value) = value else {
        return;
    };
    if value.trim().is_empty() {
        return;
    }
    details.push(CodexModelProviderUsageDetail {
        key: key.to_string(),
        label: label.to_string(),
        value,
    });
}

fn summarize_new_api_model_provider_usage(
    subscription: &serde_json::Value,
    usage: &serde_json::Value,
    token_usage: Option<&serde_json::Value>,
    latency_ms: u64,
) -> CodexModelProviderUsageSummary {
    let raw_quota_limit = json_f64_at(subscription, &["hard_limit_usd"])
        .or_else(|| json_f64_at(subscription, &["soft_limit_usd"]))
        .or_else(|| json_f64_at(subscription, &["system_hard_limit_usd"]));
    let quota_used = json_f64_at(usage, &["total_usage"]).map(|value| value / 100.0);
    let token_data = token_usage.and_then(|value| value.get("data"));
    let quota_unlimited = token_data
        .and_then(|value| json_bool_at(value, &["unlimited_quota"]))
        .unwrap_or_else(|| {
            let hard = json_f64_at(subscription, &["hard_limit_usd"]);
            let soft = json_f64_at(subscription, &["soft_limit_usd"]);
            let system = json_f64_at(subscription, &["system_hard_limit_usd"]);
            matches!(
                (hard, soft, system),
                (Some(h), Some(s), Some(sys))
                    if (h - 100_000_000.0).abs() < f64::EPSILON
                        && (s - 100_000_000.0).abs() < f64::EPSILON
                        && (sys - 100_000_000.0).abs() < f64::EPSILON
            )
        });
    let quota_limit = if quota_unlimited {
        None
    } else {
        raw_quota_limit
    };
    let quota_remaining = match (quota_limit, quota_used) {
        (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
        _ => None,
    };
    let mut details = Vec::new();
    push_usage_detail(
        &mut details,
        "hardLimitUsd",
        "Hard Limit USD",
        json_f64_at(subscription, &["hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "softLimitUsd",
        "Soft Limit USD",
        json_f64_at(subscription, &["soft_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "systemHardLimitUsd",
        "System Hard Limit USD",
        json_f64_at(subscription, &["system_hard_limit_usd"]).map(format_usage_number),
    );
    push_usage_detail(
        &mut details,
        "accessUntil",
        "Access Until",
        json_i64_at(subscription, &["access_until"]).map(|value| value.to_string()),
    );
    push_usage_detail(
        &mut details,
        "quotaUnlimited",
        "Unlimited Quota",
        Some(quota_unlimited.to_string()),
    );
    if let Some(token_data) = token_data {
        push_usage_detail(
            &mut details,
            "totalGranted",
            "Total Granted",
            json_f64_at(token_data, &["total_granted"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "totalAvailable",
            "Total Available",
            json_f64_at(token_data, &["total_available"]).map(format_usage_number),
        );
        push_usage_detail(
            &mut details,
            "expiresAt",
            "Expires At",
            json_i64_at(token_data, &["expires_at"]).map(|value| value.to_string()),
        );
        push_usage_detail(
            &mut details,
            "modelLimitsEnabled",
            "Model Limits",
            json_bool_at(token_data, &["model_limits_enabled"]).map(|value| value.to_string()),
        );
    }
    push_usage_detail(
        &mut details,
        "totalUsage",
        "Total Usage",
        json_f64_at(usage, &["total_usage"]).map(format_usage_number),
    );

    CodexModelProviderUsageSummary {
        mode: Some("new_api".to_string()),
        is_valid: None,
        status: None,
        plan_name: None,
        remaining: quota_remaining,
        balance: None,
        unit: Some("USD".to_string()),
        quota_unlimited: Some(quota_unlimited),
        quota_limit,
        quota_used,
        quota_remaining,
        today_requests: None,
        today_total_tokens: None,
        today_cost: None,
        total_requests: None,
        total_total_tokens: None,
        total_cost: quota_used,
        model_stats_count: 0,
        latency_ms,
        details,
    }
}

#[tauri::command]
pub async fn codex_test_model_provider_connection(
    base_url: String,
    api_key: String,
    wire_api: Option<String>,
) -> Result<CodexLocalAccessTestResult, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(codex_model_provider_failure(
            "missing_api_key",
            "credential",
            "MISSING_API_KEY".to_string(),
            "add_api_key",
            None,
            None,
        ));
    }

    let url = match codex_model_provider_models_url(&base_url) {
        Ok(url) => url,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "invalid_base_url",
                "url",
                error,
                "check_base_url",
                None,
                None,
            ));
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;
    let started = Instant::now();
    let response = match client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "network_failed",
                "network",
                error.to_string(),
                "check_network",
                None,
                Some(format!("GET {}", url)),
            ));
        }
    };
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();

    if !status.is_success() {
        let suggestion = if status == reqwest::StatusCode::UNAUTHORIZED
            || status == reqwest::StatusCode::FORBIDDEN
        {
            "check_api_key"
        } else if status == reqwest::StatusCode::NOT_FOUND {
            "check_base_url"
        } else {
            "check_provider_status"
        };
        return Ok(codex_model_provider_failure(
            "provider_http_failed",
            "models",
            "HTTP_STATUS".to_string(),
            suggestion,
            Some(status.as_u16()),
            Some(text.chars().take(1000).collect()),
        ));
    }

    let parsed = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(value) => value,
        Err(error) => {
            return Ok(codex_model_provider_failure(
                "response_parse_failed",
                "parse",
                error.to_string(),
                "check_openai_compatible_models",
                Some(status.as_u16()),
                Some(text.chars().take(1000).collect()),
            ));
        }
    };
    let (model_id, output) = summarize_model_provider_models(&parsed);
    let protocol = wire_api.unwrap_or_else(|| "auto".to_string());
    Ok(CodexLocalAccessTestResult {
        model_id,
        latency_ms: Some(latency_ms),
        output: output.or_else(|| Some(format!("{} connection ok", protocol))),
        failure: None,
    })
}

#[tauri::command]
pub async fn codex_query_model_provider_usage(
    base_url: String,
    api_key: String,
    integration_type: Option<String>,
) -> Result<CodexModelProviderUsageSummary, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("MISSING_API_KEY".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(CODEX_MODEL_PROVIDER_TEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("CREATE_HTTP_CLIENT_FAILED: {}", e))?;

    let requested_type = integration_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match requested_type {
        Some("new_api") => query_new_api_model_provider_usage(&client, &base_url, key).await,
        Some("sub2api") => query_sub2api_model_provider_usage(&client, &base_url, key).await,
        Some(value) => Err(format!("PROVIDER_USAGE_TYPE_UNSUPPORTED: {}", value)),
        None => {
            let new_api_error =
                match query_new_api_model_provider_usage(&client, &base_url, key).await {
                    Ok(summary) => return Ok(summary),
                    Err(error) => error,
                };
            match query_sub2api_model_provider_usage(&client, &base_url, key).await {
                Ok(summary) => Ok(summary),
                Err(sub2api_error) => Err(format!(
                    "PROVIDER_USAGE_DETECT_FAILED: new_api: {}; sub2api: {}",
                    new_api_error, sub2api_error
                )),
            }
        }
    }
}

async fn query_new_api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let subscription_url =
        codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/subscription")?;
    let usage_url = codex_model_provider_new_api_billing_url(base_url, "dashboard/billing/usage")?;
    let token_usage_url = codex_model_provider_new_api_api_url(base_url, "api/usage/token/")?;
    let started = Instant::now();
    let subscription_response = client
        .get(&subscription_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let subscription_status = subscription_response.status();
    let subscription_text = subscription_response.text().await.unwrap_or_default();
    if !subscription_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            subscription_status.as_u16(),
            subscription_text.chars().take(300).collect::<String>()
        ));
    }
    let usage_response = client
        .get(&usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let usage_status = usage_response.status();
    let usage_text = usage_response.text().await.unwrap_or_default();
    if !usage_status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            usage_status.as_u16(),
            usage_text.chars().take(300).collect::<String>()
        ));
    }
    let subscription = serde_json::from_str::<serde_json::Value>(&subscription_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let usage = serde_json::from_str::<serde_json::Value>(&usage_text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    let token_usage = match client
        .get(&token_usage_url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            let text = response.text().await.unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&text).ok()
        }
        _ => None,
    };
    Ok(summarize_new_api_model_provider_usage(
        &subscription,
        &usage,
        token_usage.as_ref(),
        latency_ms,
    ))
}

async fn query_sub2api_model_provider_usage(
    client: &reqwest::Client,
    base_url: &str,
    key: &str,
) -> Result<CodexModelProviderUsageSummary, String> {
    let url = codex_model_provider_usage_url(base_url)?;
    let started = Instant::now();
    let response = client
        .get(&url)
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("PROVIDER_USAGE_NETWORK_FAILED: {}", e))?;
    let latency_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "PROVIDER_USAGE_HTTP_{}: {}",
            status.as_u16(),
            text.chars().take(300).collect::<String>()
        ));
    }
    let parsed = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|e| format!("PROVIDER_USAGE_PARSE_FAILED: {}", e))?;
    Ok(summarize_model_provider_usage(&parsed, latency_ms))
}

#[tauri::command]
pub async fn codex_local_access_get_state() -> Result<CodexLocalAccessState, String> {
    codex_local_access::get_local_access_state().await
}

#[tauri::command]
pub async fn codex_local_access_save_accounts(
    account_ids: Vec<String>,
    restrict_free_accounts: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::save_local_access_accounts(
        account_ids,
        restrict_free_accounts.unwrap_or(true),
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_remove_account(
    account_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::remove_local_access_account(&account_id).await
}

#[tauri::command]
pub async fn codex_local_access_rotate_api_key() -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_api_key().await
}

#[tauri::command]
pub async fn codex_local_access_update_bound_oauth_account(
    bound_oauth_account_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_bound_oauth_account(bound_oauth_account_id).await
}

#[tauri::command]
pub async fn codex_local_access_clear_stats() -> Result<CodexLocalAccessState, String> {
    codex_local_access::clear_local_access_stats().await
}

#[tauri::command]
pub async fn codex_local_access_query_request_logs(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    codex_local_access::query_local_access_usage_events(
        page,
        page_size,
        stats_range,
        model_query,
        account_query,
        api_key_query,
        gateway_mode,
        request_kind,
        success,
        error_category,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_prepare_restart() -> Result<CodexLocalAccessState, String> {
    codex_local_access::prepare_local_access_gateway_for_restart().await
}

#[tauri::command]
pub async fn codex_local_access_kill_port() -> Result<CodexLocalAccessPortCleanupResult, String> {
    codex_local_access::kill_local_access_port_processes().await
}

#[tauri::command]
pub async fn codex_local_access_update_port(port: u16) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_port(port).await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_strategy(
    strategy: CodexLocalAccessRoutingStrategy,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_strategy(strategy).await
}

#[tauri::command]
pub async fn codex_local_access_update_custom_routing(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_custom_routing(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_account_model_rules(
    rules: Vec<CodexLocalAccessAccountModelRule>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_account_model_rules(rules).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_rules(
    model_aliases: Vec<CodexLocalAccessModelAlias>,
    excluded_models: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_rules(model_aliases, excluded_models).await
}

#[tauri::command]
pub async fn codex_local_access_update_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_model_pricings(model_pricings).await
}

#[tauri::command]
pub async fn codex_local_access_update_routing_options(
    session_affinity: bool,
    session_affinity_ttl_ms: i64,
    max_retry_credentials: u16,
    max_retry_interval_ms: u64,
    disable_cooling: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_routing_options(
        session_affinity,
        session_affinity_ttl_ms,
        max_retry_credentials,
        max_retry_interval_ms,
        disable_cooling,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_timeouts(
    timeouts: CodexLocalAccessTimeouts,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeouts(timeouts, active_timeout_preset_id).await
}

#[tauri::command]
pub async fn codex_local_access_update_timeout_presets(
    timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_timeout_presets(
        timeout_presets,
        active_timeout_preset_id,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_update_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_upstream_proxy_config(upstream_proxy_url).await
}

#[tauri::command]
pub async fn codex_local_access_update_gateway_mode(
    gateway_mode: CodexLocalAccessGatewayMode,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_gateway_mode(gateway_mode).await
}

#[tauri::command]
pub async fn codex_local_access_update_debug_logs(
    debug_logs: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_debug_logs(debug_logs).await
}

#[tauri::command]
pub async fn codex_local_access_update_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_scope(access_scope).await
}

#[tauri::command]
pub async fn codex_local_access_update_client_base_url_host(
    client_base_url_host: CodexLocalAccessClientBaseUrlHost,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_client_base_url_host(client_base_url_host).await
}

#[tauri::command]
pub async fn codex_local_access_update_image_generation_mode(
    image_generation_mode: crate::models::codex_local_access::CodexLocalAccessImageGenerationMode,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_image_generation_mode(image_generation_mode).await
}

#[tauri::command]
pub async fn codex_local_access_create_api_key(
    label: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::create_local_access_api_key(label).await
}

#[tauri::command]
pub async fn codex_local_access_update_api_key(
    api_key_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    model_prefix: Option<String>,
    allowed_models: Option<Vec<String>>,
    excluded_models: Option<Vec<String>>,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_api_key(
        api_key_id,
        label,
        enabled,
        model_prefix,
        allowed_models,
        excluded_models,
    )
    .await
}

#[tauri::command]
pub async fn codex_local_access_rotate_named_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::rotate_local_access_named_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_delete_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::delete_local_access_api_key(api_key_id).await
}

#[tauri::command]
pub async fn codex_local_access_set_enabled(
    enabled: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::set_local_access_enabled(enabled).await
}

#[tauri::command]
pub async fn codex_local_access_activate(app: AppHandle) -> Result<CodexLocalAccessState, String> {
    let codex_home = codex_account::get_codex_home();
    let state = codex_local_access::activate_local_access_for_dir(&codex_home).await?;
    let api_service_speed = codex_speed::get_api_service_app_speed_config()?.speed;
    codex_speed::write_official_app_speed(api_service_speed.clone())?;

    let mut index = codex_account::load_account_index();
    index.current_account_id = None;
    codex_account::save_account_index(&index)?;

    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(
            crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
        )),
        None,
        Some(false),
        None,
        None,
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例为 API 服务模式失败: {}", e));
    } else {
        logger::log_info("已同步更新 Codex 默认实例为 API 服务模式");
    }
    if let Err(e) = crate::modules::codex_instance::update_default_app_speed(api_service_speed) {
        logger::log_warn(&format!("更新 Codex 默认实例 API 服务速度失败: {}", e));
    }

    let user_config = config::get_user_config();

    logger::log_info("API 服务启动模式下跳过 OpenCode / OpenClaw OAuth 同步");

    if user_config.codex_launch_on_switch {
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        match crate::commands::codex_instance::codex_start_default_with_prepared_profile().await {
            Ok(_) => {}
            Err(e) => {
                logger::log_warn(&format!("Codex 启动失败: {}", e));
                if e.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({ "app": "codex", "retry": { "kind": "default" } }),
                    );
                }
            }
        }
    } else {
        logger::log_info("已关闭切换 Codex 时自动启动 Codex App");
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(state)
}

#[tauri::command]
pub async fn codex_local_access_test() -> Result<CodexLocalAccessTestResult, String> {
    codex_local_access::test_local_access_with_dialog().await
}

#[tauri::command]
pub async fn codex_local_access_chat_test(
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<CodexLocalAccessChatResult, String> {
    codex_local_access::chat_local_access_with_dialog(model_id, messages).await
}

#[tauri::command]
pub async fn codex_local_access_chat_test_stream(
    app: AppHandle,
    session_id: String,
    model_id: String,
    messages: Vec<CodexLocalAccessChatMessage>,
) -> Result<(), String> {
    codex_local_access::stream_chat_local_access_with_dialog(app, session_id, model_id, messages)
        .await
}
