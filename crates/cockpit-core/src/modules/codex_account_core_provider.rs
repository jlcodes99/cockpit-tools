// cockpit-core Codex 账号：Provider identity, account-check parsing and quick config。
// 通过 include! 保持原模块作用域和凭据调用路径。
use crate::models::codex::{
    CodexAccount, CodexAccountIndex, CodexAccountSummary, CodexApiProviderMode, CodexAuthFile,
    CodexAuthMode, CodexAuthTokens, CodexJwtPayload, CodexQuickConfig, CodexTokens,
};
use crate::modules::{account, codex_oauth, logger};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
#[cfg(target_os = "macos")]
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use toml_edit::{value, Document};

static CODEX_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_TOKEN_REFRESH_LOCKS: std::sync::LazyLock<
    Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const CODEX_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const ACCOUNT_CHECK_URL: &str = "https://chatgpt.com/backend-api/wham/accounts/check";
const API_KEY_LOGIN_PLAN_TYPE: &str = "API_KEY";
const API_KEY_EMAIL_PREFIX: &str = "api-key";
const API_KEY_AUTH_MODE: &str = "apikey";
const CODEX_AUTH_TYPE: &str = "codex";
const CODEX_CONFIG_FILE_NAME: &str = "config.toml";
const CODEX_CONFIG_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CONFIG_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_CONFIG_MODEL_PROVIDERS_KEY: &str = "model_providers";
const CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY: &str = "model_context_window";
const CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const CODEX_DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const CODEX_COCKPIT_API_PROVIDER_ID: &str = "cockpit_api";
const CODEX_OPENAI_PROVIDER_ID: &str = "openai";
const CODEX_RUNTIME_MODEL_PROVIDER_ID: &str = "codex_local_access";
const CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID: &str = "openai_api_key";
const CODEX_PROVIDER_WIRE_API: &str = "responses";
const CODEX_CONTEXT_WINDOW_1M_VALUE: i64 = 1_000_000;
const CODEX_AUTO_COMPACT_DEFAULT_LIMIT: i64 = 900_000;
#[cfg(target_os = "macos")]
const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
const DISK_FULL_ERROR_CODE: &str = "DISK_FULL";
const CODEX_TOKEN_SOURCE_MANAGED: &str = "managed";
const CODEX_AUTH_PROJECTION_FILE_NAME: &str = ".cockpit_codex_auth.json";
const CODEX_AUTH_PROJECTION_WRITER: &str = "cockpit";

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
    if config_provider.provider_id.is_some() {
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

fn apply_api_key_fields(
    account: &mut CodexAccount,
    api_key: &str,
    provider_config: ApiProviderConfig,
) {
    account.auth_mode = CodexAuthMode::Apikey;
    account.openai_api_key = Some(api_key.to_string());
    account.api_base_url = provider_config.base_url;
    account.api_provider_mode = provider_config.mode;
    account.api_provider_id = provider_config.provider_id;
    account.api_provider_name = provider_config.provider_name;
    account.email = build_api_key_email(api_key);
    account.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
    account.tokens = CodexTokens {
        id_token: String::new(),
        access_token: String::new(),
        refresh_token: None,
    };
    account.user_id = None;
    account.account_id = None;
    account.organization_id = None;
    account.account_structure = None;
    account.quota = None;
    account.quota_error = None;
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

    let doc = content
        .parse::<Document>()
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
    context_window_1m: bool,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    let config_path = get_config_toml_path(base_dir);
    let existing = fs::read_to_string(&config_path).unwrap_or_default();

    if existing.trim().is_empty() && !context_window_1m {
        return read_quick_config_from_config_toml(base_dir);
    }

    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        existing
            .parse::<Document>()
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    if context_window_1m {
        let compact_limit = auto_compact_token_limit.unwrap_or(CODEX_AUTO_COMPACT_DEFAULT_LIMIT);
        if compact_limit <= 0 {
            return Err("自动压缩阈值必须大于 0".to_string());
        }
        doc[CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY] = value(CODEX_CONTEXT_WINDOW_1M_VALUE);
        doc[CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY] = value(compact_limit);
    } else {
        let _ = doc.remove(CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY);
        let _ = doc.remove(CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY);
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    fs::write(&config_path, doc.to_string())
        .map_err(|e| format!("写入 config.toml 失败: {}", e))?;

    read_quick_config_from_config_toml(base_dir)
}

pub fn save_current_quick_config(
    context_window_1m: bool,
    auto_compact_token_limit: Option<i64>,
) -> Result<CodexQuickConfig, String> {
    write_quick_config_to_config_toml(
        &get_codex_home(),
        context_window_1m,
        auto_compact_token_limit,
    )
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

    let doc = match content.parse::<Document>() {
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
        existing
            .parse::<Document>()
            .map_err(|e| format!("解析 config.toml 失败: {}", e))?
    };

    match provider_config.mode {
        CodexApiProviderMode::OpenaiBuiltin => {
            let _ = doc.remove(CODEX_CONFIG_MODEL_PROVIDER_KEY);
            remove_managed_api_key_model_providers_from_doc(&mut doc);
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
    fs::write(&config_path, doc.to_string()).map_err(|e| format!("写入 config.toml 失败: {}", e))
}

fn collect_managed_api_key_provider_ids() -> HashSet<String> {
    HashSet::from([
        CODEX_RUNTIME_MODEL_PROVIDER_ID.to_string(),
        CODEX_COCKPIT_API_PROVIDER_ID.to_string(),
        CODEX_LEGACY_API_KEY_OPENAI_PROVIDER_ID.to_string(),
    ])
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

fn write_api_key_provider_to_config_toml(
    base_dir: &Path,
    provider_config: &ApiProviderConfig,
) -> Result<(), String> {
    // Keep relay identity in Cockpit's account record while routing the runtime through Codex's
    // built-in `openai` provider with this account's own base URL and auth.json API key.
    let builtin_openai_config = resolve_api_provider_config(
        provider_config.base_url.as_deref(),
        Some(CodexApiProviderMode::OpenaiBuiltin),
        None,
        None,
    )?;
    write_api_provider_to_config_toml(base_dir, &builtin_openai_config)
}

/// 旧版数据目录（~/Library/Application Support/com.antigravity.cockpit-tools/）
fn get_old_codex_data_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(data_dir) = std::env::var_os("COCKPIT_TOOLS_DATA_DIR") {
        return PathBuf::from(data_dir).join("legacy-codex-data");
    }

    dirs::data_local_dir()
        .unwrap_or_else(|| dirs::home_dir().expect("无法获取用户目录"))
        .join("com.antigravity.cockpit-tools")
}

/// 将旧目录中的 codex 数据迁移到新目录（一次性，迁移成功后删除旧文件）
fn migrate_codex_data_if_needed(new_data_dir: &PathBuf) {
    #[cfg(test)]
    if std::env::var_os("COCKPIT_TOOLS_DATA_DIR").is_some() {
        // Test guards use an explicit data root; never migrate the user's legacy files.
        return;
    }

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

