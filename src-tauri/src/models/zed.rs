use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedAccount {
    pub id: String,
    pub user_id: String,
    pub github_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_overdue_invoices: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_period_start_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_period_end_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial_end_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_spend_used_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_spend_limit_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_spend_remaining_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_predictions_used: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_predictions_limit_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit_predictions_remaining_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spending_limit_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_portal_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_tokens_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferences_raw: Option<Value>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedStoredAccount {
    #[serde(flatten)]
    pub public_account: ZedAccount,
    pub access_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedAccountSummary {
    pub id: String,
    pub user_id: String,
    pub github_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZedAccountIndex {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_account_id: Option<String>,
    pub accounts: Vec<ZedAccountSummary>,
}

impl ZedAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            current_account_id: None,
            accounts: Vec::new(),
        }
    }
}

impl Default for ZedAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZedOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZedRuntimeStatus {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_started_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_account_id: Option<String>,
    pub app_path_configured: bool,
}

impl ZedStoredAccount {
    pub fn to_public(&self) -> ZedAccount {
        self.public_account.clone()
    }

    pub fn summary(&self) -> ZedAccountSummary {
        ZedAccountSummary {
            id: self.public_account.id.clone(),
            user_id: self.public_account.user_id.clone(),
            github_login: self.public_account.github_login.clone(),
            display_name: self.public_account.display_name.clone(),
            plan_raw: self.public_account.plan_raw.clone(),
            tags: self.public_account.tags.clone(),
            created_at: self.public_account.created_at,
            last_used: self.public_account.last_used,
        }
    }
}
