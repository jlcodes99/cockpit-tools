//! Copy a Codex home without retaining references to the source instance.
//!
//! Rollouts are byte-addressed. Keep them verbatim; relocate only the state DB
//! and host-scoped desktop metadata. Publish the copy only after validation.
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
    copy_profile_with_cancellation(source, target, &AtomicBool::new(false))
}

struct CopyBudget<'a> {
    deadline: Instant,
    cancelled: &'a AtomicBool,
}

impl CopyBudget<'_> {
    fn check(&self) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline {
            return Err("复制实例已取消或超时，请重试".to_string());
        }
        Ok(())
    }
}

pub fn copy_profile_with_cancellation(
    source: &Path,
    target: &Path,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let budget = CopyBudget {
        deadline: Instant::now() + Duration::from_secs(90),
        cancelled,
    };
    budget.check()?;
    #[cfg(windows)]
    let normalized_target = PathBuf::from(target.to_string_lossy().replace('/', "\\"));
    #[cfg(windows)]
    let target = normalized_target.as_path();
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
    validate_target_parent_before_creation(&source, parent)?;
    fs::create_dir_all(parent).map_err(|e| format!("创建实例父目录失败: {e}"))?;
    let target = parent
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(target.file_name().ok_or("目标实例目录无效")?);
    if target.starts_with(&source) || source.starts_with(&target) {
        return Err("来源与目标实例目录不能重叠".to_string());
    }
    if fs::symlink_metadata(&target).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("目标实例目录不能是链接".to_string());
    }
    let target_permissions = if target.exists() {
        if !target.is_dir()
            || fs::read_dir(&target)
                .map_err(|e| e.to_string())?
                .next()
                .is_some()
        {
            return Err("复制来源实例需要目标目录为空".to_string());
        }
        Some(
            fs::metadata(&target)
                .map_err(|e| e.to_string())?
                .permissions(),
        )
    } else {
        None
    };
    let staging = target.with_file_name(format!(".codex-profile-copy-{}", uuid::Uuid::new_v4()));
    // The sibling staging directory is not protected by the target's permissions.
    // Restrict it at creation, before any profile data can be copied into it.
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&staging)
    };
    #[cfg(not(unix))]
    let created = fs::create_dir(&staging);
    // Do not clean up a path we did not successfully create.
    created.map_err(|e| format!("创建实例临时目录失败: {e}"))?;
    let mut removed_target = false;
    let result = (|| {
        copy_tree(&source, &staging, &budget)?;
        budget.check()?;
        relocate_metadata(&source, &source_alias, &absolute_target, &staging, &budget)?;
        validate_lineage(&staging, &budget)?;
        validate_projection_cursors(&staging, &absolute_target, &budget)?;
        budget.check()?;
        // Preserve the existing target's mode when replacing its directory.
        // A newly created Unix target retains the private staging permissions.
        if let Some(permissions) = &target_permissions {
            fs::set_permissions(&staging, permissions.clone())
                .map_err(|e| format!("保留目标实例目录权限失败: {e}"))?;
        }
        // Removing an empty pre-existing directory also fails if another writer used it.
        if target.exists() {
            fs::remove_dir(&target).map_err(|e| format!("目标实例目录已被使用: {e}"))?;
            removed_target = true;
        }
        fs::rename(&staging, &target).map_err(|e| format!("发布实例副本失败: {e}"))
    })();
    if result.is_err() {
        // A target such as 0500 may have made staging non-writable before a
        // failed publish. Restore owner access so its contents can be removed.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&staging, fs::Permissions::from_mode(0o700));
        }
        // Only clean the private directory created by this copy, within the checked parent.
        if staging
            .canonicalize()
            .is_ok_and(|path| path.parent() == target.parent())
        {
            let _ = fs::remove_dir_all(&staging);
        }
        if removed_target && !target.exists() {
            if fs::create_dir(&target).is_ok() {
                if let Some(permissions) = &target_permissions {
                    let _ = fs::set_permissions(&target, permissions.clone());
                }
            }
        }
    }
    result.map_err(|e| {
        format!("复制 Codex 实例失败，未发布副本: {e}；若来源实例正在运行，请关闭后重试")
    })
}

fn validate_target_parent_before_creation(source: &Path, parent: &Path) -> Result<(), String> {
    // Windows can resolve missing/.. without reporting the missing component.
    // Check the lexical traversal before any mkdir can create that component.
    #[cfg(windows)]
    let lexical_parent = PathBuf::from(normalize_windows_profile_path(parent));
    #[cfg(not(windows))]
    let lexical_parent = parent.to_path_buf();
    let mut prefix = PathBuf::new();
    let mut missing_component = false;
    for component in lexical_parent.components() {
        if component == Component::ParentDir && missing_component {
            return Err(
                "目标父目录包含尚未创建的路径和 ..，请先创建父目录或使用不含 .. 的路径".to_string(),
            );
        }
        prefix.push(component.as_os_str());
        if component != Component::ParentDir && !prefix.exists() {
            missing_component = true;
        }
    }
    for ancestor in parent.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                // Resolve aliases before mkdir can write into the source. A
                // dangling link must fail here, not be treated as a missing directory.
                let resolved = ancestor.canonicalize().map_err(|e| e.to_string())?;
                if resolved.starts_with(source) {
                    return Err("来源与目标实例目录不能重叠".to_string());
                }
                let missing = parent.strip_prefix(ancestor).map_err(|e| e.to_string())?;
                // Normalizing missing/../source would hide directories that
                // create_dir_all may create while traversing that path.
                if missing
                    .components()
                    .any(|part| part == Component::ParentDir)
                {
                    return Err(
                        "目标父目录包含尚未创建的路径和 ..，请先创建父目录或使用不含 .. 的路径"
                            .to_string(),
                    );
                }
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("无法解析目标实例父目录".to_string())
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

fn copy_tree(source: &Path, target: &Path, budget: &CopyBudget<'_>) -> Result<(), String> {
    budget.check()?;
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        budget.check()?;
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest = target.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_tree(&path, &dest, budget)?;
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
    budget: &CopyBudget<'_>,
) -> Result<(), String> {
    budget.check()?;
    let mut state = read_state(staging)?;
    let source_host = format!("local:{}", source.display());
    let target_host = format!("local:{}", target.display());
    for key in HOST_MAPS {
        if let Some(map) = state.get_mut(key).and_then(Value::as_object_mut) {
            let alias_host = format!("local:{}", source_alias.display());
            let canonical_value = map.remove(&source_host);
            let mut value = map.remove(&alias_host).or(canonical_value);
            let aliases = map
                .keys()
                .filter(|key| {
                    key.strip_prefix("local:").is_some_and(|home| {
                        relative_profile_path(Path::new(home), source)
                            .is_some_and(|relative| relative.as_os_str().is_empty())
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            for alias in aliases {
                let removed = map.remove(&alias);
                if value.is_none() {
                    value = removed;
                }
            }
            if let Some(value) = value {
                map.insert(target_host.clone(), value);
            }
        }
    }
    let db_path = staging.join(STATE_DB);
    if db_path.exists() {
        let mut db = Connection::open(&db_path).map_err(|e| e.to_string())?;
        db.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        relocate_threads(
            &mut db,
            source,
            source_alias,
            target,
            staging,
            &state,
            &target_host,
            budget,
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

#[cfg(windows)]
fn normalize_windows_profile_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    if text
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &text[8..])
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }
}

// Windows persisted paths may use drive-letter case, forward slashes or omit
// the canonical verbatim prefix. Match those spellings without accepting siblings.
fn relative_profile_path(path: &Path, root: &Path) -> Option<PathBuf> {
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.to_path_buf());
    }
    #[cfg(windows)]
    {
        let path = normalize_windows_profile_path(path);
        let root = normalize_windows_profile_path(root);
        let root = root.trim_end_matches('\\');
        if path.eq_ignore_ascii_case(root) {
            return Some(PathBuf::new());
        }
        if path
            .get(..root.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(root))
            && path.as_bytes().get(root.len()) == Some(&b'\\')
        {
            return Some(PathBuf::from(&path[root.len() + 1..]));
        }
    }
    None
}

fn relocate_threads(
    db: &mut Connection,
    source: &Path,
    source_alias: &Path,
    target: &Path,
    staging: &Path,
    state: &Value,
    target_host: &str,
    budget: &CopyBudget<'_>,
) -> Result<(), String> {
    budget.check()?;
    if !has_column(db, "threads", "rollout_path").map_err(|e| e.to_string())? {
        return Ok(());
    }
    let tx = db.transaction().map_err(|e| e.to_string())?;
    let mut after_id: Option<String> = None;
    loop {
        budget.check()?;
        let rows = {
            let mut stmt = tx
            .prepare("SELECT id, rollout_path FROM threads WHERE ?1 IS NULL OR id > ?1 ORDER BY id LIMIT 512")
            .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([after_id.as_deref()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        if rows.is_empty() {
            break;
        }
        after_id = rows.last().map(|(id, _)| id.clone());
        for (id, path) in rows {
            budget.check()?;
            let path = Path::new(&path);
            let relative = relative_profile_path(path, source)
                .or_else(|| relative_profile_path(path, source_alias))
                .ok_or_else(|| format!("会话 {id} 的历史文件不在来源实例中"))?;
            let history_directory = relative.components().next().is_some_and(|component| {
                let Component::Normal(name) = component else {
                    return false;
                };
                #[cfg(windows)]
                {
                    let name = name.to_string_lossy();
                    name.eq_ignore_ascii_case("sessions")
                        || name.eq_ignore_ascii_case("archived_sessions")
                }
                #[cfg(not(windows))]
                {
                    name == "sessions" || name == "archived_sessions"
                }
            });
            if !history_directory
                || relative
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || !staging.join(&relative).is_file()
            {
                return Err(format!("会话 {id} 的历史文件未完整复制"));
            }
            tx.execute(
                "UPDATE threads SET rollout_path = ?1 WHERE id = ?2",
                rusqlite::params![target.join(&relative).to_string_lossy().as_ref(), id],
            )
            .map_err(|e| e.to_string())?;
        }
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
                budget.check()?;
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
    budget.check()?;
    tx.commit().map_err(|e| e.to_string())
}

#[derive(Clone)]
struct Rollout {
    path: PathBuf,
    base: Option<Value>,
}

fn collect_rollouts(
    root: &Path,
    files: &mut HashMap<String, Rollout>,
    budget: &CopyBudget<'_>,
) -> Result<(), String> {
    budget.check()?;
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        budget.check()?;
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            collect_rollouts(&path, files, budget)?;
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

fn validate_lineage(root: &Path, budget: &CopyBudget<'_>) -> Result<(), String> {
    let mut files = HashMap::new();
    for dir in ["sessions", "archived_sessions"] {
        collect_rollouts(&root.join(dir), &mut files, budget)?;
    }
    let mut checked = std::collections::HashSet::new();
    for (id, rollout) in &files {
        budget.check()?;
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

// Derived thread-history caches are validated/recovered only inside the private staging copy.
include!("codex_profile_copy_projection.rs");

#[cfg(test)]
#[path = "codex_profile_copy_tests.rs"]
mod tests;
