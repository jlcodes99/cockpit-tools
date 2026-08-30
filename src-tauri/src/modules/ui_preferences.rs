//! 跨版本保留的侧边栏布局。
//! 存在数据目录，不依赖 WebView localStorage。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::modules::account;
use crate::modules::atomic_write::write_string_atomic;

const UI_PREFERENCES_FILE: &str = "ui_preferences.json";

static PREFERENCES_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UiPreferences {
    #[serde(default)]
    pub values: BTreeMap<String, String>,
}

fn preferences_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(UI_PREFERENCES_FILE))
}

fn read_preferences_from_path(path: &PathBuf) -> Result<UiPreferences, String> {
    if !path.exists() {
        return Ok(UiPreferences::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|error| format!("读取界面偏好失败: {error}"))?;
    if raw.trim().is_empty() {
        return Ok(UiPreferences::default());
    }
    serde_json::from_str(&raw).or_else(|_| Ok(UiPreferences::default()))
}

fn write_preferences_to_path(path: &PathBuf, preferences: &UiPreferences) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(preferences)
        .map_err(|error| format!("序列化界面偏好失败: {error}"))?;
    write_string_atomic(path, &raw)
}

pub fn load_ui_preferences() -> Result<UiPreferences, String> {
    let _guard = PREFERENCES_LOCK
        .lock()
        .map_err(|_| "界面偏好锁已损坏".to_string())?;
    read_preferences_from_path(&preferences_path()?)
}

fn apply_values(preferences: &mut UiPreferences, values: BTreeMap<String, String>) -> bool {
    let mut changed = false;
    for (key, value) in values {
        if preferences.values.get(&key) != Some(&value) {
            preferences.values.insert(key, value);
            changed = true;
        }
    }
    changed
}

pub fn save_ui_preferences(values: BTreeMap<String, String>) -> Result<UiPreferences, String> {
    let _guard = PREFERENCES_LOCK
        .lock()
        .map_err(|_| "界面偏好锁已损坏".to_string())?;
    let path = preferences_path()?;
    let mut preferences = read_preferences_from_path(&path)?;
    if apply_values(&mut preferences, values) {
        write_preferences_to_path(&path, &preferences)?;
    }
    Ok(preferences)
}

#[cfg(test)]
mod tests {
    use super::{read_preferences_from_path, write_preferences_to_path, UiPreferences};
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn roundtrip_preference_values() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-ui-preferences-{stamp}.json"));
        let _ = std::fs::remove_file(&path);

        let empty = read_preferences_from_path(&path).expect("empty preferences");
        assert!(empty.values.is_empty());

        let mut preferences = UiPreferences::default();
        preferences.values.insert(
            "agtools.platform_layout.v1".to_string(),
            "{\"sidebarEntryIds\":[\"group:codex-suite\"]}".to_string(),
        );
        write_preferences_to_path(&path, &preferences).expect("write");

        let loaded = read_preferences_from_path(&path).expect("reload");
        assert!(loaded
            .values
            .get("agtools.platform_layout.v1")
            .unwrap()
            .contains("group:codex-suite"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_or_invalid_file_returns_default() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-ui-preferences-bad-{stamp}.json"));
        std::fs::write(&path, "   ").expect("write empty");
        let empty = read_preferences_from_path(&path).expect("empty file");
        assert!(empty.values.is_empty());

        std::fs::write(&path, "{not-json").expect("write invalid");
        let invalid = read_preferences_from_path(&path).expect("invalid file");
        assert!(invalid.values.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_merges_without_dropping_other_keys() {
        let mut preferences = UiPreferences::default();
        preferences
            .values
            .insert("agtools.platform_layout.v1".to_string(), "old".to_string());
        preferences
            .values
            .insert("keep".to_string(), "yes".to_string());

        assert!(super::apply_values(
            &mut preferences,
            BTreeMap::from([(
                "agtools.platform_layout.v1".to_string(),
                "next".to_string()
            )]),
        ));
        assert_eq!(
            preferences.values.get("agtools.platform_layout.v1"),
            Some(&"next".to_string())
        );
        assert_eq!(preferences.values.get("keep"), Some(&"yes".to_string()));
        assert!(!super::apply_values(
            &mut preferences,
            BTreeMap::from([(
                "agtools.platform_layout.v1".to_string(),
                "next".to_string()
            )]),
        ));
    }
}
