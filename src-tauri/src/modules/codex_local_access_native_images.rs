// The explicit image source pool is independent of text/provider routes, as in
// the saved host configuration. Credentials and per-account limits are shared.
async fn native_image_route(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: native_contract::ImageRouteRequest,
) -> Result<native_contract::ImageRoute, native_contract::ServiceError> {
    let first = native_image_route_once(host, caller, request.clone()).await;
    let Err(error) = &first else {
        return first;
    };
    let max_wait = Duration::from_millis(host.collection().await?.max_retry_interval_ms);
    if let Some(wait) = error
        .retry_after
        .filter(|wait| !wait.is_zero() && *wait <= max_wait)
    {
        tokio::time::sleep(wait).await;
        return native_image_route_once(host, caller, request).await;
    }
    first
}

async fn native_image_route_once(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    request: native_contract::ImageRouteRequest,
) -> Result<native_contract::ImageRoute, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller)?;
    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return Err(native_error(
            403,
            "image_generation_disabled",
            "Image generation is disabled for this service",
        ));
    }
    let responses_model = request
        .responses_model
        .as_deref()
        .map(|model| native_model_for_key(&collection, key, model))
        .transpose()?;
    if request.responses_model.is_some()
        && collection.image_generation_mode == CodexLocalAccessImageGenerationMode::ImagesOnly
    {
        return Err(native_error(
            403,
            "image_generation_disabled",
            "Hosted image tools are disabled for Responses requests",
        ));
    }
    let model = native_image_requested_model(&collection, key, &request)?;
    let client_image_model = native_image_public_requested_model(&collection, key, &request);
    let ids = if caller.id == NATIVE_INTERNAL_CALLER {
        let id = request.target_account_id.as_ref().ok_or_else(|| {
            native_error(
                400,
                "missing_account",
                "Internal image request requires a target account",
            )
        })?;
        if !internal_api_account_ids().contains(id) {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account is not registered",
            ));
        }
        vec![id.clone()]
    } else {
        if request.target_account_id.is_some() {
            return Err(native_error(
                403,
                "account_not_allowed",
                "Target account header is reserved for internal callers",
            ));
        }
        if collection.image_generation_account_ids.is_empty() {
            native_scoped_account_ids(&collection, key)
        } else {
            collection.image_generation_account_ids.clone()
        }
    };
    // The conversation catalog only identifies a declared fallback model. It
    // never adds an account to the independently selected image source pool.
    let mut catalog_ids = ids.clone();
    for id in native_scoped_account_ids(&collection, key) {
        if !catalog_ids.contains(&id) {
            catalog_ids.push(id);
        }
    }
    let mut accounts = host.load_routing_accounts(catalog_ids, caller.id != NATIVE_INTERNAL_CALLER && collection.image_generation_account_ids.is_empty()).await?;
    let use_configured_main = request.responses_model.is_none()
        || !matches!(
            collection.image_generation_model.trim(),
            "gpt-image-2" | "gpt-image-2.5"
        )
        || ids.iter().any(|id| {
            accounts
                .iter()
                .find(|account| &account.id == id)
                .is_none_or(CodexAccount::is_api_key_auth)
        });
    let selection = if !collection.image_generation_account_ids.is_empty()
        && native_image_known_conversation_model(&collection, key, &model, &accounts)
    {
        collection.image_generation_model.trim()
    } else {
        native_image_selection_model(&collection, &model)
    };
    accounts.retain(|account| {
        ids.contains(&account.id)
            && (caller.id == NATIVE_INTERNAL_CALLER || !service_account_paused(&collection, &account.id))
            && !account.is_web_session_auth()
            && !request.excluded_account_ids.contains(&account.id)
            && !account.is_agent_identity_auth()
            && !account.requires_reauth
            && !codex_account::account_has_remote_api_auth_rejection(account)
            && !(collection.restrict_free_accounts
                && is_free_plan_type(account.plan_type.as_deref())
                && !account.is_api_key_auth())
            && native_quota_allows(&collection, account)
            && native_image_source_allowed(&collection, account, selection)
            && native_image_model_allowed_for_selection(
                &collection,
                key,
                account,
                &model,
                selection,
            )
            && !collection.account_model_rules.iter().any(|rule| {
                rule.account_id == account.id
                    && native_excluded(&rule.excluded_models, client_image_model)
            })
            && (account.is_api_key_auth()
                || native_image_main_model_allowed(
                    &collection,
                    key,
                    account,
                    &native_image_main_model(
                        &collection,
                        responses_model.as_deref(),
                        use_configured_main,
                    ),
                ))
    });
    let mut closest_cooldown = None;
    {
        let cooldowns = host.cooldowns.lock().map_err(|_| {
            native_error(503, "service_unavailable", "Account state is unavailable")
        })?;
        accounts.retain(|account| {
            let upstream = native_image_source_model(&collection, account, selection)
                .unwrap_or_else(|| model.clone());
            let wait = native_cooldown_wait(
                &cooldowns,
                &account.id,
                &upstream,
                collection.disable_cooling,
                Instant::now(),
            );
            if request
                .preferred_account_id
                .as_ref()
                .is_none_or(|id| id == &account.id)
            {
                if let Some(wait) = wait {
                    closest_cooldown =
                        Some(closest_cooldown.map_or(wait, |current: Duration| current.min(wait)));
                }
            }
            wait.is_none()
        });
    }
    let routing_namespace =
        native_routing_namespace(caller, &model, &ids, request.target_account_id.as_deref());
    native_sort_accounts_scoped(host, &mut accounts, &collection, key, &routing_namespace);
    if let Some(preferred) = &request.preferred_account_id {
        accounts.retain(|account| &account.id == preferred);
    }
    if accounts.is_empty() {
        return Err(native_error(
            404,
            "image_source_unavailable",
            "No selected image source supports this model",
        )
        .with_retry_after(closest_cooldown));
    }
    let (position, account_permit, image_permit) =
        native_acquire_image_source_permits(host, &collection, &accounts).await?;
    let account = accounts.swap_remove(position);
    let image_model =
        native_image_source_model(&collection, &account, selection).ok_or_else(|| {
            native_error(
                404,
                "image_model_unavailable",
                "The selected image model is unavailable",
            )
        })?;
    let main_model = (!account.is_api_key_auth()).then(|| {
        native_image_main_model(&collection, responses_model.as_deref(), use_configured_main)
    });
    let mut route = native_route_for_reserved_account(
        host,
        &collection,
        account,
        main_model.clone().unwrap_or_else(|| image_model.clone()),
        request.force_refresh,
        account_permit,
        false,
    )
    .await?;
    native_commit_routing_cursor(host, &routing_namespace, &route.account_id);
    if let Some(permit) = image_permit {
        route.permit = Some(Arc::new((route.permit.take(), permit)));
    }
    Ok(native_contract::ImageRoute {
        route,
        image_model,
        main_model,
    })
}

fn native_image_requested_model(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    request: &native_contract::ImageRouteRequest,
) -> Result<String, native_contract::ServiceError> {
    let requested = request
        .model
        .as_deref()
        .unwrap_or(&collection.image_generation_model);
    // Namespaces belong to the public top-level model. A server default or the
    // nested Responses image tool model remains an actual image model ID.
    let mut scoped_key = key.cloned();
    if request.responses_model.is_some() || request.model.is_none() {
        if let Some(key) = scoped_key.as_mut() {
            key.model_prefix = None;
        }
    }
    if let Some(key) = scoped_key.as_mut() {
        let client_model = match key
            .model_prefix
            .as_deref()
            .filter(|prefix| !prefix.is_empty())
        {
            Some(prefix) => requested.strip_prefix(&format!("{}/", prefix.trim_end_matches('/'))),
            None => Some(requested),
        };
        // As in Go, an allowed configured image model authorizes its bare
        // compatibility names. Keep namespace and exclusion checks intact;
        // the actual account model is checked separately after resolution.
        if let Some(model @ ("gpt-image-2" | "gpt-image-2.5")) = client_model {
            if native_excluded(
                &key.allowed_models,
                collection.image_generation_model.trim(),
            ) {
                key.allowed_models.push(model.to_owned());
            }
        }
    }
    native_model_for_key(collection, scoped_key.as_ref(), requested)
}

fn native_image_slot_gate(
    host: &NativeHostBridge,
    account: &str,
    limit: u16,
) -> Result<Option<Arc<Semaphore>>, native_contract::ServiceError> {
    if limit == 0 {
        return Ok(None);
    }
    let mut gates = host
        .gates
        .lock()
        .map_err(|_| native_error(503, "service_unavailable", "Image limiter is unavailable"))?;
    Ok(Some(
        gates
            .entry(format!("image:account:{account}:{limit}"))
            .or_insert_with(|| Arc::new(Semaphore::new(limit as usize)))
            .clone(),
    ))
}

async fn native_acquire_image_source_permits(
    host: &NativeHostBridge,
    collection: &CodexLocalAccessCollection,
    accounts: &[CodexAccount],
) -> Result<
    (
        usize,
        Option<Arc<dyn Send + Sync>>,
        Option<tokio::sync::OwnedSemaphorePermit>,
    ),
    native_contract::ServiceError,
> {
    let limit = collection.max_concurrent_image_requests;
    if limit == 0 {
        let (index, permit) =
            native_acquire_account_permit(host, collection, accounts, true).await?;
        return Ok((index, permit, None));
    }
    let mut immediate = collection.clone();
    immediate.account_concurrency_wait_ms = 0;
    loop {
        let mut candidates = accounts
            .iter()
            .enumerate()
            .map(|(index, account)| {
                native_image_slot_gate(host, &account.id, limit).map(|gate| (index, gate.unwrap()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        candidates
            .sort_by_key(|(_, gate)| usize::from(limit).saturating_sub(gate.available_permits()));
        let mut image_busy = Vec::new();
        for (index, gate) in candidates {
            let account_permit = match native_acquire_account_permit(
                host,
                &immediate,
                &accounts[index..index + 1],
                true,
            )
            .await
            {
                Ok((_, permit)) => permit,
                Err(error) if matches!(error.code, "account_busy" | "quota_reserved") => continue,
                Err(error) => return Err(error),
            };
            if let Ok(image_permit) = gate.try_acquire_owned() {
                return Ok((index, account_permit, Some(image_permit)));
            }
            drop(account_permit);
            image_busy.push(accounts[index].clone());
        }
        if !image_busy.is_empty() {
            // Wait only on image sources whose account gate was available.
            // Release the wake-up reservation and recheck both gates together.
            let (_, permit) = native_wait_image_source_permit(host, &image_busy, limit).await?;
            drop(permit);
            continue;
        }
        // All account gates are busy. Keep their existing configured wait and
        // never broaden the candidate set of a protected/pinned request.
        let (index, account_permit) =
            native_acquire_account_permit(host, collection, accounts, true).await?;
        let (_, image_permit) =
            native_wait_image_source_permit(host, &accounts[index..index + 1], limit).await?;
        return Ok((index, account_permit, image_permit));
    }
}

#[cfg(test)]
fn native_acquire_image_permit(
    host: &NativeHostBridge,
    account: &str,
    limit: u16,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, native_contract::ServiceError> {
    let Some(gate) = native_image_slot_gate(host, account, limit)? else {
        return Ok(None);
    };
    gate.try_acquire_owned().map(Some).map_err(|_| {
        native_error(
            429,
            "image_concurrency_exceeded",
            "Image concurrency limit reached",
        )
    })
}

async fn native_wait_image_source_permit(
    host: &NativeHostBridge,
    accounts: &[CodexAccount],
    limit: u16,
) -> Result<(usize, Option<tokio::sync::OwnedSemaphorePermit>), native_contract::ServiceError> {
    if limit == 0 {
        return Ok((0, None));
    }
    let mut gates = accounts
        .iter()
        .enumerate()
        .map(|(index, account)| {
            native_image_slot_gate(host, &account.id, limit).map(|gate| (index, gate.unwrap()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Go spreads image work by the number already running on each account.
    gates.sort_by_key(|(_, gate)| usize::from(limit).saturating_sub(gate.available_permits()));
    for (index, gate) in &gates {
        if let Ok(permit) = gate.clone().try_acquire_owned() {
            return Ok((*index, Some(permit)));
        }
    }
    // Preparation's stop/deadline scope bounds this wait. Do not return a
    // service-global 429 while another image account may soon release a slot.
    let mut waiting: futures_util::stream::FuturesUnordered<_> = gates
        .into_iter()
        .map(|(index, gate)| async move { (index, gate.acquire_owned().await) })
        .collect();
    let (index, permit) = waiting.next().await.ok_or_else(|| {
        native_error(
            503,
            "image_source_unavailable",
            "No image source is available",
        )
    })?;
    let permit =
        permit.map_err(|_| native_error(503, "service_unavailable", "Image limiter stopped"))?;
    Ok((index, Some(permit)))
}

fn native_image_model_allowed(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    account: &CodexAccount,
    model: &str,
) -> bool {
    native_image_model_allowed_for_selection(
        collection,
        key,
        account,
        model,
        native_image_selection_model(collection, model),
    )
}

fn native_image_model_allowed_for_selection(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    account: &CodexAccount,
    model: &str,
    selected: &str,
) -> bool {
    let Some(actual) = native_image_source_model(collection, account, selected) else {
        return false;
    };
    ![model, selected, actual.as_str()].iter().any(|model| {
        native_excluded(&collection.excluded_models, model)
            || key.is_some_and(|key| native_excluded(&key.excluded_models, model))
    }) && !key.is_some_and(|key| {
        !key.allowed_models.is_empty() && !native_excluded(&key.allowed_models, &actual)
    }) && !collection.account_model_rules.iter().any(|rule| {
        rule.account_id == account.id
            && (native_excluded(&rule.excluded_models, model)
                || native_excluded(&rule.excluded_models, selected)
                || native_excluded(&rule.excluded_models, &actual))
    })
}

include!("codex_local_access_native_image_fallback.rs");

fn native_prepare_image_body(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    headers: &axum::http::HeaderMap,
    mut body: Value,
    dedicated_image: bool,
    compact: bool,
) -> Value {
    // The independent plan wire validates unsupported tools itself; do not
    // silently remove a requested capability before that validation runs.
    if native_is_chatgpt_plan_account(account) {
        return body;
    }
    let override_mode = headers
        .get("x-agtools-disable-image-generation")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let disabled = collection.image_generation_mode
        == CodexLocalAccessImageGenerationMode::Disabled
        || matches!(override_mode.as_str(), "true" | "all" | "disabled");
    let images_only = collection.image_generation_mode
        == CodexLocalAccessImageGenerationMode::ImagesOnly
        || matches!(
            override_mode.as_str(),
            "chat" | "images_only" | "images-only"
        )
        || headers.contains_key("x-openai-internal-codex-responses-lite");
    let allowed = if dedicated_image {
        !disabled
            && collection
                .image_generation_account_policies
                .get(&account.id)
                != Some(&CodexLocalAccessImageGenerationPolicy::Disabled)
    } else {
        !disabled
            && !images_only
            && native_image_allows(collection, account)
            && !is_free_plan_type(account.plan_type.as_deref())
    };
    if !allowed || native_image_tool_conflict(&body) {
        native_strip_image_tools(&mut body);
    } else if !compact && !dedicated_image && !account.is_api_key_auth() {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !model.is_empty()
            && !model.to_ascii_lowercase().ends_with("spark")
            && !codex_protocol::codex_model_uses_responses_lite(model)
            && !native_contains_image_tool(&body)
        {
            if let Some(object) = body.as_object_mut() {
                let tools = object.entry("tools").or_insert_with(|| json!([]));
                if tools.is_null() {
                    *tools = json!([]);
                }
                if let Some(tools) = tools.as_array_mut() {
                    tools.push(json!({"type":"image_generation","output_format":"png"}));
                }
            }
        }
    }
    body
}

fn native_tool_groups(body: &Value) -> Vec<&Value> {
    let mut groups = vec![body];
    if let Some(input) = body.get("input").and_then(Value::as_array) {
        for item in input
            .iter()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools"))
        {
            groups.extend(native_tool_groups(item));
        }
    }
    if let Some(response) = body.get("response").filter(|value| value.is_object()) {
        groups.extend(native_tool_groups(response));
    }
    groups
}

fn native_contains_image_tool(body: &Value) -> bool {
    native_tool_groups(body).into_iter().any(|group| {
        group
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type").and_then(Value::as_str) == Some("image_generation")
                })
            })
    })
}

fn native_image_tool_conflict(body: &Value) -> bool {
    native_tool_groups(body)
        .into_iter()
        .filter_map(|group| group.get("tools").and_then(Value::as_array))
        .flatten()
        .any(|tool| {
            let matches = |value: Option<&str>, expected: &str| {
                value.is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
            };
            matches(
                tool.get("name").and_then(Value::as_str),
                "image_gen.imagegen",
            ) || matches(
                tool.pointer("/function/name").and_then(Value::as_str),
                "image_gen.imagegen",
            ) || (["name", "namespace"]
                .iter()
                .any(|key| matches(tool.get(*key).and_then(Value::as_str), "image_gen"))
                && tool
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|children| {
                        children.iter().any(|child| {
                            matches(child.get("name").and_then(Value::as_str), "imagegen")
                                || matches(
                                    child.pointer("/function/name").and_then(Value::as_str),
                                    "imagegen",
                                )
                        })
                    }))
        })
}

fn native_strip_image_tools(body: &mut Value) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| tool.get("type").and_then(Value::as_str) != Some("image_generation"));
    }
    let mut drop_choice = false;
    if let Some(choice) = object.get_mut("tool_choice") {
        drop_choice = choice.as_str() == Some("image_generation")
            || choice.get("type").and_then(Value::as_str) == Some("image_generation")
            || (choice.get("type").and_then(Value::as_str) == Some("tool")
                && choice.get("name").and_then(Value::as_str) == Some("image_generation"));
        if choice.get("type").and_then(Value::as_str) == Some("allowed_tools") {
            if let Some(tools) = choice.get_mut("tools").and_then(Value::as_array_mut) {
                tools.retain(|tool| {
                    tool.get("type").and_then(Value::as_str) != Some("image_generation")
                });
                drop_choice |= tools.is_empty();
            }
        }
    }
    if drop_choice {
        object.remove("tool_choice");
    }
    if let Some(input) = object.get_mut("input").and_then(Value::as_array_mut) {
        for item in input
            .iter_mut()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("additional_tools"))
        {
            native_strip_image_tools(item);
        }
        input.retain(|item| {
            item.get("type").and_then(Value::as_str) != Some("additional_tools")
                || item
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_none_or(|tools| !tools.is_empty())
        });
    }
    if let Some(response) = object.get_mut("response") {
        native_strip_image_tools(response);
    }
    if body.get("tool_choice").and_then(Value::as_str) == Some("required")
        && !native_tool_groups(body).into_iter().any(|group| {
            group
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| !tools.is_empty())
        })
    {
        body.as_object_mut().unwrap().remove("tool_choice");
    }
}

fn native_image_source_allowed(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    model: &str,
) -> bool {
    if native_is_chatgpt_plan_account(account) {
        return false;
    }
    let policy = collection
        .image_generation_account_policies
        .get(&account.id)
        .copied()
        .unwrap_or_default();
    if policy == CodexLocalAccessImageGenerationPolicy::Disabled {
        return false;
    }
    if account.is_api_key_auth()
        && policy != CodexLocalAccessImageGenerationPolicy::Enabled
        && !collection
            .image_generation_account_ids
            .contains(&account.id)
    {
        return false;
    }
    native_image_source_model(collection, account, model).is_some()
}

fn native_image_source_model(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    requested: &str,
) -> Option<String> {
    let requested = native_image_selection_model(collection, requested);
    let selected = collection.image_generation_model.trim();
    if !image_model_matches_selection(requested, selected) {
        return None;
    }
    let configured = collection
        .image_generation_account_models
        .get(&account.id)
        .map(String::as_str);
    let explicit = if requested == selected {
        configured
    } else {
        Some(requested)
    };
    let actual = resolve_image_generation_source_model(account, collection, explicit)?;
    // An OAuth source cannot serve a suffixed API-key-only model.
    if !account.is_api_key_auth() && requested != actual {
        return None;
    }
    Some(actual)
}

fn native_image_selection_model<'a>(
    collection: &'a CodexLocalAccessCollection,
    requested: &'a str,
) -> &'a str {
    // Go's Images relay uses the saved model for either bare GPT image name.
    // Explicit variants and custom IDs retain their existing exact matching.
    match requested {
        "gpt-image-2" | "gpt-image-2.5" => collection.image_generation_model.trim(),
        _ => requested,
    }
}

#[cfg(test)]
mod native_image_policy_tests {
    use super::*;
    include!("codex_local_access_tests_native_image_models.rs");
    include!("codex_local_access_tests_native_image_parity22.rs");
    fn collection() -> CodexLocalAccessCollection {
        serde_json::from_value(json!({"enabled":true,"port":1455,"apiKey":"fixture","accountIds":["text"],"createdAt":1,"updatedAt":2,"imageGenerationModel":"gpt-image-2.5"})).unwrap()
    }
    fn account() -> CodexAccount {
        CodexAccount::new(
            "image".into(),
            "fixture@example.invalid".into(),
            crate::models::codex::CodexTokens {
                access_token: "access".into(),
                refresh_token: Some("refresh".into()),
                id_token: "id".into(),
            },
        )
    }
    #[test]
    fn image_limit_is_per_account_and_recovers_when_request_drops() {
        let host = NativeHostBridge::default();
        let first = native_acquire_image_permit(&host, "account-a", 1).unwrap();
        assert_eq!(
            native_acquire_image_permit(&host, "account-a", 1)
                .unwrap_err()
                .status,
            429
        );
        assert!(native_acquire_image_permit(&host, "account-b", 1).is_ok());
        drop(first);
        assert!(native_acquire_image_permit(&host, "account-a", 1).is_ok());
        assert!(native_acquire_image_permit(&host, "account-a", 0)
            .unwrap()
            .is_none());
    }
    #[tokio::test]
    async fn image_slot_waits_for_release_and_selects_available_account() {
        let host = NativeHostBridge::default();
        let mut a = account();
        a.id = "account-a".into();
        let mut b = account();
        b.id = "account-b".into();
        let accounts = vec![a, b];
        let held_a = native_acquire_image_permit(&host, "account-a", 1).unwrap();
        let (index, held_b) = native_wait_image_source_permit(&host, &accounts, 1)
            .await
            .unwrap();
        assert_eq!(index, 1);
        let waiting = native_wait_image_source_permit(&host, &accounts, 1);
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        drop(held_a);
        let (index, permit) = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(index, 0);
        drop((permit, held_b));
    }
    #[tokio::test]
    async fn image_selection_checks_account_gate_and_pinned_wait_never_changes_account() {
        for image_limit in [0, 1] {
            let host = NativeHostBridge::default();
            let mut config = collection();
            config.max_concurrent_image_requests = image_limit;
            config.max_account_concurrency = 1;
            config.account_concurrency_wait_ms = 0;
            let mut a = account();
            a.id = "a".into();
            let mut b = account();
            b.id = "b".into();
            let accounts = vec![a.clone(), b];
            let (_, busy) =
                native_acquire_account_permit(&host, &config, std::slice::from_ref(&a), true)
                    .await
                    .unwrap();
            let (index, account_permit, image_permit) =
                native_acquire_image_source_permits(&host, &config, &accounts)
                    .await
                    .unwrap();
            assert_eq!(index, 1);
            drop((account_permit, image_permit));
            assert_eq!(
                native_acquire_image_source_permits(&host, &config, std::slice::from_ref(&a))
                    .await
                    .err()
                    .unwrap()
                    .code,
                "account_busy"
            );
            config.account_concurrency_wait_ms = 1000;
            let pinned = [a];
            let waiting = native_acquire_image_source_permits(&host, &config, &pinned);
            tokio::pin!(waiting);
            assert!(
                tokio::time::timeout(Duration::from_millis(10), &mut waiting)
                    .await
                    .is_err()
            );
            drop(busy);
            let (index, _, _) = tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(index, 0);
        }
    }
    #[test]
    fn body_policy_preserves_text_and_function_tools_in_images_only_mode() {
        let mut collection = collection();
        let account = account();
        collection.image_generation_mode = CodexLocalAccessImageGenerationMode::ImagesOnly;
        let body = json!({"input":"hello","tools":[{"type":"image_generation"},{"type":"function","name":"image_generation"}],"tool_choice":{"type":"image_generation"}});
        let text = native_prepare_image_body(
            &collection,
            &account,
            &Default::default(),
            body.clone(),
            false,
            false,
        );
        assert_eq!(text["input"], "hello");
        assert_eq!(text["tools"].as_array().unwrap().len(), 1);
        assert_eq!(text["tools"][0]["type"], "function");
        assert!(text.get("tool_choice").is_none());
        assert_eq!(
            native_prepare_image_body(
                &collection,
                &account,
                &Default::default(),
                body.clone(),
                true,
                false
            ),
            body
        );
    }
    #[test]
    fn actual_image_model_cannot_bypass_key_or_account_exclusions() {
        let mut collection = collection();
        let mut account = account();
        account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
        account.openai_api_key = Some("fixture".into());
        account.api_base_url = Some("https://images.example.com/v1".into());
        account.api_model_catalog = vec!["gpt-image-2.5-flare".into()];
        collection.image_generation_account_ids = vec![account.id.clone()];
        collection
            .image_generation_account_models
            .insert(account.id.clone(), "gpt-image-2.5-flare".into());
        assert!(native_image_model_allowed(
            &collection,
            None,
            &account,
            "gpt-image-2.5"
        ));
        collection.excluded_models = vec!["*-flare".into()];
        assert!(!native_image_model_allowed(
            &collection,
            None,
            &account,
            "gpt-image-2.5"
        ));
        collection.excluded_models.clear();
        let key: CodexLocalAccessApiKey = serde_json::from_value(
            json!({"id":"k","label":"k","key":"k","excludedModels":["*-flare"]}),
        )
        .unwrap();
        assert!(!native_image_model_allowed(
            &collection,
            Some(&key),
            &account,
            "gpt-image-2.5"
        ));
    }

    #[test]
    fn image_tool_and_server_default_keep_actual_model_ids_under_namespaced_keys() {
        let collection = collection();
        let mut key: CodexLocalAccessApiKey =
            serde_json::from_value(json!({"id":"k","label":"k","key":"k","modelPrefix":"team"}))
                .unwrap();
        let mut request = native_contract::ImageRouteRequest {
            model: None,
            responses_model: None,
            edit: false,
            target_account_id: None,
            force_refresh: false,
            preferred_account_id: None,
            excluded_account_ids: Vec::new(),
        };
        assert_eq!(
            native_image_requested_model(&collection, Some(&key), &request).unwrap(),
            "gpt-image-2.5"
        );
        request.model = Some("gpt-image-2.5".into());
        assert!(native_image_requested_model(&collection, Some(&key), &request).is_err());
        request.responses_model = Some("team/gpt-5".into());
        assert_eq!(
            native_image_requested_model(&collection, Some(&key), &request).unwrap(),
            "gpt-image-2.5"
        );
        key.excluded_models = vec!["gpt-image-*".into()];
        assert!(native_image_requested_model(&collection, Some(&key), &request).is_err());
    }
    #[test]
    fn hosted_image_auto_injection_obeys_compact_model_headers_and_account_policy() {
        let mut collection = collection();
        let mut account = account();
        let empty = axum::http::HeaderMap::new();
        let body = json!({"model":"gpt-5.5","input":"hello"});
        let ready =
            native_prepare_image_body(&collection, &account, &empty, body.clone(), false, false);
        assert_eq!(ready["tools"][0]["type"], "image_generation");
        let twice = native_prepare_image_body(&collection, &account, &empty, ready, false, false);
        assert_eq!(twice["tools"].as_array().unwrap().len(), 1);
        assert!(native_prepare_image_body(
            &collection,
            &account,
            &empty,
            body.clone(),
            false,
            true
        )
        .get("tools")
        .is_none());
        for model in ["gpt-reserve", "gpt-5.5-spark"] {
            let mut request = body.clone();
            request["model"] = json!(model);
            assert!(native_prepare_image_body(
                &collection,
                &account,
                &empty,
                request,
                false,
                false
            )
            .get("tools")
            .is_none());
        }
        for mode in [
            "chat",
            "images_only",
            "images-only",
            "true",
            "all",
            "disabled",
        ] {
            let mut headers = empty.clone();
            headers.insert("x-agtools-disable-image-generation", mode.parse().unwrap());
            assert!(
                native_prepare_image_body(
                    &collection,
                    &account,
                    &headers,
                    body.clone(),
                    false,
                    false
                )
                .get("tools")
                .is_none(),
                "{mode}"
            );
        }
        let mut headers = empty.clone();
        headers.insert(
            "x-openai-internal-codex-responses-lite",
            "true".parse().unwrap(),
        );
        assert!(native_prepare_image_body(
            &collection,
            &account,
            &headers,
            body.clone(),
            false,
            false
        )
        .get("tools")
        .is_none());
        account.plan_type = Some("free".into());
        assert!(native_prepare_image_body(
            &collection,
            &account,
            &empty,
            body.clone(),
            false,
            false
        )
        .get("tools")
        .is_none());
        account.plan_type = None;
        account.auth_mode = crate::models::codex::CodexAuthMode::Apikey;
        collection.image_generation_account_policies.insert(
            account.id.clone(),
            CodexLocalAccessImageGenerationPolicy::Enabled,
        );
        assert!(native_prepare_image_body(
            &collection,
            &account,
            &empty,
            body.clone(),
            false,
            false
        )
        .get("tools")
        .is_none());
        account = self::account();
        let mut conflict = body;
        conflict["tools"] =
            json!([{"type":"image_generation"},{"type":"function","name":"image_gen.imagegen"}]);
        let result =
            native_prepare_image_body(&collection, &account, &empty, conflict, false, false);
        assert_eq!(result["tools"].as_array().unwrap().len(), 1);
        assert_eq!(result["tools"][0]["type"], "function");
    }
    #[test]
    fn native_tool_filter_preserves_custom_declarations_and_valid_choices() {
        let mut collection = collection();
        let account = account();
        collection.image_generation_mode = CodexLocalAccessImageGenerationMode::ImagesOnly;
        let body = json!({"model":"gpt-5.5","input":[{"role":"user","content":"image_generation stays text"},{"type":"additional_tools","tools":[{"type":"image_generation"},{"type":"function","name":"custom_image"}]}],"tool_choice":{"type":"allowed_tools","mode":"required","tools":[{"type":"image_generation"},{"type":"function","name":"custom_image"}]}});
        let result = native_prepare_image_body(
            &collection,
            &account,
            &Default::default(),
            body,
            false,
            false,
        );
        assert_eq!(result["input"][0]["content"], "image_generation stays text");
        assert_eq!(
            result["input"][1]["tools"],
            json!([{"type":"function","name":"custom_image"}])
        );
        assert_eq!(
            result["tool_choice"]["tools"],
            json!([{"type":"function","name":"custom_image"}])
        );
        let body = json!({"model":"gpt-5.5","tools":[{"type":"image_generation"}],"tool_choice":"required"});
        let result = native_prepare_image_body(
            &collection,
            &account,
            &Default::default(),
            body,
            false,
            false,
        );
        assert!(result.get("tool_choice").is_none());
    }
}
