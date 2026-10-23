//! Version advertised by our Grok protocol implementation, independent of an
//! installed CLI executable. Refreshes never block a request or carry credentials.
use futures::StreamExt;
use std::{
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

pub(crate) const FALLBACK: &str = "1.0.52";
const FLOOR: [u64; 3] = [1, 0, 13];
const REFRESH_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BODY: usize = 1024 * 1024;
const REGISTRY_URL: &str = "https://registry.npmjs.org/@xai-official/grok/latest";

#[derive(Default)]
struct Entry {
    version: Option<String>,
    checked: Option<Instant>,
    refreshing: bool,
}
#[derive(Default)]
struct Cache(Mutex<Entry>);
static CACHE: LazyLock<Arc<Cache>> = LazyLock::new(|| Arc::new(Cache::default()));

fn acceptable(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    let Ok(parts) = parts
        .into_iter()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    parts.as_slice() >= FLOOR.as_slice()
}

impl Cache {
    fn current(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .version
            .clone()
            .filter(|value| acceptable(value))
            .unwrap_or_else(|| FALLBACK.into())
    }
    fn begin(&self, now: Instant) -> bool {
        let mut entry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if entry.refreshing
            || entry
                .checked
                .is_some_and(|last| now.saturating_duration_since(last) < REFRESH_INTERVAL)
        {
            return false;
        }
        entry.refreshing = true;
        entry.checked = Some(now);
        true
    }
    fn finish(&self, version: Option<String>) {
        let mut entry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(version) = version.filter(|version| acceptable(version)) {
            entry.version = Some(version);
        }
        entry.refreshing = false;
    }
}

pub(crate) fn cached() -> String {
    CACHE.current()
}

pub(crate) fn resolve(candidate: Option<&str>) -> String {
    candidate
        .map(str::trim)
        .filter(|value| acceptable(value))
        .map(str::to_owned)
        .unwrap_or_else(cached)
}

/// Use the selected route's proxy/TLS configuration; it has no default OAuth
/// headers. No account data is sent to the public package registry.
pub(crate) fn refresh_in_background(client: reqwest::Client) {
    start_refresh(CACHE.clone(), client, REGISTRY_URL.to_owned());
}

struct RefreshGuard(Arc<Cache>);
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        self.0.finish(None);
    }
}

fn start_refresh(cache: Arc<Cache>, client: reqwest::Client, url: String) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    if !cache.begin(Instant::now()) {
        return;
    }
    runtime.spawn(async move {
        let _guard = RefreshGuard(cache.clone());
        let version = match tokio::time::timeout(FETCH_TIMEOUT, fetch(&client, &url)).await {
            Ok(Ok(version)) => {
                if cache.current() != version {
                    crate::modules::logger::log_info(&format!("[Grok] API protocol version updated to {version}"));
                }
                Some(version)
            }
            Ok(Err(())) => {
                crate::modules::logger::log_warn("[Grok] Version registry unavailable or invalid; keeping the cached API protocol version");
                None
            }
            Err(_) => {
                crate::modules::logger::log_warn("[Grok] Version registry timed out; keeping the cached API protocol version");
                None
            }
        };
        cache.finish(version);
    });
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<String, ()> {
    let response = client
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, "Cockpit-Tools")
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|len| len > MAX_BODY as u64)
    {
        return Err(());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
    let version = value
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| acceptable(value))
        .ok_or(())?;
    Ok(version.to_owned())
}

#[cfg(test)]
#[path = "grok_client_version_tests.rs"]
mod tests;
