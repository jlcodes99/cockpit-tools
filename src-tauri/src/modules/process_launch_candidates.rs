// Process 模块：Process launch candidate discovery and command execution helpers。
// 通过 include! 保持原 modules::process 作用域和平台分支行为。
use crate::modules::config;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
const ANTIGRAVITY_LEGACY_APP_PATH: &str =
    "/Applications/Antigravity.app/Contents/MacOS/Antigravity";
#[cfg(target_os = "macos")]
const ANTIGRAVITY_APP_CONTENTS_MARKER: &str = "antigravity ide.app/contents/";
#[cfg(target_os = "macos")]
const ANTIGRAVITY_LEGACY_APP_CONTENTS_MARKER: &str = "antigravity.app/contents/";
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
#[cfg(target_os = "windows")]
static CODEX_STORE_APP_USER_MODEL_ID_CACHE: std::sync::OnceLock<String> =
    std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize)]
pub struct AppLaunchCandidate {
    pub target_type: String,
    pub label: String,
    pub target: String,
    pub source: String,
    pub supports_multi_instance: bool,
}

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
    let program = cmd.get_program().to_string_lossy().into_owned();
    let spawn_guard = crate::modules::app_lifecycle::acquire_process_spawn_guard(&program)?;
    let preview = format_command_preview(cmd);
    log_command_trace_exec(&preview);
    let start = Instant::now();
    let result = cmd.spawn();
    drop(spawn_guard);
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
    let spawn_guard = crate::modules::app_lifecycle::acquire_process_spawn_guard("PowerShell")?;
    let mut command = build_powershell_command(args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let preview = format_command_preview(&command);
    log_command_trace_exec(&preview);
    let start = Instant::now();
    let child = command.spawn();
    drop(spawn_guard);
    let result = child.and_then(Child::wait_with_output);
    log_command_trace_result(&preview, &result, start.elapsed());
    result
}

#[cfg(target_os = "windows")]
fn powershell_output_with_timeout(
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::{Error, ErrorKind, Read};

    let spawn_guard = crate::modules::app_lifecycle::acquire_process_spawn_guard("PowerShell")?;
    let mut command = build_powershell_command(args);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let preview = format_command_preview(&command);
    log_command_trace_exec(&preview);
    let child = command.spawn();
    drop(spawn_guard);
    let mut child = match child {
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

#[cfg(any(test, target_os = "windows"))]
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

    // The legacy Codex and current ChatGPT clients share this scanner. Do not
    // accept helper executables whose paths merely contain one of those names.
    if exe_names_lower.contains("chatgpt.exe") && exe_names_lower.contains("codex.exe") {
        return None;
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

#[cfg(any(test, target_os = "windows"))]
fn is_codex_embedded_backend_executable(path: &std::path::Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\windowsapps\\") && normalized.ends_with("\\app\\resources\\codex.exe")
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
#[derive(Clone, Copy)]
struct WindowsAppLaunchSignature {
    label: &'static str,
    exe_names: &'static [&'static str],
    command_names: &'static [&'static str],
    protocol_names: &'static [&'static str],
    display_keywords: &'static [&'static str],
    common_paths: &'static [&'static str],
    supports_multi_instance: bool,
}

#[cfg(target_os = "windows")]
const WINDOWS_RUNNING_APP_CANDIDATE_LIMIT: usize = 20;

#[cfg(target_os = "windows")]
fn windows_app_launch_signature(app: &str) -> Option<WindowsAppLaunchSignature> {
    match app {
        "antigravity" | "antigravity_ide" => Some(WindowsAppLaunchSignature {
            label: "Antigravity IDE",
            exe_names: &["Antigravity IDE.exe", "antigravity-ide.exe"],
            command_names: &["antigravity-ide"],
            protocol_names: &["antigravity-ide", "antigravity ide"],
            display_keywords: &["antigravity ide", "antigravity-ide"],
            common_paths: &[
                "Antigravity IDE\\Antigravity IDE.exe",
                "Antigravity IDE\\antigravity-ide.exe",
            ],
            supports_multi_instance: true,
        }),
        "antigravity_legacy" => Some(WindowsAppLaunchSignature {
            label: "Antigravity",
            exe_names: &["Antigravity.exe", "antigravity.exe", "Electron.exe"],
            command_names: &["antigravity"],
            protocol_names: &["antigravity"],
            display_keywords: &["antigravity"],
            common_paths: &["Antigravity\\Antigravity.exe", "Antigravity\\Electron.exe"],
            supports_multi_instance: true,
        }),
        "codex" => Some(WindowsAppLaunchSignature {
            label: "ChatGPT / Codex",
            exe_names: &["ChatGPT.exe", "Codex.exe"],
            command_names: &["chatgpt", "codex"],
            protocol_names: &["chatgpt", "codex"],
            display_keywords: &["chatgpt", "codex", "openai chatgpt", "openai codex"],
            common_paths: &[
                "ChatGPT\\ChatGPT.exe",
                "OpenAI ChatGPT\\ChatGPT.exe",
                "Codex\\Codex.exe",
                "OpenAI Codex\\Codex.exe",
            ],
            supports_multi_instance: true,
        }),
        "claude" => Some(WindowsAppLaunchSignature {
            label: "Claude Desktop",
            exe_names: &["Claude.exe"],
            command_names: &["claude"],
            protocol_names: &["claude"],
            display_keywords: &["claude", "anthropic claude"],
            common_paths: &[r"Claude\Claude.exe", r"AnthropicClaude\Claude.exe"],
            supports_multi_instance: true,
        }),
        "vscode" => Some(WindowsAppLaunchSignature {
            label: "Visual Studio Code",
            exe_names: &["Code.exe", "Code - Insiders.exe"],
            command_names: &["code", "code-insiders"],
            protocol_names: &["vscode", "vscode-insiders"],
            display_keywords: &["visual studio code", "vs code", "vscode"],
            common_paths: &[
                "Microsoft VS Code\\Code.exe",
                "VSCode\\Code.exe",
                "Microsoft VS Code Insiders\\Code - Insiders.exe",
            ],
            supports_multi_instance: true,
        }),
        "windsurf" => Some(WindowsAppLaunchSignature {
            // Windsurf 已重命名为 Devin；保留旧路径关键字以兼容旧安装。
            label: "Devin",
            exe_names: &["Devin.exe", "Windsurf.exe", "Electron.exe"],
            command_names: &["devin", "windsurf"],
            protocol_names: &["devin", "windsurf"],
            display_keywords: &["devin", "windsurf", "codeium", "exafunction"],
            common_paths: &[
                "Devin\\Devin.exe",
                "Devin\\Electron.exe",
                "Windsurf\\Windsurf.exe",
                "Windsurf\\Electron.exe",
            ],
            supports_multi_instance: true,
        }),
        "kiro" => Some(WindowsAppLaunchSignature {
            label: "Kiro",
            exe_names: &["Kiro.exe", "Electron.exe"],
            command_names: &["kiro"],
            protocol_names: &["kiro"],
            display_keywords: &["kiro"],
            common_paths: &["Kiro\\Kiro.exe", "Kiro\\Electron.exe"],
            supports_multi_instance: true,
        }),
        "cursor" => Some(WindowsAppLaunchSignature {
            label: "Cursor",
            exe_names: &["Cursor.exe", "Electron.exe"],
            command_names: &["cursor"],
            protocol_names: &["cursor"],
            display_keywords: &["cursor"],
            common_paths: &["Cursor\\Cursor.exe", "Cursor\\Electron.exe"],
            supports_multi_instance: true,
        }),
        "codebuddy" => Some(WindowsAppLaunchSignature {
            label: "CodeBuddy",
            exe_names: &["CodeBuddy.exe"],
            command_names: &["codebuddy"],
            protocol_names: &["codebuddy"],
            display_keywords: &["codebuddy"],
            common_paths: &["CodeBuddy\\CodeBuddy.exe"],
            supports_multi_instance: true,
        }),
        "codebuddy_cn" => Some(WindowsAppLaunchSignature {
            label: "CodeBuddy CN",
            exe_names: &["CodeBuddy CN.exe", "CodeBuddy.exe"],
            command_names: &["codebuddy-cn", "codebuddy"],
            protocol_names: &["codebuddy-cn", "codebuddy"],
            display_keywords: &["codebuddy cn", "codebuddy"],
            common_paths: &[
                "CodeBuddy CN\\CodeBuddy CN.exe",
                "CodeBuddy CN\\CodeBuddy.exe",
            ],
            supports_multi_instance: true,
        }),
        "qoder" => Some(WindowsAppLaunchSignature {
            label: "Qoder",
            exe_names: &["Qoder.exe"],
            command_names: &["qoder"],
            protocol_names: &["qoder"],
            display_keywords: &["qoder"],
            common_paths: &["Qoder\\Qoder.exe"],
            supports_multi_instance: true,
        }),
        "zcode" => Some(WindowsAppLaunchSignature {
            label: "ZCode",
            exe_names: &["ZCode.exe"],
            command_names: &["zcode"],
            protocol_names: &["zcode"],
            display_keywords: &["zcode", "z.ai"],
            common_paths: &["ZCode\\ZCode.exe"],
            supports_multi_instance: true,
        }),
        "trae" => Some(WindowsAppLaunchSignature {
            label: "Trae",
            exe_names: &["Trae.exe"],
            command_names: &["trae"],
            protocol_names: &["trae"],
            display_keywords: &["trae"],
            common_paths: &["Trae\\Trae.exe"],
            supports_multi_instance: true,
        }),
        "trae_solo" => Some(WindowsAppLaunchSignature {
            label: "TRAE SOLO",
            exe_names: &["TRAE SOLO.exe", "Trae.exe", "Electron.exe"],
            command_names: &["trae-solo", "solo"],
            protocol_names: &["solo"],
            display_keywords: &["trae solo", "solo"],
            common_paths: &[
                "TRAE SOLO\\TRAE SOLO.exe",
                "TRAE SOLO\\Trae.exe",
                "TRAE SOLO\\Electron.exe",
            ],
            supports_multi_instance: true,
        }),
        "trae_cn" => Some(WindowsAppLaunchSignature {
            label: "Trae CN",
            exe_names: &["Trae CN.exe", "Trae.exe", "Electron.exe"],
            command_names: &["trae-cn"],
            protocol_names: &["trae-cn"],
            display_keywords: &["trae cn"],
            common_paths: &[
                "Trae CN\\Trae CN.exe",
                "Trae CN\\Trae.exe",
                "Trae CN\\Electron.exe",
            ],
            supports_multi_instance: true,
        }),
        "trae_solo_cn" => Some(WindowsAppLaunchSignature {
            label: "TRAE SOLO CN",
            exe_names: &["TRAE SOLO CN.exe", "Trae.exe", "Electron.exe"],
            command_names: &["trae-solo-cn", "solo-cn"],
            protocol_names: &["solo-cn"],
            display_keywords: &["trae solo cn", "solo cn"],
            common_paths: &[
                "TRAE SOLO CN\\TRAE SOLO CN.exe",
                "TRAE SOLO CN\\Trae.exe",
                "TRAE SOLO CN\\Electron.exe",
            ],
            supports_multi_instance: true,
        }),
        "workbuddy" => Some(WindowsAppLaunchSignature {
            label: "WorkBuddy",
            exe_names: &["WorkBuddy.exe"],
            command_names: &["workbuddy"],
            protocol_names: &["workbuddy"],
            display_keywords: &["workbuddy"],
            common_paths: &["WorkBuddy\\WorkBuddy.exe"],
            supports_multi_instance: true,
        }),
        "zed" => Some(WindowsAppLaunchSignature {
            label: "Zed",
            exe_names: &["Zed.exe", "zed.exe"],
            command_names: &["zed"],
            protocol_names: &["zed"],
            display_keywords: &["zed"],
            common_paths: &["Zed\\Zed.exe", "Zed\\bin\\zed.exe"],
            supports_multi_instance: true,
        }),
        "opencode" => Some(WindowsAppLaunchSignature {
            label: "OpenCode",
            exe_names: &["OpenCode.exe", "opencode.exe"],
            command_names: &["opencode"],
            protocol_names: &["opencode"],
            display_keywords: &["opencode", "open code"],
            common_paths: &["OpenCode\\OpenCode.exe"],
            supports_multi_instance: true,
        }),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn push_app_launch_candidate(
    candidates: &mut Vec<AppLaunchCandidate>,
    seen: &mut HashSet<String>,
    path: &std::path::Path,
    signature: WindowsAppLaunchSignature,
    source: &str,
) {
    if candidates.len() >= WINDOWS_RUNNING_APP_CANDIDATE_LIMIT || !path.is_file() {
        return;
    }

    let exe_names_lower: HashSet<String> = signature
        .exe_names
        .iter()
        .map(|value| value.to_lowercase())
        .collect();
    let keywords_lower: Vec<String> = signature
        .display_keywords
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    if score_windows_candidate(path, &exe_names_lower, &keywords_lower).is_none() {
        return;
    }

    let normalized_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let target = normalized_path.to_string_lossy().to_string();
    let dedupe_key = target.to_lowercase();
    if !seen.insert(dedupe_key) {
        return;
    }

    let file_name = normalized_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    let label = if file_name.is_empty() {
        signature.label.to_string()
    } else {
        format!("{} ({})", signature.label, file_name)
    };

    candidates.push(AppLaunchCandidate {
        target_type: "exe".to_string(),
        label,
        target,
        source: source.to_string(),
        supports_multi_instance: signature.supports_multi_instance,
    });
}

#[cfg(target_os = "windows")]
fn windows_fixed_drive_roots() -> Vec<std::path::PathBuf> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

    const DRIVE_FIXED: u32 = 3;

    let drive_mask = unsafe { GetLogicalDrives() };
    let mut roots = Vec::new();
    for index in 0..26u32 {
        if drive_mask & (1 << index) == 0 {
            continue;
        }
        let drive = format!("{}:\\", (b'A' + index as u8) as char);
        let wide = drive
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        if unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) } == DRIVE_FIXED {
            roots.push(std::path::PathBuf::from(drive));
        }
    }
    roots
}

#[cfg(target_os = "windows")]
fn normalize_windows_scan_root(raw: &str) -> Option<std::path::PathBuf> {
    let mut value = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    if value.is_empty() {
        return None;
    }
    if value.len() == 2 && value.as_bytes().get(1) == Some(&b':') {
        value.push('\\');
    }
    let path = std::path::PathBuf::from(value);
    path.is_dir().then_some(path)
}

#[cfg(target_os = "windows")]
fn parse_windows_scan_roots(scan_roots: Option<&str>) -> Vec<std::path::PathBuf> {
    let mut seen = HashSet::new();
    let roots: Vec<std::path::PathBuf> = scan_roots
        .unwrap_or("")
        .split(|ch| matches!(ch, '\n' | '\r' | ';' | ','))
        .filter_map(normalize_windows_scan_root)
        .filter(|root| seen.insert(root.to_string_lossy().to_lowercase()))
        .collect();
    if !roots.is_empty() {
        return roots;
    }

    windows_fixed_drive_roots()
        .into_iter()
        .filter(|root| seen.insert(root.to_string_lossy().to_lowercase()))
        .collect()
}

#[cfg(target_os = "windows")]
fn is_windows_drive_root(path: &std::path::Path) -> bool {
    let value = path.to_string_lossy().replace('/', "\\");
    let trimmed = value.trim_end_matches('\\');
    trimmed.len() == 2
        && trimmed.as_bytes().get(1) == Some(&b':')
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
}

#[cfg(target_os = "windows")]
fn expand_windows_scan_roots(roots: Vec<std::path::PathBuf>) -> Vec<std::path::PathBuf> {
    let mut expanded = Vec::new();
    for root in roots {
        if is_windows_drive_root(&root) {
            expanded.push(root.join("Program Files"));
            expanded.push(root.join("Program Files (x86)"));
            expanded.push(root.join("WindowsApps"));
            let users_dir = root.join("Users");
            if let Ok(entries) = std::fs::read_dir(users_dir) {
                for entry in entries.flatten() {
                    expanded.push(entry.path().join("AppData").join("Local").join("Programs"));
                }
            }
        } else {
            expanded.push(root);
        }
    }

    let mut seen = HashSet::new();
    expanded
        .into_iter()
        .filter(|root| root.is_dir())
        .filter(|root| seen.insert(root.to_string_lossy().to_lowercase()))
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_trae_platform_for_app(
    app: &str,
) -> Option<crate::modules::trae_account::TraePlatformKind> {
    crate::modules::trae_account::TraePlatformKind::parse(Some(app)).ok()
}

#[cfg(target_os = "windows")]
fn windows_trae_candidate_matches_platform(
    path: &std::path::Path,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> bool {
    let expected = platform.app_support_dir_name();
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|value| value.eq_ignore_ascii_case(expected))
            .unwrap_or(false)
    })
}

#[cfg(target_os = "windows")]
fn running_app_candidate_matches(
    app: &str,
    path: &std::path::Path,
    signature: WindowsAppLaunchSignature,
) -> bool {
    if app == "codex" && is_codex_embedded_backend_executable(path) {
        return false;
    }

    #[cfg(target_os = "windows")]
    if let Some(platform) = windows_trae_platform_for_app(app) {
        if !windows_trae_candidate_matches_platform(path, platform) {
            return false;
        }
    }

    let exe_names_lower = signature
        .exe_names
        .iter()
        .map(|value| value.to_lowercase())
        .collect();
    let keywords_lower = signature
        .display_keywords
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<String>>();
    score_windows_candidate(path, &exe_names_lower, &keywords_lower).is_some()
}

#[cfg(target_os = "windows")]
fn scan_windows_app_launch_targets(
    app: &str,
    _scan_roots: Option<&str>,
) -> Result<Vec<AppLaunchCandidate>, String> {
    let started_at = Instant::now();
    let Some(signature) = windows_app_launch_signature(app) else {
        return Err("未知应用类型".to_string());
    };

    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut system = System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );

    for process in system.processes().values() {
        let path = process
            .exe()
            .map(std::path::PathBuf::from)
            .or_else(|| process.cmd().first().map(std::path::PathBuf::from));
        let Some(path) = path else {
            continue;
        };
        if !running_app_candidate_matches(app, &path, signature) {
            continue;
        }
        push_app_launch_candidate(
            &mut candidates,
            &mut seen,
            &path,
            signature,
            "running_process",
        );
    }

    candidates.sort_by_key(|candidate| {
        let codex_priority = if app == "codex" {
            let file_name = std::path::Path::new(&candidate.target)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if file_name.eq_ignore_ascii_case("ChatGPT.exe") {
                0
            } else {
                1
            }
        } else {
            0
        };
        (codex_priority, candidate.target.to_ascii_lowercase())
    });
    crate::modules::logger::log_info(&format!(
        "[Path Detect] running app probe: app={}, candidates={}, elapsed={}ms",
        app,
        candidates.len(),
        started_at.elapsed().as_millis()
    ));

    Ok(candidates)
}

pub fn scan_app_launch_targets(
    app: &str,
    scan_roots: Option<&str>,
) -> Result<Vec<AppLaunchCandidate>, String> {
    #[cfg(target_os = "windows")]
    {
        return scan_windows_app_launch_targets(app, scan_roots);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, scan_roots);
        Ok(Vec::new())
    }
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

    let output =
        match powershell_output_with_timeout(&["-Command", &script], WINDOWS_PROCESS_PROBE_TIMEOUT)
        {
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
    // Electron / Chromium rejects several Node flags. Cockpit dev shells often
    // set NODE_OPTIONS=--openssl-legacy-provider; if that leaks into `open -a`
    // or a direct Electron spawn, Trae/VS Code-based apps flash and exit with:
    //   electron: --openssl-legacy-provider is not allowed in NODE_OPTIONS
    cmd.env_remove("NODE_OPTIONS");
    cmd.env_remove("NODE_PATH");
    cmd.env_remove("NODE_ENV");
    cmd.env_remove("npm_config_prefix");
    cmd.env_remove("npm_config_devdir");
    cmd.env_remove("ELECTRON_RUN_AS_NODE");
    cmd.env_remove("ELECTRON_NO_ASAR");
    cmd.env_remove("ELECTRON_FORCE_WINDOW_MENU_BAR");
    cmd.env_remove("ELECTRON_NO_ATTACH_CONSOLE");
    // Default Codex instances must resolve their own ~/.codex and Electron data directory.
    // Managed instances pass both values explicitly through `open --env` after this cleanup.
    cmd.env_remove("CODEX_HOME");
    cmd.env_remove("CODEX_ELECTRON_USER_DATA_PATH");
}

#[cfg(target_os = "linux")]
fn sanitize_linux_gui_launch_env(cmd: &mut Command) {
    for key in [
        "NODE_OPTIONS",
        "NODE_PATH",
        "NODE_ENV",
        "npm_config_prefix",
        "npm_config_devdir",
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ASAR",
        "CODEX_HOME",
        "CODEX_ELECTRON_USER_DATA_PATH",
    ] {
        cmd.env_remove(key);
    }
}

fn managed_proxy_env_pairs() -> Vec<(&'static str, String)> {
    let config = config::get_user_config();
    let mut pairs = Vec::new();

    let proxy_url = config.global_proxy_url.trim();
    if config.global_proxy_enabled && !proxy_url.is_empty() {
        pairs.extend([
            ("http_proxy", proxy_url.to_string()),
            ("https_proxy", proxy_url.to_string()),
            ("HTTP_PROXY", proxy_url.to_string()),
            ("HTTPS_PROXY", proxy_url.to_string()),
            ("all_proxy", proxy_url.to_string()),
            ("ALL_PROXY", proxy_url.to_string()),
        ]);
    } else if config.global_proxy_enabled {
        crate::modules::logger::log_warn("[Proxy] 全局代理已启用，但代理地址为空，跳过注入");
    }

    let no_proxy_seed = [
        std::env::var("no_proxy").unwrap_or_default(),
        std::env::var("NO_PROXY").unwrap_or_default(),
        config.global_proxy_no_proxy,
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(",");
    let no_proxy = crate::modules::codex_protocol::merge_local_no_proxy(&no_proxy_seed);
    if !no_proxy.is_empty() {
        pairs.push(("no_proxy", no_proxy.clone()));
        pairs.push(("NO_PROXY", no_proxy));
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

    let label = if proxy_url.is_empty() {
        "已注入本机直连白名单"
    } else {
        "已注入全局代理"
    };

    crate::modules::logger::log_info(&format!(
        "[Proxy] {} mode={} program={} proxy_url={} no_proxy={} keys={}",
        label,
        mode,
        cmd.get_program().to_string_lossy(),
        if proxy_url.is_empty() {
            "<none>"
        } else {
            proxy_url
        },
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

/// Strip Windows extended-length path prefixes (`\\?\` / `\\?\UNC\`) for user-facing paths.
///
/// These prefixes are a Win32 technical form (long path / verbatim path). They should not be
/// shown in settings UI or stored as the primary user-facing app path.
pub fn normalize_windows_user_facing_path(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return String::new();
    }

    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        let rest: String = trimmed.chars().skip(r"\\?\UNC\".chars().count()).collect();
        format!(r"\\{rest}")
    } else if lower.starts_with(r"\\?\") {
        trimmed.chars().skip(r"\\?\".chars().count()).collect()
    } else {
        trimmed.to_string()
    }
}

fn normalize_custom_path(value: Option<&str>) -> Option<String> {
    let normalized = normalize_windows_user_facing_path(value.unwrap_or(""));
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
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
            normalize_macos_app_root(path)
                .unwrap_or_else(|| normalize_windows_user_facing_path(&path.to_string_lossy()))
        }
        #[cfg(not(target_os = "macos"))]
        {
            normalize_windows_user_facing_path(&path.to_string_lossy())
        }
    };
    let _ = config::patch_user_config(|current| {
        let configured_path = match app {
            "antigravity" => &mut current.antigravity_app_path,
            "codex" => &mut current.codex_app_path,
            "zed" => &mut current.zed_app_path,
            "vscode" => &mut current.vscode_app_path,
            "opencode" => &mut current.opencode_app_path,
            "codebuddy" => &mut current.codebuddy_app_path,
            "codebuddy_cn" => &mut current.codebuddy_cn_app_path,
            "qoder" => &mut current.qoder_app_path,
            "zcode" => &mut current.zcode_app_path,
            "trae" => &mut current.trae_app_path,
            "trae_solo" => &mut current.trae_solo_app_path,
            "trae_cn" => &mut current.trae_cn_app_path,
            "trae_solo_cn" => &mut current.trae_solo_cn_app_path,
            "workbuddy" => &mut current.workbuddy_app_path,
            _ => return Ok(()),
        };
        if app_path_matches_snapshot(configured_path, expected_current)
            && *configured_path != normalized
        {
            *configured_path = normalized;
        }
        Ok(())
    });
}

