use crate::modules::codex_reset_automation as reset_automation;

#[tauri::command]
pub async fn get_codex_reset_credit_history(
    account_id: String,
    cursor: Option<String>,
) -> Result<codex_quota::reset_history::HistoryPage, String> {
    tokio::time::timeout(Duration::from_secs(60), codex_quota::reset_history::fetch(&account_id, cursor))
        .await.map_err(|_| reset_automation::message("timeout"))?
}

#[tauri::command]
pub async fn get_codex_reset_usage_history(
    account_id: String,
    cursor: Option<i64>,
) -> Result<reset_automation::UsagePage, String> {
    tauri::async_runtime::spawn_blocking(move || reset_automation::usage_history(&account_id, cursor))
        .await.map_err(|error| error.to_string())?
}
