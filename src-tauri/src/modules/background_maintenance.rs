//! One low-priority lane for incremental data maintenance. Jobs retain only
//! paths/cursors; each invocation must perform a bounded batch and drop locks.
use std::collections::{HashSet, VecDeque};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::Duration;

pub(crate) const BATCH_ROWS: usize = 32;
pub(crate) const BATCH_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const BATCH_TIME: Duration = Duration::from_millis(25);
const PAUSE: Duration = Duration::from_millis(150);
const CAPACITY: usize = 16;

pub(crate) fn busy_message() -> String {
    super::i18n::translate(
        &super::config::get_user_config().language,
        "common.dataQueryBusy",
        &[],
    )
}

type Batch = Box<dyn FnMut() -> Result<bool, String> + Send>;
struct Job {
    key: String,
    batch: Batch,
    failures: u32,
}
static KEYS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
static SENDER: OnceLock<Option<mpsc::SyncSender<Job>>> = OnceLock::new();
// Background repricing shares this lane with scheduled account/log migrations.
// A busy lane is a reason to yield, never to wait while holding a business lock.
static RESOURCE: Mutex<()> = Mutex::new(());
pub(crate) fn try_resource() -> Option<std::sync::MutexGuard<'static, ()>> {
    RESOURCE.try_lock().ok()
}

fn remove(key: &str) {
    if let Ok(mut keys) = KEYS.lock() {
        if let Some(keys) = keys.as_mut() {
            keys.remove(key);
        }
    }
}

/// `true` completes a job; `false` yields until a later round. Failures back off
/// and release the key after five retries, allowing the next read/start to retry.
pub(crate) fn schedule(key: String, batch: impl FnMut() -> Result<bool, String> + Send + 'static) {
    {
        let Ok(mut keys) = KEYS.lock() else { return };
        let keys = keys.get_or_insert_with(HashSet::new);
        if keys.len() >= CAPACITY || !keys.insert(key.clone()) {
            return;
        }
    }
    let sender = SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Job>(CAPACITY);
        std::thread::Builder::new()
            .name("data-maintenance".into())
            .spawn(move || {
                let mut queue = VecDeque::new();
                loop {
                    if queue.is_empty() {
                        let Ok(job) = rx.recv() else { break };
                        queue.push_back(job);
                    }
                    while let Ok(job) = rx.try_recv() {
                        queue.push_back(job);
                    }
                    std::thread::sleep(PAUSE);
                    let Some(mut job) = queue.pop_front() else {
                        continue;
                    };
                    if super::app_lifecycle::is_shutdown_started() {
                        remove(&job.key);
                        continue;
                    }
                    let Some(resource) = try_resource() else {
                        queue.push_back(job);
                        continue;
                    };
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut job.batch));
                    drop(resource);
                    match result {
                        Ok(Ok(true)) => remove(&job.key),
                        Ok(Ok(false)) => {
                            job.failures = 0;
                            queue.push_back(job);
                        }
                        _ => {
                            job.failures += 1;
                            super::logger::log_warn(&format!(
                                "[Maintenance] {} batch failed ({}/5): {:?}",
                                job.key, job.failures, result
                            ));
                            if job.failures >= 5 {
                                remove(&job.key);
                            } else {
                                std::thread::sleep(Duration::from_millis(
                                    200 * u64::from(job.failures),
                                ));
                                queue.push_back(job);
                            }
                        }
                    }
                }
            })
            .ok()
            .map(|_| tx)
    });
    let job = Job {
        key: key.clone(),
        batch: Box::new(batch),
        failures: 0,
    };
    if !sender
        .as_ref()
        .is_some_and(|sender| sender.try_send(job).is_ok())
    {
        remove(&key);
    }
}
