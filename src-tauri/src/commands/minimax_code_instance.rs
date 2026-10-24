//! MiniMax Code 应用多开命令层。
//!
//! 与 `workbuddy_instance.rs` 同构。关键差异：
//! - 实例目录就是客户端的 data root（`MINIMAX_DATA_DIR`），登录态天然按实例隔离；
//! - MiniMax 账号切换是同步的（改写 `auth.json`），没有 WorkBuddy 那样的会话合并；
//! - 客户端不暴露 data root，进程匹配退化为 exe 路径匹配（见各进程层注释）。
use std::path::Path;

use crate::models::InstanceProfileView;
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

async fn inject_bound_account_for_instance_start(
    user_data_dir: &str,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let bind_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bind_id) = bind_id else {
        return Ok(());
    };

    let user_data_dir = user_data_dir.to_string();
    let bind_id = bind_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        inject_bound_account_for_instance_start_blocking(&user_data_dir, &bind_id)
    })
    .await
    .map_err(|error| format!("MiniMax Code 账号切换后台任务失败: {}", error))?
}

fn inject_bound_account_for_instance_start_blocking(
    user_data_dir: &str,
    bind_id: &str,
) -> Result<(), String> {
    let account = modules::minimax_code_account::load_account(bind_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;
    modules::logger::log_info(&format!(
        "[MiniMax Code Instance] 实例启动检测到绑定账号，准备注入: bind_account_id={}, email={}, user_data_dir={}",
        bind_id, account.email, user_data_dir
    ));

    // MiniMax 客户端把登录态写在 `<data root>/auth/<buildEnv>/<region>/<clientId>/auth.json`，
    // 因此直接改写该实例自己的 auth.json 即可，无需 WorkBuddy 那样的会话合并。
    modules::minimax_code_account::switch_minimax_code_account_to_data_dir(
        &account,
        Path::new(user_data_dir),
    )?;

    modules::logger::log_info(&format!(
        "[MiniMax Code Instance] 账号注入完成: email={}, user_data_dir={}",
        account.email, user_data_dir
    ));

    Ok(())
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
    modules::process::resolve_minimax_code_pid(last_pid, user_data_dir)
}

#[tauri::command]
pub async fn minimax_code_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String>
{
    modules::minimax_code_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn minimax_code_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::minimax_code_instance::load_instance_store()?;
    let default_dir = modules::minimax_code_instance::get_default_minimax_code_user_data_dir()?;
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
pub async fn minimax_code_create_instance(
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance = modules::minimax_code_instance::create_instance(
        modules::minimax_code_instance::CreateInstanceParams {
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
pub async fn minimax_code_update_instance(
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::minimax_code_instance::get_default_minimax_code_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::minimax_code_instance::update_default_settings(
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

    let instance = modules::minimax_code_instance::update_instance(
        modules::minimax_code_instance::UpdateInstanceParams {
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
pub async fn minimax_code_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::minimax_code_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn minimax_code_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    modules::process::ensure_minimax_code_launch_path_configured()?;

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::minimax_code_instance::get_default_minimax_code_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::minimax_code_instance::load_default_settings()?;

        if let Some(pid) = resolve_running_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::minimax_code_instance::update_default_pid(None)?;
        }
        modules::process::close_minimax_code_instances(&[default_dir_str.clone()], 20)?;
        let _ = modules::minimax_code_instance::update_default_pid(None)?;

        inject_bound_account_for_instance_start(
            &default_dir_str,
            default_settings.bind_account_id.as_deref(),
        )
        .await?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid = modules::process::start_minimax_code_default_with_args_with_new_window(
            &extra_args,
            true,
        )?;
        let _ = modules::minimax_code_instance::update_default_pid(Some(pid))?;
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

    let store = modules::minimax_code_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::minimax_code_instance::update_instance_pid(&instance.id, None)?;
    }
    modules::process::close_minimax_code_instances(&[instance.user_data_dir.clone()], 20)?;
    let _ = modules::minimax_code_instance::update_instance_pid(&instance.id, None)?;

    inject_bound_account_for_instance_start(
        &instance.user_data_dir,
        instance.bind_account_id.as_deref(),
    )
    .await?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_minimax_code_with_args_with_new_window(
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;

    // 直接记录启动器返回的 PID。`resolve_minimax_code_pid` 只信任持久化的 PID，
    // 不会猜测，因此这里不能再做一次「重新解析」（那会把别的实例的 PID 拿来用）。
    let updated = modules::minimax_code_instance::update_instance_after_start(&instance.id, pid)?;
    let running_pid = resolve_running_pid(Some(pid), Some(&updated.user_data_dir));
    let initialized = is_profile_initialized(&updated.user_data_dir);
    let mut view = InstanceProfileView::from_profile(updated, running_pid.is_some(), initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn minimax_code_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::minimax_code_instance::get_default_minimax_code_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::minimax_code_instance::load_default_settings()?;
        if let Some(pid) = resolve_running_pid(default_settings.last_pid, None) {
            modules::process::close_pid(pid, 20)?;
        }
        modules::process::close_minimax_code_instances(&[default_dir_str.clone()], 20)?;
        let _ = modules::minimax_code_instance::update_default_pid(None)?;
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

    let store = modules::minimax_code_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir)) {
        modules::process::close_pid(pid, 20)?;
    }
    modules::process::close_minimax_code_instances(&[instance.user_data_dir.clone()], 20)?;
    let updated = modules::minimax_code_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn minimax_code_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::minimax_code_instance::load_default_settings()?;
        let pid = resolve_running_pid(default_settings.last_pid, None).ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 MiniMax Code 默认实例窗口失败：{}", err))?;
        return Ok(());
    }

    let store = modules::minimax_code_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir))
        .ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 MiniMax Code 实例窗口失败：instance_id={}, err={}",
            instance.id, err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn minimax_code_close_all_instances() -> Result<(), String> {
    let store = modules::minimax_code_instance::load_instance_store()?;
    let default_dir = modules::minimax_code_instance::get_default_minimax_code_user_data_dir()?;
    let mut target_dirs: Vec<String> = Vec::new();
    target_dirs.push(default_dir.to_string_lossy().to_string());
    for instance in &store.instances {
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }

    modules::process::close_minimax_code_instances(&target_dirs, 20)?;
    let _ = modules::minimax_code_instance::clear_all_pids();
    Ok(())
}
