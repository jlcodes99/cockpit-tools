use std::fs;
use std::path::{Path, PathBuf};

use crate::models::InstanceProfileView;
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn ensure_workbuddy_state_db_path(user_data_dir: &str) -> Result<PathBuf, String> {
    let root = Path::new(user_data_dir);
    let candidates = vec![
        root.join("User").join("globalStorage").join("state.vscdb"),
        root.join("globalStorage").join("state.vscdb"),
        root.join("state.vscdb"),
    ];

    if let Some(path) = candidates.iter().find(|path| path.exists()) {
        return Ok(path.clone());
    }

    let preferred = root.join("User").join("globalStorage").join("state.vscdb");
    if let Some(parent) = preferred.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 globalStorage 目录失败：{}", e))?;
    }

    if let Some(default_db) = modules::workbuddy_account::get_default_workbuddy_state_db_path() {
        if default_db.exists() && default_db != preferred {
            match fs::copy(&default_db, &preferred) {
                Ok(_) => {
                    modules::logger::log_info(&format!(
                        "[WorkBuddy Instance] 已回退复制默认 state.vscdb: from={}, to={}",
                        default_db.to_string_lossy(),
                        preferred.to_string_lossy()
                    ));
                }
                Err(err) => {
                    modules::logger::log_warn(&format!(
                        "[WorkBuddy Instance] 复制默认 state.vscdb 失败，改为写入新库：from={}, to={}, error={}",
                        default_db.to_string_lossy(),
                        preferred.to_string_lossy(),
                        err
                    ));
                }
            }
        }
    }

    Ok(preferred)
}

fn build_session_json(account: &crate::models::workbuddy::WorkbuddyAccount) -> String {
    let uid = account.uid.as_deref().unwrap_or("");
    let nickname = account.nickname.as_deref().unwrap_or("");
    let enterprise_id = account.enterprise_id.as_deref().unwrap_or("");
    let enterprise_name = account.enterprise_name.as_deref().unwrap_or("");
    let domain = account.domain.as_deref().unwrap_or("");
    let refresh_token = account.refresh_token.as_deref().unwrap_or("");
    let expires_at = account.expires_at.unwrap_or(0);

    let session = serde_json::json!({
        "id": "Tencent-Cloud.genie-ide-cn",
        "token": account.access_token,
        "refreshToken": refresh_token,
        "expiresAt": expires_at,
        "domain": domain,
        "accessToken": format!("{}+{}", uid, account.access_token),
        "converted": true,
        "account": {
            "id": uid,
            "uid": uid,
            "label": nickname,
            "nickname": nickname,
            "enterpriseId": enterprise_id,
            "enterpriseName": enterprise_name,
            "pluginEnabled": true,
            "lastLogin": true,
        },
        "auth": {
            "accessToken": account.access_token,
            "refreshToken": refresh_token,
            "tokenType": account.token_type.as_deref().unwrap_or("Bearer"),
            "domain": domain,
            "expiresAt": expires_at,
            "expiresIn": expires_at,
            "refreshExpiresIn": 0,
            "refreshExpiresAt": 0,
            "lastRefreshTime": chrono::Utc::now().timestamp_millis(),
        }
    });

    session.to_string()
}

fn inject_bound_account_for_instance_start(
    user_data_dir: &str,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let bind_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bind_id) = bind_id else {
        return Ok(());
    };

    let account = modules::workbuddy_account::load_account(bind_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;
    modules::logger::log_info(&format!(
        "[WorkBuddy Instance] 实例启动检测到绑定账号，准备注入: bind_account_id={}, email={}, user_data_dir={}",
        bind_id, account.email, user_data_dir
    ));

    let state_db_path = ensure_workbuddy_state_db_path(user_data_dir)?;
    let session_json = build_session_json(&account);
    let secret_key = r#"{"extensionId":"tencent-cloud.coding-copilot","key":"planning-genie.new.accessTokencn"}"#;
    let db_key = format!("secret://{}", secret_key);

    if let Err(err) = modules::vscode_inject::inject_secret_to_state_db_for_workbuddy(
        &state_db_path,
        &db_key,
        &session_json,
    ) {
        let friendly_err = if err.contains("Safe Storage password")
            || err.contains("Keychain")
            || err.contains("Failed to read")
        {
            format!(
                "注入登录状态失败：{}\n\n可能的原因：\n\
                1. WorkBuddy 从未登录过，请先手动打开 WorkBuddy 并登录一次\n\
                2. macOS Keychain 中缺少加密密钥条目\n\n\
                请尝试：打开 WorkBuddy → 登录任意账号 → 退出 → 再使用切号功能",
                err
            )
        } else {
            err
        };
        return Err(friendly_err);
    }
    verify_state_db_injection(&state_db_path, &db_key)?;

    modules::logger::log_info(&format!(
        "[WorkBuddy Instance] 账号注入完成: email={}, db={}",
        account.email,
        state_db_path.to_string_lossy()
    ));

    Ok(())
}

fn verify_state_db_injection(state_db_path: &Path, db_key: &str) -> Result<(), String> {
    let conn = rusqlite::Connection::open(state_db_path)
        .map_err(|e| format!("注入校验失败，无法打开 state.vscdb: {}", e))?;

    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?1",
            [db_key],
            |row| row.get(0),
        )
        .ok();
    match value {
        Some(stored) if !stored.trim().is_empty() => Ok(()),
        _ => Err(format!(
            "注入校验失败，未在 state.vscdb 找到目标 key: db={}, key={}",
            state_db_path.to_string_lossy(),
            db_key
        )),
    }
}

fn is_profile_initialized(user_data_dir: &str) -> bool {
    let path = Path::new(user_data_dir);
    if !path.exists() {
        return false;
    }
    match std::fs::read_dir(path) {
        Ok(mut iter) => iter.next().is_some(),
        Err(_) => false,
    }
}

fn resolve_running_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    modules::process::resolve_workbuddy_pid(last_pid, user_data_dir)
}

#[tauri::command]
pub async fn workbuddy_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String>
{
    modules::workbuddy_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn workbuddy_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::workbuddy_instance::load_instance_store()?;
    let default_dir = modules::workbuddy_instance::get_default_workbuddy_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();

    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let running_pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir));
            let running = running_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = running_pid;
            view
        })
        .collect();

    let default_pid = resolve_running_pid(default_settings.last_pid, None);
    result.push(InstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: default_dir_str,
        working_dir: None,
        extra_args: default_settings.extra_args.clone(),
        bind_account_id: default_settings.bind_account_id.clone(),
        created_at: 0,
        last_launched_at: None,
        last_pid: default_pid,
        running: default_pid.is_some(),
        initialized: is_profile_initialized(&default_dir.to_string_lossy()),
        is_default: true,
        follow_local_account: false,
    });

    Ok(result)
}

#[tauri::command]
pub async fn workbuddy_create_instance(
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance = modules::workbuddy_instance::create_instance(
        modules::workbuddy_instance::CreateInstanceParams {
            working_dir: None,
            name,
            user_data_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
        },
    )?;

    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn workbuddy_update_instance(
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::workbuddy_instance::get_default_workbuddy_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::workbuddy_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        let running_pid = resolve_running_pid(updated.last_pid, None);
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: updated.extra_args,
            bind_account_id: updated.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: running_pid,
            running: running_pid.is_some(),
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let instance = modules::workbuddy_instance::update_instance(
        modules::workbuddy_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        },
    )?;

    let running_pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir));
    let running = running_pid.is_some();
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view = InstanceProfileView::from_profile(instance, running, initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn workbuddy_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::workbuddy_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn workbuddy_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    modules::process::ensure_workbuddy_launch_path_configured()?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::workbuddy_instance::get_default_workbuddy_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::workbuddy_instance::load_default_settings()?;

        if let Some(pid) = resolve_running_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::workbuddy_instance::update_default_pid(None)?;
        }

        inject_bound_account_for_instance_start(
            &default_dir_str,
            default_settings.bind_account_id.as_deref(),
        )?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid =
            modules::process::start_workbuddy_default_with_args_with_new_window(&extra_args, true)?;
        let _ = modules::workbuddy_instance::update_default_pid(Some(pid))?;
        let running_pid = resolve_running_pid(Some(pid), None);

        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: default_settings.extra_args,
            bind_account_id: default_settings.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: running_pid,
            running: running_pid.is_some(),
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let store = modules::workbuddy_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::workbuddy_instance::update_instance_pid(&instance.id, None)?;
    }

    inject_bound_account_for_instance_start(
        &instance.user_data_dir,
        instance.bind_account_id.as_deref(),
    )?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_workbuddy_with_args_with_new_window(
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;

    let updated = modules::workbuddy_instance::update_instance_after_start(&instance.id, pid)?;
    let running_pid = resolve_running_pid(Some(pid), Some(&updated.user_data_dir));
    let initialized = is_profile_initialized(&updated.user_data_dir);
    let mut view = InstanceProfileView::from_profile(updated, running_pid.is_some(), initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn workbuddy_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::workbuddy_instance::get_default_workbuddy_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::workbuddy_instance::load_default_settings()?;
        if let Some(pid) = resolve_running_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
        }
        let _ = modules::workbuddy_instance::update_default_pid(None)?;
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: default_settings.extra_args,
            bind_account_id: default_settings.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
            running: false,
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let store = modules::workbuddy_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
    }
    let updated = modules::workbuddy_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn workbuddy_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::workbuddy_instance::load_default_settings()?;
        let pid = resolve_running_pid(default_settings.last_pid, None).ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 WorkBuddy 默认实例窗口失败：{}", err))?;
        return Ok(());
    }

    let store = modules::workbuddy_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir))
        .ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 WorkBuddy 实例窗口失败：instance_id={}, err={}",
            instance.id, err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn workbuddy_close_all_instances() -> Result<(), String> {
    let store = modules::workbuddy_instance::load_instance_store()?;
    let default_settings = modules::workbuddy_instance::load_default_settings()?;

    if let Some(pid) = resolve_running_pid(default_settings.last_pid, None) {
        let _ = modules::process::close_pid(pid, 20);
    }

    for instance in &store.instances {
        if let Some(pid) = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir)) {
            let _ = modules::process::close_pid(pid, 20);
        }
    }

    let _ = modules::workbuddy_instance::clear_all_pids();
    Ok(())
}
