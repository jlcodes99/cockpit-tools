use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindsurfAccount {
    pub id: String,
    pub github_login: String,
    pub github_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub github_access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_scope: Option<String>,
    pub copilot_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_chat_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_refresh_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_quota_snapshots: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_quota_reset_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_limited_user_quotas: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_limited_user_reset_date: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_api_server_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_auth_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_user_status: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_plan_status: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windsurf_auth_status_raw: Option<serde_json::Value>,
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
pub struct WindsurfAccountSummary {
    pub id: String,
    pub github_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copilot_plan: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindsurfAccountIndex {
    pub version: String,
    pub accounts: Vec<WindsurfAccountSummary>,
}

impl WindsurfAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for WindsurfAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindsurfOAuthStartResponse {
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
pub struct WindsurfOAuthCompletePayload {
    pub github_login: String,
    pub github_id: u64,
    pub github_name: Option<String>,
    pub github_email: Option<String>,
    pub github_access_token: String,
    pub github_token_type: Option<String>,
    pub github_scope: Option<String>,
    pub copilot_token: String,
    pub copilot_plan: Option<String>,
    pub copilot_chat_enabled: Option<bool>,
    pub copilot_expires_at: Option<i64>,
    pub copilot_refresh_in: Option<i64>,
    pub copilot_quota_snapshots: Option<serde_json::Value>,
    pub copilot_quota_reset_date: Option<String>,
    pub copilot_limited_user_quotas: Option<serde_json::Value>,
    pub copilot_limited_user_reset_date: Option<i64>,
    pub windsurf_api_key: Option<String>,
    pub windsurf_api_server_url: Option<String>,
    pub windsurf_auth_token: Option<String>,
    pub windsurf_user_status: Option<serde_json::Value>,
    pub windsurf_plan_status: Option<serde_json::Value>,
    pub windsurf_auth_status_raw: Option<serde_json::Value>,
}

impl WindsurfAccount {
    pub fn summary(&self) -> WindsurfAccountSummary {
        WindsurfAccountSummary {
            id: self.id.clone(),
            github_login: self.github_login.clone(),
            github_email: self.github_email.clone(),
            tags: self.tags.clone(),
            copilot_plan: self.copilot_plan.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
