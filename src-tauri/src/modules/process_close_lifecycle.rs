// Process 模块：Managed process close/restart lifecycle and graceful shutdown。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
/// 关闭受管 Antigravity IDE 实例（按 user-data-dir 匹配，包含默认实例目录）
pub fn close_antigravity_instances(
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let _ = timeout_secs;
    let default_dir = crate::modules::instance::get_default_user_data_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty());
    crate::modules::logger::log_info(&format!(
        "[AG Close] default_dir={}",
        default_dir
            .as_deref()
            .map(|value| summarize_text_for_process_log(value, 96))
            .unwrap_or_else(|| "-".to_string())
    ));
    let collect_dirs = user_data_dirs.to_vec();
    let collect_default_dir = default_dir.clone();
    let remaining_dirs = user_data_dirs.to_vec();
    let remaining_default_dir = default_dir.clone();
    close_managed_instances_common(
        "AG Close",
        "正在关闭受管 Antigravity IDE 实例...",
        "未提供可关闭的 Antigravity IDE 实例目录",
        "受管 Antigravity IDE 实例未在运行，无需关闭",
        "受管 Antigravity IDE ",
        "无法关闭受管 Antigravity IDE 实例进程，请手动关闭后重试",
        user_data_dirs,
        timeout_secs,
        move || {
            collect_antigravity_process_entries_for_managed_dirs(
                &collect_dirs,
                collect_default_dir.as_deref(),
            )
        },
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(
                collect_antigravity_process_entries_for_managed_dirs(
                    &remaining_dirs,
                    remaining_default_dir.as_deref(),
                ),
                target_dirs,
                default_dir.as_deref(),
            )
        },
        Some(request_antigravity_graceful_close as fn(u32)),
        Some(2),
        #[cfg(target_os = "windows")]
        Some(log_antigravity_process_details_for_pids as fn(&[u32])),
        #[cfg(not(target_os = "windows"))]
        None,
    )
}

pub fn close_antigravity_legacy_instances(
    user_data_dirs: &[String],
    default_user_data_dir: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let _ = timeout_secs;
    let default_dir =
        Some(normalize_path_for_compare(default_user_data_dir)).filter(|value| !value.is_empty());
    crate::modules::logger::log_info(&format!(
        "[AG Legacy Close] default_dir={}",
        default_dir
            .as_deref()
            .map(|value| summarize_text_for_process_log(value, 96))
            .unwrap_or_else(|| "-".to_string())
    ));
    close_managed_instances_common(
        "AG Legacy Close",
        "正在关闭受管 Antigravity 实例...",
        "未提供可关闭的 Antigravity 实例目录",
        "受管 Antigravity 实例未在运行，无需关闭",
        "受管 Antigravity ",
        "无法关闭受管 Antigravity 实例进程，请手动关闭后重试",
        user_data_dirs,
        timeout_secs,
        collect_antigravity_legacy_process_entries,
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(
                collect_antigravity_legacy_process_entries(),
                target_dirs,
                default_dir.as_deref(),
            )
        },
        Some(request_antigravity_graceful_close as fn(u32)),
        Some(2),
        #[cfg(target_os = "windows")]
        Some(log_antigravity_process_details_for_pids as fn(&[u32])),
        #[cfg(not(target_os = "windows"))]
        None,
    )
}

fn close_user_data_dir_scoped_instances(
    log_prefix: &str,
    process_display_name: &str,
    failure_message: &str,
    user_data_dirs: &[String],
    timeout_secs: u64,
    default_dir: Option<String>,
    collect_entries: fn() -> Vec<(u32, Option<String>)>,
) -> Result<(), String> {
    crate::modules::logger::log_info(&format!(
        "[{}] default_dir={}",
        log_prefix,
        default_dir
            .as_deref()
            .map(|value| summarize_text_for_process_log(value, 96))
            .unwrap_or_else(|| "-".to_string())
    ));
    close_managed_instances_common(
        log_prefix,
        &format!("Closing {} instances...", process_display_name),
        &format!("No {} instance directories provided", process_display_name),
        &format!("Managed {} instances are not running", process_display_name),
        process_display_name,
        failure_message,
        user_data_dirs,
        timeout_secs,
        collect_entries,
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(collect_entries(), target_dirs, default_dir.as_deref())
        },
        None,
        None,
        None,
    )
}

pub fn close_codebuddy_instances(
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    let default_dir = get_default_codebuddy_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    close_user_data_dir_scoped_instances(
        "CodeBuddy Close",
        "CodeBuddy",
        "Unable to close managed CodeBuddy instances; please close them manually and retry",
        user_data_dirs,
        timeout_secs,
        default_dir,
        collect_codebuddy_process_entries,
    )
}

pub fn close_codebuddy_cn_instances(
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    let default_dir = get_default_codebuddy_cn_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    close_user_data_dir_scoped_instances(
        "CodeBuddy CN Close",
        "CodeBuddy CN",
        "Unable to close managed CodeBuddy CN instances; please close them manually and retry",
        user_data_dirs,
        timeout_secs,
        default_dir,
        collect_codebuddy_cn_process_entries,
    )
}

pub fn close_qoder_instances(user_data_dirs: &[String], timeout_secs: u64) -> Result<(), String> {
    let default_dir = get_default_qoder_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    close_user_data_dir_scoped_instances(
        "Qoder Close",
        "Qoder",
        "Unable to close managed Qoder instances; please close them manually and retry",
        user_data_dirs,
        timeout_secs,
        default_dir,
        collect_qoder_process_entries,
    )
}

pub fn close_trae_instances(user_data_dirs: &[String], timeout_secs: u64) -> Result<(), String> {
    let default_dir = get_default_trae_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    close_user_data_dir_scoped_instances(
        "Trae Close",
        "Trae",
        "Unable to close managed Trae instances; please close them manually and retry",
        user_data_dirs,
        timeout_secs,
        default_dir,
        collect_trae_process_entries,
    )
}

pub fn close_trae_platform_default(platform_id: &str, timeout_secs: u64) -> Result<(), String> {
    let platform = crate::modules::trae_account::TraePlatformKind::parse(Some(platform_id))?;
    let default_dir = get_default_trae_user_data_dir_for_platform_for_os(platform)
        .ok_or_else(|| format!("无法获取 {} 默认数据目录", platform.display_name()))?;
    // close_trae_platform_instances already waits for idle + settle delay.
    close_trae_platform_instances(platform, &[default_dir], timeout_secs)
}

/// Wait until no matching Trae main process is visible (best-effort).
fn wait_trae_platform_idle(
    platform: crate::modules::trae_account::TraePlatformKind,
    user_data_dir: Option<&str>,
    timeout: Duration,
) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if resolve_trae_pid_for_platform(None, user_data_dir, platform).is_none() {
            return;
        }
        thread::sleep(Duration::from_millis(150));
    }
    crate::modules::logger::log_warn(&format!(
        "[Trae Close] platform={} 在 {:?} 内仍检测到残留主进程，继续启动流程",
        platform.provider_key(),
        timeout
    ));
}

/// Launch Trae on macOS with PID verification and cold-start retries.
#[cfg(target_os = "macos")]
fn launch_trae_macos_with_verification(
    platform: crate::modules::trae_account::TraePlatformKind,
    app_root: &str,
    args: &[String],
    user_data_dir: Option<&str>,
    prefer_new_instance: bool,
) -> Result<u32, String> {
    let probe_timeout = Duration::from_secs(10);
    // Prefer a single clean LaunchServices open. Stacking open -n / open / direct
    // Electron spawns races the single-instance lock and can leave a "flash then die"
    // window when NODE_OPTIONS or other env leaks into the child.
    let attempts: &[(bool, &str)] = if prefer_new_instance {
        &[(false, "open -a"), (true, "open -n -a")]
    } else {
        &[(false, "open -a"), (true, "open -n -a")]
    };

    let mut last_open_pid: Option<u32> = None;
    for (force_new, label) in attempts {
        // Drop any stale singleton locks only when forcing a brand-new process.
        if *force_new {
            clear_trae_singleton_locks(user_data_dir, platform);
        }

        let open_pid = spawn_open_app_with_options(app_root, args, *force_new)
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        last_open_pid = Some(open_pid);
        crate::modules::logger::log_info(&format!(
            "[Trae Start] platform={} 启动命令已发送（{}） open_pid={} app={} args={:?}",
            platform.provider_key(),
            label,
            open_pid,
            app_root,
            args
        ));

        let probe_started = Instant::now();
        while probe_started.elapsed() < probe_timeout {
            if let Some(resolved_pid) = resolve_trae_pid_for_platform(None, user_data_dir, platform)
            {
                crate::modules::logger::log_info(&format!(
                    "[Trae Start] platform={} 已匹配主进程 pid={}（{} 后）",
                    platform.provider_key(),
                    resolved_pid,
                    format_duration_ms(probe_started.elapsed())
                ));
                return Ok(resolved_pid);
            }
            // Also accept loose matches when strict path filter is too harsh.
            if let Some(resolved_pid) =
                resolve_trae_pid_loose_for_platform(None, user_data_dir, platform)
            {
                crate::modules::logger::log_info(&format!(
                    "[Trae Start] platform={} 宽松匹配主进程 pid={}",
                    platform.provider_key(),
                    resolved_pid
                ));
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(250));
        }
        crate::modules::logger::log_warn(&format!(
            "[Trae Start] platform={} {} 后 {} 内未匹配到主进程，尝试下一种启动方式",
            platform.provider_key(),
            label,
            format_duration_ms(probe_timeout)
        ));
        thread::sleep(Duration::from_millis(400));
    }

    // Last resort: spawn Electron binary with sanitized env (no NODE_OPTIONS).
    if let Ok(launch_path) = resolve_trae_launch_path_for_platform(platform) {
        let mut cmd = Command::new(&launch_path);
        sanitize_macos_gui_launch_env(&mut cmd);
        apply_managed_proxy_env_to_command(&mut cmd);
        for arg in args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        match spawn_detached_unix(&mut cmd) {
            Ok(child) => {
                let direct_pid = child.id();
                crate::modules::logger::log_info(&format!(
                    "[Trae Start] platform={} 直启 Electron 已发送 pid={} path={}",
                    platform.provider_key(),
                    direct_pid,
                    launch_path.display()
                ));
                let probe_started = Instant::now();
                while probe_started.elapsed() < probe_timeout {
                    if let Some(resolved_pid) =
                        resolve_trae_pid_for_platform(None, user_data_dir, platform).or_else(|| {
                            resolve_trae_pid_loose_for_platform(None, user_data_dir, platform)
                        })
                    {
                        return Ok(resolved_pid);
                    }
                    thread::sleep(Duration::from_millis(250));
                }
                last_open_pid = Some(direct_pid);
            }
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[Trae Start] platform={} 直启 Electron 失败: {}",
                    platform.provider_key(),
                    err
                ));
            }
        }
    }

    Err(format!(
        "启动 {} 失败: 已发送启动命令，但未检测到主进程（最后 open_pid={}）。若开发环境设置了 NODE_OPTIONS=--openssl-legacy-provider，请先去掉后再试；也可手动打开一次 {} 确认应用本身正常。",
        platform.display_name(),
        last_open_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        platform.display_name()
    ))
}

/// Loose PID match: trust ps cmdline under the platform .app, ignore strict exe-path filter.
#[cfg(target_os = "macos")]
fn resolve_trae_pid_loose_for_platform(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<u32> {
    let entries = collect_trae_process_entries_macos_for_platform(platform);
    if entries.is_empty() {
        return None;
    }
    let (target, allow_none_for_target) =
        resolve_trae_target_and_fallback_for_platform(user_data_dir, platform)?;
    resolve_pid_from_entries_by_user_data_dir(last_pid, &target, allow_none_for_target, &entries)
}

#[cfg(not(target_os = "macos"))]
fn resolve_trae_pid_loose_for_platform(
    _last_pid: Option<u32>,
    _user_data_dir: Option<&str>,
    _platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<u32> {
    None
}

fn clear_trae_singleton_locks(
    user_data_dir: Option<&str>,
    platform: crate::modules::trae_account::TraePlatformKind,
) {
    let Some(dir) = user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            get_default_trae_user_data_dir_for_platform_for_os(platform)
                .map(std::path::PathBuf::from)
        })
    else {
        return;
    };
    for name in ["SingletonLock", "SingletonCookie", "SingletonSocket"] {
        let path = dir.join(name);
        if path.exists() || path.symlink_metadata().is_ok() {
            match std::fs::remove_file(&path) {
                Ok(()) => crate::modules::logger::log_info(&format!(
                    "[Trae Start] 已清理残留锁: {}",
                    path.display()
                )),
                Err(err) => crate::modules::logger::log_warn(&format!(
                    "[Trae Start] 清理残留锁失败: path={}, err={}",
                    path.display(),
                    err
                )),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn format_duration_ms(duration: Duration) -> String {
    format!("{}ms", duration.as_millis())
}

pub fn close_trae_platform_instances(
    platform: crate::modules::trae_account::TraePlatformKind,
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    let default_dir = get_default_trae_user_data_dir_for_platform_for_os(platform)
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    let log_prefix = format!("{} Close", platform.display_name());
    close_managed_instances_common(
        &log_prefix,
        &format!("Closing {} instances...", platform.display_name()),
        &format!(
            "No {} instance directories provided",
            platform.display_name()
        ),
        &format!(
            "Managed {} instances are not running",
            platform.display_name()
        ),
        platform.display_name(),
        &format!(
            "Unable to close managed {} instances; please close them manually and retry",
            platform.display_name()
        ),
        user_data_dirs,
        timeout_secs,
        || collect_trae_process_entries_for_platform(platform),
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(
                collect_trae_process_entries_for_platform(platform),
                target_dirs,
                default_dir.as_deref(),
            )
        },
        None,
        None,
        None,
    )?;
    // Wait for the targeted profiles to fully disappear before inject/start.
    for dir in user_data_dirs {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            continue;
        }
        wait_trae_platform_idle(platform, Some(trimmed), Duration::from_secs(3));
    }
    thread::sleep(Duration::from_millis(350));
    Ok(())
}

pub fn close_workbuddy_instances(
    user_data_dirs: &[String],
    timeout_secs: u64,
) -> Result<(), String> {
    let default_dir = get_default_workbuddy_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    // Official processes expose Electron userData (`.../app`) in cmdline.
    let normalized: Vec<String> = user_data_dirs
        .iter()
        .filter_map(|dir| {
            let trimmed = dir.trim();
            if trimmed.is_empty() {
                return None;
            }
            crate::modules::workbuddy_instance::resolve_workbuddy_runtime_dirs(trimmed)
                .ok()
                .map(|(_, electron)| electron.to_string_lossy().to_string())
                .or_else(|| Some(trimmed.to_string()))
        })
        .collect();
    close_user_data_dir_scoped_instances(
        "WorkBuddy Close",
        "WorkBuddy",
        "Unable to close managed WorkBuddy instances; please close them manually and retry",
        &normalized,
        timeout_secs,
        default_dir,
        collect_workbuddy_process_entries,
    )
}

fn request_antigravity_graceful_close(pid: u32) {
    if pid == 0 || !is_pid_running(pid) {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true\n\
tell application \"System Events\" to keystroke \"q\" using command down",
            pid
        );
        match Command::new("osascript").args(["-e", &script]).output() {
            Ok(output) => {
                if output.status.success() {
                    crate::modules::logger::log_info(&format!(
                        "[AG Close] 已发送优雅退出请求 pid={}",
                        pid
                    ));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    crate::modules::logger::log_warn(&format!(
                        "[AG Close] 优雅退出失败 pid={} err={}",
                        pid,
                        stderr.trim()
                    ));
                }
            }
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[AG Close] 调用 osascript 失败 pid={} err={}",
                    pid, err
                ));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        crate::modules::logger::log_info(&format!(
            "[AG Close] graceful taskkill start pid={}",
            pid
        ));
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(value) => {
                if value.status.success() {
                    crate::modules::logger::log_info(&format!(
                        "[AG Close] graceful taskkill success pid={} status={}",
                        pid, value.status
                    ));
                } else {
                    crate::modules::logger::log_warn(&format!(
                        "[AG Close] graceful taskkill failed pid={} status={}",
                        pid, value.status
                    ));
                }
            }
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[AG Close] graceful taskkill error pid={} err={}",
                    pid, err
                ));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("kill")
            .args(["-15", &pid.to_string()])
            .output();
    }
}

pub fn close_pid(pid: u32, timeout_secs: u64) -> Result<(), String> {
    if pid == 0 {
        return Err("PID 无效，无法关闭进程".to_string());
    }
    if !is_pid_running(pid) {
        return Ok(());
    }

    let close_error = send_close_signal(pid);
    if wait_pids_exit(&[pid], timeout_secs) {
        Ok(())
    } else {
        Err(crate::modules::windows_operation::format_error(
            "stop_process",
            "无法关闭实例进程",
            close_error
                .as_deref()
                .unwrap_or("目标进程在等待超时后仍在运行"),
            None,
            &[pid],
            true,
            true,
            true,
        ))
    }
}

fn send_close_signal(pid: u32) -> Option<String> {
    if pid == 0 || !is_pid_running(pid) {
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        crate::modules::logger::log_info(&format!("[AG Close] taskkill start pid={}", pid));
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(value) => {
                if value.status.success() {
                    crate::modules::logger::log_info(&format!(
                        "[AG Close] taskkill success pid={} status={}",
                        pid, value.status
                    ));
                    return None;
                } else {
                    let stderr = String::from_utf8_lossy(&value.stderr);
                    let stdout = String::from_utf8_lossy(&value.stdout);
                    crate::modules::logger::log_warn(&format!(
                        "[AG Close] taskkill failed pid={} status={} stderr={} stdout={}",
                        pid,
                        value.status,
                        stderr.trim(),
                        stdout.trim()
                    ));
                    return Some(format_kill_command_failure(
                        pid,
                        "taskkill",
                        value.status,
                        &value.stderr,
                        &value.stdout,
                    ));
                }
            }
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[AG Close] taskkill error pid={} err={}",
                    pid, err
                ));
                return Some(format!("pid {}: taskkill failed: {}", pid, err));
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let result = Command::new("kill")
            .args(["-15", &pid.to_string()])
            .output();
        return match result {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format_kill_command_failure(
                pid,
                "kill",
                output.status,
                &output.stderr,
                &output.stdout,
            )),
            Err(error) => Some(format!("pid {}: kill failed: {}", pid, error)),
        };
    }
}

#[cfg(target_os = "windows")]
fn log_antigravity_process_details_for_pids(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let mut unique = pids.to_vec();
    unique.sort();
    unique.dedup();
    let pid_list = unique
        .iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<String>>()
        .join(",");
    let script = format!(
        "$ids=@({}); Get-CimInstance Win32_Process -Filter \"Name='Antigravity IDE.exe' OR Name='Antigravity.exe'\" | Where-Object {{$ids -contains $_.ProcessId}} | ForEach-Object {{ \"$($_.ProcessId)|$($_.ParentProcessId)|$($_.CommandLine)\" }}",
        pid_list
    );
    match powershell_output(&["-Command", &script]) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().is_empty() {
                crate::modules::logger::log_warn(&format!(
                    "[AG Close] remaining pid details not found for {}",
                    summarize_pid_list_for_log(&unique)
                ));
            } else {
                for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                    crate::modules::logger::log_warn(&format!(
                        "[AG Close] remaining_pid_detail {}",
                        summarize_text_for_process_log(line.trim(), 240)
                    ));
                }
            }
        }
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[AG Close] read remaining pid details failed: {}",
                err
            ));
        }
    }
}

#[cfg(target_os = "windows")]
fn log_vscode_process_details_for_pids(pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let mut unique = pids.to_vec();
    unique.sort();
    unique.dedup();
    let pid_list = unique
        .iter()
        .map(|pid| pid.to_string())
        .collect::<Vec<String>>()
        .join(",");
    let script = format!(
        "$ids=@({}); Get-CimInstance Win32_Process -Filter \"Name='Code.exe'\" | Where-Object {{$ids -contains $_.ProcessId}} | ForEach-Object {{ \"$($_.ProcessId)|$($_.ParentProcessId)|$($_.CommandLine)\" }}",
        pid_list
    );
    match powershell_output(&["-Command", &script]) {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.trim().is_empty() {
                crate::modules::logger::log_warn(&format!(
                    "[VSCode Close] remaining pid details not found for {}",
                    summarize_pid_list_for_log(&unique)
                ));
            } else {
                for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
                    crate::modules::logger::log_warn(&format!(
                        "[VSCode Close] remaining_pid_detail {}",
                        summarize_text_for_process_log(line.trim(), 240)
                    ));
                }
            }
        }
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[VSCode Close] read remaining pid details failed: {}",
                err
            ));
        }
    }
}

fn wait_pids_exit(pids: &[u32], timeout_secs: u64) -> bool {
    if pids.is_empty() {
        return true;
    }
    let start = std::time::Instant::now();
    loop {
        let mut any_alive = false;
        for pid in pids {
            if *pid != 0 && is_pid_running(*pid) {
                any_alive = true;
                break;
            }
        }
        if !any_alive {
            return true;
        }
        if start.elapsed() >= Duration::from_secs(timeout_secs) {
            return false;
        }
        thread::sleep(Duration::from_millis(120));
    }
}

fn collect_running_pids(pids: &[u32]) -> Vec<u32> {
    let mut remaining: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| *pid != 0 && is_pid_running(*pid))
        .collect();
    remaining.sort();
    remaining.dedup();
    remaining
}

fn close_pids(pids: &[u32], timeout_secs: u64) -> Result<(), String> {
    if pids.is_empty() {
        return Ok(());
    }
    let mut targets: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| *pid != 0 && is_pid_running(*pid))
        .collect();
    targets.sort();
    targets.dedup();
    if targets.is_empty() {
        return Ok(());
    }
    crate::modules::logger::log_info(&format!(
        "[ClosePids] targets={}, timeout_secs={}",
        summarize_pid_list_for_log(&targets),
        timeout_secs
    ));

    let close_errors = targets
        .iter()
        .filter_map(|pid| send_close_signal(*pid))
        .collect::<Vec<_>>();

    if wait_pids_exit(&targets, timeout_secs) {
        crate::modules::logger::log_info(&format!(
            "[ClosePids] all exited, targets={}",
            summarize_pid_list_for_log(&targets)
        ));
        Ok(())
    } else {
        let remaining: Vec<u32> = targets
            .iter()
            .copied()
            .filter(|pid| is_pid_running(*pid))
            .collect();
        crate::modules::logger::log_error(&format!(
            "[ClosePids] timeout, remaining={}",
            summarize_pid_list_for_log(&remaining)
        ));
        let original_reason = if close_errors.is_empty() {
            format!(
                "目标进程在等待超时后仍在运行: pids={}",
                summarize_pid_list_for_log(&remaining)
            )
        } else {
            close_errors.join(" | ")
        };
        Err(crate::modules::windows_operation::format_error(
            "stop_process",
            "无法关闭实例进程",
            &original_reason,
            None,
            &remaining,
            true,
            true,
            true,
        ))
    }
}

fn is_legacy_platform_adapter_executable(executable: &str) -> bool {
    let executable = executable.trim();
    if executable.is_empty()
        || !executable.contains("/platform-packages/")
        || !executable.contains("/current/adapter/")
    {
        return false;
    }

    let Some(file_name) = Path::new(executable)
        .file_name()
        .map(|value| value.to_string_lossy())
    else {
        return false;
    };

    file_name.starts_with("cockpit-") && file_name.ends_with("-adapter")
}

fn orphaned_legacy_platform_adapter_pid_from_ps_line(line: &str, current_pid: u32) -> Option<u32> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse::<u32>().ok()?;
    let ppid = parts.next()?.parse::<u32>().ok()?;
    let executable = parts.next()?;

    if pid == 0 || pid == current_pid || ppid != 1 {
        return None;
    }
    is_legacy_platform_adapter_executable(executable).then_some(pid)
}

pub fn close_orphaned_legacy_platform_adapter_processes(
    timeout_secs: u64,
) -> Result<usize, String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid=,ppid=,command="])
            .output()
            .map_err(|err| format!("扫描旧平台 adapter 进程失败: {}", err))?;
        if !output.status.success() {
            return Err(format!(
                "扫描旧平台 adapter 进程失败: status={}, stderr={}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let current_pid = std::process::id();
        let mut pids: Vec<u32> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| orphaned_legacy_platform_adapter_pid_from_ps_line(line, current_pid))
            .collect();
        pids.sort();
        pids.dedup();
        if pids.is_empty() {
            return Ok(0);
        }

        crate::modules::logger::log_info(&format!(
            "[LegacyAdapterCleanup] closing orphaned legacy platform adapters: {}",
            summarize_pid_list_for_log(&pids)
        ));
        close_pids(&pids, timeout_secs)?;
        Ok(pids.len())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = timeout_secs;
        Ok(0)
    }
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn close_processes_by_exact_exe_paths(
    exe_paths: &[std::path::PathBuf],
    timeout_secs: u64,
) -> Result<usize, String> {
    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<(String, String, String)> = Vec::new();
        let mut expected_paths = HashSet::new();
        let mut process_names = HashSet::new();
        for path in exe_paths {
            let raw_path = path.to_string_lossy().to_string();
            let normalized = normalize_path_for_compare(&raw_path);
            if normalized.is_empty() {
                continue;
            }
            let Some(file_name) = path
                .file_name()
                .map(|value| value.to_string_lossy().trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            expected_paths.insert(normalized.clone());
            process_names.insert(file_name.to_ascii_lowercase());
            candidates.push((raw_path, normalized, file_name));
        }

        if candidates.is_empty() {
            return Ok(0);
        }

        let current_pid = std::process::id();
        let mut pids = Vec::new();
        for (raw_path, _, process_name) in &candidates {
            let script = build_windows_path_filtered_process_probe_script(process_name, raw_path);
            match powershell_output_with_timeout(
                &["-NoProfile", "-Command", &script],
                WINDOWS_PROCESS_PROBE_TIMEOUT,
            ) {
                Ok(output) if output.status.success() => {
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let mut parts = line.splitn(2, '|');
                        let pid_str = parts.next().unwrap_or("").trim();
                        let Ok(pid) = pid_str.parse::<u32>() else {
                            continue;
                        };
                        if pid != current_pid {
                            pids.push(pid);
                        }
                    }
                }
                Ok(output) => {
                    crate::modules::logger::log_warn(&format!(
                        "[CloseByExe] PowerShell probe failed: name={} status={} stderr={}",
                        process_name,
                        output.status,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
                Err(err) => {
                    crate::modules::logger::log_warn(&format!(
                        "[CloseByExe] PowerShell probe error: name={} err={}",
                        process_name, err
                    ));
                }
            }
        }

        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        for (pid, process) in system.processes() {
            let pid_u32 = pid.as_u32();
            if pid_u32 == current_pid {
                continue;
            }
            let process_name = process.name().to_string_lossy().to_ascii_lowercase();
            if !process_names.contains(&process_name) {
                continue;
            }
            let (resolved_exe, _) = resolve_windows_process_exe_for_match(process);
            if let Some(resolved_exe) = resolved_exe {
                if expected_paths.contains(&resolved_exe) {
                    pids.push(pid_u32);
                }
            }
        }

        pids.sort();
        pids.dedup();
        if pids.is_empty() {
            return Ok(0);
        }
        crate::modules::logger::log_info(&format!(
            "[CloseByExe] closing exact-path processes: targets={}, paths={:?}",
            summarize_pid_list_for_log(&pids),
            candidates
                .iter()
                .map(|(_, normalized, _)| summarize_text_for_process_log(normalized, 160))
                .collect::<Vec<_>>()
        ));
        close_pids(&pids, timeout_secs)?;
        Ok(pids.len())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (exe_paths, timeout_secs);
        Ok(0)
    }
}

#[cfg(target_os = "windows")]
fn try_launch_via_shortcut(shortcut_pattern: &str) -> Result<Option<u32>, String> {
    use std::fs;
    let Some(config_dir) = dirs::config_dir() else {
        return Ok(None);
    };

    let taskbar_dir =
        config_dir.join("Microsoft\\Internet Explorer\\Quick Launch\\User Pinned\\TaskBar");
    if taskbar_dir.exists() {
        if let Ok(entries) = fs::read_dir(taskbar_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let name_lower = name.to_lowercase();
                    let matches_pattern = if shortcut_pattern == "antigravity" {
                        name_lower.contains("antigravity")
                            && !name_lower.contains("antigravity ide")
                    } else {
                        name_lower.contains(shortcut_pattern)
                    };
                    if matches_pattern && name_lower.ends_with(".lnk") {
                        crate::modules::logger::log_info(&format!(
                            "[Shortcut Launch] 找到任务栏快捷方式: {}, 尝试通过快捷方式启动",
                            name
                        ));
                        let mut cmd = std::process::Command::new("cmd");
                        cmd.arg("/C");
                        cmd.arg("start");
                        cmd.arg("");
                        cmd.arg(&path);

                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(CREATE_NO_WINDOW)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null());
                        match cmd.spawn() {
                            Ok(child) => {
                                crate::modules::logger::log_info(
                                    "[Shortcut Launch] 快捷方式启动命令已执行",
                                );
                                return Ok(Some(resolve_antigravity_pid_after_shortcut_launch(
                                    child.id(),
                                )));
                            }
                            Err(e) => {
                                crate::modules::logger::log_warn(&format!(
                                    "[Shortcut Launch] 快捷方式启动失败: {}",
                                    e
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

#[cfg(target_os = "windows")]
fn resolve_antigravity_pid_after_shortcut_launch(command_pid: u32) -> u32 {
    let probe_started = Instant::now();
    let timeout = Duration::from_secs(6);
    while probe_started.elapsed() < timeout {
        if let Some(resolved_pid) = resolve_antigravity_pid(None, None) {
            crate::modules::logger::log_info(&format!(
                "[Shortcut Launch] 已解析 Antigravity PID: command_pid={}, resolved_pid={}",
                command_pid, resolved_pid
            ));
            return resolved_pid;
        }
        thread::sleep(Duration::from_millis(200));
    }
    crate::modules::logger::log_warn(&format!(
        "[Shortcut Launch] 启动后 6s 内未匹配到 Antigravity PID，回退 cmd pid={}",
        command_pid
    ));
    command_pid
}

#[cfg(target_os = "windows")]
pub fn normalize_actual_path_case(path: &std::path::Path) -> std::path::PathBuf {
    use std::fs;
    if let Ok(canonical) = fs::canonicalize(path) {
        let path_str = canonical.to_string_lossy().to_string();
        let stripped = if path_str.to_lowercase().starts_with("\\\\?\\unc\\") {
            format!("\\\\{}", &path_str[8..])
        } else if path_str.to_lowercase().starts_with("\\\\?\\") {
            path_str[4..].to_string()
        } else {
            path_str
        };
        std::path::PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// 启动 Antigravity IDE
pub fn start_antigravity() -> Result<u32, String> {
    start_antigravity_with_args("", &[])
}

/// 启动 Antigravity IDE（支持 user-data-dir 与附加参数）
pub fn start_antigravity_with_args(
    user_data_dir: &str,
    extra_args: &[String],
) -> Result<u32, String> {
    crate::modules::logger::log_info("正在启动 Antigravity IDE...");

    #[cfg(target_os = "macos")]
    let launch_path = resolve_antigravity_launch_path().ok();
    #[cfg(not(target_os = "macos"))]
    let launch_path = resolve_antigravity_launch_path()?;

    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_macos_app_root_from_config("antigravity").or_else(|| {
            launch_path
                .as_ref()
                .and_then(|path| normalize_macos_app_root(path))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("antigravity"))?;

        let user_data_dir_trimmed = user_data_dir.trim();
        let mut args: Vec<String> = Vec::new();
        if !user_data_dir_trimmed.is_empty() {
            args.push("--user-data-dir".to_string());
            args.push(user_data_dir_trimmed.to_string());
        }
        for arg in extra_args {
            if !arg.trim().is_empty() {
                args.push(arg.to_string());
            }
        }
        let pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Antigravity IDE 失败: {}", e))?;
        crate::modules::logger::log_info("Antigravity IDE 启动命令已发送（open -n -a）");
        if !user_data_dir_trimmed.is_empty() {
            let probe_started = Instant::now();
            let timeout = Duration::from_secs(6);
            while probe_started.elapsed() < timeout {
                if let Some(resolved_pid) =
                    resolve_antigravity_pid(None, Some(user_data_dir_trimmed))
                {
                    return Ok(resolved_pid);
                }
                thread::sleep(Duration::from_millis(200));
            }
            crate::modules::logger::log_warn(&format!(
                "[AG Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
                pid
            ));
        }
        return Ok(pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        if user_data_dir.trim().is_empty() && extra_args.is_empty() {
            if let Ok(Some(pid)) = try_launch_via_shortcut("antigravity ide") {
                return Ok(pid);
            }
        }

        let launch_path = normalize_actual_path_case(&launch_path);
        let mut cmd = Command::new(&launch_path);
        if let Some(parent) = launch_path.parent() {
            cmd.current_dir(parent);
        }
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS); // CREATE_NO_WINDOW | detached
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if !user_data_dir.trim().is_empty() {
            cmd.arg("--user-data-dir");
            cmd.arg(user_data_dir.trim());
        }
        cmd.arg("--reuse-window");
        for arg in extra_args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 Antigravity IDE 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "Antigravity IDE 已启动: {}",
            launch_path.to_string_lossy()
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if !user_data_dir.trim().is_empty() {
            cmd.arg("--user-data-dir");
            cmd.arg(user_data_dir.trim());
        }
        cmd.arg("--reuse-window");
        for arg in extra_args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
        let child = spawn_detached_unix(&mut cmd)
            .map_err(|e| format!("启动 Antigravity IDE 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "Antigravity IDE 已启动: {}",
            launch_path.to_string_lossy()
        ));
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("不支持的操作系统".to_string())
}

pub fn start_antigravity_legacy_with_args(
    user_data_dir: &str,
    extra_args: &[String],
) -> Result<u32, String> {
    crate::modules::logger::log_info("正在启动 Antigravity...");

    let launch_path = resolve_antigravity_legacy_launch_path()?;

    #[cfg(target_os = "macos")]
    {
        let app_root = normalize_macos_app_root(&launch_path)
            .ok_or_else(|| app_path_missing_error("antigravity"))?;
        let user_data_dir_trimmed = user_data_dir.trim();
        let mut args: Vec<String> = Vec::new();
        if !user_data_dir_trimmed.is_empty() {
            args.push("--user-data-dir".to_string());
            args.push(user_data_dir_trimmed.to_string());
        }
        for arg in extra_args {
            if !arg.trim().is_empty() {
                args.push(arg.to_string());
            }
        }
        let pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Antigravity 失败: {}", e))?;
        crate::modules::logger::log_info("Antigravity 启动命令已发送（open -n -a）");
        if !user_data_dir_trimmed.is_empty() {
            let probe_started = Instant::now();
            let timeout = Duration::from_secs(6);
            while probe_started.elapsed() < timeout {
                if let Some(resolved_pid) =
                    resolve_antigravity_legacy_pid(None, Some(user_data_dir_trimmed))
                {
                    return Ok(resolved_pid);
                }
                thread::sleep(Duration::from_millis(200));
            }
            crate::modules::logger::log_warn(&format!(
                "[AG Legacy Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
                pid
            ));
        }
        return Ok(pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        if user_data_dir.trim().is_empty() && extra_args.is_empty() {
            if let Ok(Some(pid)) = try_launch_via_shortcut("antigravity") {
                return Ok(pid);
            }
        }

        let launch_path = normalize_actual_path_case(&launch_path);
        let mut cmd = Command::new(&launch_path);
        if let Some(parent) = launch_path.parent() {
            cmd.current_dir(parent);
        }
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if !user_data_dir.trim().is_empty() {
            cmd.arg("--user-data-dir");
            cmd.arg(user_data_dir.trim());
        }
        cmd.arg("--reuse-window");
        for arg in extra_args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 Antigravity 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "Antigravity 已启动: {}",
            launch_path.to_string_lossy()
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if !user_data_dir.trim().is_empty() {
            cmd.arg("--user-data-dir");
            cmd.arg(user_data_dir.trim());
        }
        cmd.arg("--reuse-window");
        for arg in extra_args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Antigravity 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "Antigravity 已启动: {}",
            launch_path.to_string_lossy()
        ));
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("不支持的操作系统".to_string())
}

#[cfg(target_os = "macos")]
pub fn collect_codex_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codex_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut pids: Vec<u32> = Vec::new();
    if let Ok(output) = Command::new("pgrep")
        .args(["-f", "(ChatGPT|Codex)\\.app/Contents/MacOS/(ChatGPT|Codex)"])
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
        let output = Command::new("ps")
            .args(["-Eww", "-o", "pid=,command="])
            .output();
        let output = match output {
            Ok(value) => value,
            Err(_) => return result,
        };
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
                Ok(value) => value,
                Err(_) => continue,
            };
            let lower = cmdline.to_lowercase();
            if !is_codex_macos_main_process_command_line(&lower) {
                continue;
            }
            pids.push(pid);
        }
    }

    pids.sort();
    pids.dedup();

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
        let cmdline = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if cmdline.is_empty() {
            continue;
        }
        let lower = cmdline.to_lowercase();
        if !is_codex_macos_main_process_command_line(&lower) {
            continue;
        }
        let tokens = split_command_tokens(&cmdline);
        let mut args: Vec<String> = Vec::new();
        let mut env_tokens: Vec<String> = Vec::new();
        let mut saw_env = false;
        for (idx, token) in tokens.into_iter().enumerate() {
            if idx == 0 {
                args.push(token);
                continue;
            }
            if !saw_env && is_env_token(&token) {
                saw_env = true;
                env_tokens.push(token);
                continue;
            }
            if saw_env {
                env_tokens.push(token);
            } else {
                args.push(token);
            }
        }
        let args_lower = args.join(" ").to_lowercase();
        let is_helper = args_lower.contains("--type=")
            || args_lower.contains("helper")
            || args_lower.contains("renderer")
            || args_lower.contains("gpu")
            || args_lower.contains("crashpad")
            || args_lower.contains("utility")
            || args_lower.contains("audio")
            || args_lower.contains("sandbox");
        if is_helper {
            continue;
        }
        let mut codex_home = extract_env_value_from_tokens(&env_tokens, "CODEX_HOME");
        if codex_home.is_none() {
            codex_home = env_tokens
                .iter()
                .find_map(|token| token.strip_prefix("CODEX_HOME="))
                .map(|value| value.to_string());
        }
        if codex_home.is_none() {
            codex_home = extract_env_value(&cmdline, "CODEX_HOME");
        }
        if let Some(ref home) = codex_home {
            crate::modules::logger::log_info(&format!(
                "[Codex Instances] pid={} CODEX_HOME={}",
                pid, home
            ));
        }
        result.push((pid, codex_home));
    }
    filter_entries_by_expected_launch_path("Codex", result, expected_launch)
        .into_iter()
        .filter(|(pid, _)| is_pid_running(*pid))
        .collect()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn collect_codex_process_tree_entries() -> Vec<CodexProcessTreeEntry> {
    let output = match Command::new("ps")
        .args(["-axww", "-o", "pid=,ppid=,command="])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut pid_parts = line.trim().splitn(2, |ch: char| ch.is_whitespace());
            let pid = pid_parts.next()?.trim().parse::<u32>().ok()?;
            let remainder = pid_parts.next()?.trim_start();
            let mut parent_parts = remainder.splitn(2, |ch: char| ch.is_whitespace());
            let parent_pid = parent_parts.next()?.trim().parse::<u32>().ok()?;
            let command_line = parent_parts.next()?.trim().to_string();
            if command_line.is_empty() {
                return None;
            }
            Some(CodexProcessTreeEntry {
                pid,
                parent_pid,
                command_line,
            })
        })
        .collect()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn collect_codex_direct_app_server_pids_for_roots(root_pids: &[u32]) -> Vec<u32> {
    #[cfg(target_os = "macos")]
    let Some(app_root) = resolve_codex_launch_path()
        .ok()
        .and_then(|path| resolve_macos_app_root_from_launch_path(&path))
        .or_else(|| resolve_macos_app_root_from_config("codex"))
    else {
        return Vec::new();
    };
    #[cfg(target_os = "macos")]
    let expected_resource_executable = Path::new(&app_root)
        .join("Contents")
        .join("Resources")
        .join("codex")
        .to_string_lossy()
        .to_string();
    #[cfg(target_os = "linux")]
    let Some(expected_resource_executable) = resolve_codex_launch_path().ok().and_then(|path| {
        let resolved = std::fs::canonicalize(&path).unwrap_or(path);
        resolved
            .parent()
            .map(|parent| parent.join("resources").join("codex"))
            .filter(|candidate| candidate.is_file())
            .map(|candidate| candidate.to_string_lossy().to_string())
    }) else {
        return Vec::new();
    };
    select_codex_direct_app_server_descendants(
        &collect_codex_process_tree_entries(),
        root_pids,
        &expected_resource_executable,
    )
}

/// 查找指定 Codex profile 对应的官方 direct `app-server` 子进程。
///
/// 官方桌面端负责启动 app-server，Cockpit 无法接管它的 stdio；这里提供只读的
/// 进程归属查询，供认证诊断记录 PID、网络连接和实例关系。
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn collect_codex_app_server_pids_for_profile(profile_dir: &Path) -> Vec<u32> {
    let profile_key = normalize_path_for_compare(&profile_dir.to_string_lossy());
    let default_profile_key = dirs::home_dir()
        .map(|home| home.join(".codex"))
        .map(|path| normalize_path_for_compare(&path.to_string_lossy()));
    let root_pids = collect_codex_process_entries()
        .into_iter()
        .filter_map(|(pid, codex_home)| {
            let matches_profile = codex_home
                .as_deref()
                .map(normalize_path_for_compare)
                .is_some_and(|home| home == profile_key)
                || (codex_home.is_none()
                    && default_profile_key.as_deref() == Some(profile_key.as_str()));
            matches_profile.then_some(pid)
        })
        .collect::<Vec<_>>();
    collect_codex_direct_app_server_pids_for_roots(&root_pids)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn collect_codex_app_server_pids_for_profile(_profile_dir: &Path) -> Vec<u32> {
    Vec::new()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn close_captured_codex_direct_app_servers(
    captured_pids: &[u32],
    timeout_secs: u64,
) -> Result<(), String> {
    let remaining = collect_running_pids(captured_pids);
    if remaining.is_empty() {
        return Ok(());
    }
    crate::modules::logger::log_warn(&format!(
        "[Codex Close] direct app-server remained after main process exit; closing captured pids={}",
        summarize_pid_list_for_log(&remaining)
    ));
    close_pids(&remaining, timeout_secs.min(5).max(1)).map_err(|error| {
        format!(
            "failed to close Codex direct app-server before auth switch: {}",
            error
        )
    })
}

#[cfg(target_os = "windows")]
fn collect_codex_process_entries_from_powershell(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let expected = escape_powershell_single_quoted(expected_exe_path);
    let script = format!(
        r#"$processNames=@('ChatGPT.exe','Codex.exe');
$expectedRaw='{expected}';
function Normalize-ExePath([string]$path) {{
  if ([string]::IsNullOrWhiteSpace($path)) {{ return $null }}
  $value = $path.Trim().Trim('"')
  $value = [Environment]::ExpandEnvironmentVariables($value)
  if ($value.StartsWith('\\?\UNC\', [System.StringComparison]::OrdinalIgnoreCase)) {{
    $value = '\\' + $value.Substring(8)
  }} elseif ($value.StartsWith('\\?\', [System.StringComparison]::OrdinalIgnoreCase)) {{
    $value = $value.Substring(4)
  }}
  $value = $value -replace '/', '\'
  try {{ $value = [System.IO.Path]::GetFullPath($value) }} catch {{}}
  if ($value.StartsWith('\\?\UNC\', [System.StringComparison]::OrdinalIgnoreCase)) {{
    $value = '\\' + $value.Substring(8)
  }} elseif ($value.StartsWith('\\?\', [System.StringComparison]::OrdinalIgnoreCase)) {{
    $value = $value.Substring(4)
  }}
  return $value.ToLowerInvariant()
}}
function Get-ExePathFromCmdLine([string]$cmdline) {{
  if ([string]::IsNullOrWhiteSpace($cmdline)) {{ return $null }}
  $value = $cmdline.Trim()
  if ($value.StartsWith('"')) {{
    $end = $value.IndexOf('"', 1)
    if ($end -gt 1) {{ return $value.Substring(1, $end - 1) }}
  }}
  $exeMatch = [regex]::Match($value, '^[^""]+?\.exe', [System.Text.RegularExpressions.RegexOptions]::IgnoreCase)
  if ($exeMatch.Success) {{ return $exeMatch.Value.Trim() }}
  $space = $value.IndexOf(' ')
  if ($space -gt 0) {{ return $value.Substring(0, $space) }}
  return $value
}}
$expected = Normalize-ExePath $expectedRaw
if ([string]::IsNullOrWhiteSpace($expected)) {{ exit 0 }}
Get-CimInstance Win32_Process |
  Where-Object {{
    if (-not ($processNames -contains $_.Name)) {{
      $false
    }} else {{
      $exe = Normalize-ExePath $_.ExecutablePath
      if (-not $exe) {{ $exe = Normalize-ExePath (Get-ExePathFromCmdLine $_.CommandLine) }}
      $exe -eq $expected
    }}
  }} |
  ForEach-Object {{ "$($_.ProcessId)|$($_.ParentProcessId)|$($_.CommandLine)" }}"#
    );

    let output = match powershell_output_with_timeout(
        &["-NoProfile", "-Command", &script],
        WINDOWS_PROCESS_PROBE_TIMEOUT,
    ) {
        Ok(value) => value,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::TimedOut {
                crate::modules::logger::log_warn("[Codex Probe] PowerShell 进程探测超时（5s）");
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Probe] PowerShell 进程探测失败: {}",
                    err
                ));
            }
            return entries;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        crate::modules::logger::log_warn(&format!(
            "[Codex Probe] PowerShell 进程探测返回非 0 状态: {}, stderr={}",
            output.status,
            stderr.trim()
        ));
        return entries;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut main_entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut child_user_data_by_parent: HashMap<u32, String> = HashMap::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let pid_str = parts.next().unwrap_or("").trim();
        let parent_pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let pid = match pid_str.parse::<u32>() {
            Ok(value) => value,
            Err(_) => continue,
        };
        let parent_pid = parent_pid_str.parse::<u32>().ok();
        let lower = cmdline.to_lowercase();
        let dir = extract_user_data_dir_from_command_line(cmdline);
        if !lower.is_empty()
            && (is_helper_command_line(&lower) || lower.contains("crashpad_handler"))
        {
            if let (Some(parent_pid), Some(dir)) = (parent_pid, dir) {
                child_user_data_by_parent.entry(parent_pid).or_insert(dir);
            }
            continue;
        }
        main_entries.push((pid, dir));
    }

    for (pid, dir) in main_entries {
        let resolved_dir = dir.or_else(|| child_user_data_by_parent.get(&pid).cloned());
        entries.push((pid, resolved_dir));
    }

    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_codex_process_entries_from_sysinfo_fallback(
    expected_exe_path: &str,
) -> Vec<(u32, Option<String>)> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let current_pid = std::process::id();
    let mut main_entries: Vec<(u32, Option<String>)> = Vec::new();
    let mut child_user_data_by_parent: HashMap<u32, String> = HashMap::new();
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
        if name != "codex.exe"
            && name != "chatgpt.exe"
            && !exe_path.ends_with("\\codex.exe")
            && !exe_path.ends_with("\\chatgpt.exe")
        {
            continue;
        }
        let (resolved_exe, _) = resolve_windows_process_exe_for_match(process);
        if resolved_exe.as_deref() != Some(expected.as_str()) {
            continue;
        }

        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
            .join(" ");
        let dir = extract_user_data_dir(process.cmd());
        if !args_line.is_empty()
            && (is_helper_command_line(&args_line) || args_line.contains("crashpad_handler"))
        {
            if let (Some(parent_pid), Some(dir)) = (process.parent(), dir) {
                child_user_data_by_parent
                    .entry(parent_pid.as_u32())
                    .or_insert(dir);
            }
            continue;
        }

        main_entries.push((pid_u32, dir));
    }

    for (pid, dir) in main_entries {
        let resolved_dir = dir.or_else(|| child_user_data_by_parent.get(&pid).cloned());
        entries.push((pid, resolved_dir));
    }
    entries.sort_by_key(|(pid, _)| *pid);
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

#[cfg(target_os = "windows")]
fn collect_codex_main_process_pids_from_sysinfo_fast(expected_exe_path: &str) -> Vec<u32> {
    let expected = normalize_path_for_compare(expected_exe_path);
    if expected.is_empty() {
        return Vec::new();
    }

    let mut pids = Vec::new();
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
        if name != "codex.exe" && name != "chatgpt.exe" {
            continue;
        }
        let (resolved_exe, _) = resolve_windows_process_exe_for_match(process);
        if resolved_exe.as_deref() != Some(expected.as_str()) {
            continue;
        }

        let args_line = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().to_ascii_lowercase())
            .collect::<Vec<String>>()
            .join(" ");
        if !args_line.is_empty()
            && (is_helper_command_line(&args_line)
                || args_line.contains("crashpad_handler")
                || is_codex_windows_resource_process_command_line(&args_line))
        {
            continue;
        }

        pids.push(pid_u32);
    }
    pids.sort();
    pids.dedup();
    pids
}

#[cfg(target_os = "windows")]
fn pick_started_codex_pid(pids: Vec<u32>, before_pids: &HashSet<u32>) -> Option<u32> {
    let new_pids: Vec<u32> = pids
        .iter()
        .copied()
        .filter(|pid| !before_pids.contains(pid))
        .collect();
    if let Some(pid) = pick_preferred_pid(new_pids) {
        return Some(pid);
    }
    if before_pids.is_empty() {
        return pick_preferred_pid(pids);
    }
    None
}

#[cfg(target_os = "windows")]
fn wait_for_codex_default_start_pid_fast(
    expected_exe_path: &str,
    before_pids: &HashSet<u32>,
    timeout: Duration,
) -> Option<u32> {
    let started = Instant::now();
    let mut last_full_probe_at: Option<Instant> = None;
    while started.elapsed() < timeout {
        let fast_pids = collect_codex_main_process_pids_from_sysinfo_fast(expected_exe_path);
        if let Some(pid) = pick_started_codex_pid(fast_pids, before_pids) {
            crate::modules::logger::log_info(&format!(
                "[Codex Start] fast pid probe matched pid={}, elapsed_ms={}",
                pid,
                started.elapsed().as_millis()
            ));
            return Some(pid);
        }

        if started.elapsed() >= Duration::from_secs(2)
            && last_full_probe_at
                .map(|last| last.elapsed() >= Duration::from_secs(2))
                .unwrap_or(true)
        {
            last_full_probe_at = Some(Instant::now());
            let full_pids = collect_codex_process_entries()
                .into_iter()
                .map(|(pid, _)| pid)
                .collect::<Vec<u32>>();
            if let Some(pid) = pick_started_codex_pid(full_pids, before_pids) {
                crate::modules::logger::log_info(&format!(
                    "[Codex Start] fallback full pid probe matched pid={}, elapsed_ms={}",
                    pid,
                    started.elapsed().as_millis()
                ));
                return Some(pid);
            }
        }

        thread::sleep(Duration::from_millis(120));
    }
    None
}

#[cfg(target_os = "windows")]
pub fn collect_codex_process_entries() -> Vec<(u32, Option<String>)> {
    let launch_path = match resolve_codex_launch_path() {
        Ok(path) => path,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Probe] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return Vec::new();
        }
    };
    let expected = launch_path.to_string_lossy().to_string();
    let entries = collect_codex_process_entries_from_powershell(&expected);
    if !entries.is_empty() {
        return entries;
    }
    collect_codex_process_entries_from_sysinfo_fallback(&expected)
}

#[cfg(target_os = "linux")]
fn linux_process_env_value(pid: u32, key: &str) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{}/environ", pid)).ok()?;
    let prefix = format!("{}=", key);
    bytes.split(|byte| *byte == 0).find_map(|entry| {
        let entry = String::from_utf8_lossy(entry);
        entry
            .strip_prefix(&prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(target_os = "linux")]
pub fn collect_codex_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_codex_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always),
    );
    let current_pid = std::process::id();
    let mut entries = Vec::new();
    for (pid, process) in system.processes() {
        let pid = pid.as_u32();
        if pid == current_pid {
            continue;
        }
        let name = process.name().to_string_lossy().to_ascii_lowercase();
        let executable = process
            .exe()
            .map(|path| normalize_path_for_compare(&path.to_string_lossy()))
            .unwrap_or_default();
        if name != "chatgpt" && !executable.ends_with("/chatgpt") {
            continue;
        }
        let command_line = process
            .cmd()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let lower = command_line.to_ascii_lowercase();
        if is_helper_command_line(&lower) || lower.contains("crashpad_handler") {
            continue;
        }
        entries.push((pid, linux_process_env_value(pid, "CODEX_HOME")));
    }

    filter_entries_by_expected_launch_path("Codex", entries, expected_launch)
        .into_iter()
        .filter(|(pid, _)| is_pid_running(*pid))
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn collect_codex_process_entries() -> Vec<(u32, Option<String>)> {
    Vec::new()
}

/// 判断 Codex 是否在运行（仅 macOS）
#[cfg(target_os = "macos")]
pub fn is_codex_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        !collect_codex_process_entries().is_empty()
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// 启动 Codex 桌面实例（支持独立 CODEX_HOME、Electron user-data 与附加参数）。
pub fn start_codex_with_args(codex_home: &str, extra_args: &[String]) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_codex_launch_path()
            .ok()
            .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
            .or_else(|| resolve_macos_app_root_from_config("codex"));
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codex"))?;

        let codex_home_trimmed = codex_home.trim();
        let args = build_codex_app_launch_args(extra_args);

        // 通过 LaunchServices 启动 GUI 应用，避免直接执行 ChatGPT 主程序时
        // 被 macOS 以 Cockpit Tools 为 responsible process，导致偶发长时间停在
        // dyld/AppKit 初始化阶段。当前 macOS 的 `open` 支持 --env，因此
        // CODEX_HOME 与独立 Electron user-data-dir 都可以随启动请求传入。
        if !codex_home_trimmed.is_empty() {
            if resolve_codex_launch_path().is_ok() {
                let app_user_data_dir =
                    crate::modules::codex_instance::get_macos_app_user_data_dir(Path::new(
                        codex_home_trimmed,
                    ))?;
                std::fs::create_dir_all(&app_user_data_dir).map_err(|e| {
                    format!(
                        "创建 Codex macOS 实例运行目录失败 ({}): {}",
                        app_user_data_dir.to_string_lossy(),
                        e
                    )
                })?;

                let app_user_data_dir_string = app_user_data_dir.to_string_lossy().to_string();
                let mut launch_args = args.clone();
                launch_args.push(format!("--user-data-dir={}", app_user_data_dir_string));
                let open_pid = spawn_open_app_with_options_and_env(
                    &app_root,
                    &launch_args,
                    true,
                    &[
                        ("CODEX_HOME", codex_home_trimmed),
                        ("CODEX_ELECTRON_USER_DATA_PATH", &app_user_data_dir_string),
                    ],
                )
                .map_err(|e| format!("启动 Codex 失败: {}", e))?;
                crate::modules::logger::log_info(&format!(
                    "[Codex Start] macOS managed instance using open -n -a with --env and --user-data-dir; launcher_pid={} codex_home={} electron_user_data={} app_root={}",
                    open_pid,
                    summarize_text_for_process_log(codex_home_trimmed, 96),
                    app_user_data_dir_string,
                    app_root
                ));
                // 轮询获取真实 PID
                let probe_started = Instant::now();
                let timeout = Duration::from_secs(6);
                while probe_started.elapsed() < timeout {
                    if let Some(resolved_pid) = resolve_codex_pid(None, Some(codex_home_trimmed)) {
                        return Ok(resolved_pid);
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                return Err(format!(
                    "Codex 实例启动超时，未找到真实主进程（open launcher pid={}）",
                    open_pid
                ));
            }
            return Err(app_path_missing_error("codex"));
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Codex 失败: {}", e))?;
        crate::modules::logger::log_info("Codex 启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codex_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Codex Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let codex_home_trimmed = codex_home.trim();
        if codex_home_trimmed.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }

        let launch_path = resolve_codex_launch_path()?;
        let app_user_data_dir = crate::modules::codex_instance::get_windows_app_user_data_dir(
            Path::new(codex_home_trimmed),
        )?;
        std::fs::create_dir_all(&app_user_data_dir).map_err(|e| {
            format!(
                "创建 Codex Windows 实例运行目录失败 ({}): {}",
                app_user_data_dir.to_string_lossy(),
                e
            )
        })?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        cmd.env("CODEX_HOME", codex_home_trimmed);
        cmd.env("CODEX_ELECTRON_USER_DATA_PATH", &app_user_data_dir);
        if should_detach_child() {
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        let args = build_codex_app_launch_args(extra_args);
        for arg in &args {
            cmd.arg(arg);
        }
        cmd.arg(format!(
            "--user-data-dir={}",
            app_user_data_dir.to_string_lossy()
        ));

        let child = match spawn_command_with_trace(&mut cmd) {
            Ok(child) => Some(child),
            Err(err) => {
                let launch_path_text = launch_path.to_string_lossy().to_ascii_lowercase();
                if err.kind() == std::io::ErrorKind::PermissionDenied
                    && launch_path_text.contains("\\windowsapps\\")
                {
                    let mut fallback_args = build_codex_app_launch_args(extra_args);
                    fallback_args.push(format!(
                        "--user-data-dir={}",
                        app_user_data_dir.to_string_lossy()
                    ));
                    match launch_codex_via_powershell_exec_path(
                        &launch_path,
                        codex_home_trimmed,
                        &app_user_data_dir,
                        &fallback_args,
                    ) {
                        Ok(()) => {
                            crate::modules::logger::log_warn(&format!(
                                "[Codex Start] WindowsApps direct launch denied, PowerShell exec fallback succeeded: launch_path={} error={}",
                                launch_path.to_string_lossy(),
                                err
                            ));
                        }
                        Err(ps_err) => {
                            crate::modules::logger::log_warn(&format!(
                                "[Codex Start] WindowsApps direct launch denied and PowerShell exec fallback failed; managed Store activation blocked to avoid losing CODEX_HOME: launch_path={} error={} powershell_error={}",
                                launch_path.to_string_lossy(),
                                err,
                                ps_err
                            ));
                            return Err(codex_managed_store_launch_unsafe_error(
                                &err.to_string(),
                                &ps_err,
                            ));
                        }
                    }
                    None
                } else {
                    return Err(format!("启动 Codex 失败: {}", err));
                }
            }
        };
        crate::modules::logger::log_info(&format!(
            "[Codex Start] Windows managed instance using --user-data-dir and CODEX_ELECTRON_USER_DATA_PATH; launch_path={} codex_home={} app_user_data_dir={} pid={}",
            launch_path.to_string_lossy(),
            summarize_text_for_process_log(codex_home_trimmed, 96),
            app_user_data_dir.to_string_lossy(),
            child.as_ref().map(|item| item.id().to_string()).unwrap_or_else(|| "powershell-exec".to_string())
        ));

        let probe_started = Instant::now();
        let timeout = Duration::from_secs(15);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codex_pid(None, Some(codex_home_trimmed)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(250));
        }
        if let Some(child) = child {
            crate::modules::logger::log_warn(&format!(
                "[Codex Start] Windows 实例启动后 15s 内未匹配到实例 PID，回退 spawn pid={}",
                child.id()
            ));
            Ok(child.id())
        } else {
            let error = codex_managed_store_launch_unsafe_error(
                "WindowsApps direct launch denied",
                "PowerShell exec returned success but no managed instance matched within 15s",
            );
            crate::modules::logger::log_warn(&format!(
                "[Codex Start] PowerShell exec did not produce a matching managed instance; default PID fallback blocked: codex_home={}",
                summarize_text_for_process_log(codex_home_trimmed, 96)
            ));
            Err(error)
        }
    }

    #[cfg(target_os = "linux")]
    {
        let codex_home_trimmed = codex_home.trim();
        if codex_home_trimmed.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_codex_launch_path()?;
        let app_user_data_dir = crate::modules::codex_instance::get_linux_app_user_data_dir(
            Path::new(codex_home_trimmed),
        )?;
        std::fs::create_dir_all(&app_user_data_dir).map_err(|error| {
            format!(
                "创建 Codex Linux 实例运行目录失败 ({}): {}",
                app_user_data_dir.display(),
                error
            )
        })?;

        let mut command = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut command);
        sanitize_linux_gui_launch_env(&mut command);
        command
            .env("CODEX_HOME", codex_home_trimmed)
            .env("CODEX_ELECTRON_USER_DATA_PATH", &app_user_data_dir)
            .arg(format!("--user-data-dir={}", app_user_data_dir.display()));
        for arg in build_codex_app_launch_args(extra_args) {
            command.arg(arg);
        }
        if should_detach_child() {
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        let child = spawn_detached_unix(&mut command)
            .map_err(|error| format!("启动 Codex 失败: {}", error))?;
        let spawned_pid = child.id();
        crate::modules::logger::log_info(&format!(
            "[Codex Start] Linux managed desktop instance launched: launch_path={} codex_home={} electron_user_data={} pid={}",
            launch_path.display(),
            summarize_text_for_process_log(codex_home_trimmed, 96),
            app_user_data_dir.display(),
            spawned_pid
        ));

        let probe_started = Instant::now();
        while probe_started.elapsed() < Duration::from_secs(10) {
            if let Some(pid) = resolve_codex_pid(None, Some(codex_home_trimmed)) {
                return Ok(pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        if is_pid_running(spawned_pid) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Start] Linux 实例启动后未读取到 CODEX_HOME，回退 spawn pid={}",
                spawned_pid
            ));
            return Ok(spawned_pid);
        }
        return Err("Codex Linux 实例启动超时，未找到真实主进程".to_string());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (codex_home, extra_args);
        Err("当前系统不支持 Codex 应用多开".to_string())
    }
}
