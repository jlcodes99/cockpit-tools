// Only account IDs are queued. Each attempt reads current credentials from disk, including
// source deletion, so a retry can never restore a captured, obsolete access token.
const GROK_AUTH_SYNC_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const GROK_AUTH_SYNC_RETRY_MIN: Duration = Duration::from_secs(1);
const GROK_AUTH_SYNC_RETRY_MAX: Duration = Duration::from_secs(30);

#[derive(Default)]
struct GrokAuthSyncQueue {
    // Presence means one worker owns the account; true means another update needs a pass.
    pending: Mutex<HashMap<String, bool>>,
}

impl GrokAuthSyncQueue {
    fn request(&self, account_id: &str) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pending.insert(account_id.to_string(), true).is_none()
    }

    fn begin_attempt(&self, account_id: &str) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(dirty) = pending.get_mut(account_id) {
            *dirty = false;
        }
    }

    fn finish_attempt(&self, account_id: &str, succeeded: bool) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if succeeded && pending.get(account_id) == Some(&false) {
            pending.remove(account_id);
            true
        } else {
            // Keep ownership on errors and when an update arrived during the attempt.
            false
        }
    }

    fn remove(&self, account_id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account_id);
    }
}

fn grok_auth_sync_queue() -> &'static GrokAuthSyncQueue {
    static QUEUE: OnceLock<GrokAuthSyncQueue> = OnceLock::new();
    QUEUE.get_or_init(GrokAuthSyncQueue::default)
}

/// Single-flight per source account, with capped exponential backoff until synchronization
/// succeeds or the host shuts down. Caller/UI paths never wait for background retries.
pub fn sync_grok_upstream_auth_files_in_background(grok_account_id: String) {
    let account_id = grok_account_id.trim().to_string();
    if account_id.is_empty() || crate::modules::app_lifecycle::is_shutdown_started() {
        return;
    }
    let queue = grok_auth_sync_queue();
    if !queue.request(&account_id) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        run_grok_auth_sync_worker(
            queue,
            &account_id,
            GROK_AUTH_SYNC_LOCK_TIMEOUT,
            GROK_AUTH_SYNC_RETRY_MIN,
        )
        .await;
    });
}

async fn run_grok_auth_sync_worker(
    queue: &GrokAuthSyncQueue,
    account_id: &str,
    lock_timeout: Duration,
    retry_min: Duration,
) {
    let mut retry_delay = retry_min;
    loop {
        if crate::modules::app_lifecycle::is_shutdown_started() {
            queue.remove(account_id);
            return;
        }
        queue.begin_attempt(account_id);
        let result = sync_grok_upstream_auth_files_once(account_id.to_string(), lock_timeout).await;
        if queue.finish_attempt(account_id, result.is_ok()) {
            return;
        }
        if let Err(error) = result {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][provider-gateway] Grok 账号凭据同步失败，将自动重试: account_id={}, retry_ms={}, error={}",
                account_id, retry_delay.as_millis(), error
            ));
            // No lifecycle/state lock is held during backoff. Do not cancel an in-flight
            // blocking file write with an outer timeout and create overlapping writers.
            tokio::time::sleep(retry_delay).await;
            retry_delay = retry_delay.saturating_mul(2).min(GROK_AUTH_SYNC_RETRY_MAX);
        } else {
            retry_delay = retry_min;
            tokio::task::yield_now().await;
        }
    }
}
