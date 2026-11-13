use crate::error::{AppError, AppResult};
use crate::models;
use crate::modules;
use std::path::PathBuf;
use std::time::Duration;
use tauri::AppHandle;
use tauri::Emitter;

#[path = "account_auto_refresh.rs"]
pub(crate) mod auto_refresh;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AntigravityRuntimeTarget {
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

pub(crate) fn normalize_antigravity_runtime_target(raw: Option<&str>) -> AntigravityRuntimeTarget {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "antigravity" => AntigravityRuntimeTarget::Legacy,
        _ => AntigravityRuntimeTarget::Ide,
    }
}

impl AntigravityRuntimeTarget {
    fn platform_id(self) -> &'static str {
        match self {
            Self::Legacy => "antigravity",
            Self::Ide => "antigravity_ide",
        }
    }

    fn bound_account_id(self) -> Option<String> {
        let settings = match self {
            Self::Legacy => modules::antigravity_legacy_instance::load_default_settings(),
            Self::Ide => modules::instance::load_default_settings(),
        };
        settings.ok().and_then(|settings| settings.bind_account_id)
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
    tauri::async_runtime::spawn_blocking(modules::list_accounts)
        .await
        .map_err(|e| format!("列出账号任务失败: {}", e))?
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
        token_res
            .refresh_token
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(refresh_token),
        token_res.expires_in,
        Some(user_info.email.clone()),
        None,
        None,
    )
    .with_oauth_metadata(token_res.oauth_client_key, token_res.id_token);

    let account = tauri::async_runtime::spawn_blocking(move || {
        modules::upsert_account(user_info.email.clone(), user_info.get_display_name(), token)
    })
    .await
    .map_err(|error| error.to_string())??;
    modules::logger::log_info(&format!("添加账号成功: {}", account.email));

    // 广播通知
    modules::websocket::broadcast_data_changed("account_added");

    Ok(account)
}

#[tauri::command]
pub async fn create_pending_oauth_account(
    email: String,
    note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
    aux_email: Option<String>,
) -> Result<models::Account, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::account::create_pending_oauth_account(
            email,
            modules::account::AccountNoteUpdate {
                note,
                two_factor_secret,
                account_password,
                phone_number,
                mail_url,
                aux_email,
            },
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

const ANTIGRAVITY_MAIL_PREVIEW_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityMailPreviewFetchResult {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
    pub truncated: bool,
}

#[tauri::command]
pub async fn fetch_account_note_mail_url(
    mail_url: String,
) -> Result<AntigravityMailPreviewFetchResult, String> {
    let mail_url = mail_url.trim();
    if mail_url.is_empty() {
        return Err("MAIL_URL_EMPTY".to_string());
    }
    let parsed = reqwest::Url::parse(mail_url).map_err(|_| "MAIL_URL_INVALID".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("MAIL_URL_UNSUPPORTED_SCHEME".to_string());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("CockpitTools-MailPreview/1.0")
        .build()
        .map_err(|e| format!("MAIL_PREVIEW_CLIENT_FAILED: {}", e))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| format!("MAIL_PREVIEW_REQUEST_FAILED: {}", e))?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("MAIL_PREVIEW_READ_FAILED: {}", e))?;
    let truncated = bytes.len() > ANTIGRAVITY_MAIL_PREVIEW_MAX_BYTES;
    let visible = if truncated {
        &bytes[..ANTIGRAVITY_MAIL_PREVIEW_MAX_BYTES]
    } else {
        &bytes[..]
    };
    if !status.is_success() {
        return Err(format!("MAIL_PREVIEW_HTTP_FAILED:{}", status.as_u16()));
    }
    Ok(AntigravityMailPreviewFetchResult {
        status: status.as_u16(),
        content_type,
        body: String::from_utf8_lossy(visible).into_owned(),
        truncated,
    })
}

#[tauri::command]
pub async fn delete_account(account_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || modules::delete_account(&account_id))
        .await
        .map_err(|error| error.to_string())??;
    modules::websocket::broadcast_data_changed("account_deleted");
    Ok(())
}

#[tauri::command]
pub async fn delete_accounts(account_ids: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || modules::delete_accounts(&account_ids))
        .await
        .map_err(|error| error.to_string())??;
    modules::websocket::broadcast_data_changed("accounts_deleted");
    Ok(())
}

#[tauri::command]
pub async fn reorder_accounts(account_ids: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || modules::reorder_accounts(&account_ids))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn get_current_account(
    runtime_target: Option<String>,
) -> Result<Option<models::Account>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let runtime_target = normalize_antigravity_runtime_target(runtime_target.as_deref());
        let bound_account_id = runtime_target.bound_account_id();

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
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn set_current_account(
    app: tauri::AppHandle,
    account_id: String,
    runtime_target: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
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
        modules::set_current_account_id(&account_id)
    })
    .await
    .map_err(|error| error.to_string())??;
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
    trigger: Option<String>,
    runtime_target: Option<String>,
) -> Result<modules::account::RefreshStats, String> {
    let trigger = match trigger
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("auto") => modules::account::QuotaRefreshTrigger::Auto,
        _ => modules::account::QuotaRefreshTrigger::ManualBatch,
    };
    let result = modules::account::refresh_all_quotas_logic(trigger).await;
    if result.is_ok() {
        let mut switched = false;
        match run_auto_switch_for_runtime_target(&app, runtime_target.as_deref()).await {
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
pub async fn refresh_current_quota(
    app: tauri::AppHandle,
    runtime_target: Option<String>,
) -> Result<(), String> {
    refresh_current_quota_for_target(app, runtime_target)
        .await?
        .then_some(())
        .ok_or_else(|| "未找到当前账号".to_string())
}

async fn refresh_current_quota_for_target(
    app: tauri::AppHandle,
    runtime_target: Option<String>,
) -> Result<bool, String> {
    let Some(account) = get_current_account(runtime_target.clone()).await? else {
        return Ok(false);
    };
    let mut account = account;
    let quota = modules::fetch_quota_with_fresh_token(&mut account, true)
        .await
        .map_err(|e| e.to_string())?;
    modules::update_account_quota(&account.id, quota).map_err(|e| e.to_string())?;

    let mut switched = false;
    match run_auto_switch_for_runtime_target(&app, runtime_target.as_deref()).await {
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
    Ok(true)
}

async fn run_auto_switch_for_runtime_target(
    app: &AppHandle,
    runtime_target: Option<&str>,
) -> Result<Option<models::Account>, String> {
    if !modules::config::get_user_config().auto_switch_enabled {
        return Ok(None);
    }
    let target = normalize_antigravity_runtime_target(runtime_target);
    if !modules::tray_layout::is_platform_enabled(target.platform_id()) {
        return Ok(None);
    }
    let current_id = get_current_account(runtime_target.map(str::to_owned))
        .await?
        .map(|account| account.id);
    modules::account::run_auto_switch_if_needed(current_id, |account_id, reason| async move {
        match target {
            AntigravityRuntimeTarget::Legacy => {
                switch_account_legacy_antigravity(app.clone(), account_id).await
            }
            AntigravityRuntimeTarget::Ide => {
                modules::account::execute_ide_auto_switch(account_id, reason).await
            }
        }
    })
    .await
}

include!("account_desktop_switch.rs");

async fn switch_account_legacy_antigravity(
    app: AppHandle,
    account_id: String,
) -> Result<models::Account, String> {
    let result = switch_account_desktop(account_id.clone(), true).await;
    if let Err(error) = &result {
        if error.starts_with("APP_PATH_NOT_FOUND:") {
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
    }
    result
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

    let (account, launch_error) =
        match modules::account::switch_account_ide(&account_id, true).await {
            Ok(result) => result,
            Err(error) => {
                if error.starts_with("APP_PATH_NOT_FOUND:") {
                    let _ = app.emit(
                        "app:path_missing",
                        serde_json::json!({
                            "app": "antigravity",
                            "retry": { "kind": "switchAccount", "accountId": account_id }
                        }),
                    );
                }
                return Err(error);
            }
        };

    if let Some(err) = launch_error {
        if err.starts_with("APP_PATH_NOT_FOUND:") {
            let _ = app.emit(
                "app:path_missing",
                serde_json::json!({ "app": "antigravity", "retry": { "kind": "default" } }),
            );
        }
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
pub async fn load_antigravity_switch_history(
    retry: Option<bool>,
) -> Result<
    modules::wakeup_history_store::HistoryRead<
        modules::antigravity_switch_history::AntigravitySwitchHistoryItem,
    >,
    String,
> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::antigravity_switch_history::load_history(retry.unwrap_or(false))
    })
    .await
    .map_err(|error| format!("Antigravity 切号历史任务失败: {error}"))?
}

#[tauri::command]
pub async fn clear_antigravity_switch_history() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(modules::antigravity_switch_history::clear_history)
        .await
        .map_err(|error| format!("Antigravity 切号历史任务失败: {error}"))?
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

#[tauri::command]
pub async fn update_account_note(
    account_id: String,
    note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
    aux_email: Option<String>,
) -> Result<models::Account, String> {
    modules::account::update_account_note(
        &account_id,
        modules::account::AccountNoteUpdate {
            note,
            two_factor_secret,
            account_password,
            phone_number,
            mail_url,
            aux_email,
        },
    )
}

/// 从本地客户端同步当前账号状态
/// 当前实现已禁用“跟随本地客户端当前账号”，保留空结果以兼容旧调用。
#[tauri::command]
pub async fn sync_current_from_client(_app: tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(None)
}

// ─── 账号分组持久化 ────────────────────────────────────────────

use modules::account_groups_store::{self, GroupSnapshot};

#[tauri::command]
pub async fn load_account_group_snapshot(platform: String) -> Result<GroupSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || account_groups_store::store(&platform)?.snapshot())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn seed_account_groups(data: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        account_groups_store::store(account_groups_store::ANTIGRAVITY)?.seed_json(&data)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn load_account_groups() -> Result<String, String> {
    load_platform_account_groups(account_groups_store::ANTIGRAVITY.to_string()).await
}

#[tauri::command]
pub async fn save_account_groups(data: String, expected_revision: Option<u64>) -> Result<u64, String> {
    save_platform_account_groups(account_groups_store::ANTIGRAVITY.to_string(), data, expected_revision).await
}

#[tauri::command]
pub async fn load_platform_account_groups(platform: String) -> Result<String, String> {
    Ok(load_account_group_snapshot(platform).await?.data)
}

#[tauri::command]
pub async fn save_platform_account_groups(
    platform: String,
    data: String,
    expected_revision: Option<u64>,
) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        account_groups_store::store(&platform)?.replace_json(&data, expected_revision)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use crate::modules::account_groups_store::parse_groups;
    use super::{
        normalize_antigravity_runtime_target, resolve_antigravity_switch_flow,
        AntigravityRuntimeTarget, AntigravitySwitchFlow,
    };

    #[test]
    fn account_groups_payload_must_be_a_json_array() {
        assert!(parse_groups("[]").is_ok());
        assert!(parse_groups(r#"[{"id":"group-1","accountIds":[]}]"#).is_ok());
        assert!(parse_groups("{}").is_err());
        assert!(parse_groups("not-json").is_err());
    }

    #[test]
    fn account_groups_payload_rejects_invalid_account_ids() {
        assert!(parse_groups(r#"[{"id":"group-1","accountIds":{}}]"#).is_err());
    }

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
