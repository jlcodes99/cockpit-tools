// One bounded queue and one thread. No per-request tasks wait on the SQLite
// mutex. Persistent storage failures retain the current batch, stop admission,
// and back off visibly; producers have a bounded async wait and explicit error.
const USAGE_WRITER_CAPACITY: usize = 256;
const USAGE_WRITER_ROW_BYTES: usize = 128 * 1024;
const USAGE_WRITER_BATCH_ROWS: usize = 8;
const USAGE_WRITER_BATCH_BYTES: usize = 512 * 1024;
const USAGE_WRITER_BATCH_TIME: Duration = Duration::from_millis(25);
static USAGE_WRITER_FAILED: AtomicBool = AtomicBool::new(false);
static USAGE_IN_FLIGHT: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(USAGE_WRITER_CAPACITY)));
struct UsageCallerPermit {
    _storage: OwnedSemaphorePermit,
    _key: Option<Arc<dyn Send + Sync>>,
}
fn usage_caller_permit(
    key: Option<Arc<dyn Send + Sync>>,
) -> Result<Arc<dyn Send + Sync>, native_contract::ServiceError> {
    let storage = USAGE_IN_FLIGHT.clone().try_acquire_owned().map_err(|_| {
        native_error(
            503,
            "usage_storage_busy",
            "Too many in-flight requests; retry later",
        )
    })?;
    Ok(Arc::new(UsageCallerPermit {
        _storage: storage,
        _key: key,
    }))
}

#[derive(Debug)]
struct UsageLogWrite {
    event: CodexLocalAccessUsageEvent,
    epoch: u64,
    bytes: usize,
    official: Option<String>,
}

// Count encoded bytes without allocating another copy of the event. Stop at
// 128 KiB even for one pathological string; never parse request payload JSON.
fn usage_log_encoded_bytes(event: &CodexLocalAccessUsageEvent) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > USAGE_WRITER_ROW_BYTES.saturating_sub(self.0) {
                return Err(std::io::Error::other(
                    "usage event exceeds 128 KiB writer limit",
                ));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, event).map_err(|e| e.to_string())?;
    Ok(counter.0)
}

fn usage_log_sender() -> Option<&'static tokio::sync::mpsc::Sender<UsageLogWrite>> {
    static SENDER: OnceLock<Option<tokio::sync::mpsc::Sender<UsageLogWrite>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (sender, receiver) = tokio::sync::mpsc::channel(USAGE_WRITER_CAPACITY);
            match std::thread::Builder::new()
                .name("codex-usage-writer".into())
                .spawn(move || run_usage_log_writer(receiver))
            {
                Ok(_) => Some(sender),
                Err(error) => {
                    USAGE_WRITER_FAILED.store(true, Ordering::Release);
                    logger::log_codex_api_warn(&format!("Usage writer unavailable: {error}"));
                    None
                }
            }
        })
        .as_ref()
}

fn usage_log_admission() -> Result<(), native_contract::ServiceError> {
    if USAGE_WRITER_FAILED.load(Ordering::Acquire)
        || usage_log_sender().is_none_or(|sender| sender.capacity() == 0)
    {
        return Err(native_error(
            503,
            "usage_storage_busy",
            "Usage storage is busy; retry later",
        ));
    }
    Ok(())
}

async fn queue_local_access_usage_event(event: CodexLocalAccessUsageEvent) -> Result<(), String> {
    let result = async {
        let bytes = usage_log_encoded_bytes(&event)?;
        let sender = usage_log_sender().ok_or("usage writer unavailable")?;
        let write = UsageLogWrite {
            event,
            bytes,
            epoch: REQUEST_LOG_WRITE_EPOCH.load(Ordering::Acquire),
            official: None,
        };
        tokio::time::timeout(Duration::from_secs(2), sender.send(write))
            .await
            .map_err(|_| "usage writer queue timed out")?
            .map_err(|_| "usage writer closed")?;
        Ok(())
    }
    .await;
    if let Err(error) = &result {
        logger::log_codex_api_warn(&format!("请求日志未入队，内存用量已保留: {error}"));
    }
    result
}

fn write_usage_log_batch(
    conn: &Connection,
    batch: &[UsageLogWrite],
    epoch: u64,
) -> Result<usize, String> {
    write_usage_log_batch_limited(conn, batch, epoch, USAGE_WRITER_BATCH_ROWS)
}

fn write_usage_log_batch_limited(
    conn: &Connection,
    batch: &[UsageLogWrite],
    epoch: u64,
    limit: usize,
) -> Result<usize, String> {
    ensure_remote_analytics_schema(conn).map_err(|e| e.to_string())?;
    with_request_log_sql_budget(conn, USAGE_WRITER_BATCH_TIME, || {
        let start = Instant::now();
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let mut count = 0;
        for write in batch.iter().take(limit) {
            if count > 0 && start.elapsed() >= USAGE_WRITER_BATCH_TIME {
                break;
            }
            if write.epoch == epoch {
                insert_live_usage_event_in_transaction(
                    &tx,
                    &write.event,
                    write.official.as_deref(),
                )?;
                // SQLite can roll back the transaction on interruption even
                // when an optional summary refresh reports only a warning.
                // Stop before another row could be written in autocommit mode.
                if conn.is_autocommit() {
                    return Err("Usage transaction interrupted; retry batch".into());
                }
            }
            count += 1;
        }
        tx.commit().map_err(|e| e.to_string())?;
        REMOTE_ANALYTICS_REVISION.fetch_add(1, Ordering::AcqRel);
        Ok(count)
    })
}

fn run_usage_log_writer(mut receiver: tokio::sync::mpsc::Receiver<UsageLogWrite>) {
    let mut batch = Vec::with_capacity(USAGE_WRITER_BATCH_ROWS);
    let mut pending = None;
    let mut conn = None;
    let mut failures = 0_u32;
    let mut row_limit = USAGE_WRITER_BATCH_ROWS;
    loop {
        if batch.is_empty() {
            let first = pending.take().or_else(|| receiver.blocking_recv());
            let Some(first) = first else {
                return;
            };
            let mut bytes = first.bytes;
            batch.push(first);
            while batch.len() < USAGE_WRITER_BATCH_ROWS {
                let Ok(next) = receiver.try_recv() else {
                    break;
                };
                if bytes + next.bytes > USAGE_WRITER_BATCH_BYTES {
                    pending = Some(next);
                    break;
                }
                bytes += next.bytes;
                batch.push(next);
            }
            // Optional account snapshots are resolved once, outside every DB
            // transaction and lock. Retries reuse the captured value.
            for write in &mut batch {
                write.official = official_stats_account_id(&write.event.account_id);
            }
        }
        let result = (|| -> Result<Option<usize>, String> {
            let Ok(_guard) = local_access_logs_db_write_lock().try_lock() else {
                return Ok(None);
            };
            if conn.is_none() {
                let opened = open_local_access_logs_db_with_schema_unlocked(true)?;
                opened
                    .busy_timeout(Duration::from_millis(10))
                    .map_err(|e| e.to_string())?;
                conn = Some(opened);
            }
            write_usage_log_batch_limited(
                conn.as_ref().unwrap(),
                &batch,
                REQUEST_LOG_WRITE_EPOCH.load(Ordering::Acquire),
                row_limit,
            )
            .map(Some)
        })();
        match result {
            Ok(Some(count)) => {
                if USAGE_WRITER_FAILED.swap(false, Ordering::AcqRel) {
                    logger::log_codex_api_info("请求日志写入已恢复");
                }
                failures = 0;
                row_limit = USAGE_WRITER_BATCH_ROWS;
                batch.drain(..count);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                USAGE_WRITER_FAILED.store(true, Ordering::Release);
                failures = failures.saturating_add(1);
                row_limit = (row_limit / 2).max(1);
                logger::log_codex_api_warn(&format!(
                    "请求日志批次保留待重试，暂停新请求准入: attempt={failures}, error={error}"
                ));
                // Three short attempts, then a visible degraded state and low
                // frequency recovery. Keep accepted rows bounded in this batch.
                std::thread::sleep(if failures >= 3 {
                    Duration::from_secs(30)
                } else {
                    Duration::from_millis(100 * u64::from(failures))
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_local_access_usage_writer_tests.rs"]
mod usage_writer_tests;
