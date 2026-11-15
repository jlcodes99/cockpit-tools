use std::path::{Path, PathBuf};
use std::process::Command;

use crate::modules::codebuddy_cli::{self, AccountRef, Instance, InstanceDraft, LaunchTarget};

#[tauri::command]
pub(crate) async fn codebuddy_cli_available() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| codebuddy_cli::find_cli().is_some())
        .await
        .map_err(|_| "cliCheckFailed".into())
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_working_dir(
    account: AccountRef,
) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || codebuddy_cli::working_dir(&account))
        .await
        .map_err(|_| "storageError")?
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_list_instances() -> Result<Vec<Instance>, String> {
    tauri::async_runtime::spawn_blocking(codebuddy_cli::list_instances)
        .await
        .map_err(|_| "storageError")?
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_save_instance(
    draft: InstanceDraft,
) -> Result<Vec<Instance>, String> {
    tauri::async_runtime::spawn_blocking(move || codebuddy_cli::save_instance(draft))
        .await
        .map_err(|_| "storageError")?
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_remove_instance(id: uuid::Uuid) -> Result<Vec<Instance>, String> {
    tauri::async_runtime::spawn_blocking(move || codebuddy_cli::remove_instance(id))
        .await
        .map_err(|_| "storageError")?
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_launch(
    target: LaunchTarget,
    terminal: Option<String>,
    custom_command: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let remember_working_dir = target.remembers_working_dir();
        codebuddy_cli::find_cli().ok_or("cliMissing")?;
        let request = codebuddy_cli::prepare_launch(target, custom_command)?;
        let command = request_terminal_command(&request)?;
        if remember_working_dir {
            request.remember_working_dir()?;
        }
        open_terminal(&command, terminal)
    })
    .await
    .map_err(|_| "launchFailed")?
}

#[tauri::command]
pub(crate) async fn codebuddy_cli_launch_command(
    target: LaunchTarget,
    custom_command: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codebuddy_cli::find_cli().ok_or("cliMissing")?;
        let remember_working_dir = target.remembers_working_dir();
        let request = codebuddy_cli::prepare_launch(target, custom_command)?;
        let command = request_terminal_command(&request)?;
        if remember_working_dir {
            request.remember_working_dir()?;
        }
        Ok(command)
    })
    .await
    .map_err(|_| "launchFailed")?
}

fn request_terminal_command(request: &codebuddy_cli::LaunchRequest) -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|_| "launchFailed")?;
    Ok(terminal_command(&executable, &request.encode()?))
}

fn terminal_command(executable: &Path, encoded: &str) -> String {
    let run_arg = codebuddy_cli::RUN_ARG;
    let executable = executable.to_string_lossy();
    #[cfg(target_os = "windows")]
    {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let script = format!(
            "Start-Process -FilePath {} -ArgumentList '{} {}' -NoNewWindow -Wait",
            super::claude::powershell_quote(&executable),
            run_arg,
            encoded
        );
        let script = STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        format!("powershell -NoProfile -EncodedCommand {script}")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let quoted = format!("'{}'", executable.replace('\'', "'\"'\"'"));
        format!("{quoted} {run_arg} {encoded}")
    }
}

fn open_terminal(command: &str, terminal: Option<String>) -> Result<(), String> {
    use super::codex_instance::terminal::build_codex_terminal_launch_plan;
    let terminal =
        terminal.unwrap_or_else(|| crate::modules::config::get_user_config().default_terminal);
    let terminal = terminal.trim();
    let plan =
        build_codex_terminal_launch_plan(command, terminal).map_err(|_| "terminalUnsupported")?;
    let mut process = Command::new(&plan.program);
    process.args(&plan.args);
    #[cfg(target_os = "macos")]
    {
        use std::time::Duration;
        let output = crate::modules::process_timeout::output_with_timeout(
            &mut process,
            Duration::from_secs(10),
        )
        .map_err(|_| "terminalFailed")?;
        if !output.status.success() {
            crate::modules::logger::log_warn(&format!(
                "[CodeBuddy CLI] Terminal dispatch failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            return Err("terminalFailed".into());
        }
    }
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        fn spawn(process: &mut Command) -> std::io::Result<std::process::Child> {
            let program = process.get_program().to_string_lossy().into_owned();
            let _guard = crate::modules::app_lifecycle::acquire_process_spawn_guard(&program)?;
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                match program.as_str() {
                    // The selected PowerShell is the interactive terminal itself.
                    "powershell" | "pwsh" => {
                        process.creation_flags(0x00000010);
                    }
                    _ => {
                        process.creation_flags(0x08000000);
                    }
                }
            }
            process.spawn()
        }

        let result = spawn(&mut process);
        #[cfg(target_os = "linux")]
        let result = result.or_else(|err| {
            if !terminal.is_empty() && terminal != "system" {
                return Err(err);
            }
            let mut plan = build_codex_terminal_launch_plan(command, "gnome-terminal")
                .map_err(std::io::Error::other)?;
            if let Ok(child) = spawn(Command::new(&plan.program).args(&plan.args)) {
                return Ok(child);
            }
            plan = build_codex_terminal_launch_plan(command, "konsole")
                .map_err(std::io::Error::other)?;
            spawn(Command::new(&plan.program).args(&plan.args))
        });
        let child = result.map_err(|_| "terminalFailed")?;
        #[cfg(target_os = "linux")]
        std::thread::spawn(move || {
            let _ = { child }.wait();
        });
        #[cfg(target_os = "windows")]
        drop(child);
    }
    Ok(())
}
