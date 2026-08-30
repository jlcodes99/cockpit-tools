// Codex Session Visibility：Repair options, progress reporting and public repair entrypoints。
// 通过 include! 保持原模块作用域、私有调用关系和修复事务行为。
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::modules;
use chrono::{TimeZone, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params_from_iter, Connection, OpenFlags};
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
const GLOBAL_STATE_FILE: &str = ".codex-global-state.json";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const SESSION_VISIBILITY_REPAIR_BACKUP_PREFIX: &str = "backup-";
const SESSION_VISIBILITY_REPAIR_BACKUP_SUFFIX: &str = "-session-visibility-repair";
const MAX_SESSION_VISIBILITY_REPAIR_BACKUPS: usize = 1;
const SESSION_INDEX_ACTIVITY_DRIFT_MS: i128 = 3_600_000;
pub const SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT: &str =
    "codex:session_visibility_repair_progress";
static SESSION_VISIBILITY_REPAIR_LOCK: Mutex<()> = Mutex::new(());

fn acquire_session_visibility_repair_lock() -> Result<MutexGuard<'static, ()>, String> {
    #[cfg(test)]
    {
        return SESSION_VISIBILITY_REPAIR_LOCK
            .lock()
            .map_err(|_| "Codex 历史会话修复任务锁已损坏".to_string());
    }
    #[cfg(not(test))]
    SESSION_VISIBILITY_REPAIR_LOCK
        .try_lock()
        .map_err(|_| "已有 Codex 历史会话修复任务正在执行，请等待完成后重试".to_string())
}

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
    pub inserted_catalog_row_count: usize,
    pub removed_catalog_row_count: usize,
    pub updated_global_state_entry_count: usize,
    pub skipped_rollout_file_count: usize,
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
    pub inserted_catalog_row_count: usize,
    pub removed_catalog_row_count: usize,
    pub updated_global_state_entry_count: usize,
    pub skipped_rollout_file_count: usize,
    pub encrypted_content_warning: Option<String>,
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
    source_modified_at: Option<SystemTime>,
    source_size: u64,
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
    repair_local_thread_catalog: bool,
    normalize_global_state: bool,
    require_stopped_instances: bool,
    sidebar_visible_only: bool,
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
            repair_local_thread_catalog: false,
            normalize_global_state: false,
            require_stopped_instances: false,
            sidebar_visible_only: true,
        }
    }

    fn full_provider_migration() -> Self {
        Self {
            mode: CodexSessionVisibilityRepairMode::Deep,
            dry_run: false,
            repair_rollout: true,
            repair_referenced_rollouts: false,
            rewrite_all_session_meta: true,
            sqlite_scope: SqliteRepairScope::AllSessionDbs,
            repair_sqlite_timestamps: false,
            collect_rollout_thread_facts: true,
            repair_session_index: false,
            update_existing_session_index_entries: false,
            rebuild_metadata: true,
            repair_local_thread_catalog: true,
            normalize_global_state: true,
            require_stopped_instances: true,
            sidebar_visible_only: false,
        }
    }

    fn for_mode(mode: CodexSessionVisibilityRepairMode) -> Self {
        match mode {
            CodexSessionVisibilityRepairMode::Quick => Self::official_state_db_only(mode),
            CodexSessionVisibilityRepairMode::Deep => Self::full_provider_migration(),
        }
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
    subagent_thread_ids: HashSet<String>,
    encrypted_content_counts: HashMap<String, usize>,
}

#[derive(Debug, Clone, Copy, Default)]
struct CatalogRepairCounts {
    inserted_rows: usize,
    removed_rows: usize,
    updated_rows: usize,
}

impl CatalogRepairCounts {
    fn total(self) -> usize {
        self.inserted_rows + self.removed_rows + self.updated_rows
    }
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
    inserted_catalog_rows: usize,
    removed_catalog_rows: usize,
    updated_global_state_entries: usize,
    skipped_rollout_files: usize,
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
    id: bool,
    model_provider: bool,
    has_user_event: bool,
    first_user_message: bool,
    thread_source: bool,
    cwd: bool,
    archived: bool,
    preview: bool,
    rollout_path: bool,
    source: bool,
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

/// Repairs the launch target only, using the same bounded quick-repair plan as the
/// automatic multi-instance repair. The caller supplies the already-resolved data
/// directory so this path never discovers or mutates another configured instance.
pub fn repair_session_visibility_quick_for_instance(
    instance_id: &str,
    instance_name: &str,
    data_dir: &Path,
) -> Result<CodexSessionVisibilityRepairSummary, String> {
    let _repair_guard = acquire_session_visibility_repair_lock()?;
    let started = std::time::Instant::now();
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] launch-target quick repair started: instance_id={}, instance_name={}, data_dir={}",
        instance_id,
        instance_name,
        data_dir.display()
    ));
    let result = repair_session_visibility_for_instances_with_options(
        CodexSessionVisibilityRepairOptions::for_auto_repair_mode(
            CodexSessionVisibilityAutoRepairMode::Current,
        ),
        None,
        None,
        RepairTargetSelection::default(),
        vec![CodexSyncInstance {
            id: instance_id.to_string(),
            name: instance_name.to_string(),
            data_dir: data_dir.to_path_buf(),
            last_pid: None,
        }],
    );
    match &result {
        Ok(summary) => modules::logger::log_info(&format!(
            "[Codex Session Visibility] launch-target quick repair finished: instance_id={}, mutated_instances={}, rollout_files={}, sqlite_rows={}, elapsed_ms={}",
            instance_id,
            summary.mutated_instance_count,
            summary.changed_rollout_file_count,
            summary.updated_sqlite_row_count,
            started.elapsed().as_millis()
        )),
        Err(error) => modules::logger::log_warn(&format!(
            "[Codex Session Visibility] launch-target quick repair failed: instance_id={}, data_dir={}, elapsed_ms={}, error={}",
            instance_id,
            data_dir.display(),
            started.elapsed().as_millis(),
            error
        )),
    }
    result
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
    let _repair_guard = acquire_session_visibility_repair_lock()?;
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
    let mut inserted_catalog_row_count = 0usize;
    let mut removed_catalog_row_count = 0usize;
    let mut updated_global_state_entry_count = 0usize;
    let mut skipped_rollout_file_count = 0usize;
    let mut encrypted_content_counts = HashMap::new();
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
        let rollout_facts = if options.collect_rollout_thread_facts {
            Some(collect_rollout_thread_facts(
                &instance.data_dir,
                &selection,
            )?)
        } else {
            None
        };
        if let Some(facts) = &rollout_facts {
            for (provider, count) in &facts.encrypted_content_counts {
                *encrypted_content_counts
                    .entry(provider.clone())
                    .or_insert(0usize) += count;
            }
        }
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
        let empty_rollout_facts = RolloutThreadFacts::default();
        let catalog_scan = if options.repair_local_thread_catalog {
            repair_local_thread_catalog_for_options(
                &instance.data_dir,
                &target_provider,
                &selection,
                rollout_facts.as_ref().unwrap_or(&empty_rollout_facts),
                true,
            )?
        } else {
            CatalogRepairCounts::default()
        };
        let global_state_entries_to_update = if options.normalize_global_state {
            normalize_global_state(&instance.data_dir, true)?
        } else {
            0
        };
        if sqlite_scan.skipped_unusable_database {
            skipped_sqlite_file_count += 1;
        }

        let instance_has_planned_changes = !rollout_changes.is_empty()
            || sqlite_rows_to_update > 0
            || sqlite_timestamp_rows_to_update > 0
            || session_index_scan.entries_to_add > 0
            || session_index_scan.entries_to_update > 0
            || catalog_scan.total() > 0
            || global_state_entries_to_update > 0;

        if instance_has_planned_changes
            && running
            && options.require_stopped_instances
            && !options.dry_run
        {
            return Err(format!(
                "{} 正在运行；完整历史会话修复需要先完全退出对应 Codex App/ChatGPT 实例，避免会话文件或 SQLite 在修复过程中继续变化",
                instance.name
            ));
        }

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
            inserted_catalog_row_count += catalog_scan.inserted_rows;
            removed_catalog_row_count += catalog_scan.removed_rows;
            updated_sqlite_row_count += catalog_scan.updated_rows;
            updated_global_state_entry_count += global_state_entries_to_update;
            items.push(CodexSessionVisibilityRepairItem {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                target_provider,
                changed_rollout_file_count: rollout_changes.len(),
                updated_sqlite_row_count: sqlite_rows_to_update + catalog_scan.updated_rows,
                updated_sqlite_timestamp_row_count: sqlite_timestamp_rows_to_update,
                added_session_index_entry_count: session_index_scan.entries_to_add,
                updated_session_index_entry_count: session_index_scan.entries_to_update,
                inserted_catalog_row_count: catalog_scan.inserted_rows,
                removed_catalog_row_count: catalog_scan.removed_rows,
                updated_global_state_entry_count: global_state_entries_to_update,
                skipped_rollout_file_count: 0,
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
                inserted_catalog_row_count: 0,
                removed_catalog_row_count: 0,
                updated_global_state_entry_count: 0,
                skipped_rollout_file_count: 0,
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
            sqlite_rows_to_update > 0
                || sqlite_timestamp_rows_to_update > 0
                || catalog_scan.total() > 0,
            session_index_scan.entries_to_add > 0 || session_index_scan.entries_to_update > 0,
            global_state_entries_to_update > 0,
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
                    sqlite_rows_to_update > 0
                        || sqlite_timestamp_rows_to_update > 0
                        || catalog_scan.total() > 0,
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

        let applied_rollout_count = rollout_changes
            .len()
            .saturating_sub(repaired.skipped_rollout_files);
        let instance_mutated = applied_rollout_count > 0
            || repaired.updated_sqlite_rows > 0
            || repaired.updated_sqlite_timestamp_rows > 0
            || repaired.added_session_index_entries > 0
            || repaired.updated_session_index_entries > 0
            || repaired.inserted_catalog_rows > 0
            || repaired.removed_catalog_rows > 0
            || repaired.updated_global_state_entries > 0;
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
        changed_rollout_file_count += applied_rollout_count;
        updated_sqlite_row_count += repaired.updated_sqlite_rows;
        updated_sqlite_timestamp_row_count += repaired.updated_sqlite_timestamp_rows;
        added_session_index_entry_count += repaired.added_session_index_entries;
        updated_session_index_entry_count += repaired.updated_session_index_entries;
        inserted_catalog_row_count += repaired.inserted_catalog_rows;
        removed_catalog_row_count += repaired.removed_catalog_rows;
        updated_global_state_entry_count += repaired.updated_global_state_entries;
        skipped_rollout_file_count += repaired.skipped_rollout_files;
        backup_dirs.push(backup_dir_string.clone());
        items.push(CodexSessionVisibilityRepairItem {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            target_provider,
            changed_rollout_file_count: applied_rollout_count,
            updated_sqlite_row_count: repaired.updated_sqlite_rows,
            updated_sqlite_timestamp_row_count: repaired.updated_sqlite_timestamp_rows,
            added_session_index_entry_count: repaired.added_session_index_entries,
            updated_session_index_entry_count: repaired.updated_session_index_entries,
            inserted_catalog_row_count: repaired.inserted_catalog_rows,
            removed_catalog_row_count: repaired.removed_catalog_rows,
            updated_global_state_entry_count: repaired.updated_global_state_entries,
            skipped_rollout_file_count: repaired.skipped_rollout_files,
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
        for instance in &instances {
            let scope = modules::backup_storage::scope_for_path(&instance.data_dir);
            let _ = modules::backup_storage::prune_behavior_backups("codex", &scope);
        }
    }

    let message = if options.dry_run {
        build_dry_run_summary_message(
            mutated_instance_count,
            changed_rollout_file_count,
            updated_sqlite_row_count,
            updated_sqlite_timestamp_row_count,
            added_session_index_entry_count,
            updated_session_index_entry_count,
            inserted_catalog_row_count,
            removed_catalog_row_count,
            updated_global_state_entry_count,
            skipped_rollout_file_count,
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
            inserted_catalog_row_count,
            removed_catalog_row_count,
            updated_global_state_entry_count,
            skipped_rollout_file_count,
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
        inserted_catalog_row_count,
        removed_catalog_row_count,
        updated_global_state_entry_count,
        skipped_rollout_file_count,
        encrypted_content_warning: build_encrypted_content_warning(
            &encrypted_content_counts,
            items
                .first()
                .map(|item| item.target_provider.as_str())
                .unwrap_or(DEFAULT_PROVIDER_ID),
        ),
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
    collect_session_visibility_repair_providers_for_instances(&instances)
}

fn collect_session_visibility_repair_providers_for_instances(
    instances: &[CodexSyncInstance],
) -> Result<CodexSessionVisibilityRepairProviderList, String> {
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

    for instance in instances {
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

        for db_path in official_state_db_candidate_paths(&instance.data_dir) {
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
    let rollout_facts = if options.collect_rollout_thread_facts {
        collect_rollout_thread_facts(data_dir, selection)?
    } else {
        RolloutThreadFacts::default()
    };
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
    let catalog_repairs = if options.repair_local_thread_catalog {
        repair_local_thread_catalog_for_options(
            data_dir,
            target_provider,
            selection,
            &rollout_facts,
            false,
        )?
    } else {
        CatalogRepairCounts::default()
    };
    let rollout_total = rollout_changes.len();
    let mut skipped_rollout_files = 0usize;
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
        if !rewrite_rollout_provider(change)? {
            skipped_rollout_files += 1;
        }
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
    let updated_global_state_entries = if options.normalize_global_state {
        normalize_global_state(data_dir, false)?
    } else {
        0
    };
    Ok(RepairSingleInstanceResult {
        updated_sqlite_rows: sqlite_rows_updated + catalog_repairs.updated_rows,
        updated_sqlite_timestamp_rows: sqlite_timestamp_rows_updated,
        added_session_index_entries: session_index_result.added_entries,
        updated_session_index_entries: session_index_result.updated_entries,
        inserted_catalog_rows: catalog_repairs.inserted_rows,
        removed_catalog_rows: catalog_repairs.removed_rows,
        updated_global_state_entries,
        skipped_rollout_files,
    })
}

fn build_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    updated_sqlite_timestamp_row_count: usize,
    added_session_index_entry_count: usize,
    updated_session_index_entry_count: usize,
    inserted_catalog_row_count: usize,
    removed_catalog_row_count: usize,
    updated_global_state_entry_count: usize,
    skipped_rollout_file_count: usize,
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
    let catalog_suffix = if inserted_catalog_row_count > 0 || removed_catalog_row_count > 0 {
        format!(
            "，补齐 {} 条本地会话目录、移除 {} 条子 Agent 目录记录",
            inserted_catalog_row_count, removed_catalog_row_count
        )
    } else {
        String::new()
    };
    let global_state_suffix = if updated_global_state_entry_count > 0 {
        format!(
            "，规范化 {} 项全局工作区状态",
            updated_global_state_entry_count
        )
    } else {
        String::new()
    };
    let skipped_rollout_suffix = if skipped_rollout_file_count > 0 {
        format!(
            "；{} 个会话文件在扫描后发生变化，已安全跳过",
            skipped_rollout_file_count
        )
    } else {
        String::new()
    };

    format!(
        "已为 {} 个实例修复历史会话：校正 {} 个会话文件，更新 {} 条 SQLite 记录，校正 {} 条 SQLite 时间记录{}{}{}{}{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_index_suffix,
        updated_index_suffix,
        catalog_suffix,
        global_state_suffix,
        running_suffix,
        metadata_suffix,
        skipped_rollout_suffix
    )
}

fn build_dry_run_summary_message(
    mutated_instance_count: usize,
    changed_rollout_file_count: usize,
    updated_sqlite_row_count: usize,
    updated_sqlite_timestamp_row_count: usize,
    added_session_index_entry_count: usize,
    updated_session_index_entry_count: usize,
    inserted_catalog_row_count: usize,
    removed_catalog_row_count: usize,
    updated_global_state_entry_count: usize,
    _skipped_rollout_file_count: usize,
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
        "。包含运行中的实例；完整修复前需要先完全退出对应 Codex App/ChatGPT 实例"
    } else {
        ""
    };
    let catalog_suffix = if inserted_catalog_row_count > 0 || removed_catalog_row_count > 0 {
        format!(
            "，补齐 {} 条本地会话目录、移除 {} 条子 Agent 目录记录",
            inserted_catalog_row_count, removed_catalog_row_count
        )
    } else {
        String::new()
    };
    let global_state_suffix = if updated_global_state_entry_count > 0 {
        format!(
            "，规范化 {} 项全局工作区状态",
            updated_global_state_entry_count
        )
    } else {
        String::new()
    };

    format!(
        "预览将为 {} 个实例修复历史会话：校正 {} 个会话文件，更新 {} 条 SQLite 记录，校正 {} 条 SQLite 时间记录{}{}{}{}{}",
        mutated_instance_count,
        changed_rollout_file_count,
        updated_sqlite_row_count,
        updated_sqlite_timestamp_row_count,
        added_index_suffix,
        updated_index_suffix,
        catalog_suffix,
        global_state_suffix,
        running_suffix
    )
}

