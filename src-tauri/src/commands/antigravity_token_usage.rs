use crate::modules::antigravity_token_usage::{
    self,
    types::{
        AntigravityRehydrateStatus, AntigravityTokenUsageSettings, ConversationDetailResponse,
        DashboardBundleResponse, DashboardQueryRequest,
    },
};

#[tauri::command]
pub async fn antigravity_query_token_usage(
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    tokio::task::spawn_blocking(move || antigravity_token_usage::query_token_usage(query))
        .await
        .map_err(|e| format!("Antigravity token usage query task failed: {e}"))?
}

#[tauri::command]
pub async fn antigravity_sync_token_usage(
    rebuild: Option<bool>,
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    tokio::task::spawn_blocking(move || {
        antigravity_token_usage::sync_token_usage(rebuild.unwrap_or(false), query)
    })
    .await
    .map_err(|e| format!("Antigravity token usage sync task failed: {e}"))?
}

#[tauri::command]
pub async fn antigravity_get_conversation_token_detail(
    conversation_id: String,
) -> Result<ConversationDetailResponse, String> {
    tokio::task::spawn_blocking(move || {
        antigravity_token_usage::get_conversation_detail(&conversation_id)
    })
    .await
    .map_err(|e| format!("Antigravity conversation detail task failed: {e}"))?
}

#[tauri::command]
pub async fn antigravity_get_token_usage_settings() -> Result<AntigravityTokenUsageSettings, String>
{
    tokio::task::spawn_blocking(antigravity_token_usage::get_token_usage_settings)
        .await
        .map_err(|e| format!("Antigravity token usage settings task failed: {e}"))?
}

#[tauri::command]
pub async fn antigravity_set_token_usage_data_dir(
    custom_dir: Option<String>,
    query: Option<DashboardQueryRequest>,
) -> Result<DashboardBundleResponse, String> {
    tokio::task::spawn_blocking(move || {
        antigravity_token_usage::set_custom_data_dir(custom_dir, query)
    })
    .await
    .map_err(|e| format!("Antigravity set token usage data dir task failed: {e}"))?
}

#[tauri::command]
pub async fn antigravity_set_rehydrate_enabled(
    enabled: bool,
) -> Result<AntigravityRehydrateStatus, String> {
    tokio::task::spawn_blocking(move || antigravity_token_usage::toggle_rehydrate_hook(enabled))
        .await
        .map_err(|e| format!("Antigravity rehydrate hook toggle task failed: {e}"))?
}
