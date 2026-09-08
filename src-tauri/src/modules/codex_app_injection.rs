//! Codex/ChatGPT renderer 的 Cockpit Tools API 服务可选额度显示注入。
//!
//! 该模块只连接实例自己的 loopback CDP 端口，不修改官方 app.asar，
//! 也不修改官方额度或速度逻辑。额度以独立的小字段显示在 composer 操作栏下方。

use crate::modules::{
    app_lifecycle, codex_account, codex_local_access, codex_quota, config, i18n, logger,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{IpAddr, TcpListener};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, UNIX_EPOCH};
#[cfg(not(target_os = "macos"))]
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use toml_edit::Document;

const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const INJECTION_INTERVAL: Duration = Duration::from_secs(2);
const QUOTA_REFRESH_INTERVAL: Duration = Duration::from_secs(15);
const AUTH_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(5);
const AUTH_IDENTITY_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const AUTH_NETWORK_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(30);
const AUTH_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(1);
const AUTH_NETWORK_CAPTURE_WINDOW: Duration = Duration::from_secs(4);
const AUTH_NETWORK_BODY_PREVIEW_LIMIT: usize = 4096;

#[derive(Debug, Clone)]
pub struct CodexAppInjectionLaunch {
    pub args: Vec<String>,
    pub port: Option<u16>,
}

struct InjectionRuntime {
    task: tauri::async_runtime::JoinHandle<()>,
}

struct AuthDiagnosticRuntime {
    task: tauri::async_runtime::JoinHandle<()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppServerDiagnosticObservation {
    pids: Vec<u32>,
    sockets: String,
    stdio: String,
    auth_file: String,
}

fn runtimes() -> &'static Mutex<HashMap<String, InjectionRuntime>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<String, InjectionRuntime>>> = OnceLock::new();
    RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn auth_diagnostic_runtimes() -> &'static Mutex<HashMap<String, AuthDiagnosticRuntime>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<String, AuthDiagnosticRuntime>>> = OnceLock::new();
    RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn quota_refresh_lock() -> &'static TokioMutex<()> {
    static LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| TokioMutex::new(()))
}

fn new_document_scripts() -> &'static Mutex<HashSet<String>> {
    static INSTALLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INSTALLED.get_or_init(|| Mutex::new(HashSet::new()))
}

fn should_install_new_document_script(websocket_url: &str) -> bool {
    let Ok(installed) = new_document_scripts().lock() else {
        return true;
    };
    !installed.contains(websocket_url)
}

fn mark_new_document_script_installed(websocket_url: &str) {
    if let Ok(mut installed) = new_document_scripts().lock() {
        installed.insert(websocket_url.to_string());
    }
}

fn profile_key(profile_dir: &Path) -> String {
    fs::canonicalize(profile_dir)
        .unwrap_or_else(|_| profile_dir.to_path_buf())
        .to_string_lossy()
        .trim()
        .to_ascii_lowercase()
}

fn reserve_cdp_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("分配 Codex CDP 端口失败: {}", error))?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("读取 Codex CDP 端口失败: {}", error))
}

fn is_debug_arg(value: &str, name: &str) -> bool {
    value == name || value.starts_with(&format!("{}=", name))
}

pub fn build_launch_args(
    existing: &[String],
    enabled: bool,
) -> Result<CodexAppInjectionLaunch, String> {
    if !enabled {
        return Ok(CodexAppInjectionLaunch {
            args: existing.to_vec(),
            port: None,
        });
    }

    let mut args = Vec::with_capacity(existing.len() + 2);
    let mut skip_next = false;
    for value in existing {
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_debug_arg(value, "--remote-debugging-port")
            || is_debug_arg(value, "--remote-debugging-address")
        {
            if value == "--remote-debugging-port" || value == "--remote-debugging-address" {
                skip_next = true;
            }
            continue;
        }
        args.push(value.clone());
    }

    let port = reserve_cdp_port()?;
    args.push("--remote-debugging-address=127.0.0.1".to_string());
    args.push(format!("--remote-debugging-port={}", port));
    Ok(CodexAppInjectionLaunch {
        args,
        port: Some(port),
    })
}

pub fn enabled_for_app() -> bool {
    config::get_user_config().codex_app_ui_injection_enabled
}

/// 实例级 CDP 只读观察始终开启；它记录官方客户端实际是否进入登录页，
/// 不注入脚本、不拦截事件，也不改变官方认证状态机。
fn auth_observation_enabled(bind_account_id: Option<&str>) -> bool {
    observed_oauth_account_id(bind_account_id).is_some()
}

pub fn supports_bind_account(bind_account_id: Option<&str>) -> bool {
    bind_account_id.is_some_and(crate::modules::codex_instance::is_api_service_bind_account_id)
}

pub fn bind_uses_deepseek_cdp_injection(bind_account_id: Option<&str>) -> bool {
    let Some(bind) = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if crate::modules::codex_instance::is_api_service_bind_account_id(bind) {
        return false;
    }
    let account_id = crate::modules::codex_instance::parse_provider_gateway_bind_account_id(bind)
        .unwrap_or_else(|| bind.to_string());
    crate::modules::codex_account::load_account(&account_id).is_some_and(|account| {
        crate::modules::codex_account::account_uses_deepseek_cdp_injection(&account)
    })
}

pub fn should_enable_injection(bind_account_id: Option<&str>) -> bool {
    (enabled_for_app() && supports_bind_account(bind_account_id))
        || bind_uses_deepseek_cdp_injection(bind_account_id)
}

/// 额度注入、认证页面观察和 DeepSeek 模型适配都依赖实例自己的 loopback CDP。
pub fn should_enable_cdp(bind_account_id: Option<&str>) -> bool {
    auth_observation_enabled(bind_account_id) || should_enable_injection(bind_account_id)
}

fn observed_oauth_account_id(bind_account_id: Option<&str>) -> Option<String> {
    let bind = bind_account_id?.trim();
    if bind.is_empty() || crate::modules::codex_instance::is_api_service_bind_account_id(bind) {
        return None;
    }
    let account_id = crate::modules::codex_instance::parse_provider_gateway_bind_account_id(bind)
        .unwrap_or_else(|| bind.to_string());
    let account = codex_account::load_account(&account_id)?;
    if account.is_api_key_auth() {
        account.bound_oauth_account_id
    } else if account.is_agent_identity_auth() || account.is_web_session_auth() {
        None
    } else {
        Some(account.id)
    }
}

/// 当前 profile 的官方 OAuth 身份核对结果。
///
/// `Matched` 才允许把 CDP 页面状态写回本地账号；`Mismatched` 和 `Unknown`
/// 只用于诊断，避免 profile 串号或落盘尚未完成时污染账号状态。
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProfileOAuthIdentityObservation {
    Matched {
        account_id: String,
        observed: crate::modules::codex_account::CodexOfficialOAuthIdentity,
    },
    Mismatched {
        account_id: String,
        observed: crate::modules::codex_account::CodexOfficialOAuthIdentity,
    },
    Unknown {
        account_id: Option<String>,
        reason: &'static str,
    },
}

/// 在阻塞线程读取官方 auth.json/Keychain，避免认证存储访问卡住 Tokio 任务。
async fn observe_profile_oauth_identity(
    profile_dir: PathBuf,
    bind_account_id: Option<String>,
) -> ProfileOAuthIdentityObservation {
    let fallback_account_id = bind_account_id.as_deref().and_then(|bind| {
        if bind.is_empty() || crate::modules::codex_instance::is_api_service_bind_account_id(bind) {
            return None;
        }
        Some(
            crate::modules::codex_instance::parse_provider_gateway_bind_account_id(bind)
                .unwrap_or_else(|| bind.to_string()),
        )
    });
    let result = timeout(
        CDP_CONNECT_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            let Some(account_id) = observed_oauth_account_id(bind_account_id.as_deref()) else {
                return ProfileOAuthIdentityObservation::Unknown {
                    account_id: None,
                    reason: "no_oauth_binding",
                };
            };
            let Some(account) = codex_account::load_account(&account_id) else {
                return ProfileOAuthIdentityObservation::Unknown {
                    account_id: Some(account_id),
                    reason: "local_account_missing",
                };
            };
            let Some(identity) = codex_account::read_official_oauth_identity(&profile_dir) else {
                return ProfileOAuthIdentityObservation::Unknown {
                    account_id: Some(account_id),
                    reason: "official_identity_unavailable",
                };
            };
            match codex_account::compare_official_oauth_identity(&identity, &account) {
                codex_account::CodexOfficialOAuthIdentityMatch::Matched => {
                    ProfileOAuthIdentityObservation::Matched {
                        account_id,
                        observed: identity,
                    }
                }
                codex_account::CodexOfficialOAuthIdentityMatch::Mismatched => {
                    ProfileOAuthIdentityObservation::Mismatched {
                        account_id,
                        observed: identity,
                    }
                }
                codex_account::CodexOfficialOAuthIdentityMatch::Unknown => {
                    ProfileOAuthIdentityObservation::Unknown {
                        account_id: Some(account_id),
                        reason: "official_identity_incomplete",
                    }
                }
            }
        }),
    )
    .await;

    match result {
        Ok(Ok(observation)) => observation,
        _ => ProfileOAuthIdentityObservation::Unknown {
            account_id: fallback_account_id,
            reason: "official_identity_read_timeout",
        },
    }
}

fn log_profile_oauth_identity_observation(
    instance_id: &str,
    profile_key: &str,
    observation: &ProfileOAuthIdentityObservation,
) {
    match observation {
        ProfileOAuthIdentityObservation::Matched {
            account_id,
            observed,
        } => logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth Identity] matched: instance_id={}, profile={}, account_id={}, observed_account_id={}, observed_user_id={}, observed_email={}, observed_organization_id={}",
            instance_id,
            profile_key,
            account_id,
            observed.account_id.as_deref().unwrap_or(""),
            observed.user_id.as_deref().unwrap_or(""),
            observed.email,
            observed.organization_id.as_deref().unwrap_or(""),
        )),
        ProfileOAuthIdentityObservation::Mismatched {
            account_id,
            observed,
        } => logger::log_warn(&format!(
            "[Codex Auth Identity] profile identity mismatched: instance_id={}, profile={}, expected_account_id={}, observed_account_id={}, observed_user_id={}, observed_email={}, observed_organization_id={}",
            instance_id,
            profile_key,
            account_id,
            observed.account_id.as_deref().unwrap_or(""),
            observed.user_id.as_deref().unwrap_or(""),
            observed.email,
            observed.organization_id.as_deref().unwrap_or(""),
        )),
        ProfileOAuthIdentityObservation::Unknown { account_id, reason } => {
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth Identity] unknown: instance_id={}, profile={}, expected_account_id={}, reason={}",
                instance_id,
                profile_key,
                account_id.as_deref().unwrap_or(""),
                reason,
            ));
        }
    }
}

fn bind_account_id_value(bind_account_id: Option<&str>) -> Option<String> {
    let bind = bind_account_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    if crate::modules::codex_instance::is_api_service_bind_account_id(bind) {
        return None;
    }
    Some(
        crate::modules::codex_instance::parse_provider_gateway_bind_account_id(bind)
            .unwrap_or_else(|| bind.to_string()),
    )
}

fn remote_debugging_port_from_command_line(command_line: &str) -> Option<u16> {
    let mut tokens = command_line.split_whitespace();
    while let Some(token) = tokens.next() {
        let token = token.trim_matches(['"', '\'']);
        if token == "--remote-debugging-port" {
            return tokens
                .next()
                .map(|value| value.trim_matches(['"', '\'']))
                .and_then(|value| value.parse::<u16>().ok())
                .filter(|port| *port > 0);
        }
        if let Some(value) = token.strip_prefix("--remote-debugging-port=") {
            return value
                .trim_matches(['"', '\''])
                .parse::<u16>()
                .ok()
                .filter(|port| *port > 0);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn remote_debugging_port_for_pid(pid: u32) -> Option<u16> {
    let output = Command::new("ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    remote_debugging_port_from_command_line(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn remote_debugging_port_for_pid(pid: u32) -> Option<u16> {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let command_line = system
        .process(pid)?
        .cmd()
        .iter()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    remote_debugging_port_from_command_line(&command_line)
}

pub fn restore_running_profiles(app: AppHandle) -> Result<usize, String> {
    let store = crate::modules::codex_instance::load_instance_store()?;
    let default_dir = crate::modules::codex_instance::get_default_codex_home()?;
    let process_entries = crate::modules::process::collect_codex_process_entries();
    let mut candidates = Vec::new();

    if store.default_settings.launch_mode == crate::models::InstanceLaunchMode::App
        && should_enable_cdp(store.default_settings.bind_account_id.as_deref())
    {
        if let Some(pid) = crate::modules::process::resolve_codex_pid_from_entries(
            store.default_settings.last_pid,
            None,
            &process_entries,
        ) {
            candidates.push((
                "__default__".to_string(),
                default_dir,
                pid,
                store.default_settings.bind_account_id.clone(),
            ));
        }
    }

    for instance in store.instances {
        if instance.launch_mode != crate::models::InstanceLaunchMode::App
            || !should_enable_cdp(instance.bind_account_id.as_deref())
        {
            continue;
        }
        let Some(pid) = crate::modules::process::resolve_codex_pid_from_entries(
            instance.last_pid,
            Some(&instance.user_data_dir),
            &process_entries,
        ) else {
            continue;
        };
        candidates.push((
            instance.id,
            PathBuf::from(instance.user_data_dir),
            pid,
            instance.bind_account_id,
        ));
    }

    let mut restored = 0;
    for (instance_id, profile_dir, pid, bind_account_id) in candidates {
        let Some(port) = remote_debugging_port_for_pid(pid) else {
            if should_enable_cdp(bind_account_id.as_deref()) {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] restore_skipped: instance_id={}, pid={}, reason=missing_remote_debugging_port",
                    instance_id, pid
                ));
            }
            continue;
        };
        let injection_enabled = should_enable_injection(bind_account_id.as_deref());
        start_for_profile(
            app.clone(),
            instance_id.clone(),
            profile_dir,
            Some(port),
            bind_account_id.clone(),
        );
        restored += 1;
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] restored_running_instance: instance_id={}, pid={}, port={}, injection_enabled={}, auth_observation_enabled={}",
            instance_id,
            pid,
            port,
            injection_enabled,
            auth_observation_enabled(bind_account_id.as_deref()),
        ));
    }

    Ok(restored)
}

pub fn stop_for_profile(profile_dir: &Path) {
    stop_auth_diagnostics_for_profile(profile_dir);
    stop_injection_for_profile(profile_dir);
}

fn stop_injection_for_profile(profile_dir: &Path) {
    let key = profile_key(profile_dir);
    if let Ok(mut items) = runtimes().lock() {
        if let Some(runtime) = items.remove(&key) {
            runtime.task.abort();
        }
    }
}

fn stop_auth_diagnostics_for_profile(profile_dir: &Path) {
    let key = profile_key(profile_dir);
    if let Ok(mut items) = auth_diagnostic_runtimes().lock() {
        if let Some(runtime) = items.remove(&key) {
            runtime.task.abort();
        }
    }
}

pub fn stop_all() {
    if let Ok(mut items) = runtimes().lock() {
        for (_, runtime) in items.drain() {
            runtime.task.abort();
        }
    }
    if let Ok(mut items) = auth_diagnostic_runtimes().lock() {
        for (_, runtime) in items.drain() {
            runtime.task.abort();
        }
    }
}

fn start_auth_diagnostics_for_profile(
    app: AppHandle,
    instance_id: &str,
    profile_dir: &Path,
    port: u16,
    bind_account_id: Option<&str>,
) {
    stop_auth_diagnostics_for_profile(profile_dir);
    let key = profile_key(profile_dir);
    let instance_id = instance_id.to_string();
    let profile_key_for_task = key.clone();
    let profile_dir_for_task = profile_dir.to_path_buf();
    let bind_account_id = bind_account_id.map(str::to_string);
    let task = tauri::async_runtime::spawn(async move {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth Diagnostic] started: instance_id={}, profile={}, port={}, bind_account_id={}",
            instance_id,
            profile_key_for_task,
            port,
            bind_account_id.as_deref().unwrap_or(""),
        ));
        run_auth_diagnostic_loop(
            app,
            instance_id,
            profile_key_for_task,
            profile_dir_for_task,
            port,
            bind_account_id,
        )
        .await;
    });
    if let Ok(mut items) = auth_diagnostic_runtimes().lock() {
        items.insert(key, AuthDiagnosticRuntime { task });
    }
}

pub fn start_for_profile(
    app: AppHandle,
    instance_id: String,
    profile_dir: PathBuf,
    port: Option<u16>,
    bind_account_id: Option<String>,
) {
    let Some(port) = port else { return };
    if auth_observation_enabled(bind_account_id.as_deref()) {
        start_auth_diagnostics_for_profile(
            app.clone(),
            &instance_id,
            &profile_dir,
            port,
            bind_account_id.as_deref(),
        );
    }
    if !should_enable_injection(bind_account_id.as_deref()) {
        return;
    }
    stop_injection_for_profile(&profile_dir);
    let key = profile_key(&profile_dir);
    let task_profile = profile_dir.clone();
    let task_bind = bind_account_id.clone();
    let task = tauri::async_runtime::spawn(async move {
        run_injection_loop(app, instance_id, task_profile, port, task_bind).await;
    });
    if let Ok(mut items) = runtimes().lock() {
        items.insert(key, InjectionRuntime { task });
    }
}

#[derive(Debug, Clone)]
struct ProfileGatewayConfig {
    base_url: String,
    api_key: String,
    provider_name: String,
}

fn read_profile_gateway_config(profile_dir: &Path) -> Option<ProfileGatewayConfig> {
    let config_text = fs::read_to_string(profile_dir.join("config.toml")).ok()?;
    let document = config_text.parse::<Document>().ok()?;
    let provider_id = document.get("model_provider")?.as_str()?.trim();
    let provider = document
        .get("model_providers")?
        .as_table()?
        .get(provider_id)?
        .as_table()?;
    let base_url = provider.get("base_url")?.as_str()?.trim().to_string();
    let api_key = provider
        .get("experimental_bearer_token")
        .and_then(|item| item.as_str())
        .or_else(|| provider.get("api_key").and_then(|item| item.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let auth = fs::read_to_string(profile_dir.join("auth.json")).ok()?;
            let value = serde_json::from_str::<Value>(&auth).ok()?;
            value
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })?;
    let provider_name = provider
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or(provider_id)
        .trim()
        .to_string();
    Some(ProfileGatewayConfig {
        base_url,
        api_key,
        provider_name,
    })
}

fn local_quota_url(base_url: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(base_url.trim()).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "localhost" && host != "127.0.0.1" && host != "::1" {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    let next_path = if path.ends_with("/v1") {
        format!("{}/cockpit/quota", path)
    } else {
        format!("{}/v1/cockpit/quota", path)
    };
    url.set_path(&next_path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct QuotaPlanSummary {
    plan: String,
    count: i64,
    weekly_remaining_percent: Option<i64>,
    five_hour_remaining_percent: Option<i64>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
struct QuotaResponse {
    weekly_remaining_percent: Option<i64>,
    five_hour_remaining_percent: Option<i64>,
    account_count: Option<i64>,
    available_account_count: Option<i64>,
    abnormal_account_count: Option<i64>,
    cooldown_account_count: Option<i64>,
    plans: Vec<QuotaPlanSummary>,
}

impl QuotaResponse {
    fn empty_pool() -> Self {
        Self {
            weekly_remaining_percent: Some(0),
            five_hour_remaining_percent: Some(0),
            account_count: Some(0),
            available_account_count: Some(0),
            abnormal_account_count: Some(0),
            cooldown_account_count: Some(0),
            plans: Vec::new(),
        }
    }

    fn normalize_empty_pool(self) -> Self {
        if self.account_count == Some(0) {
            Self::empty_pool()
        } else {
            self
        }
    }
}

async fn fetch_quota(
    client: &Client,
    gateway: Option<&ProfileGatewayConfig>,
) -> Option<QuotaResponse> {
    let gateway = gateway?;
    let url = local_quota_url(&gateway.base_url)?;
    let response = client
        .get(url)
        .bearer_auth(&gateway.api_key)
        .timeout(CDP_CONNECT_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    response
        .json::<QuotaResponse>()
        .await
        .ok()
        .map(QuotaResponse::normalize_empty_pool)
}

#[derive(Debug, Clone, Deserialize)]
struct CdpTarget {
    #[serde(rename = "id", default)]
    target_id: String,
    #[serde(rename = "type")]
    target_type: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AuthPageSnapshot {
    #[serde(default)]
    route: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    ready_state: String,
    #[serde(default)]
    physical_url: String,
    #[serde(default)]
    route_source: String,
    #[serde(default)]
    login_ui_signal: bool,
    #[serde(default)]
    login_ui_markers: Vec<String>,
}

impl AuthPageSnapshot {
    fn login_signal(&self) -> bool {
        // 官方桌面端使用 MemoryRouter，物理 pathname 通常保持 /index.html。
        // 只有完整 LoginRoute 组合特征才作为 /login 的等价证据；单独的标题或
        // 普通错误文字不参与状态判定。
        is_official_login_route(&self.route) || self.has_login_ui_signal()
    }

    fn has_login_ui_signal(&self) -> bool {
        self.login_ui_signal && has_login_route_markers(&self.login_ui_markers)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthDiagnosticObservation {
    cdp_available: bool,
    target_count: usize,
    route: String,
    route_source: String,
    title: String,
    ready_state: String,
    login_route: bool,
    login_ui_signal: bool,
    login_ui_markers: Vec<String>,
}

impl AuthDiagnosticObservation {
    fn unavailable() -> Self {
        Self {
            cdp_available: false,
            target_count: 0,
            route: String::new(),
            route_source: String::new(),
            title: String::new(),
            ready_state: String::new(),
            login_route: false,
            login_ui_signal: false,
            login_ui_markers: Vec::new(),
        }
    }

    fn login_signal(&self) -> bool {
        self.login_route || self.login_ui_signal
    }
}

fn is_official_login_route(route: &str) -> bool {
    route.trim_end_matches('/') == "/login"
}

fn has_login_route_markers(markers: &[String]) -> bool {
    markers.iter().any(|marker| marker == "login_title")
        && markers
            .iter()
            .any(|marker| marker == "login_primary_action")
}

const AUTH_DIAGNOSTIC_SCRIPT: &str = r#"
(() => {
  const href = String(location.href || "");
  const path = String(location.pathname || "");
  const hash = String(location.hash || "").split(/[?#]/, 1)[0];
  const normalizeRoute = (value) => {
    const normalized = String(value || "").replace(/^#/, "").split(/[?#]/, 1)[0].replace(/\/+$/, "");
    return normalized || "/";
  };
  const pathRoute = normalizeRoute(path);
  const hashRoute = normalizeRoute(hash);
  const route = (hash && hashRoute !== "/" ? hashRoute : pathRoute).slice(0, 160);
  const textOf = (selector) => Array.from(document.querySelectorAll(selector))
    .map((node) => String(node.textContent || "").replace(/\s+/g, " ").trim().slice(0, 160))
    .filter(Boolean);
  const headings = textOf("h1,h2").join(" ");
  const controls = textOf("button,a").join(" ");
  const loginUiMarkers = [];
  if (/sign in to chatgpt|登录 chatgpt/i.test(headings)) loginUiMarkers.push("login_title");
  if (/continue to sign in|继续登录|sign in with chatgpt|使用 chatgpt 登录/i.test(controls)) {
    loginUiMarkers.push("login_primary_action");
  }
  if (/sign in another way|使用其他方式登录|sign up|注册/i.test(controls)) {
    loginUiMarkers.push("login_secondary_action");
  }
  return {
    route,
    title: String(document.title || "").slice(0, 160),
    readyState: String(document.readyState || ""),
    physicalUrl: href.slice(0, 512),
    routeSource: hash && hashRoute !== "/" ? "hash" : "pathname",
    loginUiSignal: loginUiMarkers.length > 0,
    loginUiMarkers,
  };
})()
"#;

fn deepseek_model_injection_script(
    _locale: &str,
    selected_model: &str,
    handled_selected_model: Option<&str>,
) -> String {
    let selected = serde_json::to_string(selected_model)
        .unwrap_or_else(|_| "\"deepseek-v4-flash\"".to_string());
    let handled =
        serde_json::to_string(&handled_selected_model).unwrap_or_else(|_| "null".to_string());
    format!(
        r#"(() => {{
      const flashId = "deepseek-v4-flash";
      const proId = "deepseek-v4-pro";
      const selectedModel = {selected};
      const handledSelectedModel = {handled};
      const root = window.__cockpitCodexInjection || (window.__cockpitCodexInjection = {{}});
      root.hostHeartbeatAt = Date.now();
      root.hostAvailable = true;
      root.mode = "deepseek-official-picker";
      root.selectedModel = selectedModel;
      const staleBar = document.querySelector("[data-cockpit-deepseek-bar]");
      if (staleBar) staleBar.remove();
      if (handledSelectedModel && root.pendingSelectedModel === handledSelectedModel) {{
        root.pendingSelectedModel = null;
      }}
      const shellToUpstream = {{ "gpt-5.5": flashId, "gpt-5.4": proId, "gpt-5.6-sol": flashId, "gpt-5.6-terra": proId }};
      const displayName = {{
        [flashId]: "DeepSeek-V4-Flash",
        [proId]: "DeepSeek-V4-Pro",
      }};
      const reasoningLevels = ["low", "high", "max"];
      const reasoningDescriptors = () => reasoningLevels.map((effort) => ({{
        effort,
        reasoningEffort: effort,
        description: effort,
      }}));
      const applyReasoningMetadata = (item) => {{
        if (!item || typeof item !== "object") return;
        const levels = reasoningDescriptors();
        // The desktop app has used both camelCase and snake_case model metadata
        // across releases. Keep both shapes in sync so DeepSeek exposes its
        // actual low/high/max picker instead of inheriting Codex defaults.
        item.defaultReasoningEffort = "high";
        item.supportedReasoningEfforts = levels;
        item.default_reasoning_level = "high";
        item.supported_reasoning_levels = levels;
      }};
      const listMethods = {{ "model/list": true, "list-models-for-host": true }};
      const writeMethods = {{
        "thread/start": true,
        "turn/start": true,
        "thread/resume": true,
        "thread/compact/start": true,
        "set-default-model-config-for-host": true,
        "config/value/write": true,
      }};
      const normalize = (value) => String(value || "").trim().toLowerCase();
      const toUpstream = (value) => {{
        const slug = normalize(value);
        return shellToUpstream[slug] || ((slug === flashId || slug === proId) ? slug : null);
      }};
      const keepSlug = (value) => Boolean(toUpstream(value));
      const reportSelected = (value) => {{
        const upstream = toUpstream(value);
        if (!upstream || upstream === selectedModel || upstream === handledSelectedModel) return;
        root.pendingSelectedModel = upstream;
      }};
      const descriptor = (official) => {{
        const item = {{
          model: official,
          id: official,
          slug: official,
          name: displayName[official] || official,
          displayName: displayName[official] || official,
          display_name: displayName[official] || official,
          description: displayName[official] || official,
          hidden: false,
          visibility: "list",
          isDefault: official === selectedModel,
        }};
        applyReasoningMetadata(item);
        return item;
      }};
      const patchItem = (item) => {{
        if (!item || typeof item !== "object") return false;
        const official = toUpstream(item.model || item.slug || item.id);
        if (!official) return false;
        item.hidden = false;
        item.visibility = "list";
        const name = displayName[official];
        item.displayName = name;
        item.display_name = name;
        item.name = name;
        item.description = name;
        item.model = official;
        item.slug = official;
        item.id = official;
        applyReasoningMetadata(item);
        return true;
      }};
      const isModelArray = (value) => Array.isArray(value) && value.some((item) => item && typeof item === "object" && (typeof item.model === "string" || typeof item.slug === "string"));
      const patchModelArray = (value) => {{
        if (!isModelArray(value)) return false;
        for (let index = value.length - 1; index >= 0; index -= 1) {{
          const slug = value[index]?.model || value[index]?.slug || value[index]?.id;
          if (!keepSlug(slug)) {{
            value.splice(index, 1);
            continue;
          }}
          patchItem(value[index]);
        }}
        const have = new Set(value.map((item) => normalize(item.model || item.slug || item.id)));
        for (const official of [flashId, proId]) {{
          if (!have.has(official)) value.unshift(descriptor(official));
        }}
        return true;
      }};
      const patchContainer = (value, depth) => {{
        if (!value || typeof value !== "object" || depth > 5) return false;
        let changed = patchModelArray(value);
        if (patchModelArray(value.models)) changed = true;
        if (patchModelArray(value.data)) changed = true;
        if (value.result && patchContainer(value.result, depth + 1)) changed = true;
        if (value.message?.result && patchContainer(value.message.result, depth + 1)) changed = true;
        return changed;
      }};
      const rewriteOutgoing = (method, params) => {{
        if (!params || typeof params !== "object") return;
        if (method === "set-default-model-config-for-host" && params.model) {{
          reportSelected(params.model);
          const upstream = toUpstream(params.model);
          if (upstream) params.model = upstream;
        }}
        if (method === "config/value/write") {{
          const key = String(params.key || params.path || params.name || "");
          if (key.toLowerCase().includes("model") && params.value != null) {{
            reportSelected(String(params.value));
            const upstream = toUpstream(params.value);
            if (upstream) params.value = upstream;
          }}
        }}
        if (params.model) {{
          reportSelected(params.model);
          const upstream = toUpstream(params.model);
          if (upstream) params.model = upstream;
        }}
        if (params.params && typeof params.params === "object" && params.params.model) {{
          reportSelected(params.params.model);
          const upstream = toUpstream(params.params.model);
          if (upstream) params.params.model = upstream;
        }}
        if (params.request && typeof params.request === "object") rewriteOutgoing(params.request.method, params.request.params || params.request);
      }};
      const wrapResult = (method, result) => {{
        if (listMethods[method]) {{
          try {{ patchContainer(result, 0); }} catch {{}}
        }}
        return result;
      }};
      const wrapInvoke = (method, params, invoke) => {{
        if (writeMethods[method]) {{
          try {{ rewriteOutgoing(method, params); }} catch {{}}
        }}
        const result = invoke();
        if (!listMethods[method] || result == null) return result;
        if (typeof result.then === "function") return result.then((value) => wrapResult(method, value));
        return wrapResult(method, result);
      }};
      const patchStatsigConfig = (name, config) => {{
        if (String(name || "") !== "107580212" || !config?.value || typeof config.value !== "object") return config;
        const available = Array.isArray(config.value.available_models) ? [...config.value.available_models] : [];
        for (const slug of [flashId, proId]) if (!available.includes(slug)) available.push(slug);
        config.value = {{ ...config.value, available_models: available, use_hidden_models: false }};
        return config;
      }};
      const patchStatsig = () => {{
        const statsig = window.__STATSIG__ || globalThis.__STATSIG__;
        if (!statsig || typeof statsig !== "object") return;
        const clients = [statsig.firstInstance, typeof statsig.instance === "function" ? statsig.instance() : null]
          .concat(statsig.instances && typeof statsig.instances === "object" ? Object.values(statsig.instances) : [])
          .filter(Boolean);
        for (const client of clients) {{
          if (typeof client.getDynamicConfig !== "function" || client.__cockpitModelPatched) continue;
          const original = client.getDynamicConfig.bind(client);
          client.getDynamicConfig = (name, options) => patchStatsigConfig(name, original(name, options));
          client.__cockpitModelPatched = true;
        }}
      }};
      const wrapFunction = (original) => {{
        if (typeof original !== "function" || original.__cockpitOfficialPickerWrapped) return original;
        const wrapped = function(method, params, options) {{
          return wrapInvoke(String(method || ""), params, () => options == null ? original.call(this, method, params) : original.call(this, method, params, options));
        }};
        wrapped.__cockpitOfficialPickerWrapped = true;
        return wrapped;
      }};
      const patchSendRequest = (target) => {{
        if (!target || typeof target.sendRequest !== "function" || target.__cockpitOfficialPickerPatched) return false;
        target.sendRequest = wrapFunction(target.sendRequest.bind(target));
        target.__cockpitOfficialPickerPatched = true;
        return true;
      }};
      const wrapBridge = () => {{
        const bridge = window.electronBridge;
        if (!bridge || typeof bridge.sendMessageFromView !== "function" || bridge.__cockpitOfficialPickerPatched) return;
        const original = bridge.sendMessageFromView.bind(bridge);
        bridge.sendMessageFromView = function(message) {{
          try {{
            const method = message?.type || message?.method;
            if (method) rewriteOutgoing(String(method), message);
            if (message?.request) rewriteOutgoing(String(message.request.method || ""), message.request.params || message.request);
          }} catch {{}}
          return original(message);
        }};
        bridge.__cockpitOfficialPickerPatched = true;
      }};
      const installHooks = () => {{
        if (root.officialPickerInstalled) return;
        root.officialPickerInstalled = true;
        const originalParse = JSON.parse;
        JSON.parse = function(text, reviver) {{
          const value = originalParse.apply(this, arguments);
          try {{ patchContainer(value, 0); }} catch {{}}
          return value;
        }};
        const originalDefine = Object.defineProperty;
        Object.defineProperty = function(obj, prop, desc) {{
          if (desc && (prop === "sendRequest" || prop === "setMessageHandler") && typeof desc.value === "function") {{
            if (prop === "sendRequest") {{
              desc = Object.assign({{}}, desc, {{ value: wrapFunction(desc.value) }});
            }} else {{
              const originalSet = desc.value;
              desc = Object.assign({{}}, desc, {{
                value: function(handler) {{
                  return originalSet.call(this, typeof handler === "function" ? wrapFunction(handler) : handler);
                }},
              }});
            }}
          }}
          return originalDefine.call(this, obj, prop, desc);
        }};
      }};
      installHooks();
      wrapBridge();
      patchStatsig();
      patchSendRequest(root.appServerClient);
      const pendingSelectedModel = typeof root.pendingSelectedModel === "string"
        && (root.pendingSelectedModel === flashId || root.pendingSelectedModel === proId)
        && root.pendingSelectedModel !== selectedModel
        && root.pendingSelectedModel !== handledSelectedModel
        ? root.pendingSelectedModel
        : null;
      return {{ selectedModel: pendingSelectedModel }};
    }})()"#
    )
}

fn injection_script(
    provider_name: &str,
    quota: &QuotaResponse,
    locale: &str,
    refresh_in_progress: bool,
    handled_refresh_token: Option<&str>,
) -> String {
    let provider = serde_json::to_string(provider_name).unwrap_or_else(|_| "\"Codex\"".to_string());
    let weekly = quota.weekly_remaining_percent;
    let five_hour = quota.five_hour_remaining_percent;
    let account_count = quota.account_count;
    let available_account_count = quota.available_account_count.or(account_count);
    let abnormal_account_count = quota.abnormal_account_count.unwrap_or(0);
    let cooldown_account_count = quota.cooldown_account_count.unwrap_or(0);
    let plans = serde_json::to_string(&quota.plans).unwrap_or_else(|_| "[]".to_string());
    let weekly = serde_json::to_string(&weekly).unwrap_or_else(|_| "null".to_string());
    let five_hour = serde_json::to_string(&five_hour).unwrap_or_else(|_| "null".to_string());
    let account_count_value =
        serde_json::to_string(&account_count).unwrap_or_else(|_| "null".to_string());
    let available_account_count_value =
        serde_json::to_string(&available_account_count).unwrap_or_else(|_| "null".to_string());
    let abnormal_account_count_value =
        serde_json::to_string(&abnormal_account_count).unwrap_or_else(|_| "0".to_string());
    let cooldown_account_count_value =
        serde_json::to_string(&cooldown_account_count).unwrap_or_else(|_| "0".to_string());
    let account_pool_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionPoolLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"Accounts\"".to_string());
    let weekly_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionWeeklyLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"Weekly\"".to_string());
    let five_hour_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionFiveHourLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"5h\"".to_string());
    let account_pool_title = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.accountPoolHealth.title",
        &[],
    ))
    .unwrap_or_else(|_| "\"Account Pool\"".to_string());
    let quota_empty_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.quotaPool.empty",
        &[],
    ))
    .unwrap_or_else(|_| "\"No quota stats yet\"".to_string());
    let available_text = {
        let available = available_account_count.unwrap_or(0).to_string();
        let total = account_count.unwrap_or(0).to_string();
        serde_json::to_string(&i18n::translate(
            locale,
            "codex.localAccess.accountPoolHealth.availableRatio",
            &[("available", available.as_str()), ("total", total.as_str())],
        ))
        .unwrap_or_else(|_| "\"Available\"".to_string())
    };
    let issue_text = {
        let abnormal = abnormal_account_count.to_string();
        let cooldown = cooldown_account_count.to_string();
        serde_json::to_string(&i18n::translate(
            locale,
            "codex.localAccess.accountPoolHealth.issueSummary",
            &[
                ("abnormal", abnormal.as_str()),
                ("cooldown", cooldown.as_str()),
            ],
        ))
        .unwrap_or_else(|_| "\"Issues\"".to_string())
    };
    let refresh_label =
        serde_json::to_string(&i18n::translate(locale, "common.shared.refreshQuota", &[]))
            .unwrap_or_else(|_| "\"Refresh quota\"".to_string());
    let close_label = serde_json::to_string(&i18n::translate(locale, "common.close", &[]))
        .unwrap_or_else(|_| "\"Close\"".to_string());
    let refresh_in_progress = if refresh_in_progress { "true" } else { "false" };
    let handled_refresh_token =
        serde_json::to_string(&handled_refresh_token).unwrap_or_else(|_| "null".to_string());
    format!(
        r#"(() => {{
      const providerName = {provider};
      const weeklyPercent = {weekly};
      const fiveHourPercent = {five_hour};
      const accountCount = {account_count_value};
      const availableAccountCount = {available_account_count_value};
      const abnormalAccountCount = {abnormal_account_count_value};
      const cooldownAccountCount = {cooldown_account_count_value};
      const plans = {plans};
      const accountPoolLabel = {account_pool_label};
      const weeklyLabel = {weekly_label};
      const fiveHourLabel = {five_hour_label};
      const accountPoolTitle = {account_pool_title};
      const quotaEmptyLabel = {quota_empty_label};
      const availableText = {available_text};
      const issueText = {issue_text};
      const refreshLabel = {refresh_label};
      const closeLabel = {close_label};
      const refreshInProgress = {refresh_in_progress};
      const handledRefreshToken = {handled_refresh_token};
      const hostHeartbeatTimeoutMs = 8000;
      const root = window.__cockpitCodexInjection || (window.__cockpitCodexInjection = {{}});
      root.hostHeartbeatAt = Date.now();
      root.hostAvailable = true;
      root.providerName = providerName;
      root.weeklyPercent = weeklyPercent;
      root.fiveHourPercent = fiveHourPercent;
      const pendingRefreshToken = typeof root.refreshRequestToken === 'string' && root.refreshRequestToken !== handledRefreshToken
        ? root.refreshRequestToken
        : null;
      root.refreshing = refreshInProgress || Boolean(pendingRefreshToken);
      if (handledRefreshToken && root.refreshRequestToken === handledRefreshToken) root.refreshRequestToken = null;
      const render = () => {{
        let host = document.querySelector('[data-cockpit-quota-footer]');
        const permissions = document.querySelector('[data-composer-navigation-target="permissions"]');
        const footer = permissions?.closest('._footer_1qb5a_2') || permissions?.parentElement?.parentElement?.parentElement;
        if (!footer || !permissions) {{
          if (host) host.style.display = 'none';
          const details = document.querySelector('[data-cockpit-quota-details]');
          if (details) details.style.display = 'none';
          root.quotaDetailsOpen = false;
          if (root.layoutObserver) root.layoutObserver.disconnect();
          root.layoutFooter = null;
          root.layoutPermissions = null;
          return;
        }}
        if (!host) {{
          host = document.createElement('div');
          host.setAttribute('data-cockpit-quota-footer', 'true');
        }}
        if (host.parentElement !== document.body) document.body.appendChild(host);
        if ('ResizeObserver' in window) {{
          if (!root.layoutObserver) root.layoutObserver = new ResizeObserver(() => root.scheduleRender());
          if (root.layoutFooter !== footer || root.layoutPermissions !== permissions) {{
            root.layoutObserver.disconnect();
            root.layoutObserver.observe(footer);
            if (permissions !== footer) root.layoutObserver.observe(permissions);
            root.layoutFooter = footer;
            root.layoutPermissions = permissions;
          }}
        }}
        const footerRect = footer.getBoundingClientRect();
        const permissionsRect = permissions.getBoundingClientRect();
        host.style.cssText = 'position:fixed;transform:translate(-50%,-50%);z-index:2;display:flex;align-items:center;justify-content:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-size:12px;line-height:1;white-space:nowrap;pointer-events:none;';
        host.style.left = Math.round(footerRect.left + footerRect.width / 2) + 'px';
        host.style.top = Math.round(permissionsRect.top + permissionsRect.height / 2) + 'px';
        const badgeStyle = 'display:inline-flex;align-items:center;gap:6px;height:24px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.20));border-radius:999px;padding:0 9px;background:var(--color-token-main-surface-primary,rgba(127,127,127,.10));color:inherit;font:inherit;box-shadow:0 1px 2px rgba(0,0,0,.08);backdrop-filter:blur(8px);font-weight:500;cursor:pointer;pointer-events:auto;';
        const escapeHtml = (value) => String(value ?? '').replace(/[&<>\"']/g, (char) => ({{'&':'&amp;','<':'&lt;','>':'&gt;','\"':'&quot;',"'":'&#39;'}}[char]));
        const formatPercent = (value) => Number.isFinite(value) ? Math.round(value) + '%' : '—';
        const renderPlan = (plan) => {{
          const weekly = formatPercent(plan.weeklyRemainingPercent);
          const fiveHour = formatPercent(plan.fiveHourRemainingPercent);
          const planKey = String(plan.plan || '').toUpperCase();
          const planColor = planKey.includes('PLUS') ? '#8b5cf6' : (planKey.includes('TEAM') || planKey.includes('BUSINESS')) ? '#3b82f6' : planKey.includes('API_KEY') ? '#a3a3a3' : '#10b981';
          const metrics = [];
          if (Number.isFinite(plan.weeklyRemainingPercent)) metrics.push('<span style="display:inline-flex;align-items:center;gap:4px;"><i style="width:5px;height:5px;border-radius:999px;background:#10b981;"></i>' + escapeHtml(weeklyLabel) + ' ' + escapeHtml(weekly) + '</span>');
          if (Number.isFinite(plan.fiveHourRemainingPercent)) metrics.push('<span style="display:inline-flex;align-items:center;gap:4px;"><i style="width:5px;height:5px;border-radius:999px;background:#3b82f6;"></i>' + escapeHtml(fiveHourLabel) + ' ' + escapeHtml(fiveHour) + '</span>');
          const quotaHtml = metrics.length ? metrics.join('<span style="opacity:.35;">·</span>') : '<span style="opacity:.72;">' + escapeHtml(quotaEmptyLabel) + '</span>';
          return '<div style="display:flex;align-items:center;justify-content:space-between;gap:9px;padding:6px 0;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="display:inline-flex;align-items:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-weight:500;white-space:nowrap;"><i style="width:6px;height:6px;border-radius:999px;background:' + planColor + ';box-shadow:0 0 0 2px rgba(127,127,127,.10);"></i>' + escapeHtml(plan.plan) + ' <small style="font:inherit;opacity:.62;">' + Math.max(0, Math.round(plan.count || 0)) + '</small></span><span style="display:inline-flex;align-items:center;gap:5px;color:var(--color-token-text-secondary,#737373);text-align:right;white-space:nowrap;">' + quotaHtml + '</span></div>';
        }};
        const detailCardStyle = 'position:fixed;z-index:4;width:min(260px,calc(100vw - 24px));box-sizing:border-box;padding:9px 11px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.16));border-radius:10px;background:var(--color-token-main-surface-primary,#fff);color:var(--color-token-text-secondary,#737373);box-shadow:0 4px 14px rgba(0,0,0,.09);font-family:inherit;font-size:12px;line-height:1.3;letter-spacing:normal;pointer-events:auto;';
        const fields = [];
        if (Number.isFinite(accountCount) && accountCount >= 0) fields.push('<button type="button" data-cockpit-quota-open style="' + badgeStyle + '"><span style="width:6px;height:6px;border-radius:999px;background:#8b5cf6;box-shadow:0 0 0 2px rgba(139,92,246,.14)"></span>' + accountPoolLabel + ' ' + Math.round(accountCount) + '</button>');
        if (Number.isFinite(fiveHourPercent)) fields.push('<button type="button" data-cockpit-quota-open style="' + badgeStyle + '"><span style="width:6px;height:6px;border-radius:999px;background:#3b82f6;box-shadow:0 0 0 2px rgba(59,130,246,.14)"></span>' + fiveHourLabel + ' ' + Math.round(fiveHourPercent) + '%</button>');
        if (Number.isFinite(weeklyPercent)) fields.push('<button type="button" data-cockpit-quota-open style="' + badgeStyle + '"><span style="width:6px;height:6px;border-radius:999px;background:#10b981;box-shadow:0 0 0 2px rgba(16,185,129,.14)"></span>' + weeklyLabel + ' ' + Math.round(weeklyPercent) + '%</button>');
        if (fields.length) fields.push('<button type="button" data-cockpit-quota-refresh style="display:inline-flex;align-items:center;justify-content:center;width:24px;height:24px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.20));border-radius:999px;padding:0;background:var(--color-token-main-surface-primary,rgba(127,127,127,.10));color:inherit;box-shadow:0 1px 2px rgba(0,0,0,.08);backdrop-filter:blur(8px);cursor:pointer;pointer-events:auto;transition:color .15s ease,border-color .15s ease,background .15s ease,opacity .15s ease"><svg data-cockpit-quota-refresh-icon viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6v5h-5"></path><path d="M4 18v-5h5"></path><path d="M6.1 9a7 7 0 0 1 11.6-2.6L20 11"></path><path d="M4 13l2.3 4.6A7 7 0 0 0 17.9 15"></path></svg></button>');
        const nextHtml = fields.join('');
        if (host.innerHTML !== nextHtml) host.innerHTML = nextHtml;
        host.style.display = fields.length ? 'flex' : 'none';
        let details = document.querySelector('[data-cockpit-quota-details]');
        if (!details) {{
          details = document.createElement('div');
          details.setAttribute('data-cockpit-quota-details', 'true');
          document.body.appendChild(details);
        }}
        details.style.cssText = detailCardStyle;
        details.style.left = Math.round(footerRect.left + footerRect.width / 2) + 'px';
        details.style.top = Math.max(12, Math.round(permissionsRect.top - 2)) + 'px';
        details.style.transform = 'translate(-50%,-100%)';
        details.style.display = root.quotaDetailsOpen ? 'block' : 'none';
        if (root.quotaDetailsOpen) {{
          const planRows = plans.map(renderPlan).join('');
          const detailsHtml = '<div style="display:flex;align-items:center;justify-content:space-between;gap:10px;margin:0 0 4px;padding-bottom:6px;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:500;color:var(--color-token-text-secondary,#737373);"><i style="width:6px;height:6px;border-radius:999px;background:#8b5cf6;box-shadow:0 0 0 2px rgba(139,92,246,.12);"></i>' + escapeHtml(accountPoolTitle) + '</span><button type="button" data-cockpit-quota-close aria-label="' + escapeHtml(closeLabel) + '" title="' + escapeHtml(closeLabel) + '" style="display:inline-flex;align-items:center;justify-content:center;width:18px;height:18px;border:0;border-radius:4px;background:transparent;color:var(--color-token-text-secondary,#737373);font:inherit;font-size:14px;line-height:1;cursor:pointer;padding:0;opacity:.72;">×</button></div>' + '<div>' + (planRows || '<div style="padding:6px 0;color:var(--color-token-text-secondary,#737373);opacity:.72;">' + escapeHtml(quotaEmptyLabel) + '</div>') + '</div>' + '<div style="display:flex;justify-content:space-between;gap:10px;padding-top:7px;color:var(--color-token-text-secondary,#737373);font-size:11px;opacity:.78;"><span>' + escapeHtml(availableText) + '</span><span>' + escapeHtml(issueText) + '</span></div>';
          if (details.innerHTML !== detailsHtml) details.innerHTML = detailsHtml;
        }}
        host.querySelectorAll('[data-cockpit-quota-open]').forEach((button) => {{
          button.onclick = () => {{ root.quotaDetailsOpen = !root.quotaDetailsOpen; root.render(); }};
        }});
        const closeButton = details.querySelector('[data-cockpit-quota-close]');
        if (closeButton) closeButton.onclick = () => {{ root.quotaDetailsOpen = false; root.render(); }};
        const refreshButton = host.querySelector('[data-cockpit-quota-refresh]');
        if (refreshButton) {{
          const refreshDisabled = root.refreshing || root.hostAvailable === false;
          refreshButton.title = refreshLabel;
          refreshButton.setAttribute('aria-label', refreshLabel);
          refreshButton.disabled = refreshDisabled;
          refreshButton.style.cursor = root.refreshing ? 'wait' : (root.hostAvailable === false ? 'not-allowed' : 'pointer');
          refreshButton.style.opacity = root.refreshing ? '.7' : (root.hostAvailable === false ? '.45' : '1');
          const refreshIcon = refreshButton.querySelector('[data-cockpit-quota-refresh-icon]');
          if (refreshIcon) refreshIcon.style.animation = root.refreshing ? 'cockpit-quota-spin .8s linear infinite' : 'none';
          refreshButton.onclick = () => {{
            if (root.refreshing || root.hostAvailable === false) return;
            root.refreshRequestToken = Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
            root.refreshing = true;
            root.render();
          }};
        }}
      }};
      root.render = render;
      root.scheduleRender = () => {{
        if (root.renderScheduled) return;
        root.renderScheduled = true;
        requestAnimationFrame(() => {{ root.renderScheduled = false; root.render(); }});
      }};
      if (!root.resizeHandler) {{
        root.resizeHandler = () => root.scheduleRender();
        window.addEventListener('resize', root.resizeHandler, {{passive:true}});
      }}
      if (!root.observer) {{
        root.observer = new MutationObserver((mutations) => {{
          const host = document.querySelector('[data-cockpit-quota-footer]');
          const details = document.querySelector('[data-cockpit-quota-details]');
          if (host && mutations.every((mutation) => mutation.target === host || host.contains(mutation.target) || (details && (mutation.target === details || details.contains(mutation.target))))) return;
          root.scheduleRender();
        }});
        root.observer.observe(document.documentElement, {{childList:true,subtree:true}});
      }}
      if (!document.querySelector('[data-cockpit-quota-style]')) {{
        const style = document.createElement('style');
        style.setAttribute('data-cockpit-quota-style', 'true');
        style.textContent = '@keyframes cockpit-quota-spin{{to{{transform:rotate(360deg)}}}}';
        document.head.appendChild(style);
      }}
      if (!root.watchdogTimer) {{
        root.watchdogTimer = window.setInterval(() => {{
          const hostAvailable = Date.now() - (root.hostHeartbeatAt || 0) <= hostHeartbeatTimeoutMs;
          if (root.hostAvailable === hostAvailable && (hostAvailable || (!root.refreshing && !root.refreshRequestToken))) return;
          root.hostAvailable = hostAvailable;
          if (!hostAvailable) {{
            root.refreshing = false;
            root.refreshRequestToken = null;
          }}
          if (root.render) root.render();
        }}, 1000);
      }}
      render();
      return {{refreshRequestToken: pendingRefreshToken}};
    }})()"#
    )
}

fn refresh_request_token_from_cdp_response(value: &Value) -> Option<String> {
    value
        .pointer("/result/result/value/refreshRequestToken")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn selected_model_from_cdp_response(value: &Value) -> Option<String> {
    value
        .pointer("/result/result/value/selectedModel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| {
            value.eq_ignore_ascii_case("deepseek-v4-flash")
                || value.eq_ignore_ascii_case("deepseek-v4-pro")
        })
        .map(|value| value.to_ascii_lowercase())
}

#[derive(Debug, Default)]
struct InjectionEvalResult {
    refresh_request_token: Option<String>,
    selected_model: Option<String>,
}

async fn evaluate_target(target: &CdpTarget, script: &str) -> Option<InjectionEvalResult> {
    if target.target_type != "page" && target.target_type != "webview" {
        return None;
    }
    let Some(websocket_url) = target.websocket_url.as_deref() else {
        return None;
    };
    let Ok(Ok((mut socket, _))) = timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url)).await
    else {
        return None;
    };
    let install_on_new_document = should_install_new_document_script(websocket_url);
    if install_on_new_document {
        let enable_page = socket
            .send(Message::Text(
                json!({
                    "id": 0,
                    "method": "Page.enable",
                    "params": {}
                })
                .to_string()
                .into(),
            ))
            .await
            .is_ok();
        if !enable_page {
            return None;
        }
        let install = socket
            .send(Message::Text(
                json!({
                    "id": 1,
                    "method": "Page.addScriptToEvaluateOnNewDocument",
                    "params": {"source": script}
                })
                .to_string()
                .into(),
            ))
            .await
            .is_ok();
        if !install {
            return None;
        }
        mark_new_document_script_installed(websocket_url);
    }
    if !socket
        .send(Message::Text(
            json!({
                "id": 2,
                "method": "Runtime.evaluate",
                "params": {"expression": script, "returnByValue": true, "awaitPromise": false}
            })
            .to_string()
            .into(),
        ))
        .await
        .is_ok()
    {
        return None;
    }
    timeout(CDP_CONNECT_TIMEOUT, async {
        while let Some(message) = socket.next().await {
            let Ok(Message::Text(text)) = message else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if value.get("id").and_then(Value::as_i64) == Some(2) {
                return Some(InjectionEvalResult {
                    refresh_request_token: refresh_request_token_from_cdp_response(&value),
                    selected_model: selected_model_from_cdp_response(&value),
                });
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

async fn query_targets(client: &Client, port: u16) -> Vec<CdpTarget> {
    let response = client
        .get(format!("http://127.0.0.1:{}/json/list", port))
        .timeout(CDP_CONNECT_TIMEOUT)
        .send()
        .await
        .ok();
    let Some(response) = response else {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_query_failed: port={}, endpoint=/json/list, reason=request_error_or_timeout",
            port,
        ));
        return Vec::new();
    };
    let status = response.status();
    if !status.is_success() {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_query_failed: port={}, endpoint=/json/list, status={}, reason=http_error",
            port, status,
        ));
        return Vec::new();
    }
    let mut targets = match response.json::<Vec<CdpTarget>>().await {
        Ok(targets) => targets,
        Err(error) => {
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth CDP] target_query_failed: port={}, endpoint=/json/list, reason=json_decode_error, error={}",
                port,
                sanitize_cdp_text(&error.to_string()),
            ));
            return Vec::new();
        }
    };
    for target in &mut targets {
        if target
            .websocket_url
            .as_deref()
            .is_some_and(|url| !is_safe_cdp_websocket_url(url, port))
        {
            target.websocket_url = None;
        }
    }
    targets
}

fn is_codex_app_target(target: &CdpTarget) -> bool {
    matches!(target.target_type.as_str(), "page" | "webview") && target.url.starts_with("app://-/")
}

#[cfg(target_os = "macos")]
fn app_server_socket_endpoints(pid: u32) -> Vec<String> {
    let output = Command::new("lsof")
        .args(["-nP", "-a", "-p", &pid.to_string(), "-i"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut endpoints = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            if !(line.contains(" TCP ") || line.contains(" UDP ")) {
                return None;
            }
            let value = line
                .split("->")
                .nth(1)
                .and_then(|part| part.split_whitespace().next())
                .or_else(|| line.split_whitespace().last())?
                .trim_matches(['(', ')'])
                .to_string();
            (!value.is_empty()).then_some(value)
        })
        .collect::<Vec<_>>();
    endpoints.sort();
    endpoints.dedup();
    endpoints
}

#[cfg(not(target_os = "macos"))]
fn app_server_socket_endpoints(_pid: u32) -> Vec<String> {
    Vec::new()
}

fn app_server_auth_file_snapshot(profile_dir: &Path) -> String {
    let auth_path = profile_dir.join("auth.json");
    let Ok(bytes) = fs::read(&auth_path) else {
        return "exists=false".to_string();
    };
    let modified = fs::metadata(&auth_path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let digest = Sha256::digest(&bytes);
    format!(
        "exists=true,size={},mtime={},sha256={:x}",
        bytes.len(),
        modified,
        digest
    )
}

fn collect_app_server_diagnostic_observation(profile_dir: &Path) -> AppServerDiagnosticObservation {
    let mut pids = crate::modules::process::collect_codex_app_server_pids_for_profile(profile_dir);
    pids.sort_unstable();
    pids.dedup();
    let mut sockets = pids
        .iter()
        .flat_map(|pid| {
            app_server_socket_endpoints(*pid)
                .into_iter()
                .map(move |endpoint| format!("pid={}:{}", pid, endpoint))
        })
        .collect::<Vec<_>>();
    sockets.sort();
    sockets.dedup();
    AppServerDiagnosticObservation {
        pids,
        sockets: sockets.join("|"),
        // 官方桌面端持有 app-server 的 stdio，Cockpit 只能通过进程树、socket 和
        // 认证存储快照诊断，不能从外部安全接管它的 stdin/stdout。
        stdio: "owned_by_official_electron".to_string(),
        auth_file: app_server_auth_file_snapshot(profile_dir),
    }
}

async fn app_server_diagnostic_observation(
    profile_dir: PathBuf,
) -> Option<AppServerDiagnosticObservation> {
    timeout(
        Duration::from_secs(2),
        tokio::task::spawn_blocking(move || {
            collect_app_server_diagnostic_observation(&profile_dir)
        }),
    )
    .await
    .ok()?
    .ok()
}

fn is_sensitive_cdp_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "access_token",
        "refresh_token",
        "id_token",
        "authorization",
        "cookie",
        "set-cookie",
        "token",
        "api_key",
        "apikey",
        "x-api-key",
        "x-openai-api-key",
        "openai-api-key",
        "client_secret",
        "password",
        "private_key",
        "secret",
    ]
    .iter()
    .any(|part| key == *part || key.contains(part))
}

fn sanitize_cdp_json(value: &Value, depth: usize) -> Value {
    if depth > 8 {
        return Value::String("<nested-value-redacted>".to_string());
    }
    match value {
        Value::Object(object) => {
            let mut sanitized = serde_json::Map::new();
            for (key, value) in object {
                if is_sensitive_cdp_key(key) {
                    sanitized.insert(key.clone(), Value::String("<redacted>".to_string()));
                } else {
                    sanitized.insert(key.clone(), sanitize_cdp_json(value, depth + 1));
                }
            }
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_cdp_json(item, depth + 1))
                .collect(),
        ),
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if lower.starts_with("bearer ") || lower.starts_with("rt.") || lower.starts_with("eyj")
            {
                Value::String("<redacted>".to_string())
            } else {
                Value::String(text.chars().take(AUTH_NETWORK_BODY_PREVIEW_LIMIT).collect())
            }
        }
        _ => value.clone(),
    }
}

fn sanitize_cdp_headers(value: Option<&Value>) -> Value {
    value
        .map(|value| sanitize_cdp_json(value, 0))
        .unwrap_or_else(|| json!({}))
}

fn sanitize_cdp_text(raw: &str) -> String {
    sanitize_cdp_json(&Value::String(raw.to_string()), 0)
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// 从官方 renderer/app-server 通过 console 或 Log domain 暴露的文本中提取认证错误码。
/// 这里只返回固定白名单信号，避免把 token、Cookie 或完整日志写入诊断文件。
fn auth_diagnostic_error_signal(raw: &str) -> Option<&'static str> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("invalid_refresh_token") || lower.contains("invalid refresh token") {
        return Some("invalid_refresh_token");
    }
    if lower.contains("auth_token_missing") {
        return Some("auth_token_missing");
    }
    if lower.contains("no_token_attached") {
        return Some("no_token_attached");
    }
    if lower.contains("cloud_requirements_auth_error") {
        return Some("cloud_requirements_auth_error");
    }
    // getAuthStatus 在 refresh 失败后会返回 requiresOpenaiAuth=true；只识别明确的
    // JSON/日志形式，避免把普通页面文本中的同名字段误报为认证失效。
    if lower.contains("requiresopenaiauth=true")
        || lower.contains("requires_openai_auth=true")
        || lower.contains("\"requiresopenaiauth\":true")
    {
        return Some("requiresOpenaiAuth");
    }
    None
}

fn cdp_console_auth_signal(params: &Value) -> Option<&'static str> {
    let args = params.get("args")?.as_array()?;
    for arg in args {
        for candidate in [
            arg.get("value").and_then(Value::as_str),
            arg.get("description").and_then(Value::as_str),
            arg.get("unserializableValue").and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(signal) = auth_diagnostic_error_signal(candidate) {
                return Some(signal);
            }
        }
    }
    None
}

fn sanitize_cdp_url(raw: &str) -> String {
    let Ok(parsed) = url::Url::parse(raw) else {
        return raw.chars().take(512).collect();
    };
    let mut result = format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or(""),
        parsed.path()
    );
    if parsed.query().is_some() {
        result.push_str("?<redacted-query>");
    }
    result.chars().take(768).collect()
}

fn is_auth_diagnostic_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    [
        "cloudrequirements",
        "cloud-requirements",
        "cloudconfigbundle",
        "cloud-config-bundle",
        "oauth",
        "/auth",
        "login",
        "relogin",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

fn should_capture_cdp_response_body(url: &str, status: u64) -> bool {
    is_auth_diagnostic_url(url) || status == 401 || status == 403
}

fn cdp_body_preview(value: &Value) -> Option<String> {
    let body = value.pointer("/result/body")?.as_str()?;
    let body_len = body.len();
    let preview = serde_json::from_str::<Value>(body)
        .map(|json| sanitize_cdp_json(&json, 0))
        .ok()
        .and_then(|json| serde_json::to_string(&json).ok())
        .unwrap_or_else(|| "<non-json-body-redacted>".to_string());
    Some(format!("body_len={}, body_preview={}", body_len, preview))
}

/// 复刻官方客户端对 cloudRequirements/cloudConfigBundle 响应的认证判定。
/// 这里只返回诊断结论，不把响应当作启动前的可用性保证。
fn cdp_auth_signal(value: &Value) -> Option<&'static str> {
    let body = value.pointer("/result/body")?.as_str()?;
    let payload = serde_json::from_str::<Value>(body).ok()?;
    if let Ok(serialized) = serde_json::to_string(&payload) {
        if let Some(signal) = auth_diagnostic_error_signal(&serialized) {
            return Some(signal);
        }
    }
    let data = payload.get("data")?;
    let reason = data.get("reason").and_then(Value::as_str).unwrap_or("");
    if !matches!(reason, "cloudRequirements" | "cloudConfigBundle") {
        return None;
    }
    if data.get("errorCode").and_then(Value::as_str) == Some("Auth") {
        return Some("auth_error_code");
    }
    if data.get("action").and_then(Value::as_str) == Some("relogin") {
        return Some("relogin_action");
    }
    None
}

async fn monitor_cdp_target(
    instance_id: &str,
    profile_key: &str,
    target: &CdpTarget,
) -> Option<AuthPageSnapshot> {
    let Some(websocket_url) = target.websocket_url.as_deref() else {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_skipped: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}, reason=missing_safe_websocket_url",
            instance_id,
            profile_key,
            target.target_id,
            target.target_type,
            sanitize_cdp_url(&target.url),
        ));
        return None;
    };
    let socket = timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url)).await;
    let Ok(Ok((mut socket, _))) = socket else {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_attach_failed: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}, websocket_url={}, reason=connect_or_timeout",
            instance_id,
            profile_key,
            target.target_id,
            target.target_type,
            sanitize_cdp_url(&target.url),
            sanitize_cdp_url(websocket_url),
        ));
        return None;
    };

    let mut next_command_id = 100i64;
    let mut pending_body_requests: HashMap<i64, (String, String)> = HashMap::new();
    let mut body_candidates: HashMap<String, (String, u64)> = HashMap::new();
    let mut request_started: HashMap<String, Instant> = HashMap::new();
    let target_label = if target.target_id.is_empty() {
        "unknown"
    } else {
        target.target_id.as_str()
    };
    logger::log_codex_auth_diagnostic(&format!(
        "[Codex Auth Network] target_attached: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}",
        instance_id,
        profile_key,
        target_label,
        target.target_type,
        sanitize_cdp_url(&target.url),
    ));

    let _ = socket
        .send(Message::Text(
            json!({
                "id": 1,
                "method": "Network.enable",
                "params": {
                    "maxTotalBufferSize": 4 * 1024 * 1024,
                    "maxResourceBufferSize": 512 * 1024,
                    "maxPostDataSize": 0
                }
            })
            .to_string()
            .into(),
        ))
        .await;

    for (id, method) in [(2, "Runtime.enable"), (3, "Log.enable")] {
        let _ = socket
            .send(Message::Text(
                json!({"id": id, "method": method, "params": {}})
                    .to_string()
                    .into(),
            ))
            .await;
    }

    if target.target_type == "page" || target.target_type == "webview" {
        let _ = socket
            .send(Message::Text(
                json!({
                    "id": 4,
                    "method": "Page.enable",
                    "params": {}
                })
                .to_string()
                .into(),
            ))
            .await;
        let _ = socket
            .send(Message::Text(
                json!({
                    "id": 5,
                    "method": "Page.setLifecycleEventsEnabled",
                    "params": {"enabled": true}
                })
                .to_string()
                .into(),
            ))
            .await;
        let _ = socket
            .send(Message::Text(
                json!({
                    "id": 6,
                    "method": "Runtime.evaluate",
                    "params": {
                        "expression": AUTH_DIAGNOSTIC_SCRIPT,
                        "returnByValue": true,
                        "awaitPromise": false
                    }
                })
                .to_string()
                .into(),
            ))
            .await;
    }

    let mut snapshot = None;
    let capture_until = Instant::now() + AUTH_NETWORK_CAPTURE_WINDOW;
    loop {
        let remaining = capture_until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let message = match timeout(remaining, socket.next()).await {
            Ok(Some(Ok(message))) => message,
            _ => break,
        };
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };

        if let Some(command_id) = value.get("id").and_then(Value::as_i64) {
            if command_id == 6 {
                snapshot = value
                    .pointer("/result/result/value")
                    .cloned()
                    .and_then(|result| serde_json::from_value::<AuthPageSnapshot>(result).ok());
            } else if let Some((request_id, url)) = pending_body_requests.remove(&command_id) {
                let body =
                    cdp_body_preview(&value).unwrap_or_else(|| "body_unavailable=true".to_string());
                let auth_signal = cdp_auth_signal(&value).unwrap_or("none");
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] response_body: instance_id={}, profile={}, target_id={}, request_id={}, url={}, auth_signal={}, {}",
                    instance_id,
                    profile_key,
                    target_label,
                    request_id,
                    sanitize_cdp_url(&url),
                    auth_signal,
                    body,
                ));
            }
            continue;
        }

        let Some(method) = value.get("method").and_then(Value::as_str) else {
            continue;
        };
        let Some(params) = value.get("params") else {
            continue;
        };
        match method {
            "Network.requestWillBeSent" => {
                let request_id = params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let request = params.get("request").cloned().unwrap_or_else(|| json!({}));
                let url = request.get("url").and_then(Value::as_str).unwrap_or("");
                request_started.insert(request_id.clone(), Instant::now());
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] request: instance_id={}, profile={}, target_id={}, request_id={}, type={}, method={}, url={}, has_post_data={}, headers={}",
                    instance_id,
                    profile_key,
                    target_label,
                    request_id,
                    params.get("type").and_then(Value::as_str).unwrap_or(""),
                    request.get("method").and_then(Value::as_str).unwrap_or(""),
                    sanitize_cdp_url(url),
                    request.get("hasPostData").and_then(Value::as_bool).unwrap_or(false),
                    sanitize_cdp_headers(request.get("headers")),
                ));
            }
            "Network.requestWillBeSentExtraInfo" | "Network.responseReceivedExtraInfo" => {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] {}: instance_id={}, profile={}, target_id={}, request_id={}, headers={}",
                    method,
                    instance_id,
                    profile_key,
                    target_label,
                    params.get("requestId").and_then(Value::as_str).unwrap_or(""),
                    sanitize_cdp_headers(params.get("headers")),
                ));
            }
            "Network.responseReceived" => {
                let request_id = params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let response = params.get("response").cloned().unwrap_or_else(|| json!({}));
                let url = response.get("url").and_then(Value::as_str).unwrap_or("");
                let status = response.get("status").and_then(Value::as_u64).unwrap_or(0);
                let duration_ms = request_started
                    .get(&request_id)
                    .map(|started| started.elapsed().as_millis());
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] response: instance_id={}, profile={}, target_id={}, request_id={}, status={}, duration_ms={:?}, url={}, mime_type={}, headers={}",
                    instance_id,
                    profile_key,
                    target_label,
                    request_id,
                    status,
                    duration_ms,
                    sanitize_cdp_url(url),
                    response.get("mimeType").and_then(Value::as_str).unwrap_or(""),
                    sanitize_cdp_headers(response.get("headers")),
                ));
                if !request_id.is_empty() && should_capture_cdp_response_body(url, status) {
                    // Auth endpoints can return HTTP 200 with {errorCode: "Auth", action:
                    // "relogin"}; defer getResponseBody until loadingFinished so this case is
                    // captured as reliably as a 401/403 response.
                    body_candidates.insert(request_id, (url.to_string(), status));
                }
            }
            "Network.loadingFinished" => {
                let request_id = params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let Some((url, status)) = body_candidates.remove(request_id) else {
                    continue;
                };
                let command_id = next_command_id;
                next_command_id += 1;
                pending_body_requests.insert(command_id, (request_id.to_string(), url.clone()));
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] body_capture: instance_id={}, profile={}, target_id={}, request_id={}, status={}, url={}, encoded_data_length={}",
                    instance_id,
                    profile_key,
                    target_label,
                    request_id,
                    status,
                    sanitize_cdp_url(&url),
                    params
                        .get("encodedDataLength")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                ));
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "id": command_id,
                            "method": "Network.getResponseBody",
                            "params": {"requestId": request_id}
                        })
                        .to_string()
                        .into(),
                    ))
                    .await;
            }
            "Network.loadingFailed" => {
                let request_id = params
                    .get("requestId")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                body_candidates.remove(request_id);
                let duration_ms = request_started
                    .get(request_id)
                    .map(|started| started.elapsed().as_millis());
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] loading_failed: instance_id={}, profile={}, target_id={}, request_id={}, duration_ms={:?}, error_text={}, canceled={}, blocked_reason={}",
                    instance_id,
                    profile_key,
                    target_label,
                    request_id,
                    duration_ms,
                    sanitize_cdp_text(
                        params.get("errorText").and_then(Value::as_str).unwrap_or(""),
                    ),
                    params.get("canceled").and_then(Value::as_bool).unwrap_or(false),
                    params.get("blockedReason").and_then(Value::as_str).unwrap_or(""),
                ));
            }
            "Network.webSocketCreated"
            | "Network.webSocketWillSendHandshakeRequest"
            | "Network.webSocketHandshakeResponseReceived"
            | "Network.webSocketClosed"
            | "Network.webSocketFrameError" => {
                let response = params.get("response").unwrap_or(&Value::Null);
                let url = params
                    .get("url")
                    .and_then(Value::as_str)
                    .or_else(|| response.get("url").and_then(Value::as_str))
                    .unwrap_or("");
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] {}: instance_id={}, profile={}, target_id={}, request_id={}, url={}, status={}, error={}",
                    method,
                    instance_id,
                    profile_key,
                    target_label,
                    params
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    sanitize_cdp_url(url),
                    response.get("status").and_then(Value::as_u64).unwrap_or(0),
                    sanitize_cdp_text(
                        params
                            .get("errorMessage")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    ),
                ));
            }
            "Network.webSocketFrameSent" | "Network.webSocketFrameReceived" => {
                let response = params.get("response").unwrap_or(&Value::Null);
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth Network] {}: instance_id={}, profile={}, target_id={}, request_id={}, opcode={}, payload_bytes={}",
                    method,
                    instance_id,
                    profile_key,
                    target_label,
                    params
                        .get("requestId")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    response.get("opcode").and_then(Value::as_u64).unwrap_or(0),
                    response
                        .get("payloadData")
                        .and_then(Value::as_str)
                        .map(str::len)
                        .unwrap_or(0),
                ));
            }
            "Page.frameNavigated" => {
                let frame = params.get("frame").unwrap_or(&Value::Null);
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] frame_navigated: instance_id={}, profile={}, target_id={}, frame_id={}, url={}, name={}",
                    instance_id,
                    profile_key,
                    target_label,
                    frame.get("id").and_then(Value::as_str).unwrap_or(""),
                    sanitize_cdp_url(frame.get("url").and_then(Value::as_str).unwrap_or("")),
                    sanitize_cdp_text(frame.get("name").and_then(Value::as_str).unwrap_or("")),
                ));
            }
            "Page.lifecycleEvent" => {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] lifecycle: instance_id={}, profile={}, target_id={}, frame_id={}, loader_id={}, name={}",
                    instance_id,
                    profile_key,
                    target_label,
                    params.get("frameId").and_then(Value::as_str).unwrap_or(""),
                    params.get("loaderId").and_then(Value::as_str).unwrap_or(""),
                    params.get("name").and_then(Value::as_str).unwrap_or(""),
                ));
            }
            "Runtime.consoleAPICalled" => {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] console: instance_id={}, profile={}, target_id={}, type={}, execution_context_id={}, arg_count={}",
                    instance_id,
                    profile_key,
                    target_label,
                    params.get("type").and_then(Value::as_str).unwrap_or(""),
                    params
                        .get("executionContextId")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    params
                        .get("args")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0),
                ));
                if let Some(signal) = cdp_console_auth_signal(params) {
                    logger::log_warn(&format!(
                        "[Codex Auth CDP] 捕获官方认证错误信号: instance_id={}, profile={}, target_id={}, source=console, code={}",
                        instance_id, profile_key, target_label, signal,
                    ));
                    logger::log_codex_auth_diagnostic(&format!(
                        "[Codex Auth CDP] auth_error_signal: instance_id={}, profile={}, target_id={}, source=console, code={}",
                        instance_id, profile_key, target_label, signal,
                    ));
                }
            }
            "Runtime.exceptionThrown" => {
                let details = params.get("exceptionDetails").unwrap_or(&Value::Null);
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] exception: instance_id={}, profile={}, target_id={}, text={}, url={}, line={}, column={}",
                    instance_id,
                    profile_key,
                    target_label,
                    sanitize_cdp_text(details.get("text").and_then(Value::as_str).unwrap_or("")),
                    sanitize_cdp_url(details.get("url").and_then(Value::as_str).unwrap_or("")),
                    details.get("lineNumber").and_then(Value::as_i64).unwrap_or(-1),
                    details
                        .get("columnNumber")
                        .and_then(Value::as_i64)
                        .unwrap_or(-1),
                ));
            }
            "Log.entryAdded" => {
                let entry = params.get("entry").unwrap_or(&Value::Null);
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] log_entry: instance_id={}, profile={}, target_id={}, level={}, source={}, text={}, url={}",
                    instance_id,
                    profile_key,
                    target_label,
                    entry.get("level").and_then(Value::as_str).unwrap_or(""),
                    entry.get("source").and_then(Value::as_str).unwrap_or(""),
                    sanitize_cdp_text(entry.get("text").and_then(Value::as_str).unwrap_or("")),
                    sanitize_cdp_url(entry.get("url").and_then(Value::as_str).unwrap_or("")),
                ));
                if let Some(signal) = auth_diagnostic_error_signal(
                    entry.get("text").and_then(Value::as_str).unwrap_or(""),
                ) {
                    logger::log_warn(&format!(
                        "[Codex Auth CDP] 捕获官方认证错误信号: instance_id={}, profile={}, target_id={}, source=log, code={}",
                        instance_id, profile_key, target_label, signal,
                    ));
                    logger::log_codex_auth_diagnostic(&format!(
                        "[Codex Auth CDP] auth_error_signal: instance_id={}, profile={}, target_id={}, source=log, code={}",
                        instance_id, profile_key, target_label, signal,
                    ));
                }
            }
            _ => {}
        }
    }
    if let Some(snapshot) = snapshot.as_ref() {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_snapshot: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}, physical_url={}, route={}, route_source={}, title={}, ready_state={}, official_login_route={}, login_signal={}, login_ui_signal={}, login_ui_markers={:?}",
            instance_id,
            profile_key,
            target_label,
            target.target_type,
            sanitize_cdp_url(&target.url),
            sanitize_cdp_url(&snapshot.physical_url),
            snapshot.route,
            snapshot.route_source,
            sanitize_cdp_text(&snapshot.title),
            snapshot.ready_state,
            is_official_login_route(&snapshot.route),
            snapshot.login_signal(),
            snapshot.has_login_ui_signal(),
            snapshot.login_ui_markers,
        ));
    } else {
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_snapshot_missing: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}, reason=no_runtime_snapshot",
            instance_id,
            profile_key,
            target_label,
            target.target_type,
            sanitize_cdp_url(&target.url),
        ));
    }
    snapshot
}

/// 轻量读取当前页面认证快照。
///
/// 实时状态判断只需要一次 `Runtime.evaluate`，不启用 `Network`、`Log` 或页面生命周期
/// 事件，避免每轮轮询把官方客户端的全部网络事件复制到本地。完整网络诊断由低频后台任务
/// 单独执行。
async fn monitor_cdp_target_snapshot(
    instance_id: &str,
    profile_key: &str,
    target: &CdpTarget,
) -> Option<AuthPageSnapshot> {
    let Some(websocket_url) = target.websocket_url.as_deref() else {
        return None;
    };
    let socket = timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url)).await;
    let Ok(Ok((mut socket, _))) = socket else {
        return None;
    };
    let _ = socket
        .send(Message::Text(
            json!({
                "id": 1,
                "method": "Runtime.evaluate",
                "params": {
                    "expression": AUTH_DIAGNOSTIC_SCRIPT,
                    "returnByValue": true,
                    "awaitPromise": false
                }
            })
            .to_string()
            .into(),
        ))
        .await;
    let capture_until = Instant::now() + AUTH_SNAPSHOT_TIMEOUT;
    loop {
        let remaining = capture_until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let message = match timeout(remaining, socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => text,
            _ => return None,
        };
        let Ok(value) = serde_json::from_str::<Value>(&message) else {
            continue;
        };
        if value.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        let snapshot = value
            .pointer("/result/result/value")
            .cloned()
            .and_then(|result| serde_json::from_value::<AuthPageSnapshot>(result).ok());
        if snapshot.is_none() {
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth CDP] lightweight_snapshot_missing: instance_id={}, profile={}, target_id={}, target_url={}, reason=runtime_evaluate_empty",
                instance_id,
                profile_key,
                target.target_id,
                sanitize_cdp_url(&target.url),
            ));
        }
        return snapshot;
    }
}

fn is_safe_cdp_websocket_url(raw: &str, expected_port: u16) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let Ok(address) = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
    else {
        return false;
    };
    address.is_loopback() && parsed.port() == Some(expected_port)
}

fn auth_diagnostic_observation(
    targets: &[CdpTarget],
    snapshot: Option<AuthPageSnapshot>,
) -> AuthDiagnosticObservation {
    let Some(snapshot) = snapshot else {
        return AuthDiagnosticObservation::unavailable();
    };
    let login_route = is_official_login_route(&snapshot.route);
    let login_ui_signal = snapshot.has_login_ui_signal();
    let login_ui_markers = snapshot.login_ui_markers.clone();
    AuthDiagnosticObservation {
        cdp_available: true,
        target_count: targets.len(),
        route: snapshot.route,
        route_source: snapshot.route_source,
        title: snapshot.title,
        ready_state: snapshot.ready_state,
        login_route,
        login_ui_signal,
        login_ui_markers,
    }
}

async fn run_auth_diagnostic_loop(
    app: AppHandle,
    instance_id: String,
    profile_key: String,
    profile_dir: PathBuf,
    port: u16,
    bind_account_id: Option<String>,
) {
    let client = Client::new();
    // 监测任务由实例进程启动后立即创建；记录该时刻作为本次客户端启动时间。
    let launch_started_at = chrono::Utc::now().timestamp();
    let mut launch_recorded = false;
    let mut previous: Option<AuthDiagnosticObservation> = None;
    let mut previous_app_server: Option<AppServerDiagnosticObservation> = None;
    let mut previous_profile_identity: Option<ProfileOAuthIdentityObservation> = None;
    let mut last_identity_check_at: Option<Instant> = None;
    let mut last_identity_status: Option<String> = None;
    let mut last_network_diagnostic_at: Option<Instant> = None;
    let mut login_streak = 0u8;
    let mut available_streak = 0u8;
    loop {
        if app_lifecycle::is_shutdown_started() {
            return;
        }

        // 只观察官方 Codex 应用页面，忽略 devtools、错误页和其它外部 target。
        // 这些页面的 URL/文字不能代表客户端是否跳转到登录页。
        let discovered_targets = query_targets(&client, port).await;
        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] target_scan: instance_id={}, profile={}, bind_account_id={}, port={}, discovered_count={}, discovered_targets={}",
            instance_id,
            profile_key,
            bind_account_id.as_deref().unwrap_or(""),
            port,
            discovered_targets.len(),
            discovered_targets
                .iter()
                .map(|target| format!(
                    "{}:{}:{}:{}",
                    target.target_id,
                    target.target_type,
                    sanitize_cdp_url(&target.url),
                    target.websocket_url.is_some()
                ))
                .collect::<Vec<_>>()
                .join("|"),
        ));
        let targets: Vec<CdpTarget> = discovered_targets
            .into_iter()
            .filter(is_codex_app_target)
            .collect();
        if targets.is_empty() {
            logger::log_warn(&format!(
                "[Codex Auth CDP] no_codex_app_target: instance_id={}, profile={}, bind_account_id={}, port={}, reason=filtered_all_targets",
                instance_id,
                profile_key,
                bind_account_id.as_deref().unwrap_or(""),
                port,
            ));
        }
        // 认证状态使用轻量快照高频检查，避免每轮都打开 Network/Log 事件流。
        let mut snapshot_tasks = JoinSet::new();
        for target in targets.iter().cloned() {
            let instance_id = instance_id.clone();
            let profile_key = profile_key.clone();
            snapshot_tasks.spawn(async move {
                let snapshot =
                    monitor_cdp_target_snapshot(&instance_id, &profile_key, &target).await;
                (target, snapshot)
            });
        }
        let mut selected_snapshot = None;
        while let Some(result) = snapshot_tasks.join_next().await {
            let Ok((target, snapshot)) = result else {
                logger::log_warn(&format!(
                    "[Codex Auth CDP] lightweight_snapshot_task_failed: instance_id={}, profile={}, reason=join_error",
                    instance_id, profile_key,
                ));
                continue;
            };
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth CDP] lightweight_snapshot_result: instance_id={}, profile={}, target_id={}, target_type={}, target_url={}, snapshot_present={}, snapshot_route={}, snapshot_official_login_route={}, snapshot_login_signal={}, snapshot_login_ui_signal={}, snapshot_login_ui_markers={}",
                instance_id,
                profile_key,
                target.target_id,
                target.target_type,
                sanitize_cdp_url(&target.url),
                snapshot.is_some(),
                snapshot.as_ref().map(|value| value.route.as_str()).unwrap_or(""),
                snapshot
                    .as_ref()
                    .is_some_and(|value| is_official_login_route(&value.route)),
                snapshot.as_ref().is_some_and(AuthPageSnapshot::login_signal),
                snapshot
                    .as_ref()
                    .is_some_and(AuthPageSnapshot::has_login_ui_signal),
                snapshot
                    .as_ref()
                    .map(|value| format!("{:?}", value.login_ui_markers))
                    .unwrap_or_default(),
            ));
            let Some(snapshot) = snapshot else {
                continue;
            };
            if snapshot.has_login_ui_signal() && !is_official_login_route(&snapshot.route) {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex Auth CDP] login_ui_route_mismatch: instance_id={}, profile={}, target_id={}, physical_url={}, observed_route={}, route_source={}, markers={:?}, reason=official_desktop_uses_memory_router",
                    instance_id,
                    profile_key,
                    target.target_id,
                    sanitize_cdp_url(&snapshot.physical_url),
                    snapshot.route,
                    snapshot.route_source,
                    snapshot.login_ui_markers,
                ));
            }
            if selected_snapshot.is_none() || snapshot.login_signal() {
                selected_snapshot = Some(snapshot);
            }
            if selected_snapshot
                .as_ref()
                .is_some_and(AuthPageSnapshot::login_signal)
            {
                // 不能提前取消其它 target：它们可能仍在读取当前实例的页面状态。
            }
        }
        let network_diagnostic_due = last_network_diagnostic_at
            .map(|started_at| started_at.elapsed() >= AUTH_NETWORK_DIAGNOSTIC_INTERVAL)
            .unwrap_or(true);
        if network_diagnostic_due && !targets.is_empty() {
            last_network_diagnostic_at = Some(Instant::now());
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth Network] capture_scheduled: instance_id={}, profile={}, target_count={}, interval_secs={}",
                instance_id,
                profile_key,
                targets.len(),
                AUTH_NETWORK_DIAGNOSTIC_INTERVAL.as_secs(),
            ));
            for target in targets.iter().cloned() {
                let instance_id = instance_id.clone();
                let profile_key = profile_key.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = monitor_cdp_target(&instance_id, &profile_key, &target).await;
                });
            }
        }
        if let Some(app_server) = app_server_diagnostic_observation(profile_dir.clone()).await {
            if previous_app_server.as_ref() != Some(&app_server) {
                logger::log_codex_auth_diagnostic(&format!(
                    "[Codex AppServer Diagnostic] state: instance_id={}, profile={}, bind_account_id={}, app_server_pids={:?}, sockets={}, stdio={}, auth_file={}",
                    instance_id,
                    profile_key,
                    bind_account_id.as_deref().unwrap_or(""),
                    app_server.pids,
                    app_server.sockets,
                    app_server.stdio,
                    app_server.auth_file,
                ));
                previous_app_server = Some(app_server);
            }
        }
        let observation = auth_diagnostic_observation(&targets, selected_snapshot);
        let changed = previous.as_ref() != Some(&observation);
        if changed {
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth CDP] 页面认证路由状态变化: instance_id={}, profile={}, bind_account_id={}, cdp_available={}, target_count={}, route={}, route_source={}, title={}, ready_state={}, login_route={}, login_ui_signal={}, login_ui_markers={:?}",
                instance_id,
                profile_key,
                bind_account_id.as_deref().unwrap_or(""),
                observation.cdp_available,
                observation.target_count,
                observation.route,
                observation.route_source,
                observation.title,
                observation.ready_state,
                observation.login_route,
                observation.login_ui_signal,
                observation.login_ui_markers,
            ));
            if observation.login_signal() {
                logger::log_warn(&format!(
                    "[Codex Auth CDP] 检测到官方登录状态: instance_id={}, route={}, login_route={}, login_ui_signal={}, login_signal={}",
                    instance_id,
                    observation.route,
                    observation.login_route,
                    observation.login_ui_signal,
                    observation.login_signal(),
                ));
            }
            previous = Some(observation.clone());
        }

        let mut stable_status = None;
        if observation.login_signal() {
            login_streak = login_streak.saturating_add(1);
            available_streak = 0;
            if login_streak >= 2 {
                stable_status = Some(("login_required", true));
            }
        } else if observation.cdp_available {
            available_streak = available_streak.saturating_add(1);
            login_streak = 0;
            if available_streak >= 2 {
                stable_status = Some(("available", false));
            }
        }

        logger::log_codex_auth_diagnostic(&format!(
            "[Codex Auth CDP] status_evaluation: instance_id={}, profile={}, bind_account_id={}, route={}, cdp_available={}, target_count={}, login_route={}, login_ui_signal={}, login_streak={}, available_streak={}, stable_status={}",
            instance_id,
            profile_key,
            bind_account_id.as_deref().unwrap_or(""),
            observation.route,
            observation.cdp_available,
            observation.target_count,
            observation.login_route,
            observation.login_ui_signal,
            login_streak,
            available_streak,
            stable_status
                .as_ref()
                .map(|(status, _)| *status)
                .unwrap_or("none"),
        ));

        if let Some((status, login_redirect)) = stable_status {
            let status_changed = last_identity_status.as_deref() != Some(status);
            let retry_due = last_identity_check_at
                .map(|checked_at| checked_at.elapsed() >= AUTH_IDENTITY_RETRY_INTERVAL)
                .unwrap_or(true);
            if !status_changed && !retry_due {
                tokio::time::sleep(AUTH_DIAGNOSTIC_INTERVAL).await;
                continue;
            }
            last_identity_check_at = Some(Instant::now());
            last_identity_status = Some(status.to_string());
            logger::log_codex_auth_diagnostic(&format!(
                "[Codex Auth Identity] persistence_attempt: instance_id={}, profile={}, status={}, login_redirect={}, reason=status_stable",
                instance_id, profile_key, status, login_redirect,
            ));
            let identity =
                observe_profile_oauth_identity(profile_dir.clone(), bind_account_id.clone()).await;
            if previous_profile_identity.as_ref() != Some(&identity) {
                log_profile_oauth_identity_observation(&instance_id, &profile_key, &identity);
                previous_profile_identity = Some(identity.clone());
            }
            if let ProfileOAuthIdentityObservation::Matched { account_id, .. } = identity {
                if !launch_recorded {
                    match codex_account::record_client_launch(
                        &account_id,
                        &instance_id,
                        launch_started_at,
                    )
                    .await
                    {
                        Ok(()) => {
                            launch_recorded = true;
                            let _ = app.emit(
                                "accounts:changed",
                                json!({
                                    "platformId": "codex",
                                    "accountId": account_id,
                                    "reason": "client-auth-launch",
                                    "instanceId": instance_id,
                                }),
                            );
                            logger::log_codex_auth_diagnostic(&format!(
                                "[Codex Auth Identity] launch_time_recorded: instance_id={}, profile={}, account_id={}, launched_at={}",
                                instance_id, profile_key, account_id, launch_started_at,
                            ));
                        }
                        Err(error) => logger::log_warn(&format!(
                            "[Codex Auth Identity] failed to record launch time: instance_id={}, profile={}, account_id={}, error={}",
                            instance_id, profile_key, account_id, error,
                        )),
                    }
                }
                match codex_account::update_client_auth_observation(
                    &account_id,
                    &instance_id,
                    status,
                    login_redirect,
                )
                .await
                {
                    Ok(()) => {
                        logger::log_codex_auth_diagnostic(&format!(
                            "[Codex Auth Identity] persistence_succeeded: instance_id={}, profile={}, account_id={}, status={}, login_redirect={}",
                            instance_id, profile_key, account_id, status, login_redirect,
                        ));
                        // 仅在状态发生变化时通知前端，避免 30 秒重试周期反复触发账号列表刷新。
                        if status_changed {
                            let _ = app.emit(
                                "accounts:changed",
                                json!({
                                    "platformId": "codex",
                                    "accountId": account_id,
                                    "reason": "client-auth-observation",
                                    "status": status,
                                    "loginRedirect": login_redirect,
                                    "instanceId": instance_id,
                                }),
                            );
                            logger::log_codex_auth_diagnostic(&format!(
                                "[Codex Auth Identity] frontend_sync_emitted: instance_id={}, profile={}, account_id={}, status={}, login_redirect={}",
                                instance_id, profile_key, account_id, status, login_redirect,
                            ));
                        }
                    }
                    Err(error) => logger::log_warn(&format!(
                        "[Codex Auth Identity] failed to persist observation: instance_id={}, profile={}, account_id={}, status={}, error={}",
                        instance_id, profile_key, account_id, status, error,
                    )),
                }
            }
        }

        tokio::time::sleep(AUTH_DIAGNOSTIC_INTERVAL).await;
    }
}

async fn api_service_quota_refresh_targets() -> Result<(usize, Vec<String>), String> {
    let state = codex_local_access::get_local_access_state().await?;
    let Some(collection) = state.collection else {
        return Ok((0, Vec::new()));
    };
    let mut existing_account_count = 0;
    let mut target_ids = Vec::new();
    for account_id in collection.account_ids {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        existing_account_count += 1;
        if codex_quota::supports_quota_refresh(&account) {
            target_ids.push(account_id);
        }
    }
    Ok((existing_account_count, target_ids))
}

async fn refresh_api_service_quota_pool(app: &AppHandle) -> Result<(i32, usize), String> {
    let (existing_account_count, target_ids) = api_service_quota_refresh_targets().await?;
    if existing_account_count == 0 {
        return Ok((0, 0));
    }
    if target_ids.is_empty() {
        return Err("API 服务账号池暂无可刷新的额度".to_string());
    }
    let total = target_ids.len();
    let success_count = crate::commands::codex::refresh_codex_quotas_batch(
        app.clone(),
        target_ids,
        Some(true),
        Some(false),
    )
    .await?;
    if success_count <= 0 {
        return Err("API 服务账号池额度刷新失败".to_string());
    }
    Ok((success_count, total))
}

async fn run_quota_refresh_singleflight(app: &AppHandle) -> Result<Option<(i32, usize)>, String> {
    let lock = quota_refresh_lock();
    match lock.try_lock() {
        Ok(_guard) => refresh_api_service_quota_pool(app).await.map(Some),
        Err(_) => {
            let _guard = lock.lock().await;
            let (existing_account_count, _) = api_service_quota_refresh_targets().await?;
            Ok((existing_account_count == 0).then_some((0, 0)))
        }
    }
}

async fn run_injection_loop(
    app: AppHandle,
    _instance_id: String,
    profile_dir: PathBuf,
    port: u16,
    bind_account_id: Option<String>,
) {
    let client = Client::new();
    let mut last_quota_at = Instant::now() - QUOTA_REFRESH_INTERVAL;
    let mut quota = QuotaResponse::default();
    let mut handled_refresh_token: Option<String> = None;
    let mut refresh_tasks = JoinSet::new();
    loop {
        if app_lifecycle::is_shutdown_started() {
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        }
        let mut refresh_finished = false;
        let mut refreshed_empty_pool = false;
        if let Some(result) = refresh_tasks.try_join_next() {
            refresh_finished = true;
            match result {
                Ok(Ok(Some((0, 0)))) => {
                    refreshed_empty_pool = true;
                    logger::log_info("[Codex App Injection] API 服务账号池为空，额度已归零");
                }
                Ok(Ok(Some((success_count, total)))) if success_count as usize == total => {
                    logger::log_info(&format!(
                        "[Codex App Injection] API 服务额度刷新完成: success={}/{}",
                        success_count, total
                    ));
                }
                Ok(Ok(Some((success_count, total)))) => {
                    logger::log_warn(&format!(
                        "[Codex App Injection] API 服务额度部分刷新完成: success={}/{}",
                        success_count, total
                    ));
                }
                Ok(Ok(None)) => {
                    logger::log_info("[Codex App Injection] 已等待另一个实例完成 API 服务额度刷新")
                }
                Ok(Err(error)) => logger::log_warn(&format!(
                    "[Codex App Injection] API 服务额度刷新失败: {}",
                    error
                )),
                Err(error) => logger::log_warn(&format!(
                    "[Codex App Injection] API 服务额度刷新任务异常结束: {}",
                    error
                )),
            }
        }
        let locale = config::get_user_config().language;
        let deepseek_cdp = bind_uses_deepseek_cdp_injection(bind_account_id.as_deref());
        if deepseek_cdp {
            let account_id = bind_account_id_value(bind_account_id.as_deref());
            let selected_model = account_id
                .as_deref()
                .and_then(crate::modules::codex_account::load_account)
                .and_then(|account| account.api_startup_model)
                .filter(|model| {
                    model.eq_ignore_ascii_case("deepseek-v4-flash")
                        || model.eq_ignore_ascii_case("deepseek-v4-pro")
                })
                .unwrap_or_else(|| "deepseek-v4-flash".to_string());
            let script = deepseek_model_injection_script(
                &locale,
                &selected_model,
                handled_refresh_token.as_deref(),
            );
            let targets = query_targets(&client, port).await;
            let mut pending_model = None;
            for target in &targets {
                if let Some(result) = evaluate_target(target, &script).await {
                    if let Some(model) = result.selected_model {
                        if handled_refresh_token.as_deref() != Some(model.as_str()) {
                            pending_model = Some(model);
                        }
                    }
                }
            }
            if let (Some(account_id), Some(model)) = (account_id, pending_model) {
                let profile_dir = profile_dir.clone();
                let applied_model = model.clone();
                match tauri::async_runtime::spawn_blocking(move || {
                    crate::modules::codex_account::apply_deepseek_cdp_startup_model(
                        &account_id,
                        &applied_model,
                        &profile_dir,
                    )
                })
                .await
                {
                    Ok(Ok(_)) => {
                        handled_refresh_token = Some(model.clone());
                        logger::log_info(&format!(
                            "[Codex App Injection] DeepSeek CDP 已切换启动模型: model={}",
                            model
                        ));
                    }
                    Ok(Err(error)) => logger::log_warn(&format!(
                        "[Codex App Injection] DeepSeek CDP 切换模型失败: {}",
                        error
                    )),
                    Err(error) => logger::log_warn(&format!(
                        "[Codex App Injection] DeepSeek CDP 切换模型任务异常: {}",
                        error
                    )),
                }
            }
            tokio::time::sleep(INJECTION_INTERVAL).await;
            continue;
        }
        if refresh_finished || last_quota_at.elapsed() >= QUOTA_REFRESH_INTERVAL {
            let gateway = read_profile_gateway_config(&profile_dir);
            if let Some(value) = fetch_quota(&client, gateway.as_ref()).await {
                quota = value;
            }
            if refreshed_empty_pool {
                quota = QuotaResponse::empty_pool();
            }
            last_quota_at = Instant::now();
        }
        let gateway = read_profile_gateway_config(&profile_dir);
        let provider_name = gateway
            .as_ref()
            .map(|value| value.provider_name.as_str())
            .unwrap_or("Codex");
        let script = injection_script(
            provider_name,
            &quota,
            &locale,
            !refresh_tasks.is_empty(),
            handled_refresh_token.as_deref(),
        );
        let targets = query_targets(&client, port).await;
        let mut refresh_request_token = None;
        for target in &targets {
            if let Some(result) = evaluate_target(target, &script).await {
                if let Some(token) = result.refresh_request_token {
                    if handled_refresh_token.as_deref() != Some(token.as_str()) {
                        refresh_request_token = Some(token);
                    }
                }
            }
        }
        if let Some(token) = refresh_request_token.filter(|_| refresh_tasks.is_empty()) {
            handled_refresh_token = Some(token);
            let refreshing_script = injection_script(
                provider_name,
                &quota,
                &locale,
                true,
                handled_refresh_token.as_deref(),
            );
            for target in &targets {
                let _ = evaluate_target(target, &refreshing_script).await;
            }
            let app = app.clone();
            refresh_tasks.spawn(async move { run_quota_refresh_singleflight(&app).await });
        }
        tokio::time::sleep(INJECTION_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        app_server_auth_file_snapshot, auth_diagnostic_error_signal, auth_diagnostic_observation,
        build_launch_args, cdp_auth_signal, cdp_body_preview, cdp_console_auth_signal,
        deepseek_model_injection_script, has_login_route_markers, injection_script,
        is_auth_diagnostic_url, is_codex_app_target, is_official_login_route,
        is_safe_cdp_websocket_url, refresh_request_token_from_cdp_response,
        remote_debugging_port_from_command_line, sanitize_cdp_headers,
        selected_model_from_cdp_response, should_capture_cdp_response_body, supports_bind_account,
        AuthPageSnapshot, CdpTarget, QuotaPlanSummary, QuotaResponse, AUTH_DIAGNOSTIC_SCRIPT,
    };
    use crate::models::codex::{CodexAccount, CodexTokens};
    use crate::modules::codex_account::{
        compare_official_oauth_identity, CodexOfficialOAuthIdentity,
        CodexOfficialOAuthIdentityMatch,
    };
    use serde_json::json;

    #[test]
    fn disabled_keeps_launch_args() {
        let args = vec!["--foo".to_string(), "bar".to_string()];
        let plan = build_launch_args(&args, false).expect("plan");
        assert_eq!(plan.args, args);
        assert_eq!(plan.port, None);
    }

    #[test]
    fn enabled_replaces_debug_flags_with_loopback_port() {
        let args = vec![
            "--remote-debugging-port=9333".to_string(),
            "--remote-debugging-address".to_string(),
            "0.0.0.0".to_string(),
            "--foo".to_string(),
        ];
        let plan = build_launch_args(&args, true).expect("plan");
        assert!(plan.port.is_some());
        assert!(!plan.args.iter().any(|value| value.contains("9333")));
        assert!(plan
            .args
            .iter()
            .any(|value| value == "--remote-debugging-address=127.0.0.1"));
    }

    #[test]
    fn only_api_service_binding_supports_quota_injection() {
        assert!(supports_bind_account(Some("__api_service__")));
        assert!(!supports_bind_account(Some("api-key-account")));
        assert!(!supports_bind_account(Some(
            "__provider_gateway__:custom-provider"
        )));
        assert!(!supports_bind_account(None));
    }

    #[test]
    fn deepseek_script_controls_official_picker_and_returns_pending_model() {
        let script = deepseek_model_injection_script("zh-cn", "deepseek-v4-flash", None);
        assert!(script.contains("deepseek-v4-flash"));
        assert!(script.contains("deepseek-v4-pro"));
        assert!(script.contains("gpt-5.5"));
        assert!(script.contains("gpt-5.4"));
        assert!(script.contains("item.model = official"));
        assert!(script.contains("list-models-for-host"));
        assert!(script.contains("model/list"));
        assert!(script.contains("deepseek-official-picker"));
        assert!(script.contains("const reasoningLevels = [\"low\", \"high\", \"max\"]"));
        assert!(script.contains("supported_reasoning_levels = levels"));
        assert!(script.contains("supportedReasoningEfforts = levels"));
        assert!(!script.contains("[\"low\", \"medium\", \"high\", \"xhigh\"]"));
        assert!(script.contains("staleBar"));
        assert!(!script.contains("data-cockpit-deepseek-model"));
        assert!(script.contains("pendingSelectedModel"));
        let parsed = selected_model_from_cdp_response(&json!({
            "result": { "result": { "value": { "selectedModel": "deepseek-v4-pro" } } }
        }));
        assert_eq!(parsed.as_deref(), Some("deepseek-v4-pro"));
    }

    #[test]
    fn parses_remote_debugging_port_from_running_process_command_line() {
        assert_eq!(
            remote_debugging_port_from_command_line(
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT --remote-debugging-address=127.0.0.1 --remote-debugging-port=64404"
            ),
            Some(64404)
        );
        assert_eq!(
            remote_debugging_port_from_command_line(
                r#"C:\Program Files\ChatGPT\ChatGPT.exe --remote-debugging-port "9333""#
            ),
            Some(9333)
        );
    }

    #[test]
    fn rejects_missing_or_invalid_remote_debugging_port() {
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-address=127.0.0.1"),
            None
        );
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-port=0"),
            None
        );
        assert_eq!(
            remote_debugging_port_from_command_line("ChatGPT --remote-debugging-port=70000"),
            None
        );
    }

    #[test]
    fn custom_script_renders_weekly_and_optional_five_hour_fields() {
        let script = injection_script(
            "Provider",
            &QuotaResponse {
                weekly_remaining_percent: Some(1387),
                five_hour_remaining_percent: Some(100),
                account_count: Some(14),
                available_account_count: Some(12),
                abnormal_account_count: Some(2),
                cooldown_account_count: Some(0),
                plans: vec![QuotaPlanSummary {
                    plan: "PLUS".to_string(),
                    count: 14,
                    weekly_remaining_percent: Some(1387),
                    five_hour_remaining_percent: Some(100),
                }],
            },
            "zh-cn",
            false,
            None,
        );
        assert!(script.contains("const accountPoolLabel = \"账号\""));
        assert!(script.contains("const accountCount = 14"));
        assert!(script.contains("const weeklyLabel = \"周\""));
        assert!(script.contains("const fiveHourLabel = \"5h\""));
        assert!(script.contains("data-cockpit-quota-footer"));
        assert!(script.contains("document.body.appendChild(host)"));
        assert!(script.contains("position:fixed"));
        assert!(script.contains("footerRect.left + footerRect.width / 2"));
        assert!(script.contains("permissionsRect.top + permissionsRect.height / 2"));
        assert!(script.contains("justify-content:center"));
        assert!(script.contains("host.innerHTML !== nextHtml"));
        assert!(script.contains("mutations.every"));
        assert!(script.contains("new ResizeObserver"));
        assert!(script.contains("window.addEventListener('resize'"));
        assert!(script.contains("requestAnimationFrame"));
        assert!(script.contains("root.render()"));
        assert!(script.contains("data-cockpit-quota-refresh"));
        assert!(script.contains("data-cockpit-quota-open"));
        assert!(script.contains("data-cockpit-quota-details"));
        assert!(script.contains("data-cockpit-quota-close"));
        assert!(script.contains("const plans = [{\"plan\":\"PLUS\",\"count\":14"));
        assert!(script.contains("const availableText = \"可用 12/14\""));
        assert!(script.contains("const issueText = \"异常 2 · 冷却 0\""));
        assert!(script.contains("var(--color-token-main-surface-primary"));
        assert!(script.contains("var(--color-token-text-secondary"));
        assert!(script.contains("const planColor"));
        assert!(script.contains("background:#10b981"));
        assert!(script.contains("background:#3b82f6"));
        assert!(script.contains("root.quotaDetailsOpen"));
        assert!(script.contains("details.innerHTML !== detailsHtml"));
        assert!(script.contains("details.contains(mutation.target)"));
        assert!(!script.contains("modal-overlay"));
        assert!(!script.contains("common.shared.refreshQuota"));
        assert!(script.contains("root.refreshRequestToken"));
        assert!(script.contains("cockpit-quota-spin"));
        assert!(script.contains("pointer-events:auto"));
        assert!(script.contains("root.hostHeartbeatAt = Date.now()"));
        assert!(script.contains("root.watchdogTimer"));
        assert!(script.contains("hostHeartbeatTimeoutMs = 8000"));
        assert!(script.contains("root.refreshRequestToken = null"));
        assert!(!script.contains("footer.appendChild(host)"));
        assert!(!script.contains("justify-content:flex-end"));
        assert!(!script.contains("grid-row:3"));
        assert!(!script.contains("min-height:18px"));
        assert!(!script.contains("data-cockpit-fast-mode"));
        assert!(!script.contains("Debugger"));
    }

    #[test]
    fn five_hour_only_quota_is_not_relabelled_as_weekly() {
        let script = injection_script(
            "Provider",
            &QuotaResponse {
                weekly_remaining_percent: None,
                five_hour_remaining_percent: Some(63),
                account_count: Some(2),
                ..QuotaResponse::default()
            },
            "zh-cn",
            false,
            None,
        );
        assert!(script.contains("const weeklyPercent = null"));
        assert!(script.contains("const fiveHourPercent = 63"));
    }

    #[test]
    fn empty_pool_quota_renders_zero_account_and_windows() {
        let quota = QuotaResponse::empty_pool();
        assert_eq!(quota.account_count, Some(0));
        assert_eq!(quota.five_hour_remaining_percent, Some(0));
        assert_eq!(quota.weekly_remaining_percent, Some(0));

        let script = injection_script("Provider", &quota, "zh-cn", false, None);
        assert!(script.contains("const accountCount = 0"));
        assert!(script.contains("const fiveHourPercent = 0"));
        assert!(script.contains("const weeklyPercent = 0"));
        assert!(script.contains("accountCount >= 0"));
    }

    #[test]
    fn empty_pool_response_normalizes_missing_window_values_to_zero() {
        let quota = QuotaResponse {
            weekly_remaining_percent: None,
            five_hour_remaining_percent: None,
            account_count: Some(0),
            ..QuotaResponse::default()
        }
        .normalize_empty_pool();

        assert_eq!(quota.account_count, Some(0));
        assert_eq!(quota.five_hour_remaining_percent, Some(0));
        assert_eq!(quota.weekly_remaining_percent, Some(0));
    }

    #[test]
    fn cdp_response_extracts_refresh_request_token() {
        let response = json!({
            "id": 1,
            "result": {
                "result": {
                    "type": "object",
                    "value": {"refreshRequestToken": "request-123"}
                }
            }
        });
        assert_eq!(
            refresh_request_token_from_cdp_response(&response).as_deref(),
            Some("request-123")
        );
        assert!(refresh_request_token_from_cdp_response(&json!({"id": 1})).is_none());
    }

    #[test]
    fn auth_diagnostic_cdp_websocket_must_be_loopback_and_same_port() {
        assert!(is_safe_cdp_websocket_url(
            "ws://127.0.0.1:9333/devtools/page/1",
            9333
        ));
        assert!(is_safe_cdp_websocket_url(
            "ws://[::1]:9333/devtools/page/1",
            9333
        ));
        assert!(!is_safe_cdp_websocket_url(
            "ws://192.168.1.2:9333/devtools/page/1",
            9333
        ));
        assert!(!is_safe_cdp_websocket_url(
            "ws://127.0.0.1:9444/devtools/page/1",
            9333
        ));
    }

    #[test]
    fn auth_diagnostic_observation_only_reports_safe_page_state() {
        let target = CdpTarget {
            target_id: "page-1".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: Some("ws://127.0.0.1:9333/devtools/page/1".to_string()),
        };
        let snapshot = AuthPageSnapshot {
            route: "/login".to_string(),
            title: "ChatGPT".to_string(),
            ready_state: "complete".to_string(),
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[target], Some(snapshot));

        assert!(observed.cdp_available);
        assert_eq!(observed.target_count, 1);
        assert_eq!(observed.route, "/login");
        assert!(observed.login_signal());
    }

    #[test]
    fn auth_diagnostic_requires_route_or_complete_login_ui_signal() {
        assert!(!AUTH_DIAGNOSTIC_SCRIPT.contains("innerText"));
        assert!(!AUTH_DIAGNOSTIC_SCRIPT.contains("loginText"));
        assert!(is_official_login_route("/login"));
        assert!(is_official_login_route("/login/"));
        assert!(!is_official_login_route("/auth/login"));
        assert!(!is_official_login_route("/index.html"));
        assert!(has_login_route_markers(&[
            "login_title".to_string(),
            "login_primary_action".to_string(),
        ]));
        assert!(!has_login_route_markers(&["login_title".to_string()]));
        let target = CdpTarget {
            target_id: "page-1".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: Some("ws://127.0.0.1:9333/devtools/page/1".to_string()),
        };
        let normal_page = AuthPageSnapshot {
            route: "/index.html".to_string(),
            title: "Sign in to ChatGPT".to_string(),
            ready_state: "complete".to_string(),
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[target], Some(normal_page));
        assert!(!observed.login_signal());

        let login_page = AuthPageSnapshot {
            route: "/index.html".to_string(),
            login_ui_signal: true,
            login_ui_markers: vec![
                "login_title".to_string(),
                "login_primary_action".to_string(),
            ],
            ..AuthPageSnapshot::default()
        };
        let observed = auth_diagnostic_observation(&[], Some(login_page));
        assert!(observed.login_signal());
        assert!(observed.login_ui_signal);
    }

    #[test]
    fn auth_diagnostic_ignores_non_codex_targets() {
        let codex_target = CdpTarget {
            target_id: "codex".to_string(),
            target_type: "page".to_string(),
            url: "app://-/index.html".to_string(),
            websocket_url: None,
        };
        let external_target = CdpTarget {
            target_id: "external".to_string(),
            target_type: "page".to_string(),
            url: "https://chatgpt.com/auth/login".to_string(),
            websocket_url: None,
        };
        assert!(is_codex_app_target(&codex_target));
        assert!(!is_codex_app_target(&external_target));
    }

    #[test]
    fn auth_diagnostic_profile_identity_uses_account_id_before_email() {
        let mut account = CodexAccount::new(
            "local-account".to_string(),
            "expected@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.account_id = Some("chatgpt-account".to_string());
        account.user_id = Some("chatgpt-user".to_string());

        let matching = CodexOfficialOAuthIdentity {
            email: "different@example.com".to_string(),
            user_id: Some("chatgpt-user".to_string()),
            account_id: Some("chatgpt-account".to_string()),
            organization_id: None,
        };
        assert_eq!(
            compare_official_oauth_identity(&matching, &account),
            CodexOfficialOAuthIdentityMatch::Matched
        );

        let mismatched = CodexOfficialOAuthIdentity {
            account_id: Some("other-account".to_string()),
            ..matching.clone()
        };
        assert_eq!(
            compare_official_oauth_identity(&mismatched, &account),
            CodexOfficialOAuthIdentityMatch::Mismatched
        );

        let incomplete = CodexOfficialOAuthIdentity {
            email: "expected@example.com".to_string(),
            user_id: None,
            account_id: None,
            organization_id: None,
        };
        assert_eq!(
            compare_official_oauth_identity(&incomplete, &account),
            CodexOfficialOAuthIdentityMatch::Unknown
        );
    }

    #[test]
    fn auth_network_diagnostics_redact_credentials_and_queries() {
        assert_eq!(
            super::sanitize_cdp_url(
                "https://auth.openai.com/oauth/token?client_secret=secret&code=oauth-code"
            ),
            "https://auth.openai.com/oauth/token?<redacted-query>"
        );
        let headers = sanitize_cdp_headers(Some(&json!({
            "authorization": "Bearer secret",
            "cookie": "session=secret",
            "x-api-key": "secret-api-key",
            "session_token": "secret-session-token",
            "x-request-id": "request-1"
        })));
        assert_eq!(headers["authorization"], "<redacted>");
        assert_eq!(headers["cookie"], "<redacted>");
        assert_eq!(headers["x-api-key"], "<redacted>");
        assert_eq!(headers["session_token"], "<redacted>");
        assert_eq!(headers["x-request-id"], "request-1");
    }

    #[test]
    fn auth_network_diagnostics_extract_redacted_json_error_body() {
        let body = json!({
            "error": "refresh_token_reused",
            "access_token": "eyJsecret",
            "message": "reauth required"
        });
        let response = json!({
            "result": {"body": serde_json::to_string(&body).expect("body")}
        });
        let preview = cdp_body_preview(&response).expect("preview");
        assert!(preview.contains("refresh_token_reused"));
        assert!(preview.contains("<redacted>"));
        assert!(!preview.contains("eyJsecret"));
    }

    #[test]
    fn auth_network_diagnostics_matches_official_relogin_signals() {
        let invalid_refresh = json!({
            "result": {"body": r#"{"error":{"code":"invalid_refresh_token","message":"Invalid refresh token."}}"#}
        });
        assert_eq!(
            cdp_auth_signal(&invalid_refresh),
            Some("invalid_refresh_token")
        );

        let auth_error = json!({
            "result": {"body": r#"{"data":{"reason":"cloudRequirements","errorCode":"Auth"}}"#}
        });
        assert_eq!(cdp_auth_signal(&auth_error), Some("auth_error_code"));

        let relogin = json!({
            "result": {"body": r#"{"data":{"reason":"cloudConfigBundle","action":"relogin"}}"#}
        });
        assert_eq!(cdp_auth_signal(&relogin), Some("relogin_action"));

        let normal = json!({
            "result": {"body": r#"{"data":{"reason":"cloudRequirements"}}"#}
        });
        assert_eq!(cdp_auth_signal(&normal), None);
    }

    #[test]
    fn auth_network_diagnostics_marks_relevant_endpoints() {
        assert!(is_auth_diagnostic_url(
            "https://chatgpt.com/backend-api/cloudRequirements"
        ));
        assert!(is_auth_diagnostic_url(
            "https://auth.openai.com/oauth/token"
        ));
        assert!(!is_auth_diagnostic_url("https://example.com/assets/app.js"));
    }

    #[test]
    fn auth_network_diagnostics_captures_auth_body_even_for_success_status() {
        assert!(should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/cloudRequirements",
            200
        ));
        assert!(should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/conversations",
            401
        ));
        assert!(!should_capture_cdp_response_body(
            "https://chatgpt.com/backend-api/conversations",
            200
        ));
    }

    #[test]
    fn auth_diagnostic_extracts_only_known_official_error_signals() {
        assert_eq!(
            auth_diagnostic_error_signal("401 Unauthorized: invalid_refresh_token"),
            Some("invalid_refresh_token")
        );
        assert_eq!(
            auth_diagnostic_error_signal("Invalid refresh token."),
            Some("invalid_refresh_token")
        );
        assert_eq!(
            auth_diagnostic_error_signal("auth_status_result nullReason=auth_token_missing"),
            Some("auth_token_missing")
        );
        assert_eq!(
            auth_diagnostic_error_signal("no_token_attached"),
            Some("no_token_attached")
        );
        assert_eq!(
            auth_diagnostic_error_signal("requiresOpenaiAuth=true"),
            Some("requiresOpenaiAuth")
        );
        assert_eq!(auth_diagnostic_error_signal("ordinary 401 response"), None);
        assert_eq!(auth_diagnostic_error_signal("ERR_SSL_PROTOCOL_ERROR"), None);
    }

    #[test]
    fn auth_diagnostic_reads_console_argument_values_without_logging_them() {
        let params = json!({
            "args": [
                {"type": "string", "value": "app_server_connection.auth_status_result"},
                {"type": "string", "value": "code=invalid_refresh_token"}
            ]
        });
        assert_eq!(
            cdp_console_auth_signal(&params),
            Some("invalid_refresh_token")
        );

        let nested = json!({
            "args": [{"type": "object", "description": "{\"requiresOpenaiAuth\":true}"}]
        });
        assert_eq!(cdp_console_auth_signal(&nested), Some("requiresOpenaiAuth"));
        assert_eq!(
            cdp_console_auth_signal(&json!({"args": [{"value": "Bearer eyJsecret"}]})),
            None
        );
    }

    #[test]
    fn app_server_auth_snapshot_never_contains_auth_contents() {
        let directory = std::env::temp_dir().join(format!(
            "cockpit-codex-app-server-diagnostic-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("create diagnostic directory");
        std::fs::write(
            directory.join("auth.json"),
            r#"{"access_token":"secret-access","refresh_token":"secret-refresh"}"#,
        )
        .expect("write diagnostic auth");

        let snapshot = app_server_auth_file_snapshot(&directory);
        assert!(snapshot.starts_with("exists=true,size="));
        assert!(snapshot.contains("sha256="));
        assert!(!snapshot.contains("secret-access"));
        assert!(!snapshot.contains("secret-refresh"));
        let _ = std::fs::remove_dir_all(directory);
    }
}
