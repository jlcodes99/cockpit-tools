const PROJECT_BACKFILL_CURSOR: &str = "usage_project_v1_cursor";
const PROJECT_BACKFILL_END: &str = "usage_project_v1_high_water";
const PROJECT_BACKFILL_COMPLETE: &str = "usage_project_v1_complete";
#[cfg(not(test))]
const USAGE_PROJECTS_UPDATED: &str = "codex-session-usage-projects-updated";

fn ensure_usage_projects(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_usage_projects (
            file_path TEXT PRIMARY KEY REFERENCES session_log_sync(file_path) ON DELETE CASCADE,
            session_id TEXT NOT NULL,
            project_key TEXT NOT NULL,
            project_path TEXT NOT NULL,
            session_title TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_usage_project_key
            ON session_usage_projects(project_key, file_path);
        CREATE INDEX IF NOT EXISTS idx_session_usage_project_session
            ON session_usage_projects(session_id, file_path);",
    )
    .map_err(|error| error.to_string())
}

#[derive(Default)]
struct RolloutUsageProject {
    key: String,
    path: String,
    title: String,
}

fn rollout_usage_project(path: &Path) -> std::io::Result<RolloutUsageProject> {
    rollout_usage_project_with_bytes(path).map(|(project, _)| project)
}

fn rollout_usage_project_with_bytes(path: &Path) -> std::io::Result<(RolloutUsageProject, u64)> {
    let (metadata, bytes) = super::codex_session_rollout_reader::metadata_with_bytes(path)?;
    let Some(metadata) = metadata else {
        return Ok((RolloutUsageProject::default(), bytes));
    };
    let payload = &metadata["payload"];
    let read = |key| {
        payload
            .get(key)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .unwrap_or("")
    };
    let path = read("cwd").replace('\\', "/");
    let key = super::codex_session_display::normalize_path_for_compare(&path).unwrap_or_default();
    let title = read("title");
    let title = if title.is_empty() {
        read("thread_name")
    } else {
        title
    };
    Ok((
        RolloutUsageProject {
            key,
            path: path.trim_end_matches('/').to_owned(),
            title: title.to_owned(),
        },
        bytes,
    ))
}

fn save_usage_project(
    conn: &Connection,
    file_path: &str,
    session_id: &str,
    project: Option<&RolloutUsageProject>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_usage_projects(file_path, session_id, project_key, project_path, session_title)
         VALUES (?1,?2,COALESCE(?3,''),COALESCE(?4,''),COALESCE(?5,''))
         ON CONFLICT(file_path) DO UPDATE SET
         session_id=excluded.session_id,
         project_key=COALESCE(?3,project_key), project_path=COALESCE(?4,project_path),
         session_title=COALESCE(?5,session_title)
         WHERE session_id<>excluded.session_id OR
         (?3 IS NOT NULL AND (project_key<>?3 OR project_path<>?4 OR session_title<>?5))",
        params![
            file_path, session_id,
            project.map(|p| &p.key), project.map(|p| &p.path), project.map(|p| &p.title),
        ],
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn usage_projects_complete(conn: &Connection) -> bool {
    get_meta_i64(conn, PROJECT_BACKFILL_COMPLETE) == Some(1)
}

#[cfg(not(test))]
fn schedule_usage_projects(path: PathBuf) {
    crate::modules::background_maintenance::schedule(
        format!("session-usage-projects:{}", path.display()),
        move || {
            let Ok(_guard) = sync_lock().try_lock() else {
                return Ok(false);
            };
            let conn =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|error| error.to_string())?;
            conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=10;")
                .map_err(|error| error.to_string())?;
            if usage_projects_complete(&conn) {
                return Ok(true);
            }
            let complete = backfill_usage_projects(&conn)?;
            if complete {
                tracing::info!("[CODEX-SESSION-USAGE] 项目索引回填完成");
                if let Some(app) = crate::get_app_handle() {
                    use tauri::Emitter;
                    let _ = app.emit(USAGE_PROJECTS_UPDATED, ());
                }
            }
            Ok(complete)
        },
    );
}

fn backfill_usage_projects(conn: &Connection) -> Result<bool, String> {
    if usage_projects_complete(conn) {
        return Ok(true);
    }
    let end = match get_meta_i64(conn, PROJECT_BACKFILL_END) {
        Some(end) => end,
        None => {
            let end: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(rowid),0) FROM session_log_sync",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            set_meta(conn, PROJECT_BACKFILL_END, &end.to_string())?;
            end
        }
    };
    let cursor = get_meta_i64(conn, PROJECT_BACKFILL_CURSOR).unwrap_or(0);
    use super::codex_session_rollout_reader::METADATA_LIMIT;
    use crate::modules::background_maintenance::{BATCH_BYTES, BATCH_ROWS, BATCH_TIME};
    // The shared maintenance lane bounds cursor rows and decompressed bytes.
    let batch = {
        let mut statement = conn
            .prepare(
                "SELECT s.rowid,s.file_path,p.file_path IS NOT NULL
             FROM session_log_sync s LEFT JOIN session_usage_projects p USING(file_path)
             WHERE s.rowid>?1 AND s.rowid<=?2 ORDER BY s.rowid LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![cursor, end, BATCH_ROWS as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
    };
    if batch.is_empty() {
        set_meta(conn, PROJECT_BACKFILL_COMPLETE, "1")?;
        return Ok(true);
    }

    let start = std::time::Instant::now();
    let mut next = cursor;
    let mut updates = Vec::new();
    let mut bytes = 0;
    for (rowid, file_path, indexed) in batch {
        if !indexed {
            let path = Path::new(&file_path);
            let project = rollout_usage_project_with_bytes(path).or_else(|error| {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error);
                }
                rollout_usage_project_with_bytes(&path.with_extension("jsonl.zst"))
            });
            let project = match project {
                Ok((project, read)) => {
                    bytes += read;
                    Some(project)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    bytes += METADATA_LIMIT + 1;
                    tracing::warn!(
                        "[CODEX-SESSION-USAGE] 项目索引回填跳过 {}: {error}",
                        path.display()
                    );
                    None
                }
            };
            let session_id: String = conn
                .query_row(
                    "SELECT COALESCE(
                    (SELECT session_id FROM session_usage_events WHERE file_path=?1 LIMIT 1),
                    (SELECT session_id FROM session_rewrite_events WHERE file_path=?1 LIMIT 1),'')",
                    [&file_path],
                    |row| row.get(0),
                )
                .map_err(|error| error.to_string())?;
            updates.push((file_path, session_id, project));
        }
        next = rowid;
        if bytes + METADATA_LIMIT >= BATCH_BYTES || start.elapsed() >= BATCH_TIME {
            break;
        }
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    for (file_path, session_id, project) in updates {
        save_usage_project(&tx, &file_path, &session_id, project.as_ref())?;
    }
    set_meta(&tx, PROJECT_BACKFILL_CURSOR, &next.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(false)
}
