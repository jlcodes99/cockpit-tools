use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,

    pub access_token: String,

    pub tier: OpenCodeTier,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_status: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_raw: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,

    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub tier: OpenCodeTier,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeAccountIndex {
    pub version: String,
    pub accounts: Vec<OpenCodeAccountSummary>,
}

impl OpenCodeAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for OpenCodeAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpenCodeTier {
    #[serde(rename = "go")]
    Go,
    #[serde(rename = "zen")]
    Zen,
    #[serde(rename = "free")]
    Free,
}

impl std::fmt::Display for OpenCodeTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenCodeTier::Go => write!(f, "go"),
            OpenCodeTier::Zen => write!(f, "zen"),
            OpenCodeTier::Free => write!(f, "free"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeImportPayload {
    pub email: String,
    pub name: Option<String>,
    pub access_token: String,
    pub tier: OpenCodeTier,
    pub plan_name: Option<String>,
    pub subscription_status: Option<String>,
    pub usage_raw: Option<serde_json::Value>,
    pub status: Option<String>,
    pub status_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeGoLimits {
    pub usage_5h_dollars: f64,
    pub usage_weekly_dollars: f64,
    pub usage_monthly_dollars: f64,
    pub limit_5h: f64,
    pub limit_weekly: f64,
    pub limit_monthly: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_times: Option<OpenCodeGoResetTimes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeGoResetTimes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_5h: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_weekly: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_monthly: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeZenBalance {
    pub balance_dollars: f64,
    pub auto_reload_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_spend_limit: Option<f64>,
}

impl OpenCodeAccount {
    pub fn summary(&self) -> OpenCodeAccountSummary {
        OpenCodeAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            tags: self.tags.clone(),
            tier: self.tier.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
