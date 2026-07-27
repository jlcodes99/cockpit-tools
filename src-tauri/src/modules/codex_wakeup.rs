use crate::modules::{account, codex_account, codex_local_access, logger, process};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const TASKS_FILE: &str = "codex_wakeup_tasks.json";
const HISTORY_FILE: &str = "codex_wakeup_history.json";
const RUNTIME_CONFIG_FILE: &str = "codex_wakeup_runtime_config.json";
const MANAGED_HOMES_DIR: &str = "codex_wakeup_homes";
const MAX_HISTORY_ITEMS: usize = 300;
const MAX_LOGGED_SEARCH_DIRS: usize = 8;
const REQUIRED_RUNTIME_PATH_CODEX_CLI: &str = "codex_cli_path";
const REQUIRED_RUNTIME_PATH_NODE: &str = "node_path";
const CLI_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_PROMPT: &str = "hi";
pub const PROGRESS_EVENT: &str = "codex://wakeup-progress";
const REASONING_EFFORT_LOW: &str = "low";
const REASONING_EFFORT_MEDIUM: &str = "medium";
const REASONING_EFFORT_HIGH: &str = "high";
const REASONING_EFFORT_XHIGH: &str = "xhigh";
const CODEX_WAKEUP_TEST_CANCELLED_MESSAGE: &str = "Codex 唤醒测试已取消";
const CODEX_WAKEUP_CANCEL_POLL_MS: u64 = 120;
const GPT_5_6_MODEL_PRESETS_MIGRATION_ID: &str = "add-gpt-5-6-model-presets";
const GPT_5_5_MODEL_PRESET_MIGRATION_ID: &str = "add-gpt-5-5-model-preset";
const PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID: &str =
    "prune-legacy-codex-model-presets-before-gpt-5-4";
const LEGACY_CODEX_MODEL_PRESET_IDS: &[&str] = &[
    "preset-gpt-5-3-codex",
    "preset-gpt-5-2-codex",
    "preset-gpt-5-2",
    "preset-gpt-5-1-codex-max",
    "preset-gpt-5-1-codex-mini",
];
const LEGACY_CODEX_MODEL_PRESET_MODELS: &[&str] = &[
    "gpt-5-codex",
    "gpt-5-codex-mini",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5.2",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
];

static TASKS_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));
static HISTORY_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));
static TEST_CANCEL_SCOPES: std::sync::LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn quarantine_corrupted_wakeup_file(path: &Path, label: &str, error: &impl std::fmt::Display) {
    match crate::modules::atomic_write::quarantine_file(path, "invalid-json") {
        Ok(Some(backup_path)) => logger::log_warn(&format!(
            "[CodexWakeup] {} 解析失败，已隔离并使用空状态: path={}, backup={}, error={}",
            label,
            path.display(),
            backup_path.display(),
            error
        )),
        Ok(None) => logger::log_warn(&format!(
            "[CodexWakeup] {} 解析失败，文件已不存在，使用空状态: path={}, error={}",
            label,
            path.display(),
            error
        )),
        Err(backup_error) => logger::log_warn(&format!(
            "[CodexWakeup] {} 解析失败，隔离失败，使用空状态: path={}, parse_error={}, backup_error={}",
            label,
            path.display(),
            error,
            backup_error
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCliInstallHint {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCliStatus {
    pub available: bool,
    pub binary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_codex_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_node_path: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_runtime_paths: Vec<String>,
    pub checked_at: i64,
    pub install_hints: Vec<CodexCliInstallHint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexCliResolvedRuntime {
    pub binary_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupRuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupSchedule {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daily_time: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekly_days: Vec<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_hours: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_reset_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_delay_minutes: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupModelPreset {
    pub id: String,
    pub name: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupTask {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub account_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<String>,
    pub schedule: CodexWakeupSchedule,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_timeout_minutes: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupState {
    pub enabled: bool,
    #[serde(default)]
    pub tasks: Vec<CodexWakeupTask>,
    #[serde(default = "default_model_presets")]
    pub model_presets: Vec<CodexWakeupModelPreset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub model_preset_migrations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexQuotaSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hourly_percentage: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hourly_reset_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_percentage: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_reset_time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupHistoryItem {
    pub id: String,
    pub run_id: String,
    pub timestamp: i64,
    pub trigger_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    pub account_id: String,
    pub account_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_context_text: Option<String>,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_refresh_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_before: Option<CodexQuotaSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_after: Option<CodexQuotaSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupBatchResult {
    pub run_id: String,
    pub runtime: CodexCliStatus,
    pub records: Vec<CodexWakeupHistoryItem>,
    pub success_count: usize,
    pub failure_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupOverview {
    pub runtime: CodexCliStatus,
    pub state: CodexWakeupState,
    pub history: Vec<CodexWakeupHistoryItem>,
}

#[derive(Debug, Clone)]
pub struct TaskRunContext {
    pub trigger_type: String,
    pub task_id: Option<String>,
    pub task_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupProgressPayload {
    pub run_id: String,
    pub trigger_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub failure_count: usize,
    pub running: bool,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<CodexWakeupHistoryItem>,
}

#[derive(Debug, Clone)]
struct ResolvedBinary {
    path: PathBuf,
    source: String,
    node_path: Option<PathBuf>,
    version: String,
}

#[derive(Debug, Clone)]
struct CliResolveError {
    message: String,
    required_runtime_paths: Vec<String>,
}

impl CliResolveError {
    fn new(message: impl Into<String>, required_runtime_paths: &[&str]) -> Self {
        let mut paths: Vec<String> = required_runtime_paths
            .iter()
            .map(|item| item.to_string())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        Self {
            message: message.into(),
            required_runtime_paths: paths,
        }
    }
}

#[derive(Debug)]
struct CommandOutput {
    reply: String,
    duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWakeupCliConversationResult {
    pub reply: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct CodexWakeupCliConversationDetailedError {
    pub message: String,
    pub status: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub last_message: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct CodexWakeupExecutionConfig {
    pub model: Option<String>,
    pub model_display_name: Option<String>,
    pub model_reasoning_effort: Option<String>,
}

impl Default for CodexWakeupState {
    fn default() -> Self {
        Self {
            enabled: false,
            tasks: Vec::new(),
            model_presets: default_model_presets(),
            model_preset_migrations: vec![
                GPT_5_6_MODEL_PRESETS_MIGRATION_ID.to_string(),
                GPT_5_5_MODEL_PRESET_MIGRATION_ID.to_string(),
                PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID.to_string(),
            ],
        }
    }
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn cancelled_error() -> String {
    CODEX_WAKEUP_TEST_CANCELLED_MESSAGE.to_string()
}

fn is_scope_cancelled(cancel_flag: Option<&Arc<AtomicBool>>) -> bool {
    cancel_flag
        .map(|flag| flag.load(Ordering::SeqCst))
        .unwrap_or(false)
}

fn resolve_cancel_flag(cancel_scope_id: Option<&str>) -> Result<Option<Arc<AtomicBool>>, String> {
    let Some(scope_id) = cancel_scope_id
        .map(str::trim)
        .filter(|item| !item.is_empty())
    else {
        return Ok(None);
    };

    let mut guard = TEST_CANCEL_SCOPES
        .lock()
        .map_err(|_| "Codex 唤醒取消作用域锁已损坏".to_string())?;
    let flag = guard
        .entry(scope_id.to_string())
        .or_insert_with(|| Arc::new(AtomicBool::new(false)))
        .clone();
    Ok(Some(flag))
}

pub fn cancel_wakeup_scope(cancel_scope_id: &str) -> Result<(), String> {
    let scope_id = cancel_scope_id.trim();
    if scope_id.is_empty() {
        return Ok(());
    }

    let flag = {
        let mut guard = TEST_CANCEL_SCOPES
            .lock()
            .map_err(|_| "Codex 唤醒取消作用域锁已损坏".to_string())?;
        guard.remove(scope_id)
    };

    if let Some(flag) = flag {
        flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

pub fn release_wakeup_scope(cancel_scope_id: &str) -> Result<(), String> {
    let scope_id = cancel_scope_id.trim();
    if scope_id.is_empty() {
        return Ok(());
    }

    let mut guard = TEST_CANCEL_SCOPES
        .lock()
        .map_err(|_| "Codex 唤醒取消作用域锁已损坏".to_string())?;
    guard.remove(scope_id);
    Ok(())
}

fn supported_reasoning_efforts() -> &'static [&'static str] {
    &[
        REASONING_EFFORT_LOW,
        REASONING_EFFORT_MEDIUM,
        REASONING_EFFORT_HIGH,
        REASONING_EFFORT_XHIGH,
    ]
}

fn normalize_reasoning_effort(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if supported_reasoning_efforts().contains(&normalized.as_str()) {
        Some(normalized)
    } else {
        None
    }
}

pub fn wakeup_runtime_status() -> CodexCliStatus {
    CodexCliStatus {
        available: true,
        binary_path: None,
        configured_codex_cli_path: None,
        configured_node_path: None,
        version: None,
        source: Some("official_chat".to_string()),
        message: Some("官方直连对话".to_string()),
        required_runtime_paths: Vec::new(),
        checked_at: now_ms(),
        install_hints: Vec::new(),
    }
}

fn default_reasoning_efforts_for_model(model: &str) -> Vec<String> {
    if model.trim().eq_ignore_ascii_case("gpt-5.1-codex-mini") {
        vec![
            REASONING_EFFORT_MEDIUM.to_string(),
            REASONING_EFFORT_HIGH.to_string(),
        ]
    } else {
        supported_reasoning_efforts()
            .iter()
            .map(|item| item.to_string())
            .collect()
    }
}

fn default_model_presets() -> Vec<CodexWakeupModelPreset> {
    let items = [
        ("preset-gpt-5-6-sol", "GPT-5.6 Sol", "gpt-5.6-sol"),
        ("preset-gpt-5-6-terra", "GPT-5.6 Terra", "gpt-5.6-terra"),
        ("preset-gpt-5-6-luna", "GPT-5.6 Luna", "gpt-5.6-luna"),
        ("preset-gpt-5-5", "GPT-5.5", "gpt-5.5"),
        ("preset-gpt-5-4", "GPT-5.4", "gpt-5.4"),
        ("preset-gpt-5-4-mini", "GPT-5.4-Mini", "gpt-5.4-mini"),
    ];

    items
        .into_iter()
        .map(|(id, name, model)| {
            let allowed_reasoning_efforts = default_reasoning_efforts_for_model(model);
            let default_reasoning_effort = if allowed_reasoning_efforts
                .iter()
                .any(|item| item == REASONING_EFFORT_MEDIUM)
            {
                REASONING_EFFORT_MEDIUM.to_string()
            } else {
                allowed_reasoning_efforts
                    .first()
                    .cloned()
                    .unwrap_or_else(|| REASONING_EFFORT_MEDIUM.to_string())
            };
            CodexWakeupModelPreset {
                id: id.to_string(),
                name: name.to_string(),
                model: model.to_string(),
                allowed_reasoning_efforts,
                default_reasoning_effort,
            }
        })
        .collect()
}

fn gpt_5_6_model_presets() -> Vec<CodexWakeupModelPreset> {
    default_model_presets()
        .into_iter()
        .filter(|preset| preset.model.starts_with("gpt-5.6-"))
        .collect()
}

fn ensure_gpt_5_6_model_presets(state: &mut CodexWakeupState) -> bool {
    if state
        .model_preset_migrations
        .iter()
        .any(|item| item == GPT_5_6_MODEL_PRESETS_MIGRATION_ID)
    {
        return false;
    }

    state
        .model_preset_migrations
        .push(GPT_5_6_MODEL_PRESETS_MIGRATION_ID.to_string());

    let existing_models = state
        .model_presets
        .iter()
        .map(|preset| preset.model.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let additions = gpt_5_6_model_presets()
        .into_iter()
        .filter(|preset| !existing_models.contains(&preset.model.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    if additions.is_empty() {
        return true;
    }

    state.model_presets.splice(0..0, additions);
    true
}

fn gpt_5_5_model_preset() -> CodexWakeupModelPreset {
    let model = "gpt-5.5";
    let allowed_reasoning_efforts = default_reasoning_efforts_for_model(model);
    let default_reasoning_effort = if allowed_reasoning_efforts
        .iter()
        .any(|item| item == REASONING_EFFORT_MEDIUM)
    {
        REASONING_EFFORT_MEDIUM.to_string()
    } else {
        allowed_reasoning_efforts
            .first()
            .cloned()
            .unwrap_or_else(|| REASONING_EFFORT_MEDIUM.to_string())
    };
    CodexWakeupModelPreset {
        id: "preset-gpt-5-5".to_string(),
        name: "GPT-5.5".to_string(),
        model: model.to_string(),
        allowed_reasoning_efforts,
        default_reasoning_effort,
    }
}

fn ensure_gpt_5_5_model_preset(state: &mut CodexWakeupState) -> bool {
    if state
        .model_preset_migrations
        .iter()
        .any(|item| item == GPT_5_5_MODEL_PRESET_MIGRATION_ID)
    {
        return false;
    }

    state
        .model_preset_migrations
        .push(GPT_5_5_MODEL_PRESET_MIGRATION_ID.to_string());

    if state
        .model_presets
        .iter()
        .any(|preset| preset.model.trim().eq_ignore_ascii_case("gpt-5.5"))
    {
        return true;
    }

    state.model_presets.insert(0, gpt_5_5_model_preset());
    true
}

fn is_legacy_codex_model_preset(preset: &CodexWakeupModelPreset) -> bool {
    let id = preset.id.trim();
    let model = preset.model.trim();
    LEGACY_CODEX_MODEL_PRESET_IDS
        .iter()
        .any(|item| id.eq_ignore_ascii_case(item))
        || LEGACY_CODEX_MODEL_PRESET_MODELS
            .iter()
            .any(|item| model.eq_ignore_ascii_case(item))
}

fn prune_legacy_model_presets(state: &mut CodexWakeupState) -> bool {
    if state
        .model_preset_migrations
        .iter()
        .any(|item| item == PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID)
    {
        return false;
    }

    state
        .model_preset_migrations
        .push(PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID.to_string());
    state
        .model_presets
        .retain(|preset| !is_legacy_codex_model_preset(preset));
    true
}

fn apply_model_preset_migrations(state: &mut CodexWakeupState) -> bool {
    let mut changed = false;
    changed |= prune_legacy_model_presets(state);
    changed |= ensure_gpt_5_6_model_presets(state);
    changed |= ensure_gpt_5_5_model_preset(state);
    state.model_preset_migrations.sort();
    state.model_preset_migrations.dedup();
    changed
}

fn data_dir() -> Result<PathBuf, String> {
    account::get_data_dir()
}

fn tasks_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(TASKS_FILE))
}

fn history_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(HISTORY_FILE))
}

fn runtime_config_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(RUNTIME_CONFIG_FILE))
}

fn managed_homes_root() -> Result<PathBuf, String> {
    Ok(data_dir()?.join(MANAGED_HOMES_DIR))
}

fn managed_home_path(account_id: &str) -> Result<PathBuf, String> {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return Err("账号 ID 为空，无法定位受管 CODEX_HOME".to_string());
    }
    Ok(managed_homes_root()?.join(trimmed))
}

fn install_hints() -> Vec<CodexCliInstallHint> {
    #[cfg(target_os = "macos")]
    let mut hints = vec![
        CodexCliInstallHint {
            label: "Homebrew (Node.js)".to_string(),
            command: "brew install node".to_string(),
        },
        CodexCliInstallHint {
            label: "npm".to_string(),
            command: "npm install -g @openai/codex".to_string(),
        },
    ];
    #[cfg(not(target_os = "macos"))]
    let hints = vec![CodexCliInstallHint {
        label: "npm".to_string(),
        command: "npm install -g @openai/codex".to_string(),
    }];
    #[cfg(target_os = "macos")]
    {
        hints.push(CodexCliInstallHint {
            label: "Homebrew".to_string(),
            command: "brew install --cask codex".to_string(),
        });
    }
    hints
}

fn summarize_path_dirs_for_log(dirs: &[PathBuf]) -> String {
    if dirs.is_empty() {
        return "<empty>".to_string();
    }

    let mut preview: Vec<String> = dirs
        .iter()
        .take(MAX_LOGGED_SEARCH_DIRS)
        .map(|item| item.display().to_string())
        .collect();

    if dirs.len() > MAX_LOGGED_SEARCH_DIRS {
        preview.push(format!(
            "...(+{} more)",
            dirs.len() - MAX_LOGGED_SEARCH_DIRS
        ));
    }

    preview.join(" | ")
}

fn truncate_log_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut result = value.chars().take(max_chars).collect::<String>();
    result.push_str("...");
    result
}

fn format_optional_path_for_log(path: Option<&Path>) -> String {
    path.map(|item| item.display().to_string())
        .unwrap_or_else(|| "<none>".to_string())
}

fn normalize_text(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_runtime_config(raw: &CodexWakeupRuntimeConfig) -> CodexWakeupRuntimeConfig {
    CodexWakeupRuntimeConfig {
        codex_cli_path: normalize_text(raw.codex_cli_path.as_deref()),
        node_path: normalize_text(raw.node_path.as_deref()),
    }
}

fn is_team_like_plan(plan_type: Option<&str>) -> bool {
    let Some(raw) = plan_type else {
        return false;
    };
    let upper = raw.trim().to_ascii_uppercase();
    upper.contains("TEAM")
        || upper.contains("BUSINESS")
        || upper.contains("ENTERPRISE")
        || upper.contains("EDU")
}

fn decode_token_payload_value(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    serde_json::from_slice(&payload).ok()
}

fn read_json_string_map(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(|value| value.as_str())
            .and_then(|value| normalize_text(Some(value)))
    })
}

fn read_json_bool_map(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<bool> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(|value| value.as_bool()))
}

fn extract_workspace_title(account: &crate::models::codex::CodexAccount) -> Option<String> {
    let payload = decode_token_payload_value(&account.tokens.id_token)?;
    let auth = payload
        .get("https://api.openai.com/auth")
        .and_then(|value| value.as_object())?;
    let organizations = auth
        .get("organizations")
        .and_then(|value| value.as_array())?;
    let expected_org = normalize_text(account.organization_id.as_deref());
    let mut matched_title: Option<String> = None;
    let mut default_title: Option<String> = None;
    let mut first_title: Option<String> = None;

    for item in organizations {
        let Some(object) = item.as_object() else {
            continue;
        };
        let org_id = read_json_string_map(object, &["id", "organization_id", "workspace_id"]);
        let title = read_json_string_map(
            object,
            &[
                "title",
                "name",
                "display_name",
                "workspace_name",
                "organization_name",
            ],
        )
        .or_else(|| org_id.clone());
        let Some(title) = title else {
            continue;
        };

        if first_title.is_none() {
            first_title = Some(title.clone());
        }
        if read_json_bool_map(object, &["is_default"]) == Some(true) && default_title.is_none() {
            default_title = Some(title.clone());
        }
        if matched_title.is_none() && expected_org.is_some() && org_id == expected_org {
            matched_title = Some(title);
        }
    }

    matched_title.or(default_title).or(first_title)
}

fn resolve_account_context_text(account: &crate::models::codex::CodexAccount) -> Option<String> {
    let structure = normalize_text(account.account_structure.as_deref())
        .map(|value| value.to_ascii_lowercase());
    let is_personal = structure
        .as_deref()
        .map(|value| value.contains("personal"))
        .unwrap_or(false);

    if is_personal || (structure.is_none() && !is_team_like_plan(account.plan_type.as_deref())) {
        return Some("个人账户".to_string());
    }

    normalize_text(account.account_name.as_deref()).or_else(|| extract_workspace_title(account))
}

#[cfg(target_os = "windows")]
fn binary_candidates() -> &'static [&'static str] {
    &["codex.exe", "codex.cmd", "codex.bat", "codex"]
}

#[cfg(not(target_os = "windows"))]
fn binary_candidates() -> &'static [&'static str] {
    &["codex"]
}

#[cfg(target_os = "windows")]
fn node_binary_candidates() -> &'static [&'static str] {
    &["node.exe", "node.cmd", "node.bat", "node"]
}

#[cfg(not(target_os = "windows"))]
fn node_binary_candidates() -> &'static [&'static str] {
    &["node"]
}

fn collect_path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default()
}

fn append_home_cli_dirs(dirs: &mut Vec<PathBuf>) {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };

    let home = PathBuf::from(home);
    for dir in [
        home.join(".npm-global/bin"),
        home.join(".local/bin"),
        home.join(".cargo/bin"),
        home.join(".volta/bin"),
        home.join(".yarn/bin"),
        home.join("bin"),
    ] {
        push_unique_dir(dirs, dir);
    }
    append_version_manager_cli_dirs(dirs, &home);
}

/// Discover CLI bins managed by nvm / fnm / asdf, which GUI apps often miss
/// because they inherit a minimal PATH without login-shell hooks.
fn append_version_manager_cli_dirs(dirs: &mut Vec<PathBuf>, home: &Path) {
    if let Some(nvm_bin) = std::env::var_os("NVM_BIN") {
        push_unique_dir(dirs, PathBuf::from(nvm_bin));
    }
    if let Some(nvm_dir) = std::env::var_os("NVM_DIR") {
        append_node_version_bin_dirs(dirs, PathBuf::from(nvm_dir).join("versions/node"));
    } else {
        append_node_version_bin_dirs(dirs, home.join(".nvm/versions/node"));
    }

    if let Some(fnm_multishell) = std::env::var_os("FNM_MULTISHELL_PATH") {
        push_unique_dir(dirs, PathBuf::from(fnm_multishell));
    }
    if let Some(fnm_dir) = std::env::var_os("FNM_DIR") {
        append_node_version_bin_dirs(dirs, PathBuf::from(fnm_dir).join("node-versions"));
    } else {
        append_node_version_bin_dirs(dirs, home.join(".local/share/fnm/node-versions"));
        append_node_version_bin_dirs(dirs, home.join(".fnm/node-versions"));
    }

    let asdf_data = std::env::var_os("ASDF_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".asdf"));
    append_node_version_bin_dirs(dirs, asdf_data.join("installs/nodejs"));
    push_unique_dir(dirs, asdf_data.join("shims"));
}

fn append_node_version_bin_dirs(dirs: &mut Vec<PathBuf>, versions_root: PathBuf) {
    let Ok(entries) = std::fs::read_dir(&versions_root) else {
        return;
    };
    let mut version_dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // Prefer newer versions when multiple nvm installs exist.
    version_dirs.sort();
    version_dirs.reverse();
    for version_dir in version_dirs {
        // nvm: ~/.nvm/versions/node/v20.x.y/bin
        push_unique_dir(dirs, version_dir.join("bin"));
        // fnm: ~/.fnm/node-versions/v20.x.y/installation/bin
        push_unique_dir(dirs, version_dir.join("installation/bin"));
    }
}

#[cfg(target_os = "macos")]
fn append_platform_cli_dirs(dirs: &mut Vec<PathBuf>) {
    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ] {
        push_unique_dir(dirs, PathBuf::from(dir));
    }
    append_home_cli_dirs(dirs);
}

#[cfg(target_os = "windows")]
fn append_platform_cli_dirs(dirs: &mut Vec<PathBuf>) {
    if let Some(app_data) = std::env::var_os("APPDATA") {
        push_unique_dir(dirs, PathBuf::from(app_data).join("npm"));
    }
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn append_platform_cli_dirs(dirs: &mut Vec<PathBuf>) {
    append_home_cli_dirs(dirs);
}

fn collect_runtime_search_dirs() -> Vec<PathBuf> {
    let mut dirs = collect_path_dirs();
    append_platform_cli_dirs(&mut dirs);
    dirs
}

fn resolve_binary_in_dirs(dirs: &[PathBuf], candidates: &[&str]) -> Option<PathBuf> {
    for dir in dirs {
        for candidate in candidates {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn resolve_configured_binary_path(
    configured_path: &Path,
    candidates: &[&str],
    display_name: &str,
) -> Result<PathBuf, String> {
    if configured_path.is_file() {
        return Ok(configured_path.to_path_buf());
    }
    if configured_path.is_dir() {
        if let Some(path) = resolve_binary_in_dirs(&[configured_path.to_path_buf()], candidates) {
            return Ok(path);
        }
        return Err(format!(
            "自定义 {} 目录下未找到可执行文件: {}",
            display_name,
            configured_path.display()
        ));
    }
    Err(format!(
        "自定义 {} 路径不存在: {}",
        display_name,
        configured_path.display()
    ))
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if dir.as_os_str().is_empty() {
        return;
    }
    if !dirs.iter().any(|existing| existing == &dir) {
        dirs.push(dir);
    }
}

fn is_node_binary_name(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "node" | "node.exe" | "node.cmd" | "node.bat"
    )
}

fn read_script_header_line(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let first_line = bytes.split(|byte| *byte == b'\n').next()?;
    Some(
        String::from_utf8_lossy(first_line)
            .trim_end_matches('\r')
            .to_string(),
    )
}

#[derive(Debug, Clone)]
enum NodeLaunchRequirement {
    NotNeeded,
    Search,
    Direct(PathBuf),
}

fn parse_node_launch_requirement(path: &Path) -> NodeLaunchRequirement {
    let extension = path
        .extension()
        .and_then(|item| item.to_str())
        .map(|item| item.trim().to_ascii_lowercase());
    if matches!(extension.as_deref(), Some("js" | "mjs" | "cjs")) {
        return NodeLaunchRequirement::Search;
    }

    let Some(line) = read_script_header_line(path) else {
        return NodeLaunchRequirement::NotNeeded;
    };
    let Some(shebang) = line.strip_prefix("#!") else {
        return NodeLaunchRequirement::NotNeeded;
    };
    let shebang = shebang.trim();
    if shebang.is_empty() {
        return NodeLaunchRequirement::NotNeeded;
    }

    let mut parts = shebang.split_whitespace();
    let Some(program) = parts.next() else {
        return NodeLaunchRequirement::NotNeeded;
    };

    let program_path = PathBuf::from(program);
    if program_path
        .file_name()
        .and_then(|item| item.to_str())
        .map(is_node_binary_name)
        .unwrap_or(false)
    {
        return NodeLaunchRequirement::Direct(program_path);
    }

    if program_path
        .file_name()
        .and_then(|item| item.to_str())
        .map(|item| item.eq_ignore_ascii_case("env"))
        .unwrap_or(false)
    {
        for token in parts {
            if token == "-S" {
                continue;
            }
            if token.contains('=') {
                continue;
            }
            if is_node_binary_name(token) {
                return NodeLaunchRequirement::Search;
            }
            break;
        }
    }

    NodeLaunchRequirement::NotNeeded
}

fn resolve_binary_paths_from_path() -> Vec<PathBuf> {
    let dirs = collect_runtime_search_dirs();

    logger::log_info(&format!(
        "[CodexWakeup][CLI] 扫描 CLI 搜索目录查找 codex: 目录数={}, 预览={}",
        dirs.len(),
        summarize_path_dirs_for_log(&dirs)
    ));

    let mut paths = Vec::new();
    for dir in dirs {
        for candidate in binary_candidates() {
            let path = dir.join(candidate);
            if path.is_file() && !paths.iter().any(|existing| existing == &path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn resolve_node_from_binary_path(binary_path: &Path) -> Option<PathBuf> {
    let mut dirs = collect_runtime_search_dirs();

    if let Some(parent) = binary_path.parent() {
        push_unique_dir(&mut dirs, parent.to_path_buf());
    }

    for ancestor in binary_path.ancestors().skip(1) {
        push_unique_dir(&mut dirs, ancestor.join("bin"));
    }

    logger::log_info(&format!(
        "[CodexWakeup][CLI] 扫描 node 解释器目录: codex_path={}, 目录数={}, 预览={}",
        binary_path.display(),
        dirs.len(),
        summarize_path_dirs_for_log(&dirs)
    ));

    resolve_binary_in_dirs(&dirs, node_binary_candidates())
}

fn resolve_node_for_binary(
    binary_path: &Path,
    configured_node_path: Option<&Path>,
) -> Result<Option<PathBuf>, CliResolveError> {
    let launch_requirement = parse_node_launch_requirement(binary_path);

    if matches!(launch_requirement, NodeLaunchRequirement::NotNeeded) {
        if let Some(path) = configured_node_path {
            logger::log_info(&format!(
                "[CodexWakeup][CLI] CLI 无需 Node，忽略自定义 node 路径: codex_path={}, node_path={}",
                binary_path.display(),
                path.display()
            ));
        } else {
            logger::log_info(&format!(
                "[CodexWakeup][CLI] CLI 无需额外 Node 解释器: {}",
                binary_path.display()
            ));
        }
        return Ok(None);
    }

    if let Some(path) = configured_node_path {
        let resolved = resolve_configured_binary_path(path, node_binary_candidates(), "Node.js")
            .map_err(|err| {
                logger::log_warn(&format!(
                    "[CodexWakeup][CLI] {} | codex_path={}",
                    err,
                    binary_path.display()
                ));
                CliResolveError::new(err, &[REQUIRED_RUNTIME_PATH_NODE])
            })?;
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 使用自定义 node 解释器: codex_path={}, node_path={}",
            binary_path.display(),
            resolved.display()
        ));
        return Ok(Some(resolved));
    }

    match launch_requirement {
        NodeLaunchRequirement::NotNeeded => Ok(None),
        NodeLaunchRequirement::Direct(path) => {
            if path.is_file() {
                logger::log_info(&format!(
                    "[CodexWakeup][CLI] CLI 使用 shebang 指定的 Node 解释器: codex_path={}, node_path={}",
                    binary_path.display(),
                    path.display()
                ));
                Ok(Some(path))
            } else {
                let err = format!("Codex CLI 指定的 Node.js 不存在: {}", path.display());
                logger::log_warn(&format!(
                    "[CodexWakeup][CLI] {} | codex_path={}",
                    err,
                    binary_path.display()
                ));
                Err(CliResolveError::new(err, &[REQUIRED_RUNTIME_PATH_NODE]))
            }
        }
        NodeLaunchRequirement::Search => {
            logger::log_info(&format!(
                "[CodexWakeup][CLI] CLI 需要通过 PATH 解析 Node 解释器: {}",
                binary_path.display()
            ));
            resolve_node_from_binary_path(binary_path)
                .map(|path| {
                    logger::log_info(&format!(
                        "[CodexWakeup][CLI] 已解析 Node 解释器: codex_path={}, node_path={}",
                        binary_path.display(),
                        path.display()
                    ));
                    Some(path)
                })
                .ok_or_else(|| {
                    let err = format!(
                        "Codex CLI 依赖 Node.js，但未找到可用的 node 解释器: {}",
                        binary_path.display()
                    );
                    logger::log_warn(&format!("[CodexWakeup][CLI] {}", err));
                    CliResolveError::new(err, &[REQUIRED_RUNTIME_PATH_NODE])
                })
        }
    }
}

fn build_resolved_binary(
    path: PathBuf,
    source: String,
    configured_node_path: Option<&Path>,
) -> Result<ResolvedBinary, CliResolveError> {
    let node_path = resolve_node_for_binary(&path, configured_node_path)?;
    Ok(ResolvedBinary {
        path,
        source,
        node_path,
        version: String::new(),
    })
}

fn probe_binary_version(binary: &ResolvedBinary) -> Result<String, String> {
    let mut command = build_binary_command(binary);
    command.arg("--version");
    let output = crate::modules::process_timeout::output_with_timeout(
        &mut command,
        CLI_VERSION_PROBE_TIMEOUT,
    )
    .map_err(|error| {
        format!(
            "Codex CLI 无法启动: path={}, error={}",
            binary.path.display(),
            error
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if !stderr.is_empty() { &stderr } else { &stdout };
        return Err(format!(
            "Codex CLI 版本探测失败: path={}, status={}, detail={}",
            binary.path.display(),
            output.status,
            truncate_log_text(detail, 240)
        ));
    }
    let version = if !stdout.is_empty() { stdout } else { stderr };
    if version.is_empty() {
        return Err(format!(
            "Codex CLI 版本探测未返回内容: {}",
            binary.path.display()
        ));
    }
    Ok(version)
}

fn build_usable_resolved_binary(
    path: PathBuf,
    source: String,
    configured_node_path: Option<&Path>,
) -> Result<ResolvedBinary, CliResolveError> {
    let mut binary = build_resolved_binary(path, source, configured_node_path)?;
    binary.version = probe_binary_version(&binary)
        .map_err(|error| CliResolveError::new(error, &[REQUIRED_RUNTIME_PATH_CODEX_CLI]))?;
    Ok(binary)
}

fn push_runtime_candidate_error(errors: &mut Vec<String>, source: &str, error: &str) {
    let message = format!("{}: {}", source, truncate_log_text(error, 280));
    logger::log_warn(&format!("[CodexWakeup][CLI] 跳过不可用候选: {}", message));
    errors.push(message);
}

fn resolve_binary_with_runtime_config(
    runtime_config: &CodexWakeupRuntimeConfig,
) -> Result<ResolvedBinary, CliResolveError> {
    let configured_cli_path = normalize_text(runtime_config.codex_cli_path.as_deref());
    let configured_node_path = runtime_config.node_path.as_deref().map(PathBuf::from);
    let code_cli_path = std::env::var("CODEX_CLI_PATH")
        .ok()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    logger::log_info(&format!(
        "[CodexWakeup][CLI] 开始检测 CLI: custom_codex_cli_path={}, custom_node_path={}, CODEX_CLI_PATH={}, PATH目录数={}, 搜索目录数={}",
        configured_cli_path.as_deref().unwrap_or("<unset>"),
        configured_node_path
            .as_deref()
            .map(|item| item.display().to_string())
            .unwrap_or_else(|| "<unset>".to_string()),
        code_cli_path.as_deref().unwrap_or("<unset>"),
        collect_path_dirs().len(),
        collect_runtime_search_dirs().len()
    ));

    let mut candidate_errors = Vec::new();
    let mut checked_paths = HashSet::new();

    if let Some(configured_cli_path) = configured_cli_path {
        let configured_path = PathBuf::from(&configured_cli_path);
        match resolve_configured_binary_path(&configured_path, binary_candidates(), "Codex CLI") {
            Ok(resolved) => {
                checked_paths.insert(resolved.clone());
                logger::log_info(&format!(
                    "[CodexWakeup][CLI] 命中自定义 Codex CLI 路径: input={}, resolved={}",
                    configured_path.display(),
                    resolved.display()
                ));
                match build_usable_resolved_binary(
                    resolved,
                    "runtime_config".to_string(),
                    configured_node_path.as_deref(),
                ) {
                    Ok(binary) => return Ok(binary),
                    Err(error) => push_runtime_candidate_error(
                        &mut candidate_errors,
                        "runtime_config",
                        &error.message,
                    ),
                }
            }
            Err(error) => {
                push_runtime_candidate_error(&mut candidate_errors, "runtime_config", &error)
            }
        }
    }

    if let Ok(raw) = std::env::var("CODEX_CLI_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                checked_paths.insert(path.clone());
                logger::log_info(&format!(
                    "[CodexWakeup][CLI] 命中 CODEX_CLI_PATH: {}",
                    path.display()
                ));
                match build_usable_resolved_binary(
                    path,
                    "CODEX_CLI_PATH".to_string(),
                    configured_node_path.as_deref(),
                ) {
                    Ok(binary) => return Ok(binary),
                    Err(error) => push_runtime_candidate_error(
                        &mut candidate_errors,
                        "CODEX_CLI_PATH",
                        &error.message,
                    ),
                }
            } else {
                push_runtime_candidate_error(
                    &mut candidate_errors,
                    "CODEX_CLI_PATH",
                    &format!("指向的文件不存在: {}", trimmed),
                );
            }
        }
    }

    for path in resolve_binary_paths_from_path() {
        if !checked_paths.insert(path.clone()) {
            continue;
        }
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 从 PATH 检查 codex 候选: {}",
            path.display()
        ));
        match build_usable_resolved_binary(
            path,
            "PATH".to_string(),
            configured_node_path.as_deref(),
        ) {
            Ok(binary) => return Ok(binary),
            Err(error) => {
                push_runtime_candidate_error(&mut candidate_errors, "PATH", &error.message)
            }
        }
    }

    if let Ok(path) = crate::modules::codex_official_app_server::official_app_server_executable() {
        if checked_paths.insert(path.clone()) {
            logger::log_info(&format!(
                "[CodexWakeup][CLI] 检查官方客户端内置 Codex: {}",
                path.display()
            ));
            match build_usable_resolved_binary(
                path,
                "official_app".to_string(),
                configured_node_path.as_deref(),
            ) {
                Ok(binary) => return Ok(binary),
                Err(error) => push_runtime_candidate_error(
                    &mut candidate_errors,
                    "official_app",
                    &error.message,
                ),
            }
        }
    }

    let err = if candidate_errors.is_empty() {
        "未检测到 Codex CLI，请先安装 `codex` 命令。".to_string()
    } else {
        format!(
            "未检测到可运行的 Codex CLI，已跳过不可用候选：{}",
            candidate_errors.join(" | ")
        )
    };
    logger::log_warn(&format!("[CodexWakeup][CLI] {}", err));
    Err(CliResolveError::new(
        err,
        &[REQUIRED_RUNTIME_PATH_CODEX_CLI],
    ))
}

fn resolve_binary() -> Result<ResolvedBinary, CliResolveError> {
    let runtime_config = load_runtime_config().map_err(|err| {
        CliResolveError::new(
            err,
            &[REQUIRED_RUNTIME_PATH_CODEX_CLI, REQUIRED_RUNTIME_PATH_NODE],
        )
    })?;
    resolve_binary_with_runtime_config(&runtime_config)
}

fn fetch_binary_version(binary: &ResolvedBinary) -> Option<String> {
    if binary.version.trim().is_empty() {
        None
    } else {
        Some(binary.version.clone())
    }
}

fn build_binary_command(binary: &ResolvedBinary) -> Command {
    let mut command = if let Some(node_path) = &binary.node_path {
        let mut command = Command::new(node_path);
        command.arg(&binary.path);
        command
    } else {
        Command::new(&binary.path)
    };
    process::apply_managed_proxy_env_to_command(&mut command);
    apply_hidden_window_flags(&mut command);
    command
}

pub fn get_cli_status() -> CodexCliStatus {
    let runtime_config = match load_runtime_config() {
        Ok(config) => config,
        Err(err) => {
            logger::log_warn(&format!("[CodexWakeup][CLI] 读取运行时配置失败: {}", err));
            return CodexCliStatus {
                available: false,
                binary_path: None,
                configured_codex_cli_path: None,
                configured_node_path: None,
                version: None,
                source: None,
                message: Some(err),
                required_runtime_paths: vec![REQUIRED_RUNTIME_PATH_CODEX_CLI.to_string()],
                checked_at: now_ms(),
                install_hints: install_hints(),
            };
        }
    };

    match resolve_binary_with_runtime_config(&runtime_config) {
        Ok(binary) => {
            let version = fetch_binary_version(&binary);
            logger::log_info(&format!(
                "[CodexWakeup][CLI] 检测成功: source={}, codex_path={}, node_path={}, version={}",
                binary.source,
                binary.path.display(),
                format_optional_path_for_log(binary.node_path.as_deref()),
                version.as_deref().unwrap_or("<unknown>")
            ));
            CodexCliStatus {
                available: true,
                binary_path: Some(binary.path.display().to_string()),
                configured_codex_cli_path: runtime_config.codex_cli_path.clone(),
                configured_node_path: runtime_config.node_path.clone(),
                version,
                source: Some(binary.source),
                message: None,
                required_runtime_paths: Vec::new(),
                checked_at: now_ms(),
                install_hints: install_hints(),
            }
        }
        Err(err) => {
            logger::log_warn(&format!("[CodexWakeup][CLI] 检测失败: {}", err.message));
            CodexCliStatus {
                available: false,
                binary_path: None,
                configured_codex_cli_path: runtime_config.codex_cli_path.clone(),
                configured_node_path: runtime_config.node_path.clone(),
                version: None,
                source: None,
                message: Some(err.message),
                required_runtime_paths: err.required_runtime_paths,
                checked_at: now_ms(),
                install_hints: install_hints(),
            }
        }
    }
}

pub fn resolve_cli_runtime() -> Result<CodexCliResolvedRuntime, String> {
    let binary = resolve_binary().map_err(|err| err.message)?;
    Ok(CodexCliResolvedRuntime {
        binary_path: binary.path.display().to_string(),
        node_path: binary.node_path.map(|path| path.display().to_string()),
        source: binary.source,
    })
}

pub fn run_cli_conversation_in_home_detailed(
    codex_home: &Path,
    prompt: &str,
    execution_config: &CodexWakeupExecutionConfig,
) -> Result<CodexWakeupCliConversationResult, CodexWakeupCliConversationDetailedError> {
    let runtime = get_cli_status();
    if !runtime.available {
        return Err(CodexWakeupCliConversationDetailedError {
            message: runtime
                .message
                .unwrap_or_else(|| "Codex CLI 不可用，请先配置 Codex CLI 路径。".to_string()),
            status: None,
            stdout: None,
            stderr: None,
            last_message: None,
            duration_ms: None,
        });
    }

    let binary = resolve_binary().map_err(|err| CodexWakeupCliConversationDetailedError {
        message: err.message,
        status: None,
        stdout: None,
        stderr: None,
        last_message: None,
        duration_ms: None,
    })?;
    let output = run_codex_exec_sync_detailed(&binary, codex_home, prompt, execution_config)?;
    Ok(CodexWakeupCliConversationResult {
        reply: output.reply,
        duration_ms: output.duration_ms,
    })
}

fn parse_time_to_minutes(value: &str) -> Option<i32> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hour: i32 = parts[0].parse().ok()?;
    let minute: i32 = parts[1].parse().ok()?;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return None;
    }
    Some(hour * 60 + minute)
}

fn normalize_model_preset(raw: &CodexWakeupModelPreset) -> Option<CodexWakeupModelPreset> {
    let id = raw.id.trim().to_string();
    let name = raw.name.trim().to_string();
    let model = raw.model.trim().to_string();

    if id.is_empty() || name.is_empty() || model.is_empty() {
        return None;
    }

    let mut allowed_reasoning_efforts: Vec<String> = raw
        .allowed_reasoning_efforts
        .iter()
        .filter_map(|item| normalize_reasoning_effort(item))
        .collect();
    allowed_reasoning_efforts.dedup();
    if allowed_reasoning_efforts.is_empty() {
        allowed_reasoning_efforts = default_reasoning_efforts_for_model(&model);
    }

    let default_reasoning_effort = normalize_reasoning_effort(&raw.default_reasoning_effort)
        .filter(|item| allowed_reasoning_efforts.contains(item))
        .or_else(|| {
            if allowed_reasoning_efforts
                .iter()
                .any(|item| item == REASONING_EFFORT_MEDIUM)
            {
                Some(REASONING_EFFORT_MEDIUM.to_string())
            } else {
                allowed_reasoning_efforts.first().cloned()
            }
        })
        .unwrap_or_else(|| REASONING_EFFORT_MEDIUM.to_string());

    Some(CodexWakeupModelPreset {
        id,
        name,
        model,
        allowed_reasoning_efforts,
        default_reasoning_effort,
    })
}

fn normalize_schedule(raw: &CodexWakeupSchedule) -> CodexWakeupSchedule {
    let mut weekly_days: Vec<i32> = raw
        .weekly_days
        .iter()
        .copied()
        .filter(|day| (0..=6).contains(day))
        .collect();
    weekly_days.sort_unstable();
    weekly_days.dedup();

    let normalized_kind = raw.kind.trim().to_ascii_lowercase();
    let quota_reset_window = raw
        .quota_reset_window
        .as_ref()
        .map(|item| item.trim().to_ascii_lowercase())
        .and_then(|item| match item.as_str() {
            "primary_window" => Some("primary_window".to_string()),
            "secondary_window" => Some("secondary_window".to_string()),
            "either" => Some("either".to_string()),
            _ => None,
        });
    let startup_delay_minutes = raw
        .startup_delay_minutes
        .map(|value| value.clamp(0, 24 * 60));

    CodexWakeupSchedule {
        kind: normalized_kind.clone(),
        daily_time: raw
            .daily_time
            .as_ref()
            .map(|item| item.trim().to_string())
            .filter(|item| parse_time_to_minutes(item).is_some()),
        weekly_days,
        weekly_time: raw
            .weekly_time
            .as_ref()
            .map(|item| item.trim().to_string())
            .filter(|item| parse_time_to_minutes(item).is_some()),
        interval_hours: raw.interval_hours.map(|value| value.max(1)),
        quota_reset_window: if normalized_kind == "quota_reset" {
            Some(quota_reset_window.unwrap_or_else(|| "either".to_string()))
        } else {
            None
        },
        startup_delay_minutes: if normalized_kind == "startup" {
            Some(startup_delay_minutes.unwrap_or(0))
        } else {
            None
        },
    }
}

fn existing_codex_account_id_set() -> HashSet<String> {
    codex_account::list_accounts()
        .into_iter()
        .map(|account| account.id)
        .collect()
}

/// Keep only non-empty account ids that still exist in the Codex account store.
/// Returns true when the list changed.
fn retain_existing_account_ids(account_ids: &mut Vec<String>, existing: &HashSet<String>) -> bool {
    let before = account_ids.clone();
    account_ids.retain(|account_id| {
        let trimmed = account_id.trim();
        !trimmed.is_empty() && existing.contains(trimmed)
    });
    if account_ids != &before {
        return true;
    }
    false
}

/// Drop deleted/missing account ids from every task. Tasks that become empty are removed.
/// Returns true when state changed.
fn prune_missing_accounts_from_state(
    state: &mut CodexWakeupState,
    existing: &HashSet<String>,
) -> bool {
    let mut changed = false;
    let mut kept = Vec::with_capacity(state.tasks.len());
    for mut task in state.tasks.drain(..) {
        if retain_existing_account_ids(&mut task.account_ids, existing) {
            changed = true;
            task.updated_at = now_ts();
        }
        if task.account_ids.is_empty() {
            changed = true;
            continue;
        }
        kept.push(task);
    }
    state.tasks = kept;
    changed
}

/// Remove deleted account ids from persisted Codex wakeup tasks.
/// Called when Codex accounts are deleted so task snapshots stay in sync.
pub fn remove_deleted_accounts_from_tasks(account_ids: &[String]) -> Result<(), String> {
    let remove_ids: HashSet<String> = account_ids
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if remove_ids.is_empty() {
        return Ok(());
    }

    let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
    let path = tasks_path()?;
    if !path.exists() {
        return Ok(());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex 唤醒任务失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(());
    }
    let mut state: CodexWakeupState = match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(error) => {
            quarantine_corrupted_wakeup_file(&path, "任务配置", &error);
            return Ok(());
        }
    };

    let mut changed = false;
    let mut kept = Vec::with_capacity(state.tasks.len());
    for mut task in state.tasks.drain(..) {
        let before_len = task.account_ids.len();
        task.account_ids
            .retain(|account_id| !remove_ids.contains(account_id.trim()));
        if task.account_ids.len() != before_len {
            changed = true;
            task.updated_at = now_ts();
        }
        if task.account_ids.is_empty() {
            changed = true;
            continue;
        }
        kept.push(task);
    }
    if !changed {
        return Ok(());
    }

    state.tasks = kept;
    // Normalize + refresh schedules before writing, without re-entering load_state locks.
    state.tasks = state.tasks.iter().map(normalize_task).collect();
    state.tasks.retain(|task| !task.account_ids.is_empty());
    let existing = existing_codex_account_id_set();
    let _ = prune_missing_accounts_from_state(&mut state, &existing);
    refresh_next_run_at(&mut state);
    save_json_atomic(&path, &state)?;
    logger::log_info(&format!(
        "[CodexWakeup] 已从唤醒任务中移除已删除账号引用: removed={}, remaining_tasks={}",
        remove_ids.len(),
        state.tasks.len()
    ));
    Ok(())
}

fn normalize_task(raw: &CodexWakeupTask) -> CodexWakeupTask {
    let now = now_ts();
    let mut account_ids: Vec<String> = raw
        .account_ids
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    account_ids.sort();
    account_ids.dedup();

    let name = raw.name.trim();
    let prompt = raw
        .prompt
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let model = raw
        .model
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let model_display_name = raw
        .model_display_name
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let model_reasoning_effort = raw
        .model_reasoning_effort
        .as_ref()
        .and_then(|item| normalize_reasoning_effort(item));
    let schedule = normalize_schedule(&raw.schedule);

    CodexWakeupTask {
        id: raw.id.trim().to_string(),
        name: if name.is_empty() {
            "Codex Wakeup Task".to_string()
        } else {
            name.to_string()
        },
        enabled: raw.enabled,
        account_ids,
        prompt,
        model,
        model_display_name,
        model_reasoning_effort,
        schedule,
        created_at: if raw.created_at > 0 {
            raw.created_at
        } else {
            now
        },
        updated_at: if raw.updated_at > 0 {
            raw.updated_at
        } else {
            now
        },
        last_run_at: raw.last_run_at,
        last_status: raw.last_status.clone(),
        last_message: raw.last_message.clone(),
        last_success_count: raw.last_success_count,
        last_failure_count: raw.last_failure_count,
        last_duration_ms: raw.last_duration_ms,
        next_run_at: raw.next_run_at,
        execution_mode: raw
            .execution_mode
            .as_ref()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty()),
        confirm_timeout_minutes: raw.confirm_timeout_minutes.map(|value| value.max(1)),
    }
}

fn refresh_next_run_at(state: &mut CodexWakeupState) {
    for task in &mut state.tasks {
        task.next_run_at = if state.enabled && task.enabled {
            crate::modules::codex_wakeup_scheduler::calculate_next_run_at(task)
        } else {
            None
        };
    }
}

fn save_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    let content =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化 JSON 失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
}

pub fn load_runtime_config() -> Result<CodexWakeupRuntimeConfig, String> {
    let path = runtime_config_path()?;
    if !path.exists() {
        return Ok(CodexWakeupRuntimeConfig::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex 唤醒运行时配置失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(CodexWakeupRuntimeConfig::default());
    }
    let raw: CodexWakeupRuntimeConfig = match serde_json::from_str(&content) {
        Ok(raw) => raw,
        Err(error) => {
            quarantine_corrupted_wakeup_file(&path, "运行时配置", &error);
            return Ok(CodexWakeupRuntimeConfig::default());
        }
    };
    Ok(normalize_runtime_config(&raw))
}

pub fn save_runtime_config(
    next_config: &CodexWakeupRuntimeConfig,
) -> Result<CodexWakeupRuntimeConfig, String> {
    let normalized = normalize_runtime_config(next_config);
    save_json_atomic(&runtime_config_path()?, &normalized)?;
    Ok(normalized)
}

fn load_state_inner() -> Result<CodexWakeupState, String> {
    let path = tasks_path()?;
    if !path.exists() {
        return Ok(CodexWakeupState::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex 唤醒任务失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(CodexWakeupState::default());
    }
    let mut state: CodexWakeupState = match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(error) => {
            quarantine_corrupted_wakeup_file(&path, "任务配置", &error);
            return Ok(CodexWakeupState::default());
        }
    };
    state.tasks = state.tasks.iter().map(normalize_task).collect();
    let mut preset_ids = HashSet::new();
    state.model_presets = state
        .model_presets
        .iter()
        .filter_map(normalize_model_preset)
        .filter(|preset| preset_ids.insert(preset.id.clone()))
        .collect();
    state.model_preset_migrations = state
        .model_preset_migrations
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    state.model_preset_migrations.sort();
    state.model_preset_migrations.dedup();
    let migration_changed = apply_model_preset_migrations(&mut state);
    let existing_accounts = existing_codex_account_id_set();
    let account_prune_changed = prune_missing_accounts_from_state(&mut state, &existing_accounts);
    refresh_next_run_at(&mut state);
    if migration_changed || account_prune_changed {
        let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
        save_json_atomic(&path, &state)?;
    }
    Ok(state)
}

pub fn load_state() -> Result<CodexWakeupState, String> {
    load_state_inner()
}

pub fn load_state_for_scheduler() -> Result<CodexWakeupState, String> {
    load_state_inner()
}

pub fn load_overview() -> Result<CodexWakeupOverview, String> {
    let runtime = wakeup_runtime_status();
    let state = load_state()?;
    let history = load_history()?;
    Ok(CodexWakeupOverview {
        runtime,
        state,
        history,
    })
}

pub fn save_state(next_state: &CodexWakeupState) -> Result<CodexWakeupState, String> {
    let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
    let mut seen = HashSet::new();
    let mut preset_seen = HashSet::new();
    let mut state = CodexWakeupState {
        enabled: next_state.enabled,
        tasks: next_state
            .tasks
            .iter()
            .map(normalize_task)
            .filter(|task| {
                !task.id.is_empty() && !task.account_ids.is_empty() && seen.insert(task.id.clone())
            })
            .collect(),
        model_presets: next_state
            .model_presets
            .iter()
            .filter_map(normalize_model_preset)
            .filter(|preset| preset_seen.insert(preset.id.clone()))
            .collect(),
        model_preset_migrations: next_state
            .model_preset_migrations
            .iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
    };
    state.model_preset_migrations.sort();
    state.model_preset_migrations.dedup();
    apply_model_preset_migrations(&mut state);
    let existing_accounts = existing_codex_account_id_set();
    let _ = prune_missing_accounts_from_state(&mut state, &existing_accounts);

    refresh_next_run_at(&mut state);

    save_json_atomic(&tasks_path()?, &state)?;
    Ok(state)
}

pub fn load_history() -> Result<Vec<CodexWakeupHistoryItem>, String> {
    let path = history_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex 唤醒历史失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str(&content) {
        Ok(history) => Ok(history),
        Err(error) => {
            quarantine_corrupted_wakeup_file(&path, "历史记录", &error);
            Ok(Vec::new())
        }
    }
}

pub fn add_history_items(new_items: Vec<CodexWakeupHistoryItem>) -> Result<(), String> {
    if new_items.is_empty() {
        return Ok(());
    }
    let _lock = HISTORY_LOCK
        .lock()
        .map_err(|_| "获取 Codex 唤醒历史锁失败")?;
    let mut existing = load_history().unwrap_or_default();
    let existing_ids: HashSet<String> = existing.iter().map(|item| item.id.clone()).collect();
    let mut merged: Vec<CodexWakeupHistoryItem> = new_items
        .into_iter()
        .filter(|item| !existing_ids.contains(&item.id))
        .collect();
    merged.append(&mut existing);
    merged.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    merged.truncate(MAX_HISTORY_ITEMS);
    save_json_atomic(&history_path()?, &merged)
}

pub fn clear_history() -> Result<(), String> {
    let _lock = HISTORY_LOCK
        .lock()
        .map_err(|_| "获取 Codex 唤醒历史锁失败")?;
    save_json_atomic(&history_path()?, &Vec::<CodexWakeupHistoryItem>::new())
}

#[cfg(target_os = "windows")]
fn apply_hidden_window_flags(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(target_os = "windows"))]
fn apply_hidden_window_flags(_command: &mut Command) {}

fn run_command_with_cancel(
    command: &mut Command,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> Result<ExitStatus, String> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 Codex CLI 失败: {}", e))?;

    loop {
        if is_scope_cancelled(cancel_flag) {
            if let Err(err) = child.kill() {
                if err.kind() != io::ErrorKind::InvalidInput {
                    logger::log_warn(&format!(
                        "[CodexWakeup][CLI] 取消测试时终止子进程失败: {}",
                        err
                    ));
                }
            }
            let _ = child.wait();
            return Err(cancelled_error());
        }

        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {
                std::thread::sleep(std::time::Duration::from_millis(
                    CODEX_WAKEUP_CANCEL_POLL_MS,
                ));
            }
            Err(err) => return Err(format!("等待 Codex CLI 进程状态失败: {}", err)),
        }
    }
}

fn run_codex_exec_sync(
    binary: &ResolvedBinary,
    codex_home: &Path,
    prompt: &str,
    execution_config: &CodexWakeupExecutionConfig,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> Result<CommandOutput, String> {
    if is_scope_cancelled(cancel_flag) {
        return Err(cancelled_error());
    }
    crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &codex_home.join("config.toml"),
    )?;
    let workspace_dir = codex_home.join("workspace");
    fs::create_dir_all(&workspace_dir).map_err(|e| format!("创建唤醒工作目录失败: {}", e))?;
    let last_message_path = codex_home.join("last_message.txt");

    let started = std::time::Instant::now();
    logger::log_info(&format!(
        "[CodexWakeup][CLI] 开始执行唤醒命令: codex_path={}, node_path={}, codex_home={}, workspace_dir={}, prompt_chars={}, model={}, reasoning_effort={}",
        binary.path.display(),
        format_optional_path_for_log(binary.node_path.as_deref()),
        codex_home.display(),
        workspace_dir.display(),
        prompt.chars().count(),
        execution_config
            .model
            .as_deref()
            .unwrap_or("<default>"),
        execution_config
            .model_reasoning_effort
            .as_deref()
            .unwrap_or("<default>")
    ));
    let mut command = build_binary_command(&binary);
    command
        .env("CODEX_HOME", codex_home)
        .arg("exec")
        .arg("--skip-git-repo-check")
        .arg("--color")
        .arg("never")
        .arg("--output-last-message")
        .arg(&last_message_path)
        .arg("-C")
        .arg(&workspace_dir);

    if let Some(model) = execution_config.model.as_deref() {
        command
            .arg("-c")
            .arg(format!(r#"model="{}""#, escape_toml_basic_string(model)));
    }
    if let Some(reasoning_effort) = execution_config.model_reasoning_effort.as_deref() {
        command.arg("-c").arg(format!(
            r#"model_reasoning_effort="{}""#,
            escape_toml_basic_string(reasoning_effort)
        ));
    }
    command.arg(prompt);

    let status = run_command_with_cancel(&mut command, cancel_flag)?;
    let duration_ms = started.elapsed().as_millis().max(0) as u64;

    let reply = fs::read_to_string(&last_message_path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());

    if status.success() {
        let reply = reply.unwrap_or_else(|| "Codex CLI 已完成，但未返回可读消息。".to_string());
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 唤醒命令执行成功: duration_ms={}, reply_chars={}",
            duration_ms,
            reply.chars().count()
        ));
        return Ok(CommandOutput { reply, duration_ms });
    }

    logger::log_warn(&format!(
        "[CodexWakeup][CLI] 唤醒命令执行失败: status={}",
        status,
    ));
    let message = format!("Codex CLI 退出失败: {}", status);
    Err(message)
}

fn clean_cli_output_text(value: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(value).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(truncate_log_text(&text, 4000))
    }
}

fn run_codex_exec_sync_detailed(
    binary: &ResolvedBinary,
    codex_home: &Path,
    prompt: &str,
    execution_config: &CodexWakeupExecutionConfig,
) -> Result<CommandOutput, CodexWakeupCliConversationDetailedError> {
    crate::modules::codex_config_format::sanitize_codex_config_toml_file(
        &codex_home.join("config.toml"),
    )
    .map_err(|message| CodexWakeupCliConversationDetailedError {
        message,
        status: None,
        stdout: None,
        stderr: None,
        last_message: None,
        duration_ms: None,
    })?;
    let workspace_dir = codex_home.join("workspace");
    fs::create_dir_all(&workspace_dir).map_err(|e| CodexWakeupCliConversationDetailedError {
        message: format!("创建唤醒工作目录失败: {}", e),
        status: None,
        stdout: None,
        stderr: None,
        last_message: None,
        duration_ms: None,
    })?;
    let last_message_path = codex_home.join("last_message.txt");

    let started = std::time::Instant::now();
    logger::log_info(&format!(
        "[CodexWakeup][CLI] 开始执行诊断命令: codex_path={}, node_path={}, codex_home={}, workspace_dir={}, prompt_chars={}, model={}, reasoning_effort={}",
        binary.path.display(),
        format_optional_path_for_log(binary.node_path.as_deref()),
        codex_home.display(),
        workspace_dir.display(),
        prompt.chars().count(),
        execution_config
            .model
            .as_deref()
            .unwrap_or("<default>"),
        execution_config
            .model_reasoning_effort
            .as_deref()
            .unwrap_or("<default>")
    ));
    let mut command = build_binary_command(&binary);
    command
        .env("CODEX_HOME", codex_home)
        .arg("exec")
        .arg("--skip-git-repo-check")
        .arg("--color")
        .arg("never")
        .arg("--output-last-message")
        .arg(&last_message_path)
        .arg("-C")
        .arg(&workspace_dir);

    if let Some(model) = execution_config.model.as_deref() {
        command
            .arg("-c")
            .arg(format!(r#"model="{}""#, escape_toml_basic_string(model)));
    }
    if let Some(reasoning_effort) = execution_config.model_reasoning_effort.as_deref() {
        command.arg("-c").arg(format!(
            r#"model_reasoning_effort="{}""#,
            escape_toml_basic_string(reasoning_effort)
        ));
    }
    command.arg(prompt);

    let output = command
        .output()
        .map_err(|e| CodexWakeupCliConversationDetailedError {
            message: format!("启动 Codex CLI 失败: {}", e),
            status: None,
            stdout: None,
            stderr: None,
            last_message: None,
            duration_ms: None,
        })?;
    let duration_ms = started.elapsed().as_millis().max(0) as u64;
    let stdout = clean_cli_output_text(&output.stdout);
    let stderr = clean_cli_output_text(&output.stderr);
    let last_message = fs::read_to_string(&last_message_path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .map(|text| truncate_log_text(&text, 4000));

    if output.status.success() {
        let reply = last_message
            .clone()
            .or_else(|| stdout.clone())
            .unwrap_or_else(|| "Codex CLI 已完成，但未返回可读消息。".to_string());
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 诊断命令执行成功: duration_ms={}, reply_chars={}",
            duration_ms,
            reply.chars().count()
        ));
        return Ok(CommandOutput { reply, duration_ms });
    }

    logger::log_warn(&format!(
        "[CodexWakeup][CLI] 诊断命令执行失败: status={}, stdout={}, stderr={}",
        output.status,
        stdout.as_deref().unwrap_or("-"),
        stderr.as_deref().unwrap_or("-")
    ));
    Err(CodexWakeupCliConversationDetailedError {
        message: format!("Codex CLI 退出失败: {}", output.status),
        status: Some(output.status.to_string()),
        stdout,
        stderr,
        last_message,
        duration_ms: Some(duration_ms),
    })
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn create_failure_record(
    run_id: &str,
    trigger_type: &str,
    task_id: Option<&str>,
    task_name: Option<&str>,
    account_id: &str,
    account_email: String,
    account_context_text: Option<String>,
    prompt: Option<String>,
    execution_config: &CodexWakeupExecutionConfig,
    error: String,
    cli_path: Option<String>,
) -> CodexWakeupHistoryItem {
    CodexWakeupHistoryItem {
        id: uuid::Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        timestamp: now_ms(),
        trigger_type: trigger_type.to_string(),
        task_id: task_id.map(|item| item.to_string()),
        task_name: task_name.map(|item| item.to_string()),
        account_id: account_id.to_string(),
        account_email,
        account_context_text,
        success: false,
        prompt,
        model: execution_config.model.clone(),
        model_display_name: execution_config.model_display_name.clone(),
        model_reasoning_effort: execution_config.model_reasoning_effort.clone(),
        reply: None,
        error: Some(error),
        quota_refresh_error: None,
        duration_ms: None,
        cli_path,
        quota_before: None,
        quota_after: None,
    }
}

fn emit_progress(
    app: Option<&AppHandle>,
    run_id: &str,
    context: &TaskRunContext,
    total: usize,
    completed: usize,
    success_count: usize,
    failure_count: usize,
    running: bool,
    phase: &str,
    current_account_id: Option<&str>,
    item: Option<CodexWakeupHistoryItem>,
) {
    let Some(app) = app else {
        return;
    };

    let payload = CodexWakeupProgressPayload {
        run_id: run_id.to_string(),
        trigger_type: context.trigger_type.clone(),
        task_id: context.task_id.clone(),
        task_name: context.task_name.clone(),
        total,
        completed,
        success_count,
        failure_count,
        running,
        phase: phase.to_string(),
        current_account_id: current_account_id.map(|value| value.to_string()),
        item,
    };
    let _ = app.emit(PROGRESS_EVENT, payload);
}

fn create_cli_missing_record(
    run_id: &str,
    context: &TaskRunContext,
    account_id: &str,
    prompt: Option<String>,
    execution_config: &CodexWakeupExecutionConfig,
) -> CodexWakeupHistoryItem {
    let existing = codex_account::load_account(account_id);
    let account_email = existing
        .as_ref()
        .map(|account| account.email.clone())
        .unwrap_or_else(|| account_id.to_string());
    let account_context_text = existing.as_ref().and_then(resolve_account_context_text);

    create_failure_record(
        run_id,
        &context.trigger_type,
        context.task_id.as_deref(),
        context.task_name.as_deref(),
        account_id,
        account_email,
        account_context_text,
        prompt,
        execution_config,
        "未检测到 Codex CLI，请先安装后再执行唤醒。".to_string(),
        None,
    )
}

fn create_cancelled_record(
    run_id: &str,
    context: &TaskRunContext,
    account_id: &str,
    prompt: Option<String>,
    execution_config: &CodexWakeupExecutionConfig,
    cli_path: Option<String>,
) -> CodexWakeupHistoryItem {
    let existing = codex_account::load_account(account_id);
    let account_email = existing
        .as_ref()
        .map(|account| account.email.clone())
        .unwrap_or_else(|| account_id.to_string());
    let account_context_text = existing.as_ref().and_then(resolve_account_context_text);

    create_failure_record(
        run_id,
        &context.trigger_type,
        context.task_id.as_deref(),
        context.task_name.as_deref(),
        account_id,
        account_email,
        account_context_text,
        prompt,
        execution_config,
        cancelled_error(),
        cli_path,
    )
}

async fn run_single_account(
    run_id: &str,
    context: &TaskRunContext,
    account_id: &str,
    prompt: &str,
    execution_config: &CodexWakeupExecutionConfig,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> CodexWakeupHistoryItem {
    let prompt_value = Some(prompt.to_string());
    if is_scope_cancelled(cancel_flag) {
        return create_cancelled_record(
            run_id,
            context,
            account_id,
            prompt_value,
            execution_config,
            None,
        );
    }

    let existing = match codex_account::load_account(account_id) {
        Some(account) => account,
        None => {
            return create_failure_record(
                run_id,
                &context.trigger_type,
                context.task_id.as_deref(),
                context.task_name.as_deref(),
                account_id,
                account_id.to_string(),
                None,
                prompt_value,
                execution_config,
                "账号不存在".to_string(),
                None,
            )
        }
    };
    let existing_context_text = resolve_account_context_text(&existing);

    if existing.is_api_key_auth() {
        return create_failure_record(
            run_id,
            &context.trigger_type,
            context.task_id.as_deref(),
            context.task_name.as_deref(),
            account_id,
            existing.email,
            existing_context_text,
            prompt_value,
            execution_config,
            "Codex 官方直连唤醒仅支持 OAuth 账号。".to_string(),
            None,
        );
    }

    let started_at = std::time::Instant::now();
    match codex_local_access::run_official_wakeup_chat(
        account_id,
        execution_config.model.as_deref(),
        execution_config.model_reasoning_effort.as_deref(),
        prompt,
    )
    .await
    {
        Ok(output) => {
            if is_scope_cancelled(cancel_flag) {
                return create_cancelled_record(
                    run_id,
                    context,
                    account_id,
                    prompt_value,
                    execution_config,
                    None,
                );
            }
            let account_context_text = resolve_account_context_text(&output.account);
            let account_email = output.account.email;
            CodexWakeupHistoryItem {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run_id.to_string(),
                timestamp: now_ms(),
                trigger_type: context.trigger_type.clone(),
                task_id: context.task_id.clone(),
                task_name: context.task_name.clone(),
                account_id: account_id.to_string(),
                account_email,
                account_context_text,
                success: true,
                prompt: prompt_value,
                model: execution_config.model.clone(),
                model_display_name: execution_config.model_display_name.clone(),
                model_reasoning_effort: execution_config.model_reasoning_effort.clone(),
                reply: Some(output.reply),
                error: None,
                quota_refresh_error: None,
                duration_ms: Some(output.duration_ms),
                cli_path: None,
                quota_before: None,
                quota_after: None,
            }
        }
        Err(err) => {
            let mut record = create_failure_record(
                run_id,
                &context.trigger_type,
                context.task_id.as_deref(),
                context.task_name.as_deref(),
                account_id,
                existing.email,
                existing_context_text,
                prompt_value,
                execution_config,
                err,
                None,
            );
            record.duration_ms = Some(started_at.elapsed().as_millis() as u64);
            record
        }
    }
}

pub async fn run_batch(
    app: Option<&AppHandle>,
    account_ids: Vec<String>,
    prompt: Option<String>,
    execution_config: CodexWakeupExecutionConfig,
    context: TaskRunContext,
    run_id: Option<String>,
    cancel_scope_id: Option<&str>,
) -> Result<CodexWakeupBatchResult, String> {
    let existing_accounts = existing_codex_account_id_set();
    let cleaned_ids: Vec<String> = account_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .filter(|item| existing_accounts.contains(item))
        .collect();
    if cleaned_ids.is_empty() {
        return Err("至少选择一个有效账号（已删除的账号不会被执行）".to_string());
    }

    let prompt = prompt
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let total = cleaned_ids.len();
    let runtime = wakeup_runtime_status();
    let run_id = run_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let cancel_flag = resolve_cancel_flag(cancel_scope_id)?;
    emit_progress(
        app,
        &run_id,
        &context,
        total,
        0,
        0,
        0,
        true,
        "batch_started",
        None,
        None,
    );
    let mut records = Vec::with_capacity(cleaned_ids.len());
    let mut success_count = 0usize;
    let mut failure_count = 0usize;

    for (index, account_id) in cleaned_ids.into_iter().enumerate() {
        if is_scope_cancelled(cancel_flag.as_ref()) {
            let record = create_cancelled_record(
                &run_id,
                &context,
                &account_id,
                Some(prompt.clone()),
                &execution_config,
                None,
            );
            failure_count += 1;
            emit_progress(
                app,
                &run_id,
                &context,
                total,
                index + 1,
                success_count,
                failure_count,
                index + 1 < total,
                "account_completed",
                Some(&account_id),
                Some(record.clone()),
            );
            records.push(record);
            continue;
        }

        emit_progress(
            app,
            &run_id,
            &context,
            total,
            index,
            success_count,
            failure_count,
            true,
            "account_started",
            Some(&account_id),
            None,
        );
        let record = run_single_account(
            &run_id,
            &context,
            &account_id,
            &prompt,
            &execution_config,
            cancel_flag.as_ref(),
        )
        .await;
        if record.success {
            success_count += 1;
        } else {
            failure_count += 1;
        }
        emit_progress(
            app,
            &run_id,
            &context,
            total,
            index + 1,
            success_count,
            failure_count,
            index + 1 < total,
            "account_completed",
            Some(&account_id),
            Some(record.clone()),
        );
        records.push(record);
    }

    add_history_items(records.clone())?;
    emit_progress(
        app,
        &run_id,
        &context,
        records.len(),
        records.len(),
        success_count,
        failure_count,
        false,
        "batch_completed",
        None,
        None,
    );

    Ok(CodexWakeupBatchResult {
        run_id,
        runtime,
        records,
        success_count,
        failure_count,
    })
}

fn summarize_task_result(
    records: &[CodexWakeupHistoryItem],
) -> (Option<String>, Option<u64>, Option<i64>) {
    let latest_ts = records.iter().map(|item| item.timestamp).max();
    let total_duration = records
        .iter()
        .filter_map(|item| item.duration_ms)
        .sum::<u64>();

    (
        None,
        if records.is_empty() {
            None
        } else {
            Some(total_duration)
        },
        latest_ts,
    )
}

pub fn update_task_after_run(
    task_id: &str,
    records: &[CodexWakeupHistoryItem],
) -> Result<(), String> {
    let mut state = load_state()?;
    let Some(task) = state.tasks.iter_mut().find(|item| item.id == task_id) else {
        return Ok(());
    };

    let all_success = !records.is_empty() && records.iter().all(|item| item.success);
    let success_count = records.iter().filter(|item| item.success).count() as u32;
    let failure_count = records.len().saturating_sub(success_count as usize) as u32;
    let (summary_message, total_duration, _) = summarize_task_result(records);
    task.last_run_at = Some(now_ts());
    task.last_status = Some(if all_success { "success" } else { "error" }.to_string());
    task.last_message = summary_message;
    task.last_success_count = if records.is_empty() {
        None
    } else {
        Some(success_count)
    };
    task.last_failure_count = if records.is_empty() {
        None
    } else {
        Some(failure_count)
    };
    task.last_duration_ms = total_duration;
    task.updated_at = now_ts();
    task.next_run_at = crate::modules::codex_wakeup_scheduler::calculate_next_run_at(task);
    save_state(&state)?;
    Ok(())
}

pub fn get_task(task_id: &str) -> Result<Option<CodexWakeupTask>, String> {
    Ok(load_state()?
        .tasks
        .into_iter()
        .find(|item| item.id == task_id))
}

#[cfg(test)]
mod tests {
    use super::{
        append_version_manager_cli_dirs, apply_model_preset_migrations,
        build_usable_resolved_binary, default_model_presets, prune_missing_accounts_from_state,
        retain_existing_account_ids, CodexWakeupModelPreset, CodexWakeupSchedule, CodexWakeupState,
        CodexWakeupTask, GPT_5_5_MODEL_PRESET_MIGRATION_ID, GPT_5_6_MODEL_PRESETS_MIGRATION_ID,
        PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID, REASONING_EFFORT_MEDIUM,
    };
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn model_preset(id: &str, name: &str, model: &str) -> CodexWakeupModelPreset {
        CodexWakeupModelPreset {
            id: id.to_string(),
            name: name.to_string(),
            model: model.to_string(),
            allowed_reasoning_efforts: vec![REASONING_EFFORT_MEDIUM.to_string()],
            default_reasoning_effort: REASONING_EFFORT_MEDIUM.to_string(),
        }
    }

    fn sample_task(id: &str, account_ids: &[&str]) -> CodexWakeupTask {
        CodexWakeupTask {
            id: id.to_string(),
            name: id.to_string(),
            enabled: true,
            account_ids: account_ids.iter().map(|item| item.to_string()).collect(),
            prompt: None,
            model: None,
            model_display_name: None,
            model_reasoning_effort: None,
            schedule: CodexWakeupSchedule {
                kind: "daily".to_string(),
                daily_time: Some("09:00".to_string()),
                weekly_days: Vec::new(),
                weekly_time: None,
                interval_hours: None,
                quota_reset_window: None,
                startup_delay_minutes: None,
            },
            created_at: 1,
            updated_at: 1,
            last_run_at: None,
            last_status: None,
            last_message: None,
            last_success_count: None,
            last_failure_count: None,
            last_duration_ms: None,
            next_run_at: None,
            execution_mode: None,
            confirm_timeout_minutes: None,
        }
    }

    #[test]
    fn retain_existing_account_ids_drops_missing() {
        let existing = HashSet::from(["keep-a".to_string(), "keep-b".to_string()]);
        let mut account_ids = vec![
            "keep-a".to_string(),
            "deleted".to_string(),
            "keep-b".to_string(),
            "".to_string(),
        ];
        assert!(retain_existing_account_ids(&mut account_ids, &existing));
        assert_eq!(
            account_ids,
            vec!["keep-a".to_string(), "keep-b".to_string()]
        );
        assert!(!retain_existing_account_ids(&mut account_ids, &existing));
    }

    #[test]
    fn prune_missing_accounts_removes_stale_ids_and_empty_tasks() {
        let mut state = CodexWakeupState {
            enabled: true,
            tasks: vec![
                sample_task("task-keep", &["acc-1", "acc-gone"]),
                sample_task("task-empty", &["acc-gone"]),
            ],
            model_presets: Vec::new(),
            model_preset_migrations: Vec::new(),
        };
        let existing = HashSet::from(["acc-1".to_string()]);

        assert!(prune_missing_accounts_from_state(&mut state, &existing));
        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.tasks[0].id, "task-keep");
        assert_eq!(state.tasks[0].account_ids, vec!["acc-1".to_string()]);
    }

    #[test]
    fn default_model_presets_include_gpt_5_6_models() {
        let models: Vec<String> = default_model_presets()
            .into_iter()
            .map(|preset| preset.model)
            .collect();

        assert_eq!(
            models,
            vec![
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini"
            ]
        );
    }

    #[test]
    fn model_preset_migration_prunes_legacy_codex_defaults() {
        let mut state = CodexWakeupState {
            enabled: false,
            tasks: Vec::new(),
            model_presets: vec![
                model_preset("preset-gpt-5-5", "GPT-5.5", "gpt-5.5"),
                model_preset("preset-gpt-5-codex", "GPT-5 Codex", "gpt-5-codex"),
                model_preset("preset-gpt-5-3-codex", "GPT-5.3-Codex", "gpt-5.3-codex"),
                model_preset("custom-gpt-5-4", "Custom GPT-5.4", "gpt-5.4"),
            ],
            model_preset_migrations: Vec::new(),
        };

        assert!(apply_model_preset_migrations(&mut state));
        let models: Vec<String> = state
            .model_presets
            .iter()
            .map(|preset| preset.model.clone())
            .collect();

        assert_eq!(
            models,
            vec![
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
                "gpt-5.4"
            ]
        );
        assert!(state
            .model_preset_migrations
            .iter()
            .any(|item| item == PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID));
        assert!(state
            .model_preset_migrations
            .iter()
            .any(|item| item == GPT_5_6_MODEL_PRESETS_MIGRATION_ID));
    }

    #[test]
    fn model_preset_migration_keeps_existing_5_6_and_custom_presets() {
        let mut state = CodexWakeupState {
            enabled: false,
            tasks: Vec::new(),
            model_presets: vec![
                model_preset("custom-sol", "Custom Sol", "gpt-5.6-sol"),
                model_preset("custom-model", "Custom Model", "custom-model"),
            ],
            model_preset_migrations: vec![
                GPT_5_5_MODEL_PRESET_MIGRATION_ID.to_string(),
                PRUNE_LEGACY_MODEL_PRESETS_MIGRATION_ID.to_string(),
            ],
        };

        assert!(apply_model_preset_migrations(&mut state));
        let models = state
            .model_presets
            .iter()
            .map(|preset| preset.model.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            models,
            vec![
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.6-sol",
                "custom-model"
            ]
        );
        assert!(!apply_model_preset_migrations(&mut state));
    }

    #[test]
    fn version_manager_cli_dirs_include_nvm_node_bins() {
        let root = std::env::temp_dir().join(format!(
            "codex-wakeup-nvm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        let nvm_bin = root.join(".nvm/versions/node/v20.19.2/bin");
        fs::create_dir_all(&nvm_bin).expect("create nvm bin");
        fs::write(nvm_bin.join("codex"), "#!/bin/sh\n").expect("write codex");
        fs::write(nvm_bin.join("node"), "#!/bin/sh\n").expect("write node");

        let mut dirs: Vec<PathBuf> = Vec::new();
        append_version_manager_cli_dirs(&mut dirs, &root);

        assert!(
            dirs.iter().any(|dir| dir == &nvm_bin),
            "expected nvm bin dir in search paths: {:?}",
            dirs
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn runtime_candidate_probe_rejects_existing_but_unusable_launcher() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "codex-wakeup-probe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create probe root");

        let broken = root.join("broken-codex");
        fs::write(
            &broken,
            "#!/bin/sh\necho missing-native-binary >&2\nexit 1\n",
        )
        .expect("write broken launcher");
        fs::set_permissions(&broken, fs::Permissions::from_mode(0o755))
            .expect("make broken launcher executable");

        let error = build_usable_resolved_binary(broken, "test".to_string(), None)
            .expect_err("an existing launcher that cannot run must be rejected");
        assert!(error.message.contains("missing-native-binary"));

        let working = root.join("working-codex");
        fs::write(&working, "#!/bin/sh\necho codex-cli-test 1.0\n")
            .expect("write working launcher");
        fs::set_permissions(&working, fs::Permissions::from_mode(0o755))
            .expect("make working launcher executable");

        let resolved = build_usable_resolved_binary(working, "test".to_string(), None)
            .expect("a runnable launcher should be accepted");
        assert_eq!(resolved.version, "codex-cli-test 1.0");

        let _ = fs::remove_dir_all(&root);
    }
}
