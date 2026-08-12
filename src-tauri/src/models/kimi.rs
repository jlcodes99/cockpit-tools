use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KimiUsageRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_duration: Option<i64>,
    pub used: f64,
    pub limit: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KimiQuota {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_limit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_reset_at: Option<String>,
    #[serde(default)]
    pub limits: Vec<KimiUsageRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub booster_balance_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub booster_total_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub booster_currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_level_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiAccount {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default)]
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Unix seconds when access_token expires (official wire).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<KimiQuota>,
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
    pub created_at: i64,
    pub last_used: i64,
}

/// IPC-facing account DTO (camelCase wire). Credentials always empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiAccountView {
    pub id: String,
    pub email: String,
    /// Always empty over IPC — credentials stay in Rust storage.
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<KimiQuota>,
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
    pub created_at: i64,
    pub last_used: i64,
}

impl From<&KimiAccount> for KimiAccountView {
    fn from(account: &KimiAccount) -> Self {
        Self {
            id: account.id.clone(),
            email: account.email.clone(),
            access_token: String::new(),
            tags: account.tags.clone(),
            nickname: account.nickname.clone(),
            user_id: account.user_id.clone(),
            avatar: account.avatar.clone(),
            expires_at: account.expires_at,
            plan_type: account.plan_type.clone(),
            quota: account.quota.clone(),
            status: account.status.clone(),
            status_reason: account.status_reason.clone(),
            quota_query_last_error: account.quota_query_last_error.clone(),
            quota_query_last_error_at: account.quota_query_last_error_at,
            usage_updated_at: account.usage_updated_at,
            created_at: account.created_at,
            last_used: account.last_used,
        }
    }
}

/// On-disk index row (snake_case for stable local files) — not the IPC wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KimiAccountSummary {
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
pub struct KimiAccountIndex {
    #[serde(default = "default_index_version")]
    pub version: String,
    #[serde(default)]
    pub accounts: Vec<KimiAccountSummary>,
}

fn default_index_version() -> String {
    "1.0".to_string()
}

impl KimiAccount {
    pub fn summary(&self) -> KimiAccountSummary {
        KimiAccountSummary {
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
pub struct KimiOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct KimiOAuthCompletePayload {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub expires_at: i64,
    pub expires_in: i64,
    pub device_id: String,
    pub email: String,
    pub nickname: Option<String>,
    pub user_id: Option<String>,
    pub avatar: Option<String>,
    pub plan_type: Option<String>,
}
