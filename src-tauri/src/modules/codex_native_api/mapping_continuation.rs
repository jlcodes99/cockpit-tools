//! Expand an authenticated completed checkpoint without selecting another route.
use super::*;

pub(in crate::modules::codex_native_api) async fn recover(
    state: &AppState,
    prepared: &mut Prepared,
) -> Result<(), ServiceError> {
    let execution = prepared.execution.renew_open();
    execution
        .run(super::super::execution::prepare(
            state,
            recover_inner(state, prepared),
        ))
        .await?;
    prepared.execution = execution;
    Ok(())
}

async fn recover_inner(state: &AppState, prepared: &mut Prepared) -> Result<(), ServiceError> {
    let headers = prepared.replay_headers().clone();
    let body = prepared.replay_body().clone();
    let mut context = inspect_mode(state, &prepared.caller, &headers, &body, true).await?;
    let identity = state.identity.clone();
    let account = prepared.route.continuity_id().to_owned();
    let limit = state.options.max_body_bytes;
    let (mut context, plan) = super::super::blocking::run_prepared(state, move || {
        if context.state_account_id.as_deref() != Some(account.as_str()) {
            return Err(ServiceError::new(
                409,
                "continuation_owner_mismatch",
                "Continuation checkpoint belongs to another account",
            ));
        }
        // This validates caller, thread, complete output, tool pairing and
        // opaque provenance, and inherits only parameters omitted by the client.
        let plan = identity.expand_http_previous(&mut context, &headers, &body, &body)?;
        if limit != usize::MAX && plan.body.to_string().len() > limit {
            return Err(ServiceError::new(
                413,
                "continuation_history_too_large",
                "Expanded conversation exceeds the request size limit",
            ));
        }
        Ok((context, plan))
    })
    .await?;
    context.retain_binding_generation(&prepared.binding);
    let (body, mut headers, binding) =
        rewrite_with_context(state, prepared, context, plan.body.clone()).await?;
    if limit != usize::MAX && body.to_string().len() > limit {
        return Err(ServiceError::new(
            413,
            "continuation_history_too_large",
            "Expanded conversation exceeds the request size limit",
        ));
    }
    // A rejected continuation must not carry its stale routing token forward.
    headers.remove("x-codex-turn-state");
    prepared.binding.abort_rebuild();
    prepared.body = body;
    prepared.headers = headers;
    prepared.binding = binding;
    prepared.rebuild_plan = Some(plan);
    prepared.rebuild_applied = true;
    prepared.continuation_recovered = true;
    // The one recovery send stays on this account, including capability gates.
    prepared.capability_fallback_enabled = false;
    Ok(())
}
