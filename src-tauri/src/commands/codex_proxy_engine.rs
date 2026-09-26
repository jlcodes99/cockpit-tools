use crate::modules::codex_proxy_engine_install::{self as installer, EngineInstallStatus};

#[tauri::command]
pub async fn codex_proxy_activity_snapshot(
    account_id: String,
) -> Result<crate::modules::codex_proxy_activity::ProxyActivitySnapshot, String> {
    crate::modules::codex_proxy_activity::snapshot(account_id).await
}

/// 全账号流量汇总：只返回计数与字节数，读取失败按账号降级，不影响其他账号。
#[tauri::command]
pub async fn codex_proxy_activity_summary(
) -> Result<Vec<crate::modules::codex_proxy_activity::ProxyActivitySummaryEntry>, String> {
    crate::modules::codex_proxy_activity::summary().await
}

#[tauri::command]
pub async fn codex_proxy_activity_set_enabled(
    account_id: String,
    enabled: bool,
) -> Result<(), String> {
    crate::modules::codex_proxy_activity::set_enabled(account_id, enabled).await
}

#[tauri::command]
pub async fn codex_proxy_activity_clear(account_id: String) -> Result<(), String> {
    let account = crate::modules::codex_proxy_runtime::load(&account_id).await?;
    if !crate::modules::codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    crate::modules::codex_proxy_activity::clear(&account_id);
    Ok(())
}

#[tauri::command]
pub async fn codex_proxy_engine_status() -> Result<EngineInstallStatus, String> {
    installer::status().await
}

/// Explicit prerequisite check; does not install or contact a proxy server.
#[tauri::command]
pub async fn codex_proxy_engine_preflight() -> Result<(), String> {
    crate::modules::codex_proxy_engine_preflight::require().await
}

/// Check the actual saved route before a UI restart stops its current client.
#[tauri::command]
pub async fn codex_proxy_instance_preflight(instance_id: String) -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(15), async move {
        let target = tokio::task::spawn_blocking(move || {
            super::codex_instance::resolve_codex_instance_start_target(&instance_id)
        })
        .await
        .map_err(|_| "PROXY_RUNTIME_FAILED")??;
        target.preflight_desktop_proxy().await
    })
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT".to_string())?
}

#[tauri::command]
pub async fn codex_proxy_engine_install(
    archive_path: Option<String>,
) -> Result<EngineInstallStatus, String> {
    installer::begin(archive_path).await
}

#[tauri::command]
pub fn codex_proxy_engine_cancel(job_id: String) -> Result<(), String> {
    installer::cancel(&job_id)
}
