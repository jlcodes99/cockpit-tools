//! Mihomo is a separate executable, never a library linked into the host.
//! Configuration (including secrets) travels over stdin, not argv or log files.
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, LazyLock, Mutex, Weak,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};

#[path = "codex_proxy_engine_delay.rs"]
mod delay;
pub(crate) use delay::validate_delay_url;

pub const ENGINE_VERSION: &str = "1.19.31";
const START_TIMEOUT: Duration = Duration::from_secs(8);
static CHILDREN: LazyLock<Mutex<HashMap<uuid::Uuid, Weak<Mutex<Child>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Isolate Mihomo's non-configuration working files from other clients and
/// accounts. Proxy secrets are sent only through stdin.
struct RuntimeDirectory(PathBuf);
impl RuntimeDirectory {
    async fn create() -> Result<Self, String> {
        tokio::time::timeout(
            Duration::from_secs(3),
            tokio::task::spawn_blocking(|| {
                let path =
                    std::env::temp_dir().join(format!("cockpit-mihomo-{}", uuid::Uuid::new_v4()));
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(&path)
                    .map_err(|_| "PROXY_ENGINE_START_FAILED".to_string())?;
                Ok::<_, String>(Self(path))
            }),
        )
        .await
        .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?
    }
}
impl Drop for RuntimeDirectory {
    fn drop(&mut self) {
        let path = self.0.clone();
        // Windows may briefly retain cache handles while a killed child exits.
        // Cleanup never waits on the UI or async runtime thread.
        let _ = std::thread::Builder::new()
            .name("mihomo-cleanup".into())
            .spawn(move || {
                for _ in 0..20 {
                    match std::fs::remove_dir_all(&path) {
                        Ok(()) => return,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                        Err(_) => std::thread::sleep(Duration::from_millis(100)),
                    }
                }
                super::logger::log_warn("[ProxyEngine] runtime directory cleanup deferred");
            });
    }
}

/// Dropping a lease also kills the child, including cancellation during a probe.
/// Long-lived account tunnels will keep a lease in the per-account runtime store.
pub struct NodeTunnel {
    error: Arc<AtomicU8>,
    error_readers: Vec<tokio::task::JoinHandle<()>>,
    child: Arc<Mutex<Child>>,
    id: uuid::Uuid,
    proxy_url: String,
    controller: EngineController,
    selection: Option<SelectionReader>,
    // Shared release lease prevents cleanup while the tunnel uses this engine.
    _engine_lease: Option<std::fs::File>,
    _runtime_dir: RuntimeDirectory,
}

#[derive(Clone)]
pub struct EngineController {
    pub(crate) endpoint: String,
    pub(crate) secret: String,
    pub(crate) tunnel_id: uuid::Uuid,
}

/// Clone before releasing the runtime lock; all HTTP work happens without that lock.
#[derive(Clone)]
pub struct SelectionReader {
    endpoint: String,
    secret: String,
    names: BTreeMap<String, String>,
    groups: BTreeMap<String, Vec<String>>,
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxySelection {
    pub name: String,
    pub delay_ms: Option<u64>,
    pub checked_at: Option<i64>,
}

fn selection_history(name: String, value: &Value) -> ProxySelection {
    let history = value["history"].as_array().and_then(|entries| entries.last());
    let checked_at = history.and_then(|entry| entry["time"].as_str())
        .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
        .map(|time| time.timestamp_millis());
    let delay_ms = history.and_then(|entry| entry["delay"].as_u64())
        .filter(|delay| *delay > 0 && checked_at.is_some() && value["alive"] != false);
    ProxySelection { name, delay_ms, checked_at }
}

impl SelectionReader {
    pub async fn selected_node(&self) -> Result<Option<String>, String> {
        Ok(self.selected_tag().await?.and_then(|(tag, _)| self.names.get(&tag).cloned()))
    }

    /// Read the running engine's last health check for the selected leaf. Never
    /// substitute a group delay or an unrelated UI probe for this node's history.
    pub async fn selected_info(&self) -> Result<Option<ProxySelection>, String> {
        tokio::time::timeout(Duration::from_secs(3), async {
            let Some((tag, test_url)) = self.selected_tag().await? else { return Ok(None); };
            let Some(name) = self.names.get(&tag).cloned() else { return Ok(None); };
            let history = async {
                let client = reqwest::Client::builder().no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_millis(900)).build().map_err(|_| ())?;
                let mut response = client.get(format!("{}/proxies/{}", self.endpoint, urlencoding::encode(&tag)))
                    .bearer_auth(&self.secret).send().await.map_err(|_| ())?;
                if !response.status().is_success() { return Err(()); }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
                    if bytes.len() + chunk.len() > 64 * 1024 { return Err(()); }
                    bytes.extend_from_slice(&chunk);
                }
                serde_json::from_slice::<Value>(&bytes).map_err(|_| ())
            }.await.unwrap_or(Value::Null);
            let history = test_url.as_deref().and_then(|url| history["extra"].get(url)).unwrap_or(&history);
            Ok(Some(selection_history(name, history)))
        }).await.map_err(|_| "PROXY_STATUS_FAILED".to_string())?
    }

    async fn selected_tag(&self) -> Result<Option<(String, Option<String>)>, String> {
        // Total deadline includes nested selectors. Never follow a controller redirect
        // or inherit user/system proxy settings for the authenticated localhost query.
        tokio::time::timeout(Duration::from_secs(2), async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .map_err(|_| "PROXY_STATUS_FAILED")?;
            let mut tag = "account-node".to_owned();
            let mut test_url = None;
            for _ in 0..=16 {
                let Some(members) = self.groups.get(&tag) else {
                    return Ok(self.names.contains_key(&tag).then_some((tag, test_url)));
                };
                let mut response = client
                    .get(format!(
                        "{}/proxies/{}",
                        self.endpoint,
                        urlencoding::encode(&tag)
                    ))
                    .bearer_auth(&self.secret)
                    .send()
                    .await
                    .map_err(|_| "PROXY_STATUS_FAILED")?;
                if !response.status().is_success() {
                    return Err("PROXY_STATUS_FAILED".into());
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| "PROXY_STATUS_FAILED")? {
                    if bytes.len() + chunk.len() > 64 * 1024 {
                        return Err("PROXY_STATUS_FAILED".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let value: Value =
                    serde_json::from_slice(&bytes).map_err(|_| "PROXY_STATUS_FAILED")?;
                // A load-balancer selects per connection; it has no single
                // current exit. Do not present its group name as a node.
                if value["type"] == "LoadBalance" {
                    return Ok(None);
                }
                if let Some(url) = value["testUrl"].as_str().filter(|url| !url.is_empty()) {
                    test_url = Some(url.to_owned());
                }
                let now = value["now"].as_str().ok_or("PROXY_STATUS_FAILED")?;
                if now.is_empty() {
                    return Ok(None);
                }
                if !members.iter().any(|member| member == now) {
                    return Err("PROXY_STATUS_FAILED".into());
                }
                tag = now.to_owned();
            }
            Err("PROXY_STATUS_FAILED".into())
        })
        .await
        .map_err(|_| "PROXY_STATUS_FAILED".to_string())?
    }
}

impl NodeTunnel {
    pub fn failure(&self, fallback: String) -> String {
        super::codex_proxy_engine_errors::code(self.error.load(Ordering::Relaxed))
            .unwrap_or(&fallback)
            .to_owned()
    }

    pub fn is_running(&self) -> bool {
        self.child
            .lock()
            .is_ok_and(|mut child| matches!(child.try_wait(), Ok(None)))
    }
    pub fn proxy_url(&self) -> &str {
        &self.proxy_url
    }

    pub fn selection_reader(&self) -> Option<SelectionReader> {
        self.selection.clone()
    }
    pub fn controller(&self) -> EngineController {
        self.controller.clone()
    }
    pub async fn selected_node(&self) -> Result<Option<String>, String> {
        match &self.selection {
            Some(reader) => reader.selected_node().await,
            None => Ok(None),
        }
    }

    pub async fn stop(self) -> Result<(), String> {
        self.child
            .lock()
            .map_err(|_| "PROXY_ENGINE_STOP_FAILED")?
            .start_kill()
            .map_err(|_| "PROXY_ENGINE_STOP_FAILED")?;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let exited = self
                    .child
                    .lock()
                    .map_err(|_| "PROXY_ENGINE_STOP_FAILED")?
                    .try_wait()
                    .map_err(|_| "PROXY_ENGINE_STOP_FAILED")?
                    .is_some();
                if exited {
                    return Ok::<(), String>(());
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| "PROXY_ENGINE_STOP_FAILED")?
    }
}

impl Drop for NodeTunnel {
    fn drop(&mut self) {
        for reader in self.error_readers.drain(..) {
            reader.abort();
        }
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
        if let Ok(mut children) = CHILDREN.lock() {
            children.remove(&self.id);
        }
    }
}

/// Called synchronously at host exit: signal only, never wait or spawn a shell.
/// Covers children still starting, not merely committed account runtimes.
pub fn shutdown_all() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let children = CHILDREN
        .lock()
        .map(|children| {
            children
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for child in children {
        if let Ok(mut child) = child.lock() {
            let _ = child.start_kill();
        }
    }
}

/// Explicitly installed engines take precedence; lookup never downloads or searches PATH.
pub async fn engine_ready() -> Result<bool, String> {
    match engine_path_async().await {
        Ok(_) => Ok(true),
        Err(error) if error == "PROXY_ENGINE_MISSING" => Ok(false),
        Err(error) => Err(error),
    }
}

/// Filesystem lookup must never run on an async runtime thread or under a runtime lock.
pub(crate) async fn engine_path_async() -> Result<PathBuf, String> {
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(engine_path),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
    .map_err(|_| "PROXY_ENGINE_START_FAILED")?
}

fn engine_path() -> Result<PathBuf, String> {
    if let Some(path) = super::codex_proxy_engine_install::managed_path()? {
        return Ok(path);
    }
    let name = if cfg!(windows) {
        "mihomo.exe"
    } else {
        "mihomo"
    };
    // Do not search PATH or download executables implicitly at runtime.
    let current = std::env::current_exe().map_err(|_| "PROXY_ENGINE_MISSING")?;
    for candidate in engine_candidates(&current, name) {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    #[cfg(debug_assertions)]
    {
        let local = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sidecars/mihomo/bin")
            .join(name);
        if local.is_file() {
            return Ok(local);
        }
    }
    Err("PROXY_ENGINE_MISSING".into())
}

fn engine_candidates(current_exe: &Path, name: &str) -> Vec<PathBuf> {
    let Some(parent) = current_exe.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![
        parent.join(name),
        parent.join("proxy-engine").join(name),
        parent.join("resources").join("proxy-engine").join(name),
    ];
    // macOS app bundle: Contents/MacOS/app -> Contents/Resources/proxy-engine
    if let Some(contents) = parent.parent() {
        candidates.push(contents.join("Resources").join("proxy-engine").join(name));
        candidates.push(contents.join("resources").join("proxy-engine").join(name));
        // Tauri Linux resources live under ../lib/<executable name> for
        // AppImage, deb/rpm, and local builds, not beside the executable.
        #[cfg(target_os = "linux")]
        if let Some(exe_name) = current_exe.file_name() {
            candidates.push(
                contents
                    .join("lib")
                    .join(exe_name)
                    .join("proxy-engine")
                    .join(name),
            );
        }
    }
    candidates
}

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    // Avoid inherited global proxy/credential settings affecting node connections.
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        if inherited_engine_override(&key) {
            command.env_remove(key);
        }
    }
    command
}

fn inherited_engine_override(key: &std::ffi::OsStr) -> bool {
    let key = key.to_string_lossy().to_ascii_uppercase();
    key.starts_with("CLASH_")
        || key.starts_with("MIHOMO_")
        || matches!(key.as_str(), "SKIP_SAFE_PATH_CHECK" | "SAFE_PATHS")
}

pub(crate) async fn verify_version(binary: &Path) -> Result<(), String> {
    verify_version_with_timeout(binary, Duration::from_secs(3)).await
}

/// Installation is a cold launch: OS security assessment can precede any output.
/// Runtime callers retain their short deadline; installers supply a separate budget.
pub(crate) async fn verify_version_with_timeout(
    binary: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let output = tokio::time::timeout(
        timeout,
        command(binary)
            .arg("-v")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| {
        super::logger::log_warn(&format!(
            "[ProxyEngine] version check timed out (budget_ms={})",
            timeout.as_millis()
        ));
        "PROXY_ENGINE_TIMEOUT"
    })?
    .map_err(|error| {
        // Never log a binary path, arbitrary process output or inherited credentials.
        super::logger::log_warn(&format!(
            "[ProxyEngine] version check spawn failed (kind={:?}, os_code={:?})",
            error.kind(),
            error.raw_os_error()
        ));
        "PROXY_ENGINE_START_FAILED"
    })?;
    if !output.status.success() {
        super::logger::log_warn(&format!(
            "[ProxyEngine] version check exited unsuccessfully (status={})",
            output.status
        ));
        return Err("PROXY_ENGINE_START_FAILED".into());
    }
    if !version_matches(&output.stdout) {
        super::logger::log_warn("[ProxyEngine] version check returned an unexpected version");
        return Err("PROXY_ENGINE_VERSION".into());
    }
    Ok(())
}

fn version_matches(output: &[u8]) -> bool {
    let text = String::from_utf8_lossy(output);
    let mut words = text.split_whitespace();
    words.next() == Some("Mihomo")
        && words.next() == Some("Meta")
        && words
            .next()
            .is_some_and(|v| v.strip_prefix('v').unwrap_or(v) == ENGINE_VERSION)
}

fn configuration(
    outbound: Value,
    port: u16,
    username: &str,
    password: &str,
) -> Result<Value, String> {
    let (proxies, groups) = super::codex_proxy_catalog_binding::runtime_parts(outbound)?;
    Ok(json!({
        "mixed-port": port, "bind-address": "127.0.0.1", "allow-lan": false,
        "authentication": [format!("{username}:{password}")], "skip-auth-prefixes": [],
        "mode": "rule", "log-level": "warning", "ipv6": true,
        "find-process-mode": "off", "geo-auto-update": false,
        "profile": {"store-selected": false, "store-fake-ip": false},
        "tun": {"enable": false}, "sniffer": {"enable": false},
        "dns": {"enable": false},
        "proxies": proxies, "proxy-groups": groups,
        "rules": ["MATCH,account-node"]
    }))
}

async fn start_resource(
    input: &str,
    desktop: bool,
    unified_delay: bool,
) -> Result<NodeTunnel, String> {
    let outbounds = super::codex_proxy_catalog_binding::outbounds(input)?;
    let names = super::codex_proxy_catalog_binding::names(input)?;
    let binary = engine_path_async().await?;
    start_with_binary_options(
        &binary,
        json!(outbounds),
        desktop,
        Some(names),
        super::codex_proxy_catalog_binding::decode(input)?.network,
        unified_delay,
    )
    .await
}

pub async fn start(input: &str) -> Result<NodeTunnel, String> {
    if input.starts_with(super::codex_proxy_catalog_binding::PREFIX) {
        return start_resource(input, false, false).await;
    }
    let outbound = crate::modules::codex_proxy_node_parser::parse_node_link(input)?;
    let binary = engine_path_async().await?;
    start_with_binary(&binary, outbound).await
}

/// A latency check gets its own authenticated controller and process. Unified
/// delay affects only this isolated probe, never an account's running tunnel.
pub async fn start_latency(input: &str) -> Result<NodeTunnel, String> {
    if input.starts_with(super::codex_proxy_catalog_binding::PREFIX) {
        return start_resource(input, false, true).await;
    }
    let outbound = crate::modules::codex_proxy_node_parser::parse_node_link(input)?;
    let binary = engine_path_async().await?;
    start_with_binary_options(&binary, outbound, false, None, Default::default(), true).await
}

/// Desktop Chromium cannot authenticate a SOCKS proxy from command-line flags.
/// This explicitly requested listener is loopback-only, but not a local-user ACL.
pub async fn start_desktop(input: &str) -> Result<NodeTunnel, String> {
    if input.starts_with(super::codex_proxy_catalog_binding::PREFIX) {
        return start_resource(input, true, false).await;
    }
    let outbound = desktop_outbound(input)?;
    let binary = engine_path_async().await?;
    start_with_binary_mode(&binary, outbound, true).await
}

/// A sidecar receives an authenticated loopback proxy URL, never the upstream
/// proxy credentials. Unlike Chromium, it can use the random local SOCKS auth.
pub async fn start_sidecar(input: &str) -> Result<NodeTunnel, String> {
    if input.starts_with(super::codex_proxy_catalog_binding::PREFIX) {
        return start_resource(input, false, false).await;
    }
    let outbound = if direct_scheme(input) {
        desktop_outbound(input)?
    } else {
        crate::modules::codex_proxy_node_parser::parse_node_link(input)?
    };
    let binary = engine_path_async().await?;
    start_with_binary(&binary, outbound).await
}

fn direct_scheme(input: &str) -> bool {
    url::Url::parse(input)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h"))
}

fn desktop_outbound(input: &str) -> Result<Value, String> {
    let url = url::Url::parse(input).map_err(|_| "PROXY_INVALID_URL")?;
    if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
        return crate::modules::codex_proxy_node_parser::parse_node_link(input);
    }
    crate::modules::codex_account_proxy::normalize_direct_proxy(input)?;
    let decode = |value: &str| {
        urlencoding::decode(value)
            .map(|s| s.into_owned())
            .map_err(|_| "PROXY_INVALID_URL".to_string())
    };
    let host = url
        .host_str()
        .ok_or("PROXY_INVALID_URL")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let mut outbound = json!({
        "type": if url.scheme().starts_with("socks") { "socks" } else { "http" },
        "server": host, "server_port": url.port_or_known_default().ok_or("PROXY_INVALID_URL")?,
        "connect_timeout": "10s"
    });
    if url.scheme().starts_with("socks") {
        outbound["version"] = json!("5");
    }
    if url.scheme() == "https" {
        outbound["tls"] = json!({"enabled":true,"server_name":host});
    }
    if !url.username().is_empty() || url.password().is_some() {
        outbound["username"] = json!(decode(url.username())?);
        outbound["password"] = json!(decode(url.password().unwrap_or(""))?);
    }
    Ok(outbound)
}

async fn start_with_binary(binary: &Path, outbound: Value) -> Result<NodeTunnel, String> {
    start_with_binary_mode(binary, outbound, false).await
}

async fn start_with_binary_mode(
    binary: &Path,
    outbound: Value,
    desktop: bool,
) -> Result<NodeTunnel, String> {
    start_with_binary_named(binary, outbound, desktop, None).await
}

async fn start_with_binary_named(
    binary: &Path,
    outbound: Value,
    desktop: bool,
    names: Option<BTreeMap<String, String>>,
) -> Result<NodeTunnel, String> {
    start_with_binary_options(binary, outbound, desktop, names, Default::default(), false).await
}
async fn start_with_binary_options(
    binary: &Path,
    outbound: Value,
    desktop: bool,
    names: Option<BTreeMap<String, String>>,
    network: super::codex_proxy_network::NetworkOptions,
    unified_delay: bool,
) -> Result<NodeTunnel, String> {
    network.validate()?;
    if SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err("PROXY_ENGINE_STOPPED".into());
    }
    let lease_binary = binary.to_owned();
    let engine_lease = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            super::codex_proxy_engine_install::lease_managed(&lease_binary)
        }),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
    .map_err(|_| "PROXY_ENGINE_START_FAILED")?
    .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    super::codex_proxy_engine_install::verify_managed(binary).await?;
    verify_version(binary).await?;
    tokio::time::timeout(
        START_TIMEOUT,
        start_inner(
            binary,
            outbound,
            desktop,
            engine_lease,
            names,
            network,
            unified_delay,
        ),
    )
    .await
    .map_err(|_| "PROXY_ENGINE_TIMEOUT".to_string())?
}

async fn start_inner(
    binary: &Path,
    outbound: Value,
    desktop: bool,
    engine_lease: Option<std::fs::File>,
    names: Option<BTreeMap<String, String>>,
    network: super::codex_proxy_network::NetworkOptions,
    unified_delay: bool,
) -> Result<NodeTunnel, String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    let port = listener
        .local_addr()
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?
        .port();
    let username = uuid::Uuid::new_v4().simple().to_string();
    let password = uuid::Uuid::new_v4().simple().to_string();
    let mut config = configuration(outbound, port, &username, &password)?;
    network.apply(&mut config)?;
    if unified_delay {
        config["unified-delay"] = json!(true);
    }
    if desktop {
        config["authentication"] = json!([]);
    }
    let controller_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    let address = controller_listener
        .local_addr()
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    let secret = uuid::Uuid::new_v4().simple().to_string();
    config["external-controller"] = json!(address.to_string());
    config["secret"] = json!(secret);
    config["external-controller-cors"] = json!({
        "allow-origins": ["http://cockpit.invalid"], "allow-private-network": false
    });
    let selection = if let Some(names) = names {
        let groups = config["proxy-groups"]
            .as_array()
            .ok_or("PROXY_ENGINE_START_FAILED")?
            .iter()
            .filter_map(|v| {
                Some((
                    v["name"].as_str()?.to_owned(),
                    v["proxies"]
                        .as_array()?
                        .iter()
                        .filter_map(|m| m.as_str().map(str::to_owned))
                        .collect(),
                ))
            })
            .collect();
        Some(SelectionReader {
            endpoint: format!("http://{address}"),
            secret: secret.clone(),
            names,
            groups,
        })
    } else {
        None
    };
    let config = serde_json::to_vec(&config).map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    drop(listener);
    drop(controller_listener);
    let runtime_dir = RuntimeDirectory::create().await?;
    let mut child = command(binary)
        .args(["-f", "-"])
        .arg("-d")
        .arg(&runtime_dir.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    let mut stdin = child.stdin.take().ok_or("PROXY_ENGINE_START_FAILED")?;
    let error = Arc::new(AtomicU8::new(0));
    let mut error_readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        error_readers.push(super::codex_proxy_engine_errors::read(
            stdout,
            error.clone(),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        error_readers.push(super::codex_proxy_engine_errors::read(
            stderr,
            error.clone(),
        ));
    }
    let id = uuid::Uuid::new_v4();
    let tunnel = NodeTunnel {
        error,
        error_readers,
        child: Arc::new(Mutex::new(child)),
        id,
        _engine_lease: engine_lease,
        _runtime_dir: runtime_dir,
        controller: EngineController {
            endpoint: format!("http://{address}"),
            secret,
            tunnel_id: id,
        },
        selection,
        proxy_url: if desktop {
            format!("http://127.0.0.1:{port}")
        } else {
            format!("socks5h://{username}:{password}@127.0.0.1:{port}")
        },
    };
    {
        let mut children = CHILDREN.lock().map_err(|_| "PROXY_ENGINE_START_FAILED")?;
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            return Err("PROXY_ENGINE_STOPPED".into());
        }
        children.insert(tunnel.id, Arc::downgrade(&tunnel.child));
    }
    stdin
        .write_all(&config)
        .await
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    stdin
        .shutdown()
        .await
        .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
    drop(stdin);
    loop {
        if !tunnel.is_running() {
            return Err("PROXY_ENGINE_START_FAILED".into());
        }
        if let Ok(Ok(mut socket)) = tokio::time::timeout(
            Duration::from_millis(200),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            let authenticated = tokio::time::timeout(Duration::from_millis(500), async {
                socket
                    .write_all(&[5, 1, if desktop { 0 } else { 2 }])
                    .await?;
                let mut response = [0; 2];
                socket.read_exact(&mut response).await?;
                if desktop {
                    return Ok::<bool, std::io::Error>(response == [5, 0]);
                }
                if response != [5, 2] {
                    return Ok::<bool, std::io::Error>(false);
                }
                let mut auth = vec![1, username.len() as u8];
                auth.extend_from_slice(username.as_bytes());
                auth.push(password.len() as u8);
                auth.extend_from_slice(password.as_bytes());
                socket.write_all(&auth).await?;
                socket.read_exact(&mut response).await?;
                Ok(response == [1, 0])
            })
            .await;
            if matches!(authenticated, Ok(Ok(true))) && tunnel.is_running() {
                return Ok(tunnel);
            }
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_latency_keeps_the_latest_leaf_check_and_never_invents_zero() {
        let history = json!({"alive":true,"history":[
            {"time":"2026-09-25T10:00:00Z","delay":14},
            {"time":"2026-09-25T10:01:00Z","delay":216}
        ]});
        let value = selection_history("US".into(), &history);
        assert_eq!(value.delay_ms, Some(216));
        assert_eq!(value.checked_at, Some(chrono::DateTime::parse_from_rfc3339("2026-09-25T10:01:00Z").unwrap().timestamp_millis()));
        let failed = json!({"alive":false,"history":[{"time":"2026-09-25T10:01:00Z","delay":216}]});
        assert_eq!(selection_history("US".into(), &failed).delay_ms, None);
        for value in [Value::Null, json!({"history":[]}), json!({"history":[{"delay":216}]}), json!({"history":[{"time":"invalid","delay":216}]}), json!({"history":[{"time":"2026-09-25T10:01:00Z","delay":0}]})] {
            assert_eq!(selection_history("US".into(), &value).delay_ms, None);
        }
    }

    #[tokio::test]
    async fn selection_info_reads_the_selected_leaf_history_without_exposing_credentials() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, body) in [
                ("account-node", json!({"type":"URLTest","now":"leaf","testUrl":"https://check.invalid"})),
                ("leaf", json!({"alive":true,"history":[{"time":"2026-09-25T10:01:00Z","delay":999}],"extra":{"https://check.invalid":{"alive":true,"history":[{"time":"2026-09-25T10:01:00Z","delay":216}]}},"password":"MUST_NOT_LEAK"})),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                while !bytes.ends_with(b"\r\n\r\n") { bytes.push(socket.read_u8().await.unwrap()); assert!(bytes.len()<8192); }
                let request = String::from_utf8(bytes).unwrap().to_lowercase();
                assert!(request.starts_with(&format!("get /proxies/{path} ")));
                assert!(request.contains("authorization: bearer test-secret"));
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        let reader = SelectionReader { endpoint, secret:"test-secret".into(),
            names:BTreeMap::from([("leaf".into(),"US".into())]),
            groups:BTreeMap::from([("account-node".into(),vec!["leaf".into()])]) };
        let info = reader.selected_info().await.unwrap().unwrap();
        assert_eq!(info.name,"US"); assert_eq!(info.delay_ms,Some(216));
        assert!(!serde_json::to_string(&info).unwrap().contains("MUST_NOT_LEAK"));
        server.await.unwrap();
    }

    #[test]
    fn version_requires_mihomo_identity_and_exact_release() {
        assert!(version_matches(
            b"Mihomo Meta v1.19.31 darwin arm64 with go1.26\n"
        ));
        assert!(!version_matches(b"sing-box version 1.19.31\n"));
        assert!(!version_matches(b"Mihomo Meta v1.19.310 darwin arm64\n"));
        assert!(!version_matches(b"Mihomo Meta v1.19.31-alpha\n"));
        assert!(!version_matches(b"1.19.31"));
    }

    #[test]
    fn inherited_engine_override_is_case_insensitive() {
        for key in [
            "CLASH_CONFIG_STRING",
            "clash_post_up",
            "Clash_Override_External_Controller",
            "mihomo_home",
        ] {
            assert!(inherited_engine_override(std::ffi::OsStr::new(key)));
        }
        assert!(!inherited_engine_override(std::ffi::OsStr::new("PATH")));
    }
    #[tokio::test]
    async fn selection_reader_resolves_nested_groups_with_auth_and_no_secret_output() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (path, next) in [("account-node", "auto"), ("auto", "leaf")] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    request.push(socket.read_u8().await.unwrap());
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with(&format!("get /proxies/{path} ")));
                assert!(request.contains("authorization: bearer local-secret"));
                let body = json!({"now":next,"password":"SHOULD_NOT_SURFACE"}).to_string();
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        let reader = SelectionReader {
            endpoint,
            secret: "local-secret".into(),
            names: BTreeMap::from([("leaf".into(), "Tokyo".into())]),
            groups: BTreeMap::from([
                ("account-node".into(), vec!["auto".into()]),
                ("auto".into(), vec!["leaf".into()]),
            ]),
        };
        assert_eq!(reader.selected_node().await.unwrap(), Some("Tokyo".into()));
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo binary"]
    async fn real_engine_resource_groups_expose_authenticated_selection() {
        let binary = test_engine();
        async fn proxy(delay: u64) -> (u16, tokio::task::JoinHandle<()>) {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    tokio::spawn(async move {
                        // Bounded mock HTTP proxy: CONNECT followed by URLTest request.
                        for pass in 0..2 {
                            let mut request = Vec::new();
                            loop {
                                let Ok(byte) = socket.read_u8().await else {
                                    return;
                                };
                                request.push(byte);
                                if request.ends_with(b"\r\n\r\n") {
                                    break;
                                }
                                if request.len() > 8192 {
                                    return;
                                }
                            }
                            if pass == 0 {
                                socket
                                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                                    .await
                                    .unwrap();
                            } else {
                                tokio::time::sleep(Duration::from_millis(delay)).await;
                                let _ = socket
                                    .write_all(
                                        b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n",
                                    )
                                    .await;
                            }
                        }
                    });
                }
            });
            (port, task)
        }
        let (slow_port, slow) = proxy(350).await;
        let (fast_port, fast) = proxy(0).await;
        let outbounds = json!([
            {"type":"selector","tag":"account-node","outbounds":["auto"],"default":"auto"},
            {"type":"urltest","tag":"auto","outbounds":["slow","leaf"],"url":"http://probe.invalid/test","interval":"180s","tolerance":0},
            {"type":"http","tag":"slow","server":"127.0.0.1","server_port":slow_port,"connect_timeout":"10s"},
            {"type":"http","tag":"leaf","server":"127.0.0.1","server_port":fast_port,"connect_timeout":"10s"}
        ]);
        let names = BTreeMap::from([
            ("account-node".into(), "Choose".into()),
            ("auto".into(), "Auto".into()),
            ("slow".into(), "Slow".into()),
            ("leaf".into(), "Tokyo".into()),
        ]);
        let tunnel = start_with_binary_named(&binary, outbounds, false, Some(names))
            .await
            .unwrap();
        let reader = tunnel.selection_reader().unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client
            .get(format!("{}/proxies/account-node", reader.endpoint))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if reader.selected_node().await.unwrap() == Some("Tokyo".into()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("native URLTest should choose the faster proxy, not the first member");
        tunnel.stop().await.unwrap();
        slow.abort();
        fast.abort();
    }

    #[test]
    fn direct_proxy_auth_is_in_outbound_config_only() {
        let outbound = desktop_outbound("https://user:p%40ss@proxy.example:8443").unwrap();
        assert_eq!(outbound["type"], "http");
        assert_eq!(outbound["password"], "p@ss");
        assert_eq!(outbound["tls"]["enabled"], true);
        assert_eq!(outbound["server"], "proxy.example");
        let socks = desktop_outbound("socks5h://user:password@[::1]:1080").unwrap();
        assert_eq!(socks["type"], "socks");
        assert_eq!(socks["server"], "::1");
        assert_eq!(socks["version"], "5");
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo binary"]
    async fn real_engine_desktop_bridge_authenticates_upstream_without_client_secrets() {
        let binary = test_engine();
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_address = upstream.local_addr().unwrap();
        async fn headers(socket: &mut tokio::net::TcpStream) -> String {
            let mut bytes = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                bytes.push(byte);
                assert!(bytes.len() <= 8192);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            String::from_utf8(bytes).unwrap()
        }
        let server = tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let connect = headers(&mut socket).await.to_lowercase();
            assert!(connect.starts_with("connect example.invalid:80 "));
            assert!(connect.contains("proxy-authorization: basic dxnlcjpwyxnzd29yza=="));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            let request = headers(&mut socket).await.to_lowercase();
            assert!(request.starts_with("get / "));
            assert!(!request.contains("proxy-authorization"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let outbound =
            desktop_outbound(&format!("http://user:password@{upstream_address}")).unwrap();
        let bridge = start_with_binary_mode(&binary, outbound, true)
            .await
            .unwrap();
        let url = url::Url::parse(bridge.proxy_url()).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert!(url.username().is_empty() && url.password().is_none());
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(bridge.proxy_url()).unwrap())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let response = client.get("http://example.invalid/").send().await.unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        server.await.unwrap();
        bridge.stop().await.unwrap();
    }

    fn test_engine() -> PathBuf {
        std::env::var_os("COCKPIT_TEST_MIHOMO")
            .map(PathBuf::from)
            .expect("Set COCKPIT_TEST_MIHOMO to a checksum-verified Mihomo 1.19.31 executable")
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo binary"]
    async fn real_engine_stdin_authentication_forwarding_and_stop() {
        let binary = test_engine();
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_address = origin.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = origin.accept().await.unwrap();
            let mut input = [0u8; 4096];
            let size = socket.read(&mut input).await.unwrap();
            let text = String::from_utf8_lossy(&input[..size]);
            assert!(text.starts_with("CONNECT "));
            socket
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
            let size = socket.read(&mut input).await.unwrap();
            assert!(String::from_utf8_lossy(&input[..size]).starts_with("GET / "));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        });
        let tunnel = start_with_binary(
            &binary,
            json!({"type":"http", "server":"127.0.0.1", "server_port":origin_address.port()}),
        )
        .await
        .unwrap();
        let local = url::Url::parse(tunnel.proxy_url()).unwrap();
        let port = local.port().unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let result = client.get("http://example.invalid/").send().await.unwrap();
        assert_eq!(result.text().await.unwrap(), "ok");
        server.await.unwrap();
        tunnel.stop().await.unwrap();
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo binary"]
    async fn real_engine_accepts_each_supported_node_configuration() {
        use base64::Engine as _;
        let binary = test_engine();
        verify_version(&binary).await.unwrap();
        let id = "11111111-2222-3333-4444-555555555555";
        let links = [
            format!("vless://{id}@example.com:443?security=tls&type=ws&path=%2Fnode"),
            format!("vless://{id}@example.com:443?security=tls&type=tcp&flow=xtls-rprx-vision"),
            "trojan://password@example.com:443".into(),
            "trojan://password@example.com:443?type=grpc&serviceName=Tunnel".into(),
            "ss://aes-256-gcm:password@example.com:8388".into(),
            "hy2://password@example.com:443".into(),
            format!("tuic://{id}:password@example.com:443?alpn=h3"),
            format!("vmess://{}", base64::engine::general_purpose::STANDARD.encode(
                json!({"v":"2","add":"example.com","port":"443","id":id,"net":"ws","path":"/proxy","tls":"tls"}).to_string())),
        ];
        for link in links {
            let outbound = crate::modules::codex_proxy_node_parser::parse_node_link(&link).unwrap();
            let config = configuration(outbound, 12345, "local-user", "local-password").unwrap();
            let mut child = command(&binary).args(["-t", "-f", "-"]).spawn().unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin
                .write_all(&serde_json::to_vec(&config).unwrap())
                .await
                .unwrap();
            drop(stdin);
            let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .unwrap()
                .unwrap();
            assert!(
                status.success(),
                "engine rejected {} configuration",
                config["proxies"][0]["type"]
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo binary"]
    async fn real_engine_accepts_native_clash_options_and_policies() {
        let binary = test_engine();
        verify_version(&binary).await.unwrap();
        let source = json!({"proxies":[
            {"name":"xhttp","type":"vless","server":"example.com","port":443,
             "uuid":"11111111-2222-3333-4444-555555555555","tls":true,"network":"xhttp",
             "xhttp-opts":{"path":"/tunnel","mode":"auto"}},
            {"name":"hy2","type":"hysteria2","server":"example.com","port":443,
             "password":"example-secret","up":20,"down":100,"ports":"20000-20010","hop-interval":30},
            {"name":"ss","type":"ss","server":"example.com","port":443,
             "cipher":"aes-128-gcm","password":"example-secret","plugin":"obfs",
             "plugin-opts":{"mode":"http","host":"example.com"}}
        ],"proxy-groups":[
            {"name":"failover","type":"fallback","proxies":["xhttp","hy2"],
             "url":"https://www.gstatic.com/generate_204","interval":180,"timeout":3000,"lazy":true},
            {"name":"balanced","type":"load-balance","proxies":["xhttp","ss"],
             "url":"https://www.gstatic.com/generate_204","interval":180,"strategy":"consistent-hashing","lazy":true}
        ]});
        let catalog =
            super::super::codex_proxy_subscription_parser::parse(&source.to_string()).unwrap();
        for item_id in catalog
            .nodes
            .iter()
            .map(|n| &n.id)
            .chain(catalog.groups.iter().map(|g| &g.id))
        {
            let binding = super::super::codex_proxy_catalog_binding::encode(
                "test",
                "test",
                item_id,
                &catalog,
                &BTreeMap::new(),
            )
            .unwrap();
            let outbounds = super::super::codex_proxy_catalog_binding::outbounds(&binding).unwrap();
            let config = configuration(json!(outbounds), 12345, "test-user", "test-pass").unwrap();
            let dir = RuntimeDirectory::create().await.unwrap();
            let mut child = command(&binary)
                .args(["-t", "-f", "-"])
                .arg("-d")
                .arg(&dir.0)
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin
                .write_all(&serde_json::to_vec(&config).unwrap())
                .await
                .unwrap();
            drop(stdin);
            assert!(tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success());
        }
    }

    #[test]
    fn config_is_loopback_authenticated_and_has_no_direct_fallback() {
        let value = configuration(
            json!({"type":"trojan","server":"node.example","server_port":443,"password":"secret","tls":{"enabled":true}}),
            23456,
            "local-user",
            "local-secret",
        )
        .unwrap();
        assert_eq!(value["bind-address"], "127.0.0.1");
        assert_eq!(value["authentication"][0], "local-user:local-secret");
        assert_eq!(value["skip-auth-prefixes"], json!([]));
        assert_eq!(value["allow-lan"], false);
        assert_eq!(value["tun"]["enable"], false);
        assert_eq!(value["proxies"].as_array().unwrap().len(), 1);
        assert_eq!(value["rules"], json!(["MATCH,account-node"]));
        assert_eq!(value["profile"]["store-selected"], false);
    }

    #[tokio::test]
    #[ignore = "requires explicitly provided, checksum-verified Mihomo; config check only, no listeners"]
    async fn real_engine_accepts_nested_auto_groups_and_blocking_members() {
        use super::super::{
            codex_proxy_catalog_binding as binding, codex_proxy_subscription_parser as parser,
        };
        let binary = test_engine();
        verify_version(&binary).await.unwrap();
        let directory = RuntimeDirectory::create().await.unwrap();
        for kind in ["url-test", "fallback", "load-balance"] {
            let input = format!("proxies:\n  - {{name: HY, type: hysteria2, server: 127.0.0.1, port: 443, password: synthetic, up: 50, down: 100, udp: true, ports: '20000-20002', mport: '20000-20002'}}\nproxy-groups:\n  - {{name: Auto, type: {kind}, proxies: [REJECT, HY, REJECT-DROP], url: http://127.0.0.1:9, interval: 300}}\n  - {{name: Region, type: select, proxies: [Auto, REJECT]}}\n");
            let catalog = parser::parse(&input).unwrap();
            let auto = &catalog.groups[0];
            let parent = &catalog.groups[1];
            for (item, selections) in [
                (
                    &parent.id,
                    BTreeMap::from([(parent.id.clone(), "Auto".into())]),
                ),
                (&auto.id, BTreeMap::new()),
                (
                    &parent.id,
                    BTreeMap::from([(parent.id.clone(), "REJECT".into())]),
                ),
            ] {
                let raw = binding::encode_with_group(
                    "s",
                    "s",
                    item,
                    &catalog,
                    &selections,
                    Some(&parent.id),
                )
                .unwrap();
                // Uses the exact account runtime configuration builder; -t only parses it.
                let config = configuration(
                    json!(binding::outbounds(&raw).unwrap()),
                    12345,
                    "test-user",
                    "test-password",
                )
                .unwrap();
                let mut child = command(&binary)
                    .arg("-d")
                    .arg(&directory.0)
                    .args(["-t", "-f", "-"])
                    .spawn()
                    .unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    let mut stdin = child.stdin.take().unwrap();
                    stdin
                        .write_all(&serde_json::to_vec(&config).unwrap())
                        .await
                        .unwrap();
                    drop(stdin);
                    assert!(
                        child.wait().await.unwrap().success(),
                        "rejected {kind} binding"
                    );
                })
                .await
                .expect("configuration check deadline");
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires private subscription file and verified engine; check-only, no node connections"]
    async fn real_engine_accepts_private_subscription_configurations() {
        let binary = test_engine();
        verify_version(&binary).await.unwrap();
        let path = std::env::var("COCKPIT_TEST_SUBSCRIPTION_FILE")
            .expect("private subscription file required");
        let input = std::fs::read_to_string(path).expect("cannot read private subscription file");
        let catalog = super::super::codex_proxy_subscription_parser::parse(&input)
            .expect("subscription parsing failed");
        let mut checked = 0;
        for (index, node) in catalog.nodes.iter().enumerate() {
            let Some(outbound) = &node.outbound else {
                continue;
            };
            let config =
                configuration(outbound.clone(), 12345, "test-user", "test-password").unwrap();
            let mut child = command(&binary).args(["-t", "-f", "-"]).spawn().unwrap();
            let mut stdin = child.stdin.take().unwrap();
            stdin
                .write_all(&serde_json::to_vec(&config).unwrap())
                .await
                .unwrap();
            drop(stdin);
            let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
                .await
                .unwrap()
                .unwrap();
            // Index only: provider-controlled display names can contain secrets.
            assert!(status.success(), "engine rejected node index {index}");
            checked += 1;
        }
        assert!(checked > 0);
        println!("engine accepted {checked} node configurations");
    }

    #[tokio::test]
    #[ignore = "explicit private subscription diagnosis: samples four nodes, no account changes"]
    async fn real_subscription_probe_diagnostic() {
        let binary = test_engine();
        let network = super::super::codex_proxy_network::NetworkOptions {
            doh: std::env::var("COCKPIT_TEST_DOH").as_deref() == Ok("1"),
            interface: String::new(),
        };
        verify_version(&binary).await.unwrap();
        let input =
            std::fs::read_to_string(std::env::var("COCKPIT_TEST_SUBSCRIPTION_FILE").unwrap())
                .unwrap();
        let raw: Value = serde_yaml::from_str(&input).unwrap();
        let catalog = super::super::codex_proxy_subscription_parser::parse(&input).unwrap();
        let nodes = raw["proxies"].as_array().unwrap();
        println!(
            "DIAG nodes={} groups={} skip_cert={} supported={}",
            nodes.len(),
            catalog.groups.len(),
            nodes
                .iter()
                .filter(|n| n["skip-cert-verify"] == true)
                .count(),
            catalog.nodes.iter().filter(|n| n.error.is_none()).count()
        );
        for (i, g) in catalog.groups.iter().enumerate() {
            let mut pending = vec![g];
            let mut seen = std::collections::HashSet::new();
            let mut leaves = std::collections::HashSet::new();
            while let Some(g) = pending.pop() {
                if !seen.insert(g.id.clone()) {
                    continue;
                }
                for name in &g.members {
                    if let Some(n) = catalog.nodes.iter().find(|n| &n.name == name) {
                        leaves.insert(n.id.clone());
                    } else if let Some(child) = catalog.groups.iter().find(|c| &c.name == name) {
                        pending.push(child);
                    }
                }
            }
            let region = if g.name.contains("美国") {
                "US"
            } else if g.name.contains("日本") {
                "JP"
            } else if g.name.contains("香港") {
                "HK"
            } else {
                "other"
            };
            println!("DIAG group={i} region={region} direct_members={} recursive_nodes={} nested_groups={} supported={}",g.members.len(),leaves.len(),seen.len()-1,g.error.is_none());
        }
        let verge = std::env::var("COCKPIT_TEST_CLASH_CONFIG")
            .expect("private Clash diagnostic config required");
        let cfg: Value = serde_yaml::from_str(&std::fs::read_to_string(&verge).unwrap()).unwrap();
        println!(
            "DIAG clash_tun={} fake_ip={} rule_count={}",
            cfg["tun"]["enable"] == true,
            cfg["dns"]["enhanced-mode"] == "fake-ip",
            cfg["rules"].as_array().map_or(0, |v| v.len())
        );
        if let Some(port) = cfg["mixed-port"].as_u64() {
            let c = reqwest::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap();
            for target in [
                "https://www.gstatic.com/generate_204",
                "https://api64.ipify.org?format=json",
            ] {
                let start = std::time::Instant::now();
                let r = c.get(target).send().await;
                println!(
                    "DIAG clash_target={} status={} timeout={} elapsed_ms={}",
                    if target.contains("gstatic") {
                        "latency"
                    } else {
                        "egress"
                    },
                    r.as_ref().map(|r| r.status().as_u16()).unwrap_or(0),
                    r.as_ref().err().is_some_and(|e| e.is_timeout()),
                    start.elapsed().as_millis()
                );
            }
        }
        let requested: Vec<usize> = std::env::var("COCKPIT_TEST_NODE_INDICES")
            .unwrap_or_else(|_| "0".into())
            .split(',')
            .map(|s| s.parse().expect("numeric node index required"))
            .take(4)
            .collect();
        let sample: Vec<_> = catalog
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.error.is_none())
            .filter(|(i, _)| requested.contains(i))
            .collect();
        for (index, node) in sample {
            let out = node.outbound.clone().unwrap();
            #[cfg(unix)]
            if let Ok(dir) = std::env::var("COCKPIT_TEST_DIAGNOSTIC_DIR") {
                use std::os::unix::fs::OpenOptionsExt;
                let path = std::path::Path::new(&dir).join(format!("node-{index}.json"));
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(path)
                    .unwrap();
                std::io::Write::write_all(
                    &mut file,
                    &serde_json::to_vec(
                        &configuration(out.clone(), 0, "diagnostic", "diagnostic").unwrap(),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
            let host = out["server"].as_str().unwrap();
            let port = out["server_port"].as_u64().unwrap() as u16;
            let dns = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::lookup_host((host, port)),
            )
            .await;
            let addresses: Vec<_> = dns
                .ok()
                .and_then(Result::ok)
                .map(|v| v.collect())
                .unwrap_or_default();
            let fake = addresses
                .iter()
                .any(|a| a.ip().to_string().starts_with("198.18."));
            let tcp = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::TcpStream::connect((host, port)),
            )
            .await;
            println!(
                "DIAG node={index} protocol={} dns_count={} fake_ip={fake} tcp_ok={}",
                node.protocol,
                addresses.len(),
                matches!(tcp, Ok(Ok(_)))
            );
            let started = std::time::Instant::now();
            let tunnel =
                start_with_binary_options(&binary, out, false, None, network.clone(), false).await;
            let tunnel = match tunnel {
                Ok(t) => t,
                Err(e) => {
                    println!("DIAG node={index} stage=start error={e}");
                    continue;
                }
            };
            println!(
                "DIAG node={index} start_ms={}",
                started.elapsed().as_millis()
            );
            let c = reqwest::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap();
            for target in [
                "https://www.gstatic.com/generate_204",
                "https://api64.ipify.org?format=json",
            ] {
                let start = std::time::Instant::now();
                let r = c.get(target).send().await;
                if let Err(e) = &r {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    println!(
                        "DIAG node={index} safe_error={}",
                        tunnel.failure(
                            if e.is_timeout() {
                                "PROXY_CONNECT_TIMEOUT"
                            } else {
                                "PROXY_PROBE_FAILED"
                            }
                            .into()
                        )
                    );
                }
                if std::env::var("COCKPIT_TEST_EXPECT_SUCCESS").as_deref() == Ok("1") {
                    assert!(
                        r.as_ref().is_ok_and(|r| r.status().is_success()),
                        "sample did not connect"
                    );
                }
                println!("DIAG node={index} target={} status={} timeout={} connect_error={} elapsed_ms={}",if target.contains("gstatic"){"latency"}else{"egress"},r.as_ref().map(|r|r.status().as_u16()).unwrap_or(0),r.as_ref().err().is_some_and(|e|e.is_timeout()),r.as_ref().err().is_some_and(|e|e.is_connect()),start.elapsed().as_millis());
            }
            tunnel.stop().await.unwrap();
        }
    }

    #[test]
    fn engine_lookup_does_not_search_path() {
        let _env = super::super::test_support::env_lock()
            .lock().unwrap_or_else(|error| error.into_inner());
        let missing = engine_path();
        if missing.is_err() {
            assert_eq!(missing.unwrap_err(), "PROXY_ENGINE_MISSING");
        } else {
            let path = missing.unwrap();
            assert!(path.ends_with("mihomo") || path.ends_with("mihomo.exe"));
            assert!(!path
                .to_string_lossy()
                .split(std::path::MAIN_SEPARATOR)
                .any(|part| part.eq_ignore_ascii_case("bin")
                    && path.to_string_lossy().contains("/usr/")));
        }
    }

    #[test]
    fn engine_lookup_covers_macos_bundle_resources_without_searching_path() {
        let exe = PathBuf::from("/Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools");
        let candidates = engine_candidates(&exe, "mihomo");
        assert!(candidates
            .iter()
            .any(|path| path.ends_with("Resources/proxy-engine/mihomo")));
        assert!(candidates
            .iter()
            .all(|path| !path.to_string_lossy().contains("/usr/bin")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn engine_lookup_covers_linux_resource_dir() {
        let exe = PathBuf::from("/usr/bin/cockpit_tools");
        let candidates = engine_candidates(&exe, "mihomo");
        assert!(candidates
            .iter()
            .any(|path| path == Path::new("/usr/lib/cockpit_tools/proxy-engine/mihomo")));
    }
}
