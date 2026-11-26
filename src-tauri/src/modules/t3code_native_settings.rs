//! Access the default T3 Code profile's native desktop and server preferences.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use super::t3code_provider_bindings::{
    live::ProviderControl, read_settings_object, read_t3_settings_json, reserve_provider_write,
    resolve_state_dir_for_t3_instance, write_t3_settings_json,
};
use super::{atomic_write, config, i18n, t3code_app_injection};

const RPC_TIMEOUT: Duration = Duration::from_secs(8);
const LANGUAGES: &[&str] = &[
    "system", "en", "zh", "de", "es", "fr", "ja", "ko", "pt-BR", "ru", "sv",
];

fn message(key: &str) -> String {
    i18n::translate(
        &config::get_user_config().language,
        &format!("t3code.nativeSettings.{key}"),
        &[],
    )
}

fn client_path(state_dir: &Path) -> PathBuf {
    state_dir.join("client-settings.json")
}

fn available_models(config: &Value) -> Vec<Value> {
    config["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|provider| {
            provider["enabled"] == true
                && provider["installed"] == true
                && provider["availability"] != "unavailable"
                && provider["modelsRetained"] != true
        })
        .flat_map(|provider| {
            provider["models"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(move |model| {
                    let instance_id = provider["instanceId"].as_str()?;
                    let slug = model["slug"].as_str()?;
                    let effort = model
                        .pointer("/capabilities/optionDescriptors")
                        .and_then(Value::as_array)
                        .and_then(|options| {
                            options.iter().find(|option| {
                                option["type"] == "select"
                                    && (option["id"] == "reasoningEffort"
                                        || option["role"] == "reasoning")
                            })
                        });
                    Some(json!({
                        "instanceId": instance_id,
                        "providerName": provider["displayName"].as_str().unwrap_or(instance_id),
                        "model": slug,
                        "label": model["name"].as_str().unwrap_or(slug),
                        "effort": effort,
                    }))
                })
        })
        .collect()
}

async fn read_client_with_state(state_dir: &Path) -> Result<Value, String> {
    if let Some(mut live) = t3code_app_injection::native_client_settings(None, &Value::Null).await {
        live["clientConnected"] = json!(true);
        return Ok(live);
    }
    let path = client_path(state_dir);
    let client = tokio::task::spawn_blocking(move || read_settings_object(&path))
        .await
        .map_err(|error| error.to_string())??;
    Ok(json!({
        "language": client["language"].as_str().unwrap_or("en"),
        "notificationMode": client["notificationMode"].as_str().unwrap_or("off"),
        "clientConnected": false,
    }))
}

async fn read_with_state(state_dir: &Path, client_only: bool) -> Result<Value, String> {
    if client_only {
        return read_client_with_state(state_dir).await;
    }
    let dir = state_dir.to_path_buf();
    let (client, server, control) = tokio::join!(
        read_client_with_state(state_dir),
        tokio::task::spawn_blocking(move || read_t3_settings_json(&dir)),
        ProviderControl::connect("__default__", state_dir),
    );
    let client = client?;
    let server = server.map_err(|error| error.to_string())??;
    let live_server = if let Ok(Some(mut control)) = control {
        control
            .call("server.getConfig", json!({}), RPC_TIMEOUT)
            .await
            .ok()
    } else {
        None
    };
    let server = live_server
        .as_ref()
        .and_then(|snapshot| snapshot.get("settings"))
        .unwrap_or(&server);
    Ok(json!({
        "language": client["language"].as_str().unwrap_or("en"),
        "notificationMode": client["notificationMode"].as_str().unwrap_or("off"),
        "defaultModelSelection": server["defaultModelSelection"],
        "models": live_server.as_ref().map(available_models).unwrap_or_default(),
        "clientConnected": client["clientConnected"],
    }))
}

pub async fn read(client_only: bool) -> Result<Value, String> {
    let (_, dir) = resolve_state_dir_for_t3_instance(None)?;
    read_with_state(&dir, client_only).await
}

fn checked_model(config: &Value, selection: &Value) -> Option<Value> {
    let instance = selection["instanceId"].as_str()?;
    let model = selection["model"].as_str()?;
    let available = available_models(config)
        .into_iter()
        .find(|entry| entry["instanceId"] == instance && entry["model"] == model)?;
    let mut next = json!({ "instanceId": instance, "model": model });
    let previous = &config["settings"]["defaultModelSelection"];
    let mut options = if previous["instanceId"] == instance && previous["model"] == model {
        previous["options"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Some(id) = available.pointer("/effort/id").and_then(Value::as_str) {
        options.retain(|item| item["id"] != id);
    }
    if let Some(effort) = selection["effort"].as_str() {
        let descriptor = available.get("effort")?;
        let valid = descriptor["options"]
            .as_array()?
            .iter()
            .any(|option| option["id"] == effort);
        if !valid {
            return None;
        }
        options.push(json!({ "id": descriptor["id"].as_str()?, "value": effort }));
    }
    if !options.is_empty() {
        next["options"] = json!(options);
    }
    Some(next)
}

pub async fn update(field: &str, value: Value, client_only: bool) -> Result<Value, String> {
    let (_, dir) = resolve_state_dir_for_t3_instance(None)?;
    match field {
        "defaultModelSelection" if !client_only => {
            let _reservation = reserve_provider_write(&dir)?;
            if let Some(mut control) = ProviderControl::connect("__default__", &dir).await? {
                let next = if value.is_null() {
                    value
                } else {
                    let snapshot = control
                        .call("server.getConfig", json!({}), RPC_TIMEOUT)
                        .await?;
                    checked_model(&snapshot, &value).ok_or_else(|| message("invalidModel"))?
                };
                let mut patch = serde_json::Map::new();
                patch.insert(field.into(), next.clone());
                let saved = control
                    .call(
                        "server.updateSettings",
                        json!({ "patch": patch }),
                        RPC_TIMEOUT,
                    )
                    .await?;
                if saved[field] != next {
                    return Err(message("saveFailed"));
                }
            } else if value.is_null() {
                let state = dir.clone();
                tokio::task::spawn_blocking(move || {
                    let mut settings = read_t3_settings_json(&state)?;
                    settings["defaultModelSelection"] = Value::Null;
                    write_t3_settings_json(&state, &settings)
                })
                .await
                .map_err(|error| error.to_string())??;
            } else {
                return Err(message("connectRequired"));
            }
        }
        "language" | "notificationMode" => {
            // The two Cockpit settings surfaces can save concurrently. Serialize
            // their native read/merge/write transactions to retain each field.
            static CLIENT_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
            let _reservation = CLIENT_WRITES.lock().await;
            let allowed = match field {
                "language" => LANGUAGES,
                _ => &["off", "notifications", "sound", "notifications-and-sound"],
            };
            if !value.as_str().is_some_and(|text| allowed.contains(&text)) {
                return Err(message("saveFailed"));
            }
            let live = t3code_app_injection::native_client_settings(None, &Value::Null).await;
            if live.is_some() {
                let saved = t3code_app_injection::native_client_settings(Some(field), &value)
                    .await
                    .ok_or_else(|| message("saveFailed"))?;
                if saved[field] != value {
                    return Err(message("saveFailed"));
                }
            } else {
                let field = field.to_owned();
                let state = dir.clone();
                tokio::task::spawn_blocking(move || {
                    let path = client_path(&state);
                    let mut settings = read_settings_object(&path)?;
                    settings[&field] = value;
                    std::fs::create_dir_all(&state).map_err(|error| error.to_string())?;
                    let data = serde_json::to_string_pretty(&settings)
                        .map_err(|error| error.to_string())?;
                    atomic_write::write_string_atomic(&path, &data)
                })
                .await
                .map_err(|error| error.to_string())??;
            }
        }
        _ => return Err(message("saveFailed")),
    }
    read_with_state(&dir, client_only).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_save_only_available_model_options_and_preserve_unrelated_options() {
        let config = json!({
            "settings": {"defaultModelSelection": { "instanceId": "codex", "model": "gpt-example", "options": [
                { "id": "reasoningEffort", "value": "low" }, { "id": "fast", "value": true }
            ]}},
            "providers": [{ "instanceId": "codex", "enabled": true, "installed": true,
                "models": [{ "slug": "gpt-example", "capabilities": { "optionDescriptors": [
                    { "id": "reasoningEffort", "type": "select", "options": [{ "id": "low" }, { "id": "high" }] }
                ]}}]
            }]
        });
        let model =
            |effort| json!({ "instanceId": "codex", "model": "gpt-example", "effort": effort });
        assert!(checked_model(&config, &model("unavailable")).is_none());
        let saved = checked_model(&config, &model("high")).unwrap();
        assert_eq!(
            saved["options"],
            json!([
                { "id": "fast", "value": true }, { "id": "reasoningEffort", "value": "high" }
            ])
        );
        let mut disabled = config;
        disabled["providers"][0]["enabled"] = json!(false);
        assert!(checked_model(&disabled, &model("high")).is_none());
    }
}
