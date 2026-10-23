//! Connects host account selection to protocol identity isolation.
use axum::http::HeaderMap;
use serde_json::Value;

use super::{
    contract::{Caller, ImageRoute, ImageRouteRequest, Route, RouteRequest, ServiceError},
    identity::{Binding, RequestIdentity},
    server::{outbound_headers, AppState},
};

#[path = "mapping_capability.rs"]
mod capability;
pub(super) use capability::retry as retry_capability;
#[path = "mapping_official_fallback.rs"]
mod official_fallback;
pub(super) use official_fallback::retry as retry_official_local;

#[derive(Clone)]
pub(super) struct Prepared {
    pub execution: super::execution::Execution,
    pub route: Route,
    pub headers: HeaderMap,
    pub body: Value,
    pub binding: Binding,
    pub caller: Caller,
    pub request: RouteRequest,
    pub source_body: Value,
    pub source_headers: HeaderMap,
    pub image_route_request: Option<ImageRouteRequest>,
    pub compact: bool,
    pub grok_context: Option<super::grok_wire::Context>,
    pub plan_tools_context: Option<super::chatgpt_plan_tools::Context>,
    pub multi_agent_policy: super::contract::MultiAgentPolicy,
    pub multi_agent_context: super::multi_agent_compat::Context,
    pub raw_chat: Option<super::native_chat::RawChat>,
    pub rebuild_plan: Option<super::identity::rebuild::Plan>,
    pub rebuild_applied: bool,
    pub bridge_body: Option<Value>,
    pub capability_fallback_enabled: bool,
    pub capability_fallback: Option<super::diagnostics::CapabilityFallbackTrace>,
    pub transport_pinned: bool,
    pub preflight_attempts: Vec<super::diagnostics::DiagnosticAttempt>,
}

impl Prepared {
    pub(super) fn replay_body(&self) -> &Value {
        self.rebuild_plan
            .as_ref()
            .filter(|_| self.rebuild_applied)
            .map_or(
                self.bridge_body.as_ref().unwrap_or(&self.source_body),
                |plan| &plan.body,
            )
    }
    fn replay_headers(&self) -> &HeaderMap {
        self.rebuild_plan
            .as_ref()
            .filter(|_| self.rebuild_applied)
            .map_or(&self.source_headers, |plan| &plan.headers)
    }
}

pub(super) async fn prepare(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
    preferred: Option<&str>,
) -> Result<Prepared, ServiceError> {
    prepare_excluding(
        state,
        caller,
        incoming,
        body,
        preferred,
        Vec::new(),
        None,
        false,
        false,
    )
    .await
}

pub(super) async fn prepare_websocket(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
) -> Result<Prepared, ServiceError> {
    prepare_websocket_preferred(state, caller, incoming, body, None).await
}

pub(super) async fn prepare_websocket_preferred(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
    preferred: Option<&str>,
) -> Result<Prepared, ServiceError> {
    // A WebSocket can outlive a key rotation. Revalidate its original credential,
    // not only the stored caller ID, before starting every new inference turn.
    if state.authorize(incoming).await?.id != caller.id {
        return Err(ServiceError::new(
            401,
            "invalid_api_key",
            "WebSocket credential is no longer valid",
        ));
    }
    prepare_excluding(
        state,
        caller,
        incoming,
        body,
        preferred,
        Vec::new(),
        None,
        false,
        true,
    )
    .await
}

pub(super) async fn prepare_compact(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
    preferred: Option<&str>,
) -> Result<Prepared, ServiceError> {
    prepare_excluding(
        state,
        caller,
        incoming,
        body,
        preferred,
        Vec::new(),
        None,
        true,
        false,
    )
    .await
}

async fn prepare_excluding(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
    preferred: Option<&str>,
    excluded_account_ids: Vec<String>,
    capability_fallback_from: Option<String>,
    compact: bool,
    websocket: bool,
) -> Result<Prepared, ServiceError> {
    let execution = super::execution::current_or_new(state).await?;
    execution
        .run(super::execution::prepare(
            state,
            prepare_excluding_inner(
                state,
                caller,
                incoming,
                body,
                preferred,
                excluded_account_ids,
                capability_fallback_from,
                compact,
                websocket,
            ),
        ))
        .await
}

async fn prepare_excluding_inner(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    mut body: Value,
    preferred: Option<&str>,
    excluded_account_ids: Vec<String>,
    capability_fallback_from: Option<String>,
    compact: bool,
    websocket: bool,
) -> Result<Prepared, ServiceError> {
    apply_request_image_policy(incoming, &mut body);
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| ServiceError::new(400, "model_required", "A model is required"))?
        .to_string();
    let (prepared_body, boundary_tools_prepared) = super::multi_agent_transport::ingress(state, caller, incoming, body, &model).await?;
    body = prepared_body;
    let requested_rebuild = !compact
        && super::execution::current()
            .is_some_and(|execution| execution.policy.account_switch_rebuild);
    let mut context =
        inspect_mode(state, caller, incoming, &body, requested_rebuild || compact).await?;
    // A preferred owner supplied by the partial-HTTP continuation path is a
    // transport constraint, not a replaceable affinity hint.
    let enabled = requested_rebuild && preferred.is_none();
    let (context_next, rebuild_plan) = if enabled {
        let identity = state.identity.clone();
        let headers_copy = incoming.clone();
        let body_copy = body.clone();
        let max_body_bytes = state.options.max_body_bytes;
        super::blocking::run_prepared(state, move || {
            let mut plan =
                identity.configure_rebuild(&mut context, &headers_copy, &body_copy, enabled)?;
            if plan.is_none() {
                // Account selection must know a complete cached continuation
                // is available even before the selected HTTP transport applies
                // its defaults. The actual send is validated again after routing.
                plan = identity.preview_rebuild_with_checkpoint_parameters(
                    &context,
                    &headers_copy,
                    &body_copy,
                )?;
            }
            let plan = plan.filter(|plan| {
                max_body_bytes == usize::MAX || plan.body.to_string().len() <= max_body_bytes
            });
            if enabled && plan.is_none() {
                identity.protect_unrebuildable(&mut context)?;
            }
            Ok::<_, ServiceError>((context, plan))
        })
        .await?
    } else {
        (context, None)
    };
    let mut context = context_next;
    if preferred.is_some_and(|account| {
        context
            .state_account_id
            .as_deref()
            .is_some_and(|owner| owner != account)
    }) {
        return Err(ServiceError::new(
            409,
            "continuation_owner_mismatch",
            "The continuation belongs to a different account; reconnect with full input",
        ));
    }
    let request = RouteRequest {
        model,
        session_id: context.session_id.clone(),
        thread_id: context.thread_id.clone(),
        parent_thread_id: context.parent_thread_id.clone(),
        preferred_account_id: context
            .state_account_id
            .clone()
            .or_else(|| preferred.map(str::to_string)),
        soft_preferred_account_id: None,
        target_account_id: target_account(incoming),
        force_refresh: false,
        image_generation_requested: image_generation_requested(&body),
        image_input: image_input(&body),
        excluded_account_ids,
        requires_permit: true,
        required_capabilities: super::capabilities::collect(&body, compact),
        capability_fallback_from,
        auxiliary: websocket.then(|| "responses.websocket".to_owned()),
    };
    let selected = match route(state, caller, request.clone()).await {
        Ok(selected) => selected,
        Err(error)
            if matches!(
                error.code,
                "account_cooling" | "account_binding_unavailable" | "no_available_account"
            ) && rebuild_plan.is_some()
                && request.preferred_account_id.is_some() =>
        {
            let mut excluded = request.excluded_account_ids.clone();
            excluded.push(request.preferred_account_id.clone().unwrap());
            return prepare_rebuilt(
                state,
                caller,
                incoming,
                &body,
                rebuild_plan.unwrap(),
                excluded,
                request.capability_fallback_from.clone(),
                websocket,
            )
            .await;
        }
        Err(error) => return Err(error),
    };
    if !compact
        // SIWC sockets are bridged to stateless HTTP. A reconnect has no local
        // socket checkpoint, so expand a validated durable response here too.
        // Partial connection-local anchors were already resolved by WS history.
        && (!websocket || selected.chatgpt_plan)
        && ((selected.chatgpt && !selected.headers.contains_key("x-xai-token-auth"))
            || selected.chatgpt_plan
            || super::grok_errors::applies(&selected))
        && selected.wire_api == super::contract::WireApi::Responses
        && (body
            .get("previous_response_id")
            .is_some_and(Value::is_string)
            || body.get("input").is_some_and(Value::is_string))
    {
        let identity = state.identity.clone();
        let original_body = body.clone();
        let original_headers = incoming.clone();
        let source_body = original_body.clone();
        let source_headers = original_headers.clone();
        let max_body_bytes = state.options.max_body_bytes;
        let expanded_previous = body
            .get("previous_response_id")
            .is_some_and(Value::is_string);
        let (context, normalized_body, plan) = super::blocking::run_prepared(state, move || {
            let mut normalized = original_body.clone();
            identity.normalize_http_input(&mut context, &original_headers, &mut normalized)?;
            let plan = if expanded_previous {
                Some(identity.expand_http_previous(&mut context, &original_headers, &normalized, &original_body)?)
            } else {
                identity.configure_rebuild(&mut context, &original_headers, &normalized, enabled)?
            };
            if max_body_bytes != usize::MAX && plan.as_ref().map_or(&normalized, |plan| &plan.body).to_string().len() > max_body_bytes {
                return Err(ServiceError::new(413, "continuation_history_too_large",
                    "Expanded conversation exceeds the request size limit; existing history was preserved"));
            }
            Ok::<_, ServiceError>((context, normalized, plan))
        }).await?;
        let headers = plan.as_ref().map_or(incoming, |plan| &plan.headers);
        let body = plan
            .as_ref()
            .map_or(normalized_body, |plan| plan.body.clone());
        let mut prepared = finalize(
            state,
            caller,
            headers,
            body,
            context,
            selected,
            request,
            None,
            false,
            preferred.is_some(),
            boundary_tools_prepared,
        )
        .await?;
        prepared.source_body = source_body;
        prepared.source_headers = source_headers;
        if prepared.rebuild_plan.is_none() {
            prepared.rebuild_plan = plan;
        }
        prepared.rebuild_applied |= expanded_previous;
        return Ok(prepared);
    }
    let mut prepared = finalize(
        state,
        caller,
        incoming,
        body,
        context,
        selected,
        request,
        None,
        compact,
        preferred.is_some(),
        boundary_tools_prepared,
    )
    .await?;
    if prepared.route.wire_api == super::contract::WireApi::Responses
        && prepared.rebuild_plan.is_none()
    {
        prepared.rebuild_plan = rebuild_plan;
    }
    Ok(prepared)
}

pub(super) fn apply_request_image_policy(headers: &HeaderMap, body: &mut Value) {
    let mode = headers
        .get("x-agtools-disable-image-generation")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let all = matches!(mode.as_str(), "true" | "all" | "disabled");
    if !all
        && !matches!(mode.as_str(), "chat" | "images_only" | "images-only")
        && !headers.contains_key("x-openai-internal-codex-responses-lite")
    {
        return;
    }
    fn disabled(tool: &Value, all: bool) -> bool {
        tool["type"] == "image_generation"
            || (all
                && (tool.pointer("/function/name").and_then(Value::as_str) == Some("image_gen")
                    || tool["name"] == "image_gen"))
    }
    fn strip(body: &mut Value, all: bool) {
        if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
            tools.retain(|tool| !disabled(tool, all));
        }
        if body
            .get("tool_choice")
            .is_some_and(|tool| disabled(tool, all))
        {
            body.as_object_mut().map(|body| body.remove("tool_choice"));
        }
        if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
            for item in input
                .iter_mut()
                .filter(|item| item["type"] == "additional_tools")
            {
                strip(item, all);
            }
            input.retain(|item| {
                item["type"] != "additional_tools"
                    || item["tools"]
                        .as_array()
                        .is_none_or(|tools| !tools.is_empty())
            });
        }
    }
    strip(body, all);
}

pub(super) async fn prepare_selected(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    body: Value,
    selected: ImageRoute,
    image_request: ImageRouteRequest,
) -> Result<Prepared, ServiceError> {
    let execution = super::execution::current_or_new(state).await?;
    execution
        .run(super::execution::prepare(
            state,
            prepare_selected_inner(state, caller, incoming, body, selected, image_request),
        ))
        .await
}

async fn prepare_selected_inner(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    mut body: Value,
    selected: ImageRoute,
    image_request: ImageRouteRequest,
) -> Result<Prepared, ServiceError> {
    let context = inspect_mode(state, caller, incoming, &body, true).await?;
    if context
        .state_account_id
        .as_deref()
        .is_some_and(|id| id != selected.route.continuity_id())
    {
        return Err(ServiceError::new(
            409,
            "continuation_owner_mismatch",
            "The image continuation belongs to a different account",
        ));
    }
    let model = body["model"]
        .as_str()
        .ok_or_else(|| ServiceError::new(400, "model_required", "A model is required"))?
        .to_owned();
    let request = RouteRequest {
        model,
        session_id: context.session_id.clone(),
        thread_id: context.thread_id.clone(),
        parent_thread_id: context.parent_thread_id.clone(),
        preferred_account_id: context.state_account_id.clone(),
        soft_preferred_account_id: None,
        target_account_id: target_account(incoming),
        force_refresh: false,
        image_generation_requested: true,
        image_input: image_input(&body),
        excluded_account_ids: Vec::new(),
        requires_permit: true,
        required_capabilities: super::capabilities::collect(&body, false),
        capability_fallback_from: None,
        auxiliary: None,
    };
    // Image routing has already resolved and authorized the concrete tool model.
    // Apply it for OAuth Responses too, including every reselected retry route.
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            if tool.get("type").and_then(Value::as_str) == Some("image_generation") {
                tool["model"] = Value::String(selected.image_model.clone());
            }
        }
    }
    finalize(
        state,
        caller,
        incoming,
        body,
        context,
        selected.route,
        request,
        Some(image_request),
        false,
        true,
        false,
    )
    .await
}

async fn finalize(
    state: &AppState,
    caller: &Caller,
    incoming: &HeaderMap,
    mut body: Value,
    mut context: RequestIdentity,
    mut selected: Route,
    mut request: RouteRequest,
    image_route_request: Option<ImageRouteRequest>,
    compact: bool,
    transport_pinned: bool,
    boundary_tools_prepared: bool,
) -> Result<Prepared, ServiceError> {
    if request.excluded_account_ids.contains(&selected.account_id) {
        return Err(ServiceError::new(
            503,
            "no_available_account",
            "No untried account is available",
        ));
    }
    let logical_source = body.clone();
    let logical_headers = incoming.clone();
    let mut official_plan = None;
    if !selected.chatgpt_plan && selected.official_fallback_registration_id.is_some()
        && !official_fallback::same_owner(state, &context, &selected).await? {
        let (rebuilt, plan) = official_fallback::replay(state, caller, incoming, &body, context.clone(), None)
            .await?.ok_or_else(|| official_fallback::history_recovery("history_unavailable"))?;
        context = rebuilt;
        body = plan.body.clone();
        request.preferred_account_id = None;
        request.soft_preferred_account_id = None;
        official_plan = Some(plan);
    }
    let mut force_capability_fallback = false;
    let preflight_attempts = Vec::new();
    if selected.chatgpt_plan && image_route_request.is_none()
        && request.auxiliary.as_deref().is_none_or(|operation| operation == "responses.websocket") {
        let mut official_body = body.clone();
        super::chatgpt_plan_wire::adapt_service_tier(&mut official_body);
        if let Err(error) = super::chatgpt_plan_wire::validate_before_transport(&official_body, compact, request.auxiliary.as_deref() == Some("responses.websocket")) {
            if error.code == "subscription_sharing_unsupported_capability" {
                let failure = super::contract::OfficialFallbackFailure {
                    trigger_stage: "preflight",
                    status: error.status, code: error.code.into(), message: Some(error.message.clone()), retry_after: error.retry_after,
                };
                match official_fallback::select(state, caller, incoming, &body,
                    context.clone(), &mut selected, request.clone(), None, failure.clone()).await {
                    Ok(Some(fallback)) => {
                        body = fallback.plan.body.clone();
                        context = fallback.context;
                        selected = fallback.route;
                        request = fallback.request;
                        official_plan = Some(fallback.plan);
                    }
                    Err(error) if super::official_fallback::local_unavailable(&error) => {
                        // Keep the original logical request and owner fence. The
                        // scoped selector may try another account. Neither this
                        // local candidate nor the official validation sent a
                        // request, so they are not failed upstream attempts.
                        force_capability_fallback = true;
                    }
                    Err(error) => return Err(error),
                    Ok(None) => {}
                }
            }
        }
    }
    let incoming = official_plan.as_ref().map_or(incoming, |plan| &plan.headers);
    let official_candidate = selected.chatgpt_plan || selected.official_fallback_registration_id.is_some();
    let selection = capability::select(
        state,
        caller,
        incoming,
        body,
        context,
        selected,
        request,
        transport_pinned || image_route_request.is_some(),
        force_capability_fallback,
        "preflight",
    )
    .await.map_err(|error| {
        if official_candidate && matches!(error.code,
            "capability_fallback_history_unsafe" | "capability_fallback_encryption_unsafe") {
            official_fallback::history_recovery("history_not_portable")
        } else {
            error
        }
    })?;
    let mut context = selection.context;
    let selected = selection.route;
    let request = selection.request;
    let mut body = selection.body;
    let incoming = &selection.headers;
    let mut multi_agent_policy = super::execution::within(
        state,
        state.host.multi_agent_policy(caller, Some(&selected), &request.model),
    ).await?;
    multi_agent_policy.options.tools_prepared = boundary_tools_prepared;
    if selected.chatgpt_plan {
        super::chatgpt_plan_wire::adapt_service_tier(&mut body);
        super::chatgpt_plan_wire::validate_before_transport(
            &body,
            compact,
            request.auxiliary.as_deref() == Some("responses.websocket"),
        )?;
    }
    let source_body = logical_source;
    if compact || image_route_request.is_some() {
        // These Responses operations still carry the caller's complete history.
        // They do not create a generated-history checkpoint, but validated
        // portable blobs must survive identity rewriting unchanged.
        context.disable_checkpoints();
    } else if selected.wire_api != super::contract::WireApi::Responses {
        context.disable_rebuild();
    }
    body["model"] = Value::String(selected.model.clone());
    apply_grok_thinking(&selected, &source_body, &mut body, false)?;
    body = super::execution::within(
        state,
        state.host.prepare_body(
            caller,
            &selected,
            incoming,
            body,
            image_route_request.is_some(),
            compact,
        ),
    )
    .await?;
    let source_headers = logical_headers;
    let mut headers = outbound_headers(incoming);
    headers.extend(selected.headers.clone());
    let identity = state.identity.clone();
    let account_id = selected.continuity_id().to_owned();
    let owner_id = selected.owner_id.clone();
    let (mut body, headers, binding) = super::blocking::run_prepared(state, move || {
        let binding =
            identity.rewrite(&context, &account_id, &owner_id, &mut headers, &mut body)?;
        Ok::<_, ServiceError>((body, headers, binding))
    })
    .await?;
    let mut headers = headers;
    super::account_installation::apply(&selected, &mut headers, &mut body)?;
    if selected.headers.contains_key("x-xai-token-auth") {
        headers.remove("x-grok-conv-id");
        if let Some(session) = headers.get("session-id").cloned() {
            headers.insert("x-grok-conv-id", session);
        }
    }
    body["model"] = Value::String(selected.model.clone());
    if selected.chatgpt_plan {
        super::chatgpt_plan_wire::adapt_service_tier(&mut body);
    } else if body.get("service_tier").is_none() {
        if let Some(tier) = &selected.default_service_tier {
            body["service_tier"] = Value::String(tier.clone());
        }
    }
    Ok(Prepared {
        execution: super::execution::current_or_new(state).await?,
        route: selected,
        headers,
        body,
        binding,
        caller: caller.clone(),
        request,
        source_body,
        source_headers,
        image_route_request,
        compact,
        grok_context: None,
        plan_tools_context: None,
        multi_agent_policy,
        multi_agent_context: Default::default(),
        raw_chat: None,
        rebuild_applied: selection.plan.is_some() || official_plan.is_some(),
        rebuild_plan: selection.plan.or(official_plan),
        bridge_body: None,
        capability_fallback_enabled: selection.enabled,
        capability_fallback: selection.trace,
        transport_pinned,
        preflight_attempts,
    })
}

async fn route(
    state: &AppState,
    caller: &Caller,
    request: RouteRequest,
) -> Result<Route, ServiceError> {
    super::execution::route(state, caller, request).await
}

async fn prepare_rebuilt(
    state: &AppState,
    caller: &Caller,
    original_headers: &HeaderMap,
    original_body: &Value,
    mut plan: super::identity::rebuild::Plan,
    excluded: Vec<String>,
    capability_fallback_from: Option<String>,
    websocket: bool,
) -> Result<Prepared, ServiceError> {
    let model = plan.body["model"].as_str().unwrap_or_default().to_owned();
    let (body, boundary_tools_prepared) = super::multi_agent_transport::ingress(
        state, caller, &plan.headers, plan.body, &model,
    ).await?;
    plan.body = body;
    let mut context = inspect_mode(state, caller, &plan.headers, &plan.body, true).await?;
    let identity = state.identity.clone();
    let headers = plan.headers.clone();
    let body = plan.body.clone();
    context = super::blocking::run_prepared(state, move || {
        identity.configure_rebuild(&mut context, &headers, &body, true)?;
        Ok::<_, ServiceError>(context)
    })
    .await?;
    context.start_rebuild_generation(&plan);
    let request = RouteRequest {
        model: plan.body["model"].as_str().unwrap_or_default().to_owned(),
        session_id: context.session_id.clone(),
        thread_id: context.thread_id.clone(),
        parent_thread_id: context.parent_thread_id.clone(),
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: target_account(original_headers),
        force_refresh: false,
        image_generation_requested: image_generation_requested(&plan.body),
        image_input: image_input(&plan.body),
        excluded_account_ids: excluded,
        requires_permit: true,
        required_capabilities: super::capabilities::collect(&plan.body, false),
        capability_fallback_from,
        auxiliary: websocket.then(|| "responses.websocket".to_owned()),
    };
    let selected = route(state, caller, request.clone()).await?;
    if selected.wire_api != super::contract::WireApi::Responses {
        return Err(ServiceError::new(
            503,
            "rebuild_protocol_unavailable",
            "Context rebuilding requires a Responses account",
        ));
    }
    let mut next = finalize(
        state,
        caller,
        &plan.headers,
        plan.body.clone(),
        context,
        selected,
        request,
        None,
        false,
        false,
        boundary_tools_prepared,
    )
    .await?;
    next.source_headers = original_headers.clone();
    next.source_body = original_body.clone();
    next.rebuild_plan = Some(plan);
    next.rebuild_applied = true;
    Ok(next)
}

fn target_account(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-cockpit-target-account-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) async fn observe_headers(
    state: &AppState,
    prepared: &Prepared,
    headers: &HeaderMap,
) -> Result<(), ServiceError> {
    let identity = state.identity.clone();
    let binding = prepared.binding.clone();
    let headers = headers.clone();
    super::blocking::run_tracked(state, move || identity.observe_headers(&binding, &headers)).await
}

pub(super) async fn observe_event(
    state: &AppState,
    prepared: &Prepared,
    event: &Value,
) -> Result<(), ServiceError> {
    let kind = event.get("type").and_then(Value::as_str);
    let reusable = matches!(
        kind,
        Some(
            "response.created"
                | "response.metadata"
                | "response.output_item.done"
                | "response.completed"
                | "response.done"
                | "response.incomplete"
                | "response.failed"
                | "response.error"
                | "response"
                | "error"
        )
    ) || super::events::response_terminal_kind(event).is_some()
        || event.get("output").is_some()
        || (kind.is_none() && event.get("id").is_some());
    if !reusable {
        return Ok(());
    }
    let identity = state.identity.clone();
    let binding = prepared.binding.clone();
    let event = event.clone();
    super::blocking::run_tracked(state, move || identity.observe_event(&binding, &event)).await
}

async fn inspect_mode(
    state: &AppState,
    caller: &Caller,
    headers: &HeaderMap,
    body: &Value,
    allow_portable_input: bool,
) -> Result<RequestIdentity, ServiceError> {
    let identity = state.identity.clone();
    let caller_id = caller.id.clone();
    let headers = headers.clone();
    let body = body.clone();
    super::blocking::run_prepared(state, move || {
        if allow_portable_input {
            identity.inspect_for_rebuild(&caller_id, &headers, &body)
        } else {
            identity.inspect(&caller_id, &headers, &body)
        }
    })
    .await
}

/// A 401 is the only automatic replay: same account and owner, before any output.
pub(super) async fn refresh(state: &AppState, prepared: &mut Prepared) -> Result<(), ServiceError> {
    refresh_route(state, prepared, true).await
}

/// Rebuild a retired transport only inside an existing retry budget. This
/// retains the account/owner fence and performs no forced credential refresh.
pub(super) async fn refresh_transport(state: &AppState, prepared: &mut Prepared) -> Result<(), ServiceError> {
    refresh_route(state, prepared, false).await
}

async fn refresh_route(state: &AppState, prepared: &mut Prepared, force_refresh: bool) -> Result<(), ServiceError> {
    prepared.execution = prepared.execution.renew_open();
    let execution = prepared.execution.clone();
    execution
        .run(super::execution::prepare(
            state,
            refresh_inner(state, prepared, force_refresh),
        ))
        .await
}

async fn refresh_inner(state: &AppState, prepared: &mut Prepared, force_refresh: bool) -> Result<(), ServiceError> {
    prepared.route.permit = None;
    let mut request = prepared.request.clone();
    request.force_refresh = force_refresh;
    request.preferred_account_id = Some(prepared.route.continuity_id().to_owned());
    let selected = if let Some(mut image_request) = prepared.image_route_request.clone() {
        image_request.force_refresh = force_refresh;
        image_request.preferred_account_id = Some(prepared.route.continuity_id().to_owned());
        super::execution::within(
            state,
            state.host.image_route(&prepared.caller, image_request),
        )
        .await?
        .route
    } else {
        super::execution::within(
            state,
            state.host.refresh_route(&prepared.caller, &prepared.route, request),
        ).await?
    };
    if selected.account_id != prepared.route.account_id
        || selected.continuity_id() != prepared.route.continuity_id()
        || selected.official_registration_id != prepared.route.official_registration_id
        || selected.official_fallback_registration_id != prepared.route.official_fallback_registration_id
        || selected.official_fallback_authorization_revision != prepared.route.official_fallback_authorization_revision
        || (prepared.route.official_fallback_registration_id.is_some()
            && selected.authorization_scope_fingerprint != prepared.route.authorization_scope_fingerprint)
        || selected.owner_id != prepared.route.owner_id
    {
        return Err(ServiceError::new(
            409,
            "refresh_owner_changed",
            "Credential refresh cannot change the account or owner of an active request",
        ));
    }
    if !force_refresh && (selected.base_url != prepared.route.base_url
        || selected.model != prepared.route.model
        || selected.wire_api != prepared.route.wire_api
        || selected.chatgpt != prepared.route.chatgpt
        || selected.chatgpt_plan != prepared.route.chatgpt_plan)
    {
        return Err(ServiceError::new(409, "transport_route_changed",
            "Transport recovery cannot change the upstream endpoint, model or protocol"));
    }
    let mut multi_agent_policy = super::execution::within(
        state,
        state.host.multi_agent_policy(
            &prepared.caller, Some(&selected), &prepared.request.model,
        ),
    ).await?;
    multi_agent_policy.options.tools_prepared = prepared.multi_agent_policy.options.tools_prepared;
    prepared.multi_agent_policy = multi_agent_policy;
    for name in prepared.route.headers.keys() {
        prepared.headers.remove(name);
    }
    prepared.headers.extend(selected.headers.clone());
    prepared.route = selected;
    Ok(())
}

pub(super) async fn rewrite_body(
    state: &AppState,
    prepared: &Prepared,
    body: Value,
) -> Result<Value, ServiceError> {
    let execution = prepared.execution.clone();
    execution
        .run(super::execution::prepare(
            state,
            rewrite_body_inner(state, prepared, body),
        ))
        .await
}

async fn rewrite_body_inner(
    state: &AppState,
    prepared: &Prepared,
    mut body: Value,
) -> Result<Value, ServiceError> {
    let mut context = inspect_mode(
        state,
        &prepared.caller,
        prepared.replay_headers(),
        &body,
        !prepared.compact
            && prepared.raw_chat.is_none()
            && prepared.route.wire_api == super::contract::WireApi::Responses
            && prepared.execution.policy.account_switch_rebuild,
    )
    .await?;
    context.retain_binding_generation(&prepared.binding);
    body["model"] = Value::String(prepared.route.model.clone());
    apply_grok_thinking(
        &prepared.route,
        &prepared.source_body,
        &mut body,
        prepared.raw_chat.is_some(),
    )?;
    body = super::execution::within(
        state,
        state.host.prepare_body(
            &prepared.caller,
            &prepared.route,
            prepared.replay_headers(),
            body,
            prepared.image_route_request.is_some(),
            prepared.compact,
        ),
    )
    .await?;
    let store = state.identity.clone();
    let account = prepared.route.continuity_id().to_owned();
    let owner = prepared.route.owner_id.clone();
    let mut headers = prepared.headers.clone();
    let (mut body, mut headers) = super::blocking::run_prepared(state, move || {
        store.rewrite(&context, &account, &owner, &mut headers, &mut body)?;
        Ok((body, headers))
    })
    .await?;
    super::account_installation::apply(&prepared.route, &mut headers, &mut body)?;
    body["model"] = Value::String(prepared.route.model.clone());
    if prepared.route.chatgpt_plan {
        super::chatgpt_plan_wire::adapt_service_tier(&mut body);
    } else if body.get("service_tier").is_none() {
        if let Some(tier) = &prepared.route.default_service_tier {
            body["service_tier"] = Value::String(tier.clone());
        }
    }
    Ok(body)
}

fn apply_grok_thinking(
    route: &Route,
    source: &Value,
    body: &mut Value,
    chat: bool,
) -> Result<(), ServiceError> {
    if super::grok_errors::applies(route) {
        if let Some(model) = source.get("model").and_then(Value::as_str) {
            super::grok_thinking::apply(body, model, chat)?;
        }
    } else if let Some(model) = source.get("model").and_then(Value::as_str) {
        let selected = route.model.rsplit('/').next().unwrap_or(&route.model);
        let known_gpt = selected.starts_with("gpt-") || selected.starts_with("codex");
        // API-key Chat/Responses providers use OpenAI thinking semantics for
        // their user-defined names too. A Grok-looking ordinary route is not
        // proof of xAI capability; only the OAuth marker or official Responses
        // endpoint verified above may rewrite it.
        let grok_name = |name: &str| {
            super::gpt_thinking::model_name(name)
                .split('/')
                .any(|part| {
                    part.eq_ignore_ascii_case("grok")
                        || part.to_ascii_lowercase().starts_with("grok-")
                })
        };
        let compatible = !route.chatgpt && !grok_name(&route.model) && !grok_name(model);
        if !known_gpt && !compatible {
            return Ok(());
        }
        let levels = known_gpt
            .then(|| crate::modules::codex_protocol::known_codex_model_template(&route.model))
            .flatten()
            .map(|template| {
                template["supported_reasoning_levels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().or_else(|| value["effort"].as_str()))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        super::gpt_thinking::apply(body, model, chat, levels.as_deref())?;
    }
    Ok(())
}

/// Retry only a self-contained input before any downstream output; opaque
/// continuation state is account-owned and is never replayed on another owner.
pub(super) async fn retry_other_account(
    state: &AppState,
    prepared: &mut Prepared,
) -> Result<(), ServiceError> {
    prepared.execution = prepared.execution.renew_open();
    let execution = prepared.execution.clone();
    execution
        .run(super::execution::prepare(
            state,
            retry_other_account_inner(state, prepared),
        ))
        .await
}

async fn retry_other_account_inner(
    state: &AppState,
    prepared: &mut Prepared,
) -> Result<(), ServiceError> {
    if let Some(plan) = prepared
        .rebuild_plan
        .clone()
        .filter(|_| prepared.raw_chat.is_none() && prepared.image_route_request.is_none())
    {
        let mut excluded = prepared.request.excluded_account_ids.clone();
        if !excluded.contains(&prepared.route.account_id) {
            excluded.push(prepared.route.account_id.clone());
        }
        prepared.route.permit = None;
        let mut next = prepare_rebuilt(
            state,
            &prepared.caller,
            &prepared.source_headers,
            &prepared.source_body,
            plan,
            excluded,
            prepared.request.capability_fallback_from.clone(),
            prepared.request.auxiliary.as_deref() == Some("responses.websocket"),
        )
        .await?;
        inherit_capability_trace(prepared, &mut next);
        *prepared = next;
        return Ok(());
    }
    if prepared.request.preferred_account_id.is_some() {
        return Err(ServiceError::new(
            409,
            "continuation_owner_unavailable",
            "The continuation owner is unavailable; resend full input in a new conversation",
        ));
    }
    prepared.route.permit = None;
    let mut excluded = prepared.request.excluded_account_ids.clone();
    if !excluded.contains(&prepared.route.account_id) {
        excluded.push(prepared.route.account_id.clone());
    }
    let mut next = if let Some(mut image_request) = prepared.image_route_request.clone() {
        image_request.preferred_account_id = None;
        image_request.excluded_account_ids = excluded.clone();
        let selected = super::execution::within(
            state,
            state
                .host
                .image_route(&prepared.caller, image_request.clone()),
        )
        .await?;
        if excluded.contains(&selected.route.account_id)
            || selected.route.chatgpt != prepared.route.chatgpt
        {
            return Err(ServiceError::new(
                503,
                "image_source_unavailable",
                "No alternate image source is available",
            ));
        }
        let mut next = prepare_selected(
            state,
            &prepared.caller,
            &prepared.source_headers,
            prepared.source_body.clone(),
            selected,
            image_request,
        )
        .await?;
        next.request.excluded_account_ids = excluded;
        next
    } else {
        prepare_excluding(
            state,
            &prepared.caller,
            &prepared.source_headers,
            prepared.source_body.clone(),
            None,
            excluded,
            prepared.request.capability_fallback_from.clone(),
            prepared.compact,
            prepared.request.auxiliary.as_deref() == Some("responses.websocket"),
        )
        .await?
    };
    inherit_capability_trace(prepared, &mut next);
    next.raw_chat = prepared.raw_chat.clone();
    *prepared = next;
    Ok(())
}

pub(super) async fn retry_cooldown_round(
    state: &AppState,
    prepared: &mut Prepared,
) -> Result<(), ServiceError> {
    let execution = prepared.execution.clone();
    execution
        .run(super::execution::prepare(
            state,
            retry_cooldown_round_inner(state, prepared),
        ))
        .await
}

async fn retry_cooldown_round_inner(
    state: &AppState,
    prepared: &mut Prepared,
) -> Result<(), ServiceError> {
    if prepared.request.capability_fallback_from.is_some() {
        return Err(ServiceError::new(
            503,
            "capability_fallback_exhausted",
            "Capability fallback cannot restart an unrestricted credential round",
        ));
    }
    let pinned = prepared.request.preferred_account_id.clone();
    if prepared.rebuild_plan.is_some() && pinned.is_none() {
        prepared.route.permit = None;
        *prepared = prepare_rebuilt(
            state,
            &prepared.caller,
            &prepared.source_headers,
            &prepared.source_body,
            prepared.rebuild_plan.clone().unwrap(),
            Vec::new(),
            prepared.request.capability_fallback_from.clone(),
            prepared.request.auxiliary.as_deref() == Some("responses.websocket"),
        )
        .await?;
        return Ok(());
    }
    prepared.route.permit = None;
    let mut next = if let Some(mut image_request) = prepared.image_route_request.clone() {
        image_request.excluded_account_ids.clear();
        image_request.preferred_account_id = pinned.clone();
        let selected = super::execution::within(
            state,
            state
                .host
                .image_route(&prepared.caller, image_request.clone()),
        )
        .await?;
        if selected.route.chatgpt != prepared.route.chatgpt {
            return Err(ServiceError::new(
                503,
                "image_source_unavailable",
                "Retry cannot change the image wire protocol",
            ));
        }
        prepare_selected(
            state,
            &prepared.caller,
            &prepared.source_headers,
            prepared.source_body.clone(),
            selected,
            image_request,
        )
        .await?
    } else {
        prepare_excluding(
            state,
            &prepared.caller,
            &prepared.source_headers,
            prepared.source_body.clone(),
            pinned.as_deref(),
            Vec::new(),
            prepared.request.capability_fallback_from.clone(),
            prepared.compact,
            prepared.request.auxiliary.as_deref() == Some("responses.websocket"),
        )
        .await?
    };
    if pinned.is_some()
        && (next.route.account_id != prepared.route.account_id
            || next.route.continuity_id() != prepared.route.continuity_id()
            || next.route.owner_id != prepared.route.owner_id)
    {
        return Err(ServiceError::new(
            409,
            "continuation_owner_mismatch",
            "Retry cannot change the continuation owner",
        ));
    }
    inherit_capability_trace(prepared, &mut next);
    next.raw_chat = prepared.raw_chat.clone();
    *prepared = next;
    Ok(())
}

pub(super) fn image_input(body: &Value) -> bool {
    fn content(value: &Value) -> bool {
        match value {
            Value::Array(items) => items.iter().any(content),
            Value::Object(item) => {
                let image = item
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| {
                        matches!(
                            kind.trim().to_ascii_lowercase().as_str(),
                            "input_image" | "image_url" | "image"
                        )
                    });
                image
                    || ["content", "parts", "input", "messages", "response"]
                        .iter()
                        .any(|key| item.get(*key).is_some_and(content))
            }
            _ => false,
        }
    }
    // Inspect content blocks only; tool declarations may use these names without images.
    ["input", "messages", "response"]
        .iter()
        .any(|key| body.get(*key).is_some_and(content))
}

pub(super) fn image_generation_requested(body: &Value) -> bool {
    let contains_image_tool = |tools: Option<&Value>| {
        tools.and_then(Value::as_array).is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("type").and_then(Value::as_str) == Some("image_generation"))
        })
    };
    contains_image_tool(body.get("tools"))
        || body
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    item.get("type").and_then(Value::as_str) == Some("additional_tools")
                        && contains_image_tool(item.get("tools"))
                })
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn api_key_non_gpt_suffix_applies_without_assuming_unverified_grok_capability() {
        let mut route = Route {
            continuity_account_id: None,
            installation_profile: None,
            official_authorization_revision: None,
            official_registration_id: None,
            official_fallback_registration_id: None,
            official_fallback_authorization_revision: None,
            authorization_scope_fingerprint: None,
                official_fallback_failure: None,
            transport_identity: None,
            account_id: "account".into(),
            owner_id: "owner".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "custom".into(),
            headers: HeaderMap::new(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            proxy_url: None,
            proxy_route: None,
            chatgpt: false,
            chatgpt_plan: false,
            wire_api: super::super::contract::WireApi::Responses,
            compact_endpoint: None,
            websocket_endpoint: None,
            default_service_tier: None,
            permit: None,
        };
        let original = json!({"model":"custom","reasoning":{"effort":"low","summary":"detailed"},"input":[{"type":"reasoning","encrypted_content":"cipher"},{"type":"function_call","call_id":"raw-call","name":"tool","arguments":"raw 250ms"}],"client_metadata":{"session_id":"raw-session","thread_id":"raw-thread"}});
        for (selected, requested, effort) in [
            ("o3", "o3(high)", "high"),
            ("custom", "custom(8192)", "medium"),
        ] {
            route.model = selected.into();
            let source = json!({"model":requested});
            let mut body = original.clone();
            apply_grok_thinking(&route, &source, &mut body, false).unwrap();
            assert_eq!(body["reasoning"]["effort"], effort);
            assert_eq!(body["input"], original["input"]);
            assert_eq!(body["client_metadata"], original["client_metadata"]);
            assert_eq!(body["reasoning"]["summary"], "detailed");
            let mut chat = original.clone();
            apply_grok_thinking(&route, &source, &mut chat, true).unwrap();
            assert_eq!(chat["reasoning_effort"], effort);
        }
        route.model = "custom".into();
        for requested in ["custom", "custom(unsupported)", "grok/public(high)"] {
            let mut body = original.clone();
            apply_grok_thinking(&route, &json!({"model":requested}), &mut body, false).unwrap();
            assert_eq!(body, original);
        }
        route.chatgpt = true;
        let mut body = original.clone();
        apply_grok_thinking(&route, &json!({"model":"custom(high)"}), &mut body, false).unwrap();
        assert_eq!(body, original, "unknown OAuth capability is not inferred");
        route.model = "gpt-5.6-luna".into();
        assert!(apply_grok_thinking(
            &route,
            &json!({"model":"gpt-5.6-luna(minimal)"}),
            &mut body,
            false
        )
        .is_err());
    }
    #[test]
    fn xai_thinking_requires_verified_route_and_keeps_history_in_both_protocols() {
        let mut route = Route {
            continuity_account_id: None,
            installation_profile: None,
            official_authorization_revision: None,
            official_registration_id: None,
            official_fallback_registration_id: None,
            official_fallback_authorization_revision: None,
            authorization_scope_fingerprint: None,
                official_fallback_failure: None,
            transport_identity: None,
            account_id: "account".into(),
            owner_id: "owner".into(),
            base_url: "https://example.invalid/v1".into(),
            model: "grok-4.5".into(),
            headers: HeaderMap::new(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            proxy_url: None,
            proxy_route: None,
            chatgpt: false,
            chatgpt_plan: false,
            wire_api: super::super::contract::WireApi::Responses,
            compact_endpoint: None,
            websocket_endpoint: None,
            default_service_tier: None,
            permit: None,
        };
        let source = json!({"model":"grok/public(none)"});
        let original = json!({"model":"grok-4.5","reasoning":{"effort":"high","summary":"detailed"},"input":[{"type":"reasoning","encrypted_content":"original-cipher"}]});
        let mut body = original.clone();
        apply_grok_thinking(&route, &source, &mut body, false).unwrap();
        assert_eq!(
            body, original,
            "ordinary providers must retain their original payload"
        );
        route.base_url = "https://api.x.ai/v1".into();
        apply_grok_thinking(&route, &source, &mut body, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(!route.headers.contains_key("x-xai-token-auth"));
        assert_eq!(body["input"], original["input"]);
        route.base_url = "https://example.invalid/v1".into();
        body = original.clone();
        route
            .headers
            .insert("x-xai-token-auth", "fixture".parse().unwrap());
        apply_grok_thinking(&route, &source, &mut body, false).unwrap();
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(
            body["reasoning"]["summary"],
            original["reasoning"]["summary"]
        );
        assert_eq!(body["input"], original["input"]);
        let mut chat = json!({"model":"grok-4.5","messages":[{"role":"assistant","content":"history"}],"reasoning_effort":"high"});
        let history = chat["messages"].clone();
        apply_grok_thinking(&route, &source, &mut chat, true).unwrap();
        assert_eq!(chat["reasoning_effort"], "low");
        assert_eq!(chat["messages"], history);
    }
    #[test]
    fn vision_input_uses_real_content_and_preserves_history_and_raw_chat_fallback() {
        for body in [
            json!({"input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,fixture"}]}]}),
            json!({"input":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"fixture"}}]}]}),
            json!({"messages":[{"role":"user","content":[{"type":"image","source":{"type":"base64"}}]}]}),
            json!({"response":{"input":[{"role":"user","content":[{"type":"input_image","file_id":"file-fixture"}]}]}}),
        ] {
            assert!(image_input(&body));
        }
        assert!(!image_input(
            &json!({"tools":[{"type":"image_generation"},{"type":"function","parameters":{"type":"input_image"}}],"input":[{"type":"function_call","arguments":"{\"type\":\"input_image\"}"},{"role":"user","content":"input_image"},{"type":"reasoning","encrypted_content":"image_url"}]})
        ));
    }
    #[test]
    fn image_policy_sees_responses_lite_tools_but_never_message_text() {
        assert!(image_generation_requested(
            &json!({"tools":[{"type":"image_generation"}]})
        ));
        assert!(image_generation_requested(
            &json!({"input":[{"type":"additional_tools","tools":[{"type":"image_generation"}]}]})
        ));
        assert!(!image_generation_requested(
            &json!({"input":[{"role":"user","content":"Use image_generation"}]})
        ));
        assert!(!image_generation_requested(
            &json!({"input":[{"type":"message","tools":[{"type":"image_generation"}]}]})
        ));
    }
}

fn inherit_capability_trace(previous: &Prepared, next: &mut Prepared) {
    if next.capability_fallback.is_none() && next.request.capability_fallback_from.is_some() {
        next.capability_fallback = previous.capability_fallback.clone().map(|mut trace| {
            trace.to_account_id = next.route.account_id.clone();
            trace.to_model = next.route.model.clone();
            trace
        });
    }
}
