// Conversation-model compatibility applies only to declared conversation IDs.
// Unknown/custom image IDs and explicit GPT image variants remain exact IDs.
fn native_image_public_requested_model<'a>(
    collection: &'a CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    request: &'a native_contract::ImageRouteRequest,
) -> &'a str {
    let requested = request
        .model
        .as_deref()
        .unwrap_or(&collection.image_generation_model);
    if request.responses_model.is_some() || request.model.is_none() {
        return requested;
    }
    key.and_then(|key| key.model_prefix.as_deref())
        .filter(|prefix| !prefix.is_empty())
        .and_then(|prefix| requested.strip_prefix(&format!("{}/", prefix.trim_end_matches('/'))))
        .unwrap_or(requested)
}

fn native_image_known_conversation_model(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    model: &str,
    accounts: &[CodexAccount],
) -> bool {
    let is_model = |value: &str| value.trim().eq_ignore_ascii_case(model);
    if model.to_ascii_lowercase().starts_with("gpt-image-")
        || is_model(&collection.image_generation_model)
        || collection
            .image_generation_models
            .iter()
            .any(|value| is_model(value))
        || collection
            .image_generation_deleted_models
            .iter()
            .any(|value| is_model(value))
        || collection
            .image_generation_account_models
            .values()
            .any(|value| is_model(value))
    {
        return false;
    }
    let official = api_service_supported_codex_model_ids();
    let official_model = |value: &str| official.iter().any(|id| id.eq_ignore_ascii_case(value));
    if official_model(model)
        || collection.model_aliases.iter().any(|alias| {
            (is_model(&alias.source_model) && official_model(&alias.alias))
                || (is_model(&alias.alias) && official_model(&alias.source_model))
        })
    {
        return true;
    }
    if let Some(key) = key {
        if key
            .provider_gateway
            .as_ref()
            .is_some_and(|gateway| native_gateway_models(gateway).iter().any(|id| is_model(id)))
            || key.model_routing.as_ref().is_some_and(|routing| {
                routing.routes.iter().any(|route| {
                    model
                        .strip_prefix(&format!("{}/", route.namespace))
                        .is_some_and(|upstream| {
                            native_gateway_models(&route.provider_gateway)
                                .iter()
                                .any(|id| id.eq_ignore_ascii_case(upstream))
                        })
                })
            })
        {
            return true;
        }
    }
    let scope = native_scoped_account_ids(collection, key);
    accounts
        .iter()
        .filter(|account| scope.contains(&account.id))
        .any(|account| {
            account.api_model_catalog.iter().any(|id| is_model(id))
                || account.api_model_mappings.iter().any(|mapping| {
                    is_model(&mapping.client_model) || is_model(&mapping.upstream_model)
                })
        })
}

fn native_image_main_model(
    collection: &CodexLocalAccessCollection,
    responses_model: Option<&str>,
    use_configured: bool,
) -> String {
    if !use_configured {
        if let Some(model) = responses_model {
            return model.to_owned();
        }
    }
    effective_image_generation_main_model(collection.image_generation_main_model.as_deref())
        .to_owned()
}

fn native_image_main_model_allowed(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    account: &CodexAccount,
    model: &str,
) -> bool {
    !native_excluded(&collection.excluded_models, model)
        && !key.is_some_and(|key| native_excluded(&key.excluded_models, model))
        && !collection.account_model_rules.iter().any(|rule| {
            rule.account_id == account.id && native_excluded(&rule.excluded_models, model)
        })
}
