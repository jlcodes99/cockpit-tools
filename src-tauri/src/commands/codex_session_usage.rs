use crate::modules::codex_session_usage::{
    query_session_usage_requests, CodexSessionUsageCursor, CodexSessionUsagePage,
    CodexSessionUsageQuery,
};

#[tauri::command]
pub async fn codex_query_session_usage_requests(
    query: CodexSessionUsageQuery,
    cursor: Option<CodexSessionUsageCursor>,
) -> Result<CodexSessionUsagePage, String> {
    tauri::async_runtime::spawn_blocking(move || query_session_usage_requests(query, cursor))
        .await
        .map_err(|error| error.to_string())?
}
