use crate::modules::claude_session_handoff::{self as handoff, HandoffPreview, Outcome, Status};
use tauri::{AppHandle, Emitter};

#[tauri::command]
pub async fn claude_handoff_status() -> Result<Status, String> {
    tauri::async_runtime::spawn_blocking(handoff::status)
        .await
        .map_err(|_| "HANDOFF_TASK_FAILED")?
}

#[tauri::command]
pub async fn claude_handoff_preview(
    source_account_id: String,
    target_account_id: String,
) -> Result<HandoffPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handoff::preview(&source_account_id, &target_account_id)
    })
    .await
    .map_err(|_| "HANDOFF_TASK_FAILED")?
}

#[tauri::command]
pub async fn claude_handoff_apply(
    source_account_id: String,
    target_account_id: String,
    fingerprint: String,
    desktop_version: String,
) -> Result<Outcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        handoff::apply(
            &source_account_id,
            &target_account_id,
            &fingerprint,
            &desktop_version,
        )
    })
    .await
    .map_err(|_| "HANDOFF_TASK_FAILED")?
}

#[tauri::command]
pub async fn claude_handoff_apply_and_switch(
    app: AppHandle,
    source_account_id: String,
    target_account_id: String,
    fingerprint: String,
    desktop_version: String,
) -> Result<Outcome, String> {
    let progress_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        handoff::apply_and_switch_observed(
            &source_account_id,
            &target_account_id,
            &fingerprint,
            &desktop_version,
            &mut |stage, completed, total| {
                let _ = progress_app.emit(
                    "claude-handoff-progress",
                    serde_json::json!({"stage":stage,"completed":completed,"total":total}),
                );
            },
        )
    })
    .await
    .map_err(|_| "HANDOFF_TASK_FAILED")??;
    if outcome.account_switched == Some(true) {
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    Ok(outcome)
}

#[tauri::command]
pub async fn claude_handoff_rollback(run_id: String) -> Result<Outcome, String> {
    tauri::async_runtime::spawn_blocking(move || handoff::rollback(&run_id))
        .await
        .map_err(|_| "HANDOFF_TASK_FAILED")?
}
