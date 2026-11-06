//! Bridge Codex Desktop's account-scoped renderer state at a stopped launch.
//! Only projects and user threads verified in this profile's SQLite registry are
//! shared. Remote/cloud state and account atoms stay in their original scope.
use super::*;

const SCOPES: &str = "electron-account-renderer-state-v1";
const PARKED: &str = "cockpit-local-project-account-state-v1";
const ATOMS: &str = "electron-persisted-atom-state";

// Codex Desktop 26.1103 account-scoped globals. Keep unknown globals untouched.
// This follows its native owner transition, rather than patching Desktop code.
const SCOPED_GLOBALS: &[&str] = &[
    "open-in-target-preferences",
    "worktree-repository-preferences",
    "desktop-app-beta-preferences-by-account-id",
    "active-workspace-roots",
    "electron-saved-workspace-roots",
    "electron-workspace-root-labels",
    "local-projects",
    MAP,
    MIGRATION,
    "app-server-pending-project-deletions-by-host",
    "selected-project",
    "project-writable-roots",
    "project-appearances",
    "project-files",
    "pinned-thread-ids",
    "app-server-migrated-pinned-thread-ids-by-host",
    "pinned-project-ids",
    "sidebar-project-thread-orders",
    "sidebar-thread-metadata",
    ASSIGN,
    HOSTS,
    "thread-writable-roots",
    "thread-workspace-root-hints",
    "thread-projectless-output-directories",
    "projectless-thread-ids",
    "selected-remote-host-id",
    "remote-projects",
    "active-remote-project-id",
    "project-order",
    "connection-group-order",
    "remote-cwds-by-host-and-workspace",
    "host-id-remote-control-allowed",
    "added-remote-control-env-ids",
    "remote-project-connection-backfill-completed",
    "remote-connection-auto-connect-by-host-id",
    "queued-follow-ups",
];
const SHARED_ATOMS: &[&str] = &[
    "last_completed_onboarding",
    "electron:onboarding-projectless-completed",
    "electron:bedrock-govcloud-guidance-acknowledged",
    "ctrl-tab-shortcut-notice-shown",
    "number-shortcut-notice-shown",
    "thread-sort-key",
];

fn validate_scopes(value: &Value) -> Result<(), String> {
    let valid = value.is_object()
        && value
            .get("owner")
            .is_some_and(|v| v.is_null() || v.is_string())
        && value["inactive"].as_object().is_some_and(|entries| {
            entries.len() <= 128
                && entries
                    .values()
                    .all(|entry| entry["atoms"].is_object() && entry["globals"].is_object())
        });
    if valid {
        Ok(())
    } else {
        Err("CODEX_PROJECT_ACCOUNT_STATE_UNSUPPORTED".into())
    }
}

fn scoped_globals(state: &Value) -> Value {
    Value::Object(
        SCOPED_GLOBALS
            .iter()
            .filter_map(|key| state.get(*key).map(|value| ((*key).into(), value.clone())))
            .collect(),
    )
}

fn local_view(home: &Path, sources: &[Value], projects: &[Project], threads: &[Thread]) -> Value {
    let host = format!("local:{}", home.display());
    let mut view = json!({});
    let mut recovered = BTreeMap::new();
    let mut mapping = BTreeMap::new();
    let mut aliases = HashMap::new();
    let mut server_to_legacy = HashMap::new();
    let blocked: HashSet<String> = sources
        .iter()
        .flat_map(|source| {
            object(&source["app-server-pending-project-deletions-by-host"][&host])
                .into_iter()
                .flat_map(|(id, value)| {
                    [Some(id), value["projectId"].as_str().map(str::to_string)]
                        .into_iter()
                        .flatten()
                })
        })
        .collect();
    for project in projects {
        let roots: HashSet<_> = project.roots.iter().map(|p| path_key(p)).collect();
        if cloud(&project.metadata)
            || roots.is_empty()
            || project
                .roots
                .iter()
                .any(|p| cloud_root(p) || !Path::new(p).is_dir())
            || projects.iter().any(|other| {
                other.id != project.id
                    && !cloud(&other.metadata)
                    && other
                        .roots
                        .iter()
                        .map(|p| path_key(p))
                        .collect::<HashSet<_>>()
                        == roots
            })
            || blocked.contains(&project.id)
        {
            continue;
        }
        let mut candidates = Vec::new();
        let mut ambiguous = false;
        for source in sources {
            let matches: Vec<_> = object(&source["local-projects"])
                .into_iter()
                .filter(|(id, p)| {
                    !cloud(p)
                        && root_set(&p["rootPaths"]) == roots
                        && source[MAP][&host]
                            .get(id)
                            .is_none_or(|v| v.as_str() == Some(&project.id))
                })
                .collect();
            // Do not guess between duplicate aliases inside the same account.
            if matches.len() > 1 {
                ambiguous = true;
                break;
            }
            candidates.extend(matches);
        }
        if ambiguous {
            continue;
        }
        let (legacy, entry) = candidates.first().cloned().unwrap_or_else(|| {
            (
                project.id.clone(),
                json!({"id":project.id,"name":project.name,
                "rootPaths":project.roots,"createdAt":project.created,"updatedAt":project.updated}),
            )
        });
        if blocked.contains(&legacy)
            || recovered.contains_key(&legacy)
            || sources
                .first()
                .and_then(|source| source["local-projects"].get(&legacy))
                .is_some_and(|entry| cloud(entry) || root_set(&entry["rootPaths"]) != roots)
        {
            continue;
        }
        for (alias, _) in candidates {
            aliases.insert(alias, legacy.clone());
        }
        aliases.insert(project.id.clone(), legacy.clone());
        server_to_legacy.insert(project.id.clone(), legacy.clone());
        mapping.insert(legacy.clone(), json!(project.id));
        recovered.insert(legacy, entry);
    }
    set_map(&mut view, "local-projects", recovered.clone());
    view[MAP] = json!({ &host: mapping });
    let mut order = Vec::<Value>::new();
    for source in sources {
        for id in source["project-order"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(id) = aliases.get(id) {
                if !order.contains(&json!(id)) {
                    order.push(json!(id));
                }
            }
        }
    }
    for id in recovered.keys() {
        if !order.contains(&json!(id)) {
            order.push(json!(id));
        }
    }
    view["project-order"] = json!(order);
    let mut assignments = BTreeMap::new();
    let mut hosts = BTreeMap::new();
    for thread in threads.iter().filter(|thread| thread.root) {
        // A recorded remote membership must never be turned into a local one.
        if sources.iter().any(|source| {
            source[HOSTS][&thread.id]
                .as_str()
                .is_some_and(|host| host != "local")
        }) {
            continue;
        }
        let explicit = sources
            .iter()
            .find_map(|source| source[ASSIGN].get(&thread.id));
        // Keep an explicit ungrouped choice in the preferred (target) view.
        if sources
            .first()
            .is_some_and(|source| projectless(source).contains(&thread.id))
        {
            continue;
        }
        let legacy = if let Some(server) = &thread.project {
            server_to_legacy.get(server)
        } else {
            explicit
                .filter(|assignment| assignment["projectKind"] == "local")
                .and_then(|assignment| assignment["projectId"].as_str())
                .and_then(|id| aliases.get(id))
                .filter(|id| {
                    thread.cwd.as_deref().is_some_and(|cwd| {
                        root_set(&recovered[*id]["rootPaths"]).contains(&path_key(cwd))
                    })
                })
        };
        if let Some(legacy) = legacy {
            assignments.insert(
                thread.id.clone(),
                json!({"projectKind":"local","projectId":legacy}),
            );
            hosts.insert(thread.id.clone(), json!("local"));
        }
    }
    set_map(&mut view, ASSIGN, assignments);
    set_map(&mut view, HOSTS, hosts);
    view
}

// Add verified local entries; existing target cloud/remote entries and settings win.
fn merge_local(target: &mut Value, view: &Value) {
    for key in ["local-projects", ASSIGN, HOSTS] {
        let mut entries = object(&target[key]);
        for (id, entry) in object(&view[key]) {
            entries.entry(id).or_insert(entry);
        }
        set_map(target, key, entries);
    }
    for (host, value) in object(&view[MAP]) {
        let mut entries = object(&target[MAP][&host]);
        for (id, entry) in object(&value) {
            entries.entry(id).or_insert(entry);
        }
        if !target[MAP].is_object() {
            target[MAP] = json!({});
        }
        target[MAP][&host] = json!(entries);
    }
    let mut order = target["project-order"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for id in view["project-order"].as_array().into_iter().flatten() {
        if !order.contains(id) {
            order.push(id.clone());
        }
    }
    target["project-order"] = json!(order);
}

/// Caller holds the launch/profile lease and has verified the Desktop is stopped.
/// Native Desktop clears top-level groups during its constructor, so restore the
/// target owner *and* its globals together, before the existing quick repair.
pub(crate) fn prepare(home: &Path, owner: Option<&str>) -> Result<bool, String> {
    let Some(bytes) = read_bytes(&home.join(STATE))? else {
        return Ok(false);
    };
    let before = decode(Some(&bytes))?;
    let mut after = before.clone();
    if before.get(SCOPES).is_some() && before.get(PARKED).is_some() {
        return Err("CODEX_PROJECT_ACCOUNT_STATE_AMBIGUOUS".into());
    }
    let mut scopes = before
        .get(SCOPES)
        .or_else(|| before.get(PARKED))
        .cloned()
        .unwrap_or_else(|| json!({"owner":null,"inactive":{}}));
    validate_scopes(&scopes)?;
    if before.get(ATOMS).is_some_and(|v| !v.is_object()) {
        return Err("CODEX_PROJECT_ACCOUNT_STATE_UNSUPPORTED".into());
    }
    let atoms = object(&before[ATOMS]);
    if let Some(current) = scopes["owner"].as_str().map(str::to_string) {
        scopes["inactive"][&current] = json!({
            "globals":scoped_globals(&before),
            "atoms":atoms.iter().filter(|(key, _)| !SHARED_ATOMS.contains(&key.as_str()))
                .map(|(key,value)| (key.clone(),value.clone())).collect::<BTreeMap<_,_>>(),
        });
    }
    let mut target = owner
        .and_then(|id| scopes["inactive"].get(id))
        .cloned()
        .unwrap_or_else(|| json!({"atoms":{},"globals":{}}));
    // Preserve unowned local UI state when starting again with an API key.
    if owner.is_none() && before.get(PARKED).is_some() {
        target["globals"] = scoped_globals(&before);
    }
    let mut sources = vec![target["globals"].clone(), scoped_globals(&before)];
    sources.extend(
        object(&scopes["inactive"])
            .into_values()
            .map(|scope| scope["globals"].clone()),
    );
    for source in &sources {
        if ["local-projects", MAP, ASSIGN, HOSTS]
            .iter()
            .any(|key| source.get(*key).is_some_and(|v| !v.is_object()))
            || source.get("project-order").is_some_and(|v| !v.is_array())
        {
            return Err("CODEX_PROJECT_ACCOUNT_STATE_UNSUPPORTED".into());
        }
    }
    let Some(primary) = project_database(home)? else {
        return Ok(false);
    };
    let db = open(&home.join(primary), false)?;
    db.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let projects = projects_with_limit(&db, Some(QUICK_MAX_PROJECTS))?;
    let threads = threads_with_limit(&db, Some(QUICK_MAX_THREADS))?;
    let view = local_view(home, &sources, &projects, &threads);
    merge_local(&mut target["globals"], &view);
    for key in SCOPED_GLOBALS {
        after.as_object_mut().unwrap().remove(*key);
        if let Some(value) = target["globals"].get(*key) {
            after[*key] = value.clone();
        }
    }
    let mut next_atoms = object(&target["atoms"]);
    next_atoms.extend(
        atoms
            .into_iter()
            .filter(|(key, _)| SHARED_ATOMS.contains(&key.as_str())),
    );
    after[ATOMS] = json!(next_atoms);
    scopes["owner"] = json!(owner);
    if let Some(owner) = owner {
        scopes["inactive"].as_object_mut().unwrap().remove(owner);
        after.as_object_mut().unwrap().remove(PARKED);
        after[SCOPES] = scopes;
    } else {
        // Desktop has no native null-owner slot. Park ONLY the native archive in
        // this same JSON file while API-key mode is active, then restore it for
        // OAuth. No extra profile, auth file, watcher or credential is created.
        scopes["owner"] = Value::Null;
        after.as_object_mut().unwrap().remove(SCOPES);
        after[PARKED] = scopes;
    }
    if after == before {
        return Ok(false);
    }
    let expected = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let content = serde_json::to_string(&after).map_err(|e| e.to_string())?;
    if !crate::modules::atomic_write::replace_secret_string_if_matches(
        &home.join(STATE),
        Some(expected),
        Some(&content),
    )? {
        return Err("CODEX_PROJECT_CHANGED".into());
    }
    Ok(true)
}

#[cfg(test)]
#[path = "account_state_tests.rs"]
mod tests;
