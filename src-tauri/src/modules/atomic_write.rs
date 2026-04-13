use std::fs;
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

fn write_string_atomic_internal(
    path: &Path,
    content: &str,
    create_backup: bool,
) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format_io_error("创建目录", parent, &e))?;

    if create_backup && path.exists() {
        let backup_path = build_backup_path(path)?;
        fs::copy(path, &backup_path)
            .map_err(|e| format_io_error("写入备份文件", &backup_path, &e))?;
    }

    let temp_path = build_temp_file_path(parent, path, "atomic");
    fs::write(&temp_path, content).map_err(|e| format_io_error("写入临时文件", &temp_path, &e))?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_io_error("替换文件", path, &err));
    }

    Ok(())
}

pub fn write_string_atomic(path: &Path, content: &str) -> Result<(), String> {
    write_string_atomic_internal(path, content, true)
}

pub fn restore_from_backup(path: &Path) -> Result<bool, String> {
    let backup_path = build_backup_path(path)?;
    if !backup_path.exists() {
        return Ok(false);
    }

    let backup_content = fs::read_to_string(&backup_path)
        .map_err(|e| format_io_error("读取备份文件", &backup_path, &e))?;
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
