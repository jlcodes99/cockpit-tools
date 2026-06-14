use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{TimeZone, Utc};
use rusqlite::{params_from_iter, types::Value as SqlValue, Connection};
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use toml_edit::Document;

use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DEFAULT_PROVIDER_ID: &str = "openai";
const STATE_DB_FILE: &str = "state_5.sqlite";
const STATE_DB_RELATIVE_PATHS: [&str; 2] = ["sqlite/state_5.sqlite", STATE_DB_FILE];
const CONFIG_FILE_NAME: &str = "config.toml";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX: &str = "backup-";
const SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX: &str = "-session-visibility-repair";
const MAX_SESSION_VISIBILITY_REPAIR_BACKUPS: usize = 1;
const SESSION_INDEX_ACTIVITY_DRIFT_MS: i128 = 3_600_000;
const THREAD_TITLE_MAX_CHARS: usize = 120;
const THREAD_PREVIEW_MAX_CHARS: usize = 240;
const CODEX_APP_VISIBLE_SOURCE: &str = "cli";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairItem {
    pub instance_id: String,
    pub instance_name: String,
    pub target_provider: String,
    pub changed_rollout_file_count: usize,
    pub normalized_source_count: usize,
    pub updated_sqlite_row_count: usize,
    pub missing_sqlite_thread_count: usize,
    pub added_session_index_entry_count: usize,
    pub repaired_project_index_workspace_count: usize,
    pub metadata_rebuild_failed: bool,
    pub skipped_sqlite_file: bool,
    pub backup_dir: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairSummary {
    pub instance_count: usize,
    pub mutated_instance_count: usize,
    pub changed_rollout_file_count: usize,
    pub normalized_source_count: usize,
    pub updated_sqlite_row_count: usize,
    pub missing_sqlite_thread_count: usize,
    pub added_session_index_entry_count: usize,
    pub repaired_project_index_workspace_count: usize,
    pub metadata_rebuild_failed_instance_count: usize,
    pub skipped_sqlite_file_count: usize,
    pub items: Vec<CodexSessionVisibilityRepairItem>,
    pub backup_dirs: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionSourceRepairCandidate {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub instance_id: String,
    pub instance_name: String,
    pub current_source: String,
    pub target_source: String,
    pub rollout_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionSourceRepairPreviewSummary {
    pub source_repair_count: usize,
    pub candidates: Vec<CodexSessionSourceRepairCandidate>,
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
struct RolloutProviderChange {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    updated_first_line: Option<String>,
    target_modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct RolloutSourceRepair {
    relative_path: PathBuf,
    absolute_path: PathBuf,
    session_id: String,
    title: String,
    cwd: String,
    current_source: String,
}

#[derive(Debug, Clone, Copy)]
struct SqliteProviderScan {
    rows_to_update: usize,
    missing_thread_count: usize,
    skipped_unusable_database: bool,
}

#[derive(Debug, Clone, Copy)]
struct ThreadsTableColumns {
    source: bool,
    model_provider: bool,
    cwd: bool,
    title: bool,
    preview: bool,
    has_user_event: bool,
    first_user_message: bool,
    thread_source: bool,
    rollout_path: bool,
    archived: bool,
    updated_at: bool,
    updated_at_ms: bool,
}

#[derive(Debug, Clone)]
struct SqliteThreadIndexRow {
    id: String,
    title: String,
    updated_at: Option<i64>,
    rollout_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct DerivedThreadMetadata {
    title: Option<String>,
    preview: Option<String>,
    first_user_message: Option<String>,
}

#[derive(Debug, Clone)]
struct RolloutThreadMetadata {
    title: Option<String>,
    preview: Option<String>,
    first_user_message: Option<String>,
    cwd: Option<String>,
    rollout_path: PathBuf,
    archived: bool,
}

#[derive(Debug, Clone)]
struct SqliteThreadRepair {
    id: String,
    model_provider: Option<String>,
    cwd: Option<String>,
    rollout_path: Option<String>,
    title: Option<String>,
    preview: Option<String>,
    has_user_event: Option<i64>,
    first_user_message: Option<String>,
    thread_source: Option<String>,
}

pub fn repair_session_visibility_across_instances(
    normalize_sources: bool,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut changed_rollout_file_count = 0usize;
    let mut normalized_source_count = 0usize;
    let mut updated_sqlite_row_count = 0usize;
    let mut missing_sqlite_thread_count = 0usize;
    let mut added_session_index_entry_count = 0usize;
    let mut repaired_project_index_workspace_count = 0usize;
    let mut metadata_rebuild_failed_instance_count = 0usize;
    let mut skipped_sqlite_file_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    for instance in &instances {
        let running = is_instance_running(instance, &process_entries);
        let target_provider = read_target_provider(&instance.data_dir)?;
        let mut derived_metadata =
            collect_thread_metadata_from_rollout_headers(&instance.data_dir).unwrap_or_default();
        let sidebar_entries = sidebar_thread_entries_from_metadata(&derived_metadata);
        let rollout_changes =
            collect_rollout_provider_changes(&instance.data_dir, &target_provider)?;
        let source_repairs = if normalize_sources {
            collect_source_repairs_for_instance(instance)?
        } else {
            Vec::new()
        };
        let sqlite_scan = count_sqlite_rows_to_update_with_metadata(
            &instance.data_dir,
            &target_provider,
            &mut derived_metadata,
        )?;
        let sqlite_rows_to_update = sqlite_scan.rows_to_update;
        let sqlite_threads_missing = sqlite_scan.missing_thread_count;
        let missing_session_index_entries = count_missing_session_index_entries_with_metadata(
            &instance.data_dir,
            &mut derived_metadata,
        )?;
        let missing_project_index_workspace_count =
            modules::codex_thread_sync::count_missing_sidebar_global_state_for_threads(
                &instance.data_dir,
                &sidebar_entries,
            )?;
        if sqlite_scan.skipped_unusable_database {
            skipped_sqlite_file_count += 1;
        }

        if rollout_changes.is_empty()
            && source_repairs.is_empty()
            && sqlite_rows_to_update == 0
            && sqlite_threads_missing == 0
            && missing_session_index_entries == 0
            && missing_project_index_workspace_count == 0
        {
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: 0,
                normalized_source_count: 0,
                updated_sqlite_row_count: 0,
                missing_sqlite_thread_count: 0,
                added_session_index_entry_count: 0,
                repaired_project_index_workspace_count: 0,
                metadata_rebuild_failed: false,
                skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
                backup_dir: None,
                running,
            });
            continue;
        }

        let backup_dir = backup_instance_files(
            &instance.data_dir,
            &rollout_changes,
            &source_repairs,
            sqlite_rows_to_update > 0 || sqlite_threads_missing > 0 || !source_repairs.is_empty(),
            missing_session_index_entries > 0,
            missing_project_index_workspace_count > 0,
            &instance.id,
            &target_provider,
        )?;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();

        let repaired = repair_single_instance(
            &instance.data_dir,
            &target_provider,
            &rollout_changes,
            &source_repairs,
            sqlite_rows_to_update > 0,
            sqlite_threads_missing,
            missing_session_index_entries > 0,
            missing_project_index_workspace_count > 0,
            missing_project_index_workspace_count,
            &sidebar_entries,
            &mut derived_metadata,
        );
        let (
            sqlite_rows_updated,
            session_index_entries_added,
            sources_normalized,
            project_index_workspaces_repaired,
            metadata_rebuild_failed,
        ) = match repaired {
            Ok(value) => value,
            Err(error) => {
                let restore_result = restore_instance_files_from_backup(
                    &instance.data_dir,
                    &backup_dir,
                    sqlite_rows_to_update > 0
                        || sqlite_threads_missing > 0
                        || !source_repairs.is_empty(),
                );
                if let Err(restore_error) = restore_result {
                    return Err(format!(
                        "修复实例历史会话可见性失败 ({}): {}；自动回滚也失败: {}；备份目录: {}",
                        instance.name,
                        error,
                        restore_error,
                        backup_dir.display()
                    ));
                }
                return Err(format!(
                    "修复实例历史会话可见性失败 ({}): {}；已自动回滚，备份目录: {}",
                    instance.name,
                    error,
                    backup_dir.display()
                ));
            }
        };

        mutated_instance_count += 1;
        changed_rollout_file_count += rollout_changes.len();
        normalized_source_count += sources_normalized;
        updated_sqlite_row_count += sqlite_rows_updated;
        missing_sqlite_thread_count += sqlite_threads_missing;
        added_session_index_entry_count += session_index_entries_added;
        repaired_project_index_workspace_count += project_index_workspaces_repaired;
        if metadata_rebuild_failed {
            metadata_rebuild_failed_instance_count += 1;
        }
        if running {
            mutated_running_instance_count += 1;
        }
        backup_dirs.push(backup_dir_string.clone());
        items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            target_provider,
            changed_rollout_file_count: rollout_changes.len(),
            normalized_source_count: sources_normalized,
            updated_sqlite_row_count: sqlite_rows_updated,
            missing_sqlite_thread_count: sqlite_threads_missing,
            added_session_index_entry_count: session_index_entries_added,
            repaired_project_index_workspace_count: project_index_workspaces_repaired,
            metadata_rebuild_failed,
            skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
            backup_dir: Some(backup_dir_string),
            running,
        });
    }

    prune_session_visibility_repair_backups(&instances);

    let message = build_summary_message(
        mutated_instance_count,
        changed_rollout_file_count,
        normalized_source_count,
        updated_sqlite_row_count,
        missing_sqlite_thread_count,
        added_session_index_entry_count,
        repaired_project_index_workspace_count,
        metadata_rebuild_failed_instance_count,
        mutated_running_instance_count,
        skipped_sqlite_file_count,
    );

    Ok(CodexSessionVisibilityRepairSummary {
        instance_count: instances.len(),
        mutated_instance_count,
        changed_rollout_file_count,
        normalized_source_count,
        updated_sqlite_row_count,
        missing_sqlite_thread_count,
        added_session_index_entry_count,
        repaired_project_index_workspace_count,
        metadata_rebuild_failed_instance_count,
        skipped_sqlite_file_count,
        items,
        backup_dirs,
        message,
    })
}

pub fn preview_session_visibility_source_repairs(
) -> Result<CodexSessionSourceRepairPreviewSummary, String> {
    let instances = collect_instances()?;
    let mut candidates = Vec::new();
    for instance in &instances {
        candidates.extend(collect_source_repair_candidates_for_instance(instance)?);
    }
    candidates.sort_by(|left, right| {
        left.instance_name
            .cmp(&right.instance_name)
            .then_with(|| left.cwd.cmp(&right.cwd))
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let message = if candidates.is_empty() {
        "未发现需要恢复 Codex App 可见来源的会话".to_string()
    } else {
        format!(
            "发现 {} 条会话来源不是 cli，需要确认后恢复为 Codex App 可见来源",
            candidates.len()
        )
    };
    Ok(CodexSessionSourceRepairPreviewSummary {
        source_repair_count: candidates.len(),
        candidates,
        message,
    })
}

pub fn read_history_visibility_provider_for_dir(data_dir: &Path) -> Result<String, String> {
    read_target_provider(data_dir)
}

fn repair_single_instance(
    data_dir: &Path,
    target_provider: &str,
    rollout_changes: &[RolloutProviderChange],
    source_repairs: &[RolloutSourceRepair],
    update_sqlite: bool,
    missing_sqlite_thread_count: usize,
    reconcile_session_index: bool,
    repair_project_index: bool,
    project_index_workspace_count: usize,
    sidebar_entries: &[modules::codex_thread_sync::CodexSidebarThreadEntry],
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<(usize, usize, usize, usize, bool), String> {
    let sqlite_rows_updated = if update_sqlite {
        update_sqlite_provider_with_metadata(data_dir, target_provider, derived_metadata)?
    } else {
        0
    };
    for change in rollout_changes {
        rewrite_rollout_provider(change)?;
    }
    let source_paths = source_repairs
        .iter()
        .map(|repair| repair.absolute_path.clone())
        .collect::<Vec<_>>();
    let sources_normalized = normalize_session_sources_for_rollout_paths(data_dir, &source_paths)?;
    repair_sqlite_thread_timestamps(data_dir)?;
    let session_index_entries_added = if reconcile_session_index {
        reconcile_session_index_from_sqlite_with_metadata(data_dir, derived_metadata)?
    } else {
        0
    };
    let project_index_workspaces_repaired = if repair_project_index {
        modules::codex_thread_sync::repair_sidebar_global_state_for_threads(
            data_dir,
            sidebar_entries,
        )?;
        project_index_workspace_count
    } else {
        0
    };
    let mutated = sqlite_rows_updated > 0
        || !rollout_changes.is_empty()
        || sources_normalized > 0
        || missing_sqlite_thread_count > 0
        || session_index_entries_added > 0
        || project_index_workspaces_repaired > 0;
    let metadata_rebuild_failed = if mutated {
        try_rebuild_thread_metadata(data_dir)
    } else {
        false
    };
    Ok((
        sqlite_rows_updated,
        session_index_entries_added,
        sources_normalized,
        project_index_workspaces_repaired,
        metadata_rebuild_failed,
    ))
}

fn build_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    normalized_source_count: usize,
    updated_sqlite_row_count: usize,
    missing_sqlite_thread_count: usize,
    added_session_index_entry_count: usize,
    repaired_project_index_workspace_count: usize,
    metadata_rebuild_failed_instance_count: usize,
    mutated_running_instance_count: usize,
    _skipped_sqlite_file_count: usize,
) -> String {
    if mutated_instance_count == 0 {
        return "所有 Codex 实例的历史会话 provider 元数据与 session_index 已与当前 provider 一致，无需修复。请手动彻底退出Codex进程后再启动"
            .to_string();
    }

    let index_suffix = if added_session_index_entry_count > 0 {
        format!(
            "，补写 {} 条 session_index 记录",
            added_session_index_entry_count
        )
    } else {
        String::new()
    };

    let source_suffix = if normalized_source_count > 0 {
        format!("，恢复 {} 条 Codex App 可见来源", normalized_source_count)
    } else {
        String::new()
    };
    let sqlite_rebuild_suffix = if missing_sqlite_thread_count > 0 {
        format!("，触发重建 {} 条缺失 SQLite 线程索引", missing_sqlite_thread_count)
    } else {
        String::new()
    };
    let project_suffix = if repaired_project_index_workspace_count > 0 {
        format!(
            "，修复 {} 项 Codex 侧栏状态",
            repaired_project_index_workspace_count
        )
    } else {
        String::new()
    };
    let restart_suffix = if mutated_running_instance_count > 0 {
        "。运行中的实例可能需要重启后显示；请手动彻底退出Codex进程后再启动"
    } else {
        "。请手动彻底退出Codex进程后再启动"
    };
    let rebuild_suffix = if metadata_rebuild_failed_instance_count > 0 {
        format!(
            "；{} 个实例未能触发官方 Codex 重建会话索引，请确认 Codex 启动路径或 CLI 可用后重试",
            metadata_rebuild_failed_instance_count
        )
    } else {
        String::new()
    };

    format!(
        "已为 {} 个实例修复历史会话可见性：改写 {} 个 rollout 文件，更新 {} 条 SQLite 记录{}{}{}{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        index_suffix,
        source_suffix,
        sqlite_rebuild_suffix,
        project_suffix,
        rebuild_suffix,
        restart_suffix
    )
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

fn try_rebuild_thread_metadata(data_dir: &Path) -> bool {
    match modules::codex_official_app_server::rebuild_thread_metadata(data_dir) {
        Ok(()) => false,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "Codex 会话可见性修复后重建官方会话索引失败 ({}): {}",
                data_dir.display(),
                error
            ));
            true
        }
    }
}

fn read_target_provider(data_dir: &Path) -> Result<String, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let doc = content.parse::<Document>().map_err(|error| {
        format!(
            "解析 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn collect_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
) -> Result<Vec<RolloutProviderChange>, String> {
    let session_index_map = match read_session_index_map(data_dir) {
        Ok(value) => value,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "读取 Codex session_index.jsonl 失败，跳过该时间来源并继续修复会话可见性: {}",
                error
            ));
            HashMap::new()
        }
    };
    let mut changes = Vec::new();

    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        let rollout_paths = list_rollout_files(&root_dir)?;
        for rollout_path in rollout_paths {
            let Some((first_line, _separator)) = read_first_line(&rollout_path)? else {
                continue;
            };
            let Some(mut parsed) = parse_session_meta_record(&first_line) else {
                continue;
            };
            let session_id = session_meta_id(&parsed);
            let fallback_modified_ms =
                modules::codex_session_file_time::read_modified_time(&rollout_path)
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as i128);
            let target_modified_at = resolve_target_modified_at_ms(
                session_id.as_deref(),
                &session_index_map,
                &rollout_path,
                fallback_modified_ms,
            )
            .and_then(modules::codex_session_file_time::system_time_from_unix_millis);
            let current_modified_at =
                modules::codex_session_file_time::read_modified_time(&rollout_path);
            let current_provider = parsed["payload"]
                .get("model_provider")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let provider_matches = current_provider == target_provider;
            let modified_time_matches = target_modified_at.is_none()
                || modules::codex_session_file_time::same_modified_time_millis(
                    current_modified_at,
                    target_modified_at,
                );
            if provider_matches && modified_time_matches {
                continue;
            }

            let updated_first_line = if provider_matches {
                None
            } else if let Some(payload) =
                parsed.get_mut("payload").and_then(JsonValue::as_object_mut)
            {
                payload.insert(
                    "model_provider".to_string(),
                    JsonValue::String(target_provider.to_string()),
                );
                Some(
                    serde_json::to_string(&parsed)
                        .map_err(|error| format!("序列化 session_meta 失败: {}", error))?,
                )
            } else {
                None
            };

            let relative_path = rollout_path
                .strip_prefix(data_dir)
                .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?;
            changes.push(RolloutProviderChange {
                relative_path: relative_path.to_path_buf(),
                absolute_path: rollout_path,
                updated_first_line,
                target_modified_at,
            });
        }
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn collect_source_repair_candidates_for_instance(
    instance: &CodexSyncInstance,
) -> Result<Vec<CodexSessionSourceRepairCandidate>, String> {
    let repairs = collect_source_repairs_for_instance(instance)?;
    Ok(repairs
        .into_iter()
        .map(|repair| CodexSessionSourceRepairCandidate {
            session_id: repair.session_id,
            title: repair.title,
            cwd: repair.cwd,
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            current_source: repair.current_source,
            target_source: CODEX_APP_VISIBLE_SOURCE.to_string(),
            rollout_path: repair.absolute_path.to_string_lossy().to_string(),
        })
        .collect())
}

fn collect_source_repairs_for_instance(
    instance: &CodexSyncInstance,
) -> Result<Vec<RolloutSourceRepair>, String> {
    let root_dir = instance.data_dir.join("sessions");
    if !root_dir.exists() {
        return Ok(Vec::new());
    }
    let session_index_map = read_session_index_map(&instance.data_dir).unwrap_or_default();
    let mut repairs = Vec::new();
    for rollout_path in list_rollout_files(&root_dir)? {
        let Some((first_line, _separator)) = read_first_line(&rollout_path)? else {
            continue;
        };
        let Some(parsed) = parse_session_meta_record(&first_line) else {
            continue;
        };
        let Some(session_id) = session_meta_id(&parsed) else {
            continue;
        };
        let current_source = session_meta_source(&parsed).unwrap_or_default();
        if current_source == CODEX_APP_VISIBLE_SOURCE {
            continue;
        }
        let relative_path = rollout_path
            .strip_prefix(&instance.data_dir)
            .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?
            .to_path_buf();
        let title = session_index_map
            .get(&session_id)
            .and_then(session_index_title)
            .unwrap_or_else(|| session_id.clone());
        let cwd = session_meta_cwd(&parsed).unwrap_or_else(|| "未知工作目录".to_string());
        repairs.push(RolloutSourceRepair {
            relative_path,
            absolute_path: rollout_path,
            session_id,
            title,
            cwd,
            current_source,
        });
    }
    repairs.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(repairs)
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
        if file_type.is_file() {
            let file_name = path
                .file_name()
                .and_then(|item| item.to_str())
                .unwrap_or_default();
            if file_name.starts_with("rollout-") && file_name.ends_with(".jsonl") {
                result.push(path);
            }
        }
    }

    result.sort();
    Ok(result)
}

fn read_first_line(path: &Path) -> Result<Option<(String, String)>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("打开 rollout 文件失败 ({}): {}", path.display(), error))?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let bytes_read = reader
        .read_until(b'\n', &mut buffer)
        .map_err(|error| format!("读取 rollout 首行失败 ({}): {}", path.display(), error))?;
    if bytes_read == 0 {
        return Ok(None);
    }

    let (line_bytes, separator) = if buffer.ends_with(b"\r\n") {
        (&buffer[..buffer.len() - 2], "\r\n")
    } else if buffer.ends_with(b"\n") {
        (&buffer[..buffer.len() - 1], "\n")
    } else {
        (&buffer[..], "")
    };

    let line = String::from_utf8(line_bytes.to_vec()).map_err(|error| {
        format!(
            "解析 rollout 首行 UTF-8 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(Some((line, separator.to_string())))
}

fn parse_session_meta_record(first_line: &str) -> Option<JsonValue> {
    if first_line.trim().is_empty() {
        return None;
    }

    let parsed = serde_json::from_str::<JsonValue>(first_line).ok()?;
    if parsed.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return None;
    }
    if !parsed.get("payload").is_some_and(JsonValue::is_object) {
        return None;
    }
    Some(parsed)
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

fn session_meta_source(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("source"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn session_meta_cwd(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("cwd"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
        let Ok(entry) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), entry);
    }
    Ok(entries)
}

fn existing_state_db_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    for relative in STATE_DB_RELATIVE_PATHS {
        let path = data_dir.join(relative);
        if !path.exists() {
            continue;
        }
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            paths.push(path);
        }
    }
    paths
}

fn session_index_title(entry: &JsonValue) -> Option<String> {
    ["thread_name", "threadName", "title", "name"]
        .iter()
        .filter_map(|key| entry.get(*key).and_then(JsonValue::as_str))
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_display_text(value: &str, max_chars: usize) -> Option<String> {
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let truncated = normalized.chars().take(max_chars).collect::<String>();
    if truncated.is_empty() {
        None
    } else {
        Some(truncated)
    }
}

fn normalize_first_user_message(value: &str) -> Option<String> {
    let normalized = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn is_placeholder_user_text(value: &str) -> bool {
    let trimmed = value.trim_start();
    trimmed.starts_with("# AGENTS.md instructions") || trimmed.starts_with("<environment_context>")
}

fn extract_message_text_parts(content: &JsonValue) -> Vec<String> {
    let Some(parts) = content.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
                .or_else(|| {
                    part.get("content")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                })
        })
        .collect()
}

fn extract_rollout_user_message(value: &JsonValue) -> Option<String> {
    match value.get("type").and_then(JsonValue::as_str) {
        Some("response_item") => {
            let payload = value.get("payload")?;
            let item = payload.get("item").unwrap_or(payload);
            if item.get("type").and_then(JsonValue::as_str) != Some("message")
                || item.get("role").and_then(JsonValue::as_str) != Some("user")
            {
                return None;
            }
            normalize_first_user_message(
                &extract_message_text_parts(item.get("content").unwrap_or(&JsonValue::Null))
                    .join("\n"),
            )
        }
        Some("message") if value.get("role").and_then(JsonValue::as_str) == Some("user") => {
            normalize_first_user_message(
                &extract_message_text_parts(value.get("content").unwrap_or(&JsonValue::Null))
                    .join("\n"),
            )
        }
        _ => value
            .get("payload")
            .filter(|payload| payload.is_object())
            .and_then(|payload| {
                if payload.get("role").and_then(JsonValue::as_str) != Some("user") {
                    return None;
                }
                payload
                    .get("content")
                    .and_then(JsonValue::as_str)
                    .and_then(normalize_first_user_message)
            }),
    }
}

fn derive_thread_metadata_from_rollout(
    rollout_path: &Path,
    session_index_title: Option<String>,
) -> Result<Option<(String, DerivedThreadMetadata)>, String> {
    let file = fs::File::open(rollout_path).map_err(|error| {
        format!(
            "打开 rollout 文件失败 ({}): {}",
            rollout_path.display(),
            error
        )
    })?;
    let reader = BufReader::new(file);
    let mut session_id = None::<String>;
    let mut first_meaningful_user_message = None::<String>;

    for line in reader.lines() {
        let line = line.map_err(|error| {
            format!(
                "读取 rollout 文件失败 ({}): {}",
                rollout_path.display(),
                error
            )
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        if session_id.is_none() {
            if let Some(meta) = parse_session_meta_record(trimmed) {
                session_id = session_meta_id(&meta);
                continue;
            }
        }
        if first_meaningful_user_message.is_some() {
            continue;
        }
        let Some(text) = extract_rollout_user_message(&parsed) else {
            continue;
        };
        if !is_placeholder_user_text(&text) {
            first_meaningful_user_message = Some(text);
        }
    }

    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let title = session_index_title
        .or_else(|| {
            first_meaningful_user_message
                .as_deref()
                .and_then(|value| normalize_display_text(value, THREAD_TITLE_MAX_CHARS))
        })
        .or_else(|| normalize_display_text(&session_id, THREAD_TITLE_MAX_CHARS));
    let preview = first_meaningful_user_message
        .as_deref()
        .and_then(|value| normalize_display_text(value, THREAD_PREVIEW_MAX_CHARS))
        .or_else(|| title.clone());
    Ok(Some((
        session_id,
        DerivedThreadMetadata {
            title,
            preview,
            first_user_message: first_meaningful_user_message,
        },
    )))
}

fn collect_thread_metadata_from_rollouts(
    data_dir: &Path,
) -> Result<HashMap<String, RolloutThreadMetadata>, String> {
    let mut metadata_by_session_id = collect_thread_metadata_from_rollout_headers(data_dir)?;
    let session_index_map = read_session_index_map(data_dir).unwrap_or_default();
    let session_ids = metadata_by_session_id.keys().cloned().collect::<Vec<_>>();

    for session_id in session_ids {
        let session_index_title = session_index_map
            .get(&session_id)
            .and_then(session_index_title);
        complete_rollout_metadata_for_session(
            &session_id,
            session_index_title,
            &mut metadata_by_session_id,
        );
    }

    Ok(metadata_by_session_id)
}

fn collect_thread_metadata_from_rollout_headers(
    data_dir: &Path,
) -> Result<HashMap<String, RolloutThreadMetadata>, String> {
    let session_index_map = read_session_index_map(data_dir).unwrap_or_default();
    let mut metadata_by_session_id = HashMap::new();

    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let Some(first_line) = read_first_line(&rollout_path)? else {
                continue;
            };
            let Some(session_meta) = parse_session_meta_record(&first_line.0) else {
                continue;
            };
            let Some(session_id) = session_meta_id(&session_meta) else {
                continue;
            };
            let cwd = session_meta_cwd(&session_meta);
            let session_index_title = session_index_map
                .get(&session_id)
                .and_then(session_index_title);
            let metadata = RolloutThreadMetadata {
                title: session_index_title,
                preview: None,
                first_user_message: None,
                cwd,
                rollout_path: rollout_path.clone(),
                archived: dir_name == "archived_sessions",
            };
            merge_rollout_thread_metadata(&mut metadata_by_session_id, session_id, metadata);
        }
    }

    Ok(metadata_by_session_id)
}

fn merge_rollout_thread_metadata(
    metadata_by_session_id: &mut HashMap<String, RolloutThreadMetadata>,
    session_id: String,
    metadata: RolloutThreadMetadata,
) {
    metadata_by_session_id
        .entry(session_id)
        .and_modify(|current| {
            if current.archived && !metadata.archived {
                *current = metadata.clone();
                return;
            }
            if current.title.is_none() {
                current.title = metadata.title.clone();
            }
            if current.preview.is_none() {
                current.preview = metadata.preview.clone();
            }
            if current.first_user_message.is_none() {
                current.first_user_message = metadata.first_user_message.clone();
            }
            if current.cwd.is_none() {
                current.cwd = metadata.cwd.clone();
            }
        })
        .or_insert(metadata);
}

fn complete_rollout_metadata_for_session(
    session_id: &str,
    session_index_title: Option<String>,
    metadata_by_session_id: &mut HashMap<String, RolloutThreadMetadata>,
) {
    let Some(existing) = metadata_by_session_id.get(session_id).cloned() else {
        return;
    };
    if existing.title.is_some()
        && existing.preview.is_some()
        && existing.first_user_message.is_some()
    {
        return;
    }

    match derive_thread_metadata_from_rollout(&existing.rollout_path, session_index_title) {
        Ok(Some((derived_session_id, derived))) if derived_session_id == session_id => {
            if let Some(current) = metadata_by_session_id.get_mut(session_id) {
                if current.title.is_none() {
                    current.title = derived.title;
                }
                if current.preview.is_none() {
                    current.preview = derived.preview;
                }
                if current.first_user_message.is_none() {
                    current.first_user_message = derived.first_user_message;
                }
            }
        }
        Ok(_) => {}
        Err(error) => {
            modules::logger::log_warn(&format!(
                "按需解析 Codex rollout 元数据失败，已跳过正文补全 ({}): {}",
                existing.rollout_path.display(),
                error
            ));
        }
    }
}

fn sidebar_thread_entries_from_metadata(
    metadata_by_session_id: &HashMap<String, RolloutThreadMetadata>,
) -> Vec<modules::codex_thread_sync::CodexSidebarThreadEntry> {
    let mut entries = metadata_by_session_id
        .iter()
        .map(
            |(session_id, metadata)| modules::codex_thread_sync::CodexSidebarThreadEntry {
                session_id: session_id.clone(),
                workspace_root: metadata.cwd.clone(),
                archived: metadata.archived,
            },
        )
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    entries
}

fn count_missing_session_index_entries(data_dir: &Path) -> Result<usize, String> {
    let mut derived_metadata =
        collect_thread_metadata_from_rollout_headers(data_dir).unwrap_or_default();
    count_missing_session_index_entries_with_metadata(data_dir, &mut derived_metadata)
}

fn count_missing_session_index_entries_with_metadata(
    data_dir: &Path,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<usize, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_with_metadata(data_dir, derived_metadata)?;
    Ok(rows
        .iter()
        .filter(|row| {
            session_index_map
                .get(&row.id)
                .map(|entry| should_repair_session_index_entry(data_dir, entry, row))
                .unwrap_or(true)
        })
        .count())
}

fn should_repair_session_index_entry(
    data_dir: &Path,
    entry: &JsonValue,
    row: &SqliteThreadIndexRow,
) -> bool {
    if session_index_title(entry).is_none() {
        return true;
    }

    let Some(target_updated_at_seconds) = resolve_thread_updated_at_seconds(data_dir, row) else {
        return false;
    };
    let Some(indexed_updated_at_ms) = parse_session_index_updated_at_ms(entry) else {
        return true;
    };
    (indexed_updated_at_ms - (target_updated_at_seconds as i128 * 1000)).abs()
        > SESSION_INDEX_ACTIVITY_DRIFT_MS
}

fn load_sqlite_thread_index_rows(data_dir: &Path) -> Result<Vec<SqliteThreadIndexRow>, String> {
    let mut derived_metadata =
        collect_thread_metadata_from_rollout_headers(data_dir).unwrap_or_default();
    load_sqlite_thread_index_rows_with_metadata(data_dir, &mut derived_metadata)
}

fn load_sqlite_thread_index_rows_with_metadata(
    data_dir: &Path,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    let session_index_map = read_session_index_map(data_dir).unwrap_or_default();
    let mut rows_by_id = HashMap::<String, SqliteThreadIndexRow>::new();

    for db_path in existing_state_db_paths(data_dir) {
        let connection = match Connection::open(&db_path) {
            Ok(connection) => connection,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "打开实例数据库失败 ({}): {}",
                    db_path.display(),
                    error
                ));
            }
        };

        let columns = match read_threads_table_columns(&connection) {
            Ok(columns) => columns,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) if is_missing_threads_table_error(&error) => continue,
            Err(error) => {
                return Err(format_sqlite_read_error(
                    &db_path,
                    "读取 SQLite threads 表结构失败",
                    &error,
                ));
            }
        };
        let Some(columns) = columns else {
            continue;
        };

        let title_expr = if columns.title {
            "COALESCE(title, '')"
        } else {
            "''"
        };
        let updated_at_expr = if columns.updated_at {
            "updated_at"
        } else {
            "NULL"
        };
        let rollout_path_expr = if columns.rollout_path {
            "rollout_path"
        } else {
            "NULL"
        };
        let order_by = if columns.updated_at {
            "updated_at DESC, id ASC"
        } else {
            "id ASC"
        };
        let sql = format!(
            "SELECT id, {title_expr}, {updated_at_expr}, {rollout_path_expr} FROM threads ORDER BY {order_by}"
        );
        let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
            format!(
                "准备 SQLite 会话索引查询失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
        let mapped = statement
            .query_map([], |row| {
                Ok(SqliteThreadIndexRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    rollout_path: row.get(3)?,
                })
            })
            .map_err(|error| {
                format!(
                    "查询 SQLite 会话索引行失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;
        for row in mapped {
            let row = row.map_err(|error| {
                format!(
                    "读取 SQLite 会话索引行失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;
            rows_by_id
                .entry(row.id.clone())
                .and_modify(|current| merge_thread_index_row(current, &row))
                .or_insert(row);
        }
    }

    let mut result = rows_by_id.into_values().collect::<Vec<_>>();
    for row in &mut result {
        if row.title.trim().is_empty() {
            let indexed_title = session_index_map.get(&row.id).and_then(session_index_title);
            if indexed_title.is_none()
                && derived_metadata
                    .get(&row.id)
                    .and_then(|metadata| metadata.title.as_ref())
                    .is_none()
            {
                complete_rollout_metadata_for_session(&row.id, None, derived_metadata);
            }
            row.title = derived_metadata
                .get(&row.id)
                .and_then(|metadata| metadata.title.clone())
                .or(indexed_title)
                .unwrap_or_else(|| row.id.clone());
        }
    }
    result.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(result)
}

fn merge_thread_index_row(current: &mut SqliteThreadIndexRow, candidate: &SqliteThreadIndexRow) {
    if current.title.trim().is_empty() && !candidate.title.trim().is_empty() {
        current.title = candidate.title.clone();
    }

    if current
        .rollout_path
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
        && !candidate
            .rollout_path
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        current.rollout_path = candidate.rollout_path.clone();
    }

    if candidate.updated_at > current.updated_at {
        current.updated_at = candidate.updated_at;
        if !candidate.title.trim().is_empty() {
            current.title = candidate.title.clone();
        }
        if !candidate
            .rollout_path
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            current.rollout_path = candidate.rollout_path.clone();
        }
    }
}

fn format_thread_updated_at_iso(updated_at: Option<i64>) -> String {
    let seconds = updated_at.unwrap_or_else(|| Utc::now().timestamp());
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn resolve_thread_updated_at_seconds(data_dir: &Path, row: &SqliteThreadIndexRow) -> Option<i64> {
    let rollout_activity_seconds = row
        .rollout_path
        .as_deref()
        .map(|path| resolve_rollout_path(data_dir, path))
        .filter(|path| path.exists())
        .and_then(|path| rollout_file_activity_ms(&path))
        .map(|value| (value / 1000) as i64);
    match (row.updated_at, rollout_activity_seconds) {
        (Some(sqlite_seconds), Some(activity_seconds))
            if i64::abs(sqlite_seconds - activity_seconds) > 3600 =>
        {
            Some(activity_seconds)
        }
        (Some(sqlite_seconds), _) => Some(sqlite_seconds),
        (None, Some(activity_seconds)) => Some(activity_seconds),
        (None, None) => None,
    }
}

fn build_session_index_entry_from_thread(
    data_dir: &Path,
    row: &SqliteThreadIndexRow,
    existing_entry: Option<&JsonValue>,
) -> JsonValue {
    let mut object = existing_entry
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    object.insert("id".to_string(), JsonValue::String(row.id.clone()));
    object.insert(
        "thread_name".to_string(),
        JsonValue::String(if row.title.trim().is_empty() {
            "Untitled".to_string()
        } else {
            row.title.clone()
        }),
    );
    object.insert(
        "updated_at".to_string(),
        JsonValue::String(format_thread_updated_at_iso(
            resolve_thread_updated_at_seconds(data_dir, row),
        )),
    );
    JsonValue::Object(object)
}

fn reconcile_session_index_from_sqlite(data_dir: &Path) -> Result<usize, String> {
    let mut derived_metadata =
        collect_thread_metadata_from_rollout_headers(data_dir).unwrap_or_default();
    reconcile_session_index_from_sqlite_with_metadata(data_dir, &mut derived_metadata)
}

fn reconcile_session_index_from_sqlite_with_metadata(
    data_dir: &Path,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<usize, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_with_metadata(data_dir, derived_metadata)?;
    let rows_to_repair = rows
        .iter()
        .filter(|row| {
            session_index_map
                .get(&row.id)
                .map(|entry| should_repair_session_index_entry(data_dir, entry, row))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if rows_to_repair.is_empty() {
        return Ok(0);
    }

    let path = data_dir.join(SESSION_INDEX_FILE);
    let mut lines = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|error| {
                format!(
                    "读取 session_index.jsonl 失败 ({}): {}",
                    path.display(),
                    error
                )
            })?
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut entry_positions = HashMap::<String, usize>::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entry_positions.insert(id.to_string(), index);
    }

    for row in &rows_to_repair {
        let entry =
            build_session_index_entry_from_thread(data_dir, row, session_index_map.get(&row.id));
        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        if let Some(index) = entry_positions.get(&row.id).copied() {
            lines[index] = line;
        } else {
            lines.push(line);
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
    Ok(rows_to_repair.len())
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

fn parse_timestamp_ms(value: &JsonValue) -> Option<i128> {
    match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_ms),
        JsonValue::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp_millis() as i128)
            .or_else(|| text.parse::<i64>().ok().map(normalize_codex_timestamp_ms)),
        _ => None,
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
    .find_map(parse_timestamp_ms)
}

fn parse_rollout_line_timestamp_ms(value: &JsonValue) -> Option<i128> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .and_then(parse_timestamp_ms)
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
                .and_then(parse_timestamp_ms)
        })
}

fn rollout_file_activity_ms(path: &Path) -> Option<i128> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<JsonValue>(line.trim()).ok())
        .filter_map(|value| parse_rollout_line_timestamp_ms(&value))
        .max()
}

fn resolve_target_modified_at_ms(
    session_id: Option<&str>,
    session_index_map: &HashMap<String, JsonValue>,
    rollout_path: &Path,
    fallback_ms: Option<i128>,
) -> Option<i128> {
    let indexed = session_id
        .and_then(|id| session_index_map.get(id))
        .and_then(parse_session_index_updated_at_ms);
    let activity = rollout_file_activity_ms(rollout_path);
    match (indexed, activity) {
        (Some(indexed), Some(activity))
            if (indexed - activity).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity)
        }
        (Some(indexed), _) => Some(indexed),
        (None, Some(activity)) => Some(activity),
        (None, None) => fallback_ms,
    }
}

fn resolve_rollout_path(data_dir: &Path, rollout_path: &str) -> PathBuf {
    let trimmed = rollout_path.trim();
    let stripped = trimmed.strip_prefix(r"\\?\").unwrap_or(trimmed);
    let path = Path::new(stripped);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir.join(path)
    }
}

fn normalize_path_text_for_compare(value: &str) -> String {
    let mut value = value.trim();
    if let Some(stripped) = value.strip_prefix(r"\\?\") {
        value = stripped;
    }
    let is_windows_path = value.starts_with(r"\\")
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
    if is_windows_path {
        normalized = normalized.to_ascii_lowercase();
    }
    normalized
}

fn path_text_matches(left: &str, right: &str) -> bool {
    normalize_path_text_for_compare(left) == normalize_path_text_for_compare(right)
}

fn paths_refer_to_same_location(left: &Path, right: &Path) -> bool {
    path_text_matches(&left.to_string_lossy(), &right.to_string_lossy())
}

fn repair_sqlite_thread_timestamps(data_dir: &Path) -> Result<usize, String> {
    let mut updated_rows = 0usize;
    for db_path in existing_state_db_paths(data_dir) {
        updated_rows += repair_sqlite_thread_timestamps_in_db(data_dir, &db_path)?;
    }
    Ok(updated_rows)
}

fn repair_sqlite_thread_timestamps_in_db(data_dir: &Path, db_path: &Path) -> Result<usize, String> {
    let mut connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let columns = match read_threads_table_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) if is_missing_threads_table_error(&error) => return Ok(0),
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let Some(columns) = columns else {
        return Ok(0);
    };
    if !columns.rollout_path || !columns.updated_at {
        return Ok(0);
    }

    let mut statement = connection
        .prepare(
            "SELECT id, rollout_path, updated_at FROM threads WHERE rollout_path IS NOT NULL AND rollout_path <> ''",
        )
        .map_err(|error| {
            format_sqlite_read_error(db_path, "准备 SQLite 会话时间修复查询失败", &error)
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })
        .map_err(|error| format_sqlite_read_error(db_path, "查询 SQLite 会话时间失败", &error))?;

    let mut updates = Vec::new();
    for row in rows {
        let (thread_id, rollout_path, updated_at) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite 会话时间失败", &error)
        })?;
        let rollout = resolve_rollout_path(data_dir, &rollout_path);
        if !rollout.exists() {
            continue;
        }
        let Some(activity_ms) = rollout_file_activity_ms(&rollout) else {
            continue;
        };
        let activity_seconds = (activity_ms / 1000) as i64;
        if i64::abs(updated_at.unwrap_or(0) - activity_seconds) <= 1 {
            continue;
        }
        updates.push((activity_seconds, thread_id));
    }
    drop(statement);

    if updates.is_empty() {
        return Ok(0);
    }

    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    for (activity_seconds, thread_id) in &updates {
        if columns.updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    (
                        *activity_seconds,
                        *activity_seconds * 1000,
                        thread_id.as_str(),
                    ),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
                    (*activity_seconds, thread_id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updates.len())
}

fn is_missing_threads_table_error(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table: threads")
}

fn log_skipped_sqlite_database(path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "跳过无效或损坏的 Codex state_5.sqlite ({}): {}",
        path.display(),
        reason
    ));
}

fn count_sqlite_rows_to_update(
    data_dir: &Path,
    target_provider: &str,
) -> Result<SqliteProviderScan, String> {
    let mut derived_metadata =
        collect_thread_metadata_from_rollout_headers(data_dir).unwrap_or_default();
    count_sqlite_rows_to_update_with_metadata(data_dir, target_provider, &mut derived_metadata)
}

fn count_sqlite_rows_to_update_with_metadata(
    data_dir: &Path,
    target_provider: &str,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<SqliteProviderScan, String> {
    let mut rows_to_update = 0usize;
    let mut missing_thread_ids = HashSet::<String>::new();
    let active_rollout_thread_ids = active_rollout_thread_ids(derived_metadata);
    let mut skipped_unusable_database = false;
    let mut readable_threads_table_count = 0usize;
    let db_paths = existing_state_db_paths(data_dir);

    for db_path in &db_paths {
        let connection = match Connection::open(&db_path) {
            Ok(connection) => connection,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                skipped_unusable_database = true;
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "打开实例数据库失败 ({}): {}",
                    db_path.display(),
                    error
                ));
            }
        };
        let columns = match read_threads_table_columns(&connection) {
            Ok(columns) => columns,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                skipped_unusable_database = true;
                continue;
            }
            Err(error) if is_missing_threads_table_error(&error) => continue,
            Err(error) => {
                return Err(format_sqlite_read_error(
                    &db_path,
                    "读取 SQLite threads 表结构失败",
                    &error,
                ));
            }
        };
        let Some(columns) = columns else {
            continue;
        };
        readable_threads_table_count += 1;
        let sqlite_thread_ids = read_sqlite_thread_ids(&connection).map_err(|error| {
            format_sqlite_read_error(&db_path, "读取 SQLite 会话 ID 失败", &error)
        })?;
        for session_id in &active_rollout_thread_ids {
            if !sqlite_thread_ids.contains(session_id) {
                missing_thread_ids.insert(session_id.clone());
            }
        }
        rows_to_update += collect_sqlite_thread_repairs(
            data_dir,
            &connection,
            columns,
            target_provider,
            derived_metadata,
        )
        .map_err(|error| {
            format_sqlite_read_error(&db_path, "统计 SQLite 会话可见性差异失败", &error)
        })?
        .len();
    }
    let missing_thread_count = if active_rollout_thread_ids.is_empty() {
        0
    } else if readable_threads_table_count > 0 {
        missing_thread_ids.len()
    } else if db_paths.is_empty() || !skipped_unusable_database {
        active_rollout_thread_ids.len()
    } else {
        0
    };

    Ok(SqliteProviderScan {
        rows_to_update,
        missing_thread_count,
        skipped_unusable_database,
    })
}

fn active_rollout_thread_ids(
    derived_metadata: &HashMap<String, RolloutThreadMetadata>,
) -> HashSet<String> {
    derived_metadata
        .iter()
        .filter(|(_, metadata)| !metadata.archived)
        .map(|(session_id, _)| session_id.clone())
        .collect()
}

fn read_sqlite_thread_ids(connection: &Connection) -> Result<HashSet<String>, rusqlite::Error> {
    let mut statement = connection.prepare("SELECT id FROM threads")?;
    let rows = statement.query_map([], |row| row.get::<usize, String>(0))?;
    let mut ids = HashSet::new();
    for row in rows {
        let id = row?;
        if !id.trim().is_empty() {
            ids.insert(id);
        }
    }
    Ok(ids)
}

fn update_sqlite_provider(data_dir: &Path, target_provider: &str) -> Result<usize, String> {
    let mut derived_metadata =
        collect_thread_metadata_from_rollout_headers(data_dir).unwrap_or_default();
    update_sqlite_provider_with_metadata(data_dir, target_provider, &mut derived_metadata)
}

fn update_sqlite_provider_with_metadata(
    data_dir: &Path,
    target_provider: &str,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<usize, String> {
    let mut updated_rows = 0usize;

    for db_path in existing_state_db_paths(data_dir) {
        let mut connection = match Connection::open(&db_path) {
            Ok(connection) => connection,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "打开实例数据库失败 ({}): {}",
                    db_path.display(),
                    error
                ));
            }
        };
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|error| {
                format!(
                    "设置 SQLite busy_timeout 失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;
        let columns = match read_threads_table_columns(&connection) {
            Ok(columns) => columns,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) if is_missing_threads_table_error(&error) => continue,
            Err(error) => {
                return Err(format_sqlite_read_error(
                    &db_path,
                    "读取 SQLite threads 表结构失败",
                    &error,
                ));
            }
        };
        let Some(columns) = columns else {
            continue;
        };
        let repairs = collect_sqlite_thread_repairs(
            data_dir,
            &connection,
            columns,
            target_provider,
            derived_metadata,
        )
        .map_err(|error| {
            format_sqlite_read_error(&db_path, "读取 SQLite 会话修复候选失败", &error)
        })?;
        updated_rows += apply_sqlite_thread_repairs(&mut connection, &db_path, &repairs)?;
    }

    Ok(updated_rows)
}

fn read_threads_table_columns(
    connection: &Connection,
) -> Result<Option<ThreadsTableColumns>, rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA table_info(threads)")?;
    let rows = statement.query_map([], |row| row.get::<usize, String>(1))?;
    let mut names = HashSet::new();
    for row in rows {
        let name = row?;
        names.insert(name);
    }
    if names.is_empty() || !names.contains("id") {
        return Ok(None);
    }
    Ok(Some(ThreadsTableColumns {
        source: names.contains("source"),
        model_provider: names.contains("model_provider"),
        cwd: names.contains("cwd"),
        title: names.contains("title"),
        preview: names.contains("preview"),
        has_user_event: names.contains("has_user_event"),
        first_user_message: names.contains("first_user_message"),
        thread_source: names.contains("thread_source"),
        rollout_path: names.contains("rollout_path"),
        archived: names.contains("archived"),
        updated_at: names.contains("updated_at"),
        updated_at_ms: names.contains("updated_at_ms"),
    }))
}

fn collect_sqlite_thread_repairs(
    data_dir: &Path,
    connection: &Connection,
    columns: ThreadsTableColumns,
    target_provider: &str,
    derived_metadata: &mut HashMap<String, RolloutThreadMetadata>,
) -> Result<Vec<SqliteThreadRepair>, rusqlite::Error> {
    let session_index_map = read_session_index_map(data_dir).unwrap_or_default();
    let model_provider_expr = if columns.model_provider {
        "COALESCE(model_provider, '')"
    } else {
        "''"
    };
    let cwd_expr = if columns.cwd {
        "COALESCE(cwd, '')"
    } else {
        "''"
    };
    let rollout_path_expr = if columns.rollout_path {
        "COALESCE(rollout_path, '')"
    } else {
        "''"
    };
    let archived_expr = if columns.archived {
        "COALESCE(archived, 0)"
    } else {
        "0"
    };
    let title_expr = if columns.title {
        "COALESCE(title, '')"
    } else {
        "''"
    };
    let preview_expr = if columns.preview {
        "COALESCE(preview, '')"
    } else {
        "''"
    };
    let has_user_event_expr = if columns.has_user_event {
        "COALESCE(has_user_event, 0)"
    } else {
        "0"
    };
    let first_user_message_expr = if columns.first_user_message {
        "COALESCE(first_user_message, '')"
    } else {
        "''"
    };
    let thread_source_expr = if columns.thread_source {
        "COALESCE(thread_source, '')"
    } else {
        "''"
    };
    let sql = format!(
        "SELECT id, {model_provider_expr}, {cwd_expr}, {rollout_path_expr}, {archived_expr}, {title_expr}, {preview_expr}, {has_user_event_expr}, {first_user_message_expr}, {thread_source_expr} FROM threads"
    );
    let mut statement = connection.prepare(sql.as_str())?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;
    let mut repairs = Vec::new();
    for row in rows {
        let (
            id,
            current_model_provider,
            current_cwd,
            current_rollout_path,
            current_archived,
            current_title,
            current_preview,
            current_has_user_event,
            current_first_user_message,
            current_thread_source,
        ) = row?;
        let indexed_title = session_index_map.get(&id).and_then(session_index_title);
        if sqlite_row_needs_rollout_body_metadata(
            columns,
            current_title.trim(),
            current_preview.trim(),
            current_has_user_event,
            current_first_user_message.trim(),
            current_thread_source.trim(),
            derived_metadata.get(&id),
            indexed_title.as_deref(),
        ) {
            complete_rollout_metadata_for_session(&id, indexed_title.clone(), derived_metadata);
        }
        let derived = derived_metadata.get(&id);
        let derived_title = derived
            .and_then(|metadata| metadata.title.clone())
            .or(indexed_title)
            .or_else(|| normalize_display_text(&id, THREAD_TITLE_MAX_CHARS));
        let derived_preview = derived
            .and_then(|metadata| metadata.preview.clone())
            .or_else(|| derived_title.clone());
        let derived_first_user_message =
            derived.and_then(|metadata| metadata.first_user_message.clone());
        let derived_active_rollout =
            derived.filter(|metadata| !metadata.archived && current_archived == 0);
        let has_user_text = !current_first_user_message.trim().is_empty()
            || derived_first_user_message
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());

        let repair = SqliteThreadRepair {
            id,
            model_provider: if columns.model_provider
                && current_model_provider.trim() != target_provider
            {
                Some(target_provider.to_string())
            } else {
                None
            },
            cwd: if columns.cwd {
                derived_active_rollout
                    .and_then(|metadata| metadata.cwd.clone())
                    .filter(|cwd| !path_text_matches(current_cwd.trim(), cwd.trim()))
            } else {
                None
            },
            rollout_path: if columns.rollout_path {
                derived_active_rollout
                    .map(|metadata| metadata.rollout_path.to_string_lossy().to_string())
                    .filter(|rollout_path| {
                        !paths_refer_to_same_location(
                            &resolve_rollout_path(data_dir, current_rollout_path.trim()),
                            &resolve_rollout_path(data_dir, rollout_path),
                        )
                    })
            } else {
                None
            },
            title: if columns.title && current_title.trim().is_empty() {
                derived_title.clone()
            } else {
                None
            },
            preview: if columns.preview && current_preview.trim().is_empty() {
                derived_preview.clone()
            } else {
                None
            },
            has_user_event: if columns.has_user_event
                && current_has_user_event != 1
                && has_user_text
            {
                Some(1)
            } else {
                None
            },
            first_user_message: if columns.first_user_message
                && current_first_user_message.trim().is_empty()
            {
                derived_first_user_message.clone()
            } else {
                None
            },
            thread_source: if columns.thread_source
                && current_thread_source.trim().is_empty()
                && has_user_text
            {
                Some("user".to_string())
            } else {
                None
            },
        };
        if sqlite_thread_repair_has_changes(&repair) {
            repairs.push(repair);
        }
    }
    Ok(repairs)
}

fn sqlite_row_needs_rollout_body_metadata(
    columns: ThreadsTableColumns,
    current_title: &str,
    current_preview: &str,
    current_has_user_event: i64,
    current_first_user_message: &str,
    current_thread_source: &str,
    derived: Option<&RolloutThreadMetadata>,
    indexed_title: Option<&str>,
) -> bool {
    let derived_title_missing = derived
        .and_then(|metadata| metadata.title.as_ref())
        .is_none();
    let derived_preview_missing = derived
        .and_then(|metadata| metadata.preview.as_ref())
        .is_none();
    let derived_first_user_message_missing = derived
        .and_then(|metadata| metadata.first_user_message.as_ref())
        .is_none();
    let indexed_title_missing = indexed_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none();
    let current_first_user_message_missing = current_first_user_message.trim().is_empty();

    if columns.title
        && current_title.trim().is_empty()
        && indexed_title_missing
        && derived_title_missing
    {
        return true;
    }
    if columns.preview && current_preview.trim().is_empty() && derived_preview_missing {
        return true;
    }
    if columns.first_user_message
        && current_first_user_message_missing
        && derived_first_user_message_missing
    {
        return true;
    }
    if columns.has_user_event
        && current_has_user_event != 1
        && current_first_user_message_missing
        && derived_first_user_message_missing
    {
        return true;
    }
    columns.thread_source
        && current_thread_source.trim().is_empty()
        && current_first_user_message_missing
        && derived_first_user_message_missing
}

fn sqlite_thread_repair_has_changes(repair: &SqliteThreadRepair) -> bool {
    repair.model_provider.is_some()
        || repair.cwd.is_some()
        || repair.rollout_path.is_some()
        || repair.title.is_some()
        || repair.preview.is_some()
        || repair.has_user_event.is_some()
        || repair.first_user_message.is_some()
        || repair.thread_source.is_some()
}

fn apply_sqlite_thread_repairs(
    connection: &mut Connection,
    db_path: &Path,
    repairs: &[SqliteThreadRepair],
) -> Result<usize, String> {
    if repairs.is_empty() {
        return Ok(0);
    }

    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let mut updated_rows = 0usize;
    for repair in repairs {
        let mut assignments = Vec::<&str>::new();
        let mut params = Vec::<SqlValue>::new();
        if let Some(value) = &repair.model_provider {
            assignments.push("model_provider = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = &repair.cwd {
            assignments.push("cwd = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = &repair.rollout_path {
            assignments.push("rollout_path = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = &repair.title {
            assignments.push("title = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = &repair.preview {
            assignments.push("preview = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = repair.has_user_event {
            assignments.push("has_user_event = ?");
            params.push(SqlValue::Integer(value));
        }
        if let Some(value) = &repair.first_user_message {
            assignments.push("first_user_message = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if let Some(value) = &repair.thread_source {
            assignments.push("thread_source = ?");
            params.push(SqlValue::Text(value.clone()));
        }
        if assignments.is_empty() {
            continue;
        }
        params.push(SqlValue::Text(repair.id.clone()));
        let sql = format!("UPDATE threads SET {} WHERE id = ?", assignments.join(", "));
        updated_rows += transaction
            .execute(sql.as_str(), params_from_iter(params))
            .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    }
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updated_rows)
}

fn update_sqlite_thread_sources(data_dir: &Path, session_ids: &[String]) -> Result<usize, String> {
    if session_ids.is_empty() {
        return Ok(0);
    }
    let mut updated_rows = 0usize;
    let unique_ids = session_ids.iter().collect::<HashSet<_>>();
    for db_path in existing_state_db_paths(data_dir) {
        let mut connection = match Connection::open(&db_path) {
            Ok(connection) => connection,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) => {
                return Err(format!(
                    "打开实例数据库失败 ({}): {}",
                    db_path.display(),
                    error
                ));
            }
        };
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|error| {
                format!(
                    "设置 SQLite busy_timeout 失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;
        let columns = match read_threads_table_columns(&connection) {
            Ok(columns) => columns,
            Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                log_skipped_sqlite_database(&db_path, &error.to_string());
                continue;
            }
            Err(error) if is_missing_threads_table_error(&error) => continue,
            Err(error) => {
                return Err(format_sqlite_read_error(
                    &db_path,
                    "读取 SQLite threads 表结构失败",
                    &error,
                ));
            }
        };
        let Some(columns) = columns else {
            continue;
        };
        if !columns.source {
            continue;
        }
        let transaction = connection
            .transaction()
            .map_err(|error| format_sqlite_write_error(&db_path, &error))?;
        for session_id in &unique_ids {
            let sql = if columns.archived {
                "UPDATE threads SET source = ?1 WHERE id = ?2 AND archived = 0 AND source <> ?1"
            } else {
                "UPDATE threads SET source = ?1 WHERE id = ?2 AND source <> ?1"
            };
            updated_rows += transaction
                .execute(sql, (CODEX_APP_VISIBLE_SOURCE, session_id.as_str()))
                .map_err(|error| format_sqlite_write_error(&db_path, &error))?;
        }
        transaction
            .commit()
            .map_err(|error| format_sqlite_write_error(&db_path, &error))?;
    }
    Ok(updated_rows)
}

fn format_sqlite_read_error(path: &Path, action: &str, error: &rusqlite::Error) -> String {
    format!("{} ({}): {}", action, path.display(), error)
}

fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("database is locked") || lowered.contains("database busy") {
        return format!(
            "state_5.sqlite 当前被占用，请关闭 Codex / Codex App 后重试 ({}): {}",
            path.display(),
            message
        );
    }
    format!(
        "更新 SQLite 会话可见性失败 ({}): {}",
        path.display(),
        message
    )
}

fn rewrite_rollout_provider(change: &RolloutProviderChange) -> Result<(), String> {
    let original_modified_at =
        modules::codex_session_file_time::read_modified_time(&change.absolute_path);
    if let Some(updated_first_line) = change.updated_first_line.as_deref() {
        let bytes = fs::read(&change.absolute_path).map_err(|error| {
            format!(
                "读取 rollout 文件失败 ({}): {}",
                change.absolute_path.display(),
                error
            )
        })?;
        let (offset, separator) = detect_first_line_boundary(&bytes);
        let mut next_bytes = Vec::with_capacity(updated_first_line.len() + bytes.len());
        next_bytes.extend_from_slice(updated_first_line.as_bytes());
        next_bytes.extend_from_slice(separator.as_bytes());
        next_bytes.extend_from_slice(&bytes[offset..]);
        write_bytes_atomic(&change.absolute_path, &next_bytes)?;
    }
    modules::codex_session_file_time::restore_modified_time(
        &change.absolute_path,
        change.target_modified_at.or(original_modified_at),
    )
}

pub fn normalize_session_sources_for_rollout_paths(
    data_dir: &Path,
    rollout_paths: &[PathBuf],
) -> Result<usize, String> {
    if rollout_paths.is_empty() {
        return Ok(0);
    }
    let mut normalized_count = 0usize;
    let mut session_ids = Vec::new();
    for rollout_path in rollout_paths {
        let Some((first_line, separator)) = read_first_line(rollout_path)? else {
            continue;
        };
        let Some(mut parsed) = parse_session_meta_record(&first_line) else {
            continue;
        };
        let Some(session_id) = session_meta_id(&parsed) else {
            continue;
        };
        if session_meta_source(&parsed).as_deref() == Some(CODEX_APP_VISIBLE_SOURCE) {
            continue;
        }
        let original_modified_at =
            modules::codex_session_file_time::read_modified_time(rollout_path);
        let Some(payload) = parsed.get_mut("payload").and_then(JsonValue::as_object_mut) else {
            continue;
        };
        payload.insert(
            "source".to_string(),
            JsonValue::String(CODEX_APP_VISIBLE_SOURCE.to_string()),
        );
        let updated_first_line = serde_json::to_string(&parsed)
            .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
        let bytes = fs::read(rollout_path).map_err(|error| {
            format!(
                "读取 rollout 文件失败 ({}): {}",
                rollout_path.display(),
                error
            )
        })?;
        let (offset, _) = detect_first_line_boundary(&bytes);
        let mut next_bytes = Vec::with_capacity(updated_first_line.len() + bytes.len());
        next_bytes.extend_from_slice(updated_first_line.as_bytes());
        next_bytes.extend_from_slice(separator.as_bytes());
        next_bytes.extend_from_slice(&bytes[offset..]);
        write_bytes_atomic(rollout_path, &next_bytes)?;
        modules::codex_session_file_time::restore_modified_time(
            rollout_path,
            original_modified_at,
        )?;
        normalized_count += 1;
        session_ids.push(session_id);
    }
    update_sqlite_thread_sources(data_dir, &session_ids)?;
    Ok(normalized_count)
}

fn detect_first_line_boundary(bytes: &[u8]) -> (usize, &'static str) {
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                return (index + 1, "\r\n");
            }
            return (index + 1, "\n");
        }
    }
    (bytes.len(), "")
}

fn write_bytes_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("无法定位目标目录: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建目录失败 ({}): {}", parent.display(), error))?;

    let temp_path = parent.join(format!(
        ".{}.provider-repair.{}.{}",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&temp_path, content)
        .map_err(|error| format!("写入临时文件失败 ({}): {}", temp_path.display(), error))?;
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("替换文件失败 ({}): {}", path.display(), error));
    }
    Ok(())
}

fn sqlite_sidecar_paths(db_path: &Path) -> Vec<PathBuf> {
    let raw = db_path.to_string_lossy();
    vec![
        PathBuf::from(format!("{}-wal", raw)),
        PathBuf::from(format!("{}-shm", raw)),
    ]
}

fn remove_sqlite_sidecar_files(db_path: &Path) -> Result<(), String> {
    for path in sqlite_sidecar_paths(db_path) {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "清理 SQLite sidecar 文件失败 ({}): {}",
                    path.display(),
                    error
                ));
            }
        }
    }
    Ok(())
}

fn backup_sqlite_database(data_dir: &Path, backup_dir: &Path) -> Result<bool, String> {
    let mut backed_up = false;
    for db_path in existing_state_db_paths(data_dir) {
        let relative_path = db_path
            .strip_prefix(data_dir)
            .map_err(|_| format!("无法计算 SQLite 备份相对路径: {}", db_path.display()))?;
        let backup_db_path = backup_dir.join(relative_path);
        if let Some(parent) = backup_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 备份目录失败 ({}): {}", parent.display(), error)
            })?;
        }

        let connection = Connection::open(&db_path).map_err(|error| {
            format!(
                "打开 state_5.sqlite 以创建一致备份失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(|error| {
                format!(
                    "设置 SQLite 备份 busy_timeout 失败 ({}): {}",
                    db_path.display(),
                    error
                )
            })?;

        if backup_db_path.exists() {
            fs::remove_file(&backup_db_path).map_err(|error| {
                format!(
                    "删除旧 state_5.sqlite 备份失败 ({}): {}",
                    backup_db_path.display(),
                    error
                )
            })?;
        }
        let backup_target = backup_db_path.to_string_lossy().to_string();
        connection
            .execute("VACUUM main INTO ?1", [backup_target.as_str()])
            .map_err(|error| {
                format!(
                    "备份 state_5.sqlite 失败 ({} -> {}): {}",
                    db_path.display(),
                    backup_db_path.display(),
                    error
                )
            })?;
        backed_up = true;
    }
    Ok(backed_up)
}

fn restore_sqlite_database_from_backup(data_dir: &Path, backup_dir: &Path) -> Result<bool, String> {
    let mut restored = false;
    let mut seen = HashSet::new();
    for relative_path in STATE_DB_RELATIVE_PATHS {
        let backup_db_path = backup_dir.join(relative_path);
        if !backup_db_path.exists() {
            continue;
        }
        let target_db_path = data_dir.join(relative_path);
        let key = target_db_path.to_string_lossy().to_string();
        if !seen.insert(key) {
            continue;
        }

        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 state_5.sqlite 恢复目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        } else {
            fs::create_dir_all(data_dir).map_err(|error| {
                format!(
                    "创建 state_5.sqlite 恢复目录失败 ({}): {}",
                    data_dir.display(),
                    error
                )
            })?;
        }
        remove_sqlite_sidecar_files(&target_db_path)?;
        fs::copy(&backup_db_path, &target_db_path).map_err(|error| {
            format!(
                "恢复 state_5.sqlite 失败 ({} -> {}): {}",
                backup_db_path.display(),
                target_db_path.display(),
                error
            )
        })?;
        remove_sqlite_sidecar_files(&target_db_path)?;
        restored = true;
    }
    Ok(restored)
}

fn backup_instance_files(
    data_dir: &Path,
    rollout_changes: &[RolloutProviderChange],
    source_repairs: &[RolloutSourceRepair],
    include_sqlite: bool,
    include_session_index: bool,
    include_global_state: bool,
    instance_id: &str,
    target_provider: &str,
) -> Result<PathBuf, String> {
    let backup_dir_name = format!(
        "{}{}{}",
        SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX,
        Utc::now().format("%Y%m%d-%H%M%S"),
        SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX
    );
    let backup_dir = data_dir.join(backup_dir_name);
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("创建备份目录失败 ({}): {}", backup_dir.display(), error))?;

    let mut backed_up_files = Vec::new();
    let mut backed_up_relative_paths = HashSet::new();
    let mut sqlite_backup_created = false;
    for change in rollout_changes {
        if !backed_up_relative_paths.insert(change.relative_path.clone()) {
            continue;
        }
        let target = backup_dir.join("files").join(&change.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 备份目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&change.absolute_path, &target).map_err(|error| {
            format!(
                "备份 rollout 文件失败 ({} -> {}): {}",
                change.absolute_path.display(),
                target.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target,
            modules::codex_session_file_time::read_modified_time(&change.absolute_path),
        )?;
        backed_up_files.push(change.relative_path.to_string_lossy().to_string());
    }
    for repair in source_repairs {
        if !backed_up_relative_paths.insert(repair.relative_path.clone()) {
            continue;
        }
        let target = backup_dir.join("files").join(&repair.relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "创建 rollout 来源备份目录失败 ({}): {}",
                    parent.display(),
                    error
                )
            })?;
        }
        fs::copy(&repair.absolute_path, &target).map_err(|error| {
            format!(
                "备份 rollout 来源文件失败 ({} -> {}): {}",
                repair.absolute_path.display(),
                target.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target,
            modules::codex_session_file_time::read_modified_time(&repair.absolute_path),
        )?;
        backed_up_files.push(repair.relative_path.to_string_lossy().to_string());
    }

    if include_sqlite {
        sqlite_backup_created = backup_sqlite_database(data_dir, &backup_dir)?;
    }

    let mut session_index_backup_created = false;
    if include_session_index {
        let source = data_dir.join(SESSION_INDEX_FILE);
        if source.exists() {
            let target = backup_dir.join("files").join(SESSION_INDEX_FILE);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "创建 session_index 备份目录失败 ({}): {}",
                        parent.display(),
                        error
                    )
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "备份 session_index.jsonl 失败 ({} -> {}): {}",
                    source.display(),
                    target.display(),
                    error
                )
            })?;
            session_index_backup_created = true;
        }
    }

    let mut global_state_backup_created = false;
    if include_global_state {
        let source = data_dir.join(GLOBAL_STATE_FILE);
        if source.exists() {
            let target = backup_dir.join("files").join(GLOBAL_STATE_FILE);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("创建全局状态备份目录失败 ({}): {}", parent.display(), error)
                })?;
            }
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "备份 Codex 全局状态失败 ({} -> {}): {}",
                    source.display(),
                    target.display(),
                    error
                )
            })?;
            global_state_backup_created = true;
        }
    }

    let manifest = json!({
        "instanceId": instance_id,
        "instanceRoot": data_dir,
        "targetProvider": target_provider,
        "createdAt": Utc::now().to_rfc3339(),
        "hasSqliteBackup": sqlite_backup_created,
        "hasSessionIndexBackup": session_index_backup_created,
        "hasGlobalStateBackup": global_state_backup_created,
        "rolloutFiles": backed_up_files,
    });
    fs::write(
        backup_dir.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&manifest)
                .map_err(|error| format!("序列化可见性修复备份清单失败: {}", error))?
        ),
    )
    .map_err(|error| {
        format!(
            "写入可见性修复备份清单失败 ({}): {}",
            backup_dir.display(),
            error
        )
    })?;

    Ok(backup_dir)
}

fn parse_session_visibility_repair_backup_timestamp(name: &str) -> Option<&str> {
    let timestamp = name
        .strip_prefix(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)?
        .strip_suffix(SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX)?;
    if timestamp.len() != 15 {
        return None;
    }
    if !timestamp.chars().enumerate().all(|(index, value)| {
        if index == 8 {
            value == '-'
        } else {
            value.is_ascii_digit()
        }
    }) {
        return None;
    }
    Some(timestamp)
}

fn prune_session_visibility_repair_backups(instances: &[CodexSyncInstance]) {
    for instance in instances {
        if let Err(error) = prune_instance_session_visibility_repair_backups(&instance.data_dir) {
            modules::logger::log_warn(&format!(
                "清理 Codex 会话可见性修复旧备份失败 ({}): {}",
                instance.data_dir.display(),
                error
            ));
        }
    }
}

fn prune_instance_session_visibility_repair_backups(data_dir: &Path) -> Result<(), String> {
    let entries = match fs::read_dir(data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "读取实例目录失败 ({}): {}",
                data_dir.display(),
                error
            ));
        }
    };
    let mut backups: Vec<(String, PathBuf)> = Vec::new();

    for entry in entries {
        let entry = entry
            .map_err(|error| format!("读取实例目录项失败 ({}): {}", data_dir.display(), error))?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取实例目录项类型失败 ({}): {}",
                entry.path().display(),
                error
            )
        })?;
        if !file_type.is_dir() {
            continue;
        }

        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(timestamp) = parse_session_visibility_repair_backup_timestamp(file_name) else {
            continue;
        };
        backups.push((timestamp.to_string(), entry.path()));
    }

    if backups.len() <= MAX_SESSION_VISIBILITY_REPAIR_BACKUPS {
        return Ok(());
    }

    backups.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in backups
        .into_iter()
        .skip(MAX_SESSION_VISIBILITY_REPAIR_BACKUPS)
    {
        fs::remove_dir_all(&path)
            .map_err(|error| format!("删除旧备份失败 ({}): {}", path.display(), error))?;
    }

    Ok(())
}

fn restore_instance_files_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
    include_sqlite: bool,
) -> Result<(), String> {
    let files_root = backup_dir.join("files");
    if files_root.exists() {
        restore_directory_contents(&files_root, data_dir)?;
    }

    if include_sqlite {
        let _ = restore_sqlite_database_from_backup(data_dir, backup_dir)?;
    }

    Ok(())
}

fn restore_directory_contents(source_root: &Path, target_root: &Path) -> Result<(), String> {
    let entries = fs::read_dir(source_root)
        .map_err(|error| format!("读取备份目录失败 ({}): {}", source_root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取备份目录项失败 ({}): {}", source_root.display(), error)
        })?;
        let source_path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取备份文件类型失败 ({}): {}",
                source_path.display(),
                error
            )
        })?;
        let relative = source_path
            .strip_prefix(source_root)
            .map_err(|_| format!("无法计算备份相对路径: {}", source_path.display()))?;
        let target_path = target_root.join(relative);

        if file_type.is_dir() {
            fs::create_dir_all(&target_path).map_err(|error| {
                format!("创建恢复目录失败 ({}): {}", target_path.display(), error)
            })?;
            restore_directory_contents(&source_path, &target_path)?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建恢复父目录失败 ({}): {}", parent.display(), error))?;
        }
        fs::copy(&source_path, &target_path).map_err(|error| {
            format!(
                "恢复备份文件失败 ({} -> {}): {}",
                source_path.display(),
                target_path.display(),
                error
            )
        })?;
        modules::codex_session_file_time::restore_modified_time(
            &target_path,
            modules::codex_session_file_time::read_modified_time(&source_path),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn set_modified_time(path: &Path, modified_at: SystemTime) {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file for mtime update");
        file.set_modified(modified_at)
            .expect("set file modified time");
        drop(file);
    }

    #[test]
    fn summary_message_includes_restart_instruction_when_no_changes_are_needed() {
        let message = build_summary_message(0, 0, 0, 0, 0, 0, 0, 0, 0, 0);

        assert!(
            message.ends_with("请手动彻底退出Codex进程后再启动"),
            "message should end with the Codex restart instruction: {message}"
        );
    }

    #[test]
    fn rollout_repair_updates_provider_and_preserves_session_time() {
        let data_dir = make_temp_dir("codex-session-visibility-rollout-time-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("05").join("23");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"old\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .expect("write rollout");
        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"Test\",\"updated_at\":\"2024-02-03T04:05:06Z\"}\n",
        )
        .expect("write session index");
        let polluted_modified_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        set_modified_time(&rollout_path, polluted_modified_at);

        let changes =
            collect_rollout_provider_changes(&data_dir, "relay").expect("collect rollout changes");
        assert_eq!(changes.len(), 1);

        let mut metadata = HashMap::new();
        repair_single_instance(
            &data_dir,
            "relay",
            &changes,
            &[],
            false,
            0,
            false,
            false,
            0,
            &[],
            &mut metadata,
        )
        .expect("repair rollout");

        let content = fs::read_to_string(&rollout_path).expect("read repaired rollout");
        assert!(content.contains("\"model_provider\":\"relay\""));
        assert_eq!(
            fs::metadata(&rollout_path)
                .expect("rollout metadata")
                .modified()
                .expect("rollout mtime"),
            UNIX_EPOCH + Duration::from_secs(1_704_067_200)
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn rollout_repair_restores_session_time_without_provider_change() {
        let data_dir = make_temp_dir("codex-session-visibility-mtime-only-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("05").join("23");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");
        let polluted_modified_at = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        set_modified_time(&rollout_path, polluted_modified_at);

        let changes =
            collect_rollout_provider_changes(&data_dir, "relay").expect("collect rollout changes");
        assert_eq!(changes.len(), 1);
        assert!(changes[0].updated_first_line.is_none());

        let mut metadata = HashMap::new();
        repair_single_instance(
            &data_dir,
            "relay",
            &changes,
            &[],
            false,
            0,
            false,
            false,
            0,
            &[],
            &mut metadata,
        )
        .expect("repair rollout time");

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read repaired rollout"),
            rollout_content
        );
        assert_eq!(
            fs::metadata(&rollout_path)
                .expect("rollout metadata")
                .modified()
                .expect("rollout mtime"),
            UNIX_EPOCH + Duration::from_secs(1_704_067_200)
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn source_repair_normalizes_active_rollout_and_sqlite_source() {
        let data_dir = make_temp_dir("codex-session-visibility-source-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"C:/work\",\"source\":\"vscode\",\"model_provider\":\"relay\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"item\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Restore visible folder\"}]}}}\n"
            ),
        )
        .expect("write rollout");

        let archived_dir = data_dir
            .join("archived_sessions")
            .join("2026")
            .join("06")
            .join("14");
        fs::create_dir_all(&archived_dir).expect("create archived dir");
        let archived_path = archived_dir.join("rollout-archived.jsonl");
        fs::write(
            &archived_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"archived-1\",\"cwd\":\"C:/work\",\"source\":\"vscode\",\"model_provider\":\"relay\"}}\n",
        )
        .expect("write archived rollout");

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    source TEXT,
                    archived INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, source, archived) VALUES
                 ('thread-1', 'vscode', 0),
                 ('archived-1', 'vscode', 1)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let instance = CodexSyncInstance {
            id: "instance-a".to_string(),
            name: "Instance A".to_string(),
            data_dir: data_dir.clone(),
            last_pid: None,
        };
        let candidates =
            collect_source_repair_candidates_for_instance(&instance).expect("collect candidates");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].session_id, "thread-1");

        let repaired =
            normalize_session_sources_for_rollout_paths(&data_dir, &[rollout_path.clone()])
                .expect("normalize sources");
        assert_eq!(repaired, 1);
        assert!(fs::read_to_string(&rollout_path)
            .expect("read rollout")
            .contains("\"source\":\"cli\""));
        assert!(fs::read_to_string(&archived_path)
            .expect("read archived rollout")
            .contains("\"source\":\"vscode\""));

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let active_source = connection
            .query_row(
                "SELECT source FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read active source");
        let archived_source = connection
            .query_row(
                "SELECT source FROM threads WHERE id = 'archived-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read archived source");
        assert_eq!(active_source, "cli");
        assert_eq!(archived_source, "vscode");

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_marks_threads_with_first_user_message_visible() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES
                 ('matched-invisible', 'relay', 0, 'hello', ''),
                 ('old-invisible', 'old', 0, 'hi', NULL),
                 ('already-visible', 'relay', 1, 'visible', 'user'),
                 ('provider-only', '', 0, '', NULL)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 3);
        assert!(!scan.skipped_unusable_database);

        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 3);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let matched_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'matched-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read matched row");
        assert_eq!(
            matched_invisible,
            ("relay".to_string(), 1, "user".to_string())
        );

        let old_invisible = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'old-invisible'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read old row");
        assert_eq!(old_invisible, ("relay".to_string(), 1, "user".to_string()));

        let provider_only = connection
            .query_row(
                "SELECT model_provider, has_user_event FROM threads WHERE id = 'provider-only'",
                [],
                |row| Ok((row.get::<usize, String>(0)?, row.get::<usize, i64>(1)?)),
            )
            .expect("read provider-only row");
        assert_eq!(provider_only, ("relay".to_string(), 0));

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_keeps_provider_only_schema_working() {
        let data_dir = make_temp_dir("codex-session-provider-only-sqlite-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('old', 'old'), ('same', 'relay')",
                [],
            )
            .expect("insert rows");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 1);
        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let old_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'old'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read old provider");
        assert_eq!(old_provider, "relay");

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_updates_nested_state_db_and_fills_display_metadata() {
        let data_dir = make_temp_dir("codex-session-visibility-nested-sqlite-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"item\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix restore visibility bug\"}]}}}\n"
            ),
        )
        .expect("write rollout");

        let db_path = data_dir.join("sqlite").join(STATE_DB_FILE);
        fs::create_dir_all(db_path.parent().expect("nested sqlite parent"))
            .expect("create nested sqlite dir");
        let connection = Connection::open(&db_path).expect("open nested sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    title TEXT,
                    preview TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (
                    id, model_provider, title, preview, has_user_event, first_user_message, thread_source
                ) VALUES ('thread-1', 'old', '', '', 0, '', NULL)",
                [],
            )
            .expect("insert thread row");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan nested sqlite");
        assert_eq!(scan.rows_to_update, 1);
        assert!(!scan.skipped_unusable_database);

        let updated_rows =
            update_sqlite_provider(&data_dir, "relay").expect("update nested sqlite");
        assert_eq!(updated_rows, 1);

        let connection = Connection::open(&db_path).expect("reopen nested sqlite");
        let row = connection
            .query_row(
                "SELECT model_provider, title, preview, has_user_event, first_user_message, thread_source
                 FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, String>(1)?,
                        row.get::<usize, String>(2)?,
                        row.get::<usize, i64>(3)?,
                        row.get::<usize, String>(4)?,
                        row.get::<usize, String>(5)?,
                    ))
                },
            )
            .expect("read repaired nested row");
        assert_eq!(
            row,
            (
                "relay".to_string(),
                "Fix restore visibility bug".to_string(),
                "Fix restore visibility bug".to_string(),
                1,
                "Fix restore visibility bug".to_string(),
                "user".to_string(),
            )
        );

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_repair_restores_project_cwd_and_rollout_path_from_active_rollout() {
        let data_dir = make_temp_dir("codex-session-visibility-cwd-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"cwd\":\"C:/Users/demo/project\",\"source\":\"cli\",\"model_provider\":\"relay\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"item\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Restore project folder\"}]}}}\n"
            ),
        )
        .expect("write rollout");

        let db_path = data_dir.join("sqlite").join(STATE_DB_FILE);
        fs::create_dir_all(db_path.parent().expect("nested sqlite parent"))
            .expect("create nested sqlite dir");
        let connection = Connection::open(&db_path).expect("open nested sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    cwd TEXT,
                    rollout_path TEXT,
                    archived INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, cwd, rollout_path, archived)
                 VALUES ('thread-1', 'relay', 'D:/stale/project', 'sessions/old.jsonl', 0)",
                [],
            )
            .expect("insert thread row");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 1);

        let updated_rows = update_sqlite_provider(&data_dir, "relay").expect("update sqlite");
        assert_eq!(updated_rows, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let row = connection
            .query_row(
                "SELECT cwd, rollout_path FROM threads WHERE id = 'thread-1'",
                [],
                |row| Ok((row.get::<usize, String>(0)?, row.get::<usize, String>(1)?)),
            )
            .expect("read repaired row");
        assert_eq!(row.0, "C:/Users/demo/project");
        assert_eq!(resolve_rollout_path(&data_dir, &row.1), rollout_path);

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_scan_detects_rollout_threads_missing_from_state_db() {
        let data_dir = make_temp_dir("codex-session-visibility-missing-sqlite-row-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-missing.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"missing-thread\",\"cwd\":\"C:/Users/demo/project\",\"source\":\"cli\",\"model_provider\":\"relay\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"item\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Restore missing SQLite row\"}]}}}\n"
            ),
        )
        .expect("write rollout");
        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"missing-thread\",\"thread_name\":\"Restore missing SQLite row\"}\n",
        )
        .expect("write session index");

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    model_provider TEXT,
                    cwd TEXT,
                    rollout_path TEXT,
                    archived INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider, cwd, rollout_path, archived)
                 VALUES ('existing-thread', 'relay', 'C:/Users/demo/project', 'sessions/existing.jsonl', 0)",
                [],
            )
            .expect("insert unrelated row");
        drop(connection);

        let scan = count_sqlite_rows_to_update(&data_dir, "relay").expect("scan sqlite");

        assert_eq!(scan.rows_to_update, 0);
        assert_eq!(scan.missing_thread_count, 1);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn header_metadata_collects_project_roots_without_reading_rollout_body() {
        let data_dir = make_temp_dir("codex-session-visibility-header-metadata-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            [
                br#"{"type":"session_meta","payload":{"id":"thread-1","cwd":"C:/Users/demo/project","source":"cli","model_provider":"relay"}}"#.as_slice(),
                b"\n",
                &[0xff, 0xfe, 0xfd],
            ]
            .concat(),
        )
        .expect("write rollout with invalid body");

        let metadata = collect_thread_metadata_from_rollout_headers(&data_dir)
            .expect("collect header metadata");
        let entries = sidebar_thread_entries_from_metadata(&metadata);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id, "thread-1");
        assert_eq!(
            entries[0].workspace_root.as_deref(),
            Some("C:/Users/demo/project")
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_backup_restore_replaces_db_and_clears_sidecars() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-backup-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('thread-1', 'old')",
                [],
            )
            .expect("insert old row");
        drop(connection);

        let backup_dir =
            backup_instance_files(&data_dir, &[], &[], true, false, false, "default", "relay")
                .expect("backup db");

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        connection
            .execute(
                "UPDATE threads SET model_provider = 'new' WHERE id = 'thread-1'",
                [],
            )
            .expect("mutate db after backup");
        drop(connection);
        for path in sqlite_sidecar_paths(&db_path) {
            fs::write(path, b"stale wal/shm").expect("write stale sidecar");
        }

        restore_instance_files_from_backup(&data_dir, &backup_dir, true).expect("restore db");
        for path in sqlite_sidecar_paths(&db_path) {
            assert!(
                !path.exists(),
                "stale sidecar should be removed: {:?}",
                path
            );
        }

        let connection = Connection::open(&db_path).expect("open restored sqlite");
        let provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read restored provider");
        assert_eq!(provider, "old");

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_backup_restore_replaces_nested_db_and_clears_sidecars() {
        let data_dir = make_temp_dir("codex-session-visibility-nested-backup-test");
        let db_path = data_dir.join("sqlite").join(STATE_DB_FILE);
        fs::create_dir_all(db_path.parent().expect("nested sqlite parent"))
            .expect("create nested sqlite dir");
        let connection = Connection::open(&db_path).expect("open nested sqlite");
        connection
            .execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT)",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, model_provider) VALUES ('thread-1', 'old')",
                [],
            )
            .expect("insert old row");
        drop(connection);

        let backup_dir =
            backup_instance_files(&data_dir, &[], &[], true, false, false, "default", "relay")
                .expect("backup nested db");

        let connection = Connection::open(&db_path).expect("reopen nested sqlite");
        connection
            .execute(
                "UPDATE threads SET model_provider = 'new' WHERE id = 'thread-1'",
                [],
            )
            .expect("mutate nested db after backup");
        drop(connection);
        for path in sqlite_sidecar_paths(&db_path) {
            fs::write(path, b"stale wal/shm").expect("write stale nested sidecar");
        }

        restore_instance_files_from_backup(&data_dir, &backup_dir, true)
            .expect("restore nested db");
        for path in sqlite_sidecar_paths(&db_path) {
            assert!(
                !path.exists(),
                "stale nested sidecar should be removed: {:?}",
                path
            );
        }

        let connection = Connection::open(&db_path).expect("open restored nested sqlite");
        let provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read restored nested provider");
        assert_eq!(provider, "old");

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn resolve_target_modified_prefers_rollout_activity_when_index_drifts() {
        let data_dir = make_temp_dir("codex-session-visibility-index-drift-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("08");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"s1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-01-01T00:00:00Z\"}\n",
        )
        .expect("write rollout");
        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"s1\",\"thread_name\":\"Test\",\"updated_at\":\"2026-03-16T23:36:58.7406859Z\"}\n",
        )
        .expect("write session index");

        let session_index_map = read_session_index_map(&data_dir).expect("read session index");
        let target =
            resolve_target_modified_at_ms(Some("s1"), &session_index_map, &rollout_path, None)
                .expect("resolve target modified");

        assert_eq!(target, 1_704_067_200_000);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sqlite_timestamp_repair_syncs_from_rollout_activity() {
        let data_dir = make_temp_dir("codex-session-visibility-sqlite-time-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("08");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"relay\"}}\n{\"type\":\"event\",\"timestamp\":\"2024-02-03T04:05:06Z\"}\n",
        )
        .expect("write rollout");
        let rollout_path_string = rollout_path.to_string_lossy().to_string();

        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    updated_at INTEGER,
                    updated_at_ms INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, updated_at, updated_at_ms) VALUES
                 ('thread-1', ?1, 1_800_000_000, 1_800_000_000_000)",
                [rollout_path_string.as_str()],
            )
            .expect("insert row");
        drop(connection);

        let updated = repair_sqlite_thread_timestamps(&data_dir).expect("repair sqlite timestamps");
        assert_eq!(updated, 1);

        let connection = Connection::open(&db_path).expect("reopen sqlite");
        let (updated_at, updated_at_ms) = connection
            .query_row(
                "SELECT updated_at, updated_at_ms FROM threads WHERE id = 'thread-1'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("read repaired timestamps");
        assert_eq!(updated_at, 1_706_933_106);
        assert_eq!(updated_at_ms, 1_706_933_106_000);

        drop(connection);
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn session_index_repair_appends_missing_and_updates_stale_sqlite_threads() {
        let data_dir = make_temp_dir("codex-session-visibility-index-test");
        let db_path = data_dir.join(STATE_DB_FILE);
        let connection = Connection::open(&db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    updated_at INTEGER
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, title, updated_at) VALUES
                 ('indexed-thread', 'Indexed', 1_700_000_000),
                 ('missing-thread', 'Missing chat', 1_800_000_000)",
                [],
            )
            .expect("insert rows");
        drop(connection);

        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"indexed-thread\",\"thread_name\":\"Indexed\",\"updated_at\":\"2024-01-01T00:00:00.0000000Z\"}\n",
        )
        .expect("write session index");

        let missing =
            count_missing_session_index_entries(&data_dir).expect("count missing index entries");
        assert_eq!(missing, 2);

        let repaired = reconcile_session_index_from_sqlite(&data_dir).expect("reconcile index");
        assert_eq!(repaired, 2);

        let index_map = read_session_index_map(&data_dir).expect("read session index");
        assert!(index_map.contains_key("missing-thread"));
        assert_eq!(
            index_map
                .get("missing-thread")
                .and_then(|entry| entry.get("thread_name"))
                .and_then(JsonValue::as_str),
            Some("Missing chat")
        );
        assert_ne!(
            index_map
                .get("indexed-thread")
                .and_then(|entry| entry.get("updated_at"))
                .and_then(JsonValue::as_str),
            Some("2024-01-01T00:00:00.0000000Z")
        );
        assert_eq!(
            count_missing_session_index_entries(&data_dir).expect("recount missing index entries"),
            0
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn session_index_repair_updates_blank_thread_name_from_nested_sqlite_and_rollout() {
        let data_dir = make_temp_dir("codex-session-visibility-index-repair-test");
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("14");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-test.jsonl");
        fs::write(
            &rollout_path,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"relay\"}}\n",
                "{\"type\":\"response_item\",\"payload\":{\"item\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Recover archived Codex sessions\"}]}}}\n"
            ),
        )
        .expect("write rollout");

        let db_path = data_dir.join("sqlite").join(STATE_DB_FILE);
        fs::create_dir_all(db_path.parent().expect("nested sqlite parent"))
            .expect("create nested sqlite dir");
        let connection = Connection::open(&db_path).expect("open nested sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    updated_at INTEGER,
                    rollout_path TEXT
                )",
                [],
            )
            .expect("create threads table");
        connection
            .execute(
                "INSERT INTO threads (id, title, updated_at, rollout_path) VALUES
                 ('thread-1', '', 1_800_000_000, ?1)",
                [rollout_path.to_string_lossy().as_ref()],
            )
            .expect("insert thread row");
        drop(connection);

        fs::write(
            data_dir.join(SESSION_INDEX_FILE),
            "{\"id\":\"thread-1\",\"thread_name\":\"\",\"updated_at\":\"2024-01-01T00:00:00.0000000Z\"}\n",
        )
        .expect("write incomplete session index");

        let pending =
            count_missing_session_index_entries(&data_dir).expect("count pending index repairs");
        assert_eq!(pending, 1);

        let repaired =
            reconcile_session_index_from_sqlite(&data_dir).expect("repair session index entries");
        assert_eq!(repaired, 1);

        let index_map = read_session_index_map(&data_dir).expect("read session index");
        assert_eq!(
            index_map
                .get("thread-1")
                .and_then(|entry| entry.get("thread_name"))
                .and_then(JsonValue::as_str),
            Some("Recover archived Codex sessions")
        );
        assert_eq!(
            count_missing_session_index_entries(&data_dir).expect("recount pending index repairs"),
            0
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }
}
