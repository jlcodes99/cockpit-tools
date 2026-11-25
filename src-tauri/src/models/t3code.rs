use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum T3CodeAuthMode {
    #[default]
    Clerk,
    LocalProfile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct T3CodeAccount {
    pub id: String,
    #[serde(default)]
    pub auth_mode: T3CodeAuthMode,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub clerk_jwt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_verified_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_verified_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t3chat_linked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_reset_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_query_last_error_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clerk_tokens_raw: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_snapshot_raw: Option<Value>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct T3CodeAccountSummary {
    pub id: String,
    #[serde(default)]
    pub auth_mode: T3CodeAuthMode,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: i64,
    pub last_used: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct T3CodeAccountIndex {
    pub version: String,
    #[serde(default)]
    pub current_account_id: Option<String>,
    pub accounts: Vec<T3CodeAccountSummary>,
    #[serde(skip)]
    pub(crate) storage_revision: crate::modules::provider_account_index::Revision,
}

impl Default for T3CodeAccountIndex {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            current_account_id: None,
            accounts: Vec::new(),
            storage_revision: Default::default(),
        }
    }
}

impl T3CodeAccount {
    pub fn summary(&self) -> T3CodeAccountSummary {
        T3CodeAccountSummary {
            id: self.id.clone(),
            auth_mode: self.auth_mode,
            email: self.email.clone(),
            user_id: self.user_id.clone(),
            plan_type: self.plan_type.clone(),
            tags: self.tags.clone(),
            created_at: self.created_at,
            last_used: self.last_used,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum T3CodeQuotaRemaining {
    Percent { remaining_percent: i32 },
    Amount { remaining_amount: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeQuotaWindowView {
    pub id: String,
    pub label: String,
    #[serde(flatten)]
    pub remaining: T3CodeQuotaRemaining,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeBindableAccountView {
    pub id: String,
    pub driver: String,
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub plan_badge: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_badge_kind: Option<String>,
    pub auth_mode: String,
    pub is_api_service: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    pub windows: Vec<T3CodeQuotaWindowView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_summary_groups: Option<Vec<crate::models::quota::QuotaSummaryGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_quota: Option<crate::modules::codex_provider_pool_quota::ProviderPoolQuota>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_percent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_percent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_tok_per_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeProviderRuntimeView {
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
    pub account_email: Option<String>,
    pub workspace_id: Option<String>,
    pub auth_status: Option<String>,
    #[serde(default)]
    pub auth_uses_keyring: bool,
    pub model_count: usize,
    pub availability: Option<String>,
    pub matches_gateway: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum T3CodeDefaultSelectionState {
    Waiting,
    RestartRequired,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct T3CodePendingDefaultView {
    pub state: T3CodeDefaultSelectionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeProviderInstanceView {
    pub instance_id: String,
    pub driver: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent_color: Option<String>,
    pub enabled: bool,
    pub is_builtin_default: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow_home_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_account_email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_account_plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_account_plan_badge_kind: Option<String>,
    pub bound_is_api_service: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_account_workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_updated_at: Option<i64>,
    #[serde(default)]
    pub binding_applied_live: bool,
    #[serde(default)]
    pub model_refresh_pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pending_default: Option<T3CodePendingDefaultView>,
    #[serde(default)]
    pub needs_repair: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit_runtime_path: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<T3CodeProviderRuntimeView>,
    pub windows: Vec<T3CodeQuotaWindowView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_summary_groups: Option<Vec<crate::models::quota::QuotaSummaryGroup>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) provider_quota: Option<crate::modules::codex_provider_pool_quota::ProviderPoolQuota>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_tok_per_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeProviderWorkspaceView {
    pub t3_instance_id: String,
    pub state_dir: String,
    pub t3_running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t3_started_at: Option<String>,
    pub injection_connected: bool,
    pub provider_instances: Vec<T3CodeProviderInstanceView>,
    pub codex_accounts: Vec<T3CodeBindableAccountView>,
    #[serde(default)]
    pub codex_unverified_count: usize,
    pub antigravity_accounts: Vec<T3CodeBindableAccountView>,
    pub claude_accounts: Vec<T3CodeBindableAccountView>,
    #[serde(default)]
    pub extra_accounts: std::collections::HashMap<String, Vec<T3CodeBindableAccountView>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct T3CodeProviderSwitchView {
    pub workspace: T3CodeProviderWorkspaceView,
    pub applied_live: bool,
    pub restart_required: bool,
    pub message: String,
}
