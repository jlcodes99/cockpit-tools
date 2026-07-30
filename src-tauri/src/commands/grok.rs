use crate::models::grok::{GrokAccountView, GrokOAuthStartResponse};
use crate::modules::{config, grok_account, grok_oauth, logger};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;

#[cfg(target_os = "windows")]
const GROK_CLI_INSTALL_COMMAND: &str = "irm https://x.ai/cli/install.ps1 | iex";
#[cfg(not(target_os = "windows"))]
const GROK_CLI_INSTALL_COMMAND: &str = "curl -fsSL https://x.ai/cli/install.sh | bash";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokCliStatus {
    pub available: bool,
    pub binary_path: Option<String>,
    pub configured_path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub message: Option<String>,
    pub checked_at: i64,
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
        return Err(format!("Grok CLI 路径不存在: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|error| format!("读取 Grok CLI 路径失败: {}", error))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!("Grok CLI 路径不可执行: {}", path.display()));
        }
    }
    Ok(())
}

pub fn resolve_grok_cli_path() -> Result<(PathBuf, &'static str), String> {
    let config = crate::modules::config::get_user_config();
    if let Some(path) = config
        .grok_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = expand_home_path(path);
        validate_cli_file(&path).map_err(|error| format!("配置的 {}", error))?;
        return Ok((path, "configured"));
    }

    if let Some(home) = dirs::home_dir() {
        let names: &[&str] = if cfg!(target_os = "windows") {
            &["grok.exe", "grok.cmd", "grok.bat"]
        } else {
            &["grok"]
        };
        for root in [home.join(".grok/bin"), home.join(".local/bin")] {
            for name in names {
                let candidate = root.join(name);
                if validate_cli_file(&candidate).is_ok() {
                    return Ok((candidate, "common_path"));
                }
            }
        }
    }
    command_exists("grok")
        .map(|path| (path, "path"))
        .ok_or_else(|| "未检测到 Grok CLI，请先通过官方安装脚本安装".to_string())
}

fn fetch_grok_version(path: &Path) -> Option<String> {
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

pub(crate) fn parse_grok_client_version(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .map(|part| part.trim_start_matches(['v', 'V']))
        .find(|part| {
            !part.is_empty()
                && part.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
        })
        .map(str::to_string)
}

pub(crate) async fn detect_grok_client_version() -> Option<String> {
    let (path, _) = resolve_grok_cli_path().ok()?;
    let mut command = tokio::process::Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(std::time::Duration::from_secs(3), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_grok_client_version(&String::from_utf8_lossy(&output.stdout))
        .or_else(|| parse_grok_client_version(&String::from_utf8_lossy(&output.stderr)))
}

#[tauri::command]
pub fn grok_get_cli_status() -> Result<GrokCliStatus, String> {
    let checked_at = chrono::Utc::now().timestamp_millis();
    let configured_path = crate::modules::config::get_user_config()
        .grok_cli_path
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    match resolve_grok_cli_path() {
        Ok((path, source)) => Ok(GrokCliStatus {
            available: true,
            version: fetch_grok_version(&path),
            binary_path: Some(path.to_string_lossy().to_string()),
            configured_path,
            source: Some(source.to_string()),
            message: None,
            checked_at,
        }),
        Err(error) => Ok(GrokCliStatus {
            available: false,
            binary_path: None,
            configured_path,
            version: None,
            source: None,
            message: Some(error),
            checked_at,
        }),
    }
}

#[tauri::command]
pub fn grok_update_cli_runtime_config(
    grok_cli_path: Option<String>,
) -> Result<GrokCliStatus, String> {
    let normalized = grok_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home_path);
    if let Some(path) = normalized.as_deref() {
        validate_cli_file(path)?;
    }
    crate::modules::config::set_grok_cli_path(
        normalized.map(|path| path.to_string_lossy().to_string()),
    )?;
    grok_get_cli_status()
}

#[tauri::command]
pub fn grok_execute_cli_install_command(terminal: Option<String>) -> Result<(), String> {
    let command = GROK_CLI_INSTALL_COMMAND.to_string();

    #[cfg(target_os = "linux")]
    {
        let configured_terminal = crate::modules::config::get_user_config().default_terminal;
        let terminal = terminal.unwrap_or(configured_terminal).trim().to_string();
        let shell_command = format!("{}; exec bash", command);
        let open = |program: &str, args: &[&str]| {
            Command::new(program)
                .args(args)
                .spawn()
                .map(|_| ())
                .map_err(|error| error.to_string())
        };
        if terminal != "system" && !terminal.is_empty() {
            return open(&terminal, &["-e", "bash", "-lc", &shell_command])
                .map_err(|error| format!("打开终端失败 ({}): {}", terminal, error));
        }
        return open(
            "x-terminal-emulator",
            &["-e", "bash", "-lc", &shell_command],
        )
        .or_else(|_| open("gnome-terminal", &["--", "bash", "-lc", &shell_command]))
        .or_else(|_| open("konsole", &["-e", "bash", "-lc", &shell_command]))
        .map_err(|error| format!("未找到可用终端，无法执行 Grok CLI 安装命令: {}", error));
    }

    #[cfg(not(target_os = "linux"))]
    super::claude::execute_claude_cli_command(&command, terminal).map(|_| ())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::validate_cli_file;
    use super::{expand_home_path, parse_grok_client_version};

    #[test]
    fn expands_tilde_prefixed_cli_path() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        assert_eq!(
            expand_home_path("~/.local/bin/grok"),
            home.join(".local/bin/grok")
        );
    }

    #[test]
    fn parses_official_grok_version_output() {
        assert_eq!(
            parse_grok_client_version("grok 0.2.93 (f00f96316d4b)"),
            Some("0.2.93".to_string())
        );
        assert_eq!(
            parse_grok_client_version("grok v1.0.0-beta.1"),
            Some("1.0.0-beta.1".to_string())
        );
        assert_eq!(parse_grok_client_version("grok unknown"), None);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_executable_cli_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = std::env::temp_dir().join(format!(
            "cockpit-grok-cli-path-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, "not executable").expect("write test file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("set test permissions");
        assert!(validate_cli_file(&path).is_err());
        let _ = std::fs::remove_file(path);
    }
}

#[tauri::command]
pub fn list_grok_accounts() -> Result<Vec<GrokAccountView>, String> {
    grok_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_grok_account(account_id: String) -> Result<(), String> {
    grok_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_grok_accounts(account_ids: Vec<String>) -> Result<(), String> {
    grok_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_grok_from_json(json_content: String) -> Result<Vec<GrokAccountView>, String> {
    grok_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn add_grok_account_with_api_key(
    api_key: String,
    api_base_url: Option<String>,
    api_model: Option<String>,
) -> Result<GrokAccountView, String> {
    grok_account::upsert_api_key(&api_key, api_base_url.as_deref(), api_model.as_deref())
}

#[tauri::command]
pub fn import_grok_from_local() -> Result<Vec<GrokAccountView>, String> {
    grok_account::import_from_local()
}

#[tauri::command]
pub fn export_grok_accounts(account_ids: Vec<String>) -> Result<String, String> {
    grok_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn grok_oauth_login_start() -> Result<GrokOAuthStartResponse, String> {
    logger::log_info("[Grok OAuth] device flow 开始");
    grok_oauth::start_login().await
}

#[tauri::command]
pub async fn grok_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    reauth_account_id: Option<String>,
) -> Result<GrokAccountView, String> {
    let payload = grok_oauth::complete_login(&login_id).await?;
    let reauth_account_id = reauth_account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account = if let Some(account_id) = reauth_account_id {
        grok_account::upsert_oauth_for_reauth(payload, account_id)?
    } else {
        grok_account::upsert_oauth(payload)?
    };
    // 每账号独立 home：登录/重授权后刷新该账号 profile，不写官方默认 ~/.grok。
    if let Err(error) = grok_account::prepare_account_home(&account.id) {
        logger::log_warn(&format!(
            "[Grok OAuth] 准备独立 GROK_HOME 失败: account_id={}, error={}",
            account.id, error
        ));
    }
    let view = match grok_account::refresh_account(&account.id).await {
        Ok(view) => view,
        Err(error) => {
            logger::log_warn(&format!(
                "[Grok OAuth] 登录成功但首次刷新失败: account_id={}, error={}",
                account.id, error
            ));
            GrokAccountView::from(&account)
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(view)
}

#[tauri::command]
pub fn grok_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    grok_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_grok_account(
    app: AppHandle,
    account_id: String,
) -> Result<GrokAccountView, String> {
    let account = grok_account::refresh_account(&account_id).await?;
    if let Err(error) = grok_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[Grok Account] 配额预警检查失败: {}", error));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn force_refresh_grok_account(
    app: AppHandle,
    account_id: String,
) -> Result<GrokAccountView, String> {
    let account = grok_account::force_refresh_account(&account_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_grok_accounts(app: AppHandle) -> Result<i32, String> {
    let results = grok_account::refresh_all_accounts().await?;
    let success = results.iter().filter(|(_, result)| result.is_ok()).count() as i32;
    if let Err(error) = grok_account::run_quota_alert_if_needed() {
        logger::log_warn(&format!("[Grok Account] 配额预警检查失败: {}", error));
    }
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success)
}

#[tauri::command]
pub fn switch_grok_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let sync_official = config::get_user_config().grok_sync_official_auth_on_switch;
    let message = if sync_official {
        let email = grok_account::inject_to_default(&account_id)?;
        format!("已同步官方登录: {}", email)
    } else {
        let (email, home) = grok_account::prepare_account_home(&account_id)?;
        format!("已准备独立目录: {} ({})", email, home.display())
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(message)
}

#[tauri::command]
pub fn update_grok_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<GrokAccountView, String> {
    grok_account::update_tags(&account_id, tags)
}

#[tauri::command]
pub fn update_grok_account_working_dir(
    account_id: String,
    working_dir: Option<String>,
) -> Result<GrokAccountView, String> {
    grok_account::update_working_dir(&account_id, working_dir)
}

#[tauri::command]
pub fn get_grok_current_account_id() -> Result<Option<String>, String> {
    grok_account::current_account_id()
}

#[tauri::command]
pub fn get_grok_accounts_index_path() -> Result<String, String> {
    grok_account::accounts_index_path_string()
}
