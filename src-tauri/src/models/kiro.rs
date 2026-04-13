use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    // 敏感字段：前端仅用于切号/刷新，不应打印到日志。
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub idc_region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_reset_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bonus_expire_days: Option<i64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub kiro_auth_token_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kiro_profile_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kiro_usage_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,

    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroAccountIndex {
    pub version: String,
    pub accounts: Vec<KiroAccountSummary>,
}

impl KiroAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for KiroAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroOAuthStartResponse {
    pub login_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct KiroOAuthCompletePayload {
    pub email: String,
    pub user_id: Option<String>,
    pub login_provider: Option<String>,

    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,

    pub idc_region: Option<String>,
    pub issuer_url: Option<String>,
    pub client_id: Option<String>,
    pub scopes: Option<String>,
    pub login_hint: Option<String>,

    pub plan_name: Option<String>,
    pub plan_tier: Option<String>,
    pub credits_total: Option<f64>,
    pub credits_used: Option<f64>,
    pub bonus_total: Option<f64>,
    pub bonus_used: Option<f64>,
    pub usage_reset_at: Option<i64>,
    pub bonus_expire_days: Option<i64>,

    pub kiro_auth_token_raw: Option<serde_json::Value>,
    pub kiro_profile_raw: Option<serde_json::Value>,
    pub kiro_usage_raw: Option<serde_json::Value>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
}

impl KiroAccount {
    pub fn summary(&self) -> KiroAccountSummary {
        KiroAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            tags: self.tags.clone(),
            plan_name: self.plan_name.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
