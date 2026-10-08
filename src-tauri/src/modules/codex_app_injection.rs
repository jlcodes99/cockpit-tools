//! Codex/ChatGPT renderer 的 Cockpit Tools 额度显示注入。
//!
//! 两种注入共用同一套 loopback CDP 通道：
//! - API 服务绑定：账号数、周额度、5h 额度；
//! - DeepSeek 账号绑定（网关列出 / CDP 注入 / 直连官方）：该账号的余额。
//!
//! 该模块只连接实例自己的 loopback CDP 端口，不修改官方 app.asar，
//! 也不修改官方额度或速度逻辑。额度以独立的小字段显示在 composer 操作栏下方。

use crate::commands::codex::{
    codex_model_provider_deepseek_balance_url, query_deepseek_balance_snapshot,
    DeepSeekBalanceSnapshot,
};
use crate::models::codex::CodexAccount;
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
/// DeepSeek 余额变化很慢，保持较长的刷新间隔，避免无谓的上游请求。
const DEEPSEEK_BALANCE_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const DEEPSEEK_BALANCE_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
/// 绑定账号有变更（模型列表、接入方式等）时最多延迟这么久生效，避免每 2 秒读取账号文件。
const DEEPSEEK_ACCOUNT_CACHE_TTL: Duration = Duration::from_secs(15);
/// 页面读不到当前模型时，用实例 config.toml 兜底刷新的最小间隔，避免每轮都读文件。
const MODEL_RESYNC_INTERVAL: Duration = Duration::from_secs(10);
/// 新文档脚本按「脚本种类 | CDP target」记录，同一实例可同时运行多套注入。
const QUOTA_SCRIPT_KIND: &str = "api-service-quota";
const DEEPSEEK_MODEL_SCRIPT_KIND: &str = "deepseek-model-picker";
const DEEPSEEK_BALANCE_SCRIPT_KIND: &str = "deepseek-balance";
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

/// 同一个 CDP target 可能同时跑多套注入脚本（DeepSeek 模型列表 + 余额），
/// 因此“已安装新文档脚本”按 `脚本标识|websocket` 记录，避免互相顶掉。
fn should_install_new_document_script(install_key: &str) -> bool {
    let Ok(installed) = new_document_scripts().lock() else {
        return true;
    };
    !installed.contains(install_key)
}

fn mark_new_document_script_installed(install_key: &str) {
    if let Ok(mut installed) = new_document_scripts().lock() {
        installed.insert(install_key.to_string());
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

/// 绑定账号是否选择了「CDP 注入」（官方 DeepSeek 与第三方供应商同一语义）。
pub fn bind_uses_cdp_model_injection(bind_account_id: Option<&str>) -> bool {
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
        crate::modules::codex_account::account_uses_cdp_model_injection(&account)
    })
}

/// 绑定账号（含 `__provider_gateway__:` 前缀）对应的 API Key 供应商账号。
fn bound_api_key_account(bind_account_id: Option<&str>) -> Option<CodexAccount> {
    let account_id = bind_account_id_value(bind_account_id)?;
    let account = crate::modules::codex_account::load_account(&account_id)?;
    account.is_api_key_auth().then_some(account)
}

/// 绑定账号（含 `__provider_gateway__:` 前缀）对应的 DeepSeek 账号。
///
/// DeepSeek 的余额注入与接入方式无关：网关列出、CDP 注入、直连官方都要显示，
/// 因此这里只解析绑定账号本身，不看 `api_instance_access_mode`。
fn deepseek_bound_account(bind_account_id: Option<&str>) -> Option<CodexAccount> {
    let account_id = bind_account_id_value(bind_account_id)?;
    let account = crate::modules::codex_account::load_account(&account_id)?;
    crate::modules::codex_account::is_deepseek_account(&account).then_some(account)
}

/// DeepSeek 余额接口地址；非官方 host（第三方中转）返回 `None`，此时不注入余额。
fn deepseek_balance_endpoint(account: &CodexAccount) -> Option<String> {
    let base_url = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    codex_model_provider_deepseek_balance_url(base_url)
        .ok()
        .flatten()
}

fn bind_uses_deepseek_balance_injection(bind_account_id: Option<&str>) -> bool {
    deepseek_bound_account(bind_account_id)
        .is_some_and(|account| deepseek_balance_endpoint(&account).is_some())
}

pub fn should_enable_injection(bind_account_id: Option<&str>) -> bool {
    (enabled_for_app() && supports_bind_account(bind_account_id))
        || bind_uses_cdp_model_injection(bind_account_id)
        || (enabled_for_app() && bind_uses_deepseek_balance_injection(bind_account_id))
}

/// 额度注入、DeepSeek 余额、认证页面观察和 DeepSeek 模型适配都依赖实例自己的 loopback CDP。
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InjectionBadgeKind {
    CodexQuota,
    GrokQuota,
    DeepSeekBalance,
}

impl InjectionBadgeKind {
    fn as_js_str(self) -> &'static str {
        match self {
            Self::CodexQuota => "codex",
            Self::GrokQuota => "grok",
            Self::DeepSeekBalance => "deepseek",
        }
    }
}

fn read_profile_selected_model(profile_dir: &Path) -> Option<String> {
    let config_text = fs::read_to_string(profile_dir.join("config.toml")).ok()?;
    let document = config_text.parse::<Document>().ok()?;
    normalize_injection_model_slug(document.get("model")?.as_str().unwrap_or(""))
}

fn normalize_injection_model_slug(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("deepseek") {
        return Some(
            if lower.contains("vision") {
                "deepseek-v4-flash-vision-exp"
            } else if lower.contains("pro") {
                "deepseek-v4-pro"
            } else if lower.contains("v4.1") || lower.contains("v4-1") {
                "deepseek-flash"
            } else if lower.contains("v4-flash") || lower.contains("v4.flash") {
                "deepseek-v4-flash"
            } else {
                "deepseek-flash"
            }
            .to_string(),
        );
    }
    if lower.contains("grok") {
        for official in crate::modules::codex_account::GROK_CODEX_MODELS {
            let needle = official.to_ascii_lowercase();
            let compact = needle.replace('.', "");
            if lower.contains(&needle) || lower.replace('.', "").contains(&compact) {
                return Some((*official).to_string());
            }
        }
        return Some((*crate::modules::codex_account::GROK_CODEX_MODELS.first()?).to_string());
    }
    let slug = lower
        .split_whitespace()
        .next()
        .unwrap_or(&lower)
        .trim_matches(|ch: char| {
            !ch.is_ascii_alphanumeric() && ch != '-' && ch != '.' && ch != '_'
        });
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
}

fn injection_badge_kind_for_model(model: Option<&str>) -> InjectionBadgeKind {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return InjectionBadgeKind::CodexQuota;
    };
    let lower = model.to_ascii_lowercase();
    if lower.contains("deepseek") {
        InjectionBadgeKind::DeepSeekBalance
    } else if lower.contains("grok") {
        InjectionBadgeKind::GrokQuota
    } else {
        InjectionBadgeKind::CodexQuota
    }
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

/// API 服务账号池里可查官方余额的 DeepSeek 账号汇总（按币种合并成一行）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct InjectionBalanceLine {
    currency: Option<String>,
    total_balance: f64,
}

/// API 服务账号池里的 Grok 平台账号额度（只含 GrokBuild），按产品合并成一行。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct InjectionGrokQuotaLine {
    product: String,
    count: i64,
    remaining_percent: i64,
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

/// 查询绑定账号的 DeepSeek 余额；失败时保留上一次的有效快照，不清空已显示的额度。
async fn fetch_deepseek_balance(
    client: &Client,
    account: &CodexAccount,
) -> Option<DeepSeekBalanceSnapshot> {
    let url = deepseek_balance_endpoint(account)?;
    let api_key = account
        .openai_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match query_deepseek_balance_snapshot(client, &url, api_key).await {
        Ok(snapshot) => Some(snapshot),
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex App Injection] DeepSeek 余额查询失败: {}",
                error
            ));
            None
        }
    }
}

/// API 服务账号池里的 Grok 账号额度汇总：只取 GrokBuild，多账号取最优剩余额度。
///
/// 数据来自 Cockpit 本地保存的 Grok 平台账号额度（账号保活会定期刷新），
/// 因此这里不发额外网络请求；池里没有 Grok 账号时返回空列表，弹框不显示该行。
async fn api_service_grok_quota_lines() -> Vec<InjectionGrokQuotaLine> {
    let Ok(state) = codex_local_access::get_local_access_state().await else {
        return Vec::new();
    };
    let Some(collection) = state.collection else {
        return Vec::new();
    };
    let mut grouped: std::collections::BTreeMap<String, (i64, i64)> =
        std::collections::BTreeMap::new();
    for account_id in collection.account_ids {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        if !codex_account::is_grok_upstream_provider(&account) {
            continue;
        }
        let Some(grok_account_id) = account
            .upstream_grok_account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some(grok_account) = crate::modules::grok_account::load_account(grok_account_id) else {
            continue;
        };
        let Some(quota) = grok_account.quota else {
            continue;
        };
        for product in quota.products {
            let normalized = product
                .product
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase();
            // 卡片只展示 GrokBuild（其余产品额度太杂，会把弹框撑长）。
            if normalized != "grokbuild" {
                continue;
            }
            let used = product.used;
            let total = product.total;
            let remaining = product.remaining;
            let used_percent = product.usage_percent.or_else(|| {
                match (used, total, remaining) {
                    (Some(used), Some(total), _) if total > 0.0 => Some(used / total * 100.0),
                    (_, Some(total), Some(remaining)) if total > 0.0 => {
                        Some((total - remaining) / total * 100.0)
                    }
                    _ => None,
                }
            });
            let Some(used_percent) = used_percent.filter(|value| value.is_finite()) else {
                continue;
            };
            let remaining_percent = (100.0 - used_percent).round().clamp(0.0, 100.0) as i64;
            let entry = grouped.entry(product.product).or_insert((0, remaining_percent));
            entry.0 += 1;
            entry.1 = entry.1.max(remaining_percent);
        }
    }
    grouped
        .into_iter()
        .map(|(product, (count, remaining_percent))| InjectionGrokQuotaLine {
            product,
            count,
            remaining_percent,
        })
        .collect()
}

/// API 服务账号池里的 DeepSeek 官方余额汇总（按币种合并成一行），供点击弹框显示。
///
/// - 池里没有可查余额的账号：返回空列表，注入脚本隐藏余额行；
/// - 池里账号全部查询失败：返回 `None`，保留上一次的有效结果。
async fn fetch_api_service_balance_lines(client: &Client) -> Option<Vec<InjectionBalanceLine>> {
    let state = codex_local_access::get_local_access_state().await.ok()?;
    let collection = state.collection?;
    let mut accounts = Vec::new();
    for account_id in collection.account_ids {
        let Some(account) = codex_account::load_account(&account_id) else {
            continue;
        };
        if deepseek_balance_endpoint(&account).is_some() {
            accounts.push(account);
        }
    }
    if accounts.is_empty() {
        return Some(Vec::new());
    }
    let mut tasks = JoinSet::new();
    for account in accounts {
        let client = client.clone();
        tasks.spawn(async move { fetch_deepseek_balance(&client, &account).await });
    }
    let mut totals: Vec<InjectionBalanceLine> = Vec::new();
    let mut any_success = false;
    while let Some(result) = tasks.join_next().await {
        let Ok(Some(snapshot)) = result else {
            continue;
        };
        let Some(total_balance) = snapshot.total_balance else {
            continue;
        };
        any_success = true;
        match totals
            .iter_mut()
            .find(|line| line.currency == snapshot.currency)
        {
            Some(line) => line.total_balance += total_balance,
            None => totals.push(InjectionBalanceLine {
                currency: snapshot.currency,
                total_balance,
            }),
        }
    }
    any_success.then_some(totals)
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
    payload: &serde_json::Value,
    handled_selected_model: Option<&str>,
) -> String {
    let payload =
        serde_json::to_string(payload).unwrap_or_else(|_| "{\"models\":[]}".to_string());
    let handled =
        serde_json::to_string(&handled_selected_model).unwrap_or_else(|_| "null".to_string());
    format!(
        r#"(() => {{
      const payload = {payload};
      const models = Array.isArray(payload.models)
        ? payload.models.filter((item) => item && typeof item.id === "string" && item.id.trim() !== "")
        : [];
      const modelIds = models.map((item) => String(item.id));
      const modelMeta = {{}};
      for (const item of models) modelMeta[String(item.id).trim().toLowerCase()] = item;
      const shellToUpstream = {{}};
      const rawShells = payload.shells && typeof payload.shells === "object" ? payload.shells : {{}};
      for (const [shell, upstream] of Object.entries(rawShells)) {{
        const key = String(shell || "").trim().toLowerCase();
        const value = String(upstream || "").trim();
        if (key && value) shellToUpstream[key] = value;
      }}
      const selectedModel =
        typeof payload.selectedModel === "string" && payload.selectedModel.trim() !== ""
          ? payload.selectedModel.trim()
          : (modelIds[0] || "");
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
      const displayName = {{}};
      for (const item of models) displayName[String(item.id)] = item.name || String(item.id);
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
      const applyVisionMetadata = (item, official) => {{
        if (!item || typeof item !== "object") return;
        const meta = modelMeta[String(official || "").trim().toLowerCase()];
        const supportsImage = Boolean(meta && meta.vision);
        const modalities = supportsImage ? ["text", "image"] : ["text"];
        // Same dual-shape rule as reasoning metadata: the app reads one of these
        // depending on release, and a missing value would hide image input.
        item.inputModalities = modalities;
        item.input_modalities = modalities;
        item.supportsImageDetailOriginal = supportsImage;
        item.supports_image_detail_original = supportsImage;
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
        if (!slug) return null;
        if (shellToUpstream[slug]) return shellToUpstream[slug];
        return modelMeta[slug] ? String(modelMeta[slug].id) : null;
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
        applyVisionMetadata(item, official);
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
        applyVisionMetadata(item, official);
        return true;
      }};
      // 只认真正的模型描述符：每一项都必须有 slug，并带模型展示字段。
      // 队列、线程、项目列表同样是数组，宽松判定会把它们当成模型列表改写。
      const isModelArray = (value) => Array.isArray(value) && value.length > 0 && value.every((item) => {{
        if (!item || typeof item !== "object") return false;
        if (typeof item.slug !== "string" || item.slug.trim() === "") return false;
        return (
          typeof item.display_name === "string" ||
          typeof item.displayName === "string" ||
          typeof item.description === "string"
        );
      }});
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
        for (const official of modelIds) {{
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
      // 只有用户在 Codex 里显式切模型（默认模型配置、config 写入）才算切换；
      // 发消息时的 turn/start、thread/start 也会带 model 字段，那是当次请求用的模型，
      // 不能据此写配置——turn 进行中改后端状态会让服务端队列失效。
      const switchMethods = {{ "set-default-model-config-for-host": true, "config/value/write": true }};
      const rewriteOutgoing = (method, params) => {{
        if (!params || typeof params !== "object") return;
        const reportable = switchMethods[method] === true;
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
          if (reportable) reportSelected(params.model);
          const upstream = toUpstream(params.model);
          if (upstream) params.model = upstream;
        }}
        if (params.params && typeof params.params === "object" && params.params.model) {{
          if (reportable) reportSelected(params.params.model);
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
        for (const slug of modelIds) if (!available.includes(slug)) available.push(slug);
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
      const pendingMeta = typeof root.pendingSelectedModel === "string"
        ? modelMeta[normalize(root.pendingSelectedModel)]
        : null;
      const pendingSelectedModel = pendingMeta
        && pendingMeta.id !== selectedModel
        && pendingMeta.id !== handledSelectedModel
        ? String(pendingMeta.id)
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
    balance_lines: &[InjectionBalanceLine],
    grok_lines: &[InjectionGrokQuotaLine],
    badge_kind: InjectionBadgeKind,
    fallback_model: Option<&str>,
) -> String {
    let provider = serde_json::to_string(provider_name).unwrap_or_else(|_| "\"Codex\"".to_string());
    let weekly = quota.weekly_remaining_percent;
    let five_hour = quota.five_hour_remaining_percent;
    let account_count = quota.account_count;
    let available_account_count = quota.available_account_count.or(account_count);
    let abnormal_account_count = quota.abnormal_account_count.unwrap_or(0);
    let cooldown_account_count = quota.cooldown_account_count.unwrap_or(0);
    let plans = serde_json::to_string(&quota.plans).unwrap_or_else(|_| "[]".to_string());
    let grok_lines_value = serde_json::to_string(grok_lines).unwrap_or_else(|_| "[]".to_string());
    let grok_badge_line = grok_lines.iter().max_by_key(|line| line.remaining_percent);
    let grok_badge_percent =
        serde_json::to_string(&grok_badge_line.map(|line| line.remaining_percent))
            .unwrap_or_else(|_| "null".to_string());
    let grok_badge_product =
        serde_json::to_string(&grok_badge_line.map(|line| line.product.clone()))
            .unwrap_or_else(|_| "null".to_string());
    let badge_kind_value =
        serde_json::to_string(badge_kind.as_js_str()).unwrap_or_else(|_| "\"codex\"".to_string());
    let fallback_model_value =
        serde_json::to_string(&fallback_model).unwrap_or_else(|_| "null".to_string());
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
    let missing_label = serde_json::to_string(&i18n::translate(
        locale,
        "settings.general.codexAppUiInjectionMissingLabel",
        &[],
    ))
    .unwrap_or_else(|_| "\"\u{2014}\"".to_string());
    let refresh_in_progress = if refresh_in_progress { "true" } else { "false" };
    let handled_refresh_token =
        serde_json::to_string(&handled_refresh_token).unwrap_or_else(|_| "null".to_string());
    let balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.balance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Balance\"".to_string());
    let account_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.accountBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Account Balance\"".to_string());
    let total_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.totalBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Total balance\"".to_string());
    // Grok 弹框只讲 Grok 额度：标题复用额度概览，空态复用 Grok 平台的空数据文案。
    let quota_overview_label =
        serde_json::to_string(&i18n::translate(locale, "codex.modelProviders.usage.title", &[]))
            .unwrap_or_else(|_| "\"Quota Overview\"".to_string());
    let grok_empty_label =
        serde_json::to_string(&i18n::translate(locale, "grok.quotaQuery.empty", &[]))
            .unwrap_or_else(|_| "\"No quota\"".to_string());
    let balance_lines = serde_json::to_string(balance_lines).unwrap_or_else(|_| "[]".to_string());
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
      const grokLines = {grok_lines_value};
      const grokBadgePercent = {grok_badge_percent};
      const grokBadgeProduct = {grok_badge_product};
      const hostBadgeKind = {badge_kind_value};
      const fallbackModel = {fallback_model_value};
      const accountPoolLabel = {account_pool_label};
      const weeklyLabel = {weekly_label};
      const fiveHourLabel = {five_hour_label};
      const accountPoolTitle = {account_pool_title};
      const quotaEmptyLabel = {quota_empty_label};
      const availableText = {available_text};
      const issueText = {issue_text};
      const refreshLabel = {refresh_label};
      const closeLabel = {close_label};
      const missingLabel = {missing_label};
      const balanceLabel = {balance_label};
      const accountBalanceLabel = {account_balance_label};
      const totalBalanceLabel = {total_balance_label};
      const quotaOverviewLabel = {quota_overview_label};
      const grokEmptyLabel = {grok_empty_label};
      const balanceLines = {balance_lines};
      const refreshInProgress = {refresh_in_progress};
      const handledRefreshToken = {handled_refresh_token};
      const hostHeartbeatTimeoutMs = 8000;
      const root = window.__cockpitCodexInjection || (window.__cockpitCodexInjection = {{}});
      root.hostHeartbeatAt = Date.now();
      root.hostAvailable = true;
      root.providerName = providerName;
      root.weeklyPercent = weeklyPercent;
      root.fiveHourPercent = fiveHourPercent;
      // 只有本次渲染真实读到模型名时才回报给宿主，读不到就交回宿主按实例配置兜底。
      let observedModelThisRender = null;
      // API 服务注入运行期间不显示 DeepSeek 余额徽章：清掉切换绑定后残留的宿主与定时器，
      // 否则两套徽章会叠在同一位置。余额本身改为显示在点击弹框里。
      const staleBalanceRoot = window.__cockpitDeepSeekBalance;
      if (staleBalanceRoot) {{
        if (staleBalanceRoot.watchdogTimer) window.clearInterval(staleBalanceRoot.watchdogTimer);
        if (staleBalanceRoot.observer) staleBalanceRoot.observer.disconnect();
        if (staleBalanceRoot.resizeHandler) window.removeEventListener('resize', staleBalanceRoot.resizeHandler);
        delete window.__cockpitDeepSeekBalance;
      }}
      document.querySelectorAll('[data-cockpit-deepseek-balance], [data-cockpit-deepseek-balance-details]').forEach((node) => node.remove());
      const pendingRefreshToken = typeof root.refreshRequestToken === 'string' && root.refreshRequestToken !== handledRefreshToken
        ? root.refreshRequestToken
        : null;
      root.refreshing = refreshInProgress || Boolean(pendingRefreshToken);
      if (handledRefreshToken && root.refreshRequestToken === handledRefreshToken) root.refreshRequestToken = null;
      const render = () => {{
        let host = document.querySelector('[data-cockpit-quota-footer]');
        const permissions = document.querySelector('[data-composer-navigation-target="permissions"]');
        const footer = permissions?.closest('[class*="_ComposerLayoutFooter_"]') || permissions?.closest('[class*="_footer_"]') || permissions?.parentElement;
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
        // 底部徽章水平居中在「访问权限（完全访问）」和它右侧的「背景信息 / 模型」之间：
        // 先找背景信息（上下文用量）元素，找不到就用模型选择器，取两个元素之间空档的中点。
        const contextUsageRect = () => {{
          const nodes = document.querySelectorAll('[aria-label*="上下文用量"], [aria-label*="背景信息"], [aria-label*="context usage" i], [aria-label*="context window" i]');
          for (const node of nodes) {{
            const rect = node.getBoundingClientRect();
            if (rect.width > 0 && rect.height > 0) return rect;
          }}
          return null;
        }};
        const contextRect = contextUsageRect();
        const modelAnchorNode = contextRect ? null : document.querySelector('[data-composer-navigation-target="reasoning"]');
        const rightAnchorRect = contextRect || (modelAnchorNode ? modelAnchorNode.getBoundingClientRect() : null);
        const permissionsRight = permissionsRect.left + permissionsRect.width;
        const badgeAnchorLeft = rightAnchorRect && rightAnchorRect.left > permissionsRight
          ? Math.round((permissionsRight + rightAnchorRect.left) / 2)
          : Math.round(footerRect.left + footerRect.width / 2);
        host.style.cssText = 'position:fixed;transform:translate(-50%,-50%);z-index:2;display:flex;align-items:center;justify-content:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-size:12px;line-height:1;white-space:nowrap;pointer-events:none;';
        host.style.left = badgeAnchorLeft + 'px';
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
        const currencySymbol = (code) => {{
          const key = String(code || '').toUpperCase();
          if (key === 'CNY' || key === 'RMB') return '¥';
          if (key === 'USD') return '$';
          if (key === 'EUR') return '€';
          return '';
        }};
        const formatMoney = (value, currency) => {{
          if (!Number.isFinite(value)) return '—';
          const symbol = currencySymbol(currency);
          const amount = Number(value).toFixed(2);
          return symbol ? symbol + amount : (currency ? amount + ' ' + String(currency) : amount);
        }};
        const balanceRows = balanceLines.map((line) => '<div style="display:flex;align-items:center;justify-content:space-between;gap:9px;padding:6px 0;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="color:var(--color-token-text-secondary,#737373);font-weight:500;white-space:nowrap;">' + escapeHtml(balanceLabel) + '</span><span style="color:var(--color-token-text-secondary,#737373);text-align:right;white-space:nowrap;">' + escapeHtml(formatMoney(line.totalBalance, line.currency)) + '</span></div>').join('');
        const grokRows = grokLines.map((line) => '<div style="display:flex;align-items:center;justify-content:space-between;gap:9px;padding:6px 0;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="display:inline-flex;align-items:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-weight:500;white-space:nowrap;"><i style="width:6px;height:6px;border-radius:999px;background:#f59e0b;box-shadow:0 0 0 2px rgba(245,158,11,.12);"></i>' + escapeHtml(line.product) + ' <small style="font:inherit;opacity:.62;">' + Math.max(0, Math.round(line.count || 0)) + '</small></span><span style="display:inline-flex;align-items:center;gap:5px;color:var(--color-token-text-secondary,#737373);text-align:right;white-space:nowrap;"><i style="width:5px;height:5px;border-radius:999px;background:#f59e0b;"></i>' + Math.round(line.remainingPercent || 0) + '%</span></div>').join('');
        // Grok 徽章只写产品短名（Build），不重复 Grok 前缀；拿不到产品名时退回 Grok。
        const shortProductLabel = (value) => {{
          const text = String(value || '').trim();
          const stripped = text.replace(/^grok[\s_\-]*/i, '').trim();
          return stripped || text || 'Grok';
        }};
        const dot = (color, ring) => '<span style="width:6px;height:6px;border-radius:999px;background:' + color + ';box-shadow:0 0 0 2px ' + ring + '"></span>';
        const quotaBadge = (color, ring, text, hint) => '<button type="button" data-cockpit-quota-open title="' + escapeHtml(hint) + '" aria-label="' + escapeHtml(hint) + '" style="' + badgeStyle + '">' + dot(color, ring) + escapeHtml(text) + '</button>';
        const fields = [];
        // 底部按当前所选模型套用各自的原生样式：GPT 恢复「账号 / 5h / 周」三个并排胶囊，
        // Grok 显示产品短名 + 剩余百分比，DeepSeek 显示余额徽章；点击弹框同样按模型切换。
        // 只有读到明确供应商关键字才切换：模型选择器展开、切换推理档位时，官方客户端会把
        // 触发器内容换成「选择模型 / Select effort」这类占位文案，把占位文案当成模型名会让
        // 底部在打开选择器的瞬间跳到别的模型样式。
        const classifyModel = (value) => {{
          const slug = String(value || '').trim().toLowerCase();
          if (!slug) return null;
          if (slug.includes('deepseek')) return 'deepseek';
          if (slug.includes('grok')) return 'grok';
          if (slug.includes('gpt') || slug.includes('codex') || slug.includes('openai')) return 'codex';
          // 官方客户端会裁掉 GPT- 前缀（例如 5.4-Codex 只显示 5.4），数字开头仍按 GPT 处理。
          if (/^[0-9]/.test(slug)) return 'codex';
          return null;
        }};
        const readVisibleModel = () => {{
          const nodes = [];
          const trigger = document.querySelector('[data-composer-navigation-target="reasoning"]');
          if (trigger) {{
            nodes.push(trigger.querySelector('[class*="ModelPickerTriggerModelLabel_"]'));
            nodes.push(trigger.querySelector('[class*="ModelPickerTriggerModelGroup_"]'));
            nodes.push(trigger.querySelector('[class*="ModelPickerTriggerContent_"]'));
            nodes.push(trigger);
          }}
          // 选择器展开时触发器可能只剩占位文案，当前模型名在弹框里：
          // 只取弹框里的当前模型行，不读列表里的其它模型。
          const dropdown = document.querySelector('[class*="ModelPickerDropdownContent_"]');
          if (dropdown) {{
            nodes.push(dropdown.querySelector('[data-model-picker-model-row]'));
            nodes.push(dropdown.querySelector('[class*="ViewToggleModelLabel_"]'));
          }}
          for (const node of nodes) {{
            if (!node) continue;
            const text = String(node.textContent || '').replace(/\s+/g, ' ').trim();
            if (classifyModel(text)) return text;
          }}
          return '';
        }};
        const visibleModel = readVisibleModel();
        if (visibleModel) {{
          // 记住最后一次识别成功的模型：占位文案期间保持不变，避免徽章来回跳。
          root.visibleModel = visibleModel;
          root.visibleModelAt = Date.now();
          observedModelThisRender = visibleModel;
        }}
        // 长期读不到模型名时重新交回宿主判断（宿主会按实例 config.toml 兜底刷新），
        // 避免一直停在切换前的模型上。
        const modelStickyMs = 15000;
        const stickyFresh = Boolean(root.visibleModel)
          && Date.now() - (root.visibleModelAt || 0) <= modelStickyMs;
        const selectedModel = stickyFresh ? root.visibleModel : (fallbackModel || '');
        const badgeKind = classifyModel(selectedModel) || hostBadgeKind;
        if (badgeKind === 'deepseek') {{
          const line = Array.isArray(balanceLines) ? balanceLines[0] : null;
          const text = balanceLabel + ' ' + (line ? formatMoney(line.totalBalance, line.currency) : missingLabel);
          fields.push(quotaBadge('#3b82f6', 'rgba(59,130,246,.14)', text, text));
        }} else if (badgeKind === 'grok') {{
          const grokPercent = Number.isFinite(grokBadgePercent) ? Math.round(grokBadgePercent) + '%' : missingLabel;
          const text = shortProductLabel(grokBadgeProduct) + ' ' + grokPercent;
          fields.push(quotaBadge('#f59e0b', 'rgba(245,158,11,.14)', text, quotaOverviewLabel + ' ' + text));
        }} else {{
          // GPT 原生模型恢复账号池的三个独立胶囊，没有数据的窗口不渲染。
          if (Number.isFinite(accountCount) && accountCount >= 0) {{
            const text = accountPoolLabel + ' ' + Math.round(accountCount);
            fields.push(quotaBadge('#8b5cf6', 'rgba(139,92,246,.14)', text, text));
          }}
          if (Number.isFinite(fiveHourPercent)) {{
            const text = fiveHourLabel + ' ' + Math.round(fiveHourPercent) + '%';
            fields.push(quotaBadge('#3b82f6', 'rgba(59,130,246,.14)', text, text));
          }}
          if (Number.isFinite(weeklyPercent)) {{
            const text = weeklyLabel + ' ' + Math.round(weeklyPercent) + '%';
            fields.push(quotaBadge('#10b981', 'rgba(16,185,129,.14)', text, text));
          }}
        }}
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
        // 弹框与徽章共用同一个锚点（访问权限与背景信息/模型之间的中点）。
        details.style.left = badgeAnchorLeft + 'px';
        details.style.top = Math.max(12, Math.round(permissionsRect.top - 2)) + 'px';
        details.style.transform = 'translate(-50%,-100%)';
        details.style.display = root.quotaDetailsOpen ? 'block' : 'none';
        if (root.quotaDetailsOpen) {{
          const detailHeader = (color, ring, title) => '<div style="display:flex;align-items:center;justify-content:space-between;gap:10px;margin:0 0 4px;padding-bottom:6px;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:500;color:var(--color-token-text-secondary,#737373);">' + dot(color, ring) + escapeHtml(title) + '</span><button type="button" data-cockpit-quota-close aria-label="' + escapeHtml(closeLabel) + '" title="' + escapeHtml(closeLabel) + '" style="display:inline-flex;align-items:center;justify-content:center;width:18px;height:18px;border:0;border-radius:4px;background:transparent;color:var(--color-token-text-secondary,#737373);font:inherit;font-size:14px;line-height:1;cursor:pointer;padding:0;opacity:.72;">×</button></div>';
          const emptyRow = (text) => '<div style="padding:6px 0;color:var(--color-token-text-secondary,#737373);opacity:.72;">' + escapeHtml(text) + '</div>';
          const rowStyle = 'display:flex;align-items:center;justify-content:space-between;gap:9px;padding:6px 0;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));';
          const metricStyle = 'display:inline-flex;align-items:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-weight:500;white-space:nowrap;';
          const valueStyle = 'display:inline-flex;align-items:center;gap:5px;color:var(--color-token-text-secondary,#737373);text-align:right;white-space:nowrap;';
          let detailsHtml;
          if (badgeKind === 'grok') {{
            // Grok 弹框只讲 Grok 额度：产品短名 + 账号数 + 剩余百分比。
            const grokDetailRows = grokLines.map((line) => '<div style="' + rowStyle + '"><span style="' + metricStyle + '"><i style="width:6px;height:6px;border-radius:999px;background:#f59e0b;box-shadow:0 0 0 2px rgba(245,158,11,.12);"></i>' + escapeHtml(shortProductLabel(line.product)) + ' <small style="font:inherit;opacity:.62;">' + Math.max(0, Math.round(line.count || 0)) + '</small></span><span style="' + valueStyle + '"><i style="width:5px;height:5px;border-radius:999px;background:#f59e0b;"></i>' + Math.round(line.remainingPercent || 0) + '%</span></div>').join('');
            detailsHtml = detailHeader('#f59e0b', 'rgba(245,158,11,.12)', quotaOverviewLabel)
              + '<div>' + (grokDetailRows || emptyRow(grokEmptyLabel)) + '</div>';
          }} else if (badgeKind === 'deepseek') {{
            // 余额弹框只讲余额：账号池里可查官方余额的账号按币种合并后的总余额。
            const balanceDetailRows = balanceLines.map((line) => '<div style="' + rowStyle + '"><span style="' + metricStyle + '">' + escapeHtml(totalBalanceLabel) + '</span><span style="' + valueStyle + '">' + escapeHtml(formatMoney(line.totalBalance, line.currency)) + '</span></div>').join('');
            detailsHtml = detailHeader('#3b82f6', 'rgba(59,130,246,.12)', accountBalanceLabel)
              + '<div>' + (balanceDetailRows || emptyRow(missingLabel)) + '</div>';
          }} else {{
            const planRows = plans.map(renderPlan).join('') + grokRows;
            detailsHtml = detailHeader('#8b5cf6', 'rgba(139,92,246,.12)', accountPoolTitle)
              + '<div>' + (planRows || emptyRow(quotaEmptyLabel)) + '</div>'
              + balanceRows
              + '<div style="display:flex;justify-content:space-between;gap:10px;padding-top:7px;color:var(--color-token-text-secondary,#737373);font-size:11px;opacity:.78;"><span>' + escapeHtml(availableText) + '</span><span>' + escapeHtml(issueText) + '</span></div>';
          }}
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
      return {{refreshRequestToken: pendingRefreshToken, selectedModel: observedModelThisRender}};
    }})()"#
    )
}

/// DeepSeek 账号余额徽标：与 API 服务额度徽标同位置、同交互，但只展示账号余额。
///
/// 独立的 `window.__cockpitDeepSeekBalance` 命名空间与 `data-cockpit-deepseek-balance*`
/// 标记，避免与 CDP 模式下的模型列表注入脚本互相覆盖渲染函数。
fn deepseek_balance_injection_script(
    locale: &str,
    balance: Option<&DeepSeekBalanceSnapshot>,
    refresh_in_progress: bool,
    handled_refresh_token: Option<&str>,
) -> String {
    let balance = serde_json::to_string(&balance).unwrap_or_else(|_| "null".to_string());
    let balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.balance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Balance\"".to_string());
    let account_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.accountBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Account Balance\"".to_string());
    let total_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.totalBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Total balance\"".to_string());
    let granted_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.grantedBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Granted balance\"".to_string());
    let topped_up_balance_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.modelProviders.usage.fields.toppedUpBalance",
        &[],
    ))
    .unwrap_or_else(|_| "\"Topped-up balance\"".to_string());
    let available_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.healthAvailable",
        &[],
    ))
    .unwrap_or_else(|_| "\"Available\"".to_string());
    let unavailable_label = serde_json::to_string(&i18n::translate(
        locale,
        "codex.localAccess.healthUnavailable",
        &[],
    ))
    .unwrap_or_else(|_| "\"Unavailable\"".to_string());
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
      const balance = {balance};
      const balanceLabel = {balance_label};
      const accountBalanceLabel = {account_balance_label};
      const totalBalanceLabel = {total_balance_label};
      const grantedBalanceLabel = {granted_balance_label};
      const toppedUpBalanceLabel = {topped_up_balance_label};
      const availableLabel = {available_label};
      const unavailableLabel = {unavailable_label};
      const refreshLabel = {refresh_label};
      const closeLabel = {close_label};
      const refreshInProgress = {refresh_in_progress};
      const handledRefreshToken = {handled_refresh_token};
      const hostHeartbeatTimeoutMs = 8000;
      const root = window.__cockpitDeepSeekBalance || (window.__cockpitDeepSeekBalance = {{}});
      root.hostHeartbeatAt = Date.now();
      root.hostAvailable = true;
      root.hasBalance = Boolean(balance);
      // 余额注入运行期间不再叠加 API 服务额度徽章：清掉切换绑定后残留的宿主、定时器与
      // 额度脚本专属状态，共享命名空间里的模型注入字段保持不变。
      const staleQuotaRoot = window.__cockpitCodexInjection;
      if (staleQuotaRoot) {{
        if (staleQuotaRoot.watchdogTimer) window.clearInterval(staleQuotaRoot.watchdogTimer);
        if (staleQuotaRoot.observer) staleQuotaRoot.observer.disconnect();
        if (staleQuotaRoot.resizeHandler) window.removeEventListener('resize', staleQuotaRoot.resizeHandler);
        if (staleQuotaRoot.layoutObserver) staleQuotaRoot.layoutObserver.disconnect();
        delete staleQuotaRoot.watchdogTimer;
        delete staleQuotaRoot.observer;
        delete staleQuotaRoot.resizeHandler;
        delete staleQuotaRoot.layoutObserver;
        delete staleQuotaRoot.render;
        delete staleQuotaRoot.scheduleRender;
        delete staleQuotaRoot.renderScheduled;
        delete staleQuotaRoot.providerName;
        delete staleQuotaRoot.weeklyPercent;
        delete staleQuotaRoot.fiveHourPercent;
        delete staleQuotaRoot.refreshRequestToken;
        delete staleQuotaRoot.refreshing;
        delete staleQuotaRoot.quotaDetailsOpen;
        delete staleQuotaRoot.layoutFooter;
        delete staleQuotaRoot.layoutPermissions;
      }}
      document.querySelectorAll('[data-cockpit-quota-footer], [data-cockpit-quota-details]').forEach((node) => node.remove());
      const pendingRefreshToken = typeof root.refreshRequestToken === 'string' && root.refreshRequestToken !== handledRefreshToken
        ? root.refreshRequestToken
        : null;
      root.refreshing = refreshInProgress || Boolean(pendingRefreshToken);
      if (handledRefreshToken && root.refreshRequestToken === handledRefreshToken) root.refreshRequestToken = null;
      const escapeHtml = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[char]));
      const currencySymbol = (code) => {{
        const key = String(code || '').toUpperCase();
        if (key === 'CNY' || key === 'RMB') return '¥';
        if (key === 'USD') return '$';
        if (key === 'EUR') return '€';
        return '';
      }};
      const formatMoney = (value, currency) => {{
        if (!Number.isFinite(value)) return '—';
        const symbol = currencySymbol(currency);
        const amount = Number(value).toFixed(2);
        return symbol ? symbol + amount : (currency ? amount + ' ' + String(currency) : amount);
      }};
      const render = () => {{
        let host = document.querySelector('[data-cockpit-deepseek-balance]');
        let details = document.querySelector('[data-cockpit-deepseek-balance-details]');
        const permissions = document.querySelector('[data-composer-navigation-target="permissions"]');
        const footer = permissions?.closest('[class*="_ComposerLayoutFooter_"]') || permissions?.closest('[class*="_footer_"]') || permissions?.parentElement;
        if (!permissions || !footer || !balance) {{
          if (host) host.style.display = 'none';
          if (details) details.style.display = 'none';
          root.detailsOpen = false;
          return;
        }}
        if (!host) {{
          host = document.createElement('div');
          host.setAttribute('data-cockpit-deepseek-balance', 'true');
          document.body.appendChild(host);
        }}
        if (!details) {{
          details = document.createElement('div');
          details.setAttribute('data-cockpit-deepseek-balance-details', 'true');
          document.body.appendChild(details);
        }}
        const footerRect = footer.getBoundingClientRect();
        const permissionsRect = permissions.getBoundingClientRect();
        // 与 API 服务额度注入同一套锚点：居中在「访问权限」和右侧「背景信息 / 模型」之间。
        const contextUsageRect = () => {{
          const nodes = document.querySelectorAll('[aria-label*="上下文用量"], [aria-label*="背景信息"], [aria-label*="context usage" i], [aria-label*="context window" i]');
          for (const node of nodes) {{
            const rect = node.getBoundingClientRect();
            if (rect.width > 0 && rect.height > 0) return rect;
          }}
          return null;
        }};
        const contextRect = contextUsageRect();
        const modelAnchorNode = contextRect ? null : document.querySelector('[data-composer-navigation-target="reasoning"]');
        const rightAnchorRect = contextRect || (modelAnchorNode ? modelAnchorNode.getBoundingClientRect() : null);
        const permissionsRight = permissionsRect.left + permissionsRect.width;
        const badgeAnchorLeft = rightAnchorRect && rightAnchorRect.left > permissionsRight
          ? Math.round((permissionsRight + rightAnchorRect.left) / 2)
          : Math.round(footerRect.left + footerRect.width / 2);
        host.style.cssText = 'position:fixed;transform:translate(-50%,-50%);z-index:2;display:flex;align-items:center;justify-content:center;gap:6px;color:var(--color-token-text-secondary,#737373);font-size:12px;line-height:1;white-space:nowrap;pointer-events:none;';
        host.style.left = badgeAnchorLeft + 'px';
        host.style.top = Math.round(permissionsRect.top + permissionsRect.height / 2) + 'px';
        const badgeStyle = 'display:inline-flex;align-items:center;gap:6px;height:24px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.20));border-radius:999px;padding:0 9px;background:var(--color-token-main-surface-primary,rgba(127,127,127,.10));color:inherit;font:inherit;box-shadow:0 1px 2px rgba(0,0,0,.08);backdrop-filter:blur(8px);font-weight:500;cursor:pointer;pointer-events:auto;';
        const currency = balance.currency || null;
        const totalText = formatMoney(balance.totalBalance, currency);
        const badgeHtml = '<button type="button" data-cockpit-deepseek-balance-open style="' + badgeStyle + '"><span style="width:6px;height:6px;border-radius:999px;background:#3b82f6"></span>' + escapeHtml(balanceLabel) + ' ' + escapeHtml(totalText) + '</button>'
          + '<button type="button" data-cockpit-deepseek-balance-refresh title="' + escapeHtml(refreshLabel) + '" aria-label="' + escapeHtml(refreshLabel) + '" style="display:inline-flex;align-items:center;justify-content:center;width:24px;height:24px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.20));border-radius:999px;padding:0;background:var(--color-token-main-surface-primary,rgba(127,127,127,.10));color:inherit;box-shadow:0 1px 2px rgba(0,0,0,.08);backdrop-filter:blur(8px);cursor:pointer;pointer-events:auto;"><svg data-cockpit-deepseek-balance-refresh-icon viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 6v5h-5"></path><path d="M4 18v-5h5"></path><path d="M6.1 9a7 7 0 0 1 11.6-2.6L20 11"></path><path d="M4 13l2.3 4.6A7 7 0 0 0 17.9 15"></path></svg></button>';
        if (host.innerHTML !== badgeHtml) host.innerHTML = badgeHtml;
        host.style.display = 'flex';
        const row = (label, value) => '<div style="display:flex;align-items:center;justify-content:space-between;gap:9px;padding:6px 0;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="color:var(--color-token-text-secondary,#737373);font-weight:500;white-space:nowrap;">' + escapeHtml(label) + '</span><span style="color:var(--color-token-text-secondary,#737373);text-align:right;white-space:nowrap;">' + escapeHtml(value) + '</span></div>';
        details.style.cssText = 'position:fixed;z-index:4;width:min(240px,calc(100vw - 24px));box-sizing:border-box;padding:9px 11px;border:1px solid var(--color-token-border-subtle,rgba(127,127,127,.16));border-radius:10px;background:var(--color-token-main-surface-primary,#fff);color:var(--color-token-text-secondary,#737373);box-shadow:0 4px 14px rgba(0,0,0,.09);font-family:inherit;font-size:12px;line-height:1.3;letter-spacing:normal;pointer-events:auto;';
        details.style.left = badgeAnchorLeft + 'px';
        details.style.top = Math.max(12, Math.round(permissionsRect.top - 2)) + 'px';
        details.style.transform = 'translate(-50%,-100%)';
        details.style.display = root.detailsOpen ? 'block' : 'none';
        if (root.detailsOpen) {{
          const detailsHtml = '<div style="display:flex;align-items:center;justify-content:space-between;gap:10px;margin:0 0 4px;padding-bottom:6px;border-bottom:1px solid var(--color-token-border-subtle,rgba(127,127,127,.10));"><span style="display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:500;color:var(--color-token-text-secondary,#737373);"><i style="width:6px;height:6px;border-radius:999px;background:#3b82f6;box-shadow:0 0 0 2px rgba(59,130,246,.12);"></i>' + escapeHtml(accountBalanceLabel) + '</span><button type="button" data-cockpit-deepseek-balance-close aria-label="' + escapeHtml(closeLabel) + '" title="' + escapeHtml(closeLabel) + '" style="display:inline-flex;align-items:center;justify-content:center;width:18px;height:18px;border:0;border-radius:4px;background:transparent;color:var(--color-token-text-secondary,#737373);font:inherit;font-size:14px;line-height:1;cursor:pointer;padding:0;opacity:.72;">×</button></div>'
            + '<div>'
            + row(totalBalanceLabel, totalText)
            + row(grantedBalanceLabel, formatMoney(balance.grantedBalance, currency))
            + row(toppedUpBalanceLabel, formatMoney(balance.toppedUpBalance, currency))
            + '</div>'
            + '<div style="display:flex;justify-content:space-between;gap:10px;padding-top:7px;font-size:11px;opacity:.78;"><span>' + escapeHtml(balance.isAvailable ? availableLabel : unavailableLabel) + '</span><span>' + escapeHtml(currency || '') + '</span></div>';
          if (details.innerHTML !== detailsHtml) details.innerHTML = detailsHtml;
        }}
        const openButton = host.querySelector('[data-cockpit-deepseek-balance-open]');
        if (openButton) openButton.onclick = () => {{ root.detailsOpen = !root.detailsOpen; root.render(); }};
        const closeButton = details.querySelector('[data-cockpit-deepseek-balance-close]');
        if (closeButton) closeButton.onclick = () => {{ root.detailsOpen = false; root.render(); }};
        const refreshButton = host.querySelector('[data-cockpit-deepseek-balance-refresh]');
        if (refreshButton) {{
          const refreshDisabled = root.refreshing || root.hostAvailable === false;
          refreshButton.disabled = refreshDisabled;
          refreshButton.style.cursor = refreshDisabled ? 'not-allowed' : 'pointer';
          refreshButton.style.opacity = root.refreshing ? '.7' : (root.hostAvailable === false ? '.45' : '1');
          const refreshIcon = refreshButton.querySelector('[data-cockpit-deepseek-balance-refresh-icon]');
          if (refreshIcon) refreshIcon.style.animation = root.refreshing ? 'cockpit-quota-spin .8s linear infinite' : 'none';
          refreshButton.onclick = () => {{
            if (refreshDisabled) return;
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
          const host = document.querySelector('[data-cockpit-deepseek-balance]');
          const details = document.querySelector('[data-cockpit-deepseek-balance-details]');
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
        .and_then(normalize_injection_model_slug)
}

#[derive(Debug, Default)]
struct InjectionEvalResult {
    refresh_request_token: Option<String>,
    selected_model: Option<String>,
}

async fn evaluate_target(
    target: &CdpTarget,
    script: &str,
    script_kind: &str,
) -> Option<InjectionEvalResult> {
    if target.target_type != "page" && target.target_type != "webview" {
        return None;
    }
    let Some(websocket_url) = target.websocket_url.as_deref() else {
        return None;
    };
    let install_key = format!("{}|{}", script_kind, websocket_url);
    let Ok(Ok((mut socket, _))) = timeout(CDP_CONNECT_TIMEOUT, connect_async(websocket_url)).await
    else {
        return None;
    };
    let install_on_new_document = should_install_new_document_script(&install_key);
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
        mark_new_document_script_installed(&install_key);
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

include!("codex_app_injection_quota.rs");

async fn run_injection_loop(
    app: AppHandle,
    _instance_id: String,
    profile_dir: PathBuf,
    port: u16,
    bind_account_id: Option<String>,
) {
    let client = Client::new();
    // 余额接口是上游网络请求，必须自带超时，不能让注入循环被拖住。
    let balance_client = Client::builder()
        .timeout(DEEPSEEK_BALANCE_QUERY_TIMEOUT)
        .build()
        .unwrap_or_else(|_| Client::new());
    let mut last_quota_at = Instant::now() - QUOTA_REFRESH_INTERVAL;
    let mut last_balance_at = Instant::now() - DEEPSEEK_BALANCE_REFRESH_INTERVAL;
    let mut last_api_service_balance_at = Instant::now() - DEEPSEEK_BALANCE_REFRESH_INTERVAL;
    let mut quota = QuotaResponse::default();
    let mut balance: Option<DeepSeekBalanceSnapshot> = None;
    let mut api_service_balance_lines: Vec<InjectionBalanceLine> = Vec::new();
    let mut api_service_grok_lines: Vec<InjectionGrokQuotaLine> = Vec::new();
    let mut handled_refresh_token: Option<String> = None;
    let mut handled_balance_token: Option<String> = None;
    let mut deepseek_account_cache: Option<CodexAccount> = None;
    let mut deepseek_account_at = Instant::now() - DEEPSEEK_ACCOUNT_CACHE_TTL;
    let mut observed_model = read_profile_selected_model(&profile_dir);
    let mut last_model_resync_at = Instant::now() - MODEL_RESYNC_INTERVAL;
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
        // DeepSeek 绑定（网关列出 / CDP 注入 / 直连官方）统一在底部显示账号余额；
        // 只有 CDP 接入方式才需要额外的模型列表注入；官方 DeepSeek 仍额外注入余额。
        if deepseek_account_at.elapsed() >= DEEPSEEK_ACCOUNT_CACHE_TTL {
            deepseek_account_at = Instant::now();
            // 账号文件短暂读取失败时保留上一份快照，不因为一次读取失败就撤掉余额。
            if let Some(account) = bound_api_key_account(bind_account_id.as_deref()) {
                deepseek_account_cache = Some(account);
            }
        }
        // 只有「CDP 模型注入」或「可查官方余额」时才需要进入注入分支：
        // 第三方中转没有官方余额接口，只做模型清单注入。
        let deepseek_account = deepseek_account_cache.as_ref().filter(|account| {
            crate::modules::codex_account::account_uses_cdp_model_injection(account)
                || deepseek_balance_endpoint(account).is_some()
        });
        if let Some(deepseek_account) = deepseek_account {
            let deepseek_cdp =
                crate::modules::codex_account::account_uses_cdp_model_injection(deepseek_account);
            if deepseek_cdp {
                let account_id = Some(deepseek_account.id.clone());
                let payload =
                    crate::modules::codex_account::provider_injection_model_payload(
                        deepseek_account,
                    );
                let script = deepseek_model_injection_script(
                    &locale,
                    &payload,
                    handled_refresh_token.as_deref(),
                );
                let targets = query_targets(&client, port).await;
                let mut pending_model = None;
                for target in &targets {
                    if let Some(result) =
                        evaluate_target(target, &script, DEEPSEEK_MODEL_SCRIPT_KIND).await
                    {
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
                            observed_model = Some(model.clone());
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
            }
            if enabled_for_app() {
                if last_balance_at.elapsed() >= DEEPSEEK_BALANCE_REFRESH_INTERVAL {
                    if let Some(value) =
                        fetch_deepseek_balance(&balance_client, deepseek_account).await
                    {
                        balance = Some(value);
                    }
                    last_balance_at = Instant::now();
                }
                let script = deepseek_balance_injection_script(
                    &locale,
                    balance.as_ref(),
                    false,
                    handled_balance_token.as_deref(),
                );
                let targets = query_targets(&client, port).await;
                let mut pending_refresh_token = None;
                for target in &targets {
                    if let Some(result) =
                        evaluate_target(target, &script, DEEPSEEK_BALANCE_SCRIPT_KIND).await
                    {
                        if let Some(token) = result.refresh_request_token {
                            if handled_balance_token.as_deref() != Some(token.as_str()) {
                                pending_refresh_token = Some(token);
                            }
                        }
                    }
                }
                if let Some(token) = pending_refresh_token {
                    handled_balance_token = Some(token);
                    if let Some(value) =
                        fetch_deepseek_balance(&balance_client, deepseek_account).await
                    {
                        balance = Some(value);
                    }
                    last_balance_at = Instant::now();
                    // 立刻重绘一次，让刷新按钮结束 loading 状态。
                    let refreshed_script = deepseek_balance_injection_script(
                        &locale,
                        balance.as_ref(),
                        false,
                        handled_balance_token.as_deref(),
                    );
                    for target in &targets {
                        let _ = evaluate_target(
                            target,
                            &refreshed_script,
                            DEEPSEEK_BALANCE_SCRIPT_KIND,
                        )
                        .await;
                    }
                }
            }
            tokio::time::sleep(INJECTION_INTERVAL).await;
            continue;
        }
        if refresh_finished || last_quota_at.elapsed() >= QUOTA_REFRESH_INTERVAL {
            let gateway = read_profile_gateway_config(&profile_dir);
            if let Some(value) = fetch_quota(&client, gateway.as_ref()).await {
                let value_is_empty = value.account_count == Some(0);
                let confirmed_empty = if value_is_empty {
                    api_service_account_pool_is_empty()
                        .await
                        .ok()
                        .flatten()
                        .unwrap_or(false)
                } else {
                    false
                };
                if !value_is_empty || confirmed_empty || refreshed_empty_pool {
                    quota = value;
                }
            }
            if refreshed_empty_pool {
                quota = QuotaResponse::empty_pool();
            }
            last_quota_at = Instant::now();
        }
        // API 服务注入不再显示独立的余额徽章，余额只放在点击弹框里；查询失败保留上次结果。
        if refresh_finished
            || last_api_service_balance_at.elapsed() >= DEEPSEEK_BALANCE_REFRESH_INTERVAL
        {
            if let Some(lines) = fetch_api_service_balance_lines(&balance_client).await {
                api_service_balance_lines = lines;
            }
            // Grok 额度取自本地账号快照，无网络请求，随余额刷新节拍一起更新。
            api_service_grok_lines = api_service_grok_quota_lines().await;
            last_api_service_balance_at = Instant::now();
        }
        let gateway = read_profile_gateway_config(&profile_dir);
        let provider_name = gateway
            .as_ref()
            .map(|value| value.provider_name.as_str())
            .unwrap_or("Codex");
        if observed_model.is_none() {
            observed_model = read_profile_selected_model(&profile_dir);
        }
        let badge_kind = injection_badge_kind_for_model(observed_model.as_deref());
        let script = injection_script(
            provider_name,
            &quota,
            &locale,
            !refresh_tasks.is_empty(),
            handled_refresh_token.as_deref(),
            &api_service_balance_lines,
            &api_service_grok_lines,
            badge_kind,
            observed_model.as_deref(),
        );
        let targets = query_targets(&client, port).await;
        let mut refresh_request_token = None;
        let mut visible_model = None;
        let mut model_reported = false;
        for target in &targets {
            if let Some(result) = evaluate_target(target, &script, QUOTA_SCRIPT_KIND).await {
                if let Some(token) = result.refresh_request_token {
                    if handled_refresh_token.as_deref() != Some(token.as_str()) {
                        refresh_request_token = Some(token);
                    }
                }
                if result.selected_model.is_some() {
                    visible_model = result.selected_model;
                }
            }
        }
        if let Some(model) = visible_model {
            model_reported = true;
            if observed_model.as_deref() != Some(model.as_str()) {
                observed_model = Some(model);
                let next_kind = injection_badge_kind_for_model(observed_model.as_deref());
                if next_kind != badge_kind {
                    let switched_script = injection_script(
                        provider_name,
                        &quota,
                        &locale,
                        !refresh_tasks.is_empty(),
                        handled_refresh_token.as_deref(),
                        &api_service_balance_lines,
                        &api_service_grok_lines,
                        next_kind,
                        observed_model.as_deref(),
                    );
                    for target in &targets {
                        let _ = evaluate_target(target, &switched_script, QUOTA_SCRIPT_KIND).await;
                    }
                }
            }
        }
        // 页面读不到模型名（选择器展开、界面尚未渲染完）时，用实例 config.toml 的
        // 已选模型兜底刷新，避免底部徽章长期停在切换前的模型上。
        if !model_reported && last_model_resync_at.elapsed() >= MODEL_RESYNC_INTERVAL {
            last_model_resync_at = Instant::now();
            if let Some(model) = read_profile_selected_model(&profile_dir) {
                observed_model = Some(model);
            }
        }
        if let Some(token) = refresh_request_token.filter(|_| refresh_tasks.is_empty()) {
            handled_refresh_token = Some(token);
            // 手动刷新同时把余额与 Grok 额度快照请求一次，点刷新不会只更新账号池。
            last_api_service_balance_at = Instant::now() - DEEPSEEK_BALANCE_REFRESH_INTERVAL;
            let refreshing_kind = injection_badge_kind_for_model(observed_model.as_deref());
            let refreshing_script = injection_script(
                provider_name,
                &quota,
                &locale,
                true,
                handled_refresh_token.as_deref(),
                &api_service_balance_lines,
                &api_service_grok_lines,
                refreshing_kind,
                observed_model.as_deref(),
            );
            for target in &targets {
                let _ = evaluate_target(target, &refreshing_script, QUOTA_SCRIPT_KIND).await;
            }
            let app = app.clone();
            refresh_tasks.spawn(async move { run_quota_refresh_singleflight(&app).await });
        }
        tokio::time::sleep(INJECTION_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    include!("codex_app_injection_tests.rs");
}
