use crate::modules;
use crate::modules::wakeup_history_store::HistoryRead;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

pub(crate) const WAKEUP_VERIFICATION_STATE_FILE: &str = "wakeup_verification_state.json";
const WAKEUP_ERROR_JSON_PREFIX: &str = "AG_WAKEUP_ERROR_JSON:";
const MAX_HISTORY_BATCHES: usize = 100;

#[path = "wakeup_verification_store.rs"]
mod store;

const STATUS_IDLE: &str = "idle";
const STATUS_SUCCESS: &str = "success";
const STATUS_VERIFICATION_REQUIRED: &str = "verification_required";
const STATUS_TOS_VIOLATION: &str = "tos_violation";
const STATUS_AUTH_EXPIRED: &str = "auth_expired";
const STATUS_FAILED: &str = "failed";

static RESULT_WRITE_LOCK: std::sync::LazyLock<Mutex<()>> =
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_error: Option<String>,
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

fn verification_store() -> Result<store::VerificationStore, String> {
    Ok(store::VerificationStore::new(
        modules::account::get_data_dir()?.join(WAKEUP_VERIFICATION_STATE_FILE),
    ))
}

pub fn schedule_migration() -> Result<(), String> {
    verification_store()?.schedule();
    Ok(())
}

pub fn load_history(
    retry: bool,
) -> Result<HistoryRead<WakeupVerificationBatchHistoryItem>, String> {
    let mut history = verification_store()?.load_history(retry)?;
    if let Ok(index) = modules::account::load_account_index() {
        let email_by_id: HashMap<String, String> = index
            .accounts
            .into_iter()
            .map(|account| (account.id, account.email))
            .collect();
        for batch in &mut history.records {
            for item in &mut batch.records {
                if let Some(email) = email_by_id.get(&item.account_id) {
                    item.account_email = email.clone();
                }
            }
        }
    }
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
    verification_store()?.delete_batches(&targets)
}

fn sort_state_items(items: &mut Vec<WakeupVerificationStateItem>) {
    items.sort_by(|a, b| {
        b.last_verify_at
            .unwrap_or_default()
            .cmp(&a.last_verify_at.unwrap_or_default())
            .then_with(|| a.account_email.cmp(&b.account_email))
    });
}

fn record_result(item: &WakeupVerificationStateItem) -> Result<(), String> {
    let _lock = RESULT_WRITE_LOCK
        .lock()
        .map_err(|_| "获取验证结果写入锁失败")?;
    if !verification_store()?.upsert_result(item)? {
        return Ok(());
    }
    match item.status.as_str() {
        STATUS_SUCCESS => {
            if let Ok(mut account) = modules::load_account(&item.account_id) {
                if account.disabled {
                    modules::logger::log_info(&format!(
                        "[WakeupVerification] 验证成功，自动解除禁用状态: {}",
                        account.email
                    ));
                    account.clear_disabled();
                    account.quota_error = None;
                    modules::save_account(&account)?;
                }
            }
        }
        reason @ (STATUS_VERIFICATION_REQUIRED | STATUS_TOS_VIOLATION) => {
            if let Ok(mut account) = modules::load_account(&item.account_id) {
                account.disabled = true;
                account.disabled_reason = Some(reason.to_string());
                account.disabled_at = Some(chrono::Utc::now().timestamp());
                modules::save_account(&account)?;
            }
        }
        _ => {}
    }
    Ok(())
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

    let all_accounts = tauri::async_runtime::spawn_blocking(modules::list_accounts)
        .await
        .map_err(|error| error.to_string())??;
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

    let batch_id = format!("verify_{}", uuid::Uuid::new_v4());
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
                    let validation_url = validation_url.map(|u| {
                        crate::modules::quota::format_google_validation_url(
                            &u,
                            &account_email_owned,
                        )
                    });
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
    let mut storage_error = None;

    while let Some(item) = futures.next().await {
        completed += 1;
        match item.status.as_str() {
            STATUS_SUCCESS => success_count += 1,
            STATUS_VERIFICATION_REQUIRED => verification_required_count += 1,
            STATUS_TOS_VIOLATION => tos_violation_count += 1,
            _ => failed_count += 1,
        }

        let stored = item.clone();
        let result = tauri::async_runtime::spawn_blocking(move || record_result(&stored))
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
        if let Err(error) = result {
            modules::logger::log_warn(&format!(
                "[WakeupVerification] 保存账号验证结果失败: {error}"
            ));
            storage_error.get_or_insert(error);
        }
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

    let result = tauri::async_runtime::spawn_blocking(move || {
        verification_store()?.append_batch(&history_item)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = result {
        modules::logger::log_warn(&format!("[WakeupVerification] 保存验证历史失败: {error}"));
        storage_error.get_or_insert(error);
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
        storage_error,
    })
}

pub fn build_display_state_for_all_accounts(
    retry: bool,
) -> Result<HistoryRead<WakeupVerificationStateItem>, String> {
    let account_list = modules::list_accounts()?;
    let mut saved = verification_store()?.load_results(retry)?;
    let complete = saved.is_complete();
    saved.records = display_state_for_accounts(account_list, saved.records, complete);
    Ok(saved)
}

fn display_state_for_accounts(
    account_list: Vec<crate::models::account::Account>,
    saved: Vec<WakeupVerificationStateItem>,
    complete: bool,
) -> Vec<WakeupVerificationStateItem> {
    let mut saved_map: HashMap<String, WakeupVerificationStateItem> = saved
        .into_iter()
        .map(|item| (item.account_id.clone(), item))
        .collect();

    let mut result = Vec::new();
    for account in account_list {
        if let Some(mut item) = saved_map.remove(&account.id) {
            item.account_email = account.email.clone();
            result.push(item);
        } else if complete {
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
    result
}
