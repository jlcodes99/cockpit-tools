use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

const OAUTH_PENDING_DIR: &str = "oauth_pending";

fn pending_dir_path() -> Result<PathBuf, String> {
    let data_dir = crate::modules::account::get_data_dir()?;
    let dir = data_dir.join(OAUTH_PENDING_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("创建 OAuth pending 目录失败: {}", e))?;
    }
    Ok(dir)
}

fn pending_file_path(file_name: &str) -> Result<PathBuf, String> {
    let normalized = file_name.trim();
    if normalized.is_empty() {
        return Err("OAuth pending 文件名不能为空".to_string());
    }
    if normalized.contains('/') || normalized.contains('\\') {
        return Err("OAuth pending 文件名非法".to_string());
    }
    Ok(pending_dir_path()?.join(normalized))
}

pub fn load<T>(file_name: &str) -> Result<Option<T>, String>
where
    T: DeserializeOwned,
{
    let path = pending_file_path(file_name)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("读取 OAuth pending 文件失败({}): {}", path.display(), e))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let parsed = serde_json::from_str::<T>(&raw)
        .map_err(|e| format!("解析 OAuth pending 文件失败({}): {}", path.display(), e))?;
    Ok(Some(parsed))
}

pub fn save<T>(file_name: &str, value: &T) -> Result<(), String>
where
    T: Serialize,
{
    let path = pending_file_path(file_name)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 OAuth pending 失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 OAuth pending 文件失败({}): {}", path.display(), e))?;
    Ok(())
}

pub fn clear(file_name: &str) -> Result<(), String> {
    let path = pending_file_path(file_name)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path)
        .map_err(|e| format!("删除 OAuth pending 文件失败({}): {}", path.display(), e))
}
