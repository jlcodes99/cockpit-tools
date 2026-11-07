#[tauri::command]
pub async fn codex_local_access_update_capability_fallback(
    capability_fallback: crate::models::codex_local_access::CodexLocalAccessCapabilityFallbackConfig,
) -> Result<crate::models::codex_local_access::CodexLocalAccessState, String> {
    crate::modules::codex_local_access::update_local_access_capability_fallback(capability_fallback)
        .await
}
