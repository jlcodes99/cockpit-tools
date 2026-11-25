use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use super::{
    antigravity_identity, apply_provider_routing, extensions, get_provider_workspace,
    read_t3_settings_json, save_provider_binding, write_provider_account,
    T3CodeBindableAccountView, T3CodeProviderInstanceView, T3CodeProviderWorkspaceView,
};
use crate::modules::{config, i18n, t3code_app_injection};

#[path = "t3code_provider_rollback.rs"]
pub(super) mod rollback;
#[path = "t3code_provider_sessions.rs"]
mod sessions;

const RPC_TIMEOUT: Duration = Duration::from_secs(8);
const RELOAD_TIMEOUT: Duration = Duration::from_secs(20);
const MODEL_REFRESH_TIMEOUT: Duration = Duration::from_secs(100);
const SESSION_DETACH_TIMEOUT: Duration = Duration::from_secs(30);
const ACTIVE_RUN_TIMEOUT: Duration = Duration::from_secs(120);
const GENERATION_ENV: &str = "COCKPIT_T3_PROVIDER_GENERATION";

type ModelRefreshJobs = HashMap<(PathBuf, String), bool>;

fn model_refresh_jobs() -> &'static Mutex<ModelRefreshJobs> {
    static JOBS: OnceLock<Mutex<ModelRefreshJobs>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

pub(crate) fn model_refresh_pending(state_dir: &Path, provider_id: &str) -> bool {
    model_refresh_jobs()
        .lock()
        .is_ok_and(|jobs| jobs.contains_key(&(state_dir.to_path_buf(), provider_id.to_owned())))
}

pub(crate) fn schedule_model_refresh(instance_id: &str, state_dir: &Path, provider_id: &str) {
    schedule_model_refreshes(instance_id, state_dir, std::iter::once(provider_id));
}

pub(super) fn schedule_model_refreshes<'a>(
    instance_id: &str,
    state_dir: &Path,
    provider_ids: impl IntoIterator<Item = &'a str>,
) {
    let Ok(mut jobs) = model_refresh_jobs().lock() else {
        return;
    };
    let mut keys = Vec::new();
    for provider_id in provider_ids {
        let key = (state_dir.to_path_buf(), provider_id.to_owned());
        if let Some(recheck) = jobs.get_mut(&key) {
            *recheck = true;
        } else {
            jobs.insert(key.clone(), false);
            keys.push(key);
        }
    }
    drop(jobs);
    if keys.is_empty() {
        return;
    }
    let instance_id = instance_id.to_owned();
    tokio::spawn(async move {
        for key in keys {
            loop {
                let result = async {
                    let Some(mut control) = ProviderControl::connect(&instance_id, &key.0).await?
                    else {
                        return Err(message("switchControlUnavailable"));
                    };
                    control.refresh_models(&key.1).await
                }
                .await;
                if let Err(error) = result {
                    crate::modules::logger::log_warn(&format!(
                        "[T3 Code] Provider model refresh failed for {}: {error}",
                        key.1
                    ));
                }
                let Ok(mut jobs) = model_refresh_jobs().lock() else {
                    return;
                };
                if jobs.get_mut(&key).is_some_and(std::mem::take) {
                    continue;
                }
                jobs.remove(&key);
                break;
            }
            t3code_app_injection::notify_provider_binding_changed(&instance_id);
        }
    });
}

fn pending_bindings() -> &'static Mutex<HashMap<PathBuf, Vec<T3CodeProviderInstanceView>>> {
    static BINDINGS: OnceLock<Mutex<HashMap<PathBuf, Vec<T3CodeProviderInstanceView>>>> =
        OnceLock::new();
    BINDINGS.get_or_init(Default::default)
}

pub(super) fn bindings_during_switch(state_dir: &Path) -> Vec<T3CodeProviderInstanceView> {
    pending_bindings()
        .lock()
        .ok()
        .and_then(|bindings| bindings.get(state_dir).cloned())
        .unwrap_or_default()
}

pub(super) struct PendingBinding(PathBuf);

impl PendingBinding {
    pub(super) fn keep_visible(
        state_dir: &Path,
        providers: Vec<T3CodeProviderInstanceView>,
    ) -> Result<Self, String> {
        let path = state_dir.to_path_buf();
        pending_bindings()
            .lock()
            .map_err(|error| error.to_string())?
            .insert(path.clone(), providers);
        Ok(Self(path))
    }
}

impl Drop for PendingBinding {
    fn drop(&mut self) {
        if let Ok(mut bindings) = pending_bindings().lock() {
            bindings.remove(&self.0);
        }
    }
}

pub(super) fn message(key: &str) -> String {
    i18n::translate(
        &config::get_user_config().language,
        &format!("t3code.providerWorkspace.{key}"),
        &[],
    )
}

pub(crate) struct ProviderControl {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    sequence: u64,
}

impl ProviderControl {
    pub(crate) async fn connect(
        instance_id: &str,
        state_dir: &Path,
    ) -> Result<Option<Self>, String> {
        let path = state_dir.join("environment-id");
        let environment = match tokio::fs::read_to_string(path).await {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(message("switchControlUnavailable")),
        };
        if environment.trim().is_empty() {
            return Ok(None);
        }
        let url = timeout(
            RELOAD_TIMEOUT,
            t3code_app_injection::provider_control_url(instance_id, environment.trim()),
        )
        .await
        .map_err(|_| message("switchControlUnavailable"))?;
        let Some(url) = url else {
            return Ok(None);
        };
        let (socket, _) = timeout(RPC_TIMEOUT, connect_async(url.as_str()))
            .await
            .ok()
            .and_then(Result::ok)
            .ok_or_else(|| message("switchControlUnavailable"))?;
        Ok(Some(Self {
            socket,
            sequence: 0,
        }))
    }

    pub(crate) async fn call(
        &mut self,
        method: &str,
        payload: Value,
        limit: Duration,
    ) -> Result<Value, String> {
        self.sequence += 1;
        let id = self.sequence.to_string();
        let request = json!({
            "_tag": "Request",
            "id": id,
            "tag": method,
            "payload": payload,
            "headers": [],
        });
        timeout(limit, async {
            self.socket
                .send(Message::Text(request.to_string().into()))
                .await
                .map_err(|_| message("switchControlUnavailable"))?;
            while let Some(frame) = self.socket.next().await {
                let Ok(Message::Text(frame)) = frame else {
                    continue;
                };
                let Ok(response) = serde_json::from_str::<Value>(&frame) else {
                    continue;
                };
                if response["requestId"].as_str() != Some(id.as_str()) || response["_tag"] != "Exit"
                {
                    continue;
                }
                return if response["exit"]["_tag"] == "Success" {
                    Ok(response["exit"]["value"].clone())
                } else {
                    Err(message("switchControlRejected"))
                };
            }
            Err(message("switchControlUnavailable"))
        })
        .await
        .map_err(|_| message("switchControlUnavailable"))?
    }

    pub(super) async fn reload_entry_by_id(
        &mut self,
        instance_id: &str,
        entry: Value,
    ) -> Result<String, String> {
        let generation = uuid::Uuid::new_v4().to_string();
        let entry = with_generation(entry, &generation)?;
        let settings = self
            .call(
                "server.updateSettings",
                json!({
                    "patch": {},
                    "providerInstanceMutation": {
                        "operation": "upsert",
                        "instanceId": instance_id,
                        "instance": entry,
                    },
                }),
                RPC_TIMEOUT,
            )
            .await?;
        let saved = &settings["providerInstances"][instance_id];
        // T3 applies the accepted settings through its provider settings watcher.
        if has_generation(saved, &generation) && saved["enabled"] != false {
            Ok(generation)
        } else {
            Err(message("switchControlRejected"))
        }
    }

    async fn refresh_models(&mut self, provider_id: &str) -> Result<(), String> {
        timeout(MODEL_REFRESH_TIMEOUT, async {
            for attempt in 0..8 {
                if attempt > 0 {
                    sleep(Duration::from_millis(350)).await;
                }
                let result = self
                    .call(
                        "server.refreshProviders",
                        json!({ "instanceId": provider_id, "refreshModels": true, "fresh": true,
                        "workspaceSnapshotsOnDemand": true }),
                        MODEL_REFRESH_TIMEOUT,
                    )
                    .await?;
                let row = result["providers"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row["instanceId"] == provider_id));
                // A settings write can precede registration in the live provider registry.
                let Some(row) = row else { continue };
                if row["enabled"] == false {
                    return Ok(());
                }
                if row["status"] == "warning" && row["installed"] == false {
                    continue;
                }
                if row["status"] == "error" {
                    return Err(row["message"]
                        .as_str()
                        .unwrap_or("Provider status error")
                        .to_owned());
                }
                return Ok(());
            }
            Err(message("switchTargetChanged"))
        })
        .await
        .map_err(|_| message("switchControlUnavailable"))?
    }

    pub(super) async fn prepare_switch(
        &mut self,
        state_dir: &Path,
        provider: &T3CodeProviderInstanceView,
    ) -> Result<(), String> {
        wait_for_active_runs(state_dir, &provider.instance_id).await?;
        self.detach_sessions(state_dir, provider).await
    }

    pub(super) async fn detach_sessions(
        &mut self,
        state_dir: &Path,
        provider: &T3CodeProviderInstanceView,
    ) -> Result<(), String> {
        let state_dir = state_dir.to_path_buf();
        let instance = provider.instance_id.clone();
        let driver = extensions::t3_raw_driver_for_platform(&provider.driver)
            .ok_or_else(|| message("switchTargetChanged"))?;
        timeout(SESSION_DETACH_TIMEOUT, async {
            let attachments = read_sessions(&state_dir, {
                move |db| sessions::attached_sessions(db, &instance, driver)
            })
            .await?;
            let mut effects = Vec::with_capacity(attachments.len());
            for (session_id, thread_id) in attachments {
                let command = uuid::Uuid::new_v4().to_string();
                self.call(
                    "orchestration.dispatchCommand",
                    json!({
                        "type": "provider-session.detach",
                        "commandId": command,
                        "threadId": thread_id,
                        "providerSessionId": session_id,
                        "reason": "client-requested",
                    }),
                    RPC_TIMEOUT,
                )
                .await?;
                effects.push(format!(
                    "effect:{command}:provider-session.detach:{session_id}"
                ));
            }
            while !effects.is_empty() {
                let receipts = read_sessions(&state_dir, {
                    let effects = effects.clone();
                    move |db| sessions::detach_receipts(db, &effects)
                })
                .await?;
                if receipts
                    .iter()
                    .any(|status| matches!(status.as_deref(), Some("failed" | "cancelled")))
                {
                    return Err(message("switchControlRejected"));
                }
                if receipts
                    .iter()
                    .all(|status| status.as_deref() == Some("succeeded"))
                {
                    break;
                }
                sleep(Duration::from_millis(250)).await;
            }

            Ok(())
        })
        .await
        .map_err(|_| message("switchDetachTimeout"))?
    }

    pub(super) async fn switch(
        &mut self,
        instance_id: &str,
        state_dir: &Path,
        provider: &T3CodeProviderInstanceView,
        account: &T3CodeBindableAccountView,
    ) -> Result<T3CodeProviderWorkspaceView, String> {
        if provider.instance_id.contains(['/', '\\'])
            || matches!(provider.instance_id.as_str(), "" | "." | "..")
        {
            return Err(message("switchTargetChanged"));
        }
        wait_for_active_runs(state_dir, &provider.instance_id).await?;
        let (dir, id, driver, oauth) = (
            state_dir.to_path_buf(),
            provider.instance_id.clone(),
            provider.driver.clone(),
            account.auth_mode == "oauth",
        );
        let (previous, snapshot) = tokio::task::spawn_blocking(move || {
            let settings = read_t3_settings_json(&dir)?;
            let previous = settings["providerInstances"]
                .get(&id)
                .cloned()
                .or_else(|| {
                    (id == driver)
                        .then(|| json!({ "driver": driver, "enabled": true, "config": {} }))
                })
                .filter(|entry| entry["driver"] == driver && entry["enabled"] != false)
                .ok_or_else(|| message("switchTargetChanged"))?;
            let snapshot = rollback::ProviderProfileSnapshot::capture(&dir, &id, &driver, oauth)?;
            Ok::<_, String>((previous, snapshot))
        })
        .await
        .map_err(|error| error.to_string())??;

        self.prepare_switch(state_dir, provider).await?;
        let pending_binding = PendingBinding::keep_visible(state_dir, vec![provider.clone()])?;
        let mut reload_attempted = false;
        let apply = async {
            let (routing, mut binding) = write_provider_account(
                state_dir,
                &provider.instance_id,
                &provider.driver,
                &account.id,
                previous.as_object(),
                false,
            )
            .await?;
            let mut entry = previous.clone();
            if entry.get("config").is_none() {
                entry["config"] = json!({});
            }
            let config = entry["config"]
                .as_object_mut()
                .ok_or_else(|| message("switchTargetChanged"))?;
            apply_provider_routing(config, &provider.driver, &routing);
            if let Some(entry) = entry.as_object_mut() {
                antigravity_identity::isolate_personal_account(entry);
            }
            reload_attempted = true;
            binding.live_generation = Some(
                self.reload_entry_by_id(&provider.instance_id, entry)
                    .await?,
            );
            let (dir, id) = (state_dir.to_path_buf(), provider.instance_id.clone());
            tokio::task::spawn_blocking(move || save_provider_binding(&dir, &id, binding))
                .await
                .map_err(|error| error.to_string())??;
            Ok::<_, String>(())
        }
        .await;
        if let Err(error) = apply {
            let restored = tokio::task::spawn_blocking(move || snapshot.restore()).await;
            let credentials_restored = match restored {
                Ok(Ok(())) => true,
                Ok(Err(error)) => {
                    crate::modules::logger::log_warn(&format!(
                        "[T3 Code] Provider credential rollback failed: {error}"
                    ));
                    false
                }
                Err(error) => {
                    crate::modules::logger::log_warn(&format!(
                        "[T3 Code] Provider rollback task failed: {error}"
                    ));
                    false
                }
            };
            let runtime_restored = !reload_attempted
                || self
                    .reload_entry_by_id(&provider.instance_id, previous)
                    .await
                    .is_ok();
            return Err(if credentials_restored && runtime_restored {
                error
            } else {
                format!("{error} {}", message("switchRollbackFailed"))
            });
        }
        t3code_app_injection::forget_provider_status(instance_id, &provider.instance_id);
        drop(pending_binding);
        let id = instance_id.to_owned();
        tokio::task::spawn_blocking(move || get_provider_workspace(Some(&id)))
            .await
            .map_err(|error| error.to_string())?
    }
}

async fn read_sessions<T: Send + 'static>(
    state_dir: &Path,
    read: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    let path = state_dir.to_path_buf();
    tokio::task::spawn_blocking(move || sessions::open(&path).and_then(|db| read(&db)))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| {
            crate::modules::logger::log_warn(&format!(
                "[T3 Code] Cannot read provider run or session state: {error}"
            ));
            message("switchControlUnavailable")
        })
}

pub(super) async fn wait_for_active_runs(state_dir: &Path, instance: &str) -> Result<(), String> {
    timeout(ACTIVE_RUN_TIMEOUT, async {
        loop {
            let instance = instance.to_owned();
            if !read_sessions(state_dir, move |db| {
                sessions::has_active_runs(db, &instance)
            })
            .await?
            {
                return Ok(());
            }
            sleep(Duration::from_millis(350)).await;
        }
    })
    .await
    .map_err(|_| message("switchActiveRunTimeout"))?
}

pub(super) async fn default_change_has_active_runs(
    state_dir: &Path,
    default_id: &str,
    source_id: &str,
) -> Result<bool, String> {
    let default_id = default_id.to_owned();
    let source_id = source_id.to_owned();
    read_sessions(state_dir, move |db| {
        Ok(sessions::has_active_runs(db, &default_id)? || sessions::has_active_runs(db, &source_id)?)
    })
    .await
}

pub(super) fn with_generation(mut entry: Value, generation: &str) -> Result<Value, String> {
    // T3 Code reconciles runtime fields including environment and closes the
    // old driver scope before constructing its replacement with the same ID.
    if entry.get("environment").is_none() {
        entry["environment"] = json!([]);
    }
    let variables = entry["environment"]
        .as_array_mut()
        .ok_or_else(|| message("switchTargetChanged"))?;
    variables.retain(|variable| variable["name"] != GENERATION_ENV);
    variables.push(json!({ "name": GENERATION_ENV, "value": generation, "sensitive": false }));
    Ok(entry)
}

pub(super) fn has_generation(entry: &Value, generation: &str) -> bool {
    entry["environment"].as_array().is_some_and(|variables| {
        variables
            .iter()
            .any(|variable| variable["name"] == GENERATION_ENV && variable["value"] == generation)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn model_refresh_targets_the_new_instance_after_registration() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            for providers in [
                json!([]),
                json!([{
                    "instanceId": "parallel-ag", "enabled": true, "installed": true, "status": "ready"
                }]),
            ] {
                let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
                let request: Value = serde_json::from_str(&frame).unwrap();
                assert_eq!(request["tag"], "server.refreshProviders");
                assert_eq!(request["payload"]["instanceId"], "parallel-ag");
                assert_eq!(request["payload"]["refreshModels"], true);
                let response = json!({ "_tag": "Exit", "requestId": request["id"],
                    "exit": { "_tag": "Success", "value": { "providers": providers } } });
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        });
        let (socket, _) = connect_async(format!("ws://{address}")).await.unwrap();
        let mut control = ProviderControl {
            socket,
            sequence: 0,
        };
        control.refresh_models("parallel-ag").await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn reload_keeps_the_enabled_provider_and_other_environment_settings() {
        let preserved = json!({"name": "USER_SETTING", "value": "example", "sensitive": false});
        let entry = json!({
            "driver": "codex", "displayName": "Work", "enabled": true,
            "config": { "homePath": "example-home" },
            "environment": [preserved, { "name": GENERATION_ENV, "value": "old" }],
        });
        let reloaded = with_generation(entry.clone(), "new").unwrap();
        for field in ["driver", "displayName", "enabled", "config"] {
            assert_eq!(reloaded[field], entry[field]);
        }
        assert_eq!(
            reloaded["environment"],
            json!([
                preserved, { "name": GENERATION_ENV, "value": "new", "sensitive": false },
            ])
        );
        assert!(has_generation(&reloaded, "new"));
        assert!(!has_generation(&reloaded, "old"));
    }
}
