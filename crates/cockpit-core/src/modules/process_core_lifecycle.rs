// cockpit-core Process：Process close/restart lifecycle, Codex and OpenCode runtime。
// 通过 include! 保持原模块作用域和跨平台调用路径。
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
    close_managed_instances_common(
        "AG Close",
        "正在关闭受管 Antigravity IDE 实例...",
        "未提供可关闭的 Antigravity IDE 实例目录",
        "受管 Antigravity IDE 实例未在运行，无需关闭",
        "受管 Antigravity IDE ",
        "无法关闭受管 Antigravity IDE 实例进程，请手动关闭后重试",
        user_data_dirs,
        timeout_secs,
        collect_antigravity_process_entries,
        |entries, target_dirs| {
            select_main_pids_by_target_dirs(entries, target_dirs, default_dir.as_deref())
        },
        |target_dirs| {
            filter_entries_by_target_dirs(
                collect_antigravity_process_entries(),
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

    send_close_signal(pid);
    if wait_pids_exit(&[pid], timeout_secs) {
        Ok(())
    } else {
        Err("无法关闭实例进程，请手动关闭后重试".to_string())
    }
}

fn send_close_signal(pid: u32) {
    if pid == 0 || !is_pid_running(pid) {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        crate::modules::logger::log_info(&format!("[AG Close] taskkill start pid={}", pid));
        let output = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match output {
            Ok(value) => {
                if value.status.success() {
                    crate::modules::logger::log_info(&format!(
                        "[AG Close] taskkill success pid={} status={}",
                        pid, value.status
                    ));
                } else {
                    let stderr = String::from_utf8_lossy(&value.stderr);
                    crate::modules::logger::log_warn(&format!(
                        "[AG Close] taskkill failed pid={} status={} stderr={}",
                        pid,
                        value.status,
                        stderr.trim()
                    ));
                }
            }
            Err(err) => {
                crate::modules::logger::log_warn(&format!(
                    "[AG Close] taskkill error pid={} err={}",
                    pid, err
                ));
            }
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = Command::new("kill")
            .args(["-15", &pid.to_string()])
            .output();
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
        thread::sleep(Duration::from_millis(350));
    }
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

    for pid in &targets {
        send_close_signal(*pid);
    }

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
        Err("无法关闭实例进程，请手动关闭后重试".to_string())
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

        let mut cmd = Command::new(&launch_path);
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
        } else {
            crate::modules::logger::log_info(&format!(
                "[Codex Instances] pid={} CODEX_HOME not found",
                pid
            ));
        }
        result.push((pid, codex_home));
    }
    filter_entries_by_expected_launch_path("Codex", result, expected_launch)
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

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
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

/// 启动 Codex（支持 CODEX_HOME 与附加参数，仅 macOS）
pub fn start_codex_with_args(codex_home: &str, extra_args: &[String]) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_codex_launch_path()
            .ok()
            .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
            .or_else(|| resolve_macos_app_root_from_config("codex"));
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codex"))?;

        let codex_home_trimmed = codex_home.trim();
        let mut args: Vec<String> = Vec::new();
        for arg in extra_args {
            if !arg.trim().is_empty() {
                args.push(arg.to_string());
            }
        }

        // 使用 open -a 启动，避免 macOS Responsible Process 归因
        // 注意：CODEX_HOME 环境变量无法通过 open -a 传递，
        // 如果指定了 codex_home 则需要回退到直接执行
        if !codex_home_trimmed.is_empty() {
            if let Ok(launch_path) = resolve_codex_launch_path() {
                let mut cmd = Command::new(&launch_path);
                apply_managed_proxy_env_to_command(&mut cmd);
                sanitize_macos_gui_launch_env(&mut cmd);
                cmd.env("CODEX_HOME", codex_home_trimmed);
                for arg in &args {
                    cmd.arg(arg);
                }
                let child =
                    spawn_detached_unix(&mut cmd).map_err(|e| format!("启动 Codex 失败: {}", e))?;
                crate::modules::logger::log_info("Codex 启动命令已发送（直接执行，带 CODEX_HOME）");
                // 轮询获取真实 PID
                let probe_started = Instant::now();
                let timeout = Duration::from_secs(6);
                while probe_started.elapsed() < timeout {
                    if let Some(resolved_pid) = resolve_codex_pid(None, Some(codex_home_trimmed)) {
                        return Ok(resolved_pid);
                    }
                    thread::sleep(Duration::from_millis(200));
                }
                return Ok(child.id());
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
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
        }

        let child =
            spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Codex 失败: {}", e))?;
        crate::modules::logger::log_info(&format!(
            "[Codex Start] Windows 实例启动命令已发送 launch_path={} codex_home={} app_user_data_dir={} pid={}",
            launch_path.to_string_lossy(),
            summarize_text_for_process_log(codex_home_trimmed, 96),
            app_user_data_dir.to_string_lossy(),
            child.id()
        ));

        let probe_started = Instant::now();
        let timeout = Duration::from_secs(15);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_codex_pid(None, Some(codex_home_trimmed)) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(250));
        }
        crate::modules::logger::log_warn(&format!(
            "[Codex Start] Windows 实例启动后 15s 内未匹配到实例 PID，回退 spawn pid={}",
            child.id()
        ));
        Ok(child.id())
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = (codex_home, extra_args);
        Err("Codex 应用多开仅支持 macOS 和 Windows".to_string())
    }
}

/// 启动 Codex 默认实例（不注入 CODEX_HOME，支持附加参数，支持 macOS / Windows）
pub fn start_codex_default(extra_args: &[String]) -> Result<u32, String> {
    #[cfg(target_os = "macos")]
    {
        let app_root = resolve_codex_launch_path()
            .ok()
            .and_then(|p| resolve_macos_app_root_from_launch_path(&p))
            .or_else(|| resolve_macos_app_root_from_config("codex"));
        let app_root = app_root.ok_or_else(|| app_path_missing_error("codex"))?;

        let mut args: Vec<String> = Vec::new();
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                args.push(trimmed.to_string());
            }
        }

        // 使用 open -n -a 启动默认实例，避免复用已运行的其他 Codex 实例
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
            "[Codex Start] 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            open_pid
        ));
        return Ok(open_pid);
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let before_pids: HashSet<u32> = collect_codex_process_entries()
            .into_iter()
            .map(|(pid, _)| pid)
            .collect();
        let app_user_model_id = detect_codex_store_app_user_model_id();
        if let Some(app_user_model_id) = app_user_model_id {
            crate::modules::logger::log_info(&format!(
                "[Codex Start] 启动策略候选=system-store-entry app_id={}",
                app_user_model_id
            ));
            match launch_codex_via_store_app_user_model_id(&app_user_model_id) {
                Ok(()) => {
                    crate::modules::logger::log_info(&format!(
                        "[Codex Start] 已通过系统入口启动 Codex: {}",
                        app_user_model_id
                    ));
                    let probe_started = Instant::now();
                    let timeout = Duration::from_secs(15);
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
                    if let Some(pid) = resolve_codex_pid(None, None) {
                        crate::modules::logger::log_info(&format!(
                            "[Codex Start] 启动策略=system-store-entry app_id={} pid={}",
                            app_user_model_id, pid
                        ));
                        return Ok(pid);
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
        // Codex 是 GUI 应用，不设置 CREATE_NO_WINDOW，否则会导致其内部 spawn CLI 子进程失败
        for arg in extra_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                cmd.arg(trimmed);
            }
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = extra_args;
        Err("Codex 启动仅支持 macOS 和 Windows".to_string())
    }
}

/// 关闭受管 Codex 实例（按 CODEX_HOME 匹配，包含默认实例目录）
pub fn close_codex_instances(codex_homes: &[String], timeout_secs: u64) -> Result<(), String> {
    #[cfg(target_os = "macos")]
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

        crate::modules::logger::log_info(&format!(
            "准备关闭 {} 个受管 Codex 主进程...",
            pids.len()
        ));
        let _ = close_pids(&pids, timeout_secs);

        let still_running = collect_codex_process_entries()
            .into_iter()
            .any(|(_, home)| {
                let resolved_home = home
                    .as_ref()
                    .map(|value| normalize_path_for_compare(value))
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| default_home.clone());
                !resolved_home.is_empty() && target_homes.contains(&resolved_home)
            });
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

        let default_app_dir = get_default_codex_windows_app_user_data_dir()
            .map(|value| normalize_path_for_compare(&value))
            .filter(|value| !value.is_empty());

        if target_app_dirs.is_empty() && !includes_default {
            crate::modules::logger::log_info("未提供可关闭的 Codex 实例目录");
            return Ok(());
        }

        let matches_target =
            |dir: Option<&String>, target_app_dirs: &HashSet<String>, includes_default: bool| {
                match dir {
                    Some(value) => {
                        let normalized = normalize_path_for_compare(value);
                        (!normalized.is_empty() && target_app_dirs.contains(&normalized))
                            || (includes_default
                                && default_app_dir.as_deref() == Some(normalized.as_str()))
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
        let _ = close_pids(&pids, timeout_secs);

        let still_running = collect_codex_process_entries()
            .into_iter()
            .any(|(_, dir)| matches_target(dir.as_ref(), &target_app_dirs, includes_default));
        if still_running {
            return Err("无法关闭受管 Codex 实例进程，请手动关闭后重试".to_string());
        }
        Ok(())
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = (codex_homes, timeout_secs);
        Err("Codex 应用多开仅支持 macOS 和 Windows".to_string())
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
    #[cfg(target_os = "macos")]
    {
        let bundle_pattern = format!(
            "{}/contents/",
            platform.macos_app_name().to_ascii_lowercase()
        );
        if let Ok(output) = Command::new("ps")
            .args(["-axww", "-o", "command="])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return stdout.lines().any(|line| {
                let lower = line.to_lowercase();
                lower.contains(&bundle_pattern)
                    && !lower.contains("--type=")
                    && !lower.contains("crashpad_handler")
                    && !is_helper_command_line(&lower)
            });
        }
        return false;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = platform;
        is_trae_running()
    }
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

