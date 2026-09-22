//! Copy a Codex home without retaining references to the source instance.
//!
//! Rollouts are byte-addressed. Keep them verbatim; relocate only the state DB
//! and host-scoped desktop metadata. Publish the copy only after validation.
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;

const GLOBAL_STATE: &str = ".codex-global-state.json";
const STATE_DB: &str = "state_5.sqlite";
const HOST_MAPS: [&str; 3] = [
    "app-server-project-id-by-legacy-project-id-by-host",
    "app-server-projects-migration-by-host",
    "app-server-migrated-pinned-thread-ids-by-host",
];

pub fn copy_profile(source: &Path, target: &Path) -> Result<(), String> {
    let source_alias = source.to_path_buf();
    let source = source
        .canonicalize()
        .map_err(|e| format!("读取来源实例目录失败: {e}"))?;
    let absolute_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(target)
    };
    if absolute_target.starts_with(&source) || source.starts_with(&absolute_target) {
        return Err("来源与目标实例目录不能重叠".to_string());
    }
    let target = absolute_target.as_path();
    let parent = target.parent().ok_or("目标实例目录没有父目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建实例父目录失败: {e}"))?;
    let target = parent
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(target.file_name().ok_or("目标实例目录无效")?);
    if target.starts_with(&source) || source.starts_with(&target) {
        return Err("来源与目标实例目录不能重叠".to_string());
    }
    if target.exists()
        && (!target.is_dir()
            || fs::read_dir(&target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some())
    {
        return Err("复制来源实例需要目标目录为空".to_string());
    }
    let staging = target.with_file_name(format!(".codex-profile-copy-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        copy_tree(&source, &staging)?;
        relocate_metadata(&source, &source_alias, &absolute_target, &staging)?;
        validate_lineage(&staging)?;
        validate_projection_cursors(&staging, &absolute_target)?;
        // Removing an empty pre-existing directory also fails if another writer used it.
        if target.exists() {
            fs::remove_dir(&target).map_err(|e| format!("目标实例目录已被使用: {e}"))?;
        }
        fs::rename(&staging, &target).map_err(|e| format!("发布实例副本失败: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result.map_err(|e| {
        format!("复制 Codex 实例失败，未发布副本: {e}；若来源实例正在运行，请关闭后重试")
    })
}

fn is_database(path: &Path) -> bool {
    if !matches!(
        path.extension().and_then(|s| s.to_str()),
        Some("sqlite" | "db")
    ) {
        return false;
    }
    let mut header = [0; 16];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .is_ok()
        && &header == b"SQLite format 3\0"
}

fn is_database_sidecar(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    ["-wal", "-shm", "-journal"].iter().any(|suffix| {
        name.strip_suffix(suffix)
            .is_some_and(|base| is_database(&path.with_file_name(base)))
    })
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest = target.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&path, &dest)?;
        } else if kind.is_file() && !is_database_sidecar(&path) {
            if is_database(&path) {
                // VACUUM INTO includes committed WAL pages without copying live WAL/SHM files.
                let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|e| format!("打开来源数据库失败 ({}): {e}", path.display()))?;
                db.busy_timeout(Duration::from_secs(5))
                    .map_err(|e| e.to_string())?;
                db.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])
                    .map_err(|e| format!("创建数据库快照失败 ({}): {e}", path.display()))?;
                fs::set_permissions(
                    &dest,
                    fs::metadata(&path)
                        .map_err(|e| e.to_string())?
                        .permissions(),
                )
                .map_err(|e| e.to_string())?;
            } else {
                fs::copy(&path, &dest)
                    .map_err(|e| format!("复制文件失败 ({}): {e}", path.display()))?;
                if let Ok(time) = fs::metadata(&path).and_then(|m| m.modified()) {
                    let _ = fs::File::open(&dest).and_then(|f| f.set_modified(time));
                }
            }
        }
    }
    Ok(())
}

fn read_state(root: &Path) -> Result<Value, String> {
    let path = root.join(GLOBAL_STATE);
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("解析实例项目配置失败: {e}"))
}

fn has_column(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in names {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn relocate_metadata(
    source: &Path,
    source_alias: &Path,
    target: &Path,
    staging: &Path,
) -> Result<(), String> {
    let mut state = read_state(staging)?;
    let source_host = format!("local:{}", source.display());
    let target_host = format!("local:{}", target.display());
    for key in HOST_MAPS {
        if let Some(map) = state.get_mut(key).and_then(Value::as_object_mut) {
            let alias_host = format!("local:{}", source_alias.display());
            let value = map.remove(&source_host).or_else(|| map.remove(&alias_host));
            if let Some(value) = value {
                map.insert(target_host.clone(), value);
            }
        }
    }
    let db_path = staging.join(STATE_DB);
    if db_path.exists() {
        let mut db = Connection::open(&db_path).map_err(|e| e.to_string())?;
        relocate_threads(
            &mut db,
            source,
            source_alias,
            target,
            staging,
            &state,
            &target_host,
        )
        .map_err(|e| format!("重定位会话索引失败: {e}"))?;
        // Snapshot databases may retain WAL mode. Checkpoint our local edits before publish.
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(|e| e.to_string())?;
    }
    if staging.join(GLOBAL_STATE).exists() {
        fs::write(
            staging.join(GLOBAL_STATE),
            serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn relocate_threads(
    db: &mut Connection,
    source: &Path,
    source_alias: &Path,
    target: &Path,
    staging: &Path,
    state: &Value,
    target_host: &str,
) -> Result<(), String> {
    if !has_column(db, "threads", "rollout_path").map_err(|e| e.to_string())? {
        return Ok(());
    }
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let rows = {
        let mut stmt = tx
            .prepare("SELECT id, rollout_path FROM threads")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    for (id, path) in rows {
        let path = Path::new(&path);
        let relative = path
            .strip_prefix(source)
            .or_else(|_| path.strip_prefix(source_alias))
            .map_err(|_| format!("会话 {id} 的历史文件不在来源实例中"))?;
        if !matches!(relative.components().next(), Some(std::path::Component::Normal(s)) if s == "sessions" || s == "archived_sessions")
            || relative
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || !staging.join(relative).is_file()
        {
            return Err(format!("会话 {id} 的历史文件未完整复制"));
        }
        tx.execute(
            "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
            rusqlite::params![target.join(relative).to_string_lossy().as_ref(), id],
        )
        .map_err(|e| e.to_string())?;
    }
    // Modern projects keep their IDs because the entire state DB is copied.
    // Older desktops kept explicit assignments only in the JSON state. Reconcile
    // unassigned rows using the copied ID map, including projects without roots.
    if has_column(&tx, "threads", "project_id").map_err(|e| e.to_string())?
        && has_column(&tx, "projects", "id").map_err(|e| e.to_string())?
    {
        if let Some(assignments) = state
            .get("thread-project-assignments")
            .and_then(Value::as_object)
        {
            let mapping = state.get(HOST_MAPS[0]).and_then(|v| v.get(target_host));
            for (id, assignment) in assignments {
                if assignment.get("projectKind").and_then(Value::as_str) != Some("local") {
                    continue;
                }
                let Some(legacy_id) = assignment.get("projectId").and_then(Value::as_str) else {
                    continue;
                };
                let Some(project_id) = mapping
                    .and_then(|v| v.get(legacy_id))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                let exists = tx
                    .query_row("SELECT id FROM projects WHERE id = ?1", [project_id], |r| {
                        r.get::<_, String>(0)
                    })
                    .optional()
                    .map_err(|e| e.to_string())?
                    .is_some();
                if exists {
                    tx.execute(
                        "UPDATE threads SET project_id = ?1 WHERE id = ?2 AND project_id IS NULL",
                        [project_id, id],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

#[derive(Clone)]
struct Rollout {
    path: PathBuf,
    base: Option<Value>,
}

fn collect_rollouts(root: &Path, files: &mut HashMap<String, Rollout>) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            collect_rollouts(&path, files)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
            let mut first = Vec::new();
            BufReader::new(fs::File::open(&path).map_err(|e| e.to_string())?)
                .read_until(b'\n', &mut first)
                .map_err(|e| e.to_string())?;
            let Ok(meta) = serde_json::from_slice::<Value>(&first) else {
                continue;
            };
            if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            // Continuation files end in their physical rollout ID while payload.id
            // may still be the root session ID shared by every segment.
            let Some(id) = stem
                .get(stem.len().saturating_sub(36)..)
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            else {
                continue;
            };
            let rollout = Rollout {
                path: path.clone(),
                base: meta
                    .get("payload")
                    .and_then(|p| p.get("history_base"))
                    .filter(|b| !b.is_null())
                    .cloned(),
            };
            if files.insert(id.to_string(), rollout).is_some() {
                return Err(format!("分页历史包含重复的 rollout ID: {id}"));
            }
        }
    }
    Ok(())
}

fn validate_lineage(root: &Path) -> Result<(), String> {
    let mut files = HashMap::new();
    for dir in ["sessions", "archived_sessions"] {
        collect_rollouts(&root.join(dir), &mut files)?;
    }
    let mut checked = std::collections::HashSet::new();
    for (id, rollout) in &files {
        let Some(base) = &rollout.base else {
            continue;
        };
        let parent_id = base
            .get("thread_id")
            .and_then(Value::as_str)
            .ok_or("分页历史缺少来源 ID")?;
        let parent = files
            .get(parent_id)
            .ok_or_else(|| format!("分页历史 {id} 缺少来源文件 {parent_id}"))?;
        let offset = base
            .get("end_byte_offset")
            .and_then(Value::as_u64)
            .ok_or("分页历史缺少字节边界")?;
        let mut file = fs::File::open(&parent.path).map_err(|e| e.to_string())?;
        if offset > file.metadata().map_err(|e| e.to_string())?.len() {
            return Err(format!("分页历史 {id} 的字节边界超出来源文件"));
        }
        if offset > 0 {
            file.seek(SeekFrom::Start(offset - 1))
                .map_err(|e| e.to_string())?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(|e| e.to_string())?;
            if byte[0] != b'\n' {
                return Err(format!("分页历史 {id} 的字节边界不在记录末尾"));
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut current = id.as_str();
        while let Some(next) = files
            .get(current)
            .and_then(|r| r.base.as_ref())
            .and_then(|b| b.get("thread_id"))
            .and_then(Value::as_str)
        {
            if checked.contains(current) {
                break;
            }
            if !seen.insert(current) {
                return Err(format!("分页历史 {id} 存在循环引用"));
            }
            current = next;
        }
        checked.extend(seen);
    }
    Ok(())
}

// Databases and rollout files cannot share a SQLite transaction. Detect a
// projection captured after its rollout snapshot instead of publishing it.
fn validate_projection_cursors(root: &Path, target: &Path) -> Result<(), String> {
    let state_path = root.join(STATE_DB);
    let history_path = root.join("thread_history_1.sqlite");
    if !state_path.exists() || !history_path.exists() {
        return Ok(());
    }
    let state = Connection::open_with_flags(state_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    let history = Connection::open_with_flags(history_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    if !has_column(&state, "threads", "rollout_path").map_err(|e| e.to_string())?
        || !has_column(
            &history,
            "thread_history_projection_state",
            "next_rollout_byte_offset",
        )
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    let mut stmt = history
        .prepare("SELECT thread_id, next_rollout_byte_offset FROM thread_history_projection_state")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (id, offset) = row.map_err(|e| e.to_string())?;
        let path: Option<String> = state
            .query_row("SELECT rollout_path FROM threads WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(path) = path else {
            continue;
        };
        let relative = Path::new(&path)
            .strip_prefix(target)
            .map_err(|e| e.to_string())?;
        let mut file = fs::File::open(root.join(relative)).map_err(|e| e.to_string())?;
        if offset < 0 || offset as u64 > file.metadata().map_err(|e| e.to_string())?.len() {
            return Err(format!("会话 {id} 的历史索引超出副本文件边界"));
        }
        if offset > 0 {
            file.seek(SeekFrom::Start(offset as u64 - 1))
                .map_err(|e| e.to_string())?;
            let mut byte = [0];
            file.read_exact(&mut byte).map_err(|e| e.to_string())?;
            if byte[0] != b'\n' {
                return Err(format!("会话 {id} 的历史索引不在记录末尾"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "codex_profile_copy_tests.rs"]
mod tests;
