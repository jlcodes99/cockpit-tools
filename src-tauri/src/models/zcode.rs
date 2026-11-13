use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZcodeAuthMode {
    #[default]
    Oauth,
    ApiKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccount {
    pub id: String,
    #[serde(default)]
    pub auth_mode: ZcodeAuthMode,
    pub provider: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub zcode_jwt_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_reset_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_info_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_raw: Option<Value>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccountSummary {
    pub id: String,
    #[serde(default)]
    pub auth_mode: ZcodeAuthMode,
    pub provider: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccountIndex {
    pub version: String,
    #[serde(default)]
    pub current_account_id: Option<String>,
    pub accounts: Vec<ZcodeAccountSummary>,
    #[serde(skip)]
    pub(crate) storage_revision: crate::modules::provider_account_index::Revision,
}

impl Default for ZcodeAccountIndex {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            current_account_id: None,
            accounts: Vec::new(),
            storage_revision: Default::default(),
        }
    }
}

impl ZcodeAccount {
    pub fn summary(&self) -> ZcodeAccountSummary {
        ZcodeAccountSummary {
            id: self.id.clone(),
            auth_mode: self.auth_mode,
            provider: self.provider.clone(),
            email: self.email.clone(),
            user_id: self.user_id.clone(),
            plan_type: self.plan_type.clone(),
            tags: self.tags.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZcodeOAuthStartResponse {
    pub login_id: String,
    pub provider: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
    pub callback_url: String,
}
