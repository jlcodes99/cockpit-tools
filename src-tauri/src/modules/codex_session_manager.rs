use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_TRASH_ROOT_DIR: &str = "cockpit-tools-codex-session-trash";
const SESSION_EXPORT_KIND: &str = "codex-session-export";
const SESSION_EXPORT_VERSION: u32 = 1;
pub const SESSION_TRANSFER_PROGRESS_EVENT: &str = "codex:session-transfer-progress";
const SESSION_INDEX_ACTIVITY_DRIFT_SECONDS: i64 = 3_600;
const TOKEN_STATS_READ_CHUNK_BYTES: usize = 64 * 1024;
const ROLLOUT_ACTIVITY_READ_CHUNK_BYTES: usize = 64 * 1024;
const ROLLOUT_ACTIVITY_MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;
const CONTENT_SEARCH_READ_CHUNK_BYTES: usize = 64 * 1024;
const CONTENT_SEARCH_CACHE_MAX_ENTRIES: usize = 512;

static TOKEN_STATS_CACHE: LazyLock<Mutex<HashMap<PathBuf, TokenStatsCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CONTENT_SEARCH_CACHE: LazyLock<Mutex<HashMap<ContentSearchCacheKey, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionLocation {
    pub instance_id: String,
    pub instance_name: String,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRecord {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
    pub location_count: usize,
    pub locations: Vec<CodexSessionLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionTokenStats {
    pub session_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionTrashSummary {
    pub requested_session_count: usize,
    pub trashed_session_count: usize,
    pub trashed_instance_count: usize,
    pub trash_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTrashedSessionLocation {
    pub instance_id: String,
    pub instance_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTrashedSessionRecord {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub deleted_at: Option<i64>,
    pub size_bytes: u64,
    pub location_count: usize,
    pub locations: Vec<CodexTrashedSessionLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRestoreSummary {
    pub requested_session_count: usize,
    pub restored_session_count: usize,
    pub restored_instance_count: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionTrashDeleteSummary {
    pub requested_session_count: usize,
    pub deleted_session_count: usize,
    pub deleted_entry_count: usize,
    pub freed_size_bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionExportSummary {
    pub requested_session_count: usize,
    pub exported_session_count: usize,
    pub skipped_session_count: usize,
    pub export_path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionExportPreview {
    pub requested_session_count: usize,
    pub exportable_session_count: usize,
    pub missing_session_count: usize,
    pub total_size_bytes: u64,
    pub items: Vec<CodexSessionExportPreviewItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionExportPreviewItem {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
    pub size_bytes: u64,
    pub source_instance_id: String,
    pub source_instance_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionImportPreview {
    pub package_version: u32,
    pub exported_at: Option<String>,
    pub import_file_path: String,
    pub target_instance_id: String,
    pub target_instance_name: String,
    pub total_session_count: usize,
    pub importable_session_count: usize,
    pub items: Vec<CodexSessionImportPreviewItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionImportPreviewItem {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
    pub size_bytes: u64,
    pub status: String,
    pub reason: Option<String>,
    pub existing_instance_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionImportSummary {
    pub requested_session_count: usize,
    pub imported_session_count: usize,
    pub skipped_session_count: usize,
    pub target_instance_id: String,
    pub target_instance_name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionTransferProgress {
    pub transfer_id: String,
    pub operation: String,
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub percent: u8,
    pub current_label: Option<String>,
    pub running: bool,
}

type SessionTransferProgressReporter<'a> = &'a (dyn Fn(CodexSessionTransferProgress) + Send + Sync);

#[derive(Debug, Clone, Default)]
pub struct CodexSessionSearchFilter {
    pub title_query: Option<String>,
    pub content_query: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexSyncInstance {
    id: String,
    name: String,
    data_dir: PathBuf,
    last_pid: Option<u32>,
}

#[derive(Debug, Clone)]
struct ThreadSnapshot {
    id: String,
    title: String,
    cwd: String,
    updated_at: Option<i64>,
    rollout_path: PathBuf,
    session_index_entry: JsonValue,
    source_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionExportManifest {
    kind: String,
    package_version: u32,
    exported_at: String,
    sessions: Vec<SessionExportManifestItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionExportManifestItem {
    session_id: String,
    title: String,
    cwd: String,
    updated_at: Option<i64>,
    relative_rollout_path: String,
    file_entry: String,
    size_bytes: u64,
    sha256: String,
    session_index_entry: JsonValue,
    source_instance: SessionExportInstance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionExportInstance {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrashedSessionManifest {
    session_id: String,
    title: String,
    cwd: String,
    instance_id: String,
    instance_name: String,
    instance_root: PathBuf,
    original_rollout_path: PathBuf,
    relative_rollout_path: String,
    session_index_entry: JsonValue,
    deleted_at: Option<String>,
}

#[derive(Debug, Clone)]
struct TrashedSessionEntry {
    entry_dir: PathBuf,
    manifest: TrashedSessionManifest,
    trashed_rollout_path: PathBuf,
}

#[derive(Debug, Clone)]
struct TrashRoot {
    path: PathBuf,
    optional: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct RestoreTrashedSessionOutcome {
    metadata_rebuild_failed: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct TrashSnapshotsOutcome {
    metadata_rebuild_failed: bool,
}

#[derive(Debug, Clone)]
struct TokenStatsCacheEntry {
    file_len: u64,
    modified_at: Option<SystemTime>,
    stats: Option<(u64, u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ContentSearchCacheKey {
    rollout_path: PathBuf,
    query: String,
    file_len: u64,
    modified_at_nanos: Option<u128>,
}

/// 从 rollout JSONL 文件中读取 token 统计信息
/// 返回 (input_tokens, output_tokens, total_tokens)
fn read_token_stats_from_rollout(rollout_path: &Path) -> Option<(u64, u64, u64)> {
    let metadata = fs::metadata(rollout_path).ok()?;
    let cache_key = rollout_path.to_path_buf();
    let file_len = metadata.len();
    let modified_at = metadata.modified().ok();

    {
        let cache = TOKEN_STATS_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get(&cache_key) {
            if entry.file_len == file_len && entry.modified_at == modified_at {
                return entry.stats;
            }
        }
    }

    let stats = read_token_stats_from_rollout_uncached(rollout_path, file_len);
    let mut cache = TOKEN_STATS_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        cache_key,
        TokenStatsCacheEntry {
            file_len,
            modified_at,
            stats,
        },
    );
    stats
}

fn read_token_stats_from_rollout_uncached(
    rollout_path: &Path,
    file_len: u64,
) -> Option<(u64, u64, u64)> {
    let mut file = File::open(rollout_path).ok()?;
    let mut offset = file_len;
    let mut pending_prefix = Vec::new();

    while offset > 0 {
        let chunk_len = TOKEN_STATS_READ_CHUNK_BYTES.min(offset as usize);
        offset -= chunk_len as u64;

        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = vec![0u8; chunk_len];
        file.read_exact(&mut chunk).ok()?;

        let starts_on_line_boundary =
            offset == 0 || byte_before_is_newline(&mut file, offset).ok()?;
        chunk.extend_from_slice(&pending_prefix);

        let parse_from_index = if starts_on_line_boundary {
            pending_prefix.clear();
            0
        } else if let Some(newline_index) = chunk.iter().position(|byte| *byte == b'\n') {
            pending_prefix = chunk[..newline_index].to_vec();
            newline_index + 1
        } else {
            pending_prefix = chunk;
            continue;
        };

        if let Some(stats) = parse_token_stats_lines(&chunk[parse_from_index..]) {
            return Some(stats);
        }
    }

    if pending_prefix.is_empty() {
        None
    } else {
        parse_token_stats_lines(&pending_prefix)
    }
}

fn byte_before_is_newline(file: &mut File, offset: u64) -> std::io::Result<bool> {
    if offset == 0 {
        return Ok(true);
    }

    file.seek(SeekFrom::Start(offset - 1))?;
    let mut byte = [0u8; 1];
    file.read_exact(&mut byte)?;
    Ok(byte[0] == b'\n')
}

fn parse_token_stats_lines(content: &[u8]) -> Option<(u64, u64, u64)> {
    for line in content.split(|byte| *byte == b'\n').rev() {
        let raw = String::from_utf8_lossy(line);
        let trimmed = raw.trim();
        if trimmed.is_empty()
            || !trimmed.contains("\"token_count\"")
            || !trimmed.contains("\"total_token_usage\"")
        {
            continue;
        }

        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        if parsed.get("type").and_then(|value| value.as_str()) != Some("event_msg") {
            continue;
        }
        let Some(payload) = parsed.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(|value| value.as_str()) != Some("token_count") {
            continue;
        }
        let Some(usage) = payload
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
        else {
            continue;
        };

        let input = usage
            .get("input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let output = usage
            .get("output_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let total = usage
            .get("total_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        return Some((input, output, total));
    }

    None
}

pub fn list_sessions_across_instances(
    title_query: Option<String>,
    content_query: Option<String>,
) -> Result<Vec<CodexSessionRecord>, String> {
    let filter = CodexSessionSearchFilter {
        title_query: normalize_search_query(title_query),
        content_query: normalize_content_search_query(content_query),
    };
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut session_map = HashMap::<String, CodexSessionRecord>::new();
    let has_search_filter = filter.title_query.is_some() || filter.content_query.is_some();
    let mut matched_session_ids = HashSet::<String>::new();

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        for snapshot in load_thread_snapshots(instance)? {
            if !has_search_filter
                || matched_session_ids.contains(&snapshot.id)
                || matches_session_search_filter(&snapshot, &filter)?
            {
                matched_session_ids.insert(snapshot.id.clone());
            }

            let entry =
                session_map
                    .entry(snapshot.id.clone())
                    .or_insert_with(|| CodexSessionRecord {
                        session_id: snapshot.id.clone(),
                        title: snapshot.title.clone(),
                        cwd: snapshot.cwd.clone(),
                        updated_at: snapshot.updated_at,
                        location_count: 0,
                        locations: Vec::new(),
                    });

            if entry.updated_at.is_none() {
                entry.updated_at = snapshot.updated_at;
            }
            if entry.title.trim().is_empty() {
                entry.title = snapshot.title.clone();
            }
            if entry.cwd.trim().is_empty() {
                entry.cwd = snapshot.cwd.clone();
            }

            entry.locations.push(CodexSessionLocation {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                running,
            });
            entry.location_count = entry.locations.len();
        }
    }

    let mut sessions = session_map
        .into_values()
        .filter(|session| !has_search_filter || matched_session_ids.contains(&session.session_id))
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .unwrap_or_default()
            .cmp(&left.updated_at.unwrap_or_default())
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(sessions)
}

fn normalize_search_query(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty())
}

fn normalize_content_search_query(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn matches_session_search_filter(
    snapshot: &ThreadSnapshot,
    filter: &CodexSessionSearchFilter,
) -> Result<bool, String> {
    if let Some(query) = filter.title_query.as_deref() {
        if !text_contains_query(&snapshot.title, query) {
            return Ok(false);
        }
    }

    if let Some(query) = filter.content_query.as_deref() {
        if !rollout_conversation_contains_query(&snapshot.rollout_path, query)? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn text_contains_query(value: &str, query: &str) -> bool {
    value.to_lowercase().contains(query)
}

fn rollout_conversation_contains_query(path: &Path, query: &str) -> Result<bool, String> {
    let cache_key = content_search_cache_key(path, query);
    if let Some(key) = cache_key.as_ref() {
        let cache = CONTENT_SEARCH_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = cache.get(key) {
            return Ok(*cached);
        }
    }

    let matched = rollout_conversation_contains_query_uncached(path, query)?;
    if let Some(key) = cache_key {
        let mut cache = CONTENT_SEARCH_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cache.len() >= CONTENT_SEARCH_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(key, matched);
    }

    Ok(matched)
}

fn content_search_cache_key(path: &Path, query: &str) -> Option<ContentSearchCacheKey> {
    let metadata = fs::metadata(path).ok()?;
    let modified_at_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos());

    Some(ContentSearchCacheKey {
        rollout_path: path.to_path_buf(),
        query: query.to_string(),
        file_len: metadata.len(),
        modified_at_nanos,
    })
}

fn rollout_conversation_contains_query_uncached(path: &Path, query: &str) -> Result<bool, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("打开 rollout 文件失败 ({}): {}", path.display(), error))?;
    let query_bytes = query.as_bytes();
    if query_bytes.is_empty() {
        return Ok(true);
    }

    let mut chunk = vec![0u8; CONTENT_SEARCH_READ_CHUNK_BYTES];
    let mut carry = Vec::<u8>::new();
    let keep_len = query_bytes.len().saturating_sub(1);

    loop {
        let bytes_read = file
            .read(&mut chunk)
            .map_err(|error| format!("读取 rollout 文件失败 ({}): {}", path.display(), error))?;
        if bytes_read == 0 {
            break;
        }

        let mut haystack = Vec::with_capacity(carry.len() + bytes_read);
        haystack.extend_from_slice(&carry);
        haystack.extend_from_slice(&chunk[..bytes_read]);
        if raw_bytes_contains_normalized_query(&haystack, query_bytes, query.is_ascii()) {
            return Ok(true);
        }

        if keep_len == 0 {
            carry.clear();
        } else {
            let next_carry_len = keep_len.min(haystack.len());
            carry.clear();
            carry.extend_from_slice(&haystack[haystack.len() - next_carry_len..]);
        }
    }

    Ok(false)
}

fn raw_bytes_contains_normalized_query(
    value: &[u8],
    query: &[u8],
    ascii_case_insensitive: bool,
) -> bool {
    if query.is_empty() {
        return true;
    }
    if ascii_case_insensitive {
        return ascii_case_insensitive_contains(value, query);
    }
    value.windows(query.len()).any(|window| window == query)
}

fn ascii_case_insensitive_contains(value: &[u8], query: &[u8]) -> bool {
    if query.is_empty() {
        return true;
    }
    if query.len() > value.len() {
        return false;
    }

    value.windows(query.len()).any(|window| {
        window
            .iter()
            .zip(query.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
    })
}

pub fn get_session_token_stats_across_instances(
    session_ids: Vec<String>,
) -> Result<Vec<CodexSessionTokenStats>, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Ok(Vec::new());
    }

    let instances = collect_instances()?;
    let mut pending_ids = requested_ids.clone();
    let mut stats_by_session_id = HashMap::<String, CodexSessionTokenStats>::new();

    for instance in &instances {
        if pending_ids.is_empty() {
            break;
        }

        for snapshot in load_thread_snapshots(instance)? {
            if !pending_ids.contains(&snapshot.id) {
                continue;
            }

            let Some((input_tokens, output_tokens, total_tokens)) =
                read_token_stats_from_rollout(&snapshot.rollout_path)
            else {
                continue;
            };

            stats_by_session_id.insert(
                snapshot.id.clone(),
                CodexSessionTokenStats {
                    session_id: snapshot.id.clone(),
                    input_tokens,
                    output_tokens,
                    total_tokens,
                },
            );
            pending_ids.remove(&snapshot.id);
        }
    }

    let mut stats = stats_by_session_id.into_values().collect::<Vec<_>>();
    stats.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    Ok(stats)
}

pub fn move_sessions_to_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<CodexSessionTrashSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let trash_root = create_trash_root_dir()?;
    let mut trashed_session_ids = HashSet::new();
    let mut trashed_instance_count = 0usize;
    let mut mutated_running_instance_count = 0usize;
    let mut metadata_rebuild_failed_count = 0usize;

    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?
            .into_iter()
            .filter(|snapshot| requested_ids.contains(&snapshot.id))
            .collect::<Vec<_>>();
        if snapshots.is_empty() {
            continue;
        }

        if is_instance_running(instance, &process_entries) {
            mutated_running_instance_count += 1;
        }

        let outcome = trash_snapshots_for_instance(instance, &trash_root, &snapshots)?;
        if outcome.metadata_rebuild_failed {
            metadata_rebuild_failed_count += 1;
        }
        trashed_instance_count += 1;
        for snapshot in snapshots {
            trashed_session_ids.insert(snapshot.id);
        }
    }

    if trashed_instance_count == 0 {
        return Ok(CodexSessionTrashSummary {
            requested_session_count: requested_ids.len(),
            trashed_session_count: 0,
            trashed_instance_count: 0,
            trash_dirs: Vec::new(),
            message: "所选会话在当前实例集合中不存在，无需处理".to_string(),
        });
    }

    let mut message = if mutated_running_instance_count > 0 {
        format!(
            "已将 {} 条会话移到废纸篓，并已触发官方 Codex 重建会话索引；运行中的实例可能需要刷新或重启后显示",
            trashed_session_ids.len()
        )
    } else {
        format!(
            "已将 {} 条会话移到废纸篓，并已触发官方 Codex 重建会话索引",
            trashed_session_ids.len()
        )
    };
    if metadata_rebuild_failed_count > 0 {
        message.push_str(&format!(
            "；{} 个实例的官方侧边栏索引重建未完成，重启 Codex 后会重新加载",
            metadata_rebuild_failed_count
        ));
    }

    Ok(CodexSessionTrashSummary {
        requested_session_count: requested_ids.len(),
        trashed_session_count: trashed_session_ids.len(),
        trashed_instance_count,
        trash_dirs: vec![trash_root.to_string_lossy().to_string()],
        message,
    })
}

pub fn list_trashed_sessions_across_instances() -> Result<Vec<CodexTrashedSessionRecord>, String> {
    let entries = load_trash_entries()?;
    let mut session_map = HashMap::<String, CodexTrashedSessionRecord>::new();

    for entry in entries {
        let deleted_at = parse_deleted_at(entry.manifest.deleted_at.as_deref());
        let record = session_map
            .entry(entry.manifest.session_id.clone())
            .or_insert_with(|| CodexTrashedSessionRecord {
                session_id: entry.manifest.session_id.clone(),
                title: entry.manifest.title.clone(),
                cwd: entry.manifest.cwd.clone(),
                deleted_at,
                size_bytes: 0,
                location_count: 0,
                locations: Vec::new(),
            });

        if deleted_at.unwrap_or_default() > record.deleted_at.unwrap_or_default() {
            record.deleted_at = deleted_at;
        }
        if record.title.trim().is_empty() {
            record.title = entry.manifest.title.clone();
        }
        if record.cwd.trim().is_empty() {
            record.cwd = entry.manifest.cwd.clone();
        }

        record.locations.push(CodexTrashedSessionLocation {
            instance_id: entry.manifest.instance_id.clone(),
            instance_name: entry.manifest.instance_name.clone(),
        });
        record.location_count = record.locations.len();
        record.size_bytes = record
            .size_bytes
            .saturating_add(calculate_path_size(&entry.entry_dir).unwrap_or(0));
    }

    let mut sessions = session_map.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .deleted_at
            .unwrap_or_default()
            .cmp(&left.deleted_at.unwrap_or_default())
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.title.cmp(&right.title))
    });
    Ok(sessions)
}

pub fn delete_trashed_sessions_across_instances(
    session_ids: Vec<String>,
) -> Result<CodexSessionTrashDeleteSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let entries = load_trash_entries()?
        .into_iter()
        .filter(|entry| requested_ids.contains(&entry.manifest.session_id))
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Ok(CodexSessionTrashDeleteSummary {
            requested_session_count: requested_ids.len(),
            deleted_session_count: 0,
            deleted_entry_count: 0,
            freed_size_bytes: 0,
            message: "所选会话在废纸篓中不存在，无需删除".to_string(),
        });
    }

    let (deleted_session_ids, deleted_entry_count, freed_size_bytes) =
        delete_trash_entries(&entries)?;
    Ok(CodexSessionTrashDeleteSummary {
        requested_session_count: requested_ids.len(),
        deleted_session_count: deleted_session_ids.len(),
        deleted_entry_count,
        freed_size_bytes,
        message: format!(
            "已永久删除 {} 条废纸篓会话，释放约 {}",
            deleted_session_ids.len(),
            format_bytes(freed_size_bytes)
        ),
    })
}

pub fn empty_session_trash_across_instances() -> Result<CodexSessionTrashDeleteSummary, String> {
    let entries = match load_trash_entries() {
        Ok(entries) => entries,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "清空 Codex 会话废纸篓前读取清单失败，将直接清理废纸篓目录: {}",
                error
            ));
            Vec::new()
        }
    };
    let requested_session_ids = entries
        .iter()
        .map(|entry| entry.manifest.session_id.clone())
        .collect::<HashSet<_>>();

    let mut freed_size_bytes = 0u64;
    let mut removed_root_count = 0usize;
    for root in get_session_trash_roots_for_read()? {
        if !root.path.exists() {
            continue;
        }
        freed_size_bytes =
            freed_size_bytes.saturating_add(calculate_path_size(&root.path).unwrap_or(0));
        match remove_path_recursively(&root.path) {
            Ok(()) => {
                removed_root_count += 1;
            }
            Err(error) if root.optional => {
                modules::logger::log_warn(&format!(
                    "清理旧 Codex 会话废纸篓失败，已跳过 ({}): {}",
                    root.path.display(),
                    error
                ));
            }
            Err(error) => return Err(error),
        }
    }

    Ok(CodexSessionTrashDeleteSummary {
        requested_session_count: requested_session_ids.len(),
        deleted_session_count: requested_session_ids.len(),
        deleted_entry_count: entries.len(),
        freed_size_bytes,
        message: if removed_root_count == 0 {
            "废纸篓为空，无需清理".to_string()
        } else {
            format!(
                "已清空 Codex 会话废纸篓，永久删除 {} 条会话，释放约 {}",
                requested_session_ids.len(),
                format_bytes(freed_size_bytes)
            )
        },
    })
}

pub fn restore_sessions_from_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<CodexSessionRestoreSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let entries = load_trash_entries()?
        .into_iter()
        .filter(|entry| requested_ids.contains(&entry.manifest.session_id))
        .collect::<Vec<_>>();

    if entries.is_empty() {
        return Ok(CodexSessionRestoreSummary {
            requested_session_count: requested_ids.len(),
            restored_session_count: 0,
            restored_instance_count: 0,
            message: "所选会话在废纸篓中不存在，无需恢复".to_string(),
        });
    }

    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let running_instance_ids = instances
        .iter()
        .filter(|instance| is_instance_running(instance, &process_entries))
        .map(|instance| instance.id.clone())
        .collect::<HashSet<_>>();

    let mut restored_session_ids = HashSet::new();
    let mut restored_instance_ids = HashSet::new();
    let mut metadata_rebuild_failed_count = 0usize;

    for entry in &entries {
        let outcome = restore_trashed_session_entry(entry)?;
        if outcome.metadata_rebuild_failed {
            metadata_rebuild_failed_count += 1;
        }
        restored_session_ids.insert(entry.manifest.session_id.clone());
        restored_instance_ids.insert(entry.manifest.instance_id.clone());
    }

    let restored_running_instance = restored_instance_ids
        .iter()
        .any(|instance_id| running_instance_ids.contains(instance_id));
    let mut message = if restored_running_instance {
        format!(
            "已恢复 {} 条会话，并已触发官方 Codex 重建会话索引；运行中的实例可能需要刷新或重启后显示",
            restored_session_ids.len()
        )
    } else {
        format!(
            "已恢复 {} 条会话，并已触发官方 Codex 重建会话索引",
            restored_session_ids.len()
        )
    };
    if metadata_rebuild_failed_count > 0 {
        message.push_str(&format!(
            "；{} 个实例的官方侧边栏索引重建未完成，重启 Codex 后会重新加载",
            metadata_rebuild_failed_count
        ));
    }

    Ok(CodexSessionRestoreSummary {
        requested_session_count: requested_ids.len(),
        restored_session_count: restored_session_ids.len(),
        restored_instance_count: restored_instance_ids.len(),
        message,
    })
}

pub fn preview_session_export(
    session_ids: Vec<String>,
) -> Result<CodexSessionExportPreview, String> {
    let requested_ids = normalize_session_id_list(session_ids);
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let selected_entries = collect_export_session_entries(&requested_ids)?;
    let mut items = Vec::with_capacity(selected_entries.len());
    let mut total_size_bytes = 0u64;

    for (instance, snapshot) in &selected_entries {
        let size_bytes = fs::metadata(&snapshot.rollout_path)
            .map_err(|error| {
                format!(
                    "读取会话文件大小失败 ({}): {}",
                    snapshot.rollout_path.display(),
                    error
                )
            })?
            .len();
        total_size_bytes = total_size_bytes.saturating_add(size_bytes);
        items.push(CodexSessionExportPreviewItem {
            session_id: snapshot.id.clone(),
            title: snapshot.title.clone(),
            cwd: snapshot.cwd.clone(),
            updated_at: snapshot.updated_at,
            size_bytes,
            source_instance_id: instance.id.clone(),
            source_instance_name: instance.name.clone(),
        });
    }

    Ok(CodexSessionExportPreview {
        requested_session_count: requested_ids.len(),
        exportable_session_count: items.len(),
        missing_session_count: requested_ids.len().saturating_sub(items.len()),
        total_size_bytes,
        items,
    })
}

pub fn export_sessions(
    session_ids: Vec<String>,
    export_path: String,
    transfer_id: Option<String>,
    progress_reporter: Option<SessionTransferProgressReporter<'_>>,
) -> Result<CodexSessionExportSummary, String> {
    let requested_ids = normalize_session_id_list(session_ids);
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }
    emit_session_transfer_progress(
        progress_reporter,
        transfer_id.as_deref(),
        "export",
        "collect",
        0,
        requested_ids.len(),
        None,
        true,
    );
    let export_path = PathBuf::from(export_path.trim());
    if export_path.as_os_str().is_empty() {
        return Err("请选择会话导出文件".to_string());
    }
    if let Some(parent) = export_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建会话导出目录失败 ({}): {}", parent.display(), error)
            })?;
        }
    }

    let selected_entries = collect_export_session_entries(&requested_ids)?;
    if selected_entries.is_empty() {
        return Ok(CodexSessionExportSummary {
            requested_session_count: requested_ids.len(),
            exported_session_count: 0,
            skipped_session_count: requested_ids.len(),
            export_path: export_path.to_string_lossy().to_string(),
            message: "所选会话在当前实例集合中不存在，未导出任何内容".to_string(),
        });
    }

    let mut manifest_items = Vec::with_capacity(selected_entries.len());
    for (index, (instance, snapshot)) in selected_entries.iter().enumerate() {
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id.as_deref(),
            "export",
            "hash",
            index,
            selected_entries.len(),
            Some(snapshot.title.clone()),
            true,
        );
        let (size_bytes, sha256) = sha256_file(&snapshot.rollout_path)?;
        let relative_rollout_path = snapshot_relative_rollout_path(snapshot);
        let file_entry = format!(
            "files/{:04}-{}/rollout.jsonl",
            index + 1,
            sanitize_for_file_name(&snapshot.id)
        );
        manifest_items.push(SessionExportManifestItem {
            session_id: snapshot.id.clone(),
            title: snapshot.title.clone(),
            cwd: snapshot.cwd.clone(),
            updated_at: snapshot.updated_at,
            relative_rollout_path,
            file_entry,
            size_bytes,
            sha256,
            session_index_entry: snapshot.session_index_entry.clone(),
            source_instance: SessionExportInstance {
                id: instance.id.clone(),
                name: instance.name.clone(),
            },
        });
    }
    emit_session_transfer_progress(
        progress_reporter,
        transfer_id.as_deref(),
        "export",
        "write",
        0,
        manifest_items.len(),
        None,
        true,
    );

    let manifest = SessionExportManifest {
        kind: SESSION_EXPORT_KIND.to_string(),
        package_version: SESSION_EXPORT_VERSION,
        exported_at: Utc::now().to_rfc3339(),
        sessions: manifest_items,
    };

    write_session_export_package(
        &export_path,
        &manifest,
        &selected_entries,
        transfer_id.as_deref(),
        progress_reporter,
    )?;
    emit_session_transfer_progress(
        progress_reporter,
        transfer_id.as_deref(),
        "export",
        "done",
        manifest.sessions.len(),
        manifest.sessions.len(),
        None,
        false,
    );

    Ok(CodexSessionExportSummary {
        requested_session_count: requested_ids.len(),
        exported_session_count: manifest.sessions.len(),
        skipped_session_count: requested_ids.len().saturating_sub(manifest.sessions.len()),
        export_path: export_path.to_string_lossy().to_string(),
        message: format!("已导出 {} 条会话", manifest.sessions.len()),
    })
}

pub fn preview_session_import(
    import_file_path: String,
    target_instance_id: Option<String>,
) -> Result<CodexSessionImportPreview, String> {
    let import_file_path = PathBuf::from(import_file_path.trim());
    if import_file_path.as_os_str().is_empty() {
        return Err("请选择会话包文件".to_string());
    }
    let manifest = read_session_export_manifest_from_path(&import_file_path)?;
    let target = resolve_session_import_target(target_instance_id)?;
    let target_snapshots = load_thread_snapshots(&target)?;
    let target_by_id = target_snapshots
        .into_iter()
        .map(|snapshot| (snapshot.id.clone(), snapshot))
        .collect::<HashMap<_, _>>();
    let existing_instance_names = collect_existing_session_instance_names()?;

    let mut items = Vec::with_capacity(manifest.sessions.len());
    for item in &manifest.sessions {
        let mut status = "ready".to_string();
        let mut reason = None;

        if validate_manifest_item(item).is_err() {
            status = "invalid".to_string();
            reason = Some("会话包条目无效".to_string());
        } else if let Some(existing) = target_by_id.get(&item.session_id) {
            let existing_hash = sha256_file(&existing.rollout_path)
                .map(|(_, hash)| hash)
                .unwrap_or_default();
            if existing_hash == item.sha256 {
                status = "duplicate".to_string();
                reason = Some("目标实例已存在相同会话".to_string());
            } else {
                status = "conflict".to_string();
                reason = Some("目标实例已存在同 ID 的不同会话，已跳过避免覆盖".to_string());
            }
        }

        items.push(CodexSessionImportPreviewItem {
            session_id: item.session_id.clone(),
            title: item.title.clone(),
            cwd: item.cwd.clone(),
            updated_at: item.updated_at,
            size_bytes: item.size_bytes,
            status,
            reason,
            existing_instance_names: existing_instance_names
                .get(&item.session_id)
                .cloned()
                .unwrap_or_default(),
        });
    }

    let importable_session_count = items.iter().filter(|item| item.status == "ready").count();
    Ok(CodexSessionImportPreview {
        package_version: manifest.package_version,
        exported_at: Some(manifest.exported_at),
        import_file_path: import_file_path.to_string_lossy().to_string(),
        target_instance_id: target.id,
        target_instance_name: target.name,
        total_session_count: items.len(),
        importable_session_count,
        items,
    })
}

pub fn import_sessions(
    import_file_path: String,
    target_instance_id: Option<String>,
    session_ids: Vec<String>,
    transfer_id: Option<String>,
    progress_reporter: Option<SessionTransferProgressReporter<'_>>,
) -> Result<CodexSessionImportSummary, String> {
    let requested_ids = normalize_session_id_list(session_ids);
    if requested_ids.is_empty() {
        return Err("请至少选择一条要导入的会话".to_string());
    }
    emit_session_transfer_progress(
        progress_reporter,
        transfer_id.as_deref(),
        "import",
        "read",
        0,
        requested_ids.len(),
        None,
        true,
    );
    let import_file_path = PathBuf::from(import_file_path.trim());
    if import_file_path.as_os_str().is_empty() {
        return Err("请选择会话包文件".to_string());
    }
    let target = resolve_session_import_target(target_instance_id)?;
    let manifest = read_session_export_manifest_from_path(&import_file_path)?;
    let manifest_by_id = manifest
        .sessions
        .iter()
        .map(|item| (item.session_id.clone(), item.clone()))
        .collect::<HashMap<_, _>>();

    let mut target_session_ids = load_thread_snapshots(&target)?
        .into_iter()
        .map(|snapshot| snapshot.id)
        .collect::<HashSet<_>>();
    let original_session_index_content = read_session_index_content(&target.data_dir)?;
    let mut imported_count = 0usize;
    let mut skipped_count = 0usize;
    let mut next_session_index_content = original_session_index_content.clone();

    let file = File::open(&import_file_path)
        .map_err(|error| format!("打开会话包失败 ({}): {}", import_file_path.display(), error))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("读取会话包失败: {}", error))?;

    for (index, session_id) in requested_ids.iter().enumerate() {
        let Some(item) = manifest_by_id.get(session_id) else {
            skipped_count += 1;
            continue;
        };
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id.as_deref(),
            "import",
            "write",
            index,
            requested_ids.len(),
            Some(item.title.clone()),
            true,
        );
        validate_manifest_item(item)?;
        if target_session_ids.contains(session_id) {
            skipped_count += 1;
            continue;
        }

        let target_rollout_path = resolve_import_target_rollout_path(&target.data_dir, item);
        let target_rollout_path = uniquify_rollout_path(&target_rollout_path);
        let written_path =
            write_imported_rollout_from_archive(&mut archive, item, &target_rollout_path)?;
        let session_index_entry = build_imported_session_index_entry(item, &written_path);
        if let Err(error) = upsert_session_index_with_entry(
            &target.data_dir,
            &next_session_index_content,
            session_id,
            &session_index_entry,
        ) {
            let _ = fs::remove_file(&written_path);
            let _ = restore_session_index_content(
                &target.data_dir,
                original_session_index_content.as_deref(),
            );
            return Err(error);
        }
        next_session_index_content = read_session_index_content(&target.data_dir)?;
        target_session_ids.insert(session_id.clone());
        imported_count += 1;
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id.as_deref(),
            "import",
            "write",
            index + 1,
            requested_ids.len(),
            Some(item.title.clone()),
            true,
        );
    }

    if imported_count > 0 {
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id.as_deref(),
            "import",
            "rebuild",
            requested_ids.len(),
            requested_ids.len(),
            Some(target.name.clone()),
            true,
        );
        if let Err(error) =
            modules::codex_official_app_server::rebuild_thread_metadata(&target.data_dir)
        {
            modules::logger::log_warn(&format!(
                "会话已导入，但官方 Codex 重建会话索引失败 ({}): {}",
                target.name, error
            ));
        }
    }
    emit_session_transfer_progress(
        progress_reporter,
        transfer_id.as_deref(),
        "import",
        "done",
        requested_ids.len(),
        requested_ids.len(),
        None,
        false,
    );

    Ok(CodexSessionImportSummary {
        requested_session_count: requested_ids.len(),
        imported_session_count: imported_count,
        skipped_session_count: skipped_count,
        target_instance_id: target.id,
        target_instance_name: target.name.clone(),
        message: if imported_count > 0 {
            format!(
                "已导入 {} 条会话到 {}；已跳过 {} 条",
                imported_count, target.name, skipped_count
            )
        } else {
            format!("没有导入新会话；已跳过 {} 条", skipped_count)
        },
    })
}

pub fn resolve_session_location_dir(session_id: String) -> Result<PathBuf, String> {
    let session_id = session_id.trim().to_string();
    if session_id.is_empty() {
        return Err("请选择一条会话".to_string());
    }
    let instances = collect_instances()?;
    let mut best_snapshot: Option<ThreadSnapshot> = None;
    for instance in &instances {
        for snapshot in load_thread_snapshots(instance)? {
            if snapshot.id != session_id {
                continue;
            }
            let should_replace = best_snapshot
                .as_ref()
                .map(|current| {
                    snapshot.updated_at.unwrap_or_default() > current.updated_at.unwrap_or_default()
                })
                .unwrap_or(true);
            if should_replace {
                best_snapshot = Some(snapshot);
            }
        }
    }
    let Some(snapshot) = best_snapshot else {
        return Err("未找到该会话文件".to_string());
    };
    snapshot
        .rollout_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "无法解析会话文件所在目录: {}",
                snapshot.rollout_path.display()
            )
        })
}

fn collect_instances() -> Result<Vec<CodexSyncInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = modules::codex_instance::get_default_codex_home()?;
    let store = modules::codex_instance::load_instance_store()?;
    instances.push(CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
        last_pid: store.default_settings.last_pid,
    });

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(CodexSyncInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
            last_pid: instance.last_pid,
        });
    }

    Ok(instances)
}

fn normalize_session_id_list(session_ids: Vec<String>) -> HashSet<String> {
    session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>()
}

fn collect_export_session_entries(
    requested_ids: &HashSet<String>,
) -> Result<Vec<(CodexSyncInstance, ThreadSnapshot)>, String> {
    let instances = collect_instances()?;
    let mut selected = HashMap::<String, (CodexSyncInstance, ThreadSnapshot)>::new();
    for instance in &instances {
        for snapshot in load_thread_snapshots(instance)? {
            if !requested_ids.contains(&snapshot.id) {
                continue;
            }
            let should_replace = selected
                .get(&snapshot.id)
                .map(|(_, current)| {
                    snapshot.updated_at.unwrap_or_default() > current.updated_at.unwrap_or_default()
                })
                .unwrap_or(true);
            if should_replace {
                selected.insert(snapshot.id.clone(), (instance.clone(), snapshot));
            }
        }
    }

    let mut selected_entries = selected.into_values().collect::<Vec<_>>();
    selected_entries.sort_by(|left, right| {
        right
            .1
            .updated_at
            .unwrap_or_default()
            .cmp(&left.1.updated_at.unwrap_or_default())
            .then_with(|| left.1.title.cmp(&right.1.title))
    });
    Ok(selected_entries)
}

fn emit_session_transfer_progress(
    progress_reporter: Option<SessionTransferProgressReporter<'_>>,
    transfer_id: Option<&str>,
    operation: &str,
    phase: &str,
    current: usize,
    total: usize,
    current_label: Option<String>,
    running: bool,
) {
    let (Some(progress_reporter), Some(transfer_id)) = (progress_reporter, transfer_id) else {
        return;
    };
    let percent = if total == 0 {
        0
    } else {
        ((current.min(total) * 100) / total).min(100) as u8
    };
    progress_reporter(CodexSessionTransferProgress {
        transfer_id: transfer_id.to_string(),
        operation: operation.to_string(),
        phase: phase.to_string(),
        current: current.min(total),
        total,
        percent,
        current_label,
        running,
    });
}

fn resolve_session_import_target(
    target_instance_id: Option<String>,
) -> Result<CodexSyncInstance, String> {
    let target_id = target_instance_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_INSTANCE_ID)
        .to_string();
    collect_instances()?
        .into_iter()
        .find(|instance| instance.id == target_id)
        .ok_or_else(|| "目标实例不存在".to_string())
}

fn snapshot_relative_rollout_path(snapshot: &ThreadSnapshot) -> String {
    snapshot
        .rollout_path
        .strip_prefix(&snapshot.source_root)
        .ok()
        .and_then(path_to_package_path)
        .unwrap_or_else(|| generated_import_rollout_relative_path(&snapshot.id))
}

fn path_to_package_path(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => {
                let part = value.to_str()?.trim();
                if part.is_empty() || part == "." || part == ".." || part.contains(':') {
                    return None;
                }
                parts.push(part.to_string());
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn normalize_package_entry_path(value: &str) -> Option<String> {
    let normalized = value.trim().replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return None;
    }
    let parts = normalized.split('/').collect::<Vec<_>>();
    if parts.is_empty() {
        return None;
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." || part.contains(':') {
            return None;
        }
    }
    Some(parts.join("/"))
}

fn validate_manifest_item(item: &SessionExportManifestItem) -> Result<(), String> {
    if item.session_id.trim().is_empty() {
        return Err("会话包中存在空会话 ID".to_string());
    }
    let file_entry = normalize_package_entry_path(&item.file_entry)
        .ok_or_else(|| format!("会话包文件路径无效: {}", item.file_entry))?;
    if !file_entry.starts_with("files/") || !file_entry.ends_with(".jsonl") {
        return Err(format!("会话包文件路径无效: {}", item.file_entry));
    }
    if item.sha256.len() != 64 || !item.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("会话包 hash 无效: {}", item.session_id));
    }
    Ok(())
}

fn write_session_export_package(
    export_path: &Path,
    manifest: &SessionExportManifest,
    selected_entries: &[(CodexSyncInstance, ThreadSnapshot)],
    transfer_id: Option<&str>,
    progress_reporter: Option<SessionTransferProgressReporter<'_>>,
) -> Result<(), String> {
    let file = File::create(export_path).map_err(|error| {
        format!(
            "创建会话导出文件失败 ({}): {}",
            export_path.display(),
            error
        )
    })?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    zip.start_file("manifest.json", options)
        .map_err(|error| format!("写入会话包清单失败: {}", error))?;
    let manifest_content = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("序列化会话包清单失败: {}", error))?;
    zip.write_all(&manifest_content)
        .map_err(|error| format!("写入会话包清单失败: {}", error))?;

    for (index, (item, (_, snapshot))) in manifest
        .sessions
        .iter()
        .zip(selected_entries.iter())
        .enumerate()
    {
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id,
            "export",
            "write",
            index,
            manifest.sessions.len(),
            Some(item.title.clone()),
            true,
        );
        zip.start_file(item.file_entry.as_str(), options)
            .map_err(|error| format!("写入会话包文件失败 ({}): {}", item.file_entry, error))?;
        let mut source = File::open(&snapshot.rollout_path).map_err(|error| {
            format!(
                "打开会话 rollout 文件失败 ({}): {}",
                snapshot.rollout_path.display(),
                error
            )
        })?;
        std::io::copy(&mut source, &mut zip)
            .map_err(|error| format!("写入会话包文件失败 ({}): {}", item.file_entry, error))?;
        emit_session_transfer_progress(
            progress_reporter,
            transfer_id,
            "export",
            "write",
            index + 1,
            manifest.sessions.len(),
            Some(item.title.clone()),
            true,
        );
    }

    zip.finish()
        .map_err(|error| format!("完成会话导出文件失败: {}", error))?;
    Ok(())
}

fn read_session_export_manifest_from_path(
    import_file_path: &Path,
) -> Result<SessionExportManifest, String> {
    let file = File::open(import_file_path)
        .map_err(|error| format!("打开会话包失败 ({}): {}", import_file_path.display(), error))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("读取会话包失败: {}", error))?;
    read_session_export_manifest(&mut archive)
}

fn read_session_export_manifest(
    archive: &mut ZipArchive<File>,
) -> Result<SessionExportManifest, String> {
    let mut manifest_file = archive
        .by_name("manifest.json")
        .map_err(|error| format!("会话包缺少 manifest.json: {}", error))?;
    let mut content = String::new();
    manifest_file
        .read_to_string(&mut content)
        .map_err(|error| format!("读取会话包清单失败: {}", error))?;
    let manifest = serde_json::from_str::<SessionExportManifest>(&content)
        .map_err(|error| format!("解析会话包清单失败: {}", error))?;
    if manifest.kind != SESSION_EXPORT_KIND {
        return Err("这不是 Cockpit Tools Codex 会话包".to_string());
    }
    if manifest.package_version == 0 || manifest.package_version > SESSION_EXPORT_VERSION {
        return Err(format!("不支持的会话包版本: {}", manifest.package_version));
    }
    Ok(manifest)
}

fn collect_existing_session_instance_names() -> Result<HashMap<String, Vec<String>>, String> {
    let mut result = HashMap::<String, Vec<String>>::new();
    for instance in collect_instances()? {
        for snapshot in load_thread_snapshots(&instance)? {
            let names = result.entry(snapshot.id).or_default();
            if !names.iter().any(|name| name == &instance.name) {
                names.push(instance.name.clone());
            }
        }
    }
    Ok(result)
}

fn resolve_import_target_rollout_path(
    target_root: &Path,
    item: &SessionExportManifestItem,
) -> PathBuf {
    let relative_path = normalize_package_entry_path(&item.relative_rollout_path)
        .filter(|path| is_safe_rollout_relative_path(path))
        .unwrap_or_else(|| generated_import_rollout_relative_path(&item.session_id));
    target_root.join(PathBuf::from(relative_path))
}

fn is_safe_rollout_relative_path(value: &str) -> bool {
    let Some(first) = value.split('/').next() else {
        return false;
    };
    if !SESSION_DIRS.contains(&first) {
        return false;
    }
    let Some(file_name) = value.rsplit('/').next() else {
        return false;
    };
    file_name.starts_with("rollout-") && file_name.ends_with(".jsonl")
}

fn generated_import_rollout_relative_path(session_id: &str) -> String {
    format!(
        "sessions/imported/{}/rollout-{}.jsonl",
        Utc::now().format("%Y/%m/%d"),
        sanitize_for_file_name(session_id)
    )
}

fn uniquify_rollout_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("rollout");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{}", value))
        .unwrap_or_default();
    for index in 1..1000 {
        let candidate = parent.join(format!("{}-import-{}{}", stem, index, extension));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{}-import-{}{}", stem, Uuid::new_v4(), extension))
}

fn write_imported_rollout_from_archive(
    archive: &mut ZipArchive<File>,
    item: &SessionExportManifestItem,
    target_path: &Path,
) -> Result<PathBuf, String> {
    let entry_name = normalize_package_entry_path(&item.file_entry)
        .ok_or_else(|| format!("会话包文件路径无效: {}", item.file_entry))?;
    let mut zip_file = archive
        .by_name(&entry_name)
        .map_err(|error| format!("会话包缺少会话文件 ({}): {}", item.file_entry, error))?;
    let parent = target_path
        .parent()
        .ok_or_else(|| format!("无法解析目标会话目录: {}", target_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建目标会话目录失败 ({}): {}", parent.display(), error))?;
    let temp_path = parent.join(format!(".cockpit-session-import-{}.tmp", Uuid::new_v4()));
    let mut output = File::create(&temp_path)
        .map_err(|error| format!("创建临时会话文件失败 ({}): {}", temp_path.display(), error))?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];

    let write_result = (|| -> Result<(), String> {
        loop {
            let bytes_read = zip_file
                .read(&mut buffer)
                .map_err(|error| format!("读取会话包文件失败 ({}): {}", item.file_entry, error))?;
            if bytes_read == 0 {
                break;
            }
            output.write_all(&buffer[..bytes_read]).map_err(|error| {
                format!("写入临时会话文件失败 ({}): {}", temp_path.display(), error)
            })?;
            hasher.update(&buffer[..bytes_read]);
            size_bytes += bytes_read as u64;
        }
        output.flush().map_err(|error| {
            format!("写入临时会话文件失败 ({}): {}", temp_path.display(), error)
        })?;
        Ok(())
    })();

    drop(output);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    let sha256 = hex_lower(hasher.finalize().as_slice());
    if size_bytes != item.size_bytes || !sha256.eq_ignore_ascii_case(&item.sha256) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("会话包文件校验失败: {}", item.session_id));
    }
    fs::rename(&temp_path, target_path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!(
            "写入目标会话文件失败 ({}): {}",
            target_path.display(),
            error
        )
    })?;
    modules::codex_session_file_time::restore_modified_time(
        target_path,
        system_time_from_unix_seconds(item.updated_at),
    )?;
    Ok(target_path.to_path_buf())
}

fn build_imported_session_index_entry(
    item: &SessionExportManifestItem,
    rollout_path: &Path,
) -> JsonValue {
    let mut imported = item.session_index_entry.clone();
    if !imported.is_object() {
        imported = json!({});
    }
    let Some(object) = imported.as_object_mut() else {
        return json!({
            "id": item.session_id.clone(),
            "thread_name": item.title.clone(),
        });
    };
    object.insert("id".to_string(), JsonValue::String(item.session_id.clone()));
    if !item.title.trim().is_empty() {
        object
            .entry("thread_name".to_string())
            .or_insert_with(|| JsonValue::String(item.title.clone()));
    }
    if let Some(updated_at) = item
        .updated_at
        .or_else(|| rollout_file_activity_seconds(rollout_path))
        .or_else(|| rollout_file_modified_seconds(rollout_path))
    {
        object.insert(
            "updated_at".to_string(),
            JsonValue::String(format_session_index_updated_at(updated_at)),
        );
    }
    imported
}

fn sha256_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path)
        .map_err(|error| format!("打开文件失败 ({}): {}", path.display(), error))?;
    let mut hasher = Sha256::new();
    let mut size_bytes = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取文件失败 ({}): {}", path.display(), error))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
        size_bytes += bytes_read as u64;
    }
    Ok((size_bytes, hex_lower(hasher.finalize().as_slice())))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

fn system_time_from_unix_seconds(value: Option<i64>) -> Option<SystemTime> {
    let seconds = value?;
    if seconds < 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
}

fn is_instance_running(
    instance: &CodexSyncInstance,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let codex_home = instance.data_dir.to_str();
    modules::process::resolve_codex_pid_from_entries(instance.last_pid, codex_home, process_entries)
        .is_some()
}

fn load_thread_snapshots(instance: &CodexSyncInstance) -> Result<Vec<ThreadSnapshot>, String> {
    let session_index_map = read_session_index_map(&instance.data_dir)?;
    let mut snapshots = Vec::new();
    for dir_name in SESSION_DIRS {
        let root_dir = instance.data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let Some(session_meta) = read_rollout_session_meta(&rollout_path)? else {
                continue;
            };
            let Some(id) = session_meta_id(&session_meta) else {
                continue;
            };
            let title = session_index_map
                .get(&id)
                .and_then(session_index_title)
                .unwrap_or_else(|| id.clone());
            let cwd = session_meta_cwd(&session_meta).unwrap_or_else(|| "未知工作目录".to_string());
            let updated_at = resolve_thread_snapshot_updated_at_seconds(
                session_index_map.get(&id),
                &rollout_path,
            );
            let session_index_entry = session_index_map
                .get(&id)
                .cloned()
                .unwrap_or_else(|| json!({ "id": id, "thread_name": title }));

            snapshots.push(ThreadSnapshot {
                id,
                title,
                cwd,
                updated_at,
                rollout_path,
                session_index_entry,
                source_root: instance.data_dir.clone(),
            });
        }
    }

    Ok(snapshots)
}

fn list_rollout_files(root_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root_dir)
        .map_err(|error| format!("读取目录失败 ({}): {}", root_dir.display(), error))?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取目录项失败 ({}): {}", root_dir.display(), error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取文件类型失败 ({}): {}", path.display(), error))?;
        if file_type.is_dir() {
            result.extend(list_rollout_files(&path)?);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or_default();
        if file_name.starts_with("rollout-") && file_name.ends_with(".jsonl") {
            result.push(path);
        }
    }

    result.sort();
    Ok(result)
}

fn read_rollout_session_meta(path: &Path) -> Result<Option<JsonValue>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("打开 rollout 文件失败 ({}): {}", path.display(), error))?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line =
            line.map_err(|error| format!("读取 rollout 文件失败 ({}): {}", path.display(), error))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            return Ok(None);
        };
        if parsed.get("type").and_then(JsonValue::as_str) == Some("session_meta") {
            return Ok(Some(parsed));
        }
        return Ok(None);
    }
    Ok(None)
}

fn session_meta_id(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("id").or_else(|| payload.get("session_id")))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .or_else(|| {
            meta.get("id")
                .or_else(|| meta.get("session_id"))
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
}

fn session_meta_cwd(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("cwd"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn trash_snapshots_for_instance(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<TrashSnapshotsOutcome, String> {
    trash_snapshots_for_instance_with_rebuild(instance, trash_root, snapshots, |data_dir| {
        modules::codex_official_app_server::rebuild_thread_metadata(data_dir)
    })
}

fn trash_snapshots_for_instance_with_rebuild<F>(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshots: &[ThreadSnapshot],
    rebuild_metadata: F,
) -> Result<TrashSnapshotsOutcome, String>
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    for snapshot in snapshots {
        move_snapshot_rollout_to_trash(instance, trash_root, snapshot)?;
    }

    rewrite_session_index_without_ids(&instance.data_dir, snapshots)?;
    let mut metadata_rebuild_failed = false;
    if let Err(error) = rebuild_metadata(&instance.data_dir) {
        metadata_rebuild_failed = true;
        modules::logger::log_warn(&format!(
            "会话文件已移到废纸篓，但官方 Codex 重建会话索引失败 ({}): {}",
            instance.name, error
        ));
    }

    Ok(TrashSnapshotsOutcome {
        metadata_rebuild_failed,
    })
}

fn create_trash_root_dir() -> Result<PathBuf, String> {
    let root = get_session_trash_base_dir()?.join(Utc::now().format("%Y%m%d-%H%M%S").to_string());
    fs::create_dir_all(&root)
        .map_err(|error| format!("创建会话废纸篓目录失败 ({}): {}", root.display(), error))?;
    Ok(root)
}

fn get_session_trash_base_dir() -> Result<PathBuf, String> {
    Ok(modules::account::get_data_dir()?.join(SESSION_TRASH_ROOT_DIR))
}

fn get_legacy_session_trash_base_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".Trash").join(SESSION_TRASH_ROOT_DIR))
}

fn get_session_trash_roots_for_read() -> Result<Vec<TrashRoot>, String> {
    let primary = get_session_trash_base_dir()?;
    let mut roots = vec![TrashRoot {
        path: primary.clone(),
        optional: false,
    }];
    if let Some(legacy) = get_legacy_session_trash_base_dir() {
        if legacy != primary {
            roots.push(TrashRoot {
                path: legacy,
                optional: true,
            });
        }
    }
    Ok(roots)
}

fn move_snapshot_rollout_to_trash(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshot: &ThreadSnapshot,
) -> Result<(), String> {
    if !snapshot.rollout_path.exists() {
        return Ok(());
    }

    let relative_path = snapshot
        .rollout_path
        .strip_prefix(&snapshot.source_root)
        .unwrap_or(snapshot.rollout_path.as_path());
    let entry_dir = trash_root.join(format!(
        "{}--{}",
        sanitize_for_file_name(&instance.id),
        sanitize_for_file_name(&snapshot.id)
    ));
    let file_target = entry_dir.join("files").join(relative_path);
    if let Some(parent) = file_target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建废纸篓会话目录失败 ({}): {}", parent.display(), error))?;
    }

    let manifest = json!({
        "sessionId": snapshot.id,
        "title": snapshot.title,
        "cwd": snapshot.cwd,
        "instanceId": instance.id,
        "instanceName": instance.name,
        "instanceRoot": instance.data_dir,
        "originalRolloutPath": snapshot.rollout_path,
        "relativeRolloutPath": relative_path.to_string_lossy(),
        "sessionIndexEntry": snapshot.session_index_entry,
        "deletedAt": Utc::now().to_rfc3339(),
    });

    fs::create_dir_all(&entry_dir)
        .map_err(|error| format!("创建废纸篓条目失败 ({}): {}", entry_dir.display(), error))?;
    let manifest_path = entry_dir.join("manifest.json");
    let manifest_content = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| format!("序列化会话废纸篓清单失败: {}", error))?
    );
    modules::atomic_write::write_string_atomic(&manifest_path, &manifest_content).map_err(
        |error| {
            format!(
                "写入会话废纸篓清单失败 ({}): {}",
                entry_dir.display(),
                error
            )
        },
    )?;
    fs::rename(&snapshot.rollout_path, &file_target).map_err(|error| {
        format!(
            "移动会话文件到废纸篓失败 ({} -> {}): {}",
            snapshot.rollout_path.display(),
            file_target.display(),
            error
        )
    })?;
    Ok(())
}

fn rewrite_session_index_without_ids(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(());
    }

    let removed_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.id.as_str())
        .collect::<HashSet<_>>();
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    let retained = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return false;
            }
            match serde_json::from_str::<JsonValue>(trimmed) {
                Ok(value) => value
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .map(|id| !removed_ids.contains(id))
                    .unwrap_or(true),
                Err(_) => true,
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let final_content = if retained.is_empty() {
        String::new()
    } else {
        format!("{}\n", retained)
    };
    modules::atomic_write::write_string_atomic(&path, &final_content).map_err(|error| {
        format!(
            "重写 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(())
}

fn read_session_index_map(root_dir: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    let mut entries = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = parsed.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), parsed);
    }

    Ok(entries)
}

fn sanitize_for_file_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
}

fn session_index_title(entry: &JsonValue) -> Option<String> {
    ["thread_name", "threadName", "title", "name"]
        .iter()
        .filter_map(|key| entry.get(*key))
        .find_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_session_index_updated_at_seconds(entry: &JsonValue) -> Option<i64> {
    [
        "updated_at",
        "updatedAt",
        "last_updated_at",
        "lastUpdatedAt",
    ]
    .iter()
    .filter_map(|key| entry.get(*key))
    .find_map(parse_json_timestamp_seconds)
}

fn resolve_thread_snapshot_updated_at_seconds(
    session_index_entry: Option<&JsonValue>,
    rollout_path: &Path,
) -> Option<i64> {
    let indexed = session_index_entry.and_then(parse_session_index_updated_at_seconds);
    let activity = rollout_file_activity_seconds(rollout_path);
    let resolved = match (indexed, activity) {
        (Some(indexed), Some(activity))
            if indexed.abs_diff(activity) > SESSION_INDEX_ACTIVITY_DRIFT_SECONDS as u64 =>
        {
            Some(activity)
        }
        (Some(indexed), _) => Some(indexed),
        (None, Some(activity)) => Some(activity),
        (None, None) => None,
    };
    resolved.or_else(|| rollout_file_modified_seconds(rollout_path))
}

fn rollout_file_activity_seconds(path: &Path) -> Option<i64> {
    let metadata = fs::metadata(path).ok()?;
    let file_len = metadata.len();
    let mut file = File::open(path).ok()?;
    let mut offset = file_len;
    let mut scanned_bytes = 0u64;
    let mut pending_prefix = Vec::new();

    while offset > 0 && scanned_bytes < ROLLOUT_ACTIVITY_MAX_SCAN_BYTES {
        let remaining_scan = ROLLOUT_ACTIVITY_MAX_SCAN_BYTES - scanned_bytes;
        let chunk_len = ROLLOUT_ACTIVITY_READ_CHUNK_BYTES
            .min(offset as usize)
            .min(remaining_scan as usize);
        if chunk_len == 0 {
            break;
        }
        offset -= chunk_len as u64;
        scanned_bytes += chunk_len as u64;

        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk = vec![0u8; chunk_len];
        file.read_exact(&mut chunk).ok()?;

        let starts_on_line_boundary =
            offset == 0 || byte_before_is_newline(&mut file, offset).ok()?;
        chunk.extend_from_slice(&pending_prefix);

        let parse_from_index = if starts_on_line_boundary {
            pending_prefix.clear();
            0
        } else if let Some(newline_index) = chunk.iter().position(|byte| *byte == b'\n') {
            pending_prefix = chunk[..newline_index].to_vec();
            newline_index + 1
        } else {
            pending_prefix = chunk;
            continue;
        };

        if let Some(timestamp) = parse_latest_rollout_activity_seconds(&chunk[parse_from_index..]) {
            return Some(timestamp);
        }
    }

    if offset == 0 && !pending_prefix.is_empty() {
        parse_latest_rollout_activity_seconds(&pending_prefix)
    } else {
        None
    }
}

fn parse_latest_rollout_activity_seconds(content: &[u8]) -> Option<i64> {
    for line in content.split(|byte| *byte == b'\n').rev() {
        let raw = String::from_utf8_lossy(line);
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        if let Some(timestamp) = parse_rollout_line_timestamp_seconds(&parsed) {
            return Some(timestamp);
        }
    }

    None
}

fn parse_rollout_line_timestamp_seconds(value: &JsonValue) -> Option<i64> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .and_then(parse_json_timestamp_seconds)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| {
                    payload
                        .get("timestamp")
                        .or_else(|| payload.get("time"))
                        .or_else(|| payload.get("created_at"))
                        .or_else(|| payload.get("createdAt"))
                })
                .and_then(parse_json_timestamp_seconds)
        })
}

fn parse_json_timestamp_seconds(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_seconds),
        JsonValue::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp())
            .or_else(|| {
                text.parse::<i64>()
                    .ok()
                    .map(normalize_codex_timestamp_seconds)
            }),
        _ => None,
    }
}

fn normalize_codex_timestamp_seconds(timestamp: i64) -> i64 {
    if timestamp > 10_000_000_000_000 {
        timestamp / 1_000_000
    } else if timestamp > 10_000_000_000 {
        timestamp / 1_000
    } else {
        timestamp
    }
}

fn rollout_file_modified_seconds(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| i64::try_from(value.as_secs()).ok())
}

fn parse_deleted_at(value: Option<&str>) -> Option<i64> {
    let parsed = value.and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())?;
    Some(parsed.timestamp())
}

fn load_trash_entries() -> Result<Vec<TrashedSessionEntry>, String> {
    load_trash_entries_from_roots(&get_session_trash_roots_for_read()?)
}

fn load_trash_entries_from_roots(roots: &[TrashRoot]) -> Result<Vec<TrashedSessionEntry>, String> {
    let mut entries = Vec::new();

    for root in roots {
        let mut root_entries = match load_trash_entries_from_root(root) {
            Ok(root_entries) => root_entries,
            Err(error) if root.optional => {
                modules::logger::log_warn(&format!(
                    "跳过旧会话废纸篓目录，读取失败 ({}): {}",
                    root.path.display(),
                    error
                ));
                continue;
            }
            Err(error) => return Err(error),
        };
        entries.append(&mut root_entries);
    }

    entries.sort_by(|left, right| {
        parse_deleted_at(right.manifest.deleted_at.as_deref())
            .unwrap_or_default()
            .cmp(&parse_deleted_at(left.manifest.deleted_at.as_deref()).unwrap_or_default())
            .then_with(|| left.manifest.session_id.cmp(&right.manifest.session_id))
            .then_with(|| left.manifest.instance_id.cmp(&right.manifest.instance_id))
    });
    Ok(entries)
}

fn load_trash_entries_from_root(root: &TrashRoot) -> Result<Vec<TrashedSessionEntry>, String> {
    let root = &root.path;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    let timestamp_dirs = fs::read_dir(&root)
        .map_err(|error| format!("读取会话废纸篓目录失败 ({}): {}", root.display(), error))?;
    for timestamp_dir in timestamp_dirs {
        let timestamp_dir = timestamp_dir
            .map_err(|error| format!("读取会话废纸篓目录项失败 ({}): {}", root.display(), error))?;
        let timestamp_path = timestamp_dir.path();
        let file_type = timestamp_dir.file_type().map_err(|error| {
            format!(
                "读取会话废纸篓目录类型失败 ({}): {}",
                timestamp_path.display(),
                error
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let entry_dirs = fs::read_dir(&timestamp_path).map_err(|error| {
            format!(
                "读取会话废纸篓批次目录失败 ({}): {}",
                timestamp_path.display(),
                error
            )
        })?;
        for entry in entry_dirs {
            let entry = entry.map_err(|error| {
                format!(
                    "读取会话废纸篓条目失败 ({}): {}",
                    timestamp_path.display(),
                    error
                )
            })?;
            let entry_path = entry.path();
            let entry_type = entry.file_type().map_err(|error| {
                format!(
                    "读取会话废纸篓条目类型失败 ({}): {}",
                    entry_path.display(),
                    error
                )
            })?;
            if !entry_type.is_dir() {
                continue;
            }

            let manifest_path = entry_path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }
            let manifest_content = fs::read_to_string(&manifest_path).map_err(|error| {
                format!(
                    "读取会话废纸篓清单失败 ({}): {}",
                    manifest_path.display(),
                    error
                )
            })?;
            let manifest = serde_json::from_str::<TrashedSessionManifest>(&manifest_content)
                .map_err(|error| {
                    format!(
                        "解析会话废纸篓清单失败 ({}): {}",
                        manifest_path.display(),
                        error
                    )
                })?;
            let trashed_rollout_path = entry_path
                .join("files")
                .join(PathBuf::from(&manifest.relative_rollout_path));
            entries.push(TrashedSessionEntry {
                entry_dir: entry_path,
                manifest,
                trashed_rollout_path,
            });
        }
    }

    Ok(entries)
}

fn restore_trashed_session_entry(
    entry: &TrashedSessionEntry,
) -> Result<RestoreTrashedSessionOutcome, String> {
    restore_trashed_session_entry_with_metadata_rebuild(entry, true)
}

fn restore_trashed_session_entry_with_metadata_rebuild(
    entry: &TrashedSessionEntry,
    rebuild_metadata: bool,
) -> Result<RestoreTrashedSessionOutcome, String> {
    if !entry.trashed_rollout_path.exists() {
        return Err(format!(
            "废纸篓中的会话文件不存在，无法恢复 ({}): {}",
            entry.manifest.session_id,
            entry.trashed_rollout_path.display()
        ));
    }

    let session_id = entry.manifest.session_id.clone();
    if let Some(trashed_session_id) = rollout_session_id(&entry.trashed_rollout_path)? {
        if trashed_session_id != session_id {
            return Err(format!(
                "废纸篓中的会话文件与清单不一致，无法恢复 (清单: {}, 文件: {}): {}",
                session_id,
                trashed_session_id,
                entry.trashed_rollout_path.display()
            ));
        }
    }

    let target_rollout_path = entry.manifest.original_rollout_path.clone();
    let original_session_index_content = read_session_index_content(&entry.manifest.instance_root)?;
    let target_existed_before_restore = target_rollout_path.exists();

    if target_existed_before_restore {
        match rollout_session_id(&target_rollout_path)? {
            Some(existing_session_id) if existing_session_id == session_id => {}
            Some(existing_session_id) => {
                return Err(format!(
                    "目标位置已存在不同会话文件，为避免覆盖，无法恢复 (待恢复: {}, 已存在: {}): {}",
                    session_id,
                    existing_session_id,
                    target_rollout_path.display()
                ));
            }
            None => {
                return Err(format!(
                    "目标位置已存在无法确认会话 ID 的文件，为避免覆盖，无法恢复 ({}): {}",
                    session_id,
                    target_rollout_path.display()
                ));
            }
        }
    } else {
        if let Some(parent) = target_rollout_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建会话恢复目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        fs::copy(&entry.trashed_rollout_path, &target_rollout_path).map_err(|error| {
            format!(
                "恢复会话文件失败 ({} -> {}): {}",
                entry.trashed_rollout_path.display(),
                target_rollout_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target_rollout_path,
            modules::codex_session_file_time::read_modified_time(&entry.trashed_rollout_path),
        )?;
    }

    let restore_result = (|| {
        let session_index_entry = build_restored_session_index_entry(entry, &target_rollout_path);
        upsert_session_index_with_entry(
            &entry.manifest.instance_root,
            &original_session_index_content,
            &session_id,
            &session_index_entry,
        )?;
        Ok::<(), String>(())
    })();

    if let Err(error) = restore_result {
        if !target_existed_before_restore {
            let _ = fs::remove_file(&target_rollout_path);
        }
        let _ = restore_session_index_content(
            &entry.manifest.instance_root,
            original_session_index_content.as_deref(),
        );
        return Err(error);
    }

    let mut metadata_rebuild_failed = false;
    if rebuild_metadata {
        if let Err(error) = modules::codex_official_app_server::rebuild_thread_metadata(
            &entry.manifest.instance_root,
        ) {
            metadata_rebuild_failed = true;
            modules::logger::log_warn(&format!(
                "会话已恢复，但官方 Codex 重建会话索引失败 ({}): {}",
                entry.manifest.instance_name, error
            ));
        }
    }

    if let Err(error) = fs::remove_dir_all(&entry.entry_dir) {
        modules::logger::log_warn(&format!(
            "会话已恢复，但清理废纸篓条目失败 ({}): {}",
            entry.entry_dir.display(),
            error
        ));
    } else {
        cleanup_empty_trash_ancestors(&entry.entry_dir);
    }

    Ok(RestoreTrashedSessionOutcome {
        metadata_rebuild_failed,
    })
}

fn read_session_index_content(root_dir: &Path) -> Result<Option<String>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(Some(content))
}

fn rollout_session_id(path: &Path) -> Result<Option<String>, String> {
    Ok(read_rollout_session_meta(path)?.and_then(|meta| session_meta_id(&meta)))
}

fn format_session_index_updated_at(seconds: i64) -> String {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn build_restored_session_index_entry(
    entry: &TrashedSessionEntry,
    rollout_path: &Path,
) -> JsonValue {
    let mut restored = entry.manifest.session_index_entry.clone();
    if !restored.is_object() {
        restored = json!({});
    }
    let Some(object) = restored.as_object_mut() else {
        return json!({
            "id": entry.manifest.session_id.clone(),
            "thread_name": entry.manifest.title.clone(),
        });
    };
    object.insert(
        "id".to_string(),
        JsonValue::String(entry.manifest.session_id.clone()),
    );
    if !entry.manifest.title.trim().is_empty() {
        object
            .entry("thread_name".to_string())
            .or_insert_with(|| JsonValue::String(entry.manifest.title.clone()));
    }
    if let Some(updated_at) = rollout_file_activity_seconds(rollout_path)
        .or_else(|| rollout_file_modified_seconds(rollout_path))
    {
        object.insert(
            "updated_at".to_string(),
            JsonValue::String(format_session_index_updated_at(updated_at)),
        );
    }
    restored
}

fn merge_session_index_entry(existing: JsonValue, restored: &JsonValue) -> JsonValue {
    let (JsonValue::Object(mut existing_object), JsonValue::Object(restored_object)) =
        (existing, restored)
    else {
        return restored.clone();
    };
    for (key, value) in restored_object {
        existing_object.insert(key.clone(), value.clone());
    }
    JsonValue::Object(existing_object)
}

fn upsert_session_index_with_entry(
    root_dir: &Path,
    original_content: &Option<String>,
    session_id: &str,
    entry: &JsonValue,
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    let serialized_entry = serde_json::to_string(entry)
        .map_err(|error| format!("序列化 session_index 条目失败 ({}): {}", session_id, error))?;
    let lines = original_content
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let mut next_lines = Vec::with_capacity(lines.len() + 1);
    let mut replaced = false;
    for line in lines {
        let parsed = serde_json::from_str::<JsonValue>(&line);
        let Ok(parsed) = parsed else {
            next_lines.push(line);
            continue;
        };
        let current_id = parsed.get("id").and_then(JsonValue::as_str);
        if current_id != Some(session_id) {
            next_lines.push(line);
            continue;
        }
        if replaced {
            continue;
        }
        let merged = merge_session_index_entry(parsed, entry);
        next_lines.push(serde_json::to_string(&merged).map_err(|error| {
            format!("序列化 session_index 条目失败 ({}): {}", session_id, error)
        })?);
        replaced = true;
    }
    if !replaced {
        next_lines.push(serialized_entry);
    }
    let next_content = if next_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", next_lines.join("\n"))
    };
    modules::atomic_write::write_string_atomic(&path, &next_content).map_err(|error| {
        format!(
            "写入 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(())
}

fn restore_session_index_content(root_dir: &Path, content: Option<&str>) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    match content {
        Some(value) => {
            modules::atomic_write::write_string_atomic(&path, value).map_err(|error| {
                format!(
                    "恢复 session_index.jsonl 失败 ({}): {}",
                    path.display(),
                    error
                )
            })?
        }
        None => {
            if path.exists() {
                fs::remove_file(&path).map_err(|error| {
                    format!(
                        "删除恢复失败的 session_index.jsonl 失败 ({}): {}",
                        path.display(),
                        error
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn delete_trash_entries(
    entries: &[TrashedSessionEntry],
) -> Result<(HashSet<String>, usize, u64), String> {
    let mut deleted_session_ids = HashSet::new();
    let mut deleted_entry_count = 0usize;
    let mut freed_size_bytes = 0u64;

    for entry in entries {
        freed_size_bytes =
            freed_size_bytes.saturating_add(calculate_path_size(&entry.entry_dir).unwrap_or(0));
        remove_path_recursively(&entry.entry_dir)?;
        cleanup_empty_trash_ancestors(&entry.entry_dir);
        deleted_session_ids.insert(entry.manifest.session_id.clone());
        deleted_entry_count += 1;
    }

    Ok((deleted_session_ids, deleted_entry_count, freed_size_bytes))
}

fn calculate_path_size(path: &Path) -> Result<u64, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("读取路径大小失败 ({}): {}", path.display(), error))?;
    let file_type = metadata.file_type();
    if file_type.is_file() || file_type.is_symlink() {
        return Ok(metadata.len());
    }
    if !file_type.is_dir() {
        return Ok(metadata.len());
    }

    let mut total = metadata.len();
    for entry in fs::read_dir(path)
        .map_err(|error| format!("读取目录大小失败 ({}): {}", path.display(), error))?
    {
        let entry =
            entry.map_err(|error| format!("读取目录项大小失败 ({}): {}", path.display(), error))?;
        total = total.saturating_add(calculate_path_size(&entry.path())?);
    }
    Ok(total)
}

fn remove_path_recursively(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "读取待删除路径失败 ({}): {}",
                path.display(),
                error
            ))
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_dir() && !file_type.is_symlink() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("删除目录失败 ({}): {}", path.display(), error))
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("删除文件失败 ({}): {}", path.display(), error))
    }
}

fn format_bytes(value: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;
    const GB: f64 = 1024.0 * MB;
    let value = value as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{} B", value as u64)
    }
}

fn cleanup_empty_trash_ancestors(entry_dir: &Path) {
    let mut current = entry_dir.parent();
    while let Some(dir) = current {
        if dir.file_name().and_then(|value| value.to_str()) == Some(SESSION_TRASH_ROOT_DIR) {
            break;
        }
        let is_empty = fs::read_dir(dir)
            .ok()
            .and_then(|mut iterator| iterator.next().transpose().ok())
            .flatten()
            .is_none();
        if !is_empty {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }

    fn write_rollout(path: &Path, session_id: &str, marker: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create rollout parent");
        }
        fs::write(
            path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"/tmp/project\",\"model_provider\":\"relay\"}}}}\n{{\"type\":\"event\",\"timestamp\":\"2026-06-02T01:02:03Z\",\"payload\":{{\"marker\":\"{}\"}}}}\n",
                session_id, marker
            ),
        )
        .expect("write rollout");
    }

    #[test]
    fn resolve_thread_snapshot_updated_at_prefers_rollout_activity_when_index_is_stale() {
        let base_dir = make_temp_dir("codex-session-updated-at-drift-test");
        let rollout_path = base_dir.join("rollout-session-1.jsonl");
        write_rollout(&rollout_path, "session-1", "activity");
        let stale_index = json!({
            "id": "session-1",
            "thread_name": "Old index",
            "updated_at": "2024-01-01T00:00:00.000000Z",
        });

        assert_eq!(
            resolve_thread_snapshot_updated_at_seconds(Some(&stale_index), &rollout_path),
            Some(1_780_362_123)
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn resolve_thread_snapshot_updated_at_keeps_index_when_close_to_rollout_activity() {
        let base_dir = make_temp_dir("codex-session-updated-at-index-test");
        let rollout_path = base_dir.join("rollout-session-1.jsonl");
        write_rollout(&rollout_path, "session-1", "activity");
        let current_index = json!({
            "id": "session-1",
            "thread_name": "Current index",
            "updated_at": "2026-06-02T01:30:00.000000Z",
        });

        assert_eq!(
            resolve_thread_snapshot_updated_at_seconds(Some(&current_index), &rollout_path),
            Some(1_780_363_800)
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn conversation_search_matches_raw_keyword_anywhere_in_rollout() {
        let base_dir = make_temp_dir("codex-session-keyword-search-test");
        let rollout_path = base_dir.join("rollout-keyword.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"output\":\"needle only appears in command output\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"中文关键字\"}}\n",
            ),
        )
        .expect("write rollout");

        assert!(
            rollout_conversation_contains_query_uncached(&rollout_path, "NEEDLE ONLY")
                .expect("search ascii keyword")
        );
        assert!(
            rollout_conversation_contains_query_uncached(&rollout_path, "中文关键字")
                .expect("search unicode keyword")
        );
        assert!(
            !rollout_conversation_contains_query_uncached(&rollout_path, "missing")
                .expect("search missing keyword")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn conversation_search_matches_keyword_across_read_chunks() {
        let base_dir = make_temp_dir("codex-session-keyword-chunk-test");
        let rollout_path = base_dir.join("rollout-keyword.jsonl");
        let mut content = vec![b'a'; CONTENT_SEARCH_READ_CHUNK_BYTES - 3];
        content.extend_from_slice(b"Sea");
        content.extend_from_slice(b"rchable");
        fs::write(&rollout_path, content).expect("write rollout");

        assert!(
            rollout_conversation_contains_query_uncached(&rollout_path, "searchable")
                .expect("search chunked keyword")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn load_trash_entries_skips_unreadable_optional_legacy_root() {
        let base_dir = make_temp_dir("codex-session-trash-roots-test");
        let primary_root = base_dir.join("primary-trash");
        let session_id = "session-1";
        let relative_rollout_path =
            PathBuf::from("sessions/2026/06/02").join(format!("rollout-{}.jsonl", session_id));
        let entry_dir = primary_root
            .join("20260613-000000")
            .join(format!("default--{}", session_id));
        let trashed_rollout_path = entry_dir.join("files").join(&relative_rollout_path);
        write_rollout(&trashed_rollout_path, session_id, "trashed");
        let instance_root = base_dir.join("codex-home");
        fs::write(
            entry_dir.join("manifest.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "sessionId": session_id,
                    "title": "Restored title",
                    "cwd": "/tmp/project",
                    "instanceId": DEFAULT_INSTANCE_ID,
                    "instanceName": DEFAULT_INSTANCE_NAME,
                    "instanceRoot": instance_root,
                    "originalRolloutPath": instance_root.join(&relative_rollout_path),
                    "relativeRolloutPath": relative_rollout_path.to_string_lossy(),
                    "sessionIndexEntry": {
                        "id": session_id,
                        "thread_name": "Restored title",
                    },
                    "deletedAt": "2026-06-13T00:00:00Z",
                }))
                .expect("serialize manifest")
            ),
        )
        .expect("write manifest");
        let legacy_root_file = base_dir.join("legacy-trash-file");
        fs::write(&legacy_root_file, "not a directory").expect("write legacy root file");

        let entries = load_trash_entries_from_roots(&[
            TrashRoot {
                path: primary_root,
                optional: false,
            },
            TrashRoot {
                path: legacy_root_file,
                optional: true,
            },
        ])
        .expect("load primary trash entries");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].manifest.session_id, session_id);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn move_to_trash_keeps_file_change_when_metadata_rebuild_fails() {
        let base_dir = make_temp_dir("codex-session-trash-rebuild-failure-test");
        let instance_root = base_dir.join("codex-home");
        let session_id = "session-1";
        let other_session_id = "session-2";
        let rollout_path = instance_root
            .join("sessions")
            .join("2026")
            .join("06")
            .join("02")
            .join(format!("rollout-{}.jsonl", session_id));
        write_rollout(&rollout_path, session_id, "active");
        fs::write(
            instance_root.join(SESSION_INDEX_FILE),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"Deleted title\"}}\n{{\"id\":\"{}\",\"thread_name\":\"Kept title\"}}\n",
                session_id, other_session_id
            ),
        )
        .expect("write session index");

        let instance = CodexSyncInstance {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: DEFAULT_INSTANCE_NAME.to_string(),
            data_dir: instance_root.clone(),
            last_pid: None,
        };
        let snapshot = ThreadSnapshot {
            id: session_id.to_string(),
            title: "Deleted title".to_string(),
            cwd: "/tmp/project".to_string(),
            updated_at: Some(1_780_362_123),
            rollout_path: rollout_path.clone(),
            session_index_entry: json!({
                "id": session_id,
                "thread_name": "Deleted title",
            }),
            source_root: instance_root.clone(),
        };
        let trash_root = base_dir
            .join(".Trash")
            .join(SESSION_TRASH_ROOT_DIR)
            .join("20260613-000000");

        let outcome =
            trash_snapshots_for_instance_with_rebuild(&instance, &trash_root, &[snapshot], |_| {
                Err("spawn denied".to_string())
            })
            .expect("trash should succeed with rebuild warning");

        assert!(outcome.metadata_rebuild_failed);
        assert!(!rollout_path.exists());
        assert!(trash_root
            .join(format!("{}--{}", DEFAULT_INSTANCE_ID, session_id))
            .join("files")
            .join("sessions")
            .join("2026")
            .join("06")
            .join("02")
            .join(format!("rollout-{}.jsonl", session_id))
            .exists());
        let index_map = read_session_index_map(&instance_root).expect("read index map");
        assert!(!index_map.contains_key(session_id));
        assert!(index_map.contains_key(other_session_id));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn import_package_path_validation_rejects_unsafe_entries() {
        assert_eq!(
            normalize_package_entry_path("files/0001-session/rollout.jsonl").as_deref(),
            Some("files/0001-session/rollout.jsonl")
        );
        assert!(normalize_package_entry_path("../rollout.jsonl").is_none());
        assert!(normalize_package_entry_path("/tmp/rollout.jsonl").is_none());
        assert!(normalize_package_entry_path("files/../rollout.jsonl").is_none());
        assert!(normalize_package_entry_path("C:/tmp/rollout.jsonl").is_none());
        assert!(is_safe_rollout_relative_path(
            "sessions/2026/06/02/rollout-session-1.jsonl"
        ));
        assert!(!is_safe_rollout_relative_path(
            "config/2026/06/02/rollout-session-1.jsonl"
        ));
    }

    #[test]
    fn imported_session_index_entry_preserves_existing_fields_and_sets_id() {
        let base_dir = make_temp_dir("codex-session-import-index-entry-test");
        let rollout_path = base_dir.join("rollout-session-1.jsonl");
        write_rollout(&rollout_path, "session-1", "imported");
        let item = SessionExportManifestItem {
            session_id: "session-1".to_string(),
            title: "Imported title".to_string(),
            cwd: "/tmp/project".to_string(),
            updated_at: Some(1_780_362_123),
            relative_rollout_path: "sessions/2026/06/02/rollout-session-1.jsonl".to_string(),
            file_entry: "files/0001-session-1/rollout.jsonl".to_string(),
            size_bytes: 10,
            sha256: "0".repeat(64),
            session_index_entry: json!({
                "thread_name": "Original package title",
                "pinned": true,
            }),
            source_instance: SessionExportInstance {
                id: DEFAULT_INSTANCE_ID.to_string(),
                name: DEFAULT_INSTANCE_NAME.to_string(),
            },
        };

        let entry = build_imported_session_index_entry(&item, &rollout_path);

        assert_eq!(
            entry.get("id").and_then(JsonValue::as_str),
            Some("session-1")
        );
        assert_eq!(
            entry.get("thread_name").and_then(JsonValue::as_str),
            Some("Original package title")
        );
        assert_eq!(entry.get("pinned").and_then(JsonValue::as_bool), Some(true));
        assert_eq!(
            parse_session_index_updated_at_seconds(&entry),
            Some(1_780_362_123)
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    fn make_trash_entry(
        base_dir: &Path,
        session_id: &str,
        target_rollout_path: PathBuf,
    ) -> TrashedSessionEntry {
        let entry_dir = base_dir
            .join(".Trash")
            .join(SESSION_TRASH_ROOT_DIR)
            .join("20260613-000000")
            .join(format!("default--{}", session_id));
        let relative_rollout_path =
            PathBuf::from("sessions/2026/06/02").join(format!("rollout-{}.jsonl", session_id));
        let trashed_rollout_path = entry_dir.join("files").join(&relative_rollout_path);
        write_rollout(&trashed_rollout_path, session_id, "trashed");
        TrashedSessionEntry {
            entry_dir,
            manifest: TrashedSessionManifest {
                session_id: session_id.to_string(),
                title: "Restored title".to_string(),
                cwd: "/tmp/project".to_string(),
                instance_id: DEFAULT_INSTANCE_ID.to_string(),
                instance_name: DEFAULT_INSTANCE_NAME.to_string(),
                instance_root: target_rollout_path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .and_then(Path::parent)
                    .unwrap()
                    .to_path_buf(),
                original_rollout_path: target_rollout_path,
                relative_rollout_path: relative_rollout_path.to_string_lossy().to_string(),
                session_index_entry: json!({
                    "id": session_id,
                    "thread_name": "Restored title",
                    "source": "trash",
                }),
                deleted_at: Some("2026-06-13T00:00:00Z".to_string()),
            },
            trashed_rollout_path,
        }
    }

    #[test]
    fn delete_trash_entries_removes_only_selected_entries() {
        let base_dir = make_temp_dir("codex-session-trash-delete-test");
        let instance_root = base_dir.join("codex-home");
        let first_entry = make_trash_entry(
            &base_dir,
            "session-1",
            instance_root
                .join("sessions")
                .join("2026")
                .join("06")
                .join("02")
                .join("rollout-session-1.jsonl"),
        );
        let second_entry = make_trash_entry(
            &base_dir,
            "session-2",
            instance_root
                .join("sessions")
                .join("2026")
                .join("06")
                .join("02")
                .join("rollout-session-2.jsonl"),
        );

        let (deleted_session_ids, deleted_entry_count, freed_size_bytes) =
            delete_trash_entries(std::slice::from_ref(&first_entry)).expect("delete trash entry");

        assert_eq!(deleted_entry_count, 1);
        assert!(deleted_session_ids.contains("session-1"));
        assert!(!deleted_session_ids.contains("session-2"));
        assert!(freed_size_bytes > 0);
        assert!(!first_entry.entry_dir.exists());
        assert!(second_entry.entry_dir.exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn restore_allows_existing_same_rollout_and_upserts_index() {
        let base_dir = make_temp_dir("codex-session-restore-idempotent-test");
        let instance_root = base_dir.join("codex-home");
        let session_id = "session-1";
        let target_rollout_path = instance_root
            .join("sessions")
            .join("2026")
            .join("06")
            .join("02")
            .join(format!("rollout-{}.jsonl", session_id));
        write_rollout(&target_rollout_path, session_id, "existing");
        let original_target_content =
            fs::read_to_string(&target_rollout_path).expect("read target rollout");
        fs::write(
            instance_root.join(SESSION_INDEX_FILE),
            format!(
                "{{\"id\":\"{}\",\"thread_name\":\"Old title\",\"updated_at\":\"2024-01-01T00:00:00.000000Z\",\"pinned\":true}}\n",
                session_id
            ),
        )
        .expect("write session index");
        let entry = make_trash_entry(&base_dir, session_id, target_rollout_path.clone());

        let outcome = restore_trashed_session_entry_with_metadata_rebuild(&entry, false)
            .expect("restore idempotently");

        assert!(!outcome.metadata_rebuild_failed);
        assert_eq!(
            fs::read_to_string(&target_rollout_path).expect("read target rollout after restore"),
            original_target_content
        );
        let index_map = read_session_index_map(&instance_root).expect("read index map");
        let restored = index_map.get(session_id).expect("restored index entry");
        assert_eq!(
            restored.get("thread_name").and_then(JsonValue::as_str),
            Some("Restored title")
        );
        assert_eq!(
            restored.get("pinned").and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(
            restored.get("source").and_then(JsonValue::as_str),
            Some("trash")
        );
        assert_eq!(
            parse_session_index_updated_at_seconds(restored),
            Some(1_780_362_123)
        );
        assert!(!entry.entry_dir.exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn restore_rejects_existing_different_rollout() {
        let base_dir = make_temp_dir("codex-session-restore-conflict-test");
        let instance_root = base_dir.join("codex-home");
        let session_id = "session-1";
        let target_rollout_path = instance_root
            .join("sessions")
            .join("2026")
            .join("06")
            .join("02")
            .join(format!("rollout-{}.jsonl", session_id));
        write_rollout(&target_rollout_path, "other-session", "existing");
        let entry = make_trash_entry(&base_dir, session_id, target_rollout_path.clone());

        let error = restore_trashed_session_entry_with_metadata_rebuild(&entry, false)
            .expect_err("different existing rollout should be rejected");

        assert!(error.contains("不同会话文件"));
        assert!(target_rollout_path.exists());
        assert!(entry.entry_dir.exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }
}
