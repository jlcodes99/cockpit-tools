use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuthMode {
    #[serde(rename = "oauth", alias = "o_auth")]
    OAuth,
    SetupToken,
    ApiKey,
    #[serde(rename = "desktop_oauth", alias = "desktop_o_auth")]
    DesktopOAuth,
    DesktopGateway,
}

impl Default for ClaudeAuthMode {
    fn default() -> Self {
        Self::OAuth
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccount {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub auth_mode: ClaudeAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<ClaudeQuota>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<ClaudeQuotaErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_source_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_api_key_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_model_catalog: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_extra_env: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_auth_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_credential_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_profile_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_connection_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_upstream_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_model_mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_profile_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_profile_imported_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_credentials_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_config_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_usage_raw: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_note: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeQuota {
    pub five_hour_percentage: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour_reset_time: Option<i64>,
    pub seven_day_percentage: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_reset_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_sonnet_percentage: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day_sonnet_reset_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage_percentage: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage_reset_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage_used_cents: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_usage_limit_cents: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeQuotaErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountSummary {
    pub id: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_updated_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_source_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_provider_api_key_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_model_catalog: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_auth_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_credential_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_profile_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_connection_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_upstream_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_gateway_model_mappings: Option<Vec<ClaudeDesktopGatewayModelMapping>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeAccountIndex {
    pub version: String,
    pub accounts: Vec<ClaudeAccountSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopLoginStartResponse {
    pub login_id: String,
    pub user_data_dir: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeOAuthStartResponse {
    pub login_id: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopGatewayModel {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopGatewayModelMapping {
    pub desktop_model: String,
    pub upstream_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic_family_tier: Option<String>,
    #[serde(
        default,
        rename = "supports1m",
        skip_serializing_if = "Option::is_none"
    )]
    pub supports_1m: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopGatewayModelsResult {
    pub models: Vec<ClaudeDesktopGatewayModel>,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_mode: Option<String>,
    #[serde(default)]
    pub has_claude_models: bool,
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
            account_uuid: self.account_uuid.clone(),
            organization_uuid: self.organization_uuid.clone(),
            organization_name: self.organization_name.clone(),
            plan_type: self.plan_type.clone(),
            avatar_url: self.avatar_url.clone(),
            profile_updated_at: self.profile_updated_at,
            tags: self.tags.clone(),
            account_note: self.account_note.clone(),
            api_base_url: self.api_base_url.clone(),
            api_provider_id: self.api_provider_id.clone(),
            api_provider_name: self.api_provider_name.clone(),
            api_provider_source_tag: self.api_provider_source_tag.clone(),
            api_provider_website: self.api_provider_website.clone(),
            api_provider_api_key_url: self.api_provider_api_key_url.clone(),
            api_key_field: self.api_key_field.clone(),
            api_model_catalog: self.api_model_catalog.clone(),
            desktop_gateway_auth_scheme: self.desktop_gateway_auth_scheme.clone(),
            desktop_gateway_credential_kind: self.desktop_gateway_credential_kind.clone(),
            desktop_gateway_profile_dir: self.desktop_gateway_profile_dir.clone(),
            desktop_gateway_models: self.desktop_gateway_models.clone(),
            desktop_gateway_connection_mode: self.desktop_gateway_connection_mode.clone(),
            desktop_gateway_upstream_models: self.desktop_gateway_upstream_models.clone(),
            desktop_gateway_model_mappings: self.desktop_gateway_model_mappings.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}
