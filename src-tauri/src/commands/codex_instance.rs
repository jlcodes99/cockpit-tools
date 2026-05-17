use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::models::codex::CodexAppSpeed;
use crate::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile};
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceProfileView {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    pub launch_mode: InstanceLaunchMode,
    pub app_speed: CodexAppSpeed,
    pub created_at: i64,
    pub last_launched_at: Option<i64>,
    pub last_pid: Option<u32>,
    pub running: bool,
    pub initialized: bool,
    pub is_default: bool,
    pub follow_local_account: bool,
}

impl CodexInstanceProfileView {
    fn from_profile(profile: InstanceProfile, running: bool, initialized: bool) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            user_data_dir: profile.user_data_dir,
            working_dir: profile.working_dir,
            extra_args: profile.extra_args,
            bind_account_id: profile.bind_account_id,
            launch_mode: profile.launch_mode,
            app_speed: profile.app_speed,
            created_at: profile.created_at,
            last_launched_at: profile.last_launched_at,
            last_pid: profile.last_pid,
            running,
            initialized,
            is_default: false,
            follow_local_account: false,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
}

struct CodexLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
}

fn is_profile_initialized(user_data_dir: &str) -> bool {
    modules::instance::is_profile_initialized(Path::new(user_data_dir))
}

fn resolve_default_account_id(settings: &DefaultInstanceSettings) -> Option<String> {
    if settings.follow_local_account {
        resolve_local_account_id()
    } else {
        settings.bind_account_id.clone()
    }
}

fn resolve_local_account_id() -> Option<String> {
    let account = modules::codex_account::get_current_account()?;
    Some(account.id)
}

async fn inject_bound_account_to_profile(
    profile_dir: &Path,
    bind_account_id: &str,
) -> Result<(), String> {
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::activate_local_access_for_dir(profile_dir).await?;
        return Ok(());
    }

    modules::codex_instance::inject_account_to_profile(profile_dir, bind_account_id).await
}

fn default_instance_view(
    default_dir: &Path,
    default_settings: &DefaultInstanceSettings,
    bind_account_id: Option<String>,
    running: bool,
    last_pid: Option<u32>,
) -> CodexInstanceProfileView {
    CodexInstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: default_dir.to_string_lossy().to_string(),
        working_dir: None,
        extra_args: default_settings.extra_args.clone(),
        bind_account_id,
        launch_mode: default_settings.launch_mode.clone(),
        app_speed: default_settings.app_speed.clone(),
        created_at: 0,
        last_launched_at: None,
        last_pid,
        running,
        initialized: modules::instance::is_profile_initialized(default_dir),
        is_default: true,
        follow_local_account: default_settings.follow_local_account,
    }
}

fn resolve_instance_base_dir(instance_id: &str) -> Result<PathBuf, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return modules::codex_instance::get_default_codex_home();
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    Ok(PathBuf::from(instance.user_data_dir))
}

fn resolve_instance_launch_context(instance_id: &str) -> Result<CodexLaunchContext, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::codex_instance::load_default_settings()?;
        if default_settings.launch_mode != InstanceLaunchMode::Cli {
            return Err("当前实例未启用 CLI 启动方式".to_string());
        }
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        return Ok(CodexLaunchContext {
            user_data_dir: default_dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: default_settings.extra_args,
        });
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    if instance.launch_mode != InstanceLaunchMode::Cli {
        return Err("当前实例未启用 CLI 启动方式".to_string());
    }
    Ok(CodexLaunchContext {
        user_data_dir: instance.user_data_dir,
        working_dir: instance.working_dir,
        extra_args: instance.extra_args,
    })
}

#[cfg(not(target_os = "windows"))]
fn posix_shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let needs_quote = value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | '$' | '`' | '\\' | '&' | '|' | ';' | '<' | '>' | '(' | ')'
            )
    });
    if !needs_quote {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn build_launch_command(context: &CodexLaunchContext) -> Result<String, String> {
    let runtime = modules::codex_wakeup::resolve_cli_runtime()?;
    let parsed_args = modules::process::parse_extra_args(&context.extra_args);

    #[cfg(not(target_os = "windows"))]
    {
        let mut command_parts = Vec::new();
        if let Some(ref dir) = context.working_dir {
            if !dir.trim().is_empty() {
                command_parts.push(format!("cd {}", posix_shell_quote(dir)));
            }
        }

        let mut codex_cmd = String::new();
        codex_cmd.push_str("CODEX_HOME=");
        codex_cmd.push_str(&posix_shell_quote(&context.user_data_dir));
        codex_cmd.push(' ');
        if let Some(node_path) = runtime.node_path.as_deref() {
            codex_cmd.push_str(&posix_shell_quote(node_path));
            codex_cmd.push(' ');
        }
        codex_cmd.push_str(&posix_shell_quote(&runtime.binary_path));

        for arg in parsed_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                codex_cmd.push(' ');
                codex_cmd.push_str(&posix_shell_quote(trimmed));
            }
        }

        command_parts.push(codex_cmd);
        return Ok(command_parts.join(" && "));
    }

    #[cfg(target_os = "windows")]
    {
        let mut command_parts = Vec::new();
        command_parts.push(format!(
            "$env:CODEX_HOME={}",
            powershell_quote(&context.user_data_dir)
        ));

        if let Some(ref dir) = context.working_dir {
            if !dir.trim().is_empty() {
                command_parts.push(format!(
                    "Set-Location -LiteralPath {}",
                    powershell_quote(dir)
                ));
            }
        }

        let mut codex_cmd = String::new();
        if let Some(node_path) = runtime.node_path.as_deref() {
            codex_cmd.push_str("& ");
            codex_cmd.push_str(&powershell_quote(node_path));
            codex_cmd.push(' ');
            codex_cmd.push_str(&powershell_quote(&runtime.binary_path));
        } else {
            codex_cmd.push_str("& ");
            codex_cmd.push_str(&powershell_quote(&runtime.binary_path));
        }

        for arg in parsed_args {
            let trimmed = arg.trim();
            if !trimmed.is_empty() {
                codex_cmd.push(' ');
                codex_cmd.push_str(&powershell_quote(trimmed));
            }
        }

        command_parts.push(codex_cmd);
        return Ok(command_parts.join("; "));
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持生成 Codex CLI 启动命令".to_string())
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[tauri::command]
pub async fn codex_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::codex_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn codex_list_instances() -> Result<Vec<CodexInstanceProfileView>, String> {
    let store = modules::codex_instance::load_instance_store()?;
    let default_dir = modules::codex_instance::get_default_codex_home()?;

    let default_settings = store.default_settings.clone();
    let process_entries = modules::process::collect_codex_process_entries();
    let mut result: Vec<CodexInstanceProfileView> = store
        .instances
        .into_iter()
        .map(|instance| {
            let resolved_pid = modules::process::resolve_codex_pid_from_entries(
                instance.last_pid,
                Some(&instance.user_data_dir),
                &process_entries,
            );
            let running = resolved_pid.is_some();
            let initialized = is_profile_initialized(&instance.user_data_dir);
            let mut view = CodexInstanceProfileView::from_profile(instance, running, initialized);
            view.last_pid = resolved_pid;
            view
        })
        .collect();

    let default_pid = modules::process::resolve_codex_pid_from_entries(
        default_settings.last_pid,
        None,
        &process_entries,
    );
    let default_running = default_pid.is_some();
    let default_bind_account_id = resolve_default_account_id(&default_settings);
    result.push(default_instance_view(
        &default_dir,
        &default_settings,
        default_bind_account_id,
        default_running,
        default_pid,
    ));

    Ok(result)
}

#[tauri::command]
pub async fn codex_get_instance_quick_config(
    instance_id: String,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    let base_dir = resolve_instance_base_dir(instance_id.as_str())?;
    modules::codex_account::read_quick_config_from_config_toml(&base_dir)
}

#[tauri::command]
pub async fn codex_save_instance_quick_config(
    instance_id: String,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    let base_dir = resolve_instance_base_dir(instance_id.as_str())?;
    modules::codex_account::save_quick_config_for_base_dir(
        &base_dir,
        model_context_window,
        auto_compact_token_limit,
    )
}

#[tauri::command]
pub async fn codex_open_instance_config_toml(
    app: AppHandle,
    instance_id: String,
) -> Result<(), String> {
    let base_dir = resolve_instance_base_dir(instance_id.as_str())?;
    let path = base_dir.join("config.toml");
    if !path.exists() {
        return Err(format!("未找到实例 config.toml 文件: {}", path.display()));
    }
    app.opener()
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开实例 config.toml 失败: {}", e))
}

#[tauri::command]
pub async fn codex_sync_threads_across_instances(
) -> Result<modules::codex_thread_sync::CodexInstanceThreadSyncSummary, String> {
    modules::codex_thread_sync::sync_threads_across_instances()
}

#[tauri::command]
pub async fn codex_sync_sessions_to_instance(
    session_ids: Vec<String>,
    target_instance_id: String,
) -> Result<modules::codex_thread_sync::CodexInstanceTargetThreadSyncSummary, String> {
    modules::codex_thread_sync::sync_sessions_to_instance(session_ids, target_instance_id)
}

#[tauri::command]
pub async fn codex_repair_session_visibility_across_instances(
) -> Result<modules::codex_session_visibility::CodexSessionVisibilityRepairSummary, String> {
    modules::codex_session_visibility::repair_session_visibility_across_instances()
}

#[tauri::command]
pub async fn codex_list_sessions_across_instances(
) -> Result<Vec<modules::codex_session_manager::CodexSessionRecord>, String> {
    modules::codex_session_manager::list_sessions_across_instances()
}

#[tauri::command]
pub async fn codex_get_session_token_stats_across_instances(
    session_ids: Vec<String>,
) -> Result<Vec<modules::codex_session_manager::CodexSessionTokenStats>, String> {
    modules::codex_session_manager::get_session_token_stats_across_instances(session_ids)
}

#[tauri::command]
pub async fn codex_move_sessions_to_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<modules::codex_session_manager::CodexSessionTrashSummary, String> {
    modules::codex_session_manager::move_sessions_to_trash_across_instances(session_ids)
}

#[tauri::command]
pub async fn codex_list_trashed_sessions_across_instances(
) -> Result<Vec<modules::codex_session_manager::CodexTrashedSessionRecord>, String> {
    modules::codex_session_manager::list_trashed_sessions_across_instances()
}

#[tauri::command]
pub async fn codex_restore_sessions_from_trash_across_instances(
    session_ids: Vec<String>,
) -> Result<modules::codex_session_manager::CodexSessionRestoreSummary, String> {
    modules::codex_session_manager::restore_sessions_from_trash_across_instances(session_ids)
}

#[tauri::command]
pub async fn codex_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
    launch_mode: Option<InstanceLaunchMode>,
    app_speed: Option<CodexAppSpeed>,
) -> Result<CodexInstanceProfileView, String> {
    let instance =
        modules::codex_instance::create_instance(modules::codex_instance::CreateInstanceParams {
            name,
            user_data_dir,
            working_dir,
            extra_args: extra_args.unwrap_or_default(),
            bind_account_id,
            copy_source_instance_id,
            init_mode,
            launch_mode,
            app_speed,
        })?;

    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(CodexInstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn codex_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
    app_speed: Option<CodexAppSpeed>,
) -> Result<CodexInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let mut updated = modules::codex_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
            launch_mode,
        )?;
        if let Some(speed) = app_speed {
            updated = modules::codex_instance::update_default_app_speed(speed)?;
        }
        let running = updated
            .last_pid
            .map(modules::process::is_pid_running)
            .unwrap_or(false);
        let default_bind_account_id = resolve_default_account_id(&updated);
        let _ = working_dir;
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            updated.last_pid,
        ));
    }

    let wants_bind = bind_account_id
        .as_ref()
        .and_then(|next| next.as_ref())
        .is_some();
    if wants_bind {
        let store = modules::codex_instance::load_instance_store()?;
        if let Some(target) = store.instances.iter().find(|item| item.id == instance_id) {
            if !is_profile_initialized(&target.user_data_dir) {
                return Err(
                    "INSTANCE_NOT_INITIALIZED:请先启动一次实例创建数据后，再进行账号绑定"
                        .to_string(),
                );
            }
        }
    }

    let instance =
        modules::codex_instance::update_instance(modules::codex_instance::UpdateInstanceParams {
            instance_id,
            name,
            working_dir,
            extra_args,
            bind_account_id,
            launch_mode,
            app_speed,
        })?;

    let running = instance
        .last_pid
        .map(modules::process::is_pid_running)
        .unwrap_or(false);
    let initialized = is_profile_initialized(&instance.user_data_dir);
    Ok(CodexInstanceProfileView::from_profile(
        instance,
        running,
        initialized,
    ))
}

#[tauri::command]
pub async fn codex_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::codex_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn codex_start_instance(instance_id: String) -> Result<CodexInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let default_settings = modules::codex_instance::load_default_settings()?;
        let default_bind_account_id = resolve_default_account_id(&default_settings);
        if default_settings.launch_mode != InstanceLaunchMode::Cli {
            modules::process::ensure_codex_launch_path_configured()?;
        }
        modules::process::close_codex_default(20)?;
        let _ = modules::codex_instance::update_default_pid(None)?;
        modules::codex_speed::write_app_speed_for_dir(
            &default_dir,
            default_settings.app_speed.clone(),
        )?;
        if let Some(ref account_id) = default_bind_account_id {
            inject_bound_account_to_profile(&default_dir, account_id).await?;
        }

        if default_settings.launch_mode == InstanceLaunchMode::Cli {
            let context = resolve_instance_launch_context(DEFAULT_INSTANCE_ID)?;
            let _ = build_launch_command(&context)?;
            let _ = modules::codex_instance::update_default_pid(None)?;
            return Ok(default_instance_view(
                &default_dir,
                &default_settings,
                default_bind_account_id,
                false,
                None,
            ));
        }

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let pid = modules::process::start_codex_default(&extra_args)?;
        let updated = modules::codex_instance::update_default_pid(Some(pid))?;
        let running = modules::process::is_pid_running(pid);
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            Some(pid),
        ));
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    modules::codex_instance::ensure_instance_shared_skills(Path::new(&instance.user_data_dir))?;

    if let Some(pid) =
        modules::process::resolve_codex_pid(instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    }

    if let Some(ref account_id) = instance.bind_account_id {
        inject_bound_account_to_profile(Path::new(&instance.user_data_dir), account_id).await?;
    }
    modules::codex_speed::write_app_speed_for_dir(
        Path::new(&instance.user_data_dir),
        instance.app_speed.clone(),
    )?;

    if instance.launch_mode == InstanceLaunchMode::Cli {
        let context = resolve_instance_launch_context(&instance.id)?;
        let _ = build_launch_command(&context)?;
        let updated = modules::codex_instance::update_instance_after_cli_prepare(&instance.id)?;
        let initialized = is_profile_initialized(&updated.user_data_dir);
        return Ok(CodexInstanceProfileView::from_profile(
            updated,
            false,
            initialized,
        ));
    }

    modules::process::ensure_codex_launch_path_configured()?;
    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let pid = modules::process::start_codex_with_args(&instance.user_data_dir, &extra_args)?;
    let updated = modules::codex_instance::update_instance_after_start(&instance.id, pid)?;
    let running = modules::process::is_pid_running(pid);
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(CodexInstanceProfileView::from_profile(
        updated,
        running,
        initialized,
    ))
}

#[tauri::command]
pub async fn codex_stop_instance(instance_id: String) -> Result<CodexInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        modules::process::close_codex_default(20)?;
        let updated = modules::codex_instance::update_default_pid(None)?;
        let default_bind_account_id = resolve_default_account_id(&updated);
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            false,
            None,
        ));
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    if let Some(pid) =
        modules::process::resolve_codex_pid(instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
    }
    let updated = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    Ok(CodexInstanceProfileView::from_profile(
        updated,
        false,
        initialized,
    ))
}

#[tauri::command]
pub async fn codex_close_all_instances() -> Result<(), String> {
    let store = modules::codex_instance::load_instance_store()?;
    let default_home = modules::codex_instance::get_default_codex_home()?;
    let mut target_homes: Vec<String> = Vec::new();
    target_homes.push(default_home.to_string_lossy().to_string());
    for instance in &store.instances {
        let home = instance.user_data_dir.trim();
        if !home.is_empty() {
            target_homes.push(home.to_string());
        }
    }

    modules::process::close_codex_instances(&target_homes, 20)?;
    let _ = modules::codex_instance::clear_all_pids();
    Ok(())
}

#[tauri::command]
pub async fn codex_open_instance_window(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_settings = modules::codex_instance::load_default_settings()?;
        if default_settings.launch_mode == InstanceLaunchMode::Cli {
            return Err("CLI 模式实例不支持窗口定位，请改用终端执行。".to_string());
        }
        modules::process::focus_codex_instance(default_settings.last_pid, None)
            .map_err(|err| format!("定位 Codex 默认实例窗口失败: {}", err))?;
        return Ok(());
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    if instance.launch_mode == InstanceLaunchMode::Cli {
        return Err("CLI 模式实例不支持窗口定位，请改用终端执行。".to_string());
    }

    modules::process::focus_codex_instance(instance.last_pid, Some(&instance.user_data_dir))
        .map_err(|err| {
            format!(
                "定位 Codex 实例窗口失败: instance_id={}, err={}",
                instance.id, err
            )
        })?;
    Ok(())
}

#[tauri::command]
pub async fn codex_get_instance_launch_command(
    instance_id: String,
) -> Result<CodexInstanceLaunchInfo, String> {
    let context = resolve_instance_launch_context(&instance_id)?;
    Ok(CodexInstanceLaunchInfo {
        instance_id,
        user_data_dir: context.user_data_dir.clone(),
        launch_command: build_launch_command(&context)?,
    })
}

#[tauri::command]
pub async fn codex_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let context = resolve_instance_launch_context(&instance_id)?;

    let command = build_launch_command(&context)?;

    #[cfg(target_os = "macos")]
    {
        let config = crate::modules::config::get_user_config();
        let terminal = terminal
            .unwrap_or(config.default_terminal)
            .trim()
            .to_string();
        let is_iterm = terminal.to_lowercase().contains("iterm");
        let is_terminal_app = terminal == "system" || terminal.is_empty() || terminal == "Terminal";
        let app_name = if is_terminal_app {
            "Terminal"
        } else {
            &terminal
        };

        let script = if is_iterm {
            format!(
                "tell application \"iTerm\"
                    activate
                    if not (exists window 1) then
                        create window with default profile
                        tell current session of current window
                            write text \"{}\"
                        end tell
                    else
                        tell current window
                            create tab with default profile
                            tell current session
                                write text \"{}\"
                            end tell
                        end tell
                    end if
                end tell",
                escape_applescript(&command),
                escape_applescript(&command)
            )
        } else if is_terminal_app {
            format!(
                "tell application \"Terminal\"
                    activate
                    do script \"{}\"
                end tell",
                escape_applescript(&command)
            )
        } else {
            return Err(format!(
                "当前终端暂不支持直接执行：{}。请改用 Terminal 或 iTerm2。",
                terminal
            ));
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("打开终端失败 ({}): {}", app_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("终端执行失败: {}", stderr.trim()));
        }
        return Ok(format!("已在 {} 执行 Codex CLI 命令", app_name));
    }

    #[cfg(target_os = "windows")]
    {
        let config = crate::modules::config::get_user_config();
        let terminal = terminal
            .unwrap_or(config.default_terminal)
            .trim()
            .to_string();

        let mut cmd = if terminal == "pwsh" {
            let mut command_process = Command::new("pwsh");
            command_process.args(["-NoExit", "-Command", &command]);
            command_process
        } else if terminal == "wt" {
            let mut command_process = Command::new("wt");
            command_process.args(["powershell", "-NoExit", "-Command", &command]);
            command_process
        } else if terminal == "cmd" {
            let mut command_process = Command::new("cmd");
            command_process.args([
                "/C",
                "start",
                "",
                "powershell",
                "-NoExit",
                "-Command",
                &command,
            ]);
            command_process
        } else {
            let mut command_process = Command::new("powershell");
            command_process.args(["-NoExit", "-Command", &command]);
            command_process
        };

        cmd.spawn().map_err(|e| format!("打开终端失败: {}", e))?;
        return Ok("已在终端执行 Codex CLI 命令".to_string());
    }

    #[allow(unreachable_code)]
    Err("Codex CLI 终端执行仅支持 macOS 和 Windows".to_string())
}
