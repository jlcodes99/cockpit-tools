use crate::modules::logger;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_CHECK_INTERVAL_HOURS: u64 = 1;
const LEGACY_DEFAULT_CHECK_INTERVAL_HOURS: u64 = 24;
const LEGACY_PREVIOUS_DEFAULT_CHECK_INTERVAL_HOURS: u64 = 6;
const PENDING_UPDATE_NOTES_FILE: &str = "pending_update_notes.json";
const CHANGELOG_MARKDOWN_EN: &str = include_str!("../../../CHANGELOG.md");
const CHANGELOG_MARKDOWN_ZH: &str = include_str!("../../../CHANGELOG.zh-CN.md");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettings {
    pub auto_check: bool,
    pub last_check_time: u64,
    #[serde(default = "default_check_interval")]
    pub check_interval_hours: u64,
    #[serde(default)]
    pub auto_install: bool,
    #[serde(default)]
    pub last_run_version: String,
    #[serde(default = "default_remind_on_update")]
    pub remind_on_update: bool,
    #[serde(default)]
    pub skipped_version: String,
}

fn default_check_interval() -> u64 {
    DEFAULT_CHECK_INTERVAL_HOURS
}

fn default_remind_on_update() -> bool {
    true
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            auto_check: true,
            last_check_time: 0,
            check_interval_hours: DEFAULT_CHECK_INTERVAL_HOURS,
            auto_install: false,
            last_run_version: String::new(),
            remind_on_update: true,
            skipped_version: String::new(),
        }
    }
}

/// Version jump info returned when app was updated since last run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionJumpInfo {
    pub previous_version: String,
    pub current_version: String,
    pub release_notes: String,
    pub release_notes_zh: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseHistoryItem {
    pub version: String,
    pub date: String,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub fixed: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseHistorySection {
    Added,
    Changed,
    Fixed,
    Removed,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingUpdateNotes {
    pub version: String,
    #[serde(default)]
    pub release_notes: String,
    #[serde(default)]
    pub release_notes_zh: String,
}

/// Compare two semantic versions (e.g., "0.2.0" vs "0.1.0")
fn compare_versions(latest: &str, current: &str) -> bool {
    let parse_version =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse::<u32>().ok()).collect() };

    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for i in 0..latest_parts.len().max(current_parts.len()) {
        let latest_part = latest_parts.get(i).unwrap_or(&0);
        let current_part = current_parts.get(i).unwrap_or(&0);

        if latest_part > current_part {
            return true;
        } else if latest_part < current_part {
            return false;
        }
    }

    false
}

/// Check if enough time has passed since last check
pub fn should_check_for_updates(settings: &UpdateSettings) -> bool {
    if !settings.auto_check {
        return false;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let elapsed_hours = now.saturating_sub(settings.last_check_time) / 3600;
    let interval = if settings.check_interval_hours > 0 {
        settings.check_interval_hours
    } else {
        DEFAULT_CHECK_INTERVAL_HOURS
    };
    elapsed_hours >= interval
}

/// Get data directory for storing update settings
fn get_data_dir() -> Result<std::path::PathBuf, String> {
    dirs::data_local_dir()
        .map(|d| d.join("cockpit-tools"))
        .ok_or_else(|| "Failed to get data directory".to_string())
}

fn ensure_data_dir() -> Result<std::path::PathBuf, String> {
    let data_dir = get_data_dir()?;
    if !data_dir.exists() {
        std::fs::create_dir_all(&data_dir)
            .map_err(|e| format!("Failed to create data dir: {}", e))?;
    }
    Ok(data_dir)
}

fn pending_update_notes_path() -> Result<std::path::PathBuf, String> {
    Ok(get_data_dir()?.join(PENDING_UPDATE_NOTES_FILE))
}

fn parse_release_header(line: &str) -> Option<(String, String)> {
    if !line.starts_with("## [") {
        return None;
    }
    let body = line.strip_prefix("## [")?;
    let end_bracket = body.find(']')?;
    let version = body[..end_bracket].trim();
    if version.is_empty() {
        return None;
    }

    let tail = body[(end_bracket + 1)..].trim();
    let date = tail
        .strip_prefix('-')
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    Some((version.to_string(), date))
}

fn parse_release_section(line: &str) -> Option<ReleaseHistorySection> {
    let heading = line.strip_prefix("### ")?.trim().to_lowercase();
    match heading.as_str() {
        "added" | "新增" => Some(ReleaseHistorySection::Added),
        "changed" | "变更" => Some(ReleaseHistorySection::Changed),
        "fixed" | "修复" => Some(ReleaseHistorySection::Fixed),
        "removed" | "移除" => Some(ReleaseHistorySection::Removed),
        _ => Some(ReleaseHistorySection::Unknown),
    }
}

fn parse_release_history_markdown(markdown: &str, limit: usize) -> Vec<ReleaseHistoryItem> {
    let mut releases: Vec<ReleaseHistoryItem> = Vec::new();
    let mut current_release: Option<ReleaseHistoryItem> = None;
    let mut current_section = ReleaseHistorySection::Unknown;
    let normalized = markdown.replace("\r\n", "\n");

    for raw_line in normalized.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some((version, date)) = parse_release_header(line) {
            if let Some(release) = current_release.take() {
                releases.push(release);
            }
            current_release = Some(ReleaseHistoryItem {
                version,
                date,
                added: Vec::new(),
                changed: Vec::new(),
                fixed: Vec::new(),
                removed: Vec::new(),
            });
            current_section = ReleaseHistorySection::Unknown;
            continue;
        }

        if let Some(section) = parse_release_section(line) {
            current_section = section;
            continue;
        }

        if !line.starts_with("- ") {
            continue;
        }

        let Some(release) = current_release.as_mut() else {
            continue;
        };
        let content = line.trim_start_matches("- ").trim();
        if content.is_empty() {
            continue;
        }

        match current_section {
            ReleaseHistorySection::Added => release.added.push(content.to_string()),
            ReleaseHistorySection::Changed => release.changed.push(content.to_string()),
            ReleaseHistorySection::Fixed => release.fixed.push(content.to_string()),
            ReleaseHistorySection::Removed => release.removed.push(content.to_string()),
            ReleaseHistorySection::Unknown => {}
        }
    }

    if let Some(release) = current_release.take() {
        releases.push(release);
    }

    if releases.len() > limit {
        releases.truncate(limit);
    }

    releases
}

fn release_history_markdown_for_locale(locale: &str) -> &'static str {
    if locale.trim().to_lowercase().starts_with("zh") {
        CHANGELOG_MARKDOWN_ZH
    } else {
        CHANGELOG_MARKDOWN_EN
    }
}

pub fn get_release_history(
    locale: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ReleaseHistoryItem>, String> {
    let resolved_locale = locale.unwrap_or("en");
    let content = release_history_markdown_for_locale(resolved_locale);
    let safe_limit = limit.unwrap_or(30).max(1).min(100);
    Ok(parse_release_history_markdown(content, safe_limit))
}

fn load_pending_update_notes() -> Result<Option<PendingUpdateNotes>, String> {
    let path = pending_update_notes_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read pending update notes: {}", e))?;
    match serde_json::from_str(&content) {
        Ok(pending) => Ok(Some(pending)),
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_warn(&format!(
                    "[UpdateChecker] 待安装更新说明解析失败，已隔离并忽略: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_warn(&format!(
                    "[UpdateChecker] 待安装更新说明解析失败，文件已不存在，忽略: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_warn(&format!(
                    "[UpdateChecker] 待安装更新说明解析失败，隔离失败，忽略: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(None)
        }
    }
}

fn remove_pending_update_notes_file() {
    match pending_update_notes_path() {
        Ok(path) => {
            if path.exists() {
                if let Err(err) = std::fs::remove_file(&path) {
                    logger::log_error(&format!(
                        "删除待安装更新说明文件失败: path={}, error={}",
                        path.display(),
                        err
                    ));
                }
            }
        }
        Err(err) => {
            logger::log_error(&format!("解析待安装更新说明路径失败: {}", err));
        }
    }
}

pub fn save_pending_update_notes(
    version: String,
    release_notes: String,
    release_notes_zh: String,
) -> Result<(), String> {
    let version = version.trim().to_string();
    if version.is_empty() {
        return Err("Version cannot be empty".to_string());
    }

    let data_dir = ensure_data_dir()?;
    let path = data_dir.join(PENDING_UPDATE_NOTES_FILE);
    let payload = PendingUpdateNotes {
        version: version.clone(),
        release_notes,
        release_notes_zh,
    };
    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Failed to serialize pending update notes: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("Failed to write pending update notes: {}", e))?;

    logger::log_info(&format!(
        "已保存待安装更新说明: version={}, path={}",
        version,
        path.display()
    ));
    Ok(())
}

/// Load update settings from config file
pub fn load_update_settings() -> Result<UpdateSettings, String> {
    let data_dir = get_data_dir()?;
    let settings_path = data_dir.join("update_settings.json");

    if !settings_path.exists() {
        return Ok(UpdateSettings::default());
    }

    let content = std::fs::read_to_string(&settings_path)
        .map_err(|e| format!("Failed to read settings file: {}", e))?;
    let mut settings: UpdateSettings = match serde_json::from_str(&content) {
        Ok(settings) => settings,
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&settings_path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_warn(&format!(
                    "[UpdateChecker] 更新设置解析失败，已隔离并使用默认设置: path={}, backup={}, error={}",
                    settings_path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_warn(&format!(
                    "[UpdateChecker] 更新设置解析失败，文件已不存在，使用默认设置: path={}, error={}",
                    settings_path.display(),
                    error
                )),
                Err(backup_error) => logger::log_warn(&format!(
                    "[UpdateChecker] 更新设置解析失败，隔离失败，使用默认设置: path={}, parse_error={}, backup_error={}",
                    settings_path.display(),
                    error,
                    backup_error
                )),
            }
            return Ok(UpdateSettings::default());
        }
    };

    let mut should_persist = false;
    if settings.check_interval_hours == 0
        || settings.check_interval_hours == LEGACY_DEFAULT_CHECK_INTERVAL_HOURS
        || settings.check_interval_hours == LEGACY_PREVIOUS_DEFAULT_CHECK_INTERVAL_HOURS
    {
        settings.check_interval_hours = DEFAULT_CHECK_INTERVAL_HOURS;
        should_persist = true;
    }

    if should_persist {
        let _ = save_update_settings(&settings);
    }

    Ok(settings)
}

/// Save update settings to config file
pub fn save_update_settings(settings: &UpdateSettings) -> Result<(), String> {
    let data_dir = ensure_data_dir()?;

    let settings_path = data_dir.join("update_settings.json");

    let content = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&settings_path, &content)
        .map_err(|e| format!("Failed to write settings file: {}", e))
}

/// Update last check time
pub fn update_last_check_time() -> Result<(), String> {
    let mut settings = load_update_settings()?;
    settings.last_check_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    save_update_settings(&settings)
}

/// Check if a version jump occurred (app was updated since last run)
/// Returns Some(VersionJumpInfo) if the current version is higher than the last recorded version
pub fn check_version_jump() -> Result<Option<VersionJumpInfo>, String> {
    let mut settings = load_update_settings()?;
    let current = CURRENT_VERSION.to_string();

    // First run or same version – just record and return
    if settings.last_run_version.is_empty() || settings.last_run_version == current {
        if settings.last_run_version != current {
            settings.last_run_version = current;
            save_update_settings(&settings)?;
        }
        return Ok(None);
    }

    let previous = settings.last_run_version.clone();

    // Only trigger if current > previous (upgrade, not downgrade)
    if !compare_versions(&current, &previous) {
        settings.last_run_version = current;
        save_update_settings(&settings)?;
        return Ok(None);
    }

    let mut release_notes = String::new();
    let mut release_notes_zh = String::new();
    match load_pending_update_notes() {
        Ok(Some(pending)) => {
            if pending.version == current {
                release_notes = pending.release_notes;
                release_notes_zh = pending.release_notes_zh;
                remove_pending_update_notes_file();
            } else if compare_versions(&current, &pending.version) {
                // 当前版本已经超过缓存版本，缓存内容过期，直接清理。
                remove_pending_update_notes_file();
            }
        }
        Ok(None) => {}
        Err(err) => {
            logger::log_error(&format!("读取待安装更新说明失败: {}", err));
        }
    }

    // Update the stored version
    settings.last_run_version = current.clone();
    save_update_settings(&settings)?;

    logger::log_info(&format!("检测到版本跳跃: {} -> {}", previous, current));

    Ok(Some(VersionJumpInfo {
        previous_version: previous,
        current_version: current,
        release_notes,
        release_notes_zh,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_versions() {
        assert!(compare_versions("0.2.0", "0.1.0"));
        assert!(compare_versions("1.0.0", "0.9.9"));
        assert!(compare_versions("0.1.1", "0.1.0"));
        assert!(!compare_versions("0.1.0", "0.1.0"));
        assert!(!compare_versions("0.1.0", "0.2.0"));
    }

    #[test]
    fn test_should_check_for_updates() {
        let mut settings = UpdateSettings::default();
        assert!(should_check_for_updates(&settings));

        settings.last_check_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(!should_check_for_updates(&settings));

        settings.auto_check = false;
        assert!(!should_check_for_updates(&settings));
    }

    #[test]
    fn test_compare_versions_handles_longer_version_segments() {
        assert!(compare_versions("1.0.0.1", "1.0.0"));
        assert!(!compare_versions("1.0.0", "1.0.0.1"));
    }

    #[test]
    fn test_parse_release_history_markdown_with_english_sections() {
        let input = r#"
## [0.2.0] - 2026-04-19

### Added
- Added feature A

### Changed
- Changed behavior B

### Fixed
- Fixed bug C

### Removed
- Removed old D
"#;

        let result = parse_release_history_markdown(input, 30);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "0.2.0");
        assert_eq!(result[0].date, "2026-04-19");
        assert_eq!(result[0].added, vec!["Added feature A".to_string()]);
        assert_eq!(result[0].changed, vec!["Changed behavior B".to_string()]);
        assert_eq!(result[0].fixed, vec!["Fixed bug C".to_string()]);
        assert_eq!(result[0].removed, vec!["Removed old D".to_string()]);
    }

    #[test]
    fn test_parse_release_history_markdown_with_chinese_sections() {
        let input = r#"
## [0.1.0] - 2026-04-18

### 新增
- 新能力 A

### 变更
- 调整 B

### 修复
- 修复 C
"#;

        let result = parse_release_history_markdown(input, 30);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "0.1.0");
        assert_eq!(result[0].added, vec!["新能力 A".to_string()]);
        assert_eq!(result[0].changed, vec!["调整 B".to_string()]);
        assert_eq!(result[0].fixed, vec!["修复 C".to_string()]);
        assert!(result[0].removed.is_empty());
    }

    #[test]
    fn test_parse_release_history_markdown_respects_limit() {
        let input = r#"
## [0.3.0] - 2026-04-20
### Added
- A
## [0.2.0] - 2026-04-19
### Added
- B
"#;
        let result = parse_release_history_markdown(input, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].version, "0.3.0");
    }
}
