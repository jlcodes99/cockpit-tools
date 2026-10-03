use std::path::Path;

use serde::Serialize;

use crate::models::{InstanceProfile, InstanceProfileView};
use crate::modules::{self, config, pi_account, pi_instance};

const DEFAULT_INSTANCE_ID: &str = "__default__";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

struct PiLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
    /// When true, export PI_CODING_AGENT_DIR=user_data_dir for this launch.
    managed: bool,
}

fn find_instance(instance_id: &str) -> Result<InstanceProfile, String> {
    pi_instance::load_instance_store()?
        .instances
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| "pi 实例不存在".to_string())
}

fn resolve_launch_account_id(
    instance_id: &str,
    account_id_override: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(account_id) = account_id_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(Some(account_id.to_string()));
    }
    if instance_id == DEFAULT_INSTANCE_ID {
        return Ok(pi_instance::load_default_settings()?.bind_account_id);
    }
    Ok(find_instance(instance_id)?.bind_account_id)
}

#[cfg(not(target_os = "windows"))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn pick_working_dir(override_value: Option<Option<String>>, fallback: Option<String>) -> Option<String> {
    match override_value {
        Some(value) => value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        None => fallback,
    }
}

fn should_use_managed_home(instance_id: &str, sync_official_auth: bool) -> bool {
    instance_id != DEFAULT_INSTANCE_ID || !sync_official_auth
}

fn resolve_context(
    instance_id: &str,
    working_dir_override: Option<Option<String>>,
    account_id_override: Option<&str>,
) -> Result<PiLaunchContext, String> {
    let account_id = resolve_launch_account_id(instance_id, account_id_override)?;
    let sync_official = config::get_user_config().pi_sync_official_auth_on_switch;

    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = pi_instance::load_default_settings()?;
        let working_dir = pick_working_dir(working_dir_override, settings.working_dir);
        if let Some(account_id) = account_id.as_deref() {
            if should_use_managed_home(instance_id, sync_official) {
                return Ok(PiLaunchContext {
                    user_data_dir: pi_account::managed_profile_dir(account_id)?
                        .to_string_lossy()
                        .to_string(),
                    working_dir,
                    extra_args: settings.extra_args,
                    managed: true,
                });
            }
        }
        // 未绑定账号或同步官方目录：直接运行本机 pi（读官方 ~/.pi/agent）。
        return Ok(PiLaunchContext {
            user_data_dir: pi_instance::get_default_pi_home()?
                .to_string_lossy()
                .to_string(),
            working_dir,
            extra_args: settings.extra_args,
            managed: false,
        });
    }

    let instance = find_instance(instance_id)?;
    let working_dir = pick_working_dir(working_dir_override, instance.working_dir.clone());
    if let Some(account_id) = account_id.as_deref() {
        return Ok(PiLaunchContext {
            user_data_dir: pi_account::managed_profile_dir(account_id)?
                .to_string_lossy()
                .to_string(),
            working_dir,
            extra_args: instance.extra_args,
            managed: true,
        });
    }
    pi_instance::ensure_managed_instance_path(Path::new(&instance.user_data_dir))?;
    Ok(PiLaunchContext {
        user_data_dir: instance.user_data_dir,
        working_dir,
        extra_args: instance.extra_args,
        managed: true,
    })
}

fn build_launch_command_with_binary(
    context: &PiLaunchContext,
    binary: &Path,
) -> Result<String, String> {
    let working_dir = context
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(dir) = working_dir {
        if !Path::new(dir).is_dir() {
            return Err(format!("pi 工作目录不存在: {}", dir));
        }
    }
    let args = modules::process::parse_extra_args(&context.extra_args);

    #[cfg(not(target_os = "windows"))]
    {
        let mut parts = Vec::new();
        if let Some(dir) = working_dir {
            parts.push(format!("cd -- {}", shell_quote(dir)));
        }
        let mut command = String::new();
        if context.managed {
            command.push_str(pi_account::PI_HOME_ENV);
            command.push('=');
            command.push_str(&shell_quote(&context.user_data_dir));
            command.push(' ');
        }
        command.push_str(&shell_quote(&binary.to_string_lossy()));
        for arg in args.iter().map(|arg| arg.trim()).filter(|arg| !arg.is_empty()) {
            command.push(' ');
            command.push_str(&shell_quote(arg));
        }
        parts.push(command);
        return Ok(parts.join(" && "));
    }

    #[cfg(target_os = "windows")]
    {
        let mut parts = Vec::new();
        if let Some(dir) = working_dir {
            parts.push(format!("Set-Location -LiteralPath {}", powershell_quote(dir)));
        }
        if context.managed {
            parts.push(format!(
                "$env:{}={}",
                pi_account::PI_HOME_ENV,
                powershell_quote(&context.user_data_dir)
            ));
        }
        let mut command = format!("& {}", powershell_quote(&binary.to_string_lossy()));
        for arg in args.iter().map(|arg| arg.trim()).filter(|arg| !arg.is_empty()) {
            command.push(' ');
            command.push_str(&powershell_quote(arg));
        }
        parts.push(command);
        return Ok(parts.join("; "));
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持生成 pi 启动命令".to_string())
}

fn build_launch_command(context: &PiLaunchContext) -> Result<String, String> {
    let (binary, _) = super::pi::resolve_pi_cli_path()?;
    build_launch_command_with_binary(context, &binary)
}

fn profile_view(mut profile: InstanceProfile) -> InstanceProfileView {
    profile.last_pid = None;
    let initialized = pi_instance::is_profile_initialized(Path::new(&profile.user_data_dir));
    InstanceProfileView::from_profile(profile, false, initialized)
}

fn default_view() -> Result<InstanceProfileView, String> {
    let home = pi_instance::get_default_pi_home()?;
    let settings = pi_instance::load_default_settings()?;
    Ok(InstanceProfileView {
        id: DEFAULT_INSTANCE_ID.to_string(),
        name: String::new(),
        user_data_dir: home.to_string_lossy().to_string(),
        working_dir: settings.working_dir,
        extra_args: settings.extra_args,
        bind_account_id: settings.bind_account_id,
        created_at: 0,
        last_launched_at: None,
        last_pid: None,
        running: false,
        initialized: pi_instance::is_profile_initialized(&home),
        is_default: true,
        follow_local_account: settings.follow_local_account,
    })
}

/// Write the bound account's credentials to wherever the launch will read them.
fn prepare_bound_account(
    instance_id: &str,
    account_id_override: Option<&str>,
) -> Result<(), String> {
    let Some(account_id) = resolve_launch_account_id(instance_id, account_id_override)? else {
        return Ok(());
    };
    let sync_official = config::get_user_config().pi_sync_official_auth_on_switch;
    if should_use_managed_home(instance_id, sync_official) {
        pi_account::prepare_account_home(&account_id)?;
    } else {
        pi_account::inject_to_default(&account_id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pi_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    pi_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn pi_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    let mut views: Vec<_> = pi_instance::load_instance_store()?
        .instances
        .into_iter()
        .map(profile_view)
        .collect();
    views.push(default_view()?);
    Ok(views)
}

#[tauri::command]
pub async fn pi_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let profile = pi_instance::create_instance(pi_instance::CreateInstanceParams {
        name,
        user_data_dir,
        working_dir,
        extra_args: extra_args.unwrap_or_default(),
        bind_account_id,
        copy_source_instance_id,
        init_mode,
    })?;
    Ok(profile_view(profile))
}

#[tauri::command]
pub async fn pi_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    let should_sync_account = bind_account_id.is_some() || follow_local_account.is_some();
    if instance_id == DEFAULT_INSTANCE_ID {
        pi_instance::update_default_settings(
            bind_account_id,
            working_dir,
            extra_args,
            follow_local_account,
        )?;
        if should_sync_account {
            prepare_bound_account(DEFAULT_INSTANCE_ID, None)?;
        }
        return default_view();
    }
    let profile = pi_instance::update_instance(pi_instance::UpdateInstanceParams {
        instance_id,
        name,
        working_dir,
        extra_args,
        bind_account_id,
    })?;
    if should_sync_account {
        prepare_bound_account(&profile.id, None)?;
    }
    Ok(profile_view(profile))
}

#[tauri::command]
pub async fn pi_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认 pi 实例不可删除".to_string());
    }
    pi_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn pi_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    super::pi::resolve_pi_cli_path()?;
    prepare_bound_account(&instance_id, None)?;
    if instance_id == DEFAULT_INSTANCE_ID {
        pi_instance::update_default_pid(None)?;
        default_view()
    } else {
        Ok(profile_view(pi_instance::mark_launched(&instance_id, None)?))
    }
}

#[tauri::command]
pub async fn pi_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        pi_instance::update_default_pid(None)?;
        default_view()
    } else {
        Ok(profile_view(pi_instance::update_instance_pid(&instance_id, None)?))
    }
}

#[tauri::command]
pub async fn pi_close_all_instances() -> Result<(), String> {
    let ids: Vec<String> = pi_instance::load_instance_store()?
        .instances
        .into_iter()
        .map(|instance| instance.id)
        .collect();
    pi_instance::update_default_pid(None)?;
    for id in ids {
        pi_instance::update_instance_pid(&id, None)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pi_open_instance_window(_instance_id: String) -> Result<(), String> {
    Err("pi 不支持窗口定位，请使用“启动”后的命令在终端中运行".to_string())
}

fn normalize_account_override(account_id: &Option<String>) -> Option<&str> {
    account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[tauri::command]
pub async fn pi_get_instance_launch_command(
    instance_id: String,
    working_dir: Option<String>,
    apply_working_dir_override: Option<bool>,
    account_id: Option<String>,
) -> Result<PiInstanceLaunchInfo, String> {
    super::pi::resolve_pi_cli_path()?;
    let override_value = apply_working_dir_override
        .unwrap_or(false)
        .then_some(working_dir);
    let account_id = normalize_account_override(&account_id);
    prepare_bound_account(&instance_id, account_id)?;
    let context = resolve_context(&instance_id, override_value, account_id)?;
    Ok(PiInstanceLaunchInfo {
        instance_id,
        user_data_dir: context.user_data_dir.clone(),
        launch_command: build_launch_command(&context)?,
        warning: None,
    })
}

#[tauri::command]
pub async fn pi_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
    working_dir: Option<String>,
    apply_working_dir_override: Option<bool>,
    account_id: Option<String>,
) -> Result<String, String> {
    super::pi::resolve_pi_cli_path()?;
    let override_value = apply_working_dir_override
        .unwrap_or(false)
        .then_some(working_dir);
    let account_id = normalize_account_override(&account_id);
    prepare_bound_account(&instance_id, account_id)?;
    let context = resolve_context(&instance_id, override_value, account_id)?;
    let command = build_launch_command(&context)?;
    super::claude::execute_claude_cli_command(&command, terminal)
        .map(|message| message.replace("Claude", "pi"))
}

#[cfg(test)]
mod tests {
    use super::{
        build_launch_command_with_binary, should_use_managed_home, PiLaunchContext,
        DEFAULT_INSTANCE_ID,
    };
    use std::path::Path;

    #[test]
    fn default_command_never_sets_agent_dir() {
        let context = PiLaunchContext {
            user_data_dir: "/tmp/.pi/agent".to_string(),
            working_dir: None,
            extra_args: String::new(),
            managed: false,
        };
        let command =
            build_launch_command_with_binary(&context, Path::new("/opt/pi")).expect("command");
        assert!(!command.contains("PI_CODING_AGENT_DIR"));
    }

    #[test]
    fn managed_command_exports_agent_dir_once() {
        let context = PiLaunchContext {
            user_data_dir: "/tmp/pi home/team's".to_string(),
            working_dir: None,
            extra_args: "--model x".to_string(),
            managed: true,
        };
        let command =
            build_launch_command_with_binary(&context, Path::new("/opt/pi")).expect("command");
        assert_eq!(command.matches("PI_CODING_AGENT_DIR").count(), 1);
        assert!(command.contains("--model"));
    }

    #[test]
    fn managed_home_rules() {
        assert!(should_use_managed_home(DEFAULT_INSTANCE_ID, false));
        assert!(!should_use_managed_home(DEFAULT_INSTANCE_ID, true));
        assert!(should_use_managed_home("other", true));
    }
}
