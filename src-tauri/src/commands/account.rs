use crate::error::{AppError, AppResult};
use crate::models;
use crate::modules;
use tauri::AppHandle;
use tauri::Emitter;

#[tauri::command]
pub async fn list_accounts() -> Result<Vec<models::Account>, String> {
    modules::list_accounts()
}

/// 从 VS Code SecretStorage 同步插件账号
#[tauri::command]
pub async fn sync_from_extension(app: tauri::AppHandle) -> Result<usize, String> {
    modules::import::import_from_extension_credentials(Some(&app)).await
}

#[tauri::command]
pub async fn add_account(refresh_token: String) -> Result<models::Account, String> {
    let token_res = modules::oauth::refresh_access_token(&refresh_token).await?;
    let user_info = modules::oauth::get_user_info(&token_res.access_token).await?;

    let token = models::TokenData::new(
        token_res.access_token,
        refresh_token,
        token_res.expires_in,
        Some(user_info.email.clone()),
        None,
        None,
    );

    let account =
        modules::upsert_account(user_info.email.clone(), user_info.get_display_name(), token)?;
    modules::logger::log_info(&format!("添加账号成功: {}", account.email));

    // 广播通知
    modules::websocket::broadcast_data_changed("account_added");

    Ok(account)
}

#[tauri::command]
pub async fn delete_account(account_id: String) -> Result<(), String> {
    modules::delete_account(&account_id)?;
    modules::websocket::broadcast_data_changed("account_deleted");
    Ok(())
}

#[tauri::command]
pub async fn delete_accounts(account_ids: Vec<String>) -> Result<(), String> {
    modules::delete_accounts(&account_ids)?;
    modules::websocket::broadcast_data_changed("accounts_deleted");
    Ok(())
}

#[tauri::command]
pub async fn reorder_accounts(account_ids: Vec<String>) -> Result<(), String> {
    modules::reorder_accounts(&account_ids)
}

#[tauri::command]
pub async fn get_current_account() -> Result<Option<models::Account>, String> {
    modules::get_current_account()
}

#[tauri::command]
pub async fn set_current_account(app: tauri::AppHandle, account_id: String) -> Result<(), String> {
    modules::set_current_account_id(&account_id)?;
    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

#[tauri::command]
pub async fn fetch_account_quota(account_id: String) -> AppResult<models::Account> {
    let mut account = modules::load_account(&account_id).map_err(AppError::Account)?;
    let quota = modules::fetch_quota_with_fresh_token(&mut account, true).await?;
    modules::update_account_quota(&account_id, quota).map_err(AppError::Account)?;
    // 重载账号，包含写入的 quota_error 等最新信息
    let updated_account = modules::load_account(&account_id).map_err(AppError::Account)?;
    Ok(updated_account)
}

#[tauri::command]
pub async fn refresh_all_quotas(
    app: tauri::AppHandle,
) -> Result<modules::account::RefreshStats, String> {
    let result = modules::account::refresh_all_quotas_logic(
        modules::account::QuotaRefreshTrigger::ManualBatch,
    )
    .await;
    if result.is_ok() {
        let mut switched = false;
        match modules::account::run_auto_switch_if_needed().await {
            Ok(Some(account)) => {
                modules::logger::log_info(&format!("[AutoSwitch] 自动切号完成: {}", account.email));
                switched = true;
            }
            Ok(None) => {}
            Err(e) => {
                modules::logger::log_warn(&format!("[AutoSwitch] 自动切号执行失败: {}", e));
            }
        }
        if !switched {
            if let Err(e) = modules::account::run_quota_alert_if_needed() {
                modules::logger::log_warn(&format!("[QuotaAlert] 预警检查失败: {}", e));
            }
        }
        let _ = crate::modules::tray::update_tray_menu(&app);
    }
    result
}

#[tauri::command]
pub async fn refresh_current_quota(app: tauri::AppHandle) -> Result<(), String> {
    let Some(account) = modules::get_current_account().map_err(|e| e.to_string())? else {
        return Err("未找到当前账号".to_string());
    };
    let mut account = account;
    let quota = modules::fetch_quota_with_fresh_token(&mut account, true)
        .await
        .map_err(|e| e.to_string())?;
    modules::update_account_quota(&account.id, quota).map_err(|e| e.to_string())?;

    let mut switched = false;
    match modules::account::run_auto_switch_if_needed().await {
        Ok(Some(account)) => {
            modules::logger::log_info(&format!(
                "[AutoSwitch] 当前账号刷新后自动切号完成: {}",
                account.email
            ));
            switched = true;
        }
        Ok(None) => {}
        Err(e) => {
            modules::logger::log_warn(&format!("[AutoSwitch] 当前账号刷新后自动切号失败: {}", e));
        }
    }

    if !switched {
        if let Err(e) = modules::account::run_quota_alert_if_needed() {
            modules::logger::log_warn(&format!("[QuotaAlert] 当前账号刷新后预警检查失败: {}", e));
        }
    }

    let _ = crate::modules::tray::update_tray_menu(&app);
    Ok(())
}

/// 切换账号（完整流程：Token刷新 + 关闭程序 + 注入 + 指纹同步 + 重启）
#[tauri::command]
pub async fn switch_account(app: AppHandle, account_id: String) -> Result<models::Account, String> {
    if modules::config::get_user_config().antigravity_dual_switch_no_restart_enabled {
        let result = modules::account::switch_account_dual_no_restart(
            &account_id,
            "manual",
            "tools.account.switch",
            "dual_no_restart",
            None,
        )
        .await;
        if let Err(error) = &result {
            if error.starts_with("APP_PATH_NOT_FOUND:") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({
                        "app": "antigravity",
                        "retry": { "kind": "switchAccount", "accountId": account_id }
                    }),
                );
            }
        }
        return result;
    }

    modules::logger::log_info(&format!("开始切换账号: {}", account_id));

    // 1. 加载并验证账号存在
    let mut account = modules::load_account(&account_id)?;
    modules::logger::log_info(&format!(
        "正在切换到账号: {} (ID: {})",
        account.email, account.id
    ));

    // 预检应用路径：路径缺失时只触发引导，不执行任何关闭/注入动作。
    if let Err(e) = modules::process::ensure_antigravity_launch_path_configured() {
        if e.starts_with("APP_PATH_NOT_FOUND:") {
            let _ = app.emit(
                "app:path_missing",
                serde_json::json!({
                    "app": "antigravity",
                    "retry": { "kind": "switchAccount", "accountId": account_id }
                }),
            );
        }
        return Err(e);
    }

    // 2. 确保 Token 有效（自动刷新过期的 Token）
    let fresh_token = modules::oauth::ensure_fresh_token(&account.token)
        .await
        .map_err(|e| format!("Token 刷新失败: {}", e))?;

    // 如果 Token 更新了，保存回账号文件
    if fresh_token.access_token != account.token.access_token {
        modules::logger::log_info(&format!("Token 已刷新: {}", account.email));
        account.token = fresh_token.clone();
        modules::save_account(&account)?;
    }

    // 3. 更新工具内部状态
    modules::set_current_account_id(&account_id)?;
    account.update_last_used();
    modules::save_account(&account)?;

    // 4. 同步更新 Antigravity 默认实例的绑定账号（不同步到 Codex，因为账号体系不同）
    if let Err(e) = modules::instance::update_default_settings(
        Some(Some(account_id.clone())),
        None,
        Some(false),
    ) {
        modules::logger::log_warn(&format!("更新 Antigravity 默认实例绑定账号失败: {}", e));
    } else {
        modules::logger::log_info(&format!(
            "已同步更新 Antigravity 默认实例绑定账号: {}",
            account_id
        ));
    }

    // 5. 关闭受管进程：按默认实例目录关闭受管进程，等待其完全退出
    let default_dir = modules::instance::get_default_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();
    modules::process::close_antigravity_instances(&[default_dir_str], 20)?;
    let _ = modules::instance::update_default_pid(None);

    // 6. 进程完全退出后，执行磁盘级别的文件注入
    // 6.1 写入设备指纹到 storage.json 和 state.vscdb
    if let Ok(storage_path) = modules::device::get_storage_path() {
        if let Some(ref fp_id) = account.fingerprint_id {
            // 优先使用绑定的指纹
            if let Ok(fingerprint) = modules::fingerprint::get_fingerprint(fp_id) {
                modules::logger::log_info(&format!(
                    "写入设备指纹: machineId={}, serviceMachineId={}",
                    fingerprint.profile.machine_id, fingerprint.profile.service_machine_id
                ));
                let _ = modules::device::write_profile(&storage_path, &fingerprint.profile);
                let _ =
                    modules::db::write_service_machine_id(&fingerprint.profile.service_machine_id);
                // 更新当前应用的指纹ID
                let _ = modules::fingerprint::set_current_fingerprint_id(fp_id);
            }
        }
    }

    // 6.2 将账号 Token 注入默认实例目录
    modules::instance::inject_account_to_profile(&default_dir, &account_id)?;

    // 7. 启动 Antigravity（带默认实例自定义启动参数；启动失败不阻断切号，保持原行为）
    modules::logger::log_info("正在启动 Antigravity 默认实例...");
    let default_settings = modules::instance::load_default_settings()?;
    let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
    let launch_result = if extra_args.is_empty() {
        modules::process::start_antigravity()
    } else {
        modules::process::start_antigravity_with_args("", &extra_args)
    };
    let launch_error = match launch_result {
        Ok(pid) => {
            if let Err(e) = modules::instance::update_default_pid(Some(pid)) {
                modules::logger::log_warn(&format!("更新默认实例 PID 失败: {}", e));
            }
            None
        }
        Err(e) => {
            modules::logger::log_warn(&format!("Antigravity 启动失败: {}", e));
            if e.starts_with("APP_PATH_NOT_FOUND:") {
                let _ = app.emit(
                    "app:path_missing",
                    serde_json::json!({ "app": "antigravity", "retry": { "kind": "default" } }),
                );
            }
            Some(e)
        }
    };

    if let Some(err) = launch_error {
        // 账号状态已经切换成功，仍广播账号切换事件，确保前端状态与本地落盘一致
        modules::websocket::broadcast_account_switched(&account.id, &account.email);
        if err.starts_with("APP_PATH_NOT_FOUND:") {
            return Err(err);
        }
        return Err(format!("账号已切换，但启动 Antigravity 失败: {}", err));
    }

    modules::logger::log_info(&format!("账号切换完成: {}", account.email));

    // 广播切换完成通知
    modules::websocket::broadcast_account_switched(&account.id, &account.email);

    Ok(account)
}

#[tauri::command]
pub fn load_antigravity_switch_history(
) -> Result<Vec<modules::antigravity_switch_history::AntigravitySwitchHistoryItem>, String> {
    modules::antigravity_switch_history::load_history()
}

#[tauri::command]
pub fn clear_antigravity_switch_history() -> Result<(), String> {
    modules::antigravity_switch_history::clear_history()
}

#[tauri::command]
pub async fn bind_account_fingerprint(
    account_id: String,
    fingerprint_id: String,
) -> Result<(), String> {
    let mut account = modules::load_account(&account_id)?;
    // 验证指纹存在
    let _ = modules::fingerprint::get_fingerprint(&fingerprint_id)?;
    account.fingerprint_id = Some(fingerprint_id);
    modules::save_account(&account)
}

#[tauri::command]
pub async fn update_account_tags(
    account_id: String,
    tags: Vec<String>,
) -> Result<models::Account, String> {
    let account = modules::update_account_tags(&account_id, tags)?;
    modules::websocket::broadcast_data_changed("account_tags_updated");
    Ok(account)
}

#[tauri::command]
pub async fn update_account_notes(
    account_id: String,
    notes: String,
) -> Result<models::Account, String> {
    let account = modules::account::update_account_notes(&account_id, notes)?;
    Ok(account)
}

#[tauri::command]
pub async fn get_bound_accounts(fingerprint_id: String) -> Result<Vec<models::Account>, String> {
    modules::fingerprint::get_bound_accounts(&fingerprint_id)
}

/// 从本地客户端同步当前账号状态
/// 当前实现已禁用“跟随本地客户端当前账号”，保留空结果以兼容旧调用。
#[tauri::command]
pub async fn sync_current_from_client(_app: tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(None)
}

// ─── 账号分组持久化 ────────────────────────────────────────────

const GROUPS_FILE: &str = "account_groups.json";

#[tauri::command]
pub async fn load_account_groups() -> Result<String, String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let path = home.join(".antigravity_cockpit").join(GROUPS_FILE);
    if !path.exists() {
        return Ok("[]".to_string());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read groups: {}", e))
}

#[tauri::command]
pub async fn save_account_groups(data: String) -> Result<(), String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let dir = home.join(".antigravity_cockpit");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    }
    let path = dir.join(GROUPS_FILE);
    std::fs::write(&path, data).map_err(|e| format!("Failed to write groups: {}", e))
}
