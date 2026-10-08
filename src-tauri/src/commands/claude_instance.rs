use std::path::Path;

use serde::Serialize;

use crate::models::claude::ClaudeAuthMode;
use crate::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile};
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeInstanceProfileView {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    pub launch_mode: InstanceLaunchMode,
    pub created_at: i64,
    pub last_launched_at: Option<i64>,
    pub last_pid: Option<u32>,
    pub running: bool,
    pub initialized: bool,
    pub is_default: bool,
    pub follow_local_account: bool,
}

impl ClaudeInstanceProfileView {
    fn from_profile(profile: InstanceProfile, running: bool, initialized: bool) -> Self {
        let last_pid = if matches!(profile.launch_mode, InstanceLaunchMode::Cli) {
            None
        } else {
            profile.last_pid
        };
        Self {
            id: profile.id,
            name: profile.name,
            user_data_dir: profile.user_data_dir,
            working_dir: profile.working_dir,
            extra_args: profile.extra_args,
            bind_account_id: profile.bind_account_id,
            launch_mode: profile.launch_mode,
            created_at: profile.created_at,
            last_launched_at: profile.last_launched_at,
            last_pid,
            running,
            initialized,
            is_default: false,
            follow_local_account: false,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
}

struct ClaudeCliLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
    use_config_env: bool,
}

fn is_profile_initialized(user_data_dir: &str) -> bool {
    modules::claude_instance::is_profile_initialized(Path::new(user_data_dir))
}

fn inject_bound_account_for_instance_start(
    user_data_dir: &str,
    bind_account_id: Option<&str>,
    backup_existing: bool,
) -> Result<(), String> {
    let bind_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bind_id) = bind_id else {
        return Ok(());
    };

    let account = modules::claude_account::load_account(bind_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;

    match account.auth_mode {
        ClaudeAuthMode::DesktopOAuth => {
            modules::claude_account::restore_desktop_account_to_profile(
                bind_id,
                Path::new(user_data_dir),
                backup_existing,
            )
        }
        ClaudeAuthMode::DesktopGateway => {
            modules::claude_account::restore_desktop_gateway_account_to_profile(
                bind_id,
                Path::new(user_data_dir),
                backup_existing,
            )
        }
        ClaudeAuthMode::ApiKey => Err(
            "Claude API Key 账号不能写入 Claude 登录态，请选择 Claude 登录账号或取消绑定。"
                .to_string(),
        ),
        _ => {
            Err("旧 OAuth 账号已不再支持用于 Claude 实例，请重新添加 Claude 登录账号。".to_string())
        }
    }
}

fn inject_bound_account_for_cli_instance_start(
    config_dir: Option<&Path>,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let bind_id = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(bind_id) = bind_id else {
        return Ok(());
    };

    let account = modules::claude_account::load_account(bind_id)
        .ok_or_else(|| format!("绑定账号不存在: {}", bind_id))?;
    if matches!(
        account.auth_mode,
        ClaudeAuthMode::DesktopOAuth | ClaudeAuthMode::DesktopGateway
    ) {
        return Err(
            "Claude 登录账号不能写入 Claude CLI 实例，请选择 Claude CLI OAuth / API Key 账号。"
                .to_string(),
        );
    }

    let effective_config_dir = match config_dir {
        Some(path) => path.to_path_buf(),
        None => modules::claude_account::get_default_claude_code_config_dir()?,
    };
    let _ = modules::claude_account::sync_cli_account_from_config_dir_if_same(
        bind_id,
        &effective_config_dir,
    )?;
    modules::claude_account::inject_to_claude_config(bind_id, config_dir)?;
    crate::modules::provider_current_state::set_current_account_id(
        "claude_code_account",
        Some(bind_id),
    )?;
    Ok(())
}

fn is_cli_launch_mode(mode: &InstanceLaunchMode) -> bool {
    matches!(mode, InstanceLaunchMode::Cli)
}

fn default_user_data_dir_for_launch_mode(
    mode: &InstanceLaunchMode,
) -> Result<std::path::PathBuf, String> {
    if is_cli_launch_mode(mode) {
        modules::claude_instance::get_default_claude_cli_config_dir()
    } else {
        modules::claude_instance::get_default_claude_config_dir()
    }
}

fn default_instance_view(
    user_data_dir: &Path,
    settings: &DefaultInstanceSettings,
    running: bool,
    last_pid: Option<u32>,
) -> ClaudeInstanceProfileView {
    ClaudeInstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: user_data_dir.to_string_lossy().to_string(),
        working_dir: settings.working_dir.clone(),
        extra_args: settings.extra_args.clone(),
        bind_account_id: settings.bind_account_id.clone(),
        launch_mode: settings.launch_mode.clone(),
        created_at: 0,
        last_launched_at: None,
        last_pid: if is_cli_launch_mode(&settings.launch_mode) {
            None
        } else {
            last_pid
        },
        running,
        initialized: modules::claude_instance::is_profile_initialized(user_data_dir),
        is_default: true,
        follow_local_account: false,
    }
}

fn resolve_cli_launch_context(instance_id: &str) -> Result<ClaudeCliLaunchContext, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::claude_instance::load_default_settings()?;
        if !is_cli_launch_mode(&default_settings.launch_mode) {
            return Err("当前实例未启用 CLI 启动方式".to_string());
        }
        let default_dir = modules::claude_instance::get_default_claude_cli_config_dir()?;
        return Ok(ClaudeCliLaunchContext {
            user_data_dir: default_dir.to_string_lossy().to_string(),
            working_dir: default_settings.working_dir,
            extra_args: default_settings.extra_args,
            use_config_env: false,
        });
    }

    let store = modules::claude_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    if !is_cli_launch_mode(&instance.launch_mode) {
        return Err("当前实例未启用 CLI 启动方式".to_string());
    }
    Ok(ClaudeCliLaunchContext {
        user_data_dir: instance.user_data_dir,
        working_dir: instance.working_dir,
        extra_args: instance.extra_args,
        use_config_env: true,
    })
}

fn build_cli_launch_command(context: &ClaudeCliLaunchContext) -> String {
    let env = std::collections::BTreeMap::new();
    crate::commands::claude::build_claude_cli_command_for_context(
        context.working_dir.as_deref(),
        context
            .use_config_env
            .then_some(context.user_data_dir.as_str()),
        &context.extra_args,
        &env,
    )
}

fn prepare_cli_instance_config(
    instance_id: &str,
    context: &ClaudeCliLaunchContext,
) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::claude_instance::load_default_settings()?;
        return inject_bound_account_for_cli_instance_start(
            None,
            default_settings.bind_account_id.as_deref(),
        );
    }

    let store = modules::claude_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("Claude instance not found")?;
    inject_bound_account_for_cli_instance_start(
        Some(Path::new(&context.user_data_dir)),
        instance.bind_account_id.as_deref(),
    )
}

#[tauri::command]
pub async fn claude_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::claude_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn claude_list_instances() -> Result<Vec<ClaudeInstanceProfileView>, String> {
    let store = modules::claude_instance::load_instance_store()?;
    let default_settings = store.default_settings.clone();
    let default_dir = default_user_data_dir_for_launch_mode(&default_settings.launch_mode)?;
    let process_entries = modules::claude_instance::collect_claude_process_entries();

    let mut result: Vec<ClaudeInstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let is_cli = is_cli_launch_mode(&instance.launch_mode);
            let resolved_pid = if is_cli {
                None
            } else {
                modules::claude_instance::resolve_claude_pid_from_entries(
                    instance.last_pid,
                    Some(&instance.user_data_dir),
                    &process_entries,
                )
            };
            let running = !is_cli && resolved_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = ClaudeInstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = resolved_pid;
            view
        })
        .collect();

    let default_pid = if is_cli_launch_mode(&default_settings.launch_mode) {
        None
    } else {
        modules::claude_instance::resolve_claude_pid_from_entries(
            default_settings.last_pid,
            None,
            &process_entries,
        )
    };
    result.push(default_instance_view(
        &default_dir,
        &default_settings,
        default_pid.is_some(),
        default_pid,
    ));

    Ok(result)
}

#[tauri::command]
pub async fn claude_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
    launch_mode: Option<InstanceLaunchMode>,
) -> Result<ClaudeInstanceProfileView, String> {
    let instance = modules::claude_instance::create_instance(
        modules::claude_instance::CreateInstanceParams {
            name,
            user_data_dir,
            working_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
            launch_mode,
        },
    )?;
    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(ClaudeInstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn claude_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
) -> Result<ClaudeInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let updated = modules::claude_instance::update_default_settings(
            bind_account_id,
            working_dir,
            extra_args,
            follow_local_account,
            launch_mode,
        )?;
        let default_dir = default_user_data_dir_for_launch_mode(&updated.launch_mode)?;
        let resolved_pid = if is_cli_launch_mode(&updated.launch_mode) {
            None
        } else {
            modules::claude_instance::resolve_claude_pid(updated.last_pid, None)
        };
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            resolved_pid.is_some(),
            resolved_pid,
        ));
    }

    let instance = modules::claude_instance::update_instance(
        modules::claude_instance::UpdateInstanceParams {
            instance_id,
            name,
            working_dir,
            extra_args,
            bind_account_id,
            launch_mode,
        },
    )?;
    let resolved_pid = if is_cli_launch_mode(&instance.launch_mode) {
        None
    } else {
        modules::claude_instance::resolve_claude_pid(
            instance.last_pid,
            Some(&instance.user_data_dir),
        )
    };
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let mut view =
        ClaudeInstanceProfileView::from_profile(instance, resolved_pid.is_some(), initialized);
    view.last_pid = resolved_pid;
    Ok(view)
}

#[tauri::command]
pub async fn claude_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::claude_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn claude_start_instance(
    instance_id: String,
) -> Result<ClaudeInstanceProfileView, String> {
    modules::logger::log_info(&format!("开始启动 Claude 实例: {}", instance_id));

    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::claude_instance::load_default_settings()?;
        let default_dir = default_user_data_dir_for_launch_mode(&default_settings.launch_mode)?;
        let default_dir_str = default_dir.to_string_lossy().to_string();

        if is_cli_launch_mode(&default_settings.launch_mode) {
            inject_bound_account_for_cli_instance_start(
                None,
                default_settings.bind_account_id.as_deref(),
            )?;
            let updated = modules::claude_instance::update_default_pid(None)?;
            return Ok(default_instance_view(&default_dir, &updated, false, None));
        }

        modules::claude_instance::ensure_claude_launch_path_configured()?;

        if let Some(pid) =
            modules::claude_instance::resolve_claude_pid(default_settings.last_pid, None)
        {
            modules::process::close_pid(pid, 20)?;
            let _ = modules::claude_instance::update_default_pid(None)?;
        }

        modules::claude_instance::close_claude(&[default_dir_str.clone()], 20)?;
        inject_bound_account_for_instance_start(
            &default_dir_str,
            default_settings.bind_account_id.as_deref(),
            true,
        )?;

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid = modules::claude_instance::start_claude_default_with_args_with_new_window(
            &extra_args,
            false,
        )?;
        let _ = modules::claude_instance::update_default_pid(Some(pid))?;
        let running = modules::claude_instance::resolve_claude_pid(Some(pid), None).is_some();
        return Ok(default_instance_view(
            &default_dir,
            &default_settings,
            running,
            Some(pid),
        ));
    }

    let store = modules::claude_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if is_cli_launch_mode(&instance.launch_mode) {
        inject_bound_account_for_cli_instance_start(
            Some(Path::new(&instance.user_data_dir)),
            instance.bind_account_id.as_deref(),
        )?;
        let updated = modules::claude_instance::update_instance_last_launched(&instance.id)?;
        let initialized = is_profile_initialized(&updated.user_data_dir);
        return Ok(ClaudeInstanceProfileView::from_profile(
            updated,
            false,
            initialized,
        ));
    }

    modules::claude_instance::ensure_claude_multi_instance_launch_path_configured()?;

    if let Some(pid) = modules::claude_instance::resolve_claude_pid(
        instance.last_pid,
        Some(&instance.user_data_dir),
    ) {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::claude_instance::update_instance_pid(&instance.id, None)?;
    }

    modules::claude_instance::close_claude(&[instance.user_data_dir.clone()], 20)?;
    inject_bound_account_for_instance_start(
        &instance.user_data_dir,
        instance.bind_account_id.as_deref(),
        false,
    )?;

    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::claude_instance::start_claude_with_args_with_new_window(
        &instance.user_data_dir,
        &extra_args,
        true,
    )?;
    let updated = modules::claude_instance::update_instance_after_start(&instance.id, pid)?;
    let running =
        modules::claude_instance::resolve_claude_pid(Some(pid), Some(&updated.user_data_dir))
            .is_some();
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(ClaudeInstanceProfileView::from_profile(
        updated,
        running,
        initialized,
    ))
}

#[tauri::command]
pub async fn claude_stop_instance(
    instance_id: String,
) -> Result<ClaudeInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::claude_instance::load_default_settings()?;
        let default_dir = default_user_data_dir_for_launch_mode(&default_settings.launch_mode)?;

        if is_cli_launch_mode(&default_settings.launch_mode) {
            let updated_settings = modules::claude_instance::update_default_pid(None)?;
            return Ok(default_instance_view(
                &default_dir,
                &updated_settings,
                false,
                None,
            ));
        }

        if let Some(pid) =
            modules::claude_instance::resolve_claude_pid(default_settings.last_pid, None)
        {
            modules::process::close_pid(pid, 20)?;
        }

        let updated_settings = modules::claude_instance::update_default_pid(None)?;
        let running = updated_settings
            .last_pid
            .and_then(|pid| modules::claude_instance::resolve_claude_pid(Some(pid), None))
            .is_some();
        return Ok(default_instance_view(
            &default_dir,
            &default_settings,
            running,
            None,
        ));
    }

    let store = modules::claude_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if is_cli_launch_mode(&instance.launch_mode) {
        let updated = modules::claude_instance::update_instance_pid(&instance.id, None)?;
        let initialized = is_profile_initialized(&updated.user_data_dir);
        return Ok(ClaudeInstanceProfileView::from_profile(
            updated,
            false,
            initialized,
        ));
    }

    if let Some(pid) = modules::claude_instance::resolve_claude_pid(
        instance.last_pid,
        Some(&instance.user_data_dir),
    ) {
        modules::process::close_pid(pid, 20)?;
    }

    let updated = modules::claude_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(ClaudeInstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn claude_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings: DefaultInstanceSettings =
            modules::claude_instance::load_default_settings()?;
        if is_cli_launch_mode(&default_settings.launch_mode) {
            return Err("Claude CLI 实例不支持窗口定位，请使用启动命令在终端中运行".to_string());
        }
        modules::claude_instance::focus_claude_instance(default_settings.last_pid, None)
            .map_err(|err| format!("定位 Claude 默认实例窗口失败: {}", err))?;
        return Ok(());
    }

    let store = modules::claude_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    if is_cli_launch_mode(&instance.launch_mode) {
        return Err("Claude CLI 实例不支持窗口定位，请使用启动命令在终端中运行".to_string());
    }

    modules::claude_instance::focus_claude_instance(
        instance.last_pid,
        Some(&instance.user_data_dir),
    )
    .map_err(|err| {
        format!(
            "定位 Claude 实例窗口失败: instance_id={}, err={}",
            instance.id, err
        )
    })?;

    Ok(())
}

#[tauri::command]
pub async fn claude_close_all_instances() -> Result<(), String> {
    let store = modules::claude_instance::load_instance_store()?;

    let mut target_dirs = Vec::new();
    if !is_cli_launch_mode(&store.default_settings.launch_mode) {
        let default_dir = modules::claude_instance::get_default_claude_config_dir()?;
        target_dirs.push(default_dir.to_string_lossy().to_string());
    }
    for instance in &store.instances {
        if is_cli_launch_mode(&instance.launch_mode) {
            continue;
        }
        let dir = instance.user_data_dir.trim();
        if !dir.is_empty() {
            target_dirs.push(dir.to_string());
        }
    }

    if !target_dirs.is_empty() {
        modules::claude_instance::close_claude(&target_dirs, 20)?;
    }
    let _ = modules::claude_instance::clear_all_pids();
    Ok(())
}

#[tauri::command]
pub async fn claude_get_instance_launch_command(
    instance_id: String,
) -> Result<ClaudeInstanceLaunchInfo, String> {
    let context = resolve_cli_launch_context(&instance_id)?;
    prepare_cli_instance_config(&instance_id, &context)?;
    Ok(ClaudeInstanceLaunchInfo {
        instance_id,
        launch_command: build_cli_launch_command(&context),
        user_data_dir: context.user_data_dir,
    })
}

#[tauri::command]
pub async fn claude_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let context = resolve_cli_launch_context(&instance_id)?;
    prepare_cli_instance_config(&instance_id, &context)?;
    let command = build_cli_launch_command(&context);
    crate::commands::claude::execute_claude_cli_command(&command, terminal)
}
