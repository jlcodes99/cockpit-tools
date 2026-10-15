//! Shared desktop/CLI storage paths. Upgrade by adding an alias, never by moving
//! live profiles: their canonical paths also identify Codex window storage.
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

pub const DATA_DIR: &str = ".cockpit_tools";
pub const DEV_DATA_DIR: &str = ".cockpit_tools_dev";
const LEGACY_DATA_DIR: &str = ".antigravity_cockpit";
const LEGACY_DEV_DATA_DIR: &str = ".antigravity_cockpit_dev";
static DATA_ROOT: OnceLock<PathBuf> = OnceLock::new();
static DEV_ROOT: OnceLock<PathBuf> = OnceLock::new();
#[cfg(target_os = "windows")]
static WINDOWS_INSTANCE_ROOT: OnceLock<PathBuf> = OnceLock::new();
static COMPATIBILITY_TASKS: LazyLock<Mutex<HashSet<PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn is_dev_profile() -> bool {
    std::env::var("COCKPIT_TOOLS_PROFILE")
        .map(|value| value.trim().eq_ignore_ascii_case("dev"))
        .unwrap_or(false)
}

fn custom_data_dir() -> Option<PathBuf> {
    std::env::var("COCKPIT_TOOLS_DATA_DIR")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn resolve_data_dir() -> Result<PathBuf, String> {
    if let Some(path) = custom_data_dir() {
        return Ok(path);
    }
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let dev = is_dev_profile();
    resolve_cached_root(if dev { &DEV_ROOT } else { &DATA_ROOT }, &home, dev)
}

pub fn fallback_data_dir() -> PathBuf {
    if let Ok(path) = resolve_data_dir() {
        return path;
    }
    let base = dirs::home_dir().unwrap_or_default();
    let dev = is_dev_profile();
    let legacy = base.join(if dev {
        LEGACY_DEV_DATA_DIR
    } else {
        LEGACY_DATA_DIR
    });
    // A failed read must not redirect a legacy installation into an empty store.
    match fs::symlink_metadata(&legacy) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            base.join(if dev { DEV_DATA_DIR } else { DATA_DIR })
        }
        _ => legacy,
    }
}

/// Keep the historical Windows Roaming location for these platform profiles.
/// Custom/development storage is isolated beneath its explicitly selected root.
pub fn managed_instances_root_dir(platform: &str) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let root = if custom_data_dir().is_some() || is_dev_profile() {
        resolve_data_dir()?
    } else {
        let roaming = std::env::var_os("APPDATA").ok_or("无法获取 APPDATA 环境变量")?;
        resolve_cached_root(&WINDOWS_INSTANCE_ROOT, Path::new(&roaming), false)?
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let root = resolve_data_dir()?;
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    return Ok(root.join("instances").join(platform));
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    Err(format!("不支持的平台实例目录: {platform}"))
}

struct RootSelection {
    path: PathBuf,
    compatibility: Option<(PathBuf, PathBuf)>,
}

fn metadata_if_present(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "读取数据目录状态失败 ({}): {error}",
            path.display()
        )),
    }
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn is_directory_link(metadata: &fs::Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(target_os = "windows"))]
    metadata.file_type().is_symlink()
}

/// Remove only our verified public alias prefix for fail-closed cleanup checks.
/// Never canonicalize the remainder: linked profiles/ancestors must stay visible.
pub fn without_compatibility_alias(path: &Path) -> PathBuf {
    // Custom roots keep their existing fail-closed link policy. A matching
    // directory name inside a profile is never enough to authorize cleanup.
    if custom_data_dir().is_some() {
        return path.to_path_buf();
    }
    if let Some(home) = dirs::home_dir() {
        let resolved = without_compatibility_alias_at(path, &home);
        if resolved != path {
            return resolved;
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(roaming) = std::env::var_os("APPDATA") {
        return without_compatibility_alias_at(path, Path::new(&roaming));
    }
    path.to_path_buf()
}

fn without_compatibility_alias_at(path: &Path, base: &Path) -> PathBuf {
    for (current_name, legacy_name) in [
        (DATA_DIR, LEGACY_DATA_DIR),
        (DEV_DATA_DIR, LEGACY_DEV_DATA_DIR),
    ] {
        let alias = base.join(current_name);
        let Ok(relative) = path.strip_prefix(&alias) else {
            continue;
        };
        let legacy = base.join(legacy_name);
        let (Ok(alias_metadata), Ok(legacy_metadata)) =
            (fs::symlink_metadata(&alias), fs::symlink_metadata(&legacy))
        else {
            continue;
        };
        if is_directory_link(&alias_metadata)
            && legacy_metadata.is_dir()
            && !is_directory_link(&legacy_metadata)
            && same_directory(&alias, &legacy)
        {
            return legacy.join(relative);
        }
    }
    path.to_path_buf()
}

fn select_root(base: &Path, dev: bool) -> Result<RootSelection, String> {
    let current = base.join(if dev { DEV_DATA_DIR } else { DATA_DIR });
    let legacy = base.join(if dev {
        LEGACY_DEV_DATA_DIR
    } else {
        LEGACY_DATA_DIR
    });
    // symlink_metadata also detects dangling aliases; they must never be
    // mistaken for fresh installs or overwritten with an empty directory.
    let current_metadata = metadata_if_present(&current)?;
    let legacy_metadata = metadata_if_present(&legacy)?;
    if legacy_metadata.is_some() {
        if !legacy.is_dir() {
            return Err(format!("旧数据目录不可用: {}", legacy.display()));
        }
        if current_metadata.is_some() && same_directory(&current, &legacy) {
            return Ok(RootSelection {
                path: current,
                compatibility: None,
            });
        }
        // Even when both independent directories exist, retain the established
        // store and let the background task report the conflict without merging.
        return Ok(RootSelection {
            path: legacy.clone(),
            compatibility: Some((current, legacy)),
        });
    }
    if current_metadata.is_some() && !current.is_dir() {
        return Err(format!("数据目录不可用: {}", current.display()));
    }
    Ok(RootSelection {
        path: current,
        compatibility: None,
    })
}

fn resolve_cached_root(
    cache: &OnceLock<PathBuf>,
    base: &Path,
    dev: bool,
) -> Result<PathBuf, String> {
    if let Some(path) = cache.get() {
        return Ok(path.clone());
    }
    // Cache only successful choices, so a transient filesystem failure can be
    // retried instead of making this session permanently unable to load accounts.
    let selection = select_root(base, dev)?;
    if cache.set(selection.path).is_ok() {
        if let Some((alias, legacy)) = selection.compatibility {
            schedule_compatibility_link(alias, legacy);
        }
    }
    // Concurrent callers always return the winning choice. Open log files and
    // cached absolute paths remain stable until the next process adopts the alias.
    Ok(cache.get().expect("successful root initialization").clone())
}

#[cfg(unix)]
fn create_directory_link(target: &Path, alias: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, alias)
}

#[cfg(target_os = "windows")]
fn create_directory_link(target: &Path, alias: &Path) -> io::Result<()> {
    // Junctions do not require Windows Developer Mode or elevated privileges.
    junction::create(target, alias)
}

#[cfg(not(any(unix, target_os = "windows")))]
fn create_directory_link(_target: &Path, _alias: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "directory links unavailable",
    ))
}

fn ensure_compatibility_link(alias: &Path, legacy: &Path) -> Result<(), String> {
    ensure_compatibility_link_with(alias, legacy, create_directory_link)
}

fn ensure_compatibility_link_with(
    alias: &Path,
    legacy: &Path,
    create: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    if metadata_if_present(alias)?.is_some() {
        return if same_directory(alias, legacy) {
            Ok(())
        } else {
            Err(format!("新目录已被占用，保留旧目录: {}", alias.display()))
        };
    }
    let target =
        fs::canonicalize(legacy).map_err(|error| format!("解析旧数据目录失败: {error}"))?;
    if !target.is_dir() {
        return Err("旧数据目录不可用".to_owned());
    }
    // Creating a directory link is exclusive. Another desktop/CLI process may
    // win the race, but an independently occupied destination is never replaced.
    let result = create(&target, alias);
    if same_directory(alias, legacy) {
        return Ok(());
    }
    Err(format!(
        "创建数据目录兼容链接失败 ({}): {}",
        alias.display(),
        result
            .err()
            .map(|error| error.to_string())
            .unwrap_or_else(|| "链接目标不一致".into())
    ))
}

fn claim_compatibility_task(alias: &Path) -> bool {
    COMPATIBILITY_TASKS
        .lock()
        .map(|mut tasks| tasks.insert(alias.to_path_buf()))
        .unwrap_or(false)
}

fn schedule_compatibility_link(alias: PathBuf, legacy: PathBuf) {
    if !claim_compatibility_task(&alias) {
        return;
    }
    let task_alias = alias.clone();
    let result = std::thread::Builder::new()
        .name("cockpit-data-path-compat".into())
        .spawn(move || {
            for attempt in 0..3 {
                match ensure_compatibility_link(&task_alias, &legacy) {
                    Ok(()) => {
                        crate::modules::logger::log_info(&format!(
                            "[DataPaths] 新目录入口已就绪，保留现有数据与实例路径: {}",
                            task_alias.display()
                        ));
                        return;
                    }
                    Err(error) => {
                        if attempt == 2 {
                            crate::modules::logger::log_warn(&format!(
                                "[DataPaths] 静默兼容未完成，继续使用旧目录，下次启动重试: {error}"
                            ));
                            return;
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(1 << attempt));
            }
        });
    if result.is_err() {
        if let Ok(mut tasks) = COMPATIBILITY_TASKS.lock() {
            tasks.remove(&alias);
        }
    }
}

#[cfg(test)]
#[path = "data_paths_tests.rs"]
mod tests;
