// Optional capability declarations and ordering for automatic routing.
use crate::models::codex_local_access::{
    CodexLocalAccessCapabilityFallbackConfig, CodexLocalAccessCapabilitySupport,
    CODEX_CAPABILITY_IDS,
};

fn normalize_capability_fallback(
    mut config: CodexLocalAccessCapabilityFallbackConfig,
    members: &[String],
) -> Result<CodexLocalAccessCapabilityFallbackConfig, String> {
    let mut seen = HashSet::new();
    config.account_ids = config
        .account_ids
        .into_iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect();
    config.enabled = true;
    if config.account_ids.iter().any(|id| !members.contains(id)) {
        return Err("CAPABILITY_FALLBACK_ACCOUNT_NOT_IN_POOL".into());
    }
    if config.overrides.len() > 4096 {
        return Err("CAPABILITY_FALLBACK_TOO_MANY_OVERRIDES".into());
    }
    let mut identities = HashSet::new();
    for entry in &mut config.overrides {
        entry.account_id = entry.account_id.trim().to_owned();
        entry.model = entry.model.trim().to_owned();
        if !members.contains(&entry.account_id) {
            return Err("CAPABILITY_FALLBACK_ACCOUNT_NOT_IN_POOL".into());
        }
        if entry.model.is_empty()
            || entry.model.len() > 256
            || entry.model.chars().any(char::is_control)
            || (entry.model.contains('*') && entry.model != "*")
        {
            return Err("CAPABILITY_FALLBACK_INVALID_MODEL".into());
        }
        if !identities.insert((entry.account_id.clone(), entry.model.clone())) {
            return Err("CAPABILITY_FALLBACK_DUPLICATE_OVERRIDE".into());
        }
        if entry
            .capabilities
            .keys()
            .any(|id| !CODEX_CAPABILITY_IDS.contains(&id.as_str()))
        {
            return Err("CAPABILITY_FALLBACK_INVALID_CAPABILITY".into());
        }
    }
    Ok(config)
}

fn prune_capability_fallback_references(collection: &mut CodexLocalAccessCollection) -> bool {
    let previous = collection.capability_fallback.clone();
    let members = &collection.account_ids;
    collection
        .capability_fallback
        .account_ids
        .retain(|id| members.contains(id));
    collection
        .capability_fallback
        .overrides
        .retain(|entry| members.contains(&entry.account_id));
    collection.capability_fallback.enabled = true;
    previous != collection.capability_fallback
}

fn persist_capability_fallback_at_path(
    path: &Path,
    config: CodexLocalAccessCapabilityFallbackConfig,
    fallback: Option<CodexLocalAccessCollection>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut collection = if let Some(current) = current {
            serde_json::from_str::<CodexLocalAccessCollection>(current)
                .map_err(|_| "CAPABILITY_FALLBACK_STORAGE".to_string())?
        } else {
            fallback
                .clone()
                .map(Ok)
                .unwrap_or_else(new_local_access_collection)?
        };
        collection.capability_fallback =
            normalize_capability_fallback(config.clone(), &collection.account_ids)?;
        collection.updated_at = now_ms();
        let content = serde_json::to_string_pretty(&collection)
            .map_err(|_| "CAPABILITY_FALLBACK_STORAGE".to_string())?;
        published = Some(collection);
        Ok(content)
    })?;
    published.ok_or_else(|| "CAPABILITY_FALLBACK_STORAGE".to_string())
}

fn capability_fallback_save_lock() -> &'static tokio::sync::Mutex<()> {
    static SAVES: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SAVES.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub async fn update_local_access_capability_fallback(
    config: CodexLocalAccessCapabilityFallbackConfig,
) -> Result<CodexLocalAccessState, String> {
    // Serialize field publications, so two concurrent saves cannot publish in
    // reverse disk order. This lock never guards network I/O or the runtime.
    let _save = capability_fallback_save_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let fallback = gateway_runtime().lock().await.collection.clone();
    let path = local_access_file_path()?;
    let published = tokio::task::spawn_blocking(move || {
        persist_capability_fallback_at_path(&path, config, fallback)
    })
    .await
    .map_err(|_| "CAPABILITY_FALLBACK_STORAGE".to_string())??;
    {
        let mut runtime = gateway_runtime().lock().await;
        if let Some(collection) = &mut runtime.collection {
            collection.capability_fallback = published.capability_fallback;
            // An explicit account removal may have completed while the file
            // transaction was on a blocking worker. Never republish its reference.
            prune_capability_fallback_references(collection);
            collection.updated_at = collection.updated_at.max(published.updated_at);
        } else {
            sync_runtime_collection(&mut runtime, published);
        }
    }
    emit_local_access_state_updated();
    snapshot_state_without_gateway_reload().await
}

#[cfg(test)]
#[path = "codex_local_access_capability_settings_tests.rs"]
mod capability_settings_tests;
