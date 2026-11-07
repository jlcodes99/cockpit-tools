// A cooperative deadline belongs to the synchronous launch worker, never to a
// detached task. The caller awaits repair and rollback before it can launch Codex.
const LAUNCH_VISIBILITY_REPAIR_BUDGET: Duration = Duration::from_secs(5);
const MAX_LAUNCH_ROLLOUT_FIRST_LINE_BYTES: u64 = 1024 * 1024;
const LAUNCH_VISIBILITY_TIMEOUT: &str = "CODEX_SESSION_VISIBILITY_TIMEOUT";
const LAUNCH_VISIBILITY_LINE_TOO_LARGE: &str = "CODEX_SESSION_VISIBILITY_ROLLOUT_LINE_TOO_LARGE";
const LAUNCH_VISIBILITY_ROLLBACK_FAILED: &str = "CODEX_SESSION_VISIBILITY_ROLLBACK_FAILED";

/// Only the read before any mutation can be deferred. Never suppress a timeout
/// after writes, a rollback failure, or an actual history/configuration error.
pub fn can_defer_launch_snapshot_error(error: &str) -> bool {
    !error.contains(LAUNCH_VISIBILITY_ROLLBACK_FAILED)
        && (error.contains("CODEX_SESSION_VISIBILITY_PROCESS_UNAVAILABLE")
            || (error.contains(LAUNCH_VISIBILITY_TIMEOUT)
                && error.ends_with("stage=process_snapshot")))
}

struct LaunchVisibilityBudget {
    deadline: std::time::Instant,
    interrupts: Mutex<Vec<rusqlite::InterruptHandle>>,
    suspended: std::sync::atomic::AtomicBool,
}

thread_local! {
    static LAUNCH_VISIBILITY_BUDGET: std::cell::RefCell<Option<std::sync::Arc<LaunchVisibilityBudget>>> =
        const { std::cell::RefCell::new(None) };
}

struct LaunchVisibilityBudgetGuard {
    previous: Option<std::sync::Arc<LaunchVisibilityBudget>>,
    stop: std::sync::mpsc::Sender<()>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Drop for LaunchVisibilityBudgetGuard {
    fn drop(&mut self) {
        LAUNCH_VISIBILITY_BUDGET.with(|slot| slot.replace(self.previous.take()));
        let _ = self.stop.send(());
        if let Some(watcher) = self.watcher.take() {
            // The watcher only waits on this channel and calls SQLite interrupt;
            // joining cannot leave an old deadline attached to a later launch.
            let _ = watcher.join();
        }
    }
}

fn launch_visibility_budget() -> Option<std::sync::Arc<LaunchVisibilityBudget>> {
    LAUNCH_VISIBILITY_BUDGET.with(|slot| slot.borrow().clone())
}

fn with_launch_visibility_budget<T>(
    duration: Duration,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let budget = std::sync::Arc::new(LaunchVisibilityBudget {
        deadline: std::time::Instant::now() + duration,
        interrupts: Mutex::new(Vec::new()),
        suspended: std::sync::atomic::AtomicBool::new(false),
    });
    let (stop, receiver) = std::sync::mpsc::channel();
    let worker_budget = budget.clone();
    let watcher = std::thread::Builder::new()
        .name("codex-launch-visibility-budget".to_string())
        .spawn(move || {
            let remaining = worker_budget
                .deadline
                .saturating_duration_since(std::time::Instant::now());
            if matches!(
                receiver.recv_timeout(remaining),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ) {
                loop {
                    let interrupts = worker_budget
                        .interrupts
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    if !worker_budget
                        .suspended
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        for interrupt in interrupts.iter() {
                            interrupt.interrupt();
                        }
                    }
                    drop(interrupts);
                    // Re-interrupt until the owning worker returns. A statement
                    // beginning just after the first interrupt must also stop.
                    if !matches!(
                        receiver.recv_timeout(Duration::from_millis(10)),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            }
        })
        .map_err(|error| format!("{LAUNCH_VISIBILITY_TIMEOUT}: deadline worker: {error}"))?;
    let previous = LAUNCH_VISIBILITY_BUDGET.with(|slot| slot.replace(Some(budget.clone())));
    let _guard = LaunchVisibilityBudgetGuard {
        previous,
        stop,
        watcher: Some(watcher),
    };
    match work() {
        Err(error) if error.contains(LAUNCH_VISIBILITY_ROLLBACK_FAILED) => Err(error),
        Err(error) if std::time::Instant::now() >= budget.deadline => {
            Err(format!("{LAUNCH_VISIBILITY_TIMEOUT}: {error}"))
        }
        result => result,
    }
}

fn check_launch_visibility_budget(stage: &str) -> Result<(), String> {
    if launch_visibility_budget().is_some_and(|budget| std::time::Instant::now() >= budget.deadline)
    {
        return Err(format!("{LAUNCH_VISIBILITY_TIMEOUT}: stage={stage}"));
    }
    Ok(())
}

fn configure_launch_visibility_connection(connection: &Connection) -> Result<(), String> {
    let Some(budget) = launch_visibility_budget() else {
        return Ok(());
    };
    check_launch_visibility_budget("open_sqlite")?;
    let mut interrupts = budget
        .interrupts
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Serialize registration with the watcher, so a connection opened at the
    // deadline cannot miss interruption and begin a long statement afterwards.
    check_launch_visibility_budget("register_sqlite")?;
    interrupts.push(connection.get_interrupt_handle());
    drop(interrupts);
    connection
        .busy_timeout(
            budget
                .deadline
                .saturating_duration_since(std::time::Instant::now())
                .min(Duration::from_millis(250)),
        )
        .map_err(|error| format!("SQLite launch repair busy timeout: {error}"))?;
    Ok(())
}

fn without_launch_visibility_budget<T>(work: impl FnOnce() -> T) -> T {
    struct Restore {
        previous: Option<std::sync::Arc<LaunchVisibilityBudget>>,
        was_suspended: bool,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if let Some(budget) = &self.previous {
                budget
                    .suspended
                    .store(self.was_suspended, std::sync::atomic::Ordering::Release);
            }
            LAUNCH_VISIBILITY_BUDGET.with(|slot| slot.replace(self.previous.take()));
        }
    }
    let previous = LAUNCH_VISIBILITY_BUDGET.with(|slot| slot.take());
    let was_suspended = previous.as_ref().is_some_and(|budget| {
        // Serialize with an in-progress interrupt, then discard old connection
        // handles. Neither existing nor fresh rollback connections can inherit
        // interruption while restoring a backup after a partial multi-DB write.
        let mut interrupts = budget
            .interrupts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let was_suspended = budget
            .suspended
            .swap(true, std::sync::atomic::Ordering::AcqRel);
        interrupts.clear();
        was_suspended
    });
    let _restore = Restore {
        previous,
        was_suspended,
    };
    // Recovery must finish even after the scan/write deadline. It uses fresh
    // file handles and must not inherit a timeout from the failed operation.
    work()
}
