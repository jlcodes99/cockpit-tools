use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde_json::{json, Map, Value};

pub(super) const FORCE_PROFILE_TOKEN: &str = "AGY_CLI_FORCE_FILE_TOKEN_STORAGE";

pub(super) fn isolate_personal_account(entry: &mut Map<String, Value>) -> bool {
    if entry.get("driver").and_then(Value::as_str) != Some("antigravity")
        || entry
            .get("config")
            .and_then(|config| config.get("authMethod"))
            .and_then(Value::as_str)
            != Some("oauth-personal")
    {
        return false;
    }
    let Some(environment) = entry
        .entry("environment")
        .or_insert_with(|| json!([]))
        .as_array_mut()
    else {
        return false;
    };
    let managed = |row: &Value| {
        row.get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.eq_ignore_ascii_case(FORCE_PROFILE_TOKEN))
    };
    if environment.iter().filter(|row| managed(row)).count() == 1
        && environment
            .iter()
            .any(|row| row["name"] == FORCE_PROFILE_TOKEN && row["value"] == "1")
    {
        return false;
    }
    environment.retain(|row| !managed(row));
    environment.push(json!({"name": FORCE_PROFILE_TOKEN, "value": "1"}));
    true
}

#[derive(Clone, Debug)]
pub(super) struct Identity {
    pub email: String,
    pub uses_keyring: bool,
}

fn parse_identity(log: &str) -> Option<Identity> {
    let email = log
        .lines()
        .rev()
        .filter_map(|line| {
            let (_, tail) = line.split_once("applyAuthResult: email=")?;
            if !line.contains("server_oauth.go:") {
                return None;
            }
            let email = tail
                .split_whitespace()
                .next()?
                .trim_matches(['"', ',', ';']);
            let (user, domain) = email.split_once('@')?;
            (!user.is_empty() && !domain.is_empty()).then(|| email.to_owned())
        })
        .next()?;
    Some(Identity {
        email,
        // The CLI calls both backing stores "keyring" in auth events.
        // Its storage startup message identifies the per-profile file store.
        uses_keyring: log.contains("effective: keyring")
            && !log.lines().any(|line| {
                line.contains("composite_token_storage.go:")
                    && line.contains("Using file-based token storage")
            }),
    })
}

#[derive(Default)]
struct LogIndex {
    directory_modified: Option<SystemTime>,
    latest: Option<PathBuf>,
    file_modified: Option<SystemTime>,
    identity: Option<Identity>,
}

pub(super) fn observed_identity(
    profile: &Path,
    since: Option<SystemTime>,
    pid: Option<u32>,
) -> Option<Identity> {
    let directory = profile.join("antigravity-acp").join("log");
    let find_log = || {
        let pid_suffix = pid.map(|id| format!("-{id}.log"));
        fs::read_dir(&directory)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                (name.starts_with("acp-server-")
                    && name.ends_with(".log")
                    && pid_suffix
                        .as_ref()
                        .is_none_or(|suffix| name.ends_with(suffix)))
                .then(|| entry.path())
            })
            .max()
    };
    let read = |file: &Path| {
        let mut bytes = Vec::new();
        File::open(file)
            .ok()?
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .ok()?;
        parse_identity(&String::from_utf8_lossy(&bytes))
    };
    let started_after = |metadata: &fs::Metadata| {
        since.is_none_or(|minimum| {
            metadata
                .created()
                .or_else(|_| metadata.modified())
                .is_ok_and(|ts| ts >= minimum)
        })
    };

    if pid.is_some() {
        let log = find_log()?;
        return started_after(&log.metadata().ok()?)
            .then(|| read(&log))
            .flatten();
    }

    static LOGS: OnceLock<Mutex<HashMap<PathBuf, LogIndex>>> = OnceLock::new();
    let modified = directory.metadata().ok()?.modified().ok()?;
    let mut logs = LOGS.get_or_init(Default::default).lock().ok()?;
    let index = logs.entry(profile.to_owned()).or_default();
    if index.directory_modified != Some(modified) {
        index.directory_modified = Some(modified);
        index.latest = find_log();
        index.file_modified = None;
        index.identity = None;
    }
    let log = index.latest.as_ref()?;
    let metadata = log.metadata().ok()?;
    if !started_after(&metadata) {
        return None;
    }
    let modified = metadata.modified().ok()?;
    if index.file_modified != Some(modified) {
        index.identity = read(log);
        index.file_modified = Some(modified);
    }
    index.identity.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_runtime_email_and_shared_storage_from_auth_events() {
        let auth = "I1124 22:56:44 auth.go:157] ChainedAuth: authenticated via keyring (effective: keyring)\nI1124 22:56:44 server_oauth.go:221] applyAuthResult: email=actual@example.test";
        let identity = parse_identity(auth).unwrap();
        assert_eq!(identity.email, "actual@example.test");
        assert!(identity.uses_keyring);
        let file_auth = format!("I1124 22:56:44 composite_token_storage.go:207] Using file-based token storage because AGY_CLI_FORCE_FILE_TOKEN_STORAGE is set\n{auth}");
        assert!(!parse_identity(&file_auth).unwrap().uses_keyring);
        assert!(parse_identity("user says applyAuthResult: email=fake@example.test").is_none());
    }

    #[test]
    fn isolates_personal_profiles_without_replacing_other_environment_settings() {
        let mut entry = json!({"driver":"antigravity","config":{"authMethod":"oauth-personal"},"environment":[{"name":"EXAMPLE","value":"keep"},{"name":"agy_cli_force_file_token_storage","value":"0"}]});
        assert!(isolate_personal_account(entry.as_object_mut().unwrap()));
        assert!(!isolate_personal_account(entry.as_object_mut().unwrap()));
        assert_eq!(
            entry["environment"],
            json!([{"name":"EXAMPLE","value":"keep"},{"name":FORCE_PROFILE_TOKEN,"value":"1"}])
        );
    }
}
