use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::Client;
use serde_json::{Map, Value};
use tokio::sync::OnceCell;

use super::{atomic_write, codex_wakeup::is_codex_model_before_5_5, config, logger};

const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const SOURCE_TIMEOUT: Duration = Duration::from_secs(8);
const PRIMARY_URL: &str = "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/codex_client_models.json";
const SECONDARY_URL: &str = "https://models.router-for.me/codex_client_models.json";
const PINNED_MODELS: [&str; 4] = ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna"];
const EMBEDDED_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../sidecars/cockpit-cliproxy/third_party/CLIProxyAPI/internal/registry/models/codex_client_models.json"
));

static EMBEDDED: OnceLock<Arc<Value>> = OnceLock::new();
static ACTIVE: OnceLock<Arc<Value>> = OnceLock::new();
static INITIALIZED: OnceCell<()> = OnceCell::const_new();

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
    ACTIVE.get().cloned().unwrap_or_else(embedded)
}

// Only startup calls initialize; all later consumers and sidecar restarts share ACTIVE.
pub(crate) async fn initialize() {
    initialize_once(&INITIALIZED, &ACTIVE, || async {
        let cache_path = config::get_shared_dir().join("codex-client-models.cache.json");
        let client = match Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::limited(3))
            .timeout(SOURCE_TIMEOUT)
            .connect_timeout(Duration::from_secs(4))
            .build()
        {
            Ok(client) => Some(client),
            Err(error) => {
                logger::log_warn(&format!(
                    "[Codex model catalog] HTTP client unavailable: {error}"
                ));
                None
            }
        };
        load_startup_catalog(
            client.as_ref(),
            &cache_path,
            &[PRIMARY_URL, SECONDARY_URL],
            SOURCE_TIMEOUT,
        )
        .await
    })
    .await;
}

async fn initialize_once<F, Fut>(once: &OnceCell<()>, active: &OnceLock<Arc<Value>>, load: F)
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Arc<Value>>,
{
    once.get_or_init(|| async {
        let catalog = load().await;
        let _ = active.set(catalog);
    })
    .await;
}

async fn load_startup_catalog(
    client: Option<&Client>,
    cache_path: &Path,
    urls: &[&str],
    timeout: Duration,
) -> Arc<Value> {
    if let Some(client) = client {
        if let Some(catalog) = fetch_catalog(client, urls, timeout).await {
            let content =
                serde_json::to_string(&catalog).expect("validated JSON catalog must serialize");
            if let Err(error) = atomic_write::write_string_atomic(cache_path, &content) {
                logger::log_warn(&format!(
                    "[Codex model catalog] Cache persistence failed; using fetched snapshot: {error}"
                ));
            }
            return Arc::new(catalog);
        }
    }
    match read_cache(cache_path) {
        Ok(catalog) => Arc::new(catalog),
        Err(error) => {
            if cache_path.exists() {
                logger::log_warn(&format!(
                    "[Codex model catalog] Cache rejected; using embedded snapshot: {error}"
                ));
            }
            embedded()
        }
    }
}

fn read_cache(path: &Path) -> Result<Value, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_CATALOG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    parse_and_merge(&bytes)
}

async fn fetch_catalog(client: &Client, urls: &[&str], timeout: Duration) -> Option<Value> {
    for url in urls {
        let result = tokio::time::timeout(timeout, async {
            let mut response = client
                .get(*url)
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
            parse_and_merge(&bytes)
        })
        .await;
        match result {
            Ok(Ok(catalog)) => return Some(catalog),
            Ok(Err(error)) => logger::log_warn(&format!(
                "[Codex model catalog] Rejected source {url}: {error}"
            )),
            Err(_) => logger::log_warn(&format!("[Codex model catalog] Source timed out: {url}")),
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
    for slug in PINNED_MODELS {
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
        "base_instructions",
        "minimal_client_version",
        "visibility",
        "default_reasoning_level",
    ] {
        if !partial || field == "slug" || model.contains_key(field) {
            required_string(model, field)?;
        }
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
    if !models
        .iter()
        .any(|model| model["slug"].as_str() == Some("gpt-5.5"))
    {
        return Err("catalog missing canonical gpt-5.5 fallback template".to_string());
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
            let base = by_id
                .get(&id)
                .unwrap_or_else(|| by_id.get("gpt-5.5").unwrap());
            let mut combined = (*base).clone();
            combined.extend(model.clone());
            validate_model(&combined, false)?;
        }
    }
    Ok(())
}

pub(crate) fn recommended_models() -> Vec<(String, String)> {
    let catalog = snapshot();
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
    recommended.sort_by_key(|model| model.0);
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
        let mut model = embedded()["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == "gpt-5.5")
            .unwrap()
            .clone();
        model["slug"] = json!(slug);
        model
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
        for slug in PINNED_MODELS {
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
        for slug in [" GPT-6.2-SOL ", " gpt-5.5", "GPT-5.5"] {
            let mut malformed = remote();
            malformed["models"][0]["slug"] = json!(slug);
            assert!(parse(&malformed).is_err());
        }
        let mut missing_fallback = remote();
        missing_fallback["models"].as_array_mut().unwrap().remove(0);
        assert!(parse(&missing_fallback).is_err());
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
                recommended[0],
                ("gpt-6.2-sol".into(), "Remote GPT-6.2 Sol".into())
            );
            assert_eq!(
                &recommended[1..3]
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
        let loaded =
            load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_secs(2)).await;
        task.await.unwrap();
        assert_eq!(*loaded, parse(&remote()).unwrap());
        let bytes = fs::read(&cache).unwrap();
        assert_eq!(*loaded, read_cache(&cache).unwrap());
        let cached =
            load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_millis(100)).await;
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
            let loaded =
                load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_secs(2)).await;
            task.await.unwrap();
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
        let loaded = load_startup_catalog(
            Some(&client),
            &blocked.join("cache.json"),
            &[&url],
            Duration::from_secs(2),
        )
        .await;
        task.await.unwrap();
        assert_eq!(*loaded, parse(&remote()).unwrap());
    }

    #[tokio::test]
    async fn invalid_missing_or_oversized_cache_falls_back_without_writes() {
        let dir = TempDir::new();
        let cache = dir.cache();
        assert_eq!(
            *load_startup_catalog(None, &cache, &[], Duration::ZERO).await,
            *embedded()
        );
        assert!(!cache.exists());
        for body in [b"{}".to_vec(), vec![b' '; MAX_CATALOG_BYTES + 1]] {
            fs::write(&cache, &body).unwrap();
            assert!(read_cache(&cache).is_err());
            let loaded = load_startup_catalog(None, &cache, &[], Duration::ZERO).await;
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
        let loaded =
            load_startup_catalog(Some(&client), &cache, &[&failed], Duration::from_secs(2)).await;
        failed_task.await.unwrap();
        assert!(Arc::ptr_eq(&loaded, &embedded()));
        assert!(!cache.exists());
    }

    #[tokio::test]
    async fn initialization_is_single_flight_and_never_refreshes_a_frozen_snapshot() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = TempDir::new();
        let cache = dir.cache();
        let client = local_client();
        let (url, task) = mock_response(serde_json::to_vec(&remote()).unwrap(), "").await;
        let once = OnceCell::new();
        let active = OnceLock::new();
        let calls = AtomicUsize::new(0);
        let load = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            load_startup_catalog(Some(&client), &cache, &[&url], Duration::from_secs(2)).await
        };
        tokio::join!(
            initialize_once(&once, &active, load),
            initialize_once(&once, &active, load)
        );
        task.await.unwrap();
        let frozen = active.get().unwrap().clone();
        fs::write(&cache, serde_json::to_vec(&*embedded()).unwrap()).unwrap();
        initialize_once(&once, &active, load).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(active.get().unwrap(), &frozen));
        assert_eq!(*frozen, parse(&remote()).unwrap());
    }
}
