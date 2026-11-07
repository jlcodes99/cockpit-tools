use super::{parse_release_header, parse_release_history_markdown, ReleaseHistoryItem};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReleaseNotes {
    pub release_notes: String,
    pub release_notes_zh: String,
}

#[derive(Serialize, Deserialize)]
struct PendingUpdateNotes {
    version: String,
    #[serde(default)]
    release_notes: String,
    #[serde(default)]
    release_notes_zh: String,
}

fn pending_notes_path() -> Result<std::path::PathBuf, String> {
    Ok(super::get_data_dir()?.join("pending_update_notes.json"))
}

pub fn save_pending_update_notes(
    version: String,
    release_notes: String,
    release_notes_zh: String,
) -> Result<(), String> {
    if version.trim().is_empty() {
        return Err("Version cannot be empty".into());
    }
    let path = super::ensure_data_dir()?.join("pending_update_notes.json");
    let pending = PendingUpdateNotes {
        version,
        release_notes,
        release_notes_zh,
    };
    let content = serde_json::to_string_pretty(&pending).map_err(|error| error.to_string())?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|error| error.to_string())
}

fn read_pending(path: &std::path::Path, version: &str) -> Option<ReleaseNotes> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            super::logger::log_warn(&format!(
                "[UpdateChecker] Cannot read local update notes: {error}"
            ));
            return None;
        }
    };
    let pending: PendingUpdateNotes = match serde_json::from_str(&contents) {
        Ok(pending) => pending,
        Err(error) => {
            super::logger::log_warn(&format!(
                "[UpdateChecker] Cannot parse local update notes: {error}"
            ));
            return None;
        }
    };
    if pending.version.trim().trim_start_matches('v') != version
        || (pending.release_notes.is_empty() && pending.release_notes_zh.is_empty())
    {
        return None;
    }
    Some(ReleaseNotes {
        release_notes: pending.release_notes,
        release_notes_zh: pending.release_notes_zh,
    })
}

const EN: &str = include_str!("../../../CHANGELOG.md");
const ZH: &str = include_str!("../../../CHANGELOG.zh-CN.md");

fn version_notes(markdown: &str, version: &str) -> String {
    let mut found = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        if let Some((entry_version, _)) = parse_release_header(line.trim()) {
            if found {
                break;
            }
            found = entry_version == version;
        } else if found {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_string()
}

pub fn get_pending_release_notes(version: &str) -> Option<ReleaseNotes> {
    let version = version.trim().trim_start_matches('v');
    pending_notes_path()
        .ok()
        .and_then(|path| read_pending(&path, version))
}

pub fn get_bundled_release_notes(version: &str) -> ReleaseNotes {
    let version = version.trim().trim_start_matches('v');
    ReleaseNotes {
        release_notes: version_notes(EN, version),
        release_notes_zh: version_notes(ZH, version),
    }
}

pub fn get_local_release_history(
    locale: Option<&str>,
    limit: Option<usize>,
) -> Vec<ReleaseHistoryItem> {
    let markdown = if locale.unwrap_or("en").to_lowercase().starts_with("zh") {
        ZH
    } else {
        EN
    };
    // Unreleased is a development entry, never a published version.
    parse_release_history_markdown(markdown, usize::MAX)
        .into_iter()
        .filter(|item| item.version != "Unreleased")
        .take(limit.unwrap_or(30).clamp(1, 100))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_notes_are_available_without_network_and_match_exact_version() {
        let en = version_notes(EN, super::super::CURRENT_VERSION);
        let zh = version_notes(ZH, super::super::CURRENT_VERSION);
        assert!(!en.is_empty());
        assert!(!zh.is_empty());
        assert_ne!(en, zh);
        assert!(version_notes(EN, "999.0.0").is_empty());
        assert_eq!(
            version_notes(
                "## [1.0.0] - today\n### Added\n- One\n## [0.9.0]\n- Old",
                "1.0.0"
            ),
            "### Added\n- One"
        );
    }

    #[test]
    fn pending_notes_are_version_scoped_and_invalid_files_do_not_hide_bundled_notes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pending_update_notes.json");
        std::fs::write(
            &path,
            r#"{"version":"1.3.76","release_notes":"English","release_notes_zh":"中文"}"#,
        )
        .unwrap();
        let pending = read_pending(&path, "1.3.76").unwrap();
        assert_eq!(pending.release_notes_zh, "中文");
        assert!(read_pending(&path, "1.3.75").is_none());
        std::fs::write(
            &path,
            r#"{"version":"1.3.76","release_notes":"","release_notes_zh":""}"#,
        )
        .unwrap();
        assert!(read_pending(&path, "1.3.76").is_none());
        std::fs::write(&path, "not json").unwrap();
        assert!(read_pending(&path, "1.3.76").is_none());
    }

    #[test]
    fn local_history_excludes_unreleased_and_honors_locale_and_limit() {
        let en = get_local_release_history(Some("en"), Some(1));
        let zh = get_local_release_history(Some("zh-CN"), Some(1));
        assert_eq!(en.len(), 1);
        assert_eq!(en[0].version, super::super::CURRENT_VERSION);
        assert_eq!(en[0].version, zh[0].version);
        assert_ne!(en[0].added, zh[0].added);
    }
}
