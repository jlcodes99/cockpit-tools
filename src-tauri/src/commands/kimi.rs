use crate::models::kimi::{KimiAccountView, KimiOAuthStartResponse};
use crate::modules::{config, kimi_account, kimi_oauth, logger};
use serde::Serialize;
#[cfg(unix)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCliLaunchInfo {
    pub home: String,
    pub binary_path: String,
    pub launch_command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiCliStatus {
    pub available: bool,
    pub binary_path: Option<String>,
    pub configured_path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub message: Option<String>,
    pub checked_at: i64,
    pub home: String,
    pub configured_home: Option<String>,
}

fn command_exists(name: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("where.exe");
        command.creation_flags(0x0800_0000);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new("which");
    let output = command
        .arg(name)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn expand_home_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(relative) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Some(home) = dirs::home_dir() {
            return home.join(relative);
        }
    }
    PathBuf::from(trimmed)
}

fn validate_cli_file(path: &Path) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("Kimi CLI 路径不存在: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|error| format!("读取 Kimi CLI 路径失败: {}", error))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!("Kimi CLI 路径不可执行: {}", path.display()));
        }
    }
    Ok(())
}

fn resolve_kimi_cli_path() -> Result<(PathBuf, &'static str), String> {
    let user_config = config::get_user_config();
    if let Some(path) = user_config
        .kimi_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = expand_home_path(path);
        validate_cli_file(&path).map_err(|error| format!("配置的 {}", error))?;
        return Ok((path, "configured"));
    }

    let names: &[&str] = if cfg!(target_os = "windows") {
        &["kimi.exe", "kimi.cmd", "kimi.bat", "kimi"]
    } else {
        &["kimi"]
    };
    let home = kimi_account::default_kimi_home()?;
    let mut roots = vec![home.join("bin")];
    if let Some(user_home) = dirs::home_dir() {
        roots.push(user_home.join(".kimi-code").join("bin"));
        roots.push(user_home.join(".local").join("bin"));
    }
    for root in roots {
        for name in names {
            let candidate = root.join(name);
            if validate_cli_file(&candidate).is_ok() {
                return Ok((candidate, "common_path"));
            }
        }
    }
    for name in names {
        if let Some(path) = command_exists(name) {
            if validate_cli_file(&path).is_ok() {
                return Ok((path, "path"));
            }
        }
    }
    Err("未检测到 Kimi Code CLI。请先安装官方 CLI（https://code.kimi.com）。".to_string())
}

fn fetch_kimi_version(path: &Path) -> Option<String> {
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn kimi_launch_working_dir(home: &Path) -> PathBuf {
    if let Some(user_home) = dirs::home_dir().filter(|path| path.is_dir()) {
        return user_home;
    }
    if home.is_dir() {
        return home.to_path_buf();
    }
    PathBuf::from(".")
}

fn build_kimi_launch_command(binary: &Path, home: &Path) -> String {
    let cwd = kimi_launch_working_dir(home);
    #[cfg(target_os = "windows")]
    {
        format!(
            "Set-Location {}; $env:KIMI_CODE_HOME={}; & {}",
            powershell_quote(&cwd.to_string_lossy()),
            powershell_quote(&home.to_string_lossy()),
            powershell_quote(&binary.to_string_lossy())
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!(
            "cd {} && KIMI_CODE_HOME={} {}",
            shell_quote(&cwd.to_string_lossy()),
            shell_quote(&home.to_string_lossy()),
            shell_quote(&binary.to_string_lossy())
        )
    }
}

fn prepare_kimi_launch() -> Result<KimiCliLaunchInfo, String> {
    let home = kimi_account::default_kimi_home()?;
    let (binary, _) = resolve_kimi_cli_path()?;
    Ok(KimiCliLaunchInfo {
        launch_command: build_kimi_launch_command(&binary, &home),
        home: home.to_string_lossy().to_string(),
        binary_path: binary.to_string_lossy().to_string(),
    })
}

fn launch_kimi_cli_in_terminal() -> Result<String, String> {
    let info = prepare_kimi_launch()?;
    let binary = PathBuf::from(&info.binary_path);
    let home = PathBuf::from(&info.home);
    #[cfg(target_os = "windows")]
    {
        launch_kimi_cli_on_windows(&binary, &home)
    }
    #[cfg(not(target_os = "windows"))]
    {
        crate::commands::claude::execute_claude_cli_command(&info.launch_command, None)
    }
}

#[cfg(target_os = "windows")]
fn launch_kimi_cli_on_windows(binary: &Path, home: &Path) -> Result<String, String> {
    use std::os::windows::process::CommandExt;

    // Official kimi is a Node TUI. Wrapping it in `cmd start "" powershell`
    // inherits Cockpit's cwd (often src-tauri) and trips Node's
    // `process_title` assertion. Spawn the official binary in its own console.
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    let cwd = kimi_launch_working_dir(home);
    let mut command = Command::new(binary);
    command
        .env("KIMI_CODE_HOME", home)
        .current_dir(&cwd)
        .creation_flags(CREATE_NEW_CONSOLE);
    command
        .spawn()
        .map_err(|error| format!("打开 Kimi CLI 失败: {}", error))?;
    Ok("已打开 Kimi CLI".to_string())
}

fn configured_kimi_home() -> Option<String> {
    config::get_user_config().kimi_code_home.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

#[tauri::command]
pub fn kimi_get_cli_status() -> Result<KimiCliStatus, String> {
    let checked_at = chrono::Utc::now().timestamp_millis();
    let configured_path = config::get_user_config().kimi_cli_path.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let configured_home = configured_kimi_home();
    let home = kimi_account::default_kimi_home()?
        .to_string_lossy()
        .to_string();
    match resolve_kimi_cli_path() {
        Ok((path, source)) => Ok(KimiCliStatus {
            available: true,
            version: fetch_kimi_version(&path),
            binary_path: Some(path.to_string_lossy().to_string()),
            configured_path,
            source: Some(source.to_string()),
            message: None,
            checked_at,
            home,
            configured_home,
        }),
        Err(error) => Ok(KimiCliStatus {
            available: false,
            binary_path: None,
            configured_path,
            version: None,
            source: None,
            message: Some(error),
            checked_at,
            home,
            configured_home,
        }),
    }
}

#[tauri::command]
pub fn kimi_update_cli_runtime_config(
    kimi_cli_path: Option<String>,
) -> Result<KimiCliStatus, String> {
    let normalized = kimi_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home_path);
    if let Some(path) = normalized.as_deref() {
        validate_cli_file(path)?;
    }
    config::set_kimi_cli_path(normalized.map(|path| path.to_string_lossy().to_string()))?;
    kimi_get_cli_status()
}

#[tauri::command]
pub fn kimi_update_home_config(kimi_code_home: Option<String>) -> Result<KimiCliStatus, String> {
    let normalized = kimi_code_home
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home_path);
    config::set_kimi_code_home(normalized.map(|path| path.to_string_lossy().to_string()))?;
    kimi_get_cli_status()
}

#[tauri::command]
pub fn list_kimi_accounts() -> Result<Vec<KimiAccountView>, String> {
    kimi_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_kimi_account(account_id: String) -> Result<(), String> {
    kimi_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_kimi_accounts(account_ids: Vec<String>) -> Result<(), String> {
    kimi_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_kimi_from_json(json_content: String) -> Result<Vec<KimiAccountView>, String> {
    kimi_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn import_kimi_from_local() -> Result<Vec<KimiAccountView>, String> {
    kimi_account::import_from_local()
}

#[tauri::command]
pub fn export_kimi_accounts(account_ids: Vec<String>) -> Result<String, String> {
    kimi_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn kimi_oauth_login_start() -> Result<KimiOAuthStartResponse, String> {
    logger::log_info("[Kimi OAuth] device flow 开始");
    kimi_oauth::start_login().await
}

#[tauri::command]
pub async fn kimi_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    reauth_account_id: Option<String>,
) -> Result<KimiAccountView, String> {
    let (token, device_id, expires_at, expires_in) = kimi_oauth::complete_login(&login_id).await?;
    let payload = crate::models::kimi::KimiOAuthCompletePayload {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        token_type: token.token_type,
        scope: token.scope,
        expires_at,
        expires_in,
        device_id,
        email: "unknown@kimi.local".to_string(),
        nickname: None,
        user_id: None,
        avatar: None,
        plan_type: Some("Kimi Code".to_string()),
    };
    let reauth_account_id = reauth_account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account = if let Some(account_id) = reauth_account_id {
        kimi_account::upsert_oauth_for_reauth(payload, account_id)?
    } else {
        kimi_account::upsert_oauth(payload)?
    };
    let view = match kimi_account::hydrate_profile_only(&account.id).await {
        Ok(view) => view,
        Err(error) => {
            logger::log_warn(&format!(
                "[Kimi OAuth] 登录成功但拉取资料失败: account_id={}, error={}",
                account.id, error
            ));
            KimiAccountView::from(&account)
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(view)
}

#[tauri::command]
pub fn kimi_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    kimi_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_kimi_account(
    app: AppHandle,
    account_id: String,
) -> Result<KimiAccountView, String> {
    let account = kimi_account::refresh_account(&account_id).await?;
    if let Err(error) = kimi_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[Kimi Account] 配额预警检查失败: {}", error));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_kimi_accounts(app: AppHandle) -> Result<i32, String> {
    let results = kimi_account::refresh_all_accounts().await?;
    let success = results.iter().filter(|(_, result)| result.is_ok()).count() as i32;
    if let Err(error) = kimi_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[Kimi Account] 配额预警检查失败: {}", error));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success)
}

#[tauri::command]
pub async fn switch_kimi_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let user_config = config::get_user_config();
    let email = kimi_account::inject_to_default(&account_id).await?;
    let mut message = format!("已写入官方配置（{}）", email);
    if user_config.kimi_launch_on_switch {
        match launch_kimi_cli_in_terminal() {
            Ok(_) => {
                message.push_str("，已打开 Kimi CLI 终端");
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "[Kimi Account] 切号成功但打开终端失败: account_id={}, error={}",
                    account_id, error
                ));
                message.push_str(&format!("；打开终端失败: {}", error));
            }
        }
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(message)
}

#[tauri::command]
pub fn update_kimi_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<KimiAccountView, String> {
    kimi_account::update_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_kimi_current_account_id() -> Result<Option<String>, String> {
    kimi_account::current_account_id()
}

#[tauri::command]
pub fn get_kimi_accounts_index_path() -> Result<String, String> {
    kimi_account::accounts_index_path_string()
}

#[cfg(test)]
mod tests {
    use super::build_kimi_launch_command;
    use std::path::Path;

    #[cfg(target_os = "windows")]
    #[test]
    fn launch_command_sets_home_and_binary_for_powershell() {
        let command = build_kimi_launch_command(
            Path::new(r"C:\Users\demo\.kimi-code\bin\kimi.exe"),
            Path::new(r"E:\Save\Temp\cockpit-kimi-isolated\kimi-home"),
        );
        assert!(command.contains("Set-Location "));
        assert!(command.contains("$env:KIMI_CODE_HOME="));
        assert!(command.contains(r"E:\Save\Temp\cockpit-kimi-isolated\kimi-home"));
        assert!(command.contains(r"C:\Users\demo\.kimi-code\bin\kimi.exe"));
        assert!(command.contains("& "));
    }

    #[test]
    fn launch_working_dir_prefers_user_home() {
        let fallback = Path::new(r"E:\Save\Temp\cockpit-kimi-isolated\kimi-home");
        let cwd = super::kimi_launch_working_dir(fallback);
        if let Some(user_home) = dirs::home_dir() {
            assert_eq!(cwd, user_home);
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn launch_command_sets_home_and_binary_for_unix() {
        let command = build_kimi_launch_command(
            Path::new("/home/demo/.kimi-code/bin/kimi"),
            Path::new("/tmp/kimi-home"),
        );
        assert!(command.contains("KIMI_CODE_HOME="));
        assert!(command.contains("/tmp/kimi-home"));
        assert!(command.contains("/home/demo/.kimi-code/bin/kimi"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn powershell_quote_escapes_single_quotes() {
        assert_eq!(super::powershell_quote("O'Brien"), "'O''Brien'");
    }
}
