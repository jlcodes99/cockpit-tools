// Public protocol catalogs expose known capabilities without bootstrap guesses.
fn native_public_model_entry(
    collection: &CodexLocalAccessCollection,
    plan: &NativeModelRoutePlan,
    account: &CodexAccount,
    id: &str,
    upstream: &str,
) -> Value {
    let template = account_model_template(account, upstream)
        .or_else(|| codex_protocol::known_codex_model_template(upstream));
    let window = account
        .api_model_context_windows
        .get(upstream)
        .copied()
        .filter(|n| *n > 0);
    let vision = native_catalog_vision(collection, plan, account, upstream);
    let tools = !account.is_api_key_auth() || template.is_some();
    native_model_entry_from_metadata(id, template.as_ref(), window, vision, tools)
}

fn native_model_entry_from_metadata(
    id: &str,
    template: Option<&Value>,
    window: Option<i64>,
    vision: bool,
    tools: bool,
) -> Value {
    let mut entry = json!({"id":id,"object":"model","created":0,"owned_by":"configured-upstream"});
    let mut capabilities = vec!["completion"];
    if tools {
        capabilities.push("tools");
    }
    if vision {
        capabilities.push("vision");
    }
    let levels = template
        .and_then(|template| template["supported_reasoning_levels"].as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().or_else(|| item["effort"].as_str()))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if levels.iter().any(|level| level != "none") {
        capabilities.push("thinking");
    }
    entry["capabilities"] = json!(capabilities);
    entry["model_info"] = json!({"upstream_id":id,"supported_reasoning_efforts":levels,"input_modalities":if vision {vec!["text","image"]} else {vec!["text"]}});
    if let Some(template) = template {
        for field in ["display_name", "description"] {
            if let Some(value) = template.get(field) {
                entry[field] = value.clone();
            }
        }
        if let Some(effort) = template.get("default_reasoning_level") {
            entry["model_info"]["default_reasoning_effort"] = effort.clone();
        }
    }
    if let Some(window) = window.or_else(|| {
        template
            .and_then(|template| template["context_window"].as_i64())
            .filter(|n| *n > 0)
    }) {
        entry["context_length"] = json!(window);
        entry["model_info"]["context_length"] = json!(window);
    }
    entry
}

#[cfg(test)]
mod native_catalog_metadata_tests {
    use super::*;
    #[test]
    fn grok_47_native_catalog_keeps_known_capabilities_and_explicit_vision_permission() {
        let template = codex_protocol::known_codex_model_template("grok-4.7").unwrap();
        for vision in [false, true] {
            let model = native_model_entry_from_metadata(
                "grok-4.7", Some(&template), None, vision, true,
            );
            assert_eq!(model["context_length"], 500_000);
            assert_eq!(model["model_info"]["default_reasoning_effort"], "high");
            assert_eq!(model["model_info"]["supported_reasoning_efforts"], json!(["low", "medium", "high", "xhigh"]));
            let capabilities = model["capabilities"].as_array().unwrap();
            assert!(capabilities.contains(&json!("thinking")));
            assert!(capabilities.contains(&json!("tools")));
            assert_eq!(capabilities.contains(&json!("vision")), vision);
        }
    }
    #[test]
    fn known_capabilities_and_explicit_window_reach_ollama_and_gemini() {
        let template = json!({"display_name":"Known","context_window":9000,"supported_reasoning_levels":[{"effort":"high"}],"default_reasoning_level":"high"});
        let model =
            native_model_entry_from_metadata("known", Some(&template), Some(7000), true, true);
        assert_eq!(model["context_length"], 7000);
        assert_eq!(model["created"], 0);
        let shown = native_api::compat_catalog_show_for_test(&model).unwrap();
        assert!(shown["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("tools")));
        assert!(shown["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("thinking")));
        assert_eq!(shown["model_info"]["context_length"], 7000);
    }
    #[test]
    fn unknown_provider_does_not_advertise_bootstrap_context_or_thinking() {
        let entry = native_model_entry_from_metadata("custom", None, None, false, false);
        assert!(entry.get("context_length").is_none());
        assert_eq!(entry["capabilities"], json!(["completion"]));
    }
}
