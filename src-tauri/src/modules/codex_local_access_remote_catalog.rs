const REMOTE_MODEL_FIELDS: &[&str] = &[
    "display_name",
    "description",
    "priority",
    "visibility",
    "supported_reasoning_levels",
    "default_reasoning_level",
    "context_window",
    "max_context_window",
    "auto_compact_token_limit",
    "input_modalities",
    "supports_reasoning_summaries",
    "support_verbosity",
    "default_verbosity",
    "supports_parallel_tool_calls",
    "web_search_tool_type",
    "supports_image_detail_original",
    "supported_in_api",
    "prefer_websockets",
    "additional_speed_tiers",
    "service_tiers",
    "multi_agent_version",
    "supports_search_tool",
];
// One catalog builder for local API profiles and the authenticated remote view.
// Only public metadata crosses the boundary. No instructions, tool code, paths,
// upstream credentials or arbitrary renderer payloads are exported.
fn remote_client_model_catalog(
    collection: &CodexLocalAccessCollection,
    key: &str,
    allowed: &[String],
    accounts: &[CodexAccount],
) -> Result<Value, String> {
    if allowed.is_empty() {
        return Ok(json!({"models":[]}));
    }
    let definitions = local_access_profile_model_definitions_with_accounts(
        &codex_account::get_codex_home(),
        collection,
        key,
        true,
        accounts,
    )?;
    let mut catalog = build_local_access_profile_catalog(&definitions);
    let resolved_key = collection
        .api_keys
        .iter()
        .find(|entry| entry.key.trim() == key.trim());
    native_apply_profile_catalog_tool_capabilities(
        &mut catalog,
        collection,
        resolved_key,
        accounts,
    );
    let prefix = resolve_collection_api_key(collection, key)
        .and_then(|k| k.model_prefix.clone())
        .unwrap_or_default();
    Ok(project_public_model_catalog(&catalog, allowed, &prefix))
}

fn project_public_model_catalog(catalog: &Value, allowed: &[String], prefix: &str) -> Value {
    let prefix = prefix.trim_matches('/');
    let allowed_set: HashSet<&str> = allowed.iter().map(String::as_str).collect();
    let mut exported = HashSet::new();
    let mut models = Vec::new();
    for entry in catalog["models"].as_array().into_iter().flatten() {
        let Some(slug) = entry["slug"].as_str() else {
            continue;
        };
        let exposed = if prefix.is_empty() {
            slug.to_string()
        } else {
            format!("{prefix}/{slug}")
        };
        if !allowed_set.contains(exposed.as_str()) || !exported.insert(exposed.clone()) {
            continue;
        }
        let mut public = serde_json::Map::new();
        public.insert("slug".into(), json!(exposed));
        for name in REMOTE_MODEL_FIELDS {
            if let Some(value) = entry.get(*name) {
                public.insert((*name).into(), value.clone());
            }
        }
        models.push(Value::Object(public));
    }
    // Custom exposed models absent from the local recommended set retain their
    // protocol template; never add unauthorized or standalone image models.
    for id in allowed {
        if exported.contains(id)
            || id
                .rsplit('/')
                .next()
                .is_some_and(|id| id.starts_with("gpt-image-"))
        {
            continue;
        }
        // Model IDs are public; fallback templates contain no account state.
        let fallback = codex_protocol::build_codex_client_models_response(&[id.clone()]);
        if let Some(model) = fallback["models"].as_array().and_then(|m| m.first()) {
            let mut value = serde_json::Map::new();
            value.insert("slug".into(), json!(id));
            for field in REMOTE_MODEL_FIELDS {
                if let Some(v) = model.get(*field) {
                    value.insert((*field).into(), v.clone());
                }
            }
            models.push(Value::Object(value));
        }
    }
    models.sort_by_key(|model| model["priority"].as_i64().unwrap_or(i64::MAX));
    json!({"models":models})
}

pub(crate) fn remote_catalog_for_account(account: &CodexAccount) -> Option<Value> {
    let remote = crate::modules::codex_remote_service::for_account(account)?;
    project_remote_model_catalog(&remote)
}

// Use the advertised ID set as the authorization boundary. Templates only add
// local protocol defaults; remote metadata cannot introduce additional models.
fn project_remote_model_catalog(
    remote: &crate::modules::codex_remote_service::Connection,
) -> Option<Value> {
    let source = remote.model_catalog.as_ref()?;
    let allowed: HashSet<&str> = remote.models.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let entries = source["models"]
        .as_array()?
        .iter()
        .filter(|model| {
            model["slug"].as_str().is_some_and(|id| {
                allowed.contains(id)
                    && !remote.image_models.iter().any(|image| image == id)
                    && seen.insert(id)
            })
        })
        .collect::<Vec<_>>();
    let ids = entries
        .iter()
        .filter_map(|m| m["slug"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut catalog = codex_protocol::build_codex_client_models_response(&ids);
    if let Some(models) = catalog["models"].as_array_mut() {
        for (model, entry) in models.iter_mut().zip(entries) {
            for field in REMOTE_MODEL_FIELDS {
                if let Some(value) = entry.get(*field) {
                    model[*field] = value.clone();
                }
            }
        }
        models.sort_by_key(|model| model["priority"].as_i64().unwrap_or(i64::MAX));
    }
    Some(catalog)
}

#[cfg(test)]
#[path = "codex_local_access_remote_catalog_tests.rs"]
mod remote_catalog_tests;
