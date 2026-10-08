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
    tunnel: Option<Arc<DesktopTunnel>>,
    account_starting: bool,
    account_start_generation: u64,
}

pub(crate) struct DesktopTunnel(NodeTunnel, Arc<codex_proxy_engine::RequestRouteObserver>);

impl DesktopTunnel {
    fn new(tunnel: NodeTunnel, binding: &str) -> Self {
        let mut observer = tunnel.request_route_observer();
        decorate_request_route_observer(&mut observer, binding);
        Self(tunnel, Arc::new(observer))
    }

    pub(crate) fn request_route_observer(&self) -> Arc<codex_proxy_engine::RequestRouteObserver> {
        self.1.clone()
    }
}

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
static READS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
const MAX_RUNTIMES: usize = 256;
const READ_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);
// Includes queueing and blocking-pool scheduling, rather than restarting after admission.
const READ_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);

include!("codex_proxy_runtime_request_routes.rs");
include!("codex_proxy_runtime_latency.rs");

#[derive(Clone, Copy)]
enum StartKind {
    Account,
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
        _ => {}
    }
}

/// All consumers share one controller; old streams retain their Arc until EOF.
pub async fn active_controllers(account_id: &str) -> Vec<(&'static str, EngineController)> {
    let shared = RUNTIMES
        .lock()
        .ok()
        .and_then(|store| store.get(account_id).cloned());
    let Some(shared) = shared else {
        return Vec::new();
    };
    let Ok(state) = shared.try_lock() else {
        return Vec::new();
    };
    state
        .tunnel
        .as_ref()
        .filter(|tunnel| tunnel.is_running())
        .map(|tunnel| vec![("account", tunnel.controller())])
        .unwrap_or_default()
}

/// Binding changes still refresh sidecar policy/auth metadata; its dial port stays stable.
fn notify_local_access_gateway_after_proxy_change() {
    crate::modules::codex_local_access::trigger_gateway_reload_in_background("账号代理通道已重建");
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub shared_entry: bool,
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
    let port = result
        .desktop_entry
        .as_ref()
        .filter(|entry| entry.state == "listening")
        .and_then(|entry| entry.port);
    result.account_port = port;
    result.desktop_port = port;
    result.sidecar_port = port;
    result.proxy_source = if account.egress_proxy_disabled {
        "disabled"
    } else if value.is_none() {
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

pub(crate) fn desktop_engine_required(value: &str) -> bool {
    !is_direct(value)
}

fn initial_status(value: &str, engine_ready: bool) -> RuntimeStatus {
    let state = if is_direct(value) {
        "direct"
    } else if engine_ready {
        "idle"
    } else {
        "missing"
    };
    RuntimeStatus {
        shared_entry: true,
        account: state,
        desktop: state,
        sidecar: state,
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
        let mut result = initial_status("http://localhost", true);
        result.account = "unbound";
        result.desktop = "unbound";
        result.sidecar = "unbound";
        return Ok(with_routing_status(result, &account, None));
    };
    let value = value.as_ref();
    let direct = is_direct(value);
    let engine_ready = direct || codex_proxy_engine::engine_ready().await?;
    let mut result = initial_status(value, engine_ready);
    let shared = RUNTIMES
        .lock()
        .map_err(|_| "PROXY_RUNTIME_FAILED")?
        .get(account_id)
        .cloned();
    let reader = if let Some(shared) = shared {
        if let Ok(mut state) = shared.try_lock() {
            result = observe(&mut state, value, direct, engine_ready);
            state
                .tunnel
                .as_ref()
                .filter(|_| result.account == "running")
                .and_then(|tunnel| tunnel.selection_reader())
        } else {
            if !direct {
                result.account = "starting";
                result.desktop = "starting";
                result.sidecar = "starting";
            }
            None
        }
    } else {
        None
    };
    let selection = match reader {
        Some(reader) => reader.selected_info().await.ok().flatten(),
        None => None,
    };
    result.account_node = selection.as_ref().map(|value| value.name.clone());
    result.desktop_node = result.account_node.clone();
    result.sidecar_node = result.account_node.clone();
    result.account_selection = selection.clone();
    result.desktop_selection = selection.clone();
    result.sidecar_selection = selection;
    Ok(with_routing_status(result, &account, Some(value)))
}

fn signature(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn observe(state: &mut Runtime, value: &str, direct: bool, engine_ready: bool) -> RuntimeStatus {
    let mut result = initial_status(value, engine_ready);
    if !direct && engine_ready {
        result.account = if state.account_starting {
            "starting"
        } else {
            tunnel_status(
                state.tunnel.as_deref().map(Deref::deref),
                state.signature == signature(value),
            )
            .0
        };
    }
    result.desktop = result.account;
    result.sidecar = result.account;
    result
}

fn slot(account_id: &str) -> Result<Slot, String> {
    slot_in(&RUNTIMES, account_id, MAX_RUNTIMES)
}

fn existing_slot(account_id: &str) -> Result<Option<Slot>, String> {
    Ok(RUNTIMES
        .lock()
        .map_err(|_| "PROXY_RUNTIME_FAILED")?
        .get(account_id)
        .cloned())
}

fn tunnel_is_reclaimable<T>(
    tunnel: Option<&Arc<T>>,
    is_running: impl FnOnce(&T) -> Option<bool>,
) -> bool {
    tunnel.is_none_or(|tunnel| Arc::strong_count(tunnel) == 1 && is_running(tunnel) == Some(false))
}

/// Only reclaim unleased, inactive slots. Probe child state outside the registry lock;
/// remove only after rechecking ownership so concurrent starts keep their single-flight slot.
fn slot_in(
    registry: &Mutex<HashMap<String, Slot>>,
    account_id: &str,
    limit: usize,
) -> Result<Slot, String> {
    let candidates = {
        let mut store = registry.lock().map_err(|_| "PROXY_RUNTIME_FAILED")?;
        if let Some(shared) = store.get(account_id) {
            return Ok(shared.clone());
        }
        if store.len() < limit {
            return Ok(store.entry(account_id.to_owned()).or_default().clone());
        }
        // Snapshot identities only: holding every Slot would make concurrent reclamation
        // attempts mistake each other's scan leases for active account transactions.
        store.keys().cloned().collect::<Vec<_>>()
    };
    for id in &candidates {
        let shared = {
            let store = registry.lock().map_err(|_| "PROXY_RUNTIME_FAILED")?;
            if let Some(existing) = store.get(account_id) {
                return Ok(existing.clone());
            }
            // Claim at most one idle candidate under the registry lock. Another scanner
            // skips this one and can claim a different record without an Arc-count race.
            let Some(shared) = store
                .get(id)
                .filter(|shared| Arc::strong_count(shared) == 1)
            else {
                continue;
            };
            shared.clone()
        };
        let Ok(state) = shared.try_lock() else {
            continue;
        };
        if state.account_starting
            || !tunnel_is_reclaimable(state.tunnel.as_ref(), |tunnel| tunnel.try_is_running())
        {
            continue;
        }
        let mut store = registry.lock().map_err(|_| "PROXY_RUNTIME_FAILED")?;
        if let Some(existing) = store.get(account_id) {
            return Ok(existing.clone());
        }
        if Arc::strong_count(&shared) == 2
            && store
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, &shared))
        {
            // Our local lease keeps removed resources alive until the registry lock is gone.
            store.remove(id);
            return Ok(store.entry(account_id.to_owned()).or_default().clone());
        }
    }
    let mut store = registry.lock().map_err(|_| "PROXY_RUNTIME_FAILED")?;
    if let Some(shared) = store.get(account_id) {
        return Ok(shared.clone());
    }
    if store.len() < limit {
        return Ok(store.entry(account_id.to_owned()).or_default().clone());
    }
    Err("PROXY_RUNTIME_CAPACITY".into())
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
            state.signature.clear();
            state.account_start_generation = state.account_start_generation.wrapping_add(1);
            state.account_starting = false;
            let old = state.tunnel.take();
            drop(state);
            drop(old);
        });
    }
}

pub(crate) fn is_direct(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h"))
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
    // Refresh the routing choice before applying caller defaults.
    let current = load(&account.id).await?;
    if current.egress_proxy_disabled {
        return Ok(builder.no_proxy());
    }
    let proxy_url = ensure(&account.id).await?;
    let Some(proxy_url) = proxy_url else {
        return Ok(builder);
    };
    let proxy = reqwest::Proxy::all(proxy_url).map_err(|_| "PROXY_INVALID_URL")?;
    Ok(builder.no_proxy().proxy(proxy))
}

pub(crate) async fn load(account_id: &str) -> Result<CodexAccount, String> {
    let account = load_stored(account_id).await?;
    ensure_account_proxy_state(&account).await?;
    Ok(account)
}

async fn read_with_limit<T: Send + 'static>(
    reads: Arc<tokio::sync::Semaphore>,
    queue_timeout: Duration,
    total_timeout: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let started = tokio::time::Instant::now();
    let deadline = started + total_timeout;
    let permit = tokio::time::timeout_at(
        (started + queue_timeout).min(deadline),
        reads.acquire_owned(),
    )
    .await
    .map_err(|_| "PROXY_RUNTIME_BUSY")?
    .map_err(|_| "PROXY_RUNTIME_FAILED")?;
    tokio::time::timeout_at(
        deadline,
        tauri::async_runtime::spawn_blocking(move || {
            // Cancellation or a caller timeout cannot release admission while disk I/O runs.
            let _permit = permit;
            work()
        }),
    )
    .await
    .map_err(|_| "PROXY_RUNTIME_READ_TIMEOUT".to_owned())?
    .map_err(|_| "PROXY_RUNTIME_FAILED".to_owned())
}

async fn load_stored(account_id: &str) -> Result<CodexAccount, String> {
    let account_id = account_id.to_string();
    read_with_limit(
        READS.clone(),
        READ_QUEUE_TIMEOUT,
        READ_TOTAL_TIMEOUT,
        move || codex_account::load_account(&account_id),
    )
    .await?
    .ok_or_else(|| "PROXY_ACCOUNT_UNSUPPORTED".to_string())
}

pub(crate) async fn ensure_account_proxy_state(account: &CodexAccount) -> Result<(), String> {
    if codex_account_proxy::eligible(account)
        && !account.egress_proxy_disabled
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
        if codex_account_proxy::configured_url(&account)?.is_some() {
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

/// Manifest reads never start a listener or engine. Bound accounts fail closed until prepared.
pub fn prepared_url(account: &CodexAccount) -> Result<Option<String>, String> {
    let Some(value) = codex_account_proxy::configured_url(account)? else {
        return Ok(None);
    };
    if !is_direct(value.as_ref()) {
        let shared = RUNTIMES
            .lock()
            .map_err(|_| "PROXY_RUNTIME_FAILED")?
            .get(&account.id)
            .cloned()
            .ok_or("PROXY_RUNTIME_NOT_READY")?;
        let state = shared.try_lock().map_err(|_| "PROXY_RUNTIME_NOT_READY")?;
        if state.signature != signature(value.as_ref()) {
            return Err("PROXY_RUNTIME_NOT_READY".into());
        }
        if !state
            .tunnel
            .as_ref()
            .is_some_and(|tunnel| tunnel.is_running())
        {
            return Err("PROXY_ENGINE_STOPPED".into());
        }
    }
    super::codex_proxy_desktop_router::prepared_url(&account.id).map(Some)
}

pub fn prepared_sidecar_url(account: &CodexAccount) -> Result<Option<String>, String> {
    prepared_url(account)
}

/// Account HTTP clients and sidecars use the same stable entry as the desktop.
pub async fn ensure(account_id: &str) -> Result<Option<String>, String> {
    if desktop_target(account_id).await?.is_none() {
        return Ok(None);
    }
    super::codex_proxy_desktop_router::ensure_account_entry(account_id)
        .await
        .map(Some)
}

pub async fn ensure_sidecar(account_id: &str) -> Result<Option<String>, String> {
    ensure(account_id).await
}

/// This resolves the upstream, never the entry itself (which would form a loop).
/// The returned lease preserves in-flight streams across binding changes.
pub(crate) async fn desktop_target(
    account_id: &str,
) -> Result<Option<(String, Option<Arc<DesktopTunnel>>)>, String> {
    tokio::time::timeout(Duration::from_secs(25), ensure_target(account_id))
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT".to_string())?
}

async fn ensure_target(
    account_id: &str,
) -> Result<Option<(String, Option<Arc<DesktopTunnel>>)>, String> {
    loop {
        // Slow reads happen before the runtime lock, never while holding it.
        let account = load(account_id).await?;
        let Some(value) = codex_account_proxy::configured_url(&account)? else {
            return Ok(None);
        };
        let value = value.as_ref();
        if is_direct(value) {
            return Ok(Some((
                codex_account_proxy::normalize_direct_proxy(value)?,
                None,
            )));
        }
        let shared = slot(account_id)?;
        let stamp = signature(value);
        let mut state = shared.lock().await;
        if state.signature == stamp {
            if let Some(tunnel) = state.tunnel.as_ref().filter(|tunnel| tunnel.is_running()) {
                return Ok(Some((tunnel.proxy_url().to_string(), Some(tunnel.clone()))));
            }
        }
        if state.account_starting {
            drop(state);
            // Wait only on in-memory startup state. Do not reread account files every tick.
            loop {
                tokio::time::sleep(Duration::from_millis(25)).await;
                if !shared.lock().await.account_starting {
                    break;
                }
            }
            continue;
        }
        state.account_start_generation = state.account_start_generation.wrapping_add(1);
        let generation = state.account_start_generation;
        state.account_starting = true;
        drop(state);
        let mut guard = StartGuard::new(shared.clone(), StartKind::Account, generation);
        let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
            .await
            .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
            .map_err(|_| "PROXY_RUNTIME_FAILED")?;
        let candidate = Arc::new(DesktopTunnel::new(
            codex_proxy_engine::start(value).await?,
            value,
        ));
        // Serialize final validation/publication with durable binding updates.
        let token_lock = codex_account::codex_token_lock_for(account_id);
        let _token_guard = token_lock.lock().await;
        let latest = load(account_id).await?;
        if codex_account_proxy::configured_url(&latest)?
            .map(|value| signature(value.as_ref()))
            .as_deref()
            != Some(&stamp)
        {
            return Err("PROXY_BINDING_CHANGED".into());
        }
        let mut state = shared.lock().await;
        if state.account_start_generation != generation {
            return Err("PROXY_BINDING_CHANGED".into());
        }
        let old = state.tunnel.replace(candidate.clone());
        state.signature = stamp;
        clear_starting(&mut state, StartKind::Account, generation);
        guard.complete();
        drop(state);
        drop(old);
        super::codex_proxy_activity::attach_if_enabled(
            account_id,
            "account",
            candidate.controller(),
        );
        // The stable entry and its observation endpoint survive engine replacement;
        // restarting the API gateway here would interrupt unrelated active requests.
        return Ok(Some((candidate.proxy_url().to_string(), Some(candidate))));
    }
}

/// A new node must start before durable binding replaces the old one. On failure,
/// dropping the candidate kills it and the previous binding/runtime stays intact.
/// Saving does not require an external probe; connectivity is checked explicitly.
pub async fn save_binding(
    account_id: String,
    input: Option<String>,
) -> Result<CodexAccount, String> {
    save_binding_with_mode(account_id, input, false).await
}

/// Blocking writes outlive cancellation of their caller. Keep the account lock
/// with the write, then return it so publication stays in the same transaction.
async fn persist_binding_mutation<T: Send + 'static>(
    lock: Arc<tokio::sync::Mutex<()>>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<(T, tokio::sync::OwnedMutexGuard<()>), String> {
    let guard = lock.lock_owned().await;
    let (result, guard) = tauri::async_runtime::spawn_blocking(move || (work(), guard))
        .await
        .map_err(|_| "PROXY_SAVE_FAILED")?;
    Ok((result?, guard))
}

pub async fn save_binding_with_mode(
    account_id: String,
    input: Option<String>,
    disabled: bool,
) -> Result<CodexAccount, String> {
    save_binding_with_policy(account_id, input, disabled, false).await
}

/// Explicit pool allocation must not overwrite a manual binding made while a
/// candidate engine was starting. Recheck under the durable account token lock.
pub(crate) async fn save_binding_if_unbound(account_id: String, input: String) -> Result<CodexAccount, String> {
    save_binding_with_policy(account_id, Some(input), false, true).await
}

async fn save_binding_with_policy(account_id: String, input: Option<String>, disabled: bool, require_unbound: bool) -> Result<CodexAccount, String> {
    if disabled && input.is_some() {
        return Err("PROXY_INVALID_URL".into());
    }
    // Removing/replacing a binding must not depend on reading the previous shared config.
    let account = load_stored(&account_id).await?;
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
        )
        .await?;
    }
    // Reserve capacity before starting a core. Clearing/direct bindings need no new slot.
    let reserved = if normalized.as_deref().is_some_and(|value| !is_direct(value)) {
        Some(slot(&account_id)?)
    } else {
        None
    };
    let candidate = if let Some(value) = normalized.as_deref().filter(|value| !is_direct(value)) {
        let _permit = tokio::time::timeout(Duration::from_secs(12), STARTS.acquire())
            .await
            .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
            .map_err(|_| "PROXY_RUNTIME_FAILED")?;
        Some(codex_proxy_engine::start(value).await?)
    } else {
        None
    };
    let candidate = candidate.map(|tunnel| {
        Arc::new(DesktopTunnel::new(
            tunnel,
            normalized.as_deref().unwrap_or_default(),
        ))
    });
    // Candidate startup is outside the account token lock; durable proxy
    // mutation then serializes with refresh/authority writes so old snapshots
    // cannot erase a newer binding.
    let token_lock = crate::modules::codex_account::codex_token_lock_for(&account_id);
    let persisted = normalized.clone();
    let write_reservation = reserved.clone();
    let (account, _token_guard) = persist_binding_mutation(token_lock, move || {
        // A cancelled caller cannot make an in-progress binding transaction reclaimable.
        let _write_reservation = write_reservation;
        if require_unbound {
            let current = codex_account::load_account(&account_id).ok_or("PROXY_ACCOUNT_NOT_FOUND")?;
            if current.egress_proxy_url.is_some() || current.egress_proxy_disabled {
                return Err("codex.proxyQuality.errors.bindingChanged".into());
            }
        }
        codex_account::update_account_egress_proxy(&account_id, persisted, disabled)
    })
    .await?;
    // Publication is still under the token lock. Include starts that appeared during the
    // write, without allocating a record just to remove a binding at capacity.
    let shared = match reserved {
        Some(shared) => Some(shared),
        None => existing_slot(&account.id)?,
    };
    let Some(shared) = shared else {
        return Ok(account);
    };
    let mut state = shared.lock().await;
    let controller = candidate.as_ref().map(|tunnel| tunnel.controller());
    let old = std::mem::replace(&mut state.tunnel, candidate);
    state.signature = normalized.as_deref().map(signature).unwrap_or_default();
    state.account_start_generation = state.account_start_generation.wrapping_add(1);
    state.account_starting = false;
    drop(state);
    drop(old);
    if let Some(controller) = controller {
        super::codex_proxy_activity::attach_if_enabled(&account.id, "account", controller);
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
    let (account, _token_guard) = persist_binding_mutation(token_lock, move || {
        codex_account::clear_account_egress_proxy_for_source(&account_id, &source_id)
    })
    .await?;
    let Some(account) = account else {
        return Ok(None);
    };
    if let Some(shared) = existing_slot(&account.id)? {
        let mut state = shared.lock().await;
        let old = state.tunnel.take();
        state.signature.clear();
        state.account_start_generation = state.account_start_generation.wrapping_add(1);
        state.account_starting = false;
        drop(state);
        drop(old);
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
        let previous_signature = state.signature.clone();
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
        if previous_signature != state.signature {
            continue;
        }
        let old = if !state.signature.is_empty() && state.signature != desired {
            state.signature.clear();
            state.account_start_generation = state.account_start_generation.wrapping_add(1);
            state.account_starting = false;
            state.tunnel.take()
        } else {
            None
        };
        drop(state);
        drop(old);
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
        assert_eq!(
            prepared_url(&account).unwrap_err(),
            "PROXY_RUNTIME_NOT_READY"
        );
        account.egress_proxy_url = Some("trojan://secret@example.com:443".into());
        assert!(prepared_url(&account).is_err());
        assert!(normalize_binding(account.egress_proxy_url.as_deref().unwrap()).is_ok());
    }

    #[test]
    fn status_reports_in_progress_starts_without_waiting() {
        let mut state = Runtime::default();
        state.account_starting = true;
        let status = observe(&mut state, "trojan://secret@example.com:443", false, true);
        assert_eq!(status.account, "starting");
        assert_eq!(status.desktop, "starting");
        assert_eq!(status.account_port, None);
        state.account_starting = false;
        let status = observe(&mut state, "trojan://secret@example.com:443", false, true);
        assert_eq!(status.account, "idle");
        assert_eq!(status.desktop, "idle");
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
    }

    #[tokio::test]
    async fn cancelled_binding_write_keeps_account_locked_until_disk_work_finishes() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let (entered, waiting) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let task = tokio::spawn(persist_binding_mutation(lock.clone(), move || {
            entered.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        }));
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(
            lock.try_lock().is_err(),
            "cancelled caller must not unlock an active write"
        );
        release.send(()).unwrap();
        let _guard = tokio::time::timeout(Duration::from_secs(5), lock.lock())
            .await
            .expect("completed detached write must release its lock");
    }

    #[tokio::test]
    async fn binding_write_keeps_success_locked_for_publication_and_unlocks_on_error() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let (value, guard) = persist_binding_mutation(lock.clone(), || Ok(7))
            .await
            .unwrap();
        assert_eq!(value, 7);
        assert!(
            lock.try_lock().is_err(),
            "publication must retain the write lock"
        );
        drop(guard);
        assert_eq!(
            persist_binding_mutation(lock.clone(), || Err::<(), _>("write failed".to_owned()))
                .await
                .unwrap_err(),
            "write failed"
        );
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn shared_engine_is_required_only_for_node_protocols() {
        assert!(!desktop_engine_required("http://proxy.example:8080"));
        assert!(!desktop_engine_required(
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
        assert_eq!(authenticated.desktop, "direct");

        let node = initial_status("trojan://pass@node.example:443", false);
        assert_eq!(node.account, "missing");
        assert_eq!(node.desktop, "missing");

        let prepared_node = initial_status("trojan://pass@node.example:443", true);
        assert_eq!(prepared_node.account, "idle");
        assert_eq!(prepared_node.desktop, "idle");
    }
}

#[cfg(test)]
#[path = "codex_proxy_runtime_pressure_tests.rs"]
mod pressure_tests;
