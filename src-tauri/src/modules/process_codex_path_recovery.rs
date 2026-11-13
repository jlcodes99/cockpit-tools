/// Validate an explicitly selected desktop entry before retrying a launch.
/// Settings-only saves do not use this helper, so empty paths can still be cleared.
pub fn validate_codex_app_path(path: &str) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("CODEX_APP_PATH_INVALID".into());
    }
    let selected = Path::new(path);
    #[cfg(target_os = "windows")]
    let valid = is_usable_codex_windows_gui_path(selected);
    #[cfg(target_os = "macos")]
    let valid = is_codex_macos_gui_candidate(selected)
        && resolve_codex_macos_exec_path(path).is_some_and(|executable| {
            use std::os::unix::fs::PermissionsExt;
            executable.metadata().is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        });
    #[cfg(target_os = "linux")]
    let valid = {
        use std::os::unix::fs::PermissionsExt;
        selected
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let valid = false;
    if valid {
        Ok(())
    } else {
        Err("CODEX_APP_PATH_INVALID".into())
    }
}

#[cfg(any(test, target_os = "macos"))]
fn is_codex_macos_gui_candidate(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let macos = path.parent();
    let contents = macos.and_then(Path::parent);
    let bundle = contents.and_then(Path::parent);
    (name.eq_ignore_ascii_case("ChatGPT") || name.eq_ignore_ascii_case("Codex"))
        && macos
            .and_then(Path::file_name)
            .is_some_and(|name| name == "MacOS")
        && contents
            .and_then(Path::file_name)
            .is_some_and(|name| name == "Contents")
        && bundle
            .and_then(Path::extension)
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
}

#[cfg(any(test, target_os = "windows"))]
fn codex_close_path_or_confirmed_idle(
    path: Result<PathBuf, String>,
    confirm_idle: impl FnOnce() -> bool,
) -> Result<Option<PathBuf>, String> {
    match path {
        Ok(path) => Ok(Some(path)),
        Err(error) if error == "APP_PATH_NOT_FOUND:codex" && confirm_idle() => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(any(test, target_os = "windows"))]
fn codex_pathless_probe_confirms_idle(success: bool, stdout: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct RuntimeProcess {
        name: String,
        executable_path: Option<String>,
    }

    #[derive(serde::Deserialize)]
    struct Probe {
        completed: bool,
        processes: Vec<RuntimeProcess>,
    }
    let stdout = stdout.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(stdout);
    success
        && serde_json::from_slice::<Probe>(stdout).is_ok_and(|probe| {
            probe.completed
                && probe.processes.iter().all(|process| {
                    process.name.eq_ignore_ascii_case("codex.exe")
                        && process
                            .executable_path
                            .as_deref()
                            .is_some_and(is_codex_vscode_extension_executable)
                })
        })
}

#[cfg(any(test, target_os = "windows"))]
fn is_codex_vscode_extension_executable(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let mut parts = path.rsplit(['\\', '/']);
    let (Some("codex.exe"), Some(platform), Some("bin"), Some(extension), Some(_)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    platform.starts_with("windows-")
        && (extension == "openai.chatgpt"
            || extension
                .strip_prefix("openai.chatgpt-")
                .and_then(|version| version.as_bytes().first())
                .is_some_and(u8::is_ascii_digit))
}

#[cfg(target_os = "windows")]
fn confirm_no_codex_windows_runtime() -> bool {
    // Distinguish the editor extension by its binary location. The desktop
    // backend has the same Codex.exe name and can outlive its GUI.
    let script = r#"$ErrorActionPreference='Stop';
$items=@(Get-CimInstance Win32_Process -Filter "Name='ChatGPT.exe' OR Name='Codex.exe'" |
  Select-Object Name, ExecutablePath);
ConvertTo-Json -InputObject @{completed=$true;processes=$items} -Depth 4 -Compress"#;
    match powershell_output_with_timeout(
        &["-NoProfile", "-Command", script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    ) {
        Ok(output) => codex_pathless_probe_confirms_idle(output.status.success(), &output.stdout),
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] pathless runtime probe failed; keeping credentials unchanged: {error}"
            ));
            false
        }
    }
}

/// Managed-instance PID resolution also needs the desktop path on Windows.
/// Do not treat a missing path as a successful stop before credential writes.
pub fn ensure_codex_runtime_path_or_idle() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        codex_close_path_or_confirmed_idle(
            resolve_codex_launch_path(),
            confirm_no_codex_windows_runtime,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "process_codex_path_recovery_tests.rs"]
mod codex_path_recovery_tests;
