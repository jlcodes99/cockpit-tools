use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{
    codex::CodexAppSpeed, DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile,
    InstanceStore,
};
use crate::modules::{self, instance::InstanceDefaults, instance_store};

pub use crate::modules::instance_store::{CreateInstanceParams, UpdateInstanceParams};

const INSTANCES_FILE: &str = "grok_instances.json";
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

pub fn get_default_grok_home() -> Result<PathBuf, String> {
    modules::grok_account::default_grok_home()
}

pub fn get_default_instances_root_dir() -> Result<PathBuf, String> {
    Ok(modules::account::get_data_dir()?.join("instances/grok"))
}

pub fn get_instance_defaults() -> Result<InstanceDefaults, String> {
    Ok(InstanceDefaults {
        root_dir: get_default_instances_root_dir()?
            .to_string_lossy()
            .to_string(),
        default_user_data_dir: get_default_grok_home()?.to_string_lossy().to_string(),
    })
}

pub fn is_profile_initialized(home: &Path) -> bool {
    home.is_dir()
}

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path)
        .map_err(|error| format!("创建 Grok 实例目录失败({}): {}", path.display(), error))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("设置 Grok 实例目录权限失败: {}", error))
}

#[cfg(not(unix))]
fn ensure_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("创建 Grok 实例目录失败({}): {}", path.display(), error))
}

fn ensure_empty_target(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_dir() {
        return Err("所选 Grok 实例路径不是目录".to_string());
    }
    if fs::read_dir(path)
        .map_err(|error| format!("读取 Grok 实例目录失败: {}", error))?
        .next()
        .is_some()
    {
        return Err(format!(
            "Grok 实例目标目录必须为空: {}",
            instance_store::display_path(path)
        ));
    }
    Ok(())
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("获取当前目录失败: {}", error))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err("Grok 实例目录包含无效的上级路径".to_string());
                }
            }
        }
    }
    Ok(normalized)
}

fn ensure_instance_path_within_root(path: &Path, root: &Path) -> Result<(), String> {
    let normalized_path = normalize_absolute_path(path)?;
    let normalized_root = normalize_absolute_path(root)?;
    if normalized_path == normalized_root {
        return Err("Grok 实例目录不能直接使用受管根目录".to_string());
    }

    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("解析 Grok 实例根目录失败: {}", error))?;
    let mut existing_ancestor = normalized_path.clone();
    while !existing_ancestor.exists() {
        if !existing_ancestor.pop() {
            return Err("无法解析 Grok 实例目录".to_string());
        }
    }
    let canonical_ancestor = fs::canonicalize(&existing_ancestor)
        .map_err(|error| format!("解析 Grok 实例目录失败: {}", error))?;
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(format!(
            "Grok 实例目录必须位于受管根目录内: {}",
            instance_store::display_path(root)
        ));
    }
    if path.exists() {
        let canonical_path =
            fs::canonicalize(path).map_err(|error| format!("解析 Grok 实例目录失败: {}", error))?;
        if canonical_path == canonical_root || !canonical_path.starts_with(&canonical_root) {
            return Err(format!(
                "Grok 实例目录必须位于受管根目录内: {}",
                instance_store::display_path(root)
            ));
        }
    }
    Ok(())
}

pub fn ensure_managed_instance_path(path: &Path) -> Result<(), String> {
    let root = get_default_instances_root_dir()?;
    ensure_private_dir(&root)?;
    ensure_instance_path_within_root(path, &root)
}

fn is_managed_instance_directory(path: &Path, root: &Path) -> Result<bool, String> {
    if !path.exists() || !root.exists() {
        return Ok(false);
    }
    let canonical_path =
        fs::canonicalize(path).map_err(|error| format!("解析 Grok 实例目录失败: {}", error))?;
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("解析 Grok 实例根目录失败: {}", error))?;
    Ok(canonical_path != canonical_root && canonical_path.starts_with(canonical_root))
}

pub fn create_instance(params: CreateInstanceParams) -> Result<InstanceProfile, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let name = instance_store::normalize_name(&params.name)?;
    let user_data_dir = params.user_data_dir.trim().to_string();
    if user_data_dir.is_empty() {
        return Err("Grok 实例目录不能为空".to_string());
    }
    instance_store::ensure_unique(&store, &name, &user_data_dir, None)?;

    let target = PathBuf::from(&user_data_dir);
    ensure_managed_instance_path(&target)?;
    ensure_empty_target(&target)?;
    ensure_private_dir(&target)?;

    let instance = InstanceProfile {
        id: Uuid::new_v4().to_string(),
        name,
        user_data_dir,
        working_dir: params
            .working_dir
            .and_then(|value| (!value.trim().is_empty()).then(|| value.trim().to_string())),
        extra_args: params.extra_args.trim().to_string(),
        bind_account_id: params.bind_account_id,
        model_routing: None,
        launch_mode: InstanceLaunchMode::Cli,
        app_speed: CodexAppSpeed::Standard,
        created_at: Utc::now().timestamp_millis(),
        last_launched_at: None,
        last_pid: None,
    };
    store.instances.push(instance.clone());
    save_instance_store(&store)?;
    Ok(instance)
}

pub fn update_instance(params: UpdateInstanceParams) -> Result<InstanceProfile, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == params.instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    let current_id = store.instances[index].id.clone();
    let current_dir = store.instances[index].user_data_dir.clone();
    let next_name = params
        .name
        .as_deref()
        .map(instance_store::normalize_name)
        .transpose()?;
    if let Some(name) = next_name.as_deref() {
        instance_store::ensure_unique(&store, name, &current_dir, Some(&current_id))?;
    }

    let instance = &mut store.instances[index];
    if let Some(name) = next_name {
        instance.name = name;
    }
    if let Some(working_dir) = params.working_dir {
        instance.working_dir =
            (!working_dir.trim().is_empty()).then(|| working_dir.trim().to_string());
    }
    if let Some(extra_args) = params.extra_args {
        instance.extra_args = extra_args.trim().to_string();
    }
    if let Some(bind_account_id) = params.bind_account_id {
        instance.bind_account_id = bind_account_id;
    }
    instance.launch_mode = InstanceLaunchMode::Cli;
    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn load_default_settings() -> Result<DefaultInstanceSettings, String> {
    Ok(load_instance_store()?.default_settings)
}

pub fn update_default_settings(
    bind_account_id: Option<Option<String>>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    follow_local_account: Option<bool>,
) -> Result<DefaultInstanceSettings, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    if let Some(value) = bind_account_id {
        store.default_settings.bind_account_id = value;
    }
    if let Some(value) = working_dir {
        store.default_settings.working_dir =
            (!value.trim().is_empty()).then(|| value.trim().to_string());
    }
    if let Some(value) = extra_args {
        store.default_settings.extra_args = value.trim().to_string();
    }
    if let Some(value) = follow_local_account {
        store.default_settings.follow_local_account = value;
    }
    store.default_settings.launch_mode = InstanceLaunchMode::Cli;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

/// 删除账号时调用：清除默认实例与多开实例上对该账号的绑定。
pub fn unbind_account(account_id: &str) -> Result<Vec<String>, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Ok(Vec::new());
    }
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let mut unbound_names: Vec<String> = Vec::new();

    if store.default_settings.bind_account_id.as_deref() == Some(account_id) {
        store.default_settings.bind_account_id = None;
        // 解绑后恢复跟随本地，避免默认实例空绑定且不跟随
        store.default_settings.follow_local_account = true;
        unbound_names.push("默认实例".to_string());
    }

    for instance in store.instances.iter_mut() {
        if instance.bind_account_id.as_deref() == Some(account_id) {
            instance.bind_account_id = None;
            unbound_names.push(instance.name.clone());
        }
    }

    if !unbound_names.is_empty() {
        save_instance_store(&store)?;
    }
    Ok(unbound_names)
}

pub fn delete_instance(instance_id: &str) -> Result<(), String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let index = store
        .instances
        .iter()
        .position(|instance| instance.id == instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    let directory = PathBuf::from(&store.instances[index].user_data_dir);
    if directory.exists() {
        let managed_root = get_default_instances_root_dir()?;
        if is_managed_instance_directory(&directory, &managed_root)? {
            modules::instance::delete_instance_directory(&directory)?;
        } else {
            modules::logger::log_warn(&format!(
                "[Grok Instance] 外部实例目录仅解除登记，不执行删除: {}",
                instance_store::display_path(&directory)
            ));
        }
    }
    store.instances.remove(index);
    save_instance_store(&store)
}

pub fn mark_launched(instance_id: &str, pid: Option<u32>) -> Result<InstanceProfile, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let instance = store
        .instances
        .iter_mut()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    instance.last_launched_at = Some(Utc::now().timestamp_millis());
    instance.last_pid = pid;
    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_instance_pid(instance_id: &str, pid: Option<u32>) -> Result<InstanceProfile, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    let instance = store
        .instances
        .iter_mut()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    instance.last_pid = pid;
    let updated = instance.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

pub fn update_default_pid(pid: Option<u32>) -> Result<DefaultInstanceSettings, String> {
    let _guard = STORE_LOCK.lock().map_err(|_| "获取 Grok 实例锁失败")?;
    let mut store = load_instance_store()?;
    store.default_settings.last_pid = pid;
    let updated = store.default_settings.clone();
    save_instance_store(&store)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_instance_path_within_root, get_default_grok_home, is_managed_instance_directory,
    };

    #[test]
    fn default_home_matches_official_location() {
        let home = get_default_grok_home().expect("default Grok home");
        assert_eq!(
            home.file_name().and_then(|value| value.to_str()),
            Some(".grok")
        );
    }

    #[test]
    fn only_descendants_of_managed_root_are_deletable() {
        let base = std::env::temp_dir().join(format!(
            "cockpit-grok-instance-delete-test-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("managed");
        let managed = root.join("instance-a");
        let external = base.join("external");
        std::fs::create_dir_all(&managed).expect("create managed instance");
        std::fs::create_dir_all(&external).expect("create external instance");

        assert!(is_managed_instance_directory(&managed, &root).expect("check managed descendant"));
        assert!(!is_managed_instance_directory(&root, &root).expect("check root itself"));
        assert!(!is_managed_instance_directory(&external, &root).expect("check external directory"));
        assert!(ensure_instance_path_within_root(&managed, &root).is_ok());
        assert!(ensure_instance_path_within_root(&external, &root).is_err());

        let _ = std::fs::remove_dir_all(base);
    }
}
