//! Preview-only, read-only access to an existing Cockpit identity index.
//! Never open account detail files, credentials, cookies, or Desktop profiles.
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::modules::account;

const INDEX_ENV: &str = "COCKPIT_HANDOFF_SAVED_INDEX";
const MAX_INDEX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub(super) struct SavedAccount {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub account_uuid: Option<String>,
    #[serde(default)]
    pub organization_uuid: Option<String>,
}

#[derive(Deserialize)]
struct Index {
    accounts: Vec<SavedAccount>,
}

pub(super) fn external_index() -> Result<Option<PathBuf>, String> {
    match std::env::var(INDEX_ENV) {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Ok(value) if account::is_dev_profile() && !value.trim().is_empty() => {
            let path = PathBuf::from(value);
            if !path.is_absolute() || path.file_name().is_none_or(|n| n != "claude_accounts.json") {
                return Err("ACCOUNT_INDEX_UNAVAILABLE".into());
            }
            Ok(Some(path))
        }
        _ => Err("ACCOUNT_INDEX_UNAVAILABLE".into()),
    }
}

fn read_index(path: &Path) -> Result<Vec<SavedAccount>, String> {
    // Upstream creates .cockpit_tools as a verified alias of the legacy store.
    // Remove only that prefix; linked index files and nested directories remain
    // subject to the existing no-follow checks below.
    let path = crate::modules::data_paths::without_compatibility_alias(path);
    let read = || -> Result<Vec<SavedAccount>, ()> {
        super::runtime::regular_directory(path.parent().ok_or(())?).map_err(|_| ())?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path).map_err(|_| ())?;
        let stat = file.metadata().map_err(|_| ())?;
        if !stat.is_file() || stat.len() > MAX_INDEX_BYTES {
            return Err(());
        }
        let mut bytes = Vec::new();
        file.take(MAX_INDEX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        if bytes.len() as u64 > MAX_INDEX_BYTES {
            return Err(());
        }
        let index: Index = serde_json::from_slice(&bytes).map_err(|_| ())?;
        let mut ids = HashSet::new();
        for entry in &index.accounts {
            if entry.id.is_empty() || !ids.insert(entry.id.as_str()) {
                return Err(());
            }
        }
        Ok(index.accounts)
    };
    read().map_err(|_| "ACCOUNT_INDEX_UNAVAILABLE".into())
}

pub(super) fn accounts() -> Result<Vec<SavedAccount>, String> {
    if let Some(path) = external_index()? {
        // An index identity is eligible only if the caller also verifies its
        // existing account/org Code namespace. No login capability is imported.
        return read_index(&path);
    }
    // list_accounts_checked repairs/saves account profiles and can prune stored
    // snapshots. Preview and per-write guards must not trigger those mutations
    // or open credential-bearing detail files. Read only the identity index;
    // the switcher validates the selected target's Desktop auth mode separately.
    let path = account::resolve_data_dir()?.join("claude_accounts.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        _ => read_index(&path),
    }
}

fn other_manager_running(output: &str, own_pid: u32) -> Result<bool, String> {
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let (pid, command) = line
            .trim()
            .split_once(char::is_whitespace)
            .ok_or("PROCESS_INVENTORY_UNAVAILABLE")?;
        let pid: u32 = pid.parse().map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
        if pid != own_pid
            && Path::new(command.trim())
                .file_name()
                .is_some_and(|n| n == "cockpit-tools")
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn assert_other_manager_closed() -> Result<(), String> {
    if external_index()?.is_none() {
        return Ok(());
    }
    // The installed release does not share this preview's in-process switch
    // lock. Require it to be closed before any mutation, and recheck at guards.
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,comm="])
        .output()
        .map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
    if !output.status.success() {
        return Err("PROCESS_INVENTORY_UNAVAILABLE".into());
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| "PROCESS_INVENTORY_UNAVAILABLE")?;
    if other_manager_running(text, std::process::id())? {
        return Err("EXTERNAL_COCKPIT_RUNNING".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        let dir = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("cockpit-identity-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        dir.join("claude_accounts.json")
    }

    #[test]
    fn index_needs_no_account_details_and_remains_unchanged() {
        let path = fixture();
        let data = br#"{"accounts":[{"id":"saved","email":"synthetic@example.test","account_uuid":"00000000-0000-4000-8000-000000000001","organization_uuid":"00000000-0000-4000-8000-000000000002","unknown_future_field":true}]}"#;
        std::fs::write(&path, data).unwrap();
        let accounts = read_index(&path).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "saved");
        assert_eq!(std::fs::read(&path).unwrap(), data);
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn malformed_duplicate_and_oversized_indices_fail_closed() {
        let path = fixture();
        for data in [
            "not json",
            r#"{"accounts":[{"id":"same","email":"a"},{"id":"same","email":"b"}]}"#,
        ] {
            std::fs::write(&path, data).unwrap();
            assert!(read_index(&path).is_err());
        }
        std::fs::write(&path, vec![b' '; MAX_INDEX_BYTES as usize + 1]).unwrap();
        assert!(read_index(&path).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn index_symlinks_are_rejected() {
        let path = fixture();
        let actual = path.with_file_name("actual.json");
        std::fs::write(&actual, r#"{"accounts":[]}"#).unwrap();
        std::os::unix::fs::symlink(&actual, &path).unwrap();
        assert!(read_index(&path).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn linked_index_ancestors_are_rejected() {
        let path = fixture();
        let actual = path.parent().unwrap().join("actual");
        std::fs::create_dir(&actual).unwrap();
        std::fs::write(actual.join("claude_accounts.json"), r#"{"accounts":[]}"#).unwrap();
        let linked = path.parent().unwrap().join("linked");
        std::os::unix::fs::symlink(&actual, &linked).unwrap();
        assert!(read_index(&linked.join("claude_accounts.json")).is_err());
    }

    #[test]
    fn separate_manager_is_detected_without_reading_process_arguments() {
        let text = "42 /Applications/Cockpit Tools Handoff Preview.app/Contents/MacOS/cockpit-tools\n43 /Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools\n";
        assert!(other_manager_running(text, 42).unwrap());
        assert!(!other_manager_running(text.lines().next().unwrap(), 42).unwrap());
        assert!(
            !other_manager_running("44 /Applications/Claude.app/Contents/MacOS/Claude", 42)
                .unwrap()
        );
        assert!(other_manager_running("invalid", 42).is_err());
    }
}
