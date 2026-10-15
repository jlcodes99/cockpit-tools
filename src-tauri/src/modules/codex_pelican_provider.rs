//! Persist references and display labels only. Credentials are resolved for every attempt.
use crate::modules::codex_local_access::CodexModelProviderGatewayChatTestRequest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTarget {
    pub provider_id: String,
    pub api_key_id: String,
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSource {
    #[serde(flatten)]
    pub target: ProviderTarget,
    pub provider_name: String,
    pub api_key_name: String,
    pub base_url: String,
    pub wire_api: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Provider {
    id: String,
    name: String,
    base_url: String,
    wire_api: Option<String>,
    #[serde(default)]
    model_catalog: Vec<String>,
    api_keys: Vec<ApiKey>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKey {
    id: String,
    name: String,
    api_key: String,
}

fn resolve(
    providers: &[Provider],
    target: &ProviderTarget,
) -> Result<(ProviderSource, String, Vec<String>), String> {
    if target.provider_id.trim().is_empty()
        || target.api_key_id.trim().is_empty()
        || target.model.trim().is_empty()
        || target.model.len() > 200
    {
        return Err("pelican.error.invalidRequest".into());
    }
    let provider = providers
        .iter()
        .find(|p| p.id == target.provider_id)
        .ok_or("pelican.error.providerUnavailable")?;
    let key = provider
        .api_keys
        .iter()
        .find(|k| k.id == target.api_key_id && !k.api_key.trim().is_empty())
        .ok_or("pelican.error.providerUnavailable")?;
    let url = reqwest::Url::parse(provider.base_url.trim())
        .map_err(|_| "pelican.error.invalidRequest")?;
    // Do not copy embedded credentials or query secrets into history.
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("pelican.error.invalidRequest".into());
    }
    let wire_api = crate::modules::codex_provider_protocol::normalize_model_provider_wire_api(
        provider.wire_api.as_deref(),
        &provider.base_url,
    );
    Ok((
        ProviderSource {
            target: ProviderTarget {
                model: target.model.trim().into(),
                ..target.clone()
            },
            provider_name: provider.name.clone(),
            api_key_name: key.name.clone(),
            base_url: provider.base_url.trim().trim_end_matches('/').into(),
            wire_api,
        },
        key.api_key.clone(),
        provider.model_catalog.clone(),
    ))
}

async fn read_targets(
    targets: Vec<ProviderTarget>,
) -> Result<Vec<(ProviderSource, String, Vec<String>)>, String> {
    static SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)));
    tokio::time::timeout(super::IO_TIMEOUT, async move {
        let permit = SLOTS
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| "pelican.error.providerUnavailable".to_string())?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let path = crate::modules::account::get_data_dir()?.join("codex_model_providers.json");
            // Bound a corrupt or unexpectedly large catalog before allocating it.
            if std::fs::metadata(&path)
                .map_err(|_| "pelican.error.providerUnavailable")?
                .len()
                > 16 * 1024 * 1024
            {
                return Err("pelican.error.providerUnavailable".into());
            }
            let raw =
                std::fs::read_to_string(path).map_err(|_| "pelican.error.providerUnavailable")?;
            let providers: Vec<Provider> = serde_json::from_str(&raw)
                .map_err(|_| "pelican.error.providerUnavailable".to_string())?;
            targets
                .iter()
                .map(|target| resolve(&providers, target))
                .collect()
        })
        .await
        .map_err(|_| "pelican.error.providerUnavailable".to_string())?
    })
    .await
    .map_err(|_| "pelican.error.storageTimeout".to_string())?
}

pub(super) async fn sources(targets: Vec<ProviderTarget>) -> Result<Vec<ProviderSource>, String> {
    Ok(read_targets(targets)
        .await?
        .into_iter()
        .map(|(source, _, _)| source)
        .collect())
}

pub(super) async fn request(
    source: &ProviderSource,
    run_id: &str,
    prompt: &str,
) -> Result<CodexModelProviderGatewayChatTestRequest, String> {
    let (current, api_key, model_catalog) = read_targets(vec![source.target.clone()])
        .await?
        .pop()
        .ok_or("pelican.error.providerUnavailable")?;
    // Never send historical continuation to a newly configured endpoint or protocol.
    if current.base_url != source.base_url || current.wire_api != source.wire_api {
        return Err("pelican.error.providerChanged".into());
    }
    Ok(CodexModelProviderGatewayChatTestRequest {
        run_id: run_id.into(),
        provider_id: source.target.provider_id.clone(),
        provider_name: source.provider_name.clone(),
        base_url: current.base_url,
        api_key_id: Some(source.target.api_key_id.clone()),
        api_key_name: Some(source.api_key_name.clone()),
        api_key,
        wire_api: current.wire_api,
        model_catalog,
        model_id: source.target.model.clone(),
        prompt: prompt.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn resolve_raw(
        raw: &str,
        target: &ProviderTarget,
    ) -> Result<(ProviderSource, String, Vec<String>), String> {
        let providers = serde_json::from_str::<Vec<Provider>>(raw)
            .map_err(|_| "invalid fixture".to_string())?;
        resolve(&providers, target)
    }
    const RAW: &str = r#"[{"id":"p","name":"Relay","baseUrl":"https://example.test/v1","wireApi":"chat_completions","apiKeys":[{"id":"k","name":"Test key","apiKey":"secret-123"}]}]"#;
    fn target() -> ProviderTarget {
        ProviderTarget {
            provider_id: "p".into(),
            api_key_id: "k".into(),
            model: "custom-model".into(),
        }
    }
    #[test]
    fn provider_pelican_snapshot_excludes_secrets_and_keeps_model_and_key_identity() {
        let (source, key, _) = resolve_raw(RAW, &target()).unwrap();
        assert_eq!(key, "secret-123");
        let encoded = serde_json::to_string(&source).unwrap();
        assert!(!encoded.contains("secret-123"));
        assert_eq!(source.target.model, "custom-model");
        assert_eq!(source.api_key_name, "Test key");
        assert_eq!(source.wire_api, "chat_completions");
    }
    #[test]
    fn provider_pelican_missing_key_and_credential_url_are_rejected() {
        let mut missing = target();
        missing.api_key_id = "deleted".into();
        assert!(resolve_raw(RAW, &missing).is_err());
        assert!(resolve_raw(
            &RAW.replace(
                "https://example.test/v1",
                "https://user:secret@example.test/v1"
            ),
            &target()
        )
        .is_err());
        assert!(resolve_raw("invalid", &target()).is_err());
    }
}
