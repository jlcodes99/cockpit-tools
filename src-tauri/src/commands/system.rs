// System commands 统一入口。
// 按配置模型、备份/WebDAV、网络/通用设置和应用命令职责拆分，
// 通过 include! 保持 Tauri command 注册路径与调用行为不变。
include!("system_config_types.rs");
include!("system_backup_webdav.rs");
include!("system_mfa_recovery.rs");
include!("system_mfa_vault.rs");
include!("system_apikey_fun_vault.rs");
include!("system_network_general.rs");
include!("system_app_commands.rs");

#[tauri::command]
pub async fn load_ui_preferences() -> Result<modules::ui_preferences::UiPreferences, String> {
    tauri::async_runtime::spawn_blocking(modules::ui_preferences::load_ui_preferences)
        .await.map_err(|error| format!("读取界面偏好任务失败: {}", error))?
}

#[tauri::command]
pub async fn save_ui_preferences(
    values: std::collections::BTreeMap<String, String>,
) -> Result<modules::ui_preferences::UiPreferences, String> {
    tauri::async_runtime::spawn_blocking(move || modules::ui_preferences::save_ui_preferences(values))
        .await.map_err(|error| format!("保存界面偏好任务失败: {}", error))?
}

#[tauri::command]
pub async fn migrate_ui_preference_values(
    legacy: std::collections::BTreeMap<String, String>,
) -> Result<modules::ui_preferences::UiPreferences, String> {
    tauri::async_runtime::spawn_blocking(move || modules::ui_preferences::migrate_ui_preference_values(legacy))
        .await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn update_ui_preference(
    app: tauri::AppHandle,
    key: String,
    value: String,
    expected_revision: u64,
) -> Result<modules::ui_preferences::UiPreferences, String> {
    let changed_key = key.clone();
    let mut snapshot = tauri::async_runtime::spawn_blocking(move || {
        modules::ui_preferences::update_ui_preference(key, value, expected_revision)
    }).await.map_err(|error| error.to_string())??;
    snapshot.values.retain(|key, _| key == &changed_key);
    snapshot.revisions.retain(|key, _| key == &changed_key);
    let _ = tauri::Emitter::emit(&app, "ui-preferences:updated", &snapshot);
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    include!("system_tests.rs");
    include!("system_tests_floating_appearance.rs");
    include!("system_tests_codex_refresh_scope.rs");
    include!("system_tests_codex_path_validation.rs");
}
