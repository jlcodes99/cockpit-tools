// Account-card window counters are read-only, independently bounded queries.
// They do not initialize a gateway, import JSON history, or migrate a schema.
const ACCOUNT_WINDOW_QUERY_LIMIT: usize = 4096;
const ACCOUNT_WINDOW_QUERY_BUDGET: Duration = Duration::from_millis(1500);
const ACCOUNT_WINDOW_AGGREGATE_SQL: &str = r#"
    SELECT COUNT(*),
           COALESCE(SUM(MAX(COALESCE(input_tokens, 0), 0)), 0),
           COALESCE(SUM(MAX(COALESCE(output_tokens, 0), 0)), 0),
           COALESCE(SUM(MAX(COALESCE(total_tokens, 0), 0)), 0),
           COALESCE(SUM(MAX(COALESCE(cached_tokens, 0), 0)), 0),
           COALESCE(SUM(CASE WHEN estimated_cost_usd > 0
                            AND estimated_cost_usd <= 1.7976931348623157e308
                            THEN estimated_cost_usd ELSE 0 END), 0)
    FROM request_logs
    WHERE account_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
"#;

fn account_window_query_slots() -> &'static std::sync::Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    SLOTS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
}

pub async fn query_local_access_account_window_stats(
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    validate_account_window_queries(&queries)?;
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    run_bounded_account_window_query(move || {
        query_local_access_account_window_stats_blocking(queries)
    })
    .await
}

async fn run_bounded_account_window_query(
    query: impl FnOnce() -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> + Send + 'static,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    let permit = account_window_query_slots()
        .clone()
        .try_acquire_owned()
        .map_err(|_| "API 服务账号窗口统计繁忙，请稍后重试".to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        // The physical slot stays in this closure even if the caller goes away.
        // Interrupting/abandoning an await must not create replacement workers.
        let _permit = permit;
        query()
    })
    .await
    .map_err(|error| format!("统计 API 服务账号窗口用量任务失败: {error}"))?
}

fn validate_account_window_queries(
    queries: &[CodexLocalAccessAccountWindowQuery],
) -> Result<(), String> {
    if queries.len() > ACCOUNT_WINDOW_QUERY_LIMIT {
        return Err("API 服务账号窗口统计范围过大".into());
    }
    let mut keys = HashSet::new();
    for query in queries {
        let account_id = query.account_id.trim();
        let window_key = query.window_key.trim();
        let start = normalize_request_log_time_bound(query.start_at);
        let end = normalize_request_log_time_bound(query.end_at);
        if account_id.is_empty()
            || account_id.len() > 256
            || window_key.is_empty()
            || window_key.len() > 64
            || account_id.chars().any(char::is_control)
            || window_key.chars().any(char::is_control)
            || start < 0
            || end < start
            || !keys.insert((account_id, window_key))
        {
            return Err("API 服务账号窗口统计参数无效".into());
        }
    }
    Ok(())
}

fn normalize_request_log_time_bound(value: i64) -> i64 {
    if value > 0 && value < 1_000_000_000_000 {
        value.saturating_mul(1000)
    } else {
        value
    }
}

// Team/workspace IDs can be shared; only local account IDs isolate members.
#[cfg(test)]
fn account_window_stat_identity_matches(row_account_id: &str, requested_account_id: &str) -> bool {
    let requested = requested_account_id.trim();
    !requested.is_empty() && row_account_id.trim() == requested
}

fn with_request_log_sql_budget<T>(
    conn: &Connection,
    budget: Duration,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    struct Completion(Option<std::sync::mpsc::Sender<()>>);
    impl Drop for Completion {
        fn drop(&mut self) {
            if let Some(finished) = self.0.take() {
                let _ = finished.send(());
            }
        }
    }
    let interrupt = conn.get_interrupt_handle();
    let (finished, completion) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|threads| {
        std::thread::Builder::new()
            .name("codex-log-query-budget".into())
            .spawn_scoped(threads, move || {
                if matches!(
                    completion.recv_timeout(budget),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    // SQLite ignores interrupts between statements. Keep the
                    // deadline armed until the physical operation has stopped.
                    loop {
                        interrupt.interrupt();
                        if !matches!(
                            completion.recv_timeout(Duration::from_millis(10)),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                        ) {
                            break;
                        }
                    }
                }
            })
            .map_err(|error| format!("创建请求日志查询预算失败: {error}"))?;
        // Also signal during unwinding: a panicking worker must not keep its
        // scoped interrupt thread (and physical slot) alive forever.
        let _completion = Completion(Some(finished));
        let result = operation();
        result
    })
}

fn query_local_access_account_window_stats_blocking(
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    let path = local_access_logs_db_path()?;
    query_local_access_account_window_stats_at_path(&path, queries)
}

fn query_local_access_account_window_stats_at_path(
    path: &Path,
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("API 服务账号窗口统计数据库尚未就绪: {error}"))?;
    conn.busy_timeout(Duration::from_millis(75))
        .map_err(|error| error.to_string())?;
    query_local_access_account_window_stats_from_conn(&conn, queries)
}

fn query_local_access_account_window_stats_from_conn(
    conn: &Connection,
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    validate_account_window_queries(&queries)?;
    if queries.is_empty() {
        return Ok(Vec::new());
    }
    let started = std::time::Instant::now();
    with_request_log_sql_budget(conn, ACCOUNT_WINDOW_QUERY_BUDGET, || {
        // All account windows share one WAL snapshot, including concurrent
        // writes/clear. Return a complete result or an error, never a mixture.
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let mut statement = transaction
            .prepare(ACCOUNT_WINDOW_AGGREGATE_SQL)
            .map_err(|error| format!("准备 API 服务账号窗口统计失败: {error}"))?;
        let mut totals = Vec::with_capacity(queries.len());
        for query in queries {
            if started.elapsed() >= ACCOUNT_WINDOW_QUERY_BUDGET {
                return Err("API 服务账号窗口统计超时，请稍后重试".into());
            }
            let mut stats = statement
                .query_row(
                    params![
                        query.account_id.trim(),
                        normalize_request_log_time_bound(query.start_at),
                        normalize_request_log_time_bound(query.end_at),
                    ],
                    |row| {
                        // Integer SUM is exact. Overflow is an explicit error rather
                        // than silently rounding large token counters through f64.
                        Ok(CodexLocalAccessAccountWindowStats {
                            account_id: query.account_id.trim().to_string(),
                            window_key: query.window_key.trim().to_string(),
                            request_count: row.get::<_, i64>(0)?.max(0) as u64,
                            input_tokens: row.get::<_, i64>(1)?.max(0) as u64,
                            output_tokens: row.get::<_, i64>(2)?.max(0) as u64,
                            total_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                            cached_tokens: row.get::<_, i64>(4)?.max(0) as u64,
                            estimated_cost_usd: row.get(5)?,
                        })
                    },
                )
                .map_err(|error| format!("读取 API 服务账号窗口统计失败: {error}"))?;
            if !stats.estimated_cost_usd.is_finite() {
                return Err("API 服务账号窗口金额统计超出范围".into());
            }
            stats.estimated_cost_usd = stats.estimated_cost_usd.max(0.0);
            totals.push(stats);
        }
        if started.elapsed() >= ACCOUNT_WINDOW_QUERY_BUDGET {
            return Err("API 服务账号窗口统计超时，请稍后重试".into());
        }
        drop(statement);
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(totals)
    })
}

#[cfg(test)]
#[path = "codex_local_access_window_stats_tests.rs"]
mod account_window_query_tests;
