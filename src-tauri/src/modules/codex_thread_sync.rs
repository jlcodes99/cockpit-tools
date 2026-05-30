use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const BACKUP_FILE_NAMES: [&str; 2] = [SESSION_INDEX_FILE, GLOBAL_STATE_FILE];
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceThreadSyncItem {
    pub instance_id: String,
    pub instance_name: String,
    pub added_thread_count: usize,
    pub updated_thread_count: usize,
    pub backup_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceThreadSyncSummary {
    pub instance_count: usize,
    pub thread_universe_count: usize,
    pub mutated_instance_count: usize,
    pub total_synced_thread_count: usize,
    pub total_added_thread_count: usize,
    pub total_updated_thread_count: usize,
    pub items: Vec<CodexInstanceThreadSyncItem>,
    pub backup_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceTargetThreadSyncSummary {
    pub requested_session_count: usize,
    pub target_instance_id: String,
    pub target_instance_name: String,
    pub synced_session_count: usize,
    pub skipped_existing_count: usize,
    pub missing_session_count: usize,
    pub backup_dir: Option<String>,
    pub running: bool,
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
    rollout_path: PathBuf,
    rollout_actual_modified_at: Option<SystemTime>,
    rollout_modified_at: Option<SystemTime>,
    merged_rollout_content: Option<String>,
    session_index_entry: JsonValue,
    workspace_root: Option<String>,
    source_root: PathBuf,
    freshness: ThreadFreshness,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct ThreadFreshness {
    activity_ms: i128,
    rollout_len: u64,
    rollout_modified_ms: i128,
}

#[derive(Debug, Clone)]
struct ThreadSyncPlanItem {
    snapshot: ThreadSnapshot,
    existing_rollout_path: Option<PathBuf>,
    is_update: bool,
}

#[derive(Debug, Clone)]
struct ThreadSyncWriteResult {
    backup_dir: PathBuf,
    metadata_rebuild_failed: bool,
}

#[derive(Debug, Clone)]
struct RolloutMergeLine {
    line: String,
    timestamp_ms: Option<i128>,
    source_rank: usize,
    line_index: usize,
}

pub fn sync_threads_across_instances() -> Result<CodexInstanceThreadSyncSummary, String> {
    let instances = collect_instances()?;
    if instances.len() < 2 {
        return Err("至少需要两个 Codex 实例才能同步线程".to_string());
    }

    let mut snapshots_by_thread = HashMap::<String, Vec<ThreadSnapshot>>::new();
    let mut snapshots_by_instance = HashMap::<String, HashMap<String, ThreadSnapshot>>::new();

    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?;
        let mut snapshots_by_id = HashMap::<String, ThreadSnapshot>::new();
        for snapshot in snapshots {
            snapshots_by_thread
                .entry(snapshot.id.clone())
                .or_default()
                .push(snapshot.clone());
            match snapshots_by_id.get(&snapshot.id) {
                Some(existing) if existing.freshness >= snapshot.freshness => {}
                _ => {
                    snapshots_by_id.insert(snapshot.id.clone(), snapshot);
                }
            }
        }
        snapshots_by_instance.insert(instance.id.clone(), snapshots_by_id);
    }

    let mut thread_universe = HashMap::<String, ThreadSnapshot>::new();
    for (thread_id, snapshots) in snapshots_by_thread {
        thread_universe.insert(thread_id, merge_thread_snapshots(&snapshots)?);
    }

    let mut universe_ids = thread_universe.keys().cloned().collect::<Vec<_>>();
    universe_ids.sort();

    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut total_synced_thread_count = 0usize;
    let mut total_added_thread_count = 0usize;
    let mut total_updated_thread_count = 0usize;
    let mut project_index_repaired_instance_count = 0usize;
    let mut mutated_running_instance_count = 0usize;
    let mut metadata_rebuild_failed_instance_count = 0usize;

    for instance in &instances {
        let existing_snapshots = snapshots_by_instance
            .get(&instance.id)
            .cloned()
            .unwrap_or_default();
        let mut plan_items = Vec::new();
        let mut added_thread_count = 0usize;
        let mut updated_thread_count = 0usize;
        let expected_snapshots = universe_ids
            .iter()
            .filter_map(|id| thread_universe.get(id).cloned())
            .collect::<Vec<_>>();

        for id in &universe_ids {
            let Some(best_snapshot) = thread_universe.get(id) else {
                continue;
            };
            match existing_snapshots.get(id) {
                Some(existing)
                    if existing.freshness >= best_snapshot.freshness
                        && snapshot_rollout_matches(existing, best_snapshot)
                        && snapshot_modified_time_matches(existing, best_snapshot) => {}
                Some(existing) => {
                    updated_thread_count += 1;
                    plan_items.push(ThreadSyncPlanItem {
                        snapshot: best_snapshot.clone(),
                        existing_rollout_path: Some(existing.rollout_path.clone()),
                        is_update: true,
                    });
                }
                None => {
                    added_thread_count += 1;
                    plan_items.push(ThreadSyncPlanItem {
                        snapshot: best_snapshot.clone(),
                        existing_rollout_path: None,
                        is_update: false,
                    });
                }
            }
        }

        let missing_workspace_roots =
            find_missing_thread_workspace_roots(&instance.data_dir, &expected_snapshots)?;
        let repairs_project_index = !missing_workspace_roots.is_empty();

        if plan_items.is_empty() && !repairs_project_index {
            items.push(CodexInstanceThreadSyncItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                added_thread_count: 0,
                updated_thread_count: 0,
                backup_dir: None,
            });
            continue;
        }

        let write_result =
            sync_thread_plan_to_instance(instance, &plan_items, &expected_snapshots)?;
        let backup_dir = write_result.backup_dir;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();
        backup_dirs.push(backup_dir_string.clone());
        mutated_instance_count += 1;
        if repairs_project_index {
            project_index_repaired_instance_count += 1;
        }
        if write_result.metadata_rebuild_failed {
            metadata_rebuild_failed_instance_count += 1;
        }
        total_synced_thread_count += plan_items.len();
        total_added_thread_count += added_thread_count;
        total_updated_thread_count += updated_thread_count;
        if is_instance_running(instance, &process_entries) {
            mutated_running_instance_count += 1;
        }

        items.push(CodexInstanceThreadSyncItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            added_thread_count,
            updated_thread_count,
            backup_dir: Some(backup_dir_string),
        });
    }

    let message = if total_synced_thread_count == 0 && project_index_repaired_instance_count == 0 {
        "所有 Codex 实例会话已是最新，无需同步".to_string()
    } else if total_synced_thread_count == 0 {
        format!(
            "会话内容已是最新，已修复 {} 个实例的项目索引",
            project_index_repaired_instance_count
        )
    } else if mutated_running_instance_count > 0 {
        format!(
            "已为 {} 个实例同步 {} 条会话（新增 {} 条，更新 {} 条），并已触发官方 Codex 重建会话索引；运行中的实例可能需要刷新或重启后显示",
            mutated_instance_count,
            total_synced_thread_count,
            total_added_thread_count,
            total_updated_thread_count
        )
    } else {
        format!(
            "已为 {} 个实例同步 {} 条会话（新增 {} 条，更新 {} 条），并已触发官方 Codex 重建会话索引",
            mutated_instance_count,
            total_synced_thread_count,
            total_added_thread_count,
            total_updated_thread_count
        )
    };

    let message = append_metadata_rebuild_warning(
        message,
        metadata_rebuild_failed_instance_count,
        total_synced_thread_count,
    );

    Ok(CodexInstanceThreadSyncSummary {
        instance_count: instances.len(),
        thread_universe_count: thread_universe.len(),
        mutated_instance_count,
        total_synced_thread_count,
        total_added_thread_count,
        total_updated_thread_count,
        items,
        backup_dirs,
        message,
    })
}

pub fn sync_threads_across_instances_if_all_stopped(
) -> Result<Option<CodexInstanceThreadSyncSummary>, String> {
    let instances = collect_instances()?;
    if instances.len() < 2 {
        return Ok(None);
    }

    let process_entries = modules::process::collect_codex_process_entries();
    if instances
        .iter()
        .any(|instance| is_instance_running(instance, &process_entries))
    {
        return Ok(None);
    }

    sync_threads_across_instances().map(Some)
}

pub fn sync_sessions_to_instance(
    session_ids: Vec<String>,
    target_instance_id: String,
) -> Result<CodexInstanceTargetThreadSyncSummary, String> {
    let requested_ids = session_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<HashSet<_>>();
    if requested_ids.is_empty() {
        return Err("请至少选择一条会话".to_string());
    }

    let target_id = target_instance_id.trim();
    if target_id.is_empty() {
        return Err("请选择目标实例".to_string());
    }

    let instances = collect_instances()?;
    let target = instances
        .iter()
        .find(|instance| instance.id == target_id)
        .cloned()
        .ok_or_else(|| format!("目标实例不存在: {}", target_id))?;

    let mut source_snapshots = HashMap::<String, ThreadSnapshot>::new();
    let mut target_existing_ids = HashSet::<String>::new();
    for instance in &instances {
        let snapshots = load_thread_snapshots(instance)?;
        if instance.id == target.id {
            target_existing_ids = snapshots
                .iter()
                .map(|snapshot| snapshot.id.clone())
                .collect::<HashSet<_>>();
            continue;
        }

        for snapshot in snapshots {
            if requested_ids.contains(&snapshot.id) {
                source_snapshots
                    .entry(snapshot.id.clone())
                    .or_insert(snapshot);
            }
        }
    }

    let mut snapshots_to_sync = Vec::new();
    let mut skipped_existing_count = 0usize;
    let mut missing_session_count = 0usize;
    let mut ordered_ids = requested_ids.iter().cloned().collect::<Vec<_>>();
    ordered_ids.sort();
    for session_id in ordered_ids {
        if target_existing_ids.contains(&session_id) {
            skipped_existing_count += 1;
            continue;
        }
        match source_snapshots.get(&session_id) {
            Some(snapshot) => snapshots_to_sync.push(snapshot.clone()),
            None => missing_session_count += 1,
        }
    }

    let process_entries = modules::process::collect_codex_process_entries();
    let running = is_instance_running(&target, &process_entries);

    if snapshots_to_sync.is_empty() {
        let message = if skipped_existing_count > 0 && missing_session_count == 0 {
            format!(
                "目标实例已存在所选 {} 条会话，无需恢复",
                skipped_existing_count
            )
        } else {
            "所选会话在其他实例中不存在，无法恢复到目标实例".to_string()
        };
        return Ok(CodexInstanceTargetThreadSyncSummary {
            requested_session_count: requested_ids.len(),
            target_instance_id: target.id,
            target_instance_name: target.name,
            synced_session_count: 0,
            skipped_existing_count,
            missing_session_count,
            backup_dir: None,
            running,
            message,
        });
    }

    let write_result = sync_missing_threads_to_instance(&target, &snapshots_to_sync)?;
    let backup_dir = write_result.backup_dir;
    let synced_session_count = snapshots_to_sync.len();
    let message = if running {
        format!(
            "已恢复 {} 条会话到「{}」，并已触发官方 Codex 重建会话索引；目标实例运行中，可能需要刷新或重启后显示",
            synced_session_count, target.name
        )
    } else {
        format!(
            "已恢复 {} 条会话到「{}」，并已触发官方 Codex 重建会话索引",
            synced_session_count, target.name
        )
    };
    let message = append_metadata_rebuild_warning(
        message,
        usize::from(write_result.metadata_rebuild_failed),
        synced_session_count,
    );

    Ok(CodexInstanceTargetThreadSyncSummary {
        requested_session_count: requested_ids.len(),
        target_instance_id: target.id,
        target_instance_name: target.name,
        synced_session_count,
        skipped_existing_count,
        missing_session_count,
        backup_dir: Some(backup_dir.to_string_lossy().to_string()),
        running,
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
            let freshness = build_thread_freshness(session_index_map.get(&id), &rollout_path);
            let title = session_index_map
                .get(&id)
                .and_then(session_index_title)
                .unwrap_or_else(|| id.clone());
            let updated_at = session_index_map
                .get(&id)
                .and_then(session_index_updated_at_text)
                .or_else(|| format_timestamp_from_ms(freshness.activity_ms));
            let session_index_entry = session_index_map.get(&id).cloned().unwrap_or_else(|| {
                build_fallback_session_index_entry(&id, &title, updated_at.as_deref())
            });
            let workspace_root = session_meta_cwd(&session_meta);
            let rollout_actual_modified_at =
                modules::codex_session_file_time::read_modified_time(&rollout_path);
            let rollout_modified_at =
                modules::codex_session_file_time::system_time_from_unix_millis(
                    freshness.activity_ms,
                )
                .or(rollout_actual_modified_at);

            snapshots.push(ThreadSnapshot {
                id,
                rollout_path,
                rollout_actual_modified_at,
                rollout_modified_at,
                merged_rollout_content: None,
                session_index_entry,
                workspace_root,
                source_root: instance.data_dir.clone(),
                freshness,
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
        .or_else(|| meta.get("cwd"))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

fn sync_missing_threads_to_instance(
    target: &CodexSyncInstance,
    snapshots: &[ThreadSnapshot],
) -> Result<ThreadSyncWriteResult, String> {
    let plan_items = snapshots
        .iter()
        .cloned()
        .map(|snapshot| ThreadSyncPlanItem {
            snapshot,
            existing_rollout_path: None,
            is_update: false,
        })
        .collect::<Vec<_>>();
    sync_thread_plan_to_instance(target, &plan_items, snapshots)
}

fn sync_thread_plan_to_instance(
    target: &CodexSyncInstance,
    plan_items: &[ThreadSyncPlanItem],
    workspace_snapshots: &[ThreadSnapshot],
) -> Result<ThreadSyncWriteResult, String> {
    let backup_dir = backup_instance_files(&target.data_dir)?;
    let target_provider =
        modules::codex_session_visibility::read_history_visibility_provider_for_dir(
            &target.data_dir,
        )?;

    for item in plan_items {
        let target_rollout_path = copy_rollout_file_for_plan(item, &target.data_dir, &backup_dir)?;
        rewrite_rollout_provider_for_target(&target_rollout_path, &target_provider)?;
    }

    let mut metadata_rebuild_failed = false;
    if !plan_items.is_empty() {
        let snapshots = plan_items
            .iter()
            .map(|item| item.snapshot.clone())
            .collect::<Vec<_>>();
        upsert_session_index_entries(&target.data_dir, &snapshots)?;
        metadata_rebuild_failed = !try_rebuild_thread_metadata(target);
    }
    update_global_state_thread_workspaces(&target.data_dir, workspace_snapshots)?;
    Ok(ThreadSyncWriteResult {
        backup_dir,
        metadata_rebuild_failed,
    })
}

fn try_rebuild_thread_metadata(target: &CodexSyncInstance) -> bool {
    match modules::codex_official_app_server::rebuild_thread_metadata(&target.data_dir) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "Codex thread sync: skipped official metadata rebuild for {} ({}): {}",
                target.name,
                target.data_dir.display(),
                error
            );
            false
        }
    }
}

fn append_metadata_rebuild_warning(
    message: String,
    failed_instance_count: usize,
    synced_thread_count: usize,
) -> String {
    if failed_instance_count == 0 || synced_thread_count == 0 {
        return message;
    }

    let message = message.replace("，并已触发官方 Codex 重建会话索引", "");
    format!(
        "{}；{} 个实例未能触发官方 Codex 重建会话索引，但 rollout/session_index 已同步完成",
        message, failed_instance_count
    )
}

fn merge_thread_snapshots(snapshots: &[ThreadSnapshot]) -> Result<ThreadSnapshot, String> {
    let mut ordered = snapshots.to_vec();
    ordered.sort_by(|left, right| right.freshness.cmp(&left.freshness));
    let Some(mut merged) = ordered.first().cloned() else {
        return Err("没有可同步的会话快照".to_string());
    };

    if ordered.len() <= 1 {
        return Ok(merged);
    }

    let merged_rollout_content = merge_rollout_contents(&ordered)?;
    let (activity_ms, rollout_len) = rollout_content_activity_and_len(&merged_rollout_content);
    merged.freshness = ThreadFreshness {
        activity_ms: merged.freshness.activity_ms.max(activity_ms),
        rollout_len,
        rollout_modified_ms: ordered
            .iter()
            .map(|snapshot| snapshot.freshness.rollout_modified_ms)
            .max()
            .unwrap_or(merged.freshness.rollout_modified_ms),
    };
    merged.rollout_modified_at = ordered
        .iter()
        .filter_map(|snapshot| snapshot.rollout_modified_at)
        .max_by_key(|modified_at| {
            modified_at
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        });
    merged.rollout_actual_modified_at = merged.rollout_modified_at;
    merged.merged_rollout_content = Some(merged_rollout_content);
    Ok(merged)
}

fn merge_rollout_contents(snapshots: &[ThreadSnapshot]) -> Result<String, String> {
    let mut session_meta = None::<String>;
    let mut seen_lines = HashSet::<String>::new();
    let mut merged_lines = Vec::<RolloutMergeLine>::new();

    for (source_rank, snapshot) in snapshots.iter().enumerate() {
        let content = fs::read_to_string(&snapshot.rollout_path).map_err(|error| {
            format!(
                "读取 rollout 文件失败 ({}): {}",
                snapshot.rollout_path.display(),
                error
            )
        })?;

        for (line_index, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let parsed = serde_json::from_str::<JsonValue>(trimmed).ok();
            if parsed
                .as_ref()
                .and_then(|value| value.get("type"))
                .and_then(JsonValue::as_str)
                == Some("session_meta")
            {
                if session_meta.is_none() {
                    session_meta = Some(trimmed.to_string());
                }
                continue;
            }

            let key = rollout_line_dedupe_key(trimmed, parsed.as_ref());
            if !seen_lines.insert(key) {
                continue;
            }

            merged_lines.push(RolloutMergeLine {
                line: trimmed.to_string(),
                timestamp_ms: parsed.as_ref().and_then(parse_rollout_line_timestamp_ms),
                source_rank,
                line_index,
            });
        }
    }

    merged_lines.sort_by(|left, right| {
        match (left.timestamp_ms, right.timestamp_ms) {
            (Some(left_time), Some(right_time)) => left_time.cmp(&right_time),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| left.source_rank.cmp(&right.source_rank))
        .then_with(|| left.line_index.cmp(&right.line_index))
    });

    let mut output_lines = Vec::with_capacity(merged_lines.len() + 1);
    if let Some(meta) = session_meta {
        output_lines.push(meta);
    }
    output_lines.extend(merged_lines.into_iter().map(|line| line.line));

    let mut output = output_lines.join("\n");
    output.push('\n');
    Ok(output)
}

fn rollout_line_dedupe_key(line: &str, parsed: Option<&JsonValue>) -> String {
    parsed
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_else(|| line.to_string())
}

fn rollout_content_activity_and_len(content: &str) -> (i128, u64) {
    let activity_ms = content
        .lines()
        .filter_map(|line| serde_json::from_str::<JsonValue>(line.trim()).ok())
        .filter_map(|value| parse_rollout_line_timestamp_ms(&value))
        .max()
        .unwrap_or(0);
    (activity_ms, content.as_bytes().len() as u64)
}

fn parse_rollout_line_timestamp_ms(value: &JsonValue) -> Option<i128> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .and_then(parse_json_timestamp_ms)
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
                .and_then(parse_json_timestamp_ms)
        })
}

fn parse_json_timestamp_ms(value: &JsonValue) -> Option<i128> {
    match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_ms),
        JsonValue::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp_millis() as i128)
            .or_else(|| text.parse::<i64>().ok().map(normalize_codex_timestamp_ms)),
        _ => None,
    }
}

fn snapshot_rollout_matches(existing: &ThreadSnapshot, expected: &ThreadSnapshot) -> bool {
    let Some(expected_content) = expected.merged_rollout_content.as_deref() else {
        return paths_point_to_same_file(&existing.rollout_path, &expected.rollout_path)
            || existing.freshness == expected.freshness;
    };

    fs::read_to_string(&existing.rollout_path)
        .map(|content| content == expected_content)
        .unwrap_or(false)
}

fn snapshot_modified_time_matches(existing: &ThreadSnapshot, expected: &ThreadSnapshot) -> bool {
    modules::codex_session_file_time::same_modified_time_millis(
        existing.rollout_actual_modified_at,
        expected.rollout_modified_at,
    )
}

fn build_thread_freshness(
    session_index_entry: Option<&JsonValue>,
    rollout_path: &Path,
) -> ThreadFreshness {
    let index_activity_ms = session_index_entry
        .and_then(parse_session_index_updated_at_ms)
        .unwrap_or(0);
    let (rollout_modified_ms, rollout_len) = rollout_file_metadata(rollout_path);
    let rollout_activity_ms = rollout_file_activity_ms(rollout_path).unwrap_or(0);
    let activity_ms = index_activity_ms.max(rollout_activity_ms).max(
        if index_activity_ms == 0 && rollout_activity_ms == 0 {
            rollout_modified_ms
        } else {
            0
        },
    );

    ThreadFreshness {
        activity_ms,
        rollout_len,
        rollout_modified_ms,
    }
}

fn normalize_codex_timestamp_ms(timestamp: i64) -> i128 {
    let timestamp = timestamp as i128;
    if timestamp > 10_000_000_000_000 {
        timestamp / 1_000
    } else if timestamp > 10_000_000_000 {
        timestamp
    } else {
        timestamp * 1_000
    }
}

fn parse_session_index_updated_at_ms(entry: &JsonValue) -> Option<i128> {
    [
        "updated_at",
        "updatedAt",
        "last_updated_at",
        "lastUpdatedAt",
    ]
    .iter()
    .filter_map(|key| entry.get(*key))
    .find_map(|value| match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_ms),
        JsonValue::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp_millis() as i128)
            .or_else(|| text.parse::<i64>().ok().map(normalize_codex_timestamp_ms)),
        _ => None,
    })
}

fn session_index_title(entry: &JsonValue) -> Option<String> {
    ["thread_name", "threadName", "title", "name"]
        .iter()
        .filter_map(|key| entry.get(*key))
        .find_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_index_updated_at_text(entry: &JsonValue) -> Option<String> {
    [
        "updated_at",
        "updatedAt",
        "last_updated_at",
        "lastUpdatedAt",
    ]
    .iter()
    .filter_map(|key| entry.get(*key))
    .find_map(|value| value.as_str().map(str::trim))
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

fn rollout_file_activity_ms(path: &Path) -> Option<i128> {
    let content = fs::read_to_string(path).ok()?;
    let (activity_ms, _) = rollout_content_activity_and_len(&content);
    (activity_ms > 0).then_some(activity_ms)
}

fn rollout_file_metadata(path: &Path) -> (i128, u64) {
    let Ok(metadata) = fs::metadata(path) else {
        return (0, 0);
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as i128)
        .unwrap_or(0);
    (modified_ms, metadata.len())
}

fn backup_instance_files(data_dir: &Path) -> Result<PathBuf, String> {
    let backup_dir = data_dir.join(format!(
        "backup-{}-instance-thread-sync",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("创建备份目录失败 ({}): {}", data_dir.display(), error))?;

    for file_name in BACKUP_FILE_NAMES {
        let source = data_dir.join(file_name);
        if !source.exists() {
            continue;
        }
        let target = backup_dir.join(format!("{}.bak", file_name));
        fs::copy(&source, &target).map_err(|error| {
            format!(
                "备份文件失败 ({} -> {}): {}",
                source.display(),
                target.display(),
                error
            )
        })?;
    }

    Ok(backup_dir)
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

fn build_fallback_session_index_entry(
    id: &str,
    title: &str,
    updated_at: Option<&str>,
) -> JsonValue {
    let mut value = json!({
        "id": id,
        "thread_name": title,
    });
    if let Some(updated_at) = updated_at {
        value["updated_at"] = JsonValue::String(updated_at.to_string());
    }
    value
}

fn upsert_session_index_entries(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<(), String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    let replacements = snapshots
        .iter()
        .map(|snapshot| {
            serde_json::to_string(&snapshot.session_index_entry)
                .map(|line| (snapshot.id.clone(), line))
                .map_err(|error| format!("序列化 session_index 条目失败: {}", error))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    if replacements.is_empty() {
        return Ok(());
    }

    let existing_content = if path.exists() {
        fs::read_to_string(&path).map_err(|error| {
            format!(
                "读取 session_index.jsonl 失败 ({}): {}",
                path.display(),
                error
            )
        })?
    } else {
        String::new()
    };

    let mut lines = Vec::new();
    let mut seen_ids = HashSet::new();
    for line in existing_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            lines.push(line.to_string());
            continue;
        }
        let replacement = serde_json::from_str::<JsonValue>(trimmed)
            .ok()
            .and_then(|parsed| {
                parsed
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .and_then(|id| {
                replacements.get(&id).map(|replacement| {
                    seen_ids.insert(id);
                    replacement.clone()
                })
            });
        lines.push(replacement.unwrap_or_else(|| line.to_string()));
    }

    let mut ordered_ids = replacements.keys().cloned().collect::<Vec<_>>();
    ordered_ids.sort();
    for id in ordered_ids {
        if !seen_ids.contains(&id) {
            if let Some(line) = replacements.get(&id) {
                lines.push(line.clone());
            }
        }
    }

    let mut output = lines.join("\n");
    output.push('\n');
    modules::atomic_write::write_string_atomic(&path, &output).map_err(|error| {
        format!(
            "写入 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(())
}

fn collect_thread_workspace_roots(snapshots: &[ThreadSnapshot]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();

    for snapshot in snapshots {
        let Some(root) = snapshot_workspace_root(snapshot) else {
            continue;
        };
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }

    roots
}

fn snapshot_workspace_root(snapshot: &ThreadSnapshot) -> Option<String> {
    snapshot
        .workspace_root
        .as_deref()
        .and_then(normalize_workspace_root)
        .or_else(|| session_index_workspace_root(&snapshot.session_index_entry))
}

fn session_index_workspace_root(entry: &JsonValue) -> Option<String> {
    [
        "cwd",
        "workspace_root",
        "workspaceRoot",
        "working_directory",
        "workingDirectory",
    ]
    .iter()
    .find_map(|key| entry.get(key).and_then(JsonValue::as_str))
    .and_then(normalize_workspace_root)
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
        None
    } else {
        Some(normalized)
    }
}

fn read_global_state(root_dir: &Path) -> Result<JsonValue, String> {
    let path = root_dir.join(GLOBAL_STATE_FILE);
    if !path.exists() {
        return Ok(json!({}));
    }

    let raw = fs::read_to_string(&path)
        .map_err(|error| format!("读取全局状态失败 ({}): {}", path.display(), error))?;
    Ok(serde_json::from_str::<JsonValue>(&raw).unwrap_or_else(|_| json!({})))
}

fn global_state_array_contains(
    object: &serde_json::Map<String, JsonValue>,
    key: &str,
    workspace: &str,
) -> bool {
    object
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|values| {
            values.iter().any(|value| {
                value.as_str().and_then(normalize_workspace_root).as_deref() == Some(workspace)
            })
        })
        .unwrap_or(false)
}

fn find_missing_thread_workspace_roots(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<Vec<String>, String> {
    let roots = collect_thread_workspace_roots(snapshots);
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let value = read_global_state(root_dir)?;
    let Some(object) = value.as_object() else {
        return Ok(roots);
    };

    Ok(roots
        .into_iter()
        .filter(|root| {
            !global_state_array_contains(object, "project-order", root)
                || !global_state_array_contains(object, "electron-saved-workspace-roots", root)
        })
        .collect())
}

fn update_global_state_thread_workspaces(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<bool, String> {
    let roots = collect_thread_workspace_roots(snapshots);
    if roots.is_empty() {
        return Ok(false);
    }

    let path = root_dir.join(GLOBAL_STATE_FILE);
    let mut value = read_global_state(root_dir)?;
    if !value.is_object() {
        value = json!({});
    }
    let Some(object) = value.as_object_mut() else {
        return Err("全局状态文件格式无效".to_string());
    };

    let mut changed = false;
    changed |= merge_string_array(object, "project-order", &roots);
    changed |= merge_string_array(object, "electron-saved-workspace-roots", &roots);

    if changed {
        let serialized = serde_json::to_string_pretty(&value)
            .map_err(|error| format!("序列化全局状态失败: {}", error))?;
        fs::write(&path, format!("{}\n", serialized))
            .map_err(|error| format!("写入全局状态失败 ({}): {}", path.display(), error))?;
    }

    Ok(changed)
}

fn merge_string_array(
    object: &mut serde_json::Map<String, JsonValue>,
    key: &str,
    additions: &[String],
) -> bool {
    let mut changed = false;
    let mut values = object
        .get(key)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| item.as_str().map(|value| value.to_string()))
        .collect::<Vec<_>>();
    let mut normalized_values = values
        .iter()
        .filter_map(|value| normalize_workspace_root(value))
        .collect::<HashSet<_>>();

    for addition in additions {
        let Some(normalized) = normalize_workspace_root(addition) else {
            continue;
        };
        if normalized_values.insert(normalized.clone()) {
            values.push(normalized);
            changed = true;
        }
    }

    if changed {
        object.insert(
            key.to_string(),
            JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
        );
    }

    changed
}

fn copy_rollout_file_for_plan(
    item: &ThreadSyncPlanItem,
    target_root: &Path,
    backup_dir: &Path,
) -> Result<PathBuf, String> {
    let target_path = resolve_target_rollout_path(
        &item.snapshot,
        target_root,
        item.existing_rollout_path.as_deref(),
    )?;
    if item.is_update {
        backup_existing_rollout_file(backup_dir, target_root, &target_path, &item.snapshot.id)?;
    }
    copy_rollout_file_to_path(&item.snapshot, &target_path)
}

fn resolve_target_rollout_path(
    snapshot: &ThreadSnapshot,
    target_root: &Path,
    existing_rollout_path: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(existing_path) = existing_rollout_path {
        if existing_path.starts_with(target_root) {
            return Ok(existing_path.to_path_buf());
        }
    }

    let relative_path = snapshot
        .rollout_path
        .strip_prefix(&snapshot.source_root)
        .map_err(|_| {
            format!(
                "线程 {} 的 rollout 路径不在实例目录下: {}",
                snapshot.id,
                snapshot.rollout_path.display()
            )
        })?;
    Ok(target_root.join(relative_path))
}

fn copy_rollout_file_to_path(
    snapshot: &ThreadSnapshot,
    target_path: &Path,
) -> Result<PathBuf, String> {
    if let Some(content) = snapshot.merged_rollout_content.as_deref() {
        let parent = target_path
            .parent()
            .ok_or_else(|| format!("无法解析目标 rollout 父目录: {}", target_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建 rollout 目录失败 ({}): {}", parent.display(), error))?;
        if fs::read_to_string(target_path)
            .map(|existing| existing == content)
            .unwrap_or(false)
        {
            modules::codex_session_file_time::restore_modified_time(
                target_path,
                snapshot.rollout_modified_at,
            )?;
            return Ok(target_path.to_path_buf());
        }
        modules::atomic_write::write_string_atomic(target_path, content).map_err(|error| {
            format!(
                "写入合并 rollout 文件失败 ({}): {}",
                target_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            target_path,
            snapshot.rollout_modified_at,
        )?;
        return Ok(target_path.to_path_buf());
    }

    if paths_point_to_same_file(&snapshot.rollout_path, target_path) {
        modules::codex_session_file_time::restore_modified_time(
            target_path,
            snapshot.rollout_modified_at,
        )?;
        return Ok(target_path.to_path_buf());
    }

    let parent = target_path
        .parent()
        .ok_or_else(|| format!("无法解析目标 rollout 父目录: {}", target_path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建 rollout 目录失败 ({}): {}", parent.display(), error))?;
    fs::copy(&snapshot.rollout_path, &target_path).map_err(|error| {
        format!(
            "复制 rollout 文件失败 ({} -> {}): {}",
            snapshot.rollout_path.display(),
            target_path.display(),
            error
        )
    })?;
    modules::codex_session_file_time::restore_modified_time(
        &target_path,
        snapshot.rollout_modified_at,
    )?;
    Ok(target_path.to_path_buf())
}

fn backup_existing_rollout_file(
    backup_dir: &Path,
    target_root: &Path,
    rollout_path: &Path,
    session_id: &str,
) -> Result<(), String> {
    if !rollout_path.exists() {
        return Ok(());
    }

    let backup_path = match rollout_path.strip_prefix(target_root) {
        Ok(relative_path) => backup_dir.join("rollouts").join(relative_path),
        Err(_) => backup_dir
            .join("rollouts")
            .join(format!("{}.jsonl.bak", sanitize_file_name(session_id))),
    };
    let parent = backup_path
        .parent()
        .ok_or_else(|| format!("无法解析 rollout 备份父目录: {}", backup_path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "创建 rollout 备份目录失败 ({}): {}",
            parent.display(),
            error
        )
    })?;
    fs::copy(rollout_path, &backup_path).map_err(|error| {
        format!(
            "备份目标 rollout 文件失败 ({} -> {}): {}",
            rollout_path.display(),
            backup_path.display(),
            error
        )
    })?;
    modules::codex_session_file_time::restore_modified_time(
        &backup_path,
        modules::codex_session_file_time::read_modified_time(rollout_path),
    )?;
    Ok(())
}

fn paths_point_to_same_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn sanitize_file_name(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '_',
        })
        .collect()
}

fn rewrite_rollout_provider_for_target(
    rollout_path: &Path,
    target_provider: &str,
) -> Result<(), String> {
    let original_modified_at = modules::codex_session_file_time::read_modified_time(rollout_path);
    let content = fs::read_to_string(rollout_path).map_err(|error| {
        format!(
            "读取目标 rollout 文件失败 ({}): {}",
            rollout_path.display(),
            error
        )
    })?;
    let Some(newline_index) = content.find('\n') else {
        return Ok(());
    };
    let first_line = &content[..newline_index];
    let rest = &content[newline_index..];
    let Ok(mut parsed) = serde_json::from_str::<JsonValue>(first_line) else {
        return Ok(());
    };
    if parsed.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return Ok(());
    }
    let Some(payload) = parsed.get_mut("payload").and_then(JsonValue::as_object_mut) else {
        return Ok(());
    };
    if payload.get("model_provider").and_then(JsonValue::as_str) == Some(target_provider) {
        return Ok(());
    }

    payload.insert(
        "model_provider".to_string(),
        JsonValue::String(target_provider.to_string()),
    );
    let updated_first_line = serde_json::to_string(&parsed)
        .map_err(|error| format!("序列化 rollout provider 元数据失败: {}", error))?;
    let updated_content = format!("{}{}", updated_first_line, rest);
    modules::atomic_write::write_string_atomic(rollout_path, &updated_content).map_err(
        |error| {
            format!(
                "写入目标 rollout provider 元数据失败 ({}): {}",
                rollout_path.display(),
                error
            )
        },
    )?;
    modules::codex_session_file_time::restore_modified_time(rollout_path, original_modified_at)
}

fn format_timestamp(timestamp: i64) -> Option<String> {
    if timestamp > 1_000_000_000_000 {
        chrono::DateTime::<Utc>::from_timestamp_millis(timestamp)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true))
    } else {
        chrono::DateTime::<Utc>::from_timestamp(timestamp, 0)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Micros, true))
    }
}

fn format_timestamp_from_ms(timestamp_ms: i128) -> Option<String> {
    if timestamp_ms <= 0 || timestamp_ms > i64::MAX as i128 {
        return None;
    }
    format_timestamp(timestamp_ms as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

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

    #[test]
    fn copied_rollout_preserves_source_modified_time() {
        let temp_dir = make_temp_dir("codex-thread-sync-mtime-copy-test");
        let source_root = temp_dir.join("source");
        let target_root = temp_dir.join("target");
        let rollout_dir = source_root
            .join("sessions")
            .join("2026")
            .join("05")
            .join("23");
        fs::create_dir_all(&rollout_dir).expect("create source rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"openai\"}}\n{\"type\":\"event\"}\n",
        )
        .expect("write source rollout");
        let source_modified_at = UNIX_EPOCH + Duration::from_secs(1_710_000_000);
        fs::File::open(&rollout_path)
            .expect("open source rollout")
            .set_modified(source_modified_at)
            .expect("set source mtime");

        let snapshot = ThreadSnapshot {
            id: "s1".to_string(),
            rollout_path: rollout_path.clone(),
            rollout_actual_modified_at: Some(source_modified_at),
            rollout_modified_at: Some(source_modified_at),
            merged_rollout_content: None,
            session_index_entry: json!({"id":"s1"}),
            workspace_root: None,
            source_root: source_root.clone(),
            freshness: ThreadFreshness {
                activity_ms: 0,
                rollout_len: 0,
                rollout_modified_ms: 0,
            },
        };
        let target_path = target_root.join("sessions/2026/05/23/rollout-test.jsonl");

        copy_rollout_file_to_path(&snapshot, &target_path).expect("copy rollout");

        assert_eq!(
            fs::metadata(&target_path)
                .expect("target metadata")
                .modified()
                .expect("target mtime"),
            source_modified_at
        );
        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_rewrite_preserves_rollout_modified_time() {
        let temp_dir = make_temp_dir("codex-thread-sync-mtime-provider-test");
        let rollout_path = temp_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old\"}}\n{\"type\":\"event\"}\n",
        )
        .expect("write rollout");
        let original_modified_at = UNIX_EPOCH + Duration::from_secs(1_720_000_000);
        fs::File::open(&rollout_path)
            .expect("open rollout")
            .set_modified(original_modified_at)
            .expect("set rollout mtime");

        rewrite_rollout_provider_for_target(&rollout_path, "relay").expect("rewrite provider");

        let content = fs::read_to_string(&rollout_path).expect("read rollout");
        assert!(content.contains("\"model_provider\":\"relay\""));
        assert_eq!(
            fs::metadata(&rollout_path)
                .expect("rollout metadata")
                .modified()
                .expect("rollout mtime"),
            original_modified_at
        );
        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn normalize_workspace_root_preserves_posix_paths() {
        assert_eq!(
            normalize_workspace_root("/Users/demo/project/").as_deref(),
            Some("/Users/demo/project")
        );
    }

    #[test]
    fn normalize_workspace_root_normalizes_windows_paths() {
        assert_eq!(
            normalize_workspace_root(r"\\?\C:\Users\demo\project\").as_deref(),
            Some(r"C:\Users\demo\project")
        );
        assert_eq!(
            normalize_workspace_root("C:/Users/demo/project/").as_deref(),
            Some(r"C:\Users\demo\project")
        );
    }
}
