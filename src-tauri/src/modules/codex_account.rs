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
use std::sync::Mutex;
use toml_edit::{value, Document};

static CODEX_QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CODEX_AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const CODEX_QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const ACCOUNT_CHECK_URL: &str = "https://chatgpt.com/backend-api/wham/accounts/check";
const API_KEY_LOGIN_PLAN_TYPE: &str = "API_KEY";
const API_KEY_EMAIL_PREFIX: &str = "api-key";
const API_KEY_AUTH_MODE: &str = "apikey";
const CODEX_CONFIG_FILE_NAME: &str = "config.toml";
const CODEX_CONFIG_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_CONFIG_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_CONFIG_MODEL_PROVIDERS_KEY: &str = "model_providers";
const CODEX_CONFIG_MODEL_CONTEXT_WINDOW_KEY: &str = "model_context_window";
const CODEX_CONFIG_MODEL_AUTO_COMPACT_TOKEN_LIMIT_KEY: &str = "model_auto_compact_token_limit";
const CODEX_DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const CODEX_OPENAI_PROVIDER_ID: &str = "openai";
const CODEX_PROVIDER_WIRE_API: &str = "responses";
const CODEX_CONTEXT_WINDOW_1M_VALUE: i64 = 1_000_000;
const CODEX_AUTO_COMPACT_DEFAULT_LIMIT: i64 = 900_000;
#[cfg(target_os = "macos")]
const CODEX_KEYCHAIN_SERVICE: &str = "Codex Auth";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_ALL: &str = "all_accounts";
const CODEX_AUTO_SWITCH_ACCOUNT_SCOPE_SELECTED: &str = "selected_accounts";
const DISK_FULL_ERROR_CODE: &str = "DISK_FULL";

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
            provider_table["requires_openai_auth"] = value(true);
        }
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 config.toml 目录失败: {}", e))?;
    }
    fs::write(&config_path, doc.to_string()).map_err(|e| format!("写入 config.toml 失败: {}", e))
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

fn should_force_refresh_token(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("token_invalidated")
        || lower.contains("your authentication token has been invalidated")
        || lower.contains("401 unauthorized")
}

pub fn extract_chatgpt_account_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    normalize_optional_ref(auth_data.get("chatgpt_account_id").and_then(|v| v.as_str()))
}

pub fn extract_chatgpt_organization_id_from_access_token(access_token: &str) -> Option<String> {
    let payload = decode_jwt_payload_value(access_token)?;
    let auth_data = payload.get("https://api.openai.com/auth")?;
    const ORG_KEYS: [&str; 4] = [
        "organization_id",
        "chatgpt_organization_id",
        "chatgpt_org_id",
        "org_id",
    ];
    for key in ORG_KEYS {
        if let Some(value) = normalize_optional_ref(auth_data.get(key).and_then(|v| v.as_str())) {
            return Some(value);
        }
    }
    None
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
    }

    if expected_account_id.is_some() || expected_org_id.is_some() {
        return None;
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
    ),
    String,
> {
    let payload = decode_jwt_payload(id_token)?;

    let email = payload.email.ok_or("id_token 中缺少 email")?;
    let user_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_user_id.clone());
    let plan_type = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.chatgpt_plan_type.clone());
    let account_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.account_id.clone());
    let organization_id = payload
        .auth_data
        .as_ref()
        .and_then(|d| d.organization_id.clone());

    Ok((email, user_id, plan_type, account_id, organization_id))
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
            Ok(index) if !index.accounts.is_empty() => index,
            Ok(_) => repair_account_index_from_details("索引账号列表为空")
                .unwrap_or_else(CodexAccountIndex::new),
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
        Ok(index) if !index.accounts.is_empty() => Ok(index),
        Ok(index) => {
            logger::log_warn(&format!(
                "[Codex Account][Repair] 账号索引可解析但列表为空，准备尝试自动修复: path={}",
                path.display()
            ));
            if let Some(repaired) = repair_account_index_from_details("索引账号列表为空") {
                logger::log_info(&format!(
                    "[Codex Account][Repair] 空账号列表已自动修复完成: recovered_accounts={}",
                    repaired.accounts.len()
                ));
                return Ok(repaired);
            }
            Ok(index)
        }
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
    let content = serde_json::to_string_pretty(index).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号索引失败: {}", e))?;
    Ok(())
}

fn repair_account_index_from_details(reason: &str) -> Option<CodexAccountIndex> {
    let index_path = get_accounts_storage_path();
    let accounts_dir = get_accounts_dir();
    logger::log_warn(&format!(
        "[Codex Account][Repair] 检测到索引异常，开始按详情文件重建: reason={}, index_path={}, accounts_dir={}",
        reason,
        index_path.display(),
        accounts_dir.display()
    ));

    let mut accounts = match crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id),
    ) {
        Ok(accounts) => accounts,
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

    if accounts.is_empty() {
        logger::log_warn(&format!(
            "[Codex Account][Repair] 账号详情目录中未发现可恢复账号，放弃自动修复: reason={}, accounts_dir={}",
            reason,
            accounts_dir.display()
        ));
        return None;
    }

    logger::log_info(&format!(
        "[Codex Account][Repair] 已扫描到 {} 个账号详情，准备重建索引",
        accounts.len()
    ));

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = CodexAccountIndex::new();
    index.accounts = accounts
        .iter()
        .map(|account| CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        })
        .collect();
    index.current_account_id = accounts.first().map(|account| account.id.clone());

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

/// 读取单个账号详情
pub fn load_account(account_id: &str) -> Option<CodexAccount> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if !path.exists() {
        return None;
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).ok(),
        Err(_) => None,
    }
}

/// 保存单个账号详情
pub fn save_account(account: &CodexAccount) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", &account.id));
    let content =
        serde_json::to_string_pretty(account).map_err(|e| format!("序列化失败: {}", e))?;
    write_string_atomic(&path, &content).map_err(|e| format!("写入账号详情失败: {}", e))?;
    Ok(())
}

/// 删除单个账号
pub fn delete_account_file(account_id: &str) -> Result<(), String> {
    let path = get_accounts_dir().join(format!("{}.json", account_id));
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("删除文件失败: {}", e))?;
    }
    Ok(())
}

/// 列出所有账号
pub fn list_accounts() -> Vec<CodexAccount> {
    let index = load_account_index();
    index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect()
}

pub fn list_accounts_checked() -> Result<Vec<CodexAccount>, String> {
    let index = load_account_index_checked()?;
    Ok(index
        .accounts
        .iter()
        .filter_map(|summary| load_account(&summary.id))
        .collect())
}

/// 刷新账号资料（团队名/结构）
async fn refresh_account_profile_once(account_id: &str) -> Result<CodexAccount, String> {
    let mut account = prepare_account_for_injection(account_id).await?;
    if account.is_api_key_auth() {
        return Ok(account);
    }

    let (account_name, account_structure, account_id_from_remote) =
        match fetch_remote_account_profile(&account).await {
            Ok(profile) => profile,
            Err(err) if should_force_refresh_token(&err) => {
                let refresh_token = account.tokens.refresh_token.clone().ok_or(err.clone())?;

                logger::log_warn(&format!(
                    "Codex 账号资料请求检测到失效 Token，准备强制刷新后重试: account={}, error={}",
                    account.email, err
                ));

                account.tokens = codex_oauth::refresh_access_token_with_fallback(
                    &refresh_token,
                    Some(account.tokens.id_token.as_str()),
                )
                .await
                .map_err(|e| format!("账号资料接口返回 Token 失效，刷新 Token 失败: {}", e))?;
                save_account(&account)?;

                fetch_remote_account_profile(&account).await?
            }
            Err(err) => return Err(err),
        };

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
    crate::modules::refresh_retry::retry_once_with_delay(
        "Codex Profile Refresh",
        account_id,
        || async { refresh_account_profile_once(account_id).await },
    )
    .await
}

/// 添加或更新账号
pub fn upsert_account(tokens: CodexTokens) -> Result<CodexAccount, String> {
    upsert_account_with_hints(tokens, None, None)
}

pub fn upsert_api_key_account(
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
) -> Result<CodexAccount, String> {
    let (api_key, api_base_url) = validate_api_key_credentials(&api_key, api_base_url.as_deref())?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        api_provider_mode,
        api_provider_id.as_deref(),
        api_provider_name.as_deref(),
    )?;
    let account_id = build_api_key_account_id(&api_key);
    let mut index = load_account_index();
    let existing = index.accounts.iter().position(|item| item.id == account_id);

    let mut account = if let Some(pos) = existing {
        let existing_id = index.accounts[pos].id.clone();
        let mut acc = load_account(&existing_id).unwrap_or_else(|| {
            CodexAccount::new_api_key(
                existing_id,
                build_api_key_email(&api_key),
                api_key.clone(),
                provider_config.mode.clone(),
                provider_config.base_url.clone(),
                provider_config.provider_id.clone(),
                provider_config.provider_name.clone(),
            )
        });
        apply_api_key_fields(&mut acc, &api_key, provider_config.clone());
        if acc.email.trim().is_empty() {
            acc.email = build_api_key_email(&api_key);
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
        );
        acc.plan_type = Some(API_KEY_LOGIN_PLAN_TYPE.to_string());
        index.accounts.push(CodexAccountSummary {
            id: account_id.clone(),
            email: acc.email.clone(),
            plan_type: acc.plan_type.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    account.auth_mode = CodexAuthMode::Apikey;
    save_account(&account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
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
    let (email, user_id, plan_type, id_token_account_id, id_token_org_id) =
        extract_user_info(&tokens.id_token)?;
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

    // 优先按 email + account_id + organization_id 严格匹配已有账号
    let existing_id = find_existing_account_id(
        &index,
        &email,
        account_id.as_deref(),
        organization_id.as_deref(),
    )
    .unwrap_or_else(|| generated_id.clone());
    let existing = index.accounts.iter().position(|a| a.id == existing_id);

    let account = if let Some(pos) = existing {
        // 更新现有账号
        let existing_id = index.accounts[pos].id.clone();
        let mut acc = load_account(&existing_id)
            .unwrap_or_else(|| CodexAccount::new(existing_id, email.clone(), tokens.clone()));
        acc.tokens = tokens;
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.update_last_used();
        acc
    } else {
        // 创建新账号
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };

    // 保存账号详情
    save_account(&account)?;

    // 更新索引中的摘要信息
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
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
) -> Result<(), String> {
    let mut index = load_account_index();
    if let Some(summary) = index.accounts.iter_mut().find(|a| a.id == account_id) {
        summary.plan_type = plan_type.clone();
        save_account_index(&index)?;
    }
    Ok(())
}

/// 删除账号
pub fn remove_account(account_id: &str) -> Result<(), String> {
    let mut index = load_account_index();

    // 从索引中移除
    index.accounts.retain(|a| a.id != account_id);

    // 如果删除的是当前账号，清除 current_account_id
    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = None;
    }

    save_account_index(&index)?;
    delete_account_file(account_id)?;

    Ok(())
}

/// 批量删除账号
pub fn remove_accounts(account_ids: &[String]) -> Result<(), String> {
    for id in account_ids {
        remove_account(id)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct LocalCodexOAuthSnapshot {
    tokens: CodexTokens,
    email: String,
    account_id: Option<String>,
    organization_id: Option<String>,
}

fn build_local_oauth_snapshot(tokens: CodexAuthTokens) -> Option<LocalCodexOAuthSnapshot> {
    let (email, _, _, id_token_account_id, id_token_org_id) =
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
        account_id,
        organization_id,
    })
}

fn load_local_oauth_snapshot_from_dir(base_dir: &Path) -> Option<LocalCodexOAuthSnapshot> {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&auth_path).ok()?;
    let auth_file: CodexAuthFile = serde_json::from_str(&content).ok()?;
    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        return None;
    }

    build_local_oauth_snapshot(auth_file.tokens?)
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

    if account.tokens.id_token != snapshot.tokens.id_token {
        account.tokens.id_token = snapshot.tokens.id_token.clone();
        changed = true;
    }

    if account.tokens.access_token != snapshot.tokens.access_token {
        account.tokens.access_token = snapshot.tokens.access_token.clone();
        changed = true;
    }

    if let Some(refresh_token) = normalize_optional_ref(snapshot.tokens.refresh_token.as_deref()) {
        if account.tokens.refresh_token.as_deref() != Some(refresh_token.as_str()) {
            account.tokens.refresh_token = Some(refresh_token);
            changed = true;
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

    changed
}

fn sync_account_from_auth_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_dir(base_dir) else {
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        logger::log_info(&format!(
            "Codex 账号已从本地 auth.json 同步最新 Token: account_id={}, source_dir={}",
            account.id,
            base_dir.display()
        ));
    }

    Ok(true)
}

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

/// 获取当前激活的账号（基于 auth.json）
pub fn get_current_account() -> Option<CodexAccount> {
    let auth_path = get_auth_json_path();
    if !auth_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&auth_path).ok()?;
    let auth_file: CodexAuthFile = serde_json::from_str(&content).ok()?;
    let is_apikey_mode = is_auth_mode_apikey(auth_file.auth_mode.as_deref());
    let api_key = extract_api_key_from_auth_file(&auth_file);
    let config_provider = read_api_provider_from_config_toml(&get_codex_home());
    let current_provider = infer_api_provider_config(
        extract_api_base_url_from_auth_file(&auth_file)
            .or_else(|| config_provider.base_url.clone())
            .as_deref(),
        Some(config_provider.mode.clone()),
        config_provider.provider_id.as_deref(),
        config_provider.provider_name.as_deref(),
    );

    if is_apikey_mode || (auth_file.tokens.is_none() && api_key.is_some()) {
        let api_key = api_key?;
        let normalized_key = normalize_optional_ref(Some(api_key.as_str()))?;
        let accounts = list_accounts();
        if let Some(mut account) = accounts.into_iter().find(|account| {
            account.is_api_key_auth()
                && normalize_optional_ref(account.openai_api_key.as_deref())
                    == Some(normalized_key.clone())
        }) {
            let account_provider = infer_api_provider_config(
                account.api_base_url.as_deref(),
                Some(account.api_provider_mode.clone()),
                account.api_provider_id.as_deref(),
                account.api_provider_name.as_deref(),
            );
            if account_provider != current_provider {
                account.api_base_url = current_provider.base_url.clone();
                account.api_provider_mode = current_provider.mode.clone();
                account.api_provider_id = current_provider.provider_id.clone();
                account.api_provider_name = current_provider.provider_name.clone();
                let _ = save_account(&account);
            }
            return Some(account);
        }
        logger::log_info("当前 auth.json 为 API Key 模式，但本地账号库未命中，跳过自动补录");
        return None;
    }

    let snapshot = build_local_oauth_snapshot(auth_file.tokens?)?;

    // 在我们的账号列表中查找
    let accounts = list_accounts();
    if let Some(account_id) = snapshot.account_id.as_deref() {
        if let Some(account) = accounts.iter().find(|account| {
            account.email.eq_ignore_ascii_case(&snapshot.email)
                && normalize_optional_ref(account.account_id.as_deref())
                    == Some(account_id.to_string())
                && (snapshot.organization_id.is_none()
                    || normalize_optional_ref(account.organization_id.as_deref())
                        == snapshot.organization_id.clone())
        }) {
            let mut account = account.clone();
            if apply_local_oauth_snapshot(&mut account, &snapshot) {
                let _ = save_account(&account);
            }
            return Some(account);
        }
    }

    if let Some(organization_id) = snapshot.organization_id.as_deref() {
        if let Some(account) = accounts.iter().find(|account| {
            account.email.eq_ignore_ascii_case(&snapshot.email)
                && normalize_optional_ref(account.organization_id.as_deref())
                    == Some(organization_id.to_string())
        }) {
            let mut account = account.clone();
            if apply_local_oauth_snapshot(&mut account, &snapshot) {
                let _ = save_account(&account);
            }
            return Some(account);
        }
    }

    accounts.into_iter().find_map(|mut account| {
        if !account.email.eq_ignore_ascii_case(&snapshot.email) {
            return None;
        }
        if apply_local_oauth_snapshot(&mut account, &snapshot) {
            let _ = save_account(&account);
        }
        Some(account)
    })
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

    if account.tokens.id_token.trim().is_empty() || account.tokens.access_token.trim().is_empty() {
        return Err("OAuth 账号缺少 id_token/access_token，无法写入 auth.json".to_string());
    }

    serde_json::to_value(CodexAuthFile {
        auth_mode: None,
        openai_api_key: Some(serde_json::Value::Null),
        base_url: None,
        tokens: Some(CodexAuthTokens {
            id_token: account.tokens.id_token.clone(),
            access_token: account.tokens.access_token.clone(),
            refresh_token: account.tokens.refresh_token.clone(),
            account_id: account.account_id.clone(),
        }),
        last_refresh: Some(serde_json::Value::String(
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                .to_string(),
        )),
    })
    .map_err(|e| format!("auth.json 序列化失败: {}", e))
}

#[cfg(target_os = "macos")]
fn build_codex_keychain_account(base_dir: &Path) -> String {
    let resolved_home = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(resolved_home.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let digest_hex = format!("{:x}", digest);
    format!("cli|{}", &digest_hex[..16])
}

#[cfg(target_os = "macos")]
fn write_codex_keychain_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    if account.is_api_key_auth() {
        return Ok(());
    }

    let payload = build_auth_file_value(account)?;
    let secret = serde_json::to_string(&payload)
        .map_err(|e| format!("序列化 Codex keychain 数据失败: {}", e))?;
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
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "写入 Codex keychain 失败: status={}, stderr={}, stdout={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            },
            if stdout.trim().is_empty() {
                "<empty>"
            } else {
                stdout.trim()
            }
        ));
    }

    logger::log_info(&format!(
        "[Codex切号] 已更新 keychain 登录信息: service={}, account={}",
        CODEX_KEYCHAIN_SERVICE, keychain_account
    ));
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
    let parent = path.parent().ok_or("无法定位目标目录")?;
    fs::create_dir_all(parent).map_err(|e| format_io_error("创建目录", parent, &e))?;
    let temp_path = build_temp_file_path(parent, path, "atomic");
    fs::write(&temp_path, content).map_err(|e| format_io_error("写入临时文件", &temp_path, &e))?;
    if let Err(err) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format_io_error("替换文件", path, &err));
    }

    Ok(())
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
        infer_api_provider_config(
            account.api_base_url.as_deref(),
            Some(account.api_provider_mode.clone()),
            account.api_provider_id.as_deref(),
            account.api_provider_name.as_deref(),
        )
    } else {
        ApiProviderConfig {
            mode: CodexApiProviderMode::OpenaiBuiltin,
            base_url: None,
            provider_id: None,
            provider_name: None,
        }
    };
    write_api_provider_to_config_toml(base_dir, &provider_config)?;

    logger::log_info(&format!(
        "[Codex切号] 已写入登录信息: account_id={}, target_file={}, has_base_url={}",
        account.id,
        auth_path.display(),
        provider_config.base_url.is_some()
    ));

    Ok(())
}

pub fn write_account_bundle_to_dir(base_dir: &Path, account: &CodexAccount) -> Result<(), String> {
    write_auth_file_to_dir(base_dir, account)?;
    if let Err(err) = write_codex_keychain_to_dir(base_dir, account) {
        logger::log_warn(&format!(
            "[Codex切号] 写入 keychain 失败，目标目录可能缺少完整登录快照: {}",
            err
        ));
    }
    Ok(())
}

/// 准备账号注入：优先同步本地 auth.json，再在必要时刷新 Token 并写回存储
pub async fn prepare_account_for_injection_from_auth_dir(
    account_id: &str,
    auth_dir: Option<&Path>,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() {
        return Ok(account);
    }

    let mut matched_auth_dir: Option<PathBuf> = None;

    if let Some(dir) = auth_dir {
        if sync_account_from_auth_dir_if_current(&mut account, dir)? {
            matched_auth_dir = Some(dir.to_path_buf());
        }
    }

    if matched_auth_dir.is_none() {
        let default_home = get_codex_home();
        if sync_account_from_auth_dir_if_current(&mut account, &default_home)? {
            matched_auth_dir = Some(default_home);
        }
    }

    if codex_oauth::is_token_expired(&account.tokens.access_token) {
        logger::log_info(&format!("账号 {} 的 Token 已过期，尝试刷新", account.email));
        if let Some(ref refresh_token) = account.tokens.refresh_token {
            match codex_oauth::refresh_access_token_with_fallback(
                refresh_token,
                Some(account.tokens.id_token.as_str()),
            )
            .await
            {
                Ok(new_tokens) => {
                    logger::log_info(&format!("账号 {} 的 Token 刷新成功", account.email));
                    account.tokens = new_tokens;
                    save_account(&account)?;
                    if let Some(dir) = matched_auth_dir.as_deref() {
                        if let Err(err) = write_account_bundle_to_dir(dir, &account) {
                            logger::log_warn(&format!(
                                "Codex 账号刷新后回写本地 auth.json 失败: account_id={}, target_dir={}, error={}",
                                account.id,
                                dir.display(),
                                err
                            ));
                        }
                    }
                }
                Err(e) => {
                    logger::log_error(&format!("账号 {} Token 刷新失败: {}", account.email, e));
                    return Err(format!("Token 已过期且刷新失败: {}", e));
                }
            }
        } else {
            return Err("Token 已过期且无 refresh_token，请重新登录".to_string());
        }
    }
    Ok(account)
}

pub async fn prepare_account_for_injection(account_id: &str) -> Result<CodexAccount, String> {
    prepare_account_for_injection_from_auth_dir(account_id, None).await
}

/// 切换账号（写入 auth.json）
pub fn switch_account(account_id: &str) -> Result<CodexAccount, String> {
    let account = load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let codex_home = get_codex_home();
    let auth_path = codex_home.join("auth.json");
    logger::log_info(&format!(
        "[Codex切号] 开始切换账号: account_id={}, email={}, target_dir={}",
        account.id,
        account.email,
        codex_home.display()
    ));
    write_account_bundle_to_dir(&codex_home, &account)?;
    logger::log_info(&format!(
        "[Codex切号] 已替换目录登录信息: target_dir={}, target_file={}",
        codex_home.display(),
        auth_path.display()
    ));

    // 更新索引中的 current_account_id
    let mut index = load_account_index();
    index.current_account_id = Some(account_id.to_string());
    save_account_index(&index)?;

    // 更新账号的 last_used
    let mut updated_account = account.clone();
    updated_account.update_last_used();
    save_account(&updated_account)?;

    logger::log_info(&format!("已切换到 Codex 账号: {}", account.email));

    Ok(updated_account)
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
        );
    }

    if let Some(tokens) = auth_file.tokens {
        let account_id_hint = tokens.account_id.clone();
        let tokens = CodexTokens {
            id_token: tokens.id_token,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
        };
        return upsert_account_with_hints(tokens, account_id_hint, None);
    }

    if let Some(api_key) = fallback_api_key {
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
        );
    }

    Err("auth.json 缺少可导入的账号信息".to_string())
}

fn import_account_struct(account: CodexAccount) -> Result<CodexAccount, String> {
    if account.is_api_key_auth() || account.openai_api_key.is_some() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        return upsert_api_key_account(
            api_key,
            account.api_base_url.clone(),
            Some(account.api_provider_mode),
            account.api_provider_id.clone(),
            account.api_provider_name.clone(),
        );
    }

    upsert_account(account.tokens)
}

/// 从 JSON 字符串导入账号
pub fn import_from_json(json_content: &str) -> Result<Vec<CodexAccount>, String> {
    ensure_storage_writable_for_import()?;

    // 尝试解析为 auth.json 格式
    if let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(json_content) {
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = infer_api_provider_config(
            extract_api_base_url_from_auth_file(&auth_file).as_deref(),
            None,
            None,
            None,
        );
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            return Ok(vec![upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
            )?]);
        }

        if let Some(tokens) = auth_file.tokens {
            let account_id_hint = tokens.account_id.clone();
            let tokens = CodexTokens {
                id_token: tokens.id_token,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
            };
            let account = upsert_account_with_hints(tokens, account_id_hint, None)?;
            return Ok(vec![account]);
        }

        if let Some(api_key) = fallback_api_key {
            return Ok(vec![upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
            )?]);
        }
    }

    // 尝试解析为单账号（顶层 token）或通用数组（支持混合对象）
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_content) {
        match parsed {
            serde_json::Value::Object(_) => {
                if is_auth_mode_apikey(
                    parsed
                        .get("auth_mode")
                        .and_then(|value| value.as_str())
                        .or_else(|| parsed.get("authMode").and_then(|value| value.as_str())),
                ) {
                    if let Some(api_key) = parsed
                        .get("OPENAI_API_KEY")
                        .and_then(|value| value.as_str())
                        .and_then(normalize_api_key)
                    {
                        return Ok(vec![upsert_api_key_account(
                            api_key,
                            extract_api_base_url_from_json_value(&parsed),
                            None,
                            parsed
                                .get("api_provider_id")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                            parsed
                                .get("api_provider_name")
                                .and_then(|value| value.as_str())
                                .map(|value| value.to_string()),
                        )?]);
                    }
                }

                if let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(&parsed) {
                    let account = upsert_account_with_hints(tokens, account_id_hint, None)?;
                    return Ok(vec![account]);
                }

                if let Ok(account) = serde_json::from_value::<CodexAccount>(parsed) {
                    let imported = import_account_struct(account)?;
                    return Ok(vec![imported]);
                }
            }
            serde_json::Value::Array(items) => {
                let mut result = Vec::new();

                for item in items {
                    if let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(&item)
                    {
                        result.push(upsert_account_with_hints(tokens, account_id_hint, None)?);
                        continue;
                    }

                    if is_auth_mode_apikey(
                        item.get("auth_mode")
                            .and_then(|value| value.as_str())
                            .or_else(|| item.get("authMode").and_then(|value| value.as_str())),
                    ) {
                        if let Some(api_key) = item
                            .get("OPENAI_API_KEY")
                            .and_then(|value| value.as_str())
                            .and_then(normalize_api_key)
                        {
                            result.push(upsert_api_key_account(
                                api_key,
                                extract_api_base_url_from_json_value(&item),
                                None,
                                item.get("api_provider_id")
                                    .and_then(|value| value.as_str())
                                    .map(|value| value.to_string()),
                                item.get("api_provider_name")
                                    .and_then(|value| value.as_str())
                                    .map(|value| value.to_string()),
                            )?);
                            continue;
                        }
                    }

                    if let Ok(account) = serde_json::from_value::<CodexAccount>(item) {
                        result.push(import_account_struct(account)?);
                    }
                }

                if !result.is_empty() {
                    return Ok(result);
                }
            }
            _ => {}
        }
    }

    // 尝试解析为账号数组
    if let Ok(accounts) = serde_json::from_str::<Vec<CodexAccount>>(json_content) {
        let mut result = Vec::new();
        for acc in accounts {
            let imported = import_account_struct(acc)?;
            result.push(imported);
        }
        return Ok(result);
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

/// 从单个 JSON 值中提取 CodexTokens
fn extract_codex_tokens_from_value(
    obj: &serde_json::Value,
) -> Option<(CodexTokens, Option<String>)> {
    let obj = obj.as_object()?;

    // 格式1: 顶层 access_token + id_token（用户导出格式）
    if let (Some(id_token), Some(access_token)) = (
        obj.get("id_token").and_then(|v| v.as_str()),
        obj.get("access_token").and_then(|v| v.as_str()),
    ) {
        let refresh_token = obj
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let account_id_hint = obj
            .get("account_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Some((
            CodexTokens {
                id_token: id_token.to_string(),
                access_token: access_token.to_string(),
                refresh_token,
            },
            account_id_hint,
        ));
    }

    // 格式2: 嵌套 tokens 对象（CodexAuthFile 或 CodexAccount 格式）
    if let Some(tokens_obj) = obj.get("tokens").and_then(|v| v.as_object()) {
        if let (Some(id_token), Some(access_token)) = (
            tokens_obj.get("id_token").and_then(|v| v.as_str()),
            tokens_obj.get("access_token").and_then(|v| v.as_str()),
        ) {
            let refresh_token = tokens_obj
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let account_id_hint = tokens_obj
                .get("account_id")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("account_id").and_then(|v| v.as_str()))
                .map(|s| s.to_string());
            return Some((
                CodexTokens {
                    id_token: id_token.to_string(),
                    access_token: access_token.to_string(),
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
        build_account_storage_id, extract_codex_tokens_from_value, get_accounts_dir,
        get_accounts_storage_path, get_current_account, list_accounts_checked, load_account,
        load_account_index, read_api_provider_from_config_toml, read_quick_config_from_config_toml,
        resolve_api_provider_config, save_account, save_account_index, sync_account_from_auth_dir,
        validate_api_key_credentials, write_api_provider_to_config_toml,
        write_quick_config_to_config_toml, ApiProviderConfig, CodexAccountIndex,
        CodexAccountSummary, CodexAuthFile, CodexAuthTokens, CODEX_AUTO_COMPACT_DEFAULT_LIMIT,
        CODEX_CONTEXT_WINDOW_1M_VALUE,
    };
    use crate::models::codex::{CodexAccount, CodexApiProviderMode, CodexTokens};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::fs;
    use std::sync::{LazyLock, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
    }

    impl TestEnvGuard {
        fn new(prefix: &str) -> Self {
            let home_dir = make_temp_dir(prefix);
            let codex_home = home_dir.join(".codex");
            fs::create_dir_all(&codex_home).expect("create codex home");

            let previous_home = std::env::var("HOME").ok();
            let previous_codex_home = std::env::var("CODEX_HOME").ok();
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("CODEX_HOME", &codex_home);

            Self {
                home_dir,
                previous_home,
                previous_codex_home,
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
            let _ = fs::remove_dir_all(&self.home_dir);
        }
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

    fn seed_oauth_account(tokens: CodexTokens) -> CodexAccount {
        let email = "demo@example.com";
        let account_id = "acc-current";
        let organization_id = "org-current";
        let storage_id = build_account_storage_id(email, Some(account_id), Some(organization_id));

        let mut account = CodexAccount::new(storage_id.clone(), email.to_string(), tokens);
        account.user_id = Some("user-current".to_string());
        account.plan_type = Some("pro".to_string());
        account.account_id = Some(account_id.to_string());
        account.organization_id = Some(organization_id.to_string());
        save_account(&account).expect("save account");

        let mut index = CodexAccountIndex::new();
        index.accounts.push(CodexAccountSummary {
            id: storage_id,
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
        index.current_account_id = Some(account.id.clone());
        save_account_index(&index).expect("save index");

        account
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
    fn current_account_syncs_latest_tokens_from_local_auth_json() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let env = TestEnvGuard::new("codex-current-account-sync-test");

        let stored = seed_oauth_account(make_codex_tokens(
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
        write_oauth_auth_file(&env.codex_home(), &latest_tokens, "acc-current");

        let current = get_current_account().expect("current account");
        assert_eq!(current.id, stored.id);
        assert_eq!(current.tokens.access_token, latest_tokens.access_token);
        assert_eq!(
            current.tokens.refresh_token.as_deref(),
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
    fn sync_account_from_auth_dir_updates_store_for_managed_home() {
        let _lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
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
        assert!(!content.contains("model_provider ="));
        assert!(!content
            .lines()
            .any(|line| line.trim_start().starts_with("base_url =")));
        assert_eq!(
            read_api_provider_from_config_toml(&base_dir)
                .base_url
                .as_deref(),
            Some("https://api.example.com")
        );
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
    fn config_toml_skips_openai_base_url_for_default_official_endpoint() {
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
        if config_path.exists() {
            let content = fs::read_to_string(&config_path).expect("read config");
            assert!(!content.contains("openai_base_url"));
        }
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
        assert!(content.contains("name = \"Relay\""));
        assert!(content.contains("base_url = \"https://relay.example.com/v1\""));
        assert!(content.contains("wire_api = \"responses\""));
        assert!(content.contains("requires_openai_auth = true"));
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

        let result = write_quick_config_to_config_toml(&base_dir, true, Some(880000))
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
            write_quick_config_to_config_toml(&base_dir, false, None).expect("save quick config");

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
}

/// 从本地文件导入 Codex 账号（支持多种 JSON 格式）
pub fn import_from_files(file_paths: Vec<String>) -> Result<CodexFileImportResult, String> {
    use std::path::Path;

    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;

    logger::log_info(&format!(
        "Codex: 开始从 {} 个文件导入账号...",
        file_paths.len()
    ));

    // 收集所有候选: (CodexTokens, account_id_hint, label)
    let mut candidates: Vec<(CodexTokens, Option<String>, String)> = Vec::new();

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

        let parsed: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                logger::log_error(&format!("解析 JSON 失败 {:?}: {}", file_path, e));
                continue;
            }
        };

        match &parsed {
            serde_json::Value::Object(_) => {
                if let Some((tokens, hint)) = extract_codex_tokens_from_value(&parsed) {
                    candidates.push((tokens, hint, filename_label));
                } else {
                    logger::log_error(&format!("未找到有效 Token {:?}", file_path));
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
                        candidates.push((tokens, hint, label));
                    }
                }
            }
            _ => {
                logger::log_error(&format!("不支持的 JSON 格式 {:?}", file_path));
            }
        }
    }

    if candidates.is_empty() {
        return Err("未找到有效的 Codex Token（需要 id_token 和 access_token）".to_string());
    }

    logger::log_info(&format!(
        "Codex: 发现 {} 个候选账号，开始导入...",
        candidates.len()
    ));

    let mut imported = Vec::new();
    let mut failed: Vec<CodexFileImportFailure> = Vec::new();
    let total = candidates.len();

    for (index, (tokens, account_id_hint, label)) in candidates.into_iter().enumerate() {
        // 发送进度事件
        if let Some(app_handle) = crate::get_app_handle() {
            use tauri::Emitter;
            let _ = app_handle.emit(
                "codex:file-import-progress",
                serde_json::json!({
                    "current": index + 1,
                    "total": total,
                    "email": &label,
                }),
            );
        }

        match upsert_account_with_hints(tokens, account_id_hint, None) {
            Ok(account) => {
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

pub fn update_api_key_credentials(
    account_id: &str,
    api_key: String,
    api_base_url: Option<String>,
    api_provider_mode: Option<CodexApiProviderMode>,
    api_provider_id: Option<String>,
    api_provider_name: Option<String>,
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

    apply_api_key_fields(&mut account, &normalized_key, provider_config);
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
        write_auth_file_to_dir(&codex_home, &account)?;
        if let Err(err) = write_codex_keychain_to_dir(&codex_home, &account) {
            logger::log_warn(&format!(
                "Codex API Key 账号编辑后写入 keychain 失败: {}",
                err
            ));
        }
    }

    logger::log_info(&format!(
        "Codex API Key 账号凭据已更新: old_id={}, new_id={}, has_base_url={}",
        old_id,
        account.id,
        normalize_optional_ref(account.api_base_url.as_deref()).is_some()
    ));

    Ok(account)
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
    if let Some(account) = get_current_account() {
        return Some(account.id);
    }

    if let Ok(settings) = crate::modules::codex_instance::load_default_settings() {
        if let Some(bind_id) = settings.bind_account_id {
            let trimmed = bind_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    accounts
        .iter()
        .max_by_key(|account| account.last_used)
        .map(|account| account.id.clone())
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
