// Retired built-in selections are distinct from user-supplied provider models.
// API-key providers can still legitimately expose this model through their API.
pub(crate) fn is_retired_gpt55_model(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model == "gpt-5.5" || model.starts_with("gpt-5.5-") || model.starts_with("gpt-5.5(")
}

fn is_retired_builtin_codex_model(model: &str) -> bool {
    is_retired_gpt55_model(model) || crate::modules::codex_wakeup::is_codex_model_before_5_5(model)
}

fn without_retired_gpt55_models(
    base_dir: &Path,
    mut models: Vec<CodexExperimentalModelDefinition>,
) -> Vec<CodexExperimentalModelDefinition> {
    if !profile_uses_chatgpt_models(base_dir) {
        return models;
    }
    let had_retired = models
        .iter()
        .any(|item| is_retired_gpt55_model(&item.model_id));
    models.retain(|item| !is_retired_gpt55_model(&item.model_id));
    if had_retired && models.is_empty() {
        models.extend(
            default_experimental_model_definitions(base_dir)
                .into_iter()
                .filter(|item| item.model_id == DEFAULT_CODEX_MODEL_ID),
        );
    }
    models
}

fn profile_uses_chatgpt_models(base_dir: &Path) -> bool {
    match fs::read_to_string(get_config_toml_path(base_dir)) {
        Ok(source) => crate::modules::codex_config_format::read_codex_config_doc_from_str(&source)
            .map(|doc| doc_uses_chatgpt_models(&doc))
            .unwrap_or(false),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn migrate_retired_gpt55_catalog(base_dir: &Path) -> Result<bool, String> {
    if !profile_uses_chatgpt_models(base_dir) {
        return Ok(false);
    }
    let Some(config) = read_experimental_model_catalog_config(base_dir) else {
        return Ok(false);
    };
    if config.version < EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION {
        // Preserve the existing migration for older catalog schema versions.
        return Ok(false);
    }
    if !config
        .models
        .iter()
        .any(|item| is_retired_gpt55_model(&item.model_id))
        && !config
            .default_model_id
            .as_deref()
            .is_some_and(is_retired_gpt55_model)
    {
        return Ok(false);
    }
    let models = without_retired_gpt55_models(base_dir, config.models);
    let default = config.default_model_id.as_deref().map(|model| {
        if is_retired_gpt55_model(model) {
            models
                .iter()
                .find(|item| item.model_id == DEFAULT_CODEX_MODEL_ID)
                .or_else(|| models.first())
                .map(|item| item.model_id.as_str())
                .unwrap_or(DEFAULT_CODEX_MODEL_ID)
        } else {
            model
        }
    });
    persist_experimental_model_definitions(base_dir, models.clone(), default)?;
    Ok(true)
}

fn doc_uses_chatgpt_models(doc: &Document) -> bool {
    if doc
        .get(CODEX_CONFIG_FORCED_LOGIN_METHOD_KEY)
        .and_then(|item| item.as_str())
        .is_some_and(|method| method.eq_ignore_ascii_case(CODEX_FORCED_LOGIN_METHOD_API))
    {
        return false;
    }
    if doc
        .get(CODEX_CONFIG_OPENAI_BASE_URL_KEY)
        .and_then(|item| item.as_str())
        .is_some_and(|url| !url.trim().is_empty())
    {
        return false;
    }
    if doc
        .get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|providers| providers.get("openai"))
        .and_then(|provider| provider.get("requires_openai_auth"))
        .and_then(|item| item.as_bool())
        == Some(false)
    {
        return false;
    }
    match doc
        .get(CODEX_CONFIG_MODEL_PROVIDER_KEY)
        .and_then(|item| item.as_str())
    {
        None | Some("openai") | Some("codex_local_access") => true,
        _ => false,
    }
}

fn migrate_retired_chatgpt_default_in_doc(doc: &mut Document) -> bool {
    if !doc_uses_chatgpt_models(doc)
        || !doc
            .get("model")
            .and_then(|item| item.as_str())
            .is_some_and(is_retired_gpt55_model)
    {
        return false;
    }
    doc["model"] = value(DEFAULT_CODEX_MODEL_ID);
    true
}

fn migrate_retired_chatgpt_default_for_dir(base_dir: &Path) -> Result<bool, String> {
    let path = get_config_toml_path(base_dir);
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Read model configuration: {error}")),
    };
    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&source)
        .map_err(|error| format!("Parse model configuration: {error}"))?;
    if !migrate_retired_chatgpt_default_in_doc(&mut doc) {
        return Ok(false);
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    // Invalidate before persisting so a failed cleanup leaves the old selection
    // eligible for retry instead of permanently retaining a stale model cache.
    crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir)?;
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&path, &content)
        .map_err(|error| format!("Write model configuration: {error}"))?;
    Ok(true)
}

#[cfg(test)]
mod retired_model_tests {
    use super::*;

    #[test]
    fn retired_model_identity_does_not_match_provider_names_or_newer_versions() {
        for id in ["gpt-5.5", "GPT-5.5", "gpt-5.5-pro", "gpt-5.5(high)"] {
            assert!(is_retired_gpt55_model(id));
        }
        for id in [
            "gpt-5.50",
            "gpt-5.6-sol",
            "provider/gpt-5.5",
            "custom-gpt-5.5",
        ] {
            assert!(!is_retired_gpt55_model(id));
        }
    }

    #[test]
    fn retired_default_migrates_on_disk_once_without_touching_provider_defaults() {
        let root = tempfile::tempdir().unwrap();
        let path = get_config_toml_path(root.path());
        fs::write(
            &path,
            "# keep comment\nmodel = \"gpt-5.5\"\nmodel_reasoning_effort = \"high\"\n",
        )
        .unwrap();
        let cache = root.path().join("models_cache.json");
        fs::write(&cache, "stale model catalog").unwrap();
        assert!(migrate_retired_chatgpt_default_for_dir(root.path()).unwrap());
        assert!(!cache.exists());
        assert!(!migrate_retired_chatgpt_default_for_dir(root.path()).unwrap());
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("gpt-5.6-sol"));
        assert!(after.contains("# keep comment"));
        assert!(after.contains("\"high\""));
        for extra in [
            "forced_login_method = \"api\"\n",
            "model_provider = \"custom\"\n",
        ] {
            let original = format!("model = \"gpt-5.5\"\n{extra}");
            fs::write(&path, &original).unwrap();
            fs::write(&cache, "provider model catalog").unwrap();
            assert!(!migrate_retired_chatgpt_default_for_dir(root.path()).unwrap());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert_eq!(
                fs::read_to_string(&cache).unwrap(),
                "provider model catalog"
            );
        }
    }

    #[test]
    fn retired_saved_catalog_is_removed_without_resetting_custom_entries() {
        let root = tempfile::tempdir().unwrap();
        let custom = CodexExperimentalModelDefinition {
            model_id: "provider/custom".into(),
            display_name: "Custom".into(),
            reasoning_efforts: None,
            default_reasoning_effort: None,
            context_window: Some(32768),
            auto_compact_token_limit: Some(28000),
        };
        let mut retired = custom.clone();
        retired.model_id = "gpt-5.5".into();
        fs::write(
            experimental_model_config_path(root.path()),
            serde_json::to_vec(&ExperimentalModelCatalogConfig {
                version: EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION,
                models: vec![retired],
                default_model_id: Some("gpt-5.5".into()),
                migrations: vec!["existing-marker".into()],
            })
            .unwrap(),
        )
        .unwrap();
        assert!(migrate_retired_gpt55_catalog(root.path()).unwrap());
        assert!(!migrate_retired_gpt55_catalog(root.path()).unwrap());
        let config = read_experimental_model_catalog_config(root.path()).unwrap();
        assert_eq!(
            config.default_model_id.as_deref(),
            Some(DEFAULT_CODEX_MODEL_ID)
        );
        assert!(config.migrations.contains(&"existing-marker".into()));
        let mut retired = custom.clone();
        retired.model_id = "gpt-5.5".into();
        let kept = persist_experimental_model_definitions(
            root.path(),
            vec![retired, custom.clone()],
            Some("provider/custom"),
        )
        .unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].context_window, custom.context_window);
        assert_eq!(kept[0].model_id, custom.model_id);
    }
}
