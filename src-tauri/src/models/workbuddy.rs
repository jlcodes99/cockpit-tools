use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbuddyAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage_notify_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage_notify_zh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dosage_notify_en: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_raw: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,

    // 签到相关字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checkin_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin_streak: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_rewards: Option<serde_json::Value>,

    pub created_at: i64,
    pub last_used: i64,

    // 网页会话（WebView）相关字段
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_session_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbuddyAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkbuddyAccountIndex {
    pub version: String,
    pub accounts: Vec<WorkbuddyAccountSummary>,
}

impl WorkbuddyAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for WorkbuddyAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct WorkbuddyOAuthCompletePayload {
    pub email: String,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub enterprise_id: Option<String>,
    pub enterprise_name: Option<String>,

    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,
    pub domain: Option<String>,

    pub plan_type: Option<String>,
    pub dosage_notify_code: Option<String>,
    pub dosage_notify_zh: Option<String>,
    pub dosage_notify_en: Option<String>,
    pub payment_type: Option<String>,

    pub quota_raw: Option<serde_json::Value>,
    pub auth_raw: Option<serde_json::Value>,
    pub profile_raw: Option<serde_json::Value>,
    pub usage_raw: Option<serde_json::Value>,

    pub status: Option<String>,
    pub status_reason: Option<String>,
}

impl WorkbuddyAccount {
    pub fn summary(&self) -> WorkbuddyAccountSummary {
        WorkbuddyAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            tags: self.tags.clone(),
            plan_type: self.plan_type.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
