use std::collections::{HashSet, VecDeque};
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

const CAPACITY: usize = 1024;
#[derive(Default)]
struct Queue {
    ids: VecDeque<String>,
    pending: HashSet<String>,
    running: bool,
}
impl Queue {
    fn push(&mut self, id: String) -> bool {
        if self.pending.contains(&id) {
            return true;
        }
        if self.pending.len() >= CAPACITY {
            return false;
        }
        self.pending.insert(id.clone());
        self.ids.push_back(id);
        true
    }
    fn next(&mut self) -> Option<String> {
        let next = self.ids.pop_front();
        if next.is_none() {
            self.running = false;
        }
        next
    }
}
static QUEUE: LazyLock<Mutex<Queue>> = LazyLock::new(|| Mutex::new(Queue::default()));

pub(super) fn schedule(app: tauri::AppHandle, ids: Vec<String>) {
    let start = {
        let Ok(mut queue) = QUEUE.lock() else {
            return;
        };
        for id in ids {
            if !queue.push(id) {
                // Import is already durable; overflow leaves it available for
                // the existing manual/periodic refresh instead of dropping data.
                crate::modules::logger::log_warn("[Kiro Import] refresh queue full; imported account retained for manual refresh");
            }
        }
        if queue.running {
            false
        } else {
            queue.running = true;
            true
        }
    };
    if !start {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let id = QUEUE.lock().ok().and_then(|mut queue| queue.next());
            let Some(id) = id else {
                break;
            };
            if let Err(error) = crate::modules::kiro_account::refresh_account_token(&id).await {
                crate::modules::logger::log_warn(&format!(
                    "[Kiro Import] background refresh failed: account_id={id}, error={error}"
                ));
            }
            if let Ok(mut queue) = QUEUE.lock() {
                queue.pending.remove(&id);
            }
            let _ = app.emit(
                "accounts:changed",
                serde_json::json!({"platformId":"kiro", "accountId":id, "reason":"import-refresh"}),
            );
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_deduplicates_pending_and_active_accounts_and_bounds_work() {
        let mut queue = Queue::default();
        assert!(queue.push("one".into()));
        assert_eq!(queue.next().as_deref(), Some("one"));
        assert!(queue.push("one".into()));
        assert!(queue.ids.is_empty());
        for n in 1..CAPACITY {
            assert!(queue.push(n.to_string()));
        }
        assert!(!queue.push("overflow".into()));
        queue.pending.remove("one");
        assert!(queue.push("retry".into()));
        assert_eq!(queue.pending.len(), CAPACITY);
    }
}
