//! A stable, per-account SOCKS endpoint for managed Codex desktop processes.
//! Each new connection resolves the current account/unified binding. Existing
//! connections hold their engine lease until they finish, so changing a proxy
//! never leaves a running desktop process pointing at a retired random port.
//!
//! 入口固定为 SOCKS5（仅 TCP）。每条新连接都会重新解析当前绑定：有绑定就走 Mihomo 出口，
//! 未绑定（例如运行中解绑）则直接连客户端给出的目标主机名，出口 IP 与客户端自己直连一致；
//! UDP/QUIC 由客户端按 SOCKS 代理语义处理（Chromium 不会把 UDP 交给 SOCKS 入口），因此
//! 这种链路不能描述成与“完全直连”等价。
//!
//! 启动时只为**已有生效出口**（独立绑定或统一代理）的账号安装入口：未绑定账号若也被注入
//! 本地入口，会替代应用全局代理环境变量、用户自定义的 `--proxy-server`/PAC 参数与系统代理，
//! 属于静默改变出口。因此未绑定账号保持原有启动参数，首次绑定后需要重启一次，之后换节点、
//! 换策略或解绑都不再需要重启。
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
static TLS_CONFIG: OnceCell<Arc<ClientConfig>> = OnceCell::const_new();

#[path = "codex_proxy_desktop_entry.rs"]
mod entry;
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
/// 调用方（`ensure_route`）持 `ROUTES` 锁串行执行，因此同一账号不会注册出两个入口；
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
    format!("socks5://127.0.0.1:{port}")
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
    match ensure_for_account(account_id, &account, proxy_active, path.as_deref()).await {
        Ok(url) => Ok(url),
        Err(error) => {
            // 入口创建失败不能阻断客户端启动，也不能让客户端误以为已经走代理：
            // 回退到不注入代理参数的原有启动行为，并留下可排查的日志。
            logger::log_warn(&format!(
                "[CodexProxy] 桌面入口创建失败，本次启动不注入代理参数: account={account_id}, error={error}"
            ));
            Ok(None)
        }
    }
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
    if !codex_account_proxy::has_effective_proxy(account, unified_active) {
        return Ok(None);
    }
    Ok(Some(ensure_route(account_id, path).await?))
}

async fn ensure_route(account_id: &str, path: Option<&Path>) -> Result<String, String> {
    let mut routes = ROUTES.lock().await;
    if let Some(port) = routes.get(account_id) {
        if entry_status(account_id)
            .is_some_and(|status| status.state == "listening" && status.port == Some(*port))
        {
            return Ok(entry_url(*port));
        }
    }
    let (listener, port) = match install(account_id, path).await {
        Ok(installed) => installed,
        Err(error) => {
            entry::failed_to_listen(account_id);
            return Err(error);
        }
    };
    let observation = entry::listening(account_id, port);
    let id = account_id.to_owned();
    tokio::spawn(async move { serve(listener, id, observation).await });
    routes.insert(account_id.to_owned(), port);
    Ok(entry_url(port))
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
    let destination = tokio::time::timeout(CONNECT_TIMEOUT, read_socks_request(&mut client))
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")??;
    // A TCP accept or SOCKS greeting alone is not a proxy connection request.
    let request = observed_entry.request();
    let upstream = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let target = resolve_desktop_target(account_id).await?;
        dial(&destination, target).await
    })
    .await
    .unwrap_or_else(|_| Err("PROXY_ENGINE_TIMEOUT".into()));
    let (mut upstream, _lease) = match upstream {
        Ok(value) => value,
        Err(error) => {
            request.failed(&error);
            let _ = client.write_all(&[5, 1, 0, 1, 0, 0, 0, 0, 0, 0]).await;
            return Err(error);
        }
    };
    client
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    request.forwarded();
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
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
async fn dial(
    destination: &Destination,
    target: Option<(String, Option<Arc<DesktopTunnel>>)>,
) -> Result<(BoxIo, Option<Arc<DesktopTunnel>>), String> {
    match target {
        Some((url, lease)) => Ok((connect_via_proxy(&url, destination).await?, lease)),
        None => Ok((connect_direct(destination).await?, None)),
    }
}

/// 未绑定账号的出口沿用客户端原本的直连结果：直接连目标主机名，不改写地址。
async fn connect_direct(destination: &Destination) -> Result<BoxIo, String> {
    let stream = TcpStream::connect((destination.host.as_str(), destination.port))
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
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
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    if greeting[0] != 5 || greeting[1] == 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let mut methods = vec![0u8; greeting[1] as usize];
    client
        .read_exact(&mut methods)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    if !methods.contains(&0) {
        let _ = client.write_all(&[5, 255]).await;
        return Err("PROXY_CONNECT_FAILED".into());
    }
    client
        .write_all(&[5, 0])
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    let mut head = [0u8; 4];
    client
        .read_exact(&mut head)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    if head[0] != 5 || head[1] != 1 || head[2] != 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let host = match head[3] {
        1 => {
            let mut octets = [0u8; 4];
            client
                .read_exact(&mut octets)
                .await
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            IpAddr::V4(Ipv4Addr::from(octets)).to_string()
        }
        4 => {
            let mut octets = [0u8; 16];
            client
                .read_exact(&mut octets)
                .await
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            IpAddr::V6(Ipv6Addr::from(octets)).to_string()
        }
        3 => {
            let mut length = [0u8; 1];
            client
                .read_exact(&mut length)
                .await
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            if length[0] == 0 {
                return Err("PROXY_CONNECT_FAILED".into());
            }
            let mut name = vec![0u8; length[0] as usize];
            client
                .read_exact(&mut name)
                .await
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            String::from_utf8(name).map_err(|_| "PROXY_CONNECT_FAILED")?
        }
        _ => return Err("PROXY_CONNECT_FAILED".into()),
    };
    let mut port = [0u8; 2];
    client
        .read_exact(&mut port)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    let port = u16::from_be_bytes(port);
    if port == 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    Ok(Destination { host, port })
}

async fn connect_via_proxy(proxy_url: &str, target: &Destination) -> Result<BoxIo, String> {
    let url = url::Url::parse(proxy_url).map_err(|_| "PROXY_INVALID_URL")?;
    let host = url
        .host_str()
        .ok_or("PROXY_INVALID_URL")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url.port_or_known_default().ok_or("PROXY_INVALID_URL")?;
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    match url.scheme() {
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
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            http_connect(Box::new(tls), &url, target).await
        }
        "socks5" | "socks5h" => socks_connect(Box::new(tcp), &url, target).await,
        _ => Err("PROXY_UNSUPPORTED_PROTOCOL".into()),
    }
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
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    let mut response = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    while response.len() < 16 * 1024 {
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|_| "PROXY_CONNECT_FAILED")?;
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
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    let mut reply = [0u8; 2];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
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
            .map_err(|_| "PROXY_CONNECT_FAILED")?;
        stream
            .read_exact(&mut reply)
            .await
            .map_err(|_| "PROXY_CONNECT_FAILED")?;
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
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
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
                .map_err(|_| "PROXY_CONNECT_FAILED")?;
            length[0] as usize
        }
        _ => return Err("PROXY_CONNECT_FAILED".into()),
    };
    let mut address_and_port = vec![0u8; skip + 2];
    stream
        .read_exact(&mut address_and_port)
        .await
        .map_err(|_| "PROXY_CONNECT_FAILED")?;
    Ok(stream)
}

#[cfg(test)]
#[path = "codex_proxy_desktop_router_tests.rs"]
mod tests;
