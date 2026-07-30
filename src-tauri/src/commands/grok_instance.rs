use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::models::grok::GrokAccount;
use crate::models::{InstanceProfile, InstanceProfileView};
use crate::modules::{self, config, grok_account, grok_instance, logger};

const DEFAULT_INSTANCE_ID: &str = "__default__";

fn cleanup_legacy_runtime_dir() {
    let Ok(data_dir) = modules::account::get_data_dir() else {
        return;
    };
    let runtime_dir = data_dir.join("grok_runtime");
    if runtime_dir.exists() {
        let _ = fs::remove_dir_all(runtime_dir);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
    /// 非致命提示（如 refresh token 已吊销需重新授权）。有值时命令仍可生成/执行。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

fn is_grok_reauth_prepare_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("invalid_grant")
        || normalized.contains("refresh token has been revoked")
        || normalized.contains("refresh_token 为空")
        || normalized.contains("access_denied")
        || normalized.contains("reauth")
}

fn is_grok_cli_missing_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("未检测到 grok cli")
        || normalized.contains("grok cli 路径不存在")
        || normalized.contains("请先通过官方安装脚本安装")
        || (normalized.contains("grok cli")
            && (normalized.contains("不存在")
                || normalized.contains("not found")
                || normalized.contains("未检测")))
}

struct GrokLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
    managed: bool,
    /// Official CLI API-key path: export XAI_API_KEY for this launch.
    xai_api_key: Option<String>,
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
        // 无全局当前账号：默认实例仅在显式绑定时使用绑定账号。
        let settings = grok_instance::load_default_settings()?;
        return Ok(settings.bind_account_id);
    }
    let instance = grok_instance::load_instance_store()?
        .instances
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    Ok(instance.bind_account_id)
}

fn resolve_api_key_launch_context(
    account_id: Option<&str>,
) -> Result<(Option<String>, bool), String> {
    let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok((None, false));
    };
    let account = grok_account::load_account(account_id)
        .ok_or_else(|| format!("Grok 账号不存在: {}", account_id))?;
    Ok((
        account.resolved_api_key().map(|value| value.to_string()),
        account.is_api_key_auth(),
    ))
}

#[cfg(not(target_os = "windows"))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn normalize_working_dir_override(working_dir: Option<String>) -> Option<String> {
    working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn should_use_managed_home(
    instance_id: &str,
    sync_official_auth: bool,
    force_managed_home: bool,
) -> bool {
    force_managed_home || instance_id != DEFAULT_INSTANCE_ID || !sync_official_auth
}

fn resolve_context(
    instance_id: &str,
    working_dir_override: Option<Option<String>>,
    account_id_override: Option<&str>,
) -> Result<GrokLaunchContext, String> {
    let launch_account_id = resolve_launch_account_id(instance_id, account_id_override)?;
    let (xai_api_key, force_managed_home) =
        resolve_api_key_launch_context(launch_account_id.as_deref())?;

    if let Some(account_id) = launch_account_id.as_deref() {
        let home = grok_account::managed_profile_dir(account_id)?;
        if instance_id == DEFAULT_INSTANCE_ID {
            let settings = grok_instance::load_default_settings()?;
            let managed = should_use_managed_home(
                instance_id,
                config::get_user_config().grok_sync_official_auth_on_switch,
                force_managed_home,
            );
            return Ok(GrokLaunchContext {
                user_data_dir: if managed {
                    home.to_string_lossy().to_string()
                } else {
                    grok_instance::get_default_grok_home()?
                        .to_string_lossy()
                        .to_string()
                },
                working_dir: match working_dir_override {
                    Some(value) => normalize_working_dir_override(value),
                    None => settings.working_dir,
                },
                extra_args: settings.extra_args,
                managed,
                xai_api_key,
            });
        }

        let instance = grok_instance::load_instance_store()?
            .instances
            .into_iter()
            .find(|instance| instance.id == instance_id)
            .ok_or_else(|| "Grok 实例不存在".to_string())?;
        return Ok(GrokLaunchContext {
            // 鉴权隔离以账号 profile 为准（prepare 阶段会同步写 profile）。
            user_data_dir: home.to_string_lossy().to_string(),
            working_dir: match working_dir_override {
                Some(value) => normalize_working_dir_override(value),
                None => instance.working_dir,
            },
            extra_args: instance.extra_args,
            managed: true,
            xai_api_key,
        });
    }

    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = grok_instance::load_default_settings()?;
        // 未指定账号：不注入凭据，直接启动本机 CLI（读官方默认 home，也不强制 GROK_HOME）。
        return Ok(GrokLaunchContext {
            user_data_dir: grok_instance::get_default_grok_home()?
                .to_string_lossy()
                .to_string(),
            working_dir: match working_dir_override {
                Some(value) => normalize_working_dir_override(value),
                None => settings.working_dir,
            },
            extra_args: settings.extra_args,
            managed: false,
            xai_api_key: None,
        });
    }

    let instance = grok_instance::load_instance_store()?
        .instances
        .into_iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| "Grok 实例不存在".to_string())?;
    grok_instance::ensure_managed_instance_path(Path::new(&instance.user_data_dir))?;
    Ok(GrokLaunchContext {
        user_data_dir: instance.user_data_dir,
        working_dir: match working_dir_override {
            Some(value) => normalize_working_dir_override(value),
            None => instance.working_dir,
        },
        extra_args: instance.extra_args,
        managed: true,
        xai_api_key: None,
    })
}

fn validate_working_dir(context: &GrokLaunchContext) -> Result<Option<&str>, String> {
    let working_dir = context
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(working_dir) = working_dir {
        if !Path::new(working_dir).is_dir() {
            return Err(format!("Grok CLI 工作目录不存在: {}", working_dir));
        }
    }
    Ok(working_dir)
}

fn build_launch_command_with_binary(
    context: &GrokLaunchContext,
    binary: &Path,
) -> Result<String, String> {
    let working_dir = validate_working_dir(context)?;
    let args = modules::process::parse_extra_args(&context.extra_args);

    #[cfg(not(target_os = "windows"))]
    {
        let mut command_parts = Vec::new();
        if let Some(working_dir) = working_dir {
            command_parts.push(format!("cd -- {}", shell_quote(working_dir)));
        }
        let mut command = String::new();
        if let Some(api_key) = context.xai_api_key.as_deref() {
            command.push_str("XAI_API_KEY=");
            command.push_str(&shell_quote(api_key));
            command.push(' ');
        }
        if context.managed {
            command.push_str("GROK_HOME=");
            command.push_str(&shell_quote(&context.user_data_dir));
            command.push(' ');
        }
        command.push_str(&shell_quote(&binary.to_string_lossy()));
        for arg in args {
            let arg = arg.trim();
            if !arg.is_empty() {
                command.push(' ');
                command.push_str(&shell_quote(arg));
            }
        }
        command_parts.push(command);
        return Ok(command_parts.join(" && "));
    }

    #[cfg(target_os = "windows")]
    {
        let mut command_parts = Vec::new();
        if let Some(working_dir) = working_dir {
            command_parts.push(format!(
                "Set-Location -LiteralPath {}",
                powershell_quote(working_dir)
            ));
        }
        if let Some(api_key) = context.xai_api_key.as_deref() {
            command_parts.push(format!("$env:XAI_API_KEY={}", powershell_quote(api_key)));
        }
        if context.managed {
            command_parts.push(format!(
                "$env:GROK_HOME={}",
                powershell_quote(&context.user_data_dir)
            ));
        }
        let mut command = format!("& {}", powershell_quote(&binary.to_string_lossy()));
        for arg in args {
            let arg = arg.trim();
            if !arg.is_empty() {
                command.push(' ');
                command.push_str(&powershell_quote(arg));
            }
        }
        command_parts.push(command);
        return Ok(command_parts.join("; "));
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持生成 Grok CLI 启动命令".to_string())
}

fn build_launch_command(context: &GrokLaunchContext) -> Result<String, String> {
    let (binary, _) = super::grok::resolve_grok_cli_path()?;
    build_launch_command_with_binary(context, &binary)
}

fn profile_view(mut profile: InstanceProfile) -> InstanceProfileView {
    profile.last_pid = None;
    let initialized = grok_instance::is_profile_initialized(Path::new(&profile.user_data_dir));
    InstanceProfileView::from_profile(profile, false, initialized)
}

fn default_view() -> Result<InstanceProfileView, String> {
    let home = grok_instance::get_default_grok_home()?;
    let settings = grok_instance::load_default_settings()?;
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
        initialized: grok_instance::is_profile_initialized(&home),
        is_default: true,
        follow_local_account: settings.follow_local_account,
    })
}

fn write_account_launch_profiles(
    instance_id: &str,
    account: &GrokAccount,
) -> Result<std::path::PathBuf, String> {
    if instance_id == DEFAULT_INSTANCE_ID
        && config::get_user_config().grok_sync_official_auth_on_switch
        && !account.is_api_key_auth()
    {
        grok_account::inject_to_default(&account.id)?;
        return grok_account::default_grok_home();
    }

    let home = grok_account::managed_profile_dir(&account.id)?;
    grok_account::write_account_to_profile(account, &home)?;

    // 多开实例若仍使用独立 user_data_dir 且与 profile 不同：同步一份便于目录自检。
    if instance_id != DEFAULT_INSTANCE_ID {
        if let Ok(store) = grok_instance::load_instance_store() {
            if let Some(instance) = store.instances.iter().find(|item| item.id == instance_id) {
                let instance_dir = Path::new(&instance.user_data_dir);
                if instance_dir != home.as_path() {
                    grok_instance::ensure_managed_instance_path(instance_dir)?;
                    grok_account::write_account_to_profile(account, instance_dir)?;
                }
            }
        }
    }
    Ok(home)
}

/// 准备绑定账号的启动凭据。默认实例按开关选择官方 auth.json 或独立 GROK_HOME；
/// 非默认实例始终使用独立 GROK_HOME。
/// - Ok(None)：凭据就绪
/// - Ok(Some(warning))：需重新授权等非致命问题，已尽量落盘最后已知凭据，启动命令仍可生成
/// - Err：致命错误（账号不存在、CLI 路径等）
async fn prepare_bound_account(
    instance_id: &str,
    account_id_override: Option<&str>,
) -> Result<Option<String>, String> {
    let launch_account_id = resolve_launch_account_id(instance_id, account_id_override)?;
    let Some(account_id) = launch_account_id else {
        return Ok(None);
    };
    match grok_account::prepare_account_for_injection(&account_id).await {
        Ok(account) => {
            write_account_launch_profiles(instance_id, &account)?;
            Ok(None)
        }
        Err(error) if is_grok_reauth_prepare_error(&error) => {
            // invalid_grant / 吊销：账号状态已在 prepare 里落库；启动弹框不展示账号错误，
            // 仅保证仍可生成命令、profile 尽量落盘。
            logger::log_warn(&format!(
                "[Grok Launch] 账号需重新授权（不阻断启动命令）: account_id={}, error={}",
                account_id, error
            ));
            if let Some(account) = grok_account::load_account(&account_id) {
                let should_write_fallback = instance_id != DEFAULT_INSTANCE_ID
                    || !config::get_user_config().grok_sync_official_auth_on_switch;
                if should_write_fallback {
                    if let Err(write_error) = write_account_launch_profiles(instance_id, &account) {
                        logger::log_warn(&format!(
                            "[Grok Launch] reauth 状态下写入 profile 失败: account_id={}, error={}",
                            account_id, write_error
                        ));
                    }
                }
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn grok_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    grok_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn grok_list_instances() -> Result<Vec<InstanceProfileView>, String> {
    cleanup_legacy_runtime_dir();
    let mut views: Vec<_> = grok_instance::load_instance_store()?
        .instances
        .into_iter()
        .map(profile_view)
        .collect();
    views.push(default_view()?);
    Ok(views)
}

#[tauri::command]
pub async fn grok_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
) -> Result<InstanceProfileView, String> {
    let profile = grok_instance::create_instance(grok_instance::CreateInstanceParams {
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
pub async fn grok_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    follow_local_account: Option<bool>,
) -> Result<InstanceProfileView, String> {
    let should_sync_account = bind_account_id.is_some() || follow_local_account.is_some();
    if instance_id == DEFAULT_INSTANCE_ID {
        grok_instance::update_default_settings(
            bind_account_id,
            working_dir,
            extra_args,
            follow_local_account,
        )?;
        if should_sync_account {
            let _ = prepare_bound_account(DEFAULT_INSTANCE_ID, None).await?;
        }
        return default_view();
    }
    let profile = grok_instance::update_instance(grok_instance::UpdateInstanceParams {
        instance_id,
        name,
        working_dir,
        extra_args,
        bind_account_id,
    })?;
    if should_sync_account {
        let _ = prepare_bound_account(&profile.id, None).await?;
    }
    Ok(profile_view(profile))
}

#[tauri::command]
pub async fn grok_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认 Grok 实例不可删除".to_string());
    }
    grok_instance::delete_instance(&instance_id)
}

#[tauri::command]
pub async fn grok_start_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    cleanup_legacy_runtime_dir();
    super::grok::resolve_grok_cli_path()?;
    // 启动准备：reauth 警告不阻断「实例已准备」状态，前端再展示命令/授权提示
    let _ = prepare_bound_account(&instance_id, None).await?;
    if instance_id == DEFAULT_INSTANCE_ID {
        grok_instance::update_default_pid(None)?;
        default_view()
    } else {
        Ok(profile_view(grok_instance::mark_launched(
            &instance_id,
            None,
        )?))
    }
}

#[tauri::command]
pub async fn grok_stop_instance(instance_id: String) -> Result<InstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        grok_instance::update_default_pid(None)?;
        default_view()
    } else {
        Ok(profile_view(grok_instance::update_instance_pid(
            &instance_id,
            None,
        )?))
    }
}

#[tauri::command]
pub async fn grok_close_all_instances() -> Result<(), String> {
    let instance_ids = grok_instance::load_instance_store()?
        .instances
        .into_iter()
        .map(|instance| instance.id)
        .collect::<Vec<_>>();
    grok_instance::update_default_pid(None)?;
    for instance_id in instance_ids {
        grok_instance::update_instance_pid(&instance_id, None)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn grok_open_instance_window(_instance_id: String) -> Result<(), String> {
    Err("Grok CLI 不支持窗口定位，请使用“启动”后的命令在终端中运行".to_string())
}

#[tauri::command]
pub async fn grok_get_instance_launch_command(
    instance_id: String,
    working_dir: Option<String>,
    apply_working_dir_override: Option<bool>,
    account_id: Option<String>,
) -> Result<GrokInstanceLaunchInfo, String> {
    // 先确认 CLI 可用；缺 CLI 才应引导安装
    super::grok::resolve_grok_cli_path()?;
    let override_value = if apply_working_dir_override.unwrap_or(false) {
        Some(working_dir)
    } else {
        None
    };
    let account_id = account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let warning = prepare_bound_account(&instance_id, account_id).await?;
    let context = resolve_context(&instance_id, override_value, account_id)?;
    Ok(GrokInstanceLaunchInfo {
        instance_id,
        user_data_dir: context.user_data_dir.clone(),
        launch_command: build_launch_command(&context)?,
        warning,
    })
}

#[tauri::command]
pub async fn grok_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
    working_dir: Option<String>,
    apply_working_dir_override: Option<bool>,
    account_id: Option<String>,
) -> Result<String, String> {
    super::grok::resolve_grok_cli_path()?;
    let override_value = if apply_working_dir_override.unwrap_or(false) {
        Some(working_dir)
    } else {
        None
    };
    let account_id = account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    // reauth 警告不阻断终端执行；CLI 侧会自行报鉴权失败
    let _warning = prepare_bound_account(&instance_id, account_id).await?;
    let context = resolve_context(&instance_id, override_value, account_id)?;
    let command = build_launch_command(&context)?;
    super::claude::execute_claude_cli_command(&command, terminal)
        .map(|message| message.replace("Claude", "Grok"))
}

#[cfg(test)]
mod tests {
    use super::{
        build_launch_command_with_binary, is_grok_cli_missing_error, is_grok_reauth_prepare_error,
        should_use_managed_home, GrokLaunchContext, DEFAULT_INSTANCE_ID,
    };
    use std::path::Path;

    #[test]
    fn reauth_errors_are_soft_for_launch_prepare() {
        assert!(is_grok_reauth_prepare_error(
            "刷新 Grok token 失败: invalid_grant (Refresh token has been revoked)"
        ));
        assert!(!is_grok_reauth_prepare_error("网络超时"));
    }

    #[test]
    fn cli_missing_errors_are_detected() {
        assert!(is_grok_cli_missing_error(
            "未检测到 Grok CLI，请先通过官方安装脚本安装"
        ));
        assert!(!is_grok_cli_missing_error(
            "刷新 Grok token 失败: invalid_grant"
        ));
    }

    #[test]
    fn default_command_is_direct_and_never_sets_grok_home() {
        let context = GrokLaunchContext {
            user_data_dir: "/tmp/.grok".to_string(),
            working_dir: None,
            extra_args: String::new(),
            managed: false,
            xai_api_key: None,
        };
        let command = build_launch_command_with_binary(&context, Path::new("/opt/grok"))
            .expect("build default command");

        assert!(!command.contains("GROK_HOME"));
        assert!(!command.contains("XAI_API_KEY"));
        assert!(!command.contains("launch-"));
        assert!(!command.contains(".pid"));
        #[cfg(not(target_os = "windows"))]
        assert_eq!(command, "'/opt/grok'");
        #[cfg(target_os = "windows")]
        assert_eq!(command, "& '/opt/grok'");
    }

    #[test]
    fn managed_command_exposes_profile_path_and_arguments() {
        let context = GrokLaunchContext {
            user_data_dir: "/tmp/Grok Home/team's profile".to_string(),
            working_dir: None,
            extra_args: "--label \"team's files\"".to_string(),
            managed: true,
            xai_api_key: None,
        };
        let command = build_launch_command_with_binary(&context, Path::new("/opt/Grok CLI/grok"))
            .expect("build managed command");

        assert_eq!(command.matches("GROK_HOME").count(), 1);
        assert!(!command.contains("launch-"));
        assert!(!command.contains(".pid"));
        assert!(command.contains("team"));
        assert!(command.contains("/opt/Grok CLI/grok"));
    }

    #[test]
    fn api_key_command_exports_xai_api_key() {
        let context = GrokLaunchContext {
            user_data_dir: "/tmp/.grok".to_string(),
            working_dir: None,
            extra_args: String::new(),
            managed: false,
            xai_api_key: Some("xai-test-key".to_string()),
        };
        let command = build_launch_command_with_binary(&context, Path::new("/opt/grok"))
            .expect("build api key command");
        assert!(command.contains("XAI_API_KEY"));
        assert!(command.contains("xai-test-key"));
    }

    #[test]
    fn default_instance_home_follows_official_sync_setting() {
        assert!(should_use_managed_home(DEFAULT_INSTANCE_ID, false, false));
        assert!(!should_use_managed_home(DEFAULT_INSTANCE_ID, true, false));
        assert!(should_use_managed_home(DEFAULT_INSTANCE_ID, true, true));
    }

    #[test]
    fn non_default_instances_always_use_managed_home() {
        assert!(should_use_managed_home("team-instance", false, false));
        assert!(should_use_managed_home("team-instance", true, false));
    }
}
