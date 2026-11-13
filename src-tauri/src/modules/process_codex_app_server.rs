// Bundled app-server ownership and command parsing, shared by shutdown and diagnostics.
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexProcessTreeEntry {
    pid: u32,
    parent_pid: u32,
    command_line: String,
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
fn is_codex_direct_app_server_command_line(
    command_line: &str,
    expected_resource_executable: &str,
) -> bool {
    let command_line = command_line.trim();
    let expected = expected_resource_executable.trim();
    if command_line.is_empty() || expected.is_empty() {
        return false;
    }
    // The current ChatGPT bundle nests its CLI in a signed helper app. Keep
    // matching the exact installation, including legacy Codex bundles; never
    // accept an arbitrary executable merely because it is named `codex`.
    let current = expected
        .strip_suffix("/Contents/Resources/codex")
        .map(|root| {
            format!("{root}/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex")
        });
    let args = std::iter::once(expected)
        .chain(current.as_deref())
        .find_map(|executable| {
            let quoted = format!("\"{executable}\"");
            let remainder = command_line
                .strip_prefix(executable)
                .or_else(|| command_line.strip_prefix(&quoted))?;
            // Do not accept a path suffix (codex-other, codex-cli/...) as arguments.
            remainder
                .starts_with(char::is_whitespace)
                .then(|| remainder.trim_start())
        });
    let Some(args) = args else {
        return false;
    };
    let args = split_command_tokens(args);
    has_direct_codex_app_server_subcommand(&args)
}

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
fn has_direct_codex_app_server_subcommand(args: &[String]) -> bool {
    let mut index = 0;
    while let Some(arg) = args.get(index).map(String::as_str) {
        match arg {
            "app-server" => {
                return !args[index + 1..].iter().any(|value| value == "daemon");
            }
            "-c" | "--config" | "--enable" | "--disable" => {
                if args
                    .get(index + 1)
                    .is_none_or(|value| value.is_empty() || value.starts_with('-'))
                {
                    return false;
                }
                index += 2;
            }
            _ if arg
                .strip_prefix("--config=")
                .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("--enable=")
                    .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("--disable=")
                    .is_some_and(|value| !value.is_empty())
                || arg
                    .strip_prefix("-c")
                    .is_some_and(|value| !value.is_empty()) =>
            {
                index += 1;
            }
            // In particular, `exec ... app-server` is not a server process.
            _ => return false,
        }
    }
    false
}

#[cfg(any(test, target_os = "macos", target_os = "linux", target_os = "windows"))]
fn is_codex_process_descendant(
    mut parent: u32,
    roots: &HashSet<u32>,
    parents: &HashMap<u32, u32>,
) -> bool {
    let mut visited = HashSet::new();
    while parent != 0 && visited.insert(parent) {
        if roots.contains(&parent) {
            return true;
        }
        let Some(next) = parents.get(&parent) else {
            break;
        };
        parent = *next;
    }
    false
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
fn select_codex_direct_app_server_descendants(
    entries: &[CodexProcessTreeEntry],
    root_pids: &[u32],
    expected_resource_executable: &str,
) -> Vec<u32> {
    let roots: HashSet<u32> = root_pids.iter().copied().filter(|pid| *pid != 0).collect();
    if roots.is_empty() {
        return Vec::new();
    }
    let parents: HashMap<u32, u32> = entries
        .iter()
        .map(|entry| (entry.pid, entry.parent_pid))
        .collect();
    let mut selected = Vec::new();

    for entry in entries {
        if !is_codex_direct_app_server_command_line(
            &entry.command_line,
            expected_resource_executable,
        ) {
            continue;
        }
        if is_codex_process_descendant(entry.parent_pid, &roots, &parents) {
            selected.push(entry.pid);
        }
    }

    selected.sort();
    selected.dedup();
    selected
}

#[cfg(any(test, target_os = "windows"))]
fn is_windows_bundled_codex_app_server(
    exe: &str,
    app_dir: &str,
    copied_bin_dir: Option<&str>,
    args: &[String],
) -> bool {
    let exe = exe.replace('/', "\\");
    let app_dir = app_dir.replace('/', "\\");
    let Some((dir, name)) = exe.rsplit_once('\\') else {
        return false;
    };
    if !name.eq_ignore_ascii_case("codex.exe") || !has_direct_codex_app_server_subcommand(args) {
        return false;
    }
    let resources = format!("{app_dir}\\resources");
    if dir.eq_ignore_ascii_case(&resources)
        || dir.eq_ignore_ascii_case(&format!("{resources}\\app.asar.unpacked"))
    {
        return true;
    }
    // The official MSIX desktop copies its bundled CLI to
    // LOCALAPPDATA/OpenAI/Codex/bin/<16-hex digest>/codex.exe.
    let Some((parent, digest)) = dir.rsplit_once('\\') else {
        return false;
    };
    copied_bin_dir.is_some_and(|expected| parent.eq_ignore_ascii_case(expected))
        && digest.len() == 16
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(target_os = "windows")]
struct CapturedWindowsCodexAppServer {
    pid: u32,
    handle: std::os::windows::io::OwnedHandle,
}

#[cfg(target_os = "windows")]
fn capture_windows_codex_app_servers(
    root_pids: &[u32],
    launch_path: &Path,
) -> Result<Vec<CapturedWindowsCodexAppServer>, String> {
    use std::os::windows::io::FromRawHandle;
    use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, FILETIME};
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE,
    };

    let Some(app_dir) = launch_path.parent() else {
        return Err("CODEX_PROCESS_SCAN_FAILED".into());
    };
    let app_dir = normalize_path_for_compare(&app_dir.to_string_lossy());
    let copied_bin_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join("AppData").join("Local")))
        .map(|path| normalize_path_for_compare(&path.join(r"OpenAI\Codex\bin").to_string_lossy()));
    let roots: HashSet<_> = root_pids.iter().copied().collect();
    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    let parents: HashMap<_, _> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let parent = process.parent()?;
            let parent_process = system.process(parent)?;
            (parent_process.start_time() <= process.start_time())
                .then_some((pid.as_u32(), parent.as_u32()))
        })
        .collect();
    let candidates: Vec<_> = system
        .processes()
        .iter()
        .filter(|(pid, process)| {
            process
                .name()
                .to_string_lossy()
                .eq_ignore_ascii_case("codex.exe")
                && parents
                    .get(&pid.as_u32())
                    .is_some_and(|parent| is_codex_process_descendant(*parent, &roots, &parents))
        })
        .map(|(pid, process)| (*pid, process.start_time()))
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let pids: Vec<_> = candidates.iter().map(|(pid, _)| *pid).collect();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&pids),
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let mut captured = Vec::new();
    for (pid, start_time) in candidates {
        let Some(process) = system.process(pid) else {
            continue;
        };
        if process.start_time() != start_time
            || process.parent().map(Pid::as_u32) != parents.get(&pid.as_u32()).copied()
        {
            continue;
        }
        let (exe, _) = resolve_windows_process_exe_for_match(process);
        let Some(exe) = exe else { continue };
        let args: Vec<_> = process
            .cmd()
            .iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        if !is_windows_bundled_codex_app_server(&exe, &app_dir, copied_bin_dir.as_deref(), &args) {
            continue;
        }

        let pid = pid.as_u32();
        let handle = match unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                false,
                pid,
            )
        } {
            Ok(handle) => handle,
            Err(error) if error.code() == ERROR_INVALID_PARAMETER.to_hresult() => continue,
            Err(error) => return Err(format!("OpenProcess app-server pid={pid}: {error}")),
        };
        // Keep the process object open across Electron shutdown so termination
        // and waiting still refer to the captured process if the PID changes owner.
        let owned_handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(handle.0) };
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
            .map_err(|error| format!("GetProcessTimes app-server pid={pid}: {error}"))?;
        let ticks = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
        if (ticks / 10_000_000).checked_sub(11_644_473_600) == Some(start_time) {
            captured.push(CapturedWindowsCodexAppServer {
                pid,
                handle: owned_handle,
            });
        }
    }
    Ok(captured)
}

#[cfg(target_os = "windows")]
fn close_captured_windows_codex_app_servers(
    captured: &[CapturedWindowsCodexAppServer],
    timeout_secs: u64,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};

    let started = Instant::now();
    let deadline = started + Duration::from_secs(timeout_secs.clamp(1, 5));
    // The desktop has already requested a tree shutdown. Give its app-server a
    // brief chance to finish pending writes after the stdio connection closes.
    let graceful_deadline = started + Duration::from_millis(500);
    for process in captured {
        let handle = HANDLE(process.handle.as_raw_handle());
        let graceful_ms = graceful_deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        match unsafe { WaitForSingleObject(handle, graceful_ms) } {
            WAIT_OBJECT_0 => continue,
            WAIT_TIMEOUT => {}
            _ => {
                return Err(format!(
                    "WaitForSingleObject app-server pid={}: {}",
                    process.pid,
                    windows::core::Error::from_win32()
                ))
            }
        }
        crate::modules::logger::log_info(&format!(
            "[Codex Close] closing captured Windows app-server pid={}",
            process.pid
        ));
        if let Err(error) = unsafe { TerminateProcess(handle, 1) } {
            // It can exit after the nonblocking check and before termination.
            if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
                return Err(format!(
                    "TerminateProcess app-server pid={}: {error}",
                    process.pid
                ));
            }
        }
        let remaining_ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u32;
        if unsafe { WaitForSingleObject(handle, remaining_ms) } != WAIT_OBJECT_0 {
            return Err(format!(
                "failed to close Codex app-server pid={}",
                process.pid
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod codex_app_server_ownership_tests {
    use super::*;

    const LEGACY: &str = "/Applications/ChatGPT.app/Contents/Resources/codex";
    const CURRENT: &str =
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex";

    #[test]
    fn recognizes_current_bundle_with_global_cli_options() {
        for options in [
            "-c features.code_mode_host=true",
            "--config features.code_mode_host=true --enable feature --disable other",
            "--config=features.code_mode_host=true --enable=feature",
            "-cfeatures.code_mode_host=true",
            "-c 'instructions=keep app-server as text'",
            "",
        ] {
            assert!(
                is_codex_direct_app_server_command_line(
                    &format!("{CURRENT} {options} app-server --analytics-default-enabled"),
                    LEGACY,
                ),
                "options: {options}"
            );
        }
    }

    #[test]
    fn supports_quoted_and_unquoted_installation_paths_with_spaces() {
        let legacy = "/Volumes/Apps Disk/ChatGPT.app/Contents/Resources/codex";
        let current = CURRENT.replace("/Applications", "/Volumes/Apps Disk");
        for executable in [current.clone(), format!("\"{current}\"")] {
            assert!(is_codex_direct_app_server_command_line(
                &format!("{executable} -c features.code_mode_host=true app-server"),
                legacy,
            ));
        }
    }

    #[test]
    fn rejects_other_installations_daemons_and_app_server_in_argument_values() {
        for command in [
            format!("{CURRENT}-other app-server"),
            format!("{CURRENT} exec app-server"),
            format!("{CURRENT} -c app-server"),
            format!("{CURRENT} --config= app-server"),
            format!("{CURRENT} --unknown app-server"),
            format!("{CURRENT} -c features.x=true app-server daemon"),
            format!("{CURRENT} app-server-other"),
            "/tmp/codex app-server".into(),
            CURRENT.replace("/Applications", "/tmp") + " app-server",
        ] {
            assert!(
                !is_codex_direct_app_server_command_line(&command, LEGACY),
                "{command}"
            );
        }
    }

    #[test]
    fn preserves_other_instances_orphans_and_cyclic_process_trees() {
        let entry = |pid, parent_pid, command: &str| CodexProcessTreeEntry {
            pid,
            parent_pid,
            command_line: command.to_owned(),
        };
        let command = format!("{CURRENT} -c features.code_mode_host=true app-server");
        let entries = vec![
            entry(100, 1, "desktop"),
            entry(110, 100, "helper"),
            entry(120, 110, &command),
            entry(200, 1, "another desktop"),
            entry(220, 200, &command),
            entry(300, 1, &command),
            entry(400, 410, &command),
            entry(410, 400, "cycle"),
            entry(500, 100, "/tmp/codex app-server"),
        ];
        assert_eq!(
            select_codex_direct_app_server_descendants(&entries, &[100], LEGACY),
            vec![120]
        );
        assert!(select_codex_direct_app_server_descendants(&entries, &[], LEGACY).is_empty());
    }

    #[test]
    fn windows_app_server_requires_the_bundled_executable_and_direct_subcommand() {
        let app = r"C:\Program Files\WindowsApps\OpenAI.Codex_test\app";
        let exe = format!("{app}\\resources\\codex.exe");
        let copied = r"C:\Users\tester\AppData\Local\OpenAI\Codex\bin";
        let args = |values: &[&str]| {
            values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
        };
        assert!(is_windows_bundled_codex_app_server(
            &exe.to_ascii_uppercase(),
            app,
            Some(copied),
            &args(&[
                "-c",
                "features.x=true",
                "app-server",
                "--listen",
                "stdio://"
            ]),
        ));
        assert!(!is_windows_bundled_codex_app_server(
            &exe,
            app,
            Some(copied),
            &args(&["app-server", "daemon"]),
        ));
        assert!(!is_windows_bundled_codex_app_server(
            &exe,
            app,
            Some(copied),
            &args(&["exec", "app-server"]),
        ));
        assert!(!is_windows_bundled_codex_app_server(
            r"D:\Other\resources\codex.exe",
            app,
            Some(copied),
            &args(&["app-server"]),
        ));
        assert!(is_windows_bundled_codex_app_server(
            &format!(r"{copied}\ac01356ff01c362f\codex.exe"),
            app,
            Some(copied),
            &args(&["app-server"]),
        ));
        assert!(!is_windows_bundled_codex_app_server(
            &format!(r"{copied}\unrelated\codex.exe"),
            app,
            Some(copied),
            &args(&["app-server"]),
        ));
    }
}
