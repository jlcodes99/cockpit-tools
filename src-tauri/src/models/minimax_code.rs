use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimaxCodeAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
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
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_file: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_raw: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_raw: Option<serde_json::Value>,

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

    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_checkin_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkin_streak: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkin_rewards: Option<serde_json::Value>,

    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimaxCodeAccountSummary {
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
pub struct MinimaxCodeAccountIndex {
    pub version: String,
    pub accounts: Vec<MinimaxCodeAccountSummary>,
}

impl MinimaxCodeAccountIndex {
    pub fn new() -> Self { Self { version: "1.0".to_string(), accounts: Vec::new() } }
}
impl Default for MinimaxCodeAccountIndex { fn default() -> Self { Self::new() } }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub user_code: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct MinimaxCodeOAuthCompletePayload {
    pub email: String,
    /// Optional explicit account id (used by local import, where there is no uid).
    /// When present it becomes the identity seed so accounts imported from
    /// different data dirs do not collapse onto the same id.
    pub id: Option<String>,
    pub uid: Option<String>,
    pub nickname: Option<String>,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: Option<String>,
    pub expires_at: Option<i64>,
    pub region: Option<String>,
    pub auth_file: Option<String>,
    pub quota_raw: Option<serde_json::Value>,
    pub auth_raw: Option<serde_json::Value>,
    pub profile_raw: Option<serde_json::Value>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
}

impl MinimaxCodeAccount {
    pub fn summary(&self) -> MinimaxCodeAccountSummary {
        MinimaxCodeAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            tags: self.tags.clone(),
            plan_type: self.plan_type.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
