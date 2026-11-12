use modules::mfa_vault_store::{
    CheckpointSummary, VaultChanges, VaultList, VaultProgress, VaultRecord, VaultSnapshot,
};

#[tauri::command]
pub async fn read_mfa_vault(list: VaultList) -> Result<VaultSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || modules::mfa_vault_store::read(list))
        .await
        .map_err(|error| format!("读取 2FA 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn create_mfa_recovery_checkpoint() -> Result<Option<CheckpointSummary>, String> {
    tauri::async_runtime::spawn_blocking(modules::mfa_vault_store::create_checkpoint)
        .await
        .map_err(|error| format!("创建 2FA 恢复检查点任务失败: {error}"))?
}

#[tauri::command]
pub async fn verify_mfa_recovery_checkpoint() -> Result<CheckpointSummary, String> {
    tauri::async_runtime::spawn_blocking(modules::mfa_vault_store::verify_checkpoint)
        .await
        .map_err(|error| format!("验证 2FA 恢复检查点任务失败: {error}"))?
}

#[tauri::command]
pub async fn restore_mfa_recovery_checkpoint() -> Result<CheckpointSummary, String> {
    tauri::async_runtime::spawn_blocking(modules::mfa_vault_store::restore_checkpoint)
        .await
        .map_err(|error| format!("恢复 2FA 检查点任务失败: {error}"))?
}

#[tauri::command]
pub async fn import_mfa_vault_batch(
    list: VaultList,
    source_id: String,
    offset: usize,
    records: Vec<VaultRecord>,
    final_batch: bool,
) -> Result<Option<VaultProgress>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(_resource) = modules::background_maintenance::try_resource() else {
            return Ok(None);
        };
        modules::mfa_vault_store::import(list, source_id, offset, records, final_batch).map(Some)
    })
    .await
    .map_err(|error| format!("迁移 2FA 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn change_mfa_vault(
    list: VaultList,
    changes: VaultChanges,
) -> Result<VaultProgress, String> {
    tauri::async_runtime::spawn_blocking(move || modules::mfa_vault_store::change(list, changes))
        .await
        .map_err(|error| format!("更新 2FA 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn read_legacy_webkit_vault(
    app: tauri::AppHandle,
    vault: modules::webkit_legacy_vault::LegacyVault,
) -> Result<Option<Vec<JsonValue>>, String> {
    let identifier = app.config().identifier.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        modules::webkit_legacy_vault::read(&identifier, vault)
    });
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .map_err(|_| "读取旧 WebView 密钥库超时".to_string())?
        .map_err(|_| "读取旧 WebView 密钥库任务失败".to_string())?
}
