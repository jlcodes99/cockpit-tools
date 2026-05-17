//! 离线配置同步模块
//!
//! 用于在 WebSocket 离线时，通过共享文件同步配置
//!
//! 设计说明:
//! - 在线时: 通过 WebSocket 实时同步，不写入共享文件
//! - 离线时: 写入共享文件，等对方启动时读取合并
//! - 启动时: 读取共享文件，与本地配置比较时间戳后合并
//!
//! 可扩展性:
//! - 目前支持 language 配置
//! - 可扩展支持 theme、accounts 等其他配置

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use super::config::get_shared_dir;

/// 同步配置文件名
const SYNC_SETTINGS_FILE: &str = "sync_settings.json";

/// 配置来源
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Plugin,
    Desktop,
}

/// 单个配置项结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettingValue {
    pub value: String,
    pub updated_at: i64,
    pub updated_by: ConfigSource,
}

/// 同步配置文件结构
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<SyncSettingValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<SyncSettingValue>,
    // 可扩展其他配置项...
}

/// 获取同步配置文件路径
fn get_sync_settings_path() -> PathBuf {
    get_shared_dir().join(SYNC_SETTINGS_FILE)
}

/// 读取同步配置文件
/// 如果文件不存在或损坏，返回空配置
pub fn read_sync_settings() -> SyncSettings {
    let path = get_sync_settings_path();

    if !path.exists() {
        return SyncSettings::default();
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            crate::modules::logger::log_warn(&format!(
                "[SyncSettings] 解析配置失败, 返回空配置: {}",
                e
            ));
            SyncSettings::default()
        }),
        Err(e) => {
            crate::modules::logger::log_warn(&format!(
                "[SyncSettings] 读取配置失败, 返回空配置: {}",
                e
            ));
            SyncSettings::default()
        }
    }
}

/// 保存同步配置文件
fn save_sync_settings(settings: &SyncSettings) -> Result<(), String> {
    let path = get_sync_settings_path();

    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let content =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化失败: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    Ok(())
}

/// 写入单个同步配置项
/// 用于离线时保存配置，等对方启动时读取
pub fn write_sync_setting(key: &str, value: &str) {
    let mut settings = read_sync_settings();

    let setting_value = SyncSettingValue {
        value: value.to_string(),
        updated_at: chrono::Utc::now().timestamp_millis(),
        updated_by: ConfigSource::Desktop,
    };

    match key {
        "language" => settings.language = Some(setting_value),
        "theme" => settings.theme = Some(setting_value),
        _ => {
            crate::modules::logger::log_warn(&format!("[SyncSettings] 未知配置项: {}", key));
            return;
        }
    }

    if let Err(e) = save_sync_settings(&settings) {
        crate::modules::logger::log_error(&format!("[SyncSettings] 写入配置失败: {}", e));
    } else {
        crate::modules::logger::log_info(&format!(
            "[SyncSettings] 写入离线配置: {} = {}",
            key, value
        ));
    }
}

/// 清除单个同步配置项
/// 用于已同步后清理，避免下次重复同步
pub fn clear_sync_setting(key: &str) {
    let mut settings = read_sync_settings();

    let had_value = match key {
        "language" => settings.language.take().is_some(),
        "theme" => settings.theme.take().is_some(),
        _ => false,
    };

    if had_value {
        if let Err(e) = save_sync_settings(&settings) {
            crate::modules::logger::log_error(&format!("[SyncSettings] 清除配置失败: {}", e));
        } else {
            crate::modules::logger::log_info(&format!("[SyncSettings] 清除已同步配置: {}", key));
        }
    }
}

/// 获取单个同步配置项
pub fn get_sync_setting(key: &str) -> Option<SyncSettingValue> {
    let settings = read_sync_settings();

    match key {
        "language" => settings.language,
        "theme" => settings.theme,
        _ => None,
    }
}

/// 比较并合并配置（启动时调用）
/// 返回是否需要更新本地配置
///
/// # Arguments
/// * `key` - 配置项键名
/// * `local_value` - 本地当前值
/// * `local_updated_at` - 本地更新时间（如果有的话）
///
/// # Returns
/// 如果需要更新本地，返回新值；否则返回 None
pub fn merge_setting_on_startup(
    key: &str,
    local_value: &str,
    local_updated_at: Option<i64>,
) -> Option<String> {
    let sync_setting = get_sync_setting(key)?;

    // 如果共享文件的值和本地相同，不需要更新
    if sync_setting.value == local_value {
        // 清除共享文件中的配置（已一致）
        clear_sync_setting(key);
        return None;
    }

    // 如果共享文件更新时间更晚，或者本地没有更新时间记录，使用共享文件的值
    if local_updated_at.is_none() || sync_setting.updated_at > local_updated_at.unwrap_or(0) {
        crate::modules::logger::log_info(&format!(
            "[SyncSettings] 合并配置 {}: 共享文件 \"{}\" > 本地 \"{}\"",
            key, sync_setting.value, local_value
        ));
        // 清除共享文件中的配置（已合并）
        clear_sync_setting(key);
        return Some(sync_setting.value);
    }

    // 本地更新时间更晚，不需要更新本地
    None
}
