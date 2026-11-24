use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use crate::models::codex::{
    CodexAccount, CodexAppSpeed, CodexExperimentalModelDefinition, CodexQuickConfig,
};
use crate::models::{
    CodexInstanceModelRouting, DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile,
};
use crate::modules;

#[path = "codex_instance_terminal.rs"]
pub(super) mod terminal;
use terminal::build_codex_terminal_launch_plan;

#[cfg(test)]
use super::codex_instance_app_exit::idle_codex_profile_dirs_for_app_exit;
pub use super::codex_instance_app_exit::restore_mixed_model_profiles_for_app_exit;
use super::codex_instance_model_catalog::{
    apply_pending_model_catalog, read_pending_model_catalog, restore_pending_model_catalog,
    save_pending_model_catalog,
};
#[cfg(test)]
use super::codex_instance_model_catalog::PENDING_MODEL_CATALOG_FILE;
use super::codex_instance_routing::{
    launch_mode_uses_desktop_runtime, model_routing_update_error,
    validate_instance_model_routing,
};
use super::codex_instance_start_runtime::{stop_runtime_for_start, StartRuntimeState};

#[path = "codex_instance_profile_switch.rs"]
pub(crate) mod profile_switch;

#[path = "codex_instance_cli_resume.rs"]
pub(crate) mod cli_resume;

pub(crate) const DEFAULT_INSTANCE_ID: &str = "__default__";
const CODEX_INSTANCE_LAUNCH_PROGRESS_EVENT: &str = "codex:instance-launch-progress";
static CODEX_INSTANCE_STARTS_IN_PROGRESS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static CODEX_INSTANCE_START_CANCEL_REQUESTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static CODEX_INSTANCE_START_FLOW_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

fn request_codex_instance_start_cancel(instance_id: &str) {
    CODEX_INSTANCE_START_CANCEL_REQUESTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(instance_id.to_string());
}

fn clear_codex_instance_start_cancel(instance_id: &str) {
    CODEX_INSTANCE_START_CANCEL_REQUESTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(instance_id);
}

fn codex_instance_start_cancelled(instance_id: &str) -> bool {
    CODEX_INSTANCE_START_CANCEL_REQUESTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(instance_id)
}

fn ensure_codex_instance_start_not_cancelled(instance_id: &str) -> Result<(), String> {
    if codex_instance_start_cancelled(instance_id) {
        Err("CODEX_START_CANCELLED".to_string())
    } else {
        Ok(())
    }
}

fn should_skip_launch_step(skip_failed_step: Option<&str>, step: &str) -> bool {
    skip_failed_step.is_some_and(|value| value == step || value == "all")
}

#[derive(Debug)]
struct CodexInstanceStartGuard {
    instance_id: String,
}

impl CodexInstanceStartGuard {
    fn acquire(instance_id: &str) -> Result<Self, String> {
        let starts = CODEX_INSTANCE_STARTS_IN_PROGRESS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut starts = starts.lock().unwrap_or_else(|error| error.into_inner());
        if !starts.insert(instance_id.to_string()) {
            return Err("该 Codex 实例正在启动，请稍候".to_string());
        }
        Ok(Self {
            instance_id: instance_id.to_string(),
        })
    }
}

impl Drop for CodexInstanceStartGuard {
    fn drop(&mut self) {
        let starts = CODEX_INSTANCE_STARTS_IN_PROGRESS.get_or_init(|| Mutex::new(HashSet::new()));
        starts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.instance_id);
    }
}

/// 该实例是否正在启动流程中。
///
/// 启动过程内部会先停止旧网关再重新接管，后台监控不能在这段时间释放网关。
pub(crate) fn codex_instance_start_in_progress(instance_id: &str) -> bool {
    CODEX_INSTANCE_STARTS_IN_PROGRESS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map(|starts| starts.contains(instance_id))
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub(crate) struct CodexInstanceStartTarget {
    pub(crate) instance_id: String,
    pub(crate) instance_name: String,
    pub(crate) user_data_dir: PathBuf,
    pub(crate) bind_account_id: Option<String>,
    pub(crate) model_routing: Option<CodexInstanceModelRouting>,
    pub(crate) launch_mode: InstanceLaunchMode,
    pub(crate) is_default: bool,
    pub(crate) launch_operation: Option<String>,
    expected_switch_binding: Option<String>,
}

impl CodexInstanceStartTarget {
    pub(crate) async fn preflight_desktop_proxy(&self, app: Option<&AppHandle>) -> Result<(), String> {
        if launch_mode_uses_desktop_runtime(&self.launch_mode) {
            modules::codex_instance::preflight_egress_proxy_for_bind_account(
                self.bind_account_id.as_deref(),
            ).await?;
            if let Some(app) = app {
                modules::codex_proxy_launch_guard::check_launch(
                    app, self.bind_account_id.as_deref(), &self.user_data_dir, None,
                ).await?;
            } else {
                modules::codex_proxy_launch_guard::check_for_launch(
                    self.bind_account_id.as_deref(), &self.user_data_dir,
                ).await?;
            }
        }
        Ok(())
    }
}

fn emit_codex_instance_launch_progress(
    app: &AppHandle,
    enabled: bool,
    target: &CodexInstanceStartTarget,
    payload: serde_json::Value,
) {
    if !enabled {
        return;
    }
    let mut payload = payload.as_object().cloned().unwrap_or_default();
    payload.insert(
        "instanceId".to_string(),
        serde_json::json!(target.instance_id),
    );
    payload.insert(
        "instanceName".to_string(),
        serde_json::json!(target.instance_name),
    );
    payload.insert(
        "isDefault".to_string(),
        serde_json::json!(target.is_default),
    );
    payload.entry("accountId".to_string()).or_insert_with(|| serde_json::json!(target.bind_account_id));
    if let Some(operation) = target.launch_operation.as_deref() {
        payload.insert("operation".to_string(), serde_json::json!(operation));
    }
    if let Some(binding) = target.expected_switch_binding.as_deref() {
        payload.insert("expectedSwitchBinding".to_string(), serde_json::json!(binding));
    }
    let _ = app.emit(
        CODEX_INSTANCE_LAUNCH_PROGRESS_EVENT,
        serde_json::Value::Object(payload),
    );
}

fn emit_codex_instance_launch_step(
    app: &AppHandle,
    enabled: bool,
    target: &CodexInstanceStartTarget,
    step: &str,
    status: &str,
    progress: u8,
    details: serde_json::Value,
) {
    emit_codex_instance_launch_progress(
        app,
        enabled,
        target,
        serde_json::json!({
            "step": step,
            "stepStatus": status,
            "progress": progress,
            "details": details,
        }),
    );
}

pub(crate) fn resolve_codex_instance_start_target(
    instance_id: &str,
) -> Result<CodexInstanceStartTarget, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = modules::codex_instance::load_default_settings()?;
        return Ok(CodexInstanceStartTarget {
            instance_id: DEFAULT_INSTANCE_ID.to_string(),
            instance_name: String::new(),
            user_data_dir: modules::codex_instance::get_default_codex_home()?,
            bind_account_id: resolve_default_account_id(&settings),
            model_routing: settings.model_routing,
            launch_mode: settings.launch_mode,
            is_default: true,
            launch_operation: None,
            expected_switch_binding: None,
        });
    }

    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    Ok(CodexInstanceStartTarget {
        instance_id: instance.id,
        instance_name: instance.name,
        user_data_dir: PathBuf::from(instance.user_data_dir),
        bind_account_id: instance.bind_account_id,
        model_routing: instance.model_routing,
        launch_mode: instance.launch_mode,
        is_default: false,
        launch_operation: None,
        expected_switch_binding: None,
    })
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_routing: Option<CodexInstanceModelRouting>,
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
            model_routing: profile.model_routing,
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceLaunchInfo {
    pub instance_id: String,
    pub user_data_dir: String,
    pub launch_command: String,
    pub terminal_command: String,
    pub terminal: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceLaunchPreviewInfo {
    pub user_data_dir: String,
    pub launch_command: String,
    pub terminal_command: String,
    pub terminal: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceConfigurationSaveResult {
    pub instance: CodexInstanceProfileView,
    pub quick_config: CodexQuickConfig,
}

struct CodexLaunchContext {
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: String,
}

fn is_profile_initialized(user_data_dir: &str) -> bool {
    modules::instance::is_profile_initialized(Path::new(user_data_dir))
}

pub(super) fn resolve_default_account_id(settings: &DefaultInstanceSettings) -> Option<String> {
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

// Older automatic launches could store a gateway binding for an otherwise direct account.
fn effective_provider_bind_account_id(bind_account_id: &str) -> std::borrow::Cow<'_, str> {
    let Some(account_id) =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
    else {
        return bind_account_id.into();
    };
    if modules::codex_account::load_account(&account_id)
        .as_ref()
        .is_some_and(|account| !modules::codex_local_access::account_requires_provider_gateway(account))
    {
        account_id.into()
    } else {
        bind_account_id.into()
    }
}

async fn inject_bound_account_to_profile(
    profile_dir: &Path,
    bind_account_id: &str,
    revalidate_for_launch: bool,
) -> Result<(), String> {
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::prepare_local_access_for_bound_profile_dir(profile_dir)
            .await?;
        return Ok(());
    }

    if let Some(provider_gateway_account_id) =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
    {
        return modules::codex_local_access::activate_provider_gateway_for_dir(
            profile_dir,
            &provider_gateway_account_id,
        )
        .await;
    }

    modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    if revalidate_for_launch {
        modules::codex_instance::inject_account_to_profile_for_launch(profile_dir, bind_account_id)
            .await
    } else {
        modules::codex_instance::inject_account_to_profile(profile_dir, bind_account_id).await
    }
}

async fn inject_preflighted_bound_account_to_profile(
    profile_dir: &Path,
    bind_account_id: &str,
) -> Result<(), String> {
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::prepare_local_access_for_bound_profile_dir(profile_dir)
            .await?;
        return Ok(());
    }

    let effective_binding = effective_provider_bind_account_id(bind_account_id);
    let bind_account_id = effective_binding.as_ref();
    if let Some(provider_gateway_account_id) =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id)
    {
        return modules::codex_local_access::activate_provider_gateway_for_dir(
            profile_dir,
            &provider_gateway_account_id,
        )
        .await;
    }

    modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
    modules::codex_instance::project_preflighted_account_to_profile_for_launch(
        profile_dir,
        bind_account_id,
    )
    .await
}

async fn ensure_provider_gateway_for_bind_account(
    profile_dir: &Path,
    bind_account_id: Option<&str>,
    model_routing: Option<&CodexInstanceModelRouting>,
    discovered: Option<&modules::codex_provider_discovered_models::LaunchModels>,
) -> Result<(), String> {
    if let Some(routing) = model_routing.filter(|routing| routing.enabled) {
        let oauth_account_id = bind_account_id
            .and_then(|value| {
                modules::codex_account::oauth_account_id_for_runtime_binding(Some(value))
            })
            .ok_or("混合模型路由缺少 OAuth 订阅账号")?;
        return modules::codex_local_access::ensure_mixed_model_gateway_for_dir(
            profile_dir,
            &oauth_account_id,
            routing,
        )
        .await;
    }
    let Some(bind_account_id) = bind_account_id else {
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    };
    if modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    }
    let provider_gateway_account_id =
        modules::codex_instance::parse_provider_gateway_bind_account_id(bind_account_id);
    let account_id = provider_gateway_account_id.as_deref().unwrap_or(bind_account_id);
    let Some(mut account) = modules::codex_account::load_account(account_id) else {
        if provider_gateway_account_id.is_some() {
            return Err(format!("供应商网关账号不存在: {account_id}"));
        }
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
        return Ok(());
    };
    modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
    let requires_gateway = modules::codex_local_access::account_requires_provider_gateway(&account);
    let has_discovered_models = discovered.is_some_and(|models| models.apply_to(&mut account));
    if requires_gateway {
        if has_discovered_models {
            return modules::codex_local_access::ensure_provider_gateway_for_account(
                profile_dir,
                &account,
            )
            .await;
        }
        return modules::codex_local_access::ensure_provider_gateway_for_dir(profile_dir, account_id)
            .await;
    }
    if has_discovered_models {
        let profile_dir = profile_dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            modules::codex_account::sync_discovered_provider_catalog_to_dir(&profile_dir, &account)
        })
        .await
        .map_err(|error| error.to_string())??;
        return Ok(());
    }
    if modules::codex_local_access::account_requires_bound_oauth_local_gateway(&account) {
        return modules::codex_local_access::ensure_bound_oauth_local_gateway_for_dir(profile_dir, account_id)
            .await;
    }
    Ok(())
}

#[cfg(test)]
async fn apply_launch_speed_after_auth(
    profile_dir: &Path,
    bind_account_id: Option<&str>,
    speed: CodexAppSpeed,
) -> Result<(), String> {
    apply_launch_speed_and_catalog_after_auth(profile_dir, bind_account_id, speed, false).await
}

async fn apply_launch_speed_and_catalog_after_auth(
    profile_dir: &Path,
    bind_account_id: Option<&str>,
    speed: CodexAppSpeed,
    desktop: bool,
) -> Result<(), String> {
    let profile_dir = profile_dir.to_path_buf();
    let api_service_binding = bind_account_id
        .is_some_and(modules::codex_instance::is_api_service_bind_account_id);
    let account_id = bind_account_id.map(|id| {
        modules::codex_instance::parse_provider_gateway_bind_account_id(id)
            .unwrap_or_else(|| id.to_string())
    });
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_model_catalog_capabilities::prepare_managed_catalog(&profile_dir)?;
        if api_service_binding && desktop {
            if let Err(error) = modules::codex_model_catalog_capabilities::prepare_dynamic_api_service_catalog(
                &profile_dir,
                modules::process::codex_desktop_supports_dynamic_models,
            ) {
                modules::logger::log_warn(&format!("[Codex模型目录] 动态目录准备失败: {error}"));
            }
        }
        // DeepSeek deliberately removes service_tier as an unsupported option.
        if account_id.as_deref().and_then(modules::codex_account::load_account)
            .is_some_and(|account| modules::codex_account::is_deepseek_account(&account)) {
            return Ok(());
        }
        let speed = if api_service_binding {
            modules::codex_speed::resolve_app_speed_for_launch(&profile_dir, speed)?
        } else {
            speed
        };
        modules::codex_speed::write_app_speed_for_launch(&profile_dir, speed.clone())?;
        if let Err(error) = modules::codex_app_enhancements::prepare_native_fast_for_profile(&profile_dir, desktop) {
            if speed == CodexAppSpeed::Ultrafast {
                return Err(format!("无法准备超高速请求能力: {error}"));
            }
            modules::logger::log_warn(&format!("[Codex Speed] {error}"));
        }
        Ok(())
    }).await.map_err(|error| error.to_string())?
}

async fn restore_mixed_model_gateway_when_disabled(
    profile_dir: &Path,
    model_routing: Option<&CodexInstanceModelRouting>,
) -> Result<(), String> {
    if model_routing.is_some_and(|routing| routing.enabled) {
        return Ok(());
    }
    modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
    modules::codex_local_access::restore_mixed_model_gateway_profile(profile_dir)?;
    modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)
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
        model_routing: default_settings.model_routing.clone(),
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

fn should_apply_instance_binding_immediately(
    binding_changed: bool,
    defer_bind_account_application: Option<bool>,
) -> bool {
    binding_changed && defer_bind_account_application != Some(true)
}

/// 归一化后路由被自动关闭时，即使请求里没有显式更新路由，也要把关闭状态写回配置，
/// 否则后台监控仍会按“启用”反复尝试恢复混合路由网关。
fn merged_model_routing_update(
    requested: Option<Option<CodexInstanceModelRouting>>,
    effective: Option<&CodexInstanceModelRouting>,
    normalized: Option<CodexInstanceModelRouting>,
) -> Option<Option<CodexInstanceModelRouting>> {
    let auto_disabled = effective.is_some_and(|routing| routing.enabled)
        && normalized.as_ref().is_some_and(|routing| !routing.enabled);
    if auto_disabled {
        return Some(normalized);
    }
    requested.map(|_| normalized)
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

async fn sync_codex_threads_across_idle_instances(context: &'static str) {
    if let Err(error) = tauri::async_runtime::spawn_blocking(move || {
        sync_codex_threads_across_idle_instances_blocking(context)
    })
    .await
    {
        modules::logger::log_warn(&format!("[Codex Thread Sync] {}: {}", context, error));
    }
}

fn sync_codex_threads_across_idle_instances_blocking(context: &str) {
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
    model_routing: Option<&CodexInstanceModelRouting>,
    context: &str,
) -> Result<(), String> {
    if !is_profile_initialized(&profile_dir.to_string_lossy()) {
        return Ok(());
    }

    if bind_account_id.is_some() {
        modules::codex_account::preserve_profile_oauth_before_replacement(profile_dir, None).await?;
    }
    restore_mixed_model_gateway_when_disabled(profile_dir, model_routing).await?;
    let previous_kind = read_applied_launch_credential_kind_for_dir(profile_dir);
    if let Some(account_id) = bind_account_id {
        // 已初始化 profile 的绑定变更也可能立即被官方客户端读取。
        // 先刷新实际 OAuth 账号，再写入目标 profile，避免投影旧凭据。
        if let Some(oauth_account_id) =
            modules::codex_account::oauth_account_id_for_runtime_binding(Some(account_id))
        {
            modules::codex_account::prepare_account_for_instance_launch_preflight(
                &oauth_account_id,
            )
            .await?;
        }
        let provider_binding = profile_switch::provider_binding_for_switch(account_id);
        let projected_binding =
            effective_provider_bind_account_id(provider_binding.as_deref().unwrap_or(account_id));
        inject_bound_account_to_profile(profile_dir, &projected_binding, false).await?;
        if modules::codex_instance::parse_provider_gateway_bind_account_id(&projected_binding).is_none() {
            ensure_provider_gateway_for_bind_account(
                profile_dir,
                bind_account_id,
                model_routing,
                None,
            )
            .await?;
        }
    } else {
        modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(profile_dir)?;
        modules::codex_local_access::stop_provider_gateways_for_profile(profile_dir).await;
    }
    let launch_credential_change = build_launch_credential_change(
        previous_kind,
        bind_account_id.and_then(launch_credential_kind_for_bind_account_id),
    );
    log_launch_credential_change(context, &launch_credential_change);
    Ok(())
}

pub use super::codex_instance_gateway_watchdog::start_mixed_model_gateway_watchdog;

async fn created_instance_view_after_binding<F, Fut>(
    instance: InstanceProfile,
    apply_binding: F,
) -> Result<CodexInstanceProfileView, String>
where
    F: FnOnce(PathBuf, String, Option<CodexInstanceModelRouting>) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let initialized = is_profile_initialized(&instance.user_data_dir);
    if let (true, Some(bind_account_id)) = (initialized, instance.bind_account_id.clone()) {
        apply_binding(
            PathBuf::from(&instance.user_data_dir),
            bind_account_id,
            instance.model_routing.clone(),
        )
        .await?;
    }

    Ok(CodexInstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
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

fn log_launch_credential_change(
    context: &str,
    launch_provider_change: &Option<CodexLaunchCredentialChange>,
) {
    let Some(change) = launch_provider_change else {
        return;
    };
    modules::logger::log_info(&format!(
        "[Codex Session Visibility] {}: credential kind changed; the selected instance will reconcile before its next launch, from={}, to={}",
        context,
        change.from,
        change.to
    ));
}

async fn repair_session_visibility_for_selected_instance(
    instance_id: &str,
    instance_name: &str,
    data_dir: &Path,
    launch_mode: &InstanceLaunchMode,
) -> Result<(bool, Option<String>), String> {
    let instance_id = instance_id.to_string();
    let instance_name = instance_name.to_string();
    let data_dir = data_dir.to_path_buf();
    let launch_desktop = launch_mode_uses_desktop_runtime(launch_mode);
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_visibility::repair_session_visibility_for_instance(
            &instance_id,
            &instance_name,
            &data_dir,
            launch_desktop,
            modules::config::get_user_config().codex_automatic_session_repair,
        )
        .map(|summary| (
            !summary.local_project_recovery_deferred,
            summary.local_project_recovery_deferred_message,
        ))
        .or_else(|error| {
            if modules::codex_session_visibility::can_defer_launch_snapshot_error(&error) {
                modules::logger::log_warn(&format!("[Codex Start] session maintenance deferred before writes: instance_id={instance_id}, error={error}"));
                return Ok((false, None));
            }
            Err(format!(
                "同步修复当前 Codex 实例的历史会话可见性失败 ({} / {}): {}",
                instance_name,
                data_dir.display(),
                error
            ))
        })
    })
    .await
    .map_err(|error| format!("等待 Codex 会话可见性修复任务失败: {}", error))?
}

#[cfg(test)]
include!("codex_instance_tests.rs");

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

fn build_launch_command_text(
    context: &CodexLaunchContext,
    binary_path: &str,
    node_path: Option<&str>,
    custom_command: Option<&str>,
) -> Result<String, String> {
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
        #[cfg(target_os = "macos")]
        codex_cmd.push_str(&terminal::macos_loopback_bypass_prefix());
        codex_cmd.push_str("CODEX_HOME=");
        codex_cmd.push_str(&posix_shell_quote(&context.user_data_dir));
        codex_cmd.push(' ');
        if let Some(script) = custom_command {
            if let Some(dir) = Path::new(binary_path).parent().filter(|dir| dir.is_absolute()) {
                codex_cmd.push_str(&format!(
                    "PATH={}:\"$PATH\" ",
                    posix_shell_quote(&dir.to_string_lossy()),
                ));
            }
            codex_cmd.push_str("/bin/sh -c ");
            codex_cmd.push_str(&posix_shell_quote(script));
        } else {
            if let Some(node_path) = node_path {
                codex_cmd.push_str(&posix_shell_quote(node_path));
                codex_cmd.push(' ');
            }
            codex_cmd.push_str(&posix_shell_quote(binary_path));
            for arg in parsed_args.iter().map(|arg| arg.trim()).filter(|arg| !arg.is_empty()) {
                codex_cmd.push(' ');
                codex_cmd.push_str(&posix_shell_quote(arg));
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
                    "Set-Location -LiteralPath {}{}",
                    powershell_quote(dir),
                    if custom_command.is_some() { " -ErrorAction Stop" } else { "" },
                ));
            }
        }

        if let Some(script) = custom_command {
            return Ok(terminal::powershell_cli_script(&command_parts, binary_path, script));
        }

        let mut codex_cmd = String::new();
        if let Some(node_path) = node_path {
            codex_cmd.push_str("& ");
            codex_cmd.push_str(&powershell_quote(node_path));
            codex_cmd.push(' ');
            codex_cmd.push_str(&powershell_quote(binary_path));
        } else {
            codex_cmd.push_str("& ");
            codex_cmd.push_str(&powershell_quote(binary_path));
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

fn build_launch_command_preview(context: &CodexLaunchContext) -> Result<String, String> {
    build_launch_command_text(context, "codex", None, None)
}

#[cfg(all(test, unix))]
#[path = "codex_instance_cli_command_tests.rs"]
mod cli_command_tests;

fn build_launch_command(
    context: &CodexLaunchContext,
    custom_command: Option<&str>,
    prepare_api_service_catalog: bool,
) -> Result<String, String> {
    let profile_dir = Path::new(&context.user_data_dir);
    sanitize_codex_config_before_launch(profile_dir)?;
    let runtime = modules::codex_wakeup::resolve_cli_runtime()?;
    if prepare_api_service_catalog {
        if let Err(error) = modules::codex_model_catalog_capabilities::prepare_dynamic_api_service_catalog(
            profile_dir,
            || modules::process::supports_codex_catalog_url_version(&runtime.version),
        ) {
            modules::logger::log_warn(&format!("[Codex模型目录] 动态目录准备失败: {error}"));
        }
    }
    build_launch_command_text(
        context,
        &runtime.binary_path,
        runtime.node_path.as_deref(),
        custom_command.filter(|value| !value.trim().is_empty()),
    )
}

async fn prepare_cli_instance_launch(
    instance_id: &str,
    bind_account_id: Option<&str>,
) -> Result<(), String> {
    let instance_id = instance_id.to_owned();
    let api_service_binding =
        bind_account_id.is_some_and(modules::codex_instance::is_api_service_bind_account_id);
    tauri::async_runtime::spawn_blocking(move || {
        let context = resolve_instance_launch_context(&instance_id)?;
        build_launch_command(&context, None, api_service_binding).map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?
}

fn resolve_codex_launch_terminal(terminal: Option<String>) -> String {
    terminal
        .unwrap_or_else(|| crate::modules::config::get_user_config().default_terminal)
        .trim()
        .to_string()
}

#[tauri::command]
pub async fn codex_get_instance_defaults() -> Result<modules::instance::InstanceDefaults, String> {
    modules::codex_instance::get_instance_defaults()
}

#[tauri::command]
pub async fn codex_list_instances(fresh: Option<bool>) -> Result<Vec<CodexInstanceProfileView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = modules::codex_instance::load_instance_store()?;
        let default_dir = modules::codex_instance::get_default_codex_home()?;

        let default_settings = store.default_settings.clone();
        let process_entries = if fresh.unwrap_or(false) {
            modules::process::collect_codex_launch_confirmation_process_entries()?
        } else {
            modules::process::collect_codex_process_entries()
        };
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
    }).await.map_err(|error| format!("Codex status worker failed: {error}"))?
}

#[tauri::command]
pub async fn codex_get_session_resume_settings(
    instance_id: String,
) -> Result<modules::codex_session_resume::ResumeSettings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        resolve_codex_instance_start_target(&instance_id)?;
        modules::codex_session_resume::get(&instance_id)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn codex_save_session_resume_settings(
    instance_id: String,
    settings: modules::codex_session_resume::ResumeSettings,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        resolve_codex_instance_start_target(&instance_id)?;
        modules::codex_session_resume::save(&instance_id, &settings)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn codex_get_instance_quick_config(
    instance_id: String,
    api_service_preview: Option<bool>,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = resolve_codex_instance_start_target(&instance_id)?;
        let base_dir = &target.user_data_dir;
        let binding = target.bind_account_id.as_deref();
        let api_service_instance = api_service_preview.unwrap_or(false)
            || (!target.is_default
                && binding.is_some_and(modules::codex_instance::is_api_service_bind_account_id));
        let mut config = modules::codex_account::read_quick_config_from_config_toml(base_dir)?;
        if api_service_instance {
            // 启动预览属于 API 服务时，清单直接按账号池能力计算，
            // 这样实例尚未接管也能看到本次真正会渲染的模型。
            if let Some(models) =
                modules::codex_local_access::api_service_preview_model_definitions(base_dir)
            {
                config.experimental_model_catalog_models = models
                    .into_iter()
                    .map(|model| {
                        config.experimental_model_catalog_models
                            .iter()
                            .find(|saved| saved.model_id == model.model_id)
                            .cloned()
                            .unwrap_or(model)
                    })
                    .collect();
            }
        } else if !target.model_routing.as_ref().is_some_and(|routing| routing.enabled) {
            if let Some(account) = binding
                .and_then(|id| {
                    let provider_id = modules::codex_instance::parse_provider_gateway_bind_account_id(id);
                    modules::codex_account::load_account(provider_id.as_deref().unwrap_or(id))
                })
                .filter(|account| {
                    account.is_api_key_auth()
                        && modules::codex_local_access::account_requires_provider_gateway(account)
                })
            {
                modules::codex_local_access::apply_provider_gateway_catalog_reset_defaults(
                    base_dir, &account, &mut config,
                )?;
            }
        }
        if let Some(draft) = read_pending_model_catalog(base_dir)? {
            draft.apply_to_view(&mut config);
        }
        Ok(config)
    })
    .await
    .map_err(|error| format!("读取 Codex 实例快捷配置后台任务失败: {}", error))?
}

#[tauri::command]
pub async fn codex_save_instance_context_management(
    instance_id: String,
    experimental_mode: bool,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    let base_dir = resolve_instance_base_dir(instance_id.as_str())?;
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_account::save_context_management_for_base_dir(&base_dir, experimental_mode)
    })
    .await
    .map_err(|error| format!("保存 Codex 实例上下文管理开关后台任务失败: {}", error))?
}

fn instance_catalog_used_by_api_service(target: &CodexInstanceStartTarget) -> bool {
    target.is_default
        || target
            .bind_account_id
            .as_deref()
            .is_some_and(modules::codex_instance::is_api_service_bind_account_id)
}

#[tauri::command]
pub async fn codex_save_instance_quick_config(
    instance_id: String,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: Option<bool>,
    experimental_model_catalog_models: Option<
        Vec<crate::models::codex::CodexExperimentalModelDefinition>,
    >,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    let target = resolve_codex_instance_start_target(&instance_id)?;
    let affects_api_service = instance_catalog_used_by_api_service(&target);
    let base_dir = target.user_data_dir;
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let saved = modules::codex_account::save_quick_config_for_base_dir_with_default(
            &base_dir,
            model_context_window,
            auto_compact_token_limit,
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        )?;
        if affects_api_service && experimental_model_catalog_enabled.is_some() {
            modules::codex_local_access::refresh_api_service_experimental_model_ids();
        }
        Ok::<crate::models::codex::CodexQuickConfig, String>(saved)
    })
    .await
    .map_err(|error| format!("保存 Codex 实例快捷配置后台任务失败: {}", error))??;
    if affects_api_service && experimental_model_catalog_enabled.is_some() {
        modules::codex_local_access::trigger_gateway_reload_in_background("实验模型目录已更新");
    }
    Ok(saved)
}

#[tauri::command]
pub async fn codex_save_instance_model_catalog(
    instance_id: String,
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<crate::models::codex::CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<crate::models::codex::CodexQuickConfig, String> {
    let target = resolve_codex_instance_start_target(&instance_id)?;
    let affects_api_service = instance_catalog_used_by_api_service(&target);
    let base_dir = target.user_data_dir;
    if read_pending_model_catalog(&base_dir)?.is_some() {
        return save_pending_model_catalog(
            &base_dir,
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        );
    }
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let saved = modules::codex_account::save_model_catalog_for_base_dir_preserving_context(
            &base_dir,
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        )?;
        if affects_api_service {
            modules::codex_local_access::refresh_api_service_experimental_model_ids();
        }
        Ok::<crate::models::codex::CodexQuickConfig, String>(saved)
    })
    .await
    .map_err(|error| format!("保存 Codex 实例可见模型后台任务失败: {}", error))??;
    if affects_api_service {
        modules::codex_local_access::trigger_gateway_reload_in_background("实验模型目录已更新");
    }
    Ok(saved)
}

/// Save the instance record and its managed model catalog as one compensated transaction.
/// The quick-config write happens first because `codex_update_instance` already rolls its own
/// routing changes back. If the instance update fails, this command restores the old catalog.
#[tauri::command]
pub async fn codex_save_instance_configuration(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    model_routing: Option<Option<CodexInstanceModelRouting>>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
    app_speed: Option<CodexAppSpeed>,
    auto_sync_threads: Option<bool>,
    defer_bind_account_application: Option<bool>,
    update_context_override: Option<bool>,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
    experimental_model_catalog_enabled: bool,
    experimental_model_catalog_models: Vec<CodexExperimentalModelDefinition>,
    experimental_model_catalog_default_model_id: Option<String>,
) -> Result<CodexInstanceConfigurationSaveResult, String> {
    let profile = resolve_instance_base_dir(&instance_id)?;
    if defer_bind_account_application == Some(true) && model_routing.is_some() {
        let previous_pending_catalog = read_pending_model_catalog(&profile)?;
        let previous_quick_config =
            codex_get_instance_quick_config(instance_id.clone(), None).await?;
        let mut quick_config = save_pending_model_catalog(
            &profile, experimental_model_catalog_enabled,
            experimental_model_catalog_models, experimental_model_catalog_default_model_id,
        )?;
        if update_context_override == Some(true) {
            let context_profile = profile.clone();
            let context_result = tauri::async_runtime::spawn_blocking(move || {
                modules::codex_account::save_quick_config_for_base_dir_with_default(
                    &context_profile,
                    model_context_window,
                    auto_compact_token_limit,
                    None,
                    None,
                    None,
                )
            })
            .await
            .map_err(|error| format!("保存 Codex 实例上下文配置后台任务失败: {}", error))
            .and_then(|result| result);
            match context_result {
                Ok(saved_context) => {
                    quick_config.detected_model_context_window =
                        saved_context.detected_model_context_window;
                    quick_config.detected_auto_compact_token_limit =
                        saved_context.detected_auto_compact_token_limit;
                    quick_config.context_window_1m = saved_context.context_window_1m;
                    quick_config.auto_compact_token_limit =
                        saved_context.auto_compact_token_limit;
                }
                Err(error) => {
                    restore_pending_model_catalog(
                        &profile,
                        previous_pending_catalog.as_ref(),
                    )?;
                    return Err(error);
                }
            }
        }
        let result = codex_update_instance(
            instance_id.clone(), name, working_dir, extra_args, bind_account_id, model_routing,
            follow_local_account, launch_mode, app_speed, auto_sync_threads, Some(true),
        ).await;
        return match result {
            Ok(instance) => Ok(CodexInstanceConfigurationSaveResult { instance, quick_config }),
            Err(error) => {
                let context_rollback = if update_context_override == Some(true) {
                    codex_save_instance_quick_config(
                        instance_id,
                        previous_quick_config.detected_model_context_window,
                        previous_quick_config.detected_auto_compact_token_limit,
                        None,
                        None,
                        None,
                    )
                    .await
                    .map(|_| ())
                } else {
                    Ok(())
                };
                let pending_rollback =
                    restore_pending_model_catalog(&profile, previous_pending_catalog.as_ref());
                match (context_rollback, pending_rollback) {
                    (Ok(()), Ok(())) => Err(error),
                    (context_result, pending_result) => Err(format!(
                        "{}; 恢复配置失败: context={:?}, pending={:?}",
                        error,
                        context_result.err(),
                        pending_result.err()
                    )),
                }
            }
        };
    }
    let previous_quick_config = codex_get_instance_quick_config(instance_id.clone(), None).await?;
    let saved_quick_config = if update_context_override == Some(true) {
        codex_save_instance_quick_config(
            instance_id.clone(),
            model_context_window,
            auto_compact_token_limit,
            Some(experimental_model_catalog_enabled),
            Some(experimental_model_catalog_models),
            experimental_model_catalog_default_model_id,
        )
        .await?
    } else {
        codex_save_instance_model_catalog(
            instance_id.clone(),
            experimental_model_catalog_enabled,
            experimental_model_catalog_models,
            experimental_model_catalog_default_model_id,
        )
        .await?
    };

    let update_result = codex_update_instance(
        instance_id.clone(),
        name,
        working_dir,
        extra_args,
        bind_account_id,
        model_routing,
        follow_local_account,
        launch_mode,
        app_speed,
        auto_sync_threads,
        defer_bind_account_application,
    )
    .await;

    match update_result {
        Ok(instance) => Ok(CodexInstanceConfigurationSaveResult {
            instance,
            quick_config: saved_quick_config,
        }),
        Err(update_error) => {
            let rollback_result = codex_save_instance_quick_config(
                instance_id,
                previous_quick_config.detected_model_context_window,
                previous_quick_config.detected_auto_compact_token_limit,
                Some(previous_quick_config.experimental_model_catalog_enabled),
                Some(previous_quick_config.experimental_model_catalog_models),
                previous_quick_config.experimental_model_catalog_default_model_id,
            )
            .await;
            match rollback_result {
                Ok(_) => Err(update_error),
                Err(rollback_error) => Err(format!(
                    "{}；恢复原 Codex 可见模型配置失败: {}",
                    update_error, rollback_error
                )),
            }
        }
    }
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
    tauri::async_runtime::spawn_blocking(modules::codex_thread_sync::sync_threads_across_instances)
        .await
        .map_err(thread_sync_worker_error)?
}

#[tauri::command]
pub async fn codex_sync_sessions_to_instance(
    session_ids: Vec<String>,
    target_instance_id: String,
) -> Result<modules::codex_thread_sync::CodexInstanceTargetThreadSyncSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_thread_sync::sync_sessions_to_instance(session_ids, target_instance_id)
    })
    .await
    .map_err(thread_sync_worker_error)?
}

fn thread_sync_worker_error(error: impl std::fmt::Display) -> String {
    let error = error.to_string();
    modules::i18n::translate(
        &modules::config::get_user_config().language,
        "codex.instance.errors.threadSyncWorkerFailed",
        &[("error", &error)],
    )
}

include!("codex_instance_history_repair.rs");

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
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::get_session_token_stats_across_instances(session_ids)
    })
    .await
    .map_err(|error| format!("读取 Codex 会话 Token 用量失败: {error}"))?
}

#[tauri::command]
pub async fn codex_query_session_usage(
    query: modules::codex_session_usage::CodexSessionUsageQuery,
) -> Result<modules::codex_session_usage::CodexSessionUsageReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_usage::query_session_usage(query)
    })
    .await
    .map_err(|error| format!("读取 Codex 会话用量失败: {error}"))?
}

#[tauri::command]
pub async fn codex_sync_session_usage(
    rebuild: Option<bool>,
    query: Option<modules::codex_session_usage::CodexSessionUsageQuery>,
) -> Result<modules::codex_session_usage::CodexSessionUsageSyncResult, String> {
    let rebuild = rebuild.unwrap_or(false);
    let query = query.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_usage::sync_session_usage(rebuild, query)
    })
    .await
    .map_err(|error| format!("扫描 Codex 会话用量失败: {error}"))?
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
pub async fn codex_validate_session_import_paths(
    cwd_mappings: std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    tokio::time::timeout(std::time::Duration::from_secs(5),
        tauri::async_runtime::spawn_blocking(move || {
            modules::codex_session_import_paths::validate_target_paths(&cwd_mappings)
        }),
    ).await.map_err(|_| "验证项目目录超时，请重试".to_string())?
        .map_err(|error| format!("验证项目目录失败: {}", error))
}

#[tauri::command]
pub async fn codex_import_sessions(
    app: AppHandle,
    import_file_path: String,
    target_instance_id: Option<String>,
    session_ids: Vec<String>,
    cwd_mappings: Option<std::collections::HashMap<String, String>>,
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
            cwd_mappings.unwrap_or_default(),
            transfer_id,
            Some(&reporter),
        )
    })
    .await
    .map_err(|error| format!("导入 Codex 会话失败: {}", error))?
}

#[tauri::command]
pub async fn codex_open_session_location(
    app: AppHandle,
    session_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let location_dir = tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::resolve_session_location_dir(session_id, instance_id)
    })
    .await
    .map_err(|error| format!("打开 Codex 会话位置失败: {}", error))??;
    app.opener()
        .open_path(location_dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| format!("打开 Codex 会话位置失败: {}", error))
}

/// Open the session rollout JSONL with the OS default application (#1510).
#[tauri::command]
pub async fn codex_open_session_rollout(
    app: AppHandle,
    session_id: String,
    instance_id: Option<String>,
) -> Result<(), String> {
    let rollout_path = tauri::async_runtime::spawn_blocking(move || {
        modules::codex_session_manager::resolve_session_rollout_path(session_id, instance_id)
    })
    .await
    .map_err(|error| format!("打开 Codex 会话文件失败: {}", error))??;
    app.opener()
        .open_path(rollout_path.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| format!("打开 Codex 会话文件失败: {}", error))
}

#[tauri::command]
pub async fn codex_create_instance(
    name: String,
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<String>,
    model_routing: Option<CodexInstanceModelRouting>,
    copy_source_instance_id: Option<String>,
    init_mode: Option<String>,
    launch_mode: Option<InstanceLaunchMode>,
    app_speed: Option<CodexAppSpeed>,
    copy_timeout_seconds: Option<u32>,
) -> Result<CodexInstanceProfileView, String> {
    let params = modules::codex_instance::CreateInstanceParams {
        name, user_data_dir, working_dir, extra_args: extra_args.unwrap_or_default(),
        bind_account_id, model_routing, copy_source_instance_id, init_mode, launch_mode, app_speed,
    };
    let copies_profile = params.copies_profile();
    let copy_timeout = copy_timeout_seconds
        .map(|seconds| std::time::Duration::from_secs(u64::from(seconds)))
        .unwrap_or(modules::codex_profile_copy::DEFAULT_COPY_TIMEOUT);
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = tauri::async_runtime::spawn_blocking(move || {
        let mut params = params;
        let effective_launch_mode = params.launch_mode.clone().unwrap_or_default();
        params.model_routing = validate_instance_model_routing(
            params.bind_account_id.as_deref(), &effective_launch_mode, params.model_routing.as_ref(),
        )?;
        modules::codex_instance::create_instance_with_cancellation(
            params,
            &worker_cancelled,
            copy_timeout,
        )
    });
    let result = if copies_profile {
        // The copy worker owns its deadline and staging cleanup. Wait for its
        // result so an in-flight filesystem operation cannot publish after a timeout reply.
        worker.await
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(120), worker).await {
            Ok(result) => result,
            Err(_) => {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
                return Err("创建实例超时，请检查来源目录和磁盘后重试".to_string());
            }
        }
    };
    let instance = result.map_err(|error| format!("创建实例任务失败: {error}"))??;

    created_instance_view_after_binding(
        instance,
        |profile_dir, bind_account_id, model_routing| async move {
            apply_bound_account_to_initialized_profile(
                &profile_dir,
                Some(&bind_account_id),
                model_routing.as_ref(),
                "create-instance-bind-account",
            )
            .await
        },
    )
    .await
}

#[tauri::command]
pub async fn codex_update_instance(
    instance_id: String,
    name: Option<String>,
    working_dir: Option<String>,
    extra_args: Option<String>,
    bind_account_id: Option<Option<String>>,
    model_routing: Option<Option<CodexInstanceModelRouting>>,
    follow_local_account: Option<bool>,
    launch_mode: Option<InstanceLaunchMode>,
    app_speed: Option<CodexAppSpeed>,
    auto_sync_threads: Option<bool>,
    defer_bind_account_application: Option<bool>,
) -> Result<CodexInstanceProfileView, String> {
    let model_routing_update_requested = model_routing.is_some();
    let app_speed_update_requested = app_speed.is_some();
    let should_apply_bind_account = should_apply_instance_binding_immediately(
        bind_account_id.is_some() || model_routing.is_some() || follow_local_account.is_some(),
        defer_bind_account_application,
    );
    if instance_id == DEFAULT_INSTANCE_ID {
        let current = modules::codex_instance::load_default_settings()?;
        let effective_bind_account_id = bind_account_id
            .as_ref()
            .cloned()
            .unwrap_or_else(|| resolve_default_account_id(&current));
        let effective_model_routing = model_routing
            .as_ref()
            .cloned()
            .unwrap_or_else(|| current.model_routing.clone());
        let effective_launch_mode = launch_mode
            .clone()
            .unwrap_or_else(|| current.launch_mode.clone());
        let normalized_effective_model_routing = validate_instance_model_routing(
            effective_bind_account_id.as_deref(),
            &effective_launch_mode,
            effective_model_routing.as_ref(),
        )?;
        let model_routing = merged_model_routing_update(
            model_routing,
            effective_model_routing.as_ref(),
            normalized_effective_model_routing.clone(),
        );
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let mut updated = modules::codex_instance::update_default_settings(
            bind_account_id,
            model_routing,
            extra_args,
            follow_local_account,
            launch_mode,
            auto_sync_threads,
        )?;
        let mut update_error = None;
        if let Some(speed) = app_speed {
            match modules::codex_instance::update_default_app_speed(speed.clone()) {
                Ok(next) => {
                    updated = next;
                    if let Err(error) =
                        modules::codex_speed::write_app_speed_for_dir(&default_dir, speed)
                    {
                        update_error = Some(error);
                    }
                }
                Err(error) => update_error = Some(error),
            }
        }
        let resolved_pid = modules::process::resolve_codex_pid(updated.last_pid, None);
        let running = resolved_pid.is_some();
        let default_bind_account_id = resolve_default_account_id(&updated);
        if update_error.is_none() && should_apply_bind_account {
            if let Err(error) = apply_bound_account_to_initialized_profile(
                &default_dir,
                default_bind_account_id.as_deref(),
                updated.model_routing.as_ref(),
                "update-default-bind-account",
            )
            .await
            {
                update_error = Some(error);
            }
        }
        if let Some(error) = update_error {
            if !model_routing_update_requested {
                return Err(error);
            }
            let mut rollback_errors = Vec::new();
            if let Err(rollback_error) = modules::codex_instance::update_default_settings(
                Some(current.bind_account_id.clone()),
                Some(current.model_routing.clone()),
                Some(current.extra_args.clone()),
                Some(current.follow_local_account),
                Some(current.launch_mode.clone()),
                Some(current.auto_sync_threads),
            ) {
                rollback_errors.push(rollback_error);
            }
            if app_speed_update_requested {
                if let Err(rollback_error) =
                    modules::codex_instance::update_default_app_speed(current.app_speed.clone())
                {
                    rollback_errors.push(rollback_error);
                }
                if let Err(rollback_error) = modules::codex_speed::write_app_speed_for_dir(
                    &default_dir,
                    current.app_speed.clone(),
                ) {
                    rollback_errors.push(rollback_error);
                }
            }
            let previous_bind_account_id = resolve_default_account_id(&current);
            if let Err(rollback_error) = apply_bound_account_to_initialized_profile(
                &default_dir,
                previous_bind_account_id.as_deref(),
                current.model_routing.as_ref(),
                "rollback-default-model-routing",
            )
            .await
            {
                rollback_errors.push(rollback_error);
            }
            return Err(model_routing_update_error(error, rollback_errors));
        }
        let _ = working_dir;
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            resolved_pid,
        ));
    }

    let current = modules::codex_instance::load_instance_store()?
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;
    let effective_bind_account_id = bind_account_id
        .as_ref()
        .cloned()
        .unwrap_or_else(|| current.bind_account_id.clone());
    let effective_model_routing = model_routing
        .as_ref()
        .cloned()
        .unwrap_or_else(|| current.model_routing.clone());
    let effective_launch_mode = launch_mode
        .clone()
        .unwrap_or_else(|| current.launch_mode.clone());
    let normalized_effective_model_routing = validate_instance_model_routing(
        effective_bind_account_id.as_deref(),
        &effective_launch_mode,
        effective_model_routing.as_ref(),
    )?;
    let model_routing = merged_model_routing_update(
        model_routing,
        effective_model_routing.as_ref(),
        normalized_effective_model_routing.clone(),
    );

    let wants_bind = bind_account_id
        .as_ref()
        .and_then(|next| next.as_ref())
        .is_some();
    if wants_bind && defer_bind_account_application != Some(true) {
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

    let should_apply_instance_bind_account = should_apply_instance_binding_immediately(
        bind_account_id.is_some() || model_routing.is_some(),
        defer_bind_account_application,
    );
    let selected_app_speed = app_speed.clone();
    let instance =
        modules::codex_instance::update_instance(modules::codex_instance::UpdateInstanceParams {
            instance_id,
            name,
            working_dir,
            extra_args,
            bind_account_id,
            model_routing,
            launch_mode,
            app_speed,
        })?;
    let mut update_error = selected_app_speed.and_then(|speed| {
        modules::codex_speed::write_app_speed_for_dir(Path::new(&instance.user_data_dir), speed)
            .err()
    });

    let running = instance
        .last_pid
        .map(modules::process::is_pid_running)
        .unwrap_or(false);
    let initialized = is_profile_initialized(&instance.user_data_dir);
    if update_error.is_none() && should_apply_instance_bind_account {
        if let Err(error) = apply_bound_account_to_initialized_profile(
            Path::new(&instance.user_data_dir),
            instance.bind_account_id.as_deref(),
            instance.model_routing.as_ref(),
            "update-instance-bind-account",
        )
        .await
        {
            update_error = Some(error);
        }
    }
    if let Some(error) = update_error {
        if !model_routing_update_requested {
            return Err(error);
        }
        let mut rollback_errors = Vec::new();
        if let Err(rollback_error) = modules::codex_instance::update_instance(
            modules::codex_instance::UpdateInstanceParams {
                instance_id: current.id.clone(),
                name: Some(current.name.clone()),
                working_dir: current.working_dir.clone(),
                extra_args: Some(current.extra_args.clone()),
                bind_account_id: Some(current.bind_account_id.clone()),
                model_routing: Some(current.model_routing.clone()),
                launch_mode: Some(current.launch_mode.clone()),
                app_speed: Some(current.app_speed.clone()),
            },
        ) {
            rollback_errors.push(rollback_error);
        }
        if app_speed_update_requested {
            if let Err(rollback_error) = modules::codex_speed::write_app_speed_for_dir(
                Path::new(&current.user_data_dir),
                current.app_speed.clone(),
            ) {
                rollback_errors.push(rollback_error);
            }
        }
        if let Err(rollback_error) = apply_bound_account_to_initialized_profile(
            Path::new(&current.user_data_dir),
            current.bind_account_id.as_deref(),
            current.model_routing.as_ref(),
            "rollback-instance-model-routing",
        )
        .await
        {
            rollback_errors.push(rollback_error);
        }
        return Err(model_routing_update_error(error, rollback_errors));
    }
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
    let operation = CodexInstanceStartGuard::acquire(&instance_id)?;
    let lookup_id = instance_id.clone();
    let (instance, running) = tauri::async_runtime::spawn_blocking(move || {
        let instance = modules::codex_instance::load_instance_store()?
            .instances
            .into_iter()
            .find(|instance| instance.id == lookup_id)
            .ok_or("实例不存在")?;
        let running = modules::process::resolve_codex_pid(
            instance.last_pid,
            Some(&instance.user_data_dir),
        )
        .is_some();
        Ok::<_, String>((instance, running))
    })
    .await
    .map_err(|error| error.to_string())??;
    if running {
        codex_stop_instance(instance_id.clone()).await?;
    } else {
        modules::codex_local_access::stop_provider_gateways_for_profile(Path::new(
            &instance.user_data_dir,
        ))
        .await;
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        modules::codex_instance::delete_instance(&instance_id)
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 执行默认实例和多开实例共用的完整启动事务。
///
/// 调用方传入实例 ID、profile 是否已经由上游准备、启动来源等差异参数；本方法统一完成
/// Token 预检/刷新、profile 写入、provider gateway 准备和客户端启动，并按需发出统一进度事件。
async fn codex_start_instance_internal(
    app: AppHandle,
    instance_id: String,
    skip_default_bind_account_injection: bool,
    _transfer_conflicting_account: bool,
    skip_failed_step: Option<&str>,
    emit_launch_progress: bool,
    launch_operation: Option<&str>,
    expected_prepared_binding: Option<&str>,
    runtime_state: StartRuntimeState,
    previous_desktop_thread: Option<uuid::Uuid>,
) -> Result<CodexInstanceProfileView, String> {
    let _start_guard = CodexInstanceStartGuard::acquire(&instance_id)?;
    clear_codex_instance_start_cancel(&instance_id);
    let mut launch_target = resolve_codex_instance_start_target(&instance_id)?;
    modules::codex_instance::verify_prepared_launch_binding(
        expected_prepared_binding, launch_target.bind_account_id.as_deref(),
    )?;
    let configured_launch_mode = launch_target.launch_mode.clone();
    let resume_cli = should_resume_cli_after_start(
        runtime_state, expected_prepared_binding, skip_failed_step,
    );
    if matches!(runtime_state, StartRuntimeState::NeedsStop) {
        launch_target.expected_switch_binding = expected_prepared_binding.map(str::to_owned);
    }
    let resume_settings = if configured_launch_mode == InstanceLaunchMode::App {
        let id = instance_id.clone();
        match tauri::async_runtime::spawn_blocking(move || modules::codex_session_resume::get(&id))
            .await.map_err(|error| error.to_string()).and_then(|result| result)
        {
            Ok(settings) => settings.enabled.then_some(settings),
            Err(error) => {
                modules::logger::log_warn(&format!("[Codex Session Resume] 读取设置失败: {error}"));
                None
            }
        }
    } else {
        None
    };
    // 绑定账号不再是可直接登录的 OAuth 订阅账号时，路由按关闭处理：
    // 这里必须用归一化结果，否则启动阶段仍会尝试建立混合路由网关。
    launch_target.model_routing = validate_instance_model_routing(
        launch_target.bind_account_id.as_deref(),
        &configured_launch_mode,
        launch_target.model_routing.as_ref(),
    )?;
    launch_target.launch_operation = launch_operation.map(str::to_owned);
    emit_codex_instance_launch_progress(
        &app,
        emit_launch_progress,
        &launch_target,
        serde_json::json!({
            "type": "start",
            "progress": 2,
            "oauthRuntimePolicy": "latest-runtime-wins",
        }),
    );
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkInstance",
        "running",
        4,
        serde_json::json!({}),
    );
    let start_flow_lock =
        CODEX_INSTANCE_START_FLOW_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _start_flow_guard = start_flow_lock.lock().await;
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    // Check after waiting for the launch lock, but before credentials, profile
    // writes or stopping the previous client. A skipped step cannot bypass this.
    launch_target.preflight_desktop_proxy(Some(&app)).await?;
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    let discovered_models = if should_skip_launch_step(skip_failed_step, "writeProfile") {
        None
    } else {
        modules::codex_provider_discovered_models::for_launch(launch_target.bind_account_id.as_deref())
            .await
    };
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkInstance",
        "completed",
        10,
        serde_json::json!({
            "userDataDir": launch_target.user_data_dir,
        }),
    );
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkAccount",
        "running",
        12,
        serde_json::json!({}),
    );
    let is_api_service_binding = launch_target
        .bind_account_id
        .as_deref()
        .is_some_and(modules::codex_instance::is_api_service_bind_account_id);
    let oauth_account_id = if is_api_service_binding {
        modules::codex_local_access::bound_oauth_account_id_for_instance_start().await?
    } else {
        // 未显式绑定账号时，默认实例/多开实例仍可能已经落盘了官方 OAuth
        // 凭据。按实际 profile 快照解析账号并刷新，避免跳过凭据准备。
        modules::codex_account::oauth_account_id_for_runtime_binding(
            launch_target.bind_account_id.as_deref(),
        )
        .or_else(|| {
            modules::codex_account::oauth_account_id_for_runtime_dir(&launch_target.user_data_dir)
        })
    };
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    let oauth_account = oauth_account_id
        .as_deref()
        .and_then(modules::codex_account::load_account);
    let oauth_access_token_refresh_due = oauth_account.as_ref().is_some_and(|account| {
        modules::codex_oauth::is_token_expired(&account.tokens.access_token)
    });
    let oauth_refresh_required = oauth_access_token_refresh_due;
    let oauth_token_generation_before = oauth_account
        .as_ref()
        .map(|account| account.token_generation)
        .unwrap_or(0);
    let oauth_has_refresh_token = oauth_account
        .as_ref()
        .is_some_and(|account| modules::codex_account::account_has_refresh_token(account));
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkAccount",
        if oauth_account.is_none() {
            "skipped"
        } else if oauth_refresh_required {
            "warning"
        } else {
            "running"
        },
        20,
        serde_json::json!({
            "accountId": oauth_account.as_ref().map(|account| account.id.clone()),
            "accountEmail": oauth_account.as_ref().map(|account| account.email.clone()),
            "accessTokenExpiresAt": oauth_account.as_ref().and_then(|account| {
                modules::codex_oauth::jwt_token_expiration_timestamp(
                    &account.tokens.access_token,
                )
            }),
            "accessTokenRefreshDue": oauth_access_token_refresh_due,
            "refreshRequired": oauth_refresh_required,
            "hasRefreshToken": oauth_has_refresh_token,
            "tokenGenerationBefore": oauth_token_generation_before,
        }),
    );
    if let Some(account_id) = oauth_account_id.as_deref() {
        if launch_mode_uses_desktop_runtime(&launch_target.launch_mode) {
            modules::codex_device_restrictions::ensure_allowed(account_id,
                modules::codex_device_restrictions::DESKTOP).await?;
        }

        if skip_failed_step == Some("checkAccount") {
            emit_codex_instance_launch_step(
                &app,
                emit_launch_progress,
                &launch_target,
                "checkAccount",
                "skipped",
                20,
                serde_json::json!({ "skippedByUser": true }),
            );
        } else {
            modules::codex_account::prepare_account_for_instance_launch_preflight(account_id)
                .await?;
            ensure_codex_instance_start_not_cancelled(&instance_id)?;
        }
        let checked_account = modules::codex_account::load_account(account_id)
            .ok_or_else(|| format!("账号不存在: {}", account_id))?;
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "checkAccount",
            if skip_failed_step == Some("checkAccount") {
                "skipped"
            } else {
                "completed"
            },
            20,
            serde_json::json!({
                "accountId": checked_account.id,
                "accountEmail": checked_account.email,
                "accessTokenExpiresAt": modules::codex_oauth::jwt_token_expiration_timestamp(
                    &checked_account.tokens.access_token,
                ),
                "accessTokenRefreshDue": false,
                "refreshRequired": oauth_refresh_required,
                "hasRefreshToken": modules::codex_account::account_has_refresh_token(
                    &checked_account,
                ),
                "tokenGenerationBefore": oauth_token_generation_before,
                "tokenGenerationChanged": checked_account.token_generation
                    > oauth_token_generation_before,
                "localCredentialsValidated": skip_failed_step != Some("checkAccount"),
                "skippedByUser": skip_failed_step == Some("checkAccount"),
            }),
        );
    }
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkOccupancy",
        "running",
        22,
        serde_json::json!({}),
    );
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    // 同一 OAuth 账号可以被默认实例、多开实例和 API Key 绑定同时使用。
    // 启动前的 Token Authority 已从运行态 profile 回收最新凭据，因此这里不再
    // 以“账号占用”为由阻断，也不会关闭其它正在运行的实例。
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "checkOccupancy",
        "completed",
        28,
        serde_json::json!({ "policy": "latest-runtime-wins" }),
    );
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "stopPrevious",
        "skipped",
        40,
        serde_json::json!({ "preserveOtherOauthRuntimes": true }),
    );
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
        modules::codex_instance::verify_prepared_launch_binding(
            expected_prepared_binding, default_bind_account_id.as_deref(),
        )?;
        if default_settings.launch_mode != InstanceLaunchMode::Cli {
            tauri::async_runtime::spawn_blocking(modules::process::ensure_codex_launch_path_configured)
                .await.map_err(|error| error.to_string())??;
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default prepare phase finished: bind_account_id={:?}, launch_mode={:?}, elapsed_ms={}, total_ms={}",
            default_bind_account_id,
            default_settings.launch_mode,
            prepare_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let close_started = Instant::now();
        let selected = if matches!(runtime_state, StartRuntimeState::NeedsStop)
            && resume_settings.as_ref().is_some_and(|settings| settings.uses_previous_desktop_session())
        {
            modules::codex_app_injection::capture_before_desktop_stop(&instance_id).await
        } else {
            None
        };
        modules::codex_app_injection::stop_for_profile(&default_dir);
        let close_mode = if launch_mode_uses_desktop_runtime(&default_settings.launch_mode) {
            stop_runtime_for_start(runtime_state, || modules::process::close_codex_default(20))
                .await?
        } else {
            modules::logger::log_info("[Codex Start] CLI 模式无需关闭桌面运行态，继续准备实例配置");
            "cli-no-desktop"
        };
        modules::codex_app_injection::record_after_desktop_stop(&instance_id, selected).await;
        if default_bind_account_id.is_some()
            && !skip_default_bind_account_injection
            && !should_skip_launch_step(skip_failed_step, "prepareCredentials")
        {
            modules::codex_account::preserve_profile_oauth_before_replacement(&default_dir, None).await?;
        }
        modules::codex_local_access::stop_provider_gateways_for_profile(&default_dir).await;
        restore_mixed_model_gateway_when_disabled(
            &default_dir,
            default_settings.model_routing.as_ref(),
        )
        .await?;
        apply_pending_model_catalog(&default_dir)?;
        ensure_codex_instance_start_not_cancelled(&instance_id)?;
        modules::logger::log_info(&format!(
            "[Codex Start] default close phase finished, mode={}, elapsed_ms={}",
            close_mode,
            close_started.elapsed().as_millis()
        ));
        let speed_started = Instant::now();
        let _ = modules::codex_instance::update_default_pid(None)?;
        modules::logger::log_info(&format!(
            "[Codex Start] default pid reset phase finished: elapsed_ms={}, total_ms={}",
            speed_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let inject_started = Instant::now();
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "prepareCredentials",
            if default_bind_account_id.is_some() {
                "running"
            } else {
                "skipped"
            },
            46,
            serde_json::json!({
                "refreshRequired": oauth_refresh_required,
                "accessTokenRefreshDue": oauth_access_token_refresh_due,
                "hasRefreshToken": oauth_has_refresh_token,
                "tokenGenerationBefore": oauth_token_generation_before,
            }),
        );
        if let Some(ref account_id) = default_bind_account_id {
            if should_skip_launch_step(skip_failed_step, "prepareCredentials") {
                modules::logger::log_warn(&format!(
                    "[Codex Start] 用户选择跳过凭据准备步骤: instance_id={}, account_id={}",
                    instance_id, account_id
                ));
                emit_codex_instance_launch_step(
                    &app,
                    emit_launch_progress,
                    &launch_target,
                    "prepareCredentials",
                    "skipped",
                    62,
                    serde_json::json!({ "skippedByUser": true }),
                );
            } else if skip_default_bind_account_injection {
                modules::logger::log_info(&format!(
                    "[Codex Start] skip default bind-account injection because upstream already prepared profile: account_id={}",
                    account_id
                ));
            } else {
                inject_preflighted_bound_account_to_profile(&default_dir, account_id).await?;
                ensure_codex_instance_start_not_cancelled(&instance_id)?;
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
        // Auth/provider restoration can restore a previous service_tier. Apply
        // the selected speed afterwards, before the gateway reads its defaults.
        apply_launch_speed_and_catalog_after_auth(&default_dir, default_bind_account_id.as_deref(), default_settings.app_speed.clone(), default_settings.launch_mode != InstanceLaunchMode::Cli).await?;
        let refreshed_oauth_account = oauth_account_id
            .as_deref()
            .and_then(modules::codex_account::load_account);
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "prepareCredentials",
            if default_bind_account_id.is_some() {
                "completed"
            } else {
                "skipped"
            },
            62,
            serde_json::json!({
                "refreshRequired": oauth_refresh_required,
                "tokenGenerationChanged": refreshed_oauth_account.as_ref().is_some_and(|account| {
                    account.token_generation > oauth_token_generation_before
                }),
            }),
        );
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "writeProfile",
            "completed",
            70,
            serde_json::json!({}),
        );
        let provider_gateway_started = Instant::now();
        if should_skip_launch_step(skip_failed_step, "writeProfile") {
            modules::logger::log_warn("[Codex Start] 用户选择跳过 provider gateway 准备步骤");
            emit_codex_instance_launch_step(
                &app,
                emit_launch_progress,
                &launch_target,
                "writeProfile",
                "skipped",
                70,
                serde_json::json!({ "skippedByUser": true }),
            );
        } else {
            ensure_provider_gateway_for_bind_account(
                &default_dir,
                default_bind_account_id.as_deref(),
                default_settings.model_routing.as_ref(),
                discovered_models.as_ref(),
            )
            .await?;
            ensure_codex_instance_start_not_cancelled(&instance_id)?;
        }
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
        log_launch_credential_change("before-start-default", &launch_credential_change);
        let sanitize_started = Instant::now();
        if should_skip_launch_step(skip_failed_step, "writeProfile") {
            modules::logger::log_warn("[Codex Start] 用户选择跳过配置清理步骤");
        } else {
            sanitize_codex_config_before_launch(&default_dir)?;
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default sanitize phase finished: elapsed_ms={}, total_ms={}",
            sanitize_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let visibility_repair_started = Instant::now();
        if should_skip_launch_step(skip_failed_step, "writeProfile") {
            modules::logger::log_warn("[Codex Start] 用户选择跳过会话可见性修复步骤");
        } else {
            let (repaired, detail) = repair_session_visibility_for_selected_instance(
                DEFAULT_INSTANCE_ID,
                "默认实例",
                &default_dir,
                &default_settings.launch_mode,
            )
            .await
            .map_err(|error| format!("Codex 启动已取消: {}", error))?;
            if !repaired {
                emit_codex_instance_launch_step(&app, emit_launch_progress, &launch_target,
                    "writeProfile", "warning", 80, serde_json::json!({
                        "sessionMaintenanceDeferred": true, "projectRecoveryDeferredMessage": detail
                    }));
            }
            ensure_codex_instance_start_not_cancelled(&instance_id)?;
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default session visibility repair phase finished: elapsed_ms={}, total_ms={}",
            visibility_repair_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));

        if default_settings.launch_mode == InstanceLaunchMode::Cli {
            let cli_prepare_started = Instant::now();
            prepare_cli_instance_launch(DEFAULT_INSTANCE_ID, default_bind_account_id.as_deref()).await?;
            let _ = modules::codex_instance::update_default_pid(None)?;
            let resume =
                resume_cli_after_start(resume_cli, DEFAULT_INSTANCE_ID, &default_dir).await;
            modules::logger::log_info(&format!(
                "[Codex Start] default cli prepare finished: elapsed_ms={}, total_ms={}",
                cli_prepare_started.elapsed().as_millis(),
                flow_started.elapsed().as_millis()
            ));
            emit_cli_start_result(&app, emit_launch_progress, &launch_target, resume)?;
            emit_codex_instance_launch_progress(
                &app,
                emit_launch_progress,
                &launch_target,
                serde_json::json!({ "type": "complete", "progress": 100 }),
            );
            return Ok(default_instance_view(
                &default_dir,
                &default_settings,
                default_bind_account_id,
                false,
                None,
            ));
        }

        let extra_args = modules::process::parse_extra_args(&default_settings.extra_args);
        let cdp_enabled = resume_settings.is_some()
            || modules::config::get_user_config().codex_auto_switch_enabled
            || modules::codex_app_injection::should_enable_cdp(default_bind_account_id.as_deref());
        let mut injection_plan =
            modules::codex_app_injection::build_launch_args(&extra_args, cdp_enabled)?;
        let launch_proxy = modules::codex_instance::resolve_egress_proxy_for_bind_account(
            default_bind_account_id.as_deref(),
        ).await?;
        modules::codex_proxy_dotenv::apply(&default_dir, default_bind_account_id.as_deref().unwrap_or("__client__"), &launch_proxy)?;
        modules::codex_proxy_launch_guard::check_launch(&app, default_bind_account_id.as_deref(), &default_dir, Some(&launch_proxy)).await?;
        modules::codex_proxy_launch_guard::verify_profile(&default_dir, default_bind_account_id.as_deref().unwrap_or("__client__"), &launch_proxy)?;
        launch_proxy.append_electron_args(&mut injection_plan.args);
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "startClient",
            "running",
            84,
            serde_json::json!({ "launchMode": "app" }),
        );
        let launch_started = Instant::now();
        ensure_codex_instance_start_not_cancelled(&instance_id)?;
        modules::codex_app_injection::start_for_launched_profile(
            app.clone(),
            DEFAULT_INSTANCE_ID.to_string(),
            default_dir.clone(),
            injection_plan.port,
            default_bind_account_id.clone(),
        );
        let launch_args = injection_plan.args.clone();
        let prepared_home = default_dir.clone();
        let verify_proxy = launch_proxy.clone();
        // Package registration, activation and PID confirmation can wait on
        // Windows services. Keep that synchronous work off the async runtime.
        let pid = tauri::async_runtime::spawn_blocking(move || {
            modules::process::start_codex_default_prepared_with_egress(
                &launch_args,
                &prepared_home,
                skip_default_bind_account_injection,
                &launch_proxy,
            )
        })
        .await
        .map_err(|error| format!("Codex launch worker failed: {error}"))??;
        modules::codex_proxy_launch_guard::verify_or_stop(&default_dir, default_bind_account_id.as_deref().unwrap_or("__client__"), pid, &verify_proxy)?;
        if codex_instance_start_cancelled(&instance_id) {
            let _ = modules::process::close_pid(pid, 5);
            return Err("CODEX_START_CANCELLED".to_string());
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default launch phase finished, pid={}, elapsed_ms={}, total_ms={}",
            pid,
            launch_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        let finalize_started = Instant::now();
        let updated = modules::codex_instance::update_default_pid(Some(pid))?;
        let running = modules::process::is_pid_running(pid);
        if running {
            if let (Some(settings), Some(port)) = (resume_settings, injection_plan.port) {
                modules::codex_app_injection::schedule_desktop_resume(
                    instance_id.clone(), pid, port, settings,
                );
            } else if let Some(thread) = previous_desktop_thread {
                tauri::async_runtime::spawn(
                    modules::codex_app_injection::reopen_desktop_thread_after_switch(pid, thread),
                );
            }
        }
        modules::logger::log_info(&format!(
            "[Codex Start] default finalize phase finished: elapsed_ms={}, total_ms={}",
            finalize_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "startClient",
            "completed",
            96,
            serde_json::json!({ "pid": pid }),
        );
        emit_codex_instance_launch_progress(
            &app,
            emit_launch_progress,
            &launch_target,
            serde_json::json!({ "type": "complete", "progress": 100 }),
        );
        return Ok(default_instance_view(
            &default_dir,
            &updated,
            default_bind_account_id,
            running,
            Some(pid),
        ));
    }

    let prepare_started = Instant::now();
    let store = modules::codex_instance::load_instance_store()?;
    let instance = store
        .instances
        .into_iter()
        .find(|item| item.id == instance_id)
        .ok_or("实例不存在")?;

    modules::codex_instance::verify_prepared_launch_binding(
        expected_prepared_binding, instance.bind_account_id.as_deref(),
    )?;
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
    let selected = if matches!(runtime_state, StartRuntimeState::NeedsStop)
        && resume_settings.as_ref().is_some_and(|settings| settings.uses_previous_desktop_session())
    {
        modules::codex_app_injection::capture_before_desktop_stop(&instance_id).await
    } else {
        None
    };
    modules::codex_app_injection::stop_for_profile(instance_dir);
    let target_home = instance.user_data_dir.clone();
    stop_runtime_for_start(runtime_state, move || {
        modules::process::close_codex_instances(&[target_home], 20)
    }).await?;
    modules::codex_app_injection::record_after_desktop_stop(&instance_id, selected).await;
    if instance.bind_account_id.is_some()
        && !should_skip_launch_step(skip_failed_step, "prepareCredentials")
    {
        modules::codex_account::preserve_profile_oauth_before_replacement(instance_dir, None).await?;
    }
    let _ = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    modules::codex_local_access::stop_provider_gateways_for_profile(instance_dir).await;
    restore_mixed_model_gateway_when_disabled(instance_dir, instance.model_routing.as_ref())
        .await?;
    apply_pending_model_catalog(instance_dir)?;
    modules::logger::log_info(&format!(
        "[Codex Start] instance close/provider-stop phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        close_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let inject_started = Instant::now();
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "prepareCredentials",
        if instance.bind_account_id.is_some() {
            "running"
        } else {
            "skipped"
        },
        46,
        serde_json::json!({
            "refreshRequired": oauth_refresh_required,
            "accessTokenRefreshDue": oauth_access_token_refresh_due,
            "hasRefreshToken": oauth_has_refresh_token,
            "tokenGenerationBefore": oauth_token_generation_before,
        }),
    );
    if let Some(ref account_id) = instance.bind_account_id {
        if should_skip_launch_step(skip_failed_step, "prepareCredentials") {
            modules::logger::log_warn(&format!(
                "[Codex Start] 用户选择跳过凭据准备步骤: instance_id={}, account_id={}",
                instance.id, account_id
            ));
            emit_codex_instance_launch_step(
                &app,
                emit_launch_progress,
                &launch_target,
                "prepareCredentials",
                "skipped",
                62,
                serde_json::json!({ "skippedByUser": true }),
            );
        } else {
            inject_preflighted_bound_account_to_profile(instance_dir, account_id).await?;
            ensure_codex_instance_start_not_cancelled(&instance_id)?;
        }
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
    apply_launch_speed_and_catalog_after_auth(instance_dir, instance.bind_account_id.as_deref(), instance.app_speed.clone(), instance.launch_mode != InstanceLaunchMode::Cli).await?;
    let refreshed_oauth_account = oauth_account_id
        .as_deref()
        .and_then(modules::codex_account::load_account);
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "prepareCredentials",
        if instance.bind_account_id.is_some() {
            "completed"
        } else {
            "skipped"
        },
        62,
        serde_json::json!({
            "refreshRequired": oauth_refresh_required,
            "tokenGenerationChanged": refreshed_oauth_account.as_ref().is_some_and(|account| {
                account.token_generation > oauth_token_generation_before
            }),
        }),
    );
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "writeProfile",
        "completed",
        70,
        serde_json::json!({}),
    );
    let provider_gateway_started = Instant::now();
    if should_skip_launch_step(skip_failed_step, "writeProfile") {
        modules::logger::log_warn("[Codex Start] 用户选择跳过 provider gateway 准备步骤");
        emit_codex_instance_launch_step(
            &app,
            emit_launch_progress,
            &launch_target,
            "writeProfile",
            "skipped",
            70,
            serde_json::json!({ "skippedByUser": true }),
        );
    } else {
        ensure_provider_gateway_for_bind_account(
            instance_dir,
            instance.bind_account_id.as_deref(),
            instance.model_routing.as_ref(),
            discovered_models.as_ref(),
        )
        .await?;
        ensure_codex_instance_start_not_cancelled(&instance_id)?;
    }
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
    log_launch_credential_change("before-start-instance", &launch_credential_change);
    let sanitize_started = Instant::now();
    if should_skip_launch_step(skip_failed_step, "writeProfile") {
        modules::logger::log_warn("[Codex Start] 用户选择跳过配置清理步骤");
    } else {
        sanitize_codex_config_before_launch(instance_dir)?;
    }
    modules::logger::log_info(&format!(
        "[Codex Start] instance sanitize phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        sanitize_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let visibility_repair_started = Instant::now();
    if should_skip_launch_step(skip_failed_step, "writeProfile") {
        modules::logger::log_warn("[Codex Start] 用户选择跳过会话可见性修复步骤");
    } else {
        let (repaired, detail) = repair_session_visibility_for_selected_instance(
            &instance.id, &instance.name, instance_dir, &instance.launch_mode,
        )
            .await
            .map_err(|error| format!("Codex 启动已取消: {}", error))?;
        if !repaired {
            emit_codex_instance_launch_step(&app, emit_launch_progress, &launch_target,
                "writeProfile", "warning", 80, serde_json::json!({
                    "sessionMaintenanceDeferred": true, "projectRecoveryDeferredMessage": detail
                }));
        }
        ensure_codex_instance_start_not_cancelled(&instance_id)?;
    }
    modules::logger::log_info(&format!(
        "[Codex Start] instance session visibility repair phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        visibility_repair_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));

    if instance.launch_mode == InstanceLaunchMode::Cli {
        let cli_prepare_started = Instant::now();
        prepare_cli_instance_launch(&instance.id, instance.bind_account_id.as_deref()).await?;
        let updated = modules::codex_instance::update_instance_after_cli_prepare(&instance.id)?;
        let initialized = is_profile_initialized(&updated.user_data_dir);
        let resume = resume_cli_after_start(resume_cli, &instance.id, instance_dir).await;
        modules::logger::log_info(&format!(
            "[Codex Start] instance cli prepare finished: instance_id={}, elapsed_ms={}, total_ms={}",
            instance.id,
            cli_prepare_started.elapsed().as_millis(),
            flow_started.elapsed().as_millis()
        ));
        emit_cli_start_result(&app, emit_launch_progress, &launch_target, resume)?;
        emit_codex_instance_launch_progress(
            &app,
            emit_launch_progress,
            &launch_target,
            serde_json::json!({ "type": "complete", "progress": 100 }),
        );
        return Ok(CodexInstanceProfileView::from_profile(
            updated,
            false,
            initialized,
        ));
    }

    tauri::async_runtime::spawn_blocking(modules::process::ensure_codex_launch_path_configured)
        .await.map_err(|error| error.to_string())??;
    let extra_args = modules::process::parse_extra_args(&instance.extra_args);
    let cdp_enabled = resume_settings.is_some()
        || modules::codex_app_injection::should_enable_cdp(instance.bind_account_id.as_deref());
    let mut injection_plan =
        modules::codex_app_injection::build_launch_args(&extra_args, cdp_enabled)?;
    let launch_proxy = modules::codex_instance::resolve_egress_proxy_for_bind_account(
        instance.bind_account_id.as_deref(),
    ).await?;
    modules::codex_proxy_dotenv::apply(instance_dir, instance.bind_account_id.as_deref().unwrap_or("__client__"), &launch_proxy)?;
    modules::codex_proxy_launch_guard::check_launch(&app, instance.bind_account_id.as_deref(), instance_dir, Some(&launch_proxy)).await?;
    modules::codex_proxy_launch_guard::verify_profile(instance_dir, instance.bind_account_id.as_deref().unwrap_or("__client__"), &launch_proxy)?;
    launch_proxy.append_electron_args(&mut injection_plan.args);
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "startClient",
        "running",
        84,
        serde_json::json!({ "launchMode": "app" }),
    );
    let launch_started = Instant::now();
    ensure_codex_instance_start_not_cancelled(&instance_id)?;
    modules::codex_app_injection::start_for_launched_profile(
        app.clone(),
        instance.id.clone(),
        instance_dir.to_path_buf(),
        injection_plan.port,
        instance.bind_account_id.clone(),
    );
    let launch_home = instance.user_data_dir.clone();
    let verify_proxy = launch_proxy.clone();
    let launch_args = injection_plan.args.clone();
    let pid = tauri::async_runtime::spawn_blocking(move || {
        modules::process::start_codex_with_args_and_env_and_egress(
            &launch_home,
            &launch_args,
            &[],
            &launch_proxy,
        )
    })
    .await
    .map_err(|error| format!("Codex launch worker failed: {error}"))??;
    modules::codex_proxy_launch_guard::verify_or_stop(instance_dir, instance.bind_account_id.as_deref().unwrap_or("__client__"), pid, &verify_proxy)?;
    if codex_instance_start_cancelled(&instance_id) {
        let _ = modules::process::close_pid(pid, 5);
        return Err("CODEX_START_CANCELLED".to_string());
    }
    modules::logger::log_info(&format!(
        "[Codex Start] instance launch phase finished: instance_id={}, pid={}, elapsed_ms={}, total_ms={}",
        instance.id,
        pid,
        launch_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    let finalize_started = Instant::now();
    let updated = modules::codex_instance::update_instance_after_start(&instance.id, pid)?;
    let running = modules::process::is_pid_running(pid);
    if running {
        if let (Some(settings), Some(port)) = (resume_settings, injection_plan.port) {
            modules::codex_app_injection::schedule_desktop_resume(
                instance_id.clone(), pid, port, settings,
            );
        }
    }
    let initialized = is_profile_initialized(&updated.user_data_dir);
    modules::logger::log_info(&format!(
        "[Codex Start] instance finalize phase finished: instance_id={}, elapsed_ms={}, total_ms={}",
        instance.id,
        finalize_started.elapsed().as_millis(),
        flow_started.elapsed().as_millis()
    ));
    emit_codex_instance_launch_step(
        &app,
        emit_launch_progress,
        &launch_target,
        "startClient",
        "completed",
        96,
        serde_json::json!({ "pid": pid }),
    );
    emit_codex_instance_launch_progress(
        &app,
        emit_launch_progress,
        &launch_target,
        serde_json::json!({ "type": "complete", "progress": 100 }),
    );
    Ok(CodexInstanceProfileView::from_profile(
        updated,
        running,
        initialized,
    ))
}

fn should_resume_cli_after_start(
    runtime_state: StartRuntimeState,
    expected_binding: Option<&str>,
    skip_failed_step: Option<&str>,
) -> bool {
    (matches!(runtime_state, StartRuntimeState::Stopped) || expected_binding.is_some())
        && !matches!(skip_failed_step, Some("prepareCredentials" | "writeProfile"))
}

async fn resume_cli_after_start(
    requested: bool,
    instance_id: &str,
    profile: &Path,
) -> Result<bool, String> {
    if !requested {
        return Ok(false);
    }
    ensure_codex_instance_start_not_cancelled(instance_id)?;
    cli_resume::start_after_switch(instance_id, profile).await
}

fn emit_cli_start_result(
    app: &AppHandle,
    emit_launch_progress: bool,
    target: &CodexInstanceStartTarget,
    resume: Result<bool, String>,
) -> Result<(), String> {
    let (status, details) = match resume {
        Ok(started) => (
            if started { "completed" } else { "skipped" },
            serde_json::json!({ "launchMode": "cli", "cliResumeStarted": started }),
        ),
        Err(error) if error == "CODEX_START_CANCELLED" => return Err(error),
        Err(error) => {
            // The switch is already committed. Only the added continuation failed.
            modules::logger::log_warn(&format!(
                "[Codex CLI Resume] Background command did not start: instance={}, error={error}",
                target.instance_id
            ));
            (
                "warning",
                serde_json::json!({ "launchMode": "cli", "cliResumeError": error }),
            )
        }
    };
    emit_codex_instance_launch_step(
        app, emit_launch_progress, target, "startClient", status, 96, details,
    );
    Ok(())
}

/// 启动已经由账号切换或 API 服务激活流程准备好 profile 的默认实例。
///
/// 本方法调用 `codex_start_instance_internal` 复用多开实例的启动事务；调用方必须在整个
/// “凭据写入 + 默认实例启动”期间持有默认 profile 写入租约。`launch_operation` 仅用于标识
/// 启动来源并关联前端进度状态，不改变 Token Authority 和 profile 落盘规则。
/// 调用前必须成功停止目标运行态；启动事务复用该结果，不再重复关闭。
pub(crate) async fn codex_start_default_with_prepared_profile(
    app: AppHandle,
    emit_launch_progress: bool,
    launch_operation: Option<&str>,
    skip_failed_step: Option<&str>,
    expected_prepared_binding: Option<&str>,
    previous_desktop_thread: Option<uuid::Uuid>,
) -> Result<CodexInstanceProfileView, String> {
    let mut launch_target = resolve_codex_instance_start_target(DEFAULT_INSTANCE_ID)?;
    launch_target.launch_operation = launch_operation.map(str::to_owned);
    let result = codex_start_instance_internal(
        app.clone(),
        DEFAULT_INSTANCE_ID.to_string(),
        true,
        false,
        skip_failed_step,
        emit_launch_progress,
        launch_operation,
        expected_prepared_binding,
        StartRuntimeState::Stopped,
        previous_desktop_thread,
    )
    .await;
    let result = match result {
        Ok(profile) => Ok(profile),
        Err(error) => {
            let auth_account_id = if launch_target
                .bind_account_id
                .as_deref()
                .is_some_and(modules::codex_instance::is_api_service_bind_account_id)
            {
                modules::codex_local_access::bound_oauth_account_id_for_instance_start()
                    .await
                    .ok()
                    .flatten()
            } else {
                modules::codex_account::oauth_account_id_for_runtime_binding(
                    launch_target.bind_account_id.as_deref(),
                )
                .or_else(|| {
                    modules::codex_account::oauth_account_id_for_runtime_dir(
                        &launch_target.user_data_dir,
                    )
                })
            };
            Err(auth_account_id
                .as_deref()
                .map(|account_id| {
                    modules::codex_account::format_account_switch_error(account_id, error.clone())
                })
                .unwrap_or(error))
        }
    };
    if let Err(error) = &result {
        let cancelled = error == "CODEX_START_CANCELLED";
        emit_codex_instance_launch_progress(
            &app,
            emit_launch_progress,
            &launch_target,
            serde_json::json!({
                "type": if cancelled { "cancelled" } else { "error" },
                "error": error,
                "cancelled": cancelled,
                "canRetry": !cancelled && !error.starts_with("CODEX_SWITCH_AUTH_REQUIRED:"),
            }),
        );
    }
    clear_codex_instance_start_cancel(DEFAULT_INSTANCE_ID);
    result
}

/// 启动已经由 API Service 激活流程准备好 profile 的非默认实例。
/// 调用方必须在整个“凭据写入 + 实例启动”期间持有目标 profile 写入租约。
/// 调用前必须成功停止目标运行态。
pub(crate) async fn codex_start_instance_with_prepared_profile(
    app: AppHandle,
    instance_id: String,
    emit_launch_progress: bool,
    launch_operation: Option<&str>,
    skip_failed_step: Option<&str>,
) -> Result<CodexInstanceProfileView, String> {
    let launch_target = resolve_codex_instance_start_target(&instance_id)?;
    let result = codex_start_instance_internal(
        app.clone(),
        instance_id.clone(),
        false,
        false,
        skip_failed_step,
        emit_launch_progress,
        launch_operation,
        launch_target.bind_account_id.as_deref(),
        StartRuntimeState::Stopped,
        None,
    )
    .await;
    let result = match result {
        Ok(profile) => Ok(profile),
        Err(error) => {
            let auth_account_id = if launch_target
                .bind_account_id
                .as_deref()
                .is_some_and(modules::codex_instance::is_api_service_bind_account_id)
            {
                modules::codex_local_access::bound_oauth_account_id_for_instance_start()
                    .await
                    .ok()
                    .flatten()
            } else {
                modules::codex_account::oauth_account_id_for_runtime_binding(
                    launch_target.bind_account_id.as_deref(),
                )
                .or_else(|| {
                    modules::codex_account::oauth_account_id_for_runtime_dir(
                        &launch_target.user_data_dir,
                    )
                })
            };
            Err(auth_account_id
                .as_deref()
                .map(|account_id| {
                    modules::codex_account::format_account_switch_error(account_id, error.clone())
                })
                .unwrap_or(error))
        }
    };
    if let Err(error) = &result {
        let cancelled = error == "CODEX_START_CANCELLED";
        emit_codex_instance_launch_progress(
            &app,
            emit_launch_progress,
            &launch_target,
            serde_json::json!({
                "type": if cancelled { "cancelled" } else { "error" },
                "error": error,
                "cancelled": cancelled,
                "canRetry": !cancelled && !error.starts_with("CODEX_SWITCH_AUTH_REQUIRED:"),
            }),
        );
    }
    clear_codex_instance_start_cancel(&instance_id);
    result
}

#[tauri::command]
pub async fn codex_start_instance(
    app: AppHandle,
    instance_id: String,
    transfer_conflicting_account: Option<bool>,
    skip_failed_step: Option<String>,
    expected_switch_binding: Option<String>,
) -> Result<CodexInstanceProfileView, String> {
    let mut launch_target = resolve_codex_instance_start_target(&instance_id)?;
    launch_target.expected_switch_binding = expected_switch_binding;
    let _profile_lease = modules::codex_account::try_acquire_profile_mutation_lease(
        &launch_target.user_data_dir,
        "instance-start",
    )?;
    let result = codex_start_instance_internal(
        app.clone(),
        instance_id,
        false,
        transfer_conflicting_account.unwrap_or(false),
        skip_failed_step.as_deref(),
        true,
        None,
        launch_target.expected_switch_binding.as_deref(),
        StartRuntimeState::NeedsStop,
        None,
    )
    .await;
    if let Err(error) = &result {
        let auth_account_id = if launch_target
            .bind_account_id
            .as_deref()
            .is_some_and(modules::codex_instance::is_api_service_bind_account_id)
        {
            modules::codex_local_access::bound_oauth_account_id_for_instance_start()
                .await
                .ok()
                .flatten()
        } else {
            modules::codex_account::oauth_account_id_for_runtime_binding(
                launch_target.bind_account_id.as_deref(),
            )
        }
        .or_else(|| launch_target.bind_account_id.clone());
        let error_for_ui = auth_account_id
            .as_deref()
            .map(|account_id| {
                modules::codex_account::format_account_switch_error(account_id, error.clone())
            })
            .unwrap_or_else(|| error.clone());
        let cancelled = error == "CODEX_START_CANCELLED";
        emit_codex_instance_launch_progress(
            &app,
            true,
            &launch_target,
            serde_json::json!({
                "type": if cancelled { "cancelled" } else { "error" },
                "error": error_for_ui,
                "cancelled": cancelled,
                "canRetry": !cancelled,
                "oauthRuntimePolicy": "latest-runtime-wins",
            }),
        );
    }
    clear_codex_instance_start_cancel(&launch_target.instance_id);
    result
}

/// 请求停止指定实例的启动事务。取消不会关闭弹框，由前端收到 cancelled 事件后决定是否关闭。
#[tauri::command]
pub async fn codex_cancel_instance_start(
    app: AppHandle,
    instance_id: String,
) -> Result<(), String> {
    let target = resolve_codex_instance_start_target(&instance_id)?;
    request_codex_instance_start_cancel(&instance_id);
    emit_codex_instance_launch_progress(
        &app,
        true,
        &target,
        serde_json::json!({
            "type": "cancelled",
            "progress": 0,
            "error": "CODEX_START_CANCELLED",
            "cancelled": true,
        }),
    );
    Ok(())
}

#[tauri::command]
pub async fn codex_stop_instance(instance_id: String) -> Result<CodexInstanceProfileView, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let default_dir = modules::codex_instance::get_default_codex_home()?;
        let default_settings = modules::codex_instance::load_default_settings()?;
        let selected = modules::codex_app_injection::capture_before_desktop_stop(&instance_id).await;
        modules::codex_app_injection::stop_for_profile(&default_dir);
        if launch_mode_uses_desktop_runtime(&default_settings.launch_mode) {
            modules::process::close_codex_default(20)?;
        }
        modules::codex_app_injection::record_after_desktop_stop(&instance_id, selected).await;
        modules::codex_local_access::stop_provider_gateways_for_profile(&default_dir).await;
        let updated = modules::codex_instance::update_default_pid(None)?;
        let default_bind_account_id = resolve_default_account_id(&updated);
        sync_codex_threads_across_idle_instances("after-stop-default").await;
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

    // A missing desktop path must not turn a failed process lookup into proof
    // that an instance is stopped before its credentials are changed.
    if launch_mode_uses_desktop_runtime(&instance.launch_mode) {
        tauri::async_runtime::spawn_blocking(modules::process::ensure_codex_runtime_path_or_idle)
            .await
            .map_err(|error| error.to_string())??;
    }
    let selected = modules::codex_app_injection::capture_before_desktop_stop(&instance_id).await;
    modules::codex_app_injection::stop_for_profile(Path::new(&instance.user_data_dir));
    let target_home = instance.user_data_dir.clone();
    let last_pid = instance.last_pid;
    tauri::async_runtime::spawn_blocking(move || {
        if modules::process::resolve_codex_pid(last_pid, Some(&target_home)).is_some() {
            modules::process::close_codex_instances(&[target_home], 20)
        } else {
            Ok(())
        }
    })
    .await
    .map_err(|error| error.to_string())??;
    modules::codex_app_injection::record_after_desktop_stop(&instance_id, selected).await;
    modules::codex_local_access::stop_provider_gateways_for_profile(Path::new(
        &instance.user_data_dir,
    ))
    .await;
    let updated = modules::codex_instance::update_instance_pid(&instance.id, None)?;
    let initialized = is_profile_initialized(&updated.user_data_dir);
    sync_codex_threads_across_idle_instances("after-stop-instance").await;
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
    let instance_ids = std::iter::once(DEFAULT_INSTANCE_ID)
        .chain(store.instances.iter().map(|instance| instance.id.as_str()));
    let selections = futures_util::future::join_all(instance_ids.map(|id| async move {
        (id, modules::codex_app_injection::capture_before_desktop_stop(id).await)
    })).await;
    modules::codex_app_injection::stop_for_profile(&default_home);
    let mut target_homes: Vec<String> = Vec::new();
    if launch_mode_uses_desktop_runtime(&store.default_settings.launch_mode) {
        target_homes.push(default_home.to_string_lossy().to_string());
    }
    for instance in &store.instances {
        let home = instance.user_data_dir.trim();
        if !home.is_empty() {
            modules::codex_app_injection::stop_for_profile(Path::new(home));
            if launch_mode_uses_desktop_runtime(&instance.launch_mode) {
                target_homes.push(home.to_string());
            }
        }
    }

    if !target_homes.is_empty() {
        modules::process::close_codex_instances(&target_homes, 20)?;
    }
    for (id, selection) in selections {
        modules::codex_app_injection::record_after_desktop_stop(id, selection).await;
    }
    modules::codex_local_access::stop_provider_gateways_for_profile(&default_home).await;
    for instance in &store.instances {
        let home = instance.user_data_dir.trim();
        if !home.is_empty() {
            modules::codex_local_access::stop_provider_gateways_for_profile(Path::new(home)).await;
        }
    }
    let _ = modules::codex_instance::clear_all_pids();
    sync_codex_threads_across_idle_instances("after-close-all").await;
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
pub async fn codex_focus_runtime_owner(
    pid: u32,
    user_data_dir: String,
    is_default: bool,
) -> Result<(), String> {
    modules::process::focus_codex_instance(
        Some(pid),
        if is_default {
            None
        } else {
            Some(user_data_dir.as_str())
        },
    )
    .map(|_| ())
    .map_err(|error| format!("定位 Codex 运行实例失败: {}", error))
}

#[tauri::command]
pub async fn codex_preview_instance_launch_command(
    user_data_dir: String,
    working_dir: Option<String>,
    extra_args: Option<String>,
    terminal: Option<String>,
    launch_command: Option<String>,
) -> Result<CodexInstanceLaunchPreviewInfo, String> {
    let user_data_dir = user_data_dir.trim().to_string();
    if user_data_dir.is_empty() {
        return Err("Codex CLI 实例目录不能为空".to_string());
    }
    let context = CodexLaunchContext {
        user_data_dir: user_data_dir.clone(),
        working_dir: working_dir.filter(|value| !value.trim().is_empty()),
        extra_args: extra_args.unwrap_or_default(),
    };
    let launch_command = launch_command
        .filter(|value| !value.trim().is_empty())
        .map(Ok)
        .unwrap_or_else(|| build_launch_command_preview(&context))?;
    let terminal = terminal
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "system".to_string());
    let terminal_plan = build_codex_terminal_launch_plan(&launch_command, &terminal)?;
    Ok(CodexInstanceLaunchPreviewInfo {
        user_data_dir,
        launch_command,
        terminal_command: terminal_plan.display_command,
        terminal,
    })
}

#[tauri::command]
pub async fn codex_get_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
    custom_command: Option<String>,
) -> Result<CodexInstanceLaunchInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let context = resolve_instance_launch_context(&instance_id)?;
        let launch_command = build_launch_command(&context, custom_command.as_deref(), false)?;
        let terminal = resolve_codex_launch_terminal(terminal);
        let terminal_plan = build_codex_terminal_launch_plan(&launch_command, &terminal)?;
        Ok(CodexInstanceLaunchInfo {
            instance_id,
            user_data_dir: context.user_data_dir,
            launch_command,
            terminal_command: terminal_plan.display_command,
            terminal,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

fn cli_resume_terminal_command(executable: &str, instance_id: &str, data_dir: &str, codex_home: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        let quote = |value: &str| powershell_quote(&value.replace('‘', "‘‘").replace('’', "’’"));
        format!(
            "& {} {} {} {} {}",
            quote(executable), cli_resume::RUN_ARG, quote(instance_id), quote(data_dir), quote(codex_home),
        )
    }

    #[cfg(not(target_os = "windows"))]
    {
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\"'\"'"));
        #[cfg(target_os = "macos")]
        let prefix = terminal::macos_loopback_bypass_prefix();
        #[cfg(not(target_os = "macos"))]
        let prefix = "";
        format!(
            "{prefix}{} {} {} {} {}",
            quote(executable), cli_resume::RUN_ARG, quote(instance_id), quote(data_dir), quote(codex_home),
        )
    }
}

#[tauri::command]
pub async fn codex_execute_instance_launch_command(
    instance_id: String,
    terminal: Option<String>,
    custom_command: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let context = resolve_instance_launch_context(&instance_id)?;
        let command = if custom_command.as_ref().is_none_or(|value| value.trim().is_empty())
            && modules::codex_session_resume::get(&instance_id)?.enabled
        {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let data_dir = std::path::absolute(modules::config::get_data_dir()?)
                .map_err(|error| error.to_string())?;
            let codex_home = std::path::absolute(&context.user_data_dir).map_err(|error| error.to_string())?;
            cli_resume_terminal_command(
                &executable.to_string_lossy(), &instance_id, &data_dir.to_string_lossy(), &codex_home.to_string_lossy(),
            )
        } else {
            build_launch_command(&context, custom_command.as_deref(), false)?
        };
        let terminal = resolve_codex_launch_terminal(terminal);
        terminal::execute_codex_terminal_command(&command, &terminal)
    })
    .await
    .map_err(|error| error.to_string())?
}
