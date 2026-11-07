//! Account-scoped reset-card rules and a durable redemption journal.
use super::{atomic_write, codex_account, codex_quota, config, logger};
use crate::models::codex::CodexQuota;
use codex_quota::ResetCreditConsumeOutcome;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const FILE: &str = "codex_reset_automation.json";
const EVENT: &str = "codex:reset-automation-updated";
static STORAGE: Mutex<()> = Mutex::new(());
static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static APP: OnceLock<AppHandle> = OnceLock::new();

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Rule {
    pub enabled: bool,
    pub expiry_enabled: bool,
    pub before_seconds: u64,
    pub quota_enabled: bool,
    pub remaining_percent: u8,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            enabled: false,
            expiry_enabled: true,
            before_seconds: 3600,
            quota_enabled: true,
            remaining_percent: 5,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum AccountRule {
    #[default]
    Inherit,
    Off,
    Custom {
        rule: Rule,
    },
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
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Document {
    revision: u64,
    public_rule: Rule,
    accounts: HashMap<String, AccountRule>,
    pending: HashMap<String, Pending>,
    last_evidence: HashMap<String, String>,
    records: Vec<Record>,
}
#[derive(Clone, Serialize)]
pub struct View {
    pub revision: u64,
    pub public_rule: Rule,
    pub accounts: HashMap<String, AccountRule>,
    pub records: Vec<Record>,
}
impl From<Document> for View {
    fn from(d: Document) -> Self {
        Self {
            revision: d.revision,
            public_rule: d.public_rule,
            accounts: d.accounts,
            records: d.records,
        }
    }
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
fn read(path: &Path) -> Result<Document, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", message("storageError")))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Document::default()),
        Err(e) => Err(format!("{}: {e}", message("storageError"))),
    }
}
fn update_at<T>(
    path: &Path,
    edit: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = STORAGE.lock().map_err(|_| message("storageError"))?;
    let mut d = read(path)?;
    d.revision = d
        .revision
        .checked_add(1)
        .ok_or_else(|| message("storageError"))?;
    let result = edit(&mut d)?;
    let bytes = serde_json::to_string_pretty(&d).map_err(|e| e.to_string())?;
    atomic_write::write_string_atomic(path, &bytes)?;
    Ok(result)
}
fn update<T>(edit: impl FnOnce(&mut Document) -> Result<T, String>) -> Result<T, String> {
    let result = update_at(&path()?, edit)?;
    if let Some(app) = APP.get() {
        let _ = app.emit(EVENT, ());
    }
    Ok(result)
}
pub fn view() -> Result<View, String> {
    let _guard = STORAGE.lock().map_err(|_| message("storageError"))?;
    Ok(read(&path()?)?.into())
}
fn validate(rule: &Rule) -> Result<(), String> {
    if rule.expiry_enabled && (rule.before_seconds == 0 || rule.before_seconds > i64::MAX as u64) {
        return Err(message("invalidTime"));
    }
    if rule.remaining_percent > 100 {
        return Err(message("invalidPercent"));
    }
    Ok(())
}
fn set_public(d: &mut Document, rule: Rule, expected: &Rule) -> Result<(), String> {
    validate(&rule)?;
    if &d.public_rule != expected {
        return Err(message("conflict"));
    }
    d.public_rule = rule;
    Ok(())
}
fn set_account(
    d: &mut Document,
    id: &str,
    rule: AccountRule,
    expected: &AccountRule,
) -> Result<(), String> {
    if let AccountRule::Custom { rule } = &rule {
        validate(rule)?;
    }
    if d.accounts.get(id).unwrap_or(&AccountRule::Inherit) != expected {
        return Err(message("conflict"));
    }
    if rule == AccountRule::Inherit {
        d.accounts.remove(id);
    } else {
        d.accounts.insert(id.into(), rule);
    }
    Ok(())
}
fn set_public_for_accounts(
    d: &mut Document,
    rule: Rule,
    expected: &Rule,
    ids: &[String],
    expected_accounts: &HashMap<String, AccountRule>,
) -> Result<(), String> {
    // Compare only rule settings, not journal revisions changed by background work.
    if ids.iter().any(|id| {
        d.accounts.get(id).unwrap_or(&AccountRule::Inherit)
            != expected_accounts.get(id).unwrap_or(&AccountRule::Inherit)
    }) {
        return Err(message("conflict"));
    }
    set_public(d, rule.clone(), expected)?;
    for id in ids {
        d.accounts
            .insert(id.clone(), AccountRule::Custom { rule: rule.clone() });
    }
    Ok(())
}
pub fn save_public(
    rule: Rule,
    expected: Rule,
    apply_to_all: bool,
    expected_accounts: Option<HashMap<String, AccountRule>>,
) -> Result<View, String> {
    if apply_to_all {
        let expected_accounts = expected_accounts.ok_or_else(|| message("conflict"))?;
        // The account mutation lock keeps deletion/import from changing the target set
        // between the authoritative index snapshot and the atomic settings write.
        codex_account::with_reset_automation_account_ids(|ids| {
            update(|d| {
                set_public_for_accounts(d, rule, &expected, ids, &expected_accounts)?;
                Ok(d.clone().into())
            })
        })
    } else {
        update(|d| {
            set_public(d, rule, &expected)?;
            Ok(d.clone().into())
        })
    }
}
pub fn save_account(id: &str, rule: AccountRule, expected: AccountRule) -> Result<View, String> {
    // Saving settings never creates or revives an account.
    if codex_account::load_account(id).is_none() {
        return Err(message("accountMissing"));
    }
    update(|d| {
        set_account(d, id, rule, &expected)?;
        Ok(d.clone().into())
    })
}
fn effective<'a>(d: &'a Document, id: &str) -> Option<&'a Rule> {
    let rule = match d.accounts.get(id) {
        None | Some(AccountRule::Inherit) => &d.public_rule,
        Some(AccountRule::Off) => return None,
        Some(AccountRule::Custom { rule }) => rule,
    };
    (rule.enabled && (rule.expiry_enabled || rule.quota_enabled)).then_some(rule)
}
fn trigger(
    rule: &Rule,
    quota: Option<&CodexQuota>,
    cards: &codex_quota::CodexResetCreditsSnapshot,
    now: i64,
) -> Option<&'static str> {
    if !rule.enabled {
        return None;
    }
    if cards.available_count.is_none_or(|count| count <= 0) {
        return None;
    }
    if rule.expiry_enabled
        && cards
            .next_expires_at
            .is_some_and(|at| at > now && at.saturating_sub(now) <= rule.before_seconds as i64)
    {
        return Some("expiry");
    }
    if !rule.quota_enabled {
        return None;
    }
    let q = quota?;
    let low = |value: i32, present: Option<bool>, reset: Option<i64>| {
        present != Some(false)
            && (present == Some(true) || reset.is_some())
            && reset.is_none_or(|at| at > now)
            && (0..=100).contains(&value)
            && value <= i32::from(rule.remaining_percent)
    };
    // Legacy `window` JSON fields are obsolete and ignored by serde. Preserve
    // every threshold/switch while applying the requested weekly-only behavior.
    low(
        q.weekly_percentage,
        q.weekly_window_present,
        q.weekly_reset_time,
    )
    .then_some("quota")
}
fn evidence(cards: &codex_quota::CodexResetCreditsSnapshot) -> Result<String, String> {
    // Sort identities, so changing response order is not a new observation.
    let mut identities: Vec<_> = cards
        .credits
        .iter()
        .map(|c| {
            serde_json::json!([c.id, c.status, c.raw_status, c.expires_at, c.redeemed_at])
                .to_string()
        })
        .collect();
    identities.sort();
    serde_json::to_string(&(cards.available_count, cards.next_expires_at, identities))
        .map_err(|e| e.to_string())
}
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
    d.records.truncate(100);
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
fn begin_redemption(
    d: &mut Document,
    id: &str,
    identity: &str,
    reason: &str,
    before: Option<String>,
) -> Pending {
    if let Some(pending) = d
        .pending
        .get(id)
        .filter(|pending| pending.identity == identity)
    {
        return pending.clone();
    }
    let pending = Pending {
        identity: identity.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        reason: reason.into(),
        evidence: before,
    };
    d.pending.insert(id.into(), pending.clone());
    record(d, id, reason, "pending", None);
    pending
}
fn finish_redemption(
    d: &mut Document,
    id: &str,
    pending: &Pending,
    result: Result<ResetCreditConsumeOutcome, String>,
) -> Result<(), String> {
    if d.pending
        .get(id)
        .is_none_or(|current| current.request_id != pending.request_id)
    {
        return Err(message("conflict"));
    }
    // A known no-op closes this attempt without marking the card used. An
    // ambiguous response or transport failure keeps the same idempotency key.
    if result.is_ok() {
        d.pending.remove(id);
    }
    let result = result.and_then(ResetCreditConsumeOutcome::into_result);
    if result.is_ok() {
        if let Some(e) = &pending.evidence {
            d.last_evidence.insert(id.into(), e.clone());
        }
    }
    record(
        d,
        id,
        &pending.reason,
        if result.is_ok() { "success" } else { "failed" },
        result.err(),
    );
    Ok(())
}
async fn redeem(
    id: &str,
    reason: &str,
    before: Option<String>,
    expected: Option<(Rule, String)>,
) -> Result<(), String> {
    let account_id = id.to_string();
    let reason = reason.to_string();
    let pending = blocking(move || {
        let account =
            codex_account::load_account(&account_id).ok_or_else(|| message("accountMissing"))?;
        let identity = account_identity(&account);
        update(|d| {
            // A rule changed or was disabled while fresh network evidence was loading.
            // Manual redemption has no automatic-rule prerequisites.
            if expected.as_ref().is_some_and(|(rule, owner)| {
                effective(d, &account_id) != Some(rule) || owner != &identity
            }) {
                return Err(message("conflict"));
            }
            Ok(begin_redemption(d, &account_id, &identity, &reason, before))
        })
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
    let saved_result = result.clone();
    blocking(move || update(|d| finish_redemption(d, &id, &pending, saved_result))).await?;
    result.and_then(ResetCreditConsumeOutcome::into_result)
}
pub async fn consume_manual(id: &str) -> Result<(), String> {
    let _lease = acquire(id)?.ok_or_else(|| message("busy"))?;
    redeem(id, "manual", None, None).await
}
async fn check_account(id: &str) -> Result<(), String> {
    let Some(_lease) = acquire(id)? else {
        return Ok(());
    };
    let id_owned = id.to_string();
    let rule = blocking(move || {
        let Some(_resource) = super::background_maintenance::try_resource() else {
            return Ok(None);
        };
        let d = read(&path()?)?;
        let Some(rule) = effective(&d, &id_owned).cloned() else {
            return Ok(None);
        };
        let account =
            codex_account::load_account(&id_owned).ok_or_else(|| message("accountMissing"))?;
        if account.is_api_key_auth()
            || account.is_agent_identity_auth()
            || account.is_web_session_auth()
        {
            return Ok(None);
        }
        Ok(Some((rule, account_identity(&account))))
    })
    .await?;
    let Some((rule, identity)) = rule else {
        return Ok(());
    };
    // Always obtain fresh evidence; unknown/missing windows are never zero.
    let cards = codex_quota::fetch_account_reset_credits(id).await?;
    let quota = if rule.quota_enabled
        && trigger(&rule, None, &cards, chrono::Utc::now().timestamp()).is_none()
        && cards.available_count.is_some_and(|count| count > 0)
    {
        Some(Box::pin(codex_quota::refresh_account_quota_background(id)).await?)
    } else {
        None
    };
    let observed = format!("{}:{}", identity, evidence(&cards)?);
    let id_owned = id.to_string();
    let observed_copy = observed.clone();
    let reason = blocking(move || {
        let d = read(&path()?)?;
        let current =
            codex_account::load_account(&id_owned).ok_or_else(|| message("accountMissing"))?;
        if account_identity(&current) != identity {
            return Err(message("conflict"));
        }
        let Some(rule) = effective(&d, &id_owned) else {
            return Ok(None);
        };
        if d.last_evidence.get(&id_owned) == Some(&observed_copy) {
            return Ok(None);
        }
        Ok(
            trigger(rule, quota.as_ref(), &cards, chrono::Utc::now().timestamp())
                .map(|reason| (reason.to_string(), (rule.clone(), identity))),
        )
    })
    .await?;
    if let Some((reason, rule)) = reason {
        redeem(id, &reason, Some(observed), Some(rule)).await?;
        Box::pin(codex_quota::refresh_account_quota_background(id)).await?;
    }
    Ok(())
}
pub async fn run(app: AppHandle) {
    if APP.set(app).is_err() {
        return;
    }
    tokio::time::sleep(Duration::from_secs(10)).await;
    while !super::app_lifecycle::is_shutdown_started() {
        if super::tray_layout::is_platform_enabled("codex") {
            let ids = blocking(|| {
                let Some(_resource) = super::background_maintenance::try_resource() else {
                    return Ok(Vec::new());
                };
                let d = read(&path()?)?;
                if !d.public_rule.enabled
                    && !d
                        .accounts
                        .values()
                        .any(|r| matches!(r, AccountRule::Custom { rule } if rule.enabled))
                {
                    return Ok(Vec::new());
                }
                Ok(codex_account::load_account_index()
                    .accounts
                    .into_iter()
                    .filter(|a| effective(&d, &a.id).is_some())
                    .map(|a| a.id)
                    .collect::<Vec<_>>())
            })
            .await;
            match ids {
                Ok(ids) => {
                    for id in ids {
                        if super::app_lifecycle::is_shutdown_started()
                            || !super::tray_layout::is_platform_enabled("codex")
                        {
                            break;
                        }
                        let result =
                            tokio::time::timeout(Duration::from_secs(180), check_account(&id))
                                .await
                                .unwrap_or_else(|_| {
                                    Err(super::i18n::translate(
                                        &config::get_user_config().language,
                                        "common.configSaveTimeout",
                                        &[],
                                    ))
                                });
                        if let Err(error) = result {
                            logger::log_warn(&format!(
                                "[Codex Reset Automation] account={id}, {error}"
                            ));
                            if let Err(storage_error) = blocking(move || {
                                update(|d| {
                                    if !d.records.first().is_some_and(|last| {
                                        last.account_id == id
                                            && last.status == "failed"
                                            && last.detail.as_ref() == Some(&error)
                                    }) {
                                        record(d, &id, "check", "failed", Some(error));
                                    }
                                    Ok(())
                                })
                            })
                            .await
                            {
                                logger::log_warn(&format!("[Codex Reset Automation] activity persistence failed: {storage_error}"));
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                }
                Err(error) => logger::log_warn(&format!("[Codex Reset Automation] {error}")),
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
#[cfg(test)]
#[path = "codex_reset_automation_tests.rs"]
mod tests;
