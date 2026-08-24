#[cfg(target_os = "windows")]
use serde::Serialize;

#[cfg(target_os = "windows")]
pub const WINDOWS_OPERATION_ERROR_PREFIX: &str = "WINDOWS_OPERATION_ERROR:";

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WindowsOperationErrorPayload<'a> {
    code: &'a str,
    operation: &'a str,
    summary: &'a str,
    original_reason: &'a str,
    target: Option<&'a str>,
    pids: &'a [u32],
    retryable: bool,
    can_elevate: bool,
    manual_action_available: bool,
    attempted_recoveries: &'a [&'a str],
}

#[cfg(any(target_os = "windows", test))]
pub fn is_access_denied_message(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    normalized.contains("os error 5")
        || normalized.contains("permissiondenied")
        || normalized.contains("permission denied")
        || normalized.contains("access is denied")
        || normalized.contains("access denied")
        || message.contains("拒绝访问")
}

pub fn format_error(
    operation: &str,
    summary: &str,
    original_reason: &str,
    target: Option<&str>,
    pids: &[u32],
    retryable: bool,
    can_elevate: bool,
    manual_action_available: bool,
) -> String {
    #[cfg(target_os = "windows")]
    {
        let code = if is_access_denied_message(original_reason) {
            "access_denied"
        } else {
            "operation_failed"
        };
        let payload = WindowsOperationErrorPayload {
            code,
            operation,
            summary,
            original_reason,
            target,
            pids,
            retryable,
            can_elevate,
            manual_action_available,
            attempted_recoveries: match operation {
                "stop_process" => &["graceful_stop", "taskkill"],
                "write_file" | "replace_file" => &["atomic_write"],
                _ => &[],
            },
        };
        return serde_json::to_string(&payload)
            .map(|json| format!("{}{}", WINDOWS_OPERATION_ERROR_PREFIX, json))
            .unwrap_or_else(|_| format!("{}: {}", summary, original_reason));
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (
            operation,
            target,
            pids,
            retryable,
            can_elevate,
            manual_action_available,
        );
        format!("{}: {}", summary, original_reason)
    }
}

pub fn format_permission_io_error(
    operation: &str,
    summary: &str,
    target: &str,
    error: &std::io::Error,
) -> Option<String> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (operation, summary, target, error);
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        if error.kind() != std::io::ErrorKind::PermissionDenied && error.raw_os_error() != Some(5) {
            return None;
        }
        Some(format_error(
            operation,
            summary,
            &error.to_string(),
            Some(target),
            &[],
            true,
            false,
            false,
        ))
    }
}

#[cfg(target_os = "windows")]
fn supported_elevation_process_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "antigravity ide.exe"
            | "antigravity.exe"
            | "chatgpt.exe"
            | "codex.exe"
            | "claude.exe"
            | "cursor.exe"
            | "windsurf.exe"
            | "devin.exe"
            | "kiro.exe"
            | "zed.exe"
            | "zcode.exe"
            | "codebuddy.exe"
            | "codebuddy cn.exe"
            | "qoder.exe"
            | "trae.exe"
            | "trae cn.exe"
            | "trae solo.exe"
            | "trae solo cn.exe"
            | "workbuddy.exe"
            | "opencode.exe"
            | "code.exe"
            | "code - insiders.exe"
    )
}

#[cfg(target_os = "windows")]
fn validate_elevation_targets(pids: &[u32]) -> Result<Vec<u32>, String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut targets = pids
        .iter()
        .copied()
        .filter(|pid| *pid != 0 && *pid != std::process::id())
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    if targets.is_empty() || targets.len() > 32 {
        return Err("WINDOWS_ELEVATION_TARGET_INVALID".to_string());
    }

    let refresh_pids = targets
        .iter()
        .map(|pid| Pid::from_u32(*pid))
        .collect::<Vec<_>>();
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&refresh_pids),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );

    let mut running = Vec::new();
    for pid in targets {
        let Some(process) = system.process(Pid::from_u32(pid)) else {
            continue;
        };
        let name = process.name().to_string_lossy();
        if !supported_elevation_process_name(&name) {
            return Err(format!(
                "WINDOWS_ELEVATION_TARGET_NOT_ALLOWED: pid={}, process={}",
                pid, name
            ));
        }
        running.push(pid);
    }
    Ok(running)
}

#[cfg(target_os = "windows")]
fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn run_elevated_taskkill(pids: &[u32]) -> Result<(), String> {
    use std::mem::size_of;
    use std::path::PathBuf;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or_else(|| "WINDOWS_SYSTEM_ROOT_NOT_FOUND".to_string())?;
    let taskkill = system_root.join("System32").join("taskkill.exe");
    if !taskkill.is_file() {
        return Err(format!(
            "WINDOWS_TASKKILL_NOT_FOUND: {}",
            taskkill.display()
        ));
    }

    let mut args = String::new();
    for pid in pids {
        args.push_str(&format!(" /PID {}", pid));
    }
    args.push_str(" /T /F");

    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(taskkill.as_os_str());
    let parameters = wide(std::ffi::OsStr::new(args.trim()));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: 0,
        ..Default::default()
    };

    unsafe {
        ShellExecuteExW(&mut info).map_err(|error| {
            if error.code() == windows::core::HRESULT::from_win32(1223) {
                "WINDOWS_ELEVATION_CANCELLED".to_string()
            } else {
                format!("WINDOWS_ELEVATION_START_FAILED: {}", error)
            }
        })?;
        if info.hProcess.is_invalid() {
            return Err("WINDOWS_ELEVATION_PROCESS_HANDLE_MISSING".to_string());
        }

        let wait_result = WaitForSingleObject(info.hProcess, 120_000);
        if wait_result == WAIT_TIMEOUT {
            let _ = CloseHandle(info.hProcess);
            return Err("WINDOWS_ELEVATION_TIMEOUT".to_string());
        }
        if wait_result != WAIT_OBJECT_0 {
            let _ = CloseHandle(info.hProcess);
            return Err(format!("WINDOWS_ELEVATION_WAIT_FAILED: {}", wait_result.0));
        }

        let mut exit_code = 0u32;
        let exit_result = GetExitCodeProcess(info.hProcess, &mut exit_code);
        let _ = CloseHandle(info.hProcess);
        exit_result.map_err(|error| format!("WINDOWS_ELEVATION_EXIT_READ_FAILED: {}", error))?;
        if exit_code != 0
            && pids
                .iter()
                .any(|pid| crate::modules::process::is_pid_running(*pid))
        {
            return Err(format!(
                "WINDOWS_ELEVATION_TASKKILL_FAILED: exit_code={}",
                exit_code
            ));
        }
    }
    Ok(())
}

pub fn elevated_close_supported_processes(pids: &[u32]) -> Result<u32, String> {
    #[cfg(target_os = "windows")]
    {
        let targets = validate_elevation_targets(pids)?;
        if targets.is_empty() {
            return Ok(0);
        }
        run_elevated_taskkill(&targets)?;
        let remaining = targets
            .iter()
            .filter(|pid| crate::modules::process::is_pid_running(**pid))
            .copied()
            .collect::<Vec<_>>();
        if !remaining.is_empty() {
            return Err(format!(
                "WINDOWS_ELEVATION_TARGET_STILL_RUNNING: pids={}",
                remaining
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        return Ok(targets.len() as u32);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = pids;
        Err("WINDOWS_ELEVATION_UNSUPPORTED".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_access_denied_variants() {
        assert!(is_access_denied_message("拒绝访问。 (os error 5)"));
        assert!(is_access_denied_message("PermissionDenied"));
        assert!(is_access_denied_message("Access is denied"));
        assert!(!is_access_denied_message("file not found"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn elevation_process_allowlist_rejects_generic_electron() {
        assert!(supported_elevation_process_name("Codex.exe"));
        assert!(supported_elevation_process_name("Code - Insiders.exe"));
        assert!(!supported_elevation_process_name("Electron.exe"));
        assert!(!supported_elevation_process_name("explorer.exe"));
    }
}
