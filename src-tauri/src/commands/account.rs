use crate::error::{AppError, AppResult};
use crate::models;
use crate::modules;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AntigravityRuntimeTarget {
    Legacy,
    Ide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AntigravitySwitchFlow {
    Legacy,
    LocalNoLaunch,
    DualNoRestart,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AntigravityDesktopAuthMode {
    LegacyStateDb,
    SystemCredential,
}

fn normalize_antigravity_runtime_target(raw: Option<&str>) -> AntigravityRuntimeTarget {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "antigravity" => AntigravityRuntimeTarget::Legacy,
        _ => AntigravityRuntimeTarget::Ide,
    }
}

fn resolve_antigravity_switch_flow(
    runtime_target: AntigravityRuntimeTarget,
    launch_on_switch: bool,
    dual_switch_no_restart_enabled: bool,
) -> AntigravitySwitchFlow {
    match runtime_target {
        AntigravityRuntimeTarget::Legacy => AntigravitySwitchFlow::Legacy,
        AntigravityRuntimeTarget::Ide if !launch_on_switch => AntigravitySwitchFlow::LocalNoLaunch,
        AntigravityRuntimeTarget::Ide if dual_switch_no_restart_enabled => {
            AntigravitySwitchFlow::DualNoRestart
        }
        AntigravityRuntimeTarget::Ide => AntigravitySwitchFlow::Restart,
    }
}

fn parse_version_parts(value: &str) -> Vec<u64> {
    value
        .trim()
        .trim_start_matches(|ch| ch == 'v' || ch == 'V')
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left_parts = parse_version_parts(left);
    let right_parts = parse_version_parts(right);
    if left_parts.is_empty() || right_parts.is_empty() {
        return None;
    }
    let max_len = left_parts.len().max(right_parts.len());
    for index in 0..max_len {
        let left_value = left_parts.get(index).copied().unwrap_or(0);
        let right_value = right_parts.get(index).copied().unwrap_or(0);
        match left_value.cmp(&right_value) {
            std::cmp::Ordering::Equal => {}
            ordering => return Some(ordering),
        }
    }
    Some(std::cmp::Ordering::Equal)
}

#[allow(dead_code)]
fn resolve_antigravity_desktop_auth_mode_from_info(
    info: crate::commands::system::AntigravityInstalledVersionInfo,
) -> AntigravityDesktopAuthMode {
    modules::logger::log_info(&format!(
        "[Antigravity] 检测到桌面版版本: version={}, path={}, source={}",
        info.version, info.app_path, info.source
    ));
    match compare_versions(&info.version, "2.0.0") {
        Some(std::cmp::Ordering::Less) => AntigravityDesktopAuthMode::LegacyStateDb,
        Some(_) => AntigravityDesktopAuthMode::SystemCredential,
        None => {
            modules::logger::log_warn(&format!(
                "[Antigravity] 无法解析 Antigravity 安装版本: {}，默认采用系统凭据认证模式",
                info.version
            ));
            AntigravityDesktopAuthMode::SystemCredential
        }
    }
}

#[allow(dead_code)]
fn resolve_antigravity_desktop_auth_mode() -> Result<AntigravityDesktopAuthMode, String> {
    if let Some(info) =
        crate::commands::system::resolve_antigravity_installed_version_info_for_target(Some(
            "antigravity",
        ))
    {
        return Ok(resolve_antigravity_desktop_auth_mode_from_info(info));
    }

    if let Some(info) =
        crate::commands::system::get_cached_antigravity_installed_version_info_for_target(Some(
            "antigravity",
        ))
    {
        return Ok(resolve_antigravity_desktop_auth_mode_from_info(info));
    }

    modules::logger::log_warn(
        "[Antigravity] 无法确认 Antigravity 安装版本，将默认采用系统凭据认证模式",
    );
    Ok(AntigravityDesktopAuthMode::SystemCredential)
}

fn legacy_antigravity_user_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join("Library/Application Support/Antigravity"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join("Antigravity"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config/Antigravity"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 Antigravity 默认目录".to_string())
}

fn legacy_antigravity_state_db_path() -> Result<PathBuf, String> {
    let path = legacy_antigravity_user_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(conn) = rusqlite::Connection::open(&path) {
            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value TEXT)",
                [],
            );
        }
    }
    Ok(path)
}

#[tauri::command]
pub async fn list_accounts() -> Result<Vec<models::Account>, String> {
    modules::list_accounts()
}

/// 从 VS Code SecretStorage 同步插件账号
#[tauri::command]
pub async fn sync_from_extension(app: tauri::AppHandle) -> Result<usize, String> {
    modules::import::import_from_extension_credentials(Some(&app)).await
}

#[tauri::command]
pub async fn add_account(refresh_token: String) -> Result<models::Account, String> {
    let token_res = modules::oauth::refresh_access_token(&refresh_token).await?;
    let user_info = modules::oauth::get_user_info(&token_res.access_token).await?;

    let token = models::TokenData::new(
        token_res.access_token,
        refresh_token,
        token_res.expires_in,
        Some(user_info.email.clone()),
        None,
        None,
    )
    .with_oauth_metadata(token_res.oauth_client_key, token_res.id_token);

    let account =
        modules::upsert_account(user_info.email.clone(), user_info.get_display_name(), token)?;
    modules::logger::log_info(&format!("添加账号成功: {}", account.email));

    // 广播通知
    modules::websocket::broadcast_data_changed("account_added");

    Ok(account)
}

#[tauri::command]
pub async fn delete_account(account_id: String) -> Result<(), String> {
    modules::delete_account(&account_id)?;
    modules::websocket::broadcast_data_changed("account_deleted");
    Ok(())
}

#[tauri::command]
pub async fn delete_accounts(account_ids: Vec<String>) -> Result<(), String> {
    modules::delete_accounts(&account_ids)?;
    modules::websocket::broadcast_data_changed("accounts_deleted");
    Ok(())
}

#[tauri::command]
pub async fn reorder_accounts(account_ids: Vec<String>) -> Result<(), String> {
    modules::reorder_accounts(&account_ids)
}

#[tauri::command]
pub async fn get_current_account(
    runtime_target: Option<String>,
) -> Result<Option<models::Account>, String> {
    let runtime_target = normalize_antigravity_runtime_target(runtime_target.as_deref());
    let bound_account_id = match runtime_target {
        AntigravityRuntimeTarget::Legacy => {
            modules::antigravity_legacy_instance::load_default_settings()
                .ok()
                .and_then(|settings| settings.bind_account_id)
        }
        AntigravityRuntimeTarget::Ide => modules::instance::load_default_settings()
            .ok()
            .and_then(|settings| settings.bind_account_id),
    };

    if let Some(account_id) = bound_account_id.as_deref() {
        match modules::load_account(account_id) {
            Ok(mut account) => {
                let _ = modules::quota_cache::apply_cached_quota(&mut account, "authorized");
                return Ok(Some(account));
            }
            Err(error) => modules::logger::log_warn(&format!(
                "[Antigravity] 当前账号绑定已失效，将回退全局当前账号: target={:?}, account_id={}, error={}",
                runtime_target, account_id, error
            )),
        }
    }

    modules::get_current_account()
}

#[tauri::command]
pub async fn set_current_account(
    app: tauri::AppHandle,
    account_id: String,
    runtime_target: Option<String>,
) -> Result<(), String> {
    let runtime_target = normalize_antigravity_runtime_target(runtime_target.as_deref());
    match runtime_target {
        AntigravityRuntimeTarget::Legacy => {
            let _ = modules::antigravity_legacy_instance::update_default_settings(
                Some(Some(account_id.clone())),
                None,
                Some(false),
            )?;
        }
        AntigravityRuntimeTarget::Ide => {
            let _ = modules::instance::update_default_settings(
                Some(Some(account_id.clone())),
                None,
                Some(false),
            )?;
        }
    }
    modules::set_current_account_id(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub async fn fetch_account_quota(account_id: String) -> AppResult<models::Account> {
    let mut account = modules::load_account(&account_id).map_err(AppError::Account)?;
    let quota = modules::fetch_quota_with_fresh_token(&mut account, true).await?;
    modules::update_account_quota(&account_id, quota).map_err(AppError::Account)?;
    // 重载账号，包含写入的 quota_error 等最新信息
    let updated_account = modules::load_account(&account_id).map_err(AppError::Account)?;
    Ok(updated_account)
}

#[tauri::command]
pub async fn refresh_all_quotas(
    app: tauri::AppHandle,
) -> Result<modules::account::RefreshStats, String> {
    let result = modules::account::refresh_all_quotas_logic(
        modules::account::QuotaRefreshTrigger::ManualBatch,
    )
    .await;
    if result.is_ok() {
        let mut switched = false;
        match modules::account::run_auto_switch_if_needed().await {
            Ok(Some(account)) => {
                modules::logger::log_info(&format!("[AutoSwitch] 自动切号完成: {}", account.email));
                switched = true;
            }
            Ok(None) => {}
            Err(e) => {
                modules::logger::log_warn(&format!("[AutoSwitch] 自动切号执行失败: {}", e));
            }
        }
        if !switched {
            if let Err(e) = modules::account::run_quota_alert_if_needed() {
                modules::logger::log_warn(&format!("[QuotaAlert] 预警检查失败: {}", e));
            }
        }
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn refresh_current_quota(app: tauri::AppHandle) -> Result<(), String> {
    let Some(account) = modules::get_current_account().map_err(|e| e.to_string())? else {
        return Err("未找到当前账号".to_string());
    };
    let mut account = account;
    let quota = modules::fetch_quota_with_fresh_token(&mut account, true)
        .await
        .map_err(|e| e.to_string())?;
    modules::update_account_quota(&account.id, quota).map_err(|e| e.to_string())?;

    let mut switched = false;
    match modules::account::run_auto_switch_if_needed().await {
        Ok(Some(account)) => {
            modules::logger::log_info(&format!(
                "[AutoSwitch] 当前账号刷新后自动切号完成: {}",
                account.email
            ));
            switched = true;
        }
        Ok(None) => {}
        Err(e) => {
            modules::logger::log_warn(&format!("[AutoSwitch] 当前账号刷新后自动切号失败: {}", e));
        }
    }

    if !switched {
        if let Err(e) = modules::account::run_quota_alert_if_needed() {
            modules::logger::log_warn(&format!("[QuotaAlert] 当前账号刷新后预警检查失败: {}", e));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

async fn switch_account_legacy_antigravity(
    app: AppHandle,
    account_id: String,
) -> Result<models::Account, String> {
    modules::logger::log_info(&format!("开始切换 Antigravity 账号: {}", account_id));

    if let Err(e) = modules::process::ensure_antigravity_legacy_launch_path_configured() {
        if e.starts_with("APP_PATH_NOT_FOUND:") {
            let _ = app.emit(
                "app:path_missing",
                serde_json::json!({
                    "app": "antigravity",
                    "retry": {
                        "kind": "switchAccount",
                        "accountId": account_id,
                        "runtimeTarget": "antigravity"
                    }
                }),
            );
        }
        return Err(e);
    }

    let auth_mode = resolve_antigravity_desktop_auth_mode()?;
    let mut account = modules::account::prepare_account_for_injection(&account_id).await?;
    modules::set_current_account_id(&account_id)?;
    account.update_last_used();
    modules::save_account(&account)?;

    if let Err(e) = modules::antigravity_legacy_instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        modules::logger::log_warn(&format!("更新 Antigravity 默认实例绑定账号失败: {}", e));
    }

    let default_dir = legacy_antigravity_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();
    modules::process::close_antigravity_legacy_instances(
        &[default_dir_str.clone()],
        &default_dir_str,
        20,
    )?;
    let _ = modules::antigravity_legacy_instance::update_default_pid(None);

    match auth_mode {
        AntigravityDesktopAuthMode::SystemCredential => {
            modules::logger::log_info("[Antigravity] 使用系统凭据认证模式写入账号");
            modules::antigravity_credential::write_antigravity_system_credential(&account)?;
        }
        AntigravityDesktopAuthMode::LegacyStateDb => {
            modules::logger::log_info("[Antigravity] 使用旧版 SQLite 认证模式写入账号");
            let db_path = legacy_antigravity_state_db_path()?;
            modules::db::inject_account_token_to_path(&db_path, &account)?;
        }
    }

    modules::logger::log_info("正在启动 Antigravity 默认实例...");
    let default_settings = modules::antigravity_legacy_instance::load_default_settings()?;
    let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
    let launch_result = modules::process::start_antigravity_legacy_with_args("", &extra_args);
    let launch_error = match launch_result {
        Ok(pid) => {
            if let Err(e) = modules::antigravity_legacy_instance::update_default_pid(Some(pid)) {
                modules::logger::log_warn(&format!("更新默认实例 PID 失败: {}", e));
            }
            None
        }
        Err(e) => {
            modules::logger::log_warn(&format!("Antigravity 启动失败: {}", e));
            if e.starts_with("APP_PATH_NOT_FOUND:") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "antigravity",
                        "retry": {
                            "kind": "switchAccount",
                            "accountId": account_id,
                            "runtimeTarget": "antigravity"
                        }
                    }),
                );
            }
            Some(e)
        }
    };

    if let Some(err) = launch_error {
        modules::websocket::broadcast_account_switched(&account.id, &account.email);
        if err.starts_with("APP_PATH_NOT_FOUND:") {
            return Err(err);
        }
        return Err(format!("账号已切换，但启动 Antigravity 失败: {}", err));
    }

    modules::logger::log_info(&format!("Antigravity 账号切换完成: {}", account.email));
    modules::websocket::broadcast_account_switched(&account.id, &account.email);
    Ok(account)
}

/// 切换账号（完整流程：Token刷新 + 关闭程序 + 注入 + 重启）
#[tauri::command]
pub async fn switch_account(
    app: AppHandle,
    account_id: String,
    runtime_target: Option<String>,
) -> Result<models::Account, String> {
    let runtime_target = normalize_antigravity_runtime_target(runtime_target.as_deref());
    let user_config = modules::config::get_user_config();
    match resolve_antigravity_switch_flow(
        runtime_target,
        user_config.antigravity_launch_on_switch,
        user_config.antigravity_dual_switch_no_restart_enabled,
    ) {
        AntigravitySwitchFlow::Legacy => {
            return switch_account_legacy_antigravity(app, account_id).await;
        }
        AntigravitySwitchFlow::LocalNoLaunch => {
            let result = modules::account::switch_account_local_no_restart(&account_id).await;
            if let Ok(account) = &result {
                modules::websocket::broadcast_account_switched(&account.id, &account.email);
                modules::websocket::broadcast_data_changed("switch_account_local_no_launch");
            }
            return result;
        }
        AntigravitySwitchFlow::DualNoRestart => {
            let result = modules::account::switch_account_dual_no_restart(
                &account_id,
                "manual",
                "tools.account.switch",
                "dual_no_restart",
                None,
            )
            .await;
            if let Err(error) = &result {
                if error.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({
                            "app": "antigravity",
                            "retry": { "kind": "switchAccount", "accountId": account_id }
                        }),
                    );
                }
            }
            return result;
        }
        AntigravitySwitchFlow::Restart => {}
    }

    modules::logger::log_info(&format!("开始切换账号: {}", account_id));

    // 1. 加载并验证账号存在
    let mut account = modules::load_account(&account_id)?;
    modules::logger::log_info(&format!(
        "正在切换到账号: {} (ID: {})",
        account.email, account.id
    ));

    // 预检应用路径：路径缺失时只触发引导，不执行任何关闭/注入动作。
    if let Err(e) = modules::process::ensure_antigravity_launch_path_configured() {
        if e.starts_with("APP_PATH_NOT_FOUND:") {
            let _ = app.emit(
                "app:path_missing",
                serde_json::json!({
                    "app": "antigravity",
                    "retry": { "kind": "switchAccount", "accountId": account_id }
                }),
            );
        }
        return Err(e);
    }

    // 2. 确保 Token 有效（自动刷新过期的 Token）
    let fresh_token = modules::oauth::ensure_fresh_token(&account.token)
        .await
        .map_err(|e| format!("Token 刷新失败: {}", e))?;

    // 如果 Token 更新了，保存回账号文件
    if fresh_token.access_token != account.token.access_token {
        modules::logger::log_info(&format!("Token 已刷新: {}", account.email));
        account.token = fresh_token.clone();
        modules::save_account(&account)?;
    }

    // 3. 更新工具内部状态
    modules::set_current_account_id(&account_id)?;
    account.update_last_used();
    modules::save_account(&account)?;

    // 4. 同步更新 Antigravity IDE 默认实例的绑定账号（不同步到 Codex，因为账号体系不同）
    if let Err(e) = modules::instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        modules::logger::log_warn(&format!("更新 Antigravity IDE 默认实例绑定账号失败: {}", e));
    } else {
        modules::logger::log_info(&format!(
            "已同步更新 Antigravity IDE 默认实例绑定账号: {}",
            account_id
        ));
    }

    // 5. 关闭受管进程：按默认实例目录关闭受管进程，等待其完全退出
    let default_dir = modules::instance::get_default_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();
    modules::process::close_antigravity_instances(&[default_dir_str], 20)?;
    let _ = modules::instance::update_default_pid(None);

    // 6. 进程完全退出后，执行磁盘级别的文件注入
    // 6.1 将账号 Token 注入默认实例目录
    modules::instance::inject_account_to_profile(&default_dir, &account_id)?;

    // 7. 启动 Antigravity IDE（带默认实例自定义启动参数；启动失败不阻断切号，保持原行为）
    modules::logger::log_info("正在启动 Antigravity IDE 默认实例...");
    let default_settings = modules::instance::load_default_settings()?;
    let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
    let launch_result = if extra_args.is_empty() {
        modules::process::start_antigravity()
    } else {
        modules::process::start_antigravity_with_args("", &extra_args)
    };
    let launch_error = match launch_result {
        Ok(pid) => {
            if let Err(e) = modules::instance::update_default_pid(Some(pid)) {
                modules::logger::log_warn(&format!("更新默认实例 PID 失败: {}", e));
            }
            None
        }
        Err(e) => {
            modules::logger::log_warn(&format!("Antigravity IDE 启动失败: {}", e));
            if e.starts_with("APP_PATH_NOT_FOUND:") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({ "app": "antigravity", "retry": { "kind": "default" } }),
                );
            }
            Some(e)
        }
    };

    if let Some(err) = launch_error {
        // 账号状态已经切换成功，仍广播账号切换事件，确保前端状态与本地落盘一致
        modules::websocket::broadcast_account_switched(&account.id, &account.email);
        if err.starts_with("APP_PATH_NOT_FOUND:") {
            return Err(err);
        }
        return Err(format!("账号已切换，但启动 Antigravity IDE 失败: {}", err));
    }

    modules::logger::log_info(&format!("账号切换完成: {}", account.email));

    // 广播切换完成通知
    modules::websocket::broadcast_account_switched(&account.id, &account.email);

    Ok(account)
}

#[tauri::command]
pub fn load_antigravity_switch_history(
) -> Result<Vec<modules::antigravity_switch_history::AntigravitySwitchHistoryItem>, String> {
    modules::antigravity_switch_history::load_history()
}

#[tauri::command]
pub fn clear_antigravity_switch_history() -> Result<(), String> {
    modules::antigravity_switch_history::clear_history()
}

#[tauri::command]
pub async fn update_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<models::Account, String> {
    let account = modules::update_account_tags(&account_id, tags)?;
    modules::websocket::broadcast_data_changed("account_tags_updated");
    Ok(account)
}

#[tauri::command]
pub async fn update_account_notes(
    account_id: String,
    notes: String,
) -> Result<models::Account, String> {
    let account = modules::account::update_account_notes(&account_id, notes)?;
    Ok(account)
}

/// 从本地客户端同步当前账号状态
/// 当前实现已禁用“跟随本地客户端当前账号”，保留空结果以兼容旧调用。
#[tauri::command]
pub async fn sync_current_from_client(_app: tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(None)
}

// ─── 账号分组持久化 ────────────────────────────────────────────

const GROUPS_FILE: &str = "account_groups.json";

#[tauri::command]
pub async fn load_account_groups() -> Result<String, String> {
    let path = modules::account::get_data_dir()?.join(GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read groups: {}", e))
}

#[tauri::command]
pub async fn save_account_groups(data: String) -> Result<(), String> {
    let dir = modules::account::get_data_dir()?;
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write groups: {}", e))
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_antigravity_runtime_target, resolve_antigravity_switch_flow,
        AntigravityRuntimeTarget, AntigravitySwitchFlow,
    };

    #[test]
    fn antigravity_switch_flow_uses_legacy_for_legacy_target() {
        assert_eq!(
            resolve_antigravity_switch_flow(AntigravityRuntimeTarget::Legacy, false, true),
            AntigravitySwitchFlow::Legacy
        );
    }

    #[test]
    fn antigravity_switch_flow_prefers_local_no_launch_for_ide_when_launch_disabled() {
        assert_eq!(
            resolve_antigravity_switch_flow(AntigravityRuntimeTarget::Ide, false, true),
            AntigravitySwitchFlow::LocalNoLaunch
        );
        assert_eq!(
            resolve_antigravity_switch_flow(AntigravityRuntimeTarget::Ide, false, false),
            AntigravitySwitchFlow::LocalNoLaunch
        );
    }

    #[test]
    fn antigravity_switch_flow_uses_dual_no_restart_only_when_launch_is_enabled() {
        assert_eq!(
            resolve_antigravity_switch_flow(AntigravityRuntimeTarget::Ide, true, true),
            AntigravitySwitchFlow::DualNoRestart
        );
        assert_eq!(
            resolve_antigravity_switch_flow(AntigravityRuntimeTarget::Ide, true, false),
            AntigravitySwitchFlow::Restart
        );
    }

    #[test]
    fn antigravity_runtime_target_defaults_to_ide() {
        assert_eq!(
            normalize_antigravity_runtime_target(None),
            AntigravityRuntimeTarget::Ide
        );
        assert_eq!(
            normalize_antigravity_runtime_target(Some("antigravity_ide")),
            AntigravityRuntimeTarget::Ide
        );
        assert_eq!(
            normalize_antigravity_runtime_target(Some("antigravity")),
            AntigravityRuntimeTarget::Legacy
        );
    }
}
