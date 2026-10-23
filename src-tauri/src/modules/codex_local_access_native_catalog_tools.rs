// Share tool capability gates between /models and managed on-disk catalogs.
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
    // tool-search history bridge; SIWC and Grok have explicit client adapters.
    if accounts
        .iter()
        .filter(|account| native_model_visible(collection, key, plan, account))
        .any(|account| {
            native_wire_api(account).map_or(true, |wire| wire == native_contract::WireApi::Chat)
        })
    {
        model["supports_search_tool"] = json!(false);
    }
}

fn native_apply_profile_catalog_tool_capabilities(
    catalog: &mut Value,
    collection: &CodexLocalAccessCollection,
    key: Option<&CodexLocalAccessApiKey>,
    accounts: &[CodexAccount],
) {
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
        }
    }
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
    let mut ids = native_scoped_account_ids(collection, key);
    if let Some(routing) = key.and_then(|key| key.model_routing.as_ref()) {
        ids.extend(
            routing
                .routes
                .iter()
                .map(|route| route.provider_account_id.clone()),
        );
    }
    ids.sort();
    ids.dedup();
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
            native_apply_profile_catalog_tool_capabilities(
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
        }
        Ok(())
    })
    .await
    .map_err(|_| "NATIVE_CATALOG_WRITE_FAILED".to_string())?
}

#[cfg(test)]
#[path = "codex_local_access_native_catalog_tools_tests.rs"]
mod native_catalog_tools_tests;
