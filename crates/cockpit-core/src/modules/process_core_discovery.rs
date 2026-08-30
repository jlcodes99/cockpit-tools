// cockpit-core Process：Process launch helpers and executable/path discovery。
// 通过 include! 保持原模块作用域和跨平台调用路径。
use crate::modules::config;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
#[cfg(not(target_os = "macos"))]
use sysinfo::{Pid, ProcessRefreshKind, System, UpdateKind};

#[cfg(any(target_os = "macos", target_os = "windows"))]
const OPENCODE_APP_NAME: &str = "OpenCode";
#[cfg(target_os = "macos")]
const TRAE_APP_NAME: &str = "Trae";
#[cfg(target_os = "macos")]
const CODEX_APP_PATH: &str = "/Applications/Codex.app/Contents/MacOS/Codex";
#[cfg(target_os = "macos")]
const CODEX_CHATGPT_APP_PATH: &str = "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT";
#[cfg(target_os = "macos")]
const ANTIGRAVITY_APP_PATH: &str = "/Applications/Antigravity IDE.app/Contents/MacOS/Electron";
#[cfg(target_os = "macos")]
const ANTIGRAVITY_APP_CONTENTS_MARKER: &str = "antigravity ide.app/contents/";
#[cfg(target_os = "macos")]
const ANTIGRAVITY_APP_EXEC_MARKER: &str = "antigravity ide.app/contents/macos/electron";
#[cfg(target_os = "macos")]
const VSCODE_APP_PATH: &str = "/Applications/Visual Studio Code.app/Contents/MacOS/Electron";

#[cfg(target_os = "windows")]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
#[cfg(target_os = "windows")]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const WINDOWS_PROCESS_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// On macOS, extract the executable path from a `ps` command line output.
/// Handles paths with spaces in .app bundles (e.g., "Visual Studio Code.app").
#[cfg(target_os = "macos")]
fn extract_macos_exe_from_cmdline(cmdline: &str) -> Option<String> {
    let lower = cmdline.to_lowercase();
    // For .app bundles: find the binary after Contents/MacOS/
    if let Some(contents_pos) = lower.find(".app/contents/macos/") {
        let after_macos = contents_pos + ".app/contents/macos/".len();
        // Binary name goes until next whitespace or end
        let rest = &cmdline[after_macos..];
        let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        return Some(cmdline[..after_macos + end].to_string());
    }
    // For non-.app executables: first whitespace-delimited token
    cmdline.split_whitespace().next().map(|s| s.to_string())
}

fn strict_process_detect_enabled() -> bool {
    std::env::var("AG_STRICT_PROCESS_DETECT")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn command_trace_enabled() -> bool {
    if let Ok(value) = std::env::var("COCKPIT_COMMAND_TRACE") {
        if let Some(enabled) = parse_env_bool(&value) {
            return enabled;
        }
    }
    false
}

pub fn summarize_text_for_process_log(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let mut current = String::new();
    for _ in 0..max_chars {
        let Some(ch) = iter.next() else {
            return text.to_string();
        };
        current.push(ch);
    }
    if iter.next().is_none() {
        text.to_string()
    } else {
        format!("{}...", current)
    }
}

pub fn summarize_pid_list_for_log(pids: &[u32]) -> String {
    let sample_limit = 8usize;
    let sample = pids
        .iter()
        .take(sample_limit)
        .map(u32::to_string)
        .collect::<Vec<_>>();
    format!(
        "count={}, sample=[{}{}]",
        pids.len(),
        sample.join(", "),
        if pids.len() > sample_limit {
            ", ..."
        } else {
            ""
        }
    )
}

fn summarize_target_dirs_for_log(target_dirs: &HashSet<String>) -> String {
    let mut items = target_dirs
        .iter()
        .map(|value| summarize_text_for_process_log(value, 96))
        .collect::<Vec<_>>();
    items.sort();
    let sample_limit = 4usize;
    let sample = items.iter().take(sample_limit).cloned().collect::<Vec<_>>();
    format!(
        "count={}, sample={:?}{}",
        items.len(),
        sample,
        if items.len() > sample_limit {
            "..."
        } else {
            ""
        }
    )
}

fn summarize_process_entries_for_log(entries: &[(u32, Option<String>)]) -> String {
    let sample_limit = 4usize;
    let sample = entries
        .iter()
        .take(sample_limit)
        .map(|(pid, path)| {
            format!(
                "{}|{}",
                pid,
                path.as_deref()
                    .map(|value| summarize_text_for_process_log(value, 96))
                    .unwrap_or_else(|| "-".to_string())
            )
        })
        .collect::<Vec<_>>();
    format!(
        "count={}, sample={:?}{}",
        entries.len(),
        sample,
        if entries.len() > sample_limit {
            "..."
        } else {
            ""
        }
    )
}

fn quote_command_part(part: &str) -> String {
    if part.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quote = part.chars().any(|ch| {
        ch.is_whitespace() || matches!(ch, '"' | '\'' | '$' | '`' | '|' | '&' | ';' | '(' | ')')
    });
    if !needs_quote {
        return part.to_string();
    }
    format!("{:?}", part)
}

fn format_command_preview(command: &Command) -> String {
    let program = quote_command_part(command.get_program().to_string_lossy().as_ref());
    let args = command
        .get_args()
        .map(|arg| quote_command_part(arg.to_string_lossy().as_ref()))
        .collect::<Vec<String>>();
    let preview = if args.is_empty() {
        program
    } else {
        format!("{} {}", program, args.join(" "))
    };
    summarize_text_for_process_log(&preview, 600)
}

#[cfg(target_os = "windows")]
fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(target_os = "windows")]
fn build_windows_path_filtered_process_probe_script(
    process_name: &str,
    expected_exe_path: &str,
) -> String {
    let process = escape_powershell_single_quoted(process_name);
    let expected = escape_powershell_single_quoted(expected_exe_path);
    format!(
        r#"$processName='{process}';
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
Get-CimInstance Win32_Process -Filter ("Name='" + $processName + "'") |
  Where-Object {{
    $exe = Normalize-ExePath $_.ExecutablePath
    if (-not $exe) {{ $exe = Normalize-ExePath (Get-ExePathFromCmdLine $_.CommandLine) }}
    $exe -eq $expected
  }} |
  ForEach-Object {{ "$($_.ProcessId)|$($_.CommandLine)" }}"#
    )
}

#[cfg(target_os = "windows")]
fn truncate_for_trace(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let mut current = String::new();
    for _ in 0..max_chars {
        let Some(ch) = iter.next() else {
            return text.to_string();
        };
        current.push(ch);
    }
    if iter.next().is_none() {
        text.to_string()
    } else {
        format!("{}...(truncated)", current)
    }
}

#[cfg(target_os = "windows")]
fn output_bytes_for_trace(bytes: &[u8]) -> String {
    let value = String::from_utf8_lossy(bytes);
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "<empty>".to_string()
    } else {
        truncate_for_trace(trimmed, 400)
    }
}

fn log_command_trace_exec(command_preview: &str) {
    if !command_trace_enabled() {
        return;
    }
    crate::modules::logger::log_info(&format!("[CmdTrace] EXEC {}", command_preview));
}

#[cfg(target_os = "windows")]
fn log_command_trace_result(
    command_preview: &str,
    result: &std::io::Result<std::process::Output>,
    elapsed: Duration,
) {
    if !command_trace_enabled() {
        return;
    }
    match result {
        Ok(output) => {
            crate::modules::logger::log_info(&format!(
                "[CmdTrace] RESULT elapsed={}ms status={} cmd={}",
                elapsed.as_millis(),
                output.status,
                command_preview
            ));
            crate::modules::logger::log_info(&format!(
                "[CmdTrace] STDOUT cmd={} => {}",
                command_preview,
                output_bytes_for_trace(&output.stdout)
            ));
            crate::modules::logger::log_info(&format!(
                "[CmdTrace] STDERR cmd={} => {}",
                command_preview,
                output_bytes_for_trace(&output.stderr)
            ));
        }
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[CmdTrace] ERROR elapsed={}ms cmd={} err={}",
                elapsed.as_millis(),
                command_preview,
                err
            ));
        }
    }
}

fn log_command_trace_spawn_result(
    command_preview: &str,
    result: &std::io::Result<Child>,
    elapsed: Duration,
) {
    if !command_trace_enabled() {
        return;
    }
    match result {
        Ok(child) => crate::modules::logger::log_info(&format!(
            "[CmdTrace] SPAWN elapsed={}ms pid={} cmd={}",
            elapsed.as_millis(),
            child.id(),
            command_preview
        )),
        Err(err) => crate::modules::logger::log_warn(&format!(
            "[CmdTrace] SPAWN_ERROR elapsed={}ms cmd={} err={}",
            elapsed.as_millis(),
            command_preview,
            err
        )),
    }
}

fn spawn_command_with_trace(cmd: &mut Command) -> std::io::Result<Child> {
    let preview = format_command_preview(cmd);
    log_command_trace_exec(&preview);
    let start = Instant::now();
    let result = cmd.spawn();
    log_command_trace_spawn_result(&preview, &result, start.elapsed());
    result
}

#[cfg(target_os = "windows")]
fn build_powershell_command(args: &[&str]) -> Command {
    use std::os::windows::process::CommandExt;

    let mut final_args: Vec<String> = vec![
        "-WindowStyle".to_string(),
        "Hidden".to_string(),
        "-NonInteractive".to_string(),
        "-NoProfile".to_string(),
    ];
    let mut index = 0;
    while index < args.len() {
        let arg = args[index];
        if arg.eq_ignore_ascii_case("-NoProfile") || arg.eq_ignore_ascii_case("-NonInteractive") {
            index += 1;
            continue;
        }
        if arg.eq_ignore_ascii_case("-WindowStyle") {
            index += if index + 1 < args.len() { 2 } else { 1 };
            continue;
        }
        if arg.eq_ignore_ascii_case("-Command") {
            let script = args.get(index + 1).copied().unwrap_or("");
            let wrapped = format!(
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; $OutputEncoding=[System.Text.Encoding]::UTF8; {}",
                script
            );
            final_args.push("-Command".to_string());
            final_args.push(wrapped);
            index += if index + 1 < args.len() { 2 } else { 1 };
            continue;
        }
        final_args.push(arg.to_string());
        index += 1;
    }

    let mut command = Command::new("powershell");
    command.creation_flags(CREATE_NO_WINDOW).args(final_args);
    command
}

#[cfg(target_os = "windows")]
fn powershell_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = build_powershell_command(args);
    let preview = format_command_preview(&command);
    log_command_trace_exec(&preview);
    let start = Instant::now();
    let result = command.output();
    log_command_trace_result(&preview, &result, start.elapsed());
    result
}

#[cfg(target_os = "windows")]
fn powershell_output_with_timeout(
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::{Error, ErrorKind, Read};

    let mut command = build_powershell_command(args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let preview = format_command_preview(&command);
    log_command_trace_exec(&preview);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            if command_trace_enabled() {
                crate::modules::logger::log_warn(&format!(
                    "[CmdTrace] SPAWN_ERROR elapsed=0ms cmd={} err={}",
                    preview, err
                ));
            }
            return Err(err);
        }
    };
    let start = Instant::now();

    loop {
        if let Some(status) = child.try_wait()? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_end(&mut stdout);
            }
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_end(&mut stderr);
            }
            let result = Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
            log_command_trace_result(&preview, &result, start.elapsed());
            return result;
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let result = Err(Error::new(
                ErrorKind::TimedOut,
                format!("PowerShell 进程探测超时（{}ms）", timeout.as_millis()),
            ));
            log_command_trace_result(&preview, &result, start.elapsed());
            return result;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(target_os = "windows")]
fn cmd_output(args: &[&str]) -> std::io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new("cmd");
    command.creation_flags(CREATE_NO_WINDOW).args(args);
    let preview = format_command_preview(&command);
    log_command_trace_exec(&preview);
    let start = Instant::now();
    let result = command.output();
    log_command_trace_result(&preview, &result, start.elapsed());
    result
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(target_os = "windows")]
fn powershell_array_literal(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| powershell_quote(value))
        .collect::<Vec<String>>()
        .join(",")
}

#[cfg(target_os = "windows")]
fn normalize_windows_candidate_path(raw: &str) -> Option<std::path::PathBuf> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }

    let mut normalized = text.trim_matches('"').trim_matches('\'').trim().to_string();
    let lowered = normalized.to_lowercase();
    if let Some(index) = lowered.find(".exe") {
        normalized.truncate(index + 4);
    }
    let normalized = normalized
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches(',')
        .trim()
        .to_string();
    if normalized.is_empty() {
        return None;
    }

    let path = std::path::PathBuf::from(normalized);
    if path.exists() && path.is_file() {
        Some(path)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn score_windows_candidate(
    path: &std::path::Path,
    exe_names_lower: &HashSet<String>,
    keywords_lower: &[String],
) -> Option<i32> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if file_name.is_empty() {
        return None;
    }

    let path_lower = path.to_string_lossy().to_lowercase();
    let has_keyword = keywords_lower
        .iter()
        .any(|keyword| !keyword.is_empty() && path_lower.contains(keyword));

    if exe_names_lower.contains(&file_name) {
        if file_name == "electron.exe" && !has_keyword {
            return None;
        }
        let mut score = if file_name == "electron.exe" { 60 } else { 100 };
        if has_keyword {
            score += 5;
        }
        return Some(score);
    }

    let is_exe = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);
    if is_exe && has_keyword {
        return Some(50);
    }
    None
}

#[cfg(target_os = "windows")]
fn parse_windows_exec_candidates(
    app_label: &str,
    exe_names: &[&str],
    display_keywords: &[&str],
    output: std::process::Output,
) -> Option<std::path::PathBuf> {
    let exe_names_lower: HashSet<String> =
        exe_names.iter().map(|value| value.to_lowercase()).collect();
    let keywords_lower: Vec<String> = display_keywords
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();

    let mut seen: HashSet<String> = HashSet::new();
    let mut best: Option<(std::path::PathBuf, i32)> = None;
    let mut raw_lines = 0usize;
    let mut scored_candidates = 0usize;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with("STAGE:") {
            continue;
        }
        raw_lines += 1;
        let Some(path) = normalize_windows_candidate_path(line) else {
            continue;
        };
        let dedupe_key = path.to_string_lossy().to_lowercase();
        if !seen.insert(dedupe_key) {
            continue;
        }
        let Some(score) = score_windows_candidate(&path, &exe_names_lower, &keywords_lower) else {
            continue;
        };
        scored_candidates += 1;
        match best.as_ref() {
            Some((_, current_score)) if *current_score >= score => {}
            _ => best = Some((path, score)),
        }
    }

    if let Some((path, score)) = best {
        crate::modules::logger::log_info(&format!(
            "[Path Detect] {} auto detect hit: {}, score={}",
            app_label,
            path.to_string_lossy(),
            score
        ));
        return Some(path);
    }

    let local_appdata = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "<unset>".to_string());
    let program_files = std::env::var("PROGRAMFILES").unwrap_or_else(|_| "<unset>".to_string());
    let program_files_x86 =
        std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| "<unset>".to_string());
    crate::modules::logger::log_warn(&format!(
        "[Path Detect] {} Windows multi-source detect miss: raw_lines={}, unique_candidates={}, scored_candidates={}, local_appdata={}, program_files={}, program_files_x86={}",
        app_label,
        raw_lines,
        seen.len(),
        scored_candidates,
        local_appdata,
        program_files,
        program_files_x86
    ));
    None
}

#[cfg(target_os = "windows")]
fn decode_utf16le(bytes: &[u8]) -> String {
    // Skip UTF-16 LE BOM if present
    let bytes = if bytes.starts_with(&[0xFF, 0xFE]) {
        &bytes[2..]
    } else {
        bytes
    };
    let mut words = Vec::with_capacity(bytes.len() / 2);
    let mut iter = bytes.iter().copied();
    while let Some(lo) = iter.next() {
        let hi = iter.next().unwrap_or(0);
        words.push(u16::from_le_bytes([lo, hi]));
    }
    String::from_utf16_lossy(&words)
}

#[cfg(target_os = "windows")]
fn reg_query_value(key: &str, value_name: &str) -> Option<String> {
    let cmd = if value_name == "(Default)" {
        format!("reg query \"{}\" /ve", key)
    } else {
        format!("reg query \"{}\" /v {}", key, value_name)
    };
    let output = cmd_output(&["/u", "/c", &cmd]).ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = decode_utf16le(&output.stdout);
    let value_name_lower = value_name.to_lowercase();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let matches_name = if value_name == "(Default)" {
            trimmed.starts_with("(Default)")
        } else {
            trimmed.to_lowercase().starts_with(&value_name_lower)
        };
        if !matches_name {
            continue;
        }
        if let Some(pos) = trimmed.find("REG_") {
            let after = &trimmed[pos..];
            if let Some(ws_idx) = after.find(char::is_whitespace) {
                let value = after[ws_idx..].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_vscode_exec_path_by_registry() -> Option<std::path::PathBuf> {
    let exe_names = ["Code.exe", "Code - Insiders.exe"];
    let app_path_roots = [
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
        "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths",
    ];
    for root in app_path_roots {
        for exe in exe_names {
            let key = format!("{}\\{}", root, exe);
            if let Some(value) = reg_query_value(&key, "(Default)") {
                if let Some(path) = normalize_windows_candidate_path(&value) {
                    crate::modules::logger::log_info(&format!(
                        "[Path Detect] vscode registry hit: {}",
                        path.to_string_lossy()
                    ));
                    return Some(path);
                }
            }
            if let Some(path_root) = reg_query_value(&key, "Path") {
                let candidate = std::path::PathBuf::from(path_root).join(exe);
                if candidate.exists() {
                    crate::modules::logger::log_info(&format!(
                        "[Path Detect] vscode registry hit: {}",
                        candidate.to_string_lossy()
                    ));
                    return Some(candidate);
                }
            }
        }
    }

    let uninstall_roots = [
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    ];
    let keywords = ["visual studio code", "vs code", "vscode"];
    for root in uninstall_roots {
        let cmd = format!("reg query \"{}\" /s /v DisplayName", root);
        let output = match cmd_output(&["/u", "/c", &cmd]) {
            Ok(o) => o,
            Err(_) => continue,
        };
        if !output.status.success() {
            continue;
        }
        let stdout = decode_utf16le(&output.stdout);
        let mut current_key: Option<String> = None;
        let mut matched_keys: Vec<String> = Vec::new();
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("HKEY_") {
                current_key = Some(trimmed.to_string());
                continue;
            }
            if !trimmed.to_lowercase().starts_with("displayname") {
                continue;
            }
            if let Some(pos) = trimmed.find("REG_") {
                let after = &trimmed[pos..];
                if let Some(ws_idx) = after.find(char::is_whitespace) {
                    let value = after[ws_idx..].trim().to_lowercase();
                    if keywords.iter().any(|kw| value.contains(kw)) {
                        if let Some(key) = current_key.as_ref() {
                            matched_keys.push(key.clone());
                        }
                    }
                }
            }
        }
        for key in matched_keys {
            for value_name in ["DisplayIcon", "UninstallString"] {
                if let Some(value) = reg_query_value(&key, value_name) {
                    if let Some(path) = normalize_windows_candidate_path(&value) {
                        crate::modules::logger::log_info(&format!(
                            "[Path Detect] vscode registry hit: {}",
                            path.to_string_lossy()
                        ));
                        return Some(path);
                    }
                }
            }
            if let Some(install_root) = reg_query_value(&key, "InstallLocation") {
                for exe in exe_names {
                    let candidate = std::path::PathBuf::from(&install_root).join(exe);
                    if candidate.exists() {
                        crate::modules::logger::log_info(&format!(
                            "[Path Detect] vscode registry hit: {}",
                            candidate.to_string_lossy()
                        ));
                        return Some(candidate);
                    }
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
pub fn detect_windows_exec_path_by_signatures(
    app_label: &str,
    exe_names: &[&str],
    command_names: &[&str],
    protocol_names: &[&str],
    display_keywords: &[&str],
) -> Option<std::path::PathBuf> {
    if exe_names.is_empty() {
        return None;
    }

    let exe_array = powershell_array_literal(exe_names);
    let command_array = powershell_array_literal(command_names);
    let protocol_array = powershell_array_literal(protocol_names);
    let keyword_array = powershell_array_literal(display_keywords);

    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue'
Write-Output 'STAGE:BEGIN'
$exeNames=@({exe_array})
$commandNames=@({command_array})
$protocolNames=@({protocol_array})
$keywords=@({keyword_array})

function Normalize-Candidate([string]$raw) {{
  if ([string]::IsNullOrWhiteSpace($raw)) {{ return $null }}
  $text = $raw.Trim()
  if ($text -match '(?i)(?<p>[A-Za-z]:\\.+?\.exe)') {{
    $text = $matches['p']
  }}
  $text = $text.Trim().Trim('"').Trim("'")
  if ([string]::IsNullOrWhiteSpace($text)) {{ return $null }}
  return $text
}}

function Emit-Candidate([string]$raw) {{
  $candidate = Normalize-Candidate $raw
  if ([string]::IsNullOrWhiteSpace($candidate)) {{ return }}
  if (Test-Path -LiteralPath $candidate) {{ Write-Output $candidate }}
}}

Write-Output 'STAGE:APP_PATHS'
$appPathRoots=@(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\App Paths',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths'
)
foreach ($root in $appPathRoots) {{
  foreach ($exe in $exeNames) {{
    $keyPath = Join-Path $root $exe
    $entry = Get-ItemProperty -Path $keyPath -ErrorAction SilentlyContinue
    if ($entry) {{
      Emit-Candidate $entry.'(default)'
      if ($entry.Path) {{
        Emit-Candidate (Join-Path $entry.Path $exe)
      }}
    }}
  }}
}}

Write-Output 'STAGE:UNINSTALL'
$uninstallRoots=@(
  'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
  'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*'
)
foreach ($root in $uninstallRoots) {{
  Get-ItemProperty -Path $root -ErrorAction SilentlyContinue | ForEach-Object {{
    $display = [string]$_.DisplayName
    $displayLower = $display.ToLowerInvariant()
    $hit = $false
    foreach ($kw in $keywords) {{
      if ([string]::IsNullOrWhiteSpace($kw)) {{ continue }}
      if ($displayLower.Contains($kw.ToLowerInvariant())) {{
        $hit = $true
        break
      }}
    }}
    if (-not $hit) {{ return }}
    Emit-Candidate $_.DisplayIcon
    Emit-Candidate $_.UninstallString
    $install = [string]$_.InstallLocation
    if (-not [string]::IsNullOrWhiteSpace($install)) {{
      foreach ($exe in $exeNames) {{
        Emit-Candidate (Join-Path $install $exe)
      }}
    }}
  }}
}}

Write-Output 'STAGE:CLASSES'
$classRoots=@('HKCU:\Software\Classes','HKLM:\Software\Classes')
foreach ($protocol in $protocolNames) {{
  if ([string]::IsNullOrWhiteSpace($protocol)) {{ continue }}
  foreach ($classRoot in $classRoots) {{
    $commandPath = Join-Path (Join-Path $classRoot $protocol) 'shell\open\command'
    Emit-Candidate ((Get-ItemProperty -Path $commandPath -ErrorAction SilentlyContinue).'(default)')
  }}
}}

Write-Output 'STAGE:SHORTCUTS'
$shortcutRoots=@(
  "$env:ProgramData\Microsoft\Windows\Start Menu\Programs",
  "$env:APPDATA\Microsoft\Windows\Start Menu\Programs",
  "$env:USERPROFILE\Desktop",
  "$env:PUBLIC\Desktop"
)
$shell = $null
try {{ $shell = New-Object -ComObject WScript.Shell }} catch {{}}
if ($shell) {{
  foreach ($root in $shortcutRoots) {{
    if (-not (Test-Path -LiteralPath $root)) {{ continue }}
    Get-ChildItem -Path $root -Filter *.lnk -Recurse -ErrorAction SilentlyContinue | ForEach-Object {{
      try {{
        $shortcut = $shell.CreateShortcut($_.FullName)
        Emit-Candidate $shortcut.TargetPath
      }} catch {{}}
    }}
  }}
}}

Write-Output 'STAGE:COMMANDS'
foreach ($commandName in $commandNames) {{
  if ([string]::IsNullOrWhiteSpace($commandName)) {{ continue }}
  $command = Get-Command $commandName -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($command) {{
    Emit-Candidate $command.Source
    Emit-Candidate $command.Definition
  }}
}}
Write-Output 'STAGE:END'
exit 0
"#
    );

    let output = match powershell_output(&["-Command", &script]) {
        Ok(value) => value,
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Path Detect] {} PowerShell detect failed: {}",
                app_label, err
            ));
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        crate::modules::logger::log_warn(&format!(
            "[Path Detect] {} PowerShell command failed(-Command): status={}, stdout_head={}, stderr_head={}",
            app_label,
            output.status,
            stdout.chars().take(400).collect::<String>(),
            stderr.chars().take(400).collect::<String>()
        ));
        return None;
    }

    parse_windows_exec_candidates(app_label, exe_names, display_keywords, output)
}

fn should_detach_child() -> bool {
    if let Ok(value) = std::env::var("COCKPIT_CHILD_LOGS") {
        let lowered = value.trim().to_lowercase();
        if matches!(lowered.as_str(), "1" | "true" | "yes" | "on") {
            return false;
        }
    }
    if let Ok(value) = std::env::var("COCKPIT_CHILD_DETACH") {
        let lowered = value.trim().to_lowercase();
        if matches!(lowered.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
    }
    true
}

#[cfg(target_os = "macos")]
fn sanitize_macos_gui_launch_env(cmd: &mut Command) {
    // Avoid inheriting Cockpit bundle identity into child GUI apps.
    cmd.env_remove("__CFBundleIdentifier");
    cmd.env_remove("XPC_SERVICE_NAME");
}

fn managed_proxy_env_pairs() -> Vec<(&'static str, String)> {
    let config = config::get_user_config();
    if !config.global_proxy_enabled {
        return Vec::new();
    }

    let proxy_url = config.global_proxy_url.trim();
    if proxy_url.is_empty() {
        crate::modules::logger::log_warn("[Proxy] 全局代理已启用，但代理地址为空，跳过注入");
        return Vec::new();
    }

    let mut pairs = vec![
        ("http_proxy", proxy_url.to_string()),
        ("https_proxy", proxy_url.to_string()),
        ("HTTP_PROXY", proxy_url.to_string()),
        ("HTTPS_PROXY", proxy_url.to_string()),
        ("all_proxy", proxy_url.to_string()),
        ("ALL_PROXY", proxy_url.to_string()),
    ];

    let no_proxy = config.global_proxy_no_proxy.trim();
    if !no_proxy.is_empty() {
        pairs.push(("no_proxy", no_proxy.to_string()));
        pairs.push(("NO_PROXY", no_proxy.to_string()));
    }

    pairs
}

fn log_managed_proxy_injection(mode: &str, cmd: &Command, pairs: &[(&'static str, String)]) {
    if pairs.is_empty() {
        return;
    }

    let proxy_url = pairs
        .iter()
        .find_map(|(key, value)| (*key == "http_proxy").then_some(value.as_str()))
        .unwrap_or("");
    let no_proxy = pairs
        .iter()
        .find_map(|(key, value)| (*key == "no_proxy").then_some(value.as_str()))
        .unwrap_or("");
    let keys = pairs
        .iter()
        .map(|(key, _)| *key)
        .collect::<Vec<&str>>()
        .join(",");

    crate::modules::logger::log_info(&format!(
        "[Proxy] 已注入全局代理 mode={} program={} proxy_url={} no_proxy={} keys={}",
        mode,
        cmd.get_program().to_string_lossy(),
        proxy_url,
        if no_proxy.is_empty() {
            "<empty>"
        } else {
            no_proxy
        },
        keys
    ));
}

pub fn apply_managed_proxy_env_to_command(cmd: &mut Command) {
    let pairs = managed_proxy_env_pairs();
    if pairs.is_empty() {
        return;
    }
    log_managed_proxy_injection("env", cmd, &pairs);
    for (key, value) in pairs {
        cmd.env(key, value);
    }
}

#[cfg(target_os = "macos")]
pub fn append_managed_proxy_env_to_open_args(cmd: &mut Command) {
    let pairs = managed_proxy_env_pairs();
    if pairs.is_empty() {
        return;
    }
    log_managed_proxy_injection("open-arg", cmd, &pairs);
    for (key, value) in pairs {
        cmd.arg("--env").arg(format!("{}={}", key, value));
    }
}

#[cfg(not(target_os = "macos"))]
pub fn append_managed_proxy_env_to_open_args(_cmd: &mut Command) {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn spawn_detached_unix(cmd: &mut Command) -> Result<Child, String> {
    use std::os::unix::process::CommandExt;
    if !should_detach_child() {
        return spawn_command_with_trace(cmd).map_err(|e| format!("启动失败: {}", e));
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    spawn_command_with_trace(cmd).map_err(|e| format!("启动失败: {}", e))
}

fn normalize_custom_path(value: Option<&str>) -> Option<String> {
    let trimmed = value.unwrap_or("").trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

const APP_PATH_NOT_FOUND_PREFIX: &str = "APP_PATH_NOT_FOUND:";

fn app_path_missing_error(app: &str) -> String {
    format!("{}{}", APP_PATH_NOT_FOUND_PREFIX, app)
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root(path: &Path) -> Option<String> {
    let path_str = path.to_string_lossy();
    if let Some(app_idx) = path_str.find(".app") {
        return Some(path_str[..app_idx + 4].to_string());
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_macos_exec_path(path_str: &str, binary_name: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let exec_path = std::path::PathBuf::from(&app_root)
            .join("Contents")
            .join("MacOS")
            .join(binary_name);
        if exec_path.exists() {
            return Some(exec_path);
        }
    }
    if path.exists() {
        return Some(path);
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn resolve_macos_exec_path(path_str: &str, _binary_name: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn app_path_matches_snapshot(current: &str, expected: &str) -> bool {
    current.trim() == expected.trim()
}

fn update_app_path_in_config(app: &str, path: &Path, expected_current: &str) {
    let normalized = {
        #[cfg(target_os = "macos")]
        {
            normalize_macos_app_root(path).unwrap_or_else(|| path.to_string_lossy().to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            path.to_string_lossy().to_string()
        }
    };
    let _ = config::patch_user_config(move |current| {
        let target = match app {
            "antigravity" => &mut current.antigravity_app_path,
            "codex" => &mut current.codex_app_path,
            "zed" => &mut current.zed_app_path,
            "vscode" => &mut current.vscode_app_path,
            "opencode" => &mut current.opencode_app_path,
            "codebuddy" => &mut current.codebuddy_app_path,
            "codebuddy_cn" => &mut current.codebuddy_cn_app_path,
            "qoder" => &mut current.qoder_app_path,
            "trae" => &mut current.trae_app_path,
            "workbuddy" => &mut current.workbuddy_app_path,
            _ => return Ok(()),
        };
        if app_path_matches_snapshot(target, expected_current) && *target != normalized {
            *target = normalized;
        }
        Ok(())
    });
}

#[cfg(test)]
mod app_path_config_guard_tests {
    use super::app_path_matches_snapshot;

    #[test]
    fn detected_path_only_replaces_the_snapshot_it_was_detected_for() {
        assert!(app_path_matches_snapshot("", ""));
        assert!(app_path_matches_snapshot(" /old/path ", "/old/path"));
        assert!(!app_path_matches_snapshot("/manual/path", ""));
        assert!(!app_path_matches_snapshot("/new/path", "/old/path"));
    }
}

#[cfg(target_os = "macos")]
fn is_legacy_antigravity_macos_path(path: &str) -> bool {
    let lower = path.trim().to_ascii_lowercase();
    lower.contains("/applications/antigravity.app")
        || lower.ends_with("/antigravity.app")
        || lower.contains("/antigravity.app/contents/")
}

#[cfg(target_os = "macos")]
fn resolve_macos_app_root_from_config(app: &str) -> Option<String> {
    let current = config::get_user_config();
    let raw = match app {
        "antigravity" => current.antigravity_app_path,
        "codex" => current.codex_app_path,
        "zed" => current.zed_app_path,
        "vscode" => current.vscode_app_path,
        "codebuddy" => current.codebuddy_app_path,
        "codebuddy_cn" => current.codebuddy_cn_app_path,
        _ => String::new(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = std::path::Path::new(trimmed);
    let app_root = normalize_macos_app_root(path)?;
    if std::path::Path::new(&app_root).exists() {
        return Some(app_root);
    }
    None
}

/// 从已解析的可执行文件路径中提取 .app 根路径
#[cfg(target_os = "macos")]
fn resolve_macos_app_root_from_launch_path(launch_path: &std::path::Path) -> Option<String> {
    let app_root = normalize_macos_app_root(launch_path)?;
    if std::path::Path::new(&app_root).exists() {
        Some(app_root)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn spawn_open_app_with_options(
    app_root: &str,
    args: &[String],
    force_new_instance: bool,
) -> Result<u32, String> {
    let mut cmd = Command::new("open");
    sanitize_macos_gui_launch_env(&mut cmd);
    append_managed_proxy_env_to_open_args(&mut cmd);
    if force_new_instance {
        cmd.arg("-n");
    }
    cmd.arg("-a").arg(app_root);
    if !args.is_empty() {
        cmd.arg("--args");
        for arg in args {
            if !arg.trim().is_empty() {
                cmd.arg(arg);
            }
        }
    }
    let child = spawn_detached_unix(&mut cmd).map_err(|e| format!("启动失败: {}", e))?;
    Ok(child.id())
}

fn find_antigravity_process_exe() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let output = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let _pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let lower = cmdline.to_lowercase();
            if !lower.contains(ANTIGRAVITY_APP_CONTENTS_MARKER) {
                continue;
            }
            if lower.contains("antigravity tools.app/contents/") {
                continue;
            }
            if lower.contains("--type=") || lower.contains("crashpad_handler") {
                continue;
            }
            if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                return Some(std::path::PathBuf::from(exe));
            }
        }
        return None;
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
            let is_antigravity = is_windows_antigravity_main_executable(&name, &exe_path);
            #[cfg(target_os = "linux")]
            let is_antigravity = (name.contains("antigravity-ide")
                || exe_path.contains("/antigravity-ide"))
                && !name.contains("tools")
                && !exe_path.contains("tools");

            if is_antigravity && !is_helper {
                if let Some(exe) = process.exe() {
                    return Some(exe.to_path_buf());
                }
            }
        }

        None
    }
}

fn find_vscode_process_exe() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // Use ps to avoid sysinfo TCC dialogs on macOS
        let output = Command::new("ps")
            .args(["-axww", "-o", "pid=,command="])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
            let _pid_str = parts.next().unwrap_or("").trim();
            let cmdline = parts.next().unwrap_or("").trim();
            let lower = cmdline.to_lowercase();
            if !lower.contains("visual studio code.app/contents/macos/") {
                continue;
            }
            if lower.contains("--type=") || lower.contains("crashpad_handler") {
                continue;
            }
            if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
                return Some(std::path::PathBuf::from(exe));
            }
        }
        return None;
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
                || name.contains("renderer")
                || name.contains("gpu")
                || name.contains("crashpad")
                || name.contains("utility")
                || name.contains("audio")
                || name.contains("sandbox");

            #[cfg(target_os = "windows")]
            let is_vscode = (name == "code.exe" || exe_path.ends_with("\\code.exe")) && !is_helper;
            #[cfg(target_os = "linux")]
            let is_vscode = (name == "code" || exe_path.ends_with("/code")) && !is_helper;

            if is_vscode {
                if let Some(exe) = process.exe() {
                    return Some(exe.to_path_buf());
                }
            }
        }

        None
    }
}

#[cfg(target_os = "macos")]
fn find_codex_process_exe() -> Option<std::path::PathBuf> {
    // Use ps to avoid sysinfo TCC dialogs on macOS
    let output = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
        let _pid_str = parts.next().unwrap_or("").trim();
        let cmdline = parts.next().unwrap_or("").trim();
        let lower = cmdline.to_lowercase();
        if !is_codex_macos_main_process_command_line(&lower) {
            continue;
        }
        if lower.contains("--type=") || lower.contains("crashpad_handler") {
            continue;
        }
        if let Some(exe) = extract_macos_exe_from_cmdline(cmdline) {
            return Some(std::path::PathBuf::from(exe));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn is_codex_macos_main_process_command_line(lower_cmdline: &str) -> bool {
    lower_cmdline.contains("chatgpt.app/contents/macos/chatgpt")
        || lower_cmdline.contains("codex.app/contents/macos/codex")
}

#[cfg(target_os = "macos")]
fn resolve_codex_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "ChatGPT")
        .or_else(|| resolve_macos_exec_path(path_str, "Codex"))
}

#[cfg(target_os = "windows")]
fn is_windows_antigravity_main_executable(name: &str, exe_path: &str) -> bool {
    (name == "antigravity ide.exe"
        || name == "antigravity.exe"
        || exe_path.ends_with("\\antigravity ide.exe")
        || exe_path.ends_with("\\antigravity.exe"))
        && !exe_path.contains("crashpad")
}

fn detect_antigravity_exec_path() -> Option<std::path::PathBuf> {
    if let Some(path) = find_antigravity_process_exe() {
        return Some(path);
    }

    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(ANTIGRAVITY_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Antigravity")
                    .join("Antigravity.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Antigravity")
                    .join("Electron.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(local_appdata)
                    .join("Programs")
                    .join("Antigravity IDE")
                    .join("Antigravity IDE.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(&program_files)
                    .join("Antigravity")
                    .join("Antigravity.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&program_files)
                    .join("Antigravity")
                    .join("Electron.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Antigravity IDE")
                    .join("Antigravity IDE.exe"),
            );
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            candidates.push(
                std::path::PathBuf::from(&program_files_x86)
                    .join("Antigravity")
                    .join("Antigravity.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&program_files_x86)
                    .join("Antigravity")
                    .join("Electron.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(program_files_x86)
                    .join("Antigravity IDE")
                    .join("Antigravity IDE.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "antigravity",
            &[
                "Antigravity.exe",
                "antigravity.exe",
                "Antigravity IDE.exe",
                "antigravity-ide.exe",
                "Electron.exe",
            ],
            &["antigravity", "antigravity ide"],
            &["antigravity", "antigravity ide"],
            &["antigravity ide", "antigravity"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/antigravity-ide",
            "/opt/antigravity-ide/antigravity-ide",
            "/usr/share/antigravity-ide/antigravity-ide",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let user_local = home.join(".local/bin/antigravity-ide");
            if user_local.exists() {
                return Some(user_local);
            }
        }
    }

    None
}

fn detect_vscode_exec_path() -> Option<std::path::PathBuf> {
    if let Some(path) = find_vscode_process_exe() {
        return Some(path);
    }

    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(VSCODE_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("VSCode")
                    .join("Code.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            candidates.push(
                std::path::PathBuf::from(program_files_x86)
                    .join("Microsoft VS Code")
                    .join("Code.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "vscode",
            &["Code.exe", "Code - Insiders.exe"],
            &["code", "code-insiders"],
            &["vscode", "vscode-insiders"],
            &["visual studio code", "vs code", "vscode"],
        ) {
            return Some(path);
        }
        if let Some(path) = detect_vscode_exec_path_by_registry() {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/code",
            "/snap/bin/code",
            "/var/lib/flatpak/exports/bin/com.visualstudio.code",
            "/usr/local/bin/code",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let user_local = home.join(".local/bin/code");
            if user_local.exists() {
                return Some(user_local);
            }
        }
    }

    None
}

fn detect_codebuddy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/CodeBuddy.app/Contents/MacOS/CodeBuddy",
            "/Applications/CodeBuddy.app/Contents/MacOS/Electron",
            "/Applications/CodeBuddy.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy")
                    .join("CodeBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("CodeBuddy")
                    .join("CodeBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/codebuddy",
            "/usr/local/bin/codebuddy",
            "/opt/codebuddy/codebuddy",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_codebuddy_cn_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/CodeBuddy CN.app/Contents/MacOS/CodeBuddy CN",
            "/Applications/CodeBuddy CN.app/Contents/MacOS/CodeBuddy",
            "/Applications/CodeBuddy CN.app/Contents/MacOS/Electron",
            "/Applications/CodeBuddy CN.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy CN")
                    .join("CodeBuddy CN.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("CodeBuddy CN")
                    .join("CodeBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(&program_files)
                    .join("CodeBuddy CN")
                    .join("CodeBuddy CN.exe"),
            );
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("CodeBuddy CN")
                    .join("CodeBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/codebuddy-cn",
            "/usr/local/bin/codebuddy-cn",
            "/opt/codebuddy-cn/codebuddy-cn",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_qoder_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Qoder.app/Contents/MacOS/Qoder",
            "/Applications/Qoder.app/Contents/MacOS/Electron",
            "/Applications/Qoder.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Qoder")
                    .join("Qoder.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Qoder")
                    .join("Qoder.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/qoder", "/usr/local/bin/qoder", "/opt/qoder/qoder"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_zed_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Zed.app/Contents/MacOS/zed",
            "/Applications/Zed.app",
            "/usr/local/bin/zed",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Zed")
                    .join("Zed.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Zed")
                    .join("Zed.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/zed", "/usr/local/bin/zed", "/opt/zed/zed"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_trae_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Trae.app/Contents/MacOS/Trae",
            "/Applications/Trae.app/Contents/MacOS/Electron",
            "/Applications/Trae.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("Trae")
                    .join("Trae.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("Trae")
                    .join("Trae.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/trae", "/usr/local/bin/trae", "/opt/trae/trae"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn detect_workbuddy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/WorkBuddy.app/Contents/MacOS/WorkBuddy",
            "/Applications/WorkBuddy.app/Contents/MacOS/Electron",
            "/Applications/WorkBuddy.app",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(&local_appdata)
                    .join("Programs")
                    .join("WorkBuddy")
                    .join("WorkBuddy.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("WorkBuddy")
                    .join("WorkBuddy.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/workbuddy",
            "/usr/local/bin/workbuddy",
            "/opt/workbuddy/workbuddy",
        ];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(target_os = "macos")]
fn resolve_codebuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["CodeBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("codebuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_codebuddy_cn_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["CodeBuddy CN", "CodeBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("codebuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_qoder_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["Qoder", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("qoder") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_zed_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["zed", "Zed"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name == "zed" || file_name.contains("zed") {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_trae_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["Trae", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("trae") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_workbuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = std::path::PathBuf::from(&app_root);
        let macos_dir = app_root_path.join("Contents").join("MacOS");

        for binary_name in ["WorkBuddy", "Electron"] {
            let candidate = macos_dir.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        if let Ok(entries) = std::fs::read_dir(&macos_dir) {
            let mut fallback: Option<std::path::PathBuf> = None;
            for entry in entries.flatten() {
                let candidate = entry.path();
                if !candidate.is_file() {
                    continue;
                }
                let file_name = candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("crashpad") || file_name.contains("helper") {
                    continue;
                }
                if file_name.contains("workbuddy") || file_name == "electron" {
                    return Some(candidate);
                }
                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
            if let Some(candidate) = fallback {
                return Some(candidate);
            }
        }
    }

    if path.is_file() {
        return Some(path);
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn resolve_qoder_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "Qoder")
}

#[cfg(not(target_os = "macos"))]
fn resolve_zed_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "zed")
}

#[cfg(not(target_os = "macos"))]
fn resolve_trae_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "Trae")
}

#[cfg(not(target_os = "macos"))]
fn resolve_workbuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "WorkBuddy")
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_cn_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "CodeBuddy CN")
}

#[cfg(not(target_os = "macos"))]
fn resolve_codebuddy_macos_exec_path(path_str: &str) -> Option<std::path::PathBuf> {
    resolve_macos_exec_path(path_str, "CodeBuddy")
}

#[cfg(target_os = "windows")]
fn compare_windows_store_version(left: &[u32], right: &[u32]) -> std::cmp::Ordering {
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let left_part = *left.get(idx).unwrap_or(&0);
        let right_part = *right.get(idx).unwrap_or(&0);
        match left_part.cmp(&right_part) {
            std::cmp::Ordering::Equal => continue,
            non_eq => return non_eq,
        }
    }
    std::cmp::Ordering::Equal
}

#[cfg(target_os = "windows")]
fn parse_codex_store_version_from_dir_name(dir_name: &str) -> Option<Vec<u32>> {
    let lower = dir_name.to_ascii_lowercase();
    let prefix = [
        "openai.chatgpt_",
        "openai.chatgpt-desktop_",
        "openai.codex_",
    ]
    .iter()
    .find(|prefix| lower.starts_with(**prefix))?;
    let suffix = dir_name.get(prefix.len()..)?;
    let version_part = suffix.split('_').next()?.trim();
    if version_part.is_empty() {
        return None;
    }
    let mut version: Vec<u32> = Vec::new();
    for part in version_part.split('.') {
        if part.is_empty() {
            return None;
        }
        version.push(part.parse::<u32>().ok()?);
    }
    if version.is_empty() {
        return None;
    }
    Some(version)
}

#[cfg(target_os = "windows")]
fn find_codex_windows_app_main_exe(app_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    for exe_name in ["ChatGPT.exe", "Codex.exe"] {
        let candidate = app_dir.join(exe_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_exec_path_by_windowsapps_scan() -> Option<std::path::PathBuf> {
    let mut best: Option<(Vec<u32>, std::path::PathBuf)> = None;

    for drive in b'A'..=b'Z' {
        let drive_letter = drive as char;
        let windows_apps_root = if drive_letter == 'C' {
            format!(r"{}:\Program Files\WindowsApps", drive_letter)
        } else {
            format!(r"{}:\WindowsApps", drive_letter)
        };
        let root_path = std::path::PathBuf::from(&windows_apps_root);
        if !root_path.exists() {
            continue;
        }

        let entries = match std::fs::read_dir(&root_path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !file_type.is_dir() {
                continue;
            }

            let dir_name = entry.file_name();
            let dir_name = dir_name.to_string_lossy();
            let Some(version) = parse_codex_store_version_from_dir_name(&dir_name) else {
                continue;
            };

            let candidate = match find_codex_windows_app_main_exe(&entry.path().join("app")) {
                Some(path) => path,
                None => continue,
            };

            let replace = match &best {
                None => true,
                Some((best_version, _)) => {
                    compare_windows_store_version(&version, best_version).is_gt()
                }
            };
            if replace {
                best = Some((version, candidate));
            }
        }
    }

    if let Some((_, path)) = best {
        crate::modules::logger::log_info(&format!(
            "[Path Detect] codex windowsapps scan hit: {}",
            path.to_string_lossy()
        ));
        return Some(path);
    }

    None
}

#[cfg(target_os = "windows")]
fn detect_codex_exec_path_by_appx_install_location() -> Option<std::path::PathBuf> {
    let script = r#"$names = @('OpenAI.ChatGPT', 'OpenAI.ChatGPT-Desktop', 'OpenAI.Codex')
$pkg = $names |
  ForEach-Object { Get-AppxPackage -Name $_ -ErrorAction SilentlyContinue } |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
}
if ($pkg -and -not [string]::IsNullOrWhiteSpace($pkg.InstallLocation)) {
  Write-Output ([string]$pkg.InstallLocation.Trim())
}"#;

    let output = powershell_output(&["-Command", script]).ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let install_location = line.trim().trim_matches('"');
        if install_location.is_empty() {
            continue;
        }
        let Some(candidate) = find_codex_windows_app_main_exe(
            &std::path::PathBuf::from(install_location).join("app"),
        ) else {
            continue;
        };
        if candidate.exists() {
            crate::modules::logger::log_info(&format!(
                "[Path Detect] codex appx install hit: {}",
                candidate.to_string_lossy()
            ));
            return Some(candidate);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id_by_startapps() -> Option<String> {
    let script = r#"$entry = Get-StartApps |
  Where-Object {
    $_.AppID -like 'OpenAI.ChatGPT*' -or
    $_.AppID -like 'OpenAI.Codex_*' -or
    $_.Name -like 'ChatGPT*' -or
    $_.Name -like 'Codex*'
  } |
  Sort-Object @{ Expression = { if ($_.AppID -like 'OpenAI.ChatGPT*' -or $_.Name -like 'ChatGPT*') { 0 } else { 1 } } }, Name |
  Select-Object -First 1
if ($entry -and -not [string]::IsNullOrWhiteSpace($entry.AppID)) {
  Write-Output ([string]$entry.AppID.Trim())
}"#;

    let output = powershell_output(&["-Command", script]).ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let app_user_model_id = line.trim().trim_matches('"');
        if !app_user_model_id.is_empty() {
            return Some(app_user_model_id.to_string());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id_by_appx_fallback() -> Option<String> {
    let script = r#"$names = @('OpenAI.ChatGPT', 'OpenAI.ChatGPT-Desktop', 'OpenAI.Codex')
$pkg = $names |
  ForEach-Object { Get-AppxPackage -Name $_ -ErrorAction SilentlyContinue } |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object -Property Version -Descending |
  Select-Object -First 1
}
if ($pkg -and -not [string]::IsNullOrWhiteSpace($pkg.PackageFamilyName)) {
  Write-Output ([string]($pkg.PackageFamilyName.Trim() + '!App'))
}"#;

    let output = powershell_output(&["-Command", script]).ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let app_user_model_id = line.trim().trim_matches('"');
        if !app_user_model_id.is_empty() {
            return Some(app_user_model_id.to_string());
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn detect_codex_store_app_user_model_id() -> Option<String> {
    if let Some(app_user_model_id) = detect_codex_store_app_user_model_id_by_startapps() {
        crate::modules::logger::log_info(&format!(
            "[Codex Store] StartApps 命中 AppUserModelId: {}",
            app_user_model_id
        ));
        return Some(app_user_model_id);
    }
    if let Some(app_user_model_id) = detect_codex_store_app_user_model_id_by_appx_fallback() {
        crate::modules::logger::log_info(&format!(
            "[Codex Store] Appx fallback 命中 AppUserModelId: {}",
            app_user_model_id
        ));
        return Some(app_user_model_id);
    }
    None
}

#[cfg(target_os = "windows")]
fn launch_codex_via_store_app_user_model_id(app_user_model_id: &str) -> Result<(), String> {
    let app_user_model_id = app_user_model_id.trim();
    if app_user_model_id.is_empty() {
        return Err("Codex AppUserModelId 为空".to_string());
    }

    let escaped = escape_powershell_single_quoted(app_user_model_id);
    let script = format!(
        r#"$appId='{escaped}';
$target='shell:AppsFolder\' + $appId
Start-Process -FilePath $target -ErrorAction Stop | Out-Null"#
    );

    let output = powershell_output(&["-Command", &script])
        .map_err(|e| format!("系统入口启动调用失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_head = stderr.trim().chars().take(400).collect::<String>();
        return Err(format!(
            "系统入口启动失败: status={}, stderr={}",
            output.status,
            if stderr_head.is_empty() {
                "<empty>".to_string()
            } else {
                stderr_head
            }
        ));
    }
    Ok(())
}

fn detect_codex_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if let Some(path) = find_codex_process_exe() {
            return Some(path);
        }
        let path = std::path::PathBuf::from(CODEX_CHATGPT_APP_PATH);
        if path.exists() {
            return Some(path);
        }
        let path = std::path::PathBuf::from(CODEX_APP_PATH);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = detect_codex_exec_path_by_windowsapps_scan() {
            return Some(path);
        }
        if let Some(path) = detect_codex_exec_path_by_appx_install_location() {
            return Some(path);
        }
    }

    None
}

#[cfg(any(test, target_os = "macos"))]
fn normalized_macos_codex_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

#[cfg(any(test, target_os = "macos"))]
fn is_official_legacy_codex_macos_path(path: &Path) -> bool {
    matches!(
        normalized_macos_codex_path_text(path).as_str(),
        "/applications/codex.app" | "/applications/codex.app/contents/macos/codex"
    )
}

#[cfg(any(test, target_os = "macos"))]
fn is_official_chatgpt_macos_path(path: &Path) -> bool {
    matches!(
        normalized_macos_codex_path_text(path).as_str(),
        "/applications/chatgpt.app" | "/applications/chatgpt.app/contents/macos/chatgpt"
    )
}

#[cfg(any(test, target_os = "macos"))]
fn should_migrate_legacy_codex_launch_path(current: &Path, detected: &Path) -> bool {
    is_official_legacy_codex_macos_path(current) && is_official_chatgpt_macos_path(detected)
}

#[cfg(target_os = "macos")]
fn migrate_legacy_codex_launch_path(custom_path: &str) -> Option<std::path::PathBuf> {
    let current_path = std::path::PathBuf::from(custom_path);
    let detected = detect_codex_exec_path()?;
    if !should_migrate_legacy_codex_launch_path(&current_path, &detected) {
        return None;
    }

    update_app_path_in_config("codex", &detected, custom_path);
    crate::modules::logger::log_info(&format!(
        "[Path Detect] migrated legacy Codex launch path to ChatGPT: old={} new={}",
        current_path.to_string_lossy(),
        detected.to_string_lossy()
    ));
    Some(detected)
}

