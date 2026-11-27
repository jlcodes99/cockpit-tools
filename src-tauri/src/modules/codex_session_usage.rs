//! 从 Codex 会话 JSONL（rollout）汇总真实 Token 用量。
//!
//! 算法对齐 cc-switch：优先 `last_token_usage`，按完整快照签名去重，
//! 缺 last 时回退 `total_token_usage` 高水位差；分叉会话跳过父 rollout
//! 在 fork 时刻之前的重放前缀。
//!
//! 数据与官方配额、API 服务 `request_logs` 完全隔离，打开用量面板时才扫描。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::modules::{account, codex_instance};

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DB_FILE_NAME: &str = "codex_session_usage.sqlite";
const REQUEST_ID_PREFIX: &str = "codex_session:thread-v1";
#[cfg(test)]
const INSERT_BATCH_SIZE: usize = 32;
const FOREGROUND_INSERT_BATCH_SIZE: usize = 2048;
const FOREGROUND_BATCH_BYTES: u64 = 32 * 1024 * 1024;
const FOREGROUND_BATCH_TIME: Duration = Duration::from_millis(500);
const FOREGROUND_REWRITE_BATCH_ROWS: usize = 2048;
const PARALLEL_PARSE_MIN_FILES: usize = 2;
const PARALLEL_PARSE_MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const PARALLEL_PARSE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const PARALLEL_PARSE_MAX_EVENTS: usize = 50_000;
const PARALLEL_PARSE_MAX_TIME: Duration = Duration::from_secs(30);
const PARALLEL_COMMIT_BATCH_FILES: usize = 16;
const PARALLEL_COMMIT_BATCH_ROWS: usize = 2048;
const RECENT_SCAN_REUSE_WINDOW: Duration = Duration::from_millis(2500);

static SYNC_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static REPLAY_CACHE: OnceLock<Mutex<ReplayCaches>> = OnceLock::new();
static LAST_CLEAN_SYNC_AT: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
static USAGE_SNAPSHOT_REVISION: AtomicU64 = AtomicU64::new(1);
static USAGE_BUCKET_SNAPSHOT: OnceLock<RwLock<Option<Arc<UsageBucketSnapshot>>>> = OnceLock::new();
#[cfg(not(test))]
static INITIALIZED_DB_PATHS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn sync_lock() -> &'static Mutex<()> {
    SYNC_LOCK.get_or_init(|| Mutex::new(()))
}

fn replay_caches() -> &'static Mutex<ReplayCaches> {
    REPLAY_CACHE.get_or_init(|| Mutex::new(ReplayCaches::default()))
}

fn last_clean_sync_at() -> &'static Mutex<Option<Instant>> {
    LAST_CLEAN_SYNC_AT.get_or_init(|| Mutex::new(None))
}

fn is_recent_clean_sync(since: Option<Instant>) -> bool {
    let Ok(guard) = last_clean_sync_at().lock() else {
        return false;
    };
    let Some(completed_at) = *guard else {
        return false;
    };
    if let Some(wait_started) = since {
        if completed_at >= wait_started {
            return true;
        }
    }
    completed_at.elapsed() <= RECENT_SCAN_REUSE_WINDOW
}

fn record_clean_sync(clean: bool) {
    if let Ok(mut guard) = last_clean_sync_at().lock() {
        *guard = clean.then(Instant::now);
    }
}

fn usage_bucket_snapshot_cache() -> &'static RwLock<Option<Arc<UsageBucketSnapshot>>> {
    USAGE_BUCKET_SNAPSHOT.get_or_init(|| RwLock::new(None))
}

fn invalidate_usage_bucket_snapshot() {
    USAGE_SNAPSHOT_REVISION.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut guard) = usage_bucket_snapshot_cache().write() {
        *guard = None;
    }
}

#[cfg(not(test))]
fn initialized_db_paths() -> &'static Mutex<HashSet<PathBuf>> {
    INITIALIZED_DB_PATHS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[derive(Debug, Clone, Copy)]
struct ParseBatchBudget {
    max_bytes: u64,
    max_events: usize,
    max_time: Duration,
}

impl ParseBatchBudget {
    #[cfg(test)]
    fn background() -> Self {
        Self {
            max_bytes: crate::modules::background_maintenance::BATCH_BYTES,
            max_events: INSERT_BATCH_SIZE,
            max_time: crate::modules::background_maintenance::BATCH_TIME,
        }
    }

    fn foreground() -> Self {
        Self {
            max_bytes: FOREGROUND_BATCH_BYTES,
            max_events: FOREGROUND_INSERT_BATCH_SIZE,
            max_time: FOREGROUND_BATCH_TIME,
        }
    }

    fn parallel() -> Self {
        Self {
            max_bytes: PARALLEL_PARSE_MAX_BYTES,
            max_events: PARALLEL_PARSE_MAX_EVENTS,
            max_time: PARALLEL_PARSE_MAX_TIME,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SyncCursorState {
    last_modified: i64,
    last_size: i64,
    last_offset: i64,
    checkpoint_exists: bool,
    checkpoint_complete: bool,
    checkpoint_compressed: bool,
    checkpoint_version_valid: bool,
    checkpoint_byte_offset: u64,
}

impl SyncCursorState {
    fn from_checkpoint(
        last_modified: i64,
        last_size: i64,
        last_offset: i64,
        checkpoint: &ParserCheckpoint,
    ) -> Self {
        let checkpoint_version_valid = checkpoint.version == PARSER_VERSION
            || (checkpoint.version == 1 && checkpoint.parent != ParentResolution::Parent);
        Self {
            last_modified,
            last_size,
            last_offset,
            checkpoint_exists: true,
            checkpoint_complete: checkpoint.complete,
            checkpoint_compressed: checkpoint.compressed,
            checkpoint_version_valid,
            checkpoint_byte_offset: checkpoint.byte_offset,
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
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
    pub by_project: Vec<CodexSessionUsageBreakdownRow>,
    pub projects_pending: bool,
    pub model_options: Vec<CodexSessionUsageInstanceOption>,
    pub project_options: Vec<CodexSessionUsageInstanceOption>,
    pub by_day: Vec<CodexSessionUsageBreakdownRow>,
    #[serde(default)]
    pub by_hour: Vec<CodexSessionUsageBreakdownRow>,
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
    pub model: Option<String>,
    pub project_key: Option<String>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub include_projects: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct TokenCountersSignature {
    input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
    reasoning_output: Option<u64>,
    total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct TokenUsageSignature {
    total: Option<TokenCountersSignature>,
    last: Option<TokenCountersSignature>,
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
    pending: HashMap<PathBuf, PendingEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ParentResolution {
    None,
    Parent,
    Paginated,
    Continuation,
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
    checkpoint: ParserCheckpoint,
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
    more: bool,
    imported: u32,
    skipped: u32,
    deferred: bool,
}

include!("codex_session_usage_checkpoint.rs");
include!("codex_session_usage_rollup.rs");
include!("codex_session_usage_parent.rs");
include!("codex_session_usage_rewrite.rs");
include!("codex_session_usage_projects.rs");
include!("codex_session_usage_details.rs");

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
    let store = SessionUsageStore::open_default()?;
    if !rebuild {
        if is_recent_clean_sync(None) {
            return Ok(CodexSessionUsageSyncResult {
                report: Some(store.query(&query)?),
                ..CodexSessionUsageSyncResult::default()
            });
        }
        let wait_started = Instant::now();
        let guard = sync_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if is_recent_clean_sync(Some(wait_started)) {
            drop(guard);
            return Ok(CodexSessionUsageSyncResult {
                report: Some(store.query(&query)?),
                ..CodexSessionUsageSyncResult::default()
            });
        }
        let instances = collect_usage_instances()?;
        let mut result = store.sync(false, &instances)?;
        record_clean_sync(result.errors.is_empty() && result.deferred_files == 0);
        drop(guard);
        result.report = Some(store.query(&query)?);
        return Ok(result);
    }

    let _guard = sync_lock()
        .try_lock()
        .map_err(|_| crate::modules::background_maintenance::busy_message())?;
    record_clean_sync(false);
    let instances = collect_usage_instances()?;
    let mut result = store.sync(true, &instances)?;
    record_clean_sync(result.errors.is_empty() && result.deferred_files == 0);
    result.report = Some(store.query(&query)?);
    Ok(result)
}

/// Fast read-only lookup of per-rollout cumulative token totals from persisted parser checkpoints.
/// Only returns entries whose `(modified_nanos, size, compressed, complete)` match disk metadata.
pub fn lookup_cached_rollout_token_stats(
    rollout_paths: &[PathBuf],
) -> HashMap<PathBuf, (u64, u64, u64)> {
    let mut stats = HashMap::new();
    if rollout_paths.is_empty() {
        return stats;
    }
    let Ok(db_path) = usage_db_path() else {
        return stats;
    };
    if !db_path.is_file() {
        return stats;
    }
    let Ok(conn) = Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return stats;
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(50));
    lookup_cached_rollout_token_stats_from_conn(&conn, rollout_paths, &mut stats);
    stats
}

fn lookup_cached_rollout_token_stats_from_conn(
    conn: &Connection,
    rollout_paths: &[PathBuf],
    stats: &mut HashMap<PathBuf, (u64, u64, u64)>,
) {
    let Ok(mut stmt) = conn.prepare_cached(
        "SELECT
            s.last_modified,
            s.last_size,
            CASE WHEN json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.complete') AS INTEGER), 0) ELSE 0 END,
            CASE WHEN json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.compressed') AS INTEGER), 0) ELSE 0 END,
            CASE WHEN json_valid(c.state) THEN json_extract(c.state, '$.total_high_water.input') ELSE NULL END,
            CASE WHEN json_valid(c.state) THEN json_extract(c.state, '$.total_high_water.output') ELSE NULL END
         FROM session_log_sync s
         JOIN session_parser_checkpoints c USING (file_path)
         WHERE s.file_path = ?1",
    ) else {
        return;
    };

    for path in rollout_paths {
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let file_modified = metadata_modified_nanos(&metadata);
        let file_size = metadata.len();
        let compressed = is_compressed_rollout(path);
        let logical_path = logical_rollout_path(path);
        let logical_key = logical_path.to_string_lossy();
        let row = stmt
            .query_row([logical_key.as_ref()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .optional();
        let Ok(Some((last_modified, last_size, complete, ckpt_compressed, input, output))) = row
        else {
            continue;
        };
        if last_modified > 0
            && file_modified == last_modified
            && file_size == last_size.max(0) as u64
            && complete
            && ckpt_compressed == compressed
        {
            if let (Some(input), Some(output)) = (input, output) {
                let input = input.max(0) as u64;
                let output = output.max(0) as u64;
                stats.insert(path.clone(), (input, output, input.saturating_add(output)));
            }
        }
    }
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
        #[cfg(not(test))]
        let already_initialized = self.db_path.is_file()
            && initialized_db_paths()
                .lock()
                .is_ok_and(|paths| paths.contains(&self.db_path));
        #[cfg(test)]
        let already_initialized = false;

        let conn = Connection::open(&self.db_path)
            .map_err(|error| format!("打开会话用量库失败: {error}"))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(|error| format!("设置会话用量库超时失败: {error}"))?;
        if already_initialized {
            conn.execute_batch(
                "PRAGMA synchronous = NORMAL;
                 PRAGMA foreign_keys = ON;",
            )
            .map_err(|error| format!("配置会话用量库连接失败: {error}"))?;
            return Ok(conn);
        }
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
            CREATE TABLE IF NOT EXISTS session_parser_checkpoints (
                file_path TEXT PRIMARY KEY, state TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_parent_sync (file_path TEXT PRIMARY KEY,state TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS session_parent_signatures (
                file_path TEXT NOT NULL,generation INTEGER NOT NULL,line INTEGER NOT NULL,timestamp INTEGER NOT NULL,signature TEXT NOT NULL,
                UNIQUE(file_path,generation,line));
            CREATE INDEX IF NOT EXISTS idx_parent_signature ON session_parent_signatures(file_path,generation,signature,line);
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
        SESSION_USAGE_ROLLUP
            .ensure(&conn)
            .map_err(|e| e.to_string())?;
        ensure_rewrite_schema(&conn)?;
        ensure_usage_projects(&conn)?;
        #[cfg(not(test))]
        if !usage_projects_complete(&conn) {
            schedule_usage_projects(self.db_path.clone());
        }
        #[cfg(not(test))]
        schedule_session_rollup_backfill(self.db_path.clone());
        #[cfg(not(test))]
        if let Ok(mut paths) = initialized_db_paths().lock() {
            paths.insert(self.db_path.clone());
        }
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
                DELETE FROM session_parser_checkpoints;
                DELETE FROM session_parent_sync;
                DELETE FROM session_parent_signatures;
                DELETE FROM session_rewrite_state;
                DELETE FROM session_rewrite_events;
                DELETE FROM session_usage_meta;
                ",
            )
            .map_err(|error| format!("清空会话用量缓存失败: {error}"))?;
            if let Ok(mut caches) = replay_caches().lock() {
                *caches = ReplayCaches::default();
            }
            invalidate_usage_bucket_snapshot();
        }

        let mut cursors = load_cursors(&conn)?;
        let mut result = CodexSessionUsageSyncResult {
            rebuilt: rebuild,
            ..CodexSessionUsageSyncResult::default()
        };

        for instance in instances {
            if crate::modules::app_lifecycle::is_shutdown_started() {
                invalidate_usage_bucket_snapshot();
                return Ok(result);
            }
            let no_existing_checkpoints = !conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM session_parser_checkpoints)",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(true);
            let files = collect_codex_session_files_with_meta(&instance.data_dir);
            let rollout_index = build_rollout_index(&files);
            result.files_scanned = result.files_scanned.saturating_add(files.len() as u32);
            sync_instance_files(
                &mut conn,
                instance,
                &files,
                &rollout_index,
                &mut cursors,
                no_existing_checkpoints,
                &mut result,
            )?;
        }

        set_meta(&conn, "last_synced_at", &now_unix_seconds().to_string())?;
        set_meta(&conn, "last_error_count", &result.errors.len().to_string())?;
        set_meta(
            &conn,
            "last_deferred_files",
            &result.deferred_files.to_string(),
        )?;
        invalidate_usage_bucket_snapshot();
        Ok(result)
    }

    #[cfg(not(test))]
    fn try_query_from_snapshot_cache(
        &self,
        query: &CodexSessionUsageQuery,
        instances: &[UsageInstance],
    ) -> Result<Option<CodexSessionUsageReport>, String> {
        if query
            .session_id
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            return Ok(None);
        }
        let current_revision = USAGE_SNAPSHOT_REVISION.load(Ordering::Acquire);
        if let Ok(guard) = usage_bucket_snapshot_cache().read() {
            if let Some(snapshot) = guard.as_ref() {
                if snapshot.db_path == self.db_path && snapshot.revision == current_revision {
                    if snapshot_covers_query(snapshot, query) {
                        return Ok(Some(query_report_from_snapshot(snapshot, query, instances)));
                    }
                    return Ok(None);
                }
            }
        }
        let conn = self.open_conn()?;
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let Some(snapshot) = load_usage_bucket_snapshot(&conn, &self.db_path)? else {
            return Ok(None);
        };
        let snapshot = Arc::new(snapshot);
        if USAGE_SNAPSHOT_REVISION.load(Ordering::Acquire) == snapshot.revision {
            if let Ok(mut guard) = usage_bucket_snapshot_cache().write() {
                if USAGE_SNAPSHOT_REVISION.load(Ordering::Acquire) == snapshot.revision {
                    *guard = Some(Arc::clone(&snapshot));
                }
            }
        }
        if snapshot_covers_query(&snapshot, query) {
            Ok(Some(query_report_from_snapshot(&snapshot, query, instances)))
        } else {
            Ok(None)
        }
    }

    fn query(&self, query: &CodexSessionUsageQuery) -> Result<CodexSessionUsageReport, String> {
        let usage_instances = collect_usage_instances().unwrap_or_default();
        #[cfg(not(test))]
        if let Some(report) = self.try_query_from_snapshot_cache(query, &usage_instances)? {
            return Ok(report);
        }

        let conn = self.open_conn()?;
        conn.execute_batch("BEGIN").map_err(|e| e.to_string())?;
        let (from, to) = (
            query.from_timestamp.unwrap_or(0),
            query.to_timestamp.unwrap_or(i64::MAX / 2),
        );
        let rollup_source = session_report_source(&conn, from, to);
        let source = if query.session_id.is_some() {
            format!("SELECT *, 1 AS request_count FROM ({})", usage_detail_source())
        } else {
            rollup_source.clone()
        };
        let source = if query.include_projects || query.project_key.is_some() {
            format!(
                "SELECT u.*, p.project_key, COALESCE(p.project_path,'') AS project_path
                 FROM ({source}) u LEFT JOIN session_usage_projects p USING(file_path)"
            )
        } else {
            source
        };
        conn.execute_batch(&format!("CREATE TEMP VIEW session_usage_report AS {source}"))
            .map_err(|e| e.to_string())?;

        let projects_complete = usage_projects_complete(&conn);
        let (where_sql, params) = usage_filters(query, projects_complete);
        let instance_names: HashMap<String, String> = usage_instances
            .iter()
            .map(|instance| (instance.id.clone(), instance.name.clone()))
            .collect();
        let include_hour = query
            .from_timestamp
            .is_some_and(|from_ts| to.saturating_sub(from_ts) <= 48 * 3600);
        let aggregates = query_report_aggregates(
            &conn,
            &where_sql,
            &params,
            query.include_projects,
            include_hour,
            &instance_names,
        )?;
        // Keep every model/project choice available within the selected time and instance.
        let has_dimension_filter =
            query.model.is_some() || query.project_key.is_some() || query.session_id.is_some();
        let (model_options, project_options) = if query.include_projects && has_dimension_filter {
            let scope = CodexSessionUsageQuery {
                model: None,
                project_key: None,
                session_id: None,
                ..query.clone()
            };
            usage_filter_options(&conn, &scope, &rollup_source)?
        } else {
            (
                usage_options(&aggregates.by_model),
                usage_options(&aggregates.by_project),
            )
        };
        let totals = aggregates.totals;
        let by_model = aggregates.by_model;
        let by_instance = aggregates.by_instance;
        let by_project = aggregates.by_project;
        let by_day = aggregates.by_day;
        let by_hour = aggregates.by_hour;

        let files_tracked = conn
            .query_row("SELECT COUNT(*) FROM session_log_sync", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as u64;
        let event_count = conn
            .query_row(
                &format!(
                    "SELECT COALESCE(SUM(request_count),0) FROM ({})",
                    session_report_source(&conn, 0, i64::MAX / 2)
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            .max(0) as u64;

        Ok(CodexSessionUsageReport {
            totals,
            by_model,
            by_instance,
            by_project,
            projects_pending: query.include_projects && !projects_complete,
            model_options,
            project_options,
            by_day,
            by_hour,
            instances: usage_instances
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

#[derive(Debug, Clone)]
struct DiscoveredRolloutFile {
    path: PathBuf,
    modified: i64,
    size: u64,
}

impl AsRef<Path> for DiscoveredRolloutFile {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
fn collect_codex_session_files(codex_dir: &Path) -> Vec<PathBuf> {
    collect_codex_session_files_with_meta(codex_dir)
        .into_iter()
        .map(|file| file.path)
        .collect()
}

fn collect_codex_session_files_with_meta(codex_dir: &Path) -> Vec<DiscoveredRolloutFile> {
    let mut files = Vec::new();
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_jsonl_recursive_with_meta(&sessions_dir, &mut files, 0, 3);
    }
    let active_names: HashSet<_> = files
        .iter()
        .filter_map(|file| {
            logical_rollout_path(&file.path)
                .file_name()
                .map(ToOwned::to_owned)
        })
        .collect();
    let archived_dir = codex_dir.join("archived_sessions");
    if archived_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&archived_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if is_rollout_path(&path)
                    && !logical_rollout_path(&path)
                        .file_name()
                        .is_some_and(|name| active_names.contains(name))
                {
                    if let Some(discovered) = discovered_rollout_from_entry(&entry, path) {
                        files.push(discovered);
                    }
                }
            }
        }
    }
    let plain_paths: HashSet<PathBuf> = files
        .iter()
        .filter(|file| !is_compressed_rollout(&file.path))
        .map(|file| file.path.clone())
        .collect();
    files.retain(|file| {
        if !is_compressed_rollout(&file.path) {
            return true;
        }
        let logical = logical_rollout_path(&file.path);
        !plain_paths.contains::<Path>(&logical)
    });
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn discovered_rollout_from_entry(
    entry: &fs::DirEntry,
    path: PathBuf,
) -> Option<DiscoveredRolloutFile> {
    let is_symlink = entry
        .file_type()
        .map(|file_type| file_type.is_symlink())
        .unwrap_or(false);
    let metadata = if is_symlink {
        fs::metadata(&path).ok()?
    } else {
        entry.metadata().ok()?
    };
    if !metadata.is_file() {
        return None;
    }
    Some(DiscoveredRolloutFile {
        path,
        modified: metadata_modified_nanos(&metadata),
        size: metadata.len(),
    })
}

fn collect_jsonl_recursive_with_meta(
    dir: &Path,
    files: &mut Vec<DiscoveredRolloutFile>,
    depth: u32,
    max_depth: u32,
) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry
            .file_type()
            .map(|file_type| file_type.is_dir() || (file_type.is_symlink() && path.is_dir()))
            .unwrap_or_else(|_| path.is_dir());
        if is_dir && depth < max_depth {
            collect_jsonl_recursive_with_meta(&path, files, depth + 1, max_depth);
        } else if !is_dir && is_rollout_path(&path) {
            if let Some(discovered) = discovered_rollout_from_entry(&entry, path) {
                files.push(discovered);
            }
        }
    }
}

fn is_rollout_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_rollout_filename)
}

fn is_rollout_filename(file_name: &str) -> bool {
    let Some(stem) = rollout_filename_stem(file_name) else {
        return false;
    };
    stem.get(stem.len().saturating_sub(36)..)
        .is_some_and(|candidate| uuid::Uuid::parse_str(candidate).is_ok())
}

fn is_compressed_rollout(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "zst")
}

fn logical_rollout_path(path: &Path) -> std::borrow::Cow<'_, Path> {
    if is_compressed_rollout(path) {
        std::borrow::Cow::Owned(path.with_extension(""))
    } else {
        std::borrow::Cow::Borrowed(path)
    }
}

fn rollout_filename_stem(file_name: &str) -> Option<&str> {
    file_name.strip_prefix("rollout-")?;
    file_name
        .strip_suffix(".jsonl")
        .or_else(|| file_name.strip_suffix(".jsonl.zst"))
}

fn thread_id_from_filename(path: &Path) -> Option<String> {
    let stem = rollout_filename_stem(path.file_name()?.to_str()?)?;
    let candidate = stem.get(stem.len().checked_sub(36)?..)?;
    uuid::Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

// Continuation/branch rollouts are named `...-<root-uuid>_<leaf-uuid>.jsonl`, so the stem
// carries two UUIDs joined by `_`. `thread_id_from_filename` returns the trailing leaf,
// while `session_meta.id` is the root before `_`. Returns that root id so such files can be
// treated as forks of the root (issue #2897), or None for a plain single-uuid rollout.
fn continuation_root_thread_id(file_path: &Path) -> Option<String> {
    const UUID_LEN: usize = 36;
    let stem = rollout_filename_stem(file_path.file_name()?.to_str()?)?;
    let leaf_start = stem.len().checked_sub(UUID_LEN)?;
    uuid::Uuid::parse_str(stem.get(leaf_start..)?).ok()?;
    let sep = leaf_start.checked_sub(1)?;
    if stem.get(sep..leaf_start)? != "_" {
        return None;
    }
    let root_start = sep.checked_sub(UUID_LEN)?;
    uuid::Uuid::parse_str(stem.get(root_start..sep)?)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

type RolloutIndex = HashMap<String, Vec<PathBuf>>;

fn build_rollout_index<P: AsRef<Path>>(files: &[P]) -> RolloutIndex {
    let mut index = RolloutIndex::new();
    for entry in files {
        let path = entry.as_ref();
        if let Some(thread_id) = thread_id_from_filename(path) {
            index.entry(thread_id).or_default().push(path.to_path_buf());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
    }
    index
}

fn load_cursors(conn: &Connection) -> Result<HashMap<String, SyncCursorState>, String> {
    let mut statement = conn
        .prepare(
            "SELECT
                s.file_path,
                s.last_modified,
                s.last_size,
                s.last_line_offset,
                CASE WHEN c.file_path IS NOT NULL THEN 1 ELSE 0 END,
                CASE WHEN c.state IS NOT NULL AND json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.complete') AS INTEGER), 0) ELSE 0 END,
                CASE WHEN c.state IS NOT NULL AND json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.compressed') AS INTEGER), 0) ELSE 0 END,
                CASE WHEN c.state IS NOT NULL AND json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.version') AS INTEGER), 0) ELSE 0 END,
                CASE WHEN c.state IS NOT NULL AND json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.parent') AS TEXT), '') ELSE '' END,
                CASE WHEN c.state IS NOT NULL AND json_valid(c.state) THEN COALESCE(CAST(json_extract(c.state, '$.byte_offset') AS INTEGER), 0) ELSE 0 END
             FROM session_log_sync s
             LEFT JOIN session_parser_checkpoints c USING (file_path)",
        )
        .map_err(|error| format!("读取会话用量游标失败: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let version = row.get::<_, i64>(7)?;
            let parent = row.get::<_, String>(8)?;
            let checkpoint_version_valid = version == i64::from(PARSER_VERSION)
                || (version == 1 && parent != "Parent");
            Ok((
                row.get::<_, String>(0)?,
                SyncCursorState {
                    last_modified: row.get::<_, i64>(1)?,
                    last_size: row.get::<_, i64>(2)?,
                    last_offset: row.get::<_, i64>(3)?,
                    checkpoint_exists: row.get::<_, i64>(4)? != 0,
                    checkpoint_complete: row.get::<_, i64>(5)? != 0,
                    checkpoint_compressed: row.get::<_, i64>(6)? != 0,
                    checkpoint_version_valid,
                    checkpoint_byte_offset: row.get::<_, i64>(9)?.max(0) as u64,
                },
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
    conn: &Connection,
    file_path: &Path,
    cursors: &HashMap<String, SyncCursorState>,
) -> Result<Option<(i64, i64, i64)>, String> {
    if file_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        != Some("archived_sessions")
    {
        return Ok(None);
    }
    let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    let slash_suffix = format!("/{file_name}");
    let backslash_suffix = format!("\\{file_name}");
    let Some((prior_path, &prior_cursor)) = cursors
        .iter()
        .filter(|(path, _)| {
            path.as_str() != file_path.to_string_lossy().as_ref()
                && (path.ends_with(&slash_suffix) || path.ends_with(&backslash_suffix))
        })
        .max_by_key(|(_, state)| (state.last_offset, state.last_modified, state.last_size))
    else {
        return Ok(None);
    };
    let prior_checkpoint: Option<String> = conn
        .query_row(
            "SELECT state FROM session_parser_checkpoints WHERE file_path=?1",
            [prior_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let compressed = prior_checkpoint
        .and_then(|state| serde_json::from_str::<ParserCheckpoint>(&state).ok())
        .is_some_and(|checkpoint| checkpoint.compressed);
    Ok((!compressed).then_some((
        prior_cursor.last_modified,
        prior_cursor.last_size,
        prior_cursor.last_offset,
    )))
}

fn is_cursor_unchanged_completed(
    cursor: &SyncCursorState,
    file_modified: i64,
    file_size: u64,
    compressed: bool,
) -> bool {
    cursor.last_modified > 0
        && file_modified == cursor.last_modified
        && file_size == cursor.last_size.max(0) as u64
        && cursor.last_offset > 0
        && if cursor.checkpoint_exists {
            cursor.checkpoint_complete
                && cursor.checkpoint_compressed == compressed
                && cursor.checkpoint_version_valid
                && (compressed || cursor.checkpoint_byte_offset <= file_size)
        } else {
            !compressed
        }
}

struct ParallelParseJob {
    file: DiscoveredRolloutFile,
    logical_path: PathBuf,
    file_path_str: String,
    last_size: i64,
    last_offset: i64,
    prior_offset: u64,
    checkpoint: Option<ParserCheckpoint>,
}

struct ParallelParseOutcome {
    job: ParallelParseJob,
    parsed: Result<ParsedCodexFile, String>,
    project: Option<RolloutUsageProject>,
}

struct ReadyFileCommit {
    file_path_str: String,
    logical_path: PathBuf,
    file_size: u64,
    cursor_modified: i64,
    line_offset: i64,
    checkpoint: ParserCheckpoint,
    project: Option<RolloutUsageProject>,
    has_billable_tokens: bool,
    root_thread_id: String,
    to_insert: Vec<(ParsedTokenEvent, u32)>,
    pre_skipped: u32,
}

fn sync_instance_files(
    conn: &mut Connection,
    instance: &UsageInstance,
    files: &[DiscoveredRolloutFile],
    rollout_index: &RolloutIndex,
    cursors: &mut HashMap<String, SyncCursorState>,
    no_existing_checkpoints: bool,
    result: &mut CodexSessionUsageSyncResult,
) -> Result<(), String> {
    let mut parallel_jobs = Vec::new();
    let mut serial_files = Vec::new();

    for file in files {
        if crate::modules::app_lifecycle::is_shutdown_started() {
            return Ok(());
        }
        let logical_path = logical_rollout_path(&file.path).into_owned();
        let file_path_str = logical_path.to_string_lossy().into_owned();
        let compressed = is_compressed_rollout(&file.path);

        if cursors.get(&file_path_str).is_some_and(|cursor| {
            is_cursor_unchanged_completed(cursor, file.modified, file.size, compressed)
        }) {
            continue;
        }

        let (last_modified, last_size, last_offset) = match cursors.get(&file_path_str) {
            Some(cursor) => (cursor.last_modified, cursor.last_size, cursor.last_offset),
            None if compressed || cursors.is_empty() => (0, 0, 0),
            None => inherit_archived_cursor(conn, &file.path, cursors)?.unwrap_or((0, 0, 0)),
        };

        let (checkpoint, checkpoint_exists) = if no_existing_checkpoints {
            (None, false)
        } else {
            let checkpoint = if last_modified > 0
                && file.modified != last_modified
                && file.size == last_size.max(0) as u64
            {
                None
            } else {
                load_parser_checkpoint(conn, &file.path, file.size, last_size)?
            };
            let checkpoint_exists = checkpoint.is_some()
                || conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM session_parser_checkpoints WHERE file_path=?1)",
                        [&file_path_str],
                        |r| r.get::<_, bool>(0),
                    )
                    .map_err(|e| e.to_string())?;
            (checkpoint, checkpoint_exists)
        };

        if file.size >= last_size.max(0) as u64
            && file.modified <= last_modified
            && last_offset > 0
            && (checkpoint.as_ref().is_some_and(|c| c.complete)
                || (!compressed && !checkpoint_exists))
        {
            continue;
        }

        if let Ok(mut caches) = replay_caches().lock() {
            if let Some(pending) = caches.pending.get(&file.path).cloned() {
                if pending.modified == file.modified && pending.size == file.size {
                    match &pending.reason {
                        PendingReason::MissingParent(parent)
                            if !rollout_index.contains_key(parent) =>
                        {
                            result.deferred_files = result.deferred_files.saturating_add(1);
                            continue;
                        }
                        PendingReason::Stable(_) => {
                            result.deferred_files = result.deferred_files.saturating_add(1);
                            continue;
                        }
                        _ => {
                            caches.pending.remove(&file.path);
                        }
                    }
                }
            }
        }

        let rewrite =
            file.size < last_size.max(0) as u64 || (checkpoint.is_none() && checkpoint_exists);
        if rewrite || file.size > PARALLEL_PARSE_MAX_FILE_BYTES {
            serial_files.push(file.clone());
            continue;
        }

        let prior_offset = checkpoint.as_ref().map(|c| c.byte_offset).unwrap_or(0);
        let effective_last_offset = if compressed && checkpoint.is_none() {
            0
        } else {
            last_offset
        };
        parallel_jobs.push(ParallelParseJob {
            file: file.clone(),
            logical_path,
            file_path_str,
            last_size,
            last_offset: effective_last_offset,
            prior_offset,
            checkpoint,
        });
    }

    if parallel_jobs.len() < PARALLEL_PARSE_MIN_FILES {
        serial_files.extend(parallel_jobs.into_iter().map(|job| job.file));
    } else {
        // LPT scheduling: start largest rollout files first to minimize tail latency.
        parallel_jobs.sort_by(|left, right| right.file.size.cmp(&left.file.size));
        let worker_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 8)
            .min(parallel_jobs.len());
        let job_queue = Mutex::new(parallel_jobs.into_iter());
        let (tx, rx) = std::sync::mpsc::sync_channel::<ParallelParseOutcome>(worker_count * 2);

        let scope_result: Result<(), String> = std::thread::scope(|scope| {
            for _ in 0..worker_count {
                let worker_tx = tx.clone();
                let queue_ref = &job_queue;
                scope.spawn(move || loop {
                    if crate::modules::app_lifecycle::is_shutdown_started() {
                        break;
                    }
                    let Some(mut job) = queue_ref.lock().ok().and_then(|mut q| q.next()) else {
                        break;
                    };
                    let mut reader = UsageReaderCache::default();
                    let checkpoint = job.checkpoint.take();
                    let parsed = parse_codex_file_with_budget(
                        &job.file.path,
                        thread_id_from_filename(&job.file.path),
                        checkpoint,
                        &mut reader,
                        ParseBatchBudget::parallel(),
                    );
                    let project = match &parsed {
                        Ok(parsed_file)
                            if parsed_file.checkpoint.complete || job.prior_offset == 0 =>
                        {
                            rollout_usage_project(&job.file.path)
                                .map_err(|error| {
                                    tracing::warn!(
                                        "[CODEX-SESSION-USAGE] 项目元数据读取失败 {}: {error}",
                                        job.file.path.display()
                                    );
                                })
                                .ok()
                        }
                        _ => None,
                    };
                    if worker_tx
                        .send(ParallelParseOutcome {
                            job,
                            parsed,
                            project,
                        })
                        .is_err()
                    {
                        break;
                    }
                });
            }
            drop(tx);

            let mut parent_reader = UsageReaderCache::default();
            let mut ready_commits: Vec<ReadyFileCommit> = Vec::new();
            let mut pending_rows: usize = 0;
            let mut loop_result: Result<(), String> = Ok(());

            for outcome in rx.iter() {
                if crate::modules::app_lifecycle::is_shutdown_started() {
                    break;
                }
                if let Err(error) = handle_parallel_outcome(
                    conn,
                    instance,
                    outcome,
                    rollout_index,
                    cursors,
                    &mut parent_reader,
                    &mut ready_commits,
                    &mut pending_rows,
                    &mut serial_files,
                    result,
                ) {
                    loop_result = Err(error);
                    break;
                }
            }

            // Drop receiver before thread scope joins so any blocked worker unblocks immediately.
            drop(rx);

            if !ready_commits.is_empty() {
                if let Err(error) = flush_ready_commits_with_resource(
                    conn,
                    instance,
                    &mut ready_commits,
                    cursors,
                    result,
                ) {
                    if loop_result.is_ok() {
                        loop_result = Err(error);
                    }
                }
            }
            loop_result
        });
        scope_result?;
    }

    for file in &serial_files {
        sync_serial_file_loop(conn, instance, file, rollout_index, cursors, result)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_parallel_outcome(
    conn: &mut Connection,
    instance: &UsageInstance,
    outcome: ParallelParseOutcome,
    rollout_index: &RolloutIndex,
    cursors: &mut HashMap<String, SyncCursorState>,
    parent_reader: &mut UsageReaderCache,
    ready_commits: &mut Vec<ReadyFileCommit>,
    pending_rows: &mut usize,
    followup_serial_files: &mut Vec<DiscoveredRolloutFile>,
    result: &mut CodexSessionUsageSyncResult,
) -> Result<(), String> {
    let ParallelParseOutcome {
        job,
        parsed,
        project,
    } = outcome;
    let mut parsed = match parsed {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                "[CODEX-SESSION-USAGE] 解析失败 {}: {error}",
                job.file.path.display()
            );
            if result.errors.len() < 20 {
                result
                    .errors
                    .push(format!("{}: {error}", job.file.path.display()));
            }
            return Ok(());
        }
    };

    let more = !parsed.checkpoint.complete && parsed.checkpoint.byte_offset > job.prior_offset;
    let cursor_modified = if parsed.checkpoint.complete {
        job.file.modified
    } else {
        0
    };

    if !parsed.has_billable_tokens {
        ready_commits.push(ReadyFileCommit {
            file_path_str: job.file_path_str,
            logical_path: job.logical_path,
            file_size: job.file.size,
            cursor_modified,
            line_offset: parsed.line_offset,
            checkpoint: parsed.checkpoint,
            project,
            has_billable_tokens: false,
            root_thread_id: parsed.root_thread_id.unwrap_or_default(),
            to_insert: Vec::new(),
            pre_skipped: 0,
        });
        if more {
            followup_serial_files.push(job.file);
        }
        if ready_commits.len() >= PARALLEL_COMMIT_BATCH_FILES {
            flush_ready_commits_with_resource(conn, instance, ready_commits, cursors, result)?;
            *pending_rows = 0;
        }
        return Ok(());
    }

    let Some(root_thread_id) = parsed.root_thread_id.clone() else {
        let _ = mark_deferred(
            &job.file.path,
            job.file.modified,
            job.file.size,
            PendingReason::Stable("文件名缺少有效的尾部 UUID".to_string()),
        );
        result.deferred_files = result.deferred_files.saturating_add(1);
        return Ok(());
    };
    if !parsed.root_meta_seen {
        let _ = mark_deferred(
            &job.file.path,
            job.file.modified,
            job.file.size,
            PendingReason::Stable("含计费 token 但尚无 session_meta".to_string()),
        );
        result.deferred_files = result.deferred_files.saturating_add(1);
        return Ok(());
    }

    let replay_prefix = if parsed.checkpoint.replay_closed {
        0
    } else {
        match parsed.parent {
            ParentResolution::None | ParentResolution::Paginated => 0,
            ParentResolution::Deferred => {
                let _ = mark_deferred(
                    &job.file.path,
                    job.file.modified,
                    job.file.size,
                    PendingReason::Stable(
                        parsed
                            .deferred_reason
                            .unwrap_or_else(|| "分叉会话元数据不完整".to_string()),
                    ),
                );
                result.deferred_files = result.deferred_files.saturating_add(1);
                return Ok(());
            }
            ParentResolution::Parent => {
                let Some(parent_id) = parsed.parent_id.as_deref() else {
                    let _ = mark_deferred(
                        &job.file.path,
                        job.file.modified,
                        job.file.size,
                        PendingReason::Stable("分叉会话缺少父会话 ID".to_string()),
                    );
                    result.deferred_files = result.deferred_files.saturating_add(1);
                    return Ok(());
                };
                let Some(cutoff) = parsed.root_timestamp else {
                    let _ = mark_deferred(
                        &job.file.path,
                        job.file.modified,
                        job.file.size,
                        PendingReason::Stable(
                            "parented rollout 的 root meta 缺少有效 timestamp".to_string(),
                        ),
                    );
                    result.deferred_files = result.deferred_files.saturating_add(1);
                    return Ok(());
                };
                if !ready_commits.is_empty() {
                    flush_ready_commits_with_resource(
                        conn,
                        instance,
                        ready_commits,
                        cursors,
                        result,
                    )?;
                    *pending_rows = 0;
                }
                let replay = loop {
                    if crate::modules::app_lifecycle::is_shutdown_started() {
                        return Ok(());
                    }
                    let Some(resource) = crate::modules::background_maintenance::try_resource()
                    else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    let outcome = resolve_parent_replay_complete(
                        conn,
                        parent_id,
                        cutoff,
                        rollout_index,
                        &parsed.token_events,
                        &mut parsed.checkpoint,
                        parent_reader,
                    );
                    drop(resource);
                    break outcome;
                };
                match replay {
                    Ok(ParentReplay::Pending) => {
                        followup_serial_files.push(job.file);
                        return Ok(());
                    }
                    Ok(ParentReplay::Matched(count)) => count,
                    Err(reason) => {
                        let pending_reason = if rollout_index.contains_key(parent_id) {
                            PendingReason::Retryable(reason)
                        } else {
                            PendingReason::MissingParent(parent_id.to_string())
                        };
                        let _ = mark_deferred(
                            &job.file.path,
                            job.file.modified,
                            job.file.size,
                            pending_reason,
                        );
                        result.deferred_files = result.deferred_files.saturating_add(1);
                        return Ok(());
                    }
                }
            }
            ParentResolution::Continuation => {
                match (parsed.parent_id.as_deref(), parsed.root_timestamp) {
                    (Some(parent_id), Some(cutoff)) => {
                        if !ready_commits.is_empty() {
                            flush_ready_commits_with_resource(
                                conn,
                                instance,
                                ready_commits,
                                cursors,
                                result,
                            )?;
                            *pending_rows = 0;
                        }
                        let replay = loop {
                            if crate::modules::app_lifecycle::is_shutdown_started() {
                                return Ok(());
                            }
                            let Some(resource) =
                                crate::modules::background_maintenance::try_resource()
                            else {
                                std::thread::sleep(Duration::from_millis(10));
                                continue;
                            };
                            let outcome = resolve_parent_replay_complete(
                                conn,
                                parent_id,
                                cutoff,
                                rollout_index,
                                &parsed.token_events,
                                &mut parsed.checkpoint,
                                parent_reader,
                            );
                            drop(resource);
                            break outcome;
                        };
                        match replay {
                            Ok(ParentReplay::Pending) => {
                                followup_serial_files.push(job.file);
                                return Ok(());
                            }
                            Ok(ParentReplay::Matched(count)) => count,
                            Err(_) => 0,
                        }
                    }
                    _ => 0,
                }
            }
        }
    };
    parsed.checkpoint.replay_closed |=
        matches!(parsed.parent, ParentResolution::None | ParentResolution::Paginated)
            || replay_prefix < parsed.token_events.len();

    if let Ok(mut caches) = replay_caches().lock() {
        caches.pending.remove(&job.file.path);
    }

    let mut to_insert = Vec::new();
    let mut pre_skipped = 0u32;
    for (token_offset, event) in parsed.token_events.into_iter().enumerate() {
        let Some(event_index) = event.event_index else {
            continue;
        };
        if token_offset < replay_prefix {
            if event.line_offset > job.last_offset {
                pre_skipped = pre_skipped.saturating_add(1);
            }
            continue;
        }
        if event.line_offset <= job.last_offset && job.file.size >= job.last_size.max(0) as u64 {
            continue;
        }
        to_insert.push((event, event_index));
    }

    *pending_rows = pending_rows.saturating_add(to_insert.len());
    ready_commits.push(ReadyFileCommit {
        file_path_str: job.file_path_str,
        logical_path: job.logical_path,
        file_size: job.file.size,
        cursor_modified,
        line_offset: parsed.line_offset,
        checkpoint: parsed.checkpoint,
        project,
        has_billable_tokens: true,
        root_thread_id,
        to_insert,
        pre_skipped,
    });
    if more {
        followup_serial_files.push(job.file);
    }

    if ready_commits.len() >= PARALLEL_COMMIT_BATCH_FILES
        || *pending_rows >= PARALLEL_COMMIT_BATCH_ROWS
    {
        flush_ready_commits_with_resource(conn, instance, ready_commits, cursors, result)?;
        *pending_rows = 0;
    }
    Ok(())
}

fn flush_ready_commits_with_resource(
    conn: &mut Connection,
    instance: &UsageInstance,
    ready_commits: &mut Vec<ReadyFileCommit>,
    cursors: &mut HashMap<String, SyncCursorState>,
    result: &mut CodexSessionUsageSyncResult,
) -> Result<(), String> {
    if ready_commits.is_empty() {
        return Ok(());
    }
    let resource = loop {
        if let Some(resource) = crate::modules::background_maintenance::try_resource() {
            break resource;
        }
        if crate::modules::app_lifecycle::is_shutdown_started() {
            ready_commits.clear();
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    let mut per_file_counts: Vec<(u32, u32)> = Vec::with_capacity(ready_commits.len());
    {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| format!("开启会话用量批量写入事务失败: {error}"))?;
        {
            let mut statement = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO session_usage_events (
                        request_id, instance_id, instance_name, session_id, model,
                        timestamp, input_tokens, cached_input_tokens, output_tokens, file_path
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|error| format!("准备会话用量写入失败: {error}"))?;
            for commit in ready_commits.iter() {
                let mut imported = 0u32;
                let mut skipped = commit.pre_skipped;
                for (event, event_index) in &commit.to_insert {
                    let request_id =
                        format!("{REQUEST_ID_PREFIX}:{}:{event_index}", commit.root_thread_id);
                    let changed = statement
                        .execute(params![
                            request_id,
                            instance.id,
                            instance.name,
                            commit.root_thread_id,
                            event.model,
                            event.timestamp.unwrap_or(0),
                            event.delta.input as i64,
                            event.delta.cached_input as i64,
                            event.delta.output as i64,
                            commit.file_path_str,
                        ])
                        .map_err(|error| format!("写入会话用量失败: {error}"))?;
                    if changed > 0 {
                        imported = imported.saturating_add(1);
                    } else {
                        skipped = skipped.saturating_add(1);
                    }
                }
                per_file_counts.push((imported, skipped));
            }
        }
        for commit in ready_commits.iter() {
            upsert_cursor_on_conn(
                &tx,
                &commit.file_path_str,
                &instance.id,
                commit.cursor_modified,
                commit.file_size,
                commit.line_offset,
            )?;
            save_parser_checkpoint(&tx, &commit.logical_path, &commit.checkpoint)?;
            if commit.has_billable_tokens || commit.project.is_some() {
                save_usage_project(
                    &tx,
                    &commit.file_path_str,
                    &commit.root_thread_id,
                    commit.project.as_ref(),
                )?;
            }
            publish_rewrite(&tx, &commit.file_path_str, commit.checkpoint.complete)?;
        }
        tx.commit()
            .map_err(|error| format!("提交会话用量批量写入失败: {error}"))?;
    }
    drop(resource);

    for (commit, (imported, skipped)) in ready_commits.drain(..).zip(per_file_counts) {
        result.imported = result.imported.saturating_add(imported);
        result.skipped = result.skipped.saturating_add(skipped);
        if imported > 0 || skipped > 0 {
            result.files_changed = result.files_changed.saturating_add(1);
        }
        cursors.insert(
            commit.file_path_str,
            SyncCursorState::from_checkpoint(
                commit.cursor_modified,
                commit.file_size as i64,
                commit.line_offset,
                &commit.checkpoint,
            ),
        );
    }
    Ok(())
}

fn sync_serial_file_loop(
    conn: &mut Connection,
    instance: &UsageInstance,
    file: &DiscoveredRolloutFile,
    rollout_index: &RolloutIndex,
    cursors: &mut HashMap<String, SyncCursorState>,
    result: &mut CodexSessionUsageSyncResult,
) -> Result<(), String> {
    let mut file_changed = false;
    let mut first_attempt = true;
    let mut reader = UsageReaderCache::default();
    let mut parent_reader = UsageReaderCache::default();
    loop {
        if crate::modules::app_lifecycle::is_shutdown_started() {
            return Ok(());
        }
        let Some(resource) = crate::modules::background_maintenance::try_resource() else {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        };
        let known_meta = first_attempt.then_some((file.modified, file.size));
        first_attempt = false;
        let file_result = sync_single_file(
            conn,
            instance,
            &file.path,
            known_meta,
            rollout_index,
            cursors,
            &mut reader,
            &mut parent_reader,
        );
        drop(resource);
        match file_result {
            Ok(file_result) => {
                result.imported = result.imported.saturating_add(file_result.imported);
                result.skipped = result.skipped.saturating_add(file_result.skipped);
                if !file_changed && (file_result.imported > 0 || file_result.skipped > 0) {
                    result.files_changed = result.files_changed.saturating_add(1);
                    file_changed = true;
                }
                if file_result.deferred {
                    result.deferred_files = result.deferred_files.saturating_add(1);
                }
                if file_result.more {
                    std::thread::yield_now();
                    continue;
                }
            }
            Err(error) => {
                tracing::warn!(
                    "[CODEX-SESSION-USAGE] 解析失败 {}: {error}",
                    file.path.display()
                );
                if result.errors.len() < 20 {
                    result
                        .errors
                        .push(format!("{}: {error}", file.path.display()));
                }
            }
        }
        break;
    }
    Ok(())
}

fn sync_single_file(
    conn: &mut Connection,
    instance: &UsageInstance,
    file_path: &Path,
    known_meta: Option<(i64, u64)>,
    rollout_index: &RolloutIndex,
    cursors: &mut HashMap<String, SyncCursorState>,
    reader: &mut UsageReaderCache,
    parent_reader: &mut UsageReaderCache,
) -> Result<FileSyncResult, String> {
    let logical_path = logical_rollout_path(file_path);
    let file_path_str = logical_path.to_string_lossy().into_owned();
    let compressed = is_compressed_rollout(file_path);
    let (file_modified, file_size) = match known_meta {
        Some(meta) => meta,
        None => {
            let metadata =
                fs::metadata(file_path).map_err(|error| format!("无法读取文件元数据: {error}"))?;
            (metadata_modified_nanos(&metadata), metadata.len())
        }
    };

    // Fast path: skip unchanged completed files without per-file SQLite queries or SHA-256 disk reads.
    if cursors.get(&file_path_str).is_some_and(|cursor| {
        is_cursor_unchanged_completed(cursor, file_modified, file_size, compressed)
    }) {
        return Ok(FileSyncResult::default());
    }

    let (last_modified, last_size, last_offset) = match cursors.get(&file_path_str) {
        Some(cursor) => (cursor.last_modified, cursor.last_size, cursor.last_offset),
        // A compressed rollout moved to the archive must replay from zero
        // and deduplicate by ID because the cursor uses different offsets.
        None if compressed => (0, 0, 0),
        None => inherit_archived_cursor(conn, file_path, cursors)?.unwrap_or((0, 0, 0)),
    };

    let checkpoint = if last_modified > 0
        && file_modified != last_modified
        && file_size == last_size.max(0) as u64
    {
        None
    } else {
        load_parser_checkpoint(conn, file_path, file_size, last_size)?
    };
    let checkpoint_exists = checkpoint.is_some()
        || conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_parser_checkpoints WHERE file_path=?1)",
                [&file_path_str],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?;
    if file_size >= last_size.max(0) as u64
        && file_modified <= last_modified
        && last_offset > 0
        && (checkpoint.as_ref().is_some_and(|c| c.complete) || (!compressed && !checkpoint_exists))
    {
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

    let prior_offset = checkpoint.as_ref().map(|c| c.byte_offset).unwrap_or(0);
    let rewrite =
        file_size < last_size.max(0) as u64 || (checkpoint.is_none() && checkpoint_exists);
    let last_offset = if rewrite || (compressed && checkpoint.is_none()) {
        0
    } else {
        last_offset
    };
    if rewrite && !prepare_rewrite_batch_limit(conn, &file_path_str, FOREGROUND_REWRITE_BATCH_ROWS)? {
        return Ok(FileSyncResult {
            more: true,
            ..Default::default()
        });
    }
    let mut parsed = parse_codex_file_with_budget(
        file_path,
        thread_id_from_filename(file_path),
        checkpoint,
        reader,
        ParseBatchBudget::foreground(),
    )?;
    let more = !parsed.checkpoint.complete && parsed.checkpoint.byte_offset > prior_offset;
    let cursor_modified = if parsed.checkpoint.complete {
        file_modified
    } else {
        0
    };
    let project = if parsed.checkpoint.complete || (!rewrite && prior_offset == 0) {
        rollout_usage_project(file_path)
            .map_err(|error| {
                tracing::warn!("[CODEX-SESSION-USAGE] 项目元数据读取失败 {}: {error}", file_path.display());
            })
            .ok()
    } else {
        None
    };
    if !parsed.has_billable_tokens {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        upsert_cursor_on_conn(
            &tx,
            &file_path_str,
            &instance.id,
            cursor_modified,
            file_size,
            parsed.line_offset,
        )?;
        save_parser_checkpoint(&tx, &logical_path, &parsed.checkpoint)?;
        if let Some(project) = &project {
            save_usage_project(
                &tx,
                &file_path_str,
                parsed.root_thread_id.as_deref().unwrap_or(""),
                Some(project),
            )?;
        }
        publish_rewrite(&tx, &file_path_str, parsed.checkpoint.complete)?;
        tx.commit().map_err(|e| e.to_string())?;
        cursors.insert(
            file_path_str,
            SyncCursorState::from_checkpoint(
                cursor_modified,
                file_size as i64,
                parsed.line_offset,
                &parsed.checkpoint,
            ),
        );
        return Ok(FileSyncResult {
            more,
            ..FileSyncResult::default()
        });
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

    let replay_prefix = if parsed.checkpoint.replay_closed {
        0
    } else {
        match parsed.parent {
            ParentResolution::None | ParentResolution::Paginated => 0,
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
                match resolve_parent_replay_complete(
                    conn,
                    parent_id,
                    cutoff,
                    rollout_index,
                    &parsed.token_events,
                    &mut parsed.checkpoint,
                    parent_reader,
                ) {
                    Ok(ParentReplay::Pending) => {
                        return Ok(FileSyncResult {
                            more: true,
                            ..Default::default()
                        })
                    }
                    Ok(ParentReplay::Matched(count)) => count,
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
            ParentResolution::Continuation => {
                // Continuation/branch rollout `...-<root-uuid>_<leaf-uuid>.jsonl`: the parent is
                // only used to skip replayed context. If the root rollout is unavailable, count
                // every event rather than dropping the file.
                match (parsed.parent_id.as_deref(), parsed.root_timestamp) {
                    (Some(parent_id), Some(cutoff)) => {
                        match resolve_parent_replay_complete(
                            conn,
                            parent_id,
                            cutoff,
                            rollout_index,
                            &parsed.token_events,
                            &mut parsed.checkpoint,
                            parent_reader,
                        ) {
                            Ok(ParentReplay::Pending) => {
                                return Ok(FileSyncResult {
                                    more: true,
                                    ..Default::default()
                                })
                            }
                            Ok(ParentReplay::Matched(count)) => count,
                            Err(_) => 0,
                        }
                    }
                    _ => 0,
                }
            }
        }
    };
    parsed.checkpoint.replay_closed |= matches!(parsed.parent, ParentResolution::None | ParentResolution::Paginated)
        || replay_prefix < parsed.token_events.len();

    if let Ok(mut caches) = replay_caches().lock() {
        caches.pending.remove(file_path);
    }

    let mut to_insert = Vec::new();
    let mut result = FileSyncResult {
        more,
        ..FileSyncResult::default()
    };
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

    {
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
            for (event, event_index) in &to_insert {
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
        upsert_cursor_on_conn(
            &tx,
            &file_path_str,
            &instance.id,
            cursor_modified,
            file_size,
            parsed.line_offset,
        )?;
        save_parser_checkpoint(&tx, &logical_path, &parsed.checkpoint)?;
        save_usage_project(&tx, &file_path_str, &root_thread_id, project.as_ref())?;
        publish_rewrite(&tx, &file_path_str, parsed.checkpoint.complete)?;
        tx.commit()
            .map_err(|error| format!("提交会话用量写入失败: {error}"))?;
        cursors.insert(
            file_path_str,
            SyncCursorState::from_checkpoint(
                cursor_modified,
                file_size as i64,
                parsed.line_offset,
                &parsed.checkpoint,
            ),
        );
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

#[cfg(test)]
fn parse_codex_file(
    file_path: &Path,
    root_thread_id: Option<String>,
) -> Result<ParsedCodexFile, String> {
    let mut reader = UsageReaderCache::default();
    let mut parsed = parse_codex_file_batch(file_path, root_thread_id.clone(), None, &mut reader)?;
    while !parsed.checkpoint.complete {
        let next = parse_codex_file_batch(
            file_path,
            root_thread_id.clone(),
            Some(parsed.checkpoint.clone()),
            &mut reader,
        )?;
        if next.checkpoint.byte_offset == parsed.checkpoint.byte_offset {
            break;
        }
        parsed.token_events.extend(next.token_events);
        parsed.line_offset = next.line_offset;
        parsed.has_billable_tokens = next.has_billable_tokens;
        parsed.checkpoint = next.checkpoint;
    }
    Ok(parsed)
}

#[cfg(test)]
fn parse_codex_file_batch(
    file_path: &Path,
    root_thread_id: Option<String>,
    checkpoint: Option<ParserCheckpoint>,
    cache: &mut UsageReaderCache,
) -> Result<ParsedCodexFile, String> {
    parse_codex_file_with_budget(
        file_path,
        root_thread_id,
        checkpoint,
        cache,
        ParseBatchBudget::background(),
    )
}

fn parse_codex_file_with_budget(
    file_path: &Path,
    root_thread_id: Option<String>,
    checkpoint: Option<ParserCheckpoint>,
    cache: &mut UsageReaderCache,
    budget: ParseBatchBudget,
) -> Result<ParsedCodexFile, String> {
    let previous = checkpoint.unwrap_or_default();
    let compressed = is_compressed_rollout(file_path);
    let reader = cache.at(file_path, previous.byte_offset)?;
    let mut root_meta_seen = previous.root_meta_seen;
    let mut root_timestamp = previous.root_timestamp;
    let mut parent = previous.parent;
    let mut parent_id = previous.parent_id.clone();
    let mut deferred_reason = previous.deferred_reason.clone();
    let mut current_model = previous.current_model.clone();
    let mut total_high_water = previous.total_high_water.clone();
    let mut last_signature_by_source: HashMap<Option<String>, TokenUsageSignature> =
        previous.signatures.iter().cloned().collect();
    let mut previous_token_signature = previous.previous_token_signature.clone();
    let mut event_index = previous.event_index;
    let mut token_events = Vec::new();
    let mut line_offset = previous.line_offset;
    let mut has_billable_tokens = previous.has_billable_tokens;
    let mut discard_line = previous.discard_line;
    let started = std::time::Instant::now();
    let (byte_offset, complete) = loop {
        let position = reader.offset;
        if line_offset > previous.line_offset
            && (position.saturating_sub(previous.byte_offset) >= budget.max_bytes
                || token_events.len() >= budget.max_events
                || started.elapsed() >= budget.max_time)
        {
            break (position, reader.at_eof()?);
        }
        let Some(line) = read_usage_line(reader, &mut discard_line)? else {
            break (position, true);
        };
        // A plain rollout can grow, so retry its unterminated line later.
        if !compressed && !line.ends_with('\n') && !line.is_empty() {
            break (position, false);
        }
        line_offset += 1;
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
                            let base = &payload["history_base"];
                            let first_ordinal = value["ordinal"].as_u64();
                            parent = if payload["history_mode"] == "paginated"
                                && base["thread_id"] == parent_thread_id
                                && base["end_byte_offset"].as_u64().is_some()
                                && first_ordinal.is_some_and(|start| base["end_ordinal_exclusive"].as_u64() == Some(start))
                            {
                                // Parent history is referenced by byte/ordinal, not copied into this file.
                                ParentResolution::Paginated
                            } else {
                                ParentResolution::Parent
                            };
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
                        // The trailing leaf UUID (root_thread_id) differs from the meta id. For
                        // the continuation/branch form `...-<root-uuid>_<leaf-uuid>.jsonl` the meta
                        // id is the root before `_`; treat it as a fork of the root so replayed
                        // parent context is skipped and only new turns are counted, instead of
                        // dropping the whole rollout (issue #2897).
                        let continuation_root = continuation_root_thread_id(file_path)
                            .filter(|root_uuid| *root_uuid == meta_id);
                        if let Some(root_uuid) = continuation_root {
                            if matches!(parent, ParentResolution::None) {
                                parent = ParentResolution::Continuation;
                                parent_id = Some(root_uuid);
                            }
                        } else {
                            parent = ParentResolution::Deferred;
                            deferred_reason = Some(format!(
                                "文件名线程 ID ({filename_id}) 与 root meta ID ({meta_id}) 不一致"
                            ));
                        }
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
                    if !last_signature_by_source.contains_key(&snapshot_source)
                        && last_signature_by_source.len() >= 1024
                    {
                        return Err("usage snapshot source count exceeds checkpoint budget".into());
                    }
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
                    if parent == ParentResolution::Paginated && total_high_water.is_none() {
                        // The first cumulative snapshot also includes the ancestor's usage.
                        // Without a per-turn counter it can only establish a baseline.
                        DeltaTokens::default()
                    } else {
                        compute_delta(&total_high_water, total)
                    }
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
    };
    let checkpoint = ParserCheckpoint {
        version: PARSER_VERSION,
        byte_offset,
        line_offset,
        root_meta_seen,
        root_timestamp,
        parent,
        parent_id: parent_id.clone(),
        deferred_reason: deferred_reason.clone(),
        current_model,
        total_high_water,
        signatures: last_signature_by_source.into_iter().collect(),
        previous_token_signature,
        event_index,
        has_billable_tokens,
        replay_matched: previous.replay_matched,
        replay_closed: previous.replay_closed,
        anchor: checkpoint_anchor(file_path, byte_offset)?,
        complete,
        discard_line,
        compressed,
    };
    Ok(ParsedCodexFile {
        checkpoint,
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

struct ReportAggregates {
    totals: CodexSessionUsageTotals,
    by_model: Vec<CodexSessionUsageBreakdownRow>,
    by_instance: Vec<CodexSessionUsageBreakdownRow>,
    by_project: Vec<CodexSessionUsageBreakdownRow>,
    by_day: Vec<CodexSessionUsageBreakdownRow>,
    by_hour: Vec<CodexSessionUsageBreakdownRow>,
}

#[derive(Default)]
struct BreakdownAccumulator {
    label: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    request_count: u64,
    by_model: HashMap<String, (u64, u64, u64)>,
}

impl BreakdownAccumulator {
    fn add(
        &mut self,
        label: &str,
        model: Option<&str>,
        input: u64,
        cached: u64,
        output: u64,
        requests: u64,
    ) {
        if label > self.label.as_str() {
            self.label.clear();
            self.label.push_str(label);
        }
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.cached_input_tokens = self.cached_input_tokens.saturating_add(cached);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.request_count = self.request_count.saturating_add(requests);
        if let Some(model) = model {
            let entry = self.by_model.entry(model.to_owned()).or_default();
            entry.0 = entry.0.saturating_add(input);
            entry.1 = entry.1.saturating_add(cached);
            entry.2 = entry.2.saturating_add(output);
        }
    }

    fn into_row(
        self,
        key: String,
        label: String,
        price_models: bool,
    ) -> CodexSessionUsageBreakdownRow {
        let estimated_cost_usd = if price_models {
            self.by_model.into_iter().try_fold(0.0_f64, |cost, (model, (input, cached, output))| {
                crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
                    &model, input, cached, output,
                )
                .map(|model_cost| cost + model_cost)
            })
        } else {
            None
        };
        CodexSessionUsageBreakdownRow {
            estimated_cost_usd,
            key,
            label,
            input_tokens: self.input_tokens,
            cached_input_tokens: self.cached_input_tokens,
            output_tokens: self.output_tokens,
            total_tokens: self.input_tokens.saturating_add(self.output_tokens),
            request_count: self.request_count,
        }
    }
}

struct CachedUsageBucket {
    timestamp: i64,
    instance_id: String,
    instance_name: String,
    model: String,
    project_key: String,
    project_path: String,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    request_count: u64,
}

struct UsageBucketSnapshot {
    db_path: PathBuf,
    revision: u64,
    buckets: Vec<CachedUsageBucket>,
    max_updated_at: i64,
    projects_complete: bool,
    files_tracked: u64,
    event_count: u64,
    last_synced_at: Option<i64>,
    deferred_files: u32,
    last_error_count: u32,
}

fn snapshot_covers_query(snapshot: &UsageBucketSnapshot, query: &CodexSessionUsageQuery) -> bool {
    if query
        .session_id
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return false;
    }
    let from_ok = query
        .from_timestamp
        .map_or(true, |from| from <= 0 || from % 60 == 0);
    let to_ok = query.to_timestamp.map_or(true, |to| {
        to >= 0 && (to >= snapshot.max_updated_at || (to + 1) % 60 == 0)
    });
    from_ok && to_ok
}

fn load_usage_bucket_snapshot(
    conn: &Connection,
    db_path: &Path,
) -> Result<Option<UsageBucketSnapshot>, String> {
    if !SESSION_USAGE_ROLLUP.ready(conn) {
        return Ok(None);
    }
    let active_rewrite: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_rewrite_state WHERE phase>=1)",
            [],
            |row| row.get(0),
        )
        .unwrap_or(true);
    if active_rewrite {
        return Ok(None);
    }
    let revision = USAGE_SNAPSHOT_REVISION.load(Ordering::Acquire);
    let mut statement = conn
        .prepare(
            "SELECT
                b.timestamp,
                b.instance_id,
                COALESCE(MAX(b.instance_name), ''),
                b.model,
                COALESCE(p.project_key, ''),
                COALESCE(MAX(p.project_path), ''),
                COALESCE(SUM(b.input_tokens), 0),
                COALESCE(SUM(b.cached_input_tokens), 0),
                COALESCE(SUM(b.output_tokens), 0),
                COALESCE(SUM(b.request_count), 0),
                COALESCE(MAX(b.updated_at), 0)
             FROM session_usage_v1_buckets b
             LEFT JOIN session_usage_projects p USING (file_path)
             GROUP BY b.timestamp, b.instance_id, b.model, COALESCE(p.project_key, '')",
        )
        .map_err(|error| format!("加载会话用量快照失败: {error}"))?;
    let mut rows = statement
        .query([])
        .map_err(|error| format!("查询会话用量快照失败: {error}"))?;
    let mut buckets = Vec::new();
    let mut max_updated_at = 0i64;
    let mut event_count = 0u64;
    while let Some(row) = rows
        .next()
        .map_err(|error| format!("遍历会话用量快照失败: {error}"))?
    {
        let bucket = (|| -> rusqlite::Result<(CachedUsageBucket, i64)> {
            let request_count = row.get::<_, i64>(9)?.max(0) as u64;
            let updated_at = row.get::<_, i64>(10)?;
            Ok((
                CachedUsageBucket {
                    timestamp: row.get::<_, i64>(0)?,
                    instance_id: row.get::<_, String>(1)?,
                    instance_name: row.get::<_, String>(2)?,
                    model: row.get::<_, String>(3)?,
                    project_key: row.get::<_, String>(4)?,
                    project_path: row.get::<_, String>(5)?,
                    input_tokens: row.get::<_, i64>(6)?.max(0) as u64,
                    cached_input_tokens: row.get::<_, i64>(7)?.max(0) as u64,
                    output_tokens: row.get::<_, i64>(8)?.max(0) as u64,
                    request_count,
                },
                updated_at,
            ))
        })()
        .map_err(|error| format!("解析会话用量快照失败: {error}"))?;
        max_updated_at = max_updated_at.max(bucket.1);
        event_count = event_count.saturating_add(bucket.0.request_count);
        buckets.push(bucket.0);
    }

    let projects_complete = usage_projects_complete(conn);
    let files_tracked = conn
        .query_row("SELECT COUNT(*) FROM session_log_sync", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap_or(0)
        .max(0) as u64;

    Ok(Some(UsageBucketSnapshot {
        db_path: db_path.to_path_buf(),
        revision,
        buckets,
        max_updated_at,
        projects_complete,
        files_tracked,
        event_count,
        last_synced_at: get_meta_i64(conn, "last_synced_at"),
        deferred_files: get_meta_i64(conn, "last_deferred_files")
            .unwrap_or(0)
            .max(0) as u32,
        last_error_count: get_meta_i64(conn, "last_error_count").unwrap_or(0).max(0) as u32,
    }))
}

fn query_report_from_snapshot(
    snapshot: &UsageBucketSnapshot,
    query: &CodexSessionUsageQuery,
    usage_instances: &[UsageInstance],
) -> CodexSessionUsageReport {
    let to = query.to_timestamp.unwrap_or(i64::MAX / 2);
    let include_hour = query
        .from_timestamp
        .is_some_and(|from_ts| to.saturating_sub(from_ts) <= 48 * 3600);
    let instance_names: HashMap<String, String> = usage_instances
        .iter()
        .map(|instance| (instance.id.clone(), instance.name.clone()))
        .collect();
    let filter_instance = query
        .instance_id
        .as_deref()
        .filter(|value| !value.is_empty());
    let filter_model = query.model.as_deref().filter(|value| !value.is_empty());
    let filter_project = query.project_key.as_deref();
    let has_dimension_filter =
        query.model.is_some() || query.project_key.is_some() || query.session_id.is_some();
    let collect_scope_options = query.include_projects && has_dimension_filter;

    let mut totals = CodexSessionUsageTotals::default();
    let mut models: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut instances: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut projects: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut days: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut hours: HashMap<i64, BreakdownAccumulator> = HashMap::new();
    let mut scope_models: HashMap<&str, u64> = HashMap::new();
    let mut scope_projects: HashMap<&str, (&str, u64)> = HashMap::new();
    let mut day_cache: HashMap<i64, String> = HashMap::new();

    for bucket in &snapshot.buckets {
        if let Some(from) = query.from_timestamp {
            if bucket.timestamp < from {
                continue;
            }
        }
        if let Some(to_ts) = query.to_timestamp {
            if bucket.timestamp > to_ts {
                continue;
            }
        }
        if let Some(instance_id) = filter_instance {
            if bucket.instance_id != instance_id {
                continue;
            }
        }

        let bucket_total = bucket.input_tokens.saturating_add(bucket.output_tokens);
        if collect_scope_options {
            let model_entry = scope_models.entry(bucket.model.as_str()).or_insert(0);
            *model_entry = model_entry.saturating_add(bucket_total);
            let project_entry = scope_projects
                .entry(bucket.project_key.as_str())
                .or_insert(("", 0));
            if bucket.project_path.as_str() > project_entry.0 {
                project_entry.0 = bucket.project_path.as_str();
            }
            project_entry.1 = project_entry.1.saturating_add(bucket_total);
        }

        if let Some(model) = filter_model {
            if bucket.model != model {
                continue;
            }
        }
        if let Some(project_key) = filter_project {
            if bucket.project_key != project_key {
                continue;
            }
        }

        totals.input_tokens = totals.input_tokens.saturating_add(bucket.input_tokens);
        totals.cached_input_tokens = totals
            .cached_input_tokens
            .saturating_add(bucket.cached_input_tokens);
        totals.output_tokens = totals.output_tokens.saturating_add(bucket.output_tokens);
        totals.total_tokens = totals.total_tokens.saturating_add(bucket_total);
        totals.request_count = totals.request_count.saturating_add(bucket.request_count);

        models.entry(bucket.model.clone()).or_default().add(
            &bucket.model,
            None,
            bucket.input_tokens,
            bucket.cached_input_tokens,
            bucket.output_tokens,
            bucket.request_count,
        );
        instances
            .entry(bucket.instance_id.clone())
            .or_default()
            .add(
                &bucket.instance_name,
                query.include_projects.then_some(bucket.model.as_str()),
                bucket.input_tokens,
                bucket.cached_input_tokens,
                bucket.output_tokens,
                bucket.request_count,
            );
        if query.include_projects {
            projects
                .entry(bucket.project_key.clone())
                .or_default()
                .add(
                    &bucket.project_path,
                    Some(bucket.model.as_str()),
                    bucket.input_tokens,
                    bucket.cached_input_tokens,
                    bucket.output_tokens,
                    bucket.request_count,
                );
        }
        let day = if bucket.timestamp <= 0 {
            String::new()
        } else {
            let slot = bucket.timestamp / 900;
            day_cache
                .entry(slot)
                .or_insert_with(|| local_day_key(bucket.timestamp))
                .clone()
        };
        days.entry(day).or_default().add(
            "",
            Some(bucket.model.as_str()),
            bucket.input_tokens,
            bucket.cached_input_tokens,
            bucket.output_tokens,
            bucket.request_count,
        );
        if include_hour && bucket.timestamp > 0 {
            let hour_bucket = (bucket.timestamp / 3600) * 3600;
            if hour_bucket > 0 {
                hours.entry(hour_bucket).or_default().add(
                    "",
                    Some(bucket.model.as_str()),
                    bucket.input_tokens,
                    bucket.cached_input_tokens,
                    bucket.output_tokens,
                    bucket.request_count,
                );
            }
        }
    }

    let aggregates = finalize_report_aggregates(
        totals,
        models,
        instances,
        projects,
        days,
        hours,
        query.include_projects,
        &instance_names,
    );

    let (model_options, project_options) = if collect_scope_options {
        let mut model_list: Vec<(&str, u64)> = scope_models.into_iter().collect();
        model_list.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
        let mut project_list: Vec<(&str, &str, u64)> = scope_projects
            .into_iter()
            .map(|(key, (label, total))| (key, label, total))
            .collect();
        project_list
            .sort_by(|left, right| right.2.cmp(&left.2).then_with(|| left.0.cmp(right.0)));
        (
            model_list
                .into_iter()
                .map(|(key, _)| CodexSessionUsageInstanceOption {
                    id: key.to_owned(),
                    name: key.to_owned(),
                })
                .collect(),
            project_list
                .into_iter()
                .map(|(key, label, _)| CodexSessionUsageInstanceOption {
                    id: key.to_owned(),
                    name: label.to_owned(),
                })
                .collect(),
        )
    } else {
        (
            usage_options(&aggregates.by_model),
            usage_options(&aggregates.by_project),
        )
    };

    CodexSessionUsageReport {
        totals: aggregates.totals,
        by_model: aggregates.by_model,
        by_instance: aggregates.by_instance,
        by_project: aggregates.by_project,
        projects_pending: query.include_projects && !snapshot.projects_complete,
        model_options,
        project_options,
        by_day: aggregates.by_day,
        by_hour: aggregates.by_hour,
        instances: usage_instances
            .iter()
            .map(|instance| CodexSessionUsageInstanceOption {
                id: instance.id.clone(),
                name: instance.name.clone(),
            })
            .collect(),
        from_timestamp: query.from_timestamp,
        to_timestamp: query.to_timestamp,
        last_synced_at: snapshot.last_synced_at,
        files_tracked: snapshot.files_tracked,
        event_count: snapshot.event_count,
        deferred_files: snapshot.deferred_files,
        last_error_count: snapshot.last_error_count,
    }
}

#[allow(clippy::too_many_arguments)]
fn finalize_report_aggregates(
    mut totals: CodexSessionUsageTotals,
    models: HashMap<String, BreakdownAccumulator>,
    instances: HashMap<String, BreakdownAccumulator>,
    projects: HashMap<String, BreakdownAccumulator>,
    days: HashMap<String, BreakdownAccumulator>,
    hours: HashMap<i64, BreakdownAccumulator>,
    include_projects: bool,
    instance_names: &HashMap<String, String>,
) -> ReportAggregates {
    let mut by_model: Vec<CodexSessionUsageBreakdownRow> = models
        .into_iter()
        .map(|(key, acc)| {
            let label = key.clone();
            let mut row = acc.into_row(key, label, false);
            row.estimated_cost_usd = known_usage_cost(&row.key, &row);
            row
        })
        .collect();
    by_model.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.key.cmp(&right.key))
    });

    let mut by_instance: Vec<CodexSessionUsageBreakdownRow> = instances
        .into_iter()
        .map(|(key, acc)| {
            let label = instance_names
                .get(&key)
                .cloned()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| acc.label.clone());
            acc.into_row(key, label, include_projects)
        })
        .collect();
    by_instance.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.key.cmp(&right.key))
    });

    let by_project: Vec<CodexSessionUsageBreakdownRow> = if include_projects {
        let mut rows: Vec<_> = projects
            .into_iter()
            .map(|(key, acc)| {
                let label = acc.label.clone();
                acc.into_row(key, label, true)
            })
            .collect();
        rows.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.key.cmp(&right.key))
        });
        rows
    } else {
        Vec::new()
    };

    let mut by_day: Vec<CodexSessionUsageBreakdownRow> = days
        .into_iter()
        .map(|(key, acc)| {
            let label = key.clone();
            acc.into_row(key, label, true)
        })
        .collect();
    by_day.sort_by(|left, right| right.key.cmp(&left.key));

    let mut by_hour: Vec<CodexSessionUsageBreakdownRow> = hours
        .into_iter()
        .map(|(bucket, acc)| {
            let key = bucket.to_string();
            let label = key.clone();
            acc.into_row(key, label, true)
        })
        .collect();
    by_hour.sort_by(|left, right| {
        left.key
            .parse::<i64>()
            .unwrap_or(0)
            .cmp(&right.key.parse::<i64>().unwrap_or(0))
    });

    totals.estimated_cost_usd = by_model.iter().filter_map(|row| row.estimated_cost_usd).sum();

    ReportAggregates {
        totals,
        by_model,
        by_instance,
        by_project,
        by_day,
        by_hour,
    }
}

fn query_report_aggregates(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
    include_projects: bool,
    include_hour: bool,
    instance_names: &HashMap<String, String>,
) -> Result<ReportAggregates, String> {
    let (project_select, project_group) = if include_projects {
        (
            "COALESCE(project_key,''), COALESCE(MAX(project_path),'')",
            ", COALESCE(project_key,'')",
        )
    } else {
        ("'', ''", "")
    };
    let (hour_select, hour_group) = if include_hour {
        (
            "CASE WHEN timestamp<=0 THEN 0 ELSE (timestamp / 3600) * 3600 END",
            ", hour_bucket",
        )
    } else {
        ("0", "")
    };
    let sql = format!(
        "SELECT
            CASE WHEN timestamp<=0 THEN '' ELSE COALESCE(date(timestamp,'unixepoch','localtime'),'') END AS day,
            {hour_select} AS hour_bucket,
            instance_id,
            COALESCE(MAX(instance_name), ''),
            model,
            {project_select},
            COALESCE(SUM(input_tokens), 0),
            COALESCE(SUM(cached_input_tokens), 0),
            COALESCE(SUM(output_tokens), 0),
            COALESCE(SUM(request_count), 0)
         FROM session_usage_report {where_sql}
         GROUP BY day{hour_group}, instance_id, model{project_group}"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("汇总会话用量失败: {error}"))?;
    let mut rows = statement
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|error| format!("查询会话用量分组失败: {error}"))?;

    let mut totals = CodexSessionUsageTotals::default();
    let mut models: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut instances: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut projects: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut days: HashMap<String, BreakdownAccumulator> = HashMap::new();
    let mut hours: HashMap<i64, BreakdownAccumulator> = HashMap::new();

    while let Some(row) = rows
        .next()
        .map_err(|error| format!("遍历会话用量分组失败: {error}"))?
    {
        let (day, hour_bucket, instance_id, instance_name, model, project_key, project_path, input, cached, output, requests) =
            (|| -> rusqlite::Result<_> {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?.max(0) as u64,
                    row.get::<_, i64>(8)?.max(0) as u64,
                    row.get::<_, i64>(9)?.max(0) as u64,
                    row.get::<_, i64>(10)?.max(0) as u64,
                ))
            })()
            .map_err(|error| format!("解析会话用量分组失败: {error}"))?;

        totals.input_tokens = totals.input_tokens.saturating_add(input);
        totals.cached_input_tokens = totals.cached_input_tokens.saturating_add(cached);
        totals.output_tokens = totals.output_tokens.saturating_add(output);
        totals.total_tokens = totals
            .total_tokens
            .saturating_add(input.saturating_add(output));
        totals.request_count = totals.request_count.saturating_add(requests);

        models
            .entry(model.clone())
            .or_default()
            .add(&model, None, input, cached, output, requests);
        instances.entry(instance_id).or_default().add(
            &instance_name,
            include_projects.then_some(model.as_str()),
            input,
            cached,
            output,
            requests,
        );
        if include_projects {
            projects.entry(project_key).or_default().add(
                &project_path,
                Some(model.as_str()),
                input,
                cached,
                output,
                requests,
            );
        }
        days.entry(day)
            .or_default()
            .add("", Some(model.as_str()), input, cached, output, requests);
        if include_hour && hour_bucket > 0 {
            hours
                .entry(hour_bucket)
                .or_default()
                .add("", Some(model.as_str()), input, cached, output, requests);
        }
    }

    Ok(finalize_report_aggregates(
        totals,
        models,
        instances,
        projects,
        days,
        hours,
        include_projects,
        instance_names,
    ))
}

#[cfg(test)]
fn query_day_breakdown(
    conn: &Connection,
    where_sql: &str,
    params: &[rusqlite::types::Value],
) -> Result<Vec<CodexSessionUsageBreakdownRow>, String> {
    let sql = format!(
        "SELECT CASE WHEN timestamp<=0 THEN '' ELSE COALESCE(date(timestamp,'unixepoch','localtime'),'') END AS day,
                model,SUM(input_tokens),SUM(cached_input_tokens),SUM(output_tokens),SUM(request_count)
         FROM session_usage_report {where_sql} GROUP BY day,model"
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|error| format!("查询会话用量日期失败: {error}"))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?.max(0) as u64,
                row.get::<_, i64>(4)?.max(0) as u64,
                row.get::<_, i64>(5)?.max(0) as u64,
            ))
        })
        .map_err(|error| format!("遍历会话用量日期失败: {error}"))?;
    let mut days: HashMap<String, HashMap<String, CodexSessionUsageTotals>> = HashMap::new();
    for row in rows {
        let (day, model, input, cached, output, requests) =
            row.map_err(|error| format!("解析会话用量日期失败: {error}"))?;
        let totals = days
            .entry(day)
            .or_default()
            .entry(model)
            .or_default();
        totals.input_tokens = totals.input_tokens.saturating_add(input);
        totals.cached_input_tokens = totals.cached_input_tokens.saturating_add(cached);
        totals.output_tokens = totals.output_tokens.saturating_add(output);
        totals.total_tokens = totals
            .total_tokens
            .saturating_add(input.saturating_add(output));
        totals.request_count = totals.request_count.saturating_add(requests);
    }
    Ok(days
        .into_iter()
        .map(|(key, models)| {
            let mut row = CodexSessionUsageBreakdownRow {
                label: key.clone(),
                key,
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                request_count: 0,
                estimated_cost_usd: Some(0.0),
            };
            for (model, totals) in models {
                row.input_tokens = row.input_tokens.saturating_add(totals.input_tokens);
                row.cached_input_tokens = row
                    .cached_input_tokens
                    .saturating_add(totals.cached_input_tokens);
                row.output_tokens = row.output_tokens.saturating_add(totals.output_tokens);
                row.total_tokens = row.total_tokens.saturating_add(totals.total_tokens);
                row.request_count = row.request_count.saturating_add(totals.request_count);
                row.estimated_cost_usd = row.estimated_cost_usd.and_then(|cost| {
                    crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
                        &model,
                        totals.input_tokens,
                        totals.cached_input_tokens,
                        totals.output_tokens,
                    )
                    .map(|model_cost| cost + model_cost)
                });
            }
            row
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
    include!("codex_session_usage_checkpoint_tests.rs");
    include!("codex_session_usage_details_tests.rs");

    fn daily_cost_test_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE session_usage_events (
            timestamp INTEGER, model TEXT, input_tokens INTEGER, cached_input_tokens INTEGER,
            output_tokens INTEGER, instance_id TEXT
        ); CREATE TEMP VIEW session_usage_report AS SELECT *,1 AS request_count FROM session_usage_events;").unwrap();
        connection
    }

    #[test]
    fn daily_cost_sums_models_and_preserves_time_and_instance_filters() {
        let conn = daily_cost_test_connection();
        let noon = Local
            .with_ymd_and_hms(2026, 10, 3, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let next_day = Local
            .with_ymd_and_hms(2026, 10, 4, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        for (timestamp, model, instance) in [
            (noon, "gpt-5.4", "a"),
            (noon + 1, "gpt-5.4-mini", "a"),
            (next_day, "gpt-5.4", "a"),
            (noon, "gpt-5.4", "b"),
        ] {
            conn.execute(
                "INSERT INTO session_usage_events VALUES (?1, ?2, 1000000, 200000, 100000, ?3)",
                rusqlite::params![timestamp, model, instance],
            )
            .unwrap();
        }
        let params = vec![
            rusqlite::types::Value::Integer(noon),
            rusqlite::types::Value::Integer(next_day),
            rusqlite::types::Value::Text("a".into()),
        ];
        let rows = query_day_breakdown(
            &conn,
            "WHERE timestamp >= ?1 AND timestamp < ?2 AND instance_id = ?3",
            &params,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.key, "2026-10-03");
        assert_eq!(row.request_count, 2);
        assert_eq!(row.total_tokens, 2200000);
        let expected = ["gpt-5.4", "gpt-5.4-mini"]
            .into_iter()
            .map(|model| {
                crate::modules::codex_local_access::estimate_known_model_token_cost_usd(
                    model, 1000000, 200000, 100000,
                )
                .unwrap()
            })
            .sum::<f64>();
        assert!((row.estimated_cost_usd.unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn daily_cost_distinguishes_unpriced_models_from_real_zero() {
        let conn = daily_cost_test_connection();
        let noon = Local
            .with_ymd_and_hms(2026, 10, 3, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        conn.execute(
            "INSERT INTO session_usage_events VALUES (?1, 'gpt-5.4', 0, 0, 0, 'a')",
            [noon],
        )
        .unwrap();
        assert_eq!(
            query_day_breakdown(&conn, "", &[]).unwrap()[0].estimated_cost_usd,
            Some(0.0)
        );
        conn.execute("INSERT INTO session_usage_events VALUES (?1, 'unpriced-future-model', 500, 0, 20, 'a')", [noon]).unwrap();
        let row = &query_day_breakdown(&conn, "", &[]).unwrap()[0];
        assert_eq!(row.estimated_cost_usd, None);
        assert_eq!(row.request_count, 2);
        assert_eq!(row.total_tokens, 520);
    }

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
    fn paginated_fork_counts_own_usage_without_local_parent() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let sessions = home.join("sessions");
        let child = rollout_path(&sessions, CHILD_ID);
        let parent = rollout_path(&sessions, PARENT_ID);
        let mut meta = session_meta_at(CHILD_ID, Some(PARENT_ID), "2026-07-10T03:10:00Z");
        meta["ordinal"] = json!(25);
        meta["payload"]["history_mode"] = json!("paginated");
        meta["payload"]["history_base"] = json!({
            "thread_id": PARENT_ID, "end_ordinal_exclusive": 25, "end_byte_offset": 41904,
        });
        write_jsonl(&child, &[meta]);
        let store = SessionUsageStore::open_path(temp.path().join("usage.sqlite"));
        let instances = [UsageInstance {
            id: DEFAULT_INSTANCE_ID.into(), name: DEFAULT_INSTANCE_NAME.into(), data_dir: home,
        }];
        assert_eq!(store.sync(false, &instances).unwrap().imported, 0);
        store.open_conn().unwrap().execute(
            "UPDATE session_parser_checkpoints SET state=json_set(state,'$.version',1,'$.parent','Parent')",
            [],
        ).unwrap();
        use std::io::Write;
        let mut stream = fs::OpenOptions::new().append(true).open(&child).unwrap();
        writeln!(stream, "{}", turn_context("gpt-5.6-sol")).unwrap();
        writeln!(stream, "{}", token_count_with_last(
            30, 6, 12, 11, 3, 5, "codex", "2026-07-10T03:10:05Z",
        )).unwrap();
        drop(stream);
        let result = store.sync(false, &instances).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.deferred_files, 0);
        let query = CodexSessionUsageQuery::default();
        let report = store.query(&query).unwrap();
        assert_eq!(report.totals.input_tokens, 11);
        assert_eq!(report.totals.output_tokens, 5);
        assert_eq!(report.totals.total_tokens, 16);
        assert_eq!(report.totals.request_count, 1);

        write_jsonl(&parent, &[
            session_meta(PARENT_ID),
            turn_context("gpt-5.6-sol"),
            token_count_with_last(9, 1, 3, 9, 1, 3, "codex", "2026-07-10T03:00:02Z"),
            token_count_with_last(19, 3, 7, 10, 2, 4, "codex", "2026-07-10T03:00:03Z"),
        ]);
        store.sync(false, &instances).unwrap();
        store.sync(false, &instances).unwrap();
        let report = store.query(&query).unwrap();
        assert_eq!(report.totals.input_tokens, 30);
        assert_eq!(report.totals.cached_input_tokens, 6);
        assert_eq!(report.totals.output_tokens, 12);
        assert_eq!(report.totals.total_tokens, 42);
        assert_eq!(report.totals.request_count, 3);
    }

    #[test]
    fn paginated_fork_treats_an_inherited_cumulative_snapshot_as_baseline() {
        let temp = tempfile::tempdir().unwrap();
        let child = rollout_path(temp.path(), CHILD_ID);
        let mut meta = session_meta_at(CHILD_ID, Some(PARENT_ID), "2026-07-10T03:10:00Z");
        meta["ordinal"] = json!(25);
        meta["payload"]["history_mode"] = json!("paginated");
        meta["payload"]["history_base"] = json!({
            "thread_id": PARENT_ID, "end_ordinal_exclusive": 25, "end_byte_offset": 41904,
        });
        write_jsonl(&child, &[
            meta,
            token_count_total(1_400, 100, 35, "2026-07-10T03:10:05Z"),
            token_count_total(1_460, 120, 46, "2026-07-10T03:11:05Z"),
        ]);
        let parsed = parse_codex_file(&child, Some(CHILD_ID.to_string())).unwrap();
        assert_eq!(nonzero_deltas(&parsed), vec![(60, 20, 11)]);
    }

    #[test]
    fn lookup_cached_rollout_token_stats_uses_checkpoint_high_water_and_validates_freshness() {
        use std::io::Write;
        let dir = make_temp_dir("codex-usage-cached-rollout-stats");
        let home = dir.join("home");
        let plain = rollout_path(&home.join("sessions"), PARENT_ID);
        let compressed = rollout_path(&home.join("sessions"), CHILD_ID).with_extension("jsonl.zst");
        write_jsonl(
            &plain,
            &[
                session_meta(PARENT_ID),
                turn_context("gpt-5.4"),
                token_count_total(1_000, 200, 150, "2026-07-10T03:00:02Z"),
                token_count_total(1_600, 350, 240, "2026-07-10T03:00:03Z"),
            ],
        );
        write_compressed_jsonl(
            &compressed,
            &[
                session_meta(CHILD_ID),
                turn_context("gpt-5.4"),
                token_count_total(500, 100, 80, "2026-07-10T03:00:04Z"),
            ],
        );
        let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
        let instances = [UsageInstance {
            id: DEFAULT_INSTANCE_ID.into(),
            name: DEFAULT_INSTANCE_NAME.into(),
            data_dir: home,
        }];
        let sync = store.sync(false, &instances).unwrap();
        assert!(sync.errors.is_empty());
        assert_eq!(sync.imported, 3);

        let conn = store.open_conn().unwrap();
        let mut cached = HashMap::new();
        lookup_cached_rollout_token_stats_from_conn(
            &conn,
            &[plain.clone(), compressed.clone()],
            &mut cached,
        );
        assert_eq!(cached.get(&plain), Some(&(1_600, 240, 1_840)));
        assert_eq!(cached.get(&compressed), Some(&(500, 80, 580)));

        // Appending invalidates the cached entry immediately.
        let mut append = std::fs::OpenOptions::new().append(true).open(&plain).unwrap();
        writeln!(
            append,
            "{}",
            token_count_total(2_000, 400, 300, "2026-07-10T03:00:05Z")
        )
        .unwrap();
        let mut refreshed = HashMap::new();
        lookup_cached_rollout_token_stats_from_conn(
            &conn,
            &[plain.clone(), compressed.clone()],
            &mut refreshed,
        );
        assert!(!refreshed.contains_key(&plain));
        assert_eq!(refreshed.get(&compressed), Some(&(500, 80, 580)));
        drop(conn);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn single_pass_query_aggregates_match_breakdowns_and_costs() {
        let dir = make_temp_dir("codex-usage-single-pass");
        let home = dir.join("home");
        let file = rollout_path(&home.join("sessions"), PARENT_ID);
        let first_ts = Local
            .with_ymd_and_hms(2026, 7, 10, 12, 5, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let second_ts = Local
            .with_ymd_and_hms(2026, 7, 10, 13, 5, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let meta_ts = Local
            .with_ymd_and_hms(2026, 7, 10, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        write_jsonl(
            &file,
            &[
                json!({
                    "timestamp": meta_ts.to_rfc3339(),
                    "type": "session_meta",
                    "payload": { "id": PARENT_ID, "cwd": "/work/demo", "title": "Demo" }
                }),
                turn_context("gpt-5.4"),
                token_count_with_last(
                    1_000_000,
                    200_000,
                    100_000,
                    1_000_000,
                    200_000,
                    100_000,
                    "codex",
                    &first_ts.to_rfc3339(),
                ),
                turn_context("gpt-5.4-mini"),
                token_count_with_last(
                    2_000_000,
                    400_000,
                    200_000,
                    1_000_000,
                    200_000,
                    100_000,
                    "codex",
                    &second_ts.to_rfc3339(),
                ),
            ],
        );
        let store = SessionUsageStore::open_path(dir.join("usage.sqlite"));
        let instances = [UsageInstance {
            id: DEFAULT_INSTANCE_ID.into(),
            name: DEFAULT_INSTANCE_NAME.into(),
            data_dir: home,
        }];
        let sync = store.sync(false, &instances).unwrap();
        assert!(sync.errors.is_empty());
        assert_eq!(sync.imported, 2);

        let from = Local
            .with_ymd_and_hms(2026, 7, 10, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let to = Local
            .with_ymd_and_hms(2026, 7, 10, 23, 59, 59)
            .single()
            .unwrap()
            .timestamp();
        let report = store
            .query(&CodexSessionUsageQuery {
                from_timestamp: Some(from),
                to_timestamp: Some(to),
                include_projects: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(report.totals.request_count, 2);
        assert_eq!(report.totals.input_tokens, 2_000_000);
        assert_eq!(report.totals.cached_input_tokens, 400_000);
        assert_eq!(report.totals.output_tokens, 200_000);
        assert_eq!(report.totals.total_tokens, 2_200_000);
        assert_eq!(report.by_model.len(), 2);
        assert_eq!(report.by_instance.len(), 1);
        assert_eq!(report.by_project.len(), 1);
        assert_eq!(report.by_project[0].key, "/work/demo");
        assert_eq!(report.by_day.len(), 1);
        assert_eq!(report.by_hour.len(), 2);
        assert!(report.by_project[0].estimated_cost_usd.is_some());
        assert!((report.by_project[0].estimated_cost_usd.unwrap() - report.totals.estimated_cost_usd).abs() < 1e-10);
        assert!((report.by_day[0].estimated_cost_usd.unwrap() - report.totals.estimated_cost_usd).abs() < 1e-10);

        // Second sync on unchanged file must take fast skip path with 0 imports.
        let second = store.sync(false, &instances).unwrap();
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped, 0);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn continuation_rollout_is_counted_as_parent_fork_not_deferred() {
        let dir = make_temp_dir("codex-usage-continuation");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let sessions = home.join("sessions").join("2026").join("07").join("10");
        let parent = sessions.join(format!("rollout-2026-07-10T03-00-00-{PARENT_ID}.jsonl"));
        // Continuation/branch form: the filename carries root UUID + `_` + leaf UUID while
        // session_meta.id is the root. This used to be deferred and dropped entirely (#2897).
        let child = sessions.join(format!(
            "rollout-2026-07-10T03-10-00-{PARENT_ID}_{CHILD_ID}.jsonl"
        ));
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
                session_meta_at(PARENT_ID, None, "2026-07-10T03:10:00Z"),
                turn_context("gpt-5.4"),
                // replayed parent context: identical snapshot to the parent's only event
                token_count_with_last(1_000, 0, 20, 1_000, 0, 20, "codex", "2026-07-10T03:00:02Z"),
                // new usage after the branch point
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
        // The parent turn and the continuation's new turn are imported; the replayed parent
        // context inside the continuation is skipped, so nothing is double counted.
        assert_eq!(sync.imported, 2);
        let report = store.query(&CodexSessionUsageQuery::default()).unwrap();
        assert_eq!(report.totals.input_tokens, 1_400);
        assert_eq!(report.totals.output_tokens, 35);
        assert_eq!(report.totals.request_count, 2);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn continuation_rollout_counts_fully_when_root_missing() {
        let dir = make_temp_dir("codex-usage-continuation-orphan");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let sessions = home.join("sessions").join("2026").join("07").join("10");
        // Only the continuation exists; the root rollout is absent, so there is nothing to
        // dedup against and the turn must still be counted (not dropped).
        let child = sessions.join(format!(
            "rollout-2026-07-10T03-10-00-{PARENT_ID}_{CHILD_ID}.jsonl"
        ));
        write_jsonl(
            &child,
            &[
                session_meta_at(PARENT_ID, None, "2026-07-10T03:10:00Z"),
                turn_context("gpt-5.4"),
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
        assert_eq!(sync.imported, 1);
        let report = store.query(&CodexSessionUsageQuery::default()).unwrap();
        assert_eq!(report.totals.input_tokens, 400);
        assert_eq!(report.totals.request_count, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn genuine_filename_meta_mismatch_is_still_deferred() {
        let dir = make_temp_dir("codex-usage-mismatch");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let sessions = home.join("sessions").join("2026").join("07").join("10");
        // Single-UUID filename whose id disagrees with session_meta.id and is not a
        // root_leaf continuation: still treated as corrupt and dropped.
        let file = sessions.join(format!("rollout-2026-07-10T03-00-00-{PARENT_ID}.jsonl"));
        write_jsonl(
            &file,
            &[
                session_meta(CHILD_ID),
                turn_context("gpt-5.4"),
                token_count_with_last(1_400, 0, 35, 400, 0, 15, "codex", "2026-07-10T03:00:02Z"),
            ],
        );

        let store = SessionUsageStore::open_path(db_path);
        let instances = vec![UsageInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: home,
        }];
        let sync = store.sync(false, &instances).unwrap();
        assert_eq!(sync.imported, 0);
        let report = store.query(&CodexSessionUsageQuery::default()).unwrap();
        assert_eq!(report.totals.request_count, 0);
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

    #[test]
    fn parallel_lpt_sync_and_snapshot_query_match_sql_across_filters() {
        let dir = make_temp_dir("codex-usage-parallel-snapshot");
        let db_path = dir.join("usage.sqlite");
        let home = dir.join("home");
        let sessions = home.join("sessions").join("2026").join("07").join("10");
        let third_id = "019c0000-0000-7000-8000-000000000003";

        let t1 = Local
            .with_ymd_and_hms(2026, 7, 10, 10, 15, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let t2 = Local
            .with_ymd_and_hms(2026, 7, 10, 11, 20, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let t3 = Local
            .with_ymd_and_hms(2026, 7, 10, 12, 25, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);

        let parent_file = sessions.join(format!("rollout-2026-07-10T10-00-00-{PARENT_ID}.jsonl"));
        let child_file = sessions.join(format!("rollout-2026-07-10T11-00-00-{CHILD_ID}.jsonl"));
        let compressed_file =
            sessions.join(format!("rollout-2026-07-10T12-00-00-{third_id}.jsonl.zst"));

        write_jsonl(
            &parent_file,
            &[
                json!({
                    "timestamp": t1.to_rfc3339(),
                    "type": "session_meta",
                    "payload": { "id": PARENT_ID, "cwd": "/work/alpha", "title": "Alpha" }
                }),
                turn_context("gpt-5.4"),
                token_count_with_last(
                    1_000_000,
                    200_000,
                    100_000,
                    1_000_000,
                    200_000,
                    100_000,
                    "codex",
                    &t1.to_rfc3339(),
                ),
            ],
        );
        write_jsonl(
            &child_file,
            &[
                json!({
                    "timestamp": t2.to_rfc3339(),
                    "type": "session_meta",
                    "payload": {
                        "id": CHILD_ID,
                        "cwd": "/work/alpha",
                        "title": "Alpha Fork",
                        "forked_from_id": PARENT_ID
                    }
                }),
                turn_context("gpt-5.4"),
                // Replayed parent event: must be deduplicated even under parallel parsing.
                token_count_with_last(
                    1_000_000,
                    200_000,
                    100_000,
                    1_000_000,
                    200_000,
                    100_000,
                    "codex",
                    &t1.to_rfc3339(),
                ),
                token_count_with_last(
                    1_500_000,
                    250_000,
                    140_000,
                    500_000,
                    50_000,
                    40_000,
                    "codex",
                    &t2.to_rfc3339(),
                ),
            ],
        );
        let compressed_plain = dir.join("third.jsonl");
        write_jsonl(
            &compressed_plain,
            &[
                json!({
                    "timestamp": t3.to_rfc3339(),
                    "type": "session_meta",
                    "payload": { "id": third_id, "cwd": "/work/beta", "title": "Beta" }
                }),
                turn_context("gpt-5.4-mini"),
                token_count_with_last(
                    300_000,
                    100_000,
                    60_000,
                    300_000,
                    100_000,
                    60_000,
                    "codex",
                    &t3.to_rfc3339(),
                ),
            ],
        );
        fs::create_dir_all(compressed_file.parent().unwrap()).unwrap();
        let compressed_bytes =
            zstd::stream::encode_all(fs::read(&compressed_plain).unwrap().as_slice(), 1).unwrap();
        fs::write(&compressed_file, compressed_bytes).unwrap();

        let store = SessionUsageStore::open_path(db_path.clone());
        let instances = vec![UsageInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: home,
        }];
        let sync = store.sync(false, &instances).unwrap();
        assert!(sync.errors.is_empty());
        assert_eq!(sync.files_scanned, 3);
        assert_eq!(sync.imported, 3);

        // Complete rollup backfill so snapshot can be loaded cleanly.
        let conn = store.open_conn().unwrap();
        while !SESSION_USAGE_ROLLUP.batch(&conn).unwrap() {}
        let snapshot = load_usage_bucket_snapshot(&conn, &db_path)
            .unwrap()
            .expect("rollup ready snapshot");
        assert_eq!(snapshot.db_path, db_path);
        assert!(snapshot.revision > 0);
        assert_eq!(snapshot.event_count, 3);

        let from = Local
            .with_ymd_and_hms(2026, 7, 10, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let to = Local
            .with_ymd_and_hms(2026, 7, 10, 23, 59, 59)
            .single()
            .unwrap()
            .timestamp();

        let queries = [
            CodexSessionUsageQuery {
                from_timestamp: Some(from),
                to_timestamp: Some(to),
                include_projects: true,
                ..Default::default()
            },
            CodexSessionUsageQuery {
                from_timestamp: Some(from),
                to_timestamp: Some(to),
                model: Some("gpt-5.4".to_string()),
                include_projects: true,
                ..Default::default()
            },
            CodexSessionUsageQuery {
                from_timestamp: Some(from),
                to_timestamp: Some(to),
                project_key: Some("/work/beta".to_string()),
                include_projects: true,
                ..Default::default()
            },
        ];

        for query in &queries {
            assert!(snapshot_covers_query(&snapshot, query));
            let sql_report = store.query(query).unwrap();
            let snap_report = query_report_from_snapshot(&snapshot, query, &instances);
            assert_eq!(snap_report.totals.input_tokens, sql_report.totals.input_tokens);
            assert_eq!(
                snap_report.totals.cached_input_tokens,
                sql_report.totals.cached_input_tokens
            );
            assert_eq!(snap_report.totals.output_tokens, sql_report.totals.output_tokens);
            assert_eq!(snap_report.totals.total_tokens, sql_report.totals.total_tokens);
            assert_eq!(snap_report.totals.request_count, sql_report.totals.request_count);
            assert!(
                (snap_report.totals.estimated_cost_usd - sql_report.totals.estimated_cost_usd)
                    .abs()
                    < 1e-10
            );
            assert_eq!(snap_report.by_model.len(), sql_report.by_model.len());
            assert_eq!(snap_report.by_project.len(), sql_report.by_project.len());
            assert_eq!(snap_report.by_day.len(), sql_report.by_day.len());
            assert_eq!(snap_report.by_hour.len(), sql_report.by_hour.len());
            assert_eq!(snap_report.model_options.len(), sql_report.model_options.len());
            assert_eq!(snap_report.project_options.len(), sql_report.project_options.len());
        }

        // Mid-minute boundary before max_updated_at must decline snapshot and use SQL exact path.
        assert!(!snapshot_covers_query(
            &snapshot,
            &CodexSessionUsageQuery {
                from_timestamp: Some(from + 17),
                to_timestamp: Some(t1.timestamp() + 13),
                ..Default::default()
            }
        ));

        fs::remove_dir_all(&dir).ok();
    }
}
