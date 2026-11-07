fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("CODEX_PROJECT_PATH_INVALID")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn atomic_value(path: &Path, value: &impl Serialize) -> Result<(), String> {
    atomic_bytes(path, &serde_json::to_vec(value).map_err(|e| e.to_string())?)
}

fn validate(plan: &Plan, home: &Path) -> Result<(), String> {
    if path_key(&plan.home.to_string_lossy()) != path_key(&home.to_string_lossy()) {
        return Err("CODEX_PROJECT_JOURNAL_INVALID".into());
    }
    for db in &plan.databases {
        if db.relative.is_absolute()
            || db
                .relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !["threads", "local_thread_catalog"].contains(&db.table.as_str())
        {
            return Err("CODEX_PROJECT_JOURNAL_INVALID".into());
        }
    }
    Ok(())
}

fn update_rows(db: &Connection, patch: &DatabasePatch, undo: bool) -> Result<(), String> {
    let id_col = if patch.table == "threads" {
        "id"
    } else {
        "thread_id"
    };
    let host_clause = if patch.table == "threads" {
        ""
    } else {
        " AND host_id='local'"
    };
    let sql = format!("UPDATE {} SET project_id=?1,cwd=?2 WHERE {id_col}=?3 AND project_id IS ?4 AND cwd IS ?5{host_clause}", patch.table);
    for row in &patch.rows {
        if !undo {
            check()?;
        }
        let after = Some(row.after_project.as_str());
        let (new_project, new_cwd, old_project, old_cwd) = if undo {
            (
                row.before_project.as_deref(),
                row.before_cwd.as_deref(),
                after,
                row.after_cwd.as_deref(),
            )
        } else {
            (
                after,
                row.after_cwd.as_deref(),
                row.before_project.as_deref(),
                row.before_cwd.as_deref(),
            )
        };
        let changed = db
            .execute(
                &sql,
                rusqlite::params![new_project, new_cwd, row.id, old_project, old_cwd],
            )
            .map_err(|e| e.to_string())?;
        if !undo && changed != 1 {
            return Err("CODEX_PROJECT_CHANGED".into());
        }
    }
    if patch.table == "local_thread_catalog"
        && !patch.rows.is_empty()
        && super::table_columns(db, "local_thread_catalog_metadata")?.contains("catalog_revision")
    {
        db.execute(
            "UPDATE local_thread_catalog_metadata SET catalog_revision=catalog_revision+1",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Restore only values still equal to this repair's output. Recurse through maps
// so a new chat/assignment added later does not prevent unrelated field recovery.
fn undo_value(current: &mut Value, before: &Value, after: &Value) {
    if before == after {
        return;
    }
    if current == after {
        *current = before.clone();
        return;
    }
    let (Some(current), Some(after)) = (current.as_object_mut(), after.as_object()) else {
        return;
    };
    for (key, written) in after {
        let original = before.get(key).unwrap_or(&Value::Null);
        if original == written {
            continue;
        }
        if let Some(value) = current.get_mut(key) {
            undo_value(value, original, written);
            if value.is_null() && before.get(key).is_none() {
                current.remove(key);
            }
        }
    }
}

fn restore_state(path: &Path, before: Option<&[u8]>, after: &Value) -> Result<(), String> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(());
    };
    let mut current = decode(Some(&bytes))?;
    let original = decode(before)?;
    undo_value(&mut current, &original, after);
    if before.is_none() && current == json!({}) {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    } else {
        atomic_value(path, &current)?;
    }
    Ok(())
}

fn rollback(plan: &Plan) -> Result<(), String> {
    // Whole SQLite backup replacement would destroy chats created after repair.
    for patch in &plan.databases {
        if patch.rows.is_empty() {
            continue;
        }
        let db = open(&plan.home.join(&patch.relative), true)?;
        db.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        update_rows(&db, patch, true)?;
        db.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    }
    restore_state(
        &plan.home.join(STATE),
        plan.before_bytes.as_deref(),
        &plan.after,
    )?;
    restore_state(
        &plan.home.join(format!("{STATE}.bak")),
        plan.before_backup.as_deref(),
        &plan.after,
    )?;
    Ok(())
}

pub(super) fn has_pending(home: &Path) -> bool {
    home.join(JOURNAL).exists()
}

// Called only after stopped-target verification, before a fresh manual scan.
pub(super) fn recover_pending(home: &Path) -> Result<(), String> {
    let Some(bytes) = read_bytes(&home.join(JOURNAL))? else {
        return Ok(());
    };
    let backup: PathBuf = serde_json::from_slice(&bytes)
        .map_err(|e| format!("CODEX_PROJECT_JOURNAL_INVALID: {e}"))?;
    let bytes = fs::read(backup.join("project-recovery-plan.json")).map_err(|e| e.to_string())?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate(&plan, home)?;
    super::without_launch_visibility_budget(|| rollback(&plan))?;
    fs::remove_file(home.join(JOURNAL)).map_err(|e| e.to_string())
}

pub(super) fn apply(plan: &Plan, backup: &Path) -> Result<(), String> {
    validate(plan, &plan.home)?;
    check()?;
    if read_bytes(&plan.home.join(STATE))? != plan.before_bytes
        || read_bytes(&plan.home.join(format!("{STATE}.bak")))? != plan.before_backup
    {
        return Err("CODEX_PROJECT_CHANGED".into());
    }
    fs::create_dir_all(backup).map_err(|e| e.to_string())?;
    if let Some(bytes) = &plan.before_bytes {
        atomic_bytes(&backup.join(STATE), bytes)?;
    }
    if let Some(bytes) = &plan.before_backup {
        atomic_bytes(&backup.join(format!("{STATE}.bak")), bytes)?;
    }
    let mut grouped: BTreeMap<PathBuf, Vec<&DatabasePatch>> = BTreeMap::new();
    grouped.entry(plan.primary.clone()).or_default();
    for patch in &plan.databases {
        if !patch.rows.is_empty() {
            grouped
                .entry(patch.relative.clone())
                .or_default()
                .push(patch);
        }
    }
    // Quick repair backs up exact before/after fields in the durable plan and
    // JSON copies. Whole-database VACUUM backups belong to deep maintenance.
    for relative in grouped.keys().filter(|_| !plan.quick) {
        check()?;
        let db = open(&plan.home.join(relative), false)?;
        let target = backup.join("db").join(relative);
        fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        db.execute("VACUUM main INTO ?1", [target.to_string_lossy().as_ref()])
            .map_err(|e| e.to_string())?;
        let saved = open(&target, false)?;
        if saved
            .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            != "ok"
        {
            return Err("CODEX_PROJECT_BACKUP_INVALID".into());
        }
    }
    atomic_value(&backup.join("project-recovery-plan.json"), plan)?;
    // Durable intent precedes both SQLite and JSON writes. A process crash can
    // interrupt a multi-file commit; the next explicit repair compensates it.
    atomic_value(&plan.home.join(JOURNAL), &backup)?;
    let mut connections = Vec::new();
    let result = (|| {
        for (relative, patches) in &grouped {
            let db = open(&plan.home.join(relative), true)?;
            db.execute_batch("BEGIN IMMEDIATE")
                .map_err(|e| e.to_string())?;
            if relative == &plan.primary
                && fingerprint(&projects_with_limit(
                    &db,
                    plan.quick.then_some(QUICK_MAX_PROJECTS),
                )?) != plan.project_fingerprint
            {
                return Err("CODEX_PROJECT_CHANGED".into());
            }
            if relative == &plan.primary {
                let roots: HashSet<_> =
                    threads_with_limit(&db, plan.quick.then_some(QUICK_MAX_THREADS))?
                        .into_iter()
                        .filter(|t| t.root)
                        .map(|t| t.id)
                        .collect();
                if patches
                    .iter()
                    .any(|p| p.table == "threads" && p.rows.iter().any(|r| !roots.contains(&r.id)))
                {
                    return Err("CODEX_PROJECT_CHANGED".into());
                }
            }
            for patch in patches {
                update_rows(&db, patch, false)?;
            }
            connections.push(db);
        }
        if read_bytes(&plan.home.join(STATE))? != plan.before_bytes
            || read_bytes(&plan.home.join(format!("{STATE}.bak")))? != plan.before_backup
        {
            return Err("CODEX_PROJECT_CHANGED".into());
        }
        check()?;
        atomic_value(&plan.home.join(STATE), &plan.after)?;
        atomic_value(&plan.home.join(format!("{STATE}.bak")), &plan.after)?;
        for db in &connections {
            db.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    // Release all remaining write transactions before conditional compensation.
    drop(connections);
    if let Err(error) = result {
        if let Err(rollback_error) = super::without_launch_visibility_budget(|| rollback(plan)) {
            return Err(format!(
                "CODEX_PROJECT_ROLLBACK_FAILED: {error}; {rollback_error}; backup={}",
                backup.display()
            ));
        }
        fs::remove_file(plan.home.join(JOURNAL)).map_err(|e| e.to_string())?;
        return Err(error);
    }
    atomic_value(
        &backup.join("result.json"),
        &json!({"status":"applied","counts":plan.counts}),
    )?;
    fs::remove_file(plan.home.join(JOURNAL)).map_err(|e| e.to_string())
}
