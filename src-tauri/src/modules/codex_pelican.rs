//! User-triggered, account-isolated HTML generation. No startup work or automatic retries.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Notify, Semaphore};

#[path = "codex_pelican_store.rs"]
mod store;

const EVENT: &str = "codex://pelican-progress";
const PREVIEW_CHARS: usize = 1600;
const MAX_BATCH_ACCOUNTS: usize = 200;
const MAX_CONCURRENCY: usize = 10;
const STREAM_EMIT_INTERVAL: Duration = Duration::from_millis(150);
const IO_TIMEOUT: Duration = Duration::from_secs(30);
static ACTIVE: LazyLock<Mutex<Option<Arc<ActiveBatch>>>> = LazyLock::new(|| Mutex::new(None));
static MAINTENANCE: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub account_ids: Vec<String>,
    pub prompt: String,
    pub model: String,
    pub effort: String,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
}

fn default_concurrency() -> usize {
    3
}

fn validate_concurrency(concurrency: usize) -> Result<(), String> {
    if !(1..=MAX_CONCURRENCY).contains(&concurrency) {
        return Err("pelican.error.invalidConcurrency".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub id: String,
    pub revision: u64,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub prompt: String,
    pub model: String,
    pub effort: String,
    pub concurrency: usize,
    pub transport: String,
    pub delivery_instructions: String,
    pub items: Vec<Item>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub account_id: String,
    pub account_email: String,
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub reply_preview: Option<String>,
    pub has_html: bool,
    pub error: Option<String>,
    pub usage: Option<serde_json::Value>,
    pub response_id: Option<String>,
    pub response_model: Option<String>,
    pub quota: Option<QuotaUsage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaUsage {
    pub consumed_percent: Option<f64>,
    pub before_remaining_percent: Option<i32>,
    pub remaining_percent: Option<i32>,
    pub window_minutes: Option<i64>,
    pub reset_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub raw_reply: String,
    pub html: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub items: Vec<Batch>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionSettings {
    pub days: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSummary {
    pub deleted_count: usize,
}

struct ActiveBatch {
    batch: Mutex<Batch>,
    last_stream_emit: Mutex<Instant>,
    retry_gate: Mutex<()>,
    accepting_retries: AtomicBool,
    pending_retries: AtomicUsize,
    retry_done: Notify,
    retry_semaphore: Mutex<Option<Arc<Semaphore>>>,
    cancel: watch::Sender<bool>,
    done: watch::Sender<bool>,
    app: AppHandle,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lock_error() -> String {
    "pelican.error.stateUnavailable".into()
}

fn same_pelican_quota_window(
    before: &crate::modules::codex_local_access::PelicanQuotaSnapshot,
    after: &crate::modules::codex_local_access::PelicanQuotaSnapshot,
) -> bool {
    if let (Some(before_window), Some(after_window)) =
        (before.window_minutes, after.window_minutes)
    {
        if before_window != after_window {
            return false;
        }
    }
    match (before.reset_at, after.reset_at) {
        (Some(before_reset), Some(after_reset)) => after_reset + 60 >= before_reset,
        _ => true,
    }
}

fn pelican_quota_usage(
    before: Option<&crate::modules::codex_local_access::PelicanQuotaSnapshot>,
    after: Option<&crate::modules::codex_local_access::PelicanQuotaSnapshot>,
) -> Option<QuotaUsage> {
    if before.is_none() && after.is_none() {
        return None;
    }
    let consumed_percent = match (before, after) {
        (Some(before), Some(after)) if same_pelican_quota_window(before, after) => {
            let consumed = before.remaining_percent as f64 - after.remaining_percent as f64;
            (consumed >= 0.0).then_some(consumed)
        }
        _ => None,
    };
    Some(QuotaUsage {
        consumed_percent,
        before_remaining_percent: before.map(|snapshot| snapshot.remaining_percent),
        remaining_percent: after.map(|snapshot| snapshot.remaining_percent),
        window_minutes: after
            .and_then(|snapshot| snapshot.window_minutes)
            .or_else(|| before.and_then(|snapshot| snapshot.window_minutes)),
        reset_at: after.and_then(|snapshot| snapshot.reset_at),
    })
}

fn take_stream_emit_slot(last_emit: &mut Instant, current: Instant) -> bool {
    if current.saturating_duration_since(*last_emit) < STREAM_EMIT_INTERVAL {
        return false;
    }
    *last_emit = current;
    true
}

impl ActiveBatch {
    fn snapshot(&self) -> Result<Batch, String> {
        self.batch
            .lock()
            .map(|batch| batch.clone())
            .map_err(|_| lock_error())
    }

    fn update(&self, change: impl FnOnce(&mut Batch)) -> Result<Batch, String> {
        let snapshot = {
            let mut batch = self.batch.lock().map_err(|_| lock_error())?;
            change(&mut batch);
            batch.revision += 1;
            batch.clone()
        };
        if let Err(error) = self.app.emit(EVENT, &snapshot) {
            crate::modules::logger::log_warn(&format!("[Pelican] progress event: {error}"));
        }
        Ok(snapshot)
    }

    fn update_stream_preview(&self, index: usize, preview: String) -> Result<(), String> {
        let snapshot = {
            let mut batch = self.batch.lock().map_err(|_| lock_error())?;
            batch.items[index].reply_preview = Some(preview);
            batch.revision += 1;
            // Stream updates share one batch-wide budget. The clock is only locked here,
            // after the batch lock; skipped emissions never clone the whole batch.
            let mut last_emit = self.last_stream_emit.lock().map_err(|_| lock_error())?;
            if !take_stream_emit_slot(&mut last_emit, Instant::now()) {
                return Ok(());
            }
            batch.clone()
        };
        if let Err(error) = self.app.emit(EVENT, &snapshot) {
            crate::modules::logger::log_warn(&format!("[Pelican] progress event: {error}"));
        }
        Ok(())
    }

    fn storage_error(&self, error: String) {
        crate::modules::logger::log_error(&format!("[Pelican] persistence: {error}"));
        let _ = self.update(|batch| batch.error = Some(error));
    }
}

pub fn start(app: AppHandle, mut request: StartRequest) -> Result<Batch, String> {
    let mut seen = HashSet::new();
    request
        .account_ids
        .retain(|id| !id.trim().is_empty() && seen.insert(id.clone()));
    if request.account_ids.is_empty() || request.account_ids.len() > MAX_BATCH_ACCOUNTS {
        return Err("pelican.error.accountsRequired".into());
    }
    if request.account_ids.iter().any(|id| !valid_account_id(id)) {
        return Err("pelican.error.invalidAccount".into());
    }
    if request.prompt.trim().is_empty()
        || request.prompt.len() > 100_000
        || request.model.trim().is_empty()
        || request.model.len() > 200
        || !matches!(
            request.effort.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        )
    {
        return Err("pelican.error.invalidRequest".into());
    }
    validate_concurrency(request.concurrency)?;
    let batch = Batch {
        id: uuid::Uuid::new_v4().to_string(),
        revision: 1,
        created_at: now(),
        finished_at: None,
        status: "running".into(),
        prompt: request.prompt,
        model: request.model,
        effort: request.effort,
        concurrency: request.concurrency,
        transport: "direct-chat".into(),
        error: None,
        delivery_instructions: crate::modules::codex_local_access::PELICAN_DELIVERY_INSTRUCTIONS
            .into(),
        items: request
            .account_ids
            .into_iter()
            .map(|account_id| Item {
                id: uuid::Uuid::new_v4().to_string(),
                account_email: account_id.clone(),
                account_id,
                status: "queued".into(),
                started_at: None,
                finished_at: None,
                reply_preview: None,
                has_html: false,
                error: None,
                usage: None,
                response_id: None,
                response_model: None,
                quota: None,
            })
            .collect(),
    };
    let (cancel, _) = watch::channel(false);
    let (done, _) = watch::channel(false);
    let state = Arc::new(ActiveBatch {
        batch: Mutex::new(batch.clone()),
        last_stream_emit: Mutex::new(Instant::now()),
        retry_gate: Mutex::new(()),
        accepting_retries: AtomicBool::new(true),
        pending_retries: AtomicUsize::new(0),
        retry_done: Notify::new(),
        retry_semaphore: Mutex::new(None),
        cancel,
        done,
        app,
    });
    {
        let mut active = ACTIVE.lock().map_err(|_| lock_error())?;
        // Keep completed results reachable until dismissed; never replace a running batch.
        if active.as_ref().is_some_and(|job| !*job.done.borrow()) {
            return Err("pelican.error.alreadyRunning".into());
        }
        *active = Some(state.clone());
    }
    tauri::async_runtime::spawn(async move {
        run_batch(state).await;
    });
    Ok(batch)
}

pub async fn retry(app: AppHandle, batch_id: String, item_id: String) -> Result<Batch, String> {
    let _maintenance = MAINTENANCE.lock().await;
    if let Some(job) = find_active(&batch_id)? {
        let mut wait_for_done = None;
        let retry = {
            let _gate = job.retry_gate.lock().map_err(|_| lock_error())?;
            let mut batch = job.batch.lock().map_err(|_| lock_error())?;
            let Some(index) = batch.items.iter().position(|item| item.id == item_id) else {
                return Err("pelican.error.artifactMissing".into());
            };
            if !matches!(
                batch.items[index].status.as_str(),
                "failed" | "cancelled" | "interrupted"
            ) {
                return Err("pelican.error.invalidRequest".into());
            }
            if *job.done.borrow()
                || *job.cancel.borrow()
                || !job.accepting_retries.load(Ordering::Acquire)
            {
                wait_for_done = Some(job.done.subscribe());
                None
            } else {
                prepare_item_retry(&mut batch, &item_id)?;
                job.pending_retries.fetch_add(1, Ordering::AcqRel);
                Some((index, batch.clone()))
            }
        };
        if let Some((index, snapshot)) = retry {
            if let Err(error) = job.app.emit(EVENT, &snapshot) {
                crate::modules::logger::log_warn(&format!("[Pelican] progress event: {error}"));
            }
            if let Err(error) = persist(&job).await {
                job.pending_retries.fetch_sub(1, Ordering::AcqRel);
                job.retry_done.notify_one();
                finish_failed(&job, index, error.clone()).await;
                return Err(error);
            }
            let semaphore = job
                .retry_semaphore
                .lock()
                .map_err(|_| lock_error())?
                .clone()
                .unwrap_or_else(|| Arc::new(Semaphore::new(1)));
            let worker = job.clone();
            tauri::async_runtime::spawn(async move {
                let attempt = worker.clone();
                let result = tokio::spawn(async move {
                    let batch = attempt.snapshot()?;
                    let item = batch
                        .items
                        .get(index)
                        .cloned()
                        .ok_or_else(|| "pelican.error.artifactMissing".to_string())?;
                    store::save_artifact(
                        batch.id,
                        item.id.clone(),
                        Artifact {
                            raw_reply: String::new(),
                            html: None,
                        },
                    )
                    .await?;
                    run_item(attempt, semaphore, index, item).await;
                    Ok::<(), String>(())
                })
                .await
                .map_err(|error| format!("pelican.error.workerFailed: {error}"))
                .and_then(|result| result);
                if let Err(error) = result {
                    finish_failed(&worker, index, error).await;
                }
                worker.pending_retries.fetch_sub(1, Ordering::AcqRel);
                worker.retry_done.notify_one();
            });
            return Ok(snapshot);
        }
        if let Some(mut done) = wait_for_done {
            if !*done.borrow() {
                tokio::time::timeout(Duration::from_secs(30), async {
                    while !*done.borrow() {
                        done.changed().await.map_err(|_| lock_error())?;
                    }
                    Ok::<(), String>(())
                })
                .await
                .map_err(|_| "pelican.error.storageTimeout".to_string())??;
            }
        }
    }
    let batch = get(batch_id.clone()).await?;
    let mut retry_batch = batch;
    let index = prepare_item_retry(&mut retry_batch, &item_id)?;
    let (cancel, _) = watch::channel(false);
    let (done, _) = watch::channel(false);
    let state = Arc::new(ActiveBatch {
        batch: Mutex::new(retry_batch.clone()),
        last_stream_emit: Mutex::new(Instant::now()),
        retry_gate: Mutex::new(()),
        accepting_retries: AtomicBool::new(true),
        pending_retries: AtomicUsize::new(0),
        retry_done: Notify::new(),
        retry_semaphore: Mutex::new(None),
        cancel,
        done,
        app,
    });
    {
        let mut active = ACTIVE.lock().map_err(|_| lock_error())?;
        if active.as_ref().is_some_and(|job| !*job.done.borrow()) {
            return Err("pelican.error.alreadyRunning".into());
        }
        *active = Some(state.clone());
    }
    tauri::async_runtime::spawn(async move {
        run_retry(state, index).await;
    });
    Ok(retry_batch)
}

fn valid_account_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() < 256
        && !id.contains("..")
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn prepare_item_retry(batch: &mut Batch, item_id: &str) -> Result<usize, String> {
    let Some(index) = batch.items.iter().position(|item| item.id == item_id) else {
        return Err("pelican.error.artifactMissing".into());
    };
    if !matches!(
        batch.items[index].status.as_str(),
        "failed" | "cancelled" | "interrupted"
    ) {
        return Err("pelican.error.invalidRequest".into());
    }
    batch.revision += 1;
    batch.status = "running".into();
    batch.finished_at = None;
    batch.error = None;
    let item = &mut batch.items[index];
    item.status = "queued".into();
    item.started_at = None;
    item.finished_at = None;
    item.reply_preview = None;
    item.has_html = false;
    item.error = None;
    item.usage = None;
    item.response_id = None;
    item.response_model = None;
    item.quota = None;
    Ok(index)
}

pub fn active() -> Result<Option<Batch>, String> {
    ACTIVE
        .lock()
        .map_err(|_| lock_error())?
        .as_ref()
        .map(|job| job.snapshot())
        .transpose()
}

fn find_active(id: &str) -> Result<Option<Arc<ActiveBatch>>, String> {
    let active = ACTIVE.lock().map_err(|_| lock_error())?;
    match active.as_ref() {
        Some(job) if job.snapshot()?.id == id => Ok(Some(job.clone())),
        _ => Ok(None),
    }
}

pub async fn get(batch_id: String) -> Result<Batch, String> {
    if let Some(job) = find_active(&batch_id)? {
        return job.snapshot();
    }
    store::read(batch_id).await
}

pub async fn history(offset: usize, limit: usize) -> Result<History, String> {
    store::history(offset, limit.clamp(1, 50)).await
}

pub async fn retention_settings() -> Result<RetentionSettings, String> {
    Ok(RetentionSettings {
        days: store::retention_days().await?,
    })
}

pub async fn update_retention_days(days: u32) -> Result<RetentionSettings, String> {
    if !(1..=3650).contains(&days) {
        return Err("pelican.error.invalidRetention".into());
    }
    store::set_retention_days(days).await?;
    Ok(RetentionSettings { days })
}

pub async fn cleanup_expired() -> Result<CleanupSummary, String> {
    let Ok(_maintenance) = MAINTENANCE.try_lock() else {
        return Ok(CleanupSummary { deleted_count: 0 });
    };
    let days = store::retention_days().await?;
    let active_id = ACTIVE
        .lock()
        .map_err(|_| lock_error())?
        .as_ref()
        .and_then(|job| job.snapshot().ok().map(|batch| batch.id));
    let deleted_ids = store::cleanup_expired(days, now(), active_id).await?;
    Ok(CleanupSummary {
        deleted_count: deleted_ids.len(),
    })
}

pub async fn clear_all() -> Result<CleanupSummary, String> {
    let _maintenance = MAINTENANCE.lock().await;
    {
        let active = ACTIVE.lock().map_err(|_| lock_error())?;
        if active.as_ref().is_some_and(|job| !*job.done.borrow()) {
            return Err("pelican.error.stillRunning".into());
        }
    }
    let deleted_count = store::clear_all().await?;
    *ACTIVE.lock().map_err(|_| lock_error())? = None;
    Ok(CleanupSummary { deleted_count })
}

pub async fn artifact(batch_id: String, item_id: String) -> Result<Artifact, String> {
    let batch = get(batch_id.clone()).await?;
    if !batch.items.iter().any(|item| item.id == item_id) {
        return Err("pelican.error.artifactMissing".into());
    }
    store::artifact(batch_id, item_id).await
}

pub async fn cancel(batch_id: String) -> Result<Batch, String> {
    let Some(job) = find_active(&batch_id)? else {
        return get(batch_id).await;
    };
    let mut done = job.done.subscribe();
    if !*done.borrow() {
        job.update(|batch| {
            if matches!(batch.status.as_str(), "running" | "cancelling") {
                batch.status = "cancelling".into();
                job.cancel.send_replace(true);
            }
        })?;
        // Waiting is bounded and does not own the batch/global lock.
        tokio::time::timeout(Duration::from_secs(40), async {
            while !*done.borrow() {
                done.changed().await.map_err(|_| lock_error())?;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "pelican.error.cancelTimeout".to_string())??;
    }
    job.snapshot()
}

pub async fn dismiss(batch_id: String) -> Result<(), String> {
    cancel(batch_id.clone()).await?;
    let mut active = ACTIVE.lock().map_err(|_| lock_error())?;
    if active
        .as_ref()
        .is_some_and(|job| job.snapshot().is_ok_and(|batch| batch.id == batch_id))
    {
        *active = None;
    }
    Ok(())
}

pub async fn delete(batch_id: String) -> Result<(), String> {
    if let Some(job) = find_active(&batch_id)? {
        if !*job.done.borrow() {
            return Err("pelican.error.stillRunning".into());
        }
    }
    store::delete(batch_id.clone()).await?;
    let mut active = ACTIVE.lock().map_err(|_| lock_error())?;
    if active
        .as_ref()
        .is_some_and(|job| job.snapshot().is_ok_and(|batch| batch.id == batch_id))
    {
        *active = None;
    }
    Ok(())
}

async fn persist(job: &ActiveBatch) -> Result<(), String> {
    store::save(job.snapshot()?).await
}

async fn run_batch(job: Arc<ActiveBatch>) {
    let worker = job.clone();
    let result = tokio::spawn(async move { run_batch_inner(worker).await })
        .await
        .map_err(|error| format!("pelican.error.workerFailed: {error}"))
        .and_then(|result| result);
    if let Err(error) = result {
        job.storage_error(error);
        let _ = job.update(|batch| {
            batch.status = "interrupted".into();
            batch.finished_at = Some(now());
            for item in &mut batch.items {
                if matches!(item.status.as_str(), "queued" | "running") {
                    item.status = "interrupted".into();
                    item.finished_at = Some(now());
                }
            }
        });
        if let Err(error) = persist(&job).await {
            job.storage_error(error);
        }
    }
    job.done.send_replace(true);
}

async fn run_batch_inner(job: Arc<ActiveBatch>) -> Result<(), String> {
    // No network request before the initial durable record exists.
    persist(&job).await?;
    let batch = job.snapshot()?;
    let semaphore = Arc::new(Semaphore::new(batch.concurrency));
    *job.retry_semaphore.lock().map_err(|_| lock_error())? = Some(semaphore.clone());
    let mut tasks = tokio::task::JoinSet::new();
    for (index, item) in batch.items.into_iter().enumerate() {
        let job = job.clone();
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            run_item(job, semaphore, index, item).await;
        });
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            job.storage_error(format!("pelican.error.workerFailed: {error}"));
        }
    }
    loop {
        let retry_done = job.retry_done.notified();
        let finalized = {
            let _retry_gate = job.retry_gate.lock().map_err(|_| lock_error())?;
            if job.pending_retries.load(Ordering::Acquire) != 0 {
                false
            } else {
                job.accepting_retries.store(false, Ordering::Release);
                job.update(|batch| {
                    batch.status = if *job.cancel.borrow() {
                        "cancelled"
                    } else {
                        "completed"
                    }
                    .into();
                    batch.finished_at = Some(now());
                    for item in &mut batch.items {
                        if matches!(item.status.as_str(), "queued" | "running") {
                            item.status = "interrupted".into();
                            item.finished_at = Some(now());
                            item.error = Some("pelican.error.workerFailed".into());
                        }
                    }
                })?;
                true
            }
        };
        if finalized {
            break;
        }
        retry_done.await;
    }
    persist(&job).await
}

async fn run_retry(job: Arc<ActiveBatch>, index: usize) {
    let worker = job.clone();
    let result = tokio::spawn(async move {
        persist(&worker).await?;
        let semaphore = Arc::new(Semaphore::new(
            worker.snapshot()?.concurrency.clamp(1, MAX_CONCURRENCY),
        ));
        *worker.retry_semaphore.lock().map_err(|_| lock_error())? = Some(semaphore.clone());
        let item = worker.snapshot()?.items[index].clone();
        store::save_artifact(
            worker.snapshot()?.id,
            item.id.clone(),
            Artifact {
                raw_reply: String::new(),
                html: None,
            },
        )
        .await?;
        run_item(worker.clone(), semaphore, index, item).await;
        loop {
            let retry_done = worker.retry_done.notified();
            let finalized = {
                let _retry_gate = worker.retry_gate.lock().map_err(|_| lock_error())?;
                if worker.pending_retries.load(Ordering::Acquire) != 0 {
                    false
                } else {
                    worker.accepting_retries.store(false, Ordering::Release);
                    worker.update(|batch| {
                        batch.status = if *worker.cancel.borrow() {
                            "cancelled"
                        } else {
                            "completed"
                        }
                        .into();
                        batch.finished_at = Some(now());
                    })?;
                    true
                }
            };
            if finalized {
                break;
            }
            retry_done.await;
        }
        persist(&worker).await
    })
    .await
    .map_err(|error| format!("pelican.error.workerFailed: {error}"))
    .and_then(|result| result);
    if let Err(error) = result {
        job.storage_error(error);
        let _ = job.update(|batch| {
            batch.status = "interrupted".into();
            batch.finished_at = Some(now());
            if let Some(item) = batch.items.get_mut(index) {
                if matches!(item.status.as_str(), "queued" | "running") {
                    item.status = "interrupted".into();
                    item.finished_at = Some(now());
                }
            }
        });
        let _ = persist(&job).await;
    }
    job.done.send_replace(true);
}

async fn acquire_or_cancel(
    semaphore: Arc<Semaphore>,
    mut cancel: watch::Receiver<bool>,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    if *cancel.borrow() {
        return None;
    }
    tokio::select! {
        biased;
        _ = cancel.changed() => None,
        permit = semaphore.acquire_owned() => permit.ok(),
    }
}

async fn run_item(job: Arc<ActiveBatch>, semaphore: Arc<Semaphore>, index: usize, item: Item) {
    let mut cancel = job.cancel.subscribe();
    let permit = acquire_or_cancel(semaphore, cancel.clone()).await;
    if permit.is_none() || *cancel.borrow() {
        finish_cancelled(&job, index).await;
        return;
    }
    let account_id = item.account_id.clone();
    let item_id = item.id.clone();
    let account_id_for_load = account_id.clone();
    let identity = tokio::select! {
        biased;
        _ = cancel.changed() => { finish_cancelled(&job, index).await; return; },
        result = tokio::time::timeout(IO_TIMEOUT, tokio::task::spawn_blocking(move || {
            let account = crate::modules::codex_account::load_account(&account_id_for_load)
                .ok_or_else(|| "pelican.error.accountUnavailable".to_string())?;
            if account.is_api_key_auth() || account.is_web_session_auth() {
                return Err("PELICAN_UNSUPPORTED_ACCOUNT".into());
            }
            Ok((account.email, account.quota, account.usage_updated_at))
        })) => result.map_err(|_| "pelican.error.storageTimeout".to_string())
            .and_then(|result| result.map_err(|error| error.to_string())).and_then(|result| result),
    };
    let (email, cached_quota, quota_updated_at) = match identity {
        Ok(identity) => identity,
        Err(error) => {
            finish_failed(&job, index, error).await;
            return;
        }
    };
    let batch = match job.update(|batch| {
        batch.items[index].account_email = email;
        batch.items[index].status = "running".into();
        batch.items[index].started_at = Some(now());
    }) {
        Ok(batch) => batch,
        Err(_) => return,
    };
    if let Err(error) = persist(&job).await {
        job.storage_error(error.clone());
        finish_failed(&job, index, error).await;
        return;
    }
    let partial = Arc::new(Mutex::new((String::new(), Instant::now())));
    let partial_delta = partial.clone();
    let delta_job = job.clone();
    let result = crate::modules::codex_local_access::run_pelican_chat(
        &item.account_id,
        &batch.model,
        &batch.effort,
        &batch.prompt,
        cancel,
        move |delta| {
            let preview = if let Ok(mut state) = partial_delta.lock() {
                state.0.push_str(&delta);
                if state.1.elapsed() < STREAM_EMIT_INTERVAL {
                    None
                } else {
                    state.1 = Instant::now();
                    Some(preview_text(&state.0))
                }
            } else {
                None
            };
            if let Some(preview) = preview {
                let _ = delta_job.update_stream_preview(index, preview);
            }
        },
    )
    .await;
    let result_ok = result.is_ok();
    let header_quota = result
        .as_ref()
        .ok()
        .and_then(|output| output.quota.as_ref().cloned());
    let raw_reply = match &result {
        Ok(output) => output.reply.clone(),
        Err(_) => partial
            .lock()
            .map(|state| state.0.clone())
            .unwrap_or_default(),
    };
    let html = extract_html(&raw_reply);
    let has_html = html.is_some();
    let preview = preview_text(&raw_reply);
    // A stale cached quota would turn other sessions' usage into this run's delta.
    let current_timestamp = now() / 1000;
    let before_quota = quota_updated_at
        .filter(|updated_at| (current_timestamp - updated_at).abs() <= 900)
        .and_then(|_| cached_quota.as_ref())
        .and_then(crate::modules::codex_local_access::pelican_quota_snapshot_from_codex_quota)
        .filter(|snapshot| {
            snapshot
                .reset_at
                .map(|reset_at| reset_at > current_timestamp)
                .unwrap_or(true)
        });
    let artifact_result =
        store::save_artifact(batch.id, item_id.clone(), Artifact { raw_reply, html }).await;
    if let Err(error) = &artifact_result {
        job.storage_error(error.clone());
    }
    let cancelled = *job.cancel.borrow();
    let updated = job.update(|batch| {
        let item = &mut batch.items[index];
        item.finished_at = Some(now());
        item.reply_preview = Some(preview);
        item.has_html = has_html && artifact_result.is_ok();
        match result {
            Ok(output) => {
                item.status = if artifact_result.is_ok() {
                    "completed"
                } else {
                    "failed"
                }
                .into();
                item.error = artifact_result.err();
                item.quota = pelican_quota_usage(before_quota.as_ref(), output.quota.as_ref());
                item.usage = output.usage;
                item.response_id = output.response_id;
                item.response_model = output.response_model;
            }
            Err(error) => {
                item.status = if cancelled { "cancelled" } else { "failed" }.into();
                item.error = Some(error);
            }
        }
    });
    if let Err(error) = persist(&job).await {
        job.storage_error(error);
    }
    if result_ok && header_quota.is_none() {
        if let Ok(updated) = updated {
            if let Some(finished_at) = updated
                .items
                .get(index)
                .filter(|item| item.id == item_id)
                .and_then(|item| item.finished_at)
            {
                spawn_pelican_quota_refresh(
                    job.clone(),
                    index,
                    item_id,
                    finished_at,
                    account_id,
                    before_quota,
                );
            }
        }
    }
}

fn spawn_pelican_quota_refresh(
    job: Arc<ActiveBatch>,
    index: usize,
    item_id: String,
    finished_at: i64,
    account_id: String,
    before_quota: Option<crate::modules::codex_local_access::PelicanQuotaSnapshot>,
) {
    tokio::spawn(async move {
        let refreshed = tokio::time::timeout(
            Duration::from_secs(15),
            crate::modules::codex_quota::refresh_account_quota(&account_id),
        )
        .await;
        let Ok(Ok(quota)) = refreshed else {
            return;
        };
        let Some(after) =
            crate::modules::codex_local_access::pelican_quota_snapshot_from_codex_quota(&quota)
        else {
            return;
        };
        let usage = pelican_quota_usage(before_quota.as_ref(), Some(&after));
        let mut matched = false;
        let updated = job.update(|batch| {
            let Some(item) = batch.items.get_mut(index) else {
                return;
            };
            if item.id == item_id && item.finished_at == Some(finished_at) {
                item.quota = usage;
                matched = true;
            }
        });
        if updated.is_ok() && matched {
            if let Err(error) = persist(&job).await {
                job.storage_error(error);
            }
        }
    });
}

async fn finish_cancelled(job: &ActiveBatch, index: usize) {
    let _ = job.update(|batch| {
        batch.items[index].status = "cancelled".into();
        batch.items[index].finished_at = Some(now());
    });
    // Queue cancellation has no artifact to commit. The batch's final snapshot persists these
    // states once, rather than scheduling up to 200 redundant writes during cancellation.
}

async fn finish_failed(job: &ActiveBatch, index: usize, error: String) {
    let _ = job.update(|batch| {
        batch.items[index].status = "failed".into();
        batch.items[index].finished_at = Some(now());
        batch.items[index].error = Some(error);
    });
    if let Err(error) = persist(job).await {
        job.storage_error(error);
    }
}

fn preview_text(reply: &str) -> String {
    reply.chars().take(PREVIEW_CHARS).collect()
}

/// Extract only complete, authored documents. Never synthesize or repair model output.
fn extract_html(reply: &str) -> Option<String> {
    let mut fence = false;
    let mut block = String::new();
    let mut eligible = false;
    for line in reply.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if fence {
                if eligible {
                    if let Some(document) = extract_document(block.trim()) {
                        return Some(document);
                    }
                }
                fence = false;
                block.clear();
            } else {
                let language = trimmed.trim_start_matches('`').trim().to_ascii_lowercase();
                eligible = matches!(language.as_str(), "html" | "svg" | "xml" | "");
                fence = true;
            }
        } else if fence {
            block.push_str(line);
            block.push('\n');
        }
    }
    // Do not accept a truncated fenced response as if the model finished its document.
    if fence {
        return None;
    }
    extract_document(reply.trim())
}

fn extract_document(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    if let Some(start) = lower
        .find("<html")
        .filter(|index| tag_boundary(&lower[*index + 5..]))
    {
        if let Some(end) = lower.rfind("</html>").filter(|end| *end > start) {
            let start = lower[..start].rfind("<!doctype html").unwrap_or(start);
            return Some(text[start..end + 7].to_string());
        }
        return None;
    }
    if let Some(start) = lower
        .find("<svg")
        .filter(|index| tag_boundary(&lower[*index + 4..]))
    {
        if let Some(end) = lower.rfind("</svg>").filter(|end| *end > start) {
            // A standalone SVG is directly renderable in srcdoc without altering it.
            return Some(text[start..end + 6].to_string());
        }
    }
    None
}

fn tag_boundary(suffix: &str) -> bool {
    suffix
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_whitespace() || ch == '>')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quota_snapshot(
        remaining_percent: i32,
        reset_at: i64,
    ) -> crate::modules::codex_local_access::PelicanQuotaSnapshot {
        crate::modules::codex_local_access::PelicanQuotaSnapshot {
            used_percent: (100 - remaining_percent) as f64,
            remaining_percent,
            window_minutes: Some(300),
            reset_at: Some(reset_at),
        }
    }

    #[test]
    fn quota_usage_reports_remaining_and_consumed_percent() {
        let before = quota_snapshot(80, 1_000);
        let after = quota_snapshot(73, 1_300);
        let usage = pelican_quota_usage(Some(&before), Some(&after)).expect("quota usage");
        assert_eq!(usage.remaining_percent, Some(73));
        assert_eq!(usage.consumed_percent, Some(7.0));
    }

    #[test]
    fn quota_usage_does_not_invent_consumption_after_reset() {
        let before = quota_snapshot(12, 1_000);
        let after = quota_snapshot(95, 1_300);
        let usage = pelican_quota_usage(Some(&before), Some(&after)).expect("quota usage");
        assert_eq!(usage.remaining_percent, Some(95));
        assert_eq!(usage.consumed_percent, None);
    }

    #[test]
    fn retry_resets_failed_item_without_creating_a_new_batch() {
        let item_id = uuid::Uuid::new_v4().to_string();
        let mut batch = Batch {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 8,
            created_at: 1,
            finished_at: None,
            status: "running".into(),
            prompt: "prompt".into(),
            model: "gpt-6-astra".into(),
            effort: "medium".into(),
            concurrency: 3,
            transport: "direct-chat".into(),
            delivery_instructions: "standalone HTML".into(),
            error: Some("old batch error".into()),
            items: vec![Item {
                id: item_id.clone(),
                account_id: "account".into(),
                account_email: "account@example.com".into(),
                status: "failed".into(),
                started_at: Some(2),
                finished_at: Some(3),
                reply_preview: Some("old reply".into()),
                has_html: true,
                error: Some("old item error".into()),
                usage: Some(serde_json::json!({"tokens": 10})),
                response_id: Some("response".into()),
                response_model: Some("model".into()),
                quota: None,
            }],
        };
        let batch_id = batch.id.clone();
        assert_eq!(prepare_item_retry(&mut batch, &item_id).unwrap(), 0);
        assert_eq!(batch.id, batch_id);
        assert_eq!(batch.revision, 9);
        assert_eq!(batch.status, "running");
        assert!(batch.error.is_none());
        let item = &batch.items[0];
        assert_eq!(item.status, "queued");
        assert!(item.started_at.is_none());
        assert!(item.finished_at.is_none());
        assert!(item.reply_preview.is_none());
        assert!(!item.has_html);
        assert!(item.error.is_none());
        assert!(item.usage.is_none());
        assert!(item.response_id.is_none());
        assert!(item.response_model.is_none());
    }

    #[test]
    fn stream_emit_budget_is_shared_across_all_accounts() {
        let started = Instant::now();
        let mut last_emit = started;
        assert!(!take_stream_emit_slot(
            &mut last_emit,
            started + Duration::from_millis(149)
        ));
        let first_tick = started + STREAM_EMIT_INTERVAL;
        let emitted = (0..MAX_BATCH_ACCOUNTS)
            .filter(|_| take_stream_emit_slot(&mut last_emit, first_tick))
            .count();
        assert_eq!(emitted, 1);
        assert!(!take_stream_emit_slot(
            &mut last_emit,
            first_tick + Duration::from_millis(149)
        ));
        assert!(take_stream_emit_slot(
            &mut last_emit,
            first_tick + STREAM_EMIT_INTERVAL
        ));
    }

    #[test]
    fn validates_custom_concurrency_within_batch_limit() {
        assert_eq!(default_concurrency(), 3);
        for concurrency in [1, 6, 10] {
            assert_eq!(validate_concurrency(concurrency), Ok(()));
        }
        for concurrency in [0, 11, 200] {
            assert_eq!(
                validate_concurrency(concurrency),
                Err("pelican.error.invalidConcurrency".into())
            );
        }
    }

    #[test]
    fn extracts_complete_documents_without_repairing() {
        let html = "<!DOCTYPE html><html><body>鹈鹕</body></html>";
        assert_eq!(
            extract_html(&format!("Done:\n```html\n{html}\n```\nNote")),
            Some(html.into())
        );
        assert_eq!(
            extract_html("<svg viewBox='0 0 1 1'></svg>"),
            Some("<svg viewBox='0 0 1 1'></svg>".into())
        );
        assert_eq!(
            extract_html("<HTML><BODY>x</BODY></HTML>"),
            Some("<HTML><BODY>x</BODY></HTML>".into())
        );
        assert_eq!(extract_html("Created test.html"), None);
        assert_eq!(extract_html("<html><svg></svg>"), None);
        assert_eq!(extract_html("```html\n<html></html>"), None);
        assert_eq!(extract_html("<htmlish></html>"), None);
    }

    #[test]
    fn bounds_previews_without_splitting_unicode() {
        assert_eq!(
            preview_text(&"鹈".repeat(2000)).chars().count(),
            PREVIEW_CHARS
        );
        assert!(!valid_account_id("../account"));
        assert!(!valid_account_id("C:\\account"));
        assert!(valid_account_id("codex_123-abc"));
    }

    #[tokio::test]
    async fn queued_work_does_not_start_after_cancellation() {
        let semaphore = Arc::new(Semaphore::new(0));
        let (cancel, receiver) = watch::channel(false);
        let work = tokio::spawn(acquire_or_cancel(semaphore, receiver));
        cancel.send_replace(true);
        assert!(tokio::time::timeout(Duration::from_millis(200), work)
            .await
            .unwrap()
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn fake_executor_respects_parallel_limit_and_cancels_queue() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let semaphore = Arc::new(Semaphore::new(3));
        let running = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let (cancel, _) = watch::channel(false);
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..20 {
            let semaphore = semaphore.clone();
            let running = running.clone();
            let maximum = maximum.clone();
            let started = started.clone();
            let mut receiver = cancel.subscribe();
            tasks.spawn(async move {
                let Some(_permit) = acquire_or_cancel(semaphore, receiver.clone()).await else {
                    return;
                };
                started.fetch_add(1, Ordering::SeqCst);
                let count = running.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(count, Ordering::SeqCst);
                if !*receiver.borrow() {
                    let _ = receiver.changed().await;
                }
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while started.load(Ordering::SeqCst) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cancel.send_replace(true);
        while let Some(result) = tasks.join_next().await {
            result.unwrap();
        }
        assert_eq!(maximum.load(Ordering::SeqCst), 3);
        assert_eq!(started.load(Ordering::SeqCst), 3);
        assert_eq!(running.load(Ordering::SeqCst), 0);
    }
}
