use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::models::InstanceProfileView;
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn is_profile_initialized(user_data_dir: &str) -> bool {
    modules::gemini_instance::is_profile_initialized(Path::new(user_data_dir))
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

struct GeminiLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
    use_home_env: bool,
}

fn resolve_instance_launch_context(instance_id: &str) -> Result<GeminiLaunchContext, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::gemini_instance::get_default_gemini_cli_home_root()?;
        let default_settings = modules::gemini_instance::load_default_settings()?;
        return Ok(GeminiLaunchContext {
            user_data_dir: default_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: default_settings.extra_args,
            use_home_env: false,
        });
    }

    let store = modules::gemini_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    Ok(GeminiLaunchContext {
        user_data_dir: instance.user_data_dir,
        working_dir: instance.working_dir,
        extra_args: instance.extra_args,
        use_home_env: true,
    })
}

fn build_launch_command(context: &GeminiLaunchContext) -> String {
    let parsed_args = modules::process::parse_extra_args(&context.extra_args);

    let mut command_parts = Vec::new();

    // 如果指定了工作目录，先 cd 过去
    if let Some(ref dir) = context.working_dir {
        if !dir.trim().is_empty() {
            #[cfg(target_os = "windows")]
            command_parts.push(format!("cd /d \"{}\"", dir.replace("\"", "\"\"")));
            #[cfg(not(target_os = "windows"))]
            command_parts.push(format!("cd {}", posix_shell_quote(dir)));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if context.use_home_env {
            let escaped_home = context.user_data_dir.replace('"', "\"\"");
            command_parts.push(format!("set \"GEMINI_CLI_HOME={}\"", escaped_home));
        }

        let mut gemini_cmd = "gemini".to_string();
        for arg in parsed_args {
            if !arg.trim().is_empty() {
                gemini_cmd.push(' ');
                gemini_cmd.push_str(&windows_cmd_quote(arg.trim()));
            }
        }
        command_parts.push(gemini_cmd);
        return command_parts.join(" && ");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut gemini_cmd = if context.use_home_env {
            format!(
                "GEMINI_CLI_HOME={} gemini",
                posix_shell_quote(&context.user_data_dir)
            )
        } else {
            "gemini".to_string()
        };

        for arg in parsed_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                gemini_cmd.push(' ');
                gemini_cmd.push_str(&posix_shell_quote(trimmed));
            }
        }
        command_parts.push(gemini_cmd);
        command_parts.join(" && ")
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
}

#[tauri::command]
pub async fn gemini_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::gemini_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn gemini_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::gemini_instance::load_instance_store()?;
    let default_dir = modules::gemini_instance::get_default_gemini_cli_home_root()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();
    let default_settings = store.default_settings.clone();

    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let initialized = is_profile_initialized(&instance.user_data_dir);
            InstanceProfileView::from_profile(instance, false, initialized)
        })
        .collect();

    result.push(InstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: default_dir_str,
        working_dir: None,
        extra_args: default_settings.extra_args.clone(),
        bind_account_id: default_settings.bind_account_id.clone(),
        created_at: 0,
        last_launched_at: None,
        last_pid: None,
        running: false,
        initialized: modules::gemini_instance::is_profile_initialized(&default_dir),
        is_default: true,
        follow_local_account: false,
    });

    Ok(result)
}

#[tauri::command]
pub async fn gemini_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance = modules::gemini_instance::create_instance(
        modules::gemini_instance::CreateInstanceParams {
            name,
            user_data_dir,
            working_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
        },
    )?;

    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn gemini_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::gemini_instance::get_default_gemini_cli_home_root()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::gemini_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;

        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: updated.extra_args,
            bind_account_id: updated.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
            running: false,
            initialized: modules::gemini_instance::is_profile_initialized(&default_dir),
            is_default: true,
            follow_local_account: false,
        });
    }

    let wants_bind = bind_account_id
        .as_ref()
        .and_then(|next| next.as_ref())
        .is_some();
    if wants_bind {
        let store = modules::gemini_instance::load_instance_store()?;
        if let Some(target) = store.instances.iter().find(|item| item.id == instance_id) {
            if !is_profile_initialized(&target.user_data_dir) {
                return Err(
                    "INSTANCE_NOT_INITIALIZED:请先启动一次实例创建数据后，再进行账号绑定"
                        .to_string(),
                );
            }
        }
    }

    let instance = modules::gemini_instance::update_instance(
        modules::gemini_instance::UpdateInstanceParams {
            instance_id,
            name,
            working_dir,
            extra_args,
            bind_account_id,
        },
    )?;
    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn gemini_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::gemini_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn gemini_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::gemini_instance::get_default_gemini_cli_home_root()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::gemini_instance::load_default_settings()?;
        if let Some(ref account_id) = default_settings.bind_account_id {
            modules::gemini_account::inject_to_gemini_home(account_id, None)?;
        }
        let _ = modules::gemini_instance::update_default_pid(None)?;
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: default_settings.extra_args,
            bind_account_id: default_settings.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
            running: false,
            initialized: modules::gemini_instance::is_profile_initialized(&default_dir),
            is_default: true,
            follow_local_account: false,
        });
    }

    let store = modules::gemini_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(ref account_id) = instance.bind_account_id {
        modules::gemini_account::inject_to_gemini_home(
            account_id,
            Some(Path::new(&instance.user_data_dir)),
        )?;
    }

    let updated = modules::gemini_instance::update_instance_last_launched(&instance.id)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn gemini_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::gemini_instance::get_default_gemini_cli_home_root()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::gemini_instance::load_default_settings()?;
        let _ = modules::gemini_instance::update_default_pid(None)?;
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: default_settings.extra_args,
            bind_account_id: default_settings.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
            running: false,
            initialized: modules::gemini_instance::is_profile_initialized(&default_dir),
            is_default: true,
            follow_local_account: false,
        });
    }

    let updated = modules::gemini_instance::update_instance_pid(&instance_id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn gemini_open_instance_window(_instance_id: String) -> Result<(), String> {
    Err("Gemini Cli 不支持窗口定位，请使用“启动”后的命令在终端中运行".to_string())
}

#[tauri::command]
pub async fn gemini_close_all_instances() -> Result<(), String> {
    modules::gemini_instance::clear_all_pids()
}

#[tauri::command]
pub async fn gemini_get_instance_launch_command(
    instance_id: String,
) -> Result<GeminiInstanceLaunchInfo, String> {
    let context = resolve_instance_launch_context(&instance_id)?;
    Ok(GeminiInstanceLaunchInfo {
        instance_id,
        launch_command: build_launch_command(&context),
        user_data_dir: context.user_data_dir,
    })
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[tauri::command]
pub async fn gemini_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let context = resolve_instance_launch_context(&instance_id)?;
    let command = build_launch_command(&context);

    let config = crate::modules::config::get_user_config();
    let terminal = terminal
        .unwrap_or(config.default_terminal)
        .trim()
        .to_string();

    #[cfg(target_os = "macos")]
    {
        let is_iterm = terminal.to_lowercase().contains("iterm");
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
                escape_applescript(&command),
                escape_applescript(&command)
            )
        } else if is_terminal_app {
            format!(
                "tell application \"Terminal\"
                    activate
                    do script \"{}\"
                end tell",
                escape_applescript(&command)
            )
        } else {
            return Err(format!(
                "当前终端暂不支持直接执行：{}。请改用 Terminal 或 iTerm2。",
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
        return Ok(format!("已在 {} 执行 Gemini Cli 命令", app_name));
    }

    #[cfg(target_os = "windows")]
    {
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
            // 默认为 cmd
            cmd = Command::new("cmd");
            cmd.args(["/C", "start", "", "cmd", "/K", &command]);
        }

        cmd.spawn().map_err(|e| format!("打开终端失败: {}", e))?;
        return Ok("已在终端执行 Gemini Cli 命令".to_string());
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
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("konsole")
                        .args(["-e", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "指定终端未找到",
                    ))
                }
            })
            .or_else(|_| Command::new("sh").args(["-lc", &command]).spawn())
            .map_err(|e| format!("执行 Gemini Cli 命令失败: {}", e))?;
        return Ok("已执行 Gemini Cli 命令".to_string());
    }

    #[allow(unreachable_code)]
    Err("不支持的操作系统".to_string())
}
