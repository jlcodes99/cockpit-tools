//! Preserve per-host results and stop queued writes when a switch is superseded.
use std::future::Future;

pub(super) async fn run<T, R, F: Future<Output = R>>(
    targets: Vec<T>,
    is_current: impl Fn() -> bool,
    is_selected: impl Fn(&T) -> bool,
    sync: impl Fn(T) -> F,
    mut publish: impl FnMut(R),
) {
    for target in targets {
        if !is_current() {
            break;
        }
        if !is_selected(&target) {
            continue;
        }
        let result = sync(target).await;
        if !is_current() {
            break;
        }
        publish(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn a_failed_host_does_not_prevent_other_selected_hosts_from_syncing() {
        let mut results = Vec::new();
        run(
            vec!["offline", "online"],
            || true,
            |_| true,
            |host| async move { (host, host != "offline") },
            |result| results.push(result),
        )
        .await;
        assert_eq!(results, [("offline", false), ("online", true)]);
    }

    #[tokio::test]
    async fn superseded_account_stops_the_queue_and_drops_old_results() {
        let current = AtomicBool::new(true);
        let mut results = Vec::new();
        let calls = std::sync::Mutex::new(Vec::new());
        run(
            vec!["first", "must-not-write"],
            || current.load(Ordering::SeqCst),
            |_| true,
            |host| {
                calls.lock().unwrap().push(host);
                current.store(false, Ordering::SeqCst);
                async move { host }
            },
            |result| results.push(result),
        )
        .await;
        assert_eq!(*calls.lock().unwrap(), ["first"]);
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn deselected_hosts_are_skipped_while_other_targets_continue() {
        let disabled = AtomicBool::new(false);
        let mut results = Vec::new();
        run(
            vec!["first", "deselected", "last"],
            || true,
            |host| *host != "deselected" || !disabled.load(Ordering::SeqCst),
            |host| {
                disabled.store(true, Ordering::SeqCst);
                async move { host }
            },
            |result| results.push(result),
        )
        .await;
        assert_eq!(results, ["first", "last"]);
    }
}
