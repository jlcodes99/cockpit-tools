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
const ELECTRON_PERSISTED_ATOM_STATE_KEY: &str = "electron-persisted-atom-state";
const PROJECT_ORDER_KEY: &str = "project-order";
const SAVED_WORKSPACE_ROOTS_KEY: &str = "electron-saved-workspace-roots";
const PROJECTLESS_THREAD_IDS_KEY: &str = "projectless-thread-ids";
const SIDEBAR_CHAT_THREAD_ORDER_KEY: &str = "sidebar-chat-thread-order";
const SIDEBAR_PROJECT_THREAD_ORDERS_KEY: &str = "sidebar-project-thread-orders";
const THREAD_WORKSPACE_ROOT_HINTS_KEY: &str = "thread-workspace-root-hints";
const THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY: &str = "thread-projectless-output-directories";
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
    archived: bool,
    freshness: ThreadFreshness,
}

#[derive(Debug, Clone)]
pub(crate) struct CodexSidebarThreadEntry {
    pub session_id: String,
    pub workspace_root: Option<String>,
    pub archived: bool,
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

        let sidebar_state_repair_count = count_sidebar_global_state_repairs_for_snapshots(
            &instance.data_dir,
            &expected_snapshots,
        )?;
        let repairs_project_index = sidebar_state_repair_count > 0;

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
            "会话内容已是最新，已修复 {} 个实例的 Codex 侧栏状态",
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
        mutated_instance_count,
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
                archived: dir_name == "archived_sessions",
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

    let mut should_rebuild_metadata = false;
    if !plan_items.is_empty() {
        let snapshots = plan_items
            .iter()
            .map(|item| item.snapshot.clone())
            .collect::<Vec<_>>();
        upsert_session_index_entries(&target.data_dir, &snapshots)?;
        should_rebuild_metadata = true;
    }
    if update_global_state_thread_workspaces(&target.data_dir, workspace_snapshots)? {
        should_rebuild_metadata = true;
    }
    let metadata_rebuild_failed = should_rebuild_metadata && !try_rebuild_thread_metadata(target);
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
    repaired_instance_count: usize,
) -> String {
    if failed_instance_count == 0 || repaired_instance_count == 0 {
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

fn normalize_workspace_roots(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for value in values {
        let Some(root) = normalize_workspace_root(value) else {
            continue;
        };
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }
    roots
}

pub(crate) fn count_missing_project_index_workspace_roots(
    root_dir: &Path,
    workspace_roots: &[String],
) -> Result<usize, String> {
    let roots = normalize_workspace_roots(workspace_roots);
    Ok(find_missing_workspace_roots(root_dir, &roots)?.len())
}

pub(crate) fn repair_project_index_workspace_roots(
    root_dir: &Path,
    workspace_roots: &[String],
) -> Result<bool, String> {
    let roots = normalize_workspace_roots(workspace_roots);
    update_global_state_workspace_roots(root_dir, &roots)
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

fn read_global_state_file(path: &Path) -> Option<JsonValue> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<JsonValue>(&raw).ok()
}

fn read_global_state_history(root_dir: &Path, current: &JsonValue) -> Vec<JsonValue> {
    let mut candidates = Vec::<(SystemTime, JsonValue)>::new();
    let entries = match fs::read_dir(root_dir) {
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
        let direct = path.join(GLOBAL_STATE_FILE);
        let nested = path.join("files").join(GLOBAL_STATE_FILE);
        for state_path in [direct, nested] {
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

fn global_state_workspace_array_contains(
    object: &serde_json::Map<String, JsonValue>,
    key: &str,
    workspace: &str,
) -> bool {
    if let Some(atom_state) = object
        .get(ELECTRON_PERSISTED_ATOM_STATE_KEY)
        .and_then(JsonValue::as_object)
    {
        return global_state_array_contains(atom_state, key, workspace);
    }
    global_state_array_contains(object, key, workspace)
}

fn find_missing_thread_workspace_roots(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<Vec<String>, String> {
    let roots = collect_thread_workspace_roots(snapshots);
    find_missing_workspace_roots(root_dir, &roots)
}

fn find_missing_workspace_roots(root_dir: &Path, roots: &[String]) -> Result<Vec<String>, String> {
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let value = read_global_state(root_dir)?;
    let Some(object) = value.as_object() else {
        return Ok(roots.to_vec());
    };

    Ok(roots
        .iter()
        .filter(|root| {
            !global_state_workspace_array_contains(object, PROJECT_ORDER_KEY, root)
                || !global_state_workspace_array_contains(object, SAVED_WORKSPACE_ROOTS_KEY, root)
        })
        .cloned()
        .collect())
}

fn update_global_state_thread_workspaces(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<bool, String> {
    let entries = sidebar_entries_from_snapshots(snapshots);
    repair_sidebar_global_state_for_threads(root_dir, &entries).map(|changed| changed > 0)
}

fn count_sidebar_global_state_repairs_for_snapshots(
    root_dir: &Path,
    snapshots: &[ThreadSnapshot],
) -> Result<usize, String> {
    let entries = sidebar_entries_from_snapshots(snapshots);
    count_missing_sidebar_global_state_for_threads(root_dir, &entries)
}

fn sidebar_entries_from_snapshots(snapshots: &[ThreadSnapshot]) -> Vec<CodexSidebarThreadEntry> {
    snapshots
        .iter()
        .map(|snapshot| CodexSidebarThreadEntry {
            session_id: snapshot.id.clone(),
            workspace_root: snapshot_workspace_root(snapshot),
            archived: snapshot.archived,
        })
        .collect::<Vec<_>>()
}

fn update_global_state_workspace_roots(root_dir: &Path, roots: &[String]) -> Result<bool, String> {
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
    changed |= merge_string_array(object, PROJECT_ORDER_KEY, &roots);
    changed |= merge_string_array(object, SAVED_WORKSPACE_ROOTS_KEY, &roots);
    let atom_state = ensure_object_child(object, ELECTRON_PERSISTED_ATOM_STATE_KEY)?;
    changed |= merge_string_array(atom_state, PROJECT_ORDER_KEY, &roots);
    changed |= merge_string_array(atom_state, SAVED_WORKSPACE_ROOTS_KEY, &roots);

    if changed {
        let serialized = serde_json::to_string_pretty(&value)
            .map_err(|error| format!("序列化全局状态失败: {}", error))?;
        fs::write(&path, format!("{}\n", serialized))
            .map_err(|error| format!("写入全局状态失败 ({}): {}", path.display(), error))?;
    }

    Ok(changed)
}

fn ensure_object_child<'a>(
    object: &'a mut serde_json::Map<String, JsonValue>,
    key: &str,
) -> Result<&'a mut serde_json::Map<String, JsonValue>, String> {
    let needs_object = !object.get(key).is_some_and(JsonValue::is_object);
    if needs_object {
        object.insert(key.to_string(), json!({}));
    }
    object
        .get_mut(key)
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| format!("全局状态字段 {} 格式无效", key))
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

fn normalized_project_roots_from_state(value: &JsonValue) -> Vec<String> {
    normalize_workspace_roots(&json_string_array(value, PROJECT_ORDER_KEY))
}

fn projectless_output_parent_roots(value: &JsonValue) -> HashSet<String> {
    value
        .get(THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY)
        .and_then(JsonValue::as_object)
        .map(|object| {
            object
                .values()
                .filter_map(JsonValue::as_str)
                .filter_map(|output_dir| {
                    let output_path = Path::new(output_dir);
                    let parent = match output_path.file_name().and_then(|name| name.to_str()) {
                        Some(name) if name.eq_ignore_ascii_case("outputs") => output_path.parent(),
                        _ => Some(output_path),
                    }?;
                    normalize_workspace_root(&parent.to_string_lossy())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn removed_project_roots_from_semantic_state(value: &JsonValue) -> HashSet<String> {
    let visible_roots = normalized_project_roots_from_state(value)
        .into_iter()
        .collect::<HashSet<_>>();
    value
        .get(SIDEBAR_PROJECT_THREAD_ORDERS_KEY)
        .and_then(JsonValue::as_object)
        .map(|object| {
            object
                .keys()
                .filter_map(|root| normalize_workspace_root(root))
                .filter(|root| !visible_roots.contains(root))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default()
}

fn state_sidebar_semantic_score(value: &JsonValue) -> usize {
    [
        PROJECTLESS_THREAD_IDS_KEY,
        SIDEBAR_CHAT_THREAD_ORDER_KEY,
        SIDEBAR_PROJECT_THREAD_ORDERS_KEY,
        THREAD_WORKSPACE_ROOT_HINTS_KEY,
        THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY,
    ]
    .iter()
    .filter(|key| value.get(**key).is_some())
    .count()
}

fn projectless_project_order_pollution_count(value: &JsonValue) -> usize {
    let projectless_roots = projectless_output_parent_roots(value);
    if projectless_roots.is_empty() {
        return 0;
    }
    normalized_project_roots_from_state(value)
        .iter()
        .filter(|root| projectless_roots.contains(*root))
        .count()
}

fn select_sidebar_semantic_state(history: &[JsonValue]) -> Option<&JsonValue> {
    history
        .iter()
        .filter(|value| state_sidebar_semantic_score(value) > 0)
        .max_by(|left, right| {
            let left_pollution = projectless_project_order_pollution_count(left);
            let right_pollution = projectless_project_order_pollution_count(right);
            right_pollution.cmp(&left_pollution).then_with(|| {
                state_sidebar_semantic_score(left).cmp(&state_sidebar_semantic_score(right))
            })
        })
}

fn normalized_root_key(value: &str) -> String {
    normalize_workspace_root(value).unwrap_or_else(|| value.trim().to_string())
}

fn active_sidebar_entries(entries: &[CodexSidebarThreadEntry]) -> Vec<CodexSidebarThreadEntry> {
    entries
        .iter()
        .filter(|entry| !entry.archived && !entry.session_id.trim().is_empty())
        .cloned()
        .collect()
}

fn filtered_active_id_array(values: Vec<String>, active_ids: &HashSet<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        if !active_ids.contains(&value) || !seen.insert(value.clone()) {
            continue;
        }
        result.push(value);
    }
    result
}

fn visible_project_roots_for_sidebar_repair(
    current: &JsonValue,
    semantic_state: Option<&JsonValue>,
    entries: &[CodexSidebarThreadEntry],
    projectless_roots: &HashSet<String>,
    projectless_ids: &HashSet<String>,
) -> Vec<String> {
    let semantic_roots = semantic_state
        .map(normalized_project_roots_from_state)
        .unwrap_or_default();
    let removed_roots = semantic_state
        .map(removed_project_roots_from_semantic_state)
        .unwrap_or_default();
    let current_roots = normalized_project_roots_from_state(current);
    let mut roots = if current_roots.is_empty() {
        semantic_roots.clone()
    } else {
        current_roots
    };

    roots.retain(|root| !projectless_roots.contains(root) && !removed_roots.contains(root));
    roots = normalize_workspace_roots(&roots);
    let mut seen = roots.iter().cloned().collect::<HashSet<_>>();
    for root in semantic_roots {
        if projectless_roots.contains(&root) || removed_roots.contains(&root) {
            continue;
        }
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }
    if !roots.is_empty() {
        return roots;
    }

    let mut seen = HashSet::new();
    let mut fallback = Vec::new();
    for entry in entries {
        if projectless_ids.contains(&entry.session_id) {
            continue;
        }
        let Some(root) = entry
            .workspace_root
            .as_deref()
            .and_then(normalize_workspace_root)
        else {
            continue;
        };
        if projectless_roots.contains(&root) {
            continue;
        }
        if seen.insert(root.clone()) {
            fallback.push(root);
        }
    }
    fallback
}

fn set_json_string_array(
    object: &mut serde_json::Map<String, JsonValue>,
    key: &str,
    values: &[String],
) -> bool {
    let next = JsonValue::Array(values.iter().cloned().map(JsonValue::String).collect());
    if object.get(key) == Some(&next) {
        return false;
    }
    object.insert(key.to_string(), next);
    true
}

fn set_project_root_arrays(
    object: &mut serde_json::Map<String, JsonValue>,
    roots: &[String],
) -> Result<usize, String> {
    let mut changed = 0usize;
    if set_json_string_array(object, PROJECT_ORDER_KEY, roots) {
        changed += 1;
    }
    if set_json_string_array(object, SAVED_WORKSPACE_ROOTS_KEY, roots) {
        changed += 1;
    }
    let atom_state = ensure_object_child(object, ELECTRON_PERSISTED_ATOM_STATE_KEY)?;
    if set_json_string_array(atom_state, PROJECT_ORDER_KEY, roots) {
        changed += 1;
    }
    if set_json_string_array(atom_state, SAVED_WORKSPACE_ROOTS_KEY, roots) {
        changed += 1;
    }
    Ok(changed)
}

fn set_string_map(
    object: &mut serde_json::Map<String, JsonValue>,
    key: &str,
    values: &HashMap<String, String>,
) -> bool {
    let mut keys = values.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut next = serde_json::Map::new();
    for map_key in keys {
        if let Some(value) = values.get(&map_key) {
            next.insert(map_key, JsonValue::String(value.clone()));
        }
    }
    let next = JsonValue::Object(next);
    if object.get(key) == Some(&next) {
        return false;
    }
    object.insert(key.to_string(), next);
    true
}

fn set_project_thread_orders(
    object: &mut serde_json::Map<String, JsonValue>,
    orders: &HashMap<String, Vec<String>>,
) -> bool {
    let mut roots = orders.keys().cloned().collect::<Vec<_>>();
    roots.sort();
    let mut next = serde_json::Map::new();
    for root in roots {
        if let Some(ids) = orders.get(&root) {
            if ids.is_empty() {
                continue;
            }
            next.insert(
                root,
                JsonValue::Array(ids.iter().cloned().map(JsonValue::String).collect()),
            );
        }
    }
    let next = JsonValue::Object(next);
    if object.get(SIDEBAR_PROJECT_THREAD_ORDERS_KEY) == Some(&next) {
        return false;
    }
    object.insert(SIDEBAR_PROJECT_THREAD_ORDERS_KEY.to_string(), next);
    true
}

fn sidebar_state_string_map(
    value: Option<&JsonValue>,
    key: &str,
    active_ids: &HashSet<String>,
    allowed_ids: Option<&HashSet<String>>,
) -> HashMap<String, String> {
    value
        .and_then(|state| state.get(key))
        .and_then(JsonValue::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(item_key, item_value)| {
                    if !active_ids.contains(item_key) {
                        return None;
                    }
                    if allowed_ids.is_some_and(|ids| !ids.contains(item_key)) {
                        return None;
                    }
                    item_value
                        .as_str()
                        .map(|value| (item_key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn sidebar_project_thread_orders(
    semantic_state: Option<&JsonValue>,
    entries: &[CodexSidebarThreadEntry],
    active_ids: &HashSet<String>,
    projectless_ids: &HashSet<String>,
    visible_roots: &[String],
) -> HashMap<String, Vec<String>> {
    let mut orders = HashMap::<String, Vec<String>>::new();
    if let Some(object) = semantic_state
        .and_then(|state| state.get(SIDEBAR_PROJECT_THREAD_ORDERS_KEY))
        .and_then(JsonValue::as_object)
    {
        for (root, ids) in object {
            let Some(root) = normalize_workspace_root(root) else {
                continue;
            };
            let filtered = filtered_active_id_array(
                ids.as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(JsonValue::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
                active_ids,
            )
            .into_iter()
            .filter(|id| !projectless_ids.contains(id))
            .collect::<Vec<_>>();
            if !filtered.is_empty() {
                orders.insert(root, filtered);
            }
        }
    }

    let visible_root_set = visible_roots.iter().cloned().collect::<HashSet<_>>();
    for entry in entries {
        if projectless_ids.contains(&entry.session_id) {
            continue;
        }
        let Some(root) = entry
            .workspace_root
            .as_deref()
            .and_then(normalize_workspace_root)
        else {
            continue;
        };
        if !visible_root_set.contains(&root) && !orders.contains_key(&root) {
            continue;
        }
        let ids = orders.entry(root).or_default();
        if !ids.contains(&entry.session_id) {
            ids.push(entry.session_id.clone());
        }
    }

    orders
}

fn build_repaired_sidebar_global_state(
    root_dir: &Path,
    entries: &[CodexSidebarThreadEntry],
) -> Result<(JsonValue, usize), String> {
    let active_entries = active_sidebar_entries(entries);
    if active_entries.is_empty() {
        return Ok((read_global_state(root_dir)?, 0));
    }

    let mut current = read_global_state(root_dir)?;
    if !current.is_object() {
        current = json!({});
    }
    let history = read_global_state_history(root_dir, &current);
    let semantic_state = select_sidebar_semantic_state(&history);
    let active_ids = active_entries
        .iter()
        .map(|entry| entry.session_id.clone())
        .collect::<HashSet<_>>();

    let mut projectless_ids = filtered_active_id_array(
        semantic_state
            .map(|state| json_string_array(state, PROJECTLESS_THREAD_IDS_KEY))
            .unwrap_or_default(),
        &active_ids,
    )
    .into_iter()
    .collect::<HashSet<_>>();
    let output_dirs = sidebar_state_string_map(
        semantic_state,
        THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY,
        &active_ids,
        None,
    );
    projectless_ids.extend(output_dirs.keys().cloned());
    let projectless_roots = semantic_state
        .map(projectless_output_parent_roots)
        .unwrap_or_default();
    for entry in &active_entries {
        let Some(root) = entry
            .workspace_root
            .as_deref()
            .and_then(normalize_workspace_root)
        else {
            continue;
        };
        if projectless_roots.contains(&root) {
            projectless_ids.insert(entry.session_id.clone());
        }
    }

    let projectless_ids_vec = {
        let preferred = semantic_state
            .map(|state| json_string_array(state, PROJECTLESS_THREAD_IDS_KEY))
            .unwrap_or_default();
        let mut values = filtered_active_id_array(preferred, &active_ids);
        for id in &projectless_ids {
            if !values.contains(id) {
                values.push(id.clone());
            }
        }
        values
    };
    let projectless_id_set = projectless_ids_vec.iter().cloned().collect::<HashSet<_>>();
    let output_dirs = sidebar_state_string_map(
        semantic_state,
        THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY,
        &active_ids,
        Some(&projectless_id_set),
    );
    let workspace_hints = sidebar_state_string_map(
        semantic_state,
        THREAD_WORKSPACE_ROOT_HINTS_KEY,
        &active_ids,
        Some(&projectless_id_set),
    );
    let mut projectless_roots = semantic_state
        .map(projectless_output_parent_roots)
        .unwrap_or_default();
    projectless_roots.extend(output_dirs.values().filter_map(|output_dir| {
        let output_path = Path::new(output_dir);
        let parent = match output_path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.eq_ignore_ascii_case("outputs") => output_path.parent(),
            _ => Some(output_path),
        }?;
        normalize_workspace_root(&parent.to_string_lossy())
    }));

    let visible_roots = visible_project_roots_for_sidebar_repair(
        &current,
        semantic_state,
        &active_entries,
        &projectless_roots,
        &projectless_id_set,
    );
    let mut chat_order = filtered_active_id_array(
        semantic_state
            .map(|state| json_string_array(state, SIDEBAR_CHAT_THREAD_ORDER_KEY))
            .unwrap_or_default(),
        &active_ids,
    );
    for entry in &active_entries {
        if !chat_order.contains(&entry.session_id) {
            chat_order.push(entry.session_id.clone());
        }
    }
    let project_thread_orders = sidebar_project_thread_orders(
        semantic_state,
        &active_entries,
        &active_ids,
        &projectless_id_set,
        &visible_roots,
    );

    let Some(object) = current.as_object_mut() else {
        return Err("全局状态文件格式无效".to_string());
    };
    let mut changed = set_project_root_arrays(object, &visible_roots)?;
    if set_json_string_array(object, PROJECTLESS_THREAD_IDS_KEY, &projectless_ids_vec) {
        changed += 1;
    }
    if set_json_string_array(object, SIDEBAR_CHAT_THREAD_ORDER_KEY, &chat_order) {
        changed += 1;
    }
    if set_project_thread_orders(object, &project_thread_orders) {
        changed += 1;
    }
    if set_string_map(object, THREAD_WORKSPACE_ROOT_HINTS_KEY, &workspace_hints) {
        changed += 1;
    }
    if set_string_map(
        object,
        THREAD_PROJECTLESS_OUTPUT_DIRECTORIES_KEY,
        &output_dirs,
    ) {
        changed += 1;
    }

    Ok((current, changed))
}

pub(crate) fn count_missing_sidebar_global_state_for_threads(
    root_dir: &Path,
    entries: &[CodexSidebarThreadEntry],
) -> Result<usize, String> {
    build_repaired_sidebar_global_state(root_dir, entries).map(|(_, changed)| changed)
}

pub(crate) fn repair_sidebar_global_state_for_threads(
    root_dir: &Path,
    entries: &[CodexSidebarThreadEntry],
) -> Result<usize, String> {
    let (value, changed) = build_repaired_sidebar_global_state(root_dir, entries)?;
    if changed == 0 {
        return Ok(0);
    }

    let path = root_dir.join(GLOBAL_STATE_FILE);
    let serialized = serde_json::to_string_pretty(&value)
        .map_err(|error| format!("序列化全局状态失败: {}", error))?;
    fs::write(&path, format!("{}\n", serialized))
        .map_err(|error| format!("写入全局状态失败 ({}): {}", path.display(), error))?;
    Ok(changed)
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
            archived: false,
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

    #[test]
    fn project_index_scan_accepts_official_atom_state_workspaces() {
        let data_dir = make_temp_dir("codex-thread-project-index-scan-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\project"],"electron-saved-workspace-roots":["C:\\Users\\demo\\project"]}}"#,
        )
        .expect("write global state");
        let snapshots = vec![ThreadSnapshot {
            id: "thread-1".to_string(),
            rollout_path: data_dir.join("sessions/rollout-test.jsonl"),
            rollout_actual_modified_at: None,
            rollout_modified_at: None,
            merged_rollout_content: None,
            session_index_entry: json!({"id":"thread-1"}),
            workspace_root: Some(r"\\?\C:\Users\demo\project\".to_string()),
            source_root: data_dir.clone(),
            archived: false,
            freshness: ThreadFreshness {
                activity_ms: 0,
                rollout_len: 0,
                rollout_modified_ms: 0,
            },
        }];

        let missing =
            find_missing_thread_workspace_roots(&data_dir, &snapshots).expect("scan project index");

        assert!(missing.is_empty());
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn project_index_repair_writes_official_atom_state_workspaces() {
        let data_dir = make_temp_dir("codex-thread-project-index-write-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"electron-persisted-atom-state":{"sidebar-collapsed-sections-v1":{"threads":false}},"other":1}"#,
        )
        .expect("write global state");
        let snapshots = vec![ThreadSnapshot {
            id: "thread-1".to_string(),
            rollout_path: data_dir.join("sessions/rollout-test.jsonl"),
            rollout_actual_modified_at: None,
            rollout_modified_at: None,
            merged_rollout_content: None,
            session_index_entry: json!({"id":"thread-1"}),
            workspace_root: Some(r"\\?\C:\Users\demo\project\".to_string()),
            source_root: data_dir.clone(),
            archived: false,
            freshness: ThreadFreshness {
                activity_ms: 0,
                rollout_len: 0,
                rollout_modified_ms: 0,
            },
        }];

        let changed = update_global_state_thread_workspaces(&data_dir, &snapshots)
            .expect("repair project index");

        assert!(changed);
        let state: JsonValue = serde_json::from_str(
            &fs::read_to_string(data_dir.join(GLOBAL_STATE_FILE)).expect("read global state"),
        )
        .expect("parse global state");
        assert_eq!(state["other"], 1);
        assert_eq!(
            state["electron-persisted-atom-state"]["sidebar-collapsed-sections-v1"]["threads"],
            false
        );
        assert_eq!(
            state["electron-persisted-atom-state"]["project-order"][0],
            r"C:\Users\demo\project"
        );
        assert_eq!(
            state["electron-persisted-atom-state"]["electron-saved-workspace-roots"][0],
            r"C:\Users\demo\project"
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn project_index_scan_treats_top_level_only_as_missing_when_atom_state_exists() {
        let data_dir = make_temp_dir("codex-thread-project-index-legacy-only-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\project"],"electron-saved-workspace-roots":["C:\\Users\\demo\\project"],"electron-persisted-atom-state":{"sidebar-collapsed-sections-v1":{"threads":false}}}"#,
        )
        .expect("write global state");
        let snapshots = vec![ThreadSnapshot {
            id: "thread-1".to_string(),
            rollout_path: data_dir.join("sessions/rollout-test.jsonl"),
            rollout_actual_modified_at: None,
            rollout_modified_at: None,
            merged_rollout_content: None,
            session_index_entry: json!({"id":"thread-1"}),
            workspace_root: Some(r"C:\Users\demo\project".to_string()),
            source_root: data_dir.clone(),
            archived: false,
            freshness: ThreadFreshness {
                activity_ms: 0,
                rollout_len: 0,
                rollout_modified_ms: 0,
            },
        }];

        let missing =
            find_missing_thread_workspace_roots(&data_dir, &snapshots).expect("scan project index");

        assert_eq!(missing, vec![r"C:\Users\demo\project".to_string()]);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidebar_state_scan_detects_missing_semantics_when_roots_are_already_present() {
        let data_dir = make_temp_dir("codex-thread-sidebar-scan-missing-semantics-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"]}}"#,
        )
        .expect("write over-restored global state");
        let backup_dir = data_dir.join("backup-20260613-170559-global-state-sidebar-repair");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\real"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real"],"projectless-thread-ids":["projectless-thread"],"sidebar-chat-thread-order":["projectless-thread"],"sidebar-project-thread-orders":{"C:\\Users\\demo\\real":["project-thread"]},"thread-workspace-root-hints":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex"},"thread-projectless-output-directories":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat\\outputs"},"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\real"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real"]}}"#,
        )
        .expect("write sidebar backup state");
        let snapshots = vec![
            ThreadSnapshot {
                id: "project-thread".to_string(),
                rollout_path: data_dir.join("sessions/rollout-project.jsonl"),
                rollout_actual_modified_at: None,
                rollout_modified_at: None,
                merged_rollout_content: None,
                session_index_entry: json!({"id":"project-thread"}),
                workspace_root: Some(r"C:\Users\demo\real".to_string()),
                source_root: data_dir.clone(),
                archived: false,
                freshness: ThreadFreshness {
                    activity_ms: 0,
                    rollout_len: 0,
                    rollout_modified_ms: 0,
                },
            },
            ThreadSnapshot {
                id: "projectless-thread".to_string(),
                rollout_path: data_dir.join("sessions/rollout-projectless.jsonl"),
                rollout_actual_modified_at: None,
                rollout_modified_at: None,
                merged_rollout_content: None,
                session_index_entry: json!({"id":"projectless-thread"}),
                workspace_root: Some(
                    r"C:\Users\demo\Documents\Codex\2026-06-13\new-chat".to_string(),
                ),
                source_root: data_dir.clone(),
                archived: false,
                freshness: ThreadFreshness {
                    activity_ms: 0,
                    rollout_len: 0,
                    rollout_modified_ms: 0,
                },
            },
        ];

        let legacy_missing =
            find_missing_thread_workspace_roots(&data_dir, &snapshots).expect("legacy scan");
        let semantic_missing =
            count_sidebar_global_state_repairs_for_snapshots(&data_dir, &snapshots)
                .expect("semantic scan");

        assert!(legacy_missing.is_empty());
        assert!(semantic_missing > 0);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidebar_state_repair_restores_projectless_state_without_showing_generated_roots() {
        let data_dir = make_temp_dir("codex-thread-sidebar-projectless-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real","C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat"]}}"#,
        )
        .expect("write over-restored global state");
        let backup_dir = data_dir.join("backup-20260613-170559-global-state-sidebar-repair");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\real"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real"],"projectless-thread-ids":["projectless-thread"],"sidebar-chat-thread-order":["projectless-thread"],"sidebar-project-thread-orders":{"C:\\Users\\demo\\real":["project-thread"]},"thread-workspace-root-hints":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex"},"thread-projectless-output-directories":{"projectless-thread":"C:\\Users\\demo\\Documents\\Codex\\2026-06-13\\new-chat\\outputs"},"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\real"],"electron-saved-workspace-roots":["C:\\Users\\demo\\real"]}}"#,
        )
        .expect("write sidebar backup state");
        let entries = vec![
            CodexSidebarThreadEntry {
                session_id: "project-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\real".to_string()),
                archived: false,
            },
            CodexSidebarThreadEntry {
                session_id: "projectless-thread".to_string(),
                workspace_root: Some(
                    r"C:\Users\demo\Documents\Codex\2026-06-13\new-chat".to_string(),
                ),
                archived: false,
            },
        ];

        let repaired =
            repair_sidebar_global_state_for_threads(&data_dir, &entries).expect("repair sidebar");

        assert!(repaired > 0);
        let state: JsonValue = serde_json::from_str(
            &fs::read_to_string(data_dir.join(GLOBAL_STATE_FILE)).expect("read repaired state"),
        )
        .expect("parse repaired state");
        assert_eq!(state[PROJECT_ORDER_KEY], json!([r"C:\Users\demo\real"]));
        assert_eq!(
            state["electron-persisted-atom-state"][PROJECT_ORDER_KEY],
            json!([r"C:\Users\demo\real"])
        );
        assert_eq!(
            state["projectless-thread-ids"],
            json!(["projectless-thread"])
        );
        assert_eq!(
            state["thread-projectless-output-directories"]["projectless-thread"],
            r"C:\Users\demo\Documents\Codex\2026-06-13\new-chat\outputs"
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidebar_state_repair_preserves_removed_project_order_without_readding_project_root() {
        let data_dir = make_temp_dir("codex-thread-sidebar-removed-project-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\visible","C:\\Users\\demo\\removed"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible","C:\\Users\\demo\\removed"],"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\visible","C:\\Users\\demo\\removed"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible","C:\\Users\\demo\\removed"]}}"#,
        )
        .expect("write over-restored global state");
        let backup_dir = data_dir.join("backup-20260613-170559-global-state-sidebar-repair");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\visible"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible"],"sidebar-project-thread-orders":{"C:\\Users\\demo\\visible":["visible-thread"],"C:\\Users\\demo\\removed":["removed-thread"]},"sidebar-chat-thread-order":["visible-thread","removed-thread"],"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\visible"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible"]}}"#,
        )
        .expect("write sidebar backup state");
        let entries = vec![
            CodexSidebarThreadEntry {
                session_id: "visible-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\visible".to_string()),
                archived: false,
            },
            CodexSidebarThreadEntry {
                session_id: "removed-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\removed".to_string()),
                archived: false,
            },
        ];

        repair_sidebar_global_state_for_threads(&data_dir, &entries).expect("repair sidebar");

        let state: JsonValue = serde_json::from_str(
            &fs::read_to_string(data_dir.join(GLOBAL_STATE_FILE)).expect("read repaired state"),
        )
        .expect("parse repaired state");
        assert_eq!(state[PROJECT_ORDER_KEY], json!([r"C:\Users\demo\visible"]));
        assert_eq!(
            state["sidebar-project-thread-orders"][r"C:\Users\demo\removed"],
            json!(["removed-thread"])
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidebar_state_repair_preserves_current_roots_not_known_removed_or_projectless() {
        let data_dir = make_temp_dir("codex-thread-sidebar-preserve-current-roots-test");
        fs::write(
            data_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\visible","C:\\Users\\demo\\new-visible","C:\\Users\\demo\\generated-chat","C:\\Users\\demo\\removed"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible","C:\\Users\\demo\\new-visible","C:\\Users\\demo\\generated-chat","C:\\Users\\demo\\removed"],"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\visible","C:\\Users\\demo\\new-visible","C:\\Users\\demo\\generated-chat","C:\\Users\\demo\\removed"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible","C:\\Users\\demo\\new-visible","C:\\Users\\demo\\generated-chat","C:\\Users\\demo\\removed"]}}"#,
        )
        .expect("write current state with restored roots");
        let backup_dir = data_dir.join("backup-20260613-170559-global-state-sidebar-repair");
        fs::create_dir_all(&backup_dir).expect("create backup dir");
        fs::write(
            backup_dir.join(GLOBAL_STATE_FILE),
            r#"{"project-order":["C:\\Users\\demo\\visible"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible"],"projectless-thread-ids":["projectless-thread"],"sidebar-project-thread-orders":{"C:\\Users\\demo\\visible":["visible-thread"],"C:\\Users\\demo\\removed":["removed-thread"]},"thread-projectless-output-directories":{"projectless-thread":"C:\\Users\\demo\\generated-chat\\outputs"},"electron-persisted-atom-state":{"project-order":["C:\\Users\\demo\\visible"],"electron-saved-workspace-roots":["C:\\Users\\demo\\visible"]}}"#,
        )
        .expect("write semantic backup state");
        let entries = vec![
            CodexSidebarThreadEntry {
                session_id: "visible-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\visible".to_string()),
                archived: false,
            },
            CodexSidebarThreadEntry {
                session_id: "new-visible-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\new-visible".to_string()),
                archived: false,
            },
            CodexSidebarThreadEntry {
                session_id: "projectless-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\generated-chat".to_string()),
                archived: false,
            },
            CodexSidebarThreadEntry {
                session_id: "removed-thread".to_string(),
                workspace_root: Some(r"C:\Users\demo\removed".to_string()),
                archived: false,
            },
        ];

        repair_sidebar_global_state_for_threads(&data_dir, &entries).expect("repair sidebar");

        let state: JsonValue = serde_json::from_str(
            &fs::read_to_string(data_dir.join(GLOBAL_STATE_FILE)).expect("read repaired state"),
        )
        .expect("parse repaired state");
        assert_eq!(
            state[PROJECT_ORDER_KEY],
            json!([r"C:\Users\demo\visible", r"C:\Users\demo\new-visible"])
        );
        assert_eq!(
            state[SIDEBAR_PROJECT_THREAD_ORDERS_KEY][r"C:\Users\demo\new-visible"],
            json!(["new-visible-thread"])
        );
        assert_eq!(
            state[SIDEBAR_PROJECT_THREAD_ORDERS_KEY][r"C:\Users\demo\removed"],
            json!(["removed-thread"])
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }
}
