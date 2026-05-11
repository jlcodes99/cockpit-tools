use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Read dsw account list
#[tauri::command]
pub async fn read_devin_cli_accounts() -> Result<Vec<DevinCliAccount>, String> {
    let dsw_dir = resolve_dsw_dir()?;
    let store_path = Path::new(&dsw_dir).join("accounts.json");

    if !store_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&store_path)
        .map_err(|e| format!("Failed to read dsw accounts file: {}", e))?;

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    let store: DswStore = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse dsw accounts file: {}", e))?;

    Ok(store.accounts)
}

/// Check if dsw is installed
#[tauri::command]
pub async fn devin_cli_is_installed() -> Result<bool, String> {
    Ok(which_dsw().is_some())
}

/// Execute a dsw command in a terminal
#[tauri::command]
pub async fn execute_devin_cli_command(
    args: Vec<String>,
    terminal: Option<String>,
) -> Result<String, String> {
    validate_dsw_args(&args)?;
    let dsw_path = which_dsw().ok_or("dsw (Devin Switcher) not found in PATH. Please install it first: npm install -g @itsddvn/dsw")?;

    let command = build_dsw_command(&dsw_path, &args);

    #[cfg(target_os = "macos")]
    {
        let app_name = terminal.as_deref().unwrap_or("Terminal");
        let script = if app_name == "iTerm" {
            format!(
                "tell application \"iTerm\"\n  create window with default profile\n  tell current session of current window\n    write text \"{}\"\n  end tell\nend tell",
                escape_applescript(&command)
            )
        } else {
            format!(
                "tell application \"Terminal\"\n  do script \"{}\"\n  activate\nend tell",
                escape_applescript(&command)
            )
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("Failed to open terminal ({}): {}", app_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Terminal execution failed: {}", stderr.trim()));
        }
        return Ok(format!("Executed Devin CLI command in {}", app_name));
    }

    #[cfg(target_os = "windows")]
    {
        let terminal = terminal.as_deref().unwrap_or("cmd");
        let mut cmd;
        if terminal == "PowerShell" || terminal == "powershell" {
            cmd = Command::new("powershell");
            cmd.args(["-NoExit", "-Command", &command]);
        } else if terminal == "pwsh" {
            cmd = Command::new("pwsh");
            cmd.args(["-NoExit", "-Command", &command]);
        } else if terminal == "wt" {
            cmd = Command::new("wt");
            cmd.args(["-p", "Command Prompt", "cmd", "/K", &command]);
        } else {
            cmd = Command::new("cmd");
            cmd.args(["/C", "start", "", "cmd", "/K", &command]);
        }

        cmd.spawn().map_err(|e| format!("Failed to open terminal: {}", e))?;
        return Ok("Executed Devin CLI command in terminal".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let shell_command = format!("{}; exec bash", command);
        let terminal = terminal.as_deref().unwrap_or("system");
        let mut cmd = if terminal == "system" || terminal.is_empty() {
            Command::new("x-terminal-emulator")
        } else {
            Command::new(terminal)
        };

        cmd.args(["-e", "bash", "-lc", &shell_command])
            .spawn()
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("gnome-terminal")
                        .args(["--", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Terminal not found",
                    ))
                }
            })
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("konsole")
                        .args(["-e", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Terminal not found",
                    ))
                }
            })
            .or_else(|_| Command::new("sh").args(["-lc", &command]).spawn())
            .map_err(|e| format!("Failed to execute Devin CLI command: {}", e))?;
        return Ok("Executed Devin CLI command".to_string());
    }

    #[allow(unreachable_code)]
    Ok("Executed".to_string())
}

fn which_dsw() -> Option<String> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["dsw.cmd", "dsw.exe", "dsw"]
    } else {
        vec!["dsw"]
    };

    let paths = std::env::var_os("PATH")?;
    for path_dir in std::env::split_paths(&paths) {
        for candidate in &candidates {
            let full_path = path_dir.join(candidate);
            if full_path.is_file() {
                return Some(full_path.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn resolve_dsw_dir() -> Result<String, String> {
    if let Ok(data_home) = std::env::var("DSW_DATA_HOME") {
        let trimmed = data_home.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    let home = dirs::home_dir().ok_or("Unable to determine home directory")?;
    Ok(home.join(".dsw").to_string_lossy().to_string())
}

fn validate_dsw_args(args: &[String]) -> Result<(), String> {
    const MAX_ARG_COUNT: usize = 32;
    const MAX_ARG_LEN: usize = 512;
    if args.len() > MAX_ARG_COUNT {
        return Err("Too many dsw arguments".to_string());
    }
    for arg in args {
        if arg.len() > MAX_ARG_LEN {
            return Err("dsw argument is too long".to_string());
        }
        if arg.contains('\n') || arg.contains('\r') || arg.contains('\0') {
            return Err("dsw argument contains unsupported control characters".to_string());
        }
    }
    Ok(())
}

fn build_dsw_command(dsw_path: &str, args: &[String]) -> String {
    let mut parts = Vec::new();
    #[cfg(target_os = "windows")]
    {
        parts.push(windows_cmd_quote(dsw_path));
    }
    #[cfg(not(target_os = "windows"))]
    {
        parts.push(posix_shell_quote(dsw_path));
    }
    for arg in args {
        #[cfg(target_os = "windows")]
        {
            parts.push(windows_cmd_quote(arg));
        }
        #[cfg(not(target_os = "windows"))]
        {
            parts.push(posix_shell_quote(arg));
        }
    }
    parts.join(" ")
}

#[cfg(not(target_os = "windows"))]
fn posix_shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let needs_quote = value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | '$' | '`' | '\\' | '&' | '|' | ';' | '<' | '>' | '(' | ')'
            )
    });
    if !needs_quote {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn windows_cmd_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quote = value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '^' | '&' | '|' | '<' | '>' | '%'));
    if !needs_quote {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevinCliAccount {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub tier: Option<String>,
    pub plan: Option<String>,
    pub org_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub needs_login: bool,
}

#[derive(Debug, Deserialize)]
struct DswStore {
    #[serde(default)]
    pub accounts: Vec<DevinCliAccount>,
}
