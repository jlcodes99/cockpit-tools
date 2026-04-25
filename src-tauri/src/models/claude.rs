use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    pub config_dir: String,
    pub login_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_hint_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anthropic_auth_token: Option<String>,
    #[serde(default)]
    pub disable_nonessential_traffic: bool,

    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_raw: Option<serde_json::Value>,

    pub created_at: i64,
    pub last_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub login_mode: String,
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountIndex {
    pub version: String,
    pub accounts: Vec<ClaudeAccountSummary>,
}

impl ClaudeAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for ClaudeAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeAccount {
    pub fn summary(&self) -> ClaudeAccountSummary {
        ClaudeAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            name: self.name.clone(),
            tags: self.tags.clone(),
            login_mode: self.login_mode.clone(),
            logged_in: self.logged_in,
            subscription_type: self.subscription_type.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
            last_synced_at: self.last_synced_at,
        }
    }
}
