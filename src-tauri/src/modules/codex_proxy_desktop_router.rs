//! A stable, per-account SOCKS5 / HTTP CONNECT endpoint shared by account requests,
//! API sidecars, and Codex desktop processes.
//! Each new connection resolves the current account/unified binding. Existing
//! connections hold their engine lease until they finish, so changing a proxy
//! never leaves a running desktop process pointing at a retired random port.
//!
//! 同一入口兼容 SOCKS5 与 HTTP CONNECT（仅 TCP）。每条新连接都会重新解析当前绑定：有绑定就走 Mihomo 出口，
//! 未绑定（例如运行中解绑）则直接连客户端给出的目标主机名，出口 IP 与客户端自己直连一致；
//! UDP/QUIC 由客户端按 SOCKS 代理语义处理（Chromium 不会把 UDP 交给 SOCKS 入口），因此
//! 这种链路不能描述成与“完全直连”等价。
//!
//! 启动时只为**已有生效出口**（独立绑定或统一代理）的账号安装入口：未绑定账号若也被注入
//! 本地入口，会替代应用全局代理环境变量、用户自定义的 `--proxy-server`/PAC 参数与系统代理，
//! 属于静默改变出口。因此未绑定账号保持原有启动参数，首次绑定后需要重启一次，之后换节点、
//! 换策略或解绑都不再需要重启。显式“不使用代理”也安装固定入口以覆盖启动时的代理默认值。
//!
//! 端口属于账号入口而不是某次内核：分配结果持久化在应用数据目录的
//! `codex-proxy-desktop-ports.json`，宿主重启后优先复用，端口被占用时按确定性顺序探测
//! 下一个端口并写回记录。
use super::{
    account, atomic_write, codex_account_proxy,
    codex_proxy_runtime::{self, DesktopTunnel},
    logger,
};
use crate::models::codex::CodexAccount;
use base64::Engine as _;
use rustls::{pki_types::ServerName, ClientConfig, RootCertStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, OnceCell, Semaphore},
};
use tokio_rustls::TlsConnector;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_CONNECTIONS: usize = 256;
const PORTS_FILE: &str = "codex-proxy-desktop-ports.json";
const PORTS_VERSION: u32 = 1;
/// 起始端口区间与既有确定性算法保持一致，避免升级后端口整体变化。
const PORT_RANGE_START: u16 = 40_000;
const PORT_RANGE_SIZE: u16 = 20_000;
/// 记录端口被占用时向后探测的次数，耗尽即判定入口不可用并回退。
const PORT_ATTEMPTS: u16 = 64;
const MAX_PORT_RECORDS: usize = 1024;
const REGISTRY_IO_TIMEOUT: Duration = Duration::from_secs(2);
static ROUTES: LazyLock<Mutex<HashMap<String, u16>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static ENTRY_STARTS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
// Only acquired inside spawn_blocking; atomic read/modify/write cannot lose another account's port.
static REGISTRY_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn entry_start_lock(account_id: &str) -> Arc<Mutex<()>> {
    ENTRY_STARTS.lock().await.entry(account_id.to_owned()).or_default().clone()
}

static TLS_CONFIG: OnceCell<Arc<ClientConfig>> = OnceCell::const_new();

#[path = "codex_proxy_desktop_entry.rs"]
mod entry;
#[path = "codex_proxy_desktop_routes.rs"]
mod request_routes;

pub(crate) fn request_route_observer(proxy_url: &str) -> Option<super::codex_proxy_engine::RequestRouteObserver> {
    request_routes::observer(proxy_url)
}

#[path = "codex_proxy_desktop_protocol.rs"]
mod protocol;
#[path = "codex_proxy_desktop_diagnostics.rs"]
mod diagnostics;
#[path = "codex_proxy_desktop_processes.rs"]
mod processes;
#[path = "codex_proxy_desktop_recovery.rs"]
mod recovery;
pub use recovery::{restore_on_startup, restore_account_entry};
pub use entry::Status as EntryStatus;

pub fn entry_status(account_id: &str) -> Option<EntryStatus> {
    entry::snapshot(account_id)
}

trait ProxyIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ProxyIo for T {}
type BoxIo = Box<dyn ProxyIo>;

#[derive(Debug)]
struct Destination {
    host: String,
    port: u16,
}

impl Destination {
    fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// 还没有端口记录时的起点。保持与旧算法一致，让升级后的首次分配沿用原端口。
fn preferred_port(account_id: &str) -> u16 {
    let digest = Sha256::digest(account_id.as_bytes());
    PORT_RANGE_START + u16::from_be_bytes([digest[0], digest[1]]) % PORT_RANGE_SIZE
}

fn is_managed_port(port: u16) -> bool {
    port >= PORT_RANGE_START && port < PORT_RANGE_START + PORT_RANGE_SIZE
}

/// 候选端口：优先复用已记录端口，其次从确定性起点逐个向后探测并在区间内回绕。
fn candidate_ports(account_id: &str, recorded: Option<u16>) -> impl Iterator<Item = u16> {
    let start = recorded
        .filter(|port| is_managed_port(*port))
        .unwrap_or_else(|| preferred_port(account_id));
    let offset = u32::from(start - PORT_RANGE_START);
    (0..PORT_ATTEMPTS).map(move |step| {
        PORT_RANGE_START + ((offset + u32::from(step)) % u32::from(PORT_RANGE_SIZE)) as u16
    })
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PortRegistry {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    ports: BTreeMap<String, u16>,
}

impl PortRegistry {
    fn recorded(&self, account_id: &str) -> Option<u16> {
        self.ports
            .get(account_id)
            .copied()
            .filter(|port| is_managed_port(*port))
    }

    fn record(&mut self, account_id: &str, port: u16) {
        self.version = PORTS_VERSION;
        self.ports.insert(account_id.to_owned(), port);
    }
}

fn registry_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(PORTS_FILE))
}

/// 记录缺失、损坏或超限时只当成“还没有记录”。这里不隔离用户文件：端口记录可以重新
/// 分配，损坏时重算比移动文件更安全。
fn read_registry(path: &Path) -> PortRegistry {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return PortRegistry::default()
        }
        Err(error) => {
            logger::log_warn(&format!("[CodexProxy] 读取桌面入口端口记录失败: {error}"));
            return PortRegistry::default();
        }
    };
    match serde_json::from_str::<PortRegistry>(&content) {
        Ok(registry) if registry.ports.len() <= MAX_PORT_RECORDS => registry,
        Ok(_) => {
            logger::log_warn("[CodexProxy] 桌面入口端口记录条目过多，本次忽略该文件");
            PortRegistry::default()
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexProxy] 桌面入口端口记录解析失败，本次忽略该文件: {error}"
            ));
            PortRegistry::default()
        }
    }
}

fn save_registry(path: &Path, registry: &PortRegistry) -> Result<(), String> {
    let content =
        serde_json::to_string_pretty(registry).map_err(|_| "PROXY_ENTRY_PORTS_SERIALIZE")?;
    atomic_write::write_string_atomic(path, &content)
}

/// 端口记录读写放在阻塞线程池，超时或失败都只降级为“本次不持久化”，不影响入口可用性。
async fn registry_io<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    match tokio::time::timeout(REGISTRY_IO_TIMEOUT, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(value)) => Some(value),
        _ => {
            logger::log_warn("[CodexProxy] 桌面入口端口记录读写超时，本次只使用内存分配");
            None
        }
    }
}

/// 绑定账号固定入口。
///
/// 调用方持账号独立的启动锁，因此同一账号不会注册出两个入口，慢磁盘不会占住全局路由锁；
/// 记录端口被占用时改绑确定性顺序里的下一个端口并写回记录。
async fn install(account_id: &str, path: Option<&Path>) -> Result<(TcpListener, u16), String> {
    let recorded = match path {
        Some(path) => {
            let path = path.to_path_buf();
            let account_id = account_id.to_owned();
            registry_io(move || read_registry(&path).recorded(&account_id))
                .await
                .flatten()
        }
        None => None,
    };
    for port in candidate_ports(account_id, recorded) {
        let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await else {
            continue;
        };
        persist_port(path, account_id, port).await;
        return Ok((listener, port));
    }
    Err("PROXY_ENTRY_PORT_UNAVAILABLE".into())
}

/// 只在端口与记录不一致时写回，避免每次启动重写同一个文件。
async fn persist_port(path: Option<&Path>, account_id: &str, port: u16) {
    let Some(path) = path else { return };
    let path = path.to_path_buf();
    let account_id = account_id.to_owned();
    let outcome = registry_io(move || {
        let _write = REGISTRY_WRITES.lock().unwrap_or_else(|error| error.into_inner());
        let mut registry = read_registry(&path);
        if registry.recorded(&account_id) == Some(port) {
            return Ok(());
        }
        registry.record(&account_id, port);
        save_registry(&path, &registry)
    })
    .await;
    match outcome {
        Some(Ok(())) => {}
        Some(Err(error)) => logger::log_warn(&format!(
            "[CodexProxy] 桌面入口端口记录写入失败，本次运行继续使用已绑定端口: error={error}, port={port}"
        )),
        None => {}
    }
}

fn entry_url(port: u16) -> String {
    format!("socks5h://127.0.0.1:{port}")
}

#[cfg(test)]
async fn route_port(account_id: &str) -> Option<u16> {
    ROUTES.lock().await.get(account_id).copied()
}

/// Returns the same loopback URL for an account throughout this host process, and the
/// recorded port after a host restart. The entry only needs to be installed once per
/// process; each connection resolves the current binding on its own.
pub async fn ensure(account_id: &str) -> Result<Option<String>, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Ok(None);
    }
    // 已存在的监听仍供运行中的客户端使用，但不能证明这次启动还应注入代理。
    // 先读取当前账号并检查生效出口，再由 ensure_route 复用对应端口。
    let account = codex_proxy_runtime::load(account_id).await?;
    let path = match registry_path() {
        Ok(path) => Some(path),
        Err(error) => {
            logger::log_warn(&format!(
                "[CodexProxy] 无法定位桌面入口端口记录目录，本次只使用内存端口: error={error}"
            ));
            None
        }
    };
    let proxy_active = codex_account_proxy::has_configured_url(&account)?;
    // A configured route must either be injected or fail explicitly. Returning
    // None on listener failure silently launches through inherited/system proxies.
    ensure_for_account(account_id, &account, proxy_active, path.as_deref()).await
}

/// 资格与生效出口检查、入口安装拆开，便于直接覆盖「通过 eligible 但尚未绑定代理」的场景。
/// `unified_active` 由调用方传入，测试可据此显式构造两种状态，不依赖本机统一代理配置。
async fn ensure_for_account(
    account_id: &str,
    account: &CodexAccount,
    unified_active: bool,
    path: Option<&Path>,
) -> Result<Option<String>, String> {
    if !codex_account_proxy::eligible(account) {
        return Ok(None);
    }
    // An explicit direct choice also uses the stable entry, overriding launch proxy defaults.
    if !account.egress_proxy_disabled
        && !codex_account_proxy::has_effective_proxy(account, unified_active)
    {
        return Ok(None);
    }
    Ok(Some(ensure_route(account_id, path).await?))
}

/// Bound account requests must fail closed if their stable entry cannot be installed.
pub(crate) async fn ensure_account_entry(account_id: &str) -> Result<String, String> {
    let path = registry_path()?;
    tokio::time::timeout(Duration::from_secs(25), ensure_route(account_id, Some(&path)))
        .await.map_err(|_| "PROXY_ENGINE_TIMEOUT".to_string())?
}

/// Synchronous, read-only manifest lookup; never creates or waits for a listener.
pub(crate) fn prepared_url(account_id: &str) -> Result<String, String> {
    entry_status(account_id)
        .filter(|status| status.state == "listening")
        .and_then(|status| status.port)
        .map(entry_url)
        .ok_or_else(|| "PROXY_RUNTIME_NOT_READY".to_string())
}

async fn ensure_route(account_id: &str, path: Option<&Path>) -> Result<String, String> {
    let start = entry_start_lock(account_id).await;
    let _start = start.lock().await;
    let existing = ROUTES.lock().await.get(account_id).copied();
    if let Some(port) = existing {
        if entry_status(account_id)
            .is_some_and(|status| status.state == "listening" && status.port == Some(port))
        {
            return Ok(entry_url(port));
        }
    }
    let (listener, port) = match install(account_id, path).await {
        Ok(installed) => installed,
        Err(error) => {
            entry::failed_to_listen(account_id);
            return Err(error);
        }
    };
    spawn_listener(account_id, port, listener);
    ROUTES.lock().await.insert(account_id.to_owned(), port);
    Ok(entry_url(port))
}

fn spawn_listener(account_id: &str, port: u16, listener: TcpListener) {
    let observation = entry::listening(account_id, port);
    let id = account_id.to_owned();
    tokio::spawn(async move { serve(listener, id, observation).await });
}

/// Running clients cannot adopt another port. Recovery never rewrites the registry.
async fn restore_exact_port(account_id: &str, port: u16) -> Result<(), String> {
    if !is_managed_port(port) {
        return Err("PROXY_ENTRY_RECOVERY_FAILED".into());
    }
    let start = entry_start_lock(account_id).await;
    let _start = tokio::time::timeout(Duration::from_secs(3), start.lock())
        .await.map_err(|_| "PROXY_ENTRY_RECOVERY_TIMEOUT")?;
    if crate::modules::app_lifecycle::is_shutdown_started() {
        return Err("PROXY_ENTRY_RECOVERY_FAILED".into());
    }
    let existing = ROUTES.lock().await.get(account_id).copied();
    if let Some(current) = existing {
        if entry_status(account_id).is_some_and(|status| status.state == "listening" && status.port == Some(current)) {
            return if current == port { Ok(()) } else { Err("PROXY_ENTRY_PORT_UNAVAILABLE".into()) };
        }
    }
    let listener = match TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
        Ok(listener) => listener,
        Err(_) => {
            entry::recovery_failed(account_id, port, "PROXY_ENTRY_PORT_UNAVAILABLE");
            return Err("PROXY_ENTRY_PORT_UNAVAILABLE".into());
        }
    };
    spawn_listener(account_id, port, listener);
    ROUTES.lock().await.insert(account_id.to_owned(), port);
    Ok(())
}

async fn serve(listener: TcpListener, account_id: String, observation: entry::Listener) {
    let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(error) => {
                observation.failed();
                logger::log_warn(&format!("[CodexProxy] 桌面入口监听失败: {error}"));
                break;
            }
        };
        let Ok(permit) = permits.clone().try_acquire_owned() else {
            logger::log_warn(&format!("[CodexProxy] connection_rejected: reason=capacity, limit={MAX_CONNECTIONS}, entry_port={}, source_port={}", stream.local_addr().map(|addr| addr.port()).unwrap_or(0), stream.peer_addr().map(|addr| addr.port()).unwrap_or(0)));
            drop(stream);
            continue;
        };
        let id = account_id.clone();
        let observed_entry = observation.entry();
        tokio::spawn(async move {
            let _permit = permit;
            let _ = handle(stream, &id, observed_entry).await;
        });
    }
}

async fn handle(
    mut client: TcpStream,
    account_id: &str,
    observed_entry: Arc<entry::Entry>,
) -> Result<(), String> {
    let diagnostic = diagnostics::Connection::new(
        client.local_addr().map(|addr| addr.port()).unwrap_or(0),
        client.peer_addr().map(|addr| addr.port()).unwrap_or(0),
    );
    diagnostic.run(handle_connection(&mut client, account_id, observed_entry, &diagnostic)).await
}

async fn handle_connection(
    mut client: &mut TcpStream,
    account_id: &str,
    observed_entry: Arc<entry::Entry>,
    diagnostic: &diagnostics::Connection,
) -> Result<(), String> {
    diagnostic.phase("ingress_probe");
    if tokio::time::timeout(CONNECT_TIMEOUT, request_routes::try_serve(&mut client, account_id))
        .await.map_err(|_| "PROXY_ENGINE_TIMEOUT")?? {
        return Ok(());
    }
    let source = client.peer_addr().map_err(|e| diagnostics::io_failed("client_peer_addr", &e))?;
    diagnostic.phase("handshake");
    let mut protocol = None;
    let handshake = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let detected = protocol::Protocol::detect(&client).await?;
        protocol = Some(detected);
        let destination = detected.read_destination(&mut client).await?;
        Ok::<_, String>((detected, destination))
    })
    .await
    .unwrap_or_else(|_| Err("PROXY_ENGINE_TIMEOUT".into()));
    let (protocol, destination) = match handshake {
        Ok(request) => request,
        Err(error) => {
            diagnostic.failed(protocol, "handshake", &error);
            if let Some(protocol::Protocol::HttpConnect) = protocol {
                let _ = protocol::Protocol::HttpConnect
                    .reply(&mut client, protocol::Reply::BadRequest)
                    .await;
            }
            return Err(error);
        }
    };
    // A TCP accept or SOCKS greeting alone is not a proxy connection request.
    let request = observed_entry.request();
    let upstream = tokio::time::timeout(CONNECT_TIMEOUT, async {
        diagnostic.phase("resolve_route");
        let target = resolve_desktop_target(account_id).await?;
        diagnostic.route(target.as_ref().and_then(|(_, lease)| lease.as_ref().map(|tunnel| tunnel.diagnostic_id())), None);
        diagnostic.phase("connect_upstream");
        let target_url = target.as_ref().map(|(url, _)| url.clone());
        let (stream, lease, upstream_addr) = dial_observed(&destination, target).await?;
        diagnostic.route(lease.as_ref().map(|tunnel| tunnel.diagnostic_id()), upstream_addr.map(|addr| addr.port()));
        let route = request_routes::register(account_id, source, upstream_addr, target_url.as_deref(), lease.as_deref());
        Ok::<_, String>((stream, lease, route))
    })
    .await
    .unwrap_or_else(|_| Err("PROXY_ENGINE_TIMEOUT".into()));
    let (mut upstream, _lease, _route) = match upstream {
        Ok(value) => value,
        Err(error) => {
            request.failed(&error);
            diagnostic.failed(Some(protocol), "upstream", &error);
            let _ = protocol
                .reply(&mut client, protocol::Reply::BadGateway)
                .await;
            return Err(error);
        }
    };
    diagnostic.phase("reply");
    protocol
        .reply(&mut client, protocol::Reply::Established)
        .await
        .map_err(|error| {
            diagnostic.failed(Some(protocol), "reply", &error);
            error
        })?;
    request.forwarded();
    diagnostic.phase("relay");
    diagnostic
        .relay(protocol, &mut client, &mut upstream)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    Ok(())
}

/// 每条新连接都重新解析当前生效出口：换节点、换策略、解绑或统一代理变化都会在
/// 下一批新连接上立即生效，不需要重启已经运行中的客户端。
async fn resolve_desktop_target(
    account_id: &str,
) -> Result<Option<(String, Option<Arc<DesktopTunnel>>)>, String> {
    // The lease keeps an old Mihomo instance alive for in-flight HTTP/WebSocket
    // connections even if a new binding replaces it while this stream is open.
    match codex_proxy_runtime::desktop_target(account_id).await {
        Err(error) if error == "PROXY_RUNTIME_STARTING" || error == "PROXY_BINDING_CHANGED" => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            codex_proxy_runtime::desktop_target(account_id).await
        }
        result => result,
    }
}

/// 有绑定就走当前出口，没有绑定（未绑定且无统一代理）则在本地按 SOCKS 语义直连目标主机名。
#[cfg(test)]
async fn dial(
    destination: &Destination,
    target: Option<(String, Option<Arc<DesktopTunnel>>)>,
) -> Result<(BoxIo, Option<Arc<DesktopTunnel>>), String> {
    let (stream, lease, _) = dial_observed(destination, target).await?;
    Ok((stream, lease))
}

async fn dial_observed(
    destination: &Destination,
    target: Option<(String, Option<Arc<DesktopTunnel>>)>,
) -> Result<(BoxIo, Option<Arc<DesktopTunnel>>, Option<std::net::SocketAddr>), String> {
    match target {
        Some((url, lease)) => {
            let (stream, source) = connect_via_proxy(&url, destination).await?;
            Ok((stream, lease, Some(source)))
        }
        None => Ok((connect_direct(destination).await?, None, None)),
    }
}

/// 未绑定账号的出口沿用客户端原本的直连结果：直接连目标主机名，不改写地址。
async fn connect_direct(destination: &Destination) -> Result<BoxIo, String> {
    let stream = TcpStream::connect((destination.host.as_str(), destination.port))
        .await
        .map_err(|e| diagnostics::io_failed("direct_tcp_connect", &e))?;
    Ok(Box::new(stream))
}

async fn read_socks_request<S>(client: &mut S) -> Result<Destination, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut greeting = [0u8; 2];
    client
        .read_exact(&mut greeting)
        .await
        .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
    if greeting[0] != 5 || greeting[1] == 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    client
        .read_exact(&mut methods)
        .await
        .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
    if !methods.contains(&0) {
        let _ = client.write_all(&[5, 255]).await;
        return Err("PROXY_CONNECT_FAILED".into());
    }
    client
        .write_all(&[5, 0])
        .await
        .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
    let mut head = [0u8; 4];
    client
        .read_exact(&mut head)
        .await
        .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
    if head[0] != 5 || head[1] != 1 || head[2] != 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let host = match head[3] {
        1 => {
            let mut octets = [0u8; 4];
            client
                .read_exact(&mut octets)
                .await
                .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
            IpAddr::V4(Ipv4Addr::from(octets)).to_string()
        }
        4 => {
            let mut octets = [0u8; 16];
            client
                .read_exact(&mut octets)
                .await
                .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
            IpAddr::V6(Ipv6Addr::from(octets)).to_string()
        }
        3 => {
            let mut length = [0u8; 1];
            client
                .read_exact(&mut length)
                .await
                .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
            if length[0] == 0 {
                return Err("PROXY_CONNECT_FAILED".into());
            }
            let mut name = vec![0u8; length[0] as usize];
            client
                .read_exact(&mut name)
                .await
                .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
            String::from_utf8(name).map_err(|_| "PROXY_CONNECT_FAILED")?
        }
        _ => return Err("PROXY_CONNECT_FAILED".into()),
    };
    let mut port = [0u8; 2];
    client
        .read_exact(&mut port)
        .await
        .map_err(|e| diagnostics::io_failed("ingress_socks_io", &e))?;
    let port = u16::from_be_bytes(port);
    if port == 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    Ok(Destination { host, port })
}

async fn connect_via_proxy(proxy_url: &str, target: &Destination) -> Result<(BoxIo, std::net::SocketAddr), String> {
    let url = url::Url::parse(proxy_url).map_err(|_| "PROXY_INVALID_URL")?;
    let host = url
        .host_str()
        .ok_or("PROXY_INVALID_URL")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url.port_or_known_default().ok_or("PROXY_INVALID_URL")?;
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| diagnostics::io_failed("proxy_tcp_connect", &e))?;
    let source = tcp.local_addr().map_err(|e| diagnostics::io_failed("upstream_local_addr", &e))?;
    diagnostics::upstream_socket(source.port());
    let stream = match url.scheme() {
        "http" => http_connect(Box::new(tcp), &url, target).await,
        "https" => {
            let config = TLS_CONFIG
                .get_or_try_init(|| async {
                    tokio::task::spawn_blocking(|| {
                        let certs = rustls_native_certs::load_native_certs().certs;
                        let mut roots = RootCertStore::empty();
                        roots.add_parsable_certificates(certs);
                        if roots.is_empty() {
                            return Err("PROXY_CONNECT_FAILED".to_string());
                        }
                        Ok(Arc::new(
                            ClientConfig::builder()
                                .with_root_certificates(roots)
                                .with_no_client_auth(),
                        ))
                    })
                    .await
                    .map_err(|_| "PROXY_CONNECT_FAILED".to_string())?
                })
                .await?
                .clone();
            let name = ServerName::try_from(host.to_owned()).map_err(|_| "PROXY_INVALID_URL")?;
            let tls = TlsConnector::from(config)
                .connect(name, tcp)
                .await
                .map_err(|e| diagnostics::io_failed("proxy_tls_handshake", &e))?;
            http_connect(Box::new(tls), &url, target).await
        }
        "socks5" | "socks5h" => socks_connect(Box::new(tcp), &url, target).await,
        _ => Err("PROXY_UNSUPPORTED_PROTOCOL".into()),
    }?;
    Ok((stream, source))
}

fn credentials(url: &url::Url) -> Result<Option<(String, String)>, String> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let username = urlencoding::decode(url.username())
        .map_err(|_| "PROXY_INVALID_URL")?
        .into_owned();
    let password = urlencoding::decode(url.password().unwrap_or(""))
        .map_err(|_| "PROXY_INVALID_URL")?
        .into_owned();
    Ok(Some((username, password)))
}

async fn http_connect(
    mut stream: BoxIo,
    url: &url::Url,
    target: &Destination,
) -> Result<BoxIo, String> {
    let authority = target.authority();
    let auth = credentials(url)?
        .map(|(user, password)| {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            format!("Proxy-Authorization: Basic {encoded}\r\n")
        })
        .unwrap_or_default();
    let request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{auth}\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| diagnostics::io_failed("upstream_http_connect_io", &e))?;
    let mut response = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    while response.len() < 16 * 1024 {
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|e| diagnostics::io_failed("upstream_http_connect_io", &e))?;
        response.push(byte[0]);
        if response.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    if !response.ends_with(b"\r\n\r\n") {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let first = response
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or("PROXY_CONNECT_FAILED")?;
    diagnostics::protocol_reply("http_connect", std::str::from_utf8(first).ok()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok()));
    if !first.starts_with(b"HTTP/1.") || !first.windows(4).any(|value| value == b" 200") {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    Ok(stream)
}

async fn socks_connect(
    mut stream: BoxIo,
    url: &url::Url,
    target: &Destination,
) -> Result<BoxIo, String> {
    let auth = credentials(url)?;
    stream
        .write_all(&[5, 1, if auth.is_some() { 2 } else { 0 }])
        .await
        .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
    let mut reply = [0u8; 2];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
    diagnostics::protocol_reply("socks_method", Some(reply[1].into()));
    if reply != [5, if auth.is_some() { 2 } else { 0 }] {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    if let Some((username, password)) = auth {
        if username.len() > 255 || password.len() > 255 {
            return Err("PROXY_INVALID_URL".into());
        }
        let mut request = vec![1, username.len() as u8];
        request.extend_from_slice(username.as_bytes());
        request.push(password.len() as u8);
        request.extend_from_slice(password.as_bytes());
        stream
            .write_all(&request)
            .await
            .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
        stream
            .read_exact(&mut reply)
            .await
            .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
        diagnostics::protocol_reply("socks_auth", Some(reply[1].into()));
        if reply != [1, 0] {
            return Err("PROXY_CONNECT_FAILED".into());
        }
    }
    let mut request = vec![5, 1, 0];
    if let Ok(ip) = target.host.parse::<IpAddr>() {
        match ip {
            IpAddr::V4(ip) => {
                request.push(1);
                request.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                request.push(4);
                request.extend_from_slice(&ip.octets());
            }
        }
    } else {
        if target.host.len() > 255 {
            return Err("PROXY_CONNECT_FAILED".into());
        }
        request.extend_from_slice(&[3, target.host.len() as u8]);
        request.extend_from_slice(target.host.as_bytes());
    }
    request.extend_from_slice(&target.port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
    diagnostics::protocol_reply("socks_connect", Some(head[1].into()));
    if head[0] != 5 || head[1] != 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let skip = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut length = [0];
            stream
                .read_exact(&mut length)
                .await
                .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
            length[0] as usize
        }
        _ => return Err("PROXY_CONNECT_FAILED".into()),
    };
    let mut address_and_port = vec![0u8; skip + 2];
    stream
        .read_exact(&mut address_and_port)
        .await
        .map_err(|e| diagnostics::io_failed("upstream_socks_io", &e))?;
    Ok(stream)
}

#[cfg(test)]
#[path = "codex_proxy_desktop_router_tests.rs"]
mod tests;
