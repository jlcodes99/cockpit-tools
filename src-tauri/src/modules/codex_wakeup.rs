use crate::modules::{account, codex_account, logger};
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
use tauri::{AppHandle, Emitter};

const TASKS_FILE: &str = "codex_wakeup_tasks.json";
const HISTORY_FILE: &str = "codex_wakeup_history.json";
const RUNTIME_CONFIG_FILE: &str = "codex_wakeup_runtime_config.json";
const MANAGED_HOMES_DIR: &str = "codex_wakeup_homes";
const MAX_HISTORY_ITEMS: usize = 300;
const MAX_LOGGED_SEARCH_DIRS: usize = 8;
const REQUIRED_RUNTIME_PATH_CODEX_CLI: &str = "codex_cli_path";
const REQUIRED_RUNTIME_PATH_NODE: &str = "node_path";
pub const DEFAULT_PROMPT: &str = "hi";
pub const PROGRESS_EVENT: &str = "codex://wakeup-progress";
const REASONING_EFFORT_LOW: &str = "low";
const REASONING_EFFORT_MEDIUM: &str = "medium";
const REASONING_EFFORT_HIGH: &str = "high";
const REASONING_EFFORT_XHIGH: &str = "xhigh";
const CODEX_WAKEUP_TEST_CANCELLED_MESSAGE: &str = "Codex 唤醒测试已取消";
const CODEX_WAKEUP_CANCEL_POLL_MS: u64 = 120;
const GPT_5_5_MODEL_PRESET_MIGRATION_ID: &str = "add-gpt-5-5-model-preset";

static TASKS_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));
static HISTORY_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));
static TEST_CANCEL_SCOPES: std::sync::LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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
            model_preset_migrations: vec![GPT_5_5_MODEL_PRESET_MIGRATION_ID.to_string()],
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
        ("preset-gpt-5-5", "GPT-5.5", "gpt-5.5"),
        ("preset-gpt-5-4", "GPT-5.4", "gpt-5.4"),
        ("preset-gpt-5-4-mini", "GPT-5.4-Mini", "gpt-5.4-mini"),
        ("preset-gpt-5-3-codex", "GPT-5.3-Codex", "gpt-5.3-codex"),
        ("preset-gpt-5-2-codex", "GPT-5.2-Codex", "gpt-5.2-codex"),
        ("preset-gpt-5-2", "GPT-5.2", "gpt-5.2"),
        (
            "preset-gpt-5-1-codex-max",
            "GPT-5.1-Codex-Max",
            "gpt-5.1-codex-max",
        ),
        (
            "preset-gpt-5-1-codex-mini",
            "GPT-5.1-Codex-Mini",
            "gpt-5.1-codex-mini",
        ),
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

fn resolve_binary_from_path() -> Option<PathBuf> {
    let dirs = collect_runtime_search_dirs();

    logger::log_info(&format!(
        "[CodexWakeup][CLI] 扫描 CLI 搜索目录查找 codex: 目录数={}, 预览={}",
        dirs.len(),
        summarize_path_dirs_for_log(&dirs)
    ));

    resolve_binary_in_dirs(&dirs, binary_candidates())
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
    })
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

    if let Some(configured_cli_path) = configured_cli_path {
        let configured_path = PathBuf::from(&configured_cli_path);
        let resolved =
            resolve_configured_binary_path(&configured_path, binary_candidates(), "Codex CLI")
                .map_err(|err| {
                    logger::log_warn(&format!("[CodexWakeup][CLI] {}", err));
                    CliResolveError::new(err, &[REQUIRED_RUNTIME_PATH_CODEX_CLI])
                })?;
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 命中自定义 Codex CLI 路径: input={}, resolved={}",
            configured_path.display(),
            resolved.display()
        ));
        return build_resolved_binary(
            resolved,
            "runtime_config".to_string(),
            configured_node_path.as_deref(),
        );
    }

    if let Ok(raw) = std::env::var("CODEX_CLI_PATH") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.is_file() {
                logger::log_info(&format!(
                    "[CodexWakeup][CLI] 命中 CODEX_CLI_PATH: {}",
                    path.display()
                ));
                return build_resolved_binary(
                    path,
                    "CODEX_CLI_PATH".to_string(),
                    configured_node_path.as_deref(),
                );
            }
            let err = format!("CODEX_CLI_PATH 指向的文件不存在: {}", trimmed);
            logger::log_warn(&format!("[CodexWakeup][CLI] {}", err));
            return Err(CliResolveError::new(
                err,
                &[REQUIRED_RUNTIME_PATH_CODEX_CLI],
            ));
        }
    }

    if let Some(path) = resolve_binary_from_path() {
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 已从 PATH 解析到 codex: {}",
            path.display()
        ));
        return build_resolved_binary(path, "PATH".to_string(), configured_node_path.as_deref());
    }

    let err = "未检测到 Codex CLI，请先安装 `codex` 命令。".to_string();
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
    logger::log_info(&format!(
        "[CodexWakeup][CLI] 开始探测版本: codex_path={}, node_path={}",
        binary.path.display(),
        format_optional_path_for_log(binary.node_path.as_deref())
    ));
    let mut command = build_binary_command(&binary);
    command.arg("--version");
    let output = match command.output() {
        Ok(output) => output,
        Err(err) => {
            logger::log_warn(&format!("[CodexWakeup][CLI] 启动版本探测进程失败: {}", err));
            return None;
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        logger::log_warn(&format!(
            "[CodexWakeup][CLI] 版本探测失败: status={}, stdout={}, stderr={}",
            output.status,
            truncate_log_text(&stdout, 200),
            truncate_log_text(&stderr, 200)
        ));
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 版本探测成功: {}",
            truncate_log_text(&stdout, 200)
        ));
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        logger::log_info(&format!(
            "[CodexWakeup][CLI] 版本探测成功(stderr): {}",
            truncate_log_text(&stderr, 200)
        ));
        return Some(stderr);
    }
    logger::log_info("[CodexWakeup][CLI] 版本探测完成，但未返回输出");
    None
}

fn build_binary_command(binary: &ResolvedBinary) -> Command {
    let mut command = if let Some(node_path) = &binary.node_path {
        let mut command = Command::new(node_path);
        command.arg(&binary.path);
        command
    } else {
        Command::new(&binary.path)
    };
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
    }
}

fn disable_tasks_when_cli_missing_with_runtime(
    state: &mut CodexWakeupState,
    runtime_available: bool,
) -> bool {
    if runtime_available {
        return false;
    }

    let mut changed = false;
    if state.enabled {
        state.enabled = false;
        changed = true;
    }

    for task in &mut state.tasks {
        if task.enabled {
            task.enabled = false;
            task.updated_at = now_ts();
            changed = true;
        }
    }

    changed
}

fn disable_tasks_when_cli_missing(state: &mut CodexWakeupState) -> bool {
    let runtime = get_cli_status();
    disable_tasks_when_cli_missing_with_runtime(state, runtime.available)
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
    let temp_path = parent.join(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("codex_wakeup")
    ));
    let content =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化 JSON 失败: {}", e))?;
    fs::write(&temp_path, content).map_err(|e| format!("写入临时文件失败: {}", e))?;
    fs::rename(&temp_path, path).map_err(|e| format!("替换文件失败: {}", e))
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
    let raw: CodexWakeupRuntimeConfig = serde_json::from_str(&content)
        .map_err(|e| format!("解析 Codex 唤醒运行时配置失败: {}", e))?;
    Ok(normalize_runtime_config(&raw))
}

pub fn save_runtime_config(
    next_config: &CodexWakeupRuntimeConfig,
) -> Result<CodexWakeupRuntimeConfig, String> {
    let normalized = normalize_runtime_config(next_config);
    save_json_atomic(&runtime_config_path()?, &normalized)?;
    Ok(normalized)
}

fn load_state_inner(
    apply_cli_guard: bool,
    runtime_available_override: Option<bool>,
) -> Result<CodexWakeupState, String> {
    let path = tasks_path()?;
    if !path.exists() {
        return Ok(CodexWakeupState::default());
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 Codex 唤醒任务失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(CodexWakeupState::default());
    }
    let mut state: CodexWakeupState =
        serde_json::from_str(&content).map_err(|e| format!("解析 Codex 唤醒任务失败: {}", e))?;
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
    let migration_changed = ensure_gpt_5_5_model_preset(&mut state);
    let changed = if apply_cli_guard {
        if let Some(runtime_available) = runtime_available_override {
            disable_tasks_when_cli_missing_with_runtime(&mut state, runtime_available)
        } else {
            disable_tasks_when_cli_missing(&mut state)
        }
    } else {
        false
    };
    refresh_next_run_at(&mut state);
    if migration_changed || changed {
        let _lock = TASKS_LOCK.lock().map_err(|_| "获取 Codex 唤醒任务锁失败")?;
        save_json_atomic(&path, &state)?;
    }
    Ok(state)
}

pub fn load_state() -> Result<CodexWakeupState, String> {
    load_state_inner(true, None)
}

pub fn load_state_for_scheduler() -> Result<CodexWakeupState, String> {
    load_state_inner(false, None)
}

fn load_state_with_runtime_available(runtime_available: bool) -> Result<CodexWakeupState, String> {
    load_state_inner(true, Some(runtime_available))
}

pub fn load_overview() -> Result<CodexWakeupOverview, String> {
    let runtime = get_cli_status();
    let state = load_state_with_runtime_available(runtime.available)?;
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
    ensure_gpt_5_5_model_preset(&mut state);

    disable_tasks_when_cli_missing(&mut state);
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
    serde_json::from_str(&content).map_err(|e| format!("解析 Codex 唤醒历史失败: {}", e))
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
    binary: Option<&ResolvedBinary>,
    run_id: &str,
    context: &TaskRunContext,
    account_id: &str,
    prompt: &str,
    execution_config: &CodexWakeupExecutionConfig,
    cancel_flag: Option<&Arc<AtomicBool>>,
) -> CodexWakeupHistoryItem {
    let prompt_value = Some(prompt.to_string());
    let binary_path = binary.map(|item| item.path.display().to_string());
    if is_scope_cancelled(cancel_flag) {
        return create_cancelled_record(
            run_id,
            context,
            account_id,
            prompt_value,
            execution_config,
            binary_path,
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
                binary_path,
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
            "Codex 唤醒任务暂不支持 API Key 账号。".to_string(),
            binary_path,
        );
    }

    let Some(binary) = binary else {
        return create_cli_missing_record(
            run_id,
            context,
            account_id,
            prompt_value,
            execution_config,
        );
    };

    let managed_home = match managed_home_path(account_id) {
        Ok(path) => path,
        Err(err) => {
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
                err,
                Some(binary.path.display().to_string()),
            );
        }
    };

    if let Err(err) = fs::create_dir_all(&managed_home) {
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
            format!("创建受管 CODEX_HOME 失败: {}", err),
            Some(binary.path.display().to_string()),
        );
    }

    let (account, command_result, sync_error) =
        match codex_account::execute_with_managed_account_projection(
            account_id,
            &managed_home,
            "Codex 唤醒执行",
            |_| run_codex_exec_sync(binary, &managed_home, prompt, execution_config, cancel_flag),
        )
        .await
        {
            Ok(result) => result,
            Err(err) => {
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
                    err,
                    Some(binary.path.display().to_string()),
                )
            }
        };
    if let Some(err) = sync_error {
        logger::log_warn(&format!(
            "Codex 唤醒执行后同步受管目录 Token 失败: account_id={}, managed_home={}, error={}",
            account_id,
            managed_home.display(),
            err
        ));
    }

    match command_result {
        Ok(output) => {
            let account_context_text = resolve_account_context_text(&account);
            let account_email = account.email;
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
                cli_path: Some(binary.path.display().to_string()),
                quota_before: None,
                quota_after: None,
            }
        }
        Err(err) => {
            let account_context_text = resolve_account_context_text(&account);
            let account_email = account.email;
            create_failure_record(
                run_id,
                &context.trigger_type,
                context.task_id.as_deref(),
                context.task_name.as_deref(),
                account_id,
                account_email,
                account_context_text,
                prompt_value,
                execution_config,
                err,
                Some(binary.path.display().to_string()),
            )
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
    let cleaned_ids: Vec<String> = account_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if cleaned_ids.is_empty() {
        return Err("至少选择一个账号".to_string());
    }

    let prompt = prompt
        .as_ref()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let total = cleaned_ids.len();
    let runtime = get_cli_status();
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
    if !runtime.available {
        let mut records = Vec::with_capacity(cleaned_ids.len());
        let mut success_count = 0usize;
        let mut failure_count = 0usize;

        for (index, account_id) in cleaned_ids.iter().enumerate() {
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
                Some(account_id),
                None,
            );

            let record = create_cli_missing_record(
                &run_id,
                &context,
                account_id,
                Some(prompt.clone()),
                &execution_config,
            );
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
                Some(account_id),
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

        return Ok(CodexWakeupBatchResult {
            run_id,
            runtime,
            records,
            success_count,
            failure_count,
        });
    }

    let binary = resolve_binary().ok();
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
                binary.as_ref().map(|item| item.path.display().to_string()),
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
            binary.as_ref(),
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
