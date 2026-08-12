use crate::models::kimi::{KimiAccountView, KimiOAuthStartResponse};
use crate::modules::{
    kimi_account, kimi_oauth, kimi_wakeup,
    kimi_wakeup::{
        KimiCliStatus, KimiWakeupBatchResult, KimiWakeupHistoryItem, KimiWakeupModelInfo,
        KimiWakeupOverview, KimiWakeupRuntimeConfig, KimiWakeupState,
    },
    logger,
};
use tauri::AppHandle;

#[tauri::command]
pub fn list_kimi_accounts() -> Result<Vec<KimiAccountView>, String> {
    kimi_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_kimi_account(account_id: String) -> Result<(), String> {
    kimi_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_kimi_accounts(account_ids: Vec<String>) -> Result<(), String> {
    kimi_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_kimi_from_json(json_content: String) -> Result<Vec<KimiAccountView>, String> {
    kimi_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn import_kimi_from_local() -> Result<Vec<KimiAccountView>, String> {
    kimi_account::import_from_local()
}

#[tauri::command]
pub fn export_kimi_accounts(account_ids: Vec<String>) -> Result<String, String> {
    kimi_account::export_accounts(&account_ids)
}

#[tauri::command]
pub async fn kimi_oauth_login_start() -> Result<KimiOAuthStartResponse, String> {
    logger::log_info("[Kimi OAuth] device flow 开始");
    kimi_oauth::start_login().await
}

#[tauri::command]
pub async fn kimi_oauth_login_complete(
    app: AppHandle,
    login_id: String,
    reauth_account_id: Option<String>,
) -> Result<KimiAccountView, String> {
    let (token, device_id, expires_at, expires_in) =
        kimi_oauth::complete_login(&login_id).await?;
    let payload = crate::models::kimi::KimiOAuthCompletePayload {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        token_type: token.token_type,
        scope: token.scope,
        expires_at,
        expires_in,
        device_id,
        email: "unknown@kimi.local".to_string(),
        nickname: None,
        user_id: None,
        avatar: None,
        plan_type: Some("Kimi Code".to_string()),
    };
    let reauth_account_id = reauth_account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account = if let Some(account_id) = reauth_account_id {
        kimi_account::upsert_oauth_for_reauth(payload, account_id)?
    } else {
        kimi_account::upsert_oauth(payload)?
    };
    // Identity only (/me). Skip /usages on login to conserve membership traffic.
    let view = match kimi_account::hydrate_profile_only(&account.id).await {
        Ok(view) => view,
        Err(error) => {
            logger::log_warn(&format!(
                "[Kimi OAuth] 登录成功但拉取资料失败: account_id={}, error={}",
                account.id, error
            ));
            KimiAccountView::from(&account)
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(view)
}

#[tauri::command]
pub fn kimi_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    kimi_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub async fn refresh_kimi_account(
    app: AppHandle,
    account_id: String,
) -> Result<KimiAccountView, String> {
    let account = kimi_account::refresh_account(&account_id).await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(account)
}

#[tauri::command]
pub async fn refresh_all_kimi_accounts(app: AppHandle) -> Result<i32, String> {
    let results = kimi_account::refresh_all_accounts().await?;
    let success = results.iter().filter(|(_, result)| result.is_ok()).count() as i32;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(success)
}

#[tauri::command]
pub fn switch_kimi_account(app: AppHandle, account_id: String) -> Result<String, String> {
    let email = kimi_account::inject_to_default(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(format!("已同步 Kimi Code 官方登录: {}", email))
}

#[tauri::command]
pub fn update_kimi_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<KimiAccountView, String> {
    kimi_account::update_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_kimi_current_account_id() -> Result<Option<String>, String> {
    kimi_account::current_account_id()
}

#[tauri::command]
pub fn get_kimi_accounts_index_path() -> Result<String, String> {
    kimi_account::accounts_index_path_string()
}

// --- Kimi wakeup (Codex-shaped) ---

#[tauri::command]
pub fn kimi_wakeup_list_models() -> Vec<KimiWakeupModelInfo> {
    kimi_wakeup::builtin_models()
}

#[tauri::command]
pub fn kimi_wakeup_get_cli_status() -> KimiCliStatus {
    kimi_wakeup::get_cli_status()
}

/// Auto-detect Kimi CLI on PATH / known install dirs (ignores saved custom path).
#[tauri::command]
pub fn kimi_wakeup_detect_cli() -> KimiCliStatus {
    kimi_wakeup::detect_cli_on_system()
}

#[tauri::command]
pub fn kimi_wakeup_update_runtime_config(
    config: KimiWakeupRuntimeConfig,
) -> Result<KimiWakeupRuntimeConfig, String> {
    kimi_wakeup::save_runtime_config(&config)
}

#[tauri::command]
pub fn kimi_wakeup_get_overview() -> Result<KimiWakeupOverview, String> {
    kimi_wakeup::load_overview()
}

#[tauri::command]
pub fn kimi_wakeup_get_state() -> Result<KimiWakeupState, String> {
    kimi_wakeup::load_state()
}

#[tauri::command]
pub fn kimi_wakeup_save_state(state: KimiWakeupState) -> Result<KimiWakeupState, String> {
    kimi_wakeup::save_state(&state)
}

#[tauri::command]
pub fn kimi_wakeup_load_history() -> Result<Vec<KimiWakeupHistoryItem>, String> {
    kimi_wakeup::load_history()
}

#[tauri::command]
pub fn kimi_wakeup_clear_history() -> Result<(), String> {
    kimi_wakeup::clear_history()
}

#[tauri::command]
pub fn kimi_wakeup_test(
    account_ids: Vec<String>,
    prompt: Option<String>,
    model: Option<String>,
) -> Result<KimiWakeupBatchResult, String> {
    kimi_wakeup::run_batch(
        &account_ids,
        prompt.as_deref(),
        model.as_deref(),
        "test",
        None,
        Some("manual-test"),
    )
}

#[tauri::command]
pub fn kimi_wakeup_run_task(task_id: String) -> Result<KimiWakeupBatchResult, String> {
    kimi_wakeup::run_task(&task_id, "manual")
}

#[tauri::command]
pub fn kimi_wakeup_run_enabled_tasks() -> Result<Vec<KimiWakeupBatchResult>, String> {
    kimi_wakeup::run_enabled_tasks("manual")
}
