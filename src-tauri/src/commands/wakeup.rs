use crate::modules;
use tauri::AppHandle;

#[tauri::command]
pub async fn wakeup_ensure_runtime_ready(
    transport_mode: Option<modules::wakeup::WakeupTransportMode>,
) -> Result<Option<String>, String> {
    modules::wakeup::ensure_wakeup_runtime_ready(transport_mode).await
}

#[tauri::command]
pub async fn trigger_wakeup(
    account_id: String,
    model: String,
    prompt: Option<String>,
    max_output_tokens: Option<u32>,
    cancel_scope_id: Option<String>,
    transport_mode: Option<modules::wakeup::WakeupTransportMode>,
) -> Result<modules::wakeup::WakeupResponse, String> {
    let final_prompt = prompt.unwrap_or_else(|| "hi".to_string());
    let final_tokens = max_output_tokens.unwrap_or(0);
    modules::wakeup::trigger_wakeup(
        &account_id,
        &model,
        &final_prompt,
        final_tokens,
        cancel_scope_id.as_deref(),
        transport_mode,
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
pub async fn wakeup_load_state() -> Result<modules::wakeup_scheduler::PersistedWakeupState, String> {
    tauri::async_runtime::spawn_blocking(modules::wakeup_scheduler::load_state)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn wakeup_initialize_state(
    app: AppHandle,
    legacy_state: Option<serde_json::Value>,
) -> Result<modules::wakeup_scheduler::PersistedWakeupState, String> {
    let state = tauri::async_runtime::spawn_blocking(move || {
        let legacy_state = legacy_state.and_then(|snapshot| match serde_json::from_value(snapshot) {
            Ok(state) => Some(state),
            Err(error) => {
                modules::logger::log_warn(&format!("[WakeupTasks] 旧 WebView 任务格式无效: {error}"));
                None
            }
        });
        modules::wakeup_scheduler::initialize_state(legacy_state)
    })
    .await
    .map_err(|error| error.to_string())??;
    modules::websocket::broadcast_wakeup_override(state.enabled);
    modules::wakeup_scheduler::ensure_started(app.clone());
    modules::wakeup_scheduler::trigger_startup_tasks_if_needed(app);
    Ok(state)
}

#[tauri::command]
pub async fn wakeup_sync_state(
    app: AppHandle,
    enabled: bool,
    tasks: Vec<modules::wakeup_scheduler::WakeupTaskInput>,
    run_startup_tasks: Option<bool>,
    expected_rules_revision: Option<u64>,
) -> Result<modules::wakeup_scheduler::PersistedWakeupState, String> {
    let state = tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_scheduler::sync_state(enabled, tasks, expected_rules_revision)
    })
    .await
    .map_err(|error| error.to_string())??;
    modules::websocket::broadcast_wakeup_override(state.enabled);
    modules::wakeup_scheduler::ensure_started(app.clone());
    if run_startup_tasks.unwrap_or(false) {
        modules::wakeup_scheduler::trigger_startup_tasks_if_needed(app);
    }
    Ok(state)
}

#[tauri::command]
pub async fn wakeup_run_enabled_tasks(
    app: AppHandle,
    trigger_source: Option<String>,
) -> Result<u32, String> {
    let source = trigger_source.unwrap_or_else(|| "startup".to_string());
    let started = modules::wakeup_scheduler::run_enabled_tasks_now(&app, &source).await;
    Ok(started as u32)
}

#[tauri::command]
pub async fn wakeup_load_history(
    retry: Option<bool>,
) -> Result<
    modules::wakeup_history_store::HistoryRead<modules::wakeup_history::WakeupHistoryItem>,
    String,
> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_history::load_history(retry.unwrap_or(false))
    })
    .await
    .map_err(|error| format!("唤醒历史任务失败: {error}"))?
}

#[tauri::command]
pub async fn wakeup_add_history(
    items: Vec<modules::wakeup_history::WakeupHistoryItem>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || modules::wakeup_history::add_history_items(items))
        .await
        .map_err(|error| format!("唤醒历史任务失败: {error}"))?
}

#[tauri::command]
pub async fn wakeup_clear_history() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(modules::wakeup_history::clear_history)
        .await
        .map_err(|error| format!("唤醒历史任务失败: {error}"))?
}

#[tauri::command]
pub async fn wakeup_verification_load_state(
    retry: Option<bool>,
) -> Result<
    modules::wakeup_history_store::HistoryRead<
        modules::wakeup_verification::WakeupVerificationStateItem,
    >,
    String,
> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_verification::build_display_state_for_all_accounts(retry.unwrap_or(false))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn wakeup_verification_load_history(
    retry: Option<bool>,
) -> Result<
    modules::wakeup_history_store::HistoryRead<
        modules::wakeup_verification::WakeupVerificationBatchHistoryItem,
    >,
    String,
> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::wakeup_verification::load_history(retry.unwrap_or(false))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn wakeup_verification_delete_history(batch_ids: Vec<String>) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || modules::wakeup_verification::delete_history(batch_ids))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn wakeup_verification_run_batch(
    app: AppHandle,
    account_ids: Vec<String>,
    model: String,
    prompt: Option<String>,
    max_output_tokens: Option<u32>,
) -> Result<modules::wakeup_verification::WakeupVerificationBatchResult, String> {
    let final_prompt = prompt.unwrap_or_else(|| "hi".to_string());
    let final_tokens = max_output_tokens.unwrap_or(0);
    modules::wakeup_verification::run_batch(&app, account_ids, &model, &final_prompt, final_tokens)
        .await
}

#[tauri::command]
pub async fn confirm_wakeup_task(app: AppHandle, task_id: String) -> Result<(), String> {
    modules::wakeup_scheduler::execute_pending_confirmation(&app, &task_id).await
}

#[tauri::command]
pub async fn cancel_wakeup_task(task_id: String) -> Result<(), String> {
    modules::wakeup_scheduler::cancel_pending_confirmation(&task_id)
}

#[tauri::command]
pub async fn check_wakeup_timeouts(app: AppHandle) -> Result<(), String> {
    modules::wakeup_scheduler::check_and_handle_timeouts(&app).await
}
