//! Same-source route fallback with the same verified replay fence as account replacement.
use super::super::contract::{OfficialFallbackFailure, WireApi};
use super::*;

pub(super) struct Selection {
    pub context: RequestIdentity,
    pub route: Route,
    pub request: RouteRequest,
    pub plan: super::super::identity::rebuild::Plan,
}

pub(super) fn history_recovery(reason: &str) -> ServiceError {
    let message = "Continuation requires complete conversation input, including matching tool calls and results and history compatible with the selected account; resend the full conversation input; the original history was preserved";
    ServiceError::new(400, "continuation_history_incomplete", message)
        .with_error_details(serde_json::json!({"code":"continuation_history_incomplete",
            "message":message,"param":"input","reason":reason}))
}

pub(super) async fn same_owner(
    state: &AppState,
    context: &RequestIdentity,
    route: &Route,
) -> Result<bool, ServiceError> {
    let identity = state.identity.clone();
    let context = context.clone();
    let account = route.continuity_id().to_owned();
    let owner = route.owner_id.clone();
    super::super::blocking::run_prepared(state, move || {
        identity.same_upstream_owner(&context, &account, &owner)
    })
    .await
}

pub(super) async fn replay(
    state: &AppState,
    caller: &Caller,
    headers: &HeaderMap,
    body: &Value,
    mut context: RequestIdentity,
    plan: Option<super::super::identity::rebuild::Plan>,
) -> Result<Option<(RequestIdentity, super::super::identity::rebuild::Plan)>, ServiceError> {
    let identity = state.identity.clone();
    let caller_id = caller.id.clone();
    let incoming = headers.clone();
    let mut logical = body.clone();
    let replay = super::super::blocking::run_prepared(state, move || {
        identity.normalize_capability_input(&mut context, &incoming, &mut logical)?;
        let plan = match plan {
            Some(plan) => Some(plan),
            None => match identity.configure_rebuild(&mut context, &incoming, &logical, true)? {
                Some(plan) => Some(plan),
                None => identity
                    .preview_rebuild_with_checkpoint_parameters(&context, &incoming, &logical)?,
            },
        };
        let Some(plan) = plan else { return Ok(None) };
        super::super::capabilities::validate_replay(&plan.headers, &plan.body)?;
        // Do not detach an old continuation simply by clearing an owner field.
        let mut rebuilt = identity.inspect_for_rebuild(&caller_id, &plan.headers, &plan.body)?;
        identity
            .configure_rebuild(&mut rebuilt, &plan.headers, &plan.body, true)?
            .ok_or_else(|| {
                ServiceError::new(
                    409,
                    "continuation_history_incomplete",
                    "The original conversation cannot be safely rebuilt",
                )
            })?;
        rebuilt.start_rebuild_generation(&plan);
        Ok::<_, ServiceError>(Some((rebuilt, plan)))
    })
    .await;
    let (context, plan) = match replay {
        Ok(Some(replay)) => replay,
        // No local request has been made. The caller decides whether another
        // candidate can reuse the owner or full input is required.
        Ok(None) => return Ok(None),
        Err(error)
            if matches!(
                error.code,
                "capability_fallback_history_unsafe" | "continuation_history_incomplete"
            ) =>
        {
            return Err(history_recovery("history_not_portable"));
        }
        Err(error) => return Err(error),
    };
    if plan.body.to_string().len() > state.options.max_body_bytes {
        return Err(ServiceError::new(413, "continuation_history_too_large",
            "Expanded conversation exceeds the request size limit; the original history was preserved"));
    }
    if super::super::capabilities::encrypted(&plan.body) {
        return Err(history_recovery("cipher_incompatible"));
    }
    Ok(Some((context, plan)))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn select(
    state: &AppState,
    caller: &Caller,
    headers: &HeaderMap,
    body: &Value,
    context: RequestIdentity,
    selected: &mut Route,
    mut request: RouteRequest,
    plan: Option<super::super::identity::rebuild::Plan>,
    failure: OfficialFallbackFailure,
) -> Result<Option<Selection>, ServiceError> {
    if !selected.chatgpt_plan
        || selected.wire_api != WireApi::Responses
        || request
            .auxiliary
            .as_deref()
            .is_some_and(|operation| operation != "responses.websocket")
        || super::super::official_fallback::reason(&failure).is_none()
    {
        return Ok(None);
    }
    request.preferred_account_id = None;
    request.soft_preferred_account_id = None;
    request.force_refresh = false;
    // Drop the official permit before acquiring local capacity, including the
    // Prepared clone retained by the caller (released at the retry entrypoint).
    selected.permit = None;
    let Some(mut target) = super::super::execution::within(
        state,
        state
            .host
            .fallback_to_local(caller, selected, request.clone(), failure.clone()),
    )
    .await?
    else {
        return Ok(None);
    };
    if target.chatgpt_plan
        || target.account_id != selected.account_id
        || target.wire_api != WireApi::Responses
        || target.official_registration_id.is_some()
        || target.official_authorization_revision.is_some()
    {
        return Err(ServiceError::new(
            409,
            "official_fallback_owner_changed",
            "The fallback route does not belong to the original local source",
        ));
    }
    // An absent local route must preserve the original official error (including
    // its field). Only a confirmed candidate requires a portable history check.
    let Some((context, plan)) = replay(state, caller, headers, body, context, plan).await? else {
        return Err(history_recovery("history_unavailable"));
    };
    request.required_capabilities = super::super::capabilities::collect(
        &plan.body,
        request.required_capabilities.iter().any(|v| v == "compact"),
    );
    target.official_fallback_registration_id = selected.official_registration_id.clone();
    target.official_fallback_authorization_revision =
        selected.official_authorization_revision.clone();
    target.official_fallback_failure = Some(failure);
    Ok(Some(Selection {
        context,
        route: target,
        request,
        plan,
    }))
}

pub(in crate::modules::codex_native_api) async fn retry(
    state: &AppState,
    prepared: &mut Prepared,
    failure: OfficialFallbackFailure,
) -> Result<bool, ServiceError> {
    if prepared.raw_chat.is_some()
        || prepared.image_route_request.is_some()
        || !prepared.route.chatgpt_plan
    {
        return Ok(false);
    }
    prepared.execution = prepared.execution.renew_open();
    let execution = prepared.execution.clone();
    execution
        .run(super::super::execution::prepare(state, async {
            let body = prepared.replay_body().clone();
            let headers = prepared.replay_headers().clone();
            let context = inspect_mode(state, &prepared.caller, &headers, &body, true).await?;
            let Some(selection) = select(
                state,
                &prepared.caller,
                &headers,
                &body,
                context,
                &mut prepared.route,
                prepared.request.clone(),
                prepared.rebuild_plan.clone(),
                failure,
            )
            .await?
            else {
                return Ok(false);
            };
            let mut next = finalize(
                state,
                &prepared.caller,
                &selection.plan.headers,
                selection.plan.body.clone(),
                selection.context,
                selection.route,
                selection.request,
                None,
                prepared.compact,
                prepared.transport_pinned,
                prepared.multi_agent_policy.options.tools_prepared,
            )
            .await?;
            next.source_body = prepared.source_body.clone();
            next.source_headers = prepared.source_headers.clone();
            next.rebuild_plan = Some(selection.plan);
            next.rebuild_applied = true;
            next.capability_fallback = prepared.capability_fallback.clone();
            *prepared = next;
            Ok(true)
        }))
        .await
}

#[cfg(test)]
#[path = "official_fallback_long_history_tests.rs"]
mod long_history_tests;
