use crate::modules::config;
use serde::Serialize;
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
        let rest: String = trimmed
            .chars()
            .skip(r"\\?\UNC\".chars().count())
            .collect();
        format!(r"\\{rest}")
    } else if lower.starts_with(r"\\?\") {
        trimmed
            .chars()
            .skip(r"\\?\".chars().count())
            .collect()
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
            normalize_macos_app_root(path).unwrap_or_else(|| {
                normalize_windows_user_facing_path(&path.to_string_lossy())
            })
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

#[cfg(test)]
mod app_path_config_guard_tests {
    use super::{app_path_matches_snapshot, normalize_windows_user_facing_path};

    #[test]
    fn detected_path_only_replaces_the_snapshot_it_was_detected_for() {
        assert!(app_path_matches_snapshot("", ""));
        assert!(app_path_matches_snapshot(" /old/path ", "/old/path"));
        assert!(!app_path_matches_snapshot("/manual/path", ""));
        assert!(!app_path_matches_snapshot("/new/path", "/old/path"));
    }

    #[test]
    fn strips_windows_extended_path_prefix_for_user_facing_paths() {
        assert_eq!(
            normalize_windows_user_facing_path(r"\\?\C:\Program Files\WindowsApps\ChatGPT.exe"),
            r"C:\Program Files\WindowsApps\ChatGPT.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r"\\?\UNC\server\share\app.exe"),
            r"\\server\share\app.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r"C:\Apps\Codex\ChatGPT.exe"),
            r"C:\Apps\Codex\ChatGPT.exe"
        );
        assert_eq!(
            normalize_windows_user_facing_path(r#"  "\\?\D:\Codex\ChatGPT.exe"  "#),
            r"D:\Codex\ChatGPT.exe"
        );
        assert_eq!(normalize_windows_user_facing_path("   "), "");
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
        "zcode" => current.zcode_app_path,
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
    spawn_open_app_with_options_and_env(app_root, args, force_new_instance, &[])
}

#[cfg(target_os = "macos")]
fn spawn_open_app_with_options_and_env(
    app_root: &str,
    args: &[String],
    force_new_instance: bool,
    env_pairs: &[(&str, &str)],
) -> Result<u32, String> {
    let mut cmd = Command::new("open");
    sanitize_macos_gui_launch_env(&mut cmd);
    append_managed_proxy_env_to_open_args(&mut cmd);
    for (key, value) in env_pairs {
        cmd.arg("--env").arg(format!("{}={}", key, value));
    }
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
            let is_antigravity = is_windows_antigravity_ide_main_executable(&name, &exe_path);
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
        || name == "antigravity-ide.exe"
        || exe_path.ends_with("\\antigravity ide.exe")
        || exe_path.ends_with("\\antigravity.exe")
        || exe_path.ends_with("\\antigravity-ide.exe"))
        && !exe_path.contains("crashpad")
}

#[cfg(target_os = "windows")]
fn is_windows_antigravity_ide_main_executable(name: &str, exe_path: &str) -> bool {
    (name == "antigravity ide.exe"
        || name == "antigravity-ide.exe"
        || exe_path.ends_with("\\antigravity ide.exe")
        || exe_path.ends_with("\\antigravity-ide.exe"))
        && !exe_path.contains("crashpad")
}

#[cfg(target_os = "windows")]
fn resolve_windows_antigravity_ide_custom_path(path_str: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path_str);
    if path.is_file() {
        let lower = path.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with("\\antigravity ide.exe") || lower.ends_with("\\antigravity-ide.exe") {
            return Some(path);
        }
        return None;
    }

    if path.is_dir() {
        for exe_name in ["Antigravity IDE.exe", "antigravity-ide.exe"] {
            let candidate = path.join(exe_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

pub fn detect_antigravity_exec_path() -> Option<std::path::PathBuf> {
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
            let base = std::path::PathBuf::from(&local_appdata)
                .join("Programs")
                .join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            let base = std::path::PathBuf::from(&program_files).join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            let base = std::path::PathBuf::from(&program_files_x86).join("Antigravity IDE");
            candidates.push(base.join("Antigravity IDE.exe"));
            candidates.push(base.join("antigravity-ide.exe"));
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "antigravity",
            &["Antigravity IDE.exe", "antigravity-ide.exe"],
            &["antigravity-ide"],
            &["antigravity-ide", "antigravity ide"],
            &["antigravity ide", "antigravity-ide"],
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

pub fn detect_antigravity_legacy_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        for candidate in [
            ANTIGRAVITY_LEGACY_APP_PATH,
            "/Applications/Antigravity.app/Contents/MacOS/Electron",
        ] {
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
            let base = std::path::PathBuf::from(local_appdata)
                .join("Programs")
                .join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            let base = std::path::PathBuf::from(program_files).join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        if let Ok(program_files_x86) = std::env::var("PROGRAMFILES(X86)") {
            let base = std::path::PathBuf::from(program_files_x86).join("Antigravity");
            candidates.push(base.join("Antigravity.exe"));
            candidates.push(base.join("Electron.exe"));
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "antigravity",
            &["Antigravity.exe", "antigravity.exe", "Electron.exe"],
            &["antigravity"],
            &["antigravity"],
            &["antigravity"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/bin/antigravity",
            "/opt/antigravity/antigravity",
            "/usr/share/antigravity/antigravity",
        ] {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
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

fn detect_zcode_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut candidates = vec![std::path::PathBuf::from(
            "/Applications/ZCode.app/Contents/MacOS/ZCode",
        )];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("Applications/ZCode.app/Contents/MacOS/ZCode"));
        }
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates
                .push(std::path::PathBuf::from(&local_appdata).join("Programs/ZCode/ZCode.exe"));
            candidates.push(std::path::PathBuf::from(local_appdata).join("ZCode/ZCode.exe"));
        }
        for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(root) = std::env::var(variable) {
                candidates.push(std::path::PathBuf::from(root).join("ZCode/ZCode.exe"));
            }
        }
        if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
            return Some(path);
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "ZCode",
            &["ZCode.exe"],
            &["zcode"],
            &["zcode"],
            &["zcode", "z.ai"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/bin/zcode",
            "/usr/local/bin/zcode",
            "/opt/ZCode/zcode",
            "/opt/zcode/zcode",
            "/snap/bin/zcode",
        ] {
            let path = std::path::PathBuf::from(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".local/bin/zcode");
            if path.is_file() {
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
    detect_trae_exec_path_for_platform(crate::modules::trae_account::TraePlatformKind::Trae)
}

fn detect_trae_exec_path_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let app_root = format!("/Applications/{}", platform.macos_app_name());
        let candidates = [
            format!("{}/Contents/MacOS/Trae", app_root),
            format!("{}/Contents/MacOS/Electron", app_root),
            app_root,
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
        let app_dir = platform.app_support_dir_name();
        let exe_names: &[&str] = match platform {
            crate::modules::trae_account::TraePlatformKind::Trae => &["Trae.exe"],
            crate::modules::trae_account::TraePlatformKind::TraeSolo => {
                &["TRAE SOLO.exe", "Trae.exe", "Electron.exe"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeCn => {
                &["Trae CN.exe", "Trae.exe", "Electron.exe"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
                &["TRAE SOLO CN.exe", "Trae.exe", "Electron.exe"]
            }
        };
        for base_path in crate::modules::trae_account::windows_trae_install_base_paths(platform) {
            if base_path.is_file() {
                candidates.push(base_path);
                continue;
            }
            for exe_name in exe_names {
                candidates.push(base_path.join(exe_name));
            }
        }
        let current = config::get_user_config();
        let configured_scan_roots = trae_configured_app_scan_roots(&current, platform);
        if !configured_scan_roots.trim().is_empty() {
            for root in
                expand_windows_scan_roots(parse_windows_scan_roots(Some(configured_scan_roots)))
            {
                for exe_name in exe_names {
                    if root
                        .file_name()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case(app_dir))
                        .unwrap_or(false)
                    {
                        candidates.push(root.join(exe_name));
                    }
                    candidates.push(root.join(app_dir).join(exe_name));
                }
            }
        }
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            for exe_name in exe_names {
                candidates.push(
                    std::path::PathBuf::from(&local_appdata)
                        .join("Programs")
                        .join(app_dir)
                        .join(exe_name),
                );
            }
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            for exe_name in exe_names {
                candidates.push(
                    std::path::PathBuf::from(&program_files)
                        .join(app_dir)
                        .join(exe_name),
                );
            }
        }
        for candidate in candidates {
            if candidate.exists() && windows_trae_candidate_matches_platform(&candidate, platform) {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates: &[&str] = match platform {
            crate::modules::trae_account::TraePlatformKind::Trae => {
                &["/usr/bin/trae", "/usr/local/bin/trae", "/opt/trae/trae"]
            }
            crate::modules::trae_account::TraePlatformKind::TraeSolo => &[
                "/usr/bin/trae-solo",
                "/usr/local/bin/trae-solo",
                "/opt/trae-solo/trae-solo",
            ],
            crate::modules::trae_account::TraePlatformKind::TraeCn => &[
                "/usr/bin/trae-cn",
                "/usr/local/bin/trae-cn",
                "/opt/trae-cn/trae-cn",
            ],
            crate::modules::trae_account::TraePlatformKind::TraeSoloCn => &[
                "/usr/bin/trae-solo-cn",
                "/usr/local/bin/trae-solo-cn",
                "/opt/trae-solo-cn/trae-solo-cn",
            ],
        };
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
fn codex_store_package_priority(dir_name: &str) -> u8 {
    let lower = dir_name.to_ascii_lowercase();
    if lower.starts_with("openai.chatgpt_") || lower.starts_with("openai.chatgpt-desktop_") {
        2
    } else if lower.starts_with("openai.codex_") {
        1
    } else {
        0
    }
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
    let mut best: Option<(u8, Vec<u32>, std::path::PathBuf)> = None;

    for drive_root in windows_fixed_drive_roots() {
        let drive_letter = drive_root.to_string_lossy().chars().next().unwrap_or('C');
        let windows_apps_root = if drive_letter == 'C' {
            drive_root.join("Program Files").join("WindowsApps")
        } else {
            drive_root.join("WindowsApps")
        };
        let root_path = windows_apps_root;
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
            let package_priority = codex_store_package_priority(&dir_name);

            let candidate = match find_codex_windows_app_main_exe(&entry.path().join("app")) {
                Some(path) => path,
                None => continue,
            };

            let replace = match &best {
                None => true,
                Some((best_priority, best_version, _)) => {
                    package_priority > *best_priority
                        || (package_priority == *best_priority
                            && compare_windows_store_version(&version, best_version).is_gt())
                }
            };
            if replace {
                best = Some((package_priority, version, candidate));
            }
        }
    }

    if let Some((_, _, path)) = best {
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
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*' -or $_.PackageFamilyName -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
}
if ($pkg -and -not [string]::IsNullOrWhiteSpace($pkg.InstallLocation)) {
  Write-Output ([string]$pkg.InstallLocation.Trim())
}"#;

    let output =
        powershell_output_with_timeout(&["-Command", script], WINDOWS_PROCESS_PROBE_TIMEOUT)
            .ok()?;
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
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
  Select-Object -First 1
if (-not $pkg) {
  $pkg = Get-AppxPackage |
    Where-Object {
      $_.Name -like 'OpenAI.ChatGPT*' -or
      $_.Name -like 'OpenAI.Codex*' -or
      $_.PackageFamilyName -like 'OpenAI.ChatGPT*' -or
      $_.PackageFamilyName -like 'OpenAI.Codex*'
    } |
  Sort-Object @{ Expression = { if ($_.Name -like 'OpenAI.ChatGPT*' -or $_.PackageFamilyName -like 'OpenAI.ChatGPT*') { 0 } else { 1 } } }, @{ Expression = { $_.Version }; Descending = $true } |
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
fn detect_codex_store_app_user_model_id_uncached() -> Option<String> {
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
fn detect_codex_store_app_user_model_id() -> Option<String> {
    if let Some(app_user_model_id) = CODEX_STORE_APP_USER_MODEL_ID_CACHE.get() {
        return Some(app_user_model_id.clone());
    }

    let detected = detect_codex_store_app_user_model_id_uncached();
    if let Some(ref app_user_model_id) = detected {
        let _ = CODEX_STORE_APP_USER_MODEL_ID_CACHE.set(app_user_model_id.clone());
    }
    detected
}

#[cfg(target_os = "windows")]
fn powershell_argument_list_clause(values: &[String]) -> String {
    let arguments = values
        .iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("'{}'", escape_powershell_single_quoted(value)))
        .collect::<Vec<_>>();
    if arguments.is_empty() {
        return String::new();
    }
    format!(" -ArgumentList @({})", arguments.join(", "))
}

#[cfg(target_os = "windows")]
fn launch_codex_via_store_app_user_model_id(
    app_user_model_id: &str,
    codex_home: Option<&str>,
    app_user_data_dir: Option<&str>,
    extra_args: &[String],
) -> Result<(), String> {
    let app_user_model_id = app_user_model_id.trim();
    if app_user_model_id.is_empty() {
        return Err("Codex AppUserModelId 为空".to_string());
    }

    let escaped = escape_powershell_single_quoted(app_user_model_id);
    let mut env_pairs = managed_proxy_env_pairs();
    if let Some(codex_home) = codex_home.map(str::trim).filter(|value| !value.is_empty()) {
        env_pairs.push(("CODEX_HOME", codex_home.to_string()));
    }
    if let Some(app_user_data_dir) = app_user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        env_pairs.push((
            "CODEX_ELECTRON_USER_DATA_PATH",
            app_user_data_dir.to_string(),
        ));
    }
    let env_lines = env_pairs
        .into_iter()
        .map(|(key, value)| format!("$env:{}='{}'", key, escape_powershell_single_quoted(&value)))
        .collect::<Vec<_>>()
        .join("\n");
    let argument_list = powershell_argument_list_clause(extra_args);
    let script = format!(
        r#"{env_lines}
$appId='{escaped}';
$target='shell:AppsFolder\' + $appId
Start-Process -FilePath $target{argument_list} -ErrorAction Stop | Out-Null"#
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

#[cfg(target_os = "windows")]
fn launch_codex_via_powershell_exec_path(
    launch_path: &std::path::Path,
    codex_home: &str,
    app_user_data_dir: &std::path::Path,
    extra_args: &[String],
) -> Result<(), String> {
    let launch_path = launch_path.to_string_lossy();
    let launch_path = launch_path.trim();
    if launch_path.is_empty() {
        return Err("Codex 启动路径为空".to_string());
    }

    let mut env_pairs = managed_proxy_env_pairs();
    env_pairs.push(("CODEX_HOME", codex_home.to_string()));
    env_pairs.push((
        "CODEX_ELECTRON_USER_DATA_PATH",
        app_user_data_dir.to_string_lossy().to_string(),
    ));
    let env_lines = env_pairs
        .into_iter()
        .map(|(key, value)| format!("$env:{}='{}'", key, escape_powershell_single_quoted(&value)))
        .collect::<Vec<_>>()
        .join("\n");
    let argument_list = powershell_argument_list_clause(extra_args);
    let script = format!(
        r#"{env_lines}
$exe='{exe}';
Start-Process -FilePath $exe{argument_list} -ErrorAction Stop | Out-Null"#,
        exe = escape_powershell_single_quoted(launch_path),
    );

    let output = powershell_output(&["-Command", &script])
        .map_err(|e| format!("PowerShell 启动 Codex 失败: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_head = stderr.trim().chars().take(400).collect::<String>();
        return Err(format!(
            "PowerShell 启动 Codex 失败: status={}, stderr={}",
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

const CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX: &str = "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:";

fn codex_managed_store_launch_unsafe_error(direct_error: &str, powershell_error: &str) -> String {
    format!(
        "{}direct_error={}; powershell_error={}",
        CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX, direct_error, powershell_error
    )
}

pub(crate) fn detect_codex_exec_path() -> Option<std::path::PathBuf> {
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

fn detect_and_save_codex_launch_path() -> Option<std::path::PathBuf> {
    let expected_current = config::get_user_config().codex_app_path;
    let detected = detect_codex_exec_path()?;
    update_app_path_in_config("codex", &detected, &expected_current);
    Some(detected)
}

#[cfg(any(test, target_os = "windows"))]
fn normalized_windows_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(any(test, target_os = "windows"))]
fn is_legacy_codex_store_launch_path(path: &Path) -> bool {
    let normalized = normalized_windows_path_text(path);
    normalized.ends_with("\\codex.exe") && normalized.contains("\\windowsapps\\openai.codex_")
}

#[cfg(any(test, target_os = "windows"))]
fn is_chatgpt_windows_launch_path(path: &Path) -> bool {
    normalized_windows_path_text(path).ends_with("\\chatgpt.exe")
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

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn should_migrate_legacy_codex_launch_path(current: &Path, detected: &Path) -> bool {
    let mut should_migrate = false;

    #[cfg(any(test, target_os = "windows"))]
    {
        should_migrate |=
            is_legacy_codex_store_launch_path(current) && is_chatgpt_windows_launch_path(detected);
    }

    #[cfg(any(test, target_os = "macos"))]
    {
        should_migrate |= is_official_legacy_codex_macos_path(current)
            && is_official_chatgpt_macos_path(detected);
    }

    should_migrate
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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

fn detect_opencode_exec_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidate = std::path::PathBuf::from("/Applications/OpenCode.app");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                std::path::PathBuf::from(local_appdata)
                    .join("Programs")
                    .join("OpenCode")
                    .join("OpenCode.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                std::path::PathBuf::from(program_files)
                    .join("OpenCode")
                    .join("OpenCode.exe"),
            );
        }
        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(path) = detect_windows_exec_path_by_signatures(
            "opencode",
            &["OpenCode.exe", "opencode.exe"],
            &["opencode"],
            &["opencode"],
            &["opencode", "open code"],
        ) {
            return Some(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = ["/usr/bin/opencode", "/opt/opencode/opencode"];
        for candidate in candidates {
            let path = std::path::PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_antigravity_launch_path() -> Result<std::path::PathBuf, String> {
    let configured_path = config::get_user_config().antigravity_app_path;
    if let Some(custom) = normalize_custom_path(Some(&configured_path)) {
        #[cfg(target_os = "macos")]
        if is_legacy_antigravity_macos_path(&custom) {
            if let Some(detected) = detect_antigravity_exec_path() {
                update_app_path_in_config("antigravity", &detected, &configured_path);
                return Ok(detected);
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(exec) = resolve_windows_antigravity_ide_custom_path(&custom) {
                return Ok(exec);
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }

        if let Some(detected) = detect_antigravity_exec_path() {
            update_app_path_in_config("antigravity", &detected, &configured_path);
            return Ok(detected);
        }
        return Err(app_path_missing_error("antigravity"));
    }

    if let Some(detected) = detect_antigravity_exec_path() {
        update_app_path_in_config("antigravity", &detected, &configured_path);
        return Ok(detected);
    }

    Err(app_path_missing_error("antigravity"))
}

pub fn ensure_antigravity_launch_path_configured() -> Result<(), String> {
    resolve_antigravity_launch_path().map(|_| ())
}

fn resolve_antigravity_legacy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) =
        normalize_custom_path(Some(&config::get_user_config().antigravity_app_path))
    {
        #[cfg(target_os = "macos")]
        if is_legacy_antigravity_macos_path(&custom) {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Antigravity")
                .or_else(|| resolve_macos_exec_path(&custom, "Electron"))
            {
                return Ok(exec);
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let custom_path = std::path::PathBuf::from(&custom);
            let lower = custom.to_ascii_lowercase();
            if lower.contains("antigravity") && !lower.contains("antigravity ide") {
                if custom_path.is_file() {
                    return Ok(custom_path);
                }
                for exe_name in ["Antigravity.exe", "antigravity.exe", "Electron.exe"] {
                    let candidate = custom_path.join(exe_name);
                    if candidate.exists() {
                        return Ok(candidate);
                    }
                }
            }
        }
    }

    if let Some(detected) = detect_antigravity_legacy_exec_path() {
        return Ok(detected);
    }

    Err(app_path_missing_error("antigravity"))
}

pub fn ensure_antigravity_legacy_launch_path_configured() -> Result<(), String> {
    resolve_antigravity_legacy_launch_path().map(|_| ())
}

pub fn ensure_vscode_launch_path_configured() -> Result<(), String> {
    resolve_vscode_launch_path().map(|_| ())
}

pub fn ensure_codex_launch_path_configured() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if detect_codex_store_app_user_model_id().is_some() {
            return Ok(());
        }
        if resolve_codex_launch_path().is_ok() {
            return Ok(());
        }
        return Err("未检测到 Codex 商店安装，请先在 Microsoft Store 安装 Codex".to_string());
    }

    #[cfg(not(target_os = "windows"))]
    {
        if resolve_codex_launch_path().is_ok() {
            return Ok(());
        }
        if detect_and_save_app_path("codex", true).is_some() {
            return Ok(());
        }
        resolve_codex_launch_path().map(|_| ())
    }
}

pub fn ensure_codebuddy_launch_path_configured() -> Result<(), String> {
    resolve_codebuddy_launch_path().map(|_| ())
}

pub fn ensure_codebuddy_cn_launch_path_configured() -> Result<(), String> {
    resolve_codebuddy_cn_launch_path().map(|_| ())
}

pub fn ensure_qoder_launch_path_configured() -> Result<(), String> {
    resolve_qoder_launch_path().map(|_| ())
}

pub fn ensure_trae_launch_path_configured() -> Result<(), String> {
    resolve_trae_launch_path().map(|_| ())
}

pub fn ensure_workbuddy_launch_path_configured() -> Result<(), String> {
    resolve_workbuddy_launch_path().map(|_| ())
}

fn resolve_vscode_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().vscode_app_path)) {
        #[cfg(target_os = "macos")]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if let Some(exec) = resolve_macos_exec_path(&custom, "Electron") {
                return Ok(exec);
            }
        }
        return Err(app_path_missing_error("vscode"));
    }

    Err(app_path_missing_error("vscode"))
}

fn resolve_codebuddy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codebuddy_app_path))
    {
        if let Some(exec) = resolve_codebuddy_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("codebuddy"));
    }

    if let Some(detected) = detect_codebuddy_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_codebuddy_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codebuddy"))
}

fn resolve_codebuddy_cn_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) =
        normalize_custom_path(Some(&config::get_user_config().codebuddy_cn_app_path))
    {
        if let Some(exec) = resolve_codebuddy_cn_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("codebuddy_cn"));
    }

    if let Some(detected) = detect_codebuddy_cn_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_codebuddy_cn_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codebuddy_cn"))
}

fn resolve_qoder_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().qoder_app_path)) {
        if let Some(exec) = resolve_qoder_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("qoder"));
    }

    if let Some(detected) = detect_qoder_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_qoder_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("qoder"))
}

pub fn resolve_zcode_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().zcode_app_path)) {
        if let Some(exec) = resolve_macos_exec_path(&custom, "ZCode") {
            return Ok(exec);
        }
        return Err(app_path_missing_error("zcode"));
    }

    if let Some(detected) = detect_zcode_exec_path() {
        update_app_path_in_config("zcode", &detected, "");
        let detected = detected.to_string_lossy();
        if let Some(exec) = resolve_macos_exec_path(&detected, "ZCode") {
            return Ok(exec);
        }
    }

    Err(app_path_missing_error("zcode"))
}

pub fn ensure_zed_launch_path_configured() -> Result<(), String> {
    resolve_zed_launch_path().map(|_| ())
}

pub fn resolve_zed_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().zed_app_path)) {
        if let Some(exec) = resolve_zed_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("zed"));
    }

    if let Some(detected) = detect_zed_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_zed_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("zed"))
}

fn resolve_trae_launch_path() -> Result<std::path::PathBuf, String> {
    resolve_trae_launch_path_for_platform(crate::modules::trae_account::TraePlatformKind::Trae)
}

fn trae_configured_app_path(
    current: &config::UserConfig,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> &str {
    match platform {
        crate::modules::trae_account::TraePlatformKind::Trae => current.trae_app_path.as_str(),
        crate::modules::trae_account::TraePlatformKind::TraeSolo => {
            current.trae_solo_app_path.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeCn => current.trae_cn_app_path.as_str(),
        crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
            current.trae_solo_cn_app_path.as_str()
        }
    }
}

#[cfg(target_os = "windows")]
fn trae_configured_app_scan_roots(
    current: &config::UserConfig,
    platform: crate::modules::trae_account::TraePlatformKind,
) -> &str {
    match platform {
        crate::modules::trae_account::TraePlatformKind::Trae => {
            current.trae_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeSolo => {
            current.trae_solo_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeCn => {
            current.trae_cn_app_scan_roots.as_str()
        }
        crate::modules::trae_account::TraePlatformKind::TraeSoloCn => {
            current.trae_solo_cn_app_scan_roots.as_str()
        }
    }
}

fn resolve_trae_launch_path_for_platform(
    platform: crate::modules::trae_account::TraePlatformKind,
) -> Result<std::path::PathBuf, String> {
    let current = config::get_user_config();
    if let Some(custom) = normalize_custom_path(Some(trae_configured_app_path(&current, platform)))
    {
        if let Some(exec) = resolve_trae_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error(platform.provider_key()));
    }

    if let Some(detected) = detect_trae_exec_path_for_platform(platform) {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_trae_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error(platform.provider_key()))
}

fn resolve_workbuddy_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().workbuddy_app_path))
    {
        if let Some(exec) = resolve_workbuddy_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err(app_path_missing_error("workbuddy"));
    }

    if let Some(detected) = detect_workbuddy_exec_path() {
        let detected_str = detected.to_string_lossy();
        if let Some(exec) = resolve_workbuddy_macos_exec_path(&detected_str) {
            return Ok(exec);
        }
        #[cfg(target_os = "macos")]
        if detected.is_file() {
            return Ok(detected);
        }
        #[cfg(not(target_os = "macos"))]
        if detected.exists() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("workbuddy"))
}

#[cfg(target_os = "macos")]
fn resolve_codex_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codex_app_path)) {
        if let Some(migrated) = migrate_legacy_codex_launch_path(&custom) {
            return Ok(migrated);
        }
        if let Some(exec) = resolve_codex_macos_exec_path(&custom) {
            return Ok(exec);
        }
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
        return Err(app_path_missing_error("codex"));
    }

    if let Some(detected) = detect_and_save_codex_launch_path() {
        return Ok(detected);
    }

    Err(app_path_missing_error("codex"))
}

#[cfg(not(target_os = "macos"))]
fn resolve_codex_launch_path() -> Result<std::path::PathBuf, String> {
    if let Some(custom) = normalize_custom_path(Some(&config::get_user_config().codex_app_path)) {
        #[cfg(target_os = "windows")]
        if let Some(migrated) = migrate_legacy_codex_launch_path(&custom) {
            return Ok(migrated);
        }
        if let Some(exec) = resolve_macos_exec_path(&custom, "Codex") {
            return Ok(exec);
        }
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
        return Err(app_path_missing_error("codex"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(detected) = detect_and_save_codex_launch_path() {
            return Ok(detected);
        }
    }

    Err(app_path_missing_error("codex"))
}

pub fn detect_and_save_app_path(app: &str, force: bool) -> Option<String> {
    detect_and_save_app_path_raw(app, force).map(|path| normalize_windows_user_facing_path(&path))
}

fn detect_and_save_app_path_raw(app: &str, force: bool) -> Option<String> {
    let current = config::get_user_config();
    match app {
        "antigravity" | "antigravity_ide" => {
            if !force && !current.antigravity_app_path.trim().is_empty() {
                return Some(current.antigravity_app_path);
            }
            if let Some(detected) = detect_antigravity_exec_path() {
                update_app_path_in_config("antigravity", &detected, &current.antigravity_app_path);
                return Some(config::get_user_config().antigravity_app_path);
            }
        }
        "antigravity_legacy" => {
            if !force && !current.antigravity_app_path.trim().is_empty() {
                return Some(current.antigravity_app_path);
            }
            if let Some(detected) = detect_antigravity_legacy_exec_path() {
                update_app_path_in_config("antigravity", &detected, &current.antigravity_app_path);
                return Some(config::get_user_config().antigravity_app_path);
            }
        }
        "codex" => {
            if !force && !current.codex_app_path.trim().is_empty() {
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                if migrate_legacy_codex_launch_path(&current.codex_app_path).is_some() {
                    return Some(config::get_user_config().codex_app_path);
                }
                return Some(current.codex_app_path);
            }
            if let Some(detected) = detect_codex_exec_path() {
                update_app_path_in_config("codex", &detected, &current.codex_app_path);
                return Some(config::get_user_config().codex_app_path);
            }
        }
        "zed" => {
            if !force && !current.zed_app_path.trim().is_empty() {
                return Some(current.zed_app_path);
            }
            if let Some(detected) = detect_zed_exec_path() {
                update_app_path_in_config("zed", &detected, &current.zed_app_path);
                return Some(config::get_user_config().zed_app_path);
            }
        }
        "vscode" => {
            if !force && !current.vscode_app_path.trim().is_empty() {
                return Some(current.vscode_app_path);
            }
            if let Some(detected) = detect_vscode_exec_path() {
                update_app_path_in_config("vscode", &detected, &current.vscode_app_path);
                return Some(config::get_user_config().vscode_app_path);
            }
        }
        "codebuddy" => {
            if !force && !current.codebuddy_app_path.trim().is_empty() {
                return Some(current.codebuddy_app_path);
            }
            if let Some(detected) = detect_codebuddy_exec_path() {
                update_app_path_in_config("codebuddy", &detected, &current.codebuddy_app_path);
                return Some(config::get_user_config().codebuddy_app_path);
            }
        }
        "codebuddy_cn" => {
            if !force && !current.codebuddy_cn_app_path.trim().is_empty() {
                return Some(current.codebuddy_cn_app_path);
            }
            if let Some(detected) = detect_codebuddy_cn_exec_path() {
                update_app_path_in_config(
                    "codebuddy_cn",
                    &detected,
                    &current.codebuddy_cn_app_path,
                );
                return Some(config::get_user_config().codebuddy_cn_app_path);
            }
        }
        "qoder" => {
            if !force && !current.qoder_app_path.trim().is_empty() {
                return Some(current.qoder_app_path);
            }
            if let Some(detected) = detect_qoder_exec_path() {
                update_app_path_in_config("qoder", &detected, &current.qoder_app_path);
                return Some(config::get_user_config().qoder_app_path);
            }
        }
        "zcode" => {
            if !force && !current.zcode_app_path.trim().is_empty() {
                return Some(current.zcode_app_path);
            }
            if let Some(detected) = detect_zcode_exec_path() {
                update_app_path_in_config("zcode", &detected, &current.zcode_app_path);
                return Some(config::get_user_config().zcode_app_path);
            }
        }
        "trae" | "trae_solo" | "trae_cn" | "trae_solo_cn" => {
            if let Ok(platform) = crate::modules::trae_account::TraePlatformKind::parse(Some(app)) {
                if !force {
                    let configured = trae_configured_app_path(&current, platform);
                    if !configured.trim().is_empty() {
                        return Some(configured.to_string());
                    }
                }
                if let Some(detected) = detect_trae_exec_path_for_platform(platform) {
                    update_app_path_in_config(
                        app,
                        &detected,
                        trae_configured_app_path(&current, platform),
                    );
                    let refreshed = config::get_user_config();
                    return Some(trae_configured_app_path(&refreshed, platform).to_string());
                }
            }
        }
        "opencode" => {
            if !force && !current.opencode_app_path.trim().is_empty() {
                return Some(current.opencode_app_path);
            }
            if let Some(detected) = detect_opencode_exec_path() {
                update_app_path_in_config("opencode", &detected, &current.opencode_app_path);
                return Some(config::get_user_config().opencode_app_path);
            }
        }
        "workbuddy" => {
            if !force && !current.workbuddy_app_path.trim().is_empty() {
                return Some(current.workbuddy_app_path);
            }
            if let Some(detected) = detect_workbuddy_exec_path() {
                update_app_path_in_config("workbuddy", &detected, &current.workbuddy_app_path);
                return Some(config::get_user_config().workbuddy_app_path);
            }
        }
        _ => {}
    }
    None
}

pub fn is_pid_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        // On macOS, use ps to avoid sysinfo TCC dialogs.
        // Treat zombie/defunct process as not running.
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output();
        let output = match output {
            Ok(value) if value.status.success() => value,
            _ => return false,
        };

        let stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stat.is_empty() {
            return false;
        }
        let first = stat.chars().next().unwrap_or_default();
        if first == 'Z' || first == 'z' {
            return false;
        }
        true
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
        system.process(Pid::from(pid as usize)).is_some()
    }
}

#[cfg(not(target_os = "macos"))]
fn extract_user_data_dir(args: &[std::ffi::OsString]) -> Option<String> {
    let tokens: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect();
    let mut index = 0;
    while index < tokens.len() {
        let value = tokens[index].as_str();
        if let Some(rest) = value.strip_prefix("--user-data-dir=") {
            return Some(rest.to_string());
        }
        if value == "--user-data-dir" {
            index += 1;
            if index >= tokens.len() {
                return None;
            }
            let mut parts = Vec::new();
            while index < tokens.len() {
                let part = tokens[index].as_str();
                if part.starts_with("--") {
                    break;
                }
                parts.push(part);
                index += 1;
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
            return None;
        }
        index += 1;
    }
    None
}

fn extract_user_data_dir_from_command_line(command_line: &str) -> Option<String> {
    let tokens = split_command_tokens(command_line);
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix("--user-data-dir=") {
            if !rest.trim().is_empty() {
                return Some(rest.to_string());
            }
        }
        if token == "--user-data-dir" {
            index += 1;
            if index >= tokens.len() {
                return None;
            }
            let mut parts = Vec::new();
            while index < tokens.len() {
                let part = tokens[index].as_str();
                if part.starts_with("--") || is_env_token(part) {
                    break;
                }
                parts.push(part);
                index += 1;
            }
            if !parts.is_empty() {
                return Some(parts.join(" "));
            }
            return None;
        }
        index += 1;
    }
    None
}

#[cfg(target_os = "macos")]
fn parse_env_value(raw: &str) -> Option<String> {
    let rest = raw.trim_start();
    if rest.is_empty() {
        return None;
    }
    let value = if rest.starts_with('"') {
        let end = rest[1..].find('"').map(|idx| idx + 1).unwrap_or(rest.len());
        &rest[1..end]
    } else if rest.starts_with('\'') {
        let end = rest[1..]
            .find('\'')
            .map(|idx| idx + 1)
            .unwrap_or(rest.len());
        &rest[1..end]
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        &rest[..end]
    };
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(target_os = "macos")]
fn extract_env_value_from_tokens(tokens: &[String], key: &str) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    let prefix = format!("{}=", key);
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix(&prefix) {
            let mut parts: Vec<&str> = Vec::new();
            if !rest.is_empty() {
                parts.push(rest);
            }
            let mut next = index + 1;
            while next < tokens.len() {
                let value = tokens[next].as_str();
                if value.starts_with("--") || is_env_token(value) {
                    break;
                }
                parts.push(value);
                next += 1;
            }
            if parts.is_empty() {
                return None;
            }
            let joined = parts.join(" ");
            let trimmed = joined.trim();
            if trimmed.is_empty() {
                return None;
            }
            return Some(trimmed.to_string());
        }
        index += 1;
    }
    None
}

fn split_command_tokens(command_line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in command_line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(current.clone());
                        current.clear();
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn is_env_token(token: &str) -> bool {
    let (key, _) = match token.split_once('=') {
        Some(parts) => parts,
        None => return false,
    };
    if key.is_empty() {
        return false;
    }
    let mut chars = key.chars();
    let first = match chars.next() {
        Some(value) => value,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(target_os = "macos")]
fn extract_env_value(command_line: &str, key: &str) -> Option<String> {
    let needle = format!("{}=", key);
    let pos = command_line.find(&needle)?;
    let rest = &command_line[pos + needle.len()..];
    parse_env_value(rest)
}

fn normalize_path_for_compare(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    #[cfg(target_os = "windows")]
    let normalized_input = normalize_windows_user_facing_path(trimmed).replace('/', "\\");
    #[cfg(not(target_os = "windows"))]
    let normalized_input = trimmed.to_string();

    let resolved = std::fs::canonicalize(&normalized_input)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(normalized_input);

    #[cfg(target_os = "windows")]
    let resolved = normalize_windows_user_facing_path(&resolved);

    #[cfg(target_os = "windows")]
    {
        return resolved.to_lowercase();
    }
    #[cfg(not(target_os = "windows"))]
    {
        return resolved;
    }
}

fn is_helper_command_line(cmdline_lower: &str) -> bool {
    cmdline_lower.contains("--type=")
        || cmdline_lower.contains("helper")
        || cmdline_lower.contains("plugin")
        || cmdline_lower.contains("renderer")
        || cmdline_lower.contains("gpu")
        || cmdline_lower.contains("crashpad")
        || cmdline_lower.contains("utility")
        || cmdline_lower.contains("audio")
        || cmdline_lower.contains("sandbox")
        || cmdline_lower.contains("--node-ipc")
        || cmdline_lower.contains("--clientprocessid=")
        || cmdline_lower.contains("\\resources\\app\\extensions\\")
        || cmdline_lower.contains("/resources/app/extensions/")
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
    if entries.is_empty() {
        return entries;
    }
    let Some(expected) = expected else {
        return Vec::new();
    };
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "linux")]
fn collect_antigravity_process_entries_from_proc() -> Vec<(u32, Option<String>)> {
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
        if cmdline.is_empty() {
            continue;
        }
        let cmdline_str = String::from_utf8_lossy(&cmdline).replace('\0', " ");
        let cmd_lower = cmdline_str.to_lowercase();
        let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
            .ok()
            .and_then(|p| p.to_str().map(|s| s.to_lowercase()))
            .unwrap_or_default();
        if !cmd_lower.contains("antigravity-ide") && !exe_path.contains("antigravity-ide") {
            continue;
        }
        if cmd_lower.contains("tools") || exe_path.contains("tools") {
            continue;
        }
        if is_helper_command_line(&cmd_lower) {
            continue;
        }
        let dir = extract_user_data_dir_from_command_line(&cmdline_str);
        result.push((pid, dir));
    }
    result
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
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
        }
        let entries = collect_antigravity_process_entries_from_ps();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
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
        let entries = collect_antigravity_process_entries_from_proc();
        if !entries.is_empty() {
            return filter_entries_by_expected_launch_path("AG", entries, expected_launch.clone());
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
            return filter_entries_by_expected_launch_path("AG Legacy", entries, expected_launch);
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
        let entries = collect_antigravity_process_entries_from_proc()
            .into_iter()
            .filter(|(_, dir)| {
                dir.as_deref()
                    .is_some_and(|value| value.contains("Antigravity"))
            })
            .collect::<Vec<_>>();
        return filter_entries_by_expected_launch_path("AG Legacy", entries, expected_launch);
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
    let entries = collect_antigravity_process_entries();
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
    crate::modules::logger::log_info(&format!(
        "[Focus] Windows PowerShell focus hwnd={}",
        hwnd
    ));
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

#[cfg(target_os = "macos")]
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

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub fn resolve_codex_pid_from_entries(
    last_pid: Option<u32>,
    _codex_home: Option<&str>,
    _entries: &[(u32, Option<String>)],
) -> Option<u32> {
    last_pid.filter(|pid| is_pid_running(*pid))
}

#[cfg(target_os = "macos")]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(target_os = "windows")]
pub fn resolve_codex_pid(last_pid: Option<u32>, codex_home: Option<&str>) -> Option<u32> {
    let entries = collect_codex_process_entries();
    resolve_codex_pid_from_entries(last_pid, codex_home, &entries)
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
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
    close_trae_platform_instances(platform, &[default_dir], timeout_secs)
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
    )
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

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let _ = (codex_home, extra_args);
        Err("Codex 应用多开仅支持 macOS 和 Windows".to_string())
    }
}

/// 启动 Codex 默认实例（不注入 CODEX_HOME，支持附加参数，支持 macOS / Windows）
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = extra_args;
        Err("Codex 启动仅支持 macOS 和 Windows".to_string())
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
    #[cfg(target_os = "macos")]
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

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = timeout_secs;
        Err("Codex 启动仅支持 macOS 和 Windows".to_string())
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

/// 重启 Codex 默认实例，让官方 App 重新读取磁盘上的全局状态。
pub fn restart_codex_default(extra_args: &[String], timeout_secs: u64) -> Result<u32, String> {
    ensure_codex_launch_path_configured()?;
    close_codex_default(timeout_secs)?;
    start_codex_default(extra_args)
}

#[cfg(target_os = "macos")]
fn is_macos_codex_app_server_command_line(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    lower.contains(".app/contents/resources/codex")
        && lower.contains(" app-server")
        && !lower.contains("cockpit-tools")
}

#[cfg(target_os = "macos")]
fn parse_macos_codex_app_server_pid_line(line: &str) -> Result<Option<u32>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
    let pid_text = parts
        .next()
        .ok_or_else(|| "ps 输出缺少 PID".to_string())?
        .trim();
    let pid = pid_text
        .parse::<u32>()
        .map_err(|_| format!("ps 输出包含无效 PID: {}", pid_text))?;
    let command = parts
        .next()
        .ok_or_else(|| format!("ps 输出缺少命令行: PID={}", pid))?
        .trim();
    Ok(is_macos_codex_app_server_command_line(command).then_some(pid))
}

#[cfg(target_os = "macos")]
fn parse_macos_codex_app_server_ps_output(
    command_succeeded: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Vec<u32>, String> {
    if !command_succeeded {
        let stderr = String::from_utf8_lossy(stderr).trim().to_string();
        return Err(format!(
            "执行 ps 失败: {}",
            if stderr.is_empty() {
                "命令返回非零状态"
            } else {
                &stderr
            }
        ));
    }

    let stdout = std::str::from_utf8(stdout)
        .map_err(|error| format!("解析 ps 输出失败：不是有效 UTF-8: {}", error))?;
    let mut pids = Vec::new();
    for line in stdout.lines() {
        if let Some(pid) = parse_macos_codex_app_server_pid_line(line)? {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn collect_codex_app_server_pids() -> Result<Vec<u32>, String> {
    let output = Command::new("ps")
        .args(["-axww", "-o", "pid=,command="])
        .output()
        .map_err(|error| format!("执行 ps 命令失败: {}", error))?;
    parse_macos_codex_app_server_ps_output(output.status.success(), &output.stdout, &output.stderr)
}

#[cfg(target_os = "macos")]
fn codex_app_server_switch_preflight_for_pids(pids: &[u32]) -> Result<(), String> {
    if pids.is_empty() {
        return Ok(());
    }

    let instance_hint = if pids.len() > 1 {
        "检测到多个官方 ChatGPT/Codex app-server 实例"
    } else {
        "检测到官方 ChatGPT/Codex app-server 正在运行"
    };
    Err(format!(
        "{}（{}）。为避免中断当前会话，先关闭 ChatGPT/Codex 后再切换。",
        instance_hint,
        summarize_pid_list_for_log(pids)
    ))
}

#[cfg(target_os = "macos")]
pub fn ensure_codex_app_server_not_running() -> Result<(), String> {
    let pids = collect_codex_app_server_pids()?;
    codex_app_server_switch_preflight_for_pids(&pids)
}

#[cfg(target_os = "windows")]
pub fn ensure_codex_app_server_not_running() -> Result<(), String> {
    let pids = collect_codex_windows_resource_process_pids();
    if pids.is_empty() {
        return Ok(());
    }
    Err(format!(
        "检测到官方 ChatGPT/Codex app-server 正在运行（{}）。为避免中断当前会话，先关闭 ChatGPT/Codex 后再切换。",
        summarize_pid_list_for_log(&pids)
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn ensure_codex_app_server_not_running() -> Result<(), String> {
    Ok(())
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
                    crate::modules::logger::log_info(&format!(
                        "[Codex Close] graceful close finished, targets={}",
                        summarize_pid_list_for_log(&pids)
                    ));
                    return Ok(());
                }
            }
        } else {
            crate::modules::logger::log_warn(
                "[Codex Close] graceful close request failed for all macOS targets, skip grace wait",
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
            if let Err(err) = close_pids(&remaining, timeout_secs) {
                crate::modules::logger::log_warn(&format!(
                    "[Codex Close] force close_pids failed: {}",
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

        let open_pid = spawn_open_app_with_options(&app_root, &args, true)
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!(
            "{} 启动命令已发送（open -n -a）",
            platform.display_name()
        ));
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_trae_pid_for_platform(None, Some(target), platform)
            {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Trae Start] platform={} 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
            platform.provider_key(),
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
            .map_err(|e| format!("启动 {} 失败: {}", platform.display_name(), e))?;
        crate::modules::logger::log_info(&format!(
            "{} 默认实例启动命令已发送（open -n -a）",
            platform.display_name()
        ));
        let probe_started = Instant::now();
        let timeout = Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_trae_pid_for_platform(None, None, platform) {
                return Ok(resolved_pid);
            }
            thread::sleep(Duration::from_millis(200));
        }
        crate::modules::logger::log_warn(&format!(
            "[Trae Start] platform={} 启动后 6s 内未匹配到默认实例 PID，回退 open pid={}",
            platform.provider_key(),
            open_pid
        ));
        return Ok(open_pid);
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

#[cfg(test)]
mod legacy_platform_adapter_cleanup_tests {
    use super::{orphaned_legacy_platform_adapter_pid_from_ps_line, utf8_command_output_snippet};

    #[test]
    fn matches_orphaned_legacy_platform_adapter() {
        let line = " 1359     1 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(line, 99999),
            Some(1359)
        );
    }

    #[test]
    fn ignores_non_orphaned_or_current_processes() {
        let line = " 1359 1805 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(line, 99999),
            None
        );

        let current_line = " 1359 1 /Users/jieli/.antigravity_cockpit/platform-packages/codex/current/adapter/macos/cockpit-codex-adapter";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(current_line, 1359),
            None
        );
    }

    #[test]
    fn ignores_current_sidecar_and_official_apps() {
        let sidecar =
            " 64680 1805 /Applications/Cockpit Tools.app/Contents/MacOS/cockpit-cliproxy --parent-pid 1805";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(sidecar, 99999),
            None
        );

        let official_codex =
            " 9300 1 /Applications/Codex.app/Contents/Frameworks/Codex Framework.framework/Helpers/browser_crashpad_handler";
        assert_eq!(
            orphaned_legacy_platform_adapter_pid_from_ps_line(official_codex, 99999),
            None
        );
    }

    #[test]
    fn command_output_snippet_drops_non_utf8_bytes() {
        assert_eq!(utf8_command_output_snippet(&[0xb4, 0xed, 0xce, 0xf3]), None);
    }

    #[test]
    fn command_output_snippet_keeps_utf8_text() {
        assert_eq!(
            utf8_command_output_snippet("No such process\n".as_bytes()).as_deref(),
            Some("No such process")
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod codex_macos_launch_tests {
    use super::{
        is_codex_macos_main_process_command_line, is_macos_codex_app_server_command_line,
        parse_macos_codex_app_server_pid_line,
    };

    #[test]
    fn matches_chatgpt_and_legacy_codex_main_processes() {
        assert!(is_codex_macos_main_process_command_line(
            "/applications/chatgpt.app/contents/macos/chatgpt"
        ));
        assert!(is_codex_macos_main_process_command_line(
            "/applications/codex.app/contents/macos/codex"
        ));
        assert!(!is_codex_macos_main_process_command_line(
            "/applications/chatgpt.app/contents/resources/codex app-server"
        ));
    }

    #[test]
    fn matches_official_chatgpt_codex_app_server_process() {
        assert!(is_macos_codex_app_server_command_line(
            "/Applications/ChatGPT.app/Contents/Resources/codex -c features.code_mode_host=true app-server"
        ));
        assert!(is_macos_codex_app_server_command_line(
            "/Applications/Codex.app/Contents/Resources/codex app-server --listen stdio://"
        ));
    }

    #[test]
    fn rejects_non_app_server_and_cockpit_processes() {
        assert!(!is_macos_codex_app_server_command_line(
            "/Applications/ChatGPT.app/Contents/Resources/codex --version"
        ));
        assert!(!is_macos_codex_app_server_command_line(
            "/Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools app-server"
        ));
    }

    #[test]
    fn switch_preflight_allows_idle_and_rejects_single_or_multiple_servers() {
        assert!(super::codex_app_server_switch_preflight_for_pids(&[]).is_ok());

        let single = super::codex_app_server_switch_preflight_for_pids(&[4242])
            .expect_err("a running app-server must block switching");
        assert!(single.contains("先关闭 ChatGPT/Codex 后再切换"));
        assert!(single.contains("count=1"));

        let multiple = super::codex_app_server_switch_preflight_for_pids(&[4242, 4243])
            .expect_err("multiple app-servers must block switching");
        assert!(multiple.contains("多个官方 ChatGPT/Codex app-server"));
        assert!(multiple.contains("count=2"));
    }

    #[test]
    fn parses_pid_only_for_matching_app_server_lines() {
        assert_eq!(
            parse_macos_codex_app_server_pid_line(
                " 4242 /Applications/ChatGPT.app/Contents/Resources/codex app-server"
            )
            .expect("valid ps line"),
            Some(4242)
        );
        assert_eq!(
            parse_macos_codex_app_server_pid_line(
                " 4242 /Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
            )
            .expect("valid ps line"),
            None
        );
        assert!(parse_macos_codex_app_server_pid_line("not-a-process").is_err());
    }

    #[test]
    fn ps_output_parser_fails_closed_on_command_or_output_errors() {
        let output = b" 4242 /Applications/ChatGPT.app/Contents/Resources/codex app-server\n 99 /usr/bin/other\n";
        assert_eq!(
            super::parse_macos_codex_app_server_ps_output(true, output, b"")
                .expect("valid ps output"),
            vec![4242]
        );
        assert!(super::parse_macos_codex_app_server_ps_output(
            false,
            b"",
            b"ps: permission denied"
        )
        .is_err());
        assert!(
            super::parse_macos_codex_app_server_ps_output(true, b"not-a-process\n", b"")
                .is_err()
        );
        assert!(super::parse_macos_codex_app_server_ps_output(true, &[0xff], b"").is_err());
    }
}

#[cfg(test)]
mod codex_launch_args_tests {
    use super::{
        build_codex_app_launch_args, codex_managed_store_launch_unsafe_error,
        CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX,
    };

    #[test]
    fn keeps_user_launch_args_without_adding_remote_debugging() {
        assert!(build_codex_app_launch_args(&[]).is_empty());
        assert_eq!(
            build_codex_app_launch_args(&[
                " --remote-debugging-port=9333 ".to_string(),
                "".to_string(),
                " --disable-gpu ".to_string(),
            ]),
            vec![
                "--remote-debugging-port=9333".to_string(),
                "--disable-gpu".to_string(),
            ]
        );
    }

    #[test]
    fn managed_store_launch_error_is_machine_readable_and_keeps_causes() {
        let error = codex_managed_store_launch_unsafe_error("denied", "fallback failed");
        assert!(error.starts_with(CODEX_MANAGED_STORE_LAUNCH_UNSAFE_PREFIX));
        assert!(error.contains("direct_error=denied"));
        assert!(error.contains("powershell_error=fallback failed"));
    }
}

#[cfg(test)]
mod codex_path_migration_tests {
    use super::{
        is_codex_embedded_backend_executable, score_windows_candidate,
        should_migrate_legacy_codex_launch_path,
    };
    use std::collections::HashSet;
    use std::path::Path;

    #[test]
    fn migrates_official_windows_store_codex_path_when_chatgpt_exists() {
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_2.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
            ),
        ));
    }

    #[test]
    fn keeps_legacy_path_when_chatgpt_is_not_detected() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__8wekyb3d8bbwe\app\Codex.exe"
            ),
        ));
    }

    #[test]
    fn does_not_replace_custom_codex_executable() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new(r"D:\Tools\Codex.exe"),
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.ChatGPT_2.0.0.0_x64__8wekyb3d8bbwe\app\ChatGPT.exe"
            ),
        ));
    }

    #[test]
    fn migrates_official_macos_codex_path_when_chatgpt_exists() {
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app"),
            Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
        ));
        assert!(should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app/Contents/MacOS/Codex"),
            Path::new("/Applications/ChatGPT.app"),
        ));
    }

    #[test]
    fn keeps_macos_legacy_path_when_chatgpt_is_not_detected() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new("/Applications/Codex.app"),
            Path::new("/Applications/Codex.app/Contents/MacOS/Codex"),
        ));
    }

    #[test]
    fn does_not_replace_custom_macos_codex_path() {
        assert!(!should_migrate_legacy_codex_launch_path(
            Path::new("/Users/test/Applications/Codex.app"),
            Path::new("/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"),
        ));
    }

    #[test]
    fn scan_rejects_codex_keyword_helper_executables() {
        let exe_names = HashSet::from(["chatgpt.exe".to_string(), "codex.exe".to_string()]);
        let keywords = vec!["chatgpt".to_string(), "codex".to_string()];

        assert!(score_windows_candidate(
            Path::new("C:/Tools/CodexHelper.exe"),
            &exe_names,
            &keywords,
        )
        .is_none());
        assert!(
            score_windows_candidate(Path::new("C:/Tools/ChatGPT.exe"), &exe_names, &keywords,)
                .is_some()
        );
    }

    #[test]
    fn scan_excludes_embedded_resources_backend() {
        assert!(is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
        )));
        assert!(!is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        )));
        assert!(!is_codex_embedded_backend_executable(Path::new(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0.0.0_x64__2p2nqsd0c76g0\app\Codex.exe"
        )));
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::{
        running_app_candidate_matches, windows_app_launch_signature,
        windows_trae_candidate_matches_platform,
    };
    use crate::modules::trae_account::TraePlatformKind;
    use std::path::Path;

    #[test]
    fn windows_launch_signatures_cover_provider_apps() {
        for app in [
            "antigravity_ide",
            "cursor",
            "zed",
            "codebuddy",
            "codebuddy_cn",
            "qoder",
            "trae",
            "trae_solo",
            "trae_cn",
            "trae_solo_cn",
            "workbuddy",
            "windsurf",
            "kiro",
            "codex",
            "claude",
            "vscode",
        ] {
            let signature =
                windows_app_launch_signature(app).unwrap_or_else(|| panic!("missing {app}"));
            assert!(
                !signature.exe_names.is_empty(),
                "{app} must define executable names"
            );
            assert!(
                !signature.common_paths.is_empty(),
                "{app} must define common install paths"
            );
            assert!(
                !signature.display_keywords.is_empty(),
                "{app} must define display keywords"
            );
        }
    }

    #[test]
    fn antigravity_ide_signature_uses_ide_executable_only() {
        let signature = windows_app_launch_signature("antigravity_ide")
            .expect("antigravity ide signature must exist");
        assert!(signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Antigravity IDE.exe")));
        assert!(!signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Antigravity.exe")));
    }

    #[test]
    fn codex_signature_accepts_chatgpt_and_legacy_codex_executables() {
        let signature = windows_app_launch_signature("codex").expect("codex signature must exist");
        assert!(signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("ChatGPT.exe")));
        assert!(signature
            .exe_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Codex.exe")));
    }

    #[test]
    fn running_codex_match_accepts_main_executable_and_rejects_embedded_backend() {
        let signature = windows_app_launch_signature("codex").expect("codex signature must exist");
        assert!(running_app_candidate_matches(
            "codex",
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
            ),
            signature,
        ));
        assert!(!running_app_candidate_matches(
            "codex",
            Path::new(
                r"C:\Program Files\WindowsApps\OpenAI.Codex_26.707.9564.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
            ),
            signature,
        ));
    }

    #[test]
    fn running_claude_match_requires_claude_executable() {
        let signature =
            windows_app_launch_signature("claude").expect("claude signature must exist");
        assert!(running_app_candidate_matches(
            "claude",
            Path::new(r"C:\Program Files\WindowsApps\Claude_1.0.0\app\Claude.exe"),
            signature,
        ));
        assert!(!running_app_candidate_matches(
            "claude",
            Path::new(r"C:\Tools\Electron.exe"),
            signature,
        ));
    }

    #[test]
    fn trae_windows_scan_candidates_match_exact_platform_dirs() {
        let trae = Path::new(r"D:\Users\李杰\AppData\Local\Programs\Trae\Trae.exe");
        let trae_cn = Path::new(r"D:\Users\李杰\AppData\Local\Programs\Trae CN\Trae CN.exe");
        let solo_cn =
            Path::new(r"D:\Users\李杰\AppData\Local\Programs\TRAE SOLO CN\TRAE SOLO CN.exe");

        assert!(windows_trae_candidate_matches_platform(
            trae,
            TraePlatformKind::Trae
        ));
        assert!(!windows_trae_candidate_matches_platform(
            trae,
            TraePlatformKind::TraeCn
        ));
        assert!(windows_trae_candidate_matches_platform(
            trae_cn,
            TraePlatformKind::TraeCn
        ));
        assert!(!windows_trae_candidate_matches_platform(
            trae_cn,
            TraePlatformKind::Trae
        ));
        assert!(windows_trae_candidate_matches_platform(
            solo_cn,
            TraePlatformKind::TraeSoloCn
        ));
        assert!(!windows_trae_candidate_matches_platform(
            solo_cn,
            TraePlatformKind::TraeSolo
        ));
    }
}
