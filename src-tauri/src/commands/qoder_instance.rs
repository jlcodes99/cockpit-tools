use std::path::Path;

use crate::models::InstanceProfileView;
use crate::modules;
use crate::modules::qoder_channel::QoderChannel;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn parse_channel(platform_id: Option<String>) -> Result<QoderChannel, String> {
    QoderChannel::parse(platform_id.as_deref())
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

fn resolve_running_pid(
    channel: QoderChannel,
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Option<u32> {
    modules::process::resolve_qoder_pid_for_channel(last_pid, user_data_dir, channel)
}

fn inject_bound_account_for_instance_start(
    channel: QoderChannel,
    user_data_dir: &str,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let bind_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bind_id) = bind_id else {
        return Ok(());
    };

    let account = modules::qoder_account::load_account_for_channel(channel, bind_id)
        .or_else(|| modules::qoder_account::load_account(bind_id))
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;
    modules::logger::log_info(&format!(
        "[{}] 实例启动检测到绑定账号，准备注入: bind_account_id={}, email={}, user_data_dir={}",
        channel.display_name(),
        bind_id,
        account.email,
        user_data_dir
    ));

    modules::qoder_account::inject_to_qoder_channel_for_user_data_dir(channel, user_data_dir, bind_id)?;
    Ok(())
}

#[tauri::command]
pub async fn qoder_get_instance_defaults(
    platform_id: Option<String>,
) -> Result<modules::instance::InstanceDefaults, String> {
    let channel = parse_channel(platform_id)?;
    modules::qoder_instance::get_instance_defaults_for_channel(channel)
}

#[tauri::command]
pub async fn qoder_list_instances(
    platform_id: Option<String>,
) -> Result<Vec<InstanceProfileView>, String> {
    let channel = parse_channel(platform_id)?;
    let store = modules::qoder_instance::load_instance_store_for_channel(channel)?;
    let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir_for_channel(channel)?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();

    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let running_pid = resolve_running_pid(channel, instance.last_pid, Some(&instance.user_data_dir));
            let running = running_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = running_pid;
            view
        })
        .collect();

    let default_pid = resolve_running_pid(channel, default_settings.last_pid, None);
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
pub async fn qoder_create_instance(
    platform_id: Option<String>,
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let channel = parse_channel(platform_id)?;
    let instance =
        modules::qoder_instance::create_instance_for_channel(channel, modules::qoder_instance::CreateInstanceParams {
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
pub async fn qoder_update_instance(
    platform_id: Option<String>,
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    let channel = parse_channel(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir_for_channel(channel)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::qoder_instance::update_default_settings_for_channel(
            channel,
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        let running_pid = resolve_running_pid(channel, updated.last_pid, None);
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
        modules::qoder_instance::update_instance_for_channel(channel, modules::qoder_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        })?;

    let running_pid = resolve_running_pid(channel, instance.last_pid, Some(&instance.user_data_dir));
    let running = running_pid.is_some();
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view = InstanceProfileView::from_profile(instance, running, initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn qoder_delete_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<(), String> {
    let channel = parse_channel(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::qoder_instance::delete_instance_for_channel(channel, &instance_id)
}

#[tauri::command]
pub async fn qoder_start_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    let channel = parse_channel(platform_id)?;
    let _ = modules::process::resolve_qoder_launch_path_for_channel(channel)
        .or_else(|_| modules::process::resolve_qoder_launch_path())?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir_for_channel(channel)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::qoder_instance::load_default_settings_for_channel(channel)?;

        if let Some(pid) = resolve_running_pid(channel, default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::qoder_instance::update_default_pid_for_channel(channel, None)?;
        }
        modules::process::close_qoder_channel_instances(channel, &[default_dir_str.clone()], 20)?;
        let _ = modules::qoder_instance::update_default_pid_for_channel(channel, None)?;

        inject_bound_account_for_instance_start(
            channel,
            &default_dir_str,
            default_settings.bind_account_id.as_deref(),
        )?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid =
            modules::process::start_qoder_channel_default_with_args(channel, &extra_args, true)?;
        let _ = modules::qoder_instance::update_default_pid_for_channel(channel, Some(pid))?;
        let running_pid = resolve_running_pid(channel, Some(pid), None);

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

    let store = modules::qoder_instance::load_instance_store_for_channel(channel)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(channel, instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::qoder_instance::update_instance_pid_for_channel(channel, &instance.id, None)?;
    }
    modules::process::close_qoder_channel_instances(channel, &[instance.user_data_dir.clone()], 20)?;
    let _ = modules::qoder_instance::update_instance_pid_for_channel(channel, &instance.id, None)?;

    inject_bound_account_for_instance_start(
        channel,
        &instance.user_data_dir,
        instance.bind_account_id.as_deref(),
    )?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_qoder_channel_with_args_with_new_window(
        channel,
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;

    let updated = modules::qoder_instance::update_instance_after_start_for_channel(channel, &instance.id, pid)?;
    let running_pid = resolve_running_pid(channel, Some(pid), Some(&updated.user_data_dir));
    let initialized = is_profile_initialized(&updated.user_data_dir);
    let mut view = InstanceProfileView::from_profile(updated, running_pid.is_some(), initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn qoder_stop_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    let channel = parse_channel(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir_for_channel(channel)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::qoder_instance::load_default_settings_for_channel(channel)?;
        if let Some(pid) = resolve_running_pid(channel, default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
        }
        modules::process::close_qoder_channel_instances(channel, &[default_dir_str.clone()], 20)?;
        let _ = modules::qoder_instance::update_default_pid_for_channel(channel, None)?;
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

    let store = modules::qoder_instance::load_instance_store_for_channel(channel)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(channel, instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
    }
    modules::process::close_qoder_channel_instances(channel, &[instance.user_data_dir.clone()], 20)?;
    let updated = modules::qoder_instance::update_instance_pid_for_channel(channel, &instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn qoder_open_instance_window(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<(), String> {
    let channel = parse_channel(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::qoder_instance::load_default_settings_for_channel(channel)?;
        let pid = resolve_running_pid(channel, default_settings.last_pid, None).ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 {} 默认实例窗口失败: {}", channel.display_name(), err))?;
        return Ok(());
    }

    let store = modules::qoder_instance::load_instance_store_for_channel(channel)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(channel, instance.last_pid, Some(&instance.user_data_dir))
        .ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 {} 实例窗口失败: instance_id={}, err={}",
            channel.display_name(),
            instance.id,
            err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn qoder_close_all_instances(
    platform_id: Option<String>,
) -> Result<(), String> {
    let channel = parse_channel(platform_id)?;
    let store = modules::qoder_instance::load_instance_store_for_channel(channel)?;
    let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir_for_channel(channel)?;
    let mut target_dirs: Vec<String> = Vec::new();
    target_dirs.push(default_dir.to_string_lossy().to_string());
    for instance in &store.instances {
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }

    modules::process::close_qoder_channel_instances(channel, &target_dirs, 20)?;
    let _ = modules::qoder_instance::clear_all_pids_for_channel(channel);
    Ok(())
}
