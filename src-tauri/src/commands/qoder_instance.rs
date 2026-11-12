use std::path::Path;

use crate::models::InstanceProfileView;
use crate::modules;
use modules::qoder_instance::QODER_LIFECYCLE;

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn is_profile_initialized(user_data_dir: &str) -> bool {
    Path::new(user_data_dir)
        .join("User/globalStorage/state.vscdb")
        .is_file()
}

fn resolve_running_pid(last_pid: Option<u32>, user_data_dir: Option<&str>) -> Option<u32> {
    modules::process::resolve_qoder_pid(last_pid, user_data_dir)
}

#[tauri::command]
pub async fn qoder_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::qoder_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn qoder_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    tauri::async_runtime::spawn_blocking(list_qoder_instances)
        .await
        .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn list_qoder_instances() -> Result<Vec<InstanceProfileView>, String> {
    let store = modules::qoder_instance::load_instance_store()?;
    let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir()?;
    let default_dir_str = default_dir.to_string_lossy().to_string();

    let default_settings = store.default_settings.clone();
    let entries = modules::process::collect_qoder_process_entries();

    let mut result: Vec<InstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let running_pid = modules::process::resolve_qoder_pid_from_entries(
                instance.last_pid,
                Some(&instance.user_data_dir),
                &entries,
            );
            let running = running_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = InstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = running_pid;
            view
        })
        .collect();

    let default_pid =
        modules::process::resolve_qoder_pid_from_entries(default_settings.last_pid, None, &entries);
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
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        create_qoder_instance(
            name,
            user_data_dir,
            extra_args,
            bind_account_id,
            copy_source_instance_id,
            init_mode,
        )
    })
    .await
    .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn create_qoder_instance(
    name: String,
    user_data_dir: String,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let instance =
        modules::qoder_instance::create_instance(modules::qoder_instance::CreateInstanceParams {
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
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        update_qoder_instance(
            instance_id,
            name,
            extra_args,
            bind_account_id,
            follow_local_account,
        )
    })
    .await
    .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn update_qoder_instance(
    instance_id: String,
    name: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    let _operation = QODER_LIFECYCLE
        .try_lock()
        .map_err(|_| "QODER_INSTANCE_BUSY")?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let updated = modules::qoder_instance::update_default_settings(
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

    let instance =
        modules::qoder_instance::update_instance(modules::qoder_instance::UpdateInstanceParams {
            working_dir: None,
            instance_id,
            name,
            extra_args,
            bind_account_id,
        })?;

    let running_pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir));
    let running = running_pid.is_some();
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view = InstanceProfileView::from_profile(instance, running, initialized);
    view.last_pid = running_pid;
    Ok(view)
}

#[tauri::command]
pub async fn qoder_delete_instance(instance_id: String) -> Result<(), String> {
    let _operation = QODER_LIFECYCLE
        .try_lock()
        .map_err(|_| "QODER_INSTANCE_BUSY")?;
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::qoder_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn qoder_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    start_qoder_instance(instance_id, None).await
}

pub(super) async fn start_qoder_instance(
    instance_id: String,
    switch_account_id: Option<String>,
) -> Result<InstanceProfileView, String> {
    let operation = QODER_LIFECYCLE
        .try_lock()
        .map_err(|_| "QODER_INSTANCE_BUSY")?;
    let target = tauri::async_runtime::spawn_blocking(move || {
        QoderStartTarget::load(&instance_id, switch_account_id)
    })
    .await
    .map_err(|error| format!("QODER_START_WORKER_FAILED: {}", error))??;

    if let Some(id) = target.bind_account_id.as_deref() {
        modules::qoder_oauth::prepare_account_for_switch(id).await?;
    }

    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        target.start()
    })
    .await
    .map_err(|error| format!("QODER_START_WORKER_FAILED: {}", error))?
}

struct QoderStartTarget {
    profile: Option<crate::models::InstanceProfile>,
    user_data_dir: String,
    extra_args: String,
    bind_account_id: Option<String>,
    last_pid: Option<u32>,
    change_default_binding: bool,
}

impl QoderStartTarget {
    fn load(instance_id: &str, switch_account_id: Option<String>) -> Result<Self, String> {
        let executable = modules::process::resolve_qoder_launch_path()?;
        let store = modules::qoder_instance::load_instance_store()?;
        if instance_id == DEFAULT_INSTANCE_ID {
            let settings = store.default_settings;
            let args = modules::process::parse_extra_args(&settings.extra_args);
            let root = modules::qoder_paths::user_data_dir(&args, Some(&executable))?;
            return Ok(Self {
                profile: None,
                user_data_dir: root.to_string_lossy().into_owned(),
                extra_args: settings.extra_args,
                change_default_binding: switch_account_id.is_some(),
                bind_account_id: switch_account_id.or(settings.bind_account_id),
                last_pid: settings.last_pid,
            });
        }

        let profile = store
            .instances
            .into_iter()
            .find(|item| item.id == instance_id)
            .ok_or("实例不存在")?;
        let mut args = vec!["--user-data-dir".to_string(), profile.user_data_dir.clone()];
        args.extend(modules::process::parse_extra_args(&profile.extra_args));
        let root = modules::qoder_paths::user_data_dir(&args, Some(&executable))?;
        let default_args = modules::process::parse_extra_args(&store.default_settings.extra_args);
        let default_root = modules::qoder_paths::user_data_dir(&default_args, Some(&executable))?;
        if !modules::process::same_qoder_profile_path(&root, Path::new(&profile.user_data_dir))
            || modules::process::same_qoder_profile_path(&root, &default_root)
        {
            return Err(format!("QODER_PROFILE_CONFLICT: {}", root.display()));
        }
        Ok(Self {
            user_data_dir: root.to_string_lossy().into_owned(),
            extra_args: profile.extra_args.clone(),
            bind_account_id: profile.bind_account_id.clone(),
            last_pid: profile.last_pid,
            change_default_binding: false,
            profile: Some(profile),
        })
    }

    fn launch(&self, args: &[String]) -> Result<u32, String> {
        if self.profile.is_some() || modules::qoder_paths::user_data_override(args).is_some() {
            let args = modules::qoder_paths::without_user_data_override(args.to_vec());
            modules::process::start_qoder_with_args_with_new_window(
                &self.user_data_dir,
                &args,
                true,
            )
        } else {
            modules::process::start_qoder_default_with_args_with_new_window(args, true)
        }
    }

    fn start(self) -> Result<InstanceProfileView, String> {
        let args = modules::process::parse_extra_args(&self.extra_args);
        let root = Path::new(&self.user_data_dir);
        let match_dir = self.profile.as_ref().map(|_| self.user_data_dir.as_str());

        if self.bind_account_id.is_some() && !modules::vscode_inject::qoder_storage_ready(root)? {
            if resolve_running_pid(self.last_pid, match_dir).is_none() {
                self.launch(&args)?;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
            while !is_profile_initialized(&self.user_data_dir)
                || !modules::vscode_inject::qoder_storage_ready(root)?
            {
                if std::time::Instant::now() >= deadline {
                    return Err(format!("QODER_PROFILE_UNINITIALIZED: {}", root.display()));
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }

        modules::process::close_qoder_instances(std::slice::from_ref(&self.user_data_dir), 20)?;
        if let Some(account_id) = self.bind_account_id.as_deref() {
            modules::qoder_account::inject_to_qoder_for_user_data_dir(
                &self.user_data_dir,
                account_id,
            )?;
            if self.profile.is_none() {
                if self.change_default_binding {
                    modules::qoder_instance::update_default_settings(
                        Some(Some(account_id.to_string())),
                        None,
                        Some(false),
                    )?;
                }
                modules::provider_current_state::set_current_account_id("qoder", Some(account_id))?;
            }
        }

        let running_pid = self
            .launch(&args)
            .and_then(|pid| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
                loop {
                    if let Some(running) = resolve_running_pid(Some(pid), match_dir) {
                        return Ok(running);
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("QODER_LAUNCH_UNCONFIRMED".into());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            })
            .map_err(|error| {
                if self.change_default_binding {
                    format!("QODER_SWITCH_LAUNCH_FAILED: {error}")
                } else {
                    error
                }
            })?;
        let initialized = is_profile_initialized(&self.user_data_dir);
        if let Some(profile) = self.profile {
            let updated =
                modules::qoder_instance::update_instance_after_start(&profile.id, running_pid)?;
            return Ok(InstanceProfileView::from_profile(
                updated,
                true,
                initialized,
            ));
        }

        modules::qoder_instance::update_default_pid(Some(running_pid))?;
        Ok(InstanceProfileView {
            id: DEFAULT_INSTANCE_ID.to_string(),
            name: String::new(),
            user_data_dir: self.user_data_dir,
            working_dir: None,
            extra_args: self.extra_args,
            bind_account_id: self.bind_account_id,
            created_at: 0,
            last_launched_at: None,
            last_pid: Some(running_pid),
            running: true,
            initialized,
            is_default: true,
            follow_local_account: false,
        })
    }
}

#[tauri::command]
pub async fn qoder_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    tauri::async_runtime::spawn_blocking(move || qoder_stop_instance_blocking(instance_id))
        .await
        .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn qoder_stop_instance_blocking(instance_id: String) -> Result<InstanceProfileView, String> {
    let _operation = QODER_LIFECYCLE
        .try_lock()
        .map_err(|_| "QODER_INSTANCE_BUSY")?;
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir()?;
        let default_dir_str = default_dir.to_string_lossy().to_string();
        let default_settings = modules::qoder_instance::load_default_settings()?;
        modules::process::close_qoder_instances(&[default_dir_str.clone()], 20)?;
        let _ = modules::qoder_instance::update_default_pid(None)?;
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

    let store = modules::qoder_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    modules::process::close_qoder_instances(&[instance.user_data_dir.clone()], 20)?;
    let updated = modules::qoder_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(InstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn qoder_open_instance_window(instance_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || qoder_open_instance_window_blocking(instance_id))
        .await
        .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn qoder_open_instance_window_blocking(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::qoder_instance::load_default_settings()?;
        let pid = resolve_running_pid(default_settings.last_pid, None).ok_or("默认实例未运行")?;
        modules::process::focus_process_pid(pid)
            .map_err(|err| format!("定位 Qoder 默认实例窗口失败: {}", err))?;
        return Ok(());
    }

    let store = modules::qoder_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let pid = resolve_running_pid(instance.last_pid, Some(&instance.user_data_dir))
        .ok_or("实例未运行")?;

    modules::process::focus_process_pid(pid).map_err(|err| {
        format!(
            "定位 Qoder 实例窗口失败: instance_id={}, err={}",
            instance.id, err
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn qoder_close_all_instances() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || qoder_close_all_instances_blocking())
        .await
        .map_err(|error| format!("QODER_START_WORKER_FAILED: {error}"))?
}

fn qoder_close_all_instances_blocking() -> Result<(), String> {
    let _operation = QODER_LIFECYCLE
        .try_lock()
        .map_err(|_| "QODER_INSTANCE_BUSY")?;
    let store = modules::qoder_instance::load_instance_store()?;
    let default_dir = modules::qoder_instance::get_default_qoder_user_data_dir()?;
    let mut target_dirs: Vec<String> = Vec::new();
    target_dirs.push(default_dir.to_string_lossy().to_string());
    for instance in &store.instances {
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }

    modules::process::close_qoder_instances(&target_dirs, 20)?;
    let _ = modules::qoder_instance::clear_all_pids();
    Ok(())
}
