use tauri::Emitter;

use crate::models::workbuddy::{WorkbuddyAccount, WorkbuddyOAuthStartResponse};
use crate::modules::{
    workbuddy_ai_account as account, workbuddy_ai_auto_checkin, workbuddy_ai_checkin,
    workbuddy_ai_oauth as oauth,
};

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn list_workbuddy_ai_accounts() -> Result<Vec<WorkbuddyAccount>, String> {
    blocking(account::list).await
}

#[tauri::command]
pub async fn delete_workbuddy_ai_accounts(account_ids: Vec<String>) -> Result<(), String> {
    blocking(move || account::delete(account_ids)).await
}

#[tauri::command]
pub async fn import_workbuddy_ai_from_json(
    json_content: String,
) -> Result<Vec<WorkbuddyAccount>, String> {
    let imported = blocking(move || account::import_json(&json_content)).await?;
    for acc in &imported {
        workbuddy_ai_auto_checkin::clear_account_retry_state(&acc.id);
    }
    Ok(imported)
}

#[tauri::command]
pub async fn import_workbuddy_ai_from_local(
    app: tauri::AppHandle,
) -> Result<Vec<WorkbuddyAccount>, String> {
    let saved = blocking(account::import_local).await?;
    workbuddy_ai_auto_checkin::clear_account_retry_state(&saved.id);
    queue_credit_refresh(app, &saved);
    Ok(vec![saved])
}

#[tauri::command]
pub async fn export_workbuddy_ai_accounts(account_ids: Vec<String>) -> Result<String, String> {
    blocking(move || {
        let accounts = account_ids
            .iter()
            .map(|id| account::load(id))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_string_pretty(&accounts).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
pub async fn workbuddy_ai_oauth_login_start() -> Result<WorkbuddyOAuthStartResponse, String> {
    oauth::start().await
}

#[tauri::command]
pub async fn workbuddy_ai_oauth_login_complete(
    app: tauri::AppHandle,
    login_id: String,
) -> Result<WorkbuddyAccount, String> {
    let session = oauth::complete(&login_id).await?;
    let saved = blocking(move || account::store_session(session)).await?;
    workbuddy_ai_auto_checkin::clear_account_retry_state(&saved.id);
    queue_credit_refresh(app, &saved);
    Ok(saved)
}

#[tauri::command]
pub fn workbuddy_ai_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    oauth::cancel(login_id.as_deref())
}

#[tauri::command]
pub async fn add_workbuddy_ai_account_with_token(
    app: tauri::AppHandle,
    access_token: String,
) -> Result<WorkbuddyAccount, String> {
    let session = oauth::session_from_access_token(access_token.trim()).await?;
    let saved = blocking(move || account::store_session(session)).await?;
    workbuddy_ai_auto_checkin::clear_account_retry_state(&saved.id);
    queue_credit_refresh(app, &saved);
    Ok(saved)
}

fn queue_credit_refresh(app: tauri::AppHandle, saved: &WorkbuddyAccount) {
    let Ok(guard) = account::RefreshGuard::acquire(&saved.id) else {
        return; // The already-running refresh owns the account.
    };
    let saved = saved.clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        if let Some(session) = saved.auth_raw.clone() {
            match refresh_quota(saved, session).await {
                Ok(account) => {
                    let _ = app.emit(
                        "accounts:changed",
                        serde_json::json!({
                            "platformId": "workbuddy_ai",
                            "accountId": account.id,
                            "reason": "quota",
                        }),
                    );
                }
                Err(error) => crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy AI] Could not save credit refresh: {error}"
                )),
            }
        }
    });
}

async fn refresh_quota(
    saved: WorkbuddyAccount,
    session: serde_json::Value,
) -> Result<WorkbuddyAccount, String> {
    let (saved, quota) = match oauth::credit_usage(&session).await {
        Err(error)
            if billing_rejected_token(&error)
                && session["auth"]["refreshToken"]
                    .as_str()
                    .is_some_and(|token| !token.is_empty()) =>
        {
            match workbuddy_ai_checkin::renew_account_session_shared(&saved.id, &saved.access_token)
                .await
            {
                Ok(updated) => {
                    let quota = match updated.auth_raw.as_ref() {
                        Some(renewed) => oauth::credit_usage(renewed).await,
                        None => Err("WORKBUDDY_AI_MISSING_SESSION".to_string()),
                    };
                    (updated, quota)
                }
                Err(error) => (saved, Err(error)),
            }
        }
        quota => (saved, quota),
    };
    if let Err(error) = &quota {
        crate::modules::logger::log_warn(&format!("[WorkBuddy AI] Refresh credits: {error}"));
    }
    blocking(move || account::update_quota(&saved.id, &saved.access_token, quota)).await
}

fn billing_rejected_token(error: &str) -> bool {
    error
        .strip_prefix("BUDDY_RESOURCE_HTTP: ")
        .and_then(|detail| detail.rsplit_once(' '))
        .is_some_and(|(_, status)| status == "401")
}

#[tauri::command]
pub async fn refresh_workbuddy_ai_token(account_id: String) -> Result<WorkbuddyAccount, String> {
    let _refresh = account::RefreshGuard::acquire(&account_id)?;
    let saved = blocking(move || account::load(&account_id)).await?;
    let session = saved
        .auth_raw
        .clone()
        .ok_or("WORKBUDDY_AI_MISSING_SESSION")?;
    refresh_quota(saved, session).await
}

#[tauri::command]
pub async fn refresh_all_workbuddy_ai_tokens() -> Result<usize, String> {
    use futures::{stream, StreamExt};
    let accounts = blocking(account::list).await?;
    let results: Vec<_> = stream::iter(
        accounts
            .into_iter()
            .map(|account| refresh_workbuddy_ai_token(account.id)),
    )
    .buffer_unordered(5)
    .collect()
    .await;
    let mut succeeded = 0;
    let mut first_error = None;
    for result in results {
        match result {
            Ok(account) if account.quota_query_last_error.is_none() => succeeded += 1,
            Ok(account) => {
                first_error.get_or_insert(account.quota_query_last_error.unwrap_or_default());
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(succeeded),
    }
}

#[tauri::command]
pub async fn update_workbuddy_ai_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<WorkbuddyAccount, String> {
    blocking(move || account::update_tags(&account_id, tags)).await
}

#[tauri::command]
pub async fn switch_workbuddy_ai_account(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<(), String> {
    let id = account_id.clone();
    let result = blocking(move || account::switch(&id)).await;
    if let Err(error) = &result {
        let retry = match error.as_str() {
            account::MISSING_APP_PATH => Some(serde_json::json!({
                "kind": "switchAccount", "accountId": account_id
            })),
            account::SWITCHED_MISSING_APP_PATH => Some(serde_json::json!({ "kind": "launchOnly" })),
            _ => None,
        };
        if let Some(retry) = retry {
            let _ = app.emit(
                "app:path_missing",
                serde_json::json!({
                    "app": "workbuddy_ai", "retry": retry
                }),
            );
        }
    }
    result
}

#[tauri::command]
pub async fn workbuddy_ai_launch_current() -> Result<(), String> {
    blocking(account::launch_current).await
}

#[tauri::command]
pub async fn get_checkin_status_workbuddy_ai(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<workbuddy_ai_checkin::WorkbuddyAiCheckinStatus, String> {
    let (status, reconciled) =
        workbuddy_ai_checkin::get_account_checkin_status_with_reconcile(&account_id).await?;
    if reconciled {
        let _ = app.emit("workbuddy-ai-auto-checkin-logs-changed", ());
        let _ = app.emit("workbuddy-ai-auto-checkin-config-changed", ());
    }
    Ok(status)
}

#[tauri::command]
pub async fn checkin_workbuddy_ai(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<workbuddy_ai_checkin::WorkbuddyAiCheckinResult, String> {
    let started = std::time::Instant::now();
    let result = workbuddy_ai_checkin::checkin_account(&account_id).await;
    let log_account = account_id.clone();
    let log_result = result.clone();
    let duration_ms = started.elapsed().as_millis() as u64;
    let record_err = blocking(move || {
        workbuddy_ai_auto_checkin::record_manual_result(&log_account, &log_result, duration_ms)
    })
    .await
    .err();
    let _ = app.emit("workbuddy-ai-auto-checkin-logs-changed", ());
    let _ = app.emit("workbuddy-ai-auto-checkin-config-changed", ());
    let mut response = result?;
    if let Some(err) = record_err {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy AI Checkin] Remote check-in completed but local log write failed: {err}"
        ));
        match &mut response.warning {
            Some(existing) if !existing.is_empty() => {
                existing.push_str(&format!("; local_history: {err}"));
            }
            _ => response.warning = Some(format!("local_history: {err}")),
        }
    }
    if response.success {
        let _ = crate::modules::tray::update_tray_menu(&app);
        let _ = app.emit(
            "workbuddy_ai:checkin_completed",
            serde_json::json!({
                "accountId": account_id,
                "success": true,
                "credit": response.credit,
                "streakDays": response.streak_days,
            }),
        );
    }
    Ok(response)
}

#[tauri::command]
pub async fn get_workbuddy_ai_auto_checkin_config(
) -> Result<workbuddy_ai_auto_checkin::WorkbuddyAiAutoCheckinConfig, String> {
    blocking(workbuddy_ai_auto_checkin::get_config_checked).await
}

#[tauri::command]
pub async fn save_workbuddy_ai_auto_checkin_config(
    config: workbuddy_ai_auto_checkin::WorkbuddyAiAutoCheckinConfig,
) -> Result<(), String> {
    blocking(move || workbuddy_ai_auto_checkin::save_config(&config)).await
}

#[tauri::command]
pub async fn get_workbuddy_ai_auto_checkin_logs(
    retry: Option<bool>,
) -> Result<
    crate::modules::wakeup_history_store::HistoryRead<
        crate::modules::workbuddy_ai_auto_checkin::WorkbuddyAiAutoCheckinLogRecord,
    >,
    String,
> {
    blocking(move || workbuddy_ai_auto_checkin::get_logs_checked(retry.unwrap_or(false))).await
}

#[tauri::command]
pub async fn clear_workbuddy_ai_auto_checkin_logs() -> Result<(), String> {
    blocking(workbuddy_ai_auto_checkin::clear_logs).await
}

#[tauri::command]
pub async fn run_workbuddy_ai_auto_checkin_now(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<String, String> {
    workbuddy_ai_auto_checkin::run_workbuddy_ai_auto_checkin_cycle_if_needed(
        &app,
        force.unwrap_or(false),
    )
    .await
}
