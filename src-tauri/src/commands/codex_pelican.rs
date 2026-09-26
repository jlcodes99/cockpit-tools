use crate::modules::codex_pelican::{
    self, Artifact, Batch, CleanupSummary, History, RetentionSettings, StartRequest,
};

#[tauri::command]
pub async fn codex_pelican_start(
    app: tauri::AppHandle,
    request: StartRequest,
) -> Result<Batch, String> {
    codex_pelican::start(app, request)
}

#[tauri::command]
pub async fn codex_pelican_retry(
    app: tauri::AppHandle,
    batch_id: String,
    item_id: String,
) -> Result<Batch, String> {
    codex_pelican::retry(app, batch_id, item_id).await
}

#[tauri::command]
pub async fn codex_pelican_active() -> Result<Option<Batch>, String> {
    codex_pelican::active()
}

#[tauri::command]
pub async fn codex_pelican_get(batch_id: String) -> Result<Batch, String> {
    codex_pelican::get(batch_id).await
}

#[tauri::command]
pub async fn codex_pelican_history(offset: usize, limit: usize) -> Result<History, String> {
    codex_pelican::history(offset, limit).await
}

#[tauri::command]
pub async fn codex_pelican_retention_settings() -> Result<RetentionSettings, String> {
    codex_pelican::retention_settings().await
}

#[tauri::command]
pub async fn codex_pelican_update_retention_days(days: u32) -> Result<RetentionSettings, String> {
    codex_pelican::update_retention_days(days).await
}

#[tauri::command]
pub async fn codex_pelican_cleanup_expired() -> Result<CleanupSummary, String> {
    codex_pelican::cleanup_expired().await
}

#[tauri::command]
pub async fn codex_pelican_clear_all(app: tauri::AppHandle) -> Result<CleanupSummary, String> {
    let summary = codex_pelican::clear_all().await?;
    if let Err(error) = crate::modules::codex_pelican_preview::close_all(&app).await {
        crate::modules::logger::log_warn(&format!(
            "[Pelican] close previews after cleanup: {error}"
        ));
    }
    Ok(summary)
}

#[tauri::command]
pub async fn codex_pelican_cancel(batch_id: String) -> Result<Batch, String> {
    codex_pelican::cancel(batch_id).await
}

#[tauri::command]
pub async fn codex_pelican_dismiss(batch_id: String) -> Result<(), String> {
    codex_pelican::dismiss(batch_id).await
}

#[tauri::command]
pub async fn codex_pelican_artifact(batch_id: String, item_id: String) -> Result<Artifact, String> {
    codex_pelican::artifact(batch_id, item_id).await
}

#[tauri::command]
pub async fn codex_pelican_delete(batch_id: String) -> Result<(), String> {
    codex_pelican::delete(batch_id).await
}
