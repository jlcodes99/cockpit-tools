use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use uuid::Uuid;

use crate::modules::atomic_write;

const LIST_DIRECTORY: &str = "claude-code-sessions";
const OBSERVED_FILE: &str = ".cockpit-observed-sessions.json";
type Observed = BTreeMap<String, BTreeSet<String>>;

struct Entry {
    bytes: Vec<u8>,
    modified: SystemTime,
    activity: u64,
}

fn directory_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err("Claude Code session path is not a directory".into()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn newest_entry(path: &Path) -> Result<Option<Entry>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let Ok(record) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    if !record["cliSessionId"]
        .as_str()
        .is_some_and(|id| !id.trim().is_empty())
    {
        return Ok(None);
    }
    let modified = metadata.modified().map_err(|error| error.to_string())?;
    let modified_ms = modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let activity = record["lastActivityAt"]
        .as_u64()
        .unwrap_or_default()
        .max(record["lastFocusedAt"].as_u64().unwrap_or_default())
        .max(u64::try_from(modified_ms).unwrap_or(u64::MAX));
    Ok(Some(Entry {
        bytes,
        modified,
        activity,
    }))
}

fn uuid_subdirectories(path: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut dirs = Vec::new();
    for child in fs::read_dir(path).map_err(|error| error.to_string())? {
        let child = child.map_err(|error| error.to_string())?;
        let Ok(name) = child.file_name().into_string() else {
            continue;
        };
        if Uuid::parse_str(&name).is_ok()
            && child
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
        {
            dirs.push((name, child.path()));
        }
    }
    dirs.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(dirs)
}

fn save_observed(path: &Path, observed: &Observed) -> Result<(), String> {
    let bytes = serde_json::to_vec(observed).map_err(|error| error.to_string())?;
    atomic_write::write_bytes_atomic(path, &bytes)
}

fn sync_lists(
    root: &Path,
    account_uuid: Uuid,
    organization_uuid: Uuid,
    make_backup: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<(), String> {
    if !directory_exists(root)? {
        return Ok(());
    }
    let target_account_dir = root.join(account_uuid.to_string());
    let target_dir = target_account_dir.join(organization_uuid.to_string());
    let target_key = format!("{account_uuid}/{organization_uuid}");
    let mut target_exists =
        directory_exists(&target_account_dir)? && directory_exists(&target_dir)?;
    let state_path = root.join(OBSERVED_FILE);
    let mut observed: Observed = match fs::symlink_metadata(&state_path) {
        Ok(metadata) if metadata.is_file() => {
            let bytes = fs::read(&state_path).map_err(|error| error.to_string())?;
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?
        }
        Ok(_) => return Err("Claude Code sync state is not a regular file".into()),
        Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error.to_string()),
    };
    let previously_seen = observed.get(&target_key).cloned().unwrap_or_default();
    let mut newest = BTreeMap::<String, Entry>::new();
    let mut destination = BTreeMap::new();
    let mut newly_observed = false;
    for (account, account_dir) in uuid_subdirectories(root)? {
        for (org, org_dir) in uuid_subdirectories(&account_dir)? {
            let key = format!("{account}/{org}");
            let seen = observed.entry(key.clone()).or_default();
            for file in fs::read_dir(org_dir).map_err(|error| error.to_string())? {
                let file = file.map_err(|error| error.to_string())?;
                let Ok(name) = file.file_name().into_string() else {
                    continue;
                };
                if !name.starts_with("local_") || !name.ends_with(".json") {
                    continue;
                }
                let entry = newest_entry(&file.path())?;
                if entry.is_some() {
                    newly_observed |= seen.insert(name.clone());
                }
                if key == target_key {
                    destination.insert(name, entry);
                } else if let Some(entry) = entry {
                    if newest
                        .get(&name)
                        .is_none_or(|current| entry.activity > current.activity)
                    {
                        newest.insert(name, entry);
                    }
                }
            }
        }
    }
    if newly_observed {
        save_observed(&state_path, &observed)?;
    }

    let mut backup = None;
    let mut make_backup = Some(make_backup);
    let mut copied_new_entry = false;
    let writes = (|| {
        for (name, entry) in newest {
            if let Some(existing) = destination.get(&name) {
                let Some(existing) = existing else { continue };
                if existing.activity >= entry.activity || existing.bytes == entry.bytes {
                    continue;
                }
                if let Some(make_backup) = make_backup.take() {
                    backup = Some(make_backup()?);
                }
                let backup_file = backup.as_ref().unwrap().join(&name);
                atomic_write::write_bytes_atomic(&backup_file, &existing.bytes)?;
            } else if previously_seen.contains(&name) {
                // The user removed this entry from this account and organization.
                continue;
            }
            if !target_exists {
                fs::create_dir_all(&target_dir).map_err(|error| error.to_string())?;
                target_exists = true;
            }
            let path = target_dir.join(&name);
            atomic_write::write_bytes_atomic(&path, &entry.bytes)?;
            copied_new_entry |= observed.entry(target_key.clone()).or_default().insert(name);
            // Preserve source mtime so copying cannot make an older entry win the next sync.
            fs::File::open(&path)
                .and_then(|file| file.set_modified(entry.modified))
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    })();
    let state_saved = if copied_new_entry {
        save_observed(&state_path, &observed)
    } else {
        Ok(())
    };
    writes.and(state_saved)
}

#[cfg(target_os = "macos")]
pub(super) fn sync_before_launch(
    profile: &Path,
    account: &crate::models::claude::ClaudeAccount,
) -> Result<(), String> {
    let account_uuid = account
        .account_uuid
        .as_deref()
        .and_then(|value| Uuid::parse_str(value.trim()).ok());
    let org_uuid = account
        .organization_uuid
        .as_deref()
        .and_then(|value| Uuid::parse_str(value.trim()).ok());
    let (Some(account_uuid), Some(org_uuid)) = (account_uuid, org_uuid) else {
        return Err("Claude Desktop account or organization ID unavailable".into());
    };
    sync_lists(
        &profile.join(LIST_DIRECTORY),
        account_uuid,
        org_uuid,
        || {
            crate::modules::backup_storage::behavior_backup_dir(
                "claude",
                "desktop-code-sessions",
                &Uuid::new_v4().to_string(),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn write_entry(dir: &Path, name: &str, title: &str, activity: u64, mtime: u64) -> Vec<u8> {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        let bytes = serde_json::to_vec(&json!({
            "cliSessionId": "session-id-fixture",
            "title": title,
            "lastActivityAt": activity,
        }))
        .unwrap();
        fs::write(&path, &bytes).unwrap();
        fs::File::open(path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_millis(mtime))
            .unwrap();
        bytes
    }

    #[test]
    fn claude_code_session_sync_uses_the_newest_source_and_keeps_deletions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join(LIST_DIRECTORY);
        let backup = temp.path().join("backup");
        let account = Uuid::new_v4();
        let org = Uuid::new_v4();
        let source = root.join(Uuid::new_v4().to_string()).join(org.to_string());
        let other = root.join(Uuid::new_v4().to_string()).join(org.to_string());
        let target = root.join(account.to_string()).join(org.to_string());
        let name = "local_session.json";
        write_entry(&source, name, "less recent", 20, 1_000);
        let latest = write_entry(&other, name, "best by modification", 20, 5_000);
        let new_entry = write_entry(&source, "local_new.json", "new", 120, 1_000);
        fs::write(source.join("local_incomplete.json"), b"{").unwrap();

        sync_lists(&root, account, org, || {
            panic!("no replacement should need a backup")
        })
        .unwrap();
        assert_eq!(fs::read(target.join(name)).unwrap(), latest);
        assert_eq!(fs::read(target.join("local_new.json")).unwrap(), new_entry);
        assert!(!target.join("local_incomplete.json").exists());
        assert_eq!(
            fs::metadata(target.join(name)).unwrap().modified().unwrap(),
            fs::metadata(other.join(name)).unwrap().modified().unwrap()
        );

        fs::remove_file(target.join("local_new.json")).unwrap();
        let next = write_entry(&source, name, "best by activity", 9_000, 1_000);
        sync_lists(&root, account, org, || Ok(backup.clone())).unwrap();
        assert_eq!(fs::read(backup.join(name)).unwrap(), latest);
        assert_eq!(fs::read(target.join(name)).unwrap(), next);
        assert!(!target.join("local_new.json").exists());
        assert!(source.join("local_new.json").is_file());
    }

    #[test]
    fn claude_code_session_sync_does_not_overwrite_if_backup_fails() {
        let root = tempfile::tempdir().unwrap();
        let account = Uuid::new_v4();
        let org = Uuid::new_v4();
        let source = root
            .path()
            .join(Uuid::new_v4().to_string())
            .join(org.to_string());
        let target = root.path().join(account.to_string()).join(org.to_string());
        let name = "local_existing.json";
        let original = write_entry(&target, name, "existing", 20, 1_000);
        write_entry(&source, name, "newer", 2_000, 1_000);
        assert!(sync_lists(root.path(), account, org, || Err(
            "backup directory unavailable".into()
        ))
        .is_err());
        assert_eq!(fs::read(target.join(name)).unwrap(), original);
    }
}
