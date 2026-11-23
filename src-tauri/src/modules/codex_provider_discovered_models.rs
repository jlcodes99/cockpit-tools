use crate::models::codex::{CodexAccount, CodexApiProviderMode};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::watch;

const LAUNCH_DISCOVERY_WAIT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModel {
    pub id: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelProviderModelsResult {
    pub models: Vec<CodexModelProviderModel>,
    pub latency_ms: u64,
}

pub(crate) fn codex_model_provider_models_url(base_url: &str) -> Result<String, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("PROVIDER_BASE_URL_INVALID".to_string());
    }
    let mut url =
        reqwest::Url::parse(trimmed).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    match url.scheme() {
        "http" | "https" => {}
        _ => return Err("PROVIDER_BASE_URL_INVALID".to_string()),
    }
    let next_path = if url.path().is_empty() || url.path() == "/" {
        "/models".to_string()
    } else {
        format!("{}/models", url.path().trim_end_matches('/'))
    };
    url.set_path(&next_path);
    url.set_query(None);
    Ok(url.to_string())
}

fn list_model_provider_models(body: &serde_json::Value) -> Option<Vec<CodexModelProviderModel>> {
    let (entries, id_key) = match body["data"].as_array() {
        Some(entries) => (entries, "id"),
        None => (body["models"].as_array()?, "slug"),
    };
    Some(
        entries
            .iter()
            .filter_map(|item| {
                let id = item[id_key]
                    .as_str()
                    .or_else(|| item["id"].as_str())?
                    .trim();
                if id.is_empty() {
                    return None;
                }
                Some(CodexModelProviderModel {
                    id: id.to_owned(),
                    display_name: item["display_name"]
                        .as_str()
                        .or_else(|| item["displayName"].as_str())
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned),
                })
            })
            .collect(),
    )
}

pub(crate) async fn fetch_provider_models(
    base_url: &str,
    api_key: &str,
    wire_api: Option<&str>,
    auth_scheme: Option<&str>,
    launch_discovery: bool,
) -> Result<CodexModelProviderModelsResult, String> {
    if api_key.trim().is_empty() {
        return Err("MISSING_API_KEY".into());
    }
    let mut client = reqwest::Client::builder().timeout(Duration::from_secs(20));
    if launch_discovery {
        client = client.redirect(reqwest::redirect::Policy::none());
    }
    let client = client
        .build()
        .map_err(|_| "PROVIDER_MODELS_NETWORK_FAILED".to_string())?;
    let protocol =
        super::codex_provider_protocol::normalize_model_provider_wire_api(wire_api, base_url);
    let raw = if protocol == "anthropic" {
        crate::modules::codex_native_api::anthropic::endpoint(base_url, "models")
            .map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?
    } else {
        codex_model_provider_models_url(base_url)?
    };
    let mut url = reqwest::Url::parse(&raw).map_err(|_| "PROVIDER_BASE_URL_INVALID".to_string())?;
    let started = Instant::now();
    let mut models = Vec::new();
    let mut seen = HashSet::new();
    let mut remaining_bytes = super::codex_model_catalog::MAX_CATALOG_BYTES;
    for page in 0..10 {
        let remaining = Duration::from_secs(20)
            .checked_sub(started.elapsed())
            .ok_or("PROVIDER_MODELS_NETWORK_FAILED")?;
        let request = crate::modules::codex_provider_protocol::authenticate_provider_request(
            client.get(url.clone()),
            &protocol,
            auth_scheme,
            api_key,
        )
        .timeout(remaining);
        let response = request
            .send()
            .await
            .map_err(|_| "PROVIDER_MODELS_NETWORK_FAILED".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "PROVIDER_MODELS_HTTP_{}",
                response.status().as_u16()
            ));
        }
        let body: serde_json::Value = if launch_discovery {
            let mut response = response;
            let mut bytes = Vec::new();
            let limit = crate::modules::codex_model_catalog::MAX_CATALOG_BYTES;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "PROVIDER_MODELS_PARSE_FAILED")?
            {
                if chunk.len() > remaining_bytes {
                    return Err("PROVIDER_MODELS_PARSE_FAILED".to_string());
                }
                remaining_bytes -= chunk.len();
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes)
                .map_err(|_| "PROVIDER_MODELS_PARSE_FAILED".to_string())?
        } else {
            response
                .json()
                .await
                .map_err(|_| "PROVIDER_MODELS_PARSE_FAILED".to_string())?
        };
        let page_models = match list_model_provider_models(&body) {
            Some(models) => models,
            None if launch_discovery => return Err("PROVIDER_MODELS_PARSE_FAILED".into()),
            None => Vec::new(),
        };
        models.extend(
            page_models
                .into_iter()
                .filter(|model| seen.insert(model.id.to_ascii_lowercase())),
        );
        if body["has_more"] != true {
            break;
        }
        let last = body["last_id"].as_str().filter(|id| !id.is_empty());
        if launch_discovery && (last.is_none() || page == 9) {
            return Err("PROVIDER_MODELS_PARSE_FAILED".into());
        }
        let Some(last) = last else { break };
        url.query_pairs_mut().clear().append_pair("after_id", last);
    }
    Ok(CodexModelProviderModelsResult {
        models,
        latency_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    })
}

#[derive(Clone)]
struct Source {
    account_id: String,
    base_url: String,
    api_key: String,
    protocol: String,
    auth_scheme: Option<String>,
}

impl Source {
    fn for_account(account: &CodexAccount) -> Option<Self> {
        use super::codex_account;
        if !account.is_api_key_auth()
            || account.api_provider_mode != CodexApiProviderMode::Custom
            || !account.api_model_catalog.is_empty()
            || codex_account::is_chatgpt_plan_account(account)
            || codex_account::account_uses_cdp_model_injection(account)
            || codex_account::is_grok_upstream_provider(account)
            || super::codex_remote_service::is_known_account(account)
        {
            return None;
        }
        let base_url = account
            .api_base_url
            .as_deref()?
            .trim()
            .trim_end_matches('/');
        let api_key = account.openai_api_key.as_deref()?.trim();
        if base_url.is_empty() || api_key.is_empty() {
            return None;
        }
        Some(Self {
            account_id: account.id.clone(),
            base_url: base_url.to_owned(),
            api_key: api_key.to_owned(),
            protocol: super::codex_provider_protocol::normalize_model_provider_wire_api(
                account.api_wire_api.as_deref(),
                base_url,
            ),
            auth_scheme: account.api_auth_scheme.clone(),
        })
    }

    fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for value in [
            self.account_id.as_str(),
            &self.base_url,
            &self.api_key,
            &self.protocol,
            self.auth_scheme.as_deref().unwrap_or_default(),
        ] {
            hash.update(value.len().to_le_bytes());
            hash.update(value.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }
}

pub(crate) struct LaunchModels {
    source: String,
    cached: Vec<String>,
    refresh: Option<RefreshReceiver>,
}

impl LaunchModels {
    pub(crate) fn apply_to(&self, account: &mut CodexAccount) -> bool {
        if !Source::for_account(account).is_some_and(|source| source.fingerprint() == self.source) {
            return false;
        }
        let refreshed = self.refresh.as_ref().and_then(|receiver| {
            receiver
                .borrow()
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .map(|models| model_ids(models))
        });
        let models = refreshed.unwrap_or_else(|| self.cached.clone());
        if models.is_empty() {
            return false;
        }
        account.api_model_catalog = models;
        account.api_sync_model_catalog_to_codex = true;
        true
    }
}

type RefreshResult = Result<Arc<CodexModelProviderModelsResult>, String>;
type RefreshReceiver = watch::Receiver<Option<RefreshResult>>;
static REFRESHES: LazyLock<Mutex<HashMap<String, RefreshReceiver>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_path(root: &Path, source: &str) -> PathBuf {
    root.join("codex-provider-model-discovery")
        .join(format!("{source}.json"))
}

fn read_cached_models(path: &Path) -> Result<Option<Vec<String>>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let limit = super::codex_model_catalog::MAX_CATALOG_BYTES;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("PROVIDER_MODELS_PARSE_FAILED".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

async fn cached_models(path: PathBuf) -> Option<Vec<String>> {
    match tokio::task::spawn_blocking(move || read_cached_models(&path)).await {
        Ok(Ok(models)) => models,
        Ok(Err(error)) => {
            super::logger::log_warn(&format!("[Codex] 读取供应商模型缓存失败: {error}"));
            None
        }
        Err(error) => {
            super::logger::log_warn(&format!("[Codex] 供应商模型缓存任务失败: {error}"));
            None
        }
    }
}

fn model_ids(result: &CodexModelProviderModelsResult) -> Vec<String> {
    result.models.iter().map(|model| model.id.clone()).collect()
}

fn refresh(source: Source, fingerprint: String, path: PathBuf) -> RefreshReceiver {
    let mut refreshes = REFRESHES.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(existing) = refreshes.get(&fingerprint) {
        return existing.clone();
    }
    let (sender, receiver) = watch::channel(None);
    refreshes.insert(fingerprint.clone(), receiver.clone());
    tokio::spawn(async move {
        let result = fetch_provider_models(
            &source.base_url,
            &source.api_key,
            Some(&source.protocol),
            source.auth_scheme.as_deref(),
            true,
        )
        .await;
        match &result {
            Ok(models) => {
                let ids = model_ids(models);
                match tokio::task::spawn_blocking(move || {
                    let json = serde_json::to_string(&ids).map_err(|error| error.to_string())?;
                    if json.len() > super::codex_model_catalog::MAX_CATALOG_BYTES {
                        return Err("PROVIDER_MODELS_PARSE_FAILED".into());
                    }
                    super::atomic_write::write_secret_string_atomic(&path, &json)
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        super::logger::log_warn(&format!("[Codex] 保存供应商模型缓存失败: {error}"))
                    }
                    Err(error) => {
                        super::logger::log_warn(&format!("[Codex] 供应商模型缓存任务失败: {error}"))
                    }
                }
            }
            Err(error) => super::logger::log_warn(&format!("[Codex] 拉取供应商模型失败: {error}")),
        }
        sender.send_replace(Some(result.map(Arc::new)));
        REFRESHES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&fingerprint);
    });
    receiver
}

async fn refreshed_models(mut receiver: RefreshReceiver) -> RefreshResult {
    receiver
        .wait_for(Option::is_some)
        .await
        .map_err(|_| "PROVIDER_MODELS_NETWORK_FAILED".to_string())?
        .as_ref()
        .expect("wait_for requires a result")
        .clone()
}

pub(crate) async fn prefetch(account_id: String) -> Result<CodexModelProviderModelsResult, String> {
    let source = tokio::task::spawn_blocking(move || {
        super::codex_account::load_account(&account_id)
            .as_ref()
            .and_then(Source::for_account)
    })
    .await
    .map_err(|error| error.to_string())?
    .ok_or("PROVIDER_BASE_URL_INVALID")?;
    let fingerprint = source.fingerprint();
    let path = cache_path(&super::config::get_shared_dir(), &fingerprint);
    refreshed_models(refresh(source, fingerprint, path))
        .await
        .map(|models| (*models).clone())
}

pub(crate) async fn for_launch(bind_account_id: Option<&str>) -> Option<LaunchModels> {
    let bind_account_id = bind_account_id?;
    if super::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        return None;
    }
    let account_id = super::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
        .unwrap_or_else(|| bind_account_id.to_owned());
    let source = tokio::task::spawn_blocking(move || {
        super::codex_account::load_account(&account_id)
            .as_ref()
            .and_then(Source::for_account)
    })
    .await
    .ok()??;
    let root = super::config::get_shared_dir();
    launch_models(source, &root, LAUNCH_DISCOVERY_WAIT).await
}

async fn launch_models(source: Source, root: &Path, wait: Duration) -> Option<LaunchModels> {
    let fingerprint = source.fingerprint();
    let path = cache_path(root, &fingerprint);
    let cached = cached_models(path.clone()).await;
    let receiver = refresh(source, fingerprint.clone(), path);
    if cached.is_none() {
        // A slow first discovery can still finish during profile preparation
        // or populate the cache for the next launch.
        let _ = tokio::time::timeout(wait, refreshed_models(receiver.clone())).await;
    }
    Some(LaunchModels {
        source: fingerprint,
        cached: cached.unwrap_or_default(),
        refresh: Some(receiver),
    })
}

pub(crate) async fn apply_cached(account: &mut CodexAccount) {
    if let Some(source) = Source::for_account(account) {
        let fingerprint = source.fingerprint();
        let path = cache_path(&super::config::get_shared_dir(), &fingerprint);
        if let Some(models) = cached_models(path).await {
            LaunchModels {
                source: fingerprint,
                cached: models,
                refresh: None,
            }
            .apply_to(account);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn relay() -> (tokio::net::TcpListener, CodexAccount) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let account = CodexAccount::new_api_key(
            "relay".into(),
            "relay@example.invalid".into(),
            "fixture-key".into(),
            CodexApiProviderMode::Custom,
            Some(base_url),
            None,
            None,
            Vec::new(),
        );
        (listener, account)
    }

    async fn serve_models(listener: tokio::net::TcpListener, status: &str, body: &str) {
        let (stream, _) = tokio::time::timeout(LAUNCH_DISCOVERY_WAIT, listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut stream = BufReader::new(stream);
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(line, "GET /v1/models HTTP/1.1\r\n");
        let mut authenticated = false;
        loop {
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            if line == "\r\n" || line.is_empty() {
                break;
            }
            authenticated |= line
                .trim()
                .eq_ignore_ascii_case("authorization: Bearer fixture-key");
        }
        assert!(authenticated);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn first_launch_writes_discovered_6_2_sol_with_its_real_id() {
        use crate::modules::{codex_account, codex_config_format, codex_local_access};

        let (listener, saved) = relay().await;
        let upstream = tokio::spawn(serve_models(
            listener,
            "200 OK",
            r#"{"models":[{"slug":"gpt-6.2-sol"},{"slug":"custom-model"}]}"#,
        ));
        let cache = tempfile::tempdir().unwrap();
        let source = Source::for_account(&saved).unwrap();
        let (first, second) = tokio::join!(
            launch_models(source.clone(), cache.path(), LAUNCH_DISCOVERY_WAIT),
            launch_models(source.clone(), cache.path(), LAUNCH_DISCOVERY_WAIT),
        );
        upstream.await.unwrap();
        let first = first.unwrap();
        let second = second.unwrap();
        let mut launch = saved.clone();
        assert!(!codex_local_access::account_requires_provider_gateway(
            &launch
        ));
        assert!(first.apply_to(&mut launch));
        assert_eq!(launch.api_model_catalog, ["gpt-6.2-sol", "custom-model"]);
        assert_eq!(
            read_cached_models(&cache_path(cache.path(), &source.fingerprint())).unwrap(),
            Some(launch.api_model_catalog.clone()),
        );
        assert!(saved.api_model_catalog.is_empty());

        let profile = tempfile::tempdir().unwrap();
        let other_profile = tempfile::tempdir().unwrap();
        let config = profile.path().join("config.toml");
        std::fs::write(&config, "model = 'custom-model'\n").unwrap();
        assert!(
            codex_account::sync_discovered_provider_catalog_to_dir(profile.path(), &launch)
                .unwrap()
        );
        let doc = codex_config_format::load_codex_config_doc(&config).unwrap();
        assert_eq!(doc["model"].as_str(), Some("custom-model"));
        let catalog_path = profile
            .path()
            .join(doc["model_catalog_json"].as_str().unwrap());
        let catalog: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(catalog_path).unwrap()).unwrap();
        assert!(catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| { model["slug"] == "gpt-6.2-sol" && model["visibility"] == "list" }));
        assert_eq!(std::fs::read_dir(other_profile.path()).unwrap().count(), 0);

        let slots = codex_local_access::provider_model_slots_for_account(
            &launch,
            &launch.api_model_catalog,
        );
        let gateway_catalog: serde_json::Value = serde_json::from_str(
            &codex_local_access::build_provider_model_catalog_json(&slots).unwrap(),
        )
        .unwrap();
        assert!(gateway_catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| {
                model["display_name"] == "gpt-6.2-sol" && model["visibility"] == "list"
            }));

        let mut manual = saved.clone();
        manual.api_model_catalog = vec!["selected-model".into()];
        assert!(!second.apply_to(&mut manual));
        assert_eq!(manual.api_model_catalog, ["selected-model"]);

        let mut changed_key = saved;
        changed_key.openai_api_key = Some("changed-key".into());
        assert!(!second.apply_to(&mut changed_key));
        assert!(changed_key.api_model_catalog.is_empty());
    }

    #[tokio::test]
    async fn cached_models_survive_a_failed_background_refresh() {
        let (listener, mut account) = relay().await;
        let cache = tempfile::tempdir().unwrap();
        let source = Source::for_account(&account).unwrap();
        let path = cache_path(cache.path(), &source.fingerprint());
        let cached = r#"["gpt-6.2-sol"]"#;
        super::super::atomic_write::write_secret_string_atomic(&path, cached).unwrap();
        let launch = launch_models(source, cache.path(), Duration::ZERO)
            .await
            .unwrap();
        assert!(launch.apply_to(&mut account));
        let upstream = tokio::spawn(serve_models(listener, "503 Service Unavailable", ""));
        let error = refreshed_models(launch.refresh.clone().unwrap())
            .await
            .unwrap_err();
        upstream.await.unwrap();
        assert_eq!(error, "PROVIDER_MODELS_HTTP_503");
        assert_eq!(std::fs::read_to_string(path).unwrap(), cached);
    }
}
