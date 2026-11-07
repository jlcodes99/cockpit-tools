// Multi-agent support is built in; only the selected wire protocol controls projection.
fn native_multi_agent_profiles(catalog: &Value) -> Vec<native_contract::ModelProfile> {
    let mut entries: Vec<_> = catalog["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|metadata| {
            let id = metadata["slug"].as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let template = codex_protocol::known_codex_model_template(id);
            let mut profile = native_contract::ModelProfile::from_metadata(id, metadata);
            let known = template.is_some();
            let public = catalog["data"]
                .as_array()
                .and_then(|entries| entries.iter().find(|entry| entry["id"] == id));
            if !known {
                profile.service_tiers.clear();
                // The Codex catalog may synthesize unknown models from a GPT
                // template. Its description is not the custom model's identity.
                profile.description = public
                    .and_then(|entry| entry["description"].as_str())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(id)
                    .trim()
                    .to_owned();
                let efforts = profile.reasoning_efforts.clone();
                profile.apply_thinking(&efforts);
            }
            let priority = template
                .as_ref()
                .and_then(|value| value["priority"].as_i64())
                .unwrap_or_default();
            let display = if known {
                metadata["display_name"].as_str()
            } else {
                public.and_then(|entry| entry["display_name"].as_str())
            }
            .unwrap_or(id)
            .to_lowercase();
            Some((known, priority, display, profile))
        })
        .collect();
    entries.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            if a.0 {
                a.1.cmp(&b.1).then(a.3.id.cmp(&b.3.id))
            } else {
                a.2.cmp(&b.2).then(a.3.id.cmp(&b.3.id))
            }
        })
    });
    entries.into_iter().map(|entry| entry.3).collect()
}

async fn native_multi_agent_policy(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    route: Option<&native_contract::Route>,
    _requested_model: &str,
) -> Result<native_contract::MultiAgentPolicy, native_contract::ServiceError> {
    let collection = host.collection().await?;
    native_caller_key(&collection, caller)?;
    // Route selection already checked account grants. Public provider protocols
    // need portable messages; official Codex/Plan routes keep their own adapters
    // and opaque reasoning/signature metadata. Legacy switches and model lists
    // are deliberately not consulted.
    let options = native_contract::MultiAgentOptions {
        optimize: true,
        orphan_delegation: true,
        is_compat: route.is_some_and(|route| !route.chatgpt && !route.chatgpt_plan),
        ..Default::default()
    };
    let models = if route.is_none() {
        native_multi_agent_profiles(&native_models(host, caller).await?)
    } else {
        Vec::new()
    };
    Ok(native_contract::MultiAgentPolicy { options, models })
}

#[cfg(test)]
#[path = "codex_local_access_native_multi_agent_tests.rs"]
mod native_multi_agent_tests;
