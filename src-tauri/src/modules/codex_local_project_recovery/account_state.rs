//! Bridge Codex Desktop's account-scoped renderer state while the profile is stopped.
//! Only projects and user threads verified in this profile's SQLite registry are
//! shared. Remote/cloud state and account atoms stay in their original scope.
use super::*;
use sha2::{Digest, Sha256};
use crate::modules::codex_session_visibility::{
    launch_visibility_budget, with_launch_visibility_budget, configure_launch_visibility_connection,
    LAUNCH_VISIBILITY_TIMEOUT, QUICK_PROJECT_REPAIR_BUDGET,
};

#[path = "account_state_ssh.rs"]
mod ssh;

const SCOPES: &str = "electron-account-renderer-state-v1";
const PARKED: &str = "cockpit-local-project-account-state-v1";
const ATOMS: &str = "electron-persisted-atom-state";
const PINNED_PROJECTS: &str = "pinned-project-ids";
const PROJECT_APPEARANCES: &str = "project-appearances";
const THREAD_ORDERS: &str = "sidebar-project-thread-orders";
const PINNED_THREADS: &str = "pinned-thread-ids";
const MIGRATED_PINS: &str = "app-server-migrated-pinned-thread-ids-by-host";

#[path = "account_presentation.rs"]
mod presentation;

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
    PROJECT_APPEARANCES,
    "project-files",
    PINNED_THREADS,
    MIGRATED_PINS,
    PINNED_PROJECTS,
    THREAD_ORDERS,
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
    let mut current_aliases = HashMap::new();
    let mut current_server_ids = HashMap::new();
    let mut presentation_projects = Vec::new();
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
        if threads.is_empty() {
            continue;
        }
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
        for (source_index, (source, index)) in sources.iter().zip(&by_source).enumerate() {
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
                candidates.push((source_index, *entry));
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
                candidates.push((source_index, first));
            }
        }
        // This launch step preserves archived groups; creating new groups is
        // the existing project-recovery planner's job.
        let Some((_, (legacy, entry))) = candidates.first().copied().filter(|_| !ambiguous) else {
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
        for (source_index, (alias, _)) in candidates {
            aliases.insert(alias, legacy);
            // Sources are the target, the active scope, then inactive scopes.
            if source_index == 1 {
                current_aliases.insert(alias, legacy);
                current_server_ids.insert(project.id.as_str(), legacy);
                presentation_projects.push((alias, project.id.as_str(), legacy));
            }
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
    if let Some(pins) = merge_local_pins(
        sources[0],
        sources[1],
        &current_aliases,
        &current_server_ids,
    ) {
        view[PINNED_PROJECTS] = pins;
    }
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
    presentation::merge(
        &mut view, sources, &host, &presentation_projects, threads, &assignments,
        &nonlocal_threads,
    )?;
    set_map(&mut view, "local-projects", recovered);
    set_map(&mut view, ASSIGN, assignments);
    set_map(&mut view, HOSTS, hosts);
    Ok(view)
}

fn merge_local_pins(
    target: &Value,
    current: &Value,
    current_aliases: &HashMap<&str, &str>,
    current_server_ids: &HashMap<&str, &str>,
) -> Option<Value> {
    let current_pins = current[PINNED_PROJECTS].as_array()?;
    if current_aliases.is_empty() {
        return None;
    }
    let mut target_pins = match target.get(PINNED_PROJECTS) {
        Some(pins) => pins.as_array()?.clone(),
        None => Vec::new(),
    };
    // An explicit list includes unpins. Use it only for verified projects in
    // the active scope, so older archives cannot re-pin them.
    let current_projects = current_aliases.values().copied().collect::<HashSet<_>>();
    let pinned = current_pins
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|id| {
            current_aliases
                .get(id)
                .or_else(|| current_server_ids.get(id))
                .copied()
        })
        .collect::<Vec<_>>();
    let pinned_ids = pinned.iter().copied().collect::<HashSet<_>>();
    let mut kept_local_pins = HashSet::new();
    target_pins.retain(|value| {
        let Some(id) = value.as_str() else {
            return true;
        };
        let project_id = current_projects
            .get(id)
            .or_else(|| current_server_ids.get(id));
        project_id.is_none_or(|id| pinned_ids.contains(id) && kept_local_pins.insert(*id))
    });
    for id in pinned {
        if kept_local_pins.insert(id) {
            target_pins.push(json!(id));
        }
    }
    Some(json!(target_pins))
}

// Add verified local entries; existing target cloud/remote entries and settings win.
fn merge_local(target: &mut Value, view: &Value) {
    for key in [PINNED_PROJECTS, PROJECT_APPEARANCES, THREAD_ORDERS, PINNED_THREADS] {
        if let Some(value) = view.get(key) {
            target[key] = value.clone();
        }
    }
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
    let mut seen = order.iter().filter_map(Value::as_str).map(str::to_owned).collect::<HashSet<_>>();
    for id in view["project-order"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if seen.insert(id.to_owned()) {
            order.push(json!(id));
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
        return Ok(ssh::prepare_startup(home, &before));
    }
    let mut after = before.clone();
    let mut scopes = before
        .get(SCOPES)
        .or_else(|| before.get(PARKED))
        .cloned()
        .unwrap_or_else(|| json!({"owner":null,"inactive":{}}));
    validate_scopes(&scopes)?;
    if let (Some(_), Some(parked)) = (before.get(SCOPES), before.get(PARKED)) {
        validate_scopes(parked)?;
        // Native Desktop may create a new archive after signing in while the
        // Cockpit archive is parked. Native entries and its active owner are
        // newer. Only recover parked accounts that native state does not own.
        let active = scopes["owner"].as_str().map(str::to_owned);
        let inactive = scopes["inactive"].as_object_mut().unwrap();
        for (id, entry) in parked["inactive"].as_object().unwrap() {
            if active.as_deref() != Some(id.as_str()) {
                inactive.entry(id.clone()).or_insert_with(|| entry.clone());
            }
        }
    }
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
    let mut ssh_state = ssh::shared_state(&before, &sources)?;
    // Account-state restoration also carries the native onboarding flags. It
    // must run for projectless profiles, before a project database exists.
    if let Some(primary) = project_database(home)? {
        let db = open(&home.join(primary), false)?;
        db.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        // Project visibility depends on the registry. Read thread memberships
        // within a separate budget so they cannot block restoring the groups.
        let projects = projects_with_limit(&db, None)?;
        let large_history: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM threads LIMIT 1 OFFSET ?1)",
                [QUICK_MAX_THREADS as i64],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let threads = if large_history {
            let ids = presentation::referenced_threads(&sources[..2])?;
            let budget = launch_visibility_budget().map(|budget|
                budget.deadline.saturating_duration_since(std::time::Instant::now()) / 4)
                .unwrap_or(QUICK_PROJECT_REPAIR_BUDGET);
            match with_launch_visibility_budget(budget, || {
                configure_launch_visibility_connection(&db)?;
                threads_for_presentation(&db, &ids)
            }) {
                Err(error) if error.starts_with(LAUNCH_VISIBILITY_TIMEOUT) => {
                    crate::modules::logger::log_warn("[Codex Local Projects] Thread membership recovery deferred within launch budget");
                    Vec::new()
                }
                result => result?,
            }
        } else {
            threads_with_limit(&db, Some(QUICK_MAX_THREADS))?
        };
        let view = local_view(home, &sources, &projects, &threads)?;
        // Finish the read transaction before writing the renderer state.
        drop(db);
        merge_local(&mut target["globals"], &view);
    }
    let remaining = launch_visibility_budget().map(|budget|
        budget.deadline.saturating_duration_since(std::time::Instant::now()));
    let verification = if let Some(remaining) = remaining {
        with_launch_visibility_budget((remaining / 4).min(QUICK_PROJECT_REPAIR_BUDGET),
            || ssh::verify_threads(home, &mut ssh_state))
    } else {
        ssh::verify_threads(home, &mut ssh_state)
    };
    match verification {
        Err(error) if error.starts_with(LAUNCH_VISIBILITY_TIMEOUT) => {
            crate::modules::logger::log_warn("[Codex SSH] Thread membership recovery deferred within launch budget");
        }
        result => result?,
    }
    ssh::merge(&mut target["globals"], &ssh_state)?;
    ssh::remember_hosts(&mut after, &ssh_state);
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
        return Ok(ssh::prepare_startup(home, &after));
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
    ssh::prepare_startup(home, &after);
    Ok(true)
}

#[cfg(test)]
#[path = "account_state_tests.rs"]
mod tests;
