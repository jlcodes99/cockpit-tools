use crate::models::{CreditInfo, QuotaData, TokenData};
use crate::modules;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

const CLOUD_CODE_DAILY_BASE_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
const CLOUD_CODE_PROD_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
const CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL: &str =
    "https://autopush-cloudcode-pa.sandbox.googleapis.com";
const LOAD_CODE_ASSIST_PATH: &str = "v1internal:loadCodeAssist";
const ONBOARD_USER_PATH: &str = "v1internal:onboardUser";
const FETCH_AVAILABLE_MODELS_PATH: &str = "v1internal:fetchAvailableModels";
const DEFAULT_ATTEMPTS: usize = 2;
const BACKOFF_BASE_MS: u64 = 500;
const BACKOFF_MAX_MS: u64 = 4000;
const ONBOARD_POLL_DELAY_MS: u64 = 500;
const API_CACHE_DIR: &str = "cache/quota_api_v1_desktop";
const API_CACHE_VERSION: u8 = 1;
const API_CACHE_TTL_MS: i64 = 60_000;
const DEFAULT_CLOUD_CODE_IDE_VERSION: &str = "1.20.5";
const DEFAULT_LOAD_CODE_ASSIST_UA_OS: &str = "windows";
const DEFAULT_LOAD_CODE_ASSIST_UA_ARCH: &str = "amd64";
const DEFAULT_GOOGLE_API_NODEJS_CLIENT_VERSION: &str = "10.3.0";
const DEFAULT_CLOUD_CODE_USER_AGENT: &str = "antigravity/1.20.5 windows/amd64";
const DEFAULT_LOAD_CODE_ASSIST_USER_AGENT: &str =
    "antigravity/1.20.5 windows/amd64 google-api-nodejs-client/10.3.0";
const DEFAULT_X_GOOG_API_CLIENT_NODE_VERSION: &str = "22.21.1";

#[derive(Debug, Clone, Default)]
pub struct QuotaCloudCodeContext {
    pub preferred_project_id: Option<String>,
    pub is_gcp_tos: bool,
}

impl QuotaCloudCodeContext {
    pub fn from_token(token: &TokenData) -> Self {
        Self {
            preferred_project_id: token.project_id.clone(),
            is_gcp_tos: token.is_gcp_tos.unwrap_or(false),
        }
    }
}

fn env_var_trimmed(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_bool(name: &str) -> bool {
    match env_var_trimmed(name)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        _ => false,
    }
}

fn env_quality_is_insider_or_dev() -> bool {
    matches!(
        env_var_trimmed("ANTIGRAVITY_APP_QUALITY")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "insider" | "dev"
    )
}

fn official_antigravity_version_for_cloud_code() -> String {
    let version = crate::modules::wakeup_gateway::official_antigravity_app_version();
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return DEFAULT_CLOUD_CODE_IDE_VERSION.to_string();
    }
    trimmed.to_string()
}

fn load_code_assist_user_agent_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        _ => DEFAULT_LOAD_CODE_ASSIST_UA_OS,
    }
}

fn load_code_assist_user_agent_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => DEFAULT_LOAD_CODE_ASSIST_UA_ARCH,
    }
}

fn read_json_version_field(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn google_api_nodejs_client_version_for_load_code_assist() -> String {
    if let Some(root) = crate::modules::wakeup_gateway::official_antigravity_root_for_version() {
        let candidates = [
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("node_modules")
                .join("google-auth-library")
                .join("package.json"),
            root.join("resources")
                .join("app")
                .join("node_modules")
                .join("google-auth-library")
                .join("package.json"),
            root.join("node_modules")
                .join("google-auth-library")
                .join("package.json"),
        ];

        for candidate in candidates {
            if let Some(version) = read_json_version_field(&candidate) {
                return version;
            }
        }
    }

    DEFAULT_GOOGLE_API_NODEJS_CLIENT_VERSION.to_string()
}

fn build_load_code_assist_user_agent() -> String {
    let ide_version = official_antigravity_version_for_cloud_code();
    let os = load_code_assist_user_agent_os();
    let arch = load_code_assist_user_agent_arch();
    let google_client_version = google_api_nodejs_client_version_for_load_code_assist();
    if ide_version.trim().is_empty()
        || os.trim().is_empty()
        || arch.trim().is_empty()
        || google_client_version.trim().is_empty()
    {
        return DEFAULT_LOAD_CODE_ASSIST_USER_AGENT.to_string();
    }

    format!(
        "antigravity/{} {}/{} google-api-nodejs-client/{}",
        ide_version, os, arch, google_client_version
    )
}

fn build_cloud_code_user_agent() -> String {
    let ide_version = official_antigravity_version_for_cloud_code();
    let os = load_code_assist_user_agent_os();
    let arch = load_code_assist_user_agent_arch();
    if ide_version.trim().is_empty() || os.trim().is_empty() || arch.trim().is_empty() {
        return DEFAULT_CLOUD_CODE_USER_AGENT.to_string();
    }
    format!("antigravity/{} {}/{}", ide_version, os, arch)
}

fn load_code_assist_user_agent() -> String {
    let ua = build_load_code_assist_user_agent();
    if ua.trim().is_empty() {
        return DEFAULT_LOAD_CODE_ASSIST_USER_AGENT.to_string();
    }
    ua
}

fn load_code_assist_x_goog_api_client() -> String {
    if let Ok(raw) = std::env::var("AG_LOAD_CODE_ASSIST_NODE_VERSION") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return format!("gl-node/{}", trimmed);
        }
    }
    format!("gl-node/{}", DEFAULT_X_GOOG_API_CLIENT_NODE_VERSION)
}

fn cloud_code_platform_name() -> &'static str {
    match (
        load_code_assist_user_agent_os(),
        load_code_assist_user_agent_arch(),
    ) {
        ("darwin", "amd64") => "DARWIN_AMD64",
        ("darwin", "arm64") => "DARWIN_ARM64",
        ("linux", "amd64") => "LINUX_AMD64",
        ("linux", "arm64") => "LINUX_ARM64",
        ("windows", "amd64") => "WINDOWS_AMD64",
        _ => "PLATFORM_UNSPECIFIED",
    }
}

fn cloud_code_plugin_version() -> String {
    let version = env!("CARGO_PKG_VERSION").trim();
    if version.is_empty() {
        return "unknown".to_string();
    }
    version.to_string()
}

fn build_cloud_code_metadata(duet_project: Option<&str>) -> Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "ideName".to_string(),
        Value::String("antigravity".to_string()),
    );
    metadata.insert(
        "ideType".to_string(),
        Value::String("ANTIGRAVITY".to_string()),
    );
    metadata.insert(
        "ideVersion".to_string(),
        Value::String(official_antigravity_version_for_cloud_code()),
    );
    metadata.insert(
        "pluginVersion".to_string(),
        Value::String(cloud_code_plugin_version()),
    );
    metadata.insert(
        "platform".to_string(),
        Value::String(cloud_code_platform_name().to_string()),
    );
    metadata.insert(
        "updateChannel".to_string(),
        Value::String("stable".to_string()),
    );
    metadata.insert(
        "pluginType".to_string(),
        Value::String("GEMINI".to_string()),
    );
    if let Some(project) = duet_project
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "duetProject".to_string(),
            Value::String(project.to_string()),
        );
    }
    Value::Object(metadata)
}

fn resolve_cloud_code_base_url(ctx: &QuotaCloudCodeContext) -> String {
    // 与 Antigravity.app 的 IYs(...) 选择顺序保持一致：override > gcpTos > internal(insider/dev) > daily
    if let Some(override_url) = env_var_trimmed("ANTIGRAVITY_CLOUD_CODE_URL_OVERRIDE") {
        return override_url;
    }

    if ctx.is_gcp_tos {
        return CLOUD_CODE_PROD_BASE_URL.to_string();
    }

    if env_bool("ANTIGRAVITY_IS_GOOGLE_INTERNAL") && env_quality_is_insider_or_dev() {
        return CLOUD_CODE_AUTOPUSH_SANDBOX_BASE_URL.to_string();
    }

    CLOUD_CODE_DAILY_BASE_URL.to_string()
}

fn truncate_log_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    let mut preview = text.chars().take(max_len).collect::<String>();
    preview.push_str("...");
    preview
}

fn header_value(headers: &reqwest::header::HeaderMap, name: reqwest::header::HeaderName) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_string()
}

fn log_subscription_tier_result(email: &str, subscription_tier: Option<&String>, reason: &str) {
    if let Some(tier) = subscription_tier {
        crate::modules::logger::log_info(&format!("📊 [{}] 订阅识别成功: {}", email, tier));
    } else {
        crate::modules::logger::log_warn(&format!(
            "⚠️ [{}] 订阅识别失败: UNKNOWN ({})",
            email, reason
        ));
    }
}

fn hash_email(email: &str) -> String {
    let normalized = email.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn api_cache_path(source: &str, email: &str) -> Result<PathBuf, String> {
    let data_dir = modules::account::get_data_dir()?;
    let dir = data_dir.join(API_CACHE_DIR).join(source);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create quota api cache dir: {}", e))?;
    }
    Ok(dir.join(format!("{}.json", hash_email(email))))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuotaApiCacheRecord {
    version: u8,
    source: String,
    custom_source: String,
    email: String,
    project_id: Option<String>,
    updated_at: i64,
    payload: serde_json::Value,
}

fn read_api_cache(source: &str, email: &str) -> Option<QuotaApiCacheRecord> {
    let path = api_cache_path(source, email).ok()?;
    let content = fs::read_to_string(path).ok()?;
    let record = serde_json::from_str::<QuotaApiCacheRecord>(&content).ok()?;
    if record.version != API_CACHE_VERSION {
        return None;
    }
    if record.source != source {
        return None;
    }
    Some(record)
}

fn is_api_cache_valid(record: &QuotaApiCacheRecord) -> bool {
    let now_ms = Utc::now().timestamp_millis();
    now_ms - record.updated_at < API_CACHE_TTL_MS
}

fn api_cache_age_secs(record: &QuotaApiCacheRecord) -> i64 {
    let now_ms = Utc::now().timestamp_millis();
    std::cmp::max(0, (now_ms - record.updated_at) / 1000)
}

fn write_api_cache(
    source: &str,
    custom_source: &str,
    email: &str,
    project_id: Option<String>,
    payload: serde_json::Value,
) {
    if let Ok(path) = api_cache_path(source, email) {
        let record = QuotaApiCacheRecord {
            version: API_CACHE_VERSION,
            source: source.to_string(),
            custom_source: custom_source.to_string(),
            email: email.to_string(),
            project_id,
            updated_at: Utc::now().timestamp_millis(),
            payload,
        };
        if let Ok(content) = serde_json::to_string_pretty(&record) {
            let _ = fs::write(path, content);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct QuotaResponse {
    models: std::collections::HashMap<String, ModelInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelInfo {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "quotaInfo")]
    quota_info: Option<QuotaInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QuotaInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuotaFetchError {
    pub code: Option<u16>,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct QuotaFetchResult {
    pub quota: QuotaData,
    pub error: Option<QuotaFetchError>,
}

#[derive(Debug, Deserialize)]
struct LoadProjectResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<serde_json::Value>,
    #[serde(rename = "currentTier")]
    current_tier: Option<Tier>,
    #[serde(rename = "paidTier")]
    paid_tier: Option<Tier>,
    #[serde(rename = "allowedTiers")]
    allowed_tiers: Option<Vec<AllowedTier>>,
}

#[derive(Debug, Deserialize)]
struct AllowedTier {
    id: Option<String>,
    #[serde(rename = "isDefault")]
    is_default: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Tier {
    id: Option<String>,
    #[serde(rename = "availableCredits", default)]
    available_credits: Option<Vec<AvailableCreditRaw>>,
}

#[derive(Debug, Deserialize)]
struct AvailableCreditRaw {
    #[serde(rename = "creditType")]
    credit_type: Option<String>,
    #[serde(rename = "creditAmount")]
    credit_amount: Option<String>,
    #[serde(rename = "minimumCreditAmountForUsage")]
    minimum_credit_amount_for_usage: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OnboardUserResponse {
    name: Option<String>,
    done: Option<bool>,
    response: Option<OnboardResponse>,
}

#[derive(Debug, Deserialize)]
struct OnboardResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project: Option<serde_json::Value>,
}

fn create_client() -> reqwest::Client {
    crate::utils::http::create_client(15)
}

fn build_load_code_assist_payload(project_id: Option<&str>) -> serde_json::Value {
    let mut payload = json!({
        "metadata": build_cloud_code_metadata(project_id),
        "mode": "FULL_ELIGIBILITY_CHECK"
    });
    if let Some(project_id) = project_id.filter(|id| !id.trim().is_empty()) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "cloudaicompanionProject".to_string(),
                serde_json::Value::String(project_id.to_string()),
            );
        }
    }
    payload
}

fn extract_project_id(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        if !text.is_empty() {
            return Some(text.to_string());
        }
    }
    if let Some(obj) = value.as_object() {
        if let Some(id_value) = obj.get("id") {
            if let Some(id) = id_value.as_str() {
                if !id.is_empty() {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

fn pick_onboard_tier(allowed: &[AllowedTier]) -> Option<String> {
    if let Some(default) = allowed.iter().find(|tier| tier.is_default.unwrap_or(false)) {
        if let Some(id) = default.id.clone() {
            return Some(id);
        }
    }
    if let Some(first) = allowed.iter().find(|tier| tier.id.is_some()) {
        return first.id.clone();
    }
    if !allowed.is_empty() {
        return Some("LEGACY".to_string());
    }
    None
}

fn get_backoff_delay_ms(attempt: usize) -> u64 {
    if attempt < 2 {
        return 0;
    }
    let raw = BACKOFF_BASE_MS.saturating_mul(2u64.saturating_pow((attempt - 2) as u32));
    let jitter = rand::random::<u64>() % 100;
    std::cmp::min(raw + jitter, BACKOFF_MAX_MS)
}

async fn try_onboard_user(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    tier_id: &str,
    project_id: Option<&str>,
) -> Result<Option<String>, String> {
    let mut payload = json!({
        "tierId": tier_id,
        "metadata": build_cloud_code_metadata(project_id)
    });
    let ua = load_code_assist_user_agent();
    if let Some(project_id) = project_id.filter(|id| !id.trim().is_empty()) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "cloudaicompanionProject".to_string(),
                serde_json::Value::String(project_id.to_string()),
            );
        }
    }

    let response = client
        .post(format!("{}/{}", base_url, ONBOARD_USER_PATH))
        .bearer_auth(access_token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, &ua)
        .header(reqwest::header::ACCEPT_ENCODING, "gzip")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("onboardUser 网络错误: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("onboardUser 失败: {} - {}", status, text));
    }

    let mut data = response
        .json::<OnboardUserResponse>()
        .await
        .map_err(|e| format!("onboardUser 解析失败: {}", e))?;

    loop {
        if data.done.unwrap_or(false) {
            if let Some(project) = data.response.and_then(|resp| resp.project) {
                return Ok(extract_project_id(&project));
            }
            return Ok(None);
        }

        let op_name = data
            .name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "onboardUser 未完成但缺少 operation name".to_string())?;

        let poll_response = client
            .get(format!("{}/v1internal/{}", base_url, op_name))
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::USER_AGENT, &ua)
            .header(reqwest::header::ACCEPT_ENCODING, "gzip")
            .send()
            .await
            .map_err(|e| format!("onboardUser 轮询网络错误: {}", e))?;

        if !poll_response.status().is_success() {
            let status = poll_response.status();
            let text = poll_response.text().await.unwrap_or_default();
            return Err(format!("onboardUser 轮询失败: {} - {}", status, text));
        }

        data = poll_response
            .json::<OnboardUserResponse>()
            .await
            .map_err(|e| format!("onboardUser 轮询解析失败: {}", e))?;

        tokio::time::sleep(std::time::Duration::from_millis(ONBOARD_POLL_DELAY_MS)).await;
    }
}

/// 从 paidTier.availableCredits 提取有效积分信息
fn extract_credits_from_tier(tier: &Tier) -> Vec<CreditInfo> {
    tier.available_credits
        .as_ref()
        .map(|credits| {
            credits
                .iter()
                .filter_map(|raw| {
                    let credit_type = raw.credit_type.as_ref()?.clone();
                    // 仅保留有 creditAmount 的条目
                    if raw.credit_amount.is_none() {
                        return None;
                    }
                    Some(CreditInfo {
                        credit_type,
                        credit_amount: raw.credit_amount.clone(),
                        minimum_credit_amount_for_usage: raw
                            .minimum_credit_amount_for_usage
                            .clone(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 获取项目 ID、订阅类型和积分信息（优先使用 token 中的 project_id / is_gcp_tos 上下文）
pub async fn fetch_project_id_for_token(
    token: &TokenData,
    email: &str,
) -> (Option<String>, Option<String>, Vec<CreditInfo>) {
    let ctx = QuotaCloudCodeContext::from_token(token);
    fetch_project_id_with_context(&token.access_token, email, &ctx).await
}

pub async fn fetch_project_id_with_context(
    access_token: &str,
    email: &str,
    ctx: &QuotaCloudCodeContext,
) -> (Option<String>, Option<String>, Vec<CreditInfo>) {
    let client = create_client();
    let mut subscription_tier: Option<String> = None;
    let mut allowed_tiers: Vec<AllowedTier> = Vec::new();
    let mut last_error: Option<String> = None;
    let mut credits: Vec<CreditInfo> = Vec::new();
    let base_url = resolve_cloud_code_base_url(ctx);
    let ua = load_code_assist_user_agent();
    let x_goog_api_client = load_code_assist_x_goog_api_client();
    let preferred_project_id = ctx
        .preferred_project_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    for attempt in 1..=DEFAULT_ATTEMPTS {
        crate::modules::logger::log_info(&format!(
            "[Quota][loadCodeAssist] account={} attempt={}/{} url={}/{} user-agent=\"{}\" x-goog-api-client=\"{}\"",
            email,
            attempt,
            DEFAULT_ATTEMPTS,
            base_url,
            LOAD_CODE_ASSIST_PATH,
            ua,
            x_goog_api_client
        ));

        let response = client
            .post(format!("{}/{}", base_url, LOAD_CODE_ASSIST_PATH))
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::USER_AGENT, &ua)
            .header("x-goog-api-client", &x_goog_api_client)
            .header(reqwest::header::ACCEPT, "*/*")
            .header(reqwest::header::ACCEPT_ENCODING, "gzip, deflate, br")
            .json(&build_load_code_assist_payload(
                preferred_project_id.as_deref(),
            ))
            .send()
            .await;

        match response {
            Ok(res) => {
                let status = res.status();
                let headers = res.headers().clone();
                if status.is_success() {
                    let text_result = res.text().await;
                    match text_result {
                        Ok(text) => {
                            match serde_json::from_str::<LoadProjectResponse>(&text) {
                                Ok(data) => {
                                    let paid_tier_id =
                                        data.paid_tier.as_ref().and_then(|tier| tier.id.clone());
                                    let current_tier_id =
                                        data.current_tier.as_ref().and_then(|tier| tier.id.clone());
                                    subscription_tier =
                                        paid_tier_id.clone().or(current_tier_id.clone());

                                    // 提取积分数据
                                    if let Some(ref paid_tier) = data.paid_tier {
                                        credits = extract_credits_from_tier(paid_tier);
                                        if !credits.is_empty() {
                                            crate::modules::logger::log_info(&format!(
                                                "💰 [{}] 积分数据: {} 条 ({})",
                                                email,
                                                credits.len(),
                                                credits
                                                    .iter()
                                                    .map(|c| format!(
                                                        "{}={}",
                                                        c.credit_type,
                                                        c.credit_amount.as_deref().unwrap_or("-")
                                                    ))
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ));
                                        }
                                    }

                                    if subscription_tier.is_some() {
                                        log_subscription_tier_result(
                                            email,
                                            subscription_tier.as_ref(),
                                            "loadCodeAssist 正常返回",
                                        );
                                    } else {
                                        let allowed_tier_preview = data
                                            .allowed_tiers
                                            .as_ref()
                                            .map(|tiers| {
                                                tiers
                                                    .iter()
                                                    .filter_map(|tier| tier.id.as_deref())
                                                    .collect::<Vec<_>>()
                                                    .join(",")
                                            })
                                            .unwrap_or_else(|| "-".to_string());
                                        let reason = format!(
                                        "loadCodeAssist 成功但无 tier: paidTier={:?}, currentTier={:?}, allowedTiers=[{}], hasProject={}",
                                        paid_tier_id,
                                        current_tier_id,
                                        allowed_tier_preview,
                                        data.project.is_some()
                                    );
                                        log_subscription_tier_result(
                                            email,
                                            subscription_tier.as_ref(),
                                            &reason,
                                        );
                                    }

                                    let response_project_id =
                                        data.project.as_ref().and_then(extract_project_id);
                                    if let Some(project_id) = response_project_id.clone() {
                                        return (Some(project_id), subscription_tier, credits);
                                    }

                                    if let Some(tiers) = data.allowed_tiers {
                                        allowed_tiers = tiers;
                                    }

                                    let onboard_tier = pick_onboard_tier(&allowed_tiers)
                                        .or_else(|| subscription_tier.clone());
                                    if let Some(tier_id) = onboard_tier {
                                        let onboard_project_hint = preferred_project_id
                                            .as_deref()
                                            .or(response_project_id.as_deref());
                                        match try_onboard_user(
                                            &client,
                                            &base_url,
                                            access_token,
                                            &tier_id,
                                            onboard_project_hint,
                                        )
                                        .await
                                        {
                                            Ok(project_id) => {
                                                if let Some(project_id) = project_id {
                                                    return (
                                                        Some(project_id),
                                                        subscription_tier,
                                                        credits,
                                                    );
                                                }
                                            }
                                            Err(err) => {
                                                crate::modules::logger::log_warn(&format!(
                                                    "⚠️ [{}] onboardUser 失败: {}",
                                                    email, err
                                                ));
                                            }
                                        }
                                    }

                                    return (None, subscription_tier, credits);
                                }
                                Err(err) => {
                                    last_error = Some(format!("loadCodeAssist 解析失败: {}", err));
                                    let header_info = format!(
                                    "status={}, content-type={}, content-encoding={}, content-length={}",
                                    status,
                                    header_value(&headers, reqwest::header::CONTENT_TYPE),
                                    header_value(&headers, reqwest::header::CONTENT_ENCODING),
                                    header_value(&headers, reqwest::header::CONTENT_LENGTH)
                                );
                                    crate::modules::logger::log_error(&format!(
                                        "❌ [{}] loadCodeAssist 解析失败: {}, {}",
                                        email, err, header_info
                                    ));
                                    crate::modules::logger::log_error(&format!(
                                        "❌ [{}] loadCodeAssist 原始响应长度: {}",
                                        email,
                                        text.len()
                                    ));
                                }
                            }
                        }
                        Err(err) => {
                            last_error = Some(format!("loadCodeAssist 读取失败: {}", err));
                            let header_info = format!(
                                "status={}, content-type={}, content-encoding={}, content-length={}",
                                status,
                                header_value(&headers, reqwest::header::CONTENT_TYPE),
                                header_value(&headers, reqwest::header::CONTENT_ENCODING),
                                header_value(&headers, reqwest::header::CONTENT_LENGTH)
                            );
                            crate::modules::logger::log_error(&format!(
                                "❌ [{}] loadCodeAssist 响应读取失败: {}, {}",
                                email, err, header_info
                            ));
                        }
                    }
                } else if status == reqwest::StatusCode::UNAUTHORIZED {
                    let text = res.text().await.unwrap_or_default();
                    let reason = format!(
                        "loadCodeAssist 返回 401 Unauthorized, base={}, attempt={}/{}, body_len={}",
                        base_url,
                        attempt,
                        DEFAULT_ATTEMPTS,
                        text.len()
                    );
                    log_subscription_tier_result(email, subscription_tier.as_ref(), &reason);
                    return (None, subscription_tier, credits);
                } else if status == reqwest::StatusCode::FORBIDDEN {
                    let text = res.text().await.unwrap_or_default();
                    let reason = format!(
                        "loadCodeAssist 返回 403 Forbidden, base={}, attempt={}/{}, body_len={}",
                        base_url,
                        attempt,
                        DEFAULT_ATTEMPTS,
                        text.len()
                    );
                    log_subscription_tier_result(email, subscription_tier.as_ref(), &reason);
                    return (None, subscription_tier, credits);
                } else {
                    let text = res.text().await.unwrap_or_default();
                    let retryable =
                        status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.as_u16() >= 500;
                    last_error = Some(format!(
                        "loadCodeAssist 失败: status={}, base={}, attempt={}/{}, body_len={}",
                        status,
                        base_url,
                        attempt,
                        DEFAULT_ATTEMPTS,
                        text.len()
                    ));
                    if retryable && attempt < DEFAULT_ATTEMPTS {
                        let delay = get_backoff_delay_ms(attempt + 1);
                        if delay > 0 {
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                        continue;
                    }
                }
            }
            Err(e) => {
                last_error = Some(format!(
                    "loadCodeAssist 网络错误: base={}, attempt={}/{}, error={}",
                    base_url, attempt, DEFAULT_ATTEMPTS, e
                ));
                if attempt < DEFAULT_ATTEMPTS {
                    let delay = get_backoff_delay_ms(attempt + 1);
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    continue;
                }
            }
        }
    }

    if let Some(err) = last_error {
        crate::modules::logger::log_error(&format!("❌ [{}] loadCodeAssist 失败: {}", email, err));
        log_subscription_tier_result(email, subscription_tier.as_ref(), &err);
    } else {
        log_subscription_tier_result(email, subscription_tier.as_ref(), "未知错误");
    }

    (None, subscription_tier, credits)
}

fn build_quota_data_from_response(
    quota_response: QuotaResponse,
    subscription_tier: Option<String>,
    credits: Vec<CreditInfo>,
) -> QuotaData {
    let mut quota_data = QuotaData::new();

    for (name, info) in quota_response.models {
        let display_name = info
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(quota_info) = info.quota_info {
            let percentage = quota_info
                .remaining_fraction
                .map(|f| (f * 100.0) as i32)
                .unwrap_or(0);
            let reset_time = quota_info.reset_time.unwrap_or_default();
            if name.contains("gemini") || name.contains("claude") {
                quota_data.add_model(name, display_name, percentage, reset_time);
            }
        }
    }

    quota_data.subscription_tier = subscription_tier;
    quota_data.credits = credits;
    quota_data
}

pub async fn fetch_quota_for_token(
    token: &TokenData,
    email: &str,
    skip_cache: bool,
) -> crate::error::AppResult<QuotaFetchResult> {
    let ctx = QuotaCloudCodeContext::from_token(token);
    fetch_quota_with_context(&token.access_token, email, skip_cache, &ctx).await
}

pub async fn fetch_quota_with_context(
    access_token: &str,
    email: &str,
    skip_cache: bool,
    ctx: &QuotaCloudCodeContext,
) -> crate::error::AppResult<QuotaFetchResult> {
    use crate::error::AppError;

    let base_url = resolve_cloud_code_base_url(ctx);
    let (resolved_project_id, subscription_tier, credits) =
        fetch_project_id_with_context(access_token, email, ctx).await;
    let effective_project_id = resolved_project_id
        .clone()
        .or_else(|| ctx.preferred_project_id.clone());

    // 保留缓存，但缓存命中前仍先执行与 Antigravity.app 对齐的项目识别流程。
    if !skip_cache {
        if let Some(record) = read_api_cache("authorized", email) {
            if is_api_cache_valid(&record) {
                crate::modules::logger::log_info(&format!(
                    "[QuotaApiCache] Using api cache for {} (age: {}s)",
                    email,
                    api_cache_age_secs(&record),
                ));
                if let Ok(quota_response) =
                    serde_json::from_value::<QuotaResponse>(record.payload.clone())
                {
                    let quota_data = build_quota_data_from_response(
                        quota_response,
                        subscription_tier.clone(),
                        credits.clone(),
                    );
                    return Ok(QuotaFetchResult {
                        quota: quota_data,
                        error: None,
                    });
                }
            } else {
                crate::modules::logger::log_info(&format!(
                    "[QuotaApiCache] Cache expired for {} (age: {}s), fetching from network",
                    email,
                    api_cache_age_secs(&record),
                ));
            }
        }
    }

    let client = create_client();
    let payload = effective_project_id
        .as_ref()
        .map(|id| json!({ "project": id }))
        .unwrap_or_else(|| json!({}));
    let cloud_code_user_agent = build_cloud_code_user_agent();

    let max_retries = 3;

    for attempt in 1..=max_retries {
        match client
            .post(format!("{}/{}", base_url, FETCH_AVAILABLE_MODELS_PATH))
            .bearer_auth(access_token)
            .header(reqwest::header::USER_AGENT, &cloud_code_user_agent)
            .header(reqwest::header::ACCEPT_ENCODING, "gzip")
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => {
                if response.error_for_status_ref().is_err() {
                    let status = response.status();

                    if status == reqwest::StatusCode::FORBIDDEN {
                        crate::modules::logger::log_warn(&format!(
                            "账号无权限 (403 Forbidden), 标记为 forbidden 状态: {}",
                            email
                        ));
                        let text = response.text().await.unwrap_or_default();
                        let mut q = QuotaData::new();
                        q.is_forbidden = true;
                        q.subscription_tier = subscription_tier.clone();
                        let message = if text.trim().is_empty() {
                            "API returned 403 Forbidden".to_string()
                        } else {
                            text
                        };
                        return Ok(QuotaFetchResult {
                            quota: q,
                            error: Some(QuotaFetchError {
                                code: Some(status.as_u16()),
                                message,
                            }),
                        });
                    }

                    if attempt < max_retries {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        continue;
                    }

                    let text = response.text().await.unwrap_or_default();
                    return Err(AppError::Unknown(format!(
                        "API 错误: {} - {}",
                        status, text
                    )));
                }

                let body = response.text().await.map_err(AppError::Network)?;
                let payload_value: serde_json::Value = serde_json::from_str(&body)
                    .map_err(|e| AppError::Unknown(format!("API 响应解析失败: {}", e)))?;

                write_api_cache(
                    "authorized",
                    "desktop",
                    email,
                    effective_project_id.clone(),
                    payload_value.clone(),
                );

                let quota_response: QuotaResponse = serde_json::from_value(payload_value)
                    .map_err(|e| AppError::Unknown(format!("API 响应解析失败: {}", e)))?;
                let quota_data = build_quota_data_from_response(
                    quota_response,
                    subscription_tier.clone(),
                    credits.clone(),
                );

                return Ok(QuotaFetchResult {
                    quota: quota_data,
                    error: None,
                });
            }
            Err(e) => {
                if attempt < max_retries {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                } else {
                    return Err(AppError::Network(e));
                }
            }
        }
    }

    Err(AppError::Unknown("配额查询失败".to_string()))
}
