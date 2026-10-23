// Request-local credential selection; durable membership/official bindings never change.
#[derive(Clone)]
struct NativeOfficialLocalIntent {
    snapshot: crate::modules::chatgpt_plan::OfficialRouteSnapshot,
    failure: native_contract::OfficialFallbackFailure,
}

fn native_note_official_health(result: Result<(), String>) {
    if let Err(error) = result {
        // Storage/association functions return typed internal codes. Never log
        // a token, upstream response, file contents, or account identity here.
        let code = if error.len() <= 128
            && error
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
        {
            error.as_str()
        } else {
            "CHATGPT_PLAN_STORAGE"
        };
        logger::log_warn(&format!(
            "[NativeAPI] Cannot persist official route health: {code}"
        ));
    }
}

fn native_official_scope_fingerprint(
    collection: &CodexLocalAccessCollection,
    caller: &native_contract::Caller,
) -> String {
    fn ordered(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut entries = map.into_iter().collect::<Vec<_>>();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key, ordered(value)))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(values.into_iter().map(ordered).collect()),
            value => value,
        }
    }
    let key = collection.api_keys.iter().find(|key| key.id == caller.id).map(|key| json!({
        "id":key.id,"key":key.key,"enabled":key.enabled,"provider":key.provider_gateway,
        "routing":key.model_routing,"inherit":key.inherit_account_pool,"accounts":key.account_ids,
        "prefix":key.model_prefix,"allowed":key.allowed_models,"excluded":key.excluded_models,"limit":key.token_limit
    }));
    let material = json!({"caller":caller.id,"key":key,"enabled":collection.enabled,
        "primary":if collection.api_keys.is_empty() { collection.api_key.as_str() } else { "" },
        "members":collection.account_ids,"paused":collection.paused_account_ids,
        "official":collection.official_account_bindings,"revision":collection.chatgpt_plan_service_revision,
        "identities":collection.service_account_identity_fingerprints,"excluded":collection.excluded_models,
        "aliases":collection.model_aliases,"rules":collection.account_model_rules,"restrictFree":collection.restrict_free_accounts});
    format!(
        "{:x}",
        Sha256::digest(ordered(material).to_string().as_bytes())
    )
}

fn native_official_fallback_operation(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: &native_contract::RouteRequest,
) -> bool {
    host.profile_dir.is_none()
        && caller.id != NATIVE_INTERNAL_CALLER
        && request.target_account_id.is_none()
        && matches!(
            request.auxiliary.as_deref(),
            None | Some("responses.websocket")
        )
}

fn native_official_prepare_recoverable(error: &native_contract::ServiceError) -> bool {
    matches!(
        error.code,
        "chatgpt_plan_reauthorization_required"
            | "chatgpt_plan_permission_required"
            | "chatgpt_plan_verification_required"
            | "chatgpt_plan_temporarily_unavailable"
    )
}

fn native_local_fallback_model(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    request: &native_contract::RouteRequest,
    account: &CodexAccount,
) -> Result<String, native_contract::ServiceError> {
    if plan.provider
        || !plan.account_ids.contains(&account.id)
        || service_account_paused(collection, &account.id)
        || request.excluded_account_ids.contains(&account.id)
    {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "Original local authorization is outside the current request scope",
        ));
    }
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
        || codex_account::is_grok_upstream_provider(account)
        || !native_eligible(account)
        || (collection.restrict_free_accounts && account.plan_type.as_deref() == Some("free"))
    {
        return Err(native_error(
            503,
            "official_local_unavailable",
            "Original local credentials are unavailable",
        ));
    }
    let upstream = native_operation_model(
        collection,
        plan,
        account,
        request.auxiliary.as_deref(),
        request.image_input,
    )
    .ok_or_else(|| {
        native_error(
            403,
            "model_not_allowed",
            "The original local authorization does not expose this model",
        )
    })?;
    let known_model = native_is_reserve_model(&upstream)
        || api_service_routable_codex_model_ids()
            .iter()
            .chain(api_service_supported_codex_model_ids().iter())
            .any(|model| model.eq_ignore_ascii_case(&upstream));
    if (!native_is_reserve_model(&upstream) && native_quota(account) <= 0)
        || !native_model_quota_allows(collection, account, &upstream)
    {
        return Err(native_error(
            429,
            "official_local_unavailable",
            "Original local authorization has no available quota",
        ));
    }
    if !known_model
        || native_excluded(&collection.excluded_models, &upstream)
        || key.is_some_and(|key| native_excluded(&key.excluded_models, &upstream))
        || collection.account_model_rules.iter().any(|rule| {
            rule.account_id == account.id
                && (native_excluded(&rule.excluded_models, &plan.model)
                    || native_excluded(&rule.excluded_models, &upstream))
        })
    {
        return Err(native_error(
            403,
            "local_fallback_unavailable",
            "Original local authorization has no available model or quota",
        ));
    }
    Ok(upstream)
}

impl NativeHostBridge {
    async fn selected_fallback_current(
        &self,
        account: &str,
        registration: &str,
        revision: &str,
    ) -> bool {
        let Ok(collection) = self.collection().await else {
            return false;
        };
        self.profile_dir.is_none()
            && collection.account_ids.contains(&account.to_string())
            && !service_account_paused(&collection, account)
            && collection
                .official_account_bindings
                .get(account)
                .map(String::as_str)
                == Some(registration)
            && crate::modules::chatgpt_plan::fallback_epoch_matches(registration, account, revision)
                .await
    }

    async fn selected_route_current(&self, route: &native_contract::Route) -> bool {
        match (
            &route.official_fallback_registration_id,
            &route.official_fallback_authorization_revision,
        ) {
            (Some(id), Some(revision)) => {
                self.selected_fallback_current(&route.account_id, id, revision)
                    .await
            }
            (None, None) => {
                self.selected_authorization_current(
                    &route.account_id,
                    route.official_registration_id.as_deref(),
                    route.official_authorization_revision.as_deref(),
                )
                .await
            }
            _ => false,
        }
    }
}

async fn native_official_preflight_fallbacks(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    request: &native_contract::RouteRequest,
    accounts: &mut [CodexAccount],
) -> HashMap<String, NativeOfficialLocalIntent> {
    let mut intents = HashMap::new();
    if !native_official_fallback_operation(host, caller, request) || plan.provider {
        return intents;
    }
    let raw = host
        .load_accounts(
            accounts
                .iter()
                .filter(|account| account.official_registration_id.is_some())
                .map(|account| account.id.clone())
                .collect(),
        )
        .await;
    let Ok(raw) = raw else {
        return intents;
    };
    for account in accounts.iter_mut() {
        let Some(id) = account.official_registration_id.as_deref() else {
            continue;
        };
        if request.excluded_account_ids.contains(&format!(
            "{}{id}",
            codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX
        )) {
            continue;
        }
        let official_model = native_operation_model(
            collection,
            plan,
            account,
            request.auxiliary.as_deref(),
            request.image_input,
        )
        .unwrap_or_else(|| plan.model.clone());
        let Ok(snapshot) =
            crate::modules::chatgpt_plan::official_route_snapshot(id, &account.id, &official_model)
                .await
        else {
            continue;
        };
        let failure = if !snapshot.available {
            Some(native_contract::OfficialFallbackFailure { status: 0, code: snapshot.failure_code.clone().unwrap_or_else(|| "official_unavailable".into()), message: Some("Official authorization is paused or cooling; this request did not contact the official upstream".into()), retry_after: None, trigger_stage: "health_pause" })
        } else if !native_eligible(account) {
            Some(native_contract::OfficialFallbackFailure {
                status: 401,
                code: "authorization_failed".into(),
                message: Some(
                    "Selected official authorization is unavailable before sending".into(),
                ),
                retry_after: None,
                trigger_stage: "preflight",
            })
        } else if native_operation_model(
            collection,
            plan,
            account,
            request.auxiliary.as_deref(),
            request.image_input,
        )
        .is_none()
        {
            Some(native_contract::OfficialFallbackFailure {
                status: 400,
                code: "capability_unsupported".into(),
                message: Some("Selected official authorization does not expose this model".into()),
                retry_after: None,
                trigger_stage: "preflight",
            })
        } else {
            None
        };
        let Some(failure) = failure else {
            continue;
        };
        // A paused official route must not be retried merely because its local
        // fallback is unavailable. This flag is only in this request's overlay.
        if !snapshot.available {
            account.requires_reauth = true;
        }
        let Some(source) = raw.iter().find(|source| source.id == account.id) else {
            continue;
        };
        if native_local_fallback_model(collection, key, plan, request, source).is_err() {
            continue;
        }
        intents.insert(
            source.id.clone(),
            NativeOfficialLocalIntent { snapshot, failure },
        );
        *account = source.clone();
    }
    intents
}

async fn native_attach_official_fallback(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    mut route: native_contract::Route,
    intent: NativeOfficialLocalIntent,
    scope: &str,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    let current = host.collection().await?;
    if native_official_scope_fingerprint(&current, caller) != scope
        || !host
            .selected_fallback_current(
                &route.account_id,
                &intent.snapshot.registration_id,
                &intent.snapshot.authorization_revision,
            )
            .await
    {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "Authorization changed while preparing the local fallback",
        ));
    }
    route.official_fallback_registration_id = Some(intent.snapshot.registration_id);
    route.official_fallback_authorization_revision = Some(intent.snapshot.authorization_revision);
    route.authorization_scope_fingerprint = Some(scope.into());
    route.official_fallback_failure = Some(intent.failure);
    Ok(route)
}

async fn native_prepare_official_local_fallback(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: native_contract::RouteRequest,
    intent: NativeOfficialLocalIntent,
    scope: &str,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller).map_err(|_| {
        native_error(
            409,
            "official_fallback_authorization_changed",
            "API key authorization changed",
        )
    })?;
    if !native_official_fallback_operation(host, caller, &request)
        || request.excluded_account_ids.contains(&format!(
            "{}{}",
            codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX,
            intent.snapshot.registration_id
        ))
        || native_official_scope_fingerprint(&collection, caller) != scope
        || !host
            .selected_fallback_current(
                &intent.snapshot.source_account_id,
                &intent.snapshot.registration_id,
                &intent.snapshot.authorization_revision,
            )
            .await
    {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "Official fallback authorization changed",
        ));
    }
    let public_model = native_model_for_key(&collection, key, &request.model)?;
    let plan = native_model_route_plan(&collection, key, &public_model)?;
    let source = host
        .load_accounts(vec![intent.snapshot.source_account_id.clone()])
        .await?
        .pop()
        .ok_or_else(|| {
            native_error(
                404,
                "account_not_found",
                "Original local account is unavailable",
            )
        })?;
    if crate::modules::chatgpt_plan::source_identity_fingerprint(&source)
        .ok()
        .as_deref()
        != Some(intent.snapshot.source_fingerprint.as_str())
    {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "Original local workspace changed",
        ));
    }
    let upstream = native_local_fallback_model(&collection, key, &plan, &request, &source)?;
    let cooldown = host
        .cooldowns
        .lock()
        .map_err(|_| native_error(503, "service_unavailable", "Account state is unavailable"))
        .map(|states| {
            native_cooldown_wait(
                &states,
                &source.id,
                &upstream,
                collection.disable_cooling,
                Instant::now(),
            )
        })?;
    if let Some(wait) = cooldown {
        return Err(native_error(
            503,
            "account_cooling",
            "Original local authorization is cooling",
        )
        .with_retry_after(Some(wait)));
    }
    let (_, permit) = native_acquire_account_permit(
        host,
        &collection,
        &[source.clone()],
        request.requires_permit,
    )
    .await?;
    let route = native_route_for_reserved_account(
        host,
        &collection,
        source,
        upstream,
        request.force_refresh,
        permit,
        request.auxiliary.as_deref() == Some("responses.websocket"),
    )
    .await?;
    native_attach_official_fallback(host, caller, route, intent, scope).await
}

async fn native_refresh_route(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    previous: &native_contract::Route,
    request: native_contract::RouteRequest,
) -> Result<native_contract::Route, native_contract::ServiceError> {
    if previous.official_fallback_registration_id.is_none()
        && previous.official_fallback_authorization_revision.is_none()
        && previous.official_fallback_failure.is_none()
    {
        return native_select_route(host, caller, request).await;
    }
    let changed = || {
        native_error(
            409,
            "official_fallback_authorization_changed",
            "Local fallback authorization changed before refresh",
        )
    };
    let (Some(id), Some(revision), Some(scope), Some(failure)) = (
        previous.official_fallback_registration_id.as_deref(),
        previous.official_fallback_authorization_revision.as_deref(),
        previous.authorization_scope_fingerprint.as_deref(),
        previous.official_fallback_failure.as_ref(),
    ) else {
        return Err(changed());
    };
    if previous.chatgpt_plan
        || !previous.chatgpt
        || previous.official_registration_id.is_some()
        || previous.wire_api != native_contract::WireApi::Responses
    {
        return Err(changed());
    }
    let snapshot = crate::modules::chatgpt_plan::official_route_snapshot(
        id,
        &previous.account_id,
        &previous.model,
    )
    .await
    .map_err(|_| changed())?;
    if snapshot.authorization_revision != revision {
        return Err(changed());
    }
    // Reuse the local transaction, including source identity, current caller
    // scope, binding, quota/cooldown and final authorization checks. A healthy
    // official route must not change this request's credential namespace.
    native_prepare_official_local_fallback(
        host,
        caller,
        request,
        NativeOfficialLocalIntent {
            snapshot,
            failure: failure.clone(),
        },
        scope,
    )
    .await
}

async fn native_fallback_to_local(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    failed: &native_contract::Route,
    request: native_contract::RouteRequest,
    failure: native_contract::OfficialFallbackFailure,
) -> Result<Option<native_contract::Route>, native_contract::ServiceError> {
    let (Some(id), Some(revision), Some(scope)) = (
        failed.official_registration_id.as_deref(),
        failed.official_authorization_revision.as_deref(),
        failed.authorization_scope_fingerprint.as_deref(),
    ) else {
        return Ok(None);
    };
    if !failed.chatgpt_plan
        || failed
            .account_id
            .starts_with(codex_account::CHATGPT_PLAN_ACCOUNT_PREFIX)
        || !native_official_fallback_operation(host, caller, &request)
    {
        return Ok(None);
    }
    let snapshot = crate::modules::chatgpt_plan::official_route_snapshot(
        id,
        &failed.account_id,
        &request.model,
    )
    .await
    .map_err(|_| {
        native_error(
            409,
            "official_fallback_authorization_changed",
            "Official fallback source is no longer available",
        )
    })?;
    if snapshot.authorization_revision != revision {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "Official authorization changed",
        ));
    }
    let current = host.collection().await?;
    if native_official_scope_fingerprint(&current, caller) != scope {
        return Err(native_error(
            409,
            "official_fallback_authorization_changed",
            "API key authorization changed",
        ));
    }
    native_prepare_official_local_fallback(
        host,
        caller,
        request,
        NativeOfficialLocalIntent { snapshot, failure },
        scope,
    )
    .await
    .map(Some)
}

async fn native_observe_official_failure(
    route: &native_contract::Route,
    model: &str,
    status: u16,
    message: Option<&str>,
    retry_after: Option<Duration>,
) {
    if !matches!(status, 401 | 403 | 429 | 500..=599) {
        return;
    }
    native_observe_official_rejection(
        route,
        model,
        &native_contract::OfficialFallbackFailure {
            status,
            code: "official_unavailable".into(),
            message: message.map(str::to_owned),
            retry_after,
            trigger_stage: "upstream",
        },
    )
    .await;
}

async fn native_observe_official_rejection(
    route: &native_contract::Route,
    model: &str,
    failure: &native_contract::OfficialFallbackFailure,
) {
    // Only actual upstream rejections establish route health. History safety
    // and availability of the same source's local credentials are independent.
    if failure.trigger_stage != "upstream" || !route.chatgpt_plan {
        return;
    }
    let (Some(id), Some(revision)) = (
        &route.official_registration_id,
        &route.official_authorization_revision,
    ) else {
        return;
    };
    let Ok(snapshot) =
        crate::modules::chatgpt_plan::official_route_snapshot(id, &route.account_id, model).await
    else {
        return;
    };
    if snapshot.authorization_revision != *revision {
        return;
    }
    let fingerprint = route
        .headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(crate::modules::chatgpt_plan::token_fingerprint);
    native_note_official_health(
        crate::modules::chatgpt_plan::record_official_route_failure(
            &snapshot,
            model,
            failure.status,
            &failure.code,
            failure.message.as_deref(),
            failure.retry_after,
            fingerprint.as_deref(),
        )
        .await,
    );
}
