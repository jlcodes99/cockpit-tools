use crate::models::codex::{CodexAccount, CodexApiProviderMode};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountStats, CodexLocalAccessCollection, CodexLocalAccessPortCleanupResult,
    CodexLocalAccessRoutingStrategy, CodexLocalAccessScope, CodexLocalAccessState,
    CodexLocalAccessStats, CodexLocalAccessStatsWindow, CodexLocalAccessTestFailure,
    CodexLocalAccessTestResult, CodexLocalAccessUsageEvent, CodexLocalAccessUsageStats,
};
use crate::modules::atomic_write::write_string_atomic;
use crate::modules::{codex_account, codex_oauth, codex_wakeup, logger, process};
use base64::{engine::general_purpose, Engine as _};
use futures_util::StreamExt;
use rand::{distributions::Alphanumeric, Rng};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, Method, NoProxy, Proxy, StatusCode, Url};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex as TokioMutex};
use tokio::time::{timeout, Duration};

const CODEX_LOCAL_ACCESS_FILE: &str = "codex_local_access.json";
const CODEX_LOCAL_ACCESS_STATS_FILE: &str = "codex_local_access_stats.json";
const CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST: &str = "127.0.0.1";
const CODEX_LOCAL_ACCESS_LAN_BIND_HOST: &str = "0.0.0.0";
const CODEX_LOCAL_ACCESS_URL_HOST: &str = "127.0.0.1";
const MAX_HTTP_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REQUEST_RETRY_WAIT: Duration = Duration::from_secs(3);
const MAX_REQUEST_RETRY_ATTEMPTS: usize = 1;
const UPSTREAM_SEND_RETRY_ATTEMPTS: usize = 3;
const UPSTREAM_SEND_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
const UPSTREAM_SEND_RETRY_MAX_DELAY: Duration = Duration::from_millis(1200);
const SINGLE_ACCOUNT_STATUS_RETRY_ATTEMPTS: usize = 2;
const SINGLE_ACCOUNT_STATUS_RETRY_BASE_DELAY: Duration = Duration::from_millis(300);
const SINGLE_ACCOUNT_STATUS_RETRY_MAX_DELAY: Duration = Duration::from_millis(1500);
const STATS_FLUSH_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RETRY_CREDENTIALS_PER_REQUEST: usize = 8;
const RESPONSE_AFFINITY_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const MAX_RESPONSE_AFFINITY_BINDINGS: usize = 4096;
const PREPARED_ACCOUNT_CACHE_TTL_MS: i64 = 30 * 1000;
const DAY_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
const WEEK_WINDOW_MS: i64 = 7 * DAY_WINDOW_MS;
const MONTH_WINDOW_MS: i64 = 30 * DAY_WINDOW_MS;
const GATEWAY_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const GATEWAY_PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const GATEWAY_PORT_RELEASE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const UPSTREAM_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const DEFAULT_CODEX_USER_AGENT: &str =
    "codex-tui/0.118.0 (Mac OS 26.3.1; arm64) iTerm.app/3.6.9 (codex-tui; 0.118.0)";
const DEFAULT_CODEX_ORIGINATOR: &str = "codex-tui";
const CORS_ALLOW_HEADERS: &str = "Authorization, Content-Type, OpenAI-Beta, X-API-Key, X-Codex-Beta-Features, X-Client-Request-Id, Originator, Session_id, ChatGPT-Account-Id";
const DEFAULT_CODEX_MODELS: &[&str] = &[
    "gpt-5-codex",
    "gpt-5-codex-mini",
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
const DEFAULT_IMAGES_MAIN_MODEL: &str = "gpt-5.4-mini";
const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const RESPONSES_PATH: &str = "/v1/responses";
const IMAGES_GENERATIONS_PATH: &str = "/v1/images/generations";
const IMAGES_EDITS_PATH: &str = "/v1/images/edits";
static GATEWAY_RUNTIME: OnceLock<TokioMutex<GatewayRuntime>> = OnceLock::new();
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
    prepared_accounts: HashMap<String, CachedPreparedAccount>,
    running: bool,
    actual_port: Option<u16>,
    actual_bind_host: Option<String>,
    last_error: Option<String>,
    shutdown_sender: Option<watch::Sender<bool>>,
    task: Option<tokio::task::JoinHandle<()>>,
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
    next_retry_at_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedPreparedAccount {
    account: CodexAccount,
    cached_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpstreamHttpClientSignature {
    proxy_url: Option<String>,
    no_proxy: Option<String>,
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
}

struct ResponseUsageCollector {
    is_stream: bool,
    body: Vec<u8>,
    stream_buffer: Vec<u8>,
    usage: Option<UsageCapture>,
    response_id: Option<String>,
}

#[derive(Debug)]
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

fn upstream_http_client_cache() -> &'static Mutex<Option<CachedUpstreamHttpClient>> {
    UPSTREAM_HTTP_CLIENT.get_or_init(|| Mutex::new(None))
}

fn current_upstream_http_client_signature() -> UpstreamHttpClientSignature {
    let config = crate::modules::config::get_user_config();
    if !config.global_proxy_enabled {
        return UpstreamHttpClientSignature {
            proxy_url: None,
            no_proxy: None,
        };
    }

    let proxy_url = config.global_proxy_url.trim();
    if proxy_url.is_empty() {
        return UpstreamHttpClientSignature {
            proxy_url: None,
            no_proxy: None,
        };
    }

    let no_proxy = config.global_proxy_no_proxy.trim();
    UpstreamHttpClientSignature {
        proxy_url: Some(proxy_url.to_string()),
        no_proxy: (!no_proxy.is_empty()).then(|| no_proxy.to_string()),
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

fn build_upstream_http_client(signature: &UpstreamHttpClientSignature) -> Result<Client, String> {
    let mut builder = Client::builder();

    if let Some(proxy_url) = signature.proxy_url.as_deref() {
        let mut proxy =
            Proxy::all(proxy_url).map_err(|e| format!("Codex 本地接入代理地址无效: {}", e))?;
        if let Some(no_proxy) = signature.no_proxy.as_deref() {
            proxy = proxy.no_proxy(NoProxy::from_string(no_proxy));
        }
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| format!("创建 Codex 上游 HTTP 客户端失败: {}", e))
}

fn log_upstream_http_client_signature(signature: &UpstreamHttpClientSignature) {
    match signature.proxy_url.as_deref() {
        Some(proxy_url) => logger::log_info(&format!(
            "[CodexLocalAccess] 上游 HTTP 客户端已应用全局代理 proxy_url={} no_proxy={}",
            redact_proxy_url_for_log(proxy_url),
            signature.no_proxy.as_deref().unwrap_or("<empty>")
        )),
        None => logger::log_info("[CodexLocalAccess] 上游 HTTP 客户端使用系统代理配置"),
    }
}

fn upstream_http_client() -> Result<Client, String> {
    let signature = current_upstream_http_client_signature();
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
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    Ok(home
        .join(".antigravity_cockpit")
        .join(CODEX_LOCAL_ACCESS_FILE))
}

fn local_access_stats_file_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    Ok(home
        .join(".antigravity_cockpit")
        .join(CODEX_LOCAL_ACCESS_STATS_FILE))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn is_prepared_account_cache_valid(entry: &CachedPreparedAccount, now: i64) -> bool {
    now.saturating_sub(entry.cached_at_ms) <= PREPARED_ACCOUNT_CACHE_TTL_MS
        && !codex_oauth::is_token_expired(&entry.account.tokens.access_token)
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

    if model_ids.is_empty() {
        model_ids = DEFAULT_CODEX_MODELS
            .iter()
            .map(|model| (*model).to_string())
            .collect();
    }

    let mut seen_model_ids: HashSet<String> = model_ids
        .iter()
        .map(|model| model.trim().to_ascii_lowercase())
        .filter(|model| !model.is_empty())
        .collect();
    if seen_model_ids.insert(CODEX_IMAGE_MODEL_ID.to_string()) {
        model_ids.push(CODEX_IMAGE_MODEL_ID.to_string());
    }

    model_ids
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

    let Some(body_obj) = body_value.as_object_mut() else {
        return Ok(None);
    };
    let Some(model) = body_obj.get("model").and_then(Value::as_str) else {
        return Ok(None);
    };

    let resolved_model = resolve_supported_model_alias(model);
    if resolved_model == model {
        return Ok(None);
    }

    body_obj.insert("model".to_string(), Value::String(resolved_model));
    serde_json::to_vec(&body_value)
        .map(Some)
        .map_err(|e| format!("重写请求 model 失败: {}", e))
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
    path == RESPONSES_PATH || path.ends_with("/responses")
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
        return RequestRoutingHint::default();
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

    ensure_image_generation_tool_in_object(&mut responses_obj);

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
        if let Some(rewritten_body) = rewrite_request_model_alias(&request.body)? {
            request.body = rewritten_body;
        }
        if is_responses_request(&request.target) {
            if let Some(mut body_value) = parse_request_body_json(&request.body) {
                if let Some(body_obj) = body_value.as_object_mut() {
                    if ensure_image_generation_tool_in_object(body_obj) {
                        request.body = serde_json::to_vec(&body_value)
                            .map_err(|e| format!("序列化 responses 请求体失败: {}", e))?;
                    }
                }
            }
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
    Some(format!("{}\u{1f}{}", account_id, model_key))
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
        "business" => 650,
        "team" => 640,
        "edu" => 630,
        // CPA 对齐：plan_type='pro' 默认视为 promax (20x)，
        // 只有显式声明 prolite 时才降级
        "pro" => match auth_file_plan_type {
            Some("prolite") => 520,
            _ => 560, // pro / promax 均为 20x 级别
        },
        "plus" => 420,
        "go" => 360,
        "free" => 300,
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

fn apply_routing_strategy(
    account_ids: &[String],
    strategy: CodexLocalAccessRoutingStrategy,
) -> Vec<String> {
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
        daily: CodexLocalAccessStatsWindow {
            since: day_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
        },
        weekly: CodexLocalAccessStatsWindow {
            since: week_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
        },
        monthly: CodexLocalAccessStatsWindow {
            since: month_since,
            updated_at: now,
            totals: CodexLocalAccessUsageStats::default(),
            accounts: Vec::new(),
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

fn trim_recent_events(events: &mut Vec<CodexLocalAccessUsageEvent>, month_since: i64) {
    events.retain(|event| event.timestamp > 0 && event.timestamp >= month_since);
    events.sort_by_key(|event| event.timestamp);
}

fn append_usage_event(
    events: &mut Vec<CodexLocalAccessUsageEvent>,
    now: i64,
    account_id: Option<&str>,
    account_email: Option<&str>,
    success: bool,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
) {
    let usage = usage.cloned().unwrap_or_default();
    events.push(CodexLocalAccessUsageEvent {
        timestamp: now,
        account_id: account_id.unwrap_or_default().trim().to_string(),
        email: account_email.unwrap_or_default().trim().to_string(),
        success,
        latency_ms,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        cached_tokens: usage.cached_tokens,
        reasoning_tokens: usage.reasoning_tokens,
    });
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
        event.success,
        event.latency_ms,
        Some(&usage),
    );
    upsert_account_usage_stats(
        &mut window.accounts,
        Some(event.account_id.as_str()),
        Some(event.email.as_str()),
        event.success,
        event.latency_ms,
        Some(&usage),
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

fn build_lan_base_url(port: u16) -> Option<String> {
    resolve_primary_lan_ipv4().map(|addr| format!("http://{addr}:{port}/v1"))
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
    let output = Command::new("ifconfig").arg("-a").output();
    match output {
        Ok(output) => parse_ifconfig_ipv4_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "linux")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    match output {
        Ok(output) => parse_linux_ip_addr_candidates(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Vec::new(),
    }
}

#[cfg(target_os = "windows")]
fn collect_private_lan_ipv4_candidates() -> Vec<LanIpv4Candidate> {
    let mut command = Command::new("ipconfig");
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
        Some("codex_local_access".to_string()),
        Some("Codex API Service".to_string()),
    );
    runtime_account.account_name = Some("API Service".to_string());
    runtime_account.bound_oauth_account_id = bound_oauth_account_id;
    runtime_account
}

fn generate_local_api_key() -> String {
    let suffix: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    format!("agt_codex_{}", suffix)
}

fn allocate_random_local_port(bind_host: &str) -> Result<u16, String> {
    let listener =
        StdTcpListener::bind((bind_host, 0)).map_err(|e| format!("分配本地接入端口失败: {}", e))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("读取本地接入端口失败: {}", e))
}

fn load_collection_from_disk() -> Result<Option<CodexLocalAccessCollection>, String> {
    let path = local_access_file_path()?;
    if !path.exists() {
        return Ok(None);
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取本地接入配置失败: {}", e))?;
    let parsed = serde_json::from_str::<CodexLocalAccessCollection>(&content)
        .map_err(|e| format!("解析本地接入配置失败: {}", e))?;
    Ok(Some(parsed))
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
    recompute_time_windows(stats, now);
}

fn load_stats_from_disk() -> Result<CodexLocalAccessStats, String> {
    let path = local_access_stats_file_path()?;
    if !path.exists() {
        return Ok(empty_stats_snapshot());
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 API 服务统计失败: {}", e))?;
    let mut parsed = serde_json::from_str::<CodexLocalAccessStats>(&content)
        .map_err(|e| format!("解析 API 服务统计失败: {}", e))?;
    normalize_stats(&mut parsed);
    Ok(parsed)
}

fn save_stats_to_disk(stats: &CodexLocalAccessStats) -> Result<(), String> {
    let path = local_access_stats_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 API 服务统计目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(stats)
        .map_err(|e| format!("序列化 API 服务统计失败: {}", e))?;
    write_string_atomic(&path, &content)
}

fn prune_runtime_routing_state(runtime: &mut GatewayRuntime, now: i64) {
    runtime
        .response_affinity
        .retain(|_, binding| now.saturating_sub(binding.updated_at_ms) <= RESPONSE_AFFINITY_TTL_MS);
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

async fn clear_model_cooldown(account_id: &str, model_key: &str) {
    let Some(cooldown_key) = build_cooldown_key(account_id, model_key) else {
        return;
    };

    let mut runtime = gateway_runtime().lock().await;
    let now = now_ms();
    prune_runtime_routing_state(&mut runtime, now);
    runtime.model_cooldowns.remove(&cooldown_key);
}

async fn set_model_cooldown(account_id: &str, model_key: &str, retry_after: Duration) {
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
    runtime
        .model_cooldowns
        .insert(cooldown_key, AccountModelCooldown { next_retry_at_ms });
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
    if account.is_api_key_auth() {
        return false;
    }
    if restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref()) {
        return false;
    }
    true
}

fn sanitize_collection(
    collection: &mut CodexLocalAccessCollection,
) -> Result<(bool, HashSet<String>), String> {
    let mut changed = false;

    if collection.port == 0 {
        collection.port = allocate_random_local_port(bind_host_for_collection(collection))?;
        changed = true;
    }
    if collection.api_key.trim().is_empty() {
        collection.api_key = generate_local_api_key();
        changed = true;
    }
    if collection.created_at <= 0 {
        collection.created_at = now_ms();
        changed = true;
    }
    if collection.updated_at <= 0 {
        collection.updated_at = now_ms();
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
            port: allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
            api_key: generate_local_api_key(),
            access_scope: CodexLocalAccessScope::Localhost,
            routing_strategy: CodexLocalAccessRoutingStrategy::default(),
            restrict_free_accounts: true,
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
    }

    Ok(())
}

async fn ensure_gateway_matches_runtime() -> Result<(), String> {
    let (collection, running, actual_port, actual_bind_host, stale_task) = {
        let mut runtime = gateway_runtime().lock().await;
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
            stale_task,
        )
    };

    if let Some(task) = stale_task {
        let _ = task.await;
    }

    let Some(collection) = collection else {
        stop_gateway().await;
        return Ok(());
    };

    if !collection.enabled {
        stop_gateway().await;
        return Ok(());
    }

    let bind_host = bind_host_for_collection(&collection);
    if running
        && actual_port == Some(collection.port)
        && actual_bind_host.as_deref() == Some(bind_host)
    {
        return Ok(());
    }

    let stopped_endpoint = stop_gateway().await;
    if let Some(endpoint) = stopped_endpoint {
        wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    }

    let listener = match bind_gateway_listener(bind_host, collection.port).await {
        Ok(listener) => listener,
        Err(error) => {
            let message = format_gateway_bind_error(bind_host, collection.port, &error);
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.last_error = Some(message.clone());
            return Err(message);
        }
    };
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(false);
    let port = collection.port;
    let bind_host = bind_host.to_string();
    let task_bind_host = bind_host.clone();

    let task = tokio::spawn(async move {
        logger::log_codex_api_info(&format!(
            "[CodexLocalAccess] 本地接入服务已启动: bind={}:{} base={}",
            task_bind_host,
            port,
            build_base_url(port)
        ));

        loop {
            tokio::select! {
                changed = shutdown_receiver.changed() => {
                    if changed.is_ok() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, addr)) => {
                            tokio::spawn(async move {
                                if let Err(err) = handle_connection(stream, addr).await {
                                    logger::log_codex_api_warn(&format!(
                                        "[CodexLocalAccess] 请求处理失败 {}: {}",
                                        addr, err
                                    ));
                                }
                            });
                        }
                        Err(err) => {
                            logger::log_codex_api_warn(&format!(
                                "[CodexLocalAccess] 接收请求失败: {}",
                                err
                            ));
                            break;
                        }
                    }
                }
            }
        }

        let mut runtime = gateway_runtime().lock().await;
        if runtime.actual_port == Some(port)
            && runtime.actual_bind_host.as_deref() == Some(&task_bind_host)
        {
            runtime.running = false;
            runtime.actual_port = None;
            runtime.actual_bind_host = None;
            runtime.shutdown_sender = None;
        }
    });

    let mut runtime = gateway_runtime().lock().await;
    runtime.running = true;
    runtime.actual_port = Some(collection.port);
    runtime.actual_bind_host = Some(bind_host);
    runtime.last_error = None;
    runtime.shutdown_sender = Some(shutdown_sender);
    runtime.task = Some(task);
    Ok(())
}

async fn stop_gateway() -> Option<GatewayBindEndpoint> {
    let (shutdown_sender, task, endpoint) = {
        let mut runtime = gateway_runtime().lock().await;
        let endpoint = runtime
            .actual_port
            .zip(runtime.actual_bind_host.clone())
            .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
        runtime.running = false;
        runtime.actual_port = None;
        runtime.actual_bind_host = None;
        (
            runtime.shutdown_sender.take(),
            runtime.task.take(),
            endpoint,
        )
    };

    if let Some(sender) = shutdown_sender {
        let _ = sender.send(true);
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
    success: bool,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
) {
    target.request_count = target.request_count.saturating_add(1);
    if success {
        target.success_count = target.success_count.saturating_add(1);
    } else {
        target.failure_count = target.failure_count.saturating_add(1);
    }
    target.total_latency_ms = target.total_latency_ms.saturating_add(latency_ms);

    if let Some(usage) = usage {
        target.input_tokens = target.input_tokens.saturating_add(usage.input_tokens);
        target.output_tokens = target.output_tokens.saturating_add(usage.output_tokens);
        target.total_tokens = target.total_tokens.saturating_add(usage.total_tokens);
        target.cached_tokens = target.cached_tokens.saturating_add(usage.cached_tokens);
        target.reasoning_tokens = target
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
    }
}

fn upsert_account_usage_stats(
    accounts: &mut Vec<CodexLocalAccessAccountStats>,
    account_id: Option<&str>,
    account_email: Option<&str>,
    success: bool,
    latency_ms: u64,
    usage: Option<&UsageCapture>,
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
        apply_usage_stats(&mut account_stats.usage, success, latency_ms, usage);
        return;
    }

    let mut account_stats = CodexLocalAccessAccountStats {
        account_id: account_id.to_string(),
        email: normalized_email,
        usage: CodexLocalAccessUsageStats::default(),
        updated_at,
    };
    apply_usage_stats(&mut account_stats.usage, success, latency_ms, usage);
    accounts.push(account_stats);
}

async fn record_request_stats(
    account_id: Option<&str>,
    account_email: Option<&str>,
    success: bool,
    latency_ms: u64,
    usage: Option<UsageCapture>,
) -> Result<(), String> {
    {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_ms();
        let usage_ref = usage.as_ref();
        if runtime.stats.since <= 0 {
            runtime.stats.since = now;
        }
        runtime.stats.updated_at = now;
        apply_usage_stats(&mut runtime.stats.totals, success, latency_ms, usage_ref);
        upsert_account_usage_stats(
            &mut runtime.stats.accounts,
            account_id,
            account_email,
            success,
            latency_ms,
            usage_ref,
            now,
        );
        append_usage_event(
            &mut runtime.stats.events,
            now,
            account_id,
            account_email,
            success,
            latency_ms,
            usage_ref,
        );

        normalize_stats(&mut runtime.stats);
        runtime.stats_dirty = true;
    }

    schedule_stats_flush_if_needed().await;
    Ok(())
}

fn build_state_snapshot(runtime: &GatewayRuntime) -> CodexLocalAccessState {
    let collection = runtime.collection.clone();
    let member_count = collection
        .as_ref()
        .map(|item| item.account_ids.len())
        .unwrap_or(0);
    let api_port_url = collection
        .as_ref()
        .map(|item| build_api_port_url(item.port));
    let base_url = collection.as_ref().map(|item| build_base_url(item.port));
    let lan_base_url = collection.as_ref().and_then(|item| {
        if item.access_scope == CodexLocalAccessScope::Lan {
            build_lan_base_url(item.port)
        } else {
            None
        }
    });
    let model_ids = supported_codex_model_ids();
    let mut stats = runtime.stats.clone();
    stats.events.clear();

    CodexLocalAccessState {
        collection,
        running: runtime.running,
        api_port_url,
        base_url,
        lan_base_url,
        model_ids,
        last_error: runtime.last_error.clone(),
        member_count,
        stats,
    }
}

async fn snapshot_state() -> Result<CodexLocalAccessState, String> {
    ensure_runtime_loaded_without_start().await?;
    if let Err(err) = ensure_gateway_matches_runtime().await {
        let mut runtime = gateway_runtime().lock().await;
        runtime.last_error = Some(err);
        return Ok(build_state_snapshot(&runtime));
    }
    let runtime = gateway_runtime().lock().await;
    Ok(build_state_snapshot(&runtime))
}

pub async fn get_local_access_state() -> Result<CodexLocalAccessState, String> {
    snapshot_state().await
}

pub async fn activate_local_access_for_dir(
    profile_dir: &Path,
) -> Result<CodexLocalAccessState, String> {
    let state = set_local_access_enabled(true).await?;
    let collection = state
        .collection
        .clone()
        .ok_or_else(|| "API 服务集合尚未创建".to_string())?;
    let base_url = state
        .base_url
        .clone()
        .unwrap_or_else(|| build_base_url(collection.port));
    let bound_oauth_account_id =
        normalize_optional_account_ref(collection.bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_oauth_account_id.as_deref() {
        let _ = validate_local_access_bound_oauth_account(bound_id)?;
        let _ = codex_account::ensure_managed_account_fresh(bound_id).await?;
    }
    let runtime_account =
        build_runtime_account(base_url, collection.api_key.clone(), bound_oauth_account_id);
    codex_account::write_account_bundle_to_dir(profile_dir, &runtime_account)?;
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
    let client = match Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(90))
        .build()
    {
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
                "在 API 服务账号集合中加入可用的 Codex OAuth 账号，并确认未被 Free 账号限制或账号类型限制拦截。",
            )
        } else {
            (
                "上游服务或代理不可用",
                "上游请求",
                "检查全局代理、网络连通性和 Codex 上游服务状态；如只影响单个账号，刷新或移除该账号后重试。",
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
            "在 API 服务账号集合中加入可用的 Codex OAuth 账号后再测试。",
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
                port: allocate_random_local_port(CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST)?,
                api_key: generate_local_api_key(),
                access_scope: CodexLocalAccessScope::Localhost,
                routing_strategy: CodexLocalAccessRoutingStrategy::default(),
                restrict_free_accounts: true,
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

    collection.api_key = generate_local_api_key();
    collection.updated_at = now_ms();
    save_collection_to_disk(&collection)?;

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

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
    ensure_runtime_loaded().await?;

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

    {
        let mut runtime = gateway_runtime().lock().await;
        sync_runtime_collection(&mut runtime, collection);
    }

    ensure_gateway_matches_runtime().await?;
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

async fn read_http_request<R>(stream: &mut R) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = Vec::with_capacity(4096);
    let mut chunk = [0u8; 2048];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;

    loop {
        let bytes_read = timeout(REQUEST_READ_TIMEOUT, stream.read(&mut chunk))
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

fn build_local_models_response() -> Value {
    let data: Vec<Value> = supported_codex_model_ids()
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
            }
        } else {
            let parsed = serde_json::from_slice::<Value>(&self.body).ok();
            ResponseCapture {
                usage: parsed.as_ref().and_then(extract_usage_capture),
                response_id: parsed.as_ref().and_then(extract_response_id),
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
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
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
    if !target.starts_with("/v1") {
        return Err("仅支持 /v1 路径".to_string());
    }

    let trimmed = target.trim_start_matches("/v1");
    if trimmed.is_empty() {
        Ok("/".to_string())
    } else if trimmed.starts_with('/') {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("/{}", trimmed))
    }
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

fn should_try_next_account(status: StatusCode, body: &str) -> bool {
    if status == StatusCode::UNAUTHORIZED {
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

    let response = json_response(status, status_text, &json!({ "error": message }));
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
        return Ok(parsed);
    }

    let mut stream_buffer = body_bytes.to_vec();
    let mut completed_response: Option<Value> = None;
    let mut output_text = String::new();
    let mut output_items: Vec<Value> = Vec::new();

    let mut process_frame = |frame: &[u8]| {
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

        let Ok(value) = serde_json::from_str::<Value>(&payload) else {
            return;
        };
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
    };

    loop {
        let Some((boundary_index, separator_len)) = find_sse_frame_boundary(&stream_buffer) else {
            break;
        };
        let frame = stream_buffer[..boundary_index].to_vec();
        stream_buffer.drain(..boundary_index + separator_len);
        process_frame(&frame);
    }
    if !stream_buffer.is_empty() {
        process_frame(&stream_buffer);
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
        while let Some(chunk_result) = body_stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
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
        while let Some(chunk_result) = body_stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("读取上游图片响应失败: {}", e))?;
            let transformed = transformer.feed(&chunk);
            write_chunked_response_chunk(stream, &transformed).await?;
        }

        let (tail, response_capture) = transformer.finish();
        write_chunked_response_chunk(stream, &tail).await?;
        finish_chunked_response(stream).await?;
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
) -> Result<ResponseCapture, String> {
    match response_adapter {
        GatewayResponseAdapter::Passthrough { request_is_stream } => {
            write_upstream_response(stream, upstream, request_is_stream).await
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
            )
            .await
        }
    }
}

async fn write_upstream_response(
    stream: &mut TcpStream,
    upstream: reqwest::Response,
    request_is_stream: bool,
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
    while let Some(chunk_result) = body_stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取上游响应失败: {}", e))?;
        if chunk.is_empty() {
            continue;
        }
        write_chunked_response_chunk(stream, &chunk).await?;
        usage_collector.feed(&chunk);
    }

    finish_chunked_response(stream).await?;
    Ok(usage_collector.finish())
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

fn upstream_send_retry_delay(retry_attempt: usize) -> Duration {
    let multiplier = match retry_attempt {
        0 | 1 => 1u32,
        2 => 2u32,
        _ => 4u32,
    };
    let delay = UPSTREAM_SEND_RETRY_BASE_DELAY.saturating_mul(multiplier);
    if delay > UPSTREAM_SEND_RETRY_MAX_DELAY {
        UPSTREAM_SEND_RETRY_MAX_DELAY
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

fn single_account_status_retry_delay(retry_attempt: usize) -> Duration {
    let multiplier = match retry_attempt {
        0 | 1 => 1u32,
        2 => 2u32,
        _ => 4u32,
    };
    let delay = SINGLE_ACCOUNT_STATUS_RETRY_BASE_DELAY.saturating_mul(multiplier);
    if delay > SINGLE_ACCOUNT_STATUS_RETRY_MAX_DELAY {
        SINGLE_ACCOUNT_STATUS_RETRY_MAX_DELAY
    } else {
        delay
    }
}

async fn send_upstream_request(
    method: &str,
    target: &str,
    headers: &HashMap<String, String>,
    body: &[u8],
    account: &CodexAccount,
) -> Result<reqwest::Response, String> {
    let method =
        Method::from_bytes(method.as_bytes()).map_err(|e| format!("不支持的请求方法: {}", e))?;
    let url = format!("{}{}", UPSTREAM_CODEX_BASE_URL, target);
    let client = upstream_http_client()?;
    for retry_attempt in 0..=UPSTREAM_SEND_RETRY_ATTEMPTS {
        let mut request = client.request(method.clone(), &url);

        for (name, value) in headers {
            if matches!(
                name.as_str(),
                "authorization"
                    | "host"
                    | "content-length"
                    | "connection"
                    | "accept-encoding"
                    | "x-api-key"
            ) {
                continue;
            }
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("无效请求头 {}: {}", name, e))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| format!("无效请求头值 {}: {}", name, e))?;
            request = request.header(header_name, header_value);
        }

        request = request.header(
            AUTHORIZATION,
            format!("Bearer {}", account.tokens.access_token.trim()),
        );
        if !headers.contains_key("user-agent") {
            request = request.header(USER_AGENT, DEFAULT_CODEX_USER_AGENT);
        }
        if !headers.contains_key("originator") {
            request = request.header("Originator", DEFAULT_CODEX_ORIGINATOR);
        }
        if let Some(account_id) = resolve_upstream_account_id(account) {
            request = request.header("ChatGPT-Account-Id", account_id);
        }
        if !headers.contains_key("accept") {
            request = request.header(
                ACCEPT,
                if is_stream_request(headers, body) {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            );
        }
        request = request.header("Connection", "Keep-Alive");
        if !headers.contains_key("content-type") && !body.is_empty() {
            request = request.header(CONTENT_TYPE, "application/json");
        }
        if !body.is_empty() {
            request = request.body(body.to_vec());
        }

        match request.send().await {
            Ok(response) => return Ok(response),
            Err(error) => {
                let should_retry = retry_attempt < UPSTREAM_SEND_RETRY_ATTEMPTS
                    && should_retry_upstream_send_error(&error);
                if !should_retry {
                    return Err(format!("请求 Codex 上游失败: {}", error));
                }
                tokio::time::sleep(upstream_send_retry_delay(retry_attempt + 1)).await;
            }
        }
    }

    Err("请求 Codex 上游失败: 未知错误".to_string())
}

async fn proxy_request_with_account_pool(
    request: &ParsedRequest,
    collection: &CodexLocalAccessCollection,
) -> Result<ProxyDispatchSuccess, ProxyDispatchError> {
    if collection.account_ids.is_empty() {
        return Err(ProxyDispatchError {
            status: 503,
            message: "本地接入集合暂无账号".to_string(),
            account_id: None,
            account_email: None,
        });
    }

    let upstream_target =
        resolve_upstream_target(&request.target).map_err(|err| ProxyDispatchError {
            status: 400,
            message: err,
            account_id: None,
            account_email: None,
        })?;
    let routing_hint = build_request_routing_hint(request);
    let total = collection.account_ids.len();
    let max_credential_attempts = total.min(MAX_RETRY_CREDENTIALS_PER_REQUEST).max(1);
    let affinity_account_id = match routing_hint.previous_response_id.as_deref() {
        Some(previous_response_id) => resolve_affinity_account(previous_response_id).await,
        None => None,
    };
    let mut last_status = 503u16;
    let mut last_error = "本地接入集合暂无可用账号".to_string();
    let mut last_account_id: Option<String> = None;
    let mut last_account_email: Option<String> = None;
    let mut attempts = 0usize;
    let mut retry_round = 0usize;
    let mut earliest_cooldown_wait: Option<Duration>;

    loop {
        let start = GATEWAY_ROUND_ROBIN_CURSOR.fetch_add(1, Ordering::Relaxed);
        let ordered_account_ids = build_ordered_account_ids(
            &collection.account_ids,
            start,
            affinity_account_id.as_deref(),
        );
        let strategy_account_ids = pin_account_to_front(
            apply_routing_strategy(&ordered_account_ids, collection.routing_strategy),
            affinity_account_id.as_deref(),
        );
        let mut attempted_in_round = false;
        let mut round_cooldown_wait: Option<Duration> = None;

        for account_id in strategy_account_ids {
            if attempts >= max_credential_attempts {
                break;
            }

            if let Some(wait) = get_model_cooldown_wait(&account_id, &routing_hint.model_key).await
            {
                round_cooldown_wait = Some(match round_cooldown_wait {
                    Some(current) if current <= wait => current,
                    _ => wait,
                });
                continue;
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
                    continue;
                }
            };

            if account.is_api_key_auth() {
                log_codex_api_failure(
                    None,
                    Some(request),
                    None,
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    "API Key 账号不支持加入本地接入",
                );
                last_error = "API Key 账号不支持加入本地接入".to_string();
                continue;
            }
            if collection.restrict_free_accounts && is_free_plan_type(account.plan_type.as_deref())
            {
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
                continue;
            }

            last_account_id = Some(account.id.clone());
            last_account_email = Some(account.email.clone());

            let mut single_account_status_retry_attempt = 0usize;
            loop {
                let first_response = send_upstream_request(
                    &request.method,
                    &upstream_target,
                    &request.headers,
                    &request.body,
                    &account,
                )
                .await;

                let mut response = match first_response {
                    Ok(response) => response,
                    Err(err) => {
                        log_codex_api_failure(
                            None,
                            Some(request),
                            None,
                            Some(account.id.as_str()),
                            Some(account.email.as_str()),
                            None,
                            format!("上游请求失败: {}", err).as_str(),
                        );
                        last_error = err;
                        break;
                    }
                };

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
                            )
                            .await
                            {
                                Ok(response) => response,
                                Err(err) => {
                                    log_codex_api_failure(
                                        None,
                                        Some(request),
                                        None,
                                        Some(account.id.as_str()),
                                        Some(account.email.as_str()),
                                        None,
                                        format!("刷新后重试上游失败: {}", err).as_str(),
                                    );
                                    last_error = err;
                                    break;
                                }
                            };

                            if response.status() == StatusCode::UNAUTHORIZED {
                                last_status = StatusCode::UNAUTHORIZED.as_u16();
                                invalidate_prepared_account(&account_id).await;
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
                                break;
                            }
                        }
                        Err(err) => {
                            invalidate_prepared_account(&account_id).await;
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
                            break;
                        }
                    }
                }

                if response.status().is_success() {
                    clear_model_cooldown(&account.id, &routing_hint.model_key).await;
                    return Ok(ProxyDispatchSuccess {
                        upstream: response,
                        account_id: account.id.clone(),
                        account_email: account.email.clone(),
                    });
                }

                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let message = summarize_upstream_error(status, &body);
                log_codex_api_failure(
                    None,
                    Some(request),
                    Some(status.as_u16()),
                    Some(account.id.as_str()),
                    Some(account.email.as_str()),
                    None,
                    format!("上游返回失败: {}", message).as_str(),
                );

                if let Some(retry_after) = parse_codex_retry_after(status, &body) {
                    set_model_cooldown(&account.id, &routing_hint.model_key, retry_after).await;
                    round_cooldown_wait = Some(match round_cooldown_wait {
                        Some(current) if current <= retry_after => current,
                        _ => retry_after,
                    });
                }

                let can_retry_single_account = total == 1
                    && single_account_status_retry_attempt < SINGLE_ACCOUNT_STATUS_RETRY_ATTEMPTS
                    && should_retry_single_account_upstream_status(status);
                if can_retry_single_account {
                    single_account_status_retry_attempt += 1;
                    tokio::time::sleep(single_account_status_retry_delay(
                        single_account_status_retry_attempt,
                    ))
                    .await;
                    continue;
                }

                if should_try_next_account(status, &body) {
                    last_status = status.as_u16();
                    last_error =
                        format!("账号 {} 当前不可用，已尝试轮转: {}", account.email, message);
                    break;
                }

                return Err(ProxyDispatchError {
                    status: status.as_u16(),
                    message,
                    account_id: Some(account.id.clone()),
                    account_email: Some(account.email.clone()),
                });
            }
        }

        earliest_cooldown_wait = round_cooldown_wait;
        let Some(wait) = earliest_cooldown_wait else {
            break;
        };
        if attempts >= max_credential_attempts
            || retry_round >= MAX_REQUEST_RETRY_ATTEMPTS
            || wait > MAX_REQUEST_RETRY_WAIT
        {
            if !attempted_in_round {
                return Err(ProxyDispatchError {
                    status: StatusCode::TOO_MANY_REQUESTS.as_u16(),
                    message: build_cooldown_unavailable_message(&routing_hint.model_key, wait),
                    account_id: affinity_account_id.clone(),
                    account_email: None,
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
    })
}

async fn handle_connection(
    mut stream: TcpStream,
    addr: std::net::SocketAddr,
) -> Result<(), String> {
    let raw_request = read_http_request(&mut stream).await?;
    let mut parsed = parse_http_request(&raw_request)?;

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
    if !parsed.target.starts_with("/v1/") {
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
        build_state_snapshot(&runtime)
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

    if api_key != collection.api_key {
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

        let response = json_response(200, "OK", &build_local_models_response());
        stream
            .write_all(&response)
            .await
            .map_err(|e| format!("写入模型响应失败: {}", e))?;
        return Ok(());
    }

    let started_at = Instant::now();
    let (prepared_request, response_adapter) = match prepare_gateway_request(parsed) {
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

    match proxy_request_with_account_pool(&prepared_request, &collection).await {
        Ok(success) => {
            let response_capture =
                write_gateway_response(&mut stream, success.upstream, response_adapter).await?;
            if let Some(response_id) = response_capture.response_id.as_deref() {
                bind_response_affinity(response_id, &success.account_id).await;
            }
            let latency_ms = started_at.elapsed().as_millis() as u64;
            if let Err(err) = record_request_stats(
                Some(success.account_id.as_str()),
                Some(success.account_email.as_str()),
                true,
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
            Ok(())
        }
        Err(error) => {
            let ProxyDispatchError {
                status,
                message,
                account_id,
                account_email,
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
                404 => "Not Found",
                405 => "Method Not Allowed",
                429 => "Too Many Requests",
                502 => "Bad Gateway",
                _ => "Service Unavailable",
            };
            let response = json_response(status, status_text, &json!({ "error": message }));
            let write_result = stream
                .write_all(&response)
                .await
                .map_err(|e| format!("写入错误响应失败: {}", e));
            if let Err(err) = record_request_stats(
                account_id.as_deref(),
                account_email.as_deref(),
                false,
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
        build_chat_completion_payload, build_chat_completion_stream_body, build_images_api_payload,
        build_local_models_response, build_ordered_account_ids, build_request_routing_hint,
        extract_usage_capture, is_responses_completion_event, parse_codex_retry_after,
        parse_responses_payload_from_upstream, prepare_gateway_request,
        resolve_supported_model_alias, should_retry_single_account_upstream_status,
        should_treat_response_as_stream, should_try_next_account, GatewayResponseAdapter,
        ParsedRequest, ResponseUsageCollector,
    };
    use reqwest::StatusCode;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use tokio::time::Duration;

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
        let response = build_local_models_response();
        let has_image_model = response
            .get("data")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .any(|model| model.get("id").and_then(Value::as_str) == Some("gpt-image-2"))
            })
            .unwrap_or(false);

        assert!(has_image_model);
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
        assert!(mapped_body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("image_generation")
            }))
            .unwrap_or(false));

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

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(!request_is_stream);
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
            body: br#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        assert_eq!(prepared.target, "/v1/responses");

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(request_is_stream);
            }
            _ => panic!("expected responses stream passthrough adapter"),
        }
    }

    #[test]
    fn injects_image_generation_tool_for_responses_requests() {
        let request = ParsedRequest {
            method: "POST".to_string(),
            target: "/v1/responses".to_string(),
            headers: HashMap::new(),
            body: br#"{"model":"gpt-5.4","input":"draw an icon"}"#.to_vec(),
        };

        let (prepared, adapter) = prepare_gateway_request(request).expect("request should map");
        let mapped_body: Value =
            serde_json::from_slice(&prepared.body).expect("mapped body should be json");
        assert!(mapped_body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) == Some("image_generation")
                    && tool.get("output_format").and_then(Value::as_str) == Some("png")
            }))
            .unwrap_or(false));

        match adapter {
            GatewayResponseAdapter::Passthrough { request_is_stream } => {
                assert!(!request_is_stream);
            }
            _ => panic!("expected passthrough adapter"),
        }
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
}
