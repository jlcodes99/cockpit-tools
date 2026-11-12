//! 跨版本保留的界面偏好。
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
    #[serde(default)]
    pub revisions: BTreeMap<String, u64>,
}

fn preferences_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(UI_PREFERENCES_FILE))
}

fn read_preferences_from_path(path: &PathBuf) -> Result<UiPreferences, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(UiPreferences::default());
        }
        Err(error) => return Err(format!("读取界面偏好失败: {error}")),
    };
    if raw.trim().is_empty() {
        return Err("界面偏好文件为空，已保留原文件".into());
    }
    serde_json::from_str(&raw).map_err(|error| format!("界面偏好格式无效: {error}"))
}

fn write_preferences_to_path(path: &PathBuf, preferences: &UiPreferences) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(preferences)
        .map_err(|error| format!("序列化界面偏好失败: {error}"))?;
    write_string_atomic(path, &raw)
}

pub fn load_ui_preferences() -> Result<UiPreferences, String> {
    let _guard = PREFERENCES_LOCK
        .try_lock()
        .map_err(|_| "界面偏好正在读写，请重试".to_string())?;
    read_preferences_from_path(&preferences_path()?)
}

fn apply_values(preferences: &mut UiPreferences, values: BTreeMap<String, String>) -> bool {
    let mut changed = false;
    for (key, value) in values {
        if preferences.values.get(&key) != Some(&value) {
            let revision = preferences.revisions.entry(key.clone()).or_default();
            *revision = revision.saturating_add(1);
            preferences.values.insert(key, value);
            changed = true;
        }
    }
    changed
}

fn migrate_missing(preferences: &mut UiPreferences, legacy: BTreeMap<String, String>) -> bool {
    let missing = legacy
        .into_iter()
        .filter(|(key, _)| !preferences.values.contains_key(key))
        .collect();
    apply_values(preferences, missing)
}

fn update_value(
    preferences: &mut UiPreferences,
    key: String,
    value: String,
    expected_revision: u64,
) -> Result<bool, String> {
    if preferences.values.get(&key) == Some(&value) {
        return Ok(false);
    }
    let current_revision = preferences.revisions.get(&key).copied().unwrap_or(0);
    if current_revision != expected_revision {
        return Err("ui_preference_revision_conflict".into());
    }
    Ok(apply_values(preferences, BTreeMap::from([(key, value)])))
}

pub fn migrate_ui_preference_values(
    legacy: BTreeMap<String, String>,
) -> Result<UiPreferences, String> {
    mutate_preferences(|preferences| Ok(migrate_missing(preferences, legacy)))
}

pub fn update_ui_preference(
    key: String,
    value: String,
    expected_revision: u64,
) -> Result<UiPreferences, String> {
    mutate_preferences(|preferences| update_value(preferences, key, value, expected_revision))
}

fn mutate_preferences(
    edit: impl FnOnce(&mut UiPreferences) -> Result<bool, String>,
) -> Result<UiPreferences, String> {
    let _guard = PREFERENCES_LOCK
        .lock()
        .map_err(|_| "界面偏好写入锁不可用".to_string())?;
    let path = preferences_path()?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|error| format!("打开界面偏好写入锁失败: {error}"))?;
    fs2::FileExt::try_lock_exclusive(&lock)
        .map_err(|error| format!("获取界面偏好写入锁失败: {error}"))?;
    let mut preferences = read_preferences_from_path(&path)?;
    if edit(&mut preferences)? {
        write_preferences_to_path(&path, &preferences)?;
    }
    Ok(preferences)
}

fn check_layout_revision(
    preferences: &UiPreferences,
    values: &BTreeMap<String, String>,
) -> Result<(), String> {
    const KEY: &str = "agtools.platform_layout.v1";
    let revision = |raw: Option<&String>| -> u64 {
        raw.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| {
                value
                    .get("_layoutUpdatedAt")
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(0)
    };
    if values.contains_key(KEY) && revision(values.get(KEY)) < revision(preferences.values.get(KEY))
    {
        return Err("平台布局已在其他窗口更新，已保留较新设置，请重新加载后重试".into());
    }
    Ok(())
}

pub fn save_ui_preferences(values: BTreeMap<String, String>) -> Result<UiPreferences, String> {
    mutate_preferences(|preferences| {
        check_layout_revision(preferences, &values)?;
        Ok(apply_values(preferences, values))
    })
}

#[cfg(test)]
mod tests {
    use super::{read_preferences_from_path, write_preferences_to_path, UiPreferences};
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn stale_window_cannot_overwrite_newer_layout() {
        let key = "agtools.platform_layout.v1".to_string();
        let preferences = UiPreferences {
            values: BTreeMap::from([(key.clone(), r#"{"_layoutUpdatedAt":200}"#.into())]),
            ..UiPreferences::default()
        };
        assert!(super::check_layout_revision(
            &preferences,
            &BTreeMap::from([(key.clone(), r#"{"_layoutUpdatedAt":100}"#.into())])
        )
        .is_err());
        assert!(super::check_layout_revision(
            &preferences,
            &BTreeMap::from([(key, r#"{"_layoutUpdatedAt":201}"#.into())])
        )
        .is_ok());
    }

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
    fn empty_or_invalid_file_errors_without_overwriting_data() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cockpit-ui-preferences-bad-{stamp}.json"));
        std::fs::write(&path, "   ").expect("write empty");
        assert!(read_preferences_from_path(&path).is_err());

        std::fs::write(&path, "{not-json").expect("write invalid");
        assert!(read_preferences_from_path(&path).is_err());
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
            BTreeMap::from([("agtools.platform_layout.v1".to_string(), "next".to_string())]),
        ));
        assert_eq!(
            preferences.values.get("agtools.platform_layout.v1"),
            Some(&"next".to_string())
        );
        assert_eq!(preferences.values.get("keep"), Some(&"yes".to_string()));
        assert!(!super::apply_values(
            &mut preferences,
            BTreeMap::from([("agtools.platform_layout.v1".to_string(), "next".to_string())]),
        ));
    }
    #[test]
    fn webview_import_fills_missing_keys_and_revisions_prevent_lost_updates() {
        let mut snapshot = UiPreferences::default();
        assert!(super::migrate_missing(
            &mut snapshot,
            BTreeMap::from([("privacy".into(), "1".into())])
        ));
        assert_eq!(snapshot.revisions["privacy"], 1);
        assert!(super::update_value(&mut snapshot, "privacy".into(), "0".into(), 1).unwrap());
        assert!(!super::migrate_missing(
            &mut snapshot,
            BTreeMap::from([("privacy".into(), "1".into())])
        ));
        assert!(super::update_value(&mut snapshot, "privacy".into(), "1".into(), 1).is_err());
        assert_eq!(snapshot.values["privacy"], "0");
        assert_eq!(snapshot.revisions["privacy"], 2);
        assert!(super::update_value(&mut snapshot, "other".into(), "value".into(), 0).unwrap());
    }
}
