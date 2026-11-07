#[tauri::command]
pub async fn codex_local_access_update_multi_agent(
    multi_agent: crate::models::codex_local_access::CodexLocalAccessMultiAgentConfig,
) -> Result<crate::models::codex_local_access::CodexLocalAccessState, String> {
    crate::modules::codex_local_access::update_local_access_multi_agent(multi_agent).await
}
