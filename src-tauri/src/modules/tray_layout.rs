//! 托盘平台布局配置
//! 用于控制托盘中平台的显示、排序模式与分组层级

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

const TRAY_LAYOUT_FILE: &str = "tray_layout.json";

pub const PLATFORM_ANTIGRAVITY: &str = "antigravity";
pub const PLATFORM_CODEX: &str = "codex";
pub const PLATFORM_ZED: &str = "zed";
pub const PLATFORM_GITHUB_COPILOT: &str = "github-copilot";
pub const PLATFORM_WINDSURF: &str = "windsurf";
pub const PLATFORM_KIRO: &str = "kiro";
pub const PLATFORM_CURSOR: &str = "cursor";
pub const PLATFORM_GEMINI: &str = "gemini";
pub const PLATFORM_CODEBUDDY: &str = "codebuddy";
pub const PLATFORM_CODEBUDDY_CN: &str = "codebuddy_cn";
pub const PLATFORM_QODER: &str = "qoder";
pub const PLATFORM_TRAE: &str = "trae";
pub const PLATFORM_WORKBUDDY: &str = "workbuddy";

pub const SUPPORTED_PLATFORM_IDS: [&str; 13] = [
    PLATFORM_ANTIGRAVITY,
    PLATFORM_CODEX,
    PLATFORM_ZED,
    PLATFORM_GITHUB_COPILOT,
    PLATFORM_WINDSURF,
    PLATFORM_KIRO,
    PLATFORM_CURSOR,
    PLATFORM_GEMINI,
    PLATFORM_CODEBUDDY,
    PLATFORM_CODEBUDDY_CN,
    PLATFORM_QODER,
    PLATFORM_TRAE,
    PLATFORM_WORKBUDDY,
];

pub const SORT_MODE_AUTO: &str = "auto";
pub const SORT_MODE_MANUAL: &str = "manual";

const DEFAULT_CODEBUDDY_GROUP_ID: &str = "codebuddy-suite";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayLayoutGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub platform_ids: Vec<String>,
    pub default_platform_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayLayoutConfig {
    #[serde(default = "default_sort_mode")]
    pub sort_mode: String,
    #[serde(default = "default_order")]
    pub ordered_platform_ids: Vec<String>,
    #[serde(default = "default_tray_platforms")]
    pub tray_platform_ids: Vec<String>,
    #[serde(default = "default_ordered_entries")]
    pub ordered_entry_ids: Vec<String>,
    #[serde(default = "default_platform_groups")]
    pub platform_groups: Vec<TrayLayoutGroup>,
}

fn default_sort_mode() -> String {
    SORT_MODE_AUTO.to_string()
}

fn default_order() -> Vec<String> {
    SUPPORTED_PLATFORM_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect()
}

fn default_tray_platforms() -> Vec<String> {
    default_order()
}

fn default_platform_groups() -> Vec<TrayLayoutGroup> {
    vec![TrayLayoutGroup {
        id: DEFAULT_CODEBUDDY_GROUP_ID.to_string(),
        name: "CodeBuddy".to_string(),
        platform_ids: vec![
            PLATFORM_CODEBUDDY.to_string(),
            PLATFORM_CODEBUDDY_CN.to_string(),
            PLATFORM_WORKBUDDY.to_string(),
        ],
        default_platform_id: PLATFORM_CODEBUDDY.to_string(),
    }]
}

fn default_ordered_entries() -> Vec<String> {
    build_ordered_entries_from_platform_order(&default_order(), &default_platform_groups())
}

impl Default for TrayLayoutConfig {
    fn default() -> Self {
        Self {
            sort_mode: default_sort_mode(),
            ordered_platform_ids: default_order(),
            tray_platform_ids: default_tray_platforms(),
            ordered_entry_ids: default_ordered_entries(),
            platform_groups: default_platform_groups(),
        }
    }
}

fn get_tray_layout_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(TRAY_LAYOUT_FILE))
}

fn is_supported_platform_id(id: &str) -> bool {
    SUPPORTED_PLATFORM_IDS.contains(&id)
}

fn sanitize_platform_ids(ids: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for id in ids {
        let trimmed = id.trim();
        if trimmed.is_empty() || !is_supported_platform_id(trimmed) {
            continue;
        }
        if result.iter().any(|existing| existing == trimmed) {
            continue;
        }
        result.push(trimmed.to_string());
    }
    result
}

fn normalize_order(ids: &[String]) -> Vec<String> {
    let mut ordered = sanitize_platform_ids(ids);
    for default_id in SUPPORTED_PLATFORM_IDS {
        if !ordered.iter().any(|id| id == default_id) {
            ordered.push(default_id.to_string());
        }
    }
    ordered
}

fn contains_platform(ids: &[String], target: &str) -> bool {
    ids.iter().any(|id| id == target)
}

fn normalize_tray_platforms(
    ids: &[String],
    raw_order_has_new: &[&str],
    allow_legacy_migration: bool,
) -> Vec<String> {
    let mut sanitized = sanitize_platform_ids(ids);

    if !allow_legacy_migration {
        return sanitized;
    }

    let has_legacy_all = contains_platform(&sanitized, PLATFORM_ANTIGRAVITY)
        && contains_platform(&sanitized, PLATFORM_CODEX)
        && contains_platform(&sanitized, PLATFORM_GITHUB_COPILOT)
        && contains_platform(&sanitized, PLATFORM_WINDSURF);

    for &new_platform in &[
        PLATFORM_ZED,
        PLATFORM_KIRO,
        PLATFORM_CURSOR,
        PLATFORM_GEMINI,
        PLATFORM_CODEBUDDY,
        PLATFORM_CODEBUDDY_CN,
        PLATFORM_QODER,
        PLATFORM_TRAE,
        PLATFORM_WORKBUDDY,
    ] {
        let already_present = contains_platform(&sanitized, new_platform);
        let was_in_raw_order = raw_order_has_new.contains(&new_platform);
        let looks_like_old_default = !already_present
            && has_legacy_all
            && sanitized.len() <= SUPPORTED_PLATFORM_IDS.len().saturating_sub(1);

        if !was_in_raw_order && !already_present && looks_like_old_default {
            sanitized.push(new_platform.to_string());
        }
    }

    sanitized
}

fn normalize_sort_mode(raw: &str) -> String {
    match raw.trim() {
        SORT_MODE_MANUAL => SORT_MODE_MANUAL.to_string(),
        _ => SORT_MODE_AUTO.to_string(),
    }
}

fn normalize_group_id(raw: &str, index: usize, used: &HashSet<String>) -> String {
    let mut candidate = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if candidate.is_empty() {
        candidate = format!("group-{}", index + 1);
    }

    if !used.contains(&candidate) {
        return candidate;
    }

    let mut suffix = 2usize;
    loop {
        let next = format!("{}-{}", candidate, suffix);
        if !used.contains(&next) {
            return next;
        }
        suffix += 1;
    }
}

fn normalize_platform_groups(groups: &[TrayLayoutGroup]) -> Vec<TrayLayoutGroup> {
    let mut normalized = Vec::new();
    let mut used_platforms: HashSet<String> = HashSet::new();
    let mut used_group_ids: HashSet<String> = HashSet::new();

    for (index, group) in groups.iter().enumerate() {
        let group_id = normalize_group_id(&group.id, index, &used_group_ids);

        let mut platform_ids = Vec::new();
        for platform in sanitize_platform_ids(&group.platform_ids) {
            if used_platforms.insert(platform.clone()) {
                platform_ids.push(platform);
            }
        }

        if platform_ids.is_empty() {
            continue;
        }

        let default_platform_id = if platform_ids
            .iter()
            .any(|id| id == &group.default_platform_id)
        {
            group.default_platform_id.clone()
        } else {
            platform_ids[0].clone()
        };

        let name = if group.name.trim().is_empty() {
            default_platform_id.clone()
        } else {
            group.name.trim().to_string()
        };

        normalized.push(TrayLayoutGroup {
            id: group_id.clone(),
            name,
            platform_ids,
            default_platform_id,
        });
        used_group_ids.insert(group_id);
    }

    normalized
}

fn get_available_entry_ids(groups: &[TrayLayoutGroup]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut grouped_platforms: HashSet<String> = HashSet::new();

    for group in groups {
        entries.push(format!("group:{}", group.id));
        for platform in &group.platform_ids {
            grouped_platforms.insert(platform.clone());
        }
    }

    for platform in SUPPORTED_PLATFORM_IDS {
        if grouped_platforms.contains(platform) {
            continue;
        }
        entries.push(format!("platform:{}", platform));
    }

    entries
}

fn build_ordered_entries_from_platform_order(
    ordered_platform_ids: &[String],
    groups: &[TrayLayoutGroup],
) -> Vec<String> {
    let mut platform_to_group: HashMap<String, String> = HashMap::new();
    for group in groups {
        for platform in &group.platform_ids {
            platform_to_group.insert(platform.clone(), group.id.clone());
        }
    }

    let mut entries = Vec::new();
    let mut added_groups: HashSet<String> = HashSet::new();

    for platform in normalize_order(ordered_platform_ids) {
        if let Some(group_id) = platform_to_group.get(&platform) {
            if added_groups.insert(group_id.clone()) {
                entries.push(format!("group:{}", group_id));
            }
            continue;
        }
        entries.push(format!("platform:{}", platform));
    }

    for entry in get_available_entry_ids(groups) {
        if !entries.iter().any(|value| value == &entry) {
            entries.push(entry);
        }
    }

    entries
}

fn normalize_ordered_entries(
    raw_entries: &[String],
    ordered_platform_ids: &[String],
    groups: &[TrayLayoutGroup],
) -> Vec<String> {
    let available = get_available_entry_ids(groups);
    let available_set: HashSet<&String> = available.iter().collect();

    if raw_entries.is_empty() {
        return build_ordered_entries_from_platform_order(ordered_platform_ids, groups);
    }

    let mut result = Vec::new();
    for entry in raw_entries {
        let trimmed = entry.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        if !available_set.contains(&trimmed) {
            continue;
        }
        if result.iter().any(|value| value == &trimmed) {
            continue;
        }
        result.push(trimmed);
    }

    for fallback in build_ordered_entries_from_platform_order(ordered_platform_ids, groups) {
        if !result.iter().any(|value| value == &fallback) {
            result.push(fallback);
        }
    }

    result
}

fn normalize_config(
    config: TrayLayoutConfig,
    allow_legacy_tray_migration: bool,
) -> TrayLayoutConfig {
    let ordered_platform_ids = normalize_order(&config.ordered_platform_ids);

    let raw_order_new_platforms: Vec<&str> = [
        PLATFORM_KIRO,
        PLATFORM_CURSOR,
        PLATFORM_GEMINI,
        PLATFORM_CODEBUDDY,
        PLATFORM_CODEBUDDY_CN,
        PLATFORM_QODER,
        PLATFORM_TRAE,
        PLATFORM_WORKBUDDY,
    ]
    .iter()
    .filter(|&&p| ordered_platform_ids.iter().any(|id| id.trim() == p))
    .copied()
    .collect();

    let platform_groups = normalize_platform_groups(&config.platform_groups);
    let ordered_entry_ids = normalize_ordered_entries(
        &config.ordered_entry_ids,
        &ordered_platform_ids,
        &platform_groups,
    );

    TrayLayoutConfig {
        sort_mode: normalize_sort_mode(&config.sort_mode),
        ordered_platform_ids,
        tray_platform_ids: normalize_tray_platforms(
            &config.tray_platform_ids,
            &raw_order_new_platforms,
            allow_legacy_tray_migration,
        ),
        ordered_entry_ids,
        platform_groups,
    }
}

pub fn load_tray_layout() -> TrayLayoutConfig {
    let path = match get_tray_layout_path() {
        Ok(path) => path,
        Err(_) => return TrayLayoutConfig::default(),
    };

    if !path.exists() {
        return TrayLayoutConfig::default();
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return TrayLayoutConfig::default(),
    };

    match serde_json::from_str::<TrayLayoutConfig>(&content) {
        Ok(config) => normalize_config(config, true),
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => crate::modules::logger::log_warn(&format!(
                    "托盘布局配置解析失败，已隔离并使用默认布局: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => crate::modules::logger::log_warn(&format!(
                    "托盘布局配置解析失败，文件已不存在，使用默认布局: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => crate::modules::logger::log_warn(&format!(
                    "托盘布局配置解析失败，隔离失败，使用默认布局: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            TrayLayoutConfig::default()
        }
    }
}

pub fn save_tray_layout(
    sort_mode: String,
    ordered_platform_ids: Vec<String>,
    tray_platform_ids: Vec<String>,
    ordered_entry_ids: Option<Vec<String>>,
    platform_groups: Option<Vec<TrayLayoutGroup>>,
) -> Result<TrayLayoutConfig, String> {
    let normalized = normalize_config(
        TrayLayoutConfig {
            sort_mode,
            ordered_platform_ids,
            tray_platform_ids,
            ordered_entry_ids: ordered_entry_ids.unwrap_or_default(),
            platform_groups: platform_groups.unwrap_or_else(default_platform_groups),
        },
        false,
    );

    let path = get_tray_layout_path()?;
    let content = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("序列化托盘布局配置失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存托盘布局配置失败: {}", e))?;
    Ok(normalized)
}
