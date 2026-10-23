//! Shared request-boundary and selected-route multi-agent stages.
use axum::http::HeaderMap;
use serde_json::Value;

use super::{
    contract::{Caller, ServiceError},
    mapping::Prepared,
    server::AppState,
};

pub(super) async fn ingress(
    state: &AppState,
    caller: &Caller,
    headers: &HeaderMap,
    mut body: Value,
    model: &str,
) -> Result<(Value, bool), ServiceError> {
    let policy =
        super::execution::within(state, state.host.multi_agent_policy(caller, None, model)).await?;
    if !policy.options.optimize && !policy.options.orphan_delegation {
        return Ok((body, false));
    }
    let headers = headers.clone();
    super::blocking::run_prepared(state, move || {
        let tools_prepared = super::multi_agent_compat::prepare_ingress(
            &headers,
            &mut body,
            policy.options,
            &policy.models,
        );
        Ok((body, tools_prepared))
    })
    .await
}

/// The history source and mapped input are retained; only this attempt's wire
/// copy is adapted. Every reselected route receives a fresh host policy.
pub(super) fn sanitize_reasoning(prepared: &Prepared, body: &mut Value) {
    if (prepared.route.chatgpt || prepared.route.chatgpt_plan)
        && !super::grok_errors::applies(&prepared.route) {
        super::codex_cpa_wire::sanitize_reasoning(body, prepared.multi_agent_policy.options.is_compat);
    }
}

pub(super) fn prepare(prepared: &mut Prepared, body: &mut Value) -> Result<(), ServiceError> {
    let policy = &prepared.multi_agent_policy;
    if !prepared.route.chatgpt && !prepared.route.chatgpt_plan
        && !super::grok_errors::applies(&prepared.route)
    {
        super::multi_agent_compat::prepare_portable_gateway_input(
            &prepared.source_headers, body, policy.options,
        )?;
    }
    if prepared.route.wire_api == super::contract::WireApi::Chat
        || super::grok_errors::applies(&prepared.route)
    {
        // CPA applies this independent Codex-UA helper to every non-Codex
        // target before translation, even with both optimization flags off.
        super::multi_agent_compat::normalize_integer_tools(&prepared.source_headers, body);
        super::multi_agent_compat::prepare_translated_input(
            &prepared.source_headers,
            body,
            policy.options,
        );
        prepared.multi_agent_context = Default::default();
    } else {
        prepared.multi_agent_context = super::multi_agent_compat::prepare(
            &prepared.source_headers,
            body,
            policy.options,
            &policy.models,
        );
    }
    Ok(())
}
