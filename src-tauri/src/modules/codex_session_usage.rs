//! 从 Codex 会话 JSONL（rollout）汇总真实 Token 用量。
//!
//! 算法对齐 cc-switch：优先 `last_token_usage`，按完整快照签名去重，
//! 缺 last 时回退 `total_token_usage` 高水位差；分叉会话跳过父 rollout
//! 在 fork 时刻之前的重放前缀。
//!
//! 数据与官方配额、API 服务 `request_logs` 完全隔离，打开用量面板时才扫描。

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::modules::{account, codex_instance};

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DB_FILE_NAME: &str = "codex_session_usage.sqlite";
const REQUEST_ID_PREFIX: &str = "codex_session:thread-v1";
const INSERT_BATCH_SIZE: usize = 500;

static SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static REPLAY_CACHE: OnceLock<Mutex<ReplayCaches>> = OnceLock::new();

fn sync_lock() -> &'static Mutex<()> {
    SYNC_LOCK.get_or_init(|| Mutex::new(()))
}

fn replay_caches() -> &'static Mutex<ReplayCaches> {
    REPLAY_CACHE.get_or_init(|| Mutex::new(ReplayCaches::default()))
}

#[derive(Debug, Clone)]
pub struct UsageInstance {
    pub id: String,
    pub name: String,
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageBreakdownRow {
    pub key: String,
    pub label: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageInstanceOption {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageReport {
    pub totals: CodexSessionUsageTotals,
    pub by_model: Vec<CodexSessionUsageBreakdownRow>,
    pub by_instance: Vec<CodexSessionUsageBreakdownRow>,
    pub by_day: Vec<CodexSessionUsageBreakdownRow>,
    pub instances: Vec<CodexSessionUsageInstanceOption>,
    pub from_timestamp: Option<i64>,
    pub to_timestamp: Option<i64>,
    pub last_synced_at: Option<i64>,
    pub files_tracked: u64,
    pub event_count: u64,
    pub deferred_files: u32,
    pub last_error_count: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub files_changed: u32,
    pub deferred_files: u32,
    pub errors: Vec<String>,
    pub rebuilt: bool,
    pub report: Option<CodexSessionUsageReport>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionUsageQuery {
    pub from_timestamp: Option<i64>,
    pub to_timestamp: Option<i64>,
    pub instance_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    output: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct DeltaTokens {
    input: u64,
    cached_input: u64,
    output: u64,
}

impl DeltaTokens {
    fn is_zero(self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.output == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenCountersSignature {
    input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
    reasoning_output: Option<u64>,
    total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenUsageSignature {
    total: Option<TokenCountersSignature>,
    last: Option<TokenCountersSignature>,
}

#[derive(Debug, Clone)]
struct TimestampedTokenSignature {
    timestamp: DateTime<Utc>,
    signature: TokenUsageSignature,
}

#[derive(Debug, Default)]
struct ParentTokenTimeline {
    events: Vec<TimestampedTokenSignature>,
    has_token_without_timestamp: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParentFileStamp {
    modified_nanos: i64,
    size: u64,
}

#[derive(Debug)]
struct CachedParentTimeline {
    stamp: ParentFileStamp,
    timeline: ParentTokenTimeline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingReason {
    MissingParent(String),
    Stable(String),
    Retryable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEntry {
    modified: i64,
    size: u64,
    reason: PendingReason,
}

#[derive(Debug, Default)]
struct ReplayCaches {
    parent_timelines: HashMap<PathBuf, CachedParentTimeline>,
    pending: HashMap<PathBuf, PendingEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentResolution {
    None,
    Parent,
    Deferred,
}

#[derive(Debug)]
struct ParsedTokenEvent {
    line_offset: i64,
    signature: TokenUsageSignature,
    delta: DeltaTokens,
    event_index: Option<u32>,
    model: String,
    timestamp: Option<i64>,
}

#[derive(Debug)]
struct ParsedCodexFile {
    root_thread_id: Option<String>,
    root_meta_seen: bool,
    root_timestamp: Option<DateTime<Utc>>,
    parent_id: Option<String>,
    parent: ParentResolution,
    deferred_reason: Option<String>,
    token_events: Vec<ParsedTokenEvent>,
    line_offset: i64,
    has_billable_tokens: bool,
}

#[derive(Debug, Default)]
struct FileSyncResult {
    imported: u32,
    skipped: u32,
    deferred: bool,
}

pub fn query_session_usage(
    query: CodexSessionUsageQuery,
) -> Result<CodexSessionUsageReport, String> {
    let store = SessionUsageStore::open_default()?;
    store.query(&query)
}

pub fn sync_session_usage(
    rebuild: bool,
    query: CodexSessionUsageQuery,
) -> Result<CodexSessionUsageSyncResult, String> {
    let _guard = sync_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let store = SessionUsageStore::open_default()?;
    let instances = collect_usage_instances()?;
    let mut result = store.sync(rebuild, &instances)?;
    result.report = Some(store.query(&query)?);
    Ok(result)
}

struct SessionUsageStore {
    db_path: PathBuf,
}

impl SessionUsageStore {
    fn open_default() -> Result<Self, String> {
        Ok(Self {
            db_path: usage_db_path()?,
        })
    }

    #[cfg(test)]
    fn open_path(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    fn open_conn(&self) -> Result<Connection, String> {
        if let Some(parent) = self.db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建会话用量目录失败: {error}"))?;
        }
        let conn = Connection::open(&self.db_path)
            .map_err(|error| format!("打开会话用量库失败: {error}"))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS session_log_sync (
                file_path TEXT PRIMARY KEY,
                instance_id TEXT NOT NULL,
                last_modified INTEGER NOT NULL,
                last_size INTEGER NOT NULL,
                last_line_offset INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_usage_events (
                request_id TEXT PRIMARY KEY,
                instance_id TEXT NOT NULL,
                instance_name TEXT NOT NULL,
                session_id TEXT NOT NULL,
                model TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                file_path TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_usage_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_session_usage_timestamp
                ON session_usage_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_session_usage_model
                ON session_usage_events(model, timestamp);
            CREATE INDEX IF NOT EXISTS idx_session_usage_instance
                ON session_usage_events(instance_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_session_usage_file
                ON session_usage_events(file_path);
            ",
        )
        .map_err(|error| format!("初始化会话用量库失败: {error}"))?;
        Ok(conn)
    }

    fn sync(
        &self,
        rebuild: bool,
        instances: &[UsageInstance],
    ) -> Result<CodexSessionUsageSyncResult, String> {
        let mut conn = self.open_conn()?;
        if rebuild {
            conn.execute_batch(
                "
                DELETE FROM session_usage_events;
                DELETE FROM session_log_sync;
                DELETE FROM session_usage_meta;
                ",
            )
            .map_err(|error| format!("清空会话用量缓存失败: {error}"))?;
            if let Ok(mut caches) = replay_caches().lock() {
                *caches = ReplayCaches::default();
            }
        }

        let mut cursors = load_cursors(&conn)?;
        let mut result = CodexSessionUsageSyncResult {
            rebuilt: rebuild,
            ..CodexSessionUsageSyncResult::default()
        };

        for instance in instances {
            let files = collect_codex_session_files(&instance.data_dir);
            let rollout_index = build_rollout_index(&files);
            result.files_scanned = result.files_scanned.saturating_add(files.len() as u32);

            for file_path in &files {
                match sync_single_file(&mut conn, instance, file_path, &rollout_index, &mut cursors)
                {
                    Ok(file_result) => {
                        result.imported = result.imported.saturating_add(file_result.imported);
                        result.skipped = result.skipped.saturating_add(file_result.skipped);
                        if file_result.imported > 0 || file_result.skipped > 0 {
                            result.files_changed = result.files_changed.saturating_add(1);
                        }
                        if file_result.deferred {
                            result.deferred_files = result.deferred_files.saturating_add(1);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            "[CODEX-SESSION-USAGE] 解析失败 {}: {error}",
                            file_path.display()
                        );
                        if result.errors.len() < 20 {
                            result
                                .errors
                                .push(format!("{}: {error}", file_path.display()));
                        }
                    }
                }
            }
        }

        set_meta(&conn, "last_synced_at", &now_unix_seconds().to_string())?;
        set_meta(&conn, "last_error_count", &result.errors.len().to_string())?;
        set_meta(
            &conn,
            "last_deferred_files",
            &result.deferred_files.to_string(),
        )?;
        Ok(result)
    }

    fn query(&self, query: &CodexSessionUsageQuery) -> Result<CodexSessionUsageReport, String> {
        let conn = self.open_conn()?;
        let instance_names = current_instance_names();
        let instance_filter = query
            .instance_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let mut where_sql = String::from("WHERE 1 = 1");
        let mut params: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(from) = query.from_timestamp {
            where_sql.push_str(" AND timestamp >= ?");
            params.push(rusqlite::types::Value::Integer(from));
        }
        if let Some(to) = query.to_timestamp {
            where_sql.push_str(" AND timestamp <= ?");
            params.push(rusqlite::types::Value::Integer(to));
        }
        if let Some(instance_id) = instance_filter {
            where_sql.push_str(" AND instance_id = ?");
            params.push(rusqlite::types::Value::Text(instance_id.to_string()));
        }

        let totals = query_totals(&conn, &where_sql, &params)?;
        let by_model = query_breakdown(&conn, &where_sql, &params, "model", None, &instance_names)?;
        let by_instance = query_breakdown(
            &conn,
            &where_sql,
            &params,
            "instance_id",
            Some("instance_name"),
            &instance_names,
        )?;
        let mut by_day = query_day_breakdown(&conn, &where_sql, &params)?;
        by_day.sort_by(|left, right| right.key.cmp(&left.key));
        let estimated_cost_usd = estimate_usage_cost_usd(&by_model);
        let totals = CodexSessionUsageTotals {
            estimated_cost_usd,
            ..totals
        };

        let files_tracked = conn
            .query_row("SELECT COUNT(*) FROM session_log_sync", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as u64;
        let event_count = conn
            .query_row("SELECT COUNT(*) FROM session_usage_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as u64;

        Ok(CodexSessionUsageReport {
            totals,
            by_model,
            by_instance,
            by_day,
            instances: collect_usage_instances()
                .unwrap_or_default()
                .into_iter()
                .map(|instance| CodexSessionUsageInstanceOption {
                    id: instance.id,
                    name: instance.name,
                })
                .collect(),
            from_timestamp: query.from_timestamp,
            to_timestamp: query.to_timestamp,
            last_synced_at: get_meta_i64(&conn, "last_synced_at"),
            files_tracked,
            event_count,
            deferred_files: get_meta_i64(&conn, "last_deferred_files")
                .unwrap_or(0)
                .max(0) as u32,
            last_error_count: get_meta_i64(&conn, "last_error_count").unwrap_or(0).max(0) as u32,
        })
    }
}

fn usage_db_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(DB_FILE_NAME))
}

fn collect_usage_instances() -> Result<Vec<UsageInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = codex_instance::get_default_codex_home()?;
    let store = codex_instance::load_instance_store()?;
    instances.push(UsageInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
    });
    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(UsageInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
        });
    }
    Ok(instances)
}

fn current_instance_names() -> HashMap<String, String> {
    collect_usage_instances()
        .unwrap_or_default()
        .into_iter()
        .map(|instance| (instance.id, instance.name))
        .collect()
}

fn collect_codex_session_files(codex_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_jsonl_recursive(&sessions_dir, &mut files, 0, 3);
    }
    let archived_dir = codex_dir.join("archived_sessions");
    if archived_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&archived_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_rollout_path(&path) {
                    files.push(path);
                }
            }
        }
    }
    files.sort();
    files
}

fn collect_jsonl_recursive(dir: &Path, files: &mut Vec<PathBuf>, depth: u32, max_depth: u32) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < max_depth {
            collect_jsonl_recursive(&path, files, depth + 1, max_depth);
        } else if is_rollout_path(&path) {
            files.push(path);
        }
    }
}

fn is_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_rollout_filename)
}

fn is_rollout_filename(file_name: &str) -> bool {
    if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
        return false;
    }
    let stem = file_name.trim_end_matches(".jsonl");
    stem.get(stem.len().saturating_sub(36)..)
        .is_some_and(|candidate| uuid::Uuid::parse_str(candidate).is_ok())
}

fn thread_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    uuid::Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

type RolloutIndex = HashMap<String, Vec<PathBuf>>;

fn build_rollout_index(files: &[PathBuf]) -> RolloutIndex {
    let mut index = RolloutIndex::new();
    for path in files {
        if let Some(thread_id) = thread_id_from_filename(path) {
            index.entry(thread_id).or_default().push(path.clone());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
    }
    index
}

fn load_cursors(conn: &Connection) -> Result<HashMap<String, (i64, i64, i64)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT file_path, last_modified, last_size, last_line_offset FROM session_log_sync",
        )
        .map_err(|error| format!("读取会话用量游标失败: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ),
            ))
        })
        .map_err(|error| format!("查询会话用量游标失败: {error}"))?;
    let mut cursors = HashMap::new();
    for row in rows {
        let (path, state) = row.map_err(|error| format!("解析会话用量游标失败: {error}"))?;
        cursors.insert(path, state);
    }
    Ok(cursors)
}

fn inherit_archived_cursor(
    file_path: &Path,
    cursors: &HashMap<String, (i64, i64, i64)>,
) -> Option<(i64, i64, i64)> {
    if file_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        != Some("archived_sessions")
    {
        return None;
    }
    let file_name = file_path.file_name()?.to_str()?;
    let slash_suffix = format!("/{file_name}");
    let backslash_suffix = format!("\\{file_name}");
    cursors
        .iter()
        .filter(|(path, _)| {
            path.as_str() != file_path.to_string_lossy().as_ref()
                && (path.ends_with(&slash_suffix) || path.ends_with(&backslash_suffix))
        })
        .map(|(_, &(modified, size, offset))| (offset, modified, size))
        .max()
        .map(|(offset, modified, size)| (modified, size, offset))
}

fn sync_single_file(
    conn: &mut Connection,
    instance: &UsageInstance,
    file_path: &Path,
    rollout_index: &RolloutIndex,
    cursors: &mut HashMap<String, (i64, i64, i64)>,
) -> Result<FileSyncResult, String> {
    let file_path_str = file_path.to_string_lossy().to_string();
    let metadata =
        fs::metadata(file_path).map_err(|error| format!("无法读取文件元数据: {error}"))?;
    let file_modified = metadata_modified_nanos(&metadata);
    let file_size = metadata.len();

    let (last_modified, last_size, last_offset) = cursors
        .get(&file_path_str)
        .copied()
        .or_else(|| inherit_archived_cursor(file_path, cursors))
        .unwrap_or((0, 0, 0));

    if file_size >= last_size.max(0) as u64 && file_modified <= last_modified && last_offset > 0 {
        return Ok(FileSyncResult::default());
    }

    if let Ok(mut caches) = replay_caches().lock() {
        if let Some(pending) = caches.pending.get(file_path).cloned() {
            if pending.modified == file_modified && pending.size == file_size {
                match &pending.reason {
                    PendingReason::MissingParent(parent) if !rollout_index.contains_key(parent) => {
                        return Ok(FileSyncResult {
                            deferred: true,
                            ..FileSyncResult::default()
                        });
                    }
                    PendingReason::Stable(_) => {
                        return Ok(FileSyncResult {
                            deferred: true,
                            ..FileSyncResult::default()
                        });
                    }
                    _ => {
                        caches.pending.remove(file_path);
                    }
                }
            }
        }
    }

    if file_size < last_size.max(0) as u64 {
        conn.execute(
            "DELETE FROM session_usage_events WHERE file_path = ?1",
            params![file_path_str],
        )
        .map_err(|error| format!("清理已重写会话用量失败: {error}"))?;
    }

    let parsed = parse_codex_file(file_path, thread_id_from_filename(file_path))?;
    if !parsed.has_billable_tokens {
        upsert_cursor(
            conn,
            cursors,
            &file_path_str,
            &instance.id,
            file_modified,
            file_size,
            parsed.line_offset,
        )?;
        return Ok(FileSyncResult::default());
    }

    let Some(root_thread_id) = parsed.root_thread_id.as_deref() else {
        return Ok(mark_deferred(
            file_path,
            file_modified,
            file_size,
            PendingReason::Stable("文件名缺少有效的尾部 UUID".to_string()),
        ));
    };
    if !parsed.root_meta_seen {
        return Ok(mark_deferred(
            file_path,
            file_modified,
            file_size,
            PendingReason::Stable("含计费 token 但尚无 session_meta".to_string()),
        ));
    }

    let replay_prefix = match parsed.parent {
        ParentResolution::None => 0,
        ParentResolution::Deferred => {
            return Ok(mark_deferred(
                file_path,
                file_modified,
                file_size,
                PendingReason::Stable(
                    parsed
                        .deferred_reason
                        .unwrap_or_else(|| "分叉会话元数据不完整".to_string()),
                ),
            ));
        }
        ParentResolution::Parent => {
            let Some(parent_id) = parsed.parent_id.as_deref() else {
                return Ok(mark_deferred(
                    file_path,
                    file_modified,
                    file_size,
                    PendingReason::Stable("分叉会话缺少父会话 ID".to_string()),
                ));
            };
            let Some(cutoff) = parsed.root_timestamp else {
                return Ok(mark_deferred(
                    file_path,
                    file_modified,
                    file_size,
                    PendingReason::Stable(
                        "parented rollout 的 root meta 缺少有效 timestamp".to_string(),
                    ),
                ));
            };
            match resolve_parent_signatures(parent_id, cutoff, rollout_index) {
                Ok(signatures) => matching_replay_prefix(&parsed.token_events, &signatures),
                Err(reason) => {
                    let pending_reason = if rollout_index.contains_key(parent_id) {
                        PendingReason::Retryable(reason)
                    } else {
                        PendingReason::MissingParent(parent_id.to_string())
                    };
                    return Ok(mark_deferred(
                        file_path,
                        file_modified,
                        file_size,
                        pending_reason,
                    ));
                }
            }
        }
    };

    if let Ok(mut caches) = replay_caches().lock() {
        caches.pending.remove(file_path);
    }

    let mut to_insert = Vec::new();
    let mut result = FileSyncResult::default();
    for (token_offset, event) in parsed.token_events.iter().enumerate() {
        let Some(event_index) = event.event_index else {
            continue;
        };
        if token_offset < replay_prefix {
            if event.line_offset > last_offset {
                result.skipped = result.skipped.saturating_add(1);
            }
            continue;
        }
        if event.line_offset <= last_offset && file_size >= last_size.max(0) as u64 {
            continue;
        }
        to_insert.push((event, event_index));
    }

    if !to_insert.is_empty() {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("开启会话用量写入事务失败: {error}"))?;
        {
            let mut statement = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO session_usage_events (
                        request_id, instance_id, instance_name, session_id, model,
                        timestamp, input_tokens, cached_input_tokens, output_tokens, file_path
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|error| format!("准备会话用量写入失败: {error}"))?;
            for chunk in to_insert.chunks(INSERT_BATCH_SIZE) {
                for (event, event_index) in chunk {
                    let request_id = format!("{REQUEST_ID_PREFIX}:{root_thread_id}:{event_index}");
                    let changed = statement
                        .execute(params![
                            request_id,
                            instance.id,
                            instance.name,
                            root_thread_id,
                            event.model,
                            event.timestamp.unwrap_or(0),
                            event.delta.input as i64,
                            event.delta.cached_input as i64,
                            event.delta.output as i64,
                            file_path_str,
                        ])
                        .map_err(|error| format!("写入会话用量失败: {error}"))?;
                    if changed > 0 {
                        result.imported = result.imported.saturating_add(1);
                    } else {
                        result.skipped = result.skipped.saturating_add(1);
                    }
                }
            }
        }
        upsert_cursor_on_conn(
            &tx,
            &file_path_str,
            &instance.id,
            file_modified,
            file_size,
            parsed.line_offset,
        )?;
        tx.commit()
            .map_err(|error| format!("提交会话用量写入失败: {error}"))?;
        cursors.insert(
            file_path_str,
            (file_modified, file_size as i64, parsed.line_offset),
        );
    } else {
        upsert_cursor(
            conn,
            cursors,
            &file_path_str,
            &instance.id,
            file_modified,
            file_size,
            parsed.line_offset,
        )?;
    }

    Ok(result)
}

fn mark_deferred(
    file_path: &Path,
    modified: i64,
    size: u64,
    reason: PendingReason,
) -> FileSyncResult {
    let entry = PendingEntry {
        modified,
        size,
        reason,
    };
    if let Ok(mut caches) = replay_caches().lock() {
        caches.pending.insert(file_path.to_path_buf(), entry);
    }
    FileSyncResult {
        deferred: true,
        ..FileSyncResult::default()
    }
}

fn upsert_cursor(
    conn: &Connection,
    cursors: &mut HashMap<String, (i64, i64, i64)>,
    file_path: &str,
    instance_id: &str,
    modified: i64,
    size: u64,
    line_offset: i64,
) -> Result<(), String> {
    upsert_cursor_on_conn(conn, file_path, instance_id, modified, size, line_offset)?;
    cursors.insert(file_path.to_string(), (modified, size as i64, line_offset));
    Ok(())
}

fn upsert_cursor_on_conn(
    conn: &Connection,
    file_path: &str,
    instance_id: &str,
    modified: i64,
    size: u64,
    line_offset: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_log_sync (
            file_path, instance_id, last_modified, last_size, last_line_offset
        ) VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(file_path) DO UPDATE SET
            instance_id = excluded.instance_id,
            last_modified = excluded.last_modified,
            last_size = excluded.last_size,
            last_line_offset = excluded.last_line_offset",
        params![file_path, instance_id, modified, size as i64, line_offset],
    )
    .map_err(|error| format!("更新会话用量游标失败: {error}"))?;
    Ok(())
}

fn parse_codex_file(
    file_path: &Path,
    root_thread_id: Option<String>,
) -> Result<ParsedCodexFile, String> {
    let file = File::open(file_path).map_err(|error| format!("无法打开文件: {error}"))?;
    let reader = BufReader::new(file);
    let mut root_meta_seen = false;
    let mut root_timestamp = None;
    let mut parent = ParentResolution::None;
    let mut parent_id = None;
    let mut deferred_reason = None;
    let mut current_model = "unknown".to_string();
    let mut total_high_water = None;
    let mut last_signature_by_source: HashMap<Option<String>, TokenUsageSignature> = HashMap::new();
    let mut previous_token_signature = None;
    let mut event_index = 0u32;
    let mut token_events = Vec::new();
    let mut line_offset = 0i64;
    let mut has_billable_tokens = false;

    for line_result in reader.lines() {
        line_offset += 1;
        let Ok(line) = line_result else {
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }

        let is_event_msg = line.contains("\"event_msg\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_session_meta = line.contains("\"session_meta\"");
        if !is_event_msg && !is_turn_context && !is_session_meta {
            continue;
        }
        if is_event_msg && !line.contains("\"token_count\"") {
            continue;
        }

        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        let Some(event_type) = value.get("type").and_then(JsonValue::as_str) else {
            continue;
        };

        match event_type {
            "session_meta" if !root_meta_seen => {
                root_meta_seen = true;
                root_timestamp = parse_timestamp(value.get("timestamp"));
                let payload = value.get("payload").unwrap_or(&JsonValue::Null);
                match explicit_parent_from_meta(payload) {
                    Ok(None) => {}
                    Ok(Some(parent_thread_id)) => {
                        if root_thread_id.as_deref() == Some(parent_thread_id.as_str()) {
                            parent = ParentResolution::Deferred;
                            deferred_reason =
                                Some("parent_thread_id 与 root_thread_id 相同".to_string());
                        } else {
                            parent = ParentResolution::Parent;
                            parent_id = Some(parent_thread_id);
                        }
                    }
                    Err(reason) => {
                        parent = ParentResolution::Deferred;
                        deferred_reason = Some(reason);
                    }
                }

                let meta_thread_id = non_empty_string(
                    payload
                        .get("id")
                        .or_else(|| payload.get("thread_id"))
                        .or_else(|| payload.get("threadId")),
                );
                if let (Some(filename_id), Some(meta_id)) = (&root_thread_id, meta_thread_id) {
                    if filename_id != &meta_id {
                        parent = ParentResolution::Deferred;
                        deferred_reason = Some(format!(
                            "文件名线程 ID ({filename_id}) 与 root meta ID ({meta_id}) 不一致"
                        ));
                    }
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    if let Some(model) = payload
                        .get("model")
                        .or_else(|| payload.get("info").and_then(|info| info.get("model")))
                        .and_then(JsonValue::as_str)
                    {
                        current_model = normalize_codex_model(model);
                    }
                }
            }
            "event_msg" => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(JsonValue::as_str) != Some("token_count") {
                    continue;
                }
                let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                    continue;
                };
                let Some(signature) = parse_token_signature(info) else {
                    continue;
                };
                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(JsonValue::as_str)
                {
                    current_model = normalize_codex_model(model);
                }

                let snapshot_source = token_snapshot_source(payload);
                let total = info
                    .get("total_token_usage")
                    .and_then(parse_cumulative_tokens);
                let last = info
                    .get("last_token_usage")
                    .and_then(parse_cumulative_tokens);
                if total.is_none() && last.is_none() {
                    continue;
                }
                let has_total_snapshot = total.is_some();
                let duplicate_snapshot = has_total_snapshot
                    && (last_signature_by_source.get(&snapshot_source) == Some(&signature)
                        || previous_token_signature.as_ref() == Some(&signature));
                if has_total_snapshot {
                    last_signature_by_source.insert(snapshot_source, signature.clone());
                }
                previous_token_signature = Some(signature.clone());

                let delta = if duplicate_snapshot {
                    DeltaTokens::default()
                } else if let Some(last) = last {
                    DeltaTokens {
                        input: last.input,
                        cached_input: last.cached_input,
                        output: last.output,
                    }
                } else if let Some(total) = total.as_ref() {
                    compute_delta(&total_high_water, total)
                } else {
                    continue;
                };
                if let Some(total) = total {
                    if let Some(high_water) = total_high_water.as_mut() {
                        update_high_water(high_water, &total);
                    } else {
                        total_high_water = Some(total);
                    }
                }
                let delta = DeltaTokens {
                    cached_input: delta.cached_input.min(delta.input),
                    ..delta
                };
                let nonzero_index = if delta.is_zero() {
                    None
                } else {
                    has_billable_tokens = true;
                    event_index = event_index.saturating_add(1);
                    Some(event_index)
                };
                token_events.push(ParsedTokenEvent {
                    line_offset,
                    signature,
                    delta,
                    event_index: nonzero_index,
                    model: current_model.clone(),
                    timestamp: parse_timestamp(value.get("timestamp"))
                        .map(|value| value.timestamp()),
                });
            }
            _ => {}
        }
    }

    Ok(ParsedCodexFile {
        root_thread_id,
        root_meta_seen,
        root_timestamp,
        parent_id,
        parent,
        deferred_reason,
        token_events,
        line_offset,
        has_billable_tokens,
    })
}

fn explicit_parent_from_meta(payload: &JsonValue) -> Result<Option<String>, String> {
    let forked_from = non_empty_string(payload.get("forked_from_id"));
    let spawned_from = payload
        .get("source")
        .and_then(|source| source.get("subagent"))
        .and_then(|subagent| subagent.get("thread_spawn"))
        .and_then(|spawn| non_empty_string(spawn.get("parent_thread_id")));

    let parent = match (forked_from, spawned_from) {
        (None, None) => return Ok(None),
        (Some(parent), None) | (None, Some(parent)) => parent,
        (Some(forked), Some(spawned)) if forked == spawned => forked,
        (Some(forked), Some(spawned)) => {
            return Err(format!(
                "forked_from_id ({forked}) 与 thread_spawn.parent_thread_id ({spawned}) 不一致"
            ));
        }
    };

    uuid::Uuid::parse_str(&parent)
        .map(|value| Some(value.hyphenated().to_string()))
        .map_err(|_| format!("显式 parent_thread_id 不是有效 UUID: {parent}"))
}

fn resolve_parent_signatures(
    parent_id: &str,
    cutoff: DateTime<Utc>,
    rollout_index: &RolloutIndex,
) -> Result<Vec<TokenUsageSignature>, String> {
    let Some(candidates) = rollout_index.get(parent_id) else {
        return Err(format!("找不到父 rollout: {parent_id}"));
    };
    let mut snapshots = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        snapshots.push(parent_signatures_before(candidate, cutoff)?);
    }
    let Some(first) = snapshots.first() else {
        return Err(format!("找不到父 rollout: {parent_id}"));
    };
    if snapshots.iter().skip(1).any(|snapshot| snapshot != first) {
        return Err(format!(
            "父 rollout UUID {parent_id} 对应多个内容不一致的文件"
        ));
    }
    Ok(first.clone())
}

fn parent_signatures_before(
    parent_path: &Path,
    cutoff: DateTime<Utc>,
) -> Result<Vec<TokenUsageSignature>, String> {
    let metadata = fs::metadata(parent_path)
        .map_err(|error| format!("无法读取父 rollout {}: {error}", parent_path.display()))?;
    let stamp = ParentFileStamp {
        modified_nanos: metadata_modified_nanos(&metadata),
        size: metadata.len(),
    };
    if let Ok(caches) = replay_caches().lock() {
        if let Some(cached) = caches
            .parent_timelines
            .get(parent_path)
            .filter(|entry| entry.stamp == stamp)
        {
            return signatures_before(&cached.timeline, parent_path, cutoff);
        }
    }

    let file = File::open(parent_path)
        .map_err(|error| format!("无法打开父 rollout {}: {error}", parent_path.display()))?;
    let mut events = Vec::new();
    let mut has_token_without_timestamp = false;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        let timestamp = parse_timestamp(value.get("timestamp"));
        if value.get("type").and_then(JsonValue::as_str) != Some("event_msg")
            || value
                .get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(JsonValue::as_str)
                != Some("token_count")
        {
            continue;
        }
        let Some(info) = value
            .get("payload")
            .and_then(|payload| payload.get("info"))
            .filter(|info| !info.is_null())
        else {
            continue;
        };
        let Some(signature) = parse_token_signature(info) else {
            continue;
        };
        let Some(timestamp) = timestamp else {
            has_token_without_timestamp = true;
            continue;
        };
        events.push(TimestampedTokenSignature {
            timestamp,
            signature,
        });
    }

    let timeline = ParentTokenTimeline {
        events,
        has_token_without_timestamp,
    };
    let result = signatures_before(&timeline, parent_path, cutoff);
    if let Ok(mut caches) = replay_caches().lock() {
        caches.parent_timelines.insert(
            parent_path.to_path_buf(),
            CachedParentTimeline { stamp, timeline },
        );
    }
    result
}

fn signatures_before(
    timeline: &ParentTokenTimeline,
    parent_path: &Path,
    cutoff: DateTime<Utc>,
) -> Result<Vec<TokenUsageSignature>, String> {
    if timeline.has_token_without_timestamp {
        return Err(format!(
            "父 rollout {} 的 token_count 缺少有效 timestamp",
            parent_path.display()
        ));
    }
    if timeline.events.is_empty() {
        return Err(format!(
            "父 rollout {} 尚无可用 token_count",
            parent_path.display()
        ));
    }
    Ok(timeline
        .events
        .iter()
        .filter(|event| event.timestamp <= cutoff)
        .map(|event| event.signature.clone())
        .collect())
}

fn matching_replay_prefix(child: &[ParsedTokenEvent], parent: &[TokenUsageSignature]) -> usize {
    let mut parent_offset = 0usize;
    let mut matched = 0usize;
    for event in child {
        let Some(relative_match) = parent[parent_offset..]
            .iter()
            .position(|signature| signature == &event.signature)
        else {
            break;
        };
        parent_offset += relative_match + 1;
        matched += 1;
    }
    matched
}

fn normalize_codex_model(raw: &str) -> String {
    let mut name = raw.to_lowercase();
    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }
    if name.len() > 11 && name.is_char_boundary(name.len() - 11) {
        let suffix = &name[name.len() - 11..];
        if suffix.is_ascii()
            && suffix.as_bytes()[0] == b'-'
            && suffix[1..5].chars().all(|c| c.is_ascii_digit())
            && suffix.as_bytes()[5] == b'-'
            && suffix[6..8].chars().all(|c| c.is_ascii_digit())
            && suffix.as_bytes()[8] == b'-'
            && suffix[9..11].chars().all(|c| c.is_ascii_digit())
        {
            name.truncate(name.len() - 11);
        }
    }
    if name.len() > 9 {
        let parts: Vec<&str> = name.rsplitn(2, '-').collect();
        if parts.len() == 2 {
            if let Some(suffix) = parts.first() {
                if suffix.len() == 8 && suffix.chars().all(|c| c.is_ascii_digit()) {
                    name = parts[1].to_string();
                }
            }
        }
    }
    name
}

fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    match prev {
        None => DeltaTokens {
            input: current.input,
            cached_input: current.cached_input,
            output: current.output,
        },
        Some(previous) => DeltaTokens {
            input: current.input.saturating_sub(previous.input),
            cached_input: current.cached_input.saturating_sub(previous.cached_input),
            output: current.output.saturating_sub(previous.output),
        },
    }
}

fn update_high_water(high_water: &mut CumulativeTokens, current: &CumulativeTokens) {
    high_water.input = high_water.input.max(current.input);
    high_water.cached_input = high_water.cached_input.max(current.cached_input);
    high_water.output = high_water.output.max(current.output);
}

fn parse_cumulative_tokens(total_usage: &JsonValue) -> Option<CumulativeTokens> {
    let fields = total_usage.as_object()?;
    if ![
        "input_tokens",
        "cached_input_tokens",
        "cache_read_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
        "total_tokens",
    ]
    .iter()
    .any(|field| fields.contains_key(*field))
    {
        return None;
    }
    Some(CumulativeTokens {
        input: total_usage
            .get("input_tokens")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0),
        cached_input: total_usage
            .get("cached_input_tokens")
            .or_else(|| total_usage.get("cache_read_input_tokens"))
            .and_then(JsonValue::as_u64)
            .unwrap_or(0),
        output: total_usage
            .get("output_tokens")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0),
    })
}

fn parse_signature_counters(value: Option<&JsonValue>) -> Option<TokenCountersSignature> {
    let value = value?.as_object()?;
    Some(TokenCountersSignature {
        input: value.get("input_tokens").and_then(JsonValue::as_u64),
        cached_input: value
            .get("cached_input_tokens")
            .or_else(|| value.get("cache_read_input_tokens"))
            .and_then(JsonValue::as_u64),
        output: value.get("output_tokens").and_then(JsonValue::as_u64),
        reasoning_output: value
            .get("reasoning_output_tokens")
            .and_then(JsonValue::as_u64),
        total: value.get("total_tokens").and_then(JsonValue::as_u64),
    })
}

fn parse_token_signature(info: &JsonValue) -> Option<TokenUsageSignature> {
    let total = parse_signature_counters(info.get("total_token_usage"));
    let last = parse_signature_counters(info.get("last_token_usage"));
    (total.is_some() || last.is_some()).then_some(TokenUsageSignature { total, last })
}

fn token_snapshot_source(payload: &JsonValue) -> Option<String> {
    payload
        .get("rate_limits")
        .and_then(|rate_limits| rate_limits.get("limit_id"))
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_timestamp(value: Option<&JsonValue>) -> Option<DateTime<Utc>> {
    value
        .and_then(JsonValue::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn non_empty_string(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn metadata_modified_nanos(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as i64)
        .unwrap_or(0)
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_usage_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|error| format!("写入会话用量元数据失败: {error}"))?;
    Ok(())
}

fn get_meta_i64(conn: &Connection, key: &str) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM session_usage_meta WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|value| value.parse().ok())
}

fn estimate_usage_cost_usd(by_model: &[CodexSessionUsageBreakdownRow]) -> f64 {
    by_model.iter().fold(0.0, |sum, row| {
        sum + crate::modules::codex_local_access::estimate_model_token_cost_usd(
            &row.key,
            row.input_tokens,
            row.cached_input_tokens,
            row.output_tokens,
        )
    })
}

fn query_totals(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<CodexSessionUsageTotals, String> {
    let sql = format!(
        "SELECT
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(cached_input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(COUNT(*), 0)
         FROM session_usage_events {where_sql}"
    );
    conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |row| {
        let input = row.get::<_, i64>(0)?.max(0) as u64;
        let cached = row.get::<_, i64>(1)?.max(0) as u64;
        let output = row.get::<_, i64>(2)?.max(0) as u64;
        let requests = row.get::<_, i64>(3)?.max(0) as u64;
        Ok(CodexSessionUsageTotals {
            input_tokens: input,
            cached_input_tokens: cached,
            output_tokens: output,
            total_tokens: input.saturating_add(output),
            request_count: requests,
            estimated_cost_usd: 0.0,
        })
    })
    .map_err(|error| format!("汇总会话用量失败: {error}"))
}

fn query_breakdown(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
    key_column: &str,
    label_column: Option<&str>,
    instance_names: &HashMap<String, String>,
) -> Result<Vec<CodexSessionUsageBreakdownRow>, String> {
    let label_sql = label_column.unwrap_or(key_column);
    let sql = format!(
        "SELECT
            {key_column},
            MAX({label_sql}),
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(cached_input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(COUNT(*), 0)
         FROM session_usage_events {where_sql}
         GROUP BY {key_column}
         ORDER BY SUM(input_tokens + output_tokens) DESC, {key_column} ASC"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("查询会话用量分组失败: {error}"))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let key = row.get::<_, String>(0)?;
            let stored_label = row.get::<_, String>(1)?;
            let input = row.get::<_, i64>(2)?.max(0) as u64;
            let cached = row.get::<_, i64>(3)?.max(0) as u64;
            let output = row.get::<_, i64>(4)?.max(0) as u64;
            let requests = row.get::<_, i64>(5)?.max(0) as u64;
            let label = if key_column == "instance_id" {
                instance_names
                    .get(&key)
                    .cloned()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(stored_label)
            } else {
                stored_label
            };
            Ok(CodexSessionUsageBreakdownRow {
                key,
                label,
                input_tokens: input,
                cached_input_tokens: cached,
                output_tokens: output,
                total_tokens: input.saturating_add(output),
                request_count: requests,
            })
        })
        .map_err(|error| format!("遍历会话用量分组失败: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("解析会话用量分组失败: {error}"))
}

fn query_day_breakdown(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<Vec<CodexSessionUsageBreakdownRow>, String> {
    let sql = format!(
        "SELECT timestamp, input_tokens, cached_input_tokens, output_tokens
         FROM session_usage_events {where_sql}"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("查询会话用量日期失败: {error}"))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?.max(0) as u64,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("遍历会话用量日期失败: {error}"))?;

    let mut grouped: HashMap<String, CodexSessionUsageTotals> = HashMap::new();
    for row in rows {
        let (timestamp, input, cached, output) =
            row.map_err(|error| format!("解析会话用量日期失败: {error}"))?;
        let key = local_day_key(timestamp);
        let entry = grouped.entry(key).or_default();
        entry.input_tokens = entry.input_tokens.saturating_add(input);
        entry.cached_input_tokens = entry.cached_input_tokens.saturating_add(cached);
        entry.output_tokens = entry.output_tokens.saturating_add(output);
        entry.total_tokens = entry
            .total_tokens
            .saturating_add(input.saturating_add(output));
        entry.request_count = entry.request_count.saturating_add(1);
    }

    Ok(grouped
        .into_iter()
        .map(|(key, totals)| CodexSessionUsageBreakdownRow {
            label: key.clone(),
            key,
            input_tokens: totals.input_tokens,
            cached_input_tokens: totals.cached_input_tokens,
            output_tokens: totals.output_tokens,
            total_tokens: totals.total_tokens,
            request_count: totals.request_count,
        })
        .collect())
}

fn local_day_key(timestamp: i64) -> String {
    if timestamp <= 0 {
        return String::new();
    }
    Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|value| {
            format!(
                "{:04}-{:02}-{:02}",
                value.year(),
                value.month(),
                value.day()
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PARENT_ID: &str = "00000000-0000-4000-8000-000000000001";
    const CHILD_ID: &str = "00000000-0000-4000-8000-000000000002";

    fn write_jsonl(path: &Path, values: &[JsonValue]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let contents = values
            .iter()
            .map(JsonValue::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(path, contents).unwrap();
    }

    fn rollout_path(dir: &Path, thread_id: &str) -> PathBuf {
        dir.join(format!("rollout-2026-07-10T03-00-00-{thread_id}.jsonl"))
    }

    fn session_meta_at(
        thread_id: &str,
        forked_from_id: Option<&str>,
        timestamp: &str,
    ) -> JsonValue {
        json!({
            "timestamp": timestamp,
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "forked_from_id": forked_from_id,
            }
        })
    }

    fn session_meta(thread_id: &str) -> JsonValue {
        session_meta_at(thread_id, None, "2026-07-10T03:00:00Z")
    }

    fn turn_context(model: &str) -> JsonValue {
        json!({
            "timestamp": "2026-07-10T03:00:01Z",
            "type": "turn_context",
            "payload": { "model": model }
        })
    }

    fn token_count_total(input: u64, cached: u64, output: u64, timestamp: &str) -> JsonValue {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": input,
                        "cached_input_tokens": cached,
                        "output_tokens": output,
                        "reasoning_output_tokens": 0,
                        "total_tokens": input + output
                    }
                }
            }
        })
    }

    fn token_count_with_last(
        total_input: u64,
        total_cached: u64,
        total_output: u64,
        last_input: u64,
        last_cached: u64,
        last_output: u64,
        limit_id: &str,
        timestamp: &str,
    ) -> JsonValue {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": total_input,
                        "cached_input_tokens": total_cached,
                        "output_tokens": total_output,
                        "reasoning_output_tokens": 0,
                        "total_tokens": total_input + total_output
                    },
                    "last_token_usage": {
                        "input_tokens": last_input,
                        "cached_input_tokens": last_cached,
                        "output_tokens": last_output,
                        "reasoning_output_tokens": 0,
                        "total_tokens": last_input + last_output
                    }
                },
                "rate_limits": { "limit_id": limit_id }
            }
        })
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), unique));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn nonzero_deltas(parsed: &ParsedCodexFile) -> Vec<(u64, u64, u64)> {
        parsed
            .token_events
            .iter()
            .filter(|event| !event.delta.is_zero())
            .map(|event| {
                (
                    event.delta.input,
                    event.delta.cached_input,
                    event.delta.output,
                )
            })
            .collect()
    }

    #[test]
    fn normalize_model_strips_provider_and_date() {
        assert_eq!(
            normalize_codex_model("OpenAI/GPT-5.4-2026-03-05"),
            "gpt-5.4"
        );
        assert_eq!(normalize_codex_model("gpt-5.4-20260305"), "gpt-5.4");
    }

    #[test]
    fn prefers_last_token_usage_and_skips_duplicate_snapshots() {
        let dir = make_temp_dir("codex-usage-last");
        let file = rollout_path(&dir, PARENT_ID);
        let replay = token_count_with_last(
            87_709_262,
            83_563_008,
            240_919,
            151_258,
            147_200,
            87,
            "codex_bengalfox",
            "2026-07-10T03:00:03Z",
        );
        write_jsonl(
            &file,
            &[
                session_meta(PARENT_ID),
                turn_context("openai/gpt-5.4"),
                token_count_with_last(
                    76_780_408,
                    73_010_432,
                    243_036,
                    175_074,
                    169_728,
                    6_827,
                    "codex",
                    "2026-07-10T03:00:02Z",
                ),
                replay.clone(),
                token_count_with_last(
                    76_962_538,
                    73_180_160,
                    243_258,
                    182_130,
                    169_728,
                    222,
                    "codex",
                    "2026-07-10T03:00:04Z",
                ),
                replay,
            ],
        );

        let parsed = parse_codex_file(&file, Some(PARENT_ID.to_string())).unwrap();
        assert_eq!(
            nonzero_deltas(&parsed),
            vec![
                (175_074, 169_728, 6_827),
                (151_258, 147_200, 87),
                (182_130, 169_728, 222),
            ]
        );
        assert!(parsed.token_events[3].delta.is_zero());
        assert_eq!(parsed.token_events[0].model, "gpt-5.4");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cross_limit_identical_snapshot_is_not_double_counted() {
        let dir = make_temp_dir("codex-usage-cross-limit");
        let file = rollout_path(&dir, PARENT_ID);
        write_jsonl(
            &file,
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_with_last(1_000, 0, 10, 100, 0, 10, "codex", "2026-07-10T03:00:02Z"),
                token_count_with_last(
                    1_000,
                    0,
                    10,
                    100,
                    0,
                    10,
                    "codex_bengalfox",
                    "2026-07-10T03:00:03Z",
                ),
            ],
        );
        let parsed = parse_codex_file(&file, Some(PARENT_ID.to_string())).unwrap();
        assert_eq!(nonzero_deltas(&parsed), vec![(100, 0, 10)]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn falls_back_to_cumulative_delta_when_last_missing() {
        let dir = make_temp_dir("codex-usage-delta");
        let file = rollout_path(&dir, PARENT_ID);
        write_jsonl(
            &file,
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_total(17_934, 9_600, 454, "2026-07-10T03:00:02Z"),
                token_count_total(36_722, 27_904, 804, "2026-07-10T03:00:03Z"),
                token_count_total(36_722, 27_904, 804, "2026-07-10T03:00:04Z"),
            ],
        );
        let parsed = parse_codex_file(&file, Some(PARENT_ID.to_string())).unwrap();
        assert_eq!(
            nonzero_deltas(&parsed),
            vec![(17_934, 9_600, 454), (18_788, 18_304, 350)]
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parent_fork_replay_is_skipped_and_only_new_usage_is_stored() {
        let dir = make_temp_dir("codex-usage-fork");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let sessions = home.join("sessions").join("2026").join("07").join("10");
        let parent = sessions.join(format!("rollout-2026-07-10T03-00-00-{PARENT_ID}.jsonl"));
        let child = sessions.join(format!("rollout-2026-07-10T03-10-00-{CHILD_ID}.jsonl"));
        write_jsonl(
            &parent,
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_with_last(1_000, 0, 20, 1_000, 0, 20, "codex", "2026-07-10T03:00:02Z"),
            ],
        );
        write_jsonl(
            &child,
            &[
                session_meta_at(CHILD_ID, Some(PARENT_ID), "2026-07-10T03:10:00Z"),
                turn_context("gpt-5.4"),
                token_count_with_last(1_000, 0, 20, 1_000, 0, 20, "codex", "2026-07-10T03:00:02Z"),
                token_count_with_last(1_400, 0, 35, 400, 0, 15, "codex", "2026-07-10T03:10:05Z"),
            ],
        );

        let store = SessionUsageStore::open_path(db_path);
        let instances = vec![UsageInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: home,
        }];
        let sync = store.sync(false, &instances).unwrap();
        assert_eq!(sync.imported, 2);
        let report = store.query(&CodexSessionUsageQuery::default()).unwrap();
        assert_eq!(report.totals.input_tokens, 1_400);
        assert_eq!(report.totals.output_tokens, 35);
        assert_eq!(report.totals.request_count, 2);
        assert_eq!(report.by_model.len(), 1);
        assert_eq!(report.by_model[0].key, "gpt-5.4");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn incremental_sync_does_not_double_count_unchanged_files() {
        let dir = make_temp_dir("codex-usage-incr");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let file = home
            .join("sessions")
            .join("2026")
            .join("07")
            .join("10")
            .join(format!("rollout-2026-07-10T03-00-00-{PARENT_ID}.jsonl"));
        write_jsonl(
            &file,
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_with_last(100, 0, 10, 100, 0, 10, "codex", "2026-07-10T03:00:02Z"),
            ],
        );
        let store = SessionUsageStore::open_path(db_path);
        let instances = vec![UsageInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: home,
        }];
        assert_eq!(store.sync(false, &instances).unwrap().imported, 1);
        assert_eq!(store.sync(false, &instances).unwrap().imported, 0);
        let report = store.query(&CodexSessionUsageQuery::default()).unwrap();
        assert_eq!(report.totals.request_count, 1);
        assert_eq!(report.totals.input_tokens, 100);
        fs::remove_dir_all(&dir).ok();
    }
}
