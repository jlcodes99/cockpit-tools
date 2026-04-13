use std::collections::{HashMap, HashSet};
#[cfg(not(target_os = "macos"))]
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(target_os = "macos"))]
use std::process::Stdio;
use std::sync::Mutex;

use chrono::Utc;
#[cfg(not(target_os = "macos"))]
use sysinfo::{ProcessRefreshKind, System, UpdateKind};
use uuid::Uuid;

use crate::models::{DefaultInstanceSettings, InstanceProfile, InstanceStore};
use crate::modules;
use crate::modules::cursor_account;
use crate::modules::instance::InstanceDefaults;
use crate::modules::instance_store;

pub use crate::modules::instance_store::{CreateInstanceParams, UpdateInstanceParams};

static CURSOR_INSTANCE_STORE_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const CURSOR_INSTANCES_FILE: &str = "cursor_instances.json";

fn instances_path() -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    Ok(data_dir.join(CURSOR_INSTANCES_FILE))
}

pub fn load_instance_store() -> Result<InstanceStore, String> {
    let path = instances_path()?;
    instance_store::load_instance_store(&path, CURSOR_INSTANCES_FILE)
}

pub fn save_instance_store(store: &InstanceStore) -> Result<(), String> {
    let path = instances_path()?;
    instance_store::save_instance_store(&path, CURSOR_INSTANCES_FILE, store)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    let store = load_instance_store()?;
    Ok(store.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    extra_args: Option<String>,
    follow_local_account: Option<bool>,
) -> Result<DefaultInstanceSettings, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let settings = &mut store.default_settings;

    // Cursor 实例不支持“跟随当前账号”，直接忽略 follow_local_account。
    if follow_local_account == Some(true) {
        settings.follow_local_account = false;
    }

    if let Some(bind) = bind_account_id {
        settings.bind_account_id = bind;
        settings.follow_local_account = false;
    }

    if let Some(args) = extra_args {
        settings.extra_args = args.trim().to_string();
    }

    let updated = settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn get_default_cursor_user_data_dir() -> Result<PathBuf, String> {
    cursor_account::get_default_cursor_data_dir()
}

pub fn get_default_instances_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/cursor"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join(".antigravity_cockpit\\instances\\cursor"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/cursor"));
    }

    #[allow(unreachable_code)]
    Err("Cursor 多开实例仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_instance_defaults() -> Result<InstanceDefaults, String> {
    let root_dir = get_default_instances_root_dir()?;
    let default_user_data_dir = get_default_cursor_user_data_dir()?;
    Ok(InstanceDefaults {
        root_dir: root_dir.to_string_lossy().to_string(),
        default_user_data_dir: default_user_data_dir.to_string_lossy().to_string(),
    })
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;

    let name = instance_store::normalize_name(&params.name)?;
    let user_data_dir = params.user_data_dir.trim().to_string();
    if user_data_dir.is_empty() {
        return Err("实例目录不能为空".to_string());
    }

    instance_store::ensure_unique(&store, &name, &user_data_dir, None)?;

    let user_dir_path = PathBuf::from(&user_data_dir);
    let init_mode = params
        .init_mode
        .as_deref()
        .unwrap_or("copy")
        .to_ascii_lowercase();
    let create_empty = init_mode == "empty";
    let use_existing_dir = init_mode == "existingdir" || init_mode == "existing_dir";

    if use_existing_dir {
        if !user_dir_path.exists() {
            let resolved = instance_store::display_path(&user_dir_path);
            return Err(format!("所选目录不存在: {}", resolved));
        }
        if !user_dir_path.is_dir() {
            return Err("所选路径不是目录".to_string());
        }
    } else if create_empty {
        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                return Err(format!("空白实例需要目标目录为空: {}", resolved_path));
            }
        }
        fs::create_dir_all(&user_dir_path).map_err(|e| format!("创建实例目录失败: {}", e))?;
    } else {
        let source_dir = match params.copy_source_instance_id.as_deref() {
            Some("__default__") | None => get_default_cursor_user_data_dir()?,
            Some(source_id) => {
                let source_instance = store
                    .instances
                    .iter()
                    .find(|item| item.id == source_id)
                    .ok_or("复制来源实例不存在")?;
                PathBuf::from(&source_instance.user_data_dir)
            }
        };

        if user_dir_path.exists() {
            let mut has_entries = false;
            if let Ok(mut iter) = fs::read_dir(&user_dir_path) {
                if iter.next().is_some() {
                    has_entries = true;
                }
            }
            if has_entries {
                let resolved_path = instance_store::display_path(&user_dir_path);
                return Err(format!("复制来源实例需要目标目录为空: {}", resolved_path));
            }
        }

        if !source_dir.exists() {
            return Err("未找到复制来源目录，请先确保来源实例已初始化".to_string());
        }

        instance_store::copy_dir_recursive(&source_dir, &user_dir_path)?;
    }

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir,
        working_dir: params.working_dir,
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: if create_empty {
            None
        } else if use_existing_dir {
            params.bind_account_id
        } else {
            params.bind_account_id
        },
        launch_mode: crate::models::InstanceLaunchMode::App,
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };

    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == params.instance_id)
        .ok_or("实例不存在")?;

    let current_id = store.instances[index].id.clone();
    let current_dir = store.instances[index].user_data_dir.clone();
    let next_name = params
        .name
        .as_ref()
        .map(|name| instance_store::normalize_name(name))
        .transpose()?;

    if let Some(ref normalized) = next_name {
        instance_store::ensure_unique(&store, normalized, &current_dir, Some(&current_id))?;
    }

    let instance = &mut store.instances[index];
    if let Some(normalized) = next_name {
        instance.name = normalized;
    }
    if let Some(ref extra_args) = params.extra_args {
        instance.extra_args = extra_args.trim().to_string();
    }
    if let Some(bind) = params.bind_account_id.clone() {
        instance.bind_account_id = bind;
    }

    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn delete_instance(instance_id: &str) -> Result<(), String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == instance_id)
        .ok_or("实例不存在")?;
    let user_data_dir = store.instances[index].user_data_dir.clone();

    if !user_data_dir.trim().is_empty() {
        let dir_path = PathBuf::from(&user_data_dir);
        modules::instance::delete_instance_directory(&dir_path)?;
    }

    store.instances.remove(index);
    save_instance_store(&store)?;
    Ok(())
}

pub fn update_instance_after_start(instance_id: &str, pid: u32) -> Result<InstanceProfile, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut updated = None;
    for instance in &mut store.instances {
        if instance.id == instance_id {
            instance.last_launched_at = Some(Utc::now().timestamp_millis());
            instance.last_pid = Some(pid);
            updated = Some(instance.clone());
            break;
        }
    }
    let updated = updated.ok_or("实例不存在")?;
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_instance_pid(instance_id: &str, pid: Option<u32>) -> Result<InstanceProfile, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut updated = None;
    for instance in &mut store.instances {
        if instance.id == instance_id {
            instance.last_pid = pid;
            updated = Some(instance.clone());
            break;
        }
    }
    let updated = updated.ok_or("实例不存在")?;
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_default_pid(pid: Option<u32>) -> Result<DefaultInstanceSettings, String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = pid;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn clear_all_pids() -> Result<(), String> {
    let _lock = CURSOR_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = None;
    for instance in &mut store.instances {
        instance.last_pid = None;
    }
    save_instance_store(&store)?;
    Ok(())
}

fn normalize_path_for_compare(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let resolved = fs::canonicalize(trimmed)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| trimmed.to_string());

    #[cfg(target_os = "windows")]
    {
        return resolved.to_lowercase();
    }
    #[cfg(not(target_os = "windows"))]
    {
        resolved
    }
}

fn normalize_non_empty_path(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_path_for_compare)
        .filter(|text| !text.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn parse_user_data_dir_value(raw: &str) -> Option<String> {
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
        let end = rest.find(" --").unwrap_or(rest.len());
        &rest[..end]
    };
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
fn extract_user_data_dir(args: &[OsString]) -> Option<String> {
    let tokens: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if let Some(rest) = token.strip_prefix("--user-data-dir=") {
            return parse_user_data_dir_value(rest);
        }
        if token == "--user-data-dir" {
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
            if parts.is_empty() {
                return None;
            }
            return Some(parts.join(" "));
        }
        index += 1;
    }
    None
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(not(target_os = "macos"))]
fn is_helper_process(name: &str, args_line: &str) -> bool {
    args_line.contains("--type=")
        || name.contains("helper")
        || name.contains("renderer")
        || name.contains("gpu")
        || name.contains("utility")
        || name.contains("crashpad")
        || name.contains("sandbox")
}

fn command_trace_enabled() -> bool {
    if let Ok(value) = std::env::var("COCKPIT_COMMAND_TRACE") {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => return true,
            "0" | "false" | "no" | "off" => return false,
            _ => {}
        }
    }
    false
}

fn quote_command_part(part: &str) -> String {
    if part.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quote = part
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '\'' | '$' | '`' | '|' | '&' | ';'));
    if needs_quote {
        format!("{:?}", part)
    } else {
        part.to_string()
    }
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
    modules::process::summarize_text_for_process_log(&preview, 600)
}

fn spawn_command_with_trace(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    let preview = format_command_preview(cmd);
    if command_trace_enabled() {
        modules::logger::log_info(&format!("[CmdTrace][Cursor] EXEC {}", preview));
    }
    let start = std::time::Instant::now();
    let result = cmd.spawn();
    if command_trace_enabled() {
        match &result {
            Ok(child) => modules::logger::log_info(&format!(
                "[CmdTrace][Cursor] SPAWN elapsed={}ms pid={} cmd={}",
                start.elapsed().as_millis(),
                child.id(),
                preview
            )),
            Err(err) => modules::logger::log_warn(&format!(
                "[CmdTrace][Cursor] SPAWN_ERROR elapsed={}ms cmd={} err={}",
                start.elapsed().as_millis(),
                preview,
                err
            )),
        }
    }
    result
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
                // Extract exe path: for .app bundles, find Contents/MacOS/ boundary
                let lower = cmdline.to_lowercase();
                let exe = if let Some(contents_pos) = lower.find(".app/contents/macos/") {
                    let after = contents_pos + ".app/contents/macos/".len();
                    let rest = &cmdline[after..];
                    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                    &cmdline[..after + end]
                } else {
                    cmdline.split_whitespace().next().unwrap_or("")
                };
                if !exe.is_empty() {
                    let normalized = normalize_path_for_compare(exe);
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

fn resolve_expected_cursor_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_cursor_launch_path() {
        Ok(path) => path,
        Err(err) => {
            modules::logger::log_warn(&format!(
                "[Cursor Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        modules::logger::log_warn("[Cursor Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

fn filter_cursor_entries_by_launch_path(
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
    let mut result = Vec::new();
    let mut missing_exe = 0usize;
    let mut path_mismatch = 0usize;
    for (pid, dir) in entries {
        match exe_by_pid.get(&pid) {
            Some(actual) if actual == &expected => result.push((pid, dir)),
            Some(_) => path_mismatch += 1,
            None => missing_exe += 1,
        }
    }
    if result.is_empty() {
        modules::logger::log_warn(&format!(
            "[Cursor Resolve] 启动路径硬匹配未命中：expected={}, path_mismatch={}, missing_exe={}",
            expected, path_mismatch, missing_exe
        ));
    }
    result
}

pub fn collect_cursor_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_cursor_launch_path_for_match();
    if expected_launch.is_none() {
        return Vec::new();
    }

    let mut entries: HashMap<u32, Option<String>> = HashMap::new();

    // On macOS, skip sysinfo to avoid TCC dialogs
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
            let args_line = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");

            #[cfg(target_os = "windows")]
            let is_cursor = name == "cursor.exe"
                || exe_path.ends_with("\\cursor.exe")
                || (name == "electron.exe" && exe_path.contains("\\cursor\\"));
            #[cfg(target_os = "linux")]
            let is_cursor = name.contains("cursor") || exe_path.contains("/cursor");

            if !is_cursor || is_helper_process(&name, &args_line) {
                continue;
            }

            let dir = extract_user_data_dir(process.cmd()).and_then(|value| {
                let normalized = normalize_path_for_compare(&value);
                if normalized.is_empty() {
                    None
                } else {
                    Some(normalized)
                }
            });
            entries.insert(pid_u32, dir);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = Command::new("ps").args(["-axo", "pid,command"]).output() {
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
                if !lower.contains("cursor.app/contents/") || lower.contains("--type=") {
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
                entries.entry(pid).or_insert(dir);
            }
        }
    }

    let mut result: Vec<(u32, Option<String>)> = entries.into_iter().collect();
    result.sort_by_key(|(pid, _)| *pid);
    filter_cursor_entries_by_launch_path(result, expected_launch)
}

fn pick_preferred_pid(mut pids: Vec<u32>) -> Option<u32> {
    if pids.is_empty() {
        return None;
    }
    pids.sort();
    pids.dedup();
    pids.first().copied()
}

pub fn resolve_cursor_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let default_dir = get_default_cursor_user_data_dir()
        .ok()
        .map(|dir| normalize_path_for_compare(&dir.to_string_lossy()));
    let target = normalize_non_empty_path(user_data_dir).or(default_dir.clone());
    let allow_none_for_target = default_dir
        .as_ref()
        .zip(target.as_ref())
        .map(|(value, current)| value == current)
        .unwrap_or(false);

    let target = target?;

    let mut matches = Vec::new();
    for (pid, dir) in entries {
        match dir.as_ref() {
            Some(actual_dir) => {
                let normalized = normalize_path_for_compare(actual_dir);
                if !normalized.is_empty() && normalized == target {
                    matches.push(*pid);
                }
            }
            None if allow_none_for_target => matches.push(*pid),
            _ => {}
        }
    }

    if let Some(pid) = last_pid {
        if modules::process::is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if modules::process::is_pid_running(pid) {
            modules::logger::log_warn(&format!(
                "[Cursor Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                modules::process::summarize_text_for_process_log(&target, 96),
                modules::process::summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

pub fn resolve_cursor_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_cursor_process_entries();
    resolve_cursor_pid_from_entries(last_pid, user_data_dir, &entries)
}

#[cfg(target_os = "macos")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {}) to true",
        pid
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|e| format!("调用 osascript 失败: {}", e))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("定位 Cursor 窗口失败: {}", stderr.trim()))
}

#[cfg(target_os = "windows")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

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
    let output = Command::new("powershell")
        .creation_flags(0x08000000)
        .args(["-NoProfile", "-NonInteractive", "-Command", &command])
        .output()
        .map_err(|e| format!("调用 PowerShell 失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("定位 Cursor 窗口失败: {}", stderr.trim()))
    }
}

#[cfg(target_os = "linux")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    let output = Command::new("xdotool")
        .args(["search", "--pid", &pid.to_string(), "windowactivate"])
        .output()
        .map_err(|e| format!("调用 xdotool 失败: {}", e))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("定位 Cursor 窗口失败: {}", stderr.trim()))
    }
}

pub fn focus_cursor_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let pid = resolve_cursor_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    focus_window_by_pid(pid)?;
    Ok(pid)
}

fn normalize_custom_path(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root(path: &Path) -> Option<String> {
    let path_str = path.to_string_lossy();
    if let Some(index) = path_str.find(".app") {
        return Some(path_str[..index + 4].to_string());
    }
    None
}

#[cfg(target_os = "macos")]
fn resolve_macos_exec_path(path_str: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let cursor_exec = PathBuf::from(&app_root)
            .join("Contents")
            .join("MacOS")
            .join("Cursor");
        if cursor_exec.exists() {
            return Some(cursor_exec);
        }
        let electron_exec = PathBuf::from(&app_root)
            .join("Contents")
            .join("MacOS")
            .join("Electron");
        if electron_exec.exists() {
            return Some(electron_exec);
        }
    }
    if path.exists() {
        return Some(path);
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn resolve_macos_exec_path(path_str: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path_str);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn detect_cursor_exec_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // On macOS, check well-known paths first to avoid sysinfo TCC dialogs
        let candidates = [
            "/Applications/Cursor.app/Contents/MacOS/Cursor",
            "/Applications/Cursor.app/Contents/MacOS/Electron",
        ];
        for candidate in candidates {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        // Fallback: try to find from running processes via ps
        for (pid, _) in collect_cursor_process_entries() {
            if let Ok(output) = Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "command="])
                .output()
            {
                let cmdline = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let lower = cmdline.to_lowercase();
                if let Some(contents_pos) = lower.find(".app/contents/macos/") {
                    let after = contents_pos + ".app/contents/macos/".len();
                    let rest = &cmdline[after..];
                    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
                    return Some(PathBuf::from(&cmdline[..after + end]));
                }
            }
        }
        return None;
    }

    #[cfg(not(target_os = "macos"))]
    {
        for (pid, _) in collect_cursor_process_entries() {
            let mut system = System::new();
            system.refresh_processes_specifics(
                sysinfo::ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
            );
            if let Some(process) = system.process(sysinfo::Pid::from(pid as usize)) {
                if let Some(path) = process.exe() {
                    return Some(path.to_path_buf());
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
                candidates.push(
                    Path::new(&local_appdata)
                        .join("Programs")
                        .join("Cursor")
                        .join("Cursor.exe"),
                );
                candidates.push(
                    Path::new(&local_appdata)
                        .join("Programs")
                        .join("Cursor")
                        .join("Electron.exe"),
                );
            }
            for candidate in candidates {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
            if let Some(path) = modules::process::detect_windows_exec_path_by_signatures(
                "cursor",
                &["Cursor.exe", "Electron.exe"],
                &["cursor"],
                &["cursor"],
                &["cursor"],
            ) {
                return Some(path);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let candidates = ["/usr/bin/cursor", "/opt/cursor/cursor"];
            for candidate in candidates {
                let path = PathBuf::from(candidate);
                if path.exists() {
                    return Some(path);
                }
            }
        }

        return None;
    }
}

fn path_looks_like_cursor(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase();
    text.contains("cursor")
}

fn normalize_cursor_path_for_config(path: &Path) -> String {
    #[cfg(target_os = "macos")]
    {
        return normalize_macos_app_root(path)
            .unwrap_or_else(|| path.to_string_lossy().to_string());
    }
    #[cfg(not(target_os = "macos"))]
    {
        path.to_string_lossy().to_string()
    }
}

pub fn detect_and_save_cursor_launch_path(force: bool) -> Option<String> {
    let current = modules::config::get_user_config();
    if !force && normalize_custom_path(&current.cursor_app_path).is_some() {
        return Some(current.cursor_app_path);
    }

    let detected = detect_cursor_exec_path()?;
    let normalized = normalize_cursor_path_for_config(&detected);
    if current.cursor_app_path != normalized {
        let mut next = current.clone();
        next.cursor_app_path = normalized.clone();
        if let Err(err) = modules::config::save_user_config(&next) {
            modules::logger::log_warn(&format!("保存 Cursor 启动路径失败（已忽略）: {}", err));
        }
    }
    Some(normalized)
}

fn resolve_cursor_launch_path() -> Result<PathBuf, String> {
    let config = modules::config::get_user_config();
    if let Some(custom) = normalize_custom_path(&config.cursor_app_path) {
        if let Some(exec) = resolve_macos_exec_path(&custom) {
            if path_looks_like_cursor(&exec) {
                return Ok(exec);
            }
            modules::logger::log_warn(&format!(
                "忽略非 Cursor 启动路径配置: {}",
                exec.to_string_lossy()
            ));
        }
        return Err("APP_PATH_NOT_FOUND:cursor".to_string());
    }

    Err("APP_PATH_NOT_FOUND:cursor".to_string())
}

pub fn ensure_cursor_launch_path_configured() -> Result<(), String> {
    resolve_cursor_launch_path().map(|_| ())
}

#[cfg(target_os = "macos")]
fn sanitize_macos_gui_launch_env(cmd: &mut Command) {
    // Avoid inheriting Cockpit bundle identity into child GUI apps.
    cmd.env_remove("__CFBundleIdentifier");
    cmd.env_remove("XPC_SERVICE_NAME");
}

#[cfg(target_os = "linux")]
fn sanitize_macos_gui_launch_env(_cmd: &mut Command) {}

#[cfg(target_os = "windows")]
fn spawn_cursor_windows(
    launch_path: &Path,
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;

    let mut cmd = Command::new(launch_path);
    crate::modules::process::apply_managed_proxy_env_to_command(&mut cmd);
    cmd.creation_flags(0x08000000);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.arg("--user-data-dir").arg(user_data_dir.trim());
    if use_new_window {
        cmd.arg("--new-window");
    } else {
        cmd.arg("--reuse-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }
    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Cursor 失败: {}", e))?;
    Ok(child.id())
}

#[cfg(target_os = "macos")]
fn spawn_cursor_macos_open(
    launch_path: &Path,
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let app_root = normalize_macos_app_root(launch_path).ok_or("APP_PATH_NOT_FOUND:cursor")?;
    let target = user_data_dir.trim();

    let mut cmd = Command::new("open");
    sanitize_macos_gui_launch_env(&mut cmd);
    crate::modules::process::append_managed_proxy_env_to_open_args(&mut cmd);
    cmd.arg("-n").arg("-a").arg(&app_root);
    cmd.arg("--args");
    cmd.arg("--user-data-dir").arg(target);
    if use_new_window {
        cmd.arg("--new-window");
    } else {
        cmd.arg("--reuse-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }

    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Cursor 失败: {}", e))?;
    modules::logger::log_info("Cursor 启动命令已发送（open -n -a）");
    let probe_started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(6);
    while probe_started.elapsed() < timeout {
        if let Some(resolved_pid) = resolve_cursor_pid(None, Some(target)) {
            return Ok(resolved_pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    modules::logger::log_warn(&format!(
        "[Cursor Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
        child.id()
    ));
    Ok(child.id())
}

#[cfg(target_os = "linux")]
fn spawn_cursor_unix(
    launch_path: &Path,
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let mut cmd = Command::new(launch_path);
    crate::modules::process::apply_managed_proxy_env_to_command(&mut cmd);
    sanitize_macos_gui_launch_env(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.arg("--user-data-dir").arg(user_data_dir.trim());
    if use_new_window {
        cmd.arg("--new-window");
    } else {
        cmd.arg("--reuse-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }
    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Cursor 失败: {}", e))?;
    Ok(child.id())
}

pub fn start_cursor_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let target = user_data_dir.trim();
    if target.is_empty() {
        return Err("实例目录为空，无法启动".to_string());
    }
    let launch_path = resolve_cursor_launch_path()?;

    #[cfg(target_os = "windows")]
    {
        return spawn_cursor_windows(&launch_path, target, extra_args, use_new_window);
    }

    #[cfg(target_os = "macos")]
    {
        return spawn_cursor_macos_open(&launch_path, target, extra_args, use_new_window);
    }

    #[cfg(target_os = "linux")]
    {
        return spawn_cursor_unix(&launch_path, target, extra_args, use_new_window);
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (target, extra_args, use_new_window);
        Err("Cursor 多开实例仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_cursor_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let default_dir = get_default_cursor_user_data_dir()?;
    start_cursor_with_args_with_new_window(
        &default_dir.to_string_lossy(),
        extra_args,
        use_new_window,
    )
}

pub fn close_cursor(user_data_dirs: &[String], timeout_secs: u64) -> Result<(), String> {
    let target_dirs: HashSet<String> = user_data_dirs
        .iter()
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty())
        .collect();
    if target_dirs.is_empty() {
        return Ok(());
    }

    let default_dir = get_default_cursor_user_data_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty());
    let allow_none_for_default = default_dir
        .as_ref()
        .map(|value| target_dirs.contains(value))
        .unwrap_or(false);

    let entries = collect_cursor_process_entries();
    let mut pids = Vec::new();
    for (pid, dir) in entries {
        match dir.as_ref() {
            Some(value) => {
                if target_dirs.contains(value) {
                    pids.push(pid);
                }
            }
            None if allow_none_for_default => pids.push(pid),
            _ => {}
        }
    }

    pids.sort();
    pids.dedup();
    if pids.is_empty() {
        return Ok(());
    }

    for pid in &pids {
        let _ = modules::process::close_pid(*pid, timeout_secs);
    }

    let still_running: Vec<u32> = pids
        .into_iter()
        .filter(|pid| modules::process::is_pid_running(*pid))
        .collect();
    if !still_running.is_empty() {
        return Err(format!(
            "无法关闭 Cursor 实例进程，请手动关闭后重试: {}",
            modules::process::summarize_pid_list_for_log(&still_running)
        ));
    }

    Ok(())
}

fn ensure_profile_global_storage(profile_dir: &Path) -> Result<PathBuf, String> {
    let global_storage = profile_dir.join("User").join("globalStorage");
    if !global_storage.exists() {
        fs::create_dir_all(&global_storage)
            .map_err(|e| format!("创建 globalStorage 失败: {}", e))?;
    }
    Ok(global_storage)
}

fn ensure_state_db_for_injection(profile_dir: &Path) -> Result<PathBuf, String> {
    let db_path = profile_dir
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");
    if db_path.exists() {
        return Ok(db_path);
    }

    let default_db = cursor_account::get_default_cursor_state_db_path()?;
    if default_db.exists() {
        let _ = ensure_profile_global_storage(profile_dir)?;
        fs::copy(&default_db, &db_path).map_err(|e| format!("复制 state.vscdb 失败: {}", e))?;
    }

    if !db_path.exists() {
        return Err("未找到 state.vscdb，请先勾选复制当前登录状态或先启动实例一次".to_string());
    }

    let default_storage = cursor_account::get_default_cursor_data_dir()?
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    let target_storage = profile_dir
        .join("User")
        .join("globalStorage")
        .join("storage.json");
    if default_storage.exists() && !target_storage.exists() {
        let _ = fs::copy(&default_storage, &target_storage);
    }

    Ok(db_path)
}

pub fn inject_account_to_profile(profile_dir: &Path, account_id: &str) -> Result<(), String> {
    let account = cursor_account::load_account(account_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", account_id))?;
    let db_path = ensure_state_db_for_injection(profile_dir)?;
    cursor_account::inject_to_cursor_at_path(&db_path, account_id)?;
    modules::logger::log_info(&format!(
        "Cursor 账号注入完成: email={}, db={}",
        account.email,
        db_path.to_string_lossy()
    ));
    Ok(())
}
