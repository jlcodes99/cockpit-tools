// Codex Local Access：Sidecar paths, auth manifests, API-key scopes and upstream resolution。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn build_lan_base_url(port: u16) -> Option<String> {
    resolve_primary_lan_ipv4().map(|addr| format!("http://{addr}:{port}/v1"))
}

fn sidecar_config_fingerprint(config_content: &str, manifest_content: &str) -> String {
    let stable_config_content = stable_sidecar_config_for_fingerprint(config_content);
    let stable_manifest_content = stable_sidecar_manifest_for_fingerprint(manifest_content);
    let mut hasher = Sha1::new();
    hasher.update(stable_config_content.as_bytes());
    hasher.update(b"\n--manifest--\n");
    hasher.update(stable_manifest_content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn sidecar_localized_messages(key: &str) -> Value {
    let values = SIDECAR_MESSAGE_LOCALES
        .iter()
        .map(|locale| {
            (
                (*locale).to_string(),
                Value::String(crate::modules::i18n::translate(locale, key, &[])),
            )
        })
        .collect::<Map<String, Value>>();
    Value::Object(values)
}

fn stable_sidecar_config_for_fingerprint(config_content: &str) -> String {
    let Ok(mut config) = serde_json::from_str::<Value>(config_content) else {
        return config_content.to_string();
    };
    if let Some(config) = config.as_object_mut() {
        // CLIProxyAPI watches the config file and applies payload defaults to new
        // requests. Excluding this hot-reloadable field keeps active streams alive
        // when the API service speed changes.
        config.remove("payload");
    }
    serde_json::to_string(&config).unwrap_or_else(|_| config_content.to_string())
}

fn stable_sidecar_manifest_for_fingerprint(manifest_content: &str) -> String {
    let Ok(mut manifest) = serde_json::from_str::<Value>(manifest_content) else {
        return manifest_content.to_string();
    };
    if let Some(accounts) = manifest.get_mut("accounts").and_then(Value::as_array_mut) {
        for account in accounts {
            if let Some(account) = account.as_object_mut() {
                account.remove("remainingQuota");
                if let Some(reserve) = account
                    .get_mut("quotaReserve")
                    .and_then(Value::as_object_mut)
                {
                    for key in [
                        "snapshotUpdatedAtUnixSeconds",
                        "hourlyRemainingPercent",
                        "weeklyRemainingPercent",
                        "hourlyWindowPresent",
                        "weeklyWindowPresent",
                        "hourlyReserveState",
                        "weeklyReserveState",
                    ] {
                        reserve.remove(key);
                    }
                }
            }
        }
    }
    if let Some(api_keys) = manifest.get_mut("apiKeys").and_then(Value::as_array_mut) {
        for api_key in api_keys {
            if let Some(api_key) = api_key.as_object_mut() {
                // Usage is initialized when the process starts and then maintained in memory.
                // It must not restart active streams when unrelated gateway state is refreshed.
                api_key.remove("tokenUsed");
            }
        }
    }
    serde_json::to_string(&manifest).unwrap_or_else(|_| manifest_content.to_string())
}

#[derive(Debug, Clone)]
struct SidecarLaunchConfig {
    config_path: PathBuf,
    manifest_path: PathBuf,
    quota_reserve_path: PathBuf,
    quota_pool_path: PathBuf,
    fingerprint: String,
    proxy_signature: UpstreamHttpClientSignature,
}

#[derive(Debug, Clone, Default)]
struct SidecarReadySignal {
    host: String,
    port: Option<u16>,
}

#[derive(Debug, Clone, Default)]
struct SidecarStartupDiagnostics {
    ready_seen: bool,
    last_stdout: Option<String>,
    last_stderr: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SidecarUsageDetails {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
    #[serde(default)]
    cached_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
    #[serde(default)]
    token_breakdown: Option<CodexTokenBreakdown>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarUsageEvent {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    alias: String,
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    account_email: String,
    #[serde(default)]
    api_key_id: String,
    #[serde(default)]
    api_key_label: String,
    #[serde(default)]
    client_instance_id: String,
    #[serde(default)]
    request_kind: String,
    #[serde(default)]
    #[serde(alias = "service_tier")]
    service_tier: Option<String>,
    #[serde(default)]
    #[serde(alias = "reasoning_effort")]
    reasoning_effort: Option<String>,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    error_category: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    usage: SidecarUsageDetails,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarAuthResultEvent {
    #[serde(default)]
    api_key_id: String,
    #[serde(default)]
    api_key_label: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    account_email: String,
    #[serde(default)]
    request_kind: String,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    http_status: Option<u16>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    auth_available: Option<bool>,
    #[serde(default)]
    next_retry_at_ms: Option<i64>,
    #[serde(default)]
    auth_state_reason: Option<String>,
    #[serde(default)]
    candidate_auths: usize,
    #[serde(default)]
    scoped_auths: usize,
    #[serde(default)]
    available_auths: usize,
    #[serde(default)]
    unavailable_auths: usize,
    #[serde(default)]
    model_excluded_auths: usize,
    #[serde(default)]
    quota_reserved_auths: usize,
    #[serde(default)]
    image_policy_blocked_auths: usize,
    #[serde(default)]
    account_statuses: Vec<SidecarAccountStatus>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarAccountStatus {
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    account_email: String,
    #[serde(default)]
    available: bool,
    #[serde(default)]
    reason_code: String,
    #[serde(default)]
    reason_message: String,
}

fn local_access_sidecar_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_SIDECAR_DIR))
}

fn provider_gateway_sidecars_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_PROVIDER_GATEWAY_SIDECAR_DIR))
}

fn sidecar_config_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_CONFIG_FILE)
}

fn sidecar_manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_MANIFEST_FILE)
}

fn sidecar_api_key_priority_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_API_KEY_PRIORITY_FILE)
}

fn sidecar_quota_reserve_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_QUOTA_RESERVE_FILE)
}

fn sidecar_quota_pool_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_QUOTA_POOL_FILE)
}

fn sidecar_auths_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_AUTHS_DIR)
}

fn provider_gateway_runtime_store() -> &'static TokioMutex<HashMap<String, ProviderGatewayRuntime>>
{
    PROVIDER_GATEWAY_RUNTIMES.get_or_init(|| TokioMutex::new(HashMap::new()))
}

fn provider_gateway_lifecycle_lock() -> &'static TokioMutex<()> {
    PROVIDER_GATEWAY_LIFECYCLE_LOCK.get_or_init(|| TokioMutex::new(()))
}

fn sidecar_binary_file_names() -> Vec<String> {
    let target = env!("COCKPIT_RUST_TARGET");
    if cfg!(target_os = "windows") {
        vec![
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}.exe"),
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}-{target}.exe"),
        ]
    } else {
        vec![
            CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME.to_string(),
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}-{target}"),
        ]
    }
}

fn push_sidecar_binary_candidates(candidates: &mut Vec<PathBuf>, dir: &Path) {
    for name in sidecar_binary_file_names() {
        let path = dir.join(name);
        if !candidates.iter().any(|candidate| candidate == &path) {
            candidates.push(path);
        }
    }
}

fn sidecar_binary_candidates() -> Result<Vec<PathBuf>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("读取当前程序路径失败: {}", e))?;
    let parent = exe
        .parent()
        .ok_or_else(|| format!("当前程序路径缺少父目录: {}", exe.display()))?;
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_sidecar_dir = manifest_dir.join("../sidecars/cockpit-cliproxy/bin");

    if cfg!(debug_assertions) {
        push_sidecar_binary_candidates(&mut candidates, &dev_sidecar_dir);
    }
    push_sidecar_binary_candidates(&mut candidates, parent);
    if let Some(contents_dir) = parent.parent() {
        push_sidecar_binary_candidates(&mut candidates, &contents_dir.join("Resources"));
    }
    if !cfg!(debug_assertions) {
        push_sidecar_binary_candidates(&mut candidates, &dev_sidecar_dir);
    }
    Ok(candidates)
}

fn sidecar_binary_path() -> Result<PathBuf, String> {
    let candidates = sidecar_binary_candidates()?;
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "API 服务 sidecar 二进制不存在，已检查: {}。请重新构建应用。",
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn sanitize_sidecar_command_env(command: &mut TokioCommand) {
    #[cfg(target_os = "macos")]
    {
        // 避免把 Cockpit 的应用身份和 XPC 上下文传给 sidecar，导致局域网访问异常。
        command.env_remove("__CFBundleIdentifier");
        command.env_remove("XPC_SERVICE_NAME");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = command;
}

fn sidecar_auth_file_name(account_id: &str) -> String {
    let mut safe = account_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.trim_matches('_').is_empty() {
        safe = uuid::Uuid::new_v4().to_string();
    }
    format!("{safe}.json")
}

fn write_string_atomic_if_changed(path: &Path, content: &str) -> Result<bool, String> {
    match std::fs::read_to_string(path) {
        Ok(existing) if existing == content => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
    write_string_atomic(path, content)?;
    Ok(true)
}

fn harden_sidecar_auth_file_permissions(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| {
                format!(
                    "设置 API 服务 sidecar 认证文件权限失败: path={}, error={}",
                    path.display(),
                    error
                )
            },
        )?;
    }
    Ok(())
}

fn remove_stale_sidecar_auth_files(
    auths_dir: &Path,
    expected_file_names: &HashSet<String>,
) -> Result<(), String> {
    if !auths_dir.exists() {
        return Ok(());
    }
    let entries = std::fs::read_dir(auths_dir)
        .map_err(|e| format!("读取 API 服务 sidecar 认证目录失败: {}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取 API 服务 sidecar 认证文件失败: {}", e))?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|item| item.to_str()) != Some("json") {
            continue;
        }
        let Some(file_name) = path
            .file_name()
            .and_then(|item| item.to_str())
            .map(|item| item.to_string())
        else {
            continue;
        };
        if expected_file_names.contains(&file_name) {
            continue;
        }
        std::fs::remove_file(&path).map_err(|e| {
            format!(
                "清理过期 API 服务 sidecar 认证文件失败: path={}, error={}",
                path.display(),
                e
            )
        })?;
    }
    Ok(())
}

fn sidecar_stable_id(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.trim().as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.trim().as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    let short = digest.get(..12).unwrap_or(digest.as_str());
    format!("{}:{}", kind.trim(), short)
}

fn sidecar_codex_api_key_auth_id(account: &CodexAccount) -> Option<String> {
    let api_key = account.openai_api_key.as_deref()?.trim();
    if api_key.is_empty() {
        return None;
    }
    let base_url = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPENAI_RESPONSES_BASE_URL);
    Some(sidecar_stable_id("codex:apikey", &[api_key, base_url]))
}

fn sidecar_auth_id_for_account(account: &CodexAccount) -> Option<String> {
    if account.is_api_key_auth() {
        return sidecar_codex_api_key_auth_id(account);
    }
    Some(sidecar_auth_file_name(&account.id))
}

fn sidecar_auth_id_for_account_id(account_id: &str) -> Option<String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return None;
    }
    let account = codex_account::load_account(account_id)?;
    sidecar_auth_id_for_account(&account)
}

fn sidecar_auth_ids_for_account_ids_with_overrides(
    account_ids: Vec<String>,
    account_overrides: &HashMap<String, CodexAccount>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for account_id in account_ids {
        let auth_id = account_overrides
            .get(account_id.trim())
            .and_then(sidecar_auth_id_for_account)
            .or_else(|| sidecar_auth_id_for_account_id(&account_id));
        let Some(auth_id) = auth_id else {
            continue;
        };
        if seen.insert(auth_id.clone()) {
            values.push(auth_id);
        }
    }
    values
}

fn sidecar_duration_ms(value_ms: i64) -> String {
    format!("{}ms", value_ms.max(1))
}

fn sidecar_routing_strategy_value(strategy: CodexLocalAccessRoutingStrategy) -> &'static str {
    match strategy {
        CodexLocalAccessRoutingStrategy::Auto => "auto",
        CodexLocalAccessRoutingStrategy::Random => "random",
        CodexLocalAccessRoutingStrategy::SingleAccount => "single_account",
        CodexLocalAccessRoutingStrategy::QuotaHighFirst => "quota_high_first",
        CodexLocalAccessRoutingStrategy::QuotaLowFirst => "quota_low_first",
        CodexLocalAccessRoutingStrategy::PlanHighFirst => "plan_high_first",
        CodexLocalAccessRoutingStrategy::PlanLowFirst => "plan_low_first",
        CodexLocalAccessRoutingStrategy::ExpirySoonFirst => "expiry_soon_first",
        CodexLocalAccessRoutingStrategy::Custom => "custom",
    }
}

fn sidecar_model_alias_values(collection: &CodexLocalAccessCollection) -> Vec<Value> {
    collection
        .model_aliases
        .iter()
        .map(|alias| {
            json!({
                "name": alias.source_model.clone(),
                "alias": alias.alias.clone(),
                "fork": alias.fork,
            })
        })
        .collect()
}

fn sidecar_codex_key_model_values(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
) -> Vec<Value> {
    if !account.api_model_mappings.is_empty() {
        return account
            .api_model_mappings
            .iter()
            .map(|mapping| {
                json!({
                    "name": mapping.upstream_model.clone(),
                    "alias": mapping.client_model.clone(),
                })
            })
            .collect();
    }
    collection
        .model_aliases
        .iter()
        .map(|alias| {
            json!({
                "name": alias.source_model.clone(),
                "alias": alias.alias.clone(),
            })
        })
        .collect()
}

fn account_api_model_mapping_ids(account: &CodexAccount) -> HashSet<String> {
    account
        .api_model_mappings
        .iter()
        .flat_map(|mapping| {
            [
                mapping.client_model.as_str(),
                mapping.upstream_model.as_str(),
            ]
        })
        .map(|model| model.trim().to_ascii_lowercase())
        .filter(|model| !model.is_empty())
        .collect()
}

fn legacy_api_key_is_active(collection: &CodexLocalAccessCollection) -> bool {
    let key = collection.api_key.trim();
    !key.is_empty()
        && !collection.account_ids.is_empty()
        && !collection
            .api_keys
            .iter()
            .any(|item| item.key.trim() == key)
}

fn sidecar_api_key_manifest_values(collection: &CodexLocalAccessCollection) -> Vec<Value> {
    let mut values = Vec::new();
    let bound_oauth =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref()).is_some();
    if legacy_api_key_is_active(collection) {
        values.push(json!({
            "id": "legacy",
            "label": default_local_api_key_label(),
            "key": collection.api_key.trim(),
            "enabled": true,
            "boundOAuth": bound_oauth,
            "accountIds": collection.account_ids.clone(),
            "responsesWebsockets": collection.responses_websockets_enabled,
            "allowedModels": [],
            "excludedModels": [],
            "tokenLimit": null,
            "tokenUsed": 0,
        }));
    }
    for item in &collection.api_keys {
        if !item.enabled || item.key.trim().is_empty() {
            continue;
        }
        let account_ids = effective_api_key_account_ids(collection, item);
        if account_ids.is_empty() {
            continue;
        }
        values.push(json!({
            "id": item.id.clone(),
            "label": item.label.clone(),
            "key": item.key.trim(),
            "providerGateway": item.provider_gateway.clone(),
            "modelRouting": item.model_routing.clone(),
            "boundOAuth": bound_oauth,
            "responsesWebsockets": collection.responses_websockets_enabled
                && item.provider_gateway.is_none()
                && item.model_routing.is_none(),
            "accountIds": account_ids,
            "modelPrefix": item.model_prefix.clone(),
            "allowedModels": item.allowed_models.clone(),
            "excludedModels": item.excluded_models.clone(),
            "tokenLimit": item.token_limit,
            "tokenUsed": item.token_used,
            "enabled": item.enabled,
        }));
    }
    values
}

fn api_key_token_limit_exceeded(api_key: &ResolvedLocalApiKey) -> Option<(u64, u64)> {
    api_key
        .token_limit
        .filter(|limit| *limit > 0 && api_key.token_used >= *limit)
        .map(|limit| (api_key.token_used, limit))
}

fn add_api_key_token_usage(
    collection: &mut CodexLocalAccessCollection,
    api_key_id: &str,
    total_tokens: u64,
) -> bool {
    let api_key_id = api_key_id.trim();
    if api_key_id.is_empty() || api_key_id == "legacy" || total_tokens == 0 {
        return false;
    }
    let Some(api_key) = collection
        .api_keys
        .iter_mut()
        .find(|item| item.id == api_key_id)
    else {
        return false;
    };
    api_key.token_used = api_key.token_used.saturating_add(total_tokens);
    true
}

fn effective_usage_total_tokens(usage: &UsageCapture) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input_tokens.saturating_add(usage.output_tokens)
    }
}

fn sidecar_api_key_priority_state_values(collection: &CodexLocalAccessCollection) -> Value {
    let mut priority_account_ids = Map::new();
    for item in &collection.api_keys {
        if api_key_inherits_account_pool(item) || api_key_has_fixed_account_scope(collection, item)
        {
            continue;
        }
        let priorities = normalize_account_id_list(item.priority_account_ids.clone())
            .into_iter()
            .filter(|account_id| {
                item.account_ids
                    .iter()
                    .any(|selected_id| selected_id == account_id)
            })
            .collect::<Vec<_>>();
        if !priorities.is_empty() {
            priority_account_ids.insert(
                item.id.clone(),
                Value::Array(priorities.into_iter().map(Value::String).collect()),
            );
        }
    }
    json!({
        "priorityAccountIds": priority_account_ids,
    })
}

fn write_sidecar_api_key_priority_state_in_dir(
    collection: &CodexLocalAccessCollection,
    base_dir: &Path,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(&sidecar_api_key_priority_state_values(collection))
        .map_err(|error| format!("序列化 sidecar API Key 置顶状态失败: {}", error))?;
    write_string_atomic_if_changed(&sidecar_api_key_priority_path(base_dir), &content).map(|_| ())
}

fn api_key_inherits_account_pool(api_key: &CodexLocalAccessApiKey) -> bool {
    if api_key.provider_gateway.is_some() {
        return false;
    }
    api_key
        .inherit_account_pool
        .unwrap_or_else(|| api_key.account_ids.is_empty())
}

fn api_key_has_fixed_account_scope(
    collection: &CodexLocalAccessCollection,
    api_key: &CodexLocalAccessApiKey,
) -> bool {
    if api_key.provider_gateway.is_some() {
        return true;
    }

    collection.bound_oauth_account_id.is_some()
        && collection.account_ids.is_empty()
        && api_key.account_ids.len() == 1
        && api_key.id == provider_gateway_api_key_id(&api_key.account_ids[0])
}

fn validate_api_key_account_scope_update(
    collection: &CodexLocalAccessCollection,
    api_key: &CodexLocalAccessApiKey,
    account_ids: Option<&[String]>,
    inherit_account_pool: Option<bool>,
) -> Result<(), String> {
    if !api_key_has_fixed_account_scope(collection, api_key) {
        return Ok(());
    }

    let requested_inherit = inherit_account_pool.unwrap_or_else(|| {
        account_ids
            .map(|ids| ids.is_empty())
            .unwrap_or_else(|| api_key_inherits_account_pool(api_key))
    });
    if requested_inherit {
        return Err("固定账号 Key 不支持继承服务账号池".to_string());
    }
    if account_ids.is_some_and(|ids| ids.is_empty()) {
        return Err("固定账号 Key 不能清空账号范围".to_string());
    }
    if let Some(requested_account_ids) = account_ids {
        let expected_account_ids = normalize_account_id_list(api_key.account_ids.clone());
        let requested_account_ids = normalize_account_id_list(requested_account_ids.to_vec());
        if requested_account_ids != expected_account_ids {
            return Err("固定账号 Key 不支持修改账号范围".to_string());
        }
    }
    Ok(())
}

fn codex_app_speed_service_tier(speed: &CodexAppSpeed) -> Option<&'static str> {
    match speed {
        CodexAppSpeed::Fast => Some("priority"),
        CodexAppSpeed::Standard => None,
    }
}

fn effective_api_key_account_ids(
    collection: &CodexLocalAccessCollection,
    api_key: &CodexLocalAccessApiKey,
) -> Vec<String> {
    if api_key_inherits_account_pool(api_key) {
        collection.account_ids.clone()
    } else {
        api_key.account_ids.clone()
    }
}

fn effective_sidecar_account_ids(collection: &CodexLocalAccessCollection) -> Vec<String> {
    let mut account_ids = collection.account_ids.clone();
    let mut seen: HashSet<String> = account_ids.iter().cloned().collect();
    for api_key in &collection.api_keys {
        for account_id in &api_key.account_ids {
            if seen.insert(account_id.clone()) {
                account_ids.push(account_id.clone());
            }
        }
        if let Some(model_routing) = &api_key.model_routing {
            for route in &model_routing.routes {
                if seen.insert(route.provider_account_id.clone()) {
                    account_ids.push(route.provider_account_id.clone());
                }
            }
        }
    }
    account_ids
}

/// 池内某一类额度窗口的汇总（按真实窗口时长归类，避免把周窗误标成 5h）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiServicePoolWindowSum {
    /// 稳定 key：如 "5h" / "weekly" / "2d"
    pub key: String,
    /// 展示用英文标签，上层再做本地化（Weekly → 周）。
    pub label: String,
    pub percentage: i32,
    pub window_minutes: i64,
}

/// 菜单栏 / 托盘菜单：API 服务账号池额度摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApiServiceMenuBarQuota {
    /// 各窗口合计中的较小值；用于菜单栏单数字展示与配色。
    pub remaining_percent: Option<i32>,
    /// 按窗口时长归类后的汇总行（与悬浮卡 / API 服务页一致）。
    pub windows: Vec<ApiServicePoolWindowSum>,
    /// 池内 OAuth 账号数量（参与汇总的账号）。
    pub account_count: usize,
}

fn api_service_window_bucket(window_minutes: Option<i64>, fallback: &str) -> (String, String, i64) {
    const HOUR_MINUTES: i64 = 60;
    const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
    const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

    let minutes = window_minutes
        .filter(|value| *value > 0)
        .unwrap_or_else(|| {
            if fallback.eq_ignore_ascii_case("weekly") {
                WEEK_MINUTES
            } else {
                5 * HOUR_MINUTES
            }
        });

    let (key, label) = if minutes >= WEEK_MINUTES - 1 {
        let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
        if weeks <= 1 {
            ("weekly".to_string(), "Weekly".to_string())
        } else {
            (format!("{weeks}week"), format!("{weeks} Week"))
        }
    } else if minutes >= DAY_MINUTES - 1 {
        let days = (minutes + DAY_MINUTES - 1) / DAY_MINUTES;
        (format!("{days}d"), format!("{days}d"))
    } else if minutes >= HOUR_MINUTES {
        let hours = (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES;
        (format!("{hours}h"), format!("{hours}h"))
    } else {
        (format!("{minutes}m"), format!("{minutes}m"))
    };

    (key, label, minutes)
}

fn add_api_service_window_sum(
    windows: &mut Vec<ApiServicePoolWindowSum>,
    window_minutes: Option<i64>,
    fallback: &str,
    percentage: i32,
) {
    let (key, label, minutes) = api_service_window_bucket(window_minutes, fallback);
    let value = percentage.clamp(0, 100);
    if let Some(existing) = windows.iter_mut().find(|item| item.key == key) {
        existing.percentage = existing.percentage.saturating_add(value);
        existing.window_minutes = existing.window_minutes.min(minutes);
        return;
    }
    windows.push(ApiServicePoolWindowSum {
        key,
        label,
        percentage: value,
        window_minutes: minutes,
    });
}

/// 读取本地 API 服务集合，按真实窗口时长汇总池内 OAuth 账号剩余百分比。
pub(crate) fn menu_bar_api_service_quota() -> ApiServiceMenuBarQuota {
    let Ok(Some(collection)) = load_collection_from_disk() else {
        return ApiServiceMenuBarQuota {
            remaining_percent: None,
            windows: Vec::new(),
            account_count: 0,
        };
    };

    let mut windows: Vec<ApiServicePoolWindowSum> = Vec::new();
    let mut account_count = 0usize;

    for account_id in effective_sidecar_account_ids(&collection) {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        // 池汇总仅计 OAuth 类窗口额度；API Key 账号走单独额度模型。
        if account.is_api_key_auth() {
            continue;
        }
        account_count += 1;
        let Some(quota) = account.quota.as_ref() else {
            continue;
        };
        let has_presence_flags =
            quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
        // 与前端 getCodexQuotaWindows 一致：按 present 决定是否纳入，标签看 window_minutes。
        if !has_presence_flags || quota.hourly_window_present == Some(true) {
            add_api_service_window_sum(
                &mut windows,
                quota.hourly_window_minutes,
                "5h",
                quota.hourly_percentage,
            );
        }
        if !has_presence_flags || quota.weekly_window_present == Some(true) {
            add_api_service_window_sum(
                &mut windows,
                quota.weekly_window_minutes,
                "Weekly",
                quota.weekly_percentage,
            );
        }
    }

    windows.sort_by(|left, right| {
        left.window_minutes
            .cmp(&right.window_minutes)
            .then_with(|| left.label.cmp(&right.label))
    });

    let remaining_percent = windows.iter().map(|item| item.percentage).min();

    ApiServiceMenuBarQuota {
        remaining_percent,
        windows,
        account_count,
    }
}

/// 池内可刷新额度的 OAuth 账号 ID（用于托盘菜单刷新 API 服务额度）。
pub(crate) fn api_service_refreshable_account_ids() -> Vec<String> {
    let Ok(Some(collection)) = load_collection_from_disk() else {
        return Vec::new();
    };
    effective_sidecar_account_ids(&collection)
        .into_iter()
        .filter(|account_id| {
            codex_account::load_account(account_id)
                .map(|account| {
                    !account.is_api_key_auth()
                        && crate::modules::codex_quota::supports_quota_refresh(&account)
                })
                .unwrap_or(false)
        })
        .collect()
}

/// 是否存在 API 服务集合（有账号即可在托盘中展示 API 服务卡片）。
pub(crate) fn api_service_collection_has_accounts() -> bool {
    let Ok(Some(collection)) = load_collection_from_disk() else {
        return false;
    };
    !effective_sidecar_account_ids(&collection).is_empty()
}

fn remove_account_refs_from_collection(
    collection: &mut CodexLocalAccessCollection,
    remove_ids: &HashSet<String>,
) -> bool {
    if remove_ids.is_empty() {
        return false;
    }

    let mut changed = false;

    let before_account_ids = collection.account_ids.clone();
    collection.account_ids.retain(|id| !remove_ids.contains(id));
    changed |= collection.account_ids != before_account_ids;

    for api_key in &mut collection.api_keys {
        let before = api_key.account_ids.clone();
        api_key.account_ids.retain(|id| !remove_ids.contains(id));
        changed |= api_key.account_ids != before;
        if api_key
            .priority_account_ids
            .iter()
            .any(|id| remove_ids.contains(id))
        {
            api_key
                .priority_account_ids
                .retain(|id| !remove_ids.contains(id));
            changed = true;
        }
        if api_key
            .preferred_account_id
            .as_ref()
            .is_some_and(|id| remove_ids.contains(id))
        {
            api_key.preferred_account_id = None;
            changed = true;
        }
    }

    let before_custom_rules = collection.custom_routing_rules.clone();
    collection
        .custom_routing_rules
        .retain(|rule| !remove_ids.contains(&rule.account_id));
    changed |= collection.custom_routing_rules != before_custom_rules;

    let before_model_rules = collection.account_model_rules.clone();
    collection
        .account_model_rules
        .retain(|rule| !remove_ids.contains(&rule.account_id));
    changed |= collection.account_model_rules != before_model_rules;

    if collection
        .bound_oauth_account_id
        .as_ref()
        .map(|id| remove_ids.contains(id))
        .unwrap_or(false)
    {
        collection.bound_oauth_account_id = None;
        collection.bound_oauth_quota_reserve = None;
        changed = true;
    }

    changed
}

fn sidecar_client_api_keys(
    collection: &CodexLocalAccessCollection,
    account_overrides: &HashMap<String, CodexAccount>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    if legacy_api_key_is_active(collection)
        && !sidecar_auth_ids_for_account_ids_with_overrides(
            collection.account_ids.clone(),
            account_overrides,
        )
        .is_empty()
        && seen.insert(collection.api_key.trim().to_string())
    {
        keys.push(collection.api_key.trim().to_string());
    }
    for item in &collection.api_keys {
        let key = item.key.trim();
        let has_resolvable_scope = item.provider_gateway.is_some()
            || !sidecar_auth_ids_for_account_ids_with_overrides(
                effective_api_key_account_ids(collection, item),
                account_overrides,
            )
            .is_empty();
        if item.enabled && !key.is_empty() && has_resolvable_scope && seen.insert(key.to_string()) {
            keys.push(key.to_string());
        }
    }
    keys
}

fn sidecar_api_key_account_scope_values(
    collection: &CodexLocalAccessCollection,
    account_overrides: &HashMap<String, CodexAccount>,
) -> Value {
    let mut values = Map::new();
    if legacy_api_key_is_active(collection) {
        let auth_ids = sidecar_auth_ids_for_account_ids_with_overrides(
            collection.account_ids.clone(),
            account_overrides,
        );
        if !auth_ids.is_empty() {
            values.insert(collection.api_key.trim().to_string(), json!(auth_ids));
        }
    }
    for item in &collection.api_keys {
        let key = item.key.trim();
        if !item.enabled || key.is_empty() {
            continue;
        }
        if item.provider_gateway.is_some() {
            continue;
        }
        let auth_ids = sidecar_auth_ids_for_account_ids_with_overrides(
            effective_api_key_account_ids(collection, item),
            account_overrides,
        );
        if auth_ids.is_empty() {
            continue;
        }
        values.insert(key.to_string(), json!(auth_ids));
    }
    Value::Object(values)
}

fn sidecar_account_last_refresh(account: &CodexAccount) -> String {
    account
        .token_updated_at
        .or_else(|| (account.created_at > 0).then_some(account.created_at))
        .or_else(|| (account.last_used > 0).then_some(account.last_used))
        .unwrap_or(0)
        .to_string()
}

fn sidecar_auth_json_for_account(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
) -> Value {
    let metered_feature_patterns =
        metered_feature_model_patterns_for_pool(collection, &HashMap::new());
    sidecar_auth_json_for_account_with_metered_feature_patterns(
        account,
        collection,
        proxy_url,
        &metered_feature_patterns,
    )
}

fn sidecar_auth_json_for_account_with_metered_feature_patterns(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
    metered_feature_patterns: &HashMap<String, String>,
) -> Value {
    let account_id = account
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let excluded_models =
        sidecar_excluded_models_for_account(account, collection, metered_feature_patterns);
    if let Some(identity) = account.agent_identity.as_ref() {
        let mut value = json!({
            "type": "codex",
            "auth_mode": "agentIdentity",
            "openai_auth_mode": "agentIdentity",
            "agent_runtime_id": identity.agent_runtime_id,
            "agent_private_key": identity.agent_private_key,
            "task_id": identity.task_id,
            "account_id": identity.account_id,
            "chatgpt_user_id": identity.chatgpt_user_id,
            "chatgpt_account_is_fedramp": identity.chatgpt_account_is_fedramp,
            "email": account.email,
            "plan_type": account.plan_type,
            "excluded_models": excluded_models,
            "disable_cooling": collection.disable_cooling,
            "websockets": collection.responses_websockets_enabled,
        });
        if let Some(proxy_url) = proxy_url {
            value["proxy_url"] = Value::String(proxy_url.to_string());
        }
        return value;
    }
    let mut value = json!({
        "type": "codex",
        "id_token": account.tokens.id_token.clone(),
        "access_token": account.tokens.access_token.clone(),
        // sidecar 只消费 Token Authority 下发的 bearer token，禁止自行轮换 RT。
        // 否则它会与官方 ChatGPT/Codex app-server 竞争一次性 refresh_token。
        "refresh_token": "",
        "refresh_owner": "cockpit_token_authority",
        "last_refresh": sidecar_account_last_refresh(account),
        "email": account.email.clone(),
        "plan_type": account.plan_type.clone(),
        "excluded_models": excluded_models,
        "disable_cooling": collection.disable_cooling,
        "websockets": collection.responses_websockets_enabled,
        "codex_cli_only": account.codex_cli_only,
        "codex_cli_only_allow_app_server": account.codex_cli_only_allow_app_server,
        "codex_cli_only_allow_app_server_clients": config::get_user_config()
            .codex_cli_only_allow_app_server_clients,
    });
    if account_uses_codex_fingerprint_convergence(account) {
        value["codex_fingerprint_mode"] =
            json!(crate::modules::codex_account::resolved_codex_fingerprint_mode(account));
    }
    if let Some(account_id) = account_id {
        value["account_id"] = json!(account_id);
    }
    if account_is_access_token_only(account) {
        value["auth_mode"] = json!("personal_access_token");
        value["openai_auth_mode"] = json!("personal_access_token");
        value["token_type"] = json!("Bearer");
    } else {
        value["auth_mode"] = json!("oauth");
        value["openai_auth_mode"] = json!("oauth");
    }
    if account_uses_personal_access_token(account) {
        value["personal_access_token"] = json!(account.tokens.access_token.clone());
        value["at_token"] = json!(account.tokens.access_token.clone());
    }
    if let Some(expired_at) =
        codex_oauth::jwt_token_expiration_timestamp(&account.tokens.access_token)
    {
        value["expired"] = json!(expired_at);
    }
    if let Some(proxy_url) = proxy_url {
        value["proxy_url"] = Value::String(proxy_url.to_string());
    }
    value
}

fn existing_sidecar_agent_identity_task(
    account: &CodexAccount,
    auth_path: &Path,
) -> Result<Option<String>, String> {
    let Some(identity) = account.agent_identity.as_ref() else {
        return Ok(None);
    };
    if !auth_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(auth_path)
        .map_err(|e| format!("读取 sidecar Agent Identity 认证失败: {}", e))?;
    let payload: Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析 sidecar Agent Identity 认证失败: {}", e))?;
    let runtime_id = payload
        .get("agent_runtime_id")
        .or_else(|| payload.get("agentRuntimeId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let private_key = payload
        .get("agent_private_key")
        .or_else(|| payload.get("agentPrivateKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if runtime_id != identity.agent_runtime_id.trim()
        || private_key != identity.agent_private_key.trim()
    {
        return Ok(None);
    }
    Ok(payload
        .get("task_id")
        .or_else(|| payload.get("taskId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string))
}

fn adopt_sidecar_agent_identity_task(
    account: &mut CodexAccount,
    auth_path: &Path,
) -> Result<bool, String> {
    let Some(task_id) = existing_sidecar_agent_identity_task(account, auth_path)? else {
        return Ok(false);
    };
    let Some(identity) = account.agent_identity.as_mut() else {
        return Ok(false);
    };
    if identity.task_id.as_deref().map(str::trim) == Some(task_id.as_str()) {
        return Ok(false);
    }
    identity.task_id = Some(task_id.clone());

    if let Some(mut stored) = codex_account::load_account(&account.id) {
        if let Some(stored_identity) = stored.agent_identity.as_mut() {
            if stored_identity.agent_runtime_id.trim() == identity.agent_runtime_id.trim()
                && stored_identity.agent_private_key.trim() == identity.agent_private_key.trim()
                && stored_identity.task_id.as_deref().map(str::trim) != Some(task_id.as_str())
            {
                stored_identity.task_id = Some(task_id);
                codex_account::save_account(&stored)?;
            }
        }
    }
    Ok(true)
}

fn sync_sidecar_auth_file_for_account_with_task_source(
    account: &CodexAccount,
    prefer_account_task: bool,
) -> Result<(), String> {
    if account.is_api_key_auth() {
        return Ok(());
    }

    let Some(collection) = load_collection_from_disk()? else {
        return Ok(());
    };
    if !sidecar_auth_account_is_scoped(&collection, &account.id) {
        return Ok(());
    }

    let base_dir = local_access_sidecar_dir()?;
    let auths_dir = sidecar_auths_dir(&base_dir);
    if !auths_dir.exists() {
        return Ok(());
    }

    let proxy_signature = sidecar_effective_proxy_signature(&collection)?;
    let auth_path = auths_dir.join(sidecar_auth_file_name(&account.id));
    if !auth_path.exists() {
        return Ok(());
    }
    let mut effective_account = account.clone();
    if !prefer_account_task {
        adopt_sidecar_agent_identity_task(&mut effective_account, &auth_path)?;
    }
    let auth_json = sidecar_auth_json_for_account(
        &effective_account,
        &collection,
        proxy_signature.proxy_url.as_deref(),
    );
    let auth_content = serde_json::to_string_pretty(&auth_json)
        .map_err(|e| format!("序列化 sidecar Codex OAuth 认证失败: {}", e))?;
    write_string_atomic(&auth_path, &auth_content)?;
    harden_sidecar_auth_file_permissions(&auth_path)?;
    invalidate_prepared_account_if_unlocked(&account.id);
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][sidecar] 已写穿 Cockpit Token Authority 凭证: account_id={}",
        account.id
    ));
    Ok(())
}

pub fn sync_sidecar_auth_file_for_account(account: &CodexAccount) -> Result<(), String> {
    let result = sync_sidecar_auth_file_for_account_with_task_source(account, false);
    sync_provider_gateway_auth_files_for_account_in_background(account.clone());
    result
}

/// 通用设置中的 Codex 客户端策略变更后，后台刷新正在使用的 OAuth auth 文件。
/// 账号列表与文件写入不阻塞设置页保存；sidecar 文件观察器会随后加载新元数据。
pub fn schedule_codex_client_policy_sync() {
    if CODEX_CLIENT_POLICY_SYNC_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    std::thread::spawn(|| {
        for account in crate::modules::codex_account::list_accounts() {
            if account.is_api_key_auth() || account.is_agent_identity_auth() {
                continue;
            }
            if let Err(error) = sync_sidecar_auth_file_for_account(&account) {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 后台刷新 Codex 客户端策略失败: account_id={}, error={}",
                    account.id, error
                ));
            }
        }
        CODEX_CLIENT_POLICY_SYNC_RUNNING.store(false, Ordering::Release);
    });
}

pub fn sync_sidecar_auth_file_for_account_with_current_task(
    account: &CodexAccount,
) -> Result<(), String> {
    sync_sidecar_auth_file_for_account_with_task_source(account, true)
}

fn sidecar_quota_reserve_manifest_value(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Option<Value> {
    let reserve = collection.bound_oauth_quota_reserve.as_ref()?;
    let bound_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())?;
    if account.id != bound_account_id {
        return None;
    }

    Some(json!({
        "hourlyThresholdPercent": reserve.hourly_percent,
        "weeklyThresholdPercent": reserve.weekly_percent,
    }))
}

fn sidecar_quota_reserve_snapshot_value(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Option<Value> {
    let reserve = collection.bound_oauth_quota_reserve.as_ref()?;
    let bound_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())?;
    if account.id != bound_account_id {
        return None;
    }

    let quota = fresh_quota_for_bound_oauth_reserve(account);
    Some(json!({
        "snapshotUpdatedAtUnixSeconds": account.usage_updated_at,
        "hourlyRemainingPercent": quota
            .and_then(|quota| valid_quota_remaining_percent(quota.hourly_percentage)),
        "weeklyRemainingPercent": quota
            .and_then(|quota| valid_quota_remaining_percent(quota.weekly_percentage)),
        "hourlyWindowPresent": quota.and_then(|quota| quota.hourly_window_present),
        "weeklyWindowPresent": quota.and_then(|quota| quota.weekly_window_present),
        "hourlyThresholdPercent": reserve.hourly_percent,
        "weeklyThresholdPercent": reserve.weekly_percent,
    }))
}

fn sidecar_quota_reserve_state_value(collection: &CodexLocalAccessCollection) -> Value {
    let mut accounts = Map::new();
    if let Some(account_id) =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref())
    {
        if let Some(account) = codex_account::load_account(&account_id) {
            if let Some(snapshot) = sidecar_quota_reserve_snapshot_value(collection, &account) {
                accounts.insert(account_id, snapshot);
            }
        }
    }
    json!({ "accounts": accounts })
}

fn write_sidecar_quota_reserve_state(
    collection: &CodexLocalAccessCollection,
) -> Result<PathBuf, String> {
    let base_dir = local_access_sidecar_dir()?;
    write_sidecar_quota_reserve_state_in_dir(collection, &base_dir)
}

fn write_sidecar_quota_reserve_state_in_dir(
    collection: &CodexLocalAccessCollection,
    base_dir: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&base_dir)
        .map_err(|error| format!("创建 API 服务 sidecar 目录失败: {}", error))?;
    let path = sidecar_quota_reserve_path(&base_dir);
    let content = serde_json::to_string_pretty(&sidecar_quota_reserve_state_value(collection))
        .map_err(|error| format!("序列化 OAuth 保留额度快照失败: {}", error))?;
    write_string_atomic_if_changed(&path, &content)?;
    Ok(path)
}

fn sidecar_quota_pool_window_value(
    percentage: i32,
    window_minutes: Option<i64>,
    present: Option<bool>,
    reset_at: Option<i64>,
) -> Value {
    let resolved_present =
        present.unwrap_or(window_minutes.is_some() || reset_at.is_some() || percentage > 0);
    json!({
        "present": resolved_present,
        "remainingPercent": percentage.clamp(0, 100),
        "windowMinutes": window_minutes.filter(|value| *value > 0),
        "resetAt": reset_at,
    })
}

fn sidecar_quota_pool_state_value(collection: &CodexLocalAccessCollection) -> Value {
    let mut accounts = Map::new();
    for account_id in effective_sidecar_account_ids(collection) {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        let Some(quota) = account.quota.as_ref() else {
            continue;
        };
        accounts.insert(
            account_id,
            json!({
                "primary": sidecar_quota_pool_window_value(
                    quota.hourly_percentage,
                    quota.hourly_window_minutes,
                    quota.hourly_window_present,
                    quota.hourly_reset_time,
                ),
                "secondary": sidecar_quota_pool_window_value(
                    quota.weekly_percentage,
                    quota.weekly_window_minutes,
                    quota.weekly_window_present,
                    quota.weekly_reset_time,
                ),
                "updatedAt": account.usage_updated_at,
                "cooldown": account_quota_cooldown(&account, now_ms()),
            }),
        );
    }
    json!({ "accounts": accounts })
}

fn write_sidecar_quota_pool_state_in_dir(
    collection: &CodexLocalAccessCollection,
    base_dir: &Path,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(base_dir)
        .map_err(|error| format!("创建 API 服务 sidecar 额度目录失败: {}", error))?;
    let path = sidecar_quota_pool_path(base_dir);
    let content = serde_json::to_string_pretty(&sidecar_quota_pool_state_value(collection))
        .map_err(|error| format!("序列化 API 服务额度池快照失败: {}", error))?;
    write_string_atomic_if_changed(&path, &content)?;
    Ok(path)
}

fn write_sidecar_quota_pool_state(
    collection: &CodexLocalAccessCollection,
) -> Result<PathBuf, String> {
    let base_dir = local_access_sidecar_dir()?;
    write_sidecar_quota_pool_state_in_dir(collection, &base_dir)
}

fn sidecar_account_manifest_value(
    account: &CodexAccount,
    auth_id: Option<&str>,
    collection: &CodexLocalAccessCollection,
) -> Value {
    let auth_kind = if account.is_api_key_auth() {
        "api_key"
    } else if account.is_agent_identity_auth() {
        "agent_identity"
    } else if account_is_access_token_only(account) {
        "access_token"
    } else {
        "oauth"
    };
    let mut value = json!({
        "id": account.id.clone(),
        "email": account.email.clone(),
        "authId": auth_id,
        "authKind": auth_kind,
        "planType": account.plan_type.as_deref(),
        "accessTokenOnly": account_is_access_token_only(account),
        "chatgptAccountId": account.account_id.as_deref().unwrap_or_default(),
        "upstreamApiKey": account.openai_api_key.as_deref().unwrap_or_default(),
        "planRank": resolve_plan_rank(account),
        "remainingQuota": resolve_remaining_quota(account),
        "quotaCooldown": account_quota_cooldown(account, now_ms()),
        "subscriptionExpiryMs": resolve_subscription_expiry_ms(account),
        "gptReserveAllowed": account_has_gpt_reserve_entitlement(account),
        "imageGenerationPolicy": match collection.image_generation_account_policies.get(&account.id) {
            Some(CodexLocalAccessImageGenerationPolicy::Enabled) => "enabled",
            Some(CodexLocalAccessImageGenerationPolicy::Disabled) => "disabled",
            _ => "inherit",
        },
    });
    if let Some(quota_reserve) = sidecar_quota_reserve_manifest_value(collection, account) {
        value["quotaReserve"] = quota_reserve;
    }
    if !account.api_model_context_windows.is_empty() {
        value["modelContextWindows"] = json!(account.api_model_context_windows);
    }
    value
}

/// Hosts that must not be treated as a real upstream for the local API sidecar.
fn is_loopback_http_host(host: &str) -> bool {
    matches!(
        host.trim().to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]"
    )
}

fn parse_http_url_host_port(raw: &str) -> Option<(String, u16)> {
    let parsed = Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed
        .port_or_known_default()
        .or_else(|| match parsed.scheme() {
            "https" => Some(443),
            "http" => Some(80),
            _ => None,
        })?;
    Some((host, port))
}

/// True when `raw` is the Cockpit API Service client URL (gateway), not a real upstream.
fn is_local_access_gateway_base_url(raw: &str, collection: &CodexLocalAccessCollection) -> bool {
    if profile_base_url_matches(Some(raw), &build_collection_base_url(collection)) {
        return true;
    }
    let Some((host, port)) = parse_http_url_host_port(raw) else {
        return false;
    };
    // Same loopback port as the running local API service ⇒ self-referential for sidecar.
    is_loopback_http_host(&host) && port == collection.port
}

/// True when `raw` must not be used as a sidecar upstream Base URL.
///
/// Only rejects the current Cockpit API Service client/gateway URL (same host/port
/// self-reference). Loopback URLs on a **different** port are allowed so users can
/// point API Key providers at a separate local OpenAI-compatible process.
fn is_unsafe_sidecar_upstream_base_url(raw: &str, collection: &CodexLocalAccessCollection) -> bool {
    is_local_access_gateway_base_url(raw, collection)
}

fn normalize_upstream_base_url_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_end_matches('/').to_string())
}

/// Look up real provider Base URL from a Cockpit data directory store.
fn lookup_codex_model_provider_base_url_in_dir(
    data_dir: &Path,
    provider_id: Option<&str>,
    provider_name: Option<&str>,
) -> Option<String> {
    let path = data_dir.join("codex_model_providers.json");
    if !path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(path).ok()?;
    let items = serde_json::from_str::<Value>(&raw).ok()?;
    let arr = items.as_array()?;
    let id = provider_id.map(str::trim).filter(|value| !value.is_empty());
    let name = provider_name
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let read_base = |item: &Value| -> Option<String> {
        item.get("baseUrl")
            .or_else(|| item.get("base_url"))
            .and_then(Value::as_str)
            .and_then(normalize_upstream_base_url_string)
    };

    if let Some(id) = id {
        for item in arr {
            let item_id = item.get("id").and_then(Value::as_str).unwrap_or("").trim();
            if item_id == id {
                if let Some(base) = read_base(item) {
                    return Some(base);
                }
            }
        }
    }
    if let Some(name) = name {
        for item in arr {
            let item_name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if item_name.eq_ignore_ascii_case(name) {
                if let Some(base) = read_base(item) {
                    return Some(base);
                }
            }
        }
    }
    None
}

/// Look up real provider Base URL from Cockpit's model-provider store.
fn lookup_codex_model_provider_base_url(
    provider_id: Option<&str>,
    provider_name: Option<&str>,
) -> Option<String> {
    let data_dir = account::get_data_dir().ok()?;
    lookup_codex_model_provider_base_url_in_dir(&data_dir, provider_id, provider_name)
}

/// Resolve the real upstream Base URL for sidecar `codex-api-key` entries.
/// Never returns the local API Service client/gateway URL (would self-route).
fn resolve_sidecar_upstream_base_url(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
) -> Option<String> {
    resolve_sidecar_upstream_base_url_with(account, collection, |provider_id, provider_name| {
        lookup_codex_model_provider_base_url(provider_id, provider_name)
    })
}

fn resolve_sidecar_upstream_base_url_with(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    lookup_provider: impl Fn(Option<&str>, Option<&str>) -> Option<String>,
) -> Option<String> {
    let candidate = account
        .api_base_url
        .as_deref()
        .and_then(normalize_upstream_base_url_string);

    if let Some(url) = candidate.as_ref() {
        if !is_unsafe_sidecar_upstream_base_url(url, collection) {
            return Some(url.clone());
        }
    }

    if let Some(recovered) = lookup_provider(
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    ) {
        if !is_unsafe_sidecar_upstream_base_url(&recovered, collection) {
            return Some(recovered);
        }
    }

    // Polluted gateway / loopback URL on a built-in OpenAI key: fall back to official default.
    if matches!(
        account.api_provider_mode,
        CodexApiProviderMode::OpenaiBuiltin
    ) && candidate
        .as_ref()
        .map(|url| is_unsafe_sidecar_upstream_base_url(url, collection))
        .unwrap_or(false)
    {
        return Some(DEFAULT_OPENAI_RESPONSES_BASE_URL.to_string());
    }

    // Empty base URL + OpenAI builtin → official default.
    if candidate.is_none()
        && matches!(
            account.api_provider_mode,
            CodexApiProviderMode::OpenaiBuiltin
        )
    {
        return Some(DEFAULT_OPENAI_RESPONSES_BASE_URL.to_string());
    }

    None
}

fn sidecar_codex_key_config_value(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
) -> Option<Value> {
    let metered_feature_patterns =
        metered_feature_model_patterns_for_pool(collection, &HashMap::new());
    sidecar_codex_key_config_value_with_metered_feature_patterns(
        account,
        collection,
        proxy_url,
        &metered_feature_patterns,
    )
}

fn sidecar_codex_key_config_value_with_metered_feature_patterns(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
    metered_feature_patterns: &HashMap<String, String>,
) -> Option<Value> {
    let api_key = account.openai_api_key.as_deref()?.trim();
    if api_key.is_empty() {
        return None;
    }
    let Some(base_url) = resolve_sidecar_upstream_base_url(account, collection) else {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][sidecar] 跳过上游 Base URL 为本地网关或无法恢复真实上游的 API Key 账号: account_id={} api_base_url={:?}",
            account.id,
            account.api_base_url
        ));
        return None;
    };
    let excluded_models =
        sidecar_excluded_models_for_account(account, collection, metered_feature_patterns);
    // Map account/provider supportsWebsockets → cliproxy codex-api-key.websockets so the
    // second hop (Cockpit → OpenAI-compatible upstream such as Sub2API) can stay on
    // Responses WebSocket when the client already connected with WebSocket.
    let mut value = json!({
        "api-key": api_key,
        "base-url": base_url,
        "proxy-url": proxy_url,
        "models": sidecar_codex_key_model_values(account, collection),
        "excluded-models": excluded_models,
        "disable-cooling": collection.disable_cooling,
        "websockets": account.api_supports_websockets,
    });
    if proxy_url.is_none() {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("proxy-url");
        }
    }
    Some(value)
}

fn sidecar_effective_proxy_signature(
    collection: &CodexLocalAccessCollection,
) -> Result<UpstreamHttpClientSignature, String> {
    let mut signature = current_upstream_http_client_signature(
        collection.upstream_proxy_url.as_deref(),
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    if signature.proxy_source == UpstreamProxySource::SystemAuto && signature.proxy_url.is_none() {
        signature.proxy_url = system_proxy_url_for_target(DEFAULT_OPENAI_RESPONSES_BASE_URL);
    }
    if let Some(proxy_url) = signature.proxy_url.as_deref() {
        Proxy::all(proxy_url).map_err(|e| match signature.proxy_source {
            UpstreamProxySource::ApiService => format!("API 代理地址无效: {}", e),
            UpstreamProxySource::Global => format!("全局代理地址无效: {}", e),
            UpstreamProxySource::SystemEnv => format!("环境代理地址无效: {}", e),
            UpstreamProxySource::SystemAuto => format!("上游代理地址无效: {}", e),
        })?;
    }
    Ok(signature)
}

fn gateway_mode_label(mode: CodexLocalAccessGatewayMode) -> &'static str {
    gateway_mode_to_db_value(mode)
}

fn collection_gateway_mode(collection: &CodexLocalAccessCollection) -> CodexLocalAccessGatewayMode {
    match collection.gateway_mode {
        CodexLocalAccessGatewayMode::Legacy | CodexLocalAccessGatewayMode::Sidecar => {
            CodexLocalAccessGatewayMode::Sidecar
        }
    }
}

fn log_gateway_mode_info(mode: CodexLocalAccessGatewayMode, message: impl AsRef<str>) {
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][{}] {}",
        gateway_mode_label(mode),
        message.as_ref()
    ));
}

fn log_gateway_mode_warn(mode: CodexLocalAccessGatewayMode, message: impl AsRef<str>) {
    logger::log_codex_api_warn(&format!(
        "[CodexLocalAccess][{}] {}",
        gateway_mode_label(mode),
        message.as_ref()
    ));
}

fn legacy_debug_log(enabled: bool, message: impl AsRef<str>) {
    if !enabled {
        return;
    }

    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][legacy][debug] {}",
        message.as_ref()
    ));
}

fn request_kind_log_label(request_kind: CodexLocalAccessRequestKind) -> &'static str {
    match request_kind {
        CodexLocalAccessRequestKind::Text => "text",
        CodexLocalAccessRequestKind::ImageGeneration => "image_generation",
        CodexLocalAccessRequestKind::ImageEdit => "image_edit",
        CodexLocalAccessRequestKind::Other => "other",
    }
}

fn is_client_disconnect_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection aborted")
        || lower.contains("unexpected eof")
        || lower.contains("客户端已断开")
        || lower.contains("客户端在发送")
}

fn is_client_canceled_error_category(category: &str) -> bool {
    category.trim().eq_ignore_ascii_case("client_canceled")
}

fn is_stream_incomplete_error_category(category: &str) -> bool {
    category.trim().eq_ignore_ascii_case("stream_incomplete")
}

fn is_upstream_response_failed_error_category(category: &str) -> bool {
    category
        .trim()
        .eq_ignore_ascii_case("upstream_response_failed")
}

fn is_upstream_response_failed_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("upstream_response_failed")
        || lower.contains("codex upstream response.failed")
        || lower.contains("last_event=response.failed")
}

fn is_stream_incomplete_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("stream disconnected before completion")
        || lower.contains("error decoding response body")
        || lower.contains("closed before response.completed")
        || lower.contains("closed before response.done")
        || lower.contains("stream ended before completion")
        || lower.contains("incomplete_eof")
}

fn legacy_stream_error_category(message: &str) -> &'static str {
    if is_upstream_response_failed_error_message(message) {
        "upstream_response_failed"
    } else if is_stream_incomplete_error_message(message) {
        "stream_incomplete"
    } else if message.contains("流式响应超时")
        || (message.contains("连续") && message.contains("未收到新数据"))
    {
        "upstream_stream_timeout"
    } else if message.contains("读取上游") {
        "upstream_stream_read_failed"
    } else {
        "stream_write_failed"
    }
}

fn compact_json_for_log(value: &Value) -> String {
    let mut text = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    const MAX_LEN: usize = 800;
    if text.len() > MAX_LEN {
        text.truncate(MAX_LEN);
        text.push_str("...");
    }
    text
}

fn json_field_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|item| {
        item.as_str()
            .map(str::to_string)
            .or_else(|| item.as_i64().map(|number| number.to_string()))
            .or_else(|| item.as_u64().map(|number| number.to_string()))
            .or_else(|| item.as_f64().map(|number| number.to_string()))
            .or_else(|| item.as_bool().map(|flag| flag.to_string()))
    })
}

fn nested_error_object(value: &Value) -> Option<&Value> {
    value
        .get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| value.get("error"))
        .or_else(|| value.get("last_error"))
}

fn upstream_response_failed_signal(
    event_name: Option<&str>,
    value: &Value,
) -> Option<UpstreamResponseFailedSignal> {
    let value_type = value.get("type").and_then(Value::as_str);
    let event_type = value_type.or(event_name).unwrap_or("").trim();
    if event_type != "response.failed" && event_type != "error" {
        return None;
    }

    let error_value = nested_error_object(value).unwrap_or(value);
    let code = json_field_string(error_value, "code").or_else(|| json_field_string(value, "code"));
    let error_type =
        json_field_string(error_value, "type").or_else(|| json_field_string(value, "error_type"));
    let message = json_field_string(error_value, "message")
        .or_else(|| json_field_string(error_value, "detail"))
        .or_else(|| json_field_string(value, "message"));
    let raw = compact_json_for_log(error_value);

    Some(UpstreamResponseFailedSignal {
        event_type: event_type.to_string(),
        code,
        error_type,
        message,
        raw,
    })
}

fn format_upstream_response_failed_error(signal: &UpstreamResponseFailedSignal) -> String {
    format!(
        "upstream_response_failed: Codex upstream {}: code={} type={} message={} raw={}",
        signal.event_type,
        signal.code.as_deref().unwrap_or("-"),
        signal.error_type.as_deref().unwrap_or("-"),
        signal.message.as_deref().unwrap_or("-"),
        signal.raw
    )
}

async fn start_legacy_gateway_locked(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let bind_host = bind_host_for_collection(collection).to_string();
    let port = collection.port;
    let listener = bind_gateway_listener(&bind_host, port)
        .await
        .map_err(|error| format_gateway_bind_error(&bind_host, port, &error))?;
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    let task = tokio::spawn(async move {
        let listener = listener;
        loop {
            tokio::select! {
                changed = shutdown_receiver.changed() => {
                    if changed.is_ok() && *shutdown_receiver.borrow() {
                        break;
                    }
                    if changed.is_err() {
                        break;
                    }
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, addr).await {
                                    if is_client_disconnect_error_message(&error) {
                                        logger::log_codex_api_info(&format!(
                                            "[CodexLocalAccess][legacy] 客户端已断开，停止写入响应: {}",
                                            error
                                        ));
                                    } else {
                                        logger::log_codex_api_warn(&format!(
                                            "[CodexLocalAccess][legacy] 处理网关请求失败: {}",
                                            error
                                        ));
                                    }
                                }
                            });
                        }
                        Err(error) => {
                            logger::log_codex_api_warn(&format!(
                                "[CodexLocalAccess][legacy] 网关监听 accept 失败: {}",
                                error
                            ));
                            break;
                        }
                    }
                }
            }
        }
    });

    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][legacy] API 服务 legacy 网关已启动: bind={}:{} base={}",
        bind_host,
        port,
        build_base_url(port)
    ));

    let mut runtime = gateway_runtime().lock().await;
    runtime.running = true;
    runtime.actual_port = Some(port);
    runtime.actual_bind_host = Some(bind_host);
    runtime.sidecar_config_fingerprint = None;
    runtime.last_error = None;
    runtime.shutdown_sender = Some(shutdown_sender);
    runtime.task = Some(task);
    runtime.sidecar_monitor_task = None;
    runtime.sidecar_generation = None;
    runtime.sidecar_child = None;
    Ok(())
}

fn sidecar_local_account_usable_for_start(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || (!account.is_web_session_auth()
            && (!codex_oauth::is_token_expired(&account.tokens.access_token)
                || (!account.requires_reauth && codex_account::account_has_refresh_token(account))))
}

fn load_sidecar_account_for_start(account_id: &str) -> Option<CodexAccount> {
    // Gateway startup must stay local-only. Expired OAuth credentials are refreshed after the
    // listener becomes available; doing network refreshes here makes stop/update wait behind every
    // account in the pool.
    codex_account::load_account(account_id).filter(sidecar_local_account_usable_for_start)
}

async fn prepare_sidecar_launch_config(
    collection: &CodexLocalAccessCollection,
    preparation: GatewayPreparationContext,
) -> Result<SidecarLaunchConfig, String> {
    let health_snapshot = {
        let runtime = gateway_runtime().lock().await;
        runtime.account_health.clone()
    };
    let default_service_tier = api_service_default_service_tier()?;
    let collection = collection.clone();
    let base_dir = local_access_sidecar_dir()?;
    tauri::async_runtime::spawn_blocking(move || {
        prepare_sidecar_launch_config_in_dir_sync(
            &collection,
            base_dir,
            health_snapshot,
            default_service_tier,
            HashMap::new(),
            true,
            Some(preparation),
        )
    })
    .await
    .map_err(|error| format!("准备 API 服务 sidecar 配置任务失败: {}", error))?
}

async fn prepare_sidecar_launch_config_in_dir(
    collection: &CodexLocalAccessCollection,
    base_dir: PathBuf,
    health_snapshot: HashMap<String, RuntimeAccountHealth>,
    default_service_tier: Option<&str>,
    account_overrides: HashMap<String, CodexAccount>,
) -> Result<SidecarLaunchConfig, String> {
    prepare_sidecar_launch_config_in_dir_sync(
        collection,
        base_dir,
        health_snapshot,
        default_service_tier,
        account_overrides,
        false,
        None,
    )
}

fn prepare_sidecar_launch_config_in_dir_sync(
    collection: &CodexLocalAccessCollection,
    base_dir: PathBuf,
    health_snapshot: HashMap<String, RuntimeAccountHealth>,
    default_service_tier: Option<&str>,
    account_overrides: HashMap<String, CodexAccount>,
    api_service: bool,
    preparation: Option<GatewayPreparationContext>,
) -> Result<SidecarLaunchConfig, String> {
    let auths_dir = sidecar_auths_dir(&base_dir);
    std::fs::create_dir_all(&auths_dir)
        .map_err(|e| format!("创建 API 服务 sidecar 认证目录失败: {}", e))?;

    let proxy_signature = sidecar_effective_proxy_signature(collection)?;
    let effective_proxy_url_ref = proxy_signature.proxy_url.as_deref();

    let mut manifest_accounts = Vec::new();
    let mut codex_keys = Vec::new();
    let mut expected_auth_files = HashSet::new();
    let metered_feature_patterns =
        metered_feature_model_patterns_for_pool(collection, &account_overrides);
    for (index, account_id) in effective_sidecar_account_ids(collection)
        .into_iter()
        .enumerate()
    {
        if preparation
            .is_some_and(|context| gateway_lifecycle_generation_changed(context.generation))
        {
            return Err(GATEWAY_PREPARATION_CANCELLED.to_string());
        }
        if let Some(context) = preparation {
            update_gateway_preparation_progress(context, index + 1);
        }
        if account_health_blocks_routing(health_snapshot.get(&account_id)) {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar 跳过异常账号: account_id={}",
                account_id
            ));
            continue;
        }
        let (mut account, is_override_account) =
            if let Some(account) = account_overrides.get(&account_id).cloned() {
                (account, true)
            } else {
                let Some(account) = load_sidecar_account_for_start(&account_id) else {
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess] sidecar 跳过不存在账号: account_id={}",
                        account_id
                    ));
                    continue;
                };
                (account, false)
            };
        if codex_account::account_has_remote_api_auth_rejection(&account) {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar 跳过远端拒绝账号: account_id={}",
                account_id
            ));
            continue;
        }
        let eligible = if collection_uses_provider_gateway_account(collection, &account.id) {
            is_override_account || is_provider_gateway_eligible_account(&account)
        } else {
            is_local_access_eligible_account(&account, collection.restrict_free_accounts)
        };
        if !eligible {
            continue;
        }

        if account.is_api_key_auth() {
            if let Some(config_value) = sidecar_codex_key_config_value_with_metered_feature_patterns(
                &account,
                collection,
                effective_proxy_url_ref,
                &metered_feature_patterns,
            ) {
                codex_keys.push(config_value);
                manifest_accounts.push(sidecar_account_manifest_value(&account, None, collection));
            } else {
                // Base-URL rejection is already logged inside resolve/config builders.
                // Only emit the missing-key message when the key itself is empty.
                let missing_key = account
                    .openai_api_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none();
                if missing_key {
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess][sidecar] 跳过缺少上游 API Key 的 API Key 账号: account_id={}",
                        account.id
                    ));
                }
            }
            continue;
        }

        let file_name = sidecar_auth_file_name(&account.id);
        let auth_path = auths_dir.join(&file_name);
        expected_auth_files.insert(file_name.clone());
        adopt_sidecar_agent_identity_task(&mut account, &auth_path)?;
        let auth_json = sidecar_auth_json_for_account_with_metered_feature_patterns(
            &account,
            collection,
            effective_proxy_url_ref,
            &metered_feature_patterns,
        );
        let auth_content = serde_json::to_string_pretty(&auth_json)
            .map_err(|e| format!("序列化 sidecar Codex OAuth 认证失败: {}", e))?;
        write_string_atomic_if_changed(&auth_path, &auth_content)?;
        harden_sidecar_auth_file_permissions(&auth_path)?;
        manifest_accounts.push(sidecar_account_manifest_value(
            &account,
            Some(&file_name),
            collection,
        ));
    }
    remove_stale_sidecar_auth_files(&auths_dir, &expected_auth_files)?;

    let mut model_ids = visible_codex_model_ids_for_collection(collection, Some(&health_snapshot));
    let mut model_id_keys = model_ids
        .iter()
        .map(|model| model.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for api_key in &collection.api_keys {
        let Some(model_routing) = api_key.model_routing.as_ref() else {
            continue;
        };
        for route in &model_routing.routes {
            for upstream_model in &route.provider_gateway.upstream_models {
                let client_model = format!("{}/{}", route.namespace, upstream_model.trim());
                if !upstream_model.trim().is_empty()
                    && model_id_keys.insert(client_model.to_ascii_lowercase())
                {
                    model_ids.push(client_model);
                }
            }
        }
    }
    let app_locale = crate::modules::config::get_user_config().language;
    let manifest = json!({
        "locale": app_locale,
        "apiKeys": sidecar_api_key_manifest_values(collection),
        "accounts": manifest_accounts,
        "modelIds": model_ids,
        "modelAliases": collection.model_aliases.iter().map(|alias| json!({
            "sourceModel": alias.source_model.clone(),
            "alias": alias.alias.clone(),
            "fork": alias.fork,
        })).collect::<Vec<_>>(),
        "excludedModels": collection.excluded_models.clone(),
        "routingStrategy": sidecar_routing_strategy_value(collection.routing_strategy),
        "customRoutingRules": collection.custom_routing_rules.iter().map(|rule| json!({
            "accountId": rule.account_id.clone(),
            "priority": rule.priority,
            "weight": rule.weight,
            "isBackup": rule.is_backup,
            "isPreferred": rule.is_preferred,
        })).collect::<Vec<_>>(),
        "accountModelRules": collection.account_model_rules.iter().map(|rule| json!({
            "accountId": rule.account_id.clone(),
            "excludedModels": rule.excluded_models.clone(),
        })).collect::<Vec<_>>(),
        "debugLogs": collection.debug_logs,
        "immediateSseResponse": collection.immediate_sse_response,
        "maxConcurrentImageRequests": collection.max_concurrent_image_requests,
    });

    let mut config = Map::new();
    config.insert(
        "host".to_string(),
        json!(bind_host_for_collection(collection)),
    );
    config.insert("port".to_string(), json!(collection.port));
    config.insert(
        "auth-dir".to_string(),
        json!(auths_dir.to_string_lossy().to_string()),
    );
    config.insert("debug".to_string(), json!(collection.debug_logs));
    config.insert(
        "api-keys".to_string(),
        json!(sidecar_client_api_keys(collection, &account_overrides)),
    );
    config.insert(
        "api-key-account-ids".to_string(),
        sidecar_api_key_account_scope_values(collection, &account_overrides),
    );
    config.insert(
        "auth-error-localization".to_string(),
        json!({
            "default-locale": app_locale,
            "auth-unavailable": sidecar_localized_messages(
                "codex.localAccess.gatewayErrors.authUnavailable",
            ),
            "auth-not-found": sidecar_localized_messages(
                "codex.localAccess.gatewayErrors.authNotFound",
            ),
        }),
    );
    config.insert("request-log".to_string(), json!(false));
    config.insert("logging-to-file".to_string(), json!(false));
    config.insert("commercial-mode".to_string(), json!(true));
    config.insert(
        "codex".to_string(),
        json!({
            "optimize-multi-agent-v2": true,
            "stream-bootstrap-buffering": api_service,
            "api-service-compatibility": api_service,
        }),
    );
    config.insert("ws-auth".to_string(), json!(true));
    config.insert("disable-auth-auto-refresh".to_string(), json!(true));
    // 不写 disable-image-generation：默认允许生图（绑定 OAuth 与改前一致；纯 API Key 也靠正常注入/上游能力）。
    config.insert(
        "request-retry".to_string(),
        json!(MAX_REQUEST_RETRY_ATTEMPTS as i32),
    );
    let timeouts = collection_timeouts(collection);
    config.insert(
        "streaming".to_string(),
        json!({
            "keepalive-seconds": timeouts.sidecar_stream_keepalive_seconds,
            "bootstrap-retries": timeouts.sidecar_streaming_bootstrap_retries,
            "bootstrap-retry-base-delay-ms": timeouts.single_account_status_retry_base_delay_ms,
            "bootstrap-retry-max-delay-ms": timeouts.single_account_status_retry_max_delay_ms,
            "stream-open-timeout-ms": timeouts.sidecar_stream_open_timeout_ms,
            "stream-idle-timeout-ms": timeouts.sidecar_stream_idle_timeout_ms,
            "image-stream-open-timeout-ms": timeouts.sidecar_image_stream_open_timeout_ms,
            "image-stream-idle-timeout-ms": timeouts.sidecar_image_stream_idle_timeout_ms,
            "stream-open-max-attempts": timeouts.sidecar_stream_open_max_attempts,
        }),
    );
    config.insert(
        "max-retry-credentials".to_string(),
        json!(collection.max_retry_credentials as i32),
    );
    config.insert(
        "max-retry-interval".to_string(),
        json!(((collection.max_retry_interval_ms + 999) / 1000) as i32),
    );
    config.insert(
        "disable-cooling".to_string(),
        json!(collection.disable_cooling),
    );
    config.insert(
        "routing".to_string(),
        json!({
            "strategy": "round-robin",
            "session-affinity": collection.session_affinity,
            "session-affinity-ttl": sidecar_duration_ms(collection.session_affinity_ttl_ms),
        }),
    );
    if let Some(proxy_url) = effective_proxy_url_ref {
        config.insert("proxy-url".to_string(), json!(proxy_url));
    }
    if !codex_keys.is_empty() {
        config.insert("codex-api-key".to_string(), Value::Array(codex_keys));
    }
    if !collection.excluded_models.is_empty() {
        config.insert(
            "oauth-excluded-models".to_string(),
            json!({ "codex": collection.excluded_models.clone() }),
        );
    }
    if !collection.model_aliases.is_empty() {
        config.insert(
            "oauth-model-alias".to_string(),
            json!({ "codex": sidecar_model_alias_values(collection) }),
        );
    }
    config.insert(
        "codex-header-defaults".to_string(),
        json!({
            "user-agent": DEFAULT_CODEX_USER_AGENT,
            "beta-features": CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE,
        }),
    );
    if let Some(payload) = sidecar_payload_default_service_tier(default_service_tier) {
        config.insert("payload".to_string(), payload);
    }

    let config_path = sidecar_config_path(&base_dir);
    let manifest_path = sidecar_manifest_path(&base_dir);
    let quota_reserve_path = sidecar_quota_reserve_path(&base_dir);
    let quota_pool_path = sidecar_quota_pool_path(&base_dir);
    let config_content = serde_json::to_string_pretty(&Value::Object(config))
        .map_err(|e| format!("序列化 sidecar 配置失败: {}", e))?;
    let manifest_content = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("序列化 sidecar manifest 失败: {}", e))?;
    let fingerprint = sidecar_config_fingerprint(&config_content, &manifest_content);
    write_string_atomic_if_changed(&config_path, &config_content)?;
    write_string_atomic_if_changed(&manifest_path, &manifest_content)?;
    write_sidecar_api_key_priority_state_in_dir(collection, &base_dir)?;
    write_sidecar_quota_reserve_state_in_dir(collection, &base_dir)?;
    write_sidecar_quota_pool_state_in_dir(collection, &base_dir)?;

    Ok(SidecarLaunchConfig {
        config_path,
        manifest_path,
        quota_reserve_path,
        quota_pool_path,
        fingerprint,
        proxy_signature,
    })
}
