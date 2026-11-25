use tauri::{AppHandle, Emitter};

use crate::models::t3code::{
    T3CodeAccount, T3CodeProviderRuntimeView, T3CodeProviderSwitchView, T3CodeProviderWorkspaceView,
    T3CodeQuotaWindowView,
};
use crate::modules::{
    logger, process, t3code_account, t3code_app_injection, t3code_instance,
    t3code_provider_bindings, tray,
};

#[tauri::command]
pub fn list_t3code_accounts() -> Result<Vec<T3CodeAccount>, String> {
    t3code_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_t3code_account(account_id: String) -> Result<(), String> {
    t3code_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_t3code_accounts(account_ids: Vec<String>) -> Result<(), String> {
    t3code_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub async fn import_t3code_from_json(json_content: String) -> Result<Vec<T3CodeAccount>, String> {
    tokio::task::spawn_blocking(move || t3code_account::import_from_json(&json_content))
        .await
        .map_err(|_| "导入 T3 Code 账号失败".to_string())?
}

#[tauri::command]
pub async fn import_t3code_from_file(path: String) -> Result<Vec<T3CodeAccount>, String> {
    tokio::task::spawn_blocking(move || {
        t3code_account::import_from_file(std::path::Path::new(&path))
    })
    .await
    .map_err(|_| "导入 T3 Code 文件失败".to_string())?
}

#[tauri::command]
pub async fn import_t3code_from_local(app: AppHandle) -> Result<Vec<T3CodeAccount>, String> {
    let accounts = t3code_account::import_from_local().await?;
    let _ = tray::update_tray_menu(&app);
    Ok(accounts)
}

#[tauri::command]
pub fn import_t3code_token(
    app: AppHandle,
    token: String,
    account_name: Option<String>,
) -> Result<T3CodeAccount, String> {
    let account = t3code_account::import_token_or_clerk_json(&token, account_name.as_deref())?;
    let _ = tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub fn export_t3code_accounts(account_ids: Vec<String>) -> Result<String, String> {
    t3code_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn refresh_t3code_account(
    app: AppHandle,
    account_id: String,
) -> Result<T3CodeAccount, String> {
    let account = t3code_account::refresh_account_quota(&account_id).await?;
    let _ = tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_t3code_accounts(app: AppHandle) -> Result<i32, String> {
    let count = t3code_account::refresh_all_accounts().await?;
    let _ = tray::update_tray_menu(&app);
    Ok(count)
}

#[tauri::command]
pub async fn inject_t3code_account(app: AppHandle, account_id: String) -> Result<String, String> {
    switch_t3code(app, account_id, false).await
}

pub(crate) async fn switch_t3code(
    app: AppHandle,
    account_id: String,
    report_launch_failure: bool,
) -> Result<String, String> {
    let app_for_stop = app.clone();
    let id_for_stop = account_id.clone();
    let (account, extra_args) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        if let Err(error) = t3code_instance::resolve_executable() {
            if error == "APP_PATH_NOT_FOUND:t3code" {
                let _ = app_for_stop.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "t3code",
                        "retry": { "kind": "switchAccount", "accountId": id_for_stop }
                    }),
                );
            }
            return Err(error);
        }
        let settings = t3code_instance::load_default_settings()?;
        if let Some(pid) = t3code_instance::resolve_pid(None, settings.last_pid) {
            process::close_pid(pid, 20)?;
        }
        t3code_instance::mark_stopped(None)?;
        let account = t3code_account::inject_to_default(&id_for_stop)?;
        if let Err(error) =
            t3code_instance::update_default_settings(Some(Some(id_for_stop)), None, Some(false))
        {
            logger::log_warn(&format!("更新 T3 Code 默认实例绑定账号失败: {}", error));
        }
        Ok((account, settings.extra_args))
    })
    .await
    .map_err(|error| error.to_string())??;

    t3code_provider_bindings::prepare_before_t3_launch(Some("__default__")).await;

    let app_for_start = app.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let args = process::parse_extra_args(&extra_args);
        match t3code_instance::start_default(&args) {
            Ok(pid) => t3code_instance::mark_started(None, pid)?,
            Err(error) if error == "APP_PATH_NOT_FOUND:t3code" => {
                let _ = app_for_start.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "t3code",
                        "retry": { "kind": "launchOnly" }
                    }),
                );
                if report_launch_failure {
                    return Err("APP_PATH_MISSING_AFTER_SWITCH:t3code".into());
                }
            }
            Err(error) => {
                return Err(format!("T3 Code 账号已切换，但默认实例启动失败: {error}"));
            }
        }
        let _ = tray::update_tray_menu(&app_for_start);
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())??;

    Ok(account.email)
}

#[tauri::command]
pub async fn t3code_launch_current() -> Result<(), String> {
    t3code_provider_bindings::prepare_before_t3_launch(Some("__default__")).await;
    tokio::task::spawn_blocking(|| {
        let settings = t3code_instance::load_default_settings()?;
        let args = process::parse_extra_args(&settings.extra_args);
        let pid = t3code_instance::start_default(&args)?;
        t3code_instance::mark_started(None, pid)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn update_t3code_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<T3CodeAccount, String> {
    t3code_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_t3code_current_account_id() -> Result<Option<String>, String> {
    Ok(t3code_account::confirmed_current_account_id())
}

#[tauri::command]
pub fn get_t3code_accounts_index_path() -> Result<String, String> {
    t3code_account::accounts_index_path_string()
}

#[tauri::command]
pub async fn get_t3code_provider_workspace(
    t3_instance_id: Option<String>,
) -> Result<T3CodeProviderWorkspaceView, String> {
    tokio::task::spawn_blocking(move || {
        let workspace = t3code_provider_bindings::get_provider_workspace(t3_instance_id.as_deref())?;
        t3code_provider_bindings::resume_pending_defaults(&workspace);
        Ok(workspace)
    })
    .await
    .map_err(|e| format!("读取 T3 Code Provider 绑定失败: {}", e))?
}

#[tauri::command]
pub async fn get_t3code_api_service_quota() -> Option<Vec<T3CodeQuotaWindowView>> {
    crate::modules::codex_app_injection::read_t3code_api_service_quota().await
}

#[tauri::command]
pub fn t3code_sync_provider_launch_paths(t3_instance_id: Option<String>) {
    t3code_provider_bindings::launch_paths::schedule_sync(t3_instance_id.as_deref());
}

#[tauri::command]
pub async fn get_t3code_provider_runtime_status(
    t3_instance_id: String,
) -> Vec<T3CodeProviderRuntimeView> {
    tokio::task::spawn_blocking(move || {
        t3code_provider_bindings::read_provider_runtime_status(&t3_instance_id)
    }).await.unwrap_or_default()
}

#[tauri::command]
pub async fn t3code_install_antigravity_runtime(instance_id: String) -> Result<(), String> {
    t3code_provider_bindings::launch_paths::install_antigravity_runtime(&instance_id).await
}

#[tauri::command]
pub async fn bind_t3code_provider_instance_account(
    t3_instance_id: Option<String>,
    provider_instance_id: String,
    driver: String,
    cockpit_account_id: String,
) -> Result<T3CodeProviderWorkspaceView, String> {
    t3code_provider_bindings::bind_provider_instance_account(
        t3_instance_id.as_deref(),
        &provider_instance_id,
        &driver,
        &cockpit_account_id,
    )
    .await
}

#[tauri::command]
pub async fn switch_t3code_provider_instance_account(
    t3_instance_id: Option<String>,
    provider_instance_id: String,
    driver: String,
    cockpit_account_id: String,
) -> Result<crate::models::t3code::T3CodeProviderSwitchView, String> {
    t3code_provider_bindings::switch_provider_instance_account(
        t3_instance_id.as_deref(),
        &provider_instance_id,
        &driver,
        &cockpit_account_id,
    )
    .await
}

#[tauri::command]
pub async fn upsert_t3code_provider_instance(
    t3_instance_id: Option<String>,
    instance_id: String,
    driver: String,
    display_name: Option<String>,
    accent_color: Option<String>,
    enabled: bool,
    bind_account_id: Option<String>,
) -> Result<T3CodeProviderWorkspaceView, String> {
    t3code_provider_bindings::upsert_provider_instance(
        t3_instance_id.as_deref(),
        &instance_id,
        &driver,
        display_name.as_deref(),
        accent_color.as_deref(),
        enabled,
        bind_account_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn promote_t3code_provider_account(
    t3_instance_id: Option<String>,
    provider_instance_id: String,
    cockpit_account_id: String,
) -> Result<T3CodeProviderSwitchView, String> {
    t3code_provider_bindings::promote_provider_account(
        t3_instance_id.as_deref(),
        &provider_instance_id,
        &cockpit_account_id,
    )
    .await
}

#[tauri::command]
pub async fn remove_t3code_provider_instance(
    t3_instance_id: Option<String>,
    instance_id: String,
    cockpit_account_id: Option<String>,
) -> Result<T3CodeProviderSwitchView, String> {
    t3code_provider_bindings::remove_provider_instance(
        t3_instance_id.as_deref(),
        &instance_id,
        cockpit_account_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn refresh_t3code_provider_account_quota(
    t3_instance_id: Option<String>,
    provider_instance_id: String,
) -> Result<T3CodeProviderWorkspaceView, String> {
    t3code_provider_bindings::refresh_provider_account_quota(
        t3_instance_id.as_deref(),
        &provider_instance_id,
    )
    .await
}

#[tauri::command]
pub async fn get_t3code_native_settings() -> Result<serde_json::Value, String> {
    crate::modules::t3code_native_settings::read().await
}

#[tauri::command]
pub async fn update_t3code_native_settings(
    field: String,
    value: serde_json::Value,
) -> Result<serde_json::Value, String> {
    crate::modules::t3code_native_settings::update(&field, value).await
}
