//! Resolve service identity and selected model off the injection/render loop.
use super::*;

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const RETAIN_VERIFIED_FOR: Duration = Duration::from_secs(15);
type BindingResult = Result<(Option<String>, output_speed::OutputSpeedSelection), String>;
type BindingReader = std::sync::Arc<dyn Fn(&Path) -> BindingResult + Send + Sync>;

pub(super) struct BindingPoller {
    profile: PathBuf,
    task: JoinSet<(Instant, BindingResult)>,
    task_started: Option<Instant>,
    task_expired: bool,
    reader: BindingReader,
    binding: Option<String>,
    selection: output_speed::OutputSpeedSelection,
    initialized: bool,
    verified: bool,
    verified_at: Instant,
    warned_at: Instant,
}

impl BindingPoller {
    pub(super) fn new(profile: &Path) -> Self {
        Self::with_reader(profile, std::sync::Arc::new(read_binding))
    }

    fn with_reader(profile: &Path, reader: BindingReader) -> Self {
        Self {
            profile: profile.into(),
            task: JoinSet::new(),
            task_started: None,
            task_expired: false,
            reader,
            binding: None,
            selection: output_speed::OutputSpeedSelection::default(),
            initialized: false,
            verified: false,
            verified_at: Instant::now() - Duration::from_secs(30),
            warned_at: Instant::now() - Duration::from_secs(30),
        }
    }

    pub(super) fn collect(&mut self) -> Value {
        if let Some(result) = self.task.try_join_next() {
            self.initialized = true;
            if let Ok((finished, _)) = &result {
                self.task_expired |= self.task_started.is_some_and(|started| {
                    finished.saturating_duration_since(started) >= READ_TIMEOUT
                });
            }
            self.task_started = None;
            match result {
                // A late successful read must not overwrite the already-settled
                // error. Its physical slot is released only on real completion.
                _ if self.task_expired => self.read_failed("timeout"),
                Ok((_, Ok((binding, selection)))) => {
                    self.binding = binding;
                    self.selection = selection;
                    self.verified = self.binding.is_some();
                    self.verified_at = Instant::now();
                }
                Ok((_, Err(_))) => self.read_failed("config_read"),
                Err(_) => self.read_failed("worker_join"),
            }
        }
        if !self.task_expired
            && self
                .task_started
                .is_some_and(|started| started.elapsed() >= READ_TIMEOUT)
        {
            self.task_expired = true;
            self.initialized = true;
            self.read_failed("timeout");
        }
        // Expire even while a physical read remains blocked; no further error
        // completion is needed to settle loading or invalidate old identity.
        if !self.verified && self.verified_at.elapsed() >= RETAIN_VERIFIED_FOR {
            self.binding = None;
            self.selection = output_speed::OutputSpeedSelection::default();
        }
        if self.task.is_empty() {
            let profile = self.profile.clone();
            let reader = self.reader.clone();
            self.task_started = Some(Instant::now());
            self.task_expired = false;
            self.task.spawn_blocking(move || {
                let result = reader(&profile);
                (Instant::now(), result)
            });
        }
        json!({"binding":self.binding,"model":self.selection.model,
            "serviceTier":self.selection.service_tier,"available":self.verified,
            "error":self.initialized && !self.verified,
            "pending":!self.initialized})
    }

    pub(super) fn selection(&self) -> output_speed::OutputSpeedSelection {
        self.selection.clone()
    }

    fn read_failed(&mut self, error: &str) {
        self.verified = false;
        if self.warned_at.elapsed() >= Duration::from_secs(30) {
            logger::log_warn(&format!("[Codex Usage] 服务绑定读取失败: {error}"));
            self.warned_at = Instant::now();
        }
    }
}

fn read_binding(profile: &Path) -> BindingResult {
    let before = output_speed::binding_snapshot(profile)?;
    let selection = output_speed::read_profile_selection(profile)?;
    let valid = match recent_account::open_gateway_logs(profile)? {
        Some((conn, scope)) => dialog_data::matches_local_service(profile, &conn, &scope)?,
        None => false,
    };
    let binding = (valid && output_speed::binding_snapshot(profile)? == before)
        .then_some(before)
        .flatten();
    Ok((binding, selection))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Condvar,
    };

    fn selection() -> output_speed::OutputSpeedSelection {
        output_speed::OutputSpeedSelection {
            model: Some("gpt-model".into()),
            service_tier: Some("priority".into()),
        }
    }

    async fn wait_state(poll: &mut BindingPoller, predicate: impl Fn(&Value) -> bool) -> Value {
        timeout(Duration::from_secs(2), async {
            loop {
                let state = poll.collect();
                if predicate(&state) {
                    return state;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("binding worker must settle promptly")
    }

    #[tokio::test]
    async fn binding_transient_failure_retains_verified_identity_until_expiry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let reader_calls = calls.clone();
        let mut poll = BindingPoller::with_reader(
            Path::new("unused-fixture"),
            Arc::new(move |_| {
                if reader_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok((Some("binding-a".into()), selection()))
                } else {
                    Err("fixture read temporarily unavailable".into())
                }
            }),
        );
        let state = wait_state(&mut poll, |state| state["available"] == true).await;
        assert_eq!(state["binding"], "binding-a");
        let failed = wait_state(&mut poll, |state| state["error"] == true).await;
        assert_eq!(failed["binding"], "binding-a");
        assert_eq!(failed["available"], false);
        assert_eq!(failed["model"], "gpt-model");
        poll.verified_at = Instant::now() - RETAIN_VERIFIED_FOR - Duration::from_millis(1);
        let expired = poll.collect();
        assert!(expired["binding"].is_null());
        assert!(expired["model"].is_null());
    }

    #[tokio::test]
    async fn binding_explicit_unbind_immediately_clears_identity() {
        let calls = Arc::new(AtomicUsize::new(0));
        let reader_calls = calls.clone();
        let mut poll = BindingPoller::with_reader(
            Path::new("unused-fixture"),
            Arc::new(move |_| {
                let binding =
                    (reader_calls.fetch_add(1, Ordering::SeqCst) == 0).then(|| "binding-a".into());
                Ok((binding, selection()))
            }),
        );
        wait_state(&mut poll, |state| state["available"] == true).await;
        let state = wait_state(&mut poll, |state| state["error"] == true).await;
        assert!(state["binding"].is_null());
        assert!(poll.verified_at.elapsed() < RETAIN_VERIFIED_FOR);
    }

    #[tokio::test]
    async fn binding_worker_deadline_settles_ui_without_releasing_physical_slot() {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let reader_gate = gate.clone();
        let reader_calls = calls.clone();
        let mut poll = BindingPoller::with_reader(
            Path::new("unused-fixture"),
            Arc::new(move |_| {
                reader_calls.fetch_add(1, Ordering::SeqCst);
                let (lock, condition) = &*reader_gate;
                let released = lock.lock().unwrap();
                let (released, _) = condition
                    .wait_timeout_while(released, Duration::from_secs(2), |released| !*released)
                    .unwrap();
                if !*released {
                    return Err("fixture gate timed out".into());
                }
                Ok((Some("late-binding".into()), selection()))
            }),
        );
        assert_eq!(poll.collect()["pending"], true);
        timeout(Duration::from_secs(1), async {
            while calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        poll.binding = Some("verified-binding".into());
        poll.verified = true;
        poll.verified_at = Instant::now();
        poll.task_started = Some(Instant::now() - READ_TIMEOUT - Duration::from_millis(1));
        let timed_out = poll.collect();
        assert_eq!(timed_out["pending"], false);
        assert_eq!(timed_out["error"], true);
        assert_eq!(timed_out["binding"], "verified-binding");
        for _ in 0..20 {
            poll.collect();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(poll.task.len(), 1);
        poll.verified_at = Instant::now() - RETAIN_VERIFIED_FOR - Duration::from_millis(1);
        assert!(poll.collect()["binding"].is_null());
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        timeout(Duration::from_secs(1), async {
            loop {
                let state = poll.collect();
                if !poll.task_expired {
                    assert!(state["binding"].is_null());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
}
