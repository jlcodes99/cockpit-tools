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
use serde::Serialize;
#[cfg(not(target_os = "macos"))]
use sysinfo::{ProcessRefreshKind, System, UpdateKind};
use uuid::Uuid;

use crate::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile, InstanceStore};
use crate::modules;
use crate::modules::instance::InstanceDefaults;
use crate::modules::instance_store;

static CLAUDE_INSTANCE_STORE_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

#[cfg(target_os = "windows")]
const WINDOWS_CLAUDE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const CLAUDE_INSTANCES_FILE: &str = "claude_instances.json";
const CLAUDE_GLOBAL_CONFIG_FILE: &str = ".claude.json";
const CLAUDE_CODE_CONFIG_FILE: &str = ".config.json";
const CLAUDE_CREDENTIALS_FILE: &str = ".credentials.json";
const CLAUDE_DESKTOP_CONFIG_FILE: &str = "config.json";
const CLAUDE_CODE_SETTINGS_FILE: &str = "settings.json";
const CLAUDE_USER_DATA_DIR_ENV: &str = "CLAUDE_USER_DATA_DIR";

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeDesktopLaunchCandidate {
    pub target_type: String,
    pub label: String,
    pub target: String,
    pub source: String,
    pub supports_multi_instance: bool,
}

#[derive(Debug, Clone)]
pub struct CreateInstanceParams {
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    pub copy_source_instance_id: Option<String>,
    pub init_mode: Option<String>,
    pub launch_mode: Option<InstanceLaunchMode>,
}

#[derive(Debug, Clone)]
pub struct UpdateInstanceParams {
    pub instance_id: String,
    pub name: Option<String>,
    pub working_dir: Option<String>,
    pub extra_args: Option<String>,
    pub bind_account_id: Option<Option<String>>,
    pub launch_mode: Option<InstanceLaunchMode>,
}

fn instances_path() -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    Ok(data_dir.join(CLAUDE_INSTANCES_FILE))
}

pub fn is_profile_initialized(config_dir: &Path) -> bool {
    config_dir.join(CLAUDE_DESKTOP_CONFIG_FILE).exists()
        || config_dir.join("Cookies").exists()
        || config_dir.join("Local Storage").exists()
        || config_dir.join("IndexedDB").exists()
        || config_dir.join("Session Storage").exists()
        || config_dir.join("Network").join("Cookies").exists()
        || config_dir.join("Network").join("Cookies-journal").exists()
        || config_dir.join(CLAUDE_CREDENTIALS_FILE).exists()
        || config_dir.join(CLAUDE_CODE_CONFIG_FILE).exists()
        || config_dir.join(CLAUDE_GLOBAL_CONFIG_FILE).exists()
        || config_dir.join(CLAUDE_CODE_SETTINGS_FILE).exists()
}

pub fn load_instance_store() -> Result<InstanceStore, String> {
    let path = instances_path()?;
    instance_store::load_instance_store(&path, CLAUDE_INSTANCES_FILE)
}

pub fn save_instance_store(store: &InstanceStore) -> Result<(), String> {
    let path = instances_path()?;
    instance_store::save_instance_store(&path, CLAUDE_INSTANCES_FILE, store)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    let store = load_instance_store()?;
    Ok(store.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
) -> Result<DefaultInstanceSettings, String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let settings = &mut store.default_settings;

    settings.follow_local_account = false;
    if let Some(bind) = bind_account_id {
        settings.bind_account_id = bind;
    }
    if let Some(dir) = working_dir {
        settings.working_dir = if dir.trim().is_empty() {
            None
        } else {
            Some(dir.trim().to_string())
        };
    }
    if let Some(args) = extra_args {
        settings.extra_args = args.trim().to_string();
    }
    if let Some(mode) = launch_mode {
        settings.launch_mode = mode;
    }
    if follow_local_account.is_some() {
        settings.follow_local_account = false;
    }

    let updated = settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn get_default_claude_config_dir() -> Result<PathBuf, String> {
    modules::claude_account::get_default_claude_desktop_user_data_dir()
}

pub fn get_default_claude_cli_config_dir() -> Result<PathBuf, String> {
    modules::claude_account::get_default_claude_code_config_dir()
}

pub fn get_default_instances_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/claude"));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join(".antigravity_cockpit\\instances\\claude"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/claude"));
    }

    #[allow(unreachable_code)]
    Err("Claude 应用多开仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_instance_defaults() -> Result<InstanceDefaults, String> {
    let root_dir = get_default_instances_root_dir()?;
    let default_user_data_dir = get_default_claude_config_dir()?;
    Ok(InstanceDefaults {
        root_dir: root_dir.to_string_lossy().to_string(),
        default_user_data_dir: default_user_data_dir.to_string_lossy().to_string(),
    })
}

fn ensure_target_root_empty(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err("所选路径不是目录".to_string());
    }
    if fs::read_dir(path)
        .map(|mut iter| iter.next().is_some())
        .unwrap_or(false)
    {
        return Err(format!(
            "复制来源实例需要目标目录为空: {}",
            instance_store::display_path(path)
        ));
    }
    Ok(())
}

fn resolve_copy_source_config_dir(
    store: &InstanceStore,
    copy_source_instance_id: Option<&str>,
    launch_mode: &InstanceLaunchMode,
) -> Result<PathBuf, String> {
    match copy_source_instance_id {
        Some("__default__") | None => match launch_mode {
            InstanceLaunchMode::Cli => get_default_claude_cli_config_dir(),
            InstanceLaunchMode::App => get_default_claude_config_dir(),
        },
        Some(source_id) => {
            let source = store
                .instances
                .iter()
                .find(|item| item.id == source_id)
                .ok_or("复制来源实例不存在")?;
            if &source.launch_mode != launch_mode {
                return Err("复制来源实例的启动方式不同，请选择同类型实例".to_string());
            }
            Ok(PathBuf::from(&source.user_data_dir))
        }
    }
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
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
    let launch_mode = params.launch_mode.unwrap_or_default();
    let create_empty = init_mode == "empty";
    let use_existing_dir = init_mode == "existingdir" || init_mode == "existing_dir";

    if use_existing_dir {
        if !user_dir_path.exists() {
            return Err(format!(
                "所选目录不存在: {}",
                instance_store::display_path(&user_dir_path)
            ));
        }
        if !user_dir_path.is_dir() {
            return Err("所选路径不是目录".to_string());
        }
    } else if create_empty {
        ensure_target_root_empty(&user_dir_path)?;
        fs::create_dir_all(&user_dir_path).map_err(|e| format!("创建实例目录失败: {}", e))?;
    } else {
        ensure_target_root_empty(&user_dir_path)?;
        let source_id = params.copy_source_instance_id.as_deref();
        let source_dir = resolve_copy_source_config_dir(&store, source_id, &launch_mode)?;
        if !source_dir.exists() {
            let is_default_cli_source = launch_mode == InstanceLaunchMode::Cli
                && matches!(source_id, Some("__default__") | None);
            if is_default_cli_source {
                fs::create_dir_all(&user_dir_path)
                    .map_err(|e| format!("创建实例目录失败: {}", e))?;
            } else {
                return Err("未找到复制来源目录，请先确保来源实例已初始化".to_string());
            }
        } else {
            fs::create_dir_all(&user_dir_path).map_err(|e| format!("创建实例目录失败: {}", e))?;
            instance_store::copy_dir_recursive(&source_dir, &user_dir_path)?;
        }
    }

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir,
        working_dir: params.working_dir,
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: if create_empty {
            None
        } else {
            params.bind_account_id
        },
        launch_mode,
        app_speed: crate::models::codex::CodexAppSpeed::Standard,
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };

    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|item| item.id == params.instance_id)
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
    if let Some(working_dir) = params.working_dir {
        instance.working_dir = if working_dir.trim().is_empty() {
            None
        } else {
            Some(working_dir.trim().to_string())
        };
    }
    if let Some(extra_args) = params.extra_args {
        instance.extra_args = extra_args.trim().to_string();
    }
    if let Some(bind) = params.bind_account_id {
        instance.bind_account_id = bind;
    }
    if let Some(mode) = params.launch_mode {
        instance.launch_mode = mode;
    }

    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn delete_instance(instance_id: &str) -> Result<(), String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let user_data_dir = store.instances[index].user_data_dir.clone();
    if !user_data_dir.trim().is_empty() {
        modules::instance::delete_instance_directory(&PathBuf::from(&user_data_dir))?;
    }
    store.instances.remove(index);
    save_instance_store(&store)?;
    Ok(())
}

pub fn update_instance_last_launched(instance_id: &str) -> Result<InstanceProfile, String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut updated = None;
    for instance in &mut store.instances {
        if instance.id == instance_id {
            instance.last_launched_at = Some(Utc::now().timestamp_millis());
            instance.last_pid = None;
            updated = Some(instance.clone());
            break;
        }
    }
    let updated = updated.ok_or("实例不存在")?;
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_instance_after_start(instance_id: &str, pid: u32) -> Result<InstanceProfile, String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
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
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
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
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = pid;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn clear_all_pids() -> Result<(), String> {
    let _lock = CLAUDE_INSTANCE_STORE_LOCK
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
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| trimmed.to_string());

    #[cfg(target_os = "windows")]
    {
        resolved.to_lowercase()
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

#[cfg(target_os = "macos")]
fn extract_macos_exe_from_command_line(command_line: &str) -> Option<String> {
    let lower = command_line.to_lowercase();
    if let Some(contents_pos) = lower.find(".app/contents/macos/") {
        let after = contents_pos + ".app/contents/macos/".len();
        let rest = &command_line[after..];
        let end = rest
            .find(|ch: char| ch.is_whitespace())
            .unwrap_or(rest.len());
        let exe = &command_line[..after + end];
        if !exe.trim().is_empty() {
            return Some(exe.to_string());
        }
    }
    command_line
        .split_whitespace()
        .next()
        .map(|value| value.to_string())
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
        modules::logger::log_info(&format!("[CmdTrace][Claude] EXEC {}", preview));
    }
    let start = std::time::Instant::now();
    let result = cmd.spawn();
    if command_trace_enabled() {
        match &result {
            Ok(child) => modules::logger::log_info(&format!(
                "[CmdTrace][Claude] SPAWN elapsed={}ms pid={} cmd={}",
                start.elapsed().as_millis(),
                child.id(),
                preview
            )),
            Err(err) => modules::logger::log_warn(&format!(
                "[CmdTrace][Claude] SPAWN_ERROR elapsed={}ms cmd={} err={}",
                start.elapsed().as_millis(),
                preview,
                err
            )),
        }
    }
    result
}

fn normalize_custom_path(value: &str) -> Option<String> {
    let normalized = modules::process::normalize_windows_user_facing_path(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(target_os = "macos")]
fn normalize_macos_app_root(path: &Path) -> Option<String> {
    let path_str = path.to_string_lossy();
    path_str
        .find(".app")
        .map(|index| path_str[..index + 4].to_string())
}

#[cfg(target_os = "macos")]
fn read_macos_bundle_executable(app_root: &Path) -> Option<String> {
    let plist_path = app_root.join("Contents").join("Info.plist");
    if !plist_path.exists() {
        return None;
    }
    let output = Command::new("plutil")
        .arg("-p")
        .arg(&plist_path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("\"CFBundleExecutable\"") {
            continue;
        }
        let value = line.split("=>").nth(1)?.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn first_macos_bundle_executable(app_root: &Path) -> Option<PathBuf> {
    let macos_dir = app_root.join("Contents").join("MacOS");
    let mut candidates = fs::read_dir(macos_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

#[cfg(target_os = "macos")]
fn resolve_macos_exec_path(path_str: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path_str);
    if let Some(app_root) = normalize_macos_app_root(&path) {
        let app_root_path = PathBuf::from(&app_root);
        if let Some(executable) = read_macos_bundle_executable(&app_root_path) {
            let candidate = app_root_path
                .join("Contents")
                .join("MacOS")
                .join(executable);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if let Some(candidate) = first_macos_bundle_executable(&app_root_path) {
            return Some(candidate);
        }
    }
    if path.exists() {
        Some(path)
    } else {
        None
    }
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

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
enum ClaudeWindowsLaunchTarget {
    Executable(PathBuf),
    ShellApp(String),
}

#[cfg(target_os = "windows")]
const WINDOWS_SHELL_APP_PROCESS_MATCH: &str = "__claude_windows_shell_app__";

#[cfg(target_os = "windows")]
const CLAUDE_MULTI_INSTANCE_REQUIRES_EXE_ERROR: &str = "CLAUDE_MULTI_INSTANCE_REQUIRES_EXE:claude";

#[cfg(target_os = "windows")]
fn is_windows_claude_code_cli_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if file_name != "claude.exe" {
        return false;
    }

    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\.local\\bin\\claude.exe")
        || normalized.contains("\\claude-code\\")
        || normalized.contains("\\node_modules\\@anthropic-ai\\claude-code\\")
}

#[cfg(target_os = "windows")]
fn is_windows_claude_code_cli_process(exe_path: &str, args_line: &str) -> bool {
    is_windows_claude_code_cli_path(Path::new(exe_path))
        || args_line.contains("\\.local\\bin\\claude.exe")
        || args_line.contains("\\claude-code\\")
        || args_line.contains("\\node_modules\\@anthropic-ai\\claude-code\\")
}

#[cfg(target_os = "windows")]
fn is_windows_claude_desktop_exec_path(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    path.exists()
        && path.is_file()
        && file_name.eq_ignore_ascii_case("claude.exe")
        && !is_windows_claude_code_cli_path(path)
}

#[cfg(target_os = "windows")]
fn log_ignored_windows_claude_path(path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "[Claude Resolve] ignored non-Desktop Claude launch path: path={}, reason={}",
        path.to_string_lossy(),
        reason
    ));
}

#[cfg(target_os = "windows")]
fn normalize_windows_shell_app_target(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("shell:appsfolder\\") || lower.starts_with("shell:appsfolder/") {
        return Some(trimmed.replace('/', "\\"));
    }
    if trimmed.contains('!') && !trimmed.contains('\\') && !trimmed.contains('/') {
        return Some(format!(r"shell:AppsFolder\{}", trimmed));
    }
    None
}

#[cfg(target_os = "windows")]
fn resolve_windows_claude_launch_target(
    path_str: &str,
) -> Result<ClaudeWindowsLaunchTarget, String> {
    if let Some(shell_target) = normalize_windows_shell_app_target(path_str) {
        return Ok(ClaudeWindowsLaunchTarget::ShellApp(shell_target));
    }

    let path = PathBuf::from(path_str);
    if is_windows_claude_desktop_exec_path(&path) {
        return Ok(ClaudeWindowsLaunchTarget::Executable(path));
    }
    if path.exists() {
        log_ignored_windows_claude_path(&path, "configured path is not Claude Desktop");
    }
    Err("APP_PATH_NOT_FOUND:claude".to_string())
}

#[cfg(target_os = "windows")]
fn split_scan_roots(value: Option<&str>) -> Vec<PathBuf> {
    value
        .unwrap_or("")
        .lines()
        .flat_map(|line| line.split(';'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_drive_roots() -> Vec<PathBuf> {
    ('C'..='Z')
        .map(|drive| PathBuf::from(format!("{}:\\", drive)))
        .filter(|path| path.exists())
        .collect()
}

#[cfg(target_os = "windows")]
fn windows_claude_scan_roots(scan_roots: Option<&str>) -> Vec<PathBuf> {
    let configured = split_scan_roots(scan_roots);
    if !configured.is_empty() {
        return configured;
    }

    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for root in windows_drive_roots() {
        let key = normalize_path_for_compare(root.to_string_lossy().as_ref());
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        roots.push(root);
    }
    roots
}

#[cfg(target_os = "windows")]
fn push_claude_exe_candidate(
    candidates: &mut Vec<ClaudeDesktopLaunchCandidate>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    source: &str,
) {
    if !is_windows_claude_desktop_exec_path(&path) {
        return;
    }
    let target = path.to_string_lossy().to_string();
    let key = format!("exe:{}", normalize_path_for_compare(&target));
    if !seen.insert(key) {
        return;
    }
    candidates.push(ClaudeDesktopLaunchCandidate {
        target_type: "exe".to_string(),
        label: "Claude.exe".to_string(),
        target,
        source: source.to_string(),
        supports_multi_instance: true,
    });
}

#[cfg(target_os = "windows")]
fn windows_apps_dir_for_scan_root(root: &Path) -> Option<PathBuf> {
    let mut current = if root.is_file() {
        root.parent()?.to_path_buf()
    } else {
        root.to_path_buf()
    };

    loop {
        let is_windows_apps_dir = current
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("WindowsApps"))
            .unwrap_or(false);
        if is_windows_apps_dir {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }

    let program_files_windows_apps = root.join("Program Files").join("WindowsApps");
    if program_files_windows_apps.exists() {
        return Some(program_files_windows_apps);
    }

    let direct_windows_apps = root.join("WindowsApps");
    if direct_windows_apps.exists() {
        return Some(direct_windows_apps);
    }

    None
}

#[cfg(target_os = "windows")]
fn scan_windows_apps_claude_exe_under_root(
    root: &Path,
    candidates: &mut Vec<ClaudeDesktopLaunchCandidate>,
    seen: &mut HashSet<String>,
) -> bool {
    let Some(windows_apps_dir) = windows_apps_dir_for_scan_root(root) else {
        return false;
    };
    let entries = match fs::read_dir(&windows_apps_dir) {
        Ok(entries) => entries,
        Err(_) => return false,
    };

    let mut package_dirs = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase().starts_with("claude"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    package_dirs.sort_by(|left, right| {
        right
            .file_name()
            .and_then(|value| value.to_str())
            .cmp(&left.file_name().and_then(|value| value.to_str()))
    });

    let mut found = false;
    for package_dir in package_dirs {
        let exe = package_dir.join("app").join("claude.exe");
        let before = candidates.len();
        push_claude_exe_candidate(candidates, seen, exe, "windows_apps");
        found = found || candidates.len() > before;
    }
    found
}

#[cfg(target_os = "windows")]
fn scan_claude_exe_under_root(
    root: &Path,
    candidates: &mut Vec<ClaudeDesktopLaunchCandidate>,
    seen: &mut HashSet<String>,
) {
    if !root.exists() {
        return;
    }
    if root.is_file() {
        push_claude_exe_candidate(candidates, seen, root.to_path_buf(), "manual");
        return;
    }

    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 4 {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let is_claude_exe = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("Claude.exe"))
                    .unwrap_or(false);
                if is_claude_exe {
                    push_claude_exe_candidate(candidates, seen, path, "scan");
                }
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if depth == 0 || name.contains("claude") || name.contains("anthropic") {
                stack.push((path, depth + 1));
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn scan_windows_start_apps_for_claude(
    candidates: &mut Vec<ClaudeDesktopLaunchCandidate>,
    seen: &mut HashSet<String>,
) {
    use std::os::windows::process::CommandExt;

    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Get-StartApps | Where-Object { $_.Name -like '*Claude*' -or $_.AppID -like 'Claude_*' } | ForEach-Object { \"$($_.Name)`t$($_.AppID)\" }",
        ])
        .creation_flags(0x08000000)
        .stdin(Stdio::null());
    let output =
        modules::process_timeout::output_with_timeout(&mut command, WINDOWS_CLAUDE_PROBE_TIMEOUT);
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let mut parts = line.splitn(2, '\t');
        let name = parts.next().unwrap_or("").trim();
        let app_id = parts.next().unwrap_or("").trim();
        if name.is_empty() || app_id.is_empty() || !app_id.contains('!') {
            continue;
        }
        let shell_target = format!(r"shell:AppsFolder\{}", app_id);
        let key = format!("windows_app:{}", shell_target.to_ascii_lowercase());
        if !seen.insert(key) {
            continue;
        }
        candidates.push(ClaudeDesktopLaunchCandidate {
            target_type: "windows_app".to_string(),
            label: name.to_string(),
            target: shell_target,
            source: "windows_start_apps".to_string(),
            supports_multi_instance: false,
        });
    }
}

#[cfg(target_os = "windows")]
fn scan_windows_appx_packages_for_claude(
    scan_roots: Option<&str>,
    candidates: &mut Vec<ClaudeDesktopLaunchCandidate>,
    seen: &mut HashSet<String>,
) -> bool {
    use std::os::windows::process::CommandExt;

    let configured_roots = split_scan_roots(scan_roots);
    let normalized_roots = configured_roots
        .iter()
        .map(|root| normalize_path_for_compare(root.to_string_lossy().as_ref()))
        .filter(|root| !root.is_empty())
        .collect::<Vec<_>>();

    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "Get-AppxPackage | Where-Object { $_.Name -like '*Claude*' -or $_.PackageFamilyName -like '*Claude*' -or $_.PackageFullName -like '*Claude*' } | ForEach-Object { $_.InstallLocation }",
        ])
        .creation_flags(0x08000000)
        .stdin(Stdio::null());
    let output =
        modules::process_timeout::output_with_timeout(&mut command, WINDOWS_CLAUDE_PROBE_TIMEOUT);
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    let mut found = false;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let install_location = line.trim();
        if install_location.is_empty() {
            continue;
        }
        let install_dir = PathBuf::from(install_location);
        let normalized_install_dir = normalize_path_for_compare(install_location);
        if !normalized_roots.is_empty()
            && !normalized_roots.iter().any(|root| {
                normalized_install_dir.starts_with(root.as_str())
                    || root.starts_with(normalized_install_dir.as_str())
            })
        {
            continue;
        }
        let before = candidates.len();
        push_claude_exe_candidate(
            candidates,
            seen,
            install_dir.join("app").join("claude.exe"),
            "windows_appx",
        );
        found = found || candidates.len() > before;
    }
    found
}

#[cfg(target_os = "windows")]
pub fn scan_claude_desktop_launch_targets(
    scan_roots: Option<&str>,
) -> Vec<ClaudeDesktopLaunchCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let has_configured_roots = !split_scan_roots(scan_roots).is_empty();
    let roots = windows_claude_scan_roots(scan_roots);
    let mut found_windows_apps_exe = false;
    for root in &roots {
        if scan_windows_apps_claude_exe_under_root(root, &mut candidates, &mut seen) {
            found_windows_apps_exe = true;
            if !has_configured_roots {
                break;
            }
        }
    }

    if !found_windows_apps_exe
        && scan_windows_appx_packages_for_claude(scan_roots, &mut candidates, &mut seen)
    {
        found_windows_apps_exe = true;
    }

    if has_configured_roots && !found_windows_apps_exe {
        for root in &roots {
            scan_claude_exe_under_root(root, &mut candidates, &mut seen);
        }
    }

    scan_windows_start_apps_for_claude(&mut candidates, &mut seen);

    candidates
}

#[cfg(not(target_os = "windows"))]
pub fn scan_claude_desktop_launch_targets(
    _scan_roots: Option<&str>,
) -> Vec<ClaudeDesktopLaunchCandidate> {
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    {
        if let Some(path) = detect_claude_exec_path() {
            let target = normalize_claude_path_for_config(&path);
            candidates.push(ClaudeDesktopLaunchCandidate {
                target_type: "app".to_string(),
                label: "Claude.app".to_string(),
                target,
                source: "default".to_string(),
                supports_multi_instance: true,
            });
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(path) = detect_claude_exec_path() {
            candidates.push(ClaudeDesktopLaunchCandidate {
                target_type: "exe".to_string(),
                label: "Claude".to_string(),
                target: path.to_string_lossy().to_string(),
                source: "default".to_string(),
                supports_multi_instance: true,
            });
        }
    }

    candidates
}

#[cfg(not(target_os = "windows"))]
fn detect_claude_exec_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let candidates = [
            "/Applications/Claude.app",
            "/Applications/Claude.app/Contents/MacOS/Claude",
        ];
        for candidate in candidates {
            if let Some(path) = resolve_macos_exec_path(candidate) {
                if path.exists() {
                    return Some(path);
                }
            }
        }
        if let Ok(output) = Command::new("ps").args(["-axo", "pid,command"]).output() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
                let _pid = parts.next();
                let cmdline = parts.next().unwrap_or("").trim();
                let lower = cmdline.to_lowercase();
                if !lower.contains("claude.app/contents/macos/claude") {
                    continue;
                }
                if let Some(exe) = extract_macos_exe_from_command_line(cmdline) {
                    return Some(PathBuf::from(exe));
                }
            }
        }
        None
    }

    #[cfg(target_os = "linux")]
    {
        let candidates = [
            "/usr/bin/claude",
            "/opt/Claude/claude",
            "/opt/claude/claude",
        ];
        for candidate in candidates {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }
}

fn normalize_claude_path_for_config(path: &Path) -> String {
    #[cfg(target_os = "macos")]
    {
        normalize_macos_app_root(path).unwrap_or_else(|| {
            modules::process::normalize_windows_user_facing_path(&path.to_string_lossy())
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        modules::process::normalize_windows_user_facing_path(&path.to_string_lossy())
    }
}

pub fn detect_and_save_claude_launch_path(force: bool) -> Option<String> {
    let current = modules::config::get_user_config();
    #[cfg(target_os = "windows")]
    {
        if force {
            return None;
        }
        let custom = normalize_custom_path(&current.claude_app_path)?;
        return resolve_windows_claude_launch_target(&custom)
            .ok()
            .map(|_| custom);
    }

    #[cfg(not(target_os = "windows"))]
    {
        if !force {
            if let Some(configured) = normalize_custom_path(&current.claude_app_path) {
                return Some(configured);
            }
        }

        let detected = detect_claude_exec_path()?;
        let normalized = normalize_claude_path_for_config(&detected);
        if current.claude_app_path != normalized {
            let path = normalized.clone();
            if let Err(err) = modules::config::patch_user_config(move |config| {
                if force || normalize_custom_path(&config.claude_app_path).is_none() {
                    config.claude_app_path = path;
                }
                Ok(())
            }) {
                modules::logger::log_warn(&format!("保存 Claude 启动路径失败（已忽略）: {}", err));
            }
        }
        Some(normalized)
    }
}

#[cfg(target_os = "windows")]
fn resolve_claude_windows_launch_target() -> Result<ClaudeWindowsLaunchTarget, String> {
    let config = modules::config::get_user_config();
    let custom = normalize_custom_path(&config.claude_app_path)
        .ok_or_else(|| "APP_PATH_NOT_FOUND:claude".to_string())?;
    resolve_windows_claude_launch_target(&custom)
}

#[cfg(target_os = "windows")]
fn resolve_claude_windows_multi_instance_launch_target() -> Result<PathBuf, String> {
    match resolve_claude_windows_launch_target()? {
        ClaudeWindowsLaunchTarget::Executable(path) => Ok(path),
        ClaudeWindowsLaunchTarget::ShellApp(_) => {
            Err(CLAUDE_MULTI_INSTANCE_REQUIRES_EXE_ERROR.to_string())
        }
    }
}

#[cfg(target_os = "windows")]
pub fn ensure_claude_launch_path_configured() -> Result<(), String> {
    resolve_claude_windows_launch_target().map(|_| ())
}

#[cfg(target_os = "windows")]
pub fn ensure_claude_multi_instance_launch_path_configured() -> Result<(), String> {
    resolve_claude_windows_multi_instance_launch_target().map(|_| ())
}

#[cfg(not(target_os = "windows"))]
fn resolve_claude_launch_path() -> Result<PathBuf, String> {
    let config = modules::config::get_user_config();
    if let Some(custom) = normalize_custom_path(&config.claude_app_path) {
        if let Some(exec) = resolve_macos_exec_path(&custom) {
            return Ok(exec);
        }
        return Err("APP_PATH_NOT_FOUND:claude".to_string());
    }

    detect_and_save_claude_launch_path(false)
        .and_then(|value| resolve_macos_exec_path(&value))
        .ok_or_else(|| "APP_PATH_NOT_FOUND:claude".to_string())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_claude_launch_path_configured() -> Result<(), String> {
    resolve_claude_launch_path().map(|_| ())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_claude_multi_instance_launch_path_configured() -> Result<(), String> {
    ensure_claude_launch_path_configured()
}

#[cfg(not(target_os = "windows"))]
fn resolve_expected_claude_launch_path_for_match() -> Option<String> {
    let launch_path = match resolve_claude_launch_path() {
        Ok(path) => path,
        Err(err) => {
            modules::logger::log_warn(&format!(
                "[Claude Resolve] 启动路径未配置或无效，跳过 PID 匹配: {}",
                err
            ));
            return None;
        }
    };
    let normalized = normalize_path_for_compare(launch_path.to_string_lossy().as_ref());
    if normalized.is_empty() {
        modules::logger::log_warn("[Claude Resolve] 启动路径为空，跳过 PID 匹配");
        return None;
    }
    Some(normalized)
}

#[cfg(target_os = "windows")]
fn resolve_expected_claude_launch_path_for_match() -> Option<String> {
    match resolve_claude_windows_launch_target() {
        Ok(ClaudeWindowsLaunchTarget::Executable(path)) => {
            let normalized = normalize_path_for_compare(path.to_string_lossy().as_ref());
            if normalized.is_empty() {
                modules::logger::log_warn("[Claude Resolve] configured launch path is empty");
                None
            } else {
                Some(normalized)
            }
        }
        Ok(ClaudeWindowsLaunchTarget::ShellApp(target)) => {
            modules::logger::log_info(&format!(
                "[Claude Resolve] using configured Windows shell app target for process match: {}",
                modules::process::summarize_text_for_process_log(&target, 160)
            ));
            Some(WINDOWS_SHELL_APP_PROCESS_MATCH.to_string())
        }
        Err(err) => {
            modules::logger::log_warn(&format!(
                "[Claude Resolve] launch path is not configured or invalid; skip PID match: {}",
                err
            ));
            None
        }
    }
}

pub fn collect_claude_process_entries() -> Vec<(u32, Option<String>)> {
    let expected_launch = resolve_expected_claude_launch_path_for_match();
    let Some(expected_launch) = expected_launch else {
        return Vec::new();
    };

    let mut entries: HashMap<u32, Option<String>> = HashMap::new();

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
                let Some(exe) = extract_macos_exe_from_command_line(cmdline) else {
                    continue;
                };
                if normalize_path_for_compare(&exe) != expected_launch {
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
            let exe_path = process
                .exe()
                .and_then(|path| path.to_str())
                .map(normalize_path_for_compare)
                .unwrap_or_default();
            let name = process.name().to_string_lossy().to_lowercase();
            let args_line = process
                .cmd()
                .iter()
                .map(|arg| arg.to_string_lossy().to_lowercase())
                .collect::<Vec<String>>()
                .join(" ");
            #[cfg(target_os = "windows")]
            {
                let matches_expected = if expected_launch == WINDOWS_SHELL_APP_PROCESS_MATCH {
                    name.eq_ignore_ascii_case("claude.exe")
                        && !is_windows_claude_code_cli_process(&exe_path, &args_line)
                } else {
                    exe_path == expected_launch
                };
                if !matches_expected {
                    continue;
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                if exe_path != expected_launch {
                    continue;
                }
            }
            if is_helper_process(&name, &args_line) {
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

    let mut result: Vec<(u32, Option<String>)> = entries.into_iter().collect();
    result.sort_by_key(|(pid, _)| *pid);
    result
}

fn pick_preferred_pid(mut pids: Vec<u32>) -> Option<u32> {
    if pids.is_empty() {
        return None;
    }
    pids.sort();
    pids.dedup();
    pids.first().copied()
}

pub fn resolve_claude_pid_from_entries(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let default_dir = get_default_claude_config_dir()
        .ok()
        .map(|dir| normalize_path_for_compare(&dir.to_string_lossy()));
    let target = normalize_non_empty_path(user_data_dir);
    let mut matches = Vec::new();

    for (pid, dir) in entries {
        match (target.as_ref(), dir.as_ref()) {
            (Some(target_dir), Some(actual_dir)) if actual_dir == target_dir => matches.push(*pid),
            (Some(_), None) => {}
            (None, None) => matches.push(*pid),
            (None, Some(actual_dir))
                if default_dir
                    .as_ref()
                    .map(|default_dir| default_dir == actual_dir)
                    .unwrap_or(false) =>
            {
                matches.push(*pid)
            }
            _ => {}
        }
    }

    if let Some(pid) = last_pid {
        if modules::process::is_pid_running(pid) && matches.contains(&pid) {
            return Some(pid);
        }
        if modules::process::is_pid_running(pid) {
            let target_label = target.as_deref().unwrap_or("<default>");
            modules::logger::log_warn(&format!(
                "[Claude Resolve] 忽略不匹配的 last_pid={}，target={}，matched_pids={}",
                pid,
                modules::process::summarize_text_for_process_log(target_label, 96),
                modules::process::summarize_pid_list_for_log(&matches)
            ));
        }
    }

    pick_preferred_pid(matches)
}

pub fn resolve_claude_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    let entries = collect_claude_process_entries();
    resolve_claude_pid_from_entries(last_pid, user_data_dir, &entries)
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
    Err(format!("定位 Claude 窗口失败: {}", stderr.trim()))
}

#[cfg(target_os = "windows")]
fn focus_window_by_pid(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let _spawn_guard = crate::modules::app_lifecycle::acquire_process_spawn_guard("PowerShell")
        .map_err(|error| error.to_string())?;
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
        Err(format!("定位 Claude 窗口失败: {}", stderr.trim()))
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
        Err(format!("定位 Claude 窗口失败: {}", stderr.trim()))
    }
}

pub fn focus_claude_instance(
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Result<u32, String> {
    let pid = resolve_claude_pid(last_pid, user_data_dir)
        .ok_or_else(|| "实例未运行，无法定位窗口".to_string())?;
    focus_window_by_pid(pid)?;
    Ok(pid)
}

#[cfg(target_os = "macos")]
fn sanitize_macos_gui_launch_env(cmd: &mut Command) {
    cmd.env_remove("__CFBundleIdentifier");
    cmd.env_remove("XPC_SERVICE_NAME");
}

#[cfg(not(target_os = "macos"))]
fn sanitize_macos_gui_launch_env(_cmd: &mut Command) {}

#[cfg(target_os = "windows")]
fn spawn_claude_windows(
    launch_target: &ClaudeWindowsLaunchTarget,
    user_data_dir: Option<&str>,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;

    let target = user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut cmd = match launch_target {
        ClaudeWindowsLaunchTarget::Executable(launch_path) => Command::new(launch_path),
        ClaudeWindowsLaunchTarget::ShellApp(shell_target) => {
            if target.is_some()
                || use_new_window
                || extra_args.iter().any(|arg| !arg.trim().is_empty())
            {
                return Err("APP_PATH_NOT_FOUND:claude".to_string());
            }
            let mut command = Command::new("explorer.exe");
            command.arg(shell_target);
            command
        }
    };
    crate::modules::process::apply_managed_proxy_env_to_command(&mut cmd);
    cmd.creation_flags(0x08000000);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(target) = target {
        cmd.env(CLAUDE_USER_DATA_DIR_ENV, target);
        cmd.arg("--user-data-dir").arg(target);
    }
    if use_new_window {
        cmd.arg("--new-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }
    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Claude 失败: {}", e))?;
    if matches!(launch_target, ClaudeWindowsLaunchTarget::ShellApp(_)) {
        let probe_started = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(6);
        while probe_started.elapsed() < timeout {
            if let Some(resolved_pid) = resolve_claude_pid(None, None) {
                return Ok(resolved_pid);
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        modules::logger::log_warn(&format!(
            "[Claude Start] Windows shell app target launched but no Claude.exe PID matched in 6s; explorer pid={}",
            child.id()
        ));
        return Err("Claude Desktop 已发起启动，但 6 秒内未匹配到真实 Claude.exe 进程".to_string());
    }
    Ok(child.id())
}

#[cfg(target_os = "macos")]
fn spawn_claude_macos_open(
    launch_path: &Path,
    user_data_dir: Option<&str>,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let app_root = normalize_macos_app_root(launch_path).ok_or("APP_PATH_NOT_FOUND:claude")?;
    let target = user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let mut cmd = Command::new("open");
    sanitize_macos_gui_launch_env(&mut cmd);
    crate::modules::process::append_managed_proxy_env_to_open_args(&mut cmd);
    if let Some(target) = target {
        cmd.arg("--env")
            .arg(format!("{}={}", CLAUDE_USER_DATA_DIR_ENV, target));
    }
    cmd.arg("-n").arg("-a").arg(&app_root);
    cmd.arg("--args");
    if let Some(target) = target {
        cmd.arg("--user-data-dir").arg(target);
    }
    if use_new_window {
        cmd.arg("--new-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }

    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Claude 失败: {}", e))?;
    modules::logger::log_info("Claude 启动命令已发送（open -n -a）");
    let probe_started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(6);
    while probe_started.elapsed() < timeout {
        if let Some(resolved_pid) = resolve_claude_pid(None, target) {
            return Ok(resolved_pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    modules::logger::log_warn(&format!(
        "[Claude Start] 启动后 6s 内未匹配到实例 PID，回退 open pid={}",
        child.id()
    ));
    Ok(child.id())
}

#[cfg(target_os = "linux")]
fn spawn_claude_unix(
    launch_path: &Path,
    user_data_dir: Option<&str>,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let mut cmd = Command::new(launch_path);
    crate::modules::process::apply_managed_proxy_env_to_command(&mut cmd);
    sanitize_macos_gui_launch_env(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(target) = user_data_dir
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        cmd.env(CLAUDE_USER_DATA_DIR_ENV, target);
        cmd.arg("--user-data-dir").arg(target);
    }
    if use_new_window {
        cmd.arg("--new-window");
    }
    for arg in extra_args {
        if !arg.trim().is_empty() {
            cmd.arg(arg.trim());
        }
    }
    let child =
        spawn_command_with_trace(&mut cmd).map_err(|e| format!("启动 Claude 失败: {}", e))?;
    Ok(child.id())
}

pub fn start_claude_with_args_with_new_window(
    user_data_dir: &str,
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    let target = user_data_dir.trim();
    if target.is_empty() {
        return Err("实例目录为空，无法启动".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        let launch_path = resolve_claude_windows_multi_instance_launch_target()?;
        let launch_target = ClaudeWindowsLaunchTarget::Executable(launch_path);
        return spawn_claude_windows(&launch_target, Some(target), extra_args, use_new_window);
    }
    #[cfg(target_os = "macos")]
    {
        let launch_path = resolve_claude_launch_path()?;
        return spawn_claude_macos_open(&launch_path, Some(target), extra_args, use_new_window);
    }
    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_claude_launch_path()?;
        return spawn_claude_unix(&launch_path, Some(target), extra_args, use_new_window);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (target, extra_args, use_new_window);
        Err("Claude 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn start_claude_default_with_args_with_new_window(
    extra_args: &[String],
    use_new_window: bool,
) -> Result<u32, String> {
    #[cfg(target_os = "windows")]
    {
        let launch_target = resolve_claude_windows_launch_target()?;
        return spawn_claude_windows(&launch_target, None, extra_args, use_new_window);
    }
    #[cfg(target_os = "macos")]
    {
        let launch_path = resolve_claude_launch_path()?;
        return spawn_claude_macos_open(&launch_path, None, extra_args, use_new_window);
    }
    #[cfg(target_os = "linux")]
    {
        let launch_path = resolve_claude_launch_path()?;
        return spawn_claude_unix(&launch_path, None, extra_args, use_new_window);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (extra_args, use_new_window);
        Err("Claude 应用多开仅支持 macOS、Windows 和 Linux".to_string())
    }
}

pub fn close_claude(user_data_dirs: &[String], timeout_secs: u64) -> Result<(), String> {
    let target_dirs: HashSet<String> = user_data_dirs
        .iter()
        .map(|value| normalize_path_for_compare(value))
        .filter(|value| !value.is_empty())
        .collect();
    if target_dirs.is_empty() {
        return Ok(());
    }

    let default_dir = get_default_claude_config_dir()
        .ok()
        .map(|value| normalize_path_for_compare(&value.to_string_lossy()))
        .filter(|value| !value.is_empty());
    let allow_none_for_default = default_dir
        .as_ref()
        .map(|value| target_dirs.contains(value))
        .unwrap_or(false);

    let entries = collect_claude_process_entries();
    let mut pids = Vec::new();
    for (pid, dir) in entries {
        match dir.as_ref() {
            Some(value) if target_dirs.contains(value) => pids.push(pid),
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
            "无法关闭 Claude 实例进程，请手动关闭后重试: {}",
            modules::process::summarize_pid_list_for_log(&still_running)
        ));
    }

    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn windows_claude_code_cli_paths_are_not_desktop_paths() {
        assert!(is_windows_claude_code_cli_path(Path::new(
            r"C:\Users\me\.local\bin\claude.exe"
        )));
        assert!(is_windows_claude_code_cli_path(Path::new(
            r"C:\Users\me\AppData\Roaming\Claude\claude-code\2.1.177\claude.exe"
        )));
        assert!(!is_windows_claude_code_cli_path(Path::new(
            r"C:\Program Files\WindowsApps\Claude_1.0.0.0_x64__pzs8sxrjxfjjc\app\Claude.exe"
        )));
    }
}
