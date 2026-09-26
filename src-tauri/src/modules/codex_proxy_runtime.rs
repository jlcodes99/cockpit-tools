//! Per-account single-flight tunnel state. The registry lock never spans I/O.
use crate::models::codex::CodexAccount;
use crate::modules::{
    codex_account, codex_account_proxy,
    codex_proxy_engine::{self, EngineController, NodeTunnel},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Runtime {
    signature: String,
    tunnel: Option<NodeTunnel>,
    desktop_signature: String,
    desktop_tunnel: Option<Arc<DesktopTunnel>>,
    sidecar_signature: String,
    sidecar_tunnel: Option<NodeTunnel>,
    account_starting: bool,
    desktop_starting: bool,
    sidecar_starting: bool,
    account_start_generation: u64,
    desktop_start_generation: u64,
    sidecar_start_generation: u64,
}

pub(crate) struct DesktopTunnel(NodeTunnel);

impl Deref for DesktopTunnel {
    type Target = NodeTunnel;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for DesktopTunnel {
    fn drop(&mut self) {
        super::codex_proxy_activity::detach(self.0.controller().tunnel_id);
    }
}

type Slot = Arc<tokio::sync::Mutex<Runtime>>;
static RUNTIMES: LazyLock<Mutex<HashMap<String, Slot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STARTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static READS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);

#[derive(Clone, Copy)]
enum StartKind {
    Account,
    Desktop,
    Sidecar,
}

struct StartGuard {
    slot: Slot,
    kind: StartKind,
    generation: u64,
    armed: bool,
}

impl StartGuard {
    fn new(slot: Slot, kind: StartKind, generation: u64) -> Self {
        Self {
            slot,
            kind,
            generation,
            armed: true,
        }
    }

    fn complete(&mut self) {
        self.armed = false;
    }
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let slot = self.slot.clone();
        let kind = self.kind;
        let generation = self.generation;
        tauri::async_runtime::spawn(async move {
            let mut state = slot.lock().await;
            clear_starting(&mut state, kind, generation);
        });
    }
}

fn clear_starting(state: &mut Runtime, kind: StartKind, generation: u64) {
    match kind {
        StartKind::Account if state.account_start_generation == generation => {
            state.account_starting = false;
        }
        StartKind::Desktop if state.desktop_start_generation == generation => {
            state.desktop_starting = false;
        }
        StartKind::Sidecar if state.sidecar_start_generation == generation => {
            state.sidecar_starting = false;
        }
        _ => {}
    }
}

async fn retire(tunnel: NodeTunnel) {
    super::codex_proxy_activity::detach(tunnel.controller().tunnel_id);
    if tunnel.stop().await.is_err() {
        crate::modules::logger::log_warn(
            "[CodexProxy] 旧内核回收未确认，已再次发送终止信号；已提交配置保持有效",
        );
    }
}

/// Clone controller handles before any local HTTP work; never hold a runtime lock
/// while reading activity. Non-engine direct proxies have no controller.
pub async fn active_controllers(account_id: &str) -> Vec<(&'static str, EngineController)> {
    let shared = RUNTIMES
        .lock()
        .ok()
        .and_then(|store| store.get(account_id).cloned());
    let Some(shared) = shared else {
        return Vec::new();
    };
    let Ok(mut state) = tokio::time::timeout(Duration::from_secs(2), shared.lock()).await else {
        return Vec::new();
    };
    let mut result = Vec::new();
    if let Some(tunnel) = state.tunnel.as_mut().filter(|tunnel| tunnel.is_running()) {
        result.push(("account", tunnel.controller()));
    }
    if let Some(tunnel) = state
        .desktop_tunnel
        .as_ref()
        .filter(|tunnel| tunnel.is_running())
    {
        result.push(("desktop", tunnel.controller()));
    }
    if let Some(tunnel) = state
        .sidecar_tunnel
        .as_mut()
        .filter(|tunnel| tunnel.is_running())
    {
        result.push(("sidecar", tunnel.controller()));
    }
    result
}

/// 账号本地通道换端口后，后台让 API 服务网关重新核对配置。
///
/// sidecar 的账号 auth 文件里记着本地代理端口，而网关只按 config/manifest 指纹判断是否需要重启；
/// 隧道重建会换到新的随机端口，若不主动触发一次核对，运行中的 sidecar 会继续拨已消失的旧端口。
fn notify_local_access_gateway_after_proxy_change() {
    crate::modules::codex_local_access::trigger_gateway_reload_in_background("账号代理通道已重建");
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub account: &'static str,
    pub desktop: &'static str,
    pub sidecar: &'static str,
    pub account_port: Option<u16>,
    pub desktop_port: Option<u16>,
    pub sidecar_port: Option<u16>,
    pub account_node: Option<String>,
    pub desktop_node: Option<String>,
    pub sidecar_node: Option<String>,
    pub account_selection: Option<codex_proxy_engine::ProxySelection>,
    pub desktop_selection: Option<codex_proxy_engine::ProxySelection>,
    pub sidecar_selection: Option<codex_proxy_engine::ProxySelection>,
    pub desktop_entry: Option<super::codex_proxy_desktop_router::EntryStatus>,
    pub proxy_source: &'static str,
    #[serde(serialize_with = "crate::modules::codex_account_proxy::serialize_summary")]
    pub effective_proxy: Option<String>,
}

fn with_routing_status(
    mut result: RuntimeStatus,
    account: &CodexAccount,
    value: Option<&str>,
) -> RuntimeStatus {
    result.desktop_entry = super::codex_proxy_desktop_router::entry_status(&account.id);
    result.proxy_source = if value.is_none() {
        "none"
    } else if account
        .egress_proxy_url
        .as_deref()
        .is_some_and(|own| !own.trim().is_empty())
    {
        "account"
    } else {
        "unified"
    };
    result.effective_proxy = value.map(str::to_owned);
    result
}

fn tunnel_status(tunnel: Option<&NodeTunnel>, matches: bool) -> (&'static str, Option<u16>) {
    if !matches {
        return ("idle", None);
    }
    match tunnel {
        Some(tunnel) if tunnel.is_running() => (
            "running",
            url::Url::parse(tunnel.proxy_url())
                .ok()
                .and_then(|url| url.port()),
        ),
        Some(_) => ("stopped", None),
        None => ("idle", None),
    }
}

fn direct_proxy_has_credentials(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| !url.username().is_empty() || url.password().is_some())
}

pub(crate) fn desktop_engine_required(value: &str) -> bool {
    !is_direct(value) || direct_proxy_has_credentials(value)
}

fn initial_status(value: &str, engine_ready: bool) -> RuntimeStatus {
    let direct = is_direct(value);
    let engine_required = desktop_engine_required(value);
    RuntimeStatus {
        account: if direct {
            "direct"
        } else if engine_ready {
            "idle"
        } else {
            "missing"
        },
        desktop: if direct && !direct_proxy_has_credentials(value) {
            "direct"
        } else if engine_required && !engine_ready {
            "missing"
        } else {
            "idle"
        },
        sidecar: if direct && !direct_proxy_has_credentials(value) {
            "direct"
        } else if engine_required && !engine_ready {
            "missing"
        } else if !direct {
            if engine_ready {
                "idle"
            } else {
                "missing"
            }
        } else {
            "idle"
        },
        account_port: None,
        desktop_port: None,
        sidecar_port: None,
        account_node: None,
        desktop_node: None,
        sidecar_node: None,
        account_selection: None,
        desktop_selection: None,
        sidecar_selection: None,
        desktop_entry: None,
        proxy_source: "none",
        effective_proxy: None,
    }
}

/// Read-only: checking status never starts a core or probes a remote endpoint.
pub async fn status(account_id: &str) -> Result<RuntimeStatus, String> {
    let account = load(account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    let Some(value) = codex_account_proxy::configured_url(&account)? else {
        return Ok(with_routing_status(
            RuntimeStatus {
                account: "unbound",
                desktop: "unbound",
                sidecar: "unbound",
                account_port: None,
                desktop_port: None,
                sidecar_port: None,
                account_node: None,
                desktop_node: None,
                sidecar_node: None,
        account_selection: None,
        desktop_selection: None,
        sidecar_selection: None,
                desktop_entry: None,
                proxy_source: "none",
                effective_proxy: None,
            },
            &account,
            None,
        ));
    };
    let value = value.as_ref();
    let direct = is_direct(value);
    let engine_required = desktop_engine_required(value);
    // Direct unauthenticated proxies never depend on the optional engine or its disk state.
    let engine_ready = !engine_required || codex_proxy_engine::engine_ready().await?;
    let mut result = initial_status(value, engine_ready);
    let shared = RUNTIMES
        .lock()
        .map_err(|_| "PROXY_RUNTIME_FAILED")?
        .get(account_id)
        .cloned();
    let Some(shared) = shared else {
        return Ok(with_routing_status(result, &account, Some(value)));
    };
    let Ok(mut state) = shared.try_lock() else {
        if !direct {
            result.account = "starting";
        }
        if engine_required {
            result.desktop = "starting";
            if direct && direct_proxy_has_credentials(value) {
                result.sidecar = "starting";
            }
        }
        return Ok(with_routing_status(result, &account, Some(value)));
    };
    let mut result = observe(&mut state, value, direct, engine_ready);
    let reader = |tunnel: Option<&NodeTunnel>, running: bool| {
        if running {
            tunnel.and_then(NodeTunnel::selection_reader)
        } else {
            None
        }
    };
    let account_reader = reader(state.tunnel.as_ref(), result.account == "running");
    let desktop_reader = reader(
        state.desktop_tunnel.as_deref().map(Deref::deref),
        result.desktop == "running",
    );
    let sidecar_reader = reader(state.sidecar_tunnel.as_ref(), result.sidecar == "running");
    drop(state);
    async fn selected(reader: Option<codex_proxy_engine::SelectionReader>) -> Option<codex_proxy_engine::ProxySelection> {
        match reader {
            Some(reader) => reader.selected_info().await.ok().flatten(),
            None => None,
        }
    }
    let (a, d, s) = tokio::join!(
        selected(account_reader),
        selected(desktop_reader),
        selected(sidecar_reader)
    );
    result.account_node = a.as_ref().map(|value| value.name.clone());
    result.desktop_node = d.as_ref().map(|value| value.name.clone());
    result.account_selection = a;
    result.desktop_selection = d;
    result.sidecar_selection = if !direct { result.account_selection.clone() } else { s.clone() };
    result.sidecar_node = if !direct {
        result.account_node.clone()
    } else {
        s.map(|value| value.name)
    };
    Ok(with_routing_status(result, &account, Some(value)))
}

fn signature(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn observe(state: &mut Runtime, value: &str, direct: bool, engine_ready: bool) -> RuntimeStatus {
    let stamp = signature(value);
    let mut result = RuntimeStatus {
        account: if direct { "direct" } else { "idle" },
        desktop: if direct && !direct_proxy_has_credentials(value) {
            "direct"
        } else {
            "idle"
        },
        sidecar: if direct && !direct_proxy_has_credentials(value) {
            "direct"
        } else {
            "idle"
        },
        account_port: None,
        desktop_port: None,
        sidecar_port: None,
        account_node: None,
        desktop_node: None,
        sidecar_node: None,
        account_selection: None,
        desktop_selection: None,
        sidecar_selection: None,
        desktop_entry: None,
        proxy_source: "none",
        effective_proxy: None,
    };
    if !direct {
        if !engine_ready {
            result.account = "missing";
        } else if state.account_starting {
            result.account = "starting";
        } else {
            let matches = state.signature == stamp;
            (result.account, result.account_port) = tunnel_status(state.tunnel.as_ref(), matches);
        }
    }
    if desktop_engine_required(value) {
        if !engine_ready {
            result.desktop = "missing";
        } else if state.desktop_starting {
            result.desktop = "starting";
        } else {
            let matches = state.desktop_signature == stamp;
            (result.desktop, result.desktop_port) =
                tunnel_status(state.desktop_tunnel.as_deref().map(Deref::deref), matches);
        }
    }
    if !engine_ready && desktop_engine_required(value) {
        result.sidecar = "missing";
    } else if direct && direct_proxy_has_credentials(value) && state.sidecar_starting {
        result.sidecar = "starting";
    } else if direct && direct_proxy_has_credentials(value) {
        let matches = state.sidecar_signature == stamp;
        (result.sidecar, result.sidecar_port) =
            tunnel_status(state.sidecar_tunnel.as_ref(), matches);
    } else if !direct {
        result.sidecar = result.account;
        result.sidecar_port = result.account_port;
    }
    result
}

fn slot(account_id: &str) -> Result<Slot, String> {
    let mut store = RUNTIMES.lock().map_err(|_| "PROXY_RUNTIME_FAILED")?;
    if store.len() >= 256 && !store.contains_key(account_id) {
        return Err("PROXY_RUNTIME_LIMIT".into());
    }
    Ok(store.entry(account_id.to_string()).or_default().clone())
}

pub fn release_deleted_account(account_id: &str) {
    super::codex_proxy_activity::forget(account_id);
    let shared = RUNTIMES
        .lock()
        .ok()
        .and_then(|mut store| store.remove(account_id));
    if let Some(shared) = shared {
        tauri::async_runtime::spawn(async move {
            let mut state = shared.lock().await;
            let tunnel = state.tunnel.take();
            let desktop = state.desktop_tunnel.take();
            let sidecar = state.sidecar_tunnel.take();
            state.signature.clear();
            state.sidecar_signature.clear();
            drop(state);
            drop(desktop);
            for tunnel in [tunnel, sidecar].into_iter().flatten() {
                retire(tunnel).await;
            }
        });
    }
}

pub(crate) fn is_direct(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h"))
}

fn sidecar_needs_private_tunnel(value: &str) -> bool {
    is_direct(value) && direct_proxy_has_credentials(value)
}

pub fn normalize_binding(value: &str) -> Result<String, String> {
    if value.starts_with("cockpit-proxy://") {
        super::codex_proxy_catalog_binding::outbounds(value)?;
        return Ok(value.to_owned());
    }
    if is_direct(value) {
        return codex_account_proxy::normalize_direct_proxy(value);
    }
    crate::modules::codex_proxy_node_parser::parse_node_link(value)?;
    Ok(value.trim().to_string())
}

/// Preserve each caller's existing defaults when no ordinary-account binding exists.
pub async fn client_builder(
    account: &CodexAccount,
    builder: reqwest::ClientBuilder,
) -> Result<reqwest::ClientBuilder, String> {
    if !codex_account_proxy::eligible(account) {
        return Ok(builder);
    }
    let proxy_url = ensure(&account.id).await?;
    let Some(proxy_url) = proxy_url else {
        return Ok(builder);
    };
    let proxy = reqwest::Proxy::all(proxy_url).map_err(|_| "PROXY_INVALID_URL")?;
    Ok(builder.no_proxy().proxy(proxy))
}

pub(crate) async fn load(account_id: &str) -> Result<CodexAccount, String> {
    let permit = READS.try_acquire().map_err(|_| "PROXY_RUNTIME_LIMIT")?;
    let account_id = account_id.to_string();
    let account = tokio::time::timeout(
        Duration::from_secs(5),
        tauri::async_runtime::spawn_blocking(move || {
            let _permit = permit;
            codex_account::load_account(&account_id)
        }),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
    .map_err(|_| "PROXY_RUNTIME_FAILED")?
    .ok_or_else(|| "PROXY_ACCOUNT_UNSUPPORTED".to_string())?;
    ensure_account_proxy_state(&account).await?;
    Ok(account)
}

pub(crate) async fn ensure_account_proxy_state(account: &CodexAccount) -> Result<(), String> {
    if codex_account_proxy::eligible(account)
        && !account
            .egress_proxy_url
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        crate::modules::codex_unified_proxy::ensure_loaded().await?;
    }
    Ok(())
}

/// Synchronous manifest builders consume a PREPARED tunnel. Never start or wait here.
pub async fn prepare_accounts(account_ids: Vec<String>) -> Result<(), String> {
    use futures::{StreamExt, TryStreamExt};
    let stored = tauri::async_runtime::spawn_blocking(move || {
        account_ids
            .into_iter()
            .filter_map(|id| codex_account::load_account(&id))
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|_| "PROXY_RUNTIME_FAILED")?;
    let mut accounts = Vec::new();
    for account in stored {
        ensure_account_proxy_state(&account).await?;
        if codex_account_proxy::configured_url(&account)?.is_some_and(|value| {
            !is_direct(value.as_ref()) || sidecar_needs_private_tunnel(value.as_ref())
        }) {
            accounts.push(account.id);
        }
    }
    tokio::time::timeout(
        Duration::from_secs(25),
        futures::stream::iter(accounts)
            .map(|id| async move { ensure_sidecar(&id).await.map(|_| ()) })
            .buffer_unordered(4)
            .try_collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT")??;
    Ok(())
}

/// Synchronous manifest builders consume a PREPARED tunnel. Never start or wait here.
pub fn prepared_url(account: &CodexAccount) -> Result<Option<String>, String> {
    let Some(value) = codex_account_proxy::configured_url(account)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if is_direct(value) {
        return codex_account_proxy::normalize_direct_proxy(value).map(Some);
    }
    let shared = RUNTIMES
        .lock()
        .map_err(|_| "PROXY_RUNTIME_FAILED")?
        .get(&account.id)
        .cloned()
        .ok_or("PROXY_RUNTIME_NOT_READY")?;
    let mut state = shared.try_lock().map_err(|_| "PROXY_RUNTIME_NOT_READY")?;
    if state.signature != signature(value) {
        return Err("PROXY_RUNTIME_NOT_READY".into());
    }
    let tunnel = state.tunnel.as_mut().ok_or("PROXY_RUNTIME_NOT_READY")?;
    if !tunnel.is_running() {
        return Err("PROXY_ENGINE_STOPPED".into());
    }
    Ok(Some(tunnel.proxy_url().to_string()))
}

pub fn prepared_sidecar_url(account: &CodexAccount) -> Result<Option<String>, String> {
    let Some(value) = codex_account_proxy::configured_url(account)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if !sidecar_needs_private_tunnel(value) {
        return prepared_url(account);
    }
    let shared = RUNTIMES
        .lock()
        .map_err(|_| "PROXY_RUNTIME_FAILED")?
        .get(&account.id)
        .cloned()
        .ok_or("PROXY_RUNTIME_NOT_READY")?;
    let mut state = shared.try_lock().map_err(|_| "PROXY_RUNTIME_NOT_READY")?;
    if state.sidecar_signature != signature(value) {
        return Err("PROXY_RUNTIME_NOT_READY".into());
    }
    let tunnel = state
        .sidecar_tunnel
        .as_mut()
        .ok_or("PROXY_RUNTIME_NOT_READY")?;
    if !tunnel.is_running() {
        return Err("PROXY_ENGINE_STOPPED".into());
    }
    Ok(Some(tunnel.proxy_url().to_string()))
}

/// Ordinary account operations call this BEFORE building their client/manifest.
pub async fn ensure(account_id: &str) -> Result<Option<String>, String> {
    let account = load(account_id).await?;
    let Some(value) = codex_account_proxy::configured_url(&account)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if is_direct(value) {
        return codex_account_proxy::normalize_direct_proxy(value).map(Some);
    }
    let shared = slot(account_id)?;
    let mut state = tokio::time::timeout(Duration::from_secs(2), shared.lock())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?;
    let current = load(account_id).await?;
    let Some(value) = codex_account_proxy::configured_url(&current)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if is_direct(value) {
        return codex_account_proxy::normalize_direct_proxy(value).map(Some);
    }
    let stamp = signature(value);
    if state.signature == stamp {
        if let Some(tunnel) = state.tunnel.as_mut() {
            if tunnel.is_running() {
                return Ok(Some(tunnel.proxy_url().to_string()));
            }
        }
    }
    if state.account_starting {
        return Err("PROXY_RUNTIME_STARTING".into());
    }
    state.account_start_generation = state.account_start_generation.wrapping_add(1);
    let generation = state.account_start_generation;
    state.account_starting = true;
    drop(state);
    let mut start_guard = StartGuard::new(shared.clone(), StartKind::Account, generation);
    let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
        .map_err(|_| "PROXY_RUNTIME_FAILED")?;
    let tunnel_result = codex_proxy_engine::start(value).await;
    let mut state = shared.lock().await;
    if state.account_start_generation == generation {
        state.account_starting = false;
    }
    start_guard.complete();
    let tunnel = tunnel_result?;
    // Import/delete/update may have changed persistent state while the core started.
    let latest = load(account_id).await?;
    if codex_account_proxy::configured_url(&latest)?
        .map(|value| signature(value.as_ref()))
        .as_deref()
        != Some(stamp.as_str())
    {
        return Err("PROXY_BINDING_CHANGED".into());
    }
    let url = tunnel.proxy_url().to_string();
    let controller = tunnel.controller();
    let old = state.tunnel.replace(tunnel);
    state.signature = stamp;
    drop(state);
    super::codex_proxy_activity::attach_if_enabled(account_id, "account", controller);
    if let Some(old) = old {
        retire(old).await;
    }
    notify_local_access_gateway_after_proxy_change();
    Ok(Some(url))
}

/// Keep the selected engine alive for the lifetime of a desktop proxy connection.
pub(crate) async fn desktop_target(
    account_id: &str,
) -> Result<Option<(String, Option<Arc<DesktopTunnel>>)>, String> {
    let account = load(account_id).await?;
    let Some(value) = codex_account_proxy::configured_url(&account)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if is_direct(value) {
        let normalized = codex_account_proxy::normalize_direct_proxy(value)?;
        let url = url::Url::parse(&normalized).map_err(|_| "PROXY_INVALID_URL")?;
        if url.username().is_empty() && url.password().is_none() {
            return Ok(Some((normalized, None)));
        }
    }
    let shared = slot(account_id)?;
    let mut state = tokio::time::timeout(Duration::from_secs(2), shared.lock())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?;
    let stamp = signature(value);
    if state.desktop_signature == stamp {
        if let Some(tunnel) = state.desktop_tunnel.as_ref() {
            if tunnel.is_running() {
                return Ok(Some((tunnel.proxy_url().to_string(), Some(tunnel.clone()))));
            }
        }
    }
    if state.desktop_starting {
        return Err("PROXY_RUNTIME_STARTING".into());
    }
    state.desktop_start_generation = state.desktop_start_generation.wrapping_add(1);
    let generation = state.desktop_start_generation;
    state.desktop_starting = true;
    drop(state);
    let mut start_guard = StartGuard::new(shared.clone(), StartKind::Desktop, generation);
    let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
        .map_err(|_| "PROXY_RUNTIME_FAILED")?;
    let candidate_result = codex_proxy_engine::start_desktop(value).await;
    let mut state = shared.lock().await;
    if state.desktop_start_generation == generation {
        state.desktop_starting = false;
    }
    start_guard.complete();
    let candidate = Arc::new(DesktopTunnel(candidate_result?));
    drop(state);
    let latest = load(account_id).await?;
    if codex_account_proxy::configured_url(&latest)?
        .map(|value| signature(value.as_ref()))
        .as_deref()
        != Some(stamp.as_str())
    {
        return Err("PROXY_BINDING_CHANGED".into());
    }
    let mut state = shared.lock().await;
    if state.desktop_start_generation != generation {
        return Err("PROXY_BINDING_CHANGED".into());
    }
    let url = candidate.proxy_url().to_string();
    let controller = candidate.controller();
    let previous = state.desktop_tunnel.replace(candidate.clone());
    state.desktop_signature = stamp;
    drop(state);
    super::codex_proxy_activity::attach_if_enabled(account_id, "desktop", controller);
    drop(previous);
    Ok(Some((url, Some(candidate))))
}

pub async fn ensure_sidecar(account_id: &str) -> Result<Option<String>, String> {
    let account = load(account_id).await?;
    let Some(value) = codex_account_proxy::configured_url(&account)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if !sidecar_needs_private_tunnel(value) {
        return ensure(account_id).await;
    }
    let shared = slot(account_id)?;
    let mut state = tokio::time::timeout(Duration::from_secs(2), shared.lock())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?;
    let current = load(account_id).await?;
    let Some(value) = codex_account_proxy::configured_url(&current)? else {
        return Ok(None);
    };
    let value = value.as_ref();
    if !sidecar_needs_private_tunnel(value) {
        drop(state);
        return ensure(account_id).await;
    }
    let stamp = signature(value);
    if state.sidecar_signature == stamp {
        if let Some(tunnel) = state.sidecar_tunnel.as_mut() {
            if tunnel.is_running() {
                return Ok(Some(tunnel.proxy_url().to_string()));
            }
        }
    }
    if state.sidecar_starting {
        return Err("PROXY_RUNTIME_STARTING".into());
    }
    state.sidecar_start_generation = state.sidecar_start_generation.wrapping_add(1);
    let generation = state.sidecar_start_generation;
    state.sidecar_starting = true;
    drop(state);
    let mut start_guard = StartGuard::new(shared.clone(), StartKind::Sidecar, generation);
    let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
        .map_err(|_| "PROXY_RUNTIME_FAILED")?;
    let candidate_result = codex_proxy_engine::start_sidecar(value).await;
    let mut state = shared.lock().await;
    clear_starting(&mut state, StartKind::Sidecar, generation);
    start_guard.complete();
    let candidate = candidate_result?;
    let latest = load(account_id).await?;
    if codex_account_proxy::configured_url(&latest)?
        .map(|value| signature(value.as_ref()))
        .as_deref()
        != Some(stamp.as_str())
    {
        return Err("PROXY_BINDING_CHANGED".into());
    }
    let url = candidate.proxy_url().to_string();
    let controller = candidate.controller();
    let old = state.sidecar_tunnel.replace(candidate);
    state.sidecar_signature = stamp;
    drop(state);
    super::codex_proxy_activity::attach_if_enabled(account_id, "sidecar", controller);
    if let Some(old) = old {
        retire(old).await;
    }
    notify_local_access_gateway_after_proxy_change();
    Ok(Some(url))
}

/// A new node must start before durable binding replaces the old one. On failure,
/// dropping the candidate kills it and the previous binding/runtime stays intact.
/// Saving does not require an external probe; connectivity is checked explicitly.
pub async fn save_binding(
    account_id: String,
    input: Option<String>,
) -> Result<CodexAccount, String> {
    let account = load(&account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    let normalized = input
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_binding)
        .transpose()?;
    if let Some(value) = normalized.as_deref() {
        super::codex_proxy_engine_preflight::for_url(
            value,
            super::codex_proxy_engine_preflight::Usage::AccountRequest,
        ).await?;
    }
    let candidate = if let Some(value) = normalized.as_deref().filter(|value| !is_direct(value)) {
        let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
            .await
            .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
            .map_err(|_| "PROXY_RUNTIME_FAILED")?;
        Some(codex_proxy_engine::start(value).await?)
    } else {
        None
    };
    let sidecar_candidate = if let Some(value) = normalized
        .as_deref()
        .filter(|value| sidecar_needs_private_tunnel(value))
    {
        // The account's raw HTTP/SOCKS request path does not need Mihomo.
        // Preparing its private sidecar bridge is optional at save time; the
        // desktop/sidecar launch itself still fails closed if no engine is ready.
        let ready = match super::codex_proxy_engine_preflight::require().await {
            Ok(()) => true,
            Err(error) if super::codex_proxy_engine_preflight::is_prerequisite_error(&error) => {
                crate::modules::logger::log_info(&format!(
                    "[CodexProxy] raw request proxy does not require an engine; private bridge preparation deferred: {error}"
                ));
                false
            }
            Err(error) => return Err(error),
        };
        if !ready {
            None
        } else {
            let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
                .await
                .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
                .map_err(|_| "PROXY_RUNTIME_FAILED")?;
            Some(codex_proxy_engine::start_sidecar(value).await?)
        }
    } else {
        None
    };
    // Candidate startup is outside the account token lock; durable proxy
    // mutation then serializes with refresh/authority writes so old snapshots
    // cannot erase a newer binding.
    let token_lock = crate::modules::codex_account::codex_token_lock_for(&account_id);
    let _token_guard = token_lock.lock().await;
    let shared = slot(&account_id)?;
    let mut state = tokio::time::timeout(Duration::from_secs(15), shared.lock())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?;
    let persisted = normalized.clone();
    let account = tauri::async_runtime::spawn_blocking(move || {
        codex_account::update_account_egress_proxy(&account_id, persisted)
    })
    .await
    .map_err(|_| "PROXY_SAVE_FAILED")??;
    let account_controller = candidate.as_ref().map(NodeTunnel::controller);
    let sidecar_controller = sidecar_candidate.as_ref().map(NodeTunnel::controller);
    let old = std::mem::replace(&mut state.tunnel, candidate);
    let desktop = state.desktop_tunnel.take();
    let old_sidecar = std::mem::replace(&mut state.sidecar_tunnel, sidecar_candidate);
    state.desktop_signature.clear();
    state.sidecar_signature = normalized
        .as_deref()
        .filter(|value| sidecar_needs_private_tunnel(value))
        .filter(|_| state.sidecar_tunnel.is_some())
        .map(signature)
        .unwrap_or_default();
    state.signature = normalized.as_deref().map(signature).unwrap_or_default();
    drop(state);
    if let Some(controller) = account_controller {
        super::codex_proxy_activity::attach_if_enabled(&account.id, "account", controller);
    }
    if let Some(controller) = sidecar_controller {
        super::codex_proxy_activity::attach_if_enabled(&account.id, "sidecar", controller);
    }
    if let Some(old) = old {
        retire(old).await;
    }
    drop(desktop);
    if let Some(old_sidecar) = old_sidecar {
        retire(old_sidecar).await;
    }
    Ok(account)
}

/// Clear only a binding that still belongs to the deleted source after taking
/// the account token lock. A concurrent user rebind must not be overwritten.
pub async fn clear_binding_if_source(
    account_id: String,
    source_id: String,
) -> Result<Option<CodexAccount>, String> {
    let token_lock = codex_account::codex_token_lock_for(&account_id);
    let _token_guard = token_lock.lock().await;
    let shared = slot(&account_id)?;
    let mut state = tokio::time::timeout(Duration::from_secs(15), shared.lock())
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?;
    let account = tauri::async_runtime::spawn_blocking(move || {
        codex_account::clear_account_egress_proxy_for_source(&account_id, &source_id)
    })
    .await
    .map_err(|_| "PROXY_SAVE_FAILED")??;
    let Some(account) = account else {
        return Ok(None);
    };
    let old = state.tunnel.take();
    let desktop = state.desktop_tunnel.take();
    let old_sidecar = state.sidecar_tunnel.take();
    state.signature.clear();
    state.desktop_signature.clear();
    state.sidecar_signature.clear();
    drop(state);
    drop(desktop);
    for tunnel in [old, old_sidecar].into_iter().flatten() {
        retire(tunnel).await;
    }
    notify_local_access_gateway_after_proxy_change();
    Ok(Some(account))
}

/// 统一代理开关或出口变化后，让所有已缓存的隧道回到「待重建」状态。
///
/// 只失效、不预启动：真正需要出口的入口（切号、启动、sidecar 组装）会在自己的
/// `ensure*` 里按新配置重建，避免一次性拉起 N 个内核进程。
pub async fn invalidate_changed_bindings() {
    let slots = RUNTIMES
        .lock()
        .map(|store| {
            store
                .iter()
                .map(|(id, slot)| (id.clone(), slot.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (account_id, shared) in slots {
        let Ok(state) = shared.try_lock() else {
            continue;
        };
        let signatures = (
            state.signature.clone(),
            state.desktop_signature.clone(),
            state.sidecar_signature.clone(),
        );
        drop(state);
        let account = tauri::async_runtime::spawn_blocking({
            let account_id = account_id.clone();
            move || codex_account::load_account(&account_id)
        })
        .await
        .ok()
        .flatten();
        let desired = match account
            .as_ref()
            .map(codex_account_proxy::configured_url)
            .transpose()
        {
            Ok(value) => value
                .flatten()
                .map(|value| signature(value.as_ref()))
                .unwrap_or_default(),
            // A failed shared-config read cannot invalidate a known working route as if unbound.
            Err(_) => continue,
        };
        let Ok(mut state) = shared.try_lock() else {
            continue;
        };
        if signatures
            != (
                state.signature.clone(),
                state.desktop_signature.clone(),
                state.sidecar_signature.clone(),
            )
        {
            continue;
        }
        let account_changed = !state.signature.is_empty() && state.signature != desired;
        let desktop_changed =
            !state.desktop_signature.is_empty() && state.desktop_signature != desired;
        let sidecar_changed =
            !state.sidecar_signature.is_empty() && state.sidecar_signature != desired;
        let account_tunnel = if account_changed {
            state.signature.clear();
            state.tunnel.take()
        } else {
            None
        };
        let desktop = if desktop_changed {
            state.desktop_signature.clear();
            state.desktop_tunnel.take()
        } else {
            None
        };
        let sidecar_tunnel = if sidecar_changed {
            state.sidecar_signature.clear();
            state.sidecar_tunnel.take()
        } else {
            None
        };
        drop(state);
        drop(desktop);
        for tunnel in [account_tunnel, sidecar_tunnel].into_iter().flatten() {
            retire(tunnel).await;
        }
    }
}

/// 统一代理变更后的后台收敛：先失效旧隧道，再让 API 服务网关重新读取配置。
pub fn spawn_reload_after_unified_change() {
    tauri::async_runtime::spawn(async move {
        invalidate_changed_bindings().await;
        notify_local_access_gateway_after_proxy_change();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codex::CodexTokens;
    #[test]
    fn prepared_lookup_never_falls_back_for_unstarted_node() {
        let shared_state = crate::modules::codex_unified_proxy::TestCacheGuard::new();
        let mut account = CodexAccount::new(
            "unstarted-node-test".into(),
            "test@example.com".into(),
            CodexTokens {
                access_token: "access".into(),
                id_token: "id".into(),
                refresh_token: Some("refresh".into()),
            },
        );
        assert_eq!(prepared_url(&account).unwrap_err(), "UNIFIED_PROXY_LOADING");
        shared_state.prepare_disabled();
        assert_eq!(prepared_url(&account).unwrap(), None);
        account.egress_proxy_url = Some("http://127.0.0.1:8080".into());
        assert!(prepared_url(&account).unwrap().is_some());
        account.egress_proxy_url = Some("trojan://secret@example.com:443".into());
        assert!(prepared_url(&account).is_err());
        assert!(normalize_binding(account.egress_proxy_url.as_deref().unwrap()).is_ok());
    }

    #[test]
    fn status_reports_in_progress_starts_without_waiting() {
        let mut state = Runtime::default();
        state.account_starting = true;
        state.desktop_starting = true;
        let status = observe(&mut state, "trojan://secret@example.com:443", false, true);
        assert_eq!(status.account, "starting");
        assert_eq!(status.desktop, "starting");
        assert_eq!(status.account_port, None);
        state.account_starting = false;
        let status = observe(&mut state, "trojan://secret@example.com:443", false, true);
        assert_eq!(status.account, "idle");
        assert_eq!(status.desktop, "starting");
        let status = observe(&mut state, "http://127.0.0.1:8080", true, false);
        assert_eq!(status.account, "direct");
        assert_eq!(status.desktop, "direct");
        let missing = observe(
            &mut Runtime::default(),
            "trojan://secret@example.com:443",
            false,
            false,
        );
        assert_eq!(missing.account, "missing");
        assert_eq!(missing.desktop, "missing");
        assert_eq!(missing.sidecar, "missing");
    }

    #[test]
    fn cancelled_start_cleanup_does_not_clear_a_newer_attempt() {
        let mut state = Runtime::default();
        state.account_starting = true;
        state.account_start_generation = 9;
        clear_starting(&mut state, StartKind::Account, 8);
        assert!(state.account_starting);
        clear_starting(&mut state, StartKind::Account, 9);
        assert!(!state.account_starting);

        state.desktop_starting = true;
        state.desktop_start_generation = 4;
        clear_starting(&mut state, StartKind::Desktop, 4);
        assert!(!state.desktop_starting);
    }

    #[test]
    fn desktop_bridge_is_required_for_nodes_and_authenticated_direct_proxies() {
        assert!(!desktop_engine_required("http://proxy.example:8080"));
        assert!(desktop_engine_required(
            "http://user:pass@proxy.example:8080"
        ));
        assert!(desktop_engine_required("trojan://pass@node.example:443"));
    }

    #[test]
    fn initial_runtime_status_distinguishes_direct_and_engine_backed_paths() {
        let direct = initial_status("http://proxy.example:8080", false);
        assert_eq!(direct.account, "direct");
        assert_eq!(direct.desktop, "direct");
        let observed_direct = observe(
            &mut Runtime::default(),
            "http://proxy.example:8080",
            true,
            false,
        );
        assert_eq!(observed_direct.desktop, "direct");

        let authenticated = initial_status("http://user:pass@proxy.example:8080", false);
        assert_eq!(authenticated.account, "direct");
        assert_eq!(authenticated.desktop, "missing");

        let node = initial_status("trojan://pass@node.example:443", false);
        assert_eq!(node.account, "missing");
        assert_eq!(node.desktop, "missing");

        let prepared_node = initial_status("trojan://pass@node.example:443", true);
        assert_eq!(prepared_node.account, "idle");
        assert_eq!(prepared_node.desktop, "idle");
    }
}
