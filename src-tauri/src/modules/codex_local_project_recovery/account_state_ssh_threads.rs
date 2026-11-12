//! Carry existing remote project memberships only when this profile's catalog
//! confirms the same SSH host and user-visible thread. Never infer from titles.
use super::super::super::super::table_columns;
use super::*;

pub(super) fn collect_memberships(
    globals: &mut Value,
    projects: &BTreeMap<String, Value>,
    aliases: &BTreeMap<String, String>,
    sources: &[&Value],
) {
    let mut seen = HashSet::new();
    let mut assignments = BTreeMap::new();
    let mut hosts = BTreeMap::new();
    for source in sources {
        for (id, assignment) in source[ASSIGN].as_object().into_iter().flatten() {
            if !seen.insert(id) || assignment["projectKind"] != "remote" {
                continue;
            }
            let Some(project) = assignment["projectId"]
                .as_str()
                .and_then(|id| aliases.get(id))
                .and_then(|id| projects.get(id))
            else {
                continue;
            };
            let host = &project["hostId"];
            if source[HOSTS]
                .get(id)
                .is_some_and(|existing| existing != host)
            {
                continue;
            }
            assignments.insert(
                id.clone(),
                json!({"projectKind":"remote","projectId":project["id"]}),
            );
            hosts.insert(id.clone(), host.clone());
        }
    }
    globals[ASSIGN] = json!(assignments);
    globals[HOSTS] = json!(hosts);
}

pub(super) fn verify_memberships(
    home: &Path,
    globals: &mut Value,
    aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    if object(&globals[ASSIGN]).is_empty() {
        return Ok(());
    }
    let projects: BTreeMap<_, _> = globals[PROJECTS]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|project| project["id"].as_str().map(|id| (id.to_string(), project)))
        .collect();
    let mut verified = HashSet::new();
    for path in catalog_paths(home, true)
        .into_iter()
        .filter(|p| p.is_file())
    {
        check()?;
        let db = open(&path, false)?;
        let columns = table_columns(&db, "local_thread_catalog")?;
        if !["host_id", "thread_id", "project_id", "cwd", "source_kind"]
            .iter()
            .all(|key| columns.contains(*key))
        {
            continue;
        }
        let sql = format!("SELECT thread_id,project_id,cwd,source_kind,{} FROM local_thread_catalog WHERE host_id=?1 AND thread_id=?2",
            column_expr(&columns, "source_detail", "NULL"));
        let mut statement = db.prepare(&sql).map_err(|e| e.to_string())?;
        for (id, _) in globals[ASSIGN].as_object().into_iter().flatten() {
            check()?;
            let Some(host) = globals[HOSTS][id].as_str() else {
                continue;
            };
            let mut rows = statement
                .query([host, id.as_str()])
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                check()?;
                let id: String = row.get(0).map_err(|e| e.to_string())?;
                let Some(assignment) = globals[ASSIGN].get(&id) else {
                    continue;
                };
                if globals[HOSTS][&id] != host {
                    continue;
                }
                let project = projects[assignment["projectId"].as_str().unwrap()];
                if matches_project(row, project, aliases)? {
                    verified.insert(id);
                }
            }
        }
    }
    for key in [ASSIGN, HOSTS] {
        globals[key]
            .as_object_mut()
            .unwrap()
            .retain(|id, _| verified.contains(id));
    }
    Ok(())
}

fn matches_project(
    row: &rusqlite::Row<'_>,
    project: &Value,
    aliases: &BTreeMap<String, String>,
) -> Result<bool, String> {
    let source: Option<String> = row.get(3).map_err(|e| e.to_string())?;
    let detail: Option<String> = row.get(4).map_err(|e| e.to_string())?;
    if [source.as_deref(), detail.as_deref()]
        .into_iter()
        .flatten()
        .any(|v| v.to_lowercase().contains("subagent") || v.eq_ignore_ascii_case("guardian"))
    {
        return Ok(false);
    }
    let native_project: Option<String> = row.get(1).map_err(|e| e.to_string())?;
    let cwd: Option<String> = row.get(2).map_err(|e| e.to_string())?;
    Ok(match native_project {
        Some(id) => aliases.get(&id).is_some_and(|id| project["id"] == *id),
        None => cwd.as_deref().is_some_and(|cwd| {
            project["remotePath"]
                .as_str()
                .is_some_and(|root| path_key(root) == path_key(cwd))
        }),
    })
}

pub(super) fn merge_memberships(target: &mut Value, shared: &SharedState) -> Result<(), String> {
    if [ASSIGN, HOSTS]
        .iter()
        .any(|key| target.get(*key).is_some_and(|v| !v.is_object()))
    {
        return Err("CODEX_PROJECT_SSH_STATE_UNSUPPORTED".into());
    }
    let mut assignments = object(&target[ASSIGN]);
    let mut hosts = object(&target[HOSTS]);
    let stale: BTreeSet<_> = assignments
        .iter()
        .filter(|(id, entry)| {
            owned_host(shared, &target[HOSTS][*id])
                || (entry["projectKind"] == "remote"
                    && entry["projectId"]
                        .as_str()
                        .is_some_and(|id| shared.known_projects.contains(id)))
        })
        .map(|(id, _)| id.clone())
        .chain(
            hosts
                .iter()
                .filter(|(_, host)| owned_host(shared, host))
                .map(|(id, _)| id.clone()),
        )
        .collect();
    for id in stale {
        assignments.remove(&id);
        hosts.remove(&id);
    }
    for (id, assignment) in object(&shared.globals[ASSIGN]) {
        // Current target cloud/local membership has priority over a remote
        // thread with a colliding ID. Never replace another host's ownership.
        if assignments.contains_key(&id) || hosts.contains_key(&id) {
            continue;
        }
        hosts.insert(id.clone(), shared.globals[HOSTS][&id].clone());
        assignments.insert(id, assignment);
    }
    set_map(target, ASSIGN, assignments);
    set_map(target, HOSTS, hosts);
    Ok(())
}
