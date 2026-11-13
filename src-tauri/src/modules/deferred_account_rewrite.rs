//! Defer secure-account rewrite / token re-encryption off the list/load hot path.
//!
//! Jobs are bounded and coalesced per account. Safe callers also bind a job to
//! the source file hash so a delayed migration can never overwrite a newer
//! account update or recreate an account that was deleted in the meantime.

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use super::logger;

const REWRITE_QUEUE_CAPACITY: usize = 256;

type RewriteAction = Box<dyn FnOnce() + Send + 'static>;

struct RewriteJob {
    key: String,
    action: RewriteAction,
}

static SENDER: OnceLock<Option<SyncSender<RewriteJob>>> = OnceLock::new();
static PENDING_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn pending_keys() -> &'static Mutex<HashSet<String>> {
    PENDING_KEYS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn remove_pending_key(key: &str) {
    if let Ok(mut pending) = pending_keys().lock() {
        pending.remove(key);
    }
}

fn worker_sender() -> Option<&'static SyncSender<RewriteJob>> {
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel::<RewriteJob>(REWRITE_QUEUE_CAPACITY);
            match thread::Builder::new()
                .name("deferred-account-rewrite".into())
                .spawn(move || {
                    logger::log_info("[DeferredAccountRewrite] 后台账号详情迁移写回 worker 已启动");
                    while let Ok(job) = rx.recv() {
                        let key = job.key;
                        let result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.action));
                        remove_pending_key(&key);
                        if let Err(panic) = result {
                            logger::log_warn(&format!(
                                "[DeferredAccountRewrite] 写回任务 panic: key={}, panic={:?}",
                                key, panic
                            ));
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                }) {
                Ok(_) => Some(tx),
                Err(error) => {
                    logger::log_error(&format!(
                        "[DeferredAccountRewrite] 启动 worker 失败: {}",
                        error
                    ));
                    None
                }
            }
        })
        .as_ref()
}

fn schedule_keyed(key: String, action: RewriteAction) {
    {
        let Ok(mut pending) = pending_keys().lock() else {
            logger::log_warn("[DeferredAccountRewrite] 获取任务去重锁失败，跳过写回");
            return;
        };
        if !pending.insert(key.clone()) {
            return;
        }
    }

    let Some(sender) = worker_sender() else {
        remove_pending_key(&key);
        return;
    };
    let job = RewriteJob {
        key: key.clone(),
        action,
    };
    match sender.try_send(job) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            remove_pending_key(&key);
            logger::log_warn(&format!(
                "[DeferredAccountRewrite] 写回队列已满，稍后读取时重试: key={}",
                key
            ));
        }
        Err(TrySendError::Disconnected(_)) => {
            remove_pending_key(&key);
            logger::log_error(&format!(
                "[DeferredAccountRewrite] 写回 worker 已退出: key={}",
                key
            ));
        }
    }
}

fn content_hash(content: &[u8]) -> [u8; 32] {
    Sha256::digest(content).into()
}

/// Queue a rewrite only if the source file is byte-for-byte unchanged when the
/// worker gets to it. A changed or missing file means a newer write/delete won.
pub fn schedule_account_rewrite_if_unchanged<F>(
    platform: &'static str,
    account_id: String,
    source_path: PathBuf,
    source_content: &[u8],
    action: F,
) where
    F: FnOnce() -> Result<String, String> + Send + 'static,
{
    let key = format!("{}:{}:{}", platform, account_id, source_path.display());
    let expected_hash = content_hash(source_content);
    schedule_keyed(
        key,
        Box::new(move || {
            logger::log_info(&format!(
                "[DeferredAccountRewrite] 开始写回: platform={}, account_id={}",
                platform, account_id
            ));
            let rewritten = action().and_then(|content| {
                let _lock = crate::modules::account_file_crypto::lock_account_file(&source_path)?;
                crate::modules::atomic_write::write_string_atomic_if_hash_matches(
                    &source_path,
                    expected_hash,
                    || Ok(content),
                )
            });
            match rewritten {
                Ok(true) => {}
                Ok(false) => logger::log_info(&format!(
                    "[DeferredAccountRewrite] 源文件已变化或删除，跳过旧快照写回: platform={}, account_id={}, path={}",
                    platform,
                    account_id,
                    source_path.display()
                )),
                Err(error) => logger::log_warn(&format!(
                    "[DeferredAccountRewrite] 写回失败: platform={}, account_id={}, path={}, error={}",
                    platform,
                    account_id,
                    source_path.display(),
                    error
                )),
            }
        }),
    );
}

/// Compatibility path for callers that cannot yet provide the source file.
/// Jobs are still bounded and coalesced, but callers should prefer the CAS API.
pub fn schedule_account_rewrite<F>(platform: &'static str, account_id: String, action: F)
where
    F: FnOnce() + Send + 'static,
{
    let key = format!("{}:{}", platform, account_id);
    schedule_keyed(
        key,
        Box::new(move || {
            logger::log_info(&format!(
                "[DeferredAccountRewrite] 开始写回: platform={}, account_id={}",
                platform, account_id
            ));
            action();
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::{content_hash, schedule_account_rewrite};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn content_hash_changes_with_source_bytes() {
        assert_ne!(content_hash(b"old"), content_hash(b"new"));
        assert_eq!(content_hash(b"same"), content_hash(b"same"));
    }

    #[test]
    fn schedule_coalesces_same_platform_account_key() {
        let counter = Arc::new(Mutex::new(0u32));
        let counter_a = Arc::clone(&counter);
        let counter_b = Arc::clone(&counter);
        let unique_id = format!(
            "acc-coalesce-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );

        schedule_account_rewrite("test", unique_id.clone(), move || {
            if let Ok(mut value) = counter_a.lock() {
                *value += 1;
            }
            // Keep the first job pending long enough for the duplicate schedule call.
            thread::sleep(Duration::from_millis(40));
        });
        // Same key while first job is still pending/queued should be dropped.
        schedule_account_rewrite("test", unique_id, move || {
            if let Ok(mut value) = counter_b.lock() {
                *value += 10;
            }
        });

        thread::sleep(Duration::from_millis(120));
        let value = *counter.lock().expect("counter lock");
        assert_eq!(value, 1, "duplicate key must not run a second job");
    }
}
