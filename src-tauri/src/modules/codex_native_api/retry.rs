//! Bounded retries before response bytes are exposed to a caller.
use super::{
    contract::{ExecutionPolicy, ServiceError, WireApi},
    events::retry_after_headers,
    lifecycle::{stopping_error, RequestTracker},
    mapping::{self, Prepared},
    server::AppState,
};
use axum::http::{header, HeaderValue, StatusCode};
use futures_util::StreamExt;
use serde_json::Value;
use std::{sync::Arc, time::Duration};

pub(super) async fn execution_policy(state: &AppState) -> Result<ExecutionPolicy, ServiceError> {
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = tokio::time::timeout(state.options.request_timeout, state.host.execution_policy()) =>
            result.map_err(|_| ServiceError::new(504, "execution_policy_timeout", "Execution policy lookup timed out"))?,
    }
}

pub(super) fn total_timeout() -> ServiceError {
    ServiceError::new(
        504,
        "upstream_total_timeout",
        "Upstream request exceeded the total time limit",
    )
}

pub(super) async fn send(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    compact: bool,
    policy: &ExecutionPolicy,
) -> Result<reqwest::Response, ServiceError> {
    if let Some(raw) = prepared.raw_chat.clone() {
        if prepared.route.chatgpt || prepared.route.wire_api != WireApi::Chat {
            return Err(ServiceError::new(
                503,
                "retry_protocol_changed",
                "The selected account does not support the original native Chat request",
            ));
        }
        return send_inner(
            state,
            prepared,
            tracker,
            compact,
            policy,
            Some((&raw.body, raw.endpoint)),
        )
        .await;
    }
    send_inner(state, prepared, tracker, compact, policy, None).await
}

pub(super) async fn send_native_chat(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    body: &Value,
    endpoint: &str,
) -> Result<reqwest::Response, ServiceError> {
    send_inner(
        state,
        prepared,
        tracker,
        false,
        policy,
        Some((body, endpoint)),
    )
    .await
}

async fn send_inner(
    state: &Arc<AppState>,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    compact: bool,
    policy: &ExecutionPolicy,
    raw: Option<(&Value, &str)>,
) -> Result<reqwest::Response, ServiceError> {
    let mut refreshed = false;
    let mut credential_attempts = prepared.request.excluded_account_ids.len();
    let mut send_attempts = 0u8;
    let mut status_attempts = 0u8;
    let mut open_attempts = 1u8;
    let mut round_attempted = false;
    let mut bootstrap_attempts = 0u8;
    let deadline = prepared
        .execution
        .total_deadline
        .min(tracker.deadline(policy.total_timeout));
    let mut previous_open_deadline = None;
    loop {
        let compact = compact
            || (super::grok_errors::applies(&prepared.route)
                && super::grok_wire::compaction::triggered(&prepared.body));
        if state.stop.stopped() {
            return Err(stopping_error());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(total_timeout());
        }
        if previous_open_deadline == Some(prepared.execution.open_deadline) {
            prepared.execution = prepared.execution.renew_open();
        }
        previous_open_deadline = Some(prepared.execution.open_deadline);
        tracker.begin_attempt("http");
        tracker.capability_fallback(prepared.capability_fallback.clone());
        if raw.is_some() && (prepared.route.wire_api != WireApi::Chat || prepared.route.chatgpt) {
            return Err(ServiceError::new(
                503,
                "retry_protocol_changed",
                "The replacement account does not support this request's native Chat protocol",
            ));
        }
        if (prepared.route.chatgpt || prepared.route.chatgpt_plan) && !compact {
            prepared.body["stream"] = Value::Bool(true);
            prepared.body["store"] = Value::Bool(false);
        }
        prepared.headers.remove(header::CONTENT_ENCODING);
        prepared.headers.remove(header::PROXY_AUTHORIZATION);
        prepared.headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        let chat = prepared.route.wire_api == WireApi::Chat && !compact;
        let path = if let Some((_, endpoint)) = raw {
            endpoint
        } else if chat {
            "chat/completions"
        } else if compact {
            "responses/compact"
        } else {
            "responses"
        };
        let mut body = if let Some((body, _)) = raw {
            let mut body = mapping::rewrite_body(state, prepared, body.clone()).await?;
            body["model"] = Value::String(prepared.route.model.clone());
            body
        } else {
            let mut body = prepared.body.clone();
            super::multi_agent_transport::sanitize_reasoning(prepared, &mut body);
            if chat || super::grok_errors::applies(&prepared.route) {
                super::multi_agent_transport::prepare(prepared, &mut body)?;
            }
            if chat { super::chat_upstream::responses_to_chat(&body)? } else { body }
        };
        if chat
            && path == "chat/completions"
            && body.get("stream").and_then(Value::as_bool) == Some(true)
        {
            // Force only the per-attempt upstream copy, preserving all native
            // options and the client's original request used for later retries.
            if !body.get("stream_options").is_some_and(Value::is_object) {
                body["stream_options"] = serde_json::json!({});
            }
            body["stream_options"]["include_usage"] = Value::Bool(true);
        }
        prepared.grok_context = if super::grok_errors::applies(&prepared.route) {
            let (wire_body, context) = if compact {
                super::grok_wire::prepare_compact(body)?
            } else {
                super::grok_wire::prepare(body)?
            };
            body = wire_body;
            prepared.binding.retain_transmitted_input_blobs(&body);
            Some(context)
        } else {
            None
        };
        let endpoint = compact
            .then_some(prepared.route.compact_endpoint.as_ref())
            .flatten().cloned();
        let mut headers = prepared.headers.clone();
        // OAuth forces an upstream stream for buffered clients too. Keep the
        // client's delivery option only when it actually requested streaming.
        if prepared.route.chatgpt && prepared.source_body.get("stream") != Some(&Value::Bool(true))
        {
            if let Some(object) = body.as_object_mut() {
                object.remove("stream_options");
            }
        }
        if let Some(endpoint) = endpoint.as_ref() {
            for name in prepared.route.headers.keys() {
                headers.remove(name);
            }
            headers.extend(endpoint.headers.clone());
        }
        headers.remove(header::PROXY_AUTHORIZATION);
        if prepared.route.chatgpt_plan {
            prepared.plan_tools_context =
                Some(super::chatgpt_plan_wire::prepare_with_effort_override(
                    &mut headers,
                    &mut body,
                    compact,
                    prepared
                        .source_body
                        .get("model")
                        .and_then(Value::as_str)
                        .is_some_and(|model| super::gpt_thinking::parse_model(model).1.is_some()),
                )?);
        } else {
            prepared.plan_tools_context = None;
            super::codex_wire::normalize(
                &mut headers,
                &mut body,
                prepared.route.chatgpt && !super::grok_errors::applies(&prepared.route),
                compact,
            )?;
        }
        if raw.is_none() && !chat && !super::grok_errors::applies(&prepared.route) {
            super::multi_agent_transport::prepare(prepared, &mut body)?;
        }
        let codex_backend = prepared.route.chatgpt && !super::grok_errors::applies(&prepared.route);
        if codex_backend || prepared.route.chatgpt_plan {
            super::codex_cpa_wire::sanitize_input_ids(&mut body);
        }
        super::codex_wire::finalize_routing_hint(
            &mut headers, &body, codex_backend && !prepared.route.chatgpt_plan, &prepared.route.headers,
        )?;
        if chat && path == "chat/completions" {
            crate::modules::minimax_request::normalize_flash_request(&mut body, false);
        }
        prepared.multi_agent_context.capture_final(&body);
        prepared.binding.retain_transmitted_input_blobs(&body);
        let base_url = endpoint.as_ref().map_or(prepared.route.base_url.as_str(), |endpoint| {
            endpoint.base_url.as_str()
        });
        let url = format!("{}/{}", base_url.trim_end_matches('/'), path);
        let streaming = !compact && body.get("stream").and_then(Value::as_bool).unwrap_or(false);
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static(if streaming {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
        tracker.payload("upstream_request", "http", &headers, &body.to_string());
        let request = prepared.route.client.post(url).headers(headers).json(&body);
        // This is the constructed request for this attempt, independent of
        // payload logging. Do not replace these IDs with body metadata.
        if let Some(request) = request.try_clone().and_then(|request| request.build().ok()) {
            tracker.upstream_headers(request.headers());
        }
        tracker.check_quota_before_send()?;
        let mut stop = state.stop.clone();
        let result = tokio::select! {
            _ = stop.wait() => Err(stopping_error()),
            result = tokio::time::timeout_at(prepared.execution.prepare_deadline().min(deadline), request.send()) => match result {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(super::diagnostics::transport_failure(502, "upstream_unavailable", "Unable to connect to the selected upstream", &error)),
                Err(_) if tokio::time::Instant::now() >= deadline => Err(total_timeout()),
                Err(_) => Err(ServiceError::new(504, "upstream_connect_timeout", "Upstream response headers timed out")),
            }
        };
        let mut response = match result {
            Ok(value) => value,
            Err(error) => {
                tracker.attempt_error(&error);
                if state.stop.stopped() || tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
                let stream_open_failed = streaming && error.code == "upstream_connect_timeout";
                if stream_open_failed && open_attempts < policy.stream_open_max_attempts.max(1) {
                    open_attempts += 1;
                    continue;
                }
                // Exhausting this route's open budget must not consume network
                // retries, but a self-contained request may still change routes.
                if !stream_open_failed && send_attempts < policy.send_retries {
                    send_attempts += 1;
                    delay(
                        state,
                        retry_delay(policy, send_attempts),
                        policy.send_retry_max_delay,
                        deadline,
                    )
                    .await?;
                    renew_transport_after_tls(state, prepared, tracker, &error).await?;
                    continue;
                }
                if credential_limit_allows(policy, credential_attempts)
                    && can_switch_account(prepared)
                {
                    // A transport error does not identify a bad credential.
                    // The replacement path excludes it for this request only.
                    tracker.release_route();
                    if alternate(state, prepared, deadline).await.is_ok() {
                        credential_attempts += 1;
                        send_attempts = 0;
                        status_attempts = 0;
                        open_attempts = 1;
                        refreshed = false;
                        tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                        tracker.routing_identity(&prepared.request);
                        tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                        tracker.request_metadata(&prepared.body);
                        continue;
                    }
                }
                if !round_attempted
                    && cooldown_round(state, prepared, tracker, policy, deadline, error.status)
                        .await?
                {
                    round_attempted = true;
                    credential_attempts = 0;
                    send_attempts = 0;
                    status_attempts = 0;
                    open_attempts = 1;
                    refreshed = false;
                    continue;
                }
                return Err(error);
            }
        };
        tracker.upstream_http_version(response.version());
        tracker.quota_headers(response.headers());
        let status = response.status();
        if status.is_success()
            && !compact
            && (prepared.route.wire_api == WireApi::Responses || (chat && streaming))
        {
            let probe = async {
                if chat {
                    super::bootstrap::probe_chat(state, response, policy, deadline).await
                } else {
                    super::bootstrap::probe_provider(
                        state,
                        response,
                        policy,
                        deadline,
                        super::grok_errors::applies(&prepared.route),
                        prepared.capability_fallback_enabled,
                    )
                    .await
                }
            };
            response = match probe.await {
                Ok(response) => response,
                Err(error)
                    if matches!(
                        error.code,
                        "upstream_stream_error"
                            | "incomplete_upstream_stream"
                            | "upstream_timeout"
                            | "upstream_idle_timeout"
                    ) && bootstrap_attempts < policy.bootstrap_retries =>
                {
                    tracker.attempt_error(&error);
                    bootstrap_attempts += 1;
                    delay(
                        state,
                        policy.retry_base_delay,
                        policy.retry_max_delay,
                        deadline,
                    )
                    .await?;
                    renew_transport_after_tls(state, prepared, tracker, &error).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
        }
        let mut status = response.status();
        let mut credential_scope = false;
        let mut body_retry_after = None;
        let mut failure_message = None;
        let mut model_unavailable = false;
        let mut request_scoped = false;
        let mut capability_rejection = false;
        if !status.is_success() {
            let inspected = inspect_status_error(
                state,
                response,
                policy,
                deadline,
                &prepared.route.model,
                super::grok_errors::applies(&prepared.route),
            )
            .await?;
            response = inspected.0;
            status = response.status();
            credential_scope = inspected.1;
            body_retry_after = inspected.3;
            tracker.capture_response_error(inspected.2.as_deref(), credential_scope);
            failure_message = inspected.2;
            model_unavailable = !compact && inspected.4;
            request_scoped = inspected.5;
            capability_rejection = inspected.6;
            tracker.capture_request_fault(request_scoped);
            tracker.attempt_result(status.as_u16(), false, Some("upstream_http_error"));
        }
        let wait = match (retry_after_headers(response.headers()), body_retry_after) {
            (Some(headers), Some(body)) => Some(headers.max(body)),
            (headers, body) => headers.or(body),
        };
        tracker.set_retry_after(wait);
        if capability_rejection
            && prepared.capability_fallback_enabled
            && !prepared.request.required_capabilities.is_empty()
        {
            if mapping::retry_capability(state, prepared).await? {
                drop(response);
                tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                tracker.routing_identity(&prepared.request);
                tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                tracker.request_metadata(&prepared.body);
                continue;
            }
        }
        // Preserve the original response before refresh, retry or cooldown can
        // penalize an account for a malformed caller request.
        if request_scoped {
            return Ok(response);
        }
        if status == StatusCode::UNAUTHORIZED
            && !refreshed
            && !model_unavailable
            && can_refresh_credentials(prepared)
        {
            tracker.release_route();
            let mut stop = state.stop.clone();
            let refresh = tokio::select! {
                _ = stop.wait() => return Err(stopping_error()),
                result = tokio::time::timeout_at(deadline, mapping::refresh(state, prepared)) => result.map_err(|_| total_timeout())?,
            };
            refreshed = true;
            match refresh {
                Ok(()) => {
                    drop(response);
                    tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                    tracker.routing_identity(&prepared.request);
                    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                    tracker.request_metadata(&prepared.body);
                    continue;
                }
                Err(error)
                    if state.stop.stopped()
                        || tokio::time::Instant::now() >= deadline
                        || error.code == "refresh_owner_changed" =>
                {
                    return Err(error)
                }
                // Keep the original upstream failure and continue through the
                // normal owner/replay fences before considering another account.
                Err(_) => {}
            }
        }
        let retryable = model_unavailable
            || matches!(
                status.as_u16(),
                401 | 402 | 403 | 408 | 429 | 500 | 502 | 503 | 504
            );
        // Status failures have an independent budget from transport send errors.
        // Authentication refresh is separate; invalid credentials, throttling and
        // forbidden responses are never blindly retried on the same account.
        if matches!(status.as_u16(), 408 | 500 | 502 | 503 | 504)
            && !model_unavailable
            && status_attempts < policy.status_retries
        {
            drop(response);
            status_attempts += 1;
            let backoff = policy
                .status_retry_base_delay
                .saturating_mul(1u32 << u32::from(status_attempts.saturating_sub(1).min(8)));
            delay(
                state,
                wait.unwrap_or(backoff),
                policy.status_retry_max_delay,
                deadline,
            )
            .await?;
            continue;
        }
        let mut failure_recorded = false;
        if retryable
            && credential_limit_allows(policy, credential_attempts)
            && can_switch_account(prepared)
        {
            note_failed(
                state,
                prepared,
                if model_unavailable {
                    404
                } else {
                    status.as_u16()
                },
                wait,
                credential_scope,
                failure_message.as_deref(),
                deadline,
            )
            .await?;
            failure_recorded = true;
            tracker.release_route();
            match alternate(state, prepared, deadline).await {
                Ok(()) => {
                    drop(response);
                    credential_attempts += 1;
                    send_attempts = 0;
                    status_attempts = 0;
                    open_attempts = 1;
                    refreshed = false;
                    tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
                    tracker.routing_identity(&prepared.request);
                    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
                    tracker.request_metadata(&prepared.body);
                    continue;
                }
                Err(error) if state.stop.stopped() || tokio::time::Instant::now() >= deadline => {
                    return Err(error)
                }
                // Preserve the real upstream status if the alternate pool is exhausted.
                Err(_) => {}
            }
        }
        if retryable
            && !round_attempted
            && !compact_availability_neutral(
                prepared,
                status.as_u16(),
                credential_scope,
                failure_message.as_deref(),
            )
        {
            if !failure_recorded {
                note_failed(
                    state,
                    prepared,
                    if model_unavailable {
                        404
                    } else {
                        status.as_u16()
                    },
                    wait,
                    credential_scope,
                    failure_message.as_deref(),
                    deadline,
                )
                .await?;
            }
            if cooldown_round(state, prepared, tracker, policy, deadline, status.as_u16()).await? {
                drop(response);
                round_attempted = true;
                credential_attempts = 0;
                send_attempts = 0;
                status_attempts = 0;
                open_attempts = 1;
                refreshed = false;
                continue;
            }
        }
        return Ok(response);
    }
}

/// Called only after an existing retry fence/budget permits another attempt.
/// Do not turn TLS recovery into an authorization refresh or a replay policy.
pub(super) async fn renew_transport_after_tls(
    state: &AppState,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    error: &ServiceError,
) -> Result<(), ServiceError> {
    if error.transport_failure != Some(super::contract::TransportFailureKind::TlsDecrypt) {
        return Ok(());
    }
    tracker.release_route();
    mapping::refresh_transport(state, prepared).await?;
    tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
    tracker.routing_identity(&prepared.request);
    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
    tracker.request_metadata(&prepared.body);
    Ok(())
}

pub(super) async fn cooldown_round(
    state: &AppState,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: tokio::time::Instant,
    status: u16,
) -> Result<bool, ServiceError> {
    prepared.execution = prepared.execution.renew_open();
    let execution = prepared.execution.clone();
    execution
        .run(super::execution::within(
            state,
            cooldown_round_inner(state, prepared, tracker, policy, deadline, status),
        ))
        .await
}

async fn cooldown_round_inner(
    state: &AppState,
    prepared: &mut Prepared,
    tracker: &mut RequestTracker,
    policy: &ExecutionPolicy,
    deadline: tokio::time::Instant,
    status: u16,
) -> Result<bool, ServiceError> {
    let mut request = prepared.request.clone();
    request.excluded_account_ids.clear();
    if let Some(image_request) = &prepared.image_route_request {
        request.model = image_request
            .model
            .clone()
            .unwrap_or_else(|| prepared.route.model.clone());
        request.auxiliary = Some(
            if image_request.edit {
                "images.edits"
            } else {
                "images.generations"
            }
            .into(),
        );
    }
    let mut stop = state.stop.clone();
    let wait = tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, state.host.retry_cooldown_wait(&prepared.caller, request, policy.retry_max_delay)) => result.map_err(|_| total_timeout())??,
    };
    let Some(mut wait) = wait else {
        return Ok(false);
    };
    if status == 429 && !wait.is_zero() {
        wait = wait.max(std::time::Duration::from_secs(10));
    }
    if wait > policy.retry_max_delay || tokio::time::Instant::now() + wait >= deadline {
        return Ok(false);
    }
    tracker.release_route();
    prepared.route.permit = None;
    delay(state, wait, policy.retry_max_delay, deadline).await?;
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => return Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, mapping::retry_cooldown_round(state, prepared)) => result.map_err(|_| total_timeout())??,
    }
    tracker.routed(&prepared.caller, &prepared.route, &prepared.request.model);
    tracker.routing_identity(&prepared.request);
    tracker.track_rebuild(prepared.binding.rebuild_abort_handle());
    tracker.request_metadata(&prepared.body);
    Ok(true)
}

// Host routes distinguish managed OAuth credentials from API keys through
// chatgpt; native xAI OAuth projections additionally carry their verified marker.
pub(super) fn can_refresh_credentials(prepared: &Prepared) -> bool {
    prepared.route.chatgpt
        || prepared.route.chatgpt_plan
        || prepared.route.headers.contains_key("x-xai-token-auth")
}

pub(super) fn can_switch_account(prepared: &Prepared) -> bool {
    // A capability replacement may only move again through the stricter
    // pre-execution capability gate, never ordinary network/status failover.
    if prepared.request.capability_fallback_from.is_some() {
        return false;
    }
    if prepared.rebuild_plan.is_some()
        && prepared.raw_chat.is_none()
        && prepared.image_route_request.is_none()
        && prepared.route.wire_api == WireApi::Responses
    {
        return true;
    }
    prepared.request.preferred_account_id.is_none()
        && prepared
            .source_body
            .get("previous_response_id")
            .is_none_or(Value::is_null)
        && !prepared.source_headers.contains_key("x-codex-turn-state")
        && !has_opaque_content(&prepared.source_body)
}

/// Go's max-retry-credentials counts the initial credential; zero disables the
/// count cap, not failover. Each round still excludes all previously tried IDs.
pub(super) fn credential_limit_allows(policy: &ExecutionPolicy, replacements: usize) -> bool {
    policy.credential_retries == 0
        || replacements.saturating_add(1) < usize::from(policy.credential_retries)
}

fn has_opaque_content(value: &Value) -> bool {
    match value {
        Value::Object(value) => value.iter().any(|(key, value)| {
            (key == "encrypted_content" && !value.is_null()) || has_opaque_content(value)
        }),
        Value::Array(value) => value.iter().any(has_opaque_content),
        _ => false,
    }
}

fn retry_delay(policy: &ExecutionPolicy, attempt: u8) -> Duration {
    policy
        .retry_base_delay
        .saturating_mul(1u32 << u32::from(attempt.saturating_sub(1).min(8)))
}

async fn note_failed(
    state: &AppState,
    prepared: &Prepared,
    status: u16,
    wait: Option<Duration>,
    credential_scope: bool,
    message: Option<&str>,
    deadline: tokio::time::Instant,
) -> Result<(), ServiceError> {
    if compact_availability_neutral(prepared, status, credential_scope, message) {
        return Ok(());
    }
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, state.host.route_failed_attempt(&prepared.route, &prepared.route.model, status, wait, credential_scope, message)) => result.map_err(|_| total_timeout()),
    }
}

fn compact_availability_neutral(
    prepared: &Prepared,
    status: u16,
    credential_scope: bool,
    message: Option<&str>,
) -> bool {
    (prepared.compact
        || (super::grok_errors::applies(&prepared.route)
            && super::grok_wire::compaction::triggered(&prepared.body)))
        && !matches!(status, 401 | 402 | 403 | 429)
        && !credential_scope
        && !message.is_some_and(|message| {
            super::diagnostics::is_cloudflare_challenge(message)
                || message.to_ascii_lowercase().contains("invalid_grant")
        })
}

pub(super) async fn inspect_status_error(
    state: &AppState,
    response: reqwest::Response,
    policy: &ExecutionPolicy,
    deadline: tokio::time::Instant,
    model: &str,
    grok: bool,
) -> Result<
    (
        reqwest::Response,
        bool,
        Option<String>,
        Option<Duration>,
        bool,
        bool,
        bool,
    ),
    ServiceError,
> {
    let status = response.status();
    let version = response.version();
    let headers = response.headers().clone();
    let mut source = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        let mut stop = state.stop.clone();
        let chunk = tokio::select! {
            _=stop.wait()=>return Err(stopping_error()),
            result=tokio::time::timeout_at((tokio::time::Instant::now()+policy.idle_timeout).min(deadline),source.next())=>result.map_err(|_| ServiceError::new(status.as_u16(),"upstream_http_error","Upstream error response timed out"))?,
        };
        match chunk {
            Some(Ok(chunk)) => {
                if bytes.len().saturating_add(chunk.len()) > state.options.max_response_bytes {
                    return Err(ServiceError::new(
                        status.as_u16(),
                        "upstream_http_error",
                        "Upstream error response exceeds its size limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            Some(Err(error)) => {
                return Err(super::diagnostics::transport_failure(
                    status.as_u16(),
                    "upstream_http_error",
                    "Unable to read the upstream error response",
                    &error,
                ))
            }
            None => break,
        }
    }
    let value = serde_json::from_slice::<Value>(&bytes).ok();
    let (normalized_status, grok_wait) = if grok {
        super::grok_errors::classify(
            status.as_u16(),
            &value
                .clone()
                .unwrap_or_else(|| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        )
    } else {
        (status.as_u16(), None)
    };
    let status = StatusCode::from_u16(normalized_status).unwrap_or(status);
    let (mut message, scope) = value
        .as_ref()
        .map(|value| super::diagnostics::upstream_error_metadata_status(status.as_u16(), value))
        .unwrap_or_default();
    let challenge = headers
        .get("cf-mitigated")
        .is_some_and(|v| v == "challenge")
        || super::diagnostics::is_cloudflare_challenge(&String::from_utf8_lossy(&bytes));
    if challenge {
        message = Some("cloudflare challenge".into());
    }
    let body_retry_after = value
        .as_ref()
        .and_then(super::events::retry_after_event)
        .max(grok_wait);
    let model_unavailable = match value.as_ref() {
        Some(value) => super::upstream_errors::model_unavailable(status.as_u16(), value, model),
        None => super::upstream_errors::model_unavailable(
            status.as_u16(),
            &Value::String(String::from_utf8_lossy(&bytes).into_owned()),
            model,
        ),
    };
    let capability_rejection = !challenge
        && value.as_ref().is_some_and(|value| {
            super::capabilities::pre_execution_rejection(status.as_u16(), value)
        });
    let request_scoped = !challenge
        && super::upstream_errors::request_fault(
            status.as_u16(),
            &value.unwrap_or_else(|| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
        );
    let mut rebuilt = axum::http::Response::new(bytes);
    *rebuilt.status_mut() = status;
    *rebuilt.version_mut() = version;
    *rebuilt.headers_mut() = headers;
    Ok((
        reqwest::Response::from(rebuilt),
        scope,
        message,
        body_retry_after,
        model_unavailable,
        request_scoped,
        capability_rejection,
    ))
}

async fn alternate(
    state: &AppState,
    prepared: &mut Prepared,
    deadline: tokio::time::Instant,
) -> Result<(), ServiceError> {
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        result = tokio::time::timeout_at(deadline, mapping::retry_other_account(state, prepared)) => result.map_err(|_| total_timeout())?,
    }
}

pub(super) async fn delay(
    state: &AppState,
    value: Duration,
    maximum: Duration,
    deadline: tokio::time::Instant,
) -> Result<(), ServiceError> {
    let mut stop = state.stop.clone();
    tokio::select! {
        _ = stop.wait() => Err(stopping_error()),
        _ = tokio::time::sleep_until((tokio::time::Instant::now() + value.min(maximum)).min(deadline)) => {
            if tokio::time::Instant::now() >= deadline { Err(total_timeout()) } else { Ok(()) }
        },
    }
}

#[cfg(test)]
#[path = "cooldown_retry_tests.rs"]
mod cooldown_tests;
