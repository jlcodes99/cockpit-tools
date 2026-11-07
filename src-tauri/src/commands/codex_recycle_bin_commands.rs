#[tauri::command]
pub async fn list_codex_recycled_accounts(
) -> Result<Vec<codex_account::CodexRecycledAccount>, String> {
    tauri::async_runtime::spawn_blocking(codex_account::list_recycled_accounts)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn export_codex_recycled_accounts(
    recycle_ids: Vec<String>,
    path: String,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::export_recycled_accounts_to_file(&recycle_ids, &path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn restore_codex_recycled_account(recycle_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::restore_recycled_account(&recycle_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn delete_codex_recycled_account(recycle_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_account::delete_recycled_account(&recycle_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Only clear the user's confirmed snapshot, preserving concurrently recycled accounts.
#[tauri::command]
pub async fn empty_codex_recycle_bin(recycle_ids: Vec<String>) -> Result<(), String> {
    for recycle_id in recycle_ids {
        delete_codex_recycled_account(recycle_id).await?;
        tokio::task::yield_now().await;
    }
    Ok(())
}
