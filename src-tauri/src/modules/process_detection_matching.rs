// Process 模块：Running-process enumeration, PID matching and window focus resolution。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
#[cfg(test)]
mod linux_antigravity_process_candidate_tests {
    use super::{
        extract_user_data_dir_from_tokens, is_linux_antigravity_process_candidate,
        is_linux_antigravity_process_candidate_from_tokens,
        linux_antigravity_external_runtime_matches_expected_launch, linux_proc_cmdline_args,
    };

    #[test]
    fn exact_custom_executable_match_is_a_main_process_candidate() {
        assert!(is_linux_antigravity_process_candidate(
            "/opt/Custom AG/Antigravity.AppImage --reuse-window",
            "/opt/custom ag/antigravity.appimage",
            true,
        ));
    }

    #[test]
    fn exact_match_rejects_structured_helpers_but_not_path_words() {
        assert!(!is_linux_antigravity_process_candidate(
            "/opt/Custom AG/Antigravity.AppImage --type=renderer",
            "/opt/custom ag/antigravity.appimage",
            true,
        ));
        assert!(is_linux_antigravity_process_candidate(
            "/opt/Antigravity Tools/antigravity-ide",
            "/opt/antigravity tools/antigravity-ide",
            true,
        ));
    }

    #[test]
    fn standard_signature_still_matches_without_expected_path_match() {
        assert!(is_linux_antigravity_process_candidate(
            "/opt/antigravity-ide/bin/antigravity-ide --reuse-window",
            "/opt/antigravity-ide/bin/antigravity-ide",
            false,
        ));
        assert!(!is_linux_antigravity_process_candidate(
            "/opt/unrelated/app --reuse-window",
            "/opt/unrelated/app",
            false,
        ));
    }

    #[test]
    fn external_electron_runtime_matches_an_antigravity_app_argument() {
        assert!(is_linux_antigravity_process_candidate(
            "/usr/bin/electron --app=\"/opt/Antigravity IDE/resources/app.asar\" --reuse-window",
            "/usr/bin/electron",
            false,
        ));
    }

    #[test]
    fn external_electron_runtime_requires_the_configured_install_root() {
        let args = linux_proc_cmdline_args(
            b"/usr/bin/electron\0--app=/opt/Antigravity IDE/resources/app.asar\0--reuse-window\0",
        );

        assert!(linux_antigravity_external_runtime_matches_expected_launch(
            &args,
            "/opt/Antigravity IDE/bin/antigravity-ide",
        ));
        assert!(!linux_antigravity_external_runtime_matches_expected_launch(
            &args,
            "/opt/Other IDE/bin/antigravity-ide",
        ));
    }

    #[test]
    fn proc_argv_preserves_spaces_in_runtime_and_profile_paths() {
        let args = linux_proc_cmdline_args(
            b"/usr/bin/electron\0--app=/opt/Antigravity IDE/resources/app.asar\0--user-data-dir=/work/profiles/managed profile\0",
        );

        assert!(is_linux_antigravity_process_candidate_from_tokens(
            &args,
            "/usr/bin/electron",
            false,
        ));
        assert_eq!(
            extract_user_data_dir_from_tokens(&args).as_deref(),
            Some("/work/profiles/managed profile")
        );
    }

    #[test]
    fn persisted_pid_candidate_rejects_unrelated_runtime_without_launcher_signature() {
        assert!(!is_linux_antigravity_process_candidate(
            "/usr/bin/sleep --user-data-dir=/work/profiles/managed 60",
            "/usr/bin/sleep",
            false,
        ));
    }
}

#[cfg(not(target_os = "macos"))]
fn is_antigravity_main_process(
    name: &str,
    exe_path: &str,
    command_line_lower: Option<&str>,
) -> bool {
    let cmdline = command_line_lower.unwrap_or("");
    if cmdline.contains("antigravity tools") || cmdline.contains("antigravity tools.app/contents/")
    {
        return false;
    }
    if !cmdline.is_empty() && is_helper_command_line(cmdline) {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        let _ = name;
        return exe_path.contains("antigravity ide.app")
            && !exe_path.contains("antigravity tools.app")
            && !exe_path.contains("crashpad");
    }

    #[cfg(target_os = "windows")]
    {
        return is_windows_antigravity_main_executable(name, exe_path);
    }

    #[cfg(target_os = "linux")]
    {
        return (name.contains("antigravity-ide") || exe_path.contains("/antigravity-ide"))
            && !name.contains("tools")
            && !exe_path.contains("tools");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (name, exe_path);
        false
    }
}

fn collect_running_process_exe_by_pid() -> HashMap<u32, String> {
    let mut map = HashMap::new();

    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        if let Ok(output) = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                    let normalized = normalize_path_for_compare(&exe);
                    if !normalized.is_empty() {
                        map.insert(pid, normalized);
                    }
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        for (pid, process) in system.processes() {
            let Some(exe) = process.exe().and_then(|value| value.to_str()) else {
                continue;
            };
            let normalized = normalize_path_for_compare(exe);
            if normalized.is_empty() {
                continue;
            }
            map.insert(pid.as_u32(), normalized);
        }
    }

    map
}

fn collect_pids_by_custom_launch_path(path: &str) -> Vec<u32> {
    let expected = normalize_path_for_compare(path);
    if expected.is_empty() {
        return Vec::new();
    }

    #[cfg(target_os = "macos")]
    let expected_app_root = normalize_macos_app_root(std::path::Path::new(path))
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());

    let mut pids = Vec::new();
    for (pid, actual) in collect_running_process_exe_by_pid() {
        if actual == expected {
            pids.push(pid);
            continue;
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(expected_root) = expected_app_root.as_ref() {
                let actual_root = normalize_macos_app_root(std::path::Path::new(&actual))
                    .map(|value| normalize_path_for_compare(&value))
                    .filter(|value| !value.is_empty());
                if actual_root.as_ref() == Some(expected_root) {
                    pids.push(pid);
                }
            }
        }
    }

    pids.sort();
    pids.dedup();
    pids
}

fn start_custom_app_from_path(path: &str) -> Result<(), String> {
    let path_obj = Path::new(path);
    if !path_obj.exists() {
        return Err(format!("指定应用路径不存在: {}", path));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(app_root) = normalize_macos_app_root(path_obj) {
            let mut cmd = Command::new("open");
            sanitize_macos_gui_launch_env(&mut cmd);
            append_managed_proxy_env_to_open_args(&mut cmd);
            cmd.args(["-n", "-a", &app_root]);
            let output = cmd
                .output()
                .map_err(|err| format!("启动指定应用失败: {}", err))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stderr = stderr.trim();
                return Err(if stderr.is_empty() {
                    "启动指定应用失败".to_string()
                } else {
                    format!("启动指定应用失败: {}", stderr)
                });
            }
            return Ok(());
        }

        let mut cmd = Command::new(path_obj);
        apply_managed_proxy_env_to_command(&mut cmd);
        sanitize_macos_gui_launch_env(&mut cmd);
        spawn_detached_unix(&mut cmd)?;
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let mut cmd = Command::new(path_obj);
        apply_managed_proxy_env_to_command(&mut cmd);
        cmd.creation_flags(CREATE_NO_WINDOW);
        spawn_command_with_trace(&mut cmd).map_err(|err| format!("启动指定应用失败: {}", err))?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new(path_obj);
        apply_managed_proxy_env_to_command(&mut cmd);
        spawn_command_with_trace(&mut cmd).map_err(|err| format!("启动指定应用失败: {}", err))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持启动指定应用".to_string())
}

pub fn restart_specified_app_by_path(custom_path: &str, timeout_secs: u64) -> Result<(), String> {
    let path = normalize_custom_path(Some(custom_path)).ok_or("指定应用启动路径不能为空")?;
    let pids = collect_pids_by_custom_launch_path(&path);
    if !pids.is_empty() {
        close_pids(&pids, timeout_secs)?;
    }
    start_custom_app_from_path(&path)
}

fn filter_entries_by_expected_launch_path(
    app_label: &str,
    entries: Vec<(u32, Option<String>)>,
    expected: Option<String>,
) -> Vec<(u32, Option<String>)> {
    filter_entries_by_expected_launch_path_with_options(app_label, entries, expected, false)
}

fn filter_antigravity_entries_by_expected_launch_path(
    app_label: &str,
    entries: Vec<(u32, Option<String>)>,
    expected: Option<String>,
) -> Vec<(u32, Option<String>)> {
    filter_entries_by_expected_launch_path_with_options(app_label, entries, expected, true)
}

fn filter_entries_by_expected_launch_path_with_options(
    app_label: &str,
    entries: Vec<(u32, Option<String>)>,
    expected: Option<String>,
    allow_linux_antigravity_layout: bool,
) -> Vec<(u32, Option<String>)> {
    if entries.is_empty() {
        return entries;
    }
    let Some(expected) = expected else {
        return Vec::new();
    };
    #[cfg(not(target_os = "linux"))]
    let _ = allow_linux_antigravity_layout;
    let exe_by_pid = collect_running_process_exe_by_pid();
    #[cfg(target_os = "macos")]
    let expected_app_root = normalize_macos_app_root(std::path::Path::new(&expected))
        .map(|root| normalize_path_for_compare(&root))
        .filter(|root| !root.is_empty());
    let mut result = Vec::new();
    let mut missing_exe = 0usize;
    let mut path_mismatch = 0usize;
    #[cfg(target_os = "macos")]
    let mut app_root_match = 0usize;
    for (pid, dir) in entries {
        match exe_by_pid.get(&pid) {
            Some(actual) if actual == &expected => result.push((pid, dir)),
            Some(actual) => {
                #[cfg(target_os = "linux")]
                if allow_linux_antigravity_layout
                    && antigravity_executable_paths_match(&expected, actual)
                {
                    result.push((pid, dir));
                    continue;
                }
                #[cfg(target_os = "linux")]
                if allow_linux_antigravity_layout
                    && linux_antigravity_external_runtime_pid_matches_expected_launch(
                        pid, &expected,
                    )
                {
                    result.push((pid, dir));
                    continue;
                }
                #[cfg(not(target_os = "macos"))]
                let _ = actual;
                #[cfg(target_os = "macos")]
                {
                    if let Some(expected_root) = expected_app_root.as_ref() {
                        let actual_root = normalize_macos_app_root(std::path::Path::new(actual))
                            .map(|root| normalize_path_for_compare(&root))
                            .filter(|root| !root.is_empty());
                        if actual_root.as_ref() == Some(expected_root) {
                            app_root_match += 1;
                            result.push((pid, dir));
                            continue;
                        }
                    }
                }
                path_mismatch += 1;
            }
            None => missing_exe += 1,
        }
    }
    if result.is_empty() {
        #[cfg(target_os = "macos")]
        {
            crate::modules::logger::log_warn(&format!(
                "[{} Resolve] 启动路径硬匹配未命中：expected={}, path_mismatch={}, missing_exe={}, app_root_match={}",
                app_label, expected, path_mismatch, missing_exe, app_root_match
            ));
        }
        #[cfg(not(target_os = "macos"))]
        crate::modules::logger::log_warn(&format!(
            "[{} Resolve] 启动路径硬匹配未命中：expected={}, path_mismatch={}, missing_exe={}",
            app_label, expected, path_mismatch, missing_exe
        ));
    } else {
        #[cfg(target_os = "macos")]
        if app_root_match > 0 {
            crate::modules::logger::log_info(&format!(
                "[{} Resolve] 使用 .app 根路径匹配到进程：expected={}, app_root_match={}",
                app_label, expected, app_root_match
            ));
        }
    }
    result
}

fn resolve_expected_antigravity_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_antigravity_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[AG Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[AG Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_vscode_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_vscode_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[VSCode Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[VSCode Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_codebuddy_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codebuddy_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[CodeBuddy Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[CodeBuddy Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_codebuddy_cn_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codebuddy_cn_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[CodeBuddy CN Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[CodeBuddy CN Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_qoder_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_qoder_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Qoder Resolve] launch path missing or invalid, skip PID match: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[Qoder Resolve] launch path is empty, skip PID match");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_trae_launch_path_for_platform_match(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<String> {
    let launch_path = match resolve_trae_launch_path_for_platform(platform) {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Trae Resolve] platform={} launch path missing or invalid, skip PID match: {}",
                platform.provider_key(),
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[Trae Resolve] launch path is empty, skip PID match");
        return None;
    }
    Some(normalized)
}

fn resolve_expected_workbuddy_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_workbuddy_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddy Resolve] 启动路径未配置或无效，跳过 PID 匹配：{}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[WorkBuddy Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn resolve_expected_codex_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_codex_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[Codex Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "macos")]
fn collect_antigravity_process_entries_from_ps() -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    let output = match output {
        Ok(value) => value,
        Err(_) => return result,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !lower.contains(ANTIGRAVITY_APP_CONTENTS_MARKER) {
            continue;
        }
        if lower.contains("antigravity tools.app/contents/")
            || lower.contains("crashpad_handler")
            || is_helper_command_line(&lower)
        {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "windows")]
fn windows_antigravity_process_name_for_expected_exe(expected_exe_path: &str) -> String {
    Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.eq_ignore_ascii_case("Antigravity.exe")
                || name.eq_ignore_ascii_case("Antigravity IDE.exe")
        })
        .unwrap_or("Antigravity IDE.exe")
        .to_string()
}

#[cfg(target_os = "windows")]
fn collect_antigravity_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let process_name = windows_antigravity_process_name_for_expected_exe(expected_exe_path);
    let script = build_windows_path_filtered_process_probe_script(&process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[AG Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[AG Probe] PowerShell 进程探测失败: {}",
                    err
                ));
            }
            return result;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[AG Probe] PowerShell 进程探测返回非 0 状态: {}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return result;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !is_antigravity_main_process(&process_name.to_lowercase(), "", Some(&lower)) {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "windows")]
fn resolve_windows_process_exe_for_match(process: &sysinfo::Process) -> (Option<String>, bool) {
    if let Some(exe) = process.exe().and_then(|value| value.to_str()) {
        let normalized = normalize_path_for_compare(exe);
        if !normalized.is_empty() {
            return (Some(normalized), false);
        }
    }
    if let Some(first) = process.cmd().first() {
        let normalized = normalize_path_for_compare(first.to_string_lossy().as_ref());
        if !normalized.is_empty() {
            return (Some(normalized), true);
        }
    }
    (None, false)
}

#[cfg(target_os = "windows")]
fn collect_antigravity_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args = process.cmd();
        let args_lower = args
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        if !is_antigravity_main_process(&name, &exe_path, Some(&args_lower)) {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(args);
                result.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    if result.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[AG Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[AG Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, result.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    result
}

#[cfg(any(target_os = "linux", test))]
fn collect_antigravity_process_entries_from_proc(
    expected_launch: &str,
) -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(value) => value,
        Err(_) => return result,
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let pid_str = file_name.to_string_lossy();
        if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        let cmdline = match std::fs::read(&cmdline_path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let args = linux_proc_cmdline_args(&cmdline);
        if args.is_empty() {
            continue;
        }
        let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
            .ok()
            .and_then(|path| path.to_str().map(str::to_string))
            .unwrap_or_default();
        let exe_path_lower = exe_path.to_lowercase();
        let expected_executable_match =
            !exe_path.is_empty() && antigravity_executable_paths_match(expected_launch, &exe_path);
        if !is_linux_antigravity_process_candidate_from_tokens(
            &args,
            &exe_path_lower,
            expected_executable_match,
        ) {
            continue;
        }
        if !expected_executable_match
            && !linux_antigravity_external_runtime_matches_expected_launch(&args, expected_launch)
        {
            continue;
        }
        let dir = extract_user_data_dir_from_tokens(&args);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "linux")]
fn read_linux_process_entry(
    pid: u32,
    expected_launch: &str,
    allowed_user_data_dirs: &HashSet<String>,
    allow_missing_user_data_dir: bool,
) -> Option<(u32, Option<String>)> {
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args = linux_proc_cmdline_args(&cmdline);
    if args.is_empty() {
        return None;
    }
    let exe_path = std::fs::read_link(format!("/proc/{pid}/exe"))
        .ok()?
        .to_str()?
        .to_string();
    if exe_path.is_empty() {
        return None;
    }
    let expected_executable_match = antigravity_executable_paths_match(expected_launch, &exe_path);
    if !is_linux_antigravity_process_candidate_from_tokens(
        &args,
        &exe_path.to_ascii_lowercase(),
        expected_executable_match,
    ) {
        return None;
    }
    if !expected_executable_match
        && !linux_antigravity_external_runtime_matches_expected_launch(&args, expected_launch)
    {
        return None;
    }
    let user_data_dir = extract_user_data_dir_from_tokens(&args);
    if !persisted_linux_antigravity_identity_matches(
        expected_launch,
        &exe_path,
        user_data_dir.as_deref(),
        allowed_user_data_dirs,
        allow_missing_user_data_dir,
    ) {
        return None;
    }
    Some((pid, user_data_dir))
}

#[cfg(target_os = "linux")]
fn linux_antigravity_external_runtime_pid_matches_expected_launch(
    pid: u32,
    expected_launch: &str,
) -> bool {
    let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args = linux_proc_cmdline_args(&cmdline);
    linux_antigravity_external_runtime_matches_expected_launch(&args, expected_launch)
}

#[cfg(target_os = "linux")]
fn append_persisted_linux_antigravity_pid(
    entries: &mut Vec<(u32, Option<String>)>,
    last_pid: Option<u32>,
    expected_launch: Option<&str>,
    allowed_user_data_dirs: &HashSet<String>,
    allow_missing_user_data_dir: bool,
) {
    let (Some(pid), Some(expected_launch)) = (last_pid, expected_launch) else {
        return;
    };
    if entries.iter().any(|(entry_pid, _)| *entry_pid == pid) {
        return;
    }
    if let Some(entry) = read_linux_process_entry(
        pid,
        expected_launch,
        allowed_user_data_dirs,
        allow_missing_user_data_dir,
    ) {
        entries.push(entry);
    }
}

pub fn collect_antigravity_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_antigravity_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "macos")]
    {
        let entries = collect_antigravity_process_entries_macos();
        if !entries.is_empty() {
            return filter_antigravity_entries_by_expected_launch_path(
                "AG",
                entries,
                expected_launch.clone(),
            );
        }
        let entries = collect_antigravity_process_entries_from_ps();
        if !entries.is_empty() {
            return filter_antigravity_entries_by_expected_launch_path(
                "AG",
                entries,
                expected_launch.clone(),
            );
        }
        // macOS 下避免回退到 sysinfo，防止触发 TCC「其他 App 数据」授权弹窗
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_antigravity_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        if strict_process_detect_enabled() {
            crate::modules::logger::log_warn(
                "[AG Probe] strict mode enabled and PowerShell returned empty; skip sysinfo fallback",
            );
            return Vec::new();
        }
        crate::modules::logger::log_warn(
            "[AG Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_antigravity_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "linux")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_antigravity_process_entries_from_proc(expected);
        if !entries.is_empty() {
            return filter_antigravity_entries_by_expected_launch_path(
                "AG",
                entries,
                expected_launch.clone(),
            );
        }
        return Vec::new();
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

fn resolve_expected_antigravity_legacy_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_antigravity_legacy_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[AG Legacy Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        crate::modules::logger::log_warn("[AG Legacy Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "macos")]
fn collect_antigravity_legacy_process_entries_from_ps() -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    let output = match output {
        Ok(value) => value,
        Err(_) => return result,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !lower.contains(ANTIGRAVITY_LEGACY_APP_CONTENTS_MARKER) {
            continue;
        }
        if lower.contains("antigravity ide.app/contents/")
            || lower.contains("antigravity tools.app/contents/")
            || lower.contains("crashpad_handler")
            || is_helper_command_line(&lower)
        {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

#[cfg(target_os = "windows")]
fn collect_antigravity_legacy_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut result = Vec::new();
    let script =
        build_windows_path_filtered_process_probe_script("Antigravity.exe", expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[AG Legacy Probe] PowerShell 进程探测失败: {}",
                err
            ));
            return result;
        }
    };
    if !output.status.success() {
        return result;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if !lower.contains("antigravity.exe") || lower.contains("antigravity ide.exe") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline);
        result.push((pid, dir));
    }
    result
}

pub fn collect_antigravity_legacy_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_antigravity_legacy_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "macos")]
    {
        let entries = collect_antigravity_legacy_process_entries_from_ps();
        if !entries.is_empty() {
            return filter_antigravity_entries_by_expected_launch_path(
                "AG Legacy",
                entries,
                expected_launch,
            );
        }
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        return collect_antigravity_legacy_process_entries_from_powershell(expected);
    }

    #[cfg(target_os = "linux")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_antigravity_process_entries_from_proc(expected)
            .into_iter()
            .filter(|(_, dir)| {
                dir.as_deref()
                    .is_some_and(|value| value.contains("Antigravity"))
            })
            .collect::<Vec<_>>();
        return filter_antigravity_entries_by_expected_launch_path(
            "AG Legacy",
            entries,
            expected_launch,
        );
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

fn pick_preferred_pid(mut pids: Vec<u32>) -> Option<u32> {
    if pids.is_empty() {
        return None;
    }
    pids.sort();
    pids.dedup();
    pids.first().copied()
}

fn normalize_non_empty_path_for_compare(value: &str) -> Option<String> {
    let normalized = normalize_path_for_compare(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn build_user_data_dir_match_target(
    requested_user_data_dir: Option<&str>,
    default_user_data_dir: Option<String>,
    fallback_to_default_when_missing: bool,
) -> Option<(String, bool)> {
    let requested_target =
        requested_user_data_dir.and_then(|value| normalize_non_empty_path_for_compare(value));
    let default_target =
        default_user_data_dir.and_then(|value| normalize_non_empty_path_for_compare(&value));
    let target = requested_target.or_else(|| {
        if fallback_to_default_when_missing {
            default_target.clone()
        } else {
            None
        }
    })?;
    let allow_none_for_target = default_target
        .as_ref()
        .map(|value| value == &target)
        .unwrap_or(false);
    Some((target, allow_none_for_target))
}

fn collect_matching_pids_by_user_data_dir(
    entries: &[(u32, Option<String>)],
    target_dir: &str,
    allow_none_for_target: bool,
) -> Vec<u32> {
    let mut matches = Vec::new();
    for (pid, dir) in entries {
        match dir.as_ref() {
            Some(value) => {
                let normalized = normalize_path_for_compare(value);
                if !normalized.is_empty() && normalized == target_dir {
                    matches.push(*pid);
                }
            }
            None => {
                if allow_none_for_target {
                    matches.push(*pid);
                }
            }
        }
    }
    matches
}

fn resolve_pid_from_entries_by_user_data_dir(
    last_pid: Option<u32>,
    target_dir: &str,
    allow_none_for_target: bool,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    if target_dir.is_empty() {
        return None;
    }

    let matches =
        collect_matching_pids_by_user_data_dir(entries, target_dir, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[PID Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(target_dir, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

fn get_default_antigravity_user_data_dir() -> Option<String> {
    crate::modules::instance::get_default_user_data_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty())
}

fn get_default_antigravity_legacy_user_data_dir() -> Option<String> {
    crate::modules::antigravity_legacy_instance::get_default_user_data_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty())
}

fn resolve_antigravity_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_antigravity_user_data_dir(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_antigravity_legacy_target_and_fallback(
    user_data_dir: Option<&str>,
) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_antigravity_legacy_user_data_dir(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_vscode_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_vscode_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_codebuddy_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_codebuddy_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_codebuddy_cn_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_codebuddy_cn_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_qoder_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_qoder_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

fn resolve_trae_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    resolve_trae_target_and_fallback_for_platform(
        user_data_dir,
        crate::modules::trae_account::TraePlatformKind::Trae,
    )
}

fn resolve_trae_target_and_fallback_for_platform(
    user_data_dir: Option<&str>,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<(String, bool)> {
    build_user_data_dir_match_target(
        user_data_dir,
        get_default_trae_user_data_dir_for_platform_for_os(platform),
        !strict_process_detect_enabled(),
    )
}

fn resolve_workbuddy_target_and_fallback(user_data_dir: Option<&str>) -> Option<(String, bool)> {
    // Prefer matching Electron userData (`.../app`) which is what the official
    // process command line contains (`--user-data-dir=~/.workbuddy/app`).
    let normalized_request = user_data_dir.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }
        crate::modules::workbuddy_instance::resolve_workbuddy_runtime_dirs(trimmed)
            .ok()
            .map(|(_, electron_dir)| electron_dir.to_string_lossy().to_string())
            .or_else(|| Some(trimmed.to_string()))
    });
    build_user_data_dir_match_target(
        normalized_request.as_deref(),
        get_default_workbuddy_user_data_dir_for_os(),
        !strict_process_detect_enabled(),
    )
}

#[cfg(target_os = "windows")]
fn get_default_codex_windows_app_user_data_dir() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let legacy_dir = std::path::PathBuf::from(&appdata).join("Codex");
    let modern_dir = legacy_dir.join("web").join("Codex");
    let target = if modern_dir.exists() {
        modern_dir
    } else if legacy_dir.exists() {
        legacy_dir
    } else {
        modern_dir
    };
    Some(target.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn get_managed_codex_windows_app_user_data_dir(codex_home: &str) -> Option<String> {
    let trimmed = codex_home.trim();
    if trimmed.is_empty() {
        return None;
    }
    crate::modules::codex_instance::get_windows_app_user_data_dir(Path::new(trimmed))
        .ok()
        .map(|value| value.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn get_default_codex_windows_app_user_data_dirs(default_codex_home: &str) -> HashSet<String> {
    let mut dirs = HashSet::new();
    if let Some(app_dir) = get_default_codex_windows_app_user_data_dir() {
        let normalized = normalize_path_for_compare(&app_dir);
        if !normalized.is_empty() {
            dirs.insert(normalized);
        }
    }
    if let Some(app_dir) = get_managed_codex_windows_app_user_data_dir(default_codex_home) {
        let normalized = normalize_path_for_compare(&app_dir);
        if !normalized.is_empty() {
            dirs.insert(normalized);
        }
    }
    dirs
}

#[cfg(target_os = "windows")]
fn is_codex_windows_main_process_command_line(cmdline: &str) -> bool {
    let lower = cmdline.to_ascii_lowercase();
    !lower.is_empty() && !is_helper_command_line(&lower) && !lower.contains("crashpad_handler")
}

#[cfg(target_os = "windows")]
fn is_codex_windows_resource_process_command_line(cmdline: &str) -> bool {
    let lower = cmdline.to_ascii_lowercase();
    lower.contains(r"\app\resources\codex.exe") || lower.contains("resources\\codex.exe")
}

#[cfg(target_os = "windows")]
fn get_codex_windows_resource_exec_path() -> Option<String> {
    let launch_path = resolve_codex_launch_path().ok()?;
    let resource_path = launch_path.parent()?.join("resources").join("codex.exe");
    Some(resource_path.to_string_lossy().to_string())
}

#[cfg(target_os = "windows")]
fn collect_codex_windows_resource_process_pids() -> Vec<u32> {
    let Some(expected_exe_path) = get_codex_windows_resource_exec_path() else {
        return Vec::new();
    };
    let script = build_windows_path_filtered_process_probe_script("codex.exe", &expected_exe_path);
    let mut pids = Vec::new();
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    if let Ok(output) = output {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, '|');
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let Ok(pid) = pid_str.parse::<u32>() else {
                    continue;
                };
                if is_codex_windows_resource_process_command_line(cmdline) {
                    pids.push(pid);
                }
            }
        }
    }
    if !pids.is_empty() {
        pids.sort();
        pids.dedup();
        return pids;
    }

    let expected = normalize_path_for_compare(&expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();
    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        if name != "codex.exe" {
            continue;
        }
        let (resolved_exe, _) = resolve_windows_process_exe_for_match(process);
        if resolved_exe.as_deref() != Some(expected.as_str()) {
            continue;
        }
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<String>>()
            .join(" ");
        if is_codex_windows_resource_process_command_line(&args_line) {
            pids.push(pid_u32);
        }
    }
    pids.sort();
    pids.dedup();
    pids
}

#[cfg(target_os = "windows")]
fn resolve_codex_windows_target_and_fallback(codex_home: Option<&str>) -> Option<(String, bool)> {
    match codex_home {
        Some(home) => build_user_data_dir_match_target(
            get_managed_codex_windows_app_user_data_dir(home).as_deref(),
            get_default_codex_windows_app_user_data_dir(),
            false,
        ),
        None => build_user_data_dir_match_target(
            None,
            get_default_codex_windows_app_user_data_dir(),
            true,
        ),
    }
}

#[cfg(target_os = "macos")]
fn collect_qoder_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let pid = match pid_str.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let lower = cmdline.to_lowercase();
            let is_qoder = lower.contains("qoder.app/contents/macos/");
            if !is_qoder {
                continue;
            }
            if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            entries.push((pid, dir));
        }
    }
    entries
}

#[cfg(target_os = "macos")]
fn collect_trae_process_entries_macos_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let bundle_pattern = format!(
        "{}/contents/macos/",
        platform.macos_app_name().to_ascii_lowercase()
    );
    let output = Command::new("ps").args(["-axo", "pid,command"]).output();
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let pid = match pid_str.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let lower = cmdline.to_lowercase();
            let is_trae = lower.contains(&bundle_pattern);
            if !is_trae {
                continue;
            }
            if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            entries.push((pid, dir));
        }
    }
    entries
}

pub fn resolve_qoder_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_qoder_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_qoder_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_qoder_process_entries();
    resolve_qoder_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn resolve_trae_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_trae_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

fn resolve_trae_pid_from_entries_for_platform(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<u32> {
    let (target, allow_none_for_target) =
        resolve_trae_target_and_fallback_for_platform(user_data_dir, platform)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_trae_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_trae_process_entries();
    resolve_trae_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn resolve_trae_pid_for_platform(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<u32> {
    let entries = collect_trae_process_entries_for_platform(platform);
    resolve_trae_pid_from_entries_for_platform(last_pid, user_data_dir, &entries, platform)
}

pub fn resolve_antigravity_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_antigravity_target_and_fallback(user_data_dir)?;
    let matches = collect_matching_pids_by_user_data_dir(entries, &target, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[AG Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(&target, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

pub fn resolve_antigravity_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    #[cfg(target_os = "linux")]
    let mut entries = collect_antigravity_process_entries();
    #[cfg(not(target_os = "linux"))]
    let entries = collect_antigravity_process_entries();
    #[cfg(target_os = "linux")]
    {
        let (allowed_user_data_dirs, allow_missing_user_data_dir) =
            resolve_antigravity_target_and_fallback(user_data_dir)
                .map(|(target, allow_missing)| (HashSet::from([target]), allow_missing))
                .unwrap_or_default();
        let expected_launch = resolve_expected_antigravity_launch_path_for_match();
        append_persisted_linux_antigravity_pid(
            &mut entries,
            last_pid,
            expected_launch.as_deref(),
            &allowed_user_data_dirs,
            allow_missing_user_data_dir,
        );
    }
    resolve_antigravity_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn resolve_antigravity_legacy_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) =
        resolve_antigravity_legacy_target_and_fallback(user_data_dir)?;
    let matches = collect_matching_pids_by_user_data_dir(entries, &target, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[AG Legacy Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(&target, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

pub fn resolve_antigravity_legacy_pid(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Option<u32> {
    let entries = collect_antigravity_legacy_process_entries();
    resolve_antigravity_legacy_pid_from_entries(last_pid, user_data_dir, &entries)
}

#[cfg(target_os = "macos")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true",
        pid
    );
    crate::modules::logger::log_info(&format!("[Focus] macOS osascript start pid={}", pid));
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("调用 osascript 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!("[Focus] macOS osascript success pid={}", pid));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "窗口聚焦失败，请检查系统辅助功能权限: {}",
        stderr.trim()
    ))
}

#[cfg(target_os = "windows")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let command = format!(
        r#"$targetPid={pid};$h=[IntPtr]::Zero;for($i=0;$i -lt 20;$i++){{$p=Get-Process -Id $targetPid -ErrorAction Stop;$h=$p.MainWindowHandle;if ($h -ne 0) {{ break }};Start-Sleep -Milliseconds 150}};if ($h -eq 0) {{ throw 'MAIN_WINDOW_HANDLE_EMPTY' }};Add-Type @' 
using System; 
using System.Runtime.InteropServices; 
public class Win32 {{ 
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd); 
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow); 
}} 
'@;[Win32]::ShowWindowAsync($h, 9) | Out-Null;[Win32]::SetForegroundWindow($h) | Out-Null;"#
    );
    crate::modules::logger::log_info(&format!("[Focus] Windows PowerShell start pid={}", pid));
    let output = powershell_output(&["-NoProfile", "-Command", &command])
        .map_err(|e| format!("调用 PowerShell 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!(
            "[Focus] Windows PowerShell success pid={}",
            pid
        ));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("窗口聚焦失败: {}", stderr.trim()))
}

#[cfg(target_os = "windows")]
pub fn focus_current_process_main_window() -> Result<(), String> {
    focus_window_by_pid(std::process::id())
}

/// Focus a specific HWND (e.g. Tauri `main` window), not the process MainWindowHandle.
/// After tray destroy, MainWindowHandle often points at the floating card.
#[cfg(target_os = "windows")]
pub fn focus_window_by_hwnd(hwnd: isize) -> Result<(), String> {
    if hwnd == 0 {
        return Err("HWND_EMPTY".to_string());
    }
    let command = format!(
        r#"Add-Type @'
using System;
using System.Runtime.InteropServices;
public class Win32FocusHwnd {{
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
}}
'@; $h = [IntPtr]{hwnd}; [Win32FocusHwnd]::ShowWindowAsync($h, 9) | Out-Null; [Win32FocusHwnd]::SetForegroundWindow($h) | Out-Null;"#
    );
    crate::modules::logger::log_info(&format!("[Focus] Windows PowerShell focus hwnd={}", hwnd));
    let output = powershell_output(&["-NoProfile", "-Command", &command])
        .map_err(|e| format!("调用 PowerShell 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!(
            "[Focus] Windows PowerShell hwnd success hwnd={}",
            hwnd
        ));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("窗口聚焦失败: {}", stderr.trim()))
}

#[cfg(target_os = "linux")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    if let Ok(output) = Command::new("wmctrl").arg("-lp").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let mut parts = line.split_whitespace();
                let win_id = parts.next();
                let _desktop = parts.next();
                let pid_str = parts.next();
                if let (Some(win_id), Some(pid_str)) = (win_id, pid_str) {
                    if pid_str == pid.to_string() {
                        let focus = Command::new("wmctrl").args(["-ia", win_id]).output();
                        if let Ok(focus) = focus {
                            if focus.status.success() {
                                crate::modules::logger::log_info(&format!(
                                    "[Focus] Linux wmctrl success pid={}",
                                    pid
                                ));
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    crate::modules::logger::log_info(&format!(
        "[Focus] Linux wmctrl not available or failed, trying xdotool pid={}",
        pid
    ));
    let output = Command::new("xdotool")
        .args(["search", "--pid", &pid.to_string(), "windowactivate"])
        .output()
        .map_err(|e| format!("调用 xdotool 失败: {}", e))?;
    if output.status.success() {
        crate::modules::logger::log_info(&format!("[Focus] Linux xdotool success pid={}", pid));
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("窗口聚焦失败: {}", stderr.trim()))
}

pub fn focus_antigravity_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_antigravity_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity IDE resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity IDE focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

pub fn focus_antigravity_legacy_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_antigravity_legacy_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Antigravity focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let target = codex_home
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty());

    let mut matches = Vec::new();
    for (pid, home) in entries {
        match (&target, home.as_ref()) {
            (Some(target_home), Some(home)) => {
                let normalized = normalize_path_for_compare(home);
                if !normalized.is_empty() && &normalized == target_home {
                    matches.push(*pid);
                }
            }
            (None, None) => {
                matches.push(*pid);
            }
            (None, Some(home)) => {
                let normalized = normalize_path_for_compare(home);
                if normalized.is_empty() {
                    matches.push(*pid);
                }
            }
            _ => {}
        }
    }

    // `ps`/`pgrep` may briefly retain zombie entries after a GUI child exits. Never
    // resolve those stale PIDs back into an instance's running state.
    matches.retain(|pid| is_pid_running(*pid));

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                target
                    .as_deref()
                    .map(|value| summarize_text_for_process_log(value, 96))
                    .unwrap_or_else(|| "-".to_string()),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

#[cfg(target_os = "windows")]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codex_windows_target_and_fallback(codex_home)?;
    let matches = collect_matching_pids_by_user_data_dir(entries, &target, allow_none_for_target);

    if let Some(pid) = last_pid {
        if is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if is_pid_running(pid) && !matches.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[Codex Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                summarize_text_for_process_log(&target, 96),
                summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    _codex_home: Option<&str>,
    _entries: &[(u32, Option<String>)],
) -> Option<u32> {
    last_pid.filter(|pid| is_pid_running(*pid))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(target_os = "windows")]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn resolve_codex_pid(last_pid: Option<u32>, _codex_home: Option<&str>) -> Option<u32> {
    last_pid.filter(|pid| is_pid_running(*pid))
}

pub fn focus_codex_instance(
    last_pid: Option<u32>,
    codex_home: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_codex_pid(last_pid, codex_home)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Codex resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] Codex focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

#[cfg(target_os = "windows")]
fn collect_vscode_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let script = build_windows_path_filtered_process_probe_script("Code.exe", expected_exe_path);
    let output = powershell_output(&["-Command", &script]);
    let output = match output {
        Ok(value) => value,
        Err(_) => return entries,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_vscode_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");
        let is_vscode = name == "code.exe" || exe_path.ends_with("\\code.exe");
        if !is_vscode
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[VSCode Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[VSCode Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

pub fn collect_vscode_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_vscode_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_vscode_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[VSCode Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_vscode_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                if !lower.contains("visual studio code.app/contents/macos/") {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("code") && !exe_path.contains("/code") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_vscode_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_vscode_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_vscode_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_vscode_process_entries();
    resolve_vscode_pid_from_entries(last_pid, user_data_dir, &entries)
}

fn get_default_vscode_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("Code")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

#[cfg(target_os = "windows")]
fn collect_codebuddy_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let process_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("CodeBuddy.exe");
    let script = build_windows_path_filtered_process_probe_script(process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[CodeBuddy Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[CodeBuddy Probe] PowerShell 进程探测失败: {}",
                    err
                ));
            }
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Probe] PowerShell 进程探测返回非 0 状态: {}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return entries;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_codebuddy_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let expected_file_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codebuddy.exe")
        .to_ascii_lowercase();

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        let is_codebuddy = name == expected_file_name
            || exe_path.ends_with(&format!("\\{}", expected_file_name))
            || name == "codebuddy.exe"
            || exe_path.ends_with("\\codebuddy.exe")
            || exe_path.contains("\\codebuddy\\");
        if !is_codebuddy
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[CodeBuddy Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

#[cfg(target_os = "windows")]
fn collect_workbuddy_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let process_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("WorkBuddy.exe");
    let script = build_windows_path_filtered_process_probe_script(process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[WorkBuddy Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy Probe] PowerShell 进程探测失败：{}",
                    err
                ));
            }
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Probe] PowerShell 进程探测返回非 0 状态：{}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return entries;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_workbuddy_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let expected_file_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workbuddy.exe")
        .to_ascii_lowercase();

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        let is_workbuddy = name == expected_file_name
            || exe_path.ends_with(&format!("\\{}", expected_file_name))
            || name == "workbuddy.exe"
            || exe_path.ends_with("\\workbuddy.exe")
            || exe_path.contains("\\workbuddy\\");
        if !is_workbuddy
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

#[cfg(target_os = "windows")]
fn collect_named_electron_process_entries_from_powershell(
    expected_exe_path: &str,
    fallback_process_name: &str,
    log_prefix: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries = Vec::new();
    let process_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback_process_name);
    let script = build_windows_path_filtered_process_probe_script(process_name, expected_exe_path);
    let output = powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    );
    let output = match output {
        Ok(value) => value,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[{} Probe] PowerShell process probe failed: {}",
                log_prefix, err
            ));
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[{} Probe] PowerShell process probe returned non-zero: {}, stderr={}",
            log_prefix,
            output.status,
            stderr.trim()
        ));
        return entries;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let lower = cmdline.to_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(cmdline).and_then(|value| {
            let normalized = normalize_path_for_compare(&value);
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        });
        entries.push((pid, dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_named_electron_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
    app_token: &str,
    fallback_process_name: &str,
    log_prefix: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let expected_file_name = Path::new(expected_exe_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(fallback_process_name)
        .to_ascii_lowercase();
    let fallback_file_name = fallback_process_name.to_ascii_lowercase();
    let app_token = app_token.to_ascii_lowercase();

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut candidates = 0usize;
    let mut path_mismatch = 0usize;
    let mut missing_exe = 0usize;
    let mut cmdline_fallback_hit = 0usize;

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();

    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }

        let name = process.name().to_string_lossy().to_lowercase();
        let exe_path = process
            .exe()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_lowercase();
        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");

        let is_target_app = name == expected_file_name
            || name == fallback_file_name
            || exe_path.ends_with(&format!("\\{}", expected_file_name))
            || exe_path.ends_with(&format!("\\{}", fallback_file_name))
            || exe_path.contains(&format!("\\{}\\", app_token))
            || exe_path.contains(&app_token);
        if !is_target_app
            || is_helper_command_line(&args_line)
            || args_line.contains("crashpad_handler")
        {
            continue;
        }
        candidates += 1;

        let (actual, used_cmdline_fallback) = resolve_windows_process_exe_for_match(process);
        match actual {
            Some(actual_path) if actual_path == expected => {
                if used_cmdline_fallback {
                    cmdline_fallback_hit += 1;
                }
                let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                    let normalized = normalize_path_for_compare(&value);
                    if normalized.is_empty() {
                        None
                    } else {
                        Some(normalized)
                    }
                });
                entries.push((pid_u32, dir));
            }
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);

    if entries.is_empty() {
        crate::modules::logger::log_warn(&format!(
            "[{} Probe] sysinfo fallback no match: expected={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            log_prefix, expected, candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    } else {
        crate::modules::logger::log_info(&format!(
            "[{} Probe] sysinfo fallback matched: expected={}, matched={}, candidates={}, path_mismatch={}, missing_exe={}, cmdline_fallback_hit={}",
            log_prefix, expected, entries.len(), candidates, path_mismatch, missing_exe, cmdline_fallback_hit
        ));
    }

    entries
}

#[cfg(target_os = "linux")]
fn collect_named_electron_process_entries_from_proc(app_token: &str) -> Vec<(u32, Option<String>)> {
    let app_token = app_token.to_ascii_lowercase();
    let mut entries = Vec::new();
    if let Ok(proc_entries) = std::fs::read_dir("/proc") {
        for entry in proc_entries.flatten() {
            let file_name = entry.file_name();
            let pid_str = file_name.to_string_lossy();
            if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }
            let pid = match pid_str.parse::<u32>() {
                Ok(value) => value,
                Err(_) => continue,
            };
            let cmdline_path = format!("/proc/{}/cmdline", pid);
            let cmdline = match std::fs::read(&cmdline_path) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if cmdline.is_empty() {
                continue;
            }
            let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
            let cmd_lower = cmdline_str.to_lowercase();
            let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                .ok()
                .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                .unwrap_or_default();
            if !cmd_lower.contains(&app_token) && !exe_path.contains(&app_token) {
                continue;
            }
            if is_helper_command_line(&cmd_lower) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(&cmdline_str);
            entries.push((pid, dir));
        }
    }
    entries
}

pub fn collect_codebuddy_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codebuddy_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_codebuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[CodeBuddy Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_codebuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                if !lower.contains("codebuddy.app/contents/macos/") {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("codebuddy") && !exe_path.contains("/codebuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn collect_codebuddy_cn_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codebuddy_cn_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_codebuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[CodeBuddy CN Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_codebuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                let is_codebuddy = lower.contains("codebuddy cn.app/contents/macos/")
                    || lower.contains("codebuddy.app/contents/macos/");
                if !is_codebuddy {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("codebuddy") && !exe_path.contains("/codebuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_codebuddy_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codebuddy_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_codebuddy_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_codebuddy_process_entries();
    resolve_codebuddy_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn resolve_codebuddy_cn_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_codebuddy_cn_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_codebuddy_cn_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_codebuddy_cn_process_entries();
    resolve_codebuddy_cn_pid_from_entries(last_pid, user_data_dir, &entries)
}

pub fn collect_qoder_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_qoder_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries =
            collect_named_electron_process_entries_from_powershell(expected, "Qoder.exe", "Qoder");
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[Qoder Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_named_electron_process_entries_from_sysinfo_fallback(
            expected,
            "qoder",
            "Qoder.exe",
            "Qoder",
        );
    }

    #[cfg(target_os = "macos")]
    {
        let entries = collect_qoder_process_entries_macos();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("Qoder", entries, expected_launch);
        }
        return Vec::new();
    }

    #[cfg(target_os = "linux")]
    {
        let entries = collect_named_electron_process_entries_from_proc("qoder");
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("Qoder", entries, expected_launch);
        }
        return Vec::new();
    }
}

pub fn collect_trae_process_entries() -> Vec<(u32, Option<String>)> {
    collect_trae_process_entries_for_platform(crate::modules::trae_account::TraePlatformKind::Trae)
}

pub fn collect_trae_process_entries_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_trae_launch_path_for_platform_match(platform);
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_named_electron_process_entries_from_powershell(
            expected,
            "Trae.exe",
            platform.display_name(),
        );
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[Trae Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_named_electron_process_entries_from_sysinfo_fallback(
            expected,
            "trae",
            "Trae.exe",
            platform.display_name(),
        );
    }

    #[cfg(target_os = "macos")]
    {
        let entries = collect_trae_process_entries_macos_for_platform(platform);
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path(
                platform.display_name(),
                entries,
                expected_launch,
            );
        }
        return Vec::new();
    }

    #[cfg(target_os = "linux")]
    {
        let entries = collect_named_electron_process_entries_from_proc("trae");
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path(
                platform.display_name(),
                entries,
                expected_launch,
            );
        }
        return Vec::new();
    }
}

pub fn collect_workbuddy_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_workbuddy_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    #[cfg(target_os = "windows")]
    {
        let expected = expected_launch
            .as_deref()
            .expect("expected launch path must exist");
        let entries = collect_workbuddy_process_entries_from_powershell(expected);
        if !entries.is_empty() {
            return entries;
        }
        crate::modules::logger::log_warn(
            "[WorkBuddy Probe] PowerShell returned empty; fallback to sysinfo probe",
        );
        return collect_workbuddy_process_entries_from_sysinfo_fallback(expected);
    }

    #[cfg(target_os = "macos")]
    {
        let mut entries = Vec::new();
        let output = Command::new("ps").args(["-axo", "pid,command"]).output();
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let pid_str = parts.next().unwrap_or("").trim();
                let cmdline = parts.next().unwrap_or("").trim();
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let lower = cmdline.to_lowercase();
                let is_workbuddy = lower.contains("workbuddy.app/contents/macos/");
                if !is_workbuddy {
                    continue;
                }
                if lower.contains("crashpad_handler") || is_helper_command_line(&lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(cmdline);
                entries.push((pid, dir));
            }
        }
        return entries;
    }

    #[cfg(target_os = "linux")]
    {
        let mut entries = Vec::new();
        if let Ok(proc_entries) = std::fs::read_dir("/proc") {
            for entry in proc_entries.flatten() {
                let file_name = entry.file_name();
                let pid_str = file_name.to_string_lossy();
                if !pid_str.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let pid = match pid_str.parse::<u32>() {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                let cmdline = match std::fs::read(&cmdline_path) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if cmdline.is_empty() {
                    continue;
                }
                let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
                let cmd_lower = cmdline_str.to_lowercase();
                let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
                    .ok()
                    .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
                    .unwrap_or_default();
                if !cmd_lower.contains("workbuddy") && !exe_path.contains("/workbuddy") {
                    continue;
                }
                if is_helper_command_line(&cmd_lower) {
                    continue;
                }
                let dir = extract_user_data_dir_from_command_line(&cmdline_str);
                entries.push((pid, dir));
            }
        }
        return entries;
    }
}

pub fn resolve_workbuddy_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let (target, allow_none_for_target) = resolve_workbuddy_target_and_fallback(user_data_dir)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, entries)
}

pub fn resolve_workbuddy_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_workbuddy_process_entries();
    resolve_workbuddy_pid_from_entries(last_pid, user_data_dir, &entries)
}

fn get_default_codebuddy_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("CodeBuddy")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

fn get_default_codebuddy_cn_user_data_dir_for_os() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join("Library")
                .join("Application Support")
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        return Some(
            Path::new(&appdata)
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        return Some(
            home.join(".config")
                .join("CodeBuddy CN")
                .to_string_lossy()
                .to_string(),
        );
    }

    #[allow(unreachable_code)]
    None
}

fn get_default_qoder_user_data_dir_for_os() -> Option<String> {
    crate::modules::qoder_instance::get_default_qoder_user_data_dir()
        .ok()
        .map(|value| value.to_string_lossy().to_string())
}

fn get_default_trae_user_data_dir_for_os() -> Option<String> {
    get_default_trae_user_data_dir_for_platform_for_os(
        crate::modules::trae_account::TraePlatformKind::Trae,
    )
}

fn get_default_trae_user_data_dir_for_platform_for_os(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<String> {
    crate::modules::trae_account::get_default_trae_data_dir_for_platform(platform)
        .ok()
        .map(|value| value.to_string_lossy().to_string())
}

fn get_default_workbuddy_user_data_dir_for_os() -> Option<String> {
    crate::modules::workbuddy_instance::get_default_workbuddy_user_data_dir()
        .ok()
        .map(|value| value.to_string_lossy().to_string())
}

pub fn focus_vscode_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let resolve_start = Instant::now();
    let pid = resolve_vscode_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    crate::modules::logger::log_info(&format!(
        "[Focus] VS Code resolve pid={} elapsed={}ms",
        pid,
        resolve_start.elapsed().as_millis()
    ));
    let focus_start = Instant::now();
    focus_window_by_pid(pid)?;
    crate::modules::logger::log_info(&format!(
        "[Focus] VS Code focus pid={} elapsed={}ms",
        pid,
        focus_start.elapsed().as_millis()
    ));
    Ok(pid)
}

pub fn focus_process_pid(pid: u32) -> Result<u32, String> {
    if pid == 0 || !is_pid_running(pid) {
        return Err("实例未运行，无法定位窗口".to_string());
    }
    focus_window_by_pid(pid)?;
    Ok(pid)
}

#[cfg(target_os = "macos")]
fn collect_antigravity_process_entries_macos() -> Vec<(u32, Option<String>)> {
    let mut pids = Vec::new();
    if let Ok(output) = Command::new("pgrep")
        .args(["-f", ANTIGRAVITY_APP_PATH])
        .output()
    {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    pids.push(pid);
                }
            }
        }
    }

    if pids.is_empty() {
        return Vec::new();
    }

    pids.sort();
    pids.dedup();

    let mut result = Vec::new();
    for pid in pids {
        let output = Command::new("ps")
            .args(["-Eww", "-p", &pid.to_string(), "-o", "command="])
            .output();
        let output = match output {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let cmdline = line.trim();
            if cmdline.is_empty() {
                continue;
            }
            if !cmdline.to_lowercase().contains(ANTIGRAVITY_APP_EXEC_MARKER) {
                continue;
            }
            let dir = extract_user_data_dir_from_command_line(cmdline);
            result.push((pid, dir));
        }
    }

    result
}
pub fn parse_extra_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in raw.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    args.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        args.push(current);
    }

    args
}

fn collect_remaining_pids(entries: &[(u32, Option<String>)]) -> Vec<u32> {
    let mut pids: Vec<u32> = entries.iter().map(|(pid, _)| *pid).collect();
    pids.sort();
    pids.dedup();
    pids
}

fn resolve_entry_user_data_dir_for_matching(
    dir: Option<&String>,
    default_dir: Option<&str>,
) -> Option<String> {
    dir.and_then(|value| normalize_non_empty_path_for_compare(value))
        .or_else(|| default_dir.and_then(normalize_non_empty_path_for_compare))
}

fn entry_matches_target_dirs(
    dir: Option<&String>,
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> bool {
    resolve_entry_user_data_dir_for_matching(dir, default_dir)
        .map(|value| target_dirs.contains(&value))
        .unwrap_or(false)
}

fn select_main_pids_by_target_dirs(
    entries: &[(u32, Option<String>)],
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> Vec<u32> {
    entries
        .iter()
        .filter_map(|(pid, dir)| {
            entry_matches_target_dirs(dir.as_ref(), target_dirs, default_dir).then_some(*pid)
        })
        .collect()
}

fn filter_entries_by_target_dirs(
    entries: Vec<(u32, Option<String>)>,
    target_dirs: &HashSet<String>,
    default_dir: Option<&str>,
) -> Vec<(u32, Option<String>)> {
    entries
        .into_iter()
        .filter(|(_, dir)| entry_matches_target_dirs(dir.as_ref(), target_dirs, default_dir))
        .collect()
}

fn collect_antigravity_process_entries_for_managed_dirs(
    user_data_dirs: &[String],
    default_dir: Option<&str>,
) -> Vec<(u32, Option<String>)> {
    #[cfg(target_os = "linux")]
    let mut entries = collect_antigravity_process_entries();
    #[cfg(not(target_os = "linux"))]
    let entries = collect_antigravity_process_entries();

    #[cfg(target_os = "linux")]
    {
        let targets: HashSet<String> = user_data_dirs
            .iter()
            .map(|value| normalize_path_for_compare(value))
            .filter(|value| !value.is_empty())
            .collect();
        let default_target = default_dir
            .map(normalize_path_for_compare)
            .filter(|value| !value.is_empty());
        let allow_missing_user_data_dir = default_target
            .as_ref()
            .is_some_and(|target| targets.contains(target));
        let expected_launch = resolve_expected_antigravity_launch_path_for_match();
        if let Ok(store) = crate::modules::instance::load_instance_store() {
            if default_target
                .as_ref()
                .is_some_and(|target| targets.contains(target))
            {
                append_persisted_linux_antigravity_pid(
                    &mut entries,
                    store.default_settings.last_pid,
                    expected_launch.as_deref(),
                    &targets,
                    allow_missing_user_data_dir,
                );
            }
            for instance in store.instances {
                let instance_target = normalize_path_for_compare(&instance.user_data_dir);
                if !instance_target.is_empty() && targets.contains(&instance_target) {
                    append_persisted_linux_antigravity_pid(
                        &mut entries,
                        instance.last_pid,
                        expected_launch.as_deref(),
                        &targets,
                        default_target.as_ref() == Some(&instance_target),
                    );
                }
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = (user_data_dirs, default_dir);
    }

    entries
}

fn close_managed_instances_common<CollectEntries, SelectMainPids, CollectRemainingEntries>(
    log_prefix: &str,
    start_message: &str,
    empty_targets_message: &str,
    not_running_message: &str,
    process_display_name: &str,
    failure_message: &str,
    user_data_dirs: &[String],
    timeout_secs: u64,
    collect_entries: CollectEntries,
    select_main_pids: SelectMainPids,
    collect_remaining_entries: CollectRemainingEntries,
    graceful_close: Option<fn(u32)>,
    graceful_wait_secs: Option<u64>,
    detail_logger: Option<fn(&[u32])>,
) -> Result<(), String>
where
    CollectEntries: Fn() -> Vec<(u32, Option<String>)>,
    SelectMainPids: Fn(&[(u32, Option<String>)], &HashSet<String>) -> Vec<u32>,
    CollectRemainingEntries: Fn(&HashSet<String>) -> Vec<(u32, Option<String>)>,
{
    crate::modules::logger::log_info(start_message);

    let target_dirs: HashSet<String> = user_data_dirs
        .iter()
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty())
        .collect();
    if target_dirs.is_empty() {
        crate::modules::logger::log_info(empty_targets_message);
        return Ok(());
    }
    crate::modules::logger::log_info(&format!(
        "[{}] target_dirs={}, timeout_secs={}",
        log_prefix,
        summarize_target_dirs_for_log(&target_dirs),
        timeout_secs
    ));

    let entries = collect_entries();
    crate::modules::logger::log_info(&format!(
        "[{}] collected_entries={}",
        log_prefix,
        summarize_process_entries_for_log(&entries)
    ));

    let mut pids = select_main_pids(&entries, &target_dirs);
    pids.sort();
    pids.dedup();
    if pids.is_empty() {
        crate::modules::logger::log_info(not_running_message);
        return Ok(());
    }
    crate::modules::logger::log_info(&format!(
        "[{}] matched_main_pids={}",
        log_prefix,
        summarize_pid_list_for_log(&pids)
    ));

    crate::modules::logger::log_info(&format!(
        "准备关闭 {} 个{}主进程...",
        pids.len(),
        process_display_name
    ));

    if let Some(graceful_close_fn) = graceful_close {
        for pid in &pids {
            graceful_close_fn(*pid);
        }
        if let Some(wait_secs) = graceful_wait_secs {
            if wait_pids_exit(&pids, wait_secs) {
                crate::modules::logger::log_info(&format!(
                    "[{}] graceful close finished, targets={}",
                    log_prefix,
                    summarize_pid_list_for_log(&pids)
                ));
                return Ok(());
            }
        }
    }

    if let Err(err) = close_pids(&pids, timeout_secs) {
        crate::modules::logger::log_warn(&format!(
            "[{}] close_pids returned error: {}",
            log_prefix, err
        ));
    }

    let mut remaining_entries = collect_remaining_entries(&target_dirs);
    if !remaining_entries.is_empty() {
        let remaining_pids = collect_remaining_pids(&remaining_entries);
        crate::modules::logger::log_warn(&format!(
            "[{}] first remaining pids after close={}",
            log_prefix,
            summarize_pid_list_for_log(&remaining_pids)
        ));
        if let Some(detail_logger_fn) = detail_logger {
            detail_logger_fn(&remaining_pids);
        }
        if !remaining_pids.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[{}] retry force close for remaining pids={}",
                log_prefix,
                summarize_pid_list_for_log(&remaining_pids)
            ));
            if let Err(err) = close_pids(&remaining_pids, 6) {
                crate::modules::logger::log_warn(&format!(
                    "[{}] retry close_pids returned error: {}",
                    log_prefix, err
                ));
            }
            remaining_entries = collect_remaining_entries(&target_dirs);
        }
    }

    if !remaining_entries.is_empty() {
        let remaining_pids = collect_remaining_pids(&remaining_entries);
        if let Some(detail_logger_fn) = detail_logger {
            detail_logger_fn(&remaining_pids);
        }
        crate::modules::logger::log_error(&format!(
            "[{}] still_running_entries={}",
            log_prefix,
            summarize_process_entries_for_log(&remaining_entries)
        ));
        return Err(format!(
            "{} ({})",
            failure_message,
            summarize_pid_list_for_log(&remaining_pids)
        ));
    }

    Ok(())
}
