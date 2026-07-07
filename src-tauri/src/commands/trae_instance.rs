use std::path::Path;

use crate::models::InstanceProfileView;
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn parse_platform(
    platform_id: Option<String>,
) -> Result<modules::trae_account::TraePlatformKind, String> {
    modules::trae_account::TraePlatformKind::parse(platform_id.as_deref())
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
    platform: modules::trae_account::TraePlatformKind,
    last_pid: Option<u32>,
    user_data_dir: Option<&str>,
) -> Option<u32> {
    modules::process::resolve_trae_pid_for_platform(last_pid, user_data_dir, platform)
}

async fn inject_bound_account(
    user_data_dir: &str,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let Some(account_id) = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    if let Ok(accounts) = modules::trae_account::list_accounts_checked() {
        let protection_map =
            modules::trae_account::resolve_running_account_refresh_protection_map(&accounts);
        if let Some(storage_path) = protection_map.get(account_id) {
            modules::logger::log_info(&format!(
                "[Trae Instance] 启动前命中运行中账号保护，改为仅额度刷新: account_id={}, storage_path={}",
                account_id,
                storage_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            ));
            modules::trae_account::refresh_account_usage_only_async(
                account_id,
                storage_path.as_deref(),
            )
            .await
            .map_err(|err| format!("Trae 实例启动前刷新账号失败({}): {}", account_id, err))?;
        } else {
            modules::trae_account::refresh_account_async(account_id)
                .await
                .map_err(|err| format!("Trae 实例启动前刷新账号失败({}): {}", account_id, err))?;
        }
    } else {
        modules::trae_account::refresh_account_async(account_id)
            .await
            .map_err(|err| format!("Trae 实例启动前刷新账号失败({}): {}", account_id, err))?;
    }

    let storage_path = modules::trae_instance::build_storage_json_path(user_data_dir);
    modules::trae_account::inject_to_trae_at_path(storage_path.as_path(), account_id)
}

async fn verify_bound_account_after_start(user_data_dir: &str, bind_account_id: Option<&str>) {
    let Some(account_id) = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    match modules::trae_account::check_login_then_refresh_if_needed(account_id).await {
        Ok(true) => {
            let storage_path = modules::trae_instance::build_storage_json_path(user_data_dir);
            if let Err(err) =
                modules::trae_account::inject_to_trae_at_path(storage_path.as_path(), account_id)
            {
                modules::logger::log_warn(&format!(
                    "[Trae Instance] 启动后静默刷新成功，但账号回写实例失败: account_id={}, error={}",
                    account_id, err
                ));
            } else {
                modules::logger::log_info(&format!(
                    "[Trae Instance] 启动后严格校验触发静默刷新并回写: account_id={}",
                    account_id
                ));
            }
        }
        Ok(false) => {
            modules::logger::log_info(&format!(
                "[Trae Instance] 启动后无需执行 Token 静默刷新: account_id={}",
                account_id
            ));
        }
        Err(err) => {
            modules::logger::log_warn(&format!(
                "[Trae Instance] 启动后严格校验失败，跳过静默刷新: account_id={}, error={}",
                account_id, err
            ));
        }
    }
}

#[tauri::command]
pub async fn trae_get_instance_defaults(
    platform_id: Option<String>,
) -> Result<modules::instance::InstanceDefaults, String> {
    let platform = parse_platform(platform_id)?;
    modules::trae_instance::get_instance_defaults_for_platform(platform)
}

#[tauri::command]
pub async fn trae_list_instances(
    platform_id: Option<String>,
) -> Result<Vec<InstanceProfileView>, String> {
    let platform = parse_platform(platform_id)?;
    let store = modules::trae_instance::load_instance_store_for_platform(platform)?;
    let default_dir =
        modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();
    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let running_pid =
                resolve_running_pid(platform, instance.last_pid, Some(&instance.user_data_dir));
            let running = running_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = running_pid;
            view
        })
        .collect();

    let default_pid = resolve_running_pid(platform, default_settings.last_pid, None);
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
    platform_id: Option<String>,
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let platform = parse_platform(platform_id)?;
    let instance = modules::trae_instance::create_instance_for_platform(
        platform,
        modules::trae_instance::CreateInstanceParams {
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
pub async fn trae_update_instance(
    platform_id: Option<String>,
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    let platform = parse_platform(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir =
            modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::trae_instance::update_default_settings_for_platform(
            platform,
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        let running_pid = resolve_running_pid(platform, updated.last_pid, None);
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

    let instance = modules::trae_instance::update_instance_for_platform(
        platform,
        modules::trae_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        },
    )?;

    let running_pid =
        resolve_running_pid(platform, instance.last_pid, Some(&instance.user_data_dir));
    let running = running_pid.is_some();
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view = InstanceProfileView::from_profile(instance, running, initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn trae_delete_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<(), String> {
    let platform = parse_platform(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::trae_instance::delete_instance_for_platform(platform, &instance_id)
}

#[tauri::command]
pub async fn trae_start_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    let platform = parse_platform(platform_id)?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir =
            modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings =
            modules::trae_instance::load_default_settings_for_platform(platform)?;

        if let Some(pid) = resolve_running_pid(platform, default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::trae_instance::update_default_pid_for_platform(platform, None)?;
        }
        modules::process::close_trae_platform_instances(platform, &[default_dir_str.clone()], 20)?;
        let _ = modules::trae_instance::update_default_pid_for_platform(platform, None)?;

        inject_bound_account(
            default_dir_str.as_str(),
            default_settings.bind_account_id.as_deref(),
        )
        .await?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid = modules::process::start_trae_platform_default_with_args_with_new_window(
            platform.provider_key(),
            &extra_args,
            true,
        )?;
        let _ = modules::trae_instance::update_default_pid_for_platform(platform, Some(pid))?;
        verify_bound_account_after_start(
            default_dir_str.as_str(),
            default_settings.bind_account_id.as_deref(),
        )
        .await;
        let running_pid = resolve_running_pid(platform, Some(pid), None);

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

    let store = modules::trae_instance::load_instance_store_for_platform(platform)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) =
        resolve_running_pid(platform, instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
        let _ =
            modules::trae_instance::update_instance_pid_for_platform(platform, &instance.id, None)?;
    }
    modules::process::close_trae_platform_instances(
        platform,
        &[instance.user_data_dir.clone()],
        20,
    )?;
    let _ = modules::trae_instance::update_instance_pid_for_platform(platform, &instance.id, None)?;

    inject_bound_account(&instance.user_data_dir, instance.bind_account_id.as_deref()).await?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_trae_platform_with_args_with_new_window(
        platform.provider_key(),
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;
    let updated = modules::trae_instance::update_instance_after_start_for_platform(
        platform,
        &instance.id,
        pid,
    )?;
    verify_bound_account_after_start(&instance.user_data_dir, instance.bind_account_id.as_deref())
        .await;

    let running_pid = resolve_running_pid(platform, Some(pid), Some(&updated.user_data_dir));
    let initialized = is_profile_initialized(&updated.user_data_dir);
    let mut view = InstanceProfileView::from_profile(updated, running_pid.is_some(), initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn trae_stop_instance(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<InstanceProfileView, String> {
    let platform = parse_platform(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir =
            modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings =
            modules::trae_instance::load_default_settings_for_platform(platform)?;
        if let Some(pid) = resolve_running_pid(platform, default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
        }
        modules::process::close_trae_platform_instances(platform, &[default_dir_str.clone()], 20)?;
        let _ = modules::trae_instance::update_default_pid_for_platform(platform, None)?;
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

    let store = modules::trae_instance::load_instance_store_for_platform(platform)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) =
        resolve_running_pid(platform, instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
    }
    modules::process::close_trae_platform_instances(
        platform,
        &[instance.user_data_dir.clone()],
        20,
    )?;
    let updated =
        modules::trae_instance::update_instance_pid_for_platform(platform, &instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn trae_open_instance_window(
    platform_id: Option<String>,
    instance_id: String,
) -> Result<(), String> {
    let platform = parse_platform(platform_id)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings =
            modules::trae_instance::load_default_settings_for_platform(platform)?;
        let pid = resolve_running_pid(platform, default_settings.last_pid, None)
            .ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 {} 默认实例窗口失败: {}", platform.display_name(), err))?;
        return Ok(());
    }

    let store = modules::trae_instance::load_instance_store_for_platform(platform)?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(platform, instance.last_pid, Some(&instance.user_data_dir))
        .ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 {} 实例窗口失败: instance_id={}, err={}",
            platform.display_name(),
            instance.id,
            err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn trae_close_all_instances(platform_id: Option<String>) -> Result<(), String> {
    let platform = parse_platform(platform_id)?;
    let store = modules::trae_instance::load_instance_store_for_platform(platform)?;
    let default_dir =
        modules::trae_instance::get_default_trae_user_data_dir_for_platform(platform)?;
    let mut target_dirs: Vec<String> = Vec::new();
    target_dirs.push(default_dir.to_string_lossy().to_string());
    for instance in &store.instances {
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }

    modules::process::close_trae_platform_instances(platform, &target_dirs, 20)?;
    let _ = modules::trae_instance::clear_all_pids_for_platform(platform);
    Ok(())
}
