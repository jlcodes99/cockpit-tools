use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tauri::Emitter;
use tokio::sync::Mutex as AsyncMutex;

use crate::models::codex::{CodexAccount, CodexAccountIndex, CodexAccountSummary};
use crate::modules::{account, codex_account, codex_oauth, codex_quota, logger};

const HOUR: i64 = 3600;
const INDEX_INTERVAL: i64 = 30;
const REQUEST_SPACING: Duration = Duration::from_secs(3);

static ACCOUNT_GATES: LazyLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn account_gate(id: &str) -> Arc<AsyncMutex<()>> {
    let mut gates = ACCOUNT_GATES
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(gate) = gates.get(id).and_then(Weak::upgrade) {
        return gate;
    }
    gates.retain(|_, gate| gate.strong_count() > 0);
    let gate = Arc::new(AsyncMutex::new(()));
    gates.insert(id.to_owned(), Arc::downgrade(&gate));
    gate
}

fn phase(id: &str, expiry: Option<&str>) -> i64 {
    expiry
        .and_then(codex_quota::parse_subscription_timestamp)
        .unwrap_or_else(|| {
            let hash = Sha256::digest(id.as_bytes());
            i64::from(u32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]))
        })
        .rem_euclid(HOUR)
}

fn next_slot(phase: i64, from: i64, last_hour: Option<i64>) -> i64 {
    let hour = from.div_euclid(HOUR);
    let slot_hour = hour + i64::from(phase < from.rem_euclid(HOUR));
    slot_hour.max(last_hour.map_or(slot_hour, |hour| hour + 1)) * HOUR + phase
}

// Read the existing lightweight index. Account listing owns repair and migration;
// scheduling only reads the committed index and never scans account details.
fn read_index() -> Result<Vec<CodexAccountSummary>, String> {
    let path = account::get_data_dir()?.join("codex_accounts.json");
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let index: CodexAccountIndex =
        serde_json::from_slice(&data).map_err(|error| error.to_string())?;
    Ok(index.accounts)
}

fn supports_subscription_query(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && !account.is_agent_identity_auth()
        && !account.is_web_session_auth()
        && !codex_account::is_chatgpt_plan_account(account)
        && !codex_account::is_pending_oauth_account(account)
}

async fn refresh_due_account(id: &str, scheduled_at: i64) -> Result<Option<i64>, String> {
    // A manual refresh already in progress serves this slot. Don't queue behind it.
    let Ok(_gate) = account_gate(id).try_lock_owned() else {
        return Ok(None);
    };
    let account_id = id.to_owned();
    let Some(mut account) =
        tokio::task::spawn_blocking(move || codex_account::load_account(&account_id))
            .await
            .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if !supports_subscription_query(&account)
        || account
            .subscription_query_last_attempt_at
            .is_some_and(|attempt| {
                attempt >= scheduled_at
                    || (attempt.div_euclid(HOUR) == scheduled_at.div_euclid(HOUR)
                        && account
                            .subscription_query_last_success_at
                            .is_some_and(|success| {
                                success >= attempt
                                    && account.subscription_query_last_error.is_none()
                            }))
            })
    {
        return Ok(None);
    }
    // A usable access token is enough. Renew an expired one through the
    // ownership-aware path, off the runtime threads that serve the UI.
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        let id = id.to_owned();
        let runtime = tokio::runtime::Handle::current();
        account = tokio::task::spawn_blocking(move || {
            runtime.block_on(codex_account::prepare_account_for_quota_query(&id))
        })
        .await
        .map_err(|error| error.to_string())??;
        if !supports_subscription_query(&account) {
            return Ok(None);
        }
    }
    let account = codex_quota::query_and_save_subscription(account).await?;
    Ok(Some(phase(
        id,
        account.subscription_active_until.as_deref(),
    )))
}

pub(crate) async fn run() {
    let mut reload_at = chrono::Utc::now().timestamp() + 1;
    let mut next_request = tokio::time::Instant::now();
    let mut last_hours = HashMap::new();
    let mut schedule: BinaryHeap<Reverse<(i64, String)>> = BinaryHeap::new();

    loop {
        let now = chrono::Utc::now().timestamp();
        if now >= reload_at {
            // After a long suspension, skip missed slots instead of starting a batch.
            if now - reload_at >= HOUR {
                schedule.clear();
            }
            reload_at = now + INDEX_INTERVAL;
            last_hours.retain(|_, hour| *hour >= now.div_euclid(HOUR) - 1);
            match tokio::task::spawn_blocking(read_index).await {
                Ok(Ok(accounts)) => {
                    let pending: HashMap<_, _> = schedule
                        .drain()
                        .map(|Reverse((due, id))| (id, due))
                        .collect();
                    schedule = accounts
                        .into_iter()
                        .map(|account| {
                            let phase =
                                phase(&account.id, account.subscription_active_until.as_deref());
                            // Keep an unchanged slot even if a collision delayed it across an hour.
                            let due = pending
                                .get(&account.id)
                                .copied()
                                .filter(|due| due.rem_euclid(HOUR) == phase)
                                .unwrap_or_else(|| {
                                    next_slot(phase, now, last_hours.get(&account.id).copied())
                                });
                            Reverse((due, account.id))
                        })
                        .collect();
                }
                Ok(Err(error)) => {
                    logger::log_warn(&format!("[Codex Subscription] Account index: {error}"))
                }
                Err(error) => {
                    logger::log_warn(&format!("[Codex Subscription] Account index task: {error}"))
                }
            }
            continue;
        }
        if let Some(Reverse((due, _))) = schedule.peek() {
            if *due <= now && next_request <= tokio::time::Instant::now() {
                let Some(Reverse((due, id))) = schedule.pop() else {
                    continue;
                };
                let hour = now.div_euclid(HOUR);
                if last_hours.insert(id.clone(), hour) == Some(hour) {
                    continue;
                }
                let result = refresh_due_account(&id, due).await;
                let next_phase = result
                    .as_ref()
                    .ok()
                    .copied()
                    .flatten()
                    .unwrap_or_else(|| due.rem_euclid(HOUR));
                schedule.push(Reverse(((hour + 1) * HOUR + next_phase, id.clone())));
                if !matches!(result, Ok(None)) {
                    next_request = tokio::time::Instant::now() + REQUEST_SPACING;
                }
                if let Err(error) = result {
                    logger::log_warn(&format!("[Codex Subscription] {id}: {error}"));
                }
                continue;
            }
        }
        let next_slot_at = schedule.peek().map_or(reload_at, |Reverse((due, _))| *due);
        let now_utc = chrono::Utc::now();
        let now = tokio::time::Instant::now();
        let until = |timestamp: i64| {
            let millis = (timestamp * 1000_i64 - now_utc.timestamp_millis()).max(0);
            now + Duration::from_millis(millis as u64)
        };
        tokio::time::sleep_until(until(next_slot_at).max(next_request).min(until(reload_at))).await;
    }
}

pub(crate) fn publish(original: &CodexAccount, account: &CodexAccount) {
    let Some(app) = crate::get_app_handle() else {
        return;
    };
    let payload = serde_json::json!({
        "id": account.id,
        "previous_account_id": original.account_id,
        "account_id": account.account_id,
        "organization_id": account.organization_id,
        "token_generation": account.token_generation,
        "plan_type": account.plan_type,
        "usage_updated_at": account.usage_updated_at,
        "subscription_active_until": account.subscription_active_until,
        "subscription_query_last_attempt_at": account.subscription_query_last_attempt_at,
        "subscription_query_last_success_at": account.subscription_query_last_success_at,
        "subscription_query_last_error": account.subscription_query_last_error,
    });
    if let Err(error) = app.emit("codex:subscription-updated", payload) {
        logger::log_warn(&format!("[Codex Subscription] Update event: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_by_expiry_minute_and_second_including_expired_dates_and_epoch_millis() {
        let expiry = "2020-03-01T14:23:45+08:00";
        assert_eq!(phase("account-a", Some(expiry)), 23 * 60 + 45);
        let millis = chrono::DateTime::parse_from_rfc3339(expiry)
            .unwrap()
            .timestamp_millis();
        assert_eq!(
            phase("account-b", Some(&millis.to_string())),
            phase("account-a", Some(expiry))
        );
        assert_eq!(next_slot(1425, HOUR + 1425, None), HOUR + 1425);
        assert_eq!(next_slot(1425, HOUR + 1426, None), 2 * HOUR + 1425);
    }

    #[test]
    fn missing_and_invalid_dates_use_the_same_stable_account_slot() {
        let slot = phase("account-a", None);
        assert_eq!(slot, phase("account-a", Some("invalid")));
        assert_ne!(slot, phase("account-b", None));
        assert!((0..HOUR).contains(&slot));
    }

    #[test]
    fn renewal_changes_the_phase_from_the_next_hour() {
        assert_eq!(next_slot(3590, HOUR + 1426, Some(1)), 2 * HOUR + 3590);
        assert_eq!(next_slot(15, HOUR + 1426, Some(1)), 2 * HOUR + 15);
    }
}
