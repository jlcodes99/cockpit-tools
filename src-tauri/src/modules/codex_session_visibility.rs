use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::modules;
use chrono::{TimeZone, Utc};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

const DEFAULT_INSTANCE_ID: &str = "__default__";
const DEFAULT_INSTANCE_NAME: &str = "默认实例";
const DEFAULT_PROVIDER_ID: &str = "openai";
const STATE_DB_FILE: &str = "state_5.sqlite";
const SQLITE_DIR_NAME: &str = "sqlite";
const PREFERRED_SQLITE_DB_FILE: &str = "codex-dev.db";
const OFFICIAL_STATE_DB_FILE: &str = "state_5.sqlite";
const CONFIG_FILE_NAME: &str = "config.toml";
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX: &str = "backup-";
const SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX: &str = "-session-visibility-repair";
const MAX_SESSION_VISIBILITY_REPAIR_BACKUPS: usize = 1;
const SESSION_INDEX_ACTIVITY_DRIFT_MS: i128 = 3_600_000;
pub const SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT: &str =
    "codex:session_visibility_repair_progress";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSessionVisibilityRepairMode {
    Quick,
    Deep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSessionVisibilityAutoRepairMode {
    #[serde(rename = "legacy_before_4eb75d96")]
    LegacyBefore4eb75d96,
    #[serde(rename = "legacy_4eb75d96")]
    Legacy4eb75d96,
    Current,
}

impl Default for CodexSessionVisibilityAutoRepairMode {
    fn default() -> Self {
        Self::Current
    }
}

impl CodexSessionVisibilityAutoRepairMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::LegacyBefore4eb75d96 => "legacy_before_4eb75d96",
            Self::Legacy4eb75d96 => "legacy_4eb75d96",
            Self::Current => "current",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairProgress {
    pub run_id: Option<String>,
    pub mode: CodexSessionVisibilityRepairMode,
    pub stage: String,
    pub percent: u8,
    pub current: usize,
    pub total: usize,
    pub instance_id: Option<String>,
    pub instance_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexSessionVisibilityRepairProviderSource {
    Config,
    Rollout,
    Sqlite,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairProviderOption {
    pub id: String,
    pub sources: Vec<CodexSessionVisibilityRepairProviderSource>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairProviderList {
    pub default_provider: String,
    pub providers: Vec<CodexSessionVisibilityRepairProviderOption>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairInstanceOption {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    pub current_provider: String,
    pub is_default: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairInstanceList {
    pub default_instance_id: String,
    pub instances: Vec<CodexSessionVisibilityRepairInstanceOption>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairItem {
    pub instance_id: String,
    pub instance_name: String,
    pub target_provider: String,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub updated_sqlite_timestamp_row_count: usize,
    pub added_session_index_entry_count: usize,
    pub updated_session_index_entry_count: usize,
    pub skipped_sqlite_file: bool,
    pub metadata_rebuild_failed: bool,
    pub backup_dir: Option<String>,
    pub running: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSessionVisibilityRepairSummary {
    pub instance_count: usize,
    pub mutated_instance_count: usize,
    pub changed_rollout_file_count: usize,
    pub updated_sqlite_row_count: usize,
    pub updated_sqlite_timestamp_row_count: usize,
    pub added_session_index_entry_count: usize,
    pub updated_session_index_entry_count: usize,
    pub skipped_sqlite_file_count: usize,
    pub metadata_rebuild_failed_count: usize,
    pub items: Vec<CodexSessionVisibilityRepairItem>,
    pub backup_dirs: Vec<String>,
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
    updated_content: Option<RolloutProviderUpdate>,
    target_modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
enum RolloutProviderUpdate {
    FullContent(String),
    FirstLine(String),
}

#[derive(Debug, Clone, Copy)]
struct CodexSessionVisibilityRepairOptions {
    mode: CodexSessionVisibilityRepairMode,
    dry_run: bool,
    repair_rollout: bool,
    repair_referenced_rollouts: bool,
    rewrite_all_session_meta: bool,
    sqlite_scope: SqliteRepairScope,
    repair_sqlite_timestamps: bool,
    collect_rollout_thread_facts: bool,
    repair_session_index: bool,
    update_existing_session_index_entries: bool,
    rebuild_metadata: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqliteRepairScope {
    LegacyStateOnly,
    OfficialStateDbs,
    AllSessionDbs,
}

#[derive(Debug, Clone, Default)]
struct RepairTargetSelection {
    target_provider: Option<String>,
    session_ids: Option<HashSet<String>>,
    instance_ids: Option<HashSet<String>>,
}

impl RepairTargetSelection {
    fn from_inputs(
        target_provider: Option<String>,
        session_ids: Option<Vec<String>>,
        instance_ids: Option<Vec<String>>,
    ) -> Result<Self, String> {
        let target_provider = target_provider
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(provider) = target_provider.as_deref() {
            validate_provider_id(provider)?;
        }

        let session_ids = session_ids
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        let instance_ids = instance_ids
            .unwrap_or_default()
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<HashSet<_>>();
        Ok(Self {
            target_provider,
            session_ids: if session_ids.is_empty() {
                None
            } else {
                Some(session_ids)
            },
            instance_ids: if instance_ids.is_empty() {
                None
            } else {
                Some(instance_ids)
            },
        })
    }

    fn target_provider_for(&self, data_dir: &Path) -> Result<String, String> {
        match self.target_provider.as_ref() {
            Some(provider) => Ok(provider.clone()),
            None => read_target_provider(data_dir),
        }
    }

    fn includes_session_id(&self, session_id: &str) -> bool {
        self.session_ids
            .as_ref()
            .map(|ids| ids.contains(session_id))
            .unwrap_or(true)
    }

    fn includes_instance_id(&self, instance_id: &str) -> bool {
        self.instance_ids
            .as_ref()
            .map(|ids| ids.contains(instance_id))
            .unwrap_or(true)
    }

    fn has_session_filter(&self) -> bool {
        self.session_ids.is_some()
    }

    fn session_ids(&self) -> Option<&HashSet<String>> {
        self.session_ids.as_ref()
    }
}

impl CodexSessionVisibilityRepairOptions {
    fn official_state_db_only(mode: CodexSessionVisibilityRepairMode) -> Self {
        Self {
            mode,
            dry_run: false,
            repair_rollout: false,
            repair_referenced_rollouts: true,
            rewrite_all_session_meta: matches!(mode, CodexSessionVisibilityRepairMode::Deep),
            sqlite_scope: SqliteRepairScope::OfficialStateDbs,
            repair_sqlite_timestamps: false,
            collect_rollout_thread_facts: false,
            repair_session_index: false,
            update_existing_session_index_entries: false,
            rebuild_metadata: false,
        }
    }

    fn for_mode(mode: CodexSessionVisibilityRepairMode) -> Self {
        Self::official_state_db_only(mode)
    }

    fn for_auto_repair_mode(mode: CodexSessionVisibilityAutoRepairMode) -> Self {
        let _ = mode;
        Self::official_state_db_only(CodexSessionVisibilityRepairMode::Quick)
    }

    fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }
}

#[derive(Debug, Clone, Default)]
struct RolloutThreadFacts {
    user_event_thread_ids: HashSet<String>,
    cwd_by_thread_id: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct SqliteProviderScan {
    rows_to_update: usize,
    skipped_unusable_database: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionIndexRepairScan {
    entries_to_add: usize,
    entries_to_update: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct SessionIndexReconcileResult {
    added_entries: usize,
    updated_entries: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct RepairSingleInstanceResult {
    updated_sqlite_rows: usize,
    updated_sqlite_timestamp_rows: usize,
    added_session_index_entries: usize,
    updated_session_index_entries: usize,
}

#[derive(Debug, Clone)]
struct SqliteTimestampUpdate {
    id: String,
    updated_at_seconds: i64,
    updated_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
struct SqliteTimestampRepairPlan {
    updates: Vec<SqliteTimestampUpdate>,
    has_updated_at: bool,
    has_updated_at_ms: bool,
}

#[derive(Debug, Clone, Copy)]
struct ThreadsTableColumns {
    model_provider: bool,
    has_user_event: bool,
    first_user_message: bool,
    thread_source: bool,
    cwd: bool,
}

#[derive(Debug, Clone)]
struct SqliteThreadIndexRow {
    id: String,
    title: String,
    updated_at: Option<i64>,
    updated_at_ms: Option<i64>,
    rollout_path: Option<String>,
}

type ProgressReporter<'a> = Option<&'a dyn Fn(CodexSessionVisibilityRepairProgress)>;

pub fn repair_session_visibility_across_instances(
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    repair_session_visibility_across_instances_with_progress(
        CodexSessionVisibilityRepairMode::Quick,
        None,
        None,
    )
}

pub fn repair_session_visibility_quick_across_instances(
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    repair_session_visibility_auto_across_instances(CodexSessionVisibilityAutoRepairMode::Current)
}

pub fn repair_session_visibility_auto_across_instances(
    mode: CodexSessionVisibilityAutoRepairMode,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let started = std::time::Instant::now();
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] auto repair started: mode={}",
        mode.label()
    ));
    let result = repair_session_visibility_across_instances_with_options(
        CodexSessionVisibilityRepairOptions::for_auto_repair_mode(mode),
        None,
        None,
        RepairTargetSelection::default(),
    );
    match &result {
        Ok(summary) => modules::logger::log_info(&format!(
            "[Codex Session Visibility] auto repair finished: mode={}, instances={}, mutated_instances={}, rollout_files={}, sqlite_rows={}, sqlite_timestamp_rows={}, session_index_added={}, session_index_updated={}, metadata_failed={}, elapsed_ms={}",
            mode.label(),
            summary.instance_count,
            summary.mutated_instance_count,
            summary.changed_rollout_file_count,
            summary.updated_sqlite_row_count,
            summary.updated_sqlite_timestamp_row_count,
            summary.added_session_index_entry_count,
            summary.updated_session_index_entry_count,
            summary.metadata_rebuild_failed_count,
            started.elapsed().as_millis()
        )),
        Err(error) => modules::logger::log_warn(&format!(
            "[Codex Session Visibility] auto repair failed: mode={}, elapsed_ms={}, error={}",
            mode.label(),
            started.elapsed().as_millis(),
            error
        )),
    }
    result
}

pub fn repair_session_visibility_across_instances_with_progress(
    mode: CodexSessionVisibilityRepairMode,
    run_id: Option<String>,
    progress_reporter: ProgressReporter<'_>,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    repair_session_visibility_across_instances_with_target(
        mode,
        run_id,
        progress_reporter,
        None,
        None,
        None,
        false,
    )
}

pub fn repair_session_visibility_across_instances_with_target(
    mode: CodexSessionVisibilityRepairMode,
    run_id: Option<String>,
    progress_reporter: ProgressReporter<'_>,
    target_provider: Option<String>,
    session_ids: Option<Vec<String>>,
    instance_ids: Option<Vec<String>>,
    dry_run: bool,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let options = CodexSessionVisibilityRepairOptions::for_mode(mode).with_dry_run(dry_run);
    let selection = RepairTargetSelection::from_inputs(target_provider, session_ids, instance_ids)?;
    repair_session_visibility_across_instances_with_options(
        options,
        run_id,
        progress_reporter,
        selection,
    )
}

fn repair_session_visibility_across_instances_with_options(
    options: CodexSessionVisibilityRepairOptions,
    run_id: Option<String>,
    progress_reporter: ProgressReporter<'_>,
    selection: RepairTargetSelection,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    report_repair_progress(
        progress_reporter,
        &run_id,
        options,
        "collect_instances",
        2,
        0,
        0,
        None,
    );
    let instances = collect_instances()?
        .into_iter()
        .filter(|instance| selection.includes_instance_id(&instance.id))
        .collect::<Vec<_>>();
    repair_session_visibility_for_instances_with_options(
        options,
        run_id,
        progress_reporter,
        selection,
        instances,
    )
}

fn repair_session_visibility_for_instances_with_options(
    options: CodexSessionVisibilityRepairOptions,
    run_id: Option<String>,
    progress_reporter: ProgressReporter<'_>,
    selection: RepairTargetSelection,
    instances: Vec<CodexSyncInstance>,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    if instances.is_empty() {
        return Err("未找到要修复的 Codex 实例".to_string());
    }
    let process_entries = modules::process::collect_codex_process_entries();
    let mut items = Vec::with_capacity(instances.len());
    let mut backup_dirs = Vec::new();
    let mut mutated_instance_count = 0usize;
    let mut changed_rollout_file_count = 0usize;
    let mut updated_sqlite_row_count = 0usize;
    let mut updated_sqlite_timestamp_row_count = 0usize;
    let mut added_session_index_entry_count = 0usize;
    let mut updated_session_index_entry_count = 0usize;
    let mut skipped_sqlite_file_count = 0usize;
    let mut metadata_rebuild_failed_count = 0usize;
    let mut mutated_running_instance_count = 0usize;

    let total_instances = instances.len().max(1);
    report_repair_progress(
        progress_reporter,
        &run_id,
        options,
        "scan_instances",
        6,
        0,
        total_instances,
        None,
    );

    for (index, instance) in instances.iter().enumerate() {
        let current_instance = index + 1;
        report_repair_progress(
            progress_reporter,
            &run_id,
            options,
            "scan_instance",
            instance_progress_percent(index, total_instances, 0, 4),
            current_instance,
            total_instances,
            Some(instance),
        );
        let running = is_instance_running(instance, &process_entries);
        let target_provider = selection.target_provider_for(&instance.data_dir)?;
        let rollout_changes = if options.repair_rollout {
            collect_rollout_provider_changes(
                &instance.data_dir,
                &target_provider,
                options,
                &selection,
            )?
        } else if options.repair_referenced_rollouts {
            collect_referenced_rollout_provider_changes(
                &instance.data_dir,
                &target_provider,
                options,
                &selection,
            )?
        } else {
            Vec::new()
        };
        let sqlite_scan = count_sqlite_rows_to_update_for_options(
            &instance.data_dir,
            &target_provider,
            options,
            &selection,
        )?;
        let sqlite_rows_to_update = sqlite_scan.rows_to_update;
        let sqlite_timestamp_rows_to_update = if options.repair_sqlite_timestamps {
            count_sqlite_thread_timestamps_to_update_for_options(
                &instance.data_dir,
                options,
                &selection,
            )?
        } else {
            0
        };
        let session_index_scan = if options.repair_session_index {
            count_session_index_entries_to_repair_for_options(
                &instance.data_dir,
                options,
                &selection,
            )?
        } else {
            SessionIndexRepairScan::default()
        };
        if sqlite_scan.skipped_unusable_database {
            skipped_sqlite_file_count += 1;
        }

        let instance_has_planned_changes = !rollout_changes.is_empty()
            || sqlite_rows_to_update > 0
            || sqlite_timestamp_rows_to_update > 0
            || session_index_scan.entries_to_add > 0
            || session_index_scan.entries_to_update > 0;

        if options.dry_run {
            if instance_has_planned_changes {
                mutated_instance_count += 1;
                if running {
                    mutated_running_instance_count += 1;
                }
            }
            changed_rollout_file_count += rollout_changes.len();
            updated_sqlite_row_count += sqlite_rows_to_update;
            updated_sqlite_timestamp_row_count += sqlite_timestamp_rows_to_update;
            added_session_index_entry_count += session_index_scan.entries_to_add;
            updated_session_index_entry_count += session_index_scan.entries_to_update;
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: rollout_changes.len(),
                updated_sqlite_row_count: sqlite_rows_to_update,
                updated_sqlite_timestamp_row_count: sqlite_timestamp_rows_to_update,
                added_session_index_entry_count: session_index_scan.entries_to_add,
                updated_session_index_entry_count: session_index_scan.entries_to_update,
                skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
                metadata_rebuild_failed: false,
                backup_dir: None,
                running,
            });
            continue;
        }

        if !instance_has_planned_changes {
            let mut metadata_rebuild_failed = false;
            if options.rebuild_metadata {
                report_repair_progress(
                    progress_reporter,
                    &run_id,
                    options,
                    "rebuild_metadata",
                    instance_progress_percent(index, total_instances, 3, 4),
                    current_instance,
                    total_instances,
                    Some(instance),
                );
                if !try_rebuild_thread_metadata(instance) {
                    metadata_rebuild_failed = true;
                    metadata_rebuild_failed_count += 1;
                }
            }
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: 0,
                updated_sqlite_row_count: 0,
                updated_sqlite_timestamp_row_count: 0,
                added_session_index_entry_count: 0,
                updated_session_index_entry_count: 0,
                skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
                metadata_rebuild_failed,
                backup_dir: None,
                running,
            });
            continue;
        }

        report_repair_progress(
            progress_reporter,
            &run_id,
            options,
            "backup_instance",
            instance_progress_percent(index, total_instances, 1, 4),
            current_instance,
            total_instances,
            Some(instance),
        );
        let backup_dir = backup_instance_files(
            &instance.data_dir,
            &rollout_changes,
            sqlite_rows_to_update > 0 || sqlite_timestamp_rows_to_update > 0,
            session_index_scan.entries_to_add > 0 || session_index_scan.entries_to_update > 0,
            &instance.id,
            &target_provider,
            options,
        )?;
        let backup_dir_string = backup_dir.to_string_lossy().to_string();

        report_repair_progress(
            progress_reporter,
            &run_id,
            options,
            "write_instance",
            instance_progress_percent(index, total_instances, 2, 4),
            current_instance,
            total_instances,
            Some(instance),
        );
        let repaired = repair_single_instance_with_progress(
            &instance.data_dir,
            &target_provider,
            &rollout_changes,
            sqlite_rows_to_update > 0,
            sqlite_timestamp_rows_to_update > 0,
            session_index_scan.entries_to_add > 0 || session_index_scan.entries_to_update > 0,
            options,
            &selection,
            progress_reporter,
            &run_id,
            instance,
            index,
            total_instances,
        );
        let repaired = match repaired {
            Ok(value) => value,
            Err(error) => {
                let restore_result = restore_instance_files_from_backup(
                    &instance.data_dir,
                    &backup_dir,
                    sqlite_rows_to_update > 0 || sqlite_timestamp_rows_to_update > 0,
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

        let instance_mutated = !rollout_changes.is_empty()
            || repaired.updated_sqlite_rows > 0
            || repaired.updated_sqlite_timestamp_rows > 0
            || repaired.added_session_index_entries > 0
            || repaired.updated_session_index_entries > 0;
        let mut metadata_rebuild_failed = false;
        if options.rebuild_metadata && instance_mutated {
            report_repair_progress(
                progress_reporter,
                &run_id,
                options,
                "rebuild_metadata",
                instance_progress_percent(index, total_instances, 3, 4),
                current_instance,
                total_instances,
                Some(instance),
            );
        }
        if options.rebuild_metadata && instance_mutated && !try_rebuild_thread_metadata(instance) {
            metadata_rebuild_failed = true;
            metadata_rebuild_failed_count += 1;
        }

        if instance_mutated {
            mutated_instance_count += 1;
            if running {
                mutated_running_instance_count += 1;
            }
        }
        changed_rollout_file_count += rollout_changes.len();
        updated_sqlite_row_count += repaired.updated_sqlite_rows;
        updated_sqlite_timestamp_row_count += repaired.updated_sqlite_timestamp_rows;
        added_session_index_entry_count += repaired.added_session_index_entries;
        updated_session_index_entry_count += repaired.updated_session_index_entries;
        backup_dirs.push(backup_dir_string.clone());
        items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            target_provider,
            changed_rollout_file_count: rollout_changes.len(),
            updated_sqlite_row_count: repaired.updated_sqlite_rows,
            updated_sqlite_timestamp_row_count: repaired.updated_sqlite_timestamp_rows,
            added_session_index_entry_count: repaired.added_session_index_entries,
            updated_session_index_entry_count: repaired.updated_session_index_entries,
            skipped_sqlite_file: sqlite_scan.skipped_unusable_database,
            metadata_rebuild_failed,
            backup_dir: Some(backup_dir_string),
            running,
        });
    }

    if !options.dry_run {
        report_repair_progress(
            progress_reporter,
            &run_id,
            options,
            "prune_backups",
            96,
            total_instances,
            total_instances,
            None,
        );
        prune_session_visibility_repair_backups(&instances);
    }

    let message = if options.dry_run {
        build_dry_run_summary_message(
            mutated_instance_count,
            changed_rollout_file_count,
            updated_sqlite_row_count,
            updated_sqlite_timestamp_row_count,
            added_session_index_entry_count,
            updated_session_index_entry_count,
            mutated_running_instance_count,
        )
    } else {
        build_summary_message(
            mutated_instance_count,
            changed_rollout_file_count,
            updated_sqlite_row_count,
            updated_sqlite_timestamp_row_count,
            added_session_index_entry_count,
            updated_session_index_entry_count,
            mutated_running_instance_count,
            skipped_sqlite_file_count,
            metadata_rebuild_failed_count,
        )
    };

    let summary = CodexSessionVisibilityRepairSummary {
        instance_count: instances.len(),
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_session_index_entry_count,
        updated_session_index_entry_count,
        skipped_sqlite_file_count,
        metadata_rebuild_failed_count,
        items,
        backup_dirs,
        message,
    };
    report_repair_progress(
        progress_reporter,
        &run_id,
        options,
        "done",
        100,
        total_instances,
        total_instances,
        None,
    );
    Ok(summary)
}

pub fn list_session_visibility_repair_providers(
) -> Result<CodexSessionVisibilityRepairProviderList, String> {
    let instances = collect_instances()?;
    let default_provider = instances
        .first()
        .map(|instance| read_target_provider(&instance.data_dir))
        .transpose()?
        .unwrap_or_else(|| DEFAULT_PROVIDER_ID.to_string());

    let mut sources: HashMap<String, HashSet<CodexSessionVisibilityRepairProviderSource>> =
        HashMap::new();
    add_provider_source(
        &mut sources,
        default_provider.clone(),
        CodexSessionVisibilityRepairProviderSource::Config,
    );

    for instance in &instances {
        match list_configured_provider_ids(&instance.data_dir) {
            Ok(provider_ids) => {
                for provider_id in provider_ids {
                    add_provider_source(
                        &mut sources,
                        provider_id,
                        CodexSessionVisibilityRepairProviderSource::Config,
                    );
                }
            }
            Err(error) => modules::logger::log_warn(&format!(
                "读取 Codex provider 候选配置失败 ({}): {}",
                instance.data_dir.display(),
                error
            )),
        }

        match collect_rollout_provider_ids(&instance.data_dir) {
            Ok(provider_ids) => {
                for provider_id in provider_ids {
                    add_provider_source(
                        &mut sources,
                        provider_id,
                        CodexSessionVisibilityRepairProviderSource::Rollout,
                    );
                }
            }
            Err(error) => modules::logger::log_warn(&format!(
                "读取 Codex rollout provider 候选失败 ({}): {}",
                instance.data_dir.display(),
                error
            )),
        }

        for db_path in sqlite_candidate_paths(&instance.data_dir) {
            match sqlite_provider_ids(&db_path) {
                Ok(provider_ids) => {
                    for provider_id in provider_ids {
                        add_provider_source(
                            &mut sources,
                            provider_id,
                            CodexSessionVisibilityRepairProviderSource::Sqlite,
                        );
                    }
                }
                Err(error) => modules::logger::log_warn(&format!(
                    "读取 Codex SQLite provider 候选失败 ({}): {}",
                    db_path.display(),
                    error
                )),
            }
        }
    }

    if sources.is_empty() {
        add_provider_source(
            &mut sources,
            default_provider.clone(),
            CodexSessionVisibilityRepairProviderSource::Config,
        );
    }

    let mut providers = sources
        .into_iter()
        .map(|(id, source_set)| {
            let mut sources = source_set.into_iter().collect::<Vec<_>>();
            sources.sort();
            CodexSessionVisibilityRepairProviderOption {
                is_default: id == default_provider,
                id,
                sources,
            }
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(CodexSessionVisibilityRepairProviderList {
        default_provider,
        providers,
    })
}

pub fn list_session_visibility_repair_instances(
) -> Result<CodexSessionVisibilityRepairInstanceList, String> {
    let instances = collect_instances()?;
    let process_entries = modules::process::collect_codex_process_entries();
    let options = instances
        .into_iter()
        .map(|instance| {
            let current_provider = read_target_provider(&instance.data_dir)?;
            let running = is_instance_running(&instance, &process_entries);
            Ok(CodexSessionVisibilityRepairInstanceOption {
                is_default: instance.id == DEFAULT_INSTANCE_ID,
                id: instance.id,
                name: instance.name,
                user_data_dir: instance.data_dir.to_string_lossy().to_string(),
                current_provider,
                running,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(CodexSessionVisibilityRepairInstanceList {
        default_instance_id: DEFAULT_INSTANCE_ID.to_string(),
        instances: options,
    })
}

pub fn resolve_session_visibility_target_provider_from_instance_id(
    instance_id: &str,
) -> Result<String, String> {
    let instance_id = instance_id.trim();
    if instance_id.is_empty() {
        return Err("目标实例不能为空".to_string());
    }

    let instance = collect_instances()?
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("目标实例不存在: {}", instance_id))?;
    read_target_provider(&instance.data_dir)
}

fn report_repair_progress(
    reporter: ProgressReporter<'_>,
    run_id: &Option<String>,
    options: CodexSessionVisibilityRepairOptions,
    stage: &str,
    percent: u8,
    current: usize,
    total: usize,
    instance: Option<&CodexSyncInstance>,
) {
    let Some(reporter) = reporter else {
        return;
    };
    reporter(CodexSessionVisibilityRepairProgress {
        run_id: run_id.clone(),
        mode: options.mode,
        stage: stage.to_string(),
        percent: percent.min(100),
        current,
        total,
        instance_id: instance.map(|item| item.id.clone()),
        instance_name: instance.map(|item| item.name.clone()),
    });
}

fn instance_progress_percent(
    instance_index: usize,
    total_instances: usize,
    phase_index: usize,
    phase_count: usize,
) -> u8 {
    let total_instances = total_instances.max(1) as f64;
    let phase_count = phase_count.max(1) as f64;
    let slot = 86.0 / total_instances;
    let value = 8.0 + slot * instance_index as f64 + slot * (phase_index as f64 / phase_count);
    value.round().clamp(8.0, 94.0) as u8
}

fn instance_progress_percent_between(
    instance_index: usize,
    total_instances: usize,
    phase_start: usize,
    phase_end: usize,
    phase_count: usize,
    current: usize,
    total: usize,
) -> u8 {
    let total_instances = total_instances.max(1) as f64;
    let phase_count = phase_count.max(1) as f64;
    let slot = 86.0 / total_instances;
    let progress = if total == 0 {
        0.0
    } else {
        current.min(total) as f64 / total as f64
    };
    let phase = phase_start as f64 + (phase_end.saturating_sub(phase_start) as f64 * progress);
    let value = 8.0 + slot * instance_index as f64 + slot * (phase / phase_count);
    value.round().clamp(8.0, 94.0) as u8
}

pub fn read_history_visibility_provider_for_dir(data_dir: &Path) -> Result<String, String> {
    read_target_provider(data_dir)
}

fn repair_single_instance(
    data_dir: &Path,
    target_provider: &str,
    rollout_changes: &[RolloutProviderChange],
    update_sqlite: bool,
    update_sqlite_timestamps: bool,
    reconcile_session_index: bool,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<RepairSingleInstanceResult, String> {
    let placeholder_instance = CodexSyncInstance {
        id: String::new(),
        name: String::new(),
        data_dir: data_dir.to_path_buf(),
        last_pid: None,
    };
    repair_single_instance_with_progress(
        data_dir,
        target_provider,
        rollout_changes,
        update_sqlite,
        update_sqlite_timestamps,
        reconcile_session_index,
        options,
        selection,
        None,
        &None,
        &placeholder_instance,
        0,
        1,
    )
}

fn repair_single_instance_with_progress(
    data_dir: &Path,
    target_provider: &str,
    rollout_changes: &[RolloutProviderChange],
    update_sqlite: bool,
    update_sqlite_timestamps: bool,
    reconcile_session_index: bool,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
    progress_reporter: ProgressReporter<'_>,
    run_id: &Option<String>,
    instance: &CodexSyncInstance,
    instance_index: usize,
    total_instances: usize,
) -> Result<RepairSingleInstanceResult, String> {
    let sqlite_rows_updated = if update_sqlite {
        report_repair_progress(
            progress_reporter,
            run_id,
            options,
            "write_sqlite_provider",
            instance_progress_percent(instance_index, total_instances, 3, 8),
            0,
            0,
            Some(instance),
        );
        update_sqlite_provider_for_options(data_dir, target_provider, options, selection)?
    } else {
        0
    };
    let rollout_total = rollout_changes.len();
    for (rollout_index, change) in rollout_changes.iter().enumerate() {
        report_repair_progress(
            progress_reporter,
            run_id,
            options,
            "write_rollout_files",
            instance_progress_percent_between(
                instance_index,
                total_instances,
                4,
                5,
                8,
                rollout_index + 1,
                rollout_total,
            ),
            rollout_index + 1,
            rollout_total,
            Some(instance),
        );
        rewrite_rollout_provider(change)?;
    }
    let sqlite_timestamp_rows_updated = if update_sqlite_timestamps {
        report_repair_progress(
            progress_reporter,
            run_id,
            options,
            "write_sqlite_timestamps",
            instance_progress_percent(instance_index, total_instances, 6, 8),
            0,
            0,
            Some(instance),
        );
        repair_sqlite_thread_timestamps_for_options(data_dir, options, selection)?
    } else {
        0
    };
    let session_index_result = if reconcile_session_index {
        report_repair_progress(
            progress_reporter,
            run_id,
            options,
            "write_session_index",
            instance_progress_percent(instance_index, total_instances, 7, 8),
            0,
            0,
            Some(instance),
        );
        reconcile_session_index_from_sqlite_for_options(data_dir, options, selection)?
    } else {
        SessionIndexReconcileResult::default()
    };
    Ok(RepairSingleInstanceResult {
        updated_sqlite_rows: sqlite_rows_updated,
        updated_sqlite_timestamp_rows: sqlite_timestamp_rows_updated,
        added_session_index_entries: session_index_result.added_entries,
        updated_session_index_entries: session_index_result.updated_entries,
    })
}

fn build_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    updated_sqlite_timestamp_row_count: usize,
    added_session_index_entry_count: usize,
    updated_session_index_entry_count: usize,
    mutated_running_instance_count: usize,
    _skipped_sqlite_file_count: usize,
    metadata_rebuild_failed_count: usize,
) -> String {
    if mutated_instance_count == 0 {
        if metadata_rebuild_failed_count > 0 {
            return format!(
                "所有 Codex 实例的会话文件与 SQLite 可见性记录均一致；{} 个实例的官方侧边栏状态刷新未完成，重启 Codex 后会重新加载",
                metadata_rebuild_failed_count
            );
        }
        return "所有 Codex 实例的会话文件与 SQLite 可见性记录均一致".to_string();
    }

    let added_index_suffix = if added_session_index_entry_count > 0 {
        format!(
            "，补写 {} 条 session_index 记录",
            added_session_index_entry_count
        )
    } else {
        String::new()
    };
    let updated_index_suffix = if updated_session_index_entry_count > 0 {
        format!(
            "，刷新 {} 条 session_index 记录",
            updated_session_index_entry_count
        )
    } else {
        String::new()
    };
    let running_suffix = if mutated_running_instance_count > 0 {
        "。运行中的实例可能需要重启后完全刷新"
    } else {
        ""
    };
    let metadata_suffix = if metadata_rebuild_failed_count > 0 {
        format!(
            "；{} 个实例的官方侧边栏索引重建未完成，重启 Codex 后会重新加载",
            metadata_rebuild_failed_count
        )
    } else {
        String::new()
    };

    format!(
        "已为 {} 个实例修复会话可见性：校正 {} 个会话文件，更新 {} 条 SQLite 可见性记录，校正 {} 条 SQLite 时间记录{}{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_index_suffix,
        updated_index_suffix,
        running_suffix,
        metadata_suffix
    )
}

fn build_dry_run_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    updated_sqlite_timestamp_row_count: usize,
    added_session_index_entry_count: usize,
    updated_session_index_entry_count: usize,
    mutated_running_instance_count: usize,
) -> String {
    if mutated_instance_count == 0 {
        return "预览未发现需要写入的会话可见性差异".to_string();
    }

    let added_index_suffix = if added_session_index_entry_count > 0 {
        format!(
            "，补写 {} 条 session_index 记录",
            added_session_index_entry_count
        )
    } else {
        String::new()
    };
    let updated_index_suffix = if updated_session_index_entry_count > 0 {
        format!(
            "，刷新 {} 条 session_index 记录",
            updated_session_index_entry_count
        )
    } else {
        String::new()
    };
    let running_suffix = if mutated_running_instance_count > 0 {
        "。包含运行中的实例，确认修复后可能需要重启 Codex 才能完全刷新"
    } else {
        ""
    };

    format!(
        "预览将为 {} 个实例修复会话可见性：校正 {} 个会话文件，更新 {} 条 SQLite 可见性记录，校正 {} 条 SQLite 时间记录{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_index_suffix,
        updated_index_suffix,
        running_suffix
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

fn try_rebuild_thread_metadata(instance: &CodexSyncInstance) -> bool {
    let started = std::time::Instant::now();
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] rebuild official metadata started: instance_id={}, instance_name={}, data_dir={}",
        instance.id,
        instance.name,
        instance.data_dir.display()
    ));
    match modules::codex_official_app_server::rebuild_thread_metadata(&instance.data_dir) {
        Ok(()) => {
            modules::logger::log_info(&format!(
                "[Codex Session Visibility] rebuild official metadata finished: instance_id={}, elapsed_ms={}",
                instance.id,
                started.elapsed().as_millis()
            ));
            true
        }
        Err(error) => {
            modules::logger::log_warn(&format!(
                "Codex 会话索引修复后触发官方侧边栏索引重建失败 ({} / {}): {}; elapsed_ms={}",
                instance.name,
                instance.data_dir.display(),
                error,
                started.elapsed().as_millis()
            ));
            false
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

    let doc = modules::codex_config_format::read_codex_config_doc_from_str(&content).map_err(
        |error| {
            format!(
                "解析 config.toml 失败 ({}): {}",
                config_path.display(),
                error
            )
        },
    )?;
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn validate_provider_id(provider_id: &str) -> Result<(), String> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err("provider 不能为空".to_string());
    }
    if trimmed.len() > 200 || trimmed.chars().any(char::is_control) {
        return Err("provider 包含非法字符".to_string());
    }
    Ok(())
}

fn is_valid_provider_id_for_discovery(provider_id: &str) -> bool {
    validate_provider_id(provider_id).is_ok()
}

fn add_provider_source(
    sources: &mut HashMap<String, HashSet<CodexSessionVisibilityRepairProviderSource>>,
    provider_id: String,
    source: CodexSessionVisibilityRepairProviderSource,
) {
    let provider_id = provider_id.trim().to_string();
    if !is_valid_provider_id_for_discovery(&provider_id) {
        return;
    }
    sources.entry(provider_id).or_default().insert(source);
}

fn list_configured_provider_ids(data_dir: &Path) -> Result<Vec<String>, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(vec![DEFAULT_PROVIDER_ID.to_string()]);
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(vec![DEFAULT_PROVIDER_ID.to_string()]);
    }

    let doc = modules::codex_config_format::read_codex_config_doc_from_str(&content).map_err(
        |error| {
            format!(
                "解析 config.toml 失败 ({}): {}",
                config_path.display(),
                error
            )
        },
    )?;
    let mut ids = HashSet::new();
    if let Some(provider) = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        ids.insert(provider.to_string());
    }
    if let Some(model_providers) = doc.get("model_providers").and_then(|item| item.as_table()) {
        for (provider_id, _) in model_providers.iter() {
            let provider_id = provider_id.trim();
            if !provider_id.is_empty() {
                ids.insert(provider_id.to_string());
            }
        }
    }
    if ids.is_empty() {
        ids.insert(DEFAULT_PROVIDER_ID.to_string());
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn collect_rollout_provider_ids(data_dir: &Path) -> Result<Vec<String>, String> {
    let mut ids = HashSet::new();
    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let content = fs::read_to_string(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            for line in content.lines() {
                let Ok(record) = serde_json::from_str::<JsonValue>(line.trim()) else {
                    continue;
                };
                if record.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
                    continue;
                }
                let Some(provider_id) = record
                    .get("payload")
                    .and_then(JsonValue::as_object)
                    .and_then(|payload| payload.get("model_provider"))
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                if is_valid_provider_id_for_discovery(provider_id) {
                    ids.insert(provider_id.to_string());
                }
            }
        }
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn sqlite_provider_ids(db_path: &Path) -> Result<Vec<String>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let Some(columns) = read_threads_table_columns(&connection).map_err(|error| {
        format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
    })?
    else {
        return Ok(Vec::new());
    };
    if !columns.model_provider {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT model_provider FROM threads WHERE COALESCE(model_provider, '') <> ''",
        )
        .map_err(|error| {
            format_sqlite_read_error(db_path, "准备 SQLite provider 查询失败", &error)
        })?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format_sqlite_read_error(db_path, "查询 SQLite provider 失败", &error))?;
    let mut ids = HashSet::new();
    for row in rows {
        let provider_id = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite provider 失败", &error)
        })?;
        if is_valid_provider_id_for_discovery(&provider_id) {
            ids.insert(provider_id);
        }
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn collect_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
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
            let rewrite = if options.rewrite_all_session_meta {
                let content = fs::read_to_string(&rollout_path).map_err(|error| {
                    format!(
                        "读取 rollout 文件失败 ({}): {}",
                        rollout_path.display(),
                        error
                    )
                })?;
                rewrite_rollout_session_meta_providers(&content, target_provider)?
            } else {
                rewrite_rollout_first_session_meta_provider(&rollout_path, target_provider)?
            };
            if rewrite.session_meta_count == 0 {
                continue;
            }
            let session_id = rewrite.thread_id.clone();
            if let Some(session_id) = session_id.as_deref() {
                if !selection.includes_session_id(session_id) {
                    continue;
                }
            } else if selection.has_session_filter() {
                continue;
            }
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
            let provider_matches = !rewrite.rewrite_needed;
            let modified_time_matches = target_modified_at.is_none()
                || modules::codex_session_file_time::same_modified_time_millis(
                    current_modified_at,
                    target_modified_at,
                );
            if provider_matches && modified_time_matches {
                continue;
            }

            let relative_path = rollout_path
                .strip_prefix(data_dir)
                .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?;
            changes.push(RolloutProviderChange {
                relative_path: relative_path.to_path_buf(),
                absolute_path: rollout_path,
                updated_content: rewrite.updated_content,
                target_modified_at,
            });
        }
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn collect_referenced_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<Vec<RolloutProviderChange>, String> {
    let mut candidates: HashMap<PathBuf, Option<SystemTime>> = HashMap::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        collect_referenced_rollout_paths_for_db(data_dir, &db_path, selection, &mut candidates)?;
    }

    let mut changes = Vec::new();
    for (rollout_path, target_modified_at) in candidates {
        if !rollout_path.exists() {
            continue;
        }
        let rewrite = if options.rewrite_all_session_meta {
            let content = fs::read_to_string(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            rewrite_rollout_session_meta_providers(&content, target_provider)?
        } else {
            rewrite_rollout_first_session_meta_provider(&rollout_path, target_provider)?
        };
        if rewrite.session_meta_count == 0 || !rewrite.rewrite_needed {
            continue;
        }
        let Some(relative_path) = rollout_path
            .strip_prefix(data_dir)
            .ok()
            .map(Path::to_path_buf)
        else {
            modules::logger::log_warn(&format!(
                "跳过 Codex 会话可见性修复中的实例外 rollout: data_dir={}, rollout={}",
                data_dir.display(),
                rollout_path.display()
            ));
            continue;
        };
        changes.push(RolloutProviderChange {
            relative_path,
            absolute_path: rollout_path,
            updated_content: rewrite.updated_content,
            target_modified_at,
        });
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn collect_referenced_rollout_paths_for_db(
    data_dir: &Path,
    db_path: &Path,
    selection: &RepairTargetSelection,
    candidates: &mut HashMap<PathBuf, Option<SystemTime>>,
) -> Result<(), String> {
    if !db_path.exists() {
        return Ok(());
    }
    let connection = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut table_info = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let names = table_info
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    drop(table_info);

    if !names.contains("id") || !names.contains("rollout_path") {
        return Ok(());
    }
    let updated_at_expr = if names.contains("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let updated_at_ms_expr = if names.contains("updated_at_ms") {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, rollout_path, {updated_at_expr}, {updated_at_ms_expr} FROM threads WHERE rollout_path IS NOT NULL AND rollout_path <> ''"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format_sqlite_read_error(db_path, "准备 SQLite rollout 引用查询失败", &error)
    })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| {
            format_sqlite_read_error(db_path, "查询 SQLite rollout 引用失败", &error)
        })?;

    for row in rows {
        let (thread_id, rollout_path, updated_at, updated_at_ms) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite rollout 引用失败", &error)
        })?;
        if !selection.includes_session_id(&thread_id) {
            continue;
        }
        let rollout_path = resolve_rollout_path(data_dir, &rollout_path);
        let target_modified_at = updated_at_ms
            .or_else(|| updated_at.map(|value| value * 1000))
            .and_then(|value| {
                modules::codex_session_file_time::system_time_from_unix_millis(value as i128)
            });
        candidates
            .entry(rollout_path)
            .and_modify(|existing| {
                if existing.is_none() {
                    *existing = target_modified_at;
                }
            })
            .or_insert(target_modified_at);
    }
    Ok(())
}

#[derive(Debug, Default)]
struct RolloutProviderRewrite {
    updated_content: Option<RolloutProviderUpdate>,
    rewrite_needed: bool,
    thread_id: Option<String>,
    session_meta_count: usize,
}

fn rewrite_rollout_session_meta_providers(
    content: &str,
    target_provider: &str,
) -> Result<RolloutProviderRewrite, String> {
    let mut rewrite = RolloutProviderRewrite::default();
    let mut next_content = String::new();
    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        let mut next_line = line.to_string();
        if !line.trim().is_empty() {
            if let Ok(mut record) = serde_json::from_str::<JsonValue>(line) {
                if record.get("type").and_then(JsonValue::as_str) == Some("session_meta") {
                    let Some(payload) =
                        record.get_mut("payload").and_then(JsonValue::as_object_mut)
                    else {
                        next_content.push_str(&next_line);
                        next_content.push_str(line_ending);
                        continue;
                    };
                    rewrite.session_meta_count += 1;
                    if rewrite.thread_id.is_none() {
                        rewrite.thread_id = payload
                            .get("id")
                            .or_else(|| payload.get("session_id"))
                            .and_then(JsonValue::as_str)
                            .map(str::to_string);
                    }
                    if payload.get("model_provider").and_then(JsonValue::as_str)
                        != Some(target_provider)
                    {
                        payload.insert(
                            "model_provider".to_string(),
                            JsonValue::String(target_provider.to_string()),
                        );
                        next_line = serde_json::to_string(&record)
                            .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
                        rewrite.rewrite_needed = true;
                    }
                }
            }
        }
        next_content.push_str(&next_line);
        next_content.push_str(line_ending);
    }
    if !content.ends_with('\n') && next_content.ends_with('\n') {
        next_content.pop();
    }
    if rewrite.rewrite_needed {
        rewrite.updated_content = Some(RolloutProviderUpdate::FullContent(next_content));
    }
    Ok(rewrite)
}

fn rewrite_rollout_first_session_meta_provider(
    path: &Path,
    target_provider: &str,
) -> Result<RolloutProviderRewrite, String> {
    let Some((first_line, _separator)) = read_first_line(path)? else {
        return Ok(RolloutProviderRewrite::default());
    };
    let Some(mut record) = parse_session_meta_record(&first_line) else {
        return Ok(RolloutProviderRewrite::default());
    };
    let thread_id = session_meta_id(&record);
    let current_provider = record
        .get("payload")
        .and_then(|payload| payload.get("model_provider"))
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    if current_provider == target_provider {
        return Ok(RolloutProviderRewrite {
            updated_content: None,
            rewrite_needed: false,
            thread_id,
            session_meta_count: 1,
        });
    }

    let Some(payload) = record.get_mut("payload").and_then(JsonValue::as_object_mut) else {
        return Ok(RolloutProviderRewrite::default());
    };
    payload.insert(
        "model_provider".to_string(),
        JsonValue::String(target_provider.to_string()),
    );
    let updated_first_line = serde_json::to_string(&record)
        .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
    Ok(RolloutProviderRewrite {
        updated_content: Some(RolloutProviderUpdate::FirstLine(updated_first_line)),
        rewrite_needed: true,
        thread_id,
        session_meta_count: 1,
    })
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

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

fn collect_rollout_thread_facts(
    data_dir: &Path,
    selection: &RepairTargetSelection,
) -> Result<RolloutThreadFacts, String> {
    let mut facts = RolloutThreadFacts::default();
    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let content = fs::read_to_string(&rollout_path).map_err(|error| {
                format!(
                    "读取 rollout 文件失败 ({}): {}",
                    rollout_path.display(),
                    error
                )
            })?;
            let has_user_event =
                content.contains("\"user_message\"") || content.contains("\"user_input\"");
            for line in content.lines() {
                let Ok(record) = serde_json::from_str::<JsonValue>(line.trim()) else {
                    continue;
                };
                if record.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
                    continue;
                }
                let Some(payload) = record.get("payload").and_then(JsonValue::as_object) else {
                    continue;
                };
                let Some(thread_id) = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                if !selection.includes_session_id(&thread_id) {
                    continue;
                }
                if has_user_event {
                    facts.user_event_thread_ids.insert(thread_id.clone());
                }
                if let Some(cwd) = payload
                    .get("cwd")
                    .and_then(JsonValue::as_str)
                    .and_then(to_desktop_workspace_path)
                {
                    facts.cwd_by_thread_id.entry(thread_id).or_insert(cwd);
                }
            }
        }
    }
    Ok(facts)
}

fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let stripped = value.trim();
    if stripped.is_empty() {
        return None;
    }
    let lower = stripped.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") {
        return Some(format!(r"\\{}", stripped[8..].replace('/', r"\")));
    }
    if stripped.starts_with(r"\\?\") {
        return Some(stripped[4..].replace('\\', "/"));
    }
    Some(stripped.to_string())
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

fn count_session_index_entries_to_repair(
    data_dir: &Path,
) -> Result<SessionIndexRepairScan, String> {
    count_session_index_entries_to_repair_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_session_index_entries_to_repair_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SessionIndexRepairScan, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_for_options(data_dir, options, selection)?;
    let mut scan = SessionIndexRepairScan::default();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(entry)
                if options.update_existing_session_index_entries
                    && session_index_entry_needs_update(data_dir, row, entry) =>
            {
                scan.entries_to_update += 1;
            }
            Some(_) => {}
            None => {
                scan.entries_to_add += 1;
            }
        }
    }
    Ok(scan)
}

fn count_missing_session_index_entries(data_dir: &Path) -> Result<usize, String> {
    Ok(count_session_index_entries_to_repair(data_dir)?.entries_to_add)
}

fn load_sqlite_thread_index_rows(data_dir: &Path) -> Result<Vec<SqliteThreadIndexRow>, String> {
    load_sqlite_thread_index_rows_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn load_sqlite_thread_index_rows_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        for row in load_sqlite_thread_index_rows_from_db(&db_path)? {
            if !selection.includes_session_id(&row.id) {
                continue;
            }
            if seen_ids.insert(row.id.clone()) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn load_sqlite_thread_index_rows_from_db(
    db_path: &Path,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => return Ok(Vec::new()),
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    if !names.contains("id") {
        return Ok(Vec::new());
    }

    let title_expr = if names.contains("title") {
        "COALESCE(title, '')"
    } else {
        "''"
    };
    let updated_at_expr = if names.contains("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let updated_at_ms_expr = if names.contains("updated_at_ms") {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let rollout_path_expr = if names.contains("rollout_path") {
        "rollout_path"
    } else {
        "NULL"
    };
    let order_expr = if names.contains("updated_at") {
        "updated_at DESC"
    } else {
        "id ASC"
    };
    let sql = format!(
        "SELECT id, {title_expr}, {updated_at_expr}, {updated_at_ms_expr}, {rollout_path_expr} FROM threads ORDER BY {order_expr}"
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
                updated_at_ms: row.get(3)?,
                rollout_path: row.get(4)?,
            })
        })
        .map_err(|error| {
            format!(
                "查询 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let mut result = Vec::new();
    for row in mapped {
        result.push(row.map_err(|error| {
            format!(
                "读取 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?);
    }
    Ok(result)
}

fn format_thread_updated_at_iso_ms(updated_at_ms: Option<i128>) -> String {
    let milliseconds = updated_at_ms.unwrap_or_else(|| Utc::now().timestamp_millis() as i128);
    i64::try_from(milliseconds)
        .ok()
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn resolve_thread_updated_at_ms(data_dir: &Path, row: &SqliteThreadIndexRow) -> Option<i128> {
    let rollout_activity_ms = row
        .rollout_path
        .as_deref()
        .map(|path| resolve_rollout_path(data_dir, path))
        .filter(|path| path.exists())
        .and_then(|path| rollout_file_activity_ms(&path));
    let sqlite_ms = row
        .updated_at_ms
        .map(|value| value as i128)
        .or_else(|| row.updated_at.map(|value| value as i128 * 1000));
    match (sqlite_ms, rollout_activity_ms) {
        (Some(sqlite_ms), Some(activity_ms))
            if (sqlite_ms - activity_ms).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity_ms)
        }
        (Some(sqlite_ms), _) => Some(sqlite_ms),
        (None, Some(activity_ms)) => Some(activity_ms),
        (None, None) => None,
    }
}

fn build_session_index_entry_from_thread(data_dir: &Path, row: &SqliteThreadIndexRow) -> JsonValue {
    json!({
        "id": row.id,
        "thread_name": if row.title.trim().is_empty() {
            "Untitled"
        } else {
            row.title.as_str()
        },
        "updated_at": format_thread_updated_at_iso_ms(resolve_thread_updated_at_ms(data_dir, row)),
    })
}

fn build_updated_session_index_entry(
    data_dir: &Path,
    existing: &JsonValue,
    row: &SqliteThreadIndexRow,
) -> JsonValue {
    let mut entry = existing.clone();
    let Some(object) = entry.as_object_mut() else {
        return build_session_index_entry_from_thread(data_dir, row);
    };
    object.insert("id".to_string(), JsonValue::String(row.id.clone()));
    if !row.title.trim().is_empty() {
        object.insert(
            "thread_name".to_string(),
            JsonValue::String(row.title.clone()),
        );
    }
    object.insert(
        "updated_at".to_string(),
        JsonValue::String(format_thread_updated_at_iso_ms(
            resolve_thread_updated_at_ms(data_dir, row),
        )),
    );
    entry
}

fn session_index_entry_needs_update(
    data_dir: &Path,
    row: &SqliteThreadIndexRow,
    entry: &JsonValue,
) -> bool {
    let Some(target_ms) = resolve_thread_updated_at_ms(data_dir, row) else {
        return false;
    };
    match parse_session_index_updated_at_ms(entry) {
        Some(current_ms) => (current_ms - target_ms).abs() > 1000,
        None => true,
    }
}

fn reconcile_session_index_from_sqlite(
    data_dir: &Path,
) -> Result<SessionIndexReconcileResult, String> {
    reconcile_session_index_from_sqlite_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn reconcile_session_index_from_sqlite_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SessionIndexReconcileResult, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_for_options(data_dir, options, selection)?;
    let mut entries_to_add = Vec::<JsonValue>::new();
    let mut entries_to_update = HashMap::<String, JsonValue>::new();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(existing)
                if options.update_existing_session_index_entries
                    && session_index_entry_needs_update(data_dir, row, existing) =>
            {
                entries_to_update.insert(
                    row.id.clone(),
                    build_updated_session_index_entry(data_dir, existing, row),
                );
            }
            Some(_) => {}
            None => entries_to_add.push(build_session_index_entry_from_thread(data_dir, row)),
        }
    }
    if entries_to_add.is_empty() && entries_to_update.is_empty() {
        return Ok(SessionIndexReconcileResult::default());
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

    let mut updated_ids = HashSet::<String>::new();
    for line in &mut lines {
        let Ok(entry) = serde_json::from_str::<JsonValue>(line.trim()) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        let Some(updated_entry) = entries_to_update.get(id) else {
            continue;
        };
        *line = serde_json::to_string(updated_entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        updated_ids.insert(id.to_string());
    }

    for entry in &entries_to_add {
        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        lines.push(line);
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
    Ok(SessionIndexReconcileResult {
        added_entries: entries_to_add.len(),
        updated_entries: updated_ids.len(),
    })
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

fn count_sqlite_thread_timestamps_to_update(data_dir: &Path) -> Result<usize, String> {
    count_sqlite_thread_timestamps_to_update_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_sqlite_thread_timestamps_to_update_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total += plan_sqlite_thread_timestamp_repair_for_db(data_dir, &db_path, selection)?
            .updates
            .len();
    }
    Ok(total)
}

fn plan_sqlite_thread_timestamp_repair_for_db(
    data_dir: &Path,
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<SqliteTimestampRepairPlan, String> {
    if !db_path.exists() {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteTimestampRepairPlan::default());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => {
            return Ok(SqliteTimestampRepairPlan::default())
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    drop(statement);

    let has_updated_at = names.contains("updated_at");
    let has_updated_at_ms = names.contains("updated_at_ms");
    if !names.contains("id")
        || !names.contains("rollout_path")
        || (!has_updated_at && !has_updated_at_ms)
    {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let updated_at_expr = if has_updated_at { "updated_at" } else { "NULL" };
    let updated_at_ms_expr = if has_updated_at_ms {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, rollout_path, {updated_at_expr}, {updated_at_ms_expr} FROM threads WHERE rollout_path IS NOT NULL AND rollout_path <> ''"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format_sqlite_read_error(db_path, "准备 SQLite 会话时间修复查询失败", &error)
    })?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| format_sqlite_read_error(db_path, "查询 SQLite 会话时间失败", &error))?;

    let mut updates = Vec::new();
    for row in rows {
        let (thread_id, rollout_path, updated_at, updated_at_ms) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite 会话时间失败", &error)
        })?;
        if !selection.includes_session_id(&thread_id) {
            continue;
        }
        let rollout = resolve_rollout_path(data_dir, &rollout_path);
        if !rollout.exists() {
            continue;
        }
        let Some(activity_ms) = rollout_file_activity_ms(&rollout) else {
            continue;
        };
        let activity_seconds = (activity_ms / 1000) as i64;
        let activity_ms = activity_seconds * 1000;
        let current_ms = updated_at_ms
            .or_else(|| updated_at.map(|value| value * 1000))
            .unwrap_or(0);
        if i64::abs(current_ms - activity_ms) <= 1000 {
            continue;
        }
        updates.push(SqliteTimestampUpdate {
            id: thread_id,
            updated_at_seconds: activity_seconds,
            updated_at_ms: activity_ms,
        });
    }
    Ok(SqliteTimestampRepairPlan {
        updates,
        has_updated_at,
        has_updated_at_ms,
    })
}

fn repair_sqlite_thread_timestamps(data_dir: &Path) -> Result<usize, String> {
    repair_sqlite_thread_timestamps_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn repair_sqlite_thread_timestamps_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total += repair_sqlite_thread_timestamps_for_db(data_dir, &db_path, selection)?;
    }
    Ok(total)
}

fn repair_sqlite_thread_timestamps_for_db(
    data_dir: &Path,
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let plan = plan_sqlite_thread_timestamp_repair_for_db(data_dir, db_path, selection)?;
    let updates = plan.updates;

    if updates.is_empty() {
        return Ok(0);
    }

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
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    for update in &updates {
        if plan.has_updated_at && plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    (
                        update.updated_at_seconds,
                        update.updated_at_ms,
                        update.id.as_str(),
                    ),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
                    (update.updated_at_seconds, update.id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at_ms = ?1 WHERE id = ?2",
                    (update.updated_at_ms, update.id.as_str()),
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
        "跳过无效或损坏的 Codex SQLite 会话库 ({}): {}",
        path.display(),
        reason
    ));
}

fn count_sqlite_rows_to_update(
    data_dir: &Path,
    target_provider: &str,
) -> Result<SqliteProviderScan, String> {
    count_sqlite_rows_to_update_for_options(
        data_dir,
        target_provider,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_sqlite_rows_to_update_for_options(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SqliteProviderScan, String> {
    let facts = if options.collect_rollout_thread_facts {
        Some(collect_rollout_thread_facts(data_dir, selection)?)
    } else {
        None
    };
    let mut scan = SqliteProviderScan {
        rows_to_update: 0,
        skipped_unusable_database: false,
    };
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        let item = count_sqlite_rows_to_update_for_db(
            &db_path,
            target_provider,
            facts.as_ref(),
            selection,
        )?;
        scan.rows_to_update += item.rows_to_update;
        scan.skipped_unusable_database |= item.skipped_unusable_database;
    }
    Ok(scan)
}

fn count_sqlite_rows_to_update_for_db(
    db_path: &Path,
    target_provider: &str,
    facts: Option<&RolloutThreadFacts>,
    selection: &RepairTargetSelection,
) -> Result<SqliteProviderScan, String> {
    if !db_path.exists() {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
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
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let Some(columns) = columns else {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    };
    let mut count = 0i64;
    if let Some(where_clause) = build_threads_repair_where_clause(columns) {
        if let Some(session_ids) = selection.session_ids() {
            if columns.model_provider {
                let sql =
                    format!("SELECT COUNT(*) FROM threads WHERE ({where_clause}) AND id = ?2");
                for thread_id in session_ids {
                    count += connection
                        .query_row(sql.as_str(), (target_provider, thread_id.as_str()), |row| {
                            row.get::<usize, i64>(0)
                        })
                        .map_err(|error| {
                            format!(
                                "统计 SQLite 会话可见性差异失败 ({}): {}",
                                db_path.display(),
                                error
                            )
                        })?;
                }
            } else {
                let sql =
                    format!("SELECT COUNT(*) FROM threads WHERE ({where_clause}) AND id = ?1");
                for thread_id in session_ids {
                    count += connection
                        .query_row(sql.as_str(), [thread_id.as_str()], |row| {
                            row.get::<usize, i64>(0)
                        })
                        .map_err(|error| {
                            format!(
                                "统计 SQLite 会话可见性差异失败 ({}): {}",
                                db_path.display(),
                                error
                            )
                        })?;
                }
            }
        } else {
            let sql = format!("SELECT COUNT(*) FROM threads WHERE {where_clause}");
            let count_result = if columns.model_provider {
                connection.query_row(sql.as_str(), [target_provider], |row| {
                    row.get::<usize, i64>(0)
                })
            } else {
                connection.query_row(sql.as_str(), [], |row| row.get::<usize, i64>(0))
            };
            count += match count_result {
                Ok(count) => count,
                Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                    log_skipped_sqlite_database(db_path, &error.to_string());
                    return Ok(SqliteProviderScan {
                        rows_to_update: 0,
                        skipped_unusable_database: true,
                    });
                }
                Err(error) if is_missing_threads_table_error(&error) => {
                    return Ok(SqliteProviderScan {
                        rows_to_update: 0,
                        skipped_unusable_database: false,
                    });
                }
                Err(error) => {
                    return Err(format!(
                        "统计 SQLite 会话可见性差异失败 ({}): {}",
                        db_path.display(),
                        error
                    ));
                }
            };
        }
    }
    if let Some(facts) = facts {
        if columns.has_user_event {
            for thread_id in &facts.user_event_thread_ids {
                count += connection
                    .query_row(
                        "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                        [thread_id.as_str()],
                        |row| row.get::<usize, i64>(0),
                    )
                    .map_err(|error| {
                        format!(
                            "统计 SQLite has_user_event 差异失败 ({}): {}",
                            db_path.display(),
                            error
                        )
                    })?;
            }
        }
        if columns.cwd {
            for (thread_id, cwd) in &facts.cwd_by_thread_id {
                count += connection
                    .query_row(
                        "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(cwd, '') <> ?2",
                        (thread_id.as_str(), cwd.as_str()),
                        |row| row.get::<usize, i64>(0),
                    )
                    .map_err(|error| {
                        format!(
                            "统计 SQLite cwd 差异失败 ({}): {}",
                            db_path.display(),
                            error
                        )
                    })?;
            }
        }
    }
    Ok(SqliteProviderScan {
        rows_to_update: count.max(0) as usize,
        skipped_unusable_database: false,
    })
}

fn update_sqlite_provider(data_dir: &Path, target_provider: &str) -> Result<usize, String> {
    update_sqlite_provider_for_options(
        data_dir,
        target_provider,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn update_sqlite_provider_for_options(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let facts = if options.collect_rollout_thread_facts {
        Some(collect_rollout_thread_facts(data_dir, selection)?)
    } else {
        None
    };
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total +=
            update_sqlite_provider_for_db(&db_path, target_provider, facts.as_ref(), selection)?;
    }
    Ok(total)
}

fn update_sqlite_provider_for_db(
    db_path: &Path,
    target_provider: &str,
    facts: Option<&RolloutThreadFacts>,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

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
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
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
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let mut updated_rows = 0usize;
    if let Some(where_clause) = build_threads_repair_where_clause(columns) {
        let set_clause = build_threads_repair_set_clause(columns);
        if let Some(session_ids) = selection.session_ids() {
            if columns.model_provider {
                let sql =
                    format!("UPDATE threads SET {set_clause} WHERE ({where_clause}) AND id = ?2");
                for thread_id in session_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), (target_provider, thread_id.as_str()))
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            } else {
                let sql =
                    format!("UPDATE threads SET {set_clause} WHERE ({where_clause}) AND id = ?1");
                for thread_id in session_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), [thread_id.as_str()])
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            }
        } else {
            let sql = format!("UPDATE threads SET {set_clause} WHERE {where_clause}");
            let update_result = if columns.model_provider {
                transaction.execute(sql.as_str(), [target_provider])
            } else {
                transaction.execute(sql.as_str(), [])
            };
            updated_rows += match update_result {
                Ok(updated_rows) => updated_rows,
                Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                    log_skipped_sqlite_database(db_path, &error.to_string());
                    return Ok(0);
                }
                Err(error) if is_missing_threads_table_error(&error) => {
                    return Ok(0);
                }
                Err(error) => return Err(format_sqlite_write_error(db_path, &error)),
            };
        }
    }
    if let Some(facts) = facts {
        if columns.has_user_event {
            for thread_id in &facts.user_event_thread_ids {
                updated_rows += transaction
                    .execute(
                        "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                        [thread_id.as_str()],
                    )
                    .map_err(|error| format_sqlite_write_error(db_path, &error))?;
            }
        }
        if columns.cwd {
            for (thread_id, cwd) in &facts.cwd_by_thread_id {
                updated_rows += transaction
                    .execute(
                        "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                        (cwd.as_str(), thread_id.as_str()),
                    )
                    .map_err(|error| format_sqlite_write_error(db_path, &error))?;
            }
        }
    }
    if let Err(error) = transaction.commit() {
        if modules::db::is_unusable_sqlite_database_error(&error) {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        return Err(format_sqlite_write_error(db_path, &error));
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
    if names.is_empty() {
        return Ok(None);
    }
    Ok(Some(ThreadsTableColumns {
        model_provider: names.contains("model_provider"),
        has_user_event: names.contains("has_user_event"),
        first_user_message: names.contains("first_user_message"),
        thread_source: names.contains("thread_source"),
        cwd: names.contains("cwd"),
    }))
}

fn build_threads_repair_where_clause(columns: ThreadsTableColumns) -> Option<String> {
    let mut predicates = Vec::new();
    if columns.model_provider {
        predicates.push("COALESCE(model_provider, '') <> ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(has_user_event, 0) <> 1)");
    }
    if columns.thread_source && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(thread_source, '') = '')");
    }
    if predicates.is_empty() {
        None
    } else {
        Some(predicates.join(" OR "))
    }
}

fn build_threads_repair_set_clause(columns: ThreadsTableColumns) -> String {
    let mut assignments = Vec::new();
    if columns.model_provider {
        assignments.push("model_provider = ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        assignments.push(
            "has_user_event = CASE WHEN COALESCE(first_user_message, '') <> '' THEN 1 ELSE has_user_event END",
        );
    }
    if columns.thread_source && columns.first_user_message {
        assignments.push(
            "thread_source = CASE WHEN COALESCE(thread_source, '') = '' AND COALESCE(first_user_message, '') <> '' THEN 'user' ELSE thread_source END",
        );
    }
    assignments.join(", ")
}

fn format_sqlite_read_error(path: &Path, action: &str, error: &rusqlite::Error) -> String {
    format!("{} ({}): {}", action, path.display(), error)
}

fn format_sqlite_write_error(path: &Path, error: &rusqlite::Error) -> String {
    let message = error.to_string();
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("database is locked") || lowered.contains("database busy") {
        return format!(
            "Codex SQLite 会话库当前被占用，请关闭 Codex / Codex App 后重试 ({}): {}",
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
    if let Some(updated_content) = change.updated_content.as_ref() {
        match updated_content {
            RolloutProviderUpdate::FullContent(content) => {
                write_bytes_atomic(&change.absolute_path, content.as_bytes())?;
            }
            RolloutProviderUpdate::FirstLine(line) => {
                rewrite_rollout_first_line(&change.absolute_path, line)?;
            }
        }
    }
    modules::codex_session_file_time::restore_modified_time(
        &change.absolute_path,
        change.target_modified_at.or(original_modified_at),
    )
}

fn rewrite_rollout_first_line(path: &Path, updated_first_line: &str) -> Result<(), String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取 rollout 文件失败 ({}): {}", path.display(), error))?;
    let (first_segment, rest) = match content.find('\n') {
        Some(index) => (&content[..index + 1], &content[index + 1..]),
        None => (content.as_str(), ""),
    };
    let (_, separator) = split_line_ending(first_segment);
    let mut output = String::new();
    output.push_str(updated_first_line);
    output.push_str(separator);
    output.push_str(rest);
    write_bytes_atomic(path, output.as_bytes())
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

fn sqlite_candidate_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = sqlite_dir_session_dbs(data_dir);
    let legacy = data_dir.join(STATE_DB_FILE);
    if !paths.iter().any(|path| path == &legacy) {
        paths.push(legacy);
    }
    paths
}

fn sqlite_candidate_paths_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
) -> Vec<PathBuf> {
    match options.sqlite_scope {
        SqliteRepairScope::LegacyStateOnly => vec![data_dir.join(STATE_DB_FILE)],
        SqliteRepairScope::OfficialStateDbs => official_state_db_candidate_paths(data_dir),
        SqliteRepairScope::AllSessionDbs => sqlite_candidate_paths(data_dir),
    }
}

fn official_state_db_candidate_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique_path(
        &mut paths,
        data_dir.join(SQLITE_DIR_NAME).join(OFFICIAL_STATE_DB_FILE),
    );
    push_unique_path(&mut paths, data_dir.join(STATE_DB_FILE));
    paths
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn sqlite_dir_session_dbs(data_dir: &Path) -> Vec<PathBuf> {
    let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
    let Ok(entries) = fs::read_dir(&sqlite_dir) else {
        return Vec::new();
    };
    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| is_sqlite_candidate(path))
        .filter(|path| has_codex_session_table(path))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|path| {
        (
            path.file_name()
                .map(|name| name != OsStr::new(PREFERRED_SQLITE_DB_FILE))
                .unwrap_or(true),
            path.file_name().map(|name| name.to_os_string()),
        )
    });
    candidates
}

fn is_sqlite_candidate(path: &Path) -> bool {
    matches!(
        path.extension().and_then(OsStr::to_str),
        Some("db") | Some("sqlite") | Some("sqlite3")
    )
}

fn has_codex_session_table(path: &Path) -> bool {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    ["threads", "automation_runs", "inbox_items"]
        .iter()
        .any(|table| {
            connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                    [table],
                    |_| Ok(()),
                )
                .is_ok()
        })
}

fn relative_to_instance_root(data_dir: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(data_dir).unwrap_or(path).to_path_buf()
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

fn backup_sqlite_databases(
    data_dir: &Path,
    backup_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
) -> Result<Vec<String>, String> {
    let mut backed_up = Vec::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        if !db_path.exists() {
            continue;
        }
        let relative = relative_to_instance_root(data_dir, &db_path);
        let backup_db_path = backup_dir.join("db").join(&relative);
        if let Some(parent) = backup_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 备份目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        let connection = Connection::open(&db_path).map_err(|error| {
            format!(
                "打开 SQLite 会话库以创建一致备份失败 ({}): {}",
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
                    "删除旧 SQLite 备份失败 ({}): {}",
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
                    "备份 SQLite 会话库失败 ({} -> {}): {}",
                    db_path.display(),
                    backup_db_path.display(),
                    error
                )
            })?;
        backed_up.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(backed_up)
}

fn restore_sqlite_databases_from_backup(
    data_dir: &Path,
    backup_dir: &Path,
) -> Result<Vec<String>, String> {
    let backup_db_root = backup_dir.join("db");
    if !backup_db_root.exists() {
        return Ok(Vec::new());
    }
    let backup_paths = list_backup_sqlite_files(&backup_db_root)?;
    let mut restored = Vec::new();
    for backup_db_path in backup_paths {
        let relative = backup_db_path
            .strip_prefix(&backup_db_root)
            .map_err(|_| format!("无法计算 SQLite 备份相对路径: {}", backup_db_path.display()))?;
        let target_db_path = data_dir.join(relative);
        if let Some(parent) = target_db_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!("创建 SQLite 恢复目录失败 ({}): {}", parent.display(), error)
            })?;
        }
        remove_sqlite_sidecar_files(&target_db_path)?;
        fs::copy(&backup_db_path, &target_db_path).map_err(|error| {
            format!(
                "恢复 SQLite 会话库失败 ({} -> {}): {}",
                backup_db_path.display(),
                target_db_path.display(),
                error
            )
        })?;
        remove_sqlite_sidecar_files(&target_db_path)?;
        restored.push(relative.to_string_lossy().replace('\\', "/"));
    }
    Ok(restored)
}

fn list_backup_sqlite_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|error| format!("读取 SQLite 备份目录失败 ({}): {}", root.display(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!("读取 SQLite 备份目录项失败 ({}): {}", root.display(), error)
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "读取 SQLite 备份文件类型失败 ({}): {}",
                path.display(),
                error
            )
        })?;
        if file_type.is_dir() {
            result.extend(list_backup_sqlite_files(&path)?);
        } else if file_type.is_file() {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}

fn backup_instance_files(
    data_dir: &Path,
    rollout_changes: &[RolloutProviderChange],
    include_sqlite: bool,
    include_session_index: bool,
    instance_id: &str,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
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
    let mut sqlite_backup_files = Vec::new();
    for change in rollout_changes {
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

    if include_sqlite {
        sqlite_backup_files = backup_sqlite_databases(data_dir, &backup_dir, options)?;
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

    let manifest = json!({
        "instanceId": instance_id,
        "instanceRoot": data_dir,
        "targetProvider": target_provider,
        "createdAt": Utc::now().to_rfc3339(),
        "hasSqliteBackup": !sqlite_backup_files.is_empty(),
        "sqliteFiles": sqlite_backup_files,
        "hasSessionIndexBackup": session_index_backup_created,
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
        let _ = restore_sqlite_databases_from_backup(data_dir, backup_dir)?;
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

    fn repair_options(
        mode: CodexSessionVisibilityRepairMode,
    ) -> CodexSessionVisibilityRepairOptions {
        CodexSessionVisibilityRepairOptions::for_mode(mode)
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

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 3);
        assert!(!scan.skipped_unusable_database);

        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
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

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan sqlite");
        assert_eq!(scan.rows_to_update, 1);
        let updated_rows =
            update_sqlite_provider_for_options(&data_dir, "relay", options, &selection)
                .expect("update sqlite");
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

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_repair_uses_official_state_dbs_without_touching_rollouts() {
        let data_dir = make_temp_dir("codex-session-quick-official-state-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let legacy_db_path = data_dir.join(STATE_DB_FILE);
        let unrelated_db_path = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        for db_path in [&official_db_path, &legacy_db_path, &unrelated_db_path] {
            let connection = Connection::open(db_path).expect("open sqlite");
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
                     VALUES ('thread-1', 'old', 0, 'hello', '')",
                    [],
                )
                .expect("insert row");
        }

        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("16");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-thread-1.jsonl");
        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan quick sqlite");
        assert_eq!(scan.rows_to_update, 2);
        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &[],
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("quick repair");
        assert_eq!(repaired.updated_sqlite_rows, 2);
        assert_eq!(repaired.updated_sqlite_timestamp_rows, 0);
        assert_eq!(repaired.added_session_index_entries, 0);
        assert_eq!(repaired.updated_session_index_entries, 0);

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read rollout"),
            rollout_content
        );

        for db_path in [&official_db_path, &legacy_db_path] {
            let connection = Connection::open(db_path).expect("reopen sqlite");
            let row = connection
                .query_row(
                    "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'thread-1'",
                    [],
                    |row| {
                        Ok((
                            row.get::<usize, String>(0)?,
                            row.get::<usize, i64>(1)?,
                            row.get::<usize, String>(2)?,
                        ))
                    },
                )
                .expect("read repaired row");
            assert_eq!(row, ("relay".to_string(), 1, "user".to_string()));
        }

        let connection = Connection::open(&unrelated_db_path).expect("reopen unrelated sqlite");
        let unrelated_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read unrelated provider");
        assert_eq!(unrelated_provider, "old");

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_repair_updates_rollouts_referenced_by_official_state_dbs() {
        let data_dir = make_temp_dir("codex-session-quick-referenced-rollout-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let connection = Connection::open(&official_db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");

        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("17");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let referenced_rollout = rollout_dir.join("rollout-thread-1.jsonl");
        let unreferenced_rollout = rollout_dir.join("rollout-thread-2.jsonl");
        let referenced_relative = referenced_rollout
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                [referenced_relative.as_str()],
            )
            .expect("insert thread");
        drop(connection);

        let old_line = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old-later\"}}\n"
        );
        let unreferenced_line =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-2\",\"model_provider\":\"old\"}}\n";
        fs::write(&referenced_rollout, old_line).expect("write referenced rollout");
        fs::write(&unreferenced_rollout, unreferenced_line).expect("write unreferenced rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick);
        let selection = RepairTargetSelection::default();
        let rollout_changes =
            collect_referenced_rollout_provider_changes(&data_dir, "relay", options, &selection)
                .expect("collect referenced rollout changes");
        assert_eq!(rollout_changes.len(), 1);
        assert_eq!(rollout_changes[0].absolute_path, referenced_rollout);

        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &rollout_changes,
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("quick repair");
        assert_eq!(repaired.updated_sqlite_rows, 1);

        let referenced_content =
            fs::read_to_string(&referenced_rollout).expect("read referenced rollout");
        assert!(referenced_content.contains("\"model_provider\":\"relay\""));
        assert!(referenced_content.contains("\"model_provider\":\"old-later\""));
        assert_eq!(
            fs::read_to_string(&unreferenced_rollout).expect("read unreferenced rollout"),
            unreferenced_line
        );

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn deep_mode_repairs_official_state_db_and_referenced_rollout_all_session_meta() {
        let data_dir = make_temp_dir("codex-session-deep-compat-official-state-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let unrelated_db_path = sqlite_dir.join(PREFERRED_SQLITE_DB_FILE);
        let rollout_dir = data_dir.join("sessions").join("2026").join("06").join("17");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let referenced_rollout = rollout_dir.join("rollout-thread-1.jsonl");
        let referenced_relative = referenced_rollout
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");

        for db_path in [&official_db_path, &unrelated_db_path] {
            let connection = Connection::open(db_path).expect("open sqlite");
            connection
                .execute(
                    "CREATE TABLE threads (
                        id TEXT PRIMARY KEY,
                        rollout_path TEXT,
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
                    "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                     VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                    [referenced_relative.as_str()],
                )
                .expect("insert row");
        }
        fs::write(
            &referenced_rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n",
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old-later\"}}\n"
            ),
        )
        .expect("write referenced rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Deep);
        assert_eq!(options.mode, CodexSessionVisibilityRepairMode::Deep);
        assert_eq!(options.sqlite_scope, SqliteRepairScope::OfficialStateDbs);
        assert!(!options.repair_rollout);
        assert!(options.repair_referenced_rollouts);
        assert!(options.rewrite_all_session_meta);
        assert!(!options.repair_session_index);
        assert!(!options.rebuild_metadata);

        let selection = RepairTargetSelection::default();
        let rollout_changes =
            collect_referenced_rollout_provider_changes(&data_dir, "relay", options, &selection)
                .expect("collect deep referenced rollout changes");
        assert_eq!(rollout_changes.len(), 1);
        let scan = count_sqlite_rows_to_update_for_options(&data_dir, "relay", options, &selection)
            .expect("scan compatibility sqlite");
        assert_eq!(scan.rows_to_update, 1);

        let repaired = repair_single_instance(
            &data_dir,
            "relay",
            &rollout_changes,
            true,
            false,
            false,
            options,
            &selection,
        )
        .expect("compatibility repair");
        assert_eq!(repaired.updated_sqlite_rows, 1);

        let connection = Connection::open(&official_db_path).expect("reopen official sqlite");
        let official_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read official provider");
        assert_eq!(official_provider, "relay");

        let connection = Connection::open(&unrelated_db_path).expect("reopen unrelated sqlite");
        let unrelated_provider = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'thread-1'",
                [],
                |row| row.get::<usize, String>(0),
            )
            .expect("read unrelated provider");
        assert_eq!(unrelated_provider, "old");

        let referenced_content =
            fs::read_to_string(&referenced_rollout).expect("read deep repaired rollout");
        assert!(referenced_content.contains("\"model_provider\":\"relay\""));
        assert!(!referenced_content.contains("old-later"));

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn auto_repair_mode_stays_on_official_state_db_only() {
        let options = CodexSessionVisibilityRepairOptions::for_auto_repair_mode(
            CodexSessionVisibilityAutoRepairMode::Current,
        );
        assert_eq!(options.mode, CodexSessionVisibilityRepairMode::Quick);
        assert_eq!(options.sqlite_scope, SqliteRepairScope::OfficialStateDbs);
        assert!(!options.repair_rollout);
        assert!(options.repair_referenced_rollouts);
        assert!(!options.rewrite_all_session_meta);
        assert!(!options.repair_session_index);
        assert!(!options.rebuild_metadata);
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

        let backup_dir = backup_instance_files(
            &data_dir,
            &[],
            true,
            false,
            "default",
            "relay",
            repair_options(CodexSessionVisibilityRepairMode::Quick),
        )
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

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn dry_run_reports_planned_changes_without_writing_files_or_backups() {
        let data_dir = make_temp_dir("codex-session-dry-run-test");
        let sqlite_dir = data_dir.join(SQLITE_DIR_NAME);
        fs::create_dir_all(&sqlite_dir).expect("create sqlite dir");
        let official_db_path = sqlite_dir.join(OFFICIAL_STATE_DB_FILE);
        let connection = Connection::open(&official_db_path).expect("open sqlite");
        connection
            .execute(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    has_user_event INTEGER,
                    first_user_message TEXT,
                    thread_source TEXT
                )",
                [],
            )
            .expect("create threads table");

        let rollout_dir = data_dir.join("sessions").join("2026").join("07").join("03");
        fs::create_dir_all(&rollout_dir).expect("create rollout dir");
        let rollout_path = rollout_dir.join("rollout-thread-1.jsonl");
        let rollout_relative = rollout_path
            .strip_prefix(&data_dir)
            .expect("relative rollout")
            .to_string_lossy()
            .replace('\\', "/");
        connection
            .execute(
                "INSERT INTO threads (id, rollout_path, model_provider, has_user_event, first_user_message, thread_source)
                 VALUES ('thread-1', ?1, 'old', 0, 'hello', '')",
                [rollout_relative.as_str()],
            )
            .expect("insert thread");
        drop(connection);

        let rollout_content =
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-1\",\"model_provider\":\"old\"}}\n";
        fs::write(&rollout_path, rollout_content).expect("write rollout");

        let options = repair_options(CodexSessionVisibilityRepairMode::Quick).with_dry_run(true);
        let selection = RepairTargetSelection::from_inputs(Some("relay".to_string()), None, None)
            .expect("selection");
        let summary = repair_session_visibility_for_instances_with_options(
            options,
            None,
            None,
            selection,
            vec![CodexSyncInstance {
                id: "dry-run-instance".to_string(),
                name: "Dry run instance".to_string(),
                data_dir: data_dir.clone(),
                last_pid: None,
            }],
        )
        .expect("dry run summary");

        assert_eq!(summary.instance_count, 1);
        assert_eq!(summary.mutated_instance_count, 1);
        assert_eq!(summary.changed_rollout_file_count, 1);
        assert_eq!(summary.updated_sqlite_row_count, 1);
        assert!(summary.backup_dirs.is_empty());
        assert_eq!(summary.items.len(), 1);
        assert!(summary.items[0].backup_dir.is_none());
        assert!(summary.message.contains("预览将"));

        assert_eq!(
            fs::read_to_string(&rollout_path).expect("read rollout after dry run"),
            rollout_content
        );
        let connection = Connection::open(&official_db_path).expect("reopen sqlite");
        let row = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'thread-1'",
                [],
                |row| {
                    Ok((
                        row.get::<usize, String>(0)?,
                        row.get::<usize, i64>(1)?,
                        row.get::<usize, String>(2)?,
                    ))
                },
            )
            .expect("read unchanged row");
        assert_eq!(row, ("old".to_string(), 0, "".to_string()));

        let backup_count = fs::read_dir(&data_dir)
            .expect("read data dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX)
            })
            .count();
        assert_eq!(backup_count, 0);

        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }
}
