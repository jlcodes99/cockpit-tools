#[tauri::command]
pub async fn load_apikey_fun_vault(
) -> Result<Option<modules::apikey_fun_vault::ApiKeyFunVault>, String> {
    tauri::async_runtime::spawn_blocking(modules::apikey_fun_vault::load)
        .await
        .map_err(|error| format!("读取 APIKEY.FUN 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn write_apikey_fun_vault(
    expected_revision: Option<u64>,
    keys: Vec<modules::apikey_fun_vault::ManagedApiKey>,
) -> Result<modules::apikey_fun_vault::ApiKeyFunVault, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::apikey_fun_vault::write(expected_revision, keys)
    })
    .await
    .map_err(|error| format!("写入 APIKEY.FUN 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn schedule_apikey_fun_recovery_checkpoint() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(modules::apikey_fun_vault::schedule_recovery_checkpoint)
        .await
        .map_err(|error| format!("安排 APIKEY.FUN 恢复备份任务失败: {error}"))?
}

#[tauri::command]
pub async fn restore_apikey_fun_vault() -> Result<modules::apikey_fun_vault::ApiKeyFunVault, String>
{
    tauri::async_runtime::spawn_blocking(modules::apikey_fun_vault::restore)
        .await
        .map_err(|error| format!("恢复 APIKEY.FUN 密钥库任务失败: {error}"))?
}

#[tauri::command]
pub async fn export_legacy_apikey_fun_vault(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::apikey_fun_vault::export_legacy(std::path::Path::new(&path))
    })
    .await
    .map_err(|error| format!("导出 APIKEY.FUN 旧版密钥任务失败: {error}"))?
}
