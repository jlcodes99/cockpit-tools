use crate::modules;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

const WAKEUP_VERIFICATION_STATE_FILE: &str = "wakeup_verification_state.json";
const WAKEUP_ERROR_JSON_PREFIX: &str = "AG_WAKEUP_ERROR_JSON:";
const WAKEUP_VERIFICATION_STATE_VERSION: u8 = 2;
const MAX_HISTORY_BATCHES: usize = 100;

const STATUS_IDLE: &str = "idle";
const STATUS_SUCCESS: &str = "success";
const STATUS_VERIFICATION_REQUIRED: &str = "verification_required";
const STATUS_TOS_VIOLATION: &str = "tos_violation";
const STATUS_AUTH_EXPIRED: &str = "auth_expired";
const STATUS_FAILED: &str = "failed";

static VERIFY_STATE_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupVerificationStateItem {
    pub account_id: String,
    pub account_email: String,
    pub status: String,
    pub last_verify_at: Option<i64>,
    pub last_model: Option<String>,
    pub last_error_code: Option<i64>,
    pub last_message: Option<String>,
    pub validation_url: Option<String>,
    #[serde(default)]
    pub appeal_url: Option<String>,
    pub trajectory_id: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupVerificationBatchHistoryItem {
    pub batch_id: String,
    pub verified_at: i64,
    pub model: String,
    pub prompt: String,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub verification_required_count: usize,
    #[serde(default)]
    pub tos_violation_count: usize,
    pub failed_count: usize,
    pub records: Vec<WakeupVerificationStateItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupVerificationProgressPayload {
    pub batch_id: String,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub verification_required_count: usize,
    pub tos_violation_count: usize,
    pub failed_count: usize,
    pub running: bool,
    pub item: Option<WakeupVerificationStateItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeupVerificationBatchResult {
    pub batch_id: String,
    pub verified_at: i64,
    pub model: String,
    pub prompt: String,
    pub total: usize,
    pub completed: usize,
    pub success_count: usize,
    pub verification_required_count: usize,
    pub tos_violation_count: usize,
    pub failed_count: usize,
    pub records: Vec<WakeupVerificationStateItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WakeupUiErrorPayload {
    kind: Option<String>,
    message: Option<String>,
    error_code: Option<i64>,
    validation_url: Option<String>,
    appeal_url: Option<String>,
    trajectory_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct WakeupVerificationStateFile {
    #[serde(default = "default_state_file_version")]
    version: u8,
    #[serde(default)]
    items: Vec<WakeupVerificationStateItem>,
    #[serde(default)]
    history: Vec<WakeupVerificationBatchHistoryItem>,
}

fn default_state_file_version() -> u8 {
    WAKEUP_VERIFICATION_STATE_VERSION
}

fn state_file_path() -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    Ok(data_dir.join(WAKEUP_VERIFICATION_STATE_FILE))
}

fn load_state_file_unlocked() -> Result<WakeupVerificationStateFile, String> {
    let path = state_file_path()?;
    if !path.exists() {
        return Ok(WakeupVerificationStateFile {
            version: WAKEUP_VERIFICATION_STATE_VERSION,
            items: Vec::new(),
            history: Vec::new(),
        });
    }

    let content = fs::read_to_string(&path).map_err(|e| format!("读取验证状态文件失败: {}", e))?;
    if content.trim().is_empty() {
        return Ok(WakeupVerificationStateFile {
            version: WAKEUP_VERIFICATION_STATE_VERSION,
            items: Vec::new(),
            history: Vec::new(),
        });
    }

    let mut parsed: WakeupVerificationStateFile = match serde_json::from_str(&content) {
        Ok(parsed) => parsed,
        Err(error) => {
            match modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => modules::logger::log_warn(&format!(
                    "验证状态文件解析失败，已隔离并使用空状态: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => modules::logger::log_warn(&format!(
                    "验证状态文件解析失败，文件已不存在，使用空状态: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => modules::logger::log_warn(&format!(
                    "验证状态文件解析失败，隔离失败，使用空状态: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            WakeupVerificationStateFile {
                version: WAKEUP_VERIFICATION_STATE_VERSION,
                items: Vec::new(),
                history: Vec::new(),
            }
        }
    };
    if parsed.version == 0 {
        parsed.version = WAKEUP_VERIFICATION_STATE_VERSION;
    }
    Ok(parsed)
}

fn save_state_file_unlocked(state: &WakeupVerificationStateFile) -> Result<(), String> {
    let path = state_file_path()?;

    let payload = WakeupVerificationStateFile {
        version: WAKEUP_VERIFICATION_STATE_VERSION,
        items: state.items.clone(),
        history: state.history.clone(),
    };

    let content =
        serde_json::to_string_pretty(&payload).map_err(|e| format!("序列化验证状态失败: {}", e))?;
    modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|e| format!("保存验证状态失败: {}", e))
}

pub fn load_state() -> Result<Vec<WakeupVerificationStateItem>, String> {
    let _lock = VERIFY_STATE_LOCK.lock().map_err(|_| "获取验证状态锁失败")?;
    let mut items = load_state_file_unlocked()?.items;
    sort_state_items(&mut items);
    Ok(items)
}

pub fn load_history() -> Result<Vec<WakeupVerificationBatchHistoryItem>, String> {
    let _lock = VERIFY_STATE_LOCK.lock().map_err(|_| "获取验证状态锁失败")?;
    let mut history = load_state_file_unlocked()?.history;

    if let Ok(accounts) = modules::list_accounts() {
        let email_by_id: HashMap<String, String> = accounts
            .into_iter()
            .map(|account| (account.id, account.email))
            .collect();
        for batch in &mut history {
            for item in &mut batch.records {
                if let Some(email) = email_by_id.get(&item.account_id) {
                    item.account_email = email.clone();
                }
            }
        }
    }

    history.sort_by(|a, b| b.verified_at.cmp(&a.verified_at));
    Ok(history)
}

pub fn delete_history(batch_ids: Vec<String>) -> Result<usize, String> {
    let targets: HashSet<String> = batch_ids
        .into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();

    if targets.is_empty() {
        return Ok(0);
    }

    let _lock = VERIFY_STATE_LOCK.lock().map_err(|_| "获取验证状态锁失败")?;
    let mut state = load_state_file_unlocked().unwrap_or_default();
    let before = state.history.len();
    state
        .history
        .retain(|item| !targets.contains(&item.batch_id));
    let deleted = before.saturating_sub(state.history.len());

    if deleted > 0 {
        save_state_file_unlocked(&state)?;
    }

    Ok(deleted)
}

fn sort_state_items(items: &mut Vec<WakeupVerificationStateItem>) {
    items.sort_by(|a, b| {
        b.last_verify_at
            .unwrap_or_default()
            .cmp(&a.last_verify_at.unwrap_or_default())
            .then_with(|| a.account_email.cmp(&b.account_email))
    });
}

fn upsert_state_items(items: Vec<WakeupVerificationStateItem>) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }

    let _lock = VERIFY_STATE_LOCK.lock().map_err(|_| "获取验证状态锁失败")?;
    let mut state = load_state_file_unlocked().unwrap_or_default();
    let mut by_account: HashMap<String, WakeupVerificationStateItem> = state
        .items
        .drain(..)
        .map(|item| (item.account_id.clone(), item))
        .collect();

    for item in items {
        by_account.insert(item.account_id.clone(), item);
    }

    let mut merged: Vec<WakeupVerificationStateItem> = by_account.into_values().collect();
    sort_state_items(&mut merged);

    state.items = merged;
    save_state_file_unlocked(&state)
}

fn append_history_batch(batch: WakeupVerificationBatchHistoryItem) -> Result<(), String> {
    let _lock = VERIFY_STATE_LOCK.lock().map_err(|_| "获取验证状态锁失败")?;
    let mut state = load_state_file_unlocked().unwrap_or_default();

    state.history.retain(|item| item.batch_id != batch.batch_id);
    state.history.push(batch);
    state
        .history
        .sort_by(|a, b| b.verified_at.cmp(&a.verified_at));
    if state.history.len() > MAX_HISTORY_BATCHES {
        state.history.truncate(MAX_HISTORY_BATCHES);
    }

    save_state_file_unlocked(&state)
}

fn parse_wakeup_error_payload(error: &str) -> Option<WakeupUiErrorPayload> {
    let payload = error.strip_prefix(WAKEUP_ERROR_JSON_PREFIX)?.trim();
    if payload.is_empty() {
        return None;
    }
    serde_json::from_str::<WakeupUiErrorPayload>(payload).ok()
}

fn classify_failure(
    raw_error: &str,
) -> (
    &'static str,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
) {
    if let Some(payload) = parse_wakeup_error_payload(raw_error) {
        let status = if payload.kind.as_deref() == Some(STATUS_TOS_VIOLATION) {
            STATUS_TOS_VIOLATION
        } else if payload.kind.as_deref() == Some(STATUS_VERIFICATION_REQUIRED)
            || payload.error_code == Some(403)
        {
            STATUS_VERIFICATION_REQUIRED
        } else {
            STATUS_FAILED
        };
        let message = payload.message.unwrap_or_else(|| raw_error.to_string());
        return (
            status,
            payload.error_code,
            payload.validation_url,
            payload.appeal_url,
            payload.trajectory_id,
            message,
        );
    }

    let lower = raw_error.to_ascii_lowercase();
    if lower.contains("authorization expired")
        || lower.contains("unauthorized")
        || lower.contains("unauthenticated")
    {
        return (
            STATUS_AUTH_EXPIRED,
            Some(401),
            None,
            None,
            None,
            raw_error.to_string(),
        );
    }
    if lower.contains("tos_violation") || lower.contains("violation of terms") {
        return (
            STATUS_TOS_VIOLATION,
            Some(403),
            None,
            None,
            None,
            raw_error.to_string(),
        );
    }
    if lower.contains("403") {
        return (
            STATUS_VERIFICATION_REQUIRED,
            Some(403),
            None,
            None,
            None,
            raw_error.to_string(),
        );
    }

    (STATUS_FAILED, None, None, None, None, raw_error.to_string())
}

pub async fn run_batch(
    app: &AppHandle,
    account_ids: Vec<String>,
    model: &str,
    prompt: &str,
    max_output_tokens: u32,
) -> Result<WakeupVerificationBatchResult, String> {
    let mut deduped_ids = Vec::new();
    let mut seen = HashSet::new();
    for raw in account_ids {
        let id = raw.trim();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_string()) {
            deduped_ids.push(id.to_string());
        }
    }

    if deduped_ids.is_empty() {
        return Err("未选择账号".to_string());
    }

    let all_accounts = modules::list_accounts()?;
    let account_email_by_id: HashMap<String, String> = all_accounts
        .into_iter()
        .map(|account| (account.id, account.email))
        .collect();

    let selected_accounts: Vec<(String, String)> = deduped_ids
        .iter()
        .filter_map(|id| {
            account_email_by_id
                .get(id)
                .map(|email| (id.clone(), email.clone()))
        })
        .collect();

    if selected_accounts.is_empty() {
        return Err("未找到可用账号".to_string());
    }

    let batch_id = format!("verify_{}", chrono::Utc::now().timestamp_millis());
    let total = selected_accounts.len();
    let model_owned = model.to_string();
    let prompt_owned = prompt.to_string();

    let mut futures = FuturesUnordered::new();
    for (account_id, account_email) in selected_accounts {
        let account_id_owned = account_id.clone();
        let account_email_owned = account_email.clone();
        let model_for_task = model_owned.clone();
        let prompt_for_task = prompt_owned.clone();
        futures.push(async move {
            let started_at = std::time::Instant::now();
            let result = modules::wakeup::trigger_wakeup(
                &account_id_owned,
                &model_for_task,
                &prompt_for_task,
                max_output_tokens,
                None,
            )
            .await;

            let now_ms = chrono::Utc::now().timestamp_millis();
            match result {
                Ok(resp) => WakeupVerificationStateItem {
                    account_id: account_id_owned,
                    account_email: account_email_owned,
                    status: STATUS_SUCCESS.to_string(),
                    last_verify_at: Some(now_ms),
                    last_model: Some(model_for_task),
                    last_error_code: None,
                    last_message: Some(resp.reply),
                    validation_url: None,
                    appeal_url: None,
                    trajectory_id: None,
                    duration_ms: Some(resp.duration_ms),
                },
                Err(err) => {
                    let (status, code, validation_url, appeal_url, trajectory_id, message) =
                        classify_failure(&err);
                    WakeupVerificationStateItem {
                        account_id: account_id_owned,
                        account_email: account_email_owned,
                        status: status.to_string(),
                        last_verify_at: Some(now_ms),
                        last_model: Some(model_for_task),
                        last_error_code: code,
                        last_message: Some(message),
                        validation_url,
                        appeal_url,
                        trajectory_id,
                        duration_ms: Some(started_at.elapsed().as_millis() as u64),
                    }
                }
            }
        });
    }

    let mut completed = 0usize;
    let mut success_count = 0usize;
    let mut verification_required_count = 0usize;
    let mut tos_violation_count = 0usize;
    let mut failed_count = 0usize;
    let mut records = Vec::new();

    while let Some(item) = futures.next().await {
        completed += 1;
        match item.status.as_str() {
            STATUS_SUCCESS => {
                success_count += 1;
                // 账号检测成功，账号完全可用，解除所有类型的禁用
                // （包括 verification_required / tos_violation / invalid_grant）
                if let Ok(mut account) = modules::load_account(&item.account_id) {
                    if account.disabled {
                        modules::logger::log_info(&format!(
                            "[WakeupVerification] 验证成功，自动解除禁用状态: {}",
                            account.email
                        ));
                        account.clear_disabled();
                        account.quota_error = None;
                        let _ = modules::save_account(&account);
                    }
                }
            }
            STATUS_VERIFICATION_REQUIRED => {
                verification_required_count += 1;
                // 身份验证失败，禁用账号
                if let Ok(mut account) = modules::load_account(&item.account_id) {
                    account.disabled = true;
                    account.disabled_reason = Some("verification_required".to_string());
                    account.disabled_at = Some(chrono::Utc::now().timestamp());
                    let _ = modules::save_account(&account);
                }
            }
            STATUS_TOS_VIOLATION => {
                tos_violation_count += 1;
                // TOS 违规，禁用账号
                if let Ok(mut account) = modules::load_account(&item.account_id) {
                    account.disabled = true;
                    account.disabled_reason = Some("tos_violation".to_string());
                    account.disabled_at = Some(chrono::Utc::now().timestamp());
                    let _ = modules::save_account(&account);
                }
            }
            STATUS_AUTH_EXPIRED | STATUS_FAILED => failed_count += 1,
            _ => failed_count += 1,
        }

        let _ = upsert_state_items(vec![item.clone()]);
        records.push(item.clone());

        let payload = WakeupVerificationProgressPayload {
            batch_id: batch_id.clone(),
            total,
            completed,
            success_count,
            verification_required_count,
            tos_violation_count,
            failed_count,
            running: completed < total,
            item: Some(item),
        };
        let _ = app.emit("wakeup://verification-progress", payload);
    }

    records.sort_by(|a, b| a.account_email.cmp(&b.account_email));
    let completed = records.len();
    let verified_at = chrono::Utc::now().timestamp_millis();

    let final_payload = WakeupVerificationProgressPayload {
        batch_id: batch_id.clone(),
        total,
        completed,
        success_count,
        verification_required_count,
        tos_violation_count,
        failed_count,
        running: false,
        item: None,
    };
    let _ = app.emit("wakeup://verification-progress", final_payload);

    let history_item = WakeupVerificationBatchHistoryItem {
        batch_id: batch_id.clone(),
        verified_at,
        model: model_owned.clone(),
        prompt: prompt_owned.clone(),
        total,
        completed,
        success_count,
        verification_required_count,
        tos_violation_count,
        failed_count,
        records: records.clone(),
    };

    if let Err(err) = append_history_batch(history_item) {
        crate::modules::logger::log_warn(&format!(
            "[WakeupVerification] 保存验证历史失败: {}",
            err
        ));
    }

    Ok(WakeupVerificationBatchResult {
        batch_id,
        verified_at,
        model: model_owned,
        prompt: prompt_owned,
        total,
        completed,
        success_count,
        verification_required_count,
        tos_violation_count,
        failed_count,
        records,
    })
}

pub fn build_display_state_for_all_accounts() -> Result<Vec<WakeupVerificationStateItem>, String> {
    let account_list = modules::list_accounts()?;
    let saved = load_state().unwrap_or_default();
    let mut saved_map: HashMap<String, WakeupVerificationStateItem> = saved
        .into_iter()
        .map(|item| (item.account_id.clone(), item))
        .collect();

    let mut result = Vec::new();
    for account in account_list {
        if let Some(mut item) = saved_map.remove(&account.id) {
            item.account_email = account.email.clone();
            result.push(item);
        } else {
            result.push(WakeupVerificationStateItem {
                account_id: account.id,
                account_email: account.email,
                status: STATUS_IDLE.to_string(),
                last_verify_at: None,
                last_model: None,
                last_error_code: None,
                last_message: None,
                validation_url: None,
                appeal_url: None,
                trajectory_id: None,
                duration_ms: None,
            });
        }
    }

    sort_state_items(&mut result);
    Ok(result)
}
