//! Short-lived sharing for status probes only. Stop/start verification bypasses it.
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Entries = Vec<(u32, Option<String>)>;
const STATUS_PROBE_TTL: Duration = Duration::from_millis(500);

struct CachedProbe {
    expected: String,
    completed_at: Instant,
    entries: Entries,
}

#[derive(Default)]
pub(super) struct StatusProbeCache(Mutex<Option<CachedProbe>>);

impl StatusProbeCache {
    pub(super) fn get_or_probe(
        &self,
        expected: &str,
        probe: impl FnOnce() -> Option<Entries>,
    ) -> Entries {
        // This dedicated lock coalesces only the bounded PowerShell fallback;
        // no account, profile or runtime-state lock is acquired here.
        let mut cached = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(value) = cached.as_ref() {
            if value.expected == expected && value.completed_at.elapsed() < STATUS_PROBE_TTL {
                return value.entries.clone();
            }
        }
        // A failed command is not evidence that all clients exited. Retain the
        // last readable status for this path and briefly back off before retry.
        let entries = probe().unwrap_or_else(|| {
            cached
                .as_ref()
                .filter(|value| value.expected == expected)
                .map(|value| value.entries.clone())
                .unwrap_or_default()
        });
        *cached = Some(CachedProbe {
            expected: expected.to_owned(),
            completed_at: Instant::now(),
            entries: entries.clone(),
        });
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Barrier,
    };

    #[test]
    fn concurrent_status_requests_share_one_probe_and_keep_instance_directories() {
        let cache = Arc::new(StatusProbeCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(5));
        let expected = vec![(10, None), (20, Some("managed-profile".into()))];
        let workers = (0..5)
            .map(|_| {
                let (cache, calls, barrier, expected) = (
                    cache.clone(),
                    calls.clone(),
                    barrier.clone(),
                    expected.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    cache.get_or_probe("codex.exe", || {
                        calls.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(30));
                        Some(expected)
                    })
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_probe_is_shared_but_expiry_and_path_change_reprobe() {
        let cache = StatusProbeCache::default();
        assert!(cache
            .get_or_probe("old.exe", || Some(Vec::new()))
            .is_empty());
        assert!(cache
            .get_or_probe("old.exe", || panic!("duplicate probe"))
            .is_empty());
        assert_eq!(
            cache.get_or_probe("new.exe", || Some(vec![(20, None)])),
            vec![(20, None)]
        );
        cache.0.lock().unwrap().as_mut().unwrap().completed_at =
            Instant::now() - Duration::from_secs(1);
        assert_eq!(
            cache.get_or_probe("new.exe", || Some(vec![(30, None)])),
            vec![(30, None)]
        );
    }

    #[test]
    fn timeout_keeps_previous_status_then_allows_retry() {
        let cache = StatusProbeCache::default();
        let expected = vec![(20, Some("managed".into()))];
        assert_eq!(
            cache.get_or_probe("codex.exe", || Some(expected.clone())),
            expected
        );
        cache.0.lock().unwrap().as_mut().unwrap().completed_at =
            Instant::now() - Duration::from_secs(1);
        assert_eq!(cache.get_or_probe("codex.exe", || None), expected);
        assert_eq!(
            cache.get_or_probe("codex.exe", || panic!("retry burst")),
            expected
        );
        cache.0.lock().unwrap().as_mut().unwrap().completed_at =
            Instant::now() - Duration::from_secs(1);
        assert!(cache
            .get_or_probe("codex.exe", || Some(Vec::new()))
            .is_empty());
    }
}
