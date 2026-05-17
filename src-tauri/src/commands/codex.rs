use crate::models::codex::{
    CodexAccount, CodexApiProviderMode, CodexAppSpeed, CodexAppSpeedConfig, CodexQuickConfig,
    CodexQuota, CodexTokens,
};
use crate::models::codex_local_access::{
    CodexLocalAccessPortCleanupResult, CodexLocalAccessRoutingStrategy, CodexLocalAccessScope,
    CodexLocalAccessState, CodexLocalAccessTestResult,
};
use crate::modules::{
    codex_account, codex_local_access, codex_oauth, codex_quota, codex_session_visibility,
    codex_speed, codex_wakeup, codex_wakeup_scheduler, config, logger, openclaw_auth,
    opencode_auth, process,
};
use std::sync::atomic::{AtomicBool, Ordering};
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

fn read_default_codex_history_provider_for_switch() -> Option<String> {
    let codex_home = codex_account::get_codex_home();
    match codex_session_visibility::read_history_visibility_provider_for_dir(&codex_home) {
        Ok(provider) => Some(provider),
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex切号] 读取切号前后会话 provider 失败: dir={}, error={}",
                codex_home.display(),
                error
            ));
            None
        }
    }
}

fn codex_history_provider_changed(
    previous_provider: Option<&str>,
    next_provider: Option<&str>,
) -> bool {
    match (previous_provider, next_provider) {
        (Some(previous), Some(next)) => previous != next,
        _ => false,
    }
}

fn should_close_default_codex_before_switch_repair(user_config: &config::UserConfig) -> bool {
    if !user_config.codex_launch_on_switch {
        return false;
    }

    let default_settings = match crate::modules::codex_instance::load_default_settings() {
        Ok(settings) => settings,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex切号] 会话可见性修复前读取默认实例设置失败，跳过提前关闭默认实例: {}",
                error
            ));
            return false;
        }
    };

    if default_settings.launch_mode == crate::models::InstanceLaunchMode::Cli {
        return true;
    }

    match process::ensure_codex_launch_path_configured() {
        Ok(()) => true,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex切号] Codex 启动路径未就绪，跳过会话可见性修复前的提前关闭: {}",
                error
            ));
            false
        }
    }
}

fn repair_codex_session_visibility_after_provider_change(
    previous_provider: Option<String>,
    next_provider: Option<String>,
    user_config: &config::UserConfig,
) {
    if !codex_history_provider_changed(previous_provider.as_deref(), next_provider.as_deref()) {
        return;
    }

    let previous_label = previous_provider.as_deref().unwrap_or("<unknown>");
    let next_label = next_provider.as_deref().unwrap_or("<unknown>");
    logger::log_info(&format!(
        "[Codex切号] 检测到会话 provider 变化，准备修复历史会话可见性: {} -> {}",
        previous_label, next_label
    ));

    if should_close_default_codex_before_switch_repair(user_config) {
        match process::close_codex_default(20) {
            Ok(()) => logger::log_info("[Codex切号] 已在会话可见性修复前关闭默认 Codex 实例"),
            Err(error) => logger::log_warn(&format!(
                "[Codex切号] 会话可见性修复前关闭默认 Codex 实例失败，继续尝试修复: {}",
                error
            )),
        }
    }

    match codex_session_visibility::repair_session_visibility_across_instances() {
        Ok(summary) => {
            logger::log_info(&format!(
                "[Codex切号] 会话可见性修复完成: mutated_instances={}, rollout_files={}, sqlite_rows={}, skipped_sqlite_files={}, message={}",
                summary.mutated_instance_count,
                summary.changed_rollout_file_count,
                summary.updated_sqlite_row_count,
                summary.skipped_sqlite_file_count,
                summary.message
            ));
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex切号] 会话可见性自动修复失败，切号已完成，可稍后在会话管理中手动重试: {}",
                error
            ));
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
    let current_account_id = codex_account::load_account_index().current_account_id;
    let default_bind_account_id = crate::modules::codex_instance::load_default_settings()
        .ok()
        .and_then(|settings| settings.bind_account_id);
    if current_account_id.as_deref() == Some(account_id.as_str())
        || default_bind_account_id.as_deref() == Some(account_id.as_str())
    {
        codex_speed::write_official_app_speed(account.app_speed.clone())?;
        let _ = crate::modules::codex_instance::update_default_app_speed(account.app_speed.clone());
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
    let previous_history_provider = read_default_codex_history_provider_for_switch();
    // 切换账号（写入 auth.json）
    let account = codex_account::switch_account_managed(&account_id).await?;
    let next_history_provider = read_default_codex_history_provider_for_switch();
    let account_speed = account.app_speed.clone();
    codex_speed::write_official_app_speed(account_speed.clone())?;

    // 同步更新 Codex 默认实例的绑定账号（不同步到 Antigravity，因为账号体系不同）
    if let Err(e) = crate::modules::codex_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
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
    repair_codex_session_visibility_after_provider_change(
        previous_history_provider,
        next_history_provider,
        &user_config,
    );

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
        match crate::commands::codex_instance::codex_start_instance("__default__".to_string()).await
        {
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
pub fn delete_codex_account(account_id: String) -> Result<(), String> {
    codex_account::remove_account(&account_id)
}

/// 批量删除 Codex 账号
#[tauri::command]
pub fn delete_codex_accounts(account_ids: Vec<String>) -> Result<(), String> {
    codex_account::remove_accounts(&account_ids)
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
) -> Result<CodexAccount, String> {
    let account = codex_account::upsert_api_key_account(
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
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
) -> Result<CodexAccount, String> {
    codex_account::update_api_key_credentials(
        &account_id,
        api_key,
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
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

#[tauri::command]
pub async fn load_codex_account_groups() -> Result<String, String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let path = home.join(".antigravity_cockpit").join(CODEX_GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read codex groups: {}", e))
}

#[tauri::command]
pub async fn save_codex_account_groups(data: String) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let dir = home.join(".antigravity_cockpit");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex groups: {}", e))
}

#[tauri::command]
pub async fn load_codex_model_providers() -> Result<String, String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let path = home
        .join(".antigravity_cockpit")
        .join(CODEX_MODEL_PROVIDERS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read codex model providers: {}", e))
}

#[tauri::command]
pub async fn save_codex_model_providers(data: String) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let dir = home.join(".antigravity_cockpit");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(CODEX_MODEL_PROVIDERS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write codex model providers: {}", e))
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
pub async fn codex_local_access_update_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::update_local_access_scope(access_scope).await
}

#[tauri::command]
pub async fn codex_local_access_set_enabled(
    enabled: bool,
) -> Result<CodexLocalAccessState, String> {
    codex_local_access::set_local_access_enabled(enabled).await
}

#[tauri::command]
pub async fn codex_local_access_activate(app: AppHandle) -> Result<CodexLocalAccessState, String> {
    let previous_history_provider = read_default_codex_history_provider_for_switch();
    let codex_home = codex_account::get_codex_home();
    let state = codex_local_access::activate_local_access_for_dir(&codex_home).await?;
    let next_history_provider = read_default_codex_history_provider_for_switch();
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
    ) {
        logger::log_warn(&format!("更新 Codex 默认实例为 API 服务模式失败: {}", e));
    } else {
        logger::log_info("已同步更新 Codex 默认实例为 API 服务模式");
    }
    if let Err(e) = crate::modules::codex_instance::update_default_app_speed(api_service_speed) {
        logger::log_warn(&format!("更新 Codex 默认实例 API 服务速度失败: {}", e));
    }

    let user_config = config::get_user_config();
    repair_codex_session_visibility_after_provider_change(
        previous_history_provider,
        next_history_provider,
        &user_config,
    );

    logger::log_info("API 服务启动模式下跳过 OpenCode / OpenClaw OAuth 同步");

    if user_config.codex_launch_on_switch {
        #[cfg(target_os = "macos")]
        if process::is_codex_running() {
            logger::log_info("检测到 Codex 正在运行，将按默认实例 PID 逻辑重启");
        }
        match crate::commands::codex_instance::codex_start_instance("__default__".to_string()).await
        {
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
    codex_local_access::test_local_access_with_cli().await
}

#[cfg(test)]
mod tests {
    use super::codex_history_provider_changed;

    #[test]
    fn codex_history_provider_changed_only_when_both_known_and_different() {
        assert!(codex_history_provider_changed(
            Some("openai"),
            Some("codex_local_access")
        ));
        assert!(codex_history_provider_changed(
            Some("codex_local_access"),
            Some("openai")
        ));
        assert!(!codex_history_provider_changed(
            Some("codex_local_access"),
            Some("codex_local_access")
        ));
        assert!(!codex_history_provider_changed(None, Some("openai")));
        assert!(!codex_history_provider_changed(Some("openai"), None));
    }
}
