//! Feed enabled SSH aliases to Desktop's native startup connection command.
//! The ownership journal records only entries we add, never credentials.
use super::{
    super::{check, read_bytes},
    identity,
};
use crate::modules::atomic_write;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;

const CONFIG: &str = "codex-app/config.json";
const OWNERSHIP: &str = "codex-app/.cockpit-ssh-startup.json";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NativeConfig {
    #[serde(default, deserialize_with = "optional_non_null")]
    version: Option<u64>,
    #[serde(default, deserialize_with = "optional_non_null")]
    remote_connection_max_retry_attempts: Option<u64>,
    #[serde(default, deserialize_with = "optional_non_null")]
    ssh_connect_timeout_seconds: Option<u64>,
    #[serde(default)]
    remote_connections: Vec<NativeConnection>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NativeConnection {
    ssh_alias: String,
    #[serde(default)]
    projects: Vec<NativeProject>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct NativeProject {
    remote_path: String,
    #[serde(default, deserialize_with = "optional_non_null")]
    label: Option<String>,
}

// Native Zod accepts absent optional fields, but rejects explicit JSON null.
fn optional_non_null<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    version: u32,
    aliases: BTreeSet<String>,
}

fn snapshot(path: &Path) -> Result<Option<String>, String> {
    read_bytes(path)?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| "CODEX_SSH_STARTUP_CONFIG_INVALID".into())
        })
        .transpose()
}

fn native_config(raw: Option<&str>) -> Result<Value, String> {
    let value: Value = raw
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| "CODEX_SSH_STARTUP_CONFIG_INVALID")?
        .unwrap_or_else(|| json!({"version":1,"remoteConnections":[]}));
    let config: NativeConfig = serde_json::from_value(value.clone())
        .map_err(|_| "CODEX_SSH_STARTUP_CONFIG_UNSUPPORTED")?;
    if config.version.is_some_and(|v| v != 1)
        || config.remote_connections.iter().any(|c| {
            c.ssh_alias.trim().is_empty()
                || c.projects.iter().any(|p| p.remote_path.trim().is_empty())
        })
    {
        return Err("CODEX_SSH_STARTUP_CONFIG_UNSUPPORTED".into());
    }
    Ok(value)
}

fn entry(alias: &str) -> Value {
    // Empty projects reconnect the existing host without creating bookmarks or
    // requesting directory consent; project restoration remains account_state.
    json!({"sshAlias":alias,"projects":[]})
}

fn reconcile(
    config: &Value,
    desired: &BTreeSet<String>,
    owned: &BTreeSet<String>,
) -> (Value, BTreeSet<String>) {
    let mut after = config.clone();
    let mut entries = config["remoteConnections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // An administrator may adopt a generated entry by adding projects. Once it
    // differs from our exact stub it is user-owned and must not be removed.
    entries.retain(|item| {
        !item["sshAlias"]
            .as_str()
            .is_some_and(|alias| owned.contains(alias) && *item == entry(alias))
    });
    let existing: BTreeSet<_> = entries
        .iter()
        .filter_map(|c| c["sshAlias"].as_str())
        .map(|s| s.trim().to_string())
        .collect();
    let added: BTreeSet<_> = desired.difference(&existing).cloned().collect();
    entries.extend(added.iter().map(|alias| entry(alias)));
    after["remoteConnections"] = json!(entries);
    (after, added)
}

fn replace(path: &Path, before: Option<&str>, after: Option<&str>) -> Result<(), String> {
    check()?;
    if atomic_write::replace_string_if_matches(path, before, after)? {
        Ok(())
    } else {
        Err("CODEX_SSH_STARTUP_CONFIG_CHANGED".into())
    }
}

pub(super) fn prepare(home: &Path, state: &Value) -> Result<bool, String> {
    let config_path = home.join(CONFIG);
    let journal_path = home.join(OWNERSHIP);
    let config_before = snapshot(&config_path)?;
    let journal_before = snapshot(&journal_path)?;
    let desired = identity::aliases(state)
        .into_iter()
        .map(|(_, alias)| alias)
        .collect::<BTreeSet<_>>();
    if desired.is_empty() && journal_before.is_none() {
        return Ok(false);
    }
    let owned = journal_before
        .as_deref()
        .map(serde_json::from_str::<Ownership>)
        .transpose()
        .map_err(|_| "CODEX_SSH_STARTUP_OWNERSHIP_UNSUPPORTED")?
        .unwrap_or(Ownership {
            version: 1,
            aliases: BTreeSet::new(),
        });
    if owned.version != 1 {
        return Err("CODEX_SSH_STARTUP_OWNERSHIP_UNSUPPORTED".into());
    }
    let mut before = native_config(config_before.as_deref())?;
    if config_before.is_none() {
        if let Some(retries) = state["remote-connection-max-retry-attempts"].as_u64() {
            before["remoteConnectionMaxRetryAttempts"] = json!(retries);
        }
    }
    let (after, added) = reconcile(&before, &desired, &owned.aliases);
    let config_changed = before != after || config_before.is_none();
    let next = serde_json::to_string(&Ownership {
        version: 1,
        aliases: added.clone(),
    })
    .map_err(|e| e.to_string())?;
    if !config_changed && journal_before.as_deref() == Some(next.as_str()) {
        return Ok(false);
    }
    // Write-ahead ownership makes an interrupted config write recoverable.
    let pending = serde_json::to_string(&Ownership {
        version: 1,
        aliases: owned.aliases.union(&added).cloned().collect(),
    })
    .map_err(|e| e.to_string())?;
    replace(&journal_path, journal_before.as_deref(), Some(&pending))?;
    if config_changed {
        let content = serde_json::to_string_pretty(&after).map_err(|e| e.to_string())?;
        if let Err(error) = replace(&config_path, config_before.as_deref(), Some(&content)) {
            // A newer administrator edit wins. Release only our pending claim.
            let _ = replace(&journal_path, Some(&pending), journal_before.as_deref());
            return Err(error);
        }
    }
    if pending != next {
        replace(&journal_path, Some(&pending), Some(&next))?;
    }
    Ok(true)
}
