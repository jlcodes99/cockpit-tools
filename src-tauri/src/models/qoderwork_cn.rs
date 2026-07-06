use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QoderworkCnAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits_usage_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_quota_exceeded: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_backup_at: Option<i64>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QoderworkCnAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QoderworkCnAccountIndex {
    pub version: String,
    pub accounts: Vec<QoderworkCnAccountSummary>,
}

impl QoderworkCnAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for QoderworkCnAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderworkCnOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthV2Data {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// QoderWork CN uses camelCase "refreshToken"; alias for legacy snake_case reads
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "refreshToken",
        alias = "refresh_token"
    )]
    pub refresh_token: Option<String>,
    /// QoderWork CN uses camelCase "expiresAt"; alias for legacy snake_case reads
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "expiresAt",
        alias = "expires_at"
    )]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<AuthV2User>,
    /// Required by QoderWork CN: must be 2
    #[serde(skip_serializing_if = "Option::is_none", rename = "schemaVersion", alias = "schema_version")]
    pub schema_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "loginMethod", alias = "login_method")]
    pub login_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "refreshStrategy", alias = "refresh_strategy")]
    pub refresh_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "refreshTokenExpiresAt", alias = "refresh_token_expires_at")]
    pub refresh_token_expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "loginDeviceId", alias = "login_device_id")]
    pub login_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "loginTimestamp", alias = "login_timestamp")]
    pub login_timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthV2User {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl QoderworkCnAccount {
    pub fn summary(&self) -> QoderworkCnAccountSummary {
        QoderworkCnAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            user_id: self.user_id.clone(),
            user_type: self.user_type.clone(),
            tags: self.tags.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
