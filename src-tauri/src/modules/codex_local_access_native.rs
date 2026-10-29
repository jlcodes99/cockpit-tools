// Host integration for the independently implemented native Responses engine.
// Reuses account storage and metrics, never the retired gateway's wire logic.
use crate::modules::codex_native_api::{self as native_api, contract as native_contract};

const NATIVE_PRIMARY_CALLER: &str = "native:primary";
const NATIVE_INTERNAL_CALLER: &str = "native:internal";
const NATIVE_ROUTING_CURSOR_LIMIT: usize = 1024;
const NATIVE_SELECTION_GATE_LIMIT: usize = 10_000;
const NATIVE_SOFT_BINDING_LIMIT: usize = 65_536;
static NATIVE_SYSTEM_PROXY_IO: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(1)));
static NATIVE_API_RUNNING: AtomicBool = AtomicBool::new(false);
static NATIVE_PROFILE_RUNNING: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
static NATIVE_ACCOUNT_IO: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(4)));

pub(crate) fn native_api_running() -> bool {
    NATIVE_API_RUNNING.load(Ordering::Acquire) || NATIVE_PROFILE_RUNNING.load(Ordering::Acquire) > 0
}

async fn ensure_native_gateway_locked(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let bind_host = bind_host_for_collection(collection);
    let fingerprint = format!(
        "native:{}:{}:{}:{}",
        bind_host,
        collection.port,
        collection.timeouts.legacy_request_read_timeout_ms,
        collection.timeouts.legacy_stream_idle_timeout_ms
    );
    {
        let runtime = gateway_runtime().lock().await;
        if runtime.native_handle.as_ref().is_some_and(|handle| {
            !handle.is_finished()
                && handle.addr.port() == collection.port
                && handle.addr.ip().to_string() == bind_host
        }) && runtime.sidecar_config_fingerprint.as_deref() == Some(fingerprint.as_str())
        {
            return Ok(());
        }
        if runtime
            .native_handle
            .as_ref()
            .is_some_and(|handle| !handle.pause_if_idle())
        {
            return Err("NATIVE_API_BUSY".into());
        }
    }
    let (endpoint, stopped) = stop_gateway_locked_checked("native_start").await;
    stopped?;
    if let Some(endpoint) = endpoint {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }
    let addr = format!("{bind_host}:{}", collection.port)
        .parse()
        .map_err(|_| "NATIVE_API_INVALID_ADDRESS".to_string())?;
    let path = native_global_identity_path()?;
    let options = native_api::Options {
        session_identity_mapping_enabled: collection.session_identity_mapping_enabled,
        request_timeout: Duration::from_millis(
            collection
                .timeouts
                .legacy_request_read_timeout_ms
                .clamp(1000, 120_000),
        ),
        idle_timeout: Duration::from_millis(
            collection
                .timeouts
                .legacy_stream_idle_timeout_ms
                .clamp(1000, 1_800_000),
        ),
        ..Default::default()
    };
    let host = Arc::new(NativeHostBridge::default());
    native_contract::Host::set_cooling_disabled(host.as_ref(), collection.disable_cooling);
    native_contract::Host::set_session_affinity_enabled(host.as_ref(), collection.session_affinity);
    let handle = native_api::start(addr, host.clone(), path, options).await?;
    let mut runtime = gateway_runtime().lock().await;
    runtime.actual_port = Some(handle.addr.port());
    runtime.actual_bind_host = Some(bind_host.to_string());
    runtime.native_handle = Some(handle);
    runtime.native_health_source = Some(Arc::downgrade(&host));
    runtime.sidecar_config_fingerprint = Some(fingerprint);
    runtime.applied_collection = Some(collection.clone());
    runtime.running = true;
    NATIVE_API_RUNNING.store(true, Ordering::Release);
    runtime.last_error = None;
    logger::log_codex_api_info(&format!(
        "[NativeAPI] Listening on {bind_host}:{}",
        collection.port
    ));
    Ok(())
}

async fn set_runtime_gateway_mode(mode: CodexLocalAccessGatewayMode) {
    if let Some(collection) = gateway_runtime().lock().await.collection.as_mut() {
        collection.gateway_mode = mode;
        collection.native_gateway_default_migrated = true;
    }
}

async fn persist_native_gateway_selection(mode: CodexLocalAccessGatewayMode) -> Result<(), String> {
    let path = local_access_file_path()?;
    tokio::task::spawn_blocking(move || {
        update_string_atomic(&path, |current| {
            native_patch_mode(
                current.ok_or_else(|| "API service configuration is missing".to_string())?,
                mode,
            )
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn native_patch_mode(current: &str, mode: CodexLocalAccessGatewayMode) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(current).map_err(|error| error.to_string())?;
    if !value.is_object() {
        return Err("Invalid API service configuration".into());
    }
    if mode != CodexLocalAccessGatewayMode::Native && native_json_has_chatgpt_plan_reference(&value)
    {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    value["gatewayMode"] = serde_json::to_value(mode).map_err(|error| error.to_string())?;
    value["nativeGatewayDefaultMigrated"] = json!(true);
    value["updatedAt"] = json!(now_ms());
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

fn native_json_has_chatgpt_plan_reference(value: &Value) -> bool {
    if value
        .get("officialAccountBindings")
        .and_then(Value::as_object)
        .is_some_and(|map| !map.is_empty())
    {
        return true;
    }
    ["accountIds", "imageGenerationAccountIds"]
        .iter()
        .any(|field| {
            value
                .get(field)
                .and_then(Value::as_array)
                .is_some_and(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .any(|id| id.starts_with("chatgpt_plan_"))
                })
        })
        || value
            .get("apiKeys")
            .and_then(Value::as_array)
            .is_some_and(|keys| {
                keys.iter().any(|key| {
                    native_json_has_chatgpt_plan_reference(key)
                        || key
                            .pointer("/modelRouting/routes")
                            .and_then(Value::as_array)
                            .is_some_and(|routes| {
                                routes.iter().any(|route| {
                                    route
                                        .get("providerAccountId")
                                        .and_then(Value::as_str)
                                        .is_some_and(|id| id.starts_with("chatgpt_plan_"))
                                })
                            })
                })
            })
}

fn native_validate_chatgpt_plan_mode(
    collection: &CodexLocalAccessCollection,
    mode: CodexLocalAccessGatewayMode,
) -> Result<(), String> {
    if mode != CodexLocalAccessGatewayMode::Native
        && (!collection.official_account_bindings.is_empty()
            || effective_sidecar_account_ids(collection)
                .iter()
                .any(|id| id.starts_with("chatgpt_plan_")))
    {
        return Err("CHATGPT_PLAN_NATIVE_REQUIRED".into());
    }
    Ok(())
}

async fn switch_native_aware_gateway(mode: CodexLocalAccessGatewayMode) -> Result<(), String> {
    ensure_runtime_loaded_without_start().await?;
    let mode = if mode == CodexLocalAccessGatewayMode::Legacy {
        CodexLocalAccessGatewayMode::Sidecar
    } else {
        mode
    };
    let _guard = timeout(Duration::from_secs(30), gateway_lifecycle_lock().lock())
        .await
        .map_err(|_| "NATIVE_API_SWITCH_BUSY".to_string())?;
    let previous = {
        let runtime = gateway_runtime().lock().await;
        let collection = runtime
            .collection
            .as_ref()
            .ok_or_else(|| "API service configuration is missing".to_string())?;
        // Reject before pausing or stopping the existing listener. Neither a
        // failed switch nor a stopped service may activate the sidecar grant.
        native_validate_chatgpt_plan_mode(collection, mode)?;
        let previous = collection.gateway_mode;
        if previous == mode {
            return Ok(());
        }
        if runtime
            .native_handle
            .as_ref()
            .is_some_and(|handle| !handle.pause_if_idle())
        {
            return Err("NATIVE_API_BUSY".into());
        }
        previous
    };
    advance_gateway_lifecycle_generation();
    let (endpoint, stopped) = stop_gateway_locked_checked("engine_switch").await;
    let result = async {
        stopped?;
        if let Some(endpoint) = endpoint {
            wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
        }
        set_runtime_gateway_mode(mode).await;
        // Deliberately bypass the random-port recovery wrapper: the public endpoint must remain stable.
        ensure_gateway_matches_runtime_once_locked().await?;
        persist_native_gateway_selection(mode).await
    }
    .await;
    if let Err(error) = result {
        let (endpoint, _) = stop_gateway_locked_checked("engine_switch_rollback").await;
        if let Some(endpoint) = endpoint {
            let _ = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await;
        }
        set_runtime_gateway_mode(previous).await;
        let restored = ensure_gateway_matches_runtime_once_locked().await;
        let restored_ok = restored.is_ok();
        let message = match restored {
            Ok(()) => format!("NATIVE_API_SWITCH_FAILED: {error}"),
            Err(rollback) => format!("NATIVE_API_ROLLBACK_FAILED: {error}; {rollback}"),
        };
        gateway_runtime().lock().await.last_error = if restored_ok {
            None
        } else {
            Some(message.clone())
        };
        emit_local_access_state_updated();
        return Err(message);
    }
    emit_local_access_state_updated();
    Ok(())
}

#[derive(Default)]
struct NativeHostBridge {
    collection_override: Option<CodexLocalAccessCollection>,
    profile_dir: Option<PathBuf>,
    account_overrides: HashMap<String, CodexAccount>,
    bindings: TokioMutex<HashMap<String, (String, i64)>>,
    session_affinity_enabled: AtomicBool,
    binding_epoch: AtomicU64,
    binding_cache_epoch: AtomicU64,
    binding_lru: Mutex<std::collections::BTreeMap<(i64, String), ()>>,
    routing_cursors: Mutex<HashMap<String, (String, u64)>>,
    selection_gates: Mutex<HashMap<String, std::sync::Weak<TokioMutex<()>>>>,
    gates: Mutex<HashMap<String, Arc<Semaphore>>>,
    account_waiters: AtomicU64,
    cooldowns: Mutex<HashMap<NativeCooldownKey, NativeCooldown>>,
    cooling_disabled: AtomicBool,
    clients: Mutex<NativeHttpClientCache>,
    system_proxy_cache: TokioMutex<Option<(Instant, Option<String>)>>,
    #[cfg(test)]
    routing_sequence: AtomicU64,
    #[cfg(test)]
    identity_fixture: Arc<NativeIdentityFixture>,
}

#[cfg(test)]
include!("codex_local_access_native_identity_fixture.rs");

include!("codex_local_access_native_cooldown.rs");
include!("codex_local_access_native_health.rs");
include!("codex_local_access_native_retry.rs");
include!("codex_local_access_native_official_fallback.rs");

fn native_official_report_attempt(
    report: &native_contract::RequestReport,
) -> Option<&native_api::diagnostics::DiagnosticAttempt> {
    let context = report.diagnostics.as_ref()?;
    if context.failure_phase.as_deref().is_some_and(|phase| phase != "upstream") {
        return None;
    }
    let attempt = context.attempts.last()?;
    if attempt.account_id != report.account_id
        || attempt.model_id != report.upstream_model
        || attempt.authorization_route.as_deref() != Some("official")
        || matches!(attempt.transport.as_str(), "" | "preflight")
    {
        return None;
    }
    match attempt.status? {
        200..=299 if report.success && attempt.success => Some(attempt),
        401 | 403 | 429 | 500..=599
            if !report.success && !attempt.success && attempt.failure_phase.as_deref() == Some("upstream") =>
        {
            Some(attempt)
        }
        _ => None,
    }
}

fn native_recorded_failure_cools_account(report: &native_contract::RequestReport) -> bool {
    // The final report must keep the transport's request-fault classification.
    // Retaining a soft binding is ineffective if recording the same failure
    // subsequently cools that account and forces its next request elsewhere.
    if report
        .diagnostics
        .as_ref()
        .is_some_and(|context| context.upstream_request_scoped)
        || matches!(
            report.error.as_deref(),
            Some(
                "upstream_request_scoped"
                    | "request_scoped"
                    | "connection_lifecycle"
                    | "client_cancelled"
                    | "client_disconnected"
                    | "client_interrupted"
            )
        )
    {
        return false;
    }
    if report
        .diagnostics
        .as_ref()
        .is_some_and(|d| d.endpoint.ends_with("/responses/compact"))
        && !matches!(report.status, 401 | 402 | 403 | 429)
    {
        return false;
    }
    if !report.success && !report.account_id.is_empty() && native_report_model_unavailable(report) {
        return true;
    }
    if report.success
        || report.account_id.is_empty()
        || !matches!(report.status, 401 | 402 | 403 | 404 | 408 | 429 | 500..=599)
    {
        return false;
    }
    if matches!(report.status, 401 | 402 | 403 | 429) {
        return true;
    }
    // These statuses were generated by this gateway, not returned by the
    // account's upstream. A shared network outage must not drain the pool.
    !matches!(
        report.error.as_deref(),
        Some(
            "upstream_unavailable"
                | "account_cooling"
                | "upstream_stream_error"
                | "incomplete_upstream_stream"
                | "upstream_connect_timeout"
                | "upstream_idle_timeout"
                | "upstream_total_timeout"
                | "upstream_timeout"
                | "websocket_connect_failed"
                | "websocket_connect_timeout"
                | "websocket_initial_timeout"
                | "websocket_send_failed"
                | "websocket_send_timeout"
                | "realtime_upstream_disconnected"
                | "realtime_client_disconnected"
                | "realtime_write_failed"
                | "realtime_write_timeout"
                | "service_stopping"
                | "blocking_capacity_timeout"
                | "blocking_operation_timeout"
                | "blocking_operation_failed"
                | "execution_policy_timeout"
                | "request_policy_timeout"
                | "routing_timeout"
                | "image_routing_timeout"
                | "identity_store_unavailable"
                | "identity_store_full"
                | "recovery_store_unavailable"
        )
    )
}

fn native_report_model_unavailable(report: &native_contract::RequestReport) -> bool {
    !report
        .diagnostics
        .as_ref()
        .is_some_and(|d| d.endpoint.ends_with("/responses/compact"))
        && (report.error.as_deref() == Some("upstream_model_unavailable")
            || native_api::upstream_errors::model_unavailable(
                report.status,
                &serde_json::json!({"error":{"code":report.error}}),
                &report.upstream_model,
            ))
}

impl NativeHostBridge {
    fn record_failure_cooldown(&self, report: &native_contract::RequestReport) {
        if report.official_registration_id.is_some() {
            return;
        }
        if self.cooling_disabled.load(Ordering::Acquire) {
            return;
        }
        if report.success {
            if let Ok(mut states) = self.cooldowns.lock() {
                let credential = native_cooldown_key(&report.account_id, "", true);
                if !states
                    .get(&credential)
                    .is_some_and(|state| state.quota && state.until > Instant::now())
                {
                    states.remove(&native_cooldown_key(
                        &report.account_id,
                        &report.upstream_model,
                        false,
                    ));
                    states.remove(&credential);
                }
            }
            return;
        }
        if !native_recorded_failure_cools_account(report) {
            return;
        }
        if let Ok(mut cooldowns) = self.cooldowns.lock() {
            if self.cooling_disabled.load(Ordering::Acquire) {
                return;
            }
            if report
                .error_message
                .as_deref()
                .is_some_and(native_api::diagnostics::is_cloudflare_challenge)
            {
                native_upsert_challenge_cooldown(
                    &mut cooldowns,
                    &report.account_id,
                    &report.upstream_model,
                    Instant::now(),
                );
                return;
            }
            native_upsert_cooldown(
                &mut cooldowns,
                &report.account_id,
                &report.upstream_model,
                if native_report_model_unavailable(report) {
                    404
                } else {
                    report.status
                },
                report.retry_after,
                native_report_credential_scope(report),
                Instant::now(),
            );
        }
    }

    fn for_collection(collection: CodexLocalAccessCollection) -> Self {
        Self {
            cooling_disabled: AtomicBool::new(collection.disable_cooling),
            session_affinity_enabled: AtomicBool::new(collection.session_affinity),
            collection_override: Some(collection),
            ..Default::default()
        }
    }
    async fn collection(
        &self,
    ) -> Result<CodexLocalAccessCollection, native_contract::ServiceError> {
        match &self.collection_override {
            Some(collection) => Ok(collection.clone()),
            None => native_collection().await,
        }
    }
}

fn native_error(status: u16, code: &'static str, message: &str) -> native_contract::ServiceError {
    native_contract::ServiceError::new(status, code, message)
}

async fn native_collection() -> Result<CodexLocalAccessCollection, native_contract::ServiceError> {
    gateway_runtime()
        .lock()
        .await
        .collection
        .clone()
        .ok_or_else(|| native_error(503, "service_unavailable", "API service is not configured"))
}

fn native_key_matches(expected: &str, supplied: &str) -> bool {
    if expected.is_empty() || supplied.is_empty() {
        return false;
    }
    let mut difference = expected.len() ^ supplied.len();
    for (index, byte) in expected.bytes().enumerate() {
        difference |= usize::from(byte ^ supplied.as_bytes().get(index).copied().unwrap_or(0));
    }
    difference == 0
}

fn native_caller_key<'a>(
    collection: &'a CodexLocalAccessCollection,
    caller: &native_contract::Caller,
) -> Result<Option<&'a CodexLocalAccessApiKey>, native_contract::ServiceError> {
    if caller.id == NATIVE_INTERNAL_CALLER {
        return Ok(None);
    }
    if !collection.enabled {
        return Err(native_error(
            503,
            "service_disabled",
            "API service is disabled",
        ));
    }
    if caller.id == NATIVE_PRIMARY_CALLER && collection.api_keys.is_empty() {
        return Ok(None);
    }
    let key = collection
        .api_keys
        .iter()
        .find(|key| key.id == caller.id && key.enabled)
        .ok_or_else(|| {
            native_error(
                401,
                "invalid_api_key",
                "API key is disabled or no longer exists",
            )
        })?;
    if key.token_limit.is_some_and(|limit| key.token_used >= limit) {
        return Err(native_error(
            429,
            "token_limit_reached",
            "API key token limit reached",
        ));
    }
    Ok(Some(key))
}

fn native_pattern_matches(pattern: &str, value: &str) -> bool {
    // Bounded wildcard matching, with '*' as the only special character.
    // Go uses per-code-point lowercase, without contextual final-sigma folding.
    let lower = |value: &str| {
        value
            .trim()
            .chars()
            .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
            .collect::<String>()
    };
    let (pattern, value) = (lower(pattern), lower(value));
    if pattern.is_empty() || value.is_empty() {
        return false;
    }
    let (pattern, value) = (pattern.as_bytes(), value.as_bytes());
    let (mut p, mut v, mut star, mut retry) = (0, 0, None, 0);
    while v < value.len() {
        if p < pattern.len() && pattern[p] == value[v] {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            retry = v;
        } else if let Some(position) = star {
            retry += 1;
            v = retry;
            p = position + 1;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

fn native_excluded(patterns: &[String], model: &str) -> bool {
    !model.is_empty()
        && patterns
            .iter()
            .any(|pattern| native_pattern_matches(pattern, model))
}

fn native_scoped_account_ids(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
) -> Vec<String> {
    active_service_account_ids(
        collection,
        match key {
            Some(key)
                if !key
                    .inherit_account_pool
                    .unwrap_or(key.account_ids.is_empty()) =>
            {
                key.account_ids.clone()
            }
            _ => collection.account_ids.clone(),
        },
    )
}

fn native_quota_allows(collection: &CodexLocalAccessCollection, account: &CodexAccount) -> bool {
    let now = now_ms();
    if account_quota_cooldown(account, now).is_some_and(|snapshot| snapshot.active(now)) {
        return false;
    }
    native_bound_quota_reserve_allows(collection, account)
}

fn native_is_reserve_model(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("gpt-reserve")
}

fn native_model_quota_allows(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    model: &str,
) -> bool {
    // Official subscription sharing has its own route health and upstream
    // limits. Ordinary account quota checks apply to raw local credentials.
    if native_is_chatgpt_plan_account(account) {
        return true;
    }
    if native_is_reserve_model(model) && !account.is_api_key_auth() {
        account_has_gpt_reserve_entitlement(account)
            && native_bound_quota_reserve_allows(collection, account)
    } else {
        native_quota_allows(collection, account)
    }
}

fn native_image_allows(collection: &CodexLocalAccessCollection, account: &CodexAccount) -> bool {
    if native_is_chatgpt_plan_account(account)
        || collection.image_generation_mode != CodexLocalAccessImageGenerationMode::Enabled
    {
        return false;
    }
    match collection
        .image_generation_account_policies
        .get(&account.id)
        .copied()
        .unwrap_or_default()
    {
        CodexLocalAccessImageGenerationPolicy::Enabled => true,
        CodexLocalAccessImageGenerationPolicy::Disabled => false,
        CodexLocalAccessImageGenerationPolicy::Inherit => !account.is_api_key_auth(),
    }
}

fn native_model_for_key(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    model: &str,
) -> Result<String, native_contract::ServiceError> {
    if model.is_empty() {
        return Ok(String::new());
    }
    let model = if let Some(prefix) = key
        .and_then(|key| key.model_prefix.as_deref())
        .filter(|p| !p.is_empty())
    {
        model
            .strip_prefix(&format!("{}/", prefix.trim_end_matches('/')))
            .ok_or_else(|| {
                native_error(
                    403,
                    "model_not_allowed",
                    "Model is outside this API key namespace",
                )
            })?
    } else {
        model
    };
    let base_model = native_api::gpt_thinking::model_name(model);
    if native_excluded(&collection.excluded_models, model)
        || native_excluded(&collection.excluded_models, base_model)
        || key.is_some_and(|key| {
            native_excluded(&key.excluded_models, model)
                || native_excluded(&key.excluded_models, base_model)
                || (!key.allowed_models.is_empty()
                    && !native_excluded(&key.allowed_models, model)
                    && !native_excluded(&key.allowed_models, base_model))
        })
    {
        return Err(native_error(
            403,
            "model_not_allowed",
            "Model is not allowed for this API key",
        ));
    }
    let canonical = collection
        .model_aliases
        .iter()
        .find(|alias| alias.alias.eq_ignore_ascii_case(base_model))
        .map(|alias| alias.source_model.as_str())
        .unwrap_or(base_model);
    if native_excluded(&collection.excluded_models, canonical)
        || key.is_some_and(|key| native_excluded(&key.excluded_models, canonical))
    {
        return Err(native_error(
            403,
            "model_not_allowed",
            "The upstream model is excluded for this API key",
        ));
    }
    Ok(canonical.to_string())
}

fn native_eligible(account: &CodexAccount) -> bool {
    !account.is_web_session_auth()
        && !account.requires_reauth
        && !codex_account::account_has_remote_api_auth_rejection(account)
        && (account.upstream_grok_account_id.is_none()
            || codex_account::is_grok_upstream_provider(account))
        && native_wire_api(account).is_ok()
}

fn native_account_model(account: &CodexAccount, model: &str) -> Option<String> {
    if account.is_api_key_auth() {
        let slots = automatic_api_service_account_model_slots(account);
        if !slots.is_empty() {
            return slots
                .into_iter()
                .find(|(client, _)| client.trim().eq_ignore_ascii_case(model.trim()))
                .map(|(_, upstream)| upstream);
        }
        return (provider_gateway_wire_api_for_account(account) == "responses")
            .then(|| model.to_owned());
    }
    let mapped = account
        .api_model_mappings
        .iter()
        .find(|mapping| mapping.client_model.eq_ignore_ascii_case(model))
        .map(|mapping| mapping.upstream_model.as_str())
        .unwrap_or(model);
    Some(mapped.to_string())
}

async fn native_load_accounts(
    ids: Vec<String>,
) -> Result<Vec<CodexAccount>, native_contract::ServiceError> {
    let permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| native_error(503, "service_unavailable", "Account reader is unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        ids.iter()
            .filter_map(|id| codex_account::load_account(id))
            .collect()
    })
    .await
    .map_err(|_| native_error(500, "account_read_failed", "Unable to read accounts"))
}

fn native_quota(account: &CodexAccount) -> i32 {
    account
        .quota
        .as_ref()
        .map(|q| {
            let hourly = if q.hourly_window_present == Some(false) {
                100
            } else {
                q.hourly_percentage
            };
            let weekly = if q.weekly_window_present == Some(false) {
                100
            } else {
                q.weekly_percentage
            };
            let remaining = hourly.min(weekly);
            if remaining <= 0 && quota_has_usable_credits(q) {
                1
            } else {
                remaining
            }
        })
        .unwrap_or(100)
}

include!("codex_local_access_native_scheduler.rs");

impl native_contract::Host for NativeHostBridge {
    fn observe_official_failure<'a>(
        &'a self,
        route: &'a native_contract::Route,
        model: &'a str,
        failure: &'a native_contract::OfficialFallbackFailure,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(native_observe_official_rejection(route, model, failure))
    }
    fn fallback_to_local<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        failed: &'a native_contract::Route,
        request: native_contract::RouteRequest,
        failure: native_contract::OfficialFallbackFailure,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<Option<native_contract::Route>, native_contract::ServiceError>,
    > {
        Box::pin(native_fallback_to_local(
            self, caller, failed, request, failure,
        ))
    }
    fn invalidate_transport(&self, identity: &str) -> bool {
        self.clients
            .lock()
            .map(|mut cache| cache.invalidate(identity))
            .unwrap_or(false)
    }
    fn multi_agent_policy<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: Option<&'a native_contract::Route>,
        requested_model: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::MultiAgentPolicy, native_contract::ServiceError>,
    > {
        Box::pin(native_multi_agent_policy(
            self,
            caller,
            route,
            requested_model,
        ))
    }
    fn observe_rate_limits(&self, account_id: &str, snapshot: QuotaSnapshot, started: Instant) {
        observe_gateway_quota(account_id, snapshot, started);
    }
    fn quota_request_started(&self, account_id: &str) {
        note_gateway_quota_request(account_id);
    }
    fn check_quota_before_send(
        &self,
        account_id: &str,
    ) -> Result<(), native_contract::ServiceError> {
        let local = self
            .collection_override
            .as_ref()
            .filter(|collection| collection.bound_oauth_account_id.as_deref() == Some(account_id))
            .and_then(|collection| collection.bound_oauth_quota_reserve.as_ref());
        check_gateway_quota_before_send(account_id, self.collection_override.is_none(), local)
    }
    fn set_session_affinity_enabled(&self, enabled: bool) {
        if self
            .session_affinity_enabled
            .swap(enabled, Ordering::AcqRel)
            != enabled
        {
            // Route preparation observes this epoch before reading/publishing
            // bindings. Never block a synchronous configuration callback.
            self.binding_epoch.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn set_cooling_disabled(&self, disabled: bool) {
        self.cooling_disabled.store(disabled, Ordering::Release);
        if disabled {
            if let Ok(mut states) = self.cooldowns.lock() {
                states.clear();
            }
        }
    }
    fn websockets_enabled(
        &self,
    ) -> futures_util::future::BoxFuture<'_, Result<bool, native_contract::ServiceError>> {
        Box::pin(async { Ok(self.collection().await?.responses_websockets_enabled) })
    }
    fn authorize<'a>(
        &'a self,
        bearer: &'a str,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Caller, native_contract::ServiceError>,
    > {
        Box::pin(async move {
            if self.collection_override.is_none()
                && native_key_matches(internal_api_service_key(), bearer)
            {
                return Ok(native_contract::Caller {
                    id: NATIVE_INTERNAL_CALLER.into(),
                    label: "Internal".into(),
                });
            }
            let collection = self.collection().await?;
            if !collection.enabled {
                return Err(native_error(
                    503,
                    "service_disabled",
                    "API service is disabled",
                ));
            }
            let caller = collection
                .api_keys
                .iter()
                .find(|key| native_key_matches(&key.key, bearer))
                .map(|key| native_contract::Caller {
                    id: key.id.clone(),
                    label: key.label.clone(),
                })
                .or_else(|| {
                    (collection.api_keys.is_empty()
                        && native_key_matches(&collection.api_key, bearer))
                    .then(|| native_contract::Caller {
                        id: NATIVE_PRIMARY_CALLER.into(),
                        label: "Default".into(),
                    })
                })
                .ok_or_else(|| native_error(401, "invalid_api_key", "Invalid API key"))?;
            native_caller_key(&collection, &caller)?;
            Ok(caller)
        })
    }

    fn route_failed<'a>(
        &'a self,
        account_id: &'a str,
        status: u16,
        retry_after: Option<Duration>,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(mut cooldowns) = self.cooldowns.lock() {
                if self.cooling_disabled.load(Ordering::Acquire) {
                    return;
                }
                native_upsert_cooldown(
                    &mut cooldowns,
                    account_id,
                    "",
                    status,
                    retry_after,
                    true,
                    Instant::now(),
                );
            }
        })
    }

    fn route_failed_for_model<'a>(
        &'a self,
        account_id: &'a str,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Ok(mut states) = self.cooldowns.lock() {
                if self.cooling_disabled.load(Ordering::Acquire) {
                    return;
                }
                native_upsert_cooldown(
                    &mut states,
                    account_id,
                    model,
                    status,
                    retry_after,
                    credential_scope,
                    Instant::now(),
                );
            }
        })
    }

    fn route_failed_attempt<'a>(
        &'a self,
        route: &'a native_contract::Route,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if route.official_registration_id.is_some() {
                native_observe_official_failure(route, model, status, message, retry_after).await;
                return;
            }
            if self.selected_route_current(route).await {
                self.route_failed_with_context(
                    &route.account_id,
                    model,
                    status,
                    retry_after,
                    credential_scope,
                    message,
                )
                .await;
            }
        })
    }

    fn route_failed_with_context<'a>(
        &'a self,
        account_id: &'a str,
        model: &'a str,
        status: u16,
        retry_after: Option<Duration>,
        credential_scope: bool,
        message: Option<&'a str>,
    ) -> futures_util::future::BoxFuture<'a, ()> {
        Box::pin(async move {
            if message.is_some_and(native_api::diagnostics::is_cloudflare_challenge) {
                if let Ok(mut states) = self.cooldowns.lock() {
                    if self.cooling_disabled.load(Ordering::Acquire) {
                        return;
                    }
                    native_upsert_challenge_cooldown(
                        &mut states,
                        account_id,
                        model,
                        Instant::now(),
                    );
                }
            } else {
                self.route_failed_for_model(
                    account_id,
                    model,
                    status,
                    retry_after,
                    credential_scope,
                )
                .await;
            }
        })
    }

    fn responses_websocket_passthrough<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: &'a native_contract::Route,
    ) -> futures_util::future::BoxFuture<'a, Result<bool, native_contract::ServiceError>> {
        Box::pin(async move {
            let collection = self.collection().await?;
            let key = native_caller_key(&collection, caller)?;
            if route.wire_api == native_contract::WireApi::Chat
                || key.is_some_and(|key| {
                    key.provider_gateway.is_some() || key.model_routing.is_some()
                })
            {
                return Ok(false);
            }
            let account = self.load_route_account(route).await?.ok_or_else(|| {
                native_error(
                    404,
                    "account_not_found",
                    "Selected account no longer exists",
                )
            })?;
            Ok(!native_is_chatgpt_plan_account(&account)
                && (!account.is_api_key_auth() || account.api_supports_websockets))
        })
    }

    fn retry_cooldown_wait<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
        max_wait: Duration,
    ) -> futures_util::future::BoxFuture<'a, Result<Option<Duration>, native_contract::ServiceError>>
    {
        Box::pin(native_retry_cooldown_wait(self, caller, request, max_wait))
    }

    fn execution_policy(
        &self,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<native_contract::ExecutionPolicy, native_contract::ServiceError>,
    > {
        Box::pin(async {
            let collection = self.collection().await?;
            let limits = &collection.timeouts;
            Ok(native_contract::ExecutionPolicy {
                // Rebuilding is native protocol behavior, not a persisted user setting.
                account_switch_rebuild: true,
                open_timeout: Duration::from_millis(
                    limits.sidecar_stream_open_timeout_ms.max(1000),
                ),
                idle_timeout: Duration::from_millis(
                    limits.sidecar_stream_idle_timeout_ms.max(1000),
                ),
                // Go ordinary requests have no total deadline, including active streams.
                total_timeout: Duration::ZERO,
                image_open_timeout: Duration::from_millis(
                    limits.sidecar_image_stream_open_timeout_ms.max(1000),
                ),
                image_idle_timeout: Duration::from_millis(
                    limits.sidecar_image_stream_idle_timeout_ms.max(1000),
                ),
                stream_open_max_attempts: limits.sidecar_stream_open_max_attempts.clamp(1, 3),
                credential_retries: collection.max_retry_credentials,
                retry_max_delay: Duration::from_millis(collection.max_retry_interval_ms),
                send_retries: limits.upstream_send_retry_attempts,
                send_retry_max_delay: Duration::from_millis(
                    limits.upstream_send_retry_max_delay_ms,
                ),
                retry_base_delay: Duration::from_millis(limits.upstream_send_retry_base_delay_ms),
                status_retries: limits.single_account_status_retry_attempts,
                status_retry_base_delay: Duration::from_millis(
                    limits.single_account_status_retry_base_delay_ms,
                ),
                status_retry_max_delay: Duration::from_millis(
                    limits.single_account_status_retry_max_delay_ms,
                ),
                bootstrap_retries: limits.sidecar_streaming_bootstrap_retries,
                keepalive: Duration::from_secs(limits.sidecar_stream_keepalive_seconds.into()),
                immediate_sse: collection.immediate_sse_response,
                websocket_connect_timeout: Duration::from_millis(
                    limits.websocket_connect_timeout_ms.max(1000),
                ),
                websocket_initial_timeout: Duration::from_millis(
                    limits.websocket_initial_message_timeout_ms.max(1000),
                ),
                websocket_idle_timeout: Duration::from_millis(
                    limits.websocket_idle_timeout_ms.max(1000),
                ),
                websocket_heartbeat: Duration::from_millis(limits.websocket_heartbeat_interval_ms),
            })
        })
    }

    fn capability_fallback_decision<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: &'a native_contract::Route,
        request: &'a native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::CapabilityFallbackDecision, native_contract::ServiceError>,
    > {
        Box::pin(native_capability_fallback_decision(
            self, caller, route, request,
        ))
    }

    fn route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        Box::pin(native_select_route(self, caller, request))
    }

    fn refresh_route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        previous: &'a native_contract::Route,
        request: native_contract::RouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::Route, native_contract::ServiceError>,
    > {
        Box::pin(native_refresh_route(self, caller, previous, request))
    }

    fn models<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        Box::pin(native_models(self, caller))
    }

    fn token_count_model<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        model: &'a str,
    ) -> futures_util::future::BoxFuture<'a, Result<String, native_contract::ServiceError>> {
        Box::pin(async move {
            let collection = self.collection().await?;
            let key = native_caller_key(&collection, caller)?;
            native_token_count_model(&collection, key, self.profile_dir.is_some(), model)
        })
    }

    fn payload_logging_enabled(&self) -> futures_util::future::BoxFuture<'_, bool> {
        Box::pin(async { native_request_payload_logging_enabled() })
    }
    fn diagnostic(
        &self,
        context: NativeDiagnosticContext,
    ) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            native_record_diagnostic(context);
        })
    }
    fn management<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: NativeManagementRequest,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        Box::pin(native_management(self, caller, request))
    }

    fn image_route<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        request: native_contract::ImageRouteRequest,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<native_contract::ImageRoute, native_contract::ServiceError>,
    > {
        Box::pin(native_image_route(self, caller, request))
    }

    fn prepare_body<'a>(
        &'a self,
        caller: &'a native_contract::Caller,
        route: &'a native_contract::Route,
        headers: &'a axum::http::HeaderMap,
        body: Value,
        dedicated_image: bool,
        compact: bool,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, native_contract::ServiceError>> {
        Box::pin(async move {
            let collection = self.collection().await?;
            let key = native_caller_key(&collection, caller)?;
            if route
                .authorization_scope_fingerprint
                .as_deref()
                .is_some_and(|scope| {
                    native_official_scope_fingerprint(&collection, caller) != scope
                })
            {
                return Err(native_error(
                    409,
                    "official_fallback_authorization_changed",
                    "API key or model authorization changed before sending",
                ));
            }
            let account = self.load_route_account(route).await?.ok_or_else(|| {
                native_error(
                    404,
                    "account_not_found",
                    "Selected account no longer exists",
                )
            })?;
            let body = native_prepare_provider_vision_body(&collection, key, &account, body);
            Ok(native_prepare_image_body(
                &collection,
                &account,
                headers,
                body,
                dedicated_image,
                compact,
            ))
        })
    }

    fn record(
        &self,
        report: native_contract::RequestReport,
    ) -> futures_util::future::BoxFuture<'_, ()> {
        Box::pin(async move {
            let official_attempt = native_official_report_attempt(&report);
            let official_feedback = (report.official_registration_id.is_none()
                && !report.account_id.starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX))
                || official_attempt.is_some();
            if let (Some(id), Some(revision), Some(fingerprint), Some(attempt)) = (
                report.official_registration_id.as_deref().or_else(|| {
                    report
                        .account_id
                        .strip_prefix(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                }),
                report.official_authorization_revision.as_deref(),
                report.official_access_fingerprint.as_deref(),
                official_attempt,
            ) {
                crate::modules::chatgpt_plan::observe_inference_result(
                    id,
                    revision,
                    fingerprint,
                    attempt.status.expect("upstream status checked"),
                    report
                        .official_authorization_error
                        .as_deref()
                        .or(attempt.error_category.as_deref()),
                )
                .await;
                if report.success
                    && !report
                        .account_id
                        .starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                {
                    native_note_official_health(
                        crate::modules::chatgpt_plan::record_official_route_success(
                            id,
                            &report.account_id,
                            revision,
                            &report.upstream_model,
                            attempt.started_at_ms,
                            fingerprint,
                        )
                        .await,
                    );
                } else if !report.success
                    && !report
                        .account_id
                        .starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                    && (!report
                        .diagnostics
                        .as_ref()
                        .is_some_and(|context| context.upstream_request_scoped)
                        || attempt
                            .error_category
                            .as_deref()
                            .is_some_and(|code| code.contains("not_eligible")))
                {
                    if let Ok(snapshot) = crate::modules::chatgpt_plan::official_route_snapshot(
                        id,
                        &report.account_id,
                        &report.upstream_model,
                    )
                    .await
                    {
                        if snapshot.authorization_revision == revision {
                            native_note_official_health(
                                crate::modules::chatgpt_plan::record_official_route_failure(
                                    &snapshot,
                                    &report.upstream_model,
                                    attempt.status.expect("upstream status checked"),
                                    report
                                        .official_authorization_error
                                        .as_deref()
                                        .or(attempt.error_category.as_deref())
                                        .unwrap_or("official_unavailable"),
                                    attempt.error_message.as_deref(),
                                    report.retry_after,
                                    Some(fingerprint),
                                )
                                .await,
                            );
                        }
                    }
                }
            }
            let current = if let (Some(id), Some(revision)) = (
                &report.official_fallback_registration_id,
                &report.official_fallback_authorization_revision,
            ) {
                self.selected_fallback_current(&report.account_id, id, revision)
                    .await
            } else {
                self.selected_authorization_current(
                    &report.account_id,
                    report.official_registration_id.as_deref(),
                    report.official_authorization_revision.as_deref(),
                )
                .await
            };
            if current && official_feedback {
                self.record_failure_cooldown(&report);
                self.record_soft_binding_result(&report).await;
            }
            let diagnostics = report.diagnostics.as_ref();
            let proxy_route = diagnostics
                .and_then(|d| d.proxy_route.clone())
                .and_then(|value| serde_json::from_value::<CodexLocalAccessProxyRoute>(value).ok());
            let request_kind = match report.operation {
                "image_generation" => CodexLocalAccessRequestKind::ImageGeneration,
                "image_edit" => CodexLocalAccessRequestKind::ImageEdit,
                "other" => CodexLocalAccessRequestKind::Other,
                _ => diagnostics
                    .map(|d| d.endpoint.as_str())
                    .map(|path| {
                        if path.ends_with("images/generations") {
                            CodexLocalAccessRequestKind::ImageGeneration
                        } else if path.ends_with("images/edits") {
                            CodexLocalAccessRequestKind::ImageEdit
                        } else if path.contains("audio")
                            || path.contains("realtime")
                            || path.contains("transcribe")
                            || path.contains("/live")
                        {
                            CodexLocalAccessRequestKind::Other
                        } else {
                            CodexLocalAccessRequestKind::Text
                        }
                    })
                    .unwrap_or(CodexLocalAccessRequestKind::Text),
            };
            let usage = report.usage.as_ref().map(native_usage_capture);
            if let Err(error) = record_request_stats_with_meta(
                (!report.account_id.is_empty()).then_some(report.account_id.as_str()),
                None,
                Some(&report.caller.id),
                Some(&report.caller.label),
                Some(&report.upstream_model),
                request_kind,
                report.success,
                (!report.success)
                    .then_some(report.error.as_deref().unwrap_or("native_request_failed")),
                report.elapsed.as_millis() as u64,
                usage,
                RequestStatsMeta {
                    authorization_route: Some(if report.official_registration_id.is_some() {
                        "official"
                    } else {
                        "local_gateway"
                    }),
                    gateway_mode: Some(CodexLocalAccessGatewayMode::Native),
                    proxy_route: proxy_route.as_ref(),
                    request_id: diagnostics.map(|d| d.request_id.as_str()),
                    client_instance_id: diagnostics.and_then(|d| d.client_instance_id.as_deref()),
                    first_response_ms: diagnostics.and_then(|d| d.first_response_ms),
                    output_generation_ms: diagnostics.and_then(|d| d.output_generation_ms),
                    service_tier: report
                        .response_service_tier
                        .as_deref()
                        .or(report.requested_service_tier.as_deref()),
                    requested_service_tier: report.requested_service_tier.as_deref(),
                    response_service_tier: report.response_service_tier.as_deref(),
                    reasoning_effort: report.reasoning_effort.as_deref(),
                    http_status: Some(report.status),
                    error_message: report.error_message.as_deref().or(report.error.as_deref()),
                    requested_model: Some(&report.requested_model),
                    upstream_model: Some(&report.upstream_model),
                },
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[NativeAPI] Failed to persist usage: {error}"
                ));
            }
        })
    }
}

async fn native_prepare_account(
    account: CodexAccount,
    force: bool,
) -> Result<CodexAccount, native_contract::ServiceError> {
    if native_is_chatgpt_plan_account(&account)
        || account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
        || (!force && !codex_oauth::is_token_expired(&account.tokens.access_token))
    {
        return Ok(account);
    }
    let permit = NATIVE_ACCOUNT_IO
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            native_error(
                503,
                "service_unavailable",
                "Credential worker is unavailable",
            )
        })?;
    let runtime = tokio::runtime::Handle::current();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runtime.block_on(async move {
            if force {
                codex_account::force_refresh_managed_account_after_observed(
                    &account.id,
                    account.token_generation,
                    "native-api",
                )
                .await
            } else {
                codex_account::prepare_account_for_quota_query(&account.id).await
            }
        })
    });
    timeout(Duration::from_secs(60), task)
        .await
        .map_err(|_| {
            native_error(
                504,
                "credential_refresh_timeout",
                "Credential refresh timed out",
            )
        })?
        .map_err(|_| native_error(500, "credential_refresh_failed", "Credential worker failed"))?
        .map_err(|_| {
            native_error(
                401,
                "reauthorization_required",
                "Unable to refresh account credentials; check account authorization",
            )
        })
}

include!("codex_local_access_native_parity.rs");

#[cfg(test)]
#[path = "codex_local_access_native_official_fallback_tests.rs"]
mod native_official_fallback_tests;

#[cfg(test)]
#[path = "codex_local_access_native_live_acceptance.rs"]
mod native_live_acceptance;

#[cfg(test)]
#[path = "codex_local_access_native_live_pool.rs"]
mod native_live_pool;

#[cfg(test)]
mod native_host_tests {
    use super::*;
    include!("codex_local_access_tests_native.rs");
    include!("codex_local_access_tests_quota_protection.rs");
    include!("codex_local_access_native_identity_fixture_tests.rs");
    include!("codex_local_access_native_paused_member_tests.rs");
    include!("codex_local_access_tests_native_cooldown.rs");
    include!("codex_local_access_tests_native_health.rs");
    include!("codex_local_access_tests_native_cooling_continuation.rs");
    include!("codex_local_access_tests_native_quota_continuation.rs");
    include!("codex_local_access_tests_native_quota_http.rs");
    include!("codex_local_access_tests_native_reserve.rs");
    include!("codex_local_access_tests_native_automatic.rs");
    include!("codex_local_access_tests_native_parity22.rs");
    #[test]
    fn bearer_keys_are_exact_and_nonempty() {
        assert!(native_key_matches("secret", "secret"));
        assert!(!native_key_matches("", ""));
        assert!(!native_key_matches("secret", "secret-more"));
        assert!(!native_key_matches("secret", "secreu"));
    }
    #[test]
    fn model_scopes_match_without_regex_or_substring_expansion() {
        assert!(native_pattern_matches("gpt-*", "gpt-6"));
        assert!(native_pattern_matches("gpt-*-codex", "gpt-6-codex"));
        assert!(!native_pattern_matches("gpt-*", "evil/gpt-6"));
        assert!(!native_pattern_matches("gpt-6", "gpt-6-mini"));
    }
}

fn native_bound_quota_reserve_allows(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> bool {
    quota_reserve_policy(collection, &account.id)
        .is_none_or(|reserve| quota_reserve_allows(&reserve, account))
}
