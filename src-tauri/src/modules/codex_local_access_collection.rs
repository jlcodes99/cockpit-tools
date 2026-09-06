// Codex Local Access：Collection persistence, timeout normalization and gateway lifecycle coordination。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn request_ordered_account_ids(
    collection: &CodexLocalAccessCollection,
    scoped_account_ids: &[String],
    strategy: CodexLocalAccessRoutingStrategy,
    start: usize,
    priority_account_ids: &[String],
) -> Vec<String> {
    let ordered = if strategy == CodexLocalAccessRoutingStrategy::Custom {
        let scoped: HashSet<&str> = scoped_account_ids.iter().map(String::as_str).collect();
        collection
            .account_ids
            .iter()
            .filter(|account_id| scoped.contains(account_id.as_str()))
            .cloned()
            .collect()
    } else {
        build_ordered_account_ids(scoped_account_ids, start, None)
    };
    prioritize_account_ids(ordered, priority_account_ids)
}

fn allocate_random_local_port(bind_host: &str) -> Result<u16, String> {
    let listener =
        StdTcpListener::bind((bind_host, 0)).map_err(|e| format!("分配本地接入端口失败: {}", e))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("读取本地接入端口失败: {}", e))
}

fn configured_initial_local_access_port() -> Option<u16> {
    if let Ok(raw) = std::env::var(CODEX_LOCAL_ACCESS_API_PORT_ENV) {
        if let Ok(port) = raw.trim().parse::<u16>() {
            if port > 0 {
                return Some(port);
            }
        }
    }

    if account::is_dev_profile() {
        return Some(CODEX_LOCAL_ACCESS_DEV_DEFAULT_PORT);
    }

    None
}

fn allocate_initial_local_port(bind_host: &str) -> Result<u16, String> {
    configured_initial_local_access_port()
        .map(Ok)
        .unwrap_or_else(|| allocate_random_local_port(bind_host))
}

fn load_collection_from_disk() -> Result<Option<CodexLocalAccessCollection>, String> {
    let path = local_access_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取本地接入配置失败: {}", e))?;
    match serde_json::from_str::<CodexLocalAccessCollection>(&content) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，已隔离并使用默认关闭配置: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，文件已不存在，使用默认关闭配置: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，隔离失败，使用默认关闭配置: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(None)
        }
    }
}

fn save_collection_to_disk(collection: &CodexLocalAccessCollection) -> Result<(), String> {
    let path = local_access_file_path()?;
    let content = serde_json::to_string_pretty(collection)
        .map_err(|e| format!("序列化本地接入配置失败: {}", e))?;
    write_string_atomic(&path, &content)
}

fn normalize_stats_metadata(stats: &mut CodexLocalAccessStats) {
    let now = now_ms();
    if stats.since <= 0 {
        stats.since = now;
    }
    if stats.updated_at <= 0 {
        stats.updated_at = stats.since;
    }
    sort_stats_rows(stats);
}

fn sort_stats_rows(stats: &mut CodexLocalAccessStats) {
    sort_usage_accounts(&mut stats.accounts);
    sort_usage_models(&mut stats.models);
    sort_usage_api_keys(&mut stats.api_keys);
    for window in [&mut stats.daily, &mut stats.weekly, &mut stats.monthly] {
        sort_usage_accounts(&mut window.accounts);
        sort_usage_models(&mut window.models);
        sort_usage_api_keys(&mut window.api_keys);
    }
}

fn stats_snapshot_without_events(stats: &CodexLocalAccessStats) -> CodexLocalAccessStats {
    CodexLocalAccessStats {
        since: stats.since,
        updated_at: stats.updated_at,
        totals: stats.totals.clone(),
        accounts: stats.accounts.clone(),
        models: stats.models.clone(),
        api_keys: stats.api_keys.clone(),
        daily: stats.daily.clone(),
        weekly: stats.weekly.clone(),
        monthly: stats.monthly.clone(),
        events: Vec::new(),
    }
}

fn invalid_stats_backup_path(path: &Path) -> PathBuf {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let file_name = path
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or(CODEX_LOCAL_ACCESS_STATS_FILE);
    path.with_file_name(format!("{}.invalid-{}", file_name, timestamp))
}

fn recover_invalid_stats_file(
    path: &Path,
    parse_error: &serde_json::Error,
) -> CodexLocalAccessStats {
    let empty = empty_stats_snapshot();
    let backup_path = invalid_stats_backup_path(path);
    match std::fs::rename(path, &backup_path) {
        Ok(()) => {
            logger::log_codex_api_warn(&format!(
                "API 服务统计文件解析失败，已隔离并重建空统计: path={}, backup={}, error={}",
                path.display(),
                backup_path.display(),
                parse_error
            ));
        }
        Err(rename_error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务统计文件解析失败，隔离失败，尝试直接重建空统计: path={}, backup={}, parse_error={}, rename_error={}",
                path.display(),
                backup_path.display(),
                parse_error,
                rename_error
            ));
            match serde_json::to_string_pretty(&empty) {
                Ok(content) => {
                    if let Err(write_error) = write_string_atomic(path, &content) {
                        logger::log_codex_api_warn(&format!(
                            "API 服务统计文件重建失败，本次启动使用空统计: path={}, error={}",
                            path.display(),
                            write_error
                        ));
                    }
                }
                Err(serialize_error) => {
                    logger::log_codex_api_warn(&format!(
                        "API 服务空统计序列化失败，本次启动使用内存空统计: path={}, error={}",
                        path.display(),
                        serialize_error
                    ));
                }
            }
        }
    }
    empty
}

fn load_stats_snapshot_from_disk() -> Result<CodexLocalAccessStats, String> {
    let path = local_access_stats_file_path()?;
    if path.exists() {
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("读取 API 服务统计失败: {}", e))?;
        match serde_json::from_str::<CodexLocalAccessStats>(&content) {
            Ok(parsed) => Ok(parsed),
            Err(error) => Ok(recover_invalid_stats_file(&path, &error)),
        }
    } else {
        Ok(empty_stats_snapshot())
    }
}

fn load_stats_from_disk() -> Result<CodexLocalAccessStats, String> {
    let mut parsed = load_stats_snapshot_from_disk()?;
    let json_events = std::mem::take(&mut parsed.events);
    let request_logs_schema_state = match inspect_request_logs_schema_state() {
        Ok(state) => state,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "检查 API 服务日志 schema 失败，继续按旧库待迁移处理: {}",
                error
            ));
            RequestLogsSchemaState::MissingTable
        }
    };
    let bootstrap_without_service_tier = matches!(
        request_logs_schema_state,
        RequestLogsSchemaState::MissingTable
    ) && !json_events.is_empty();
    if let Err(error) =
        migrate_local_access_json_events(&json_events, !bootstrap_without_service_tier)
    {
        logger::log_codex_api_warn(&format!(
            "API 服务请求日志迁移失败，继续使用统计快照中的最近事件: {}",
            error
        ));
    }
    let now = now_ms();
    let (_, week_since, month_since) = local_calendar_window_starts(now);
    let retention_since = week_since.min(month_since);
    match load_stats_windows_and_recent_events(now) {
        Ok((daily, weekly, monthly, recent_events)) => {
            parsed.daily = daily;
            parsed.weekly = weekly;
            parsed.monthly = monthly;
            parsed.events = recent_events;
        }
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务请求日志读取失败，继续使用统计快照中的最近事件: {}",
                error
            ));
            let (day_since, week_since, month_since) = local_calendar_window_starts(now);
            parsed.daily = empty_stats_window(day_since, now);
            parsed.weekly = empty_stats_window(week_since, now);
            parsed.monthly = empty_stats_window(month_since, now);
            parsed.events.clear();
            for event in json_events
                .into_iter()
                .filter(|event| event.timestamp >= retention_since)
            {
                apply_usage_event_to_current_windows(&mut parsed, &event, now);
                push_recent_usage_event(&mut parsed.events, event);
            }
        }
    }
    normalize_stats_metadata(&mut parsed);
    Ok(parsed)
}

fn collection_content_fingerprint(
    collection: &CodexLocalAccessCollection,
) -> Result<Vec<u8>, String> {
    // Stable byte identity for CAS: any concurrent user edit must change the fingerprint.
    serde_json::to_vec(collection)
        .map_err(|error| format!("序列化 API 服务配置指纹失败: {}", error))
}

fn collection_disk_snapshot_for_cas(
    runtime_fingerprint: &[u8],
) -> Result<Option<(PathBuf, [u8; 32])>, String> {
    let path = local_access_file_path()?;
    let content = match std::fs::read(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "读取 API 服务配置 CAS 快照失败: path={}, error={}",
                path.display(),
                error
            ));
        }
    };
    let disk_collection =
        serde_json::from_slice::<CodexLocalAccessCollection>(&content).map_err(|error| {
            format!(
                "解析 API 服务配置 CAS 快照失败: path={}, error={}",
                path.display(),
                error
            )
        })?;
    if collection_content_fingerprint(&disk_collection)? != runtime_fingerprint {
        return Ok(None);
    }
    Ok(Some((path, Sha256::digest(&content).into())))
}

/// Background membership prune. Never writes a stale clone over a newer collection:
/// sanitize against a snapshot, then commit only if runtime still matches that snapshot.
fn run_collection_account_sanitize_once() -> Result<bool, String> {
    const MAX_CAS_RETRIES: u32 = 8;

    for attempt in 0..MAX_CAS_RETRIES {
        let base = {
            let runtime = gateway_runtime().blocking_lock();
            if !runtime.loaded {
                return Ok(false);
            }
            match runtime.collection.clone() {
                Some(collection) => collection,
                None => return Ok(false),
            }
        };
        let base_fingerprint = collection_content_fingerprint(&base)?;
        let Some((collection_path, expected_disk_hash)) =
            collection_disk_snapshot_for_cas(&base_fingerprint)?
        else {
            logger::log_codex_api_info(&format!(
                "API 服务账号成员后台清理检测到磁盘配置正在更新，重新读取快照: attempt={}",
                attempt + 1
            ));
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        };

        // Refresh the account snapshot on every retry. If a concurrent account import/delete
        // also changes collection membership, the next CAS round must not reuse stale accounts.
        let accounts = codex_account::list_accounts_checked()?;

        let mut next = base;
        let (changed, _) = sanitize_collection_with_accounts(&mut next, &accounts)?;
        if !changed {
            let mut runtime = gateway_runtime().blocking_lock();
            sync_runtime_quota_cooldowns(&mut runtime, &accounts, now_ms());
            return Ok(true);
        }
        next.updated_at = now_ms();
        let next_content = serde_json::to_string_pretty(&next)
            .map_err(|error| format!("序列化 API 服务后台清理配置失败: {}", error))?;

        // Double CAS: runtime fingerprint catches already-published mutations; the disk hash
        // catches a mutation that has persisted but has not published its runtime state yet.
        let committed = {
            let mut runtime = gateway_runtime().blocking_lock();
            if !runtime.loaded {
                return Ok(false);
            }
            let Some(current) = runtime.collection.as_ref() else {
                return Ok(false);
            };
            if collection_content_fingerprint(current)? != base_fingerprint {
                logger::log_codex_api_info(&format!(
                    "API 服务账号成员后台清理检测到配置已更新，丢弃旧快照并重试: attempt={}",
                    attempt + 1
                ));
                false
            } else {
                let written = write_string_atomic_if_hash_matches(
                    &collection_path,
                    expected_disk_hash,
                    || Ok(next_content),
                )?;
                if written {
                    sync_runtime_collection(&mut runtime, next);
                }
                written
            }
        };
        if committed {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    Err(format!(
        "API 服务账号成员后台清理在 {} 次 CAS 重试后仍与用户配置冲突",
        MAX_CAS_RETRIES
    ))
}

fn ensure_collection_account_sanitize_started() {
    if GATEWAY_COLLECTION_ACCOUNT_SANITIZE_COMPLETED.load(Ordering::SeqCst) {
        return;
    }
    if GATEWAY_COLLECTION_ACCOUNT_SANITIZE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result =
            tauri::async_runtime::spawn_blocking(run_collection_account_sanitize_once).await;
        match result {
            Ok(Ok(true)) => {
                GATEWAY_COLLECTION_ACCOUNT_SANITIZE_COMPLETED.store(true, Ordering::SeqCst);
            }
            Ok(Ok(false)) => {
                // Runtime not ready yet — leave COMPLETED clear so the next state read retries.
            }
            Ok(Err(error)) => logger::log_codex_api_warn(&format!(
                "API 服务账号成员后台清理失败，将在下次状态读取时重试: {}",
                error
            )),
            Err(error) => logger::log_codex_api_warn(&format!(
                "API 服务账号成员后台清理任务失败，将在下次状态读取时重试: {}",
                error
            )),
        }
        GATEWAY_COLLECTION_ACCOUNT_SANITIZE_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn replace_stats_window_if_more_complete(
    current: &mut CodexLocalAccessStatsWindow,
    candidate: CodexLocalAccessStatsWindow,
) {
    if candidate.since != current.since {
        return;
    }
    let candidate_count = candidate.totals.request_count;
    let current_count = current.totals.request_count;
    if candidate_count > current_count
        || (candidate_count == current_count && candidate.updated_at >= current.updated_at)
    {
        *current = candidate;
    }
}

fn ensure_stats_maintenance_started() {
    if GATEWAY_STATS_MAINTENANCE_COMPLETED.load(Ordering::SeqCst) {
        return;
    }
    if GATEWAY_STATS_MAINTENANCE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let baseline_revision = {
            let runtime = gateway_runtime().lock().await;
            runtime.stats_revision
        };
        let result = tauri::async_runtime::spawn_blocking(load_stats_from_disk).await;
        match result {
            Ok(Ok(mut maintained)) => {
                let mut runtime = gateway_runtime().lock().await;
                if runtime.loaded && runtime.stats_revision == baseline_revision {
                    // Merge only the bounded recent event list with live events. The windows
                    // have already been aggregated while streaming SQLite rows and must not be
                    // recomputed from the bounded list.
                    let mut events_by_key = maintained
                        .events
                        .drain(..)
                        .map(|event| (local_access_log_event_key(&event), event))
                        .collect::<HashMap<_, _>>();
                    for event in runtime.stats.events.iter().cloned() {
                        let key = local_access_log_event_key(&event);
                        events_by_key.insert(key, event);
                    }
                    maintained.events = events_by_key.into_values().collect();
                    maintained.events.sort_by_key(|event| event.timestamp);
                    if maintained.events.len() > STATE_RECENT_USAGE_EVENT_LIMIT {
                        let remove_count = maintained.events.len() - STATE_RECENT_USAGE_EVENT_LIMIT;
                        maintained.events.drain(..remove_count);
                    }
                    sort_stats_rows(&mut maintained);

                    // Keep runtime top-level lifetime aggregates (they include live traffic).
                    // Only backfill account/model/api_key rows that runtime never saw.
                    merge_missing_usage_accounts(&mut runtime.stats.accounts, &maintained.accounts);
                    merge_missing_usage_models(&mut runtime.stats.models, &maintained.models);
                    merge_missing_usage_api_keys(&mut runtime.stats.api_keys, &maintained.api_keys);
                    sort_usage_accounts(&mut runtime.stats.accounts);
                    sort_usage_models(&mut runtime.stats.models);
                    sort_usage_api_keys(&mut runtime.stats.api_keys);

                    runtime.stats.events = maintained.events;
                    replace_stats_window_if_more_complete(
                        &mut runtime.stats.daily,
                        maintained.daily,
                    );
                    replace_stats_window_if_more_complete(
                        &mut runtime.stats.weekly,
                        maintained.weekly,
                    );
                    replace_stats_window_if_more_complete(
                        &mut runtime.stats.monthly,
                        maintained.monthly,
                    );
                    runtime.stats.updated_at = runtime.stats.updated_at.max(maintained.updated_at);
                    if runtime.stats.since <= 0 {
                        runtime.stats.since = maintained.since;
                    } else if maintained.since > 0 {
                        runtime.stats.since = runtime.stats.since.min(maintained.since);
                    }
                } else if runtime.loaded {
                    logger::log_codex_api_info(
                        "API 服务统计后台维护期间运行态已更新，保留当前统计以避免覆盖新结果",
                    );
                }
                GATEWAY_STATS_MAINTENANCE_COMPLETED.store(true, Ordering::SeqCst);
            }
            Ok(Err(error)) => logger::log_codex_api_warn(&format!(
                "API 服务统计后台维护失败，将在下次状态读取时重试: {}",
                error
            )),
            Err(error) => logger::log_codex_api_warn(&format!(
                "API 服务统计后台维护任务失败，将在下次状态读取时重试: {}",
                error
            )),
        }
        GATEWAY_STATS_MAINTENANCE_RUNNING.store(false, Ordering::SeqCst);
    });
}

fn save_stats_to_disk(stats: &CodexLocalAccessStats) -> Result<(), String> {
    let path = local_access_stats_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 API 服务统计目录失败: {}", e))?;
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct StatsSnapshotWithoutEvents<'a> {
        since: i64,
        updated_at: i64,
        totals: &'a CodexLocalAccessUsageStats,
        accounts: &'a [CodexLocalAccessAccountStats],
        models: &'a [CodexLocalAccessModelStats],
        api_keys: &'a [CodexLocalAccessApiKeyStats],
        daily: &'a CodexLocalAccessStatsWindow,
        weekly: &'a CodexLocalAccessStatsWindow,
        monthly: &'a CodexLocalAccessStatsWindow,
        events: &'a [CodexLocalAccessUsageEvent],
    }

    let snapshot = StatsSnapshotWithoutEvents {
        since: stats.since,
        updated_at: stats.updated_at,
        totals: &stats.totals,
        accounts: &stats.accounts,
        models: &stats.models,
        api_keys: &stats.api_keys,
        daily: &stats.daily,
        weekly: &stats.weekly,
        monthly: &stats.monthly,
        events: &[],
    };
    let content = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| format!("序列化 API 服务统计失败: {}", e))?;
    write_string_atomic(&path, &content)
}

fn prune_runtime_routing_state(runtime: &mut GatewayRuntime, now: i64) {
    let session_affinity_ttl_ms = runtime
        .collection
        .as_ref()
        .map(|collection| {
            collection
                .session_affinity_ttl_ms
                .clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS)
        })
        .unwrap_or(DEFAULT_SESSION_AFFINITY_TTL_MS);
    runtime.response_affinity.retain(|key, binding| {
        let ttl_ms = if key.starts_with("session:") {
            session_affinity_ttl_ms
        } else {
            RESPONSE_AFFINITY_TTL_MS
        };
        now.saturating_sub(binding.updated_at_ms) <= ttl_ms
    });
    runtime
        .model_cooldowns
        .retain(|_, cooldown| cooldown.next_retry_at_ms > now);

    if runtime.response_affinity.len() <= MAX_RESPONSE_AFFINITY_BINDINGS {
        return;
    }

    let mut bindings: Vec<(String, i64)> = runtime
        .response_affinity
        .iter()
        .map(|(response_id, binding)| (response_id.clone(), binding.updated_at_ms))
        .collect();
    bindings.sort_by_key(|(_, updated_at_ms)| *updated_at_ms);

    let remove_count = runtime
        .response_affinity
        .len()
        .saturating_sub(MAX_RESPONSE_AFFINITY_BINDINGS);
    for (response_id, _) in bindings.into_iter().take(remove_count) {
        runtime.response_affinity.remove(&response_id);
    }
}

async fn resolve_affinity_account(previous_response_id: &str) -> Option<String> {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime
        .response_affinity
        .get(previous_response_id)
        .map(|binding| binding.account_id.clone())
}

async fn bind_response_affinity(response_id: &str, account_id: &str) {
    let response_id = response_id.trim();
    let account_id = account_id.trim();
    if response_id.is_empty() || account_id.is_empty() {
        return;
    }

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime.response_affinity.insert(
        response_id.to_string(),
        ResponseAffinityBinding {
            account_id: account_id.to_string(),
            updated_at_ms: now,
        },
    );
    prune_runtime_routing_state(&mut runtime, now);
}

fn session_affinity_binding_key(value: &str) -> String {
    format!("session:{}", value.trim())
}

fn extract_body_string_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn extract_session_affinity_key(request: &ParsedRequest) -> Option<String> {
    for header in [
        "session-id",
        "session_id",
        "x-session-id",
        "thread-id",
        "x-client-request-id",
        "x-amp-thread-id",
    ] {
        if let Some(value) = request
            .headers
            .get(header)
            .map(String::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            return Some(format!("{}={}", header, value));
        }
    }

    let body = parse_request_body_json(&request.body)?;
    extract_body_string_path(&body, &["metadata", "user_id"])
        .or_else(|| extract_body_string_path(&body, &["conversation_id"]))
        .or_else(|| extract_body_string_path(&body, &["thread_id"]))
        .map(|value| format!("body={}", value))
}

fn header_value<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .get(&name.to_ascii_lowercase())
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn stable_uuid_from_text(value: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn stable_prompt_cache_key(api_key: &ResolvedLocalApiKey) -> String {
    stable_uuid_from_text(&format!("agtools:codex:prompt-cache:{}", api_key.id))
}

fn stable_codex_installation_id(api_key: &ResolvedLocalApiKey) -> String {
    stable_uuid_from_text(&format!("agtools:codex:installation:{}", api_key.id))
}

fn stable_codex_turn_id(api_key: &ResolvedLocalApiKey, session_id: &str) -> String {
    stable_uuid_from_text(&format!("agtools:codex:turn:{}:{}", api_key.id, session_id))
}

fn extract_prompt_cache_key_from_value(value: &Value) -> Option<String> {
    value
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_prompt_cache_key(
    headers: &HashMap<String, String>,
    body_value: Option<&Value>,
    api_key: &ResolvedLocalApiKey,
) -> String {
    body_value
        .and_then(extract_prompt_cache_key_from_value)
        .or_else(|| header_value(headers, "session-id").map(str::to_string))
        .or_else(|| header_value(headers, "session_id").map(str::to_string))
        .unwrap_or_else(|| stable_prompt_cache_key(api_key))
}

fn is_valid_gpt_reasoning_signature(raw_signature: &str) -> bool {
    if raw_signature.is_empty()
        || raw_signature.len() > MAX_GPT_REASONING_SIGNATURE_LEN
        || raw_signature != raw_signature.trim()
        || !raw_signature.starts_with("gAAAA")
        || raw_signature
            .chars()
            .any(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '=')
    {
        return false;
    }

    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(raw_signature)
        .or_else(|_| general_purpose::URL_SAFE.decode(raw_signature));
    let Ok(decoded) = decoded else {
        return false;
    };
    if decoded.len() < 73 || decoded.first().copied() != Some(0x80) {
        return false;
    }
    let ciphertext_len = decoded.len().saturating_sub(1 + 8 + 16 + 32);
    ciphertext_len > 0 && ciphertext_len % 16 == 0
}

fn sanitize_codex_reasoning_encrypted_content(body_value: &mut Value) -> bool {
    let Some(input_items) = body_value.get_mut("input").and_then(Value::as_array_mut) else {
        return false;
    };

    let mut changed = false;
    for item in input_items {
        let Some(item_obj) = item.as_object_mut() else {
            continue;
        };
        if item_obj.get("type").and_then(Value::as_str).map(str::trim) != Some("reasoning") {
            continue;
        }

        let should_remove = match item_obj.get("encrypted_content") {
            Some(Value::String(value)) => !is_valid_gpt_reasoning_signature(value),
            Some(_) => true,
            None => false,
        };
        if should_remove {
            item_obj.remove("encrypted_content");
            changed = true;
        }
    }
    changed
}

fn build_codex_turn_metadata(session_id: &str, turn_id: &str) -> String {
    let window_id = format!("{}:0", session_id);
    serde_json::to_string(&json!({
        "prompt_cache_key": session_id,
        "turn_id": turn_id,
        "window_id": window_id,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn apply_codex_client_metadata(
    body_obj: &mut Map<String, Value>,
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
    session_id: &str,
) {
    let installation_id = stable_codex_installation_id(api_key);
    let turn_id = stable_codex_turn_id(api_key, session_id);
    let window_id = format!("{}:0", session_id);
    let turn_metadata = build_codex_turn_metadata(session_id, &turn_id);

    let client_metadata = body_obj
        .entry("client_metadata".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !client_metadata.is_object() {
        *client_metadata = Value::Object(Map::new());
    }
    if let Some(metadata_obj) = client_metadata.as_object_mut() {
        metadata_obj
            .entry("x-codex-installation-id".to_string())
            .or_insert_with(|| Value::String(installation_id));
        metadata_obj.insert(
            "x-codex-window-id".to_string(),
            Value::String(window_id.clone()),
        );
        metadata_obj.insert(
            "x-codex-turn-metadata".to_string(),
            Value::String(turn_metadata.clone()),
        );
    }

    request
        .headers
        .insert("x-client-request-id".to_string(), session_id.to_string());
    request
        .headers
        .insert("thread-id".to_string(), session_id.to_string());
    request
        .headers
        .insert("x-codex-window-id".to_string(), window_id);
    request
        .headers
        .insert("x-codex-turn-metadata".to_string(), turn_metadata);
}

fn ensure_request_header(headers: &mut HashMap<String, String>, name: &str, value: &str) {
    headers
        .entry(name.to_ascii_lowercase())
        .or_insert_with(|| value.to_string());
}

fn apply_codex_official_headers(request: &mut ParsedRequest) {
    if !(is_responses_request(&request.target) || is_responses_compact_request(&request.target)) {
        return;
    }

    for header in CODEX_OFFICIAL_EMPTY_HEADERS {
        ensure_request_header(&mut request.headers, header, "");
    }
}

fn align_codex_prompt_cache(
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
) -> Result<Option<String>, String> {
    if !(is_responses_request(&request.target) || is_responses_compact_request(&request.target)) {
        return Ok(None);
    }

    let mut body_value = parse_request_body_json(&request.body);
    let session_id = resolve_prompt_cache_key(&request.headers, body_value.as_ref(), api_key);
    request
        .headers
        .insert("session-id".to_string(), session_id.clone());
    request
        .headers
        .insert("conversation_id".to_string(), session_id.clone());

    if let Some(Value::Object(body_obj)) = body_value.as_mut() {
        body_obj.insert(
            "prompt_cache_key".to_string(),
            Value::String(session_id.clone()),
        );
        apply_codex_client_metadata(body_obj, request, api_key, &session_id);
    }
    if let Some(body_value) = body_value.as_mut() {
        sanitize_codex_reasoning_encrypted_content(body_value);
        request.body = serde_json::to_vec(body_value)
            .map_err(|e| format!("序列化 prompt_cache_key 请求体失败: {}", e))?;
    }

    Ok(Some(session_id))
}

async fn touch_local_access_api_key(api_key_id: &str) {
    let api_key_id = api_key_id.trim();
    if api_key_id.is_empty() || api_key_id == "legacy" {
        return;
    }
    let mut collection_to_save = None;
    {
        let mut runtime = gateway_runtime().lock().await;
        let Some(collection) = runtime.collection.as_mut() else {
            return;
        };
        if let Some(api_key) = collection
            .api_keys
            .iter_mut()
            .find(|item| item.id == api_key_id)
        {
            let now = now_ms();
            api_key.last_used_at = Some(now);
            api_key.updated_at = now;
            collection.updated_at = now;
            collection_to_save = Some(collection.clone());
        }
    }
    if let Some(collection) = collection_to_save {
        if let Err(err) = save_collection_to_disk(&collection) {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 更新 API Key 最近使用时间失败: {}",
                err
            ));
        }
    }
}

async fn clear_model_cooldown(account_id: &str, model_key: &str) {
    let Some(cooldown_key) = build_cooldown_key(account_id, model_key) else {
        return;
    };

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime.model_cooldowns.remove(&cooldown_key);
}

async fn set_model_cooldown(
    account_id: &str,
    model_key: &str,
    retry_after: Duration,
    reason: &str,
) {
    let Some(cooldown_key) = build_cooldown_key(account_id, model_key) else {
        return;
    };
    if retry_after <= Duration::ZERO {
        return;
    }

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let next_retry_at_ms = now.saturating_add(retry_after.as_millis() as i64);
    prune_runtime_routing_state(&mut runtime, now);
    runtime.model_cooldowns.insert(
        cooldown_key,
        AccountModelCooldown {
            model_key: model_key.trim().to_string(),
            next_retry_at_ms,
            reason: reason.trim().to_string(),
        },
    );
}

async fn mark_account_success(account: &CodexAccount, request_kind: CodexLocalAccessRequestKind) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account.id.clone())
        .or_default();
    health.email = account.email.clone();
    health.consecutive_failures = 0;
    health.last_success_at = Some(now);
    health.last_failure_at = None;
    health.last_failure_status = None;
    health.last_failure_category = None;
    health.last_failure_message = None;
    if request_kind_is_image(request_kind) {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Available;
        health.image_generation_checked_at = Some(now);
    }
}

async fn mark_account_failure(
    account: &CodexAccount,
    status: Option<u16>,
    category: Option<&str>,
    message: &str,
    request_kind: CodexLocalAccessRequestKind,
) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account.id.clone())
        .or_default();
    health.email = account.email.clone();
    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
    health.last_failure_at = Some(now);
    health.last_failure_status = status;
    health.last_failure_category = category.map(str::to_string);
    health.last_failure_message =
        Some(message.trim().to_string()).filter(|value| !value.is_empty());
    if category == Some("image_generation_not_enabled") {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Unavailable;
        health.image_generation_checked_at = Some(now);
    } else if request_kind_is_image(request_kind)
        && health.image_generation_status == CodexLocalAccessImageGenerationStatus::Unknown
    {
        health.image_generation_checked_at = Some(now);
    }
}

async fn get_model_cooldown_wait(account_id: &str, model_key: &str) -> Option<Duration> {
    let cooldown_key = build_cooldown_key(account_id, model_key)?;
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    let cooldown = runtime.model_cooldowns.get(&cooldown_key)?;
    let wait_ms = cooldown.next_retry_at_ms.saturating_sub(now);
    if wait_ms <= 0 {
        return None;
    }
    Some(Duration::from_millis(wait_ms as u64))
}

fn ensure_local_port_available(
    bind_host: &str,
    port: u16,
    current_port: Option<u16>,
) -> Result<(), String> {
    if port == 0 {
        return Err("端口必须在 1 到 65535 之间".to_string());
    }
    if current_port == Some(port) {
        return Ok(());
    }
    let listener = StdTcpListener::bind((bind_host, port))
        .map_err(|e| format!("端口 {} 不可用: {}", port, e))?;
    drop(listener);
    Ok(())
}

fn is_local_access_port_bindable(bind_host: &str, port: u16) -> Result<bool, std::io::Error> {
    match StdTcpListener::bind((bind_host, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
        Err(error) => Err(error),
    }
}

async fn wait_for_gateway_port_release(bind_host: &str, port: u16) -> Result<(), String> {
    let deadline = Instant::now() + GATEWAY_PORT_RELEASE_TIMEOUT;

    loop {
        match is_local_access_port_bindable(bind_host, port) {
            Ok(true) => return Ok(()),
            Ok(false) if Instant::now() < deadline => {
                tokio::time::sleep(GATEWAY_PORT_RELEASE_POLL_INTERVAL).await;
            }
            Ok(false) => {
                return Err(format!("API 服务端口 {} 停止后仍未释放，请稍后重试", port));
            }
            Err(error) => {
                return Err(format!(
                    "检查 API 服务端口 {} 释放状态失败: {}",
                    port, error
                ));
            }
        }
    }
}

async fn bind_gateway_listener(bind_host: &str, port: u16) -> Result<TcpListener, std::io::Error> {
    let deadline = Instant::now() + GATEWAY_PORT_RELEASE_TIMEOUT;

    loop {
        match TcpListener::bind((bind_host, port)).await {
            Ok(listener) => return Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse && Instant::now() < deadline =>
            {
                tokio::time::sleep(GATEWAY_PORT_RELEASE_POLL_INTERVAL).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// 判断端口是否落在保留区间列表内（闭区间）。
pub fn port_in_reserved_ranges(port: u16, ranges: &[(u16, u16)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| port >= *start.min(end) && port <= *start.max(end))
}

/// 组装网关绑定失败文案；若命中保留端口区间则追加 Windows 保留端口提示。
pub fn format_gateway_bind_error_message(
    bind_host: &str,
    port: u16,
    error: &std::io::Error,
    reserved_ranges: &[(u16, u16)],
) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        let mut message = format!(
            "启动本地接入服务失败: {}:{} 已被占用，请先清理端口或改用其他端口（{}）",
            bind_host, port, error
        );
        if port_in_reserved_ranges(port, reserved_ranges) {
            message.push_str(&format!(
                "。提示：端口 {} 可能处于 Windows 排除/保留端口范围（Hyper-V/WSL 等），请换端口或用 netsh 查看 excludedportrange",
                port
            ));
        }
        return message;
    }
    format!("启动本地接入服务失败: {}", error)
}

#[cfg(target_os = "windows")]
fn windows_excluded_tcp_port_ranges() -> Vec<(u16, u16)> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new("netsh")
        .args([
            "interface",
            "ipv4",
            "show",
            "excludedportrange",
            "protocol=tcp",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_windows_excluded_port_ranges(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "windows"))]
fn windows_excluded_tcp_port_ranges() -> Vec<(u16, u16)> {
    Vec::new()
}

/// 解析 `netsh ... excludedportrange` 文本中的起止端口。
pub fn parse_windows_excluded_port_ranges(output: &str) -> Vec<(u16, u16)> {
    let mut ranges = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let Ok(start) = parts[0].parse::<u16>() else {
            continue;
        };
        let Ok(end) = parts[1].parse::<u16>() else {
            continue;
        };
        ranges.push((start, end));
    }
    ranges
}

fn format_gateway_bind_error(bind_host: &str, port: u16, error: &std::io::Error) -> String {
    let reserved = if cfg!(target_os = "windows") {
        windows_excluded_tcp_port_ranges()
    } else {
        Vec::new()
    };
    format_gateway_bind_error_message(bind_host, port, error, &reserved)
}

fn is_free_plan_type(plan_type: Option<&str>) -> bool {
    let Some(plan_type) = plan_type else {
        return false;
    };
    let normalized = plan_type.trim().to_ascii_lowercase();
    !normalized.is_empty() && normalized.contains("free")
}

fn local_access_account_has_oauth_token(account: &CodexAccount) -> bool {
    account.is_agent_identity_auth()
        || !account.tokens.access_token.trim().is_empty()
        || !account.tokens.id_token.trim().is_empty()
        || codex_account::account_has_refresh_token(account)
}

fn local_access_ineligible_reason(
    account: &CodexAccount,
    restrict_free_accounts: bool,
) -> Option<&'static str> {
    // PENDING / incomplete OAuth: no usable credentials for API service routing.
    if codex_account::is_pending_oauth_account(account)
        || (!account.is_api_key_auth() && !local_access_account_has_oauth_token(account))
    {
        return Some("pending_oauth");
    }
    // ChatGPT Web Session 仅支持查额，禁止加入 API 服务。
    if account.is_web_session_auth() {
        return Some("web_session_quota_only");
    }
    if is_chat_completions_api_key_account(account) {
        return Some("chat_completions_api_key");
    }
    if is_official_deepseek_account(account) {
        return Some("deepseek_unsupported");
    }
    if restrict_free_accounts
        && !account.is_agent_identity_auth()
        && is_free_plan_type(account.plan_type.as_deref())
    {
        return Some("free_restricted");
    }
    None
}

fn is_local_access_eligible_account(account: &CodexAccount, restrict_free_accounts: bool) -> bool {
    local_access_ineligible_reason(account, restrict_free_accounts).is_none()
}

fn normalize_upstream_proxy_url(upstream_proxy_url: Option<String>) -> Option<String> {
    upstream_proxy_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<Option<String>, String> {
    let normalized = normalize_upstream_proxy_url(upstream_proxy_url);
    if let Some(proxy_url) = normalized.as_deref() {
        Proxy::all(proxy_url).map_err(|e| format!("API 代理地址无效: {}", e))?;
    }
    Ok(normalized)
}

fn clamp_timeout_ms(value: u64, fallback: u64, max: u64) -> u64 {
    let base = if value == 0 { fallback } else { value };
    base.clamp(LOCAL_ACCESS_TIMEOUT_MIN_MS, max)
}

fn clamp_retry_delay_ms(value: u64, fallback: u64) -> u64 {
    let base = if value == 0 { fallback } else { value };
    base.clamp(
        LOCAL_ACCESS_RETRY_DELAY_MIN_MS,
        LOCAL_ACCESS_RETRY_DELAY_MAX_MS,
    )
}

fn normalize_timeouts(timeouts: &mut CodexLocalAccessTimeouts) -> bool {
    let original = timeouts.clone();
    let defaults = CodexLocalAccessTimeouts::default();
    timeouts.legacy_request_read_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_request_read_timeout_ms,
        defaults.legacy_request_read_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_upstream_connect_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_upstream_connect_timeout_ms,
        defaults.legacy_upstream_connect_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_stream_idle_timeout_ms,
        defaults.legacy_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_stream_total_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_stream_total_timeout_ms,
        defaults.legacy_stream_total_timeout_ms,
        LEGACY_STREAM_TOTAL_TIMEOUT_MAX_MS,
    );
    if timeouts.legacy_stream_total_timeout_ms < timeouts.legacy_stream_idle_timeout_ms {
        timeouts.legacy_stream_total_timeout_ms = timeouts.legacy_stream_idle_timeout_ms;
    }
    timeouts.sidecar_stream_open_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_stream_open_timeout_ms,
        defaults.sidecar_stream_open_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_stream_idle_timeout_ms,
        defaults.sidecar_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_image_stream_open_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_image_stream_open_timeout_ms,
        defaults.sidecar_image_stream_open_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_image_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_image_stream_idle_timeout_ms,
        defaults.sidecar_image_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_stream_open_max_attempts = timeouts.sidecar_stream_open_max_attempts.clamp(
        SIDECAR_STREAM_OPEN_ATTEMPTS_MIN,
        SIDECAR_STREAM_OPEN_ATTEMPTS_MAX,
    );
    timeouts.sidecar_stream_keepalive_seconds = timeouts.sidecar_stream_keepalive_seconds.clamp(
        SIDECAR_STREAM_KEEPALIVE_MIN_SECONDS,
        SIDECAR_STREAM_KEEPALIVE_MAX_SECONDS,
    );
    timeouts.websocket_connect_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_connect_timeout_ms,
        defaults.websocket_connect_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_initial_message_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_initial_message_timeout_ms,
        defaults.websocket_initial_message_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_idle_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_idle_timeout_ms,
        defaults.websocket_idle_timeout_ms,
        WEBSOCKET_IDLE_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_heartbeat_interval_ms = clamp_timeout_ms(
        timeouts.websocket_heartbeat_interval_ms,
        defaults.websocket_heartbeat_interval_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.upstream_send_retry_attempts = timeouts.upstream_send_retry_attempts.clamp(
        LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
        LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
    );
    timeouts.upstream_send_retry_base_delay_ms = clamp_retry_delay_ms(
        timeouts.upstream_send_retry_base_delay_ms,
        defaults.upstream_send_retry_base_delay_ms,
    );
    timeouts.upstream_send_retry_max_delay_ms = clamp_retry_delay_ms(
        timeouts.upstream_send_retry_max_delay_ms,
        defaults.upstream_send_retry_max_delay_ms,
    );
    if timeouts.upstream_send_retry_max_delay_ms < timeouts.upstream_send_retry_base_delay_ms {
        timeouts.upstream_send_retry_max_delay_ms = timeouts.upstream_send_retry_base_delay_ms;
    }
    timeouts.single_account_status_retry_attempts =
        timeouts.single_account_status_retry_attempts.clamp(
            LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
            LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
        );
    timeouts.single_account_status_retry_base_delay_ms = clamp_retry_delay_ms(
        timeouts.single_account_status_retry_base_delay_ms,
        defaults.single_account_status_retry_base_delay_ms,
    );
    timeouts.single_account_status_retry_max_delay_ms = clamp_retry_delay_ms(
        timeouts.single_account_status_retry_max_delay_ms,
        defaults.single_account_status_retry_max_delay_ms,
    );
    if timeouts.single_account_status_retry_max_delay_ms
        < timeouts.single_account_status_retry_base_delay_ms
    {
        timeouts.single_account_status_retry_max_delay_ms =
            timeouts.single_account_status_retry_base_delay_ms;
    }
    timeouts.sidecar_streaming_bootstrap_retries =
        timeouts.sidecar_streaming_bootstrap_retries.clamp(
            LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
            LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
        );
    *timeouts != original
}

fn collection_timeouts(collection: &CodexLocalAccessCollection) -> CodexLocalAccessTimeouts {
    let mut timeouts = collection.timeouts.clone();
    normalize_timeouts(&mut timeouts);
    timeouts
}

fn normalize_timeout_preset_name(name: &str) -> String {
    name.trim()
        .chars()
        .take(TIMEOUT_PRESET_NAME_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn normalize_timeout_preset_id(id: &str) -> Option<String> {
    let normalized = id.trim();
    if normalized.is_empty()
        || normalized == BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID
        || normalized == BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID
    {
        return None;
    }
    Some(normalized.to_string())
}

fn normalize_timeout_presets(presets: &mut Vec<CodexLocalAccessTimeoutPreset>) -> bool {
    let original = presets.clone();
    let now = now_ms();
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for mut preset in std::mem::take(presets) {
        if normalized.len() >= MAX_CUSTOM_TIMEOUT_PRESETS {
            break;
        }
        let Some(id) = normalize_timeout_preset_id(&preset.id) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let name = normalize_timeout_preset_name(&preset.name);
        if name.is_empty() {
            continue;
        }
        normalize_timeouts(&mut preset.timeouts);
        preset.id = id;
        preset.name = name;
        if preset.created_at <= 0 {
            preset.created_at = now;
        }
        if preset.updated_at <= 0 {
            preset.updated_at = preset.created_at;
        }
        normalized.push(preset);
    }

    *presets = normalized;
    *presets != original
}

fn normalize_active_timeout_preset_id(collection: &mut CodexLocalAccessCollection) -> bool {
    let original = collection.active_timeout_preset_id.clone();
    let current = collection.active_timeout_preset_id.trim();
    let normalized = if current == BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID
        || collection
            .timeout_presets
            .iter()
            .any(|preset| preset.id == current)
    {
        current.to_string()
    } else {
        BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID.to_string()
    };
    collection.active_timeout_preset_id = normalized;
    collection.active_timeout_preset_id != original
}

fn migrate_session_affinity_default_enabled(collection: &mut CodexLocalAccessCollection) -> bool {
    if collection.session_affinity_default_enabled_migrated {
        return false;
    }

    collection.session_affinity = true;
    collection.session_affinity_default_enabled_migrated = true;
    true
}

fn sanitize_collection(
    collection: &mut CodexLocalAccessCollection,
) -> Result<(bool, HashSet<String>), String> {
    let accounts = codex_account::list_accounts_checked()?;
    sanitize_collection_with_accounts(collection, &accounts)
}

/// Structure-only sanitize for cold start: ports, keys, pricing, timeouts.
/// Does **not** load account details or prune membership lists.
fn sanitize_collection_structure(
    collection: &mut CodexLocalAccessCollection,
) -> Result<bool, String> {
    let mut changed = false;

    // The legacy gateway is retired. Existing collections are migrated to the
    // sidecar before runtime startup and persisted by the normal load path.
    changed |= migrate_legacy_gateway_mode(collection);

    // v1.3.4 起已移除集合级 image_generation 禁用 UI。
    // 遗留 Disabled / ImagesOnly 会继续从 sidecar manifest 过滤 gpt-image-2，
    // 但静态 Codex catalog 仍包含该模型，造成 Codex 可点生图、网关却 model_not_available。
    // 请求级控制仍保留：Responses Lite 头与 x-agtools-disable-image-generation。
    if collection.image_generation_mode != CodexLocalAccessImageGenerationMode::Enabled {
        collection.image_generation_mode = CodexLocalAccessImageGenerationMode::Enabled;
        changed = true;
    }

    if collection.port == 0 {
        collection.port = allocate_initial_local_port(bind_host_for_collection(collection))?;
        changed = true;
    }
    if collection.api_key.trim().is_empty() {
        collection.api_key = generate_local_api_key();
        changed = true;
    }
    changed |= migrate_session_affinity_default_enabled(collection);
    changed |= normalize_collection_api_keys(collection);
    if collection.created_at <= 0 {
        collection.created_at = now_ms();
        changed = true;
    }
    if collection.updated_at <= 0 {
        collection.updated_at = now_ms();
        changed = true;
    }
    let normalized_upstream_proxy_url =
        normalize_upstream_proxy_url(collection.upstream_proxy_url.clone());
    if normalized_upstream_proxy_url != collection.upstream_proxy_url {
        collection.upstream_proxy_url = normalized_upstream_proxy_url;
        changed = true;
    }
    let normalized_bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if normalized_bound_oauth_account_id != collection.bound_oauth_account_id {
        collection.bound_oauth_account_id = normalized_bound_oauth_account_id;
        changed = true;
    }
    let has_bound_oauth_account = collection.bound_oauth_account_id.is_some();
    changed |= normalize_bound_oauth_quota_reserve(
        &mut collection.bound_oauth_quota_reserve,
        has_bound_oauth_account,
    );

    let original_custom_routing_rules = std::mem::take(&mut collection.custom_routing_rules);
    let normalized_custom_routing_rules = normalize_custom_routing_rules(
        original_custom_routing_rules.clone(),
        &collection.account_ids,
    );
    if normalized_custom_routing_rules != original_custom_routing_rules {
        changed = true;
    }
    collection.custom_routing_rules = normalized_custom_routing_rules;

    let original_account_model_rules = std::mem::take(&mut collection.account_model_rules);
    let normalized_account_model_rules = normalize_account_model_rules(
        original_account_model_rules.clone(),
        &collection.account_ids,
    );
    if normalized_account_model_rules != original_account_model_rules {
        changed = true;
    }
    collection.account_model_rules = normalized_account_model_rules;

    let original_model_aliases = std::mem::take(&mut collection.model_aliases);
    let normalized_model_aliases = normalize_model_aliases(original_model_aliases.clone());
    if normalized_model_aliases != original_model_aliases {
        changed = true;
    }
    collection.model_aliases = normalized_model_aliases;

    let original_model_pricings = std::mem::take(&mut collection.model_pricings);
    let normalized_model_pricings = normalize_model_pricings(original_model_pricings.clone());
    if normalized_model_pricings != original_model_pricings {
        changed = true;
    }
    collection.model_pricings =
        drop_superseded_default_56_model_pricings(normalized_model_pricings);
    if collection.model_pricings != original_model_pricings {
        changed = true;
    }
    if collection.model_pricing_version < DEFAULT_MODEL_PRICING_VERSION {
        collection.model_pricings = Vec::new();
        collection.model_pricing_version = DEFAULT_MODEL_PRICING_VERSION;
        changed = true;
    }

    let original_excluded_models = std::mem::take(&mut collection.excluded_models);
    let normalized_excluded_models = normalize_model_rule_list(original_excluded_models.clone());
    if normalized_excluded_models != original_excluded_models {
        changed = true;
    }
    collection.excluded_models = normalized_excluded_models;

    let normalized_session_affinity_ttl_ms = collection
        .session_affinity_ttl_ms
        .clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
    if normalized_session_affinity_ttl_ms != collection.session_affinity_ttl_ms {
        collection.session_affinity_ttl_ms = normalized_session_affinity_ttl_ms;
        changed = true;
    }
    let normalized_max_retry_credentials = collection
        .max_retry_credentials
        .min(MAX_RETRY_CREDENTIALS_PER_REQUEST as u16);
    if normalized_max_retry_credentials != collection.max_retry_credentials {
        collection.max_retry_credentials = normalized_max_retry_credentials;
        changed = true;
    }
    let normalized_max_retry_interval_ms = collection
        .max_retry_interval_ms
        .clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS);
    if normalized_max_retry_interval_ms != collection.max_retry_interval_ms {
        collection.max_retry_interval_ms = normalized_max_retry_interval_ms;
        changed = true;
    }
    changed |= normalize_timeouts(&mut collection.timeouts);
    changed |= normalize_timeout_presets(&mut collection.timeout_presets);
    changed |= normalize_active_timeout_preset_id(collection);

    Ok(changed)
}

fn sanitize_collection_with_accounts(
    collection: &mut CodexLocalAccessCollection,
    accounts: &[CodexAccount],
) -> Result<(bool, HashSet<String>), String> {
    let mut changed = sanitize_collection_structure(collection)?;

    let valid_bound_oauth_account_ids: HashSet<String> = accounts
        .iter()
        .filter(|account| {
            !account.is_api_key_auth()
                && !account.is_agent_identity_auth()
                && codex_account::account_has_refresh_token(account)
        })
        .map(|account| account.id.clone())
        .collect();
    let valid_account_ids: HashSet<String> = accounts
        .iter()
        .filter(|account| {
            is_local_access_eligible_account(account, collection.restrict_free_accounts)
        })
        .map(|account| account.id.clone())
        .collect();
    let valid_provider_gateway_account_ids: HashSet<String> = accounts
        .iter()
        .filter(|account| is_provider_gateway_eligible_account(account))
        .map(|account| account.id.clone())
        .collect();

    if let Some(bound_id) = collection.bound_oauth_account_id.as_deref() {
        if !valid_bound_oauth_account_ids.contains(bound_id) {
            collection.bound_oauth_account_id = None;
            changed = true;
            changed |= normalize_bound_oauth_quota_reserve(
                &mut collection.bound_oauth_quota_reserve,
                false,
            );
        }
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for account_id in &collection.account_ids {
        if !valid_account_ids.contains(account_id) {
            changed = true;
            continue;
        }
        if !seen.insert(account_id.clone()) {
            changed = true;
            continue;
        }
        deduped.push(account_id.clone());
    }
    if deduped != collection.account_ids {
        collection.account_ids = deduped;
        changed = true;
    }

    // 成员移除后同步清理策略，避免旧账号策略影响后续重新加入的账号。
    let before_image_policies = collection.image_generation_account_policies.clone();
    let known_account_ids: HashSet<&str> = collection.account_ids.iter().map(String::as_str).collect();
    collection
        .image_generation_account_policies
        .retain(|account_id, _| known_account_ids.contains(account_id.as_str()));
    if collection.image_generation_account_policies != before_image_policies {
        changed = true;
    }

    for api_key in &mut collection.api_keys {
        let before = api_key.account_ids.clone();
        let valid_scope_account_ids = if api_key.provider_gateway.is_some() {
            &valid_provider_gateway_account_ids
        } else {
            &valid_account_ids
        };
        api_key
            .account_ids
            .retain(|account_id| valid_scope_account_ids.contains(account_id));
        if api_key.account_ids != before {
            changed = true;
        }
    }

    // Re-normalize rules against pruned membership.
    let original_custom_routing_rules = std::mem::take(&mut collection.custom_routing_rules);
    let normalized_custom_routing_rules = normalize_custom_routing_rules(
        original_custom_routing_rules.clone(),
        &collection.account_ids,
    );
    if normalized_custom_routing_rules != original_custom_routing_rules {
        changed = true;
    }
    collection.custom_routing_rules = normalized_custom_routing_rules;

    let original_account_model_rules = std::mem::take(&mut collection.account_model_rules);
    let normalized_account_model_rules = normalize_account_model_rules(
        original_account_model_rules.clone(),
        &collection.account_ids,
    );
    if normalized_account_model_rules != original_account_model_rules {
        changed = true;
    }
    collection.account_model_rules = normalized_account_model_rules;

    Ok((changed, valid_account_ids))
}

async fn ensure_runtime_loaded_without_start_with_profile_restore(
    restore_disabled_profiles: bool,
) -> Result<(), String> {
    loop {
        {
            let runtime = gateway_runtime().lock().await;
            if runtime.loaded {
                drop(runtime);
                ensure_stats_maintenance_started();
                ensure_collection_account_sanitize_started();
                return Ok(());
            }
        }
        if GATEWAY_RUNTIME_LOAD_IN_FLIGHT
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            break;
        }
        let notified = gateway_runtime_load_notify().notified();
        if GATEWAY_RUNTIME_LOAD_IN_FLIGHT.load(Ordering::SeqCst) {
            notified.await;
        }
    }

    // Load only the compact collection/stat snapshot before publishing runtime. SQLite migration
    // and month-event rebuilding start independently after the base state becomes available.
    let load_guard = GatewayRuntimeLoadGuard;
    tauri::async_runtime::spawn_blocking(move || {
        let _load_guard = load_guard;
        let loaded_collection = load_collection_from_disk()?;
        refresh_api_service_experimental_model_ids();
        let mut next_collection = loaded_collection;
        let mut persist_after_load = false;

        if next_collection.is_none() {
            next_collection = Some(CodexLocalAccessCollection {
                enabled: false,
                port: allocate_initial_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
                api_key: generate_local_api_key(),
                api_keys: Vec::new(),
                access_scope: CodexLocalAccessScope::Localhost,
                client_base_url_host: CodexLocalAccessClientBaseUrlHost::default(),
                image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
                image_generation_account_policies: HashMap::new(),
                gateway_mode: CodexLocalAccessGatewayMode::default(),
                upstream_proxy_url: None,
                routing_strategy: CodexLocalAccessRoutingStrategy::default(),
                custom_routing_rules: Vec::new(),
                account_model_rules: Vec::new(),
                model_aliases: Vec::new(),
                model_pricing_version: DEFAULT_MODEL_PRICING_VERSION,
                model_pricings: Vec::new(),
                excluded_models: Vec::new(),
                session_affinity: true,
                session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
                session_affinity_default_enabled_migrated: true,
                responses_websockets_enabled: false,
                max_retry_credentials: 0,
                max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
                timeouts: CodexLocalAccessTimeouts::default(),
                active_timeout_preset_id: "long_wait".to_string(),
                timeout_presets: Vec::new(),
                disable_cooling: false,
                restrict_free_accounts: true,
                debug_logs: true,
                immediate_sse_response: false,
                max_concurrent_image_requests: 1,
                bound_oauth_account_id: None,
                bound_oauth_quota_reserve: None,
                account_ids: Vec::new(),
                created_at: now_ms(),
                updated_at: now_ms(),
            });
            persist_after_load = true;
        }

        let mut pricing_book_resealed = false;
        if let Some(collection) = next_collection.as_mut() {
            let previous_pricing_version = collection.model_pricing_version;
            // Cold start must not list/decrypt every Codex account before publishing
            // runtime. Membership pruning runs in ensure_collection_account_sanitize_started.
            let changed = sanitize_collection_structure(collection)?;
            pricing_book_resealed = previous_pricing_version < DEFAULT_MODEL_PRICING_VERSION;
            persist_after_load = persist_after_load || changed;
        }

        if persist_after_load {
            if let Some(collection) = next_collection.as_ref() {
                save_collection_to_disk(collection)?;
            }
        }

        let mut loaded_stats = load_stats_snapshot_from_disk()?;
        sort_usage_accounts(&mut loaded_stats.accounts);
        sort_usage_models(&mut loaded_stats.models);
        sort_usage_api_keys(&mut loaded_stats.api_keys);
        loaded_stats.events.sort_by_key(|event| event.timestamp);
        if loaded_stats.events.len() > STATE_RECENT_USAGE_EVENT_LIMIT {
            let remove_count = loaded_stats.events.len() - STATE_RECENT_USAGE_EVENT_LIMIT;
            loaded_stats.events.drain(..remove_count);
        }

        {
            let mut runtime = gateway_runtime().blocking_lock();
            runtime.stats_dirty = false;
            runtime.stats_flush_inflight = false;
            runtime.stats_revision = 0;
            runtime.stats = loaded_stats;
            if let Some(collection) = next_collection.clone() {
                sync_runtime_collection(&mut runtime, collection);
            } else {
                runtime.loaded = true;
                runtime.collection = None;
                runtime.last_error = None;
                prune_prepared_account_cache(&mut runtime, now_ms());
            }
        }

        // After the base runtime is visible, prune stale account membership in background.
        ensure_collection_account_sanitize_started();

        if restore_disabled_profiles
            && next_collection
                .as_ref()
                .is_some_and(|collection| !collection.enabled)
        {
            if let Some(collection) = next_collection.clone() {
                let _ = std::thread::Builder::new()
                    .name("codex-api-profile-restore".to_string())
                    .spawn(move || {
                        if let Err(error) = restore_takeover_profiles_after_disable(&collection) {
                            logger::log_codex_api_warn(&format!(
                                "Codex API 服务处于停用状态，但后台恢复 Live 配置失败: {}",
                                error
                            ));
                        }
                    });
            }
        }

        ensure_stats_maintenance_started();
        if pricing_book_resealed {
            if let (Some(collection), Some(app)) =
                (next_collection, crate::get_app_handle().cloned())
            {
                tauri::async_runtime::spawn(async move {
                    let model_ids = effective_price_book_model_ids(Some(&collection));
                    logger::log_codex_api_info(
                        "Codex API 服务默认价格表已升级，历史估算已转入后台重算",
                    );
                    queue_model_pricing_reprice(app, collection, model_ids).await;
                });
            } else {
                logger::log_codex_api_warn(
                    "Codex API 服务默认价格表已升级，但应用句柄尚未就绪，历史估算将在下次启动后台重算",
                );
            }
        }

        Ok::<_, String>(())
    })
    .await
    .map_err(|e| format!("加载 Codex API 服务配置/统计任务失败: {}", e))??;

    Ok(())
}

async fn ensure_runtime_loaded_without_start() -> Result<(), String> {
    ensure_runtime_loaded_without_start_with_profile_restore(true).await
}

async fn ensure_runtime_loaded() -> Result<(), String> {
    ensure_runtime_loaded_without_start().await?;
    ensure_bound_oauth_quota_monitor_started();

    let should_start = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(|collection| collection.enabled)
            .unwrap_or(false)
    };

    if should_start {
        ensure_gateway_matches_runtime().await?;
        ensure_local_access_profile_takeovers_from_runtime().await?;
        trigger_bound_oauth_quota_refresh_in_background(
            "API 服务运行态检查",
            BOUND_OAUTH_QUOTA_RESERVE_REFRESH_INTERVAL,
        );
    }

    Ok(())
}

async fn ensure_runtime_loaded_for_app_startup() -> Result<(), String> {
    ensure_runtime_loaded_without_start_with_profile_restore(false).await?;
    ensure_bound_oauth_quota_monitor_started();

    let should_start = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(|collection| collection.enabled)
            .unwrap_or(false)
    };

    if should_start {
        ensure_gateway_matches_runtime().await?;
        let collection = {
            let runtime = gateway_runtime().lock().await;
            runtime.collection.clone()
        };
        if let Some(collection) = collection.as_ref() {
            if local_access_profile_takeovers_need_websocket_sync(collection) {
                ensure_local_access_profile_takeovers_from_runtime().await?;
            }
        }
        trigger_bound_oauth_quota_refresh_in_background(
            "API 服务启动恢复",
            BOUND_OAUTH_QUOTA_RESERVE_REFRESH_INTERVAL,
        );
    }

    Ok(())
}

async fn ensure_gateway_matches_runtime() -> Result<(), String> {
    let result = {
        let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
        ensure_gateway_matches_runtime_locked().await
    };
    if result.is_ok() && GATEWAY_STOP_REQUESTS.load(Ordering::SeqCst) == 0 {
        let collection = {
            let runtime = gateway_runtime().lock().await;
            runtime
                .collection
                .clone()
                .filter(|collection| collection.enabled && runtime.running)
        };
        if let Some(collection) = collection {
            trigger_sidecar_account_refresh_in_background(collection);
        }
    }
    result
}

fn reload_gateway_in_background<F>(reason: &'static str, reload: F)
where
    F: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        match reload.await {
            Ok(()) => {
                let mut runtime = gateway_runtime().lock().await;
                runtime.last_error = None;
                logger::log_codex_api_info(&format!(
                    "[CodexLocalAccess] 后台网关重载完成: {}",
                    reason
                ));
            }
            Err(error) => {
                let mut runtime = gateway_runtime().lock().await;
                runtime.last_error = Some(error.clone());
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 后台网关重载失败: reason={}, error={}",
                    reason, error
                ));
            }
        }
    });
}

pub fn trigger_gateway_reload_in_background(reason: &'static str) {
    reload_gateway_in_background(reason, ensure_runtime_loaded());
}

pub fn collection_contains_account(account_id: &str) -> bool {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return false;
    }
    load_collection_from_disk()
        .ok()
        .flatten()
        .is_some_and(|collection| {
            collection
                .account_ids
                .iter()
                .any(|item| item.trim() == account_id)
        })
}

fn bound_oauth_quota_refresh_control() -> &'static TokioMutex<BoundOauthQuotaRefreshControl> {
    BOUND_OAUTH_QUOTA_REFRESH_CONTROL
        .get_or_init(|| TokioMutex::new(BoundOauthQuotaRefreshControl::default()))
}

async fn bound_oauth_quota_refresh_target() -> Option<String> {
    let runtime = gateway_runtime().lock().await;
    let collection = runtime.collection.as_ref()?;
    if !runtime.running || !collection.enabled || collection.bound_oauth_quota_reserve.is_none() {
        return None;
    }
    normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())
}

async fn refresh_bound_oauth_quota_if_due(reason: &'static str, min_interval: Duration) {
    let Some(account_id) = bound_oauth_quota_refresh_target().await else {
        return;
    };

    {
        let mut control = bound_oauth_quota_refresh_control().lock().await;
        if control.in_flight {
            return;
        }
        if control.last_account_id.as_deref() == Some(account_id.as_str())
            && control
                .last_started_at
                .map(|started_at| started_at.elapsed() < min_interval)
                .unwrap_or(false)
        {
            return;
        }
        control.in_flight = true;
        control.last_account_id = Some(account_id.clone());
        control.last_started_at = Some(Instant::now());
    }

    let result = codex_quota::refresh_account_quota(&account_id).await;
    {
        let mut control = bound_oauth_quota_refresh_control().lock().await;
        control.in_flight = false;
    }

    match result {
        Ok(_) => logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 绑定 OAuth 配额刷新完成: reason={}, account_id={}",
            reason, account_id
        )),
        Err(error) => logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 绑定 OAuth 配额刷新失败: reason={}, account_id={}, error={}",
            reason, account_id, error
        )),
    }
}

fn trigger_bound_oauth_quota_refresh_in_background(reason: &'static str, min_interval: Duration) {
    tauri::async_runtime::spawn(refresh_bound_oauth_quota_if_due(reason, min_interval));
}

fn ensure_bound_oauth_quota_monitor_started() {
    if BOUND_OAUTH_QUOTA_MONITOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(BOUND_OAUTH_QUOTA_RESERVE_MONITOR_TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            refresh_bound_oauth_quota_if_due(
                "API 服务定时监控",
                BOUND_OAUTH_QUOTA_RESERVE_REFRESH_INTERVAL,
            )
            .await;
        }
    });
}

pub async fn reevaluate_bound_oauth_quota_reserve_after_refresh(
    account_id: &str,
    refresh_succeeded: bool,
) {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return;
    }
    let refreshed_account = refresh_succeeded.then(|| codex_account::load_account(account_id)).flatten();
    if let Ok(mut failures) = bound_oauth_quota_refresh_failures().lock() {
        if refresh_succeeded {
            failures.remove(account_id);
        } else {
            failures.insert(account_id.to_string());
        }
    }
    let (matching_collection, active_collection) = {
        let mut runtime = gateway_runtime().lock().await;
        let collection = runtime
            .collection
            .as_ref()
            .filter(|collection| {
                collection.bound_oauth_quota_reserve.is_some()
                    && normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())
                        .as_deref()
                        == Some(account_id)
            })
            .cloned();
        if collection.is_some() {
            runtime.prepared_accounts.remove(account_id);
        }
        if let Some(account) = refreshed_account.as_ref() {
            sync_runtime_quota_cooldowns(&mut runtime, std::slice::from_ref(account), now_ms());
        }
        (collection, runtime.collection.clone())
    };

    if let Some(collection) = active_collection {
        if collection_gateway_mode(&collection) == CodexLocalAccessGatewayMode::Sidecar {
            if let Err(error) = write_sidecar_quota_pool_state(&collection) {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] API 服务额度池快照热更新失败: {}",
                    error
                ));
            }
        }
    }

    if let Some(collection) = matching_collection {
        if collection_gateway_mode(&collection) == CodexLocalAccessGatewayMode::Sidecar {
            if let Err(error) = write_sidecar_quota_reserve_state(&collection) {
                let mut runtime = gateway_runtime().lock().await;
                runtime.last_error = Some(error.clone());
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 绑定 OAuth 配额快照热更新失败: {}",
                    error
                ));
            }
        }
    }
}

fn refresh_gateway_process_status(runtime: &mut GatewayRuntime) {
    if !runtime.running {
        return;
    }
    let Some(child) = runtime.sidecar_child.as_mut() else {
        return;
    };
    let message = match child.try_wait() {
        Ok(Some(status)) => Some(format!("API 服务 sidecar 已退出: {}", status)),
        Ok(None) => None,
        Err(error) => Some(format!("检查 API 服务 sidecar 状态失败: {}", error)),
    };
    let Some(message) = message else {
        return;
    };
    log_gateway_mode_warn(CodexLocalAccessGatewayMode::Sidecar, &message);
    runtime.running = false;
    runtime.actual_port = None;
    runtime.actual_bind_host = None;
    runtime.sidecar_config_fingerprint = None;
    runtime.last_error = Some(message);
    runtime.sidecar_child = None;
}

fn is_retryable_sidecar_bind_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("listen tcp")
        && (normalized.contains("bind:")
            || normalized.contains("address already in use")
            || normalized.contains("access permissions")
            || normalized.contains("permission denied"))
}

async fn persist_recovered_local_access_port(new_port: u16) -> Result<u16, String> {
    let mut collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .ok_or_else(|| "本地接入集合尚未创建".to_string())?
    };
    let previous_port = collection.port;
    if previous_port == new_port {
        return Ok(previous_port);
    }

    collection.port = new_port;
    collection.updated_at = now_ms();
    let collection_to_save = collection.clone();
    tauri::async_runtime::spawn_blocking(move || save_collection_to_disk(&collection_to_save))
        .await
        .map_err(|error| format!("保存端口恢复配置任务失败: {}", error))??;

    let mut runtime = gateway_runtime().lock().await;
    sync_runtime_collection(&mut runtime, collection);
    Ok(previous_port)
}

async fn ensure_gateway_matches_runtime_locked() -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..=LOCAL_ACCESS_PORT_RECOVERY_ATTEMPTS {
        match ensure_gateway_matches_runtime_once_locked().await {
            Ok(()) => {
                if attempt > 0 {
                    logger::log_codex_api_info(&format!(
                        "[CodexLocalAccess] API 服务已通过随机端口恢复启动: attempts={}",
                        attempt
                    ));
                }
                return Ok(());
            }
            Err(error) => {
                let can_retry = attempt < LOCAL_ACCESS_PORT_RECOVERY_ATTEMPTS
                    && is_retryable_sidecar_bind_error(&error);
                last_error = Some(error.clone());
                if !can_retry {
                    return Err(error);
                }

                let (bind_host, current_port) = {
                    let runtime = gateway_runtime().lock().await;
                    let collection = runtime.collection.as_ref();
                    (
                        collection
                            .map(|item| bind_host_for_collection(item))
                            .unwrap_or(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)
                            .to_string(),
                        collection.map(|item| item.port).unwrap_or_default(),
                    )
                };
                let fallback_port = match allocate_random_local_port(&bind_host) {
                    Ok(port) if port != current_port => port,
                    Ok(_) => {
                        logger::log_codex_api_warn(&format!(
                            "[CodexLocalAccess] 随机端口与原端口相同，继续重试: attempt={}",
                            attempt + 1
                        ));
                        continue;
                    }
                    Err(port_error) => {
                        return Err(format!("{}；分配随机端口重试失败: {}", error, port_error));
                    }
                };
                let previous_port = persist_recovered_local_access_port(fallback_port).await?;
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] sidecar 端口绑定失败，将自动更换端口重试: old_port={}, new_port={}, attempt={}, error={}",
                    previous_port,
                    fallback_port,
                    attempt + 1,
                    error
                ));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "API 服务启动失败".to_string()))
}
