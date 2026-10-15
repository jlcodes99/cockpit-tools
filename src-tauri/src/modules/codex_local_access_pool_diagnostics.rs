// Diagnostic state is separate from account authentication and scheduler state.
const UNSCOPED_ACCOUNT_POOL_HEALTH_KEY: &str = "__unscoped__";
const ACCOUNT_POOL_DIAGNOSTIC_LIMIT: usize = 128;

fn account_pool_route_key(event: &SidecarAuthResultEvent) -> String {
    let id = event.api_key_id.trim();
    let context = (
        event.provider.trim(),
        event.model.trim(),
        event.request_kind.trim(),
    );
    if context == ("", "", "") {
        return if id.is_empty() {
            UNSCOPED_ACCOUNT_POOL_HEALTH_KEY.to_string()
        } else {
            id.to_string()
        };
    }
    serde_json::to_string(&(id, context.0, context.1, context.2)).unwrap_or_default()
}

fn account_pool_route_api_key(key: &str) -> Option<String> {
    if key == UNSCOPED_ACCOUNT_POOL_HEALTH_KEY {
        return Some(String::new());
    }
    // Modern keys encode the route as a tuple; legacy keys are just API Key IDs.
    serde_json::from_str::<(String, String, String, String)>(key)
        .map(|value| value.0)
        .ok()
        .or_else(|| Some(key.to_string()))
}

fn clear_runtime_account_pool_failure(
    runtime: &mut GatewayRuntime,
    api_key_id: &str,
    last_failure_at: i64,
) -> bool {
    let id = api_key_id.trim();
    let target = runtime
        .account_pool_health
        .iter()
        .find_map(|(key, health)| {
            let matches_id = health.api_key_id == id
                || (health.api_key_id.is_empty()
                    && account_pool_route_api_key(key).as_deref() == Some(id));
            (matches_id && health.last_failure_at == last_failure_at).then(|| key.clone())
        });
    if let Some(key) = target {
        runtime.account_pool_health.remove(&key);
        return true;
    }
    // If this Key still has records, ask the UI to reload rather than claiming
    // that its stale selection cleared an updated or sibling record.
    !runtime.account_pool_health.iter().any(|(key, health)| {
        health.api_key_id == id || account_pool_route_api_key(key).as_deref() == Some(id)
    })
}

pub async fn clear_local_access_pool_failure(
    api_key_id: String,
    last_failure_at: i64,
) -> Result<bool, String> {
    let cleared = {
        let mut runtime = tokio::time::timeout(Duration::from_secs(2), gateway_runtime().lock())
            .await
            .map_err(|_| "pool_diagnostic_clear_timeout".to_string())?;
        clear_runtime_account_pool_failure(&mut runtime, &api_key_id, last_failure_at)
    };
    emit_local_access_state_updated();
    Ok(cleared)
}

fn is_account_pool_unavailable_error(event: &SidecarAuthResultEvent) -> bool {
    matches!(
        event.error_code.as_deref().map(str::trim),
        Some("auth_not_found" | "auth_unavailable")
    ) || event
        .error_message
        .as_deref()
        .is_some_and(|message| message.to_ascii_lowercase().contains("no auth available"))
}

async fn update_sidecar_account_pool_health(
    event: &SidecarAuthResultEvent,
    pool_diagnostic: bool,
) -> bool {
    let mut runtime = gateway_runtime().lock().await;
    apply_sidecar_account_pool_health(&mut runtime, event, pool_diagnostic, now_ms())
}

fn apply_sidecar_account_pool_health(
    runtime: &mut GatewayRuntime,
    event: &SidecarAuthResultEvent,
    pool_diagnostic: bool,
    now: i64,
) -> bool {
    if !event.success
        && !pool_diagnostic
        && (!event.account_id.trim().is_empty() || !is_account_pool_unavailable_error(event))
    {
        return false;
    }
    let key = account_pool_route_key(event);
    let request_id = event.request_id.trim();
    let started = event.started_at_ms;
    if started > 0 {
        if runtime
            .account_pool_request_watermarks
            .get(&key)
            .is_some_and(|(latest, _)| started < *latest)
            || runtime
                .account_pool_health
                .get(&key)
                .is_some_and(|health| started < health.request_started_at_ms)
        {
            return false;
        }
        runtime
            .account_pool_request_watermarks
            .insert(key.clone(), (started, request_id.to_string()));
        while runtime.account_pool_request_watermarks.len() > ACCOUNT_POOL_DIAGNOSTIC_LIMIT {
            let oldest = runtime
                .account_pool_request_watermarks
                .iter()
                .min_by_key(|(key, (time, _))| (*time, *key))
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                runtime.account_pool_request_watermarks.remove(&oldest);
            }
        }
    }
    if event.success {
        let can_clear = runtime.account_pool_health.get(&key).is_some_and(|health| {
            let same_request = !request_id.is_empty() && health.request_id == request_id;
            // Same-millisecond starts cannot be ordered across different requests.
            same_request
                || (started > 0
                    && health.request_started_at_ms > 0
                    && started > health.request_started_at_ms)
                || (request_id.is_empty() && health.request_id.is_empty())
        });
        return can_clear && runtime.account_pool_health.remove(&key).is_some();
    }
    // Only an event from the exact same request may reuse its detailed decisions.
    let same_request = !request_id.is_empty()
        && runtime
            .account_pool_health
            .get(&key)
            .is_some_and(|health| health.request_id == request_id);
    let mut health = if same_request && !pool_diagnostic {
        runtime
            .account_pool_health
            .get(&key)
            .cloned()
            .unwrap_or_default()
    } else {
        RuntimeAccountPoolHealth::default()
    };
    health.request_id = request_id.to_string();
    health.request_started_at_ms = started;
    health.api_key_id = event.api_key_id.trim().to_string();
    health.api_key_label = event.api_key_label.trim().to_string();
    health.provider = event.provider.trim().to_string();
    health.model = event.model.trim().to_string();
    health.request_kind = event.request_kind.trim().to_string();
    health.error_code = event
        .error_code
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    health.error_message = event
        .error_message
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_string();
    if pool_diagnostic {
        health.diagnostic_available = true;
        health.candidate_auths = event.candidate_auths;
        health.scoped_auths = event.scoped_auths;
        health.available_auths = event.available_auths;
        health.unavailable_auths = event.unavailable_auths;
        health.model_excluded_auths = event.model_excluded_auths;
        health.quota_reserved_auths = event.quota_reserved_auths;
        health.image_policy_blocked_auths = event.image_policy_blocked_auths;
        health.scope_diagnostics = event
            .scope_diagnostics
            .iter()
            .filter(|item| !item.reason_code.trim().is_empty())
            .map(|item| CodexLocalAccessAccountPoolScopeDiagnostic {
                account_id: item.account_id.trim().to_string(),
                account_email: item.account_email.trim().to_string(),
                reason_code: item.reason_code.trim().to_string(),
            })
            .collect();
        health.account_statuses = event
            .account_statuses
            .iter()
            .filter(|item| !item.account_id.trim().is_empty())
            .map(|item| RuntimeAccountPoolMemberHealth {
                account_id: item.account_id.trim().to_string(),
                account_email: item.account_email.trim().to_string(),
                available: item.available,
                reason_code: item.reason_code.trim().to_string(),
                reason_message: item.reason_message.trim().to_string(),
            })
            .collect();
    }
    runtime.account_pool_failure_clock =
        now.max(runtime.account_pool_failure_clock.saturating_add(1));
    health.last_failure_at = runtime.account_pool_failure_clock;
    runtime.account_pool_health.insert(key, health);
    while runtime.account_pool_health.len() > ACCOUNT_POOL_DIAGNOSTIC_LIMIT {
        let oldest = runtime
            .account_pool_health
            .iter()
            .min_by_key(|(_, health)| health.last_failure_at)
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            runtime.account_pool_health.remove(&oldest);
        }
    }
    true
}
