use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;

fn format_io_error(action: &str, path: &Path, err: &std::io::Error) -> String {
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
                write_string_atomic_internal(&backup_path, &existing_content, false)?;
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
    write_string_atomic_internal(path, content, true)
}

pub fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
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

pub fn restore_from_backup(path: &Path) -> Result<bool, String> {
    let backup_path = build_backup_path(path)?;
    if !backup_path.exists() {
        return Ok(false);
    }

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
    use super::{build_backup_path, restore_from_backup, write_string_atomic};
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
