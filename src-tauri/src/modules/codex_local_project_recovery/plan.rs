#[derive(Debug)]
struct Project {
    id: String,
    name: String,
    metadata: Value,
    created: i64,
    updated: i64,
    roots: Vec<String>,
}

fn column_expr(columns: &HashSet<String>, column: &str, fallback: &str) -> String {
    if columns.contains(column) {
        column.to_string()
    } else {
        fallback.to_string()
    }
}

fn projects_with_limit(db: &Connection, limit: Option<usize>) -> Result<Vec<Project>, String> {
    let columns = super::table_columns(db, "projects")?;
    let roots = super::table_columns(db, "project_roots")?;
    if !["id", "name"].iter().all(|c| columns.contains(*c))
        || !["project_id", "path"].iter().all(|c| roots.contains(*c))
    {
        return Err("CODEX_PROJECT_SCHEMA_UNSUPPORTED".into());
    }
    let sql = format!(
        "SELECT id,name,{},{},{} FROM projects ORDER BY id{}",
        column_expr(&columns, "metadata", "'{}'"),
        column_expr(&columns, "created_at_ms", "0"),
        column_expr(&columns, "updated_at_ms", "0"),
        limit
            .map(|n| format!(" LIMIT {}", n + 1))
            .unwrap_or_default()
    );
    let mut statement = db.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check()?;
        let metadata: Option<String> = row.get(2).map_err(|e| e.to_string())?;
        result.push(Project {
            id: row.get(0).map_err(|e| e.to_string())?,
            name: row.get(1).map_err(|e| e.to_string())?,
            // Invalid metadata is never interpreted as proof of a local project.
            metadata: serde_json::from_str(metadata.as_deref().unwrap_or("{}"))
                .map_err(|e| e.to_string())?,
            created: row
                .get::<_, Option<i64>>(3)
                .map_err(|e| e.to_string())?
                .unwrap_or(0),
            updated: row
                .get::<_, Option<i64>>(4)
                .map_err(|e| e.to_string())?
                .unwrap_or(0),
            roots: Vec::new(),
        });
    }
    if limit.is_some_and(|n| result.len() > n) {
        return Err("CODEX_PROJECT_QUICK_LIMIT".into());
    }
    let order = if roots.contains("position") {
        "position,path"
    } else {
        "path"
    };
    let mut statement = db
        .prepare(&format!(
            "SELECT path FROM project_roots WHERE project_id=?1 ORDER BY {order}{}",
            if limit.is_some() { " LIMIT 33" } else { "" }
        ))
        .map_err(|e| e.to_string())?;
    for project in &mut result {
        project.roots = statement
            .query_map([&project.id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())?;
        if limit.is_some() && project.roots.len() > 32 {
            return Err("CODEX_PROJECT_QUICK_LIMIT".into());
        }
    }
    Ok(result)
}

fn fingerprint(projects: &[Project]) -> String {
    use sha2::{Digest, Sha256};
    let values: Vec<_> = projects
        .iter()
        .map(|p| json!([p.id, p.name, p.metadata, p.created, p.updated, p.roots]))
        .collect();
    format!("{:x}", Sha256::digest(serde_json::to_vec(&values).unwrap()))
}

fn snapshot(home: &Path, current: &Value) -> Result<Value, String> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(home).map_err(|e| e.to_string())?.take(512) {
        check()?;
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == STATE
            || !name.starts_with(STATE)
            || !entry.file_type().is_ok_and(|t| t.is_file())
        {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        if metadata.len() > MAX_STATE_BYTES {
            continue;
        }
        candidates.push((metadata.modified().ok(), entry.path()));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in candidates.into_iter().take(8) {
        check()?;
        let Ok(bytes) = read_bytes(&path) else {
            continue;
        };
        let Ok(value) = decode(bytes.as_deref()) else {
            continue;
        };
        if !object(&value["local-projects"]).is_empty()
            && (object(&value["local-projects"]).len() > object(&current["local-projects"]).len()
                || object(&value[ASSIGN]).len() > object(&current[ASSIGN]).len()
                || object(&value[MAP]).len() > object(&current[MAP]).len())
        {
            return Ok(value);
        }
    }
    Ok(json!({}))
}

fn root_set(value: &Value) -> HashSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(path_key)
        .collect()
}

fn cloud(value: &Value) -> bool {
    [
        "codex.chatGptBacking",
        "codex.chatGptBackingCreation",
        "codex.retiredChatGptBacking",
        "chatGptBacking",
        "chatGptBackingCreation",
        "retiredChatGptBacking",
    ]
    .iter()
    .any(|key| value.get(*key).is_some_and(|v| !v.is_null()))
}

fn cloud_root(path: &str) -> bool {
    path_key(path)
        .split(['/', '\\'])
        .any(|p| p.eq_ignore_ascii_case(".chatgpt-projects"))
}

fn projectless(state: &Value) -> HashSet<String> {
    let mut result = ids(&state["projectless-thread-ids"]);
    result.extend(object(&state["thread-projectless-output-directories"]).into_keys());
    result
}

#[derive(Debug)]
struct Thread {
    id: String,
    project: Option<String>,
    cwd: Option<String>,
    root: bool,
}

fn threads_with_limit(db: &Connection, limit: Option<usize>) -> Result<Vec<Thread>, String> {
    let cols = super::table_columns(db, "threads")?;
    if !["id", "cwd", "project_id", "source"]
        .iter()
        .all(|c| cols.contains(*c))
        || !(cols.contains("has_user_event") || cols.contains("first_user_message"))
    {
        return Err("CODEX_PROJECT_SCHEMA_UNSUPPORTED".into());
    }
    let sql = format!(
        "SELECT id,project_id,cwd,source,{},{},{} FROM threads ORDER BY id{}",
        column_expr(&cols, "thread_source", "NULL"),
        column_expr(&cols, "has_user_event", "0"),
        column_expr(&cols, "first_user_message", "NULL"),
        limit
            .map(|n| format!(" LIMIT {}", n + 1))
            .unwrap_or_default()
    );
    let mut statement = db.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        check()?;
        let source: Option<String> = row.get(3).map_err(|e| e.to_string())?;
        let detail: Option<String> = row.get(4).map_err(|e| e.to_string())?;
        let user: Option<i64> = row.get(5).map_err(|e| e.to_string())?;
        let first: Option<String> = row.get(6).map_err(|e| e.to_string())?;
        let child = source.as_deref().is_some_and(|s| {
            s.to_lowercase().contains("subagent") || s.eq_ignore_ascii_case("guardian")
        }) || detail.as_deref().is_some_and(|s| {
            ["subagent", "guardian"]
                .iter()
                .any(|x| s.eq_ignore_ascii_case(x))
        });
        result.push(Thread {
            id: row.get(0).map_err(|e| e.to_string())?,
            project: row.get(1).map_err(|e| e.to_string())?,
            cwd: row.get(2).map_err(|e| e.to_string())?,
            root: !child && (user.unwrap_or(0) != 0 || first.is_some_and(|s| !s.is_empty())),
        });
    }
    if limit.is_some_and(|n| result.len() > n) {
        return Err("CODEX_PROJECT_QUICK_LIMIT".into());
    }
    Ok(result)
}

fn catalog_paths(home: &Path, quick: bool) -> Vec<PathBuf> {
    if quick {
        [
            "state_5.sqlite",
            "sqlite/state_5.sqlite",
            "sqlite/codex-dev.db",
        ]
        .into_iter()
        .map(|p| home.join(p))
        .collect()
    } else {
        super::sqlite_candidate_paths(home)
    }
}

fn catalog_memberships(
    home: &Path,
    quick: bool,
) -> Result<HashMap<String, HashSet<String>>, String> {
    let mut memberships: HashMap<String, HashSet<String>> = HashMap::new();
    for path in catalog_paths(home, quick) {
        if !path.exists() {
            continue;
        }
        let db = open(&path, false)?;
        let columns = super::table_columns(&db, "local_thread_catalog")?;
        if !["thread_id", "host_id", "project_id"]
            .iter()
            .all(|c| columns.contains(*c))
        {
            continue;
        }
        let limit = if quick {
            format!(" LIMIT {}", QUICK_MAX_CATALOG_ROWS + 1)
        } else {
            String::new()
        };
        let mut statement = db.prepare(&format!("SELECT thread_id,project_id FROM local_thread_catalog WHERE host_id='local' AND project_id IS NOT NULL{limit}")).map_err(|e| e.to_string())?;
        let mut rows = statement.query([]).map_err(|e| e.to_string())?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            check()?;
            count += 1;
            if quick && count > QUICK_MAX_CATALOG_ROWS {
                return Err("CODEX_PROJECT_QUICK_LIMIT".into());
            }
            memberships
                .entry(row.get(0).map_err(|e| e.to_string())?)
                .or_default()
                .insert(row.get(1).map_err(|e| e.to_string())?);
        }
    }
    Ok(memberships)
}

#[cfg(test)]
pub(super) fn scan(home: &Path, selected: Option<&HashSet<String>>) -> Result<Plan, String> {
    scan_with_roots(home, selected, &|p| Path::new(p).is_dir())
}

fn project_database(home: &Path) -> Result<Option<PathBuf>, String> {
    let mut unsupported = false;
    for relative in ["state_5.sqlite", "sqlite/state_5.sqlite"] {
        let path = home.join(relative);
        if !path.exists() {
            continue;
        }
        let db = open(&path, false)?;
        let projects = super::table_columns(&db, "projects")?;
        let roots = super::table_columns(&db, "project_roots")?;
        if projects.is_empty() && roots.is_empty() {
            continue;
        }
        if ["id", "name"].iter().all(|c| projects.contains(*c))
            && ["project_id", "path"].iter().all(|c| roots.contains(*c))
        {
            return Ok(Some(PathBuf::from(relative)));
        }
        unsupported = true;
    }
    if unsupported {
        Err("CODEX_PROJECT_SCHEMA_UNSUPPORTED".into())
    } else {
        Ok(None)
    }
}

pub(super) fn scan_for_repair(
    home: &Path,
    selected: Option<&HashSet<String>>,
    quick: bool,
) -> Result<Option<Plan>, String> {
    if project_database(home)?.is_none() {
        return Ok(None);
    }
    scan_impl(home, selected, quick, &|p| quick || Path::new(p).is_dir()).map(Some)
}

#[cfg(test)]
fn scan_with_roots(
    home: &Path,
    selected: Option<&HashSet<String>>,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Plan, String> {
    scan_impl(home, selected, false, exists)
}

fn scan_impl(
    home: &Path,
    selected: Option<&HashSet<String>>,
    quick: bool,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Plan, String> {
    check()?;
    if quick
        && [STATE, &format!("{STATE}.bak")]
            .iter()
            .any(|p| fs::metadata(home.join(p)).is_ok_and(|m| m.len() > 2_000_000))
    {
        return Err("CODEX_PROJECT_QUICK_LIMIT".into());
    }
    let before_bytes = read_bytes(&home.join(STATE))?;
    let before = decode(before_bytes.as_deref())?;
    for key in ["local-projects", MAP, ASSIGN, HOSTS, MIGRATION] {
        if before.get(key).is_some_and(|value| !value.is_object()) {
            return Err("CODEX_PROJECT_SCHEMA_UNSUPPORTED".into());
        }
    }
    if before.get("project-order").is_some_and(|v| !v.is_array()) {
        return Err("CODEX_PROJECT_SCHEMA_UNSUPPORTED".into());
    }
    let old = if quick {
        json!({})
    } else {
        snapshot(home, &before)?
    };
    let host = host_key(&before, home);
    let old_host = host_key(&old, home);
    let primary = project_database(home)?.ok_or("CODEX_PROJECT_SCHEMA_UNSUPPORTED")?;
    let db = open(&home.join(&primary), false)?;
    db.execute_batch("BEGIN").map_err(|e| e.to_string())?;
    let db_projects = projects_with_limit(&db, quick.then_some(QUICK_MAX_PROJECTS))?;
    let db_threads = threads_with_limit(&db, quick.then_some(QUICK_MAX_THREADS))?;
    let catalog_memberships = catalog_memberships(home, quick)?;
    let project_fingerprint = fingerprint(&db_projects);
    let mut after = before.clone();
    let mut recovered = object(&before["local-projects"]);
    let saved = recovered.clone();
    let mut mapping = object(&before[MAP][&host]);
    let old_mapping = object(&old[MAP][&old_host]);
    let old_projects = object(&old["local-projects"]);
    let pending = object(&before["app-server-pending-project-deletions-by-host"][&host]);
    let mut blocked: HashSet<String> = pending.keys().cloned().collect();
    blocked.extend(
        pending
            .values()
            .filter_map(|v| v["projectId"].as_str())
            .map(str::to_string),
    );
    let mut allowed: BTreeMap<String, String> = BTreeMap::new();
    let mut counts = Counts::default();
    for project in &db_projects {
        check()?;
        let roots: HashSet<_> = project.roots.iter().map(|p| path_key(p)).collect();
        let explicit: Vec<_> = mapping
            .iter()
            .filter(|(id, v)| v.as_str() == Some(&project.id) && saved.contains_key(*id))
            .map(|(id, _)| id.clone())
            .collect();
        let by_roots: Vec<_> = saved
            .iter()
            .filter(|(_, p)| !roots.is_empty() && root_set(&p["rootPaths"]) == roots)
            .map(|(id, _)| id.clone())
            .collect();
        let current = if explicit.len() == 1 {
            explicit.first()
        } else if explicit.is_empty() && by_roots.len() == 1 {
            by_roots.first()
        } else {
            None
        };
        let previous: Vec<_> = old_mapping
            .iter()
            .filter(|(id, v)| {
                v.as_str() == Some(&project.id)
                    && old_projects
                        .get(*id)
                        .is_some_and(|p| !cloud(p) && root_set(&p["rootPaths"]) == roots)
            })
            .map(|(id, _)| id.clone())
            .collect();
        let legacy = current
            .or_else(|| {
                if previous.len() == 1 {
                    previous.first()
                } else {
                    None
                }
            })
            .unwrap_or(&project.id);
        if selected.is_some_and(|ids| {
            !db_threads.iter().any(|thread| {
                ids.contains(&thread.id)
                    && thread.root
                    && (thread.project.as_ref() == Some(&project.id)
                        || before[ASSIGN][&thread.id]["projectId"].as_str() == Some(legacy)
                        || old[ASSIGN][&thread.id]["projectId"].as_str() == Some(legacy)
                        || thread
                            .cwd
                            .as_deref()
                            .is_some_and(|cwd| roots.contains(&path_key(cwd))))
            })
        }) {
            continue;
        }
        let ambiguous = explicit.len() > 1
            || (current.is_none() && (by_roots.len() > 1 || previous.len() > 1))
            || (current.is_none()
                && db_projects.iter().any(|p| {
                    p.id != project.id
                        && p.roots.iter().map(|p| path_key(p)).collect::<HashSet<_>>() == roots
                }));
        if blocked.contains(&project.id)
            || blocked.contains(legacy)
            || ambiguous
            || cloud(&project.metadata)
            || current.and_then(|id| saved.get(id)).is_some_and(cloud)
            || project.roots.is_empty()
            || project.roots.iter().any(|p| cloud_root(p) || !exists(p))
            || mapping
                .get(legacy)
                .is_some_and(|v| v.as_str() != Some(&project.id))
        {
            counts.skipped_projects += 1;
            continue;
        }
        if !recovered.contains_key(legacy)
            || mapping.get(legacy).and_then(Value::as_str) != Some(&project.id)
        {
            counts.projects += 1;
        }
        if !recovered.contains_key(legacy) {
            recovered.insert(
                legacy.clone(),
                json!({"id":legacy,"name":project.name,"rootPaths":project.roots,
                "createdAt":project.created,"updatedAt":project.updated}),
            );
        }
        mapping.insert(legacy.clone(), Value::String(project.id.clone()));
        allowed.insert(legacy.clone(), project.id.clone());
    }
    set_map(&mut after, "local-projects", recovered.clone());
    if mapping != object(&before[MAP][&host]) {
        if !after[MAP].is_object() {
            after[MAP] = json!({});
        }
        after[MAP][&host] = serde_json::to_value(&mapping).unwrap();
    }
    let mut order = before["project-order"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for id in old["project-order"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(recovered.keys().map(String::as_str))
    {
        if recovered.contains_key(id) && !order.iter().any(|v| v.as_str() == Some(id)) {
            order.push(json!(id));
        }
    }
    if !order.is_empty() || before.get("project-order").is_some() {
        after["project-order"] = json!(order);
    }
    let mut assignments = object(&before[ASSIGN]);
    let mut hosts = object(&before[HOSTS]);
    let current_projectless = projectless(&before);
    let old_projectless = projectless(&old);
    let mut preserved_projectless = ids(&before["projectless-thread-ids"]);
    let mut desired = HashMap::new();
    let mut rows = Vec::new();
    let mut migrated = Vec::new();
    for thread in db_threads {
        check()?;
        if !thread.root
            || selected.is_some_and(|ids| !ids.contains(&thread.id))
            || current_projectless.contains(&thread.id)
            || before[HOSTS][&thread.id]
                .as_str()
                .is_some_and(|host| host != "local")
        {
            continue;
        }
        let existing = assignments.get(&thread.id);
        if existing.is_none() && thread.project.is_none() && old_projectless.contains(&thread.id) {
            preserved_projectless.insert(thread.id.clone());
            continue;
        }
        let pid = if let Some(assignment) = existing {
            if assignment["projectKind"] != "local" {
                continue;
            }
            assignment["projectId"]
                .as_str()
                .filter(|id| allowed.contains_key(*id))
                .map(str::to_string)
        } else if let Some(server) = &thread.project {
            allowed
                .iter()
                .find(|(_, s)| *s == server)
                .map(|(id, _)| id.clone())
        } else if let Some(servers) = catalog_memberships.get(&thread.id) {
            if servers.len() != 1 {
                counts.ambiguous_threads += 1;
                continue;
            }
            let matched = allowed
                .iter()
                .find(|(_, server)| servers.contains(*server))
                .map(|(id, _)| id.clone());
            if matched.is_none() {
                counts.ambiguous_threads += 1;
            }
            matched
        } else {
            let previous = &old[ASSIGN][&thread.id];
            if previous
                .get("projectKind")
                .is_some_and(|kind| kind != "local")
                || old[HOSTS][&thread.id]
                    .as_str()
                    .is_some_and(|host| host != "local")
            {
                continue;
            }
            if previous["projectKind"] == "local"
                && previous["projectId"]
                    .as_str()
                    .is_some_and(|id| allowed.contains_key(id))
            {
                previous["projectId"].as_str().map(str::to_string)
            } else {
                let matches: Vec<_> = allowed
                    .keys()
                    .filter(|id| {
                        thread.cwd.as_deref().is_some_and(|cwd| {
                            root_set(&recovered[*id]["rootPaths"]).contains(&path_key(cwd))
                        })
                    })
                    .cloned()
                    .collect();
                if matches.len() > 1 {
                    counts.ambiguous_threads += 1;
                }
                if matches.len() == 1 {
                    matches.first().cloned()
                } else {
                    None
                }
            }
        };
        let Some(pid) = pid else {
            continue;
        };
        let server = &allowed[&pid];
        if catalog_memberships
            .get(&thread.id)
            .is_some_and(|servers| servers.iter().any(|id| id != server))
        {
            counts.ambiguous_threads += 1;
            continue;
        }
        if thread.project.as_ref().is_some_and(|id| id != server) {
            continue;
        }
        let assignment = json!({"projectKind":"local", "projectId":pid});
        let changed = assignments.get(&thread.id) != Some(&assignment)
            || hosts.get(&thread.id) != Some(&json!("local"));
        counts.assignments += usize::from(changed);
        assignments.insert(thread.id.clone(), assignment);
        hosts.insert(thread.id.clone(), json!("local"));
        desired.insert(thread.id.clone(), server.clone());
        let cwd = thread.cwd.as_deref().map(clean_cwd);
        if thread.project.as_ref() != Some(server) || thread.cwd != cwd {
            rows.push(RowPatch {
                id: thread.id.clone(),
                host: None,
                before_project: thread.project,
                after_project: server.clone(),
                before_cwd: thread.cwd,
                after_cwd: cwd,
            });
            migrated.push(json!(thread.id));
        } else if changed {
            migrated.push(json!(thread.id));
        }
    }
    if preserved_projectless != ids(&before["projectless-thread-ids"]) {
        let mut list: Vec<_> = preserved_projectless.into_iter().collect();
        list.sort();
        after["projectless-thread-ids"] = json!(list);
    }
    set_map(&mut after, ASSIGN, assignments);
    set_map(&mut after, HOSTS, hosts);
    if !migrated.is_empty() {
        if !after[MIGRATION].is_object() {
            after[MIGRATION] = json!({});
        }
        let mut checkpoint = object(&after[MIGRATION][&host]);
        checkpoint.entry("version".into()).or_insert(json!(1));
        checkpoint.insert("projectsMigrated".into(), json!(true));
        checkpoint.insert("threadAssignmentsMigrated".into(), json!(false));
        let mut pending = checkpoint
            .get("pendingThreadAssignmentIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for id in migrated {
            if !pending.contains(&id) {
                pending.push(id);
            }
        }
        checkpoint.insert("pendingThreadAssignmentIds".into(), json!(pending));
        after[MIGRATION][&host] = serde_json::to_value(checkpoint).unwrap();
    }
    counts.thread_rows = rows.len();
    let mut databases = vec![DatabasePatch {
        relative: primary.clone(),
        table: "threads".into(),
        rows,
    }];
    for path in catalog_paths(home, quick) {
        check()?;
        if !path.exists() {
            continue;
        }
        let db = open(&path, false)?;
        let cols = super::table_columns(&db, "local_thread_catalog")?;
        if !["thread_id", "host_id", "project_id", "cwd"]
            .iter()
            .all(|c| cols.contains(*c))
        {
            continue;
        }
        let limit = if quick {
            format!(" LIMIT {}", QUICK_MAX_CATALOG_ROWS + 1)
        } else {
            String::new()
        };
        let mut statement = db
            .prepare(&format!("SELECT thread_id,project_id,cwd FROM local_thread_catalog WHERE host_id='local'{limit}"))
            .map_err(|e| e.to_string())?;
        let mut query = statement.query([]).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        let mut count = 0;
        while let Some(row) = query.next().map_err(|e| e.to_string())? {
            check()?;
            count += 1;
            if quick && count > QUICK_MAX_CATALOG_ROWS {
                return Err("CODEX_PROJECT_QUICK_LIMIT".into());
            }
            let id: String = row.get(0).map_err(|e| e.to_string())?;
            let Some(server) = desired.get(&id) else {
                continue;
            };
            let project: Option<String> = row.get(1).map_err(|e| e.to_string())?;
            let cwd: Option<String> = row.get(2).map_err(|e| e.to_string())?;
            if project.as_ref().is_some_and(|p| p != server) {
                continue;
            }
            let after_cwd = cwd.as_deref().map(clean_cwd);
            if project.as_ref() != Some(server) || cwd != after_cwd {
                rows.push(RowPatch {
                    id,
                    host: Some("local".into()),
                    before_project: project,
                    after_project: server.clone(),
                    before_cwd: cwd,
                    after_cwd,
                });
            }
        }
        counts.catalog_rows += rows.len();
        databases.push(DatabasePatch {
            relative: path
                .strip_prefix(home)
                .map_err(|e| e.to_string())?
                .to_path_buf(),
            table: "local_thread_catalog".into(),
            rows,
        });
    }
    Ok(Plan {
        quick,
        home: home.to_path_buf(),
        before_bytes,
        before_backup: read_bytes(&home.join(format!("{STATE}.bak")))?,
        before,
        after,
        project_fingerprint,
        primary,
        databases,
        counts,
    })
}
