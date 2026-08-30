// cockpit-core Process：Editor/IDE startup wrappers and platform launch arguments。
// 通过 include! 保持原模块作用域和跨平台调用路径。
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
        return Err(format!("关闭进程失败: {}", failed.join("; ")));
    }

    Ok(cleaned)
}

fn utf8_command_output_snippet(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(summarize_text_for_process_log(text, 240))
    }
}

fn format_kill_command_failure(
    pid: u32,
    command: &str,
    status: ExitStatus,
    stderr: &[u8],
    stdout: &[u8],
) -> String {
    let detail =
        utf8_command_output_snippet(stderr).or_else(|| utf8_command_output_snippet(stdout));
    match detail {
        Some(detail) => format!(
            "pid {}: {} failed with status {}: {}",
            pid, command, status, detail
        ),
        None => format!("pid {}: {} failed with status {}", pid, command, status),
    }
}

pub fn start_vscode_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("vscode").or_else(|| {
            resolve_vscode_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("vscode"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_vscode_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[VSCode Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_vscode_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_vscode_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("GitHub Copilot 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_codebuddy_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("codebuddy").or_else(|| {
            resolve_codebuddy_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codebuddy"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codebuddy_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_codebuddy_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_codebuddy_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("CodeBuddy 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_codebuddy_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("codebuddy").or_else(|| {
            resolve_codebuddy_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codebuddy"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 默认实例启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codebuddy_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_codebuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_codebuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 CodeBuddy 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("CodeBuddy 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_codebuddy_cn_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("codebuddy_cn").or_else(|| {
            resolve_codebuddy_cn_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codebuddy_cn"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codebuddy_cn_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy CN Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_codebuddy_cn_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_codebuddy_cn_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("CodeBuddy CN 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_codebuddy_cn_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("codebuddy_cn").or_else(|| {
            resolve_codebuddy_cn_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codebuddy_cn"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 默认实例启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codebuddy_cn_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[CodeBuddy CN Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_codebuddy_cn_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_codebuddy_cn_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 CodeBuddy CN 失败: {}", e))?;
        crate::modules::logger::log_info("CodeBuddy CN 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("CodeBuddy CN 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_workbuddy_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let app_root = resolve_macos_app_root_from_config("workbuddy").or_else(|| {
            resolve_workbuddy_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("workbuddy"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_workbuddy_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_workbuddy_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_workbuddy_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("WorkBuddy 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_workbuddy_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_macos_app_root_from_config("workbuddy").or_else(|| {
            resolve_workbuddy_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("workbuddy"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 默认实例启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_workbuddy_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_workbuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child = spawn_command_with_trace(&mut cmd)
            .map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_workbuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 WorkBuddy 失败：{}", e))?;
        crate::modules::logger::log_info("WorkBuddy 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("WorkBuddy 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_qoder_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_qoder_launch_path()?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error("qoder"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_qoder_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Qoder Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_qoder_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_qoder_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("Qoder 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_qoder_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let launch_path = resolve_qoder_launch_path()?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error("qoder"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 默认实例启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_qoder_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Qoder Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_qoder_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_qoder_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Qoder 失败: {}", e))?;
        crate::modules::logger::log_info("Qoder 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("Qoder 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_trae_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path()?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error("trae"))?;

        let mut args: Vec<String> = Vec::new();
        args.push("--user-data-dir".to_string());
        args.push(target.to_string());
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_trae_pid(None, Some(target)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Trae Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path()?;

        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(target);
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (user_data_dir, extra_args, use_new_window);
        Err("Trae 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_trae_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let launch_path = resolve_trae_launch_path()?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error("trae"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 默认实例启动命令已发送（open -n -a）");
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_trae_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Trae Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_trae_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_trae_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("Trae 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_vscode_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        let app_root = resolve_macos_app_root_from_config("vscode").or_else(|| {
            resolve_vscode_launch_path()
                .ok()
                .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
        });
        let app_root = app_root.ok_or_else(|| app_path_missing_error("vscode"))?;

        let mut args: Vec<String> = Vec::new();
        if use_new_window {
            args.push("--new-window".to_string());
        } else {
            args.push("--reuse-window".to_string());
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 默认实例启动命令已发送（open -n -a）");
        // 轮询获取真实 PID
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_vscode_pid(None, None) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[VSCode Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_vscode_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_vscode_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        if use_new_window {
            cmd.arg("--new-window");
        } else {
            cmd.arg("--reuse-window");
        }
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }
        let child =
            spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 VS Code 失败: {}", e))?;
        crate::modules::logger::log_info("VS Code 默认实例启动命令已发送");
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("GitHub Copilot 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn close_vscode(user_data_dirs: &[String], timeout_secs: u64) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let _ = timeout_secs;
    let default_dir = get_default_vscode_user_data_dir_for_os()
        .map(|value| normalize_path_for_compare(&value))
        .filter(|value| !value.is_empty());
    crate::modules::logger::log_info(&format!(
        "[VSCode Close] default_dir={}",
        default_dir
            .as_deref()
            .map(|value| summarize_text_for_process_log(value, 96))
            .unwrap_or_else(|| "-".to_string())
    ));
    close_managed_instances_common(
        "VSCode Close",
        "正在关闭 VS Code...",
        "未提供可关闭的实例目录",
        "受管 VS Code 实例未在运行，无需关闭",
        "VS Code ",
        "无法关闭受管 VS Code 实例进程，请手动关闭后重试",
        user_data_dirs,
        timeout_secs,
        collect_vscode_process_entries,
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(
                collect_vscode_process_entries(),
                target_dirs,
                default_dir.as_deref(),
            )
        },
        Some(request_vscode_graceful_close as fn(u32)),
        Some(2),
        #[cfg(target_os = "windows")]
        Some(log_vscode_process_details_for_pids as fn(&[u32])),
        #[cfg(not(target_os = "windows"))]
        None,
    )
}

fn request_vscode_graceful_close(pid: u32) {
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
                        "[VSCode Close] 已发送优雅退出请求 pid={}",
                        pid
                    ));
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    crate::modules::logger::log_warn(&format!(
                        "[VSCode Close] 优雅退出失败 pid={} err={}",
                        pid,
                        stderr.trim()
                    ));
                }
            }
            Err(e) => {
                crate::modules::logger::log_warn(&format!(
                    "[VSCode Close] 调用 osascript 失败 pid={} err={}",
                    pid, e
                ));
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
    }
}
