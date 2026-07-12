use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Grok CLI OAuth 账号（仅会话账号，不做 API Key）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccount {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_image_asset_id: Option<String>,

    /// JWT tier 原始值（如 4）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<i64>,
    /// 套餐展示：Free / SuperGrok Lite / SuperGrok / SuperGrok Heavy 等
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_label: Option<String>,

    /// access token（auth.json 字段名为 key）
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// 过期时间 unix 秒
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// auth.json 中 ISO 字符串形式的 expires_at
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_raw: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,
    /// auth.json 顶层 key：官方 CLI 为 `https://auth.x.ai::<client_id>`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_entry_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_data_retention_opt_out: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_grok_code_access: Option<bool>,

    /// 实时额度（cli-chat-proxy /v1/billing）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<GrokQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_updated_at: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_reauth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauth_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_query_last_success_at: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub userinfo_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_raw: Option<Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_note: Option<String>,

    pub created_at: i64,
    pub last_used: i64,
}

/// Grok 额度（对齐 cli-chat-proxy `/v1/billing`）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GrokQuota {
    /// 月额度上限（单位与上游 val 一致，通常为美元额度点）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monthly_limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    /// 已用百分比 0-100
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_demand_cap: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_demand_used: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepaid_balance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_period_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_period_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unlimited_or_free: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exhausted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exhaust_reason: Option<String>,
    /// 便于前端进度条：剩余百分比
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccountIndex {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_account_id: Option<String>,
    pub accounts: Vec<GrokAccountSummary>,
}

impl GrokAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            current_account_id: None,
            accounts: Vec::new(),
        }
    }
}

impl Default for GrokAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl GrokAccount {
    pub fn summary(&self) -> GrokAccountSummary {
        GrokAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            plan_type: self.plan_type.clone(),
            tier: self.tier,
            tags: self.tags.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }

    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return name.to_string();
        }
        let first = self.first_name.as_deref().unwrap_or("").trim();
        let last = self.last_name.as_deref().unwrap_or("").trim();
        let combined = format!("{} {}", first, last).trim().to_string();
        if !combined.is_empty() {
            return combined;
        }
        self.email.clone()
    }

    pub fn update_last_used(&mut self) {
        self.last_used = chrono::Utc::now().timestamp();
    }
}

/// Device-code OAuth 启动响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    pub expires_in: u64,
    pub interval_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}
