use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, LazyLock, OnceLock, RwLock, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::Duration;

use reqwest::Client;
use serde_json::{Map, Value};
use tokio::sync::{Mutex, OnceCell};

use super::{
    atomic_write, codex_account::is_retired_gpt55_model, codex_wakeup::is_codex_model_before_5_5,
    codex_model_prices::{self, PriceBook}, config, logger,
};

pub(crate) const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const SOURCE_TIMEOUT: Duration = Duration::from_secs(8);
const PRIMARY_URL: &str = "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/codex_client_models.json";
const SECONDARY_URL: &str = "https://models.router-for.me/codex_client_models.json";
const PROJECT_MODELS_URL: &str = "https://raw.githubusercontent.com/jlcodes99/cockpit-tools/main/resources/codex/codex_client_models.json";
const PRICES_URL: &str = "https://raw.githubusercontent.com/jlcodes99/cockpit-tools/main/resources/codex/model_prices.json";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub(crate) const GPT_6_2_SOL_MODEL_ID: &str = "gpt-6.2-sol";
pub(crate) const GPT_6_2_ASTRA_MODEL_ID: &str = "gpt-6.2-astra";
pub(crate) const GPT_6_2_MODELS: [&str; 2] = [GPT_6_2_SOL_MODEL_ID, GPT_6_2_ASTRA_MODEL_ID];
pub(crate) const RECOMMENDED_FRONTIER_MODELS: [&str; 7] = [
    "gpt-6.1-sol",
    GPT_6_2_SOL_MODEL_ID,
    GPT_6_2_ASTRA_MODEL_ID,
    "gpt-6.1-terra",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
];
const RECOMMENDED_5_6_MODELS: [&str; 3] = ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"];
const EMBEDDED_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../resources/codex/codex_client_models.json"
));

static EMBEDDED: OnceLock<Arc<Value>> = OnceLock::new();
static ACTIVE: LazyLock<RwLock<CatalogSnapshot>> = LazyLock::new(|| RwLock::new(CatalogSnapshot {
    models: embedded(),
    prices: codex_model_prices::bundled(),
    source: CatalogSource::Embedded,
}));
static CACHE_LOADED: OnceCell<()> = OnceCell::const_new();
static WORKER_STARTED: AtomicBool = AtomicBool::new(false);
static REFRESH_GENERATION: AtomicU64 = AtomicU64::new(0);
static REFRESH: Mutex<Option<RefreshReport>> = Mutex::const_new(None);

struct CatalogSnapshot {
    models: Arc<Value>,
    prices: Arc<PriceBook>,
    source: CatalogSource,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RefreshReport {
    models_updated: bool,
    prices_updated: bool,
    model_error: Option<String>,
    price_error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CatalogSource {
    Downloaded,
    Cache,
    Embedded,
}

pub(crate) fn source() -> CatalogSource {
    ACTIVE.read().unwrap_or_else(|err| err.into_inner()).source
}

fn embedded() -> Arc<Value> {
    EMBEDDED
        .get_or_init(|| {
            let value = serde_json::from_str(EMBEDDED_JSON)
                .expect("embedded Codex model catalog must be JSON");
            validate_catalog(&value).expect("embedded Codex model catalog must be valid");
            Arc::new(value)
        })
        .clone()
}

pub(crate) fn snapshot() -> Arc<Value> {
    #[cfg(test)]
    if let Some(catalog) = TEST_CATALOG.with(|value| value.borrow().clone()) {
        return catalog;
    }
    ACTIVE.read().unwrap_or_else(|err| err.into_inner()).models.clone()
}

pub(crate) fn prices() -> Arc<PriceBook> {
    ACTIVE.read().unwrap_or_else(|err| err.into_inner()).prices.clone()
}

pub(crate) async fn initialize() {
    if WORKER_STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    loop {
        refresh().await;
        tokio::time::sleep(REFRESH_INTERVAL).await;
    }
}

fn cache_paths() -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = config::get_shared_dir();
    (dir.join("codex-client-models.cache.json"), dir.join("codex-model-prices.cache.json"))
}

async fn load_cached_data() {
    CACHE_LOADED.get_or_init(|| async {
        let changed = tokio::task::spawn_blocking(|| {
            let (models_path, prices_path) = cache_paths();
            let models = read_cache(&models_path);
            let prices = read_cache_bytes(&prices_path).and_then(|data| codex_model_prices::parse_remote(&data));
            let mut active = ACTIVE.write().unwrap_or_else(|err| err.into_inner());
            let mut changed = false;
            if let Ok(models) = models {
                changed = active.models.as_ref() != &models;
                active.models = Arc::new(models);
                active.source = CatalogSource::Cache;
            }
            if let Ok(prices) = prices {
                if prices.revision > active.prices.revision {
                    active.prices = Arc::new(prices);
                    changed = true;
                }
            }
            changed
        }).await.unwrap_or(false);
        if changed {
            super::codex_local_access::emit_local_access_state_updated();
        }
    }).await;
}

pub(crate) async fn refresh() -> RefreshReport {
    load_cached_data().await;
    let generation = REFRESH_GENERATION.load(Ordering::Acquire);
    let mut last = REFRESH.lock().await;
    if REFRESH_GENERATION.load(Ordering::Relaxed) != generation {
        if let Some(report) = last.as_ref() {
            return report.clone();
        }
    }
    let report = refresh_remote().await;
    if report.model_error.is_some() || report.price_error.is_some() {
        logger::log_warn(&format!("[Codex catalog] Refresh status: models={:?}, prices={:?}",
            report.model_error, report.price_error));
    }
    *last = Some(report.clone());
    REFRESH_GENERATION.fetch_add(1, Ordering::Release);
    if report.models_updated || report.prices_updated {
        super::codex_local_access::emit_local_access_state_updated();
    }
    report
}

async fn refresh_remote() -> RefreshReport {
    let client = Client::builder().https_only(true)
        .redirect(reqwest::redirect::Policy::limited(3))
        .timeout(SOURCE_TIMEOUT).connect_timeout(Duration::from_secs(4)).build();
    let client = match client {
        Ok(client) => client,
        Err(error) => return RefreshReport {
            models_updated: false, prices_updated: false,
            model_error: Some(error.to_string()), price_error: Some(error.to_string()),
        },
    };
    let (models, price_bytes) = tokio::join!(
        fetch_configured_catalog(&client),
        fetch_bytes(&client, PRICES_URL, SOURCE_TIMEOUT),
    );
    let result = tokio::task::spawn_blocking(move || {
        let prices = price_bytes.and_then(|bytes| codex_model_prices::parse_remote(&bytes).map(|book| (book, bytes)));
        let (models_path, prices_path) = cache_paths();
        let report = activate_remote(&ACTIVE, &models_path, &prices_path, models, prices);
        if report.models_updated {
            super::codex_local_access::refresh_api_service_experimental_model_ids();
        }
        report
    }).await;
    result.unwrap_or_else(|error| RefreshReport {
        models_updated: false, prices_updated: false,
        model_error: Some(error.to_string()), price_error: Some(error.to_string()),
    })
}

async fn fetch_configured_catalog(client: &Client) -> Option<Value> {
    let urls: &[&str] = if super::remote_config::prefers_project_codex_model_catalog().await {
        &[PROJECT_MODELS_URL, PRIMARY_URL, SECONDARY_URL]
    } else {
        &[PRIMARY_URL, SECONDARY_URL]
    };
    fetch_catalog(client, urls, SOURCE_TIMEOUT).await
}

fn activate_remote(
    active: &RwLock<CatalogSnapshot>,
    models_path: &Path,
    prices_path: &Path,
    models: Option<Value>,
    prices: Result<(PriceBook, Vec<u8>), String>,
) -> RefreshReport {
    let (current_models, current_revision) = {
        let active = active.read().unwrap_or_else(|err| err.into_inner());
        (Arc::clone(&active.models), active.prices.revision)
    };
    let mut report = RefreshReport {
        models_updated: false, prices_updated: false,
        model_error: models.is_none().then(|| "Model catalog sources unavailable".into()),
        price_error: None,
    };
    let downloaded = models.is_some();
    let models = models.filter(|value| value != current_models.as_ref()).map(|value| {
        if let Ok(content) = serde_json::to_string(&value) {
            save_cache(models_path, &content);
        }
        Arc::new(value)
    });
    let prices = match prices {
        Ok((book, bytes)) if book.revision > current_revision => {
            if let Ok(content) = std::str::from_utf8(&bytes) {
                save_cache(prices_path, content);
            }
            Some(Arc::new(book))
        }
        Ok(_) => None,
        Err(error) => {
            report.price_error = Some(error);
            None
        }
    };
    // Change both sources as a unit; a failed source keeps its last good value.
    let mut active = active.write().unwrap_or_else(|err| err.into_inner());
    if downloaded {
        active.source = CatalogSource::Downloaded;
    }
    if let Some(models) = models {
        active.models = models;
        report.models_updated = true;
    }
    if let Some(prices) = prices {
        active.prices = prices;
        report.prices_updated = true;
    }
    report
}

fn save_cache(path: &Path, content: &str) {
    if let Err(error) = atomic_write::write_string_atomic(path, content) {
        logger::log_warn(&format!("[Codex catalog] Failed to persist {:?}: {error}", path.file_name()));
    }
}

#[cfg(test)]
#[cfg(test)]
async fn load_startup_catalog(
    client: Option<&Client>,
    cache_path: &Path,
    urls: &[&str],
    timeout: Duration,
) -> (Arc<Value>, CatalogSource) {
    if let Some(client) = client {
        if let Some(catalog) = fetch_catalog(client, urls, timeout).await {
            let content =
                serde_json::to_string(&catalog).expect("validated JSON catalog must serialize");
            if let Err(error) = atomic_write::write_string_atomic(cache_path, &content) {
                logger::log_warn(&format!(
                    "[Codex model catalog] Cache persistence failed; using fetched snapshot: {error}"
                ));
            }
            return (Arc::new(catalog), CatalogSource::Downloaded);
        }
    }
    match read_cache(cache_path) {
        Ok(catalog) => (Arc::new(catalog), CatalogSource::Cache),
        Err(error) => {
            if cache_path.exists() {
                logger::log_warn(&format!(
                    "[Codex model catalog] Cache rejected; using embedded snapshot: {error}"
                ));
            }
            (embedded(), CatalogSource::Embedded)
        }
    }
}

fn read_cache(path: &Path) -> Result<Value, String> {
    parse_and_merge(&read_cache_bytes(path)?)
}

fn read_cache_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_CATALOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err("Codex catalog exceeds size limit".into());
    }
    Ok(bytes)
}

async fn fetch_bytes(client: &Client, url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    tokio::time::timeout(timeout, async {
            let mut response = client
                .get(url)
                .send()
                .await
                .map_err(|error| error.to_string())?;
            if response.status() != reqwest::StatusCode::OK {
                return Err(format!("HTTP {}", response.status()));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
                if chunk.len() > MAX_CATALOG_BYTES - bytes.len() {
                    return Err("catalog exceeds 8 MiB".to_string());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
    }).await.map_err(|_| "Codex catalog request timed out".to_string())?
}

async fn fetch_catalog(client: &Client, urls: &[&str], timeout: Duration) -> Option<Value> {
    for url in urls {
        let result = match fetch_bytes(client, url, timeout).await {
            Ok(bytes) => tokio::task::spawn_blocking(move || parse_and_merge(&bytes)).await
                .map_err(|error| error.to_string()).and_then(|parsed| parsed),
            Err(error) => Err(error),
        };
        match result {
            Ok(catalog) => return Some(catalog),
            Err(error) => logger::log_warn(&format!(
                "[Codex model catalog] Rejected source {url}: {error}"
            )),
        }
    }
    None
}

fn parse_and_merge(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err("catalog exceeds 8 MiB".to_string());
    }
    let mut catalog: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    validate_catalog(&catalog)?;
    normalize_integer_fields(&mut catalog);
    let models = catalog["models"].as_array_mut().unwrap();
    models.retain(|model| !is_codex_model_before_5_5(model["slug"].as_str().unwrap()));
    let present: HashSet<String> = models
        .iter()
        .map(|model| normalized_id(model["slug"].as_str().unwrap()))
        .collect();
    let defaults = embedded();
    if let Some(sol) = models.iter_mut().find(|model| uses_legacy_sol_context(model)) {
        let current = defaults["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol")
            .expect("embedded catalog includes GPT-6.1 Sol");
        sol["context_window"] = current["context_window"].clone();
        sol["auto_compact_token_limit"] = current["auto_compact_token_limit"].clone();
    }
    for slug in RECOMMENDED_FRONTIER_MODELS {
        if !present.contains(slug) {
            let pinned = defaults["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| normalized_id(model["slug"].as_str().unwrap()) == slug)
                .ok_or_else(|| format!("embedded catalog missing pinned model {slug}"))?;
            models.push(pinned.clone());
        }
    }
    // Older startup catalogs predate announcement of Ultrafast for both 6.2 models.
    for model in models.iter_mut().filter(|model| {
        model["slug"].as_str().is_some_and(|slug| {
            GPT_6_2_MODELS.iter().any(|id| slug.trim().eq_ignore_ascii_case(id))
        })
    }) {
        for (field, tier) in [
            ("additional_speed_tiers", Value::from("ultrafast")),
            ("service_tiers", serde_json::json!({"id":"ultrafast","name":"Ultrafast","description":""})),
        ] {
            if model[field].is_null() {
                model[field] = serde_json::json!([]);
            }
            if let Some(tiers) = model[field].as_array_mut() {
                let exists = tiers.iter().any(|value| value == "ultrafast" || value["id"] == "ultrafast");
                if !exists {
                    tiers.push(tier);
                }
            }
        }
    }
    if let Some(overrides) = catalog
        .get_mut("model_overrides")
        .and_then(Value::as_array_mut)
    {
        overrides.retain(|model| !is_codex_model_before_5_5(model["slug"].as_str().unwrap()));
    }
    validate_catalog(&catalog)?;
    if serde_json::to_vec(&catalog)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_CATALOG_BYTES
    {
        return Err("merged catalog exceeds 8 MiB".to_string());
    }
    Ok(catalog)
}

pub(crate) fn uses_legacy_sol_context(model: &Value) -> bool {
    model["slug"]
        .as_str()
        .is_some_and(|slug| slug.eq_ignore_ascii_case("gpt-6.1-sol"))
        && model["context_window"].as_i64() == Some(272_000)
        && model["max_context_window"].as_i64() == Some(872_000)
        && (model["auto_compact_token_limit"].is_null()
            || integer_value(&model["auto_compact_token_limit"]) == Some(244_800))
}

fn normalized_id(id: &str) -> String {
    id.trim().to_ascii_lowercase()
}

fn required_string<'a>(model: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    model
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("field {field} must be a non-empty string"))
}

fn normalize_integer_fields(catalog: &mut Value) {
    for array in ["models", "model_overrides"] {
        if let Some(models) = catalog.get_mut(array).and_then(Value::as_array_mut) {
            for model in models {
                for field in ["context_window", "max_context_window", "priority"] {
                    if let Some(value) = model.get_mut(field) {
                        *value = Value::from(integer_value(value).unwrap());
                    }
                }
            }
        }
    }
}

fn integer_value(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        let value = value.as_f64()?;
        if value.is_finite()
            && value.fract() == 0.0
            && value >= 0.0
            && value < 9_223_372_036_854_775_808.0
        {
            Some(value as i64)
        } else {
            None
        }
    })
}

fn required_integer(
    model: &Map<String, Value>,
    field: &str,
    positive: bool,
) -> Result<i64, String> {
    let value = model
        .get(field)
        .and_then(integer_value)
        .ok_or_else(|| format!("field {field} must be an integer"))?;
    if value < 0 || (positive && value == 0) {
        return Err(format!(
            "field {field} must be {}",
            if positive { "positive" } else { "nonnegative" }
        ));
    }
    Ok(value)
}

fn validate_model(model: &Map<String, Value>, partial: bool) -> Result<(), String> {
    let slug = required_string(model, "slug")?;
    if model["slug"].as_str() != Some(slug) {
        return Err("model slug must not contain surrounding whitespace".to_string());
    }
    for field in [
        "slug",
        "display_name",
        "description",
        "minimal_client_version",
        "visibility",
        "default_reasoning_level",
    ] {
        if !partial || field == "slug" || model.contains_key(field) {
            required_string(model, field)?;
        }
    }
    if model.get("base_instructions").is_some_and(|value| !value.is_null()) {
        required_string(model, "base_instructions")?;
    } else if !partial && !model.get("model_messages")
        .and_then(|value| value.get("instructions_template"))
        .and_then(Value::as_str)
        .is_some_and(|text| !text.trim().is_empty())
    {
        return Err("model requires base_instructions or a model_messages instructions_template".into());
    }
    for field in ["context_window", "max_context_window", "priority"] {
        if !partial || model.contains_key(field) {
            required_integer(model, field, field != "priority")?;
        }
    }
    if let (Some(context), Some(max)) =
        (model.get("context_window"), model.get("max_context_window"))
    {
        if integer_value(context).unwrap() > integer_value(max).unwrap() {
            return Err("context_window exceeds max_context_window".to_string());
        }
    }
    if model
        .get("supported_in_api")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("supported_in_api must be a boolean".to_string());
    }
    if !partial || model.contains_key("supported_reasoning_levels") {
        let levels = model
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .filter(|levels| !levels.is_empty())
            .ok_or("supported_reasoning_levels must be a non-empty array")?;
        let mut seen = HashSet::new();
        for level in levels {
            let level = level
                .as_object()
                .ok_or("reasoning level must be an object")?;
            let effort = required_string(level, "effort")?;
            if !seen.insert(effort) {
                return Err(format!("duplicate reasoning effort {effort}"));
            }
        }
        if let Some(default) = model.get("default_reasoning_level") {
            if !seen.contains(default.as_str().unwrap().trim()) {
                return Err("default_reasoning_level is not supported".to_string());
            }
        }
    }
    Ok(())
}

/// 校验目录结构。不要求任何特定型号存在：上游会下线或改名模型，兜底模板由
/// 协议层从现有条目中挑选，因此这里只校验各条目自身是否完整。
fn validate_catalog(catalog: &Value) -> Result<(), String> {
    let document = catalog.as_object().ok_or("catalog must be an object")?;
    let models = document
        .get("models")
        .and_then(Value::as_array)
        .filter(|models| !models.is_empty())
        .ok_or("catalog models must be a non-empty array")?;
    let mut by_id = HashMap::new();
    for model in models {
        let model = model.as_object().ok_or("model must be an object")?;
        validate_model(model, false)?;
        let id = normalized_id(required_string(model, "slug")?);
        if by_id.insert(id.clone(), model).is_some() {
            return Err(format!("duplicate model slug {id}"));
        }
    }
    if let Some(overrides) = document.get("model_overrides") {
        let overrides = overrides
            .as_array()
            .ok_or("model_overrides must be an array")?;
        let mut seen = HashSet::new();
        for model in overrides {
            let model = model
                .as_object()
                .ok_or("model override must be an object")?;
            validate_model(model, true)?;
            let id = normalized_id(required_string(model, "slug")?);
            if !seen.insert(id.clone()) {
                return Err(format!("duplicate model override slug {id}"));
            }
            // 覆盖项可以指向目录里没有的型号；此时拿任意一个完整条目做基底，
            // 只用于补全未声明的必填字段，不依赖具体型号存在。
            let base = by_id
                .get(&id)
                .or_else(|| by_id.get("gpt-5.5"))
                .or_else(|| by_id.values().next())
                .ok_or("catalog has no models to validate overrides against")?;
            let mut combined = (*base).clone();
            combined.extend(model.clone());
            validate_model(&combined, false)?;
        }
    }
    Ok(())
}

pub(crate) fn recommended_models() -> Vec<(String, String)> {
    recommended_models_from_catalog(&snapshot())
}

pub(crate) fn bundled_recommended_models() -> Vec<(String, String)> {
    recommended_models_from_catalog(&embedded())
}

pub(crate) fn bundled_model_supports_vision(model_id: &str) -> Option<bool> {
    let catalog = embedded();
    let model = catalog["models"].as_array()?.iter().find(|model| model["slug"] == model_id)?;
    Some(model["input_modalities"].as_array().is_some_and(|values| values.iter().any(|value| value == "image")))
}

pub(crate) fn recommended_model_rank(id: &str) -> Option<usize> {
    RECOMMENDED_FRONTIER_MODELS
        .into_iter()
        .chain(RECOMMENDED_5_6_MODELS)
        .position(|model| model.eq_ignore_ascii_case(id.trim()))
}

fn recommended_models_from_catalog(catalog: &Value) -> Vec<(String, String)> {
    let Some(models) = catalog.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut recommended = Vec::new();
    for model in models {
        let Some(id) = model.get("slug").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        let normalized = normalized_id(id);
        if model.get("supported_in_api").and_then(Value::as_bool) != Some(true)
            || model.get("visibility").and_then(Value::as_str) != Some("list")
            || !(normalized.starts_with("gpt-") || normalized.starts_with("codex-"))
            || matches!(normalized.as_str(), "gpt-reserve" | "codex-auto-review")
            || normalized.contains("image")
            || is_retired_gpt55_model(id)
            || is_codex_model_before_5_5(id)
        {
            continue;
        }
        let Some(name) = model.get("display_name").and_then(Value::as_str) else {
            continue;
        };
        recommended.push((
            model
                .get("priority")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MAX),
            id.to_string(),
            name.to_string(),
        ));
    }
    recommended.sort_by_key(|(priority, id, _)| (
        recommended_model_rank(id).unwrap_or(usize::MAX),
        *priority,
    ));
    recommended
        .into_iter()
        .map(|(_, id, name)| (id, name))
        .collect()
}

pub(crate) fn is_recommended_model(id: &str) -> bool {
    recommended_models()
        .iter()
        .any(|model| model.0.eq_ignore_ascii_case(id.trim()))
}

#[cfg(test)]
thread_local! {
    static TEST_CATALOG: std::cell::RefCell<Option<Arc<Value>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn parse_and_merge_for_test(bytes: &[u8]) -> Result<Value, String> {
    parse_and_merge(bytes)
}

#[cfg(test)]
pub(crate) fn with_test_catalog<T>(catalog: Value, f: impl FnOnce() -> T) -> T {
    struct Reset(Option<Arc<Value>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CATALOG.with(|value| *value.borrow_mut() = self.0.take());
        }
    }
    let previous = TEST_CATALOG.with(|value| value.replace(Some(Arc::new(catalog))));
    let _reset = Reset(previous);
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("codex_catalog_{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn cache(&self) -> PathBuf {
            self.0.join("codex-client-models.cache.json")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn template(slug: &str) -> Value {
        // 夹具模板来源不绑定具体型号：兜底型号会随上游变化。显示名按 slug 重写，
        // 避免沿用源条目的名字造成断言困惑。
        let catalog = embedded();
        let models = catalog["models"].as_array().unwrap();
        let mut model = models
            .iter()
            .find(|model| model["slug"] == "gpt-6.1-sol")
            .or_else(|| models.first())
            .unwrap()
            .clone();
        model["slug"] = json!(slug);
        model["display_name"] = json!(slug);
        model
    }

    /// 上游会下线型号（例如 gpt-5.5 在 2026-10 一度从目录里消失），缺兜底型号
    /// 不能导致整份目录被拒，否则刷新会永远失败并停在旧目录上。
    #[test]
    fn remote_catalog_without_fallback_model_is_still_accepted() {
        let mut catalog = remote();
        catalog["models"]
            .as_array_mut()
            .unwrap()
            .retain(|model| model["slug"] != "gpt-5.5");
        let merged = parse(&catalog).expect("catalog without gpt-5.5 must still merge");
        let slugs: Vec<&str> = merged["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["slug"].as_str())
            .collect();
        assert!(slugs.contains(&"gpt-6.2-sol"));
        assert!(
            !slugs.contains(&"gpt-5.5"),
            "missing fallback must not be resurrected"
        );
        assert!(
            !crate::modules::codex_protocol::build_codex_client_models_response(&[
                "gpt-6.2-sol".to_string()
            ])["models"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    fn remote() -> Value {
        let mut future = template("gpt-6.2-sol");
        future["display_name"] = json!("Remote GPT-6.2 Sol");
        future["base_instructions"] = json!("Updated remote instructions");
        future["context_window"] = json!(516_000);
        future["max_context_window"] = json!(1_032_000);
        future["priority"] = json!(0);
        future["vendor_metadata"] = json!({"keep": [1, 2, 3]});
        json!({
            "models": [template("gpt-5.5"), future],
            "model_overrides": [{"slug": "gpt-6.2-sol", "description": "Remote override", "opaque": {"keep": true}}],
            "catalog_metadata": {"revision": "future"}
        })
    }

    fn parse(value: &Value) -> Result<Value, String> {
        parse_and_merge(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn sol_catalog_upgrades_the_legacy_default_and_keeps_other_source_limits() {
        for (source_window, source_compaction, expected_window, expected_compaction) in [
            (272_000, Value::Null, 500_000, json!(450_000)),
            (272_000, json!(244_800), 500_000, json!(450_000)),
            (272_000, json!(210_000), 272_000, json!(210_000)),
            (516_000, json!(460_000), 516_000, json!(460_000)),
        ] {
            let mut sol = template("gpt-6.1-sol");
            sol["context_window"] = json!(source_window);
            sol["auto_compact_token_limit"] = source_compaction;
            let merged = parse(&json!({"models": [sol]})).unwrap();
            assert_eq!(merged["models"][0]["context_window"], expected_window);
            assert_eq!(merged["models"][0]["max_context_window"], 872_000);
            assert_eq!(merged["models"][0]["auto_compact_token_limit"], expected_compaction);
        }
    }

    fn local_client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    async fn mock_response(body: Vec<u8>, headers: &str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/catalog", listener.local_addr().unwrap());
        let headers = headers.to_string();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0u8; 1024];
                let read = socket.read(&mut buf).await.unwrap();
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buf[..read]);
                if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 16_384);
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n{}\r\n",
                body.len(),
                headers
            );
            if socket.write_all(header.as_bytes()).await.is_ok() {
                let _ = socket.write_all(&body).await;
            }
        });
        (url, task)
    }

    #[test]
    fn embedded_is_valid_and_remote_metadata_and_overrides_are_preserved() {
        validate_catalog(&embedded()).unwrap();
        let merged = parse(&remote()).unwrap();
        let future = merged["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-6.2-sol")
            .unwrap();
        assert_eq!(future["display_name"], "Remote GPT-6.2 Sol");
        assert_eq!(future["base_instructions"], "Updated remote instructions");
        assert_eq!(future["context_window"], 516_000);
        assert_eq!(future["max_context_window"], 1_032_000);
        assert_eq!(future["vendor_metadata"], json!({"keep": [1, 2, 3]}));
        assert_eq!(merged["model_overrides"], remote()["model_overrides"]);
        assert_eq!(merged["catalog_metadata"], remote()["catalog_metadata"]);
        for slug in RECOMMENDED_FRONTIER_MODELS {
            assert!(merged["models"]
                .as_array()
                .unwrap()
                .iter()
                .any(|model| model["slug"] == slug));
        }
    }

    #[test]
    fn remote_pinned_model_wins_and_retired_models_and_overrides_are_removed() {
        let mut catalog = remote();
        let mut pinned = template("GPT-6.1-SOL");
        pinned["display_name"] = json!("Remote pinned name");
        pinned["base_instructions"] = json!("Remote pinned instructions");
        catalog["models"].as_array_mut().unwrap().extend([
            pinned.clone(),
            template("gpt-5.4"),
            template("gpt-5-codex"),
        ]);
        catalog["model_overrides"]
            .as_array_mut()
            .unwrap()
            .push(json!({"slug":"gpt-5.4", "description":"Retired"}));
        let merged = parse(&catalog).unwrap();
        let models = merged["models"].as_array().unwrap();
        assert!(models.contains(&pinned));
        assert_eq!(
            models
                .iter()
                .filter(|model| model["slug"]
                    .as_str()
                    .unwrap()
                    .eq_ignore_ascii_case("gpt-6.1-sol"))
                .count(),
            1
        );
        assert!(!models
            .iter()
            .any(|model| is_codex_model_before_5_5(model["slug"].as_str().unwrap())));
        assert_eq!(merged["model_overrides"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rejects_incomplete_duplicates_and_bad_reasoning_or_window_fields() {
        for field in [
            "slug",
            "display_name",
            "description",
            "base_instructions",
            "minimal_client_version",
            "visibility",
            "default_reasoning_level",
        ] {
            for invalid in [Value::Null, json!("  "), json!(42)] {
                let mut catalog = remote();
                if field == "base_instructions" && invalid.is_null() {
                    catalog["models"][1].as_object_mut().unwrap().remove("model_messages");
                }
                catalog["models"][1][field] = invalid;
                assert!(parse(&catalog).is_err(), "{field}");
            }
        }
        for (field, invalid) in [
            ("context_window", json!(0)),
            ("context_window", json!(1.5)),
            ("max_context_window", json!(-1)),
            ("max_context_window", json!(100)),
            ("priority", json!(-1)),
            ("priority", json!(0.5)),
            ("supported_reasoning_levels", json!([])),
            (
                "supported_reasoning_levels",
                json!([{"effort":"low"},{"effort":"low"}]),
            ),
            ("supported_reasoning_levels", json!([{"effort":""}])),
            ("supported_reasoning_levels", json!(["low"])),
            ("default_reasoning_level", json!("not-supported")),
        ] {
            let mut catalog = remote();
            catalog["models"][1][field] = invalid;
            assert!(parse(&catalog).is_err(), "{field}");
        }
        let mut duplicate = remote();
        duplicate["models"]
            .as_array_mut()
            .unwrap()
            .push(template("GPT-6.2-SOL"));
        assert!(parse(&duplicate).is_err());
        for slug in [" GPT-6.2-SOL ", " gpt-6.2-sol ", "GPT-6.2-SOL "] {
            let mut malformed = remote();
            malformed["models"][1]["slug"] = json!(slug);
            assert!(parse(&malformed).is_err(), "{slug:?}");
        }
        // 缺兜底型号不再算错误，见 remote_catalog_without_fallback_model_is_still_accepted。
        assert!(parse_and_merge(b"not json").is_err());
        assert!(parse(&json!({"models":[]})).is_err());
        // Validate before filtering: a malformed retired row cannot be hidden by merge.
        let mut retired = remote();
        retired["models"]
            .as_array_mut()
            .unwrap()
            .push(json!({"slug":"gpt-5.4"}));
        assert!(parse(&retired).is_err());
    }

    #[test]
    fn rejects_malformed_overrides_but_accepts_partial_metadata_overrides() {
        for invalid in [
            Value::Null,
            json!({}),
            json!([null]),
            json!([{}]),
            json!([{"slug":""}]),
            json!([{"slug":"gpt-6.2-sol","priority":-1}]),
            json!([{"slug":"gpt-6.2-sol","supported_in_api":"yes"}]),
            json!([{"slug":"gpt-6.2-sol","context_window":2_000_000}]),
            json!([{"slug":"gpt-6.2-sol","default_reasoning_level":"missing"}]),
            json!([{"slug":"external-custom","context_window":2_000_000}]),
            json!([{"slug":"external-custom","default_reasoning_level":"missing"}]),
            json!([{"slug":"gpt-6.2-sol"},{"slug":"GPT-6.2-SOL"}]),
        ] {
            let mut catalog = remote();
            catalog["model_overrides"] = invalid;
            assert!(parse(&catalog).is_err());
        }
        let mut catalog = remote();
        catalog["model_overrides"] = json!([{"slug":"external-custom", "opaque": ["untouched"]}]);
        assert_eq!(
            parse(&catalog).unwrap()["model_overrides"],
            catalog["model_overrides"]
        );
    }

    #[test]
    fn recommendations_are_stable_filtered_and_use_remote_names() {
        let mut catalog = remote();
        let mut rows = Vec::new();
        for slug in [
            "gpt-reserve",
            "gpt-image-2",
            "codex-auto-review",
            "gpt-5.4",
            "claude-test",
            "gpt-hidden",
            "gpt-no-api",
            "codex-future",
            "gpt-tied",
        ] {
            let mut model = template(slug);
            model["priority"] = json!(0);
            if slug == "gpt-hidden" {
                model["visibility"] = json!("hide");
            }
            if slug == "gpt-no-api" {
                model["supported_in_api"] = json!(false);
            }
            rows.push(model);
        }
        catalog["models"].as_array_mut().unwrap().extend(rows);
        let merged = parse(&catalog).unwrap();
        with_test_catalog(merged, || {
            let recommended = recommended_models();
            assert_eq!(
                recommended[1],
                ("gpt-6.2-sol".into(), "Remote GPT-6.2 Sol".into())
            );
            assert_eq!(
                recommended[..7].iter().map(|item| item.0.as_str()).collect::<Vec<_>>(),
                ["gpt-6.1-sol", "gpt-6.2-sol", "gpt-6.2-astra", "gpt-6.1-terra",
                    "gpt-6-astra", "gpt-6-sol", "gpt-6-luna"]
            );
            assert_eq!(
                &recommended[7..9]
                    .iter()
                    .map(|item| item.0.as_str())
                    .collect::<Vec<_>>(),
                &["codex-future", "gpt-tied"]
            );
            assert!(is_recommended_model(" GPT-6.2-SOL "));
            for slug in [
                "gpt-reserve",
                "gpt-image-2",
                "codex-auto-review",
                "gpt-5.4",
                "claude-test",
                "gpt-hidden",
                "gpt-no-api",
            ] {
                assert!(!is_recommended_model(slug), "{slug}");
            }
        });
    }

    #[test]
    fn test_snapshot_override_is_nested_panic_safe_and_thread_local() {
        let original = snapshot();
        with_test_catalog(json!({"marker":"outer"}), || {
            assert_eq!(snapshot()["marker"], "outer");
            let _ = std::panic::catch_unwind(|| {
                with_test_catalog(json!({"marker":"inner"}), || panic!("reset"))
            });
            assert_eq!(snapshot()["marker"], "outer");
            let other = std::thread::spawn(snapshot).join().unwrap();
            assert!(Arc::ptr_eq(&other, &original));
        });
        assert!(Arc::ptr_eq(&snapshot(), &original));
    }

    #[tokio::test]
    async fn local_fetch_persists_merged_catalog_and_offline_uses_unchanged_cache() {
        let dir = TempDir::new();
        let cache = dir.cache();
        let client = local_client();
        let (url, task) = mock_response(serde_json::to_vec(&remote()).unwrap(), "").await;
        let (loaded, source) =
            load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_secs(2)).await;
        task.await.unwrap();
        assert_eq!(*loaded, parse(&remote()).unwrap());
        let bytes = fs::read(&cache).unwrap();
        assert_eq!(*loaded, read_cache(&cache).unwrap());
        let (cached, cached_source) =
            load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_millis(100)).await;
        assert_eq!(source, CatalogSource::Downloaded);
        assert_eq!(cached_source, CatalogSource::Cache);
        assert_eq!(*cached, *loaded);
        assert_eq!(fs::read(&cache).unwrap(), bytes);
    }

    #[tokio::test]
    async fn malformed_or_oversized_remote_never_overwrites_cache_and_second_source_works() {
        let dir = TempDir::new();
        let cache = dir.cache();
        let client = local_client();
        let valid = serde_json::to_vec(&parse(&remote()).unwrap()).unwrap();
        fs::write(&cache, &valid).unwrap();
        for body in [b"invalid".to_vec(), vec![b' '; MAX_CATALOG_BYTES + 1]] {
            let (url, task) = mock_response(body, "").await;
            let (loaded, source) =
                load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_secs(2)).await;
            task.await.unwrap();
            assert_eq!(source, CatalogSource::Cache);
            assert_eq!(*loaded, parse(&remote()).unwrap());
            assert_eq!(fs::read(&cache).unwrap(), valid);
        }
        let (bad, bad_task) = mock_response(b"{}".to_vec(), "").await;
        let (good, good_task) = mock_response(serde_json::to_vec(&remote()).unwrap(), "").await;
        assert_eq!(
            fetch_catalog(&client, &[&bad, &good], Duration::from_secs(2))
                .await
                .unwrap(),
            parse(&remote()).unwrap()
        );
        bad_task.await.unwrap();
        good_task.await.unwrap();
    }

    #[tokio::test]
    async fn decompressed_size_is_bounded_and_persist_failure_keeps_remote() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder
            .write_all(&vec![b' '; MAX_CATALOG_BYTES + 1])
            .unwrap();
        let (url, task) =
            mock_response(encoder.finish().unwrap(), "Content-Encoding: gzip\r\n").await;
        let client = local_client();
        assert!(fetch_catalog(&client, &[&url], Duration::from_secs(2))
            .await
            .is_none());
        task.await.unwrap();
        let dir = TempDir::new();
        let blocked = dir.0.join("file");
        fs::write(&blocked, "not a directory").unwrap();
        let (url, task) = mock_response(serde_json::to_vec(&remote()).unwrap(), "").await;
        let (loaded, source) = load_startup_catalog(
            Some(&client),
            &blocked.join("cache.json"),
            &[&url],
            Duration::from_secs(2),
        )
        .await;
        task.await.unwrap();
        assert_eq!(source, CatalogSource::Downloaded);
        assert_eq!(*loaded, parse(&remote()).unwrap());
    }

    #[tokio::test]
    async fn invalid_missing_or_oversized_cache_falls_back_without_writes() {
        let dir = TempDir::new();
        let cache = dir.cache();
        assert_eq!(
            *load_startup_catalog(None, &cache, &[], Duration::ZERO).await.0,
            *embedded()
        );
        assert!(!cache.exists());
        for body in [b"{}".to_vec(), vec![b' '; MAX_CATALOG_BYTES + 1]] {
            fs::write(&cache, &body).unwrap();
            assert!(read_cache(&cache).is_err());
            let (loaded, source) = load_startup_catalog(None, &cache, &[], Duration::ZERO).await;
            assert_eq!(source, CatalogSource::Embedded);
            assert!(Arc::ptr_eq(&loaded, &embedded()));
            assert_eq!(fs::read(&cache).unwrap(), body);
        }
    }

    #[test]
    fn integral_numeric_fields_match_go_and_normalize_for_rust_consumers() {
        let mut catalog = remote();
        catalog["models"][1]["context_window"] = json!(516_000.0);
        catalog["models"][1]["max_context_window"] = json!(1_032_000.0);
        catalog["models"][1]["priority"] = json!(0.0);
        catalog["model_overrides"][0]["priority"] = json!(1.0);
        let merged = parse(&catalog).unwrap();
        assert_eq!(
            merged["models"][1]["context_window"].as_i64(),
            Some(516_000)
        );
        assert_eq!(
            merged["models"][1]["max_context_window"].as_i64(),
            Some(1_032_000)
        );
        assert_eq!(merged["models"][1]["priority"].as_i64(), Some(0));
        assert_eq!(merged["model_overrides"][0]["priority"].as_i64(), Some(1));
        for value in [
            json!(9_223_372_036_854_775_808u64),
            json!(1e30),
            json!("100"),
            json!(-1.0),
        ] {
            catalog["models"][1]["priority"] = value;
            assert!(parse(&catalog).is_err());
        }
    }

    #[tokio::test]
    async fn source_timeout_is_bounded_and_http_errors_fall_back_without_cache_writes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stalled = format!("http://{}/catalog", listener.local_addr().unwrap());
        let stalled_task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let client = local_client();
        let started = std::time::Instant::now();
        assert!(
            fetch_catalog(&client, &[&stalled], Duration::from_millis(50))
                .await
                .is_none()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        stalled_task.abort();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let failed = format!("http://{}/catalog", listener.local_addr().unwrap());
        let failed_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            socket.read(&mut buf).await.unwrap();
            socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let dir = TempDir::new();
        let cache = dir.cache();
        let (loaded, source) =
            load_startup_catalog(Some(&client), &cache, &[&failed], Duration::from_secs(2)).await;
        failed_task.await.unwrap();
        assert_eq!(source, CatalogSource::Embedded);
        assert!(Arc::ptr_eq(&loaded, &embedded()));
        assert!(!cache.exists());
    }

    #[test]
    fn refresh_publishes_valid_snapshots_and_keeps_them_when_sources_fail() {
        let dir = TempDir::new();
        let price_path = dir.0.join("prices.json");
        let initial = embedded();
        let bundled = codex_model_prices::bundled();
        let active = RwLock::new(CatalogSnapshot { models: initial.clone(), prices: bundled.clone(), source: CatalogSource::Embedded });
        let mut book: Value = serde_json::from_slice(codex_model_prices::BUNDLED).unwrap();
        book["revision"] = json!(bundled.revision + 1);
        let bytes = serde_json::to_vec(&book).unwrap();
        let prices = codex_model_prices::parse_remote(&bytes).unwrap();
        let models = parse(&remote()).unwrap();
        let report = activate_remote(&active, &dir.cache(), &price_path, Some(models.clone()), Ok((prices, bytes)));
        assert!(report.models_updated && report.prices_updated);
        assert_eq!(*active.read().unwrap().models, models);
        assert_eq!(read_cache(&dir.cache()).unwrap(), models);
        assert_eq!(active.read().unwrap().prices.revision, bundled.revision + 1);
        assert_eq!(initial, embedded());

        let report = activate_remote(&active, &dir.cache(), &price_path, None, Err("offline".into()));
        assert!(report.model_error.is_some() && report.price_error.is_some());
        assert_eq!(*active.read().unwrap().models, models);
        assert_eq!(active.read().unwrap().prices.revision, bundled.revision + 1);
    }
}
