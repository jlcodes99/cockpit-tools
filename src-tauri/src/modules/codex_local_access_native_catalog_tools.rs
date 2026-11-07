// Share routing-derived capabilities between /models and managed on-disk catalogs.
fn native_apply_catalog_tool_capabilities(
    model: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    accounts: &[CodexAccount],
) {
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

fn native_apply_catalog_route_image_input(
    model: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    plan: &NativeModelRoutePlan,
    accounts: &[CodexAccount],
    remote_templates: &HashMap<(String, String), Value>,
) {
    let Some(account) = native_catalog_model_account(collection, key, plan, accounts)
        .filter(|account| account.is_api_key_auth())
    else {
        return;
    };
    let Some(upstream) = native_plan_account_model(plan, account) else {
        return;
    };
    apply_provider_catalog_image_input(
        model,
        native_catalog_vision(collection, plan, account, &upstream),
    );
    // Known remote services expose their own protocol-level capabilities, just
    // as they do in native_catalog. This uses the existing local metadata cache.
    if let Some(template) = remote_templates.get(&(account.id.clone(), upstream)) {
        for field in ["input_modalities", "supports_image_detail_original"] {
            if let Some(value) = template.get(field) {
                model[field] = value.clone();
            }
        }
    }
}

fn native_apply_profile_catalog_capabilities(
    catalog: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    accounts: &[CodexAccount],
) {
    let remote_templates = native_catalog_remote_templates(accounts);
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
            native_apply_catalog_route_image_input(
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
pub(crate) fn apply_attached_catalog_image_capabilities(
    doc: &Document,
    catalog: &mut Value,
) -> Result<(), String> {
    if doc.get("model_provider").and_then(|item| item.as_str())
        != Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
    {
        return Ok(());
    }
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
                native_apply_catalog_route_image_input(
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
