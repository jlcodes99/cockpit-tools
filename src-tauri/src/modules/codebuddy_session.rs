use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::modules::{logger, workbuddy_workspace_rebind};

// ---------------------------------------------------------------------------
// Data models
// ---------------------------------------------------------------------------

/// Platform selector — determines which data directories to scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CodebuddySessionPlatform {
    Cn,
    Intl,
    /// WorkBuddy stores its SQLite session index in each configuration root.
    Workbuddy,
}

/// A single session location — which instance it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddySessionLocation {
    pub instance_id: String,
    pub instance_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<bool>,
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

#[derive(Debug, Default, Serialize)]
pub struct CodebuddySessionOverview {
    pub sessions: Vec<CodebuddySessionRecord>,
    pub warnings: Vec<String>,
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
    workbuddy_local: Option<bool>,
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
        // WorkBuddy sessions live in a single `workbuddy.db` and are handled by
        // `list_sessions` directly; it never reaches the per-instance directory scan.
        CodebuddySessionPlatform::Workbuddy => {
            Err("workbuddy sessions are not scanned per-instance".to_string())
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
        CodebuddySessionPlatform::Intl => crate::modules::codebuddy_instance::load_instance_store(),
        // WorkBuddy uses a single shared database; no per-instance store exists.
        CodebuddySessionPlatform::Workbuddy => Ok(crate::models::InstanceStore::new()),
    }
}

/// Build the list of (instance_id, instance_name, user_data_dir) tuples.
/// The first entry is always the default installation.
fn collect_data_dirs(platform: &CodebuddySessionPlatform) -> Vec<(String, String, PathBuf)> {
    let mut dirs = Vec::new();

    // Default installation (id = "default")
    if let Ok(default_dir) = get_default_user_data_dir(platform) {
        dirs.push(("default".to_string(), "Default".to_string(), default_dir));
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

/// Read the WorkBuddy `sessions` table in one configuration root.
fn read_sessions_from_workbuddy_db(db_path: &Path) -> Result<Vec<RawSession>, String> {
    if !db_path
        .try_exists()
        .map_err(|error| format!("Failed to locate the WorkBuddy session database: {error}"))?
    {
        return Ok(Vec::new());
    }

    let read = || -> rusqlite::Result<Vec<RawSession>> {
        let conn =
            Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let columns = workbuddy_workspace_rebind::session_columns(&conn)?;
        let has_transport = columns.contains("transport");
        let transport = if has_transport { "transport" } else { "NULL" };
        let mut stmt = conn.prepare(&format!(
            "SELECT id, cwd, user_id, title, status, created_at, updated_at, deleted_at, is_playground, {transport}
             FROM sessions"
        ))?;
        let rows = stmt.query_map([], |row| {
            let deleted_at: Option<i64> = row.get(7)?;
            let transport: Option<String> = row.get(9)?;
            Ok(RawSession {
                conversation_id: row.get(0)?,
                cwd: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                user_id: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                title: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                status: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                // WorkBuddy stores active local sessions as NULL and active cloud sessions as -1.
                is_deleted: deleted_at.is_some_and(|value| value != -1),
                is_playground: row.get::<_, i64>(8).unwrap_or(0) != 0,
                workbuddy_local: Some(
                    deleted_at.is_none()
                        && (!has_transport || transport.as_deref() == Some("local")),
                ),
            })
        })?;
        rows.collect()
    };
    read().map_err(|error| format!("Failed to read the WorkBuddy session database: {error}"))
}

// ---------------------------------------------------------------------------
// SQLite read / write helpers
// ---------------------------------------------------------------------------

/// Read all session records from a single vscdb file.
fn read_sessions_from_db(db_path: &Path) -> Vec<RawSession> {
    if !db_path.exists() {
        return Vec::new();
    }

    let conn =
        match Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
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
        workbuddy_local: None,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// List active sessions from the default profile and configured instances.
pub fn list_sessions(
    platform: &CodebuddySessionPlatform,
    filter: &CodebuddySessionFilter,
) -> Result<CodebuddySessionOverview, String> {
    if let CodebuddySessionPlatform::Workbuddy = platform {
        let default_root = crate::modules::workbuddy_instance::get_default_workbuddy_config_dir()?;
        return Ok(
            match crate::modules::workbuddy_instance::load_instance_store() {
                Ok(store) => workbuddy_session_overview(&default_root, &store.instances, filter),
                Err(error) => {
                    let mut overview = workbuddy_session_overview(&default_root, &[], filter);
                    overview
                        .warnings
                        .push(format!("Failed to read WorkBuddy instances: {error}"));
                    overview
                }
            },
        );
    }

    let sources =
        collect_data_dirs(platform)
            .into_iter()
            .map(|(instance_id, instance_name, dir)| {
                let location = CodebuddySessionLocation {
                    instance_id,
                    instance_name,
                    local: None,
                };
                (location, read_sessions_from_db(&sessions_db_path(&dir)))
            });
    Ok(CodebuddySessionOverview {
        sessions: aggregate_sessions(sources, filter),
        warnings: Vec::new(),
    })
}

pub fn rebind_workbuddy_workspace(
    instance_id: &str,
    old_cwd: &str,
    new_cwd: &str,
) -> Result<usize, String> {
    use crate::modules::{process, workbuddy_instance};
    let (root, runtime, last_pid) = if instance_id == "workbuddy" {
        (
            workbuddy_instance::get_default_workbuddy_config_dir()?,
            None,
            workbuddy_instance::load_default_settings()
                .ok()
                .and_then(|settings| settings.last_pid),
        )
    } else {
        let store = workbuddy_instance::load_instance_store()?;
        let profile = store
            .instances
            .iter()
            .find(|profile| profile.id == instance_id)
            .ok_or_else(|| "WorkBuddy instance no longer exists.".to_owned())?;
        let (root, electron) =
            workbuddy_instance::resolve_workbuddy_runtime_dirs(&profile.user_data_dir)?;
        (root, Some(electron), profile.last_pid)
    };
    workbuddy_workspace_rebind::rebind_workspace(&root, old_cwd, Path::new(new_cwd), || {
        if process::resolve_workbuddy_pid(last_pid, runtime.as_deref().and_then(Path::to_str))
            .is_some()
        {
            Err("Quit the selected WorkBuddy instance before changing its workspace.".to_owned())
        } else {
            Ok(())
        }
    })
}

fn workbuddy_session_overview(
    default_root: &Path,
    instances: &[crate::models::InstanceProfile],
    filter: &CodebuddySessionFilter,
) -> CodebuddySessionOverview {
    let mut overview = CodebuddySessionOverview::default();
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    let mut scan = |root: PathBuf, instance_id: String, instance_name: String| {
        if seen.insert(root.clone()) {
            let rows = read_sessions_from_workbuddy_db(&root.join("workbuddy.db"))
                .map_err(|error| format!("{instance_name}: {error}"))?;
            sources.push((
                CodebuddySessionLocation {
                    instance_id,
                    instance_name,
                    local: None,
                },
                rows,
            ));
        }
        Ok::<(), String>(())
    };
    if let Err(error) = scan(
        default_root.to_owned(),
        "workbuddy".into(),
        "WorkBuddy".into(),
    ) {
        overview.warnings.push(error);
    }
    for profile in instances {
        let result = crate::modules::workbuddy_instance::resolve_workbuddy_runtime_dirs(
            &profile.user_data_dir,
        )
        .map_err(|error| format!("{}: {error}", profile.name))
        .and_then(|(root, _)| scan(root, profile.id.clone(), profile.name.clone()));
        if let Err(error) = result {
            overview.warnings.push(error);
        }
    }
    overview.sessions = aggregate_sessions(sources, filter);
    overview
}

fn aggregate_sessions(
    sources: impl IntoIterator<Item = (CodebuddySessionLocation, Vec<RawSession>)>,
    filter: &CodebuddySessionFilter,
) -> Vec<CodebuddySessionRecord> {
    let mut aggregated = HashMap::<(String, Option<String>), CodebuddySessionRecord>::new();
    let keyword = filter.keyword.as_deref().unwrap_or_default().to_lowercase();
    let status_filter = filter.status.as_deref().unwrap_or_default();

    for (location, rows) in sources {
        for raw in rows {
            if raw.is_deleted || (!status_filter.is_empty() && raw.status != status_filter) {
                continue;
            }
            if !keyword.is_empty()
                && !raw.title.to_lowercase().contains(&keyword)
                && !raw.cwd.to_lowercase().contains(&keyword)
            {
                continue;
            }

            let location = CodebuddySessionLocation {
                local: raw.workbuddy_local,
                ..location.clone()
            };
            // Two instances can rebind the same conversation independently.
            // Keep the separate workspaces visible after either one changes.
            let workspace = raw.workbuddy_local.map(|_| raw.cwd.clone());
            aggregated
                .entry((raw.conversation_id.clone(), workspace))
                .and_modify(|existing| {
                    existing.updated_at = existing.updated_at.max(raw.updated_at);
                    if !existing
                        .locations
                        .iter()
                        .any(|item| item.instance_id == location.instance_id)
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
                    locations: vec![location.clone()],
                });
        }
    }

    let mut records: Vec<_> = aggregated.into_values().collect();
    records.sort_by(|a, b| b.updated_at.unwrap_or(0).cmp(&a.updated_at.unwrap_or(0)));
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workbuddy_lists_active_cloud_and_local_sessions_even_if_the_old_root_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("workbuddy.db");
        let old_root = directory.path().join("old-workspace");
        let new_root = directory.path().join("renamed-workspace");
        std::fs::create_dir(&old_root).unwrap();
        let connection = Connection::open(&db_path).unwrap();
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE sessions (id TEXT, cwd TEXT, user_id TEXT, title TEXT, status TEXT,
               created_at INTEGER, updated_at INTEGER, deleted_at INTEGER, is_playground INTEGER);
             INSERT INTO sessions VALUES ('cloud', '', 'fixture', 'cloud history', 'Completed', 1, 3, -1, 0);
             INSERT INTO sessions VALUES ('deleted', '', 'fixture', 'deleted history', 'Completed', 1, 5, 4, 0);",
        ).unwrap();
        connection.execute(
            "INSERT INTO sessions VALUES ('local', ?1, 'fixture', 'local history', 'Completed', 1, 2, NULL, 0)",
            [old_root.to_str().unwrap()],
        ).unwrap();
        drop(connection);
        std::fs::rename(&old_root, &new_root).unwrap();

        let filter = CodebuddySessionFilter {
            keyword: Some("history".into()),
            status: None,
        };
        let sessions = workbuddy_session_overview(directory.path(), &[], &filter).sessions;
        assert_eq!(
            sessions
                .iter()
                .map(|session| session.conversation_id.as_str())
                .collect::<Vec<_>>(),
            ["cloud", "local"]
        );
        assert_eq!(sessions[1].cwd, old_root.to_str().unwrap());
    }

    #[test]
    fn workbuddy_reports_database_errors_and_accepts_a_fresh_profile() {
        let directory = tempfile::tempdir().unwrap();
        let db_path = directory.path().join("workbuddy.db");
        assert!(read_sessions_from_workbuddy_db(&db_path)
            .unwrap()
            .is_empty());

        std::fs::write(&db_path, "not a SQLite database").unwrap();
        assert!(read_sessions_from_workbuddy_db(&db_path)
            .unwrap_err()
            .contains("Failed to read the WorkBuddy session database"));
    }

    #[test]
    fn workbuddy_lists_managed_profiles_and_keeps_rows_if_another_db_fails() {
        let directory = tempfile::tempdir().unwrap();
        let default = directory.path().join("default");
        let managed = directory.path().join("managed instance");
        let broken = directory.path().join("unreadable");
        for root in [&default, &managed, &broken] {
            std::fs::create_dir(root).unwrap();
        }
        for (root, ids) in [
            (&default, &["shared"][..]),
            (&managed, &["shared", "only-managed"][..]),
        ] {
            let conn = Connection::open(root.join("workbuddy.db")).unwrap();
            conn.execute_batch(
                "CREATE TABLE sessions (id TEXT, cwd TEXT, user_id TEXT,
                title TEXT, status TEXT, created_at INTEGER, updated_at INTEGER,
                deleted_at INTEGER, is_playground INTEGER);",
            )
            .unwrap();
            for id in ids {
                conn.execute("INSERT INTO sessions VALUES (?1, '/fixture', 'user', 'fixture', 'Completed', 1, 2, NULL, 0)", [id]).unwrap();
            }
        }
        std::fs::write(broken.join("workbuddy.db"), "bad database").unwrap();
        let profile = |id: &str, name: &str, dir: &Path| {
            serde_json::from_value::<crate::models::InstanceProfile>(serde_json::json!({
                "id":id, "name":name, "userDataDir":dir, "extraArgs":"", "createdAt":0,
            }))
            .unwrap()
        };
        let profiles = [
            profile("managed", "Managed", &managed.join("app")),
            profile("alias", "Alias of managed", &managed),
            profile("broken", "Unreadable profile", &broken),
            profile(
                "fresh",
                "Fresh profile",
                &directory.path().join("not-launched"),
            ),
        ];
        let filter = CodebuddySessionFilter {
            keyword: None,
            status: None,
        };
        let overview = workbuddy_session_overview(&default, &profiles, &filter);
        assert_eq!(overview.sessions.len(), 2);
        let shared = overview
            .sessions
            .iter()
            .find(|item| item.conversation_id == "shared")
            .unwrap();
        assert_eq!(
            shared
                .locations
                .iter()
                .map(|item| item.instance_id.as_str())
                .collect::<Vec<_>>(),
            ["workbuddy", "managed"]
        );
        let managed_session = overview
            .sessions
            .iter()
            .find(|item| item.conversation_id == "only-managed")
            .unwrap();
        assert_eq!(managed_session.locations.len(), 1);
        assert_eq!(managed_session.locations[0].instance_name, "Managed");
        assert_eq!(overview.warnings.len(), 1);
        assert!(overview.warnings[0].contains("Unreadable profile"));

        let conn = Connection::open(managed.join("workbuddy.db")).unwrap();
        conn.execute_batch("UPDATE sessions SET cwd='/rebound' WHERE id='shared';")
            .unwrap();
        let overview = workbuddy_session_overview(&default, &profiles, &filter);
        for (folder, id) in [("/fixture", "workbuddy"), ("/rebound", "managed")] {
            let session = overview
                .sessions
                .iter()
                .find(|session| session.conversation_id == "shared" && session.cwd == folder)
                .unwrap();
            assert_eq!(session.locations.len(), 1);
            assert_eq!(session.locations[0].instance_id, id);
            assert_eq!(session.locations[0].local, Some(true));
        }
    }
}
