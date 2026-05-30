use crate::modules;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const HISTORY_FILE: &str = "wakeup_history.json";
const MAX_HISTORY_ITEMS: usize = 100;

static HISTORY_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupHistoryItem {
    pub id: String,
    pub timestamp: i64,
    pub trigger_type: String,
    pub trigger_source: String,
    pub task_name: Option<String>,
    pub account_email: String,
    pub model_id: String,
    pub prompt: Option<String>,
    pub success: bool,
    pub message: Option<String>,
    pub duration: Option<u64>,
}

fn history_path() -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    Ok(data_dir.join(HISTORY_FILE))
}

/// 加载唤醒历史记录
pub fn load_history() -> Result<Vec<WakeupHistoryItem>, String> {
    let path = history_path()?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("读取唤醒历史失败: {}", e))?;

    if content.trim().is_empty() {
        return Ok(Vec::new());
    }

    let items: Vec<WakeupHistoryItem> = match serde_json::from_str(&content) {
        Ok(items) => items,
        Err(error) => {
            match modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => modules::logger::log_warn(&format!(
                    "唤醒历史解析失败，已隔离并使用空历史: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => modules::logger::log_warn(&format!(
                    "唤醒历史解析失败，文件已不存在，使用空历史: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => modules::logger::log_warn(&format!(
                    "唤醒历史解析失败，隔离失败，使用空历史: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Vec::new()
        }
    };

    Ok(items)
}

/// 保存唤醒历史记录
fn save_history(items: &[WakeupHistoryItem]) -> Result<(), String> {
    let path = history_path()?;
    let content =
        serde_json::to_string_pretty(items).map_err(|e| format!("序列化唤醒历史失败: {}", e))?;
    modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存唤醒历史失败: {}", e))
}

/// 添加历史记录（自动去重、限制数量）
pub fn add_history_items(new_items: Vec<WakeupHistoryItem>) -> Result<(), String> {
    if new_items.is_empty() {
        return Ok(());
    }

    let _lock = HISTORY_LOCK.lock().map_err(|_| "获取历史锁失败")?;

    let mut existing = load_history().unwrap_or_default();

    // 去重：根据 ID 过滤已存在的记录
    let existing_ids: std::collections::HashSet<String> =
        existing.iter().map(|item| item.id.clone()).collect();
    let filtered_new: Vec<WakeupHistoryItem> = new_items
        .into_iter()
        .filter(|item| !existing_ids.contains(&item.id))
        .collect();

    if filtered_new.is_empty() {
        return Ok(());
    }

    // 新记录放前面
    let mut merged = filtered_new;
    merged.append(&mut existing);

    // 按时间排序（最新的在前）
    merged.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    // 限制数量
    merged.truncate(MAX_HISTORY_ITEMS);

    save_history(&merged)
}

/// 清空历史记录
pub fn clear_history() -> Result<(), String> {
    let _lock = HISTORY_LOCK.lock().map_err(|_| "获取历史锁失败")?;
    save_history(&[])
}
