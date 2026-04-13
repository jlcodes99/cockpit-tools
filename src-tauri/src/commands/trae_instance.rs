use std::path::Path;

use crate::models::InstanceProfileView;
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

fn resolve_running_pid(last_pid: Option<u32>) -> Option<u32> {
    let pid = last_pid?;
    if modules::process::is_pid_running(pid) {
        Some(pid)
    } else {
        None
    }
}

fn inject_bound_account(user_data_dir: &str, bind_account_id: Option<&str>) -> Result<(), String> {
    let Some(account_id) = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let storage_path = modules::trae_instance::build_storage_json_path(user_data_dir);
    modules::trae_account::inject_to_trae_at_path(storage_path.as_path(), account_id)
}

#[tauri::command]
pub async fn trae_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::trae_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn trae_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::trae_instance::load_instance_store()?;
    let default_dir = modules::trae_instance::get_default_trae_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();
    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let running_pid = resolve_running_pid(instance.last_pid);
            let running = running_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = running_pid;
            view
        })
        .collect();

    let default_pid = resolve_running_pid(default_settings.last_pid);
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
pub async fn trae_create_instance(
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance =
        modules::trae_instance::create_instance(modules::trae_instance::CreateInstanceParams {
            working_dir: None,
            name,
            user_data_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
        })?;

    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn trae_update_instance(
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::trae_instance::get_default_trae_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::trae_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        let running_pid = resolve_running_pid(updated.last_pid);
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

    let instance =
        modules::trae_instance::update_instance(modules::trae_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        })?;

    let running_pid = resolve_running_pid(instance.last_pid);
    let running = running_pid.is_some();
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view = InstanceProfileView::from_profile(instance, running, initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn trae_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::trae_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn trae_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    modules::process::ensure_trae_launch_path_configured()?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::trae_instance::get_default_trae_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::trae_instance::load_default_settings()?;

        if let Some(pid) = resolve_running_pid(default_settings.last_pid) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::trae_instance::update_default_pid(None)?;
        }

        inject_bound_account(
            default_dir_str.as_str(),
            default_settings.bind_account_id.as_deref(),
        )?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid =
            modules::process::start_trae_default_with_args_with_new_window(&extra_args, true)?;
        let _ = modules::trae_instance::update_default_pid(Some(pid))?;
        let running_pid = resolve_running_pid(Some(pid));

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

    let store = modules::trae_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid) {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::trae_instance::update_instance_pid(&instance.id, None)?;
    }

    inject_bound_account(&instance.user_data_dir, instance.bind_account_id.as_deref())?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_trae_with_args_with_new_window(
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;

    let updated = modules::trae_instance::update_instance_after_start(&instance.id, pid)?;
    let running_pid = resolve_running_pid(Some(pid));
    let initialized = is_profile_initialized(&updated.user_data_dir);
    let mut view = InstanceProfileView::from_profile(updated, running_pid.is_some(), initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn trae_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::trae_instance::get_default_trae_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::trae_instance::load_default_settings()?;
        if let Some(pid) = resolve_running_pid(default_settings.last_pid) {
            modules::process::close_pid(pid, 20)?;
        }
        let _ = modules::trae_instance::update_default_pid(None)?;
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

    let store = modules::trae_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid) {
        modules::process::close_pid(pid, 20)?;
    }
    let updated = modules::trae_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn trae_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::trae_instance::load_default_settings()?;
        let pid = resolve_running_pid(default_settings.last_pid).ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 Trae 默认实例窗口失败: {}", err))?;
        return Ok(());
    }

    let store = modules::trae_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(instance.last_pid).ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 Trae 实例窗口失败: instance_id={}, err={}",
            instance.id, err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn trae_close_all_instances() -> Result<(), String> {
    let store = modules::trae_instance::load_instance_store()?;
    let default_settings = modules::trae_instance::load_default_settings()?;

    if let Some(pid) = resolve_running_pid(default_settings.last_pid) {
        let _ = modules::process::close_pid(pid, 20);
    }

    for instance in &store.instances {
        if let Some(pid) = resolve_running_pid(instance.last_pid) {
            let _ = modules::process::close_pid(pid, 20);
        }
    }

    let _ = modules::trae_instance::clear_all_pids();
    Ok(())
}
