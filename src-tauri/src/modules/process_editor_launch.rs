// Process 模块：Editor and IDE launch wrappers with platform-specific arguments。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
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

fn apply_workbuddy_instance_env(
    cmd: &mut Command,
    config_dir: &std::path::Path,
    electron_user_data_dir: &std::path::Path,
) {
    // Official WorkBuddy main process:
    //   WORKBUDDY_CONFIG_DIR / CODEBUDDY_CONFIG_DIR → ~/.workbuddy
    //   WORKBUDDY_USER_DATA_DIR → {config}/app  (also app.setPath("userData", ...))
    // `--user-data-dir` alone is NOT enough: configureElectronApp() overrides userData.
    // `open -a` cannot pass these envs, so managed instances must exec Electron directly.
    cmd.env("WORKBUDDY_CONFIG_DIR", config_dir);
    cmd.env("CODEBUDDY_CONFIG_DIR", config_dir);
    cmd.env("WORKBUDDY_USER_DATA_DIR", electron_user_data_dir);
}

pub fn start_workbuddy_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let (config_dir, electron_user_data_dir) =
        crate::modules::workbuddy_instance::resolve_workbuddy_runtime_dirs(user_data_dir)?;
    std::fs::create_dir_all(&config_dir).map_err(|e| {
        format!(
            "创建 WorkBuddy 配置目录失败 ({}): {}",
            config_dir.to_string_lossy(),
            e
        )
    })?;
    std::fs::create_dir_all(&electron_user_data_dir).map_err(|e| {
        format!(
            "创建 WorkBuddy Electron 数据目录失败 ({}): {}",
            electron_user_data_dir.to_string_lossy(),
            e
        )
    })?;
    let electron_dir_str = electron_user_data_dir.to_string_lossy().to_string();

    #[cfg(target_os = "macos")]
    {
        // Managed multi-instance: exec Electron binary with env (open -a cannot pass env).
        let launch_path = resolve_workbuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        sanitize_macos_gui_launch_env(&mut cmd);
        apply_workbuddy_instance_env(&mut cmd, &config_dir, &electron_user_data_dir);
        cmd.arg(format!("--user-data-dir={}", electron_dir_str));
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
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Start] managed instance via Electron binary; config_dir={} user_data_dir={} launch_path={}",
            config_dir.to_string_lossy(),
            electron_dir_str,
            launch_path.to_string_lossy()
        ));
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_workbuddy_pid(None, Some(&electron_dir_str)) {
                return Ok(resolved_pid);
            }
            // Also match config root if instance store still points there.
            if let Some(resolved_pid) = resolve_workbuddy_pid(None, Some(user_data_dir.trim())) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Start] 启动后 6s 内未匹配到实例 PID，回退 child pid={}",
            child.id()
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_workbuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        apply_workbuddy_instance_env(&mut cmd, &config_dir, &electron_user_data_dir);
        if should_detach_child() {
            cmd.creation_flags(0x08000000 | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        } else {
            cmd.creation_flags(0x08000000);
        }
        cmd.arg("--user-data-dir").arg(&electron_dir_str);
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
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Start] managed instance; config_dir={} user_data_dir={}",
            config_dir.to_string_lossy(),
            electron_dir_str
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_workbuddy_launch_path()?;
        let mut cmd = Command::new(&launch_path);
        apply_managed_proxy_env_to_command(&mut cmd);
        apply_workbuddy_instance_env(&mut cmd, &config_dir, &electron_user_data_dir);
        if should_detach_child() {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        }
        cmd.arg("--user-data-dir").arg(&electron_dir_str);
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
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Start] managed instance; config_dir={} user_data_dir={}",
            config_dir.to_string_lossy(),
            electron_dir_str
        ));
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

pub fn start_trae_platform_with_args_with_new_window(
    platform_id: &str,
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let platform = crate::modules::trae_account::TraePlatformKind::parse(Some(platform_id))?;

    #[cfg(target_os = "macos")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path_for_platform(platform)?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error(platform.provider_key()))?;

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

        return launch_trae_macos_with_verification(
            platform,
            &app_root,
            &args,
            Some(target),
            /* prefer_new_instance */ true,
        );
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path_for_platform(platform)?;

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
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!("{} 启动命令已发送", platform.display_name()));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let target = user_data_dir.trim();
        if target.is_empty() {
            return Err("实例目录为空，无法启动".to_string());
        }
        let launch_path = resolve_trae_launch_path_for_platform(platform)?;

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

        let child = spawn_detached_unix(&mut cmd)
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!("{} 启动命令已发送", platform.display_name()));
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (platform, user_data_dir, extra_args, use_new_window);
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

        let open_pid = spawn_open_app_with_options(&app_root, &args, false)
            .map_err(|e| format!("启动 Trae 失败: {}", e))?;
        crate::modules::logger::log_info("Trae 默认实例启动命令已发送（open -a）");
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

pub fn start_trae_platform_default_with_args_with_new_window(
    platform_id: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let platform = crate::modules::trae_account::TraePlatformKind::parse(Some(platform_id))?;

    #[cfg(target_os = "macos")]
    {
        let launch_path = resolve_trae_launch_path_for_platform(platform)?;
        let app_root = resolve_macos_app_root_from_launch_path(&launch_path)
            .ok_or_else(|| app_path_missing_error(platform.provider_key()))?;

        // After a hard close, cold-start without forcing --new-window/--reuse-window.
        // Window flags are only useful when an instance is already alive; on cold
        // start they are unnecessary and extra args make debugging noisier.
        let mut args: Vec<String> = Vec::new();
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }
        let _ = use_new_window;

        return launch_trae_macos_with_verification(
            platform, &app_root, &args, None, /* prefer_new_instance */ false,
        );
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let launch_path = resolve_trae_launch_path_for_platform(platform)?;
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
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!(
            "{} 默认实例启动命令已发送",
            platform.display_name()
        ));
        return Ok(child.id());
    }

    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_trae_launch_path_for_platform(platform)?;
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
        let child = spawn_detached_unix(&mut cmd)
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!(
            "{} 默认实例启动命令已发送",
            platform.display_name()
        ));
        return Ok(child.id());
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (platform, extra_args, use_new_window);
        Err("Trae 默认实例仅支持 macOS、Windows 和 Linux".to_string())
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

