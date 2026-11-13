use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

#[derive(Default)]
struct Queue {
    pending: BTreeSet<String>,
    running: bool,
}

impl Queue {
    fn enqueue(&mut self, ids: impl IntoIterator<Item = String>) -> bool {
        self.pending.extend(ids);
        if self.running || self.pending.is_empty() {
            return false;
        }
        self.running = true;
        true
    }

    fn next(&mut self) -> Option<String> {
        let id = self.pending.pop_first();
        if id.is_none() {
            self.running = false;
        }
        id
    }
}

static QUEUE: std::sync::LazyLock<Mutex<Queue>> =
    std::sync::LazyLock::new(|| Mutex::new(Queue::default()));

pub(super) fn schedule(app: &AppHandle, ids: impl IntoIterator<Item = String>) {
    let start = super::lock_or_recover(&QUEUE, "wakeup quota queue").enqueue(ids);
    if !start {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let id = super::lock_or_recover(&QUEUE, "wakeup quota queue").next();
            let Some(id) = id else { break };
            let result = tokio::time::timeout(
                Duration::from_secs(90),
                crate::modules::account::refresh_quota_after_wakeup(id.clone()),
            )
            .await
            .map_err(|_| "刷新配额超时".to_string())
            .and_then(|result| result);
            if let Err(error) = result {
                crate::modules::logger::log_warn(&format!(
                    "[WakeupTasks] 唤醒后刷新配额失败: account_id={id}, error={error}"
                ));
            }
            tokio::task::yield_now().await;
        }
        let _ = app.emit("accounts:changed", serde_json::json!({
            "platformId": "antigravity",
            "reason": "wakeup-quota-refresh"
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakeup_quota_refresh_coalesces_pending_accounts_and_keeps_new_wakeups() {
        let mut queue = Queue::default();
        assert!(!queue.enqueue([]));
        assert!(queue.enqueue(["a", "b", "b"].map(str::to_owned)));
        assert_eq!(queue.next().as_deref(), Some("a"));
        // A second wakeup may finish after a's quota request was sent.
        assert!(!queue.enqueue(["a", "b"].map(str::to_owned)));
        assert_eq!(queue.next().as_deref(), Some("a"));
        assert_eq!(queue.next().as_deref(), Some("b"));
        assert_eq!(queue.next(), None);
        assert!(queue.enqueue(["c".into()]));
        assert_eq!(queue.next().as_deref(), Some("c"));
    }
}
