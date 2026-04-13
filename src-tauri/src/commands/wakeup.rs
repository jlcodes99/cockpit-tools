use crate::modules;
use tauri::AppHandle;

#[tauri::command]
pub fn wakeup_ensure_runtime_ready(
    official_ls_version_mode: Option<String>,
) -> Result<Option<String>, String> {
    modules::wakeup::set_official_ls_version_mode(official_ls_version_mode.as_deref())?;
    modules::wakeup::ensure_wakeup_runtime_ready()
}

#[tauri::command]
pub async fn trigger_wakeup(
    account_id: String,
    model: String,
    prompt: Option<String>,
    max_output_tokens: Option<u32>,
    cancel_scope_id: Option<String>,
    official_ls_version_mode: Option<String>,
) -> Result<modules::wakeup::WakeupResponse, String> {
    let final_prompt = prompt.unwrap_or_else(|| "hi".to_string());
    let final_tokens = max_output_tokens.unwrap_or(0);
    modules::wakeup::set_official_ls_version_mode(official_ls_version_mode.as_deref())?;
    modules::wakeup::trigger_wakeup(
        &account_id,
        &model,
        &final_prompt,
        final_tokens,
        cancel_scope_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn fetch_available_models() -> Result<Vec<modules::wakeup::AvailableModel>, String> {
    modules::wakeup::fetch_available_models().await
}

#[tauri::command]
pub fn wakeup_validate_crontab(expr: String) -> Result<(), String> {
    modules::wakeup_scheduler::validate_crontab_expression(&expr)
}

#[tauri::command]
pub async fn wakeup_sync_state(
    app: AppHandle,
    enabled: bool,
    tasks: Vec<modules::wakeup_scheduler::WakeupTaskInput>,
    official_ls_version_mode: Option<String>,
    run_startup_tasks: Option<bool>,
) -> Result<(), String> {
    modules::wakeup::set_official_ls_version_mode(official_ls_version_mode.as_deref())?;
    modules::wakeup_scheduler::sync_state(enabled, tasks);
    modules::wakeup_scheduler::ensure_started(app.clone());
    if run_startup_tasks.unwrap_or(false) {
        modules::wakeup_scheduler::trigger_startup_tasks_if_needed(app);
    }
    Ok(())
}

#[tauri::command]
pub async fn wakeup_run_enabled_tasks(
    app: AppHandle,
    trigger_source: Option<String>,
    official_ls_version_mode: Option<String>,
) -> Result<u32, String> {
    modules::wakeup::set_official_ls_version_mode(official_ls_version_mode.as_deref())?;
    let source = trigger_source.unwrap_or_else(|| "startup".to_string());
    let started = modules::wakeup_scheduler::run_enabled_tasks_now(&app, &source).await;
    Ok(started as u32)
}

#[tauri::command]
pub fn wakeup_load_history() -> Result<Vec<modules::wakeup_history::WakeupHistoryItem>, String> {
    modules::wakeup_history::load_history()
}

#[tauri::command]
pub fn wakeup_add_history(
    items: Vec<modules::wakeup_history::WakeupHistoryItem>,
) -> Result<(), String> {
    modules::wakeup_history::add_history_items(items)
}

#[tauri::command]
pub fn wakeup_clear_history() -> Result<(), String> {
    modules::wakeup_history::clear_history()
}

#[tauri::command]
pub fn wakeup_cancel_scope(cancel_scope_id: String) -> Result<(), String> {
    modules::wakeup::cancel_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub fn wakeup_release_scope(cancel_scope_id: String) -> Result<(), String> {
    modules::wakeup::release_wakeup_scope(&cancel_scope_id)
}

#[tauri::command]
pub fn wakeup_verification_load_state(
) -> Result<Vec<modules::wakeup_verification::WakeupVerificationStateItem>, String> {
    modules::wakeup_verification::build_display_state_for_all_accounts()
}

#[tauri::command]
pub fn wakeup_verification_load_history(
) -> Result<Vec<modules::wakeup_verification::WakeupVerificationBatchHistoryItem>, String> {
    modules::wakeup_verification::load_history()
}

#[tauri::command]
pub fn wakeup_verification_delete_history(batch_ids: Vec<String>) -> Result<usize, String> {
    modules::wakeup_verification::delete_history(batch_ids)
}

#[tauri::command]
pub async fn wakeup_verification_run_batch(
    app: AppHandle,
    account_ids: Vec<String>,
    model: String,
    prompt: Option<String>,
    max_output_tokens: Option<u32>,
    official_ls_version_mode: Option<String>,
) -> Result<modules::wakeup_verification::WakeupVerificationBatchResult, String> {
    let final_prompt = prompt.unwrap_or_else(|| "hi".to_string());
    let final_tokens = max_output_tokens.unwrap_or(0);
    modules::wakeup::set_official_ls_version_mode(official_ls_version_mode.as_deref())?;
    modules::wakeup_verification::run_batch(&app, account_ids, &model, &final_prompt, final_tokens)
        .await
}

#[tauri::command]
pub fn wakeup_set_official_ls_version_mode(mode: Option<String>) -> Result<(), String> {
    modules::wakeup::set_official_ls_version_mode(mode.as_deref())
}
