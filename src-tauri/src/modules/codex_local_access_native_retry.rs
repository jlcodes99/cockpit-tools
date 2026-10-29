async fn native_retry_cooldown_wait(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: native_contract::RouteRequest,
    max_wait: Duration,
) -> Result<Option<Duration>, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller)?;
    if matches!(
        request.auxiliary.as_deref(),
        Some("images.generations" | "images.edits")
    ) {
        return native_image_retry_cooldown_wait(host, caller, &collection, key, request, max_wait)
            .await;
    }
    let model = native_model_for_key(&collection, key, &request.model)?;
    let plan = native_model_route_plan(&collection, key, &model)?;
    let ids = if caller.id == NATIVE_INTERNAL_CALLER {
        request
            .target_account_id
            .clone()
            .filter(|id| internal_api_account_ids().contains(id))
            .into_iter()
            .collect()
    } else {
        if request.target_account_id.is_some() {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account header is reserved for internal callers",
            ));
        }
        plan.account_ids.clone()
    };
    let mut accounts = host
        .load_routing_accounts(ids, caller.id != NATIVE_INTERNAL_CALLER && !plan.provider)
        .await?;
    let _ = native_official_preflight_fallbacks(
        host,
        caller,
        &collection,
        key,
        &plan,
        &request,
        &mut accounts,
    )
    .await;
    let mut candidates = Vec::new();
    for account in accounts {
        if (caller.id != NATIVE_INTERNAL_CALLER && service_account_paused(&collection, &account.id))
            || !native_eligible(&account)
            || !native_auxiliary_eligible(&account, request.auxiliary.as_deref())
            || request.preferred_account_id.as_ref().is_some_and(|id| {
                id != &account.id
                    && account
                        .official_registration_id
                        .as_ref()
                        .is_none_or(|grant| {
                            id != &format!("{}{grant}", codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
                        })
            })
            || (plan.oauth_only && !native_is_oauth_authorization_account(&account))
            || native_xai_base_denied(&collection, key, &plan, &account)
            || (collection.restrict_free_accounts
                && account.plan_type.as_deref() == Some("free")
                && (!account.is_api_key_auth() || native_is_chatgpt_plan_account(&account)))
        {
            continue;
        }
        let Some(upstream) = native_operation_model(
            &collection,
            &plan,
            &account,
            request.auxiliary.as_deref(),
            request.image_input,
        ) else {
            continue;
        };
        if native_excluded(&collection.excluded_models, &upstream)
            || key.is_some_and(|key| native_excluded(&key.excluded_models, &upstream))
            || collection.account_model_rules.iter().any(|rule| {
                rule.account_id == account.id
                    && (native_excluded(&rule.excluded_models, &plan.model)
                        || native_excluded(&rule.excluded_models, &upstream))
            })
        {
            continue;
        }
        let Some(quota_wait) = native_model_quota_retry_wait(&collection, &account, &upstream)
        else {
            continue;
        };
        let official = native_is_chatgpt_plan_account(&account);
        candidates.push((account.id, upstream, quota_wait, official));
    }
    // Do not hold the cooldown mutex while reading quota reservation policy.
    let states = host
        .cooldowns
        .lock()
        .map_err(|_| native_error(503, "service_unavailable", "Account state is unavailable"))?;
    let mut closest = None;
    for (account_id, upstream, quota_wait, official) in candidates {
        let wait = if official {
            Duration::ZERO
        } else {
            native_cooldown_wait(
                &states,
                &account_id,
                &upstream,
                collection.disable_cooling,
                Instant::now(),
            )
            .unwrap_or(Duration::ZERO)
        }
        .max(quota_wait);
        closest = Some(closest.map_or(wait, |current: Duration| current.min(wait)));
    }
    Ok(closest.filter(|wait| *wait <= max_wait))
}

async fn native_image_retry_cooldown_wait(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    request: native_contract::RouteRequest,
    max_wait: Duration,
) -> Result<Option<Duration>, native_contract::ServiceError> {
    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return Ok(None);
    }
    let image_request = native_contract::ImageRouteRequest {
        model: Some(request.model),
        responses_model: None,
        edit: request.auxiliary.as_deref() == Some("images.edits"),
        target_account_id: request.target_account_id.clone(),
        force_refresh: false,
        preferred_account_id: request.preferred_account_id.clone(),
        excluded_account_ids: vec![],
    };
    let model = native_image_requested_model(collection, key, &image_request)?;
    let ids = if caller.id == NATIVE_INTERNAL_CALLER {
        request
            .target_account_id
            .filter(|id| internal_api_account_ids().contains(id))
            .into_iter()
            .collect()
    } else {
        if request.target_account_id.is_some() {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account header is reserved for internal callers",
            ));
        }
        if collection.image_generation_account_ids.is_empty() {
            native_scoped_account_ids(collection, key)
        } else {
            collection.image_generation_account_ids.clone()
        }
    };
    let accounts = host
        .load_routing_accounts(
            ids,
            caller.id != NATIVE_INTERNAL_CALLER
                && collection.image_generation_account_ids.is_empty(),
        )
        .await?;
    let states = host
        .cooldowns
        .lock()
        .map_err(|_| native_error(503, "service_unavailable", "Account state is unavailable"))?;
    let mut closest = None;
    for account in accounts {
        if (caller.id != NATIVE_INTERNAL_CALLER && service_account_paused(collection, &account.id))
            || account.is_web_session_auth()
            || account.is_agent_identity_auth()
            || account.requires_reauth
            || codex_account::account_has_remote_api_auth_rejection(&account)
            || request
                .preferred_account_id
                .as_ref()
                .is_some_and(|id| id != &account.id)
            || (collection.restrict_free_accounts
                && is_free_plan_type(account.plan_type.as_deref())
                && (!account.is_api_key_auth() || native_is_chatgpt_plan_account(&account)))
            || !native_quota_allows(collection, &account)
            || !native_image_source_allowed(collection, &account, &model)
            || !native_image_model_allowed(collection, key, &account, &model)
        {
            continue;
        }
        let Some(upstream) = native_image_source_model(collection, &account, &model) else {
            continue;
        };
        let wait = native_cooldown_wait(
            &states,
            &account.id,
            &upstream,
            collection.disable_cooling,
            Instant::now(),
        )
        .unwrap_or(Duration::ZERO);
        closest = Some(closest.map_or(wait, |current: Duration| current.min(wait)));
    }
    Ok(closest.filter(|wait| *wait <= max_wait))
}
