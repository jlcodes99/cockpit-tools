//! 模型分组配置模块
//!
//! 管理模型分组设置，与插件端共享同一份配置文件
//!
//! 文件路径: ~/.antigravity_cockpit/group_settings.json
//!
//! 设计说明:
//! - 两端使用同一个 modelId (API 返回的 models Key)
//! - groupMappings: modelId -> groupId
//! - groupNames: groupId -> displayName
//! - groupOrder: 分组排序（插件端可自定义，桌面端只显示前3个）

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use super::config::get_shared_dir;

/// 分组配置文件名
const GROUP_SETTINGS_FILE: &str = "group_settings.json";

const LEGACY_GROUP_NAME_G3_PRO: &str = "G3-Pro";
const LEGACY_GROUP_NAME_G3_FLASH: &str = "G3-Flash";

const GROUP_NAME_GEMINI_PRO: &str = "Gemini Pro";
const GROUP_NAME_GEMINI_FLASH: &str = "Gemini Flash";

const DEPRECATED_GROUP_ID_G3_IMAGE: &str = "g3_image";
const DEPRECATED_MODEL_ID_GEMINI_3_PRO_IMAGE: &str = "gemini-3-pro-image";

/// 配置来源
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Plugin,
    Desktop,
}

impl Default for ConfigSource {
    fn default() -> Self {
        ConfigSource::Desktop
    }
}

/// 分组配置
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupSettings {
    /// 模型 -> 分组映射 (modelId -> groupId)
    #[serde(default)]
    pub group_mappings: HashMap<String, String>,

    /// 分组名称映射 (groupId -> displayName)
    #[serde(default)]
    pub group_names: HashMap<String, String>,

    /// 分组排序
    #[serde(default)]
    pub group_order: Vec<String>,

    /// 最后更新时间戳 (毫秒)
    #[serde(default)]
    pub updated_at: i64,

    /// 最后更新来源
    #[serde(default)]
    pub updated_by: ConfigSource,
}

impl Default for GroupSettings {
    fn default() -> Self {
        // 固定的 3 个分组
        let mut group_mappings = HashMap::new();
        let mut group_names = HashMap::new();

        // Claude 4.5 分组
        group_mappings.insert(
            "claude-opus-4-5-thinking".to_string(),
            "claude_45".to_string(),
        );
        group_mappings.insert(
            "claude-opus-4-6-thinking".to_string(),
            "claude_45".to_string(),
        );
        group_mappings.insert("claude-sonnet-4-5".to_string(), "claude_45".to_string());
        group_mappings.insert("claude-sonnet-4-6".to_string(), "claude_45".to_string());
        group_mappings.insert(
            "claude-sonnet-4-5-thinking".to_string(),
            "claude_45".to_string(),
        );
        group_mappings.insert("gpt-oss-120b-medium".to_string(), "claude_45".to_string());
        group_names.insert("claude_45".to_string(), "Claude 4.5".to_string());

        // Gemini Pro 分组
        group_mappings.insert("gemini-3-pro-high".to_string(), "g3_pro".to_string());
        group_mappings.insert("gemini-3-pro-low".to_string(), "g3_pro".to_string());
        group_mappings.insert("gemini-3.1-pro-high".to_string(), "g3_pro".to_string());
        group_mappings.insert("gemini-3.1-pro-low".to_string(), "g3_pro".to_string());
        group_names.insert("g3_pro".to_string(), GROUP_NAME_GEMINI_PRO.to_string());

        // Gemini Flash 分组
        group_mappings.insert("gemini-3-flash".to_string(), "g3_flash".to_string());
        group_names.insert("g3_flash".to_string(), GROUP_NAME_GEMINI_FLASH.to_string());

        let group_order = vec![
            "claude_45".to_string(),
            "g3_pro".to_string(),
            "g3_flash".to_string(),
        ];

        Self {
            group_mappings,
            group_names,
            group_order,
            updated_at: 0,
            updated_by: ConfigSource::Desktop,
        }
    }
}

impl GroupSettings {
    /// 获取分组显示名称
    pub fn get_group_name(&self, group_id: &str) -> String {
        self.group_names
            .get(group_id)
            .cloned()
            .unwrap_or_else(|| group_id.to_string())
    }

    /// 获取模型所属分组
    /// 获取排序后的分组列表（最多返回指定数量）
    pub fn get_ordered_groups(&self, max_count: Option<usize>) -> Vec<String> {
        let mut groups = self.group_order.clone();

        // 添加在 mappings 中但不在 order 中的分组
        for group_id in self.group_mappings.values() {
            if !groups.contains(group_id) {
                groups.push(group_id.clone());
            }
        }

        // 去重
        let mut seen = std::collections::HashSet::new();
        groups.retain(|g| seen.insert(g.clone()));

        // 限制数量
        if let Some(max) = max_count {
            groups.truncate(max);
        }

        groups
    }

    /// 获取分组内的模型列表
    pub fn get_models_in_group(&self, group_id: &str) -> Vec<String> {
        self.group_mappings
            .iter()
            .filter(|(_, gid)| *gid == group_id)
            .map(|(mid, _)| mid.clone())
            .collect()
    }

    /// 设置模型分组
    pub fn set_model_group(&mut self, model_id: &str, group_id: &str) {
        self.group_mappings
            .insert(model_id.to_string(), group_id.to_string());
        self.touch();
    }

    /// 移除模型的分组
    pub fn remove_model_group(&mut self, model_id: &str) {
        self.group_mappings.remove(model_id);
        self.touch();
    }

    /// 设置分组名称
    pub fn set_group_name(&mut self, group_id: &str, name: &str) {
        self.group_names
            .insert(group_id.to_string(), name.to_string());
        self.touch();
    }

    /// 更新分组排序
    pub fn set_group_order(&mut self, order: Vec<String>) {
        self.group_order = order;
        self.touch();
    }

    /// 删除分组（移除该分组的所有模型映射）
    pub fn delete_group(&mut self, group_id: &str) {
        self.group_mappings.retain(|_, gid| gid != group_id);
        self.group_names.remove(group_id);
        self.group_order.retain(|g| g != group_id);
        self.touch();
    }

    /// 更新时间戳和来源
    fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().timestamp_millis();
        self.updated_by = ConfigSource::Desktop;
    }
}

fn migrate_legacy_group_names(settings: &mut GroupSettings) {
    let mut migrate_name = |group_id: &str, legacy_name: &str, new_name: &str| {
        if let Some(current_name) = settings.group_names.get(group_id) {
            if current_name == legacy_name {
                settings
                    .group_names
                    .insert(group_id.to_string(), new_name.to_string());
            }
        }
    };

    migrate_name("g3_pro", LEGACY_GROUP_NAME_G3_PRO, GROUP_NAME_GEMINI_PRO);
    migrate_name(
        "g3_flash",
        LEGACY_GROUP_NAME_G3_FLASH,
        GROUP_NAME_GEMINI_FLASH,
    );
}

fn remove_deprecated_groups(settings: &mut GroupSettings) {
    settings.group_names.remove(DEPRECATED_GROUP_ID_G3_IMAGE);
    settings
        .group_order
        .retain(|group_id| group_id != DEPRECATED_GROUP_ID_G3_IMAGE);
    settings.group_mappings.retain(|model_id, group_id| {
        group_id != DEPRECATED_GROUP_ID_G3_IMAGE
            && model_id != DEPRECATED_MODEL_ID_GEMINI_3_PRO_IMAGE
    });
}

/// 获取分组配置文件路径
fn get_group_settings_path() -> PathBuf {
    get_shared_dir().join(GROUP_SETTINGS_FILE)
}

/// 读取分组配置
pub fn load_group_settings() -> GroupSettings {
    let path = get_group_settings_path();
    let default_settings = GroupSettings::default();

    if !path.exists() {
        return default_settings;
    }

    match fs::read_to_string(&path) {
        Ok(content) => {
            let mut settings: GroupSettings = serde_json::from_str(&content).unwrap_or_else(|e| {
                crate::modules::logger::log_warn(&format!(
                    "[GroupSettings] 解析配置失败, 返回默认配置: {}",
                    e
                ));
                default_settings.clone()
            });
            let original_settings = settings.clone();

            // 兼容增量升级：补齐缺失的默认映射/名称/排序，保留用户自定义配置
            for (model_id, group_id) in &default_settings.group_mappings {
                settings
                    .group_mappings
                    .entry(model_id.clone())
                    .or_insert_with(|| group_id.clone());
            }

            for (group_id, group_name) in &default_settings.group_names {
                settings
                    .group_names
                    .entry(group_id.clone())
                    .or_insert_with(|| group_name.clone());
            }

            if settings.group_order.is_empty() {
                settings.group_order = default_settings.group_order.clone();
            } else {
                for group_id in &default_settings.group_order {
                    if !settings.group_order.contains(group_id) {
                        settings.group_order.push(group_id.clone());
                    }
                }
            }

            migrate_legacy_group_names(&mut settings);
            remove_deprecated_groups(&mut settings);
            if settings != original_settings {
                if let Err(e) = save_group_settings(&settings) {
                    crate::modules::logger::log_warn(&format!(
                        "[GroupSettings] 迁移后写回配置失败: {}",
                        e
                    ));
                }
            }

            settings
        }
        Err(e) => {
            crate::modules::logger::log_warn(&format!(
                "[GroupSettings] 读取配置失败, 返回默认配置: {}",
                e
            ));
            default_settings
        }
    }
}

/// 保存分组配置
pub fn save_group_settings(settings: &GroupSettings) -> Result<(), String> {
    let path = get_group_settings_path();

    // 确保目录存在
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }

    let content =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化失败: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("写入文件失败: {}", e))?;

    crate::modules::logger::log_info(&format!(
        "[GroupSettings] 保存配置成功: {} 个映射, {} 个分组",
        settings.group_mappings.len(),
        settings.group_order.len()
    ));

    Ok(())
}

/// 更新分组配置
pub fn update_group_settings(settings: GroupSettings) -> Result<(), String> {
    save_group_settings(&settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_settings_default() {
        let settings = GroupSettings::default();
        assert_eq!(settings.updated_at, 0);
        assert_eq!(settings.updated_by, ConfigSource::Desktop);

        assert_eq!(
            settings.group_order,
            vec![
                "claude_45".to_string(),
                "g3_pro".to_string(),
                "g3_flash".to_string(),
            ]
        );

        assert_eq!(
            settings.group_names.get("claude_45").map(String::as_str),
            Some("Claude 4.5")
        );
        assert_eq!(
            settings.group_names.get("g3_pro").map(String::as_str),
            Some(GROUP_NAME_GEMINI_PRO)
        );
        assert_eq!(
            settings.group_names.get("g3_flash").map(String::as_str),
            Some(GROUP_NAME_GEMINI_FLASH)
        );

        assert_eq!(
            settings
                .group_mappings
                .get("claude-sonnet-4-5")
                .map(String::as_str),
            Some("claude_45")
        );
        assert_eq!(
            settings
                .group_mappings
                .get("gemini-3-pro-high")
                .map(String::as_str),
            Some("g3_pro")
        );
        assert_eq!(
            settings
                .group_mappings
                .get("gemini-3-flash")
                .map(String::as_str),
            Some("g3_flash")
        );
    }

    #[test]
    fn test_remove_deprecated_groups() {
        let mut settings = GroupSettings::default();
        settings
            .group_names
            .insert("g3_image".to_string(), "Gemini Image".to_string());
        settings.group_order.push("g3_image".to_string());
        settings
            .group_mappings
            .insert("gemini-3-pro-image".to_string(), "g3_image".to_string());

        remove_deprecated_groups(&mut settings);

        assert!(!settings.group_names.contains_key("g3_image"));
        assert!(!settings
            .group_order
            .iter()
            .any(|group_id| group_id == "g3_image"));
        assert!(!settings.group_mappings.contains_key("gemini-3-pro-image"));
    }

    #[test]
    fn test_set_model_group() {
        let mut settings = GroupSettings::default();
        settings.set_model_group("claude-sonnet-4-5", "claude");

        assert_eq!(
            settings.group_mappings.get("claude-sonnet-4-5"),
            Some(&"claude".to_string())
        );
    }

    #[test]
    fn test_get_ordered_groups_with_limit() {
        let mut settings = GroupSettings::default();
        settings.group_order = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ];

        let groups = settings.get_ordered_groups(Some(4));
        assert_eq!(groups.len(), 4);
        assert_eq!(groups, vec!["a", "b", "c", "d"]);
    }
}
