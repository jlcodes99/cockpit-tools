// Codex Local Access：Public local-access state, account and routing command implementations。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn new_local_access_collection() -> Result<CodexLocalAccessCollection, String> {
    let now = now_ms();
    Ok(CodexLocalAccessCollection {
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
        active_timeout_preset_id: BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID.to_string(),
        timeout_presets: Vec::new(),
        disable_cooling: false,
        restrict_free_accounts: true,
        debug_logs: true,
        immediate_sse_response: false,
        max_concurrent_image_requests: 1,
        bound_oauth_account_id: None,
        bound_oauth_quota_reserve: None,
        account_ids: Vec::new(),
        created_at: now,
        updated_at: now,
    })
}

fn append_eligible_local_access_account_ids(
    current_account_ids: &[String],
    requested_account_ids: Vec<String>,
    accounts: &[CodexAccount],
    restrict_free_accounts: bool,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<CodexLocalAccessAppendAccountSkipped>,
) {
    let account_by_id: HashMap<&str, &CodexAccount> = accounts
        .iter()
        .map(|account| (account.id.as_str(), account))
        .collect();
    let mut next_account_ids = current_account_ids.to_vec();
    let mut current_ids: HashSet<String> = current_account_ids.iter().cloned().collect();
    let mut requested_seen = HashSet::new();
    let mut synced_account_ids = Vec::new();
    let mut added_account_ids = Vec::new();
    let mut skipped_accounts = Vec::new();

    for account_id in requested_account_ids {
        let account_id = account_id.trim().to_string();
        if account_id.is_empty() || !requested_seen.insert(account_id.clone()) {
            continue;
        }
        let Some(account) = account_by_id.get(account_id.as_str()).copied() else {
            skipped_accounts.push(CodexLocalAccessAppendAccountSkipped {
                account_id,
                reason: "not_found".to_string(),
            });
            continue;
        };
        if let Some(reason) = local_access_ineligible_reason(account, restrict_free_accounts) {
            skipped_accounts.push(CodexLocalAccessAppendAccountSkipped {
                account_id,
                reason: reason.to_string(),
            });
            continue;
        }

        synced_account_ids.push(account_id.clone());
        if current_ids.insert(account_id.clone()) {
            next_account_ids.push(account_id.clone());
            added_account_ids.push(account_id);
        }
    }

    (
        next_account_ids,
        synced_account_ids,
        added_account_ids,
        skipped_accounts,
    )
}

fn apply_account_usage_priority_ids(
    collection: &mut CodexLocalAccessCollection,
    backup_account_ids: Option<&[String]>,
    preferred_account_ids: Option<&[String]>,
) {
    let account_set: HashSet<&str> = collection.account_ids.iter().map(String::as_str).collect();
    let normalize_ids = |account_ids: &[String]| {
        account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty() && account_set.contains(id.as_str()))
            .collect::<HashSet<String>>()
    };
    let backup_set = backup_account_ids.map(normalize_ids);
    let preferred_set = preferred_account_ids.map(normalize_ids);

    let mut seen = HashSet::new();
    for rule in &mut collection.custom_routing_rules {
        if !account_set.contains(rule.account_id.as_str()) {
            continue;
        }
        if let Some(backup_set) = backup_set.as_ref() {
            rule.is_backup = backup_set.contains(rule.account_id.as_str());
            if rule.is_backup {
                rule.is_preferred = false;
            }
        }
        if let Some(preferred_set) = preferred_set.as_ref() {
            rule.is_preferred = preferred_set.contains(rule.account_id.as_str());
            if rule.is_preferred {
                rule.is_backup = false;
            }
        }
        seen.insert(rule.account_id.clone());
    }

    for account_id in &collection.account_ids {
        if seen.contains(account_id) {
            continue;
        }
        let is_backup = backup_set
            .as_ref()
            .is_some_and(|ids| ids.contains(account_id.as_str()));
        let is_preferred = preferred_set
            .as_ref()
            .is_some_and(|ids| ids.contains(account_id.as_str()));
        if !is_backup && !is_preferred {
            continue;
        }
        collection
            .custom_routing_rules
            .push(CodexLocalAccessCustomRoutingRule {
                account_id: account_id.clone(),
                priority: CUSTOM_ROUTING_PRIORITY_MIN,
                weight: CUSTOM_ROUTING_WEIGHT_MIN,
                is_backup: is_backup && !is_preferred,
                is_preferred,
            });
        seen.insert(account_id.clone());
    }

    collection.custom_routing_rules = normalize_custom_routing_rules(
        collection.custom_routing_rules.clone(),
        &collection.account_ids,
    );
}

pub async fn save_local_access_accounts(
    account_ids: Vec<String>,
    restrict_free_accounts: bool,
    backup_account_ids: Option<Vec<String>>,
    preferred_account_ids: Option<Vec<String>>,
    session_affinity: Option<bool>,
    session_affinity_ttl_ms: Option<i64>,
    image_generation_account_policies:
        Option<HashMap<String, CodexLocalAccessImageGenerationPolicy>>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let mut collection = {
        let runtime = gateway_runtime().lock().await;
        match runtime.collection.clone() {
            Some(collection) => collection,
            None => new_local_access_collection()?,
        }
    };

    let accounts = codex_account::list_accounts_checked()?;
    let valid_account_ids: HashSet<String> = accounts
        .iter()
        .filter(|account| is_local_access_eligible_account(account, restrict_free_accounts))
        .map(|account| account.id.clone())
        .collect();

    let mut next_account_ids = Vec::new();
    let mut seen = HashSet::new();
    for account_id in account_ids {
        if !valid_account_ids.contains(&account_id) {
            continue;
        }
        if seen.insert(account_id.clone()) {
            next_account_ids.push(account_id);
        }
    }

    collection.restrict_free_accounts = restrict_free_accounts;
    collection.account_ids = next_account_ids;
    if let Some(policies) = image_generation_account_policies {
        collection.image_generation_account_policies = policies
            .into_iter()
            .map(|(account_id, policy)| (account_id.trim().to_string(), policy))
            .filter(|(account_id, _)| collection.account_ids.iter().any(|id| id == account_id))
            .collect();
    } else {
        collection
            .image_generation_account_policies
            .retain(|account_id, _| collection.account_ids.iter().any(|id| id == account_id));
    }
    if let Some(session_affinity) = session_affinity {
        collection.session_affinity = session_affinity;
        collection.session_affinity_default_enabled_migrated = true;
    }
    if let Some(session_affinity_ttl_ms) = session_affinity_ttl_ms {
        collection.session_affinity_ttl_ms =
            session_affinity_ttl_ms.clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
    }
    collection.updated_at = now_ms();
    let (mut changed, _) = sanitize_collection_with_accounts(&mut collection, &accounts)?;
    if backup_account_ids.is_some() || preferred_account_ids.is_some() {
        let before = collection.custom_routing_rules.clone();
        apply_account_usage_priority_ids(
            &mut collection,
            backup_account_ids.as_deref(),
            preferred_account_ids.as_deref(),
        );
        if collection.custom_routing_rules != before {
            changed = true;
        }
    }
    if changed {
        collection.updated_at = now_ms();
    }
    save_collection_to_disk(&collection)?;

    let should_reload_gateway = collection.enabled;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    if should_reload_gateway {
        trigger_gateway_reload_in_background("保存 API 服务账号集合");
    }
    snapshot_state_without_gateway_reload().await
}

pub async fn append_local_access_accounts(
    account_ids: Vec<String>,
) -> Result<CodexLocalAccessAppendAccountsResult, String> {
    ensure_runtime_loaded_without_start().await?;

    let existing_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let restrict_free_accounts = existing_collection
        .as_ref()
        .map(|collection| collection.restrict_free_accounts)
        .unwrap_or(true);
    let accounts = codex_account::list_accounts_checked()?;
    let current_account_ids = existing_collection
        .as_ref()
        .map(|collection| collection.account_ids.as_slice())
        .unwrap_or(&[]);
    let (next_account_ids, synced_account_ids, added_account_ids, skipped_accounts) =
        append_eligible_local_access_account_ids(
            current_account_ids,
            account_ids,
            &accounts,
            restrict_free_accounts,
        );

    if !added_account_ids.is_empty() {
        let mut collection = match existing_collection {
            Some(collection) => collection,
            None => new_local_access_collection()?,
        };
        collection.account_ids = next_account_ids;
        collection.updated_at = now_ms();
        let (changed, _) = sanitize_collection_with_accounts(&mut collection, &accounts)?;
        if changed {
            collection.updated_at = now_ms();
        }
        save_collection_to_disk(&collection)?;

        let should_reload_gateway = collection.enabled;
        {
            let mut runtime = gateway_runtime().lock().await;
            sync_runtime_collection(&mut runtime, collection);
        }
        if should_reload_gateway {
            trigger_gateway_reload_in_background("导入账号同步加入 API 服务");
        }
    }

    Ok(CodexLocalAccessAppendAccountsResult {
        state: snapshot_state_without_gateway_reload().await?,
        synced_account_ids,
        added_account_ids,
        skipped_accounts,
    })
}

pub async fn update_local_access_routing_strategy(
    strategy: CodexLocalAccessRoutingStrategy,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.routing_strategy == strategy {
        return snapshot_state().await;
    }

    collection.routing_strategy = strategy;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_custom_routing(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.custom_routing_rules =
        normalize_custom_routing_rules(rules, &collection.account_ids);
    collection.routing_strategy = CodexLocalAccessRoutingStrategy::Custom;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    let should_reload_gateway = collection.enabled;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    if should_reload_gateway {
        trigger_gateway_reload_in_background("删除 API 服务账号集合引用");
    }
    snapshot_state_without_gateway_reload().await
}

pub async fn update_local_access_account_model_rules(
    rules: Vec<CodexLocalAccessAccountModelRule>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.account_model_rules = normalize_account_model_rules(rules, &collection.account_ids);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_model_rules(
    model_aliases: Vec<CodexLocalAccessModelAlias>,
    excluded_models: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.model_aliases = normalize_model_aliases(model_aliases);
    collection.excluded_models = normalize_model_rule_list(excluded_models);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_model_pricings(
    app: AppHandle,
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    let normalized_model_pricings = normalize_model_pricings(model_pricings);
    let previous_model_pricings = collection.model_pricings.clone();
    if normalized_model_pricings != collection.model_pricings {
        collection.model_pricing_version = collection
            .model_pricing_version
            .max(DEFAULT_MODEL_PRICING_VERSION)
            .saturating_add(1);
    } else {
        collection.model_pricing_version = collection
            .model_pricing_version
            .max(DEFAULT_MODEL_PRICING_VERSION);
    }
    collection.model_pricings = normalized_model_pricings;
    let changed_model_ids =
        changed_model_pricing_ids(&previous_model_pricings, &collection.model_pricings);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    let reprice_collection = collection.clone();

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    if !changed_model_ids.is_empty() {
        queue_model_pricing_reprice(app, reprice_collection, changed_model_ids).await;
    }
    snapshot_state_without_gateway_reload().await
}

pub async fn reprice_local_access_request_logs() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .ok_or_else(|| "本地接入集合尚未创建".to_string())?
    };

    let mut conn = open_local_access_logs_db()
        .map_err(|e| format!("打开 API 服务请求日志数据库失败: {}", e))?;
    let updated_count = reprice_request_logs_for_collection(&mut conn, &collection)?;
    drop(conn);

    let loaded_stats = rebuild_stats_from_request_logs()?;
    save_stats_to_disk(&loaded_stats)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        runtime.stats = loaded_stats;
        runtime.stats_dirty = false;
        runtime.stats_flush_inflight = false;
    }

    logger::log_codex_api_info(&format!(
        "API 服务请求日志价格重算完成: updated_rows={}, pricing_version={}",
        updated_count,
        collection
            .model_pricing_version
            .max(DEFAULT_MODEL_PRICING_VERSION)
    ));

    snapshot_state_without_gateway_reload().await
}

pub async fn update_local_access_routing_options(
    session_affinity: bool,
    session_affinity_ttl_ms: i64,
    responses_websockets_enabled: bool,
    max_retry_credentials: u16,
    max_retry_interval_ms: u64,
    disable_cooling: bool,
    immediate_sse_response: bool,
    max_concurrent_image_requests: u16,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    let responses_websockets_changed =
        collection.responses_websockets_enabled != responses_websockets_enabled;
    let profile_websocket_sync_needed = !responses_websockets_changed
        && local_access_profile_takeovers_need_websocket_sync(&collection);
    collection.session_affinity = session_affinity;
    collection.session_affinity_default_enabled_migrated = true;
    collection.session_affinity_ttl_ms =
        session_affinity_ttl_ms.clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
    collection.responses_websockets_enabled = responses_websockets_enabled;
    collection.max_retry_credentials =
        max_retry_credentials.min(MAX_RETRY_CREDENTIALS_PER_REQUEST as u16);
    collection.max_retry_interval_ms =
        max_retry_interval_ms.clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS);
    collection.disable_cooling = disable_cooling;
    collection.immediate_sse_response = immediate_sse_response;
    collection.max_concurrent_image_requests =
        max_concurrent_image_requests.clamp(1, MAX_CONCURRENT_IMAGE_REQUESTS_PER_ACCOUNT);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    if responses_websockets_changed || profile_websocket_sync_needed {
        ensure_local_access_profile_takeovers_from_runtime().await?;
    }
    snapshot_state().await
}

pub async fn update_local_access_timeouts(
    timeouts: CodexLocalAccessTimeouts,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.timeouts = timeouts;
    normalize_timeouts(&mut collection.timeouts);
    if let Some(preset_id) = active_timeout_preset_id {
        collection.active_timeout_preset_id = preset_id;
        normalize_active_timeout_preset_id(&mut collection);
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_timeout_presets(
    timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.timeout_presets = timeout_presets;
    normalize_timeout_presets(&mut collection.timeout_presets);
    if let Some(preset_id) = active_timeout_preset_id {
        collection.active_timeout_preset_id = preset_id;
    }
    normalize_active_timeout_preset_id(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    snapshot_state().await
}

pub async fn update_local_access_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let normalized_upstream_proxy_url = validate_upstream_proxy_config(upstream_proxy_url)?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.upstream_proxy_url == normalized_upstream_proxy_url {
        return snapshot_state().await;
    }

    collection.upstream_proxy_url = normalized_upstream_proxy_url;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_gateway_mode(
    gateway_mode: CodexLocalAccessGatewayMode,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.gateway_mode == gateway_mode {
        return snapshot_state().await;
    }

    collection.gateway_mode = gateway_mode;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_debug_logs(
    debug_logs: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.debug_logs == debug_logs {
        return snapshot_state().await;
    }

    collection.debug_logs = debug_logs;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.access_scope == access_scope {
        return snapshot_state().await;
    }

    collection.access_scope = access_scope;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_client_base_url_host(
    client_base_url_host: CodexLocalAccessClientBaseUrlHost,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.client_base_url_host == client_base_url_host {
        return snapshot_state().await;
    }

    collection.client_base_url_host = client_base_url_host;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    let next_collection = collection.clone();

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    if next_collection.enabled {
        ensure_local_access_profile_takeovers(&next_collection).await?;
    }
    snapshot_state().await
}

pub async fn remove_local_access_account(
    account_id: &str,
) -> Result<CodexLocalAccessState, String> {
    remove_local_access_accounts(&[account_id.to_string()]).await
}

pub async fn remove_deleted_accounts_from_local_access_pool(
    account_ids: &[String],
) -> Result<(), String> {
    let remove_ids = account_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if remove_ids.is_empty() {
        return Ok(());
    }

    let Some(mut collection) = load_collection_from_disk()? else {
        return Ok(());
    };

    if !remove_account_refs_from_collection(&mut collection, &remove_ids) {
        return Ok(());
    }

    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    let runtime_loaded = {
        let mut runtime = gateway_runtime().lock().await;
        if runtime.loaded {
            sync_runtime_collection(&mut runtime, collection);
            true
        } else {
            false
        }
    };
    if runtime_loaded {
        reload_gateway_in_background(
            "删除账号后同步 API 服务账号池",
            ensure_gateway_matches_runtime(),
        );
    }

    Ok(())
}

pub async fn remove_local_access_accounts(
    account_ids: &[String],
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return snapshot_state().await;
    };

    let remove_ids = account_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect::<HashSet<_>>();
    if remove_ids.is_empty() {
        return snapshot_state().await;
    }

    let refs_changed = remove_account_refs_from_collection(&mut collection, &remove_ids);
    let (changed, _) = sanitize_collection(&mut collection)?;
    if !refs_changed && !changed {
        return snapshot_state().await;
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn rotate_local_access_api_key() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    normalize_collection_api_keys(&mut collection);
    let now = now_ms();
    let primary_id = collection
        .api_keys
        .iter()
        .find(|item| item.enabled)
        .or_else(|| collection.api_keys.first())
        .map(|item| item.id.clone());
    if let Some(primary_id) = primary_id {
        if let Some(api_key) = collection
            .api_keys
            .iter_mut()
            .find(|item| item.id == primary_id)
        {
            api_key.key = generate_local_api_key();
            api_key.updated_at = now;
            api_key.last_used_at = None;
            collection.api_key = api_key.key.clone();
        }
    } else {
        collection.api_key = generate_local_api_key();
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn create_local_access_api_key(
    label: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    collection
        .api_keys
        .push(build_local_access_api_key(label.as_deref()));
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn set_local_access_api_key_account_priority(
    api_key_id: String,
    account_id: String,
    pinned: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    let api_key_id = api_key_id.trim();
    let Some(index) = collection
        .api_keys
        .iter()
        .position(|item| item.id == api_key_id)
    else {
        return Err("API Key 不存在".to_string());
    };

    let api_key = &collection.api_keys[index];
    if api_key_inherits_account_pool(api_key)
        || api_key_has_fixed_account_scope(&collection, api_key)
    {
        return Err("仅自定义账号池的 API Key 支持置顶账号".to_string());
    }
    let account_id = normalize_optional_account_ref(Some(&account_id))
        .ok_or_else(|| "请选择要置顶的账号".to_string())?;
    if pinned
        && !api_key
            .account_ids
            .iter()
            .any(|selected_id| selected_id == &account_id)
    {
        return Err("置顶账号不在当前 API Key 的自定义账号池中".to_string());
    }

    let api_key = &mut collection.api_keys[index];
    let mut priority_account_ids = normalize_account_id_list(api_key.priority_account_ids.clone());
    priority_account_ids.retain(|priority_account_id| priority_account_id != &account_id);
    if pinned {
        priority_account_ids.insert(0, account_id);
    }
    api_key.priority_account_ids = priority_account_ids;
    api_key.preferred_account_id = None;
    api_key.updated_at = now_ms();
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    if collection.gateway_mode == CodexLocalAccessGatewayMode::Sidecar {
        let base_dir = local_access_sidecar_dir()?;
        std::fs::create_dir_all(&base_dir)
            .map_err(|error| format!("创建 API 服务 sidecar 目录失败: {}", error))?;
        write_sidecar_api_key_priority_state_in_dir(&collection, &base_dir)?;
    }
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    snapshot_state().await
}

pub async fn update_local_access_api_key(
    api_key_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    model_prefix: Option<String>,
    allowed_models: Option<Vec<String>>,
    excluded_models: Option<Vec<String>>,
    token_limit: Option<u64>,
    account_ids: Option<Vec<String>>,
    inherit_account_pool: Option<bool>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let (maybe_collection, historical_token_used) = {
        let runtime = gateway_runtime().lock().await;
        let historical_token_used = runtime
            .stats
            .api_keys
            .iter()
            .find(|item| item.api_key_id == api_key_id.trim())
            .map(|item| item.usage.total_tokens)
            .unwrap_or_default();
        (runtime.collection.clone(), historical_token_used)
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    let api_key_id = api_key_id.trim();
    let Some(index) = collection
        .api_keys
        .iter()
        .position(|item| item.id == api_key_id)
    else {
        return Err("API Key 不存在".to_string());
    };
    let normalized_account_ids = account_ids.map(normalize_account_id_list);
    validate_api_key_account_scope_update(
        &collection,
        &collection.api_keys[index],
        normalized_account_ids.as_deref(),
        inherit_account_pool,
    )?;
    if let Some(label) = label {
        collection.api_keys[index].label = normalize_api_key_label(Some(label.as_str()), "API Key");
    }
    if let Some(enabled) = enabled {
        collection.api_keys[index].enabled = enabled;
    }
    if model_prefix.is_some() {
        collection.api_keys[index].model_prefix = normalize_model_prefix_value(model_prefix);
    }
    if let Some(allowed_models) = allowed_models {
        collection.api_keys[index].allowed_models = normalize_model_rule_list(allowed_models);
    }
    if let Some(excluded_models) = excluded_models {
        collection.api_keys[index].excluded_models = normalize_model_rule_list(excluded_models);
    }
    if let Some(token_limit) = token_limit {
        if token_limit > 0 && collection.api_keys[index].token_limit.is_none() {
            collection.api_keys[index].token_used = collection.api_keys[index]
                .token_used
                .max(historical_token_used);
        }
        collection.api_keys[index].token_limit = (token_limit > 0).then_some(token_limit);
    }
    if let Some(account_ids) = normalized_account_ids {
        if inherit_account_pool.is_none() {
            collection.api_keys[index].inherit_account_pool = Some(account_ids.is_empty());
        }
        collection.api_keys[index].account_ids = account_ids;
    }
    if let Some(inherit_account_pool) = inherit_account_pool {
        collection.api_keys[index].inherit_account_pool = Some(inherit_account_pool);
    }
    collection.api_keys[index].updated_at = now_ms();
    if !collection.api_keys.iter().any(|item| item.enabled) {
        collection.api_keys[index].enabled = true;
    }
    normalize_collection_api_keys(&mut collection);
    let _ = sanitize_collection(&mut collection)?;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn rotate_local_access_named_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    let api_key_id = api_key_id.trim();
    let Some(api_key) = collection
        .api_keys
        .iter_mut()
        .find(|item| item.id == api_key_id)
    else {
        return Err("API Key 不存在".to_string());
    };
    api_key.key = generate_local_api_key();
    api_key.updated_at = now_ms();
    api_key.last_used_at = None;
    normalize_collection_api_keys(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn delete_local_access_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    if collection.api_keys.len() <= 1 {
        return Err("至少保留一个 API Key".to_string());
    }
    let api_key_id = api_key_id.trim();
    let before_len = collection.api_keys.len();
    collection.api_keys.retain(|item| item.id != api_key_id);
    if collection.api_keys.len() == before_len {
        return Err("API Key 不存在".to_string());
    }
    normalize_collection_api_keys(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_bound_oauth_account(
    bound_oauth_account_id: Option<String>,
    bound_oauth_quota_reserve: Option<CodexLocalAccessQuotaReserve>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    let normalized_bound_id = normalize_optional_account_ref(bound_oauth_account_id.as_deref());
    let has_bound_oauth = normalized_bound_id.is_some();
    let bound_oauth_quota_reserve =
        validate_bound_oauth_quota_reserve(bound_oauth_quota_reserve, has_bound_oauth)?;
    if let Some(bound_id) = normalized_bound_id {
        let bound_account = validate_local_access_bound_oauth_account(&bound_id)?;
        // API Service 绑定动作复用客户端启动的 OAuth 预检：仅在 access_token
        // 不可用时使用 refresh_token 换取最新链，刷新失败则由统一错误协议进入重新授权流程。
        let bound_account = match codex_account::prepare_account_for_instance_launch_preflight(
            &bound_account.id,
        )
        .await
        {
            Ok(account) => account,
            Err(error) => {
                return Err(codex_account::format_account_switch_error(
                    &bound_id, error,
                ));
            }
        };
        collection.bound_oauth_account_id = Some(bound_account.id);
        collection.bound_oauth_quota_reserve = bound_oauth_quota_reserve;
    } else {
        collection.bound_oauth_account_id = None;
        collection.bound_oauth_quota_reserve = None;
    }
    // 绑定 OAuth：改前逻辑，默认全开生图，不启用本地网关 ImagesOnly 绕路。
    collection.image_generation_mode = CodexLocalAccessImageGenerationMode::Enabled;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    let bound_account_id_for_quota_reserve = collection.bound_oauth_account_id.clone();

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
        if let Some(bound_account_id) = bound_account_id_for_quota_reserve.as_deref() {
            runtime.prepared_accounts.remove(bound_account_id);
        }
    }

    ensure_gateway_matches_runtime().await?;
    ensure_local_access_profile_takeovers_from_runtime().await?;
    snapshot_state().await
}

pub async fn clear_local_access_stats() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    if let Err(error) = clear_local_access_usage_events_db() {
        logger::log_codex_api_warn(&format!(
            "清空 API 服务请求日志失败，继续清空内存统计: {}",
            error
        ));
    }

    let cleared = empty_stats_snapshot();
    {
        let mut runtime = gateway_runtime().lock().await;
        runtime.stats = cleared;
        runtime.stats_dirty = true;
    }
    schedule_stats_flush_if_needed().await;

    snapshot_state().await
}

pub async fn prepare_local_access_gateway_for_restart() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    stop_all_sidecar_processes_for_app_shutdown().await?;

    let runtime = gateway_runtime().lock().await;
    Ok(build_state_snapshot(&runtime))
}

/// 仅重启当前 API 服务 Sidecar，保留账号集合、API Key 和持久化配置。
/// Sidecar 进程内的连接、会话和调度临时状态会随进程退出而清理。
pub async fn restart_local_access_sidecar() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    }
    .ok_or_else(|| "API 服务集合尚未创建".to_string())?;

    if !collection.enabled {
        return Err("API 服务当前未启用，无法重启 Sidecar".to_string());
    }

    logger::log_codex_api_info("[CodexLocalAccess] 执行 API 服务 Sidecar 兜底重启");
    stop_gateway().await;
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn kill_local_access_port_processes() -> Result<CodexLocalAccessPortCleanupResult, String>
{
    if let Err(err) = ensure_runtime_loaded_without_start().await {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 清理端口前加载配置失败: {}",
            err
        ));
        return Err(err);
    }

    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    }
    .ok_or_else(|| "API 服务集合尚未创建".to_string())?;

    stop_gateway().await;

    let killed_count = match process::kill_port_processes(collection.port) {
        Ok(count) => count as u32,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 清理旧端口进程失败，将继续尝试启动并准备随机端口兜底: port={}, error={}",
                collection.port, error
            ));
            0
        }
    };

    if collection.enabled {
        ensure_gateway_matches_runtime().await?;
    }

    let state = snapshot_state().await?;
    Ok(CodexLocalAccessPortCleanupResult {
        killed_count,
        state,
    })
}

pub async fn update_local_access_port(port: u16) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    ensure_local_port_available(
        bind_host_for_collection(&collection),
        port,
        Some(collection.port),
    )?;
    if collection.port == port {
        return snapshot_state().await;
    }

    collection.port = port;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn set_local_access_enabled(enabled: bool) -> Result<CodexLocalAccessState, String> {
    if enabled {
        advance_gateway_lifecycle_generation();
        ensure_runtime_loaded().await?;
    } else {
        ensure_runtime_loaded_without_start().await?;
    }

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.enabled = enabled;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    let next_collection = collection.clone();

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    if enabled {
        ensure_gateway_matches_runtime().await?;
        ensure_local_access_profile_takeovers(&next_collection).await?;
        snapshot_state().await
    } else {
        stop_gateway().await;
        restore_takeover_profiles_after_disable(&next_collection)?;
        snapshot_state_without_gateway_reload().await
    }
}

pub async fn restore_local_access_gateway() {
    if let Err(err) = ensure_runtime_loaded_for_app_startup().await {
        let mut runtime = gateway_runtime().lock().await;
        runtime.loaded = true;
        runtime.last_error = Some(err.clone());
        logger::log_codex_api_warn(&format!("[CodexLocalAccess] 初始化失败: {}", err));
    }
}

#[cfg(target_os = "windows")]
fn close_installed_sidecar_processes_by_path(timeout_secs: u64) -> Result<usize, String> {
    let candidates = sidecar_binary_candidates()?
        .into_iter()
        .filter(|path| path.exists())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(0);
    }
    let closed = process::close_processes_by_exact_exe_paths(&candidates, timeout_secs)?;
    if closed > 0 {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess][sidecar] Windows 更新前已关闭安装目录 sidecar 残留进程: count={}",
            closed
        ));
    }
    Ok(closed)
}

async fn stop_all_sidecar_processes_for_app_shutdown() -> Result<(), String> {
    let mut errors = Vec::new();
    #[cfg(target_os = "windows")]
    let preserve_running_mixed_gateway = has_running_persisted_mixed_model_gateway();

    let stopped_endpoint = stop_gateway().await;
    if let Some(endpoint) = stopped_endpoint {
        if let Err(error) = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await
        {
            errors.push(format!("等待 API 服务 sidecar 释放端口失败: {}", error));
        }
    }

    let provider_endpoints = stop_all_provider_gateways_for_app_shutdown().await;
    for endpoint in provider_endpoints {
        if let Err(error) = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await
        {
            errors.push(format!(
                "等待 provider gateway sidecar 释放端口失败: bind={}:{} error={}",
                endpoint.bind_host, endpoint.port, error
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if !preserve_running_mixed_gateway {
            if let Err(error) = close_installed_sidecar_processes_by_path(5) {
                errors.push(format!("关闭安装目录 sidecar 残留进程失败: {}", error));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub async fn shutdown_local_access_gateway_for_app_exit() {
    if let Err(error) = stop_all_sidecar_processes_for_app_shutdown().await {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 应用退出时关闭 sidecar 失败: {}",
            error
        ));
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}
