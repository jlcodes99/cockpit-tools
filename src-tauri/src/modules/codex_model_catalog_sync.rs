use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

const MODEL_CATALOG_URLS: &[&str] = &[
    "https://raw.githubusercontent.com/router-for-me/models/refs/heads/main/codex_client_models.json",
    "https://models.router-for.me/codex_client_models.json",
];
const MODEL_CATALOG_REFRESH_INTERVAL: Duration = Duration::from_secs(3 * 60 * 60);
const MODEL_CATALOG_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const PINNED_MODEL_IDS: &[&str] = &[
    "gpt-6.1-sol",
    "gpt-6-astra",
    "gpt-6-sol",
    "gpt-6-luna",
    "gpt-5.6-sol",
];

pub fn start_background_sync() {
    tauri::async_runtime::spawn(async {
        let mut pending_profiles = false;
        refresh_once(&mut pending_profiles).await;
        let mut interval = tokio::time::interval(MODEL_CATALOG_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_once(&mut pending_profiles).await;
        }
    });
}

async fn refresh_once(pending_profiles: &mut bool) {
    if *pending_profiles {
        let (_, failed) = crate::modules::codex_account::sync_uncustomized_managed_model_catalogs();
        *pending_profiles = failed > 0;
    }
    let client = match reqwest::Client::builder()
        .timeout(MODEL_CATALOG_REQUEST_TIMEOUT)
        .user_agent(format!("cockpit-tools/{}", env!("CARGO_PKG_VERSION")))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[Codex模型目录] 创建远端同步客户端失败: {}",
                error
            ));
            return;
        }
    };

    for url in MODEL_CATALOG_URLS {
        let response = match client.get(*url).send().await {
            Ok(response) => response,
            Err(error) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex模型目录] 拉取远端目录失败: url={}, error={}",
                    url, error
                ));
                continue;
            }
        };
        if !response.status().is_success() {
            crate::modules::logger::log_warn(&format!(
                "[Codex模型目录] 远端目录返回异常状态: url={}, status={}",
                url,
                response.status()
            ));
            continue;
        }
        let catalog = match response.json::<Value>().await {
            Ok(catalog) => catalog,
            Err(error) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex模型目录] 解析远端目录失败: url={}, error={}",
                    url, error
                ));
                continue;
            }
        };
        let catalog = match merge_pinned_models(catalog) {
            Ok(catalog) => catalog,
            Err(error) => {
                crate::modules::logger::log_warn(&format!(
                    "[Codex模型目录] 远端目录校验失败: url={}, error={}",
                    url, error
                ));
                continue;
            }
        };
        match crate::modules::codex_protocol::replace_codex_client_model_catalog(catalog) {
            Ok(changed) => {
                if !changed && !*pending_profiles {
                    return;
                }
                let (synced, failed) =
                    crate::modules::codex_account::sync_uncustomized_managed_model_catalogs();
                *pending_profiles = failed > 0;
                crate::modules::logger::log_info(&format!(
                    "[Codex模型目录] 已同步远端目录: source={}, profiles={}",
                    url, synced
                ));
                return;
            }
            Err(error) => crate::modules::logger::log_warn(&format!(
                "[Codex模型目录] 安装远端目录失败: url={}, error={}",
                url, error
            )),
        }
    }
}

fn merge_pinned_models(mut catalog: Value) -> Result<Value, String> {
    let models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "models must be an array".to_string())?;
    if models.is_empty() {
        return Err("models must not be empty".to_string());
    }

    let mut seen = HashSet::with_capacity(models.len());
    for model in models.iter() {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
            .ok_or_else(|| "model slug must be a non-empty string".to_string())?;
        if !seen.insert(slug.to_ascii_lowercase()) {
            return Err(format!("duplicate model slug: {}", slug));
        }
    }
    let fallback = crate::modules::codex_protocol::embedded_codex_client_model_catalog();
    let fallback_models = fallback
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| "embedded models must be an array".to_string())?;
    for pinned_id in PINNED_MODEL_IDS {
        if seen.contains(*pinned_id) {
            continue;
        }
        if let Some(model) = fallback_models.iter().find(|model| {
            model
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug.eq_ignore_ascii_case(pinned_id))
        }) {
            models.push(model.clone());
            seen.insert((*pinned_id).to_string());
        }
    }
    if let Some(template) = models.iter_mut().find(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug.eq_ignore_ascii_case("gpt-5.6-sol"))
    }) {
        template["slug"] = Value::String("gpt-5.6-sol".to_string());
    }

    let fallback_overrides = fallback.get("model_overrides").and_then(Value::as_array);
    if let Some(fallback_overrides) = fallback_overrides {
        let object = catalog.as_object_mut().ok_or("catalog must be an object")?;
        let overrides = object
            .entry("model_overrides")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or("model_overrides must be an array")?;
        for fallback_override in fallback_overrides {
            let Some(slug) = fallback_override.get("slug").and_then(Value::as_str) else {
                continue;
            };
            if !overrides.iter().any(|item| {
                item.get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|existing| existing.eq_ignore_ascii_case(slug))
            }) {
                overrides.push(fallback_override.clone());
            }
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_models() {
        let duplicate = serde_json::json!({
            "models": [
                {"slug": "gpt-5.6-sol"},
                {"slug": "GPT-5.6-SOL"}
            ]
        });
        assert!(merge_pinned_models(duplicate).is_err());
    }

    #[test]
    fn restores_missing_template_and_provider_overrides() {
        let catalog = serde_json::json!({"models": [{"slug": "gpt-6.1-terra"}]});
        let merged = merge_pinned_models(catalog).unwrap();
        assert!(merged["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["slug"] == "gpt-5.6-sol"));
        assert!(merged["model_overrides"]
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["slug"] == "grok-4.7" && model["context_window"] == 500000));
    }

    #[test]
    fn keeps_remote_models_without_a_static_allowlist() {
        let catalog = serde_json::json!({
            "models": [
                {"slug": "gpt-5.6-sol"},
                {"slug": "gpt-6.1-terra"}
            ]
        });
        let merged = merge_pinned_models(catalog).expect("valid catalog");
        let ids = merged["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["slug"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"gpt-6.1-terra"));
        assert!(ids.contains(&"gpt-6.1-sol"));
    }
}
