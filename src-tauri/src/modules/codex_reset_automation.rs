//! Manual reset-card redemption and durable local usage history.
use super::{codex_account, codex_quota, config, logger};
#[cfg(test)]
use crate::models::codex::CodexQuota;
use codex_quota::{ResetCreditConsumeOutcome, ResetCreditReceipt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tauri::Emitter;

#[path = "codex_reset_journal_store.rs"]
mod journal;
#[path = "codex_reset_journal_legacy.rs"]
mod legacy;
#[path = "codex_reset_usage_history.rs"]
mod usage_history;
pub use usage_history::UsagePage;

pub(crate) const FILE: &str = "codex_reset_automation.json";
const EVENT: &str = "codex:reset-automation-updated";
static STORAGE: Mutex<()> = Mutex::new(());
static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Retained for read-only decoding of historical automatic attempt records.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub expiry_enabled: bool,
    pub before_seconds: u64,
    pub quota_enabled: bool,
    pub remaining_percent: u8,
    pub prefer_usage_credits: bool,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: false,
            expiry_enabled: true,
            before_seconds: 3600,
            quota_enabled: true,
            remaining_percent: 5,
            prefer_usage_credits: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub account_id: String,
    pub at: i64,
    pub reason: String,
    pub status: String,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    identity: String,
    request_id: String,
    reason: String,
    evidence: Option<String>,
    #[serde(default)]
    unconfirmed_reported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<AttemptContext>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AttemptContext {
    rule: Rule,
    before: ResetObservation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResetObservation {
    available_count: Option<i64>,
    next_expires_at: Option<i64>,
    available_card_ids: Vec<String>,
    hourly: Option<WindowObservation>,
    weekly: Option<WindowObservation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WindowObservation {
    remaining: i32,
    resets_at: Option<i64>,
}

#[cfg(test)]
impl ResetObservation {
    fn from_quota(quota: Option<&CodexQuota>, now: i64) -> Self {
        let window = |remaining, present, resets_at: Option<i64>| {
            (present != Some(false)
                && (present == Some(true) || resets_at.is_some())
                && resets_at.is_none_or(|at| at > now)
                && (0..=100).contains(&remaining))
            .then_some(WindowObservation {
                remaining,
                resets_at,
            })
        };
        Self {
            available_count: quota.and_then(|q| q.reset_credits_available),
            next_expires_at: None,
            available_card_ids: Vec::new(),
            hourly: quota.and_then(|q| {
                window(
                    q.hourly_percentage,
                    q.hourly_window_present,
                    q.hourly_reset_time,
                )
            }),
            weekly: quota.and_then(|q| {
                window(
                    q.weekly_percentage,
                    q.weekly_window_present,
                    q.weekly_reset_time,
                )
            }),
        }
    }

}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Document {
    revision: u64,
    pending: HashMap<String, Pending>,
    records: Vec<Record>,
    // Only live redemption transactions populate this. Legacy records are not backfilled.
    #[serde(skip)]
    usage_updates: Vec<usage_history::UsageUpdate>,
}
pub(crate) fn message(key: &str) -> String {
    super::i18n::translate(
        &config::get_user_config().language,
        &format!("codex.resetAutomation.{key}"),
        &[],
    )
}
fn path() -> Result<PathBuf, String> {
    Ok(config::get_data_dir()?.join(FILE))
}

fn publish_update() {
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit(EVENT, ());
    }
}

fn update<T>(
    account_id: &str,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    let path = path()?;
    let result = journal::update_account(&path, account_id, edit)?;
    journal::schedule(&path);
    publish_update();
    Ok(result)
}
#[cfg(test)]
fn record(d: &mut Document, id: &str, reason: &str, status: &str, detail: Option<String>) {
    d.records.insert(
        0,
        Record {
            id: uuid::Uuid::new_v4().to_string(),
            account_id: id.into(),
            at: chrono::Utc::now().timestamp(),
            reason: reason.into(),
            status: status.into(),
            detail,
        },
    );
    journal::retain_activity_records(&mut d.records);
}
pub(crate) struct Lease(String);
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut active) = ACTIVE.lock() {
            active.remove(&self.0);
        }
    }
}
pub(crate) fn acquire(id: &str) -> Result<Option<Lease>, String> {
    if !ACTIVE
        .lock()
        .map_err(|_| message("busy"))?
        .insert(id.into())
    {
        return Ok(None);
    }
    Ok(Some(Lease(id.into())))
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}
pub(crate) fn account_identity(account: &crate::models::codex::CodexAccount) -> String {
    serde_json::json!([account.user_id, account.account_id, account.organization_id]).to_string()
}
fn begin_redemption(d: &mut Document, id: &str, identity: &str) -> Pending {
    if let Some(pending) = d
        .pending
        .get(id)
        .filter(|pending| pending.identity == identity)
    {
        // Reuse the saved request_id (and keep its original reason/context so
        // legacy unconfirmed automatic attempts retain their source and context
        // once resolved, e.g. AlreadyRedeemed, instead of being recorded as manual).
        let pending = pending.clone();
        d.usage_updates.push(usage_history::UsageUpdate::new(
            id, &pending, "pending", None,
        ));
        return pending;
    }
    let pending = Pending {
        identity: identity.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        reason: "manual".into(),
        evidence: None,
        unconfirmed_reported: false,
        context: None,
    };
    d.pending.insert(id.into(), pending.clone());
    d.usage_updates.push(usage_history::UsageUpdate::new(
        id, &pending, "pending", None,
    ));
    pending
}
fn finish_redemption(
    d: &mut Document,
    id: &str,
    pending: &Pending,
    result: Result<ResetCreditConsumeOutcome, String>,
    receipt: Option<ResetCreditReceipt>,
) -> Result<(), String> {
    let _current = d
        .pending
        .get(id)
        .filter(|current| current.request_id == pending.request_id)
        .ok_or_else(|| message("conflict"))?;
    // A known no-op closes this attempt without marking the card used. An
    // ambiguous response or transport failure keeps the same idempotency key.
    let outcome = match &result {
        Ok(ResetCreditConsumeOutcome::Reset) => "reset",
        Ok(ResetCreditConsumeOutcome::AlreadyRedeemed) => "already_redeemed",
        Ok(ResetCreditConsumeOutcome::NothingToReset) => "nothing_to_reset",
        Ok(ResetCreditConsumeOutcome::NoCredit) => "no_credit",
        Err(_) => "unconfirmed",
    };
    let mut usage =
        usage_history::UsageUpdate::new(id, pending, outcome, result.as_ref().err().cloned());
    usage.receipt = receipt;
    d.usage_updates.push(usage);
    if result.is_ok() {
        d.pending.remove(id);
    }
    Ok(())
}
async fn redeem(id: &str) -> Result<(), String> {
    let account_id = id.to_string();
    let pending = blocking(move || {
        let account =
            codex_account::load_account(&account_id).ok_or_else(|| message("accountMissing"))?;
        let identity = account_identity(&account);
        update(&account_id, |d| Ok(begin_redemption(d, &account_id, &identity)))
    })
    .await?;
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        codex_quota::consume_reset_credit_with_request_id(
            id,
            &pending.request_id,
            &pending.identity,
        ),
    )
    .await
    .unwrap_or_else(|_| Err(message("timeout")));
    let id = id.to_string();
    let receipt = result
        .as_ref()
        .ok()
        .and_then(|response| response.receipt.clone());
    let result = result.map(|response| response.outcome);
    let saved_result = result.clone();
    blocking(move || {
        update(&id, |d| {
            finish_redemption(d, &id, &pending, saved_result, receipt)
        })
    })
    .await?;
    result.and_then(ResetCreditConsumeOutcome::into_result)
}
pub async fn consume_manual(id: &str) -> Result<(), String> {
    let _lease = acquire(id)?.ok_or_else(|| message("busy"))?;
    redeem(id).await
}
pub fn usage_history(id: &str, cursor: Option<i64>) -> Result<UsagePage, String> {
    let account = codex_account::load_account(id).ok_or_else(|| message("accountMissing"))?;
    journal::usage_history(&path()?, id, &account_identity(&account), cursor)
}
fn schedule_migration_at(path: &Path) -> Result<bool, String> {
    if path.try_exists().map_err(|error| error.to_string())? {
        journal::schedule(path);
        Ok(true)
    } else {
        Ok(false)
    }
}
pub(crate) fn schedule_migration() {
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_secs(10)).await;
        if let Err(error) = blocking(|| {
            schedule_migration_at(&path()?)?;
            Ok(())
        })
        .await
        {
            logger::log_warn(&format!("[Codex Reset Migration] {error}"));
        }
    });
}
#[cfg(test)]
#[path = "codex_reset_automation_tests.rs"]
mod tests;
