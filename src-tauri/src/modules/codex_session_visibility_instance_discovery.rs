// Codex Session Visibility：Instance/provider discovery and rollout metadata collection。
// 通过 include! 保持原模块作用域、私有调用关系和修复事务行为。
fn collect_instances() -> Result<Vec<CodexSyncInstance>, String> {
    let mut instances = Vec::new();
    let default_dir = modules::codex_instance::get_default_codex_home()?;
    let store = modules::codex_instance::load_instance_store()?;
    instances.push(CodexSyncInstance {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: DEFAULT_INSTANCE_NAME.to_string(),
        data_dir: default_dir,
        last_pid: store.default_settings.last_pid,
    });

    for instance in store.instances {
        let user_data_dir = instance.user_data_dir.trim();
        if user_data_dir.is_empty() {
            continue;
        }
        instances.push(CodexSyncInstance {
            id: instance.id,
            name: instance.name,
            data_dir: PathBuf::from(user_data_dir),
            last_pid: instance.last_pid,
        });
    }

    Ok(instances)
}

fn is_instance_running(
    instance: &CodexSyncInstance,
    process_entries: &[(u32, Option<String>)],
) -> bool {
    let codex_home = if instance.id == DEFAULT_INSTANCE_ID {
        None
    } else {
        instance.data_dir.to_str()
    };
    modules::process::resolve_codex_pid_from_entries(instance.last_pid, codex_home, process_entries)
        .is_some()
}

fn try_rebuild_thread_metadata(instance: &CodexSyncInstance) -> bool {
    let started = std::time::Instant::now();
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] rebuild official metadata started: instance_id={}, instance_name={}, data_dir={}",
        instance.id,
        instance.name,
        instance.data_dir.display()
    ));
    match modules::codex_official_app_server::rebuild_thread_metadata(&instance.data_dir) {
        Ok(()) => {
            modules::logger::log_info(&format!(
                "[Codex Session Visibility] rebuild official metadata finished: instance_id={}, elapsed_ms={}",
                instance.id,
                started.elapsed().as_millis()
            ));
            true
        }
        Err(error) => {
            modules::logger::log_warn(&format!(
                "Codex 会话索引修复后触发官方侧边栏索引重建失败 ({} / {}): {}; elapsed_ms={}",
                instance.name,
                instance.data_dir.display(),
                error,
                started.elapsed().as_millis()
            ));
            false
        }
    }
}

fn read_target_provider(data_dir: &Path) -> Result<String, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(DEFAULT_PROVIDER_ID.to_string());
    }

    let doc = modules::codex_config_format::read_codex_config_doc_from_str(&content).map_err(
        |error| {
            format!(
                "解析 config.toml 失败 ({}): {}",
                config_path.display(),
                error
            )
        },
    )?;
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PROVIDER_ID);
    Ok(provider.to_string())
}

fn validate_provider_id(provider_id: &str) -> Result<(), String> {
    let trimmed = provider_id.trim();
    if trimmed.is_empty() {
        return Err("provider 不能为空".to_string());
    }
    if trimmed.len() > 200 || trimmed.chars().any(char::is_control) {
        return Err("provider 包含非法字符".to_string());
    }
    Ok(())
}

fn is_valid_provider_id_for_discovery(provider_id: &str) -> bool {
    validate_provider_id(provider_id).is_ok()
}

fn add_provider_source(
    sources: &mut HashMap<String, HashSet<CodexSessionVisibilityRepairProviderSource>>,
    provider_id: String,
    source: CodexSessionVisibilityRepairProviderSource,
) {
    let provider_id = provider_id.trim().to_string();
    if !is_valid_provider_id_for_discovery(&provider_id) {
        return;
    }
    sources.entry(provider_id).or_default().insert(source);
}

fn list_configured_provider_ids(data_dir: &Path) -> Result<Vec<String>, String> {
    let config_path = data_dir.join(CONFIG_FILE_NAME);
    if !config_path.exists() {
        return Ok(vec![DEFAULT_PROVIDER_ID.to_string()]);
    }

    let content = fs::read_to_string(&config_path).map_err(|error| {
        format!(
            "读取 config.toml 失败 ({}): {}",
            config_path.display(),
            error
        )
    })?;
    if content.trim().is_empty() {
        return Ok(vec![DEFAULT_PROVIDER_ID.to_string()]);
    }

    let doc = modules::codex_config_format::read_codex_config_doc_from_str(&content).map_err(
        |error| {
            format!(
                "解析 config.toml 失败 ({}): {}",
                config_path.display(),
                error
            )
        },
    )?;
    let mut ids = HashSet::new();
    if let Some(provider) = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        ids.insert(provider.to_string());
    }
    if let Some(model_providers) = doc.get("model_providers").and_then(|item| item.as_table()) {
        for (provider_id, _) in model_providers.iter() {
            let provider_id = provider_id.trim();
            if !provider_id.is_empty() {
                ids.insert(provider_id.to_string());
            }
        }
    }
    if ids.is_empty() {
        ids.insert(DEFAULT_PROVIDER_ID.to_string());
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn sqlite_provider_ids(db_path: &Path) -> Result<Vec<String>, String> {
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
    let mut ids = HashSet::new();
    for table in ["threads", "local_thread_catalog"] {
        let columns = table_columns(&connection, table)?;
        if !columns.contains("model_provider") {
            continue;
        }
        let sql = format!(
            "SELECT DISTINCT model_provider FROM {table} WHERE COALESCE(model_provider, '') <> ''"
        );
        let mut statement = connection.prepare(&sql).map_err(|error| {
            format_sqlite_read_error(db_path, "准备 SQLite provider 查询失败", &error)
        })?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| {
                format_sqlite_read_error(db_path, "查询 SQLite provider 失败", &error)
            })?;
        for row in rows {
            let provider_id = row.map_err(|error| {
                format_sqlite_read_error(db_path, "读取 SQLite provider 失败", &error)
            })?;
            if is_valid_provider_id_for_discovery(&provider_id) {
                ids.insert(provider_id);
            }
        }
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn collect_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<Vec<RolloutProviderChange>, String> {
    let non_root_thread_ids = collect_non_root_thread_ids(&provider_sync_sqlite_paths(data_dir))?;
    let session_index_map = match read_session_index_map(data_dir) {
        Ok(value) => value,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "读取 Codex session_index.jsonl 失败，跳过该时间来源并继续修复会话可见性: {}",
                error
            ));
            HashMap::new()
        }
    };
    let mut changes = Vec::new();

    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        let rollout_paths = list_rollout_files(&root_dir)?;
        for rollout_path in rollout_paths {
            let rewrite = if options.rewrite_all_session_meta {
                let Some(content) = read_rollout_text(&rollout_path)? else {
                    continue;
                };
                rewrite_rollout_session_meta_providers(&content, target_provider)?
            } else {
                rewrite_rollout_first_session_meta_provider(&rollout_path, target_provider)?
            };
            if rewrite.session_meta_count == 0 {
                continue;
            }
            if rewrite.non_root_agent {
                continue;
            }
            if rewrite
                .thread_id
                .as_ref()
                .is_some_and(|thread_id| non_root_thread_ids.contains(thread_id))
            {
                continue;
            }
            let session_id = rewrite.thread_id.clone();
            if let Some(session_id) = session_id.as_deref() {
                if !selection.includes_session_id(session_id) {
                    continue;
                }
            } else if selection.has_session_filter() {
                continue;
            }
            let fallback_modified_ms =
                modules::codex_session_file_time::read_modified_time(&rollout_path)
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as i128);
            let target_modified_at = resolve_target_modified_at_ms(
                session_id.as_deref(),
                &session_index_map,
                &rollout_path,
                fallback_modified_ms,
            )
            .and_then(modules::codex_session_file_time::system_time_from_unix_millis);
            let current_modified_at =
                modules::codex_session_file_time::read_modified_time(&rollout_path);
            let source_size = fs::metadata(&rollout_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let provider_matches = !rewrite.rewrite_needed;
            let modified_time_matches = target_modified_at.is_none()
                || modules::codex_session_file_time::same_modified_time_millis(
                    current_modified_at,
                    target_modified_at,
                );
            if provider_matches && modified_time_matches {
                continue;
            }

            let relative_path = rollout_path
                .strip_prefix(data_dir)
                .map_err(|_| format!("无法计算 rollout 相对路径: {}", rollout_path.display()))?;
            changes.push(RolloutProviderChange {
                relative_path: relative_path.to_path_buf(),
                absolute_path: rollout_path,
                updated_content: rewrite.updated_content,
                target_modified_at,
                source_modified_at: current_modified_at,
                source_size,
            });
        }
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn collect_referenced_rollout_provider_changes(
    data_dir: &Path,
    target_provider: &str,
    options: CodexSessionVisibilityRepairOptions,
    selection: &RepairTargetSelection,
) -> Result<Vec<RolloutProviderChange>, String> {
    let mut candidates: HashMap<PathBuf, Option<SystemTime>> = HashMap::new();
    for db_path in sqlite_candidate_paths_for_options(data_dir, options) {
        collect_referenced_rollout_paths_for_db(
            data_dir,
            &db_path,
            target_provider,
            options.sidebar_visible_only,
            selection,
            &mut candidates,
        )?;
    }

    let mut changes = Vec::new();
    for (rollout_path, target_modified_at) in candidates {
        if !rollout_path.exists() || !is_plain_rollout_file(&rollout_path) {
            continue;
        }
        let rewrite = if options.rewrite_all_session_meta {
            let Some(content) = read_rollout_text(&rollout_path)? else {
                continue;
            };
            rewrite_rollout_session_meta_providers(&content, target_provider)?
        } else {
            rewrite_rollout_first_session_meta_provider(&rollout_path, target_provider)?
        };
        if rewrite.session_meta_count == 0 || !rewrite.rewrite_needed {
            continue;
        }
        if rewrite.non_root_agent {
            continue;
        }
        let Some(relative_path) = rollout_path
            .strip_prefix(data_dir)
            .ok()
            .map(Path::to_path_buf)
        else {
            modules::logger::log_warn(&format!(
                "跳过 Codex 会话可见性修复中的实例外 rollout: data_dir={}, rollout={}",
                data_dir.display(),
                rollout_path.display()
            ));
            continue;
        };
        let source_modified_at =
            modules::codex_session_file_time::read_modified_time(&rollout_path);
        let source_size = fs::metadata(&rollout_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        changes.push(RolloutProviderChange {
            relative_path,
            absolute_path: rollout_path,
            updated_content: rewrite.updated_content,
            target_modified_at,
            source_modified_at,
            source_size,
        });
    }

    changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(changes)
}

fn collect_referenced_rollout_paths_for_db(
    data_dir: &Path,
    db_path: &Path,
    target_provider: &str,
    sidebar_visible_only: bool,
    selection: &RepairTargetSelection,
    candidates: &mut HashMap<PathBuf, Option<SystemTime>>,
) -> Result<(), String> {
    if !db_path.exists() {
        return Ok(());
    }
    let connection = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(connection) => connection,
        Err(error) if modules::db::is_unusable_sqlite_database_error(&error) => {
            log_skipped_sqlite_database(db_path, &error.to_string());
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "打开实例数据库失败 ({}): {}",
                db_path.display(),
                error
            ));
        }
    };

    let mut table_info = connection
        .prepare("PRAGMA table_info(threads)")
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    let names = table_info
        .query_map([], |row| row.get::<usize, String>(1))
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite threads 表结构失败", &error)
        })?;
    drop(table_info);

    if !names.contains("id") || !names.contains("rollout_path") {
        return Ok(());
    }
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
    let mut predicates = vec!["rollout_path IS NOT NULL", "rollout_path <> ''"];
    let provider_param = if sidebar_visible_only && names.contains("model_provider") {
        predicates.push("(?1 IS NULL OR COALESCE(model_provider, '') <> ?1)");
        Some(target_provider)
    } else {
        predicates.push("?1 IS NULL");
        None
    };
    if sidebar_visible_only {
        if names.contains("archived") {
            predicates.push("COALESCE(archived, 0) = 0");
        }
        if names.contains("preview") && names.contains("first_user_message") {
            predicates
                .push("(COALESCE(preview, '') <> '' OR COALESCE(first_user_message, '') <> '')");
        } else if names.contains("preview") {
            predicates.push("COALESCE(preview, '') <> ''");
        }
        if names.contains("source") {
            predicates.push(
                "LOWER(COALESCE(source, '')) NOT LIKE '%subagent%' AND LOWER(COALESCE(source, '')) NOT LIKE '%internal%'",
            );
        }
        if names.contains("thread_source") {
            predicates.push("COALESCE(thread_source, '') <> 'ambient_suggestions'");
        }
    }
    let sql = format!(
        "SELECT id, rollout_path, {updated_at_expr}, {updated_at_ms_expr} FROM threads WHERE {}",
        predicates.join(" AND ")
    );
    let mut statement = connection.prepare(sql.as_str()).map_err(|error| {
        format_sqlite_read_error(db_path, "准备 SQLite rollout 引用查询失败", &error)
    })?;
    let rows = statement
        .query_map([provider_param], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| {
            format_sqlite_read_error(db_path, "查询 SQLite rollout 引用失败", &error)
        })?;

    for row in rows {
        let (thread_id, rollout_path, updated_at, updated_at_ms) = row.map_err(|error| {
            format_sqlite_read_error(db_path, "读取 SQLite rollout 引用失败", &error)
        })?;
        if !selection.includes_session_id(&thread_id) {
            continue;
        }
        let rollout_path = resolve_rollout_path(data_dir, &rollout_path);
        let target_modified_at = updated_at_ms
            .or_else(|| updated_at.map(|value| value * 1000))
            .and_then(|value| {
                modules::codex_session_file_time::system_time_from_unix_millis(value as i128)
            });
        candidates
            .entry(rollout_path)
            .and_modify(|existing| {
                if existing.is_none() {
                    *existing = target_modified_at;
                }
            })
            .or_insert(target_modified_at);
    }
    Ok(())
}

#[derive(Debug, Default)]
struct RolloutProviderRewrite {
    updated_content: Option<RolloutProviderUpdate>,
    rewrite_needed: bool,
    thread_id: Option<String>,
    session_meta_count: usize,
    non_root_agent: bool,
    providers: HashSet<String>,
}

fn rewrite_rollout_session_meta_providers(
    content: &str,
    target_provider: &str,
) -> Result<RolloutProviderRewrite, String> {
    let mut rewrite = RolloutProviderRewrite::default();
    let mut next_content = String::new();
    for segment in content.split_inclusive('\n') {
        let (line, line_ending) = split_line_ending(segment);
        let mut next_line = line.to_string();
        if !line.trim().is_empty() {
            if let Ok(mut record) = serde_json::from_str::<JsonValue>(line) {
                if record.get("type").and_then(JsonValue::as_str) == Some("session_meta") {
                    let Some(payload) =
                        record.get_mut("payload").and_then(JsonValue::as_object_mut)
                    else {
                        next_content.push_str(&next_line);
                        next_content.push_str(line_ending);
                        continue;
                    };
                    rewrite.session_meta_count += 1;
                    rewrite.non_root_agent |= payload
                        .get("source")
                        .is_some_and(source_value_marks_non_root_agent);
                    if let Some(provider) = payload
                        .get("model_provider")
                        .and_then(JsonValue::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        rewrite.providers.insert(provider.to_string());
                    }
                    if rewrite.thread_id.is_none() {
                        rewrite.thread_id = payload
                            .get("id")
                            .or_else(|| payload.get("session_id"))
                            .and_then(JsonValue::as_str)
                            .map(str::to_string);
                    }
                    if payload.get("model_provider").and_then(JsonValue::as_str)
                        != Some(target_provider)
                    {
                        payload.insert(
                            "model_provider".to_string(),
                            JsonValue::String(target_provider.to_string()),
                        );
                        next_line = serde_json::to_string(&record)
                            .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
                        rewrite.rewrite_needed = true;
                    }
                }
            }
        }
        next_content.push_str(&next_line);
        next_content.push_str(line_ending);
    }
    if !content.ends_with('\n') && next_content.ends_with('\n') {
        next_content.pop();
    }
    if rewrite.rewrite_needed {
        rewrite.updated_content = Some(RolloutProviderUpdate::FullContent(next_content));
    }
    Ok(rewrite)
}

fn rewrite_rollout_first_session_meta_provider(
    path: &Path,
    target_provider: &str,
) -> Result<RolloutProviderRewrite, String> {
    let Some((first_line, _separator)) = read_first_line(path)? else {
        return Ok(RolloutProviderRewrite::default());
    };
    let Some(mut record) = parse_session_meta_record(&first_line) else {
        return Ok(RolloutProviderRewrite::default());
    };
    let thread_id = session_meta_id(&record);
    let current_provider = record
        .get("payload")
        .and_then(|payload| payload.get("model_provider"))
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let non_root_agent = record
        .get("payload")
        .and_then(|payload| payload.get("source"))
        .is_some_and(source_value_marks_non_root_agent);
    let providers = (!current_provider.is_empty())
        .then(|| current_provider.to_string())
        .into_iter()
        .collect();
    if current_provider == target_provider {
        return Ok(RolloutProviderRewrite {
            updated_content: None,
            rewrite_needed: false,
            thread_id,
            session_meta_count: 1,
            non_root_agent,
            providers,
        });
    }

    let Some(payload) = record.get_mut("payload").and_then(JsonValue::as_object_mut) else {
        return Ok(RolloutProviderRewrite::default());
    };
    payload.insert(
        "model_provider".to_string(),
        JsonValue::String(target_provider.to_string()),
    );
    let updated_first_line = serde_json::to_string(&record)
        .map_err(|error| format!("序列化 session_meta 失败: {}", error))?;
    Ok(RolloutProviderRewrite {
        updated_content: Some(RolloutProviderUpdate::FirstLine(updated_first_line)),
        rewrite_needed: true,
        thread_id,
        session_meta_count: 1,
        non_root_agent,
        providers,
    })
}

fn source_value_marks_non_root_agent(source: &JsonValue) -> bool {
    match source {
        JsonValue::Object(object) => {
            object.contains_key("sub_agent")
                || object.contains_key("subagent")
                || object.contains_key("internal")
        }
        JsonValue::String(value) => source_text_marks_non_root_agent(value),
        _ => false,
    }
}

fn source_text_marks_non_root_agent(source: &str) -> bool {
    let source = source.trim().to_ascii_lowercase();
    source == "subagent"
        || source == "internal"
        || source.starts_with("subagent_")
        || source.starts_with("internal_")
}

fn read_first_line(path: &Path) -> Result<Option<(String, String)>, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("打开 rollout 文件失败 ({}): {}", path.display(), error))?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let bytes_read = reader
        .read_until(b'\n', &mut buffer)
        .map_err(|error| format!("读取 rollout 首行失败 ({}): {}", path.display(), error))?;
    if bytes_read == 0 {
        return Ok(None);
    }

    let (line_bytes, separator) = if buffer.ends_with(b"\r\n") {
        (&buffer[..buffer.len() - 2], "\r\n")
    } else if buffer.ends_with(b"\n") {
        (&buffer[..buffer.len() - 1], "\n")
    } else {
        (&buffer[..], "")
    };

    let line = match String::from_utf8(line_bytes.to_vec()) {
        Ok(line) => line,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "跳过非 UTF-8 Codex rollout 文件 ({}): {}",
                path.display(),
                error
            ));
            return Ok(None);
        }
    };
    Ok(Some((line, separator.to_string())))
}

fn read_rollout_text(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            modules::logger::log_warn(&format!(
                "跳过非 UTF-8 Codex rollout 文件 ({}): {}",
                path.display(),
                error
            ));
            Ok(None)
        }
        Err(error) => Err(format!(
            "读取 rollout 文件失败 ({}): {}",
            path.display(),
            error
        )),
    }
}

fn parse_session_meta_record(first_line: &str) -> Option<JsonValue> {
    if first_line.trim().is_empty() {
        return None;
    }

    let parsed = serde_json::from_str::<JsonValue>(first_line).ok()?;
    if parsed.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return None;
    }
    if !parsed.get("payload").is_some_and(JsonValue::is_object) {
        return None;
    }
    Some(parsed)
}

fn session_meta_id(meta: &JsonValue) -> Option<String> {
    meta.get("payload")
        .and_then(|payload| payload.get("id").or_else(|| payload.get("session_id")))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .or_else(|| {
            meta.get("id")
                .or_else(|| meta.get("session_id"))
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
}

fn split_line_ending(segment: &str) -> (&str, &str) {
    if let Some(line) = segment.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = segment.strip_suffix('\n') {
        (line, "\n")
    } else {
        (segment, "")
    }
}

fn collect_rollout_thread_facts(
    data_dir: &Path,
    selection: &RepairTargetSelection,
) -> Result<RolloutThreadFacts, String> {
    let mut facts = RolloutThreadFacts::default();
    let projectless_thread_ids = load_projectless_thread_ids(data_dir)?;
    for dir_name in SESSION_DIRS {
        let root_dir = data_dir.join(dir_name);
        if !root_dir.exists() {
            continue;
        }
        for rollout_path in list_rollout_files(&root_dir)? {
            let Some(content) = read_rollout_text(&rollout_path)? else {
                continue;
            };
            let has_user_event =
                content.contains("\"user_message\"") || content.contains("\"user_input\"");
            let contains_encrypted_content = content.contains("encrypted_content");
            let mut file_providers = HashSet::new();
            for line in content.lines() {
                let Ok(record) = serde_json::from_str::<JsonValue>(line.trim()) else {
                    continue;
                };
                if record.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
                    continue;
                }
                let Some(payload) = record.get("payload").and_then(JsonValue::as_object) else {
                    continue;
                };
                let Some(thread_id) = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
                else {
                    continue;
                };
                if !selection.includes_session_id(&thread_id) {
                    continue;
                }
                if payload
                    .get("source")
                    .is_some_and(source_value_marks_non_root_agent)
                {
                    facts.subagent_thread_ids.insert(thread_id.clone());
                    continue;
                }
                if let Some(provider) = payload
                    .get("model_provider")
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    file_providers.insert(provider.to_string());
                }
                if has_user_event {
                    facts.user_event_thread_ids.insert(thread_id.clone());
                }
                if !projectless_thread_ids.contains(&thread_id) {
                    if let Some(cwd) = payload
                        .get("cwd")
                        .and_then(JsonValue::as_str)
                        .and_then(to_desktop_workspace_path)
                    {
                        facts.cwd_by_thread_id.entry(thread_id).or_insert(cwd);
                    }
                }
            }
            if contains_encrypted_content {
                for provider in file_providers {
                    *facts.encrypted_content_counts.entry(provider).or_insert(0) += 1;
                }
            }
        }
    }
    facts
        .subagent_thread_ids
        .extend(collect_non_root_thread_ids(&provider_sync_sqlite_paths(
            data_dir,
        ))?);
    for thread_id in &facts.subagent_thread_ids {
        facts.user_event_thread_ids.remove(thread_id);
        facts.cwd_by_thread_id.remove(thread_id);
    }
    Ok(facts)
}

fn read_global_state_object(data_dir: &Path) -> Result<serde_json::Map<String, JsonValue>, String> {
    let path = data_dir.join(GLOBAL_STATE_FILE);
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Codex 全局状态失败 ({}): {error}", path.display()))?;
    let value = serde_json::from_str::<JsonValue>(&content)
        .map_err(|error| format!("解析 Codex 全局状态失败 ({}): {error}", path.display()))?;
    Ok(value.as_object().cloned().unwrap_or_default())
}

fn load_projectless_thread_ids(data_dir: &Path) -> Result<HashSet<String>, String> {
    let state = read_global_state_object(data_dir)?;
    Ok(state
        .get("projectless-thread-ids")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn normalized_workspace_paths(value: &JsonValue) -> Vec<String> {
    let values = if let Some(values) = value.as_array() {
        values
            .iter()
            .filter_map(JsonValue::as_str)
            .collect::<Vec<_>>()
    } else {
        value.as_str().into_iter().collect::<Vec<_>>()
    };
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for value in values {
        let Some(normalized) = to_desktop_workspace_path(value) else {
            continue;
        };
        let comparable = normalized
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase();
        if seen.insert(comparable) {
            result.push(normalized);
        }
    }
    result
}

fn normalized_global_state_entries(
    state: &serde_json::Map<String, JsonValue>,
) -> serde_json::Map<String, JsonValue> {
    let mut normalized = serde_json::Map::new();
    for key in [
        "electron-saved-workspace-roots",
        "project-order",
        "active-workspace-roots",
    ] {
        let Some(value) = state.get(key) else {
            continue;
        };
        let paths = normalized_workspace_paths(value);
        let next = if key == "active-workspace-roots" && !value.is_array() {
            paths
                .first()
                .cloned()
                .map(JsonValue::String)
                .unwrap_or_else(|| value.clone())
        } else {
            JsonValue::Array(paths.into_iter().map(JsonValue::String).collect())
        };
        normalized.insert(key.to_string(), next);
    }
    if let Some(labels) = state
        .get("electron-workspace-root-labels")
        .and_then(JsonValue::as_object)
    {
        let mut next = serde_json::Map::new();
        for (path, label) in labels {
            next.insert(
                to_desktop_workspace_path(path).unwrap_or_else(|| path.clone()),
                label.clone(),
            );
        }
        normalized.insert(
            "electron-workspace-root-labels".to_string(),
            JsonValue::Object(next),
        );
    }
    if let Some(preferences) = state
        .get("open-in-target-preferences")
        .and_then(JsonValue::as_object)
    {
        let mut next_preferences = preferences.clone();
        if let Some(per_path) = preferences.get("perPath").and_then(JsonValue::as_object) {
            let mut next_per_path = serde_json::Map::new();
            for (path, preference) in per_path {
                next_per_path.insert(
                    to_desktop_workspace_path(path).unwrap_or_else(|| path.clone()),
                    preference.clone(),
                );
            }
            next_preferences.insert("perPath".to_string(), JsonValue::Object(next_per_path));
        }
        normalized.insert(
            "open-in-target-preferences".to_string(),
            JsonValue::Object(next_preferences),
        );
    }
    normalized
}

