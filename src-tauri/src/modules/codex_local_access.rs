use crate::models::codex::{CodexAccount, CodexApiProviderMode};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountCooldown, CodexLocalAccessAccountHealth, CodexLocalAccessAccountStats,
    CodexLocalAccessApiKey, CodexLocalAccessApiKeyStats, CodexLocalAccessCollection,
    CodexLocalAccessCustomRoutingRule, CodexLocalAccessGatewayMode,
    CodexLocalAccessImageGenerationMode, CodexLocalAccessImageGenerationStatus,
    CodexLocalAccessModelAlias, CodexLocalAccessModelPricing, CodexLocalAccessModelStats,
    CodexLocalAccessPortCleanupResult, CodexLocalAccessProfileAttachment,
    CodexLocalAccessRequestKind, CodexLocalAccessRoutingStrategy, CodexLocalAccessScope,
    CodexLocalAccessState, CodexLocalAccessStats, CodexLocalAccessStatsWindow,
    CodexLocalAccessTestFailure, CodexLocalAccessTestResult, CodexLocalAccessTimeoutPreset,
    CodexLocalAccessTimeouts, CodexLocalAccessUsageEvent, CodexLocalAccessUsageEventPage,
    CodexLocalAccessUsageStats,
};
use crate::modules::atomic_write::write_string_atomic;
use crate::modules::{
    account, codex_account, codex_oauth, codex_protocol, codex_wakeup, logger, process,
};
use base64::{engine::general_purpose, Engine as _};
use futures_util::{SinkExt, StreamExt};
use rand::{distributions::Alphanumeric, Rng};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, Method, Proxy, StatusCode, Url};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, Error as SqliteError,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::{oneshot, watch, Mutex as TokioMutex};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsClientRequest;
use tokio_tungstenite::tungstenite::http::header::{
    HeaderName as WsHeaderName, HeaderValue as WsHeaderValue,
};
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async_tls_with_config, MaybeTlsStream, WebSocketStream};
use toml_edit::Document;

const CODEX_LOCAL_ACCESS_FILE: &str = "codex_local_access.json";
const CODEX_LOCAL_ACCESS_STATS_FILE: &str = "codex_local_access_stats.json";
const CODEX_LOCAL_ACCESS_LOGS_DB_FILE: &str = "codex_local_access_logs.sqlite";
const CODEX_LOCAL_ACCESS_TAKEOVER_BACKUPS_FILE: &str = "codex_local_access_takeover_backups.json";
const CODEX_LOCAL_ACCESS_SIDECAR_DIR: &str = "codex_local_access_sidecar";
const CODEX_LOCAL_ACCESS_SIDECAR_CONFIG_FILE: &str = "config.json";
const CODEX_LOCAL_ACCESS_SIDECAR_MANIFEST_FILE: &str = "manifest.json";
const CODEX_LOCAL_ACCESS_SIDECAR_AUTHS_DIR: &str = "auths";
const CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME: &str = "cockpit-cliproxy";
const CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST: &str = "127.0.0.1";
const CODEX_LOCAL_ACCESS_LAN_BIND_HOST: &str = "0.0.0.0";
const CODEX_LOCAL_ACCESS_URL_HOST: &str = "localhost";
const CODEX_LOCAL_ACCESS_API_PORT_ENV: &str = "COCKPIT_TOOLS_API_PORT";
const CODEX_LOCAL_ACCESS_DEV_DEFAULT_PORT: u16 = 1456;
const CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION: u32 = 1;
const CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID: &str = "codex_local_access";
const CODEX_PROFILE_AUTH_FILE: &str = "auth.json";
const CODEX_PROFILE_CONFIG_FILE: &str = "config.toml";
const MAX_HTTP_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REQUEST_RETRY_ATTEMPTS: usize = 1;
const DEFAULT_UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT: Duration = Duration::from_secs(180);
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RETRY_CREDENTIALS_PER_REQUEST: usize = 8;
const SESSION_AFFINITY_TTL_MIN_MS: i64 = 60 * 1000;
const SESSION_AFFINITY_TTL_MAX_MS: i64 = 24 * 60 * 60 * 1000;
const DEFAULT_SESSION_AFFINITY_TTL_MS: i64 = 60 * 60 * 1000;
const MAX_RETRY_INTERVAL_MIN_MS: u64 = 0;
const MAX_RETRY_INTERVAL_MAX_MS: u64 = 30 * 1000;
const DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 3 * 1000;
const LOCAL_ACCESS_TIMEOUT_MIN_MS: u64 = 1_000;
const LOCAL_ACCESS_TIMEOUT_MAX_MS: u64 = 600_000;
const LEGACY_STREAM_TOTAL_TIMEOUT_MAX_MS: u64 = 30 * 60 * 1000;
const SIDECAR_STREAM_OPEN_ATTEMPTS_MIN: u8 = 1;
const SIDECAR_STREAM_OPEN_ATTEMPTS_MAX: u8 = 3;
const SIDECAR_STREAM_KEEPALIVE_MIN_SECONDS: u16 = 0;
const SIDECAR_STREAM_KEEPALIVE_MAX_SECONDS: u16 = 300;
const LOCAL_ACCESS_RETRY_ATTEMPTS_MIN: u8 = 0;
const LOCAL_ACCESS_RETRY_ATTEMPTS_MAX: u8 = 5;
const LOCAL_ACCESS_RETRY_DELAY_MIN_MS: u64 = 50;
const LOCAL_ACCESS_RETRY_DELAY_MAX_MS: u64 = 10 * 1000;
const WEBSOCKET_IDLE_TIMEOUT_MAX_MS: u64 = 30 * 60 * 1000;
const BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID: &str = "long_wait";
const BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID: &str = "short_wait";
const MAX_CUSTOM_TIMEOUT_PRESETS: usize = 20;
const TIMEOUT_PRESET_NAME_MAX_CHARS: usize = 40;
const RESPONSE_AFFINITY_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_RESPONSE_AFFINITY_BINDINGS: usize = 4096;
const PREPARED_ACCOUNT_CACHE_TTL_MS: i64 = 30 * 1000;
const DAY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_WINDOW_MS: i64 = 7 * DAY_WINDOW_MS;
const MONTH_WINDOW_MS: i64 = 30 * DAY_WINDOW_MS;
const STATE_RECENT_USAGE_EVENT_LIMIT: usize = 100;
const COOLDOWN_KEY_SEPARATOR: &str = "\u{1f}";
const CUSTOM_ROUTING_PRIORITY_MIN: i32 = 0;
const CUSTOM_ROUTING_PRIORITY_MAX: i32 = 100;
const CUSTOM_ROUTING_WEIGHT_MIN: u32 = 1;
const CUSTOM_ROUTING_WEIGHT_MAX: u32 = 100;
const GATEWAY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const GATEWAY_PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const GATEWAY_PORT_RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SIDECAR_READY_TIMEOUT: Duration = Duration::from_secs(8);
const UPSTREAM_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const DEFAULT_OPENAI_RESPONSES_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_CODEX_USER_AGENT: &str =
    "codex_cli_rs/0.118.0 (Mac OS 26.3.1; arm64) iTerm.app/3.6.9";
const DEFAULT_CODEX_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE: &str = "responses_websockets=2026-02-06";
const CODEX_WEBSOCKET_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_WEBSOCKET_INITIAL_MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const CODEX_WEBSOCKET_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
#[cfg(test)]
const CODEX_WEBSOCKET_HEARTBEAT_INTERVAL: Duration = Duration::from_millis(25);
const CODEX_WEBSOCKET_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const CODEX_WEBSOCKET_PROXY_CONNECT_MAX_BYTES: usize = 16 * 1024;
const CORS_ALLOW_HEADERS: &str = "Authorization, Content-Type, OpenAI-Beta, X-API-Key, X-Codex-Beta-Features, X-Codex-Turn-State, X-Codex-Turn-Metadata, X-Client-Request-Id, X-ResponsesAPI-Include-Timing-Metrics, Version, Originator, Session_id, Conversation_id, ChatGPT-Account-Id";
const CODEX_OFFICIAL_EMPTY_HEADERS: &[&str] = &[
    "version",
    "x-codex-turn-state",
    "x-codex-turn-metadata",
    "x-client-request-id",
    "x-responsesapi-include-timing-metrics",
];
const DEFAULT_CODEX_MODELS: &[&str] = &[
    "gpt-5-codex",
    "gpt-5-codex-mini",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5.2",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
];
const CODEX_IMAGE_MODEL_ID: &str = "gpt-image-2";
const CODEX_AUTO_REVIEW_MODEL_ID: &str = "codex-auto-review";
const DEFAULT_IMAGES_MAIN_MODEL: &str = "gpt-5.4-mini";
const MAX_MODEL_PRICE_USD_PER_MILLION: f64 = 1_000_000.0;
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const RESPONSES_PATH: &str = "/v1/responses";
const RESPONSES_COMPACT_PATH: &str = "/v1/responses/compact";
const BACKEND_CODEX_PREFIX: &str = "/backend-api/codex";
const BACKEND_CODEX_RESPONSES_PATH: &str = "/backend-api/codex/responses";
const BACKEND_CODEX_RESPONSES_COMPACT_PATH: &str = "/backend-api/codex/responses/compact";
const IMAGES_GENERATIONS_PATH: &str = "/v1/images/generations";
const IMAGES_EDITS_PATH: &str = "/v1/images/edits";
static GATEWAY_RUNTIME: OnceLock<TokioMutex<GatewayRuntime>> = OnceLock::new();
static GATEWAY_LIFECYCLE_LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
static GATEWAY_ROUND_ROBIN_CURSOR: AtomicUsize = AtomicUsize::new(0);
static UPSTREAM_HTTP_CLIENT: OnceLock<Mutex<Option<CachedUpstreamHttpClient>>> = OnceLock::new();

#[derive(Default)]
struct GatewayRuntime {
    loaded: bool,
    collection: Option<CodexLocalAccessCollection>,
    stats: CodexLocalAccessStats,
    stats_dirty: bool,
    stats_flush_inflight: bool,
    response_affinity: HashMap<String, ResponseAffinityBinding>,
    model_cooldowns: HashMap<String, AccountModelCooldown>,
    account_health: HashMap<String, RuntimeAccountHealth>,
    prepared_accounts: HashMap<String, CachedPreparedAccount>,
    running: bool,
    actual_port: Option<u16>,
    actual_bind_host: Option<String>,
    sidecar_config_fingerprint: Option<String>,
    last_error: Option<String>,
    shutdown_sender: Option<watch::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<()>>,
    sidecar_child: Option<Child>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CodexLocalAccessTakeoverBackups {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    profiles: Vec<CodexLocalAccessProfileTakeoverBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexLocalAccessProfileTakeoverBackup {
    profile_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_toml: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Clone)]
struct GatewayBindEndpoint {
    bind_host: String,
    port: u16,
}

#[derive(Debug, Clone, Default)]
struct UsageCapture {
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
    cached_tokens: u64,
    reasoning_tokens: u64,
}

#[derive(Debug, Clone, Default)]
struct ResponseCapture {
    usage: Option<UsageCapture>,
    response_id: Option<String>,
    terminal_error: Option<String>,
}

#[derive(Debug, Clone)]
struct UpstreamResponseFailedSignal {
    event_type: String,
    code: Option<String>,
    error_type: Option<String>,
    message: Option<String>,
    raw: String,
}

#[derive(Debug, Clone)]
struct WebSocketUpstreamError {
    status: u16,
    body: String,
    category: String,
    retry_after: Option<Duration>,
}

#[derive(Debug, Clone)]
struct WebSocketConnectError {
    status: Option<u16>,
    message: String,
    category: String,
}

#[derive(Debug, Clone, Default)]
struct WebSocketBridgeResult {
    capture: ResponseCapture,
    upstream_error: Option<WebSocketUpstreamError>,
}

#[derive(Debug, Clone, Default)]
struct ImageCallResult {
    result: String,
    revised_prompt: String,
    output_format: String,
    size: String,
    background: String,
    quality: String,
}

#[derive(Debug, Clone)]
struct MultipartFilePart {
    name: String,
    content_type: String,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
struct MultipartFormData {
    fields: HashMap<String, String>,
    files: Vec<MultipartFilePart>,
}

#[derive(Debug, Clone)]
struct ResponseAffinityBinding {
    account_id: String,
    updated_at_ms: i64,
}

#[derive(Debug, Clone)]
struct AccountModelCooldown {
    model_key: String,
    next_retry_at_ms: i64,
    reason: String,
}

#[derive(Debug, Clone, Default)]
struct RuntimeAccountHealth {
    email: String,
    consecutive_failures: u32,
    last_success_at: Option<i64>,
    last_failure_at: Option<i64>,
    last_failure_status: Option<u16>,
    last_failure_category: Option<String>,
    last_failure_message: Option<String>,
    image_generation_status: CodexLocalAccessImageGenerationStatus,
    image_generation_checked_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct CachedPreparedAccount {
    account: CodexAccount,
    cached_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpstreamHttpClientSignature {
    proxy_source: UpstreamProxySource,
    proxy_url: Option<String>,
    connect_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamProxySource {
    ApiService,
    Global,
    SystemEnv,
    SystemAuto,
}

#[derive(Debug, Clone)]
struct UpstreamProxyDiagnostics {
    proxy_source: UpstreamProxySource,
    proxy_url: Option<String>,
}

#[derive(Clone)]
struct CachedUpstreamHttpClient {
    signature: UpstreamHttpClientSignature,
    client: Client,
}

#[derive(Debug)]
struct ProxyDispatchSuccess {
    upstream: reqwest::Response,
    account_id: String,
    account_email: String,
}

#[derive(Debug)]
struct ProxyDispatchError {
    status: u16,
    message: String,
    account_id: Option<String>,
    account_email: Option<String>,
    error_category: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedLocalApiKey {
    id: String,
    label: String,
    model_prefix: Option<String>,
    allowed_models: Vec<String>,
    excluded_models: Vec<String>,
}

#[derive(Debug, Clone)]
struct RequestStatsContext {
    request_kind: CodexLocalAccessRequestKind,
    model_id: String,
    api_key_id: String,
    api_key_label: String,
}

struct ResponseUsageCollector {
    is_stream: bool,
    body: Vec<u8>,
    stream_buffer: Vec<u8>,
    usage: Option<UsageCapture>,
    response_id: Option<String>,
    terminal_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
enum GatewayResponseAdapter {
    Passthrough {
        request_is_stream: bool,
    },
    ChatCompletions {
        stream: bool,
        requested_model: String,
        original_request_body: Vec<u8>,
    },
    Images {
        stream: bool,
        response_format: String,
        stream_prefix: String,
    },
}

#[derive(Debug, Clone, Default)]
struct RequestRoutingHint {
    model_key: String,
    previous_response_id: Option<String>,
    session_affinity_key: Option<String>,
}

#[derive(Debug)]
struct WebSocketDispatchSuccess {
    upstream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    account: CodexAccount,
    account_id: String,
    account_email: String,
}

#[derive(Debug, Clone)]
struct RoutingCandidate {
    account_id: String,
    plan_rank: Option<i32>,
    remaining_quota: Option<i32>,
    subscription_expiry_ms: Option<i64>,
}

fn gateway_runtime() -> &'static TokioMutex<GatewayRuntime> {
    GATEWAY_RUNTIME.get_or_init(|| TokioMutex::new(GatewayRuntime::default()))
}

fn gateway_lifecycle_lock() -> &'static TokioMutex<()> {
    GATEWAY_LIFECYCLE_LOCK.get_or_init(|| TokioMutex::new(()))
}

fn upstream_http_client_cache() -> &'static Mutex<Option<CachedUpstreamHttpClient>> {
    UPSTREAM_HTTP_CLIENT.get_or_init(|| Mutex::new(None))
}

fn duration_to_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn duration_from_millis(value: u64, fallback: Duration) -> Duration {
    if value == 0 {
        return fallback;
    }
    Duration::from_millis(value)
}

fn upstream_env_proxy_url() -> Option<String> {
    const ENV_PROXY_KEYS: [&str; 6] = [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];

    for key in ENV_PROXY_KEYS {
        if let Ok(value) = std::env::var(key) {
            let proxy_url = value.trim();
            if !proxy_url.is_empty() {
                return Some(proxy_url.to_string());
            }
        }
    }

    None
}

fn current_upstream_http_client_signature(
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> UpstreamHttpClientSignature {
    let connect_timeout_ms = duration_to_millis(connect_timeout);
    if let Some(proxy_url) = upstream_proxy_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return UpstreamHttpClientSignature {
            proxy_source: UpstreamProxySource::ApiService,
            proxy_url: Some(proxy_url.to_string()),
            connect_timeout_ms,
        };
    }

    let config = crate::modules::config::get_user_config();
    if config.global_proxy_enabled {
        let proxy_url = config.global_proxy_url.trim();
        if !proxy_url.is_empty() {
            return UpstreamHttpClientSignature {
                proxy_source: UpstreamProxySource::Global,
                proxy_url: Some(proxy_url.to_string()),
                connect_timeout_ms,
            };
        }
    }

    if let Some(proxy_url) = upstream_env_proxy_url() {
        return UpstreamHttpClientSignature {
            proxy_source: UpstreamProxySource::SystemEnv,
            proxy_url: Some(proxy_url),
            connect_timeout_ms,
        };
    }

    UpstreamHttpClientSignature {
        proxy_source: UpstreamProxySource::SystemAuto,
        proxy_url: None,
        connect_timeout_ms,
    }
}

fn redact_proxy_url_for_log(proxy_url: &str) -> String {
    match Url::parse(proxy_url) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _ = url.set_username("redacted");
            }
            if url.password().is_some() {
                let _ = url.set_password(Some("redacted"));
            }
            url.to_string()
        }
        Err(_) => "<invalid>".to_string(),
    }
}

fn current_upstream_proxy_diagnostics(
    upstream_proxy_url: Option<&str>,
) -> UpstreamProxyDiagnostics {
    let signature = current_upstream_http_client_signature(
        upstream_proxy_url,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    UpstreamProxyDiagnostics {
        proxy_source: signature.proxy_source,
        proxy_url: signature.proxy_url.as_deref().map(redact_proxy_url_for_log),
    }
}

fn build_upstream_http_client(signature: &UpstreamHttpClientSignature) -> Result<Client, String> {
    let mut builder = Client::builder().connect_timeout(duration_from_millis(
        signature.connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    ));

    if let Some(proxy_url) = signature.proxy_url.as_deref() {
        let proxy = Proxy::all(proxy_url).map_err(|e| format!("Codex 上游代理地址无效: {}", e))?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| format!("创建 Codex 上游 HTTP 客户端失败: {}", e))
}

fn build_localhost_http_client(request_timeout: Duration, label: &str) -> Result<Client, String> {
    Client::builder()
        .no_proxy()
        .timeout(request_timeout)
        .build()
        .map_err(|e| format!("创建{}客户端失败: {}", label, e))
}

fn log_upstream_http_client_signature(signature: &UpstreamHttpClientSignature) {
    match (signature.proxy_source, signature.proxy_url.as_deref()) {
        (UpstreamProxySource::ApiService, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已应用 API 服务代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::Global, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已跟随全局代理 proxy_url={}，API 服务上游请求不应用 no_proxy 绕过",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemEnv, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][legacy] 上游 HTTP 客户端已使用环境代理 proxy_url={}，API 服务上游请求不应用 no_proxy 绕过",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemAuto, None) => logger::log_info(
            "[CodexLocalAccess][legacy] 未配置 API 服务代理、全局代理或环境代理，已回退到 reqwest 系统自动代理配置",
        ),
        _ => logger::log_warn("[CodexLocalAccess][legacy] 上游 HTTP 客户端代理状态异常"),
    }
}

fn log_sidecar_proxy_signature(signature: &UpstreamHttpClientSignature) {
    match (signature.proxy_source, signature.proxy_url.as_deref()) {
        (UpstreamProxySource::ApiService, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则应用 API 服务代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::Global, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则跟随全局代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemEnv, Some(proxy_url)) => logger::log_info(&format!(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则使用环境代理 proxy_url={}",
            redact_proxy_url_for_log(proxy_url)
        )),
        (UpstreamProxySource::SystemAuto, None) => logger::log_info(
            "[CodexLocalAccess][sidecar] 上游代理已按旧网关规则回退到系统自动代理配置",
        ),
        _ => logger::log_warn("[CodexLocalAccess][sidecar] 上游代理状态异常"),
    }
}

fn upstream_http_client(
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<Client, String> {
    let signature = current_upstream_http_client_signature(upstream_proxy_url, connect_timeout);
    let mut cache = upstream_http_client_cache()
        .lock()
        .map_err(|_| "Codex 上游 HTTP 客户端缓存已损坏".to_string())?;

    if let Some(cached) = cache.as_ref() {
        if cached.signature == signature {
            return Ok(cached.client.clone());
        }
    }

    let client = build_upstream_http_client(&signature)?;
    log_upstream_http_client_signature(&signature);
    *cache = Some(CachedUpstreamHttpClient {
        signature,
        client: client.clone(),
    });
    Ok(client)
}

fn local_access_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_FILE))
}

fn local_access_stats_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_STATS_FILE))
}

fn local_access_logs_db_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_LOGS_DB_FILE))
}

fn local_access_takeover_backups_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_TAKEOVER_BACKUPS_FILE))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn is_prepared_account_cache_valid(entry: &CachedPreparedAccount, now: i64) -> bool {
    now.saturating_sub(entry.cached_at_ms) <= PREPARED_ACCOUNT_CACHE_TTL_MS
        && !codex_oauth::is_token_expired(&entry.account.tokens.access_token)
}

fn account_has_refresh_token(account: &CodexAccount) -> bool {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .is_some()
}

fn prune_prepared_account_cache(runtime: &mut GatewayRuntime, now: i64) {
    let allowed_account_ids = runtime.collection.as_ref().map(|collection| {
        collection
            .account_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<&str>>()
    });

    runtime.prepared_accounts.retain(|account_id, entry| {
        let in_collection = allowed_account_ids
            .as_ref()
            .map(|ids| ids.contains(account_id.as_str()))
            .unwrap_or(true);
        in_collection && is_prepared_account_cache_valid(entry, now)
    });
}

fn sync_runtime_collection(runtime: &mut GatewayRuntime, collection: CodexLocalAccessCollection) {
    runtime.collection = Some(collection);
    runtime.loaded = true;
    runtime.last_error = None;
    prune_prepared_account_cache(runtime, now_ms());
}

fn normalize_optional_account_ref(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn validate_local_access_bound_oauth_account(
    bound_oauth_account_id: &str,
) -> Result<CodexAccount, String> {
    let bound_id = normalize_optional_account_ref(Some(bound_oauth_account_id))
        .ok_or_else(|| "请选择要绑定的 OAuth 账号".to_string())?;
    let oauth_account = codex_account::load_account(&bound_id)
        .ok_or_else(|| format!("绑定的 OAuth 账号不存在: {}", bound_id))?;
    if oauth_account.is_api_key_auth() {
        return Err("API 服务只能绑定 OAuth 账号，不能绑定 API Key 账号".to_string());
    }
    Ok(oauth_account)
}

async fn cache_prepared_account(account: &CodexAccount) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_prepared_account_cache(&mut runtime, now);
    runtime.prepared_accounts.insert(
        account.id.clone(),
        CachedPreparedAccount {
            account: account.clone(),
            cached_at_ms: now,
        },
    );
}

async fn invalidate_prepared_account(account_id: &str) {
    let mut runtime = gateway_runtime().lock().await;
    runtime.prepared_accounts.remove(account_id);
}

fn try_get_cached_account_for_routing(account_id: &str) -> Option<CodexAccount> {
    let Ok(mut runtime) = gateway_runtime().try_lock() else {
        return None;
    };
    let now = now_ms();
    prune_prepared_account_cache(&mut runtime, now);
    runtime
        .prepared_accounts
        .get(account_id)
        .filter(|entry| is_prepared_account_cache_valid(entry, now))
        .map(|entry| entry.account.clone())
}

async fn get_prepared_account(account_id: &str) -> Result<CodexAccount, String> {
    {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        prune_prepared_account_cache(&mut runtime, now);
        if let Some(entry) = runtime.prepared_accounts.get(account_id) {
            if is_prepared_account_cache_valid(entry, now) {
                return Ok(entry.account.clone());
            }
        }
    }

    let account = codex_account::prepare_account_for_injection(account_id).await?;
    cache_prepared_account(&account).await;
    Ok(account)
}

async fn schedule_stats_flush_if_needed() {
    let should_spawn = {
        let mut runtime = gateway_runtime().lock().await;
        if runtime.stats_flush_inflight {
            false
        } else {
            runtime.stats_flush_inflight = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(STATS_FLUSH_INTERVAL).await;

            let stats_snapshot = {
                let mut runtime = gateway_runtime().lock().await;
                if !runtime.stats_dirty {
                    runtime.stats_flush_inflight = false;
                    return;
                }
                runtime.stats_dirty = false;
                runtime.stats.clone()
            };

            if let Err(err) = save_stats_to_disk(&stats_snapshot) {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 后台写入请求统计失败: {}",
                    err
                ));
                let mut runtime = gateway_runtime().lock().await;
                runtime.stats_dirty = true;
                runtime.stats_flush_inflight = false;
                return;
            }
        }
    });
}

fn normalize_model_key(model: &str) -> String {
    model.trim().to_ascii_lowercase()
}

fn has_date_snapshot_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 11
        && bytes[0] == b'-'
        && bytes[5] == b'-'
        && bytes[8] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 0 | 5 | 8) || byte.is_ascii_digit())
}

fn supported_codex_model_ids() -> Vec<String> {
    let mut seen = HashSet::new();
    let mut model_ids: Vec<String> = codex_wakeup::load_state_for_scheduler()
        .ok()
        .map(|state| {
            state
                .model_presets
                .into_iter()
                .map(|preset| preset.model.trim().to_string())
                .filter(|model| !model.is_empty())
                .filter(|model| seen.insert(model.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let mut seen_model_ids: HashSet<String> = model_ids
        .iter()
        .map(|model| model.trim().to_ascii_lowercase())
        .filter(|model| !model.is_empty())
        .collect();
    for model in DEFAULT_CODEX_MODELS {
        if seen_model_ids.insert((*model).to_ascii_lowercase()) {
            model_ids.push((*model).to_string());
        }
    }
    if seen_model_ids.insert(CODEX_IMAGE_MODEL_ID.to_string()) {
        model_ids.push(CODEX_IMAGE_MODEL_ID.to_string());
    }
    if seen_model_ids.insert(CODEX_AUTO_REVIEW_MODEL_ID.to_string()) {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }

    model_ids
}

fn account_health_allows_image_generation(health: Option<&RuntimeAccountHealth>) -> bool {
    !matches!(
        health.map(|item| item.image_generation_status),
        Some(
            CodexLocalAccessImageGenerationStatus::Unavailable
                | CodexLocalAccessImageGenerationStatus::Disabled
        )
    )
}

fn selected_accounts_have_image_generation_capacity(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> bool {
    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return false;
    }
    let Ok(accounts) = codex_account::list_accounts_checked() else {
        return true;
    };
    let selected: HashSet<&str> = collection.account_ids.iter().map(String::as_str).collect();
    accounts.into_iter().any(|account| {
        selected.contains(account.id.as_str())
            && !account.is_api_key_auth()
            && !is_free_plan_type(account.plan_type.as_deref())
            && account_health_allows_image_generation(
                health_by_account_id.and_then(|health| health.get(account.id.as_str())),
            )
    })
}

fn base_codex_model_ids_for_collection(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let image_allowed =
        selected_accounts_have_image_generation_capacity(collection, health_by_account_id);
    supported_codex_model_ids()
        .into_iter()
        .filter(|model| model != CODEX_IMAGE_MODEL_ID || image_allowed)
        .collect()
}

fn normalize_model_rule_value(value: &str) -> String {
    value.trim().to_string()
}

fn normalize_model_rule_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| normalize_model_rule_value(&value))
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn normalize_model_prefix_value(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().trim_matches('/').trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
}

fn normalize_model_aliases(
    values: Vec<CodexLocalAccessModelAlias>,
) -> Vec<CodexLocalAccessModelAlias> {
    let mut seen_aliases = HashSet::new();
    values
        .into_iter()
        .filter_map(|item| {
            let source_model = normalize_model_rule_value(&item.source_model);
            let alias = normalize_model_rule_value(&item.alias);
            if source_model.is_empty() || alias.is_empty() {
                return None;
            }
            let alias_key = alias.to_ascii_lowercase();
            if source_model.eq_ignore_ascii_case(&alias) || !seen_aliases.insert(alias_key) {
                return None;
            }
            Some(CodexLocalAccessModelAlias {
                source_model,
                alias,
                fork: item.fork,
            })
        })
        .collect()
}

fn wildcard_model_matches(pattern: &str, model: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let model = model.trim().to_ascii_lowercase();
    if pattern.is_empty() || model.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == model;
    }

    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');
    let parts: Vec<&str> = pattern.split('*').filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return true;
    }

    let mut remaining = model.as_str();
    for (index, part) in parts.iter().enumerate() {
        let Some(found) = remaining.find(part) else {
            return false;
        };
        if index == 0 && anchored_start && found != 0 {
            return false;
        }
        let next_start = found + part.len();
        remaining = &remaining[next_start..];
    }

    if anchored_end {
        if let Some(last) = parts.last() {
            return model.ends_with(last);
        }
    }
    true
}

fn model_matches_any_rule(model: &str, rules: &[String]) -> bool {
    rules.iter().any(|rule| wildcard_model_matches(rule, model))
}

fn apply_model_aliases_to_ids(
    model_ids: Vec<String>,
    aliases: &[CodexLocalAccessModelAlias],
) -> Vec<String> {
    if aliases.is_empty() {
        return model_ids;
    }

    let alias_map: HashMap<String, &CodexLocalAccessModelAlias> = aliases
        .iter()
        .map(|alias| (alias.source_model.to_ascii_lowercase(), alias))
        .collect();
    let mut seen = HashSet::new();
    let mut visible = Vec::new();

    for model in model_ids {
        let key = model.to_ascii_lowercase();
        if let Some(alias) = alias_map.get(&key) {
            if alias.fork && seen.insert(key) {
                visible.push(model.clone());
            }
            if seen.insert(alias.alias.to_ascii_lowercase()) {
                visible.push(alias.alias.clone());
            }
        } else if seen.insert(key) {
            visible.push(model);
        }
    }

    visible
}

fn apply_model_filters(
    model_ids: Vec<String>,
    allowed: &[String],
    excluded: &[String],
) -> Vec<String> {
    model_ids
        .into_iter()
        .filter(|model| allowed.is_empty() || model_matches_any_rule(model, allowed))
        .filter(|model| !model_matches_any_rule(model, excluded))
        .collect()
}

fn strip_model_prefix<'a>(model: &'a str, prefix: Option<&str>) -> &'a str {
    let Some(prefix) = prefix.map(str::trim).filter(|item| !item.is_empty()) else {
        return model.trim();
    };
    let trimmed = model.trim();
    let expected = format!("{}/", prefix.trim_matches('/'));
    trimmed
        .strip_prefix(expected.as_str())
        .map(str::trim)
        .unwrap_or(trimmed)
}

fn add_model_prefix(model_ids: Vec<String>, prefix: Option<&str>) -> Vec<String> {
    let Some(prefix) = prefix.map(str::trim).filter(|item| !item.is_empty()) else {
        return model_ids;
    };
    model_ids
        .into_iter()
        .map(|model| format!("{}/{}", prefix.trim_matches('/'), model))
        .collect()
}

fn visible_codex_model_ids_for_collection(
    collection: &CodexLocalAccessCollection,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let base = base_codex_model_ids_for_collection(collection, health_by_account_id);
    let aliased = apply_model_aliases_to_ids(base, &collection.model_aliases);
    apply_model_filters(aliased, &[], &collection.excluded_models)
}

fn visible_codex_model_ids_for_api_key(
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Vec<String> {
    let visible = visible_codex_model_ids_for_collection(collection, health_by_account_id);
    let filtered = apply_model_filters(visible, &api_key.allowed_models, &api_key.excluded_models);
    append_codex_internal_model_ids(add_model_prefix(filtered, api_key.model_prefix.as_deref()))
}

fn is_codex_internal_model(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID)
}

fn append_codex_internal_model_ids(mut model_ids: Vec<String>) -> Vec<String> {
    if !model_ids.iter().any(|model| is_codex_internal_model(model)) {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }
    model_ids
}

fn canonical_model_for_client_model(
    model: &str,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
) -> String {
    let without_prefix = strip_model_prefix(model, api_key.model_prefix.as_deref());
    if is_codex_internal_model(without_prefix) {
        return CODEX_AUTO_REVIEW_MODEL_ID.to_string();
    }
    for alias in &collection.model_aliases {
        if alias.alias.eq_ignore_ascii_case(without_prefix) {
            return alias.source_model.clone();
        }
    }
    resolve_supported_model_alias(without_prefix)
}

fn validate_client_model_visible(
    model: &str,
    canonical_model: &str,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> bool {
    let without_prefix = strip_model_prefix(model, api_key.model_prefix.as_deref());
    if is_codex_internal_model(without_prefix) || is_codex_internal_model(canonical_model) {
        return true;
    }
    let visible = visible_codex_model_ids_for_collection(collection, health_by_account_id);
    let visible_match = visible.iter().any(|item| {
        item.eq_ignore_ascii_case(without_prefix)
            || item.eq_ignore_ascii_case(canonical_model)
            || resolve_supported_model_alias(item).eq_ignore_ascii_case(canonical_model)
    });
    if !visible_match {
        return false;
    }
    if !api_key.allowed_models.is_empty()
        && !model_matches_any_rule(without_prefix, &api_key.allowed_models)
        && !model_matches_any_rule(canonical_model, &api_key.allowed_models)
    {
        return false;
    }
    !model_matches_any_rule(without_prefix, &api_key.excluded_models)
        && !model_matches_any_rule(canonical_model, &api_key.excluded_models)
}

fn rewrite_request_model_for_access_policy_value(
    body_value: &mut Value,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Result<bool, String> {
    let Some(body_obj) = body_value.as_object_mut() else {
        return Ok(false);
    };
    let Some(model) = body_obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return Ok(false);
    };

    let canonical_model = canonical_model_for_client_model(&model, collection, api_key);
    if !validate_client_model_visible(
        &model,
        &canonical_model,
        collection,
        api_key,
        health_by_account_id,
    ) {
        return Err(format!(
            "模型 {} 不在当前 API Key 的可用模型范围内",
            model.trim()
        ));
    }

    if canonical_model == model {
        return Ok(false);
    }
    body_obj.insert("model".to_string(), Value::String(canonical_model));
    Ok(true)
}

fn rewrite_request_model_for_access_policy(
    request: &mut ParsedRequest,
    collection: &CodexLocalAccessCollection,
    api_key: &ResolvedLocalApiKey,
    health_by_account_id: Option<&HashMap<String, RuntimeAccountHealth>>,
) -> Result<(), String> {
    let Some(mut body_value) = parse_request_body_json(&request.body) else {
        return Ok(());
    };
    if !rewrite_request_model_for_access_policy_value(
        &mut body_value,
        collection,
        api_key,
        health_by_account_id,
    )? {
        return Ok(());
    }
    request.body = serde_json::to_vec(&body_value)
        .map_err(|e| format!("序列化模型访问规则后的请求体失败: {}", e))?;
    Ok(())
}

fn resolve_supported_model_alias(model: &str) -> String {
    let trimmed = model.trim();
    let normalized = trimmed.to_ascii_lowercase();

    for alias in supported_codex_model_ids() {
        if normalized == alias {
            return alias;
        }

        if let Some(suffix) = normalized.strip_prefix(&alias) {
            if has_date_snapshot_suffix(suffix) {
                return alias;
            }
        }
    }

    trimmed.to_string()
}

fn rewrite_request_model_alias(body: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let Some(mut body_value) = parse_request_body_json(body) else {
        return Ok(None);
    };

    if !rewrite_request_model_alias_value(&mut body_value) {
        return Ok(None);
    }

    serde_json::to_vec(&body_value)
        .map(Some)
        .map_err(|e| format!("重写请求 model 失败: {}", e))
}

fn rewrite_request_model_alias_value(body_value: &mut Value) -> bool {
    let Some(body_obj) = body_value.as_object_mut() else {
        return false;
    };
    let Some(model) = body_obj.get("model").and_then(Value::as_str) else {
        return false;
    };

    let resolved_model = resolve_supported_model_alias(model);
    if resolved_model == model {
        return false;
    }

    body_obj.insert("model".to_string(), Value::String(resolved_model));
    true
}

fn parse_request_body_json(body: &[u8]) -> Option<Value> {
    if body.is_empty() {
        return None;
    }
    serde_json::from_slice::<Value>(body).ok()
}

fn proxy_target_path(target: &str) -> &str {
    target.split('?').next().unwrap_or(target).trim()
}

fn is_images_generations_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == IMAGES_GENERATIONS_PATH || path.ends_with("/images/generations")
}

fn is_images_edits_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == IMAGES_EDITS_PATH || path.ends_with("/images/edits")
}

fn is_responses_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == RESPONSES_PATH || path == BACKEND_CODEX_RESPONSES_PATH || path.ends_with("/responses")
}

fn is_responses_compact_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == RESPONSES_COMPACT_PATH
        || path == BACKEND_CODEX_RESPONSES_COMPACT_PATH
        || path.ends_with("/responses/compact")
}

fn is_backend_codex_request(target: &str) -> bool {
    let path = proxy_target_path(target);
    path == BACKEND_CODEX_PREFIX || path.starts_with(&format!("{}/", BACKEND_CODEX_PREFIX))
}

fn is_backend_codex_responses_websocket_request(target: &str) -> bool {
    proxy_target_path(target) == BACKEND_CODEX_RESPONSES_PATH
}

fn is_supported_proxy_target(target: &str) -> bool {
    target.starts_with("/v1/") || is_backend_codex_request(target)
}

fn request_kind_is_image(request_kind: CodexLocalAccessRequestKind) -> bool {
    matches!(
        request_kind,
        CodexLocalAccessRequestKind::ImageGeneration | CodexLocalAccessRequestKind::ImageEdit
    )
}

fn request_kind_from_adapter(adapter: &GatewayResponseAdapter) -> CodexLocalAccessRequestKind {
    match adapter {
        GatewayResponseAdapter::ChatCompletions { .. } => CodexLocalAccessRequestKind::Text,
        GatewayResponseAdapter::Images { stream_prefix, .. } => {
            if stream_prefix == "image_edit" {
                CodexLocalAccessRequestKind::ImageEdit
            } else {
                CodexLocalAccessRequestKind::ImageGeneration
            }
        }
        GatewayResponseAdapter::Passthrough { .. } => CodexLocalAccessRequestKind::Text,
    }
}

fn request_kind_from_target(target: &str) -> CodexLocalAccessRequestKind {
    if is_images_generations_request(target) {
        CodexLocalAccessRequestKind::ImageGeneration
    } else if is_images_edits_request(target) {
        CodexLocalAccessRequestKind::ImageEdit
    } else {
        CodexLocalAccessRequestKind::Text
    }
}

fn extract_request_model_id(body: &[u8]) -> Option<String> {
    parse_request_body_json(body)
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn stats_model_id_for_request_kind(
    body: &[u8],
    request_kind: CodexLocalAccessRequestKind,
) -> String {
    if request_kind_is_image(request_kind) {
        return extract_request_model_id(body).unwrap_or_else(|| CODEX_IMAGE_MODEL_ID.to_string());
    }
    extract_request_model_id(body).unwrap_or_default()
}

fn stats_model_id_from_adapter(
    request: &ParsedRequest,
    adapter: &GatewayResponseAdapter,
) -> String {
    match adapter {
        GatewayResponseAdapter::ChatCompletions {
            requested_model, ..
        } => requested_model.clone(),
        GatewayResponseAdapter::Images { .. } => CODEX_IMAGE_MODEL_ID.to_string(),
        GatewayResponseAdapter::Passthrough { .. } => {
            stats_model_id_for_request_kind(&request.body, request_kind_from_adapter(adapter))
        }
    }
}

fn build_request_stats_context(
    request: &ParsedRequest,
    adapter: &GatewayResponseAdapter,
    api_key: &ResolvedLocalApiKey,
) -> RequestStatsContext {
    let request_kind = request_kind_from_adapter(adapter);
    RequestStatsContext {
        request_kind,
        model_id: stats_model_id_from_adapter(request, adapter),
        api_key_id: api_key.id.clone(),
        api_key_label: api_key.label.clone(),
    }
}

fn normalize_image_model_base(model: &str) -> String {
    let mut base_model = model.trim();
    if let Some(index) = base_model.rfind('/') {
        if index < base_model.len().saturating_sub(1) {
            base_model = base_model[index + 1..].trim();
        }
    }
    base_model.to_string()
}

fn normalize_image_response_format(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or("b64_json")
        .to_ascii_lowercase()
}

fn validate_image_model(model: &str) -> Result<String, String> {
    let trimmed = model.trim();
    let base_model = normalize_image_model_base(trimmed);
    if base_model == CODEX_IMAGE_MODEL_ID {
        return Ok(CODEX_IMAGE_MODEL_ID.to_string());
    }

    Err(format!(
        "Model {} is not supported on {} or {}. Use {}.",
        if trimmed.is_empty() {
            "<empty>"
        } else {
            trimmed
        },
        IMAGES_GENERATIONS_PATH,
        IMAGES_EDITS_PATH,
        CODEX_IMAGE_MODEL_ID
    ))
}

fn json_string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn insert_json_string_field(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    key: &str,
) {
    if let Some(value) = json_string_field(source, key) {
        target.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn insert_json_number_field(
    target: &mut Map<String, Value>,
    source: &Map<String, Value>,
    key: &str,
) {
    if let Some(value) = source.get(key).filter(|item| item.is_number()) {
        target.insert(key.to_string(), value.clone());
    }
}

fn build_image_generation_tool(
    source: &Map<String, Value>,
    action: &str,
    include_edit_fields: bool,
) -> Result<Value, String> {
    let image_model = json_string_field(source, "model").unwrap_or(CODEX_IMAGE_MODEL_ID);
    let canonical_model = validate_image_model(image_model)?;

    let mut tool = Map::new();
    tool.insert(
        "type".to_string(),
        Value::String("image_generation".to_string()),
    );
    tool.insert("action".to_string(), Value::String(action.to_string()));
    tool.insert("model".to_string(), Value::String(canonical_model));

    for key in [
        "size",
        "quality",
        "background",
        "output_format",
        "moderation",
    ] {
        insert_json_string_field(&mut tool, source, key);
    }
    if include_edit_fields {
        insert_json_string_field(&mut tool, source, "input_fidelity");
    }
    for key in ["output_compression", "partial_images"] {
        insert_json_number_field(&mut tool, source, key);
    }

    Ok(Value::Object(tool))
}

fn should_inject_image_generation_tool(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    !normalized.is_empty() && !normalized.ends_with("spark")
}

fn ensure_image_generation_tool_in_object(object: &mut Map<String, Value>) -> bool {
    let model = object.get("model").and_then(Value::as_str).unwrap_or("");
    if !should_inject_image_generation_tool(model) {
        return false;
    }

    let tool = json!({
        "type": "image_generation",
        "output_format": "png",
    });

    match object.get_mut("tools") {
        Some(Value::Array(tools)) => {
            if tools
                .iter()
                .any(|item| item.get("type").and_then(Value::as_str) == Some("image_generation"))
            {
                false
            } else {
                tools.push(tool);
                true
            }
        }
        _ => {
            object.insert("tools".to_string(), Value::Array(vec![tool]));
            true
        }
    }
}

fn remove_image_generation_tool_from_object(object: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        let before = tools.len();
        tools.retain(|item| item.get("type").and_then(Value::as_str) != Some("image_generation"));
        changed |= tools.len() != before;
    }

    let remove_tool_choice = object
        .get("tool_choice")
        .map(|choice| {
            choice.as_str() == Some("image_generation")
                || choice.get("type").and_then(Value::as_str) == Some("image_generation")
                || (choice.get("type").and_then(Value::as_str) == Some("tool")
                    && choice.get("name").and_then(Value::as_str) == Some("image_generation"))
        })
        .unwrap_or(false);
    if remove_tool_choice {
        object.remove("tool_choice");
        changed = true;
    }

    changed
}

fn image_generation_tools_allowed(
    mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> bool {
    match mode {
        CodexLocalAccessImageGenerationMode::Enabled => true,
        CodexLocalAccessImageGenerationMode::ImagesOnly => request_kind_is_image(request_kind),
        CodexLocalAccessImageGenerationMode::Disabled => false,
    }
}

fn build_images_responses_body(prompt: &str, images: &[String], tool: Value) -> Value {
    let mut content = vec![json!({
        "type": "input_text",
        "text": prompt,
    })];
    for image in images {
        let image_url = image.trim();
        if image_url.is_empty() {
            continue;
        }
        content.push(json!({
            "type": "input_image",
            "image_url": image_url,
        }));
    }

    json!({
        "instructions": "",
        "stream": true,
        "reasoning": {
            "effort": "medium",
            "summary": "auto",
        },
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "model": DEFAULT_IMAGES_MAIN_MODEL,
        "store": false,
        "tool_choice": {
            "type": "image_generation",
        },
        "input": [{
            "type": "message",
            "role": "user",
            "content": content,
        }],
        "tools": [tool],
    })
}

fn build_images_generation_request(body: &Value) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("images/generations 请求体必须是 JSON 对象".to_string())?;
    let prompt = json_string_field(request_obj, "prompt")
        .ok_or("images/generations 请求缺少 prompt".to_string())?;
    let response_format = normalize_image_response_format(request_obj.get("response_format"));
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tool = build_image_generation_tool(request_obj, "generate", false)?;

    Ok((
        build_images_responses_body(prompt, &[], tool),
        stream,
        response_format,
    ))
}

fn extract_json_edit_images(request_obj: &Map<String, Value>) -> Vec<String> {
    let mut images = Vec::new();

    if let Some(image) = request_obj.get("image").and_then(Value::as_str) {
        let trimmed = image.trim();
        if !trimmed.is_empty() {
            images.push(trimmed.to_string());
        }
    }

    if let Some(image_array) = request_obj.get("images").and_then(Value::as_array) {
        for image in image_array {
            if let Some(url) = image
                .get("image_url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                images.push(url.to_string());
            } else if let Some(url) = image
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                images.push(url.to_string());
            }
        }
    }

    images
}

fn build_images_edit_request_from_json(body: &Value) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("images/edits 请求体必须是 JSON 对象".to_string())?;
    let prompt = json_string_field(request_obj, "prompt")
        .ok_or("images/edits 请求缺少 prompt".to_string())?;
    let images = extract_json_edit_images(request_obj);
    if images.is_empty() {
        return Err("images/edits 请求缺少 images[].image_url".to_string());
    }

    let response_format = normalize_image_response_format(request_obj.get("response_format"));
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut tool = build_image_generation_tool(request_obj, "edit", true)?;
    if let Some(mask_url) = request_obj
        .get("mask")
        .and_then(|mask| mask.get("image_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(tool_obj) = tool.as_object_mut() {
            tool_obj.insert(
                "input_image_mask".to_string(),
                json!({ "image_url": mask_url }),
            );
        }
    }

    Ok((
        build_images_responses_body(prompt, &images, tool),
        stream,
        response_format,
    ))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let trimmed = part.trim();
        let (name, value) = trimmed.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("boundary") {
            return None;
        }
        let boundary = value.trim().trim_matches('"').to_string();
        if boundary.is_empty() {
            None
        } else {
            Some(boundary)
        }
    })
}

fn parse_content_disposition_params(value: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for part in value.split(';').skip(1) {
        let Some((name, raw_value)) = part.trim().split_once('=') else {
            continue;
        };
        let key = name.trim().to_ascii_lowercase();
        let value = raw_value.trim().trim_matches('"').to_string();
        if !key.is_empty() {
            params.insert(key, value);
        }
    }
    params
}

fn trim_part_trailing_newline(mut data: &[u8]) -> &[u8] {
    if data.ends_with(b"\r\n") {
        data = &data[..data.len().saturating_sub(2)];
    } else if data.ends_with(b"\n") {
        data = &data[..data.len().saturating_sub(1)];
    }
    data
}

fn parse_multipart_form_data(content_type: &str, body: &[u8]) -> Result<MultipartFormData, String> {
    let boundary = extract_multipart_boundary(content_type)
        .ok_or("multipart/form-data 缺少 boundary".to_string())?;
    let marker = format!("--{}", boundary).into_bytes();
    let mut form = MultipartFormData::default();
    let mut search_from = 0usize;

    loop {
        let Some(marker_index) = find_subslice(&body[search_from..], &marker) else {
            break;
        };
        let marker_start = search_from + marker_index;
        let mut part_start = marker_start + marker.len();

        if body
            .get(part_start..part_start + 2)
            .map(|bytes| bytes == b"--")
            .unwrap_or(false)
        {
            break;
        }
        if body
            .get(part_start..part_start + 2)
            .map(|bytes| bytes == b"\r\n")
            .unwrap_or(false)
        {
            part_start += 2;
        } else if body
            .get(part_start..part_start + 1)
            .map(|bytes| bytes == b"\n")
            .unwrap_or(false)
        {
            part_start += 1;
        }

        let Some(next_marker_offset) = find_subslice(&body[part_start..], &marker) else {
            break;
        };
        let next_marker_start = part_start + next_marker_offset;
        let part = trim_part_trailing_newline(&body[part_start..next_marker_start]);
        search_from = next_marker_start;

        let Some(header_end) = find_header_end(part) else {
            continue;
        };
        let header_text = String::from_utf8_lossy(&part[..header_end]);
        let part_body = &part[header_end..];
        let mut part_name = String::new();
        let mut part_filename = String::new();
        let mut part_content_type = String::new();

        for line in header_text.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("content-disposition") {
                let params = parse_content_disposition_params(value);
                part_name = params.get("name").cloned().unwrap_or_default();
                part_filename = params.get("filename").cloned().unwrap_or_default();
            } else if name.trim().eq_ignore_ascii_case("content-type") {
                part_content_type = value.trim().to_string();
            }
        }

        if part_name.is_empty() {
            continue;
        }
        if part_filename.is_empty() {
            let text = String::from_utf8_lossy(part_body).trim().to_string();
            form.fields.insert(part_name, text);
        } else {
            form.files.push(MultipartFilePart {
                name: part_name,
                content_type: part_content_type,
                data: part_body.to_vec(),
            });
        }
    }

    Ok(form)
}

fn detect_image_mime_type(data: &[u8], fallback: &str) -> String {
    let fallback = fallback.trim();
    if !fallback.is_empty() && fallback != "application/octet-stream" {
        return fallback.to_string();
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png".to_string()
    } else if data.starts_with(b"\xff\xd8\xff") {
        "image/jpeg".to_string()
    } else if data.starts_with(b"RIFF")
        && data
            .get(8..12)
            .map(|bytes| bytes == b"WEBP")
            .unwrap_or(false)
    {
        "image/webp".to_string()
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}

fn multipart_file_to_data_url(file: &MultipartFilePart) -> String {
    let mime_type = detect_image_mime_type(&file.data, &file.content_type);
    format!(
        "data:{};base64,{}",
        mime_type,
        general_purpose::STANDARD.encode(&file.data)
    )
}

fn multipart_field_value<'a>(form: &'a MultipartFormData, key: &str) -> Option<&'a str> {
    form.fields
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn multipart_field_bool(form: &MultipartFormData, key: &str, fallback: bool) -> bool {
    match multipart_field_value(form, key)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

fn multipart_field_number(form: &MultipartFormData, key: &str) -> Option<Value> {
    let raw = multipart_field_value(form, key)?;
    raw.parse::<i64>().ok().map(|value| json!(value))
}

fn build_images_edit_request_from_multipart(
    content_type: &str,
    body: &[u8],
) -> Result<(Value, bool, String), String> {
    let form = parse_multipart_form_data(content_type, body)?;
    let prompt =
        multipart_field_value(&form, "prompt").ok_or("images/edits 请求缺少 prompt".to_string())?;
    let image_files: Vec<&MultipartFilePart> = form
        .files
        .iter()
        .filter(|file| file.name == "image" || file.name == "image[]")
        .collect();
    if image_files.is_empty() {
        return Err("images/edits 请求缺少 image".to_string());
    }

    let mut request_obj = Map::new();
    request_obj.insert(
        "model".to_string(),
        Value::String(
            multipart_field_value(&form, "model")
                .unwrap_or(CODEX_IMAGE_MODEL_ID)
                .to_string(),
        ),
    );
    for key in [
        "size",
        "quality",
        "background",
        "output_format",
        "input_fidelity",
        "moderation",
    ] {
        if let Some(value) = multipart_field_value(&form, key) {
            request_obj.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    for key in ["output_compression", "partial_images"] {
        if let Some(value) = multipart_field_number(&form, key) {
            request_obj.insert(key.to_string(), value);
        }
    }

    let response_format = multipart_field_value(&form, "response_format")
        .unwrap_or("b64_json")
        .to_ascii_lowercase();
    let stream = multipart_field_bool(&form, "stream", false);
    let mut tool = build_image_generation_tool(&request_obj, "edit", true)?;
    if let Some(mask_file) = form.files.iter().find(|file| file.name == "mask") {
        if let Some(tool_obj) = tool.as_object_mut() {
            tool_obj.insert(
                "input_image_mask".to_string(),
                json!({ "image_url": multipart_file_to_data_url(mask_file) }),
            );
        }
    }

    let images: Vec<String> = image_files
        .into_iter()
        .map(multipart_file_to_data_url)
        .collect();

    Ok((
        build_images_responses_body(prompt, &images, tool),
        stream,
        response_format,
    ))
}

fn build_request_routing_hint(request: &ParsedRequest) -> RequestRoutingHint {
    let Some(body) = parse_request_body_json(&request.body) else {
        return RequestRoutingHint {
            session_affinity_key: extract_session_affinity_key(request),
            ..RequestRoutingHint::default()
        };
    };

    RequestRoutingHint {
        model_key: body
            .get("model")
            .and_then(Value::as_str)
            .map(resolve_supported_model_alias)
            .map(|model| normalize_model_key(&model))
            .unwrap_or_default(),
        previous_response_id: body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        session_affinity_key: extract_session_affinity_key(request),
    }
}

fn is_chat_completions_request(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target).trim();
    path == CHAT_COMPLETIONS_PATH || path.ends_with("/chat/completions")
}

fn is_responses_completion_event(event_type: &str) -> bool {
    matches!(event_type, "response.completed" | "response.done")
}

fn response_text_type_for_role(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("assistant") {
        "output_text"
    } else {
        "input_text"
    }
}

fn truncate_to_byte_limit(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }

    let mut end = 0usize;
    for (index, ch) in value.char_indices() {
        let next = index + ch.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    value[..end].to_string()
}

fn shorten_tool_name_if_needed(name: &str) -> String {
    const LIMIT: usize = 64;
    if name.len() <= LIMIT {
        return name.to_string();
    }
    if name.starts_with("mcp__") {
        if let Some(index) = name.rfind("__") {
            if index > 0 {
                let candidate = format!("mcp__{}", &name[index + 2..]);
                return truncate_to_byte_limit(&candidate, LIMIT);
            }
        }
    }
    truncate_to_byte_limit(name, LIMIT)
}

fn build_short_tool_name_map(body: &Value) -> HashMap<String, String> {
    const LIMIT: usize = 64;

    let mut names = Vec::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                continue;
            }
            if let Some(name) = tool
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                names.push(name.to_string());
            }
        }
    }

    let mut used = HashSet::new();
    let mut short_name_map = HashMap::new();
    for name in names {
        let base_candidate = shorten_tool_name_if_needed(&name);
        let unique = if used.insert(base_candidate.clone()) {
            base_candidate
        } else {
            let mut suffix_index = 1usize;
            loop {
                let suffix = format!("_{}", suffix_index);
                let allowed = LIMIT.saturating_sub(suffix.len());
                let candidate = format!(
                    "{}{}",
                    truncate_to_byte_limit(&base_candidate, allowed),
                    suffix
                );
                if used.insert(candidate.clone()) {
                    break candidate;
                }
                suffix_index += 1;
            }
        };
        short_name_map.insert(name, unique);
    }

    short_name_map
}

fn build_reverse_tool_name_map_from_request(
    original_request_body: &[u8],
) -> HashMap<String, String> {
    let Some(body) = parse_request_body_json(original_request_body) else {
        return HashMap::new();
    };

    build_short_tool_name_map(&body)
        .into_iter()
        .map(|(original, shortened)| (shortened, original))
        .collect()
}

fn map_tool_name(name: &str, short_name_map: &HashMap<String, String>) -> String {
    short_name_map
        .get(name)
        .cloned()
        .unwrap_or_else(|| shorten_tool_name_if_needed(name))
}

fn normalize_chat_content_part(part: &Value, role: &str) -> Option<Value> {
    match part {
        Value::String(text) => Some(json!({
            "type": response_text_type_for_role(role),
            "text": text,
        })),
        Value::Object(obj) => {
            let part_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
            match part_type {
                "" | "text" => {
                    let text = obj.get("text").and_then(Value::as_str).unwrap_or("");
                    Some(json!({
                        "type": response_text_type_for_role(role),
                        "text": text,
                    }))
                }
                "image_url" => {
                    if !role.eq_ignore_ascii_case("user") {
                        return None;
                    }
                    let image_url_value = obj.get("image_url")?;
                    match image_url_value {
                        Value::Object(image_url_obj) => {
                            let url = image_url_obj.get("url").and_then(Value::as_str)?;
                            Some(json!({
                                "type": "input_image",
                                "image_url": url,
                            }))
                        }
                        _ => None,
                    }
                }
                "file" => {
                    if !role.eq_ignore_ascii_case("user") {
                        return None;
                    }
                    let file_data = obj
                        .get("file")
                        .and_then(Value::as_object)
                        .and_then(|file| file.get("file_data"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if file_data.is_empty() {
                        return None;
                    }
                    let filename = obj
                        .get("file")
                        .and_then(Value::as_object)
                        .and_then(|file| file.get("filename"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let mut next = Map::new();
                    next.insert("type".to_string(), Value::String("input_file".to_string()));
                    next.insert(
                        "file_data".to_string(),
                        Value::String(file_data.to_string()),
                    );
                    if !filename.is_empty() {
                        next.insert("filename".to_string(), Value::String(filename.to_string()));
                    }
                    Some(Value::Object(next))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn normalize_chat_content_parts(content: &Value, role: &str) -> Vec<Value> {
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| normalize_chat_content_part(part, role))
            .collect(),
        other => normalize_chat_content_part(other, role)
            .map(|part| vec![part])
            .unwrap_or_default(),
    }
}

fn normalize_chat_tool_call(
    tool_call: &Value,
    short_name_map: &HashMap<String, String>,
) -> Option<Value> {
    let tool_call_obj = tool_call.as_object()?;
    let tool_type = tool_call_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if tool_type != "function" {
        return None;
    }

    let function_obj = tool_call_obj.get("function").and_then(Value::as_object);
    let name = function_obj
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let arguments = function_obj
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let call_id = tool_call_obj
        .get("id")
        .or_else(|| tool_call_obj.get("call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");

    Some(json!({
        "type": "function_call",
        "call_id": call_id,
        "name": map_tool_name(name, short_name_map),
        "arguments": arguments,
    }))
}

fn normalize_chat_tool_calls(
    tool_calls: &Value,
    short_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    tool_calls
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|tool_call| normalize_chat_tool_call(tool_call, short_name_map))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn normalize_chat_message_for_responses(
    message_obj: &Map<String, Value>,
    short_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    let role = message_obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");

    if role.eq_ignore_ascii_case("tool") {
        let output = message_obj
            .get("content")
            .map(extract_message_content_text)
            .unwrap_or_default();
        let call_id = message_obj
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        return vec![json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": output,
        })];
    }

    let normalized_content = message_obj
        .get("content")
        .map(|content| normalize_chat_content_parts(content, role))
        .unwrap_or_default();
    let mut items = Vec::new();

    if !normalized_content.is_empty() {
        let mapped_role = if role.eq_ignore_ascii_case("system") {
            "developer"
        } else {
            role
        };
        let next = json!({
            "type": "message",
            "role": mapped_role,
            "content": normalized_content,
        });
        items.push(next);
    }

    if role.eq_ignore_ascii_case("assistant") {
        if let Some(tool_calls) = message_obj.get("tool_calls") {
            items.extend(normalize_chat_tool_calls(tool_calls, short_name_map));
        }
    }

    items
}

fn normalize_chat_messages_for_responses(
    messages: &Value,
    short_name_map: &HashMap<String, String>,
) -> Value {
    let Some(message_items) = messages.as_array() else {
        return messages.clone();
    };

    let mut normalized = Vec::new();
    for item in message_items {
        let Some(message_obj) = item.as_object() else {
            normalized.push(item.clone());
            continue;
        };
        normalized.extend(normalize_chat_message_for_responses(
            message_obj,
            short_name_map,
        ));
    }

    Value::Array(normalized)
}

fn normalize_chat_tool(tool: &Value, short_name_map: &HashMap<String, String>) -> Option<Value> {
    let tool_obj = tool.as_object()?;
    let tool_type = tool_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");

    if tool_type != "function" {
        return Some(Value::Object(tool_obj.clone()));
    }

    let function_obj = tool_obj.get("function").and_then(Value::as_object);
    let name = function_obj
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let mut normalized = Map::new();
    normalized.insert("type".to_string(), Value::String("function".to_string()));
    normalized.insert(
        "name".to_string(),
        Value::String(map_tool_name(name, short_name_map)),
    );

    if let Some(description) = function_obj.and_then(|function| function.get("description")) {
        normalized.insert("description".to_string(), description.clone());
    }
    if let Some(parameters) = function_obj.and_then(|function| function.get("parameters")) {
        normalized.insert("parameters".to_string(), parameters.clone());
    }

    if let Some(strict) = function_obj
        .and_then(|function| function.get("strict"))
        .and_then(Value::as_bool)
    {
        normalized.insert("strict".to_string(), Value::Bool(strict));
    }

    Some(Value::Object(normalized))
}

fn normalize_chat_tools(tools: &Value, short_name_map: &HashMap<String, String>) -> Value {
    Value::Array(
        tools
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|tool| normalize_chat_tool(tool, short_name_map))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    )
}

fn normalize_chat_tool_choice(
    tool_choice: &Value,
    short_name_map: &HashMap<String, String>,
) -> Option<Value> {
    if let Some(mode) = tool_choice.as_str() {
        return Some(Value::String(mode.to_string()));
    }

    let Some(choice_obj) = tool_choice.as_object() else {
        return None;
    };
    let choice_type = choice_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if choice_type != "function" {
        return Some(Value::Object(choice_obj.clone()));
    }

    let name = choice_obj
        .get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    name.map(|name| {
        json!({
            "type": "function",
            "name": map_tool_name(name, short_name_map),
        })
    })
}

fn extract_message_content_text(content: &Value) -> String {
    match content {
        Value::String(raw) => raw.to_string(),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    append_non_empty_text(&mut text, part_text);
                    continue;
                }
                if let Some(part_text) = part.get("content").and_then(Value::as_str) {
                    append_non_empty_text(&mut text, part_text);
                }
            }
            text
        }
        _ => String::new(),
    }
}

fn build_responses_body_from_chat_completions(
    body: &Value,
) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("chat/completions 请求体必须是 JSON 对象".to_string())?;
    let model = request_obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(resolve_supported_model_alias)
        .ok_or("chat/completions 请求缺少 model".to_string())?;
    let messages = request_obj
        .get("messages")
        .ok_or("chat/completions 请求缺少 messages".to_string())?;
    let short_name_map = build_short_tool_name_map(body);
    let input = normalize_chat_messages_for_responses(messages, &short_name_map);
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut responses_obj = Map::new();
    responses_obj.insert("instructions".to_string(), Value::String(String::new()));
    responses_obj.insert("stream".to_string(), Value::Bool(true));
    responses_obj.insert("store".to_string(), Value::Bool(false));
    responses_obj.insert("model".to_string(), Value::String(model.clone()));
    responses_obj.insert("input".to_string(), input);
    responses_obj.insert("parallel_tool_calls".to_string(), Value::Bool(true));
    responses_obj.insert(
        "reasoning".to_string(),
        json!({
            "effort": request_obj
                .get("reasoning_effort")
                .cloned()
                .unwrap_or_else(|| Value::String("medium".to_string())),
            "summary": "auto",
        }),
    );
    responses_obj.insert(
        "include".to_string(),
        Value::Array(vec![Value::String(
            "reasoning.encrypted_content".to_string(),
        )]),
    );

    if let Some(tools) = request_obj.get("tools") {
        responses_obj.insert(
            "tools".to_string(),
            normalize_chat_tools(tools, &short_name_map),
        );
    }

    if let Some(tool_choice) = request_obj.get("tool_choice") {
        if let Some(choice) = normalize_chat_tool_choice(tool_choice, &short_name_map) {
            responses_obj.insert("tool_choice".to_string(), choice);
        }
    }

    let mut text_obj = Map::new();
    if let Some(response_format) = request_obj
        .get("response_format")
        .and_then(Value::as_object)
    {
        match response_format
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "text" => {
                text_obj.insert("format".to_string(), json!({ "type": "text" }));
            }
            "json_schema" => {
                if let Some(json_schema) = response_format
                    .get("json_schema")
                    .and_then(Value::as_object)
                {
                    let mut format_obj = Map::new();
                    format_obj.insert("type".to_string(), Value::String("json_schema".to_string()));
                    if let Some(name) = json_schema.get("name") {
                        format_obj.insert("name".to_string(), name.clone());
                    }
                    if let Some(strict) = json_schema.get("strict") {
                        format_obj.insert("strict".to_string(), strict.clone());
                    }
                    if let Some(schema) = json_schema.get("schema") {
                        format_obj.insert("schema".to_string(), schema.clone());
                    }
                    text_obj.insert("format".to_string(), Value::Object(format_obj));
                }
            }
            _ => {}
        }
    }
    if let Some(text_value) = request_obj.get("text").and_then(Value::as_object) {
        if let Some(verbosity) = text_value.get("verbosity") {
            text_obj.insert("verbosity".to_string(), verbosity.clone());
        }
    }
    if !text_obj.is_empty() {
        responses_obj.insert("text".to_string(), Value::Object(text_obj));
    }

    Ok((Value::Object(responses_obj), stream, model))
}

fn prepare_gateway_request(
    mut request: ParsedRequest,
) -> Result<(ParsedRequest, GatewayResponseAdapter), String> {
    if is_images_generations_request(&request.target) {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Err("images/generations 仅支持 POST".to_string());
        }
        let body_value = parse_request_body_json(&request.body)
            .ok_or("images/generations 请求体必须是合法 JSON".to_string())?;
        let (responses_body, stream, response_format) =
            build_images_generation_request(&body_value)?;
        request.target = RESPONSES_PATH.to_string();
        request.body = serde_json::to_vec(&responses_body)
            .map_err(|e| format!("序列化 images/generations 请求体失败: {}", e))?;
        request
            .headers
            .insert("accept".to_string(), "text/event-stream".to_string());
        request
            .headers
            .insert("content-type".to_string(), "application/json".to_string());
        return Ok((
            request,
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix: "image_generation".to_string(),
            },
        ));
    }

    if is_images_edits_request(&request.target) {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Err("images/edits 仅支持 POST".to_string());
        }
        let content_type = request
            .headers
            .get("content-type")
            .map(String::as_str)
            .unwrap_or("");
        let content_type_lower = content_type.to_ascii_lowercase();
        let (responses_body, stream, response_format) =
            if content_type_lower.starts_with("multipart/form-data") {
                build_images_edit_request_from_multipart(&content_type, &request.body)?
            } else {
                let body_value = parse_request_body_json(&request.body)
                    .ok_or("images/edits 请求体必须是合法 JSON".to_string())?;
                build_images_edit_request_from_json(&body_value)?
            };
        request.target = RESPONSES_PATH.to_string();
        request.body = serde_json::to_vec(&responses_body)
            .map_err(|e| format!("序列化 images/edits 请求体失败: {}", e))?;
        request
            .headers
            .insert("accept".to_string(), "text/event-stream".to_string());
        request
            .headers
            .insert("content-type".to_string(), "application/json".to_string());
        return Ok((
            request,
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix: "image_edit".to_string(),
            },
        ));
    }

    if !is_chat_completions_request(&request.target) {
        if is_responses_request(&request.target) {
            if !request.method.eq_ignore_ascii_case("POST") {
                return Err("responses 仅支持 POST".to_string());
            }
            let mut body_value = parse_request_body_json(&request.body)
                .ok_or("responses 请求体必须是合法 JSON".to_string())?;
            rewrite_request_model_alias_value(&mut body_value);
            codex_protocol::normalize_responses_body_for_codex(&mut body_value);
            request.body = serde_json::to_vec(&body_value)
                .map_err(|e| format!("序列化 responses 请求体失败: {}", e))?;
            request
                .headers
                .insert("accept".to_string(), "text/event-stream".to_string());
            request
                .headers
                .insert("content-type".to_string(), "application/json".to_string());
        } else if is_responses_compact_request(&request.target) {
            if !request.method.eq_ignore_ascii_case("POST") {
                return Err("responses/compact 仅支持 POST".to_string());
            }
            let mut body_value = parse_request_body_json(&request.body)
                .ok_or("responses/compact 请求体必须是合法 JSON".to_string())?;
            rewrite_request_model_alias_value(&mut body_value);
            codex_protocol::normalize_responses_body_for_codex(&mut body_value);
            if let Some(body_obj) = body_value.as_object_mut() {
                body_obj.remove("stream");
            }
            request.body = serde_json::to_vec(&body_value)
                .map_err(|e| format!("序列化 responses/compact 请求体失败: {}", e))?;
            request
                .headers
                .insert("accept".to_string(), "application/json".to_string());
            request
                .headers
                .insert("content-type".to_string(), "application/json".to_string());
        } else if let Some(rewritten_body) = rewrite_request_model_alias(&request.body)? {
            request.body = rewritten_body;
        }
        let request_is_stream = is_stream_request(&request.headers, &request.body);
        return Ok((
            request,
            GatewayResponseAdapter::Passthrough { request_is_stream },
        ));
    }

    if !request.method.eq_ignore_ascii_case("POST") {
        return Err("chat/completions 仅支持 POST".to_string());
    }

    let body_value = parse_request_body_json(&request.body)
        .ok_or("chat/completions 请求体必须是合法 JSON".to_string())?;
    let original_request_body = request.body.clone();
    let (responses_body, stream, requested_model) =
        build_responses_body_from_chat_completions(&body_value)?;
    request.target = RESPONSES_PATH.to_string();
    request.body = serde_json::to_vec(&responses_body)
        .map_err(|e| format!("序列化 responses 请求体失败: {}", e))?;
    request
        .headers
        .insert("accept".to_string(), "text/event-stream".to_string());
    request
        .headers
        .insert("content-type".to_string(), "application/json".to_string());

    Ok((
        request,
        GatewayResponseAdapter::ChatCompletions {
            stream,
            requested_model,
            original_request_body,
        },
    ))
}

fn response_payload_root(value: &Value) -> &Value {
    value
        .get("response")
        .filter(|item| item.is_object())
        .unwrap_or(value)
}

fn append_non_empty_text(buffer: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    buffer.push_str(text);
}

fn extract_output_text_from_response(response_body: &Value) -> String {
    let root = response_payload_root(response_body);
    let mut text = String::new();
    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if part.get("type").and_then(Value::as_str) != Some("output_text") {
                        continue;
                    }
                    if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                        append_non_empty_text(&mut text, part_text);
                    }
                }
            }
        }
    }
    text
}

fn extract_reasoning_text_from_response(response_body: &Value) -> String {
    let root = response_payload_root(response_body);
    let mut reasoning_text = String::new();
    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            if let Some(summary_items) = item.get("summary").and_then(Value::as_array) {
                for summary_item in summary_items {
                    if summary_item.get("type").and_then(Value::as_str) != Some("summary_text") {
                        continue;
                    }
                    if let Some(text) = summary_item.get("text").and_then(Value::as_str) {
                        append_non_empty_text(&mut reasoning_text, text);
                    }
                }
            }
        }
    }
    reasoning_text
}

fn extract_response_tool_calls(
    response_body: &Value,
    reverse_tool_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    let root = response_payload_root(response_body);
    root.get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let item_obj = item.as_object()?;
                    if item_obj.get("type").and_then(Value::as_str) != Some("function_call") {
                        return None;
                    }
                    let name = item_obj
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let restored_name = reverse_tool_name_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string());
                    let arguments = item_obj
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let call_id = item_obj
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Some(json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": restored_name,
                            "arguments": arguments,
                        },
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn build_chat_completion_message(
    response_body: &Value,
    reverse_tool_name_map: &HashMap<String, String>,
) -> Value {
    let content = extract_output_text_from_response(response_body);
    let reasoning_content = extract_reasoning_text_from_response(response_body);
    let tool_calls = extract_response_tool_calls(response_body, reverse_tool_name_map);
    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    message.insert("content".to_string(), Value::Null);
    message.insert("reasoning_content".to_string(), Value::Null);
    message.insert("tool_calls".to_string(), Value::Null);

    if !content.is_empty() {
        message.insert("content".to_string(), Value::String(content));
    }
    if !reasoning_content.is_empty() {
        message.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content),
        );
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    Value::Object(message)
}

fn resolve_chat_finish_reason(response_body: &Value, has_tool_calls: bool) -> String {
    let root = response_payload_root(response_body);
    if root.get("status").and_then(Value::as_str) == Some("completed") {
        if has_tool_calls {
            "tool_calls".to_string()
        } else {
            "stop".to_string()
        }
    } else {
        "stop".to_string()
    }
}

fn build_chat_completion_payload(
    response_body: &Value,
    requested_model: &str,
    original_request_body: &[u8],
) -> Value {
    let root = response_payload_root(response_body);
    let reverse_tool_name_map = build_reverse_tool_name_map_from_request(original_request_body);
    let id = root
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("chatcmpl-local-{}", now_ms()));
    let created = root
        .get("created_at")
        .or_else(|| root.get("created"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let model = root
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| requested_model.to_string());
    let message = build_chat_completion_message(response_body, &reverse_tool_name_map);
    let has_tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|tool_calls| !tool_calls.is_empty())
        .unwrap_or(false);
    let finish_reason = resolve_chat_finish_reason(response_body, has_tool_calls);
    let usage = extract_usage_capture(response_body).unwrap_or_default();

    json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
            "native_finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": {
                "cached_tokens": usage.cached_tokens,
            },
            "completion_tokens_details": {
                "reasoning_tokens": usage.reasoning_tokens,
            },
        },
    })
}

#[derive(Debug, Default)]
struct ChatCompletionStreamState {
    response_id: String,
    created_at: i64,
    model: String,
    function_call_index: i64,
    has_received_arguments_delta: bool,
    has_tool_call_announced: bool,
}

fn push_sse_payload(stream_body: &mut String, payload: Value) {
    stream_body.push_str("data: ");
    stream_body.push_str(
        serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{\"error\":\"failed to encode stream payload\"}".to_string())
            .as_str(),
    );
    stream_body.push_str("\n\n");
}

#[derive(Debug)]
struct ChatCompletionStreamTransformer {
    reverse_tool_name_map: HashMap<String, String>,
    requested_model: String,
    stream_buffer: Vec<u8>,
    state: ChatCompletionStreamState,
    response_capture: ResponseCapture,
}

impl ChatCompletionStreamTransformer {
    fn new(original_request_body: &[u8], requested_model: &str) -> Self {
        Self {
            reverse_tool_name_map: build_reverse_tool_name_map_from_request(original_request_body),
            requested_model: requested_model.to_string(),
            stream_buffer: Vec::new(),
            state: ChatCompletionStreamState {
                model: requested_model.to_string(),
                function_call_index: -1,
                ..Default::default()
            },
            response_capture: ResponseCapture::default(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }
        self.stream_buffer.extend_from_slice(chunk);
        self.process_buffer(false)
    }

    fn finish(mut self) -> (Vec<u8>, ResponseCapture) {
        let mut output = self.process_buffer(true);
        output.extend_from_slice(b"data: [DONE]\n\n");
        (output, self.response_capture)
    }

    fn process_buffer(&mut self, flush_tail: bool) -> Vec<u8> {
        let mut stream_body = String::new();

        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_frame(&frame, &mut stream_body);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_frame(&frame, &mut stream_body);
        }

        stream_body.into_bytes()
    }

    fn process_frame(&mut self, frame: &[u8], stream_body: &mut String) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        let Ok(event) = serde_json::from_str::<Value>(&payload) else {
            return;
        };

        if self.response_capture.terminal_error.is_none() {
            if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &event) {
                self.response_capture.terminal_error =
                    Some(format_upstream_response_failed_error(&signal));
            }
        }
        if let Some(usage) = extract_usage_capture(&event) {
            self.response_capture.usage = Some(usage);
        }
        if self.response_capture.response_id.is_none() {
            self.response_capture.response_id = extract_response_id(&event);
        }

        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("");

        if event_type == "response.created" {
            if let Some(response) = event.get("response").and_then(Value::as_object) {
                self.state.response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.state.created_at = response
                    .get("created_at")
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                self.state.model = response
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(self.requested_model.as_str())
                    .to_string();
            }
            if self.response_capture.response_id.is_none() && !self.state.response_id.is_empty() {
                self.response_capture.response_id = Some(self.state.response_id.clone());
            }
            return;
        }

        let mut template = build_chat_chunk_template(&self.state, &self.requested_model, &event);

        match event_type {
            "response.reasoning_summary_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["role"] =
                        Value::String("assistant".to_string());
                    template["choices"][0]["delta"]["reasoning_content"] =
                        Value::String(delta.to_string());
                    push_sse_payload(stream_body, template);
                }
            }
            "response.reasoning_summary_text.done" => {
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["reasoning_content"] =
                    Value::String("\n\n".to_string());
                push_sse_payload(stream_body, template);
            }
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["role"] =
                        Value::String("assistant".to_string());
                    template["choices"][0]["delta"]["content"] = Value::String(delta.to_string());
                    push_sse_payload(stream_body, template);
                }
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item").and_then(Value::as_object) else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }

                self.state.function_call_index += 1;
                self.state.has_received_arguments_delta = false;
                self.state.has_tool_call_announced = true;

                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let restored_name = self
                    .reverse_tool_name_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string());
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["tool_calls"] = json!([{
                    "index": self.state.function_call_index,
                    "id": item.get("call_id").cloned().unwrap_or(Value::String(String::new())),
                    "type": "function",
                    "function": {
                        "name": restored_name,
                        "arguments": "",
                    }
                }]);
                push_sse_payload(stream_body, template);
            }
            "response.function_call_arguments.delta" => {
                self.state.has_received_arguments_delta = true;
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["tool_calls"] = json!([{
                        "index": self.state.function_call_index,
                        "function": {
                            "arguments": delta,
                        }
                    }]);
                    push_sse_payload(stream_body, template);
                }
            }
            "response.function_call_arguments.done" => {
                if self.state.has_received_arguments_delta {
                    return;
                }
                if let Some(arguments) = event.get("arguments").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["tool_calls"] = json!([{
                        "index": self.state.function_call_index,
                        "function": {
                            "arguments": arguments,
                        }
                    }]);
                    push_sse_payload(stream_body, template);
                }
            }
            "response.output_item.done" => {
                let Some(item) = event.get("item").and_then(Value::as_object) else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }

                if self.state.has_tool_call_announced {
                    self.state.has_tool_call_announced = false;
                    return;
                }

                self.state.function_call_index += 1;
                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let restored_name = self
                    .reverse_tool_name_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string());
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["tool_calls"] = json!([{
                    "index": self.state.function_call_index,
                    "id": item.get("call_id").cloned().unwrap_or(Value::String(String::new())),
                    "type": "function",
                    "function": {
                        "name": restored_name,
                        "arguments": item
                            .get("arguments")
                            .cloned()
                            .unwrap_or(Value::String(String::new())),
                    }
                }]);
                push_sse_payload(stream_body, template);
            }
            event_type if is_responses_completion_event(event_type) => {
                let finish_reason = if self.state.function_call_index >= 0 {
                    "tool_calls"
                } else {
                    "stop"
                };
                template["choices"][0]["finish_reason"] = Value::String(finish_reason.to_string());
                template["choices"][0]["native_finish_reason"] =
                    Value::String(finish_reason.to_string());
                push_sse_payload(stream_body, template);
            }
            _ => {}
        }
    }
}

fn build_chat_chunk_template(
    state: &ChatCompletionStreamState,
    requested_model: &str,
    event: &Value,
) -> Value {
    let model = event
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            if state.model.trim().is_empty() {
                None
            } else {
                Some(state.model.clone())
            }
        })
        .unwrap_or_else(|| requested_model.to_string());
    let id = if state.response_id.trim().is_empty() {
        format!("chatcmpl-local-{}", now_ms())
    } else {
        state.response_id.clone()
    };
    let created = if state.created_at > 0 {
        state.created_at
    } else {
        chrono::Utc::now().timestamp()
    };

    let usage = event
        .get("response")
        .and_then(|response| response.get("usage"))
        .cloned()
        .or_else(|| event.get("usage").cloned());

    let mut template = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": Value::Null,
            "native_finish_reason": Value::Null,
        }],
    });
    if let Some(usage) = usage {
        let parsed_usage = extract_usage_capture(&json!({ "response": { "usage": usage } }))
            .or_else(|| extract_usage_capture(&json!({ "usage": usage })))
            .unwrap_or_default();
        template["usage"] = json!({
            "prompt_tokens": parsed_usage.input_tokens,
            "completion_tokens": parsed_usage.output_tokens,
            "total_tokens": parsed_usage.total_tokens,
            "prompt_tokens_details": {
                "cached_tokens": parsed_usage.cached_tokens,
            },
            "completion_tokens_details": {
                "reasoning_tokens": parsed_usage.reasoning_tokens,
            },
        });
    }
    template
}

fn build_chat_completion_stream_body(
    upstream_body: &[u8],
    original_request_body: &[u8],
    requested_model: &str,
) -> String {
    let mut transformer =
        ChatCompletionStreamTransformer::new(original_request_body, requested_model);
    let mut stream_body = transformer.feed(upstream_body);
    let (tail, _) = transformer.finish();
    stream_body.extend_from_slice(&tail);
    String::from_utf8(stream_body).unwrap_or_default()
}

fn build_cooldown_key(account_id: &str, model_key: &str) -> Option<String> {
    let account_id = account_id.trim();
    let model_key = model_key.trim();
    if account_id.is_empty() || model_key.is_empty() {
        return None;
    }
    Some(format!(
        "{}{}{}",
        account_id, COOLDOWN_KEY_SEPARATOR, model_key
    ))
}

fn build_ordered_account_ids(
    account_ids: &[String],
    start: usize,
    preferred_account_id: Option<&str>,
) -> Vec<String> {
    if account_ids.is_empty() {
        return Vec::new();
    }

    let mut ordered = Vec::with_capacity(account_ids.len());
    if let Some(preferred) = preferred_account_id {
        if account_ids.iter().any(|account_id| account_id == preferred) {
            ordered.push(preferred.to_string());
        }
    }

    for offset in 0..account_ids.len() {
        let account_id = &account_ids[(start + offset) % account_ids.len()];
        if ordered.iter().any(|value| value == account_id) {
            continue;
        }
        ordered.push(account_id.clone());
    }

    ordered
}

fn normalize_plan_key(plan_type: Option<&str>) -> String {
    let normalized = plan_type.unwrap_or("").trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return "free".to_string();
    }
    if normalized.contains("enterprise") {
        return "enterprise".to_string();
    }
    if normalized.contains("health") {
        return "health".to_string();
    }
    if normalized.contains("gov") {
        return "gov".to_string();
    }
    if normalized.contains("teacher") {
        return "teachers".to_string();
    }
    if normalized.contains("business") {
        return "business".to_string();
    }
    if normalized.contains("team") {
        return "team".to_string();
    }
    if normalized.contains("edu") {
        return "edu".to_string();
    }
    if normalized.contains("go") {
        return "go".to_string();
    }
    if normalized.contains("plus") {
        return "plus".to_string();
    }
    if normalized.contains("pro") {
        return "pro".to_string();
    }
    if normalized.contains("free") {
        return "free".to_string();
    }
    normalized
}

fn normalize_auth_file_plan_type(plan_type: Option<&str>) -> Option<&'static str> {
    let normalized = plan_type?
        .trim()
        .to_ascii_lowercase()
        .replace(['_', ' '], "-");
    match normalized.as_str() {
        "prolite" | "pro-lite" => Some("prolite"),
        "promax" | "pro-max" => Some("promax"),
        _ => None,
    }
}

fn resolve_plan_rank(account: &CodexAccount) -> Option<i32> {
    let plan_key = normalize_plan_key(account.plan_type.as_deref());
    let auth_file_plan_type = normalize_auth_file_plan_type(account.auth_file_plan_type.as_deref())
        .or_else(|| normalize_auth_file_plan_type(account.plan_type.as_deref()));

    let rank = match plan_key.as_str() {
        "enterprise" => 700,
        "edu" => 700,
        "health" => 700,
        "gov" => 700,
        "teachers" => 700,
        "pro" => match auth_file_plan_type {
            Some("promax") => 600,
            Some("prolite") => 500,
            _ => 500,
        },
        "business" => 300,
        "team" => 300,
        "plus" => 300,
        "go" => 200,
        "free" => 100,
        _ => return None,
    };

    Some(rank)
}

fn resolve_remaining_quota(account: &CodexAccount) -> Option<i32> {
    let quota = account.quota.as_ref()?;
    let mut percentages = Vec::new();
    if quota.hourly_window_present.unwrap_or(true) {
        percentages.push(quota.hourly_percentage.clamp(0, 100));
    }
    if quota.weekly_window_present.unwrap_or(true) {
        percentages.push(quota.weekly_percentage.clamp(0, 100));
    }
    percentages.into_iter().min()
}

fn resolve_subscription_expiry_ms(account: &CodexAccount) -> Option<i64> {
    let raw = account.subscription_active_until.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.chars().all(|ch| ch.is_ascii_digit()) {
        let mut timestamp = raw.parse::<i64>().ok()?;
        if timestamp < 1_000_000_000_000 {
            timestamp *= 1000;
        }
        return Some(timestamp);
    }

    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| parsed.timestamp_millis())
}

fn build_routing_candidates(ordered_account_ids: &[String]) -> Vec<RoutingCandidate> {
    ordered_account_ids
        .iter()
        .map(|account_id| {
            let account = try_get_cached_account_for_routing(account_id)
                .or_else(|| codex_account::load_account(account_id));
            RoutingCandidate {
                account_id: account_id.clone(),
                plan_rank: account.as_ref().and_then(resolve_plan_rank),
                remaining_quota: account.as_ref().and_then(resolve_remaining_quota),
                subscription_expiry_ms: account.as_ref().and_then(resolve_subscription_expiry_ms),
            }
        })
        .collect()
}

fn compare_routing_candidates(
    left: &RoutingCandidate,
    right: &RoutingCandidate,
    strategy: CodexLocalAccessRoutingStrategy,
    original_index: &HashMap<String, usize>,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let compare_option_desc = |a: Option<i32>, b: Option<i32>| match (a, b) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    let compare_option_asc = |a: Option<i32>, b: Option<i32>| match (a, b) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    let compare_option_i64_asc = |a: Option<i64>, b: Option<i64>| match (a, b) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };

    let ordering = match strategy {
        CodexLocalAccessRoutingStrategy::Auto => {
            compare_option_desc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::QuotaHighFirst => {
            compare_option_desc(left.remaining_quota, right.remaining_quota)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
        }
        CodexLocalAccessRoutingStrategy::QuotaLowFirst => {
            compare_option_asc(left.remaining_quota, right.remaining_quota)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
        }
        CodexLocalAccessRoutingStrategy::PlanHighFirst => {
            compare_option_desc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::PlanLowFirst => {
            compare_option_asc(left.plan_rank, right.plan_rank)
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::ExpirySoonFirst => {
            compare_option_i64_asc(left.subscription_expiry_ms, right.subscription_expiry_ms)
                .then_with(|| compare_option_desc(left.plan_rank, right.plan_rank))
                .then_with(|| compare_option_desc(left.remaining_quota, right.remaining_quota))
        }
        CodexLocalAccessRoutingStrategy::Custom => Ordering::Equal,
    };

    ordering.then_with(|| {
        let left_index = original_index
            .get(&left.account_id)
            .copied()
            .unwrap_or(usize::MAX);
        let right_index = original_index
            .get(&right.account_id)
            .copied()
            .unwrap_or(usize::MAX);
        left_index.cmp(&right_index)
    })
}

fn normalize_custom_routing_rule(
    rule: CodexLocalAccessCustomRoutingRule,
) -> Option<CodexLocalAccessCustomRoutingRule> {
    let account_id = rule.account_id.trim().to_string();
    if account_id.is_empty() {
        return None;
    }

    Some(CodexLocalAccessCustomRoutingRule {
        account_id,
        priority: rule
            .priority
            .clamp(CUSTOM_ROUTING_PRIORITY_MIN, CUSTOM_ROUTING_PRIORITY_MAX),
        weight: rule
            .weight
            .clamp(CUSTOM_ROUTING_WEIGHT_MIN, CUSTOM_ROUTING_WEIGHT_MAX),
    })
}

fn normalize_custom_routing_rules(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
    account_ids: &[String],
) -> Vec<CodexLocalAccessCustomRoutingRule> {
    let valid_account_ids: HashSet<&str> = account_ids.iter().map(String::as_str).collect();
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for rule in rules {
        let Some(rule) = normalize_custom_routing_rule(rule) else {
            continue;
        };
        if !valid_account_ids.contains(rule.account_id.as_str()) {
            continue;
        }
        if seen.insert(rule.account_id.clone()) {
            normalized.push(rule);
        }
    }

    normalized
}

fn custom_rule_map(rules: &[CodexLocalAccessCustomRoutingRule]) -> HashMap<&str, (i32, u32)> {
    rules
        .iter()
        .map(|rule| {
            (
                rule.account_id.as_str(),
                (
                    rule.priority
                        .clamp(CUSTOM_ROUTING_PRIORITY_MIN, CUSTOM_ROUTING_PRIORITY_MAX),
                    rule.weight
                        .clamp(CUSTOM_ROUTING_WEIGHT_MIN, CUSTOM_ROUTING_WEIGHT_MAX),
                ),
            )
        })
        .collect()
}

fn weighted_group_order(
    group: &[String],
    weights: &HashMap<&str, (i32, u32)>,
    start: usize,
) -> Vec<String> {
    if group.len() <= 1 {
        return group.to_vec();
    }

    let total_weight = group.iter().fold(0usize, |sum, account_id| {
        let weight = weights
            .get(account_id.as_str())
            .map(|(_, weight)| *weight)
            .unwrap_or(CUSTOM_ROUTING_WEIGHT_MIN) as usize;
        sum.saturating_add(weight.max(1))
    });
    if total_weight == 0 {
        return group.to_vec();
    }

    let mut slot = start % total_weight;
    let mut first_index = 0usize;
    for (index, account_id) in group.iter().enumerate() {
        let weight = weights
            .get(account_id.as_str())
            .map(|(_, weight)| *weight)
            .unwrap_or(CUSTOM_ROUTING_WEIGHT_MIN) as usize;
        if slot < weight {
            first_index = index;
            break;
        }
        slot -= weight;
    }

    (0..group.len())
        .map(|offset| group[(first_index + offset) % group.len()].clone())
        .collect()
}

fn apply_custom_routing_strategy(
    account_ids: &[String],
    rules: &[CodexLocalAccessCustomRoutingRule],
    start: usize,
) -> Vec<String> {
    let rule_map = custom_rule_map(rules);
    let mut priority_groups: Vec<(i32, Vec<String>)> = Vec::new();

    for account_id in account_ids {
        let priority = rule_map
            .get(account_id.as_str())
            .map(|(priority, _)| *priority)
            .unwrap_or(CUSTOM_ROUTING_PRIORITY_MIN);
        if let Some((_, group)) = priority_groups
            .iter_mut()
            .find(|(group_priority, _)| *group_priority == priority)
        {
            group.push(account_id.clone());
        } else {
            priority_groups.push((priority, vec![account_id.clone()]));
        }
    }

    priority_groups.sort_by(|left, right| right.0.cmp(&left.0));

    let mut ordered = Vec::with_capacity(account_ids.len());
    for (_, group) in priority_groups {
        ordered.extend(weighted_group_order(&group, &rule_map, start));
    }
    ordered
}

fn apply_routing_strategy(
    account_ids: &[String],
    strategy: CodexLocalAccessRoutingStrategy,
    custom_rules: &[CodexLocalAccessCustomRoutingRule],
    start: usize,
) -> Vec<String> {
    if strategy == CodexLocalAccessRoutingStrategy::Custom {
        return apply_custom_routing_strategy(account_ids, custom_rules, start);
    }

    let original_index: HashMap<String, usize> = account_ids
        .iter()
        .enumerate()
        .map(|(index, account_id)| (account_id.clone(), index))
        .collect();
    let mut candidates = build_routing_candidates(account_ids);
    candidates
        .sort_by(|left, right| compare_routing_candidates(left, right, strategy, &original_index));
    candidates
        .into_iter()
        .map(|candidate| candidate.account_id)
        .collect()
}

fn pin_account_to_front(
    account_ids: Vec<String>,
    preferred_account_id: Option<&str>,
) -> Vec<String> {
    let Some(preferred_account_id) = preferred_account_id else {
        return account_ids;
    };
    let preferred_account_id = preferred_account_id.trim();
    if preferred_account_id.is_empty() {
        return account_ids;
    }

    let mut ordered = Vec::with_capacity(account_ids.len());
    if account_ids
        .iter()
        .any(|account_id| account_id == preferred_account_id)
    {
        ordered.push(preferred_account_id.to_string());
    }
    for account_id in account_ids {
        if account_id == preferred_account_id {
            continue;
        }
        ordered.push(account_id);
    }
    ordered
}

fn format_retry_after_duration(wait: Duration) -> String {
    let seconds = wait.as_secs().max(1);
    format!("{} 秒", seconds)
}

fn build_cooldown_unavailable_message(model_key: &str, wait: Duration) -> String {
    let wait_text = format_retry_after_duration(wait);
    if model_key.trim().is_empty() {
        format!("当前 API 服务账号均在冷却中，请 {} 后重试", wait_text)
    } else {
        format!(
            "模型 {} 的可用账号均在冷却中，请 {} 后重试",
            model_key, wait_text,
        )
    }
}

fn parse_codex_retry_after(status: StatusCode, error_body: &str) -> Option<Duration> {
    if status != StatusCode::TOO_MANY_REQUESTS || error_body.trim().is_empty() {
        return None;
    }

    let payload = serde_json::from_str::<Value>(error_body).ok()?;
    let error = payload.get("error")?;
    if error.get("type").and_then(Value::as_str).map(str::trim) != Some("usage_limit_reached") {
        return None;
    }

    let now_seconds = chrono::Utc::now().timestamp();
    if let Some(resets_at) = error.get("resets_at").and_then(Value::as_i64) {
        if resets_at > now_seconds {
            let delta = resets_at.saturating_sub(now_seconds) as u64;
            if delta > 0 {
                return Some(Duration::from_secs(delta));
            }
        }
    }

    error
        .get("resets_in_seconds")
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0)
        .map(|seconds| Duration::from_secs(seconds as u64))
}

fn empty_stats_snapshot() -> CodexLocalAccessStats {
    let now = now_ms();
    let day_since = now.saturating_sub(DAY_WINDOW_MS);
    let week_since = now.saturating_sub(WEEK_WINDOW_MS);
    let month_since = now.saturating_sub(MONTH_WINDOW_MS);
    CodexLocalAccessStats {
        since: now,
        updated_at: now,
        totals: CodexLocalAccessUsageStats::default(),
        accounts: Vec::new(),
        models: Vec::new(),
        api_keys: Vec::new(),
        daily: CodexLocalAccessStatsWindow {
            since: day_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
        },
        weekly: CodexLocalAccessStatsWindow {
            since: week_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
        },
        monthly: CodexLocalAccessStatsWindow {
            since: month_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
            models: Vec::new(),
            api_keys: Vec::new(),
        },
        events: Vec::new(),
    }
}

fn empty_stats_window(since: i64, updated_at: i64) -> CodexLocalAccessStatsWindow {
    CodexLocalAccessStatsWindow {
        since,
        updated_at,
        totals: CodexLocalAccessUsageStats::default(),
        accounts: Vec::new(),
        models: Vec::new(),
        api_keys: Vec::new(),
    }
}

fn sort_usage_accounts(accounts: &mut [CodexLocalAccessAccountStats]) {
    accounts.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.account_id.cmp(&right.account_id))
    });
}

fn sort_usage_models(models: &mut [CodexLocalAccessModelStats]) {
    models.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
}

fn sort_usage_api_keys(api_keys: &mut [CodexLocalAccessApiKeyStats]) {
    api_keys.sort_by(|left, right| {
        right
            .usage
            .request_count
            .cmp(&left.usage.request_count)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.api_key_id.cmp(&right.api_key_id))
    });
}

fn model_pricing(
    model_id: &str,
    input_usd_per_million: f64,
    cached_input_usd_per_million: Option<f64>,
    output_usd_per_million: f64,
) -> CodexLocalAccessModelPricing {
    CodexLocalAccessModelPricing {
        model_id: model_id.to_string(),
        input_usd_per_million,
        output_usd_per_million,
        cached_input_usd_per_million,
    }
}

fn default_model_pricing_presets() -> Vec<CodexLocalAccessModelPricing> {
    vec![
        model_pricing("gpt-5.5", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.4", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.4-mini", 0.25, Some(0.025), 2.0),
        model_pricing("gpt-5-codex", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5-codex-mini", 0.25, Some(0.025), 2.0),
        model_pricing("gpt-5.3-codex", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.3-codex-spark", 0.25, Some(0.025), 2.0),
        model_pricing("gpt-5.2", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.2-codex", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.1-codex-max", 1.25, Some(0.125), 10.0),
        model_pricing("gpt-5.1-codex-mini", 0.25, Some(0.025), 2.0),
    ]
}

fn default_model_pricing_for_model(model_id: &str) -> Option<CodexLocalAccessModelPricing> {
    let key = model_id.trim().to_ascii_lowercase();
    default_model_pricing_presets()
        .into_iter()
        .find(|item| item.model_id.eq_ignore_ascii_case(&key))
}

fn normalize_price_value(value: f64) -> f64 {
    if !value.is_finite() || value < 0.0 {
        0.0
    } else {
        value.min(MAX_MODEL_PRICE_USD_PER_MILLION)
    }
}

fn normalize_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Vec<CodexLocalAccessModelPricing> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for pricing in model_pricings {
        let model_id = pricing.model_id.trim().to_string();
        if model_id.is_empty() {
            continue;
        }
        let key = model_id.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        normalized.push(CodexLocalAccessModelPricing {
            model_id,
            input_usd_per_million: normalize_price_value(pricing.input_usd_per_million),
            output_usd_per_million: normalize_price_value(pricing.output_usd_per_million),
            cached_input_usd_per_million: pricing
                .cached_input_usd_per_million
                .map(normalize_price_value),
        });
    }
    normalized
}

fn resolve_model_pricing(
    collection: Option<&CodexLocalAccessCollection>,
    model_id: Option<&str>,
) -> Option<CodexLocalAccessModelPricing> {
    let model_id = model_id.map(str::trim).filter(|value| !value.is_empty())?;
    collection
        .and_then(|collection| {
            collection
                .model_pricings
                .iter()
                .find(|item| item.model_id.eq_ignore_ascii_case(model_id))
                .cloned()
        })
        .or_else(|| default_model_pricing_for_model(model_id))
}

fn calculate_usage_cost_usd(
    usage: Option<&UsageCapture>,
    pricing: Option<&CodexLocalAccessModelPricing>,
) -> f64 {
    let (Some(usage), Some(pricing)) = (usage, pricing) else {
        return 0.0;
    };
    let cached_tokens = usage.cached_tokens.min(usage.input_tokens);
    let normal_input_tokens = usage.input_tokens.saturating_sub(cached_tokens);
    let cached_input_price = pricing
        .cached_input_usd_per_million
        .unwrap_or(pricing.input_usd_per_million);
    let cost = (normal_input_tokens as f64 * pricing.input_usd_per_million
        + cached_tokens as f64 * cached_input_price
        + usage.output_tokens as f64 * pricing.output_usd_per_million)
        / 1_000_000.0;
    if cost.is_finite() && cost > 0.0 {
        cost
    } else {
        0.0
    }
}

fn trim_recent_events(events: &mut Vec<CodexLocalAccessUsageEvent>, month_since: i64) {
    events.retain(|event| event.timestamp > 0 && event.timestamp >= month_since);
    events.sort_by_key(|event| event.timestamp);
}

fn request_kind_to_db_value(request_kind: CodexLocalAccessRequestKind) -> &'static str {
    match request_kind {
        CodexLocalAccessRequestKind::Text => "text",
        CodexLocalAccessRequestKind::ImageGeneration => "image_generation",
        CodexLocalAccessRequestKind::ImageEdit => "image_edit",
        CodexLocalAccessRequestKind::Other => "other",
    }
}

fn request_kind_from_db_value(value: &str) -> CodexLocalAccessRequestKind {
    match value.trim() {
        "text" => CodexLocalAccessRequestKind::Text,
        "image_generation" => CodexLocalAccessRequestKind::ImageGeneration,
        "image_edit" => CodexLocalAccessRequestKind::ImageEdit,
        _ => CodexLocalAccessRequestKind::Other,
    }
}

fn gateway_mode_to_db_value(gateway_mode: CodexLocalAccessGatewayMode) -> &'static str {
    match gateway_mode {
        CodexLocalAccessGatewayMode::Legacy => "legacy",
        CodexLocalAccessGatewayMode::Sidecar => "sidecar",
    }
}

fn gateway_mode_from_db_value(value: &str) -> Option<CodexLocalAccessGatewayMode> {
    match value.trim() {
        "legacy" => Some(CodexLocalAccessGatewayMode::Legacy),
        "sidecar" => Some(CodexLocalAccessGatewayMode::Sidecar),
        _ => None,
    }
}

fn bool_to_db_value(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn local_access_log_event_key(event: &CodexLocalAccessUsageEvent) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    let mut feed = |value: &str| {
        for byte in value.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    };
    feed(&event.timestamp.to_string());
    feed(event.request_id.as_str());
    feed(event.account_id.as_str());
    feed(event.email.as_str());
    feed(event.api_key_id.as_str());
    feed(event.api_key_label.as_str());
    feed(event.model_id.as_str());
    feed(
        event
            .gateway_mode
            .map(gateway_mode_to_db_value)
            .unwrap_or_default(),
    );
    feed(request_kind_to_db_value(event.request_kind));
    feed(if event.success { "1" } else { "0" });
    feed(event.error_category.as_str());
    feed(&event.latency_ms.to_string());
    feed(&event.input_tokens.to_string());
    feed(&event.output_tokens.to_string());
    feed(&event.total_tokens.to_string());
    feed(&event.cached_tokens.to_string());
    feed(&event.reasoning_tokens.to_string());
    format!("{hash:016x}")
}

fn local_access_logs_db_sidecar_paths(path: &Path) -> Vec<PathBuf> {
    let raw = path.to_string_lossy();
    vec![
        PathBuf::from(format!("{}-wal", raw)),
        PathBuf::from(format!("{}-shm", raw)),
    ]
}

fn is_recoverable_logs_db_error(error: &SqliteError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("file is not a database")
        || message.contains("not a database")
        || message.contains("database disk image is malformed")
        || message.contains("database disk image is corrupt")
}

fn quarantine_local_access_logs_db(
    path: &Path,
    error: &SqliteError,
) -> Result<Option<PathBuf>, String> {
    let backup_path = crate::modules::atomic_write::quarantine_file(path, "invalid-sqlite")?;
    for sidecar_path in local_access_logs_db_sidecar_paths(path) {
        if let Err(sidecar_error) =
            crate::modules::atomic_write::quarantine_file(&sidecar_path, "invalid-sqlite")
        {
            logger::log_codex_api_warn(&format!(
                "API 服务日志数据库 sidecar 隔离失败，已忽略: path={}, error={}",
                sidecar_path.display(),
                sidecar_error
            ));
        }
    }
    logger::log_codex_api_warn(&format!(
        "API 服务日志数据库异常，已隔离并准备重建: path={}, backup={}, error={}",
        path.display(),
        backup_path
            .as_ref()
            .map(|item| item.display().to_string())
            .unwrap_or_else(|| "-".to_string()),
        error
    ));
    Ok(backup_path)
}

fn open_local_access_logs_db_once(path: &Path) -> Result<Connection, SqliteError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        CREATE TABLE IF NOT EXISTS request_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_key TEXT NOT NULL UNIQUE,
            timestamp INTEGER NOT NULL,
            request_id TEXT NOT NULL DEFAULT '',
            account_id TEXT NOT NULL DEFAULT '',
            email TEXT NOT NULL DEFAULT '',
            api_key_id TEXT NOT NULL DEFAULT '',
            api_key_label TEXT NOT NULL DEFAULT '',
            model_id TEXT NOT NULL DEFAULT '',
            gateway_mode TEXT NOT NULL DEFAULT '',
            request_kind TEXT NOT NULL DEFAULT 'other',
            success INTEGER NOT NULL DEFAULT 0,
            http_status INTEGER,
            error_category TEXT NOT NULL DEFAULT '',
            error_message TEXT NOT NULL DEFAULT '',
            latency_ms INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            cached_tokens INTEGER NOT NULL DEFAULT 0,
            reasoning_tokens INTEGER NOT NULL DEFAULT 0,
            estimated_cost_usd REAL NOT NULL DEFAULT 0,
            input_usd_per_million REAL NOT NULL DEFAULT 0,
            output_usd_per_million REAL NOT NULL DEFAULT 0,
            cached_input_usd_per_million REAL
        );
        "#,
    )?;
    ensure_request_logs_column(&conn, "event_key", "event_key TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "request_id", "request_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "account_id", "account_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "email", "email TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(&conn, "api_key_id", "api_key_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(
        &conn,
        "api_key_label",
        "api_key_label TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "model_id", "model_id TEXT NOT NULL DEFAULT ''")?;
    ensure_request_logs_column(
        &conn,
        "gateway_mode",
        "gateway_mode TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "request_kind",
        "request_kind TEXT NOT NULL DEFAULT 'other'",
    )?;
    ensure_request_logs_column(&conn, "success", "success INTEGER NOT NULL DEFAULT 0")?;
    ensure_request_logs_column(&conn, "http_status", "http_status INTEGER")?;
    ensure_request_logs_column(
        &conn,
        "error_category",
        "error_category TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(
        &conn,
        "error_message",
        "error_message TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_request_logs_column(&conn, "latency_ms", "latency_ms INTEGER NOT NULL DEFAULT 0")?;
    ensure_request_logs_column(
        &conn,
        "input_tokens",
        "input_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "output_tokens",
        "output_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "total_tokens",
        "total_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "cached_tokens",
        "cached_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "reasoning_tokens",
        "reasoning_tokens INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "estimated_cost_usd",
        "estimated_cost_usd REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "input_usd_per_million",
        "input_usd_per_million REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "output_usd_per_million",
        "output_usd_per_million REAL NOT NULL DEFAULT 0",
    )?;
    ensure_request_logs_column(
        &conn,
        "cached_input_usd_per_million",
        "cached_input_usd_per_million REAL",
    )?;
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_timestamp
            ON request_logs(timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_model
            ON request_logs(model_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_account
            ON request_logs(account_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_email
            ON request_logs(email, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_api_key
            ON request_logs(api_key_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_gateway_mode
            ON request_logs(gateway_mode, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_kind
            ON request_logs(request_kind, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_success
            ON request_logs(success, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_error
            ON request_logs(error_category, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_codex_local_access_logs_request_id
            ON request_logs(request_id, timestamp DESC);
        "#,
    )?;
    conn.execute(
        r#"
        UPDATE request_logs
        SET error_category = 'upstream_response_failed'
        WHERE success = 0
          AND error_category != 'upstream_response_failed'
          AND (
            lower(error_message) LIKE '%upstream_response_failed%'
            OR lower(error_message) LIKE '%codex upstream response.failed%'
            OR lower(error_message) LIKE '%last_event=response.failed%'
          )
        "#,
        [],
    )?;
    conn.execute(
        r#"
        UPDATE request_logs
        SET error_category = 'stream_incomplete'
        WHERE success = 0
          AND error_category != 'stream_incomplete'
          AND error_category != 'upstream_response_failed'
          AND (
            lower(error_message) LIKE '%stream disconnected before completion%'
            OR lower(error_message) LIKE '%error decoding response body%'
            OR lower(error_message) LIKE '%closed before response.completed%'
            OR lower(error_message) LIKE '%closed before response.done%'
            OR lower(error_message) LIKE '%stream ended before completion%'
            OR lower(error_message) LIKE '%incomplete_eof%'
          )
        "#,
        [],
    )?;
    Ok(conn)
}

fn ensure_request_logs_column(
    conn: &Connection,
    column_name: &str,
    column_definition: &str,
) -> Result<(), SqliteError> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('request_logs') WHERE name = ?1",
        params![column_name],
        |row| row.get(0),
    )?;
    if exists == 0 {
        conn.execute(
            format!("ALTER TABLE request_logs ADD COLUMN {column_definition}").as_str(),
            [],
        )?;
    }
    Ok(())
}

fn open_local_access_logs_db() -> Result<Connection, String> {
    let path = local_access_logs_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 API 服务日志目录失败: {}", e))?;
    }
    match open_local_access_logs_db_once(&path) {
        Ok(conn) => Ok(conn),
        Err(error) if is_recoverable_logs_db_error(&error) => {
            quarantine_local_access_logs_db(&path, &error)?;
            open_local_access_logs_db_once(&path)
                .map_err(|e| format!("重建 API 服务日志数据库失败: {}", e))
        }
        Err(error) => Err(format!("打开 API 服务日志数据库失败: {}", error)),
    }
}

fn insert_local_access_usage_event(
    conn: &Connection,
    event: &CodexLocalAccessUsageEvent,
) -> Result<(), String> {
    conn.execute(
        r#"
        INSERT OR IGNORE INTO request_logs (
            event_key,
            timestamp,
            request_id,
            account_id,
            email,
            api_key_id,
            api_key_label,
            model_id,
            gateway_mode,
            request_kind,
            success,
            http_status,
            error_category,
            error_message,
            latency_ms,
            input_tokens,
            output_tokens,
            total_tokens,
            cached_tokens,
            reasoning_tokens,
            estimated_cost_usd,
            input_usd_per_million,
            output_usd_per_million,
            cached_input_usd_per_million
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
        "#,
        params![
            local_access_log_event_key(event),
            event.timestamp,
            event.request_id.trim(),
            event.account_id.trim(),
            event.email.trim(),
            event.api_key_id.trim(),
            event.api_key_label.trim(),
            event.model_id.trim(),
            event
                .gateway_mode
                .map(gateway_mode_to_db_value)
                .unwrap_or_default(),
            request_kind_to_db_value(event.request_kind),
            bool_to_db_value(event.success),
            event.http_status.map(|value| value as i64),
            event.error_category.trim(),
            event.error_message.trim(),
            event.latency_ms as i64,
            event.input_tokens as i64,
            event.output_tokens as i64,
            event.total_tokens as i64,
            event.cached_tokens as i64,
            event.reasoning_tokens as i64,
            event.estimated_cost_usd,
            event.input_usd_per_million,
            event.output_usd_per_million,
            event.cached_input_usd_per_million,
        ],
    )
    .map_err(|e| format!("写入 API 服务请求日志失败: {}", e))?;
    Ok(())
}

fn persist_local_access_usage_event(event: &CodexLocalAccessUsageEvent) -> Result<(), String> {
    let conn = open_local_access_logs_db()?;
    insert_local_access_usage_event(&conn, event)
}

fn migrate_local_access_json_events(events: &[CodexLocalAccessUsageEvent]) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let mut conn = open_local_access_logs_db()?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("开始迁移 API 服务请求日志失败: {}", e))?;
    for event in events {
        insert_local_access_usage_event(&tx, event)?;
    }
    tx.commit()
        .map_err(|e| format!("提交 API 服务请求日志迁移失败: {}", e))?;
    Ok(())
}

fn clear_local_access_usage_events_db() -> Result<(), String> {
    let conn = open_local_access_logs_db()?;
    conn.execute("DELETE FROM request_logs", [])
        .map_err(|e| format!("清空 API 服务请求日志失败: {}", e))?;
    Ok(())
}

fn usage_event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodexLocalAccessUsageEvent> {
    let request_kind: String = row.get("request_kind")?;
    let success: i64 = row.get("success")?;
    let http_status: Option<i64> = row.get("http_status")?;
    let gateway_mode: String = row.get("gateway_mode")?;
    let read_u64 = |name: &str| -> rusqlite::Result<u64> {
        let value: i64 = row.get(name)?;
        Ok(value.max(0) as u64)
    };
    Ok(CodexLocalAccessUsageEvent {
        timestamp: row.get("timestamp")?,
        request_id: row.get("request_id")?,
        account_id: row.get("account_id")?,
        email: row.get("email")?,
        api_key_id: row.get("api_key_id")?,
        api_key_label: row.get("api_key_label")?,
        model_id: row.get("model_id")?,
        gateway_mode: gateway_mode_from_db_value(gateway_mode.as_str()),
        request_kind: request_kind_from_db_value(request_kind.as_str()),
        success: success != 0,
        http_status: http_status.and_then(|value| u16::try_from(value).ok()),
        error_category: row.get("error_category")?,
        error_message: row.get("error_message")?,
        latency_ms: read_u64("latency_ms")?,
        input_tokens: read_u64("input_tokens")?,
        output_tokens: read_u64("output_tokens")?,
        total_tokens: read_u64("total_tokens")?,
        cached_tokens: read_u64("cached_tokens")?,
        reasoning_tokens: read_u64("reasoning_tokens")?,
        estimated_cost_usd: row.get("estimated_cost_usd")?,
        input_usd_per_million: row.get("input_usd_per_million")?,
        output_usd_per_million: row.get("output_usd_per_million")?,
        cached_input_usd_per_million: row.get("cached_input_usd_per_million")?,
    })
}

fn load_local_access_usage_events_since(
    since: i64,
) -> Result<Vec<CodexLocalAccessUsageEvent>, String> {
    let conn = open_local_access_logs_db()?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                timestamp,
                request_id,
                account_id,
                email,
                api_key_id,
                api_key_label,
                model_id,
                gateway_mode,
                request_kind,
                success,
                http_status,
                error_category,
                error_message,
                latency_ms,
                input_tokens,
                output_tokens,
                total_tokens,
                cached_tokens,
                reasoning_tokens,
                estimated_cost_usd,
                input_usd_per_million,
                output_usd_per_million,
                cached_input_usd_per_million
            FROM request_logs
            WHERE timestamp >= ?1
            ORDER BY timestamp ASC, id ASC
            "#,
        )
        .map_err(|e| format!("准备 API 服务日志读取失败: {}", e))?;
    let rows = stmt
        .query_map(params![since], usage_event_from_row)
        .map_err(|e| format!("读取 API 服务日志失败: {}", e))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("解析 API 服务日志失败: {}", e))
}

fn stats_range_since(stats_range: Option<&str>) -> Option<i64> {
    let now = now_ms();
    match stats_range.map(str::trim) {
        Some("daily") => Some(now.saturating_sub(DAY_WINDOW_MS)),
        Some("weekly") => Some(now.saturating_sub(WEEK_WINDOW_MS)),
        Some("monthly") => Some(now.saturating_sub(MONTH_WINDOW_MS)),
        _ => None,
    }
}

fn normalize_log_filter(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn push_like_filter(
    clauses: &mut Vec<String>,
    params: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<String>,
) {
    if let Some(value) = normalize_log_filter(value) {
        clauses.push(clause.to_string());
        params.push(SqlValue::Text(format!("%{}%", value)));
    }
}

fn empty_usage_event_page(page: u32, page_size: u32) -> CodexLocalAccessUsageEventPage {
    CodexLocalAccessUsageEventPage {
        events: Vec::new(),
        total: 0,
        page: page.max(1),
        page_size: page_size.clamp(1, 200),
        total_pages: 1,
    }
}

pub async fn query_local_access_usage_events(
    page: u32,
    page_size: u32,
    stats_range: Option<String>,
    model_query: Option<String>,
    account_query: Option<String>,
    api_key_query: Option<String>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: Option<CodexLocalAccessRequestKind>,
    success: Option<bool>,
    error_category: Option<String>,
) -> Result<CodexLocalAccessUsageEventPage, String> {
    ensure_runtime_loaded_without_start().await?;

    let page_size = page_size.clamp(1, 200);
    let page = page.max(1);
    let mut clauses = Vec::new();
    let mut params = Vec::<SqlValue>::new();

    if let Some(since) = stats_range_since(stats_range.as_deref()) {
        clauses.push("timestamp >= ?".to_string());
        params.push(SqlValue::Integer(since));
    }
    push_like_filter(&mut clauses, &mut params, "model_id LIKE ?", model_query);
    push_like_filter(
        &mut clauses,
        &mut params,
        "(account_id LIKE ? OR email LIKE ?)",
        account_query.clone(),
    );
    if let Some(account_query) = normalize_log_filter(account_query) {
        params.push(SqlValue::Text(format!("%{}%", account_query)));
    }
    push_like_filter(
        &mut clauses,
        &mut params,
        "(api_key_id LIKE ? OR api_key_label LIKE ?)",
        api_key_query.clone(),
    );
    if let Some(api_key_query) = normalize_log_filter(api_key_query) {
        params.push(SqlValue::Text(format!("%{}%", api_key_query)));
    }
    if let Some(gateway_mode) = gateway_mode {
        clauses.push("gateway_mode = ?".to_string());
        params.push(SqlValue::Text(
            gateway_mode_to_db_value(gateway_mode).to_string(),
        ));
    }
    if let Some(request_kind) = request_kind {
        clauses.push("request_kind = ?".to_string());
        params.push(SqlValue::Text(
            request_kind_to_db_value(request_kind).to_string(),
        ));
    }
    if let Some(success) = success {
        clauses.push("success = ?".to_string());
        params.push(SqlValue::Integer(bool_to_db_value(success)));
    }
    push_like_filter(
        &mut clauses,
        &mut params,
        "error_category LIKE ?",
        error_category,
    );

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let conn = match open_local_access_logs_db() {
        Ok(conn) => conn,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务请求日志数据库不可用，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let total_sql = format!("SELECT COUNT(*) FROM request_logs{}", where_sql);
    let total: u64 = match conn.query_row(
        total_sql.as_str(),
        params_from_iter(params.clone()),
        |row| row.get::<_, i64>(0),
    ) {
        Ok(total) => total.max(0) as u64,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "统计 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let total_pages = ((total + page_size as u64 - 1) / page_size as u64)
        .max(1)
        .min(u32::MAX as u64) as u32;
    let page = page.min(total_pages);
    let offset = (page.saturating_sub(1) as u64 * page_size as u64).min(i64::MAX as u64) as i64;
    let mut query_params = params;
    query_params.push(SqlValue::Integer(page_size as i64));
    query_params.push(SqlValue::Integer(offset));
    let list_sql = format!(
        r#"
        SELECT
            timestamp,
            request_id,
            account_id,
            email,
            api_key_id,
            api_key_label,
            model_id,
            gateway_mode,
            request_kind,
            success,
            http_status,
            error_category,
            error_message,
            latency_ms,
            input_tokens,
            output_tokens,
            total_tokens,
            cached_tokens,
            reasoning_tokens,
            estimated_cost_usd,
            input_usd_per_million,
            output_usd_per_million,
            cached_input_usd_per_million
        FROM request_logs{}
        ORDER BY timestamp DESC, id DESC
        LIMIT ? OFFSET ?
        "#,
        where_sql
    );
    let mut stmt = match conn.prepare(list_sql.as_str()) {
        Ok(stmt) => stmt,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "准备 API 服务请求日志查询失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let rows = match stmt.query_map(params_from_iter(query_params), usage_event_from_row) {
        Ok(rows) => rows,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "查询 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };
    let events = match rows.collect::<Result<Vec<_>, _>>() {
        Ok(events) => events,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "解析 API 服务请求日志失败，本次返回空日志列表: {}",
                error
            ));
            return Ok(empty_usage_event_page(page, page_size));
        }
    };

    Ok(CodexLocalAccessUsageEventPage {
        events,
        total,
        page,
        page_size,
        total_pages,
    })
}

fn append_usage_event(
    events: &mut Vec<CodexLocalAccessUsageEvent>,
    now: i64,
    request_id: Option<&str>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    model_id: Option<&str>,
    gateway_mode: Option<CodexLocalAccessGatewayMode>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    http_status: Option<u16>,
    error_category: Option<&str>,
    error_message: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    pricing: Option<&CodexLocalAccessModelPricing>,
    estimated_cost_usd: f64,
) -> CodexLocalAccessUsageEvent {
    let usage = usage.cloned().unwrap_or_default();
    let event = CodexLocalAccessUsageEvent {
        timestamp: now,
        request_id: request_id.unwrap_or_default().trim().to_string(),
        account_id: account_id.unwrap_or_default().trim().to_string(),
        email: account_email.unwrap_or_default().trim().to_string(),
        api_key_id: api_key_id.unwrap_or_default().trim().to_string(),
        api_key_label: api_key_label.unwrap_or_default().trim().to_string(),
        model_id: model_id.unwrap_or_default().trim().to_string(),
        gateway_mode,
        request_kind,
        success,
        http_status,
        error_category: error_category.unwrap_or_default().trim().to_string(),
        error_message: error_message.unwrap_or_default().trim().to_string(),
        latency_ms,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_tokens: usage.cached_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        estimated_cost_usd,
        input_usd_per_million: pricing
            .map(|item| item.input_usd_per_million)
            .unwrap_or_default(),
        output_usd_per_million: pricing
            .map(|item| item.output_usd_per_million)
            .unwrap_or_default(),
        cached_input_usd_per_million: pricing.and_then(|item| item.cached_input_usd_per_million),
    };
    events.push(event.clone());
    event
}

fn apply_usage_event_to_window(
    window: &mut CodexLocalAccessStatsWindow,
    event: &CodexLocalAccessUsageEvent,
) {
    let usage = UsageCapture {
        input_tokens: event.input_tokens,
        output_tokens: event.output_tokens,
        total_tokens: event.total_tokens,
        cached_tokens: event.cached_tokens,
        reasoning_tokens: event.reasoning_tokens,
    };
    apply_usage_stats(
        &mut window.totals,
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
    );
    upsert_account_usage_stats(
        &mut window.accounts,
        Some(event.account_id.as_str()),
        Some(event.email.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_model_usage_stats(
        &mut window.models,
        Some(event.model_id.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    upsert_api_key_usage_stats(
        &mut window.api_keys,
        Some(event.api_key_id.as_str()),
        Some(event.api_key_label.as_str()),
        event.request_kind,
        event.success,
        Some(event.error_category.as_str()),
        event.latency_ms,
        Some(&usage),
        event.estimated_cost_usd,
        event.timestamp,
    );
    window.updated_at = window.updated_at.max(event.timestamp);
}

fn recompute_time_windows(stats: &mut CodexLocalAccessStats, now: i64) {
    let day_since = now.saturating_sub(DAY_WINDOW_MS);
    let week_since = now.saturating_sub(WEEK_WINDOW_MS);
    let month_since = now.saturating_sub(MONTH_WINDOW_MS);

    trim_recent_events(&mut stats.events, month_since);

    let mut daily = empty_stats_window(day_since, stats.updated_at.max(day_since));
    let mut weekly = empty_stats_window(week_since, stats.updated_at.max(week_since));
    let mut monthly = empty_stats_window(month_since, stats.updated_at.max(month_since));

    for event in &stats.events {
        if event.timestamp >= month_since {
            apply_usage_event_to_window(&mut monthly, event);
        }
        if event.timestamp >= week_since {
            apply_usage_event_to_window(&mut weekly, event);
        }
        if event.timestamp >= day_since {
            apply_usage_event_to_window(&mut daily, event);
        }
    }

    sort_usage_accounts(&mut daily.accounts);
    sort_usage_accounts(&mut weekly.accounts);
    sort_usage_accounts(&mut monthly.accounts);
    sort_usage_models(&mut daily.models);
    sort_usage_models(&mut weekly.models);
    sort_usage_models(&mut monthly.models);
    sort_usage_api_keys(&mut daily.api_keys);
    sort_usage_api_keys(&mut weekly.api_keys);
    sort_usage_api_keys(&mut monthly.api_keys);

    stats.daily = daily;
    stats.weekly = weekly;
    stats.monthly = monthly;
}

fn build_api_port_url(port: u16) -> String {
    format!("http://{CODEX_LOCAL_ACCESS_URL_HOST}:{port}{CHAT_COMPLETIONS_PATH}")
}

fn build_base_url(port: u16) -> String {
    format!("http://{CODEX_LOCAL_ACCESS_URL_HOST}:{port}/v1")
}

#[derive(Debug, Clone, Default)]
struct ProfileConfigInspection {
    config_attached: bool,
    model_provider: Option<String>,
    base_url: Option<String>,
    token_matched: bool,
}

fn profile_auth_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CODEX_PROFILE_AUTH_FILE)
}

fn profile_config_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CODEX_PROFILE_CONFIG_FILE)
}

fn normalize_profile_dir_key(profile_dir: &Path) -> String {
    profile_dir
        .to_string_lossy()
        .trim()
        .trim_end_matches(|item| item == '/' || item == '\\')
        .to_string()
}

fn read_optional_profile_file(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read_to_string(path).map(Some).map_err(|e| {
        format!(
            "读取 Codex 配置文件失败: path={}, error={}",
            path.display(),
            e
        )
    })
}

fn write_optional_profile_file(path: &Path, content: Option<&str>) -> Result<(), String> {
    match content {
        Some(content) => write_string_atomic(path, content),
        None => {
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| {
                    format!(
                        "删除 Codex 配置文件失败: path={}, error={}",
                        path.display(),
                        e
                    )
                })?;
            }
            Ok(())
        }
    }
}

fn is_codex_local_access_config(config_text: &str) -> bool {
    let Ok(doc) = config_text.parse::<Document>() else {
        return false;
    };
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
}

fn normalize_profile_base_url_for_match(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let port = parsed
        .port()
        .map(|value| format!(":{}", value))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    Some(format!(
        "{}://{}{}{}",
        parsed.scheme().to_ascii_lowercase(),
        host,
        port,
        path
    ))
}

fn profile_base_url_matches(actual: Option<&str>, expected: &str) -> bool {
    normalize_profile_base_url_for_match(actual)
        .zip(normalize_profile_base_url_for_match(Some(expected)))
        .map(|(actual, expected)| actual == expected)
        .unwrap_or(false)
}

fn inspect_local_access_profile_config(
    config_text: &str,
    expected_base_url: &str,
    expected_api_key: &str,
) -> Result<ProfileConfigInspection, String> {
    let doc = config_text
        .parse::<Document>()
        .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
    let model_provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let provider_table = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID))
        .and_then(|item| item.as_table());
    let base_url = provider_table
        .and_then(|table| table.get("base_url"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let wire_api_matches = provider_table
        .and_then(|table| table.get("wire_api"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some("responses");
    let requires_openai_auth = provider_table
        .and_then(|table| table.get("requires_openai_auth"))
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let token_matched = provider_table
        .and_then(|table| table.get("experimental_bearer_token"))
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(expected_api_key.trim());
    let provider_selected =
        model_provider.as_deref() == Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
    let config_attached = provider_selected
        && provider_table.is_some()
        && profile_base_url_matches(base_url.as_deref(), expected_base_url)
        && wire_api_matches
        && requires_openai_auth
        && token_matched;

    Ok(ProfileConfigInspection {
        config_attached,
        model_provider,
        base_url,
        token_matched,
    })
}

fn inspect_local_access_profile_attachment(
    profile_dir: &Path,
    collection: Option<&CodexLocalAccessCollection>,
) -> CodexLocalAccessProfileAttachment {
    let profile_dir_text = normalize_profile_dir_key(profile_dir);
    let Some(collection) = collection else {
        return CodexLocalAccessProfileAttachment {
            profile_dir: profile_dir_text,
            attached: false,
            config_attached: false,
            auth_attached: false,
            model_provider: None,
            base_url: None,
            expected_base_url: None,
            error: None,
        };
    };

    let expected_base_url = build_base_url(collection.port);
    let expected_api_key = collection.api_key.trim();
    let mut attachment = CodexLocalAccessProfileAttachment {
        profile_dir: profile_dir_text,
        attached: false,
        config_attached: false,
        auth_attached: false,
        model_provider: None,
        base_url: None,
        expected_base_url: Some(expected_base_url.clone()),
        error: None,
    };

    match read_optional_profile_file(&profile_config_path(profile_dir)) {
        Ok(Some(config_text)) => match inspect_local_access_profile_config(
            &config_text,
            &expected_base_url,
            expected_api_key,
        ) {
            Ok(inspection) => {
                attachment.config_attached = inspection.config_attached;
                attachment.model_provider = inspection.model_provider;
                attachment.base_url = inspection.base_url;
                if !inspection.token_matched && attachment.config_attached {
                    attachment.error = Some("Codex API 服务接管密钥不匹配".to_string());
                }
            }
            Err(error) => {
                attachment.error = Some(error);
            }
        },
        Ok(None) => {}
        Err(error) => {
            attachment.error = Some(error);
        }
    }

    match read_optional_profile_file(&profile_auth_path(profile_dir)) {
        Ok(Some(auth_text)) => {
            attachment.auth_attached =
                is_codex_local_access_auth_text(&auth_text, expected_api_key);
        }
        Ok(None) => {}
        Err(error) => {
            attachment.error = Some(match attachment.error.take() {
                Some(existing) => format!("{}；{}", existing, error),
                None => error,
            });
        }
    }

    attachment.attached = attachment.config_attached;
    attachment
}

fn remove_codex_local_access_config(config_text: &str) -> Result<String, String> {
    if config_text.trim().is_empty() {
        return Ok(String::new());
    }

    let mut doc = config_text
        .parse::<Document>()
        .map_err(|e| format!("解析 Codex config.toml 失败: {}", e))?;
    if doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        != Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID)
    {
        return Ok(config_text.to_string());
    }

    let _ = doc.remove("model_provider");
    let should_remove_model_providers = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            let _ = model_providers.remove(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID);
            model_providers.is_empty()
        })
        .unwrap_or(false);
    if should_remove_model_providers {
        let _ = doc.remove("model_providers");
    }

    Ok(doc.to_string())
}

fn is_codex_local_access_auth_text(auth_text: &str, api_key: &str) -> bool {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return false;
    }

    let Ok(value) = serde_json::from_str::<Value>(auth_text) else {
        return false;
    };
    let auth_mode = value
        .get("auth_mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    let openai_api_key = value
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim);

    auth_mode.as_deref() == Some("apikey")
        && openai_api_key
            .map(|key| key == api_key || key.starts_with("agt_codex_"))
            .unwrap_or(false)
}

fn load_takeover_backups() -> Result<CodexLocalAccessTakeoverBackups, String> {
    let path = local_access_takeover_backups_path()?;
    if !path.exists() {
        return Ok(CodexLocalAccessTakeoverBackups {
            version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
            profiles: Vec::new(),
        });
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 Codex API 服务接管备份失败: {}", e))?;
    match serde_json::from_str::<CodexLocalAccessTakeoverBackups>(&content) {
        Ok(mut backups) => {
            backups.version = CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION;
            Ok(backups)
        }
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败，已隔离: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败，文件已不存在: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_codex_api_warn(&format!(
                    "Codex API 服务接管备份解析失败且隔离失败: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(CodexLocalAccessTakeoverBackups {
                version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
                profiles: Vec::new(),
            })
        }
    }
}

fn save_takeover_backups(backups: &CodexLocalAccessTakeoverBackups) -> Result<(), String> {
    let path = local_access_takeover_backups_path()?;
    if backups.profiles.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("删除 Codex API 服务接管备份失败: {}", e))?;
        }
        return Ok(());
    }

    let content = serde_json::to_string_pretty(backups)
        .map_err(|e| format!("序列化 Codex API 服务接管备份失败: {}", e))?;
    write_string_atomic(&path, &content)
        .map_err(|e| format!("写入 Codex API 服务接管备份失败: {}", e))
}

fn save_profile_takeover_backup(profile_dir: &Path) -> Result<(), String> {
    let profile_key = normalize_profile_dir_key(profile_dir);
    if profile_key.is_empty() {
        return Err("Codex API 服务接管目录为空".to_string());
    }

    let config_toml = read_optional_profile_file(&profile_config_path(profile_dir))?;
    let mut backups = load_takeover_backups()?;
    let existing_backup = backups
        .profiles
        .iter_mut()
        .find(|item| item.profile_dir == profile_key);

    if config_toml
        .as_deref()
        .map(is_codex_local_access_config)
        .unwrap_or(false)
    {
        if existing_backup.is_none() {
            logger::log_codex_api_warn(&format!(
                "Codex API 服务接管前发现目标目录已绑定运行时 provider，未把该状态保存为恢复备份: profile_dir={}",
                profile_key
            ));
        }
        return Ok(());
    }

    let auth_json = read_optional_profile_file(&profile_auth_path(profile_dir))?;
    let now = now_ms();
    match existing_backup {
        Some(existing) => {
            existing.auth_json = auth_json;
            existing.config_toml = config_toml;
            existing.updated_at = now;
        }
        None => backups
            .profiles
            .push(CodexLocalAccessProfileTakeoverBackup {
                profile_dir: profile_key,
                auth_json,
                config_toml,
                created_at: now,
                updated_at: now,
            }),
    }

    backups.version = CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION;
    save_takeover_backups(&backups)
}

fn restore_profile_takeover_backup(
    backup: &CodexLocalAccessProfileTakeoverBackup,
    api_key: &str,
) -> Result<bool, String> {
    let profile_dir = PathBuf::from(&backup.profile_dir);
    let config_path = profile_config_path(&profile_dir);
    let auth_path = profile_auth_path(&profile_dir);
    let current_config = read_optional_profile_file(&config_path)?;
    let current_auth = read_optional_profile_file(&auth_path)?;
    let config_is_managed = current_config
        .as_deref()
        .map(is_codex_local_access_config)
        .unwrap_or(false);
    let auth_is_managed = current_auth
        .as_deref()
        .map(|content| is_codex_local_access_auth_text(content, api_key))
        .unwrap_or(false);

    if !config_is_managed && !auth_is_managed {
        return Ok(false);
    }

    write_optional_profile_file(&auth_path, backup.auth_json.as_deref())?;
    write_optional_profile_file(&config_path, backup.config_toml.as_deref())?;
    Ok(true)
}

fn cleanup_profile_takeover_without_backup(
    profile_dir: &Path,
    api_key: &str,
) -> Result<bool, String> {
    let config_path = profile_config_path(profile_dir);
    let auth_path = profile_auth_path(profile_dir);
    let mut changed = false;

    if let Some(config_text) = read_optional_profile_file(&config_path)? {
        if is_codex_local_access_config(&config_text) {
            let cleaned = remove_codex_local_access_config(&config_text)?;
            let cleaned_content = if cleaned.trim().is_empty() {
                None
            } else {
                Some(cleaned)
            };
            write_optional_profile_file(&config_path, cleaned_content.as_deref())?;
            changed = true;
        }
    }

    if let Some(auth_text) = read_optional_profile_file(&auth_path)? {
        if is_codex_local_access_auth_text(&auth_text, api_key) {
            write_optional_profile_file(&auth_path, None)?;
            changed = true;
        }
    }

    Ok(changed)
}

fn restore_takeover_profiles_after_disable(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let backups = load_takeover_backups()?;
    let mut restored_count = 0usize;
    for backup in &backups.profiles {
        if restore_profile_takeover_backup(backup, &collection.api_key)? {
            restored_count += 1;
        }
    }

    save_takeover_backups(&CodexLocalAccessTakeoverBackups {
        version: CODEX_LOCAL_ACCESS_TAKEOVER_BACKUP_VERSION,
        profiles: Vec::new(),
    })?;

    let default_profile = codex_account::get_codex_home();
    let default_key = normalize_profile_dir_key(&default_profile);
    let default_had_backup = backups
        .profiles
        .iter()
        .any(|backup| backup.profile_dir == default_key);
    let cleaned_default_without_backup = if default_had_backup {
        false
    } else {
        cleanup_profile_takeover_without_backup(&default_profile, &collection.api_key)?
    };

    if restored_count > 0 || cleaned_default_without_backup {
        logger::log_codex_api_info(&format!(
            "Codex API 服务停用后已恢复 Live 配置: restored_profiles={}, cleaned_default_without_backup={}",
            restored_count, cleaned_default_without_backup
        ));
    }

    Ok(())
}

fn build_lan_base_url(port: u16) -> Option<String> {
    resolve_primary_lan_ipv4().map(|addr| format!("http://{addr}:{port}/v1"))
}

fn sidecar_config_fingerprint(config_content: &str, manifest_content: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(config_content.as_bytes());
    hasher.update(b"\n--manifest--\n");
    hasher.update(manifest_content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Debug, Clone)]
struct SidecarLaunchConfig {
    config_path: PathBuf,
    manifest_path: PathBuf,
    fingerprint: String,
    proxy_signature: UpstreamHttpClientSignature,
}

#[derive(Debug, Clone, Default)]
struct SidecarReadySignal {
    host: String,
    port: Option<u16>,
}

#[derive(Debug, Clone, Default)]
struct SidecarStartupDiagnostics {
    ready_seen: bool,
    last_stdout: Option<String>,
    last_stderr: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SidecarUsageDetails {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    reasoning_tokens: i64,
    #[serde(default)]
    cached_tokens: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarUsageEvent {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    account_email: String,
    #[serde(default)]
    api_key_id: String,
    #[serde(default)]
    api_key_label: String,
    #[serde(default)]
    request_kind: String,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    status: Option<u16>,
    #[serde(default)]
    error_category: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
    #[serde(default)]
    latency_ms: u64,
    #[serde(default)]
    usage: SidecarUsageDetails,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarAuthResultEvent {
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    account_email: String,
    #[serde(default)]
    request_kind: String,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    http_status: Option<u16>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

fn local_access_sidecar_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_LOCAL_ACCESS_SIDECAR_DIR))
}

fn sidecar_config_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_CONFIG_FILE)
}

fn sidecar_manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_MANIFEST_FILE)
}

fn sidecar_auths_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_LOCAL_ACCESS_SIDECAR_AUTHS_DIR)
}

fn sidecar_binary_file_names() -> Vec<String> {
    let target = env!("COCKPIT_RUST_TARGET");
    if cfg!(target_os = "windows") {
        vec![
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}.exe"),
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}-{target}.exe"),
        ]
    } else {
        vec![
            CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME.to_string(),
            format!("{CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME}-{target}"),
        ]
    }
}

fn push_sidecar_binary_candidates(candidates: &mut Vec<PathBuf>, dir: &Path) {
    for name in sidecar_binary_file_names() {
        let path = dir.join(name);
        if !candidates.iter().any(|candidate| candidate == &path) {
            candidates.push(path);
        }
    }
}

fn sidecar_binary_candidates() -> Result<Vec<PathBuf>, String> {
    let exe = std::env::current_exe().map_err(|e| format!("读取当前程序路径失败: {}", e))?;
    let parent = exe
        .parent()
        .ok_or_else(|| format!("当前程序路径缺少父目录: {}", exe.display()))?;
    let mut candidates = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_sidecar_dir = manifest_dir.join("../sidecars/cockpit-cliproxy/bin");

    if cfg!(debug_assertions) {
        push_sidecar_binary_candidates(&mut candidates, &dev_sidecar_dir);
    }
    push_sidecar_binary_candidates(&mut candidates, parent);
    if let Some(contents_dir) = parent.parent() {
        push_sidecar_binary_candidates(&mut candidates, &contents_dir.join("Resources"));
    }
    if !cfg!(debug_assertions) {
        push_sidecar_binary_candidates(&mut candidates, &dev_sidecar_dir);
    }
    Ok(candidates)
}

fn sidecar_binary_path() -> Result<PathBuf, String> {
    let candidates = sidecar_binary_candidates()?;
    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "API 服务 sidecar 二进制不存在，已检查: {}。请重新构建应用。",
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn sidecar_auth_file_name(account_id: &str) -> String {
    let mut safe = account_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.trim_matches('_').is_empty() {
        safe = uuid::Uuid::new_v4().to_string();
    }
    format!("{safe}.json")
}

fn sidecar_duration_ms(value_ms: i64) -> String {
    format!("{}ms", value_ms.max(1))
}

fn sidecar_disable_image_generation_value(
    mode: CodexLocalAccessImageGenerationMode,
) -> serde_json::Value {
    match mode {
        CodexLocalAccessImageGenerationMode::Enabled => json!(false),
        CodexLocalAccessImageGenerationMode::Disabled => json!(true),
        CodexLocalAccessImageGenerationMode::ImagesOnly => json!("chat"),
    }
}

fn sidecar_routing_strategy_value(strategy: CodexLocalAccessRoutingStrategy) -> &'static str {
    match strategy {
        CodexLocalAccessRoutingStrategy::Auto => "auto",
        CodexLocalAccessRoutingStrategy::QuotaHighFirst => "quota_high_first",
        CodexLocalAccessRoutingStrategy::QuotaLowFirst => "quota_low_first",
        CodexLocalAccessRoutingStrategy::PlanHighFirst => "plan_high_first",
        CodexLocalAccessRoutingStrategy::PlanLowFirst => "plan_low_first",
        CodexLocalAccessRoutingStrategy::ExpirySoonFirst => "expiry_soon_first",
        CodexLocalAccessRoutingStrategy::Custom => "custom",
    }
}

fn sidecar_model_alias_values(collection: &CodexLocalAccessCollection) -> Vec<Value> {
    collection
        .model_aliases
        .iter()
        .map(|alias| {
            json!({
                "name": alias.source_model.clone(),
                "alias": alias.alias.clone(),
                "fork": alias.fork,
            })
        })
        .collect()
}

fn sidecar_codex_key_model_values(collection: &CodexLocalAccessCollection) -> Vec<Value> {
    collection
        .model_aliases
        .iter()
        .map(|alias| {
            json!({
                "name": alias.source_model.clone(),
                "alias": alias.alias.clone(),
            })
        })
        .collect()
}

fn sidecar_api_key_manifest_values(collection: &CodexLocalAccessCollection) -> Vec<Value> {
    let mut values = Vec::new();
    if !collection.api_key.trim().is_empty() {
        values.push(json!({
            "id": "legacy",
            "label": default_local_api_key_label(),
            "key": collection.api_key.trim(),
            "enabled": true,
            "allowedModels": [],
            "excludedModels": [],
        }));
    }
    for item in &collection.api_keys {
        if !item.enabled || item.key.trim().is_empty() {
            continue;
        }
        values.push(json!({
            "id": item.id.clone(),
            "label": item.label.clone(),
            "key": item.key.trim(),
            "modelPrefix": item.model_prefix.clone(),
            "allowedModels": item.allowed_models.clone(),
            "excludedModels": item.excluded_models.clone(),
            "enabled": item.enabled,
        }));
    }
    values
}

fn sidecar_auth_json_for_account(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
) -> Value {
    let account_id = account
        .account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(account.id.as_str());
    let mut value = json!({
        "type": "codex",
        "id_token": account.tokens.id_token.clone(),
        "access_token": account.tokens.access_token.clone(),
        "refresh_token": account.tokens.refresh_token.clone().unwrap_or_default(),
        "account_id": account_id,
        "last_refresh": now_ms().to_string(),
        "email": account.email.clone(),
        "plan_type": account.plan_type.clone(),
        "excluded_models": collection.excluded_models.clone(),
        "disable_cooling": collection.disable_cooling,
    });
    if let Some(proxy_url) = proxy_url {
        value["proxy_url"] = Value::String(proxy_url.to_string());
    }
    value
}

fn sidecar_account_manifest_value(account: &CodexAccount, auth_id: Option<&str>) -> Value {
    json!({
        "id": account.id.clone(),
        "email": account.email.clone(),
        "authId": auth_id,
        "upstreamApiKey": account.openai_api_key.as_deref().unwrap_or_default(),
        "planRank": resolve_plan_rank(account),
        "remainingQuota": resolve_remaining_quota(account),
        "subscriptionExpiryMs": resolve_subscription_expiry_ms(account),
    })
}

fn sidecar_codex_key_config_value(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
) -> Option<Value> {
    let api_key = account.openai_api_key.as_deref()?.trim();
    if api_key.is_empty() {
        return None;
    }
    let base_url = account
        .api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_OPENAI_RESPONSES_BASE_URL);
    let mut value = json!({
        "api-key": api_key,
        "base-url": base_url,
        "proxy-url": proxy_url,
        "models": sidecar_codex_key_model_values(collection),
        "excluded-models": collection.excluded_models.clone(),
        "disable-cooling": collection.disable_cooling,
    });
    if proxy_url.is_none() {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("proxy-url");
        }
    }
    Some(value)
}

fn sidecar_effective_proxy_signature(
    collection: &CodexLocalAccessCollection,
) -> Result<UpstreamHttpClientSignature, String> {
    let signature = current_upstream_http_client_signature(
        collection.upstream_proxy_url.as_deref(),
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    if let Some(proxy_url) = signature.proxy_url.as_deref() {
        Proxy::all(proxy_url).map_err(|e| match signature.proxy_source {
            UpstreamProxySource::ApiService => format!("API 代理地址无效: {}", e),
            UpstreamProxySource::Global => format!("全局代理地址无效: {}", e),
            UpstreamProxySource::SystemEnv => format!("环境代理地址无效: {}", e),
            UpstreamProxySource::SystemAuto => format!("上游代理地址无效: {}", e),
        })?;
    }
    Ok(signature)
}

fn gateway_mode_label(mode: CodexLocalAccessGatewayMode) -> &'static str {
    gateway_mode_to_db_value(mode)
}

fn collection_gateway_mode(collection: &CodexLocalAccessCollection) -> CodexLocalAccessGatewayMode {
    collection.gateway_mode
}

fn log_gateway_mode_info(mode: CodexLocalAccessGatewayMode, message: impl AsRef<str>) {
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][{}] {}",
        gateway_mode_label(mode),
        message.as_ref()
    ));
}

fn log_gateway_mode_warn(mode: CodexLocalAccessGatewayMode, message: impl AsRef<str>) {
    logger::log_codex_api_warn(&format!(
        "[CodexLocalAccess][{}] {}",
        gateway_mode_label(mode),
        message.as_ref()
    ));
}

fn legacy_debug_log(enabled: bool, message: impl AsRef<str>) {
    if !enabled {
        return;
    }

    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][legacy][debug] {}",
        message.as_ref()
    ));
}

fn request_kind_log_label(request_kind: CodexLocalAccessRequestKind) -> &'static str {
    match request_kind {
        CodexLocalAccessRequestKind::Text => "text",
        CodexLocalAccessRequestKind::ImageGeneration => "image_generation",
        CodexLocalAccessRequestKind::ImageEdit => "image_edit",
        CodexLocalAccessRequestKind::Other => "other",
    }
}

fn is_client_disconnect_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection aborted")
        || lower.contains("unexpected eof")
        || lower.contains("客户端已断开")
        || lower.contains("客户端在发送")
}

fn is_client_canceled_error_category(category: &str) -> bool {
    category.trim().eq_ignore_ascii_case("client_canceled")
}

fn is_stream_incomplete_error_category(category: &str) -> bool {
    category.trim().eq_ignore_ascii_case("stream_incomplete")
}

fn is_upstream_response_failed_error_category(category: &str) -> bool {
    category
        .trim()
        .eq_ignore_ascii_case("upstream_response_failed")
}

fn is_upstream_response_failed_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("upstream_response_failed")
        || lower.contains("codex upstream response.failed")
        || lower.contains("last_event=response.failed")
}

fn is_stream_incomplete_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("stream disconnected before completion")
        || lower.contains("error decoding response body")
        || lower.contains("closed before response.completed")
        || lower.contains("closed before response.done")
        || lower.contains("stream ended before completion")
        || lower.contains("incomplete_eof")
}

fn legacy_stream_error_category(message: &str) -> &'static str {
    if is_upstream_response_failed_error_message(message) {
        "upstream_response_failed"
    } else if is_stream_incomplete_error_message(message) {
        "stream_incomplete"
    } else if message.contains("流式响应超时")
        || (message.contains("连续") && message.contains("未收到新数据"))
    {
        "upstream_stream_timeout"
    } else if message.contains("读取上游") {
        "upstream_stream_read_failed"
    } else {
        "stream_write_failed"
    }
}

fn compact_json_for_log(value: &Value) -> String {
    let mut text = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
    const MAX_LEN: usize = 800;
    if text.len() > MAX_LEN {
        text.truncate(MAX_LEN);
        text.push_str("...");
    }
    text
}

fn json_field_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|item| {
        item.as_str()
            .map(str::to_string)
            .or_else(|| item.as_i64().map(|number| number.to_string()))
            .or_else(|| item.as_u64().map(|number| number.to_string()))
            .or_else(|| item.as_f64().map(|number| number.to_string()))
            .or_else(|| item.as_bool().map(|flag| flag.to_string()))
    })
}

fn nested_error_object(value: &Value) -> Option<&Value> {
    value
        .get("response")
        .and_then(|response| response.get("error"))
        .or_else(|| value.get("error"))
        .or_else(|| value.get("last_error"))
}

fn upstream_response_failed_signal(
    event_name: Option<&str>,
    value: &Value,
) -> Option<UpstreamResponseFailedSignal> {
    let value_type = value.get("type").and_then(Value::as_str);
    let event_type = value_type.or(event_name).unwrap_or("").trim();
    if event_type != "response.failed" && event_type != "error" {
        return None;
    }

    let error_value = nested_error_object(value).unwrap_or(value);
    let code = json_field_string(error_value, "code").or_else(|| json_field_string(value, "code"));
    let error_type =
        json_field_string(error_value, "type").or_else(|| json_field_string(value, "error_type"));
    let message = json_field_string(error_value, "message")
        .or_else(|| json_field_string(error_value, "detail"))
        .or_else(|| json_field_string(value, "message"));
    let raw = compact_json_for_log(error_value);

    Some(UpstreamResponseFailedSignal {
        event_type: event_type.to_string(),
        code,
        error_type,
        message,
        raw,
    })
}

fn format_upstream_response_failed_error(signal: &UpstreamResponseFailedSignal) -> String {
    format!(
        "upstream_response_failed: Codex upstream {}: code={} type={} message={} raw={}",
        signal.event_type,
        signal.code.as_deref().unwrap_or("-"),
        signal.error_type.as_deref().unwrap_or("-"),
        signal.message.as_deref().unwrap_or("-"),
        signal.raw
    )
}

async fn start_legacy_gateway_locked(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let bind_host = bind_host_for_collection(collection).to_string();
    let port = collection.port;
    let listener = bind_gateway_listener(&bind_host, port)
        .await
        .map_err(|error| format_gateway_bind_error(&bind_host, port, &error))?;
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    let task = tokio::spawn(async move {
        let listener = listener;
        loop {
            tokio::select! {
                changed = shutdown_receiver.changed() => {
                    if changed.is_ok() && *shutdown_receiver.borrow() {
                        break;
                    }
                    if changed.is_err() {
                        break;
                    }
                }
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, addr)) => {
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, addr).await {
                                    if is_client_disconnect_error_message(&error) {
                                        logger::log_codex_api_info(&format!(
                                            "[CodexLocalAccess][legacy] 客户端已断开，停止写入响应: {}",
                                            error
                                        ));
                                    } else {
                                        logger::log_codex_api_warn(&format!(
                                            "[CodexLocalAccess][legacy] 处理网关请求失败: {}",
                                            error
                                        ));
                                    }
                                }
                            });
                        }
                        Err(error) => {
                            logger::log_codex_api_warn(&format!(
                                "[CodexLocalAccess][legacy] 网关监听 accept 失败: {}",
                                error
                            ));
                            break;
                        }
                    }
                }
            }
        }
    });

    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][legacy] API 服务 legacy 网关已启动: bind={}:{} base={}",
        bind_host,
        port,
        build_base_url(port)
    ));

    let mut runtime = gateway_runtime().lock().await;
    runtime.running = true;
    runtime.actual_port = Some(port);
    runtime.actual_bind_host = Some(bind_host);
    runtime.sidecar_config_fingerprint = None;
    runtime.last_error = None;
    runtime.shutdown_sender = Some(shutdown_sender);
    runtime.task = Some(task);
    runtime.sidecar_child = None;
    Ok(())
}

async fn load_sidecar_account(account_id: &str) -> Option<CodexAccount> {
    match get_prepared_account(account_id).await {
        Ok(account) => Some(account),
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar 准备账号失败，尝试使用本地缓存: account_id={}, error={}",
                account_id, error
            ));
            codex_account::load_account(account_id)
        }
    }
}

async fn prepare_sidecar_launch_config(
    collection: &CodexLocalAccessCollection,
) -> Result<SidecarLaunchConfig, String> {
    let base_dir = local_access_sidecar_dir()?;
    let auths_dir = sidecar_auths_dir(&base_dir);
    if auths_dir.exists() {
        std::fs::remove_dir_all(&auths_dir)
            .map_err(|e| format!("清理 API 服务 sidecar 认证目录失败: {}", e))?;
    }
    std::fs::create_dir_all(&auths_dir)
        .map_err(|e| format!("创建 API 服务 sidecar 认证目录失败: {}", e))?;

    let proxy_signature = sidecar_effective_proxy_signature(collection)?;
    let effective_proxy_url_ref = proxy_signature.proxy_url.as_deref();

    let mut manifest_accounts = Vec::new();
    let mut codex_keys = Vec::new();
    for account_id in &collection.account_ids {
        let Some(account) = load_sidecar_account(account_id).await else {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar 跳过不存在账号: account_id={}",
                account_id
            ));
            continue;
        };
        if !is_local_access_eligible_account(&account, collection.restrict_free_accounts) {
            continue;
        }

        if account.is_api_key_auth() {
            if let Some(config_value) =
                sidecar_codex_key_config_value(&account, collection, effective_proxy_url_ref)
            {
                codex_keys.push(config_value);
                manifest_accounts.push(sidecar_account_manifest_value(&account, None));
            } else {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess][sidecar] 跳过缺少上游 API Key 的 API Key 账号: account_id={}",
                    account.id
                ));
            }
            continue;
        }

        let file_name = sidecar_auth_file_name(&account.id);
        let auth_path = auths_dir.join(&file_name);
        let auth_json =
            sidecar_auth_json_for_account(&account, collection, effective_proxy_url_ref);
        let auth_content = serde_json::to_string_pretty(&auth_json)
            .map_err(|e| format!("序列化 sidecar Codex OAuth 认证失败: {}", e))?;
        write_string_atomic(&auth_path, &auth_content)?;
        manifest_accounts.push(sidecar_account_manifest_value(&account, Some(&file_name)));
    }

    let health_snapshot = {
        let runtime = gateway_runtime().lock().await;
        runtime.account_health.clone()
    };
    let model_ids = visible_codex_model_ids_for_collection(collection, Some(&health_snapshot));
    let manifest = json!({
        "apiKeys": sidecar_api_key_manifest_values(collection),
        "accounts": manifest_accounts,
        "modelIds": model_ids,
        "modelAliases": collection.model_aliases.iter().map(|alias| json!({
            "sourceModel": alias.source_model.clone(),
            "alias": alias.alias.clone(),
            "fork": alias.fork,
        })).collect::<Vec<_>>(),
        "excludedModels": collection.excluded_models.clone(),
        "routingStrategy": sidecar_routing_strategy_value(collection.routing_strategy),
        "customRoutingRules": collection.custom_routing_rules.iter().map(|rule| json!({
            "accountId": rule.account_id.clone(),
            "priority": rule.priority,
            "weight": rule.weight,
        })).collect::<Vec<_>>(),
        "debugLogs": collection.debug_logs,
    });

    let mut config = Map::new();
    config.insert(
        "host".to_string(),
        json!(bind_host_for_collection(collection)),
    );
    config.insert("port".to_string(), json!(collection.port));
    config.insert(
        "auth-dir".to_string(),
        json!(auths_dir.to_string_lossy().to_string()),
    );
    config.insert("debug".to_string(), json!(collection.debug_logs));
    config.insert("request-log".to_string(), json!(false));
    config.insert("logging-to-file".to_string(), json!(false));
    config.insert("commercial-mode".to_string(), json!(true));
    config.insert("ws-auth".to_string(), json!(true));
    config.insert(
        "disable-image-generation".to_string(),
        sidecar_disable_image_generation_value(collection.image_generation_mode),
    );
    config.insert(
        "request-retry".to_string(),
        json!(MAX_REQUEST_RETRY_ATTEMPTS as i32),
    );
    let timeouts = collection_timeouts(collection);
    config.insert(
        "streaming".to_string(),
        json!({
            "keepalive-seconds": timeouts.sidecar_stream_keepalive_seconds,
            "bootstrap-retries": timeouts.sidecar_streaming_bootstrap_retries,
            "stream-open-timeout-ms": timeouts.sidecar_stream_open_timeout_ms,
            "stream-idle-timeout-ms": timeouts.sidecar_stream_idle_timeout_ms,
            "image-stream-open-timeout-ms": timeouts.sidecar_image_stream_open_timeout_ms,
            "image-stream-idle-timeout-ms": timeouts.sidecar_image_stream_idle_timeout_ms,
            "stream-open-max-attempts": timeouts.sidecar_stream_open_max_attempts,
        }),
    );
    config.insert(
        "max-retry-credentials".to_string(),
        json!(collection.max_retry_credentials as i32),
    );
    config.insert(
        "max-retry-interval".to_string(),
        json!(((collection.max_retry_interval_ms + 999) / 1000) as i32),
    );
    config.insert(
        "disable-cooling".to_string(),
        json!(collection.disable_cooling),
    );
    config.insert(
        "routing".to_string(),
        json!({
            "strategy": "round-robin",
            "session-affinity": collection.session_affinity,
            "session-affinity-ttl": sidecar_duration_ms(collection.session_affinity_ttl_ms),
        }),
    );
    if let Some(proxy_url) = effective_proxy_url_ref {
        config.insert("proxy-url".to_string(), json!(proxy_url));
    }
    if !codex_keys.is_empty() {
        config.insert("codex-api-key".to_string(), Value::Array(codex_keys));
    }
    if !collection.excluded_models.is_empty() {
        config.insert(
            "oauth-excluded-models".to_string(),
            json!({ "codex": collection.excluded_models.clone() }),
        );
    }
    if !collection.model_aliases.is_empty() {
        config.insert(
            "oauth-model-alias".to_string(),
            json!({ "codex": sidecar_model_alias_values(collection) }),
        );
    }
    config.insert(
        "codex-header-defaults".to_string(),
        json!({
            "user-agent": DEFAULT_CODEX_USER_AGENT,
            "beta-features": CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE,
        }),
    );

    let config_path = sidecar_config_path(&base_dir);
    let manifest_path = sidecar_manifest_path(&base_dir);
    let config_content = serde_json::to_string_pretty(&Value::Object(config))
        .map_err(|e| format!("序列化 sidecar 配置失败: {}", e))?;
    let manifest_content = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("序列化 sidecar manifest 失败: {}", e))?;
    let fingerprint = sidecar_config_fingerprint(&config_content, &manifest_content);
    write_string_atomic(&config_path, &config_content)?;
    write_string_atomic(&manifest_path, &manifest_content)?;

    Ok(SidecarLaunchConfig {
        config_path,
        manifest_path,
        fingerprint,
        proxy_signature,
    })
}

fn parse_sidecar_request_kind(value: &str) -> CodexLocalAccessRequestKind {
    match value.trim() {
        "text" => CodexLocalAccessRequestKind::Text,
        "image_generation" => CodexLocalAccessRequestKind::ImageGeneration,
        "image_edit" => CodexLocalAccessRequestKind::ImageEdit,
        _ => CodexLocalAccessRequestKind::Other,
    }
}

fn usage_i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn sidecar_usage_capture(details: &SidecarUsageDetails) -> Option<UsageCapture> {
    let usage = UsageCapture {
        input_tokens: usage_i64_to_u64(details.input_tokens),
        output_tokens: usage_i64_to_u64(details.output_tokens),
        total_tokens: usage_i64_to_u64(details.total_tokens),
        cached_tokens: usage_i64_to_u64(details.cached_tokens),
        reasoning_tokens: usage_i64_to_u64(details.reasoning_tokens),
    };
    if usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.total_tokens == 0
        && usage.cached_tokens == 0
        && usage.reasoning_tokens == 0
    {
        None
    } else {
        Some(usage)
    }
}

fn non_empty_sidecar_string(value: &str) -> Option<String> {
    Some(value.trim().to_string()).filter(|value| !value.is_empty())
}

fn sidecar_usage_event_is_client_canceled(event: &SidecarUsageEvent) -> bool {
    if event.error_category.as_deref() == Some("client_canceled") {
        return true;
    }
    event
        .error_message
        .as_deref()
        .map(str::to_ascii_lowercase)
        .map(|message| {
            message.contains("context canceled")
                || message.contains("client canceled")
                || message.contains("client disconnected")
                || message.contains("client closed")
                || is_client_disconnect_error_message(&message)
        })
        .unwrap_or(false)
}

fn normalized_sidecar_error_category(event: &SidecarUsageEvent) -> Option<String> {
    if sidecar_usage_event_is_client_canceled(event) {
        return Some("client_canceled".to_string());
    }
    if event
        .error_message
        .as_deref()
        .map(is_upstream_response_failed_error_message)
        .unwrap_or(false)
    {
        return Some("upstream_response_failed".to_string());
    }
    if event
        .error_message
        .as_deref()
        .map(is_stream_incomplete_error_message)
        .unwrap_or(false)
    {
        return Some("stream_incomplete".to_string());
    }
    event
        .error_category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn update_sidecar_account_health(event: &SidecarUsageEvent) {
    update_sidecar_account_health_from_values(
        event.account_id.as_str(),
        event.account_email.as_str(),
        event.request_kind.as_str(),
        event.success,
        event.status,
        event.error_category.as_deref(),
        event.error_message.as_deref(),
        sidecar_usage_event_is_client_canceled(event),
    )
    .await;
}

async fn update_sidecar_auth_result_health(event: &SidecarAuthResultEvent) {
    let is_client_canceled = event
        .error_message
        .as_deref()
        .map(str::to_ascii_lowercase)
        .map(|message| {
            message.contains("context canceled")
                || message.contains("client canceled")
                || message.contains("client disconnected")
                || message.contains("client closed")
        })
        .unwrap_or(false);
    update_sidecar_account_health_from_values(
        event.account_id.as_str(),
        event.account_email.as_str(),
        event.request_kind.as_str(),
        event.success,
        event.http_status,
        event.error_code.as_deref(),
        event.error_message.as_deref(),
        is_client_canceled,
    )
    .await;
}

async fn update_sidecar_account_health_from_values(
    account_id: &str,
    account_email: &str,
    request_kind: &str,
    success: bool,
    status: Option<u16>,
    error_category: Option<&str>,
    error_message: Option<&str>,
    is_client_canceled: bool,
) {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return;
    }
    let request_kind = parse_sidecar_request_kind(request_kind);
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account_id.to_string())
        .or_default();
    if !account_email.trim().is_empty() {
        health.email = account_email.trim().to_string();
    }
    if success {
        health.consecutive_failures = 0;
        health.last_success_at = Some(now);
        if request_kind_is_image(request_kind) {
            health.image_generation_status = CodexLocalAccessImageGenerationStatus::Available;
            health.image_generation_checked_at = Some(now);
        }
        return;
    }
    if is_client_canceled {
        return;
    }

    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
    health.last_failure_at = Some(now);
    health.last_failure_status = status;
    health.last_failure_category = error_category
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    health.last_failure_message = error_message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if error_category == Some("image_generation_not_enabled") {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Unavailable;
        health.image_generation_checked_at = Some(now);
    } else if request_kind_is_image(request_kind)
        && health.image_generation_status == CodexLocalAccessImageGenerationStatus::Unknown
    {
        health.image_generation_checked_at = Some(now);
    }
}

async fn record_sidecar_usage_event(event: SidecarUsageEvent) {
    update_sidecar_account_health(&event).await;
    let account_id = non_empty_sidecar_string(&event.account_id);
    let account_email = non_empty_sidecar_string(&event.account_email);
    let api_key_id = non_empty_sidecar_string(&event.api_key_id);
    let api_key_label = non_empty_sidecar_string(&event.api_key_label);
    let model = non_empty_sidecar_string(&event.model);
    let request_id = non_empty_sidecar_string(&event.request_id);
    let error_category = normalized_sidecar_error_category(&event);
    if let Err(error) = record_request_stats_with_meta(
        account_id.as_deref(),
        account_email.as_deref(),
        api_key_id.as_deref(),
        api_key_label.as_deref(),
        model.as_deref(),
        parse_sidecar_request_kind(&event.request_kind),
        event.success,
        error_category.as_deref(),
        event.latency_ms,
        sidecar_usage_capture(&event.usage),
        RequestStatsMeta {
            request_id: request_id.as_deref(),
            http_status: event.status,
            error_message: event.error_message.as_deref(),
        },
    )
    .await
    {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 写入 sidecar 请求统计失败: {}",
            error
        ));
    }
}

type SharedSidecarStartupDiagnostics = Arc<Mutex<SidecarStartupDiagnostics>>;

fn update_sidecar_stdout_diagnostics(
    diagnostics: &SharedSidecarStartupDiagnostics,
    line: &str,
    ready_seen: bool,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.last_stdout = clean_diagnostic_text(line);
        if ready_seen {
            diagnostics.ready_seen = true;
        }
    }
}

fn update_sidecar_stderr_diagnostics(diagnostics: &SharedSidecarStartupDiagnostics, line: &str) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.last_stderr = clean_diagnostic_text(line);
    }
}

fn sidecar_startup_diagnostics_text(diagnostics: &SharedSidecarStartupDiagnostics) -> String {
    let diagnostics = diagnostics.lock().ok().map(|item| item.clone());
    let Some(diagnostics) = diagnostics else {
        return "startup_diagnostics_unavailable".to_string();
    };
    format!(
        "ready_seen={}, last_stdout={}, last_stderr={}",
        diagnostics.ready_seen,
        diagnostics
            .last_stdout
            .as_deref()
            .unwrap_or("未捕获 stdout"),
        diagnostics
            .last_stderr
            .as_deref()
            .unwrap_or("未捕获 stderr")
    )
}

fn sidecar_ready_signal_from_value(value: &Value) -> SidecarReadySignal {
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok());
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    SidecarReadySignal { host, port }
}

async fn handle_sidecar_stdout_line(
    line: &str,
    ready_sender: &mut Option<oneshot::Sender<SidecarReadySignal>>,
    diagnostics: &SharedSidecarStartupDiagnostics,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    update_sidecar_stdout_diagnostics(diagnostics, trimmed, false);
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        return;
    };
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "usage" => match serde_json::from_value::<SidecarUsageEvent>(value) {
            Ok(event) => record_sidecar_usage_event(event).await,
            Err(error) => logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] sidecar usage 事件解析失败: {}",
                error
            )),
        },
        "auth_result" => {
            match serde_json::from_value::<SidecarAuthResultEvent>(value.clone()) {
                Ok(event) => update_sidecar_auth_result_health(&event).await,
                Err(error) => logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] sidecar auth_result 事件解析失败: {}",
                    error
                )),
            }
            logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        }
        "ready" => {
            update_sidecar_stdout_diagnostics(diagnostics, trimmed, true);
            if let Some(sender) = ready_sender.take() {
                let _ = sender.send(sidecar_ready_signal_from_value(&value));
            }
            logger::log_codex_api_info(&format!("[CodexLocalAccess] sidecar 已就绪: {}", trimmed));
        }
        "error" => {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(trimmed)
                .to_string();
            {
                let mut runtime = gateway_runtime().lock().await;
                runtime.last_error = Some(message.clone());
            }
            logger::log_codex_api_warn(&format!("[CodexLocalAccess] sidecar 错误: {}", message));
        }
        _ => {
            logger::log_codex_api_info(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
        }
    }
}

async fn drain_sidecar_stdout(
    stdout: tokio::process::ChildStdout,
    ready_sender: oneshot::Sender<SidecarReadySignal>,
    diagnostics: SharedSidecarStartupDiagnostics,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut ready_sender = Some(ready_sender);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                handle_sidecar_stdout_line(&line, &mut ready_sender, &diagnostics).await
            }
            Ok(None) => break,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 读取 sidecar stdout 失败: {}",
                    error
                ));
                break;
            }
        }
    }
}

async fn drain_sidecar_stderr(
    stderr: tokio::process::ChildStderr,
    diagnostics: SharedSidecarStartupDiagnostics,
) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    update_sidecar_stderr_diagnostics(&diagnostics, trimmed);
                    logger::log_codex_api_warn(&format!("[CodexLocalAccess][sidecar] {}", trimmed));
                }
            }
            Ok(None) => break,
            Err(error) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 读取 sidecar stderr 失败: {}",
                    error
                ));
                break;
            }
        }
    }
}

async fn wait_for_sidecar_ready(
    ready_receiver: &mut oneshot::Receiver<SidecarReadySignal>,
    child: &mut Child,
) -> Result<SidecarReadySignal, String> {
    let started_at = Instant::now();

    loop {
        let Some(remaining) = SIDECAR_READY_TIMEOUT.checked_sub(started_at.elapsed()) else {
            return Err("API 服务 sidecar 启动后未收到 ready 事件".to_string());
        };
        if remaining.is_zero() {
            return Err("API 服务 sidecar 启动后未收到 ready 事件".to_string());
        }

        let poll_interval = remaining.min(Duration::from_millis(120));
        tokio::select! {
            result = &mut *ready_receiver => {
                return result.map_err(|_| "API 服务 sidecar stdout 在 ready 前关闭".to_string());
            }
            _ = tokio::time::sleep(poll_interval) => {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return Err(format!("API 服务 sidecar 在 ready 前退出: {}", status));
                    }
                    Ok(None) => {}
                    Err(error) => {
                        return Err(format!("检查 API 服务 sidecar ready 状态失败: {}", error));
                    }
                }
            }
        }
    }
}

async fn probe_sidecar_ready_once(
    collection: &CodexLocalAccessCollection,
    request_timeout: Duration,
) -> Result<(), String> {
    let url = format!(
        "http://{}:{}/v1/models",
        CODEX_LOCAL_ACCESS_URL_HOST, collection.port
    );
    let client = build_localhost_http_client(request_timeout, "sidecar 健康检测")?;
    match client
        .get(&url)
        .bearer_auth(collection.api_key.trim())
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => Err(format!("HTTP {}", response.status())),
        Err(error) => Err(error.to_string()),
    }
}

fn bind_host_for_access_scope(scope: CodexLocalAccessScope) -> &'static str {
    match scope {
        CodexLocalAccessScope::Localhost => CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST,
        CodexLocalAccessScope::Lan => CODEX_LOCAL_ACCESS_LAN_BIND_HOST,
    }
}

fn bind_host_for_collection(collection: &CodexLocalAccessCollection) -> &'static str {
    bind_host_for_access_scope(collection.access_scope)
}

#[derive(Debug)]
struct LanIpv4Candidate {
    interface_name: String,
    addr: Ipv4Addr,
}

fn resolve_primary_lan_ipv4() -> Option<Ipv4Addr> {
    let mut candidates = collect_private_lan_ipv4_candidates();
    candidates.sort_by_key(|candidate| {
        (
            lan_interface_score(&candidate.interface_name),
            lan_addr_score(candidate.addr),
            candidate.addr.octets(),
        )
    });
    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.addr)
}

fn is_lan_ipv4(addr: Ipv4Addr) -> bool {
    addr.is_private()
}

fn lan_interface_score(interface_name: &str) -> u8 {
    let name = interface_name.to_ascii_lowercase();
    if name.starts_with("en")
        || name.starts_with("eth")
        || name.starts_with("wlan")
        || name.starts_with("wi-fi")
        || name.starts_with("wifi")
        || name.starts_with("ethernet")
        || name.contains("wireless")
    {
        return 0;
    }
    if name.starts_with("lo")
        || name.starts_with("utun")
        || name.starts_with("tun")
        || name.starts_with("tap")
        || name.starts_with("awdl")
        || name.starts_with("llw")
        || name.starts_with("bridge")
        || name.starts_with("br-")
        || name.starts_with("docker")
        || name.starts_with("veth")
        || name.starts_with("virbr")
        || name.starts_with("vmnet")
        || name.starts_with("vbox")
        || name.starts_with("tailscale")
        || name.starts_with("wg")
    {
        return 2;
    }
    1
}

fn lan_addr_score(addr: Ipv4Addr) -> u8 {
    let octets = addr.octets();
    if octets[0] == 192 && octets[1] == 168 {
        return 0;
    }
    if octets[0] == 10 {
        return 1;
    }
    2
}

#[cfg(target_os = "macos")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let output = StdCommand::new("ifconfig").arg("-a").output();
    match output {
        Ok(output) => parse_ifconfig_ipv4_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "linux")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let output = StdCommand::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    match output {
        Ok(output) => parse_linux_ip_addr_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "windows")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let mut command = StdCommand::new("ipconfig");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    match command.output() {
        Ok(output) => parse_windows_ipconfig_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn parse_ifconfig_ipv4_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    let mut current_interface = String::new();
    for line in output.lines() {
        if !line
            .chars()
            .next()
            .map(|item| item.is_whitespace())
            .unwrap_or(false)
        {
            current_interface = line
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            continue;
        }
        let mut parts = line.split_whitespace();
        while let Some(part) = parts.next() {
            if part != "inet" {
                continue;
            }
            let Some(raw_addr) = parts.next() else {
                continue;
            };
            if let Ok(addr) = raw_addr.parse::<Ipv4Addr>() {
                if is_lan_ipv4(addr) {
                    candidates.push(LanIpv4Candidate {
                        interface_name: current_interface.clone(),
                        addr,
                    });
                }
            }
        }
    }
    candidates
}

#[cfg(target_os = "linux")]
fn parse_linux_ip_addr_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let _index = parts.next();
        let Some(interface_name) = parts.next() else {
            continue;
        };
        while let Some(part) = parts.next() {
            if part != "inet" {
                continue;
            }
            let Some(raw_addr) = parts.next() else {
                continue;
            };
            let addr_text = raw_addr.split('/').next().unwrap_or_default();
            if let Ok(addr) = addr_text.parse::<Ipv4Addr>() {
                if is_lan_ipv4(addr) {
                    candidates.push(LanIpv4Candidate {
                        interface_name: interface_name.trim_end_matches(':').to_string(),
                        addr,
                    });
                }
            }
        }
    }
    candidates
}

#[cfg(target_os = "windows")]
fn parse_windows_ipconfig_candidates(output: &str) -> Vec<LanIpv4Candidate> {
    let mut candidates = Vec::new();
    let mut current_interface = String::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let is_indented = line
            .chars()
            .next()
            .map(|item| item.is_whitespace())
            .unwrap_or(false);
        if trimmed.ends_with(':') && !is_indented {
            current_interface = trimmed.trim_end_matches(':').to_string();
            continue;
        }
        if !trimmed.contains("IPv4") {
            continue;
        }
        let Some(raw_addr) = trimmed.rsplit(':').next() else {
            continue;
        };
        if let Ok(addr) = raw_addr.trim().parse::<Ipv4Addr>() {
            if is_lan_ipv4(addr) {
                candidates.push(LanIpv4Candidate {
                    interface_name: current_interface.clone(),
                    addr,
                });
            }
        }
    }
    candidates
}

fn build_runtime_account(
    base_url: String,
    api_key: String,
    bound_oauth_account_id: Option<String>,
) -> CodexAccount {
    let mut runtime_account = CodexAccount::new_api_key(
        "codex_local_access_runtime".to_string(),
        "api-service-local".to_string(),
        api_key,
        CodexApiProviderMode::Custom,
        Some(base_url),
        Some(CODEX_LOCAL_ACCESS_RUNTIME_PROVIDER_ID.to_string()),
        Some("Codex API Service".to_string()),
    );
    runtime_account.account_name = Some("API Service".to_string());
    runtime_account.bound_oauth_account_id = bound_oauth_account_id;
    runtime_account
}

async fn write_local_access_profile_takeover(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }
    let runtime_account = build_runtime_account(
        build_base_url(collection.port),
        collection.api_key.clone(),
        bound_oauth_account_id,
    );
    codex_account::write_account_bundle_to_dir(profile_dir, &runtime_account)
}

fn push_local_access_takeover_dir(
    dirs: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
    profile_dir: PathBuf,
) {
    let profile_key = normalize_profile_dir_key(&profile_dir);
    if profile_key.is_empty() || !seen.insert(profile_key) {
        return;
    }
    dirs.push(profile_dir);
}

fn collect_local_access_profile_takeover_dirs_from_store(
    store: crate::models::InstanceStore,
    default_profile: PathBuf,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    if store
        .default_settings
        .bind_account_id
        .as_deref()
        .map(crate::modules::codex_instance::is_api_service_bind_account_id)
        .unwrap_or(false)
    {
        push_local_access_takeover_dir(&mut dirs, &mut seen, default_profile);
    }

    for instance in store.instances {
        let Some(bind_account_id) = instance.bind_account_id.as_deref() else {
            continue;
        };
        if !crate::modules::codex_instance::is_api_service_bind_account_id(bind_account_id) {
            continue;
        }
        let profile_text = instance.user_data_dir.trim();
        if profile_text.is_empty() {
            continue;
        }
        push_local_access_takeover_dir(&mut dirs, &mut seen, PathBuf::from(profile_text));
    }

    dirs
}

fn collect_local_access_profile_takeover_dirs() -> Vec<PathBuf> {
    let store = match crate::modules::codex_instance::load_instance_store() {
        Ok(store) => store,
        Err(err) => {
            logger::log_codex_api_warn(&format!(
                "Codex API 服务加载 Codex 多开实例失败，跳过自动接管配置: {}",
                err
            ));
            return Vec::new();
        }
    };

    collect_local_access_profile_takeover_dirs_from_store(store, codex_account::get_codex_home())
}

async fn ensure_profile_takeover(
    profile_dir: &Path,
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    if !collection.enabled {
        return Ok(());
    }

    let current = inspect_local_access_profile_attachment(profile_dir, Some(collection));
    if current.attached && current.error.is_none() {
        return Ok(());
    }

    save_profile_takeover_backup(profile_dir)?;
    write_local_access_profile_takeover(profile_dir, collection).await?;

    let next = inspect_local_access_profile_attachment(profile_dir, Some(collection));
    if !next.attached {
        return Err(format!(
            "Codex API 服务已启动，但 Codex 配置未接管本地 API: profile_dir={}, expected_base_url={}",
            next.profile_dir,
            next.expected_base_url.unwrap_or_else(|| build_base_url(collection.port))
        ));
    }

    let attached_base_url = next
        .base_url
        .clone()
        .or(next.expected_base_url.clone())
        .unwrap_or_default();
    logger::log_codex_api_info(&format!(
        "Codex API 服务已接管 Codex 配置: profile_dir={} base={}",
        next.profile_dir, attached_base_url
    ));
    Ok(())
}

async fn ensure_local_access_profile_takeovers(
    collection: &CodexLocalAccessCollection,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for profile_dir in collect_local_access_profile_takeover_dirs() {
        if let Err(err) = ensure_profile_takeover(&profile_dir, collection).await {
            failures.push(err);
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

async fn ensure_local_access_profile_takeovers_from_runtime() -> Result<(), String> {
    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .filter(|collection| collection.enabled)
    };

    if let Some(collection) = collection.as_ref() {
        ensure_local_access_profile_takeovers(collection).await?;
    }
    Ok(())
}

fn generate_local_api_key() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    format!("agt_codex_{}", suffix)
}

fn generate_local_api_key_id() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();
    format!("key_{}", suffix)
}

fn default_local_api_key_label() -> String {
    "Default".to_string()
}

fn normalize_api_key_label(label: Option<&str>, fallback: &str) -> String {
    let normalized = label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .trim()
        .to_string();
    if normalized.is_empty() {
        default_local_api_key_label()
    } else {
        normalized
    }
}

fn build_local_access_api_key(label: Option<&str>) -> CodexLocalAccessApiKey {
    let now = now_ms();
    CodexLocalAccessApiKey {
        id: generate_local_api_key_id(),
        label: normalize_api_key_label(label, &default_local_api_key_label()),
        key: generate_local_api_key(),
        model_prefix: None,
        allowed_models: Vec::new(),
        excluded_models: Vec::new(),
        enabled: true,
        created_at: now,
        updated_at: now,
        last_used_at: None,
    }
}

fn normalize_collection_api_keys(collection: &mut CodexLocalAccessCollection) -> bool {
    let mut changed = false;
    let now = now_ms();

    if collection.api_keys.is_empty() {
        let key = if collection.api_key.trim().is_empty() {
            generate_local_api_key()
        } else {
            collection.api_key.trim().to_string()
        };
        collection.api_keys.push(CodexLocalAccessApiKey {
            id: generate_local_api_key_id(),
            label: default_local_api_key_label(),
            key,
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
            enabled: true,
            created_at: now,
            updated_at: now,
            last_used_at: None,
        });
        changed = true;
    }

    let mut seen_ids = HashSet::new();
    let mut seen_keys = HashSet::new();
    let mut normalized = Vec::new();
    for mut item in std::mem::take(&mut collection.api_keys) {
        let key = item.key.trim().to_string();
        if key.is_empty() || !seen_keys.insert(key.clone()) {
            changed = true;
            continue;
        }
        item.key = key;
        if item.id.trim().is_empty() || !seen_ids.insert(item.id.trim().to_string()) {
            item.id = generate_local_api_key_id();
            changed = true;
        } else {
            item.id = item.id.trim().to_string();
        }
        let normalized_label = normalize_api_key_label(Some(item.label.as_str()), "API Key");
        if normalized_label != item.label {
            item.label = normalized_label;
            changed = true;
        }
        if item.created_at <= 0 {
            item.created_at = now;
            changed = true;
        }
        if item.updated_at <= 0 {
            item.updated_at = now;
            changed = true;
        }
        let normalized_model_prefix = normalize_model_prefix_value(item.model_prefix.clone());
        if normalized_model_prefix != item.model_prefix {
            item.model_prefix = normalized_model_prefix;
            changed = true;
        }
        let original_allowed_models = std::mem::take(&mut item.allowed_models);
        let normalized_allowed_models = normalize_model_rule_list(original_allowed_models.clone());
        if normalized_allowed_models != original_allowed_models {
            item.allowed_models = normalized_allowed_models;
            changed = true;
        } else {
            item.allowed_models = original_allowed_models;
        }
        let original_excluded_models = std::mem::take(&mut item.excluded_models);
        let normalized_excluded_models =
            normalize_model_rule_list(original_excluded_models.clone());
        if normalized_excluded_models != original_excluded_models {
            item.excluded_models = normalized_excluded_models;
            changed = true;
        } else {
            item.excluded_models = original_excluded_models;
        }
        normalized.push(item);
    }

    if normalized.is_empty() {
        normalized.push(build_local_access_api_key(Some(
            &default_local_api_key_label(),
        )));
        changed = true;
    }

    let primary_key = normalized
        .iter()
        .find(|item| item.enabled)
        .or_else(|| normalized.first())
        .map(|item| item.key.clone())
        .unwrap_or_else(generate_local_api_key);
    if collection.api_key != primary_key {
        collection.api_key = primary_key;
        changed = true;
    }

    collection.api_keys = normalized;
    changed
}

fn resolve_collection_api_key(
    collection: &CodexLocalAccessCollection,
    api_key: &str,
) -> Option<ResolvedLocalApiKey> {
    let normalized = api_key.trim();
    if normalized.is_empty() {
        return None;
    }
    collection
        .api_keys
        .iter()
        .find(|item| item.enabled && item.key == normalized)
        .map(|item| ResolvedLocalApiKey {
            id: item.id.clone(),
            label: item.label.clone(),
            model_prefix: item.model_prefix.clone(),
            allowed_models: item.allowed_models.clone(),
            excluded_models: item.excluded_models.clone(),
        })
        .or_else(|| {
            if collection.api_key == normalized {
                Some(ResolvedLocalApiKey {
                    id: "legacy".to_string(),
                    label: default_local_api_key_label(),
                    model_prefix: None,
                    allowed_models: Vec::new(),
                    excluded_models: Vec::new(),
                })
            } else {
                None
            }
        })
}

fn allocate_random_local_port(bind_host: &str) -> Result<u16, String> {
    let listener =
        StdTcpListener::bind((bind_host, 0)).map_err(|e| format!("分配本地接入端口失败: {}", e))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("读取本地接入端口失败: {}", e))
}

fn configured_initial_local_access_port() -> Option<u16> {
    if let Ok(raw) = std::env::var(CODEX_LOCAL_ACCESS_API_PORT_ENV) {
        if let Ok(port) = raw.trim().parse::<u16>() {
            if port > 0 {
                return Some(port);
            }
        }
    }

    if account::is_dev_profile() {
        return Some(CODEX_LOCAL_ACCESS_DEV_DEFAULT_PORT);
    }

    None
}

fn allocate_initial_local_port(bind_host: &str) -> Result<u16, String> {
    configured_initial_local_access_port()
        .map(Ok)
        .unwrap_or_else(|| allocate_random_local_port(bind_host))
}

fn load_collection_from_disk() -> Result<Option<CodexLocalAccessCollection>, String> {
    let path = local_access_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取本地接入配置失败: {}", e))?;
    match serde_json::from_str::<CodexLocalAccessCollection>(&content) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            match crate::modules::atomic_write::quarantine_file(&path, "invalid-json") {
                Ok(Some(backup_path)) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，已隔离并使用默认关闭配置: path={}, backup={}, error={}",
                    path.display(),
                    backup_path.display(),
                    error
                )),
                Ok(None) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，文件已不存在，使用默认关闭配置: path={}, error={}",
                    path.display(),
                    error
                )),
                Err(backup_error) => logger::log_codex_api_warn(&format!(
                    "本地接入配置解析失败，隔离失败，使用默认关闭配置: path={}, parse_error={}, backup_error={}",
                    path.display(),
                    error,
                    backup_error
                )),
            }
            Ok(None)
        }
    }
}

fn save_collection_to_disk(collection: &CodexLocalAccessCollection) -> Result<(), String> {
    let path = local_access_file_path()?;
    let content = serde_json::to_string_pretty(collection)
        .map_err(|e| format!("序列化本地接入配置失败: {}", e))?;
    write_string_atomic(&path, &content)
}

fn normalize_stats(stats: &mut CodexLocalAccessStats) {
    let now = now_ms();
    if stats.since <= 0 {
        stats.since = now;
    }
    if stats.updated_at <= 0 {
        stats.updated_at = stats.since;
    }
    sort_usage_accounts(&mut stats.accounts);
    sort_usage_models(&mut stats.models);
    sort_usage_api_keys(&mut stats.api_keys);
    recompute_time_windows(stats, now);
}

fn invalid_stats_backup_path(path: &Path) -> PathBuf {
    let timestamp = chrono::Utc::now().timestamp_millis();
    let file_name = path
        .file_name()
        .and_then(|item| item.to_str())
        .unwrap_or(CODEX_LOCAL_ACCESS_STATS_FILE);
    path.with_file_name(format!("{}.invalid-{}", file_name, timestamp))
}

fn recover_invalid_stats_file(
    path: &Path,
    parse_error: &serde_json::Error,
) -> CodexLocalAccessStats {
    let empty = empty_stats_snapshot();
    let backup_path = invalid_stats_backup_path(path);
    match std::fs::rename(path, &backup_path) {
        Ok(()) => {
            logger::log_codex_api_warn(&format!(
                "API 服务统计文件解析失败，已隔离并重建空统计: path={}, backup={}, error={}",
                path.display(),
                backup_path.display(),
                parse_error
            ));
        }
        Err(rename_error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务统计文件解析失败，隔离失败，尝试直接重建空统计: path={}, backup={}, parse_error={}, rename_error={}",
                path.display(),
                backup_path.display(),
                parse_error,
                rename_error
            ));
            match serde_json::to_string_pretty(&empty) {
                Ok(content) => {
                    if let Err(write_error) = write_string_atomic(path, &content) {
                        logger::log_codex_api_warn(&format!(
                            "API 服务统计文件重建失败，本次启动使用空统计: path={}, error={}",
                            path.display(),
                            write_error
                        ));
                    }
                }
                Err(serialize_error) => {
                    logger::log_codex_api_warn(&format!(
                        "API 服务空统计序列化失败，本次启动使用内存空统计: path={}, error={}",
                        path.display(),
                        serialize_error
                    ));
                }
            }
        }
    }
    empty
}

fn load_stats_from_disk() -> Result<CodexLocalAccessStats, String> {
    let path = local_access_stats_file_path()?;
    let mut parsed = if path.exists() {
        let content =
            std::fs::read_to_string(&path).map_err(|e| format!("读取 API 服务统计失败: {}", e))?;
        match serde_json::from_str::<CodexLocalAccessStats>(&content) {
            Ok(parsed) => parsed,
            Err(error) => recover_invalid_stats_file(&path, &error),
        }
    } else {
        empty_stats_snapshot()
    };
    let json_events = std::mem::take(&mut parsed.events);
    if let Err(error) = migrate_local_access_json_events(&json_events) {
        logger::log_codex_api_warn(&format!(
            "API 服务请求日志迁移失败，继续使用统计快照中的最近事件: {}",
            error
        ));
    }
    let month_since = now_ms().saturating_sub(MONTH_WINDOW_MS);
    parsed.events = match load_local_access_usage_events_since(month_since) {
        Ok(events) => events,
        Err(error) => {
            logger::log_codex_api_warn(&format!(
                "API 服务请求日志读取失败，继续使用统计快照中的最近事件: {}",
                error
            ));
            json_events
                .into_iter()
                .filter(|event| event.timestamp >= month_since)
                .collect()
        }
    };
    normalize_stats(&mut parsed);
    Ok(parsed)
}

fn save_stats_to_disk(stats: &CodexLocalAccessStats) -> Result<(), String> {
    let path = local_access_stats_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 API 服务统计目录失败: {}", e))?;
    }
    let mut snapshot = stats.clone();
    snapshot.events.clear();
    let content = serde_json::to_string_pretty(&snapshot)
        .map_err(|e| format!("序列化 API 服务统计失败: {}", e))?;
    write_string_atomic(&path, &content)
}

fn prune_runtime_routing_state(runtime: &mut GatewayRuntime, now: i64) {
    let session_affinity_ttl_ms = runtime
        .collection
        .as_ref()
        .map(|collection| {
            collection
                .session_affinity_ttl_ms
                .clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS)
        })
        .unwrap_or(DEFAULT_SESSION_AFFINITY_TTL_MS);
    runtime.response_affinity.retain(|key, binding| {
        let ttl_ms = if key.starts_with("session:") {
            session_affinity_ttl_ms
        } else {
            RESPONSE_AFFINITY_TTL_MS
        };
        now.saturating_sub(binding.updated_at_ms) <= ttl_ms
    });
    runtime
        .model_cooldowns
        .retain(|_, cooldown| cooldown.next_retry_at_ms > now);

    if runtime.response_affinity.len() <= MAX_RESPONSE_AFFINITY_BINDINGS {
        return;
    }

    let mut bindings: Vec<(String, i64)> = runtime
        .response_affinity
        .iter()
        .map(|(response_id, binding)| (response_id.clone(), binding.updated_at_ms))
        .collect();
    bindings.sort_by_key(|(_, updated_at_ms)| *updated_at_ms);

    let remove_count = runtime
        .response_affinity
        .len()
        .saturating_sub(MAX_RESPONSE_AFFINITY_BINDINGS);
    for (response_id, _) in bindings.into_iter().take(remove_count) {
        runtime.response_affinity.remove(&response_id);
    }
}

async fn resolve_affinity_account(previous_response_id: &str) -> Option<String> {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime
        .response_affinity
        .get(previous_response_id)
        .map(|binding| binding.account_id.clone())
}

async fn bind_response_affinity(response_id: &str, account_id: &str) {
    let response_id = response_id.trim();
    let account_id = account_id.trim();
    if response_id.is_empty() || account_id.is_empty() {
        return;
    }

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime.response_affinity.insert(
        response_id.to_string(),
        ResponseAffinityBinding {
            account_id: account_id.to_string(),
            updated_at_ms: now,
        },
    );
    prune_runtime_routing_state(&mut runtime, now);
}

fn session_affinity_binding_key(value: &str) -> String {
    format!("session:{}", value.trim())
}

fn extract_body_string_path(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor
        .as_str()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
}

fn extract_session_affinity_key(request: &ParsedRequest) -> Option<String> {
    for header in [
        "session_id",
        "x-session-id",
        "x-client-request-id",
        "x-amp-thread-id",
    ] {
        if let Some(value) = request
            .headers
            .get(header)
            .map(String::as_str)
            .map(str::trim)
            .filter(|item| !item.is_empty())
        {
            return Some(format!("{}={}", header, value));
        }
    }

    let body = parse_request_body_json(&request.body)?;
    extract_body_string_path(&body, &["metadata", "user_id"])
        .or_else(|| extract_body_string_path(&body, &["conversation_id"]))
        .or_else(|| extract_body_string_path(&body, &["thread_id"]))
        .map(|value| format!("body={}", value))
}

fn header_value<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .get(&name.to_ascii_lowercase())
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn stable_uuid_from_text(value: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn stable_prompt_cache_key(api_key: &ResolvedLocalApiKey) -> String {
    stable_uuid_from_text(&format!("agtools:codex:prompt-cache:{}", api_key.id))
}

fn extract_prompt_cache_key_from_value(value: &Value) -> Option<String> {
    value
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_prompt_cache_key(
    headers: &HashMap<String, String>,
    body_value: Option<&Value>,
    api_key: &ResolvedLocalApiKey,
) -> String {
    body_value
        .and_then(extract_prompt_cache_key_from_value)
        .or_else(|| header_value(headers, "session_id").map(str::to_string))
        .unwrap_or_else(|| stable_prompt_cache_key(api_key))
}

fn ensure_request_header(headers: &mut HashMap<String, String>, name: &str, value: &str) {
    headers
        .entry(name.to_ascii_lowercase())
        .or_insert_with(|| value.to_string());
}

fn apply_codex_official_headers(request: &mut ParsedRequest) {
    if !(is_responses_request(&request.target) || is_responses_compact_request(&request.target)) {
        return;
    }

    for header in CODEX_OFFICIAL_EMPTY_HEADERS {
        ensure_request_header(&mut request.headers, header, "");
    }
}

fn align_codex_prompt_cache(
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
) -> Result<Option<String>, String> {
    if !(is_responses_request(&request.target) || is_responses_compact_request(&request.target)) {
        return Ok(None);
    }

    let mut body_value = parse_request_body_json(&request.body);
    let session_id = resolve_prompt_cache_key(&request.headers, body_value.as_ref(), api_key);
    request
        .headers
        .insert("session_id".to_string(), session_id.clone());
    request
        .headers
        .insert("conversation_id".to_string(), session_id.clone());

    if let Some(Value::Object(body_obj)) = body_value.as_mut() {
        body_obj.insert(
            "prompt_cache_key".to_string(),
            Value::String(session_id.clone()),
        );
        request.body = serde_json::to_vec(body_value.as_ref().unwrap())
            .map_err(|e| format!("序列化 prompt_cache_key 请求体失败: {}", e))?;
    }

    Ok(Some(session_id))
}

async fn touch_local_access_api_key(api_key_id: &str) {
    let api_key_id = api_key_id.trim();
    if api_key_id.is_empty() || api_key_id == "legacy" {
        return;
    }
    let mut collection_to_save = None;
    {
        let mut runtime = gateway_runtime().lock().await;
        let Some(collection) = runtime.collection.as_mut() else {
            return;
        };
        if let Some(api_key) = collection
            .api_keys
            .iter_mut()
            .find(|item| item.id == api_key_id)
        {
            let now = now_ms();
            api_key.last_used_at = Some(now);
            api_key.updated_at = now;
            collection.updated_at = now;
            collection_to_save = Some(collection.clone());
        }
    }
    if let Some(collection) = collection_to_save {
        if let Err(err) = save_collection_to_disk(&collection) {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 更新 API Key 最近使用时间失败: {}",
                err
            ));
        }
    }
}

async fn clear_model_cooldown(account_id: &str, model_key: &str) {
    let Some(cooldown_key) = build_cooldown_key(account_id, model_key) else {
        return;
    };

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime.model_cooldowns.remove(&cooldown_key);
}

async fn set_model_cooldown(
    account_id: &str,
    model_key: &str,
    retry_after: Duration,
    reason: &str,
) {
    let Some(cooldown_key) = build_cooldown_key(account_id, model_key) else {
        return;
    };
    if retry_after <= Duration::ZERO {
        return;
    }

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let next_retry_at_ms = now.saturating_add(retry_after.as_millis() as i64);
    prune_runtime_routing_state(&mut runtime, now);
    runtime.model_cooldowns.insert(
        cooldown_key,
        AccountModelCooldown {
            model_key: model_key.trim().to_string(),
            next_retry_at_ms,
            reason: reason.trim().to_string(),
        },
    );
}

async fn mark_account_success(account: &CodexAccount, request_kind: CodexLocalAccessRequestKind) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account.id.clone())
        .or_default();
    health.email = account.email.clone();
    health.consecutive_failures = 0;
    health.last_success_at = Some(now);
    if request_kind_is_image(request_kind) {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Available;
        health.image_generation_checked_at = Some(now);
    }
}

async fn mark_account_failure(
    account: &CodexAccount,
    status: Option<u16>,
    category: Option<&str>,
    message: &str,
    request_kind: CodexLocalAccessRequestKind,
) {
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    let health = runtime
        .account_health
        .entry(account.id.clone())
        .or_default();
    health.email = account.email.clone();
    health.consecutive_failures = health.consecutive_failures.saturating_add(1);
    health.last_failure_at = Some(now);
    health.last_failure_status = status;
    health.last_failure_category = category.map(str::to_string);
    health.last_failure_message =
        Some(message.trim().to_string()).filter(|value| !value.is_empty());
    if category == Some("image_generation_not_enabled") {
        health.image_generation_status = CodexLocalAccessImageGenerationStatus::Unavailable;
        health.image_generation_checked_at = Some(now);
    } else if request_kind_is_image(request_kind)
        && health.image_generation_status == CodexLocalAccessImageGenerationStatus::Unknown
    {
        health.image_generation_checked_at = Some(now);
    }
}

async fn get_model_cooldown_wait(account_id: &str, model_key: &str) -> Option<Duration> {
    let cooldown_key = build_cooldown_key(account_id, model_key)?;
    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    let cooldown = runtime.model_cooldowns.get(&cooldown_key)?;
    let wait_ms = cooldown.next_retry_at_ms.saturating_sub(now);
    if wait_ms <= 0 {
        return None;
    }
    Some(Duration::from_millis(wait_ms as u64))
}

fn ensure_local_port_available(
    bind_host: &str,
    port: u16,
    current_port: Option<u16>,
) -> Result<(), String> {
    if port == 0 {
        return Err("端口必须在 1 到 65535 之间".to_string());
    }
    if current_port == Some(port) {
        return Ok(());
    }
    let listener = StdTcpListener::bind((bind_host, port))
        .map_err(|e| format!("端口 {} 不可用: {}", port, e))?;
    drop(listener);
    Ok(())
}

fn is_local_access_port_bindable(bind_host: &str, port: u16) -> Result<bool, std::io::Error> {
    match StdTcpListener::bind((bind_host, port)) {
        Ok(listener) => {
            drop(listener);
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Ok(false),
        Err(error) => Err(error),
    }
}

async fn wait_for_gateway_port_release(bind_host: &str, port: u16) -> Result<(), String> {
    let deadline = Instant::now() + GATEWAY_PORT_RELEASE_TIMEOUT;

    loop {
        match is_local_access_port_bindable(bind_host, port) {
            Ok(true) => return Ok(()),
            Ok(false) if Instant::now() < deadline => {
                tokio::time::sleep(GATEWAY_PORT_RELEASE_POLL_INTERVAL).await;
            }
            Ok(false) => {
                return Err(format!("API 服务端口 {} 停止后仍未释放，请稍后重试", port));
            }
            Err(error) => {
                return Err(format!(
                    "检查 API 服务端口 {} 释放状态失败: {}",
                    port, error
                ));
            }
        }
    }
}

async fn bind_gateway_listener(bind_host: &str, port: u16) -> Result<TcpListener, std::io::Error> {
    let deadline = Instant::now() + GATEWAY_PORT_RELEASE_TIMEOUT;

    loop {
        match TcpListener::bind((bind_host, port)).await {
            Ok(listener) => return Ok(listener),
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrInUse && Instant::now() < deadline =>
            {
                tokio::time::sleep(GATEWAY_PORT_RELEASE_POLL_INTERVAL).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn format_gateway_bind_error(bind_host: &str, port: u16, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        return format!(
            "启动本地接入服务失败: {}:{} 已被占用，请先清理端口或改用其他端口（{}）",
            bind_host, port, error
        );
    }
    format!("启动本地接入服务失败: {}", error)
}

fn is_free_plan_type(plan_type: Option<&str>) -> bool {
    let Some(plan_type) = plan_type else {
        return false;
    };
    let normalized = plan_type.trim().to_ascii_lowercase();
    !normalized.is_empty() && normalized.contains("free")
}

fn is_local_access_eligible_account(account: &CodexAccount, restrict_free_accounts: bool) -> bool {
    if restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref()) {
        return false;
    }
    true
}

fn normalize_upstream_proxy_url(upstream_proxy_url: Option<String>) -> Option<String> {
    upstream_proxy_url
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<Option<String>, String> {
    let normalized = normalize_upstream_proxy_url(upstream_proxy_url);
    if let Some(proxy_url) = normalized.as_deref() {
        Proxy::all(proxy_url).map_err(|e| format!("API 代理地址无效: {}", e))?;
    }
    Ok(normalized)
}

fn clamp_timeout_ms(value: u64, fallback: u64, max: u64) -> u64 {
    let base = if value == 0 { fallback } else { value };
    base.clamp(LOCAL_ACCESS_TIMEOUT_MIN_MS, max)
}

fn clamp_retry_delay_ms(value: u64, fallback: u64) -> u64 {
    let base = if value == 0 { fallback } else { value };
    base.clamp(
        LOCAL_ACCESS_RETRY_DELAY_MIN_MS,
        LOCAL_ACCESS_RETRY_DELAY_MAX_MS,
    )
}

fn normalize_timeouts(timeouts: &mut CodexLocalAccessTimeouts) -> bool {
    let original = timeouts.clone();
    let defaults = CodexLocalAccessTimeouts::default();
    timeouts.legacy_request_read_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_request_read_timeout_ms,
        defaults.legacy_request_read_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_upstream_connect_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_upstream_connect_timeout_ms,
        defaults.legacy_upstream_connect_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_stream_idle_timeout_ms,
        defaults.legacy_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.legacy_stream_total_timeout_ms = clamp_timeout_ms(
        timeouts.legacy_stream_total_timeout_ms,
        defaults.legacy_stream_total_timeout_ms,
        LEGACY_STREAM_TOTAL_TIMEOUT_MAX_MS,
    );
    if timeouts.legacy_stream_total_timeout_ms < timeouts.legacy_stream_idle_timeout_ms {
        timeouts.legacy_stream_total_timeout_ms = timeouts.legacy_stream_idle_timeout_ms;
    }
    timeouts.sidecar_stream_open_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_stream_open_timeout_ms,
        defaults.sidecar_stream_open_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_stream_idle_timeout_ms,
        defaults.sidecar_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_image_stream_open_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_image_stream_open_timeout_ms,
        defaults.sidecar_image_stream_open_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_image_stream_idle_timeout_ms = clamp_timeout_ms(
        timeouts.sidecar_image_stream_idle_timeout_ms,
        defaults.sidecar_image_stream_idle_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.sidecar_stream_open_max_attempts = timeouts.sidecar_stream_open_max_attempts.clamp(
        SIDECAR_STREAM_OPEN_ATTEMPTS_MIN,
        SIDECAR_STREAM_OPEN_ATTEMPTS_MAX,
    );
    timeouts.sidecar_stream_keepalive_seconds = timeouts.sidecar_stream_keepalive_seconds.clamp(
        SIDECAR_STREAM_KEEPALIVE_MIN_SECONDS,
        SIDECAR_STREAM_KEEPALIVE_MAX_SECONDS,
    );
    timeouts.websocket_connect_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_connect_timeout_ms,
        defaults.websocket_connect_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_initial_message_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_initial_message_timeout_ms,
        defaults.websocket_initial_message_timeout_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_idle_timeout_ms = clamp_timeout_ms(
        timeouts.websocket_idle_timeout_ms,
        defaults.websocket_idle_timeout_ms,
        WEBSOCKET_IDLE_TIMEOUT_MAX_MS,
    );
    timeouts.websocket_heartbeat_interval_ms = clamp_timeout_ms(
        timeouts.websocket_heartbeat_interval_ms,
        defaults.websocket_heartbeat_interval_ms,
        LOCAL_ACCESS_TIMEOUT_MAX_MS,
    );
    timeouts.upstream_send_retry_attempts = timeouts.upstream_send_retry_attempts.clamp(
        LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
        LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
    );
    timeouts.upstream_send_retry_base_delay_ms = clamp_retry_delay_ms(
        timeouts.upstream_send_retry_base_delay_ms,
        defaults.upstream_send_retry_base_delay_ms,
    );
    timeouts.upstream_send_retry_max_delay_ms = clamp_retry_delay_ms(
        timeouts.upstream_send_retry_max_delay_ms,
        defaults.upstream_send_retry_max_delay_ms,
    );
    if timeouts.upstream_send_retry_max_delay_ms < timeouts.upstream_send_retry_base_delay_ms {
        timeouts.upstream_send_retry_max_delay_ms = timeouts.upstream_send_retry_base_delay_ms;
    }
    timeouts.single_account_status_retry_attempts =
        timeouts.single_account_status_retry_attempts.clamp(
            LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
            LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
        );
    timeouts.single_account_status_retry_base_delay_ms = clamp_retry_delay_ms(
        timeouts.single_account_status_retry_base_delay_ms,
        defaults.single_account_status_retry_base_delay_ms,
    );
    timeouts.single_account_status_retry_max_delay_ms = clamp_retry_delay_ms(
        timeouts.single_account_status_retry_max_delay_ms,
        defaults.single_account_status_retry_max_delay_ms,
    );
    if timeouts.single_account_status_retry_max_delay_ms
        < timeouts.single_account_status_retry_base_delay_ms
    {
        timeouts.single_account_status_retry_max_delay_ms =
            timeouts.single_account_status_retry_base_delay_ms;
    }
    timeouts.sidecar_streaming_bootstrap_retries =
        timeouts.sidecar_streaming_bootstrap_retries.clamp(
            LOCAL_ACCESS_RETRY_ATTEMPTS_MIN,
            LOCAL_ACCESS_RETRY_ATTEMPTS_MAX,
        );
    *timeouts != original
}

fn collection_timeouts(collection: &CodexLocalAccessCollection) -> CodexLocalAccessTimeouts {
    let mut timeouts = collection.timeouts.clone();
    normalize_timeouts(&mut timeouts);
    timeouts
}

fn normalize_timeout_preset_name(name: &str) -> String {
    name.trim()
        .chars()
        .take(TIMEOUT_PRESET_NAME_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

fn normalize_timeout_preset_id(id: &str) -> Option<String> {
    let normalized = id.trim();
    if normalized.is_empty()
        || normalized == BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID
        || normalized == BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID
    {
        return None;
    }
    Some(normalized.to_string())
}

fn normalize_timeout_presets(presets: &mut Vec<CodexLocalAccessTimeoutPreset>) -> bool {
    let original = presets.clone();
    let now = now_ms();
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();

    for mut preset in std::mem::take(presets) {
        if normalized.len() >= MAX_CUSTOM_TIMEOUT_PRESETS {
            break;
        }
        let Some(id) = normalize_timeout_preset_id(&preset.id) else {
            continue;
        };
        if !seen.insert(id.clone()) {
            continue;
        }
        let name = normalize_timeout_preset_name(&preset.name);
        if name.is_empty() {
            continue;
        }
        normalize_timeouts(&mut preset.timeouts);
        preset.id = id;
        preset.name = name;
        if preset.created_at <= 0 {
            preset.created_at = now;
        }
        if preset.updated_at <= 0 {
            preset.updated_at = preset.created_at;
        }
        normalized.push(preset);
    }

    *presets = normalized;
    *presets != original
}

fn normalize_active_timeout_preset_id(collection: &mut CodexLocalAccessCollection) -> bool {
    let original = collection.active_timeout_preset_id.clone();
    let current = collection.active_timeout_preset_id.trim();
    let normalized = if current == BUILTIN_TIMEOUT_PRESET_SHORT_WAIT_ID
        || collection
            .timeout_presets
            .iter()
            .any(|preset| preset.id == current)
    {
        current.to_string()
    } else {
        BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID.to_string()
    };
    collection.active_timeout_preset_id = normalized;
    collection.active_timeout_preset_id != original
}

fn sanitize_collection(
    collection: &mut CodexLocalAccessCollection,
) -> Result<(bool, HashSet<String>), String> {
    let mut changed = false;

    if collection.port == 0 {
        collection.port = allocate_initial_local_port(bind_host_for_collection(collection))?;
        changed = true;
    }
    if collection.api_key.trim().is_empty() {
        collection.api_key = generate_local_api_key();
        changed = true;
    }
    changed |= normalize_collection_api_keys(collection);
    if collection.created_at <= 0 {
        collection.created_at = now_ms();
        changed = true;
    }
    if collection.updated_at <= 0 {
        collection.updated_at = now_ms();
        changed = true;
    }
    let normalized_upstream_proxy_url =
        normalize_upstream_proxy_url(collection.upstream_proxy_url.clone());
    if normalized_upstream_proxy_url != collection.upstream_proxy_url {
        collection.upstream_proxy_url = normalized_upstream_proxy_url;
        changed = true;
    }

    let accounts = codex_account::list_accounts_checked()?;
    let valid_bound_oauth_account_ids: HashSet<String> = accounts
        .iter()
        .filter(|account| !account.is_api_key_auth())
        .map(|account| account.id.clone())
        .collect();
    let valid_account_ids: HashSet<String> = accounts
        .into_iter()
        .filter(|account| {
            is_local_access_eligible_account(account, collection.restrict_free_accounts)
        })
        .map(|account| account.id)
        .collect();

    let normalized_bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if normalized_bound_oauth_account_id != collection.bound_oauth_account_id {
        collection.bound_oauth_account_id = normalized_bound_oauth_account_id;
        changed = true;
    }
    if let Some(bound_id) = collection.bound_oauth_account_id.as_deref() {
        if !valid_bound_oauth_account_ids.contains(bound_id) {
            collection.bound_oauth_account_id = None;
            changed = true;
        }
    }

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for account_id in &collection.account_ids {
        if !valid_account_ids.contains(account_id) {
            changed = true;
            continue;
        }
        if !seen.insert(account_id.clone()) {
            changed = true;
            continue;
        }
        deduped.push(account_id.clone());
    }
    if deduped != collection.account_ids {
        collection.account_ids = deduped;
        changed = true;
    }

    let original_custom_routing_rules = std::mem::take(&mut collection.custom_routing_rules);
    let normalized_custom_routing_rules = normalize_custom_routing_rules(
        original_custom_routing_rules.clone(),
        &collection.account_ids,
    );
    if normalized_custom_routing_rules != original_custom_routing_rules {
        changed = true;
    }
    collection.custom_routing_rules = normalized_custom_routing_rules;

    let original_model_aliases = std::mem::take(&mut collection.model_aliases);
    let normalized_model_aliases = normalize_model_aliases(original_model_aliases.clone());
    if normalized_model_aliases != original_model_aliases {
        changed = true;
    }
    collection.model_aliases = normalized_model_aliases;

    let original_model_pricings = std::mem::take(&mut collection.model_pricings);
    let normalized_model_pricings = normalize_model_pricings(original_model_pricings.clone());
    if normalized_model_pricings != original_model_pricings {
        changed = true;
    }
    collection.model_pricings = normalized_model_pricings;

    let original_excluded_models = std::mem::take(&mut collection.excluded_models);
    let normalized_excluded_models = normalize_model_rule_list(original_excluded_models.clone());
    if normalized_excluded_models != original_excluded_models {
        changed = true;
    }
    collection.excluded_models = normalized_excluded_models;

    let normalized_session_affinity_ttl_ms = collection
        .session_affinity_ttl_ms
        .clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
    if normalized_session_affinity_ttl_ms != collection.session_affinity_ttl_ms {
        collection.session_affinity_ttl_ms = normalized_session_affinity_ttl_ms;
        changed = true;
    }
    let normalized_max_retry_credentials = collection
        .max_retry_credentials
        .min(MAX_RETRY_CREDENTIALS_PER_REQUEST as u16);
    if normalized_max_retry_credentials != collection.max_retry_credentials {
        collection.max_retry_credentials = normalized_max_retry_credentials;
        changed = true;
    }
    let normalized_max_retry_interval_ms = collection
        .max_retry_interval_ms
        .clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS);
    if normalized_max_retry_interval_ms != collection.max_retry_interval_ms {
        collection.max_retry_interval_ms = normalized_max_retry_interval_ms;
        changed = true;
    }
    changed |= normalize_timeouts(&mut collection.timeouts);
    changed |= normalize_timeout_presets(&mut collection.timeout_presets);
    changed |= normalize_active_timeout_preset_id(collection);

    Ok((changed, valid_account_ids))
}

async fn ensure_runtime_loaded_without_start() -> Result<(), String> {
    {
        let runtime = gateway_runtime().lock().await;
        if runtime.loaded {
            return Ok(());
        }
    }

    let loaded_collection = load_collection_from_disk()?;
    let mut loaded_stats = load_stats_from_disk()?;
    let mut next_collection = loaded_collection;
    let mut persist_after_load = false;

    if next_collection.is_none() {
        next_collection = Some(CodexLocalAccessCollection {
            enabled: false,
            port: allocate_initial_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
            api_key: generate_local_api_key(),
            api_keys: Vec::new(),
            access_scope: CodexLocalAccessScope::Localhost,
            image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
            gateway_mode: CodexLocalAccessGatewayMode::default(),
            upstream_proxy_url: None,
            routing_strategy: CodexLocalAccessRoutingStrategy::default(),
            custom_routing_rules: Vec::new(),
            model_aliases: Vec::new(),
            model_pricings: Vec::new(),
            excluded_models: Vec::new(),
            session_affinity: false,
            session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
            max_retry_credentials: 0,
            max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            timeouts: CodexLocalAccessTimeouts::default(),
            active_timeout_preset_id: "long_wait".to_string(),
            timeout_presets: Vec::new(),
            disable_cooling: false,
            restrict_free_accounts: true,
            debug_logs: true,
            bound_oauth_account_id: None,
            account_ids: Vec::new(),
            created_at: now_ms(),
            updated_at: now_ms(),
        });
        persist_after_load = true;
    }

    if let Some(collection) = next_collection.as_mut() {
        let (changed, _) = sanitize_collection(collection)?;
        persist_after_load = persist_after_load || changed;
    }

    if persist_after_load {
        if let Some(collection) = next_collection.as_ref() {
            save_collection_to_disk(collection)?;
        }
    }

    if let Some(collection) = next_collection.as_ref() {
        if !collection.enabled {
            if let Err(err) = restore_takeover_profiles_after_disable(collection) {
                logger::log_codex_api_warn(&format!(
                    "Codex API 服务处于停用状态，但恢复 Live 配置失败: {}",
                    err
                ));
            }
        }
    }

    {
        let mut runtime = gateway_runtime().lock().await;
        normalize_stats(&mut loaded_stats);
        runtime.stats_dirty = false;
        runtime.stats_flush_inflight = false;
        runtime.stats = loaded_stats;
        if let Some(collection) = next_collection.clone() {
            sync_runtime_collection(&mut runtime, collection);
        } else {
            runtime.loaded = true;
            runtime.collection = None;
            runtime.last_error = None;
            prune_prepared_account_cache(&mut runtime, now_ms());
        }
    }

    Ok(())
}

async fn ensure_runtime_loaded() -> Result<(), String> {
    ensure_runtime_loaded_without_start().await?;

    let should_start = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(|collection| collection.enabled)
            .unwrap_or(false)
    };

    if should_start {
        ensure_gateway_matches_runtime().await?;
        ensure_local_access_profile_takeovers_from_runtime().await?;
    }

    Ok(())
}

async fn ensure_gateway_matches_runtime() -> Result<(), String> {
    let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
    ensure_gateway_matches_runtime_locked().await
}

async fn ensure_gateway_matches_runtime_locked() -> Result<(), String> {
    let (collection, running, actual_port, actual_bind_host, actual_fingerprint, stale_task) = {
        let mut runtime = gateway_runtime().lock().await;
        if runtime.running {
            if let Some(child) = runtime.sidecar_child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let message = format!("API 服务 sidecar 已退出: {}", status);
                        log_gateway_mode_warn(CodexLocalAccessGatewayMode::Sidecar, &message);
                        runtime.running = false;
                        runtime.actual_port = None;
                        runtime.actual_bind_host = None;
                        runtime.sidecar_config_fingerprint = None;
                        runtime.last_error = Some(message);
                        runtime.sidecar_child = None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let message = format!("检查 API 服务 sidecar 状态失败: {}", error);
                        log_gateway_mode_warn(CodexLocalAccessGatewayMode::Sidecar, &message);
                        runtime.running = false;
                        runtime.actual_port = None;
                        runtime.actual_bind_host = None;
                        runtime.sidecar_config_fingerprint = None;
                        runtime.last_error = Some(message);
                        runtime.sidecar_child = None;
                    }
                }
            }
        }
        let stale_task = if !runtime.running {
            runtime.task.take()
        } else {
            None
        };
        (
            runtime.collection.clone(),
            runtime.running,
            runtime.actual_port,
            runtime.actual_bind_host.clone(),
            runtime.sidecar_config_fingerprint.clone(),
            stale_task,
        )
    };

    if let Some(task) = stale_task {
        let _ = task.await;
    }

    let Some(collection) = collection else {
        stop_gateway_locked().await;
        return Ok(());
    };

    if !collection.enabled {
        stop_gateway_locked().await;
        return Ok(());
    }

    let bind_host = bind_host_for_collection(&collection);
    let mode = collection_gateway_mode(&collection);
    if mode == CodexLocalAccessGatewayMode::Legacy {
        if running
            && actual_port == Some(collection.port)
            && actual_bind_host.as_deref() == Some(bind_host)
            && actual_fingerprint.is_none()
        {
            return Ok(());
        }
        if running {
            log_gateway_mode_info(
                CodexLocalAccessGatewayMode::Legacy,
                format!(
                    "API 服务网关配置已变化，准备重启: mode=legacy port={}->{} bind={}->{}",
                    actual_port
                        .map(|port| port.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    collection.port,
                    actual_bind_host.as_deref().unwrap_or("-"),
                    bind_host,
                ),
            );
        }
        let stopped_endpoint = stop_gateway_locked().await;
        if let Some(endpoint) = stopped_endpoint {
            wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
        }
        return start_legacy_gateway_locked(&collection).await;
    }

    let launch_config = prepare_sidecar_launch_config(&collection).await?;
    if running
        && actual_port == Some(collection.port)
        && actual_bind_host.as_deref() == Some(bind_host)
        && actual_fingerprint.as_deref() == Some(launch_config.fingerprint.as_str())
    {
        return Ok(());
    }
    if running {
        log_gateway_mode_info(
            CodexLocalAccessGatewayMode::Sidecar,
            format!(
                "API 服务网关配置已变化，准备重启: mode=sidecar port={}->{} bind={}->{} config_changed={}",
                actual_port
                    .map(|port| port.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                collection.port,
                actual_bind_host.as_deref().unwrap_or("-"),
                bind_host,
                actual_fingerprint.as_deref() != Some(launch_config.fingerprint.as_str())
            ),
        );
    }

    let stopped_endpoint = stop_gateway_locked().await;
    if let Some(endpoint) = stopped_endpoint {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    if probe_sidecar_ready_once(&collection, Duration::from_millis(250))
        .await
        .is_ok()
    {
        match process::kill_port_processes(collection.port) {
            Ok(count) if count > 0 => {
                log_gateway_mode_info(
                    CodexLocalAccessGatewayMode::Sidecar,
                    format!(
                        "已停止旧 API 服务 sidecar 以加载新配置: port={}, killed={}",
                        collection.port, count
                    ),
                );
            }
            Ok(_) => {}
            Err(error) => {
                let message = format!("停止旧 API 服务 sidecar 失败: {}", error);
                let mut runtime = gateway_runtime().lock().await;
                runtime.running = false;
                runtime.actual_port = None;
                runtime.actual_bind_host = None;
                runtime.sidecar_config_fingerprint = None;
                runtime.last_error = Some(message.clone());
                return Err(message);
            }
        }
        wait_for_gateway_port_release(bind_host, collection.port).await?;
    }

    let binary = match sidecar_binary_path() {
        Ok(path) => path,
        Err(message) => {
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };

    let mut command = TokioCommand::new(&binary);
    command
        .arg("--config")
        .arg(&launch_config.config_path)
        .arg("--manifest")
        .arg(&launch_config.manifest_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(
            launch_config
                .config_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(0x08000000);
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("启动 API 服务 sidecar 失败: {}", error);
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (ready_sender, mut ready_receiver) = oneshot::channel();
    let startup_diagnostics = Arc::new(Mutex::new(SidecarStartupDiagnostics::default()));
    let task_startup_diagnostics = Arc::clone(&startup_diagnostics);
    let task = tokio::spawn(async move {
        let stdout_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stderr_diagnostics = Arc::clone(&task_startup_diagnostics);
        let stdout_task = stdout.map(|stdout| {
            tokio::spawn(drain_sidecar_stdout(
                stdout,
                ready_sender,
                stdout_diagnostics,
            ))
        });
        let stderr_task =
            stderr.map(|stderr| tokio::spawn(drain_sidecar_stderr(stderr, stderr_diagnostics)));
        if let Some(task) = stdout_task {
            let _ = task.await;
        }
        if let Some(task) = stderr_task {
            let _ = task.await;
        }
    });

    let ready_signal = match wait_for_sidecar_ready(&mut ready_receiver, &mut child).await {
        Ok(signal) => signal,
        Err(error) => {
            let diagnostics = sidecar_startup_diagnostics_text(&startup_diagnostics);
            let message = format!("{}; {}", error, diagnostics);
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][sidecar] sidecar ready 等待失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };
    if let Some(ready_port) = ready_signal.port {
        if ready_port != collection.port {
            let message = format!(
                "API 服务 sidecar ready 端口不一致: expected={}, actual={}, host={}",
                collection.port, ready_port, ready_signal.host
            );
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess][sidecar] sidecar ready 校验失败，将停止进程: {}",
                message
            ));
            let _ = child.kill().await;
            task.abort();
            let _ = task.await;
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.sidecar_config_fingerprint = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    } else {
        let message = format!(
            "API 服务 sidecar ready 事件缺少端口: host={}",
            ready_signal.host
        );
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess][sidecar] sidecar ready 校验失败，将停止进程: {}",
            message
        ));
        let _ = child.kill().await;
        task.abort();
        let _ = task.await;
        let mut runtime = gateway_runtime().lock().await;
        runtime.running = false;
        runtime.actual_port = None;
        runtime.actual_bind_host = None;
        runtime.sidecar_config_fingerprint = None;
        runtime.last_error = Some(message.clone());
        return Err(message);
    }

    let port = collection.port;
    let bind_host = bind_host.to_string();
    log_sidecar_proxy_signature(&launch_config.proxy_signature);
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][sidecar] API 服务 sidecar 已启动: bin={} bind={}:{} base={}",
        binary.display(),
        bind_host,
        port,
        build_base_url(port)
    ));

    let mut runtime = gateway_runtime().lock().await;
    runtime.running = true;
    runtime.actual_port = Some(collection.port);
    runtime.actual_bind_host = Some(bind_host);
    runtime.sidecar_config_fingerprint = Some(launch_config.fingerprint);
    runtime.last_error = None;
    runtime.shutdown_sender = None;
    runtime.task = Some(task);
    runtime.sidecar_child = Some(child);
    Ok(())
}

async fn stop_gateway() -> Option<GatewayBindEndpoint> {
    let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
    stop_gateway_locked().await
}

async fn stop_gateway_locked() -> Option<GatewayBindEndpoint> {
    let (shutdown_sender, task, child, endpoint) = {
        let mut runtime = gateway_runtime().lock().await;
        let endpoint = runtime
            .actual_port
            .zip(runtime.actual_bind_host.clone())
            .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
        runtime.running = false;
        runtime.actual_port = None;
        runtime.actual_bind_host = None;
        runtime.sidecar_config_fingerprint = None;
        (
            runtime.shutdown_sender.take(),
            runtime.task.take(),
            runtime.sidecar_child.take(),
            endpoint,
        )
    };

    if let Some(sender) = shutdown_sender {
        let _ = sender.send(true);
    }
    if let Some(mut child) = child {
        match timeout(GATEWAY_SHUTDOWN_TIMEOUT, child.kill()).await {
            Ok(Ok(())) => {
                let _ = child.wait().await;
            }
            Ok(Err(error)) => {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 停止 API 服务 sidecar 失败: {}",
                    error
                ));
            }
            Err(_) => {
                logger::log_codex_api_warn(
                    "[CodexLocalAccess] 停止 API 服务 sidecar 超时，继续清理监听任务",
                );
            }
        }
    }
    if let Some(mut task) = task {
        tokio::select! {
            result = &mut task => {
                let _ = result;
            }
            _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
                logger::log_codex_api_warn("[CodexLocalAccess] 停止本地接入服务超时，已强制中止监听任务");
                task.abort();
                let _ = task.await;
            }
        }
    }

    endpoint
}

fn apply_usage_stats(
    target: &mut CodexLocalAccessUsageStats,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
) {
    target.request_count = target.request_count.saturating_add(1);
    if success {
        target.success_count = target.success_count.saturating_add(1);
    } else {
        target.failure_count = target.failure_count.saturating_add(1);
    }
    let normalized_error_category = error_category
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if matches!(normalized_error_category, Some(category) if is_client_canceled_error_category(category))
    {
        target.client_canceled_count = target.client_canceled_count.saturating_add(1);
    }
    if matches!(normalized_error_category, Some(category) if is_upstream_response_failed_error_category(category))
    {
        target.upstream_response_failed_count =
            target.upstream_response_failed_count.saturating_add(1);
    }
    if matches!(normalized_error_category, Some(category) if is_stream_incomplete_error_category(category))
    {
        target.stream_incomplete_count = target.stream_incomplete_count.saturating_add(1);
    }
    target.total_latency_ms = target.total_latency_ms.saturating_add(latency_ms);
    match request_kind {
        CodexLocalAccessRequestKind::Text => {
            target.text_request_count = target.text_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::ImageGeneration => {
            target.image_request_count = target.image_request_count.saturating_add(1);
            target.image_generation_request_count =
                target.image_generation_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::ImageEdit => {
            target.image_request_count = target.image_request_count.saturating_add(1);
            target.image_edit_request_count = target.image_edit_request_count.saturating_add(1);
        }
        CodexLocalAccessRequestKind::Other => {}
    }
    if matches!(
        normalized_error_category,
        Some("image_generation_not_enabled" | "image_generation_disabled")
    ) {
        target.image_generation_capability_failure_count = target
            .image_generation_capability_failure_count
            .saturating_add(1);
    }

    if let Some(usage) = usage {
        target.input_tokens = target.input_tokens.saturating_add(usage.input_tokens);
        target.output_tokens = target.output_tokens.saturating_add(usage.output_tokens);
        target.total_tokens = target.total_tokens.saturating_add(usage.total_tokens);
        target.cached_tokens = target.cached_tokens.saturating_add(usage.cached_tokens);
        target.reasoning_tokens = target
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
    }
    if estimated_cost_usd.is_finite() && estimated_cost_usd > 0.0 {
        target.estimated_cost_usd += estimated_cost_usd;
    }
}

fn upsert_account_usage_stats(
    accounts: &mut Vec<CodexLocalAccessAccountStats>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let normalized_email = account_email
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(account_stats) = accounts
        .iter_mut()
        .find(|item| item.account_id == account_id)
    {
        if !normalized_email.is_empty() {
            account_stats.email = normalized_email;
        }
        account_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut account_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut account_stats = CodexLocalAccessAccountStats {
        account_id: account_id.to_string(),
        email: normalized_email,
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut account_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    accounts.push(account_stats);
}

fn upsert_model_usage_stats(
    models: &mut Vec<CodexLocalAccessModelStats>,
    model_id: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(model_id) = model_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    if let Some(model_stats) = models.iter_mut().find(|item| item.model_id == model_id) {
        model_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut model_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut model_stats = CodexLocalAccessModelStats {
        model_id: model_id.to_string(),
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut model_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    models.push(model_stats);
}

fn upsert_api_key_usage_stats(
    api_keys: &mut Vec<CodexLocalAccessApiKeyStats>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
    estimated_cost_usd: f64,
    updated_at: i64,
) {
    let Some(api_key_id) = api_key_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    let normalized_label = api_key_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string();

    if let Some(api_key_stats) = api_keys
        .iter_mut()
        .find(|item| item.api_key_id == api_key_id)
    {
        if !normalized_label.is_empty() {
            api_key_stats.label = normalized_label;
        }
        api_key_stats.updated_at = updated_at;
        apply_usage_stats(
            &mut api_key_stats.usage,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage,
            estimated_cost_usd,
        );
        return;
    }

    let mut api_key_stats = CodexLocalAccessApiKeyStats {
        api_key_id: api_key_id.to_string(),
        label: normalized_label,
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(
        &mut api_key_stats.usage,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        estimated_cost_usd,
    );
    api_keys.push(api_key_stats);
}

fn build_account_health_snapshot(runtime: &GatewayRuntime) -> Vec<CodexLocalAccessAccountHealth> {
    let now = now_ms();
    let Some(collection) = runtime.collection.as_ref() else {
        return Vec::new();
    };
    let stats_emails: HashMap<&str, &str> = runtime
        .stats
        .accounts
        .iter()
        .map(|item| (item.account_id.as_str(), item.email.as_str()))
        .collect();

    collection
        .account_ids
        .iter()
        .map(|account_id| {
            let health = runtime.account_health.get(account_id);
            let cooldowns = runtime
                .model_cooldowns
                .iter()
                .filter_map(|(key, cooldown)| {
                    key.strip_prefix(&format!("{}{}", account_id, COOLDOWN_KEY_SEPARATOR))
                        .map(|_| {
                            let remaining_ms = cooldown.next_retry_at_ms.saturating_sub(now).max(0);
                            CodexLocalAccessAccountCooldown {
                                model_id: cooldown.model_key.clone(),
                                next_retry_at: cooldown.next_retry_at_ms,
                                remaining_ms,
                                reason: cooldown.reason.clone(),
                            }
                        })
                })
                .collect::<Vec<_>>();
            let image_generation_status = if collection.image_generation_mode
                == CodexLocalAccessImageGenerationMode::Disabled
            {
                CodexLocalAccessImageGenerationStatus::Disabled
            } else {
                health
                    .map(|item| item.image_generation_status)
                    .unwrap_or_default()
            };
            CodexLocalAccessAccountHealth {
                account_id: account_id.clone(),
                email: health
                    .and_then(|item| {
                        Some(item.email.as_str()).filter(|value| !value.trim().is_empty())
                    })
                    .or_else(|| stats_emails.get(account_id.as_str()).copied())
                    .unwrap_or_default()
                    .to_string(),
                available: cooldowns.is_empty()
                    && health
                        .map(|item| item.consecutive_failures < 3)
                        .unwrap_or(true),
                consecutive_failures: health
                    .map(|item| item.consecutive_failures)
                    .unwrap_or_default(),
                last_success_at: health.and_then(|item| item.last_success_at),
                last_failure_at: health.and_then(|item| item.last_failure_at),
                last_failure_status: health.and_then(|item| item.last_failure_status),
                last_failure_category: health.and_then(|item| item.last_failure_category.clone()),
                last_failure_message: health.and_then(|item| item.last_failure_message.clone()),
                image_generation_status,
                image_generation_checked_at: health
                    .and_then(|item| item.image_generation_checked_at),
                cooldowns,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default)]
struct RequestStatsMeta<'a> {
    request_id: Option<&'a str>,
    http_status: Option<u16>,
    error_message: Option<&'a str>,
}

async fn record_request_stats(
    account_id: Option<&str>,
    account_email: Option<&str>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    model_id: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<UsageCapture>,
) -> Result<(), String> {
    record_request_stats_with_meta(
        account_id,
        account_email,
        api_key_id,
        api_key_label,
        model_id,
        request_kind,
        success,
        error_category,
        latency_ms,
        usage,
        RequestStatsMeta::default(),
    )
    .await
}

async fn record_request_stats_with_meta(
    account_id: Option<&str>,
    account_email: Option<&str>,
    api_key_id: Option<&str>,
    api_key_label: Option<&str>,
    model_id: Option<&str>,
    request_kind: CodexLocalAccessRequestKind,
    success: bool,
    error_category: Option<&str>,
    latency_ms: u64,
    usage: Option<UsageCapture>,
    meta: RequestStatsMeta<'_>,
) -> Result<(), String> {
    let persisted_event = {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        let usage_ref = usage.as_ref();
        let pricing = resolve_model_pricing(runtime.collection.as_ref(), model_id);
        let estimated_cost_usd = calculate_usage_cost_usd(usage_ref, pricing.as_ref());
        let gateway_mode = runtime.collection.as_ref().map(collection_gateway_mode);
        if runtime.stats.since <= 0 {
            runtime.stats.since = now;
        }
        runtime.stats.updated_at = now;
        apply_usage_stats(
            &mut runtime.stats.totals,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
        );
        upsert_account_usage_stats(
            &mut runtime.stats.accounts,
            account_id,
            account_email,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        upsert_model_usage_stats(
            &mut runtime.stats.models,
            model_id,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        upsert_api_key_usage_stats(
            &mut runtime.stats.api_keys,
            api_key_id,
            api_key_label,
            request_kind,
            success,
            error_category,
            latency_ms,
            usage_ref,
            estimated_cost_usd,
            now,
        );
        let event = append_usage_event(
            &mut runtime.stats.events,
            now,
            meta.request_id,
            account_id,
            account_email,
            api_key_id,
            api_key_label,
            model_id,
            gateway_mode,
            request_kind,
            success,
            meta.http_status,
            error_category,
            meta.error_message,
            latency_ms,
            usage_ref,
            pricing.as_ref(),
            estimated_cost_usd,
        );

        normalize_stats(&mut runtime.stats);
        runtime.stats_dirty = true;
        event
    };

    if let Err(error) = persist_local_access_usage_event(&persisted_event) {
        logger::log_codex_api_warn(&format!(
            "API 服务请求日志写入失败，已保留内存统计并继续处理请求: {}",
            error
        ));
    }

    schedule_stats_flush_if_needed().await;
    Ok(())
}

fn build_state_snapshot_inner(
    runtime: &GatewayRuntime,
    include_default_profile: bool,
) -> CodexLocalAccessState {
    let collection = runtime.collection.clone();
    let member_count = collection
        .as_ref()
        .map(|item| item.account_ids.len())
        .unwrap_or(0);
    let api_port_url = collection
        .as_ref()
        .map(|item| build_api_port_url(item.port));
    let base_url = collection.as_ref().map(|item| build_base_url(item.port));
    let default_profile = if include_default_profile {
        collection.as_ref().map(|collection| {
            inspect_local_access_profile_attachment(
                &codex_account::get_codex_home(),
                Some(collection),
            )
        })
    } else {
        None
    };
    let lan_base_url = collection.as_ref().and_then(|item| {
        if item.access_scope == CodexLocalAccessScope::Lan {
            build_lan_base_url(item.port)
        } else {
            None
        }
    });
    let model_ids = collection
        .as_ref()
        .map(|collection| {
            visible_codex_model_ids_for_collection(collection, Some(&runtime.account_health))
        })
        .unwrap_or_else(supported_codex_model_ids);
    let mut stats = runtime.stats.clone();
    stats.events = stats
        .events
        .iter()
        .rev()
        .take(STATE_RECENT_USAGE_EVENT_LIMIT)
        .cloned()
        .collect();
    let account_health = build_account_health_snapshot(runtime);

    CodexLocalAccessState {
        collection,
        running: runtime.running,
        default_profile,
        api_port_url,
        base_url,
        lan_base_url,
        model_ids,
        model_pricing_presets: default_model_pricing_presets(),
        last_error: runtime.last_error.clone(),
        member_count,
        stats,
        account_health,
    }
}

fn build_state_snapshot(runtime: &GatewayRuntime) -> CodexLocalAccessState {
    build_state_snapshot_inner(runtime, true)
}

fn build_request_state_snapshot(runtime: &GatewayRuntime) -> CodexLocalAccessState {
    build_state_snapshot_inner(runtime, false)
}

async fn snapshot_state() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    if let Err(err) = ensure_gateway_matches_runtime().await {
        let mut runtime = gateway_runtime().lock().await;
        runtime.last_error = Some(err);
        return Ok(build_state_snapshot(&runtime));
    }
    let mut runtime = gateway_runtime().lock().await;
    if runtime
        .last_error
        .as_deref()
        .map(|message| {
            message.starts_with("默认 Codex 配置接管失败:")
                || message.starts_with("Codex 配置接管失败:")
        })
        .unwrap_or(false)
    {
        runtime.last_error = None;
    }
    Ok(build_state_snapshot(&runtime))
}

pub async fn get_local_access_state() -> Result<CodexLocalAccessState, String> {
    snapshot_state().await
}

pub async fn activate_local_access_for_dir(
    profile_dir: &Path,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    save_profile_takeover_backup(profile_dir)?;
    let state = set_local_access_enabled(true).await?;
    let collection = state
        .collection
        .clone()
        .ok_or_else(|| "API 服务集合尚未创建".to_string())?;
    write_local_access_profile_takeover(profile_dir, &collection).await?;
    Ok(state)
}

#[derive(Debug, Clone)]
struct LocalAccessGatewayProbeFailure {
    status: Option<u16>,
    message: String,
    detail: Option<String>,
    gateway_output: Option<String>,
}

#[derive(Debug, Clone)]
enum LocalAccessGatewayProbeResult {
    Passed,
    Failed(LocalAccessGatewayProbeFailure),
}

fn truncate_diagnostic_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut result = value.chars().take(max_chars).collect::<String>();
    result.push_str("...");
    result
}

fn clean_diagnostic_text(value: impl Into<String>) -> Option<String> {
    let text = value.into().trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(truncate_diagnostic_text(&text, 4000))
    }
}

fn extract_gateway_error_message(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "网关未返回错误内容".to_string();
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = value.get("error").and_then(Value::as_str) {
            return message.to_string();
        }
        if let Some(message) = value
            .get("error")
            .and_then(|item| item.get("message"))
            .and_then(Value::as_str)
        {
            return message.to_string();
        }
        if let Some(message) = value.get("message").and_then(Value::as_str) {
            return message.to_string();
        }
    }

    truncate_diagnostic_text(trimmed, 800)
}

fn build_failure_result(failure: CodexLocalAccessTestFailure) -> CodexLocalAccessTestResult {
    CodexLocalAccessTestResult {
        model_id: failure.model_id.clone(),
        latency_ms: None,
        output: None,
        failure: Some(failure),
    }
}

fn local_access_test_failure(
    title: impl Into<String>,
    stage: impl Into<String>,
    cause: impl Into<String>,
    suggestion: impl Into<String>,
    model_id: Option<String>,
) -> CodexLocalAccessTestFailure {
    CodexLocalAccessTestFailure {
        title: title.into(),
        stage: stage.into(),
        cause: cause.into(),
        suggestion: suggestion.into(),
        status: None,
        model_id,
        detail: None,
        cli_output: None,
        gateway_output: None,
    }
}

async fn probe_local_access_gateway(
    base_url: &str,
    api_key: &str,
    model_id: &str,
) -> LocalAccessGatewayProbeResult {
    let url = format!("{}/v1/responses", base_url.trim_end_matches('/'));
    let client = match build_localhost_http_client(Duration::from_secs(90), "本地网关诊断") {
        Ok(client) => client,
        Err(error) => {
            return LocalAccessGatewayProbeResult::Failed(LocalAccessGatewayProbeFailure {
                status: None,
                message: format!("创建本地网关诊断客户端失败: {}", error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    let body = json!({
        "model": model_id,
        "stream": false,
        "store": false,
        "input": "Reply with exactly: pong"
    });
    let response = match client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", api_key.trim()))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return LocalAccessGatewayProbeResult::Failed(LocalAccessGatewayProbeFailure {
                status: error.status().map(|status| status.as_u16()),
                message: format!("无法连接本地网关 {}: {}", url, error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    let status = response.status();
    let body_text = match response.text().await {
        Ok(text) => text,
        Err(error) => {
            return LocalAccessGatewayProbeResult::Failed(LocalAccessGatewayProbeFailure {
                status: Some(status.as_u16()),
                message: format!("读取本地网关响应失败: {}", error),
                detail: Some(error.to_string()),
                gateway_output: None,
            });
        }
    };

    if status.is_success() {
        return LocalAccessGatewayProbeResult::Passed;
    }

    LocalAccessGatewayProbeResult::Failed(LocalAccessGatewayProbeFailure {
        status: Some(status.as_u16()),
        message: extract_gateway_error_message(&body_text),
        detail: clean_diagnostic_text(body_text.clone()),
        gateway_output: clean_diagnostic_text(format!("HTTP {}\n{}", status.as_u16(), body_text)),
    })
}

fn format_cli_failure_output(
    error: &codex_wakeup::CodexWakeupCliConversationDetailedError,
) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(status) = error.status.as_deref() {
        lines.push(format!("exit_status: {}", status));
    }
    if let Some(duration_ms) = error.duration_ms {
        lines.push(format!("duration_ms: {}", duration_ms));
    }
    if let Some(stderr) = error.stderr.as_deref() {
        lines.push(format!("stderr:\n{}", stderr));
    }
    if let Some(stdout) = error.stdout.as_deref() {
        lines.push(format!("stdout:\n{}", stdout));
    }
    if let Some(last_message) = error.last_message.as_deref() {
        lines.push(format!("last_message:\n{}", last_message));
    }
    clean_diagnostic_text(lines.join("\n\n"))
}

fn cli_error_indicates_local_proxy_interception(
    error: &codex_wakeup::CodexWakeupCliConversationDetailedError,
) -> bool {
    let mut text = error.message.clone();
    if let Some(value) = error.stdout.as_deref() {
        text.push('\n');
        text.push_str(value);
    }
    if let Some(value) = error.stderr.as_deref() {
        text.push('\n');
        text.push_str(value);
    }
    if let Some(value) = error.last_message.as_deref() {
        text.push('\n');
        text.push_str(value);
    }

    let lower = text.to_ascii_lowercase();
    let mentions_loopback =
        lower.contains("127.0.0.1") || lower.contains("localhost") || lower.contains("::1");
    let mentions_proxy = lower.contains("proxy(")
        || lower.contains(" intercepts ")
        || lower.contains("proxy_url")
        || lower.contains("proxy-url")
        || lower.contains("bad gateway")
        || lower.contains("status=502")
        || lower.contains("http 502");

    mentions_loopback && mentions_proxy
}

fn is_quota_or_rate_limit_message(status: Option<u16>, message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    matches!(status, Some(429))
        || lower.contains("usage_limit_reached")
        || lower.contains("limit reached")
        || lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("cooldown")
        || lower.contains("额度")
        || lower.contains("限流")
        || lower.contains("冷却")
}

fn is_image_generation_capability_message(status: Option<u16>, message: &str) -> bool {
    if !matches!(status, Some(400 | 403 | 422)) {
        return false;
    }
    let lower = message.to_ascii_lowercase();
    lower.contains("image_generation_not_enabled")
        || lower.contains("image generation is not enabled")
        || lower.contains("image_generation is not enabled")
        || (lower.contains("image_generation") && lower.contains("not enabled"))
        || message.contains("未启用图片生成能力")
}

fn classify_gateway_probe_failure(
    model_id: &str,
    cli_error: &codex_wakeup::CodexWakeupCliConversationDetailedError,
    probe_failure: LocalAccessGatewayProbeFailure,
) -> CodexLocalAccessTestFailure {
    let status = probe_failure.status;
    let message = probe_failure.message.trim();
    let lower = message.to_ascii_lowercase();
    let (title, stage, suggestion) = if status.is_none() {
        (
            "无法连接本地网关",
            "本地网关连接",
            "确认 API 服务仍在运行，端口未被系统占用或安全软件拦截；如端口异常，可先清理端口或更换端口后重试。",
        )
    } else if matches!(status, Some(401)) {
        if lower.contains("authorization") || message.contains("密钥") || lower.contains("api-key")
        {
            (
                "本地 API 服务密钥无效",
                "本地网关鉴权",
                "重置 API 服务密钥后重新复制 Base URL/API Key，并确认 Codex CLI 使用的是最新配置。",
            )
        } else {
            (
                "Codex 账号鉴权失败",
                "上游账号鉴权",
                "刷新该 Codex 账号额度或重新导入账号；如果账号已退出登录或令牌过期，需要重新登录后再测试。",
            )
        }
    } else if is_image_generation_capability_message(status, message) {
        (
            "图片生成能力不可用",
            "上游图片能力",
            "如果只是普通文本对话报错，请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”；如果需要生图，请换用具备图片能力的 Codex 账号。",
        )
    } else if is_quota_or_rate_limit_message(status, message) {
        (
            "上游限流或额度不足",
            "上游额度",
            "查看账号额度池，切换到仍有额度的账号，或等待冷却窗口结束后重试。",
        )
    } else if matches!(status, Some(502) | Some(503) | Some(504)) {
        if message.contains("暂无可用账号")
            || message.contains("集合暂无")
            || message.contains("Free 账号")
            || message.contains("API Key 账号")
        {
            (
                "账号池暂无可用账号",
                "账号池路由",
                "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号，并确认未被 Free 账号限制拦截。",
            )
        } else {
            (
                "上游服务或代理不可用",
                "上游请求",
                "检查 API 服务代理地址、网络连通性和 Codex 上游服务状态；如果 API 服务没有请求记录，检查代理工具是否拦截 localhost / 127.0.0.1。",
            )
        }
    } else {
        (
            "本地网关请求失败",
            "本地网关响应",
            "根据 HTTP 状态和网关返回内容处理；如果是账号相关错误，优先刷新或重新导入对应账号。",
        )
    };

    CodexLocalAccessTestFailure {
        title: title.to_string(),
        stage: stage.to_string(),
        cause: if let Some(status) = status {
            format!("本地网关返回 HTTP {}：{}", status, message)
        } else {
            message.to_string()
        },
        suggestion: suggestion.to_string(),
        status,
        model_id: Some(model_id.to_string()),
        detail: probe_failure.detail,
        cli_output: format_cli_failure_output(cli_error),
        gateway_output: probe_failure.gateway_output,
    }
}

fn build_cli_environment_failure(
    model_id: &str,
    cli_error: codex_wakeup::CodexWakeupCliConversationDetailedError,
    gateway_passed: bool,
) -> CodexLocalAccessTestFailure {
    if gateway_passed && cli_error_indicates_local_proxy_interception(&cli_error) {
        return CodexLocalAccessTestFailure {
            title: "本机 API 请求被代理拦截".to_string(),
            stage: "Codex CLI 本机连接".to_string(),
            cause: format!(
                "本地网关直接诊断已通过，但 Codex CLI 访问 127.0.0.1/localhost 时被代理接管：{}",
                cli_error.message
            ),
            suggestion:
                "将 127.0.0.1、127.0.0.0/8、localhost、::1 加入代理工具直连规则，并确认 Codex 启动环境包含 NO_PROXY/no_proxy。"
                    .to_string(),
            status: None,
            model_id: Some(model_id.to_string()),
            detail: None,
            cli_output: format_cli_failure_output(&cli_error),
            gateway_output: None,
        };
    }

    CodexLocalAccessTestFailure {
        title: "Codex CLI 执行环境异常".to_string(),
        stage: "Codex CLI".to_string(),
        cause: if gateway_passed {
            format!(
                "本地网关直接诊断已通过，但 Codex CLI 真实请求失败：{}",
                cli_error.message
            )
        } else {
            cli_error.message.clone()
        },
        suggestion: "检查 Codex CLI 路径、版本、配置文件读取权限和运行时环境；如果刚升级过 CLI，请重启应用后再测。".to_string(),
        status: None,
        model_id: Some(model_id.to_string()),
        detail: None,
        cli_output: format_cli_failure_output(&cli_error),
        gateway_output: None,
    }
}

pub async fn test_local_access_with_cli() -> Result<CodexLocalAccessTestResult, String> {
    ensure_runtime_loaded().await?;
    let state = snapshot_state().await?;
    let Some(collection) = state.collection.clone() else {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务集合尚未创建",
            "检测前置条件",
            "当前没有可用于本地 API 服务的账号集合配置。",
            "先在 API 服务弹框中选择账号并保存，然后启用服务后再测试。",
            None,
        )));
    };
    if !collection.enabled {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务未启用",
            "检测前置条件",
            "当前 API 服务处于停用状态，CLI 无法通过本地网关发起请求。",
            "先启用 API 服务，再重新执行健康检测。",
            None,
        )));
    }
    if !state.running {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务未运行",
            "本地网关进程",
            "API 服务配置已启用，但本地网关当前没有监听端口。",
            "先启动 API 服务；如果端口被占用，清理端口或更换端口后重试。",
            None,
        )));
    }
    if collection.account_ids.is_empty() {
        return Ok(build_failure_result(local_access_test_failure(
            "账号集合为空",
            "账号池配置",
            "API 服务集合中没有账号，网关没有可路由的上游账号。",
            "在 API 服务账号集合中加入可用的 Codex OAuth 或 API Key 账号后再测试。",
            None,
        )));
    }

    let base_url = state
        .base_url
        .clone()
        .unwrap_or_else(|| build_base_url(collection.port));
    let Some(model_id) = state.model_ids.first().cloned() else {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务暂无可用模型",
            "模型配置",
            "当前 API 服务没有可用于检测的模型 ID。",
            "确认账号集合中至少有一个可用账号，并刷新模型/账号状态后再测试。",
            None,
        )));
    };
    if model_id.trim().is_empty() {
        return Ok(build_failure_result(local_access_test_failure(
            "API 服务暂无可用模型",
            "模型配置",
            "当前 API 服务没有可用于检测的模型 ID。",
            "确认账号集合中至少有一个可用账号，并刷新模型/账号状态后再测试。",
            None,
        )));
    }
    let temp_home = std::env::temp_dir().join(format!(
        "antigravity-codex-api-service-test-{}",
        uuid::Uuid::new_v4()
    ));

    if let Err(error) = std::fs::create_dir_all(&temp_home) {
        return Ok(build_failure_result(local_access_test_failure(
            "创建 CLI 检测环境失败",
            "Codex CLI 环境",
            format!("无法创建临时 CODEX_HOME：{}", error),
            "检查系统临时目录写入权限和磁盘空间后重试。",
            Some(model_id),
        )));
    }
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }
    let runtime_account = build_runtime_account(
        base_url.clone(),
        collection.api_key.clone(),
        bound_oauth_account_id,
    );
    if let Err(err) = codex_account::write_account_bundle_to_dir(&temp_home, &runtime_account) {
        let _ = std::fs::remove_dir_all(&temp_home);
        return Ok(build_failure_result(local_access_test_failure(
            "写入 CLI 检测账号失败",
            "Codex CLI 环境",
            format!("无法写入检测用 auth.json/config.toml：{}", err),
            "检查临时目录写入权限；如果文件被安全软件拦截，放行后再重试。",
            Some(model_id),
        )));
    }

    let run_home = temp_home.clone();
    let run_model_id = model_id.clone();
    let cli_result = tokio::task::spawn_blocking(move || {
        codex_wakeup::run_cli_conversation_in_home_detailed(
            &run_home,
            "Reply with exactly: pong",
            &codex_wakeup::CodexWakeupExecutionConfig {
                model: Some(run_model_id),
                model_display_name: None,
                model_reasoning_effort: None,
            },
        )
    })
    .await
    .map_err(|e| {
        local_access_test_failure(
            "API 服务检测任务执行失败",
            "检测任务",
            format!("后台检测任务无法完成：{}", e),
            "重试检测；如果持续发生，重启应用后再测试。",
            Some(model_id.clone()),
        )
    });

    if let Err(err) = std::fs::remove_dir_all(&temp_home) {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 清理 API 服务检测环境失败: path={}, error={}",
            temp_home.display(),
            err
        ));
    }

    let cli_result = match cli_result {
        Ok(result) => result,
        Err(failure) => return Ok(build_failure_result(failure)),
    };
    let cli_result = match cli_result {
        Ok(result) => result,
        Err(cli_error) => {
            let probe_result =
                probe_local_access_gateway(&base_url, &collection.api_key, &model_id).await;
            let failure = match probe_result {
                LocalAccessGatewayProbeResult::Passed => {
                    build_cli_environment_failure(&model_id, cli_error, true)
                }
                LocalAccessGatewayProbeResult::Failed(probe_failure) => {
                    classify_gateway_probe_failure(&model_id, &cli_error, probe_failure)
                }
            };
            return Ok(build_failure_result(failure));
        }
    };
    Ok(CodexLocalAccessTestResult {
        model_id: Some(model_id),
        latency_ms: Some(cli_result.duration_ms),
        output: Some(cli_result.reply),
        failure: None,
    })
}

pub async fn save_local_access_accounts(
    account_ids: Vec<String>,
    restrict_free_accounts: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let mut collection = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .clone()
            .unwrap_or(CodexLocalAccessCollection {
                enabled: false,
                port: allocate_initial_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
                api_key: generate_local_api_key(),
                api_keys: Vec::new(),
                access_scope: CodexLocalAccessScope::Localhost,
                image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
                gateway_mode: CodexLocalAccessGatewayMode::default(),
                upstream_proxy_url: None,
                routing_strategy: CodexLocalAccessRoutingStrategy::default(),
                custom_routing_rules: Vec::new(),
                model_aliases: Vec::new(),
                model_pricings: Vec::new(),
                excluded_models: Vec::new(),
                session_affinity: false,
                session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
                max_retry_credentials: 0,
                max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
                timeouts: CodexLocalAccessTimeouts::default(),
                active_timeout_preset_id: BUILTIN_TIMEOUT_PRESET_LONG_WAIT_ID.to_string(),
                timeout_presets: Vec::new(),
                disable_cooling: false,
                restrict_free_accounts: true,
                debug_logs: true,
                bound_oauth_account_id: None,
                account_ids: Vec::new(),
                created_at: now_ms(),
                updated_at: now_ms(),
            })
    };

    let valid_account_ids: HashSet<String> = codex_account::list_accounts_checked()?
        .into_iter()
        .filter(|account| is_local_access_eligible_account(account, restrict_free_accounts))
        .map(|account| account.id)
        .collect();

    let mut next_account_ids = Vec::new();
    let mut seen = HashSet::new();
    for account_id in account_ids {
        if !valid_account_ids.contains(&account_id) {
            continue;
        }
        if seen.insert(account_id.clone()) {
            next_account_ids.push(account_id);
        }
    }

    collection.restrict_free_accounts = restrict_free_accounts;
    collection.account_ids = next_account_ids;
    collection.updated_at = now_ms();
    let (changed, _) = sanitize_collection(&mut collection)?;
    if changed {
        collection.updated_at = now_ms();
    }
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_routing_strategy(
    strategy: CodexLocalAccessRoutingStrategy,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.routing_strategy == strategy {
        return snapshot_state().await;
    }

    collection.routing_strategy = strategy;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_custom_routing(
    rules: Vec<CodexLocalAccessCustomRoutingRule>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.custom_routing_rules =
        normalize_custom_routing_rules(rules, &collection.account_ids);
    collection.routing_strategy = CodexLocalAccessRoutingStrategy::Custom;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_model_rules(
    model_aliases: Vec<CodexLocalAccessModelAlias>,
    excluded_models: Vec<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.model_aliases = normalize_model_aliases(model_aliases);
    collection.excluded_models = normalize_model_rule_list(excluded_models);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_model_pricings(
    model_pricings: Vec<CodexLocalAccessModelPricing>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.model_pricings = normalize_model_pricings(model_pricings);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    snapshot_state().await
}

pub async fn update_local_access_routing_options(
    session_affinity: bool,
    session_affinity_ttl_ms: i64,
    max_retry_credentials: u16,
    max_retry_interval_ms: u64,
    disable_cooling: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.session_affinity = session_affinity;
    collection.session_affinity_ttl_ms =
        session_affinity_ttl_ms.clamp(SESSION_AFFINITY_TTL_MIN_MS, SESSION_AFFINITY_TTL_MAX_MS);
    collection.max_retry_credentials =
        max_retry_credentials.min(MAX_RETRY_CREDENTIALS_PER_REQUEST as u16);
    collection.max_retry_interval_ms =
        max_retry_interval_ms.clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS);
    collection.disable_cooling = disable_cooling;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_timeouts(
    timeouts: CodexLocalAccessTimeouts,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.timeouts = timeouts;
    normalize_timeouts(&mut collection.timeouts);
    if let Some(preset_id) = active_timeout_preset_id {
        collection.active_timeout_preset_id = preset_id;
        normalize_active_timeout_preset_id(&mut collection);
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_timeout_presets(
    timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    active_timeout_preset_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.timeout_presets = timeout_presets;
    normalize_timeout_presets(&mut collection.timeout_presets);
    if let Some(preset_id) = active_timeout_preset_id {
        collection.active_timeout_preset_id = preset_id;
    }
    normalize_active_timeout_preset_id(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    snapshot_state().await
}

pub async fn update_local_access_upstream_proxy_config(
    upstream_proxy_url: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let normalized_upstream_proxy_url = validate_upstream_proxy_config(upstream_proxy_url)?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.upstream_proxy_url == normalized_upstream_proxy_url {
        return snapshot_state().await;
    }

    collection.upstream_proxy_url = normalized_upstream_proxy_url;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_gateway_mode(
    gateway_mode: CodexLocalAccessGatewayMode,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.gateway_mode == gateway_mode {
        return snapshot_state().await;
    }

    collection.gateway_mode = gateway_mode;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_debug_logs(
    debug_logs: bool,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.debug_logs == debug_logs {
        return snapshot_state().await;
    }

    collection.debug_logs = debug_logs;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_scope(
    access_scope: CodexLocalAccessScope,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.access_scope == access_scope {
        return snapshot_state().await;
    }

    collection.access_scope = access_scope;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_image_generation_mode(
    image_generation_mode: CodexLocalAccessImageGenerationMode,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    if collection.image_generation_mode == image_generation_mode {
        return snapshot_state().await;
    }

    collection.image_generation_mode = image_generation_mode;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn remove_local_access_account(
    account_id: &str,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return snapshot_state().await;
    };

    let before_len = collection.account_ids.len();
    collection.account_ids.retain(|id| id != account_id);
    if collection.account_ids.len() == before_len {
        return snapshot_state().await;
    }

    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn rotate_local_access_api_key() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    normalize_collection_api_keys(&mut collection);
    let now = now_ms();
    let primary_id = collection
        .api_keys
        .iter()
        .find(|item| item.enabled)
        .or_else(|| collection.api_keys.first())
        .map(|item| item.id.clone());
    if let Some(primary_id) = primary_id {
        if let Some(api_key) = collection
            .api_keys
            .iter_mut()
            .find(|item| item.id == primary_id)
        {
            api_key.key = generate_local_api_key();
            api_key.updated_at = now;
            api_key.last_used_at = None;
            collection.api_key = api_key.key.clone();
        }
    } else {
        collection.api_key = generate_local_api_key();
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn create_local_access_api_key(
    label: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    collection
        .api_keys
        .push(build_local_access_api_key(label.as_deref()));
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_api_key(
    api_key_id: String,
    label: Option<String>,
    enabled: Option<bool>,
    model_prefix: Option<String>,
    allowed_models: Option<Vec<String>>,
    excluded_models: Option<Vec<String>>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    let api_key_id = api_key_id.trim();
    let Some(index) = collection
        .api_keys
        .iter()
        .position(|item| item.id == api_key_id)
    else {
        return Err("API Key 不存在".to_string());
    };
    if let Some(label) = label {
        collection.api_keys[index].label = normalize_api_key_label(Some(label.as_str()), "API Key");
    }
    if let Some(enabled) = enabled {
        collection.api_keys[index].enabled = enabled;
    }
    if model_prefix.is_some() {
        collection.api_keys[index].model_prefix = normalize_model_prefix_value(model_prefix);
    }
    if let Some(allowed_models) = allowed_models {
        collection.api_keys[index].allowed_models = normalize_model_rule_list(allowed_models);
    }
    if let Some(excluded_models) = excluded_models {
        collection.api_keys[index].excluded_models = normalize_model_rule_list(excluded_models);
    }
    collection.api_keys[index].updated_at = now_ms();
    if !collection.api_keys.iter().any(|item| item.enabled) {
        collection.api_keys[index].enabled = true;
    }
    normalize_collection_api_keys(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn rotate_local_access_named_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    let api_key_id = api_key_id.trim();
    let Some(api_key) = collection
        .api_keys
        .iter_mut()
        .find(|item| item.id == api_key_id)
    else {
        return Err("API Key 不存在".to_string());
    };
    api_key.key = generate_local_api_key();
    api_key.updated_at = now_ms();
    api_key.last_used_at = None;
    normalize_collection_api_keys(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn delete_local_access_api_key(
    api_key_id: String,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };
    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };
    normalize_collection_api_keys(&mut collection);
    if collection.api_keys.len() <= 1 {
        return Err("至少保留一个 API Key".to_string());
    }
    let api_key_id = api_key_id.trim();
    let before_len = collection.api_keys.len();
    collection.api_keys.retain(|item| item.id != api_key_id);
    if collection.api_keys.len() == before_len {
        return Err("API Key 不存在".to_string());
    }
    normalize_collection_api_keys(&mut collection);
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }
    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn update_local_access_bound_oauth_account(
    bound_oauth_account_id: Option<String>,
) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    let normalized_bound_id = normalize_optional_account_ref(bound_oauth_account_id.as_deref());
    if collection.bound_oauth_account_id == normalized_bound_id {
        return snapshot_state().await;
    }

    if let Some(bound_id) = normalized_bound_id {
        let bound_account = validate_local_access_bound_oauth_account(&bound_id)?;
        collection.bound_oauth_account_id = Some(bound_account.id);
    } else {
        collection.bound_oauth_account_id = None;
    }
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    snapshot_state().await
}

pub async fn clear_local_access_stats() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;
    if let Err(error) = clear_local_access_usage_events_db() {
        logger::log_codex_api_warn(&format!(
            "清空 API 服务请求日志失败，继续清空内存统计: {}",
            error
        ));
    }

    let cleared = empty_stats_snapshot();
    {
        let mut runtime = gateway_runtime().lock().await;
        runtime.stats = cleared;
        runtime.stats_dirty = true;
    }
    schedule_stats_flush_if_needed().await;

    snapshot_state().await
}

pub async fn prepare_local_access_gateway_for_restart() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    let stopped_endpoint = stop_gateway().await;
    if let Some(endpoint) = stopped_endpoint {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    let runtime = gateway_runtime().lock().await;
    Ok(build_state_snapshot(&runtime))
}

pub async fn kill_local_access_port_processes() -> Result<CodexLocalAccessPortCleanupResult, String>
{
    if let Err(err) = ensure_runtime_loaded_without_start().await {
        logger::log_codex_api_warn(&format!(
            "[CodexLocalAccess] 清理端口前加载配置失败: {}",
            err
        ));
        return Err(err);
    }

    let collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    }
    .ok_or_else(|| "API 服务集合尚未创建".to_string())?;

    stop_gateway().await;

    let killed_count = process::kill_port_processes(collection.port)? as u32;

    if collection.enabled {
        ensure_gateway_matches_runtime().await?;
    }

    let state = snapshot_state().await?;
    Ok(CodexLocalAccessPortCleanupResult {
        killed_count,
        state,
    })
}

pub async fn update_local_access_port(port: u16) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    ensure_local_port_available(
        bind_host_for_collection(&collection),
        port,
        Some(collection.port),
    )?;
    if collection.port == port {
        return snapshot_state().await;
    }

    collection.port = port;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    snapshot_state().await
}

pub async fn set_local_access_enabled(enabled: bool) -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded().await?;

    let maybe_collection = {
        let runtime = gateway_runtime().lock().await;
        runtime.collection.clone()
    };

    let Some(mut collection) = maybe_collection else {
        return Err("本地接入集合尚未创建".to_string());
    };

    collection.enabled = enabled;
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;
    let next_collection = collection.clone();

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
    if enabled {
        ensure_local_access_profile_takeovers(&next_collection).await?;
    } else {
        restore_takeover_profiles_after_disable(&next_collection)?;
    }
    snapshot_state().await
}

pub async fn restore_local_access_gateway() {
    if let Err(err) = ensure_runtime_loaded().await {
        let mut runtime = gateway_runtime().lock().await;
        runtime.loaded = true;
        runtime.last_error = Some(err.clone());
        logger::log_codex_api_warn(&format!("[CodexLocalAccess] 初始化失败: {}", err));
    }
}

pub async fn shutdown_local_access_gateway_for_app_exit() {
    let stopped_endpoint = stop_gateway().await;
    if let Some(endpoint) = stopped_endpoint {
        if let Err(error) = wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await
        {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 应用退出时等待 API 服务 sidecar 释放端口失败: {}",
                error
            ));
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_content_length(header_bytes: &[u8]) -> Result<usize, String> {
    let header_text = String::from_utf8_lossy(header_bytes);
    for line in header_text.lines() {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .map_err(|e| format!("非法 Content-Length: {}", e));
        }
    }
    Ok(0)
}

async fn read_http_request<R>(
    stream: &mut R,
    request_read_timeout: Duration,
) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;

    loop {
        let bytes_read = timeout(request_read_timeout, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取请求超时".to_string())?
            .map_err(|e| format!("读取请求失败: {}", e))?;

        if bytes_read == 0 {
            break;
        }

        buffer.extend_from_slice(&chunk[..bytes_read]);
        if buffer.len() > MAX_HTTP_REQUEST_BYTES {
            return Err("请求体过大".to_string());
        }

        if header_end.is_none() {
            if let Some(end) = find_header_end(&buffer) {
                content_length = parse_content_length(&buffer[..end])?;
                header_end = Some(end);
            }
        }

        if let Some(end) = header_end {
            if buffer.len() >= end.saturating_add(content_length) {
                return Ok(buffer[..(end + content_length)].to_vec());
            }
        }
    }

    Err("请求不完整".to_string())
}

fn parse_http_request(raw: &[u8]) -> Result<ParsedRequest, String> {
    let Some(header_end) = find_header_end(raw) else {
        return Err("缺少 HTTP 头结束标记".to_string());
    };

    let header_text = String::from_utf8_lossy(&raw[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().ok_or("请求行为空")?.trim();

    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("请求行缺少 method")?.to_string();
    let target = parts.next().ok_or("请求行缺少 target")?.to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else { continue };
        let Some(value) = parts.next() else { continue };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Ok(ParsedRequest {
        method,
        target,
        headers,
        body: raw[header_end..].to_vec(),
    })
}

fn normalize_proxy_target(target: &str) -> Result<String, String> {
    if target.starts_with("http://") || target.starts_with("https://") {
        let parsed = url::Url::parse(target).map_err(|e| format!("解析请求地址失败: {}", e))?;
        let mut next = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            next.push('?');
            next.push_str(query);
        }
        return Ok(next);
    }

    let parsed = url::Url::parse(&format!("http://localhost{}", target))
        .map_err(|e| format!("解析请求路径失败: {}", e))?;
    let mut next = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        next.push('?');
        next.push_str(query);
    }
    Ok(next)
}

fn extract_local_api_key(headers: &HashMap<String, String>) -> Option<String> {
    if let Some(value) = headers.get("authorization") {
        let trimmed = value.trim();
        if let Some(rest) = trimmed.strip_prefix("Bearer ") {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("bearer ") {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    headers
        .get("x-api-key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn is_local_models_request(target: &str) -> bool {
    target == "/v1/models" || target.starts_with("/v1/models?")
}

fn build_local_models_response(model_ids: &[String]) -> Value {
    let data: Vec<Value> = model_ids
        .iter()
        .cloned()
        .into_iter()
        .map(|model| {
            json!({
                "id": model,
                "object": "model",
                "created": 0,
                "owned_by": "openai",
            })
        })
        .collect();

    json!({
        "object": "list",
        "data": data,
    })
}

fn build_codex_client_models_response(model_ids: &[String]) -> Value {
    codex_protocol::build_codex_client_models_response(model_ids)
}

fn usage_number(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64).or_else(|| {
        value
            .and_then(Value::as_i64)
            .filter(|number| *number >= 0)
            .map(|number| number as u64)
    })
}

fn non_null_child<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|item| !item.is_null())
}

fn extract_usage_capture(value: &Value) -> Option<UsageCapture> {
    let usage = non_null_child(value, "usage")
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usage"))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| item.get("response"))
                .and_then(|item| non_null_child(item, "usage"))
        })
        .or_else(|| non_null_child(value, "usageMetadata"))
        .or_else(|| non_null_child(value, "usage_metadata"))
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usageMetadata"))
        })
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "usage_metadata"))
        })?;

    let input_tokens = usage_number(
        usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .or_else(|| usage.get("promptTokenCount")),
    )
    .unwrap_or(0);
    let output_tokens = usage_number(
        usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .or_else(|| usage.get("candidatesTokenCount")),
    )
    .unwrap_or(0);
    let explicit_total_tokens = usage_number(
        usage
            .get("total_tokens")
            .or_else(|| usage.get("totalTokenCount")),
    );
    let cached_tokens = usage_number(
        usage
            .get("cached_tokens")
            .or_else(|| {
                usage
                    .get("input_tokens_details")
                    .and_then(|item| item.get("cached_tokens"))
            })
            .or_else(|| {
                usage
                    .get("prompt_tokens_details")
                    .and_then(|item| item.get("cached_tokens"))
            })
            .or_else(|| usage.get("cachedContentTokenCount")),
    )
    .unwrap_or(0);
    let reasoning_tokens = usage_number(
        usage
            .get("reasoning_tokens")
            .or_else(|| {
                usage
                    .get("output_tokens_details")
                    .and_then(|item| item.get("reasoning_tokens"))
            })
            .or_else(|| {
                usage
                    .get("completion_tokens_details")
                    .and_then(|item| item.get("reasoning_tokens"))
            })
            .or_else(|| usage.get("thoughtsTokenCount")),
    )
    .unwrap_or(0);

    Some(UsageCapture {
        input_tokens,
        output_tokens,
        total_tokens: if explicit_total_tokens.unwrap_or(0) == 0 {
            input_tokens
                .saturating_add(output_tokens)
                .saturating_add(reasoning_tokens)
        } else {
            explicit_total_tokens.unwrap_or(0)
        },
        cached_tokens,
        reasoning_tokens,
    })
}

fn extract_response_id(value: &Value) -> Option<String> {
    non_null_child(value, "id")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("response")
                .and_then(|item| non_null_child(item, "id"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn should_treat_response_as_stream(content_type: &str, request_is_stream: bool) -> bool {
    request_is_stream
        || content_type
            .to_ascii_lowercase()
            .contains("text/event-stream")
}

fn find_sse_frame_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    if buffer.len() < 2 {
        return None;
    }

    for index in 0..buffer.len().saturating_sub(1) {
        if index + 3 < buffer.len() && &buffer[index..index + 4] == b"\r\n\r\n" {
            return Some((index, 4));
        }
        if &buffer[index..index + 2] == b"\n\n" {
            return Some((index, 2));
        }
    }

    None
}

impl ResponseUsageCollector {
    fn new(is_stream: bool) -> Self {
        Self {
            is_stream,
            body: Vec::new(),
            stream_buffer: Vec::new(),
            usage: None,
            response_id: None,
            terminal_error: None,
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        if self.is_stream {
            self.feed_stream_chunk(chunk);
        } else {
            self.body.extend_from_slice(chunk);
        }
    }

    fn finish(mut self) -> ResponseCapture {
        if self.is_stream {
            self.process_stream_buffer(true);
            ResponseCapture {
                usage: self.usage,
                response_id: self.response_id,
                terminal_error: self.terminal_error,
            }
        } else {
            let parsed = serde_json::from_slice::<Value>(&self.body).ok();
            ResponseCapture {
                usage: parsed.as_ref().and_then(extract_usage_capture),
                response_id: parsed.as_ref().and_then(extract_response_id),
                terminal_error: None,
            }
        }
    }

    fn feed_stream_chunk(&mut self, chunk: &[u8]) {
        self.stream_buffer.extend_from_slice(chunk);
        self.process_stream_buffer(false);
    }

    fn process_stream_buffer(&mut self, flush_tail: bool) {
        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_stream_frame(&frame);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_stream_frame(&frame);
        }
    }

    fn process_stream_frame(&mut self, frame: &[u8]) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        if let Ok(value) = serde_json::from_str::<Value>(&payload) {
            if self.terminal_error.is_none() {
                if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &value)
                {
                    self.terminal_error = Some(format_upstream_response_failed_error(&signal));
                }
            }
            if let Some(usage) = extract_usage_capture(&value) {
                self.usage = Some(usage);
            }
            if self.response_id.is_none() {
                self.response_id = extract_response_id(&value);
            }
        }
    }
}

fn resolve_upstream_target(target: &str) -> Result<String, String> {
    let trimmed = if target.starts_with("/v1") {
        target.trim_start_matches("/v1")
    } else if target.starts_with(BACKEND_CODEX_PREFIX) {
        target.trim_start_matches(BACKEND_CODEX_PREFIX)
    } else {
        return Err("仅支持 /v1 或 /backend-api/codex 路径".to_string());
    };

    if trimmed.is_empty() {
        Ok("/".to_string())
    } else if trimmed.starts_with('/') {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("/{}", trimmed))
    }
}

fn account_upstream_base_url(account: &CodexAccount) -> String {
    if account.is_api_key_auth() {
        account
            .api_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_OPENAI_RESPONSES_BASE_URL)
            .trim_end_matches('/')
            .to_string()
    } else {
        UPSTREAM_CODEX_BASE_URL.to_string()
    }
}

fn account_upstream_token(account: &CodexAccount) -> Result<String, String> {
    let token = if account.is_api_key_auth() {
        account.openai_api_key.as_deref().unwrap_or_default()
    } else {
        account.tokens.access_token.as_str()
    }
    .trim();

    if token.is_empty() {
        if account.is_api_key_auth() {
            Err("API Key 账号缺少上游 API Key".to_string())
        } else {
            Err("OAuth 账号缺少 access_token".to_string())
        }
    } else {
        Ok(token.to_string())
    }
}

fn build_upstream_url(account: &CodexAccount, target: &str) -> Result<String, String> {
    let base_url = account_upstream_base_url(account);
    Url::parse(&base_url).map_err(|e| format!("上游 Base URL 无效: {}", e))?;
    Ok(format!("{}{}", base_url.trim_end_matches('/'), target))
}

fn is_stream_request(headers: &HashMap<String, String>, body: &[u8]) -> bool {
    if let Some(accept) = headers.get("accept") {
        if accept.to_ascii_lowercase().contains("text/event-stream") {
            return true;
        }
    }

    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn resolve_upstream_account_id(account: &CodexAccount) -> Option<String> {
    account
        .account_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            codex_account::extract_chatgpt_account_id_from_access_token(
                &account.tokens.access_token,
            )
        })
}

fn extract_upstream_error_message(body: &str) -> Option<String> {
    let parsed = serde_json::from_str::<Value>(body).ok()?;

    if let Some(message) = parsed
        .get("error")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
    {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed
        .get("detail")
        .and_then(|value| value.get("message"))
        .and_then(Value::as_str)
    {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed.get("message").and_then(Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(message) = parsed.get("error").and_then(Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn summarize_upstream_error(status: StatusCode, body: &str) -> String {
    let detail = extract_upstream_error_message(body).unwrap_or_else(|| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            format!("上游接口返回状态 {}", status.as_u16())
        } else {
            trimmed.to_string()
        }
    });

    format!("{}: {}", status.as_u16(), detail)
}

fn is_image_generation_capability_error(status: StatusCode, body: &str) -> bool {
    if !matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("image generation is not enabled")
        || lower.contains("image_generation is not enabled")
        || (lower.contains("image_generation") && lower.contains("not enabled"))
}

fn friendly_image_generation_capability_error(account_email: &str) -> String {
    let account_email = account_email.trim();
    if account_email.is_empty() {
        return "当前上游账号未启用图片生成能力。请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”，或换用具备图片能力的账号。".to_string();
    }
    format!(
        "账号 {} 未启用图片生成能力。请在 API 服务里将 image_generation 改为“仅图片接口启用”或“禁用”，或换用具备图片能力的账号。",
        account_email
    )
}

fn classify_upstream_error_category(status: StatusCode, body: &str) -> Option<&'static str> {
    if is_image_generation_capability_error(status, body) {
        return Some("image_generation_not_enabled");
    }
    if status == StatusCode::UNAUTHORIZED {
        return Some("auth_unavailable");
    }
    if parse_codex_retry_after(status, body).is_some() {
        return Some("usage_limit_reached");
    }
    let lower = body.to_ascii_lowercase();
    if lower.contains("context length")
        || lower.contains("context_length")
        || lower.contains("context_too_large")
        || lower.contains("too many tokens")
    {
        return Some("context_too_large");
    }
    if lower.contains("selected model is at capacity") || lower.contains("model is at capacity") {
        return Some("model_capacity");
    }
    None
}

fn should_try_next_account(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::UNAUTHORIZED {
        return true;
    }
    if is_image_generation_capability_error(status, body) {
        return true;
    }
    if matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) {
        return true;
    }

    let lower = body.to_ascii_lowercase();
    let quota_exhausted = lower.contains("usage_limit_reached")
        || lower.contains("limit reached")
        || lower.contains("insufficient_quota")
        || lower.contains("quota exceeded")
        || lower.contains("quota exceeded");
    let model_capacity =
        lower.contains("selected model is at capacity") || lower.contains("model is at capacity");

    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS | StatusCode::FORBIDDEN
    ) && (quota_exhausted || model_capacity)
}

fn json_response(status: u16, status_text: &str, body: &Value) -> Vec<u8> {
    let body_bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        status,
        status_text,
        body_bytes.len(),
        CORS_ALLOW_HEADERS
    );
    let mut response = headers.into_bytes();
    response.extend_from_slice(&body_bytes);
    response
}

fn gateway_error_code(status: u16) -> &'static str {
    match status {
        400 => "bad_request",
        401 => "unauthorized",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        429 => "rate_limited",
        502 => "upstream_unavailable",
        503 => "service_unavailable",
        _ => "codex_local_access_error",
    }
}

fn gateway_proxy_diagnostics_message(diagnostics: &UpstreamProxyDiagnostics) -> String {
    match diagnostics.proxy_source {
        UpstreamProxySource::ApiService => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => format!("当前使用 API 代理地址：{}。", proxy_url),
            None => "当前 API 代理地址为空。".to_string(),
        },
        UpstreamProxySource::Global => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => format!("当前 API 代理地址为空，已跟随全局代理：{}。", proxy_url),
            None => "当前 API 代理地址为空，已尝试跟随全局代理。".to_string(),
        },
        UpstreamProxySource::SystemEnv => match diagnostics.proxy_url.as_deref() {
            Some(proxy_url) => {
                format!(
                    "当前 API 代理地址为空，且全局代理未启用或未配置，已使用环境代理：{}。",
                    proxy_url
                )
            }
            None => {
                "当前 API 代理地址为空，且全局代理未启用或未配置，已尝试使用环境代理。".to_string()
            }
        },
        UpstreamProxySource::SystemAuto => {
            "当前 API 代理地址为空，且全局代理与环境代理均未配置，已回退到系统自动代理配置；如仍失败，请在 API 代理地址中填写 Clash 的 HTTP/mixed 端口。".to_string()
        }
    }
}

fn upstream_proxy_source_code(source: UpstreamProxySource) -> &'static str {
    match source {
        UpstreamProxySource::ApiService => "api_service",
        UpstreamProxySource::Global => "global",
        UpstreamProxySource::SystemEnv => "system_env",
        UpstreamProxySource::SystemAuto => "system_auto",
    }
}

fn gateway_user_visible_error_message(
    status: u16,
    message: &str,
    proxy_diagnostics: Option<&UpstreamProxyDiagnostics>,
) -> String {
    if status != StatusCode::BAD_GATEWAY.as_u16() {
        return message.to_string();
    }

    let proxy_context = proxy_diagnostics
        .map(|diagnostics| format!(" {}", gateway_proxy_diagnostics_message(diagnostics)))
        .unwrap_or_default();
    format!(
        "Codex API 服务连接上游失败。API 代理地址留空时会依次使用全局代理、环境代理、系统自动代理；如需固定出口，建议填写 API 代理地址（例如 http://127.0.0.1:7890）后重试。{} 如果 Codex 客户端仍显示 502 且 API 服务没有请求记录，请检查代理工具是否拦截或屏蔽 localhost / 127.0.0.1。原始错误：{}",
        proxy_context, message
    )
}

fn gateway_error_body(
    status: u16,
    message: &str,
    proxy_diagnostics: Option<&UpstreamProxyDiagnostics>,
) -> Value {
    let mut error = Map::new();
    error.insert(
        "message".to_string(),
        Value::String(gateway_user_visible_error_message(
            status,
            message,
            proxy_diagnostics,
        )),
    );
    error.insert(
        "type".to_string(),
        Value::String("codex_local_access_error".to_string()),
    );
    error.insert(
        "code".to_string(),
        Value::String(gateway_error_code(status).to_string()),
    );
    error.insert("status".to_string(), json!(status));

    if let Some(diagnostics) = proxy_diagnostics {
        error.insert(
            "upstreamProxy".to_string(),
            json!({
                "source": upstream_proxy_source_code(diagnostics.proxy_source),
                "proxyUrl": diagnostics.proxy_url.clone(),
            }),
        );
    }

    let mut body = Map::new();
    body.insert("error".to_string(), Value::Object(error));
    Value::Object(body)
}

fn options_response() -> Vec<u8> {
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 0\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        CORS_ALLOW_HEADERS
    );
    headers.into_bytes()
}

fn log_field_or_dash(value: Option<&str>) -> &str {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("-")
}

fn escape_failure_detail(detail: &str) -> String {
    detail.replace('\r', "\\r").replace('\n', "\\n")
}

fn log_codex_api_failure(
    addr: Option<&std::net::SocketAddr>,
    request: Option<&ParsedRequest>,
    status: Option<u16>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    latency_ms: Option<u64>,
    detail: &str,
) {
    let addr_text = addr
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let status_text = status
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let latency_text = latency_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_string());
    let method = request.map(|value| value.method.as_str()).unwrap_or("-");
    let target = request.map(|value| value.target.as_str()).unwrap_or("-");

    logger::log_codex_api_warn(&format!(
        "[CodexLocalAccess][Failure] addr={} method={} target={} status={} account_id={} account_email={} latency_ms={} detail={}",
        addr_text,
        method,
        target,
        status_text,
        log_field_or_dash(account_id),
        log_field_or_dash(account_email),
        latency_text,
        escape_failure_detail(detail),
    ));
}

async fn write_json_error_response(
    stream: &mut TcpStream,
    addr: Option<&std::net::SocketAddr>,
    request: Option<&ParsedRequest>,
    status: u16,
    status_text: &str,
    message: &str,
    account_id: Option<&str>,
    account_email: Option<&str>,
    latency_ms: Option<u64>,
) -> Result<(), String> {
    log_codex_api_failure(
        addr,
        request,
        Some(status),
        account_id,
        account_email,
        latency_ms,
        message,
    );

    let response = json_response(
        status,
        status_text,
        &gateway_error_body(status, message, None),
    );
    stream
        .write_all(&response)
        .await
        .map_err(|e| format!("写入错误响应失败: {}", e))
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n\r\n",
        status,
        status_text,
        content_type,
        body.len(),
        CORS_ALLOW_HEADERS
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|e| format!("写入响应头失败: {}", e))?;
    stream
        .write_all(body)
        .await
        .map_err(|e| format!("写入响应体失败: {}", e))?;
    Ok(())
}

async fn write_chunked_response_headers(
    stream: &mut TcpStream,
    status: StatusCode,
    status_text: &str,
    content_type: &str,
    upstream_headers: &reqwest::header::HeaderMap,
) -> Result<(), String> {
    let mut response_headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: {}\r\n",
        status.as_u16(),
        status_text,
        content_type,
        CORS_ALLOW_HEADERS
    );

    for header_name in ["x-request-id", "openai-processing-ms"] {
        if let Some(value) = upstream_headers
            .get(header_name)
            .and_then(|item| item.to_str().ok())
        {
            response_headers.push_str(&format!("{}: {}\r\n", header_name, value));
        }
    }

    response_headers.push_str("\r\n");
    stream
        .write_all(response_headers.as_bytes())
        .await
        .map_err(|e| format!("写入响应头失败: {}", e))
}

async fn write_chunked_response_chunk(stream: &mut TcpStream, chunk: &[u8]) -> Result<(), String> {
    if chunk.is_empty() {
        return Ok(());
    }

    let prefix = format!("{:X}\r\n", chunk.len());
    stream
        .write_all(prefix.as_bytes())
        .await
        .map_err(|e| format!("写入响应分块前缀失败: {}", e))?;
    stream
        .write_all(chunk)
        .await
        .map_err(|e| format!("写入响应分块失败: {}", e))?;
    stream
        .write_all(b"\r\n")
        .await
        .map_err(|e| format!("写入响应分块结束失败: {}", e))
}

async fn finish_chunked_response(stream: &mut TcpStream) -> Result<(), String> {
    stream
        .write_all(b"0\r\n\r\n")
        .await
        .map_err(|e| format!("写入响应结束失败: {}", e))
}

fn parse_responses_payload_from_upstream(body_bytes: &[u8]) -> Result<Value, String> {
    if let Ok(parsed) = serde_json::from_slice::<Value>(body_bytes) {
        if let Some(signal) = upstream_response_failed_signal(None, &parsed) {
            return Err(format_upstream_response_failed_error(&signal));
        }
        return Ok(parsed);
    }

    let mut stream_buffer = body_bytes.to_vec();
    let mut completed_response: Option<Value> = None;
    let mut output_text = String::new();
    let mut output_items: Vec<Value> = Vec::new();

    let mut process_frame = |frame: &[u8]| -> Result<(), String> {
        if frame.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(());
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };
        if payload == "[DONE]" {
            return Ok(());
        }

        let Ok(value) = serde_json::from_str::<Value>(&payload) else {
            return Ok(());
        };
        if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &value) {
            return Err(format_upstream_response_failed_error(&signal));
        }
        match value
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("")
        {
            "response.output_text.delta" => {
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    output_text.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if output_text.trim().is_empty() {
                    if let Some(done_text) = value.get("text").and_then(Value::as_str) {
                        output_text.push_str(done_text);
                    }
                }
            }
            "response.output_item.done" => {
                if let Some(item) = value.get("item") {
                    output_items.push(item.clone());
                }
            }
            event_type if is_responses_completion_event(event_type) => {
                if let Some(response) = value.get("response") {
                    completed_response = Some(response.clone());
                } else {
                    completed_response = Some(value.clone());
                }
            }
            _ => {}
        }
        Ok(())
    };

    loop {
        let Some((boundary_index, separator_len)) = find_sse_frame_boundary(&stream_buffer) else {
            break;
        };
        let frame = stream_buffer[..boundary_index].to_vec();
        stream_buffer.drain(..boundary_index + separator_len);
        process_frame(&frame)?;
    }
    if !stream_buffer.is_empty() {
        process_frame(&stream_buffer)?;
    }

    let Some(response_value) = completed_response else {
        return Err(
            "解析上游 responses 响应失败: 非 JSON 且未捕获 response.completed/response.done"
                .to_string(),
        );
    };

    let mut root = Map::new();
    match response_value {
        Value::Object(mut response_object) => {
            if response_object
                .get("output")
                .and_then(Value::as_array)
                .map(|items| items.is_empty())
                .unwrap_or(true)
                && !output_items.is_empty()
            {
                response_object.insert("output".to_string(), Value::Array(output_items));
            }
            if !output_text.trim().is_empty() {
                response_object.insert("output_text".to_string(), Value::String(output_text));
            }
            root.insert("response".to_string(), Value::Object(response_object));
        }
        other => {
            root.insert("response".to_string(), other);
            if !output_items.is_empty() {
                root.insert("output".to_string(), Value::Array(output_items));
            }
            if !output_text.trim().is_empty() {
                root.insert("output_text".to_string(), Value::String(output_text));
            }
        }
    }

    Ok(Value::Object(root))
}

fn mime_type_from_output_format(output_format: &str) -> String {
    let output_format = output_format.trim();
    if output_format.contains('/') {
        return output_format.to_string();
    }
    match output_format.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        "webp" => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

fn extract_images_from_responses_payload(
    response_body: &Value,
) -> (
    Vec<ImageCallResult>,
    i64,
    Option<Value>,
    Option<ImageCallResult>,
) {
    let root = response_payload_root(response_body);
    let created = root
        .get("created_at")
        .or_else(|| root.get("created"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let mut results = Vec::new();
    let mut first_meta = None;

    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
                continue;
            }
            let result = item
                .get("result")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let Some(result) = result else {
                continue;
            };
            let entry = ImageCallResult {
                result: result.to_string(),
                revised_prompt: item
                    .get("revised_prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                output_format: item
                    .get("output_format")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                size: item
                    .get("size")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                background: item
                    .get("background")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
                quality: item
                    .get("quality")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            };
            if first_meta.is_none() {
                first_meta = Some(entry.clone());
            }
            results.push(entry);
        }
    }

    let usage = root
        .get("tool_usage")
        .and_then(|tool_usage| tool_usage.get("image_gen"))
        .filter(|value| value.is_object())
        .cloned();

    (results, created, usage, first_meta)
}

fn build_images_api_payload(response_body: &Value, response_format: &str) -> Result<Value, String> {
    let (results, created, usage, first_meta) =
        extract_images_from_responses_payload(response_body);
    if results.is_empty() {
        return Err("upstream did not return image output".to_string());
    }

    let response_format = if response_format.trim().is_empty() {
        "b64_json"
    } else {
        response_format.trim()
    };
    let mut data = Vec::new();
    for image in results {
        let mut item = Map::new();
        if response_format.eq_ignore_ascii_case("url") {
            let mime_type = mime_type_from_output_format(&image.output_format);
            item.insert(
                "url".to_string(),
                Value::String(format!("data:{};base64,{}", mime_type, image.result)),
            );
        } else {
            item.insert("b64_json".to_string(), Value::String(image.result));
        }
        if !image.revised_prompt.is_empty() {
            item.insert(
                "revised_prompt".to_string(),
                Value::String(image.revised_prompt),
            );
        }
        data.push(Value::Object(item));
    }

    let mut out = Map::new();
    out.insert("created".to_string(), json!(created));
    out.insert("data".to_string(), Value::Array(data));

    if let Some(meta) = first_meta {
        if !meta.background.is_empty() {
            out.insert("background".to_string(), Value::String(meta.background));
        }
        if !meta.output_format.is_empty() {
            out.insert(
                "output_format".to_string(),
                Value::String(meta.output_format),
            );
        }
        if !meta.quality.is_empty() {
            out.insert("quality".to_string(), Value::String(meta.quality));
        }
        if !meta.size.is_empty() {
            out.insert("size".to_string(), Value::String(meta.size));
        }
    }
    if let Some(usage) = usage {
        out.insert("usage".to_string(), usage);
    }

    Ok(Value::Object(out))
}

fn push_named_sse_payload(stream_body: &mut String, event_name: &str, payload: Value) {
    let event_name = event_name.trim();
    if !event_name.is_empty() {
        stream_body.push_str("event: ");
        stream_body.push_str(event_name);
        stream_body.push('\n');
    }
    push_sse_payload(stream_body, payload);
}

#[derive(Debug)]
struct ImageStreamTransformer {
    response_format: String,
    stream_prefix: String,
    stream_buffer: Vec<u8>,
    response_capture: ResponseCapture,
}

impl ImageStreamTransformer {
    fn new(response_format: &str, stream_prefix: &str) -> Self {
        Self {
            response_format: if response_format.trim().is_empty() {
                "b64_json".to_string()
            } else {
                response_format.trim().to_ascii_lowercase()
            },
            stream_prefix: stream_prefix.to_string(),
            stream_buffer: Vec::new(),
            response_capture: ResponseCapture::default(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }
        self.stream_buffer.extend_from_slice(chunk);
        self.process_buffer(false)
    }

    fn finish(mut self) -> (Vec<u8>, ResponseCapture) {
        let output = self.process_buffer(true);
        (output, self.response_capture)
    }

    fn process_buffer(&mut self, flush_tail: bool) -> Vec<u8> {
        let mut stream_body = String::new();

        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_frame(&frame, &mut stream_body);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_frame(&frame, &mut stream_body);
        }

        stream_body.into_bytes()
    }

    fn process_frame(&mut self, frame: &[u8], stream_body: &mut String) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        let Ok(event) = serde_json::from_str::<Value>(&payload) else {
            return;
        };
        if self.response_capture.terminal_error.is_none() {
            if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &event) {
                self.response_capture.terminal_error =
                    Some(format_upstream_response_failed_error(&signal));
            }
        }
        if let Some(usage) = extract_usage_capture(&event) {
            self.response_capture.usage = Some(usage);
        }
        if self.response_capture.response_id.is_none() {
            self.response_capture.response_id = extract_response_id(&event);
        }

        match event
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("")
        {
            "response.image_generation_call.partial_image" => {
                let Some(b64) = event
                    .get("partial_image_b64")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return;
                };
                let output_format = event
                    .get("output_format")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let event_name = format!("{}.partial_image", self.stream_prefix);
                let mut data = Map::new();
                data.insert("type".to_string(), Value::String(event_name.clone()));
                data.insert(
                    "partial_image_index".to_string(),
                    json!(event
                        .get("partial_image_index")
                        .and_then(Value::as_i64)
                        .unwrap_or(0)),
                );
                if self.response_format == "url" {
                    let mime_type = mime_type_from_output_format(output_format);
                    data.insert(
                        "url".to_string(),
                        Value::String(format!("data:{};base64,{}", mime_type, b64)),
                    );
                } else {
                    data.insert("b64_json".to_string(), Value::String(b64.to_string()));
                }
                push_named_sse_payload(stream_body, &event_name, Value::Object(data));
            }
            event_type if is_responses_completion_event(event_type) => {
                let (results, _, usage, _) = extract_images_from_responses_payload(&event);
                if results.is_empty() {
                    push_named_sse_payload(
                        stream_body,
                        "error",
                        json!({ "error": "upstream did not return image output" }),
                    );
                    return;
                }
                let event_name = format!("{}.completed", self.stream_prefix);
                for image in results {
                    let mut data = Map::new();
                    data.insert("type".to_string(), Value::String(event_name.clone()));
                    if self.response_format == "url" {
                        let mime_type = mime_type_from_output_format(&image.output_format);
                        data.insert(
                            "url".to_string(),
                            Value::String(format!("data:{};base64,{}", mime_type, image.result)),
                        );
                    } else {
                        data.insert("b64_json".to_string(), Value::String(image.result));
                    }
                    if let Some(usage) = usage.clone() {
                        data.insert("usage".to_string(), usage);
                    }
                    push_named_sse_payload(stream_body, &event_name, Value::Object(data));
                }
            }
            _ => {}
        }
    }
}

async fn write_chat_completions_compatible_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    stream_mode: bool,
    requested_model: &str,
    original_request_body: &[u8],
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let upstream_headers = upstream.headers().clone();

    if stream_mode {
        write_chunked_response_headers(
            stream,
            status,
            status_text,
            "text/event-stream; charset=utf-8",
            &upstream_headers,
        )
        .await?;

        let mut transformer =
            ChatCompletionStreamTransformer::new(original_request_body, requested_model);
        let mut body_stream = upstream.bytes_stream();
        let stream_started_at = Instant::now();
        let mut first_chunk_logged = false;
        loop {
            let stream_total_timeout = duration_from_millis(
                timeouts.legacy_stream_total_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
            );
            if stream_started_at.elapsed() > stream_total_timeout {
                let message = format!(
                    "读取上游流式响应超时: 总时长超过 {} 秒",
                    stream_total_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_total_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                return Err(message);
            }

            let stream_idle_timeout = duration_from_millis(
                timeouts.legacy_stream_idle_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
            );
            let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
                .await
                .map_err(|_| {
                    let message = format!(
                        "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                        stream_idle_timeout.as_secs()
                    );
                    legacy_debug_log(
                        debug_logs,
                        format!(
                            "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                            request.method,
                            request.target,
                            started_at.elapsed().as_millis(),
                            message
                        ),
                    );
                    message
                })?;
            let Some(chunk_result) = next_chunk else {
                break;
            };
            let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
            if chunk.is_empty() {
                continue;
            }
            if !first_chunk_logged {
                first_chunk_logged = true;
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        chunk.len()
                    ),
                );
            }
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
        if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    status.as_u16(),
                    started_at.elapsed().as_millis(),
                    escape_failure_detail(&terminal_error)
                ),
            );
            return Err(terminal_error.to_string());
        }
        legacy_debug_log(
            debug_logs,
            format!(
                "stream_completed method={} target={} status={} latency_ms={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis()
            ),
        );
        return Ok(response_capture);
    }

    let body_bytes = upstream
        .bytes()
        .await
        .map_err(|e| format!("读取上游 responses 响应失败: {}", e))?;
    let parsed = parse_responses_payload_from_upstream(&body_bytes)?;
    let response_capture = ResponseCapture {
        usage: extract_usage_capture(&parsed),
        response_id: extract_response_id(&parsed),
        terminal_error: None,
    };
    let chat_payload =
        build_chat_completion_payload(&parsed, requested_model, original_request_body);

    let payload_bytes = serde_json::to_vec(&chat_payload)
        .map_err(|e| format!("序列化 chat/completions 响应失败: {}", e))?;
    write_http_response(
        stream,
        status.as_u16(),
        status_text,
        "application/json; charset=utf-8",
        &payload_bytes,
    )
    .await?;

    Ok(response_capture)
}

async fn write_images_compatible_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    stream_mode: bool,
    response_format: &str,
    stream_prefix: &str,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let upstream_headers = upstream.headers().clone();

    if stream_mode {
        write_chunked_response_headers(
            stream,
            status,
            status_text,
            "text/event-stream; charset=utf-8",
            &upstream_headers,
        )
        .await?;

        let mut transformer = ImageStreamTransformer::new(response_format, stream_prefix);
        let mut body_stream = upstream.bytes_stream();
        let stream_started_at = Instant::now();
        let mut first_chunk_logged = false;
        loop {
            let stream_total_timeout = duration_from_millis(
                timeouts.legacy_stream_total_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
            );
            if stream_started_at.elapsed() > stream_total_timeout {
                let message = format!(
                    "读取上游流式响应超时: 总时长超过 {} 秒",
                    stream_total_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_total_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                return Err(message);
            }

            let stream_idle_timeout = duration_from_millis(
                timeouts.legacy_stream_idle_timeout_ms,
                DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
            );
            let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
                .await
                .map_err(|_| {
                    let message = format!(
                        "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                        stream_idle_timeout.as_secs()
                    );
                    legacy_debug_log(
                        debug_logs,
                        format!(
                            "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                            request.method,
                            request.target,
                            started_at.elapsed().as_millis(),
                            message
                        ),
                    );
                    message
                })?;
            let Some(chunk_result) = next_chunk else {
                break;
            };
            let chunk = chunk_result.map_err(|e| format!("读取上游图片响应失败: {}", e))?;
            if chunk.is_empty() {
                continue;
            }
            if !first_chunk_logged {
                first_chunk_logged = true;
                legacy_debug_log(
                    debug_logs,
                    format!(
                        "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        chunk.len()
                    ),
                );
            }
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
        if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    status.as_u16(),
                    started_at.elapsed().as_millis(),
                    escape_failure_detail(&terminal_error)
                ),
            );
            return Err(terminal_error.to_string());
        }
        legacy_debug_log(
            debug_logs,
            format!(
                "stream_completed method={} target={} status={} latency_ms={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis()
            ),
        );
        return Ok(response_capture);
    }

    let body_bytes = upstream
        .bytes()
        .await
        .map_err(|e| format!("读取上游图片响应失败: {}", e))?;
    let parsed = parse_responses_payload_from_upstream(&body_bytes)?;
    let response_capture = ResponseCapture {
        usage: extract_usage_capture(&parsed),
        response_id: extract_response_id(&parsed),
        terminal_error: None,
    };
    let images_payload = build_images_api_payload(&parsed, response_format)?;
    let payload_bytes = serde_json::to_vec(&images_payload)
        .map_err(|e| format!("序列化 images 响应失败: {}", e))?;

    write_http_response(
        stream,
        status.as_u16(),
        status_text,
        "application/json; charset=utf-8",
        &payload_bytes,
    )
    .await?;

    Ok(response_capture)
}

async fn write_gateway_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    response_adapter: GatewayResponseAdapter,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    match response_adapter {
        GatewayResponseAdapter::Passthrough { request_is_stream } => {
            write_upstream_response(
                stream,
                upstream,
                request_is_stream,
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
        GatewayResponseAdapter::ChatCompletions {
            stream: stream_mode,
            requested_model,
            original_request_body,
        } => {
            write_chat_completions_compatible_response(
                stream,
                upstream,
                stream_mode,
                requested_model.as_str(),
                original_request_body.as_slice(),
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
        GatewayResponseAdapter::Images {
            stream: stream_mode,
            response_format,
            stream_prefix,
        } => {
            write_images_compatible_response(
                stream,
                upstream,
                stream_mode,
                response_format.as_str(),
                stream_prefix.as_str(),
                debug_logs,
                request,
                started_at,
                timeouts,
            )
            .await
        }
    }
}

async fn write_upstream_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    request_is_stream: bool,
    debug_logs: bool,
    request: &ParsedRequest,
    started_at: Instant,
    timeouts: &CodexLocalAccessTimeouts,
) -> Result<ResponseCapture, String> {
    let status = upstream.status();
    let status_text = status.canonical_reason().unwrap_or("OK");
    let headers = upstream.headers().clone();
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8");
    let is_stream = should_treat_response_as_stream(content_type, request_is_stream);
    write_chunked_response_headers(stream, status, status_text, content_type, &headers).await?;

    let mut usage_collector = ResponseUsageCollector::new(is_stream);
    let mut body_stream = upstream.bytes_stream();
    let stream_started_at = Instant::now();
    let mut first_chunk_logged = false;
    loop {
        let stream_total_timeout = duration_from_millis(
            timeouts.legacy_stream_total_timeout_ms,
            DEFAULT_UPSTREAM_STREAM_TOTAL_TIMEOUT,
        );
        if stream_started_at.elapsed() > stream_total_timeout {
            let message = format!(
                "读取上游流式响应超时: 总时长超过 {} 秒",
                stream_total_timeout.as_secs()
            );
            legacy_debug_log(
                debug_logs && is_stream,
                format!(
                    "stream_total_timeout method={} target={} latency_ms={} detail={}",
                    request.method,
                    request.target,
                    started_at.elapsed().as_millis(),
                    message
                ),
            );
            return Err(message);
        }
        let stream_idle_timeout = duration_from_millis(
            timeouts.legacy_stream_idle_timeout_ms,
            DEFAULT_UPSTREAM_STREAM_IDLE_TIMEOUT,
        );
        let next_chunk = tokio::time::timeout(stream_idle_timeout, body_stream.next())
            .await
            .map_err(|_| {
                let message = format!(
                    "读取上游流式响应超时: 连续 {} 秒未收到新数据",
                    stream_idle_timeout.as_secs()
                );
                legacy_debug_log(
                    debug_logs && is_stream,
                    format!(
                        "stream_idle_timeout method={} target={} latency_ms={} detail={}",
                        request.method,
                        request.target,
                        started_at.elapsed().as_millis(),
                        message
                    ),
                );
                message
            })?;
        let Some(chunk_result) = next_chunk else {
            break;
        };
        let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
        if chunk.is_empty() {
            continue;
        }
        if is_stream && !first_chunk_logged {
            first_chunk_logged = true;
            legacy_debug_log(
                debug_logs,
                format!(
                    "stream_first_chunk method={} target={} latency_ms={} bytes={}",
                    request.method,
                    request.target,
                    started_at.elapsed().as_millis(),
                    chunk.len()
                ),
            );
        }
        write_chunked_response_chunk(stream, &chunk).await?;
        usage_collector.feed(&chunk);
    }

    finish_chunked_response(stream).await?;
    let response_capture = usage_collector.finish();
    if let Some(terminal_error) = response_capture.terminal_error.as_deref() {
        legacy_debug_log(
            debug_logs && is_stream,
            format!(
                "stream_upstream_failed method={} target={} status={} latency_ms={} detail={}",
                request.method,
                request.target,
                status.as_u16(),
                started_at.elapsed().as_millis(),
                escape_failure_detail(&terminal_error)
            ),
        );
        return Err(terminal_error.to_string());
    }
    legacy_debug_log(
        debug_logs && is_stream,
        format!(
            "stream_completed method={} target={} status={} latency_ms={}",
            request.method,
            request.target,
            status.as_u16(),
            started_at.elapsed().as_millis()
        ),
    );
    Ok(response_capture)
}

async fn force_refresh_gateway_account(account_id: &str) -> Result<CodexAccount, String> {
    let account =
        codex_account::force_refresh_managed_account(account_id, "本地网关上游返回 401").await?;
    cache_prepared_account(&account).await;
    Ok(account)
}

fn should_retry_upstream_send_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect() || error.is_request()
}

fn format_reqwest_error_chain(error: &reqwest::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = StdError::source(error);
    while let Some(err) = source {
        let detail = err.to_string();
        if !detail.trim().is_empty() && parts.last().map(|item| item != &detail).unwrap_or(true) {
            parts.push(detail);
        }
        source = StdError::source(err);
    }
    parts.join(" | caused by: ")
}

fn format_upstream_network_error(error: &reqwest::Error) -> String {
    format!(
        "Codex 上游网络或代理不可用，未能连接到所选账号的上游服务。请检查网络、代理配置以及账号 Base URL 可访问性。技术细节: {}",
        format_reqwest_error_chain(error)
    )
}

fn backoff_retry_delay(retry_attempt: usize, base_delay_ms: u64, max_delay_ms: u64) -> Duration {
    let multiplier = match retry_attempt {
        0 | 1 => 1u32,
        2 => 2u32,
        _ => 4u32,
    };
    let base = Duration::from_millis(base_delay_ms);
    let max = Duration::from_millis(max_delay_ms);
    let delay = base.saturating_mul(multiplier);
    if delay > max {
        max
    } else {
        delay
    }
}

fn should_retry_single_account_upstream_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    )
}

fn build_account_scoped_upstream_body<'a>(
    target: &str,
    body: &'a [u8],
    account: &CodexAccount,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<Cow<'a, [u8]>, String> {
    if !is_responses_request(target) {
        return Ok(Cow::Borrowed(body));
    }

    let Some(mut body_value) = parse_request_body_json(body) else {
        return Ok(Cow::Borrowed(body));
    };
    let Some(body_obj) = body_value.as_object_mut() else {
        return Ok(Cow::Borrowed(body));
    };

    if !image_generation_tools_allowed(image_generation_mode, request_kind) {
        if !remove_image_generation_tool_from_object(body_obj) {
            return Ok(Cow::Borrowed(body));
        }
        return serde_json::to_vec(&body_value)
            .map(Cow::Owned)
            .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e));
    }

    if is_free_plan_type(account.plan_type.as_deref())
        || !ensure_image_generation_tool_in_object(body_obj)
    {
        return Ok(Cow::Borrowed(body));
    }

    serde_json::to_vec(&body_value)
        .map(Cow::Owned)
        .map_err(|e| format!("序列化账号级 responses 请求体失败: {}", e))
}

async fn send_upstream_request(
    method: &str,
    target: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    account: &CodexAccount,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
    timeouts: &CodexLocalAccessTimeouts,
    image_generation_mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<reqwest::Response, String> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|e| format!("不支持的请求方法: {}", e))?;
    let url = build_upstream_url(account, target)?;
    let upstream_token = account_upstream_token(account)?;
    let client = upstream_http_client(upstream_proxy_url, connect_timeout)?;
    let upstream_body = build_account_scoped_upstream_body(
        target,
        body,
        account,
        image_generation_mode,
        request_kind,
    )?;
    let max_send_retries = timeouts.upstream_send_retry_attempts as usize;
    for retry_attempt in 0..=max_send_retries {
        let mut request = client.request(method.clone(), &url);

        for (name, value) in headers {
            if matches!(
                name.as_str(),
                "authorization"
                    | "host"
                    | "content-length"
                    | "connection"
                    | "accept-encoding"
                    | "proxy-connection"
                    | "x-api-key"
                    | "x-agtools-local-request-kind"
            ) {
                continue;
            }
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("无效请求头 {}: {}", name, e))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| format!("无效请求头值 {}: {}", name, e))?;
            request = request.header(header_name, header_value);
        }

        request = request.header(AUTHORIZATION, format!("Bearer {}", upstream_token));
        if !account.is_api_key_auth() && !headers.contains_key("user-agent") {
            request = request.header(USER_AGENT, DEFAULT_CODEX_USER_AGENT);
        }
        if !account.is_api_key_auth() && !headers.contains_key("originator") {
            request = request.header("Originator", DEFAULT_CODEX_ORIGINATOR);
        }
        if !account.is_api_key_auth() {
            if let Some(account_id) = resolve_upstream_account_id(account) {
                request = request.header("ChatGPT-Account-Id", account_id);
            }
        }
        if !headers.contains_key("accept") {
            request = request.header(
                ACCEPT,
                if is_stream_request(headers, upstream_body.as_ref()) {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            );
        }
        request = request.header("Connection", "Keep-Alive");
        if !headers.contains_key("content-type") && !upstream_body.is_empty() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        if !upstream_body.is_empty() {
            request = request.body(upstream_body.as_ref().to_vec());
        }

        match request.send().await {
            Ok(response) => return Ok(response),
            Err(error) => {
                let should_retry =
                    retry_attempt < max_send_retries && should_retry_upstream_send_error(&error);
                if !should_retry {
                    return Err(format_upstream_network_error(&error));
                }
                tokio::time::sleep(backoff_retry_delay(
                    retry_attempt + 1,
                    timeouts.upstream_send_retry_base_delay_ms,
                    timeouts.upstream_send_retry_max_delay_ms,
                ))
                .await;
            }
        }
    }

    Err("请求 Codex 上游失败: 未知错误".to_string())
}

async fn proxy_request_with_account_pool(
    request: &ParsedRequest,
    collection: &CodexLocalAccessCollection,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<ProxyDispatchSuccess, ProxyDispatchError> {
    if collection.account_ids.is_empty() {
        return Err(ProxyDispatchError {
            status: 503,
            message: "本地接入集合暂无账号".to_string(),
            account_id: None,
            account_email: None,
            error_category: Some("no_accounts".to_string()),
        });
    }

    let upstream_target =
        resolve_upstream_target(&request.target).map_err(|err| ProxyDispatchError {
            status: 400,
            message: err,
            account_id: None,
            account_email: None,
            error_category: Some("bad_request".to_string()),
        })?;
    let timeouts = collection_timeouts(collection);
    let upstream_connect_timeout = duration_from_millis(
        timeouts.legacy_upstream_connect_timeout_ms,
        DEFAULT_UPSTREAM_CONNECT_TIMEOUT,
    );
    let routing_hint = build_request_routing_hint(request);
    let total = collection.account_ids.len();
    let configured_max_credentials = collection.max_retry_credentials as usize;
    let max_credential_attempts = if configured_max_credentials == 0 {
        total
    } else {
        configured_max_credentials.min(total)
    }
    .min(MAX_RETRY_CREDENTIALS_PER_REQUEST)
    .max(1);
    let session_affinity_key = routing_hint
        .session_affinity_key
        .as_deref()
        .filter(|_| collection.session_affinity)
        .map(session_affinity_binding_key);
    let affinity_account_id = if let Some(session_key) = session_affinity_key.as_deref() {
        resolve_affinity_account(session_key).await
    } else {
        match routing_hint.previous_response_id.as_deref() {
            Some(previous_response_id) => resolve_affinity_account(previous_response_id).await,
            None => None,
        }
    };
    let mut last_status = 503u16;
    let mut last_error = "本地接入集合暂无可用账号".to_string();
    let mut last_error_category: Option<String> = None;
    let mut last_account_id: Option<String> = None;
    let mut last_account_email: Option<String> = None;
    let mut attempts = 0usize;
    let mut retry_round = 0usize;
    let mut earliest_cooldown_wait: Option<Duration>;

    loop {
        let start = GATEWAY_ROUND_ROBIN_CURSOR.fetch_add(1, Ordering::Relaxed);
        let ordered_account_ids =
            if collection.routing_strategy == CodexLocalAccessRoutingStrategy::Custom {
                collection.account_ids.clone()
            } else {
                build_ordered_account_ids(
                    &collection.account_ids,
                    start,
                    affinity_account_id.as_deref(),
                )
            };
        let strategy_account_ids = pin_account_to_front(
            apply_routing_strategy(
                &ordered_account_ids,
                collection.routing_strategy,
                &collection.custom_routing_rules,
                start,
            ),
            affinity_account_id.as_deref(),
        );
        let mut attempted_in_round = false;
        let mut round_cooldown_wait: Option<Duration> = None;

        for account_id in strategy_account_ids {
            if attempts >= max_credential_attempts {
                break;
            }

            if !collection.disable_cooling {
                if let Some(wait) =
                    get_model_cooldown_wait(&account_id, &routing_hint.model_key).await
                {
                    round_cooldown_wait = Some(match round_cooldown_wait {
                        Some(current) if current <= wait => current,
                        _ => wait,
                    });
                    continue;
                }
            }

            attempted_in_round = true;
            attempts += 1;

            let mut account = match get_prepared_account(&account_id).await {
                Ok(account) => account,
                Err(err) => {
                    invalidate_prepared_account(&account_id).await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        None,
                        Some(account_id.as_str()),
                        None,
                        None,
                        format!("账号预处理失败: {}", err).as_str(),
                    );
                    last_error = err;
                    last_error_category = Some("account_prepare_failed".to_string());
                    continue;
                }
            };

            if collection.restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref())
            {
                mark_account_failure(
                    &account,
                    None,
                    Some("free_account_restricted"),
                    "Free 账号不支持加入本地接入",
                    request_kind,
                )
                .await;
                log_codex_api_failure(
                    None,
                    Some(request),
                    None,
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    "Free 账号不支持加入本地接入",
                );
                last_error = "Free 账号不支持加入本地接入".to_string();
                last_error_category = Some("free_account_restricted".to_string());
                continue;
            }

            last_account_id = Some(account.id.clone());
            last_account_email = Some(account.email.clone());
            legacy_debug_log(
                collection.debug_logs,
                format!(
                    "account_selected method={} target={} request_kind={} account_id={} account_email={} attempt={}/{}",
                    request.method,
                    request.target,
                    request_kind_log_label(request_kind),
                    account.id,
                    account.email,
                    attempts,
                    max_credential_attempts
                ),
            );

            let mut single_account_status_retry_attempt = 0usize;
            loop {
                let upstream_send_started_at = Instant::now();
                legacy_debug_log(
                    collection.debug_logs,
                    format!(
                        "upstream_send_started method={} target={} request_kind={} account_id={} account_email={} retry_attempt={}",
                        request.method,
                        request.target,
                        request_kind_log_label(request_kind),
                        account.id,
                        account.email,
                        single_account_status_retry_attempt
                    ),
                );
                let first_response = send_upstream_request(
                    &request.method,
                    &upstream_target,
                    &request.headers,
                    &request.body,
                    &account,
                    collection.upstream_proxy_url.as_deref(),
                    upstream_connect_timeout,
                    &timeouts,
                    collection.image_generation_mode,
                    request_kind,
                )
                .await;

                let mut response = match first_response {
                    Ok(response) => {
                        legacy_debug_log(
                            collection.debug_logs,
                            format!(
                                "upstream_response_headers method={} target={} status={} account_id={} account_email={} upstream_latency_ms={}",
                                request.method,
                                request.target,
                                response.status().as_u16(),
                                account.id,
                                account.email,
                                upstream_send_started_at.elapsed().as_millis()
                            ),
                        );
                        response
                    }
                    Err(err) => {
                        legacy_debug_log(
                            collection.debug_logs,
                            format!(
                                "upstream_send_failed method={} target={} account_id={} account_email={} upstream_latency_ms={} detail={}",
                                request.method,
                                request.target,
                                account.id,
                                account.email,
                                upstream_send_started_at.elapsed().as_millis(),
                                escape_failure_detail(&err)
                            ),
                        );
                        last_status = StatusCode::BAD_GATEWAY.as_u16();
                        mark_account_failure(
                            &account,
                            Some(last_status),
                            Some("upstream_network"),
                            &err,
                            request_kind,
                        )
                        .await;
                        log_codex_api_failure(
                            None,
                            Some(request),
                            Some(last_status),
                            Some(account.id.as_str()),
                            Some(account.email.as_str()),
                            None,
                            format!("上游请求失败: {}", err).as_str(),
                        );
                        last_error = err;
                        last_error_category = Some("upstream_network".to_string());
                        break;
                    }
                };

                if response.status() == StatusCode::UNAUTHORIZED && account.is_api_key_auth() {
                    last_status = StatusCode::UNAUTHORIZED.as_u16();
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(last_status),
                        Some("auth_unavailable"),
                        "API Key 账号上游鉴权失败",
                        request_kind,
                    )
                    .await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        Some(last_status),
                        Some(account.id.as_str()),
                        Some(account.email.as_str()),
                        None,
                        format!("API Key 账号 {} 上游鉴权失败", account.email).as_str(),
                    );
                    last_error = format!("API Key 账号 {} 上游鉴权失败", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    break;
                }

                if response.status() == StatusCode::UNAUTHORIZED
                    && !account_has_refresh_token(&account)
                {
                    last_status = StatusCode::UNAUTHORIZED.as_u16();
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(last_status),
                        Some("auth_unavailable"),
                        "access-token-only 账号的 access_token 已被上游拒绝",
                        request_kind,
                    )
                    .await;
                    log_codex_api_failure(
                        None,
                        Some(request),
                        Some(last_status),
                        Some(account.id.as_str()),
                        Some(account.email.as_str()),
                        None,
                        format!(
                            "上游返回 401，access-token-only 账号的 access_token 已不可用，按普通账号路径轮转: {}",
                            account.email
                        )
                        .as_str(),
                    );
                    last_error = format!("账号 {} 当前 access_token 已被上游拒绝", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    break;
                }

                if response.status() == StatusCode::UNAUTHORIZED {
                    match force_refresh_gateway_account(&account_id).await {
                        Ok(refreshed_account) => {
                            account = refreshed_account;
                            response = match send_upstream_request(
                                &request.method,
                                &upstream_target,
                                &request.headers,
                                &request.body,
                                &account,
                                collection.upstream_proxy_url.as_deref(),
                                upstream_connect_timeout,
                                &timeouts,
                                collection.image_generation_mode,
                                request_kind,
                            )
                            .await
                            {
                                Ok(response) => response,
                                Err(err) => {
                                    last_status = StatusCode::BAD_GATEWAY.as_u16();
                                    log_codex_api_failure(
                                        None,
                                        Some(request),
                                        Some(last_status),
                                        Some(account.id.as_str()),
                                        Some(account.email.as_str()),
                                        None,
                                        format!("刷新后重试上游失败: {}", err).as_str(),
                                    );
                                    last_error = err;
                                    last_error_category = Some("upstream_network".to_string());
                                    break;
                                }
                            };

                            if response.status() == StatusCode::UNAUTHORIZED {
                                last_status = StatusCode::UNAUTHORIZED.as_u16();
                                invalidate_prepared_account(&account_id).await;
                                mark_account_failure(
                                    &account,
                                    Some(last_status),
                                    Some("auth_unavailable"),
                                    "账号鉴权失败",
                                    request_kind,
                                )
                                .await;
                                log_codex_api_failure(
                                    None,
                                    Some(request),
                                    Some(last_status),
                                    Some(account.id.as_str()),
                                    Some(account.email.as_str()),
                                    None,
                                    format!("账号 {} 鉴权失败", account.email).as_str(),
                                );
                                last_error = format!("账号 {} 鉴权失败", account.email);
                                last_error_category = Some("auth_unavailable".to_string());
                                break;
                            }
                        }
                        Err(err) => {
                            last_status = StatusCode::UNAUTHORIZED.as_u16();
                            invalidate_prepared_account(&account_id).await;
                            mark_account_failure(
                                &account,
                                Some(last_status),
                                Some("auth_refresh_failed"),
                                &err,
                                request_kind,
                            )
                            .await;
                            log_codex_api_failure(
                                None,
                                Some(request),
                                Some(StatusCode::UNAUTHORIZED.as_u16()),
                                Some(account.id.as_str()),
                                Some(account.email.as_str()),
                                None,
                                format!("账号刷新失败: {}", err).as_str(),
                            );
                            last_error = err;
                            last_error_category = Some("auth_refresh_failed".to_string());
                            break;
                        }
                    }
                }

                if response.status().is_success() {
                    clear_model_cooldown(&account.id, &routing_hint.model_key).await;
                    mark_account_success(&account, request_kind).await;
                    return Ok(ProxyDispatchSuccess {
                        upstream: response,
                        account_id: account.id.clone(),
                        account_email: account.email.clone(),
                    });
                }

                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let category = classify_upstream_error_category(status, &body);
                let message = if category == Some("image_generation_not_enabled") {
                    friendly_image_generation_capability_error(&account.email)
                } else {
                    summarize_upstream_error(status, &body)
                };
                mark_account_failure(
                    &account,
                    Some(status.as_u16()),
                    category,
                    &message,
                    request_kind,
                )
                .await;
                log_codex_api_failure(
                    None,
                    Some(request),
                    Some(status.as_u16()),
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    format!("上游返回失败: {}", message).as_str(),
                );

                if !collection.disable_cooling {
                    if let Some(retry_after) = parse_codex_retry_after(status, &body) {
                        set_model_cooldown(
                            &account.id,
                            &routing_hint.model_key,
                            retry_after,
                            "usage_limit_reached",
                        )
                        .await;
                        round_cooldown_wait = Some(match round_cooldown_wait {
                            Some(current) if current <= retry_after => current,
                            _ => retry_after,
                        });
                    }
                }

                let can_retry_single_account = total == 1
                    && single_account_status_retry_attempt
                        < timeouts.single_account_status_retry_attempts as usize
                    && should_retry_single_account_upstream_status(status);
                if can_retry_single_account {
                    single_account_status_retry_attempt += 1;
                    tokio::time::sleep(backoff_retry_delay(
                        single_account_status_retry_attempt,
                        timeouts.single_account_status_retry_base_delay_ms,
                        timeouts.single_account_status_retry_max_delay_ms,
                    ))
                    .await;
                    continue;
                }

                if should_try_next_account(status, &body) {
                    last_status = status.as_u16();
                    last_error = if category == Some("image_generation_not_enabled") {
                        message.clone()
                    } else {
                        format!("账号 {} 当前不可用，已尝试轮转: {}", account.email, message)
                    };
                    last_error_category = category.map(str::to_string);
                    break;
                }

                return Err(ProxyDispatchError {
                    status: status.as_u16(),
                    message,
                    account_id: Some(account.id.clone()),
                    account_email: Some(account.email.clone()),
                    error_category: category.map(str::to_string),
                });
            }
        }

        earliest_cooldown_wait = round_cooldown_wait;
        let Some(wait) = earliest_cooldown_wait else {
            break;
        };
        let max_retry_wait = Duration::from_millis(
            collection
                .max_retry_interval_ms
                .clamp(MAX_RETRY_INTERVAL_MIN_MS, MAX_RETRY_INTERVAL_MAX_MS),
        );
        if attempts >= max_credential_attempts
            || retry_round >= MAX_REQUEST_RETRY_ATTEMPTS
            || wait > max_retry_wait
        {
            if !attempted_in_round {
                return Err(ProxyDispatchError {
                    status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
                    message: build_cooldown_unavailable_message(&routing_hint.model_key, wait),
                    account_id: affinity_account_id.clone(),
                    account_email: None,
                    error_category: Some("cooldown".to_string()),
                });
            }
            break;
        }

        tokio::time::sleep(wait).await;
        retry_round += 1;
    }

    Err(ProxyDispatchError {
        status: if last_status == 503 {
            earliest_cooldown_wait
                .map(|_| StatusCode::TOO_MANY_REQUESTS.as_u16())
                .unwrap_or(last_status)
        } else {
            last_status
        },
        message: if matches!(last_status, 429 | 503) {
            earliest_cooldown_wait
                .map(|wait| build_cooldown_unavailable_message(&routing_hint.model_key, wait))
                .unwrap_or(last_error)
        } else {
            last_error
        },
        account_id: last_account_id,
        account_email: last_account_email,
        error_category: last_error_category,
    })
}

fn is_websocket_upgrade_request(request: &ParsedRequest) -> bool {
    let upgrade = header_value(&request.headers, "upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection = header_value(&request.headers, "connection")
        .map(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    upgrade && connection && header_value(&request.headers, "sec-websocket-key").is_some()
}

fn websocket_accept_value(sec_websocket_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(sec_websocket_key.trim().as_bytes());
    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    general_purpose::STANDARD.encode(hasher.finalize())
}

async fn accept_downstream_websocket(
    mut stream: TcpStream,
    request: &ParsedRequest,
) -> Result<WebSocketStream<TcpStream>, String> {
    let sec_key = header_value(&request.headers, "sec-websocket-key")
        .ok_or_else(|| "WebSocket 握手缺少 Sec-WebSocket-Key".to_string())?;
    let accept_value = websocket_accept_value(sec_key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        accept_value
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("写入 WebSocket 握手响应失败: {}", e))?;
    Ok(WebSocketStream::from_raw_socket(stream, Role::Server, None).await)
}

async fn read_initial_websocket_payload(
    downstream: &mut WebSocketStream<TcpStream>,
    initial_message_timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + initial_message_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("等待 WebSocket 首个 response.create 消息超时".to_string());
        }
        let message = timeout(remaining, downstream.next())
            .await
            .map_err(|_| "等待 WebSocket 首个 response.create 消息超时".to_string())?
            .ok_or_else(|| "客户端在发送首个 WebSocket 消息前已断开".to_string())?
            .map_err(|e| format!("读取 WebSocket 首个消息失败: {}", e))?;

        match message {
            Message::Text(text) => return Ok(text.to_string().into_bytes()),
            Message::Binary(bytes) => return Ok(bytes.to_vec()),
            Message::Ping(bytes) => {
                downstream
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|e| format!("回复 WebSocket Ping 失败: {}", e))?;
            }
            Message::Pong(_) => {}
            Message::Close(frame) => {
                let _ = downstream.send(Message::Close(frame)).await;
                return Err("客户端在发送首个 WebSocket 消息前已关闭连接".to_string());
            }
            _ => {}
        }
    }
}

fn prepare_websocket_initial_request(
    request: &mut ParsedRequest,
    api_key: &ResolvedLocalApiKey,
) -> Result<(), String> {
    let mut body_value = parse_request_body_json(&request.body)
        .ok_or_else(|| "WebSocket response.create 消息必须是合法 JSON".to_string())?;
    rewrite_request_model_alias_value(&mut body_value);
    codex_protocol::normalize_responses_body_for_codex(&mut body_value);
    let body_obj = body_value
        .as_object_mut()
        .ok_or_else(|| "WebSocket response.create 消息必须是 JSON 对象".to_string())?;
    body_obj.insert(
        "type".to_string(),
        Value::String("response.create".to_string()),
    );
    request.body = serde_json::to_vec(&body_value)
        .map_err(|e| format!("序列化 WebSocket response.create 消息失败: {}", e))?;
    request
        .headers
        .insert("content-type".to_string(), "application/json".to_string());
    align_codex_prompt_cache(request, api_key)?;
    apply_codex_official_headers(request);
    Ok(())
}

fn build_upstream_websocket_url(account: &CodexAccount, target: &str) -> Result<String, String> {
    let http_url = build_upstream_url(account, target)?;
    let mut parsed =
        Url::parse(&http_url).map_err(|e| format!("上游 WebSocket URL 无效: {}", e))?;
    let next_scheme = match parsed.scheme() {
        "http" => "ws",
        "https" => "wss",
        other => return Err(format!("上游 WebSocket 不支持 {} 协议", other)),
    };
    parsed
        .set_scheme(next_scheme)
        .map_err(|_| "切换上游 WebSocket 协议失败".to_string())?;
    Ok(parsed.to_string())
}

fn should_skip_websocket_upstream_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "host"
            | "content-length"
            | "connection"
            | "upgrade"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-protocol"
            | "sec-websocket-extensions"
            | "accept-encoding"
            | "proxy-connection"
            | "x-api-key"
            | "x-agtools-local-request-kind"
    )
}

fn websocket_header_value(value: impl Into<String>) -> Result<WsHeaderValue, String> {
    WsHeaderValue::from_str(&value.into()).map_err(|e| format!("无效 WebSocket 请求头值: {}", e))
}

fn websocket_target_host_port(request: &WsClientRequest) -> Result<(String, u16), String> {
    let uri = request.uri();
    let host = uri
        .host()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "上游 WebSocket URL 缺少 Host".to_string())?
        .to_string();
    let port = uri
        .port_u16()
        .or_else(|| match uri.scheme_str() {
            Some("wss") => Some(443),
            Some("ws") => Some(80),
            _ => None,
        })
        .ok_or_else(|| "上游 WebSocket URL 缺少端口".to_string())?;
    Ok((host, port))
}

async fn tcp_connect_with_timeout(
    addr: &str,
    label: &str,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    timeout(connect_timeout, TcpStream::connect(addr))
        .await
        .map_err(|_| format!("连接 {} 超时", label))?
        .map_err(|e| format!("连接 {} 失败: {}", label, e))
}

fn decode_proxy_credential(value: &str) -> String {
    urlencoding::decode(value)
        .map(Cow::into_owned)
        .unwrap_or_else(|_| value.to_string())
}

fn proxy_authorization_header(proxy_url: &Url) -> Option<String> {
    if proxy_url.username().is_empty() {
        return None;
    }
    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let credential = general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    Some(format!("Proxy-Authorization: Basic {}\r\n", credential))
}

async fn connect_http_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket 上游代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket 上游代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket HTTP 代理", connect_timeout).await?;
    let target_addr = format!("{}:{}", target_host, target_port);
    let auth_header = proxy_authorization_header(proxy_url).unwrap_or_default();
    let request = format!(
        "CONNECT {target_addr} HTTP/1.1\r\nHost: {target_addr}\r\nProxy-Connection: Keep-Alive\r\n{auth_header}\r\n"
    );
    timeout(connect_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "发送 WebSocket 代理 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket 代理 CONNECT 请求失败: {}", e))?;

    let mut response = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if response.len() > CODEX_WEBSOCKET_PROXY_CONNECT_MAX_BYTES {
            return Err("WebSocket 代理 CONNECT 响应过大".to_string());
        }
        let read = timeout(connect_timeout, stream.read(&mut chunk))
            .await
            .map_err(|_| "读取 WebSocket 代理 CONNECT 响应超时".to_string())?
            .map_err(|e| format!("读取 WebSocket 代理 CONNECT 响应失败: {}", e))?;
        if read == 0 {
            return Err("WebSocket 代理在 CONNECT 完成前关闭连接".to_string());
        }
        response.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_header_end(&response) {
            let header_text = String::from_utf8_lossy(&response[..header_end]);
            let status_line = header_text
                .lines()
                .next()
                .ok_or_else(|| "WebSocket 代理 CONNECT 响应为空".to_string())?;
            let status = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| format!("WebSocket 代理 CONNECT 响应状态无效: {}", status_line))?;
            if (200..300).contains(&status) {
                return Ok(stream);
            }
            return Err(format!("WebSocket 代理 CONNECT 失败: HTTP {}", status));
        }
    }
}

async fn socks5_read_exact(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    connect_timeout: Duration,
) -> Result<(), String> {
    timeout(connect_timeout, stream.read_exact(buffer))
        .await
        .map_err(|_| "读取 WebSocket SOCKS5 代理响应超时".to_string())?
        .map_err(|e| format!("读取 WebSocket SOCKS5 代理响应失败: {}", e))?;
    Ok(())
}

async fn connect_socks5_proxy_tunnel(
    proxy_url: &Url,
    target_host: &str,
    target_port: u16,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let proxy_host = proxy_url
        .host_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少 Host".to_string())?;
    let proxy_port = proxy_url
        .port_or_known_default()
        .ok_or_else(|| "WebSocket SOCKS5 代理地址缺少端口".to_string())?;
    let proxy_addr = format!("{}:{}", proxy_host, proxy_port);
    let mut stream =
        tcp_connect_with_timeout(&proxy_addr, "WebSocket SOCKS5 代理", connect_timeout).await?;

    let username = decode_proxy_credential(proxy_url.username());
    let password = proxy_url
        .password()
        .map(decode_proxy_credential)
        .unwrap_or_default();
    let use_auth = !username.is_empty();
    let greeting: &[u8] = if use_auth {
        &[0x05, 0x02, 0x00, 0x02]
    } else {
        &[0x05, 0x01, 0x00]
    };
    timeout(connect_timeout, stream.write_all(greeting))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 握手超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 握手失败: {}", e))?;

    let mut method_response = [0u8; 2];
    socks5_read_exact(&mut stream, &mut method_response, connect_timeout).await?;
    if method_response[0] != 0x05 {
        return Err("WebSocket SOCKS5 代理响应版本无效".to_string());
    }
    if method_response[1] == 0xff {
        return Err("WebSocket SOCKS5 代理不接受当前认证方式".to_string());
    }
    if method_response[1] == 0x02 {
        let username_bytes = username.as_bytes();
        let password_bytes = password.as_bytes();
        if username_bytes.len() > u8::MAX as usize || password_bytes.len() > u8::MAX as usize {
            return Err("WebSocket SOCKS5 代理用户名或密码过长".to_string());
        }
        let mut auth_request = Vec::with_capacity(3 + username_bytes.len() + password_bytes.len());
        auth_request.push(0x01);
        auth_request.push(username_bytes.len() as u8);
        auth_request.extend_from_slice(username_bytes);
        auth_request.push(password_bytes.len() as u8);
        auth_request.extend_from_slice(password_bytes);
        timeout(connect_timeout, stream.write_all(&auth_request))
            .await
            .map_err(|_| "发送 WebSocket SOCKS5 认证超时".to_string())?
            .map_err(|e| format!("发送 WebSocket SOCKS5 认证失败: {}", e))?;
        let mut auth_response = [0u8; 2];
        socks5_read_exact(&mut stream, &mut auth_response, connect_timeout).await?;
        if auth_response != [0x01, 0x00] {
            return Err("WebSocket SOCKS5 代理认证失败".to_string());
        }
    } else if method_response[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 代理返回不支持的认证方式: {}",
            method_response[1]
        ));
    }

    let target_host_bytes = target_host.as_bytes();
    if target_host_bytes.len() > u8::MAX as usize {
        return Err("WebSocket SOCKS5 目标 Host 过长".to_string());
    }
    let mut connect_request = Vec::with_capacity(7 + target_host_bytes.len());
    connect_request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, target_host_bytes.len() as u8]);
    connect_request.extend_from_slice(target_host_bytes);
    connect_request.extend_from_slice(&target_port.to_be_bytes());
    timeout(connect_timeout, stream.write_all(&connect_request))
        .await
        .map_err(|_| "发送 WebSocket SOCKS5 CONNECT 请求超时".to_string())?
        .map_err(|e| format!("发送 WebSocket SOCKS5 CONNECT 请求失败: {}", e))?;

    let mut reply_header = [0u8; 4];
    socks5_read_exact(&mut stream, &mut reply_header, connect_timeout).await?;
    if reply_header[0] != 0x05 {
        return Err("WebSocket SOCKS5 CONNECT 响应版本无效".to_string());
    }
    if reply_header[1] != 0x00 {
        return Err(format!(
            "WebSocket SOCKS5 CONNECT 失败，状态码 {}",
            reply_header[1]
        ));
    }
    let addr_len = match reply_header[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0u8; 1];
            socks5_read_exact(&mut stream, &mut len, connect_timeout).await?;
            len[0] as usize
        }
        0x04 => 16,
        other => return Err(format!("WebSocket SOCKS5 CONNECT 地址类型无效: {}", other)),
    };
    let mut bound_addr = vec![0u8; addr_len + 2];
    socks5_read_exact(&mut stream, &mut bound_addr, connect_timeout).await?;
    Ok(stream)
}

async fn connect_upstream_websocket_socket(
    request: &WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<TcpStream, String> {
    let (target_host, target_port) = websocket_target_host_port(request)?;
    let signature = current_upstream_http_client_signature(upstream_proxy_url, connect_timeout);
    let Some(proxy_url) = signature.proxy_url.as_deref() else {
        return tcp_connect_with_timeout(
            &format!("{}:{}", target_host, target_port),
            "Codex 上游 WebSocket",
            connect_timeout,
        )
        .await;
    };
    let proxy_url =
        Url::parse(proxy_url).map_err(|e| format!("WebSocket 上游代理地址无效: {}", e))?;
    match proxy_url.scheme() {
        "http" => {
            connect_http_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout).await
        }
        "socks5" | "socks5h" => {
            connect_socks5_proxy_tunnel(&proxy_url, &target_host, target_port, connect_timeout)
                .await
        }
        "https" => {
            Err("WebSocket 上游代理暂不支持 https 代理，请改用 http 或 socks5 代理地址".to_string())
        }
        other => Err(format!("WebSocket 上游代理不支持 {} 协议", other)),
    }
}

impl WebSocketConnectError {
    fn upstream(message: String) -> Self {
        Self {
            status: None,
            message,
            category: "upstream_websocket".to_string(),
        }
    }
}

fn websocket_connect_error_from_http_response(
    status: StatusCode,
    body: String,
) -> WebSocketConnectError {
    let category = classify_upstream_error_category(status, &body)
        .unwrap_or("upstream_websocket")
        .to_string();
    let message = if body.trim().is_empty() {
        format!("Codex 上游 WebSocket 握手失败: HTTP {}", status.as_u16())
    } else {
        format!(
            "Codex 上游 WebSocket 握手失败: {}",
            summarize_upstream_error(status, &body)
        )
    };
    WebSocketConnectError {
        status: Some(status.as_u16()),
        message,
        category,
    }
}

fn websocket_connect_error_from_tungstenite(error: WsError) -> WebSocketConnectError {
    match error {
        WsError::Http(response) => {
            let status =
                StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let body = response
                .body()
                .as_deref()
                .map(String::from_utf8_lossy)
                .map(Cow::into_owned)
                .unwrap_or_default();
            websocket_connect_error_from_http_response(status, body)
        }
        other => {
            WebSocketConnectError::upstream(format!("连接 Codex 上游 WebSocket 失败: {}", other))
        }
    }
}

async fn connect_upstream_websocket_request(
    request: WsClientRequest,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let socket = connect_upstream_websocket_socket(&request, upstream_proxy_url, connect_timeout)
        .await
        .map_err(WebSocketConnectError::upstream)?;
    let (upstream, _) = client_async_tls_with_config(request, socket, None, None)
        .await
        .map_err(websocket_connect_error_from_tungstenite)?;
    Ok(upstream)
}

async fn connect_upstream_websocket(
    request: &ParsedRequest,
    account: &CodexAccount,
    upstream_target: &str,
    upstream_proxy_url: Option<&str>,
    connect_timeout: Duration,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, WebSocketConnectError> {
    let ws_url = build_upstream_websocket_url(account, upstream_target)
        .map_err(WebSocketConnectError::upstream)?;
    let upstream_token =
        account_upstream_token(account).map_err(WebSocketConnectError::upstream)?;
    let mut upstream_request = ws_url.as_str().into_client_request().map_err(|e| {
        WebSocketConnectError::upstream(format!("创建上游 WebSocket 请求失败: {}", e))
    })?;

    for (name, value) in &request.headers {
        if should_skip_websocket_upstream_header(name.as_str()) {
            continue;
        }
        let header_name = WsHeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            WebSocketConnectError::upstream(format!("无效 WebSocket 请求头 {}: {}", name, e))
        })?;
        let header_value =
            websocket_header_value(value.clone()).map_err(WebSocketConnectError::upstream)?;
        upstream_request
            .headers_mut()
            .insert(header_name, header_value);
    }

    upstream_request.headers_mut().insert(
        "Authorization",
        websocket_header_value(format!("Bearer {}", upstream_token))
            .map_err(WebSocketConnectError::upstream)?,
    );
    if !account.is_api_key_auth() && header_value(&request.headers, "user-agent").is_none() {
        upstream_request.headers_mut().insert(
            "User-Agent",
            websocket_header_value(DEFAULT_CODEX_USER_AGENT)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    if !account.is_api_key_auth() && header_value(&request.headers, "originator").is_none() {
        upstream_request.headers_mut().insert(
            "Originator",
            websocket_header_value(DEFAULT_CODEX_ORIGINATOR)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    let beta_header = header_value(&request.headers, "openai-beta").unwrap_or_default();
    if !beta_header.contains("responses_websockets=") {
        upstream_request.headers_mut().insert(
            "OpenAI-Beta",
            websocket_header_value(CODEX_RESPONSES_WEBSOCKET_BETA_HEADER_VALUE)
                .map_err(WebSocketConnectError::upstream)?,
        );
    }
    if !account.is_api_key_auth() {
        if let Some(account_id) = resolve_upstream_account_id(account) {
            upstream_request.headers_mut().insert(
                "ChatGPT-Account-Id",
                websocket_header_value(account_id).map_err(WebSocketConnectError::upstream)?,
            );
        }
    }

    connect_upstream_websocket_request(upstream_request, upstream_proxy_url, connect_timeout).await
}

async fn proxy_websocket_with_account_pool(
    request: &ParsedRequest,
    collection: &CodexLocalAccessCollection,
    request_kind: CodexLocalAccessRequestKind,
) -> Result<WebSocketDispatchSuccess, ProxyDispatchError> {
    if collection.account_ids.is_empty() {
        return Err(ProxyDispatchError {
            status: 503,
            message: "本地接入集合暂无账号".to_string(),
            account_id: None,
            account_email: None,
            error_category: Some("no_accounts".to_string()),
        });
    }

    let upstream_target =
        resolve_upstream_target(&request.target).map_err(|err| ProxyDispatchError {
            status: 400,
            message: err,
            account_id: None,
            account_email: None,
            error_category: Some("bad_request".to_string()),
        })?;
    let timeouts = collection_timeouts(collection);
    let websocket_connect_timeout = duration_from_millis(
        timeouts.websocket_connect_timeout_ms,
        CODEX_WEBSOCKET_CONNECT_TIMEOUT,
    );
    let routing_hint = build_request_routing_hint(request);
    let total = collection.account_ids.len();
    let configured_max_credentials = collection.max_retry_credentials as usize;
    let max_credential_attempts = if configured_max_credentials == 0 {
        total
    } else {
        configured_max_credentials.min(total)
    }
    .min(MAX_RETRY_CREDENTIALS_PER_REQUEST)
    .max(1);
    let start = GATEWAY_ROUND_ROBIN_CURSOR.fetch_add(1, Ordering::Relaxed);
    let session_affinity_key = routing_hint
        .session_affinity_key
        .as_deref()
        .filter(|_| collection.session_affinity)
        .map(session_affinity_binding_key);
    let affinity_account_id = if let Some(session_key) = session_affinity_key.as_deref() {
        resolve_affinity_account(session_key).await
    } else {
        None
    };
    let ordered_account_ids =
        if collection.routing_strategy == CodexLocalAccessRoutingStrategy::Custom {
            collection.account_ids.clone()
        } else {
            build_ordered_account_ids(
                &collection.account_ids,
                start,
                affinity_account_id.as_deref(),
            )
        };
    let strategy_account_ids = pin_account_to_front(
        apply_routing_strategy(
            &ordered_account_ids,
            collection.routing_strategy,
            &collection.custom_routing_rules,
            start,
        ),
        affinity_account_id.as_deref(),
    );

    let mut attempts = 0usize;
    let mut last_status = StatusCode::BAD_GATEWAY.as_u16();
    let mut last_error = "本地接入集合暂无可用账号".to_string();
    let mut last_error_category: Option<String> = None;
    let mut last_account_id: Option<String> = None;
    let mut last_account_email: Option<String> = None;

    for account_id in strategy_account_ids {
        if attempts >= max_credential_attempts {
            break;
        }
        if !collection.disable_cooling {
            if get_model_cooldown_wait(&account_id, &routing_hint.model_key)
                .await
                .is_some()
            {
                continue;
            }
        }
        attempts += 1;

        let mut account = match get_prepared_account(&account_id).await {
            Ok(account) => account,
            Err(err) => {
                invalidate_prepared_account(&account_id).await;
                last_status = StatusCode::BAD_GATEWAY.as_u16();
                last_error = err;
                last_error_category = Some("account_prepare_failed".to_string());
                continue;
            }
        };
        if collection.restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref()) {
            mark_account_failure(
                &account,
                None,
                Some("free_account_restricted"),
                "Free 账号不支持加入本地接入",
                request_kind,
            )
            .await;
            last_error = "Free 账号不支持加入本地接入".to_string();
            last_error_category = Some("free_account_restricted".to_string());
            continue;
        }

        last_account_id = Some(account.id.clone());
        last_account_email = Some(account.email.clone());

        match connect_upstream_websocket(
            request,
            &account,
            &upstream_target,
            collection.upstream_proxy_url.as_deref(),
            websocket_connect_timeout,
        )
        .await
        {
            Ok(upstream) => {
                return Ok(WebSocketDispatchSuccess {
                    upstream,
                    account_id: account.id.clone(),
                    account_email: account.email.clone(),
                    account,
                });
            }
            Err(err) => {
                let status = err.status.unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
                if status == StatusCode::UNAUTHORIZED.as_u16() && account.is_api_key_auth() {
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(status),
                        Some("auth_unavailable"),
                        "API Key 账号上游 WebSocket 鉴权失败",
                        request_kind,
                    )
                    .await;
                    last_status = status;
                    last_error = format!("API Key 账号 {} 上游 WebSocket 鉴权失败", account.email);
                    last_error_category = Some("auth_unavailable".to_string());
                    continue;
                }

                if status == StatusCode::UNAUTHORIZED.as_u16()
                    && !account_has_refresh_token(&account)
                {
                    invalidate_prepared_account(&account_id).await;
                    mark_account_failure(
                        &account,
                        Some(status),
                        Some("auth_unavailable"),
                        "access-token-only 账号的 WebSocket access_token 已被上游拒绝",
                        request_kind,
                    )
                    .await;
                    last_status = status;
                    last_error = format!(
                        "账号 {} 当前 WebSocket access_token 已被上游拒绝",
                        account.email
                    );
                    last_error_category = Some("auth_unavailable".to_string());
                    continue;
                }

                if status == StatusCode::UNAUTHORIZED.as_u16() {
                    match force_refresh_gateway_account(&account_id).await {
                        Ok(refreshed_account) => {
                            account = refreshed_account;
                            match connect_upstream_websocket(
                                request,
                                &account,
                                &upstream_target,
                                collection.upstream_proxy_url.as_deref(),
                                websocket_connect_timeout,
                            )
                            .await
                            {
                                Ok(upstream) => {
                                    return Ok(WebSocketDispatchSuccess {
                                        upstream,
                                        account_id: account.id.clone(),
                                        account_email: account.email.clone(),
                                        account,
                                    });
                                }
                                Err(retry_err) => {
                                    let retry_status = retry_err
                                        .status
                                        .unwrap_or(StatusCode::BAD_GATEWAY.as_u16());
                                    let retry_category =
                                        if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                            "auth_unavailable"
                                        } else {
                                            retry_err.category.as_str()
                                        };
                                    if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                        invalidate_prepared_account(&account_id).await;
                                    }
                                    mark_account_failure(
                                        &account,
                                        Some(retry_status),
                                        Some(retry_category),
                                        &retry_err.message,
                                        request_kind,
                                    )
                                    .await;
                                    last_status = retry_status;
                                    last_error =
                                        if retry_status == StatusCode::UNAUTHORIZED.as_u16() {
                                            format!("账号 {} WebSocket 鉴权失败", account.email)
                                        } else {
                                            retry_err.message
                                        };
                                    last_error_category = Some(retry_category.to_string());
                                }
                            }
                        }
                        Err(refresh_err) => {
                            invalidate_prepared_account(&account_id).await;
                            mark_account_failure(
                                &account,
                                Some(status),
                                Some("auth_refresh_failed"),
                                &refresh_err,
                                request_kind,
                            )
                            .await;
                            last_status = status;
                            last_error = refresh_err;
                            last_error_category = Some("auth_refresh_failed".to_string());
                        }
                    }
                    continue;
                }

                mark_account_failure(
                    &account,
                    Some(status),
                    Some(err.category.as_str()),
                    &err.message,
                    request_kind,
                )
                .await;
                last_status = status;
                last_error = err.message;
                last_error_category = Some(err.category);
            }
        }
    }

    Err(ProxyDispatchError {
        status: last_status,
        message: last_error,
        account_id: last_account_id,
        account_email: last_account_email,
        error_category: last_error_category,
    })
}

fn websocket_capture_from_message(message: &Message, capture: &mut ResponseCapture) {
    let parsed = match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    };
    let Some(value) = parsed else {
        return;
    };
    if let Some(usage) = extract_usage_capture(&value) {
        capture.usage = Some(usage);
    }
    if capture.response_id.is_none() {
        capture.response_id = extract_response_id(&value);
    }
}

fn websocket_message_value(message: &Message) -> Option<Value> {
    match message {
        Message::Text(text) => serde_json::from_str::<Value>(&text.to_string()).ok(),
        Message::Binary(bytes) => serde_json::from_slice::<Value>(bytes.as_ref()).ok(),
        _ => None,
    }
}

fn websocket_error_status(value: &Value) -> Option<u16> {
    for key in ["status", "status_code"] {
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|status| u16::try_from(status).ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
        if let Some(status) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .and_then(|status| status.parse::<u16>().ok())
            .filter(|status| *status > 0)
        {
            return Some(status);
        }
    }

    None
}

fn build_websocket_error_body(value: &Value, status: u16) -> Value {
    let mut out = Map::new();
    out.insert("status".to_string(), json!(status));

    if let Some(body) = value.get("body") {
        out.insert("body".to_string(), body.clone());
        if let Some(error) = body.get("error") {
            out.insert("error".to_string(), error.clone());
            return Value::Object(out);
        }
    }

    if let Some(error) = value.get("error") {
        out.insert("error".to_string(), error.clone());
        return Value::Object(out);
    }

    out.insert(
        "error".to_string(),
        json!({
            "type": "server_error",
            "message": format!("HTTP {}", status),
        }),
    );
    Value::Object(out)
}

fn retry_after_duration_from_value(value: &Value) -> Option<Duration> {
    if let Some(seconds) = value.as_u64() {
        return Some(Duration::from_secs(seconds));
    }
    value
        .as_str()
        .map(str::trim)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn parse_websocket_retry_after_header(value: &Value) -> Option<Duration> {
    let headers = value.get("headers")?.as_object()?;
    headers.iter().find_map(|(name, value)| {
        if name.eq_ignore_ascii_case("retry-after") {
            retry_after_duration_from_value(value)
        } else {
            None
        }
    })
}

fn websocket_error_matches(value: &Value, expected: &str) -> bool {
    for path in [
        &["error", "code"][..],
        &["error", "type"][..],
        &["body", "error", "code"][..],
        &["body", "error", "type"][..],
        &["code"][..],
        &["error"][..],
    ] {
        if extract_body_string_path(value, path).as_deref() == Some(expected) {
            return true;
        }
    }
    false
}

fn parse_websocket_upstream_error(message: &Message) -> Option<WebSocketUpstreamError> {
    let value = websocket_message_value(message)?;
    if value.get("type").and_then(Value::as_str).map(str::trim) != Some("error") {
        return None;
    }

    let status = websocket_error_status(&value)?;
    let body_value = build_websocket_error_body(&value, status);
    let body = serde_json::to_string(&body_value).unwrap_or_else(|_| value.to_string());
    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let usage_retry_after = parse_codex_retry_after(status_code, &body);
    let is_connection_limit = websocket_error_matches(&value, "websocket_connection_limit_reached");
    let category = if is_connection_limit {
        "websocket_connection_limit_reached"
    } else if usage_retry_after.is_some() || websocket_error_matches(&value, "usage_limit_reached")
    {
        "usage_limit_reached"
    } else {
        classify_upstream_error_category(status_code, &body).unwrap_or("upstream_websocket_error")
    }
    .to_string();
    let retry_after = usage_retry_after
        .or_else(|| parse_websocket_retry_after_header(&value))
        .or_else(|| is_connection_limit.then_some(Duration::ZERO));

    Some(WebSocketUpstreamError {
        status,
        body,
        category,
        retry_after,
    })
}

async fn bridge_websocket_streams(
    downstream: WebSocketStream<TcpStream>,
    mut upstream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    first_payload: Vec<u8>,
    timeouts: CodexLocalAccessTimeouts,
) -> Result<WebSocketBridgeResult, String> {
    let first_text = String::from_utf8(first_payload)
        .map_err(|e| format!("WebSocket response.create 不是合法 UTF-8: {}", e))?;
    upstream
        .send(Message::Text(first_text.into()))
        .await
        .map_err(|e| format!("发送首个 WebSocket 上游消息失败: {}", e))?;

    let (mut downstream_write, mut downstream_read) = downstream.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();
    let mut capture = ResponseCapture::default();
    let mut upstream_error = None;
    let heartbeat_interval = duration_from_millis(
        timeouts.websocket_heartbeat_interval_ms,
        CODEX_WEBSOCKET_HEARTBEAT_INTERVAL,
    );
    let idle_timeout = duration_from_millis(
        timeouts.websocket_idle_timeout_ms,
        CODEX_WEBSOCKET_IDLE_TIMEOUT,
    );
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                upstream_write
                    .send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|e| format!("发送 Codex 上游 WebSocket 心跳失败: {}", e))?;
                upstream_write
                    .flush()
                    .await
                    .map_err(|e| format!("刷新 Codex 上游 WebSocket 心跳失败: {}", e))?;
            }
            downstream_next = timeout(idle_timeout, downstream_read.next()) => {
                let downstream_next = downstream_next
                    .map_err(|_| "WebSocket 客户端空闲超时".to_string())?;
                let Some(message_result) = downstream_next else {
                    break;
                };
                let message = message_result
                    .map_err(|e| format!("读取 WebSocket 客户端消息失败: {}", e))?;
                let should_close = matches!(message, Message::Close(_));
                upstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 WebSocket 客户端消息失败: {}", e))?;
                if should_close {
                    break;
                }
            }
            upstream_next = timeout(idle_timeout, upstream_read.next()) => {
                let upstream_next = upstream_next
                    .map_err(|_| "Codex 上游 WebSocket 空闲超时".to_string())?;
                let Some(message_result) = upstream_next else {
                    break;
                };
                let message = message_result
                    .map_err(|e| format!("读取 Codex 上游 WebSocket 消息失败: {}", e))?;
                websocket_capture_from_message(&message, &mut capture);
                let parsed_upstream_error = parse_websocket_upstream_error(&message);
                let should_close = matches!(message, Message::Close(_));
                downstream_write
                    .send(message)
                    .await
                    .map_err(|e| format!("转发 Codex 上游 WebSocket 消息失败: {}", e))?;
                if let Some(error) = parsed_upstream_error {
                    upstream_error = Some(error);
                    break;
                }
                if should_close {
                    break;
                }
            }
        }
    }

    Ok(WebSocketBridgeResult {
        capture,
        upstream_error,
    })
}

async fn handle_websocket_connection(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    mut parsed: ParsedRequest,
    collection: CodexLocalAccessCollection,
    resolved_api_key: ResolvedLocalApiKey,
) -> Result<(), String> {
    let started_at = Instant::now();
    let timeouts = collection_timeouts(&collection);
    let mut downstream = accept_downstream_websocket(stream, &parsed).await?;
    let initial_message_timeout = duration_from_millis(
        timeouts.websocket_initial_message_timeout_ms,
        CODEX_WEBSOCKET_INITIAL_MESSAGE_TIMEOUT,
    );
    let initial_payload =
        match read_initial_websocket_payload(&mut downstream, initial_message_timeout).await {
            Ok(payload) => payload,
            Err(err) => {
                let _ = downstream.send(Message::Close(None)).await;
                return Err(err);
            }
        };
    parsed.body = initial_payload;
    prepare_websocket_initial_request(&mut parsed, &resolved_api_key)?;
    let stats_context = RequestStatsContext {
        request_kind: CodexLocalAccessRequestKind::Text,
        model_id: stats_model_id_for_request_kind(&parsed.body, CodexLocalAccessRequestKind::Text),
        api_key_id: resolved_api_key.id.clone(),
        api_key_label: resolved_api_key.label.clone(),
    };
    let routing_hint = build_request_routing_hint(&parsed);

    match proxy_websocket_with_account_pool(&parsed, &collection, stats_context.request_kind).await
    {
        Ok(success) => {
            let account_id = success.account_id.clone();
            let account_email = success.account_email.clone();
            let account = success.account.clone();
            let bridge_result = bridge_websocket_streams(
                downstream,
                success.upstream,
                parsed.body.clone(),
                timeouts.clone(),
            )
            .await?;
            if let Some(upstream_error) = bridge_result.upstream_error {
                mark_account_failure(
                    &account,
                    Some(upstream_error.status),
                    Some(upstream_error.category.as_str()),
                    upstream_error.body.as_str(),
                    stats_context.request_kind,
                )
                .await;
                if !collection.disable_cooling {
                    if let Some(retry_after) = upstream_error.retry_after {
                        set_model_cooldown(
                            &account_id,
                            &routing_hint.model_key,
                            retry_after,
                            upstream_error.category.as_str(),
                        )
                        .await;
                    }
                }

                let latency_ms = started_at.elapsed().as_millis() as u64;
                log_codex_api_failure(
                    Some(&addr),
                    Some(&parsed),
                    Some(upstream_error.status),
                    Some(account_id.as_str()),
                    Some(account_email.as_str()),
                    Some(latency_ms),
                    upstream_error.body.as_str(),
                );
                if let Err(err) = record_request_stats(
                    Some(account_id.as_str()),
                    Some(account_email.as_str()),
                    Some(stats_context.api_key_id.as_str()),
                    Some(stats_context.api_key_label.as_str()),
                    Some(stats_context.model_id.as_str()),
                    stats_context.request_kind,
                    false,
                    Some(upstream_error.category.as_str()),
                    latency_ms,
                    bridge_result.capture.usage,
                )
                .await
                {
                    logger::log_codex_api_warn(&format!(
                        "[CodexLocalAccess] 写入 WebSocket 上游失败统计失败: {}",
                        err
                    ));
                }
                return Ok(());
            }

            clear_model_cooldown(&account_id, &routing_hint.model_key).await;
            mark_account_success(&account, stats_context.request_kind).await;
            if let Some(response_id) = bridge_result.capture.response_id.as_deref() {
                bind_response_affinity(response_id, &account_id).await;
            }
            if collection.session_affinity {
                let session_key = routing_hint
                    .session_affinity_key
                    .clone()
                    .map(|key| session_affinity_binding_key(&key));
                if let Some(session_key) = session_key.as_deref() {
                    bind_response_affinity(session_key, &account_id).await;
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u64;
            if let Err(err) = record_request_stats(
                Some(account_id.as_str()),
                Some(account_email.as_str()),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                true,
                None,
                latency_ms,
                bridge_result.capture.usage,
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入 WebSocket 请求统计失败: {}",
                    err
                ));
            }
            Ok(())
        }
        Err(error) => {
            let latency_ms = started_at.elapsed().as_millis() as u64;
            log_codex_api_failure(
                Some(&addr),
                Some(&parsed),
                Some(error.status),
                error.account_id.as_deref(),
                error.account_email.as_deref(),
                Some(latency_ms),
                error.message.as_str(),
            );
            let _ = downstream.send(Message::Close(None)).await;
            if let Err(err) = record_request_stats(
                error.account_id.as_deref(),
                error.account_email.as_deref(),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                false,
                error.error_category.as_deref(),
                latency_ms,
                None,
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入 WebSocket 失败统计失败: {}",
                    err
                ));
            }
            Err(error.message)
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    addr: std::net::SocketAddr,
) -> Result<(), String> {
    let request_read_timeout = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .as_ref()
            .map(collection_timeouts)
            .map(|timeouts| {
                duration_from_millis(
                    timeouts.legacy_request_read_timeout_ms,
                    DEFAULT_REQUEST_READ_TIMEOUT,
                )
            })
            .unwrap_or(DEFAULT_REQUEST_READ_TIMEOUT)
    };
    let raw_request = match read_http_request(&mut stream, request_read_timeout).await {
        Ok(raw_request) => raw_request,
        Err(err) => {
            let message = format!("读取本地 API 请求失败: {}", err);
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                400,
                "Bad Request",
                message.as_str(),
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
    };
    let mut parsed = match parse_http_request(&raw_request) {
        Ok(parsed) => parsed,
        Err(err) => {
            let message = format!("解析本地 API 请求失败: {}", err);
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                400,
                "Bad Request",
                message.as_str(),
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
    };

    if parsed.method.eq_ignore_ascii_case("OPTIONS") {
        stream
            .write_all(&options_response())
            .await
            .map_err(|e| format!("写入 OPTIONS 响应失败: {}", e))?;
        return Ok(());
    }

    if !parsed.method.eq_ignore_ascii_case("GET") && !parsed.method.eq_ignore_ascii_case("POST") {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            405,
            "Method Not Allowed",
            "Only GET and POST are allowed",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    parsed.target = normalize_proxy_target(&parsed.target)?;
    if !is_supported_proxy_target(&parsed.target) {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            "Not Found",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(api_key) = extract_local_api_key(&parsed.headers) else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            401,
            "Unauthorized",
            "缺少 Authorization Bearer 或 X-API-Key",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    let state = {
        let runtime = gateway_runtime().lock().await;
        build_request_state_snapshot(&runtime)
    };
    let Some(collection) = state.collection else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            503,
            "Service Unavailable",
            "本地接入集合尚未创建",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    if !collection.enabled || !state.running {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            503,
            "Service Unavailable",
            "本地接入服务未启用",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(resolved_api_key) = resolve_collection_api_key(&collection, &api_key) else {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            401,
            "Unauthorized",
            "本地访问秘钥无效",
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    };
    touch_local_access_api_key(&resolved_api_key.id).await;

    if is_websocket_upgrade_request(&parsed) {
        if !is_backend_codex_responses_websocket_request(&parsed.target)
            && !is_responses_request(&parsed.target)
        {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                Some(&parsed),
                404,
                "Not Found",
                "WebSocket 仅支持 /backend-api/codex/responses",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
        return handle_websocket_connection(stream, addr, parsed, collection, resolved_api_key)
            .await;
    }

    if is_local_models_request(&parsed.target) {
        if collection.account_ids.is_empty() {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                Some(&parsed),
                503,
                "Service Unavailable",
                "本地接入集合暂无账号",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }

        let model_ids = visible_codex_model_ids_for_api_key(&collection, &resolved_api_key, None);
        let response_body = if codex_protocol::is_codex_client_models_request(&parsed.target) {
            build_codex_client_models_response(&model_ids)
        } else {
            build_local_models_response(&model_ids)
        };
        let response = json_response(200, "OK", &response_body);
        stream
            .write_all(&response)
            .await
            .map_err(|e| format!("写入模型响应失败: {}", e))?;
        return Ok(());
    }

    let started_at = Instant::now();
    if collection.image_generation_mode == CodexLocalAccessImageGenerationMode::Disabled
        && (is_images_generations_request(&parsed.target)
            || is_images_edits_request(&parsed.target))
    {
        let request_kind = request_kind_from_target(&parsed.target);
        let model_id = stats_model_id_for_request_kind(&parsed.body, request_kind);
        let message = "API 服务已禁用 image_generation，图片生成和图片编辑接口不可用。";
        let latency_ms = started_at.elapsed().as_millis() as u64;
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            message,
            None,
            None,
            Some(latency_ms),
        )
        .await?;
        if let Err(err) = record_request_stats(
            None,
            None,
            Some(resolved_api_key.id.as_str()),
            Some(resolved_api_key.label.as_str()),
            Some(model_id.as_str()),
            request_kind,
            false,
            Some("image_generation_disabled"),
            latency_ms,
            None,
        )
        .await
        {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 写入禁用图片请求统计失败: {}",
                err
            ));
        }
        return Ok(());
    }
    let health_snapshot = {
        let runtime = gateway_runtime().lock().await;
        runtime.account_health.clone()
    };
    if let Err(err) = rewrite_request_model_for_access_policy(
        &mut parsed,
        &collection,
        &resolved_api_key,
        Some(&health_snapshot),
    ) {
        let latency_ms = started_at.elapsed().as_millis() as u64;
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&parsed),
            404,
            "Not Found",
            err.as_str(),
            None,
            None,
            Some(latency_ms),
        )
        .await?;
        if let Err(stats_err) = record_request_stats(
            None,
            None,
            Some(resolved_api_key.id.as_str()),
            Some(resolved_api_key.label.as_str()),
            extract_request_model_id(&parsed.body).as_deref(),
            request_kind_from_target(&parsed.target),
            false,
            Some("model_not_available"),
            latency_ms,
            None,
        )
        .await
        {
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] 写入模型规则拦截统计失败: {}",
                stats_err
            ));
        }
        return Ok(());
    }
    let (mut prepared_request, response_adapter) = match prepare_gateway_request(parsed) {
        Ok(prepared) => prepared,
        Err(err) => {
            write_json_error_response(
                &mut stream,
                Some(&addr),
                None,
                400,
                "Bad Request",
                err.as_str(),
                None,
                None,
                Some(started_at.elapsed().as_millis() as u64),
            )
            .await?;
            return Ok(());
        }
    };
    if let Err(err) = align_codex_prompt_cache(&mut prepared_request, &resolved_api_key) {
        write_json_error_response(
            &mut stream,
            Some(&addr),
            Some(&prepared_request),
            400,
            "Bad Request",
            err.as_str(),
            None,
            None,
            Some(started_at.elapsed().as_millis() as u64),
        )
        .await?;
        return Ok(());
    }
    apply_codex_official_headers(&mut prepared_request);
    let stats_context =
        build_request_stats_context(&prepared_request, &response_adapter, &resolved_api_key);
    legacy_debug_log(
        collection.debug_logs,
        format!(
            "request_started addr={} method={} target={} request_kind={} model={} api_key_id={} api_key_label={}",
            addr,
            prepared_request.method,
            prepared_request.target,
            request_kind_log_label(stats_context.request_kind),
            stats_context.model_id,
            stats_context.api_key_id,
            stats_context.api_key_label
        ),
    );

    match proxy_request_with_account_pool(
        &prepared_request,
        &collection,
        stats_context.request_kind,
    )
    .await
    {
        Ok(success) => {
            let ProxyDispatchSuccess {
                upstream,
                account_id,
                account_email,
            } = success;
            let timeouts = collection_timeouts(&collection);
            let response_capture = match write_gateway_response(
                &mut stream,
                upstream,
                response_adapter,
                collection.debug_logs,
                &prepared_request,
                started_at,
                &timeouts,
            )
            .await
            {
                Ok(response_capture) => response_capture,
                Err(err) => {
                    if !is_client_disconnect_error_message(&err) {
                        let latency_ms = started_at.elapsed().as_millis() as u64;
                        let error_category = legacy_stream_error_category(&err);
                        let status = if error_category == "upstream_stream_timeout" {
                            StatusCode::GATEWAY_TIMEOUT.as_u16()
                        } else {
                            StatusCode::BAD_GATEWAY.as_u16()
                        };
                        log_codex_api_failure(
                            Some(&addr),
                            Some(&prepared_request),
                            Some(status),
                            Some(account_id.as_str()),
                            Some(account_email.as_str()),
                            Some(latency_ms),
                            err.as_str(),
                        );
                        if let Err(stats_err) = record_request_stats(
                            Some(account_id.as_str()),
                            Some(account_email.as_str()),
                            Some(stats_context.api_key_id.as_str()),
                            Some(stats_context.api_key_label.as_str()),
                            Some(stats_context.model_id.as_str()),
                            stats_context.request_kind,
                            false,
                            Some(error_category),
                            latency_ms,
                            None,
                        )
                        .await
                        {
                            logger::log_codex_api_warn(&format!(
                                "[CodexLocalAccess] 写入流式失败统计失败: {}",
                                stats_err
                            ));
                        }
                    }
                    return Err(err);
                }
            };
            if let Some(response_id) = response_capture.response_id.as_deref() {
                bind_response_affinity(response_id, &account_id).await;
            }
            if collection.session_affinity {
                let session_key = build_request_routing_hint(&prepared_request)
                    .session_affinity_key
                    .map(|key| session_affinity_binding_key(&key));
                if let Some(session_key) = session_key.as_deref() {
                    bind_response_affinity(session_key, &account_id).await;
                }
            }
            let latency_ms = started_at.elapsed().as_millis() as u64;
            if let Err(err) = record_request_stats(
                Some(account_id.as_str()),
                Some(account_email.as_str()),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                true,
                None,
                latency_ms,
                response_capture.usage,
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入请求统计失败: {}",
                    err
                ));
            }
            legacy_debug_log(
                collection.debug_logs,
                format!(
                    "request_completed addr={} method={} target={} status=200 account_id={} account_email={} latency_ms={}",
                    addr,
                    prepared_request.method,
                    prepared_request.target,
                    account_id,
                    account_email,
                    latency_ms
                ),
            );
            Ok(())
        }
        Err(error) => {
            let ProxyDispatchError {
                status,
                message,
                account_id,
                account_email,
                error_category,
            } = error;
            let latency_ms = started_at.elapsed().as_millis() as u64;
            log_codex_api_failure(
                Some(&addr),
                Some(&prepared_request),
                Some(status),
                account_id.as_deref(),
                account_email.as_deref(),
                Some(latency_ms),
                message.as_str(),
            );
            let status_text = match status {
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                405 => "Method Not Allowed",
                429 => "Too Many Requests",
                502 => "Bad Gateway",
                422 => "Unprocessable Entity",
                _ => "Service Unavailable",
            };
            let proxy_diagnostics = (status == StatusCode::BAD_GATEWAY.as_u16()).then(|| {
                current_upstream_proxy_diagnostics(collection.upstream_proxy_url.as_deref())
            });
            let response = json_response(
                status,
                status_text,
                &gateway_error_body(status, &message, proxy_diagnostics.as_ref()),
            );
            let write_result = stream
                .write_all(&response)
                .await
                .map_err(|e| format!("写入错误响应失败: {}", e));
            if let Err(err) = record_request_stats(
                account_id.as_deref(),
                account_email.as_deref(),
                Some(stats_context.api_key_id.as_str()),
                Some(stats_context.api_key_label.as_str()),
                Some(stats_context.model_id.as_str()),
                stats_context.request_kind,
                false,
                error_category.as_deref(),
                latency_ms,
                None,
            )
            .await
            {
                logger::log_codex_api_warn(&format!(
                    "[CodexLocalAccess] 写入失败统计失败: {}",
                    err
                ));
            }
            write_result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        account_upstream_base_url, align_codex_prompt_cache, apply_codex_official_headers,
        apply_routing_strategy, bridge_websocket_streams, build_account_scoped_upstream_body,
        build_chat_completion_payload, build_chat_completion_stream_body,
        build_codex_client_models_response, build_images_api_payload, build_local_models_response,
        build_ordered_account_ids, build_request_routing_hint, build_upstream_websocket_url,
        calculate_usage_cost_usd, canonical_model_for_client_model,
        classify_upstream_error_category, collect_local_access_profile_takeover_dirs_from_store,
        compare_routing_candidates, extract_usage_capture, inspect_local_access_profile_config,
        is_codex_local_access_auth_text, is_image_generation_capability_error,
        is_local_access_eligible_account, is_responses_completion_event,
        is_stream_incomplete_error_message, is_upstream_response_failed_error_message,
        legacy_stream_error_category, model_pricing, normalize_custom_routing_rules,
        normalized_sidecar_error_category, parse_codex_retry_after,
        parse_responses_payload_from_upstream, parse_websocket_upstream_error,
        prepare_gateway_request, profile_base_url_matches, recover_invalid_stats_file,
        remove_codex_local_access_config, resolve_plan_rank, resolve_supported_model_alias,
        resolve_upstream_target, should_retry_single_account_upstream_status,
        should_treat_response_as_stream, should_try_next_account, validate_client_model_visible,
        visible_codex_model_ids_for_api_key, websocket_accept_value,
        websocket_connect_error_from_http_response, CodexLocalAccessCollection,
        CodexLocalAccessGatewayMode, CodexLocalAccessScope, GatewayResponseAdapter, ParsedRequest,
        ResolvedLocalApiKey, ResponseUsageCollector, RoutingCandidate, SidecarUsageDetails,
        SidecarUsageEvent, UsageCapture, CODEX_AUTO_REVIEW_MODEL_ID, DEFAULT_MAX_RETRY_INTERVAL_MS,
        DEFAULT_SESSION_AFFINITY_TTL_MS,
    };
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexAppSpeed, CodexTokens};
    use crate::models::codex_local_access::{
        CodexLocalAccessCustomRoutingRule, CodexLocalAccessImageGenerationMode,
        CodexLocalAccessRequestKind, CodexLocalAccessRoutingStrategy, CodexLocalAccessStats,
        CodexLocalAccessTimeouts,
    };
    use crate::models::{
        DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile, InstanceStore,
    };
    use futures_util::{SinkExt, StreamExt};
    use reqwest::StatusCode;
    use serde_json::{json, Value};
    use std::{collections::HashMap, fs, path::PathBuf};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::time::Duration;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    fn test_local_access_collection(account_ids: Vec<String>) -> CodexLocalAccessCollection {
        CodexLocalAccessCollection {
            enabled: true,
            port: 14998,
            api_key: "local-api-key".to_string(),
            api_keys: Vec::new(),
            access_scope: CodexLocalAccessScope::Localhost,
            image_generation_mode: CodexLocalAccessImageGenerationMode::default(),
            gateway_mode: CodexLocalAccessGatewayMode::default(),
            upstream_proxy_url: None,
            routing_strategy: CodexLocalAccessRoutingStrategy::default(),
            custom_routing_rules: Vec::new(),
            model_aliases: Vec::new(),
            model_pricings: Vec::new(),
            excluded_models: Vec::new(),
            session_affinity: false,
            session_affinity_ttl_ms: DEFAULT_SESSION_AFFINITY_TTL_MS,
            max_retry_credentials: 0,
            max_retry_interval_ms: DEFAULT_MAX_RETRY_INTERVAL_MS,
            timeouts: CodexLocalAccessTimeouts::default(),
            active_timeout_preset_id: "long_wait".to_string(),
            timeout_presets: Vec::new(),
            disable_cooling: false,
            restrict_free_accounts: true,
            debug_logs: true,
            bound_oauth_account_id: None,
            account_ids,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        for _ in 0..10 {
            let dir = std::env::temp_dir().join(format!(
                "{}-{}-{}",
                prefix,
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            if fs::create_dir(&dir).is_ok() {
                return dir;
            }
        }
        panic!("create temp dir failed");
    }

    fn test_account_with_plan(plan_type: &str) -> CodexAccount {
        let mut account = CodexAccount::new(
            format!("acc-{}", plan_type),
            format!("{}@example.com", plan_type),
            CodexTokens {
                id_token: String::new(),
                access_token: "access-token".to_string(),
                refresh_token: None,
            },
        );
        account.plan_type = Some(plan_type.to_string());
        account
    }

    fn test_instance(
        id: &str,
        profile_dir: &str,
        bind_account_id: Option<&str>,
    ) -> InstanceProfile {
        InstanceProfile {
            id: id.to_string(),
            name: id.to_string(),
            user_data_dir: profile_dir.to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: bind_account_id.map(str::to_string),
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
        }
    }

    fn has_image_generation_tool(body: &Value) -> bool {
        body.get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type").and_then(Value::as_str) == Some("image_generation")
                })
            })
            .unwrap_or(false)
    }

    async fn accept_raw_upstream_websocket(listener: TcpListener) -> TcpStream {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request);
        let sec_key = request_text
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("sec-websocket-key")
                        .then(|| value.trim().to_string())
                })
            })
            .expect("client handshake should include sec-websocket-key");
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            websocket_accept_value(&sec_key)
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket
    }

    async fn read_raw_client_websocket_frame(socket: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut header = [0u8; 2];
        socket.read_exact(&mut header).await.unwrap();
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let len = match header[1] & 0x7f {
            n @ 0..=125 => n as usize,
            126 => {
                let mut ext = [0u8; 2];
                socket.read_exact(&mut ext).await.unwrap();
                u16::from_be_bytes(ext) as usize
            }
            127 => {
                let mut ext = [0u8; 8];
                socket.read_exact(&mut ext).await.unwrap();
                u64::from_be_bytes(ext) as usize
            }
            _ => unreachable!(),
        };
        let mut mask = [0u8; 4];
        if masked {
            socket.read_exact(&mut mask).await.unwrap();
        }
        let mut payload = vec![0u8; len];
        if len > 0 {
            socket.read_exact(&mut payload).await.unwrap();
        }
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        (opcode, payload)
    }

    #[tokio::test]
    async fn bridge_flushes_upstream_pong_when_downstream_is_silent() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let (pong_tx, pong_rx) = oneshot::channel();
        let upstream_server = tokio::spawn(async move {
            let (socket, _) = upstream_listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();

            let first = tokio::time::timeout(Duration::from_secs(1), ws.next())
                .await
                .expect("mock upstream should receive the initial payload")
                .expect("mock upstream stream should stay open")
                .expect("mock upstream should read a valid message");
            assert!(matches!(first, Message::Text(_)));

            ws.send(Message::Ping(b"probe".to_vec().into()))
                .await
                .unwrap();
            let pong_result = tokio::time::timeout(Duration::from_millis(250), async {
                loop {
                    let message = ws
                        .next()
                        .await
                        .expect("mock upstream stream should stay open")
                        .expect("mock upstream should read a valid message");
                    if let Message::Pong(payload) = message {
                        return payload;
                    }
                }
            })
            .await;
            let _ = pong_tx.send(pong_result);

            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                while let Some(message) = ws.next().await {
                    if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                        break;
                    }
                }
            })
            .await;
        });

        let upstream_socket = TcpStream::connect(upstream_addr).await.unwrap();
        let upstream_request = format!("ws://{upstream_addr}/responses")
            .into_client_request()
            .unwrap();
        let (downstream_listener, downstream_accept) = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let accept = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(socket).await.unwrap()
            });
            (addr, accept)
        };
        let (client_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{downstream_listener}/responses"))
                .await
                .unwrap();
        let (mut downstream_write, downstream_read) = client_ws.split();
        drop(downstream_read);
        let downstream = downstream_accept.await.unwrap();
        let (upstream, _) = tokio_tungstenite::client_async_tls_with_config(
            upstream_request,
            upstream_socket,
            None,
            None,
        )
        .await
        .unwrap();

        let bridge_task = tokio::spawn(bridge_websocket_streams(
            downstream,
            upstream,
            br#"{"type":"response.create","payload":{}}"#.to_vec(),
            CodexLocalAccessTimeouts::default(),
        ));
        let pong_result = pong_rx.await.unwrap();

        let _ = downstream_write.send(Message::Close(None)).await;
        tokio::time::timeout(Duration::from_secs(1), bridge_task)
            .await
            .expect("bridge should exit after downstream close")
            .expect("bridge task should not panic")
            .expect("bridge cleanup should succeed");
        upstream_server.await.unwrap();

        let payload = pong_result.expect("bridge should flush Pong back to the mock upstream");
        assert_eq!(payload.as_ref(), b"probe");
    }

    #[tokio::test]
    async fn bridge_sends_heartbeat_when_both_peers_are_quiet() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_socket = TcpStream::connect(upstream_addr).await.unwrap();
        let upstream_request = format!("ws://{upstream_addr}/responses")
            .into_client_request()
            .unwrap();
        let (mut raw_upstream, upstream_result) = tokio::join!(
            accept_raw_upstream_websocket(upstream_listener),
            tokio_tungstenite::client_async_tls_with_config(
                upstream_request,
                upstream_socket,
                None,
                None,
            ),
        );
        let (upstream, _) = upstream_result.unwrap();

        let downstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let downstream_addr = downstream_listener.local_addr().unwrap();
        let downstream_accept = tokio::spawn(async move {
            let (socket, _) = downstream_listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(socket).await.unwrap()
        });
        let (client_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{downstream_addr}/responses"))
                .await
                .unwrap();
        let (mut downstream_write, downstream_read) = client_ws.split();
        drop(downstream_read);
        let downstream = downstream_accept.await.unwrap();

        let bridge_task = tokio::spawn(bridge_websocket_streams(
            downstream,
            upstream,
            br#"{"type":"response.create","payload":{}}"#.to_vec(),
            CodexLocalAccessTimeouts::default(),
        ));
        let (first_opcode, first_payload) =
            read_raw_client_websocket_frame(&mut raw_upstream).await;
        assert_eq!(first_opcode, 0x1);
        assert_eq!(first_payload, br#"{"type":"response.create","payload":{}}"#);
        tokio::task::yield_now().await;
        assert!(
            !bridge_task.is_finished(),
            "bridge exited before the quiet heartbeat window"
        );
        let (heartbeat_opcode, heartbeat_payload) = tokio::time::timeout(
            Duration::from_secs(1),
            read_raw_client_websocket_frame(&mut raw_upstream),
        )
        .await
        .expect("bridge should send heartbeat Ping while both peers are quiet");
        assert_eq!(heartbeat_opcode, 0x9);
        assert!(heartbeat_payload.is_empty());

        let _ = downstream_write.send(Message::Close(None)).await;
        tokio::time::timeout(Duration::from_secs(1), bridge_task)
            .await
            .expect("bridge should exit after downstream close")
            .expect("bridge task should not panic")
            .expect("bridge cleanup should succeed");
    }

    #[test]
    fn takeover_dirs_skip_default_profile_when_default_not_bound_to_api_service() {
        let mut store = InstanceStore::new();
        store.default_settings = DefaultInstanceSettings {
            bind_account_id: Some("regular-oauth".to_string()),
            ..DefaultInstanceSettings::default()
        };
        store.instances = vec![
            test_instance("regular", "/tmp/codex-regular", Some("regular-oauth")),
            test_instance(
                "api-service",
                "/tmp/codex-api-service",
                Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
            ),
        ];

        let dirs = collect_local_access_profile_takeover_dirs_from_store(
            store,
            PathBuf::from("/tmp/default-codex"),
        );

        assert_eq!(dirs, vec![PathBuf::from("/tmp/codex-api-service")]);
    }

    #[test]
    fn takeover_dirs_include_default_profile_only_when_bound_to_api_service() {
        let mut store = InstanceStore::new();
        store.default_settings = DefaultInstanceSettings {
            bind_account_id: Some(
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
            ),
            ..DefaultInstanceSettings::default()
        };
        store.instances = vec![test_instance(
            "api-service",
            "/tmp/default-codex",
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
        )];

        let dirs = collect_local_access_profile_takeover_dirs_from_store(
            store,
            PathBuf::from("/tmp/default-codex"),
        );

        assert_eq!(dirs, vec![PathBuf::from("/tmp/default-codex")]);
    }

    #[test]
    fn calculates_usage_cost_with_cached_input_price() {
        let usage = UsageCapture {
            input_tokens: 1_000,
            output_tokens: 2_000,
            total_tokens: 3_000,
            cached_tokens: 400,
            reasoning_tokens: 0,
        };
        let pricing = model_pricing("gpt-5.4", 1.25, Some(0.125), 10.0);
        let cost = calculate_usage_cost_usd(Some(&usage), Some(&pricing));
        let expected = ((600.0 * 1.25) + (400.0 * 0.125) + (2_000.0 * 10.0)) / 1_000_000.0;
        assert!((cost - expected).abs() < 0.000000001);
    }

    #[test]
    fn removes_only_codex_local_access_provider_config() {
        let input = r#"model_provider = "codex_local_access"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://127.0.0.1:57391/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.manual]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
"#;

        let output = remove_codex_local_access_config(input).expect("cleanup config");
        let parsed = output
            .parse::<toml_edit::Document>()
            .expect("parse cleaned toml");

        assert!(parsed.get("model_provider").is_none());
        assert_eq!(
            parsed
                .get("model_context_window")
                .and_then(|item| item.as_integer()),
            Some(1_000_000)
        );
        let providers = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .expect("model_providers should remain");
        assert!(providers.get("codex_local_access").is_none());
        assert!(providers.get("manual").is_some());
    }

    #[test]
    fn detects_only_matching_local_access_auth_key() {
        assert!(is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-key"}"#,
            "local-key"
        ));
        assert!(is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"agt_codex_generated"}"#,
            "local-key"
        ));
        assert!(!is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"other-key"}"#,
            "local-key"
        ));
        assert!(!is_codex_local_access_auth_text(
            r#"{"tokens":{"access_token":"official"}}"#,
            "local-key"
        ));
    }

    #[test]
    fn local_access_profile_config_requires_selected_matching_provider() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_test"
supports_websockets = false
"#;

        let inspection = inspect_local_access_profile_config(
            config,
            "http://localhost:14998/v1",
            "agt_codex_test",
        )
        .expect("inspect config");

        assert!(inspection.config_attached);
        assert_eq!(
            inspection.model_provider.as_deref(),
            Some("codex_local_access")
        );
        assert_eq!(
            inspection.base_url.as_deref(),
            Some("http://localhost:14998/v1")
        );
    }

    #[test]
    fn local_access_profile_config_rejects_stale_port_or_key() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://127.0.0.1:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_old"
supports_websockets = false
"#;

        let stale_port = inspect_local_access_profile_config(
            config,
            "http://127.0.0.1:14999/v1",
            "agt_codex_old",
        )
        .expect("inspect stale port");
        let stale_key = inspect_local_access_profile_config(
            config,
            "http://127.0.0.1:14998/v1",
            "agt_codex_new",
        )
        .expect("inspect stale key");

        assert!(!stale_port.config_attached);
        assert!(!stale_key.config_attached);
    }

    #[test]
    fn profile_base_url_matching_ignores_trailing_slash_and_host_case() {
        assert!(profile_base_url_matches(
            Some("HTTP://LOCALHOST:14998/v1/"),
            "http://localhost:14998/v1"
        ));
        assert!(!profile_base_url_matches(
            Some("http://127.0.0.1:14998/v1"),
            "http://localhost:14998/v1"
        ));
    }

    #[test]
    fn invalid_stats_file_is_quarantined_and_replaced_by_empty_stats() {
        let dir = make_temp_dir("codex-local-access-invalid-stats");
        let path = dir.join("codex_local_access_stats.json");
        fs::write(
            &path,
            b"{\"since\":1,\"accounts\":[{\"email\":\"bad\0value\"}]}",
        )
        .expect("write invalid stats");
        let content = fs::read_to_string(&path).expect("read invalid stats");
        let parse_error =
            serde_json::from_str::<CodexLocalAccessStats>(&content).expect_err("invalid json");

        let recovered = recover_invalid_stats_file(&path, &parse_error);

        assert_eq!(recovered.totals.request_count, 0);
        assert!(!path.exists());
        let backups = fs::read_dir(&dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("codex_local_access_stats.json.invalid-")
            })
            .count();
        assert_eq!(backups, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn extracts_usage_from_codex_response_completed_payload() {
        let payload = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 16,
                    "input_tokens_details": {
                        "cached_tokens": 3
                    },
                    "output_tokens": 5,
                    "output_tokens_details": {
                        "reasoning_tokens": 2
                    },
                    "total_tokens": 21
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 16);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.cached_tokens, 3);
        assert_eq!(usage.reasoning_tokens, 2);
        assert_eq!(usage.total_tokens, 21);
    }

    #[test]
    fn extracts_usage_from_codex_response_done_payload() {
        assert!(is_responses_completion_event("response.done"));

        let payload = json!({
            "type": "response.done",
            "response": {
                "id": "resp_123",
                "usage": {
                    "input_tokens": 32,
                    "input_tokens_details": {
                        "cached_tokens": 9
                    },
                    "output_tokens": 6,
                    "output_tokens_details": {
                        "reasoning_tokens": 3
                    },
                    "total_tokens": 41
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 32);
        assert_eq!(usage.output_tokens, 6);
        assert_eq!(usage.cached_tokens, 9);
        assert_eq!(usage.reasoning_tokens, 3);
        assert_eq!(usage.total_tokens, 41);
    }

    #[test]
    fn extracts_usage_from_openai_prompt_and_completion_details() {
        let payload = json!({
            "usage": {
                "prompt_tokens": 8,
                "prompt_tokens_details": {
                    "cached_tokens": 1
                },
                "completion_tokens": 4,
                "completion_tokens_details": {
                    "reasoning_tokens": 2
                }
            }
        });

        let usage = extract_usage_capture(&payload).expect("usage should be parsed");
        assert_eq!(usage.input_tokens, 8);
        assert_eq!(usage.output_tokens, 4);
        assert_eq!(usage.cached_tokens, 1);
        assert_eq!(usage.reasoning_tokens, 2);
        assert_eq!(usage.total_tokens, 14);
    }

    #[test]
    fn parses_sse_usage_when_request_is_stream_even_if_content_type_is_json() {
        assert!(should_treat_response_as_stream(
            "application/json; charset=utf-8",
            true
        ));

        let mut collector = ResponseUsageCollector::new(true);
        collector.feed(
            br#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_123","usage":{"input_tokens":16,"input_tokens_details":{"cached_tokens":0},"output_tokens":5,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":21}}}

"#,
        );

        let capture = collector.finish();
        let usage = capture.usage.expect("stream usage should be parsed");
        assert_eq!(usage.input_tokens, 16);
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(usage.total_tokens, 21);
        assert_eq!(capture.response_id.as_deref(), Some("resp_123"));
    }

    #[test]
    fn parses_codex_retry_after_from_usage_limit_payload() {
        let wait = parse_codex_retry_after(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":{"type":"usage_limit_reached","resets_in_seconds":12}}"#,
        )
        .expect("retry after should be parsed");

        assert_eq!(wait, Duration::from_secs(12));
    }

    #[test]
    fn retries_next_account_for_transient_upstream_status() {
        assert!(should_try_next_account(
            StatusCode::SERVICE_UNAVAILABLE,
            "upstream temporarily unavailable"
        ));
        assert!(should_try_next_account(
            StatusCode::BAD_GATEWAY,
            "gateway error"
        ));
    }

    #[test]
    fn retries_single_account_for_transient_upstream_status() {
        assert!(should_retry_single_account_upstream_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(should_retry_single_account_upstream_status(
            StatusCode::GATEWAY_TIMEOUT
        ));
        assert!(!should_retry_single_account_upstream_status(
            StatusCode::TOO_MANY_REQUESTS
        ));
        assert!(!should_retry_single_account_upstream_status(
            StatusCode::FORBIDDEN
        ));
    }

    #[test]
    fn does_not_retry_forbidden_without_quota_or_capacity_markers() {
        assert!(!should_try_next_account(
            StatusCode::FORBIDDEN,
            r#"{"error":"forbidden"}"#,
        ));
    }

    #[test]
    fn retries_next_account_for_image_generation_capability_error() {
        let body = r#"{"error":{"message":"Image generation is not enabled for this group"}}"#;
        assert!(is_image_generation_capability_error(
            StatusCode::FORBIDDEN,
            body,
        ));
        assert!(should_try_next_account(StatusCode::FORBIDDEN, body));
        assert_eq!(
            classify_upstream_error_category(StatusCode::FORBIDDEN, body),
            Some("image_generation_not_enabled")
        );
    }

    #[test]
    fn classifies_stream_incomplete_errors_separately() {
        let decoding_error = "读取上游响应失败: error decoding response body";
        let disconnected_error = "stream error: stream disconnected before completion: stream closed before response.completed/response.done";
        let response_failed_error = "stream error: stream disconnected before completion: stream closed before response.completed/response.done, last_event=response.failed";

        assert!(is_stream_incomplete_error_message(decoding_error));
        assert!(is_stream_incomplete_error_message(disconnected_error));
        assert!(is_upstream_response_failed_error_message(
            response_failed_error
        ));
        assert_eq!(
            legacy_stream_error_category(decoding_error),
            "stream_incomplete"
        );
        assert_eq!(
            legacy_stream_error_category(disconnected_error),
            "stream_incomplete"
        );
        assert_eq!(
            legacy_stream_error_category(response_failed_error),
            "upstream_response_failed"
        );
    }

    #[test]
    fn sidecar_response_failed_overrides_generic_request_failed() {
        let event = SidecarUsageEvent {
            request_id: "req-1".to_string(),
            model: "gpt-5.4".to_string(),
            account_id: "account-1".to_string(),
            account_email: "user@example.com".to_string(),
            api_key_id: "key-1".to_string(),
            api_key_label: "Default".to_string(),
            request_kind: "text".to_string(),
            success: false,
            status: Some(200),
            error_category: Some("request_failed".to_string()),
            error_message: Some("stream error: stream disconnected before completion: stream closed before response.completed/response.done, last_event=response.failed".to_string()),
            latency_ms: 1754,
            usage: SidecarUsageDetails::default(),
        };

        assert_eq!(
            normalized_sidecar_error_category(&event).as_deref(),
            Some("upstream_response_failed")
        );
    }

    #[test]
    fn prefers_affinity_account_before_round_robin_order() {
        let ordered = build_ordered_account_ids(
            &[
                "acc-a".to_string(),
                "acc-b".to_string(),
                "acc-c".to_string(),
            ],
            1,
            Some("acc-c"),
        );

        assert_eq!(ordered, vec!["acc-c", "acc-b", "acc-a"]);
    }

    #[test]
    fn codex_plan_rank_matches_current_rate_card() {
        let mut promax = test_account_with_plan("pro");
        promax.auth_file_plan_type = Some("promax".to_string());
        let mut prolite = test_account_with_plan("pro");
        prolite.auth_file_plan_type = Some("prolite".to_string());

        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("free")),
            Some(100)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("go")), Some(200));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("plus")),
            Some(300)
        );
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("team")),
            Some(300)
        );
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("business")),
            Some(300)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("pro")), Some(500));
        assert_eq!(resolve_plan_rank(&prolite), Some(500));
        assert_eq!(resolve_plan_rank(&promax), Some(600));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("enterprise")),
            Some(700)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("edu")), Some(700));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("health")),
            Some(700)
        );
        assert_eq!(resolve_plan_rank(&test_account_with_plan("gov")), Some(700));
        assert_eq!(
            resolve_plan_rank(&test_account_with_plan("teachers")),
            Some(700)
        );
    }

    #[test]
    fn plan_low_first_places_business_and_team_before_pro() {
        let mut candidates = vec![
            RoutingCandidate {
                account_id: "acc-pro".to_string(),
                plan_rank: Some(500),
                remaining_quota: Some(80),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-plus".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(40),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-team".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(70),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-business".to_string(),
                plan_rank: Some(300),
                remaining_quota: Some(60),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-promax".to_string(),
                plan_rank: Some(600),
                remaining_quota: Some(90),
                subscription_expiry_ms: None,
            },
            RoutingCandidate {
                account_id: "acc-edu".to_string(),
                plan_rank: Some(700),
                remaining_quota: Some(100),
                subscription_expiry_ms: None,
            },
        ];
        let original_index = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| (candidate.account_id.clone(), index))
            .collect::<HashMap<_, _>>();

        candidates.sort_by(|left, right| {
            compare_routing_candidates(
                left,
                right,
                CodexLocalAccessRoutingStrategy::PlanLowFirst,
                &original_index,
            )
        });

        let ordered = candidates
            .into_iter()
            .map(|candidate| candidate.account_id)
            .collect::<Vec<_>>();

        assert_eq!(
            ordered,
            vec![
                "acc-team",
                "acc-business",
                "acc-plus",
                "acc-pro",
                "acc-promax",
                "acc-edu",
            ]
        );
    }

    #[test]
    fn custom_routing_prefers_higher_priority_accounts() {
        let account_ids = vec![
            "acc-low".to_string(),
            "acc-high-a".to_string(),
            "acc-high-b".to_string(),
        ];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-low".to_string(),
                priority: 10,
                weight: 1,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-high-a".to_string(),
                priority: 40,
                weight: 1,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-high-b".to_string(),
                priority: 40,
                weight: 1,
            },
        ];

        let ordered = apply_routing_strategy(
            &account_ids,
            CodexLocalAccessRoutingStrategy::Custom,
            &rules,
            0,
        );

        assert_eq!(ordered, vec!["acc-high-a", "acc-high-b", "acc-low"]);
    }

    #[test]
    fn custom_routing_uses_weight_for_same_priority_first_pick() {
        let account_ids = vec!["acc-heavy".to_string(), "acc-light".to_string()];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-heavy".to_string(),
                priority: 20,
                weight: 3,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-light".to_string(),
                priority: 20,
                weight: 1,
            },
        ];

        let first_picks = (0..8)
            .map(|start| {
                apply_routing_strategy(
                    &account_ids,
                    CodexLocalAccessRoutingStrategy::Custom,
                    &rules,
                    start,
                )[0]
                .clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            first_picks,
            vec![
                "acc-heavy",
                "acc-heavy",
                "acc-heavy",
                "acc-light",
                "acc-heavy",
                "acc-heavy",
                "acc-heavy",
                "acc-light",
            ]
        );
    }

    #[test]
    fn custom_routing_rules_are_normalized_to_collection_accounts() {
        let account_ids = vec!["acc-a".to_string(), "acc-b".to_string()];
        let rules = vec![
            CodexLocalAccessCustomRoutingRule {
                account_id: " acc-a ".to_string(),
                priority: 120,
                weight: 0,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-a".to_string(),
                priority: 20,
                weight: 10,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-removed".to_string(),
                priority: 30,
                weight: 5,
            },
            CodexLocalAccessCustomRoutingRule {
                account_id: "acc-b".to_string(),
                priority: -5,
                weight: 500,
            },
        ];

        let normalized = normalize_custom_routing_rules(rules, &account_ids);

        assert_eq!(
            normalized,
            vec![
                CodexLocalAccessCustomRoutingRule {
                    account_id: "acc-a".to_string(),
                    priority: 100,
                    weight: 1,
                },
                CodexLocalAccessCustomRoutingRule {
                    account_id: "acc-b".to_string(),
                    priority: 0,
                    weight: 100,
                },
            ]
        );
    }

    #[test]
    fn builds_routing_hint_from_previous_response_id_and_model() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"GPT-5.4-mini","previous_response_id":"resp_prev"}"#.to_vec(),
        };

        let hint = build_request_routing_hint(&request);
        assert_eq!(hint.model_key, "gpt-5.4-mini");
        assert_eq!(hint.previous_response_id.as_deref(), Some("resp_prev"));
    }

    #[test]
    fn maps_snapshot_model_ids_to_supported_aliases() {
        assert_eq!(
            resolve_supported_model_alias("gpt-5.4-2026-03-05"),
            "gpt-5.4"
        );
        assert_eq!(
            resolve_supported_model_alias("GPT-5.4-Mini-2026-03-05"),
            "gpt-5.4-mini"
        );
        assert_eq!(
            resolve_supported_model_alias("custom-model-2026-03-05"),
            "custom-model-2026-03-05"
        );
    }

    #[test]
    fn local_models_include_codex_image_model() {
        let response = build_local_models_response(&[
            "gpt-5.4".to_string(),
            "gpt-image-2".to_string(),
            CODEX_AUTO_REVIEW_MODEL_ID.to_string(),
        ]);
        let has_image_model = response
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .any(|model| model.get("id").and_then(Value::as_str) == Some("gpt-image-2"))
            })
            .unwrap_or(false);
        let has_auto_review_model = response
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models.iter().any(|model| {
                    model.get("id").and_then(Value::as_str) == Some(CODEX_AUTO_REVIEW_MODEL_ID)
                })
            })
            .unwrap_or(false);

        assert!(has_image_model);
        assert!(has_auto_review_model);
    }

    #[test]
    fn codex_client_models_use_models_catalog_shape() {
        let response = build_codex_client_models_response(&[
            "gpt-5.4".to_string(),
            "gpt-image-2".to_string(),
            CODEX_AUTO_REVIEW_MODEL_ID.to_string(),
        ]);
        assert!(response.get("object").is_none());
        assert!(response.get("data").is_none());
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("codex client models should be an array");
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
        assert!(models
            .iter()
            .all(|model| model.get("prefer_websockets").and_then(Value::as_bool) == Some(true)));
        assert!(models.iter().any(|model| {
            model.get("slug").and_then(Value::as_str) == Some(CODEX_AUTO_REVIEW_MODEL_ID)
                && model.get("visibility").and_then(Value::as_str) == Some("hide")
        }));
    }

    #[test]
    fn auto_review_model_bypasses_legacy_gateway_model_filters() {
        let collection = test_local_access_collection(vec!["account-1".to_string()]);
        let api_key = ResolvedLocalApiKey {
            id: "key-1".to_string(),
            label: "Key".to_string(),
            model_prefix: Some("team".to_string()),
            allowed_models: vec!["gpt-*".to_string()],
            excluded_models: vec!["codex-*".to_string()],
        };

        let models = visible_codex_model_ids_for_api_key(&collection, &api_key, None);
        assert!(models
            .iter()
            .any(|model| model == CODEX_AUTO_REVIEW_MODEL_ID));
        assert_eq!(
            canonical_model_for_client_model(CODEX_AUTO_REVIEW_MODEL_ID, &collection, &api_key),
            CODEX_AUTO_REVIEW_MODEL_ID
        );
        assert!(validate_client_model_visible(
            CODEX_AUTO_REVIEW_MODEL_ID,
            CODEX_AUTO_REVIEW_MODEL_ID,
            &collection,
            &api_key,
            None,
        ));
    }

    #[test]
    fn prepares_chat_completions_request_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"GPT-5.4","stream":true,"messages":[{"role":"user","content":"hello"}]}"#
                .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert!(mapped_body.get("input").is_some());
        assert_eq!(mapped_body.get("store"), Some(&Value::Bool(false)));
        assert_eq!(mapped_body.get("stream"), Some(&Value::Bool(true)));
        assert_eq!(
            mapped_body.get("instructions").and_then(Value::as_str),
            Some("")
        );
        assert_eq!(
            mapped_body
                .get("parallel_tool_calls")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            mapped_body
                .get("reasoning")
                .and_then(|reasoning| reasoning.get("effort"))
                .and_then(Value::as_str),
            Some("medium")
        );
        assert!(!has_image_generation_tool(&mapped_body));

        match adapter {
            GatewayResponseAdapter::ChatCompletions {
                stream,
                requested_model,
                original_request_body: _,
            } => {
                assert!(stream);
                assert_eq!(requested_model, "gpt-5.4");
            }
            _ => panic!("expected chat completions adapter"),
        }
    }

    #[test]
    fn prepares_images_generation_request_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/generations".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-image-2","prompt":"draw a clean icon","size":"1024x1024","response_format":"b64_json"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4-mini")
        );
        assert_eq!(
            mapped_body
                .get("tool_choice")
                .and_then(|choice| choice.get("type"))
                .and_then(Value::as_str),
            Some("image_generation")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("model"))
                .and_then(Value::as_str),
            Some("gpt-image-2")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("size"))
                .and_then(Value::as_str),
            Some("1024x1024")
        );

        match adapter {
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix,
            } => {
                assert!(!stream);
                assert_eq!(response_format, "b64_json");
                assert_eq!(stream_prefix, "image_generation");
            }
            _ => panic!("expected images adapter"),
        }
    }

    #[test]
    fn rejects_unsupported_images_model() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/generations".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-image-1.5","prompt":"draw"}"#.to_vec(),
        };

        let err = prepare_gateway_request(request).expect_err("model should be rejected");
        assert!(err.contains("Use gpt-image-2"));
    }

    #[test]
    fn prepares_multipart_images_edit_request_for_responses_proxy() {
        let boundary = "test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        body.extend_from_slice(b"gpt-image-2\r\n");
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"prompt\"\r\n\r\n");
        body.extend_from_slice(b"make it brighter\r\n");
        body.extend_from_slice(b"--test-boundary\r\n");
        body.extend_from_slice(
            b"Content-Disposition: form-data; name=\"image\"; filename=\"a.png\"\r\n",
        );
        body.extend_from_slice(b"Content-Type: image/png\r\n\r\n");
        body.extend_from_slice(b"\x89PNG\r\n\x1a\nabc\r\n");
        body.extend_from_slice(b"--test-boundary--\r\n");
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            format!("multipart/form-data; boundary={}", boundary),
        );
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/images/edits".to_string(),
            headers,
            body,
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("action"))
                .and_then(Value::as_str),
            Some("edit")
        );
        let has_input_image = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("content"))
            .and_then(Value::as_array)
            .map(|content| {
                content.iter().any(|part| {
                    part.get("type").and_then(Value::as_str) == Some("input_image")
                        && part
                            .get("image_url")
                            .and_then(Value::as_str)
                            .map(|url| url.starts_with("data:image/png;base64,"))
                            .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        assert!(has_input_image);

        match adapter {
            GatewayResponseAdapter::Images { stream_prefix, .. } => {
                assert_eq!(stream_prefix, "image_edit");
            }
            _ => panic!("expected images adapter"),
        }
    }

    #[test]
    fn builds_images_api_payload_from_responses_output() {
        let response = json!({
            "response": {
                "created_at": 123,
                "output": [{
                    "type": "image_generation_call",
                    "result": "aGVsbG8=",
                    "output_format": "png",
                    "revised_prompt": "draw a clean icon"
                }],
                "tool_usage": {
                    "image_gen": {
                        "input_images": 0,
                        "output_images": 1
                    }
                }
            }
        });

        let payload =
            build_images_api_payload(&response, "b64_json").expect("payload should build");
        assert_eq!(payload.get("created").and_then(Value::as_i64), Some(123));
        assert_eq!(
            payload
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("b64_json"))
                .and_then(Value::as_str),
            Some("aGVsbG8=")
        );
        assert_eq!(
            payload
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("revised_prompt"))
                .and_then(Value::as_str),
            Some("draw a clean icon")
        );
    }

    #[test]
    fn rewrites_snapshot_model_ids_for_passthrough_requests() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4-2026-03-05","input":"hello"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected passthrough adapter"),
        }
    }

    #[test]
    fn responses_stream_requests_stay_passthrough() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::from([("accept".to_string(), "text/event-stream".to_string())]),
            body: br#"{"model":"gpt-5.4","stream":false,"store":true,"input":"hello","temperature":0.2}"#
                .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            mapped_body.get("store").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            mapped_body.get("instructions").and_then(Value::as_str),
            Some("")
        );
        assert!(mapped_body.get("temperature").is_none());
        assert_eq!(
            mapped_body
                .pointer("/input/0/content/0/text")
                .and_then(Value::as_str),
            Some("hello")
        );

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected responses stream passthrough adapter"),
        }
    }

    #[test]
    fn injects_image_generation_tool_only_for_non_free_responses_accounts() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":"draw an icon"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert!(!has_image_generation_tool(&mapped_body));
        assert_eq!(
            mapped_body.get("stream").and_then(Value::as_bool),
            Some(true)
        );

        let paid_account = test_account_with_plan("plus");
        let paid_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &paid_account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("paid body should build");
        let paid_mapped_body: Value =
            serde_json::from_slice(paid_body.as_ref()).expect("paid body should be json");
        assert!(paid_mapped_body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("image_generation")
                    && tool.get("output_format").and_then(Value::as_str) == Some("png")
            }))
            .unwrap_or(false));

        let free_account = test_account_with_plan("free");
        let free_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &free_account,
            CodexLocalAccessImageGenerationMode::Enabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("free body should build");
        let free_mapped_body: Value =
            serde_json::from_slice(free_body.as_ref()).expect("free body should be json");
        assert!(!has_image_generation_tool(&free_mapped_body));

        let images_only_body = build_account_scoped_upstream_body(
            "/responses",
            &prepared.body,
            &paid_account,
            CodexLocalAccessImageGenerationMode::ImagesOnly,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("images-only body should build");
        let images_only_mapped_body: Value = serde_json::from_slice(images_only_body.as_ref())
            .expect("images-only body should be json");
        assert!(!has_image_generation_tool(&images_only_mapped_body));

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected passthrough adapter"),
        }
    }

    #[test]
    fn disabled_image_generation_mode_removes_declared_tool_and_choice() {
        let account = test_account_with_plan("plus");
        let body = br#"{
            "model":"gpt-5.4",
            "input":"hello",
            "tool_choice":{"type":"image_generation"},
            "tools":[
                {"type":"web_search_preview"},
                {"type":"image_generation","output_format":"png"}
            ]
        }"#;

        let mapped_body = build_account_scoped_upstream_body(
            "/responses",
            body,
            &account,
            CodexLocalAccessImageGenerationMode::Disabled,
            CodexLocalAccessRequestKind::Text,
        )
        .expect("disabled body should build");
        let parsed: Value =
            serde_json::from_slice(mapped_body.as_ref()).expect("body should remain json");

        assert!(!has_image_generation_tool(&parsed));
        assert!(parsed.get("tool_choice").is_none());
        assert!(parsed
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools
                .iter()
                .any(|tool| tool.get("type").and_then(Value::as_str) == Some("web_search_preview")))
            .unwrap_or(false));
    }

    #[test]
    fn normalizes_direct_responses_system_role_for_codex() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":[{"type":"message","role":"system","content":"be concise"},{"type":"message","role":"user","content":[{"type":"text","text":"hello"}]}],"tools":[{"type":"web_search_preview"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.pointer("/input/0/role").and_then(Value::as_str),
            Some("developer")
        );
        assert_eq!(
            mapped_body
                .pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            mapped_body
                .pointer("/input/1/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            mapped_body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("web_search")
        );
    }

    #[test]
    fn rewrites_snapshot_model_ids_for_chat_completions_requests() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body:
                br#"{"model":"gpt-5.4-2026-03-05","messages":[{"role":"user","content":"hello"}]}"#
                    .to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body.get("model").and_then(Value::as_str),
            Some("gpt-5.4")
        );

        match adapter {
            GatewayResponseAdapter::ChatCompletions {
                requested_model, ..
            } => {
                assert_eq!(requested_model, "gpt-5.4");
            }
            _ => panic!("expected chat completions adapter"),
        }
    }

    #[test]
    fn drops_unsupported_sampling_params_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","temperature":0.2,"top_p":0.7,"messages":[{"role":"user","content":"hello"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert!(mapped_body.get("temperature").is_none());
        assert!(mapped_body.get("top_p").is_none());
    }

    #[test]
    fn normalizes_text_content_parts_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let first_type = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .and_then(|messages| messages.first())
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("type"))
            .and_then(Value::as_str);
        assert_eq!(first_type, Some("input_text"));
    }

    #[test]
    fn normalizes_function_tools_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"hello"}],"tools":[{"type":"function","function":{"name":"get_weather","description":"Get weather","parameters":{"type":"object","properties":{"location":{"type":"string"}}},"strict":true}}],"tool_choice":{"type":"function","function":{"name":"get_weather"}}}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            mapped_body
                .get("tool_choice")
                .and_then(|choice| choice.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
        assert_eq!(
            mapped_body
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| tools.first())
                .and_then(|tool| tool.get("strict"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn normalizes_tool_history_messages_for_responses_proxy() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"{\"temperature_c\":18}"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let input = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .expect("input should be array");
        assert_eq!(
            input
                .first()
                .and_then(|item| item.get("role"))
                .and_then(Value::as_str),
            Some("user")
        );
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("name").and_then(Value::as_str) == Some("get_weather")
        }));
        assert!(input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some("call_1")
        }));
    }

    #[test]
    fn skips_spurious_empty_assistant_message_for_tool_calls() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/chat/completions".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","messages":[{"role":"user","content":"weather?"},{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"Paris\"}"}}]},{"role":"tool","tool_call_id":"call_1","content":"{\"temperature_c\":18}"}]}"#
                .to_vec(),
        };

        let (prepared, _) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        let input = mapped_body
            .get("input")
            .and_then(Value::as_array)
            .expect("input should be array");
        assert_eq!(input.len(), 3);
        assert_eq!(
            input
                .first()
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("message")
        );
        assert_eq!(
            input
                .get(1)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("function_call")
        );
        assert_eq!(
            input
                .get(2)
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str),
            Some("function_call_output")
        );
    }

    #[test]
    fn builds_chat_completion_payload_from_responses_output() {
        let responses_payload = json!({
            "id": "resp_123",
            "model": "gpt-5.4",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "hello world"
                }]
            }],
            "usage": {
                "input_tokens": 7,
                "output_tokens": 3,
                "total_tokens": 10
            }
        });

        let chat_payload = build_chat_completion_payload(&responses_payload, "gpt-5.4", br#"{}"#);
        assert_eq!(
            chat_payload.get("object").and_then(Value::as_str),
            Some("chat.completion")
        );
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str),
            Some("hello world")
        );
        assert_eq!(
            chat_payload
                .get("usage")
                .and_then(|usage| usage.get("total_tokens"))
                .and_then(Value::as_u64),
            Some(10)
        );
    }

    #[test]
    fn builds_chat_completion_payload_from_function_call_output() {
        let responses_payload = json!({
            "id": "resp_tool_1",
            "model": "gpt-5.4",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_abc",
                "name": "get_weather",
                "arguments": "{\"location\":\"Paris\"}"
            }]
        });

        let chat_payload = build_chat_completion_payload(&responses_payload, "gpt-5.4", br#"{}"#);
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("finish_reason"))
                .and_then(Value::as_str),
            Some("tool_calls")
        );
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
                .and_then(|tool_calls| tool_calls.first())
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some("get_weather")
        );
    }

    #[test]
    fn restores_shortened_tool_name_in_chat_payload() {
        let original_request = br#"{
            "model":"gpt-5.4",
            "messages":[{"role":"user","content":"run tool"}],
            "tools":[{
                "type":"function",
                "function":{
                    "name":"mcp__very_long_namespace_segment__very_long_server_name__super_long_tool_name_that_needs_shortening",
                    "description":"Long name",
                    "parameters":{"type":"object","properties":{}}
                }
            }]
        }"#;
        let responses_payload = json!({
            "id": "resp_tool_2",
            "model": "gpt-5.4",
            "status": "completed",
            "output": [{
                "type": "function_call",
                "call_id": "call_long",
                "name": "mcp__super_long_tool_name_that_needs_shortening",
                "arguments": "{}"
            }]
        });

        let chat_payload =
            build_chat_completion_payload(&responses_payload, "gpt-5.4", original_request);
        assert_eq!(
            chat_payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|message| message.get("tool_calls"))
                .and_then(Value::as_array)
                .and_then(|tool_calls| tool_calls.first())
                .and_then(|tool_call| tool_call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str),
            Some(
                "mcp__very_long_namespace_segment__very_long_server_name__super_long_tool_name_that_needs_shortening"
            )
        );
    }

    #[test]
    fn builds_chat_completion_stream_body_with_done_marker() {
        let upstream_sse = br#"data: {"type":"response.created","response":{"id":"resp_1","created_at":123,"model":"gpt-5.4"}}

data: {"type":"response.output_text.delta","delta":"stream-body"}

event: response.done
data: {"response":{"id":"resp_1","created_at":123,"model":"gpt-5.4","status":"completed","usage":{"input_tokens":1,"input_tokens_details":{"cached_tokens":1},"output_tokens":1,"total_tokens":2}}}

"#;

        let stream_body = build_chat_completion_stream_body(upstream_sse, br#"{}"#, "gpt-5.4");
        assert!(stream_body.contains("chat.completion.chunk"));
        assert!(stream_body.contains("stream-body"));
        assert!(stream_body.contains("\"cached_tokens\":1"));
        assert!(stream_body.contains("data: [DONE]"));
    }

    #[test]
    fn parses_responses_sse_payload_to_json() {
        let sse = br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hello "}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"world"}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","model":"gpt-5.4","status":"completed","usage":{"input_tokens":2,"output_tokens":2,"total_tokens":4}}}

data: [DONE]

"#;

        let parsed = parse_responses_payload_from_upstream(sse).expect("sse should be parsed");
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("resp_1")
        );
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("output_text"))
                .and_then(Value::as_str),
            Some("hello world")
        );
    }

    #[test]
    fn parses_response_done_sse_payload_to_json() {
        let sse = br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"done body"}

event: response.done
data: {"response":{"id":"resp_done","model":"gpt-5.4","status":"completed","usage":{"input_tokens":3,"input_tokens_details":{"cached_tokens":2},"output_tokens":1,"total_tokens":4}}}

"#;

        let parsed = parse_responses_payload_from_upstream(sse).expect("sse should be parsed");
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("id"))
                .and_then(Value::as_str),
            Some("resp_done")
        );
        assert_eq!(
            parsed
                .get("response")
                .and_then(|value| value.get("usage"))
                .and_then(|value| value.get("input_tokens_details"))
                .and_then(|value| value.get("cached_tokens"))
                .and_then(Value::as_u64),
            Some(2)
        );
    }

    #[test]
    fn parses_responses_sse_response_failed_as_upstream_failure() {
        let sse = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_failed","error":{"code":"model_at_capacity","type":"server_error","message":"model overloaded"}}}

"#;

        let error = parse_responses_payload_from_upstream(sse).expect_err("failed event");
        assert!(error.contains("upstream_response_failed"));
        assert!(error.contains("response.failed"));
        assert!(error.contains("model_at_capacity"));
        assert!(error.contains("model overloaded"));
        assert_eq!(
            legacy_stream_error_category(&error),
            "upstream_response_failed"
        );
    }

    #[test]
    fn response_usage_collector_captures_sse_error_event() {
        let sse = br#"event: error
data: {"error":{"code":"server_error","type":"upstream","message":"stream aborted"}}

"#;

        let mut collector = ResponseUsageCollector::new(true);
        collector.feed(sse);
        let capture = collector.finish();

        let error = capture.terminal_error.expect("terminal error");
        assert!(error.contains("upstream_response_failed"));
        assert!(error.contains("server_error"));
        assert!(error.contains("stream aborted"));
    }

    #[test]
    fn resolves_backend_codex_targets_to_upstream_paths() {
        assert_eq!(
            resolve_upstream_target("/backend-api/codex/responses").unwrap(),
            "/responses"
        );
        assert_eq!(
            resolve_upstream_target("/backend-api/codex/responses/compact").unwrap(),
            "/responses/compact"
        );
        assert_eq!(
            resolve_upstream_target("/v1/responses?debug=1").unwrap(),
            "/responses?debug=1"
        );
    }

    #[test]
    fn aligns_prompt_cache_key_with_session_id() {
        let api_key = ResolvedLocalApiKey {
            id: "client-key-1".to_string(),
            label: "Client".to_string(),
            model_prefix: None,
            allowed_models: Vec::new(),
            excluded_models: Vec::new(),
        };
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/backend-api/codex/responses".to_string(),
            headers: HashMap::new(),
            body: serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "prompt_cache_key": "cache-123",
            }))
            .unwrap(),
        };

        align_codex_prompt_cache(&mut request, &api_key).unwrap();
        let body = serde_json::from_slice::<Value>(&request.body).unwrap();
        assert_eq!(
            request.headers.get("session_id").map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            request.headers.get("conversation_id").map(String::as_str),
            Some("cache-123")
        );
        assert_eq!(
            body.get("prompt_cache_key").and_then(Value::as_str),
            Some("cache-123")
        );
    }

    #[test]
    fn applies_codex_official_empty_headers() {
        let mut request = ParsedRequest {
            method: "POST".to_string(),
            target: "/backend-api/codex/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":"hello"}"#.to_vec(),
        };

        apply_codex_official_headers(&mut request);

        for key in [
            "version",
            "x-codex-turn-state",
            "x-codex-turn-metadata",
            "x-client-request-id",
            "x-responsesapi-include-timing-metrics",
        ] {
            assert_eq!(request.headers.get(key).map(String::as_str), Some(""));
        }
    }

    #[test]
    fn parses_websocket_usage_limit_error() {
        let message = Message::Text(
            r#"{"type":"error","status":429,"body":{"error":{"type":"usage_limit_reached","message":"usage limit reached","resets_in_seconds":7}}}"#
                .into(),
        );

        let error = parse_websocket_upstream_error(&message).expect("error should parse");

        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS.as_u16());
        assert_eq!(error.category, "usage_limit_reached");
        assert_eq!(error.retry_after, Some(Duration::from_secs(7)));
        assert!(error.body.contains("usage_limit_reached"));
    }

    #[test]
    fn parses_websocket_connection_limit_error() {
        let message = Message::Text(
            r#"{"type":"error","status":429,"body":{"error":{"code":"websocket_connection_limit_reached","type":"server_error","message":"too many websocket connections"}},"headers":{"retry-after":"1"}}"#
                .into(),
        );

        let error = parse_websocket_upstream_error(&message).expect("error should parse");

        assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS.as_u16());
        assert_eq!(error.category, "websocket_connection_limit_reached");
        assert_eq!(error.retry_after, Some(Duration::from_secs(1)));
        assert!(error.body.contains("websocket_connection_limit_reached"));
    }

    #[test]
    fn websocket_handshake_unauthorized_is_auth_unavailable() {
        let error = websocket_connect_error_from_http_response(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"type":"invalid_token","message":"bad access token"}}"#.to_string(),
        );

        assert_eq!(error.status, Some(StatusCode::UNAUTHORIZED.as_u16()));
        assert_eq!(error.category, "auth_unavailable");
        assert!(error.message.contains("bad access token"));
    }

    #[test]
    fn api_key_accounts_are_eligible_for_local_access_pool() {
        let account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
        );

        assert!(is_local_access_eligible_account(&account, true));
        assert_eq!(
            account_upstream_base_url(&account),
            "https://relay.example/v1"
        );
    }

    #[test]
    fn builds_upstream_websocket_url_from_custom_base_url() {
        let https_account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
        );
        let http_account = CodexAccount::new_api_key(
            "api-2".to_string(),
            "local@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:8080/v1".to_string()),
            Some("local".to_string()),
            Some("Local".to_string()),
        );

        assert_eq!(
            build_upstream_websocket_url(&https_account, "/responses").unwrap(),
            "wss://relay.example/v1/responses"
        );
        assert_eq!(
            build_upstream_websocket_url(&http_account, "/responses").unwrap(),
            "ws://127.0.0.1:8080/v1/responses"
        );
    }
}
