// Passive reconciliation must never reacquire a profile the user has edited away.
const TAKEOVER_OWNERSHIP_FILE: &str = ".cockpit_api_takeover.json";

// Only called by the explicit OAuth binding command. Background maintenance
// must never use a full account projection or refresh user credentials.
async fn refresh_owned_takeovers_after_explicit_oauth_binding(
    profiles: Vec<PathBuf>,
    previous: &CodexLocalAccessCollection,
    next: &CodexLocalAccessCollection,
) -> Result<(), String> {
    if !next.enabled {
        return Ok(());
    }
    for profile in profiles {
        let check_profile = profile.clone();
        let previous = previous.clone();
        let owned = tokio::task::spawn_blocking(move || {
            profile_still_owned_for_collection(&check_profile, &previous)
        })
        .await
        .map_err(|error| error.to_string())??;
        if owned {
            write_local_access_profile_takeover(&profile, next, None, true).await?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct TakeoverOwnership {
    connection_hash: String,
}

fn takeover_connection_hash(doc: &Document) -> Option<String> {
    use sha2::Digest;
    if doc.get("model_provider")?.as_str()? != CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID {
        return None;
    }
    let provider = doc
        .get("model_providers")?
        .as_table()?
        .get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)?
        .as_table()?;
    let fields = [
        "base_url",
        "experimental_bearer_token",
        "wire_api",
        "requires_openai_auth",
    ]
    .map(|key| {
        provider.get(key).map(|item| {
            item.as_str()
                .map(|text| json!(text))
                .or_else(|| item.as_bool().map(|flag| json!(flag)))
                .unwrap_or(Value::Null)
        })
    });
    let catalog = doc.get("model_catalog_json").and_then(|item| item.as_str());
    let encoded = serde_json::to_vec(&(fields, catalog)).ok()?;
    Some(format!("{:x}", Sha256::digest(encoded)))
}

fn remember_takeover_ownership(profile_dir: &Path) -> Result<(), String> {
    let doc = crate::modules::codex_config_format::load_codex_config_doc(&profile_config_path(
        profile_dir,
    ))?;
    let connection_hash = takeover_connection_hash(&doc)
        .ok_or_else(|| "Cannot record API service profile ownership".to_string())?;
    let content = serde_json::to_string(&TakeoverOwnership { connection_hash })
        .map_err(|error| error.to_string())?;
    write_string_atomic_if_changed(&profile_dir.join(TAKEOVER_OWNERSHIP_FILE), &content)?;
    Ok(())
}

/// None means a legacy profile, false means a user edit or invalid marker.
fn recorded_takeover_ownership_matches(
    profile_dir: &Path,
    doc: &Document,
) -> Result<Option<bool>, String> {
    let Some(content) = read_optional_profile_file(&profile_dir.join(TAKEOVER_OWNERSHIP_FILE))?
    else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_str::<TakeoverOwnership>(&content)
            .ok()
            .is_some_and(|record| {
                takeover_connection_hash(doc).as_deref() == Some(record.connection_hash.as_str())
            }),
    ))
}

fn profile_still_owned_for_collection(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> Result<bool, String> {
    let doc = crate::modules::codex_config_format::load_codex_config_doc(&profile_config_path(
        profile_dir,
    ))?;
    Ok(
        match recorded_takeover_ownership_matches(profile_dir, &doc)? {
            Some(owned) => owned,
            None => inspect_local_access_profile_attachment(profile_dir, Some(collection)).attached,
        },
    )
}

fn maintain_local_access_profile(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    if !collection.enabled || !profile_config_path(profile_dir).exists() {
        return Ok(());
    }
    let _lease = match codex_account::try_acquire_profile_mutation_lease(
        profile_dir,
        "api-profile-maintenance",
    ) {
        Ok(lease) => lease,
        Err(error) => {
            logger::log_codex_api_warn(&format!("Skipping busy API profile maintenance: {error}"));
            return Ok(());
        }
    };
    let config_path = profile_config_path(profile_dir);
    let mut doc = crate::modules::codex_config_format::load_codex_config_doc(&config_path)?;
    let owned = match recorded_takeover_ownership_matches(profile_dir, &doc)? {
        Some(owned) => owned,
        None => {
            inspect_local_access_profile_attachment(profile_dir, Some(collection)).attached
                && doc
                    .get("model_catalog_json")
                    .and_then(|item| item.as_str())
                    .is_some_and(is_cockpit_managed_model_catalog_name)
        }
    };
    if !owned {
        return Ok(());
    }
    let provider = doc["model_providers"][CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID]
        .as_table_mut()
        .ok_or("Invalid API service provider table")?;
    let old_key = provider
        .get("experimental_bearer_token")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let old_base = provider
        .get("base_url")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let next_base = build_collection_base_url(collection);
    let supports_websockets = profile_api_key_supports_websockets(collection, &collection.api_key);
    // Preserve the existing migration for the historically mislabelled provider.
    if provider.get("name").and_then(|item| item.as_str()) == Some("OpenAI") {
        provider["name"] = value(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_NAME);
    }
    if old_base != next_base {
        provider["base_url"] = value(next_base.clone());
    }
    if old_key != collection.api_key {
        provider["experimental_bearer_token"] = value(collection.api_key.clone());
    }
    if provider
        .get("supports_websockets")
        .and_then(|item| item.as_bool())
        != Some(supports_websockets)
    {
        provider["supports_websockets"] = value(supports_websockets);
    }
    if old_base != next_base
        && doc
            .get("experimental_realtime_ws_base_url")
            .and_then(|item| item.as_str())
            == Some(old_base.as_str())
    {
        doc["experimental_realtime_ws_base_url"] = value(next_base);
    }
    let config = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    let original_config =
        std::fs::read_to_string(&config_path).map_err(|error| error.to_string())?;
    let auth_path = profile_auth_path(profile_dir);
    let original_auth = read_optional_profile_file(&auth_path)?;
    let updated_auth = original_auth
        .as_deref()
        .filter(|text| is_exact_codex_local_access_auth_text(text, &old_key))
        .filter(|_| old_key != collection.api_key)
        .map(|text| {
            let mut auth: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
            auth["OPENAI_API_KEY"] = json!(collection.api_key);
            serde_json::to_string_pretty(&auth).map_err(|error| error.to_string())
        })
        .transpose()?;
    persist_takeover_connection(
        profile_dir,
        &original_config,
        original_auth.as_deref(),
        &config,
        updated_auth.as_deref(),
        || remember_takeover_ownership(profile_dir),
    )?;
    let definitions =
        local_access_profile_model_definitions(profile_dir, collection, &collection.api_key, true)?;
    write_local_access_profile_model_catalog(profile_dir, supports_websockets, &definitions)?;
    remember_takeover_ownership(profile_dir)
}

// Save connection + marker together from the caller's perspective. On an I/O
// failure restore the exact previous credentials; never adopt a partial write.
fn persist_takeover_connection(
    profile_dir: &Path,
    original_config: &str,
    original_auth: Option<&str>,
    config: &str,
    updated_auth: Option<&str>,
    save_marker: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let config_path = profile_config_path(profile_dir);
    let auth_path = profile_auth_path(profile_dir);
    let result = (|| {
        if config != original_config {
            crate::modules::codex_config_format::write_codex_config_toml_atomic(
                &config_path,
                config,
            )?;
        }
        if let Some(auth) = updated_auth {
            write_string_atomic(&auth_path, auth)?;
        }
        save_marker()
    })();
    if let Err(error) = result {
        let config_rollback = write_optional_profile_file(&config_path, Some(original_config));
        let auth_rollback = write_optional_profile_file(&auth_path, original_auth);
        return Err(format!("API profile maintenance failed: {error}; config rollback={config_rollback:?}; auth rollback={auth_rollback:?}"));
    }
    Ok(())
}
