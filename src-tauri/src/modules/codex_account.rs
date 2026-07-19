use crate::models::codex::{
    CodexAccount, CodexAccountIndex, CodexAccountSummary, CodexApiProviderMode, CodexAppSpeed,
    CodexAuthFile, CodexAuthMode, CodexAuthTokens, CodexJwtPayload, CodexQuickConfig, CodexTokens,
};
use crate::modules::{account, codex_oauth, logger};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
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
static CODEX_AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static CODEX_BATCH_IMPORT_COUNTER: AtomicU64 = AtomicU64::new(1);
static CODEX_BATCH_IMPORT_SESSIONS: std::sync::LazyLock<
    Mutex<HashMap<String, CodexBatchImportSession>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
const CODEX_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const ACCOUNT_CHECK_URL: &str = "https://chatgpt.com/backend-api/wham/accounts/check";
const API_KEY_LOGIN_PLAN_TYPE: &str = "API_KEY";
const COCKPIT_API_LOGIN_PLAN_TYPE: &str = "Cockpit Api";
const COCKPIT_API_DEFAULT_ACCOUNT_NAME: &str = "Codex API";
const API_KEY_EMAIL_PREFIX: &str = "api-key";
const API_KEY_AUTH_MODE: &str = "apikey";
const CODEX_ACCOUNT_GROUPS_FILE: &str = "codex_account_groups.json";
const CODEX_CONFIG_FILE_NAME: &str = "config.toml";
const CODEX_CONFIG_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CONFIG_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_CONFIG_MODEL_PROVIDERS_KEY: &str = "model_providers";
const CODEX_CONFIG_MODEL_CATALOG_JSON_KEY: &str = "model_catalog_json";
const CODEX_CONFIG_EXPERIMENTAL_BEARER_TOKEN_KEY: &str = "experimental_bearer_token";
const CODEX_CONFIG_HTTP_HEADERS_KEY: &str = "http_headers";
const CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY: &str = "model_context_window";
const CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const CODEX_MANAGED_MODEL_CATALOG_FILE: &str = "cockpit-provider-model-catalog.json";
const CODEX_AUTO_REVIEW_MODEL_ID: &str = "codex-auto-review";
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
const CODEX_CONTEXT_WINDOW_1M_VALUE: i64 = 1_000_000;
const CODEX_AUTO_COMPACT_DEFAULT_LIMIT: i64 = 900_000;
#[cfg(target_os = "macos")]
#[cfg(all(target_os = "macos", not(test)))]
const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
const DISK_FULL_ERROR_CODE: &str = "DISK_FULL";
const CODEX_TOKEN_SOURCE_MANAGED: &str = "managed";
const CODEX_AUTHORIZATION_STATUS_PENDING: &str = "pending";
const CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON: &str =
    "Codex 登录授权缺少 refresh_token，无法自动续期；当前 access_token 已不可用，请重新登录。";
const CODEX_PROACTIVE_REFRESH_INTERVAL_SECONDS: i64 = 8 * 24 * 60 * 60;
const CODEX_AUTH_PROJECTION_FILE_NAME: &str = ".cockpit_codex_auth.json";
const CODEX_AUTH_PROJECTION_WRITER: &str = "cockpit";
const CODEX_BATCH_IMPORT_SESSIONS_DIR: &str = "codex_batch_import_sessions";
const CODEX_TOKEN_REFRESH_FILE_LOCK_TIMEOUT_SECONDS: u64 = 120;
const CODEX_TOKEN_REFRESH_FILE_LOCK_STALE_SECONDS: u64 = 10 * 60;
const CODEX_TOKEN_REFRESH_FILE_LOCK_POLL_MS: u64 = 100;
const CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION: u32 = 2;

#[allow(dead_code)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CodexManagedAuthProjection {
    version: u32,
    writer: String,
    account_id: String,
    email: String,
    token_generation: u64,
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
    account.openai_api_key = Some(api_key.to_string());
    account.api_base_url = provider_config.base_url;
    account.api_provider_mode = provider_config.mode;
    account.api_provider_id = provider_config.provider_id;
    account.api_provider_name = provider_config.provider_name;
    account.api_model_catalog = normalize_api_model_catalog(api_model_catalog);
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
    let expected_account_id = normalize_optional_ref(account.account_id.as_deref())
        .or_else(|| extract_chatgpt_account_id_from_access_token(&account.tokens.access_token));
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

async fn fetch_remote_account_profile(
    account: &CodexAccount,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    if account.is_api_key_auth() {
        return Err("API Key 账号不支持刷新远端资料".to_string());
    }

    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", account.tokens.access_token))
            .map_err(|e| format!("构建 Authorization 头失败: {}", e))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

    if let Some(account_id) = normalize_optional_ref(account.account_id.as_deref())
        .or_else(|| extract_chatgpt_account_id_from_access_token(&account.tokens.access_token))
    {
        headers.insert(
            "ChatGPT-Account-Id",
            HeaderValue::from_str(&account_id)
                .map_err(|e| format!("构建 ChatGPT-Account-Id 头失败: {}", e))?,
        );
    }

    let response = client
        .get(ACCOUNT_CHECK_URL)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求账号信息失败: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取账号信息响应失败: {}", e))?;

    if !status.is_success() {
        return Err(format!(
            "账号信息接口返回错误 {}，body_len={}",
            status,
            body.len()
        ));
    }

    let payload: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("账号信息 JSON 解析失败: {}", e))?;
    Ok(parse_account_profile_from_check_response(&payload, account))
}

/// 获取 Codex 数据目录
pub fn get_codex_home() -> PathBuf {
    if let Some(from_env) = resolve_codex_home_from_env() {
        return from_env;
    }
    dirs::home_dir().expect("无法获取用户主目录").join(".codex")
}

fn resolve_codex_home_from_env() -> Option<PathBuf> {
    let raw = std::env::var("CODEX_HOME").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 兼容用户使用 setx / shell 时可能包裹的引号
    let unquoted = trimmed.trim_matches('"').trim_matches('\'').trim();
    if unquoted.is_empty() {
        return None;
    }

    Some(PathBuf::from(unquoted))
}

/// 获取官方 auth.json 路径
pub fn get_auth_json_path() -> PathBuf {
    get_codex_home().join("auth.json")
}

fn get_config_toml_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_CONFIG_FILE_NAME)
}

fn read_top_level_int_from_doc(doc: &Document, key: &str) -> Option<i64> {
    doc.get(key).and_then(|item| item.as_integer())
}

pub fn read_quick_config_from_config_toml(base_dir: &Path) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let content = fs::read_to_string(config_path).unwrap_or_default();
    if content.trim().is_empty() {
        return Ok(CodexQuickConfig {
            context_window_1m: false,
            auto_compact_token_limit: CODEX_AUTO_COMPACT_DEFAULT_LIMIT,
            detected_model_context_window: None,
            detected_auto_compact_token_limit: None,
        });
    }

    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
        .map_err(|e| format!("解析 config.toml 失败: {}", e))?;
    let detected_model_context_window =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
    let detected_auto_compact_token_limit =
        read_top_level_int_from_doc(&doc, CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY)
            .filter(|value| *value > 0);

    Ok(CodexQuickConfig {
        context_window_1m: detected_model_context_window == Some(CODEX_CONTEXT_WINDOW_1M_VALUE),
        auto_compact_token_limit: detected_auto_compact_token_limit
            .unwrap_or(CODEX_AUTO_COMPACT_DEFAULT_LIMIT),
        detected_model_context_window,
        detected_auto_compact_token_limit,
    })
}

pub fn load_current_quick_config() -> Result<CodexQuickConfig, String> {
    read_quick_config_from_config_toml(&get_codex_home())
}

fn write_quick_config_to_config_toml(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();

    if existing.trim().is_empty()
        && model_context_window.is_none()
        && auto_compact_token_limit.is_none()
    {
        return read_quick_config_from_config_toml(base_dir);
    }

    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    if let Some(context_window) = model_context_window {
        if context_window <= 0 {
            return Err("上下文窗口必须大于 0".to_string());
        }
        doc[CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY] = value(context_window);
    } else {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
    }

    if let Some(compact_limit) = auto_compact_token_limit {
        if compact_limit <= 0 {
            return Err("自动压缩阈值必须大于 0".to_string());
        }
        doc[CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY] = value(compact_limit);
    } else {
        let _ = doc.remove(CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;

    read_quick_config_from_config_toml(base_dir)
}

pub fn save_current_quick_config(
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    save_quick_config_for_base_dir(
        &get_codex_home(),
        model_context_window,
        auto_compact_token_limit,
    )
}

pub fn save_quick_config_for_base_dir(
    base_dir: &Path,
    model_context_window: Option<i64>,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml(base_dir, model_context_window, auto_compact_token_limit)
}

fn read_api_provider_from_config_toml(base_dir: &Path) -> ApiProviderConfig {
    let config_path = get_config_toml_path(base_dir);
    let content = match fs::read_to_string(config_path) {
        Ok(content) if !content.trim().is_empty() => content,
        _ => {
            return ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            };
        }
    };

    let doc = match crate::modules::codex_config_format::read_codex_config_doc_from_str(&content) {
        Ok(doc) => doc,
        Err(_) => {
            return ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            };
        }
    };

    let openai_base_url = normalize_api_base_url(
        doc.get(CODEX_CONFIG_OPENAI_BASE_URL_KEY)
            .and_then(|item| item.as_str()),
    );
    let model_provider = normalize_optional_ref(
        doc.get(CODEX_CONFIG_MODEL_PROVIDER_KEY)
            .and_then(|item| item.as_str()),
    );

    if let Some(provider_id) = model_provider {
        if provider_id == CODEX_OPENAI_PROVIDER_ID {
            return infer_api_provider_config(
                openai_base_url.as_deref(),
                Some(CodexApiProviderMode::OpenaiBuiltin),
                None,
                None,
            );
        }
        let provider_base_url = doc
            .get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
            .and_then(|item| item.get(provider_id.as_str()))
            .and_then(|item| item.get("base_url"))
            .and_then(|item| item.as_str())
            .and_then(|raw| normalize_api_base_url(Some(raw)));
        let provider_name = normalize_api_provider_name(
            doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
                .and_then(|item| item.get(provider_id.as_str()))
                .and_then(|item| item.get("name"))
                .and_then(|item| item.as_str()),
        );

        return infer_api_provider_config(
            provider_base_url.as_deref(),
            Some(CodexApiProviderMode::Custom),
            Some(provider_id.as_str()),
            provider_name.as_deref(),
        );
    }

    infer_api_provider_config(
        openai_base_url.as_deref(),
        Some(CodexApiProviderMode::OpenaiBuiltin),
        None,
        None,
    )
}

fn write_api_provider_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
) -> Result<(), String> {
    let config_path = get_config_toml_path(base_dir);
    let normalized = provider_config.base_url.clone();

    if !config_path.exists() && normalized.is_none() {
        return Ok(());
    }

    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    match provider_config.mode {
        CodexApiProviderMode::OpenaiBuiltin => {
            remove_managed_model_catalog_from_doc(&mut doc);
            let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDER_KEY);
            remove_managed_api_key_model_providers_from_doc(&mut doc);
            #[cfg(target_os = "windows")]
            {
                write_windows_builtin_openai_provider_to_doc(&mut doc, normalized.as_deref())?;
            }
            #[cfg(not(target_os = "windows"))]
            match normalized.as_deref() {
                Some(base_url) => {
                    doc[CODEX_CONFIG_OPENAI_BASE_URL_KEY] = value(base_url);
                }
                None => {
                    let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
                }
            }
        }
        CodexApiProviderMode::Custom => {
            remove_managed_model_catalog_from_doc(&mut doc);
            let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
            let provider_id = provider_config
                .provider_id
                .as_deref()
                .ok_or("自定义供应商缺少 provider_id")?;
            let provider_name = provider_config
                .provider_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(provider_id);
            let base_url = normalized.as_deref().ok_or("自定义供应商缺少 Base URL")?;

            doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(provider_id);
            if doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY).is_none() {
                doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY] = toml_edit::table();
            }
            let model_providers = doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY]
                .as_table_mut()
                .ok_or("config.toml 中 model_providers 不是合法表结构")?;
            if !model_providers.contains_key(provider_id) {
                model_providers[provider_id] = toml_edit::table();
            }
            let provider_table = model_providers[provider_id]
                .as_table_mut()
                .ok_or("config.toml 中目标 provider 不是合法表结构")?;
            provider_table["name"] = value(provider_name);
            provider_table["base_url"] = value(base_url);
            provider_table["wire_api"] = value(CODEX_PROVIDER_WIRE_API);
            provider_table["requires_openai_auth"] = value(false);
            provider_table["supports_websockets"] = value(false);
        }
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))
}

fn remove_managed_model_catalog_from_doc(doc: &mut Document) -> bool {
    let uses_managed_catalog = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(CODEX_MANAGED_MODEL_CATALOG_FILE);
    if uses_managed_catalog {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY);
        return true;
    }
    false
}

fn account_syncs_model_catalog_to_codex(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account.api_sync_model_catalog_to_codex
        && account.api_provider_mode == CodexApiProviderMode::Custom
        && account
            .api_wire_api
            .as_deref()
            .map(str::trim)
            .unwrap_or(CODEX_PROVIDER_WIRE_API)
            .eq_ignore_ascii_case(CODEX_PROVIDER_WIRE_API)
        && !account.api_model_catalog.is_empty()
}

fn sync_api_key_model_catalog_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<bool, String> {
    if !account_syncs_model_catalog_to_codex(account) {
        return Ok(false);
    }

    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };
    if let Some(configured_catalog) = doc
        .get(CODEX_CONFIG_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if configured_catalog != CODEX_MANAGED_MODEL_CATALOG_FILE {
            return Ok(false);
        }
    }

    let mut model_ids = normalize_api_model_catalog(account.api_model_catalog.clone());
    if !model_ids
        .iter()
        .any(|model| model.eq_ignore_ascii_case(CODEX_AUTO_REVIEW_MODEL_ID))
    {
        model_ids.push(CODEX_AUTO_REVIEW_MODEL_ID.to_string());
    }
    let selected_model_is_available = doc
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(|selected_model| {
            model_ids
                .iter()
                .any(|model| model.eq_ignore_ascii_case(selected_model))
        })
        .unwrap_or(false);
    if !selected_model_is_available {
        if let Some(default_model) = model_ids.first() {
            doc["model"] = value(default_model.as_str());
        }
    }
    let catalog = crate::modules::codex_protocol::build_codex_client_models_response(&model_ids);
    let content = serde_json::to_string_pretty(&catalog)
        .map_err(|e| format!("生成 Codex 模型目录失败: {}", e))?;
    let catalog_path = base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    write_string_atomic(&catalog_path, &content).map_err(|e| {
        format!(
            "写入 Codex 模型目录失败: path={}, error={}",
            catalog_path.display(),
            e
        )
    })?;

    doc[CODEX_CONFIG_MODEL_CATALOG_JSON_KEY] = value(CODEX_MANAGED_MODEL_CATALOG_FILE);
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
    // 模型目录变更后同步生图 header：无 gpt-image-2 时清掉残留 actor，避免客户端误开生图卡住。
    if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 同步模型目录后刷新 provider 生图配置失败: path={}, error={}",
            base_dir.display(),
            err
        ));
    }
    Ok(true)
}

fn sync_or_cleanup_managed_model_catalog_for_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if account_syncs_model_catalog_to_codex(account) {
        let _ = sync_api_key_model_catalog_to_dir(base_dir, account)?;
    } else {
        let _ = cleanup_managed_model_catalog_for_dir(base_dir)?;
        // 未同步受管目录时仍按账号 catalog 收敛 header（无 image 则清）。
        if let Err(err) = refresh_api_key_provider_projection_in_dir(base_dir, account) {
            logger::log_warn(&format!(
                "[Codex切号] 清理模型目录后刷新 provider 生图配置失败: path={}, error={}",
                base_dir.display(),
                err
            ));
        }
    }
    Ok(())
}

fn cleanup_managed_model_catalog_for_dir(base_dir: &Path) -> Result<bool, String> {
    let mut changed = false;
    let catalog_path = base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE);
    if catalog_path.exists() {
        fs::remove_file(&catalog_path).map_err(|e| {
            format!(
                "删除 Codex 模型目录失败: path={}, error={}",
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
    if remove_managed_model_catalog_from_doc(&mut doc) {
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
            .map_err(|e| format!("写入 config.toml 失败: {}", e))?;
        changed = true;
    }
    Ok(changed)
}

fn collect_managed_api_key_provider_ids() -> HashSet<String> {
    let mut ids = HashSet::from([
        CODEX_RUNTIME_MODEL_PROVIDER_ID.to_string(),
        CODEX_COCKPIT_API_PROVIDER_ID.to_string(),
        CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID.to_string(),
    ]);

    for account in list_accounts() {
        if !account.is_api_key_auth() {
            continue;
        }
        if let Some(provider_id) = normalize_optional_ref(account.api_provider_id.as_deref()) {
            ids.insert(provider_id);
        }
    }

    ids
}

fn remove_managed_api_key_model_providers_from_doc(doc: &mut Document) {
    let managed_provider_ids = collect_managed_api_key_provider_ids();
    let should_remove_model_providers = doc
        .get_mut(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            for provider_id in &managed_provider_ids {
                let _ = model_providers.remove(provider_id.as_str());
            }
            model_providers.is_empty()
        })
        .unwrap_or(false);

    if should_remove_model_providers {
        let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDERS_KEY);
    }
}

#[cfg(target_os = "windows")]
fn write_windows_builtin_openai_provider_to_doc(
    doc: &mut Document,
    base_url: Option<&str>,
) -> Result<(), String> {
    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(CODEX_OPENAI_PROVIDER_ID);
    match base_url {
        Some(base_url) if base_url != CODEX_DEFAULT_OPENAI_BASE_URL => {
            doc[CODEX_CONFIG_OPENAI_BASE_URL_KEY] = value(base_url);
        }
        _ => {
            let _ = doc.remove(CODEX_CONFIG_OPENAI_BASE_URL_KEY);
        }
    }
    let should_remove_model_providers = doc
        .get_mut(CODEX_CONFIG_MODEL_PROVIDERS_KEY)
        .and_then(|item| item.as_table_mut())
        .map(|model_providers| {
            let _ = model_providers.remove(CODEX_OPENAI_PROVIDER_ID);
            model_providers.is_empty()
        })
        .unwrap_or(false);
    if should_remove_model_providers {
        let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDERS_KEY);
    }
    Ok(())
}

fn api_key_account_supports_image_generation(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account
            .api_model_catalog
            .iter()
            .any(|model| model.trim().eq_ignore_ascii_case(CODEX_IMAGE_MODEL_ID))
}

/// 是否应写入 Codex 生图兼容 header（actor 等）。
/// - 本地 API 服务 loopback：始终 true（网关自带 image 能力）
/// - 第三方：仅当账号模型目录显式包含 gpt-image-2（无则清 header，避免卡 Confirming）
fn api_key_provider_should_enable_imagegen(
    account: &CodexAccount,
    provider_config: &ApiProviderConfig,
) -> bool {
    let base_url = provider_config
        .base_url
        .as_deref()
        .unwrap_or(CODEX_DEFAULT_OPENAI_BASE_URL);
    let is_local_access_loopback = provider_config.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        && is_loopback_http_base_url(Some(base_url));
    if is_local_access_loopback {
        return true;
    }
    api_key_account_supports_image_generation(account)
}

fn remove_provider_static_header(provider_table: &mut toml_edit::Table, header_name: &str) {
    let mut remove_http_headers = false;
    if let Some(headers) = provider_table.get_mut(CODEX_CONFIG_HTTP_HEADERS_KEY) {
        if let Some(inline) = headers.as_inline_table_mut() {
            let matching_keys: Vec<String> = inline
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(header_name))
                .map(|(key, _)| key.to_string())
                .collect();
            for key in matching_keys {
                let _ = inline.remove(&key);
            }
            remove_http_headers = inline.is_empty();
        } else if let Some(table) = headers.as_table_mut() {
            let matching_keys: Vec<String> = table
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(header_name))
                .map(|(key, _)| key.to_string())
                .collect();
            for key in matching_keys {
                let _ = table.remove(&key);
            }
            remove_http_headers = table.is_empty();
        }
    }
    if remove_http_headers {
        let _ = provider_table.remove(CODEX_CONFIG_HTTP_HEADERS_KEY);
    }
}

fn set_provider_static_header(
    provider_table: &mut toml_edit::Table,
    header_name: &str,
    header_value: &str,
) {
    remove_provider_static_header(provider_table, header_name);
    if provider_table.get(CODEX_CONFIG_HTTP_HEADERS_KEY).is_none() {
        provider_table[CODEX_CONFIG_HTTP_HEADERS_KEY] =
            toml_edit::Item::Value(toml_edit::Value::InlineTable(toml_edit::InlineTable::new()));
    }

    let headers = provider_table
        .get_mut(CODEX_CONFIG_HTTP_HEADERS_KEY)
        .expect("http_headers should exist after initialization");
    if let Some(inline) = headers.as_inline_table_mut() {
        inline.insert(header_name, toml_edit::Value::from(header_value));
    } else if let Some(table) = headers.as_table_mut() {
        table[header_name] = value(header_value);
    } else {
        let mut inline = toml_edit::InlineTable::new();
        inline.insert(header_name, toml_edit::Value::from(header_value));
        *headers = toml_edit::Item::Value(toml_edit::Value::InlineTable(inline));
    }
}

fn remove_imagegen_headers(provider_table: &mut toml_edit::Table) {
    remove_provider_static_header(provider_table, CODEX_IMAGEGEN_ACTOR_HEADER);
    remove_provider_static_header(provider_table, CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER);
}

fn set_imagegen_headers(provider_table: &mut toml_edit::Table, images_only_for_chat: bool) {
    remove_imagegen_headers(provider_table);
    set_provider_static_header(
        provider_table,
        CODEX_IMAGEGEN_ACTOR_HEADER,
        CODEX_IMAGEGEN_ACTOR_HEADER_VALUE,
    );
    if images_only_for_chat {
        set_provider_static_header(
            provider_table,
            CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
            CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE,
        );
    }
}

fn write_api_key_provider_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
    bearer_token: &str,
    supports_websockets: bool,
    supports_image_generation: bool,
    // true → Codex 使用 auth.json/Keychain OAuth 登录态（绑定 OAuth）。
    // false → 纯 API Key，配合 actor 走 bearer 生图。
    require_openai_auth: bool,
) -> Result<(), String> {
    let config_path = get_config_toml_path(base_dir);
    let bearer_token = normalize_api_key(bearer_token)
        .ok_or_else(|| "API Key 账号缺少可写入 provider 的密钥".to_string())?;
    let base_url = provider_config
        .base_url
        .as_deref()
        .unwrap_or(CODEX_DEFAULT_OPENAI_BASE_URL);
    let provider_name = provider_config
        .provider_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(CODEX_DEFAULT_RUNTIME_PROVIDER_NAME);

    let existing = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        crate::modules::codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    doc[CODEX_CONFIG_MODEL_PROVIDER_KEY] = value(CODEX_RUNTIME_MODEL_PROVIDER_ID);
    if doc.get(CODEX_CONFIG_MODEL_PROVIDERS_KEY).is_none() {
        doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY] = toml_edit::table();
    }
    let model_providers = doc[CODEX_CONFIG_MODEL_PROVIDERS_KEY]
        .as_table_mut()
        .ok_or("config.toml 中 model_providers 不是合法表结构")?;
    if !model_providers.contains_key(CODEX_RUNTIME_MODEL_PROVIDER_ID) {
        model_providers[CODEX_RUNTIME_MODEL_PROVIDER_ID] = toml_edit::table();
    }
    let provider_table = model_providers[CODEX_RUNTIME_MODEL_PROVIDER_ID]
        .as_table_mut()
        .ok_or("config.toml 中目标 provider 不是合法表结构")?;
    provider_table["name"] = value(provider_name);
    provider_table["base_url"] = value(base_url);
    provider_table["wire_api"] = value(CODEX_PROVIDER_WIRE_API);
    // require_openai_auth 与生图 headers 解耦：
    // - 纯 API Key 生图：require=false + actor
    // - 绑定 OAuth 的本地 API：require=true（显示账号）+ actor + chat disable（生图走本地）
    provider_table["requires_openai_auth"] = value(require_openai_auth);
    provider_table[CODEX_CONFIG_EXPERIMENTAL_BEARER_TOKEN_KEY] = value(bearer_token);
    provider_table["supports_websockets"] = value(supports_websockets);
    let is_local_access_loopback = provider_config.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        && is_loopback_http_base_url(Some(base_url));
    if supports_image_generation {
        set_imagegen_headers(provider_table, is_local_access_loopback);
    } else {
        remove_imagegen_headers(provider_table);
    }
    // 本地 API 服务：写入实例 ID，供网关/请求日志区分多开来源。
    if is_local_access_loopback {
        let instance_id = client_instance_id_for_profile_dir(base_dir);
        set_provider_static_header(
            provider_table,
            CODEX_CLIENT_INSTANCE_ID_HEADER,
            &instance_id,
        );
    } else {
        remove_provider_static_header(provider_table, CODEX_CLIENT_INSTANCE_ID_HEADER);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
        .map_err(|e| format!("写入 config.toml 失败: {}", e))
}

pub(crate) fn client_instance_id_for_profile_dir(base_dir: &Path) -> String {
    base_dir
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// 旧版数据目录（~/Library/Application Support/com.antigravity.cockpit-tools/）
fn get_old_codex_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().expect("无法获取用户目录"))
        .join("com.antigravity.cockpit-tools")
}

/// 将旧目录中的 codex 数据迁移到新目录（一次性，迁移成功后删除旧文件）
fn migrate_codex_data_if_needed(new_data_dir: &PathBuf) {
    let old_dir = get_old_codex_data_dir();
    if !old_dir.exists() {
        return;
    }

    // 迁移 codex_accounts.json
    let old_index = old_dir.join("codex_accounts.json");
    let new_index = new_data_dir.join("codex_accounts.json");
    if old_index.exists() && !new_index.exists() {
        match fs::copy(&old_index, &new_index) {
            Ok(_) => {
                logger::log_info("[Codex Migration] codex_accounts.json 迁移成功，清理旧文件");
                let _ = fs::remove_file(&old_index);
            }
            Err(e) => {
                logger::log_warn(&format!(
                    "[Codex Migration] codex_accounts.json 迁移失败: {}",
                    e
                ));
            }
        }
    }

    // 迁移 codex_accounts/ 目录
    let old_accounts_dir = old_dir.join("codex_accounts");
    let new_accounts_dir = new_data_dir.join("codex_accounts");
    if old_accounts_dir.exists() && old_accounts_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&old_accounts_dir) {
            for entry in entries.flatten() {
                let old_path = entry.path();
                if !old_path.is_file() {
                    continue;
                }
                if let Some(fname) = old_path.file_name() {
                    let new_path = new_accounts_dir.join(fname);
                    if new_path.exists() {
                        // 新目录已有同名文件，跳过（不覆盖）
                        continue;
                    }
                    match fs::copy(&old_path, &new_path) {
                        Ok(_) => {
                            logger::log_info(&format!(
                                "[Codex Migration] 账号文件迁移成功: {:?}",
                                fname
                            ));
                            let _ = fs::remove_file(&old_path);
                        }
                        Err(e) => {
                            logger::log_warn(&format!(
                                "[Codex Migration] 账号文件迁移失败: {:?}, error={}",
                                fname, e
                            ));
                        }
                    }
                }
            }
            // 如果旧目录已空，尝试删除它
            if fs::read_dir(&old_accounts_dir)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
            {
                let _ = fs::remove_dir(&old_accounts_dir);
            }
        }
    }
}

/// 获取我们的多账号存储路径（统一使用 ~/.antigravity_cockpit/）
fn get_accounts_storage_path() -> PathBuf {
    let data_dir = account::get_data_dir().unwrap_or_else(|_| {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".antigravity_cockpit")
    });
    fs::create_dir_all(&data_dir).ok();
    migrate_codex_data_if_needed(&data_dir);
    data_dir.join("codex_accounts.json")
}

/// 获取账号详情存储目录（统一使用 ~/.antigravity_cockpit/codex_accounts/）
fn get_accounts_dir() -> PathBuf {
    let data_dir = account::get_data_dir().unwrap_or_else(|_| {
        dirs::home_dir()
            .expect("无法获取用户目录")
            .join(".antigravity_cockpit")
    });
    let accounts_dir = data_dir.join("codex_accounts");
    fs::create_dir_all(&accounts_dir).ok();
    accounts_dir
}

/// 解析 JWT Token 的 payload
pub fn decode_jwt_payload(token: &str) -> Result<CodexJwtPayload, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err("无效的 JWT Token 格式".to_string());
    }

    let payload_b64 = parts[1];
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| format!("Base64 解码失败: {}", e))?;

    let payload: CodexJwtPayload =
        serde_json::from_slice(&payload_bytes).map_err(|e| format!("JSON 解析失败: {}", e))?;

    Ok(payload)
}

fn decode_jwt_payload_value(token: &str) -> Option<serde_json::Value> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let payload_str = String::from_utf8(payload_bytes).ok()?;
    serde_json::from_str(&payload_str).ok()
}

fn normalize_optional_value(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_optional_ref(value: Option<&str>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn first_json_string(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current
            .as_str()
            .and_then(|raw| normalize_optional_ref(Some(raw)))
    })
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn codex_token_lock_for(account_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut locks = CODEX_TOKEN_REFRESH_LOCKS
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    locks
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

struct CodexTokenRefreshFileLock {
    path: PathBuf,
}

impl Drop for CodexTokenRefreshFileLock {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_dir_all(&self.path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                logger::log_warn(&format!(
                    "释放 Codex Token 跨进程刷新锁失败: lock_path={}, error={}",
                    self.path.display(),
                    err
                ));
            }
        }
    }
}

fn codex_token_refresh_file_lock_path(account_id: &str) -> PathBuf {
    let lock_name = URL_SAFE_NO_PAD.encode(account_id.as_bytes());
    get_accounts_dir()
        .join(".locks")
        .join(format!("token-refresh-{}.lock", lock_name))
}

fn codex_token_refresh_file_lock_is_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age >= Duration::from_secs(CODEX_TOKEN_REFRESH_FILE_LOCK_STALE_SECONDS))
        .unwrap_or(false)
}

async fn acquire_codex_token_refresh_file_lock(
    account_id: &str,
    reason: &str,
) -> Result<CodexTokenRefreshFileLock, String> {
    let path = codex_token_refresh_file_lock_path(account_id);
    let parent = path
        .parent()
        .ok_or_else(|| format!("Codex Token 刷新锁路径无效: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|err| format_io_error("创建 Codex Token 刷新锁目录", parent, &err))?;

    let started = Instant::now();
    loop {
        match fs::create_dir(&path) {
            Ok(()) => {
                let owner_path = path.join("owner");
                let owner = format!(
                    "pid={}\naccount_id={}\nreason={}\ncreated_at={}\n",
                    std::process::id(),
                    account_id,
                    reason,
                    now_timestamp()
                );
                if let Err(err) = fs::write(&owner_path, owner) {
                    logger::log_warn(&format!(
                        "写入 Codex Token 跨进程刷新锁元数据失败: account_id={}, lock_path={}, error={}",
                        account_id,
                        owner_path.display(),
                        err
                    ));
                }
                return Ok(CodexTokenRefreshFileLock { path });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                if codex_token_refresh_file_lock_is_stale(&path) {
                    logger::log_warn(&format!(
                        "清理过期 Codex Token 跨进程刷新锁: account_id={}, lock_path={}",
                        account_id,
                        path.display()
                    ));
                    if let Err(remove_err) = fs::remove_dir_all(&path) {
                        logger::log_warn(&format!(
                            "清理过期 Codex Token 跨进程刷新锁失败: account_id={}, lock_path={}, error={}",
                            account_id,
                            path.display(),
                            remove_err
                        ));
                    }
                    continue;
                }

                if started.elapsed()
                    >= Duration::from_secs(CODEX_TOKEN_REFRESH_FILE_LOCK_TIMEOUT_SECONDS)
                {
                    return Err(format!(
                        "等待 Codex Token 刷新锁超时: account_id={}, lock_path={}, reason={}",
                        account_id,
                        path.display(),
                        reason
                    ));
                }

                tokio::time::sleep(Duration::from_millis(CODEX_TOKEN_REFRESH_FILE_LOCK_POLL_MS))
                    .await;
            }
            Err(err) => {
                return Err(format_io_error("创建 Codex Token 刷新锁", &path, &err));
            }
        }
    }
}

fn mark_token_chain_updated(account: &mut CodexAccount) {
    account.token_generation = account.token_generation.saturating_add(1);
    account.token_updated_at = Some(now_timestamp());
    account.token_source_mode = CODEX_TOKEN_SOURCE_MANAGED.to_string();
    account.requires_reauth = false;
    account.reauth_reason = None;
}

fn sync_identity_from_tokens(account: &mut CodexAccount) {
    if let Ok((
        email,
        user_id,
        plan_type,
        subscription_active_until,
        id_token_account_id,
        id_token_org_id,
    )) = extract_user_info(&account.tokens.id_token)
    {
        if !email.trim().is_empty() {
            account.email = email;
        }
        account.user_id = user_id;
        account.plan_type = plan_type;
        account.subscription_active_until = subscription_active_until;
        account.account_id = normalize_optional_value(
            extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
                .or(id_token_account_id)
                .or_else(|| account.account_id.clone()),
        );
        account.organization_id = normalize_optional_value(
            extract_chatgpt_organization_id_from_access_token(&account.tokens.access_token)
                .or(id_token_org_id)
                .or_else(|| account.organization_id.clone()),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexRefreshErrorKind {
    RefreshTokenReused,
    RefreshTokenExpired,
    RefreshTokenInvalidated,
    InvalidGrant,
    UnsupportedCountryRegion,
    Other,
}

fn classify_refresh_error(message: &str) -> CodexRefreshErrorKind {
    let lower = message.to_ascii_lowercase();
    if lower.contains("unsupported_country_region_territory") {
        return CodexRefreshErrorKind::UnsupportedCountryRegion;
    }
    if lower.contains("refresh_token_reused") {
        return CodexRefreshErrorKind::RefreshTokenReused;
    }
    if lower.contains("refresh_token_expired") {
        return CodexRefreshErrorKind::RefreshTokenExpired;
    }
    if lower.contains("refresh_token_invalidated")
        || lower.contains("token_invalidated")
        || lower.contains("authentication token has been invalidated")
    {
        return CodexRefreshErrorKind::RefreshTokenInvalidated;
    }
    if lower.contains("invalid_grant") || lower.contains("invalid refresh token") {
        return CodexRefreshErrorKind::InvalidGrant;
    }
    CodexRefreshErrorKind::Other
}

fn is_reauth_required_refresh_error(message: &str) -> bool {
    matches!(
        classify_refresh_error(message),
        CodexRefreshErrorKind::RefreshTokenReused
            | CodexRefreshErrorKind::RefreshTokenExpired
            | CodexRefreshErrorKind::RefreshTokenInvalidated
            | CodexRefreshErrorKind::InvalidGrant
    )
}

fn format_refresh_error_for_user(raw: &str) -> String {
    match classify_refresh_error(raw) {
        CodexRefreshErrorKind::RefreshTokenReused => format!(
            "Codex 授权已失效：refresh_token 已被其它客户端或实例使用过。Codex 的 refresh_token 是轮换凭据，旧凭据再次刷新会被服务端拒绝。请重新登录，并避免官方 Codex、其它实例或外部工具同时刷新同一账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::RefreshTokenExpired => format!(
            "Codex 登录授权已过期，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::RefreshTokenInvalidated => format!(
            "Codex 登录授权已被服务端撤销，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::InvalidGrant => format!(
            "Codex 登录授权无效，无法自动刷新。请重新登录 Codex 账号。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::UnsupportedCountryRegion => format!(
            "当前网络地区不支持刷新 Codex 授权。OpenAI 授权服务拒绝了当前网络出口的刷新请求，请切换到支持的网络地区后重试。原始错误: {}",
            raw
        ),
        CodexRefreshErrorKind::Other => format!("Token 已过期且刷新失败: {}", raw),
    }
}

fn mark_account_requires_reauth(account: &mut CodexAccount, reason: &str) -> Result<(), String> {
    account.requires_reauth = true;
    account.reauth_reason = Some(reason.to_string());
    save_account(account)
}

fn is_missing_refresh_token_reason(reason: &str) -> bool {
    reason.contains("缺少 refresh_token")
}

pub(crate) fn account_has_refresh_token(account: &CodexAccount) -> bool {
    account
        .tokens
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .is_some()
}

pub fn is_pending_oauth_account(account: &CodexAccount) -> bool {
    !account.is_api_key_auth()
        && account
            .authorization_status
            .as_deref()
            .map(str::trim)
            .map(|value| value.eq_ignore_ascii_case(CODEX_AUTHORIZATION_STATUS_PENDING))
            .unwrap_or(false)
}

fn clear_stale_missing_refresh_token_reauth(account: &mut CodexAccount) -> Result<(), String> {
    let is_missing_refresh_token_reauth = account
        .reauth_reason
        .as_deref()
        .map(is_missing_refresh_token_reason)
        .unwrap_or(false);

    if !account.requires_reauth || !is_missing_refresh_token_reauth {
        return Ok(());
    }
    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Ok(());
    }

    account.requires_reauth = false;
    account.reauth_reason = None;
    save_account(account)
}

pub fn mark_access_token_only_account_requires_reauth(account_id: &str) -> Result<(), String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account_has_refresh_token(&account) {
        return Ok(());
    }
    mark_account_requires_reauth(&mut account, CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON)
}

fn retain_existing_refresh_token_if_missing(
    mut tokens: CodexTokens,
    existing: Option<&CodexAccount>,
) -> CodexTokens {
    tokens.refresh_token = normalize_optional_value(tokens.refresh_token).or_else(|| {
        existing.and_then(|account| normalize_optional_ref(account.tokens.refresh_token.as_deref()))
    });
    tokens
}

pub fn extract_chatgpt_account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    first_json_string(auth_data, &[&["chatgpt_account_id"], &["account_id"]])
}

pub fn extract_chatgpt_organization_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    const ORG_KEYS: [&str; 6] = [
        "organization_id",
        "chatgpt_organization_id",
        "chatgpt_org_id",
        "org_id",
        "poid",
        "POID",
    ];
    for key in ORG_KEYS {
        if let Some(value) = normalize_optional_ref(auth_data.get(key).and_then(|v| v.as_str())) {
            return Some(value);
        }
    }
    if let Some(orgs) = auth_data
        .get("organizations")
        .and_then(|value| value.as_array())
    {
        if let Some(default_org) = orgs.iter().find(|org| {
            org.get("is_default")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        }) {
            if let Some(value) = first_json_string(default_org, &[&["id"]]) {
                return Some(value);
            }
        }
        if let Some(first_org) = orgs.first() {
            if let Some(value) = first_json_string(first_org, &[&["id"]]) {
                return Some(value);
            }
        }
    }
    None
}

fn extract_access_token_identity(
    access_token: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let Some(payload) = decode_jwt_payload_value(access_token) else {
        return (None, None, None, None, None, None);
    };

    let auth_data = payload.get("https://api.openai.com/auth");
    let email = first_json_string(&payload, &[&["email"]])
        .or_else(|| first_json_string(&payload, &[&["https://api.openai.com/profile", "email"]]));
    let user_id = auth_data
        .and_then(|value| first_json_string(value, &[&["chatgpt_user_id"], &["user_id"]]))
        .or_else(|| first_json_string(&payload, &[&["sub"]]));
    let plan_type = auth_data.and_then(|value| first_json_string(value, &[&["chatgpt_plan_type"]]));
    let subscription_active_until = auth_data.and_then(|value| {
        value
            .get("chatgpt_subscription_active_until")
            .and_then(|item| normalize_optional_json_scalar(Some(item)))
    });
    let account_id = extract_chatgpt_account_id_from_access_token(access_token);
    let organization_id = extract_chatgpt_organization_id_from_access_token(access_token);

    (
        email,
        user_id,
        plan_type,
        subscription_active_until,
        account_id,
        organization_id,
    )
}

fn access_token_fingerprint(access_token: &str) -> String {
    let digest = format!("{:x}", md5::compute(access_token.as_bytes()));
    digest.chars().take(12).collect()
}

fn build_account_storage_id(
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> String {
    let mut seed = email.trim().to_string();
    if let Some(id) = normalize_optional_ref(account_id) {
        seed.push('|');
        seed.push_str(&id);
    }
    if let Some(org) = normalize_optional_ref(organization_id) {
        seed.push('|');
        seed.push_str(&org);
    }
    format!("codex_{:x}", md5::compute(seed.as_bytes()))
}

fn find_existing_account_id(
    index: &CodexAccountIndex,
    email: &str,
    account_id: Option<&str>,
    organization_id: Option<&str>,
) -> Option<String> {
    let expected_account_id = normalize_optional_ref(account_id);
    let expected_org_id = normalize_optional_ref(organization_id);
    let mut first_email_match: Option<String> = None;
    let mut email_match_count = 0usize;
    let mut account_id_match_without_org: Option<String> = None;
    let mut legacy_email_only_candidate: Option<String> = None;
    let mut legacy_email_only_count = 0usize;

    for summary in &index.accounts {
        if !summary.email.eq_ignore_ascii_case(email) {
            continue;
        }
        email_match_count += 1;
        if first_email_match.is_none() {
            first_email_match = Some(summary.id.clone());
        }

        let Some(account) = load_account(&summary.id) else {
            continue;
        };

        let current_account_id = normalize_optional_ref(account.account_id.as_deref());
        let current_org_id = normalize_optional_ref(account.organization_id.as_deref());

        let is_exact_match =
            current_account_id == expected_account_id && current_org_id == expected_org_id;
        if is_exact_match {
            return Some(summary.id.clone());
        }

        if expected_account_id.is_some()
            && current_account_id == expected_account_id
            && current_org_id.is_none()
            && account_id_match_without_org.is_none()
        {
            account_id_match_without_org = Some(summary.id.clone());
        }

        if (expected_account_id.is_some() || expected_org_id.is_some())
            && current_account_id.is_none()
            && current_org_id.is_none()
        {
            legacy_email_only_count += 1;
            if legacy_email_only_candidate.is_none() {
                legacy_email_only_candidate = Some(summary.id.clone());
            }
        }
    }

    if expected_account_id.is_some() || expected_org_id.is_some() {
        return account_id_match_without_org.or_else(|| {
            if legacy_email_only_count == 1 {
                legacy_email_only_candidate
            } else {
                None
            }
        });
    }

    if email_match_count == 1 {
        return first_email_match;
    }

    None
}

/// 从 id_token 提取用户信息
pub fn extract_user_info(
    id_token: &str,
) -> Result<
    (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
    String,
> {
    let payload = decode_jwt_payload(id_token)?;

    let email = payload
        .email
        .or_else(|| {
            payload
                .profile_data
                .as_ref()
                .and_then(|data| data.email.clone())
        })
        .ok_or("id_token 中缺少 email")?;
    let user_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_user_id.clone());
    let plan_type = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_plan_type.clone());
    let subscription_active_until = payload
        .auth_data
        .as_ref()
        .and_then(|d| normalize_optional_json_scalar(d.chatgpt_subscription_active_until.as_ref()));
    let account_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.account_id.clone());
    let organization_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.organization_id.clone());

    Ok((
        email,
        user_id,
        plan_type,
        subscription_active_until,
        account_id,
        organization_id,
    ))
}

fn account_summary_from_account(account: &CodexAccount) -> CodexAccountSummary {
    CodexAccountSummary {
        id: account.id.clone(),
        email: account.email.clone(),
        plan_type: account.plan_type.clone(),
        subscription_active_until: account.subscription_active_until.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    }
}

fn account_summary_matches_account(summary: &CodexAccountSummary, account: &CodexAccount) -> bool {
    summary.email == account.email
        && summary.plan_type == account.plan_type
        && summary.subscription_active_until == account.subscription_active_until
        && summary.created_at == account.created_at
        && summary.last_used == account.last_used
}

fn sync_loaded_accounts_to_index_cache(
    index: &mut CodexAccountIndex,
    accounts: &[CodexAccount],
) -> bool {
    let mut changed = false;
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
        changed = true;
    }

    for account in accounts {
        let next_summary = account_summary_from_account(account);
        if let Some(summary) = index
            .accounts
            .iter_mut()
            .find(|summary| summary.id == account.id)
        {
            if !account_summary_matches_account(summary, account) {
                *summary = next_summary;
                changed = true;
            }
        } else {
            index.accounts.push(next_summary);
            changed = true;
        }
    }

    changed
}

fn apply_index_summary_to_account_detail(
    account: &mut CodexAccount,
    summary: &CodexAccountSummary,
) -> bool {
    let mut changed = false;

    if account.email.trim().is_empty() && !summary.email.trim().is_empty() {
        account.email = summary.email.clone();
        changed = true;
    }

    if account.plan_type.is_none() && summary.plan_type.is_some() {
        account.plan_type = summary.plan_type.clone();
        changed = true;
    }

    if account.subscription_active_until.is_none() && summary.subscription_active_until.is_some() {
        account.subscription_active_until = summary.subscription_active_until.clone();
        changed = true;
    }

    if account.created_at <= 0 && summary.created_at > 0 {
        account.created_at = summary.created_at;
        changed = true;
    }

    if summary.last_used > account.last_used {
        account.last_used = summary.last_used;
        changed = true;
    } else if account.last_used <= 0 {
        account.last_used = account.created_at.max(summary.last_used);
        changed = true;
    }

    changed
}

fn collect_account_detail_file_ids() -> Result<HashSet<String>, String> {
    let accounts_dir = get_accounts_dir();
    if !accounts_dir.exists() {
        return Ok(HashSet::new());
    }

    let entries = fs::read_dir(&accounts_dir).map_err(|error| {
        format!(
            "读取 Codex 账号详情目录失败: path={}, error={}",
            accounts_dir.display(),
            error
        )
    })?;

    let mut ids = HashSet::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("遍历 Codex 账号详情目录失败: {}", error))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_json = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
        if !is_json {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|name| name.to_str()) {
            ids.insert(stem.to_string());
        }
    }

    Ok(ids)
}

fn build_account_index_from_summaries(
    mut summaries: Vec<CodexAccountSummary>,
    previous_current_account_id: Option<String>,
) -> CodexAccountIndex {
    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut summaries,
        |summary| summary.last_used,
        |summary| summary.created_at,
        |summary| summary.id.as_str(),
    );

    let mut index = CodexAccountIndex::new();
    index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    index.accounts = summaries;
    index.current_account_id = previous_current_account_id.filter(|current_id| {
        index
            .accounts
            .iter()
            .any(|summary| summary.id.as_str() == current_id.as_str())
    });
    index
}

fn empty_reconciled_account_index() -> CodexAccountIndex {
    let mut index = CodexAccountIndex::new();
    index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    index
}

fn should_reconcile_account_index_with_details(
    index: &CodexAccountIndex,
    detail_ids: &HashSet<String>,
) -> bool {
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        return true;
    }

    if index.accounts.len() != detail_ids.len() {
        return true;
    }

    let index_ids: HashSet<String> = index
        .accounts
        .iter()
        .map(|account| account.id.clone())
        .collect();
    if &index_ids != detail_ids {
        return true;
    }

    if let Some(current_id) = index.current_account_id.as_deref() {
        return !detail_ids.contains(current_id);
    }

    false
}

fn reconcile_account_index_with_details_if_needed(
    index: CodexAccountIndex,
    reason: &str,
) -> CodexAccountIndex {
    let detail_ids = match collect_account_detail_file_ids() {
        Ok(ids) => ids,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 检查账号详情目录失败，保留当前索引: reason={}, error={}",
                reason, error
            ));
            return index;
        }
    };

    if detail_ids.is_empty() {
        if !index.accounts.is_empty()
            || index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION
            || index.current_account_id.is_some()
        {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号详情目录为空，已清空索引缓存: reason={}, indexed_accounts={}",
                reason,
                index.accounts.len()
            ));
            let empty = empty_reconciled_account_index();
            if let Err(error) = save_account_index(&empty) {
                logger::log_warn(&format!(
                    "[Codex Account][Repair] 清空 Codex 索引缓存失败: reason={}, error={}",
                    reason, error
                ));
            }
            return empty;
        }
        return index;
    }

    if !should_reconcile_account_index_with_details(&index, &detail_ids) {
        return index;
    }

    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引缓存与详情文件不一致，准备按详情重建: reason={}, indexed_accounts={}, detail_files={}, detail_schema_version={}",
        reason,
        index.accounts.len(),
        detail_ids.len(),
        index.detail_schema_version
    ));

    repair_account_index_from_details_with_previous(reason, Some(&index)).unwrap_or(index)
}

/// 读取账号索引
pub fn load_account_index() -> CodexAccountIndex {
    let path = get_accounts_storage_path();
    if !path.exists() {
        return repair_account_index_from_details("索引文件不存在")
            .unwrap_or_else(CodexAccountIndex::new);
    }

    match fs::read_to_string(&path) {
        Ok(content) if content.trim().is_empty() => {
            repair_account_index_from_details("索引文件为空").unwrap_or_else(CodexAccountIndex::new)
        }
        Ok(content) => match serde_json::from_str::<CodexAccountIndex>(&content) {
            Ok(index) if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION => {
                reconcile_account_index_with_details_if_needed(index, "初始化账号详情数据")
            }
            Ok(index) => index,
            Err(err) => {
                logger::log_warn(&format!(
                    "[Codex Account] 账号索引解析失败，尝试按详情文件自动修复: path={}, error={}",
                    path.display(),
                    err
                ));
                repair_account_index_from_details("索引文件损坏")
                    .unwrap_or_else(CodexAccountIndex::new)
            }
        },
        Err(_) => CodexAccountIndex::new(),
    }
}

fn load_account_index_checked() -> Result<CodexAccountIndex, String> {
    let path = get_accounts_storage_path();
    if !path.exists() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 检测到账号索引文件不存在，准备尝试自动修复: path={}",
            path.display()
        ));
        if let Some(index) = repair_account_index_from_details("索引文件不存在") {
            logger::log_info(&format!(
                "[Codex Account][Repair] 索引文件不存在，已自动修复完成: recovered_accounts={}",
                index.accounts.len()
            ));
            return Ok(index);
        }
        logger::log_warn(
            "[Codex Account][Repair] 索引文件不存在，但未找到可恢复详情文件，返回空索引",
        );
        return Ok(CodexAccountIndex::new());
    }

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 读取账号索引失败，准备尝试自动修复: path={}, error={}",
                path.display(),
                err
            ));
            if let Some(index) = repair_account_index_from_details("索引文件读取失败") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 索引读取失败，已自动修复完成: recovered_accounts={}",
                    index.accounts.len()
                ));
                return Ok(index);
            }
            return Err(format!("读取账号索引失败: {}", err));
        }
    };

    if content.trim().is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 检测到账号索引文件为空，准备尝试自动修复: path={}",
            path.display()
        ));
        if let Some(index) = repair_account_index_from_details("索引文件为空") {
            logger::log_info(&format!(
                "[Codex Account][Repair] 空索引文件已自动修复完成: recovered_accounts={}",
                index.accounts.len()
            ));
            return Ok(index);
        }
        logger::log_warn(
            "[Codex Account][Repair] 索引文件为空，但未找到可恢复详情文件，返回空索引",
        );
        return Ok(CodexAccountIndex::new());
    }

    match serde_json::from_str::<CodexAccountIndex>(&content) {
        Ok(index) => Ok(reconcile_account_index_with_details_if_needed(
            index,
            "读取账号索引",
        )),
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号索引解析失败，准备尝试自动修复: path={}, error={}",
                path.display(),
                err
            ));
            if let Some(index) = repair_account_index_from_details("索引文件损坏") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 损坏索引文件已自动修复完成: recovered_accounts={}",
                    index.accounts.len()
                ));
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                "codex_accounts.json",
                &path.to_string_lossy(),
                &err.to_string(),
            ))
        }
    }
}

/// 保存账号索引
pub fn save_account_index(index: &CodexAccountIndex) -> Result<(), String> {
    let path = get_accounts_storage_path();
    let mut index = index.clone();
    if index.detail_schema_version < CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION {
        index.detail_schema_version = CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION;
    }
    let content = serde_json::to_string_pretty(&index).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号索引失败: {}", e))?;
    Ok(())
}

fn repair_account_index_from_details(reason: &str) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let previous_index = fs::read_to_string(&index_path)
        .ok()
        .and_then(|content| serde_json::from_str::<CodexAccountIndex>(&content).ok());
    repair_account_index_from_details_with_previous(reason, previous_index.as_ref())
}

fn repair_account_index_from_details_with_previous(
    reason: &str,
    previous_index: Option<&CodexAccountIndex>,
) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let accounts_dir = get_accounts_dir();
    let previous_current_account_id =
        previous_index.and_then(|index| index.current_account_id.clone());
    let summary_by_id: HashMap<String, CodexAccountSummary> = previous_index
        .map(|index| {
            index
                .accounts
                .iter()
                .map(|summary| (summary.id.clone(), summary.clone()))
                .collect()
        })
        .unwrap_or_default();
    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引异常，开始按详情文件重建: reason={}, index_path={}, accounts_dir={}",
        reason,
        index_path.display(),
        accounts_dir.display()
    ));

    let detail_ids = match collect_account_detail_file_ids() {
        Ok(ids) => ids,
        Err(err) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 扫描账号详情文件失败，无法自动修复: reason={}, accounts_dir={}, error={}",
                reason,
                accounts_dir.display(),
                err
            ));
            return None;
        }
    };

    if detail_ids.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    let mut account_ids: Vec<String> = detail_ids.into_iter().collect();
    account_ids.sort();
    let mut summaries = Vec::with_capacity(account_ids.len());
    let mut failed = Vec::new();
    for account_id in account_ids {
        match load_account_with_summary(&account_id, summary_by_id.get(&account_id)) {
            Ok(Some(account)) => summaries.push(account_summary_from_account(&account)),
            Ok(None) => failed.push(format!("{}: 详情文件不存在", account_id)),
            Err(error) => failed.push(format!("{}: {}", account_id, error)),
        }
    }

    if !failed.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 部分详情文件无法恢复，已跳过: reason={}, failed={}",
            reason,
            failed.join("; ")
        ));
    }

    if summaries.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已扫描到 {} 个账号详情，准备重建索引",
        summaries.len()
    ));

    let index = build_account_index_from_summaries(summaries, previous_current_account_id);

    logger::log_info(&format!(
        "[Codex Account][Repair] 索引重建完成，准备写回本地文件: recovered_accounts={}, current_account_id={}",
        index.accounts.len(),
        index.current_account_id.as_deref().unwrap_or("-")
    ));

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            logger::log_warn(&format!(
                "[Codex Account] 自动修复前备份索引失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        logger::log_warn(&format!(
            "[Codex Account] 自动修复索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已根据详情文件自动重建账号索引: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Some(index)
}

fn read_json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let raw = keys
        .iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_str()))?;
    normalize_optional_ref(Some(raw))
}

fn read_json_i64(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let item = value.get(*key)?;
        if item.is_string() {
            return parse_auth_file_last_refresh(Some(item));
        }
        item.as_i64()
            .or_else(|| item.as_u64().and_then(|raw| i64::try_from(raw).ok()))
    })
}

fn read_json_bool(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_bool()))
}

fn read_json_string_array(value: &serde_json::Value, keys: &[&str]) -> Option<Vec<String>> {
    let items = keys
        .iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_array()))?;
    let normalized = items
        .iter()
        .filter_map(|item| item.as_str())
        .filter_map(|item| normalize_optional_ref(Some(item)))
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn read_account_two_factor_secret(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "two_factor_secret",
            "twoFactorSecret",
            "account_two_factor_secret",
            "accountTwoFactorSecret",
        ],
    )
}

fn read_account_password(value: &serde_json::Value) -> Option<String> {
    read_json_string(value, &["account_password", "accountPassword", "password"])
}

fn read_account_phone_number(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "phone_number",
            "phoneNumber",
            "account_phone_number",
            "accountPhoneNumber",
        ],
    )
}

fn read_account_mail_url(value: &serde_json::Value) -> Option<String> {
    read_json_string(
        value,
        &[
            "mail_url",
            "mailUrl",
            "mail_address",
            "mailAddress",
            "mail_query_url",
            "mailQueryUrl",
        ],
    )
}

fn apply_account_sensitive_note_metadata(account: &mut CodexAccount, value: &serde_json::Value) {
    if let Some(secret) = read_account_two_factor_secret(value) {
        account.two_factor_secret = Some(secret);
    }
    if let Some(password) = read_account_password(value) {
        account.account_password = Some(password);
    }
    if let Some(phone_number) = read_account_phone_number(value) {
        account.phone_number = Some(phone_number);
    }
    if let Some(mail_url) = read_account_mail_url(value) {
        account.mail_url = Some(mail_url);
    }
}

fn read_codex_api_provider_mode(value: &serde_json::Value) -> Option<CodexApiProviderMode> {
    value
        .get("api_provider_mode")
        .or_else(|| value.get("apiProviderMode"))
        .and_then(|item| serde_json::from_value::<CodexApiProviderMode>(item.clone()).ok())
}

fn apply_compat_account_metadata(
    account: &mut CodexAccount,
    value: &serde_json::Value,
    summary: Option<&CodexAccountSummary>,
) {
    let now = now_timestamp();
    if account.email.trim().is_empty() {
        account.email = read_json_string(value, &["email", "account_email"])
            .or_else(|| summary.map(|item| item.email.clone()))
            .unwrap_or_else(|| account.id.clone());
    }
    account.account_name = read_json_string(value, &["account_name", "accountName"])
        .or_else(|| account.account_name.clone());
    account.account_structure = read_json_string(value, &["account_structure", "accountStructure"])
        .or_else(|| account.account_structure.clone());
    account.account_note = read_json_string(value, &["account_note", "accountNote"])
        .or_else(|| account.account_note.clone());
    apply_account_sensitive_note_metadata(account, value);
    account.auth_file_plan_type =
        read_json_string(value, &["auth_file_plan_type", "authFilePlanType"])
            .or_else(|| account.auth_file_plan_type.clone());
    account.plan_type = read_json_string(value, &["plan_type", "planType"])
        .or_else(|| account.plan_type.clone())
        .or_else(|| summary.and_then(|item| item.plan_type.clone()));
    account.subscription_active_until = read_json_string(
        value,
        &["subscription_active_until", "subscriptionActiveUntil"],
    )
    .or_else(|| account.subscription_active_until.clone())
    .or_else(|| summary.and_then(|item| item.subscription_active_until.clone()));
    account.created_at = read_json_i64(value, &["created_at", "createdAt"])
        .or_else(|| summary.map(|item| item.created_at))
        .unwrap_or(now);
    account.last_used = read_json_i64(value, &["last_used", "lastUsed"])
        .or_else(|| summary.map(|item| item.last_used))
        .unwrap_or(account.created_at);
    account.token_updated_at = read_json_i64(value, &["token_updated_at", "tokenUpdatedAt"])
        .or_else(|| parse_auth_file_last_refresh(value.get("last_refresh")))
        .or(account.token_updated_at);
    account.authorization_status =
        read_json_string(value, &["authorization_status", "authorizationStatus"])
            .or_else(|| account.authorization_status.clone());
    account.tags = read_json_string_array(value, &["tags"]).or_else(|| account.tags.clone());
}

fn apply_api_key_import_metadata(account: &mut CodexAccount, value: &serde_json::Value) {
    if let Some(account_name) = read_json_string(value, &["account_name", "accountName"]) {
        account.account_name = Some(account_name);
    }
    if let Some(account_note) = read_json_string(value, &["account_note", "accountNote"]) {
        account.account_note = Some(account_note);
    }
    apply_account_sensitive_note_metadata(account, value);
    if let Some(plan_type) = read_json_string(value, &["plan_type", "planType"]) {
        account.plan_type = Some(plan_type);
    }
    if let Some(subscription_active_until) = read_json_string(
        value,
        &["subscription_active_until", "subscriptionActiveUntil"],
    ) {
        account.subscription_active_until = Some(subscription_active_until);
    }
    if let Some(auth_file_plan_type) =
        read_json_string(value, &["auth_file_plan_type", "authFilePlanType"])
    {
        account.auth_file_plan_type = Some(auth_file_plan_type);
    }
    if let Some(tags) = read_json_string_array(value, &["tags"]) {
        account.tags = Some(tags);
    }
    if let Some(api_wire_api) = read_json_string(value, &["api_wire_api", "apiWireApi"]) {
        account.api_wire_api = normalize_api_wire_api(Some(api_wire_api));
    }
    if let Some(sync_model_catalog) = read_json_bool(
        value,
        &[
            "api_sync_model_catalog_to_codex",
            "apiSyncModelCatalogToCodex",
        ],
    ) {
        account.api_sync_model_catalog_to_codex = sync_model_catalog;
    }
    if let Some(supports_websockets) =
        read_json_bool(value, &["api_supports_websockets", "apiSupportsWebsockets"])
    {
        account.api_supports_websockets = supports_websockets;
        let _ = normalize_api_key_websocket_capability(account);
    }
}

fn parse_codex_account_compat(
    value: serde_json::Value,
    fallback_id: &str,
    summary: Option<&CodexAccountSummary>,
) -> Result<Option<CodexAccount>, String> {
    if let Ok(mut account) = serde_json::from_value::<CodexAccount>(value.clone()) {
        if account.id.trim().is_empty() {
            account.id = fallback_id.to_string();
        }
        apply_compat_account_metadata(&mut account, &value, summary);
        normalize_api_key_websocket_capability(&mut account);
        return Ok(Some(account));
    }

    if is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|item| item.as_str())
            .or_else(|| value.get("authMode").and_then(|item| item.as_str())),
    ) {
        let Some(api_key) = value
            .get("OPENAI_API_KEY")
            .and_then(|item| item.as_str())
            .and_then(normalize_api_key)
        else {
            return Ok(None);
        };
        let api_base_url_hint = extract_api_base_url_from_json_value(&value);
        let (api_key, api_base_url) =
            validate_api_key_credentials(&api_key, api_base_url_hint.as_deref())?;
        let provider_config = resolve_api_provider_config(
            api_base_url.as_deref(),
            read_codex_api_provider_mode(&value),
            value.get("api_provider_id").and_then(|item| item.as_str()),
            value
                .get("api_provider_name")
                .and_then(|item| item.as_str()),
        )?;
        let mut account = CodexAccount::new_api_key(
            fallback_id.to_string(),
            read_json_string(&value, &["email", "account_email"])
                .or_else(|| summary.map(|item| item.email.clone()))
                .unwrap_or_else(|| build_api_key_email(&api_key)),
            api_key,
            provider_config.mode,
            provider_config.base_url,
            provider_config.provider_id,
            provider_config.provider_name,
            Vec::new(),
        );
        apply_compat_account_metadata(&mut account, &value, summary);
        apply_api_key_import_metadata(&mut account, &value);
        account.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        return Ok(Some(account));
    }

    let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(&value) else {
        return Ok(None);
    };
    let mut account = CodexAccount::new(
        fallback_id.to_string(),
        read_json_string(&value, &["email", "account_email"])
            .or_else(|| summary.map(|item| item.email.clone()))
            .unwrap_or_else(|| fallback_id.to_string()),
        tokens,
    );
    account.account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&account.tokens.access_token)
            .or(account_id_hint)
            .or_else(|| read_json_string(&value, &["account_id", "accountId"])),
    );
    account.organization_id = normalize_optional_value(read_json_string(
        &value,
        &["organization_id", "organizationId"],
    ));
    sync_identity_from_tokens(&mut account);
    apply_compat_account_metadata(&mut account, &value, summary);
    Ok(Some(account))
}

/// 读取单个账号详情
pub fn load_account(account_id: &str) -> Option<CodexAccount> {
    load_account_with_summary(account_id, None).ok().flatten()
}

/// 绑定 OAuth 的 API Key：不走本地网关生图兼容（保持绑定显示/客户端能力）。
/// 纯 API Key 生图走 provider 的 gpt-image-2 + actor header，与本开关无关。
fn clear_bound_oauth_local_gateway_flag(account: &mut CodexAccount) -> bool {
    if !account.bound_oauth_use_local_gateway {
        return false;
    }
    account.bound_oauth_use_local_gateway = false;
    true
}

fn load_account_after_index_repair(account_id: &str) -> Option<CodexAccount> {
    if let Some(account) = load_account(account_id) {
        return Some(account);
    }

    logger::log_warn(&format!(
        "[Codex Account][Repair] 切号目标账号详情缺失，尝试按详情文件重建索引后重试: account_id={}",
        account_id
    ));
    let repaired = repair_account_index_from_details("切号目标账号不存在")?;
    if !repaired
        .accounts
        .iter()
        .any(|summary| summary.id == account_id)
    {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 重建索引后仍未找到切号目标账号: account_id={}",
            account_id
        ));
        return None;
    }

    load_account(account_id)
}

fn load_account_with_summary(
    account_id: &str,
    summary: Option<&CodexAccountSummary>,
) -> Result<Option<CodexAccount>, String> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取账号详情失败 ({}): {}", path.display(), error))?;

    // AES-GCM envelope first (#1104), then plaintext + compat paths.
    if let Ok((mut account, needs_rotation)) =
        crate::modules::secure_account_storage::deserialize_account_file::<CodexAccount>(
            &path, &content,
        )
    {
        let migrated_index_summary = summary
            .map(|summary| apply_index_summary_to_account_detail(&mut account, summary))
            .unwrap_or(false);
        // 绑定 OAuth 时强制关闭本地网关标志，避免误走旧「禁生图 + 本地网关」路径。
        let cleared_bound_oauth_gateway = clear_bound_oauth_local_gateway_flag(&mut account);
        let migrated_wire_api = migrate_apikey_fun_wire_api(&mut account);
        let migrated_websocket = normalize_api_key_websocket_capability(&mut account);
        if needs_rotation
            || migrated_wire_api
            || migrated_websocket
            || cleared_bound_oauth_gateway
            || migrated_index_summary
        {
            let account_for_rewrite = account.clone();
            crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
                "codex",
                account_for_rewrite.id.clone(),
                path.clone(),
                content.as_bytes(),
                move || {
                    crate::modules::secure_account_storage::serialize_account_file(
                        "codex",
                        &account_for_rewrite,
                    )
                },
            );
        }
        return Ok(Some(account));
    }

    let value = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|error| format!("账号详情不是有效 JSON ({}): {}", path.display(), error))?;
    let mut account = parse_codex_account_compat(value.clone(), account_id, summary)?
        .ok_or_else(|| format!("账号详情缺少可识别凭据 ({})", path.display()))?;
    let _ = migrate_apikey_fun_wire_api(&mut account);
    let _ = clear_bound_oauth_local_gateway_flag(&mut account);

    let account_for_rewrite = account.clone();
    crate::modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
        "codex",
        account_for_rewrite.id.clone(),
        path.clone(),
        content.as_bytes(),
        move || {
            crate::modules::secure_account_storage::serialize_account_file(
                "codex",
                &account_for_rewrite,
            )
        },
    );

    Ok(Some(account))
}

/// 保存单个账号详情
pub fn save_account(account: &CodexAccount) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", &account.id));
    let content = crate::modules::secure_account_storage::serialize_account_file("codex", account)?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号详情失败: {}", e))?;
    Ok(())
}

/// 删除单个账号
pub fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if path.exists() {
        crate::modules::atomic_write::remove_file_locked(&path)
            .map_err(|e| format!("删除文件失败: {}", e))?;
    }
    Ok(())
}

// ─── Codex 分组额度刷新策略（最高优先级）────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexAccountGroupRecord {
    #[serde(default)]
    account_ids: Vec<String>,
    /// null/缺省 = 继承平台；-1 = 不刷新；>0 = 自定义分钟
    #[serde(default)]
    quota_auto_refresh_minutes: Option<i32>,
    /// 旧字段兼容：false → 不刷新
    #[serde(default)]
    quota_refresh_enabled: Option<bool>,
}

/// 分组额度策略：继承 / 关闭 / 自定义分钟
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexGroupQuotaRefreshPolicy {
    Inherit,
    Disabled,
    Minutes(u32),
}

impl CodexAccountGroupRecord {
    fn policy(&self) -> CodexGroupQuotaRefreshPolicy {
        if let Some(minutes) = self.quota_auto_refresh_minutes {
            if minutes <= -1 {
                return CodexGroupQuotaRefreshPolicy::Disabled;
            }
            if minutes > 0 {
                let clamped = minutes.clamp(1, 999) as u32;
                return CodexGroupQuotaRefreshPolicy::Minutes(clamped);
            }
            // 0 视为关闭
            return CodexGroupQuotaRefreshPolicy::Disabled;
        }
        if self.quota_refresh_enabled == Some(false) {
            return CodexGroupQuotaRefreshPolicy::Disabled;
        }
        CodexGroupQuotaRefreshPolicy::Inherit
    }
}

fn codex_account_groups_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEX_ACCOUNT_GROUPS_FILE))
}

fn load_codex_account_group_records() -> Vec<CodexAccountGroupRecord> {
    let path = match codex_account_groups_path() {
        Ok(path) => path,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 解析数据目录失败，跳过分组额度策略: {}",
                error
            ));
            return Vec::new();
        }
    };

    if !path.exists() {
        return Vec::new();
    }

    let raw = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 读取分组文件失败，跳过分组额度策略: path={}, error={}",
                path.display(),
                error
            ));
            return Vec::new();
        }
    };

    match serde_json::from_str::<Vec<CodexAccountGroupRecord>>(&raw) {
        Ok(groups) => groups,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex Groups] 解析分组文件失败，跳过分组额度策略: path={}, error={}",
                path.display(),
                error
            ));
            Vec::new()
        }
    }
}

/// 读取分组配置中「关闭额度刷新」的账号 ID 集合（策略 = Disabled / -1）。
pub fn load_quota_refresh_disabled_account_ids() -> HashSet<String> {
    let mut disabled = HashSet::new();
    for group in load_codex_account_group_records() {
        if group.policy() != CodexGroupQuotaRefreshPolicy::Disabled {
            continue;
        }
        for account_id in group.account_ids {
            let trimmed = account_id.trim();
            if !trimmed.is_empty() {
                disabled.insert(trimmed.to_string());
            }
        }
    }
    disabled
}

/// 账号是否允许参与「受策略约束」的额度刷新（自动/全量/默认批量）。
pub fn is_quota_refresh_enabled_for_account(account_id: &str) -> bool {
    let trimmed = account_id.trim();
    if trimmed.is_empty() {
        return true;
    }
    !load_quota_refresh_disabled_account_ids().contains(trimmed)
}

/// 按分组策略过滤账号 ID（剔除 Disabled），保持顺序。
pub fn filter_account_ids_by_quota_refresh_policy(account_ids: &[String]) -> Vec<String> {
    let disabled = load_quota_refresh_disabled_account_ids();
    if disabled.is_empty() {
        return account_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect();
    }
    account_ids
        .iter()
        .filter_map(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() || disabled.contains(trimmed) {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// 列出所有账号
pub fn list_accounts() -> Vec<CodexAccount> {
    let mut index = load_account_index();
    let accounts: Vec<CodexAccount> = index
        .accounts
        .iter()
        .filter_map(
            |summary| match load_account_with_summary(&summary.id, Some(summary)) {
                Ok(account) => account,
                Err(error) => {
                    logger::log_warn(&format!(
                        "[Codex Account] 跳过无法读取的账号详情: account_id={}, error={}",
                        summary.id, error
                    ));
                    None
                }
            },
        )
        .collect();
    if sync_loaded_accounts_to_index_cache(&mut index, &accounts) {
        if let Err(error) = save_account_index(&index) {
            logger::log_warn(&format!(
                "[Codex Account] 同步账号详情摘要到索引缓存失败: error={}",
                error
            ));
        }
    }
    accounts
}

pub fn list_accounts_checked() -> Result<Vec<CodexAccount>, String> {
    let mut index = load_account_index_checked()?;
    let mut accounts = Vec::new();
    let mut failed = Vec::new();
    let mut missing_detail_ids = Vec::new();
    let mut has_non_missing_failure = false;

    for summary in &index.accounts {
        match load_account_with_summary(&summary.id, Some(summary)) {
            Ok(Some(account)) => accounts.push(account),
            Ok(None) => {
                missing_detail_ids.push(summary.id.clone());
                failed.push(format!("{}: 详情文件不存在", summary.id));
            }
            Err(error) => {
                has_non_missing_failure = true;
                failed.push(format!("{}: {}", summary.id, error));
            }
        }
    }

    if !index.accounts.is_empty() && accounts.is_empty() {
        if !has_non_missing_failure && missing_detail_ids.len() == index.accounts.len() {
            logger::log_warn(&format!(
                "[Codex Account] 账号索引仅剩缺失详情文件的孤儿记录，已清空索引: {}",
                missing_detail_ids.join(", ")
            ));
            index.accounts.clear();
            index.current_account_id = None;
            save_account_index(&index)?;
            return Ok(Vec::new());
        }
        return Err(format!(
            "Codex 账号索引中有 {} 个账号，但详情文件均无法读取；已保留前端缓存，请从账号备份或本地账号文件恢复。{}",
            index.accounts.len(),
            failed.join("; ")
        ));
    }

    if !failed.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account] 部分账号详情无法读取，已保留可读取账号: loaded={}, failed={}",
            accounts.len(),
            failed.join("; ")
        ));
    }

    if sync_loaded_accounts_to_index_cache(&mut index, &accounts) {
        save_account_index(&index)?;
    }

    Ok(accounts)
}

/// 刷新账号资料（团队名/结构）
async fn refresh_account_profile_once(account_id: &str) -> Result<CodexAccount, String> {
    let mut account = prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Ok(account);
    }

    let (account_name, account_structure, account_id_from_remote) =
        fetch_remote_account_profile(&account).await?;

    let mut changed = false;

    if let Some(remote_account_id) = normalize_optional_value(account_id_from_remote) {
        if normalize_optional_ref(account.account_id.as_deref()) != Some(remote_account_id.clone())
        {
            account.account_id = Some(remote_account_id);
            changed = true;
        }
    }

    if let Some(name) = normalize_optional_value(account_name) {
        if normalize_optional_ref(account.account_name.as_deref()) != Some(name.clone()) {
            account.account_name = Some(name);
            changed = true;
        }
    }

    if let Some(structure) = normalize_optional_value(account_structure) {
        if normalize_optional_ref(account.account_structure.as_deref()) != Some(structure.clone()) {
            account.account_structure = Some(structure);
            changed = true;
        }
    }

    if changed {
        save_account(&account)?;
    }

    Ok(account)
}

pub async fn refresh_account_profile(account_id: &str) -> Result<CodexAccount, String> {
    refresh_account_profile_once(account_id).await
}

/// 添加或更新账号
pub fn upsert_account(tokens: CodexTokens) -> Result<CodexAccount, String> {
    upsert_account_with_hints(tokens, None, None)
}

pub fn upsert_account_for_reauth(
    tokens: CodexTokens,
    target_account_id: &str,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(tokens, None, None, Some(target_account_id))
}

pub fn upsert_api_key_account(
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Vec<String>,
    api_sync_model_catalog_to_codex: Option<bool>,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let (api_key, api_base_url) = validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let account_id = build_api_key_account_id(&api_key);
    let account_name = normalize_optional_value(account_name);
    let mut index = load_account_index();

    let mut account = if let Some(mut acc) = load_account(&account_id) {
        let sync_model_catalog_to_codex =
            api_sync_model_catalog_to_codex.unwrap_or(acc.api_sync_model_catalog_to_codex);
        apply_api_key_fields(
            &mut acc,
            &api_key,
            provider_config.clone(),
            api_model_catalog.clone(),
            sync_model_catalog_to_codex,
            api_wire_api.clone(),
            api_supports_websockets,
            api_supports_vision,
            api_model_vision_support.clone(),
            api_vision_routing_model.clone(),
        );
        if acc.email.trim().is_empty() {
            acc.email = build_api_key_email(&api_key);
        }
        if let Some(name) = account_name.clone() {
            if normalize_optional_ref(acc.account_name.as_deref()).is_none() {
                acc.account_name = Some(name);
            }
        }
        acc.update_last_used();
        acc
    } else {
        let mut acc = CodexAccount::new_api_key(
            account_id.clone(),
            build_api_key_email(&api_key),
            api_key,
            provider_config.mode.clone(),
            provider_config.base_url.clone(),
            provider_config.provider_id.clone(),
            provider_config.provider_name.clone(),
            normalize_api_model_catalog(api_model_catalog.clone()),
        );
        acc.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        acc.account_name = account_name;
        acc.api_sync_model_catalog_to_codex = api_sync_model_catalog_to_codex.unwrap_or(false);
        acc.api_wire_api = normalize_api_wire_api(api_wire_api.clone());
        acc.api_supports_websockets = api_supports_websockets;
        let _ = normalize_api_key_websocket_capability(&mut acc);
        acc.api_supports_vision = api_supports_vision;
        acc.api_model_vision_support = normalize_api_model_vision_support(api_model_vision_support);
        acc.api_vision_routing_model = normalize_optional_value(api_vision_routing_model);
        acc
    };

    account.auth_mode = CodexAuthMode::Apikey;
    save_account(&account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex API Key 账号已保存: account_id={}, email={}, has_base_url={}",
        account.id,
        account.email,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));
    Ok(account)
}

fn upsert_account_with_hints(
    tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
) -> Result<CodexAccount, String> {
    upsert_account_with_hints_and_reauth_target(tokens, account_id_hint, organization_id_hint, None)
}

fn resolve_reauth_target_account_id(
    target_account_id: Option<&str>,
    email: &str,
) -> Result<Option<String>, String> {
    let Some(target_id) = normalize_optional_ref(target_account_id) else {
        return Ok(None);
    };
    let target =
        load_account(&target_id).ok_or_else(|| format!("重新授权目标账号不存在: {}", target_id))?;
    if target.is_api_key_auth() {
        return Err("API Key 账号不能通过 OAuth 重新授权".to_string());
    }
    if !target.email.trim().is_empty() && !target.email.eq_ignore_ascii_case(email) {
        return Err(format!(
            "重新授权账号邮箱不匹配: 目标账号为 {}，本次授权为 {}",
            target.email, email
        ));
    }
    Ok(Some(if target.id.trim().is_empty() {
        target_id
    } else {
        target.id
    }))
}

fn upsert_account_with_hints_and_reauth_target(
    mut tokens: CodexTokens,
    account_id_hint: Option<String>,
    organization_id_hint: Option<String>,
    reauth_target_account_id: Option<&str>,
) -> Result<CodexAccount, String> {
    let (
        email,
        user_id,
        plan_type,
        subscription_active_until,
        id_token_account_id,
        id_token_org_id,
    ) = extract_user_info(&tokens.id_token)?;
    let account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&tokens.access_token)
            .or(id_token_account_id)
            .or(account_id_hint),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token)
            .or(id_token_org_id)
            .or(organization_id_hint),
    );

    let mut index = load_account_index();
    let generated_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let has_reauth_target = normalize_optional_ref(reauth_target_account_id).is_some();

    // 明确的重新授权来自某个旧账号卡片，必须优先覆盖该旧账号。
    let existing_id = resolve_reauth_target_account_id(reauth_target_account_id, &email)?
        .or_else(|| {
            find_existing_account_id(
                &index,
                &email,
                account_id.as_deref(),
                organization_id.as_deref(),
            )
        })
        .unwrap_or_else(|| generated_id.clone());

    let account = if let Some(mut acc) = load_account(&existing_id) {
        // 更新现有账号
        tokens = retain_existing_refresh_token_if_missing(tokens, Some(&acc));
        acc.tokens = tokens;
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.update_last_used();
        acc
    } else {
        // 创建新账号
        tokens = retain_existing_refresh_token_if_missing(tokens, None);
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            subscription_active_until: subscription_active_until.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    if has_reauth_target && generated_id != account.id {
        let removed_duplicate = index.accounts.iter().any(|item| item.id == generated_id);
        if removed_duplicate {
            index.accounts.retain(|item| item.id != generated_id);
            if index.current_account_id.as_deref() == Some(generated_id.as_str()) {
                index.current_account_id = Some(account.id.clone());
            }
            if let Err(err) = delete_account_file(&generated_id) {
                logger::log_warn(&format!(
                    "清理 Codex 重新授权重复账号详情失败: duplicate_id={}, target_id={}, error={}",
                    generated_id, account.id, err
                ));
            } else {
                logger::log_info(&format!(
                    "已清理 Codex 重新授权重复账号: duplicate_id={}, target_id={}",
                    generated_id, account.id
                ));
            }
        }
    }

    // 保存账号详情
    save_account(&account)?;

    // 更新索引中的摘要信息
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    Ok(account)
}

/// 更新索引中账号的 plan_type（供配额刷新时同步订阅标识）
pub fn update_account_plan_type_in_index(
    account_id: &str,
    plan_type: &Option<String>,
    subscription_active_until: &Option<String>,
) -> Result<(), String> {
    let mut index = load_account_index();
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account_id) {
        summary.plan_type = plan_type.clone();
        summary.subscription_active_until = subscription_active_until.clone();
        save_account_index(&index)?;
    }
    Ok(())
}

/// 删除账号
pub fn remove_account(account_id: &str) -> Result<(), String> {
    remove_accounts(&[account_id.to_string()])
}

/// 批量删除账号
pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    let remove_ids: HashSet<String> = account_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if remove_ids.is_empty() {
        return Ok(());
    }

    let mut index = load_account_index();
    let accounts_dir = get_accounts_dir();
    let mut missing_detail_ids = HashSet::new();
    index.accounts.retain(|account| {
        if remove_ids.contains(&account.id) {
            return false;
        }
        if !accounts_dir.join(format!("{}.json", account.id)).exists() {
            missing_detail_ids.insert(account.id.clone());
            return false;
        }
        true
    });
    if !missing_detail_ids.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account] 删除账号时清理缺失详情文件的孤儿索引: {}",
            missing_detail_ids
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if index
        .current_account_id
        .as_ref()
        .map(|current_id| {
            remove_ids.contains(current_id) || missing_detail_ids.contains(current_id)
        })
        .unwrap_or(false)
    {
        index.current_account_id = None;
    }
    save_account_index(&index)?;

    for account_id in remove_ids {
        delete_account_file(&account_id)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct LocalCodexOAuthSnapshot {
    tokens: CodexTokens,
    email: String,
    subscription_active_until: Option<String>,
    account_id: Option<String>,
    organization_id: Option<String>,
    last_refresh_at: Option<i64>,
}

fn parse_auth_file_last_refresh(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(raw) = value.as_i64() {
        return Some(if raw > 1_000_000_000_000 {
            raw / 1000
        } else {
            raw
        });
    }
    if let Some(raw) = value.as_u64() {
        let normalized = if raw > 1_000_000_000_000 {
            raw / 1000
        } else {
            raw
        };
        return i64::try_from(normalized).ok();
    }

    let raw = value.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.timestamp());
    }
    if let Ok(parsed) = raw.parse::<i64>() {
        return Some(if parsed > 1_000_000_000_000 {
            parsed / 1000
        } else {
            parsed
        });
    }

    None
}

fn build_local_oauth_snapshot(tokens: CodexAuthTokens) -> Option<LocalCodexOAuthSnapshot> {
    let (email, _, _, subscription_active_until, id_token_account_id, id_token_org_id) =
        extract_user_info(&tokens.id_token).ok()?;
    let account_id = normalize_optional_value(
        tokens
            .account_id
            .clone()
            .or_else(|| extract_chatgpt_account_id_from_access_token(&tokens.access_token))
            .or(id_token_account_id),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token).or(id_token_org_id),
    );

    Some(LocalCodexOAuthSnapshot {
        tokens: CodexTokens {
            id_token: tokens.id_token,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
        },
        email,
        subscription_active_until,
        account_id,
        organization_id,
        last_refresh_at: None,
    })
}

fn read_codex_auth_file_from_dir(base_dir: &Path) -> Option<CodexAuthFile> {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&auth_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn load_local_oauth_snapshot_from_auth_file(
    auth_file: CodexAuthFile,
) -> Option<LocalCodexOAuthSnapshot> {
    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        return None;
    }

    let last_refresh_at = parse_auth_file_last_refresh(auth_file.last_refresh.as_ref());
    let mut snapshot = build_local_oauth_snapshot(auth_file.tokens?)?;
    snapshot.last_refresh_at = last_refresh_at;
    Some(snapshot)
}

#[cfg(all(target_os = "macos", not(test)))]
fn is_codex_keychain_item_not_found(status: std::process::ExitStatus, stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    status.code() == Some(44)
        || lower.contains("could not be found")
        || lower.contains("errsecitemnotfound")
        || lower.contains("specified item could not be found")
}

#[cfg(all(target_os = "macos", not(test)))]
fn read_codex_keychain_secret_from_dir(base_dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let keychain_account = build_codex_keychain_account(base_dir);
    let output = std::process::Command::new("security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .arg("-w")
        .output()
        .map_err(|e| format!("执行 security 命令失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_codex_keychain_item_not_found(output.status, &stderr) {
            return Ok(None);
        }
        return Err(format!(
            "读取 Codex keychain 失败: status={}, stderr={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            }
        ));
    }

    let mut secret = output.stdout;
    while matches!(secret.last(), Some(b'\n' | b'\r')) {
        secret.pop();
    }
    if secret.is_empty() {
        return Ok(None);
    }
    Ok(Some(secret))
}

#[cfg(all(target_os = "macos", not(test)))]
fn read_codex_keychain_auth_file_from_dir(
    base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    let Some(secret) = read_codex_keychain_secret_from_dir(base_dir)? else {
        return Ok(None);
    };
    let secret = String::from_utf8(secret)
        .map_err(|e| format!("解析 Codex keychain 数据失败: {}", e))?;
    let auth_file: CodexAuthFile = serde_json::from_str(&secret)
        .map_err(|e| format!("解析 Codex keychain JSON 失败: {}", e))?;
    Ok(Some(auth_file))
}

#[cfg(all(target_os = "macos", test))]
static CODEX_TEST_KEYCHAIN_VALUES: std::sync::LazyLock<Mutex<HashMap<String, Option<Vec<u8>>>>>
    = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(all(target_os = "macos", test))]
fn test_codex_keychain_key(base_dir: &Path) -> String {
    base_dir.to_string_lossy().into_owned()
}

#[cfg(all(target_os = "macos", test))]
fn read_codex_keychain_secret_from_dir(base_dir: &Path) -> Result<Option<Vec<u8>>, String> {
    if std::env::var_os("COCKPIT_TEST_FAIL_CODEX_KEYCHAIN_READ").is_some() {
        return Err("模拟 Codex keychain 读取失败".to_string());
    }
    Ok(CODEX_TEST_KEYCHAIN_VALUES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&test_codex_keychain_key(base_dir))
        .cloned()
        .flatten())
}

#[cfg(all(target_os = "macos", test))]
fn read_codex_keychain_auth_file_from_dir(
    base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    let Some(secret) = read_codex_keychain_secret_from_dir(base_dir)? else {
        return Ok(None);
    };
    let secret = String::from_utf8(secret)
        .map_err(|e| format!("解析 Codex keychain 数据失败: {}", e))?;
    serde_json::from_str(&secret)
        .map(Some)
        .map_err(|e| format!("解析 Codex keychain JSON 失败: {}", e))
}

#[cfg(not(target_os = "macos"))]
fn read_codex_keychain_auth_file_from_dir(
    _base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    Ok(None)
}

fn load_local_oauth_snapshot_from_official_store(
    base_dir: &Path,
) -> Option<LocalCodexOAuthSnapshot> {
    let auth_json = read_codex_auth_file_from_dir(base_dir);
    if auth_json
        .as_ref()
        .map(|auth_file| is_auth_mode_apikey(auth_file.auth_mode.as_deref()))
        .unwrap_or(false)
    {
        return None;
    }

    match read_codex_keychain_auth_file_from_dir(base_dir) {
        Ok(Some(auth_file)) => {
            if let Some(snapshot) = load_local_oauth_snapshot_from_auth_file(auth_file) {
                return Some(snapshot);
            }
        }
        Ok(None) => {}
        Err(err) => {
            logger::log_warn(&format!(
                "读取 Codex 官方 keychain 凭证失败，回退读取 auth.json: target_dir={}, error={}",
                base_dir.display(),
                err
            ));
        }
    }

    auth_json.and_then(load_local_oauth_snapshot_from_auth_file)
}

fn local_oauth_snapshot_matches_account(
    snapshot: &LocalCodexOAuthSnapshot,
    account: &CodexAccount,
) -> bool {
    if !account.email.eq_ignore_ascii_case(&snapshot.email) {
        return false;
    }

    let expected_id = build_account_storage_id(
        &snapshot.email,
        snapshot.account_id.as_deref(),
        snapshot.organization_id.as_deref(),
    );
    if account.id == expected_id {
        return true;
    }

    if let Some(account_id) = snapshot.account_id.as_deref() {
        if normalize_optional_ref(account.account_id.as_deref()).as_deref() != Some(account_id) {
            return false;
        }
    }

    if let Some(organization_id) = snapshot.organization_id.as_deref() {
        if normalize_optional_ref(account.organization_id.as_deref()).as_deref()
            != Some(organization_id)
        {
            return false;
        }
    }

    true
}

fn apply_local_oauth_snapshot(
    account: &mut CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    let mut changed = false;
    let mut token_changed = false;

    if account.tokens.id_token != snapshot.tokens.id_token {
        account.tokens.id_token = snapshot.tokens.id_token.clone();
        changed = true;
        token_changed = true;
    }

    if account.tokens.access_token != snapshot.tokens.access_token {
        account.tokens.access_token = snapshot.tokens.access_token.clone();
        changed = true;
        token_changed = true;
    }

    if let Some(refresh_token) = normalize_optional_ref(snapshot.tokens.refresh_token.as_deref()) {
        if account.tokens.refresh_token.as_deref() != Some(refresh_token.as_str()) {
            account.tokens.refresh_token = Some(refresh_token);
            changed = true;
            token_changed = true;
        }
    }

    if normalize_optional_ref(account.account_id.as_deref()) != snapshot.account_id {
        account.account_id = snapshot.account_id.clone();
        changed = true;
    }

    if normalize_optional_ref(account.organization_id.as_deref()) != snapshot.organization_id {
        account.organization_id = snapshot.organization_id.clone();
        changed = true;
    }

    if normalize_optional_ref(account.subscription_active_until.as_deref())
        != snapshot.subscription_active_until
    {
        account.subscription_active_until = snapshot.subscription_active_until.clone();
        changed = true;
    }

    if token_changed {
        mark_token_chain_updated(account);
    }

    changed
}

fn local_oauth_snapshot_has_token_delta(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    account.tokens.id_token != snapshot.tokens.id_token
        || account.tokens.access_token != snapshot.tokens.access_token
        || normalize_optional_ref(account.tokens.refresh_token.as_deref())
            != normalize_optional_ref(snapshot.tokens.refresh_token.as_deref())
}

fn should_accept_authority_snapshot(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    if !local_oauth_snapshot_has_token_delta(account, snapshot) {
        return false;
    }

    let account_updated_at = account.token_updated_at.unwrap_or(0);
    if snapshot
        .last_refresh_at
        .map(|value| value >= account_updated_at)
        .unwrap_or(false)
    {
        return true;
    }

    codex_oauth::is_token_expired(&account.tokens.access_token)
        && !codex_oauth::is_token_expired(&snapshot.tokens.access_token)
}

fn sync_account_from_authority_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        return Ok(false);
    }

    if !should_accept_authority_snapshot(account, &snapshot) {
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        logger::log_info(&format!(
            "Codex 账号刷新前已采用更近的官方凭证: account_id={}, source_dir={}, last_refresh_at={}",
            account.id,
            base_dir.display(),
            snapshot
                .last_refresh_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        return Ok(true);
    }

    Ok(false)
}

fn sync_account_from_authority_sources(account: &mut CodexAccount) -> Result<bool, String> {
    let mut dirs = vec![get_codex_home()];
    dirs.extend(managed_projection_dirs_for_account(&account.id));

    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.to_string_lossy().to_string()));

    let mut changed = false;
    for dir in dirs {
        if sync_account_from_authority_dir_if_current(account, &dir)? {
            changed = true;
        }
    }
    Ok(changed)
}

fn sync_account_from_auth_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        logger::log_info(&format!(
            "Codex 账号已从官方凭证源同步最新 Token: account_id={}, source_dir={}",
            account.id,
            base_dir.display()
        ));
    }

    Ok(true)
}

/// 显式导入/同步入口：只在用户主动选择从官方目录回读时使用，业务主路径禁止自动调用。
pub fn sync_current_official_account_from_dir(
    base_dir: &Path,
) -> Result<Option<CodexAccount>, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        return Ok(None);
    };

    for mut account in list_accounts() {
        if account.is_api_key_auth() {
            continue;
        }
        if !local_oauth_snapshot_matches_account(&snapshot, &account) {
            continue;
        }

        if apply_local_oauth_snapshot(&mut account, &snapshot) {
            save_account(&account)?;
            logger::log_info(&format!(
                "Codex 当前官方凭证已同步回账号库: account_id={}, source_dir={}",
                account.id,
                base_dir.display()
            ));
        }
        return Ok(Some(account));
    }

    Ok(None)
}

/// 显式导入/同步入口：只在用户主动选择从指定目录回读时使用，业务主路径禁止自动调用。
pub fn sync_account_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }

    let _ = sync_account_from_auth_dir_if_current(&mut account, base_dir)?;
    Ok(account)
}

pub fn sync_managed_projection_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let projection = read_managed_projection_from_dir(base_dir)
        .ok_or_else(|| "目标目录不是 Cockpit 受管 Codex 投影，已拒绝反向同步".to_string())?;
    if projection.account_id != account_id {
        return Err(format!(
            "受管投影账号不匹配: expected={}, actual={}",
            account_id, projection.account_id
        ));
    }

    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }
    if account.token_generation != projection.token_generation {
        return Err(format!(
            "受管投影版本已过期，跳过反向同步: account_id={}, store_generation={}, projection_generation={}",
            account_id, account.token_generation, projection.token_generation
        ));
    }

    let snapshot = load_local_oauth_snapshot_from_official_store(base_dir)
        .ok_or_else(|| "受管投影缺少可同步的 OAuth Token".to_string())?;
    if !local_oauth_snapshot_matches_account(&snapshot, &account) {
        return Err("受管投影 Token 与账号不匹配，已拒绝反向同步".to_string());
    }

    if apply_local_oauth_snapshot(&mut account, &snapshot) {
        save_account(&account)?;
        write_prepared_account_bundle_to_dir(base_dir, &account)?;
        write_managed_account_projections(&account);
        logger::log_info(&format!(
            "Codex 受管投影已同步回账号库: account_id={}, generation={}, source_dir={}",
            account.id,
            account.token_generation,
            base_dir.display()
        ));
    }

    Ok(account)
}

/// Local API Service / loopback client URLs must not overwrite a stored real upstream.
fn is_loopback_or_local_gateway_base_url(raw: Option<&str>) -> bool {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]"
    )
}

fn is_loopback_http_base_url(raw: Option<&str>) -> bool {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.")
        }
        None => false,
    }
}

fn sync_api_key_account_from_local_state(account: &mut CodexAccount, base_dir: &Path) {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() || !account.is_api_key_auth() {
        return;
    }

    let Ok(content) = fs::read_to_string(&auth_path) else {
        return;
    };
    let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(&content) else {
        return;
    };
    let is_apikey_mode = is_auth_mode_apikey(auth_file.auth_mode.as_deref());
    let local_api_key = extract_api_key_from_auth_file(&auth_file);
    if !(is_apikey_mode || (auth_file.tokens.is_none() && local_api_key.is_some())) {
        return;
    }

    let Some(local_api_key) = normalize_optional_ref(local_api_key.as_deref()) else {
        return;
    };
    let Some(account_api_key) = normalize_optional_ref(account.openai_api_key.as_deref()) else {
        return;
    };
    if local_api_key != account_api_key {
        return;
    }

    let config_provider = read_api_provider_from_config_toml(base_dir);
    // Local access / provider gateway profiles rewrite client base_url to loopback.
    // Never treat that runtime endpoint as the account's real upstream provider URL,
    // or sidecar codex-api-key base-url will form a self-proxy loop after switch.
    let using_runtime_local_provider = config_provider.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        || is_loopback_http_base_url(config_provider.base_url.as_deref());
    if using_runtime_local_provider {
        return;
    }

    let provider_mode = config_provider.mode.clone();
    let provider_id = config_provider.provider_id.as_deref();
    let provider_name = config_provider.provider_name.as_deref();
    let resolved_base_url = extract_api_base_url_from_auth_file(&auth_file)
        .or_else(|| config_provider.base_url.clone());
    if is_loopback_http_base_url(resolved_base_url.as_deref()) {
        return;
    }
    let current_provider = infer_api_provider_config(
        resolved_base_url.as_deref(),
        Some(provider_mode),
        provider_id,
        provider_name,
    );
    let account_provider = infer_api_provider_config(
        account.api_base_url.as_deref(),
        Some(account.api_provider_mode.clone()),
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    );

    if account_provider == current_provider {
        return;
    }

    // Profile after local API attach uses localhost as the *client* Base URL.
    // Never write that back as the account's real upstream (breaks sidecar).
    if is_loopback_or_local_gateway_base_url(current_provider.base_url.as_deref()) {
        return;
    }

    account.api_base_url = current_provider.base_url.clone();
    account.api_provider_mode = current_provider.mode.clone();
    account.api_provider_id = current_provider.provider_id.clone();
    account.api_provider_name = current_provider.provider_name.clone();
    let _ = save_account(account);
}

/// 获取当前激活的账号（基于 Tools 显式 current_account_id）
pub fn get_current_account() -> Option<CodexAccount> {
    let base_dir = get_codex_home();
    get_current_account_from_loaded(
        load_account_index(),
        |account_id| load_account(account_id),
        &base_dir,
    )
}

fn get_current_account_from_loaded(
    index: CodexAccountIndex,
    mut load: impl FnMut(&str) -> Option<CodexAccount>,
    base_dir: &Path,
) -> Option<CodexAccount> {
    let current_id = index.current_account_id?;
    let mut account = load(&current_id)?;

    if account.is_api_key_auth() {
        sync_api_key_account_from_local_state(&mut account, base_dir);
    }
    Some(account)
}

fn build_auth_file_value(account: &CodexAccount) -> Result<serde_json::Value, String> {
    if account.is_api_key_auth() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        return Ok(serde_json::json!({
            "auth_mode": API_KEY_AUTH_MODE,
            "OPENAI_API_KEY": api_key,
        }));
    }

    if account.tokens.access_token.trim().is_empty() {
        return Err("OAuth 账号缺少 access_token，无法写入 auth.json".to_string());
    }

    // Access-token-only accounts: prefer official personal_access_token shape
    // (no empty id_token / fabricated refresh) when neither id nor refresh exist.
    if account.tokens.id_token.trim().is_empty()
        && normalize_optional_ref(account.tokens.refresh_token.as_deref()).is_none()
    {
        return Ok(serde_json::json!({
            "OPENAI_API_KEY": null,
            "personal_access_token": account.tokens.access_token,
        }));
    }

    serde_json::to_value(CodexAuthFile {
        auth_mode: None,
        openai_api_key: Some(serde_json::Value::Null),
        base_url: None,
        tokens: Some(CodexAuthTokens {
            id_token: account.tokens.id_token.clone(),
            access_token: account.tokens.access_token.clone(),
            // Codex CLI's auth.json parser requires the refresh_token key to
            // exist even for access-token-only accounts. Use an empty string so
            // Cockpit can switch short-lived opaque `at-...` credentials without
            // inventing a refresh token that would be sent to OAuth refresh.
            refresh_token: Some(
                normalize_optional_ref(account.tokens.refresh_token.as_deref()).unwrap_or_default(),
            ),
            account_id: account.account_id.clone(),
        }),
        personal_access_token: None,
        last_refresh: Some(serde_json::Value::String(
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                .to_string(),
        )),
    })
    .map_err(|e| format!("auth.json 序列化失败: {}", e))
}

#[cfg(all(target_os = "macos", not(test)))]
fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved_home = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let digest_hex = format!("{:x}", digest);
    format!("cli|{}", &digest_hex[..16])
}

#[cfg(all(target_os = "macos", not(test)))]
fn write_codex_keychain_secret_to_dir(base_dir: &Path, secret: &[u8]) -> Result<(), String> {
    let secret = String::from_utf8(secret.to_vec())
        .map_err(|e| format!("Codex keychain 数据不是 UTF-8: {}", e))?;
    let keychain_account = build_codex_keychain_account(base_dir);

    let output = std::process::Command::new("security")
        .arg("add-generic-password")
        .arg("-U")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .arg("-w")
        .arg(&secret)
        .output()
        .map_err(|e| format!("执行 security 命令失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "写入 Codex keychain 失败: status={}, stderr={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            }
        ));
    }

    logger::log_info(&format!(
        "[Codex切号] 已更新 keychain 登录信息: service={}, account={}",
        CODEX_KEYCHAIN_SERVICE, keychain_account
    ));
    Ok(())
}

#[cfg(all(target_os = "macos", not(test)))]
fn write_codex_keychain_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    if account.is_api_key_auth() {
        return Ok(());
    }

    let payload = build_auth_file_value(account)?;
    let secret = serde_json::to_string(&payload)
        .map_err(|e| format!("序列化 Codex keychain 数据失败: {}", e))?;
    write_codex_keychain_secret_to_dir(base_dir, secret.as_bytes())
}

#[cfg(all(target_os = "macos", test))]
fn write_codex_keychain_to_dir(_base_dir: &Path, _account: &CodexAccount) -> Result<(), String> {
    if std::env::var_os("COCKPIT_TEST_FAIL_CODEX_KEYCHAIN").is_some() {
        return Err("模拟 Codex keychain 写入失败".to_string());
    }
    let payload = build_auth_file_value(_account)?;
    let secret = serde_json::to_vec(&payload)
        .map_err(|e| format!("序列化 Codex keychain 数据失败: {}", e))?;
    CODEX_TEST_KEYCHAIN_VALUES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(test_codex_keychain_key(_base_dir), Some(secret));
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn write_codex_keychain_to_dir(_base_dir: &Path, _account: &CodexAccount) -> Result<(), String> {
    Ok(())
}

fn is_disk_full_io_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(28) | Some(112))
}

fn is_disk_full_error_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("disk_full:")
        || lower.contains("os error 28")
        || lower.contains("os error 112")
        || lower.contains("no space left on device")
        || lower.contains("not enough space on the disk")
        || lower.contains("磁盘空间不足")
}

fn format_io_error(action: &str, path: &Path, error: &std::io::Error) -> String {
    if is_disk_full_io_error(error) {
        return format!(
            "{}:{}失败: path={}, 磁盘空间不足，请清理磁盘后重试",
            DISK_FULL_ERROR_CODE,
            action,
            path.display()
        );
    }
    format!("{}失败: path={}, error={}", action, path.display(), error)
}

fn build_temp_file_path(parent: &Path, target: &Path, suffix: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{}.tmp.{}.{}.{}",
        target
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("file"),
        std::process::id(),
        unique,
        suffix
    ))
}

fn write_string_atomic(path: &Path, content: &str) -> Result<(), String> {
    crate::modules::atomic_write::write_string_atomic(path, content)
}

fn build_managed_projection(account: &CodexAccount) -> CodexManagedAuthProjection {
    CodexManagedAuthProjection {
        version: 1,
        writer: CODEX_AUTH_PROJECTION_WRITER.to_string(),
        account_id: account.id.clone(),
        email: account.email.clone(),
        token_generation: account.token_generation,
        written_at: now_timestamp(),
    }
}

fn projection_path_for_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEX_AUTH_PROJECTION_FILE_NAME)
}

fn write_managed_projection_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    let projection = build_managed_projection(account);
    let content = serde_json::to_string_pretty(&projection)
        .map_err(|e| format!("受管投影序列化失败: {}", e))?;
    write_string_atomic(&projection_path_for_dir(base_dir), &content)
        .map_err(|e| format!("写入受管投影失败: {}", e))
}

fn read_managed_projection_from_dir(base_dir: &Path) -> Option<CodexManagedAuthProjection> {
    let path = projection_path_for_dir(base_dir);
    let content = fs::read_to_string(path).ok()?;
    let projection: CodexManagedAuthProjection = serde_json::from_str(&content).ok()?;
    if projection.writer == CODEX_AUTH_PROJECTION_WRITER {
        Some(projection)
    } else {
        None
    }
}

pub fn read_managed_projection_account_id_from_dir(base_dir: &Path) -> Option<String> {
    read_managed_projection_from_dir(base_dir).map(|projection| projection.account_id)
}

fn ensure_directory_writable_for_import(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format_io_error("创建导入目录", path, &e))?;
    let probe_path = build_temp_file_path(path, path, "import-probe");
    fs::write(&probe_path, b"probe")
        .map_err(|e| format_io_error("导入前磁盘写入预检", &probe_path, &e))?;
    fs::remove_file(&probe_path).map_err(|e| {
        format!(
            "导入预检清理失败: path={}, error={}",
            probe_path.display(),
            e
        )
    })?;
    Ok(())
}

fn ensure_storage_writable_for_import() -> Result<(), String> {
    let accounts_dir = get_accounts_dir();
    ensure_directory_writable_for_import(&accounts_dir)?;

    let index_path = get_accounts_storage_path();
    let index_dir = index_path
        .parent()
        .ok_or_else(|| format!("无法定位索引目录: {}", index_path.display()))?;
    ensure_directory_writable_for_import(index_dir)?;
    Ok(())
}

pub fn write_auth_file_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    let auth_path = base_dir.join("auth.json");
    logger::log_info(&format!(
        "[Codex切号] 准备写入登录信息: account_id={}, email={}, target_dir={}, target_file={}",
        account.id,
        account.email,
        base_dir.display(),
        auth_path.display()
    ));

    crate::modules::codex_local_access::cleanup_provider_gateway_profile_model_overrides(base_dir)?;

    let auth_file = build_auth_file_value(account)?;
    let content =
        serde_json::to_string_pretty(&auth_file).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&auth_path, &content).map_err(|e| {
        format!(
            "写入 auth.json 失败: path={}, error={}",
            auth_path.display(),
            e
        )
    })?;

    let provider_config = if account.is_api_key_auth() {
        let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
            .ok_or_else(|| "API Key 账号缺少 OPENAI_API_KEY".to_string())?;
        let provider_config = infer_api_provider_config(
            account.api_base_url.as_deref(),
            Some(account.api_provider_mode.clone()),
            account.api_provider_id.as_deref(),
            account.api_provider_name.as_deref(),
        );
        let supports_image = api_key_provider_should_enable_imagegen(account, &provider_config);
        write_api_key_provider_to_config_toml(
            base_dir,
            &provider_config,
            &api_key,
            account.api_provider_mode == CodexApiProviderMode::Custom
                && account.api_supports_websockets,
            supports_image,
            // 纯 API Key：有生图时关闭 openai auth 门，走 bearer + actor。
            !supports_image,
        )?;
        provider_config
    } else {
        let provider_config = ApiProviderConfig {
            mode: CodexApiProviderMode::OpenaiBuiltin,
            base_url: None,
            provider_id: None,
            provider_name: None,
        };
        write_api_provider_to_config_toml(base_dir, &provider_config)?;
        provider_config
    };

    logger::log_info(&format!(
        "[Codex切号] 已写入登录信息: account_id={}, target_file={}, has_base_url={}",
        account.id,
        auth_path.display(),
        provider_config.base_url.is_some()
    ));

    Ok(())
}

fn resolve_account_for_bundle_write(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<CodexAccount, String> {
    let _ = base_dir;
    Ok(account.clone())
}

#[derive(Debug, Clone)]
struct CodexBundleFileSnapshot {
    path: PathBuf,
    content: Option<Vec<u8>>,
}

fn snapshot_codex_bundle_files(base_dir: &Path) -> Result<Vec<CodexBundleFileSnapshot>, String> {
    let mut snapshots = Vec::new();
    for path in [
        base_dir.join("auth.json"),
        base_dir.join(CODEX_CONFIG_FILE_NAME),
        base_dir.join(CODEX_MANAGED_MODEL_CATALOG_FILE),
        base_dir.join(CODEX_AUTH_PROJECTION_FILE_NAME),
    ] {
        let content = match fs::read(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "读取认证文件快照失败: path={}, error={}",
                    path.display(), error
                ));
            }
        };
        snapshots.push(CodexBundleFileSnapshot { path, content });
    }
    Ok(snapshots)
}

#[derive(Debug, Clone)]
struct CodexKeychainSnapshot {
    base_dir: PathBuf,
    secret: Option<Vec<u8>>,
}

#[cfg(all(target_os = "macos", not(test)))]
fn snapshot_codex_keychain_for_dir(
    base_dir: &Path,
) -> Result<Option<CodexKeychainSnapshot>, String> {
    Ok(Some(CodexKeychainSnapshot {
        base_dir: base_dir.to_path_buf(),
        secret: read_codex_keychain_secret_from_dir(base_dir)?,
    }))
}

#[cfg(all(target_os = "macos", test))]
fn snapshot_codex_keychain_for_dir(
    base_dir: &Path,
) -> Result<Option<CodexKeychainSnapshot>, String> {
    Ok(Some(CodexKeychainSnapshot {
        base_dir: base_dir.to_path_buf(),
        secret: read_codex_keychain_secret_from_dir(base_dir)?,
    }))
}

#[cfg(not(target_os = "macos"))]
fn snapshot_codex_keychain_for_dir(
    _base_dir: &Path,
) -> Result<Option<CodexKeychainSnapshot>, String> {
    Ok(None)
}

#[cfg(all(target_os = "macos", not(test)))]
fn restore_codex_keychain_snapshot(snapshot: &CodexKeychainSnapshot) -> Result<(), String> {
    let keychain_account = build_codex_keychain_account(&snapshot.base_dir);
    let Some(secret) = snapshot.secret.as_deref() else {
        let output = std::process::Command::new("security")
            .arg("delete-generic-password")
            .arg("-s")
            .arg(CODEX_KEYCHAIN_SERVICE)
            .arg("-a")
            .arg(&keychain_account)
            .output()
            .map_err(|e| format!("执行 security 命令失败: {}", e))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_codex_keychain_item_not_found(output.status, &stderr) {
            return Ok(());
        }
        return Err(format!(
            "恢复 Codex keychain 不存在状态失败: status={}, stderr={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            }
        ));
    };

    write_codex_keychain_secret_to_dir(&snapshot.base_dir, secret)
}

#[cfg(all(target_os = "macos", test))]
fn restore_codex_keychain_snapshot(snapshot: &CodexKeychainSnapshot) -> Result<(), String> {
    CODEX_TEST_KEYCHAIN_VALUES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(
            test_codex_keychain_key(&snapshot.base_dir),
            snapshot.secret.clone(),
        );
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn restore_codex_keychain_snapshot(_snapshot: &CodexKeychainSnapshot) -> Result<(), String> {
    Ok(())
}

#[derive(Debug, Clone)]
struct CodexBundleSnapshot {
    files: Vec<CodexBundleFileSnapshot>,
    keychain: Option<CodexKeychainSnapshot>,
}

fn snapshot_codex_bundle(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<CodexBundleSnapshot, String> {
    let files = snapshot_codex_bundle_files(base_dir)?;
    let keychain = if account.is_api_key_auth() {
        None
    } else {
        snapshot_codex_keychain_for_dir(base_dir)?
    };
    Ok(CodexBundleSnapshot { files, keychain })
}

fn restore_codex_bundle_files(snapshots: &[CodexBundleFileSnapshot]) -> Result<(), String> {
    let mut errors = Vec::new();
    for snapshot in snapshots {
        let result = match snapshot.content.as_deref() {
            Some(content) => {
                crate::modules::atomic_write::write_bytes_atomic(&snapshot.path, content)
            }
            None => crate::modules::atomic_write::remove_file_locked(&snapshot.path).map(|_| ()),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {}", snapshot.path.display(), error));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn restore_codex_bundle(snapshot: &CodexBundleSnapshot) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = restore_codex_bundle_files(&snapshot.files) {
        errors.push(format!("认证文件: {}", error));
    }
    if let Some(keychain) = snapshot.keychain.as_ref() {
        if let Err(error) = restore_codex_keychain_snapshot(keychain) {
            errors.push(format!("Keychain: {}", error));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn codex_bundle_error_with_rollback(reason: String, snapshot: &CodexBundleSnapshot) -> String {
    match restore_codex_bundle(snapshot) {
        Ok(()) => reason,
        Err(rollback_error) => format!(
            "{}；切号事务回滚失败，请勿继续启动 Codex: {}",
            reason, rollback_error
        ),
    }
}

#[derive(Debug, Clone)]
struct CodexAccountStoreSnapshot {
    files: Vec<CodexBundleFileSnapshot>,
}

fn snapshot_codex_account_store(
    index: &CodexAccountIndex,
    target_account_id: &str,
) -> Result<CodexAccountStoreSnapshot, String> {
    let mut paths = vec![get_accounts_storage_path()];
    let mut account_ids = Vec::new();
    if let Some(current_account_id) = index.current_account_id.as_deref() {
        account_ids.push(current_account_id);
    }
    if !account_ids.contains(&target_account_id) {
        account_ids.push(target_account_id);
    }
    for account_id in account_ids {
        paths.push(get_accounts_dir().join(format!("{}.json", account_id)));
    }

    let mut files = Vec::new();
    for path in paths {
        if files.iter().any(|snapshot: &CodexBundleFileSnapshot| snapshot.path == path) {
            continue;
        }
        let content = match fs::read(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "读取账号库快照失败: path={}, error={}",
                    path.display(), error
                ));
            }
        };
        files.push(CodexBundleFileSnapshot { path, content });
    }
    Ok(CodexAccountStoreSnapshot { files })
}

fn restore_codex_account_store(snapshot: &CodexAccountStoreSnapshot) -> Result<(), String> {
    restore_codex_bundle_files(&snapshot.files)
}

fn codex_switch_error_with_rollback(
    reason: String,
    bundle_snapshot: &CodexBundleSnapshot,
    store_snapshot: &CodexAccountStoreSnapshot,
) -> String {
    let mut errors = Vec::new();
    if let Err(error) = restore_codex_bundle(bundle_snapshot) {
        errors.push(format!("认证状态: {}", error));
    }
    if let Err(error) = restore_codex_account_store(store_snapshot) {
        errors.push(format!("账号库: {}", error));
    }
    if errors.is_empty() {
        reason
    } else {
        format!("{}；切号事务回滚失败，请勿继续启动 Codex: {}", reason, errors.join("; "))
    }
}

pub(crate) fn write_prepared_account_bundle_to_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    let snapshot = snapshot_codex_bundle(base_dir, account)?;
    let disk_result = (|| -> Result<(), String> {
        write_auth_file_to_dir(base_dir, account)?;
        write_managed_projection_to_dir(base_dir, account)?;
        sync_or_cleanup_managed_model_catalog_for_dir(base_dir, account)?;
        Ok(())
    })();
    if let Err(error) = disk_result {
        return Err(codex_bundle_error_with_rollback(
            format!("写入 Codex 登录文件失败，已回滚认证文件: {}", error),
            &snapshot,
        ));
    }

    if let Err(error) = write_codex_keychain_to_dir(base_dir, account) {
        return Err(codex_bundle_error_with_rollback(
            format!(
                "写入 Codex keychain 失败，已回滚认证文件并阻止向上报告切号成功，以避免 ChatGPT/Codex 继续读取旧登录态: {}",
                error
            ),
            &snapshot,
        ));
    }
    Ok(())
}

fn validate_api_key_bound_oauth_account(
    api_key_account: &CodexAccount,
    bound_oauth_account_id: &str,
) -> Result<CodexAccount, String> {
    if !api_key_account.is_api_key_auth() {
        return Err("仅 API Key 账号支持绑定 OAuth 账号".to_string());
    }

    let bound_id = normalize_optional_ref(Some(bound_oauth_account_id))
        .ok_or_else(|| "请选择要绑定的 OAuth 账号".to_string())?;
    if bound_id == api_key_account.id {
        return Err("API Key 账号不能绑定自身".to_string());
    }

    let oauth_account =
        load_account(&bound_id).ok_or_else(|| format!("绑定的 OAuth 账号不存在: {}", bound_id))?;
    if oauth_account.is_api_key_auth() {
        return Err("只能绑定 OAuth 账号，不能绑定 API Key 账号".to_string());
    }
    if !account_has_refresh_token(&oauth_account) {
        return Err("只能绑定带 refresh_token 的 OAuth 账号".to_string());
    }

    Ok(oauth_account)
}

fn load_optional_bound_oauth_account_for_api_key(
    api_key_account: &CodexAccount,
) -> Result<Option<CodexAccount>, String> {
    let Some(bound_id) = normalize_optional_ref(api_key_account.bound_oauth_account_id.as_deref())
    else {
        return Ok(None);
    };
    validate_api_key_bound_oauth_account(api_key_account, &bound_id).map(Some)
}

fn write_api_key_provider_override_to_config_toml(
    base_dir: &Path,
    api_key_account: &CodexAccount,
) -> Result<ApiProviderConfig, String> {
    let api_key = normalize_api_key(
        api_key_account
            .openai_api_key
            .as_deref()
            .unwrap_or_default(),
    )
    .ok_or_else(|| "API Key 账号缺少 OPENAI_API_KEY".to_string())?;
    let provider_config = infer_api_provider_config(
        api_key_account.api_base_url.as_deref(),
        Some(api_key_account.api_provider_mode.clone()),
        api_key_account.api_provider_id.as_deref(),
        api_key_account.api_provider_name.as_deref(),
    );
    // 绑定 OAuth 一律 requires_openai_auth=true（显示/使用 OAuth 登录态）。
    // 生图：与纯 API Key 同一判定——本地 loopback 始终开；第三方仅目录含 gpt-image-2。
    let supports_image = api_key_provider_should_enable_imagegen(api_key_account, &provider_config);
    write_api_key_provider_to_config_toml(
        base_dir,
        &provider_config,
        &api_key,
        api_key_account.api_supports_websockets,
        supports_image,
        true,
    )?;
    Ok(provider_config)
}

/// 按账号当前模型目录刷新 profile 上的 provider 生图 header（有则写、无则清）。
fn refresh_api_key_provider_projection_in_dir(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if !account.is_api_key_auth() {
        return Ok(());
    }
    if let Some(oauth) = load_optional_bound_oauth_account_for_api_key(account)? {
        if !oauth.tokens.id_token.trim().is_empty() {
            write_api_key_provider_override_to_config_toml(base_dir, account)?;
            return Ok(());
        }
    }
    let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
        .ok_or_else(|| "API Key 账号缺少 OPENAI_API_KEY".to_string())?;
    let provider_config = infer_api_provider_config(
        account.api_base_url.as_deref(),
        Some(account.api_provider_mode.clone()),
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    );
    let supports_image = api_key_provider_should_enable_imagegen(account, &provider_config);
    write_api_key_provider_to_config_toml(
        base_dir,
        &provider_config,
        &api_key,
        account.api_provider_mode == CodexApiProviderMode::Custom
            && account.api_supports_websockets,
        supports_image,
        !supports_image,
    )?;
    Ok(())
}

fn write_api_key_account_bundle_with_oauth_to_dir(
    base_dir: &Path,
    api_key_account: &CodexAccount,
    oauth_account: &CodexAccount,
) -> Result<(), String> {
    if !api_key_account.is_api_key_auth() {
        return Err("仅 API Key 账号支持 OAuth 绑定写入".to_string());
    }
    if oauth_account.is_api_key_auth() {
        return Err("API Key 账号绑定目标必须是 OAuth 账号".to_string());
    }
    if api_key_account.bound_oauth_account_id.as_deref() != Some(oauth_account.id.as_str()) {
        return Err("API Key 账号绑定的 OAuth 账号不匹配".to_string());
    }

    let snapshot = snapshot_codex_bundle(base_dir, oauth_account)?;
    let result = (|| -> Result<(), String> {
        if oauth_account.tokens.id_token.trim().is_empty() {
            write_prepared_account_bundle_to_dir(base_dir, api_key_account)?;
            logger::log_info(&format!(
                "[Codex切号] 已写入 API Key 账号配置，绑定 OAuth 缺少 id_token，跳过 OAuth 登录态投影: api_account_id={}, oauth_account_id={}, target_dir={}",
                api_key_account.id,
                oauth_account.id,
                base_dir.display()
            ));
            return Ok(());
        }

        write_prepared_account_bundle_to_dir(base_dir, oauth_account)?;
        let provider_config =
            write_api_key_provider_override_to_config_toml(base_dir, api_key_account)?;
        write_managed_projection_to_dir(base_dir, api_key_account)?;
        sync_or_cleanup_managed_model_catalog_for_dir(base_dir, api_key_account)?;
        logger::log_info(&format!(
            "[Codex切号] 已写入 API Key 账号绑定 OAuth 的组合配置: api_account_id={}, oauth_account_id={}, target_dir={}, has_base_url={}",
            api_key_account.id,
            oauth_account.id,
            base_dir.display(),
            provider_config.base_url.is_some()
        ));
        Ok(())
    })();

    result.map_err(|error| {
        codex_bundle_error_with_rollback(
            format!("写入 API Key 绑定 OAuth 组合配置失败: {}", error),
            &snapshot,
        )
    })
}

pub fn write_account_bundle_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    if account.is_api_key_auth() {
        if let Some(oauth_account) = load_optional_bound_oauth_account_for_api_key(account)? {
            return write_api_key_account_bundle_with_oauth_to_dir(
                base_dir,
                account,
                &oauth_account,
            );
        }
        return write_prepared_account_bundle_to_dir(base_dir, account);
    }

    let account = resolve_account_for_bundle_write(base_dir, account)?;
    write_prepared_account_bundle_to_dir(base_dir, &account)
}

/// File entry inside a remote Codex projection bundle (#1404 full SSH sync).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodexProjectionFile {
    pub relative_path: String,
    pub content: String,
    pub mode: u32,
    pub sha256: String,
}

/// Remote-safe Codex account projection (auth.json + config.toml + marker).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodexAccountProjectionBundle {
    pub account_id: String,
    pub account_email: String,
    pub token_generation: u64,
    pub files: Vec<CodexProjectionFile>,
    pub bundle_hash: String,
}

fn sha256_hex_bytes(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

fn build_bundle_hash(files: &[CodexProjectionFile]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(file.sha256.as_bytes());
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

/// Build a remote projection bundle without writing host keychain secrets.
pub(crate) fn build_projection_bundle_for_remote(
    account: &CodexAccount,
    existing_config_toml: Option<&str>,
) -> Result<CodexAccountProjectionBundle, String> {
    let temp_dir = std::env::temp_dir().join(format!(
        "cockpit-codex-remote-bundle-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("创建远程投影临时目录失败: {}", e))?;

    let build_result = (|| {
        if let Some(existing_config) = existing_config_toml {
            let config_path = temp_dir.join(CODEX_CONFIG_FILE_NAME);
            crate::modules::atomic_write::write_string_atomic(&config_path, existing_config)?;
        }

        write_account_bundle_to_dir(&temp_dir, account)?;

        let mut files = Vec::new();
        for (relative_path, mode) in [
            ("auth.json", 0o600_u32),
            (CODEX_CONFIG_FILE_NAME, 0o600),
            (CODEX_AUTH_PROJECTION_FILE_NAME, 0o600),
        ] {
            let path = temp_dir.join(relative_path);
            let content = if path.exists() {
                fs::read_to_string(&path)
                    .map_err(|e| format!("读取 Codex 投影文件失败: {}: {}", relative_path, e))?
            } else if relative_path == CODEX_CONFIG_FILE_NAME {
                String::new()
            } else {
                return Err(format!("Codex 投影缺少必要文件: {}", relative_path));
            };
            let sha256 = sha256_hex_bytes(content.as_bytes());
            files.push(CodexProjectionFile {
                relative_path: relative_path.to_string(),
                content,
                mode,
                sha256,
            });
        }

        let bundle_hash = build_bundle_hash(&files);
        Ok(CodexAccountProjectionBundle {
            account_id: account.id.clone(),
            account_email: account.email.clone(),
            token_generation: account.token_generation,
            files,
            bundle_hash,
        })
    })();

    if let Err(err) = fs::remove_dir_all(&temp_dir) {
        logger::log_warn(&format!(
            "[Codex SSH] 清理远程投影临时目录失败: path={}, error={}",
            temp_dir.display(),
            err
        ));
    }

    build_result
}

fn configured_codex_wsl_config_dir() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }

    #[cfg(target_os = "windows")]
    {
        let cfg = crate::modules::config::get_user_config();
        if !cfg.codex_sync_wsl {
            return None;
        }
        let trimmed = cfg.codex_wsl_config_dir.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(PathBuf::from(trimmed))
    }
}

fn sync_default_codex_account_to_wsl<F>(account_id: &str, write_bundle: F)
where
    F: FnOnce(&Path) -> Result<(), String>,
{
    let Some(wsl_dir) = configured_codex_wsl_config_dir() else {
        return;
    };

    match write_bundle(&wsl_dir) {
        Ok(()) => logger::log_info(&format!(
            "[Codex切号] 已同步默认账号到 WSL 配置目录: account_id={}, target_dir={}",
            account_id,
            wsl_dir.display()
        )),
        Err(err) => logger::log_warn(&format!(
            "[Codex切号] 同步默认账号到 WSL 配置目录失败，默认实例切号已完成: account_id={}, target_dir={}, error={}",
            account_id,
            wsl_dir.display(),
            err
        )),
    }
}

fn is_default_codex_projection_dir(dir: &Path) -> bool {
    if projection_dirs_equal(dir, &get_codex_home()) {
        return true;
    }

    configured_codex_wsl_config_dir()
        .as_deref()
        .map(|wsl_dir| projection_dirs_equal(dir, wsl_dir))
        .unwrap_or(false)
}

fn is_bound_api_key_account_id(
    bound_account_id: Option<&str>,
    oauth_account_id: &str,
    api_key_accounts: &[CodexAccount],
) -> bool {
    let Some(bound_account_id) = bound_account_id else {
        return false;
    };
    api_key_accounts.iter().any(|account| {
        account.id == bound_account_id
            && account.bound_oauth_account_id.as_deref() == Some(oauth_account_id)
    })
}

fn managed_projection_dirs_for_account(account_id: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let index = load_account_index();
    let bound_api_key_accounts: Vec<CodexAccount> = list_accounts()
        .into_iter()
        .filter(|account| {
            account.is_api_key_auth()
                && account.bound_oauth_account_id.as_deref() == Some(account_id)
        })
        .collect();
    if index.current_account_id.as_deref() == Some(account_id)
        || is_bound_api_key_account_id(
            index.current_account_id.as_deref(),
            account_id,
            &bound_api_key_accounts,
        )
    {
        dirs.push(get_codex_home());
        if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
            dirs.push(wsl_dir);
        }
    }

    match crate::modules::codex_instance::load_instance_store() {
        Ok(store) => {
            if store.default_settings.bind_account_id.as_deref() == Some(account_id)
                || is_bound_api_key_account_id(
                    store.default_settings.bind_account_id.as_deref(),
                    account_id,
                    &bound_api_key_accounts,
                )
            {
                if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
                    dirs.push(default_home);
                }
            }
            for instance in store.instances {
                if instance.bind_account_id.as_deref() == Some(account_id)
                    || is_bound_api_key_account_id(
                        instance.bind_account_id.as_deref(),
                        account_id,
                        &bound_api_key_accounts,
                    )
                {
                    dirs.push(PathBuf::from(instance.user_data_dir));
                }
            }
        }
        Err(err) => {
            logger::log_warn(&format!(
                "读取 Codex 实例绑定失败，跳过投影写穿: account_id={}, error={}",
                account_id, err
            ));
        }
    }

    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.to_string_lossy().to_string()));
    dirs
}

pub fn cleanup_managed_model_catalogs_on_startup() -> Result<usize, String> {
    let current_account_id = load_account_index().current_account_id;
    let account_requires_managed_catalog = |account_id: Option<&str>| {
        account_id
            .and_then(load_account)
            .map(|account| {
                crate::modules::codex_local_access::account_requires_provider_gateway(&account)
                    || account_syncs_model_catalog_to_codex(&account)
            })
            .unwrap_or(false)
    };
    let current_requires_managed_catalog =
        account_requires_managed_catalog(current_account_id.as_deref());
    let mut dirs: HashMap<String, (PathBuf, bool)> = HashMap::new();
    let mut add_dir = |dir: PathBuf, preserve_catalog: bool| {
        let key = dir.to_string_lossy().to_string();
        dirs.entry(key)
            .and_modify(|(_, preserve)| *preserve |= preserve_catalog)
            .or_insert((dir, preserve_catalog));
    };

    add_dir(get_codex_home(), current_requires_managed_catalog);
    if let Some(wsl_dir) = configured_codex_wsl_config_dir() {
        add_dir(wsl_dir, current_requires_managed_catalog);
    }
    if let Ok(store) = crate::modules::codex_instance::load_instance_store() {
        if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
            add_dir(
                default_home,
                account_requires_managed_catalog(store.default_settings.bind_account_id.as_deref()),
            );
        }
        for instance in store.instances {
            add_dir(
                PathBuf::from(instance.user_data_dir),
                account_requires_managed_catalog(instance.bind_account_id.as_deref()),
            );
        }
    }

    let mut cleaned = 0;
    let mut failures = Vec::new();
    for (_, (dir, preserve_catalog)) in dirs {
        if preserve_catalog {
            continue;
        }
        match cleanup_managed_model_catalog_for_dir(&dir) {
            Ok(true) => cleaned += 1,
            Ok(false) => {}
            Err(error) => failures.push(format!("profile_dir={}, error={}", dir.display(), error)),
        }
    }

    if failures.is_empty() {
        Ok(cleaned)
    } else {
        Err(format!(
            "清理受管 Codex 模型目录部分失败: cleaned={}, failures={}",
            cleaned,
            failures.join("; ")
        ))
    }
}

fn projection_dirs_equal(left: &Path, right: &Path) -> bool {
    left.to_string_lossy() == right.to_string_lossy()
}

fn load_bound_api_key_account_for_projection_dir(
    oauth_account_id: &str,
    dir: &Path,
) -> Option<CodexAccount> {
    let matches_bound_api_key = |account_id: &str| {
        let account = load_account(account_id)?;
        if account.is_api_key_auth()
            && account.bound_oauth_account_id.as_deref() == Some(oauth_account_id)
        {
            Some(account)
        } else {
            None
        }
    };

    let index = load_account_index();
    if is_default_codex_projection_dir(dir) {
        if let Some(account) = index
            .current_account_id
            .as_deref()
            .and_then(matches_bound_api_key)
        {
            return Some(account);
        }
    }

    let Ok(store) = crate::modules::codex_instance::load_instance_store() else {
        return None;
    };

    if let Ok(default_home) = crate::modules::codex_instance::get_default_codex_home() {
        if projection_dirs_equal(dir, &default_home) {
            if let Some(account) = store
                .default_settings
                .bind_account_id
                .as_deref()
                .and_then(matches_bound_api_key)
            {
                return Some(account);
            }
        }
    }

    for instance in store.instances {
        if projection_dirs_equal(dir, &PathBuf::from(&instance.user_data_dir)) {
            if let Some(account) = instance
                .bind_account_id
                .as_deref()
                .and_then(matches_bound_api_key)
            {
                return Some(account);
            }
        }
    }

    None
}

fn write_managed_account_projections(account: &CodexAccount) {
    for dir in managed_projection_dirs_for_account(&account.id) {
        let bound_api_key_account =
            load_bound_api_key_account_for_projection_dir(&account.id, &dir);
        let result = if let Some(api_key_account) = bound_api_key_account.as_ref() {
            write_api_key_account_bundle_with_oauth_to_dir(&dir, api_key_account, account)
        } else {
            write_prepared_account_bundle_to_dir(&dir, account)
        };
        match result {
            Ok(()) => {
                if let Some(api_key_account) = bound_api_key_account {
                    if crate::modules::codex_local_access::account_requires_provider_gateway(
                        &api_key_account,
                    ) || crate::modules::codex_local_access::account_requires_bound_oauth_local_gateway(
                        &api_key_account,
                    ) {
                        crate::modules::codex_local_access::reload_provider_gateway_for_profile_in_background(
                            dir,
                            api_key_account.id,
                            "OAuth token 写穿后恢复本地网关配置",
                        );
                    }
                }
            }
            Err(err) => {
                logger::log_warn(&format!(
                    "Codex Token 写穿受管投影失败: account_id={}, target_dir={}, error={}",
                    account.id,
                    dir.display(),
                    err
                ));
            }
        }
    }
    if let Err(err) =
        crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(account)
    {
        logger::log_warn(&format!(
            "Codex Token 写穿 API Service sidecar 认证失败，已忽略: account_id={}, error={}",
            account.id, err
        ));
    }
}

pub fn is_managed_auth_refresh_due(account: &CodexAccount) -> bool {
    if account.is_api_key_auth() || account.requires_reauth || !account_has_refresh_token(account) {
        return false;
    }

    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        return true;
    }

    account
        .token_updated_at
        .map(|updated_at| updated_at <= now_timestamp() - CODEX_PROACTIVE_REFRESH_INTERVAL_SECONDS)
        .unwrap_or(true)
}

async fn perform_managed_token_refresh(
    mut account: CodexAccount,
    reason: &str,
    force: bool,
) -> Result<CodexAccount, String> {
    let refresh_token = match account
        .tokens
        .refresh_token
        .clone()
        .filter(|token| !token.trim().is_empty())
    {
        Some(token) => token,
        None => {
            logger::log_warn(&format!(
                "Codex Token Authority 跳过刷新：账号缺少 refresh_token，按 access-token-only 模式继续使用当前 access_token: account_id={}, email={}, reason={}",
                account.id, account.email, reason
            ));
            if force || codex_oauth::is_token_expired(&account.tokens.access_token) {
                mark_account_requires_reauth(
                    &mut account,
                    CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON,
                )?;
                return Err(CODEX_MISSING_REFRESH_TOKEN_REAUTH_REASON.to_string());
            }
            return Ok(account);
        }
    };

    logger::log_info(&format!(
        "Codex Token Authority 开始刷新: account_id={}, email={}, reason={}",
        account.id, account.email, reason
    ));

    match codex_oauth::refresh_access_token_with_fallback(
        &refresh_token,
        Some(account.tokens.id_token.as_str()),
    )
    .await
    {
        Ok(new_tokens) => {
            account.tokens = new_tokens;
            sync_identity_from_tokens(&mut account);
            mark_token_chain_updated(&mut account);
            save_account(&account)?;
            write_managed_account_projections(&account);
            logger::log_info(&format!(
                "Codex Token Authority 刷新成功: account_id={}, generation={}",
                account.id, account.token_generation
            ));
            Ok(account)
        }
        Err(err) => {
            let user_error = format_refresh_error_for_user(&err);
            if is_reauth_required_refresh_error(&err) {
                let _ = mark_account_requires_reauth(&mut account, &user_error);
                return Err(user_error);
            }
            Err(user_error)
        }
    }
}

async fn refresh_managed_account_locked(
    account_id: &str,
    force: bool,
    reason: &str,
    observed_generation: Option<u64>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }
    if let Err(err) = sync_account_from_authority_sources(&mut account) {
        logger::log_warn(&format!(
            "Codex 账号刷新前同步官方凭证失败，继续使用账号库: account_id={}, error={}",
            account.id, err
        ));
    }
    if let Err(err) = clear_stale_missing_refresh_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 清理缺失 refresh_token 的过期重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    if account.requires_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新登录".to_string()));
    }
    if let Some(observed_generation) = observed_generation {
        if account.token_generation > observed_generation {
            if !codex_oauth::is_token_expired(&account.tokens.access_token) {
                logger::log_info(&format!(
                    "Codex Token Authority 复用已完成的刷新结果: account_id={}, observed_generation={}, current_generation={}, reason={}",
                    account.id,
                    observed_generation,
                    account.token_generation,
                    reason
                ));
                return Ok(account);
            }
            logger::log_warn(&format!(
                "Codex Token Authority 检测到刷新代际已推进但 access_token 仍过期，继续刷新: account_id={}, observed_generation={}, current_generation={}, reason={}",
                account.id,
                observed_generation,
                account.token_generation,
                reason
            ));
        }
    }
    if !force && !codex_oauth::is_token_expired(&account.tokens.access_token) {
        return Ok(account);
    }

    perform_managed_token_refresh(account, reason, force).await
}

async fn refresh_managed_account_with_authority(
    account_id: &str,
    force: bool,
    reason: &str,
    observed_generation: Option<u64>,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    refresh_managed_account_locked(account_id, force, reason, observed_generation).await
}

async fn refresh_bound_oauth_account_for_api_key(
    api_key_account: &CodexAccount,
    reason: &str,
) -> Result<CodexAccount, String> {
    let bound_id = api_key_account
        .bound_oauth_account_id
        .as_deref()
        .ok_or_else(|| "API Key 账号需先绑定 OAuth 账号".to_string())?
        .to_string();
    let _ = validate_api_key_bound_oauth_account(api_key_account, &bound_id)?;
    refresh_managed_account_with_authority(&bound_id, false, reason, None).await
}

pub async fn ensure_managed_account_fresh(account_id: &str) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, false, "prepare", None).await
}

pub async fn force_refresh_managed_account(
    account_id: &str,
    reason: &str,
) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, true, reason, None).await
}

pub async fn force_refresh_managed_account_after_observed(
    account_id: &str,
    observed_generation: u64,
    reason: &str,
) -> Result<CodexAccount, String> {
    refresh_managed_account_with_authority(account_id, true, reason, Some(observed_generation))
        .await
}

pub async fn keepalive_managed_account(
    account_id: &str,
    reason: &str,
) -> Result<CodexAccount, String> {
    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }
    if let Err(err) = sync_account_from_authority_sources(&mut account) {
        logger::log_warn(&format!(
            "Codex 保活同步官方凭证失败，继续使用账号库: account_id={}, error={}",
            account.id, err
        ));
    }
    if let Err(err) = clear_stale_missing_refresh_token_reauth(&mut account) {
        logger::log_warn(&format!(
            "Codex 保活清理缺失 refresh_token 的过期重登标记失败，继续处理: account_id={}, error={}",
            account.id, err
        ));
    }
    if account.requires_reauth {
        return Err(account
            .reauth_reason
            .clone()
            .unwrap_or_else(|| "账号需要重新登录".to_string()));
    }
    if !is_managed_auth_refresh_due(&account) {
        return Ok(account);
    }

    perform_managed_token_refresh(account, reason, false).await
}

pub async fn execute_with_managed_account_projection<R, F>(
    account_id: &str,
    auth_dir: &Path,
    reason: &str,
    operation: F,
) -> Result<(CodexAccount, R, Option<String>), String>
where
    F: FnOnce(&CodexAccount) -> R,
{
    let api_key_account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if api_key_account.is_api_key_auth() {
        let sync_error = if normalize_optional_ref(
            api_key_account.bound_oauth_account_id.as_deref(),
        )
        .is_some()
        {
            let oauth_account =
                refresh_bound_oauth_account_for_api_key(&api_key_account, reason).await?;
            write_api_key_account_bundle_with_oauth_to_dir(
                auth_dir,
                &api_key_account,
                &oauth_account,
            )?;

            let sync_result =
                match sync_managed_projection_from_auth_dir(&oauth_account.id, auth_dir) {
                    Ok(_) => {
                        let latest_oauth_account = load_account(&oauth_account.id)
                            .unwrap_or_else(|| oauth_account.clone());
                        match write_api_key_account_bundle_with_oauth_to_dir(
                            auth_dir,
                            &api_key_account,
                            &latest_oauth_account,
                        ) {
                            Ok(_) => None,
                            Err(err) => Some(err),
                        }
                    }
                    Err(err) => Some(err),
                };
            sync_result
        } else {
            write_prepared_account_bundle_to_dir(auth_dir, &api_key_account)?;
            None
        };
        let result = operation(&api_key_account);
        let latest_account = load_account(account_id).unwrap_or(api_key_account);

        return Ok((latest_account, result, sync_error));
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, reason).await?;
    let account = refresh_managed_account_locked(account_id, false, reason, None).await?;
    write_prepared_account_bundle_to_dir(auth_dir, &account)?;

    let result = operation(&account);
    let sync_error = match sync_managed_projection_from_auth_dir(account_id, auth_dir) {
        Ok(_) => None,
        Err(err) => Some(err),
    };
    let latest_account = load_account(account_id).unwrap_or(account);

    Ok((latest_account, result, sync_error))
}

/// 准备账号注入：刷新前会先采用更新的官方凭证，随后由账号中心写穿受管投影。
pub async fn prepare_account_for_injection_from_auth_dir(
    account_id: &str,
    auth_dir: Option<&Path>,
) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        if let Some(dir) = auth_dir {
            if normalize_optional_ref(account.bound_oauth_account_id.as_deref()).is_some() {
                let oauth_account =
                    refresh_bound_oauth_account_for_api_key(&account, "prepare").await?;
                write_api_key_account_bundle_with_oauth_to_dir(dir, &account, &oauth_account)?;
            } else {
                write_prepared_account_bundle_to_dir(dir, &account)?;
            }
        }
        return Ok(account);
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "prepare").await?;
    let account = refresh_managed_account_locked(account_id, false, "prepare", None).await?;
    if let Some(dir) = auth_dir {
        write_prepared_account_bundle_to_dir(dir, &account)?;
    }
    Ok(account)
}

pub async fn prepare_account_for_injection(account_id: &str) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_store(account_id).await
}

/// 准备账号注入（账号中心模式）：
/// 账号中心负责最终写穿；刷新前只接受带有更新 last_refresh 或未过期访问令牌的官方凭证。
pub async fn prepare_account_for_injection_from_store(
    account_id: &str,
) -> Result<CodexAccount, String> {
    ensure_managed_account_fresh(account_id).await
}

fn switch_account_with_prepared(
    account_id: &str,
    account_for_write: CodexAccount,
) -> Result<CodexAccount, String> {
    let codex_home = get_codex_home();
    let index_before_switch = load_account_index();
    let bundle_snapshot = snapshot_codex_bundle(&codex_home, &account_for_write)?;
    let store_snapshot =
        snapshot_codex_account_store(&index_before_switch, &account_for_write.id)?;
    let auth_path = codex_home.join("auth.json");
    logger::log_info(&format!(
        "[Codex切号] 开始切换账号: account_id={}, email={}, target_dir={}",
        account_for_write.id,
        account_for_write.email,
        codex_home.display()
    ));
    write_prepared_account_bundle_to_dir(&codex_home, &account_for_write)?;
    logger::log_info(&format!(
        "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
        codex_home.display(),
        auth_path.display()
    ));
    // 更新索引中的 current_account_id
    let mut index = index_before_switch;
    index.current_account_id = Some(account_id.to_string());
    if let Err(error) = save_account_index(&index) {
        return Err(codex_switch_error_with_rollback(
            format!("保存当前账号索引失败: {}", error),
            &bundle_snapshot,
            &store_snapshot,
        ));
    }

    // 更新账号的 last_used
    let mut updated_account = account_for_write.clone();
    updated_account.update_last_used();
    if let Err(error) = save_account(&updated_account) {
        return Err(codex_switch_error_with_rollback(
            format!("保存账号详情失败: {}", error),
            &bundle_snapshot,
            &store_snapshot,
        ));
    }

    sync_default_codex_account_to_wsl(&account_for_write.id, |wsl_dir| {
        write_prepared_account_bundle_to_dir(wsl_dir, &account_for_write)
    });

    logger::log_info(&format!("已切换到 Codex 账号: {}", updated_account.email));

    Ok(updated_account)
}

async fn activate_provider_gateway_after_switch_if_needed(
    base_dir: &Path,
    account: &CodexAccount,
) -> Result<(), String> {
    if crate::modules::codex_local_access::account_requires_provider_gateway(account) {
        logger::log_info(&format!(
            "[Codex切号] API Key 账号使用 Chat Completions 协议，启用本地供应商网关: account_id={}, target_dir={}",
            account.id,
            base_dir.display()
        ));
        crate::modules::codex_local_access::ensure_provider_gateway_for_dir(base_dir, &account.id)
            .await?;
        return Ok(());
    }

    if crate::modules::codex_local_access::account_requires_bound_oauth_local_gateway(account) {
        logger::log_info(&format!(
            "[Codex切号] API Key 账号绑定 OAuth 且禁用 image_generation，启用 Responses 本地网关: account_id={}, target_dir={}",
            account.id,
            base_dir.display()
        ));
        crate::modules::codex_local_access::ensure_bound_oauth_local_gateway_for_dir(
            base_dir,
            &account.id,
        )
        .await?;
        return Ok(());
    }

    crate::modules::codex_local_access::stop_provider_gateways_for_profile(base_dir).await;
    Ok(())
}

/// 若导入结果包含当前激活账号，则重新切号落盘，避免库内 token 已更新但运行中仍用旧凭证。
/// 成功时返回已重新激活的账号，便于调用方补跑 Hermes/OpenCode/OpenClaw 等切号副作用。
/// 重新激活失败只记日志，不打断导入成功结果。
pub async fn reactivate_if_imported_matches_current(
    imported: &[CodexAccount],
) -> Option<CodexAccount> {
    let current_id = load_account_index().current_account_id?;
    if !imported
        .iter()
        .any(|account| account.id.as_str() == current_id.as_str())
    {
        return None;
    }

    match switch_account_managed(&current_id).await {
        Ok(account) => {
            logger::log_info(&format!(
                "[Codex导入] 当前账号已重新激活: id={}, email={}",
                account.id, account.email
            ));
            Some(account)
        }
        Err(error) => {
            logger::log_error(&format!(
                "[Codex导入] 当前账号重新激活失败（导入已成功）: id={}, error={}",
                current_id, error
            ));
            None
        }
    }
}

pub async fn switch_account_managed(account_id: &str) -> Result<CodexAccount, String> {
    crate::modules::process::ensure_codex_app_server_not_running()?;

    let account = load_account_after_index_repair(account_id)
        .ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        if normalize_optional_ref(account.bound_oauth_account_id.as_deref()).is_none() {
            let updated_account = switch_account_with_prepared(account_id, account)?;
            let codex_home = get_codex_home();
            if let Err(error) =
                activate_provider_gateway_after_switch_if_needed(&codex_home, &updated_account)
                    .await
            {
                logger::log_warn(&format!(
                    "[Codex切号] 核心切号已完成，但 provider gateway 激活失败，继续返回成功并等待下次重试: account_id={}, error={}",
                    updated_account.id, error
                ));
            }
            return Ok(updated_account);
        }
        let oauth_account = refresh_bound_oauth_account_for_api_key(&account, "switch").await?;
        let codex_home = get_codex_home();
        let auth_path = codex_home.join("auth.json");
        let index_before_switch = load_account_index();
        let bundle_snapshot = snapshot_codex_bundle(&codex_home, &oauth_account)?;
        let store_snapshot =
            snapshot_codex_account_store(&index_before_switch, &account.id)?;
        logger::log_info(&format!(
            "[Codex切号] 开始切换 API Key 账号绑定 OAuth: api_account_id={}, oauth_account_id={}, target_dir={}",
            account.id,
            oauth_account.id,
            codex_home.display()
        ));
        write_api_key_account_bundle_with_oauth_to_dir(&codex_home, &account, &oauth_account)?;
        logger::log_info(&format!(
            "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
            codex_home.display(),
            auth_path.display()
        ));
        let mut index = index_before_switch;
        index.current_account_id = Some(account_id.to_string());
        if let Err(error) = save_account_index(&index) {
            return Err(codex_switch_error_with_rollback(
                format!("保存当前账号索引失败: {}", error),
                &bundle_snapshot,
                &store_snapshot,
            ));
        }

        let mut updated_account = account.clone();
        updated_account.update_last_used();
        if let Err(error) = save_account(&updated_account) {
            return Err(codex_switch_error_with_rollback(
                format!("保存账号详情失败: {}", error),
                &bundle_snapshot,
                &store_snapshot,
            ));
        }

        sync_default_codex_account_to_wsl(&account.id, |wsl_dir| {
            write_api_key_account_bundle_with_oauth_to_dir(wsl_dir, &account, &oauth_account)
        });

        logger::log_info(&format!(
            "已切换到 Codex API Key 账号: {}，登录态绑定 OAuth: {}",
            updated_account.email, oauth_account.email
        ));

        if let Err(error) =
            activate_provider_gateway_after_switch_if_needed(&codex_home, &updated_account).await
        {
            logger::log_warn(&format!(
                "[Codex切号] 核心切号已完成，但 provider gateway 激活失败，继续返回成功并等待下次重试: account_id={}, error={}",
                updated_account.id, error
            ));
        }

        return Ok(updated_account);
    }

    let lock = codex_token_lock_for(account_id);
    let _guard = lock.lock().await;
    let _file_guard = acquire_codex_token_refresh_file_lock(account_id, "switch").await?;
    let account = refresh_managed_account_locked(account_id, false, "switch", None).await?;
    switch_account_with_prepared(account_id, account)
}

/// 从本地 auth.json 导入账号
pub fn import_from_local() -> Result<CodexAccount, String> {
    let auth_path = get_auth_json_path();
    if !auth_path.exists() {
        return Err("未找到 ~/.codex/auth.json 文件".to_string());
    }

    let content =
        fs::read_to_string(&auth_path).map_err(|e| format!("读取 auth.json 失败: {}", e))?;

    let auth_file: CodexAuthFile =
        serde_json::from_str(&content).map_err(|e| format!("解析 auth.json 失败: {}", e))?;
    let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
    let config_provider = read_api_provider_from_config_toml(&get_codex_home());
    let fallback_provider = infer_api_provider_config(
        extract_api_base_url_from_auth_file(&auth_file)
            .or_else(|| config_provider.base_url.clone())
            .as_deref(),
        Some(config_provider.mode.clone()),
        config_provider.provider_id.as_deref(),
        config_provider.provider_name.as_deref(),
    );

    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
            Vec::new(),
            Some(false),
            None,
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
        );
    }

    if let Some(personal_access_token) =
        normalize_optional_ref(auth_file.personal_access_token.as_deref())
    {
        return upsert_account_from_access_token(personal_access_token, None);
    }

    if let Some(tokens) = auth_file.tokens {
        return upsert_account_from_auth_tokens(tokens);
    }

    if let Some(api_key) = fallback_api_key {
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
            Vec::new(),
            Some(false),
            None,
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
        );
    }

    Err("auth.json 缺少可导入的账号信息".to_string())
}

fn import_account_struct(account: CodexAccount) -> Result<CodexAccount, String> {
    if is_pending_oauth_account(&account) {
        let mut imported = create_pending_oauth_account(
            account.email.clone(),
            codex_account_note_update_from_account(&account),
        )?;
        if let Some(tags) = account.tags {
            imported.tags = Some(tags);
            save_account(&imported)?;
        }
        return Ok(imported);
    }

    if account.is_api_key_auth() || account.openai_api_key.is_some() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        let mut api_acc = upsert_api_key_account(
            api_key,
            account.api_base_url.clone(),
            Some(account.api_provider_mode),
            account.api_provider_id.clone(),
            account.api_provider_name.clone(),
            account.api_model_catalog.clone(),
            Some(account.api_sync_model_catalog_to_codex),
            account.api_wire_api.clone(),
            account.api_supports_websockets,
            account.api_supports_vision,
            account.api_model_vision_support.clone(),
            account.api_vision_routing_model.clone(),
            account.account_name.clone(),
        )?;
        let mut changed = false;
        if let Some(tags) = account.tags {
            api_acc.tags = Some(tags);
            changed = true;
        }
        if let Some(note) = account.account_note {
            api_acc.account_note = Some(note);
            changed = true;
        }
        if let Some(secret) = account.two_factor_secret {
            api_acc.two_factor_secret = Some(secret);
            changed = true;
        }
        if let Some(password) = account.account_password {
            api_acc.account_password = Some(password);
            changed = true;
        }
        if let Some(phone_number) = account.phone_number {
            api_acc.phone_number = Some(phone_number);
            changed = true;
        }
        if let Some(mail_url) = account.mail_url {
            api_acc.mail_url = Some(mail_url);
            changed = true;
        }
        if changed {
            save_account(&api_acc)?;
        }
        return Ok(api_acc);
    }

    let imported_auth_file_plan_type =
        normalize_auth_file_plan_type(account.auth_file_plan_type.as_deref());
    let mut imported = upsert_account(account.tokens)?;
    let mut changed = apply_auth_file_plan_type(&mut imported, imported_auth_file_plan_type);

    if let Some(tags) = account.tags {
        imported.tags = Some(tags);
        changed = true;
    }
    if let Some(note) = account.account_note {
        imported.account_note = Some(note);
        changed = true;
    }
    if let Some(secret) = account.two_factor_secret {
        imported.two_factor_secret = Some(secret);
        changed = true;
    }
    if let Some(password) = account.account_password {
        imported.account_password = Some(password);
        changed = true;
    }
    if let Some(phone_number) = account.phone_number {
        imported.phone_number = Some(phone_number);
        changed = true;
    }
    if let Some(mail_url) = account.mail_url {
        imported.mail_url = Some(mail_url);
        changed = true;
    }

    if changed {
        save_account(&imported)?;
    }

    Ok(imported)
}

fn upsert_account_from_auth_tokens(tokens: CodexAuthTokens) -> Result<CodexAccount, String> {
    let account_id_hint = tokens.account_id.clone();
    let tokens = CodexTokens {
        id_token: tokens.id_token,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    };

    if normalize_optional_ref(Some(&tokens.id_token)).is_none()
        && is_importable_access_token(&tokens.access_token)
    {
        return upsert_account_from_access_token_with_hints(
            tokens.access_token,
            CodexAccessTokenImportHints {
                account_id: account_id_hint,
                ..Default::default()
            },
        );
    }

    upsert_account_with_hints(tokens, account_id_hint, None)
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct CodexAccessTokenImportHints {
    email: Option<String>,
    user_id: Option<String>,
    plan_type: Option<String>,
    subscription_active_until: Option<String>,
    account_id: Option<String>,
    organization_id: Option<String>,
    account_name: Option<String>,
    account_structure: Option<String>,
    account_note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
}

enum CodexJsonImportCandidate {
    FullToken {
        tokens: CodexTokens,
        account_id_hint: Option<String>,
        note_update: CodexAccountNoteUpdate,
    },
    AccessToken {
        access_token: String,
        hints: CodexAccessTokenImportHints,
    },
    RefreshToken {
        refresh_token: String,
        note_update: CodexAccountNoteUpdate,
    },
}

fn codex_account_note_update_from_value(value: &serde_json::Value) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: read_json_string(
            value,
            &["account_note", "accountNote", "note", "notes", "remark"],
        ),
        two_factor_secret: read_json_string(
            value,
            &[
                "two_factor_secret",
                "twoFactorSecret",
                "account_two_factor_secret",
                "accountTwoFactorSecret",
            ],
        ),
        account_password: read_json_string(
            value,
            &["account_password", "accountPassword", "password"],
        ),
        phone_number: read_json_string(
            value,
            &[
                "phone_number",
                "phoneNumber",
                "account_phone_number",
                "accountPhoneNumber",
            ],
        ),
        mail_url: read_account_mail_url(value),
    }
}

fn has_codex_account_note_update(update: &CodexAccountNoteUpdate) -> bool {
    update.note.is_some()
        || update.two_factor_secret.is_some()
        || update.account_password.is_some()
        || update.phone_number.is_some()
        || update.mail_url.is_some()
}

fn merge_codex_account_note_update(
    mut primary: CodexAccountNoteUpdate,
    fallback: CodexAccountNoteUpdate,
) -> CodexAccountNoteUpdate {
    if primary.note.is_none() {
        primary.note = fallback.note;
    }
    if primary.two_factor_secret.is_none() {
        primary.two_factor_secret = fallback.two_factor_secret;
    }
    if primary.account_password.is_none() {
        primary.account_password = fallback.account_password;
    }
    if primary.phone_number.is_none() {
        primary.phone_number = fallback.phone_number;
    }
    if primary.mail_url.is_none() {
        primary.mail_url = fallback.mail_url;
    }
    primary
}

fn codex_account_note_update_from_hints(
    hints: &CodexAccessTokenImportHints,
) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: hints.account_note.clone(),
        two_factor_secret: hints.two_factor_secret.clone(),
        account_password: hints.account_password.clone(),
        phone_number: hints.phone_number.clone(),
        mail_url: hints.mail_url.clone(),
    }
}

fn apply_account_note_update_if_present(
    account: &mut CodexAccount,
    update: CodexAccountNoteUpdate,
) -> bool {
    if !has_codex_account_note_update(&update) {
        return false;
    }
    apply_account_note_update(account, update);
    true
}

fn save_account_note_update_if_present(
    account: &mut CodexAccount,
    update: CodexAccountNoteUpdate,
) -> Result<(), String> {
    if apply_account_note_update_if_present(account, update) {
        save_account(account)?;
    }
    Ok(())
}

fn is_blank_codex_token_fields(value: &serde_json::Value) -> bool {
    let id_token = first_json_string(
        value,
        &[&["id_token"], &["idToken"], &["tokens", "id_token"]],
    );
    let access_token = first_json_string(
        value,
        &[
            &["access_token"],
            &["accessToken"],
            &["tokens", "access_token"],
        ],
    );
    let refresh_token = first_json_string(
        value,
        &[
            &["refresh_token"],
            &["refreshToken"],
            &["tokens", "refresh_token"],
            &["tokens", "refreshToken"],
        ],
    );

    id_token.is_none() && access_token.is_none() && refresh_token.is_none()
}

fn pending_oauth_account_from_value(value: &serde_json::Value) -> Option<CodexAccount> {
    let obj = value.as_object()?;
    let auth_mode = read_json_string(value, &["auth_mode", "authMode"])
        .unwrap_or_else(|| "oauth".to_string())
        .to_ascii_lowercase();
    if auth_mode == "apikey" {
        return None;
    }

    let account_type = read_json_string(value, &["type"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let authorization_status =
        read_json_string(value, &["authorization_status", "authorizationStatus"])
            .unwrap_or_default()
            .to_ascii_lowercase();
    let update = codex_account_note_update_from_value(value);
    let has_pending_marker = authorization_status == CODEX_AUTHORIZATION_STATUS_PENDING
        || account_type == "codex"
        || has_codex_account_note_update(&update);

    if !has_pending_marker || !is_blank_codex_token_fields(value) {
        return None;
    }

    let email = read_json_string(value, &["email", "account_email", "accountEmail"])
        .or_else(|| read_json_string(value, &["account_name", "accountName"]))
        .filter(|value| !value.trim().is_empty())?;
    let account_id = build_account_storage_id(&email, Some("pending_oauth"), None);
    let now = now_timestamp();
    let mut account = CodexAccount::new(
        account_id,
        email,
        CodexTokens {
            id_token: String::new(),
            access_token: String::new(),
            refresh_token: None,
        },
    );
    account.auth_mode = CodexAuthMode::OAuth;
    account.authorization_status = Some(CODEX_AUTHORIZATION_STATUS_PENDING.to_string());
    account.token_updated_at = None;
    account.token_generation = 0;
    account.created_at = read_json_i64(value, &["created_at", "createdAt"]).unwrap_or(now);
    account.last_used =
        read_json_i64(value, &["last_used", "lastUsed"]).unwrap_or(account.created_at);
    apply_account_note_update(&mut account, update);
    account.tags = read_json_string_array(value, &["tags"]);

    // Treat a token-less Codex object as a saved draft only when it actually
    // carries pending metadata. This avoids silently importing malformed auth files.
    if authorization_status == CODEX_AUTHORIZATION_STATUS_PENDING
        || has_codex_account_note_details(&account)
        || obj.contains_key("account_note")
        || obj.contains_key("accountNote")
    {
        Some(account)
    } else {
        None
    }
}

fn has_codex_account_note_details(account: &CodexAccount) -> bool {
    account
        .account_note
        .as_deref()
        .and_then(|value| normalize_optional_ref(Some(value)))
        .is_some()
        || account
            .two_factor_secret
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .account_password
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .phone_number
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .mail_url
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
}

fn codex_account_note_update_from_account(account: &CodexAccount) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: account.account_note.clone(),
        two_factor_secret: account.two_factor_secret.clone(),
        account_password: account.account_password.clone(),
        phone_number: account.phone_number.clone(),
        mail_url: account.mail_url.clone(),
    }
}

fn is_opaque_access_token(token: &str) -> bool {
    normalize_optional_ref(Some(token))
        .map(|token| token.starts_with("at-"))
        .unwrap_or(false)
}

fn is_importable_access_token(token: &str) -> bool {
    decode_jwt_payload_value(token).is_some() || is_opaque_access_token(token)
}

fn extract_bearer_token_from_header(value: &str) -> Option<String> {
    let value = normalize_optional_ref(Some(value))?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = normalize_optional_ref(Some(token))?;
    is_importable_access_token(&token).then(|| token.to_string())
}

fn extract_opaque_access_token_from_text(value: &str) -> Option<String> {
    let value = normalize_optional_ref(Some(value))?;
    for (start, _) in value.match_indices("at-") {
        let token: String = value[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        if is_opaque_access_token(&token) {
            return Some(token);
        }
    }
    None
}

fn first_json_scalar_string(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        normalize_optional_json_scalar(Some(current))
    })
}

fn merge_access_token_import_hints(
    mut primary: CodexAccessTokenImportHints,
    fallback: CodexAccessTokenImportHints,
) -> CodexAccessTokenImportHints {
    if primary.email.is_none() {
        primary.email = fallback.email;
    }
    if primary.user_id.is_none() {
        primary.user_id = fallback.user_id;
    }
    if primary.plan_type.is_none() {
        primary.plan_type = fallback.plan_type;
    }
    if primary.subscription_active_until.is_none() {
        primary.subscription_active_until = fallback.subscription_active_until;
    }
    if primary.account_id.is_none() {
        primary.account_id = fallback.account_id;
    }
    if primary.organization_id.is_none() {
        primary.organization_id = fallback.organization_id;
    }
    if primary.account_name.is_none() {
        primary.account_name = fallback.account_name;
    }
    if primary.account_structure.is_none() {
        primary.account_structure = fallback.account_structure;
    }
    if primary.account_note.is_none() {
        primary.account_note = fallback.account_note;
    }
    if primary.two_factor_secret.is_none() {
        primary.two_factor_secret = fallback.two_factor_secret;
    }
    if primary.account_password.is_none() {
        primary.account_password = fallback.account_password;
    }
    if primary.phone_number.is_none() {
        primary.phone_number = fallback.phone_number;
    }
    if primary.mail_url.is_none() {
        primary.mail_url = fallback.mail_url;
    }
    primary
}

fn first_personal_access_token_string(value: &serde_json::Value) -> Option<String> {
    first_json_scalar_string(
        value,
        &[
            &["personal_access_token"],
            &["personalAccessToken"],
            &["at_token"],
            &["atToken"],
            &["tokens", "personal_access_token"],
            &["tokens", "personalAccessToken"],
            &["tokens", "at_token"],
            &["tokens", "atToken"],
            &["credentials", "personal_access_token"],
            &["credentials", "personalAccessToken"],
            &["credentials", "at_token"],
            &["credentials", "atToken"],
        ],
    )
    .filter(|token| is_importable_access_token(token))
    .or_else(|| {
        first_json_scalar_string(
            value,
            &[
                &["headers", "authorization"],
                &["headers", "Authorization"],
                &["credentials", "headers", "authorization"],
                &["credentials", "headers", "Authorization"],
            ],
        )
        .and_then(|header| extract_bearer_token_from_header(&header))
    })
    .or_else(|| {
        first_json_scalar_string(
            value,
            &[
                &["credentials", "access_token"],
                &["credentials", "accessToken"],
                &["access_token"],
                &["accessToken"],
            ],
        )
        .filter(|token| is_opaque_access_token(token))
    })
}

fn extract_access_token_import_hints_from_value(
    value: &serde_json::Value,
) -> CodexAccessTokenImportHints {
    let note_update = codex_account_note_update_from_value(value);
    CodexAccessTokenImportHints {
        email: first_json_scalar_string(
            value,
            &[
                &["email"],
                &["account_email"],
                &["accountEmail"],
                &["user", "email"],
                &["profile", "email"],
                &["account", "email"],
                &["credentials", "email"],
            ],
        ),
        user_id: first_json_scalar_string(
            value,
            &[
                &["user_id"],
                &["userId"],
                &["user", "id"],
                &["account", "user_id"],
                &["account", "userId"],
            ],
        ),
        plan_type: first_json_scalar_string(
            value,
            &[
                &["plan_type"],
                &["planType"],
                &["account", "plan_type"],
                &["account", "planType"],
                &["account", "plan"],
                &["credentials", "plan_type"],
                &["credentials", "planType"],
                &["credentials", "chatgpt_plan_type"],
            ],
        ),
        subscription_active_until: first_json_scalar_string(
            value,
            &[
                &["subscription_active_until"],
                &["subscriptionActiveUntil"],
                &["expires_at"],
                &["expiresAt"],
                &["account", "subscription_active_until"],
                &["account", "subscriptionActiveUntil"],
                &["credentials", "subscription_active_until"],
                &["credentials", "subscriptionActiveUntil"],
                &["credentials", "expires_at"],
                &["credentials", "expiresAt"],
            ],
        ),
        account_id: first_json_scalar_string(
            value,
            &[
                &["account_id"],
                &["accountId"],
                &["chatgpt_account_id"],
                &["workspace_id"],
                &["account", "id"],
                &["account", "account_id"],
                &["account", "accountId"],
                &["credentials", "account_id"],
                &["credentials", "accountId"],
                &["credentials", "chatgpt_account_id"],
                &["credentials", "workspace_id"],
            ],
        ),
        organization_id: first_json_scalar_string(
            value,
            &[
                &["organization_id"],
                &["organizationId"],
                &["org_id"],
                &["orgId"],
                &["poid"],
                &["POID"],
                &["account", "organization_id"],
                &["account", "organizationId"],
                &["account", "org_id"],
                &["account", "orgId"],
            ],
        ),
        account_name: first_json_scalar_string(
            value,
            &[
                &["account_name"],
                &["accountName"],
                &["name"],
                &["display_name"],
                &["account", "name"],
                &["account", "display_name"],
                &["account", "account_name"],
                &["account", "accountName"],
            ],
        ),
        account_structure: first_json_scalar_string(
            value,
            &[
                &["account_structure"],
                &["accountStructure"],
                &["structure"],
                &["account", "structure"],
                &["account", "account_structure"],
                &["account", "accountStructure"],
                &["account", "type"],
            ],
        ),
        account_note: note_update.note,
        two_factor_secret: note_update.two_factor_secret,
        account_password: note_update.account_password,
        phone_number: note_update.phone_number,
        mail_url: note_update.mail_url,
    }
}

fn is_codex_session_object(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let has_access_token = first_json_string(value, &[&["accessToken"], &["access_token"]])
        .filter(|token| is_importable_access_token(token))
        .is_some();
    if !has_access_token {
        return false;
    }

    obj.get("user").and_then(|item| item.as_object()).is_some()
        || obj
            .get("account")
            .and_then(|item| item.as_object())
            .is_some()
        || obj.get("expires").is_some()
        || obj.get("sessionToken").is_some()
        || obj
            .get("authProvider")
            .and_then(|item| item.as_str())
            .map(|provider| provider.eq_ignore_ascii_case("openai"))
            .unwrap_or(false)
}

fn normalize_codex_session_value(
    value: &serde_json::Value,
    depth: usize,
) -> Option<serde_json::Value> {
    if depth > 4 {
        return None;
    }
    let obj = value.as_object()?;

    for key in ["session_json", "session"] {
        let Some(nested) = obj.get(key) else {
            continue;
        };
        match nested {
            serde_json::Value::Object(_) => {
                if let Some(session) = normalize_codex_session_value(nested, depth + 1) {
                    return Some(session);
                }
            }
            serde_json::Value::String(raw) => {
                let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
                if let Some(session) = normalize_codex_session_value(&parsed, depth + 1) {
                    return Some(session);
                }
            }
            _ => {}
        }
    }

    if is_codex_session_object(value) {
        return Some(value.clone());
    }

    None
}

fn extract_codex_session_candidate_from_value(
    value: &serde_json::Value,
) -> Option<CodexJsonImportCandidate> {
    let session = normalize_codex_session_value(value, 0)?;
    let access_token = first_json_string(&session, &[&["accessToken"], &["access_token"]])
        .filter(|token| is_importable_access_token(token))?;
    let account_id_hint = first_json_string(&session, &[&["account", "id"], &["account_id"]]);
    let note_update = merge_codex_account_note_update(
        codex_account_note_update_from_value(value),
        codex_account_note_update_from_value(&session),
    );
    let mut session_hints = merge_access_token_import_hints(
        extract_access_token_import_hints_from_value(&session),
        extract_access_token_import_hints_from_value(value),
    );
    if session_hints.account_id.is_none() {
        session_hints.account_id = account_id_hint.clone();
    }
    let session_hints_note_update = codex_account_note_update_from_hints(&session_hints);
    let session_hints_note_update =
        merge_codex_account_note_update(session_hints_note_update, note_update.clone());
    session_hints.account_note = session_hints_note_update.note;
    session_hints.two_factor_secret = session_hints_note_update.two_factor_secret;
    session_hints.account_password = session_hints_note_update.account_password;
    session_hints.phone_number = session_hints_note_update.phone_number;
    session_hints.mail_url = session_hints_note_update.mail_url;

    if let Some(id_token) = first_json_string(&session, &[&["idToken"], &["id_token"]]) {
        let refresh_token = first_json_string(&session, &[&["refreshToken"], &["refresh_token"]]);
        return Some(CodexJsonImportCandidate::FullToken {
            tokens: CodexTokens {
                id_token,
                access_token,
                refresh_token,
            },
            account_id_hint,
            note_update,
        });
    }

    if decode_jwt_payload_value(&access_token).is_some() {
        let refresh_token = first_json_string(&session, &[&["refreshToken"], &["refresh_token"]]);
        return Some(CodexJsonImportCandidate::FullToken {
            tokens: CodexTokens {
                id_token: access_token.clone(),
                access_token,
                refresh_token,
            },
            account_id_hint,
            note_update,
        });
    }

    Some(CodexJsonImportCandidate::AccessToken {
        access_token,
        hints: session_hints,
    })
}

fn extract_refresh_token_only_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw)).filter(|token| {
            decode_jwt_payload_value(token).is_none()
                && !is_opaque_access_token(token)
                && extract_opaque_access_token_from_text(raw).is_none()
        }),
        serde_json::Value::Object(_) => first_json_string(
            value,
            &[
                &["refresh_token"],
                &["refreshToken"],
                &["tokens", "refresh_token"],
                &["tokens", "refreshToken"],
            ],
        ),
        _ => None,
    }
}

fn extract_access_token_only_from_value(
    value: &serde_json::Value,
) -> Option<(String, CodexAccessTokenImportHints)> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw))
            .filter(|token| is_importable_access_token(token))
            .or_else(|| extract_opaque_access_token_from_text(raw))
            .map(|token| (token, CodexAccessTokenImportHints::default())),
        serde_json::Value::Object(_) => first_personal_access_token_string(value)
            .or_else(|| {
                first_json_string(
                    value,
                    &[
                        &["tokens", "access_token"],
                        &["tokens", "accessToken"],
                        &["credentials", "access_token"],
                        &["credentials", "accessToken"],
                        &["access_token"],
                        &["accessToken"],
                        &["token"],
                    ],
                )
                .filter(|token| is_importable_access_token(token))
            })
            .map(|token| (token, extract_access_token_import_hints_from_value(value))),
        _ => None,
    }
}

fn extract_codex_import_candidate_from_value(
    value: &serde_json::Value,
) -> Option<CodexJsonImportCandidate> {
    if value.is_object() {
        if let Some(access_token) = first_personal_access_token_string(value) {
            let hints = extract_access_token_import_hints_from_value(value);
            return Some(CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            });
        }
    }

    if let Some(candidate) = extract_codex_session_candidate_from_value(value) {
        return Some(candidate);
    }

    if let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(value) {
        return Some(CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            note_update: codex_account_note_update_from_value(value),
        });
    }

    if let Some(refresh_token) = extract_refresh_token_only_from_value(value) {
        return Some(CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            note_update: codex_account_note_update_from_value(value),
        });
    }

    extract_access_token_only_from_value(value).map(|(access_token, mut hints)| {
        let hints_note_update = codex_account_note_update_from_hints(&hints);
        let hints_note_update = merge_codex_account_note_update(
            hints_note_update,
            codex_account_note_update_from_value(value),
        );
        hints.account_note = hints_note_update.note;
        hints.two_factor_secret = hints_note_update.two_factor_secret;
        hints.account_password = hints_note_update.account_password;
        hints.phone_number = hints_note_update.phone_number;
        hints.mail_url = hints_note_update.mail_url;
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        }
    })
}

async fn upsert_account_from_refresh_token(
    refresh_token: String,
    note_update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let tokens = codex_oauth::refresh_access_token(&refresh_token).await?;
    let mut account = upsert_account(tokens)?;
    save_account_note_update_if_present(&mut account, note_update)?;
    Ok(account)
}

fn upsert_account_from_access_token(
    access_token: String,
    account_note: Option<String>,
) -> Result<CodexAccount, String> {
    upsert_account_from_access_token_with_hints(
        access_token,
        CodexAccessTokenImportHints {
            account_note,
            ..Default::default()
        },
    )
}

/// Named access-token import (community #1448): store as OAuth-shaped account with
/// optional display name; projection uses personal_access_token when no refresh/id.
pub fn import_access_token_account(
    account_name: String,
    access_token: String,
) -> Result<CodexAccount, String> {
    let account_name =
        normalize_optional_value(Some(account_name)).ok_or("账户名不能为空".to_string())?;
    let access_token = normalize_optional_value(Some(access_token))
        .ok_or("Codex access token 不能为空".to_string())?;
    if !is_importable_access_token(&access_token) {
        return Err("无效的 Codex access token".to_string());
    }

    upsert_account_from_access_token_with_hints(
        access_token,
        CodexAccessTokenImportHints {
            account_name: Some(account_name),
            ..Default::default()
        },
    )
}

fn upsert_account_from_access_token_with_hints(
    access_token: String,
    hints: CodexAccessTokenImportHints,
) -> Result<CodexAccount, String> {
    let note_update = codex_account_note_update_from_hints(&hints);
    let access_token =
        normalize_optional_value(Some(access_token)).ok_or("accessToken 不能为空")?;
    let (
        token_email,
        token_user_id,
        token_plan_type,
        token_subscription,
        token_account_id,
        token_org_id,
    ) = extract_access_token_identity(&access_token);
    let account_id = normalize_optional_value(token_account_id.or(hints.account_id.clone()));
    let organization_id = normalize_optional_value(token_org_id.or(hints.organization_id.clone()));
    let email = token_email
        .or(hints.email.clone())
        .or_else(|| account_id.as_ref().map(|value| format!("codex-{}", value)))
        .or_else(|| {
            token_user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .or_else(|| {
            hints
                .user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .unwrap_or_else(|| format!("codex-access-{}", access_token_fingerprint(&access_token)));
    let user_id = normalize_optional_value(token_user_id.or(hints.user_id.clone()));
    let plan_type = normalize_optional_value(token_plan_type.or(hints.plan_type.clone()));
    let subscription_active_until =
        normalize_optional_value(token_subscription.or(hints.subscription_active_until.clone()));
    let mut tokens = CodexTokens {
        id_token: String::new(),
        access_token,
        refresh_token: None,
    };

    let mut index = load_account_index();
    let generated_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let existing_id = find_existing_account_id(
        &index,
        &email,
        account_id.as_deref(),
        organization_id.as_deref(),
    )
    .unwrap_or_else(|| generated_id.clone());

    let mut account = if let Some(mut acc) = load_account(&existing_id) {
        tokens = retain_existing_refresh_token_if_missing(tokens, Some(&acc));
        acc.tokens = tokens;
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        if hints.account_name.is_some() {
            acc.account_name = hints.account_name.clone();
        }
        if hints.account_structure.is_some() {
            acc.account_structure = hints.account_structure.clone();
        }
        acc.update_last_used();
        acc
    } else {
        tokens = retain_existing_refresh_token_if_missing(tokens, None);
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.account_name = hints.account_name.clone();
        acc.account_structure = hints.account_structure.clone();

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            subscription_active_until: subscription_active_until.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };
    apply_account_note_update_if_present(&mut account, note_update);

    save_account(&account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex accessToken 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    Ok(account)
}

async fn import_codex_candidate(
    candidate: CodexJsonImportCandidate,
) -> Result<CodexAccount, String> {
    match candidate {
        CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            note_update,
        } => {
            let mut account = upsert_account_with_hints(tokens, account_id_hint, None)?;
            save_account_note_update_if_present(&mut account, note_update)?;
            Ok(account)
        }
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        } => upsert_account_from_access_token_with_hints(access_token, hints),
        CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            note_update,
        } => upsert_account_from_refresh_token(refresh_token, note_update).await,
    }
}

/// 快速待授权行格式：
/// `邮箱----账号密码----2FA秘钥----邮件地址`
/// 也兼容 3 段（无邮件地址）：`邮箱----账号密码----2FA秘钥`
fn try_parse_pending_oauth_delimited_line(line: &str) -> Option<(String, CodexAccountNoteUpdate)> {
    let line = normalize_optional_ref(Some(line))?;
    if !line.contains("----") {
        return None;
    }
    // 避免把 JSON / URL 误判成该格式
    let trimmed_start = line.trim_start();
    if trimmed_start.starts_with('{') || trimmed_start.starts_with('[') {
        return None;
    }

    let parts: Vec<&str> = line.splitn(4, "----").map(str::trim).collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }

    let email = parts[0];
    if email.is_empty() || !email.contains('@') {
        return None;
    }
    // 基础邮箱形态：本地部分与域名均非空
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return None;
    }

    let password = parts.get(1).copied().unwrap_or("").trim();
    let two_factor = parts.get(2).copied().unwrap_or("").trim();
    let mail_url = parts.get(3).copied().unwrap_or("").trim();

    // 至少需要密码或 2FA 之一，避免把普通带 ---- 的 token 误导入为待授权
    if password.is_empty() && two_factor.is_empty() && mail_url.is_empty() {
        return None;
    }

    Some((
        email.to_string(),
        CodexAccountNoteUpdate {
            note: None,
            two_factor_secret: normalize_optional_ref(Some(two_factor)),
            account_password: normalize_optional_ref(Some(password)),
            phone_number: None,
            mail_url: normalize_optional_ref(Some(mail_url)),
        },
    ))
}

async fn import_accounts_from_token_lines(content: &str) -> Result<Vec<CodexAccount>, String> {
    let lines: Vec<String> = content
        .lines()
        .filter_map(|line| normalize_optional_ref(Some(line)))
        .collect();

    if lines.is_empty() {
        return Err("Token 不能为空".to_string());
    }

    let mut accounts = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if let Some((email, update)) = try_parse_pending_oauth_delimited_line(&line) {
            accounts.push(
                create_pending_oauth_account(email, update)
                    .map_err(|err| format!("第 {} 行待授权账号导入失败: {}", index + 1, err))?,
            );
            continue;
        }

        let values = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(serde_json::Value::Array(items)) => items,
            Ok(value) => vec![value],
            Err(_) => vec![serde_json::Value::String(line)],
        };

        for value in values {
            let candidate = extract_codex_import_candidate_from_value(&value).ok_or_else(|| {
                "未找到有效的 Codex Token（需要 session JSON、accessToken/access_token、id_token + access_token，或 refresh_token）"
                    .to_string()
            })?;
            accounts.push(import_codex_candidate(candidate).await?);
        }
    }

    Ok(accounts)
}

fn is_sub2api_codex_oauth_account(value: &serde_json::Value) -> bool {
    let platform = first_json_string(value, &[&["platform"]])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let account_type = first_json_string(value, &[&["type"]])
        .unwrap_or_default()
        .to_ascii_lowercase();

    platform == "openai" && account_type == "oauth"
}

fn looks_like_sub2api_export(value: &serde_json::Value) -> bool {
    let Some(accounts) = value.get("accounts").and_then(|item| item.as_array()) else {
        return false;
    };

    value.get("exported_at").is_some()
        || value.get("proxies").is_some()
        || accounts
            .iter()
            .any(|item| item.get("credentials").is_some() && item.get("platform").is_some())
}

async fn import_sub2api_export_from_value(
    value: &serde_json::Value,
) -> Result<Option<Vec<CodexAccount>>, String> {
    if !looks_like_sub2api_export(value) {
        return Ok(None);
    }

    let accounts = value
        .get("accounts")
        .and_then(|item| item.as_array())
        .ok_or("Sub2API JSON 缺少 accounts 数组")?;
    let mut imported = Vec::new();

    for (index, item) in accounts.iter().enumerate() {
        if !is_sub2api_codex_oauth_account(item) {
            continue;
        }
        let candidate = extract_codex_import_candidate_from_value(item).ok_or_else(|| {
            format!(
                "Sub2API 第 {} 个 OpenAI OAuth 账号缺少有效 access_token",
                index + 1
            )
        })?;
        imported.push(import_codex_candidate(candidate).await?);
    }

    if imported.is_empty() {
        return Err("Sub2API JSON 中未找到可导入的 OpenAI OAuth access_token".to_string());
    }

    Ok(Some(imported))
}

async fn import_account_from_json_value(
    value: serde_json::Value,
) -> Result<Option<CodexAccount>, String> {
    if let Some(account) = pending_oauth_account_from_value(&value) {
        return Ok(Some(import_account_struct(account)?));
    }

    if is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("authMode").and_then(|value| value.as_str())),
    ) {
        if let Some(api_key) = value
            .get("OPENAI_API_KEY")
            .and_then(|value| value.as_str())
            .and_then(normalize_api_key)
        {
            let mut account = upsert_api_key_account(
                api_key,
                extract_api_base_url_from_json_value(&value),
                read_codex_api_provider_mode(&value),
                value
                    .get("api_provider_id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                value
                    .get("api_provider_name")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
            )?;
            apply_api_key_import_metadata(&mut account, &value);
            save_account(&account)?;
            update_account_plan_type_in_index(
                &account.id,
                &account.plan_type,
                &account.subscription_active_until,
            )?;
            return Ok(Some(account));
        }
    }

    if let Some(candidate) = extract_codex_import_candidate_from_value(&value) {
        return Ok(Some(import_codex_candidate(candidate).await?));
    }

    if let Ok(account) = serde_json::from_value::<CodexAccount>(value) {
        return Ok(Some(import_account_struct(account)?));
    }

    Ok(None)
}

fn parse_line_delimited_json_values(
    json_content: &str,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    let lines: Vec<(usize, &str)> = json_content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((index + 1, trimmed))
            }
        })
        .collect();

    if lines.len() <= 1 {
        return Ok(None);
    }

    let mut values = Vec::with_capacity(lines.len());
    for (line_number, line) in lines {
        let parsed = serde_json::from_str::<serde_json::Value>(line)
            .map_err(|e| format!("第 {} 行不是有效 JSON: {}", line_number, e))?;
        if !parsed.is_object() {
            return Err(format!("第 {} 行不是 JSON 对象", line_number));
        }
        values.push(parsed);
    }

    Ok(Some(values))
}

/// 从 JSON 字符串导入账号
pub async fn import_from_json(json_content: &str) -> Result<Vec<CodexAccount>, String> {
    ensure_storage_writable_for_import()?;
    if !json_content.trim().is_empty()
        && !json_content.trim_start().starts_with('{')
        && !json_content.trim_start().starts_with('[')
    {
        return import_accounts_from_token_lines(json_content).await;
    }

    // 尝试解析为 auth.json 格式
    if let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(json_content) {
        let raw_value = serde_json::from_str::<serde_json::Value>(json_content).ok();
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = if let Some(value) = raw_value.as_ref() {
            infer_api_provider_config(
                extract_api_base_url_from_auth_file(&auth_file).as_deref(),
                read_codex_api_provider_mode(value),
                value.get("api_provider_id").and_then(|item| item.as_str()),
                value
                    .get("api_provider_name")
                    .and_then(|item| item.as_str()),
            )
        } else {
            infer_api_provider_config(
                extract_api_base_url_from_auth_file(&auth_file).as_deref(),
                None,
                None,
                None,
            )
        };
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            let mut account = upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
            )?;
            if let Some(value) = raw_value.as_ref() {
                apply_api_key_import_metadata(&mut account, value);
                save_account(&account)?;
                update_account_plan_type_in_index(
                    &account.id,
                    &account.plan_type,
                    &account.subscription_active_until,
                )?;
            }
            return Ok(vec![account]);
        }

        if let Some(tokens) = auth_file.tokens {
            let mut account = upsert_account_from_auth_tokens(tokens)?;
            if let Some(value) = raw_value.as_ref() {
                save_account_note_update_if_present(
                    &mut account,
                    codex_account_note_update_from_value(value),
                )?;
            }
            return Ok(vec![account]);
        }

        if let Some(api_key) = fallback_api_key {
            let mut account = upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
            )?;
            if let Some(value) = raw_value.as_ref() {
                apply_api_key_import_metadata(&mut account, value);
                save_account(&account)?;
                update_account_plan_type_in_index(
                    &account.id,
                    &account.plan_type,
                    &account.subscription_active_until,
                )?;
            }
            return Ok(vec![account]);
        }
    }

    // 尝试解析为单账号（顶层 token）或通用数组（支持混合对象）
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_content) {
        if let Some(accounts) = import_sub2api_export_from_value(&parsed).await? {
            return Ok(accounts);
        }

        match parsed {
            serde_json::Value::Object(_) => {
                if let Some(account) = import_account_from_json_value(parsed).await? {
                    return Ok(vec![account]);
                }
            }
            serde_json::Value::Array(items) => {
                let mut result = Vec::new();

                for item in items {
                    if let Some(account) = import_account_from_json_value(item).await? {
                        result.push(account);
                    }
                }

                if !result.is_empty() {
                    return Ok(result);
                }
            }
            _ => {}
        }
    }

    if let Some(items) = parse_line_delimited_json_values(json_content)? {
        let mut result = Vec::new();

        for (index, item) in items.into_iter().enumerate() {
            match import_account_from_json_value(item).await? {
                Some(account) => result.push(account),
                None => {
                    return Err(format!(
                        "第 {} 行未找到有效的 Codex Token（需要 session JSON、accessToken/access_token、id_token + access_token，或 refresh_token）",
                        index + 1
                    ));
                }
            }
        }

        if !result.is_empty() {
            return Ok(result);
        }
    }

    Err("无法解析 JSON 内容".to_string())
}

/// 导出账号为 JSON
pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<CodexAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();

    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportResult {
    pub imported: Vec<CodexAccount>,
    pub failed: Vec<CodexFileImportFailure>,
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportFailure {
    pub email: String,
    pub error: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportStartResult {
    pub session_id: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportItem {
    pub item_id: String,
    pub source: String,
    pub label: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub account_type: String,
    pub provider: Option<String>,
    pub quota_status: String,
    pub quota_error: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub default_selected: bool,
    pub selectable: bool,
    pub existing: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportProgress {
    pub session_id: String,
    pub phase: String,
    pub check_quota: bool,
    pub current: usize,
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub quota_failed: usize,
    pub existing: usize,
    pub current_label: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportPreview {
    pub session_id: String,
    pub status: String,
    pub check_quota: bool,
    pub total: usize,
    pub items: Vec<CodexBatchImportItem>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportConfirmResult {
    pub imported: Vec<CodexAccount>,
    pub failed: Vec<CodexFileImportFailure>,
    pub cancelled: bool,
    pub processed: usize,
    pub total: usize,
}

#[derive(Clone)]
struct CodexBatchImportSession {
    status: String,
    check_quota: bool,
    cancel: Arc<AtomicBool>,
    source_items: Vec<CodexBatchImportSourceItem>,
    next_index: usize,
    total: usize,
    items: Vec<CodexBatchImportCachedItem>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CodexBatchImportSourceItem {
    source: String,
    value: serde_json::Value,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CodexBatchImportCachedItem {
    preview: CodexBatchImportItem,
    draft: Option<CodexBatchImportDraft>,
    quota: Option<crate::models::codex::CodexQuota>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum CodexBatchImportDraft {
    Account(CodexAccount),
    FullToken {
        tokens: CodexTokens,
        account_id_hint: Option<String>,
        #[serde(default)]
        note_update: CodexAccountNoteUpdate,
    },
    AccessToken {
        access_token: String,
        hints: CodexAccessTokenImportHints,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBatchImportSessionSnapshot {
    version: u32,
    status: String,
    check_quota: bool,
    source_items: Vec<CodexBatchImportSourceItem>,
    next_index: usize,
    total: usize,
    items: Vec<CodexBatchImportCachedItem>,
    updated_at: i64,
}

fn next_codex_batch_import_session_id() -> String {
    let id = CODEX_BATCH_IMPORT_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!(
        "codex-import-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        id
    )
}

fn get_codex_batch_import_sessions_dir() -> PathBuf {
    let data_dir = account::get_data_dir()
        .or_else(|_| account::resolve_data_dir())
        .unwrap_or_else(|_| PathBuf::from(".antigravity_cockpit"));
    data_dir.join(CODEX_BATCH_IMPORT_SESSIONS_DIR)
}

fn sanitize_codex_batch_import_session_id(session_id: &str) -> Result<String, String> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err("导入会话 ID 为空".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("导入会话 ID 不合法".to_string());
    }
    Ok(trimmed.to_string())
}

fn codex_batch_import_session_snapshot_path(session_id: &str) -> Result<PathBuf, String> {
    let safe_id = sanitize_codex_batch_import_session_id(session_id)?;
    Ok(get_codex_batch_import_sessions_dir().join(format!("{}.json", safe_id)))
}

fn ensure_codex_batch_import_sessions_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "创建导入会话目录失败: path={} 不是目录",
            path.display()
        ));
    }
    fs::create_dir(path).map_err(|error| {
        format!(
            "创建导入会话目录失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn codex_batch_import_snapshot_from_session(
    session: &CodexBatchImportSession,
) -> CodexBatchImportSessionSnapshot {
    CodexBatchImportSessionSnapshot {
        version: 1,
        status: session.status.clone(),
        check_quota: session.check_quota,
        source_items: session.source_items.clone(),
        next_index: session.next_index,
        total: session.total,
        items: session.items.clone(),
        updated_at: chrono::Utc::now().timestamp(),
    }
}

fn codex_batch_import_session_from_snapshot(
    snapshot: CodexBatchImportSessionSnapshot,
) -> CodexBatchImportSession {
    let status = if snapshot.status == "scanning" {
        "cancelled".to_string()
    } else {
        snapshot.status
    };
    CodexBatchImportSession {
        status,
        check_quota: snapshot.check_quota,
        cancel: Arc::new(AtomicBool::new(false)),
        source_items: snapshot.source_items,
        next_index: snapshot.next_index,
        total: snapshot.total,
        items: snapshot.items,
    }
}

fn save_codex_batch_import_session_snapshot(
    session_id: &str,
    session: &CodexBatchImportSession,
) -> Result<(), String> {
    let path = codex_batch_import_session_snapshot_path(session_id)?;
    if let Some(parent) = path.parent() {
        ensure_codex_batch_import_sessions_dir(parent)?;
    }
    let snapshot = codex_batch_import_snapshot_from_session(session);
    let content = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("序列化导入会话快照失败: {}", error))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).map_err(|error| {
        format!(
            "写入导入会话快照失败: path={}, error={}",
            tmp_path.display(),
            error
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "更新导入会话快照失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn save_codex_batch_import_session_snapshot_best_effort(
    session_id: &str,
    session: &CodexBatchImportSession,
) {
    if let Err(error) = save_codex_batch_import_session_snapshot(session_id, session) {
        logger::log_warn(&format!(
            "[Codex Batch Import] 保存导入会话快照失败: session_id={}, error={}",
            session_id, error
        ));
    }
}

fn load_codex_batch_import_session_snapshot(
    session_id: &str,
) -> Result<Option<CodexBatchImportSession>, String> {
    let path = codex_batch_import_session_snapshot_path(session_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取导入会话快照失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    let snapshot: CodexBatchImportSessionSnapshot =
        serde_json::from_str(&content).map_err(|error| {
            format!(
                "解析导入会话快照失败: path={}, error={}",
                path.display(),
                error
            )
        })?;
    Ok(Some(codex_batch_import_session_from_snapshot(snapshot)))
}

fn remove_codex_batch_import_session_snapshot(session_id: &str) {
    if let Ok(path) = codex_batch_import_session_snapshot_path(session_id) {
        let _ = fs::remove_file(path);
    }
}

fn ensure_codex_batch_import_session_loaded(session_id: &str) -> Result<(), String> {
    {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        if sessions.contains_key(session_id) {
            return Ok(());
        }
    }
    let Some(session) = load_codex_batch_import_session_snapshot(session_id)? else {
        return Err("导入会话不存在".to_string());
    };
    let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
    sessions.entry(session_id.to_string()).or_insert(session);
    Ok(())
}

fn emit_codex_batch_import_progress(app: &tauri::AppHandle, payload: CodexBatchImportProgress) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-progress", payload);
}

fn emit_codex_batch_import_completed(app: &tauri::AppHandle, payload: CodexBatchImportPreview) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-completed", payload);
}

fn emit_codex_batch_import_preview(app: &tauri::AppHandle, payload: CodexBatchImportPreview) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-preview", payload);
}

fn codex_batch_import_preview_from_session(
    session_id: &str,
    session: &CodexBatchImportSession,
) -> CodexBatchImportPreview {
    CodexBatchImportPreview {
        session_id: session_id.to_string(),
        status: session.status.clone(),
        check_quota: session.check_quota,
        total: session.total,
        items: session
            .items
            .iter()
            .map(|item| item.preview.clone())
            .collect(),
    }
}

fn codex_batch_import_progress_from_items(
    session_id: &str,
    phase: &str,
    check_quota: bool,
    current: usize,
    total: usize,
    items: &[CodexBatchImportCachedItem],
    current_label: Option<String>,
) -> CodexBatchImportProgress {
    CodexBatchImportProgress {
        session_id: session_id.to_string(),
        phase: phase.to_string(),
        check_quota,
        current,
        total,
        success: items
            .iter()
            .filter(|item| item.preview.status == "ready")
            .count(),
        failed: items
            .iter()
            .filter(|item| item.preview.status == "invalid")
            .count(),
        quota_failed: items
            .iter()
            .filter(|item| item.preview.status == "quota_failed")
            .count(),
        existing: items.iter().filter(|item| item.preview.existing).count(),
        current_label,
    }
}

fn preview_account_from_full_tokens(
    mut tokens: CodexTokens,
    account_id_hint: Option<String>,
    note_update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let (
        email,
        user_id,
        plan_type,
        subscription_active_until,
        id_token_account_id,
        id_token_org_id,
    ) = extract_user_info(&tokens.id_token)?;
    let account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&tokens.access_token)
            .or(id_token_account_id)
            .or(account_id_hint),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token).or(id_token_org_id),
    );
    tokens = retain_existing_refresh_token_if_missing(tokens, None);
    let storage_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let mut account = CodexAccount::new(storage_id, email, tokens);
    mark_token_chain_updated(&mut account);
    account.auth_mode = CodexAuthMode::OAuth;
    account.user_id = user_id;
    account.plan_type = plan_type;
    account.subscription_active_until = subscription_active_until;
    account.account_id = account_id;
    account.organization_id = organization_id;
    apply_account_note_update_if_present(&mut account, note_update);
    Ok(account)
}

fn preview_account_from_access_token(
    access_token: String,
    hints: CodexAccessTokenImportHints,
) -> Result<CodexAccount, String> {
    let access_token =
        normalize_optional_value(Some(access_token)).ok_or("accessToken 不能为空")?;
    let (
        token_email,
        token_user_id,
        token_plan_type,
        token_subscription,
        token_account_id,
        token_org_id,
    ) = extract_access_token_identity(&access_token);
    let account_id = normalize_optional_value(token_account_id.or(hints.account_id.clone()));
    let organization_id = normalize_optional_value(token_org_id.or(hints.organization_id.clone()));
    let email = token_email
        .or(hints.email.clone())
        .or_else(|| account_id.as_ref().map(|value| format!("codex-{}", value)))
        .or_else(|| {
            token_user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .or_else(|| {
            hints
                .user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .unwrap_or_else(|| format!("codex-access-{}", access_token_fingerprint(&access_token)));
    let tokens = CodexTokens {
        id_token: String::new(),
        access_token,
        refresh_token: None,
    };
    let storage_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let mut account = CodexAccount::new(storage_id, email, tokens);
    mark_token_chain_updated(&mut account);
    account.auth_mode = CodexAuthMode::OAuth;
    account.authorization_status = None;
    account.user_id = normalize_optional_value(token_user_id.or(hints.user_id));
    account.plan_type = normalize_optional_value(token_plan_type.or(hints.plan_type));
    account.subscription_active_until =
        normalize_optional_value(token_subscription.or(hints.subscription_active_until));
    account.account_id = account_id;
    account.organization_id = organization_id;
    account.account_name = hints.account_name;
    account.account_structure = hints.account_structure;
    account.account_note = hints.account_note;
    account.two_factor_secret = hints.two_factor_secret;
    account.account_password = hints.account_password;
    account.phone_number = hints.phone_number;
    account.mail_url = hints.mail_url;
    Ok(account)
}

fn preview_account_for_draft(draft: &CodexBatchImportDraft) -> Result<CodexAccount, String> {
    match draft {
        CodexBatchImportDraft::Account(account) => Ok(account.clone()),
        CodexBatchImportDraft::FullToken {
            tokens,
            account_id_hint,
            note_update,
        } => preview_account_from_full_tokens(
            tokens.clone(),
            account_id_hint.clone(),
            note_update.clone(),
        ),
        CodexBatchImportDraft::AccessToken {
            access_token,
            hints,
        } => preview_account_from_access_token(access_token.clone(), hints.clone()),
    }
}

fn codex_batch_import_draft_from_candidate(
    candidate: CodexJsonImportCandidate,
) -> CodexBatchImportDraft {
    match candidate {
        CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            note_update,
        } => CodexBatchImportDraft::FullToken {
            tokens,
            account_id_hint,
            note_update,
        },
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        } => CodexBatchImportDraft::AccessToken {
            access_token,
            hints,
        },
        CodexJsonImportCandidate::RefreshToken { .. } => {
            unreachable!("refresh_token candidates are resolved before creating a draft")
        }
    }
}

fn api_key_draft_from_value(
    value: &serde_json::Value,
    fallback_id: Option<String>,
) -> Result<Option<CodexBatchImportDraft>, String> {
    if !is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("authMode").and_then(|value| value.as_str())),
    ) {
        return Ok(None);
    }
    let Some(api_key) = value
        .get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .and_then(normalize_api_key)
    else {
        return Ok(None);
    };
    let (api_key, api_base_url) = validate_api_key_credentials(
        &api_key,
        extract_api_base_url_from_json_value(value).as_deref(),
    )?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        read_codex_api_provider_mode(value),
        value
            .get("api_provider_id")
            .and_then(|value| value.as_str()),
        value
            .get("api_provider_name")
            .and_then(|value| value.as_str()),
    )?;
    let mut account = CodexAccount::new_api_key(
        fallback_id.unwrap_or_else(|| build_api_key_account_id(&api_key)),
        read_json_string(value, &["email", "account_email"])
            .unwrap_or_else(|| build_api_key_email(&api_key)),
        api_key,
        provider_config.mode,
        provider_config.base_url,
        provider_config.provider_id,
        provider_config.provider_name,
        Vec::new(),
    );
    apply_api_key_import_metadata(&mut account, value);
    Ok(Some(CodexBatchImportDraft::Account(account)))
}

async fn codex_batch_import_draft_from_value(
    value: serde_json::Value,
) -> Result<Option<CodexBatchImportDraft>, String> {
    if let Some(account) = pending_oauth_account_from_value(&value) {
        return Ok(Some(CodexBatchImportDraft::Account(account)));
    }

    if let Ok(auth_file) = serde_json::from_value::<CodexAuthFile>(value.clone()) {
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = infer_api_provider_config(
            extract_api_base_url_from_auth_file(&auth_file).as_deref(),
            read_codex_api_provider_mode(&value),
            value.get("api_provider_id").and_then(|item| item.as_str()),
            value
                .get("api_provider_name")
                .and_then(|item| item.as_str()),
        );
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            let mut account = CodexAccount::new_api_key(
                build_api_key_account_id(&api_key),
                build_api_key_email(&api_key),
                api_key,
                fallback_provider.mode,
                fallback_provider.base_url,
                fallback_provider.provider_id,
                fallback_provider.provider_name,
                Vec::new(),
            );
            apply_api_key_import_metadata(&mut account, &value);
            return Ok(Some(CodexBatchImportDraft::Account(account)));
        }
        if let Some(tokens) = auth_file.tokens {
            let account_id_hint = tokens.account_id.clone();
            let tokens = CodexTokens {
                id_token: tokens.id_token,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
            };
            if normalize_optional_ref(Some(&tokens.id_token)).is_none()
                && is_importable_access_token(&tokens.access_token)
            {
                let note_update = codex_account_note_update_from_value(&value);
                return Ok(Some(CodexBatchImportDraft::AccessToken {
                    access_token: tokens.access_token,
                    hints: CodexAccessTokenImportHints {
                        account_id: account_id_hint,
                        account_note: note_update.note,
                        two_factor_secret: note_update.two_factor_secret,
                        account_password: note_update.account_password,
                        phone_number: note_update.phone_number,
                        mail_url: note_update.mail_url,
                        ..Default::default()
                    },
                }));
            }
            return Ok(Some(CodexBatchImportDraft::FullToken {
                tokens,
                account_id_hint,
                note_update: codex_account_note_update_from_value(&value),
            }));
        }
        if let Some(api_key) = fallback_api_key {
            let mut account = CodexAccount::new_api_key(
                build_api_key_account_id(&api_key),
                build_api_key_email(&api_key),
                api_key,
                fallback_provider.mode,
                fallback_provider.base_url,
                fallback_provider.provider_id,
                fallback_provider.provider_name,
                Vec::new(),
            );
            apply_api_key_import_metadata(&mut account, &value);
            return Ok(Some(CodexBatchImportDraft::Account(account)));
        }
    }

    if let Some(draft) = api_key_draft_from_value(&value, None)? {
        return Ok(Some(draft));
    }

    if let Some(candidate) = extract_codex_import_candidate_from_value(&value) {
        return match candidate {
            CodexJsonImportCandidate::RefreshToken {
                refresh_token,
                note_update,
            } => {
                let tokens = codex_oauth::refresh_access_token(&refresh_token).await?;
                Ok(Some(CodexBatchImportDraft::FullToken {
                    tokens,
                    account_id_hint: None,
                    note_update,
                }))
            }
            other => Ok(Some(codex_batch_import_draft_from_candidate(other))),
        };
    }

    if let Ok(account) = serde_json::from_value::<CodexAccount>(value) {
        return Ok(Some(CodexBatchImportDraft::Account(account)));
    }

    Ok(None)
}

fn codex_batch_import_values_from_content(content: &str) -> Result<Vec<serde_json::Value>, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let mut values = Vec::new();
        for line in trimmed
            .lines()
            .filter_map(|line| normalize_optional_ref(Some(line)))
        {
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(serde_json::Value::Array(items)) => values.extend(items),
                Ok(value) => values.push(value),
                Err(_) => values.push(serde_json::Value::String(line)),
            }
        }
        return Ok(values);
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            if looks_like_sub2api_export(&value) {
                let accounts = value
                    .get("accounts")
                    .and_then(|item| item.as_array())
                    .ok_or("Sub2API JSON 缺少 accounts 数组")?;
                return Ok(accounts
                    .iter()
                    .filter(|item| is_sub2api_codex_oauth_account(item))
                    .cloned()
                    .collect());
            }
            match value {
                serde_json::Value::Array(items) => Ok(items),
                other => Ok(vec![other]),
            }
        }
        Err(_) => parse_line_delimited_json_values(trimmed).map(|items| items.unwrap_or_default()),
    }
}

fn codex_batch_import_account_type(account: &CodexAccount) -> String {
    if account.is_api_key_auth() {
        "API Key".to_string()
    } else if normalize_optional_ref(account.tokens.refresh_token.as_deref()).is_some() {
        "OAuth".to_string()
    } else {
        "Access Token".to_string()
    }
}

async fn build_codex_batch_import_item(
    session_id: &str,
    index: usize,
    source: String,
    value: serde_json::Value,
    check_quota: bool,
) -> CodexBatchImportCachedItem {
    let item_id = format!("{}-item-{}", session_id, index + 1);
    let draft = match codex_batch_import_draft_from_value(value).await {
        Ok(Some(draft)) => draft,
        Ok(None) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "未识别账号".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some("未找到有效的 Codex 账号凭据".to_string()),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
        Err(error) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "解析失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
    };

    let account = match preview_account_for_draft(&draft) {
        Ok(account) => account,
        Err(error) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "解析失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
    };

    let existing = load_account(&account.id).is_some();
    let (quota_status, quota_error, quota, status) = if check_quota {
        let quota_result = crate::modules::codex_quota::probe_import_account_quota(&account).await;
        let (quota_status, quota_error, quota) = match quota_result {
            Ok(quota) => ("success".to_string(), None, Some(quota)),
            Err(error) => ("failed".to_string(), Some(error), None),
        };
        let status = if quota_status == "failed" {
            "quota_failed".to_string()
        } else if existing {
            "existing".to_string()
        } else {
            "ready".to_string()
        };
        (quota_status, quota_error, quota, status)
    } else if existing {
        ("skipped".to_string(), None, None, "existing".to_string())
    } else {
        ("skipped".to_string(), None, None, "ready".to_string())
    };
    let default_selected = status == "ready" || status == "existing";
    CodexBatchImportCachedItem {
        preview: CodexBatchImportItem {
            item_id,
            source,
            label: account
                .account_name
                .clone()
                .unwrap_or_else(|| account.email.clone()),
            account_id: Some(account.id.clone()),
            email: Some(account.email.clone()),
            account_type: codex_batch_import_account_type(&account),
            provider: account
                .api_provider_name
                .clone()
                .or(account.api_provider_id.clone())
                .or(account.api_base_url.clone()),
            quota_status,
            quota_error,
            status,
            error: None,
            default_selected,
            selectable: true,
            existing,
        },
        draft: Some(draft),
        quota,
    }
}

async fn run_codex_batch_import_scan(
    app: tauri::AppHandle,
    session_id: String,
    file_paths: Vec<String>,
    check_quota: bool,
) {
    let cancel = {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|session| session.cancel.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(true)))
    };
    let mut values: Vec<CodexBatchImportSourceItem> = Vec::new();
    let mut read_failures: Vec<CodexBatchImportCachedItem> = Vec::new();

    for file_path in file_paths {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let path = Path::new(&file_path);
        let source = path
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or(&file_path)
            .to_string();
        match fs::read_to_string(path) {
            Ok(content) => match codex_batch_import_values_from_content(&content) {
                Ok(items) => {
                    values.extend(items.into_iter().map(|item| CodexBatchImportSourceItem {
                        source: source.clone(),
                        value: item,
                    }));
                }
                Err(error) => read_failures.push(CodexBatchImportCachedItem {
                    preview: CodexBatchImportItem {
                        item_id: format!("{}-file-error-{}", session_id, read_failures.len() + 1),
                        source,
                        label: "文件解析失败".to_string(),
                        account_id: None,
                        email: None,
                        account_type: "-".to_string(),
                        provider: None,
                        quota_status: "skipped".to_string(),
                        quota_error: None,
                        status: "invalid".to_string(),
                        error: Some(error),
                        default_selected: false,
                        selectable: false,
                        existing: false,
                    },
                    draft: None,
                    quota: None,
                }),
            },
            Err(error) => read_failures.push(CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id: format!("{}-file-error-{}", session_id, read_failures.len() + 1),
                    source,
                    label: "文件读取失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error.to_string()),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            }),
        }
    }

    let total = values.len() + read_failures.len();
    let session_snapshot = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        if let Some(session) = sessions.get_mut(&session_id) {
            session.source_items = values;
            session.next_index = 0;
            session.total = total;
            session.items = read_failures;
            session.check_quota = check_quota;
            Some(session.clone())
        } else {
            None
        }
    };
    if let Some(session) = session_snapshot {
        save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
    }
    run_codex_batch_import_resume(app, session_id).await;
}

async fn run_codex_batch_import_resume(app: tauri::AppHandle, session_id: String) {
    let (cancel, check_quota, source_items, start_index, mut items, total, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let Some(session) = sessions.get_mut(&session_id) else {
            return;
        };
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "scanning".to_string();
        (
            session.cancel.clone(),
            session.check_quota,
            session.source_items.clone(),
            session.next_index,
            session.items.clone(),
            session.total,
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session_snapshot);

    emit_codex_batch_import_progress(
        &app,
        codex_batch_import_progress_from_items(
            &session_id,
            "scanning",
            check_quota,
            items.len(),
            total,
            &items,
            None,
        ),
    );

    for (index, source_item) in source_items.into_iter().enumerate().skip(start_index) {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let cached = build_codex_batch_import_item(
            &session_id,
            index,
            source_item.source,
            source_item.value,
            check_quota,
        )
        .await;
        let current_label = Some(cached.preview.label.clone());
        items.push(cached);
        let session_snapshot = {
            let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
            if let Some(session) = sessions.get_mut(&session_id) {
                session.next_index = index + 1;
                session.items = items.clone();
                Some(session.clone())
            } else {
                None
            }
        };
        if let Some(session) = session_snapshot {
            save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
        }
        emit_codex_batch_import_progress(
            &app,
            codex_batch_import_progress_from_items(
                &session_id,
                "scanning",
                check_quota,
                items.len(),
                total,
                &items,
                current_label,
            ),
        );
        let preview = {
            let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
            sessions
                .get(&session_id)
                .map(|session| codex_batch_import_preview_from_session(&session_id, session))
        };
        if let Some(preview) = preview {
            emit_codex_batch_import_preview(&app, preview);
        }
    }

    let status = if cancel.load(Ordering::SeqCst) {
        "cancelled"
    } else if {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|session| session.next_index < session.source_items.len())
            .unwrap_or(false)
    } {
        "cancelled"
    } else {
        "ready"
    };
    let (preview, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session =
            sessions
                .entry(session_id.clone())
                .or_insert_with(|| CodexBatchImportSession {
                    status: status.to_string(),
                    check_quota,
                    cancel: cancel.clone(),
                    source_items: Vec::new(),
                    next_index: 0,
                    total: items.len(),
                    items: Vec::new(),
                });
        session.status = status.to_string();
        session.items = items;
        (
            codex_batch_import_preview_from_session(&session_id, session),
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session_snapshot);
    emit_codex_batch_import_completed(&app, preview);
}

pub fn start_codex_batch_import_from_files(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    check_quota: bool,
) -> Result<CodexBatchImportStartResult, String> {
    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;
    let session_id = next_codex_batch_import_session_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let session = CodexBatchImportSession {
        status: "scanning".to_string(),
        check_quota,
        cancel,
        source_items: Vec::new(),
        next_index: 0,
        total: 0,
        items: Vec::new(),
    };
    // 会话快照用于崩溃恢复，失败时保留当前进程内任务，不能阻断批量导入。
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
    {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions.insert(session_id.clone(), session);
    }
    let task_session_id = session_id.clone();
    tauri::async_runtime::spawn(async move {
        run_codex_batch_import_scan(app, task_session_id, file_paths, check_quota).await;
    });
    Ok(CodexBatchImportStartResult { session_id })
}

pub fn cancel_codex_batch_import(session_id: &str) -> Result<(), String> {
    ensure_codex_batch_import_session_loaded(session_id)?;
    let session_snapshot = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        session.cancel.store(true, Ordering::SeqCst);
        session.status = "cancelled".to_string();
        session.clone()
    };
    save_codex_batch_import_session_snapshot_best_effort(session_id, &session_snapshot);
    Ok(())
}

pub fn resume_codex_batch_import(app: tauri::AppHandle, session_id: &str) -> Result<(), String> {
    {
        ensure_codex_batch_import_session_loaded(session_id)?;
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        if session.status != "cancelled" {
            return Err("只有已取消的导入会话可以继续".to_string());
        }
        if session.next_index >= session.source_items.len() {
            session.status = "ready".to_string();
            save_codex_batch_import_session_snapshot_best_effort(session_id, session);
            return Ok(());
        }
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "scanning".to_string();
        save_codex_batch_import_session_snapshot_best_effort(session_id, session);
    }

    let task_session_id = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        run_codex_batch_import_resume(app, task_session_id).await;
    });
    Ok(())
}

pub fn get_codex_batch_import_preview(session_id: &str) -> Result<CodexBatchImportPreview, String> {
    ensure_codex_batch_import_session_loaded(session_id)?;
    let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "导入会话不存在".to_string())?;
    Ok(codex_batch_import_preview_from_session(session_id, session))
}

pub fn confirm_codex_batch_import(
    app: &tauri::AppHandle,
    session_id: &str,
    item_ids: &[String],
) -> Result<CodexBatchImportConfirmResult, String> {
    ensure_storage_writable_for_import()?;
    ensure_codex_batch_import_session_loaded(session_id)?;
    let selected: HashSet<String> = item_ids.iter().cloned().collect();
    let (cached_items, cancel, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "importing".to_string();
        (
            session
                .items
                .iter()
                .filter(|cached| selected.contains(&cached.preview.item_id))
                .cloned()
                .collect::<Vec<_>>(),
            session.cancel.clone(),
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(session_id, &session_snapshot);

    let mut imported = Vec::new();
    let mut failed = Vec::new();
    let total = cached_items.len();
    let mut processed = 0usize;
    emit_codex_batch_import_progress(
        app,
        CodexBatchImportProgress {
            session_id: session_id.to_string(),
            phase: "importing".to_string(),
            check_quota: session_snapshot.check_quota,
            current: 0,
            total,
            success: 0,
            failed: 0,
            quota_failed: 0,
            existing: 0,
            current_label: None,
        },
    );

    for cached in cached_items {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let current_label = Some(cached.preview.label.clone());
        let Some(draft) = cached.draft else {
            failed.push(CodexFileImportFailure {
                email: cached.preview.label,
                error: cached
                    .preview
                    .error
                    .unwrap_or_else(|| "无可导入账号".to_string()),
            });
            processed += 1;
            emit_codex_batch_import_progress(
                app,
                CodexBatchImportProgress {
                    session_id: session_id.to_string(),
                    phase: "importing".to_string(),
                    check_quota: session_snapshot.check_quota,
                    current: processed,
                    total,
                    success: imported.len(),
                    failed: failed.len(),
                    quota_failed: 0,
                    existing: 0,
                    current_label,
                },
            );
            continue;
        };
        let result = (|| -> Result<CodexAccount, String> {
            let mut account = match draft {
                CodexBatchImportDraft::Account(account) => import_account_struct(account)?,
                CodexBatchImportDraft::FullToken {
                    tokens,
                    account_id_hint,
                    note_update,
                } => {
                    let mut account = upsert_account_with_hints(tokens, account_id_hint, None)?;
                    save_account_note_update_if_present(&mut account, note_update)?;
                    account
                }
                CodexBatchImportDraft::AccessToken {
                    access_token,
                    hints,
                } => upsert_account_from_access_token_with_hints(access_token, hints)?,
            };
            if let Some(quota) = cached.quota.clone() {
                account.quota = Some(quota);
                account.quota_error = None;
                account.usage_updated_at = Some(chrono::Utc::now().timestamp());
                save_account(&account)?;
            }
            Ok(account)
        })();
        match result {
            Ok(account) => imported.push(account),
            Err(error) => failed.push(CodexFileImportFailure {
                email: cached.preview.label,
                error,
            }),
        }
        processed += 1;
        emit_codex_batch_import_progress(
            app,
            CodexBatchImportProgress {
                session_id: session_id.to_string(),
                phase: "importing".to_string(),
                check_quota: session_snapshot.check_quota,
                current: processed,
                total,
                success: imported.len(),
                failed: failed.len(),
                quota_failed: 0,
                existing: 0,
                current_label,
            },
        );
    }
    let cancelled = cancel.load(Ordering::SeqCst);

    {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions.remove(session_id);
    }
    remove_codex_batch_import_session_snapshot(session_id);

    Ok(CodexBatchImportConfirmResult {
        imported,
        failed,
        cancelled,
        processed,
        total,
    })
}

fn normalize_auth_file_plan_type(value: Option<&str>) -> Option<String> {
    let normalized = normalize_optional_ref(value)?
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-");

    match normalized.as_str() {
        "prolite" | "pro-lite" => Some("prolite".to_string()),
        "promax" | "pro-max" => Some("promax".to_string()),
        _ => None,
    }
}

fn detect_auth_file_plan_type_from_path(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let normalized = stem
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-");

    if normalized.ends_with("-prolite") || normalized.ends_with("-pro-lite") {
        return Some("prolite".to_string());
    }
    if normalized.ends_with("-promax") || normalized.ends_with("-pro-max") {
        return Some("promax".to_string());
    }

    None
}

fn apply_auth_file_plan_type(
    account: &mut CodexAccount,
    auth_file_plan_type: Option<String>,
) -> bool {
    let Some(normalized) = normalize_auth_file_plan_type(auth_file_plan_type.as_deref()) else {
        return false;
    };

    if account.auth_file_plan_type.as_deref() == Some(normalized.as_str()) {
        return false;
    }

    account.auth_file_plan_type = Some(normalized);
    true
}

/// 从单个 JSON 值中提取 CodexTokens
fn extract_codex_tokens_from_value(
    value: &serde_json::Value,
) -> Option<(CodexTokens, Option<String>)> {
    let obj = value.as_object()?;

    // 格式1: 顶层 access_token + id_token（用户导出格式）
    if let (Some(id_token), Some(access_token)) = (
        first_json_string(value, &[&["id_token"], &["idToken"]]),
        first_json_string(value, &[&["access_token"], &["accessToken"]]),
    ) {
        let refresh_token = first_json_string(value, &[&["refresh_token"], &["refreshToken"]]);
        let account_id_hint = first_json_string(value, &[&["account_id"], &["accountId"]]);
        return Some((
            CodexTokens {
                id_token,
                access_token,
                refresh_token,
            },
            account_id_hint,
        ));
    }

    // 格式2: 嵌套 tokens 对象（CodexAuthFile 或 CodexAccount 格式）
    if obj.get("tokens").and_then(|v| v.as_object()).is_some() {
        if let (Some(id_token), Some(access_token)) = (
            first_json_string(value, &[&["tokens", "id_token"], &["tokens", "idToken"]]),
            first_json_string(
                value,
                &[&["tokens", "access_token"], &["tokens", "accessToken"]],
            ),
        ) {
            let refresh_token = first_json_string(
                value,
                &[&["tokens", "refresh_token"], &["tokens", "refreshToken"]],
            );
            let account_id_hint = first_json_string(
                value,
                &[
                    &["tokens", "account_id"],
                    &["tokens", "accountId"],
                    &["account_id"],
                    &["accountId"],
                ],
            );
            return Some((
                CodexTokens {
                    id_token,
                    access_token,
                    refresh_token,
                },
                account_id_hint,
            ));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        build_account_storage_id, build_auth_file_value, decode_jwt_payload_value,
        detect_auth_file_plan_type_from_path, ensure_managed_account_fresh,
        extract_codex_import_candidate_from_value, extract_codex_tokens_from_value,
        extract_user_info, force_refresh_managed_account_after_observed,
        format_refresh_error_for_user, get_accounts_dir, get_accounts_storage_path,
        get_current_account_from_loaded, import_from_json, is_loopback_http_base_url,
        is_managed_auth_refresh_due, is_pending_oauth_account, list_accounts_checked, load_account,
        load_account_index, looks_like_sub2api_export, now_timestamp, parse_auth_file_last_refresh,
        parse_codex_account_compat, parse_line_delimited_json_values,
        read_api_provider_from_config_toml, read_quick_config_from_config_toml, remove_accounts,
        resolve_api_provider_config, save_account, save_account_index,
        should_accept_authority_snapshot, sync_account_from_auth_dir,
        sync_api_key_account_from_local_state, sync_api_key_provider_accounts,
        sync_managed_projection_from_auth_dir, try_parse_pending_oauth_delimited_line,
        upsert_account, upsert_account_for_reauth, upsert_account_from_access_token,
        upsert_account_from_access_token_with_hints, upsert_account_from_auth_tokens,
        upsert_api_key_account, validate_api_key_credentials, write_account_bundle_to_dir,
        write_api_key_provider_to_config_toml, write_api_provider_to_config_toml,
        write_managed_projection_to_dir, write_prepared_account_bundle_to_dir,
        snapshot_codex_account_store, codex_switch_error_with_rollback,
        write_quick_config_to_config_toml, ApiProviderConfig, CodexAccessTokenImportHints,
        CodexAccountGroupRecord, CodexAccountIndex, CodexAccountSummary, CodexAuthFile,
        CodexAuthTokens, CodexGroupQuotaRefreshPolicy, CodexJsonImportCandidate,
        LocalCodexOAuthSnapshot, CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION,
        CODEX_AUTHORIZATION_STATUS_PENDING, CODEX_AUTO_COMPACT_DEFAULT_LIMIT,
        CODEX_CONTEXT_WINDOW_1M_VALUE, CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER,
        CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE, CODEX_IMAGEGEN_ACTOR_HEADER,
        CODEX_IMAGEGEN_ACTOR_HEADER_VALUE, CODEX_IMAGE_MODEL_ID,
    };
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexTokens};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};
    use toml_edit::Document;

    #[test]
    fn parse_line_delimited_json_values_accepts_one_object_per_line() {
        let raw = r#"{"id_token":"id-1","access_token":"access-1"}
{"id_token":"id-2","access_token":"access-2"}"#;

        let values = parse_line_delimited_json_values(raw)
            .expect("json lines should parse")
            .expect("multiple non-empty lines should return values");

        assert_eq!(values.len(), 2);
        assert_eq!(
            values[0].get("id_token").and_then(|value| value.as_str()),
            Some("id-1")
        );
        assert_eq!(
            values[1]
                .get("access_token")
                .and_then(|value| value.as_str()),
            Some("access-2")
        );
    }

    #[test]
    fn compat_parses_portable_codex_token_account() {
        let id_token = make_jwt(serde_json::json!({
            "email": "portable@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-portable",
                "chatgpt_plan_type": "plus",
                "account_id": "acc-portable"
            }
        }));
        let summary = CodexAccountSummary {
            id: "stored-portable".to_string(),
            email: "summary@example.com".to_string(),
            plan_type: None,
            subscription_active_until: None,
            created_at: 100,
            last_used: 200,
        };
        let account = parse_codex_account_compat(
            serde_json::json!({
                "id_token": id_token,
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "last_refresh": 300,
                "type": "codex"
            }),
            "stored-portable",
            Some(&summary),
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.id, "stored-portable");
        assert_eq!(account.email, "portable@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-portable"));
        assert_eq!(account.plan_type.as_deref(), Some("plus"));
        assert_eq!(account.account_id.as_deref(), Some("acc-portable"));
        assert_eq!(account.created_at, 100);
        assert_eq!(account.last_used, 200);
        assert_eq!(account.token_updated_at, Some(300));
    }

    #[test]
    fn compat_parses_portable_codex_api_key_account() {
        let account = parse_codex_account_compat(
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test-portable",
                "api_base_url": "https://example.com/v1",
                "api_provider_id": "custom-openai",
                "api_provider_name": "Custom OpenAI",
                "api_wire_api": "responses",
                "api_supports_websockets": true,
                "email": "api@example.com",
                "created_at": 100,
                "last_used": 200
            }),
            "stored-apikey",
            None,
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.id, "stored-apikey");
        assert!(account.is_api_key_auth());
        assert_eq!(account.email, "api@example.com");
        assert_eq!(account.openai_api_key.as_deref(), Some("sk-test-portable"));
        assert_eq!(
            account.api_base_url.as_deref(),
            Some("https://example.com/v1")
        );
        assert_eq!(account.api_provider_id.as_deref(), Some("custom-openai"));
        assert_eq!(account.api_provider_name.as_deref(), Some("Custom OpenAI"));
        assert_eq!(account.api_wire_api.as_deref(), Some("responses"));
        assert!(account.api_supports_websockets);
        assert_eq!(account.created_at, 100);
        assert_eq!(account.last_used, 200);
    }

    #[test]
    fn compat_disables_websockets_for_chat_completions_account() {
        let account = parse_codex_account_compat(
            serde_json::json!({
                "auth_mode": "apikey",
                "OPENAI_API_KEY": "sk-test-chat",
                "api_base_url": "https://example.com/v1",
                "api_wire_api": "chat_completions",
                "api_supports_websockets": true,
                "created_at": 100,
                "last_used": 200
            }),
            "stored-chat-apikey",
            None,
        )
        .expect("compat parse")
        .expect("account");

        assert_eq!(account.api_wire_api.as_deref(), Some("chat_completions"));
        assert!(!account.api_supports_websockets);
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let base_dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if base_dir.exists() {
            fs::remove_dir_all(&base_dir).expect("cleanup old temp dir");
        }
        fs::create_dir_all(&base_dir).expect("create temp dir");
        base_dir
    }

    struct TestEnvGuard {
        home_dir: std::path::PathBuf,
        previous_home: Option<String>,
        previous_codex_home: Option<String>,
        previous_data_dir: Option<String>,
    }

    impl TestEnvGuard {
        fn new(prefix: &str) -> Self {
            let home_dir = make_temp_dir(prefix);
            let codex_home = home_dir.join(".codex");
            let test_data_dir = home_dir.join(".antigravity_cockpit");
            fs::create_dir_all(&codex_home).expect("create codex home");
            fs::create_dir_all(&test_data_dir).expect("create test data dir");

            let previous_home = std::env::var("HOME").ok();
            let previous_codex_home = std::env::var("CODEX_HOME").ok();
            let previous_data_dir = std::env::var("COCKPIT_TOOLS_TEST_DATA_DIR")
                .ok()
                .or_else(|| std::env::var("COCKPIT_TOOLS_DATA_DIR").ok());
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("CODEX_HOME", &codex_home);
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &test_data_dir);
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &test_data_dir);

            Self {
                home_dir,
                previous_home,
                previous_codex_home,
                previous_data_dir,
            }
        }

        fn codex_home(&self) -> std::path::PathBuf {
            self.home_dir.join(".codex")
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match self.previous_home.as_ref() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match self.previous_codex_home.as_ref() {
                Some(value) => std::env::set_var("CODEX_HOME", value),
                None => std::env::remove_var("CODEX_HOME"),
            }
            match self.previous_data_dir.as_ref() {
                Some(value) => {
                    std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value);
                    std::env::set_var("COCKPIT_TOOLS_DATA_DIR", value);
                }
                None => {
                    std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
                    std::env::remove_var("COCKPIT_TOOLS_DATA_DIR");
                }
            }
            let _ = fs::remove_dir_all(&self.home_dir);
        }
    }

    #[test]
    fn test_env_guard_redirects_codex_account_storage() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-account-storage-isolation-test");

        let storage_path = get_accounts_storage_path();

        assert!(
            storage_path.starts_with(&env.home_dir),
            "Codex account storage should stay inside the test home, got {} for test home {}",
            storage_path.display(),
            env.home_dir.display()
        );
    }

    fn make_jwt(payload: serde_json::Value) -> String {
        let header = serde_json::json!({ "alg": "none", "typ": "JWT" });
        format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).expect("serialize header")),
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).expect("serialize payload"))
        )
    }

    fn make_codex_tokens(
        email: &str,
        account_id: &str,
        organization_id: &str,
        suffix: &str,
        refresh_token: &str,
    ) -> CodexTokens {
        let id_token = make_jwt(serde_json::json!({
            "aud": ["codex-cli"],
            "iss": "https://auth.openai.com",
            "email": email,
            "sub": format!("user-{}", suffix),
            "https://api.openai.com/auth": {
                "chatgpt_user_id": format!("user-{}", suffix),
                "chatgpt_plan_type": "pro",
                "account_id": account_id,
                "organization_id": organization_id,
            }
        }));
        let access_token = make_jwt(serde_json::json!({
            "sub": format!("access-{}", suffix),
            "exp": 4_102_444_800i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": account_id,
                "organization_id": organization_id,
            }
        }));

        CodexTokens {
            id_token,
            access_token,
            refresh_token: Some(refresh_token.to_string()),
        }
    }

    fn build_test_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let email = "demo@example.com";
        let account_id = "acc-current";
        let organization_id = "org-current";
        let storage_id = build_account_storage_id(email, Some(account_id), Some(organization_id));

        let mut account = CodexAccount::new(storage_id.clone(), email.to_string(), tokens);
        account.user_id = Some("user-current".to_string());
        account.plan_type = Some("pro".to_string());
        account.account_id = Some(account_id.to_string());
        account.organization_id = Some(organization_id.to_string());
        account
    }

    fn seed_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let account = build_test_oauth_account(tokens);
        save_account(&account).expect("save account");

        let index = build_test_account_index(&account);
        save_account_index(&index).expect("save index");

        account
    }

    fn build_test_account_index(account: &CodexAccount) -> CodexAccountIndex {
        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
        index.current_account_id = Some(account.id.clone());
        index
    }

    fn write_test_account(data_dir: &Path, account: &CodexAccount) {
        let accounts_dir = data_dir.join("codex_accounts");
        fs::create_dir_all(&accounts_dir).expect("create test accounts dir");
        fs::write(
            accounts_dir.join(format!("{}.json", account.id)),
            serde_json::to_string_pretty(account).expect("serialize test account"),
        )
        .expect("write test account");
    }

    fn load_test_account(data_dir: &Path, account_id: &str) -> CodexAccount {
        let path = data_dir
            .join("codex_accounts")
            .join(format!("{}.json", account_id));
        let content = fs::read_to_string(&path).expect("read test account");
        serde_json::from_str(&content).expect("parse test account")
    }

    #[test]
    fn load_account_clears_bound_oauth_local_gateway_flag() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let _env = TestEnvGuard::new("codex-bound-oauth-clear-gateway");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-clear-gateway".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = true;
        save_account(&account).expect("save account");

        let loaded = load_account(&account.id).expect("load account");
        assert_eq!(loaded.bound_oauth_account_id.as_deref(), Some("oauth-1"));
        assert!(!loaded.bound_oauth_use_local_gateway);
    }

    #[test]
    fn load_account_keeps_bound_oauth_account_id_when_gateway_false() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let _env = TestEnvGuard::new("codex-bound-oauth-keep-id");
        let mut account = CodexAccount::new_api_key(
            "api-bound-oauth-keep-id".to_string(),
            "api-key@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        account.bound_oauth_account_id = Some("oauth-1".to_string());
        account.bound_oauth_use_local_gateway = false;
        save_account(&account).expect("save account");

        let loaded = load_account(&account.id).expect("load account");
        assert_eq!(loaded.bound_oauth_account_id.as_deref(), Some("oauth-1"));
        assert!(!loaded.bound_oauth_use_local_gateway);
    }

    fn write_oauth_auth_file(base_dir: &std::path::Path, tokens: &CodexTokens, account_id: &str) {
        let auth_file = CodexAuthFile {
            auth_mode: None,
            openai_api_key: Some(serde_json::Value::Null),
            base_url: None,
            tokens: Some(CodexAuthTokens {
                id_token: tokens.id_token.clone(),
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                account_id: Some(account_id.to_string()),
            }),
            personal_access_token: None,
            last_refresh: Some(serde_json::Value::String(
                "2026-04-13T00:00:00.000000Z".to_string(),
            )),
        };

        fs::create_dir_all(base_dir).expect("create auth dir");
        fs::write(
            base_dir.join("auth.json"),
            serde_json::to_string_pretty(&auth_file).expect("serialize auth file"),
        )
        .expect("write auth file");
    }

    #[test]
    fn build_auth_file_value_writes_empty_refresh_token_when_account_has_none() {
        let mut account = CodexAccount::new(
            "codex-cpa-account".to_string(),
            "cpa@example.com".to_string(),
            CodexTokens {
                id_token: "id.jwt.token".to_string(),
                access_token: "access.jwt.token".to_string(),
                refresh_token: None,
            },
        );
        account.account_id = Some("acc-cpa".to_string());

        let auth_file = build_auth_file_value(&account).expect("build auth file");
        let tokens = auth_file
            .get("tokens")
            .and_then(|value| value.as_object())
            .expect("tokens object");

        assert_eq!(
            tokens.get("refresh_token").and_then(|value| value.as_str()),
            Some("")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn prepared_oauth_bundle_returns_error_when_keychain_write_fails() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-keychain-write-failure-test");
        let base_dir = make_temp_dir("codex-keychain-write-failure-target");
        let account = build_test_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-keychain-failure",
            "org-keychain-failure",
            "keychain-failure",
            "rt-keychain-failure",
        ));

        std::env::set_var("COCKPIT_TEST_FAIL_CODEX_KEYCHAIN", "1");
        fs::write(base_dir.join("auth.json"), b"{\"previous\":true}").expect("seed previous auth");
        fs::write(base_dir.join("config.toml"), b"model = \"gpt-5.5\"\n")
            .expect("seed previous config");
        let previous_auth = fs::read(base_dir.join("auth.json")).expect("read previous auth");
        let previous_config = fs::read(base_dir.join("config.toml")).expect("read previous config");
        let result = write_prepared_account_bundle_to_dir(&base_dir, &account);
        std::env::remove_var("COCKPIT_TEST_FAIL_CODEX_KEYCHAIN");

        let error = result.expect_err("keychain failure must not report success");
        assert!(error.contains("keychain"));
        assert_eq!(
            fs::read(base_dir.join("auth.json")).expect("read restored auth"),
            previous_auth
        );
        assert_eq!(
            fs::read(base_dir.join("config.toml")).expect("read restored config"),
            previous_config
        );
        assert!(!base_dir
            .join(super::CODEX_AUTH_PROJECTION_FILE_NAME)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup keychain failure target");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restores_previous_keychain_after_later_persistence_failure() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-keychain-later-persistence-failure-test");
        let base_dir = make_temp_dir("codex-keychain-later-persistence-failure-target");
        let previous_account = build_test_oauth_account(make_codex_tokens(
            "previous@example.com",
            "acc-previous-keychain",
            "org-previous-keychain",
            "previous-keychain",
            "rt-previous-keychain",
        ));
        let next_account = build_test_oauth_account(make_codex_tokens(
            "next@example.com",
            "acc-next-keychain",
            "org-next-keychain",
            "next-keychain",
            "rt-next-keychain",
        ));

        super::write_codex_keychain_to_dir(&base_dir, &previous_account)
            .expect("seed test keychain");
        let snapshot = super::snapshot_codex_bundle(&base_dir, &next_account)
            .expect("snapshot should read test keychain");
        let previous_keychain = super::read_codex_keychain_secret_from_dir(&base_dir)
            .expect("read test keychain")
            .expect("previous keychain should exist");

        super::write_prepared_account_bundle_to_dir(&base_dir, &next_account)
            .expect("new account bundle should write");
        let next_keychain = super::read_codex_keychain_secret_from_dir(&base_dir)
            .expect("read updated test keychain")
            .expect("new keychain should exist");
        assert_ne!(next_keychain, previous_keychain);

        let error = super::codex_bundle_error_with_rollback(
            "模拟后续保存账号索引失败".to_string(),
            &snapshot,
        );
        assert!(error.contains("模拟后续保存账号索引失败"));
        assert_eq!(
            super::read_codex_keychain_secret_from_dir(&base_dir)
                .expect("read restored test keychain")
                .expect("restored keychain should exist"),
            previous_keychain
        );

        fs::remove_dir_all(&base_dir).expect("cleanup later persistence failure target");
    }

    #[test]
    fn restores_account_store_index_and_details_after_later_persistence_failure() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-account-store-later-failure-test");
        let current_id = "codex-store-current";
        let target_id = "codex-store-target";
        let accounts_dir = get_accounts_dir();
        fs::create_dir_all(&accounts_dir).expect("create account store");

        let mut previous_index = CodexAccountIndex::new();
        previous_index.current_account_id = Some(current_id.to_string());
        let previous_index_content = serde_json::to_vec_pretty(&previous_index)
            .expect("serialize previous account index");
        let previous_detail_content = b"previous-current-detail".to_vec();
        fs::write(get_accounts_storage_path(), &previous_index_content)
            .expect("write previous account index");
        fs::write(accounts_dir.join(format!("{}.json", current_id)), &previous_detail_content)
            .expect("write previous current detail");

        let snapshot = snapshot_codex_account_store(&previous_index, target_id)
            .expect("snapshot account store");
        fs::write(
            get_accounts_storage_path(),
            b"{\"current_account_id\":\"codex-store-target\"}",
        )
        .expect("write changed account index");
        fs::write(
            accounts_dir.join(format!("{}.json", current_id)),
            b"changed-current-detail",
        )
        .expect("write changed current detail");
        fs::write(
            accounts_dir.join(format!("{}.json", target_id)),
            b"new-target-detail",
        )
        .expect("write changed target detail");

        let error = codex_switch_error_with_rollback(
            "模拟后续保存账号详情失败".to_string(),
            &super::CodexBundleSnapshot {
                files: Vec::new(),
                keychain: None,
            },
            &snapshot,
        );
        assert!(error.contains("模拟后续保存账号详情失败"));
        assert_eq!(
            fs::read(get_accounts_storage_path()).expect("read restored account index"),
            previous_index_content
        );
        assert_eq!(
            fs::read(accounts_dir.join(format!("{}.json", current_id)))
                .expect("read restored current detail"),
            previous_detail_content
        );
        assert!(!accounts_dir.join(format!("{}.json", target_id)).exists());

        fs::remove_dir_all(accounts_dir).expect("cleanup account store failure target");
    }

    #[test]
    fn extract_tokens_from_flat_codex_json() {
        let value = serde_json::json!({
            "id_token": "id.jwt.token",
            "access_token": "access.jwt.token",
            "refresh_token": "rt_123",
            "account_id": "acc_1",
            "type": "codex",
            "email": "demo@example.com"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_123"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_1"));
    }

    #[test]
    fn extract_tokens_from_flat_codex_json_does_not_use_session_token_as_refresh_token() {
        let value = serde_json::json!({
            "id_token": "id.jwt.token",
            "access_token": "access.jwt.token",
            "refresh_token": "",
            "session_token": "encrypted-session-token",
            "account_id": "acc_cpa",
            "type": "codex"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(account_id_hint.as_deref(), Some("acc_cpa"));
    }

    #[test]
    fn extract_tokens_from_nested_tokens_json() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "id.jwt.token",
                "access_token": "access.jwt.token",
                "refresh_token": "rt_456"
            },
            "account_id": "acc_2"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_456"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_2"));
    }

    #[test]
    fn extract_tokens_from_nested_tokens_json_does_not_use_session_token_as_refresh_token() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "id.jwt.token",
                "access_token": "access.jwt.token",
                "refresh_token": ""
            },
            "session_token": "encrypted-session-token",
            "account_id": "acc_nested"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(account_id_hint.as_deref(), Some("acc_nested"));
    }

    #[test]
    fn extract_tokens_from_camel_case_codex_json() {
        let value = serde_json::json!({
            "tokens": {
                "idToken": "id.jwt.token",
                "accessToken": "access.jwt.token",
                "refreshToken": "rt_789"
            },
            "accountId": "acc_3"
        });

        let (tokens, account_id_hint) =
            extract_codex_tokens_from_value(&value).expect("should extract tokens");

        assert_eq!(tokens.id_token, "id.jwt.token");
        assert_eq!(tokens.access_token, "access.jwt.token");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt_789"));
        assert_eq!(account_id_hint.as_deref(), Some("acc_3"));
    }

    #[test]
    fn extract_candidate_preserves_existing_token_priority() {
        let full_value = serde_json::json!({
            "idToken": "id.jwt.token",
            "accessToken": make_jwt(serde_json::json!({ "sub": "access-user" })),
            "refreshToken": "rt_existing"
        });
        let refresh_value = serde_json::json!({
            "refreshToken": "rt_existing",
            "accessToken": make_jwt(serde_json::json!({ "sub": "access-user" }))
        });
        let plain_token_value = serde_json::json!({
            "token": "not-a-jwt-token"
        });
        let opaque_access_token_value = serde_json::json!({
            "token": "at-confirmed-opaque-token",
            "email": "opaque@example.com",
            "account_id": "acc-opaque"
        });

        let full_candidate = extract_codex_import_candidate_from_value(&full_value)
            .expect("full token JSON should still be accepted");
        assert!(matches!(
            full_candidate,
            CodexJsonImportCandidate::FullToken { .. }
        ));

        let refresh_candidate = extract_codex_import_candidate_from_value(&refresh_value)
            .expect("refresh token should keep priority over accessToken-only");
        assert!(matches!(
            refresh_candidate,
            CodexJsonImportCandidate::RefreshToken { .. }
        ));

        assert!(
            extract_codex_import_candidate_from_value(&plain_token_value).is_none(),
            "plain token fields should not be treated as accessToken-only"
        );
        assert!(matches!(
            extract_codex_import_candidate_from_value(&opaque_access_token_value),
            Some(CodexJsonImportCandidate::AccessToken { .. })
        ));
    }

    #[test]
    fn extract_candidate_from_codex_session_json_as_cpa_tokens_without_session_token_refresh() {
        let access_token = make_jwt(serde_json::json!({
            "sub": "auth0|session-user",
            "https://api.openai.com/profile": {
                "email": "session@example.com",
                "email_verified": true
            },
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-session-token",
                "chatgpt_user_id": "user-session",
                "chatgpt_plan_type": "plus"
            }
        }));
        let session = serde_json::json!({
            "user": {
                "id": "user-session",
                "email": "session@example.com"
            },
            "expires": "2026-08-17T02:06:40.890Z",
            "account": {
                "id": "acc-session",
                "planType": "plus"
            },
            "accessToken": access_token,
            "authProvider": "openai",
            "sessionToken": "encrypted-session"
        });

        let candidate = extract_codex_import_candidate_from_value(&session)
            .expect("ChatGPT session JSON should be accepted");

        match candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                note_update,
            } => {
                assert_eq!(tokens.id_token, tokens.access_token);
                assert_eq!(tokens.refresh_token, None);
                assert_eq!(account_id_hint.as_deref(), Some("acc-session"));
                assert!(!super::has_codex_account_note_update(&note_update));
                assert!(decode_jwt_payload_value(&tokens.access_token).is_some());
            }
            _ => panic!("expected session JSON to be normalized to full CPA-style tokens"),
        }
    }

    #[test]
    fn extract_candidate_from_wrapped_codex_session_json_string() {
        let access_token = make_jwt(serde_json::json!({
            "email": "wrapped-session@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-wrapped-session"
            }
        }));
        let session = serde_json::json!({
            "user": {
                "email": "wrapped-session@example.com"
            },
            "account": {
                "id": "acc-wrapped-session"
            },
            "accessToken": access_token,
            "refreshToken": "rt_wrapped",
            "authProvider": "openai"
        });
        let wrapper = serde_json::json!({
            "session_json": serde_json::to_string(&session).expect("serialize session")
        });

        let candidate = extract_codex_import_candidate_from_value(&wrapper)
            .expect("wrapped session JSON string should be accepted");

        match candidate {
            CodexJsonImportCandidate::FullToken {
                tokens,
                account_id_hint,
                ..
            } => {
                assert_eq!(tokens.id_token, tokens.access_token);
                assert_eq!(tokens.refresh_token.as_deref(), Some("rt_wrapped"));
                assert_eq!(account_id_hint.as_deref(), Some("acc-wrapped-session"));
            }
            _ => panic!("expected wrapped session JSON to become full CPA-style tokens"),
        }
    }

    #[test]
    fn extract_candidate_from_sub2api_account_credentials() {
        let value = serde_json::json!({
            "name": "Sub2API account",
            "notes": "imported from sub2api",
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "sub2api@example.com",
                "access_token": "at-sub2api-team-token",
                "token_type": "Bearer",
                "auth_mode": "personal_access_token",
                "openai_auth_mode": "personal_access_token",
                "plan_type": "team",
                "chatgpt_account_id": "acc-sub2api"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("Sub2API account should expose access_token");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-sub2api-team-token");
                assert_eq!(hints.email.as_deref(), Some("sub2api@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-sub2api"));
                assert_eq!(hints.account_note.as_deref(), Some("imported from sub2api"));
            }
            _ => panic!("expected accessToken-only candidate"),
        }
    }

    #[test]
    fn extract_candidate_prefers_cpa_personal_access_token_over_session_token() {
        let session_access_token = make_jwt(serde_json::json!({
            "email": "cpa@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-cpa-session"
            }
        }));
        let value = serde_json::json!({
            "type": "codex",
            "provider": "openai",
            "id_token": "",
            "access_token": session_access_token,
            "refresh_token": "",
            "email": "cpa@example.com",
            "plan_type": "team",
            "account_id": "acc-cpa",
            "chatgpt_account_id": "acc-cpa-chatgpt",
            "at_token": "at-cpa-team-token",
            "personal_access_token": "at-cpa-personal-token",
            "token_type": "Bearer",
            "auth_mode": "personal_access_token",
            "openai_auth_mode": "personal_access_token",
            "headers": {
                "authorization": "Bearer at-cpa-header-token"
            }
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("CPA personal access token object should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-cpa-personal-token");
                assert_eq!(hints.email.as_deref(), Some("cpa@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-cpa"));
            }
            _ => panic!("expected CPA personal access token candidate"),
        }
    }

    #[test]
    fn extract_candidate_accepts_team_access_token_list_line() {
        let value = serde_json::Value::String(
            "team@example.comat-team-list-token.eyJhbGciOiJub25lIn0.payload".to_string(),
        );

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("team AT list line should expose the at-* token");

        match candidate {
            CodexJsonImportCandidate::AccessToken { access_token, .. } => {
                assert_eq!(access_token, "at-team-list-token");
            }
            _ => panic!("expected access-token-only candidate"),
        }
    }

    #[test]
    fn detects_sub2api_export_wrapper() {
        let value = serde_json::json!({
            "exported_at": "2026-05-18T09:40:35Z",
            "proxies": [],
            "accounts": [{
                "platform": "openai",
                "type": "oauth",
                "credentials": {
                    "access_token": make_jwt(serde_json::json!({ "sub": "sub2api-user" }))
                }
            }]
        });

        assert!(looks_like_sub2api_export(&value));
    }

    #[test]
    fn extract_candidate_accepts_opaque_access_token_with_hints() {
        let value = serde_json::json!({
            "tokens": {
                "id_token": "",
                "access_token": "at-confirmed-team-token",
                "refresh_token": ""
            },
            "email": "team@example.com",
            "plan_type": "team",
            "account_id": "acc-team",
            "organization_id": "org-team",
            "account_name": "Team Workspace",
            "account_structure": "team",
            "account_note": "confirmed import"
        });

        let candidate = extract_codex_import_candidate_from_value(&value)
            .expect("opaque at-* access token should be accepted");

        match candidate {
            CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            } => {
                assert_eq!(access_token, "at-confirmed-team-token");
                assert_eq!(hints.email.as_deref(), Some("team@example.com"));
                assert_eq!(hints.plan_type.as_deref(), Some("team"));
                assert_eq!(hints.account_id.as_deref(), Some("acc-team"));
                assert_eq!(hints.organization_id.as_deref(), Some("org-team"));
                assert_eq!(hints.account_name.as_deref(), Some("Team Workspace"));
                assert_eq!(hints.account_structure.as_deref(), Some("team"));
                assert_eq!(hints.account_note.as_deref(), Some("confirmed import"));
            }
            _ => panic!("expected opaque access-token-only candidate"),
        }
    }

    #[test]
    fn upsert_opaque_access_token_only_account_uses_import_hints() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-opaque-access-token-import-test");

        let account = upsert_account_from_access_token_with_hints(
            "at-confirmed-team-token".to_string(),
            CodexAccessTokenImportHints {
                email: Some("team@example.com".to_string()),
                user_id: Some("user-team".to_string()),
                plan_type: Some("team".to_string()),
                subscription_active_until: None,
                account_id: Some("acc-team".to_string()),
                organization_id: Some("org-team".to_string()),
                account_name: Some("Team Workspace".to_string()),
                account_structure: Some("team".to_string()),
                account_note: Some("confirmed import".to_string()),
                ..Default::default()
            },
        )
        .expect("upsert opaque access token account");

        assert_eq!(account.email, "team@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-team"));
        assert_eq!(account.plan_type.as_deref(), Some("team"));
        assert_eq!(account.account_id.as_deref(), Some("acc-team"));
        assert_eq!(account.organization_id.as_deref(), Some("org-team"));
        assert_eq!(account.account_name.as_deref(), Some("Team Workspace"));
        assert_eq!(account.account_structure.as_deref(), Some("team"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, "at-confirmed-team-token");
        assert_eq!(account.tokens.refresh_token, None);
        assert!(!account.requires_reauth);
        assert_eq!(account.reauth_reason, None);

        let persisted = load_account(&account.id).expect("persisted opaque account");
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
        assert_eq!(persisted.account_id.as_deref(), Some("acc-team"));
    }

    #[test]
    fn upsert_access_token_only_account_uses_access_claims() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-import-test");
        let access_token = make_jwt(serde_json::json!({
            "email": "access@example.com",
            "sub": "user-access",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access",
                "chatgpt_user_id": "user-access",
                "chatgpt_plan_type": "team",
                "chatgpt_subscription_active_until": 1767225600,
                "poid": "org-access"
            }
        }));

        let candidate = extract_codex_import_candidate_from_value(&serde_json::Value::String(
            access_token.clone(),
        ))
        .expect("raw JWT should be accepted as accessToken");
        assert!(matches!(
            candidate,
            CodexJsonImportCandidate::AccessToken { .. }
        ));

        let account = upsert_account_from_access_token(
            access_token.clone(),
            Some("imported from accessToken".to_string()),
        )
        .expect("upsert access token account");

        assert_eq!(account.email, "access@example.com");
        assert_eq!(account.user_id.as_deref(), Some("user-access"));
        assert_eq!(account.plan_type.as_deref(), Some("team"));
        assert_eq!(
            account.subscription_active_until.as_deref(),
            Some("1767225600")
        );
        assert_eq!(account.account_id.as_deref(), Some("acc-access"));
        assert_eq!(account.organization_id.as_deref(), Some("org-access"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token, None);
        assert_eq!(
            account.account_note.as_deref(),
            Some("imported from accessToken")
        );

        let persisted = load_account(&account.id).expect("persisted access token account");
        assert_eq!(persisted.tokens.access_token, account.tokens.access_token);
    }

    #[test]
    fn upsert_auth_tokens_with_empty_id_token_uses_access_token() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-auth-file-access-token-import-test");
        let access_token = make_jwt(serde_json::json!({
            "email": "auth-access@example.com",
            "sub": "auth-access-user",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-auth-access",
                "chatgpt_user_id": "auth-access-user",
                "chatgpt_plan_type": "pro",
                "poid": "org-auth-access"
            }
        }));

        let account = upsert_account_from_auth_tokens(CodexAuthTokens {
            id_token: String::new(),
            access_token: access_token.clone(),
            refresh_token: None,
            account_id: None,
        })
        .expect("empty id_token auth tokens should import from accessToken");

        assert_eq!(account.email, "auth-access@example.com");
        assert_eq!(account.user_id.as_deref(), Some("auth-access-user"));
        assert_eq!(account.account_id.as_deref(), Some("acc-auth-access"));
        assert_eq!(account.organization_id.as_deref(), Some("org-auth-access"));
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token, None);
    }

    #[test]
    fn import_multiline_pending_oauth_array_creates_pending_account() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-pending-oauth-import-test");
        let content = r#"[
  {
    "id_token": "",
    "access_token": "",
    "refresh_token": "",
    "account_id": "",
    "last_refresh": "2026-07-04T02:25:18.829Z",
    "email": "dddd",
    "type": "codex",
    "expired": "",
    "account_note": "2131",
    "two_factor_secret": "Ddddd",
    "account_password": "213123",
    "phone_number": "2312",
    "mail_url": "https://mail.example.test/inbox?mail=dddd"
  }
]"#;
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(content))
            .expect("pending OAuth JSON array should import");

        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert_eq!(account.email, "dddd");
        assert!(is_pending_oauth_account(account));
        assert_eq!(
            account.authorization_status.as_deref(),
            Some(CODEX_AUTHORIZATION_STATUS_PENDING)
        );
        assert_eq!(account.tokens.id_token, "");
        assert_eq!(account.tokens.access_token, "");
        assert_eq!(account.tokens.refresh_token, None);
        assert_eq!(account.account_note.as_deref(), Some("2131"));
        assert_eq!(account.two_factor_secret.as_deref(), Some("Ddddd"));
        assert_eq!(account.account_password.as_deref(), Some("213123"));
        assert_eq!(account.phone_number.as_deref(), Some("2312"));
        assert_eq!(
            account.mail_url.as_deref(),
            Some("https://mail.example.test/inbox?mail=dddd")
        );

        let persisted = load_account(&account.id).expect("pending account persisted");
        assert!(is_pending_oauth_account(&persisted));
        assert_eq!(persisted.account_note.as_deref(), Some("2131"));
        assert_eq!(
            persisted.mail_url.as_deref(),
            Some("https://mail.example.test/inbox?mail=dddd")
        );
    }

    #[test]
    fn import_pending_oauth_delimited_line_creates_pending_account() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-pending-oauth-delimited-import-test");
        let content = "user+tag@example.com----Pass@word123----BXU33BDMEBDIOAA2AOCFL4NBKVQAQWFY----https://mail.example.test/open.php?mail=user%2Btag%40example.com&pwd=secret&limit=5\nuser2@example.com----pwd2----ABCDEFGHIJKLMNOP";
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(content))
            .expect("delimited pending OAuth lines should import");

        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().all(is_pending_oauth_account));

        let first = accounts
            .iter()
            .find(|item| item.email == "user+tag@example.com")
            .expect("first account");
        assert_eq!(first.account_password.as_deref(), Some("Pass@word123"));
        assert_eq!(
            first.two_factor_secret.as_deref(),
            Some("BXU33BDMEBDIOAA2AOCFL4NBKVQAQWFY")
        );
        assert_eq!(
            first.mail_url.as_deref(),
            Some(
                "https://mail.example.test/open.php?mail=user%2Btag%40example.com&pwd=secret&limit=5"
            )
        );
        assert!(first.tokens.access_token.is_empty());

        let second = accounts
            .iter()
            .find(|item| item.email == "user2@example.com")
            .expect("second account");
        assert_eq!(second.account_password.as_deref(), Some("pwd2"));
        assert_eq!(
            second.two_factor_secret.as_deref(),
            Some("ABCDEFGHIJKLMNOP")
        );
        assert!(second.mail_url.is_none());
    }

    #[test]
    fn try_parse_pending_oauth_delimited_line_rejects_non_email() {
        assert!(try_parse_pending_oauth_delimited_line(
            "not-an-email----pwd----SECRET----https://example.com"
        )
        .is_none());
        assert!(try_parse_pending_oauth_delimited_line("rt_only_token").is_none());
        assert!(try_parse_pending_oauth_delimited_line(
            r#"{"email":"a@b.com","account_password":"x"}"#
        )
        .is_none());
    }

    #[test]
    fn import_auth_file_tokens_preserves_sensitive_note_metadata() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-auth-file-sensitive-note-import-test");
        let tokens = make_codex_tokens(
            "sensitive@example.com",
            "acc-sensitive",
            "org-sensitive",
            "sensitive",
            "rt-sensitive",
        );
        let content = serde_json::json!({
            "tokens": {
                "id_token": tokens.id_token,
                "access_token": tokens.access_token,
                "refresh_token": tokens.refresh_token,
                "account_id": "acc-sensitive"
            },
            "email": "sensitive@example.com",
            "type": "codex",
            "account_note": "note-1",
            "two_factor_secret": "SECRET-2FA",
            "account_password": "password-1",
            "phone_number": "15500000000",
            "mail_url": "https://mail.example.test/inbox"
        });
        let runtime = tokio::runtime::Runtime::new().expect("create runtime");

        let accounts = runtime
            .block_on(import_from_json(
                &serde_json::to_string(&content).expect("serialize import JSON"),
            ))
            .expect("auth file JSON should import");

        assert_eq!(accounts.len(), 1);
        let account = &accounts[0];
        assert_eq!(account.email, "sensitive@example.com");
        assert_eq!(account.account_note.as_deref(), Some("note-1"));
        assert_eq!(account.two_factor_secret.as_deref(), Some("SECRET-2FA"));
        assert_eq!(account.account_password.as_deref(), Some("password-1"));
        assert_eq!(account.phone_number.as_deref(), Some("15500000000"));
        assert_eq!(
            account.mail_url.as_deref(),
            Some("https://mail.example.test/inbox")
        );

        let persisted = load_account(&account.id).expect("sensitive account persisted");
        assert_eq!(persisted.account_note.as_deref(), Some("note-1"));
        assert_eq!(persisted.two_factor_secret.as_deref(), Some("SECRET-2FA"));
        assert_eq!(persisted.account_password.as_deref(), Some("password-1"));
        assert_eq!(persisted.phone_number.as_deref(), Some("15500000000"));
        assert_eq!(
            persisted.mail_url.as_deref(),
            Some("https://mail.example.test/inbox")
        );
    }

    #[test]
    fn upsert_existing_account_keeps_own_refresh_token_when_import_has_none() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-preserve-refresh-token-test");
        let existing = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-existing",
        ));
        let mut imported_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "new",
            "rt-unused",
        );
        let imported_access_token = imported_tokens.access_token.clone();
        imported_tokens.refresh_token = None;

        let account = upsert_account(imported_tokens).expect("upsert existing account");

        assert_eq!(account.id, existing.id);
        assert_eq!(account.tokens.access_token, imported_access_token);
        assert_eq!(account.tokens.refresh_token.as_deref(), Some("rt-existing"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            Some("rt-existing")
        );
    }

    #[test]
    fn upsert_reuses_legacy_email_only_account_when_identity_appears() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-legacy-email-only-dedupe-test");
        let email = "legacy@example.com";
        let account_id = "acc-legacy";
        let organization_id = "org-legacy";
        let legacy_id = build_account_storage_id(email, None, None);
        let generated_identity_id =
            build_account_storage_id(email, Some(account_id), Some(organization_id));
        assert_ne!(legacy_id, generated_identity_id);

        let mut legacy = CodexAccount::new(
            legacy_id.clone(),
            email.to_string(),
            make_codex_tokens(email, account_id, organization_id, "old", "rt-existing"),
        );
        legacy.account_id = None;
        legacy.organization_id = None;
        save_account(&legacy).expect("save legacy account");

        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: legacy.id.clone(),
            email: legacy.email.clone(),
            plan_type: legacy.plan_type.clone(),
            subscription_active_until: legacy.subscription_active_until.clone(),
            created_at: legacy.created_at,
            last_used: legacy.last_used,
        });
        save_account_index(&index).expect("save legacy index");

        let imported = upsert_account(make_codex_tokens(
            email,
            account_id,
            organization_id,
            "new",
            "rt-new",
        ))
        .expect("upsert should reuse legacy account");

        assert_eq!(imported.id, legacy_id);
        assert_eq!(imported.account_id.as_deref(), Some(account_id));
        assert_eq!(imported.organization_id.as_deref(), Some(organization_id));
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, legacy_id);
        let index = load_account_index();
        assert_eq!(index.accounts.len(), 1);
        assert_eq!(index.accounts[0].id, legacy_id);
    }

    #[test]
    fn remove_accounts_prunes_missing_detail_index_entries() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-remove-prunes-missing-details-test");
        let account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-existing",
        ));
        let missing_id = "api-legacy-bound-oauth".to_string();
        let mut index = load_account_index();
        index.accounts.push(CodexAccountSummary {
            id: missing_id.clone(),
            email: "missing@example.com".to_string(),
            plan_type: Some("API_KEY".to_string()),
            subscription_active_until: None,
            created_at: 1,
            last_used: 1,
        });
        index.current_account_id = Some(missing_id.clone());
        save_account_index(&index).expect("save index with missing detail entry");

        let accounts = list_accounts_checked().expect("list should keep readable accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, account.id);

        remove_accounts(&[account.id.clone()]).expect("remove account");

        assert!(load_account(&account.id).is_none());
        let index = load_account_index();
        assert!(index.accounts.is_empty());
        assert!(index.current_account_id.is_none());
        let accounts = list_accounts_checked().expect("empty index should be valid");
        assert!(accounts.is_empty());
    }

    #[test]
    fn list_accounts_prunes_orphan_index_when_all_details_are_missing() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-list-prunes-orphan-index-test");
        let missing_id = "api-legacy-bound-oauth".to_string();
        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: missing_id.clone(),
            email: "missing@example.com".to_string(),
            plan_type: Some("API_KEY".to_string()),
            subscription_active_until: None,
            created_at: 1,
            last_used: 1,
        });
        index.current_account_id = Some(missing_id);
        save_account_index(&index).expect("save orphan index");

        let accounts = list_accounts_checked().expect("orphan index should be pruned");
        assert!(accounts.is_empty());

        let index = load_account_index();
        assert!(index.accounts.is_empty());
        assert!(index.current_account_id.is_none());
    }

    #[test]
    fn list_accounts_recovers_details_missing_from_index_and_merges_summary_fields() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-list-recovers-missing-index-details-test");
        let mut indexed = build_test_oauth_account(make_codex_tokens(
            "indexed@example.com",
            "acc-indexed",
            "org-indexed",
            "indexed",
            "rt-indexed",
        ));
        indexed.id = build_account_storage_id(
            "indexed@example.com",
            Some("acc-indexed"),
            Some("org-indexed"),
        );
        indexed.email = "indexed@example.com".to_string();
        indexed.plan_type = None;
        indexed.subscription_active_until = None;
        indexed.created_at = 10;
        indexed.last_used = 10;
        save_account(&indexed).expect("save indexed detail");

        let mut hidden = build_test_oauth_account(make_codex_tokens(
            "hidden@example.com",
            "acc-hidden",
            "org-hidden",
            "hidden",
            "rt-hidden",
        ));
        hidden.id =
            build_account_storage_id("hidden@example.com", Some("acc-hidden"), Some("org-hidden"));
        hidden.email = "hidden@example.com".to_string();
        hidden.created_at = 20;
        hidden.last_used = 20;
        save_account(&hidden).expect("save hidden detail");

        let old_index = serde_json::json!({
            "version": "1.0",
            "accounts": [{
                "id": indexed.id,
                "email": indexed.email,
                "plan_type": "team",
                "subscription_active_until": "2026-08-01T00:00:00Z",
                "created_at": 5,
                "last_used": 30
            }],
            "current_account_id": indexed.id
        });
        fs::write(
            get_accounts_storage_path(),
            serde_json::to_string_pretty(&old_index).expect("serialize old index"),
        )
        .expect("write old index");

        let accounts = list_accounts_checked().expect("list should repair from details");
        assert_eq!(accounts.len(), 2);
        let listed_indexed = accounts
            .iter()
            .find(|account| account.id == indexed.id)
            .expect("indexed account should remain visible");
        assert_eq!(listed_indexed.plan_type.as_deref(), Some("team"));
        assert_eq!(
            listed_indexed.subscription_active_until.as_deref(),
            Some("2026-08-01T00:00:00Z")
        );
        assert!(accounts.iter().any(|account| account.id == hidden.id));

        let repaired_index = load_account_index();
        assert_eq!(
            repaired_index.detail_schema_version,
            CODEX_ACCOUNT_DETAIL_SCHEMA_VERSION
        );
        assert_eq!(repaired_index.accounts.len(), 2);
        assert_eq!(
            repaired_index.current_account_id.as_deref(),
            Some(indexed.id.as_str())
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let repaired_detail = loop {
            let account = load_account(&indexed.id).expect("indexed detail should remain");
            if account.plan_type.as_deref() == Some("team") {
                break account;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background summary migration should persist"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(repaired_detail.plan_type.as_deref(), Some("team"));
        assert_eq!(
            repaired_detail.subscription_active_until.as_deref(),
            Some("2026-08-01T00:00:00Z")
        );
        assert_eq!(repaired_detail.created_at, 10);
        assert_eq!(repaired_detail.last_used, 30);
    }

    #[test]
    fn reauth_updates_explicit_target_account_even_when_identity_changes() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-explicit-reauth-target-test");
        let email = "reauth@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let generated_new_id = build_account_storage_id(email, Some("acc-new"), Some("org-new"));
        assert_ne!(existing.id, generated_new_id);

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update target account");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.account_id.as_deref(), Some("acc-new"));
        assert_eq!(reauthed.organization_id.as_deref(), Some("org-new"));
        assert_eq!(reauthed.tokens.refresh_token.as_deref(), Some("rt-new"));
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn reauth_preserves_note_details_when_target_is_missing_from_index() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-reauth-preserve-notes-missing-index-test");
        let email = "reauth-notes@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let mut detail = load_account(&existing.id).expect("load existing account");
        detail.account_name = Some("备注账号".to_string());
        detail.account_structure = Some("个人".to_string());
        detail.account_note = Some("其他备注".to_string());
        detail.two_factor_secret = Some("JBSWY3DPEHPK3PXP".to_string());
        detail.account_password = Some("password-1".to_string());
        detail.phone_number = Some("13800000000".to_string());
        save_account(&detail).expect("save noted account");

        let mut broken_index = CodexAccountIndex::new();
        broken_index.accounts.clear();
        broken_index.current_account_id = None;
        save_account_index(&broken_index).expect("save broken index");

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update detail-backed target");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.account_id.as_deref(), Some("acc-new"));
        assert_eq!(reauthed.organization_id.as_deref(), Some("org-new"));
        assert_eq!(reauthed.account_name.as_deref(), Some("备注账号"));
        assert_eq!(reauthed.account_structure.as_deref(), Some("个人"));
        assert_eq!(reauthed.account_note.as_deref(), Some("其他备注"));
        assert_eq!(
            reauthed.two_factor_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );
        assert_eq!(reauthed.account_password.as_deref(), Some("password-1"));
        assert_eq!(reauthed.phone_number.as_deref(), Some("13800000000"));

        let persisted = load_account(&existing.id).expect("load persisted account");
        assert_eq!(persisted.account_note.as_deref(), Some("其他备注"));
        assert_eq!(
            persisted.two_factor_secret.as_deref(),
            Some("JBSWY3DPEHPK3PXP")
        );

        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn reauth_removes_generated_duplicate_for_target_identity() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-explicit-reauth-dedupe-test");
        let email = "reauth-duplicate@example.com";
        let existing = upsert_account(make_codex_tokens(
            email, "acc-old", "org-old", "old", "rt-old",
        ))
        .expect("seed existing account");
        let duplicate = upsert_account(make_codex_tokens(
            email, "acc-new", "org-new", "dup", "rt-dup",
        ))
        .expect("seed duplicate account");
        assert_ne!(existing.id, duplicate.id);
        assert_eq!(list_accounts_checked().expect("list accounts").len(), 2);

        let reauthed = upsert_account_for_reauth(
            make_codex_tokens(email, "acc-new", "org-new", "new", "rt-new"),
            &existing.id,
        )
        .expect("reauth should update target and remove duplicate");

        assert_eq!(reauthed.id, existing.id);
        assert_eq!(reauthed.tokens.refresh_token.as_deref(), Some("rt-new"));
        assert!(load_account(&duplicate.id).is_none());
        let accounts = list_accounts_checked().expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, existing.id);
    }

    #[test]
    fn upsert_access_token_only_existing_account_keeps_own_refresh_token() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-preserve-refresh-test");
        let existing = upsert_account(make_codex_tokens(
            "access@example.com",
            "acc-access",
            "org-access",
            "old",
            "rt-existing",
        ))
        .expect("seed existing account");
        let access_token = make_jwt(serde_json::json!({
            "email": "access@example.com",
            "sub": "user-access-new",
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-access",
                "chatgpt_user_id": "user-access-new",
                "chatgpt_plan_type": "team",
                "poid": "org-access"
            }
        }));

        let account =
            upsert_account_from_access_token(access_token.clone(), None).expect("upsert AT only");

        assert_eq!(account.id, existing.id);
        assert_eq!(account.tokens.access_token, access_token);
        assert_eq!(account.tokens.refresh_token.as_deref(), Some("rt-existing"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            Some("rt-existing")
        );
    }

    #[test]
    fn extracts_email_from_openai_profile_claim() {
        let id_token = make_jwt(serde_json::json!({
            "aud": ["https://api.openai.com/v1"],
            "iss": "https://auth.openai.com",
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-profile",
                "chatgpt_plan_type": "plus",
                "account_id": "acc-profile"
            },
            "https://api.openai.com/profile": {
                "email": "profile@example.com",
                "email_verified": true
            }
        }));

        let (email, user_id, plan_type, _, account_id, _) =
            extract_user_info(&id_token).expect("extract profile email");

        assert_eq!(email, "profile@example.com");
        assert_eq!(user_id.as_deref(), Some("user-profile"));
        assert_eq!(plan_type.as_deref(), Some("plus"));
        assert_eq!(account_id.as_deref(), Some("acc-profile"));
    }

    #[test]
    fn parses_auth_file_last_refresh_variants() {
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!("2026-04-13T00:00:00.000000Z"))),
            Some(1_776_038_400)
        );
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!(1_765_497_600_123i64))),
            Some(1_765_497_600)
        );
        assert_eq!(
            parse_auth_file_last_refresh(Some(&serde_json::json!(1_765_497_600i64))),
            Some(1_765_497_600)
        );
    }

    #[test]
    fn formats_refresh_errors_with_actionable_reason() {
        let reused = format_refresh_error_for_user(
            "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
        );
        assert!(reused.contains("refresh_token 已被其它客户端或实例使用过"));
        assert!(reused.contains("请重新登录"));

        let region = format_refresh_error_for_user(
            "Token 刷新失败: status=403 Forbidden, error_code=unsupported_country_region_territory",
        );
        assert!(region.contains("当前网络地区不支持刷新 Codex 授权"));
        assert!(!region.contains("请重新登录"));
    }

    #[test]
    fn access_token_only_accounts_do_not_require_proactive_refresh() {
        let mut account = CodexAccount::new(
            "codex_access_only".to_string(),
            "access-only@example.com".to_string(),
            make_codex_tokens(
                "access-only@example.com",
                "acc-access-only",
                "org-access-only",
                "access-only",
                "rt-unused",
            ),
        );
        account.tokens.refresh_token = None;
        account.token_updated_at = Some(0);

        assert!(!is_managed_auth_refresh_due(&account));
    }

    #[test]
    fn force_refresh_reuses_newer_generation_without_network_refresh() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-force-refresh-generation-test");
        let mut account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "newer-generation",
            "rt-newer-generation",
        ));
        account.token_generation = 2;
        account.token_updated_at = Some(now_timestamp());
        save_account(&account).expect("save newer generation account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let refreshed = runtime
            .block_on(force_refresh_managed_account_after_observed(
                &account.id,
                1,
                "test observed generation",
            ))
            .expect("newer generation should be reused");

        assert_eq!(refreshed.token_generation, 2);
        assert_eq!(refreshed.tokens.access_token, account.tokens.access_token);
        assert_eq!(
            refreshed.tokens.refresh_token.as_deref(),
            account.tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn missing_refresh_token_reauth_is_cleared_for_access_token_only_accounts() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-only-reauth-clear-test");
        let mut tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "access-only",
            "rt-unused",
        );
        tokens.refresh_token = None;
        let mut account = seed_oauth_account(tokens);
        account.requires_reauth = true;
        account.reauth_reason = Some(
            "Codex 登录授权缺少 refresh_token，无法自动续期；当前 access_token 已不可用。"
                .to_string(),
        );
        save_account(&account).expect("save access-token-only reauth account");

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let prepared = runtime
            .block_on(ensure_managed_account_fresh(&account.id))
            .expect("access-token-only account should remain usable");

        assert!(!prepared.requires_reauth);
        assert_eq!(prepared.tokens.refresh_token, None);
        let persisted = load_account(&account.id).expect("persisted account");
        assert!(!persisted.requires_reauth);
        assert_eq!(persisted.reauth_reason, None);
    }

    #[test]
    fn expired_access_token_only_account_requires_reauth_on_prepare() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-access-token-only-expired-test");
        let mut tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "access-only-expired",
            "rt-unused",
        );
        tokens.access_token = make_jwt(serde_json::json!({
            "sub": "access-only-expired",
            "exp": 1i64,
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acc-current",
                "organization_id": "org-current",
            }
        }));
        tokens.refresh_token = None;
        let account = seed_oauth_account(tokens);

        let runtime = tokio::runtime::Runtime::new().expect("create runtime");
        let error = runtime
            .block_on(ensure_managed_account_fresh(&account.id))
            .expect_err("expired access-token-only account should require reauth");

        assert!(error.contains("缺少 refresh_token"));
        let persisted = load_account(&account.id).expect("persisted account");
        assert!(persisted.requires_reauth);
        assert!(persisted
            .reauth_reason
            .as_deref()
            .unwrap_or_default()
            .contains("缺少 refresh_token"));
    }

    #[test]
    fn authority_snapshot_requires_newer_refresh_marker() {
        let mut account = CodexAccount::new(
            "codex_test".to_string(),
            "demo@example.com".to_string(),
            make_codex_tokens(
                "demo@example.com",
                "acc-current",
                "org-current",
                "old",
                "rt-old",
            ),
        );
        account.account_id = Some("acc-current".to_string());
        account.organization_id = Some("org-current".to_string());
        account.token_updated_at = Some(2000);

        let snapshot = LocalCodexOAuthSnapshot {
            tokens: make_codex_tokens(
                "demo@example.com",
                "acc-current",
                "org-current",
                "new",
                "rt-new",
            ),
            email: "demo@example.com".to_string(),
            subscription_active_until: None,
            account_id: Some("acc-current".to_string()),
            organization_id: Some("org-current".to_string()),
            last_refresh_at: Some(1000),
        };
        assert!(!should_accept_authority_snapshot(&account, &snapshot));

        let newer_snapshot = LocalCodexOAuthSnapshot {
            last_refresh_at: Some(3000),
            ..snapshot
        };
        assert!(should_accept_authority_snapshot(&account, &newer_snapshot));
    }

    #[test]
    fn detect_auth_file_plan_type_from_filename() {
        let prolite = detect_auth_file_plan_type_from_path(std::path::Path::new(
            "/tmp/codex-demo@example.com-prolite.json",
        ));
        let promax = detect_auth_file_plan_type_from_path(std::path::Path::new(
            "/tmp/codex-demo@example.com-pro-max.json",
        ));
        let team =
            detect_auth_file_plan_type_from_path(std::path::Path::new("/tmp/codex-demo-team.json"));

        assert_eq!(prolite.as_deref(), Some("prolite"));
        assert_eq!(promax.as_deref(), Some("promax"));
        assert_eq!(team, None);
    }

    #[test]
    fn current_account_does_not_sync_tokens_from_official_store() {
        let data_dir = make_temp_dir("codex-current-account-sync-test");
        let codex_home = data_dir.join(".codex");

        let stored = build_test_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "old",
            "rt-old",
        ));
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "latest",
            "rt-latest",
        );
        write_oauth_auth_file(&codex_home, &latest_tokens, "acc-current");

        let index = build_test_account_index(&stored);
        write_test_account(&data_dir, &stored);
        assert_eq!(
            index.current_account_id.as_deref(),
            Some(stored.id.as_str())
        );

        let current = get_current_account_from_loaded(
            index,
            |account_id| Some(load_test_account(&data_dir, account_id)),
            &codex_home,
        )
        .expect("current account");
        assert_eq!(current.id, stored.id);
        assert_eq!(current.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            current.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );

        let persisted = load_test_account(&data_dir, &stored.id);
        assert_eq!(persisted.tokens.access_token, stored.tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            stored.tokens.refresh_token.as_deref()
        );
        fs::remove_dir_all(&data_dir).expect("cleanup temp dir");
    }

    #[test]
    fn sync_account_from_auth_dir_updates_store_for_managed_home() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-auth-dir-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_account_from_auth_dir(&stored.id, &managed_home).expect("sync account");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );

        let persisted = load_account(&stored.id).expect("persisted account");
        assert_eq!(persisted.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            persisted.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
    }

    #[test]
    fn managed_projection_sync_requires_projection_marker() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-managed-projection-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "seed",
            "rt-seed",
        ));
        let managed_home = env.home_dir.join("managed-homes").join(&stored.id);
        write_oauth_auth_file(&managed_home, &stored.tokens, "acc-current");
        write_managed_projection_to_dir(&managed_home, &stored).expect("write managed projection");

        let latest_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "managed",
            "rt-managed",
        );
        write_oauth_auth_file(&managed_home, &latest_tokens, "acc-current");

        let synced = sync_managed_projection_from_auth_dir(&stored.id, &managed_home)
            .expect("sync managed projection");
        assert_eq!(synced.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            synced.tokens.refresh_token.as_deref(),
            latest_tokens.refresh_token.as_deref()
        );
        assert!(synced.token_generation > stored.token_generation);
    }

    #[test]
    fn config_toml_uses_openai_base_url_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-openai-base-url-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.example.com/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("openai_base_url = \"https://api.example.com\""));
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("codex_local_access"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: Some("https://api.example.com".to_string()),
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_skips_default_official_endpoint_for_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-openai-default-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        assert!(!config_path.exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_removes_runtime_provider_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-clean-managed-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "codex_local_access"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "cockpit-provider-model-catalog.json"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-history"

[model_providers.cockpit_api]
name = "Cockpit Api"
base_url = "https://chongcodex.cn/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.openai_api_key]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.user_manual_provider_not_managed]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false
"#,
        )
        .expect("write managed provider config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_provider = "));
        assert!(!content.contains("[model_providers.codex_local_access]"));
        assert!(!content.contains("experimental_bearer_token = \"sk-history\""));
        assert!(!content.contains("[model_providers.cockpit_api]"));
        assert!(!content.contains("[model_providers.openai_api_key]"));
        assert!(content.contains("[model_providers.user_manual_provider_not_managed]"));
        assert!(!content.contains("model_catalog_json"));
        assert!(!content.contains("openai_base_url"));
        assert!(content.contains("model_context_window = 1000000"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::OpenaiBuiltin,
                base_url: None,
                provider_id: None,
                provider_name: None,
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_preserves_user_model_catalog_when_switching_to_builtin_openai() {
        let base_dir = make_temp_dir("codex-config-preserve-user-catalog-builtin-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "user_manual_provider"
model_catalog_json = "user-model-catalog.json"
model_context_window = 1000000

[model_providers.user_manual_provider]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false

[features]
multi_agent = true
"#,
        )
        .expect("write user provider config");
        let provider_config = resolve_api_provider_config(
            None,
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_provider = "));
        assert!(content.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(content.contains("[model_providers.user_manual_provider]"));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_preserves_user_model_catalog_when_switching_to_custom_provider() {
        let base_dir = make_temp_dir("codex-config-preserve-user-catalog-custom-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "user_manual_provider"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "user-model-catalog.json"
model_context_window = 1000000

[model_providers.user_manual_provider]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
requires_openai_auth = false

[features]
multi_agent = true
"#,
        )
        .expect("write user provider config");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"relay\""));
        assert!(content.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(content.contains("[model_providers.relay]"));
        assert!(content.contains("[model_providers.user_manual_provider]"));
        assert!(!content.contains("openai_base_url"));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn config_toml_uses_model_provider_section_for_custom_provider() {
        let base_dir = make_temp_dir("codex-config-custom-provider-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_provider_to_config_toml(&base_dir, &provider_config).expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"relay\""));
        assert!(content.contains("[model_providers.relay]"));
        assert!(!content.contains("codex_local_access"));
        assert!(content.contains("name = \"Relay\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = false"));
        assert!(content.contains("supports_websockets = false"));
        assert!(!content.contains("openai_base_url"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: Some("relay".to_string()),
                provider_name: Some("Relay".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_uses_fixed_provider_for_default_official_endpoint() {
        let base_dir = make_temp_dir("codex-api-key-config-openai-default-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            false,
            true,
        )
        .expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"codex_local_access\""));
        assert!(content.contains("[model_providers.codex_local_access]"));
        assert!(content.contains("name = \"OpenAI Official\""));
        assert!(content.contains("base_url = \"https://api.openai.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = true"));
        assert!(content.contains("experimental_bearer_token = \"sk-test\""));
        assert!(content.contains("supports_websockets = false"));
        assert!(!content.contains("openai_base_url"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://api.openai.com/v1".to_string()),
                provider_id: Some("codex_local_access".to_string()),
                provider_name: Some("OpenAI Official".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_uses_fixed_provider_for_custom_provider() {
        let base_dir = make_temp_dir("codex-api-key-config-custom-provider-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            false,
            true,
        )
        .expect("write config");

        let config_path = base_dir.join("config.toml");
        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"codex_local_access\""));
        assert!(content.contains("[model_providers.codex_local_access]"));
        assert!(!content.contains("[model_providers.relay]"));
        assert!(content.contains("name = \"Relay\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = true"));
        assert!(content.contains("experimental_bearer_token = \"sk-test\""));
        assert!(content.contains("supports_websockets = false"));
        assert!(!content.contains("openai_base_url"));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir),
            ApiProviderConfig {
                mode: CodexApiProviderMode::Custom,
                base_url: Some("https://relay.example.com/v1".to_string()),
                provider_id: Some("codex_local_access".to_string()),
                provider_name: Some("Relay".to_string()),
            }
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_enables_imagegen_for_capable_provider() {
        let base_dir = make_temp_dir("codex-api-key-config-imagegen-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"[model_providers.codex_local_access.http_headers]
X-Custom = "keep-me"
"#,
        )
        .expect("write existing headers");
        let provider_config = resolve_api_provider_config(
            Some("http://127.0.0.1:14998/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("codex_local_access"),
            Some("Codex API Service"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "agt_codex_test",
            false,
            true,
            false,
        )
        .expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        let parsed = content.parse::<Document>().expect("parse config");
        let provider = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex_local_access"))
            .and_then(|item| item.as_table())
            .expect("codex_local_access provider");
        assert_eq!(
            provider
                .get("requires_openai_auth")
                .and_then(|item| item.as_bool()),
            Some(false)
        );
        let headers = provider
            .get("http_headers")
            .and_then(|item| item.as_table())
            .expect("http_headers table");
        assert_eq!(
            headers
                .get(CODEX_IMAGEGEN_ACTOR_HEADER)
                .and_then(|item| item.as_str()),
            Some(CODEX_IMAGEGEN_ACTOR_HEADER_VALUE)
        );
        assert_eq!(
            headers
                .get(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER)
                .and_then(|item| item.as_str()),
            Some(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE)
        );
        assert_eq!(
            headers.get("X-Custom").and_then(|item| item.as_str()),
            Some("keep-me")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn remote_api_key_imagegen_does_not_disable_hosted_chat_tool() {
        let base_dir = make_temp_dir("codex-remote-api-key-imagegen-test");
        let provider_config = resolve_api_provider_config(
            Some("https://api.apikey.fun/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("apikey_fun"),
            Some("APIKey.fun"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            true,
            false,
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(!content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_config_toml_removes_imagegen_header_but_keeps_custom_headers() {
        let base_dir = make_temp_dir("codex-api-key-config-imagegen-cleanup-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"[model_providers.codex_local_access]
http_headers = { "x-openai-actor-authorization" = "legacy", "X-Custom" = "keep-me" }
"#,
        )
        .expect("write existing headers");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            false,
            true,
        )
        .expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        let parsed = content.parse::<Document>().expect("parse config");
        let provider = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex_local_access"))
            .and_then(|item| item.as_table())
            .expect("codex_local_access provider");
        assert_eq!(
            provider
                .get("requires_openai_auth")
                .and_then(|item| item.as_bool()),
            Some(true)
        );
        let headers = provider
            .get("http_headers")
            .and_then(|item| item.as_inline_table())
            .expect("http_headers inline table");
        assert!(headers
            .iter()
            .all(|(name, _)| { !name.eq_ignore_ascii_case(CODEX_IMAGEGEN_ACTOR_HEADER) }));
        assert_eq!(
            headers.get("X-Custom").and_then(|item| item.as_str()),
            Some("keep-me")
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bundle_enables_imagegen_when_catalog_contains_image_model() {
        let base_dir = make_temp_dir("codex-api-key-bundle-imagegen-test");
        let account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "agt_codex_test".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("requires_openai_auth = false"));
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(content.contains(CODEX_IMAGEGEN_ACTOR_HEADER_VALUE));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn pure_third_party_without_image_catalog_clears_stale_actor_headers() {
        let base_dir = make_temp_dir("codex-third-party-clear-stale-actor");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "sk-old"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools" }
supports_websockets = false
"#,
        )
        .expect("seed stale imagegen config");

        let account = CodexAccount::new_api_key(
            "relay-no-image".to_string(),
            "relay@example.com".to_string(),
            "sk-new".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );

        write_account_bundle_to_dir(&base_dir, &account).expect("rewrite without image catalog");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(
            !content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "stale actor must be cleared when catalog has no gpt-image-2: {content}"
        );
        assert!(content.contains("experimental_bearer_token = \"sk-new\""));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn pure_api_key_local_access_writes_imagegen_takeover_shape() {
        let base_dir = make_temp_dir("codex-local-access-pure-api-key-takeover-shape");
        let provider_config = resolve_api_provider_config(
            Some("http://localhost:12345/v1"),
            Some(CodexApiProviderMode::Custom),
            Some("codex_local_access"),
            Some("Codex API Service"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "agt_codex_test",
            false,
            true,
            false,
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(
            content.contains("requires_openai_auth = false"),
            "pure API Key local-access must disable openai auth gate: {content}"
        );
        assert!(
            content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "pure API Key local-access must write actor header: {content}"
        );
        assert!(
            content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER),
            "pure API Key local-access should keep chat images-only header: {content}"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bound_oauth_keeps_oauth_login_and_imagegen_when_catalog_has_image() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-api-key-bound-oauth-auth-test");
        let base_dir = make_temp_dir("codex-api-key-bound-oauth-auth-test");
        let mut oauth = CodexAccount::new(
            "oauth-bound-auth-test".to_string(),
            "oauth@example.com".to_string(),
            CodexTokens {
                id_token: "id.token.value".to_string(),
                access_token: "access.token".to_string(),
                refresh_token: Some("refresh.token".to_string()),
            },
        );
        oauth.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
        save_account(&oauth).expect("save oauth");

        let mut api_key = CodexAccount::new_api_key(
            "api-key-bound-auth-test".to_string(),
            "api@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string(), "gpt-5.5".to_string()],
        );
        api_key.bound_oauth_account_id = Some(oauth.id.clone());
        save_account(&api_key).expect("save api key");

        write_account_bundle_to_dir(&base_dir, &api_key).expect("write bound oauth bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(
            content.contains("requires_openai_auth = true"),
            "bound OAuth must enable openai auth gate so Codex uses OAuth login: {content}"
        );
        assert!(
            content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "third-party bound OAuth with image catalog must write actor for imagegen: {content}"
        );
        // 非 loopback 不写 chat disable
        assert!(
            !content.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER),
            "third-party should not set chat-only image disable: {content}"
        );

        let auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(base_dir.join("auth.json")).expect("auth"))
                .expect("parse auth");
        assert!(
            auth.get("tokens").is_some(),
            "auth should keep oauth tokens"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
        let _ = remove_accounts(&[oauth.id, api_key.id]);
    }

    #[test]
    fn api_key_bound_oauth_without_image_catalog_skips_actor() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let _env = TestEnvGuard::new("codex-api-key-bound-oauth-no-image-test");
        let base_dir = make_temp_dir("codex-api-key-bound-oauth-no-image-test");
        let mut oauth = CodexAccount::new(
            "oauth-bound-no-image-test".to_string(),
            "oauth-no-image@example.com".to_string(),
            CodexTokens {
                id_token: "id.token.value".to_string(),
                access_token: "access.token".to_string(),
                refresh_token: Some("refresh.token".to_string()),
            },
        );
        oauth.auth_mode = crate::models::codex::CodexAuthMode::OAuth;
        save_account(&oauth).expect("save oauth");

        let mut api_key = CodexAccount::new_api_key(
            "api-key-bound-no-image-test".to_string(),
            "api-no-image@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5.5".to_string()],
        );
        api_key.bound_oauth_account_id = Some(oauth.id.clone());
        save_account(&api_key).expect("save api key");

        write_account_bundle_to_dir(&base_dir, &api_key).expect("write bound oauth bundle");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("requires_openai_auth = true"));
        assert!(
            !content.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "no image model in catalog → no actor: {content}"
        );

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
        let _ = remove_accounts(&[oauth.id, api_key.id]);
    }

    #[test]
    fn api_key_config_toml_enables_websockets_when_account_supports_them() {
        let base_dir = make_temp_dir("codex-api-key-config-websocket-test");
        let provider_config = resolve_api_provider_config(
            Some("https://relay.example.com/v1/"),
            Some(CodexApiProviderMode::Custom),
            Some("relay"),
            Some("Relay"),
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            true,
            false,
            true,
        )
        .expect("write config");

        let content = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(content.contains("supports_websockets = true"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn provider_snapshot_sync_updates_account_and_current_config_without_touching_last_used() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-provider-snapshot-sync-test");
        let mut account = CodexAccount::new_api_key(
            "relay-account".to_string(),
            "relay@example.com".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        account.last_used = 123;
        save_account(&account).expect("save account");

        let mut index = CodexAccountIndex::new();
        index.current_account_id = Some(account.id.clone());
        save_account_index(&index).expect("save account index");

        let updated = sync_api_key_provider_accounts(
            vec![account.id.clone(), account.id.clone()],
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["gpt-5".to_string()],
            Some("responses".to_string()),
            true,
            false,
            Default::default(),
            None,
        )
        .expect("sync provider snapshot");

        assert_eq!(updated, 1);
        let saved = load_account(&account.id).expect("load updated account");
        assert!(saved.api_supports_websockets);
        assert_eq!(saved.api_wire_api.as_deref(), Some("responses"));
        assert_eq!(saved.api_model_catalog, vec!["gpt-5".to_string()]);
        assert_eq!(saved.last_used, 123);

        let config =
            fs::read_to_string(env.codex_home().join("config.toml")).expect("read current config");
        assert!(config.contains("supports_websockets = true"));
    }

    #[test]
    fn api_key_bundle_bound_to_empty_id_token_oauth_writes_api_key_auth_file() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-oauth-auth-file-test");
        let mut oauth_tokens = make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "empty-id-token",
            "rt-empty-id-token",
        );
        oauth_tokens.id_token = String::new();
        let oauth_account = seed_oauth_account(oauth_tokens);

        let mut api_key_account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "local-service-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            Vec::new(),
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth file"),
        )
        .expect("parse auth file");
        assert_eq!(
            auth_file.get("auth_mode").and_then(|value| value.as_str()),
            Some("apikey")
        );
        assert_eq!(
            auth_file
                .get("OPENAI_API_KEY")
                .and_then(|value| value.as_str()),
            Some("local-service-key")
        );
        assert!(
            auth_file.get("tokens").is_none(),
            "API-key local access profile should not write OAuth tokens: {}",
            auth_file
        );

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("base_url = \"http://127.0.0.1:14998/v1\""));
        assert!(config.contains("experimental_bearer_token = \"local-service-key\""));
    }

    #[test]
    fn api_key_bundle_bound_to_full_oauth_keeps_oauth_auth_file() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-full-oauth-auth-file-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "full",
            "rt-full",
        ));

        let mut api_key_account = CodexAccount::new_api_key(
            "local-access-runtime".to_string(),
            "api-service-local".to_string(),
            "local-service-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://127.0.0.1:14998/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth file"),
        )
        .expect("parse auth file");
        assert!(auth_file.get("auth_mode").is_none());
        assert_eq!(
            auth_file.get("OPENAI_API_KEY"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            auth_file
                .get("tokens")
                .and_then(|value| value.get("id_token"))
                .and_then(|value| value.as_str()),
            Some(oauth_account.tokens.id_token.as_str())
        );

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains("experimental_bearer_token = \"local-service-key\""));
        // local-access loopback + bound OAuth → also write imagegen headers
        assert!(config.contains(CODEX_IMAGEGEN_ACTOR_HEADER));
        assert!(config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER));
    }

    #[test]
    fn local_access_runtime_bound_oauth_keeps_oauth_login_and_imagegen() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-local-access-bound-oauth-takeover-shape");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "bound@example.com",
            "acc-bound",
            "org-bound",
            "bound-oauth",
            "rt-bound-oauth",
        ));

        let mut runtime = CodexAccount::new_api_key(
            "codex_local_access_runtime".to_string(),
            "api-service-local".to_string(),
            "agt_codex_takeover".to_string(),
            CodexApiProviderMode::Custom,
            Some("http://localhost:12345/v1".to_string()),
            Some("codex_local_access".to_string()),
            Some("Codex API Service".to_string()),
            vec![CODEX_IMAGE_MODEL_ID.to_string()],
        );
        runtime.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("api-service-profile");

        write_account_bundle_to_dir(&profile_dir, &runtime).expect("write bound oauth takeover");

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(
            config.contains("requires_openai_auth = true"),
            "bound OAuth local-access must enable openai auth gate: {config}"
        );
        assert!(
            config.contains(CODEX_IMAGEGEN_ACTOR_HEADER),
            "bound OAuth local-access must write actor for imagegen: {config}"
        );
        assert!(
            config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER)
                && config.contains(CODEX_DISABLE_HOSTED_IMAGE_GENERATION_HEADER_VALUE),
            "bound OAuth local-access must disable hosted chat imagegen: {config}"
        );
        assert!(config.contains("experimental_bearer_token = \"agt_codex_takeover\""));
        assert!(config.contains("base_url = \"http://localhost:12345/v1\""));

        let auth_file: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(profile_dir.join("auth.json")).expect("read auth"),
        )
        .expect("parse auth");
        assert!(
            auth_file.get("tokens").is_some(),
            "auth.json should keep bound OAuth tokens"
        );
        assert!(auth_file.get("auth_mode").is_none());

        let _ = remove_accounts(&[oauth_account.id]);
    }

    #[test]
    fn responses_api_key_bundle_syncs_saved_model_catalog_when_enabled() {
        let base_dir = make_temp_dir("codex-api-key-managed-model-catalog-test");
        fs::write(base_dir.join("config.toml"), "model = \"legacy-model\"\n")
            .expect("write stale selected model");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![
                " custom-a ".to_string(),
                "custom-b".to_string(),
                "CUSTOM-A".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-provider-model-catalog.json\""));
        assert!(config.contains("model = \"custom-a\""));
        let catalog: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE))
                .expect("read managed catalog"),
        )
        .expect("parse managed catalog");
        let slugs = catalog
            .get("models")
            .and_then(serde_json::Value::as_array)
            .expect("models should be an array")
            .iter()
            .filter_map(|model| model.get("slug").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(slugs, vec!["custom-a", "custom-b", "codex-auto-review"]);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_upsert_without_sync_preference_preserves_instance_model_catalog() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .expect("lock test env");
        let env = TestEnvGuard::new("codex-api-key-upsert-model-catalog-test");
        let api_key = "sk-upsert-model-catalog".to_string();

        let created = upsert_api_key_account(
            api_key.clone(),
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["custom-a".to_string()],
            Some(true),
            Some("responses".to_string()),
            false,
            false,
            std::collections::HashMap::new(),
            None,
            Some("Relay Key".to_string()),
        )
        .expect("create API key account");
        assert!(created.api_sync_model_catalog_to_codex);

        let updated = upsert_api_key_account(
            api_key,
            Some("https://relay.example.com/v1".to_string()),
            Some(CodexApiProviderMode::Custom),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["custom-b".to_string()],
            None,
            Some("responses".to_string()),
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
        )
        .expect("upsert API key account without sync preference");
        assert!(updated.api_sync_model_catalog_to_codex);

        let profile_dir = env.home_dir.join("instance-profile");
        write_account_bundle_to_dir(&profile_dir, &updated)
            .expect("write multi-instance account projection");
        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"cockpit-provider-model-catalog.json\""));
        assert!(config.contains("model = \"custom-b\""));
        assert!(profile_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
    }

    #[test]
    fn responses_api_key_bundle_preserves_user_model_catalog() {
        let base_dir = make_temp_dir("codex-api-key-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_catalog_json = "user-model-catalog.json"
"#,
        )
        .expect("write user catalog config");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec![
                " custom-a ".to_string(),
                "custom-b".to_string(),
                "CUSTOM-A".to_string(),
            ],
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_sync_model_catalog_to_codex = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn responses_api_key_bundle_removes_stale_managed_model_catalog() {
        let base_dir = make_temp_dir("codex-api-key-empty-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[]}"#,
        )
        .expect("write managed catalog");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());
        account.api_supports_websockets = true;

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("supports_websockets = true"));
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn cleanup_removes_existing_managed_model_catalog() {
        let base_dir = make_temp_dir("codex-managed-model-catalog-cleanup-test");
        fs::write(
            base_dir.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write config");
        fs::write(
            base_dir.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[]}"#,
        )
        .expect("write stale catalog");

        assert!(super::cleanup_managed_model_catalog_for_dir(&base_dir)
            .expect("cleanup managed catalog"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn managed_catalog_cleanup_preserves_custom_model_catalog() {
        let base_dir = make_temp_dir("codex-custom-model-catalog-cleanup-test");
        fs::write(
            base_dir.join("config.toml"),
            "model_catalog_json = \"user-model-catalog.json\"\n",
        )
        .expect("write custom config");
        fs::write(
            base_dir.join("user-model-catalog.json"),
            r#"{"models":[{"slug":"user-model"}]}"#,
        )
        .expect("write custom catalog");

        assert!(!super::cleanup_managed_model_catalog_for_dir(&base_dir)
            .expect("preserve custom catalog"));
        assert_eq!(
            fs::read_to_string(base_dir.join("user-model-catalog.json"))
                .expect("read custom catalog"),
            r#"{"models":[{"slug":"user-model"}]}"#
        );
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn startup_cleanup_preserves_active_chat_completions_provider_catalog() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-chat-provider-startup-catalog-test");
        let mut account = CodexAccount::new_api_key(
            "deepseek-api-key".to_string(),
            "deepseek@example.com".to_string(),
            "sk-deepseek".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://api.deepseek.com/v1".to_string()),
            Some("deepseek".to_string()),
            Some("DeepSeek".to_string()),
            vec!["deepseek-v4-pro".to_string()],
        );
        account.api_wire_api = Some("chat_completions".to_string());
        save_account(&account).expect("save chat completions account");
        save_account_index(&build_test_account_index(&account))
            .expect("save current account index");

        let codex_home = env.codex_home();
        fs::write(
            codex_home.join("config.toml"),
            format!(
                "model_catalog_json = \"{}\"\n",
                super::CODEX_MANAGED_MODEL_CATALOG_FILE
            ),
        )
        .expect("write provider catalog config");
        fs::write(
            codex_home.join(super::CODEX_MANAGED_MODEL_CATALOG_FILE),
            r#"{"models":[{"slug":"deepseek-v4-pro"}]}"#,
        )
        .expect("write provider catalog");

        assert_eq!(
            super::cleanup_managed_model_catalogs_on_startup().expect("startup cleanup"),
            0
        );
        assert!(codex_home
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
        assert!(fs::read_to_string(codex_home.join("config.toml"))
            .expect("read provider config")
            .contains("model_catalog_json"));
    }

    #[test]
    fn responses_api_key_bundle_keeps_external_catalog_without_managed_catalog() {
        let base_dir = make_temp_dir("codex-api-key-user-model-catalog-test");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_catalog_json = "user-model-catalog.json"
"#,
        )
        .expect("write config");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_catalog_json = \"user-model-catalog.json\""));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn chat_completions_api_key_bundle_defers_catalog_to_provider_gateway_start() {
        let base_dir = make_temp_dir("codex-chat-api-key-model-catalog-test");
        let mut account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["chat-model".to_string()],
        );
        account.api_wire_api = Some("chat_completions".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn builtin_openai_responses_api_key_bundle_uses_official_model_discovery() {
        let base_dir = make_temp_dir("codex-builtin-responses-model-catalog-test");
        let mut account = CodexAccount::new_api_key(
            "openai-api-key".to_string(),
            "openai@example.com".to_string(),
            "sk-openai".to_string(),
            CodexApiProviderMode::OpenaiBuiltin,
            Some("https://api.openai.com/v1".to_string()),
            None,
            None,
            Vec::new(),
        );
        account.api_wire_api = Some("responses".to_string());

        write_account_bundle_to_dir(&base_dir, &account).expect("write account bundle");

        let config = fs::read_to_string(base_dir.join("config.toml")).expect("read config");
        assert!(!config.contains("model_catalog_json"));
        assert!(!base_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn api_key_bundle_bound_to_oauth_uses_dynamic_model_discovery() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-api-key-bound-oauth-model-catalog-test");
        let oauth_account = seed_oauth_account(make_codex_tokens(
            "demo@example.com",
            "acc-current",
            "org-current",
            "full",
            "rt-full",
        ));

        let mut api_key_account = CodexAccount::new_api_key(
            "custom-api-key".to_string(),
            "custom@example.com".to_string(),
            "sk-custom".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            vec!["provider-model".to_string()],
        );
        api_key_account.api_wire_api = Some("responses".to_string());
        api_key_account.bound_oauth_account_id = Some(oauth_account.id.clone());
        let profile_dir = env.home_dir.join("managed-profile");

        write_account_bundle_to_dir(&profile_dir, &api_key_account).expect("write account bundle");

        let config = fs::read_to_string(profile_dir.join("config.toml")).expect("read config");
        assert!(config.contains("model_provider = \"codex_local_access\""));
        assert!(!config.contains("model_catalog_json"));
        assert!(!profile_dir
            .join(super::CODEX_MANAGED_MODEL_CATALOG_FILE)
            .exists());
    }

    #[test]
    fn api_key_config_toml_only_updates_codex_local_access() {
        let base_dir = make_temp_dir("codex-config-clean-provider-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            r#"model_provider = "mimo"
openai_base_url = "https://legacy.example.com/v1"
model_catalog_json = "cockpit-provider-model-catalog.json"
model_context_window = 1000000

[model_providers.mimo]
name = "Mimo"
base_url = "https://mimo.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[model_providers.cockpit_api]
name = "Cockpit Api"
base_url = "https://chongcodex.cn/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.openai_api_key]
name = "OpenAI Official"
base_url = "https://api.openai.com/v1"
wire_api = "responses"
requires_openai_auth = false

[model_providers.codex_local_access]
name = "Old Local Access"
base_url = "https://old-local.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-old"
custom_flag = "keep-me"

[model_providers.relay]
name = "Relay"
base_url = "https://relay.example.com/v1"
wire_api = "responses"
requires_openai_auth = true

[features]
multi_agent = true
"#,
        )
        .expect("write legacy config");
        let provider_config = resolve_api_provider_config(
            Some("https://api.openai.com/v1/"),
            Some(CodexApiProviderMode::OpenaiBuiltin),
            None,
            None,
        )
        .expect("resolve provider config");

        write_api_key_provider_to_config_toml(
            &base_dir,
            &provider_config,
            "sk-test",
            false,
            false,
            true,
        )
        .expect("write config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_provider = \"codex_local_access\""));
        assert!(content.contains("[model_providers.codex_local_access]"));
        assert!(content.contains("base_url = \"https://api.openai.com/v1\""));
        assert!(content.contains("experimental_bearer_token = \"sk-test\""));
        assert!(content.contains("custom_flag = \"keep-me\""));
        assert!(content.contains("[model_providers.mimo]"));
        assert!(content.contains("[model_providers.cockpit_api]"));
        assert!(content.contains("[model_providers.openai_api_key]"));
        assert!(content.contains("[model_providers.relay]"));
        assert!(content.contains("model_catalog_json = \"cockpit-provider-model-catalog.json\""));
        assert!(content.contains("openai_base_url = \"https://legacy.example.com/v1\""));
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("[features]"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_reads_custom_context_window_without_hiding_it() {
        let base_dir = make_temp_dir("codex-quick-config-custom-window-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 200000\nmodel_auto_compact_token_limit = 180000\n",
        )
        .expect("write config");

        let quick_config =
            read_quick_config_from_config_toml(&base_dir).expect("read quick config");
        assert!(!quick_config.context_window_1m);
        assert_eq!(quick_config.auto_compact_token_limit, 180000);
        assert_eq!(quick_config.detected_model_context_window, Some(200000));
        assert_eq!(quick_config.detected_auto_compact_token_limit, Some(180000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_enable_1m_context_window() {
        let base_dir = make_temp_dir("codex-quick-config-enable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result = write_quick_config_to_config_toml(&base_dir, Some(1_000_000), Some(880000))
            .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 1000000"));
        assert!(content.contains("model_auto_compact_token_limit = 880000"));
        assert_eq!(result.context_window_1m, true);
        assert_eq!(result.auto_compact_token_limit, 880000);
        assert_eq!(
            result.detected_model_context_window,
            Some(CODEX_CONTEXT_WINDOW_1M_VALUE)
        );
        assert_eq!(result.detected_auto_compact_token_limit, Some(880000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_remove_managed_fields() {
        let base_dir = make_temp_dir("codex-quick-config-disable-test");
        let config_path = base_dir.join("config.toml");
        fs::write(
            &config_path,
            "model_context_window = 1000000\nmodel_auto_compact_token_limit = 900000\nmodel = \"gpt-5\"\n",
        )
        .expect("write config");

        let result =
            write_quick_config_to_config_toml(&base_dir, None, None).expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(!content.contains("model_context_window"));
        assert!(!content.contains("model_auto_compact_token_limit"));
        assert!(content.contains("model = \"gpt-5\""));
        assert!(!result.context_window_1m);
        assert_eq!(
            result.auto_compact_token_limit,
            CODEX_AUTO_COMPACT_DEFAULT_LIMIT
        );
        assert_eq!(result.detected_model_context_window, None);
        assert_eq!(result.detected_auto_compact_token_limit, None);

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_can_write_custom_context_window_and_compact_limit() {
        let base_dir = make_temp_dir("codex-quick-config-custom-write-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let result = write_quick_config_to_config_toml(&base_dir, Some(516_000), Some(460_000))
            .expect("save quick config");

        let content = fs::read_to_string(&config_path).expect("read config");
        assert!(content.contains("model_context_window = 516000"));
        assert!(content.contains("model_auto_compact_token_limit = 460000"));
        assert!(!result.context_window_1m);
        assert_eq!(result.auto_compact_token_limit, 460_000);
        assert_eq!(result.detected_model_context_window, Some(516_000));
        assert_eq!(result.detected_auto_compact_token_limit, Some(460_000));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn quick_config_rejects_non_positive_context_window() {
        let base_dir = make_temp_dir("codex-quick-config-invalid-context-test");
        let config_path = base_dir.join("config.toml");
        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");

        let err = write_quick_config_to_config_toml(&base_dir, Some(0), Some(100_000))
            .expect_err("context window should be rejected");
        assert!(err.contains("上下文窗口必须大于 0"));

        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    fn validate_api_key_credentials_rejects_url_api_key() {
        let err = validate_api_key_credentials("http://127.0.0.1:3000/v1", None)
            .expect_err("url should be rejected as api key");
        assert!(err.contains("API Key 不能是 URL"));
    }

    #[test]
    fn validate_api_key_credentials_rejects_invalid_base_url() {
        let err = validate_api_key_credentials("sk-test-key", Some("not-a-url"))
            .expect_err("invalid base url should be rejected");
        assert!(err.contains("Base URL 格式无效"));
    }

    #[test]
    fn validate_api_key_credentials_accepts_valid_values() {
        let (api_key, api_base_url) =
            validate_api_key_credentials("  sk-test-key  ", Some("https://relay.local/v1/"))
                .expect("valid api key + base url should pass");
        assert_eq!(api_key, "sk-test-key");
        assert_eq!(api_base_url.as_deref(), Some("https://relay.local/v1"));
    }

    #[test]
    fn loopback_http_base_url_detection() {
        assert!(is_loopback_http_base_url(Some("http://localhost:53549/v1")));
        assert!(is_loopback_http_base_url(Some("http://127.0.0.1:53549/v1")));
        assert!(is_loopback_http_base_url(Some("http://[::1]:53549/v1")));
        assert!(!is_loopback_http_base_url(Some("https://relay.example/v1")));
        assert!(!is_loopback_http_base_url(None));
    }

    #[test]
    fn sync_api_key_account_skips_local_access_loopback_provider() {
        let base_dir = make_temp_dir("codex-sync-api-key-local-access");
        fs::write(
            base_dir.join("auth.json"),
            r#"{
              "auth_mode": "apikey",
              "OPENAI_API_KEY": "sk-test-key"
            }"#,
        )
        .expect("write auth");
        fs::write(
            base_dir.join("config.toml"),
            r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex Local Access"
base_url = "http://localhost:53549/v1"
wire_api = "responses"
"#,
        )
        .expect("write config");

        let mut account = CodexAccount::new_api_key(
            "api-1".to_string(),
            "api-key@example.com".to_string(),
            "sk-test-key".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        let original_base = account.api_base_url.clone();
        let original_provider_id = account.api_provider_id.clone();

        sync_api_key_account_from_local_state(&mut account, &base_dir);

        assert_eq!(account.api_base_url, original_base);
        assert_eq!(account.api_provider_id, original_provider_id);
        fs::remove_dir_all(&base_dir).expect("cleanup temp dir");
    }

    #[test]
    #[ignore = "manual local Codex repair smoke test"]
    fn local_codex_index_repair_smoke() {
        crate::modules::logger::init_logger();

        let index_path = get_accounts_storage_path();
        let accounts_dir = get_accounts_dir();
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 索引路径: {}",
            index_path.display()
        );
        eprintln!(
            "[LocalCodexRepairTest] 检测到本地 Codex 详情目录: {}",
            accounts_dir.display()
        );

        let accounts = list_accounts_checked().expect("local Codex repair should succeed");
        let index = load_account_index();
        eprintln!(
            "[LocalCodexRepairTest] 修复/读取完成: accounts={}, current_account_id={}",
            accounts.len(),
            index.current_account_id.as_deref().unwrap_or("-")
        );

        if let Ok(log_file) = crate::modules::logger::get_latest_app_log_file() {
            eprintln!(
                "[LocalCodexRepairTest] 应用日志文件: {}",
                log_file.display()
            );
        }
    }

    #[test]
    fn codex_group_quota_policy_defaults_to_inherit() {
        let groups: Vec<CodexAccountGroupRecord> =
            serde_json::from_str(r#"[{"accountIds":["a1"]}]"#).expect("parse");
        assert_eq!(groups[0].policy(), CodexGroupQuotaRefreshPolicy::Inherit);
    }

    #[test]
    fn codex_group_quota_policy_supports_disabled_and_custom() {
        let groups: Vec<CodexAccountGroupRecord> = serde_json::from_str(
            r#"[
              {"accountIds":["a1"],"quotaAutoRefreshMinutes":-1},
              {"accountIds":["a2"],"quotaAutoRefreshMinutes":5},
              {"accountIds":["a3"],"quotaRefreshEnabled":false}
            ]"#,
        )
        .expect("parse");
        assert_eq!(groups[0].policy(), CodexGroupQuotaRefreshPolicy::Disabled);
        assert_eq!(groups[1].policy(), CodexGroupQuotaRefreshPolicy::Minutes(5));
        assert_eq!(groups[2].policy(), CodexGroupQuotaRefreshPolicy::Disabled);
    }
}

/// 从本地文件导入 Codex 账号（支持多种 JSON 格式）
pub async fn import_from_files(file_paths: Vec<String>) -> Result<CodexFileImportResult, String> {
    use std::path::Path;

    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;

    logger::log_info(&format!(
        "Codex: 开始从 {} 个文件导入账号...",
        file_paths.len()
    ));

    // 原有文件导入候选: (CodexTokens, account_id_hint, label, auth_file_plan_type)
    let mut candidates: Vec<(CodexTokens, Option<String>, String, Option<String>)> = Vec::new();
    // 旧规则未识别到账号时，才用 Token/JSON 粘贴框的解析逻辑处理整个文件内容。
    let mut fallback_files: Vec<(String, String, Option<String>)> = Vec::new();

    for file_path in &file_paths {
        let path = Path::new(file_path);
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                logger::log_error(&format!("读取文件失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        // 从文件名推断 email 作为 label
        let filename_label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let auth_file_plan_type = detect_auth_file_plan_type_from_path(path);

        let parsed: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                logger::log_warn(&format!(
                    "Codex 文件旧规则 JSON 解析失败，将尝试 Token/JSON 导入逻辑 {:?}: {}",
                    file_path, e
                ));
                fallback_files.push((content, filename_label, auth_file_plan_type));
                continue;
            }
        };

        let before_count = candidates.len();
        match &parsed {
            serde_json::Value::Object(_) => {
                if let Some((tokens, hint)) = extract_codex_tokens_from_value(&parsed) {
                    candidates.push((
                        tokens,
                        hint,
                        filename_label.clone(),
                        auth_file_plan_type.clone(),
                    ));
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    if let Some((tokens, hint)) = extract_codex_tokens_from_value(item) {
                        let label = item
                            .get("email")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&filename_label)
                            .to_string();
                        candidates.push((tokens, hint, label, auth_file_plan_type.clone()));
                    }
                }
            }
            _ => {}
        }

        if candidates.len() == before_count {
            logger::log_info(&format!(
                "Codex 文件旧规则未找到账号，将尝试 Token/JSON 导入逻辑 {:?}",
                file_path
            ));
            fallback_files.push((content, filename_label, auth_file_plan_type));
        }
    }

    if candidates.is_empty() && fallback_files.is_empty() {
        return Err(
            "未找到有效的 Codex Token（需要 accessToken/access_token、id_token + access_token，或 refresh_token）"
                .to_string(),
        );
    }

    logger::log_info(&format!(
        "Codex: 发现 {} 个旧格式候选账号，{} 个文件待尝试 Token/JSON 导入逻辑...",
        candidates.len(),
        fallback_files.len()
    ));

    let mut imported = Vec::new();
    let mut failed: Vec<CodexFileImportFailure> = Vec::new();
    let total = candidates.len() + fallback_files.len();
    let mut progress_index = 0usize;

    for (tokens, account_id_hint, label, auth_file_plan_type) in candidates {
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match upsert_account_with_hints(tokens, account_id_hint, None) {
            Ok(mut account) => {
                if apply_auth_file_plan_type(&mut account, auth_file_plan_type) {
                    save_account(&account)?;
                }
                logger::log_info(&format!("Codex 导入成功: {}", account.email));
                imported.push(account);
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    for (content, label, auth_file_plan_type) in fallback_files {
        progress_index += 1;
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": progress_index,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match import_from_json(&content).await {
            Ok(accounts) => {
                for mut account in accounts {
                    if apply_auth_file_plan_type(&mut account, auth_file_plan_type.clone()) {
                        save_account(&account)?;
                    }
                    logger::log_info(&format!("Codex 导入成功: {}", account.email));
                    imported.push(account);
                }
            }
            Err(e) => {
                if is_disk_full_error_message(&e) {
                    logger::log_error(&format!(
                        "Codex 导入因磁盘空间不足终止: label={}, imported={}, error={}",
                        label,
                        imported.len(),
                        e
                    ));
                    return Err(format!(
                        "磁盘空间不足，已终止导入（已成功 {} 个）。{}",
                        imported.len(),
                        e
                    ));
                }
                logger::log_error(&format!("Codex 导入失败 {}: {}", label, e));
                failed.push(CodexFileImportFailure {
                    email: label,
                    error: e,
                });
            }
        }
    }

    logger::log_info(&format!(
        "Codex 文件导入完成，成功 {} 个，失败 {} 个",
        imported.len(),
        failed.len()
    ));

    Ok(CodexFileImportResult { imported, failed })
}

pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.tags = Some(tags);
    save_account(&account)?;

    Ok(account)
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CodexAccountNoteUpdate {
    pub note: Option<String>,
    pub two_factor_secret: Option<String>,
    pub account_password: Option<String>,
    pub phone_number: Option<String>,
    pub mail_url: Option<String>,
}

fn apply_account_note_update(account: &mut CodexAccount, update: CodexAccountNoteUpdate) {
    if let Some(note) = update.note {
        account.account_note = normalize_optional_value(Some(note));
    }
    if let Some(secret) = update.two_factor_secret {
        account.two_factor_secret = normalize_optional_value(Some(secret));
    }
    if let Some(password) = update.account_password {
        account.account_password = normalize_optional_value(Some(password));
    }
    if let Some(phone_number) = update.phone_number {
        account.phone_number = normalize_optional_value(Some(phone_number));
    }
    if let Some(mail_url) = update.mail_url {
        account.mail_url = normalize_optional_value(Some(mail_url));
    }
}

pub fn update_account_note(
    account_id: &str,
    update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    apply_account_note_update(&mut account, update);
    save_account(&account)?;

    Ok(account)
}

pub fn create_pending_oauth_account(
    email: String,
    update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let email =
        normalize_optional_value(Some(email)).ok_or_else(|| "账号邮箱不能为空".to_string())?;
    let mut index = load_account_index();

    if let Some(summary) = index
        .accounts
        .iter()
        .find(|item| item.email.eq_ignore_ascii_case(&email))
        .cloned()
    {
        if let Some(mut account) = load_account(&summary.id) {
            if !is_pending_oauth_account(&account) {
                return Err(format!("Codex 账号已存在: {}", email));
            }
            apply_account_note_update(&mut account, update);
            account.email = email.clone();
            account.last_used = chrono::Utc::now().timestamp();
            save_account(&account)?;
            if let Some(item) = index.accounts.iter_mut().find(|item| item.id == account.id) {
                item.email = account.email.clone();
                item.plan_type = account.plan_type.clone();
                item.subscription_active_until = account.subscription_active_until.clone();
                item.last_used = account.last_used;
            }
            save_account_index(&index)?;
            return Ok(account);
        }
    }

    let account_id = build_account_storage_id(&email, Some("pending_oauth"), None);
    let now = chrono::Utc::now().timestamp();
    let mut account = if let Some(mut account) = load_account(&account_id) {
        if !is_pending_oauth_account(&account) {
            return Err(format!("Codex 账号已存在: {}", email));
        }
        account.email = email.clone();
        account.last_used = now;
        account
    } else {
        let mut account = CodexAccount::new(
            account_id.clone(),
            email.clone(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.auth_mode = CodexAuthMode::OAuth;
        account.authorization_status = Some(CODEX_AUTHORIZATION_STATUS_PENDING.to_string());
        account.token_updated_at = None;
        account.token_generation = 0;
        account.requires_reauth = false;
        account.reauth_reason = None;
        account.quota = None;
        account.quota_error = None;
        account.created_at = now;
        account.last_used = now;
        account
    };
    apply_account_note_update(&mut account, update);

    index.accounts.retain(|item| item.id != account_id);
    index.accounts.push(account_summary_from_account(&account));

    save_account(&account)?;
    save_account_index(&index)?;
    logger::log_info(&format!(
        "Codex 待授权 OAuth 账号已保存: account_id={}, email={}",
        account.id, account.email
    ));

    Ok(account)
}

pub fn update_account_app_speed(
    account_id: &str,
    speed: CodexAppSpeed,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    account.app_speed = speed;
    save_account(&account)?;

    Ok(account)
}

pub async fn update_api_key_bound_oauth_account(
    account_id: &str,
    bound_oauth_account_id: Option<String>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持绑定 OAuth 账号".to_string());
    }

    let bound_id = normalize_optional_ref(bound_oauth_account_id.as_deref());
    if let Some(bound_id) = bound_id.as_deref() {
        let _ = validate_api_key_bound_oauth_account(&account, bound_id)?;
    }
    account.bound_oauth_account_id = bound_id.clone();
    // 绑定 OAuth：不走本地网关生图兼容（与改前一致，保证绑定可展示、客户端能力正常）。
    // 纯 API Key 生图仍走 gpt-image-2 + actor header，不依赖此标志。
    account.bound_oauth_use_local_gateway = false;
    save_account(&account)?;

    let is_current = load_account_index()
        .current_account_id
        .as_deref()
        .map(|current_id| current_id == account.id)
        .unwrap_or(false);
    if is_current {
        let codex_home = get_codex_home();
        if bound_id.is_some() {
            let oauth_account =
                refresh_bound_oauth_account_for_api_key(&account, "bind-oauth").await?;
            write_api_key_account_bundle_with_oauth_to_dir(&codex_home, &account, &oauth_account)?;
            activate_provider_gateway_after_switch_if_needed(&codex_home, &account).await?;
        } else {
            crate::modules::codex_local_access::stop_provider_gateways_for_profile(&codex_home)
                .await;
            write_prepared_account_bundle_to_dir(&codex_home, &account)?;
        }
    }

    Ok(account)
}

pub fn update_api_key_credentials(
    account_id: &str,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Vec<String>,
    api_sync_model_catalog_to_codex: Option<bool>,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持编辑凭据".to_string());
    }

    let (normalized_key, normalized_base_url) =
        validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        normalized_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let old_id = account.id.clone();
    let new_id = build_api_key_account_id(&normalized_key);
    let mut index = load_account_index();
    let was_current = get_current_account()
        .map(|current| current.id == old_id)
        .unwrap_or(false);

    if new_id != old_id && index.accounts.iter().any(|item| item.id == new_id) {
        return Err("该 API Key 已存在，请直接使用已有账号".to_string());
    }

    if new_id != old_id {
        account.id = new_id.clone();
    }

    let sync_model_catalog_to_codex =
        api_sync_model_catalog_to_codex.unwrap_or(account.api_sync_model_catalog_to_codex);
    apply_api_key_fields(
        &mut account,
        &normalized_key,
        provider_config,
        api_model_catalog,
        sync_model_catalog_to_codex,
        api_wire_api,
        api_supports_websockets,
        api_supports_vision,
        api_model_vision_support,
        api_vision_routing_model,
    );
    account.update_last_used();
    save_account(&account)?;

    if old_id != account.id {
        delete_account_file(&old_id)?;
    }

    let mut summary_found = false;
    for summary in &mut index.accounts {
        if summary.id == old_id {
            summary.id = account.id.clone();
            summary.email = account.email.clone();
            summary.plan_type = account.plan_type.clone();
            summary.subscription_active_until = account.subscription_active_until.clone();
            summary.last_used = account.last_used;
            summary_found = true;
            break;
        }
    }

    if !summary_found {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    if index.current_account_id.as_deref() == Some(old_id.as_str()) {
        index.current_account_id = Some(account.id.clone());
    }
    save_account_index(&index)?;

    if old_id != account.id {
        if let Err(err) =
            crate::modules::codex_instance::replace_bind_account_references(&old_id, &account.id)
        {
            logger::log_warn(&format!(
                "Codex API Key 账号编辑后同步实例绑定失败: old_id={}, new_id={}, error={}",
                old_id, account.id, err
            ));
        }
    }

    if was_current {
        let codex_home = get_codex_home();
        write_account_bundle_to_dir(&codex_home, &account)?;
    }

    logger::log_info(&format!(
        "Codex API Key 账号凭据已更新: old_id={}, new_id={}, has_base_url={}",
        old_id,
        account.id,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));

    Ok(account)
}

pub fn sync_api_key_provider_accounts(
    account_ids: Vec<String>,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
    api_model_catalog: Vec<String>,
    api_wire_api: Option<String>,
    api_supports_websockets: bool,
    api_supports_vision: bool,
    api_model_vision_support: std::collections::HashMap<String, bool>,
    api_vision_routing_model: Option<String>,
) -> Result<usize, String> {
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let current_account_id = load_account_index().current_account_id;
    let mut seen = HashSet::new();
    let mut updated_accounts = Vec::new();

    for account_id in account_ids {
        if !seen.insert(account_id.clone()) {
            continue;
        }
        let Some(mut account) = load_account(&account_id) else {
            continue;
        };
        if !account.is_api_key_auth() {
            continue;
        }
        let api_key = normalize_api_key(account.openai_api_key.as_deref().unwrap_or_default())
            .ok_or_else(|| format!("API Key 账号缺少密钥: {}", account.id))?;
        let sync_model_catalog_to_codex = account.api_sync_model_catalog_to_codex;
        apply_api_key_fields(
            &mut account,
            &api_key,
            provider_config.clone(),
            api_model_catalog.clone(),
            sync_model_catalog_to_codex,
            api_wire_api.clone(),
            api_supports_websockets,
            api_supports_vision,
            api_model_vision_support.clone(),
            api_vision_routing_model.clone(),
        );
        save_account(&account)?;
        updated_accounts.push(account);
    }

    if let Some(current_account) = updated_accounts
        .iter()
        .find(|account| current_account_id.as_deref() == Some(account.id.as_str()))
    {
        write_account_bundle_to_dir(&get_codex_home(), current_account)?;
    }

    Ok(updated_accounts.len())
}

pub fn update_account_name(account_id: &str, name: String) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;

    if !account.is_api_key_auth() {
        return Err("仅 API Key 账号支持重命名".to_string());
    }

    account.account_name = normalize_optional_value(Some(name));
    save_account(&account)?;

    Ok(account)
}

fn normalize_quota_alert_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_threshold(raw: i32) -> i32 {
    raw.clamp(0, 100)
}

fn normalize_auto_switch_account_scope_mode(raw: &str) -> String {
    let normalized = raw.trim().to_lowercase();
    if normalized == CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED.to_string()
    } else {
        CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL.to_string()
    }
}

fn normalize_auto_switch_selected_account_ids(raw: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for item in raw {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        result.push(normalized);
    }
    result
}

fn resolve_monitored_auto_switch_account_ids(
    scope_mode: &str,
    selected_account_ids: &[String],
    accounts: &[CodexAccount],
) -> HashSet<String> {
    if scope_mode != CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED {
        return accounts.iter().map(|account| account.id.clone()).collect();
    }

    let selected = normalize_auto_switch_selected_account_ids(selected_account_ids);
    if selected.is_empty() {
        return HashSet::new();
    }

    let existing: HashSet<&str> = accounts.iter().map(|account| account.id.as_str()).collect();
    selected
        .into_iter()
        .filter(|account_id| existing.contains(account_id.as_str()))
        .collect()
}

fn format_codex_quota_metric_label(window_minutes: Option<i64>, fallback: &str) -> String {
    const HOUR_MINUTES: i64 = 60;
    const DAY_MINUTES: i64 = 24 * HOUR_MINUTES;
    const WEEK_MINUTES: i64 = 7 * DAY_MINUTES;

    let Some(minutes) = window_minutes.filter(|value| *value > 0) else {
        return fallback.to_string();
    };

    if minutes >= WEEK_MINUTES - 1 {
        let weeks = (minutes + WEEK_MINUTES - 1) / WEEK_MINUTES;
        return if weeks <= 1 {
            "Weekly".to_string()
        } else {
            format!("{} Week", weeks)
        };
    }

    if minutes >= DAY_MINUTES - 1 {
        let days = (minutes + DAY_MINUTES - 1) / DAY_MINUTES;
        return format!("{}d", days);
    }

    if minutes >= HOUR_MINUTES {
        let hours = (minutes + HOUR_MINUTES - 1) / HOUR_MINUTES;
        return format!("{}h", hours);
    }

    format!("{}m", minutes)
}

#[derive(Debug, Clone)]
struct CodexQuotaMetric {
    key: &'static str,
    label: String,
    percentage: i32,
}

fn extract_quota_metrics(account: &CodexAccount) -> Vec<CodexQuotaMetric> {
    let Some(quota) = account.quota.as_ref() else {
        return Vec::new();
    };

    let has_presence =
        quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
    let mut metrics = Vec::new();

    if !has_presence || quota.hourly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    if !has_presence || quota.weekly_window_present.unwrap_or(false) {
        metrics.push(CodexQuotaMetric {
            key: "secondary_window",
            label: format_codex_quota_metric_label(quota.weekly_window_minutes, "Weekly"),
            percentage: quota.weekly_percentage.clamp(0, 100),
        });
    }

    if metrics.is_empty() {
        metrics.push(CodexQuotaMetric {
            key: "primary_window",
            label: format_codex_quota_metric_label(quota.hourly_window_minutes, "5h"),
            percentage: quota.hourly_percentage.clamp(0, 100),
        });
    }

    metrics
}

fn average_quota_percentage(metrics: &[CodexQuotaMetric]) -> f64 {
    if metrics.is_empty() {
        return 0.0;
    }
    let sum: i32 = metrics.iter().map(|metric| metric.percentage).sum();
    sum as f64 / metrics.len() as f64
}

fn metric_crossed_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage <= primary_threshold,
        "secondary_window" => metric.percentage <= secondary_threshold,
        _ => false,
    }
}

fn metric_above_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> bool {
    match metric.key {
        "primary_window" => metric.percentage > primary_threshold,
        "secondary_window" => metric.percentage > secondary_threshold,
        _ => true,
    }
}

fn metric_margin_over_threshold(
    metric: &CodexQuotaMetric,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<i32> {
    match metric.key {
        "primary_window" => Some(metric.percentage - primary_threshold),
        "secondary_window" => Some(metric.percentage - secondary_threshold),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct CodexSwitchCandidate {
    account: CodexAccount,
    min_margin: i32,
    min_percentage: i32,
    average_percentage: f64,
}

fn build_switch_candidate(
    account: &CodexAccount,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexSwitchCandidate> {
    let metrics = extract_quota_metrics(account);
    if metrics.is_empty() {
        return None;
    }
    if !metrics
        .iter()
        .all(|metric| metric_above_threshold(metric, primary_threshold, secondary_threshold))
    {
        return None;
    }

    let min_margin = metrics
        .iter()
        .filter_map(|metric| {
            metric_margin_over_threshold(metric, primary_threshold, secondary_threshold)
        })
        .min()?;
    let min_percentage = metrics.iter().map(|metric| metric.percentage).min()?;
    let average_percentage = average_quota_percentage(&metrics);

    Some(CodexSwitchCandidate {
        account: account.clone(),
        min_margin,
        min_percentage,
        average_percentage,
    })
}

fn pick_best_candidate(mut candidates: Vec<CodexSwitchCandidate>) -> Option<CodexAccount> {
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        b.min_margin
            .cmp(&a.min_margin)
            .then_with(|| b.min_percentage.cmp(&a.min_percentage))
            .then_with(|| {
                b.average_percentage
                    .partial_cmp(&a.average_percentage)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.account.last_used.cmp(&b.account.last_used))
    });

    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.account)
}

fn build_quota_alert_cooldown_key(
    account_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> String {
    format!(
        "codex:{}:{}:{}",
        account_id, primary_threshold, secondary_threshold
    )
}

fn should_emit_quota_alert(cooldown_key: &str, now: i64) -> bool {
    let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() else {
        return true;
    };

    if let Some(last_sent) = state.get(cooldown_key) {
        if now - *last_sent < CODEX_QUOTA_ALERT_COOLDOWN_SECONDS {
            return false;
        }
    }

    state.insert(cooldown_key.to_string(), now);
    true
}

fn clear_quota_alert_cooldown(account_id: &str, primary_threshold: i32, secondary_threshold: i32) {
    if let Ok(mut state) = CODEX_QUOTA_ALERT_LAST_SENT.lock() {
        state.remove(&build_quota_alert_cooldown_key(
            account_id,
            primary_threshold,
            secondary_threshold,
        ));
    }
}

pub(crate) fn resolve_current_account_id(accounts: &[CodexAccount]) -> Option<String> {
    let current_id = get_current_account()?.id;
    accounts
        .iter()
        .any(|account| account.id == current_id)
        .then_some(current_id)
}

fn pick_quota_alert_recommendation(
    accounts: &[CodexAccount],
    current_id: &str,
    primary_threshold: i32,
    secondary_threshold: i32,
) -> Option<CodexAccount> {
    let candidates: Vec<CodexSwitchCandidate> = accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter_map(|account| {
            build_switch_candidate(account, primary_threshold, secondary_threshold)
        })
        .collect();

    pick_best_candidate(candidates)
}

pub fn pick_auto_switch_target_if_needed() -> Result<Option<CodexAccount>, String> {
    if CODEX_AUTO_SWITCH_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        logger::log_info("[AutoSwitch][Codex] 自动切号进行中，跳过本次检查");
        return Ok(None);
    }

    let result = (|| {
        let cfg = crate::modules::config::get_user_config();
        if !cfg.codex_auto_switch_enabled {
            return Ok(None);
        }

        let primary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_primary_threshold);
        let secondary_threshold =
            normalize_auto_switch_threshold(cfg.codex_auto_switch_secondary_threshold);
        let account_scope_mode =
            normalize_auto_switch_account_scope_mode(&cfg.codex_auto_switch_account_scope_mode);

        let accounts = list_accounts();
        let monitored_account_ids = resolve_monitored_auto_switch_account_ids(
            &account_scope_mode,
            &cfg.codex_auto_switch_selected_account_ids,
            &accounts,
        );
        if monitored_account_ids.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 可监控账号范围为空(scope={})，跳过自动切号",
                account_scope_mode
            ));
            return Ok(None);
        }
        let current_id = match resolve_current_account_id(&accounts) {
            Some(id) => id,
            None => return Ok(None),
        };
        if !monitored_account_ids.contains(&current_id) {
            logger::log_info(&format!(
                "[AutoSwitch][Codex] 当前账号不在监控范围内(current_id={}, scope={})，跳过自动切号",
                current_id, account_scope_mode
            ));
            return Ok(None);
        }

        let current = match accounts.iter().find(|account| account.id == current_id) {
            Some(account) => account,
            None => return Ok(None),
        };

        let current_metrics = extract_quota_metrics(current);
        if current_metrics.is_empty() {
            return Ok(None);
        }

        let should_switch = current_metrics
            .iter()
            .any(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold));
        if !should_switch {
            return Ok(None);
        }

        let candidates: Vec<CodexSwitchCandidate> = accounts
            .iter()
            .filter(|account| monitored_account_ids.contains(&account.id))
            .filter(|account| account.id != current_id)
            .filter_map(|account| {
                build_switch_candidate(account, primary_threshold, secondary_threshold)
            })
            .collect();

        if candidates.is_empty() {
            logger::log_warn(&format!(
                "[AutoSwitch][Codex] 当前账号命中阈值 (primary<={}%, secondary<={}%)，但没有可切换候选账号",
                primary_threshold, secondary_threshold
            ));
            return Ok(None);
        }

        Ok(pick_best_candidate(candidates))
    })();

    CODEX_AUTO_SWITCH_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

pub fn run_quota_alert_if_needed(
) -> Result<Option<crate::modules::account::QuotaAlertPayload>, String> {
    let cfg = crate::modules::config::get_user_config();
    if !cfg.codex_quota_alert_enabled {
        return Ok(None);
    }

    let primary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_primary_threshold);
    let secondary_threshold =
        normalize_quota_alert_threshold(cfg.codex_quota_alert_secondary_threshold);
    let accounts = list_accounts();
    let current_id = match resolve_current_account_id(&accounts) {
        Some(id) => id,
        None => return Ok(None),
    };

    let current = match accounts.iter().find(|account| account.id == current_id) {
        Some(account) => account,
        None => return Ok(None),
    };

    let metrics = extract_quota_metrics(current);
    let low_models: Vec<(String, i32)> = metrics
        .into_iter()
        .filter(|metric| metric_crossed_threshold(metric, primary_threshold, secondary_threshold))
        .map(|metric| (metric.label, metric.percentage))
        .collect();

    if low_models.is_empty() {
        clear_quota_alert_cooldown(&current_id, primary_threshold, secondary_threshold);
        return Ok(None);
    }

    let now = chrono::Utc::now().timestamp();
    let cooldown_key =
        build_quota_alert_cooldown_key(&current_id, primary_threshold, secondary_threshold);
    if !should_emit_quota_alert(&cooldown_key, now) {
        return Ok(None);
    }

    let recommendation = pick_quota_alert_recommendation(
        &accounts,
        &current_id,
        primary_threshold,
        secondary_threshold,
    );
    let lowest_percentage = low_models.iter().map(|(_, pct)| *pct).min().unwrap_or(0);
    let payload = crate::modules::account::QuotaAlertPayload {
        platform: "codex".to_string(),
        current_account_id: current_id,
        current_email: current.email.clone(),
        threshold: primary_threshold,
        threshold_display: Some(format!(
            "primary_window<={}%, secondary_window<={}%",
            primary_threshold, secondary_threshold
        )),
        lowest_percentage,
        low_models: low_models.into_iter().map(|(name, _)| name).collect(),
        recommended_account_id: recommendation.as_ref().map(|account| account.id.clone()),
        recommended_email: recommendation.as_ref().map(|account| account.email.clone()),
        triggered_at: now,
    };

    crate::modules::account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
