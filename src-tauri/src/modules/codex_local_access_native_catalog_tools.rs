// Share routing-derived capabilities between /models and managed on-disk catalogs.
fn native_apply_catalog_route_contract(
    model: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    accounts: &[CodexAccount],
) {
    let requires_mapping = native_plan_requires_account_mapping(plan, accounts);
    let mut routes = accounts
        .iter()
        .filter(|account| {
            (!requires_mapping || native_account_mapped_model(account, &plan.model).is_some())
                && native_model_visible(collection, key, plan, account)
        })
        .peekable();
    let mut supports_lite = true;
    let all_official = plan.gateway.is_none()
        && routes.peek().is_some()
        && routes.all(|account| {
            if account.is_api_key_auth() || native_is_chatgpt_plan_account(account) {
                return false;
            }
            supports_lite &= native_plan_account_model(plan, account)
                .as_deref()
                .is_some_and(codex_protocol::codex_model_uses_responses_lite);
            true
        });
    if !all_official {
        codex_protocol::project_public_responses_model(model);
        return;
    }
    model["use_responses_lite"] = json!(supports_lite);
    // A managed profile may still contain the projection from a previous relay.
    if model["tool_mode"] == "direct"
        && model["slug"]
            .as_str()
            .and_then(codex_protocol::known_codex_model_template)
            .is_some_and(|template| template["tool_mode"] == "code_mode_only")
    {
        model["tool_mode"] = json!("code_mode_only");
    }
}

fn native_apply_catalog_tool_capabilities(
    model: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    accounts: &[CodexAccount],
) {
    native_apply_catalog_route_contract(model, collection, key, plan, accounts);
    if model["visibility"] != "hide" {
        model["multi_agent_version"] = json!("v2");
    }
    // A model can be scheduled on any eligible pool member, not merely the
    // first account used to supply display metadata. Plain Chat has no native
    // tool-search history bridge; Anthropic, SIWC and Grok have client adapters.
    if accounts
        .iter()
        .filter(|account| native_model_visible(collection, key, plan, account))
        .any(|account| {
            native_wire_api(account).map_or(true, |wire| wire == native_contract::WireApi::Chat)
        })
    {
        model["supports_search_tool"] = json!(false);
    } else if collection.gateway_mode == CodexLocalAccessGatewayMode::Native
        && accounts.iter().any(|account| {
            native_model_visible(collection, key, plan, account)
                && native_wire_api(account).ok() == Some(native_contract::WireApi::Anthropic)
        })
    {
        model["supports_search_tool"] = json!(true);
    }
}

fn apply_provider_catalog_image_input(model: &mut Value, vision: bool) {
    model["input_modalities"] = if vision {
        json!(["text", "image"])
    } else {
        json!(["text"])
    };
    // Accepting image input does not imply support for Codex's original-detail flag.
    model["supports_image_detail_original"] = json!(false);
}

fn native_catalog_remote_templates(accounts: &[CodexAccount]) -> HashMap<(String, String), Value> {
    accounts
        .iter()
        .filter_map(|account| {
            remote_catalog_for_account(account).map(|catalog| (&account.id, catalog))
        })
        .flat_map(|(id, mut catalog)| {
            catalog["models"]
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |entry| {
                    let slug = entry["slug"].as_str()?.to_owned();
                    Some(((id.clone(), slug), entry))
                })
        })
        .collect()
}

fn native_apply_oauth_mapping_metadata(
    model: &mut Value,
    account: &CodexAccount,
    requested: &str,
    upstream: &str,
) {
    if !native_is_oauth_authorization_account(account)
        || native_account_mapped_model(account, requested).is_none()
    {
        return;
    }
    let Some(template) = codex_protocol::known_codex_model_template(upstream) else { return };
    for field in [
        "input_modalities",
        "supports_image_detail_original",
        "context_window",
        "max_context_window",
        "auto_compact_token_limit",
        "supported_reasoning_levels",
        "default_reasoning_level",
    ] {
        if let Some(value) = template.get(field) {
            model[field] = value.clone();
        }
    }
}

fn native_apply_catalog_account_context(
    model: &mut Value,
    account: &CodexAccount,
    requested: &str,
    upstream: &str,
) {
    let windows = &account.api_model_context_windows;
    let lookup = |name: &str| windows.get(name).copied().or_else(|| {
        windows.iter().find_map(|(id, window)| id.eq_ignore_ascii_case(name).then_some(*window))
    }).filter(|window| *window > 0);
    let requested_window = if native_is_oauth_authorization_account(account) {
        lookup(requested)
    } else {
        None
    };
    let Some(window) = requested_window.or_else(|| lookup(upstream))
    else {
        return;
    };
    model["context_window"] = json!(window);
    model["max_context_window"] = json!(window);
    model["auto_compact_token_limit"] =
        json!(codex_protocol::derived_auto_compact_token_limit(window));
}

fn native_apply_catalog_route_metadata(
    model: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    accounts: &[CodexAccount],
    remote_templates: &HashMap<(String, String), Value>,
) {
    let Some(account) = native_catalog_model_account(collection, key, plan, accounts)
        .filter(|account| account.is_api_key_auth()
            || native_account_mapped_model(account, &plan.model).is_some())
    else {
        return;
    };
    let Some(upstream) = native_plan_account_model(plan, account) else {
        return;
    };
    if account.is_api_key_auth() {
        apply_provider_catalog_image_input(
            model,
            native_catalog_vision(collection, plan, account, &upstream),
        );
    }
    // Known remote services expose their own protocol-level capabilities, just
    // as they do in native_catalog. This uses the existing local metadata cache.
    native_apply_oauth_mapping_metadata(model, account, &plan.model, &upstream);
    if let Some(template) = remote_templates.get(&(account.id.clone(), upstream.clone())) {
        for field in [
            "input_modalities", "supports_image_detail_original", "context_window",
            "max_context_window", "auto_compact_token_limit",
        ] {
            if let Some(value) = template.get(field) {
                model[field] = value.clone();
            }
        }
    }
    native_apply_catalog_account_context(model, account, &plan.model, &upstream);
}

fn native_apply_profile_catalog_capabilities(
    catalog: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    accounts: &[CodexAccount],
) {
    let remote_templates = native_catalog_remote_templates(accounts);
    codex_protocol::project_public_responses_catalog(catalog);
    if let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) {
        for model in models {
            let Some(slug) = model["slug"].as_str() else {
                continue;
            };
            let Ok(public) = native_model_for_key(collection, key, slug) else {
                continue;
            };
            let Ok(plan) = native_model_route_plan(collection, key, &public) else {
                continue;
            };
            native_apply_catalog_tool_capabilities(model, collection, key, &plan, accounts);
            native_apply_catalog_route_metadata(
                model,
                collection,
                key,
                &plan,
                accounts,
                &remote_templates,
            );
        }
    }
}

fn profile_catalog_account_ids(
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
) -> Vec<String> {
    let mut ids = native_scoped_account_ids(collection, key);
    if let Some(routing) = key.and_then(|key| key.model_routing.as_ref()) {
        ids.extend(
            routing
                .routes
                .iter()
                .map(|route| route.provider_account_id.clone()),
        );
    }
    let mut seen = HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

/// Apply the actual key's route capabilities when the user saves model management
/// or the versioned catalog is rebuilt. Never borrow another profile/key's scope.
/// Runs in the existing blocking catalog writer, without starting a gateway.
pub(crate) fn apply_attached_catalog_route_capabilities(
    doc: &Document,
    catalog: &mut Value,
) -> Result<(), String> {
    let provider_id = doc.get("model_provider").and_then(|item| item.as_str()).map(str::trim);
    if provider_id != Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID) {
        let openai_base_url_override = doc
            .get("openai_base_url")
            .and_then(|item| item.as_str())
            .is_some_and(|url| !url.trim().is_empty());
        if openai_base_url_override || provider_id.is_some_and(|id| !id.is_empty() && id != "openai") {
            codex_protocol::project_public_responses_catalog(catalog);
        }
        return Ok(());
    }
    // The active scoped route may establish support below. Without a matching
    // listener and key, do not inherit a private protocol from a model slug.
    codex_protocol::project_public_responses_catalog(catalog);
    let Some(provider) = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
        .and_then(|provider| provider.as_table())
    else {
        return Ok(());
    };
    let Some(token) = provider
        .get("experimental_bearer_token")
        .and_then(|item| item.as_str())
    else {
        return Ok(());
    };
    let Some(content) = read_optional_profile_file(&local_access_file_path()?)? else {
        return Ok(());
    };
    let collection: CodexLocalAccessCollection =
        serde_json::from_str(&content).map_err(|error| error.to_string())?;
    if !profile_base_url_matches(
        provider.get("base_url").and_then(|item| item.as_str()),
        &build_collection_base_url(&collection),
    ) {
        return Ok(());
    }
    let Some(resolved_key) = resolve_collection_api_key(&collection, token) else {
        return Ok(());
    };
    let key = collection
        .api_keys
        .iter()
        .find(|key| key.enabled && key.key == token.trim());
    let accounts = profile_catalog_account_ids(&collection, key)
        .iter()
        .filter_map(|id| codex_account::load_account(id))
        .collect::<Vec<_>>();
    if collection.gateway_mode == CodexLocalAccessGatewayMode::Native {
        let remote_templates = native_catalog_remote_templates(&accounts);
        if let Some(models) = catalog["models"].as_array_mut() {
            for model in models {
                let Some(slug) = model["slug"].as_str() else {
                    continue;
                };
                let Ok(public) = native_model_for_key(&collection, key, slug) else {
                    continue;
                };
                let Ok(plan) = native_model_route_plan(&collection, key, &public) else {
                    continue;
                };
                native_apply_catalog_route_contract(model, &collection, key, &plan, &accounts);
                native_apply_catalog_route_metadata(
                    model,
                    &collection,
                    key,
                    &plan,
                    &accounts,
                    &remote_templates,
                );
            }
        }
    } else {
        let capabilities =
            automatic_api_service_profile_extra_models(&collection, &resolved_key, &accounts)
                .into_iter()
                .map(|(model, vision)| (model.to_ascii_lowercase(), vision))
                .collect::<HashMap<_, _>>();
        if let Some(models) = catalog["models"].as_array_mut() {
            for model in models {
                if let Some(vision) = model["slug"]
                    .as_str()
                    .and_then(|id| capabilities.get(&id.to_ascii_lowercase()))
                {
                    apply_provider_catalog_image_input(model, *vision);
                }
            }
        }
    }
    Ok(())
}

/// Runs after existing catalog/experimental writers. Reading credentials uses
/// the established bounded worker pool; this path does not start a listener or
/// call any upstream service. The actual file update stays on a blocking worker.
async fn native_sync_profile_catalog_tool_capabilities(
    profile: &Path,
    collection: &CodexLocalAccessCollection,
    api_key: &str,
    known_accounts: &[CodexAccount],
) -> Result<(), String> {
    if collection.gateway_mode != CodexLocalAccessGatewayMode::Native {
        return Ok(());
    }
    let key = collection
        .api_keys
        .iter()
        .find(|key| key.enabled && key.key.trim() == api_key.trim());
    let mut ids = profile_catalog_account_ids(collection, key);
    ids.retain(|id| !known_accounts.iter().any(|account| &account.id == id));
    let mut accounts = timeout(Duration::from_secs(30), native_load_accounts(ids))
        .await
        .map_err(|_| "NATIVE_CATALOG_ACCOUNTS_TIMEOUT".to_string())?
        .map_err(|error| error.message)?;
    accounts.extend_from_slice(known_accounts);
    let collection = collection.clone();
    let key = key.cloned();
    let path = profile.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    let profile = profile.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if !path.exists() {
            return Ok(());
        }
        let profile_models = codex_account::read_enabled_profile_model_definitions(&profile);
        let mut changed = false;
        update_string_atomic(&path, |current| {
            let content = current.ok_or_else(|| "NATIVE_CATALOG_FILE_CHANGED".to_string())?;
            let mut catalog: Value =
                serde_json::from_str(content).map_err(|error| error.to_string())?;
            let original = catalog.clone();
            native_apply_profile_catalog_capabilities(
                &mut catalog,
                &collection,
                key.as_ref(),
                &accounts,
            );
            codex_account::apply_model_context_config_to_catalog(&mut catalog, &profile_models);
            changed = catalog != original;
            if changed {
                serde_json::to_string_pretty(&catalog).map_err(|error| error.to_string())
            } else {
                Ok(content.to_owned())
            }
        })?;
        if changed {
            invalidate_codex_model_cache(&profile)?;
            crate::modules::codex_managed_model_catalog_version::write_managed_catalog_meta(&path)?;
        }
        Ok(())
    })
    .await
    .map_err(|_| "NATIVE_CATALOG_WRITE_FAILED".to_string())?
}

#[cfg(test)]
#[path = "codex_local_access_native_catalog_tools_tests.rs"]
mod native_catalog_tools_tests;
