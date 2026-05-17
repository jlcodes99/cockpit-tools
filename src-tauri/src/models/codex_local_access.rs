use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessRoutingStrategy {
    Auto,
    QuotaHighFirst,
    QuotaLowFirst,
    PlanHighFirst,
    PlanLowFirst,
    ExpirySoonFirst,
}

impl Default for CodexLocalAccessRoutingStrategy {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessScope {
    Localhost,
    Lan,
}

fn default_access_scope_for_existing_config() -> CodexLocalAccessScope {
    CodexLocalAccessScope::Lan
}

fn default_restrict_free_accounts() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessCollection {
    pub enabled: bool,
    pub port: u16,
    pub api_key: String,
    #[serde(default = "default_access_scope_for_existing_config")]
    pub access_scope: CodexLocalAccessScope,
    #[serde(default)]
    pub routing_strategy: CodexLocalAccessRoutingStrategy,
    #[serde(default = "default_restrict_free_accounts")]
    pub restrict_free_accounts: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_oauth_account_id: Option<String>,
    pub account_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessUsageStats {
    #[serde(default)]
    pub request_count: u64,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default)]
    pub total_latency_ms: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessAccountStats {
    pub account_id: String,
    pub email: String,
    #[serde(default)]
    pub usage: CodexLocalAccessUsageStats,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessStatsWindow {
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub totals: CodexLocalAccessUsageStats,
    #[serde(default)]
    pub accounts: Vec<CodexLocalAccessAccountStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessUsageEvent {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub latency_ms: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub cached_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessStats {
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub totals: CodexLocalAccessUsageStats,
    #[serde(default)]
    pub accounts: Vec<CodexLocalAccessAccountStats>,
    #[serde(default)]
    pub daily: CodexLocalAccessStatsWindow,
    #[serde(default)]
    pub weekly: CodexLocalAccessStatsWindow,
    #[serde(default)]
    pub monthly: CodexLocalAccessStatsWindow,
    #[serde(default)]
    pub events: Vec<CodexLocalAccessUsageEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessState {
    pub collection: Option<CodexLocalAccessCollection>,
    pub running: bool,
    pub api_port_url: Option<String>,
    pub base_url: Option<String>,
    pub lan_base_url: Option<String>,
    pub model_ids: Vec<String>,
    pub last_error: Option<String>,
    pub member_count: usize,
    pub stats: CodexLocalAccessStats,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessTestFailure {
    pub title: String,
    pub stage: String,
    pub cause: String,
    pub suggestion: String,
    pub status: Option<u16>,
    pub model_id: Option<String>,
    pub detail: Option<String>,
    pub cli_output: Option<String>,
    pub gateway_output: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessTestResult {
    pub model_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub output: Option<String>,
    pub failure: Option<CodexLocalAccessTestFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessPortCleanupResult {
    pub killed_count: u32,
    pub state: CodexLocalAccessState,
}
