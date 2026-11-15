use tauri::Emitter;

use crate::models::workbuddy::{WorkbuddyAccount, WorkbuddyOAuthStartResponse};
use crate::modules::{workbuddy_ai_account as account, workbuddy_ai_oauth as oauth};

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
    blocking(move || account::import_json(&json_content)).await
}

#[tauri::command]
pub async fn import_workbuddy_ai_from_local(
    app: tauri::AppHandle,
) -> Result<Vec<WorkbuddyAccount>, String> {
    let saved = blocking(account::import_local).await?;
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
            match oauth::renew_auth(&session).await {
                Ok(renewed) => {
                    let saved = blocking({
                        let renewed = renewed.clone();
                        move || account::store_refreshed_session(&saved, renewed)
                    })
                    .await?;
                    let quota = oauth::credit_usage(&renewed).await;
                    (saved, quota)
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
pub async fn switch_workbuddy_ai_account(app: tauri::AppHandle, account_id: String) -> Result<(), String> {
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
            let _ = app.emit("app:path_missing", serde_json::json!({
                "app": "workbuddy_ai", "retry": retry
            }));
        }
    }
    result
}

#[tauri::command]
pub async fn workbuddy_ai_launch_current() -> Result<(), String> {
    blocking(account::launch_current).await
}
