//! Shared data structures & Tauri IPC contracts for Antigravity local token usage.
//! Strictly separates API-reported `exact` token counts from heuristic `estimated` tokens
//! and guarantees zero double-counting of `thinking_tokens`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationKind {
    Main,
    Swarm,
    Subagent,
}

impl ConversationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Swarm => "swarm",
            Self::Subagent => "subagent",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientSourceKind {
    #[default]
    Desktop,
    Cli,
}

impl ClientSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Cli => "cli",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSourceKind {
    SqliteDb,
    RpcCache,
    TranscriptJsonl,
    SummaryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeGranularity {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DatePreset {
    #[serde(rename = "today")]
    Today,
    #[serde(rename = "7d")]
    Days7,
    #[serde(rename = "30d")]
    Days30,
    #[serde(rename = "month")]
    Month,
    #[serde(rename = "all")]
    All,
    #[serde(rename = "custom")]
    Custom,
}

/// Exact API-reported token breakdown.
/// Invariant:
///   `total_tokens = input_tokens + output_tokens + thinking_tokens + cache_read_tokens + cache_write_tokens`
/// where `output_tokens` is pure response output (excluding `thinking_tokens`), so `thinking_tokens`
/// is NEVER double-counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactTokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

impl ExactTokenTotals {
    pub fn add_assign(&mut self, delta: &ExactTokenTotals) {
        self.input_tokens = self.input_tokens.saturating_add(delta.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(delta.output_tokens);
        self.thinking_tokens = self.thinking_tokens.saturating_add(delta.thinking_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(delta.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(delta.cache_write_tokens);
        self.total_tokens = self.total_tokens.saturating_add(delta.total_tokens);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEstimatedTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Heuristic estimated token counts ONLY for turns/steps where explicit API usage
/// telemetry was absent. NEVER added into `ExactTokenTotals`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimatedTokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub estimated_turn_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsageRecord {
    pub step_index: usize,
    pub timestamp: Option<String>,
    pub model_id: String,
    pub model_display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_model_enum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    pub api_provider: String,
    pub is_exact: bool,
    pub source: DataSourceKind,
    pub status: i32,
    pub exact: ExactTokenTotals,
    pub estimated: TurnEstimatedTotals,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationModelSlice {
    pub model_id: String,
    pub model_display_name: String,
    pub api_provider: String,
    pub turn_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated_total_tokens: u64,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildConversationRef {
    pub conversation_id: String,
    pub title: String,
    pub agent_name: String,
    pub kind: ConversationKind,
    pub exact_total_tokens: u64,
    pub estimated_total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummaryItem {
    pub conversation_id: String,
    pub title: String,
    pub display_title: String,
    pub preview: String,
    pub kind: ConversationKind,
    #[serde(default)]
    pub client_source: ClientSourceKind,
    pub has_swarm_children: bool,
    pub agent_name: String,
    pub parent_conversation_id: String,
    pub parent_title: String,
    pub nesting_depth: usize,
    pub workspace_uri: String,
    pub workspace_path: String,
    pub workspace_name: String,
    pub project_id: String,
    pub status: String,
    pub created_at: Option<String>,
    pub last_modified_time: String,
    pub step_count: usize,
    pub turn_count: usize,
    pub exact_turn_count: usize,
    pub estimated_turn_count: usize,
    pub primary_model_id: String,
    pub primary_model_display_name: String,
    pub models: Vec<String>,
    pub model_display_names: Vec<String>,
    pub data_source: DataSourceKind,
    pub exact: ExactTokenTotals,
    pub estimated: EstimatedTokenTotals,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallSummary {
    pub tool_name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationContextAudit {
    pub peak_context_tokens: u64,
    pub latest_context_tokens: u64,
    pub context_limit_tokens: u64,
    pub peak_context_percent: f64,
    pub user_prompt_count: usize,
    pub assistant_turn_count: usize,
    pub tool_call_count: usize,
    pub command_execution_count: usize,
    pub file_read_count: usize,
    pub file_edit_count: usize,
    pub subagent_spawn_count: usize,
    pub compaction_count: usize,
    pub compaction_steps: Vec<usize>,
    pub top_tools: Vec<ToolCallSummary>,
    pub active_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub summary: ConversationSummaryItem,
    pub model_breakdown: Vec<ConversationModelSlice>,
    pub turns: Vec<TurnUsageRecord>,
    pub children: Vec<ChildConversationRef>,
    pub audit: ConversationContextAudit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsageSummary {
    pub model_id: String,
    pub model_display_name: String,
    pub api_provider: String,
    pub conversation_count: usize,
    pub turn_count: usize,
    pub exact_turn_count: usize,
    pub estimated_turn_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated_total_tokens: u64,
    pub share_percent: f64,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUsageSummary {
    pub workspace_key: String,
    pub workspace_path: String,
    pub workspace_name: String,
    pub conversation_count: usize,
    pub turn_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated_total_tokens: u64,
    pub share_percent: f64,
    pub estimated_cost_usd: f64,
    pub last_active_time: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KindUsageSummary {
    pub kind: ConversationKind,
    pub conversation_count: usize,
    pub turn_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated_total_tokens: u64,
    pub share_percent: f64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeBucketPoint {
    pub bucket_key: String,
    pub label: String,
    pub start_date: String,
    pub end_date: String,
    pub turn_count: usize,
    pub conversation_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated_total_tokens: u64,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeatmapDayPoint {
    pub date: String,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub cache_tokens: u64,
    pub turn_count: usize,
    pub conversation_count: usize,
    pub estimated_cost_usd: f64,
    /// 0 (none) ..= 4 (peak)
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanMetadata {
    pub data_dir: String,
    pub data_dir_exists: bool,
    pub scanned_at: String,
    pub scan_duration_ms: u64,
    pub total_db_files: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub rpc_cache_sessions_found: usize,
    pub brain_dirs_found: usize,
    #[serde(default)]
    pub cli_sessions_found: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModelOption {
    pub model_id: String,
    pub model_display_name: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogWorkspaceOption {
    pub workspace_key: String,
    pub workspace_name: String,
    pub workspace_path: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterOptionsCatalog {
    pub models: Vec<CatalogModelOption>,
    pub workspaces: Vec<CatalogWorkspaceOption>,
    pub min_date: Option<String>,
    pub max_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KpiSummary {
    pub exact: ExactTokenTotals,
    pub estimated: EstimatedTokenTotals,
    pub total_conversations: usize,
    pub main_conversations: usize,
    pub swarm_conversations: usize,
    pub subagent_conversations: usize,
    pub desktop_conversations: usize,
    pub cli_conversations: usize,
    pub total_steps: usize,
    pub total_turns: usize,
    pub exact_turns: usize,
    pub estimated_turns: usize,
    pub exact_coverage_percent: f64,
    pub active_days: usize,
    pub avg_exact_tokens_per_active_day: u64,
    pub avg_exact_tokens_per_conversation: u64,
    pub estimated_cost_usd: f64,
    pub cache_savings_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFilters {
    pub preset: DatePreset,
    pub from: Option<String>,
    pub to: Option<String>,
    pub granularity: TimeGranularity,
    pub model: String,
    pub workspace: String,
    /// `""`, `"main"`, `"swarm"`, or `"subagent"`
    pub kind: String,
    pub search: String,
    pub tz_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSummaryResponse {
    pub scan: ScanMetadata,
    pub filters: AppliedFilters,
    pub catalog: FilterOptionsCatalog,
    pub kpi: KpiSummary,
    pub time_series: Vec<TimeBucketPoint>,
    pub heatmap: Vec<HeatmapDayPoint>,
    pub by_model: Vec<ModelUsageSummary>,
    pub by_workspace: Vec<WorkspaceUsageSummary>,
    pub by_kind: Vec<KindUsageSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationSortField {
    LastModifiedTime,
    TotalTokens,
    InputTokens,
    OutputTokens,
    CombinedOutputTokens,
    ThinkingTokens,
    CacheReadTokens,
    EstimatedCostUsd,
    StepCount,
    TurnCount,
    Title,
    Kind,
    Workspace,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationListResponse {
    pub items: Vec<ConversationSummaryItem>,
    pub total_items: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
    pub sort: ConversationSortField,
    pub order: SortDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetailResponse {
    pub scan: ScanMetadata,
    pub conversation: ConversationDetail,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityRehydrateStatus {
    pub enabled: bool,
    pub script_installed: bool,
    pub python_available: bool,
    pub python_command: Option<String>,
    pub python_version: Option<String>,
    pub hooks_config_path: String,
    pub script_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AntigravityTokenUsageSettings {
    pub custom_data_dir: Option<String>,
    pub effective_data_dir: String,
    pub default_data_dir: String,
    pub is_custom_data_dir: bool,
    #[serde(default)]
    pub rehydrate: AntigravityRehydrateStatus,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardQueryRequest {
    pub preset: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub granularity: Option<String>,
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub kind: Option<String>,
    pub search: Option<String>,
    pub tz_offset_minutes: Option<i32>,
    pub sort: Option<String>,
    pub order: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardBundleResponse {
    pub summary: DashboardSummaryResponse,
    pub conversations: ConversationListResponse,
    pub settings: AntigravityTokenUsageSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTurnGroup {
    pub minute_slot: Option<i64>,
    pub max_ts_ms: Option<i64>,
    pub model_id: String,
    pub model_display_name: String,
    pub api_provider: String,
    pub exact_turn_count: usize,
    pub estimated_turn_count: usize,
    pub exact: ExactTokenTotals,
    pub estimated: TurnEstimatedTotals,
}

#[derive(Debug, Clone)]
pub struct ScannedConversation {
    pub summary: ConversationSummaryItem,
    pub last_modified_ms: Option<i64>,
    pub created_at_ms: Option<i64>,
    pub model_hint: Option<String>,
    pub turn_groups: Vec<ResolvedTurnGroup>,
    pub model_breakdown: Vec<ConversationModelSlice>,
    pub children: Vec<ChildConversationRef>,
}

#[derive(Debug, Clone)]
pub struct ScanResult {
    pub scan: ScanMetadata,
    pub data_dir: std::path::PathBuf,
    pub cli_data_dir: Option<std::path::PathBuf>,
    pub dynamic_enum_map: HashMap<String, String>,
    pub conversations: Vec<ScannedConversation>,
    pub index_by_id: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct AggregatedDashboardBundle {
    pub summary: DashboardSummaryResponse,
    pub filtered_conversations: Vec<ConversationSummaryItem>,
}
