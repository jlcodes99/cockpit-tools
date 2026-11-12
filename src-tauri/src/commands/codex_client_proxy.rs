use crate::modules::codex_client_proxy;
use crate::modules::{codex_proxy_direct_hosts, config};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

/// One proxy preview entry shared by OAuth, API Key and API-service cards.
#[tauri::command]
pub async fn codex_proxy_target_get(
    target_id: String,
) -> Result<codex_client_proxy::TargetView, String> {
    crate::modules::codex_proxy_targets::view(&target_id).await
}

#[tauri::command]
pub fn codex_proxy_direct_hosts_get() -> Vec<String> {
    config::get_codex_proxy_direct_hosts()
}

#[tauri::command]
pub async fn codex_proxy_direct_hosts_save(
    app: tauri::AppHandle,
    input: String,
) -> Result<Vec<String>, String> {
    let rules = codex_proxy_direct_hosts::parse(&input)?;
    static SAVING: AtomicBool = AtomicBool::new(false);
    if SAVING.swap(true, Ordering::AcqRel) {
        return Err("common.configSaveTimeout".into());
    }
    struct SaveGuard;
    impl Drop for SaveGuard {
        fn drop(&mut self) {
            SAVING.store(false, Ordering::Release);
        }
    }
    let guard = SaveGuard;
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let saved = config::patch_user_config(move |current| {
            current.codex_proxy_direct_hosts = rules;
            Ok(())
        })?
        .codex_proxy_direct_hosts;
        let _ = app.emit("codex:proxy-direct-hosts-changed", &saved);
        Ok::<_, String>(saved)
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .map_err(|_| "common.configSaveTimeout".to_string())?
        .map_err(|error| error.to_string())?
}
