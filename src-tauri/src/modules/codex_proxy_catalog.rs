//! User-owned proxy sources. Secrets stay in an encrypted local catalog; IPC only gets summaries.
//! Account bindings are snapshots: refresh cannot reroute them; removing a source unbinds them.
use super::{
    account, atomic_write, codex_proxy_catalog_binding as binding,
    codex_proxy_manual_import as manual,
    codex_proxy_subscription_parser::{self as parser, ParsedCatalog},
    secure_account_storage,
};
/// 自建多节点策略的纯逻辑；与来源存储解耦，便于单独校验与单测。
#[path = "codex_proxy_strategy.rs"]
pub mod strategy;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
pub use strategy::{StrategyMember, StrategyOptions};

const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_STORE: u64 = 32 * 1024 * 1024;
const MAX_SOURCES: usize = 32;
const REFRESH_INTERVAL_MS: i64 = 6 * 60 * 60 * 1000;
const PARSER_VERSION: u8 = 1;
const DEFAULT_LATENCY_URL: &str = "http://www.gstatic.com/generate_204";
static DOWNLOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static IO: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static SOURCES_BUSY: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));
static AUTO_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize, Deserialize)]
struct Source {
    id: String,
    name: String,
    kind: String,
    url: Option<String>,
    catalog: ParsedCatalog,
    updated_at: i64,
    last_attempt_at: Option<i64>,
    auto_update: bool,
    error: Option<String>,
    revision: String,
    #[serde(default)]
    network: super::codex_proxy_network::NetworkOptions,
    /// 来源默认项只用于账号草稿预填；旧文件缺失该字段时按“无默认项”处理。
    #[serde(default)]
    default: Option<SourceDefault>,
    /// 默认项因节点消失被清除后置位，直到用户重新设置或清除。
    #[serde(default)]
    default_invalidated: bool,
    /// 仅策略来源有值：成员副本对应的原始来源身份；旧文件缺失该字段时按 None 处理。
    #[serde(default)]
    strategy_members: Option<Vec<StrategyMemberRecord>>,
    /// 订阅响应头里的用量与到期：只保存数字与时间戳，不保存响应头原文；
    /// 手动来源与旧文件缺失该字段时按“没有用量信息”处理。
    #[serde(default)]
    usage: Option<SourceUsage>,
    /// Only successful explicit import/refresh advances this marker. Old
    /// rejected nodes without retained definitions need the user's refresh.
    #[serde(default)]
    parser_version: u8,
}
/// 订阅用量（`subscription-userinfo` 响应头）：全部为数字，单位为字节与毫秒时间戳。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceUsage {
    /// 已上传与已下载字节；订阅端未提供时按 0 处理。
    #[serde(default)]
    pub upload: u64,
    #[serde(default)]
    pub download: u64,
    /// 套餐总流量；0 表示未知，界面不得据此算出 0%。
    #[serde(default)]
    pub total: u64,
    /// 到期时间（毫秒）；没有该字段表示订阅不限时或未提供。
    #[serde(default)]
    pub expire_at: Option<i64>,
    /// 最后一次成功读到用量的时间（毫秒），供界面判断数据新旧。
    #[serde(default)]
    pub at: i64,
}
/// 策略成员的真实来源身份：编辑策略时据此精确回读原来源节点，绝不按显示名猜测。
/// 只保存来源与节点的稳定 ID 及展示名，不含任何地址或凭据。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyMemberRecord {
    source_id: String,
    item_id: String,
    /// 副本名：内核按名称区分成员，同名去重后这里只留真正进入 catalog 的那个。
    name: String,
    /// 保存时的原来源名；来源改名后再次保存策略即刷新。
    source_name: String,
}
/// 已保存的来源默认项：稳定的资源 ID 与手选组成员，绝不保存节点地址或凭据。
#[derive(Clone, Serialize, Deserialize)]
struct SourceDefault {
    item_id: String,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    selections: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
struct Store {
    version: u8,
    sources: Vec<Source>,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            version: 1,
            sources: vec![],
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub sources: Vec<SourceView>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    network: super::codex_proxy_network::NetworkOptions,
    id: String,
    name: String,
    kind: String,
    updated_at: i64,
    revision: String,
    last_attempt_at: Option<i64>,
    auto_update: bool,
    error: Option<String>,
    needs_refresh: bool,
    default: Option<SourceDefaultView>,
    default_invalidated: bool,
    /// 仅策略来源有值：编辑弹框据此恢复上次保存的健康检查参数。
    strategy_options: Option<Value>,
    /// 仅策略来源有值：编辑弹框据此精确恢复每个成员的原来源节点。
    strategy_members: Option<Vec<StrategyMemberRecord>>,
    /// 订阅响应头里的用量与到期；没有该数据的来源不返回。
    usage: Option<SourceUsage>,
    nodes: Vec<NodeView>,
    groups: Vec<GroupView>,
}
/// 默认项的公开视图：只有资源 ID 与成员名，不含任何凭据。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDefaultView {
    item_id: String,
    group_id: Option<String>,
    selections: BTreeMap<String, String>,
}
#[derive(Serialize)]
pub struct NodeView {
    server: Option<String>,
    port: Option<u16>,
    insecure: bool,
    udp: Option<bool>,
    id: String,
    name: String,
    protocol: String,
    supported: bool,
    error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    id: String,
    name: String,
    kind: String,
    members: Vec<String>,
    supported: bool,
    error: Option<String>,
    issues: Vec<parser::GroupIssue>,
    insecure_node_ids: Vec<String>,
    test_url: Option<String>,
}
fn node_udp(node: &parser::ParsedNode) -> Option<bool> {
    let outbound = node.outbound.as_ref()?;
    let native = if outbound["type"] == "mihomo" {
        &outbound["proxy"]
    } else {
        outbound
    };
    match native["type"].as_str()? {
        "hysteria" | "hysteria2" | "tuic" => Some(true),
        "http" | "ssh" => Some(false),
        "ss" | "ssr" | "shadowsocks" | "socks" | "socks5" | "vmess" | "vless" | "trojan"
        | "wireguard" | "snell" | "anytls" | "mieru" => {
            Some(native.get("udp").and_then(Value::as_bool).unwrap_or(false))
        }
        _ => None,
    }
}

fn latency_url(group: Option<&parser::ParsedGroup>) -> Result<String, String> {
    let raw = group
        .and_then(|group| group.url.as_deref())
        .unwrap_or(DEFAULT_LATENCY_URL);
    let url = url::Url::parse(raw).map_err(|_| "CATALOG_INVALID")?;
    if raw.len() > 2048
        || raw.chars().any(char::is_control)
        || !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("CATALOG_INVALID".into());
    }
    Ok(url.into())
}

fn public_latency_url(group: &parser::ParsedGroup) -> Option<String> {
    let raw = latency_url(Some(group)).ok()?;
    let url = url::Url::parse(&raw).ok()?;
    // A custom health-check URL can contain a secret query even without URL
    // userinfo; it may be used locally but must not enter display metadata.
    if url.query().is_some() {
        return None;
    }
    Some(raw)
}

fn latency_target_url(
    catalog: &ParsedCatalog,
    node_id: &str,
    group_id: Option<&str>,
) -> Result<String, String> {
    let node = catalog
        .nodes
        .iter()
        .find(|node| node.id == node_id && node.error.is_none() && node.outbound.is_some())
        .ok_or("CATALOG_INVALID")?;
    let group = group_id
        .filter(|id| !id.is_empty())
        .map(|id| {
            catalog
                .groups
                .iter()
                .find(|group| group.id == id)
                .ok_or("CATALOG_NOT_FOUND")
        })
        .transpose()?;
    if let Some(group) = group {
        // A probe runs this one authorized node, not the whole group policy.
        // Unrelated missing/cyclic branches must not hide a reachable healthy
        // candidate. Group permission transactions keep their strict traversal.
        let groups: HashMap<_, _> = catalog
            .groups
            .iter()
            .map(|group| (group.name.as_str(), group))
            .collect();
        let mut visited = HashSet::new();
        let mut pending = vec![group];
        let mut reachable = false;
        while let Some(group) = pending.pop() {
            if !visited.insert(group.name.as_str()) {
                continue;
            }
            if group.members.contains(&node.name) {
                reachable = true;
                break;
            }
            pending.extend(
                group
                    .members
                    .iter()
                    .filter_map(|name| groups.get(name.as_str()).copied()),
            );
        }
        if !reachable {
            return Err("CATALOG_INVALID".into());
        }
    }
    latency_url(group)
}

fn view(store: &Store) -> CatalogView {
    CatalogView {
        sources: store
            .sources
            .iter()
            .map(|s| {
                let issues = parser::group_issues(&s.catalog);
                SourceView {
                    network: s.network.clone(),
                    id: s.id.clone(),
                    name: s.name.clone(),
                    kind: s.kind.clone(),
                    updated_at: s.updated_at,
                    revision: s.revision.clone(),
                    last_attempt_at: s.last_attempt_at,
                    auto_update: s.auto_update,
                    error: s.error.clone(),
                    needs_refresh: s.parser_version < PARSER_VERSION
                        && s.kind == "subscription"
                        && s.catalog
                            .nodes
                            .iter()
                            .any(|node| node.outbound.is_none() && node.error.is_some()),
                    default: s.default.as_ref().map(|d| SourceDefaultView {
                        item_id: d.item_id.clone(),
                        group_id: d.group_id.clone(),
                        selections: d.selections.clone(),
                    }),
                    default_invalidated: s.default_invalidated,
                    strategy_options: (s.kind == strategy::KIND)
                        .then(|| strategy::options_view(&s.catalog))
                        .flatten(),
                    usage: (s.kind == "subscription")
                        .then(|| s.usage.clone())
                        .flatten(),
                    strategy_members: (s.kind == strategy::KIND)
                        .then(|| s.strategy_members.clone())
                        .flatten(),
                    nodes: s
                        .catalog
                        .nodes
                        .iter()
                        .map(|n| NodeView {
                            server: n.outbound.as_ref().and_then(|v| {
                                let endpoint = if v["type"] == "mihomo" {
                                    &v["proxy"]
                                } else {
                                    v
                                };
                                endpoint["server"].as_str().map(str::to_owned)
                            }),
                            port: n.outbound.as_ref().and_then(|v| {
                                let endpoint = if v["type"] == "mihomo" {
                                    &v["proxy"]
                                } else {
                                    v
                                };
                                endpoint
                                    .get("port")
                                    .or_else(|| endpoint.get("server_port"))
                                    .and_then(serde_json::Value::as_u64)
                                    .and_then(|port| u16::try_from(port).ok())
                            }),
                            insecure: n.outbound.as_ref().is_some_and(|v| binding::is_insecure(v)),
                            udp: node_udp(n),
                            id: n.id.clone(),
                            name: n.name.clone(),
                            protocol: n.protocol.clone(),
                            supported: n.error.is_none() && n.outbound.is_some(),
                            error: n.error.clone(),
                        })
                        .collect(),
                    groups: s
                        .catalog
                        .groups
                        .iter()
                        .zip(issues)
                        .map(|(g, issues)| GroupView {
                            id: g.id.clone(),
                            name: g.name.clone(),
                            kind: g.kind.clone(),
                            members: g.members.clone(),
                            supported: g.error.is_none()
                                && strategy::group_supported(&s.kind, &g.kind),
                            error: g.error.clone(),
                            issues,
                            insecure_node_ids: parser::reachable_nodes(&s.catalog, &g.id)
                                .unwrap_or_default()
                                .into_iter()
                                .filter(|node| node.error.as_deref() == Some("PROXY_TLS_INSECURE"))
                                .map(|node| node.id.clone())
                                .collect(),
                            test_url: public_latency_url(g),
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}
/// 删除来源前的只读影响预览条目：只含账号 id、展示名与邮箱，不含任何凭据或地址。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDependencyAccount {
    id: String,
    name: String,
    email: String,
}
/// 删除来源前的只读影响预览：直接绑定该来源的账号、正在引用它的统一代理，以及引用它成员副本的策略。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDependenciesView {
    source_id: String,
    revision: String,
    accounts: Vec<SourceDependencyAccount>,
    account_count: usize,
    strategies: Vec<String>,
    strategy_count: usize,
    unified_proxy: bool,
}
/// 仍引用该来源成员副本的策略名称：只统计身份记录，不含任何地址或凭据。
fn strategy_dependency_names(sources: &[Source], source_id: &str) -> Vec<String> {
    sources
        .iter()
        .filter(|source| source.kind == strategy::KIND)
        .filter(|source| {
            source
                .strategy_members
                .as_deref()
                .is_some_and(|members| members.iter().any(|member| member.source_id == source_id))
        })
        .map(|source| source.name.clone())
        .collect()
}
/// 只保留直接绑定该来源的账号；展示名沿用账号名，缺失时回落到邮箱与 id。
fn dependency_accounts(
    accounts: Vec<crate::models::codex::CodexAccount>,
    source_id: &str,
) -> Vec<SourceDependencyAccount> {
    accounts
        .into_iter()
        .filter(|account| {
            account
                .egress_proxy_url
                .as_deref()
                .is_some_and(|raw| binding::belongs_to_source(raw, source_id))
        })
        .map(|account| {
            let name = account
                .account_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    let email = account.email.trim();
                    (!email.is_empty()).then(|| email.to_owned())
                })
                .unwrap_or_else(|| account.id.clone());
            SourceDependencyAccount {
                id: account.id,
                name,
                email: account.email,
            }
        })
        .collect()
}
fn path() -> Result<PathBuf, String> {
    Ok(account::resolve_data_dir()
        .map_err(|_| "CATALOG_STORAGE")?
        .join("codex-proxy-sources.json"))
}
fn read(path: &Path) -> Result<Store, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(_) => return Err("CATALOG_STORAGE".into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_STORE {
        return Err("CATALOG_STORAGE".into());
    }
    // Read-only decrypt: never restore backups, rotate data or generate a key on a read path.
    let mut store: Store = secure_account_storage::read_account_file_readonly(
        path,
        &path
            .parent()
            .ok_or("CATALOG_STORAGE")?
            .join("secure-account-storage.key"),
    )
    .map_err(|_| "CATALOG_STORAGE")?;
    if store.version != 1 || store.sources.len() > MAX_SOURCES {
        return Err("CATALOG_STORAGE".into());
    }
    for source in &mut store.sources {
        parser::revalidate_retained_nodes(&mut source.catalog);
        revalidate_groups(&mut source.catalog).map_err(|_| "CATALOG_STORAGE")?;
    }
    Ok(store)
}
fn mutate<T>(
    path: &Path,
    change: impl FnOnce(&mut Store) -> Result<T, String>,
) -> Result<T, String> {
    let parent = path.parent().ok_or("CATALOG_STORAGE")?;
    fs::create_dir_all(parent).map_err(|_| "CATALOG_STORAGE")?;
    let lock_path = parent.join("codex-proxy-sources.lock");
    if lock_path.is_symlink() {
        return Err("CATALOG_STORAGE".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| "CATALOG_STORAGE")?;
    lock.try_lock_exclusive().map_err(|_| "CATALOG_BUSY")?;
    let mut store = read(path)?;
    let result = change(&mut store)?;
    let plain = serde_json::to_vec(&store).map_err(|_| "CATALOG_STORAGE")?;
    if plain.len() as u64 > MAX_STORE / 2 {
        return Err("CATALOG_LIMIT".into());
    }
    let encrypted = secure_account_storage::serialize_account_file("codex-proxy-sources", &store)
        .map_err(|_| "CATALOG_STORAGE")?;
    atomic_write::write_secret_string_atomic(path, &encrypted).map_err(|_| "CATALOG_STORAGE")?;
    Ok(result)
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    blocking_with_timeout(Duration::from_secs(10), work).await
}
async fn blocking_with_timeout<T: Send + 'static>(
    timeout: Duration,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = IO.try_acquire().map_err(|_| "CATALOG_BUSY")?;
    tokio::time::timeout(
        timeout,
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        }),
    )
    .await
    .map_err(|_| "CATALOG_TIMEOUT")?
    .map_err(|_| "CATALOG_STORAGE")?
}
pub async fn list() -> Result<CatalogView, String> {
    blocking(|| Ok(view(&read(&path()?)?))).await
}
async fn source(id: &str) -> Result<Source, String> {
    let id = id.to_owned();
    blocking(move || {
        read(&path()?)?
            .sources
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| "CATALOG_NOT_FOUND".into())
    })
    .await
}
fn valid_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > 80
        || name.len() > 256
        || name.chars().any(char::is_control)
        || name.contains("://")
    {
        return Err("CATALOG_NAME".into());
    }
    Ok(name.to_owned())
}

fn safe_generated_name(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.contains('@')
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('?')
        || raw.contains("token=")
        || raw.contains("password=")
    {
        return None;
    }
    let name: String = raw.chars().take(80).collect();
    if name.len() > 256 {
        return None;
    }
    valid_name(&name).ok()
}
/// 解析订阅响应头 `subscription-userinfo`，形如：
/// `upload=0; download=1024; total=104857600; expire=1759999999`。
/// 只接受已知键与十进制数字；单个字段非法时忽略该字段，全部缺失或全部非法返回 None。
/// 头里可能带套餐标识等其它字段，一律忽略，也不保存响应头原文。
fn subscription_usage(headers: &reqwest::header::HeaderMap) -> Option<SourceUsage> {
    /// 1 PiB：超过这个量级按解析错误处理，避免界面显示荒唐数值。
    const MAX_BYTES: u64 = 1 << 50;
    let raw = headers.get("subscription-userinfo")?.to_str().ok()?;
    if raw.len() > 1024 {
        return None;
    }
    let (mut upload, mut download, mut total, mut expire) = (None, None, None, None);
    for field in raw.split(';') {
        let Some((key, value)) = field.trim().split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "upload" => upload = value.parse::<u64>().ok().filter(|v| *v <= MAX_BYTES),
            "download" => download = value.parse::<u64>().ok().filter(|v| *v <= MAX_BYTES),
            "total" => total = value.parse::<u64>().ok().filter(|v| *v <= MAX_BYTES),
            // `expire=0` 与负数都表示“没有到期时间”，不当作有效值。
            "expire" => expire = value.parse::<i64>().ok().filter(|v| *v > 0),
            _ => {}
        }
    }
    if upload.is_none() && download.is_none() && total.is_none() && expire.is_none() {
        return None;
    }
    Some(SourceUsage {
        upload: upload.unwrap_or(0),
        download: download.unwrap_or(0),
        total: total.unwrap_or(0),
        expire_at: expire.map(|seconds| seconds.saturating_mul(1000)),
        at: chrono::Utc::now().timestamp_millis(),
    })
}
fn subscription_title(headers: &reqwest::header::HeaderMap) -> Option<String> {
    use base64::Engine as _;
    if let Some(title) = headers
        .get("profile-title")
        .and_then(|v| v.to_str().ok())
        .filter(|s| s.len() <= 2048)
    {
        let decoded = if let Some(encoded) = title.strip_prefix("base64:") {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
        } else {
            Some(title.to_owned())
        };
        if let Some(name) = decoded.and_then(|s| safe_generated_name(&s)) {
            return Some(name);
        }
    }
    let disposition = headers
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    if disposition.len() > 2048 {
        return None;
    }
    for key in ["filename*", "filename"] {
        for field in disposition.split(';') {
            let Some((k, v)) = field.trim().split_once('=') else {
                continue;
            };
            if !k.eq_ignore_ascii_case(key) {
                continue;
            }
            let value = v.trim().trim_matches('"');
            let value = if key == "filename*" {
                value
                    .strip_prefix("UTF-8''")
                    .or_else(|| value.strip_prefix("utf-8''"))?
            } else {
                value
            };
            let value = urlencoding::decode(value).ok()?;
            let name = value
                .strip_suffix(".yaml")
                .or_else(|| value.strip_suffix(".yml"))
                .unwrap_or(&value);
            if let Some(name) = safe_generated_name(name) {
                return Some(name);
            }
        }
    }
    None
}
fn fingerprints(store: &Store) -> HashSet<String> {
    store
        .sources
        .iter()
        .flat_map(|s| s.catalog.nodes.iter())
        .filter_map(manual::fingerprint)
        .collect()
}
pub async fn preview(
    input: String,
    options: manual::ImportOptions,
) -> Result<manual::ImportPreview, String> {
    blocking(move || {
        Ok(manual::prepare(&input, &options, &fingerprints(&read(&path()?)?))?.preview)
    })
    .await
}
pub async fn rename(source_id: String, name: String) -> Result<CatalogView, String> {
    let name = valid_name(&name)?;
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string())?;
    let state = operation.state.clone();
    blocking(move || {
        mutate(&path()?, |store| {
            let source = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            // 策略的分组名与来源名保持同步：分组 ID 由来源 ID 派生，改名不会让已有绑定失效。
            if source.kind == strategy::KIND {
                strategy::rename_group(&mut source.catalog, &name)?;
            }
            commit(&state)?;
            source.name = name;
            source.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}

pub async fn latency(
    request_id: String,
    source_id: String,
    node_id: String,
    revision: String,
    group_id: Option<String>,
) -> Result<super::codex_proxy_probe::LatencyResult, String> {
    latency_with_preflight(
        request_id,
        Duration::from_secs(10),
        super::codex_proxy_engine_preflight::require(),
        async {
            let s = source(&source_id).await?;
            if s.revision != revision {
                return Err("CATALOG_CHANGED".into());
            }
            let (input, test_url) = blocking(move || {
                let test_url = latency_target_url(&s.catalog, &node_id, group_id.as_deref())?;
                let input = binding::with_network(
                    &binding::encode(&s.id, &s.name, &node_id, &s.catalog, &BTreeMap::new())?,
                    s.network,
                )?;
                Ok((input, test_url))
            })
            .await?;
            super::codex_proxy_probe::latency_resource(input, test_url).await
        },
    )
    .await
}

/// Preparation belongs to the same cancellable deadline as the node request.
/// The shared engine check may finish for other callers after cancellation, but
/// this operation can no longer start its catalog read or temporary engine.
async fn latency_with_preflight<T>(
    request_id: String,
    timeout: Duration,
    preflight: impl std::future::Future<Output = Result<(), String>>,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let op = Operation::begin(request_id)?;
    cancellable(&op, async {
        let deadline = tokio::time::Instant::now() + timeout;
        tokio::time::timeout_at(deadline, async {
            preflight.await?;
            // Cancellation and a completed preflight can become ready together.
            // Recheck before work so select ordering cannot start a stale probe.
            if op.state.load(Ordering::Acquire) == 1 {
                return Err("CATALOG_CANCELLED".into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("PROXY_PROBE_TIMEOUT".into());
            }
            work.await
        })
        .await
        .map_err(|_| "PROXY_PROBE_TIMEOUT".to_string())?
    })
    .await
}
#[derive(Clone)]
struct Job {
    state: Arc<AtomicU8>,
    created: Instant,
    active: bool,
}
struct Operation {
    id: String,
    state: Arc<AtomicU8>,
}
impl Operation {
    fn begin(id: String) -> Result<Self, String> {
        if uuid::Uuid::parse_str(&id).is_err() {
            return Err("CATALOG_INVALID".into());
        }
        let mut jobs = JOBS.lock().map_err(|_| "CATALOG_BUSY")?;
        jobs.retain(|_, j| j.active || j.created.elapsed() < Duration::from_secs(60));
        if let Some(j) = jobs.get(&id) {
            if j.active {
                return Err("CATALOG_BUSY".into());
            }
            jobs.remove(&id);
            return Err("CATALOG_CANCELLED".into());
        }
        if jobs.len() >= 64 {
            return Err("CATALOG_BUSY".into());
        }
        let state = Arc::new(AtomicU8::new(0));
        jobs.insert(
            id.clone(),
            Job {
                state: state.clone(),
                created: Instant::now(),
                active: true,
            },
        );
        Ok(Self { id, state })
    }
    async fn cancelled(&self) {
        while self.state.load(Ordering::Acquire) != 1 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        // A timed-out spawn_blocking worker may still be waiting on disk. Revoke
        // its right to publish before removing the cancellation registration.
        let _ = self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
        if let Ok(mut jobs) = JOBS.lock() {
            jobs.remove(&self.id);
        }
    }
}
pub fn cancel(id: String) -> Result<(), String> {
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err("CATALOG_INVALID".into());
    }
    let mut jobs = JOBS.lock().map_err(|_| "CATALOG_BUSY")?;
    jobs.retain(|_, j| j.active || j.created.elapsed() < Duration::from_secs(60));
    if let Some(j) = jobs.get(&id) {
        if j.state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            == Err(2)
        {
            return Err("CATALOG_FINISHING".into());
        }
    } else if jobs.len() < 64 {
        jobs.insert(
            id,
            Job {
                state: Arc::new(AtomicU8::new(1)),
                created: Instant::now(),
                active: false,
            },
        );
    }
    Ok(())
}
pub(crate) struct SourceGuard(String);
impl SourceGuard {
    pub(crate) fn new(id: String) -> Result<Self, String> {
        if !SOURCES_BUSY
            .lock()
            .map_err(|_| "CATALOG_BUSY")?
            .insert(id.clone())
        {
            return Err("CATALOG_BUSY".into());
        }
        Ok(Self(id))
    }
}
impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Ok(mut busy) = SOURCES_BUSY.lock() {
            busy.remove(&self.0);
        }
    }
}
fn commit(state: &AtomicU8) -> Result<(), String> {
    state
        .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| "CATALOG_CANCELLED".into())
}
fn subscription_url(input: &str) -> Result<url::Url, String> {
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("CATALOG_URL".into());
    }
    let u = url::Url::parse(input.trim()).map_err(|_| "CATALOG_URL")?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("CATALOG_URL".into());
    }
    Ok(u)
}
struct Download {
    body: String,
    title: Option<String>,
    usage: Option<SourceUsage>,
}
async fn fetch(input: &str) -> Result<Download, String> {
    let _permit = DOWNLOADS.try_acquire().map_err(|_| "CATALOG_BUSY")?;
    let url = subscription_url(input)?;
    let origin = url.origin();
    let client = subscription_client_builder()
        .build()
        .map_err(|_| "CATALOG_DOWNLOAD")?;
    fetch_with_client(client, url, origin).await
}

fn subscription_client_builder() -> reqwest::ClientBuilder {
    // Providers negotiate their output using this compatibility token. Plain
    // "Clash" can silently strip VLESS/Hysteria2 and their groups from responses.
    // Keep our application identity; this does not claim we run a Mihomo engine.
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .user_agent("clash.meta/CockpitTools")
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30))
}

async fn fetch_with_client(
    client: reqwest::Client,
    mut url: url::Url,
    origin: url::Origin,
) -> Result<Download, String> {
    // No account auth headers, cookies, remote converter or recursive provider fetches.
    for hop in 0..=3 {
        let response = client.get(url.clone()).send().await.map_err(|e| {
            if e.is_timeout() {
                "CATALOG_TIMEOUT"
            } else {
                "CATALOG_DOWNLOAD"
            }
        })?;
        if response.status().is_redirection() {
            if hop == 3 {
                return Err("CATALOG_REDIRECT".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or("CATALOG_REDIRECT")?;
            let next = url.join(location).map_err(|_| "CATALOG_REDIRECT")?;
            subscription_url(next.as_str())?;
            if next.origin() != origin {
                return Err("CATALOG_REDIRECT".into());
            }
            url = next;
            continue;
        }
        let title = subscription_title(response.headers()).filter(|name| {
            !url.path_segments()
                .is_some_and(|parts| parts.filter(|p| p.len() >= 8).any(|p| name.contains(p)))
                && !url
                    .query_pairs()
                    .any(|(_, v)| v.len() >= 6 && name.contains(v.as_ref()))
        });
        let usage = subscription_usage(response.headers());
        return Ok(Download {
            body: read_response(response).await?,
            title,
            usage,
        });
    }
    Err("CATALOG_DOWNLOAD".into())
}
async fn read_response(mut response: reqwest::Response) -> Result<String, String> {
    if !response.status().is_success() {
        return Err("CATALOG_DOWNLOAD".into());
    }
    if response
        .content_length()
        .is_some_and(|l| l > MAX_BODY as u64)
    {
        return Err("CATALOG_LIMIT".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "CATALOG_DOWNLOAD")? {
        if body.len() + chunk.len() > MAX_BODY {
            return Err("CATALOG_LIMIT".into());
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| "CATALOG_FORMAT".into())
}

async fn parsed(input: String) -> Result<ParsedCatalog, String> {
    if input.len() > MAX_BODY {
        return Err("CATALOG_LIMIT".into());
    }
    blocking(move || parser::parse(&input)).await
}
async fn cancellable<T>(
    op: &Operation,
    work: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    let result = tokio::select! {
        _ = op.cancelled() => Err("CATALOG_CANCELLED".into()),
        result = tokio::time::timeout(Duration::from_secs(45), work) =>
            result.unwrap_or_else(|_| Err("CATALOG_TIMEOUT".into())),
    };
    if result.is_err() {
        // Includes nested disk timeouts, not only the outer network deadline.
        let _ = op
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    result
}

pub async fn import(
    request_id: String,
    name: String,
    input: String,
    kind: String,
    options: manual::ImportOptions,
) -> Result<CatalogView, String> {
    let explicit_name = if name.trim().is_empty() {
        None
    } else {
        Some(valid_name(&name)?)
    };
    if !matches!(kind.as_str(), "subscription" | "manual") {
        return Err("CATALOG_INVALID".into());
    }
    let op = Operation::begin(request_id)?;
    cancellable(&op, async {
        let url = if kind == "subscription" {
            Some(subscription_url(&input)?.to_string())
        } else {
            None
        };
        let _source_guard = url
            .as_ref()
            .map(|url| {
                use sha2::{Digest, Sha256};
                SourceGuard::new(format!("import:{:x}", Sha256::digest(url.as_bytes())))
            })
            .transpose()?;
        let (mut catalog, suggested_name, usage) = if let Some(url) = url.as_ref() {
            let download = fetch(url).await?;
            let fallback = subscription_url(url)?
                .host_str()
                .unwrap_or("Subscription")
                .to_owned();
            let usage = download.usage;
            (
                parsed(download.body).await?,
                download.title.unwrap_or(fallback),
                usage,
            )
        } else {
            let opts = options.clone();
            let prepared = blocking(move || {
                let store = read(&path()?)?;
                manual::prepare(&input, &opts, &fingerprints(&store))
            })
            .await?;
            if prepared.preview.invalid > 0 && !options.skip_invalid {
                return Err("IMPORT_INVALID".into());
            }
            let mut catalog = prepared.catalog;
            if options.skip_invalid {
                catalog.nodes.retain(|n| n.error.is_none());
                parser::validate(&mut catalog)?;
            }
            if catalog.nodes.is_empty() {
                return Err("IMPORT_EMPTY".into());
            }
            let fallback =
                valid_name(&options.fallback_name).unwrap_or_else(|_| "Proxy list".into());
            let title = if catalog.nodes.len() == 1 {
                catalog.nodes[0].name.clone()
            } else {
                fallback
            };
            (catalog, title, None)
        };
        let name = explicit_name.unwrap_or_else(|| {
            safe_generated_name(&suggested_name).unwrap_or_else(|| "Proxy".into())
        });
        let state = op.state.clone();
        blocking(move || {
            mutate(&path()?, |store| {
                if store.sources.len() >= MAX_SOURCES {
                    return Err("CATALOG_LIMIT".into());
                }
                if url.is_some() && store.sources.iter().any(|s| s.url == url) {
                    return Err("CATALOG_EXISTS".into());
                }
                if kind == "manual" && options.skip_duplicates && catalog.groups.is_empty() {
                    let existing = fingerprints(store);
                    catalog
                        .nodes
                        .retain(|n| manual::fingerprint(n).is_some_and(|f| !existing.contains(&f)));
                    if catalog.nodes.is_empty() {
                        return Err("IMPORT_EMPTY".into());
                    }
                }
                commit(&state)?;
                store.sources.push(Source {
                    id: uuid::Uuid::new_v4().to_string(),
                    name,
                    kind,
                    url,
                    catalog,
                    network: Default::default(),
                    updated_at: chrono::Utc::now().timestamp_millis(),
                    last_attempt_at: None,
                    auto_update: false,
                    error: None,
                    revision: uuid::Uuid::new_v4().to_string(),
                    default: None,
                    default_invalidated: false,
                    strategy_members: None,
                    usage,
                    parser_version: PARSER_VERSION,
                });
                Ok(view(store))
            })
        })
        .await
    })
    .await
}
/// 把勾选项解析成待复制的节点定义：只接受原来源里受支持的真实节点。
fn strategy_member_node(source: &Source, item_id: &str) -> Result<strategy::StrategyNode, String> {
    let node = source
        .catalog
        .nodes
        .iter()
        .find(|node| node.id == item_id)
        .ok_or("CATALOG_CHANGED")?;
    // 未获授权的节点（例如跳过证书校验）不进入策略：不把不可执行的成员复制成可用策略。
    if node.error.is_some() {
        return Err("CATALOG_INVALID".into());
    }
    let outbound = node.outbound.clone().ok_or("CATALOG_INVALID")?;
    Ok(strategy::StrategyNode {
        name: node.name.clone(),
        protocol: node.protocol.clone(),
        outbound,
    })
}
/// 解析一个成员待复制的节点，返回（记录里的来源名，副本定义）。
/// 原来源仍在时按稳定节点 ID 解析；只有编辑已有策略且原来源已被删除时，才退回该策略上次
/// 保存的副本，避免再次编辑保存时静默丢掉仍然可用的成员。新建策略、其他策略的副本，以及
/// 原来源仍在但节点被移除或不可用的情况都不参与回退，保持原有失败语义。
fn strategy_member_copy(
    sources: &[Source],
    strategy_id: Option<&str>,
    member: &StrategyMember,
) -> Result<(String, strategy::StrategyNode), String> {
    if let Some(source) = sources.iter().find(|source| source.id == member.source_id) {
        // 原来源名每次都从当前来源读取：来源改名后再次保存即刷新记录。
        return Ok((
            source.name.clone(),
            strategy_member_node(source, &member.item_id)?,
        ));
    }
    let strategy = strategy_id
        .and_then(|id| {
            sources
                .iter()
                .find(|source| source.id == id && source.kind == strategy::KIND)
        })
        .ok_or_else(|| "CATALOG_NOT_FOUND".to_string())?;
    let record = strategy
        .strategy_members
        .as_deref()
        .and_then(|records| {
            records.iter().find(|record| {
                record.source_id == member.source_id && record.item_id == member.item_id
            })
        })
        .ok_or_else(|| "CATALOG_NOT_FOUND".to_string())?;
    // 副本按记录里的名称取自策略自己的 catalog；取不到或不再可用时沿用原有错误码。
    let copy = strategy
        .catalog
        .nodes
        .iter()
        .find(|node| node.name == record.name)
        .ok_or("CATALOG_CHANGED")?;
    // 来源已删除，没有更权威的来源名可读，只能沿用上次保存时记下的来源名。
    Ok((
        record.source_name.clone(),
        strategy_member_node(strategy, &copy.id)?,
    ))
}
/// 保存自建多节点策略：把勾选节点的 outbound 复制进策略来源，并按勾选顺序生成一个分组。
/// 策略来源没有 URL、不参与订阅自动刷新，只复用现有来源列表、默认项、绑定与删除链路。
pub async fn save_strategy(
    id: Option<String>,
    name: String,
    kind: String,
    members: Vec<StrategyMember>,
    options: StrategyOptions,
) -> Result<CatalogView, String> {
    let id = id.filter(|id| !id.trim().is_empty());
    let name = valid_name(&name)?;
    strategy::validate(&kind, &members, &options)?;
    blocking(move || {
        mutate(&path()?, |store| {
            let resolved = members
                .iter()
                .map(|member| strategy_member_copy(&store.sources, id.as_deref(), member))
                .collect::<Result<Vec<_>, String>>()?;
            // 内核按副本名区分成员：同名只保留首次勾选的，身份记录必须与 strategy::build
            // 的去重规则完全一致，否则编辑策略时会恢复到并未进入 catalog 的那个来源节点。
            let mut seen = HashSet::with_capacity(resolved.len());
            let mut records = Vec::with_capacity(resolved.len());
            let mut copies = Vec::with_capacity(resolved.len());
            for (member, (source_name, node)) in members.iter().zip(resolved) {
                if !seen.insert(node.name.clone()) {
                    continue;
                }
                records.push(StrategyMemberRecord {
                    source_id: member.source_id.clone(),
                    item_id: member.item_id.clone(),
                    name: node.name.clone(),
                    source_name,
                });
                copies.push(node);
            }
            let source_id = match &id {
                Some(id) => id.clone(),
                None if store.sources.len() >= MAX_SOURCES => return Err("CATALOG_LIMIT".into()),
                None => uuid::Uuid::new_v4().to_string(),
            };
            let catalog = strategy::build(
                &name,
                &kind,
                &options,
                strategy::group_id(&source_id),
                copies,
            )?;
            let now = chrono::Utc::now().timestamp_millis();
            match id {
                Some(_) => {
                    let source = store
                        .sources
                        .iter_mut()
                        .find(|source| source.id == source_id)
                        .ok_or("CATALOG_NOT_FOUND")?;
                    if source.kind != strategy::KIND {
                        return Err("CATALOG_INVALID".into());
                    }
                    source.name = name;
                    source.catalog = catalog;
                    source.parser_version = PARSER_VERSION;
                    // 编辑保存整体替换身份记录，绝不追加历史成员。
                    source.strategy_members = Some(records);
                    source.error = None;
                    source.updated_at = now;
                    source.revision = uuid::Uuid::new_v4().to_string();
                    // 成员被移除时默认项失效并提示重选，绝不指向别的节点。
                    drop_invalid_default(source);
                }
                None => store.sources.push(Source {
                    id: source_id,
                    name,
                    kind: strategy::KIND.into(),
                    url: None,
                    catalog,
                    network: Default::default(),
                    updated_at: now,
                    last_attempt_at: None,
                    auto_update: false,
                    error: None,
                    revision: uuid::Uuid::new_v4().to_string(),
                    default: None,
                    default_invalidated: false,
                    strategy_members: Some(records),
                    usage: None,
                    parser_version: PARSER_VERSION,
                }),
            }
            Ok(view(store))
        })
    })
    .await
}
/// 策略删除只允许作用于策略来源，避免误删订阅或手写来源。
pub async fn ensure_strategy(source_id: &str) -> Result<(), String> {
    let stored = source(source_id).await?;
    if stored.kind != strategy::KIND {
        return Err("CATALOG_INVALID".into());
    }
    Ok(())
}
pub async fn refresh(request_id: String, source_id: String) -> Result<CatalogView, String> {
    let _guard = SourceGuard::new(source_id.clone())?;
    let op = Operation::begin(request_id)?;
    let original = cancellable(&op, source(&source_id)).await?;
    let url = original.url.clone().ok_or("CATALOG_INVALID")?;
    let result = cancellable(&op, async {
        let download = fetch(&url).await?;
        // 有些订阅只在部分请求里带用量头：缺字段时保留上一次的数值，避免界面忽隐忽现。
        let usage = download.usage;
        let catalog = parsed(download.body).await?;
        let state = op.state.clone();
        let id = source_id.clone();
        let revision = original.revision.clone();
        blocking(move || {
            mutate(&path()?, |store| {
                let s = store
                    .sources
                    .iter_mut()
                    .find(|s| s.id == id)
                    .ok_or("CATALOG_NOT_FOUND")?;
                if s.revision != revision {
                    return Err("CATALOG_CHANGED".into());
                }
                commit(&state)?;
                let mut catalog = catalog;
                preserve_permissions(&s.catalog, &mut catalog)?;
                s.catalog = catalog;
                s.parser_version = PARSER_VERSION;
                // 节点消失时不静默换节点：清除默认项，由资源和账号页提示用户重新设置。
                drop_invalid_default(s);
                s.updated_at = chrono::Utc::now().timestamp_millis();
                s.last_attempt_at = Some(s.updated_at);
                s.error = None;
                if usage.is_some() {
                    s.usage = usage;
                }
                s.revision = uuid::Uuid::new_v4().to_string();
                Ok(view(store))
            })
        })
        .await
    })
    .await;
    if let Err(error) = &result {
        // Retain the usable source. Error recording never replaces a newer refresh/removal.
        if error != "CATALOG_CANCELLED" && op.state.load(Ordering::Acquire) != 2 {
            let revision = original.revision;
            let error = error.clone();
            let _ = blocking(move || {
                mutate(&path()?, |store| {
                    if let Some(s) = store
                        .sources
                        .iter_mut()
                        .find(|s| s.id == source_id && s.revision == revision)
                    {
                        s.error = Some(error);
                        s.last_attempt_at = Some(chrono::Utc::now().timestamp_millis());
                    }
                    Ok(())
                })
            })
            .await;
        }
    }
    result
}
pub async fn remove(source_id: String) -> Result<CatalogView, String> {
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string())?;
    let state = operation.state.clone();
    blocking(move || {
        mutate(&path()?, |store| {
            commit(&state)?;
            let before = store.sources.len();
            store.sources.retain(|s| s.id != source_id);
            if before == store.sources.len() {
                return Err("CATALOG_NOT_FOUND".into());
            }
            Ok(view(store))
        })
    })
    .await
}
/// 删除来源前的影响预览，按最新落盘状态汇总账号、策略与统一代理；只读、不修改任何配置。
pub async fn dependencies(source_id: String) -> Result<SourceDependenciesView, String> {
    // Serialize against operations that use SourceGuard; revision checks below also
    // cover catalog mutations which predate the guard discipline.
    let _guard = SourceGuard::new(source_id.clone())?;
    let stored = source(&source_id).await?;
    let target = source_id.clone();
    let (strategies, accounts) = blocking(move || {
        let strategies = strategy_dependency_names(&read(&path()?)?.sources, &target);
        let accounts = dependency_accounts(
            crate::modules::codex_account::list_accounts_for_proxy_removal()?,
            &target,
        );
        Ok((strategies, accounts))
    })
    .await?;
    let unified = crate::modules::codex_unified_proxy::ensure_loaded().await?;
    let final_revision = source(&source_id).await?.revision;
    if final_revision != stored.revision {
        return Err("CATALOG_CHANGED".into());
    }
    // 只有已启用且正引用该来源的统一代理才会随删除关闭。
    let unified_proxy = unified.active()
        && unified
            .reference
            .as_ref()
            .is_some_and(|reference| reference.source_id == source_id);
    Ok(SourceDependenciesView {
        source_id,
        revision: stored.revision,
        account_count: accounts.len(),
        accounts,
        strategy_count: strategies.len(),
        strategies,
        unified_proxy,
    })
}
pub async fn ensure_exists(source_id: &str) -> Result<(), String> {
    source(source_id).await.map(|_| ())
}
/// 默认项可解析性与选择器一致：节点或分组必须受支持，手选组必须给出合法成员。
/// 自建策略、订阅与手写来源均保留受支持的原生分组策略。
/// 与前端 `defaultProxySelections` 使用同一套规则，避免草稿预填与后端校验分叉。
fn default_error(
    source_kind: &str,
    catalog: &ParsedCatalog,
    item_id: &str,
    selections: &BTreeMap<String, String>,
) -> Option<&'static str> {
    fn node_supported(node: &parser::ParsedNode) -> bool {
        node.error.is_none() && node.outbound.is_some()
    }
    fn group_supported(source_kind: &str, group: &parser::ParsedGroup) -> bool {
        group.error.is_none() && strategy::group_supported(source_kind, &group.kind)
    }
    fn visit(
        source_kind: &str,
        name: &str,
        catalog: &ParsedCatalog,
        selections: &BTreeMap<String, String>,
        path: &mut Vec<String>,
    ) -> Option<&'static str> {
        if binding::is_blocking_builtin(name) {
            return None;
        }
        if path.len() >= 16 || path.iter().any(|entry| entry.as_str() == name) {
            return Some("CATALOG_INVALID");
        }
        if let Some(node) = catalog.nodes.iter().find(|node| node.name == name) {
            return (!node_supported(node)).then_some("CATALOG_INVALID");
        }
        let Some(group) = catalog.groups.iter().find(|group| group.name == name) else {
            return Some("CATALOG_INVALID");
        };
        if !group_supported(source_kind, group) || group.members.is_empty() {
            return Some("CATALOG_INVALID");
        }
        // 手选组必须明确成员；自动分组保留策略，仅校验成员可解析。
        let members: Vec<&str> = match group.kind.as_str() {
            "select" => match selections.get(&group.id) {
                Some(member) if group.members.iter().any(|entry| entry == member) => {
                    vec![member.as_str()]
                }
                _ => return Some("CATALOG_INVALID"),
            },
            _ => group.members.iter().map(String::as_str).collect(),
        };
        path.push(name.to_owned());
        let problem = members
            .into_iter()
            .find_map(|member| visit(source_kind, member, catalog, selections, path));
        path.pop();
        problem
    }
    let Some(name) = catalog
        .nodes
        .iter()
        .find(|node| node.id == item_id)
        .map(|node| node.name.clone())
        .or_else(|| {
            catalog
                .groups
                .iter()
                .find(|group| group.id == item_id)
                .map(|group| group.name.clone())
        })
    else {
        // 该来源已没有这个资源：属于陈旧视图，提示重新选择而不是写入。
        return Some("CATALOG_CHANGED");
    };
    if binding::is_builtin_name(&name) {
        return Some("CATALOG_INVALID");
    }
    let mut path = Vec::new();
    visit(source_kind, &name, catalog, selections, &mut path)
}
/// 手选组上下文必须存在且确实包含该默认项，否则预填出来的浏览位置不可解释。
fn group_context_error(
    catalog: &ParsedCatalog,
    item_id: &str,
    group_id: Option<&str>,
) -> Option<&'static str> {
    let id = group_id.filter(|id| !id.is_empty())?;
    let Some(group) = catalog.groups.iter().find(|group| group.id == id) else {
        return Some("CATALOG_INVALID");
    };
    if id == item_id {
        return None;
    }
    let member = catalog
        .nodes
        .iter()
        .find(|node| node.id == item_id)
        .map(|node| node.name.as_str())
        .or_else(|| {
            catalog
                .groups
                .iter()
                .find(|child| child.id == item_id)
                .map(|child| child.name.as_str())
        });
    match member {
        Some(name) if group.members.iter().any(|member| member == name) => None,
        _ => Some("CATALOG_INVALID"),
    }
}
/// 订阅刷新或权限变化后默认项不再可解析时清除并标记，绝不替换成别的节点。
fn drop_invalid_default(source: &mut Source) -> bool {
    let invalid = source.default.as_ref().is_some_and(|default| {
        default_error(
            &source.kind,
            &source.catalog,
            &default.item_id,
            &default.selections,
        )
        .is_some()
    });
    if invalid {
        source.default = None;
        source.default_invalidated = true;
    }
    invalid
}
/// 保存来源默认项。写入前按当前落盘目录校验，非法输入只返回固定错误码。
pub async fn set_default(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<CatalogView, String> {
    let group_id = group_id.filter(|id| !id.is_empty());
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string())?;
    let state = operation.state.clone();
    blocking(move || {
        mutate(&path()?, |store| {
            let source = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if let Some(problem) =
                default_error(&source.kind, &source.catalog, &item_id, &selections)
            {
                return Err(problem.into());
            }
            let context = group_context_error(&source.catalog, &item_id, group_id.as_deref());
            if let Some(problem) = context {
                return Err(problem.into());
            }
            commit(&state)?;
            source.default = Some(SourceDefault {
                item_id,
                group_id,
                selections,
            });
            source.default_invalidated = false;
            source.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}
/// 清除来源默认项；同时用于确认“默认项已失效”提示，使来源回到无默认项状态。
pub async fn clear_default(source_id: String) -> Result<CatalogView, String> {
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string())?;
    let state = operation.state.clone();
    blocking(move || {
        mutate(&path()?, |store| {
            let source = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            commit(&state)?;
            source.default = None;
            source.default_invalidated = false;
            source.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}
pub async fn set_auto_update(source_id: String, enabled: bool) -> Result<CatalogView, String> {
    let operation = Operation::begin(uuid::Uuid::new_v4().to_string())?;
    let state = operation.state.clone();
    blocking(move || {
        mutate(&path()?, |store| {
            let s = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if s.url.is_none() {
                return Err("CATALOG_INVALID".into());
            }
            commit(&state)?;
            s.auto_update = enabled;
            s.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}
fn preserve_permissions(old: &ParsedCatalog, new: &mut ParsedCatalog) -> Result<(), String> {
    for node in &mut new.nodes {
        if node.error.as_deref() == Some("PROXY_TLS_INSECURE")
            && old.nodes.iter().any(|old| {
                old.id == node.id && old.error.is_none() && old.outbound == node.outbound
            })
        {
            node.error = None;
        }
    }
    revalidate_groups(new)
}
fn revalidate_groups(catalog: &mut ParsedCatalog) -> Result<(), String> {
    parser::validate(catalog)
}
pub async fn set_network(
    source_id: String,
    revision: String,
    network: super::codex_proxy_network::NetworkOptions,
) -> Result<CatalogView, String> {
    network.validate()?;
    let network = super::codex_proxy_network::NetworkOptions {
        doh: network.doh,
        interface: String::new(),
    };
    blocking(move || {
        mutate(&path()?, |store| {
            let s = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if s.revision != revision {
                return Err("CATALOG_CHANGED".into());
            }
            s.network = network;
            s.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}
pub async fn set_insecure(
    source_id: String,
    node_id: String,
    revision: String,
    enabled: bool,
) -> Result<CatalogView, String> {
    blocking(move || {
        mutate(&path()?, |store| {
            let s = store
                .sources
                .iter_mut()
                .find(|s| s.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if s.revision != revision {
                return Err("CATALOG_CHANGED".into());
            }
            let n = s
                .catalog
                .nodes
                .iter_mut()
                .find(|n| n.id == node_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if !n.outbound.as_ref().is_some_and(|o| binding::is_insecure(o))
                || n.error.as_ref().is_some_and(|e| e != "PROXY_TLS_INSECURE")
            {
                return Err("CATALOG_INVALID".into());
            }
            n.error = if enabled {
                None
            } else {
                Some("PROXY_TLS_INSECURE".into())
            };
            revalidate_groups(&mut s.catalog)?;
            drop_invalid_default(s);
            s.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}

/// This is an explicit permission transaction, never an automatic repair. All
/// affected definitions are validated before any permission or revision changes.
pub async fn set_group_insecure(
    source_id: String,
    group_id: String,
    revision: String,
    enabled: bool,
) -> Result<CatalogView, String> {
    let guard = SourceGuard::new(source_id.clone())?;
    blocking(move || {
        let _guard = guard;
        mutate(&path()?, |store| {
            let source = store
                .sources
                .iter_mut()
                .find(|source| source.id == source_id)
                .ok_or("CATALOG_NOT_FOUND")?;
            if source.revision != revision {
                return Err("CATALOG_CHANGED".into());
            }
            let nodes = parser::reachable_nodes(&source.catalog, &group_id)?;
            let mut ids = HashSet::new();
            for node in nodes {
                if !node.outbound.as_ref().is_some_and(binding::is_insecure) {
                    continue;
                }
                if node
                    .error
                    .as_deref()
                    .is_some_and(|error| error != "PROXY_TLS_INSECURE")
                {
                    return Err("CATALOG_INVALID".into());
                }
                ids.insert(node.id.clone());
            }
            if ids.is_empty() {
                return Err("CATALOG_INVALID".into());
            }
            for node in &mut source.catalog.nodes {
                if ids.contains(&node.id) {
                    node.error = (!enabled).then(|| "PROXY_TLS_INSECURE".into());
                }
            }
            revalidate_groups(&mut source.catalog)?;
            drop_invalid_default(source);
            source.revision = uuid::Uuid::new_v4().to_string();
            Ok(view(store))
        })
    })
    .await
}
pub async fn snapshot(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
) -> Result<String, String> {
    snapshot_with_group(source_id, item_id, selections, None).await
}

pub async fn snapshot_with_group(
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
    group_id: Option<String>,
) -> Result<String, String> {
    let s = source(&source_id).await?;
    blocking(move || {
        binding::with_network(
            &binding::encode_with_group(
                &s.id,
                &s.name,
                &item_id,
                &s.catalog,
                &selections,
                group_id.as_deref(),
            )?,
            s.network,
        )
    })
    .await
}
pub async fn probe(
    request_id: String,
    source_id: String,
    item_id: String,
    selections: BTreeMap<String, String>,
) -> Result<super::codex_proxy_probe::ProxyProbeResult, String> {
    let op = Operation::begin(request_id)?;
    cancellable(&op, async {
        let input = snapshot(source_id, item_id, selections).await?;
        super::codex_proxy_probe::probe_resource(input).await
    })
    .await
}
/// Independent maintenance; only explicitly opted-in sources may contact the network.
fn due_sources(sources: &[Source], now: i64) -> Vec<String> {
    sources
        .iter()
        .filter(|s| {
            // 策略来源没有 URL，天然不参与订阅自动刷新；这里再按来源类型确认一次。
            s.kind != strategy::KIND
                && s.auto_update
                && s.url.is_some()
                && now - s.last_attempt_at.unwrap_or(s.updated_at).max(s.updated_at)
                    >= REFRESH_INTERVAL_MS
        })
        .map(|s| s.id.clone())
        .collect()
}
pub async fn auto_refresh_loop() {
    if AUTO_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let due = blocking(|| {
            let now = chrono::Utc::now().timestamp_millis();
            Ok(due_sources(&read(&path()?)?.sources, now))
        })
        .await;
        if let Ok(ids) = due {
            for id in ids {
                let source_id = id.clone();
                if refresh(uuid::Uuid::new_v4().to_string(), id).await.is_ok() {
                    // 统一代理引用同一来源时必须同步快照，否则自动刷新后账号仍走旧节点。
                    super::codex_unified_proxy::resync_source(&source_id, false).await;
                }
                tokio::task::yield_now().await;
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_proxy_catalog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codex_proxy_catalog_latency_tests.rs"]
mod latency_tests;
