use serde::{Deserialize, Serialize};

/// Codex 认证模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexAuthMode {
    OAuth,
    Apikey,
}

impl Default for CodexAuthMode {
    fn default() -> Self {
        Self::OAuth
    }
}

/// Codex API Key 账号的模型提供商模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexApiProviderMode {
    OpenaiBuiltin,
    Custom,
}

impl Default for CodexApiProviderMode {
    fn default() -> Self {
        Self::OpenaiBuiltin
    }
}

/// Codex config.toml 快捷配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexQuickConfig {
    pub context_window_1m: bool,
    pub auto_compact_token_limit: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_model_context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_auto_compact_token_limit: Option<i64>,
}

/// Codex 账号数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccount {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub auth_mode: CodexAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub api_provider_mode: CodexApiProviderMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_name: Option<String>,
    pub user_id: Option<String>,
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_structure: Option<String>,
    pub tokens: CodexTokens,
    pub quota: Option<CodexQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<CodexQuotaErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

/// Codex Token 数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokens {
    pub id_token: String,
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// Codex 配额数据（5小时配额 + 周配额）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexQuota {
    /// 5小时配额百分比 (0-100)
    pub hourly_percentage: i32,
    /// 5小时配额重置时间 (Unix timestamp)
    pub hourly_reset_time: Option<i64>,
    /// 主窗口时长（分钟）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hourly_window_minutes: Option<i64>,
    /// 主窗口是否存在（接口返回）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hourly_window_present: Option<bool>,
    /// 周配额百分比 (0-100)
    pub weekly_percentage: i32,
    /// 周配额重置时间 (Unix timestamp)
    pub weekly_reset_time: Option<i64>,
    /// 次窗口时长（分钟）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_window_minutes: Option<i64>,
    /// 次窗口是否存在（接口返回）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly_window_present: Option<bool>,
    /// 原始响应数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_data: Option<serde_json::Value>,
}

/// Codex 配额错误信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexQuotaErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    pub timestamp: i64,
}

/// ~/.codex/auth.json 文件格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<serde_json::Value>, // 可以是 null 或字符串
    #[serde(
        default,
        alias = "api_base_url",
        alias = "apiBaseUrl",
        skip_serializing_if = "Option::is_none"
    )]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<CodexAuthTokens>,
    #[serde(default)]
    pub last_refresh: Option<serde_json::Value>, // 可以是字符串或数字
}

/// auth.json 中的 tokens 字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthTokens {
    pub id_token: String,
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Codex 账号索引（存储多账号）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccountIndex {
    pub version: String,
    pub accounts: Vec<CodexAccountSummary>,
    pub current_account_id: Option<String>,
}

/// 账号摘要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccountSummary {
    pub id: String,
    pub email: String,
    pub plan_type: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl CodexAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
            current_account_id: None,
        }
    }
}

impl Default for CodexAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// JWT Payload 中的用户信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexJwtPayload {
    pub aud: serde_json::Value, // 可能是 string 或 array
    pub iss: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub sub: Option<String>,
    #[serde(rename = "https://api.openai.com/auth")]
    pub auth_data: Option<CodexAuthData>,
}

/// JWT 中的 auth 数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthData {
    pub chatgpt_user_id: Option<String>,
    pub chatgpt_plan_type: Option<String>,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
}

impl CodexAccount {
    pub fn new(id: String, email: String, tokens: CodexTokens) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id,
            email,
            auth_mode: CodexAuthMode::OAuth,
            openai_api_key: None,
            api_base_url: None,
            api_provider_mode: CodexApiProviderMode::OpenaiBuiltin,
            api_provider_id: None,
            api_provider_name: None,
            user_id: None,
            plan_type: None,
            account_id: None,
            organization_id: None,
            account_name: None,
            account_structure: None,
            tokens,
            quota: None,
            quota_error: None,
            usage_updated_at: None,
            tags: None,
            created_at: now,
            last_used: now,
        }
    }

    pub fn new_api_key(
        id: String,
        email: String,
        openai_api_key: String,
        api_provider_mode: CodexApiProviderMode,
        api_base_url: Option<String>,
        api_provider_id: Option<String>,
        api_provider_name: Option<String>,
    ) -> Self {
        let mut account = Self::new(
            id,
            email,
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.auth_mode = CodexAuthMode::Apikey;
        account.openai_api_key = Some(openai_api_key);
        account.api_provider_mode = api_provider_mode;
        account.api_base_url = api_base_url;
        account.api_provider_id = api_provider_id;
        account.api_provider_name = api_provider_name;
        account.plan_type = Some("API_KEY".to_string());
        account
    }

    pub fn is_api_key_auth(&self) -> bool {
        self.auth_mode == CodexAuthMode::Apikey
    }

    pub fn update_last_used(&mut self) {
        self.last_used = chrono::Utc::now().timestamp();
    }
}
