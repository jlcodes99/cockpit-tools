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
use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
#[cfg(not(target_os = "macos"))]
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::AppHandle;
use tokio::sync::Mutex as TokioMutex;
use tokio::task::JoinSet;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use toml_edit::Document;

const CDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const INJECTION_INTERVAL: Duration = Duration::from_secs(2);
const QUOTA_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct CodexAppInjectionLaunch {
    pub args: Vec<String>,
    pub port: Option<u16>,
}

struct InjectionRuntime {
    task: tauri::async_runtime::JoinHandle<()>,
}

fn runtimes() -> &'static Mutex<HashMap<String, InjectionRuntime>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<String, InjectionRuntime>>> = OnceLock::new();
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
        && should_enable_injection(store.default_settings.bind_account_id.as_deref())
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
            || !should_enable_injection(instance.bind_account_id.as_deref())
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
            logger::log_warn(&format!(
                "[Codex App Injection] 跳过恢复，运行中的实例缺少 CDP 端口: instance_id={}, pid={}",
                instance_id, pid
            ));
            continue;
        };
        start_for_profile(
            app.clone(),
            instance_id.clone(),
            profile_dir,
            Some(port),
            bind_account_id,
        );
        restored += 1;
        logger::log_info(&format!(
            "[Codex App Injection] 已恢复运行中实例: instance_id={}, pid={}, port={}",
            instance_id, pid, port
        ));
    }

    Ok(restored)
}

pub fn stop_for_profile(profile_dir: &Path) {
    let key = profile_key(profile_dir);
    if let Ok(mut items) = runtimes().lock() {
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
}

pub fn start_for_profile(
    app: AppHandle,
    instance_id: String,
    profile_dir: PathBuf,
    port: Option<u16>,
    bind_account_id: Option<String>,
) {
    let Some(port) = port else { return };
    if !should_enable_injection(bind_account_id.as_deref()) {
        return;
    }
    stop_for_profile(&profile_dir);
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

#[derive(Debug, Deserialize)]
struct CdpTarget {
    #[serde(rename = "type")]
    target_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    websocket_url: Option<String>,
}

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
      const descriptor = (official) => ({{
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
        defaultReasoningEffort: "high",
        supportedReasoningEfforts: ["low", "medium", "high", "xhigh"].map((effort) => ({{
          reasoningEffort: effort,
          description: effort,
        }})),
      }});
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
        return Vec::new();
    };
    response.json::<Vec<CdpTarget>>().await.unwrap_or_default()
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
    let success_count =
        crate::commands::codex::refresh_codex_quotas_batch(app.clone(), target_ids, Some(true))
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
        build_launch_args, deepseek_model_injection_script, injection_script,
        refresh_request_token_from_cdp_response, remote_debugging_port_from_command_line,
        selected_model_from_cdp_response, supports_bind_account, QuotaPlanSummary, QuotaResponse,
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
}
