use crate::modules::codex_proxy_launch_guard::{self, CountryPolicy};

#[tauri::command]
pub async fn get_codex_proxy_country_policy(account_id: String) -> Result<CountryPolicy, String> {
    tauri::async_runtime::spawn_blocking(move || codex_proxy_launch_guard::policy(&account_id))
        .await
        .map_err(|_| "PROXY_COUNTRY_STORAGE".to_string())?
}
#[tauri::command]
pub async fn set_codex_proxy_country_policy(
    account_id: String,
    expected_country: Option<String>,
) -> Result<CountryPolicy, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_proxy_launch_guard::save_policy(&account_id, expected_country)
    })
    .await
    .map_err(|_| "PROXY_COUNTRY_STORAGE".to_string())?
}

#[tauri::command]
pub async fn get_codex_proxy_env(
    account_id: String,
) -> Result<crate::modules::codex_proxy_env::EnvView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = crate::modules::codex_instance::get_default_codex_home()?;
        crate::modules::codex_proxy_env::view(&account_id, &home)
    })
    .await
    .map_err(|_| "PROXY_ENV_STORAGE".to_string())?
}
#[tauri::command]
pub async fn set_codex_proxy_env(
    account_id: String,
    document: String,
    revision: String,
) -> Result<crate::modules::codex_proxy_env::EnvView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::modules::codex_proxy_env::save(&account_id, &document, &revision)?;
        let home = crate::modules::codex_instance::get_default_codex_home()?;
        crate::modules::codex_proxy_env::view(&account_id, &home)
    })
    .await
    .map_err(|_| "PROXY_ENV_STORAGE".to_string())?
}
#[tauri::command]
pub async fn get_codex_proxy_country_checks(
    account_id: String,
) -> Result<Vec<codex_proxy_launch_guard::CountryCheck>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        codex_proxy_launch_guard::recent_checks(&account_id)
    })
    .await
    .map_err(|_| "PROXY_COUNTRY_STORAGE".to_string())?
}
#[tauri::command]
pub async fn check_codex_proxy_country(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<codex_proxy_launch_guard::CountryCheck, String> {
    let home = crate::modules::codex_instance::get_default_codex_home()?;
    codex_proxy_launch_guard::check_now(&app, &account_id, &home).await
}
