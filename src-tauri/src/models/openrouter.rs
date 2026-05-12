use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterAccount {
    pub id: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "key_type")]
    pub key_type: OpenRouterKeyType,
    #[serde(rename = "is_free_tier")]
    pub is_free_tier: bool,
    #[serde(default)]
    pub usage: Option<f64>,
    #[serde(default)]
    pub usage_daily: Option<f64>,
    #[serde(default)]
    pub usage_weekly: Option<f64>,
    #[serde(default)]
    pub usage_monthly: Option<f64>,
    #[serde(default)]
    pub limit: Option<f64>,
    #[serde(default)]
    pub limit_remaining: Option<f64>,
    #[serde(default)]
    pub total_credits: Option<f64>,
    #[serde(default)]
    pub total_usage: Option<f64>,
    #[serde(default)]
    pub rate_limit_requests: Option<i64>,
    #[serde(default)]
    pub rate_limit_interval: Option<String>,
    #[serde(default)]
    pub key_label: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub status_reason: Option<String>,
    #[serde(default)]
    pub usage_updated_at: Option<i64>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub quota_query_last_error: Option<String>,
    #[serde(default)]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_key_raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_credits_raw: Option<Value>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OpenRouterKeyType {
    #[serde(rename = "api")]
    Api,
    #[serde(rename = "management")]
    Management,
    #[serde(rename = "provisioning")]
    Provisioning,
}

impl OpenRouterKeyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OpenRouterKeyType::Api => "api",
            OpenRouterKeyType::Management => "management",
            OpenRouterKeyType::Provisioning => "provisioning",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "management" => OpenRouterKeyType::Management,
            "provisioning" => OpenRouterKeyType::Provisioning,
            _ => OpenRouterKeyType::Api,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterUsage {
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub remaining: Option<f64>,
    pub percentage: Option<f64>,
    pub daily: Option<f64>,
    pub weekly: Option<f64>,
    pub monthly: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterCreditInfo {
    pub total_credits: Option<f64>,
    pub total_usage: Option<f64>,
    pub total_paid: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModel {
    pub id: String,
    pub name: String,
    pub pricing: OpenRouterModelPricing,
    pub context_length: i64,
    pub top_provider: String,
    pub is_free: bool,
    pub supported_parameters: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterModelPricing {
    pub prompt: String,
    pub completion: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterAccountSummary {
    pub id: String,
    pub email: String,
    pub key_type: OpenRouterKeyType,
    pub is_free_tier: bool,
    pub usage: Option<f64>,
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterAccountIndex {
    pub version: String,
    pub accounts: Vec<OpenRouterAccountSummary>,
}

impl OpenRouterAccountIndex {
    pub fn new() -> Self {
        Self {
            version: "1.0".to_string(),
            accounts: Vec::new(),
        }
    }
}

impl Default for OpenRouterAccountIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenRouterAccount {
    pub fn summary(&self) -> OpenRouterAccountSummary {
        OpenRouterAccountSummary {
            id: self.id.clone(),
            email: self.email.clone(),
            key_type: self.key_type.clone(),
            is_free_tier: self.is_free_tier,
            usage: self.usage,
            tags: self.tags.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
