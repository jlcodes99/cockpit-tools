use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_token_source_mode() -> String {
    "managed".to_string()
}

fn is_false(value: &bool) -> bool {
    !*value
}

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

/// Cockpit 管理的 Codex 模型目录条目
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexExperimentalModelDefinition {
    pub model_id: String,
    pub display_name: String,
    /// None 表示跟随官方推理强度；Some 表示用户自定义可选推理强度集合。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_efforts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<i64>,
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
    #[serde(default)]
    pub experimental_model_catalog_enabled: bool,
    #[serde(default)]
    pub experimental_model_catalog_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_model_catalog_unavailable_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_model_catalog_conflict: Option<String>,
    #[serde(default)]
    pub experimental_model_catalog_models: Vec<CodexExperimentalModelDefinition>,
    /// 当前可见模型目录中写入 Codex config.toml 的默认模型；None 表示不强制指定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_model_catalog_default_model_id: Option<String>,
    /// 当前版本内置的模型目录，用于在编辑器中恢复 Cockpit 默认配置。
    #[serde(default)]
    pub experimental_model_catalog_reset_models: Vec<CodexExperimentalModelDefinition>,
    /// 恢复 Cockpit 默认配置时使用的默认模型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental_model_catalog_reset_default_model_id: Option<String>,
    /// 官方 Codex 实验性上下文管理开关；缺失时严格按官方默认关闭处理。
    #[serde(default)]
    pub context_management_experimental_mode: bool,
}

/// Codex 官方 App 推理速度
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexAppSpeed {
    /// 兼容已移除的自定义档位，旧账号、实例及速度偏好统一回落标准速度。
    #[serde(alias = "ultrafast")]
    Standard,
    Fast,
}

impl Default for CodexAppSpeed {
    fn default() -> Self {
        Self::Standard
    }
}

/// Codex 官方 App 推理速度配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAppSpeedConfig {
    pub speed: CodexAppSpeed,
    pub global_state_path: String,
}

/// API 服务账号级模型映射：调用方请求的模型 → 发给上游的模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexApiModelMapping {
    pub client_model: String,
    pub upstream_model: String,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_model_catalog: Vec<String>,
    /// 供应商目录里按模型覆盖的 `context_window`。未填写时走官方值或全局兜底。
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub api_model_context_windows: HashMap<String, i64>,
    /// API 服务按账号改写：调用方请求的模型 → 发给上游的模型。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_model_mappings: Vec<CodexApiModelMapping>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub api_sync_model_catalog_to_codex: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_wire_api: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub api_supports_websockets: bool,
    #[serde(default)]
    pub api_supports_vision: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub api_model_vision_support: HashMap<String, bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_vision_routing_model: Option<String>,
    /// DeepSeek Responses: `gateway` lists official shells via instance sidecar; `direct` talks to api.deepseek.com.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_instance_access_mode: Option<String>,
    /// Direct-start model for official DeepSeek Responses (`deepseek-v4-flash` / `deepseek-v4-pro`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_startup_model: Option<String>,
    /// 网关模式下用于生图转发的 GPT(OAuth) 账号池。
    ///
    /// 对话仍由本账号的上游模型处理；生图请求（images 端点与对话内触发的生图）
    /// 交给这里的账号执行，走 `gpt-5.5` 基础模型 + `gpt-image-2.5` 的原有链路。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_image_generation_account_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_oauth_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub bound_oauth_use_local_gateway: bool,
    /// 上游凭据来自 Grok 平台账号（OAuth）的供应商账号。
    ///
    /// 该账号自身不带上游 API Key：本地网关会把绑定的 Grok 平台账号的 OAuth
    /// 访问令牌写成 xai auth 文件交给 sidecar，由 Grok(xAI) 执行器完成上游请求。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_grok_account_id: Option<String>,
    pub user_id: Option<String>,
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_active_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_file_plan_type: Option<String>,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_identity: Option<CodexAgentIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_structure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_note: Option<String>,
    /// 账号级出口代理。留空时继续使用全局/API 服务代理。
    #[serde(default, skip_serializing_if = "Option::is_none", rename(serialize = "egress_proxy"), serialize_with = "crate::modules::codex_account_proxy::serialize_summary")]
    pub egress_proxy_url: Option<String>,
    /// Codex OAuth 设备指纹收敛模式。未设置时按 `off` 处理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    // Legacy import/export metadata only; never projected into runtime credentials.
    pub codex_fingerprint_mode: Option<String>,
    /// 历史备份元数据；不再限制客户端或下发到 sidecar。
    #[serde(default, skip_serializing_if = "is_false")]
    pub codex_cli_only: bool,
    /// 历史备份元数据；不再作为客户端放行策略。
    #[serde(default, skip_serializing_if = "is_false")]
    pub codex_cli_only_allow_app_server: bool,
    #[serde(
        default,
        alias = "twoFactorSecret",
        alias = "accountTwoFactorSecret",
        skip_serializing_if = "Option::is_none"
    )]
    pub two_factor_secret: Option<String>,
    #[serde(
        default,
        alias = "accountPassword",
        alias = "password",
        skip_serializing_if = "Option::is_none"
    )]
    pub account_password: Option<String>,
    #[serde(
        default,
        alias = "phoneNumber",
        alias = "accountPhoneNumber",
        skip_serializing_if = "Option::is_none"
    )]
    pub phone_number: Option<String>,
    #[serde(
        default,
        alias = "mailUrl",
        alias = "mailAddress",
        alias = "mail_address",
        alias = "mailQueryUrl",
        alias = "mail_query_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub mail_url: Option<String>,
    #[serde(default)]
    pub app_speed: CodexAppSpeed,
    pub tokens: CodexTokens,
    #[serde(default)]
    pub token_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_updated_at: Option<i64>,
    #[serde(default = "default_token_source_mode")]
    pub token_source_mode: String,
    #[serde(
        default,
        alias = "authorizationStatus",
        skip_serializing_if = "Option::is_none"
    )]
    pub authorization_status: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_reauth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauth_reason: Option<String>,
    /// 官方客户端实际页面认证状态，由实例 CDP 只读观察更新。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_auth_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_client_auth_observed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_client_login_redirect_at: Option<i64>,
    /// 最近一次启动并开始观测该 Codex 实例的时间（Unix seconds）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_client_launch_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_client_auth_instance_id: Option<String>,
    pub quota: Option<CodexQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_quota_history: Option<CodexTeamQuotaHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<CodexQuotaErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_query_last_attempt_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_query_last_success_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_query_next_retry_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_query_last_error: Option<String>,
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

/// Codex Agent Identity credentials from the official auth.json format.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexAgentIdentity {
    #[serde(alias = "agentRuntimeId")]
    pub agent_runtime_id: String,
    #[serde(alias = "agentPrivateKey")]
    pub agent_private_key: String,
    #[serde(default, alias = "taskId", skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(alias = "accountId")]
    pub account_id: String,
    #[serde(alias = "chatgptUserId")]
    pub chatgpt_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, alias = "planType", skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, alias = "chatgptAccountIsFedramp")]
    pub chatgpt_account_is_fedramp: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTeamQuotaHistory {
    pub user_id: String,
    pub account_id: String,
    pub observed_at: i64,
    pub hourly_reset_time: Option<i64>,
    pub weekly_reset_time: Option<i64>,
    #[serde(default)]
    pub hourly_percentage: Option<i32>,
    #[serde(default)]
    pub weekly_percentage: Option<i32>,
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
    /// 主动重置次数（rate-limit reset credits）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits_available: Option<i64>,
    /// 主动重置明细（rate-limit reset credits）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reset_credits: Vec<CodexResetCredit>,
    /// 最近一张可用主动重置次数的到期时间 (Unix timestamp)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credits_next_expires_at: Option<i64>,
    /// 原始响应数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_data: Option<serde_json::Value>,
}

/// Codex 主动重置次数明细
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexResetCredit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redeemed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_status: Option<String>,
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
    #[serde(
        default,
        alias = "agentIdentity",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_identity: Option<CodexAgentIdentity>,
    /// Official personal access token auth shape (`at-*` only, no refresh/id token).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default)]
    pub detail_schema_version: u32,
    pub accounts: Vec<CodexAccountSummary>,
    pub current_account_id: Option<String>,
}

/// 账号摘要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAccountSummary {
    pub id: String,
    pub email: String,
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_active_until: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl CodexAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            detail_schema_version: 2,
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
    #[serde(default)]
    pub aud: serde_json::Value, // 可能是 string 或 array
    pub iss: Option<String>,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub sub: Option<String>,
    #[serde(rename = "https://api.openai.com/auth")]
    pub auth_data: Option<CodexAuthData>,
    #[serde(rename = "https://api.openai.com/profile")]
    pub profile_data: Option<CodexProfileData>,
}

/// JWT 中的 profile 数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexProfileData {
    pub email: Option<String>,
    pub email_verified: Option<bool>,
}

/// JWT 中的 auth 数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAuthData {
    pub chatgpt_user_id: Option<String>,
    pub chatgpt_plan_type: Option<String>,
    pub chatgpt_subscription_active_until: Option<serde_json::Value>,
    pub account_id: Option<String>,
    pub organization_id: Option<String>,
}

impl CodexAccount {
    pub fn remember_team_quota(&mut self) {
        let (Some(user_id), Some(account_id)) = (&self.user_id, &self.account_id) else {
            self.team_quota_history = None;
            return;
        };
        if self
            .team_quota_history
            .as_ref()
            .is_some_and(|h| h.user_id != *user_id || h.account_id != *account_id)
        {
            self.team_quota_history = None;
            return;
        }
        let Some(quota) = &self.quota else { return };
        let Some(raw) = &quota.raw_data else { return };
        // The usage response identifies the plan but does not consistently include
        // user/workspace IDs. The quota belongs to this already validated account;
        // scope the snapshot with the account identity stored alongside it.
        if raw.get("plan_type").and_then(|v| v.as_str()) != Some("team") {
            return;
        }
        let Some(observed_at) = self.usage_updated_at.filter(|v| *v > 0) else {
            return;
        };
        if self
            .team_quota_history
            .as_ref()
            .is_some_and(|h| h.observed_at > observed_at)
        {
            return;
        }
        let hourly = quota
            .hourly_reset_time
            .filter(|v| *v > 0 && quota.hourly_window_present != Some(false));
        let weekly = quota
            .weekly_reset_time
            .filter(|v| *v > 0 && quota.weekly_window_present != Some(false));
        if hourly.is_none() && weekly.is_none() {
            return;
        }
        self.team_quota_history = Some(CodexTeamQuotaHistory {
            user_id: user_id.clone(),
            account_id: account_id.clone(),
            observed_at,
            hourly_reset_time: hourly,
            weekly_reset_time: weekly,
            hourly_percentage: hourly.map(|_| quota.hourly_percentage.clamp(0, 100)),
            weekly_percentage: weekly.map(|_| quota.weekly_percentage.clamp(0, 100)),
        });
    }

    pub fn replace_quota_preserving_team_history(&mut self, quota: CodexQuota, observed_at: i64) {
        self.remember_team_quota();
        self.quota = Some(quota);
        self.usage_updated_at = Some(observed_at);
        self.remember_team_quota();
    }

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
            api_model_catalog: Vec::new(),
            api_model_context_windows: HashMap::new(),
            api_model_mappings: Vec::new(),
            api_sync_model_catalog_to_codex: false,
            api_wire_api: None,
            api_supports_websockets: false,
            api_supports_vision: false,
            api_model_vision_support: HashMap::new(),
            api_vision_routing_model: None,
            api_instance_access_mode: None,
            api_startup_model: None,
            api_image_generation_account_ids: Vec::new(),
            bound_oauth_account_id: None,
            bound_oauth_use_local_gateway: false,
            upstream_grok_account_id: None,
            user_id: None,
            plan_type: None,
            subscription_active_until: None,
            auth_file_plan_type: None,
            account_id: None,
            organization_id: None,
            agent_identity: None,
            account_name: None,
            account_structure: None,
            account_note: None,
            egress_proxy_url: None,
            codex_fingerprint_mode: None,
            codex_cli_only: false,
            codex_cli_only_allow_app_server: false,
            two_factor_secret: None,
            account_password: None,
            phone_number: None,
            mail_url: None,
            app_speed: CodexAppSpeed::Standard,
            tokens,
            token_generation: 0,
            token_updated_at: Some(now),
            token_source_mode: default_token_source_mode(),
            authorization_status: None,
            requires_reauth: false,
            reauth_reason: None,
            client_auth_status: None,
            last_client_auth_observed_at: None,
            last_client_login_redirect_at: None,
            last_client_launch_at: None,
            last_client_auth_instance_id: None,
            quota: None,
            team_quota_history: None,
            quota_error: None,
            usage_updated_at: None,
            subscription_query_last_attempt_at: None,
            subscription_query_last_success_at: None,
            subscription_query_next_retry_at: None,
            subscription_query_last_error: None,
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
        api_model_catalog: Vec<String>,
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
        account.api_model_catalog = api_model_catalog;
        account.api_sync_model_catalog_to_codex = false;
        account.api_wire_api = None;
        account.api_supports_websockets = false;
        account.api_supports_vision = false;
        account.api_model_vision_support = HashMap::new();
        account.api_vision_routing_model = None;
        account.plan_type = Some("API_KEY".to_string());
        account
    }

    pub fn is_api_key_auth(&self) -> bool {
        self.auth_mode == CodexAuthMode::Apikey
    }

    pub fn is_agent_identity_auth(&self) -> bool {
        self.agent_identity.is_some()
    }

    /// ChatGPT Web Session 导入账号：仅支持查看额度，不可启动/切号/加入 API 服务。
    pub fn is_web_session_auth(&self) -> bool {
        self.token_source_mode.trim() == "chatgpt_web_session"
    }

    pub fn update_last_used(&mut self) {
        self.last_used = chrono::Utc::now().timestamp();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_account() -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            "test".into(),
            "test@example.invalid".into(),
            "test-only".into(),
            CodexApiProviderMode::Custom,
            None,
            None,
            None,
            Vec::new(),
        );
        account.user_id = Some("user-a".into());
        account.account_id = Some("space-a".into());
        account
    }

    fn history_quota(plan: &str, reset: Option<i64>) -> CodexQuota {
        serde_json::from_value(serde_json::json!({
            "hourly_percentage": 0, "weekly_percentage": 10,
            "hourly_reset_time": reset, "weekly_reset_time": reset.map(|v| v + 1000),
            "hourly_window_present": reset.is_some(), "weekly_window_present": reset.is_some(),
            "raw_data": {"plan_type": plan}
        }))
        .unwrap()
    }

    #[test]
    fn team_history_survives_usage_based_transition_and_serialization() {
        let mut account = history_account();
        account.replace_quota_preserving_team_history(history_quota("team", Some(200)), 100);
        account.replace_quota_preserving_team_history(
            history_quota("self_serve_business_usage_based", None),
            150,
        );
        let restored: CodexAccount =
            serde_json::from_value(serde_json::to_value(account).unwrap()).unwrap();
        let history = restored.team_quota_history.unwrap();
        assert_eq!(history.hourly_reset_time, Some(200));
        assert_eq!(history.observed_at, 100);
        assert_eq!(restored.quota.unwrap().hourly_reset_time, None);
    }

    #[test]
    fn team_history_recalibrates_but_rejects_other_spaces_and_old_samples() {
        let mut account = history_account();
        account.replace_quota_preserving_team_history(history_quota("team", Some(200)), 100);
        account.replace_quota_preserving_team_history(history_quota("team", Some(300)), 150);
        account.replace_quota_preserving_team_history(history_quota("team", Some(180)), 90);
        assert_eq!(
            account
                .team_quota_history
                .as_ref()
                .unwrap()
                .hourly_reset_time,
            Some(300)
        );
        account.account_id = Some("space-b".into());
        account.remember_team_quota();
        assert!(account.team_quota_history.is_none());
    }

    #[test]
    fn team_history_seeds_existing_quota_before_first_business_refresh() {
        let mut account = history_account();
        account.quota = Some(history_quota("team", Some(200)));
        account.usage_updated_at = Some(100);
        account.replace_quota_preserving_team_history(
            history_quota("self_serve_business_usage_based", None),
            150,
        );
        assert_eq!(
            account.team_quota_history.unwrap().hourly_reset_time,
            Some(200)
        );
        let mut unknown = history_account();
        unknown.replace_quota_preserving_team_history(
            history_quota("self_serve_business_usage_based", None),
            150,
        );
        assert!(unknown.team_quota_history.is_none());
    }

    #[test]
    fn team_history_requires_account_identity_and_reset_windows() {
        let mut missing_identity = history_account();
        missing_identity.user_id = None;
        missing_identity.replace_quota_preserving_team_history(
            history_quota("team", Some(200)),
            100,
        );
        assert!(missing_identity.team_quota_history.is_none());

        let mut account = history_account();
        account.replace_quota_preserving_team_history(history_quota("team", None), 110);
        assert!(account.team_quota_history.is_none());
    }

    #[test]
    fn team_history_survives_quota_loss_without_inventing_a_new_window() {
        let mut account = history_account();
        account.replace_quota_preserving_team_history(history_quota("team", Some(200)), 100);
        account.quota = None;
        account.usage_updated_at = Some(500);
        account.remember_team_quota();
        let history = account.team_quota_history.unwrap();
        assert_eq!(history.hourly_reset_time, Some(200));
        assert_eq!(history.observed_at, 100);
    }

    #[test]
    fn legacy_account_without_websocket_field_defaults_to_false() {
        let account = CodexAccount::new_api_key(
            "legacy-account".to_string(),
            "api-key-account".to_string(),
            "sk-test".to_string(),
            CodexApiProviderMode::Custom,
            Some("https://relay.example.com/v1".to_string()),
            Some("relay".to_string()),
            Some("Relay".to_string()),
            Vec::new(),
        );
        let mut value = serde_json::to_value(account).expect("serialize account");
        value
            .as_object_mut()
            .expect("account object")
            .remove("api_supports_websockets");

        let restored: CodexAccount = serde_json::from_value(value).expect("deserialize account");
        assert!(!restored.api_supports_websockets);
        assert!(!restored.api_sync_model_catalog_to_codex);
    }
}
