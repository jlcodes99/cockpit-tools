// Codex Session Visibility：SQLite thread cwd/provider repair and local catalog reconciliation。
// 通过 include! 保持原模块作用域、私有调用关系和修复事务行为。
pub(crate) fn normalize_official_thread_cwds(data_dir: &Path) -> Result<usize, String> {
    let selection = RepairTargetSelection::default();
    let mut total = 0usize;
    for db_path in official_state_db_candidate_paths(data_dir) {
        total += normalize_sqlite_thread_cwds_for_db(&db_path, &selection)?;
    }
    Ok(total)
}

fn count_sqlite_rows_to_update(
    data_dir: &Path,
    target_provider: &str,
) -> Result<SqliteProviderScan, String> {
    count_sqlite_rows_to_update_for_options(
        data_dir,
        target_provider,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn count_sqlite_rows_to_update_for_options(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<SqliteProviderScan, String> {
    let facts = if options.collect_rollout_thread_facts {
        Some(collect_rollout_thread_facts(data_dir, selection)?)
    } else {
        None
    };
    let mut scan = SqliteProviderScan {
        rows_to_update: 0,
        skipped_unusable_database: false,
    };
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        let item = count_sqlite_rows_to_update_for_db(
            &db_path,
            target_provider,
            options.sidebar_visible_only,
            facts.as_ref(),
            selection,
        )?;
        scan.rows_to_update += item.rows_to_update;
        scan.skipped_unusable_database |= item.skipped_unusable_database;
    }
    Ok(scan)
}

fn count_sqlite_rows_to_update_for_db(
    db_path: &Path,
    target_provider: &str,
    sidebar_visible_only: bool,
    facts: Option<&RolloutThreadFacts>,
    selection: &RepairTargetSelection,
) -> Result<SqliteProviderScan, String> {
    if !db_path.exists() {
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    }

    let connection = match Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };
    let columns = match read_threads_table_columns(&connection) {
        Ok(columns) => columns,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(SqliteProviderScan {
                rows_to_update: 0,
                skipped_unusable_database: true,
            });
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
        return Ok(SqliteProviderScan {
            rows_to_update: 0,
            skipped_unusable_database: false,
        });
    };
    let mut count = 0i64;
    if let Some(where_clause) = build_threads_repair_where_clause(columns, sidebar_visible_only) {
        if let Some(facts) = facts {
            count += collect_eligible_thread_ids_for_repair(
                &connection,
                columns,
                &where_clause,
                target_provider,
                selection,
                &facts.subagent_thread_ids,
            )?
            .len() as i64;
        } else if let Some(session_ids) = selection.session_ids() {
            if columns.model_provider {
                let sql =
                    format!("SELECT COUNT(*) FROM threads WHERE ({where_clause}) AND id = ?2");
                for thread_id in session_ids {
                    count += connection
                        .query_row(sql.as_str(), (target_provider, thread_id.as_str()), |row| {
                            row.get::<usize, i64>(0)
                        })
                        .map_err(|error| {
                            format!(
                                "统计 SQLite 会话可见性差异失败 ({}): {}",
                                db_path.display(),
                                error
                            )
                        })?;
                }
            } else {
                let sql =
                    format!("SELECT COUNT(*) FROM threads WHERE ({where_clause}) AND id = ?1");
                for thread_id in session_ids {
                    count += connection
                        .query_row(sql.as_str(), [thread_id.as_str()], |row| {
                            row.get::<usize, i64>(0)
                        })
                        .map_err(|error| {
                            format!(
                                "统计 SQLite 会话可见性差异失败 ({}): {}",
                                db_path.display(),
                                error
                            )
                        })?;
                }
            }
        } else {
            let sql = format!("SELECT COUNT(*) FROM threads WHERE {where_clause}");
            let count_result = if columns.model_provider {
                connection.query_row(sql.as_str(), [target_provider], |row| {
                    row.get::<usize, i64>(0)
                })
            } else {
                connection.query_row(sql.as_str(), [], |row| row.get::<usize, i64>(0))
            };
            count += match count_result {
                Ok(count) => count,
                Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                    log_skipped_sqlite_database(db_path, &error.to_string());
                    return Ok(SqliteProviderScan {
                        rows_to_update: 0,
                        skipped_unusable_database: true,
                    });
                }
                Err(error) if is_missing_threads_table_error(&error) => {
                    return Ok(SqliteProviderScan {
                        rows_to_update: 0,
                        skipped_unusable_database: false,
                    });
                }
                Err(error) => {
                    return Err(format!(
                        "统计 SQLite 会话可见性差异失败 ({}): {}",
                        db_path.display(),
                        error
                    ));
                }
            };
        }
    }
    if let Some(facts) = facts {
        if columns.has_user_event {
            for thread_id in &facts.user_event_thread_ids {
                count += connection
                    .query_row(
                        "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                        [thread_id.as_str()],
                        |row| row.get::<usize, i64>(0),
                    )
                    .map_err(|error| {
                        format!(
                            "统计 SQLite has_user_event 差异失败 ({}): {}",
                            db_path.display(),
                            error
                        )
                    })?;
            }
        }
        if columns.cwd {
            for (thread_id, cwd) in &facts.cwd_by_thread_id {
                count += connection
                    .query_row(
                        "SELECT COUNT(*) FROM threads WHERE id = ?1 AND COALESCE(cwd, '') <> ?2",
                        (thread_id.as_str(), cwd.as_str()),
                        |row| row.get::<usize, i64>(0),
                    )
                    .map_err(|error| {
                        format!(
                            "统计 SQLite cwd 差异失败 ({}): {}",
                            db_path.display(),
                            error
                        )
                    })?;
            }
        }
    }
    if columns.id && columns.cwd {
        count += collect_sqlite_cwd_normalizations(&connection, db_path, selection)?.len() as i64;
    }
    Ok(SqliteProviderScan {
        rows_to_update: count.max(0) as usize,
        skipped_unusable_database: false,
    })
}

fn update_sqlite_provider(data_dir: &Path, target_provider: &str) -> Result<usize, String> {
    update_sqlite_provider_for_options(
        data_dir,
        target_provider,
        CodexSessionVisibilityRepairOptions::for_mode(CodexSessionVisibilityRepairMode::Deep),
        &RepairTargetSelection::default(),
    )
}

fn update_sqlite_provider_for_options(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<usize, String> {
    let facts = if options.collect_rollout_thread_facts {
        Some(collect_rollout_thread_facts(data_dir, selection)?)
    } else {
        None
    };
    let mut total = 0usize;
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        total += update_sqlite_provider_for_db(
            &db_path,
            target_provider,
            options.sidebar_visible_only,
            facts.as_ref(),
            selection,
        )?;
    }
    Ok(total)
}

fn update_sqlite_provider_for_db(
    db_path: &Path,
    target_provider: &str,
    sidebar_visible_only: bool,
    facts: Option<&RolloutThreadFacts>,
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
    let cwd_normalizations = if columns.id && columns.cwd {
        collect_sqlite_cwd_normalizations(&connection, db_path, selection)?
    } else {
        Vec::new()
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    let mut updated_rows = 0usize;
    if let Some(where_clause) = build_threads_repair_where_clause(columns, sidebar_visible_only) {
        let set_clause = build_threads_repair_set_clause(columns);
        if let Some(facts) = facts {
            let thread_ids = collect_eligible_thread_ids_for_repair(
                &transaction,
                columns,
                &where_clause,
                target_provider,
                selection,
                &facts.subagent_thread_ids,
            )?;
            if columns.model_provider {
                let sql = format!("UPDATE threads SET {set_clause} WHERE id = ?2");
                for thread_id in thread_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), (target_provider, thread_id.as_str()))
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            } else {
                let sql = format!("UPDATE threads SET {set_clause} WHERE id = ?1");
                for thread_id in thread_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), [thread_id.as_str()])
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            }
        } else if let Some(session_ids) = selection.session_ids() {
            if columns.model_provider {
                let sql =
                    format!("UPDATE threads SET {set_clause} WHERE ({where_clause}) AND id = ?2");
                for thread_id in session_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), (target_provider, thread_id.as_str()))
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            } else {
                let sql =
                    format!("UPDATE threads SET {set_clause} WHERE ({where_clause}) AND id = ?1");
                for thread_id in session_ids {
                    updated_rows += transaction
                        .execute(sql.as_str(), [thread_id.as_str()])
                        .map_err(|error| format_sqlite_write_error(db_path, &error))?;
                }
            }
        } else {
            let sql = format!("UPDATE threads SET {set_clause} WHERE {where_clause}");
            let update_result = if columns.model_provider {
                transaction.execute(sql.as_str(), [target_provider])
            } else {
                transaction.execute(sql.as_str(), [])
            };
            updated_rows += match update_result {
                Ok(updated_rows) => updated_rows,
                Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
                    log_skipped_sqlite_database(db_path, &error.to_string());
                    return Ok(0);
                }
                Err(error) if is_missing_threads_table_error(&error) => {
                    return Ok(0);
                }
                Err(error) => return Err(format_sqlite_write_error(db_path, &error)),
            };
        }
    }
    if let Some(facts) = facts {
        if columns.has_user_event {
            for thread_id in &facts.user_event_thread_ids {
                updated_rows += transaction
                    .execute(
                        "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
                        [thread_id.as_str()],
                    )
                    .map_err(|error| format_sqlite_write_error(db_path, &error))?;
            }
        }
        if columns.cwd {
            for (thread_id, cwd) in &facts.cwd_by_thread_id {
                updated_rows += transaction
                    .execute(
                        "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                        (cwd.as_str(), thread_id.as_str()),
                    )
                    .map_err(|error| format_sqlite_write_error(db_path, &error))?;
            }
        }
    }
    for (thread_id, cwd) in cwd_normalizations {
        updated_rows += transaction
            .execute(
                "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                (cwd.as_str(), thread_id.as_str()),
            )
            .map_err(|error| format_sqlite_write_error(db_path, &error))?;
    }
    if let Err(error) = transaction.commit() {
        if modules::db::is_unusable_sqlite_database_error(&error) {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(0);
        }
        return Err(format_sqlite_write_error(db_path, &error));
    }
    Ok(updated_rows)
}

fn read_threads_table_columns(
    connection: &Connection,
) -> Result<Option<ThreadsTableColumns>, rusqlite::Error> {
    let mut statement = connection.prepare("PRAGMA table_info(threads)")?;
    let rows = statement.query_map([], |row| row.get::<usize, String>(1))?;
    let mut names = HashSet::new();
    for row in rows {
        let name = row?;
        names.insert(name);
    }
    if names.is_empty() {
        return Ok(None);
    }
    Ok(Some(ThreadsTableColumns {
        id: names.contains("id"),
        model_provider: names.contains("model_provider"),
        has_user_event: names.contains("has_user_event"),
        first_user_message: names.contains("first_user_message"),
        thread_source: names.contains("thread_source"),
        cwd: names.contains("cwd"),
        archived: names.contains("archived"),
        preview: names.contains("preview"),
        rollout_path: names.contains("rollout_path"),
        source: names.contains("source"),
    }))
}

fn build_threads_repair_where_clause(
    columns: ThreadsTableColumns,
    sidebar_visible_only: bool,
) -> Option<String> {
    let mut predicates = Vec::new();
    if columns.model_provider {
        predicates.push("COALESCE(model_provider, '') <> ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(has_user_event, 0) <> 1)");
    }
    if columns.thread_source && columns.first_user_message {
        predicates
            .push("(COALESCE(first_user_message, '') <> '' AND COALESCE(thread_source, '') = '')");
    }
    if columns.preview && columns.first_user_message {
        predicates.push("(COALESCE(preview, '') = '' AND COALESCE(first_user_message, '') <> '')");
    }
    if predicates.is_empty() {
        None
    } else {
        let mut where_clause = format!("({})", predicates.join(" OR "));
        if sidebar_visible_only {
            let mut visibility = Vec::new();
            if columns.archived {
                visibility.push("COALESCE(archived, 0) = 0");
            }
            if columns.preview && columns.first_user_message {
                visibility.push(
                    "(COALESCE(preview, '') <> '' OR COALESCE(first_user_message, '') <> '')",
                );
            } else if columns.preview {
                visibility.push("COALESCE(preview, '') <> ''");
            }
            if columns.rollout_path {
                visibility.push("COALESCE(rollout_path, '') <> ''");
            }
            if columns.source {
                visibility.push(
                    "LOWER(COALESCE(source, '')) NOT LIKE '%subagent%' AND LOWER(COALESCE(source, '')) NOT LIKE '%internal%'",
                );
            }
            if columns.thread_source {
                visibility.push("COALESCE(thread_source, '') <> 'ambient_suggestions'");
            }
            if !visibility.is_empty() {
                where_clause.push_str(" AND (");
                where_clause.push_str(&visibility.join(" AND "));
                where_clause.push(')');
            }
        }
        Some(where_clause)
    }
}

fn build_threads_repair_set_clause(columns: ThreadsTableColumns) -> String {
    let mut assignments = Vec::new();
    if columns.model_provider {
        assignments.push("model_provider = ?1");
    }
    if columns.has_user_event && columns.first_user_message {
        assignments.push(
            "has_user_event = CASE WHEN COALESCE(first_user_message, '') <> '' THEN 1 ELSE has_user_event END",
        );
    }
    if columns.thread_source && columns.first_user_message {
        assignments.push(
            "thread_source = CASE WHEN COALESCE(thread_source, '') = '' AND COALESCE(first_user_message, '') <> '' THEN 'user' ELSE thread_source END",
        );
    }
    if columns.preview && columns.first_user_message {
        assignments.push(
            "preview = CASE WHEN COALESCE(preview, '') = '' THEN first_user_message ELSE preview END",
        );
    }
    assignments.join(", ")
}

fn collect_eligible_thread_ids_for_repair(
    connection: &Connection,
    columns: ThreadsTableColumns,
    where_clause: &str,
    target_provider: &str,
    selection: &RepairTargetSelection,
    excluded_thread_ids: &HashSet<String>,
) -> Result<Vec<String>, String> {
    if !columns.id {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT id FROM threads WHERE {where_clause}");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| format!("准备 SQLite 会话筛选失败: {}", error))?;
    let mut ids = Vec::new();
    if columns.model_provider {
        let rows = statement
            .query_map([target_provider], |row| row.get::<_, String>(0))
            .map_err(|error| format!("查询 SQLite 会话筛选失败: {}", error))?;
        for row in rows {
            let thread_id = row.map_err(|error| format!("读取 SQLite 会话筛选失败: {}", error))?;
            if selection.includes_session_id(&thread_id)
                && !excluded_thread_ids.contains(&thread_id)
            {
                ids.push(thread_id);
            }
        }
    } else {
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("查询 SQLite 会话筛选失败: {}", error))?;
        for row in rows {
            let thread_id = row.map_err(|error| format!("读取 SQLite 会话筛选失败: {}", error))?;
            if selection.includes_session_id(&thread_id)
                && !excluded_thread_ids.contains(&thread_id)
            {
                ids.push(thread_id);
            }
        }
    }
    Ok(ids)
}

#[derive(Debug, Clone)]
struct CatalogThreadRecord {
    id: String,
    display_title: String,
    source_created_at: f64,
    source_updated_at: f64,
    cwd: String,
    source_kind: String,
    source_detail: String,
    git_branch: Option<String>,
    thread_source: Option<String>,
    project_id: Option<String>,
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>, String> {
    let escaped = table.replace('"', "\"\"");
    let mut statement = connection
        .prepare(&format!("PRAGMA table_info(\"{escaped}\")"))
        .map_err(|error| format!("读取 SQLite {table} 表结构失败: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("读取 SQLite {table} 表结构失败: {error}"))?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| format!("读取 SQLite {table} 表结构失败: {error}"))?;
    Ok(columns)
}

fn sqlite_text_expr(columns: &HashSet<String>, column: &str, fallback: &str) -> String {
    if columns.contains(column) {
        format!("COALESCE({column}, {fallback})")
    } else {
        fallback.to_string()
    }
}

fn sqlite_time_expr(
    columns: &HashSet<String>,
    seconds_column: &str,
    millis_column: &str,
) -> String {
    if columns.contains(millis_column) {
        format!("COALESCE({millis_column} / 1000.0, 0)")
    } else if columns.contains(seconds_column) {
        format!(
            "CASE WHEN COALESCE({seconds_column}, 0) > 9999999999 THEN {seconds_column} / 1000.0 ELSE COALESCE({seconds_column}, 0) END"
        )
    } else {
        "0".to_string()
    }
}

fn collect_non_root_thread_ids(paths: &[PathBuf]) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::new();
    let mut explicit_user_ids = HashSet::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("打开 SQLite 会话库失败 ({}): {error}", path.display()))?;
        for (table, column) in [
            ("thread_spawn_edges", "child_thread_id"),
            ("agent_job_items", "assigned_thread_id"),
        ] {
            if !table_columns(&connection, table)?.contains(column) {
                continue;
            }
            let sql =
                format!("SELECT DISTINCT {column} FROM {table} WHERE COALESCE({column}, '') <> ''");
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| format!("准备子 Agent 会话查询失败: {error}"))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| format!("查询子 Agent 会话失败: {error}"))?;
            for row in rows {
                ids.insert(row.map_err(|error| format!("读取子 Agent 会话失败: {error}"))?);
            }
        }
        for (table, id_column, source_column) in [
            ("threads", "id", "source"),
            ("local_thread_catalog", "thread_id", "source_kind"),
        ] {
            let columns = table_columns(&connection, table)?;
            if !columns.contains(id_column) {
                continue;
            }
            let source = sqlite_text_expr(&columns, source_column, "''");
            let thread_source = sqlite_text_expr(&columns, "thread_source", "NULL");
            let sql = format!(
                "SELECT {id_column}, {source}, {thread_source} FROM {table} WHERE COALESCE({id_column}, '') <> ''"
            );
            let mut statement = connection
                .prepare(&sql)
                .map_err(|error| format!("准备会话类型查询失败: {error}"))?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1).unwrap_or_default(),
                        row.get::<_, Option<String>>(2).unwrap_or(None),
                    ))
                })
                .map_err(|error| format!("查询会话类型失败: {error}"))?;
            for row in rows {
                let (thread_id, source, thread_source) =
                    row.map_err(|error| format!("读取会话类型失败: {error}"))?;
                if thread_source
                    .as_deref()
                    .is_some_and(|value| value.trim().eq_ignore_ascii_case("user"))
                {
                    explicit_user_ids.insert(thread_id);
                } else if thread_source_marks_non_root(thread_source.as_deref())
                    || source_text_marks_non_root_agent(&source)
                    || serde_json::from_str::<JsonValue>(&source)
                        .is_ok_and(|value| source_value_marks_non_root_agent(&value))
                {
                    ids.insert(thread_id);
                }
            }
        }
    }
    ids.retain(|thread_id| !explicit_user_ids.contains(thread_id));
    Ok(ids)
}

fn thread_source_marks_non_root(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| {
        value.eq_ignore_ascii_case("subagent") || value.eq_ignore_ascii_case("memory_consolidation")
    })
}

fn collect_catalog_thread_records(
    paths: &[PathBuf],
    selection: &RepairTargetSelection,
    rollout_facts: &RolloutThreadFacts,
) -> Result<(HashMap<String, CatalogThreadRecord>, HashSet<String>), String> {
    let mut non_root_ids = collect_non_root_thread_ids(paths)?;
    non_root_ids.extend(rollout_facts.subagent_thread_ids.iter().cloned());
    let mut records = HashMap::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("打开 SQLite 会话库失败 ({}): {error}", path.display()))?;
        let columns = table_columns(&connection, "threads")?;
        if !columns.contains("id") {
            continue;
        }
        let title = if columns.contains("title") && columns.contains("first_user_message") {
            "COALESCE(NULLIF(title, ''), NULLIF(first_user_message, ''), '')".to_string()
        } else if columns.contains("title") {
            "COALESCE(title, '')".to_string()
        } else if columns.contains("first_user_message") {
            "COALESCE(first_user_message, '')".to_string()
        } else {
            "''".to_string()
        };
        let created = sqlite_time_expr(&columns, "created_at", "created_at_ms");
        let updated = sqlite_time_expr(&columns, "updated_at", "updated_at_ms");
        let cwd = sqlite_text_expr(&columns, "cwd", "''");
        let source = sqlite_text_expr(&columns, "source", "''");
        let git_branch = sqlite_text_expr(&columns, "git_branch", "NULL");
        let thread_source = sqlite_text_expr(&columns, "thread_source", "NULL");
        let project_id = sqlite_text_expr(&columns, "project_id", "NULL");
        let sql = format!(
            "SELECT id, {title}, {created}, {updated}, {cwd}, {source}, {git_branch}, {thread_source}, {project_id} FROM threads WHERE COALESCE(id, '') <> ''"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| format!("准备会话目录修复查询失败 ({}): {error}", path.display()))?;
        let rows = statement
            .query_map([], |row| {
                Ok(CatalogThreadRecord {
                    id: row.get(0)?,
                    display_title: row.get::<_, String>(1).unwrap_or_default(),
                    source_created_at: row.get::<_, f64>(2).unwrap_or_default(),
                    source_updated_at: row.get::<_, f64>(3).unwrap_or_default(),
                    cwd: row.get::<_, String>(4).unwrap_or_default(),
                    source_kind: row.get::<_, String>(5).unwrap_or_default(),
                    source_detail: row.get::<_, String>(5).unwrap_or_default(),
                    git_branch: row.get::<_, Option<String>>(6).unwrap_or(None),
                    thread_source: row.get::<_, Option<String>>(7).unwrap_or(None),
                    project_id: row.get::<_, Option<String>>(8).unwrap_or(None),
                })
            })
            .map_err(|error| format!("查询会话目录修复数据失败 ({}): {error}", path.display()))?;
        for row in rows {
            let record = row.map_err(|error| {
                format!("读取会话目录修复数据失败 ({}): {error}", path.display())
            })?;
            if !selection.includes_session_id(&record.id) {
                continue;
            }
            let explicitly_user = record
                .thread_source
                .as_deref()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("user"));
            if explicitly_user {
                non_root_ids.remove(&record.id);
            }
            if !explicitly_user
                && (non_root_ids.contains(&record.id)
                    || thread_source_marks_non_root(record.thread_source.as_deref())
                    || source_text_marks_non_root_agent(&record.source_kind)
                    || serde_json::from_str::<JsonValue>(&record.source_kind)
                        .is_ok_and(|value| source_value_marks_non_root_agent(&value)))
            {
                non_root_ids.insert(record.id.clone());
                continue;
            }
            let replace = records
                .get(&record.id)
                .map(|current: &CatalogThreadRecord| {
                    record.source_updated_at > current.source_updated_at
                })
                .unwrap_or(true);
            if replace {
                records.insert(record.id.clone(), record);
            }
        }
    }
    for id in &non_root_ids {
        records.remove(id);
    }
    Ok((records, non_root_ids))
}

fn local_catalog_host_id(connection: &Connection) -> Result<Option<String>, String> {
    let columns = table_columns(connection, "local_thread_catalog_hosts")?;
    if !columns.contains("host_id") {
        return Ok(Some("local".to_string()));
    }
    let query = if columns.contains("host_kind") {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE LOWER(COALESCE(host_kind, '')) = 'local' ORDER BY host_id LIMIT 1"
    } else {
        "SELECT host_id FROM local_thread_catalog_hosts WHERE host_id = 'local' LIMIT 1"
    };
    match connection.query_row(query, [], |row| row.get::<_, String>(0)) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
        Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(format!("读取本地会话目录 host 失败: {error}")),
    }
}

fn repair_local_thread_catalog_for_options(
    data_dir: &Path,
    target_provider: &str,
    selection: &RepairTargetSelection,
    rollout_facts: &RolloutThreadFacts,
    dry_run: bool,
) -> Result<CatalogRepairCounts, String> {
    let paths = provider_sync_sqlite_paths(data_dir);
    let (records, non_root_ids) = collect_catalog_thread_records(&paths, selection, rollout_facts)?;
    let mut totals = CatalogRepairCounts::default();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut connection = Connection::open(&path)
            .map_err(|error| format!("打开会话目录数据库失败 ({}): {error}", path.display()))?;
        let columns = table_columns(&connection, "local_thread_catalog")?;
        let required = [
            "host_id",
            "thread_id",
            "display_title",
            "source_created_at",
            "source_updated_at",
            "cwd",
            "source_kind",
            "model_provider",
            "observation_sequence",
        ];
        if !required.iter().all(|column| columns.contains(*column)) {
            continue;
        }
        let Some(host_id) = local_catalog_host_id(&connection)? else {
            continue;
        };
        let metadata_columns = table_columns(&connection, "local_thread_catalog_metadata")?;
        let sync_state_columns = table_columns(&connection, "local_thread_catalog_sync_state")?;
        let mut observation_sequence = connection
            .query_row(
                "SELECT COALESCE(MAX(observation_sequence), 0) FROM local_thread_catalog WHERE host_id = ?1",
                [host_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0);
        let transaction = connection
            .transaction()
            .map_err(|error| format!("启动会话目录修复事务失败 ({}): {error}", path.display()))?;
        let counts_before = totals;

        for thread_id in &non_root_ids {
            if !selection.includes_session_id(thread_id) {
                continue;
            }
            totals.removed_rows += transaction
                .execute(
                    "DELETE FROM local_thread_catalog WHERE host_id = ?1 AND thread_id = ?2",
                    (host_id.as_str(), thread_id.as_str()),
                )
                .map_err(|error| {
                    format!("清理子 Agent 会话目录失败 ({}): {error}", path.display())
                })?;
        }

        for record in records.values() {
            let mut assignments = vec!["model_provider = ?1"];
            if columns.contains("missing_candidate") {
                assignments.push("missing_candidate = 0");
            }
            let update_sql = format!(
                "UPDATE local_thread_catalog SET {} WHERE host_id = ?2 AND thread_id = ?3 AND (COALESCE(model_provider, '') <> ?1{})",
                assignments.join(", "),
                if columns.contains("missing_candidate") {
                    " OR COALESCE(missing_candidate, 0) <> 0"
                } else {
                    ""
                }
            );
            let updated = transaction
                .execute(
                    &update_sql,
                    (target_provider, host_id.as_str(), record.id.as_str()),
                )
                .map_err(|error| format!("更新会话目录失败 ({}): {error}", path.display()))?;
            totals.updated_rows += updated;
            if updated > 0 {
                continue;
            }
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM local_thread_catalog WHERE host_id = ?1 AND thread_id = ?2 LIMIT 1",
                    (host_id.as_str(), record.id.as_str()),
                    |_| Ok(()),
                )
                .is_ok();
            if exists {
                continue;
            }
            observation_sequence += 1;
            let mut insert_columns = vec![
                "host_id",
                "thread_id",
                "display_title",
                "source_created_at",
                "source_updated_at",
                "cwd",
                "source_kind",
                "model_provider",
                "observation_sequence",
            ];
            let mut values = vec![
                SqlValue::Text(host_id.clone()),
                SqlValue::Text(record.id.clone()),
                SqlValue::Text(record.display_title.clone()),
                SqlValue::Real(record.source_created_at),
                SqlValue::Real(record.source_updated_at),
                SqlValue::Text(record.cwd.clone()),
                SqlValue::Text(record.source_kind.clone()),
                SqlValue::Text(target_provider.to_string()),
                SqlValue::Integer(observation_sequence),
            ];
            for (column, value) in [
                (
                    "source_detail",
                    SqlValue::Text(record.source_detail.clone()),
                ),
                (
                    "git_branch",
                    record
                        .git_branch
                        .clone()
                        .map(SqlValue::Text)
                        .unwrap_or(SqlValue::Null),
                ),
                (
                    "thread_source",
                    record
                        .thread_source
                        .clone()
                        .map(SqlValue::Text)
                        .unwrap_or(SqlValue::Null),
                ),
                (
                    "project_id",
                    record
                        .project_id
                        .clone()
                        .map(SqlValue::Text)
                        .unwrap_or(SqlValue::Null),
                ),
                ("missing_candidate", SqlValue::Integer(0)),
                (
                    "source_recency_at",
                    SqlValue::Real(record.source_updated_at),
                ),
                ("pending_observed_title", SqlValue::Integer(0)),
            ] {
                if columns.contains(column) {
                    insert_columns.push(column);
                    values.push(value);
                }
            }
            let placeholders = std::iter::repeat_n("?", insert_columns.len())
                .collect::<Vec<_>>()
                .join(", ");
            let insert_sql = format!(
                "INSERT OR IGNORE INTO local_thread_catalog ({}) VALUES ({})",
                insert_columns.join(", "),
                placeholders
            );
            totals.inserted_rows += transaction
                .execute(&insert_sql, params_from_iter(values))
                .map_err(|error| format!("补写会话目录失败 ({}): {error}", path.display()))?;
        }

        let changed_rows = totals.total().saturating_sub(counts_before.total());
        if changed_rows > 0 && metadata_columns.contains("catalog_revision") {
            let updated = transaction
                .execute(
                    "UPDATE local_thread_catalog_metadata SET catalog_revision = catalog_revision + ?1",
                    [changed_rows as i64],
                )
                .map_err(|error| {
                    format!("更新会话目录版本失败 ({}): {error}", path.display())
                })?;
            if updated == 0 && metadata_columns.contains("id") {
                transaction
                    .execute(
                        "INSERT INTO local_thread_catalog_metadata (id, catalog_revision) VALUES (1, ?1)",
                        [changed_rows as i64],
                    )
                    .map_err(|error| {
                        format!("初始化会话目录版本失败 ({}): {error}", path.display())
                    })?;
            }
        }
        if changed_rows > 0 && sync_state_columns.contains("host_id") {
            let mut assignments = Vec::new();
            let mut values = Vec::new();
            if sync_state_columns.contains("initial_build_complete") {
                assignments.push("initial_build_complete = 1");
            }
            if sync_state_columns.contains("observation_sequence") {
                assignments
                    .push("observation_sequence = MAX(COALESCE(observation_sequence, 0), ?)");
                values.push(SqlValue::Integer(observation_sequence));
            }
            if sync_state_columns.contains("watermark_updated_at") {
                assignments
                    .push("watermark_updated_at = MAX(COALESCE(watermark_updated_at, 0), ?)");
                values.push(SqlValue::Real(
                    records
                        .values()
                        .map(|record| record.source_updated_at)
                        .fold(0.0, f64::max),
                ));
            }
            if sync_state_columns.contains("last_full_reconciled_at") {
                assignments
                    .push("last_full_reconciled_at = MAX(COALESCE(last_full_reconciled_at, 0), ?)");
                values.push(SqlValue::Integer(Utc::now().timestamp()));
            }
            if !assignments.is_empty() {
                let sql = format!(
                    "UPDATE local_thread_catalog_sync_state SET {} WHERE host_id = ?",
                    assignments.join(", ")
                );
                values.push(SqlValue::Text(host_id.clone()));
                let updated = transaction
                    .execute(&sql, params_from_iter(values))
                    .map_err(|error| {
                        format!("更新会话目录同步状态失败 ({}): {error}", path.display())
                    })?;
                if updated == 0 {
                    let mut columns = vec!["host_id"];
                    let mut values = vec![SqlValue::Text(host_id.clone())];
                    if sync_state_columns.contains("watermark_updated_at") {
                        columns.push("watermark_updated_at");
                        values.push(SqlValue::Real(
                            records
                                .values()
                                .map(|record| record.source_updated_at)
                                .fold(0.0, f64::max),
                        ));
                    }
                    if sync_state_columns.contains("initial_build_complete") {
                        columns.push("initial_build_complete");
                        values.push(SqlValue::Integer(1));
                    }
                    if sync_state_columns.contains("observation_sequence") {
                        columns.push("observation_sequence");
                        values.push(SqlValue::Integer(observation_sequence));
                    }
                    if sync_state_columns.contains("last_full_reconciled_at") {
                        columns.push("last_full_reconciled_at");
                        values.push(SqlValue::Integer(Utc::now().timestamp()));
                    }
                    let placeholders = std::iter::repeat_n("?", columns.len())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let insert_sql = format!(
                        "INSERT INTO local_thread_catalog_sync_state ({}) VALUES ({})",
                        columns.join(", "),
                        placeholders
                    );
                    transaction
                        .execute(&insert_sql, params_from_iter(values))
                        .map_err(|error| {
                            format!("初始化会话目录同步状态失败 ({}): {error}", path.display())
                        })?;
                }
            }
        }

        if !dry_run {
            transaction
                .commit()
                .map_err(|error| format!("提交会话目录修复失败 ({}): {error}", path.display()))?;
        }
    }
    Ok(totals)
}

fn format_sqlite_read_error(path: &Path, action: &str, error: &rusqlite::Error) -> String {
    format!("{} ({}): {}", action, path.display(), error)
}

