use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::Instant,
};

const MAX_ENTRIES: usize = 16;

struct Entry {
    fetched_at: Instant,
    models: Arc<[String]>,
}

type Fingerprint = [u8; 32];
static MODELS: LazyLock<Mutex<HashMap<Fingerprint, Entry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn fingerprint(base_url: &str, api_key: &str, protocol: &str, auth_scheme: Option<&str>) -> Fingerprint {
    let mut hash = Sha256::new();
    for value in [base_url.trim().trim_end_matches('/'), api_key.trim(), protocol, auth_scheme.unwrap_or_default()] {
        hash.update(value.len().to_le_bytes());
        hash.update(value.as_bytes());
    }
    hash.finalize().into()
}

pub(crate) fn remember(
    base_url: &str,
    api_key: &str,
    protocol: &str,
    auth_scheme: Option<&str>,
    models: Vec<String>,
) -> Result<(), String> {
    if models.is_empty() {
        return Ok(());
    }
    let bytes = models.iter().map(|model| model.len() + size_of::<String>()).sum::<usize>();
    if bytes > super::codex_model_catalog::MAX_CATALOG_BYTES {
        return Err("PROVIDER_MODELS_PARSE_FAILED".to_string());
    }
    let id = fingerprint(base_url, api_key, protocol, auth_scheme);
    let mut entries = MODELS.lock().unwrap_or_else(|error| error.into_inner());
    if entries.len() >= MAX_ENTRIES && !entries.contains_key(&id) {
        if let Some(oldest) = entries.iter().min_by_key(|(_, entry)| entry.fetched_at).map(|(id, _)| *id) {
            entries.remove(&oldest);
        }
    }
    entries.insert(id, Entry { fetched_at: Instant::now(), models: models.into() });
    Ok(())
}

pub(crate) fn get(base_url: &str, api_key: &str, protocol: &str, auth_scheme: Option<&str>) -> Option<Arc<[String]>> {
    let id = fingerprint(base_url, api_key, protocol, auth_scheme);
    MODELS.lock().unwrap_or_else(|error| error.into_inner()).get(&id)
        .map(|entry| Arc::clone(&entry.models))
}
