//! Restore machine SSH preferences without sharing account-owned cloud state.
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
    // Keep removed identities too: any account can still hold their old data.
    known_hosts: BTreeSet<String>,
    globals: Value,
}

fn configured_hosts(state: &Value) -> BTreeSet<String> {
    state[CONNECTIONS]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["hostname"].as_str().is_some_and(|s| !s.is_empty()))
        .filter_map(|entry| entry["hostId"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string)
        .collect()
}

pub(super) fn shared_state(state: &Value, sources: &[&Value]) -> Result<SharedState, String> {
    let hosts = configured_hosts(state);
    let previous: Option<BTreeSet<String>> = state
        .get(SHARED_HOSTS)
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "CODEX_PROJECT_SSH_STATE_UNSUPPORTED".to_string())
        })
        .transpose()?;
    let mut candidates = vec![state];
    // Archives only initialize the machine state once. Afterwards absence or
    // an empty live collection means deletion, not a missing recovery source.
    if previous.is_none() {
        candidates.extend_from_slice(sources);
    }
    let globals = shared_globals(&hosts, &candidates);
    let mut known_hosts = previous.unwrap_or_default();
    known_hosts.extend(hosts);
    Ok(SharedState {
        known_hosts,
        globals,
    })
}

fn host_preferences(
    key: &str,
    hosts: &BTreeSet<String>,
    sources: &[&Value],
) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    for source in sources {
        for (host, value) in source[key].as_object().into_iter().flatten() {
            if hosts.contains(host) && (key != AUTO_CONNECT || value.is_boolean()) {
                result.entry(host.clone()).or_insert_with(|| value.clone());
            }
        }
    }
    result
}

fn ssh_bookmark(entry: &Value, hosts: &BTreeSet<String>) -> Option<(String, Value)> {
    let id = entry["id"].as_str()?;
    let host = entry["hostId"].as_str()?;
    let path = entry["remotePath"].as_str()?;
    if id.is_empty() || path.is_empty() || !hosts.contains(host) {
        return None;
    }
    // Copy bookmark fields only; unknown data may belong to the source account.
    let project = Value::Object(
        ["id", "hostId", "label", "remotePath"]
            .into_iter()
            .filter_map(|key| entry[key].as_str().map(|v| (key.into(), json!(v))))
            .collect(),
    );
    Some((id.into(), project))
}

fn shared_globals(hosts: &BTreeSet<String>, sources: &[&Value]) -> Value {
    let mut projects = BTreeMap::new();
    for source in sources {
        for entry in source[PROJECTS].as_array().into_iter().flatten() {
            if let Some((id, project)) = ssh_bookmark(entry, hosts) {
                projects.entry(id).or_insert(project);
            }
        }
    }
    let mut globals = json!({});
    for key in [AUTO_CONNECT, CWDS] {
        let entries = host_preferences(key, hosts, sources);
        if !entries.is_empty() {
            globals[key] = json!(entries);
        }
    }
    select_workspace(&mut globals, hosts, &projects, sources);
    globals[PROJECTS] = json!(projects.into_values().collect::<Vec<_>>());
    globals
}

fn select_workspace(
    globals: &mut Value,
    hosts: &BTreeSet<String>,
    projects: &BTreeMap<String, Value>,
    sources: &[&Value],
) {
    if let Some(host) = sources
        .iter()
        .filter_map(|s| s[SELECTED_HOST].as_str())
        .find(|host| *host == "local" || hosts.contains(*host))
    {
        globals[SELECTED_HOST] = json!(host);
    }
    if let Some(id) = sources
        .iter()
        .filter_map(|s| s[ACTIVE_PROJECT].as_str())
        .find(|id| {
            projects
                .get(*id)
                .is_some_and(|p| p["hostId"] == globals[SELECTED_HOST])
        })
    {
        globals[ACTIVE_PROJECT] = json!(id);
    }
}

fn owned_host(shared: &SharedState, value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|host| shared.known_hosts.contains(host))
}

fn merge_preferences(target: &mut Value, shared: &SharedState, key: &str) -> Result<(), String> {
    if target.get(key).is_none() && shared.globals.get(key).is_none() {
        return Ok(());
    }
    if target.get(key).is_some_and(|value| !value.is_object()) {
        return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
    }
    let mut entries = object(&target[key]);
    entries.retain(|host, _| !shared.known_hosts.contains(host));
    entries.extend(object(&shared.globals[key]));
    target[key] = json!(entries);
    Ok(())
}

fn merge_projects(target: &mut Value, shared: &SharedState) -> Result<bool, String> {
    if target.get(PROJECTS).is_some_and(|value| !value.is_array()) {
        return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
    }
    let mut projects = target[PROJECTS].as_array().cloned().unwrap_or_default();
    let was_ssh_project = projects
        .iter()
        .any(|entry| entry["id"] == target[ACTIVE_PROJECT] && owned_host(shared, &entry["hostId"]));
    // Replace our SSH records as a set: additions, edits and deletions all use
    // the same live source, while the target's cloud bookmarks stay untouched.
    projects.retain(|entry| !owned_host(shared, &entry["hostId"]));
    projects.extend(
        shared.globals[PROJECTS]
            .as_array()
            .into_iter()
            .flatten()
            .cloned(),
    );
    target[PROJECTS] = json!(projects);
    Ok(was_ssh_project)
}

pub(super) fn merge(target: &mut Value, shared: &SharedState) -> Result<(), String> {
    if shared.known_hosts.is_empty() {
        return Ok(());
    }
    for key in [AUTO_CONNECT, CWDS] {
        merge_preferences(target, shared, key)?;
    }
    let was_ssh_project = merge_projects(target, shared)?;
    if owned_host(shared, &target[SELECTED_HOST]) {
        target.as_object_mut().unwrap().remove(SELECTED_HOST);
    }
    if was_ssh_project {
        target.as_object_mut().unwrap().remove(ACTIVE_PROJECT);
    }
    for key in [SELECTED_HOST, ACTIVE_PROJECT] {
        if let Some(value) = shared.globals.get(key) {
            target[key] = value.clone();
        }
    }
    Ok(())
}

pub(super) fn remember_hosts(state: &mut Value, shared: &SharedState) {
    // Only initialization and identities live outside the native account scopes.
    // Live renderer data remains the sole source of machine SSH preferences.
    state[SHARED_HOSTS] = json!(shared.known_hosts);
}
