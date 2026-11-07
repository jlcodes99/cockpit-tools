//! Capability discovery for a user-configured Cockpit endpoint. Never follows
//! redirects with credentials. Cached descriptors contain no API keys.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyQuery {
    pub start_at: Option<i64>,
    pub end_at: Option<i64>,
    pub before_time: Option<i64>,
    pub before_id: Option<i64>,
    pub page_size: Option<u32>,
    pub model: Option<String>,
    pub success: Option<bool>,
    pub kind: Option<String>,
    pub thread_id: Option<String>,
    pub instance_id: Option<String>,
    pub installation_id: Option<String>,
    pub service_tier: Option<String>,
}

pub async fn key_data(
    base: &str,
    key: &str,
    resource: &str,
    query: &KeyQuery,
) -> Result<Value, String> {
    if !matches!(resource, "stats" | "requests" | "session") {
        return Err("COCKPIT_REMOTE_QUERY_INVALID".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| "COCKPIT_REMOTE_CLIENT_FAILED")?;
    let mut url = endpoint(base, resource)?;
    let value = serde_json::to_value(query).map_err(|_| "COCKPIT_REMOTE_QUERY_INVALID")?;
    for (name, value) in value.as_object().into_iter().flatten() {
        if !value.is_null() {
            url.query_pairs_mut().append_pair(
                name,
                &value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string()),
            );
        }
    }
    tokio::time::timeout(Duration::from_secs(5), get_json(&client, url, key))
        .await
        .map_err(|_| "COCKPIT_REMOTE_UNREACHABLE")??
        .ok_or_else(|| "COCKPIT_REMOTE_USAGE_UNAVAILABLE".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub product: String,
    pub protocol_version: u32,
    pub service_id: String,
    pub service_name: String,
    pub key_label: String,
    pub status: String,
    pub models: Vec<String>,
    #[serde(default)]
    pub model_catalog: Option<Value>,
    #[serde(default)]
    pub catalog_revision: Option<String>,
    #[serde(default)]
    pub key_views_version: u32,
    pub capabilities: Capabilities,
    pub image_models: Vec<String>,
    pub token_limit: Option<u64>,
    pub token_used: u64,
    pub budget: Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub responses: bool,
    pub images: bool,
    pub image_edits: bool,
    pub hosted_images: bool,
    pub usage: bool,
    pub websockets: bool,
}

#[derive(Clone)]
struct Cached {
    value: Option<Connection>,
    checked: std::time::Instant,
}
static CACHE: LazyLock<Mutex<HashMap<String, Cached>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FLIGHTS: LazyLock<Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn fingerprint(base: &str, key: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(base.trim().trim_end_matches('/').as_bytes());
    hash.update([0]);
    hash.update(key.trim().as_bytes());
    format!("{:x}", hash.finalize())
}

fn cache_path(id: &str) -> Option<std::path::PathBuf> {
    Some(
        super::account::get_data_dir()
            .ok()?
            .join("codex_remote_services")
            .join(format!("{id}.json")),
    )
}

pub fn endpoint(base: &str, resource: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(base.trim()).map_err(|_| "PROVIDER_BASE_URL_INVALID")?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("PROVIDER_BASE_URL_INVALID".into());
    }
    let path = url.path().trim_end_matches('/');
    let path = if path.is_empty() {
        format!("/v1/cockpit/{resource}")
    } else if path.ends_with("/v1") {
        format!("{path}/cockpit/{resource}")
    } else {
        format!("{path}/v1/cockpit/{resource}")
    };
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn valid(value: &Connection) -> bool {
    value.product == "cockpit-tools"
        && value.protocol_version == 1
        && !value.service_id.is_empty()
        && value.budget["policy"].is_object()
        && matches!(
            value
                .budget
                .pointer("/policy/period")
                .and_then(Value::as_str),
            Some("lifetime" | "daily" | "weekly" | "monthly")
        )
        && value.budget["usedMicrousd"].as_u64().is_some()
        && value.models.len() <= 4096
        && value.image_models.len() <= 4096
        && value
            .models
            .iter()
            .chain(&value.image_models)
            .all(|id| !id.is_empty() && id.len() <= 200 && !id.chars().any(char::is_whitespace))
}

pub fn cached(base: &str, key: &str) -> Option<Connection> {
    let id = fingerprint(base, key);
    if let Some(entry) = CACHE.lock().ok()?.get(&id) {
        return entry.value.clone();
    }
    let value = cache_path(&id)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Connection>(&text).ok())
        .filter(valid);
    if let Ok(mut cache) = CACHE.lock() {
        cache.insert(
            id,
            Cached {
                value: value.clone(),
                checked: std::time::Instant::now() - Duration::from_secs(301),
            },
        );
    }
    value
}

pub fn cached_memory(base: &str, key: &str) -> Option<Connection> {
    CACHE
        .lock()
        .ok()?
        .get(&fingerprint(base, key))?
        .value
        .clone()
}

pub(crate) fn is_known(base: &str, key: &str) -> bool {
    CACHE.lock().ok().is_some_and(|cache| {
        cache
            .get(&fingerprint(base, key))
            .is_some_and(|entry| entry.value.is_some())
    })
}

pub(crate) fn is_known_account(account: &crate::models::codex::CodexAccount) -> bool {
    account
        .api_base_url
        .as_deref()
        .zip(account.openai_api_key.as_deref())
        .is_some_and(|(base, key)| is_known(base, key))
}

pub fn for_routing_account(account: &crate::models::codex::CodexAccount) -> Option<Connection> {
    cached_memory(
        account.api_base_url.as_deref()?,
        account.openai_api_key.as_deref()?,
    )
}

pub fn for_account(account: &crate::models::codex::CodexAccount) -> Option<Connection> {
    cached(
        account.api_base_url.as_deref()?,
        account.openai_api_key.as_deref()?,
    )
}

#[cfg(test)]
pub(crate) fn remember_test_connection(base: &str, key: &str, value: Connection) {
    CACHE.lock().unwrap().insert(
        fingerprint(base, key),
        Cached {
            value: Some(value),
            checked: std::time::Instant::now(),
        },
    );
}

async fn get_json(
    client: &reqwest::Client,
    url: reqwest::Url,
    key: &str,
) -> Result<Option<Value>, String> {
    let mut response = client
        .get(url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|_| "COCKPIT_REMOTE_UNREACHABLE")?;
    if matches!(response.status().as_u16(), 404 | 405) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "COCKPIT_REMOTE_HTTP_{}",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "COCKPIT_REMOTE_READ_FAILED")?
    {
        if body.len() + chunk.len() > 1024 * 1024 {
            return Err("COCKPIT_REMOTE_RESPONSE_TOO_LARGE".into());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|_| "COCKPIT_REMOTE_INVALID_RESPONSE".into())
}

pub async fn discover(base: &str, key: &str, force: bool) -> Result<Option<Connection>, String> {
    let id = fingerprint(base, key);
    let flight = {
        let mut flights = FLIGHTS.lock().map_err(|_| "COCKPIT_REMOTE_BUSY")?;
        flights.retain(|_, value| value.strong_count() > 0);
        let flight = flights
            .get(&id)
            .and_then(std::sync::Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        flights.insert(id.clone(), Arc::downgrade(&flight));
        flight
    };
    let _guard = match flight.try_lock() {
        Ok(guard) => guard,
        Err(_) => return Ok(cached_memory(base, key)),
    };
    let known = CACHE
        .lock()
        .map_err(|_| "COCKPIT_REMOTE_BUSY")?
        .contains_key(&id);
    let previous = if known {
        cached_memory(base, key)
    } else {
        let (base, key) = (base.to_owned(), key.to_owned());
        tokio::task::spawn_blocking(move || cached(&base, &key))
            .await
            .map_err(|_| "COCKPIT_REMOTE_CACHE_FAILED")?
    };
    if !force {
        let entry = CACHE
            .lock()
            .map_err(|_| "COCKPIT_REMOTE_BUSY")?
            .get(&id)
            .cloned();
        if let Some(entry) = entry.filter(|entry| {
            entry.checked.elapsed()
                < Duration::from_secs(if entry.value.is_some() { 30 } else { 300 })
        }) {
            return Ok(entry.value);
        }
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|_| "COCKPIT_REMOTE_CLIENT_FAILED")?;
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        get_json(&client, endpoint(base, "connection")?, key),
    )
    .await;
    let value = match result {
        Ok(Ok(Some(value))) => serde_json::from_value::<Connection>(value)
            .ok()
            .filter(valid),
        Ok(Ok(None)) => None,
        _ if previous.is_some() => return Err("COCKPIT_REMOTE_UNREACHABLE".into()),
        _ => None,
    };
    {
        let mut cache = CACHE.lock().map_err(|_| "COCKPIT_REMOTE_BUSY")?;
        if cache.len() >= 1024 {
            cache.retain(|_, entry| entry.checked.elapsed() < Duration::from_secs(300));
        }
        cache.insert(
            id.clone(),
            Cached {
                value: value.clone(),
                checked: std::time::Instant::now(),
            },
        );
    }
    if let Some(value) = value.clone() {
        tokio::task::spawn_blocking(move || {
            if let Some(path) = cache_path(&id) {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                let text = serde_json::to_string(&value).map_err(|error| error.to_string())?;
                super::atomic_write::write_string_atomic(&path, &text)?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| "COCKPIT_REMOTE_CACHE_FAILED")??;
    }
    Ok(value)
}

pub async fn usage(base: &str, key: &str, range: &str) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|_| "COCKPIT_REMOTE_CLIENT_FAILED")?;
    let mut url = endpoint(base, "usage")?;
    url.query_pairs_mut().append_pair("range", range);
    tokio::time::timeout(Duration::from_secs(4), get_json(&client, url, key))
        .await
        .map_err(|_| "COCKPIT_REMOTE_UNREACHABLE")??
        .ok_or_else(|| "COCKPIT_REMOTE_USAGE_UNAVAILABLE".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_reverse_proxy_prefix_and_separates_credentials() {
        assert_eq!(
            endpoint("http://192.168.1.2:1234/team/v1/", "connection")
                .unwrap()
                .as_str(),
            "http://192.168.1.2:1234/team/v1/cockpit/connection"
        );
        assert_eq!(
            endpoint("https://example.com", "usage").unwrap().path(),
            "/v1/cockpit/usage"
        );
        assert_ne!(
            fingerprint("http://server/v1", "one"),
            fingerprint("http://server/v1", "two")
        );
        assert!(endpoint("https://user:secret@example.com", "usage").is_err());
    }
}
