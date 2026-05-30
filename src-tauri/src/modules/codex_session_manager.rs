use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_TRASH_ROOT_DIR: &str = "cockpit-tools-codex-session-trash";
const TOKEN_STATS_READ_CHUNK_BYTES: usize = 64 * 1024;

static TOKEN_STATS_CACHE: LazyLock<Mutex<HashMap<PathBuf, TokenStatsCacheEntry>>> =
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
struct TokenStatsCacheEntry {
    file_len: u64,
    modified_at: Option<SystemTime>,
    stats: Option<(u64, u64, u64)>,
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

    for entry in &entries {
        restore_trashed_session_entry(entry)?;
        restored_session_ids.insert(entry.manifest.session_id.clone());
        restored_instance_ids.insert(entry.manifest.instance_id.clone());
    }

    let restored_running_instance = restored_instance_ids
        .iter()
        .any(|instance_id| running_instance_ids.contains(instance_id));
    let message = if restored_running_instance {
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

    Ok(CodexSessionRestoreSummary {
        requested_session_count: requested_ids.len(),
        restored_session_count: restored_session_ids.len(),
        restored_instance_count: restored_instance_ids.len(),
        message,
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

fn is_instance_running(
    instance: &CodexSyncInstance,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let codex_home = if instance.id == DEFAULT_INSTANCE_ID {
        None
    } else {
        instance.data_dir.to_str()
    };
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

fn restore_trashed_session_entry(entry: &TrashedSessionEntry) -> Result<(), String> {
    if !entry.trashed_rollout_path.exists() {
        return Err(format!(
            "废纸篓中的会话文件不存在，无法恢复 ({}): {}",
            entry.manifest.session_id,
            entry.trashed_rollout_path.display()
        ));
    }

    let session_id = entry.manifest.session_id.clone();
    let target_rollout_path = entry.manifest.original_rollout_path.clone();
    if target_rollout_path.exists() {
        return Err(format!(
            "目标实例中已存在同名会话文件，无法恢复 ({}): {}",
            session_id,
            target_rollout_path.display()
        ));
    }

    let original_session_index_content = read_session_index_content(&entry.manifest.instance_root)?;
    if session_index_contains_id(&original_session_index_content, &session_id)? {
        return Err(format!(
            "目标实例的 session_index.jsonl 中已存在该会话，无法恢复 ({})",
            session_id
        ));
    }

    if let Some(parent) = target_rollout_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建会话恢复目录失败 ({}): {}", parent.display(), error))?;
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

    let restore_result = (|| {
        write_session_index_with_entry(
            &entry.manifest.instance_root,
            &original_session_index_content,
            &session_id,
            &entry.manifest.session_index_entry,
        )?;
        modules::codex_official_app_server::rebuild_thread_metadata(&entry.manifest.instance_root)
            .map_err(|error| {
                format!(
                    "会话文件已恢复，但官方 Codex 重建会话索引失败 ({}): {}",
                    entry.manifest.instance_name, error
                )
            })?;
        Ok::<(), String>(())
    })();

    if let Err(error) = restore_result {
        let _ = fs::remove_file(&target_rollout_path);
        let _ = restore_session_index_content(
            &entry.manifest.instance_root,
            original_session_index_content.as_deref(),
        );
        return Err(error);
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

    Ok(())
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

fn session_index_contains_id(content: &Option<String>, session_id: &str) -> Result<bool, String> {
    let Some(content) = content.as_deref() else {
        return Ok(false);
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = serde_json::from_str::<JsonValue>(trimmed)
            .map_err(|error| format!("解析 session_index.jsonl 条目失败: {}", error))?;
        if parsed.get("id").and_then(JsonValue::as_str) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn write_session_index_with_entry(
    root_dir: &Path,
    original_content: &Option<String>,
    session_id: &str,
    entry: &JsonValue,
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    let serialized_entry = serde_json::to_string(entry)
        .map_err(|error| format!("序列化 session_index 条目失败 ({}): {}", session_id, error))?;
    let mut lines = original_content
        .as_deref()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    lines.push(serialized_entry);
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
