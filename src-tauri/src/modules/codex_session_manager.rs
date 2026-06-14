use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const PROJECT_ORDER_KEY: &str = "project-order";
const PROJECTLESS_THREAD_IDS_KEY: &str = "projectless-thread-ids";
const SIDEBAR_PROJECT_THREAD_ORDERS_KEY: &str = "sidebar-project-thread-orders";
const THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY: &str = "thread-projectless-output-directories";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_TRASH_ROOT_DIR: &str = "cockpit-tools-codex-session-trash";
const ROLLOUT_RESTORE_BACKUP_ROOT_DIR: &str = "codex-session-restore-rollout-backups";
const TOKEN_STATS_READ_CHUNK_BYTES: usize = 64 * 1024;

static TOKEN_STATS_CACHE: LazyLock<Mutex<HashMap<PathBuf, TokenStatsCacheEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionLocation {
    pub instance_id: String,
    pub instance_name: String,
    pub running: bool,
    pub source_kind: String,
    pub projectless: bool,
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRecord {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
    pub archived: bool,
    pub has_archived_location: bool,
    pub projectless: bool,
    pub removed: bool,
    pub has_removed_location: bool,
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
    pub location_count: usize,
    pub locations: Vec<CodexTrashedSessionLocation>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRestoreSummary {
    pub requested_session_count: usize,
    pub restored_session_count: usize,
    pub restored_instance_count: usize,
    pub backup_batch_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRestoreConflict {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub instance_id: String,
    pub instance_name: String,
    pub target_rollout_path: String,
    pub trashed_rollout_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRestorePreviewSummary {
    pub requested_session_count: usize,
    pub restorable_session_count: usize,
    pub restorable_instance_count: usize,
    pub restorable_rollout_file_count: usize,
    pub conflict_count: usize,
    pub conflict_rollout_file_count: usize,
    pub source_repair_count: usize,
    pub conflicts: Vec<CodexSessionRestoreConflict>,
    pub source_repair_candidates:
        Vec<modules::codex_session_visibility::CodexSessionSourceRepairCandidate>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRolloutBackupItem {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub instance_id: String,
    pub instance_name: String,
    pub target_rollout_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionRolloutBackupBatch {
    pub batch_id: String,
    pub created_at: Option<i64>,
    pub backup_dir: String,
    pub session_count: usize,
    pub rollout_file_count: usize,
    pub instance_count: usize,
    pub items: Vec<CodexSessionRolloutBackupItem>,
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
    source_kind: String,
    projectless: bool,
    removed: bool,
}

#[derive(Debug, Clone, Default)]
struct CodexSidebarState {
    projectless_session_ids: HashSet<String>,
    projectless_workspace_roots: HashSet<String>,
    removed_session_ids: HashSet<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolloutRestoreBackupBatchManifest {
    batch_id: String,
    created_at: String,
    items: Vec<RolloutRestoreBackupItemManifest>,
    instances: Vec<RolloutRestoreBackupInstanceManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolloutRestoreBackupItemManifest {
    session_id: String,
    title: String,
    cwd: String,
    instance_id: String,
    instance_name: String,
    instance_root: PathBuf,
    original_rollout_path: PathBuf,
    relative_rollout_path: String,
    backup_rollout_relative_path: String,
    replacement_session_index_entry: JsonValue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RolloutRestoreBackupInstanceManifest {
    instance_id: String,
    instance_name: String,
    instance_root: PathBuf,
    session_index_existed: bool,
    session_index_backup_relative_path: Option<String>,
}

#[derive(Debug, Clone)]
struct TokenStatsCacheEntry {
    file_len: u64,
    modified_at: Option<SystemTime>,
    stats: Option<(u64, u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreTargetRolloutState {
    Missing,
    ExistingSameContent,
    ExistingDifferentContent,
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

pub fn list_sessions_across_instances() -> Result<Vec<CodexSessionRecord>, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut session_map = HashMap::<String, CodexSessionRecord>::new();

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        for snapshot in load_thread_snapshots(instance)? {
            let entry =
                session_map
                    .entry(snapshot.id.clone())
                    .or_insert_with(|| CodexSessionRecord {
                        session_id: snapshot.id.clone(),
                        title: snapshot.title.clone(),
                        cwd: snapshot.cwd.clone(),
                        updated_at: snapshot.updated_at,
                        archived: false,
                        has_archived_location: false,
                        projectless: false,
                        removed: false,
                        has_removed_location: false,
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
                source_kind: snapshot.source_kind.clone(),
                projectless: snapshot.projectless,
                removed: snapshot.removed,
            });
            entry.location_count = entry.locations.len();
            entry.has_archived_location = entry
                .locations
                .iter()
                .any(|location| location.source_kind == "archived_sessions");
            entry.projectless = entry.locations.iter().any(|location| location.projectless);
            entry.has_removed_location = entry.locations.iter().any(|location| location.removed);
            entry.archived = !entry.locations.is_empty()
                && entry
                    .locations
                    .iter()
                    .all(|location| location.source_kind == "archived_sessions");
            entry.removed = !entry.locations.is_empty()
                && entry.locations.iter().all(|location| location.removed);
        }
    }

    let mut sessions = session_map.into_values().collect::<Vec<_>>();
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

        trash_snapshots_for_instance(instance, &trash_root, &snapshots)?;
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

    let message = if mutated_running_instance_count > 0 {
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

pub fn preview_restore_sessions_from_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<CodexSessionRestorePreviewSummary, String> {
    let requested_ids = normalize_session_ids(session_ids)?;
    let entries = load_trash_entries()?
        .into_iter()
        .filter(|entry| requested_ids.contains(&entry.manifest.session_id))
        .collect::<Vec<_>>();
    preview_restore_entries(&entries, requested_ids.len())
}

pub fn restore_sessions_from_trash_across_instances(
    session_ids: Vec<String>,
    force_overwrite: bool,
    normalize_sources: bool,
) -> Result<CodexSessionRestoreSummary, String> {
    let requested_ids = normalize_session_ids(session_ids)?;
    let entries = load_trash_entries()?
        .into_iter()
        .filter(|entry| requested_ids.contains(&entry.manifest.session_id))
        .collect::<Vec<_>>();

    restore_trashed_session_entries_with_rebuild(
        &entries,
        requested_ids.len(),
        force_overwrite,
        normalize_sources,
        None,
        &|instance_root| modules::codex_official_app_server::rebuild_thread_metadata(instance_root),
    )
}

pub fn list_session_restore_rollout_backups() -> Result<Vec<CodexSessionRolloutBackupBatch>, String>
{
    let root = get_rollout_restore_backup_base_dir()?;
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut batches = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|error| format!("读取 rollout 备份目录失败 ({}): {}", root.display(), error))?
    {
        let entry = entry.map_err(|error| format!("读取 rollout 备份目录项失败: {}", error))?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| format!("读取 rollout 备份目录项类型失败: {}", error))?
            .is_dir()
        {
            continue;
        }
        let manifest = read_rollout_restore_backup_manifest(&path)?;
        let mut instance_ids = HashSet::new();
        let items = manifest
            .items
            .iter()
            .map(|item| {
                instance_ids.insert(item.instance_id.clone());
                CodexSessionRolloutBackupItem {
                    session_id: item.session_id.clone(),
                    title: item.title.clone(),
                    cwd: item.cwd.clone(),
                    instance_id: item.instance_id.clone(),
                    instance_name: item.instance_name.clone(),
                    target_rollout_path: item.original_rollout_path.to_string_lossy().to_string(),
                }
            })
            .collect::<Vec<_>>();
        let session_count = items
            .iter()
            .map(|item| item.session_id.as_str())
            .collect::<HashSet<_>>()
            .len();
        batches.push(CodexSessionRolloutBackupBatch {
            batch_id: manifest.batch_id,
            created_at: parse_deleted_at(Some(&manifest.created_at)),
            backup_dir: path.to_string_lossy().to_string(),
            session_count,
            rollout_file_count: items.len(),
            instance_count: instance_ids.len(),
            items,
        });
    }
    batches.sort_by(|left, right| {
        right
            .created_at
            .unwrap_or_default()
            .cmp(&left.created_at.unwrap_or_default())
            .then_with(|| right.batch_id.cmp(&left.batch_id))
    });
    Ok(batches)
}

pub fn restore_session_restore_rollout_backup(
    batch_id: String,
) -> Result<CodexSessionRestoreSummary, String> {
    let batch_id = batch_id.trim();
    if batch_id.is_empty() {
        return Err("请选择一个 rollout 备份批次".to_string());
    }
    let batch_dir = get_rollout_restore_backup_base_dir()?.join(batch_id);
    restore_rollout_backup_batch_with_rebuild(&batch_dir, None, &|instance_root| {
        modules::codex_official_app_server::rebuild_thread_metadata(instance_root)
    })
}

fn normalize_session_ids(session_ids: Vec<String>) -> Result<HashSet<String>, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }
    Ok(requested_ids)
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
    let sidebar_state = read_sidebar_state(&instance.data_dir);
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
            let normalized_cwd = normalize_workspace_root(&cwd);
            let projectless = sidebar_state.projectless_session_ids.contains(&id)
                || normalized_cwd
                    .as_ref()
                    .is_some_and(|cwd| sidebar_state.projectless_workspace_roots.contains(cwd));
            let removed = !projectless && sidebar_state.removed_session_ids.contains(&id);
            let updated_at = session_index_map
                .get(&id)
                .and_then(parse_session_index_updated_at_seconds)
                .or_else(|| rollout_file_activity_seconds(&rollout_path))
                .or_else(|| rollout_file_modified_seconds(&rollout_path));
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
                source_kind: dir_name.to_string(),
                projectless,
                removed,
            });
        }
    }

    Ok(snapshots)
}

fn normalize_workspace_root(value: &str) -> Option<String> {
    let mut value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(stripped) = value.strip_prefix("\\\\?\\") {
        value = stripped;
    }

    let is_windows_path = value.starts_with("\\\\")
        || value
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':');
    let separator = if is_windows_path { '\\' } else { '/' };
    let mut normalized = if is_windows_path {
        value.replace('/', "\\")
    } else {
        value.replace('\\', "/")
    };
    while normalized.len() > 3 && normalized.ends_with(separator) {
        normalized.pop();
    }
    if normalized.trim().is_empty() {
        return None;
    }
    if is_windows_path
        && normalized
            .as_bytes()
            .get(1)
            .is_some_and(|separator| *separator == b':')
    {
        let drive = normalized[..1].to_ascii_uppercase();
        normalized.replace_range(0..1, &drive);
    }
    Some(normalized)
}

fn json_string_array(value: &JsonValue, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn projectless_output_parent_root(output_dir: &str) -> Option<String> {
    let output_path = Path::new(output_dir);
    let parent = match output_path.file_name().and_then(|name| name.to_str()) {
        Some(name) if name.eq_ignore_ascii_case("outputs") => output_path.parent(),
        _ => Some(output_path),
    }?;
    normalize_workspace_root(&parent.to_string_lossy())
}

fn read_sidebar_state(data_dir: &Path) -> CodexSidebarState {
    let path = data_dir.join(GLOBAL_STATE_FILE);
    let Some(current) = read_global_state_file(&path) else {
        return CodexSidebarState::default();
    };
    let history = read_global_state_history(data_dir, &current);
    let value = select_sidebar_semantic_state(&history).unwrap_or(&current);
    sidebar_state_from_global_state(&value)
}

fn read_global_state_file(path: &Path) -> Option<JsonValue> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<JsonValue>(&content).ok()
}

fn read_global_state_history(data_dir: &Path, current: &JsonValue) -> Vec<JsonValue> {
    let mut candidates = Vec::<(SystemTime, JsonValue)>::new();
    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(_) => return vec![current.clone()],
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        for state_path in [
            path.join(GLOBAL_STATE_FILE),
            path.join("files").join(GLOBAL_STATE_FILE),
        ] {
            let Some(value) = read_global_state_file(&state_path) else {
                continue;
            };
            let modified = fs::metadata(&state_path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            candidates.push((modified, value));
        }
    }

    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    let mut values = candidates
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    values.push(current.clone());
    values
}

fn sidebar_semantic_score(value: &JsonValue) -> usize {
    [
        PROJECTLESS_THREAD_IDS_KEY,
        SIDEBAR_PROJECT_THREAD_ORDERS_KEY,
        THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY,
    ]
    .iter()
    .filter(|key| value.get(**key).is_some())
    .count()
}

fn projectless_project_order_pollution_count(value: &JsonValue) -> usize {
    let projectless_roots = value
        .get(THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY)
        .and_then(JsonValue::as_object)
        .map(|object| {
            object
                .values()
                .filter_map(JsonValue::as_str)
                .filter_map(projectless_output_parent_root)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    if projectless_roots.is_empty() {
        return 0;
    }
    json_string_array(value, PROJECT_ORDER_KEY)
        .iter()
        .filter_map(|root| normalize_workspace_root(root))
        .filter(|root| projectless_roots.contains(root))
        .count()
}

fn select_sidebar_semantic_state(history: &[JsonValue]) -> Option<&JsonValue> {
    history
        .iter()
        .filter(|value| sidebar_semantic_score(value) > 0)
        .max_by(|left, right| {
            let left_pollution = projectless_project_order_pollution_count(left);
            let right_pollution = projectless_project_order_pollution_count(right);
            right_pollution
                .cmp(&left_pollution)
                .then_with(|| sidebar_semantic_score(left).cmp(&sidebar_semantic_score(right)))
        })
}

fn sidebar_state_from_global_state(value: &JsonValue) -> CodexSidebarState {
    let projectless_session_ids = json_string_array(value, PROJECTLESS_THREAD_IDS_KEY)
        .into_iter()
        .collect::<HashSet<_>>();
    let projectless_workspace_roots = value
        .get(THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY)
        .and_then(JsonValue::as_object)
        .map(|object| {
            object
                .values()
                .filter_map(JsonValue::as_str)
                .filter_map(projectless_output_parent_root)
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let visible_roots = json_string_array(value, PROJECT_ORDER_KEY)
        .iter()
        .filter_map(|root| normalize_workspace_root(root))
        .collect::<HashSet<_>>();
    let mut removed_session_ids = HashSet::new();
    if let Some(project_orders) = value
        .get(SIDEBAR_PROJECT_THREAD_ORDERS_KEY)
        .and_then(JsonValue::as_object)
    {
        for (root, session_ids) in project_orders {
            let Some(root) = normalize_workspace_root(root) else {
                continue;
            };
            if visible_roots.contains(&root) {
                continue;
            }
            if let Some(ids) = session_ids.as_array() {
                removed_session_ids
                    .extend(ids.iter().filter_map(JsonValue::as_str).map(str::to_string));
            }
        }
    }

    CodexSidebarState {
        projectless_session_ids,
        projectless_workspace_roots,
        removed_session_ids,
    }
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

fn session_meta_source(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("source"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_active_session_relative_path(relative_path: &str) -> bool {
    let normalized = relative_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    normalized == "sessions" || normalized.starts_with("sessions/")
}

fn build_source_repair_candidate_from_trash_entry(
    entry: &TrashedSessionEntry,
) -> Result<Option<modules::codex_session_visibility::CodexSessionSourceRepairCandidate>, String> {
    let Some(session_meta) = read_rollout_session_meta(&entry.trashed_rollout_path)? else {
        return Ok(None);
    };
    let current_source = session_meta_source(&session_meta).unwrap_or_default();
    if current_source == "cli" {
        return Ok(None);
    }
    Ok(Some(
        modules::codex_session_visibility::CodexSessionSourceRepairCandidate {
            session_id: entry.manifest.session_id.clone(),
            title: entry.manifest.title.clone(),
            cwd: entry.manifest.cwd.clone(),
            instance_id: entry.manifest.instance_id.clone(),
            instance_name: entry.manifest.instance_name.clone(),
            current_source,
            target_source: "cli".to_string(),
            rollout_path: entry.trashed_rollout_path.to_string_lossy().to_string(),
        },
    ))
}

fn trash_snapshots_for_instance(
    instance: &CodexSyncInstance,
    trash_root: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<(), String> {
    for snapshot in snapshots {
        move_snapshot_rollout_to_trash(instance, trash_root, snapshot)?;
    }

    rewrite_session_index_without_ids(&instance.data_dir, snapshots)?;
    modules::codex_official_app_server::rebuild_thread_metadata(&instance.data_dir).map_err(
        |error| {
            format!(
                "会话文件已移到废纸篓，但官方 Codex 重建会话索引失败 ({}): {}",
                instance.name, error
            )
        },
    )?;
    Ok(())
}

fn create_trash_root_dir() -> Result<PathBuf, String> {
    let root = get_session_trash_base_dir()?.join(Utc::now().format("%Y%m%d-%H%M%S").to_string());
    fs::create_dir_all(&root)
        .map_err(|error| format!("创建会话废纸篓目录失败 ({}): {}", root.display(), error))?;
    Ok(root)
}

fn get_session_trash_base_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    Ok(home.join(".Trash").join(SESSION_TRASH_ROOT_DIR))
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

fn rollout_file_activity_seconds(path: &Path) -> Option<i64> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<JsonValue>(line.trim()).ok())
        .filter_map(|value| parse_rollout_line_timestamp_seconds(&value))
        .max()
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
    let root = get_session_trash_base_dir()?;
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

    entries.sort_by(|left, right| {
        parse_deleted_at(right.manifest.deleted_at.as_deref())
            .unwrap_or_default()
            .cmp(&parse_deleted_at(left.manifest.deleted_at.as_deref()).unwrap_or_default())
            .then_with(|| left.manifest.session_id.cmp(&right.manifest.session_id))
            .then_with(|| left.manifest.instance_id.cmp(&right.manifest.instance_id))
    });
    Ok(entries)
}

fn preview_restore_entries(
    entries: &[TrashedSessionEntry],
    requested_session_count: usize,
) -> Result<CodexSessionRestorePreviewSummary, String> {
    let mut restorable_session_ids = HashSet::new();
    let mut restorable_instance_ids = HashSet::new();
    let mut conflicts = Vec::new();
    let mut source_repair_candidates = Vec::new();

    for entry in entries {
        if !entry.trashed_rollout_path.exists() {
            continue;
        }
        let target_state = classify_restore_target_rollout(
            &entry.trashed_rollout_path,
            &entry.manifest.original_rollout_path,
        )?;
        if target_state == RestoreTargetRolloutState::ExistingDifferentContent {
            conflicts.push(CodexSessionRestoreConflict {
                session_id: entry.manifest.session_id.clone(),
                title: entry.manifest.title.clone(),
                cwd: entry.manifest.cwd.clone(),
                instance_id: entry.manifest.instance_id.clone(),
                instance_name: entry.manifest.instance_name.clone(),
                target_rollout_path: entry
                    .manifest
                    .original_rollout_path
                    .to_string_lossy()
                    .to_string(),
                trashed_rollout_path: entry.trashed_rollout_path.to_string_lossy().to_string(),
            });
        }
        if is_active_session_relative_path(&entry.manifest.relative_rollout_path) {
            if let Some(candidate) = build_source_repair_candidate_from_trash_entry(entry)? {
                source_repair_candidates.push(candidate);
            }
        }
        restorable_session_ids.insert(entry.manifest.session_id.clone());
        restorable_instance_ids.insert(entry.manifest.instance_id.clone());
    }
    source_repair_candidates.sort_by(|left, right| {
        left.instance_name
            .cmp(&right.instance_name)
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });

    let message = if entries.is_empty() {
        "所选会话在废纸篓中不存在，无需恢复".to_string()
    } else if !source_repair_candidates.is_empty() && conflicts.is_empty() {
        format!(
            "预检通过，可恢复 {} 条会话；其中 {} 个 rollout 需要确认后恢复 Codex 左侧项目可见来源",
            restorable_session_ids.len(),
            source_repair_candidates.len()
        )
    } else if conflicts.is_empty() {
        format!("预检通过，可恢复 {} 条会话", restorable_session_ids.len())
    } else {
        format!(
            "发现 {} 个同名不同内容 rollout 冲突，需要二次确认后备份覆盖",
            conflicts.len()
        )
    };

    Ok(CodexSessionRestorePreviewSummary {
        requested_session_count,
        restorable_session_count: restorable_session_ids.len(),
        restorable_instance_count: restorable_instance_ids.len(),
        restorable_rollout_file_count: entries
            .iter()
            .filter(|entry| entry.trashed_rollout_path.exists())
            .count(),
        conflict_count: conflicts.len(),
        conflict_rollout_file_count: conflicts.len(),
        source_repair_count: source_repair_candidates.len(),
        conflicts,
        source_repair_candidates,
        message,
    })
}

fn restore_trashed_session_entries_with_rebuild<F>(
    entries: &[TrashedSessionEntry],
    requested_session_count: usize,
    force_overwrite: bool,
    normalize_sources: bool,
    backup_root_override: Option<&Path>,
    rebuild: &F,
) -> Result<CodexSessionRestoreSummary, String>
where
    F: Fn(&Path) -> Result<(), String>,
{
    if entries.is_empty() {
        return Ok(CodexSessionRestoreSummary {
            requested_session_count,
            restored_session_count: 0,
            restored_instance_count: 0,
            backup_batch_id: None,
            message: "所选会话在废纸篓中不存在，无需恢复".to_string(),
        });
    }

    let plans = build_restore_plans(entries, force_overwrite)?;
    let conflicts = plans
        .iter()
        .filter(|plan| plan.target_state == RestoreTargetRolloutState::ExistingDifferentContent)
        .count();
    if conflicts > 0 && !force_overwrite {
        return Err(format!(
            "目标实例中存在 {} 个同名会话文件，且内容与废纸篓中的会话文件不同，无法自动恢复。请二次确认后使用强制覆盖恢复。",
            conflicts
        ));
    }

    let backup_root = match backup_root_override {
        Some(path) => path.to_path_buf(),
        None => get_rollout_restore_backup_base_dir()?,
    };
    let batch = if force_overwrite && conflicts > 0 {
        let conflict_plans = plans
            .iter()
            .filter(|plan| plan.target_state == RestoreTargetRolloutState::ExistingDifferentContent)
            .collect::<Vec<_>>();
        let batch = create_rollout_restore_backup_batch(&backup_root, &conflict_plans)?;
        finalize_rollout_restore_backup_batch(&batch)?;
        Some(batch)
    } else {
        None
    };

    let original_session_index_by_root = collect_original_session_index_by_root(&plans)?;
    let original_rollout_content_by_path = collect_original_rollout_content_by_path(&plans)?;
    let mut copied_missing_paths = Vec::<PathBuf>::new();
    let mut restored_session_ids = HashSet::<String>::new();
    let mut restored_instance_ids = HashSet::<String>::new();
    let restore_result = (|| {
        for plan in &plans {
            copy_trash_rollout_to_target(plan)?;
            if plan.target_state == RestoreTargetRolloutState::Missing {
                copied_missing_paths.push(plan.target_rollout_path.clone());
            }
            restored_session_ids.insert(plan.entry.manifest.session_id.clone());
            restored_instance_ids.insert(plan.entry.manifest.instance_id.clone());
        }

        for (instance_root, original_content) in &original_session_index_by_root {
            let instance_plans = plans
                .iter()
                .filter(|plan| &plan.entry.manifest.instance_root == instance_root)
                .collect::<Vec<_>>();
            write_session_index_with_entries(
                instance_root,
                original_content,
                &instance_plans
                    .iter()
                    .map(|plan| {
                        (
                            plan.entry.manifest.session_id.as_str(),
                            &plan.entry.manifest.session_index_entry,
                        )
                    })
                    .collect::<Vec<_>>(),
            )?;
        }

        for instance_root in unique_instance_roots_from_restore_plans(&plans) {
            if normalize_sources {
                let source_paths = plans
                    .iter()
                    .filter(|plan| plan.entry.manifest.instance_root == instance_root)
                    .filter(|plan| {
                        is_active_session_relative_path(&plan.entry.manifest.relative_rollout_path)
                    })
                    .map(|plan| plan.target_rollout_path.clone())
                    .collect::<Vec<_>>();
                modules::codex_session_visibility::normalize_session_sources_for_rollout_paths(
                    &instance_root,
                    &source_paths,
                )?;
            }
            let instance_name = plans
                .iter()
                .find(|plan| plan.entry.manifest.instance_root == instance_root)
                .map(|plan| plan.entry.manifest.instance_name.as_str())
                .unwrap_or("未知实例");
            rebuild(&instance_root).map_err(|error| {
                format!(
                    "会话文件已就位，但官方 Codex 重建会话索引失败 ({}): {}",
                    instance_name, error
                )
            })?;
        }
        Ok::<(), String>(())
    })();

    if let Err(error) = restore_result {
        rollback_restore_plans(
            &plans,
            &copied_missing_paths,
            &original_rollout_content_by_path,
            &original_session_index_by_root,
        );
        if let Some(batch) = &batch {
            let _ = fs::remove_dir_all(&batch.batch_dir);
        }
        return Err(format!(
            "{}；已回滚本次恢复，废纸篓中的原始会话文件仍保留。",
            error
        ));
    }

    for plan in &plans {
        if let Err(error) = fs::remove_dir_all(&plan.entry.entry_dir) {
            modules::logger::log_warn(&format!(
                "会话已恢复，但清理废纸篓条目失败 ({}): {}",
                plan.entry.entry_dir.display(),
                error
            ));
        } else {
            cleanup_empty_trash_ancestors(&plan.entry.entry_dir);
        }
    }

    Ok(CodexSessionRestoreSummary {
        requested_session_count,
        restored_session_count: restored_session_ids.len(),
        restored_instance_count: restored_instance_ids.len(),
        backup_batch_id: batch.map(|batch| batch.manifest.batch_id),
        message: format!(
            "已恢复 {} 条会话，并已触发官方 Codex 重建会话索引",
            restored_session_ids.len()
        ),
    })
}

#[derive(Debug)]
struct RestorePlan<'a> {
    entry: &'a TrashedSessionEntry,
    target_rollout_path: PathBuf,
    target_state: RestoreTargetRolloutState,
}

#[derive(Debug)]
struct RolloutRestoreBackupBatchWork {
    batch_dir: PathBuf,
    manifest: RolloutRestoreBackupBatchManifest,
}

#[derive(Debug)]
struct OriginalRolloutState {
    content: Vec<u8>,
    modified_at: Option<SystemTime>,
}

fn build_restore_plans<'a>(
    entries: &'a [TrashedSessionEntry],
    _force_overwrite: bool,
) -> Result<Vec<RestorePlan<'a>>, String> {
    let mut plans = Vec::new();
    for entry in entries {
        if !entry.trashed_rollout_path.exists() {
            return Err(format!(
                "废纸篓中的会话文件不存在，无法恢复 ({}): {}",
                entry.manifest.session_id,
                entry.trashed_rollout_path.display()
            ));
        }
        let target_rollout_path = entry.manifest.original_rollout_path.clone();
        let target_state =
            classify_restore_target_rollout(&entry.trashed_rollout_path, &target_rollout_path)?;
        plans.push(RestorePlan {
            entry,
            target_rollout_path,
            target_state,
        });
    }
    Ok(plans)
}

fn collect_original_session_index_by_root(
    plans: &[RestorePlan<'_>],
) -> Result<HashMap<PathBuf, Option<String>>, String> {
    let mut values = HashMap::new();
    for plan in plans {
        if !values.contains_key(&plan.entry.manifest.instance_root) {
            values.insert(
                plan.entry.manifest.instance_root.clone(),
                read_session_index_content(&plan.entry.manifest.instance_root)?,
            );
        }
    }
    Ok(values)
}

fn collect_original_rollout_content_by_path(
    plans: &[RestorePlan<'_>],
) -> Result<HashMap<PathBuf, OriginalRolloutState>, String> {
    let mut values = HashMap::new();
    for plan in plans {
        if plan.target_state == RestoreTargetRolloutState::Missing {
            continue;
        }
        if !values.contains_key(&plan.target_rollout_path) {
            values.insert(
                plan.target_rollout_path.clone(),
                OriginalRolloutState {
                    content: fs::read(&plan.target_rollout_path).map_err(|error| {
                        format!(
                            "读取待覆盖 rollout 文件失败 ({}): {}",
                            plan.target_rollout_path.display(),
                            error
                        )
                    })?,
                    modified_at: modules::codex_session_file_time::read_modified_time(
                        &plan.target_rollout_path,
                    ),
                },
            );
        }
    }
    Ok(values)
}

fn copy_trash_rollout_to_target(plan: &RestorePlan<'_>) -> Result<(), String> {
    if plan.target_state == RestoreTargetRolloutState::ExistingSameContent {
        return Ok(());
    }
    if let Some(parent) = plan.target_rollout_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建会话恢复目录失败 ({}): {}", parent.display(), error))?;
    }
    fs::copy(&plan.entry.trashed_rollout_path, &plan.target_rollout_path).map_err(|error| {
        format!(
            "恢复会话文件失败 ({} -> {}): {}",
            plan.entry.trashed_rollout_path.display(),
            plan.target_rollout_path.display(),
            error
        )
    })?;
    modules::codex_session_file_time::restore_modified_time(
        &plan.target_rollout_path,
        modules::codex_session_file_time::read_modified_time(&plan.entry.trashed_rollout_path),
    )?;
    Ok(())
}

fn rollback_restore_plans(
    plans: &[RestorePlan<'_>],
    copied_missing_paths: &[PathBuf],
    original_rollout_content_by_path: &HashMap<PathBuf, OriginalRolloutState>,
    original_session_index_by_root: &HashMap<PathBuf, Option<String>>,
) {
    for path in copied_missing_paths {
        if let Err(error) = fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                modules::logger::log_warn(&format!(
                    "回滚恢复时删除新增 rollout 失败 ({}): {}",
                    path.display(),
                    error
                ));
            }
        }
    }
    for (path, state) in original_rollout_content_by_path {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::write(path, &state.content) {
            modules::logger::log_warn(&format!(
                "回滚恢复时还原 rollout 失败 ({}): {}",
                path.display(),
                error
            ));
            continue;
        }
        if let Err(error) =
            modules::codex_session_file_time::restore_modified_time(path, state.modified_at)
        {
            modules::logger::log_warn(&format!(
                "回滚恢复时还原 rollout 修改时间失败 ({}): {}",
                path.display(),
                error
            ));
        }
    }
    for (root, content) in original_session_index_by_root {
        if let Err(error) = restore_session_index_content(root, content.as_deref()) {
            modules::logger::log_warn(&format!("回滚恢复时还原 session_index 失败: {}", error));
        }
    }
    let _ = plans;
}

fn unique_instance_roots_from_restore_plans(plans: &[RestorePlan<'_>]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for plan in plans {
        if seen.insert(plan.entry.manifest.instance_root.clone()) {
            roots.push(plan.entry.manifest.instance_root.clone());
        }
    }
    roots
}

fn write_session_index_with_entries(
    root_dir: &Path,
    original_content: &Option<String>,
    entries: &[(&str, &JsonValue)],
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    let ids = entries.iter().map(|(id, _)| *id).collect::<HashSet<_>>();
    let mut lines = original_content
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| match serde_json::from_str::<JsonValue>(line) {
            Ok(value) => value
                .get("id")
                .and_then(JsonValue::as_str)
                .map(|id| !ids.contains(id))
                .unwrap_or(true),
            Err(_) => true,
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    for (session_id, entry) in entries {
        lines.push(serde_json::to_string(entry).map_err(|error| {
            format!("序列化 session_index 条目失败 ({}): {}", session_id, error)
        })?);
    }

    let next_content = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
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

fn get_rollout_restore_backup_base_dir() -> Result<PathBuf, String> {
    if let Some(data_dir) = dirs::data_local_dir() {
        return Ok(data_dir
            .join("cockpit-tools")
            .join(ROLLOUT_RESTORE_BACKUP_ROOT_DIR));
    }
    let home = dirs::home_dir().ok_or("无法获取用户数据目录")?;
    Ok(home
        .join(".cockpit-tools")
        .join(ROLLOUT_RESTORE_BACKUP_ROOT_DIR))
}

fn create_rollout_restore_backup_batch(
    backup_root: &Path,
    plans: &[&RestorePlan<'_>],
) -> Result<RolloutRestoreBackupBatchWork, String> {
    let batch_id = format!(
        "{}-{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        std::process::id()
    );
    let batch_dir = backup_root.join(&batch_id);
    fs::create_dir_all(&batch_dir).map_err(|error| {
        format!(
            "创建 rollout 恢复备份目录失败 ({}): {}",
            batch_dir.display(),
            error
        )
    })?;

    let mut items = Vec::new();
    let mut instances = Vec::new();
    let mut seen_instances = HashSet::new();

    for plan in plans {
        let relative_rollout_path = normalize_relative_rollout_path(plan.entry);
        let backup_rollout_relative_path = PathBuf::from("files")
            .join(sanitize_for_file_name(&plan.entry.manifest.instance_id))
            .join(PathBuf::from(&relative_rollout_path));
        let backup_rollout_path = batch_dir.join(&backup_rollout_relative_path);
        if let Some(parent) = backup_rollout_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 备份目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&plan.target_rollout_path, &backup_rollout_path).map_err(|error| {
            format!(
                "备份冲突 rollout 失败 ({} -> {}): {}",
                plan.target_rollout_path.display(),
                backup_rollout_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &backup_rollout_path,
            modules::codex_session_file_time::read_modified_time(&plan.target_rollout_path),
        )?;

        if seen_instances.insert(plan.entry.manifest.instance_root.clone()) {
            let original_session_index =
                read_session_index_content(&plan.entry.manifest.instance_root)?;
            let session_index_backup_relative_path = if let Some(content) = original_session_index {
                let relative = PathBuf::from("session-index").join(format!(
                    "{}.jsonl",
                    sanitize_for_file_name(&plan.entry.manifest.instance_id)
                ));
                let target = batch_dir.join(&relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!(
                            "创建 session_index 备份目录失败 ({}): {}",
                            parent.display(),
                            error
                        )
                    })?;
                }
                modules::atomic_write::write_string_atomic(&target, &content).map_err(|error| {
                    format!(
                        "备份 session_index.jsonl 失败 ({}): {}",
                        target.display(),
                        error
                    )
                })?;
                Some(relative.to_string_lossy().to_string())
            } else {
                None
            };
            instances.push(RolloutRestoreBackupInstanceManifest {
                instance_id: plan.entry.manifest.instance_id.clone(),
                instance_name: plan.entry.manifest.instance_name.clone(),
                instance_root: plan.entry.manifest.instance_root.clone(),
                session_index_existed: session_index_backup_relative_path.is_some(),
                session_index_backup_relative_path,
            });
        }

        items.push(RolloutRestoreBackupItemManifest {
            session_id: plan.entry.manifest.session_id.clone(),
            title: plan.entry.manifest.title.clone(),
            cwd: plan.entry.manifest.cwd.clone(),
            instance_id: plan.entry.manifest.instance_id.clone(),
            instance_name: plan.entry.manifest.instance_name.clone(),
            instance_root: plan.entry.manifest.instance_root.clone(),
            original_rollout_path: plan.target_rollout_path.clone(),
            relative_rollout_path,
            backup_rollout_relative_path: backup_rollout_relative_path
                .to_string_lossy()
                .to_string(),
            replacement_session_index_entry: plan.entry.manifest.session_index_entry.clone(),
        });
    }

    Ok(RolloutRestoreBackupBatchWork {
        batch_dir,
        manifest: RolloutRestoreBackupBatchManifest {
            batch_id,
            created_at: Utc::now().to_rfc3339(),
            items,
            instances,
        },
    })
}

fn finalize_rollout_restore_backup_batch(
    batch: &RolloutRestoreBackupBatchWork,
) -> Result<(), String> {
    let manifest_path = batch.batch_dir.join("manifest.json");
    let manifest_content = format!(
        "{}\n",
        serde_json::to_string_pretty(&batch.manifest)
            .map_err(|error| format!("序列化 rollout 恢复备份清单失败: {}", error))?
    );
    modules::atomic_write::write_string_atomic(&manifest_path, &manifest_content).map_err(|error| {
        format!(
            "写入 rollout 恢复备份清单失败 ({}): {}",
            manifest_path.display(),
            error
        )
    })
}

fn read_rollout_restore_backup_manifest(
    batch_dir: &Path,
) -> Result<RolloutRestoreBackupBatchManifest, String> {
    let manifest_path = batch_dir.join("manifest.json");
    let content = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "读取 rollout 恢复备份清单失败 ({}): {}",
            manifest_path.display(),
            error
        )
    })?;
    serde_json::from_str::<RolloutRestoreBackupBatchManifest>(&content).map_err(|error| {
        format!(
            "解析 rollout 恢复备份清单失败 ({}): {}",
            manifest_path.display(),
            error
        )
    })
}

fn normalize_relative_rollout_path(entry: &TrashedSessionEntry) -> String {
    if !entry.manifest.relative_rollout_path.trim().is_empty() {
        return entry.manifest.relative_rollout_path.clone();
    }
    entry
        .manifest
        .original_rollout_path
        .strip_prefix(&entry.manifest.instance_root)
        .unwrap_or(&entry.manifest.original_rollout_path)
        .to_string_lossy()
        .to_string()
}

fn restore_rollout_backup_batch_with_rebuild<F>(
    batch_dir: &Path,
    trash_root_override: Option<&Path>,
    rebuild: &F,
) -> Result<CodexSessionRestoreSummary, String>
where
    F: Fn(&Path) -> Result<(), String>,
{
    let manifest = read_rollout_restore_backup_manifest(batch_dir)?;
    let trash_root = match trash_root_override {
        Some(path) => {
            fs::create_dir_all(path).map_err(|error| {
                format!("创建会话废纸篓目录失败 ({}): {}", path.display(), error)
            })?;
            path.to_path_buf()
        }
        None => create_trash_root_dir()?,
    };

    for item in &manifest.items {
        if item.original_rollout_path.exists() {
            move_snapshot_rollout_to_trash(
                &CodexSyncInstance {
                    id: item.instance_id.clone(),
                    name: item.instance_name.clone(),
                    data_dir: item.instance_root.clone(),
                    last_pid: None,
                },
                &trash_root,
                &ThreadSnapshot {
                    id: item.session_id.clone(),
                    title: item.title.clone(),
                    cwd: item.cwd.clone(),
                    updated_at: None,
                    rollout_path: item.original_rollout_path.clone(),
                    session_index_entry: item.replacement_session_index_entry.clone(),
                    source_root: item.instance_root.clone(),
                    source_kind: "sessions".to_string(),
                    projectless: false,
                    removed: false,
                },
            )?;
        }
        let backup_rollout_path = batch_dir.join(&item.backup_rollout_relative_path);
        if let Some(parent) = item.original_rollout_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 备份恢复目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&backup_rollout_path, &item.original_rollout_path).map_err(|error| {
            format!(
                "恢复 rollout 备份失败 ({} -> {}): {}",
                backup_rollout_path.display(),
                item.original_rollout_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &item.original_rollout_path,
            modules::codex_session_file_time::read_modified_time(&backup_rollout_path),
        )?;
    }

    for instance in &manifest.instances {
        if instance.session_index_existed {
            let Some(relative_path) = instance.session_index_backup_relative_path.as_deref() else {
                return Err(format!(
                    "rollout 备份清单缺少 session_index 备份路径 ({})",
                    instance.instance_name
                ));
            };
            let content = fs::read_to_string(batch_dir.join(relative_path)).map_err(|error| {
                format!(
                    "读取 session_index 备份失败 ({}): {}",
                    batch_dir.join(relative_path).display(),
                    error
                )
            })?;
            restore_session_index_content(&instance.instance_root, Some(&content))?;
        } else {
            restore_session_index_content(&instance.instance_root, None)?;
        }
    }

    for instance_root in unique_instance_roots_from_backup_manifest(&manifest) {
        let instance_name = manifest
            .instances
            .iter()
            .find(|item| item.instance_root == instance_root)
            .map(|item| item.instance_name.as_str())
            .unwrap_or("未知实例");
        rebuild(&instance_root).map_err(|error| {
            format!(
                "rollout 备份已恢复，但官方 Codex 重建会话索引失败 ({}): {}",
                instance_name, error
            )
        })?;
    }

    let session_ids = manifest
        .items
        .iter()
        .map(|item| item.session_id.clone())
        .collect::<HashSet<_>>();
    let instance_ids = manifest
        .items
        .iter()
        .map(|item| item.instance_id.clone())
        .collect::<HashSet<_>>();
    Ok(CodexSessionRestoreSummary {
        requested_session_count: session_ids.len(),
        restored_session_count: session_ids.len(),
        restored_instance_count: instance_ids.len(),
        backup_batch_id: Some(manifest.batch_id),
        message: format!(
            "已恢复 {} 条 rollout 备份，并已触发官方 Codex 重建会话索引",
            session_ids.len()
        ),
    })
}

fn unique_instance_roots_from_backup_manifest(
    manifest: &RolloutRestoreBackupBatchManifest,
) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for instance in &manifest.instances {
        if seen.insert(instance.instance_root.clone()) {
            roots.push(instance.instance_root.clone());
        }
    }
    roots
}

fn classify_restore_target_rollout(
    trashed_rollout_path: &Path,
    target_rollout_path: &Path,
) -> Result<RestoreTargetRolloutState, String> {
    if !target_rollout_path.exists() {
        return Ok(RestoreTargetRolloutState::Missing);
    }

    if rollout_files_match(trashed_rollout_path, target_rollout_path)? {
        Ok(RestoreTargetRolloutState::ExistingSameContent)
    } else {
        Ok(RestoreTargetRolloutState::ExistingDifferentContent)
    }
}

fn rollout_files_match(left: &Path, right: &Path) -> Result<bool, String> {
    let left_meta = fs::metadata(left)
        .map_err(|error| format!("读取会话文件信息失败 ({}): {}", left.display(), error))?;
    let right_meta = fs::metadata(right)
        .map_err(|error| format!("读取会话文件信息失败 ({}): {}", right.display(), error))?;
    if left_meta.len() != right_meta.len() {
        return Ok(false);
    }

    let left_digest = compute_file_md5(left)?;
    let right_digest = compute_file_md5(right)?;
    Ok(left_digest == right_digest)
}

fn compute_file_md5(path: &Path) -> Result<md5::Digest, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("打开会话文件失败 ({}): {}", path.display(), error))?;
    let mut context = md5::Context::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取会话文件失败 ({}): {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }

    Ok(context.compute())
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
    use std::cell::RefCell;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    fn write_rollout(path: &Path, session_id: &str, cwd: &str, event: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create rollout parent");
        }
        fs::write(
            path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{}\"}}}}\n{}\n",
                session_id, cwd, event
            ),
        )
        .expect("write rollout");
    }

    fn write_rollout_with_source(
        path: &Path,
        session_id: &str,
        cwd: &str,
        source: &str,
        event: &str,
    ) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create rollout parent");
        }
        fs::write(
            path,
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{}\",\"cwd\":\"{}\",\"source\":\"{}\"}}}}\n{}\n",
                session_id, cwd, source, event
            ),
        )
        .expect("write rollout");
    }

    fn make_entry(
        root: &Path,
        instance_root: &Path,
        instance_id: &str,
        session_id: &str,
        target_rel: &str,
        event: &str,
    ) -> TrashedSessionEntry {
        let entry_dir = root.join(format!("{}--{}", instance_id, session_id));
        let trashed_rollout_path = entry_dir.join("files").join(target_rel);
        write_rollout(&trashed_rollout_path, session_id, "C:/work", event);
        TrashedSessionEntry {
            entry_dir,
            manifest: TrashedSessionManifest {
                session_id: session_id.to_string(),
                title: format!("title {}", session_id),
                cwd: "C:/work".to_string(),
                instance_id: instance_id.to_string(),
                instance_name: format!("instance {}", instance_id),
                instance_root: instance_root.to_path_buf(),
                original_rollout_path: instance_root.join(target_rel),
                relative_rollout_path: target_rel.to_string(),
                session_index_entry: json!({
                    "id": session_id,
                    "thread_name": format!("title {}", session_id),
                    "updated_at": "2026-02-01T12:00:00Z"
                }),
                deleted_at: Some("2026-02-01T12:00:00Z".to_string()),
            },
            trashed_rollout_path,
        }
    }

    #[test]
    fn classify_restore_target_rollout_detects_same_content_file() {
        let root = make_temp_dir("codex-session-restore-same-content");
        let trash_path = root.join("trash-rollout.jsonl");
        let target_path = root.join("sessions").join("rollout.jsonl");
        fs::create_dir_all(target_path.parent().expect("target parent"))
            .expect("create target dir");
        fs::write(&trash_path, "{\"id\":\"s1\"}\n{\"event\":1}\n").expect("write trash rollout");
        fs::write(&target_path, "{\"id\":\"s1\"}\n{\"event\":1}\n").expect("write target rollout");

        let state =
            classify_restore_target_rollout(&trash_path, &target_path).expect("classify rollout");
        assert_eq!(state, RestoreTargetRolloutState::ExistingSameContent);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn classify_restore_target_rollout_detects_different_content_file() {
        let root = make_temp_dir("codex-session-restore-different-content");
        let trash_path = root.join("trash-rollout.jsonl");
        let target_path = root.join("sessions").join("rollout.jsonl");
        fs::create_dir_all(target_path.parent().expect("target parent"))
            .expect("create target dir");
        fs::write(&trash_path, "{\"id\":\"s1\"}\n{\"event\":1}\n").expect("write trash rollout");
        fs::write(&target_path, "{\"id\":\"s1\"}\n{\"event\":2}\n").expect("write target rollout");

        let state =
            classify_restore_target_rollout(&trash_path, &target_path).expect("classify rollout");
        assert_eq!(state, RestoreTargetRolloutState::ExistingDifferentContent);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn load_thread_snapshots_marks_archived_source_kind() {
        let root = make_temp_dir("codex-session-archived-source-kind");
        let data_dir = root.join("codex-home");
        let active_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-active.jsonl");
        let archived_path = data_dir
            .join("archived_sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-archived.jsonl");
        write_rollout(&active_path, "active-session", "C:/active", "{\"event\":1}");
        write_rollout(
            &archived_path,
            "archived-session",
            "C:/archived",
            "{\"event\":2}",
        );

        let snapshots = load_thread_snapshots(&CodexSyncInstance {
            id: "instance-a".to_string(),
            name: "Instance A".to_string(),
            data_dir: data_dir.clone(),
            last_pid: None,
        })
        .expect("load snapshots");
        let active = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "active-session")
            .expect("active snapshot");
        let archived = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "archived-session")
            .expect("archived snapshot");

        assert_eq!(active.source_kind, "sessions");
        assert_eq!(archived.source_kind, "archived_sessions");

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn load_thread_snapshots_marks_removed_and_projectless_sidebar_state() {
        let root = make_temp_dir("codex-session-sidebar-state");
        let data_dir = root.join("codex-home");
        let visible_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-visible.jsonl");
        let removed_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-removed.jsonl");
        let projectless_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-projectless.jsonl");
        write_rollout(
            &visible_path,
            "visible-thread",
            "C:/work/visible",
            "{\"event\":1}",
        );
        write_rollout(
            &removed_path,
            "removed-thread",
            "C:/work/removed",
            "{\"event\":2}",
        );
        write_rollout(
            &projectless_path,
            "projectless-thread",
            "C:/Users/demo/Documents/Codex/2026-06-13/new-chat",
            "{\"event\":3}",
        );
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\work\\visible"],"projectless-thread-ids":["projectless-thread"],"sidebar-project-thread-orders":{"C:\\work\\visible":["visible-thread"],"C:\\work\\removed":["removed-thread"]},"thread-projectless-output-directories":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat\\outputs"}}"#,
        )
        .expect("write global state");

        let snapshots = load_thread_snapshots(&CodexSyncInstance {
            id: "instance-a".to_string(),
            name: "Instance A".to_string(),
            data_dir: data_dir.clone(),
            last_pid: None,
        })
        .expect("load snapshots");
        let visible = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "visible-thread")
            .expect("visible snapshot");
        let removed = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "removed-thread")
            .expect("removed snapshot");
        let projectless = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "projectless-thread")
            .expect("projectless snapshot");

        assert!(!visible.removed);
        assert!(!visible.projectless);
        assert!(removed.removed);
        assert!(!removed.projectless);
        assert!(projectless.projectless);
        assert!(!projectless.removed);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn load_thread_snapshots_marks_removed_and_projectless_from_sidebar_backup_state() {
        let root = make_temp_dir("codex-session-sidebar-backup-state");
        let data_dir = root.join("codex-home");
        let visible_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-visible.jsonl");
        let removed_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-removed.jsonl");
        let projectless_path = data_dir
            .join("sessions")
            .join("2026")
            .join("02")
            .join("01")
            .join("rollout-projectless.jsonl");
        write_rollout(
            &visible_path,
            "visible-thread",
            "C:/work/visible",
            "{\"event\":1}",
        );
        write_rollout(
            &removed_path,
            "removed-thread",
            "C:/work/removed",
            "{\"event\":2}",
        );
        write_rollout(
            &projectless_path,
            "projectless-thread",
            "C:/Users/demo/Documents/Codex/2026-06-13/new-chat",
            "{\"event\":3}",
        );
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\work\\visible","C:\\work\\removed","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-saved-workspace-roots":["C:\\work\\visible","C:\\work\\removed","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"]}"#,
        )
        .expect("write current global state");
        let backup_dir = data_dir.join("backup-20260613-170559-global-state-sidebar-repair");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\work\\visible"],"projectless-thread-ids":["projectless-thread"],"sidebar-project-thread-orders":{"C:\\work\\visible":["visible-thread"],"C:\\work\\removed":["removed-thread"]},"thread-projectless-output-directories":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat\\outputs"}}"#,
        )
        .expect("write backup global state");

        let snapshots = load_thread_snapshots(&CodexSyncInstance {
            id: "instance-a".to_string(),
            name: "Instance A".to_string(),
            data_dir: data_dir.clone(),
            last_pid: None,
        })
        .expect("load snapshots");
        let removed = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "removed-thread")
            .expect("removed snapshot");
        let projectless = snapshots
            .iter()
            .find(|snapshot| snapshot.id == "projectless-thread")
            .expect("projectless snapshot");

        assert!(removed.removed);
        assert!(!removed.projectless);
        assert!(projectless.projectless);
        assert!(!projectless.removed);

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn preview_restore_entries_reports_different_content_conflicts() {
        let root = make_temp_dir("codex-session-restore-preview-conflict");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let target_path = instance_root.join(target_rel);
        write_rollout(&target_path, "s1", "C:/work", "{\"target\":true}");
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );

        let preview = preview_restore_entries(&[entry], 1).expect("preview restore");

        assert_eq!(preview.requested_session_count, 1);
        assert_eq!(preview.conflict_count, 1);
        assert_eq!(preview.conflicts[0].session_id, "s1");
        assert_eq!(
            preview.conflicts[0].target_rollout_path,
            target_path.to_string_lossy()
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn preview_restore_entries_reports_source_repair_candidates() {
        let root = make_temp_dir("codex-session-restore-preview-source");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let active_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let archived_rel = "archived_sessions/2026/02/01/rollout-s2.jsonl";
        let active_entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            active_rel,
            "{\"trash\":true}",
        );
        write_rollout_with_source(
            &active_entry.trashed_rollout_path,
            "s1",
            "C:/work",
            "vscode",
            "{\"trash\":true}",
        );
        let archived_entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s2",
            archived_rel,
            "{\"trash\":true}",
        );
        write_rollout_with_source(
            &archived_entry.trashed_rollout_path,
            "s2",
            "C:/work",
            "vscode",
            "{\"trash\":true}",
        );

        let preview =
            preview_restore_entries(&[active_entry, archived_entry], 2).expect("preview restore");

        assert_eq!(preview.source_repair_count, 1);
        assert_eq!(preview.source_repair_candidates[0].session_id, "s1");
        assert_eq!(preview.source_repair_candidates[0].current_source, "vscode");
        assert_eq!(preview.source_repair_candidates[0].target_source, "cli");

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn restore_entries_creates_missing_parent_directory() {
        let root = make_temp_dir("codex-session-restore-missing-parent");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );
        let rebuilt_roots = RefCell::new(Vec::<PathBuf>::new());

        let summary = restore_trashed_session_entries_with_rebuild(
            &[entry.clone()],
            1,
            false,
            false,
            Some(&backup_root),
            &|root| {
                rebuilt_roots.borrow_mut().push(root.to_path_buf());
                Ok(())
            },
        )
        .expect("restore missing parent");

        assert_eq!(summary.restored_session_count, 1);
        assert!(instance_root.join(target_rel).exists());
        assert_eq!(rebuilt_roots.borrow().as_slice(), &[instance_root.clone()]);
        assert!(!entry.entry_dir.exists());

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn restore_entries_can_normalize_sources_before_rebuild() {
        let root = make_temp_dir("codex-session-restore-normalize-source");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );
        write_rollout_with_source(
            &entry.trashed_rollout_path,
            "s1",
            "C:/work",
            "vscode",
            "{\"trash\":true}",
        );

        restore_trashed_session_entries_with_rebuild(
            &[entry],
            1,
            false,
            true,
            Some(&backup_root),
            &|root| {
                let rollout = root.join(target_rel);
                assert!(fs::read_to_string(rollout)
                    .expect("read rollout before rebuild")
                    .contains("\"source\":\"cli\""));
                Ok(())
            },
        )
        .expect("restore and normalize source");

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn force_restore_backs_up_overwrites_and_cleans_trash_entry() {
        let root = make_temp_dir("codex-session-force-restore");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let target_path = instance_root.join(target_rel);
        write_rollout(&target_path, "s1", "C:/work", "{\"target\":true}");
        fs::write(
            instance_root.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"old\"}\n{\"id\":\"other\"}\n",
        )
        .expect("write old session index");
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );

        let summary = restore_trashed_session_entries_with_rebuild(
            &[entry.clone()],
            1,
            true,
            false,
            Some(&backup_root),
            &|_| Ok(()),
        )
        .expect("force restore");

        assert_eq!(summary.restored_session_count, 1);
        assert!(summary.backup_batch_id.is_some());
        let target_content = fs::read_to_string(&target_path).expect("read target rollout");
        assert!(target_content.contains("\"trash\":true"));
        let index_content =
            fs::read_to_string(instance_root.join(SESSION_INDEX_FILE)).expect("read session index");
        assert!(index_content.contains("\"thread_name\":\"title s1\""));
        assert!(index_content.contains("\"id\":\"other\""));
        assert!(!index_content.contains("\"thread_name\":\"old\""));
        assert!(!entry.entry_dir.exists());

        let batch_dir = backup_root.join(summary.backup_batch_id.expect("backup batch id"));
        assert!(batch_dir.join("manifest.json").exists());
        let backup_rollout = list_rollout_files(&batch_dir.join("files"))
            .expect("list backup rollout")
            .into_iter()
            .next()
            .expect("backup rollout");
        assert!(fs::read_to_string(backup_rollout)
            .expect("read backup rollout")
            .contains("\"target\":true"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn force_restore_rolls_back_when_rebuild_fails() {
        let root = make_temp_dir("codex-session-force-restore-rollback");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let target_path = instance_root.join(target_rel);
        write_rollout(&target_path, "s1", "C:/work", "{\"target\":true}");
        fs::write(
            instance_root.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"old\"}\n",
        )
        .expect("write old session index");
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );

        let error = restore_trashed_session_entries_with_rebuild(
            &[entry.clone()],
            1,
            true,
            false,
            Some(&backup_root),
            &|_| Err("rebuild failed".to_string()),
        )
        .expect_err("force restore should fail");

        assert!(error.contains("已回滚"));
        assert!(fs::read_to_string(&target_path)
            .expect("read target rollout")
            .contains("\"target\":true"));
        assert_eq!(
            fs::read_to_string(instance_root.join(SESSION_INDEX_FILE)).expect("read session index"),
            "{\"id\":\"s1\",\"thread_name\":\"old\"}\n"
        );
        assert!(entry.entry_dir.exists());

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn restore_rollout_backup_batch_moves_current_rollout_to_trash_first() {
        let root = make_temp_dir("codex-session-restore-rollout-backup");
        let instance_root = root.join("instance-a");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let target_path = instance_root.join(target_rel);
        write_rollout(&target_path, "s1", "C:/work", "{\"target\":true}");
        let original_modified_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        modules::codex_session_file_time::restore_modified_time(
            &target_path,
            Some(original_modified_at),
        )
        .expect("set original rollout mtime");
        fs::write(
            instance_root.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"old\"}\n",
        )
        .expect("write old session index");
        let entry = make_entry(
            &trash_root,
            &instance_root,
            "instance-a",
            "s1",
            target_rel,
            "{\"trash\":true}",
        );
        let force_summary = restore_trashed_session_entries_with_rebuild(
            &[entry],
            1,
            true,
            false,
            Some(&backup_root),
            &|_| Ok(()),
        )
        .expect("force restore");
        let batch_dir = backup_root.join(force_summary.backup_batch_id.expect("backup batch id"));
        let rollback_trash_root = root.join("rollback-trash");

        let restore_summary = restore_rollout_backup_batch_with_rebuild(
            &batch_dir,
            Some(&rollback_trash_root),
            &|_| Ok(()),
        )
        .expect("restore backup batch");

        assert_eq!(restore_summary.restored_session_count, 1);
        assert!(fs::read_to_string(&target_path)
            .expect("read restored target")
            .contains("\"target\":true"));
        assert!(modules::codex_session_file_time::same_modified_time_millis(
            modules::codex_session_file_time::read_modified_time(&target_path),
            Some(original_modified_at)
        ));
        assert_eq!(
            fs::read_to_string(instance_root.join(SESSION_INDEX_FILE)).expect("read session index"),
            "{\"id\":\"s1\",\"thread_name\":\"old\"}\n"
        );
        let rollback_rollout = list_rollout_files(&rollback_trash_root)
            .expect("list rollback trash")
            .into_iter()
            .next()
            .expect("rollback trashed rollout");
        assert!(fs::read_to_string(rollback_rollout)
            .expect("read rollback trash rollout")
            .contains("\"trash\":true"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }

    #[test]
    fn restore_entries_rebuilds_each_instance_once_using_manifest_roots() {
        let root = make_temp_dir("codex-session-restore-multi-instance");
        let instance_a = root.join("instance-a");
        let instance_b = root.join("instance-b");
        let trash_root = root.join("trash");
        let backup_root = root.join("backups");
        let target_rel = "sessions/2026/02/01/rollout-s1.jsonl";
        let entry_a = make_entry(
            &trash_root,
            &instance_a,
            "instance-a",
            "s1",
            target_rel,
            "{\"a\":true}",
        );
        let entry_b = make_entry(
            &trash_root,
            &instance_b,
            "instance-b",
            "s1",
            target_rel,
            "{\"b\":true}",
        );
        let rebuilt_roots = RefCell::new(Vec::<PathBuf>::new());

        let summary = restore_trashed_session_entries_with_rebuild(
            &[entry_a, entry_b],
            1,
            false,
            false,
            Some(&backup_root),
            &|root| {
                rebuilt_roots.borrow_mut().push(root.to_path_buf());
                Ok(())
            },
        )
        .expect("restore multi-instance");

        assert_eq!(summary.restored_session_count, 1);
        assert_eq!(summary.restored_instance_count, 2);
        assert!(instance_a.join(target_rel).exists());
        assert!(instance_b.join(target_rel).exists());
        assert_eq!(
            rebuilt_roots.borrow().as_slice(),
            &[instance_a.clone(), instance_b.clone()]
        );

        fs::remove_dir_all(root).expect("cleanup temp dir");
    }
}
