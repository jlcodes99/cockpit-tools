// Numeric projections for reports; no request body or token-breakdown JSON is read.
fn report_query_slots() -> &'static std::sync::Arc<tokio::sync::Semaphore> {
    static SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    SLOTS.get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(2)))
}

const REQUEST_USAGE_ROLLUP: crate::modules::usage_rollup::Rollup = crate::modules::usage_rollup::Rollup {
    source: "request_logs", prefix: "request_usage_v1", width: 60_000,
    dimensions: &["account_id","email","api_key_id","api_key_label","model_id"],
    metrics: &[
        crate::modules::usage_rollup::Metric { name: "success_count", expression: "($success != 0)", real: false },
        crate::modules::usage_rollup::Metric { name: "failure_count", expression: "($success = 0)", real: false },
        crate::modules::usage_rollup::Metric { name: "client_canceled_count", expression: "(lower(trim($error_category)) = 'client_canceled')", real: false },
        crate::modules::usage_rollup::Metric { name: "upstream_response_failed_count", expression: "(lower(trim($error_category)) = 'upstream_response_failed')", real: false },
        crate::modules::usage_rollup::Metric { name: "stream_incomplete_count", expression: "(lower(trim($error_category)) = 'stream_incomplete')", real: false },
        crate::modules::usage_rollup::Metric { name: "total_latency_ms", expression: "CASE WHEN $success != 0 THEN MAX($latency_ms,0) ELSE 0 END", real: false },
        crate::modules::usage_rollup::Metric { name: "text_request_count", expression: "($request_kind = 'text')", real: false },
        crate::modules::usage_rollup::Metric { name: "image_request_count", expression: "($request_kind IN ('image_generation','image_edit'))", real: false },
        crate::modules::usage_rollup::Metric { name: "image_generation_request_count", expression: "($request_kind = 'image_generation')", real: false },
        crate::modules::usage_rollup::Metric { name: "image_edit_request_count", expression: "($request_kind = 'image_edit')", real: false },
        crate::modules::usage_rollup::Metric { name: "image_generation_capability_failure_count", expression: "(trim($error_category) IN ('image_generation_not_enabled','image_generation_disabled'))", real: false },
        crate::modules::usage_rollup::Metric { name: "input_tokens", expression: "MAX($input_tokens,0)", real: false },
        crate::modules::usage_rollup::Metric { name: "output_tokens", expression: "MAX($output_tokens,0)", real: false },
        crate::modules::usage_rollup::Metric { name: "total_tokens", expression: "MAX($total_tokens,0)", real: false },
        crate::modules::usage_rollup::Metric { name: "cached_tokens", expression: "MAX($cached_tokens,0)", real: false },
        crate::modules::usage_rollup::Metric { name: "reasoning_tokens", expression: "MAX($reasoning_tokens,0)", real: false },
        crate::modules::usage_rollup::Metric { name: "estimated_cost_usd", expression: "CASE WHEN $estimated_cost_usd>0 AND $estimated_cost_usd<=1.7976931348623157e308 THEN $estimated_cost_usd ELSE 0 END", real: true },
    ],
};

fn schedule_request_rollup_backfill() {
    let Ok(path) = local_access_logs_db_path() else {
        return;
    };
    crate::modules::background_maintenance::schedule(
        format!("request-rollup:{}", path.display()),
        move || {
            let Ok(_guard) = local_access_logs_db_write_lock().try_lock() else {
                return Ok(false);
            };
            let conn =
                Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                    .map_err(|e| e.to_string())?;
            conn.busy_timeout(Duration::from_millis(10))
                .map_err(|e| e.to_string())?;
            with_request_log_sql_budget(
                &conn,
                crate::modules::background_maintenance::BATCH_TIME,
                || REQUEST_USAGE_ROLLUP.batch(&conn).map_err(|e| e.to_string()),
            )
        },
    );
}

fn usage_stats_from_rollup(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CodexLocalAccessUsageStats> {
    Ok(CodexLocalAccessUsageStats {
        request_count: row.get::<_, i64>("request_count")?.max(0) as u64,
        success_count: row.get::<_, i64>("success_count")?.max(0) as u64,
        failure_count: row.get::<_, i64>("failure_count")?.max(0) as u64,
        client_canceled_count: row.get::<_, i64>("client_canceled_count")?.max(0) as u64,
        upstream_response_failed_count: row.get::<_, i64>("upstream_response_failed_count")?.max(0)
            as u64,
        stream_incomplete_count: row.get::<_, i64>("stream_incomplete_count")?.max(0) as u64,
        total_latency_ms: row.get::<_, i64>("total_latency_ms")?.max(0) as u64,
        text_request_count: row.get::<_, i64>("text_request_count")?.max(0) as u64,
        image_request_count: row.get::<_, i64>("image_request_count")?.max(0) as u64,
        image_generation_request_count: row.get::<_, i64>("image_generation_request_count")?.max(0)
            as u64,
        image_edit_request_count: row.get::<_, i64>("image_edit_request_count")?.max(0) as u64,
        image_generation_capability_failure_count: row
            .get::<_, i64>("image_generation_capability_failure_count")?
            .max(0) as u64,
        input_tokens: row.get::<_, i64>("input_tokens")?.max(0) as u64,
        output_tokens: row.get::<_, i64>("output_tokens")?.max(0) as u64,
        total_tokens: row.get::<_, i64>("total_tokens")?.max(0) as u64,
        cached_tokens: row.get::<_, i64>("cached_tokens")?.max(0) as u64,
        reasoning_tokens: row.get::<_, i64>("reasoning_tokens")?.max(0) as u64,
        estimated_cost_usd: row.get("estimated_cost_usd")?,
    })
}

fn add_rollup_usage(target: &mut CodexLocalAccessUsageStats, source: &CodexLocalAccessUsageStats) {
    target.request_count = target.request_count.saturating_add(source.request_count);
    target.success_count = target.success_count.saturating_add(source.success_count);
    target.failure_count = target.failure_count.saturating_add(source.failure_count);
    target.client_canceled_count = target
        .client_canceled_count
        .saturating_add(source.client_canceled_count);
    target.upstream_response_failed_count = target
        .upstream_response_failed_count
        .saturating_add(source.upstream_response_failed_count);
    target.stream_incomplete_count = target
        .stream_incomplete_count
        .saturating_add(source.stream_incomplete_count);
    target.total_latency_ms = target
        .total_latency_ms
        .saturating_add(source.total_latency_ms);
    target.text_request_count = target
        .text_request_count
        .saturating_add(source.text_request_count);
    target.image_request_count = target
        .image_request_count
        .saturating_add(source.image_request_count);
    target.image_generation_request_count = target
        .image_generation_request_count
        .saturating_add(source.image_generation_request_count);
    target.image_edit_request_count = target
        .image_edit_request_count
        .saturating_add(source.image_edit_request_count);
    target.image_generation_capability_failure_count = target
        .image_generation_capability_failure_count
        .saturating_add(source.image_generation_capability_failure_count);
    target.input_tokens = target.input_tokens.saturating_add(source.input_tokens);
    target.output_tokens = target.output_tokens.saturating_add(source.output_tokens);
    target.total_tokens = target.total_tokens.saturating_add(source.total_tokens);
    target.cached_tokens = target.cached_tokens.saturating_add(source.cached_tokens);
    target.reasoning_tokens = target
        .reasoning_tokens
        .saturating_add(source.reasoning_tokens);
    target.estimated_cost_usd += source.estimated_cost_usd;
}

fn query_rollup_window(
    conn: &Connection,
    start: i64,
    end: i64,
) -> Result<CodexLocalAccessStatsWindow, String> {
    // One snapshot prevents a concurrently advancing backfill or reprice from
    // moving a row between the bucket and boundary portions of a report.
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT * FROM ({}) ORDER BY updated_at,last_id",
        REQUEST_USAGE_ROLLUP.select(&tx, start, end)
    );
    let mut statement = tx.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = statement.query([]).map_err(|e| e.to_string())?;
    let mut window = empty_stats_window(start, start);
    window.trend_hourly = end.saturating_sub(start) <= 36 * 60 * 60 * 1000;
    let mut accounts: HashMap<String, CodexLocalAccessAccountStats> = HashMap::new();
    let mut models: HashMap<String, CodexLocalAccessModelStats> = HashMap::new();
    let mut keys: HashMap<String, CodexLocalAccessApiKeyStats> = HashMap::new();
    let mut trend: HashMap<i64, CodexLocalAccessUsageTrendPoint> = HashMap::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let usage = usage_stats_from_rollup(row).map_err(|e| e.to_string())?;
        let updated: i64 = row.get("updated_at").map_err(|e| e.to_string())?;
        let timestamp: i64 = row.get("timestamp").map_err(|e| e.to_string())?;
        add_rollup_usage(&mut window.totals, &usage);
        window.updated_at = window.updated_at.max(updated);
        let account: String = row.get("account_id").map_err(|e| e.to_string())?;
        if !account.is_empty() {
            let item =
                accounts
                    .entry(account.clone())
                    .or_insert_with(|| CodexLocalAccessAccountStats {
                        account_id: account,
                        ..Default::default()
                    });
            let email: String = row.get("email").map_err(|e| e.to_string())?;
            if !email.is_empty() {
                item.email = email;
            }
            item.updated_at = updated;
            add_rollup_usage(&mut item.usage, &usage);
        }
        let model: String = row.get("model_id").map_err(|e| e.to_string())?;
        if !model.is_empty() {
            let item = models
                .entry(model.clone())
                .or_insert_with(|| CodexLocalAccessModelStats {
                    model_id: model,
                    ..Default::default()
                });
            item.updated_at = updated;
            add_rollup_usage(&mut item.usage, &usage);
        }
        let key: String = row.get("api_key_id").map_err(|e| e.to_string())?;
        if !key.is_empty() {
            let item = keys
                .entry(key.clone())
                .or_insert_with(|| CodexLocalAccessApiKeyStats {
                    api_key_id: key,
                    ..Default::default()
                });
            let label: String = row.get("api_key_label").map_err(|e| e.to_string())?;
            if !label.is_empty() {
                item.label = label;
            }
            item.updated_at = updated;
            add_rollup_usage(&mut item.usage, &usage);
        }
        let bucket = usage_trend_bucket_start(timestamp, window.trend_hourly);
        let point = trend
            .entry(bucket)
            .or_insert_with(|| CodexLocalAccessUsageTrendPoint {
                bucket_start: bucket,
                ..Default::default()
            });
        add_rollup_usage(&mut point.usage, &usage);
    }
    window.accounts = accounts.into_values().collect();
    sort_usage_accounts(&mut window.accounts);
    window.models = models.into_values().collect();
    sort_usage_models(&mut window.models);
    window.api_keys = keys.into_values().collect();
    sort_usage_api_keys(&mut window.api_keys);
    window.trend = trend.into_values().collect();
    window.trend.sort_by_key(|v| v.bucket_start);
    Ok(window)
}

#[cfg(test)]
mod request_rollup_tests {
    use super::*;
    #[test]
    fn rollup_matches_event_statistics_and_tracks_reprice_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_local_access_logs_db_once(&dir.path().join("logs.sqlite"), true).unwrap();
        let mut expected = empty_stats_window(0, 0);
        for (i, kind) in [
            CodexLocalAccessRequestKind::Text,
            CodexLocalAccessRequestKind::ImageGeneration,
            CodexLocalAccessRequestKind::ImageEdit,
            CodexLocalAccessRequestKind::Other,
        ]
        .into_iter()
        .enumerate()
        {
            let event = CodexLocalAccessUsageEvent {
                timestamp: 60_000 + i as i64,
                request_id: format!("r{i}"),
                account_id: "account".into(),
                email: "user@example.com".into(),
                api_key_id: "key".into(),
                api_key_label: "Key".into(),
                model_id: "model".into(),
                request_kind: kind,
                success: i % 2 == 0,
                error_category: if i == 1 {
                    "stream_incomplete".into()
                } else {
                    String::new()
                },
                latency_ms: 100,
                input_tokens: 100,
                output_tokens: 10,
                total_tokens: 110,
                cached_tokens: 20,
                estimated_cost_usd: 0.25,
                ..Default::default()
            };
            insert_local_access_usage_event(&conn, &event).unwrap();
            apply_usage_event_to_window(&mut expected, &event);
        }
        let actual = query_rollup_window(&conn, 0, 119_999).unwrap();
        assert_eq!(
            serde_json::to_value(&actual.totals).unwrap(),
            serde_json::to_value(&expected.totals).unwrap()
        );
        assert_eq!(actual.accounts.len(), 1);
        assert_eq!(actual.accounts[0].usage.request_count, 4);
        while !REQUEST_USAGE_ROLLUP.batch(&conn).unwrap() {}
        assert_eq!(
            query_rollup_window(&conn, 60_001, 60_002)
                .unwrap()
                .totals
                .request_count,
            2
        );
        conn.execute(
            "UPDATE request_logs SET estimated_cost_usd=0.5 WHERE request_id='r0'",
            [],
        )
        .unwrap();
        assert!(
            (query_rollup_window(&conn, 0, 119_999)
                .unwrap()
                .totals
                .estimated_cost_usd
                - 1.25)
                .abs()
                < 1e-9
        );
        conn.execute("DELETE FROM request_logs", []).unwrap();
        assert_eq!(
            query_rollup_window(&conn, 0, 119_999)
                .unwrap()
                .totals
                .request_count,
            0
        );
    }
}
