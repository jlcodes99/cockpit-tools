// Field-only transactions preserve concurrent stats and account changes.
use crate::models::codex_local_access::CodexLocalAccessMultiAgentConfig;

fn normalize_multi_agent(
    _config: CodexLocalAccessMultiAgentConfig,
    _members: &[String],
) -> Result<CodexLocalAccessMultiAgentConfig, String> {
    // Old internal callers may still submit retired fields. They cannot disable
    // built-in support or reintroduce a hand-maintained model whitelist.
    Ok(CodexLocalAccessMultiAgentConfig::default())
}

fn prune_multi_agent_references(collection: &mut CodexLocalAccessCollection) -> bool {
    let canonical = CodexLocalAccessMultiAgentConfig::default();
    if collection.multi_agent == canonical {
        return false;
    }
    collection.multi_agent = canonical;
    true
}

fn persist_multi_agent_at_path(
    path: &Path,
    config: CodexLocalAccessMultiAgentConfig,
    fallback: Option<CodexLocalAccessCollection>,
) -> Result<CodexLocalAccessCollection, String> {
    let mut published = None;
    update_string_atomic(path, |current| {
        let mut collection = match current {
            Some(current) => serde_json::from_str::<CodexLocalAccessCollection>(current)
                .map_err(|_| "MULTI_AGENT_STORAGE".to_string())?,
            None => fallback
                .clone()
                .map(Ok)
                .unwrap_or_else(new_local_access_collection)?,
        };
        collection.multi_agent = normalize_multi_agent(config.clone(), &collection.account_ids)?;
        collection.updated_at = now_ms();
        let content = serde_json::to_string_pretty(&collection)
            .map_err(|_| "MULTI_AGENT_STORAGE".to_string())?;
        published = Some(collection);
        Ok(content)
    })?;
    published.ok_or_else(|| "MULTI_AGENT_STORAGE".to_string())
}

fn multi_agent_save_lock() -> &'static tokio::sync::Mutex<()> {
    static SAVES: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SAVES.get_or_init(|| tokio::sync::Mutex::new(()))
}

pub async fn update_local_access_multi_agent(
    config: CodexLocalAccessMultiAgentConfig,
) -> Result<CodexLocalAccessState, String> {
    let _save = multi_agent_save_lock().lock().await;
    ensure_runtime_loaded_without_start().await?;
    let fallback = gateway_runtime().lock().await.collection.clone();
    let path = local_access_file_path()?;
    let published =
        tokio::task::spawn_blocking(move || persist_multi_agent_at_path(&path, config, fallback))
            .await
            .map_err(|_| "MULTI_AGENT_STORAGE".to_string())??;
    let reload = {
        let mut runtime = gateway_runtime().lock().await;
        if let Some(collection) = &mut runtime.collection {
            collection.multi_agent = published.multi_agent;
            prune_multi_agent_references(collection);
            collection.updated_at = collection.updated_at.max(published.updated_at);
        } else {
            sync_runtime_collection(&mut runtime, published);
        }
        runtime.running
    };
    if reload {
        trigger_gateway_reload_in_background("multi-agent settings");
    }
    emit_local_access_state_updated();
    snapshot_state_without_gateway_reload().await
}

#[cfg(test)]
#[path = "codex_local_access_multi_agent_settings_tests.rs"]
mod multi_agent_settings_tests;
