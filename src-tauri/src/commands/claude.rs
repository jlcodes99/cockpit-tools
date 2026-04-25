use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;
use tauri::AppHandle;

use crate::models::claude::ClaudeAccount;
use crate::modules::claude_account;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCommandInfo {
    pub account_id: String,
    pub config_dir: String,
    pub command: String,
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
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(target_os = "macos")]
fn preferred_shell_path() -> String {
    std::env::var("SHELL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

#[cfg(target_os = "macos")]
fn launch_in_ghostty(command: &str, action_label: &str) -> Result<String, String> {
    let shell = preferred_shell_path();
    let output = Command::new("open")
        .args(["-na", "Ghostty.app", "--args", "-e", &shell, "-lc", command])
        .output()
        .map_err(|e| format!("打开终端失败 (Ghostty): {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("终端执行失败: {}", stderr.trim()));
    }

    Ok(format!("已在 Ghostty 执行 Claude {}命令", action_label))
}

fn build_shell_command(envs: &[(String, String)], binary: PathBuf, args: Vec<String>) -> String {
    #[cfg(target_os = "windows")]
    {
        let mut parts: Vec<String> = envs
            .iter()
            .map(|(key, value)| format!("set \"{}={}\"", key, value.replace('"', "\"\"")))
            .collect();
        let mut command = windows_cmd_quote(&binary.to_string_lossy());
        for arg in args {
            if !arg.trim().is_empty() {
                command.push(' ');
                command.push_str(&windows_cmd_quote(arg.trim()));
            }
        }
        parts.push(command);
        return parts.join(" && ");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut command = envs
            .iter()
            .map(|(key, value)| format!("{}={}", key, posix_shell_quote(value)))
            .collect::<Vec<_>>()
            .join(" ");
        command.push(' ');
        command.push_str(&posix_shell_quote(&binary.to_string_lossy()));
        for arg in args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                command.push(' ');
                command.push_str(&posix_shell_quote(trimmed));
            }
        }
        command
    }
}

fn get_command_info(
    account_id: String,
    builder: impl Fn(&ClaudeAccount) -> Result<(PathBuf, Vec<String>), String>,
) -> Result<ClaudeCommandInfo, String> {
    let account = claude_account::load_account_checked(&account_id)?;
    let (binary, args) = builder(&account)?;
    let envs = claude_account::build_command_env_pairs(&account)?;
    Ok(ClaudeCommandInfo {
        account_id,
        config_dir: account.config_dir.clone(),
        command: build_shell_command(&envs, binary, args),
    })
}

fn execute_command_in_terminal(
    command: &str,
    terminal: Option<String>,
    action_label: &str,
) -> Result<String, String> {
    let config = crate::modules::config::get_user_config();
    let terminal = terminal
        .unwrap_or(config.default_terminal)
        .trim()
        .to_string();

    #[cfg(target_os = "macos")]
    {
        let is_iterm = terminal.to_lowercase().contains("iterm");
        let is_ghostty = terminal.eq_ignore_ascii_case("Ghostty");
        let is_terminal_app = terminal == "system" || terminal.is_empty() || terminal == "Terminal";
        let app_name = if is_terminal_app {
            "Terminal"
        } else {
            &terminal
        };

        let script = if is_iterm {
            format!(
                "tell application \"iTerm\"
                    activate
                    if not (exists window 1) then
                        create window with default profile
                        tell current session of current window
                            write text \"{}\"
                        end tell
                    else
                        tell current window
                            create tab with default profile
                            tell current session
                                write text \"{}\"
                            end tell
                        end tell
                    end if
                end tell",
                escape_applescript(command),
                escape_applescript(command)
            )
        } else if is_terminal_app {
            format!(
                "tell application \"Terminal\"
                    activate
                    do script \"{}\"
                end tell",
                escape_applescript(command)
            )
        } else if is_ghostty {
            return launch_in_ghostty(command, action_label);
        } else {
            return Err(format!(
                "当前终端暂不支持直接执行：{}。请改用 Terminal、iTerm2 或 Ghostty。",
                terminal
            ));
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("打开终端失败 ({}): {}", app_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("终端执行失败: {}", stderr.trim()));
        }
        return Ok(format!(
            "已在 {} 执行 Claude {}命令",
            app_name, action_label
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd;
        if terminal == "PowerShell" || terminal == "powershell" {
            cmd = Command::new("powershell");
            cmd.args(["-NoExit", "-Command", command]);
        } else if terminal == "pwsh" {
            cmd = Command::new("pwsh");
            cmd.args(["-NoExit", "-Command", command]);
        } else if terminal == "wt" {
            cmd = Command::new("wt");
            cmd.args(["-p", "Command Prompt", "cmd", "/K", command]);
        } else {
            cmd = Command::new("cmd");
            cmd.args(["/C", "start", "", "cmd", "/K", command]);
        }
        cmd.spawn().map_err(|e| format!("打开终端失败: {}", e))?;
        return Ok(format!("已在终端执行 Claude {}命令", action_label));
    }

    #[cfg(target_os = "linux")]
    {
        let shell_command = format!("{}; exec bash", command);
        let mut cmd = if terminal == "system" || terminal.is_empty() {
            Command::new("x-terminal-emulator")
        } else {
            Command::new(&terminal)
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
                        "指定终端未找到",
                    ))
                }
            })
            .map_err(|e| format!("打开终端失败: {}", e))?;
        Ok(format!("已在终端执行 Claude {}命令", action_label))
    }
}

#[tauri::command]
pub fn list_claude_accounts() -> Result<Vec<ClaudeAccount>, String> {
    claude_account::list_accounts_checked()
}

#[tauri::command]
pub fn create_claude_account(
    app: AppHandle,
    name: Option<String>,
    login_mode: Option<String>,
    login_hint_email: Option<String>,
    anthropic_base_url: Option<String>,
    anthropic_auth_token: Option<String>,
    disable_nonessential_traffic: Option<bool>,
) -> Result<ClaudeAccount, String> {
    let account = claude_account::create_account(
        name,
        login_mode,
        login_hint_email,
        anthropic_base_url,
        anthropic_auth_token,
        disable_nonessential_traffic,
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn delete_claude_account(app: AppHandle, account_id: String) -> Result<(), String> {
    claude_account::remove_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_claude_accounts(app: AppHandle, account_ids: Vec<String>) -> Result<(), String> {
    claude_account::remove_accounts(&account_ids)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub fn refresh_claude_account(app: AppHandle, account_id: String) -> Result<ClaudeAccount, String> {
    let account = claude_account::refresh_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn refresh_all_claude_accounts(app: AppHandle) -> Result<i32, String> {
    let refreshed = claude_account::refresh_all_accounts()?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(refreshed.len() as i32)
}

#[tauri::command]
pub fn inject_claude_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let account = claude_account::set_current_account(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    let label = if !account.email.trim().is_empty() {
        account.email
    } else if let Some(name) = account.name {
        name
    } else {
        account.id
    };
    Ok(format!("已切换当前 Claude profile: {}", label))
}

#[tauri::command]
pub fn update_claude_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<ClaudeAccount, String> {
    claude_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn export_claude_accounts(account_ids: Vec<String>) -> Result<String, String> {
    claude_account::export_accounts(&account_ids)
}

#[tauri::command]
pub fn import_claude_from_json(
    app: AppHandle,
    json_content: String,
) -> Result<Vec<ClaudeAccount>, String> {
    let accounts = claude_account::import_from_json(&json_content)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub fn get_claude_accounts_index_path() -> Result<String, String> {
    claude_account::accounts_index_path_string()
}

#[tauri::command]
pub fn get_claude_login_command(account_id: String) -> Result<ClaudeCommandInfo, String> {
    get_command_info(account_id, claude_account::build_login_command_args)
}

#[tauri::command]
pub fn get_claude_launch_command(account_id: String) -> Result<ClaudeCommandInfo, String> {
    get_command_info(account_id, claude_account::build_launch_command_args)
}

#[tauri::command]
pub fn execute_claude_login_command(
    account_id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let info = get_claude_login_command(account_id)?;
    execute_command_in_terminal(&info.command, terminal, "登录")
}

#[tauri::command]
pub fn execute_claude_launch_command(
    account_id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let info = get_claude_launch_command(account_id.clone())?;
    let _ = claude_account::mark_account_used(&account_id)?;
    execute_command_in_terminal(&info.command, terminal, "启动")
}
