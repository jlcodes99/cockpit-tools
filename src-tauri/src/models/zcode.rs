use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ZcodeQuotaItem {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent_used: Option<f64>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub window: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_end: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccount {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_expire: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_percent_used: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quota_items: Vec<ZcodeQuotaItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeStoredAccount {
    #[serde(flatten)]
    pub public_account: ZcodeAccount,
    pub credentials: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccountSummary {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZcodeAccountIndex {
    pub version: String,
    pub accounts: Vec<ZcodeAccountSummary>,
}

impl ZcodeAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for ZcodeAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ZcodeStoredAccount {
    pub fn to_public(&self) -> ZcodeAccount {
        self.public_account.clone()
    }

    pub fn summary(&self) -> ZcodeAccountSummary {
        ZcodeAccountSummary {
            id: self.public_account.id.clone(),
            name: self.public_account.name.clone(),
            provider: self.public_account.provider.clone(),
            plan_tier: self.public_account.plan_tier.clone(),
            tags: self.public_account.tags.clone(),
            created_at: self.public_account.created_at,
            last_used: self.public_account.last_used,
        }
    }
}
