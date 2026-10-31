// Public DTOs are built from an explicit SQL projection. Never serialize internal
// UsageEvent/RequestDetail objects, diagnostics, upstream models or error messages.
fn remote_query_bounds(
    query: &crate::modules::codex_remote_service::KeyQuery,
    now: i64,
) -> Result<(i64, i64), String> {
    let day = 86_400_000;
    let start = query.start_at.unwrap_or(now / day * day - 6 * day);
    let end = query
        .end_at
        .unwrap_or((now / REMOTE_HOUR + 1) * REMOTE_HOUR);
    if start < 0
        || end <= start
        || end - start > 93 * day
        || end > now + day
        || start % REMOTE_HOUR != 0
        || end % REMOTE_HOUR != 0
        || query.model.as_ref().is_some_and(|m| m.len() > 256)
        || query.kind.as_ref().is_some_and(|k| {
            !matches!(
                k.as_str(),
                "text" | "image_generation" | "image_edit" | "other"
            )
        })
        || query.before_time.is_some() != query.before_id.is_some()
        || query.before_id.is_some_and(|id| id < 1)
        || query.service_tier.as_ref().is_some_and(|v| v.len() > 32)
        || (query.installation_id.is_some()
            && (query.thread_id.is_none() || query.instance_id.is_none()))
        || query
            .thread_id
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
        || [&query.instance_id, &query.installation_id]
            .iter()
            .any(|v| {
                v.as_ref()
                    .is_some_and(|v| v.len() > 128 || v.chars().any(char::is_control))
            })
    {
        return Err("invalid_query".into());
    }
    Ok((start, end))
}

fn remote_public_usage(row: &rusqlite::Row<'_>, offset: usize) -> Result<Value, SqliteError> {
    Ok(json!({
        "requestCount":row.get::<_,i64>(offset)?,"successCount":row.get::<_,i64>(offset+1)?,
        "canceledCount":row.get::<_,i64>(offset+2)?,"imageRequestCount":row.get::<_,i64>(offset+3)?,
        "totalLatencyMs":row.get::<_,i64>(offset+4)?,"inputTokens":row.get::<_,i64>(offset+5)?,
        "outputTokens":row.get::<_,i64>(offset+6)?,"totalTokens":row.get::<_,i64>(offset+7)?,
        "cachedTokens":row.get::<_,i64>(offset+8)?,"estimatedCostUsd":row.get::<_,f64>(offset+9)?,
        "settledMicrousd":row.get::<_,i64>(offset+10)?,"unrecordedCount":row.get::<_,i64>(offset+11)?,
        "unmeteredCount":row.get::<_,i64>(offset+12)?,
    }))
}

fn query_remote_key_stats(
    conn: &Connection,
    key: &str,
    start: i64,
    end: i64,
) -> Result<Value, String> {
    query_remote_key_stats_projection(conn, key, start, end, false)
}

fn query_remote_key_stats_projection(
    conn: &Connection,
    key: &str,
    start: i64,
    end: i64,
    previous: bool,
) -> Result<Value, String> {
    let legacy: bool = previous
        && conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cockpit_key_hourly')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
    let source = if legacy {
        "cockpit_key_hourly WHERE".to_string()
    } else {
        format!(
        "cockpit_key_hourly_v2 WHERE generation=(SELECT {} FROM cockpit_key_analytics_state WHERE id=1) AND",
        if previous {"previous_generation"} else {"generation"})
    };
    let interval = if end - start <= 36 * REMOTE_HOUR {
        REMOTE_HOUR
    } else {
        24 * REMOTE_HOUR
    };
    let fields = remote_aggregate_columns("NEW");
    let sums = fields
        .iter()
        .map(|(name, _)| format!("COALESCE(SUM({name}),0)"))
        .collect::<Vec<_>>()
        .join(",");
    let summary = conn
        .query_row(
            &format!("SELECT {sums} FROM {source} key_id=?1 AND bucket>=?2 AND bucket<?3"),
            params![key, start, end],
            |r| remote_public_usage(r, 0),
        )
        .map_err(|e| e.to_string())?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT (bucket/?4)*?4 AS t,{sums} FROM {source}
        key_id=?1 AND bucket>=?2 AND bucket<?3 GROUP BY t ORDER BY t"
        ))
        .map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![key, start, end, interval], |r| {
            Ok((r.get::<_, i64>(0)?, remote_public_usage(r, 1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| e.to_string())?;
    let empty = conn
        .query_row("SELECT 0,0,0,0,0,0,0,0,0,0.0,0,0,0", [], |r| {
            remote_public_usage(r, 0)
        })
        .map_err(|e| e.to_string())?;
    let mut trend = Vec::new();
    let mut bucket = start / interval * interval;
    while bucket < end {
        trend.push(json!({"bucketStart":bucket,"usage":rows.get(&bucket).unwrap_or(&empty)}));
        bucket += interval;
    }
    Ok(json!({"summary":summary,"trend":trend,"hourly":interval==REMOTE_HOUR}))
}

fn query_remote_key_requests(
    conn: &Connection,
    key: &str,
    start: i64,
    end: i64,
    query: &crate::modules::codex_remote_service::KeyQuery,
) -> Result<Value, String> {
    let mut clauses = vec![
        "api_key_id=?".to_string(),
        "timestamp>=?".into(),
        "timestamp<?".into(),
    ];
    let mut values = vec![
        SqlValue::Text(key.into()),
        SqlValue::Integer(start),
        SqlValue::Integer(end),
    ];
    let scoped = query.thread_id.is_some() && query.instance_id.is_some();
    let source = if scoped {
        let thread = uuid::Uuid::parse_str(query.thread_id.as_deref().unwrap())
            .map_err(|_| "invalid thread")?
            .hyphenated()
            .to_string();
        clauses.extend([
            "s.api_key_id=?".into(),
            "s.client_instance_id=?".into(),
            "s.client_thread_id=?".into(),
        ]);
        values.extend([
            SqlValue::Text(key.into()),
            SqlValue::Text(query.instance_id.clone().unwrap()),
            SqlValue::Text(thread),
        ]);
        clauses.extend(["s.timestamp>=?".into(), "s.timestamp<?".into()]);
        values.extend([SqlValue::Integer(start), SqlValue::Integer(end)]);
        if let Some(installation) = &query.installation_id {
            clauses.push("s.client_installation_id=?".into());
            values.push(SqlValue::Text(installation.clone()));
        }
        "request_display_summaries s INDEXED BY idx_request_display_chat CROSS JOIN request_logs l ON l.id=s.log_id"
    } else {
        remote_request_scope(&mut clauses, &mut values, query)?;
        "request_logs l"
    };
    // Qualify log fields once so the indexed summary join can drive chat pages.
    for clause in &mut clauses {
        if !clause.starts_with("s.") {
            *clause = format!("l.{clause}");
        }
    }
    if let (Some(time), Some(id)) = (query.before_time, query.before_id) {
        // ASC rowid matches SQLite's existing (api_key_id,timestamp DESC) index.
        clauses.push(
            if scoped {
                "s.timestamp<=? AND (s.timestamp<? OR s.log_id<?)"
            } else {
                "l.timestamp<=? AND (l.timestamp<? OR l.id>?)"
            }
            .into(),
        );
        values.extend([
            SqlValue::Integer(time),
            SqlValue::Integer(time),
            SqlValue::Integer(id),
        ]);
    }
    if let Some(model) = query.model.as_deref().filter(|m| !m.is_empty()) {
        clauses.push("l.requested_model=?".into());
        values.push(SqlValue::Text(model.into()));
    }
    if let Some(success) = query.success {
        clauses.push("l.success=?".into());
        values.push(SqlValue::Integer(i64::from(success)));
    }
    if let Some(kind) = &query.kind {
        clauses.push("l.request_kind=?".into());
        values.push(SqlValue::Text(kind.clone()));
    }
    let limit = query.page_size.unwrap_or(30).clamp(1, 100) as usize;
    values.push(SqlValue::Integer(limit as i64 + 1));
    let order = if scoped {
        "s.timestamp DESC,s.log_id DESC"
    } else {
        "l.timestamp DESC,l.id ASC"
    };
    let sql=format!("SELECT l.id,l.timestamp,substr(l.request_id,1,128),substr(l.requested_model,1,256),substr(l.request_kind,1,32),l.success,
        l.http_status,substr(l.error_category,1,64),l.latency_ms,l.first_response_ms,l.input_tokens,l.output_tokens,l.cached_tokens,l.total_tokens,
        l.estimated_cost_usd,l.settled_microusd,substr(l.settlement_status,1,32),l.settled_at,l.output_generation_ms FROM {source}
        WHERE {} ORDER BY {order} LIMIT ?",clauses.join(" AND "));
    let mut statement = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut items=statement.query_map(rusqlite::params_from_iter(values),|r| {
        let success:bool=r.get(5)?;
        let category:String=r.get(7)?;
        // Error codes are chosen here; no upstream text reaches the caller.
        let outcome=if success {"success"} else {match category.as_str() {
            "client_canceled"=>"canceled", "stream_incomplete"=>"incomplete",
            "timeout"|"upstream_timeout"|"request_timeout"=>"timeout", _=>"failed",
        }};
        Ok(json!({"id":r.get::<_,i64>(0)?,"timestamp":r.get::<_,i64>(1)?,"requestId":r.get::<_,String>(2)?,
            "model":r.get::<_,String>(3)?,"kind":r.get::<_,String>(4)?,"outcome":outcome,
            "httpStatus":r.get::<_,Option<i64>>(6)?,"latencyMs":r.get::<_,i64>(8)?,
            "firstResponseMs":r.get::<_,Option<i64>>(9)?,"inputTokens":r.get::<_,i64>(10)?,
            "outputTokens":r.get::<_,i64>(11)?,"cachedTokens":r.get::<_,i64>(12)?,"totalTokens":r.get::<_,i64>(13)?,
            "estimatedCostUsd":r.get::<_,f64>(14)?,"settledMicrousd":r.get::<_,Option<i64>>(15)?,
            "settlementStatus":r.get::<_,String>(16)?,"settledAt":r.get::<_,Option<i64>>(17)?,"outputGenerationMs":r.get::<_,Option<i64>>(18)?}))
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    let more = items.len() > limit;
    items.truncate(limit);
    let next = if more {
        items
            .last()
            .map(|row| json!({"beforeTime":row["timestamp"],"beforeId":row["id"]}))
    } else {
        None
    };
    Ok(json!({"items":items,"nextCursor":next}))
}

fn read_remote_key_data(
    key: &str,
    query: &crate::modules::codex_remote_service::KeyQuery,
    resource: &str,
) -> Result<Value, String> {
    let logs = resource == "requests";
    use rusqlite::OptionalExtension;
    let (start, end) = remote_query_bounds(query, now_ms())?;
    let path = local_access_logs_db_path()?;
    if !path.is_file() {
        return Ok(json!({"ready":false,"updatedAt":now_ms()}));
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(50))
        .map_err(|e| e.to_string())?;
    with_request_log_sql_budget(&conn, Duration::from_secs(2), || {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='cockpit_key_analytics_state')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let current = exists && remote_analytics_schema_current(&tx).map_err(|e| e.to_string())?;
        let ready = current
            && tx
                .query_row(
                    "SELECT complete FROM cockpit_key_analytics_state WHERE id=1",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?;
        // Logs are immediately readable; charts wait for complete aggregates.
        let mut response = if logs
            && query.thread_id.is_some()
            && query.instance_id.is_none()
            && !session_query_index_ready(&tx)
        {
            json!({"ready":false})
        } else if resource == "session" {
            query_remote_key_session(&tx, key, query)?
        } else if logs && exists {
            query_remote_key_requests(&tx, key, start, end, query)?
        } else if ready {
            query_remote_key_stats(&tx, key, start, end)?
        } else if current
            && tx
                .query_row(
                    "SELECT previous_complete FROM cockpit_key_analytics_state WHERE id=1",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
        {
            let mut previous = query_remote_key_stats_projection(&tx, key, start, end, true)?;
            previous["previousSnapshot"] = json!(true);
            previous
        } else {
            json!({})
        };
        let first:Option<i64>=tx.query_row("SELECT timestamp FROM request_logs WHERE api_key_id=?1 ORDER BY timestamp ASC LIMIT 1",[key],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if let Some(points) = response.get_mut("trend").and_then(Value::as_array_mut) {
            let interval = if end - start <= 36 * REMOTE_HOUR {
                REMOTE_HOUR
            } else {
                24 * REMOTE_HOUR
            };
            points.retain(|point| {
                first.is_some_and(|first| {
                    point["bucketStart"].as_i64().unwrap_or(0) + interval > first
                })
            });
        }
        if response.get("ready").is_none() {
            response["ready"] = json!(if logs { exists } else { ready });
        }
        response["startAt"] = json!(start);
        response["endAt"] = json!(end);
        response["timezone"] = json!("UTC");
        response["firstRetainedAt"] = json!(first);
        response["updatedAt"] = json!(now_ms());
        Ok(response)
    })
}

async fn native_remote_key_query(
    caller: &native_contract::Caller,
    query: crate::modules::codex_remote_service::KeyQuery,
    logs: bool,
) -> Result<Value, native_contract::ServiceError> {
    native_remote_key_resource(caller, query, if logs { "requests" } else { "stats" }).await
}

async fn native_remote_key_resource(
    caller: &native_contract::Caller,
    query: crate::modules::codex_remote_service::KeyQuery,
    resource: &'static str,
) -> Result<Value, native_contract::ServiceError> {
    remote_query_bounds(&query, now_ms()).map_err(|_| {
        native_error(
            400,
            "invalid_query",
            "Use an hourly aligned range of at most 93 days",
        )
    })?;
    ensure_remote_analytics_started();
    static IO: std::sync::LazyLock<Arc<Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(Semaphore::new(2)));
    static CACHE: std::sync::LazyLock<Mutex<HashMap<String, (i64, Value)>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    static FLIGHTS: std::sync::LazyLock<Mutex<HashMap<String, std::sync::Weak<TokioMutex<()>>>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
    let signature = serde_json::to_string(&(
        caller.id.as_str(),
        &query,
        resource,
        REMOTE_ANALYTICS_REVISION.load(Ordering::Acquire),
        REQUEST_LOG_WRITE_EPOCH.load(Ordering::Acquire),
    ))
    .unwrap_or_default();
    let cached = || {
        CACHE.lock().ok().and_then(|c| {
            c.get(&signature)
                .filter(|(at, _)| now_ms() - at < 5_000)
                .map(|(_, v)| v.clone())
        })
    };
    if let Some(value) = cached() {
        return Ok(value);
    }
    let flight = {
        let mut flights = FLIGHTS
            .lock()
            .map_err(|_| native_error(503, "query_busy", "Usage query unavailable"))?;
        flights.retain(|_, v| v.strong_count() > 0);
        let flight = flights
            .get(&signature)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| Arc::new(TokioMutex::new(())));
        flights.insert(signature.clone(), Arc::downgrade(&flight));
        flight
    };
    let _flight = tokio::time::timeout(Duration::from_secs(3), flight.lock())
        .await
        .map_err(|_| native_error(503, "query_busy", "Usage query busy"))?;
    if let Some(value) = cached() {
        return Ok(value);
    }
    let permit = tokio::time::timeout(Duration::from_millis(200), IO.clone().acquire_owned())
        .await
        .map_err(|_| native_error(503, "query_busy", "Usage query busy"))?
        .map_err(|_| native_error(503, "query_busy", "Usage query unavailable"))?;
    let key = caller.id.clone();
    let value = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read_remote_key_data(&key, &query, resource)
    })
    .await
    .map_err(|_| native_error(503, "query_failed", "Usage query failed"))?
    .map_err(|error| {
        logger::log_codex_api_warn(&format!("Remote Key query failed: {error}"));
        native_error(503, "query_failed", "Usage query failed; retry later")
    })?;
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, (at, _)| now_ms() - *at < 5_000);
        if cache.len() < 128 {
            cache.insert(signature, (now_ms(), value.clone()));
        }
    }
    Ok(value)
}
