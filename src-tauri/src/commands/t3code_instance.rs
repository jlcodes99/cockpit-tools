use std::path::Path;

use crate::models::InstanceProfileView;
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn to_view(profile: crate::models::InstanceProfile, pid: Option<u32>) -> InstanceProfileView {
    let initialized =
        modules::t3code_instance::is_profile_initialized(Path::new(&profile.user_data_dir), false);
    let mut view = InstanceProfileView::from_profile(profile, pid.is_some(), initialized);
    view.last_pid = pid;
    view
}

fn default_view() -> Result<InstanceProfileView, String> {
    let store = modules::t3code_instance::load_instance_store()?;
    let settings = store.default_settings;
    let dir = modules::t3code_account::default_t3_base_dir()?;
    let pid = modules::t3code_instance::resolve_pid(None, settings.last_pid);
    Ok(InstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: dir.to_string_lossy().to_string(),
        working_dir: None,
        extra_args: settings.extra_args,
        bind_account_id: settings.bind_account_id,
        created_at: 0,
        last_launched_at: None,
        last_pid: pid,
        running: pid.is_some(),
        initialized: modules::t3code_instance::is_profile_initialized(&dir, true),
        is_default: true,
        follow_local_account: false,
    })
}

fn inject_bound_account(root: Option<&Path>, account_id: Option<&str>) -> Result<(), String> {
    let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if let Some(root) = root {
        modules::t3code_account::inject_to_instance_root(account_id, root)?;
    } else {
        modules::t3code_account::inject_to_default(account_id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn t3code_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::t3code_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn t3code_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::t3code_instance::load_instance_store()?;
    let mut values: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|profile| {
            let pid = modules::t3code_instance::resolve_pid(
                Some(Path::new(&profile.user_data_dir)),
                profile.last_pid,
            );
            to_view(profile, pid)
        })
        .collect();
    values.push(default_view()?);
    Ok(values)
}

#[tauri::command]
pub async fn t3code_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let profile = modules::t3code_instance::create_instance(
        modules::t3code_instance::CreateInstanceParams {
            name,
            user_data_dir,
            working_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
        },
    )?;
    Ok(to_view(profile, None))
}

#[tauri::command]
pub async fn t3code_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        modules::t3code_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
        )?;
        return default_view();
    }
    let profile = modules::t3code_instance::update_instance(
        modules::t3code_instance::UpdateInstanceParams {
            instance_id,
            name,
            working_dir,
            extra_args,
            bind_account_id,
        },
    )?;
    let pid = modules::t3code_instance::resolve_pid(
        Some(Path::new(&profile.user_data_dir)),
        profile.last_pid,
    );
    Ok(to_view(profile, pid))
}

#[tauri::command]
pub async fn t3code_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    let store = modules::t3code_instance::load_instance_store()?;
    let profile = store
        .instances
        .iter()
        .find(|profile| profile.id == instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    if modules::t3code_instance::resolve_pid(
        Some(Path::new(&profile.user_data_dir)),
        profile.last_pid,
    )
    .is_some()
    {
        return Err("请先停止 T3 Code 实例再删除".to_string());
    }
    modules::t3code_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn t3code_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    let stop_id = instance_id.clone();
    tokio::task::spawn_blocking(move || stop_and_inject_before_start(&stop_id))
        .await
        .map_err(|error| error.to_string())??;

    modules::t3code_provider_bindings::prepare_before_t3_launch(Some(&instance_id)).await;

    tokio::task::spawn_blocking(move || launch_after_prepare(&instance_id))
        .await
        .map_err(|error| error.to_string())?
}

fn stop_and_inject_before_start(instance_id: &str) -> Result<(), String> {
    modules::t3code_instance::resolve_executable()?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = modules::t3code_instance::load_default_settings()?;
        if let Some(pid) = modules::t3code_instance::resolve_pid(None, settings.last_pid) {
            modules::process::close_pid(pid, 20)?;
        }
        inject_bound_account(None, settings.bind_account_id.as_deref())?;
        return Ok(());
    }

    let store = modules::t3code_instance::load_instance_store()?;
    let profile = store
        .instances
        .into_iter()
        .find(|profile| profile.id == instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    if let Some(pid) = modules::t3code_instance::resolve_pid(
        Some(Path::new(&profile.user_data_dir)),
        profile.last_pid,
    ) {
        modules::process::close_pid(pid, 20)?;
    }
    inject_bound_account(
        Some(Path::new(&profile.user_data_dir)),
        profile.bind_account_id.as_deref(),
    )
}

fn launch_after_prepare(instance_id: &str) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = modules::t3code_instance::load_default_settings()?;
        let args = modules::process::parse_extra_args(&settings.extra_args);
        let pid = modules::t3code_instance::start_default(&args)?;
        modules::t3code_instance::mark_started(None, pid)?;
        return default_view();
    }

    let store = modules::t3code_instance::load_instance_store()?;
    let profile = store
        .instances
        .into_iter()
        .find(|profile| profile.id == instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    let args = modules::process::parse_extra_args(&profile.extra_args);
    let pid = modules::t3code_instance::start_managed(&profile, &args)?;
    modules::t3code_instance::mark_started(Some(&profile.id), pid)?;
    Ok(to_view(profile, Some(pid)))
}

#[tauri::command]
pub async fn t3code_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = modules::t3code_instance::load_default_settings()?;
        if let Some(pid) = modules::t3code_instance::resolve_pid(None, settings.last_pid) {
            modules::process::close_pid(pid, 20)?;
        }
        modules::t3code_instance::mark_stopped(None)?;
        return default_view();
    }
    let store = modules::t3code_instance::load_instance_store()?;
    let profile = store
        .instances
        .into_iter()
        .find(|profile| profile.id == instance_id)
        .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
    if let Some(pid) = modules::t3code_instance::resolve_pid(
        Some(Path::new(&profile.user_data_dir)),
        profile.last_pid,
    ) {
        modules::process::close_pid(pid, 20)?;
    }
    modules::t3code_instance::mark_stopped(Some(&profile.id))?;
    Ok(to_view(profile, None))
}

#[tauri::command]
pub async fn t3code_open_instance_window(instance_id: String) -> Result<(), String> {
    let pid = if instance_id == DEFAULT_INSTANCE_ID {
        let settings = modules::t3code_instance::load_default_settings()?;
        modules::t3code_instance::resolve_pid(None, settings.last_pid)
    } else {
        let store = modules::t3code_instance::load_instance_store()?;
        let profile = store
            .instances
            .into_iter()
            .find(|profile| profile.id == instance_id)
            .ok_or_else(|| "T3 Code 实例不存在".to_string())?;
        modules::t3code_instance::resolve_pid(
            Some(Path::new(&profile.user_data_dir)),
            profile.last_pid,
        )
    }
    .ok_or_else(|| "T3 Code 实例未运行".to_string())?;
    modules::process::focus_process_pid(pid)
        .map(|_| ())
        .map_err(|error| format!("定位 T3 Code 窗口失败: {}", error))
}

#[tauri::command]
pub async fn t3code_close_all_instances() -> Result<(), String> {
    modules::t3code_instance::close_all()
}
