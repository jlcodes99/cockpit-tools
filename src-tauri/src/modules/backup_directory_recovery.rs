//! Persist an explicitly chosen backup directory outside the application data
//! directory, so an unavailable data root cannot block backup-directory recovery.
//! This preference never changes the account/configuration data root.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

const OVERRIDE_SUFFIX: &str = ".backup-directory.json";
const MAX_OVERRIDE_BYTES: u64 = 128 * 1024;
const DATA_ROOT_ENV_KEYS: [&str; 3] = [
    "COCKPIT_TOOLS_TEST_DATA_DIR",
    "COCKPIT_TEST_DATA_DIR",
    "COCKPIT_TOOLS_DATA_DIR",
];

#[derive(Serialize, Deserialize)]
struct BackupDirectoryOverride {
    directory_path: PathBuf,
}

fn environment_data_root(mut read: impl FnMut(&str) -> Option<String>) -> Option<PathBuf> {
    DATA_ROOT_ENV_KEYS.iter().find_map(|key| {
        read(key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn adjacent_override_path(root: &Path) -> Result<PathBuf, String> {
    // Append to the root's name instead of placing anything inside the root:
    // it can be an occupied file or a dangling link during recovery.
    let name = root
        .file_name()
        .ok_or_else(|| format!("无法定位备份目录恢复配置: {}", root.display()))?;
    let parent = root
        .parent()
        .ok_or_else(|| format!("无法定位备份目录恢复配置: {}", root.display()))?;
    let mut file_name = name.to_os_string();
    file_name.push(OVERRIDE_SUFFIX);
    Ok(parent.join(file_name))
}

fn override_path() -> Result<PathBuf, String> {
    if let Some(root) = environment_data_root(|key| std::env::var(key).ok()) {
        return adjacent_override_path(&root);
    }
    #[cfg(test)]
    return adjacent_override_path(&crate::modules::test_support::fallback_data_dir());
    #[cfg(not(test))]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        let name = if crate::modules::data_paths::is_dev_profile() {
            crate::modules::data_paths::DEV_DATA_DIR
        } else {
            crate::modules::data_paths::DATA_DIR
        };
        adjacent_override_path(&home.join(name))
    }
}

fn read_override_at(path: &Path) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "读取备份目录恢复配置失败({}): {error}",
                path.display()
            ));
        }
        Ok(_) => {}
    }
    // A dangling configuration link is an error, rather than an absent setting.
    let file = fs::File::open(path)
        .map_err(|error| format!("读取备份目录恢复配置失败({}): {error}", path.display()))?;
    let mut contents = String::new();
    file.take(MAX_OVERRIDE_BYTES + 1)
        .read_to_string(&mut contents)
        .map_err(|error| format!("读取备份目录恢复配置失败({}): {error}", path.display()))?;
    if contents.len() as u64 > MAX_OVERRIDE_BYTES {
        return Err(format!("备份目录恢复配置过大: {}", path.display()));
    }
    let preference: BackupDirectoryOverride = serde_json::from_str(&contents)
        .map_err(|error| format!("备份目录恢复配置损坏({}): {error}", path.display()))?;
    if !preference.directory_path.is_absolute() {
        return Err(format!(
            "备份目录恢复配置必须使用绝对路径: {}",
            path.display()
        ));
    }
    // Do not probe/create the backup target on the read path. An unavailable
    // target must remain visible and selectable for a subsequent explicit change.
    Ok(Some(preference.directory_path))
}

pub fn read_override() -> Result<Option<PathBuf>, String> {
    read_override_at(&override_path()?)
}

pub fn has_override() -> Result<bool, String> {
    override_exists_at(&override_path()?)
}

fn override_exists_at(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "读取备份目录恢复配置状态失败({}): {error}",
            path.display()
        )),
    }
}

struct WriteProbe {
    path: PathBuf,
    active: bool,
}

impl Drop for WriteProbe {
    fn drop(&mut self) {
        // Best-effort cleanup on write/sync failure; normal cleanup is checked.
        if self.active {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn prepare_target(target: &Path) -> Result<(), String> {
    if !target.is_absolute() {
        return Err("备份目录必须是非空绝对路径".to_string());
    }
    match fs::symlink_metadata(target) {
        Ok(_) if !target.is_dir() => {
            return Err(format!("备份目录不可用: {}", target.display()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(target)
                .map_err(|error| format!("创建新备份目录失败({}): {error}", target.display()))?;
        }
        Err(error) => {
            return Err(format!("读取备份目录失败({}): {error}", target.display()));
        }
    }
    let probe_path = target.join(format!(
        ".cockpit-backup-write-probe-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|error| format!("备份目录不可写({}): {error}", target.display()))?;
    // Claim cleanup ownership only after create_new succeeded, so even an
    // unexpected name collision cannot remove a pre-existing file.
    let mut probe = WriteProbe {
        path: probe_path,
        active: true,
    };
    let written = file
        .write_all(b"Cockpit backup directory write probe\n")
        .and_then(|_| file.sync_all());
    // Windows requires closing the probe before removing it.
    drop(file);
    written.map_err(|error| format!("备份目录不可写({}): {error}", target.display()))?;
    fs::remove_file(&probe.path)
        .map_err(|error| format!("清理备份目录写入检查失败({}): {error}", target.display()))?;
    probe.active = false;
    Ok(())
}

fn save_override_at_with(
    path: &Path,
    target: &Path,
    persist: impl FnOnce(&Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    prepare_target(target)?;
    let contents = serde_json::to_string_pretty(&BackupDirectoryOverride {
        directory_path: target.to_path_buf(),
    })
    .map_err(|error| format!("序列化备份目录恢复配置失败: {error}"))?;
    persist(path, &contents)
}

pub fn save_override(target: &Path) -> Result<(), String> {
    save_override_at_with(&override_path()?, target, persist_override)
}

fn persist_override(path: &Path, contents: &str) -> Result<(), String> {
    crate::modules::atomic_write::update_string_atomic(path, |current| {
        if let Some(previous) = current {
            if serde_json::from_str::<serde_json::Value>(previous).is_err() {
                // The ordinary atomic writer only backs up valid JSON. Keep a
                // damaged preference too, before the user's explicit recovery.
                let mut backup_name = path.as_os_str().to_os_string();
                backup_name.push(".bak");
                crate::modules::atomic_write::write_bytes_atomic(
                    &PathBuf::from(backup_name),
                    previous.as_bytes(),
                )?;
            }
        }
        Ok(contents.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "cockpit-backup-directory-recovery-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn preference(&self) -> PathBuf {
            adjacent_override_path(&self.0.join(".cockpit_tools")).unwrap()
        }

        fn save(&self, target: &Path) -> Result<(), String> {
            save_override_at_with(&self.preference(), target, persist_override)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reloads_persisted_directory_without_touching_unavailable_data_root() {
        let root = TestRoot::new();
        let blocked = root.0.join(".cockpit_tools");
        fs::write(&blocked, b"existing data root blocker").unwrap();
        let target = root.0.join("backups");
        root.save(&target).unwrap();
        assert_eq!(
            read_override_at(&root.preference()).unwrap(),
            Some(target.clone())
        );
        assert_eq!(fs::read(&blocked).unwrap(), b"existing data root blocker");
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
        // Reads preserve the explicit setting even if that disk is later absent.
        fs::remove_dir(&target).unwrap();
        assert_eq!(read_override_at(&root.preference()).unwrap(), Some(target));
    }

    #[test]
    fn preserves_old_data_and_setting_when_new_target_is_a_file() {
        let root = TestRoot::new();
        let old = root.0.join("old-backups");
        root.save(&old).unwrap();
        fs::write(old.join("accounts.json"), b"old backup").unwrap();
        let occupied = root.0.join("occupied");
        fs::write(&occupied, b"keep this file").unwrap();
        assert!(root.save(&occupied).is_err());
        assert_eq!(
            read_override_at(&root.preference()).unwrap(),
            Some(old.clone())
        );
        assert_eq!(fs::read(old.join("accounts.json")).unwrap(), b"old backup");
        assert_eq!(fs::read(occupied).unwrap(), b"keep this file");
    }

    #[test]
    fn rejects_relative_targets_and_invalid_configuration() {
        let root = TestRoot::new();
        assert!(root.save(Path::new("relative")).is_err());
        assert!(!root.preference().exists());
        assert_eq!(read_override_at(&root.preference()).unwrap(), None);
        fs::write(root.preference(), b"broken json").unwrap();
        assert!(read_override_at(&root.preference()).is_err());
        fs::write(root.preference(), r#"{"directory_path":"relative"}"#).unwrap();
        assert!(read_override_at(&root.preference()).is_err());
    }

    #[test]
    fn failed_persistence_keeps_previous_setting() {
        let root = TestRoot::new();
        let old = root.0.join("old");
        root.save(&old).unwrap();
        let previous = fs::read(root.preference()).unwrap();
        let error = save_override_at_with(&root.preference(), &root.0.join("new"), |_, _| {
            Err("injected disk write failure".into())
        });
        assert!(error.is_err());
        assert_eq!(fs::read(root.preference()).unwrap(), previous);
        assert_eq!(read_override_at(&root.preference()).unwrap(), Some(old));
    }

    #[test]
    fn explicit_recovery_replaces_corrupt_preference_and_keeps_a_backup() {
        let root = TestRoot::new();
        let blocked = root.0.join(".cockpit_tools");
        fs::write(&blocked, b"existing data root blocker").unwrap();
        fs::write(root.preference(), b"damaged json").unwrap();
        assert!(override_exists_at(&root.preference()).unwrap());
        assert!(read_override_at(&root.preference()).is_err());
        let target = root.0.join("recovered-backups");
        root.save(&target).unwrap();
        assert_eq!(read_override_at(&root.preference()).unwrap(), Some(target));
        assert_eq!(
            fs::read(root.0.join(".cockpit_tools.backup-directory.json.bak")).unwrap(),
            b"damaged json",
        );
        assert_eq!(fs::read(blocked).unwrap(), b"existing data root blocker");
    }

    #[test]
    fn isolates_profiles_and_environment_roots_with_account_priority() {
        let root = TestRoot::new();
        let production = adjacent_override_path(&root.0.join(".cockpit_tools")).unwrap();
        let dev = adjacent_override_path(&root.0.join(".cockpit_tools_dev")).unwrap();
        let custom = adjacent_override_path(&root.0.join("custom")).unwrap();
        assert_ne!(production, dev);
        assert_ne!(production, custom);
        assert_eq!(
            production,
            root.0.join(".cockpit_tools.backup-directory.json")
        );
        assert_eq!(custom, root.0.join("custom.backup-directory.json"));
        let values = [Some("  test  "), Some("legacy-test"), Some("custom")];
        assert_eq!(
            environment_data_root(|key| DATA_ROOT_ENV_KEYS
                .iter()
                .position(|candidate| *candidate == key)
                .and_then(|index| values[index].map(str::to_string))),
            Some(PathBuf::from("test")),
        );
        assert_eq!(
            environment_data_root(|key| match key {
                "COCKPIT_TOOLS_TEST_DATA_DIR" => Some("  ".into()),
                "COCKPIT_TEST_DATA_DIR" => Some("legacy-test".into()),
                "COCKPIT_TOOLS_DATA_DIR" => Some("custom".into()),
                _ => None,
            }),
            Some(PathBuf::from("legacy-test")),
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_dangling_targets_and_retains_original_link_and_setting() {
        let root = TestRoot::new();
        let old = root.0.join("old");
        root.save(&old).unwrap();
        let missing = root.0.join("missing");
        let link = root.0.join("dangling");
        std::os::unix::fs::symlink(&missing, &link).unwrap();
        assert!(root.save(&link).is_err());
        assert_eq!(fs::read_link(&link).unwrap(), missing);
        assert_eq!(read_override_at(&root.preference()).unwrap(), Some(old));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_recovery_replaces_dangling_preference_link() {
        let root = TestRoot::new();
        let missing = root.0.join("missing-preference");
        std::os::unix::fs::symlink(&missing, root.preference()).unwrap();
        assert!(override_exists_at(&root.preference()).unwrap());
        assert!(read_override_at(&root.preference()).is_err());
        let target = root.0.join("recovered-backups");
        root.save(&target).unwrap();
        assert_eq!(read_override_at(&root.preference()).unwrap(), Some(target));
        assert!(!missing.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn refuses_dangling_junction_targets_without_replacing_them() {
        let root = TestRoot::new();
        let missing = root.0.join("missing");
        fs::create_dir(&missing).unwrap();
        let link = root.0.join("dangling");
        junction::create(&missing, &link).unwrap();
        fs::remove_dir(&missing).unwrap();
        assert!(root.save(&link).is_err());
        assert!(fs::symlink_metadata(&link).is_ok());
        assert!(!root.preference().exists());
    }
}
