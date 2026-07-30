use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GrokAuthMode {
    #[default]
    Oauth,
    ApiKey,
}

impl GrokAuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Oauth => "oauth",
            Self::ApiKey => "api_key",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrokProductUsage {
    pub product: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GrokQuota {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_limit_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_demand_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_demand_cap: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepaid_balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequent_usage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequent_limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occasional_usage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occasional_limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<String>,
    #[serde(default)]
    pub products: Vec<GrokProductUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccount {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub auth_mode: GrokAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_data_retention_opt_out: Option<bool>,
    #[serde(default)]
    pub access_token: String,
    /// xAI API key for ApiKey auth mode. Never exposed through GrokAccountView.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// OpenAI-compatible endpoint for a third-party API-key model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Model identifier sent to the third-party endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<GrokQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_usage_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_grok_code_access: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    /// Preferred CLI working directory for this account (account overview launch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl GrokAccount {
    pub fn is_api_key_auth(&self) -> bool {
        self.auth_mode == GrokAuthMode::ApiKey
    }

    pub fn resolved_api_key(&self) -> Option<&str> {
        if !self.is_api_key_auth() {
            return None;
        }
        self.api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccountView {
    pub id: String,
    pub email: String,
    // Kept for the shared frontend account shape; real credentials never cross IPC.
    pub access_token: String,
    #[serde(default)]
    pub auth_mode: GrokAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_image_asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coding_data_retention_opt_out: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// OpenAI-compatible endpoint configured for this API-key account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    /// Model identifier configured for this API-key account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_grok_code_access: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<GrokQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl From<&GrokAccount> for GrokAccountView {
    fn from(account: &GrokAccount) -> Self {
        Self {
            id: account.id.clone(),
            email: account.email.clone(),
            access_token: String::new(),
            auth_mode: account.auth_mode,
            tags: account.tags.clone(),
            first_name: account.first_name.clone(),
            last_name: account.last_name.clone(),
            user_id: account.user_id.clone(),
            principal_id: account.principal_id.clone(),
            principal_type: account.principal_type.clone(),
            team_id: account.team_id.clone(),
            profile_image_asset_id: account.profile_image_asset_id.clone(),
            coding_data_retention_opt_out: account.coding_data_retention_opt_out,
            expires_at: account.expires_at,
            api_base_url: account.api_base_url.clone(),
            api_model: account.api_model.clone(),
            has_grok_code_access: account.has_grok_code_access,
            plan_type: account.plan_type.clone(),
            quota: account.quota.clone(),
            status: account.status.clone(),
            status_reason: account.status_reason.clone(),
            quota_query_last_error: account.quota_query_last_error.clone(),
            quota_query_last_error_at: account.quota_query_last_error_at,
            usage_updated_at: account.usage_updated_at,
            working_dir: account.working_dir.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrokAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GrokAccountIndex {
    #[serde(default = "default_index_version")]
    pub version: String,
    #[serde(default)]
    pub accounts: Vec<GrokAccountSummary>,
}

fn default_index_version() -> String {
    "1.0".to_string()
}

impl GrokAccount {
    pub fn summary(&self) -> GrokAccountSummary {
        GrokAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            tags: self.tags.clone(),
            plan_type: self.plan_type.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct GrokOAuthCompletePayload {
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub user_id: Option<String>,
    pub principal_id: Option<String>,
    pub principal_type: Option<String>,
    pub team_id: Option<String>,
    pub profile_image_asset_id: Option<String>,
    pub coding_data_retention_opt_out: Option<bool>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,
    pub token_endpoint: String,
    pub auth_raw: serde_json::Value,
}
