//! Codex drops Claude-specific search-result fields from saved tool history.
//! Like CPA, retain the original result blocks in a bounded, session-scoped cache.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Cache {
    inner: Mutex<Entries>,
}
#[derive(Default)]
struct Entries {
    items: HashMap<(String, String), Entry>,
    bytes: usize,
}
struct Entry {
    value: Arc<Value>,
    created: Instant,
    bytes: usize,
}
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 1024;
const TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub(super) fn scope(prepared: &super::mapping::Prepared) -> Option<String> {
    let legacy = prepared
        .source_headers
        .get("session_id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 4096);
    // Legacy Codex sends session_id, while routing admission uses canonical
    // metadata/session-id. This alias is only a cache key, never routing proof.
    let thread = prepared.request.thread_id.as_deref();
    let session = if thread != prepared.request.session_id.as_deref() {
        thread
    } else {
        legacy.or(thread).or(prepared.request.session_id.as_deref())
    }?;
    let mut hash = Sha256::new();
    for value in [
        prepared.caller.id.as_str(),
        prepared.route.account_id.as_str(),
        prepared.route.base_url.as_str(),
        prepared.route.model.as_str(),
        session,
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    for name in ["authorization", "x-api-key"] {
        if let Some(value) = prepared.route.headers.get(name) {
            hash.update(value.as_bytes());
        }
    }
    Some(format!("{:x}", hash.finalize()))
}

impl Cache {
    pub(super) fn restore(&self, scope: Option<&str>, body: &mut Value) {
        let Some(scope) = scope else { return };
        let Some(input) = body["input"].as_array_mut() else {
            return;
        };
        for item in input
            .iter_mut()
            .filter(|item| item["type"] == "web_search_call")
        {
            if item.get("results").is_some_and(|value| {
                !value.is_null() && !value.as_array().is_some_and(Vec::is_empty)
            }) {
                continue;
            }
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let value = {
                let entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                entries
                    .items
                    .get(&(scope.to_owned(), id.to_owned()))
                    .filter(|entry| entry.created.elapsed() < TTL)
                    .map(|entry| entry.value.clone())
            };
            if let Some(value) = value {
                item["results"] = (*value).clone()
            }
        }
    }

    pub(super) fn record(&self, scope: Option<&str>, response: &Value) {
        let Some(scope) = scope else { return };
        for item in response["output"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "web_search_call")
        {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let Some(value) = item.get("results").filter(|value| {
                value.is_object() || value.as_array().is_some_and(|values| !values.is_empty())
            }) else {
                continue;
            };
            let bytes = value.to_string().len();
            if bytes > MAX_BYTES {
                continue;
            }
            let value = Arc::new(value.clone());
            let mut entries = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let key = (scope.to_owned(), id.to_owned());
            if let Some(previous) = entries.items.remove(&key) {
                entries.bytes = entries.bytes.saturating_sub(previous.bytes)
            }
            while entries.items.len() >= MAX_ENTRIES
                || entries.bytes.saturating_add(bytes) > MAX_BYTES
            {
                let Some(oldest) = entries
                    .items
                    .iter()
                    .min_by_key(|(_, entry)| entry.created)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                if let Some(entry) = entries.items.remove(&oldest) {
                    entries.bytes = entries.bytes.saturating_sub(entry.bytes)
                }
            }
            entries.bytes += bytes;
            entries.items.insert(
                key,
                Entry {
                    value,
                    created: Instant::now(),
                    bytes,
                },
            );
        }
    }
}
