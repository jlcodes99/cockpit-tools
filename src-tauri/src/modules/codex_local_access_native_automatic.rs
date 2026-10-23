// Reuse the host's public routing/catalog policy, while keeping native wire execution independent.

fn native_xai_base_denied(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
) -> bool {
    if !plan.xai && !codex_account::is_grok_upstream_provider(account) {
        return false;
    }
    let Ok((base, _)) = native_api::grok_thinking::parse_model(&plan.full_model) else {
        return true;
    };
    let upstream_base = base.rsplit('/').next().unwrap_or(&base);
    let excluded =
        |rules: &[String]| native_excluded(rules, &base) || native_excluded(rules, upstream_base);
    excluded(&collection.excluded_models)
        || key.is_some_and(|key| {
            excluded(&key.excluded_models)
                || (!key.allowed_models.is_empty() && !native_excluded(&key.allowed_models, &base))
        })
        || collection
            .account_model_rules
            .iter()
            .any(|rule| rule.account_id == account.id && excluded(&rule.excluded_models))
}

fn native_automatic_route_models(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Vec<Value> {
    let mut fallback = api_service_supported_codex_model_ids();
    if !fallback.iter().any(|model| native_is_reserve_model(model)) {
        fallback.push(CODEX_GPT_RESERVE_MODEL_ID.into());
    }
    automatic_api_service_route_models(collection, account, &fallback)
}

fn native_automatic_upstream_models(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Vec<String> {
    native_automatic_route_models(collection, account)
        .iter()
        .filter_map(|entry| entry["upstreamModel"].as_str().map(str::to_owned))
        .collect()
}

fn native_gateway_vision_fallback(gateway: &CodexLocalAccessProviderGateway) -> Option<String> {
    let models = native_gateway_models(gateway);
    if let Some(explicit) = gateway
        .vision_routing_model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
    {
        let matched = models
            .iter()
            .find(|model| model.eq_ignore_ascii_case(explicit.trim()))?;
        return provider_gateway_model_vision(gateway, matched).then(|| matched.clone());
    }
    let mut supported = models
        .into_iter()
        .filter(|model| provider_gateway_model_vision(gateway, model));
    let selected = supported.next()?;
    supported.next().is_none().then_some(selected)
}

fn native_account_model_vision(account: &CodexAccount, model: &str) -> bool {
    account
        .api_model_vision_support
        .iter()
        .find(|(name, _)| name.trim().eq_ignore_ascii_case(model.trim()))
        .map(|(_, vision)| *vision)
        .unwrap_or_else(|| {
            account.api_supports_vision
                || automatic_api_service_account_model_vision(account, model)
        })
}

fn native_effective_gateway_vision(
    gateway: &CodexLocalAccessProviderGateway,
    account: &CodexAccount,
) -> CodexLocalAccessProviderGateway {
    let mut effective = gateway.clone();
    // Existing explicit per-model gateway overrides keep their precedence. Older
    // snapshots may lack those fields, so complete them from current account policy.
    for model in native_gateway_models(gateway) {
        if gateway
            .model_capabilities
            .keys()
            .any(|name| name.trim().eq_ignore_ascii_case(model.trim()))
        {
            continue;
        }
        let supports_vision = account
            .api_model_vision_support
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(model.trim()))
            .map(|(_, vision)| *vision)
            .unwrap_or_else(|| {
                gateway.supports_vision || native_account_model_vision(account, &model)
            });
        effective.model_capabilities.insert(
            model.trim().to_ascii_lowercase(),
            CodexLocalAccessProviderGatewayModelCapability { supports_vision },
        );
    }
    effective.supports_vision |= account.api_supports_vision;
    if effective.vision_routing_model.is_none() {
        effective.vision_routing_model = account.api_vision_routing_model.clone();
    }
    effective
}

fn native_automatic_vision_fallback(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
) -> Option<String> {
    let models = native_automatic_upstream_models(collection, account);
    let mut capabilities = account.clone();
    for model in &models {
        capabilities.api_model_vision_support.insert(
            model.to_owned(),
            native_account_model_vision(account, model),
        );
    }
    let selected = automatic_api_service_vision_routing_model(&capabilities, &models)?;
    native_account_model_vision(account, &selected).then_some(selected)
}

fn native_plan_vision_model(
    collection: &CodexLocalAccessCollection,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
    model: &str,
) -> String {
    if plan.xai || codex_account::is_grok_upstream_provider(account) {
        return model.to_owned();
    }
    if let Some(gateway) = plan.gateway.as_ref() {
        let effective = native_effective_gateway_vision(gateway, account);
        if !provider_gateway_model_vision(&effective, model) {
            if let Some(vision) = native_gateway_vision_fallback(&effective) {
                return vision;
            }
        }
    } else if plan.automatic
        && automatic_api_service_provider_route_eligible(
            account,
            &native_automatic_upstream_models(collection, account),
        )
        && !native_account_model_vision(account, model)
    {
        if let Some(vision) = native_automatic_vision_fallback(collection, account) {
            return vision;
        }
    }
    model.to_owned()
}

fn native_catalog_vision(
    collection: &CodexLocalAccessCollection,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
    upstream: &str,
) -> bool {
    if let Some(gateway) = plan.gateway.as_ref() {
        let effective = native_effective_gateway_vision(gateway, account);
        return provider_gateway_model_vision(&effective, upstream)
            || native_gateway_vision_fallback(&effective).is_some();
    }
    if plan.automatic && account.is_api_key_auth() {
        return native_account_model_vision(account, upstream)
            || (automatic_api_service_provider_route_eligible(
                account,
                &native_automatic_upstream_models(collection, account),
            ) && native_automatic_vision_fallback(collection, account).is_some());
    }
    account
        .api_model_vision_support
        .iter()
        .find(|(model, _)| model.eq_ignore_ascii_case(upstream))
        .map(|(_, vision)| *vision)
        .unwrap_or(account.api_supports_vision)
}

fn native_prepare_provider_vision_body(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    account: &CodexAccount,
    mut body: Value,
) -> Value {
    if codex_account::is_grok_upstream_provider(account) {
        return body;
    }
    let model = body["model"].as_str().unwrap_or_default();
    let gateway = key
        .and_then(|key| key.provider_gateway.as_ref())
        .or_else(|| {
            key.and_then(|key| key.model_routing.as_ref())
                .and_then(|routing| {
                    routing
                        .routes
                        .iter()
                        .find(|route| {
                            route.provider_account_id == account.id
                                && native_gateway_models(&route.provider_gateway)
                                    .iter()
                                    .any(|candidate| candidate.eq_ignore_ascii_case(model))
                        })
                        .map(|route| &route.provider_gateway)
                })
        });
    let unsupported = if let Some(gateway) = gateway {
        !provider_gateway_model_vision(&native_effective_gateway_vision(gateway, account), model)
    } else if key.is_none_or(|key| key.model_routing.is_none() && key.provider_gateway.is_none())
        && automatic_api_service_provider_route_eligible(
            account,
            &native_automatic_upstream_models(collection, account),
        )
    {
        !native_account_model_vision(account, model)
    } else {
        false
    };
    if unsupported {
        let text_type = if body.get("messages").is_some() {
            "text"
        } else {
            "input_text"
        };
        for field in ["input", "messages", "response"] {
            if let Some(content) = body.get_mut(field) {
                native_omit_vision_content(content, text_type);
            }
        }
    }
    body
}

fn native_omit_vision_content(value: &mut Value, text_type: &str) {
    match value {
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| native_omit_vision_content(item, text_type)),
        Value::Object(item) => {
            if item
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| {
                    matches!(
                        kind.trim().to_ascii_lowercase().as_str(),
                        "input_image" | "image_url" | "image"
                    )
                })
            {
                *value = json!({"type":text_type,"text":"[Image omitted because the current model does not support image input.]"});
            } else {
                for field in ["content", "parts", "input", "messages", "response"] {
                    if let Some(content) = item.get_mut(field) {
                        native_omit_vision_content(content, text_type);
                    }
                }
            }
        }
        _ => {}
    }
}
