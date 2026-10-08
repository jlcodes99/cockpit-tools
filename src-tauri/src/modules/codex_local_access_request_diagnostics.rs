// Request diagnostics use an independent, bounded row and an on-demand reader.
// Neither body persistence nor SQLite lock waits run on the gateway request task.
use crate::models::codex_local_access::{
    CodexLocalAccessRequestAttempt, CodexLocalAccessRequestDetail, CodexLocalAccessRequestPayload,
};

const REQUEST_DIAGNOSTICS_QUEUE_CAPACITY: usize = 64;
const REQUEST_DIAGNOSTICS_MAX_ATTEMPTS: usize = 32;
const REQUEST_DIAGNOSTICS_MAX_PAYLOADS: usize = 8;
const REQUEST_DIAGNOSTICS_PAYLOAD_BYTES: usize = 16 * 1024;
const REQUEST_DIAGNOSTICS_REQUEST_BYTES: usize = 64 * 1024;
const REQUEST_DIAGNOSTICS_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const REQUEST_DIAGNOSTICS_MAX_ROWS: i64 = 2000;
const REQUEST_DIAGNOSTICS_TOTAL_PAYLOAD_BYTES: i64 = 32 * 1024 * 1024;
const REQUEST_DIAGNOSTICS_PRUNE_BATCH: i64 = 256;

static REQUEST_DIAGNOSTICS_DROPPED: AtomicU64 = AtomicU64::new(0);
static REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH: AtomicU64 = AtomicU64::new(0);
static REQUEST_DIAGNOSTICS_PAYLOAD_CLEARED_BEFORE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);
static REQUEST_LOG_WRITE_EPOCH: AtomicU64 = AtomicU64::new(0);
static REQUEST_LOG_RECORDS_CLEARED_BEFORE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);
static REQUEST_PAYLOAD_LOGGING_ENABLED: AtomicBool = AtomicBool::new(false);
static REQUEST_FIRST_RESPONSE_CACHE: std::sync::LazyLock<Mutex<HashMap<String, (u64, i64)>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static REQUEST_PAYLOAD_SETTING_WRITE_LOCK: std::sync::LazyLock<TokioMutex<()>> =
    std::sync::LazyLock::new(|| TokioMutex::new(()));
static REQUEST_PAYLOAD_SETTINGS_REVISION: AtomicU64 = AtomicU64::new(0);
static REQUEST_PAYLOAD_SETTINGS_SYNC_RUNNING: AtomicBool = AtomicBool::new(false);
static REQUEST_PAYLOAD_SETTINGS_ERROR: std::sync::LazyLock<Mutex<Option<(u64, String)>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPayloadLoggingStatus {
    pending: bool,
    error: Option<String>,
}

fn request_payload_logging_status_for(
    revision: u64,
    pending: bool,
    failure: Option<&(u64, String)>,
) -> RequestPayloadLoggingStatus {
    RequestPayloadLoggingStatus {
        pending,
        error: failure
            .filter(|(failed_revision, _)| *failed_revision == revision)
            .map(|(_, error)| error.clone()),
    }
}

pub fn get_local_access_request_payload_logging_status() -> RequestPayloadLoggingStatus {
    let failure = REQUEST_PAYLOAD_SETTINGS_ERROR.lock().ok();
    request_payload_logging_status_for(
        REQUEST_PAYLOAD_SETTINGS_REVISION.load(Ordering::SeqCst),
        REQUEST_PAYLOAD_SETTINGS_SYNC_RUNNING.load(Ordering::SeqCst),
        failure.as_deref().and_then(Option::as_ref),
    )
}

enum RequestDiagnosticWrite {
    Usage(Box<CodexLocalAccessUsageEvent>, u64),
    Detail(Box<CodexLocalAccessRequestDetail>, u64, u64),
}

fn request_diagnostics_writer(
) -> Option<&'static std::sync::mpsc::SyncSender<RequestDiagnosticWrite>> {
    static WRITER: OnceLock<Option<std::sync::mpsc::SyncSender<RequestDiagnosticWrite>>> =
        OnceLock::new();
    WRITER
        .get_or_init(|| {
            let (sender, receiver) =
                std::sync::mpsc::sync_channel(REQUEST_DIAGNOSTICS_QUEUE_CAPACITY);
            match std::thread::Builder::new()
                .name("codex-request-diagnostics".into())
                .spawn(move || run_request_diagnostics_writer(receiver))
            {
                Ok(_) => Some(sender),
                Err(error) => {
                    logger::log_codex_api_warn(&format!("启动 API 服务诊断写入线程失败: {error}"));
                    None
                }
            }
        })
        .as_ref()
}

fn try_queue_request_diagnostic_to(
    sender: &std::sync::mpsc::SyncSender<RequestDiagnosticWrite>,
    write: RequestDiagnosticWrite,
) -> bool {
    // Full queues lose diagnostics only: request execution and memory statistics continue.
    sender.try_send(write).is_ok()
}

fn queue_request_diagnostic(write: RequestDiagnosticWrite) {
    if !request_diagnostics_writer()
        .is_some_and(|sender| try_queue_request_diagnostic_to(sender, write))
    {
        REQUEST_DIAGNOSTICS_DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

fn queue_local_access_usage_event(event: CodexLocalAccessUsageEvent) {
    // Existing usage logs retain their persistence guarantee, independently of the
    // lossy optional body/trace queue. A full diagnostic queue must never drop usage.
    let epoch = REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || {
            let (_guard, conn) = open_local_access_logs_db_for_write()?;
            if epoch == REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst) {
                insert_local_access_usage_event(&conn, &event)?;
            }
            Ok::<_, String>(())
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                logger::log_codex_api_warn(&format!("请求日志写入失败，已保留内存统计: {error}"))
            }
            Err(error) => logger::log_codex_api_warn(&format!(
                "请求日志写入任务失败，已保留内存统计: {error}"
            )),
        }
    });
}

fn cached_request_first_response(request_id: Option<&str>) -> Option<u64> {
    let request_id = request_id.filter(|id| !id.is_empty())?;
    REQUEST_FIRST_RESPONSE_CACHE
        .try_lock()
        .ok()?
        .get(request_id)
        .map(|entry| entry.0)
}

fn queue_request_diagnostics(detail: CodexLocalAccessRequestDetail) {
    queue_request_diagnostics_at_epoch(
        detail,
        REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst),
        REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst),
    );
}

fn prepare_request_diagnostic_payloads(
    detail: &mut CodexLocalAccessRequestDetail,
    logging_enabled: bool,
    cleared_before: i64,
) {
    if !logging_enabled || detail.captured_at_ms <= cleared_before {
        detail.payloads.clear();
    }
}

fn queue_request_diagnostics_at_epoch(
    mut detail: CodexLocalAccessRequestDetail,
    payload_epoch: u64,
    log_epoch: u64,
) {
    if detail.request_id.trim().is_empty() || detail.request_id.len() > 256 {
        return;
    }
    if log_epoch != REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst)
        || detail.captured_at_ms <= REQUEST_LOG_RECORDS_CLEARED_BEFORE.load(Ordering::SeqCst)
        || now_ms().saturating_sub(detail.captured_at_ms) > REQUEST_DIAGNOSTICS_RETENTION_MS
    {
        return;
    }
    prepare_request_diagnostic_payloads(
        &mut detail,
        REQUEST_PAYLOAD_LOGGING_ENABLED.load(Ordering::SeqCst),
        REQUEST_DIAGNOSTICS_PAYLOAD_CLEARED_BEFORE.load(Ordering::SeqCst),
    );
    if let Some(first_response_ms) = detail.first_response_ms {
        if let Ok(mut cache) = REQUEST_FIRST_RESPONSE_CACHE.try_lock() {
            let now = now_ms();
            cache.retain(|_, entry| now.saturating_sub(entry.1) < 60 * 60 * 1000);
            if cache.len() >= 512 && !cache.contains_key(&detail.request_id) {
                if let Some(oldest) = cache
                    .iter()
                    .min_by_key(|entry| entry.1 .1)
                    .map(|entry| entry.0.clone())
                {
                    cache.remove(&oldest);
                }
            }
            cache.insert(detail.request_id.clone(), (first_response_ms, now));
        }
        if let Ok(mut runtime) = gateway_runtime().try_lock() {
            for event in &mut runtime.stats.events {
                if event.request_id == detail.request_id {
                    event.first_response_ms = Some(first_response_ms);
                }
            }
        }
    }
    queue_request_diagnostic(RequestDiagnosticWrite::Detail(
        Box::new(detail),
        payload_epoch,
        log_epoch,
    ));
}

fn request_logs_first_response_select(conn: &Connection) -> Result<&'static str, String> {
    request_logs_has_column(conn, "first_response_ms")
        .map(|present| {
            if present {
                "first_response_ms"
            } else {
                "NULL AS first_response_ms"
            }
        })
        .map_err(|error| format!("检查请求首响列失败: {error}"))
}

fn create_request_diagnostics_table(conn: &Connection) -> Result<(), SqliteError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS request_diagnostics (
            request_id TEXT PRIMARY KEY,
            captured_at_ms INTEGER NOT NULL,
            first_response_ms INTEGER,
            failure_phase TEXT,
            attempts_json TEXT NOT NULL DEFAULT '[]',
            payloads_json TEXT NOT NULL DEFAULT '[]',
            payload_bytes INTEGER NOT NULL DEFAULT 0,
            truncated INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_request_diagnostics_captured_at
            ON request_diagnostics(captured_at_ms);
    "#,
    )
}

fn diagnostics_table_exists(conn: &Connection) -> Result<bool, SqliteError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='request_diagnostics')", [], |row| row.get(0))
}

fn diagnostic_truncate(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn diagnostic_optional(value: Option<String>, limit: usize) -> Option<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| diagnostic_truncate(&value, limit))
}

fn diagnostic_sensitive_field(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_', ' '], "");
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "cookie"
            | "setcookie"
            | "apikey"
            | "xapikey"
            | "accesskey"
            | "accesskeyid"
            | "secretaccesskey"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "password"
            | "passwd"
            | "clientsecret"
            | "secret"
            | "privatekey"
            | "agentprivatekey"
            | "credential"
            | "credentials"
            | "assertion"
            | "clientassertion"
    )
}

fn redact_diagnostic_json(value: &mut Value, depth: usize) {
    if depth >= 48 {
        *value = Value::String("[omitted: nesting limit]".into());
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if diagnostic_sensitive_field(key) {
                    *value = Value::String("[REDACTED]".into());
                } else {
                    redact_diagnostic_json(value, depth + 1);
                }
            }
        }
        Value::Array(array) => {
            for value in array {
                redact_diagnostic_json(value, depth + 1);
            }
        }
        _ => {}
    }
}

fn normalize_request_diagnostics(
    mut detail: CodexLocalAccessRequestDetail,
) -> CodexLocalAccessRequestDetail {
    detail.request_id = detail.request_id.trim().to_string();
    detail.captured_at_ms = detail.captured_at_ms.max(0);
    detail.failure_phase = diagnostic_optional(detail.failure_phase, 80);
    detail.truncated |= detail.attempts.len() > REQUEST_DIAGNOSTICS_MAX_ATTEMPTS;
    detail.attempts.sort_by_key(|attempt| attempt.sequence);
    detail.attempts.truncate(REQUEST_DIAGNOSTICS_MAX_ATTEMPTS);
    for attempt in &mut detail.attempts {
        attempt.account_id = diagnostic_truncate(&attempt.account_id, 256);
        attempt.account_email = diagnostic_optional(attempt.account_email.take(), 256);
        attempt.model_id = diagnostic_truncate(&attempt.model_id, 256);
        attempt.transport = diagnostic_truncate(&attempt.transport, 32);
        attempt.error_category = diagnostic_optional(attempt.error_category.take(), 128);
        attempt.error_message = diagnostic_optional(attempt.error_message.take(), 1024);
        attempt.failure_phase = diagnostic_optional(attempt.failure_phase.take(), 80);
    }
    detail.truncated |= detail.payloads.len() > REQUEST_DIAGNOSTICS_MAX_PAYLOADS;
    detail.payloads.truncate(REQUEST_DIAGNOSTICS_MAX_PAYLOADS);
    let mut remaining = REQUEST_DIAGNOSTICS_REQUEST_BYTES;
    detail
        .payloads
        .retain(|payload| matches!(payload.stage.as_str(), "client" | "upstream"));
    for payload in &mut detail.payloads {
        payload.transport = diagnostic_truncate(&payload.transport, 32);
        payload.content_type = diagnostic_truncate(&payload.content_type, 128);
        payload.headers.retain(|key, _| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "content-type"
                    | "content-encoding"
                    | "accept"
                    | "user-agent"
                    | "openai-beta"
                    | "x-request-id"
            )
        });
        for value in payload.headers.values_mut() {
            *value = diagnostic_truncate(value, 256);
        }
        // The private sidecar emits already-redacted truncated snapshots. Full JSON is
        // redacted again here; unparseable, untrusted complete bodies are never persisted.
        if let Ok(mut value) = serde_json::from_str::<Value>(&payload.body) {
            redact_diagnostic_json(&mut value, 0);
            payload.body =
                serde_json::to_string(&value).unwrap_or_else(|_| "[omitted: serialization]".into());
        } else if !payload.truncated {
            payload.body = "[omitted: non-JSON request body]".into();
            payload.truncated = true;
        }
        let limit = remaining.min(REQUEST_DIAGNOSTICS_PAYLOAD_BYTES);
        if payload.body.len() > limit {
            payload.body = diagnostic_truncate(&payload.body, limit);
            payload.truncated = true;
        }
        remaining = remaining.saturating_sub(payload.body.len());
        detail.truncated |= payload.truncated;
        payload.sha256 = format!("{:x}", Sha256::digest(payload.body.as_bytes()));
    }
    detail
}

fn request_diagnostic_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodexLocalAccessRequestDetail> {
    let first_response_ms: Option<i64> = row.get("first_response_ms")?;
    let attempts: String = row.get("attempts_json")?;
    let payloads: String = row.get("payloads_json")?;
    let parse_error = |error| {
        SqliteError::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    };
    Ok(CodexLocalAccessRequestDetail {
        request_id: row.get("request_id")?,
        captured_at_ms: row.get("captured_at_ms")?,
        first_response_ms: first_response_ms.and_then(|value| u64::try_from(value).ok()),
        failure_phase: row.get("failure_phase")?,
        attempts: serde_json::from_str(&attempts).map_err(parse_error)?,
        payloads: serde_json::from_str(&payloads).map_err(parse_error)?,
        truncated: row.get::<_, i64>("truncated")? != 0,
    })
}

fn read_request_diagnostic(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<CodexLocalAccessRequestDetail>, String> {
    if !diagnostics_table_exists(conn).map_err(|error| format!("检查请求诊断失败: {error}"))?
    {
        return Ok(None);
    }
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT * FROM request_diagnostics WHERE request_id=?1",
        params![request_id],
        request_diagnostic_from_row,
    )
    .optional()
    .map_err(|error| format!("读取请求诊断失败: {error}"))
}

fn merge_request_diagnostics(
    mut previous: CodexLocalAccessRequestDetail,
    incoming: CodexLocalAccessRequestDetail,
) -> CodexLocalAccessRequestDetail {
    let newer = incoming.captured_at_ms >= previous.captured_at_ms;
    previous.first_response_ms = match (previous.first_response_ms, incoming.first_response_ms) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    };
    if newer {
        previous.failure_phase = incoming.failure_phase;
    }
    previous.captured_at_ms = previous.captured_at_ms.max(incoming.captured_at_ms);
    previous.truncated |= incoming.truncated;
    for attempt in incoming.attempts {
        match previous
            .attempts
            .iter_mut()
            .find(|old| old.sequence == attempt.sequence)
        {
            Some(old) if newer => *old = attempt,
            Some(_) => {}
            None => previous.attempts.push(attempt),
        }
    }
    for payload in incoming.payloads {
        match previous.payloads.iter_mut().find(|old| {
            old.stage == payload.stage && old.attempt_sequence == payload.attempt_sequence
        }) {
            Some(old) if newer => *old = payload,
            Some(_) => {}
            None => previous.payloads.push(payload),
        }
    }
    normalize_request_diagnostics(previous)
}

fn insert_request_diagnostic(
    conn: &Connection,
    detail: CodexLocalAccessRequestDetail,
) -> Result<(), String> {
    create_request_diagnostics_table(conn)
        .map_err(|error| format!("创建请求诊断表失败: {error}"))?;
    let detail = normalize_request_diagnostics(detail);
    let detail = match read_request_diagnostic(conn, &detail.request_id)? {
        Some(previous) => merge_request_diagnostics(previous, detail),
        None => detail,
    };
    let attempts = serde_json::to_string(&detail.attempts).map_err(|error| error.to_string())?;
    let payloads = serde_json::to_string(&detail.payloads).map_err(|error| error.to_string())?;
    let payload_bytes: usize = detail
        .payloads
        .iter()
        .map(|payload| payload.body.len())
        .sum();
    conn.execute(r#"INSERT INTO request_diagnostics
        (request_id,captured_at_ms,first_response_ms,failure_phase,attempts_json,payloads_json,payload_bytes,truncated)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(request_id) DO UPDATE SET
        captured_at_ms=excluded.captured_at_ms,first_response_ms=excluded.first_response_ms,
        failure_phase=excluded.failure_phase,attempts_json=excluded.attempts_json,
        payloads_json=excluded.payloads_json,payload_bytes=excluded.payload_bytes,truncated=excluded.truncated"#,
        params![detail.request_id,detail.captured_at_ms,detail.first_response_ms.map(|value| value.min(i64::MAX as u64) as i64),
            detail.failure_phase,attempts,payloads,payload_bytes as i64,bool_to_db_value(detail.truncated)])
        .map_err(|error| format!("写入请求诊断失败: {error}"))?;
    // This updates metadata only; usage events and aggregate statistics are never replayed.
    if request_logs_has_column(conn, "first_response_ms").unwrap_or(false) {
        conn.execute(
            "UPDATE request_logs SET first_response_ms=?1 WHERE request_id=?2",
            params![
                detail
                    .first_response_ms
                    .map(|value| value.min(i64::MAX as u64) as i64),
                detail.request_id
            ],
        )
        .map_err(|error| format!("更新请求首响失败: {error}"))?;
    }
    Ok(())
}

fn prune_request_diagnostics(conn: &Connection, now: i64) -> Result<usize, String> {
    let mut removed = conn.execute("DELETE FROM request_diagnostics WHERE request_id IN
        (SELECT request_id FROM request_diagnostics WHERE captured_at_ms<?1 ORDER BY captured_at_ms LIMIT ?2)",
        params![now.saturating_sub(REQUEST_DIAGNOSTICS_RETENTION_MS),REQUEST_DIAGNOSTICS_PRUNE_BATCH])
        .map_err(|error| format!("清理过期请求诊断失败: {error}"))?;
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM request_diagnostics", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    if rows > REQUEST_DIAGNOSTICS_MAX_ROWS {
        removed += conn.execute("DELETE FROM request_diagnostics WHERE request_id IN
            (SELECT request_id FROM request_diagnostics ORDER BY captured_at_ms,request_id LIMIT ?1)",
            params![(rows - REQUEST_DIAGNOSTICS_MAX_ROWS).min(REQUEST_DIAGNOSTICS_PRUNE_BATCH)])
            .map_err(|error| format!("清理超量请求诊断失败: {error}"))?;
    }
    // Evict old bodies first, retaining the useful account trace and first-response metric.
    let payload_bytes: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(payload_bytes),0) FROM request_diagnostics",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if payload_bytes > REQUEST_DIAGNOSTICS_TOTAL_PAYLOAD_BYTES {
        let mut statement = conn
            .prepare(
                "SELECT request_id,payload_bytes FROM request_diagnostics WHERE payload_bytes>0
            ORDER BY captured_at_ms,request_id LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let bodies = statement
            .query_map(params![REQUEST_DIAGNOSTICS_PRUNE_BATCH], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);
        let mut excess = payload_bytes - REQUEST_DIAGNOSTICS_TOTAL_PAYLOAD_BYTES;
        for (request_id, bytes) in bodies {
            if excess <= 0 {
                break;
            }
            conn.execute("UPDATE request_diagnostics SET payloads_json='[]',payload_bytes=0,truncated=1 WHERE request_id=?1", params![request_id]).map_err(|error| error.to_string())?;
            excess -= bytes;
        }
    }
    Ok(removed)
}

fn apply_request_diagnostic_write(
    conn: &Connection,
    write: RequestDiagnosticWrite,
) -> Result<(), String> {
    match write {
        RequestDiagnosticWrite::Usage(event, epoch) => {
            if epoch == REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst) {
                insert_local_access_usage_event(conn, &event)?;
            }
        }
        RequestDiagnosticWrite::Detail(mut detail, payload_epoch, log_epoch) => {
            if log_epoch != REQUEST_LOG_WRITE_EPOCH.load(Ordering::SeqCst) {
                return Ok(());
            }
            if payload_epoch != REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.load(Ordering::SeqCst) {
                detail.payloads.clear();
            }
            insert_request_diagnostic(conn, *detail)?;
        }
    }
    Ok(())
}

fn run_request_diagnostics_writer(receiver: std::sync::mpsc::Receiver<RequestDiagnosticWrite>) {
    let mut conn: Option<Connection> = None;
    loop {
        let write = match receiver.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(write) => Some(write),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let dropped = REQUEST_DIAGNOSTICS_DROPPED.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            logger::log_codex_api_warn(&format!(
                "请求诊断队列繁忙，已跳过 {dropped} 条诊断；请求及内存统计继续运行"
            ));
        }
        if conn.is_none() {
            match open_local_access_logs_db() {
                Ok(opened) => conn = Some(opened),
                Err(error) => {
                    logger::log_codex_api_warn(&format!("请求诊断数据库不可用: {error}"));
                    continue;
                }
            }
        }
        let opened = conn.as_ref().unwrap();
        let result = (|| {
            let _guard = lock_local_access_logs_db_write()?;
            if let Some(write) = write {
                apply_request_diagnostic_write(opened, write)?;
            }
            create_request_diagnostics_table(opened).map_err(|error| error.to_string())?;
            prune_request_diagnostics(opened, now_ms())?;
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            logger::log_codex_api_warn(&format!("请求诊断写入失败，已保留请求及内存统计: {error}"));
            conn = None;
        }
        // Bound each critical section to one event and one pruning batch.
        std::thread::yield_now();
    }
}

pub async fn get_local_access_request_detail(
    request_id: String,
) -> Result<Option<CodexLocalAccessRequestDetail>, String> {
    let request_id = request_id.trim().to_string();
    if request_id.is_empty() || request_id.len() > 256 {
        return Err("codex.localAccess.requestDetail.invalidRequestId".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let path = local_access_logs_db_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("打开请求诊断失败: {error}"))?;
        conn.busy_timeout(Duration::from_millis(250))
            .map_err(|error| error.to_string())?;
        let detail = read_request_diagnostic(&conn, &request_id)?;
        Ok(detail.filter(|detail| {
            now_ms().saturating_sub(detail.captured_at_ms) <= REQUEST_DIAGNOSTICS_RETENTION_MS
        }))
    })
    .await
    .map_err(|error| format!("读取请求诊断任务失败: {error}"))?
}

pub async fn get_local_access_request_payload_logging() -> Result<bool, String> {
    ensure_runtime_loaded_without_start_with_profile_restore(false).await?;
    let runtime = gateway_runtime().lock().await;
    Ok(runtime
        .collection
        .as_ref()
        .is_some_and(|collection| collection.request_payload_logging))
}

pub async fn update_local_access_request_payload_logging(
    enabled: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start_with_profile_restore(false).await?;
    let _setting_guard = REQUEST_PAYLOAD_SETTING_WRITE_LOCK.lock().await;
    let updated_at = tauri::async_runtime::spawn_blocking(move || {
        let path = local_access_file_path()?;
        persist_request_payload_logging_at_path(&path, enabled)
    })
    .await
    .map_err(|error| format!("保存请求正文设置任务失败: {error}"))??;
    {
        let mut runtime = gateway_runtime().lock().await;
        if let Some(collection) = runtime.collection.as_mut() {
            collection.request_payload_logging = enabled;
            collection.updated_at = collection.updated_at.max(updated_at);
        }
    }
    REQUEST_PAYLOAD_LOGGING_ENABLED.store(enabled, Ordering::SeqCst);
    // Disabling keeps historical snapshots. It also discards bodies already queued.
    if !enabled {
        REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.fetch_add(1, Ordering::SeqCst);
    }
    // The disk value is already committed. Runtime control retries independently,
    // so a slow/failed sidecar never strands a settings button or starts a restart.
    REQUEST_PAYLOAD_SETTINGS_REVISION.fetch_add(1, Ordering::SeqCst);
    schedule_request_payload_settings_sync();
    emit_local_access_state_updated();
    let runtime = gateway_runtime().lock().await;
    Ok(build_state_snapshot(&runtime))
}

fn persist_request_payload_logging_at_path(path: &Path, enabled: bool) -> Result<i64, String> {
    let mut updated_at = 0;
    update_string_atomic(path, |current| {
        let current = current.ok_or_else(|| "本地接入集合尚未创建".to_string())?;
        let mut collection: CodexLocalAccessCollection = serde_json::from_str(current)
            .map_err(|error| format!("解析请求正文设置失败: {error}"))?;
        if collection.request_payload_logging != enabled {
            collection.request_payload_logging = enabled;
            collection.updated_at = now_ms().max(collection.updated_at);
        }
        updated_at = collection.updated_at;
        serde_json::to_string_pretty(&collection).map_err(|error| error.to_string())
    })?;
    Ok(updated_at)
}

async fn running_request_diagnostic_ports(enabled: bool) -> Vec<u16> {
    let mut ports = {
        let runtime = gateway_runtime().lock().await;
        if runtime.running {
            runtime.actual_port.into_iter().collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };
    {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        for runtime in runtimes.values_mut() {
            if let Some(collection) = runtime.collection.as_mut() {
                collection.request_payload_logging = enabled;
            }
            if runtime.sidecar_child.is_some() {
                ports.extend(runtime.actual_port);
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

async fn send_request_payload_setting_to_sidecar(port: u16, enabled: bool) -> Result<(), String> {
    let client = build_localhost_http_client(Duration::from_secs(3), "请求诊断设置")?;
    let response = client.post(format!("http://{CODEX_LOCAL_ACCESS_DEFAULT_CLIENT_URL_HOST}:{port}/v1/cockpit/diagnostics/config"))
        .bearer_auth(internal_api_service_key())
        .json(&json!({"requestPayloadLogging":enabled})).send().await
        .map_err(|error| format!("请求 Sidecar 诊断设置失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Sidecar 诊断设置失败: HTTP {}", response.status()));
    }
    Ok(())
}

fn schedule_request_payload_settings_sync() {
    if REQUEST_PAYLOAD_SETTINGS_SYNC_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        let mut processed_revision = REQUEST_PAYLOAD_SETTINGS_REVISION.load(Ordering::SeqCst);
        let mut retries = 0_u8;
        loop {
            let revision = REQUEST_PAYLOAD_SETTINGS_REVISION.load(Ordering::SeqCst);
            if revision != processed_revision {
                retries = 0;
                processed_revision = revision;
            }
            let enabled = REQUEST_PAYLOAD_LOGGING_ENABLED.load(Ordering::SeqCst);
            let ports = running_request_diagnostic_ports(enabled).await;
            let failures = stream::iter(ports)
                .map(|port| async move {
                    send_request_payload_setting_to_sidecar(port, enabled)
                        .await
                        .err()
                })
                .buffer_unordered(4)
                .filter_map(|error| async move { error })
                .collect::<Vec<_>>()
                .await;
            if revision != REQUEST_PAYLOAD_SETTINGS_REVISION.load(Ordering::SeqCst) {
                continue;
            }
            if failures.is_empty() {
                if let Ok(mut error) = REQUEST_PAYLOAD_SETTINGS_ERROR.lock() {
                    *error = None;
                }
                break;
            }
            retries += 1;
            if retries >= 3 {
                if let Ok(mut error) = REQUEST_PAYLOAD_SETTINGS_ERROR.lock() {
                    *error = Some((revision, failures.join("; ")));
                }
                logger::log_codex_api_warn(&format!(
                    "请求正文设置已保存，但运行网关更新失败；下次启动会应用保存值: {}",
                    failures.join("; ")
                ));
                break;
            }
            tokio::time::sleep(Duration::from_millis(u64::from(retries) * 500)).await;
        }
        REQUEST_PAYLOAD_SETTINGS_SYNC_RUNNING.store(false, Ordering::SeqCst);
        // Close the race with a toggle arriving just before the running flag cleared.
        if processed_revision != REQUEST_PAYLOAD_SETTINGS_REVISION.load(Ordering::SeqCst) {
            schedule_request_payload_settings_sync();
        }
        emit_local_access_state_updated();
    });
}

fn clear_request_payloads_from_db(conn: &Connection) -> Result<u64, String> {
    if !diagnostics_table_exists(conn).map_err(|error| error.to_string())? {
        return Ok(0);
    }
    conn.execute("UPDATE request_diagnostics SET payloads_json='[]',payload_bytes=0 WHERE payloads_json!='[]'", [])
        .map(|rows| rows as u64).map_err(|error| format!("清除请求正文失败: {error}"))
}

pub async fn clear_local_access_request_payloads() -> Result<u64, String> {
    REQUEST_DIAGNOSTICS_PAYLOAD_CLEARED_BEFORE.store(now_ms(), Ordering::SeqCst);
    REQUEST_DIAGNOSTICS_PAYLOAD_EPOCH.fetch_add(1, Ordering::SeqCst);
    tauri::async_runtime::spawn_blocking(move || {
        let (_guard, conn) = open_local_access_logs_db_for_write()?;
        clear_request_payloads_from_db(&conn)
    })
    .await
    .map_err(|error| format!("清除请求正文任务失败: {error}"))?
}
