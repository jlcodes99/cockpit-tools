use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebuddyLocalAccessScope {
    Localhost,
    Lan,
}

impl Default for CodebuddyLocalAccessScope {
    fn default() -> Self {
        Self::Localhost
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebuddyLocalAccessImageGenerationMode {
    /// 图片生成关闭。因上游图片协议未实测，默认灰度关闭。
    Disabled,
    /// 仅图片请求放行（文本请求被拒绝）。
    ImagesOnly,
    /// 全量开启（文本 + 图片）。
    Enabled,
}

impl Default for CodebuddyLocalAccessImageGenerationMode {
    fn default() -> Self {
        Self::Disabled
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebuddyLocalAccessRequestKind {
    Text,
    ImageGeneration,
    ImageEdit,
    Other,
}

impl Default for CodebuddyLocalAccessRequestKind {
    fn default() -> Self {
        Self::Other
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebuddyLocalAccessImageGenerationStatus {
    Unknown,
    Available,
    Unavailable,
    Disabled,
}

impl Default for CodebuddyLocalAccessImageGenerationStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

/// 账号池调度策略（对齐 Codex 反代的调度选项）。
///
/// 注意：Go 侧路由层当前仅实现 `round-robin` 与 `fill-first` 两种底层策略，
/// 其余策略在编排层映射到已实现的策略（见 `prepare_sidecar_launch_config`），
/// 待 Go 侧补齐账号配额/订阅/到期元数据后再逐步落地。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodebuddyLocalAccessRoutingStrategy {
    /// 自动（默认）：round-robin + 会话亲和。
    Auto,
    /// 随机分散。
    Random,
    /// 固定首个账号。
    SingleAccount,
    /// 优先高配额。
    QuotaHighFirst,
    /// 优先低配额。
    QuotaLowFirst,
    /// 优先高订阅（plan）。
    PlanHighFirst,
    /// 优先低订阅（plan）。
    PlanLowFirst,
    /// 优先近到期。
    ExpirySoonFirst,
    /// 自定义。
    Custom,
}

impl Default for CodebuddyLocalAccessRoutingStrategy {
    fn default() -> Self {
        Self::Auto
    }
}

/// 自定义路由规则（custom 策略）：按账号设置优先级/权重/备份/偏好。
///
/// 语义对齐 Codex 反代的 custom routing：
/// - `priority` 越大越优先（先按 priority 降序分组）。
/// - 同 priority 组内按 `weight` 加权轮询。
/// - `is_preferred` 账号排最前，`is_backup` 账号排最后（兜底）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessCustomRoutingRule {
    pub account_id: String,
    pub priority: i32,
    pub weight: i32,
    pub is_backup: bool,
    pub is_preferred: bool,
}

fn default_max_concurrent_image_requests() -> u16 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessModelAlias {
    pub source_model: String,
    pub alias: String,
    #[serde(default)]
    pub fork: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessApiKey {
    pub id: String,
    pub name: String,
    pub key: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ids: Option<Vec<String>>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CodebuddyLocalAccessCollection {
    /// 服务总开关。
    pub enabled: bool,
    /// 本地监听端口（默认与 Codex 反代错开）。
    pub port: u16,
    /// 监听地址（127.0.0.1 / 0.0.0.0）。
    pub bind_host: String,
    /// 访问范围（仅本机 / 局域网）。
    pub scope: CodebuddyLocalAccessScope,
    /// 国际站（www.codebuddy.ai）账号 ID 列表。
    pub intl_account_ids: Vec<String>,
    /// 中国站（www.codebuddy.cn / www.workbuddy.cn）账号 ID 列表。
    pub cn_account_ids: Vec<String>,
    /// 模型别名映射。
    pub model_aliases: Vec<CodebuddyLocalAccessModelAlias>,
    /// 排除的模型 ID。
    pub excluded_models: Vec<String>,
    /// 是否输出调试日志。
    pub debug_logs: bool,
    /// 会话亲和。
    pub session_affinity: bool,
    pub session_affinity_ttl_ms: u64,
    /// 调度策略（账号池）。
    #[serde(default)]
    pub routing_strategy: CodebuddyLocalAccessRoutingStrategy,
    /// 自定义路由规则（仅当 routing_strategy == Custom 时生效）。
    #[serde(default)]
    pub custom_routing_rules: Vec<CodebuddyLocalAccessCustomRoutingRule>,
    /// 最大重试凭据数（切换账号重试上限）。
    pub max_retry_credentials: u16,
    /// 最大重试间隔（毫秒）。
    pub max_retry_interval_ms: u64,
    /// 是否禁用账号冷却。
    pub disable_cooling: bool,
    /// 请求超时（毫秒）。
    pub request_timeout_ms: u64,
    /// 客户端 API Key（供第三方客户端鉴权接入）。
    pub api_keys: Vec<CodebuddyLocalAccessApiKey>,
    /// 图片生成模式（Disabled / ImagesOnly / Enabled）。因上游图片协议未实测，默认 Disabled。
    #[serde(default)]
    pub image_generation_mode: CodebuddyLocalAccessImageGenerationMode,
    /// 单账号最大并发图片请求数。
    #[serde(default = "default_max_concurrent_image_requests")]
    pub max_concurrent_image_requests: u16,
}

impl Default for CodebuddyLocalAccessCollection {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 11435,
            bind_host: "127.0.0.1".to_string(),
            scope: CodebuddyLocalAccessScope::Localhost,
            intl_account_ids: Vec::new(),
            cn_account_ids: Vec::new(),
            model_aliases: Vec::new(),
            excluded_models: Vec::new(),
            debug_logs: false,
            session_affinity: true,
            session_affinity_ttl_ms: 30 * 60 * 1000,
            routing_strategy: CodebuddyLocalAccessRoutingStrategy::Auto,
            custom_routing_rules: Vec::new(),
            max_retry_credentials: 2,
            max_retry_interval_ms: 2_000,
            disable_cooling: false,
            request_timeout_ms: 120_000,
            api_keys: Vec::new(),
            image_generation_mode: CodebuddyLocalAccessImageGenerationMode::Disabled,
            max_concurrent_image_requests: default_max_concurrent_image_requests(),
        }
    }
}

/// 用量统计（聚合）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessUsageStats {
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
    pub total_credit: f64,
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
    /// prompt cache 命中 token 数（命中 = 读缓存，credit 低）。
    #[serde(default)]
    pub prompt_cache_hit_tokens: u64,
    /// prompt cache 未命中 token 数（未命中 = 普通输入）。
    #[serde(default)]
    pub prompt_cache_miss_tokens: u64,
    /// prompt cache 写入 token 数（后端通常恒为 0，写入隐式计入 miss）。
    #[serde(default)]
    pub prompt_cache_write_tokens: u64,
}

/// 单条请求日志。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessRequestLog {
    pub request_id: String,
    pub timestamp: i64,
    pub model: String,
    pub api_key_id: String,
    /// 处理该请求的上游账号 ID（由 usage 事件回填，供按账号聚合）。
    #[serde(default)]
    pub account_id: String,
    pub status: u16,
    pub success: bool,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub credit: f64,
    #[serde(default)]
    pub prompt_cache_hit_tokens: u64,
    #[serde(default)]
    pub prompt_cache_miss_tokens: u64,
    #[serde(default)]
    pub prompt_cache_write_tokens: u64,
    #[serde(default)]
    pub request_kind: CodebuddyLocalAccessRequestKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// 统计查询结果。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessStats {
    #[serde(default)]
    pub since: i64,
    #[serde(default)]
    pub totals: CodebuddyLocalAccessUsageStats,
    #[serde(default)]
    pub by_model: Vec<CodebuddyLocalAccessModelStats>,
    #[serde(default)]
    pub by_api_key: Vec<CodebuddyLocalAccessApiKeyStats>,
    #[serde(default)]
    pub by_account: Vec<CodebuddyLocalAccessAccountStats>,
    #[serde(default)]
    pub recent_logs: Vec<CodebuddyLocalAccessRequestLog>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessModelStats {
    pub model_id: String,
    #[serde(default)]
    pub usage: CodebuddyLocalAccessUsageStats,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessApiKeyStats {
    pub api_key_id: String,
    #[serde(default)]
    pub usage: CodebuddyLocalAccessUsageStats,
}

/// 按账号聚合的用量统计（账号池维度，含缓存命中率）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessAccountStats {
    pub account_id: String,
    #[serde(default)]
    pub usage: CodebuddyLocalAccessUsageStats,
}

/// 账号图片生成能力健康状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessAccountHealth {
    pub account_id: String,
    pub email: String,
    pub image_generation_status: CodebuddyLocalAccessImageGenerationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_generation_checked_at: Option<i64>,
}
