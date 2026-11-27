use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tauri::AppHandle;

pub(crate) use super::super::codex_instance::terminal::build_codex_terminal_launch_plan;
#[cfg(windows)]
use super::super::codex_instance::terminal::is_powershell_program;
use super::super::codex_instance::terminal::CodexTerminalLaunchPlan;
use crate::{models::Account, modules};

#[tauri::command]
pub(crate) async fn switch_antigravity_cli(
    app: AppHandle,
    account_id: String,
    working_dir: Option<PathBuf>,
    launch: bool,
    terminal: Option<String>,
    custom_command: Option<String>,
) -> Result<Account, String> {
    let permit =
        modules::account::try_begin_antigravity_switch(&modules::account::ANTIGRAVITY_SWITCH_GATE)?;
    let account = tokio::time::timeout(
        Duration::from_secs(60),
        modules::account::prepare_account_for_injection(&account_id),
    )
    .await
    .map_err(|_| "ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT".to_string())??;
    let egress_proxy_url = account.egress_proxy_url.clone();
    let terminal_plan = if launch {
        Some(
            prepare_terminal(
                working_dir,
                terminal,
                custom_command,
                egress_proxy_url.clone(),
            )
            .await?,
        )
    } else {
        None
    };

    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let (account, _) = modules::antigravity_shared_switch::switch(
            &account,
            modules::antigravity_shared_switch::SwitchTarget::Cli,
            false,
            super::super::system::antigravity_ide_uses_shared_credentials(),
        )?;
        modules::websocket::broadcast_account_switched(&account.id, &account.email);
        modules::websocket::broadcast_data_changed("switch_antigravity_cli");
        let _ = modules::tray::update_tray_menu(&app);
        if let Some((terminal, plan)) = terminal_plan {
            open_terminal(plan, &terminal)
                .map_err(|error| format!("ANTIGRAVITY_CLI_SWITCHED_TERMINAL_FAILED: {error}"))?;
        }
        Ok(account)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn launch_antigravity_cli(
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
    custom_command: Option<String>,
) -> Result<(), String> {
    let egress_proxy_url = modules::account::get_current_account()
        .ok()
        .flatten()
        .and_then(|account| account.egress_proxy_url);
    let (terminal, plan) =
        prepare_terminal(working_dir, terminal, custom_command, egress_proxy_url).await?;
    tokio::task::spawn_blocking(move || {
        open_terminal(plan, &terminal)
            .map_err(|error| format!("ANTIGRAVITY_CLI_TERMINAL_FAILED: {error}"))
    })
    .await
    .map_err(|error| error.to_string())?
}

async fn prepare_terminal(
    working_dir: Option<PathBuf>,
    terminal: Option<String>,
    custom_command: Option<String>,
    egress_proxy_url: Option<String>,
) -> Result<(String, CodexTerminalLaunchPlan), String> {
    tokio::task::spawn_blocking(move || {
        let command = resolve_command(working_dir, custom_command, egress_proxy_url.as_deref())?;
        let terminal =
            terminal.unwrap_or_else(|| modules::config::get_user_config().default_terminal);
        let plan = build_codex_terminal_launch_plan(&command, &terminal)
            .map_err(|error| format!("ANTIGRAVITY_CLI_TERMINAL_FAILED: {error}"))?;
        Ok((terminal, plan))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn antigravity_cli_launch_command(
    working_dir: Option<PathBuf>,
    custom_command: Option<String>,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || resolve_command(working_dir, custom_command, None))
        .await
        .map_err(|error| error.to_string())?
}

fn resolve_command(
    working_dir: Option<PathBuf>,
    custom_command: Option<String>,
    egress_proxy_url: Option<&str>,
) -> Result<String, String> {
    let dir = working_dir
        .or_else(dirs::home_dir)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .ok_or("ANTIGRAVITY_CLI_INVALID_DIRECTORY")?;
    let binary = find_cli().ok_or("ANTIGRAVITY_CLI_NOT_FOUND")?;
    Ok(build_command_with_proxy(
        &binary,
        &dir,
        custom_command.as_deref(),
        egress_proxy_url,
    ))
}

/// CLI 启动代理：账号自定义出口优先，未配置时回落全局代理（修复 CLI 不吃全局代理的盲区）。
fn antigravity_cli_proxy_pairs(egress_proxy_url: Option<&str>) -> Option<Vec<(String, String)>> {
    let url = match egress_proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        Some(url) => url.to_string(),
        None => {
            let config = modules::config::get_user_config();
            if config.global_proxy_enabled && !config.global_proxy_url.trim().is_empty() {
                config.global_proxy_url.trim().to_string()
            } else {
                return None;
            }
        }
    };
    let no_proxy = crate::modules::codex_protocol::merge_local_no_proxy("");
    let mut pairs = vec![
        ("https_proxy".to_string(), url.clone()),
        ("HTTPS_PROXY".to_string(), url.clone()),
        ("http_proxy".to_string(), url.clone()),
        ("HTTP_PROXY".to_string(), url),
    ];
    if !no_proxy.is_empty() {
        pairs.push(("no_proxy".to_string(), no_proxy.clone()));
        pairs.push(("NO_PROXY".to_string(), no_proxy));
    }
    Some(pairs)
}

pub(crate) fn find_cli() -> Option<PathBuf> {
    let configured = modules::config::get_user_config().antigravity_cli_path;
    if configured.is_empty() {
        return detect_cli();
    }
    let path = PathBuf::from(configured);
    crate::commands::cli_launch_path::executable_file(&path).then_some(path)
}

pub(crate) fn detect_cli() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let installed =
        std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("agy/bin/agy.exe"));
    #[cfg(not(target_os = "windows"))]
    let installed = dirs::home_dir().map(|dir| dir.join(".local/bin/agy"));
    let name = if cfg!(target_os = "windows") {
        "agy.exe"
    } else {
        "agy"
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .chain(installed)
        .find(|candidate| {
            candidate.metadata().is_ok_and(|metadata| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
                }
                #[cfg(not(unix))]
                {
                    metadata.is_file()
                }
            })
        })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn build_command(binary: &Path, dir: &Path, custom_command: Option<&str>) -> String {
    build_command_with_env(binary, dir, custom_command, None)
}

#[cfg(target_os = "windows")]
pub(crate) fn build_command(binary: &Path, dir: &Path, custom_command: Option<&str>) -> String {
    build_command_with_env(binary, dir, custom_command, None)
}

fn build_command_with_proxy(
    binary: &Path,
    dir: &Path,
    custom_command: Option<&str>,
    egress_proxy_url: Option<&str>,
) -> String {
    build_command_with_env(
        binary,
        dir,
        custom_command,
        antigravity_cli_proxy_pairs(egress_proxy_url),
    )
}

#[cfg(not(target_os = "windows"))]
fn build_command_with_env(
    binary: &Path,
    dir: &Path,
    custom_command: Option<&str>,
    proxy_pairs: Option<Vec<(String, String)>>,
) -> String {
    fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
    }
    fn quote_value(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
    let env_prefix = match proxy_pairs {
        Some(pairs) if !pairs.is_empty() => {
            pairs
                .iter()
                .map(|(key, value)| format!("{key}={}", quote_value(value)))
                .collect::<Vec<_>>()
                .join(" ")
                + " "
        }
        _ => String::new(),
    };
    let launch = match custom_command.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!(
            "{}PATH={}:\"$PATH\" /bin/sh -c {}",
            env_prefix,
            quote(binary.parent().unwrap_or_else(|| Path::new("/"))),
            quote(Path::new(value))
        ),
        None => format!("{}{}", env_prefix, quote(binary)),
    };
    format!("cd -- {} && {}", quote(dir), launch)
}

#[cfg(target_os = "windows")]
fn build_command_with_env(
    binary: &Path,
    dir: &Path,
    custom_command: Option<&str>,
    proxy_pairs: Option<Vec<(String, String)>>,
) -> String {
    use super::super::claude::powershell_quote;
    use base64::{engine::general_purpose::STANDARD, Engine};

    let env_lines = match proxy_pairs {
        Some(pairs) if !pairs.is_empty() => pairs
            .iter()
            .map(|(key, value)| format!("$env:{} = {}\n", key, powershell_quote(value)))
            .collect::<String>(),
        _ => String::new(),
    };
    let launch = match custom_command.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!(
            "{}$env:Path = {} + ';' + $env:Path\n{}",
            env_lines,
            powershell_quote(&binary.parent().unwrap_or(binary).to_string_lossy()),
            value
        ),
        None => format!("{}& {}", env_lines, powershell_quote(&binary.to_string_lossy())),
    };
    let script = format!(
        "Set-Location -LiteralPath {} -ErrorAction Stop\n{}",
        powershell_quote(&dir.to_string_lossy()),
        launch,
    );
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    format!("powershell -NoProfile -EncodedCommand {encoded}")
}

pub(crate) fn open_terminal(plan: CodexTerminalLaunchPlan, terminal: &str) -> Result<(), String> {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args);

    #[cfg(target_os = "macos")]
    {
        let _ = terminal;
        let output =
            modules::process_timeout::output_with_timeout(&mut command, Duration::from_secs(10))
                .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        fn spawn(command: &mut Command) -> std::io::Result<std::process::Child> {
            let program = command.get_program().to_string_lossy().into_owned();
            let _guard = modules::app_lifecycle::acquire_process_spawn_guard(&program)?;
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                let flags = if is_powershell_program(&program) {
                    0x00000010 // The selected PowerShell is the interactive terminal.
                } else {
                    0x08000000
                };
                command.creation_flags(flags);
            }
            command.spawn()
        }
        let result = spawn(&mut command);
        #[cfg(target_os = "linux")]
        let result = result.or_else(|error| {
            if !terminal.is_empty() && !terminal.eq_ignore_ascii_case("system") {
                return Err(error);
            }
            let shell_command = plan.args.last().ok_or(error)?;
            spawn(Command::new("gnome-terminal").args(["--", "bash", "-lc", shell_command]))
                .or_else(|_| {
                    spawn(Command::new("konsole").args(["-e", "bash", "-lc", shell_command]))
                })
        });
        #[cfg(target_os = "linux")]
        {
            let mut child = result.map_err(|error| error.to_string())?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        #[cfg(target_os = "windows")]
        {
            let _ = terminal;
            drop(result.map_err(|error| error.to_string())?);
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn antigravity_cli_launch_uses_the_selected_working_directory_and_literal_paths() {
        let sandbox = tempfile::tempdir().unwrap();
        let dir = sandbox.path().join("working dir ' [*]");
        std::fs::create_dir(&dir).unwrap();
        let command = build_command(Path::new("/bin/pwd"), &dir, None);
        let output = Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success());
        let actual = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            Path::new(actual.trim_end()).canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn antigravity_cli_custom_command_runs_in_the_selected_directory() {
        let sandbox = tempfile::tempdir().unwrap();
        let dir = sandbox.path().join("project ' with spaces");
        std::fs::create_dir(&dir).unwrap();
        let script = "printf '%s\\n' 'quoted argument'\npwd";
        let command = build_command(Path::new("/bin/false"), &dir, Some(script));
        let output = Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let (message, path) = stdout.split_once('\n').unwrap();
        assert_eq!(message, "quoted argument");
        assert_eq!(
            Path::new(path.trim_end()).canonicalize().unwrap(),
            dir.canonicalize().unwrap()
        );
    }

    #[test]
    fn antigravity_cli_command_carries_proxy_env_overrides() {
        let sandbox = tempfile::tempdir().unwrap();
        let command = build_command_with_proxy(
            Path::new("/bin/sh"),
            sandbox.path(),
            Some("printf '%s\\n' \"$HTTPS_PROXY|$NO_PROXY\""),
            Some("socks5://127.0.0.1:19009"),
        );
        let output = Command::new("/bin/sh")
            .args(["-c", &command])
            .env_clear()
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let (proxy, no_proxy) = stdout.trim_end().split_once('|').unwrap();
        assert_eq!(proxy, "socks5://127.0.0.1:19009");
        assert!(no_proxy.contains("127.0.0.1"));
        assert!(no_proxy.contains("localhost"));
    }
}
