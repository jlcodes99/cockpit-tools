use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::models::InstanceStore;

#[derive(Debug, Clone)]
pub struct CreateInstanceParams {
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    pub copy_source_instance_id: Option<String>,
    pub init_mode: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateInstanceParams {
    pub instance_id: String,
    pub name: Option<String>,
    pub working_dir: Option<String>,
    pub extra_args: Option<String>,
    pub bind_account_id: Option<Option<String>>,
}

pub fn load_instance_store(path: &Path, _file_name: &str) -> Result<InstanceStore, String> {
    let (store, complete) = super::instance_registry::read(path, None)?;
    if !complete {
        schedule_registry(path);
    }
    Ok(store)
}

pub fn save_instance_store(
    path: &Path,
    _file_name: &str,
    store: &InstanceStore,
) -> Result<(), String> {
    super::instance_registry::save(path, store, true)
}

pub(crate) fn protect_instance_creation() -> Result<fs::File, String> {
    super::instance_registry::cleanup_gate(&super::account::get_data_dir()?, false)
}

fn schedule_registry(path: &Path) {
    use std::{path::PathBuf, sync::Mutex, time::Duration};
    static ACTIVE: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let mut active = match ACTIVE.lock() {
        Ok(active) => active,
        Err(_) => return,
    };
    if !active
        .get_or_insert_with(HashSet::new)
        .insert(path.to_path_buf())
    {
        return;
    }
    drop(active);
    let path = path.to_path_buf();
    let worker_path = path.clone();
    let worker = std::thread::Builder::new()
        .name("instance-registry-migration".into())
        .spawn(move || {
            loop {
                match super::instance_registry::migrate_batch(&worker_path) {
                    Ok(false) => std::thread::sleep(Duration::from_millis(150)),
                    Ok(true) => break,
                    Err(error) => {
                        super::logger::log_warn(&format!("[InstanceRegistry] {error}"));
                        break;
                    }
                }
            }
            if let Ok(mut active) = ACTIVE.lock() {
                if let Some(paths) = active.as_mut() {
                    paths.remove(&worker_path);
                }
            }
        });
    if let Err(error) = worker {
        if let Ok(mut active) = ACTIVE.lock() {
            if let Some(paths) = active.as_mut() {
                paths.remove(&path);
            }
        }
        super::logger::log_warn(&format!("[InstanceRegistry] {error}"));
    }
}

pub fn normalize_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("实例名称不能为空".to_string());
    }
    Ok(trimmed.to_string())
}

pub fn display_path(path: &Path) -> String {
    if path.is_absolute() {
        return path.to_string_lossy().to_string();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path).to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

pub fn ensure_unique(
    store: &InstanceStore,
    name: &str,
    user_data_dir: &str,
    current_id: Option<&str>,
) -> Result<(), String> {
    let mut names = HashSet::new();
    let mut dirs = HashSet::new();
    for instance in &store.instances {
        if let Some(id) = current_id {
            if instance.id == id {
                continue;
            }
        }
        names.insert(instance.name.to_lowercase());
        dirs.insert(instance.user_data_dir.to_lowercase());
    }
    if names.contains(&name.to_lowercase()) {
        return Err("实例名称已存在".to_string());
    }
    if dirs.contains(&user_data_dir.to_lowercase()) {
        return Err("实例目录已存在".to_string());
    }
    Ok(())
}

pub fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() {
        return Err(format!("源目录不存在: {}", src.to_string_lossy()));
    }

    if dst.exists() {
        let mut has_entries = false;
        if let Ok(mut iter) = fs::read_dir(dst) {
            if iter.next().is_some() {
                has_entries = true;
            }
        }
        if has_entries {
            return Err("目标目录已存在且不为空".to_string());
        }
    }

    fs::create_dir_all(dst).map_err(|e| format!("创建目标目录失败: {}", e))?;

    for entry in fs::read_dir(src).map_err(|e| format!("读取源目录失败: {}", e))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
        let path = entry.path();
        let target = dst.join(entry.file_name());

        let file_type = entry
            .file_type()
            .map_err(|e| format!("获取文件类型失败: {}", e))?;

        if file_type.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if file_type.is_file() {
            fs::copy(&path, &target).map_err(|e| format!("复制文件失败: {}", e))?;
        }
    }

    Ok(())
}
