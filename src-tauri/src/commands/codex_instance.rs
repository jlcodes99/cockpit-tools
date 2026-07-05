use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::models::codex::{CodexAccount, CodexAppSpeed};
use crate::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile};
use crate::modules;

const DEFAULT_INSTANCE_ID: &str = "__default__";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLaunchCredentialChange {
    pub from: String,
    pub to: String,
}

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
    pub auto_sync_threads: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_launch_credential_change: Option<CodexLaunchCredentialChange>,
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
            auto_sync_threads: false,
            codex_launch_credential_change: None,
        }
    }

    fn with_launch_credential_change(
        mut self,
        change: Option<CodexLaunchCredentialChange>,
    ) -> Self {
        self.codex_launch_credential_change = change;
        self
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

fn launch_credential_kind_for_account(account: &CodexAccount) -> String {
    if account.is_api_key_auth() {
        "api".to_string()
    } else {
        "account".to_string()
    }
}

fn launch_credential_kind_for_bind_account_id(account_id: &str) -> Option<String> {
    if modules::codex_instance::is_api_service_bind_account_id(account_id)
        || modules::codex_instance::parse_provider_gateway_bind_account_id(account_id).is_some()
        || modules::codex_local_access::is_local_access_runtime_account_id(account_id)
    {
        return Some("api".to_string());
    }

    modules::codex_account::load_account(account_id)
        .map(|account| launch_credential_kind_for_account(&account))
}

fn read_applied_launch_credential_kind_for_dir(data_dir: &Path) -> Option<String> {
    let account_id = modules::codex_account::read_managed_projection_account_id_from_dir(data_dir)?;
    launch_credential_kind_for_bind_account_id(&account_id)
}

async fn inject_bound_account_to_profile(
    profile_dir: &Path,
    bind_account_id: &str,
) -> Result<(), String> {
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::activate_local_access_for_dir(profile_dir).await?;
        return Ok(());
    }

    if let Some(provider_gateway_account_id) =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
    {
        modules::codex_local_access::activate_provider_gateway_for_dir(
            profile_dir,
            &provider_gateway_account_id,
        )
        .await?;
        return Ok(());
    }

    modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    modules::codex_instance::inject_account_to_profile(profile_dir, bind_account_id).await
}

async fn ensure_provider_gateway_for_bind_account(
    profile_dir: &Path,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let Some(bind_account_id) = bind_account_id else {
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    };
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    }
    let Some(provider_gateway_account_id) =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
    else {
        let Some(account) = modules::codex_account::load_account(bind_account_id) else {
            modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
            return Ok(());
        };
        if modules::codex_local_access::account_requires_provider_gateway(&account) {
            modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
            return modules::codex_local_access::ensure_provider_gateway_for_dir(
                profile_dir,
                bind_account_id,
            )
            .await;
        }
        if modules::codex_local_access::account_requires_bound_oauth_local_gateway(&account) {
            modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
            return modules::codex_local_access::ensure_bound_oauth_local_gateway_for_dir(
                profile_dir,
                bind_account_id,
            )
            .await;
        }
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    };
    modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
    modules::codex_local_access::ensure_provider_gateway_for_dir(
        profile_dir,
        &provider_gateway_account_id,
    )
    .await
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
        auto_sync_threads: default_settings.auto_sync_threads,
        codex_launch_credential_change: None,
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

fn sync_codex_threads_across_idle_instances(context: &str) {
    let started = Instant::now();
    let default_settings = match modules::codex_instance::load_default_settings() {
        Ok(settings) => settings,
        Err(error) => {
            modules::logger::log_warn(&format!(
                "[Codex Thread Sync] {}: skipped automatic idle sync, failed to read settings: {}",
                context, error
            ));
            return;
        }
    };
    if !default_settings.auto_sync_threads {
        return;
    }

    match modules::codex_thread_sync::sync_threads_across_instances_if_all_stopped() {
        Ok(Some(summary)) => {
            if summary.total_synced_thread_count > 0 {
                modules::logger::log_info(&format!(
                    "[Codex Thread Sync] {}: synced {} sessions across {} instances, elapsed_ms={}",
                    context,
                    summary.total_synced_thread_count,
                    summary.mutated_instance_count,
                    started.elapsed().as_millis()
                ));
            } else {
                modules::logger::log_info(&format!(
                    "[Codex Thread Sync] {}: completed with no changes, elapsed_ms={}",
                    context,
                    started.elapsed().as_millis()
                ));
            }
        }
        Ok(None) => {
            modules::logger::log_info(&format!(
                "[Codex Thread Sync] {}: skipped because instances are not idle or not enough instances, elapsed_ms={}",
                context,
                started.elapsed().as_millis()
            ));
        }
        Err(error) => {
            modules::logger::log_warn(&format!(
                "[Codex Thread Sync] {}: skipped automatic idle sync: {}",
                context, error
            ));
        }
    }
}

async fn apply_bound_account_to_initialized_profile(
    profile_dir: &Path,
    bind_account_id: Option<&str>,
    context: &str,
) -> Result<Option<CodexLaunchCredentialChange>, String> {
    if !is_profile_initialized(&profile_dir.to_string_lossy()) {
        return Ok(None);
    }

    let previous_kind = read_applied_launch_credential_kind_for_dir(profile_dir);
    if let Some(account_id) = bind_account_id {
        inject_bound_account_to_profile(profile_dir, account_id).await?;
        ensure_provider_gateway_for_bind_account(profile_dir, bind_account_id).await?;
    } else {
        modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
    }
    let launch_credential_change = build_launch_credential_change(
        previous_kind,
        bind_account_id.and_then(launch_credential_kind_for_bind_account_id),
    );
    log_session_visibility_repair_deferred_before_launch(context, &launch_credential_change);
    Ok(launch_credential_change)
}

fn sanitize_codex_config_before_launch(data_dir: &Path) -> Result<(), String> {
    modules::logger::log_info(&format!(
        "[Codex Config] sanitize before launch: data_dir={}",
        data_dir.display()
    ));
    modules::codex_config_format::sanitize_codex_config_toml_file(&data_dir.join("config.toml"))
        .map(|_| ())
}

fn build_launch_credential_change(
    before: Option<String>,
    after: Option<String>,
) -> Option<CodexLaunchCredentialChange> {
    let (Some(from), Some(to)) = (before, after) else {
        return None;
    };
    if from == to {
        return None;
    }
    Some(CodexLaunchCredentialChange { from, to })
}

fn log_session_visibility_repair_deferred_before_launch(
    context: &str,
    launch_provider_change: &Option<CodexLaunchCredentialChange>,
) {
    let Some(change) = launch_provider_change else {
        return;
    };
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] {}: credential kind changed before launch, defer quick repair to frontend notice, from={}, to={}",
        context,
        change.from,
        change.to
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_launch_credential_change_detects_account_to_api_provider_change() {
        let change =
            build_launch_credential_change(Some("account".to_string()), Some("api".to_string()))
                .expect("provider change should trigger session repair");

        assert_eq!(change.from, "account");
        assert_eq!(change.to, "api");
    }

    #[test]
    fn build_launch_credential_change_detects_api_to_account_change() {
        let change =
            build_launch_credential_change(Some("api".to_string()), Some("account".to_string()))
                .expect("credential type change should trigger session repair");

        assert_eq!(change.from, "api");
        assert_eq!(change.to, "account");
    }

    #[test]
    fn build_launch_credential_change_ignores_same_credential_type() {
        let change =
            build_launch_credential_change(Some("api".to_string()), Some("api".to_string()));

        assert!(change.is_none());
    }

    #[test]
    fn launch_credential_kind_treats_local_access_runtime_as_api() {
        assert_eq!(
            launch_credential_kind_for_bind_account_id("codex_local_access_runtime").as_deref(),
            Some("api")
        );
    }
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
    sanitize_codex_config_before_launch(Path::new(&context.user_data_dir))?;
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
    app: AppHandle,
    mode: Option<modules::codex_session_visibility::CodexSessionVisibilityRepairMode>,
    run_id: Option<String>,
    target_provider: Option<String>,
    target_instance_id: Option<String>,
    repair_instance_ids: Option<Vec<String>>,
    session_ids: Option<Vec<String>>,
    dry_run: Option<bool>,
) -> Result<modules::codex_session_visibility::CodexSessionVisibilityRepairSummary, String> {
    let mode =
        mode.unwrap_or(modules::codex_session_visibility::CodexSessionVisibilityRepairMode::Quick);
    let resolved_target_provider = match target_instance_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(instance_id) => Some(
            modules::codex_session_visibility::resolve_session_visibility_target_provider_from_instance_id(
                instance_id,
            )?,
        ),
        None => target_provider,
    };
    let progress_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let reporter =
            |progress: modules::codex_session_visibility::CodexSessionVisibilityRepairProgress| {
                let _ = progress_app.emit(
                    modules::codex_session_visibility::SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT,
                    progress,
                );
            };
        modules::codex_session_visibility::repair_session_visibility_across_instances_with_target(
            mode,
            run_id,
            Some(&reporter),
            resolved_target_provider,
            session_ids,
            repair_instance_ids,
            dry_run.unwrap_or(false),
        )
    })
    .await
    .map_err(|error| format!("修复 Codex 会话可见性任务失败: {}", error))?
}

#[tauri::command]
pub async fn codex_list_session_visibility_repair_providers(
) -> Result<modules::codex_session_visibility::CodexSessionVisibilityRepairProviderList, String> {
    tauri::async_runtime::spawn_blocking(
        modules::codex_session_visibility::list_session_visibility_repair_providers,
    )
    .await
    .map_err(|error| format!("读取 Codex 会话修复 provider 候选失败: {}", error))?
}

#[tauri::command]
pub async fn codex_list_session_visibility_repair_instances(
) -> Result<modules::codex_session_visibility::CodexSessionVisibilityRepairInstanceList, String> {
    tauri::async_runtime::spawn_blocking(
        modules::codex_session_visibility::list_session_visibility_repair_instances,
    )
    .await
    .map_err(|error| format!("读取 Codex 会话修复实例失败: {}", error))?
}

#[tauri::command]
pub async fn codex_list_sessions_across_instances(
    title_query: Option<String>,
    content_query: Option<String>,
) -> Result<Vec<modules::codex_session_manager::CodexSessionRecord>, String> {
    modules::codex_session_manager::list_sessions_across_instances(title_query, content_query)
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
pub async fn codex_delete_trashed_sessions_across_instances(
    session_ids: Vec<String>,
) -> Result<modules::codex_session_manager::CodexSessionTrashDeleteSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::delete_trashed_sessions_across_instances(session_ids)
    })
    .await
    .map_err(|error| format!("永久删除 Codex 废纸篓会话失败: {}", error))?
}

#[tauri::command]
pub async fn codex_empty_session_trash_across_instances(
) -> Result<modules::codex_session_manager::CodexSessionTrashDeleteSummary, String> {
    tauri::async_runtime::spawn_blocking(
        modules::codex_session_manager::empty_session_trash_across_instances,
    )
    .await
    .map_err(|error| format!("清空 Codex 会话废纸篓失败: {}", error))?
}

#[tauri::command]
pub async fn codex_preview_session_export(
    session_ids: Vec<String>,
) -> Result<modules::codex_session_manager::CodexSessionExportPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::preview_session_export(session_ids)
    })
    .await
    .map_err(|error| format!("预览 Codex 会话导出失败: {}", error))?
}

#[tauri::command]
pub async fn codex_export_sessions(
    app: AppHandle,
    session_ids: Vec<String>,
    export_path: String,
    transfer_id: Option<String>,
) -> Result<modules::codex_session_manager::CodexSessionExportSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let progress_app = app.clone();
        let reporter =
            move |progress: modules::codex_session_manager::CodexSessionTransferProgress| {
                let _ = progress_app.emit(
                    modules::codex_session_manager::SESSION_TRANSFER_PROGRESS_EVENT,
                    progress,
                );
            };
        modules::codex_session_manager::export_sessions(
            session_ids,
            export_path,
            transfer_id,
            Some(&reporter),
        )
    })
    .await
    .map_err(|error| format!("导出 Codex 会话失败: {}", error))?
}

#[tauri::command]
pub async fn codex_preview_session_import(
    import_file_path: String,
    target_instance_id: Option<String>,
) -> Result<modules::codex_session_manager::CodexSessionImportPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::preview_session_import(import_file_path, target_instance_id)
    })
    .await
    .map_err(|error| format!("预览 Codex 会话导入失败: {}", error))?
}

#[tauri::command]
pub async fn codex_import_sessions(
    app: AppHandle,
    import_file_path: String,
    target_instance_id: Option<String>,
    session_ids: Vec<String>,
    transfer_id: Option<String>,
) -> Result<modules::codex_session_manager::CodexSessionImportSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let progress_app = app.clone();
        let reporter =
            move |progress: modules::codex_session_manager::CodexSessionTransferProgress| {
                let _ = progress_app.emit(
                    modules::codex_session_manager::SESSION_TRANSFER_PROGRESS_EVENT,
                    progress,
                );
            };
        modules::codex_session_manager::import_sessions(
            import_file_path,
            target_instance_id,
            session_ids,
            transfer_id,
            Some(&reporter),
        )
    })
    .await
    .map_err(|error| format!("导入 Codex 会话失败: {}", error))?
}

#[tauri::command]
pub async fn codex_open_session_location(app: AppHandle, session_id: String) -> Result<(), String> {
    let location_dir = tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::resolve_session_location_dir(session_id)
    })
    .await
    .map_err(|error| format!("打开 Codex 会话位置失败: {}", error))??;
    app.opener()
        .open_path(location_dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| format!("打开 Codex 会话位置失败: {}", error))
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
    auto_sync_threads: Option<bool>,
) -> Result<CodexInstanceProfileView, String> {
    let should_apply_bind_account = bind_account_id.is_some() || follow_local_account.is_some();
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let mut updated = modules::codex_instance::update_default_settings(
            bind_account_id,
            extra_args,
            follow_local_account,
            launch_mode,
            auto_sync_threads,
        )?;
        if let Some(speed) = app_speed {
            updated = modules::codex_instance::update_default_app_speed(speed.clone())?;
            modules::codex_speed::write_app_speed_for_dir(&default_dir, speed)?;
        }
        let resolved_pid = modules::process::resolve_codex_pid(updated.last_pid, None);
        let running = resolved_pid.is_some();
        let default_bind_account_id = resolve_default_account_id(&updated);
        let launch_credential_change = if should_apply_bind_account {
            apply_bound_account_to_initialized_profile(
                &default_dir,
                default_bind_account_id.as_deref(),
                "update-default-bind-account",
            )
            .await?
        } else {
            None
        };
        let _ = working_dir;
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            resolved_pid,
        )
        .with_launch_credential_change(launch_credential_change));
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

    let should_apply_instance_bind_account = bind_account_id.is_some();
    let selected_app_speed = app_speed.clone();
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
    if let Some(speed) = selected_app_speed {
        modules::codex_speed::write_app_speed_for_dir(Path::new(&instance.user_data_dir), speed)?;
    }

    let running = instance
        .last_pid
        .map(modules::process::is_pid_running)
        .unwrap_or(false);
    let initialized = is_profile_initialized(&instance.user_data_dir);
    let launch_credential_change = if should_apply_instance_bind_account {
        apply_bound_account_to_initialized_profile(
            Path::new(&instance.user_data_dir),
            instance.bind_account_id.as_deref(),
            "update-instance-bind-account",
        )
        .await?
    } else {
        None
    };
    Ok(
        CodexInstanceProfileView::from_profile(instance, running, initialized)
            .with_launch_credential_change(launch_credential_change),
    )
}

#[tauri::command]
pub async fn codex_delete_instance(instance_id: String) -> Result<(), String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        return Err("默认实例不可删除".to_string());
    }
    modules::codex_instance::delete_instance(&instance_id)
}

async fn codex_start_instance_internal(
    instance_id: String,
    skip_default_bind_account_injection: bool,
) -> Result<CodexInstanceProfileView, String> {
    let flow_started = Instant::now();
    modules::logger::log_info(&format!(
        "[Codex Start] start_instance_internal started: instance_id={}, skip_default_bind_account_injection={}",
        instance_id, skip_default_bind_account_injection
    ));
    if instance_id == DEFAULT_INSTANCE_ID {
        let prepare_started = Instant::now();
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let previous_kind = read_applied_launch_credential_kind_for_dir(&default_dir);
        let default_settings = modules::codex_instance::load_default_settings()?;
        let default_bind_account_id = resolve_default_account_id(&default_settings);
        if default_settings.launch_mode != InstanceLaunchMode::Cli {
            modules::process::ensure_codex_launch_path_configured()?;
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default prepare phase finished: bind_account_id={:?}, launch_mode={:?}, elapsed_ms={}, total_ms={}",
            default_bind_account_id,
            default_settings.launch_mode,
            prepare_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let close_started = Instant::now();
        let fast_closed = if skip_default_bind_account_injection {
            modules::process::close_codex_default_fast_by_pid(default_settings.last_pid, 20)?
        } else {
            false
        };
        if !fast_closed {
            modules::process::close_codex_default(20)?;
        }
        modules::codex_local_access::stop_provider_gateways_for_profile(&default_dir).await;
        modules::logger::log_info(&format!(
            "[Codex Start] default close phase finished, mode={}, elapsed_ms={}",
            if fast_closed {
                "fast-pid"
            } else {
                "full-probe"
            },
            close_started.elapsed().as_millis()
        ));
        let speed_started = Instant::now();
        let _ = modules::codex_instance::update_default_pid(None)?;
        modules::codex_speed::write_app_speed_for_dir(
            &default_dir,
            default_settings.app_speed.clone(),
        )?;
        modules::logger::log_info(&format!(
            "[Codex Start] default speed/pid reset phase finished: elapsed_ms={}, total_ms={}",
            speed_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let inject_started = Instant::now();
        if let Some(ref account_id) = default_bind_account_id {
            if skip_default_bind_account_injection {
                modules::logger::log_info(&format!(
                    "[Codex Start] skip default bind-account injection because upstream already prepared profile: account_id={}",
                    account_id
                ));
            } else {
                inject_bound_account_to_profile(&default_dir, account_id).await?;
            }
        } else {
            modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(
                &default_dir,
            )?;
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default profile injection phase finished: elapsed_ms={}, total_ms={}",
            inject_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let provider_gateway_started = Instant::now();
        ensure_provider_gateway_for_bind_account(&default_dir, default_bind_account_id.as_deref())
            .await?;
        modules::logger::log_info(&format!(
            "[Codex Start] default provider gateway phase finished: elapsed_ms={}, total_ms={}",
            provider_gateway_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let launch_credential_change = build_launch_credential_change(
            previous_kind,
            default_bind_account_id
                .as_deref()
                .and_then(launch_credential_kind_for_bind_account_id),
        );
        log_session_visibility_repair_deferred_before_launch(
            "before-start-default",
            &launch_credential_change,
        );
        if skip_default_bind_account_injection {
            modules::logger::log_info(
                "[Codex Thread Sync] before-start-default: skipped on prepared-profile fast path",
            );
        } else {
            let thread_sync_started = Instant::now();
            sync_codex_threads_across_idle_instances("before-start-default");
            modules::logger::log_info(&format!(
                "[Codex Start] default thread sync phase finished: elapsed_ms={}, total_ms={}",
                thread_sync_started.elapsed().as_millis(),
                flow_started.elapsed().as_millis()
            ));
        }
        let sanitize_started = Instant::now();
        sanitize_codex_config_before_launch(&default_dir)?;
        modules::logger::log_info(&format!(
            "[Codex Start] default sanitize phase finished: elapsed_ms={}, total_ms={}",
            sanitize_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));

        if default_settings.launch_mode == InstanceLaunchMode::Cli {
            let cli_prepare_started = Instant::now();
            let context = resolve_instance_launch_context(DEFAULT_INSTANCE_ID)?;
            let _ = build_launch_command(&context)?;
            let _ = modules::codex_instance::update_default_pid(None)?;
            modules::logger::log_info(&format!(
                "[Codex Start] default cli prepare finished: elapsed_ms={}, total_ms={}",
                cli_prepare_started.elapsed().as_millis(),
                flow_started.elapsed().as_millis()
            ));
            return Ok(default_instance_view(
                &default_dir,
                &default_settings,
                default_bind_account_id,
                false,
                None,
            )
            .with_launch_credential_change(launch_credential_change));
        }

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let launch_started = Instant::now();
        let pid = if skip_default_bind_account_injection {
            modules::process::start_codex_default_fast_after_close(&extra_args)?
        } else {
            modules::process::start_codex_default(&extra_args)?
        };
        modules::logger::log_info(&format!(
            "[Codex Start] default launch phase finished, pid={}, elapsed_ms={}, total_ms={}",
            pid,
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        modules::codex_model_injector::inject_for_codex_home_later(default_dir.clone());
        let finalize_started = Instant::now();
        let updated = modules::codex_instance::update_default_pid(Some(pid))?;
        let running = modules::process::is_pid_running(pid);
        modules::logger::log_info(&format!(
            "[Codex Start] default finalize phase finished: elapsed_ms={}, total_ms={}",
            finalize_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            Some(pid),
        )
        .with_launch_credential_change(launch_credential_change));
    }

    let prepare_started = Instant::now();
    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    modules::codex_instance::ensure_instance_shared_skills(Path::new(&instance.user_data_dir))?;
    let instance_dir = Path::new(&instance.user_data_dir);
    let previous_kind = read_applied_launch_credential_kind_for_dir(instance_dir);
    modules::logger::log_info(&format!(
        "[Codex Start] instance prepare phase finished: instance_id={}, bind_account_id={:?}, launch_mode={:?}, elapsed_ms={}, total_ms={}",
        instance.id,
        instance.bind_account_id,
        instance.launch_mode,
        prepare_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let close_started = Instant::now();
    if let Some(pid) =
        modules::process::resolve_codex_pid(instance.last_pid, Some(&instance.user_data_dir))
    {
        modules::process::close_pid(pid, 20)?;
        let _ = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    }
    modules::codex_local_access::stop_provider_gateways_for_profile(instance_dir).await;
    modules::logger::log_info(&format!(
        "[Codex Start] instance close/provider-stop phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        close_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let speed_started = Instant::now();
    modules::codex_speed::write_app_speed_for_dir(instance_dir, instance.app_speed.clone())?;
    modules::logger::log_info(&format!(
        "[Codex Start] instance speed phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        speed_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    let inject_started = Instant::now();
    if let Some(ref account_id) = instance.bind_account_id {
        inject_bound_account_to_profile(instance_dir, account_id).await?;
    } else {
        modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(
            instance_dir,
        )?;
    }
    modules::logger::log_info(&format!(
        "[Codex Start] instance profile injection phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        inject_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let provider_gateway_started = Instant::now();
    ensure_provider_gateway_for_bind_account(instance_dir, instance.bind_account_id.as_deref())
        .await?;
    modules::logger::log_info(&format!(
        "[Codex Start] instance provider gateway phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        provider_gateway_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let launch_credential_change = build_launch_credential_change(
        previous_kind,
        instance
            .bind_account_id
            .as_deref()
            .and_then(launch_credential_kind_for_bind_account_id),
    );
    log_session_visibility_repair_deferred_before_launch(
        "before-start-instance",
        &launch_credential_change,
    );
    let thread_sync_started = Instant::now();
    sync_codex_threads_across_idle_instances("before-start-instance");
    modules::logger::log_info(&format!(
        "[Codex Start] instance thread sync phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        thread_sync_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let sanitize_started = Instant::now();
    sanitize_codex_config_before_launch(instance_dir)?;
    modules::logger::log_info(&format!(
        "[Codex Start] instance sanitize phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        sanitize_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    if instance.launch_mode == InstanceLaunchMode::Cli {
        let cli_prepare_started = Instant::now();
        let context = resolve_instance_launch_context(&instance.id)?;
        let _ = build_launch_command(&context)?;
        let updated = modules::codex_instance::update_instance_after_cli_prepare(&instance.id)?;
        let initialized = is_profile_initialized(&updated.user_data_dir);
        modules::logger::log_info(&format!(
            "[Codex Start] instance cli prepare finished: instance_id={}, elapsed_ms={}, total_ms={}",
            instance.id,
            cli_prepare_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        return Ok(
            CodexInstanceProfileView::from_profile(updated, false, initialized)
                .with_launch_credential_change(launch_credential_change),
        );
    }

    modules::process::ensure_codex_launch_path_configured()?;
    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let launch_started = Instant::now();
    let pid = modules::process::start_codex_with_args(&instance.user_data_dir, &extra_args)?;
    modules::logger::log_info(&format!(
        "[Codex Start] instance launch phase finished: instance_id={}, pid={}, elapsed_ms={}, total_ms={}",
        instance.id,
        pid,
        launch_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    modules::codex_model_injector::inject_for_codex_home_later(PathBuf::from(
        &instance.user_data_dir,
    ));
    let finalize_started = Instant::now();
    let updated = modules::codex_instance::update_instance_after_start(&instance.id, pid)?;
    let running = modules::process::is_pid_running(pid);
    let initialized = is_profile_initialized(&updated.user_data_dir);
    modules::logger::log_info(&format!(
        "[Codex Start] instance finalize phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        finalize_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    Ok(
        CodexInstanceProfileView::from_profile(updated, running, initialized)
            .with_launch_credential_change(launch_credential_change),
    )
}

pub(crate) async fn codex_start_default_with_prepared_profile(
) -> Result<CodexInstanceProfileView, String> {
    codex_start_instance_internal(DEFAULT_INSTANCE_ID.to_string(), true).await
}

#[tauri::command]
pub async fn codex_start_instance(instance_id: String) -> Result<CodexInstanceProfileView, String> {
    codex_start_instance_internal(instance_id, false).await
}

#[tauri::command]
pub async fn codex_stop_instance(instance_id: String) -> Result<CodexInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        modules::process::close_codex_default(20)?;
        modules::codex_local_access::stop_provider_gateways_for_profile(&default_dir).await;
        let updated = modules::codex_instance::update_default_pid(None)?;
        let default_bind_account_id = resolve_default_account_id(&updated);
        sync_codex_threads_across_idle_instances("after-stop-default");
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
    modules::codex_local_access::stop_provider_gateways_for_profile(Path::new(
        &instance.user_data_dir,
    ))
    .await;
    let updated = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    sync_codex_threads_across_idle_instances("after-stop-instance");
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
    modules::codex_local_access::stop_provider_gateways_for_profile(&default_home).await;
    for instance in &store.instances {
        let home = instance.user_data_dir.trim();
        if !home.is_empty() {
            modules::codex_local_access::stop_provider_gateways_for_profile(Path::new(home)).await;
        }
    }
    let _ = modules::codex_instance::clear_all_pids();
    sync_codex_threads_across_idle_instances("after-close-all");
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
