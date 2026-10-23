// Historical category compatibility is background maintenance, never an open-
// database side effect. New/imported rows are classified as they are written.
const ERROR_CATEGORY_MAINTENANCE_VERSION: i64 = 1;
const ERROR_CATEGORY_MAINTENANCE_BATCH: i64 = 128;
const ERROR_CATEGORY_MESSAGE_LIMIT: i64 = 65536;
const ERROR_CATEGORY_BATCH_BUDGET: Duration = Duration::from_millis(150);

fn normalized_request_log_error_category<'a>(
    success: bool,
    category: &'a str,
    message: &str,
) -> std::borrow::Cow<'a, str> {
    if success
        || category == "upstream_response_failed"
        || message.len() > ERROR_CATEGORY_MESSAGE_LIMIT as usize
    {
        return std::borrow::Cow::Borrowed(category);
    }
    if is_upstream_response_failed_error_message(message) {
        std::borrow::Cow::Borrowed("upstream_response_failed")
    } else if is_stream_incomplete_error_message(message) {
        std::borrow::Cow::Borrowed("stream_incomplete")
    } else {
        std::borrow::Cow::Borrowed(category)
    }
}

fn normalize_request_log_event_category(event: &mut CodexLocalAccessUsageEvent) {
    let next = normalized_request_log_error_category(
        event.success,
        &event.error_category,
        &event.error_message,
    );
    if next != event.error_category.as_str() {
        event.error_category = next.into_owned();
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ErrorCategoryMaintenanceBatch {
    complete: bool,
    visited: usize,
    changed: usize,
    oversized: usize,
}

fn maintain_request_log_error_category_batch(
    conn: &Connection,
) -> Result<ErrorCategoryMaintenanceBatch, String> {
    let started = std::time::Instant::now();
    let check_budget = || {
        if started.elapsed() >= ERROR_CATEGORY_BATCH_BUDGET {
            Err("请求日志分类后台兼容批次超时".to_string())
        } else {
            Ok(())
        }
    };
    with_request_log_sql_budget(conn, ERROR_CATEGORY_BATCH_BUDGET, || {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS request_log_maintenance (
                name TEXT PRIMARY KEY, version INTEGER NOT NULL,
                cursor INTEGER NOT NULL, high_water INTEGER NOT NULL,
                complete INTEGER NOT NULL DEFAULT 0
             );",
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO request_log_maintenance(name, version, cursor, high_water)
             VALUES ('error_category', ?1, 0, (SELECT COALESCE(MAX(id), 0) FROM request_logs))
             ON CONFLICT(name) DO NOTHING",
                [ERROR_CATEGORY_MAINTENANCE_VERSION],
            )
            .map_err(|error| error.to_string())?;
        let (version, cursor, high_water, complete): (i64, i64, i64, bool) = transaction
            .query_row(
                "SELECT version, cursor, high_water, complete
             FROM request_log_maintenance WHERE name = 'error_category'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| error.to_string())?;
        if version > ERROR_CATEGORY_MAINTENANCE_VERSION
            || (version == ERROR_CATEGORY_MAINTENANCE_VERSION && complete)
        {
            check_budget()?;
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(ErrorCategoryMaintenanceBatch {
                complete: true,
                visited: 0,
                changed: 0,
                oversized: 0,
            });
        }
        let (cursor, high_water) = if version < ERROR_CATEGORY_MAINTENANCE_VERSION {
            let high_water = transaction
                .query_row("SELECT COALESCE(MAX(id), 0) FROM request_logs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|error| error.to_string())?;
            (0, high_water)
        } else {
            (cursor, high_water)
        };
        let rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, success, error_category,
                        CASE WHEN success = 0 AND length(CAST(error_message AS BLOB)) <= ?3
                             THEN error_message ELSE NULL END
                 FROM request_logs WHERE id > ?1 AND id <= ?2 ORDER BY id LIMIT ?4",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(
                    params![
                        cursor,
                        high_water,
                        ERROR_CATEGORY_MESSAGE_LIMIT,
                        ERROR_CATEGORY_MAINTENANCE_BATCH
                    ],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, bool>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    },
                )
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        let mut changed = 0;
        let mut oversized = 0;
        for (id, success, category, message) in &rows {
            check_budget()?;
            let Some(message) = message else {
                if !success {
                    oversized += 1;
                }
                continue;
            };
            let next = normalized_request_log_error_category(*success, category, message);
            if next != category.as_str() {
                changed += transaction.execute(
                    "UPDATE request_logs SET error_category = ?1 WHERE id = ?2 AND error_category = ?3",
                    params![next.as_ref(), id, category],
                ).map_err(|error| error.to_string())?;
            }
        }
        let next_cursor = rows.last().map(|row| row.0).unwrap_or(high_water);
        let complete =
            next_cursor >= high_water || rows.len() < ERROR_CATEGORY_MAINTENANCE_BATCH as usize;
        transaction.execute(
            "UPDATE request_log_maintenance SET version = ?1, cursor = ?2, high_water = ?3, complete = ?4
             WHERE name = 'error_category'",
            params![ERROR_CATEGORY_MAINTENANCE_VERSION, next_cursor, high_water, complete],
        ).map_err(|error| error.to_string())?;
        // The cursor and mutations commit together. A crash/interruption can
        // never skip an uncommitted batch; retrying an old cursor is idempotent.
        check_budget()?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(ErrorCategoryMaintenanceBatch {
            complete,
            visited: rows.len(),
            changed,
            oversized,
        })
    })
}

fn ensure_request_log_error_maintenance_started() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    static NEXT_CHECK: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    if now_ms() < NEXT_CHECK.load(Ordering::Acquire)
        || RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
    {
        return;
    }
    struct Flight;
    impl Drop for Flight {
        fn drop(&mut self) {
            NEXT_CHECK.store(now_ms().saturating_add(60_000), Ordering::Release);
            RUNNING.store(false, Ordering::Release);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _flight = Flight;
        let result = (|| -> Result<(), String> {
            let path = local_access_logs_db_path()?;
            if !path.is_file() {
                return Ok(());
            }
            let conn =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|error| error.to_string())?;
            conn.busy_timeout(Duration::from_millis(20))
                .map_err(|error| error.to_string())?;
            if !request_logs_table_exists(&conn).map_err(|error| error.to_string())? {
                return Ok(());
            }
            let mut changed = 0;
            let mut oversized = 0;
            loop {
                if crate::modules::app_lifecycle::is_shutdown_started() {
                    return Ok(());
                }
                let batch = {
                    // Low-priority work yields to every existing writer. Never
                    // wait on its mutex or retain it during the between-batch pause.
                    match local_access_logs_db_write_lock().try_lock() {
                        Ok(_guard) => Some(maintain_request_log_error_category_batch(&conn)?),
                        Err(std::sync::TryLockError::WouldBlock) => None,
                        Err(std::sync::TryLockError::Poisoned(_)) => {
                            return Err("请求日志写入队列已损坏".into())
                        }
                    }
                };
                if let Some(batch) = batch {
                    changed += batch.changed;
                    oversized += batch.oversized;
                    if batch.complete {
                        if changed > 0 || oversized > 0 {
                            logger::log_codex_api_info(&format!(
                                "请求日志分类后台兼容完成: updated={changed}, oversized_preserved={oversized}"
                            ));
                        }
                        return Ok(());
                    }
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        })();
        if let Err(error) = result {
            logger::log_codex_api_warn(&format!(
                "请求日志分类后台兼容暂未完成，将从检查点重试: {error}"
            ));
        }
    });
}

#[cfg(test)]
#[path = "codex_local_access_log_maintenance_tests.rs"]
mod log_category_maintenance_tests;
