// Compatibility policies shared by the public catalog and local token estimates.

fn native_catalog_prefers_websockets(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
) -> bool {
    collection.responses_websockets_enabled
        && !native_is_chatgpt_plan_account(account)
        && key.is_none_or(|key| key.provider_gateway.is_none() && key.model_routing.is_none())
        && !plan.xai
        && !codex_account::is_grok_upstream_provider(account)
}

fn native_token_count_model(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    profile: bool,
    model: &str,
) -> Result<String, native_contract::ServiceError> {
    let canonical = native_model_for_key(collection, key, model)?;
    // Explicit namespace routes preserve their public model. A bare alias that
    // falls back to OAuth still canonicalizes, even with modelRouting present.
    if let Some(routing) = key.and_then(|key| key.model_routing.as_ref()) {
        let public = key
            .and_then(|key| key.model_prefix.as_deref())
            .filter(|prefix| !prefix.is_empty())
            .and_then(|prefix| model.strip_prefix(&format!("{}/", prefix.trim_end_matches('/'))))
            .unwrap_or(model);
        if public.split_once('/').is_some_and(|(namespace, _)| {
            routing
                .routes
                .iter()
                .any(|route| route.namespace.eq_ignore_ascii_case(namespace))
        }) {
            return Ok(model.trim().to_owned());
        }
        return Ok(canonical);
    }
    // Go's automatic route canonicalizes only when executing a generation.
    if !profile && key.is_none_or(|key| key.provider_gateway.is_none()) {
        return Ok(model.trim().to_owned());
    }
    if key.is_some_and(|key| key.provider_gateway.is_some()) {
        return native_model_route_plan(collection, key, &canonical).map(|plan| plan.model);
    }
    Ok(canonical)
}
