// Process 模块：Codex process discovery, startup and port-process management。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
/// 启动 Codex 默认桌面实例（不注入 CODEX_HOME，支持附加参数）。
pub fn start_codex_default(extra_args: &[String]) -> Result<u32, String> {
    start_codex_default_internal(extra_args, false)
}

pub fn start_codex_default_fast_after_close(extra_args: &[String]) -> Result<u32, String> {
    start_codex_default_internal(extra_args, true)
}

fn build_codex_app_launch_args(extra_args: &[String]) -> Vec<String> {
    extra_args
        .iter()
        .map(|arg| arg.trim())
        .filter(|arg| !arg.is_empty())
        .map(str::to_string)
        .collect()
}

fn build_codex_default_launch_args(extra_args: &[String]) -> Vec<String> {
    build_codex_app_launch_args(extra_args)
}

fn start_codex_default_internal(
    extra_args: &[String],
    fast_after_close: bool,
) -> Result<u32, String> {
    #[cfg(not(target_os = "windows"))]
    let _ = fast_after_close;

    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_codex_launch_path()
            .ok()
            .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
            .or_else(|| resolve_macos_app_root_from_config("codex"));
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codex"))?;

        let args = build_codex_default_launch_args(extra_args);

        // 使用 open -n -a 启动默认实例，避免复用已运行的其他 Codex 实例。
        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Codex 失败: {}", e))?;
        crate::modules::logger::log_info("Codex 默认实例启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codex_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Codex Start] 启动后 6s 内未匹配到默认实例真实 PID，open launcher pid={} 不会写入实例状态",
            open_pid
        ));
        return Err(format!(
            "Codex 默认实例启动超时，未找到真实主进程（open launcher pid={}）",
            open_pid
        ));
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path_for_probe = resolve_codex_launch_path().ok();
        let before_probe_started = Instant::now();
        let before_pids: HashSet<u32> = if fast_after_close {
            launch_path_for_probe
                .as_ref()
                .map(|path| {
                    collect_codex_main_process_pids_from_sysinfo_fast(
                        path.to_string_lossy().as_ref(),
                    )
                    .into_iter()
                    .collect()
                })
                .unwrap_or_default()
        } else {
            collect_codex_process_entries()
                .into_iter()
                .map(|(pid, _)| pid)
                .collect()
        };
        crate::modules::logger::log_info(&format!(
            "[Codex Start] before pid probe mode={}, count={}, elapsed_ms={}",
            if fast_after_close { "fast" } else { "full" },
            before_pids.len(),
            before_probe_started.elapsed().as_millis()
        ));

        let app_user_model_id = detect_codex_store_app_user_model_id();
        if let Some(app_user_model_id) = app_user_model_id {
            crate::modules::logger::log_info(&format!(
                "[Codex Start] 启动策略候选=system-store-entry app_id={}",
                app_user_model_id
            ));
            let args = build_codex_default_launch_args(extra_args);
            match launch_codex_via_store_app_user_model_id(&app_user_model_id, None, None, &args) {
                Ok(()) => {
                    crate::modules::logger::log_info(&format!(
                        "[Codex Start] 已通过系统入口启动 Codex: {}",
                        app_user_model_id
                    ));
                    let timeout = Duration::from_secs(15);
                    if fast_after_close {
                        if let Some(launch_path) = launch_path_for_probe.as_ref() {
                            if let Some(pid) = wait_for_codex_default_start_pid_fast(
                                launch_path.to_string_lossy().as_ref(),
                                &before_pids,
                                timeout,
                            ) {
                                crate::modules::logger::log_info(&format!(
                                    "[Codex Start] fast store-entry pid matched app_id={} pid={}",
                                    app_user_model_id, pid
                                ));
                                return Ok(pid);
                            }
                        } else {
                            crate::modules::logger::log_warn(
                                "[Codex Start] fast pid probe skipped because launch path is unavailable",
                            );
                        }
                    } else {
                        let probe_started = Instant::now();
                        while probe_started.elapsed() < timeout {
                            let entries = collect_codex_process_entries();
                            let mut new_pids: Vec<u32> = entries
                                .iter()
                                .map(|(pid, _)| *pid)
                                .filter(|pid| !before_pids.contains(pid))
                                .collect();
                            if let Some(pid) = pick_preferred_pid(new_pids.clone()) {
                                crate::modules::logger::log_info(&format!(
                                    "[Codex Start] 启动策略=system-store-entry app_id={} pid={}",
                                    app_user_model_id, pid
                                ));
                                return Ok(pid);
                            }
                            if before_pids.is_empty() {
                                new_pids = entries.iter().map(|(pid, _)| *pid).collect();
                                if let Some(pid) = pick_preferred_pid(new_pids) {
                                    crate::modules::logger::log_info(&format!(
                                        "[Codex Start] 启动策略=system-store-entry app_id={} pid={}",
                                        app_user_model_id, pid
                                    ));
                                    return Ok(pid);
                                }
                            }
                            thread::sleep(Duration::from_millis(250));
                        }
                        if before_pids.is_empty() {
                            if let Some(pid) = resolve_codex_pid(None, None) {
                                crate::modules::logger::log_info(&format!(
                                    "[Codex Start] 启动策略=system-store-entry app_id={} pid={}",
                                    app_user_model_id, pid
                                ));
                                return Ok(pid);
                            }
                        } else {
                            crate::modules::logger::log_warn(&format!(
                                "[Codex Start] system-store-entry only reused existing instance, before_pids={}",
                                summarize_pid_list_for_log(
                                    &before_pids.iter().copied().collect::<Vec<u32>>()
                                )
                            ));
                        }
                    }
                    crate::modules::logger::log_warn(
                        "[Codex Start] 系统入口已调用，但 15s 内未探测到 Codex 主进程，准备回退可执行路径",
                    );
                }
                Err(err) => {
                    crate::modules::logger::log_warn(&format!(
                        "[Codex Start] 系统入口启动失败，准备回退可执行路径: {}",
                        err
                    ));
                }
            }
        } else {
            crate::modules::logger::log_warn(
                "[Codex Start] 未探测到 Codex AppUserModelId，准备回退可执行路径",
            );
        }

        let launch_path = resolve_codex_launch_path()?;
        crate::modules::logger::log_info(&format!(
            "[Codex Start] 启动策略=exe-path launch_path={}",
            launch_path.to_string_lossy()
        ));
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        // Codex 是 GUI 应用，不设置 CREATE_NO_WINDOW，否则会导致其内部 spawn CLI 子进程失败。
        let args = build_codex_default_launch_args(extra_args);
        for arg in args {
            cmd.arg(arg);
        }

        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Codex 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "[Codex Start] 启动策略=exe-path launch_path={} pid={}",
            launch_path.to_string_lossy(),
            child.id()
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_codex_launch_path()?;
        let mut command = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut command);
        sanitize_linux_gui_launch_env(&mut command);
        for arg in build_codex_default_launch_args(extra_args) {
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
        let probe_started = Instant::now();
        while probe_started.elapsed() < Duration::from_secs(10) {
            if let Some(pid) = resolve_codex_pid(None, None) {
                return Ok(pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        if is_pid_running(spawned_pid) {
            return Ok(spawned_pid);
        }
        return Err("Codex Linux 默认实例启动超时，未找到真实主进程".to_string());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = extra_args;
        Err("当前系统不支持 Codex 桌面应用启动".to_string())
    }
}

/// 关闭 Codex 默认实例。默认实例没有 CODEX_HOME 环境变量时按默认 ~/.codex 处理。
pub fn close_codex_default_fast_by_pid(
    last_pid: Option<u32>,
    timeout_secs: u64,
) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        let Some(pid) = last_pid.filter(|pid| *pid != 0 && is_pid_running(*pid)) else {
            return Ok(false);
        };
        let launch_path = match resolve_codex_launch_path() {
            Ok(path) => path,
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] fast default close skipped, launch path unavailable: {}",
                    err
                ));
                return Ok(false);
            }
        };
        let fast_pids = collect_codex_main_process_pids_from_sysinfo_fast(
            launch_path.to_string_lossy().as_ref(),
        );
        if !fast_pids.contains(&pid) {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] fast default close skipped, last_pid={} not in fast matches={}",
                pid,
                summarize_pid_list_for_log(&fast_pids)
            ));
            return Ok(false);
        }

        crate::modules::logger::log_info(&format!(
            "[Codex Close] fast default close by last_pid={}",
            pid
        ));
        if request_codex_graceful_close(pid) {
            let graceful_wait_secs = timeout_secs.min(2).max(1);
            if wait_pids_exit(&[pid], graceful_wait_secs) {
                crate::modules::logger::log_info(&format!(
                    "[Codex Close] fast graceful close finished, pid={}",
                    pid
                ));
                return Ok(true);
            }
        } else {
            crate::modules::logger::log_warn(
                "[Codex Close] fast graceful taskkill failed, force close last_pid directly",
            );
        }

        let remaining = collect_running_pids(&[pid]);
        if !remaining.is_empty() {
            close_pids(&remaining, timeout_secs)?;
        }
        if is_pid_running(pid) {
            return Err(
                "failed to close managed Codex instance process; please close it manually and retry"
                    .to_string(),
            );
        }
        Ok(true)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (last_pid, timeout_secs);
        Ok(false)
    }
}

pub fn close_codex_default(timeout_secs: u64) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let default_home = crate::modules::codex_account::get_codex_home()
            .to_string_lossy()
            .to_string();
        return close_codex_instances(&[default_home], timeout_secs);
    }

    #[cfg(target_os = "windows")]
    {
        let default_home = crate::modules::codex_account::get_codex_home()
            .to_string_lossy()
            .to_string();
        return close_codex_instances(&[default_home], timeout_secs);
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = timeout_secs;
        Err("当前系统不支持 Codex 桌面应用关闭".to_string())
    }
}

#[cfg(target_os = "macos")]
fn request_codex_graceful_close(pid: u32) -> bool {
    if pid == 0 || !is_pid_running(pid) {
        return true;
    }

    let focus_script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true",
        pid
    );
    crate::modules::logger::log_info(&format!(
        "[Codex Close] graceful osascript start pid={}",
        pid
    ));
    match Command::new("osascript")
        .args([
            "-e",
            &focus_script,
            "-e",
            "tell application \"System Events\" to keystroke \"q\" using command down",
        ])
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                crate::modules::logger::log_info(&format!(
                    "[Codex Close] graceful osascript success pid={}",
                    pid
                ));
                true
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] graceful osascript failed pid={} err={}",
                    pid,
                    stderr.trim()
                ));
                false
            }
        }
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] graceful osascript error pid={} err={}",
                pid, err
            ));
            false
        }
    }
}

/// Request a normal Windows app shutdown before falling back to force close.
#[cfg(target_os = "windows")]
fn request_codex_graceful_close(pid: u32) -> bool {
    if pid == 0 || !is_pid_running(pid) {
        return true;
    }

    use std::os::windows::process::CommandExt;

    crate::modules::logger::log_info(&format!(
        "[Codex Close] graceful taskkill start pid={}",
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
                    "[Codex Close] graceful taskkill success pid={} status={}",
                    pid, value.status
                ));
                true
            } else {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] graceful taskkill failed pid={} status={}",
                    pid, value.status
                ));
                false
            }
        }
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] graceful taskkill error pid={} err={}",
                pid, err
            ));
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn request_codex_graceful_close(pid: u32) -> bool {
    if pid == 0 || !is_pid_running(pid) {
        return true;
    }
    match Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] Linux SIGTERM failed pid={} status={} stderr={}",
                pid,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            false
        }
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] Linux SIGTERM error pid={} error={}",
                pid, error
            ));
            false
        }
    }
}

/// 重启 Codex 默认实例，让官方 App 重新读取磁盘上的全局状态。
pub fn restart_codex_default(extra_args: &[String], timeout_secs: u64) -> Result<u32, String> {
    ensure_codex_launch_path_configured()?;
    close_codex_default(timeout_secs)?;
    start_codex_default(extra_args)
}

/// 关闭受管 Codex 实例（按 CODEX_HOME 匹配，包含默认实例目录）
pub fn close_codex_instances(codex_homes: &[String], timeout_secs: u64) -> Result<(), String> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        crate::modules::logger::log_info("正在关闭受管 Codex 实例...");

        let target_homes: HashSet<String> = codex_homes
            .iter()
            .map(|value| normalize_path_for_compare(value))
            .filter(|value| !value.is_empty())
            .collect();
        if target_homes.is_empty() {
            crate::modules::logger::log_info("未提供可关闭的 Codex 实例目录");
            return Ok(());
        }

        let default_home = normalize_path_for_compare(
            &crate::modules::codex_account::get_codex_home()
                .to_string_lossy()
                .to_string(),
        );
        let entries = collect_codex_process_entries();
        let mut pids: Vec<u32> = entries
            .iter()
            .filter_map(|(pid, home)| {
                let resolved_home = home
                    .as_ref()
                    .map(|value| normalize_path_for_compare(value))
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| default_home.clone());
                if !resolved_home.is_empty() && target_homes.contains(&resolved_home) {
                    Some(*pid)
                } else {
                    None
                }
            })
            .collect();
        pids.sort();
        pids.dedup();
        if pids.is_empty() {
            crate::modules::logger::log_info("受管 Codex 实例未在运行，无需关闭");
            return Ok(());
        }
        // Capture direct stdio app-server descendants before Electron exits. Once the main
        // process is gone they can be re-parented, and an in-flight OAuth refresh could otherwise
        // write the old token tuple after the account switch commits the new credentials.
        let direct_app_server_pids = collect_codex_direct_app_server_pids_for_roots(&pids);
        if !direct_app_server_pids.is_empty() {
            crate::modules::logger::log_info(&format!(
                "[Codex Close] captured direct app-server pids={}",
                summarize_pid_list_for_log(&direct_app_server_pids)
            ));
        }

        crate::modules::logger::log_info(&format!(
            "准备关闭 {} 个受管 Codex 主进程...",
            pids.len()
        ));
        let graceful_pids: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|pid| request_codex_graceful_close(*pid))
            .collect();
        if !graceful_pids.is_empty() {
            let graceful_wait_secs = timeout_secs.min(2).max(1);
            if wait_pids_exit(&graceful_pids, graceful_wait_secs) {
                let remaining = collect_running_pids(&pids);
                if remaining.is_empty() {
                    close_captured_codex_direct_app_servers(&direct_app_server_pids, timeout_secs)?;
                    crate::modules::logger::log_info(&format!(
                        "[Codex Close] graceful close finished, targets={}",
                        summarize_pid_list_for_log(&pids)
                    ));
                    return Ok(());
                }
            }
        } else {
            crate::modules::logger::log_warn(
                "[Codex Close] graceful close request failed for all targets, skip grace wait",
            );
        }
        let remaining = collect_running_pids(&pids);
        if !remaining.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] graceful close incomplete, fallback close_pids for remaining={}",
                summarize_pid_list_for_log(&remaining)
            ));
            if let Err(err) = close_pids(&remaining, timeout_secs) {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] fallback close_pids failed: {}",
                    err
                ));
            }
            let after_force_remaining = collect_running_pids(&remaining);
            if !after_force_remaining.is_empty() {
                return Err(
                    "failed to close managed Codex instance process; please close it manually and retry"
                        .to_string(),
                );
            }
        }
        close_captured_codex_direct_app_servers(&direct_app_server_pids, timeout_secs)?;

        let still_running = !collect_running_pids(&pids).is_empty();
        if still_running {
            return Err("无法关闭受管 Codex 实例进程，请手动关闭后重试".to_string());
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        crate::modules::logger::log_info("正在关闭受管 Codex 实例...");

        let default_home = normalize_path_for_compare(
            &crate::modules::codex_account::get_codex_home()
                .to_string_lossy()
                .to_string(),
        );
        let mut target_app_dirs: HashSet<String> = HashSet::new();
        let mut includes_default = false;

        for home in codex_homes {
            let normalized_home = normalize_path_for_compare(home);
            if normalized_home.is_empty() {
                continue;
            }
            if normalized_home == default_home {
                includes_default = true;
                continue;
            }
            if let Some(app_dir) = get_managed_codex_windows_app_user_data_dir(home) {
                let normalized_app_dir = normalize_path_for_compare(&app_dir);
                if !normalized_app_dir.is_empty() {
                    target_app_dirs.insert(normalized_app_dir);
                }
            }
        }

        if target_app_dirs.is_empty() && !includes_default {
            crate::modules::logger::log_info("未提供可关闭的 Codex 实例目录");
            return Ok(());
        }

        let current_default_app_dirs = if includes_default {
            get_default_codex_windows_app_user_data_dirs(
                crate::modules::codex_account::get_codex_home()
                    .to_string_lossy()
                    .as_ref(),
            )
        } else {
            HashSet::new()
        };

        let matches_target = |dir: Option<&String>,
                              target_app_dirs: &HashSet<String>,
                              includes_default: bool| {
            match dir {
                Some(value) => {
                    let normalized = normalize_path_for_compare(value);
                    !normalized.is_empty()
                        && (target_app_dirs.contains(&normalized)
                            || (includes_default && current_default_app_dirs.contains(&normalized)))
                }
                None => includes_default,
            }
        };

        let entries = collect_codex_process_entries();
        let mut pids: Vec<u32> = entries
            .iter()
            .filter_map(|(pid, dir)| {
                matches_target(dir.as_ref(), &target_app_dirs, includes_default).then_some(*pid)
            })
            .collect();
        pids.sort();
        pids.dedup();
        if pids.is_empty() {
            crate::modules::logger::log_info("受管 Codex 实例未在运行，无需关闭");
            return Ok(());
        }

        crate::modules::logger::log_info(&format!(
            "准备关闭 {} 个受管 Codex 主进程...",
            pids.len()
        ));
        let graceful_pids: Vec<u32> = pids
            .iter()
            .copied()
            .filter(|pid| request_codex_graceful_close(*pid))
            .collect();
        if graceful_pids.is_empty() {
            crate::modules::logger::log_warn(
                "[Codex Close] graceful taskkill failed for all targets, skip grace wait and force close directly",
            );
        } else {
            let graceful_wait_secs = timeout_secs.min(8).max(1);
            if wait_pids_exit(&graceful_pids, graceful_wait_secs) {
                let remaining = collect_running_pids(&pids);
                if remaining.is_empty() {
                    crate::modules::logger::log_info(&format!(
                        "[Codex Close] graceful close finished, targets={}",
                        summarize_pid_list_for_log(&pids)
                    ));
                    return Ok(());
                }
            }
        }
        let remaining = collect_running_pids(&pids);
        if !remaining.is_empty() {
            crate::modules::logger::log_warn(&format!(
                "[Codex Close] graceful close incomplete, retry force close for remaining pids={}",
                summarize_pid_list_for_log(&remaining)
            ));
            let force_close_error = close_pids(&remaining, timeout_secs).err();
            if let Some(err) = force_close_error.as_deref() {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] force close_pids failed: {}",
                    err
                ));
            }
            let after_force_remaining = collect_running_pids(&remaining);
            if !after_force_remaining.is_empty() {
                return Err(force_close_error.unwrap_or_else(|| {
                    crate::modules::windows_operation::format_error(
                        "stop_process",
                        "无法关闭受管 Codex 实例进程",
                        "目标进程在等待超时后仍在运行",
                        None,
                        &after_force_remaining,
                        true,
                        true,
                        true,
                    )
                }));
            }
        }
        if includes_default
            && std::env::var("COCKPIT_CODEX_CLOSE_RESOURCE_CLEANUP")
                .ok()
                .as_deref()
                == Some("1")
        {
            let resource_pids = collect_codex_windows_resource_process_pids();
            if !resource_pids.is_empty() {
                crate::modules::logger::log_info(&format!(
                    "[Codex Close] closing bundled resource codex processes for default instance: {}",
                    summarize_pid_list_for_log(&resource_pids)
                ));
                let _ = close_pids(&resource_pids, timeout_secs.min(5).max(1));
            }
        }

        let still_running = !collect_running_pids(&pids).is_empty();
        if still_running {
            let remaining = collect_running_pids(&pids);
            return Err(crate::modules::windows_operation::format_error(
                "stop_process",
                "无法关闭受管 Codex 实例进程",
                "目标进程在等待超时后仍在运行",
                None,
                &remaining,
                true,
                true,
                true,
            ));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (codex_homes, timeout_secs);
        Err("当前系统不支持 Codex 应用多开".to_string())
    }
}

fn get_trae_pids() -> Vec<u32> {
    let mut pids = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let app_lower = TRAE_APP_NAME.to_lowercase();
        let bundle_pattern = format!("{}.app/contents/", app_lower);
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
                let lower = cmdline.to_lowercase();
                if lower.contains(&bundle_pattern)
                    && !lower.contains("--type=")
                    && !lower.contains("crashpad_handler")
                {
                    pids.push(pid);
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
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_lowercase();

            let args = process.cmd();
            let args_str = args
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            let is_helper = args_str.contains("--type=")
                || name.contains("helper")
                || name.contains("plugin")
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox")
                || exe_path.contains("crashpad");

            #[cfg(target_os = "windows")]
            {
                if (name.contains("trae") || exe_path.contains("trae")) && !is_helper {
                    pids.push(pid_u32);
                }
            }

            #[cfg(target_os = "linux")]
            {
                if (name.contains("trae") || exe_path.contains("/trae")) && !is_helper {
                    pids.push(pid_u32);
                }
            }
        }
    }

    if !pids.is_empty() {
        crate::modules::logger::log_info(&format!(
            "找到 {} 个 Trae 进程: {}",
            pids.len(),
            summarize_pid_list_for_log(&pids)
        ));
    }

    pids
}

pub fn is_trae_running() -> bool {
    !get_trae_pids().is_empty()
}

pub fn is_trae_running_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> bool {
    !collect_trae_process_entries_for_platform(platform).is_empty()
}

pub fn close_trae(timeout_secs: u64) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let _ = timeout_secs;

    crate::modules::logger::log_info("正在关闭 Trae...");
    let pids = get_trae_pids();
    if pids.is_empty() {
        crate::modules::logger::log_info("Trae 未在运行，无需关闭");
        return Ok(());
    }

    crate::modules::logger::log_info(&format!("准备关闭 {} 个 Trae 进程...", pids.len()));
    let _ = close_pids(&pids, timeout_secs);

    if !get_trae_pids().is_empty() {
        return Err("无法关闭 Trae 进程，请手动关闭后重试".to_string());
    }

    crate::modules::logger::log_info("Trae 已成功关闭");
    Ok(())
}

/// 检查 OpenCode（桌面端）是否在运行
pub fn is_opencode_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let app_lower = OPENCODE_APP_NAME.to_lowercase();
        let bundle_pattern = format!("{}.app/contents/", app_lower);
        if let Ok(output) = Command::new("ps")
            .args(["-axww", "-o", "command="])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let lower = line.trim().to_lowercase();
                if lower.contains(&bundle_pattern)
                    && !lower.contains("--type=")
                    && !lower.contains("crashpad_handler")
                {
                    return true;
                }
            }
        }
        return false;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );

        let current_pid = std::process::id();
        #[cfg(target_os = "windows")]
        let app_lower = OPENCODE_APP_NAME.to_lowercase();

        for (pid, process) in system.processes() {
            let pid_u32 = pid.as_u32();
            if pid_u32 == current_pid {
                continue;
            }

            let name = process.name().to_string_lossy().to_lowercase();
            let exe_path = process
                .exe()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_lowercase();

            let args = process.cmd();
            let args_str = args
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            let is_helper = args_str.contains("--type=")
                || name.contains("helper")
                || name.contains("plugin")
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox")
                || exe_path.contains("crashpad");

            #[cfg(target_os = "windows")]
            {
                if (name == "opencode.exe"
                    || name == "opencode"
                    || name == app_lower
                    || exe_path.contains("opencode"))
                    && !is_helper
                {
                    return true;
                }
            }

            #[cfg(target_os = "linux")]
            {
                if (name.contains("opencode") || exe_path.contains("/opencode")) && !is_helper {
                    return true;
                }
            }
        }

        false
    }
}

fn get_opencode_pids() -> Vec<u32> {
    let mut pids = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let app_lower = OPENCODE_APP_NAME.to_lowercase();
        let bundle_pattern = format!("{}.app/contents/", app_lower);
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
                let lower = cmdline.to_lowercase();
                if lower.contains(&bundle_pattern)
                    && !lower.contains("--type=")
                    && !lower.contains("crashpad_handler")
                {
                    pids.push(pid);
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
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_lowercase();

            let args = process.cmd();
            let args_str = args
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            let is_helper = args_str.contains("--type=")
                || name.contains("helper")
                || name.contains("plugin")
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox")
                || exe_path.contains("crashpad");

            #[cfg(target_os = "windows")]
            {
                if (name.contains("opencode") || exe_path.contains("opencode")) && !is_helper {
                    pids.push(pid_u32);
                }
            }

            #[cfg(target_os = "linux")]
            {
                if (name.contains("opencode") || exe_path.contains("/opencode")) && !is_helper {
                    pids.push(pid_u32);
                }
            }
        }
    }

    if !pids.is_empty() {
        crate::modules::logger::log_info(&format!(
            "找到 {} 个 OpenCode 进程: {}",
            pids.len(),
            summarize_pid_list_for_log(&pids)
        ));
    }

    pids
}

/// 关闭 OpenCode（桌面端）
pub fn close_opencode(timeout_secs: u64) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let _ = timeout_secs;

    crate::modules::logger::log_info("正在关闭 OpenCode...");
    let pids = get_opencode_pids();
    if pids.is_empty() {
        crate::modules::logger::log_info("OpenCode 未在运行，无需关闭");
        return Ok(());
    }

    crate::modules::logger::log_info(&format!("准备关闭 {} 个 OpenCode 进程...", pids.len()));
    let _ = close_pids(&pids, timeout_secs);

    if is_opencode_running() {
        return Err("无法关闭 OpenCode 进程，请手动关闭后重试".to_string());
    }

    crate::modules::logger::log_info("OpenCode 已成功关闭");
    Ok(())
}

/// 启动 OpenCode（桌面端）
pub fn start_opencode_with_path(custom_path: Option<&str>) -> Result<(), String> {
    crate::modules::logger::log_info("正在启动 OpenCode...");

    #[cfg(target_os = "macos")]
    {
        let target =
            normalize_custom_path(custom_path).unwrap_or_else(|| OPENCODE_APP_NAME.to_string());

        let mut cmd = Command::new("open");
        sanitize_macos_gui_launch_env(&mut cmd);
        append_managed_proxy_env_to_open_args(&mut cmd);
        cmd.args(["-a", &target]);

        let output = cmd
            .output()
            .map_err(|e| format!("启动 OpenCode 失败: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Unable to find application") {
                return Err("未找到 OpenCode 应用，请在设置中配置启动路径".to_string());
            }
            return Err(format!("启动 OpenCode 失败: {}", stderr));
        }
        crate::modules::logger::log_info(&format!("OpenCode 启动命令已发送: {}", target));
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut candidates = Vec::new();
        if let Some(custom) = normalize_custom_path(custom_path) {
            candidates.push(custom);
        }

        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(format!("{}/Programs/OpenCode/OpenCode.exe", local_appdata));
        }

        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(format!("{}/OpenCode/OpenCode.exe", program_files));
        }

        for candidate in candidates {
            if candidate.contains('/') || candidate.contains('\\') {
                if !std::path::Path::new(&candidate).exists() {
                    continue;
                }
            }
            let mut cmd = Command::new(&candidate);
            apply_managed_proxy_env_to_command(&mut cmd);
            cmd.creation_flags(0x08000000);
            if spawn_command_with_trace(&mut cmd).is_ok() {
                crate::modules::logger::log_info(&format!("OpenCode 已启动: {}", candidate));
                return Ok(());
            }
        }

        return Err("未找到 OpenCode 可执行文件，请在设置中配置启动路径".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let mut candidates = Vec::new();
        if let Some(custom) = normalize_custom_path(custom_path) {
            candidates.push(custom);
        }

        candidates.push("/usr/bin/opencode".to_string());
        candidates.push("/opt/opencode/opencode".to_string());
        candidates.push("opencode".to_string());

        for candidate in candidates {
            if candidate.contains('/') {
                if !std::path::Path::new(&candidate).exists() {
                    continue;
                }
            }
            let mut cmd = Command::new(&candidate);
            apply_managed_proxy_env_to_command(&mut cmd);
            if spawn_command_with_trace(&mut cmd).is_ok() {
                crate::modules::logger::log_info(&format!("OpenCode 已启动: {}", candidate));
                return Ok(());
            }
        }

        return Err("未找到 OpenCode 可执行文件，请在设置中配置启动路径".to_string());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err("不支持的操作系统".to_string())
}

pub fn find_pids_by_port(port: u16) -> Result<Vec<u32>, String> {
    let current_pid = std::process::id();
    let mut pids = HashSet::new();

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let output = Command::new("lsof")
            .args(["-nP", &format!("-iTCP:{}", port), "-sTCP:LISTEN", "-t"])
            .output()
            .map_err(|e| format!("执行 lsof 失败: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                if pid != current_pid {
                    pids.insert(pid);
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let output = Command::new("netstat")
            .creation_flags(CREATE_NO_WINDOW)
            .args(["-ano", "-p", "tcp"])
            .output()
            .map_err(|e| format!("执行 netstat 失败: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let port_suffix = format!(":{}", port);
        for line in stdout.lines() {
            let line = line.trim();
            if !line.starts_with("TCP") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 {
                continue;
            }
            let local = parts[1];
            let state = parts[3];
            let pid_str = parts[4];
            if !state.eq_ignore_ascii_case("LISTENING") {
                continue;
            }
            if !local.ends_with(&port_suffix) {
                continue;
            }
            if let Ok(pid) = pid_str.parse::<u32>() {
                if pid != current_pid {
                    pids.insert(pid);
                }
            }
        }
    }

    Ok(pids.into_iter().collect())
}

pub fn is_port_in_use(port: u16) -> Result<bool, String> {
    Ok(!find_pids_by_port(port)?.is_empty())
}

pub fn kill_port_processes(port: u16) -> Result<usize, String> {
    let pids = find_pids_by_port(port)?;
    if pids.is_empty() {
        return Ok(0);
    }

    let mut cleaned = 0usize;
    let mut failed = Vec::new();

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        for pid in &pids {
            if *pid == 0 || !is_pid_running(*pid) {
                cleaned += 1;
                continue;
            }
            let output = Command::new("taskkill")
                .args(["/F", "/PID", &pid.to_string()])
                .creation_flags(0x08000000)
                .output();
            match output {
                Ok(out) if out.status.success() => cleaned += 1,
                Ok(out) => {
                    if !is_pid_running(*pid) {
                        cleaned += 1;
                    } else {
                        failed.push(format_kill_command_failure(
                            *pid,
                            "taskkill",
                            out.status,
                            &out.stderr,
                            &out.stdout,
                        ));
                    }
                }
                Err(e) => {
                    if !is_pid_running(*pid) {
                        cleaned += 1;
                    } else {
                        failed.push(format!("pid {}: taskkill failed: {}", pid, e));
                    }
                }
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        for pid in &pids {
            if *pid == 0 || !is_pid_running(*pid) {
                cleaned += 1;
                continue;
            }
            let output = Command::new("kill").args(["-9", &pid.to_string()]).output();
            match output {
                Ok(out) if out.status.success() => cleaned += 1,
                Ok(out) => {
                    if !is_pid_running(*pid) {
                        cleaned += 1;
                    } else {
                        failed.push(format_kill_command_failure(
                            *pid,
                            "kill",
                            out.status,
                            &out.stderr,
                            &out.stdout,
                        ));
                    }
                }
                Err(e) => {
                    if !is_pid_running(*pid) {
                        cleaned += 1;
                    } else {
                        failed.push(format!("pid {}: kill failed: {}", pid, e));
                    }
                }
            }
        }
    }

    if !failed.is_empty() {
        let original_reason = failed.join("; ");
        #[cfg(target_os = "windows")]
        {
            let remaining = collect_running_pids(&pids);
            return Err(crate::modules::windows_operation::format_error(
                "stop_process",
                "无法清理端口占用进程",
                &original_reason,
                None,
                &remaining,
                true,
                false,
                true,
            ));
        }
        #[cfg(not(target_os = "windows"))]
        return Err(format!("关闭进程失败: {}", original_reason));
    }

    Ok(cleaned)
}
