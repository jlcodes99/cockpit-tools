use crate::models::pi::PiAccountView;
use crate::modules::{config, pi_account, pi_oauth};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::AppHandle;

#[cfg(target_os = "windows")]
const PI_CLI_INSTALL_COMMAND: &str = "irm https://pi.dev/install.ps1 | iex";
#[cfg(not(target_os = "windows"))]
const PI_CLI_INSTALL_COMMAND: &str = "curl -fsSL https://pi.dev/install.sh | sh";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiCliStatus {
    pub available: bool,
    pub binary_path: Option<String>,
    pub configured_path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub message: Option<String>,
    pub checked_at: i64,
}

#[cfg(target_os = "windows")]
fn is_windows_executable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| matches!(ext.to_ascii_lowercase().as_str(), "exe" | "cmd" | "bat"))
        .unwrap_or(false)
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    let candidates = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from);
    // npm installs an extensionless sh shim next to pi.cmd; PowerShell can only run the latter.
    #[cfg(target_os = "windows")]
    let candidates = candidates.filter(|path| is_windows_executable(path));
    candidates.into_iter().next()
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
        return Err(format!("pi CLI 路径不存在: {}", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|error| format!("读取 pi CLI 路径失败: {}", error))?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(format!("pi CLI 路径不可执行: {}", path.display()));
        }
    }
    Ok(())
}

fn common_cli_paths() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    if let Some(local) = dirs::data_local_dir() {
        roots.push(local.join("pi-node").join("current"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/bin"));
        roots.push(home.join(".npm-global/bin"));
    }
    if let Some(data) = dirs::data_dir() {
        roots.push(data.join("npm"));
    }
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["pi.cmd", "pi.exe"]
    } else {
        &["pi"]
    };
    roots
        .into_iter()
        .flat_map(|root| names.iter().map(move |name| root.join(name)))
        .collect()
}

pub fn resolve_pi_cli_path() -> Result<(PathBuf, &'static str), String> {
    if let Some(path) = config::get_user_config()
        .pi_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let path = expand_home_path(path);
        validate_cli_file(&path).map_err(|error| format!("配置的 {}", error))?;
        return Ok((path, "configured"));
    }
    if let Some(candidate) = common_cli_paths()
        .into_iter()
        .find(|candidate| validate_cli_file(candidate).is_ok())
    {
        return Ok((candidate, "common_path"));
    }
    command_exists("pi")
        .map(|path| (path, "path"))
        .ok_or_else(|| "未检测到 pi CLI，请先通过官方安装脚本安装".to_string())
}

fn parse_version(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .map(|part| part.trim_start_matches(['v', 'V']))
        .find(|part| {
            part.chars().next().is_some_and(|ch| ch.is_ascii_digit())
                && part
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
        })
        .map(str::to_string)
}

fn fetch_pi_version(path: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        // .cmd shims must go through cmd.exe.
        let mut command = Command::new("cmd.exe");
        command.arg("/C").arg(path);
        command.creation_flags(0x0800_0000);
        command
    };
    #[cfg(not(target_os = "windows"))]
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&output.stdout))
        .or_else(|| parse_version(&String::from_utf8_lossy(&output.stderr)))
}

fn configured_cli_path() -> Option<String> {
    config::get_user_config().pi_cli_path.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

#[tauri::command]
pub async fn pi_get_cli_status() -> Result<PiCliStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let checked_at = chrono::Utc::now().timestamp_millis();
        let configured_path = configured_cli_path();
        match resolve_pi_cli_path() {
            Ok((path, source)) => PiCliStatus {
                available: true,
                version: fetch_pi_version(&path),
                binary_path: Some(path.to_string_lossy().to_string()),
                configured_path,
                source: Some(source.to_string()),
                message: None,
                checked_at,
            },
            Err(error) => PiCliStatus {
                available: false,
                binary_path: None,
                configured_path,
                version: None,
                source: None,
                message: Some(error),
                checked_at,
            },
        }
    })
    .await
    .map_err(|error| format!("检测 pi CLI 失败: {}", error))
}

#[tauri::command]
pub async fn pi_update_cli_runtime_config(
    pi_cli_path: Option<String>,
) -> Result<PiCliStatus, String> {
    let normalized = pi_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(expand_home_path);
    if let Some(path) = normalized.as_deref() {
        validate_cli_file(path)?;
    }
    config::set_pi_cli_path(normalized.map(|path| path.to_string_lossy().to_string()))?;
    pi_get_cli_status().await
}

#[tauri::command]
pub fn pi_execute_cli_install_command(terminal: Option<String>) -> Result<(), String> {
    let command = PI_CLI_INSTALL_COMMAND.to_string();

    #[cfg(target_os = "linux")]
    {
        let configured_terminal = config::get_user_config().default_terminal;
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
        .map_err(|error| format!("未找到可用终端，无法执行 pi 安装命令: {}", error));
    }

    #[cfg(not(target_os = "linux"))]
    super::claude::execute_claude_cli_command(&command, terminal).map(|_| ())
}

/// Open a terminal running `pi` against the official home so the user can `/login`.
#[tauri::command]
pub fn pi_execute_login_command(terminal: Option<String>) -> Result<(), String> {
    let (binary, _) = resolve_pi_cli_path()?;
    let binary = binary.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    let command = format!("& '{}'", binary.replace('\'', "''"));
    #[cfg(not(target_os = "windows"))]
    let command = format!("'{}'", binary.replace('\'', "'\"'\"'"));
    super::claude::execute_claude_cli_command(&command, terminal).map(|_| ())
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|error| format!("pi 账号任务失败: {}", error))?
}

#[tauri::command]
pub async fn list_pi_accounts() -> Result<Vec<PiAccountView>, String> {
    blocking(pi_account::list_accounts).await
}

#[tauri::command]
pub async fn delete_pi_account(app: AppHandle, account_id: String) -> Result<(), String> {
    blocking(move || pi_account::delete_accounts(&[account_id])).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_pi_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    blocking(move || pi_account::delete_accounts(&account_ids)).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn import_pi_from_json(json_content: String) -> Result<Vec<PiAccountView>, String> {
    pi_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn add_pi_account_with_api_key(
    provider: String,
    api_key: String,
    display_name: Option<String>,
    default_model: Option<String>,
) -> Result<PiAccountView, String> {
    pi_account::add_with_api_key(&provider, &api_key, display_name, default_model)
}

#[tauri::command]
pub fn import_pi_from_local() -> Result<Vec<PiAccountView>, String> {
    pi_account::import_from_local()
}

#[tauri::command]
pub fn export_pi_accounts(account_ids: Vec<String>) -> Result<String, String> {
    pi_account::export_accounts(&account_ids)
}

#[tauri::command]
pub fn switch_pi_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let message = if config::get_user_config().pi_sync_official_auth_on_switch {
        let name = pi_account::inject_to_default(&account_id)?;
        format!("已同步官方登录: {}", name)
    } else {
        let (name, home) = pi_account::prepare_account_home(&account_id)?;
        format!("已准备独立目录: {} ({})", name, home.display())
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(message)
}

#[tauri::command]
pub fn update_pi_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<PiAccountView, String> {
    pi_account::update_tags(&account_id, tags)
}

#[tauri::command]
pub fn update_pi_account_working_dir(
    account_id: String,
    working_dir: Option<String>,
) -> Result<PiAccountView, String> {
    pi_account::update_working_dir(&account_id, working_dir)
}

#[tauri::command]
pub fn update_pi_account_defaults(
    account_id: String,
    display_name: Option<String>,
    default_provider: Option<String>,
    default_model: Option<String>,
    default_thinking_level: Option<String>,
) -> Result<PiAccountView, String> {
    pi_account::update_defaults(
        &account_id,
        display_name,
        default_provider,
        default_model,
        default_thinking_level,
    )
}

#[tauri::command]
pub fn get_pi_current_account_id() -> Result<Option<String>, String> {
    pi_account::current_account_id()
}

#[tauri::command]
pub fn get_pi_accounts_index_path() -> Result<String, String> {
    pi_account::accounts_index_path_string()
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn parses_pi_version_output() {
        assert_eq!(parse_version("0.84.3"), Some("0.84.3".to_string()));
        assert_eq!(
            parse_version("pi v1.2.0-beta"),
            Some("1.2.0-beta".to_string())
        );
        assert_eq!(parse_version("pi unknown"), None);
    }
}

#[tauri::command]
pub async fn pi_oauth_login_start(
    provider: String,
) -> Result<pi_oauth::PiOAuthStartResponse, String> {
    pi_oauth::start_login(&provider).await
}

#[tauri::command]
pub async fn pi_oauth_login_complete(login_id: String) -> Result<PiAccountView, String> {
    pi_oauth::complete_login(&login_id).await
}

#[tauri::command]
pub fn pi_oauth_login_cancel(login_id: Option<String>) {
    pi_oauth::cancel_login(login_id.as_deref());
}

#[tauri::command]
pub fn pi_oauth_submit_callback(login_id: String, callback_url: String) -> Result<(), String> {
    pi_oauth::submit_callback(&login_id, &callback_url)
}
