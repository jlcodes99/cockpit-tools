use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

static PATH_WRITE_LOCKS: LazyLock<Mutex<std::collections::HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

fn path_write_lock(path: &Path) -> Result<Arc<Mutex<()>>, String> {
    let normalized = path.to_path_buf();
    let mut locks = PATH_WRITE_LOCKS
        .lock()
        .map_err(|_| "文件写入锁表已损坏".to_string())?;
    Ok(locks
        .entry(normalized)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone())
}

fn format_io_error(action: &str, path: &Path, err: &std::io::Error) -> String {
    if let Some(error) = crate::modules::windows_operation::format_permission_io_error(
        "write_file",
        action,
        path.to_string_lossy().as_ref(),
        err,
    ) {
        return error;
    }
    format!("{}失败: path={}, error={}", action, path.display(), err)
}

fn build_backup_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("无法定位目标目录")?;
    let file_name = path
        .file_name()
        .and_then(|item| item.to_str())
        .ok_or_else(|| format!("无法解析目标文件名: {}", path.display()))?;
    Ok(parent.join(format!("{}.bak", file_name)))
}

fn build_temp_file_path(parent: &Path, target: &Path, suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{}.tmp.{}.{}.{}",
        target
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        unique,
        suffix
    ))
}

fn unique_timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

pub fn quarantine_file(path: &Path, reason: &str) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let parent = path.parent().ok_or("无法定位目标目录")?;
    let file_name = path
        .file_name()
        .and_then(|item| item.to_str())
        .ok_or_else(|| format!("无法解析目标文件名: {}", path.display()))?;
    let safe_reason = reason
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let quarantine_path = parent.join(format!(
        "{}.{}.{}",
        file_name,
        safe_reason,
        unique_timestamp_nanos()
    ));
    let prefix = format!("{}.{}.", file_name, safe_reason);
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            let candidate = entry.path();
            let Some(name) = candidate.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name.starts_with(&prefix) && candidate != quarantine_path {
                let _ = fs::remove_file(candidate);
            }
        }
    }
    fs::rename(path, &quarantine_path).map_err(|e| format_io_error("隔离损坏文件", path, &e))?;
    Ok(Some(quarantine_path))
}

fn is_json_path(path: &Path) -> bool {
    path.extension()
        .and_then(|item| item.to_str())
        .map(|item| item.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn content_is_safe_backup_source(path: &Path, content: &str) -> bool {
    if content.trim().is_empty() || content.as_bytes().contains(&0) {
        return false;
    }
    if !is_json_path(path) {
        return true;
    }
    serde_json::from_str::<serde_json::Value>(content).is_ok()
}

fn write_synced_temp_file_bytes(temp_path: &Path, content: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .map_err(|e| format_io_error("创建临时文件", temp_path, &e))?;
    file.write_all(content)
        .map_err(|e| format_io_error("写入临时文件", temp_path, &e))?;
    file.sync_all()
        .map_err(|e| format_io_error("同步临时文件", temp_path, &e))?;
    Ok(())
}

fn write_synced_temp_file(temp_path: &Path, content: &str) -> Result<(), String> {
    write_synced_temp_file_bytes(temp_path, content.as_bytes())
}

fn write_string_atomic_internal(
    path: &Path,
    content: &str,
    create_backup: bool,
) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format_io_error("创建目录", parent, &e))?;

    if create_backup && path.exists() {
        let backup_path = build_backup_path(path)?;
        if let Ok(existing_content) = fs::read_to_string(path) {
            if content_is_safe_backup_source(path, &existing_content) {
                if let Err(err) =
                    write_string_atomic_internal(&backup_path, &existing_content, false)
                {
                    crate::modules::logger::log_warn(&format!(
                        "写入备份文件失败，继续写入主文件: path={}, backup={}, error={}",
                        path.display(),
                        backup_path.display(),
                        err
                    ));
                }
            }
        }
    }

    let temp_path = build_temp_file_path(parent, path, "atomic");
    if let Err(err) = write_synced_temp_file(&temp_path, content) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_io_error("替换文件", path, &err));
    }

    Ok(())
}

pub fn write_string_atomic(path: &Path, content: &str) -> Result<(), String> {
    let lock = path_write_lock(path)?;
    let _guard = lock.lock().map_err(|_| "文件写入锁已损坏".to_string())?;
    write_string_atomic_internal(path, content, true)
}

pub fn write_string_atomic_if_hash_matches<F>(
    path: &Path,
    expected_hash: [u8; 32],
    build_content: F,
) -> Result<bool, String>
where
    F: FnOnce() -> Result<String, String>,
{
    let lock = path_write_lock(path)?;
    let _guard = lock.lock().map_err(|_| "文件写入锁已损坏".to_string())?;
    let current = match fs::read(path) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format_io_error("读取待迁移文件", path, &error)),
    };
    let current_hash: [u8; 32] = Sha256::digest(&current).into();
    if current_hash != expected_hash {
        return Ok(false);
    }
    let content = build_content()?;
    write_string_atomic_internal(path, &content, true)?;
    Ok(true)
}

pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let lock = path_write_lock(path)?;
    let _guard = lock.lock().map_err(|_| "文件写入锁已损坏".to_string())?;
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format_io_error("创建目录", parent, &e))?;

    let temp_path = build_temp_file_path(parent, path, "atomic");
    if let Err(err) = write_synced_temp_file_bytes(&temp_path, content) {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_io_error("替换文件", path, &err));
    }

    Ok(())
}

pub fn remove_file_locked(path: &Path) -> Result<bool, String> {
    let lock = path_write_lock(path)?;
    let _guard = lock.lock().map_err(|_| "文件写入锁已损坏".to_string())?;
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format_io_error("删除文件", path, &error)),
    }
}

pub fn restore_from_backup(path: &Path) -> Result<bool, String> {
    let backup_path = build_backup_path(path)?;
    if !backup_path.exists() {
        return Ok(false);
    }

    // Hold the same per-path write lock as normal writes so restore cannot race a
    // concurrent CAS migration or delete on this path.
    let lock = path_write_lock(path)?;
    let _guard = lock.lock().map_err(|_| "文件写入锁已损坏".to_string())?;

    let backup_content = fs::read_to_string(&backup_path)
        .map_err(|e| format_io_error("读取备份文件", &backup_path, &e))?;
    if !content_is_safe_backup_source(path, &backup_content) {
        return Ok(false);
    }
    write_string_atomic_internal(path, &backup_content, false)?;
    Ok(true)
}

pub fn parse_json_with_auto_restore<T: DeserializeOwned>(
    path: &Path,
    content: &str,
) -> Result<T, String> {
    match serde_json::from_str::<T>(content) {
        Ok(value) => Ok(value),
        Err(parse_err) => {
            let original_err = parse_err.to_string();
            if restore_from_backup(path)? {
                let restored_content = fs::read_to_string(path)
                    .map_err(|e| format_io_error("回滚后读取文件", path, &e))?;
                return serde_json::from_str::<T>(&restored_content).map_err(|e| {
                    format!(
                        "原始解析失败: {}; 已从 .bak 回滚，但回滚后仍解析失败: {}",
                        original_err, e
                    )
                });
            }
            Err(original_err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_backup_path, quarantine_file, remove_file_locked, restore_from_backup,
        write_string_atomic, write_string_atomic_if_hash_matches,
    };
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), unique));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn write_string_atomic_keeps_previous_backup_when_source_is_corrupted() {
        let dir = make_temp_dir("atomic_write_backup");
        let path = dir.join("accounts.json");
        let backup_path = build_backup_path(&path).expect("backup path");

        write_string_atomic(&path, r#"{"version":1}"#).expect("first write");
        write_string_atomic(&path, r#"{"version":2}"#).expect("second write");
        assert_eq!(
            fs::read_to_string(&backup_path).expect("read backup"),
            r#"{"version":1}"#
        );

        fs::write(&path, vec![0u8; 32]).expect("corrupt current file");
        write_string_atomic(&path, r#"{"version":3}"#).expect("third write");

        assert_eq!(
            fs::read_to_string(&path).expect("read current"),
            r#"{"version":3}"#
        );
        assert_eq!(
            fs::read_to_string(&backup_path).expect("read backup again"),
            r#"{"version":1}"#
        );
    }

    #[test]
    fn conditional_write_never_overwrites_newer_or_deleted_file() {
        let dir = make_temp_dir("atomic_write_cas");
        let path = dir.join("account.json");
        write_string_atomic(&path, r#"{"version":1}"#).expect("write initial");
        let expected: [u8; 32] = Sha256::digest(r#"{"version":1}"#.as_bytes()).into();

        write_string_atomic(&path, r#"{"version":2}"#).expect("write newer");
        assert!(!write_string_atomic_if_hash_matches(&path, expected, || {
            Ok(r#"{"version":3}"#.to_string())
        })
        .expect("reject stale write"));
        assert_eq!(
            fs::read_to_string(&path).expect("read newer"),
            r#"{"version":2}"#
        );

        remove_file_locked(&path).expect("delete account");
        assert!(!write_string_atomic_if_hash_matches(&path, expected, || {
            Ok(r#"{"version":3}"#.to_string())
        })
        .expect("reject deleted write"));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn quarantine_file_renames_existing_file_with_reason() {
        let dir = make_temp_dir("atomic_write_quarantine");
        let path = dir.join("state.json");
        fs::write(&path, r#"{"bad":true}"#).expect("write source");

        let quarantine_path = quarantine_file(&path, "invalid-json")
            .expect("quarantine result")
            .expect("quarantine path");

        assert!(!path.exists());
        assert!(quarantine_path.exists());
        assert!(quarantine_path
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or_default()
            .starts_with("state.json.invalid-json."));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn restore_from_backup_rejects_invalid_backup_content() {
        let dir = make_temp_dir("atomic_write_restore");
        let path = dir.join("state.json");
        let backup_path = build_backup_path(&path).expect("backup path");

        fs::write(&path, r#"{"version":1}"#).expect("write current");
        fs::write(&backup_path, vec![0u8; 16]).expect("write invalid backup");

        assert!(!restore_from_backup(&path).expect("restore result"));
        assert_eq!(
            fs::read_to_string(&path).expect("current should remain unchanged"),
            r#"{"version":1}"#
        );
    }
}
