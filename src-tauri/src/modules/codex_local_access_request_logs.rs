// Codex Local Access：Request log database, usage statistics and repricing workers。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn request_kind_to_db_value(request_kind: CodexLocalAccessRequestKind) -> &'static str {
    match request_kind {
        CodexLocalAccessRequestKind::Text => "text",
        CodexLocalAccessRequestKind::ImageGeneration => "image_generation",
        CodexLocalAccessRequestKind::ImageEdit => "image_edit",
        CodexLocalAccessRequestKind::Other => "other",
    }
}

fn request_kind_from_db_value(value: &str) -> CodexLocalAccessRequestKind {
    match value.trim() {
        "text" => CodexLocalAccessRequestKind::Text,
        "image_generation" => CodexLocalAccessRequestKind::ImageGeneration,
        "image_edit" => CodexLocalAccessRequestKind::ImageEdit,
        _ => CodexLocalAccessRequestKind::Other,
    }
}

fn gateway_mode_to_db_value(gateway_mode: CodexLocalAccessGatewayMode) -> &'static str {
    match gateway_mode {
        CodexLocalAccessGatewayMode::Legacy => "legacy",
        CodexLocalAccessGatewayMode::Sidecar => "sidecar",
    }
}

fn gateway_mode_from_db_value(value: &str) -> Option<CodexLocalAccessGatewayMode> {
    match value.trim() {
        "legacy" => Some(CodexLocalAccessGatewayMode::Legacy),
        "sidecar" => Some(CodexLocalAccessGatewayMode::Sidecar),
        _ => None,
    }
}

fn migrate_legacy_gateway_mode(collection: &mut CodexLocalAccessCollection) -> bool {
    if collection.gateway_mode == CodexLocalAccessGatewayMode::Legacy {
        collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
        return true;
    }
    false
}

fn bool_to_db_value(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn local_access_log_event_key(event: &CodexLocalAccessUsageEvent) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |value: &str| {
        for byte in value.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    };
    feed(&event.timestamp.to_string());
    feed(event.request_id.as_str());
    feed(event.account_id.as_str());
    feed(event.email.as_str());
    feed(event.api_key_id.as_str());
    feed(event.api_key_label.as_str());
    feed(event.model_id.as_str());
    feed(
        event
            .gateway_mode
            .map(gateway_mode_to_db_value)
            .unwrap_or_default(),
    );
    feed(request_kind_to_db_value(event.request_kind));
    feed(if event.success { "1" } else { "0" });
    feed(event.error_category.as_str());
    feed(&event.latency_ms.to_string());
    feed(&event.input_tokens.to_string());
    feed(&event.output_tokens.to_string());
    feed(&event.total_tokens.to_string());
    feed(&event.cached_tokens.to_string());
    feed(&event.reasoning_tokens.to_string());
    format!("{hash:016x}")
}

fn local_access_logs_db_sidecar_paths(path: &Path) -> Vec<PathBuf> {
    let raw = path.to_string_lossy();
    vec![
        PathBuf::from(format!("{}-wal", raw)),
        PathBuf::from(format!("{}-shm", raw)),
    ]
}

fn is_recoverable_logs_db_error(error: &SqliteError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("file is not a database")
        || message.contains("not a database")
        || message.contains("database disk image is malformed")
        || message.contains("database disk image is corrupt")
}

fn quarantine_local_access_logs_db(
    path: &Path,
    error: &SqliteError,
) -> Result<Option<PathBuf>, String> {
    let backup_path = crate::modules::atomic_write::quarantine_file(path, "invalid-sqlite")?;
    for sidecar_path in local_access_logs_db_sidecar_paths(path) {
        if let Err(sidecar_error) =
            crate::modules::atomic_write::quarantine_file(&sidecar_path, "invalid-sqlite")
        {
            logger::log_codex_api_warn(&format!(
                "API 服务日志数据库 sidecar 隔离失败，已忽略: path={}, error={}",
                sidecar_path.display(),
                sidecar_error
            ));
        }
    }
    logger::log_codex_api_warn(&format!(
        "API 服务日志数据库异常，已隔离并准备重建: path={}, backup={}, error={}",
        path.display(),
        backup_path
            .as_ref()
            .map(|item| item.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        error
    ));
    Ok(backup_path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestLogsSchemaState {
    MissingTable,
    MissingServiceTier,
    HasServiceTier,
}

fn create_request_logs_table(
    conn: &Connection,
    include_service_tier_column: bool,
) -> Result<(), SqliteError> {
    let service_tier_column = if include_service_tier_column {
        "\n            service_tier TEXT NOT NULL DEFAULT '',"
    } else {
        ""
    };
    let create_table_sql = format!(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_key TEXT NOT NULL UNIQUE,
            timestamp INTEGER NOT NULL,
            request_id TEXT NOT NULL DEFAULT '',
            account_id TEXT NOT NULL DEFAULT '',
            official_account_id TEXT NOT NULL DEFAULT '',
            email TEXT NOT NULL DEFAULT '',
            api_key_id TEXT NOT NULL DEFAULT '',
            api_key_label TEXT NOT NULL DEFAULT '',
            client_instance_id TEXT NOT NULL DEFAULT '',
            model_id TEXT NOT NULL DEFAULT '',
            gateway_mode TEXT NOT NULL DEFAULT '',
            request_kind TEXT NOT NULL DEFAULT 'other',{service_tier_column}
            success INTEGER NOT NULL DEFAULT 0,
            http_status INTEGER,
            error_category TEXT NOT NULL DEFAULT '',
            error_message TEXT NOT NULL DEFAULT '',
            latency_ms INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cached_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            token_breakdown_json TEXT NOT NULL DEFAULT '',
            estimated_cost_usd REAL NOT NULL DEFAULT 0,
            model_pricing_version INTEGER NOT NULL DEFAULT 1,
            input_usd_per_million REAL NOT NULL DEFAULT 0,
            output_usd_per_million REAL NOT NULL DEFAULT 0,
            cached_input_usd_per_million REAL
        );
        "#
    );
    conn.execute_batch(create_table_sql.as_str())?;
    Ok(())
}

fn open_local_access_logs_db_once(
    path: &Path,
    include_service_tier_column: bool,
) -> Result<Connection, SqliteError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(LOCAL_ACCESS_LOGS_DB_BUSY_TIMEOUT)?;
    create_request_logs_table(&conn, include_service_tier_column)?;
    ensure_request_logs_column(&conn, "event_key", "event_key TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "request_id", "request_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "account_id", "account_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(
        &conn,
        "official_account_id",
        "official_account_id TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "email", "email TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "api_key_id", "api_key_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(
        &conn,
        "api_key_label",
        "api_key_label TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "client_instance_id",
        "client_instance_id TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "model_id", "model_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(
        &conn,
        "gateway_mode",
        "gateway_mode TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "request_kind",
        "request_kind TEXT NOT NULL DEFAULT 'other'",
    )?;
    if include_service_tier_column {
        ensure_request_logs_column(
            &conn,
            "service_tier",
            "service_tier TEXT NOT NULL DEFAULT ''",
        )?;
    }
    ensure_request_logs_column(
        &conn,
        "reasoning_effort",
        "reasoning_effort TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "success", "success INTEGER NOT NULL DEFAULT 0")?;
    ensure_request_logs_column(&conn, "http_status", "http_status INTEGER")?;
    ensure_request_logs_column(
        &conn,
        "error_category",
        "error_category TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "error_message",
        "error_message TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "latency_ms", "latency_ms INTEGER NOT NULL DEFAULT 0")?;
    ensure_request_logs_column(
        &conn,
        "input_tokens",
        "input_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "output_tokens",
        "output_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "total_tokens",
        "total_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "cached_tokens",
        "cached_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "reasoning_tokens",
        "reasoning_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "token_breakdown_json",
        "token_breakdown_json TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "estimated_cost_usd",
        "estimated_cost_usd REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "model_pricing_version",
        "model_pricing_version INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_request_logs_column(
        &conn,
        "input_usd_per_million",
        "input_usd_per_million REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "output_usd_per_million",
        "output_usd_per_million REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "cached_input_usd_per_million",
        "cached_input_usd_per_million REAL",
    )?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_timestamp
            ON request_logs(timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_model
            ON request_logs(model_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_model_id
            ON request_logs(model_id, id);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_account
            ON request_logs(account_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_official_account
            ON request_logs(official_account_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_email
            ON request_logs(email, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_api_key
            ON request_logs(api_key_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_client_instance
            ON request_logs(client_instance_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_gateway_mode
            ON request_logs(gateway_mode, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_kind
            ON request_logs(request_kind, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_success
            ON request_logs(success, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_error
            ON request_logs(error_category, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_request_id
            ON request_logs(request_id, timestamp DESC);
        "#,
    )?;
    conn.execute(
        r#"
        UPDATE request_logs
        SET error_category = 'upstream_response_failed'
        WHERE success = 0
          AND error_category != 'upstream_response_failed'
          AND (
            lower(error_message) LIKE '%upstream_response_failed%'
            OR lower(error_message) LIKE '%codex upstream response.failed%'
            OR lower(error_message) LIKE '%last_event=response.failed%'
          )
        "#,
        [],
    )?;
    conn.execute(
        r#"
        UPDATE request_logs
        SET error_category = 'stream_incomplete'
        WHERE success = 0
          AND error_category != 'stream_incomplete'
          AND error_category != 'upstream_response_failed'
          AND (
            lower(error_message) LIKE '%stream disconnected before completion%'
            OR lower(error_message) LIKE '%error decoding response body%'
            OR lower(error_message) LIKE '%closed before response.completed%'
            OR lower(error_message) LIKE '%closed before response.done%'
            OR lower(error_message) LIKE '%stream ended before completion%'
            OR lower(error_message) LIKE '%incomplete_eof%'
          )
        "#,
        [],
    )?;
    Ok(conn)
}

fn ensure_request_logs_column(
    conn: &Connection,
    column_name: &str,
    column_definition: &str,
) -> Result<(), SqliteError> {
    if !request_logs_has_column(conn, column_name)? {
        conn.execute(
            format!("ALTER TABLE request_logs ADD COLUMN {column_definition}").as_str(),
            [],
        )?;
    }
    Ok(())
}

fn request_logs_has_column(conn: &Connection, column_name: &str) -> Result<bool, SqliteError> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('request_logs') WHERE name = ?1",
        params![column_name],
        |row| row.get(0),
    )?;
    Ok(exists != 0)
}

fn request_logs_table_exists(conn: &Connection) -> Result<bool, SqliteError> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'request_logs'",
        [],
        |row| row.get(0),
    )?;
    Ok(exists != 0)
}

fn request_logs_has_service_tier_column(conn: &Connection) -> Result<bool, SqliteError> {
    request_logs_has_column(conn, "service_tier")
}

fn inspect_request_logs_schema_state() -> Result<RequestLogsSchemaState, String> {
    let path = local_access_logs_db_path()?;
    if !path.exists() {
        return Ok(RequestLogsSchemaState::MissingTable);
    }
    let conn = Connection::open(&path)
        .map_err(|e| format!("打开 API 服务日志数据库以检查 schema 失败: {}", e))?;
    if !request_logs_table_exists(&conn)
        .map_err(|e| format!("检查 API 服务日志 request_logs 表失败: {}", e))?
    {
        return Ok(RequestLogsSchemaState::MissingTable);
    }
    if request_logs_has_service_tier_column(&conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?
    {
        return Ok(RequestLogsSchemaState::HasServiceTier);
    }
    Ok(RequestLogsSchemaState::MissingServiceTier)
}

fn open_local_access_logs_db_with_schema_unlocked(
    include_service_tier_column: bool,
) -> Result<Connection, String> {
    let path = local_access_logs_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 API 服务日志目录失败: {}", e))?;
    }
    match open_local_access_logs_db_once(&path, include_service_tier_column) {
        Ok(conn) => Ok(conn),
        Err(error) if is_recoverable_logs_db_error(&error) => {
            quarantine_local_access_logs_db(&path, &error)?;
            open_local_access_logs_db_once(&path, include_service_tier_column)
                .map_err(|e| format!("重建 API 服务日志数据库失败: {}", e))
        }
        Err(error) => Err(format!("打开 API 服务日志数据库失败: {}", error)),
    }
}

fn open_local_access_logs_db_with_schema(
    include_service_tier_column: bool,
) -> Result<Connection, String> {
    let _write_guard = lock_local_access_logs_db_write()?;
    open_local_access_logs_db_with_schema_unlocked(include_service_tier_column)
}

fn open_local_access_logs_db() -> Result<Connection, String> {
    open_local_access_logs_db_with_schema(true)
}

fn open_local_access_logs_db_with_schema_for_write(
    include_service_tier_column: bool,
) -> Result<(std::sync::MutexGuard<'static, ()>, Connection), String> {
    let write_guard = lock_local_access_logs_db_write()?;
    let conn = open_local_access_logs_db_with_schema_unlocked(include_service_tier_column)?;
    Ok((write_guard, conn))
}

fn open_local_access_logs_db_for_write(
) -> Result<(std::sync::MutexGuard<'static, ()>, Connection), String> {
    open_local_access_logs_db_with_schema_for_write(true)
}

fn serialize_token_breakdown_for_db(breakdown: Option<&CodexTokenBreakdown>) -> String {
    breakdown
        .and_then(|value| serde_json::to_string(value).ok())
        .unwrap_or_default()
}

fn deserialize_token_breakdown_from_db(raw: &str) -> Option<CodexTokenBreakdown> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str(raw).ok()
}

fn official_stats_account_id(local_account_id: &str) -> Option<String> {
    let local_account_id = local_account_id.trim();
    if local_account_id.is_empty() {
        return None;
    }
    codex_account::load_account(local_account_id).and_then(|account| {
        account
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn insert_local_access_usage_event(
    conn: &Connection,
    event: &CodexLocalAccessUsageEvent,
) -> Result<(), String> {
    let has_service_tier_column = request_logs_has_service_tier_column(conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?;
    let has_reasoning_effort_column = request_logs_has_column(conn, "reasoning_effort")
        .map_err(|e| format!("检查 API 服务日志 reasoning_effort 列失败: {}", e))?;
    let service_tier = event
        .service_tier
        .as_deref()
        .and_then(normalize_proxy_service_tier)
        .unwrap_or_default();
    let reasoning_effort = event
        .reasoning_effort
        .as_deref()
        .and_then(normalize_recorded_reasoning_effort)
        .unwrap_or_default();
    let token_breakdown_json = serialize_token_breakdown_for_db(event.token_breakdown.as_ref());
    if has_service_tier_column && has_reasoning_effort_column {
        conn.execute(
            r#"
            INSERT OR IGNORE INTO request_logs (
                event_key,
                timestamp,
                request_id,
                account_id,
                email,
                api_key_id,
                api_key_label,
                client_instance_id,
                model_id,
                gateway_mode,
                request_kind,
                service_tier,
                reasoning_effort,
                success,
                http_status,
                error_category,
                error_message,
                latency_ms,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_tokens,
                reasoning_tokens,
                token_breakdown_json,
                estimated_cost_usd,
                model_pricing_version,
                input_usd_per_million,
                output_usd_per_million,
                cached_input_usd_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29)
            "#,
            params![
                local_access_log_event_key(event),
                event.timestamp,
                event.request_id.trim(),
                event.account_id.trim(),
                event.email.trim(),
                event.api_key_id.trim(),
                event.api_key_label.trim(),
                event.client_instance_id.trim(),
                event.model_id.trim(),
                event
                    .gateway_mode
                    .map(gateway_mode_to_db_value)
                    .unwrap_or_default(),
                request_kind_to_db_value(event.request_kind),
                service_tier,
                reasoning_effort,
                bool_to_db_value(event.success),
                event.http_status.map(|value| value as i64),
                event.error_category.trim(),
                event.error_message.trim(),
                event.latency_ms as i64,
                event.input_tokens as i64,
                event.output_tokens as i64,
                event.total_tokens as i64,
                event.cached_tokens as i64,
                event.reasoning_tokens as i64,
                token_breakdown_json,
                event.estimated_cost_usd,
                event.model_pricing_version as i64,
                event.input_usd_per_million,
                event.output_usd_per_million,
                event.cached_input_usd_per_million,
            ],
        )
        .map_err(|e| format!("写入 API 服务请求日志失败: {}", e))?;
    } else if has_service_tier_column {
        conn.execute(
            r#"
            INSERT OR IGNORE INTO request_logs (
                event_key,
                timestamp,
                request_id,
                account_id,
                email,
                api_key_id,
                api_key_label,
                client_instance_id,
                model_id,
                gateway_mode,
                request_kind,
                service_tier,
                success,
                http_status,
                error_category,
                error_message,
                latency_ms,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_tokens,
                reasoning_tokens,
                token_breakdown_json,
                estimated_cost_usd,
                model_pricing_version,
                input_usd_per_million,
                output_usd_per_million,
                cached_input_usd_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28)
            "#,
            params![
                local_access_log_event_key(event),
                event.timestamp,
                event.request_id.trim(),
                event.account_id.trim(),
                event.email.trim(),
                event.api_key_id.trim(),
                event.api_key_label.trim(),
                event.client_instance_id.trim(),
                event.model_id.trim(),
                event
                    .gateway_mode
                    .map(gateway_mode_to_db_value)
                    .unwrap_or_default(),
                request_kind_to_db_value(event.request_kind),
                service_tier,
                bool_to_db_value(event.success),
                event.http_status.map(|value| value as i64),
                event.error_category.trim(),
                event.error_message.trim(),
                event.latency_ms as i64,
                event.input_tokens as i64,
                event.output_tokens as i64,
                event.total_tokens as i64,
                event.cached_tokens as i64,
                event.reasoning_tokens as i64,
                token_breakdown_json,
                event.estimated_cost_usd,
                event.model_pricing_version as i64,
                event.input_usd_per_million,
                event.output_usd_per_million,
                event.cached_input_usd_per_million,
            ],
        )
        .map_err(|e| format!("写入 API 服务请求日志失败: {}", e))?;
    } else {
        conn.execute(
            r#"
            INSERT OR IGNORE INTO request_logs (
                event_key,
                timestamp,
                request_id,
                account_id,
                email,
                api_key_id,
                api_key_label,
                client_instance_id,
                model_id,
                gateway_mode,
                request_kind,
                success,
                http_status,
                error_category,
                error_message,
                latency_ms,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_tokens,
                reasoning_tokens,
                token_breakdown_json,
                estimated_cost_usd,
                model_pricing_version,
                input_usd_per_million,
                output_usd_per_million,
                cached_input_usd_per_million
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)
            "#,
            params![
                local_access_log_event_key(event),
                event.timestamp,
                event.request_id.trim(),
                event.account_id.trim(),
                event.email.trim(),
                event.api_key_id.trim(),
                event.api_key_label.trim(),
                event.client_instance_id.trim(),
                event.model_id.trim(),
                event
                    .gateway_mode
                    .map(gateway_mode_to_db_value)
                    .unwrap_or_default(),
                request_kind_to_db_value(event.request_kind),
                bool_to_db_value(event.success),
                event.http_status.map(|value| value as i64),
                event.error_category.trim(),
                event.error_message.trim(),
                event.latency_ms as i64,
                event.input_tokens as i64,
                event.output_tokens as i64,
                event.total_tokens as i64,
                event.cached_tokens as i64,
                event.reasoning_tokens as i64,
                token_breakdown_json,
                event.estimated_cost_usd,
                event.model_pricing_version as i64,
                event.input_usd_per_million,
                event.output_usd_per_million,
                event.cached_input_usd_per_million,
            ],
        )
        .map_err(|e| format!("写入 API 服务请求日志失败: {}", e))?;
    }
    if let Some(official_account_id) = official_stats_account_id(&event.account_id) {
        conn.execute(
            "UPDATE request_logs SET official_account_id = ?1 WHERE event_key = ?2",
            params![official_account_id, local_access_log_event_key(event)],
        )
        .map_err(|e| format!("写入 API 服务请求日志官方账号 ID 失败: {}", e))?;
    }
    Ok(())
}

fn persist_local_access_usage_event(event: &CodexLocalAccessUsageEvent) -> Result<(), String> {
    let (_write_guard, conn) = open_local_access_logs_db_for_write()?;
    insert_local_access_usage_event(&conn, event)
}

fn migrate_local_access_json_events(
    events: &[CodexLocalAccessUsageEvent],
    include_service_tier_column: bool,
) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let (_write_guard, mut conn) =
        open_local_access_logs_db_with_schema_for_write(include_service_tier_column)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("开始迁移 API 服务请求日志失败: {}", e))?;
    for event in events {
        insert_local_access_usage_event(&tx, event)?;
    }
    tx.commit()
        .map_err(|e| format!("提交 API 服务请求日志迁移失败: {}", e))?;
    Ok(())
}

fn count_request_logs_for_model_ids(
    conn: &Connection,
    model_ids: Option<&[String]>,
    target_pricing_version: Option<u64>,
) -> Result<u64, String> {
    if matches!(model_ids, Some(items) if items.is_empty()) {
        return Ok(0);
    }
    let model_filter = model_ids.filter(|items| !items.is_empty());
    let mut params = Vec::<SqlValue>::new();
    let mut count_sql = if let Some(model_ids) = model_filter {
        let placeholders = std::iter::repeat("?")
            .take(model_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        params.extend(
            model_ids
                .iter()
                .map(|item| SqlValue::Text(item.to_string())),
        );
        format!("SELECT COUNT(*) FROM request_logs WHERE model_id IN ({placeholders})")
    } else {
        "SELECT COUNT(*) FROM request_logs".to_string()
    };
    if let Some(target_pricing_version) = target_pricing_version {
        count_sql.push_str(if model_filter.is_some() {
            " AND model_pricing_version != ?"
        } else {
            " WHERE model_pricing_version != ?"
        });
        params.push(SqlValue::Integer(
            target_pricing_version.min(i64::MAX as u64) as i64,
        ));
    }
    let total: i64 = conn
        .query_row(count_sql.as_str(), params_from_iter(params), |row| {
            row.get(0)
        })
        .map_err(|e| format!("统计 API 服务历史估算价值重算行数失败: {}", e))?;
    Ok(total.max(0) as u64)
}

fn request_log_reprice_model_cursors(
    conn: &Connection,
    model_ids: Option<&[String]>,
) -> Result<HashMap<String, i64>, String> {
    if let Some(model_ids) = model_ids {
        return Ok(model_ids
            .iter()
            .map(|model_id| model_id.trim())
            .filter(|model_id| !model_id.is_empty())
            .map(|model_id| (model_id.to_string(), 0_i64))
            .collect());
    }

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT model_id FROM request_logs WHERE model_id != '' ORDER BY model_id",
        )
        .map_err(|e| format!("准备 API 服务历史估算价值模型游标失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("读取 API 服务历史估算价值模型游标失败: {}", e))?;
    let model_ids = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析 API 服务历史估算价值模型游标失败: {}", e))?;
    Ok(model_ids
        .into_iter()
        .map(|model_id| (model_id, 0_i64))
        .collect())
}

fn read_request_log_reprice_rows_for_model(
    conn: &Connection,
    model_id: &str,
    after_id: i64,
    limit: i64,
    service_tier_select: &str,
    target_pricing_version: Option<u64>,
) -> Result<Vec<RequestLogRepriceRow>, String> {
    let select_sql = format!(
        r#"
        SELECT
            id,
            event_key,
            timestamp,
            account_id,
            api_key_id,
            model_id,
            input_tokens,
            output_tokens,
            total_tokens,
            cached_tokens,
            reasoning_tokens,
            token_breakdown_json,
            estimated_cost_usd,
            model_pricing_version,
            input_usd_per_million,
            output_usd_per_million,
            cached_input_usd_per_million,
            {service_tier_select}
        FROM request_logs
        WHERE model_id = ?1
          AND id > ?2
          AND (?4 IS NULL OR model_pricing_version != ?4)
        ORDER BY id ASC
        LIMIT ?3
        "#
    );
    let mut stmt = conn
        .prepare(select_sql.as_str())
        .map_err(|e| format!("准备 API 服务历史估算价值按模型读取失败: {}", e))?;
    let rows = stmt
        .query_map(
            params![
                model_id,
                after_id,
                limit.max(1),
                target_pricing_version.map(|version| version.min(i64::MAX as u64) as i64),
            ],
            |row| {
                let read_u64 = |name: &str| -> rusqlite::Result<u64> {
                    let value: i64 = row.get(name)?;
                    Ok(value.max(0) as u64)
                };
                let token_breakdown_json: String = row.get("token_breakdown_json")?;
                Ok(RequestLogRepriceRow {
                    id: row.get("id")?,
                    event_key: row.get("event_key")?,
                    timestamp: row.get("timestamp")?,
                    account_id: row.get("account_id")?,
                    api_key_id: row.get("api_key_id")?,
                    model_id: row.get("model_id")?,
                    usage: UsageCapture {
                        input_tokens: read_u64("input_tokens")?,
                        output_tokens: read_u64("output_tokens")?,
                        total_tokens: read_u64("total_tokens")?,
                        cached_tokens: read_u64("cached_tokens")?,
                        reasoning_tokens: read_u64("reasoning_tokens")?,
                        token_breakdown: deserialize_token_breakdown_from_db(&token_breakdown_json),
                    },
                    previous_cost_usd: row.get("estimated_cost_usd")?,
                    previous_model_pricing_version: read_u64("model_pricing_version")?,
                    previous_input_usd_per_million: row.get("input_usd_per_million")?,
                    previous_output_usd_per_million: row.get("output_usd_per_million")?,
                    previous_cached_input_usd_per_million: row
                        .get("cached_input_usd_per_million")?,
                    service_tier: row.get("service_tier")?,
                })
            },
        )
        .map_err(|e| format!("读取 API 服务历史估算价值按模型数据失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析 API 服务历史估算价值按模型数据失败: {}", e))
}

fn read_request_log_reprice_batch(
    conn: &Connection,
    model_cursors: &mut HashMap<String, i64>,
    limit: i64,
    target_pricing_version: Option<u64>,
) -> Result<Vec<RequestLogRepriceRow>, String> {
    if model_cursors.is_empty() {
        return Ok(Vec::new());
    }
    let has_service_tier_column = request_logs_has_service_tier_column(conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?;
    let service_tier_select = if has_service_tier_column {
        "service_tier"
    } else {
        "'' AS service_tier"
    };
    let mut model_ids = model_cursors.keys().cloned().collect::<Vec<_>>();
    model_ids.sort_unstable();
    let mut rows = Vec::new();
    let limit = limit.max(1) as usize;

    for model_id in model_ids {
        let remaining = limit.saturating_sub(rows.len());
        if remaining == 0 {
            break;
        }
        let after_id = model_cursors.get(&model_id).copied().unwrap_or_default();
        let model_rows = read_request_log_reprice_rows_for_model(
            conn,
            model_id.as_str(),
            after_id,
            remaining as i64,
            service_tier_select,
            target_pricing_version,
        )?;
        if let Some(last) = model_rows.last() {
            if model_rows.len() < remaining {
                model_cursors.remove(&model_id);
            } else {
                model_cursors.insert(model_id.clone(), last.id);
            }
        } else {
            model_cursors.remove(&model_id);
        }
        rows.extend(model_rows);
    }
    Ok(rows)
}

fn compute_request_log_reprice_row(
    collection: Option<&CodexLocalAccessCollection>,
    row: &RequestLogRepriceRow,
) -> Option<RequestLogRepriceUpdate> {
    let pricing = resolve_effective_model_pricing(
        collection,
        Some(row.model_id.as_str()),
        Some(&row.usage),
        Some(row.service_tier.as_str()),
    );
    let pricing = pricing.as_ref()?;
    let estimated_cost_usd = calculate_usage_cost_usd(Some(&row.usage), Some(pricing));
    let model_pricing_version = collection
        .map(|collection| collection.model_pricing_version)
        .unwrap_or(DEFAULT_MODEL_PRICING_VERSION)
        .max(DEFAULT_MODEL_PRICING_VERSION);
    let pricing_snapshot_unchanged = row.previous_model_pricing_version == model_pricing_version
        && row.previous_input_usd_per_million == pricing.input_usd_per_million
        && row.previous_output_usd_per_million == pricing.output_usd_per_million
        && row.previous_cached_input_usd_per_million == pricing.cached_input_usd_per_million;
    if row.previous_cost_usd.is_finite()
        && row.previous_cost_usd == estimated_cost_usd
        && pricing_snapshot_unchanged
    {
        return None;
    }
    let previous_cost_for_delta = if row.previous_cost_usd.is_finite() {
        row.previous_cost_usd
    } else {
        0.0
    };
    let estimated_cost_delta_usd = estimated_cost_usd - previous_cost_for_delta;
    let change = if estimated_cost_delta_usd.is_finite() && estimated_cost_delta_usd != 0.0 {
        Some(RequestLogRepriceChange {
            event_key: row.event_key.clone(),
            timestamp: row.timestamp,
            account_id: row.account_id.clone(),
            api_key_id: row.api_key_id.clone(),
            model_id: row.model_id.clone(),
            estimated_cost_delta_usd,
        })
    } else {
        None
    };
    Some(RequestLogRepriceUpdate {
        id: row.id,
        estimated_cost_usd,
        model_pricing_version,
        input_usd_per_million: pricing.input_usd_per_million,
        output_usd_per_million: pricing.output_usd_per_million,
        cached_input_usd_per_million: pricing.cached_input_usd_per_million,
        change,
    })
}

fn compute_request_log_reprice_updates(
    rows: Vec<RequestLogRepriceRow>,
    collection: Option<&CodexLocalAccessCollection>,
) -> Vec<RequestLogRepriceUpdate> {
    if rows.len() < MODEL_PRICING_REPRICE_PARALLEL_MIN_ROWS {
        return rows
            .iter()
            .filter_map(|row| compute_request_log_reprice_row(collection, row))
            .collect();
    }

    let thread_count = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .min(rows.len());
    if thread_count <= 1 {
        return rows
            .iter()
            .filter_map(|row| compute_request_log_reprice_row(collection, row))
            .collect();
    }

    let chunk_size = (rows.len() + thread_count - 1) / thread_count;
    let chunks = rows.chunks(chunk_size).collect::<Vec<_>>();
    std::thread::scope(|scope| {
        let handles = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .filter_map(|row| compute_request_log_reprice_row(collection, row))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .expect("model pricing reprice worker panicked")
            })
            .collect()
    })
}

fn write_request_log_reprice_updates(
    conn: &mut Connection,
    updates: &[RequestLogRepriceUpdate],
) -> Result<Vec<RequestLogRepriceChange>, String> {
    if updates.is_empty() {
        return Ok(Vec::new());
    }
    let tx = conn
        .transaction()
        .map_err(|e| format!("开始 API 服务历史估算价值批量写回失败: {}", e))?;
    {
        let mut update_stmt = tx
            .prepare(
                r#"
                UPDATE request_logs
                SET
                    estimated_cost_usd = ?2,
                    model_pricing_version = ?3,
                    input_usd_per_million = ?4,
                    output_usd_per_million = ?5,
                    cached_input_usd_per_million = ?6
                WHERE id = ?1
                "#,
            )
            .map_err(|e| format!("准备 API 服务历史估算价值批量写回失败: {}", e))?;
        for update in updates {
            update_stmt
                .execute(params![
                    update.id,
                    update.estimated_cost_usd,
                    update.model_pricing_version as i64,
                    update.input_usd_per_million,
                    update.output_usd_per_million,
                    update.cached_input_usd_per_million,
                ])
                .map_err(|e| {
                    format!(
                        "写回 API 服务历史估算价值失败: id={}, error={}",
                        update.id, e
                    )
                })?;
        }
    }
    tx.commit()
        .map_err(|e| format!("提交 API 服务历史估算价值批量写回失败: {}", e))?;

    Ok(updates
        .iter()
        .filter_map(|update| update.change.clone())
        .collect())
}

fn emit_model_pricing_reprice_event(
    app: &AppHandle,
    job_id: u64,
    phase: &str,
    total: u64,
    processed: u64,
    updated: u64,
    model_ids: &[String],
    message: &str,
) {
    let _ = app.emit(
        CODEX_LOCAL_ACCESS_MODEL_PRICING_REPRICE_EVENT,
        json!({
            "jobId": job_id,
            "phase": phase,
            "total": total,
            "processed": processed,
            "updated": updated,
            "modelIds": model_ids,
            "message": message,
        }),
    );
}

async fn model_pricing_reprice_has_newer_pending_job(job_id: u64) -> bool {
    let worker = model_pricing_reprice_worker().lock().await;
    worker
        .pending
        .as_ref()
        .map(|pending| pending.job_id > job_id)
        .unwrap_or(false)
}

async fn apply_reprice_changes_to_runtime_stats(changes: &[RequestLogRepriceChange]) {
    if changes.is_empty() {
        return;
    }
    {
        let mut runtime = gateway_runtime().lock().await;
        apply_reprice_changes_to_stats(&mut runtime.stats, changes);
        runtime.stats_dirty = true;
        runtime.stats_revision = runtime.stats_revision.wrapping_add(1);
    }
    schedule_stats_flush_if_needed().await;
}

async fn queue_model_pricing_reprice(
    app: AppHandle,
    collection: CodexLocalAccessCollection,
    model_ids: Vec<String>,
) {
    let model_ids = normalize_reprice_model_ids(model_ids);
    if model_ids.is_empty() {
        return;
    }

    let should_spawn = {
        let mut worker = model_pricing_reprice_worker().lock().await;
        worker.next_job_id = worker.next_job_id.saturating_add(1);
        let job_id = worker.next_job_id;
        let mut pending_model_ids = model_ids;
        if worker.running {
            pending_model_ids.extend(worker.active_model_ids.iter().cloned());
            if let Some(pending) = worker.pending.as_ref() {
                pending_model_ids.extend(pending.model_ids.iter().cloned());
            }
        }
        worker.pending = Some(ModelPricingRepriceJob {
            job_id,
            collection,
            model_ids: normalize_reprice_model_ids(pending_model_ids),
        });
        if worker.running {
            false
        } else {
            worker.running = true;
            true
        }
    };

    if should_spawn {
        tokio::spawn(async move {
            run_model_pricing_reprice_worker(app).await;
        });
    }
}

async fn run_model_pricing_reprice_worker(app: AppHandle) {
    loop {
        let job = {
            let mut worker = model_pricing_reprice_worker().lock().await;
            let Some(job) = worker.pending.take() else {
                worker.running = false;
                worker.active_model_ids.clear();
                return;
            };
            worker.active_model_ids = job.model_ids.clone();
            job
        };

        run_model_pricing_reprice_job(app.clone(), job).await;

        let mut worker = model_pricing_reprice_worker().lock().await;
        if worker.pending.is_none() {
            worker.active_model_ids.clear();
        }
    }
}

async fn run_model_pricing_reprice_job(app: AppHandle, job: ModelPricingRepriceJob) {
    let mut conn = match open_local_access_logs_db() {
        Ok(conn) => conn,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "打开 API 服务日志数据库以后台重算历史估算价值失败: {}",
                error
            ));
            emit_model_pricing_reprice_event(
                &app,
                job.job_id,
                "failed",
                0,
                0,
                0,
                &job.model_ids,
                &error,
            );
            return;
        }
    };
    let target_pricing_version = job.collection.model_pricing_version;
    let total = match count_request_logs_for_model_ids(
        &conn,
        Some(&job.model_ids),
        Some(target_pricing_version),
    ) {
        Ok(total) => total,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "统计 API 服务后台历史估算价值重算行数失败: {}",
                error
            ));
            emit_model_pricing_reprice_event(
                &app,
                job.job_id,
                "failed",
                0,
                0,
                0,
                &job.model_ids,
                &error,
            );
            return;
        }
    };
    let mut model_cursors = match request_log_reprice_model_cursors(&conn, Some(&job.model_ids)) {
        Ok(cursors) => cursors,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "准备 API 服务后台历史估算价值模型游标失败: {}",
                error
            ));
            emit_model_pricing_reprice_event(
                &app,
                job.job_id,
                "failed",
                0,
                0,
                0,
                &job.model_ids,
                &error,
            );
            return;
        }
    };

    let mut processed = 0_u64;
    let mut updated = 0_u64;
    let mut all_changes = Vec::new();
    emit_model_pricing_reprice_event(
        &app,
        job.job_id,
        "started",
        total,
        processed,
        updated,
        &job.model_ids,
        "",
    );

    loop {
        if model_pricing_reprice_has_newer_pending_job(job.job_id).await {
            apply_reprice_changes_to_runtime_stats(&all_changes).await;
            emit_model_pricing_reprice_event(
                &app,
                job.job_id,
                "superseded",
                total,
                processed,
                updated,
                &job.model_ids,
                "",
            );
            return;
        }

        let rows = match read_request_log_reprice_batch(
            &conn,
            &mut model_cursors,
            MODEL_PRICING_REPRICE_BATCH_SIZE,
            Some(target_pricing_version),
        ) {
            Ok(rows) => rows,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "API 服务后台历史估算价值分批重算失败: {}",
                    error
                ));
                apply_reprice_changes_to_runtime_stats(&all_changes).await;
                emit_model_pricing_reprice_event(
                    &app,
                    job.job_id,
                    "failed",
                    total,
                    processed,
                    updated,
                    &job.model_ids,
                    &error,
                );
                return;
            }
        };
        let processed_rows = rows.len() as u64;
        let updates = compute_request_log_reprice_updates(rows, Some(&job.collection));
        let batch_changes_result = if updates.is_empty() {
            Ok(Vec::new())
        } else {
            match lock_local_access_logs_db_write() {
                Ok(_write_guard) => write_request_log_reprice_updates(&mut conn, &updates),
                Err(error) => Err(error),
            }
        };
        let mut batch_changes = match batch_changes_result {
            Ok(changes) => changes,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "API 服务后台历史估算价值分批写回失败: {}",
                    error
                ));
                apply_reprice_changes_to_runtime_stats(&all_changes).await;
                emit_model_pricing_reprice_event(
                    &app,
                    job.job_id,
                    "failed",
                    total,
                    processed,
                    updated,
                    &job.model_ids,
                    &error,
                );
                return;
            }
        };
        processed = processed.saturating_add(processed_rows).min(total);
        updated = updated.saturating_add(updates.len() as u64);
        all_changes.append(&mut batch_changes);

        if processed_rows == 0 {
            break;
        }

        emit_model_pricing_reprice_event(
            &app,
            job.job_id,
            "running",
            total,
            processed,
            updated,
            &job.model_ids,
            "",
        );

        if model_cursors.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }

    apply_reprice_changes_to_runtime_stats(&all_changes).await;
    emit_model_pricing_reprice_event(
        &app,
        job.job_id,
        "completed",
        total,
        processed,
        updated,
        &job.model_ids,
        "",
    );
}

fn reprice_request_logs_with_model_ids(
    conn: &mut Connection,
    collection: Option<&CodexLocalAccessCollection>,
    model_ids: Option<&[String]>,
) -> Result<Vec<RequestLogRepriceChange>, String> {
    if matches!(model_ids, Some(items) if items.is_empty()) {
        return Ok(Vec::new());
    }
    let has_service_tier_column = request_logs_has_service_tier_column(conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?;
    let mut model_cursors = request_log_reprice_model_cursors(conn, model_ids)?;
    let mut changes = Vec::new();
    while !model_cursors.is_empty() {
        let rows = read_request_log_reprice_batch(
            conn,
            &mut model_cursors,
            MODEL_PRICING_REPRICE_BATCH_SIZE,
            None,
        )?;
        if rows.is_empty() {
            break;
        }
        let updates = compute_request_log_reprice_updates(rows, collection);
        if !updates.is_empty() {
            let _write_guard = lock_local_access_logs_db_write()?;
            changes.extend(write_request_log_reprice_updates(conn, &updates)?);
        }
    }

    if !has_service_tier_column {
        let _write_guard = lock_local_access_logs_db_write()?;
        conn.execute(
            "ALTER TABLE request_logs ADD COLUMN service_tier TEXT NOT NULL DEFAULT ''",
            [],
        )
        .map_err(|e| format!("补齐 API 服务日志 service_tier 列失败: {}", e))?;
    }
    Ok(changes)
}

fn clear_local_access_usage_events_db() -> Result<(), String> {
    let (_write_guard, conn) = open_local_access_logs_db_for_write()?;
    conn.execute("DELETE FROM request_logs", [])
        .map_err(|e| format!("清空 API 服务请求日志失败: {}", e))?;
    Ok(())
}

fn usage_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodexLocalAccessUsageEvent> {
    let request_kind: String = row.get("request_kind")?;
    let service_tier: String = row.get("service_tier")?;
    let reasoning_effort: String = row.get::<_, String>("reasoning_effort").unwrap_or_default();
    let success: i64 = row.get("success")?;
    let http_status: Option<i64> = row.get("http_status")?;
    let gateway_mode: String = row.get("gateway_mode")?;
    let token_breakdown_json: String = row.get("token_breakdown_json")?;
    let read_u64 = |name: &str| -> rusqlite::Result<u64> {
        let value: i64 = row.get(name)?;
        Ok(value.max(0) as u64)
    };
    Ok(CodexLocalAccessUsageEvent {
        timestamp: row.get("timestamp")?,
        request_id: row.get("request_id")?,
        account_id: row.get("account_id")?,
        email: row.get("email")?,
        api_key_id: row.get("api_key_id")?,
        api_key_label: row.get("api_key_label")?,
        client_instance_id: row
            .get::<_, String>("client_instance_id")
            .unwrap_or_else(|_| String::new()),
        model_id: row.get("model_id")?,
        gateway_mode: gateway_mode_from_db_value(gateway_mode.as_str()),
        request_kind: request_kind_from_db_value(request_kind.as_str()),
        service_tier: normalize_proxy_service_tier(service_tier.as_str()).map(str::to_string),
        reasoning_effort: normalize_recorded_reasoning_effort(reasoning_effort.as_str())
            .map(str::to_string),
        success: success != 0,
        http_status: http_status.and_then(|value| u16::try_from(value).ok()),
        error_category: row.get("error_category")?,
        error_message: row.get("error_message")?,
        latency_ms: read_u64("latency_ms")?,
        input_tokens: read_u64("input_tokens")?,
        output_tokens: read_u64("output_tokens")?,
        total_tokens: read_u64("total_tokens")?,
        cached_tokens: read_u64("cached_tokens")?,
        reasoning_tokens: read_u64("reasoning_tokens")?,
        token_breakdown: deserialize_token_breakdown_from_db(&token_breakdown_json),
        estimated_cost_usd: row.get("estimated_cost_usd")?,
        model_pricing_version: read_u64("model_pricing_version")?,
        input_usd_per_million: row.get("input_usd_per_million")?,
        output_usd_per_million: row.get("output_usd_per_million")?,
        cached_input_usd_per_million: row.get("cached_input_usd_per_million")?,
    })
}

fn for_each_local_access_usage_event_since<F>(
    since: i64,
    on_event: F,
) -> Result<(), String>
where
    F: FnMut(CodexLocalAccessUsageEvent) -> Result<(), String>,
{
    let conn = open_local_access_logs_db()?;
    for_each_local_access_usage_event_since_from_conn(&conn, since, on_event)
}

fn for_each_local_access_usage_event_since_from_conn<F>(
    conn: &Connection,
    since: i64,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(CodexLocalAccessUsageEvent) -> Result<(), String>,
{
    let service_tier_select = if request_logs_has_service_tier_column(conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?
    {
        "service_tier"
    } else {
        "'' AS service_tier"
    };
    let reasoning_effort_select = if request_logs_has_column(conn, "reasoning_effort")
        .map_err(|e| format!("检查 API 服务日志 reasoning_effort 列失败: {}", e))?
    {
        "reasoning_effort"
    } else {
        "'' AS reasoning_effort"
    };
    let load_sql = format!(
        r#"
            SELECT
                timestamp,
                request_id,
                account_id,
                email,
                api_key_id,
                api_key_label,
                client_instance_id,
                model_id,
                gateway_mode,
                request_kind,
                {service_tier_select},
                {reasoning_effort_select},
                success,
                http_status,
                error_category,
                error_message,
                latency_ms,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_tokens,
                reasoning_tokens,
                token_breakdown_json,
                estimated_cost_usd,
                model_pricing_version,
                input_usd_per_million,
                output_usd_per_million,
                cached_input_usd_per_million
            FROM request_logs
            WHERE timestamp >= ?1
            ORDER BY timestamp ASC, id ASC
            "#
    );
    let mut stmt = conn
        .prepare(load_sql.as_str())
        .map_err(|e| format!("准备 API 服务日志读取失败: {}", e))?;
    let rows = stmt
        .query_map(params![since], usage_event_from_row)
        .map_err(|e| format!("读取 API 服务日志失败: {}", e))?;
    for row in rows {
        let event = row.map_err(|e| format!("解析 API 服务日志失败: {}", e))?;
        on_event(event)?;
    }
    Ok(())
}

fn local_calendar_window_starts(now: i64) -> (i64, i64, i64) {
    let starts = calendar_stats_window_starts(now);
    (starts.day, starts.week, starts.month)
}

fn stats_range_since(stats_range: Option<&str>) -> Option<i64> {
    let (day_since, week_since, month_since) = local_calendar_window_starts(now_ms());
    match stats_range.map(str::trim) {
        Some("daily") => Some(day_since),
        Some("weekly") => Some(week_since),
        Some("monthly") => Some(month_since),
        _ => None,
    }
}

fn normalize_log_filter(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn push_like_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<String>,
) {
    if let Some(value) = normalize_log_filter(value) {
        clauses.push(clause.to_string());
        params.push(SqlValue::Text(format!("%{}%", value)));
    }
}

fn empty_usage_event_page(page: u32, page_size: u32) -> CodexLocalAccessUsageEventPage {
    CodexLocalAccessUsageEventPage {
        events: Vec::new(),
        total: 0,
        page: page.max(1),
        page_size: page_size.clamp(1, 200),
        total_pages: 1,
    }
}

fn query_local_access_usage_events_blocking(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    instance_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    let page_size = page_size.clamp(1, 200);
    let page = page.max(1);
    let mut clauses = Vec::new();
    let mut params = Vec::<SqlValue>::new();

    if let Some(start_at) = start_at {
        clauses.push("timestamp >= ?".to_string());
        params.push(SqlValue::Integer(start_at));
    } else if let Some(since) = stats_range_since(stats_range.as_deref()) {
        clauses.push("timestamp >= ?".to_string());
        params.push(SqlValue::Integer(since));
    }
    if let Some(end_at) = end_at {
        clauses.push("timestamp <= ?".to_string());
        params.push(SqlValue::Integer(end_at));
    }
    push_like_filter(&mut clauses, &mut params, "model_id LIKE ?", model_query);
    push_like_filter(
        &mut clauses,
        &mut params,
        "(account_id LIKE ? OR email LIKE ?)",
        account_query.clone(),
    );
    if let Some(account_query) = normalize_log_filter(account_query) {
        params.push(SqlValue::Text(format!("%{}%", account_query)));
    }
    push_like_filter(
        &mut clauses,
        &mut params,
        "(api_key_id LIKE ? OR api_key_label LIKE ?)",
        api_key_query.clone(),
    );
    if let Some(api_key_query) = normalize_log_filter(api_key_query) {
        params.push(SqlValue::Text(format!("%{}%", api_key_query)));
    }
    push_like_filter(
        &mut clauses,
        &mut params,
        "client_instance_id LIKE ?",
        instance_query,
    );
    if let Some(gateway_mode) = gateway_mode {
        clauses.push("gateway_mode = ?".to_string());
        params.push(SqlValue::Text(
            gateway_mode_to_db_value(gateway_mode).to_string(),
        ));
    }
    if let Some(request_kind) = request_kind {
        clauses.push("request_kind = ?".to_string());
        params.push(SqlValue::Text(
            request_kind_to_db_value(request_kind).to_string(),
        ));
    }
    if let Some(success) = success {
        clauses.push("success = ?".to_string());
        params.push(SqlValue::Integer(bool_to_db_value(success)));
    }
    push_like_filter(
        &mut clauses,
        &mut params,
        "error_category LIKE ?",
        error_category,
    );

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let conn = match open_local_access_logs_db() {
        Ok(conn) => conn,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务请求日志数据库不可用，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let total_sql = format!("SELECT COUNT(*) FROM request_logs{}", where_sql);
    let total: u64 = match conn.query_row(
        total_sql.as_str(),
        params_from_iter(params.clone()),
        |row| row.get::<_, i64>(0),
    ) {
        Ok(total) => total.max(0) as u64,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "统计 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let total_pages = ((total + page_size as u64 - 1) / page_size as u64)
        .max(1)
        .min(u32::MAX as u64) as u32;
    let page = page.min(total_pages);
    let offset = (page.saturating_sub(1) as u64 * page_size as u64).min(i64::MAX as u64) as i64;
    let mut query_params = params;
    query_params.push(SqlValue::Integer(page_size as i64));
    query_params.push(SqlValue::Integer(offset));
    let service_tier_select = match request_logs_has_service_tier_column(&conn) {
        Ok(true) => "service_tier",
        Ok(false) => "'' AS service_tier",
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "检查 API 服务日志 service_tier 列失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let reasoning_effort_select = match request_logs_has_column(&conn, "reasoning_effort") {
        Ok(true) => "reasoning_effort",
        Ok(false) => "'' AS reasoning_effort",
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "检查 API 服务日志 reasoning_effort 列失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let list_sql = format!(
        r#"
        SELECT
            timestamp,
            request_id,
            account_id,
            email,
            api_key_id,
            api_key_label,
            client_instance_id,
            model_id,
            gateway_mode,
            request_kind,
            {service_tier_select},
            {reasoning_effort_select},
            success,
            http_status,
            error_category,
            error_message,
            latency_ms,
            input_tokens,
            output_tokens,
            total_tokens,
            cached_tokens,
            reasoning_tokens,
            token_breakdown_json,
            estimated_cost_usd,
            model_pricing_version,
            input_usd_per_million,
            output_usd_per_million,
            cached_input_usd_per_million
        FROM request_logs{}
        ORDER BY timestamp DESC, id DESC
        LIMIT ? OFFSET ?
        "#,
        where_sql
    );
    let mut stmt = match conn.prepare(list_sql.as_str()) {
        Ok(stmt) => stmt,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "准备 API 服务请求日志查询失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let rows = match stmt.query_map(params_from_iter(query_params), usage_event_from_row) {
        Ok(rows) => rows,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "查询 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let events = match rows.collect::<Result<Vec<_>, _>>() {
        Ok(events) => events,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "解析 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };

    Ok(CodexLocalAccessUsageEventPage {
        events,
        total,
        page,
        page_size,
        total_pages,
    })
}

pub async fn query_local_access_usage_events(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    start_at: Option<i64>,
    end_at: Option<i64>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    instance_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    ensure_runtime_loaded_without_start().await?;
    tauri::async_runtime::spawn_blocking(move || {
        query_local_access_usage_events_blocking(
            page,
            page_size,
            stats_range,
            start_at,
            end_at,
            model_query,
            account_query,
            api_key_query,
            instance_query,
            gateway_mode,
            request_kind,
            success,
            error_category,
        )
    })
    .await
    .map_err(|e| format!("查询 API 服务请求日志任务失败: {}", e))?
}

fn query_local_access_stats_window_blocking(
    start_at: i64,
    end_at: i64,
) -> Result<CodexLocalAccessStatsWindow, String> {
    if start_at < 0 || end_at < start_at {
        return Err("统计时间范围无效：结束时间必须不早于开始时间".to_string());
    }
    let conn = open_local_access_logs_db()?;
    let service_tier_select = if request_logs_has_service_tier_column(&conn)
        .map_err(|e| format!("检查 API 服务日志 service_tier 列失败: {}", e))?
    {
        "service_tier"
    } else {
        "'' AS service_tier"
    };
    let reasoning_effort_select = if request_logs_has_column(&conn, "reasoning_effort")
        .map_err(|e| format!("检查 API 服务日志 reasoning_effort 列失败: {}", e))?
    {
        "reasoning_effort"
    } else {
        "'' AS reasoning_effort"
    };
    let sql = format!(
        r#"SELECT timestamp, request_id, account_id, email, api_key_id, api_key_label,
                  client_instance_id, model_id, gateway_mode, request_kind, {service_tier_select}, {reasoning_effort_select}, success,
                  http_status, error_category, error_message, latency_ms, input_tokens,
                  output_tokens, total_tokens, cached_tokens, reasoning_tokens, token_breakdown_json,
                  estimated_cost_usd, model_pricing_version, input_usd_per_million,
                  output_usd_per_million, cached_input_usd_per_million
           FROM request_logs
           WHERE timestamp >= ?1 AND timestamp <= ?2
           ORDER BY timestamp ASC, id ASC"#
    );
    let mut statement = conn
        .prepare(sql.as_str())
        .map_err(|e| format!("准备 API 服务统计查询失败: {}", e))?;
    let mut window = empty_stats_window(start_at, start_at);
    let hourly_trend = end_at.saturating_sub(start_at) <= 36 * 60 * 60 * 1000;
    let mut trend = HashMap::new();
    let rows = statement
        .query_map(params![start_at, end_at], usage_event_from_row)
        .map_err(|e| format!("查询 API 服务统计失败: {}", e))?;
    for row in rows {
        let event = row.map_err(|e| format!("解析 API 服务统计失败: {}", e))?;
        apply_usage_event_to_window(&mut window, &event);
        apply_usage_event_to_trend(&mut trend, &event, hourly_trend);
    }
    sort_usage_accounts(&mut window.accounts);
    sort_usage_models(&mut window.models);
    sort_usage_api_keys(&mut window.api_keys);
    let mut trend = trend.into_values().collect::<Vec<_>>();
    trend.sort_by_key(|point| point.bucket_start);
    window.trend = trend;
    window.trend_hourly = hourly_trend;
    Ok(window)
}

pub async fn query_local_access_stats_window(
    start_at: i64,
    end_at: i64,
) -> Result<CodexLocalAccessStatsWindow, String> {
    ensure_runtime_loaded_without_start().await?;
    tauri::async_runtime::spawn_blocking(move || {
        query_local_access_stats_window_blocking(start_at, end_at)
    })
    .await
    .map_err(|e| format!("统计 API 服务时间范围任务失败: {}", e))?
}

pub async fn query_local_access_account_window_stats(
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    ensure_runtime_loaded_without_start().await?;
    tauri::async_runtime::spawn_blocking(move || {
        query_local_access_account_window_stats_blocking(queries)
    })
    .await
    .map_err(|e| format!("统计 API 服务账号窗口用量任务失败: {}", e))?
}

/// request_logs.timestamp 存毫秒。官方额度 resetAt / 前端误传的 Unix 秒会小于 1e12。
fn normalize_request_log_time_bound(value: i64) -> i64 {
    if value > 0 && value < 1_000_000_000_000 {
        value.saturating_mul(1000)
    } else {
        value
    }
}

struct AccountWindowStatSpec {
    account_id: String,
    window_key: String,
    start_at: i64,
    end_at: i64,
}

/// API 服务请求日志的 `account_id` 是本地 Codex 账号 ID。
/// Team/Workspace 的官方 `account_id` 可能被多个成员共享，不能作为本地账号统计键。
fn account_window_stat_identity_matches(row_account_id: &str, requested_account_id: &str) -> bool {
    let row_account_id = row_account_id.trim();
    let requested_account_id = requested_account_id.trim();
    !row_account_id.is_empty() && row_account_id == requested_account_id
}

fn query_local_access_account_window_stats_blocking(
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    let conn = open_local_access_logs_db()?;
    query_local_access_account_window_stats_from_conn(&conn, queries)
}

fn query_local_access_account_window_stats_from_conn(
    conn: &Connection,
    queries: Vec<CodexLocalAccessAccountWindowQuery>,
) -> Result<Vec<CodexLocalAccessAccountWindowStats>, String> {
    if queries.is_empty() {
        return Ok(Vec::new());
    }

    let mut specs = Vec::new();
    let mut min_start = i64::MAX;
    let mut max_end = 0i64;
    let mut local_account_ids = HashSet::new();
    for query in &queries {
        let account_id = query.account_id.trim();
        let window_key = query.window_key.trim();
        let start_at = normalize_request_log_time_bound(query.start_at);
        let end_at = normalize_request_log_time_bound(query.end_at);
        if account_id.is_empty() || window_key.is_empty() || end_at < start_at {
            continue;
        }
        min_start = min_start.min(start_at);
        max_end = max_end.max(end_at);
        local_account_ids.insert(account_id.to_string());
        specs.push(AccountWindowStatSpec {
            account_id: account_id.to_string(),
            window_key: window_key.to_string(),
            start_at,
            end_at,
        });
    }
    if specs.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = local_account_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT account_id, timestamp,
                input_tokens, output_tokens, total_tokens,
                cached_tokens, estimated_cost_usd
         FROM request_logs
         WHERE timestamp >= ? AND timestamp <= ?
           AND account_id IN ({placeholders})"
    );
    let mut statement = conn
        .prepare(sql.as_str())
        .map_err(|e| format!("准备 API 服务账号窗口查询失败: {}", e))?;
    let mut params: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::Integer(min_start),
        rusqlite::types::Value::Integer(max_end),
    ];
    for account_id in &local_account_ids {
        params.push(rusqlite::types::Value::Text(account_id.clone()));
    }

    let mut totals = HashMap::<(String, String), CodexLocalAccessAccountWindowStats>::new();
    for spec in &specs {
        totals.insert(
            (spec.account_id.clone(), spec.window_key.clone()),
            CodexLocalAccessAccountWindowStats {
                account_id: spec.account_id.clone(),
                window_key: spec.window_key.clone(),
                ..CodexLocalAccessAccountWindowStats::default()
            },
        );
    }

    let rows = statement
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?.max(0) as u64,
                row.get::<_, i64>(3)?.max(0) as u64,
                row.get::<_, i64>(4)?.max(0) as u64,
                row.get::<_, i64>(5)?.max(0) as u64,
                row.get::<_, f64>(6).unwrap_or(0.0),
            ))
        })
        .map_err(|e| format!("查询 API 服务账号窗口用量失败: {}", e))?;

    for row in rows {
        let (row_account_id, timestamp, input, output, total, cached, cost) =
            row.map_err(|e| format!("解析 API 服务账号窗口用量失败: {}", e))?;
        for spec in &specs {
            if !account_window_stat_identity_matches(&row_account_id, &spec.account_id)
                || timestamp < spec.start_at
                || timestamp > spec.end_at
            {
                continue;
            }
            if let Some(entry) = totals.get_mut(&(spec.account_id.clone(), spec.window_key.clone()))
            {
                entry.request_count = entry.request_count.saturating_add(1);
                entry.input_tokens = entry.input_tokens.saturating_add(input);
                entry.output_tokens = entry.output_tokens.saturating_add(output);
                entry.total_tokens = entry.total_tokens.saturating_add(total);
                entry.cached_tokens = entry.cached_tokens.saturating_add(cached);
                if cost.is_finite() && cost > 0.0 {
                    entry.estimated_cost_usd += cost;
                }
            }
        }
    }

    Ok(specs
        .into_iter()
        .filter_map(|spec| totals.remove(&(spec.account_id, spec.window_key)))
        .collect())
}

fn apply_usage_event_to_stats(
    stats: &mut CodexLocalAccessStats,
    event: &CodexLocalAccessUsageEvent,
) {
    let usage = UsageCapture {
        input_tokens: event.input_tokens,
        output_tokens: event.output_tokens,
        total_tokens: event.total_tokens,
        cached_tokens: event.cached_tokens,
        reasoning_tokens: event.reasoning_tokens,
        token_breakdown: event.token_breakdown.clone(),
    };
    apply_usage_stats(
        &mut stats.totals,
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
    );
    upsert_account_usage_stats(
        &mut stats.accounts,
        Some(event.account_id.as_str()),
        Some(event.email.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_model_usage_stats(
        &mut stats.models,
        Some(event.model_id.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_api_key_usage_stats(
        &mut stats.api_keys,
        Some(event.api_key_id.as_str()),
        Some(event.api_key_label.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    if event.timestamp > 0 {
        stats.since = if stats.since <= 0 {
            event.timestamp
        } else {
            stats.since.min(event.timestamp)
        };
        stats.updated_at = stats.updated_at.max(event.timestamp);
    }
    push_recent_usage_event(&mut stats.events, event.clone());
}

fn rebuild_stats_from_request_logs() -> Result<CodexLocalAccessStats, String> {
    let now = now_ms();
    let mut stats = empty_stats_snapshot();
    stats.since = 0;
    stats.updated_at = 0;
    stats.totals = CodexLocalAccessUsageStats::default();
    stats.accounts.clear();
    stats.models.clear();
    stats.api_keys.clear();
    stats.events.clear();
    ensure_stats_windows_current(&mut stats, now);
    for_each_local_access_usage_event_since(0, |event| {
        apply_usage_event_to_stats(&mut stats, &event);
        apply_usage_event_to_current_windows(&mut stats, &event, now);
        Ok(())
    })?;
    sort_stats_rows(&mut stats);
    if stats.since <= 0 {
        stats.since = now;
    }
    stats.updated_at = stats.updated_at.max(now);
    Ok(stats)
}

fn reprice_request_logs_for_collection(
    conn: &mut Connection,
    collection: &CodexLocalAccessCollection,
) -> Result<usize, String> {
    reprice_request_logs_with_model_ids(conn, Some(collection), None).map(|changes| changes.len())
}

fn append_usage_event(
    events: &mut Vec<CodexLocalAccessUsageEvent>,
    now: i64,
    request_id: Option<&str>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    client_instance_id: Option<&str>,
    model_id: Option<&str>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: CodexLocalAccessRequestKind,
    service_tier: Option<&str>,
    reasoning_effort: Option<&str>,
    success: bool,
    http_status: Option<u16>,
    error_category: Option<&str>,
    error_message: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    pricing: Option<&CodexLocalAccessModelPricing>,
    model_pricing_version: u64,
    estimated_cost_usd: f64,
) -> CodexLocalAccessUsageEvent {
    let usage = usage.cloned().unwrap_or_default();
    let event = CodexLocalAccessUsageEvent {
        timestamp: now,
        request_id: request_id.unwrap_or_default().trim().to_string(),
        account_id: account_id.unwrap_or_default().trim().to_string(),
        email: account_email.unwrap_or_default().trim().to_string(),
        api_key_id: api_key_id.unwrap_or_default().trim().to_string(),
        api_key_label: api_key_label.unwrap_or_default().trim().to_string(),
        client_instance_id: client_instance_id.unwrap_or_default().trim().to_string(),
        model_id: model_id.unwrap_or_default().trim().to_string(),
        gateway_mode,
        request_kind,
        service_tier: service_tier
            .and_then(normalize_proxy_service_tier)
            .map(str::to_string),
        reasoning_effort: reasoning_effort
            .and_then(normalize_recorded_reasoning_effort)
            .map(str::to_string),
        success,
        http_status,
        error_category: error_category.unwrap_or_default().trim().to_string(),
        error_message: error_message.unwrap_or_default().trim().to_string(),
        latency_ms,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_tokens: usage.cached_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        token_breakdown: usage.token_breakdown.clone(),
        estimated_cost_usd,
        model_pricing_version: model_pricing_version.max(DEFAULT_MODEL_PRICING_VERSION),
        input_usd_per_million: pricing
            .map(|item| item.input_usd_per_million)
            .unwrap_or_default(),
        output_usd_per_million: pricing
            .map(|item| item.output_usd_per_million)
            .unwrap_or_default(),
        cached_input_usd_per_million: pricing.and_then(|item| item.cached_input_usd_per_million),
    };
    push_recent_usage_event(events, event.clone());
    event
}

fn push_recent_usage_event(
    events: &mut Vec<CodexLocalAccessUsageEvent>,
    event: CodexLocalAccessUsageEvent,
) {
    events.push(event);
    if events.len() > STATE_RECENT_USAGE_EVENT_LIMIT {
        let remove_count = events.len() - STATE_RECENT_USAGE_EVENT_LIMIT;
        events.drain(..remove_count);
    }
}

fn apply_usage_event_to_window(
    window: &mut CodexLocalAccessStatsWindow,
    event: &CodexLocalAccessUsageEvent,
) {
    let usage = UsageCapture {
        input_tokens: event.input_tokens,
        output_tokens: event.output_tokens,
        total_tokens: event.total_tokens,
        cached_tokens: event.cached_tokens,
        reasoning_tokens: event.reasoning_tokens,
        token_breakdown: event.token_breakdown.clone(),
    };
    apply_usage_stats(
        &mut window.totals,
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
    );
    upsert_account_usage_stats(
        &mut window.accounts,
        Some(event.account_id.as_str()),
        Some(event.email.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_model_usage_stats(
        &mut window.models,
        Some(event.model_id.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_api_key_usage_stats(
        &mut window.api_keys,
        Some(event.api_key_id.as_str()),
        Some(event.api_key_label.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    window.updated_at = window.updated_at.max(event.timestamp);
}

fn usage_trend_bucket_start(timestamp: i64, hourly: bool) -> i64 {
    let local = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp)
        .unwrap_or_else(chrono::Utc::now)
        .with_timezone(&Local);
    if !hourly {
        return local_date_start_ms(local.date_naive());
    }
    let hour = local
        .date_naive()
        .and_hms_opt(local.hour(), 0, 0)
        .expect("a local hour must be valid");
    match Local.from_local_datetime(&hour) {
        LocalResult::Single(value) => value.timestamp_millis(),
        LocalResult::Ambiguous(first, second) => {
            first.timestamp_millis().min(second.timestamp_millis())
        }
        LocalResult::None => local_date_start_ms(local.date_naive()),
    }
}

fn apply_usage_event_to_trend(
    trend: &mut HashMap<i64, CodexLocalAccessUsageTrendPoint>,
    event: &CodexLocalAccessUsageEvent,
    hourly: bool,
) {
    let bucket_start = usage_trend_bucket_start(event.timestamp, hourly);
    let point = trend
        .entry(bucket_start)
        .or_insert_with(|| CodexLocalAccessUsageTrendPoint {
            bucket_start,
            usage: CodexLocalAccessUsageStats::default(),
        });
    let usage = UsageCapture {
        input_tokens: event.input_tokens,
        output_tokens: event.output_tokens,
        total_tokens: event.total_tokens,
        cached_tokens: event.cached_tokens,
        reasoning_tokens: event.reasoning_tokens,
        token_breakdown: event.token_breakdown.clone(),
    };
    apply_usage_stats(
        &mut point.usage,
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
    );
}

fn ensure_stats_windows_current(stats: &mut CodexLocalAccessStats, now: i64) {
    let (day_since, week_since, month_since) = local_calendar_window_starts(now);
    let updated_at = stats.updated_at.max(now);
    if stats.daily.since != day_since {
        stats.daily = empty_stats_window(day_since, updated_at);
    }
    if stats.weekly.since != week_since {
        stats.weekly = empty_stats_window(week_since, updated_at);
    }
    if stats.monthly.since != month_since {
        stats.monthly = empty_stats_window(month_since, updated_at);
    }
}

fn apply_usage_event_to_current_windows(
    stats: &mut CodexLocalAccessStats,
    event: &CodexLocalAccessUsageEvent,
    now: i64,
) {
    ensure_stats_windows_current(stats, now);
    if event.timestamp >= stats.monthly.since {
        apply_usage_event_to_window(&mut stats.monthly, event);
    }
    if event.timestamp >= stats.weekly.since {
        apply_usage_event_to_window(&mut stats.weekly, event);
    }
    if event.timestamp >= stats.daily.since {
        apply_usage_event_to_window(&mut stats.daily, event);
    }
}

fn load_stats_windows_and_recent_events_from_conn(
    conn: &Connection,
    now: i64,
) -> Result<
    (
        CodexLocalAccessStatsWindow,
        CodexLocalAccessStatsWindow,
        CodexLocalAccessStatsWindow,
        Vec<CodexLocalAccessUsageEvent>,
    ),
    String,
> {
    let (day_since, week_since, month_since) = local_calendar_window_starts(now);
    let mut daily = empty_stats_window(day_since, day_since);
    let mut weekly = empty_stats_window(week_since, week_since);
    let mut monthly = empty_stats_window(month_since, month_since);
    let mut recent_events = Vec::with_capacity(STATE_RECENT_USAGE_EVENT_LIMIT);

    for_each_local_access_usage_event_since_from_conn(
        conn,
        week_since.min(month_since),
        |event| {
            if event.timestamp >= month_since {
                apply_usage_event_to_window(&mut monthly, &event);
            }
            if event.timestamp >= week_since {
                apply_usage_event_to_window(&mut weekly, &event);
            }
            if event.timestamp >= day_since {
                apply_usage_event_to_window(&mut daily, &event);
            }
            push_recent_usage_event(&mut recent_events, event);
            Ok(())
        },
    )?;

    for window in [&mut daily, &mut weekly, &mut monthly] {
        sort_usage_accounts(&mut window.accounts);
        sort_usage_models(&mut window.models);
        sort_usage_api_keys(&mut window.api_keys);
    }
    Ok((daily, weekly, monthly, recent_events))
}

fn load_stats_windows_and_recent_events(
    now: i64,
) -> Result<
    (
        CodexLocalAccessStatsWindow,
        CodexLocalAccessStatsWindow,
        CodexLocalAccessStatsWindow,
        Vec<CodexLocalAccessUsageEvent>,
    ),
    String,
> {
    let conn = open_local_access_logs_db()?;
    load_stats_windows_and_recent_events_from_conn(&conn, now)
}

fn recompute_time_windows(stats: &mut CodexLocalAccessStats, now: i64) {
    let (day_since, week_since, month_since) = local_calendar_window_starts(now);

    trim_recent_events(&mut stats.events, week_since.min(month_since));

    let mut daily = empty_stats_window(day_since, stats.updated_at.max(day_since));
    let mut weekly = empty_stats_window(week_since, stats.updated_at.max(week_since));
    let mut monthly = empty_stats_window(month_since, stats.updated_at.max(month_since));

    for event in &stats.events {
        if event.timestamp >= month_since {
            apply_usage_event_to_window(&mut monthly, event);
        }
        if event.timestamp >= week_since {
            apply_usage_event_to_window(&mut weekly, event);
        }
        if event.timestamp >= day_since {
            apply_usage_event_to_window(&mut daily, event);
        }
    }

    sort_usage_accounts(&mut daily.accounts);
    sort_usage_accounts(&mut weekly.accounts);
    sort_usage_accounts(&mut monthly.accounts);
    sort_usage_models(&mut daily.models);
    sort_usage_models(&mut weekly.models);
    sort_usage_models(&mut monthly.models);
    sort_usage_api_keys(&mut daily.api_keys);
    sort_usage_api_keys(&mut weekly.api_keys);
    sort_usage_api_keys(&mut monthly.api_keys);

    stats.daily = daily;
    stats.weekly = weekly;
    stats.monthly = monthly;
}

fn apply_cost_delta(target: &mut CodexLocalAccessUsageStats, delta: f64) {
    if !delta.is_finite() || delta == 0.0 {
        return;
    }
    target.estimated_cost_usd = (target.estimated_cost_usd + delta).max(0.0);
}

fn apply_cost_delta_to_stats_list<T, F, G>(
    items: &mut [T],
    key: &str,
    delta: f64,
    key_for: F,
    usage_for: G,
) where
    F: Fn(&T) -> &str,
    G: Fn(&mut T) -> &mut CodexLocalAccessUsageStats,
{
    let Some(item) = items.iter_mut().find(|item| key_for(item) == key) else {
        return;
    };
    apply_cost_delta(usage_for(item), delta);
}

fn apply_cost_delta_to_window(
    window: &mut CodexLocalAccessStatsWindow,
    change: &RequestLogRepriceChange,
) {
    apply_cost_delta(&mut window.totals, change.estimated_cost_delta_usd);
    apply_cost_delta_to_stats_list(
        &mut window.accounts,
        change.account_id.as_str(),
        change.estimated_cost_delta_usd,
        |item| item.account_id.as_str(),
        |item| &mut item.usage,
    );
    apply_cost_delta_to_stats_list(
        &mut window.models,
        change.model_id.as_str(),
        change.estimated_cost_delta_usd,
        |item| item.model_id.as_str(),
        |item| &mut item.usage,
    );
    apply_cost_delta_to_stats_list(
        &mut window.api_keys,
        change.api_key_id.as_str(),
        change.estimated_cost_delta_usd,
        |item| item.api_key_id.as_str(),
        |item| &mut item.usage,
    );
}

fn apply_reprice_changes_to_stats(
    stats: &mut CodexLocalAccessStats,
    changes: &[RequestLogRepriceChange],
) {
    if changes.is_empty() {
        return;
    }

    let now = now_ms();
    let (day_since, week_since, month_since) = local_calendar_window_starts(now);
    let event_indexes =
        stats
            .events
            .iter()
            .enumerate()
            .fold(HashMap::new(), |mut indexes, (index, event)| {
                indexes
                    .entry(local_access_log_event_key(event))
                    .or_insert(index);
                indexes
            });

    for change in changes {
        apply_cost_delta(&mut stats.totals, change.estimated_cost_delta_usd);
        apply_cost_delta_to_stats_list(
            &mut stats.accounts,
            change.account_id.as_str(),
            change.estimated_cost_delta_usd,
            |item| item.account_id.as_str(),
            |item| &mut item.usage,
        );
        apply_cost_delta_to_stats_list(
            &mut stats.models,
            change.model_id.as_str(),
            change.estimated_cost_delta_usd,
            |item| item.model_id.as_str(),
            |item| &mut item.usage,
        );
        apply_cost_delta_to_stats_list(
            &mut stats.api_keys,
            change.api_key_id.as_str(),
            change.estimated_cost_delta_usd,
            |item| item.api_key_id.as_str(),
            |item| &mut item.usage,
        );

        if change.timestamp >= month_since {
            apply_cost_delta_to_window(&mut stats.monthly, change);
        }
        if change.timestamp >= week_since {
            apply_cost_delta_to_window(&mut stats.weekly, change);
        }
        if change.timestamp >= day_since {
            apply_cost_delta_to_window(&mut stats.daily, change);
        }

        if let Some(index) = event_indexes.get(change.event_key.as_str()) {
            if let Some(event) = stats.events.get_mut(*index) {
                apply_cost_delta_to_event(event, change.estimated_cost_delta_usd);
            }
        }
    }

    stats.updated_at = stats.updated_at.max(now);
    sort_usage_accounts(&mut stats.accounts);
    sort_usage_models(&mut stats.models);
    sort_usage_api_keys(&mut stats.api_keys);
    sort_usage_accounts(&mut stats.daily.accounts);
    sort_usage_models(&mut stats.daily.models);
    sort_usage_api_keys(&mut stats.daily.api_keys);
    sort_usage_accounts(&mut stats.weekly.accounts);
    sort_usage_models(&mut stats.weekly.models);
    sort_usage_api_keys(&mut stats.weekly.api_keys);
    sort_usage_accounts(&mut stats.monthly.accounts);
    sort_usage_models(&mut stats.monthly.models);
    sort_usage_api_keys(&mut stats.monthly.api_keys);
}
