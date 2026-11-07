//! Local project registry recovery. No credentials, providers or rollout contents.
//! Quick repair uses bounded current metadata; deep repair may inspect snapshots.
//! Neither path reads credentials, providers or rollout contents.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const STATE: &str = ".codex-global-state.json";
const MAP: &str = "app-server-project-id-by-legacy-project-id-by-host";
const ASSIGN: &str = "thread-project-assignments";
const MIGRATION: &str = "app-server-projects-migration-by-host";
const HOSTS: &str = "thread-project-membership-host-ids";
const JOURNAL: &str = ".cockpit-local-project-recovery.json";
const MAX_STATE_BYTES: u64 = 20_000_000;
const QUICK_MAX_PROJECTS: usize = 256;
const QUICK_MAX_THREADS: usize = 2048;
const QUICK_MAX_CATALOG_ROWS: usize = 4096;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub projects: usize,
    pub assignments: usize,
    pub thread_rows: usize,
    pub catalog_rows: usize,
    pub skipped_projects: usize,
    pub ambiguous_threads: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RowPatch {
    id: String,
    host: Option<String>,
    before_project: Option<String>,
    after_project: String,
    before_cwd: Option<String>,
    after_cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabasePatch {
    relative: PathBuf,
    table: String,
    rows: Vec<RowPatch>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Plan {
    #[serde(default)]
    quick: bool,
    home: PathBuf,
    before_bytes: Option<Vec<u8>>,
    before_backup: Option<Vec<u8>>,
    before: Value,
    after: Value,
    // The project registry must still be identical when the transaction begins.
    project_fingerprint: String,
    primary: PathBuf,
    databases: Vec<DatabasePatch>,
    pub counts: Counts,
}

impl Plan {
    pub fn changed(&self) -> bool {
        self.before != self.after || self.databases.iter().any(|db| !db.rows.is_empty())
    }

    pub fn state_changes(&self) -> usize {
        self.after
            .as_object()
            .unwrap()
            .iter()
            .filter(|(key, value)| self.before.get(*key) != Some(*value))
            .count()
    }
}

fn check() -> Result<(), String> {
    super::check_launch_visibility_budget("local_projects")
}

fn open(path: &Path, write: bool) -> Result<Connection, String> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = Connection::open_with_flags(path, flags).map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_millis(150))
        .map_err(|e| e.to_string())?;
    super::configure_launch_visibility_connection(&db)?;
    Ok(db)
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_STATE_BYTES => Err("CODEX_PROJECT_STATE_TOO_LARGE".into()),
        Ok(_) => fs::read(path).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn decode(bytes: Option<&[u8]>) -> Result<Value, String> {
    let Some(bytes) = bytes else {
        return Ok(json!({}));
    };
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if !value.is_object() {
        return Err("CODEX_PROJECT_STATE_INVALID".into());
    }
    Ok(value)
}

// Windows comparisons are lexical and case-insensitive, including extended/UNC
// aliases. POSIX paths retain case, slash and backslash semantics on all hosts.
fn path_key(raw: &str) -> String {
    let cleaned = clean_cwd(raw);
    if super::is_windows_workspace_path(&cleaned) {
        let mut parts = Vec::new();
        let windows = cleaned.replace('/', "\\").to_lowercase();
        let unc = windows.starts_with("\\\\");
        for part in windows.split('\\').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." && parts.len() > if unc { 2 } else { 1 } {
                parts.pop();
            } else {
                parts.push(part);
            }
        }
        let mut key = format!("{}{}", if unc { "\\\\" } else { "" }, parts.join("\\"));
        if parts.len() == 1
            && windows.as_bytes().get(2) == Some(&b'\\')
            && windows.as_bytes().get(1) == Some(&b':')
        {
            key.push('\\');
        }
        key
    } else {
        let mut parts = Vec::new();
        for part in cleaned.split('/').filter(|p| !p.is_empty() && *p != ".") {
            if part == ".." && parts.last().is_some_and(|p| *p != "..") {
                parts.pop();
            } else {
                parts.push(part);
            }
        }
        format!(
            "{}{}",
            if cleaned.starts_with('/') { "/" } else { "" },
            parts.join("/")
        )
    }
}

fn clean_cwd(raw: &str) -> String {
    if super::is_windows_workspace_path(raw) || raw.starts_with(r"\\?\") {
        super::to_desktop_workspace_path(raw).unwrap_or_default()
    } else {
        raw.to_string()
    }
}

fn object(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

fn host_key(state: &Value, home: &Path) -> String {
    let path = home.to_string_lossy();
    object(&state[MAP])
        .into_keys()
        .find(|host| {
            host.strip_prefix("local:")
                .is_some_and(|p| path_key(p) == path_key(&path))
        })
        .unwrap_or_else(|| format!("local:{path}"))
}

fn ids(value: &Value) -> HashSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn set_map(state: &mut Value, key: &str, entries: BTreeMap<String, Value>) {
    if !entries.is_empty() || state.get(key).is_some() {
        state[key] = serde_json::to_value(entries).unwrap();
    }
}

pub(super) mod account_state;

include!("plan.rs");
include!("storage.rs");

#[cfg(test)]
mod tests;
