use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::modules::logger;

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Platform selector — determines which data directories to scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CodebuddySessionPlatform {
    Cn,
    Intl,
}

/// A single session location — which instance it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddySessionLocation {
    pub instance_id: String,
    pub instance_name: String,
}

/// A deduplicated session record aggregated across multiple instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddySessionRecord {
    pub conversation_id: String,
    pub title: String,
    pub cwd: String,
    pub user_id: String,
    pub status: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub is_playground: bool,
    pub locations: Vec<CodebuddySessionLocation>,
}

/// Filter parameters for listing active (non-deleted) sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebuddySessionFilter {
    /// Text keyword to match against title and cwd (case-insensitive).
    pub keyword: Option<String>,
    /// Filter by status value (e.g. "Completed", "InProgress").
    pub status: Option<String>,
}

// ---------------------------------------------------------------------------
// Raw record from vscdb
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawSession {
    conversation_id: String,
    title: String,
    cwd: String,
    user_id: String,
    status: String,
    created_at: Option<i64>,
    updated_at: Option<i64>,
    is_deleted: bool,
    is_playground: bool,
}

// ---------------------------------------------------------------------------
// Directory helpers
// ---------------------------------------------------------------------------

/// Return the default user-data dir for the given platform.
fn get_default_user_data_dir(platform: &CodebuddySessionPlatform) -> Result<PathBuf, String> {
    match platform {
        CodebuddySessionPlatform::Cn => {
            crate::modules::codebuddy_cn_instance::get_default_codebuddy_cn_user_data_dir()
        }
        CodebuddySessionPlatform::Intl => {
            crate::modules::codebuddy_instance::get_default_codebuddy_user_data_dir()
        }
    }
}

/// Load instance store for the given platform.
fn load_instance_store(
    platform: &CodebuddySessionPlatform,
) -> Result<crate::models::InstanceStore, String> {
    match platform {
        CodebuddySessionPlatform::Cn => {
            crate::modules::codebuddy_cn_instance::load_instance_store()
        }
        CodebuddySessionPlatform::Intl => {
            crate::modules::codebuddy_instance::load_instance_store()
        }
    }
}

/// Build the list of (instance_id, instance_name, user_data_dir) tuples.
/// The first entry is always the default installation.
fn collect_data_dirs(
    platform: &CodebuddySessionPlatform,
) -> Vec<(String, String, PathBuf)> {
    let mut dirs = Vec::new();

    // Default installation (id = "default")
    if let Ok(default_dir) = get_default_user_data_dir(platform) {
        dirs.push((
            "default".to_string(),
            "Default".to_string(),
            default_dir,
        ));
    }

    // Multi-instance directories
    if let Ok(store) = load_instance_store(platform) {
        for profile in &store.instances {
            let p = PathBuf::from(&profile.user_data_dir);
            if !dirs.iter().any(|(_, _, d)| d == &p) {
                dirs.push((profile.id.clone(), profile.name.clone(), p));
            }
        }
    }

    dirs
}

/// Return the path to `codebuddy-sessions.vscdb` inside a user-data dir.
fn sessions_db_path(user_data_dir: &Path) -> PathBuf {
    user_data_dir.join("codebuddy-sessions.vscdb")
}

// ---------------------------------------------------------------------------
// SQLite read / write helpers
// ---------------------------------------------------------------------------

/// Read all session records from a single vscdb file.
fn read_sessions_from_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn = match Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) {
        Ok(c) => c,
        Err(e) => {
            logger::log_warn(&format!(
                "[CodebuddySession] Failed to open {}: {}",
                db_path.display(),
                e
            ));
            return Vec::new();
        }
    };

    let mut stmt = match conn.prepare("SELECT value FROM ItemTable WHERE key LIKE 'session:%'") {
        Ok(s) => s,
        Err(e) => {
            logger::log_warn(&format!(
                "[CodebuddySession] Failed to prepare query on {}: {}",
                db_path.display(),
                e
            ));
            return Vec::new();
        }
    };

    let rows = stmt.query_map([], |row| {
        let value: String = row.get(0)?;
        Ok(value)
    });

    let mut sessions = Vec::new();
    if let Ok(rows) = rows {
        for row in rows {
            match row {
                Ok(json_str) => {
                    if let Some(s) = parse_raw_session(&json_str) {
                        sessions.push(s);
                    }
                }
                Err(e) => {
                    logger::log_warn(&format!(
                        "[CodebuddySession] Failed to read row from {}: {}",
                        db_path.display(),
                        e
                    ));
                }
            }
        }
    }

    sessions
}

fn parse_raw_session(json_str: &str) -> Option<RawSession> {
    let v: Value = serde_json::from_str(json_str).ok()?;

    let conversation_id = v.get("conversationId")?.as_str()?.to_string();
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let cwd = v
        .get("cwd")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let user_id = v
        .get("userId")
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let status = v
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let created_at = v.get("createdAt").and_then(|t| t.as_i64());
    let updated_at = v.get("updatedAt").and_then(|t| t.as_i64());
    let is_deleted = v.get("deletedAt").and_then(|t| t.as_i64()).is_some();
    let is_playground = v
        .get("isPlayground")
        .and_then(|p| p.as_bool())
        .unwrap_or(false);

    Some(RawSession {
        conversation_id,
        title,
        cwd,
        user_id,
        status,
        created_at,
        updated_at,
        is_deleted,
        is_playground,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// List active (non-deleted) sessions across all instances for the given platform,
/// with optional keyword and status filtering.
pub fn list_sessions(
    platform: &CodebuddySessionPlatform,
    filter: &CodebuddySessionFilter,
) -> Result<Vec<CodebuddySessionRecord>, String> {
    let data_dirs = collect_data_dirs(platform);
    let mut aggregated: HashMap<String, CodebuddySessionRecord> = HashMap::new();

    let keyword = filter
        .keyword
        .as_deref()
        .map(|k| k.to_lowercase())
        .unwrap_or_default();
    let status_filter = filter.status.as_deref().unwrap_or("");

    for (instance_id, instance_name, user_data_dir) in &data_dirs {
        let db_path = sessions_db_path(user_data_dir);
        let raw_sessions = read_sessions_from_db(&db_path);

        for raw in raw_sessions {
            // Always exclude deleted sessions.
            if raw.is_deleted {
                continue;
            }

            // Apply status filter
            if !status_filter.is_empty() && raw.status != status_filter {
                continue;
            }

            // Apply keyword filter (match against title and cwd)
            if !keyword.is_empty() {
                let title_lower = raw.title.to_lowercase();
                let cwd_lower = raw.cwd.to_lowercase();
                if !title_lower.contains(&keyword) && !cwd_lower.contains(&keyword) {
                    continue;
                }
            }

            let conversation_id = raw.conversation_id.clone();
            let location = CodebuddySessionLocation {
                instance_id: instance_id.clone(),
                instance_name: instance_name.clone(),
            };

            aggregated
                .entry(conversation_id.clone())
                .and_modify(|existing| {
                    if let Some(new_updated) = raw.updated_at {
                        if existing.updated_at.map_or(true, |old| new_updated > old) {
                            existing.updated_at = Some(new_updated);
                        }
                    }
                    if !existing
                        .locations
                        .iter()
                        .any(|l| l.instance_id == *instance_id)
                    {
                        existing.locations.push(location.clone());
                    }
                })
                .or_insert_with(|| CodebuddySessionRecord {
                    conversation_id: raw.conversation_id,
                    title: raw.title,
                    cwd: raw.cwd,
                    user_id: raw.user_id,
                    status: raw.status,
                    created_at: raw.created_at,
                    updated_at: raw.updated_at,
                    is_playground: raw.is_playground,
                    locations: vec![location],
                });
        }
    }

    let mut records: Vec<CodebuddySessionRecord> = aggregated.into_values().collect();
    records.sort_by(|a, b| {
        b.updated_at
            .unwrap_or(0)
            .cmp(&a.updated_at.unwrap_or(0))
    });

    Ok(records)
}
