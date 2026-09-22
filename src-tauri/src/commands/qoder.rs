use std::time::Instant;
use tauri::{AppHandle, Emitter};

use crate::models::qoder::{QoderAccount, QoderOAuthStartResponse};
use crate::modules::qoder_channel::QoderChannel;
use crate::modules::{logger, qoder_account, qoder_oauth};

#[tauri::command]
pub fn list_qoder_channel_accounts(
    channel: Option<String>,
) -> Result<Vec<QoderAccount>, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::list_accounts_checked_for_channel(qoder_channel)
}

#[tauri::command]
pub fn list_qoder_accounts() -> Result<Vec<QoderAccount>, String> {
    list_qoder_channel_accounts(Some("qoder".to_string()))
}

#[tauri::command]
pub fn delete_qoder_channel_account(
    channel: Option<String>,
    account_id: String,
) -> Result<(), String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::remove_account_for_channel(qoder_channel, &account_id)
}

#[tauri::command]
pub fn delete_qoder_account(account_id: String) -> Result<(), String> {
    delete_qoder_channel_account(Some("qoder".to_string()), account_id)
}

#[tauri::command]
pub fn delete_qoder_channel_accounts(
    channel: Option<String>,
    account_ids: Vec<String>,
) -> Result<(), String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::remove_accounts_for_channel(qoder_channel, &account_ids)
}

#[tauri::command]
pub fn delete_qoder_accounts(account_ids: Vec<String>) -> Result<(), String> {
    delete_qoder_channel_accounts(Some("qoder".to_string()), account_ids)
}

#[tauri::command]
pub fn import_qoder_channel_from_json(
    channel: Option<String>,
    json_content: String,
) -> Result<Vec<QoderAccount>, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::import_from_json_for_channel(qoder_channel, &json_content)
}

#[tauri::command]
pub fn import_qoder_from_json(json_content: String) -> Result<Vec<QoderAccount>, String> {
    import_qoder_channel_from_json(Some("qoder".to_string()), json_content)
}

#[tauri::command]
pub fn import_qoder_channel_from_local(
    app: AppHandle,
    channel: Option<String>,
) -> Result<Vec<QoderAccount>, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    match qoder_account::import_from_local_channel(qoder_channel)? {
        Some(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            Ok(vec![account])
        }
        None => Err(format!(
            "未找到本地 {} 登录信息",
            qoder_channel.display_name()
        )),
    }
}

#[tauri::command]
pub fn import_qoder_from_local(app: AppHandle) -> Result<Vec<QoderAccount>, String> {
    import_qoder_channel_from_local(app, Some("qoder".to_string()))
}

#[tauri::command]
pub async fn qoder_oauth_login_start() -> Result<QoderOAuthStartResponse, String> {
    let started_at = Instant::now();
    logger::log_info("[Qoder OAuth] start 命令触发");
    let result = qoder_oauth::start_login().await;
    match &result {
        Ok(response) => logger::log_info(&format!(
            "[Qoder OAuth] start 命令完成: login_id={}, verification_uri_len={}, callback_url={}, elapsed={}ms",
            response.login_id,
            response.verification_uri.len(),
            response.callback_url.as_deref().unwrap_or("<none>"),
            started_at.elapsed().as_millis()
        )),
        Err(err) => logger::log_warn(&format!(
            "[Qoder OAuth] start 命令失败: elapsed={}ms, error={}",
            started_at.elapsed().as_millis(),
            err
        )),
    }
    result
}

#[tauri::command]
pub fn qoder_oauth_login_peek() -> Option<QoderOAuthStartResponse> {
    let pending = qoder_oauth::peek_pending_login();
    if let Some(state) = pending.as_ref() {
        logger::log_info(&format!(
            "[Qoder OAuth] peek 命令命中会话: login_id={}, verification_uri_len={}",
            state.login_id,
            state.verification_uri.len()
        ));
    } else {
        logger::log_info("[Qoder OAuth] peek 命令未命中会话");
    }
    pending
}

#[tauri::command]
pub async fn qoder_oauth_login_complete(
    app: AppHandle,
    login_id: String,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    logger::log_info(&format!(
        "[Qoder OAuth] complete 命令触发: login_id={}",
        login_id
    ));
    let account = match qoder_oauth::complete_login(&login_id).await {
        Ok(account) => account,
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder OAuth] complete 命令失败: login_id={}, elapsed={}ms, error={}",
                login_id,
                started_at.elapsed().as_millis(),
                err
            ));
            return Err(err);
        }
    };
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder OAuth] complete 命令完成: login_id={}, account_id={}, elapsed={}ms",
        login_id,
        account.id,
        started_at.elapsed().as_millis()
    ));
    Ok(account)
}

#[tauri::command]
pub fn qoder_oauth_login_cancel(login_id: Option<String>) -> Result<(), String> {
    logger::log_info(&format!(
        "[Qoder OAuth] cancel 命令触发: login_id={}",
        login_id.as_deref().unwrap_or("<none>")
    ));
    qoder_oauth::cancel_login(login_id.as_deref())
}

#[tauri::command]
pub fn export_qoder_channel_accounts(
    channel: Option<String>,
    account_ids: Vec<String>,
) -> Result<String, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::export_accounts_for_channel(qoder_channel, &account_ids)
}

#[tauri::command]
pub fn export_qoder_accounts(account_ids: Vec<String>) -> Result<String, String> {
    export_qoder_channel_accounts(Some("qoder".to_string()), account_ids)
}

#[tauri::command]
pub async fn refresh_qoder_token(
    app: AppHandle,
    account_id: String,
    channel: Option<String>,
) -> Result<QoderAccount, String> {
    let started_at = Instant::now();
    let qoder_channel = channel
        .as_deref()
        .and_then(|c| QoderChannel::parse(Some(c)).ok());

    logger::log_info(&format!(
        "[Qoder Command] 手动刷新账号开始: account_id={}, channel={:?}",
        account_id,
        qoder_channel
    ));
    match qoder_oauth::refresh_account_from_openapi_for_channel(qoder_channel, &account_id).await {
        Ok(account) => {
            let _ = crate::modules::tray::update_tray_menu(&app);
            logger::log_info(&format!(
                "[Qoder Command] 刷新完成: account_id={}, email={}, elapsed={}ms",
                account.id,
                account.email,
                started_at.elapsed().as_millis()
            ));
            Ok(account)
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder Command] 刷新失败: account_id={}, elapsed={}ms, error={}",
                account_id,
                started_at.elapsed().as_millis(),
                err
            ));
            Err(err)
        }
    }
}

#[tauri::command]
pub async fn refresh_all_qoder_tokens(app: AppHandle) -> Result<i32, String> {
    let started_at = Instant::now();
    logger::log_info("[Qoder Command] 批量刷新开始");
    let refreshed = qoder_oauth::refresh_all_accounts_from_openapi().await?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    logger::log_info(&format!(
        "[Qoder Command] 批量刷新完成: refreshed={}, elapsed={}ms",
        refreshed,
        started_at.elapsed().as_millis()
    ));
    Ok(refreshed)
}

#[tauri::command]
pub async fn inject_qoder_channel_account(
    app: AppHandle,
    channel: Option<String>,
    account_id: String,
) -> Result<String, String> {
    let started_at = Instant::now();
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;

    logger::log_info(&format!(
        "[Qoder Switch] 开始切换账号: channel={}, display={}, account_id={}",
        qoder_channel.provider_key(),
        qoder_channel.display_name(),
        account_id
    ));

    let account = qoder_account::load_account_for_channel(qoder_channel, &account_id)
        .or_else(|| qoder_account::load_account(&account_id))
        .ok_or_else(|| format!("{} 账号不存在: {}", qoder_channel.display_name(), account_id))?;

    // 1. 关进程：先彻底终止对应渠道正在运行的旧进程，阻断内存旧凭据回写覆盖！
    match crate::modules::process::close_qoder_channel_instances(qoder_channel, &[], 5) {
        Ok(()) => {
            logger::log_info(&format!(
                "[Qoder Switch] 成功关闭 {} 旧进程",
                qoder_channel.display_name()
            ));
        }
        Err(err) => {
            logger::log_warn(&format!(
                "[Qoder Switch] 尝试关闭 {} 旧进程异常(继续注入): {}",
                qoder_channel.display_name(),
                err
            ));
        }
    }

    // 释放文件锁微延迟
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // 2. 注入：安全写入磁盘凭据
    qoder_account::inject_to_qoder_channel_with_account(qoder_channel, &account_id)?;

    // 3. 状态：更新当前平台活跃账号标记
    let _ = crate::modules::provider_current_state::set_current_account_id(
        qoder_channel.provider_key(),
        Some(account_id.as_str()),
    );

    // 4. 启动新实例
    let launch_result =
        crate::modules::process::start_qoder_channel_default_with_args(qoder_channel, &[], false);
    let launch_warning = match launch_result {
        Ok(_) => None,
        Err(err) => {
            if err.starts_with("APP_PATH_NOT_FOUND:") || err.contains("启动 Qoder 失败") || err.contains("未找到") {
                logger::log_warn(&format!("{} 默认实例启动失败: {}", qoder_channel.display_name(), err));
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": qoder_channel.provider_key(),
                        "channel": qoder_channel.provider_key(),
                        "retry": { "kind": "default" }
                    }),
                );
                Some(err)
            } else {
                return Err(err);
            }
        }
    };

    let _ = crate::modules::tray::update_tray_menu(&app);

    if let Some(err) = launch_warning {
        logger::log_warn(&format!(
            "[Qoder Switch] 切号完成但启动失败: channel={}, account_id={}, email={}, elapsed={}ms, error={}",
            qoder_channel.display_name(),
            account.id,
            account.email,
            started_at.elapsed().as_millis(),
            err
        ));
        Ok(format!("切换完成，但 {} 启动失败: {}", qoder_channel.display_name(), err))
    } else {
        logger::log_info(&format!(
            "[Qoder Switch] 切号成功: channel={}, account_id={}, email={}, elapsed={}ms",
            qoder_channel.display_name(),
            account.id,
            account.email,
            started_at.elapsed().as_millis()
        ));
        Ok(format!("切换完成: {}", account.email))
    }
}

#[tauri::command]
pub async fn inject_qoder_account(app: AppHandle, account_id: String) -> Result<String, String> {
    inject_qoder_channel_account(app, Some("qoder".to_string()), account_id).await
}

#[tauri::command]
pub fn update_qoder_channel_account_tags(
    channel: Option<String>,
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderAccount, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::update_account_tags_for_channel(qoder_channel, &account_id, tags)
}

#[tauri::command]
pub fn update_qoder_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<QoderAccount, String> {
    update_qoder_channel_account_tags(Some("qoder".to_string()), account_id, tags)
}

#[tauri::command]
pub fn get_qoder_channel_accounts_index_path(
    channel: Option<String>,
) -> Result<String, String> {
    let qoder_channel = QoderChannel::parse(channel.as_deref())?;
    qoder_account::accounts_index_path_string_for_channel(qoder_channel)
}

#[tauri::command]
pub fn get_qoder_accounts_index_path() -> Result<String, String> {
    get_qoder_channel_accounts_index_path(Some("qoder".to_string()))
}
