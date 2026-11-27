use chrono::Utc;
use std::collections::{HashMap, HashSet};
#[cfg(not(target_os = "macos"))]
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
#[cfg(not(target_os = "macos"))]
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use uuid::Uuid;

use crate::models::{DefaultInstanceSettings, InstanceProfile, InstanceStore};
use crate::modules::{self, instance_store};

pub use crate::modules::instance_store::{CreateInstanceParams, UpdateInstanceParams};

const INSTANCES_FILE: &str = "t3code_instances.json";
const USER_DATA_PRELOAD_FILE: &str = ".cockpit-t3code-user-data-preload.cjs";
const USER_DATA_PRELOAD_SCRIPT: &str = r#""use strict";
(() => {
  const prevNodeOptions = process.env.COCKPIT_T3CODE_PREV_NODE_OPTIONS;
  if (typeof prevNodeOptions === "string" && prevNodeOptions.length > 0) {
    process.env.NODE_OPTIONS = prevNodeOptions;
  } else {
    delete process.env.NODE_OPTIONS;
  }
  delete process.env.COCKPIT_T3CODE_PREV_NODE_OPTIONS;

  const userDataDir =
    typeof process.env.COCKPIT_T3CODE_USER_DATA_DIR === "string"
      ? process.env.COCKPIT_T3CODE_USER_DATA_DIR.trim()
      : "";
  delete process.env.COCKPIT_T3CODE_USER_DATA_DIR;
  if (!userDataDir || !process.versions || !process.versions.electron) {
    return;
  }

  try {
    const electron = require("electron");
    const app = electron && electron.app;
    if (!app || typeof app.setPath !== "function") {
      return;
    }
    const originalSetPath = app.setPath.bind(app);
    originalSetPath("userData", userDataDir);
    app.setPath = function cockpitPatchedSetPath(name, targetPath) {
      if (name === "userData") {
        return originalSetPath("userData", userDataDir);
      }
      return originalSetPath(name, targetPath);
    };
  } catch {
    // Ignore when not running inside the Electron main process.
  }
})();
"#;
static STORE_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

fn instances_path() -> Result<PathBuf, String> {
    Ok(modules::account::get_data_dir()?.join(INSTANCES_FILE))
}

pub fn load_instance_store() -> Result<InstanceStore, String> {
    instance_store::load_instance_store(&instances_path()?, INSTANCES_FILE)
}

pub fn save_instance_store(store: &InstanceStore) -> Result<(), String> {
    instance_store::save_instance_store(&instances_path()?, INSTANCES_FILE, store)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    Ok(load_instance_store()?.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    extra_args: Option<String>,
    _follow_local_account: Option<bool>,
) -> Result<DefaultInstanceSettings, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    if let Some(bind) = bind_account_id {
        store.default_settings.bind_account_id = bind;
        store.default_settings.follow_local_account = false;
    }
    if let Some(args) = extra_args {
        store.default_settings.extra_args = args.trim().to_string();
    }
    let value = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(value)
}

fn default_user_data_dir_for(
    platform: &str,
    home: Option<&Path>,
    app_data: Option<&Path>,
) -> Result<PathBuf, String> {
    match platform {
        "macos" => Ok(home
            .ok_or_else(|| "无法获取用户主目录".to_string())?
            .join("Library/Application Support/t3code-v2")),
        "windows" => Ok(app_data
            .ok_or_else(|| "无法获取 APPDATA".to_string())?
            .join("t3code-v2")),
        "linux" => Ok(home
            .ok_or_else(|| "无法获取用户主目录".to_string())?
            .join(".config/t3code-v2")),
        _ => Err("T3 Code 多开仅支持 macOS、Windows 和 Linux".to_string()),
    }
}

pub fn get_default_user_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir();
        return default_user_data_dir_for("macos", home.as_deref(), None);
    }
    #[cfg(target_os = "windows")]
    {
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
        return default_user_data_dir_for("windows", None, app_data.as_deref());
    }
    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir();
        return default_user_data_dir_for("linux", home.as_deref(), None);
    }
    #[allow(unreachable_code)]
    Err("T3 Code 多开仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_instances_root_dir() -> Result<PathBuf, String> {
    crate::modules::data_paths::resolve_data_dir().map(|root| root.join("instances").join("t3code"))
}

pub fn get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    Ok(modules::instance::InstanceDefaults {
        root_dir: get_instances_root_dir()?.to_string_lossy().to_string(),
        default_user_data_dir: modules::t3code_account::default_t3_base_dir()?
            .to_string_lossy()
            .to_string(),
    })
}

fn is_non_empty_dir(path: &Path) -> bool {
    path.exists()
        && path.is_dir()
        && fs::read_dir(path)
            .ok()
            .and_then(|mut entries| entries.next())
            .is_some()
}

fn initialize_empty_root(root: &Path) -> Result<(), String> {
    let electron_dir = root.join("electron");
    let userdata_dir = root.join("t3/userdata");
    fs::create_dir_all(&electron_dir)
        .map_err(|error| format!("创建 T3 Code Electron 实例目录失败: {}", error))?;
    fs::create_dir_all(userdata_dir.join("clerk"))
        .map_err(|error| format!("创建 T3 Code 数据实例目录失败: {}", error))?;
    let cli_first_launch = userdata_dir.join("cli-command-first-launch");
    if !cli_first_launch.exists() {
        let _ = fs::write(&cli_first_launch, "");
    }
    let v1_imported = electron_dir.join("v1-local-storage-imported");
    if !v1_imported.exists() {
        let _ = fs::write(&v1_imported, "");
    }
    ensure_user_data_preload_script(root)?;
    Ok(())
}

fn ensure_user_data_preload_script(root: &Path) -> Result<PathBuf, String> {
    let script_path = root.join(USER_DATA_PRELOAD_FILE);
    let needs_write = match fs::read_to_string(&script_path) {
        Ok(existing) => existing != USER_DATA_PRELOAD_SCRIPT,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&script_path, USER_DATA_PRELOAD_SCRIPT)
            .map_err(|error| format!("写入 T3 Code 多开启动脚本失败: {}", error))?;
    }
    Ok(script_path)
}

fn format_node_options_require(script_path: &Path) -> String {
    let escaped = script_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("--require=\"{}\"", escaped)
}

fn copy_default_to_root(root: &Path) -> Result<(), String> {
    initialize_empty_root(root)?;
    let default_state = modules::t3code_account::default_state_dir()?;
    let target_state = modules::t3code_account::state_dir_for_instance_root(root);
    for file_name in [
        "settings.json",
        "client-settings.json",
        "desktop-settings.json",
        "keybindings.json",
        "model-manifest.json",
    ] {
        let src = default_state.join(file_name);
        if src.is_file() {
            let _ = fs::copy(&src, target_state.join(file_name));
        }
    }
    let src_clerk = modules::t3code_account::clerk_tokens_path_for_state_dir(&default_state);
    if src_clerk.is_file() {
        let dst_clerk = modules::t3code_account::clerk_tokens_path_for_state_dir(&target_state);
        if let Some(parent) = dst_clerk.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::copy(&src_clerk, &dst_clerk);
    }
    modules::t3code_provider_bindings::remove_copied_qoder_bindings(&default_state, &target_state)
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let directory_preexisted = Path::new(params.user_data_dir.trim()).exists();
    let creation_mode = params
        .init_mode
        .clone()
        .unwrap_or_else(|| "copy".to_string());
    let _creation_guard = crate::modules::instance_storage_cleanup::protect_instance_creation()?;
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    let name = instance_store::normalize_name(&params.name)?;
    let root = PathBuf::from(params.user_data_dir.trim());
    if params.user_data_dir.trim().is_empty() {
        return Err("实例目录不能为空".to_string());
    }
    instance_store::ensure_unique(&store, &name, &params.user_data_dir, None)?;
    let mode = params
        .init_mode
        .as_deref()
        .unwrap_or("copy")
        .to_ascii_lowercase();
    if mode == "existingdir" || mode == "existing_dir" {
        if !root.is_dir() {
            return Err("所选 T3 Code 实例目录不存在或不是目录".to_string());
        }
    } else {
        if is_non_empty_dir(&root) {
            return Err(format!(
                "T3 Code 实例目标目录必须为空: {}",
                instance_store::display_path(&root)
            ));
        }
        if mode == "empty" {
            initialize_empty_root(&root)?;
        } else if let Some(source_id) = params
            .copy_source_instance_id
            .as_deref()
            .filter(|value| *value != "__default__")
        {
            let source = store
                .instances
                .iter()
                .find(|instance| instance.id == source_id)
                .ok_or_else(|| "复制来源实例不存在".to_string())?;
            let source_root = Path::new(&source.user_data_dir);
            let source_state = modules::t3code_account::state_dir_for_instance_root(source_root);
            let target_state = modules::t3code_account::state_dir_for_instance_root(&root);
            let private_roots = modules::t3code_provider_bindings::qoder_profile_roots(&source_state);
            instance_store::copy_dir_recursive_where(source_root, &root, &|path| {
                !private_roots.iter().any(|private| private == path)
            })?;
            modules::t3code_provider_bindings::remove_copied_qoder_bindings(&source_state, &target_state)?;
        } else {
            copy_default_to_root(&root)?;
        }
    }

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir: root.to_string_lossy().to_string(),
        working_dir: params.working_dir,
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: params.bind_account_id,
        model_routing: None,
        launch_mode: crate::models::InstanceLaunchMode::App,
        app_speed: crate::models::codex::CodexAppSpeed::Standard,
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };
    crate::modules::instance_directory_ownership::record_creation(
        Path::new(&instance.user_data_dir),
        &instance.id,
        &creation_mode,
        directory_preexisted,
    )?;
    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == params.instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    let current = store.instances[index].clone();
    if let Some(name) = params.name.as_deref() {
        let name = instance_store::normalize_name(name)?;
        instance_store::ensure_unique(&store, &name, &current.user_data_dir, Some(&current.id))?;
        store.instances[index].name = name;
    }
    if let Some(working_dir) = params.working_dir {
        store.instances[index].working_dir = if working_dir.trim().is_empty() {
            None
        } else {
            Some(working_dir.trim().to_string())
        };
    }
    if let Some(args) = params.extra_args {
        store.instances[index].extra_args = args.trim().to_string();
    }
    if let Some(bind) = params.bind_account_id {
        store.instances[index].bind_account_id = bind;
    }
    let value = store.instances[index].clone();
    save_instance_store(&store)?;
    Ok(value)
}

pub fn delete_instance(instance_id: &str) -> Result<(), String> {
    let _creation_guard = crate::modules::instance_storage_cleanup::protect_instance_creation()?;
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    modules::t3code_app_injection::stop_for_instance(instance_id);
    modules::instance::delete_instance_directory(Path::new(&store.instances[index].user_data_dir))?;
    store.instances.remove(index);
    save_instance_store(&store)
}

fn update_pid(instance_id: Option<&str>, pid: Option<u32>, launched: bool) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    if let Some(id) = instance_id {
        let instance = store
            .instances
            .iter_mut()
            .find(|instance| instance.id == id)
            .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
        instance.last_pid = pid;
        if launched {
            instance.last_launched_at = Some(Utc::now().timestamp_millis());
        }
    } else {
        store.default_settings.last_pid = pid;
    }
    save_instance_store(&store)
}

pub fn mark_started(instance_id: Option<&str>, pid: u32) -> Result<(), String> {
    update_pid(instance_id, Some(pid), true)
}

pub fn mark_stopped(instance_id: Option<&str>) -> Result<(), String> {
    let key = instance_id.unwrap_or("__default__");
    modules::t3code_app_injection::stop_for_instance(key);
    update_pid(instance_id, None, false)
}

pub fn clear_all_pids() -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 T3 Code 实例锁失败")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = None;
    for instance in &mut store.instances {
        instance.last_pid = None;
    }
    modules::t3code_app_injection::stop_all();
    save_instance_store(&store)
}

pub fn resolve_executable() -> Result<PathBuf, String> {
    let custom = modules::config::get_user_config().t3code_app_path;
    let trimmed = custom.trim();
    if !trimmed.is_empty() {
        let path = PathBuf::from(trimmed);
        #[cfg(target_os = "macos")]
        {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
            {
                for bin_name in [
                    "T3 Code (Beta)",
                    "T3 Code",
                    "T3 Code (Nightly)",
                    "T3 Code (Alpha)",
                    "Electron",
                ] {
                    let candidate = path.join("Contents/MacOS").join(bin_name);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
        if path.is_file() {
            return Ok(path);
        }
        return Err("APP_PATH_NOT_FOUND:t3code".to_string());
    }

    if let Some(detected) = detect_t3code_exec_path() {
        let _ = modules::config::patch_user_config(|current| {
            if current.t3code_app_path.trim().is_empty() {
                current.t3code_app_path = modules::process::normalize_windows_user_facing_path(
                    &detected.to_string_lossy(),
                );
            }
            Ok(())
        });
        return Ok(detected);
    }

    Err("APP_PATH_NOT_FOUND:t3code".to_string())
}

pub fn detect_t3code_exec_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut roots = vec![PathBuf::from("/Applications")];
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Applications"));
        }
        for root in roots {
            for (bundle, bin) in [
                ("T3 Code (Beta).app", "T3 Code (Beta)"),
                ("T3 Code.app", "T3 Code"),
                ("T3 Code (Nightly).app", "T3 Code (Nightly)"),
                ("T3 Code (Alpha).app", "T3 Code (Alpha)"),
            ] {
                let candidate = root.join(bundle).join("Contents/MacOS").join(bin);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut candidates = Vec::new();
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            let programs = PathBuf::from(&local_appdata).join("Programs");
            for (folder, exe) in [
                ("T3 Code (Beta)", "T3 Code (Beta).exe"),
                ("T3 Code", "T3 Code.exe"),
                ("t3code", "T3 Code.exe"),
                ("t3code", "T3 Code (Beta).exe"),
                ("t3code", "T3 Code (Nightly).exe"),
                ("T3 Code (Nightly)", "T3 Code (Nightly).exe"),
            ] {
                candidates.push(programs.join(folder).join(exe));
            }
        }
        for var in ["PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Ok(root) = std::env::var(var) {
                let base = PathBuf::from(root);
                candidates.push(base.join("T3 Code (Beta)/T3 Code (Beta).exe"));
                candidates.push(base.join("T3 Code/T3 Code.exe"));
            }
        }
        if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
            return Some(found);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/bin/t3code",
            "/usr/local/bin/t3code",
            "/opt/T3 Code/t3code",
            "/opt/t3code/t3code",
        ] {
            let path = PathBuf::from(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let path = home.join(".local/bin/t3code");
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

pub fn detect_and_save_t3code_launch_path(force: bool) -> Option<String> {
    let current = modules::config::get_user_config();
    if !force && !current.t3code_app_path.trim().is_empty() {
        return Some(current.t3code_app_path);
    }
    let detected = detect_t3code_exec_path()?;
    let normalized =
        modules::process::normalize_windows_user_facing_path(&detected.to_string_lossy());
    let _ = modules::config::patch_user_config(|cfg| {
        cfg.t3code_app_path = normalized.clone();
        Ok(())
    });
    Some(normalized)
}

fn normalize_path_for_compare(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = resolved.to_string_lossy().trim().to_string();
    #[cfg(target_os = "windows")]
    {
        return value.to_ascii_lowercase();
    }
    #[cfg(not(target_os = "windows"))]
    value
}

#[cfg(not(target_os = "macos"))]
fn extract_user_data_dir(arguments: &[OsString]) -> Option<String> {
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].to_string_lossy();
        if let Some(value) = argument.strip_prefix("--user-data-dir=") {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
        if argument == "--user-data-dir" {
            return arguments
                .get(index + 1)
                .map(|value| value.to_string_lossy().trim().to_string())
                .filter(|value| !value.is_empty());
        }
        index += 1;
    }
    None
}

fn extract_user_data_dir_from_command_line(command_line: &str) -> Option<String> {
    for marker in ["--user-data-dir=", "--user-data-dir "] {
        let Some(start) = command_line.find(marker) else {
            continue;
        };
        let rest = command_line[start + marker.len()..].trim_start();
        if rest.is_empty() {
            return None;
        }
        let (value, _) =
            if let Some(quote) = rest.chars().next().filter(|ch| *ch == '\'' || *ch == '"') {
                let quoted = &rest[quote.len_utf8()..];
                let end = quoted.find(quote).unwrap_or(quoted.len());
                (&quoted[..end], true)
            } else {
                let end = rest.find(" --").unwrap_or(rest.len());
                (&rest[..end], false)
            };
        let value = value.trim();
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

fn is_t3code_command(lower: &str) -> bool {
    lower.contains("t3 code") || lower.contains("t3code")
}

fn is_helper_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("helper")
        || lower.contains("--type=")
        || lower.contains("crashpad_handler")
        || lower.contains("bin.mjs")
        || lower.contains("t3-resource-monitor")
        || lower.contains("acp-mcp-bridge")
}

#[cfg(not(target_os = "macos"))]
fn is_main_process(process: &sysinfo::Process) -> bool {
    let name = process.name().to_string_lossy().to_ascii_lowercase();
    let executable_name = process
        .exe()
        .and_then(Path::file_name)
        .map(|value| value.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let command = process
        .cmd()
        .iter()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
    (is_t3code_command(&name) || is_t3code_command(&executable_name))
        && !is_helper_command(&command)
}

fn parse_ps_process_line(line: &str) -> Option<(u32, u32, &str)> {
    let line = line.trim_start();
    let pid_end = line.find(char::is_whitespace)?;
    let pid = line[..pid_end].parse().ok()?;
    let rest = line[pid_end..].trim_start();
    let parent_end = rest.find(char::is_whitespace)?;
    let parent_pid = rest[..parent_end].parse().ok()?;
    Some((pid, parent_pid, rest[parent_end..].trim_start()))
}

fn collect_process_entries_from_ps(output: &str) -> Vec<(u32, Option<String>)> {
    let mut main_pids = HashSet::new();
    let mut main_user_data = HashMap::new();
    let mut user_data_by_parent = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((pid, parent_pid, command)) = parse_ps_process_line(line) else {
            continue;
        };
        let lower = command.to_ascii_lowercase();
        if !is_t3code_command(&lower) {
            continue;
        }
        if is_helper_command(command) {
            if let Some(user_data_dir) = extract_user_data_dir_from_command_line(command) {
                user_data_by_parent
                    .entry(parent_pid)
                    .or_insert(user_data_dir);
            }
            continue;
        }
        if lower.contains(".app/contents/macos/") || lower.starts_with("t3code") {
            main_pids.insert(pid);
            if let Some(user_data_dir) = extract_user_data_dir_from_command_line(command) {
                main_user_data.insert(pid, user_data_dir);
            }
        }
    }

    main_pids.extend(user_data_by_parent.keys().copied());
    let mut entries: Vec<_> = main_pids
        .into_iter()
        .map(|pid| {
            let user_data_dir = main_user_data
                .remove(&pid)
                .or_else(|| user_data_by_parent.remove(&pid));
            (pid, user_data_dir)
        })
        .collect();
    entries.sort_by_key(|(pid, _)| *pid);
    entries
}

pub fn collect_process_entries() -> Vec<(u32, Option<String>)> {
    #[cfg(target_os = "macos")]
    {
        return Command::new("ps")
            .args(["-axo", "pid=,ppid=,command="])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| collect_process_entries_from_ps(&String::from_utf8_lossy(&output.stdout)))
            .unwrap_or_default();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cmd(UpdateKind::OnlyIfNotSet),
        );
        let mut main_entries = Vec::new();
        let mut user_data_by_parent = HashMap::new();
        for (pid, process) in system.processes() {
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            let executable = process
                .exe()
                .map(|value| value.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if !is_t3code_command(&name) && !is_t3code_command(&executable) {
                continue;
            }
            let command = process
                .cmd()
                .iter()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            let user_data_dir = extract_user_data_dir(process.cmd());
            if is_helper_command(&command) {
                if let (Some(parent), Some(user_data_dir)) = (process.parent(), user_data_dir) {
                    user_data_by_parent
                        .entry(parent.as_u32())
                        .or_insert(user_data_dir);
                }
                continue;
            }
            if is_main_process(process) {
                main_entries.push((pid.as_u32(), user_data_dir));
            }
        }

        for (pid, user_data_dir) in &mut main_entries {
            if user_data_dir.is_none() {
                *user_data_dir = user_data_by_parent.remove(pid);
            }
        }
        main_entries.sort_by_key(|(pid, _)| *pid);
        main_entries
    }
}

fn resolve_pid_from_entries(
    instance_root: Option<&Path>,
    last_pid: Option<u32>,
    default_user_data_dir: &Path,
    entries: &[(u32, Option<String>)],
) -> Option<u32> {
    let target = instance_root
        .map(|root| root.join("electron"))
        .unwrap_or_else(|| default_user_data_dir.to_path_buf());
    let target = normalize_path_for_compare(&target);
    let allow_missing_dir = instance_root.is_none();
    let matches: Vec<u32> = entries
        .iter()
        .filter_map(|(pid, actual)| match actual {
            Some(actual) if normalize_path_for_compare(Path::new(actual)) == target => Some(*pid),
            None if allow_missing_dir => Some(*pid),
            _ => None,
        })
        .collect();
    if let Some(pid) = last_pid {
        if matches.contains(&pid) {
            return Some(pid);
        }
    }
    matches.into_iter().min()
}

pub fn resolve_pid(instance_root: Option<&Path>, last_pid: Option<u32>) -> Option<u32> {
    let default_user_data_dir = get_default_user_data_dir().ok()?;
    resolve_pid_from_entries(
        instance_root,
        last_pid,
        &default_user_data_dir,
        &collect_process_entries(),
    )
}

fn base_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for key in [
        "__CFBundleIdentifier",
        "XPC_SERVICE_NAME",
        "NODE_OPTIONS",
        "NODE_PATH",
        "NODE_ENV",
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ASAR",
        "T3CODE_HOME",
        "T3CODE_PORT",
        "COCKPIT_T3CODE_USER_DATA_DIR",
        "COCKPIT_T3CODE_PREV_NODE_OPTIONS",
    ] {
        command.env_remove(key);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000 | 0x00000200 | 0x00000008);
    }
    command
}

fn wait_for_started_pid(instance_root: Option<&Path>, spawned_pid: u32) -> u32 {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(1800);
    let min_alive = std::time::Duration::from_millis(450);
    let mut last_resolved = None;
    while started.elapsed() < timeout {
        if let Some(pid) = resolve_pid(instance_root, Some(spawned_pid)) {
            last_resolved = Some(pid);
            if started.elapsed() >= min_alive {
                return pid;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(120));
    }
    last_resolved.unwrap_or(spawned_pid)
}

pub fn start_default(extra_args: &[String]) -> Result<u32, String> {
    let executable = resolve_executable()?;
    #[cfg(target_os = "macos")]
    let app_root = modules::process::resolve_macos_app_root_from_launch_path(&executable);
    #[cfg(target_os = "macos")]
    if let (Some(app_root), Some(pid)) = (&app_root, resolve_pid(None, None)) {
        modules::process::spawn_open_app_with_options(app_root, &[], false)?;
        return Ok(pid);
    }
    let injection_enabled = modules::t3code_app_injection::enabled_for_app();
    let launch = modules::t3code_app_injection::build_launch_args(extra_args, injection_enabled)?;
    let environment = modules::t3code_privacy::launch_environment()?;
    #[cfg(target_os = "macos")]
    let pid = match app_root {
        Some(app_root) => launch_default_app_bundle(&app_root, &launch.args, &environment)?,
        None => launch_default_executable(&executable, &launch.args, &environment)?,
    };
    #[cfg(not(target_os = "macos"))]
    let pid = launch_default_executable(&executable, &launch.args, &environment)?;
    if let Some(port) = launch.port {
        let state_dir = modules::t3code_account::default_state_dir()?;
        modules::t3code_app_injection::start_for_launched_instance("__default__", &state_dir, port);
    }
    Ok(pid)
}

#[cfg(target_os = "macos")]
fn launch_default_app_bundle(
    app_root: &str,
    args: &[String],
    environment: &HashMap<String, String>,
) -> Result<u32, String> {
    let env_pairs: Vec<_> = environment
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let launcher_pid =
        modules::process::spawn_open_app_with_options_and_env(app_root, args, true, &env_pairs)?;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(6) {
        if let Some(pid) = resolve_pid(None, None) {
            return Ok(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(format!(
        "T3 Code 启动超时，未找到默认实例主进程（open PID {launcher_pid}）"
    ))
}

fn launch_default_executable(
    executable: &Path,
    args: &[String],
    environment: &HashMap<String, String>,
) -> Result<u32, String> {
    let mut command = base_command(executable);
    command
        .args(args)
        .env_remove("COCKPIT_T3CODE_PRIVACY")
        .envs(environment);
    let child = command
        .spawn()
        .map_err(|error| format!("启动 T3 Code 失败: {}", error))?;
    let spawned_pid = modules::process::background_child_pid(child);
    Ok(wait_for_started_pid(None, spawned_pid))
}

pub fn start_managed(instance: &InstanceProfile, extra_args: &[String]) -> Result<u32, String> {
    let executable = resolve_executable()?;
    let root = Path::new(&instance.user_data_dir);
    initialize_empty_root(root)?;
    let preload_script = ensure_user_data_preload_script(root)?;
    let injection_enabled = modules::t3code_app_injection::enabled_for_app();
    let mut args = extra_args.to_vec();
    let electron_dir = root.join("electron");
    args.push(format!(
        "--user-data-dir={}",
        electron_dir.to_string_lossy()
    ));
    let launch = modules::t3code_app_injection::build_launch_args(&args, injection_enabled)?;

    let mut command = base_command(&executable);
    let t3_home = root.join("t3");
    command.env("T3CODE_HOME", &t3_home);
    command.env("COCKPIT_T3CODE_USER_DATA_DIR", &electron_dir);
    command.env("COCKPIT_T3CODE_PREV_NODE_OPTIONS", "");
    command.env("NODE_OPTIONS", format_node_options_require(&preload_script));
    #[cfg(target_os = "windows")]
    command.env("APPDATA", &electron_dir);
    #[cfg(target_os = "linux")]
    command.env("XDG_CONFIG_HOME", &electron_dir);
    command.args(&launch.args);
    if let Some(working_dir) = instance
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command.current_dir(working_dir);
    }
    let child = command
        .spawn()
        .map_err(|error| format!("启动 T3 Code 多开实例失败: {}", error))?;
    let spawned_pid = modules::process::background_child_pid(child);
    let pid = wait_for_started_pid(Some(root), spawned_pid);
    if let Some(port) = launch.port {
        let state_dir = modules::t3code_account::state_dir_for_instance_root(root);
        modules::t3code_app_injection::start_for_launched_instance(&instance.id, &state_dir, port);
    }
    Ok(pid)
}

pub fn close_all() -> Result<(), String> {
    let store = load_instance_store()?;
    let mut errors = Vec::new();
    if let Some(pid) = resolve_pid(None, store.default_settings.last_pid) {
        if let Err(error) = modules::process::close_pid(pid, 20) {
            errors.push(format!("默认实例: {}", error));
        }
    }
    for instance in &store.instances {
        if let Some(pid) = resolve_pid(Some(Path::new(&instance.user_data_dir)), instance.last_pid)
        {
            if let Err(error) = modules::process::close_pid(pid, 20) {
                errors.push(format!("{}: {}", instance.name, error));
            }
        }
    }
    if errors.is_empty() {
        clear_all_pids()
    } else {
        Err(format!("部分 T3 Code 实例关闭失败: {}", errors.join("; ")))
    }
}

pub fn is_profile_initialized(root: &Path, is_default: bool) -> bool {
    if is_default {
        return modules::t3code_account::default_state_dir().is_ok_and(|dir| dir.exists());
    }
    modules::t3code_account::state_dir_for_instance_root(root).exists()
        || is_non_empty_dir(&root.join("electron"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn collect_macos_process_entries_reads_main_and_helper_arguments() {
        let ps = "\
 54913     1 /Applications/T3 Code (Beta).app/Contents/MacOS/T3 Code (Beta) --remote-debugging-address=127.0.0.1 --remote-debugging-port=56857
 54915 54913 /Applications/T3 Code (Beta).app/Contents/Frameworks/T3 Code (Beta) Helper.app/Contents/MacOS/T3 Code (Beta) Helper --type=gpu-process --user-data-dir=/Users/demo/Library/Application Support/t3code-v2 --gpu-preferences=abc
 54920 54913 /Applications/T3 Code (Beta).app/Contents/MacOS/T3 Code (Beta) --require /Applications/T3 Code (Beta).app/Contents/Resources/app.asar/apps/desktop/dist-electron/compileCache.cjs /Applications/T3 Code (Beta).app/Contents/Resources/app.asar/apps/server/dist/bin.mjs --bootstrap-fd 3
 54930 54920 /Applications/T3 Code (Beta).app/Contents/Resources/resource-monitor/t3-resource-monitor
 61200 60794 /Applications/T3 Code (Beta).app/Contents/MacOS/T3 Code (Beta) --user-data-dir=/Users/demo/.antigravity_cockpit/instances/t3code/inst-1/electron --remote-debugging-address=127.0.0.1 --remote-debugging-port=56900
";
        let entries = collect_process_entries_from_ps(ps);
        assert_eq!(
            entries,
            vec![
                (
                    54913,
                    Some("/Users/demo/Library/Application Support/t3code-v2".to_string())
                ),
                (
                    61200,
                    Some(
                        "/Users/demo/.antigravity_cockpit/instances/t3code/inst-1/electron"
                            .to_string()
                    )
                ),
            ]
        );
    }

    #[test]
    fn resolve_pid_matches_default_and_managed_directories() {
        let sandbox = tempfile::tempdir().expect("create instance fixture");
        let default_dir = sandbox.path().join("T3 Code");
        let managed_root = sandbox.path().join("managed");
        let managed_dir = managed_root.join("electron");
        let entries = vec![
            (54913, Some(default_dir.to_string_lossy().into_owned())),
            (61200, Some(managed_dir.to_string_lossy().into_owned())),
        ];
        assert_eq!(
            resolve_pid_from_entries(None, None, &default_dir, &entries),
            Some(54913)
        );
        assert_eq!(
            resolve_pid_from_entries(Some(&managed_root), None, &default_dir, &entries),
            Some(61200)
        );
    }

    #[test]
    fn initialize_empty_root_creates_preload_and_first_launch_markers() {
        let temp = std::env::temp_dir().join(format!("t3code-instance-test-{}", Uuid::new_v4()));
        initialize_empty_root(&temp).expect("init empty root");
        assert!(temp.join(USER_DATA_PRELOAD_FILE).is_file());
        assert!(temp.join("t3/userdata/cli-command-first-launch").is_file());
        assert!(temp.join("electron/v1-local-storage-imported").is_file());
        let _ = fs::remove_dir_all(&temp);
    }
}
