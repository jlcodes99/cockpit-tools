//! Read-only source display metadata. Never routing, credential or official quota evidence.
use crate::modules::{chatgpt_plan, codex_account};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};

#[derive(Clone, serde::Serialize)]
pub(super) struct DisplayAccount {
    pub source_id: String,
    pub plan: Option<String>,
    pub quota: Option<crate::models::codex::CodexQuota>,
    pub usage_updated_at: Option<i64>,
}

impl DisplayAccount {
    pub(super) fn from_source(account: &crate::models::codex::CodexAccount) -> Self {
        let mut quota = account.quota.clone();
        if let Some(quota) = &mut quota {
            quota.raw_data = None;
        }
        Self {
            source_id: account.id.clone(),
            plan: account
                .plan_type
                .as_deref()
                .filter(|plan| !plan.trim().is_empty())
                .or(account
                    .auth_file_plan_type
                    .as_deref()
                    .filter(|plan| !plan.trim().is_empty()))
                .map(str::to_owned),
            quota,
            usage_updated_at: account.usage_updated_at,
        }
    }
}

#[derive(Clone)]
struct Confirmed {
    source: String,
    subject: String,
    client: String,
    display: DisplayAccount,
}

#[derive(Default)]
struct DisplayCache {
    revisions: HashMap<String, u64>,
    accounts: HashMap<String, Confirmed>,
}
static REVISION: AtomicU64 = AtomicU64::new(0);
static DISPLAY: LazyLock<Mutex<DisplayCache>> =
    LazyLock::new(|| Mutex::new(DisplayCache::default()));
static PENDING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
static WORKER: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));
static UPDATED: tokio::sync::Notify = tokio::sync::Notify::const_new();
const DISPLAY_WAIT: Duration = Duration::from_millis(200);

fn cached(wanted: &[String]) -> HashMap<String, DisplayAccount> {
    DISPLAY
        .lock()
        .map(|cache| {
            wanted
                .iter()
                .filter_map(|id| {
                    cache
                        .accounts
                        .get(id)
                        .map(|entry| (id.clone(), entry.display.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn update(
    cache: &mut HashMap<String, Confirmed>,
    wanted: &[String],
    grants: &[chatgpt_plan::ChatgptPlanAccountSummary],
    sources: Option<&[crate::models::codex::CodexAccount]>,
) -> HashMap<String, DisplayAccount> {
    let mut result = HashMap::new();
    for endpoint in wanted {
        let Some(id) = endpoint.strip_prefix(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX) else {
            continue;
        };
        let linked = grants
            .iter()
            .filter(|grant| grant.id == id)
            .collect::<Vec<_>>();
        let grant = linked.first().copied();
        let source = grant.and_then(|grant| grant.source_account_id.as_deref());
        if linked.len() != 1
            || source.is_none()
            || grant.is_some_and(|grant| grant.source_binding_error.is_some())
            || grants
                .iter()
                .filter(|grant| grant.source_account_id.as_deref() == source)
                .count()
                != 1
        {
            cache.remove(endpoint);
            continue;
        }
        let grant = grant.unwrap();
        let source = source.unwrap();
        if cache.get(endpoint).is_some_and(|saved| {
            saved.source != source
                || saved.subject != grant.subject
                || saved.client != grant.client_id
        }) {
            cache.remove(endpoint);
        }
        if grant.source_binding_valid {
            let Some(sources) = sources else {
                continue;
            };
            let account = sources.iter().find(|account| {
                account.id == source && !codex_account::is_chatgpt_plan_account(account)
            });
            if let Some(account) = account.filter(|account| !account.is_api_key_auth()) {
                cache.insert(
                    endpoint.clone(),
                    Confirmed {
                        source: source.into(),
                        subject: grant.subject.clone(),
                        client: grant.client_id.clone(),
                        display: DisplayAccount::from_source(account),
                    },
                );
            } else {
                cache.remove(endpoint);
            }
        }
        if let Some(saved) = cache.get(endpoint) {
            result.insert(endpoint.clone(), saved.display.clone());
        }
    }
    result
}

fn commit(
    cache: &mut DisplayCache,
    revision: u64,
    wanted: &[String],
    grants: &[chatgpt_plan::ChatgptPlanAccountSummary],
    sources: Option<&[crate::models::codex::CodexAccount]>,
) -> HashMap<String, DisplayAccount> {
    let current = wanted
        .iter()
        .filter(|id| revision >= cache.revisions.get(*id).copied().unwrap_or(0))
        .cloned()
        .collect::<Vec<_>>();
    update(&mut cache.accounts, &current, grants, sources);
    for id in current {
        cache.revisions.insert(id, revision);
    }
    wanted
        .iter()
        .filter_map(|id| {
            cache
                .accounts
                .get(id)
                .map(|entry| (id.clone(), entry.display.clone()))
        })
        .collect()
}

fn publish(
    revision: u64,
    wanted: &[String],
    grants: &[chatgpt_plan::ChatgptPlanAccountSummary],
    sources: Option<&[crate::models::codex::CodexAccount]>,
) {
    let changed = DISPLAY
        .lock()
        .map(|mut cache| {
            let before = serde_json::to_value(
                wanted
                    .iter()
                    .filter_map(|id| {
                        cache
                            .accounts
                            .get(id)
                            .map(|saved| (id.clone(), saved.display.clone()))
                    })
                    .collect::<HashMap<_, _>>(),
            )
            .ok();
            let after =
                serde_json::to_value(commit(&mut cache, revision, wanted, grants, sources)).ok();
            before != after
        })
        .unwrap_or(false);
    if changed {
        super::emit_local_access_state_updated();
    }
}

async fn refresh(wanted: &[String]) {
    let revision = REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    let grants = match chatgpt_plan::list_accounts().await {
        Ok(grants) => grants,
        Err(_) => {
            crate::modules::logger::log_warn("[API quota display] Registration metadata read failed; preserving confirmed display");
            return;
        }
    };
    // Apply binding errors/unlinks before slower source reads. A read failure
    // cannot keep a display association already invalidated by metadata.
    publish(revision, wanted, &grants, None);
    let ids = grants
        .iter()
        .filter(|grant| {
            grant.source_binding_valid
                && grant.source_binding_error.is_none()
                && grant.source_account_id.is_some()
                && grants.iter().filter(|other| other.id == grant.id).count() == 1
                && grants
                    .iter()
                    .filter(|other| other.source_account_id == grant.source_account_id)
                    .count()
                    == 1
                && wanted.contains(&format!(
                    "{}{}",
                    codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX,
                    grant.id
                ))
        })
        .filter_map(|grant| grant.source_account_id.clone())
        .collect::<HashSet<_>>();
    if ids.is_empty() {
        return;
    }
    let mut sources = Vec::new();
    let ids = ids.into_iter().collect::<Vec<_>>();
    for batch in ids.chunks(64) {
        match super::native_load_accounts(batch.to_vec()).await {
            Ok(mut accounts) => sources.append(&mut accounts),
            Err(_) => {
                crate::modules::logger::log_warn(
                    "[API quota display] Source metadata read failed; preserving confirmed display",
                );
                return;
            }
        }
        tokio::task::yield_now().await;
    }
    // A source read can race an identity edit. Pending revalidation retains
    // only an earlier confirmed snapshot, never these newly read source data.
    if let Ok(current) = chatgpt_plan::list_accounts().await {
        let stable = sources
            .into_iter()
            .filter(|source| {
                grants.iter().any(|old| {
                    old.source_binding_valid
                        && old.source_account_id.as_deref() == Some(source.id.as_str())
                        && current.iter().any(|new| {
                            new.id == old.id
                                && new.subject == old.subject
                                && new.client_id == old.client_id
                                && new.source_account_id == old.source_account_id
                        })
                })
            })
            .collect::<Vec<_>>();
        publish(revision, wanted, &current, Some(&stable));
    }
}

/// Bound the display wait, not the lifetime of a blocking disk/keychain worker.
/// Its permit stays held until actual reads finish, preventing duplicate readers.
pub(super) async fn load(
    accounts: &[crate::models::codex::CodexAccount],
) -> HashMap<String, DisplayAccount> {
    let wanted = accounts
        .iter()
        .filter(|account| codex_account::is_chatgpt_plan_account(account))
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    if wanted.is_empty() {
        return HashMap::new();
    }
    let changed = UPDATED.notified();
    tokio::pin!(changed);
    changed.as_mut().enable();
    let permit = if let Ok(mut pending) = PENDING.lock() {
        pending.extend(wanted.iter().cloned());
        WORKER.clone().try_acquire_owned().ok()
    } else {
        None
    };
    if let Some(permit) = permit {
        tokio::spawn(async move {
            loop {
                let wanted = {
                    let Ok(mut pending) = PENDING.lock() else {
                        return;
                    };
                    if pending.is_empty() {
                        // Release under the queue lock; callers cannot enqueue
                        // between the empty check and worker permit release.
                        drop(permit);
                        return;
                    }
                    pending.drain().collect::<Vec<_>>()
                };
                refresh(&wanted).await;
                UPDATED.notify_waiters();
                tokio::task::yield_now().await;
            }
        });
    }
    let _ = tokio::time::timeout(DISPLAY_WAIT, changed).await;
    cached(&wanted)
}

#[cfg(test)]
#[path = "codex_local_access_native_quota_plans_tests.rs"]
mod tests;
