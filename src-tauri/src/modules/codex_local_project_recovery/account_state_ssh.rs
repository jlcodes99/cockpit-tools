//! SSH hosts are machine configuration, unlike account-owned cloud connections.
//! Only share workspace preferences for hosts still in the native SSH registry.
use super::*;
use std::collections::BTreeSet;

const CONNECTIONS: &str = "codex-managed-remote-connections";
const AUTO_CONNECT: &str = "remote-connection-auto-connect-by-host-id";
const SELECTED_HOST: &str = "selected-remote-host-id";
const PROJECTS: &str = "remote-projects";
const ACTIVE_PROJECT: &str = "active-remote-project-id";
const CWDS: &str = "remote-cwds-by-host-and-workspace";
const SHARED_HOSTS: &str = "cockpit-shared-ssh-hosts-v1";

pub(super) struct SharedState {
    // Includes previously configured hosts so removing a host also removes its
    // stale preferences and bookmarks from an old account's renderer snapshot.
    known_hosts: BTreeSet<String>,
    globals: Value,
}

pub(super) fn shared_state(state: &Value, sources: &[&Value]) -> Result<SharedState, String> {
    let hosts: BTreeSet<String> = state[CONNECTIONS]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["hostname"].as_str().is_some_and(|s| !s.is_empty()))
        .filter_map(|entry| entry["hostId"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string)
        .collect();
    let previous: Option<BTreeSet<String>> = state
        .get(SHARED_HOSTS)
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "CODEX_PROJECT_SSH_STATE_UNSUPPORTED".to_string())
        })
        .transpose()?;
    let mut candidates = vec![state];
    // Archives are a one-time migration source. Once initialized, an absent or
    // empty live preference means deletion, never "restore it from an archive".
    if previous.is_none() {
        candidates.extend_from_slice(sources);
    }
    let mut view = json!({});
    let mut auto_connect = BTreeMap::new();
    let mut projects = BTreeMap::new();
    let mut cwds = BTreeMap::new();
    // The live state is newer than parked account snapshots. In particular,
    // preserve an explicit false instead of re-enabling a host from old state.
    for source in &candidates {
        for host in &hosts {
            if let Some(enabled) = source[AUTO_CONNECT][host].as_bool() {
                auto_connect.entry(host).or_insert(enabled);
            }
            if let Some(paths) = source[CWDS].get(host) {
                cwds.entry(host).or_insert_with(|| paths.clone());
            }
        }
        if view.get(SELECTED_HOST).is_none()
            && source[SELECTED_HOST]
                .as_str()
                .is_some_and(|host| host == "local" || hosts.contains(host))
        {
            view[SELECTED_HOST] = source[SELECTED_HOST].clone();
        }
        for entry in source[PROJECTS].as_array().into_iter().flatten() {
            let (Some(id), Some(host), Some(path)) = (
                entry["id"].as_str(),
                entry["hostId"].as_str(),
                entry["remotePath"].as_str(),
            ) else {
                continue;
            };
            if id.is_empty() || path.is_empty() || !hosts.contains(host) {
                continue;
            }
            // Do not move unknown account data or credentials with a bookmark.
            projects.entry(id).or_insert_with(|| {
                Value::Object(
                    ["id", "hostId", "label", "remotePath"]
                        .into_iter()
                        .filter_map(|key| {
                            entry[key].as_str().map(|value| (key.into(), json!(value)))
                        })
                        .collect(),
                )
            });
        }
    }
    for source in &candidates {
        if let Some(id) = source[ACTIVE_PROJECT].as_str() {
            if projects
                .get(id)
                .is_some_and(|project| project["hostId"] == view[SELECTED_HOST])
            {
                view[ACTIVE_PROJECT] = json!(id);
                break;
            }
        }
    }
    if !auto_connect.is_empty() {
        view[AUTO_CONNECT] = json!(auto_connect);
    }
    if !cwds.is_empty() {
        view[CWDS] = json!(cwds);
    }
    view[PROJECTS] = json!(projects.into_values().collect::<Vec<_>>());
    let mut known_hosts = previous.unwrap_or_default();
    known_hosts.extend(hosts);
    Ok(SharedState {
        known_hosts,
        globals: view,
    })
}

pub(super) fn merge(target: &mut Value, shared: &SharedState) -> Result<(), String> {
    if shared.known_hosts.is_empty() {
        return Ok(());
    }
    let view = &shared.globals;
    for key in [AUTO_CONNECT, CWDS] {
        if target.get(key).is_some() || view.get(key).is_some() {
            if target.get(key).is_some_and(|value| !value.is_object()) {
                return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
            }
            let mut entries = object(&target[key]);
            entries.retain(|host, _| !shared.known_hosts.contains(host));
            entries.extend(object(&view[key]));
            target[key] = json!(entries);
        }
    }
    let was_ssh_project = target[PROJECTS]
        .as_array()
        .into_iter()
        .flatten()
        .any(|entry| {
            entry["id"] == target[ACTIVE_PROJECT]
                && entry["hostId"]
                    .as_str()
                    .is_some_and(|host| shared.known_hosts.contains(host))
        });
    if target.get(PROJECTS).is_some() || view.get(PROJECTS).is_some() {
        if target.get(PROJECTS).is_some_and(|value| !value.is_array()) {
            return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
        }
        let mut projects = target[PROJECTS].as_array().cloned().unwrap_or_default();
        projects.retain(|entry| {
            !entry["hostId"]
                .as_str()
                .is_some_and(|host| shared.known_hosts.contains(host))
        });
        projects.extend(view[PROJECTS].as_array().into_iter().flatten().cloned());
        target[PROJECTS] = json!(projects);
    }
    if target[SELECTED_HOST]
        .as_str()
        .is_some_and(|host| shared.known_hosts.contains(host))
    {
        target.as_object_mut().unwrap().remove(SELECTED_HOST);
    }
    if was_ssh_project {
        target.as_object_mut().unwrap().remove(ACTIVE_PROJECT);
    }
    for key in [SELECTED_HOST, ACTIVE_PROJECT] {
        if let Some(value) = view.get(key) {
            target[key] = value.clone();
        }
    }
    Ok(())
}

pub(super) fn remember_hosts(state: &mut Value, shared: &SharedState) {
    // Only initialization and host identities live outside native account
    // scopes. The live renderer remains the sole source of SSH preferences.
    state[SHARED_HOSTS] = json!(shared.known_hosts);
}
