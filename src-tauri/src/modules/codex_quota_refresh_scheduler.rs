//! Codex quota refresh scheduler.
//!
//! Keep the queue small and keyed by account ID. A duplicate refresh joins the
//! existing request instead of creating another network request or cloning a
//! full `CodexAccount`. The worker owns the operation and removes its entry on
//! every exit path, including timeout and panic.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use tokio::sync::{oneshot, Notify, Semaphore};

use crate::models::codex::CodexQuota;
use crate::modules::codex_account::CodexQuotaRuntimeSnapshot;

const MANUAL_CONCURRENCY: usize = 1;
const BACKGROUND_CONCURRENCY: usize = 1;
const MAX_BACKGROUND_PENDING_ACCOUNTS: usize = 16;
const MAX_MANUAL_PENDING_ACCOUNTS: usize = 16;
const MAX_WAITERS_PER_ACCOUNT: usize = 16;
const QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(120);

type RefreshResult = Result<CodexQuota, String>;
type Waiter = oneshot::Sender<RefreshResult>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefreshPriority {
    Manual,
    Background,
}

struct PendingRefresh {
    waiters: Vec<Waiter>,
    priority: RefreshPriority,
    started: bool,
    generation: u64,
}

static IN_FLIGHT: LazyLock<Mutex<HashMap<String, PendingRefresh>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_GENERATION: LazyLock<Mutex<u64>> = LazyLock::new(|| Mutex::new(0));
static MANUAL_PERMIT: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MANUAL_CONCURRENCY));
static BACKGROUND_PERMIT: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(BACKGROUND_CONCURRENCY));
static QUEUE_CHANGED: LazyLock<Notify> = LazyLock::new(Notify::new);
static BACKGROUND_EPOCH: AtomicU64 = AtomicU64::new(0);

pub(crate) fn background_epoch() -> u64 {
    BACKGROUND_EPOCH.load(Ordering::SeqCst)
}

fn message(key: &str) -> String {
    let locale = crate::modules::config::get_user_config().language;
    crate::modules::i18n::translate(&locale, &format!("common.quotaRefreshScheduler.{key}"), &[])
}

fn normalize_account_id(account_id: &str) -> Result<String, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err(message("invalidAccountId"));
    }
    Ok(account_id.to_string())
}

/// Queue one account refresh and await its shared result.
///
/// `runtime_snapshot` is used by batch refreshes so all accounts in that batch
/// reuse one process snapshot. If a duplicate request arrives with another
/// snapshot, the first queued request remains authoritative.
pub(crate) async fn refresh_account(
    account_id: &str,
    runtime_snapshot: Option<std::sync::Arc<CodexQuotaRuntimeSnapshot>>,
) -> RefreshResult {
    let account_id = normalize_account_id(account_id)?;
    cancel_queued_background_refreshes(Some(&account_id));
    refresh_account_with_priority(&account_id, runtime_snapshot, RefreshPriority::Manual, None)
        .await
}

pub(crate) async fn refresh_account_background(
    account_id: &str,
    runtime_snapshot: Option<std::sync::Arc<CodexQuotaRuntimeSnapshot>>,
) -> RefreshResult {
    refresh_account_with_priority(
        account_id,
        runtime_snapshot,
        RefreshPriority::Background,
        Some(background_epoch()),
    )
    .await
}

pub(crate) async fn refresh_account_with_priority(
    account_id: &str,
    runtime_snapshot: Option<std::sync::Arc<CodexQuotaRuntimeSnapshot>>,
    priority: RefreshPriority,
    epoch: Option<u64>,
) -> RefreshResult {
    refresh_with_operation(account_id, priority, epoch, |account_id| async move {
        Box::pin(crate::modules::codex_quota::refresh_account_quota_unqueued(
            &account_id,
            runtime_snapshot.as_deref(),
        ))
        .await
    })
    .await
}

async fn refresh_with_operation<F, Fut>(
    account_id: &str,
    priority: RefreshPriority,
    epoch: Option<u64>,
    operation: F,
) -> RefreshResult
where
    F: FnOnce(String) -> Fut + Send + 'static,
    Fut: Future<Output = RefreshResult> + Send + 'static,
{
    let account_id = normalize_account_id(account_id)?;
    let (receiver, leader, generation, changed) = {
        let (sender, receiver) = oneshot::channel();
        let mut pending = IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if priority == RefreshPriority::Background && epoch != Some(background_epoch()) {
            return Err(message("deferred"));
        }
        let manual_full = pending
            .values()
            .filter(|item| !item.started && item.priority == RefreshPriority::Manual)
            .count()
            >= MAX_MANUAL_PENDING_ACCOUNTS;
        if let Some(item) = pending.get_mut(&account_id) {
            item.waiters.retain(|waiter| !waiter.is_closed());
            if item.waiters.len() >= MAX_WAITERS_PER_ACCOUNT {
                return Err(message("busy"));
            }
            let promoted = priority == RefreshPriority::Manual
                && item.priority == RefreshPriority::Background
                && !item.started;
            if promoted && manual_full {
                return Err(message("busy"));
            }
            item.waiters.push(sender);
            if promoted {
                item.priority = RefreshPriority::Manual;
            }
            (receiver, false, item.generation, promoted)
        } else {
            let pending_count = pending
                .values()
                .filter(|item| !item.started && item.priority == priority)
                .count();
            let limit = match priority {
                RefreshPriority::Manual => MAX_MANUAL_PENDING_ACCOUNTS,
                RefreshPriority::Background => MAX_BACKGROUND_PENDING_ACCOUNTS,
            };
            if pending_count >= limit {
                return Err(message("busy"));
            }
            let generation = {
                let mut next = NEXT_GENERATION
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                *next = next.wrapping_add(1);
                *next
            };
            pending.insert(
                account_id.clone(),
                PendingRefresh {
                    waiters: vec![sender],
                    priority,
                    started: false,
                    generation,
                },
            );
            (receiver, true, generation, true)
        }
    };

    if changed {
        QUEUE_CHANGED.notify_waiters();
    }

    if leader {
        tauri::async_runtime::spawn(run_refresh(account_id, generation, operation));
    }

    receiver
        .await
        .unwrap_or_else(|_| Err(message("interrupted")))
}

async fn run_refresh<F, Fut>(account_id: String, generation: u64, operation: F)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = RefreshResult>,
{
    let result = async {
        let deadline = tokio::time::Instant::now() + QUEUE_WAIT_TIMEOUT;
        let permit = loop {
            // Register before inspecting state: a promotion/cancellation between
            // the state read and select must not be lost.
            let notified = QUEUE_CHANGED.notified();
            let priority = {
                let pending = IN_FLIGHT
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let Some(item) = pending.get(&account_id) else {
                    return Err(message("deferred"));
                };
                if item.generation != generation {
                    return Err(message("deferred"));
                }
                item.priority
            };
            let permit = match priority {
                RefreshPriority::Manual => {
                    tokio::time::timeout_at(deadline, MANUAL_PERMIT.acquire())
                        .await
                        .map_err(|_| message("timeout"))?
                        .map_err(|_| message("interrupted"))?
                }
                RefreshPriority::Background => {
                    tokio::select! {
                        result = BACKGROUND_PERMIT.acquire() => result
                            .map_err(|_| message("interrupted"))?,
                        _ = notified => continue,
                        _ = tokio::time::sleep_until(deadline) => {
                            return Err(message("timeout"));
                        }
                    }
                }
            };
            let still_current = {
                let mut pending = IN_FLIGHT
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match pending.get_mut(&account_id) {
                    Some(item) if item.generation == generation => {
                        if item.priority == priority {
                            item.started = true;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            };
            if still_current {
                break permit;
            }
            drop(permit);
        };

        // Construct the child inside the unwind boundary as well as polling it.
        let operation = Box::pin(async { operation(account_id.clone()).await });
        let result = execute_operation(operation, REFRESH_TIMEOUT).await;
        drop(permit);
        result
    }
    .await;

    let waiters = {
        let mut pending = IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match pending.get(&account_id) {
            Some(item) if item.generation == generation => pending
                .remove(&account_id)
                .map(|item| item.waiters)
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    };
    for waiter in waiters {
        let _ = waiter.send(result.clone());
    }
    QUEUE_CHANGED.notify_waiters();
}

async fn execute_operation(
    operation: impl Future<Output = RefreshResult>,
    timeout: Duration,
) -> RefreshResult {
    match tokio::time::timeout(timeout, AssertUnwindSafe(operation).catch_unwind()).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(message("interrupted")),
        Err(_) => Err(message("timeout")),
    }
}

/// Cancel queued background work when a user explicitly refreshes. Running
/// work is never aborted because it may already be inside token or file I/O.
pub(crate) fn cancel_queued_background_refreshes(except_account_id: Option<&str>) {
    let mut cancelled = Vec::new();
    {
        let mut pending = IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Invalidate not-yet-enqueued members of background batches too.
        BACKGROUND_EPOCH.fetch_add(1, Ordering::SeqCst);
        let ids = pending
            .iter()
            .filter(|(_, item)| item.priority == RefreshPriority::Background && !item.started)
            .filter(|(id, _)| except_account_id != Some(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(item) = pending.remove(&id) {
                cancelled.extend(item.waiters);
            }
        }
    }
    for waiter in cancelled {
        let _ = waiter.send(Err(message("deferred")));
    }
    QUEUE_CHANGED.notify_waiters();
}

#[cfg(test)]
#[path = "codex_quota_refresh_scheduler_tests.rs"]
mod tests;
