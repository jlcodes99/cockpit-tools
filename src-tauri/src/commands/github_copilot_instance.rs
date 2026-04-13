use std::path::Path;

use crate::models::{DefaultInstanceSettings, InstanceProfileView};
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

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

    let account = modules::github_copilot_account::load_account(bind_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;
    modules::logger::log_info(&format!(
        "实例启动检测到绑定账号，准备注入: bind_account_id={}, login={}, user_data_dir={}",
        bind_id, account.github_login, user_data_dir
    ));

    // Ensure DB is writable before injection.
    modules::process::close_vscode(&[user_data_dir.to_string()], 20)?;

    modules::logger::log_info("正在向实例目录注入 GitHub Copilot Token...");
    let github_id = account.github_id.to_string();
    modules::vscode_inject::inject_copilot_token_for_user_data_dir(
        user_data_dir,
        &account.github_login,
        &account.github_access_token,
        Some(&github_id),
    )
    .map_err(|e| {
        modules::logger::log_error(&format!("实例绑定账号注入失败: {}", e));
        format!("按绑定账号注入实例失败（{}）: {}", account.github_login, e)
    })?;

    modules::logger::log_info(&format!("实例绑定账号注入完成: {}", account.github_login));

    Ok(())
}

#[tauri::command]
pub async fn github_copilot_get_instance_defaults(
) -> Result<modules::instance::InstanceDefaults, String> {
    modules::github_copilot_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn github_copilot_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::github_copilot_instance::load_instance_store()?;
    let default_dir = modules::github_copilot_instance::get_default_vscode_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();
    let process_entries = modules::process::collect_vscode_process_entries();
    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let resolved_pid = modules::process::resolve_vscode_pid_from_entries(
                instance.last_pid,
                Some(&instance.user_data_dir),
                &process_entries,
            );
            let running = resolved_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = resolved_pid;
            view
        })
        .collect();

    let default_pid = modules::process::resolve_vscode_pid_from_entries(
        default_settings.last_pid,
        None,
        &process_entries,
    );
    let default_running = default_pid.is_some();
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
        running: default_running,
        initialized: is_profile_initialized(&default_dir.to_string_lossy()),
        is_default: true,
        follow_local_account: false,
    });

    Ok(result)
}

#[tauri::command]
pub async fn github_copilot_create_instance(
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance = modules::github_copilot_instance::create_instance(
        modules::github_copilot_instance::CreateInstanceParams {
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
pub async fn github_copilot_update_instance(
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::github_copilot_instance::get_default_vscode_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::github_copilot_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        let running = updated
            .last_pid
            .map(modules::process::is_pid_running)
            .unwrap_or(false);
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: updated.extra_args,
            bind_account_id: updated.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: updated.last_pid,
            running,
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let wants_bind = bind_account_id
        .as_ref()
        .and_then(|next| next.as_ref())
        .is_some();
    if wants_bind {
        let store = modules::github_copilot_instance::load_instance_store()?;
        if let Some(target) = store.instances.iter().find(|item| item.id == instance_id) {
            if !is_profile_initialized(&target.user_data_dir) {
                return Err(
                    "INSTANCE_NOT_INITIALIZED:请先启动一次实例创建数据后，再进行账号绑定"
                        .to_string(),
                );
            }
        }
    }

    let instance = modules::github_copilot_instance::update_instance(
        modules::github_copilot_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        },
    )?;

    let running = instance
        .last_pid
        .map(modules::process::is_pid_running)
        .unwrap_or(false);
    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        instance,
        running,
        initialized,
    ))
}

#[tauri::command]
pub async fn github_copilot_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::github_copilot_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn github_copilot_start_instance(
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    modules::logger::log_info(&format!("开始启动 GitHub Copilot 实例: {}", instance_id));
    modules::process::ensure_vscode_launch_path_configured()?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::github_copilot_instance::get_default_vscode_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::github_copilot_instance::load_default_settings()?;
        if let Some(pid) = modules::process::resolve_vscode_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::github_copilot_instance::update_default_pid(None)?;
        }
        modules::process::close_vscode(&[default_dir_str.clone()], 20)?;
        inject_bound_account_for_instance_start(
            &default_dir_str,
            default_settings.bind_account_id.as_deref(),
        )?;
        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid =
            modules::process::start_vscode_default_with_args_with_new_window(&extra_args, true)?;
        modules::logger::log_info(&format!("GitHub Copilot 默认实例已启动: pid={}", pid));
        let _ = modules::github_copilot_instance::update_default_pid(Some(pid))?;
        let running = modules::process::is_pid_running(pid);
        return Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: default_dir_str,
            working_dir: None,
            extra_args: default_settings.extra_args,
            bind_account_id: default_settings.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: Some(pid),
            running,
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let store = modules::github_copilot_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) =
        modules::process::resolve_vscode_pid(instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::github_copilot_instance::update_instance_pid(&instance.id, None)?;
    }
    modules::process::close_vscode(&[instance.user_data_dir.clone()], 20)?;

    inject_bound_account_for_instance_start(
        &instance.user_data_dir,
        instance.bind_account_id.as_deref(),
    )?;
    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_vscode_with_args_with_new_window(
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;
    modules::logger::log_info(&format!(
        "GitHub Copilot 实例已启动: instance_id={}, pid={}",
        instance.id, pid
    ));
    let updated = modules::github_copilot_instance::update_instance_after_start(&instance.id, pid)?;
    let running = modules::process::is_pid_running(pid);
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        running,
        initialized,
    ))
}

#[tauri::command]
pub async fn github_copilot_stop_instance(
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::github_copilot_instance::get_default_vscode_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::github_copilot_instance::load_default_settings()?;
        if let Some(pid) = modules::process::resolve_vscode_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
        }
        let updated_settings = modules::github_copilot_instance::update_default_pid(None)?;
        let running = updated_settings
            .last_pid
            .and_then(|pid| modules::process::resolve_vscode_pid(Some(pid), None))
            .is_some();
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
            running,
            initialized: is_profile_initialized(&default_dir.to_string_lossy()),
            is_default: true,
            follow_local_account: false,
        });
    }

    let store = modules::github_copilot_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) =
        modules::process::resolve_vscode_pid(instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
    }
    let updated = modules::github_copilot_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn github_copilot_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings: DefaultInstanceSettings =
            modules::github_copilot_instance::load_default_settings()?;
        modules::process::focus_vscode_instance(default_settings.last_pid, None)
            .map_err(|err| format!("定位 VS Code 默认实例窗口失败: {}", err))?;
        return Ok(());
    }

    let store = modules::github_copilot_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    modules::process::focus_vscode_instance(instance.last_pid, Some(&instance.user_data_dir))
        .map_err(|err| {
            format!(
                "定位 VS Code 实例窗口失败: instance_id={}, err={}",
                instance.id, err
            )
        })?;
    Ok(())
}

#[tauri::command]
pub async fn github_copilot_close_all_instances() -> Result<(), String> {
    let store = modules::github_copilot_instance::load_instance_store()?;
    let default_dir = modules::github_copilot_instance::get_default_vscode_user_data_dir()?;
    let mut target_dirs: Vec<String> = Vec::new();
    target_dirs.push(default_dir.to_string_lossy().to_string());
    for instance in &store.instances {
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }
    modules::process::close_vscode(&target_dirs, 20)?;
    let _ = modules::github_copilot_instance::clear_all_pids();
    Ok(())
}
