//! 跨版本保留的用户界面记忆（已关闭引导、不再提示等）。
//! 存在数据目录，不依赖 WebView localStorage。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::modules::account;
use crate::modules::atomic_write::write_string_atomic;

const USER_MEMORY_FILE: &str = "user_memory.json";

static MEMORY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserMemory {
    #[serde(default)]
    pub dismissed: BTreeMap<String, bool>,
    #[serde(default)]
    pub lists: BTreeMap<String, Vec<String>>,
}

fn memory_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(USER_MEMORY_FILE))
}

fn read_memory_from_path(path: &PathBuf) -> Result<UserMemory, String> {
    if !path.exists() {
        return Ok(UserMemory::default());
    }
    let raw =
        std::fs::read_to_string(path).map_err(|error| format!("读取用户记忆失败: {error}"))?;
    if raw.trim().is_empty() {
        return Ok(UserMemory::default());
    }
    serde_json::from_str(&raw).or_else(|_| Ok(UserMemory::default()))
}

fn write_memory_to_path(path: &PathBuf, memory: &UserMemory) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(memory)
        .map_err(|error| format!("序列化用户记忆失败: {error}"))?;
    write_string_atomic(path, &raw)
}

pub fn load_user_memory() -> Result<UserMemory, String> {
    let _guard = MEMORY_LOCK
        .lock()
        .map_err(|_| "用户记忆锁已损坏".to_string())?;
    read_memory_from_path(&memory_path()?)
}

pub fn mark_user_memory_dismissed(id: &str) -> Result<UserMemory, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("记忆项不能为空".to_string());
    }
    let _guard = MEMORY_LOCK
        .lock()
        .map_err(|_| "用户记忆锁已损坏".to_string())?;
    let path = memory_path()?;
    let mut memory = read_memory_from_path(&path)?;
    memory.dismissed.insert(id.to_string(), true);
    write_memory_to_path(&path, &memory)?;
    Ok(memory)
}

fn sanitize_memory_list(items: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut next = Vec::new();
    for item in items {
        let id = item.trim().to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        next.push(id);
    }
    next
}

fn apply_memory_list(memory: &mut UserMemory, id: &str, items: Vec<String>) -> bool {
    let next = sanitize_memory_list(items);
    let existing_empty = memory
        .lists
        .get(id)
        .map(|value| value.is_empty())
        .unwrap_or(true);
    // 账号列表尚未就绪时前端可能写出空数组，禁止用空列表覆盖已有排序。
    if next.is_empty() && !existing_empty {
        return false;
    }
    if memory.lists.get(id) == Some(&next) {
        return false;
    }
    memory.lists.insert(id.to_string(), next);
    true
}

pub fn save_user_memory_list(id: &str, items: Vec<String>) -> Result<UserMemory, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("记忆项不能为空".to_string());
    }
    let _guard = MEMORY_LOCK
        .lock()
        .map_err(|_| "用户记忆锁已损坏".to_string())?;
    let path = memory_path()?;
    let mut memory = read_memory_from_path(&path)?;
    if apply_memory_list(&mut memory, id, items) {
        write_memory_to_path(&path, &memory)?;
    }
    Ok(memory)
}

#[cfg(test)]
mod tests {
    use super::{read_memory_from_path, write_memory_to_path, UserMemory};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn roundtrip_and_mark_dismissed() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-user-memory-{stamp}.json"));
        let _ = std::fs::remove_file(&path);

        let empty = read_memory_from_path(&path).expect("empty memory");
        assert!(empty.dismissed.is_empty());

        let mut memory = UserMemory::default();
        memory
            .dismissed
            .insert("codex.gateway_guide".to_string(), true);
        write_memory_to_path(&path, &memory).expect("write");

        let loaded = read_memory_from_path(&path).expect("reload");
        assert_eq!(loaded.dismissed.get("codex.gateway_guide"), Some(&true));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refuses_to_replace_existing_list_with_empty() {
        let mut memory = UserMemory::default();
        memory.lists.insert(
            "codex.accounts.custom_sort".to_string(),
            vec!["a".to_string(), "b".to_string()],
        );

        assert!(!super::apply_memory_list(
            &mut memory,
            "codex.accounts.custom_sort",
            Vec::new(),
        ));
        assert_eq!(
            memory.lists.get("codex.accounts.custom_sort"),
            Some(&vec!["a".to_string(), "b".to_string()])
        );

        assert!(super::apply_memory_list(
            &mut memory,
            "codex.accounts.custom_sort",
            vec!["b".to_string(), "c".to_string()],
        ));
        assert_eq!(
            memory.lists.get("codex.accounts.custom_sort"),
            Some(&vec!["b".to_string(), "c".to_string()])
        );
    }
}
