// Codex 账号模块：Provider identity, API key normalization and DeepSeek provider helpers。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
use crate::models::codex::{
    CodexAccount, CodexAccountIndex, CodexAccountSummary, CodexAgentIdentity, CodexApiModelMapping,
    CodexApiProviderMode, CodexAppSpeed, CodexAuthFile, CodexAuthMode, CodexAuthTokens,
    CodexExperimentalModelDefinition, CodexJwtPayload, CodexQuickConfig, CodexTokens,
};
use crate::modules::{account, codex_oauth, logger};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::pkcs8::DecodePrivateKey;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use toml_edit::{value, Document};

static CODEX_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_TOKEN_REFRESH_LOCKS: std::sync::LazyLock<
    Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_ACCOUNT_SWITCH_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
static CODEX_ACCOUNT_MUTATION_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static CODEX_AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static CODEX_BATCH_IMPORT_COUNTER: AtomicU64 = AtomicU64::new(1);
static CODEX_BATCH_IMPORT_SESSIONS: std::sync::LazyLock<
    Mutex<HashMap<String, CodexBatchImportSession>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_FINGERPRINT_DEFAULT_SESSION_RESYNC_STARTED: AtomicBool = AtomicBool::new(false);
const CODEX_FINGERPRINT_DEFAULT_SESSION_MARKER: &str = "codex_fingerprint_default_session_v1";
const CODEX_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const ACCOUNT_CHECK_URL: &str = "https://chatgpt.com/backend-api/accounts/check/v4-2023-04-27";
const API_KEY_LOGIN_PLAN_TYPE: &str = "API_KEY";
const COCKPIT_API_LOGIN_PLAN_TYPE: &str = "Cockpit Api";
const COCKPIT_API_DEFAULT_ACCOUNT_NAME: &str = "Codex API";
const API_KEY_EMAIL_PREFIX: &str = "api-key";
const API_KEY_AUTH_MODE: &str = "apikey";
const CODEX_AUTH_TYPE: &str = "codex";
const CODEX_ACCOUNT_GROUPS_FILE: &str = "codex_account_groups.json";
const CODEX_ACCOUNT_TOMBSTONES_DIR: &str = "codex_account_tombstones";
const CODEX_CONFIG_FILE_NAME: &str = "config.toml";
const CODEX_CONFIG_CLI_AUTH_CREDENTIALS_STORE_KEY: &str = "cli_auth_credentials_store";
const CODEX_CONFIG_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CONFIG_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_CONFIG_MODEL_PROVIDERS_KEY: &str = "model_providers";
const CODEX_CONFIG_MODEL_CATALOG_JSON_KEY: &str = "model_catalog_json";
const CODEX_CONFIG_EXPERIMENTAL_BEARER_TOKEN_KEY: &str = "experimental_bearer_token";
const CODEX_CONFIG_HTTP_HEADERS_KEY: &str = "http_headers";
const CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY: &str = "model_context_window";
const CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const CODEX_MANAGED_MODEL_CATALOG_FILE: &str = "cockpit-model-catalog.json";
const CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE: &str = "cockpit-provider-model-catalog.json";
const CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE: &str =
    "cockpit-local-access-model-catalog.json";
const CODEX_EXPERIMENTAL_MODEL_POLICY_FILE: &str = ".cockpit-experimental-model-catalog-enabled";
const CODEX_EXPERIMENTAL_MODEL_CONFIG_FILE: &str =
    ".cockpit-experimental-model-catalog-config.json";
const CODEX_EXPERIMENTAL_MODEL_PREVIOUS_CATALOG_FILE: &str =
    ".cockpit-experimental-model-catalog-previous.json";
const EXPERIMENTAL_MODEL_CATALOG_CONFIG_VERSION: u32 = 4;
const CODEX_REASONING_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];
const SHIPPED_VISIBLE_CODEX_MODEL_IDS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.3-codex",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex-spark",
];
/// Official DeepSeek Codex setup writes `models.json` and points `model_catalog_json` at it.
/// Extra instances must use their own CODEX_HOME copy, not the default `~/.codex/models.json`.
const DEEPSEEK_OFFICIAL_MODEL_CATALOG_FILE: &str = "models.json";
const CODEX_IMAGE_MODEL_ID: &str = "gpt-image-2";
const CODEX_IMAGEGEN_ACTOR_HEADER: &str = "x-openai-actor-authorization";
const CODEX_IMAGEGEN_ACTOR_HEADER_VALUE: &str = "cockpit-tools";
const CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER: &str = "x-agtools-disable-image-generation";
const CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE: &str = "chat";
/// 本地 API 服务多开实例标识：Codex 请求会带上此 header，便于请求日志区分来源实例。
pub(crate) const CODEX_CLIENT_INSTANCE_ID_HEADER: &str = "x-cockpit-instance-id";
const CODEX_DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const CODEX_COCKPIT_API_BASE_URL: &str = "https://chongcodex.cn/v1";
const CODEX_COCKPIT_API_PROVIDER_ID: &str = "cockpit_api";
const CODEX_OPENAI_PROVIDER_ID: &str = "openai";
const CODEX_RUNTIME_MODEL_PROVIDER_ID: &str = "codex_local_access";
const CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID: &str = "openai_api_key";
const CODEX_DEFAULT_RUNTIME_PROVIDER_NAME: &str = "OpenAI Official";
const CODEX_PROVIDER_WIRE_API: &str = "responses";
const APIKEY_FUN_PROVIDER_BASE_URL: &str = "https://api.apikey.fun/v1";
const DEEPSEEK_API_BASE_URL: &str = "https://api.deepseek.com";
const DEEPSEEK_PROVIDER_ID: &str = "deepseek";
const DEEPSEEK_CODEX_MODELS: &[&str] = &["deepseek-v4-flash", "deepseek-v4-pro"];
const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-v4-flash";
const DEEPSEEK_ACCESS_MODE_GATEWAY: &str = "gateway";
const DEEPSEEK_ACCESS_MODE_DIRECT: &str = "direct";
const DEEPSEEK_ACCESS_MODE_CDP: &str = "cdp";
const DEEPSEEK_CODEX_MODELS_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/deepseek_codex_models.json"
));
const CODEX_CONTEXT_WINDOW_1M_VALUE: i64 = 1_000_000;
const CODEX_AUTO_COMPACT_DEFAULT_LIMIT: i64 = 900_000;
#[cfg(target_os = "macos")]
#[cfg(all(target_os = "macos", not(test)))]
const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
const DISK_FULL_ERROR_CODE: &str = "DISK_FULL";
const CODEX_TOKEN_SOURCE_MANAGED: &str = "managed";
/// ChatGPT Web Session 导入：仅查额，禁止启动/切号/加入 API。
const CODEX_TOKEN_SOURCE_WEB_SESSION: &str = "chatgpt_web_session";
const CODEX_AUTHORIZATION_STATUS_PENDING: &str = "pending";
const CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON: &str =
    "Codex 登录授权缺少 refresh_token，无法自动续期；当前 access_token 已不可用，请重新登录。";
const CODEX_RETIRED_APP_SERVER_PREFLIGHT_REAUTH_REASON: &str =
    "官方 app-server 返回 invalid_refresh_token，账号无法切换，请重新授权";
const CODEX_PROACTIVE_REFRESH_INTERVAL_SECONDS: i64 = 8 * 24 * 60 * 60;
const CODEX_AUTH_PROJECTION_FILE_NAME: &str = ".cockpit_codex_auth.json";
const CODEX_AUTH_PROJECTION_WRITER: &str = "cockpit";
const CODEX_AUTH_PROJECTION_VERSION: u32 = 2;
const CODEX_BATCH_IMPORT_SESSIONS_DIR: &str = "codex_batch_import_sessions";
const CODEX_TOKEN_REFRESH_FILE_LOCK_TIMEOUT_SECONDS: u64 = 120;
const CODEX_TOKEN_REFRESH_FILE_LOCK_STALE_SECONDS: u64 = 10 * 60;
const CODEX_TOKEN_REFRESH_FILE_LOCK_POLL_MS: u64 = 100;
const CODEX_PROFILE_MUTATION_LOCK_DIR: &str = ".cockpit-profile-mutation-locks";
const CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION: u32 = 2;

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CodexManagedAuthProjection {
    version: u32,
    writer: String,
    /// 提供 config.toml / Provider 配置的账号。组合实例中通常是 API Key 账号。
    account_id: String,
    email: String,
    token_generation: u64,
    /// 实际写入 auth.json/keychain、拥有 refresh_token 轮换链的 OAuth 账号。
    /// 历史投影缺少该字段时，普通账号回退到 `account_id`；组合投影可根据
    /// auth.json/keychain 身份在首次同步时自动补齐。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_token_generation: Option<u64>,
    written_at: i64,
}

fn is_auth_mode_apikey(value: Option<&str>) -> bool {
    matches!(
        value.map(|item| item.trim().to_ascii_lowercase()),
        Some(mode) if mode == API_KEY_AUTH_MODE
    )
}

fn normalize_api_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_api_base_url(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.trim_end_matches('/').to_string())
}

fn normalize_api_base_url_for_match(raw: Option<&str>) -> Option<String> {
    let parsed = reqwest::Url::parse(raw?.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .map(|value| format!(":{}", value))
        .unwrap_or_default();
    let path = parsed.path().trim_end_matches('/');
    Some(format!("{}://{}{}{}", parsed.scheme(), host, port, path).to_ascii_lowercase())
}

fn is_cockpit_api_base_url(raw: Option<&str>) -> bool {
    let Some(actual) = normalize_api_base_url_for_match(raw) else {
        return false;
    };
    let Some(expected) = normalize_api_base_url_for_match(Some(CODEX_COCKPIT_API_BASE_URL)) else {
        return false;
    };
    actual == expected
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApiProviderConfig {
    mode: CodexApiProviderMode,
    base_url: Option<String>,
    provider_id: Option<String>,
    provider_name: Option<String>,
}

fn is_default_openai_base_url(raw: &str) -> bool {
    raw.trim()
        .eq_ignore_ascii_case(CODEX_DEFAULT_OPENAI_BASE_URL)
}

fn normalize_api_provider_name(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn sanitize_api_provider_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = String::new();
    let mut prev_separator = false;
    for ch in trimmed.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            prev_separator = false;
            ch.to_ascii_lowercase()
        } else if ch == '-' || ch == '_' {
            if prev_separator {
                continue;
            }
            prev_separator = true;
            ch
        } else {
            if prev_separator {
                continue;
            }
            prev_separator = true;
            '_'
        };
        normalized.push(mapped);
    }

    let mut normalized = normalized
        .trim_matches(|ch| ch == '_' || ch == '-')
        .to_string();
    if normalized.is_empty() {
        return None;
    }
    let starts_with_alpha = normalized
        .chars()
        .next()
        .map(|ch| ch.is_ascii_alphabetic())
        .unwrap_or(false);
    if !starts_with_alpha || normalized == CODEX_OPENAI_PROVIDER_ID {
        normalized = format!("provider_{}", normalized);
    }
    Some(normalized)
}

fn derive_provider_name_from_base_url(base_url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(base_url).ok()?;
    let host = parsed.host_str()?.trim().trim_start_matches("www.");
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn derive_api_provider_id(
    base_url: &str,
    api_provider_id: Option<&str>,
    api_provider_name: Option<&str>,
) -> Option<String> {
    sanitize_api_provider_id(api_provider_id.unwrap_or_default())
        .or_else(|| sanitize_api_provider_id(api_provider_name.unwrap_or_default()))
        .or_else(|| {
            derive_provider_name_from_base_url(base_url)
                .and_then(|name| sanitize_api_provider_id(name.as_str()))
        })
}

fn resolve_api_provider_config(
    api_base_url: Option<&str>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<&str>,
    api_provider_name: Option<&str>,
) -> Result<ApiProviderConfig, String> {
    let normalized_base_url = normalize_api_base_url(api_base_url);
    let mode = api_provider_mode.unwrap_or_else(|| match normalized_base_url.as_deref() {
        None => CodexApiProviderMode::OpenaiBuiltin,
        Some(base_url) if is_default_openai_base_url(base_url) => {
            CodexApiProviderMode::OpenaiBuiltin
        }
        Some(_) => CodexApiProviderMode::Custom,
    });

    match mode {
        CodexApiProviderMode::OpenaiBuiltin => Ok(ApiProviderConfig {
            mode,
            base_url: normalized_base_url.filter(|base_url| !is_default_openai_base_url(base_url)),
            provider_id: None,
            provider_name: None,
        }),
        CodexApiProviderMode::Custom => {
            let base_url = normalized_base_url.ok_or("自定义供应商缺少 Base URL")?;
            let provider_name = normalize_api_provider_name(api_provider_name)
                .or_else(|| derive_provider_name_from_base_url(&base_url));
            let provider_id =
                derive_api_provider_id(&base_url, api_provider_id, provider_name.as_deref());
            Ok(ApiProviderConfig {
                mode,
                base_url: Some(base_url),
                provider_id,
                provider_name,
            })
        }
    }
}

fn infer_api_provider_config(
    api_base_url: Option<&str>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<&str>,
    api_provider_name: Option<&str>,
) -> ApiProviderConfig {
    resolve_api_provider_config(
        api_base_url,
        api_provider_mode,
        api_provider_id,
        api_provider_name,
    )
    .unwrap_or(ApiProviderConfig {
        mode: CodexApiProviderMode::OpenaiBuiltin,
        base_url: None,
        provider_id: None,
        provider_name: None,
    })
}

fn canonical_openai_base_url_for_match(raw: Option<&str>) -> Option<String> {
    normalize_api_base_url(raw)
        .filter(|base_url| !is_default_openai_base_url(base_url))
        .and_then(|base_url| normalize_api_base_url_for_match(Some(&base_url)))
}

fn should_preserve_account_provider_identity(
    account_provider: &ApiProviderConfig,
    config_provider: &ApiProviderConfig,
    local_base_url: Option<&str>,
) -> bool {
    if config_provider.provider_id.as_deref() == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID) {
        return true;
    }
    if config_provider.provider_id.is_some()
        && config_provider.provider_id.as_deref() != Some(CODEX_OPENAI_PROVIDER_ID)
    {
        return false;
    }

    matches!(
        (
            canonical_openai_base_url_for_match(local_base_url),
            canonical_openai_base_url_for_match(account_provider.base_url.as_deref()),
        ),
        (Some(local), Some(account)) if local == account
    )
}

fn is_http_like_url(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Ok(parsed) = reqwest::Url::parse(trimmed) {
        return matches!(parsed.scheme(), "http" | "https");
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn validate_api_key_credentials(
    api_key: &str,
    api_base_url: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let normalized_key = normalize_api_key(api_key).ok_or("API Key 不能为空")?;
    if is_http_like_url(&normalized_key) {
        return Err("API Key 不能是 URL，请检查是否填反".to_string());
    }

    let normalized_base_url = normalize_api_base_url(api_base_url);
    if let Some(base_url) = normalized_base_url.as_ref() {
        let parsed = reqwest::Url::parse(base_url)
            .map_err(|_| "Base URL 格式无效，请输入完整的 http:// 或 https:// 地址".to_string())?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("Base URL 仅支持 http 或 https 协议".to_string());
        }
        if base_url == &normalized_key {
            return Err("API Key 不能与 Base URL 相同".to_string());
        }
    }

    Ok((normalized_key, normalized_base_url))
}

fn build_api_key_email(api_key: &str) -> String {
    let hash = format!("{:x}", md5::compute(api_key.as_bytes()));
    format!("{}-{}", API_KEY_EMAIL_PREFIX, &hash[..8])
}

fn build_api_key_account_id(api_key: &str) -> String {
    format!("codex_apikey_{:x}", md5::compute(api_key.as_bytes()))
}

fn build_legacy_agent_identity_account_id(account_id: &str) -> String {
    format!(
        "codex_agent_identity_{:x}",
        md5::compute(account_id.trim().as_bytes())
    )
}

fn build_agent_identity_account_id(account_id: &str, chatgpt_user_id: &str) -> String {
    let identity_key = format!("{}\0{}", account_id.trim(), chatgpt_user_id.trim());
    format!(
        "codex_agent_identity_{:x}",
        md5::compute(identity_key.as_bytes())
    )
}

fn is_auth_mode_agent_identity(value: Option<&str>) -> bool {
    value
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("agentIdentity"))
}

fn normalize_agent_identity(
    mut identity: CodexAgentIdentity,
) -> Result<CodexAgentIdentity, String> {
    identity.agent_runtime_id = identity.agent_runtime_id.trim().to_string();
    identity.agent_private_key = identity.agent_private_key.trim().to_string();
    identity.account_id = identity.account_id.trim().to_string();
    identity.chatgpt_user_id = identity.chatgpt_user_id.trim().to_string();
    identity.task_id = normalize_optional_value(identity.task_id);
    identity.email = normalize_optional_value(identity.email);
    identity.plan_type = normalize_optional_value(identity.plan_type);
    if identity.agent_runtime_id.is_empty()
        || identity.agent_private_key.is_empty()
        || identity.account_id.is_empty()
        || identity.chatgpt_user_id.is_empty()
    {
        return Err(
            "Agent Identity 缺少 agent_runtime_id、agent_private_key、account_id 或 chatgpt_user_id"
                .to_string(),
        );
    }
    let private_key = base64::engine::general_purpose::STANDARD
        .decode(identity.agent_private_key.as_bytes())
        .map_err(|_| "Agent Identity agent_private_key 不是有效 Base64".to_string())?;
    ed25519_dalek::SigningKey::from_pkcs8_der(&private_key).map_err(|_| {
        "Agent Identity agent_private_key 不是有效的 PKCS#8 Ed25519 私钥".to_string()
    })?;
    Ok(identity)
}

fn parse_agent_identity_from_value(
    value: &serde_json::Value,
) -> Result<Option<CodexAgentIdentity>, String> {
    let root_auth_mode = value
        .get("auth_mode")
        .or_else(|| value.get("authMode"))
        .and_then(serde_json::Value::as_str);
    let credentials = value.get("credentials");
    let credentials_auth_mode = credentials.and_then(|item| {
        item.get("auth_mode")
            .or_else(|| item.get("authMode"))
            .and_then(serde_json::Value::as_str)
    });
    let nested = value
        .get("agent_identity")
        .or_else(|| value.get("agentIdentity"))
        .or_else(|| credentials.and_then(|item| item.get("agent_identity")))
        .or_else(|| credentials.and_then(|item| item.get("agentIdentity")));
    let credentials_look_like_identity = credentials.is_some_and(|item| {
        item.get("agent_runtime_id")
            .or_else(|| item.get("agentRuntimeId"))
            .is_some()
            && item
                .get("agent_private_key")
                .or_else(|| item.get("agentPrivateKey"))
                .is_some()
    });
    if !is_auth_mode_agent_identity(root_auth_mode)
        && !is_auth_mode_agent_identity(credentials_auth_mode)
        && nested.is_none()
        && !credentials_look_like_identity
    {
        return Ok(None);
    }
    let source = nested
        .or_else(|| {
            credentials.filter(|_| {
                is_auth_mode_agent_identity(root_auth_mode)
                    || is_auth_mode_agent_identity(credentials_auth_mode)
                    || credentials_look_like_identity
            })
        })
        .unwrap_or(value);
    // Match Sub2API's import behavior: extract each credential explicitly instead of
    // deserializing aliases into one field. Real exports can contain both account_id
    // and chatgpt_account_id, which Serde otherwise rejects as a duplicate field.
    let identity = CodexAgentIdentity {
        agent_runtime_id: read_json_string(source, &["agent_runtime_id", "agentRuntimeId"])
            .unwrap_or_default(),
        agent_private_key: read_json_string(source, &["agent_private_key", "agentPrivateKey"])
            .unwrap_or_default(),
        task_id: read_json_string(source, &["task_id", "taskId"]),
        account_id: read_json_string(
            source,
            &[
                "account_id",
                "accountId",
                "chatgpt_account_id",
                "chatgptAccountId",
            ],
        )
        .unwrap_or_default(),
        chatgpt_user_id: read_json_string(source, &["chatgpt_user_id", "chatgptUserId"])
            .unwrap_or_default(),
        email: read_json_string(source, &["email"]),
        plan_type: read_json_string(source, &["plan_type", "planType"]),
        chatgpt_account_is_fedramp: read_json_bool(
            source,
            &["chatgpt_account_is_fedramp", "chatgptAccountIsFedramp"],
        )
        .unwrap_or(false),
    };
    normalize_agent_identity(identity).map(Some)
}

fn normalize_api_model_catalog(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || !seen.insert(model.to_ascii_lowercase()) {
            continue;
        }
        values.push(model.to_string());
    }
    values
}

fn normalize_api_wire_api(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| item == "responses" || item == "chat_completions")
}

fn is_apikey_fun_base_url(raw: Option<&str>) -> bool {
    let Some(actual) = normalize_api_base_url_for_match(raw) else {
        return false;
    };
    let Some(expected) = normalize_api_base_url_for_match(Some(APIKEY_FUN_PROVIDER_BASE_URL))
    else {
        return false;
    };
    actual == expected
}

fn migrate_apikey_fun_wire_api(account: &mut CodexAccount) -> bool {
    if !account.is_api_key_auth() || !is_apikey_fun_base_url(account.api_base_url.as_deref()) {
        return false;
    }
    if account.api_wire_api.as_deref() != Some("chat_completions") {
        return false;
    }
    account.api_wire_api = Some("responses".to_string());
    true
}

fn is_deepseek_account(account: &CodexAccount) -> bool {
    account
        .api_provider_id
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(DEEPSEEK_PROVIDER_ID))
        || account
            .api_base_url
            .as_deref()
            .and_then(|value| reqwest::Url::parse(value.trim()).ok())
            .and_then(|url| url.host_str().map(str::to_string))
            .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
}

fn deepseek_official_model_catalog() -> Vec<String> {
    DEEPSEEK_CODEX_MODELS
        .iter()
        .map(|model| model.to_string())
        .collect()
}

fn is_deepseek_responses_account(account: &CodexAccount) -> bool {
    is_deepseek_account(account)
        && account
            .api_wire_api
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(CODEX_PROVIDER_WIRE_API)
            .eq_ignore_ascii_case(CODEX_PROVIDER_WIRE_API)
}

/// Normalize DeepSeek API-key accounts without locking users out of Chat Completions.
/// - Missing wire_api defaults to Responses (official Codex path).
/// - Explicit `chat_completions` is preserved.
/// - Responses mode writes official catalog slugs and talks to api.deepseek.com directly.
fn normalize_deepseek_account(account: &mut CodexAccount) -> bool {
    if !account.is_api_key_auth() || !is_deepseek_account(account) {
        return false;
    }

    let mut changed = false;
    if account.api_base_url.as_deref() != Some(DEEPSEEK_API_BASE_URL) {
        account.api_base_url = Some(DEEPSEEK_API_BASE_URL.to_string());
        changed = true;
    }
    if account.api_provider_mode != CodexApiProviderMode::Custom {
        account.api_provider_mode = CodexApiProviderMode::Custom;
        changed = true;
    }
    if account.api_provider_id.as_deref() != Some(DEEPSEEK_PROVIDER_ID) {
        account.api_provider_id = Some(DEEPSEEK_PROVIDER_ID.to_string());
        changed = true;
    }
    if account.api_provider_name.as_deref() != Some("DeepSeek") {
        account.api_provider_name = Some("DeepSeek".to_string());
        changed = true;
    }

    let wire = account
        .api_wire_api
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    match wire.as_deref() {
        None => {
            account.api_wire_api = Some(CODEX_PROVIDER_WIRE_API.to_string());
            changed = true;
        }
        Some("chat_completions") => {
            // Keep Chat Completions when the user explicitly chose it.
        }
        Some("responses") => {}
        Some(other) => {
            // Unknown values fall back to official Responses.
            if other != CODEX_PROVIDER_WIRE_API {
                account.api_wire_api = Some(CODEX_PROVIDER_WIRE_API.to_string());
                changed = true;
            }
        }
    }

    if is_deepseek_responses_account(account) {
        let model_catalog = deepseek_official_model_catalog();
        if account.api_model_catalog != model_catalog {
            account.api_model_catalog = model_catalog;
            changed = true;
        }
        if !account.api_sync_model_catalog_to_codex {
            account.api_sync_model_catalog_to_codex = true;
            changed = true;
        }
        if account.api_supports_websockets {
            account.api_supports_websockets = false;
            changed = true;
        }
        if account.api_supports_vision {
            account.api_supports_vision = false;
            changed = true;
        }
        if !account.api_model_vision_support.is_empty() {
            account.api_model_vision_support.clear();
            changed = true;
        }
        if account.api_vision_routing_model.is_some() {
            account.api_vision_routing_model = None;
            changed = true;
        }
        if account.api_model_mappings.is_empty() {
            account.api_model_mappings = default_deepseek_api_model_mappings();
            changed = true;
        }
    }

    let access_mode = if is_deepseek_responses_account(account) {
        normalize_deepseek_instance_access_mode(account.api_instance_access_mode.as_deref())
    } else {
        DEEPSEEK_ACCESS_MODE_GATEWAY
    };
    if account.api_instance_access_mode.as_deref() != Some(access_mode) {
        account.api_instance_access_mode = Some(access_mode.to_string());
        changed = true;
    }
    let startup_model = resolve_deepseek_startup_model(account);
    if account.api_startup_model.as_deref() != Some(startup_model.as_str()) {
        account.api_startup_model = Some(startup_model);
        changed = true;
    }

    changed
}

fn normalize_deepseek_instance_access_mode(raw: Option<&str>) -> &'static str {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) if value.eq_ignore_ascii_case(DEEPSEEK_ACCESS_MODE_DIRECT) => {
            DEEPSEEK_ACCESS_MODE_DIRECT
        }
        Some(value) if value.eq_ignore_ascii_case(DEEPSEEK_ACCESS_MODE_CDP) => {
            DEEPSEEK_ACCESS_MODE_CDP
        }
        _ => DEEPSEEK_ACCESS_MODE_GATEWAY,
    }
}

fn is_deepseek_official_runtime_access(account: &CodexAccount) -> bool {
    is_deepseek_responses_account(account)
        && normalize_deepseek_instance_access_mode(account.api_instance_access_mode.as_deref())
            == DEEPSEEK_ACCESS_MODE_DIRECT
}

pub fn account_uses_deepseek_cdp_injection(account: &CodexAccount) -> bool {
    is_deepseek_responses_account(account)
        && normalize_deepseek_instance_access_mode(account.api_instance_access_mode.as_deref())
            == DEEPSEEK_ACCESS_MODE_CDP
}

fn resolve_deepseek_startup_model(account: &CodexAccount) -> String {
    account
        .api_startup_model
        .as_deref()
        .filter(|model| is_official_deepseek_model_slug(model))
        .map(|model| model.trim().to_ascii_lowercase())
        .unwrap_or_else(|| DEEPSEEK_DEFAULT_MODEL.to_string())
}

fn preferred_deepseek_client_model(
    account: &CodexAccount,
    slots: &[crate::modules::codex_local_access::ProviderGatewayModelSlot],
) -> String {
    let startup = resolve_deepseek_startup_model(account);
    slots
        .iter()
        .find(|slot| slot.upstream_model.eq_ignore_ascii_case(&startup))
        .map(|slot| slot.client_model.clone())
        .or_else(|| slots.first().map(|slot| slot.client_model.clone()))
        .unwrap_or_else(|| "gpt-5.5".to_string())
}

pub fn update_account_instance_access(
    account_id: &str,
    access_mode: Option<String>,
    startup_model: Option<String>,
) -> Result<CodexAccount, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if !account.is_api_key_auth() {
        return Err("只有 API Key 账号支持接入方式".to_string());
    }
    if !is_deepseek_account(&account) {
        return Err("仅 DeepSeek 账号支持实例接入方式".to_string());
    }
    let requested_non_gateway = access_mode.as_deref().map(str::trim).is_some_and(|value| {
        value.eq_ignore_ascii_case(DEEPSEEK_ACCESS_MODE_DIRECT)
            || value.eq_ignore_ascii_case(DEEPSEEK_ACCESS_MODE_CDP)
    });
    if requested_non_gateway && !is_deepseek_responses_account(&account) {
        return Err("Chat Completions 只能走本地网关".to_string());
    }
    if access_mode.is_some() {
        account.api_instance_access_mode = access_mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
    }
    if startup_model.is_some() {
        account.api_startup_model = startup_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
    }
    let _ = normalize_deepseek_account(&mut account);
    save_account(&account)?;
    Ok(account)
}

pub fn apply_deepseek_cdp_startup_model(
    account_id: &str,
    model: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let account = update_account_instance_access(
        account_id,
        Some(DEEPSEEK_ACCESS_MODE_CDP.to_string()),
        Some(model.to_string()),
    )?;
    if !account_uses_deepseek_cdp_injection(&account) {
        return Err("当前账号未启用 DeepSeek CDP 注入".to_string());
    }
    write_deepseek_cdp_responses_runtime_to_dir(base_dir, &account)?;
    Ok(account)
}

pub(crate) fn default_deepseek_api_model_mappings() -> Vec<CodexApiModelMapping> {
    vec![
        CodexApiModelMapping {
            client_model: "gpt-5.6-sol".to_string(),
            upstream_model: "deepseek-v4-flash".to_string(),
        },
        CodexApiModelMapping {
            client_model: "gpt-5.6-terra".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
        },
        CodexApiModelMapping {
            client_model: "deepseek-v4-flash".to_string(),
            upstream_model: "deepseek-v4-flash".to_string(),
        },
        CodexApiModelMapping {
            client_model: "deepseek-v4-pro".to_string(),
            upstream_model: "deepseek-v4-pro".to_string(),
        },
    ]
}

pub(crate) fn normalize_api_model_mappings(
    mappings: Vec<CodexApiModelMapping>,
) -> Result<Vec<CodexApiModelMapping>, String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for mapping in mappings {
        let client_model = mapping.client_model.trim().to_string();
        let upstream_model = mapping.upstream_model.trim().to_string();
        if client_model.is_empty() && upstream_model.is_empty() {
            continue;
        }
        if client_model.is_empty() || upstream_model.is_empty() {
            return Err("模型映射需要同时填写请求模型和发送模型".to_string());
        }
        let key = client_model.to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(format!("请求模型 {} 重复", client_model));
        }
        normalized.push(CodexApiModelMapping {
            client_model,
            upstream_model,
        });
    }
    Ok(normalized)
}

pub(crate) fn resolve_account_upstream_model(
    account: &CodexAccount,
    requested_model: &str,
) -> String {
    let requested = requested_model.trim();
    if requested.is_empty() {
        return String::new();
    }
    for mapping in &account.api_model_mappings {
        if mapping.client_model.eq_ignore_ascii_case(requested)
            || mapping.upstream_model.eq_ignore_ascii_case(requested)
        {
            return mapping.upstream_model.clone();
        }
    }
    requested.to_string()
}

pub fn update_account_api_model_mappings(
    account_id: &str,
    mappings: Vec<CodexApiModelMapping>,
    api_model_context_windows: Option<HashMap<String, i64>>,
) -> Result<CodexAccount, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("账号 ID 不能为空".to_string());
    }
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if !account.is_api_key_auth() {
        return Err("只有 API Key 账号支持模型映射".to_string());
    }
    account.api_model_mappings = normalize_api_model_mappings(mappings)?;
    account.api_model_context_windows = normalize_api_model_context_windows(
        api_model_context_windows.unwrap_or_else(|| account.api_model_context_windows.clone()),
        &account.api_model_catalog,
        &account.api_model_mappings,
    );
    save_account(&account)?;
    Ok(account)
}

/// Backward-compatible name used by older call sites / tests.
fn enforce_deepseek_responses_account(account: &mut CodexAccount) -> bool {
    normalize_deepseek_account(account)
}

pub(crate) fn deepseek_official_models_json() -> &'static str {
    DEEPSEEK_CODEX_MODELS_JSON
}

fn selected_deepseek_official_models(selected_models: &[String]) -> Vec<String> {
    let selected: HashSet<String> = selected_models
        .iter()
        .map(|model| model.trim().to_ascii_lowercase())
        .filter(|model| !model.is_empty())
        .collect();
    let prefer_all = selected.is_empty();
    DEEPSEEK_CODEX_MODELS
        .iter()
        .filter(|model| prefer_all || selected.contains(&model.to_ascii_lowercase()))
        .map(|model| model.to_string())
        .collect()
}

fn is_official_deepseek_model_slug(model: &str) -> bool {
    DEEPSEEK_CODEX_MODELS
        .iter()
        .any(|item| item.eq_ignore_ascii_case(model.trim()))
}

fn deepseek_official_model_catalog_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE)
}

fn leftover_deepseek_models_json_path(base_dir: &Path) -> PathBuf {
    base_dir.join(DEEPSEEK_OFFICIAL_MODEL_CATALOG_FILE)
}

fn expand_user_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

fn absolute_path_for_config(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let simplified = absolute.canonicalize().unwrap_or(absolute);
    simplified.to_string_lossy().replace('\\', "/")
}

fn catalog_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn is_deepseek_official_catalog_ref(value: &str, base_dir: &Path) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if matches!(
        trimmed,
        CODEX_MANAGED_MODEL_CATALOG_FILE
            | CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE
            | CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE
    ) || trimmed == DEEPSEEK_OFFICIAL_MODEL_CATALOG_FILE
    {
        return true;
    }
    let configured = expand_user_path(trimmed);
    let name = catalog_file_name(&configured);
    if !name.eq_ignore_ascii_case(CODEX_MANAGED_MODEL_CATALOG_FILE)
        && !name.eq_ignore_ascii_case(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE)
        && !name.eq_ignore_ascii_case(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE)
        && !name.eq_ignore_ascii_case(DEEPSEEK_OFFICIAL_MODEL_CATALOG_FILE)
    {
        return false;
    }
    let configured_abs = if configured.is_absolute() {
        configured
    } else {
        base_dir.join(configured)
    };
    let managed = absolute_path_for_config(&deepseek_official_model_catalog_path(base_dir));
    let legacy_provider =
        absolute_path_for_config(&base_dir.join(CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE));
    let legacy_local =
        absolute_path_for_config(&base_dir.join(CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE));
    let leftover = absolute_path_for_config(&leftover_deepseek_models_json_path(base_dir));
    let configured_abs = absolute_path_for_config(&configured_abs);
    configured_abs.eq_ignore_ascii_case(&managed)
        || configured_abs.eq_ignore_ascii_case(&legacy_provider)
        || configured_abs.eq_ignore_ascii_case(&legacy_local)
        || configured_abs.eq_ignore_ascii_case(&leftover)
}

fn official_catalog_file_looks_like_deepseek(path: &Path) -> bool {
    fs::read_to_string(path).ok().is_some_and(|content| {
        content.contains("deepseek-v4-flash") && content.contains("apply_patch_tool_type")
    })
}

fn remove_leftover_deepseek_models_json(base_dir: &Path) {
    for file_name in [
        DEEPSEEK_OFFICIAL_MODEL_CATALOG_FILE,
        CODEX_LEGACY_PROVIDER_MODEL_CATALOG_FILE,
        CODEX_LEGACY_LOCAL_ACCESS_MODEL_CATALOG_FILE,
    ] {
        let stale = base_dir.join(file_name);
        if stale.exists() {
            if let Err(error) = fs::remove_file(&stale) {
                logger::log_warn(&format!(
                    "[Codex切号] 清理旧 DeepSeek 模型目录失败: path={}, error={}",
                    stale.display(),
                    error
                ));
            }
        }
    }
}

fn write_deepseek_official_model_catalog_file(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<PathBuf, String> {
    let content = crate::modules::codex_local_access::decorate_account_catalog_context_windows(
        &build_deepseek_direct_provider_catalog_json(&account.api_model_catalog)?,
        &[],
        account,
        crate::modules::codex_local_access::read_file_model_context_window(&get_config_toml_path(
            base_dir,
        )),
    )?;
    let content = decorate_managed_model_catalog_for_profile(base_dir, &content)?;
    let catalog_path = deepseek_official_model_catalog_path(base_dir);
    if let Some(parent) = catalog_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建 DeepSeek 官方模型目录失败: path={}, error={}",
                parent.display(),
                e
            )
        })?;
    }
    write_string_atomic(&catalog_path, &content).map_err(|e| {
        format!(
            "写入 DeepSeek 官方模型目录失败: path={}, error={}",
            catalog_path.display(),
            e
        )
    })?;
    remove_leftover_deepseek_models_json(base_dir);
    if let Err(error) = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir) {
        logger::log_warn(&format!(
            "[Codex切号] 清理 Codex 模型缓存失败: path={}, error={}",
            base_dir.display(),
            error
        ));
    }
    Ok(catalog_path)
}

fn apply_deepseek_official_catalog_to_doc(
    doc: &mut Document,
    account: &CodexAccount,
    _catalog_path: &Path,
) {
    let preferred = resolve_deepseek_default_model(account);
    let current_model = doc
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or_default()
        .to_string();
    if !is_official_deepseek_model_slug(&current_model) {
        doc["model"] = value(preferred.as_str());
    }
    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    doc["model_reasoning_effort"] = value("high");
    if doc
        .get("model_reasoning_summary")
        .and_then(|item| item.as_str())
        .is_some()
    {
        let _ = doc.remove("model_reasoning_summary");
    }
}

fn cleanup_deepseek_official_model_catalog_for_dir(base_dir: &Path) -> Result<bool, String> {
    let mut changed = false;
    let catalog_path = deepseek_official_model_catalog_path(base_dir);
    if catalog_path.exists() && official_catalog_file_looks_like_deepseek(&catalog_path) {
        fs::remove_file(&catalog_path).map_err(|e| {
            format!(
                "删除 DeepSeek 官方模型目录失败: path={}, error={}",
                catalog_path.display(),
                e
            )
        })?;
        changed = true;
    }

    let config_path = get_config_toml_path(base_dir);
    if !config_path.exists() {
        return Ok(changed);
    }
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    if existing.trim().is_empty() {
        return Ok(changed);
    }
    let mut doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
        .map_err(|e| format!("解析 config.toml 失败: {}", e))?;
    let points_at_official = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .is_some_and(|value| is_deepseek_official_catalog_ref(value, base_dir));
    if points_at_official {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
            .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
        changed = true;
    }
    Ok(changed)
}

fn resolve_deepseek_default_model(account: &CodexAccount) -> String {
    resolve_deepseek_startup_model(account)
}

fn official_deepseek_catalog_models() -> Result<Vec<serde_json::Value>, String> {
    let catalog: serde_json::Value = serde_json::from_str(DEEPSEEK_CODEX_MODELS_JSON)
        .map_err(|error| format!("解析官方 DeepSeek models.json 失败: {}", error))?;
    catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| "官方 DeepSeek models.json 缺少 models 数组".to_string())
}

fn official_deepseek_display_name(upstream_model: &str) -> String {
    if upstream_model.eq_ignore_ascii_case("deepseek-v4-pro") {
        "DeepSeek-V4-Pro".to_string()
    } else {
        "DeepSeek-V4-Flash".to_string()
    }
}

fn overlay_official_deepseek_fields(
    entry: &mut serde_json::Value,
    official_model: &serde_json::Value,
) {
    let Some(object) = entry.as_object_mut() else {
        return;
    };
    for key in [
        "apply_patch_tool_type",
        "shell_type",
        "web_search_tool_type",
        "base_instructions",
        "default_reasoning_level",
        "supported_reasoning_levels",
        "reasoning_summary_format",
        "default_reasoning_summary",
        "context_window",
        "max_context_window",
        "supports_reasoning_summaries",
        "supports_parallel_tool_calls",
        "input_modalities",
        "prefer_websockets",
        "support_verbosity",
        "default_verbosity",
        "model_messages",
    ] {
        if let Some(value) = official_model.get(key) {
            object.insert(key.to_string(), value.clone());
        }
    }
}

/// Codex picker catalog for native DeepSeek Responses (no instance gateway).
///
/// Each entry keeps three names:
/// - `display_name`: picker label (`DeepSeek-V4-Flash`)
/// - whitelist shell: official Codex client-model template (`gpt-5.6-sol`)
/// - `slug`: upstream ID actually sent to `api.deepseek.com` (`deepseek-v4-flash`)
fn build_deepseek_direct_provider_catalog_json(
    selected_models: &[String],
) -> Result<String, String> {
    let selected = selected_deepseek_official_models(selected_models);
    if selected.is_empty() {
        return Err("DeepSeek 模型目录为空，请至少保留 deepseek-v4-flash".to_string());
    }
    let slots = crate::modules::codex_local_access::allocate_provider_model_slots(&selected);
    let official_models = official_deepseek_catalog_models()?;
    let shell_ids = slots
        .iter()
        .map(|slot| slot.client_model.clone())
        .collect::<Vec<_>>();
    let mut catalog =
        crate::modules::codex_protocol::build_codex_client_models_response(&shell_ids);
    let models = catalog
        .get_mut("models")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| "生成 Codex 模型目录失败".to_string())?;

    for model in models.iter_mut() {
        let Some(shell) = model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let Some(slot) = slots
            .iter()
            .find(|slot| slot.client_model.eq_ignore_ascii_case(&shell))
        else {
            continue;
        };
        if let Some(official_model) = official_models.iter().find(|item| {
            item.get("slug")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|slug| slug.eq_ignore_ascii_case(&slot.upstream_model))
        }) {
            overlay_official_deepseek_fields(model, official_model);
            if let Some(display_name) = official_model
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(object) = model.as_object_mut() {
                    object.insert(
                        "display_name".to_string(),
                        serde_json::Value::String(display_name.to_string()),
                    );
                    object.insert(
                        "description".to_string(),
                        serde_json::Value::String(slot.upstream_model.clone()),
                    );
                }
            }
        } else if let Some(object) = model.as_object_mut() {
            object.insert(
                "display_name".to_string(),
                serde_json::Value::String(official_deepseek_display_name(&slot.upstream_model)),
            );
            object.insert(
                "description".to_string(),
                serde_json::Value::String(slot.upstream_model.clone()),
            );
        }
        if let Some(object) = model.as_object_mut() {
            object.insert(
                "slug".to_string(),
                serde_json::Value::String(slot.upstream_model.clone()),
            );
            object.insert(
                "visibility".to_string(),
                serde_json::Value::String("list".to_string()),
            );
            object.insert(
                "supported_in_api".to_string(),
                serde_json::Value::Bool(true),
            );
        }
    }

    models.retain(|model| {
        model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .is_some_and(is_official_deepseek_model_slug)
    });
    models.sort_by(|left, right| {
        let left_slug = left
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let right_slug = right
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let rank = |slug: &str| -> u8 {
            if slug.eq_ignore_ascii_case(DEEPSEEK_DEFAULT_MODEL) {
                0
            } else if slug.eq_ignore_ascii_case("deepseek-v4-pro") {
                1
            } else {
                2
            }
        };
        rank(left_slug)
            .cmp(&rank(right_slug))
            .then_with(|| left_slug.cmp(right_slug))
    });
    if models.is_empty() {
        return Err("DeepSeek 模型目录为空，请至少保留 deepseek-v4-flash".to_string());
    }

    serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("序列化 DeepSeek 模型目录失败: {}", error))
}

fn build_deepseek_official_model_catalog_json(
    selected_models: &[String],
) -> Result<String, String> {
    let mut catalog: serde_json::Value = serde_json::from_str(DEEPSEEK_CODEX_MODELS_JSON)
        .map_err(|error| format!("解析官方 DeepSeek models.json 失败: {}", error))?;
    let selected: HashSet<String> = selected_deepseek_official_models(selected_models)
        .into_iter()
        .map(|model| model.to_ascii_lowercase())
        .collect();

    let models = catalog
        .get_mut("models")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| "官方 DeepSeek models.json 缺少 models 数组".to_string())?;

    models.retain(|model| {
        let slug = model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        selected.contains(&slug)
    });

    models.sort_by(|left, right| {
        let left_slug = left
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let right_slug = right
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let rank = |slug: &str| -> u8 {
            if slug.eq_ignore_ascii_case(DEEPSEEK_DEFAULT_MODEL) {
                0
            } else if slug.eq_ignore_ascii_case("deepseek-v4-pro") {
                1
            } else {
                2
            }
        };
        rank(left_slug)
            .cmp(&rank(right_slug))
            .then_with(|| left_slug.cmp(right_slug))
    });

    if models.is_empty() {
        return Err("DeepSeek 模型目录为空，请至少保留 deepseek-v4-flash".to_string());
    }

    serde_json::to_string_pretty(&catalog)
        .map_err(|error| format!("序列化官方 DeepSeek models.json 失败: {}", error))
}

fn sync_deepseek_shell_remap_catalog_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    let selected = if account.api_model_catalog.is_empty() {
        deepseek_official_model_catalog()
    } else {
        selected_deepseek_official_models(&account.api_model_catalog)
    };
    let slots = crate::modules::codex_local_access::allocate_provider_model_slots(&selected);
    let content = crate::modules::codex_local_access::decorate_account_catalog_context_windows(
        &crate::modules::codex_local_access::build_official_template_mapped_catalog_json(
            &slots,
            deepseek_official_models_json(),
        )?,
        &slots,
        account,
        crate::modules::codex_local_access::read_file_model_context_window(&get_config_toml_path(
            base_dir,
        )),
    )?;
    let content = decorate_managed_model_catalog_for_profile(base_dir, &content)?;
    let catalog_path = deepseek_official_model_catalog_path(base_dir);
    if let Some(parent) = catalog_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建 DeepSeek 模型目录失败: path={}, error={}",
                parent.display(),
                e
            )
        })?;
    }
    write_string_atomic(&catalog_path, &content).map_err(|e| {
        format!(
            "写入 DeepSeek 模型目录失败: path={}, error={}",
            catalog_path.display(),
            e
        )
    })?;
    cleanup_legacy_managed_model_catalogs(base_dir);
    remove_leftover_deepseek_models_json(base_dir);
    if let Err(error) = crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir) {
        logger::log_warn(&format!(
            "[Codex切号] 清理 Codex 模型缓存失败: path={}, error={}",
            base_dir.display(),
            error
        ));
    }

    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    let preferred_shell = preferred_deepseek_client_model(account, &slots);
    doc["model"] = value(preferred_shell.as_str());
    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    doc["model_reasoning_effort"] = value("high");
    if doc
        .get("model_reasoning_summary")
        .and_then(|item| item.as_str())
        .is_some()
    {
        let _ = doc.remove("model_reasoning_summary");
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 同步 DeepSeek 壳映射目录后刷新 provider 失败: path={}, error={}",
            base_dir.display(),
            err
        ));
    }
    Ok(true)
}

fn sync_deepseek_official_model_catalog_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    let catalog_path = write_deepseek_official_model_catalog_file(base_dir, account)?;
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    apply_deepseek_official_catalog_to_doc(&mut doc, account, &catalog_path);

    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 同步 DeepSeek 官方模型目录后刷新 provider 失败: path={}, error={}",
            base_dir.display(),
            err
        ));
    }
    Ok(true)
}

fn normalize_api_key_websocket_capability(account: &mut CodexAccount) -> bool {
    let normalized = account.is_api_key_auth()
        && account.api_provider_mode == CodexApiProviderMode::Custom
        && account.api_wire_api.as_deref() == Some("responses")
        && account.api_supports_websockets;
    if account.api_supports_websockets == normalized {
        return false;
    }
    account.api_supports_websockets = normalized;
    true
}

fn lookup_api_model_context_window(windows: &HashMap<String, i64>, model: &str) -> Option<i64> {
    let key = model.trim();
    if key.is_empty() {
        return None;
    }
    windows
        .get(key)
        .copied()
        .or_else(|| {
            windows
                .iter()
                .find_map(|(name, window)| name.trim().eq_ignore_ascii_case(key).then_some(*window))
        })
        .filter(|value| *value > 0)
}

pub(crate) fn normalize_api_model_context_windows(
    windows: HashMap<String, i64>,
    catalog: &[String],
    mappings: &[CodexApiModelMapping],
) -> HashMap<String, i64> {
    let mut allowed = Vec::new();
    let mut seen = HashSet::new();
    for model in catalog
        .iter()
        .map(|item| item.as_str())
        .chain(mappings.iter().flat_map(|mapping| {
            [
                mapping.client_model.as_str(),
                mapping.upstream_model.as_str(),
            ]
        }))
    {
        let key = model.trim();
        if key.is_empty() {
            continue;
        }
        let fingerprint = key.to_ascii_lowercase();
        if !seen.insert(fingerprint) {
            continue;
        }
        allowed.push(key.to_string());
    }

    let mut next = HashMap::new();
    if allowed.is_empty() {
        for (name, window) in windows {
            let key = name.trim();
            if !key.is_empty() && window > 0 {
                next.insert(key.to_string(), window);
            }
        }
        return next;
    }

    for model in allowed {
        if let Some(window) = lookup_api_model_context_window(&windows, &model) {
            next.insert(model, window);
        }
    }
    next
}

fn apply_api_key_fields(
    account: &mut CodexAccount,
    api_key: &str,
    provider_config: ApiProviderConfig,
    api_model_catalog: Vec<String>,
    api_sync_model_catalog_to_codex: bool,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
    api_model_context_windows: Option<HashMap<String, i64>>,
) {
    let is_cockpit_api = provider_config
        .provider_id
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case(CODEX_COCKPIT_API_PROVIDER_ID))
        .unwrap_or(false)
        || is_cockpit_api_base_url(provider_config.base_url.as_deref());
    let plan_type = if is_cockpit_api {
        COCKPIT_API_LOGIN_PLAN_TYPE
    } else {
        API_KEY_LOGIN_PLAN_TYPE
    };

    account.auth_mode = CodexAuthMode::Apikey;
    account.agent_identity = None;
    account.openai_api_key = Some(api_key.to_string());
    account.api_base_url = provider_config.base_url;
    account.api_provider_mode = provider_config.mode;
    account.api_provider_id = provider_config.provider_id;
    account.api_provider_name = provider_config.provider_name;
    account.api_model_catalog = normalize_api_model_catalog(api_model_catalog);
    account.api_model_context_windows = normalize_api_model_context_windows(
        api_model_context_windows.unwrap_or_else(|| account.api_model_context_windows.clone()),
        &account.api_model_catalog,
        &account.api_model_mappings,
    );
    account.api_sync_model_catalog_to_codex = api_sync_model_catalog_to_codex;
    account.api_wire_api = normalize_api_wire_api(api_wire_api);
    account.api_supports_websockets = api_supports_websockets;
    let _ = normalize_api_key_websocket_capability(account);
    account.api_supports_vision = api_supports_vision;
    account.api_model_vision_support = normalize_api_model_vision_support(api_model_vision_support);
    account.api_vision_routing_model = normalize_optional_value(api_vision_routing_model);
    account.email = build_api_key_email(api_key);
    if is_cockpit_api && normalize_optional_ref(account.account_name.as_deref()).is_none() {
        account.account_name = Some(COCKPIT_API_DEFAULT_ACCOUNT_NAME.to_string());
    }
    account.plan_type = Some(plan_type.to_string());
    account.tokens = CodexTokens {
        id_token: String::new(),
        access_token: String::new(),
        refresh_token: None,
    };
    account.user_id = None;
    account.subscription_active_until = None;
    account.account_id = None;
    account.organization_id = None;
    account.account_structure = None;
    account.quota = None;
    account.quota_error = None;
}

fn normalize_api_model_vision_support(
    values: std::collections::HashMap<String, bool>,
) -> std::collections::HashMap<String, bool> {
    values
        .into_iter()
        .filter_map(|(model, supports)| {
            let model = model.trim().to_lowercase();
            if model.is_empty() {
                None
            } else {
                Some((model, supports))
            }
        })
        .collect()
}

fn extract_api_key_from_auth_file(auth_file: &CodexAuthFile) -> Option<String> {
    auth_file
        .openai_api_key
        .as_ref()
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_api_key(value))
}

fn extract_api_base_url_from_auth_file(auth_file: &CodexAuthFile) -> Option<String> {
    normalize_api_base_url(auth_file.base_url.as_deref())
}

fn extract_api_base_url_from_json_value(value: &serde_json::Value) -> Option<String> {
    normalize_api_base_url(
        value
            .get("base_url")
            .and_then(|v| v.as_str())
            .or_else(|| value.get("api_base_url").and_then(|v| v.as_str()))
            .or_else(|| value.get("apiBaseUrl").and_then(|v| v.as_str())),
    )
}

fn normalize_optional_json_str(value: Option<&serde_json::Value>) -> Option<String> {
    normalize_optional_ref(value.and_then(|item| item.as_str()))
}

fn normalize_optional_json_scalar(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|item| {
        if let Some(raw) = item.as_str() {
            return normalize_optional_ref(Some(raw));
        }
        if let Some(raw) = item.as_i64() {
            return Some(raw.to_string());
        }
        if let Some(raw) = item.as_u64() {
            return Some(raw.to_string());
        }
        if let Some(raw) = item.as_f64() {
            if raw.is_finite() {
                return Some(raw.trunc().to_string());
            }
        }
        None
    })
}

fn extract_account_record_field(
    record: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        if let Some(value) = normalize_optional_json_str(record.get(*key)) {
            return Some(value);
        }
    }
    None
}

fn collect_account_records(payload: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut records = Vec::new();

    if let Some(accounts_value) = payload.get("accounts") {
        if let Some(array) = accounts_value.as_array() {
            for item in array {
                if item.is_object() {
                    records.push(item.clone());
                }
            }
        } else if let Some(object) = accounts_value.as_object() {
            for value in object.values() {
                if value.is_object() {
                    records.push(value.clone());
                }
            }
        }
    }

    if records.is_empty() {
        if let Some(array) = payload.as_array() {
            for item in array {
                if item.is_object() {
                    records.push(item.clone());
                }
            }
        }
    }

    records
}

fn parse_account_profile_from_check_response(
    payload: &serde_json::Value,
    account: &CodexAccount,
) -> (Option<String>, Option<String>, Option<String>) {
    let records = collect_account_records(payload);
    if records.is_empty() {
        return (None, None, None);
    }

    let ordering_first_id = payload
        .get("account_ordering")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|value| value.as_str())
        .and_then(|value| normalize_optional_ref(Some(value)));
    let expected_account_id =
        extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
            .or_else(|| normalize_optional_ref(account.account_id.as_deref()));
    let expected_org_id = normalize_optional_ref(account.organization_id.as_deref());

    let mut selected_record: Option<serde_json::Value> = None;

    if let Some(expected_id) = expected_account_id.as_deref() {
        selected_record = records
            .iter()
            .find(|item| {
                let Some(record) = item.as_object() else {
                    return false;
                };
                let candidate_id = extract_account_record_field(
                    record,
                    &["id", "account_id", "chatgpt_account_id", "workspace_id"],
                );
                normalize_optional_ref(candidate_id.as_deref()) == Some(expected_id.to_string())
            })
            .cloned();
    }

    if selected_record.is_none() {
        if let Some(ordering_id) = ordering_first_id.as_deref() {
            selected_record = records
                .iter()
                .find(|item| {
                    let Some(record) = item.as_object() else {
                        return false;
                    };
                    let candidate_id = extract_account_record_field(
                        record,
                        &["id", "account_id", "chatgpt_account_id", "workspace_id"],
                    );
                    normalize_optional_ref(candidate_id.as_deref()) == Some(ordering_id.to_string())
                })
                .cloned();
        }
    }

    if selected_record.is_none() {
        if let Some(org_id) = expected_org_id.as_deref() {
            selected_record = records
                .iter()
                .find(|item| {
                    let Some(record) = item.as_object() else {
                        return false;
                    };
                    let candidate_org = extract_account_record_field(
                        record,
                        &["organization_id", "org_id", "workspace_id"],
                    );
                    normalize_optional_ref(candidate_org.as_deref()) == Some(org_id.to_string())
                })
                .cloned();
        }
    }

    let selected = selected_record.unwrap_or_else(|| records[0].clone());
    let Some(record) = selected.as_object() else {
        return (None, None, None);
    };

    let account_name = extract_account_record_field(
        record,
        &[
            "name",
            "display_name",
            "account_name",
            "organization_name",
            "workspace_name",
            "title",
        ],
    );
    let account_structure = extract_account_record_field(
        record,
        &[
            "structure",
            "account_structure",
            "kind",
            "type",
            "account_type",
        ],
    );
    let account_id = extract_account_record_field(
        record,
        &["id", "account_id", "chatgpt_account_id", "workspace_id"],
    );

    (account_name, account_structure, account_id)
}
