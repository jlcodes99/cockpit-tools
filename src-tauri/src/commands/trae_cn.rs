use tauri::AppHandle;

use crate::models::trae::TraeOAuthStartResponse;
use crate::models::trae_cn::TraeCnAccount;
use crate::modules::{config, logger, process, trae_cn_account, trae_cn_oauth};

#[derive(serde::Serialize)]
pub struct TraeCnLaunchOnSwitchConfig {
    pub enabled: bool,
}

#[tauri::command]
pub fn list_trae_cn_accounts() -> Result<Vec<TraeCnAccount>, String> {
    trae_cn_account::list_accounts_checked()
}

#[tauri::command]
pub fn delete_trae_cn_account(account_id: String) -> Result<(), String> {
    trae_cn_account::remove_account(&account_id)
}

#[tauri::command]
pub fn delete_trae_cn_accounts(account_ids: Vec<String>) -> Result<(), String> {
    trae_cn_account::remove_accounts(&account_ids)
}

#[tauri::command]
pub fn import_trae_cn_from_json(json_content: String) -> Result<Vec<TraeCnAccount>, String> {
    trae_cn_account::import_from_json(&json_content)
}

#[tauri::command]
pub fn export_trae_cn_accounts(account_ids: Vec<String>) -> Result<String, String> {
    trae_cn_account::export_accounts(&account_ids)
}

#[tauri::command]
pub fn update_trae_cn_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<TraeCnAccount, String> {
    trae_cn_account::update_account_tags(&account_id, tags)
}

#[tauri::command]
pub fn get_trae_cn_accounts_index_path() -> Result<String, String> {
    trae_cn_account::accounts_index_path_string()
}

#[tauri::command]
pub fn get_trae_cn_launch_on_switch() -> Result<TraeCnLaunchOnSwitchConfig, String> {
    Ok(TraeCnLaunchOnSwitchConfig {
        enabled: config::get_user_config().trae_cn_launch_on_switch,
    })
}

#[tauri::command]
pub fn set_trae_cn_launch_on_switch(enabled: bool) -> Result<TraeCnLaunchOnSwitchConfig, String> {
    let mut user_config = config::get_user_config();
    user_config.trae_cn_launch_on_switch = enabled;
    config::save_user_config(&user_config)
        .map_err(|err| format!("保存 Trae CN 切号启动配置失败: {}", err))?;
    Ok(TraeCnLaunchOnSwitchConfig { enabled })
}

#[tauri::command]
pub async fn import_trae_cn_from_local(_app: AppHandle) -> Result<Vec<TraeCnAccount>, String> {
    match trae_cn_account::import_from_local()? {
        Some(account) => Ok(vec![account]),
        None => Err("未找到 Trae CN 本地登录态，请先在 Trae CN 客户端完成登录".to_string()),
    }
}

#[tauri::command]
pub async fn trae_cn_oauth_login_start() -> Result<TraeOAuthStartResponse, String> {
    logger::log_info("[Trae CN OAuth] start 命令触发");
    trae_cn_oauth::start_login().await
}

#[tauri::command]
pub async fn trae_cn_oauth_login_complete(
    _app: AppHandle,
    login_id: String,
) -> Result<TraeCnAccount, String> {
    logger::log_info(&format!(
        "[Trae CN OAuth] complete 命令触发: login_id={}",
        login_id
    ));
    let payload = trae_cn_oauth::complete_login(login_id.as_str()).await?;
    trae_cn_account::upsert_import_payload(payload)
}

#[tauri::command]
pub fn trae_cn_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Trae CN OAuth] cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    trae_cn_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn trae_cn_oauth_submit_callback_url(
    login_id: String,
    callback_url: String,
) -> Result<(), String> {
    trae_cn_oauth::submit_callback_url(login_id.as_str(), callback_url.as_str())
}

#[tauri::command]
pub async fn refresh_trae_cn_token(
    _app: AppHandle,
    _account_id: String,
) -> Result<TraeCnAccount, String> {
    Err("Trae CN Token 刷新尚未支持：需要先确认官方接口和 payload 格式".to_string())
}

#[tauri::command]
pub async fn refresh_all_trae_cn_tokens(_app: AppHandle) -> Result<i32, String> {
    Err("Trae CN 批量刷新尚未支持：需要先确认官方接口和 payload 格式".to_string())
}

#[tauri::command]
pub async fn add_trae_cn_account_with_token(
    _app: AppHandle,
    _access_token: String,
) -> Result<TraeCnAccount, String> {
    Err("Trae CN 暂不支持裸 Token 导入：请使用 OAuth、本机导入或完整 JSON 导入".to_string())
}

#[tauri::command]
pub fn inject_trae_cn_account(app: AppHandle, account_id: String) -> Result<String, String> {
    logger::log_info(&format!(
        "[Trae CN Switch] 开始写入登录态: account_id={}",
        account_id
    ));
    let account = trae_cn_account::inject_to_trae_cn(account_id.as_str())?;
    crate::modules::provider_current_state::set_current_account_id(
        "trae_cn",
        Some(account_id.as_str()),
    )?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Trae CN Switch] 登录态写入完成: account_id={}, email={}",
        account.id, account.email
    ));

    if config::get_user_config().trae_cn_launch_on_switch {
        if process::is_trae_cn_running() {
            process::close_trae_cn(20)
                .map_err(|err| format!("Trae CN 登录态已写入，但重启前关闭客户端失败: {}", err))?;
        }
        match process::start_trae_cn_default() {
            Ok(_) => Ok(format!("已切换并启动 Trae CN: {}", account.email)),
            Err(err) => Ok(format!("已写入 Trae CN 登录态，但启动 Trae CN 失败: {}", err)),
        }
    } else {
        Ok(format!(
            "已写入 Trae CN 登录态: {}。如客户端仍显示旧账号，请重启 Trae CN 后验证。",
            account.email
        ))
    }
}
