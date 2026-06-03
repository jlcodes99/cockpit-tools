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
    Custom,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CodexLocalAccessClientBaseUrlHost {
    #[serde(rename = "localhost")]
    Localhost,
    #[serde(rename = "127.0.0.1")]
    Ipv4Loopback,
}

impl Default for CodexLocalAccessClientBaseUrlHost {
    fn default() -> Self {
        Self::Localhost
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessImageGenerationMode {
    Enabled,
    ImagesOnly,
    Disabled,
}

impl Default for CodexLocalAccessImageGenerationMode {
    fn default() -> Self {
        Self::Enabled
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessGatewayMode {
    Legacy,
    Sidecar,
}

impl Default for CodexLocalAccessGatewayMode {
    fn default() -> Self {
        Self::Sidecar
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessRequestKind {
    Text,
    ImageGeneration,
    ImageEdit,
    Other,
}

impl Default for CodexLocalAccessRequestKind {
    fn default() -> Self {
        Self::Other
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexLocalAccessImageGenerationStatus {
    Unknown,
    Available,
    Unavailable,
    Disabled,
}

impl Default for CodexLocalAccessImageGenerationStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

fn default_access_scope_for_existing_config() -> CodexLocalAccessScope {
    CodexLocalAccessScope::Lan
}

fn default_restrict_free_accounts() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessCustomRoutingRule {
    pub account_id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_custom_routing_weight")]
    pub weight: u32,
}

fn default_custom_routing_weight() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessAccountModelRule {
    pub account_id: String,
    #[serde(default)]
    pub excluded_models: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessModelAlias {
    pub source_model: String,
    pub alias: String,
    #[serde(default)]
    pub fork: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessModelPricing {
    pub model_id: String,
    #[serde(default)]
    pub input_usd_per_million: f64,
    #[serde(default)]
    pub output_usd_per_million: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_usd_per_million: Option<f64>,
}

fn default_session_affinity_ttl_ms() -> i64 {
    60 * 60 * 1000
}

fn default_max_retry_interval_ms() -> u64 {
    3 * 1000
}

fn default_legacy_request_read_timeout_ms() -> u64 {
    60 * 1000
}

fn default_legacy_upstream_connect_timeout_ms() -> u64 {
    60 * 1000
}

fn default_legacy_stream_idle_timeout_ms() -> u64 {
    120 * 1000
}

fn default_legacy_stream_total_timeout_ms() -> u64 {
    300 * 1000
}

fn default_sidecar_stream_open_timeout_ms() -> u64 {
    60 * 1000
}

fn default_sidecar_stream_idle_timeout_ms() -> u64 {
    120 * 1000
}

fn default_sidecar_image_stream_open_timeout_ms() -> u64 {
    60 * 1000
}

fn default_sidecar_image_stream_idle_timeout_ms() -> u64 {
    180 * 1000
}

fn default_sidecar_stream_open_max_attempts() -> u8 {
    1
}

fn default_sidecar_stream_keepalive_seconds() -> u16 {
    15
}

fn default_websocket_connect_timeout_ms() -> u64 {
    30 * 1000
}

fn default_websocket_initial_message_timeout_ms() -> u64 {
    30 * 1000
}

fn default_websocket_idle_timeout_ms() -> u64 {
    5 * 60 * 1000
}

#[cfg(not(test))]
fn default_websocket_heartbeat_interval_ms() -> u64 {
    30 * 1000
}

#[cfg(test)]
fn default_websocket_heartbeat_interval_ms() -> u64 {
    25
}

fn default_upstream_send_retry_attempts() -> u8 {
    3
}

fn default_upstream_send_retry_base_delay_ms() -> u64 {
    200
}

fn default_upstream_send_retry_max_delay_ms() -> u64 {
    1200
}

fn default_single_account_status_retry_attempts() -> u8 {
    2
}

fn default_single_account_status_retry_base_delay_ms() -> u64 {
    300
}

fn default_single_account_status_retry_max_delay_ms() -> u64 {
    1500
}

fn default_sidecar_streaming_bootstrap_retries() -> u8 {
    1
}

fn default_timeout_preset_long_wait() -> String {
    "long_wait".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessTimeouts {
    #[serde(default = "default_legacy_request_read_timeout_ms")]
    pub legacy_request_read_timeout_ms: u64,
    #[serde(default = "default_legacy_upstream_connect_timeout_ms")]
    pub legacy_upstream_connect_timeout_ms: u64,
    #[serde(default = "default_legacy_stream_idle_timeout_ms")]
    pub legacy_stream_idle_timeout_ms: u64,
    #[serde(default = "default_legacy_stream_total_timeout_ms")]
    pub legacy_stream_total_timeout_ms: u64,
    #[serde(default = "default_sidecar_stream_open_timeout_ms")]
    pub sidecar_stream_open_timeout_ms: u64,
    #[serde(default = "default_sidecar_stream_idle_timeout_ms")]
    pub sidecar_stream_idle_timeout_ms: u64,
    #[serde(default = "default_sidecar_image_stream_open_timeout_ms")]
    pub sidecar_image_stream_open_timeout_ms: u64,
    #[serde(default = "default_sidecar_image_stream_idle_timeout_ms")]
    pub sidecar_image_stream_idle_timeout_ms: u64,
    #[serde(default = "default_sidecar_stream_open_max_attempts")]
    pub sidecar_stream_open_max_attempts: u8,
    #[serde(default = "default_sidecar_stream_keepalive_seconds")]
    pub sidecar_stream_keepalive_seconds: u16,
    #[serde(default = "default_websocket_connect_timeout_ms")]
    pub websocket_connect_timeout_ms: u64,
    #[serde(default = "default_websocket_initial_message_timeout_ms")]
    pub websocket_initial_message_timeout_ms: u64,
    #[serde(default = "default_websocket_idle_timeout_ms")]
    pub websocket_idle_timeout_ms: u64,
    #[serde(default = "default_websocket_heartbeat_interval_ms")]
    pub websocket_heartbeat_interval_ms: u64,
    #[serde(default = "default_upstream_send_retry_attempts")]
    pub upstream_send_retry_attempts: u8,
    #[serde(default = "default_upstream_send_retry_base_delay_ms")]
    pub upstream_send_retry_base_delay_ms: u64,
    #[serde(default = "default_upstream_send_retry_max_delay_ms")]
    pub upstream_send_retry_max_delay_ms: u64,
    #[serde(default = "default_single_account_status_retry_attempts")]
    pub single_account_status_retry_attempts: u8,
    #[serde(default = "default_single_account_status_retry_base_delay_ms")]
    pub single_account_status_retry_base_delay_ms: u64,
    #[serde(default = "default_single_account_status_retry_max_delay_ms")]
    pub single_account_status_retry_max_delay_ms: u64,
    #[serde(default = "default_sidecar_streaming_bootstrap_retries")]
    pub sidecar_streaming_bootstrap_retries: u8,
}

impl Default for CodexLocalAccessTimeouts {
    fn default() -> Self {
        Self {
            legacy_request_read_timeout_ms: default_legacy_request_read_timeout_ms(),
            legacy_upstream_connect_timeout_ms: default_legacy_upstream_connect_timeout_ms(),
            legacy_stream_idle_timeout_ms: default_legacy_stream_idle_timeout_ms(),
            legacy_stream_total_timeout_ms: default_legacy_stream_total_timeout_ms(),
            sidecar_stream_open_timeout_ms: default_sidecar_stream_open_timeout_ms(),
            sidecar_stream_idle_timeout_ms: default_sidecar_stream_idle_timeout_ms(),
            sidecar_image_stream_open_timeout_ms: default_sidecar_image_stream_open_timeout_ms(),
            sidecar_image_stream_idle_timeout_ms: default_sidecar_image_stream_idle_timeout_ms(),
            sidecar_stream_open_max_attempts: default_sidecar_stream_open_max_attempts(),
            sidecar_stream_keepalive_seconds: default_sidecar_stream_keepalive_seconds(),
            websocket_connect_timeout_ms: default_websocket_connect_timeout_ms(),
            websocket_initial_message_timeout_ms: default_websocket_initial_message_timeout_ms(),
            websocket_idle_timeout_ms: default_websocket_idle_timeout_ms(),
            websocket_heartbeat_interval_ms: default_websocket_heartbeat_interval_ms(),
            upstream_send_retry_attempts: default_upstream_send_retry_attempts(),
            upstream_send_retry_base_delay_ms: default_upstream_send_retry_base_delay_ms(),
            upstream_send_retry_max_delay_ms: default_upstream_send_retry_max_delay_ms(),
            single_account_status_retry_attempts: default_single_account_status_retry_attempts(),
            single_account_status_retry_base_delay_ms:
                default_single_account_status_retry_base_delay_ms(),
            single_account_status_retry_max_delay_ms:
                default_single_account_status_retry_max_delay_ms(),
            sidecar_streaming_bootstrap_retries: default_sidecar_streaming_bootstrap_retries(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessTimeoutPreset {
    pub id: String,
    pub name: String,
    pub timeouts: CodexLocalAccessTimeouts,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessApiKey {
    pub id: String,
    pub label: String,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_prefix: Option<String>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<i64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessCollection {
    pub enabled: bool,
    pub port: u16,
    pub api_key: String,
    #[serde(default)]
    pub api_keys: Vec<CodexLocalAccessApiKey>,
    #[serde(default = "default_access_scope_for_existing_config")]
    pub access_scope: CodexLocalAccessScope,
    #[serde(default)]
    pub client_base_url_host: CodexLocalAccessClientBaseUrlHost,
    #[serde(default)]
    pub image_generation_mode: CodexLocalAccessImageGenerationMode,
    #[serde(default)]
    pub gateway_mode: CodexLocalAccessGatewayMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_proxy_url: Option<String>,
    #[serde(default)]
    pub routing_strategy: CodexLocalAccessRoutingStrategy,
    #[serde(default)]
    pub custom_routing_rules: Vec<CodexLocalAccessCustomRoutingRule>,
    #[serde(default)]
    pub account_model_rules: Vec<CodexLocalAccessAccountModelRule>,
    #[serde(default)]
    pub model_aliases: Vec<CodexLocalAccessModelAlias>,
    #[serde(default)]
    pub model_pricings: Vec<CodexLocalAccessModelPricing>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
    #[serde(default)]
    pub session_affinity: bool,
    #[serde(default = "default_session_affinity_ttl_ms")]
    pub session_affinity_ttl_ms: i64,
    #[serde(default)]
    pub max_retry_credentials: u16,
    #[serde(default = "default_max_retry_interval_ms")]
    pub max_retry_interval_ms: u64,
    #[serde(default)]
    pub timeouts: CodexLocalAccessTimeouts,
    #[serde(default = "default_timeout_preset_long_wait")]
    pub active_timeout_preset_id: String,
    #[serde(default)]
    pub timeout_presets: Vec<CodexLocalAccessTimeoutPreset>,
    #[serde(default)]
    pub disable_cooling: bool,
    #[serde(default = "default_restrict_free_accounts")]
    pub restrict_free_accounts: bool,
    #[serde(default = "default_true")]
    pub debug_logs: bool,
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
    pub client_canceled_count: u64,
    #[serde(default)]
    pub upstream_response_failed_count: u64,
    #[serde(default)]
    pub stream_incomplete_count: u64,
    #[serde(default)]
    pub total_latency_ms: u64,
    #[serde(default)]
    pub text_request_count: u64,
    #[serde(default)]
    pub image_request_count: u64,
    #[serde(default)]
    pub image_generation_request_count: u64,
    #[serde(default)]
    pub image_edit_request_count: u64,
    #[serde(default)]
    pub image_generation_capability_failure_count: u64,
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
    #[serde(default)]
    pub estimated_cost_usd: f64,
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
pub struct CodexLocalAccessModelStats {
    pub model_id: String,
    #[serde(default)]
    pub usage: CodexLocalAccessUsageStats,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessApiKeyStats {
    pub api_key_id: String,
    #[serde(default)]
    pub label: String,
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
    #[serde(default)]
    pub models: Vec<CodexLocalAccessModelStats>,
    #[serde(default)]
    pub api_keys: Vec<CodexLocalAccessApiKeyStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessUsageEvent {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub api_key_id: String,
    #[serde(default)]
    pub api_key_label: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_mode: Option<CodexLocalAccessGatewayMode>,
    #[serde(default)]
    pub request_kind: CodexLocalAccessRequestKind,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub http_status: Option<u16>,
    #[serde(default)]
    pub error_category: String,
    #[serde(default)]
    pub error_message: String,
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
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub input_usd_per_million: f64,
    #[serde(default)]
    pub output_usd_per_million: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_usd_per_million: Option<f64>,
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
    pub models: Vec<CodexLocalAccessModelStats>,
    #[serde(default)]
    pub api_keys: Vec<CodexLocalAccessApiKeyStats>,
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
pub struct CodexLocalAccessUsageEventPage {
    pub events: Vec<CodexLocalAccessUsageEvent>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessAccountCooldown {
    pub model_id: String,
    pub next_retry_at: i64,
    pub remaining_ms: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessAccountHealth {
    pub account_id: String,
    pub email: String,
    pub available: bool,
    pub consecutive_failures: u32,
    pub last_success_at: Option<i64>,
    pub last_failure_at: Option<i64>,
    pub last_failure_status: Option<u16>,
    pub last_failure_category: Option<String>,
    pub last_failure_message: Option<String>,
    pub image_generation_status: CodexLocalAccessImageGenerationStatus,
    pub image_generation_checked_at: Option<i64>,
    pub cooldowns: Vec<CodexLocalAccessAccountCooldown>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessProfileAttachment {
    pub profile_dir: String,
    pub attached: bool,
    pub config_attached: bool,
    pub auth_attached: bool,
    pub model_provider: Option<String>,
    pub base_url: Option<String>,
    pub expected_base_url: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessState {
    pub collection: Option<CodexLocalAccessCollection>,
    pub running: bool,
    pub default_profile: Option<CodexLocalAccessProfileAttachment>,
    pub api_port_url: Option<String>,
    pub base_url: Option<String>,
    pub lan_base_url: Option<String>,
    pub model_ids: Vec<String>,
    pub model_pricing_presets: Vec<CodexLocalAccessModelPricing>,
    pub last_error: Option<String>,
    pub member_count: usize,
    pub stats: CodexLocalAccessStats,
    pub account_health: Vec<CodexLocalAccessAccountHealth>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLocalAccessChatResult {
    pub model_id: String,
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
