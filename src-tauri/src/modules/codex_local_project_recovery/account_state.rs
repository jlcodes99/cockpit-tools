//! Bridge Codex Desktop's account-scoped renderer state at a stopped launch.
//! Only projects and user threads verified in this profile's SQLite registry are
//! shared. Remote/cloud state and account atoms stay in their original scope.
use super::*;
use sha2::{Digest, Sha256};

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
            entries
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

fn root_key(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort_unstable();
    paths.dedup();
    paths
}

fn local_view(
    home: &Path,
    sources: &[&Value],
    projects: &[Project],
    threads: &[Thread],
) -> Result<Value, String> {
    let host = format!("local:{}", home.display());
    let mut view = json!({});
    let mut recovered = BTreeMap::new();
    let mut mapping = BTreeMap::new();
    let mut aliases = HashMap::new();
    let mut server_to_legacy = HashMap::new();
    let mut blocked = HashSet::new();
    let mut by_source = Vec::new();
    let mut nonlocal_threads = HashSet::new();
    let mut explicit_assignments = HashMap::new();
    for source in sources {
        check()?;
        if let Some(deletions) =
            source["app-server-pending-project-deletions-by-host"][&host].as_object()
        {
            for (id, value) in deletions {
                blocked.insert(id.as_str());
                if let Some(id) = value["projectId"].as_str() {
                    blocked.insert(id);
                }
            }
        }
        let mut index: HashMap<Vec<String>, Vec<(&str, &Value)>> = HashMap::new();
        if let Some(entries) = source["local-projects"].as_object() {
            for (id, entry) in entries {
                check()?;
                if !cloud(entry) {
                    let roots = root_key(root_set(&entry["rootPaths"]));
                    if !roots.is_empty() {
                        index.entry(roots).or_default().push((id.as_str(), entry));
                    }
                }
            }
        }
        by_source.push(index);
        if let Some(hosts) = source[HOSTS].as_object() {
            nonlocal_threads.extend(hosts.iter().filter_map(|(id, host)| {
                host.as_str()
                    .is_some_and(|host| host != "local")
                    .then_some(id.as_str())
            }));
        }
        if let Some(assignments) = source[ASSIGN].as_object() {
            for (id, assignment) in assignments {
                if assignment["projectKind"]
                    .as_str()
                    .is_some_and(|kind| kind != "local")
                {
                    nonlocal_threads.insert(id.as_str());
                }
                explicit_assignments
                    .entry(id.as_str())
                    .or_insert(assignment);
            }
        }
    }
    // The live SQLite registry verifies a saved local project's identity. Do
    // not stat workspace paths (which can be network mounts) on the launch path.
    let mut root_counts = HashMap::new();
    let registry = projects
        .iter()
        .filter_map(|project| {
            if cloud(&project.metadata)
                || project.roots.is_empty()
                || project
                    .roots
                    .iter()
                    .any(|root| !Path::new(root).is_absolute() || cloud_root(root))
            {
                return None;
            }
            let roots = root_key(project.roots.iter().map(|path| path_key(path)));
            *root_counts.entry(roots.clone()).or_insert(0usize) += 1;
            Some((project, roots))
        })
        .collect::<Vec<_>>();
    for (project, roots) in registry {
        check()?;
        if blocked.contains(project.id.as_str()) {
            continue;
        }
        let mut candidates = Vec::new();
        let mut ambiguous = false;
        for (source, index) in sources.iter().zip(&by_source) {
            // Stable registry IDs distinguish different projects using the same
            // directories. Only infer an identity from roots when it is unique.
            let entries = index.get(&roots).into_iter().flatten().copied();
            let explicit = entries
                .clone()
                .filter(|(id, _)| match source[MAP][&host].get(*id) {
                    Some(mapped) => mapped.as_str() == Some(project.id.as_str()),
                    None => *id == project.id,
                })
                .collect::<Vec<_>>();
            if explicit.len() > 1 {
                ambiguous = true;
                break;
            }
            if let Some(entry) = explicit.first() {
                candidates.push(*entry);
                continue;
            }
            if root_counts[&roots] != 1 {
                continue;
            }
            let mut matches = entries.filter(|(id, _)| {
                source[MAP][&host]
                    .get(*id)
                    .is_none_or(|mapped| mapped.as_str() == Some(project.id.as_str()))
            });
            if let Some(first) = matches.next() {
                if matches.next().is_some() {
                    ambiguous = true;
                    break;
                }
                candidates.push(first);
            }
        }
        // This launch step preserves archived groups; creating new groups is
        // the existing project-recovery planner's job.
        let Some((legacy, entry)) = candidates.first().copied().filter(|_| !ambiguous) else {
            continue;
        };
        if blocked.contains(legacy)
            || recovered.contains_key(legacy)
            || sources
                .first()
                .and_then(|source| source["local-projects"].get(legacy))
                .is_some_and(|entry| {
                    cloud(entry)
                        || root_key(root_set(&entry["rootPaths"])) != roots
                        || sources[0][MAP][&host]
                            .get(legacy)
                            .is_some_and(|mapped| mapped.as_str() != Some(project.id.as_str()))
                })
        {
            continue;
        }
        for (alias, _) in candidates {
            aliases.insert(alias, legacy);
        }
        aliases.insert(project.id.as_str(), legacy);
        server_to_legacy.insert(project.id.as_str(), legacy);
        mapping.insert(legacy.to_string(), json!(project.id));
        recovered.insert(legacy.to_string(), entry.clone());
    }
    let mut order = Vec::new();
    let mut seen_order = HashSet::new();
    for source in sources {
        check()?;
        for id in source["project-order"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Some(id) = aliases.get(id).copied().filter(|id| seen_order.insert(*id)) {
                order.push(json!(id));
            }
        }
    }
    for id in recovered.keys() {
        if seen_order.insert(id) {
            order.push(json!(id));
        }
    }
    view["project-order"] = json!(order);
    view[MAP] = json!({ &host: mapping });
    let ungrouped = sources
        .first()
        .map(|source| projectless(source))
        .unwrap_or_default();
    let mut assignments = BTreeMap::new();
    let mut hosts = BTreeMap::new();
    for thread in threads.iter().filter(|thread| thread.root) {
        check()?;
        if nonlocal_threads.contains(thread.id.as_str()) || ungrouped.contains(&thread.id) {
            continue;
        }
        let legacy = if let Some(server) = &thread.project {
            server_to_legacy.get(server.as_str())
        } else {
            explicit_assignments
                .get(thread.id.as_str())
                .filter(|assignment| assignment["projectKind"] == "local")
                .and_then(|assignment| assignment["projectId"].as_str())
                .and_then(|id| aliases.get(id))
                .filter(|id| {
                    thread.cwd.as_deref().is_some_and(|cwd| {
                        root_set(&recovered[**id]["rootPaths"]).contains(&path_key(cwd))
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
    set_map(&mut view, "local-projects", recovered);
    set_map(&mut view, ASSIGN, assignments);
    set_map(&mut view, HOSTS, hosts);
    Ok(view)
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
    check()?;
    let Some(bytes) = read_bytes(&home.join(STATE))? else {
        return Ok(false);
    };
    check()?;
    let before = decode(Some(&bytes))?;
    // Older/unscoped state is handled by the existing recovery path. Its
    // top-level preferences do not identify an owner that we can archive.
    if before.get(SCOPES).is_none() && before.get(PARKED).is_none() {
        return Ok(false);
    }
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
    let current_globals = scoped_globals(&before);
    if owner.is_none() && scopes["owner"].is_null() {
        target["globals"] = current_globals.clone();
        target["atoms"] = before[ATOMS].clone();
    }
    let mut sources = vec![&target["globals"], &current_globals];
    sources.extend(
        scopes["inactive"]
            .as_object()
            .unwrap()
            .values()
            .map(|scope| &scope["globals"]),
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
    // Account-state restoration also carries the native onboarding flags. It
    // must run for projectless profiles, before a project database exists.
    if let Some(primary) = project_database(home)? {
        let db = open(&home.join(primary), false)?;
        db.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let projects = projects_with_limit(&db, Some(QUICK_MAX_PROJECTS))?;
        let threads = threads_with_limit(&db, Some(QUICK_MAX_THREADS))?;
        let view = local_view(home, &sources, &projects, &threads)?;
        // Finish the read transaction before writing the renderer state.
        drop(db);
        merge_local(&mut target["globals"], &view);
    }
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
    let expected_hash: [u8; 32] = Sha256::digest(&bytes).into();
    let content = serde_json::to_string(&after).map_err(|e| e.to_string())?;
    if content.len() as u64 > MAX_STATE_BYTES {
        return Err("CODEX_PROJECT_STATE_TOO_LARGE".into());
    }
    check()?;
    // Codex owns the adjacent .bak. Do not replace it with a temporary backup
    // or treat it as a plaintext credential backup to remove.
    if !crate::modules::atomic_write::restore_string_atomic_if_hash_matches(
        &home.join(STATE),
        expected_hash,
        &content,
    )? {
        return Err("CODEX_PROJECT_CHANGED".into());
    }
    Ok(true)
}

#[cfg(test)]
#[path = "account_state_tests.rs"]
mod tests;
