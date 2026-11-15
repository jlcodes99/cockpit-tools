use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tauri::AppHandle;

use super::super::codex_instance::terminal::{
    build_codex_terminal_launch_plan, CodexTerminalLaunchPlan,
};
use crate::{models::Account, modules};

#[tauri::command]
pub(crate) async fn get_antigravity_client_launch_path(
    client: String,
) -> Result<modules::process::AntigravityLaunchPath, String> {
    let task = tokio::task::spawn_blocking(move || match client.as_str() {
        "antigravity" | "antigravity_ide" => Ok(
            modules::process::inspect_antigravity_desktop_launch_path(client == "antigravity"),
        ),
        "cli" => {
            let configured = modules::config::get_user_config().antigravity_cli_path;
            let resolved_path = find_cli();
            Ok(modules::process::AntigravityLaunchPath {
                uses_configured_path: !configured.is_empty() && resolved_path.is_some(),
                configured_path: (!configured.is_empty()).then_some(configured),
                resolved_path,
            })
        },
        _ => Err("invalid_antigravity_client".to_string()),
    });
    let mut status = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .map_err(|_| "antigravity_launch_path_timeout".to_string())?
        .map_err(|error| error.to_string())??;
    status.resolved_path = status.resolved_path.map(|path| {
        PathBuf::from(modules::process::normalize_windows_user_facing_path(&path.to_string_lossy()))
    });
    Ok(status)
}

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
    let terminal_plan = if launch {
        Some(prepare_terminal(working_dir, terminal, custom_command).await?)
    } else {
        None
    };
    let account = tokio::time::timeout(
        Duration::from_secs(60),
        modules::account::prepare_account_for_injection(&account_id),
    )
    .await
    .map_err(|_| "ANTIGRAVITY_SWITCH_PREPARATION_TIMEOUT".to_string())??;

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
    let (terminal, plan) = prepare_terminal(working_dir, terminal, custom_command).await?;
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
) -> Result<(String, CodexTerminalLaunchPlan), String> {
    tokio::task::spawn_blocking(move || {
        let command = resolve_command(working_dir, custom_command)?;
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
    tokio::task::spawn_blocking(move || resolve_command(working_dir, custom_command))
        .await
        .map_err(|error| error.to_string())?
}

fn resolve_command(
    working_dir: Option<PathBuf>,
    custom_command: Option<String>,
) -> Result<String, String> {
    let dir = working_dir
        .or_else(dirs::home_dir)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .ok_or("ANTIGRAVITY_CLI_INVALID_DIRECTORY")?;
    let binary = find_cli().ok_or("ANTIGRAVITY_CLI_NOT_FOUND")?;
    Ok(build_command(&binary, &dir, custom_command.as_deref()))
}

fn find_cli() -> Option<PathBuf> {
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
fn build_command(binary: &Path, dir: &Path, custom_command: Option<&str>) -> String {
    fn quote(path: &Path) -> String {
        format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
    }
    let launch = match custom_command.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!(
            "PATH={}:\"$PATH\" /bin/sh -c {}",
            quote(binary.parent().unwrap_or_else(|| Path::new("/"))),
            quote(Path::new(value))
        ),
        None => quote(binary),
    };
    format!("cd -- {} && {}", quote(dir), launch)
}

#[cfg(target_os = "windows")]
fn build_command(binary: &Path, dir: &Path, custom_command: Option<&str>) -> String {
    use super::super::claude::powershell_quote;
    use base64::{engine::general_purpose::STANDARD, Engine};

    let launch = match custom_command.filter(|value| !value.trim().is_empty()) {
        Some(value) => format!(
            "$env:Path = {} + ';' + $env:Path\n{}",
            powershell_quote(&binary.parent().unwrap_or(binary).to_string_lossy()),
            value
        ),
        None => format!("& {}", powershell_quote(&binary.to_string_lossy())),
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

fn open_terminal(plan: CodexTerminalLaunchPlan, terminal: &str) -> Result<(), String> {
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
                let flags = if matches!(program.as_str(), "powershell" | "pwsh") {
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
}
