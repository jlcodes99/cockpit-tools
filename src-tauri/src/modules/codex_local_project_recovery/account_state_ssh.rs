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
const PROJECT_ORDER: &str = "project-order";

#[path = "account_state_ssh_threads.rs"]
mod threads;

#[path = "account_state_ssh_startup.rs"]
mod startup;

pub(super) fn prepare_startup(home: &Path, state: &Value) -> bool {
    match startup::prepare(home, state) {
        Ok(changed) => changed,
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex SSH] 自动重连启动配置未更新，保留原配置: {error}"
            ));
            false
        }
    }
}

pub(super) struct SharedState {
    // Keep removed identities too: any account can still hold their old data.
    known_hosts: BTreeSet<String>,
    known_projects: BTreeSet<String>,
    project_aliases: BTreeMap<String, String>,
    globals: Value,
    memberships_verified: bool,
}

fn configured_hosts(state: &Value) -> BTreeSet<String> {
    state[CONNECTIONS]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            ["hostname", "alias"]
                .iter()
                .any(|key| entry[*key].as_str().is_some_and(|s| !s.trim().is_empty()))
        })
        .filter_map(|entry| entry["hostId"].as_str().filter(|s| !s.is_empty()))
        .map(str::to_string)
        .collect()
}

pub(super) fn shared_state(state: &Value, sources: &[&Value]) -> Result<SharedState, String> {
    let hosts = configured_hosts(state);
    let mut known_hosts: BTreeSet<String> = state
        .get(SHARED_HOSTS)
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|_| "CODEX_PROJECT_SSH_STATE_UNSUPPORTED".to_string())
        })
        .transpose()?
        .unwrap_or_default();
    // Migrate each newly recognized host independently. This also repairs the
    // empty marker produced by older builds that ignored discovered SSH aliases.
    // Initialized hosts never reload deleted projects from account archives.
    let new_hosts = hosts.difference(&known_hosts).cloned().collect();
    let archived = shared_globals(&new_hosts, sources);
    let globals = shared_globals(&hosts, &[state, &archived]);
    let projects = globals[PROJECTS]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|project| {
            project["id"]
                .as_str()
                .map(|id| (id.to_string(), project.clone()))
        })
        .collect();
    let mut all_sources = vec![state];
    all_sources.extend_from_slice(sources);
    let project_aliases = bookmark_aliases(&projects, &all_sources);
    known_hosts.extend(hosts);
    let known_projects = std::iter::once(state)
        .chain(sources.iter().copied())
        .flat_map(|source| source[PROJECTS].as_array().into_iter().flatten())
        .filter_map(|entry| ssh_bookmark(entry, &known_hosts).map(|(id, _)| id))
        .collect();
    Ok(SharedState {
        known_hosts,
        known_projects,
        project_aliases,
        globals,
        memberships_verified: false,
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
    let mut workspaces = BTreeMap::new();
    for source in sources {
        for entry in source[PROJECTS].as_array().into_iter().flatten() {
            if let Some((id, project)) = ssh_bookmark(entry, hosts) {
                // Older accounts may assign different IDs to the exact same
                // SSH bookmark. Keep the current ID and map legacy memberships.
                if let std::collections::btree_map::Entry::Vacant(workspace) =
                    workspaces.entry(workspace_key(&project))
                {
                    workspace.insert(id.clone());
                    projects.entry(id).or_insert(project);
                }
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
    let aliases = bookmark_aliases(&projects, sources);
    select_workspace(&mut globals, hosts, &projects, &aliases, sources);
    globals[PROJECT_ORDER] = json!(project_order(&projects, &aliases, sources));
    threads::collect_memberships(&mut globals, &projects, &aliases, sources);
    globals[PROJECTS] = json!(projects.into_values().collect::<Vec<_>>());
    globals
}

fn workspace_key(project: &Value) -> [String; 3] {
    [
        project["hostId"].as_str().unwrap_or_default().to_string(),
        path_key(project["remotePath"].as_str().unwrap_or_default()),
        project["label"].as_str().unwrap_or_default().to_string(),
    ]
}

fn bookmark_aliases(
    projects: &BTreeMap<String, Value>,
    sources: &[&Value],
) -> BTreeMap<String, String> {
    let by_workspace: BTreeMap<_, _> = projects
        .iter()
        .map(|(id, project)| (workspace_key(project), id))
        .collect();
    let mut aliases: BTreeMap<_, _> = projects.keys().map(|id| (id.clone(), id.clone())).collect();
    for source in sources {
        for project in source[PROJECTS].as_array().into_iter().flatten() {
            if let (Some(id), Some(canonical)) = (
                project["id"].as_str(),
                by_workspace.get(&workspace_key(project)),
            ) {
                aliases
                    .entry(id.to_string())
                    .or_insert_with(|| (*canonical).clone());
            }
        }
    }
    aliases
}

fn project_order(
    projects: &BTreeMap<String, Value>,
    aliases: &BTreeMap<String, String>,
    sources: &[&Value],
) -> Vec<String> {
    let mut seen = BTreeSet::new();
    sources
        .iter()
        .flat_map(|source| source[PROJECT_ORDER].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .filter_map(|id| aliases.get(id).map(String::as_str))
        .chain(projects.keys().map(String::as_str))
        .filter(|id| projects.contains_key(*id) && seen.insert((*id).to_string()))
        .map(str::to_string)
        .collect()
}

pub(super) fn verify_threads(home: &Path, shared: &mut SharedState) -> Result<(), String> {
    threads::verify_memberships(home, &mut shared.globals, &shared.project_aliases)?;
    shared.memberships_verified = true;
    Ok(())
}

fn select_workspace(
    globals: &mut Value,
    hosts: &BTreeSet<String>,
    projects: &BTreeMap<String, Value>,
    aliases: &BTreeMap<String, String>,
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
        .filter_map(|id| aliases.get(id))
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
    merge_project_order(target, shared)?;
    if shared.memberships_verified {
        threads::merge_memberships(target, shared)?;
    }
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

fn merge_project_order(target: &mut Value, shared: &SharedState) -> Result<(), String> {
    if target
        .get(PROJECT_ORDER)
        .is_some_and(|value| !value.is_array())
    {
        return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
    }
    let mut order = target[PROJECT_ORDER]
        .as_array()
        .cloned()
        .unwrap_or_default();
    order.retain(|id| {
        !id.as_str()
            .is_some_and(|id| shared.known_projects.contains(id))
    });
    for id in shared.globals[PROJECT_ORDER]
        .as_array()
        .into_iter()
        .flatten()
    {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    target[PROJECT_ORDER] = json!(order);
    Ok(())
}

pub(super) fn remember_hosts(state: &mut Value, shared: &SharedState) {
    // Only initialization and identities live outside the native account scopes.
    // Live renderer data remains the sole source of machine SSH preferences.
    state[SHARED_HOSTS] = json!(shared.known_hosts);
}
