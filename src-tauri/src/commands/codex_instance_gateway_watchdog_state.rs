// Pure scheduling state: no filesystem, runtime, process, or account dependencies.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Lease {
    pub key: String,
    pub revision: u64,
    pub failures: u8,
}

#[derive(Default)]
struct Profile {
    signature: Option<String>,
    revision: u64,
    flight: Option<u64>,
    failures: u8,
    due: u64,
}

#[derive(Default)]
pub(super) struct WatchState {
    profiles: BTreeMap<String, Profile>,
    running: bool,
}

impl WatchState {
    pub fn update(&mut self, mut enabled: BTreeMap<String, String>) -> bool {
        let mut changed = false;
        for (key, profile) in &mut self.profiles {
            let signature = enabled.remove(key);
            if profile.signature != signature {
                profile.signature = signature;
                profile.revision += 1;
                profile.failures = 0;
                profile.due = 0;
                changed = true;
            }
        }
        for (key, signature) in enabled {
            self.profiles.insert(
                key,
                Profile {
                    signature: Some(signature),
                    revision: 1,
                    ..Default::default()
                },
            );
            changed = true;
        }
        changed
    }

    pub fn enabled(&self) -> bool {
        self.profiles.values().any(|p| p.signature.is_some())
    }

    pub fn arm(&mut self) -> bool {
        if self.running || !self.enabled() {
            return false;
        }
        self.running = true;
        true
    }

    pub fn retire_if_idle(&mut self, shutdown: bool) -> bool {
        if shutdown || !self.enabled() {
            self.running = false;
            true
        } else {
            false
        }
    }

    pub fn due(&mut self, now: u64) -> Vec<Lease> {
        let available = 4usize.saturating_sub(
            self.profiles
                .values()
                .filter(|p| p.flight.is_some())
                .count(),
        );
        self.profiles
            .iter_mut()
            .filter_map(|(key, p)| {
                if p.signature.is_none() || p.flight.is_some() || p.due > now {
                    return None;
                }
                p.flight = Some(p.revision);
                Some(Lease {
                    key: key.clone(),
                    revision: p.revision,
                    failures: p.failures,
                })
            })
            .take(available)
            .collect()
    }

    pub fn current(&self, lease: &Lease) -> bool {
        self.profiles
            .get(&lease.key)
            .is_some_and(|p| p.signature.is_some() && p.revision == lease.revision)
    }

    // A timed-out job retains its flight until it actually completes. Disabling
    // and re-enabling cannot create another blocking job for the same profile.
    pub fn complete(&mut self, lease: &Lease, now: u64, failed: bool) -> bool {
        let current = self.current(lease);
        let Some(p) = self.profiles.get_mut(&lease.key) else {
            return false;
        };
        if p.flight != Some(lease.revision) {
            return false;
        }
        p.flight = None;
        if current {
            p.failures = if failed {
                p.failures.saturating_add(1)
            } else {
                0
            };
            p.due = now.saturating_add(if failed {
                // After fallback only health is checked; no further recovery
                // until manual recovery succeeds or routing settings change.
                if p.failures >= 3 {
                    10
                } else {
                    10 << p.failures
                }
            } else {
                10
            });
        }
        current
    }
}

pub(super) fn process_context(is_default: bool, profile: &str) -> Option<&str> {
    if is_default {
        None
    } else {
        Some(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn configs(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn disabled_has_no_scheduler_or_work() {
        let mut state = WatchState::default();
        assert!(!state.update(configs(&[])));
        assert!(!state.arm());
        assert!(state.due(0).is_empty());
    }

    #[test]
    fn switches_deduplicate_start_and_retire_last_timer() {
        let mut s = WatchState::default();
        assert!(s.update(configs(&[("default", "a")])));
        assert!(s.arm());
        assert!(!s.arm());
        // PID and last-used refreshes have the same routing signature.
        assert!(!s.update(configs(&[("default", "a")])));
        s.update(configs(&[("default", "a"), ("other", "b")]));
        assert!(!s.arm());
        s.update(configs(&[("other", "b")]));
        assert!(!s.retire_if_idle(false));
        s.update(configs(&[]));
        assert!(s.retire_if_idle(false));
        assert!(!s.arm());
        s.update(configs(&[("other", "b")]));
        assert!(s.arm());
    }

    #[test]
    fn stale_completion_and_aba_do_not_overwrite_new_config() {
        let mut s = WatchState::default();
        s.update(configs(&[("a", "v1")]));
        let old = s.due(0).remove(0);
        s.update(configs(&[]));
        assert!(!s.current(&old));
        s.update(configs(&[("a", "v1")]));
        assert!(!s.current(&old));
        assert!(s.due(100).is_empty()); // timed-out flight is not replaced
        assert!(!s.complete(&old, 100, true));
        let new = s.due(100).remove(0);
        assert_eq!(new.failures, 0);
        assert_ne!(old.revision, new.revision);
    }

    #[test]
    fn profiles_are_isolated_and_failures_back_off() {
        let mut s = WatchState::default();
        s.update(configs(&[("a", "1"), ("b", "1")]));
        let jobs = s.due(0);
        assert!(s.complete(&jobs[0], 0, true));
        assert!(s.complete(&jobs[1], 0, false));
        assert!(s.due(9).is_empty());
        assert_eq!(s.due(10)[0].key, "b");
        let second = s.due(20).remove(0);
        assert_eq!(second.failures, 1);
        s.complete(&second, 20, true);
        assert!(s.due(59).is_empty());
        let third = s.due(60).remove(0);
        s.complete(&third, 60, true);
        let suppressed = s.due(10000).remove(0);
        assert_eq!(suppressed.failures, 3);
        s.complete(&suppressed, 10000, true);
        s.update(configs(&[("a", "2"), ("b", "1")]));
        assert!(s.current(&jobs[1]));
        assert_eq!(s.due(10000)[0].key, "a");
    }

    #[test]
    fn concurrent_notifications_have_one_scheduler_and_one_flight() {
        let s = std::sync::Arc::new(std::sync::Mutex::new(WatchState::default()));
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..16 {
                let (s, starts) = (s.clone(), starts.clone());
                scope.spawn(move || {
                    let mut s = s.lock().unwrap();
                    s.update(configs(&[("a", "1")]));
                    if s.arm() {
                        starts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                });
            }
        });
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1);
        let mut s = s.lock().unwrap();
        assert_eq!(s.due(0).len(), 1);
        assert!(s.due(1000).is_empty());
    }

    #[test]
    fn default_process_context_is_none_on_every_platform() {
        assert_eq!(process_context(true, "/default"), None);
        assert_eq!(process_context(false, "/managed"), Some("/managed"));
    }
}
