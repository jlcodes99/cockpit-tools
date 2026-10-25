use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};

use crate::modules;

#[path = "codex_session_rollout_reader.rs"]
mod rollout_reader;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const BACKUP_FILE_NAMES: [&str; 2] = [SESSION_INDEX_FILE, GLOBAL_STATE_FILE];
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
static THREAD_SYNC_OPERATION: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

pub fn sync_threads_across_instances() -> Result<CodexInstanceThreadSyncSummary, String> {
    let _operation = THREAD_SYNC_OPERATION
        .try_lock()
        .map_err(|_| thread_sync_error("codex.instance.errors.threadSyncBusy"))?;
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
    let entries = modules::process::collect_codex_process_entries();
    run_thread_sync_when_stopped(&instances, &entries, sync_threads_across_instances)
}

fn run_thread_sync_when_stopped<T>(
    instances: &[CodexSyncInstance],
    process_entries: &[(u32, Option<String>)],
    sync: impl FnOnce() -> Result<T, String>,
) -> Result<Option<T>, String> {
    if instances.len() < 2 {
        return Ok(None);
    }

    if instances
        .iter()
        .any(|instance| is_instance_running(instance, process_entries))
    {
        return Ok(None);
    }

    sync().map(Some)
}

pub fn sync_sessions_to_instance(
    session_ids: Vec<String>,
    target_instance_id: String,
) -> Result<CodexInstanceTargetThreadSyncSummary, String> {
    let _operation = THREAD_SYNC_OPERATION
        .try_lock()
        .map_err(|_| thread_sync_error("codex.instance.errors.threadSyncBusy"))?;
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
    let current_heads = rollout_reader::current_heads(&instance.data_dir)?;
    let display_context =
        modules::codex_session_display::SessionDisplayContext::load(&instance.data_dir);
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
            // A prior physical segment may have the same session id after a
            // revert. Select the official current head, never merge its ancestor.
            if current_heads
                .get(&id)
                .is_some_and(|head| !rollout_reader::same_storage_path(head, &rollout_path))
            {
                continue;
            }
            let freshness = build_thread_freshness(session_index_map.get(&id), &rollout_path);
            let index_title = session_index_map.get(&id).and_then(session_index_title);
            let title = display_context
                .resolve_title(&id, index_title.as_deref())
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
        if file_name.starts_with("rollout-")
            && (file_name.ends_with(".jsonl") || file_name.ends_with(".jsonl.zst"))
        {
            result.push(path);
        }
    }

    result.sort();
    Ok(result)
}

fn read_rollout_session_meta(path: &Path) -> Result<Option<JsonValue>, String> {
    rollout_reader::metadata(path)
        .map_err(|error| format!("读取 rollout 文件失败 ({}): {}", path.display(), error))
}

fn rollout_storage_compressed(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    if rollout_reader::is_compressed(path) {
        return Ok(true);
    }
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut magic = [0; 4];
    let count = file.read(&mut magic).map_err(|error| error.to_string())?;
    Ok(count == magic.len() && magic == [0x28, 0xb5, 0x2f, 0xfd])
}

fn read_rollout_content(path: &Path) -> Result<String, String> {
    if !rollout_storage_compressed(path)? {
        return fs::read_to_string(path).map_err(|error| error.to_string());
    }
    // Decode a stable, bounded snapshot into private temporary storage. Never
    // decompress in place or serialize parsed JSON: offsets address decoded
    // bytes, including original whitespace, CRLF and UTF-8.
    let staging = tempfile::tempdir().map_err(|error| error.to_string())?;
    let materialized = staging.path().join("rollout.jsonl");
    rollout_reader::materialize_bounded(path, &materialized).map_err(|error| error.to_string())?;
    fs::read_to_string(materialized).map_err(|error| error.to_string())
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
    sync_thread_plan_to_instance_with_rebuild(
        target,
        plan_items,
        workspace_snapshots,
        try_rebuild_thread_metadata,
    )
}

fn sync_thread_plan_to_instance_with_rebuild(
    target: &CodexSyncInstance,
    plan_items: &[ThreadSyncPlanItem],
    workspace_snapshots: &[ThreadSnapshot],
    rebuild: impl FnOnce(&CodexSyncInstance) -> bool,
) -> Result<ThreadSyncWriteResult, String> {
    // Validate the whole target plan before publishing any file or backup.
    let mut prepared = Vec::with_capacity(plan_items.len());
    for item in plan_items {
        let path = resolve_target_rollout_path(
            &item.snapshot,
            &target.data_dir,
            item.existing_rollout_path.as_deref(),
        )?;
        let bytes = rollout_snapshot_bytes(&item.snapshot)?;
        let action = validate_rollout_copy_target(&item.snapshot, &path, &bytes)?;
        if matches!(action, RolloutCopyAction::Append(_)) {
            ensure_append_target_stopped(target)?;
        }
        prepared.push((item, path, bytes, action));
    }
    let backup_dir = backup_instance_files(&target.data_dir)?;
    for (item, path, bytes, action) in prepared {
        if item.is_update {
            backup_existing_rollout_file(&backup_dir, &target.data_dir, &path, &item.snapshot.id)?;
        }
        copy_prepared_rollout(&item.snapshot, &path, &bytes, action, || {
            ensure_append_target_stopped(target)
        })?;
    }

    let mut metadata_rebuild_failed = false;
    if !plan_items.is_empty() {
        let snapshots = plan_items
            .iter()
            .map(|item| item.snapshot.clone())
            .collect::<Vec<_>>();
        upsert_session_index_entries(&target.data_dir, &snapshots)?;
        metadata_rebuild_failed = !rebuild(target);
    }
    update_global_state_thread_workspaces(&target.data_dir, workspace_snapshots)?;
    let _ = modules::backup_storage::prune_behavior_backups(
        "codex",
        &modules::backup_storage::scope_for_path(&target.data_dir),
    );
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

// A line-wise union destroys byte offsets and can silently discard continuation
// session_meta records. Only select an existing append-compatible byte stream.
fn merge_rollout_contents(snapshots: &[ThreadSnapshot]) -> Result<String, String> {
    let mut longest = String::new();
    for snapshot in snapshots {
        let content = read_rollout_content(&snapshot.rollout_path)?;
        if content.starts_with(&longest) {
            longest = content;
        } else if !longest.starts_with(&content) {
            return Err(
                "会话历史已分叉，无法在保留字节游标的情况下合并；请保留各实例原始历史".into(),
            );
        }
    }
    Ok(longest)
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

    read_rollout_content(&existing.rollout_path)
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
    let (rollout_modified_ms, stored_len) = rollout_file_metadata(rollout_path);
    let (rollout_activity_ms, rollout_len) = read_rollout_content(rollout_path)
        .map(|content| rollout_content_activity_and_len(&content))
        .unwrap_or((0, stored_len));
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
    let scope = modules::backup_storage::scope_for_path(data_dir);
    let backup_dir = modules::backup_storage::behavior_backup_dir(
        "codex",
        &scope,
        &format!(
            "{}-instance-thread-sync",
            Utc::now().format("%Y%m%d-%H%M%S")
        ),
    )?;
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
    let normalized_desktop_path =
        modules::codex_session_visibility::to_desktop_workspace_path(value)?;
    let value = normalized_desktop_path.as_str();

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
    let target = target_root.join(relative_path);
    // New copies are standalone decoded streams. Retain .jsonl, never publish
    // plaintext under .zst or compressed bytes under .jsonl.
    Ok(if rollout_reader::is_compressed(&target) {
        target.with_extension("")
    } else {
        target
    })
}

fn rollout_snapshot_bytes(snapshot: &ThreadSnapshot) -> Result<Vec<u8>, String> {
    match &snapshot.merged_rollout_content {
        Some(content) => Ok(content.as_bytes().to_vec()),
        None => read_rollout_content(&snapshot.rollout_path).map(String::into_bytes),
    }
}

enum RolloutCopyAction {
    Unchanged,
    Create,
    Append(Vec<u8>),
}

fn thread_sync_error(key: &str) -> String {
    modules::i18n::translate(&modules::config::get_user_config().language, key, &[])
}

fn ensure_append_target_stopped(target: &CodexSyncInstance) -> Result<(), String> {
    let entries = modules::process::collect_codex_process_entries();
    if is_instance_running(target, &entries) {
        return Err(thread_sync_error(
            "codex.instance.errors.threadSyncTargetRunning",
        ));
    }
    Ok(())
}

fn alternate_rollout_path(path: &Path) -> PathBuf {
    if rollout_reader::is_compressed(path) {
        path.with_extension("")
    } else {
        let mut name = path.as_os_str().to_os_string();
        name.push(".zst");
        PathBuf::from(name)
    }
}

fn rollout_conflict() -> String {
    "目标会话已有不同内容，无法安全覆盖字节寻址历史；请保留原始文件".into()
}

fn validate_rollout_copy_target(
    _snapshot: &ThreadSnapshot,
    target: &Path,
    bytes: &[u8],
) -> Result<RolloutCopyAction, String> {
    modules::codex_session_import_paths::ensure_standalone_rollout(bytes)?;
    // Treat plain/compressed names as the same physical head identity. Do not
    // create a second conflicting version beside a compressed destination.
    if fs::symlink_metadata(alternate_rollout_path(target)).is_ok() {
        return Err(rollout_conflict());
    }
    if fs::symlink_metadata(target).is_ok() {
        let compressed = rollout_storage_compressed(target)?;
        let existing = read_rollout_content(target)?.into_bytes();
        if existing == bytes {
            return Ok(RolloutCopyAction::Unchanged);
        }
        if !compressed && !existing.is_empty() && bytes.starts_with(&existing) {
            // Only plain targets admit a verbatim append. Never append decoded
            // text into a zstd frame or recompress/replace an existing head.
            return Ok(RolloutCopyAction::Append(existing));
        }
        return Err(rollout_conflict());
    }
    if rollout_reader::is_compressed(target) {
        return Err(rollout_conflict());
    }
    Ok(RolloutCopyAction::Create)
}

#[cfg(test)]
fn copy_rollout_file_to_path(
    snapshot: &ThreadSnapshot,
    target_path: &Path,
) -> Result<PathBuf, String> {
    let bytes = rollout_snapshot_bytes(snapshot)?;
    let action = validate_rollout_copy_target(snapshot, target_path, &bytes)?;
    // The low-level copy API has no instance context to authorize an append.
    copy_prepared_rollout(snapshot, target_path, &bytes, action, || {
        Err("追加会话前必须确认目标实例已停止".into())
    })
}

fn copy_prepared_rollout(
    snapshot: &ThreadSnapshot,
    target_path: &Path,
    bytes: &[u8],
    action: RolloutCopyAction,
    verify_stopped: impl Fn() -> Result<(), String>,
) -> Result<PathBuf, String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    match action {
        RolloutCopyAction::Unchanged => return Ok(target_path.to_path_buf()),
        RolloutCopyAction::Append(expected) => {
            verify_stopped()?;
            let metadata = fs::symlink_metadata(target_path).map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(thread_sync_error(
                    "codex.instance.errors.threadSyncTargetReplaced",
                ));
            }
            let mut options = fs::OpenOptions::new();
            options.read(true).append(true);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.share_mode(0);
            }
            let mut file = options.open(target_path).map_err(|e| e.to_string())?;
            fs2::FileExt::try_lock_exclusive(&file)
                .map_err(|_| thread_sync_error("codex.instance.errors.threadSyncFileBusy"))?;
            let mut current = Vec::new();
            file.read_to_end(&mut current).map_err(|e| e.to_string())?;
            if current != expected {
                return Err(thread_sync_error(
                    "codex.instance.errors.threadSyncTargetChanged",
                ));
            }
            verify_stopped()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let opened = file.metadata().map_err(|e| e.to_string())?;
                let current_path = fs::symlink_metadata(target_path).map_err(|e| e.to_string())?;
                if opened.dev() != current_path.dev()
                    || opened.ino() != current_path.ino()
                    || metadata.dev() != opened.dev()
                    || metadata.ino() != opened.ino()
                {
                    return Err(thread_sync_error(
                        "codex.instance.errors.threadSyncTargetReplaced",
                    ));
                }
            }
            // Process discovery can take time; revalidate after it, immediately
            // before the append, while retaining the exclusive file lock.
            file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            current.clear();
            file.read_to_end(&mut current).map_err(|e| e.to_string())?;
            if current != expected {
                return Err(thread_sync_error(
                    "codex.instance.errors.threadSyncTargetChanged",
                ));
            }
            // O_APPEND never replaces/truncates existing bytes. On an I/O failure
            // leave the written prefix and its backup intact; the next validated
            // attempt appends only the remaining source bytes, without duplication.
            file.write_all(&bytes[expected.len()..])
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            return Ok(target_path.to_path_buf());
        }
        RolloutCopyAction::Create => {}
    }
    let parent = target_path.parent().ok_or("无法解析目标会话目录")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let staging = parent.join(format!(
        ".cockpit-rollout-copy-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let result = (|| {
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .map_err(|e| e.to_string())?;
        output.write_all(bytes).map_err(|e| e.to_string())?;
        output.flush().map_err(|e| e.to_string())?;
        modules::codex_session_file_time::restore_modified_time(
            &staging,
            snapshot.rollout_modified_at,
        )?;
        if fs::symlink_metadata(alternate_rollout_path(target_path)).is_ok() {
            return Err(rollout_conflict());
        }
        // Same-volume hard-link publication fails if a concurrent writer created
        // the target. No rename/truncate can discard that writer's append.
        fs::hard_link(&staging, target_path).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_file(&staging);
    result?;
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
            normalize_workspace_root(r"\\?\UNC\server\share\project\").as_deref(),
            Some(r"\\server\share\project")
        );
        assert_eq!(
            normalize_workspace_root("C:/Users/demo/project/").as_deref(),
            Some(r"C:\Users\demo\project")
        );
    }
}

#[cfg(test)]
#[path = "codex_thread_sync_stability_tests.rs"]
mod stability_tests;

#[cfg(test)]
#[path = "codex_thread_sync_compressed_tests.rs"]
mod compressed_tests;
