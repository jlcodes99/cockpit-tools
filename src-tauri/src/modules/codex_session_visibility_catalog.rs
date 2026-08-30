// Codex Session Visibility：Session index, rollout catalog reconciliation and timestamp normalization。
// 通过 include! 保持原模块作用域、私有调用关系和修复事务行为。
fn normalize_global_state(data_dir: &Path, dry_run: bool) -> Result<usize, String> {
    let path = data_dir.join(GLOBAL_STATE_FILE);
    if !path.exists() {
        return Ok(0);
    }
    let mut state = read_global_state_object(data_dir)?;
    let normalized = normalized_global_state_entries(&state);
    let changed = normalized
        .iter()
        .filter(|(key, value)| state.get(*key) != Some(*value))
        .count();
    if changed == 0 || dry_run {
        return Ok(changed);
    }
    for (key, value) in normalized {
        state.insert(key, value);
    }
    let content = serde_json::to_string_pretty(&JsonValue::Object(state))
        .map_err(|error| format!("序列化 Codex 全局状态失败: {error}"))?;
    write_bytes_atomic(&path, format!("{content}\n").as_bytes())?;
    Ok(changed)
}

fn build_encrypted_content_warning(
    counts: &HashMap<String, usize>,
    target_provider: &str,
) -> Option<String> {
    let mut providers = counts
        .iter()
        .filter(|(provider, count)| **count > 0 && provider.as_str() != target_provider)
        .map(|(provider, _)| provider.clone())
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        return None;
    }
    let total = counts.values().sum::<usize>();
    Some(format!(
        "检测到 {total} 个会话文件包含来自 {} 的 encrypted_content。会话元数据可以迁移到 {target_provider}，但继续或压缩这些历史时仍可能出现 invalid_encrypted_content；需要可靠续聊时请切回原 Provider/账号或开启新会话。",
        providers.join("、")
    ))
}

pub(crate) fn to_desktop_workspace_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let lower = value.to_ascii_lowercase();
    let mut normalized = value.to_string();
    if lower.starts_with(r"\\?\unc\") {
        normalized = format!(r"\\{}", &value[8..]);
    } else if lower.starts_with(r"\\?\") {
        normalized = value[4..].to_string();
    }

    if is_windows_workspace_path(&normalized) {
        trim_safe_windows_trailing_separators(&mut normalized);
    }
    Some(normalized)
}

fn is_windows_workspace_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with(r"\\")
        || value.starts_with("//")
        || bytes.get(1).is_some_and(|separator| *separator == b':')
}

fn trim_safe_windows_trailing_separators(value: &mut String) {
    while matches!(value.as_bytes().last(), Some(b'\\' | b'/')) {
        let bytes = value.as_bytes();
        let is_drive_root = bytes.len() == 3
            && bytes.get(1) == Some(&b':')
            && matches!(bytes.get(2), Some(b'\\' | b'/'));
        if is_drive_root {
            break;
        }

        let starts_with_unc_prefix = bytes.len() >= 2
            && matches!(bytes.first(), Some(b'\\' | b'/'))
            && matches!(bytes.get(1), Some(b'\\' | b'/'));
        if starts_with_unc_prefix {
            let component_count = value[2..]
                .split(['\\', '/'])
                .filter(|component| !component.is_empty())
                .count();
            if component_count < 2 {
                break;
            }
        } else if value.len() <= 1 {
            break;
        }

        value.pop();
    }
}

fn list_rollout_files(root_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    let entries = fs::read_dir(root_dir)
        .map_err(|error| format!("读取目录失败 ({}): {}", root_dir.display(), error))?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("读取目录项失败 ({}): {}", root_dir.display(), error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取文件类型失败 ({}): {}", path.display(), error))?;
        if file_type.is_dir() {
            result.extend(list_rollout_files(&path)?);
            continue;
        }
        if file_type.is_file() && is_plain_rollout_file(&path) {
            result.push(path);
        }
    }

    result.sort();
    Ok(result)
}

fn is_plain_rollout_file(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or_default();
    file_name.starts_with("rollout-")
        && path.extension().and_then(|item| item.to_str()) == Some("jsonl")
}

fn read_session_index_map(root_dir: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = root_dir.join(SESSION_INDEX_FILE);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    let mut entries = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JsonValue>(trimmed) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        entries.insert(id.to_string(), entry);
    }
    Ok(entries)
}

fn count_session_index_entries_to_repair(
    data_dir: &Path,
) -> Result<SessionIndexRepairScan, String> {
    count_session_index_entries_to_repair_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_session_index_entries_to_repair_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SessionIndexRepairScan, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_for_options(data_dir, options, selection)?;
    let mut scan = SessionIndexRepairScan::default();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(entry)
                if options.update_existing_session_index_entries
                    && session_index_entry_needs_update(data_dir, row, entry) =>
            {
                scan.entries_to_update += 1;
            }
            Some(_) => {}
            None => {
                scan.entries_to_add += 1;
            }
        }
    }
    Ok(scan)
}

fn count_missing_session_index_entries(data_dir: &Path) -> Result<usize, String> {
    Ok(count_session_index_entries_to_repair(data_dir)?.entries_to_add)
}

fn load_sqlite_thread_index_rows(data_dir: &Path) -> Result<Vec<SqliteThreadIndexRow>, String> {
    load_sqlite_thread_index_rows_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn load_sqlite_thread_index_rows_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        for row in load_sqlite_thread_index_rows_from_db(&db_path)? {
            if !selection.includes_session_id(&row.id) {
                continue;
            }
            if seen_ids.insert(row.id.clone()) {
                rows.push(row);
            }
        }
    }
    Ok(rows)
}

fn load_sqlite_thread_index_rows_from_db(
    db_path: &Path,
) -> Result<Vec<SqliteThreadIndexRow>, String> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => return Ok(Vec::new()),
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    if !names.contains("id") {
        return Ok(Vec::new());
    }

    let title_expr = if names.contains("title") {
        "COALESCE(title, '')"
    } else {
        "''"
    };
    let updated_at_expr = if names.contains("updated_at") {
        "updated_at"
    } else {
        "NULL"
    };
    let updated_at_ms_expr = if names.contains("updated_at_ms") {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let rollout_path_expr = if names.contains("rollout_path") {
        "rollout_path"
    } else {
        "NULL"
    };
    let order_expr = if names.contains("updated_at") {
        "updated_at DESC"
    } else {
        "id ASC"
    };
    let sql = format!(
        "SELECT id, {title_expr}, {updated_at_expr}, {updated_at_ms_expr}, {rollout_path_expr} FROM threads ORDER BY {order_expr}"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format!(
            "准备 SQLite 会话索引查询失败 ({}): {}",
            db_path.display(),
            error
        )
    })?;
    let mapped = statement
        .query_map([], |row| {
            Ok(SqliteThreadIndexRow {
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: row.get(2)?,
                updated_at_ms: row.get(3)?,
                rollout_path: row.get(4)?,
            })
        })
        .map_err(|error| {
            format!(
                "查询 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let mut result = Vec::new();
    for row in mapped {
        result.push(row.map_err(|error| {
            format!(
                "读取 SQLite 会话索引行失败 ({}): {}",
                db_path.display(),
                error
            )
        })?);
    }
    Ok(result)
}

fn format_thread_updated_at_iso_ms(updated_at_ms: Option<i128>) -> String {
    let milliseconds = updated_at_ms.unwrap_or_else(|| Utc::now().timestamp_millis() as i128);
    i64::try_from(milliseconds)
        .ok()
        .and_then(|value| Utc.timestamp_millis_opt(value).single())
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

fn resolve_thread_updated_at_ms(data_dir: &Path, row: &SqliteThreadIndexRow) -> Option<i128> {
    let rollout_activity_ms = row
        .rollout_path
        .as_deref()
        .map(|path| resolve_rollout_path(data_dir, path))
        .filter(|path| path.exists())
        .and_then(|path| rollout_file_activity_ms(&path));
    let sqlite_ms = row
        .updated_at_ms
        .map(|value| value as i128)
        .or_else(|| row.updated_at.map(|value| value as i128 * 1000));
    match (sqlite_ms, rollout_activity_ms) {
        (Some(sqlite_ms), Some(activity_ms))
            if (sqlite_ms - activity_ms).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity_ms)
        }
        (Some(sqlite_ms), _) => Some(sqlite_ms),
        (None, Some(activity_ms)) => Some(activity_ms),
        (None, None) => None,
    }
}

fn build_session_index_entry_from_thread(data_dir: &Path, row: &SqliteThreadIndexRow) -> JsonValue {
    json!({
        "id": row.id,
        "thread_name": if row.title.trim().is_empty() {
            "Untitled"
        } else {
            row.title.as_str()
        },
        "updated_at": format_thread_updated_at_iso_ms(resolve_thread_updated_at_ms(data_dir, row)),
    })
}

fn build_updated_session_index_entry(
    data_dir: &Path,
    existing: &JsonValue,
    row: &SqliteThreadIndexRow,
) -> JsonValue {
    let mut entry = existing.clone();
    let Some(object) = entry.as_object_mut() else {
        return build_session_index_entry_from_thread(data_dir, row);
    };
    object.insert("id".to_string(), JsonValue::String(row.id.clone()));
    if !row.title.trim().is_empty() {
        object.insert(
            "thread_name".to_string(),
            JsonValue::String(row.title.clone()),
        );
    }
    object.insert(
        "updated_at".to_string(),
        JsonValue::String(format_thread_updated_at_iso_ms(
            resolve_thread_updated_at_ms(data_dir, row),
        )),
    );
    entry
}

fn session_index_entry_needs_update(
    data_dir: &Path,
    row: &SqliteThreadIndexRow,
    entry: &JsonValue,
) -> bool {
    let Some(target_ms) = resolve_thread_updated_at_ms(data_dir, row) else {
        return false;
    };
    match parse_session_index_updated_at_ms(entry) {
        Some(current_ms) => (current_ms - target_ms).abs() > 1000,
        None => true,
    }
}

fn reconcile_session_index_from_sqlite(
    data_dir: &Path,
) -> Result<SessionIndexReconcileResult, String> {
    reconcile_session_index_from_sqlite_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn reconcile_session_index_from_sqlite_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SessionIndexReconcileResult, String> {
    let session_index_map = read_session_index_map(data_dir)?;
    let rows = load_sqlite_thread_index_rows_for_options(data_dir, options, selection)?;
    let mut entries_to_add = Vec::<JsonValue>::new();
    let mut entries_to_update = HashMap::<String, JsonValue>::new();
    for row in &rows {
        match session_index_map.get(&row.id) {
            Some(existing)
                if options.update_existing_session_index_entries
                    && session_index_entry_needs_update(data_dir, row, existing) =>
            {
                entries_to_update.insert(
                    row.id.clone(),
                    build_updated_session_index_entry(data_dir, existing, row),
                );
            }
            Some(_) => {}
            None => entries_to_add.push(build_session_index_entry_from_thread(data_dir, row)),
        }
    }
    if entries_to_add.is_empty() && entries_to_update.is_empty() {
        return Ok(SessionIndexReconcileResult::default());
    }

    let path = data_dir.join(SESSION_INDEX_FILE);
    let mut lines = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|error| {
                format!(
                    "读取 session_index.jsonl 失败 ({}): {}",
                    path.display(),
                    error
                )
            })?
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut updated_ids = HashSet::<String>::new();
    for line in &mut lines {
        let Ok(entry) = serde_json::from_str::<JsonValue>(line.trim()) else {
            continue;
        };
        let Some(id) = entry.get("id").and_then(JsonValue::as_str) else {
            continue;
        };
        let Some(updated_entry) = entries_to_update.get(id) else {
            continue;
        };
        *line = serde_json::to_string(updated_entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        updated_ids.insert(id.to_string());
    }

    for entry in &entries_to_add {
        let line = serde_json::to_string(&entry)
            .map_err(|error| format!("序列化 session_index 条目失败: {}", error))?;
        lines.push(line);
    }

    let mut output = lines.join("\n");
    output.push('\n');
    modules::atomic_write::write_string_atomic(&path, &output).map_err(|error| {
        format!(
            "写入 session_index.jsonl 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    Ok(SessionIndexReconcileResult {
        added_entries: entries_to_add.len(),
        updated_entries: updated_ids.len(),
    })
}

fn normalize_codex_timestamp_ms(timestamp: i64) -> i128 {
    let timestamp = timestamp as i128;
    if timestamp > 10_000_000_000_000 {
        timestamp / 1_000
    } else if timestamp > 10_000_000_000 {
        timestamp
    } else {
        timestamp * 1_000
    }
}

fn parse_timestamp_ms(value: &JsonValue) -> Option<i128> {
    match value {
        JsonValue::Number(number) => number.as_i64().map(normalize_codex_timestamp_ms),
        JsonValue::String(text) => chrono::DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|value| value.timestamp_millis() as i128)
            .or_else(|| text.parse::<i64>().ok().map(normalize_codex_timestamp_ms)),
        _ => None,
    }
}

fn parse_session_index_updated_at_ms(entry: &JsonValue) -> Option<i128> {
    [
        "updated_at",
        "updatedAt",
        "last_updated_at",
        "lastUpdatedAt",
    ]
    .iter()
    .filter_map(|key| entry.get(*key))
    .find_map(parse_timestamp_ms)
}

fn parse_rollout_line_timestamp_ms(value: &JsonValue) -> Option<i128> {
    value
        .get("timestamp")
        .or_else(|| value.get("time"))
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))
        .and_then(parse_timestamp_ms)
        .or_else(|| {
            value
                .get("payload")
                .and_then(|payload| {
                    payload
                        .get("timestamp")
                        .or_else(|| payload.get("time"))
                        .or_else(|| payload.get("created_at"))
                        .or_else(|| payload.get("createdAt"))
                })
                .and_then(parse_timestamp_ms)
        })
}

fn rollout_file_activity_ms(path: &Path) -> Option<i128> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<JsonValue>(line.trim()).ok())
        .filter_map(|value| parse_rollout_line_timestamp_ms(&value))
        .max()
}

fn resolve_target_modified_at_ms(
    session_id: Option<&str>,
    session_index_map: &HashMap<String, JsonValue>,
    rollout_path: &Path,
    fallback_ms: Option<i128>,
) -> Option<i128> {
    let indexed = session_id
        .and_then(|id| session_index_map.get(id))
        .and_then(parse_session_index_updated_at_ms);
    let activity = rollout_file_activity_ms(rollout_path);
    match (indexed, activity) {
        (Some(indexed), Some(activity))
            if (indexed - activity).abs() > SESSION_INDEX_ACTIVITY_DRIFT_MS =>
        {
            Some(activity)
        }
        (Some(indexed), _) => Some(indexed),
        (None, Some(activity)) => Some(activity),
        (None, None) => fallback_ms,
    }
}

fn resolve_rollout_path(data_dir: &Path, rollout_path: &str) -> PathBuf {
    let trimmed = rollout_path.trim();
    let stripped = trimmed.strip_prefix(r"\\?\").unwrap_or(trimmed);
    let path = Path::new(stripped);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        data_dir.join(path)
    }
}

fn count_sqlite_thread_timestamps_to_update(data_dir: &Path) -> Result<usize, String> {
    count_sqlite_thread_timestamps_to_update_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_sqlite_thread_timestamps_to_update_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total += plan_sqlite_thread_timestamp_repair_for_db(data_dir, &db_path, selection)?
            .updates
            .len();
    }
    Ok(total)
}

fn plan_sqlite_thread_timestamp_repair_for_db(
    data_dir: &Path,
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<SqliteTimestampRepairPlan, String> {
    if !db_path.exists() {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteTimestampRepairPlan::default());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut statement = match connection.prepare("PRAGMA table_info(threads)") {
        Ok(statement) => statement,
        Err(error) if is_missing_threads_table_error(&error) => {
            return Ok(SqliteTimestampRepairPlan::default())
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let rows = statement
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let mut names = HashSet::new();
    for row in rows {
        names.insert(row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?);
    }
    drop(statement);

    let has_updated_at = names.contains("updated_at");
    let has_updated_at_ms = names.contains("updated_at_ms");
    if !names.contains("id")
        || !names.contains("rollout_path")
        || (!has_updated_at && !has_updated_at_ms)
    {
        return Ok(SqliteTimestampRepairPlan::default());
    }

    let updated_at_expr = if has_updated_at { "updated_at" } else { "NULL" };
    let updated_at_ms_expr = if has_updated_at_ms {
        "updated_at_ms"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, rollout_path, {updated_at_expr}, {updated_at_ms_expr} FROM threads WHERE rollout_path IS NOT NULL AND rollout_path <> ''"
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format_sqlite_read_error(db_path, "准备 SQLite 会话时间修复查询失败", &error)
    })?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| format_sqlite_read_error(db_path, "查询 SQLite 会话时间失败", &error))?;

    let mut updates = Vec::new();
    for row in rows {
        let (thread_id, rollout_path, updated_at, updated_at_ms) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite 会话时间失败", &error)
        })?;
        if !selection.includes_session_id(&thread_id) {
            continue;
        }
        let rollout = resolve_rollout_path(data_dir, &rollout_path);
        if !rollout.exists() {
            continue;
        }
        let Some(activity_ms) = rollout_file_activity_ms(&rollout) else {
            continue;
        };
        let activity_seconds = (activity_ms / 1000) as i64;
        let activity_ms = activity_seconds * 1000;
        let current_ms = updated_at_ms
            .or_else(|| updated_at.map(|value| value * 1000))
            .unwrap_or(0);
        if i64::abs(current_ms - activity_ms) <= 1000 {
            continue;
        }
        updates.push(SqliteTimestampUpdate {
            id: thread_id,
            updated_at_seconds: activity_seconds,
            updated_at_ms: activity_ms,
        });
    }
    Ok(SqliteTimestampRepairPlan {
        updates,
        has_updated_at,
        has_updated_at_ms,
    })
}

fn repair_sqlite_thread_timestamps(data_dir: &Path) -> Result<usize, String> {
    repair_sqlite_thread_timestamps_for_options(
        data_dir,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn repair_sqlite_thread_timestamps_for_options(
    data_dir: &Path,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total += repair_sqlite_thread_timestamps_for_db(data_dir, &db_path, selection)?;
    }
    Ok(total)
}

fn repair_sqlite_thread_timestamps_for_db(
    data_dir: &Path,
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let plan = plan_sqlite_thread_timestamp_repair_for_db(data_dir, db_path, selection)?;
    let updates = plan.updates;

    if updates.is_empty() {
        return Ok(0);
    }

    let mut connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    for update in &updates {
        if plan.has_updated_at && plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1, updated_at_ms = ?2 WHERE id = ?3",
                    (
                        update.updated_at_seconds,
                        update.updated_at_ms,
                        update.id.as_str(),
                    ),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at {
            transaction
                .execute(
                    "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
                    (update.updated_at_seconds, update.id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        } else if plan.has_updated_at_ms {
            transaction
                .execute(
                    "UPDATE threads SET updated_at_ms = ?1 WHERE id = ?2",
                    (update.updated_at_ms, update.id.as_str()),
                )
                .map_err(|error| format_sqlite_write_error(db_path, &error))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updates.len())
}

fn is_missing_threads_table_error(error: &rusqlite::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table: threads")
}

fn log_skipped_sqlite_database(path: &Path, reason: &str) {
    modules::logger::log_warn(&format!(
        "跳过无效或损坏的 Codex SQLite 会话库 ({}): {}",
        path.display(),
        reason
    ));
}

fn collect_sqlite_cwd_normalizations(
    connection: &Connection,
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<Vec<(String, String)>, String> {
    let mut statement = connection
        .prepare("SELECT id, cwd FROM threads WHERE COALESCE(cwd, '') <> ''")
        .map_err(|error| format_sqlite_read_error(db_path, "读取 SQLite cwd 失败", &error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<usize, String>(0)?, row.get::<usize, String>(1)?))
        })
        .map_err(|error| format_sqlite_read_error(db_path, "读取 SQLite cwd 失败", &error))?;
    let mut normalizations = Vec::new();
    for row in rows {
        let (thread_id, cwd) =
            row.map_err(|error| format_sqlite_read_error(db_path, "读取 SQLite cwd 失败", &error))?;
        if !selection.includes_session_id(&thread_id) || !is_windows_workspace_path(cwd.trim()) {
            continue;
        }
        let Some(normalized) = to_desktop_workspace_path(&cwd) else {
            continue;
        };
        if normalized != cwd {
            normalizations.push((thread_id, normalized));
        }
    }
    Ok(normalizations)
}

fn normalize_sqlite_thread_cwds_for_db(
    db_path: &Path,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    if !db_path.exists() {
        return Ok(0);
    }

    let mut connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    connection
        .busy_timeout(Duration::from_secs(3))
        .map_err(|error| {
            format!(
                "设置 SQLite busy_timeout 失败 ({}): {}",
                db_path.display(),
                error
            )
        })?;
    let columns = match read_threads_table_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        Err(error) => {
            return Err(format_sqlite_read_error(
                db_path,
                "读取 SQLite threads 表结构失败",
                &error,
            ));
        }
    };
    let Some(columns) = columns else {
        return Ok(0);
    };
    if !columns.id || !columns.cwd {
        return Ok(0);
    }

    let normalizations = collect_sqlite_cwd_normalizations(&connection, db_path, selection)?;
    if normalizations.is_empty() {
        return Ok(0);
    }
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let mut updated_rows = 0usize;
    for (thread_id, cwd) in normalizations {
        updated_rows += transaction
            .execute(
                "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                (cwd.as_str(), thread_id.as_str()),
            )
            .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    }
    transaction
        .commit()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    Ok(updated_rows)
}

