use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile, InstanceStore};
use crate::modules;
use crate::modules::instance::InstanceDefaults;
use crate::modules::instance_store;

static CODEX_INSTANCE_STORE_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const CODEX_INSTANCES_FILE: &str = "codex_instances.json";
const CODEX_SHARED_SKILLS_DIR_NAME: &str = "skills";
const CODEX_SHARED_RULES_DIR_NAME: &str = "rules";
const CODEX_SHARED_AGENTS_FILE_NAME: &str = "AGENTS.md";
const CODEX_SHARED_VENDOR_IMPORTS_SKILLS_DIR: &str = "vendor_imports/skills";

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
    Ok(data_dir.join(CODEX_INSTANCES_FILE))
}

pub fn load_instance_store() -> Result<InstanceStore, String> {
    let path = instances_path()?;
    instance_store::load_instance_store(&path, CODEX_INSTANCES_FILE)
}

pub fn save_instance_store(store: &InstanceStore) -> Result<(), String> {
    let path = instances_path()?;
    instance_store::save_instance_store(&path, CODEX_INSTANCES_FILE, store)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    let store = load_instance_store()?;
    Ok(store.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    extra_args: Option<String>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
) -> Result<DefaultInstanceSettings, String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let settings = &mut store.default_settings;

    if follow_local_account == Some(true) {
        settings.follow_local_account = true;
        settings.bind_account_id = None;
    }

    if let Some(bind) = bind_account_id {
        settings.bind_account_id = bind;
        settings.follow_local_account = false;
    }

    if follow_local_account == Some(false) && settings.bind_account_id.is_none() {
        settings.follow_local_account = false;
    }

    if let Some(args) = extra_args {
        settings.extra_args = args.trim().to_string();
    }

    if let Some(mode) = launch_mode {
        settings.launch_mode = mode;
    }

    let updated = settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn get_default_codex_home() -> Result<PathBuf, String> {
    Ok(modules::codex_account::get_codex_home())
}

pub fn get_default_instances_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/codex"));
    }

    #[allow(unreachable_code)]
    Err("Codex 多开实例仅支持 macOS".to_string())
}

pub fn get_instance_defaults() -> Result<InstanceDefaults, String> {
    let root_dir = get_default_instances_root_dir()?;
    let default_user_data_dir = get_default_codex_home()?;
    Ok(InstanceDefaults {
        root_dir: root_dir.to_string_lossy().to_string(),
        default_user_data_dir: default_user_data_dir.to_string_lossy().to_string(),
    })
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, target: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, target).map_err(|e| format!("创建目录共享链接失败: {}", e))
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, target: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(source, target)
        .map_err(|e| format!("创建目录共享链接失败: {}", e))
}

#[cfg(not(any(unix, windows)))]
fn create_directory_symlink(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("当前系统不支持创建目录符号链接".to_string())
}

#[cfg(unix)]
fn create_file_symlink(source: &Path, target: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, target).map_err(|e| format!("创建文件共享链接失败: {}", e))
}

#[cfg(windows)]
fn create_file_symlink(source: &Path, target: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_file(source, target)
        .map_err(|e| format!("创建文件共享链接失败: {}", e))
}

#[cfg(not(any(unix, windows)))]
fn create_file_symlink(_source: &Path, _target: &Path) -> Result<(), String> {
    Err("当前系统不支持创建文件符号链接".to_string())
}

fn remove_symlink(path: &Path) -> Result<(), String> {
    fs::remove_file(path)
        .or_else(|_| fs::remove_dir(path))
        .map_err(|e| format!("移除已有共享链接失败: {}", e))
}

fn is_directory_empty(path: &Path) -> Result<bool, String> {
    let mut iter = fs::read_dir(path).map_err(|e| format!("读取目录失败: {}", e))?;
    Ok(iter.next().is_none())
}

fn files_have_same_content(a: &Path, b: &Path) -> Result<bool, String> {
    let meta_a = fs::metadata(a).map_err(|e| format!("读取文件元数据失败: {}", e))?;
    let meta_b = fs::metadata(b).map_err(|e| format!("读取文件元数据失败: {}", e))?;
    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }
    let bytes_a = fs::read(a).map_err(|e| format!("读取文件失败: {}", e))?;
    let bytes_b = fs::read(b).map_err(|e| format!("读取文件失败: {}", e))?;
    Ok(bytes_a == bytes_b)
}

fn sorted_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(path)
        .map_err(|e| format!("读取目录失败: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取目录项失败: {}", e))?;
    entries.sort_by(|a, b| {
        a.file_name()
            .to_string_lossy()
            .cmp(&b.file_name().to_string_lossy())
    });
    Ok(entries)
}

fn directories_are_equivalent(a: &Path, b: &Path) -> Result<bool, String> {
    let entries_a = sorted_entries(a)?;
    let entries_b = sorted_entries(b)?;
    if entries_a.len() != entries_b.len() {
        return Ok(false);
    }

    for (entry_a, entry_b) in entries_a.into_iter().zip(entries_b.into_iter()) {
        if entry_a.file_name() != entry_b.file_name() {
            return Ok(false);
        }

        let path_a = entry_a.path();
        let path_b = entry_b.path();
        let meta_a =
            fs::symlink_metadata(&path_a).map_err(|e| format!("读取路径元数据失败: {}", e))?;
        let meta_b =
            fs::symlink_metadata(&path_b).map_err(|e| format!("读取路径元数据失败: {}", e))?;
        let type_a = meta_a.file_type();
        let type_b = meta_b.file_type();

        if type_a.is_symlink() || type_b.is_symlink() {
            return Ok(false);
        }

        if type_a.is_dir() && type_b.is_dir() {
            if !directories_are_equivalent(&path_a, &path_b)? {
                return Ok(false);
            }
            continue;
        }

        if type_a.is_file() && type_b.is_file() {
            if !files_have_same_content(&path_a, &path_b)? {
                return Ok(false);
            }
            continue;
        }

        return Ok(false);
    }

    Ok(true)
}

fn paths_point_to_same_location(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => a == b,
    }
}

fn display_abs_path(path: &Path) -> String {
    instance_store::display_path(path)
}

fn resolve_link_target(link_path: &Path, target: PathBuf) -> PathBuf {
    if target.is_absolute() {
        target
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(target)
    }
}

fn sync_shared_directory(
    profile_dir: &Path,
    default_codex_home: &Path,
    relative_path: &Path,
) -> Result<(), String> {
    let global_dir = default_codex_home.join(relative_path);
    let instance_dir = profile_dir.join(relative_path);
    let relative_display = relative_path.to_string_lossy();

    fs::create_dir_all(&global_dir).map_err(|e| {
        format!(
            "创建全局共享目录失败 ({}): {}",
            display_abs_path(&global_dir),
            e
        )
    })?;
    if let Some(parent) = instance_dir.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建实例共享目录父路径失败 ({}): {}",
                display_abs_path(parent),
                e
            )
        })?;
    }

    if !instance_dir.exists() {
        return create_directory_symlink(&global_dir, &instance_dir);
    }

    let metadata = fs::symlink_metadata(&instance_dir).map_err(|e| {
        format!(
            "读取实例共享目录信息失败 ({}): {}",
            display_abs_path(&instance_dir),
            e
        )
    })?;
    if metadata.file_type().is_symlink() {
        let current_target = fs::read_link(&instance_dir).map_err(|e| {
            format!(
                "读取实例共享目录链接失败 ({}): {}",
                display_abs_path(&instance_dir),
                e
            )
        })?;
        let resolved_target = resolve_link_target(&instance_dir, current_target);
        if paths_point_to_same_location(&resolved_target, &global_dir) {
            return Ok(());
        }
        remove_symlink(&instance_dir)?;
        return create_directory_symlink(&global_dir, &instance_dir);
    }

    if !metadata.is_dir() {
        return Err(format!(
            "实例共享目录路径不是目录 ({}): {}",
            relative_display,
            display_abs_path(&instance_dir)
        ));
    }

    let instance_empty = is_directory_empty(&instance_dir)?;
    let global_empty = is_directory_empty(&global_dir)?;
    if instance_empty {
        fs::remove_dir(&instance_dir).map_err(|e| {
            format!(
                "清理空实例共享目录失败 ({}): {}",
                display_abs_path(&instance_dir),
                e
            )
        })?;
        return create_directory_symlink(&global_dir, &instance_dir);
    }

    if global_empty {
        fs::remove_dir(&global_dir).map_err(|e| {
            format!(
                "移除空全局共享目录失败 ({}): {}",
                display_abs_path(&global_dir),
                e
            )
        })?;
        instance_store::copy_dir_recursive(&instance_dir, &global_dir).map_err(|e| {
            format!(
                "迁移实例共享目录到全局失败 ({}): {}",
                display_abs_path(&instance_dir),
                e
            )
        })?;
        fs::remove_dir_all(&instance_dir).map_err(|e| {
            format!(
                "清理实例共享目录失败 ({}): {}",
                display_abs_path(&instance_dir),
                e
            )
        })?;
        return create_directory_symlink(&global_dir, &instance_dir);
    }

    if directories_are_equivalent(&instance_dir, &global_dir)? {
        fs::remove_dir_all(&instance_dir).map_err(|e| {
            format!(
                "清理实例共享目录失败 ({}): {}",
                display_abs_path(&instance_dir),
                e
            )
        })?;
        return create_directory_symlink(&global_dir, &instance_dir);
    }

    fs::remove_dir_all(&instance_dir).map_err(|e| {
        format!(
            "强制重建实例共享目录链接前清理实例目录失败 ({}): {}",
            display_abs_path(&instance_dir),
            e
        )
    })?;
    create_directory_symlink(&global_dir, &instance_dir).map_err(|e| {
        format!(
            "强制重建实例共享目录链接失败 ({} -> {}, {}): {}",
            display_abs_path(&global_dir),
            display_abs_path(&instance_dir),
            relative_display,
            e
        )
    })
}

fn sync_shared_file(
    profile_dir: &Path,
    default_codex_home: &Path,
    relative_path: &Path,
) -> Result<(), String> {
    let global_file = default_codex_home.join(relative_path);
    let instance_file = profile_dir.join(relative_path);
    let relative_display = relative_path.to_string_lossy();

    if let Some(parent) = global_file.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建全局共享文件父目录失败 ({}): {}",
                display_abs_path(parent),
                e
            )
        })?;
    }
    if let Some(parent) = instance_file.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建实例共享文件父目录失败 ({}): {}",
                display_abs_path(parent),
                e
            )
        })?;
    }

    if !global_file.exists() {
        if instance_file.exists() {
            let meta = fs::symlink_metadata(&instance_file).map_err(|e| {
                format!(
                    "读取实例共享文件信息失败 ({}): {}",
                    display_abs_path(&instance_file),
                    e
                )
            })?;
            if meta.file_type().is_symlink() {
                remove_symlink(&instance_file)?;
            } else if meta.is_file() {
                fs::copy(&instance_file, &global_file).map_err(|e| {
                    format!(
                        "迁移实例共享文件到全局失败 ({} -> {}): {}",
                        display_abs_path(&instance_file),
                        display_abs_path(&global_file),
                        e
                    )
                })?;
                fs::remove_file(&instance_file).map_err(|e| {
                    format!(
                        "清理实例共享文件失败 ({}): {}",
                        display_abs_path(&instance_file),
                        e
                    )
                })?;
            } else {
                return Err(format!(
                    "实例共享文件路径不是文件 ({}): {}",
                    relative_display,
                    display_abs_path(&instance_file)
                ));
            }
        } else {
            return Ok(());
        }
    }

    let global_meta = fs::metadata(&global_file).map_err(|e| {
        format!(
            "读取全局共享文件信息失败 ({}): {}",
            display_abs_path(&global_file),
            e
        )
    })?;
    if !global_meta.is_file() {
        return Err(format!(
            "全局共享路径不是文件 ({}): {}",
            relative_display,
            display_abs_path(&global_file)
        ));
    }

    if !instance_file.exists() {
        return create_file_symlink(&global_file, &instance_file);
    }

    let instance_meta = fs::symlink_metadata(&instance_file).map_err(|e| {
        format!(
            "读取实例共享文件信息失败 ({}): {}",
            display_abs_path(&instance_file),
            e
        )
    })?;
    if instance_meta.file_type().is_symlink() {
        let current_target = fs::read_link(&instance_file).map_err(|e| {
            format!(
                "读取实例共享文件链接失败 ({}): {}",
                display_abs_path(&instance_file),
                e
            )
        })?;
        let resolved_target = resolve_link_target(&instance_file, current_target);
        if paths_point_to_same_location(&resolved_target, &global_file) {
            return Ok(());
        }
        remove_symlink(&instance_file)?;
        return create_file_symlink(&global_file, &instance_file);
    }

    if !instance_meta.is_file() {
        return Err(format!(
            "实例共享文件路径不是文件 ({}): {}",
            relative_display,
            display_abs_path(&instance_file)
        ));
    }

    if files_have_same_content(&instance_file, &global_file)? {
        fs::remove_file(&instance_file).map_err(|e| {
            format!(
                "清理实例共享文件失败 ({}): {}",
                display_abs_path(&instance_file),
                e
            )
        })?;
        return create_file_symlink(&global_file, &instance_file);
    }

    fs::remove_file(&instance_file).map_err(|e| {
        format!(
            "强制重建实例共享文件链接前清理实例文件失败 ({}): {}",
            display_abs_path(&instance_file),
            e
        )
    })?;
    create_file_symlink(&global_file, &instance_file).map_err(|e| {
        format!(
            "强制重建实例共享文件链接失败 ({} -> {}, {}): {}",
            display_abs_path(&global_file),
            display_abs_path(&instance_file),
            relative_display,
            e
        )
    })
}

pub fn ensure_instance_shared_skills(profile_dir: &Path) -> Result<(), String> {
    let default_codex_home = get_default_codex_home()?;
    if paths_point_to_same_location(profile_dir, &default_codex_home) {
        return Ok(());
    }
    fs::create_dir_all(profile_dir).map_err(|e| format!("创建实例目录失败: {}", e))?;

    sync_shared_directory(
        profile_dir,
        &default_codex_home,
        Path::new(CODEX_SHARED_SKILLS_DIR_NAME),
    )?;
    sync_shared_directory(
        profile_dir,
        &default_codex_home,
        Path::new(CODEX_SHARED_RULES_DIR_NAME),
    )?;
    sync_shared_directory(
        profile_dir,
        &default_codex_home,
        Path::new(CODEX_SHARED_VENDOR_IMPORTS_SKILLS_DIR),
    )?;
    sync_shared_file(
        profile_dir,
        &default_codex_home,
        Path::new(CODEX_SHARED_AGENTS_FILE_NAME),
    )?;

    Ok(())
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
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
            Some("__default__") | None => get_default_codex_home()?,
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
                modules::logger::log_info(&format!(
                    "[Codex Instance] 复制来源实例需要空目录，但目标已存在: {}",
                    resolved_path
                ));
                return Err(format!("复制来源实例需要目标目录为空: {}", resolved_path));
            }
        }

        if !source_dir.exists() {
            return Err("未找到复制来源目录，请先确保来源实例已初始化".to_string());
        }

        instance_store::copy_dir_recursive(&source_dir, &user_dir_path)?;
    }

    ensure_instance_shared_skills(&user_dir_path)?;

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
        launch_mode: params.launch_mode.unwrap_or_default(),
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };

    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
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
    if let Some(working_dir) = params.working_dir {
        instance.working_dir = if working_dir.trim().is_empty() {
            None
        } else {
            Some(working_dir.trim().to_string())
        };
    }
    if let Some(bind) = params.bind_account_id.clone() {
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
    let _lock = CODEX_INSTANCE_STORE_LOCK
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
    let _lock = CODEX_INSTANCE_STORE_LOCK
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

pub fn update_instance_after_cli_prepare(instance_id: &str) -> Result<InstanceProfile, String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
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

pub fn update_instance_pid(instance_id: &str, pid: Option<u32>) -> Result<InstanceProfile, String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
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
    let _lock = CODEX_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = pid;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn clear_all_pids() -> Result<(), String> {
    let _lock = CODEX_INSTANCE_STORE_LOCK
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

pub fn replace_bind_account_references(
    old_account_id: &str,
    new_account_id: &str,
) -> Result<(), String> {
    let old_id = old_account_id.trim();
    let new_id = new_account_id.trim();
    if old_id.is_empty() || new_id.is_empty() || old_id == new_id {
        return Ok(());
    }

    let _lock = CODEX_INSTANCE_STORE_LOCK
        .lock()
        .map_err(|_| "无法获取实例锁")?;
    let mut store = load_instance_store()?;
    let mut changed = false;

    if store.default_settings.bind_account_id.as_deref() == Some(old_id) {
        store.default_settings.bind_account_id = Some(new_id.to_string());
        store.default_settings.follow_local_account = false;
        changed = true;
    }

    for instance in &mut store.instances {
        if instance.bind_account_id.as_deref() == Some(old_id) {
            instance.bind_account_id = Some(new_id.to_string());
            changed = true;
        }
    }

    if changed {
        save_instance_store(&store)?;
    }

    Ok(())
}

pub async fn inject_account_to_profile(profile_dir: &Path, account_id: &str) -> Result<(), String> {
    let account = modules::codex_account::prepare_account_for_injection_from_auth_dir(
        account_id,
        Some(profile_dir),
    )
    .await?;
    modules::codex_account::write_account_bundle_to_dir(profile_dir, &account)
}
