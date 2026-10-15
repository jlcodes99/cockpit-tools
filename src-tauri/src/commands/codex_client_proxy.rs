use crate::modules::codex_client_proxy;

/// One proxy preview entry shared by OAuth, API Key and API-service cards.
#[tauri::command]
pub async fn codex_proxy_target_get(
    target_id: String,
) -> Result<codex_client_proxy::TargetView, String> {
    crate::modules::codex_proxy_targets::view(&target_id).await
}
