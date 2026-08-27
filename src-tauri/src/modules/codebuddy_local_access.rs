use crate::models::codebuddy::CodebuddyAccount;
use crate::models::codebuddy_local_access::{
    CodebuddyLocalAccessAccountCooldown, CodebuddyLocalAccessAccountHealth,
    CodebuddyLocalAccessApiKey, CodebuddyLocalAccessCollection,
    CodebuddyLocalAccessCustomRoutingRule, CodebuddyLocalAccessImageGenerationMode,
    CodebuddyLocalAccessImageGenerationStatus, CodebuddyLocalAccessRequestKind,
    CodebuddyLocalAccessRequestLog, CodebuddyLocalAccessRoutingStrategy,
    CodebuddyLocalAccessScope, CodebuddyLocalAccessStats, CodebuddyLocalAccessUsageStats,
    CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED, CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE,
    CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED,
};
use crate::modules::atomic_write::write_string_atomic;
use crate::modules::{account, codebuddy_account, codebuddy_cn_account, logger};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::Emitter;
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::Mutex as TokioMutex;

const CODEBUDDY_LOCAL_ACCESS_FILE: &str = "codebuddy_local_access.json";
const CODEBUDDY_LOCAL_ACCESS_SIDECAR_DIR: &str = "codebuddy_local_access_sidecar";
const CODEBUDDY_SIDECAR_CONFIG_FILE: &str = "config.json";
const CODEBUDDY_SIDECAR_MANIFEST_FILE: &str = "manifest.json";
const CODEBUDDY_SIDECAR_AUTHS_DIR: &str = "auths";
const CODEBUDDY_SIDECAR_BIN_NAME: &str = "cockpit-cliproxy";

const REGION_INTL: &str = "intl";
const REGION_CN: &str = "cn";
const BASE_URL_INTL: &str = "https://www.codebuddy.ai";
const BASE_URL_CN: &str = "https://copilot.tencent.com";

/// CodeBuddy 图片生成占位模型 ID（与 Go 侧 `defaultImagesToolModel` 的 codebuddy 分支一致）。
/// 上游图片协议尚未实测，该 ID 为占位，待官方确认后替换。
const CODEBUDDY_IMAGE_MODEL_ID: &str = "codebuddy-image-1";

/// CodeBuddy 订阅可用的模型 ID（与 Go 侧 registry 模型清单保持一致）。
/// 该清单来自官方 WorkBuddy/CodeBuddy 客户端内置模型目录（去重全集），
/// sidecar 启动时还会从本机客户端 app.asar 动态同步覆盖。
const CODEBUDDY_MODEL_IDS: &[&str] = &[
    "auto",
    "deepseek-chat",
    "deepseek-reasoner",
    "deepseek-v4-flash",
    "deepseek-v4-flash-202605",
    "deepseek-v4-pro",
    "deepseek-v4-pro-202606",
    "gemini-2.5-flash",
    "gemini-2.5-flash-lite",
    "gemini-2.5-pro",
    "gemini-3-flash-preview",
    "gemini-3.1-flash",
    "gemini-3.1-flash-lite",
    "gemini-3.1-pro-preview",
    "gemini-3.5-flash",
    "glm-4.6v",
    "glm-4.7",
    "glm-5",
    "glm-5-turbo",
    "glm-5.1",
    "glm-5.2",
    "gpt-4.1",
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-5",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5.3-codex",
    "gpt-5.4",
    "hunyuan-2.0-instruct",
    "hunyuan-2.0-thinking",
    "hunyuan-t1",
    "hunyuan-turbos",
    "hy3",
    "hy3-preview",
    "kimi-k2-0711-preview",
    "kimi-k2-0905-preview",
    "kimi-k2-thinking",
    "kimi-k2-thinking-turbo",
    "kimi-k2-turbo-preview",
    "kimi-k2.5",
    "kimi-k2.6",
    "kimi-k2.7-code",
    "kimi-k2.7-code-highspeed",
    "kimi-k3",
    "minimax-m2.5",
    "minimax-m2.7",
    "minimax-m3",
    "MiniMax-M2",
    "MiniMax-M2.1",
    "MiniMax-M2.1-highspeed",
    "MiniMax-M2.5",
    "MiniMax-M2.5-highspeed",
    "o1",
    "o3",
    "o3-mini",
    "tc-code-latest",
    CODEBUDDY_IMAGE_MODEL_ID,
];

/// 视觉代理层模式常量。
const CODEBUDDY_VISION_MODE_OFF: &str = "off";
const CODEBUDDY_VISION_MODE_AGENTIC: &str = "agentic";

/// 视觉子代理最大迭代轮次默认值（deepseek 最多调用这么多次视觉模型）。
const CODEBUDDY_VISION_MAX_ROUNDS_DEFAULT: i64 = 3;

/// 视觉代理层模式决策（环境变量可覆盖）：
/// 1. 环境变量 `CODEBUDDY_VISION_MODE` 显式设置时优先（off/routing/preprocess/agentic）；
/// 2. 否则由 UI 开关 `collection.vision_tool_enabled` 决定：true → agentic，false → off。
///
/// agentic 模式让纯文本模型（如 deepseek-v4-pro）在收到图片时，通过服务端
/// tool-calling 循环自主调用混元视觉模型（hy3-preview）"看图"，弥补纯文本
/// 模型无法理解图片的缺陷。
fn codebuddy_vision_mode(vision_tool_enabled: bool) -> String {
    if let Ok(explicit) = std::env::var("CODEBUDDY_VISION_MODE") {
        let trimmed = explicit.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    if vision_tool_enabled {
        CODEBUDDY_VISION_MODE_AGENTIC.to_string()
    } else {
        CODEBUDDY_VISION_MODE_OFF.to_string()
    }
}

/// 视觉子代理最大迭代轮次（环境变量 `CODEBUDDY_VISION_MAX_ROUNDS` 覆盖，默认 3）。
fn codebuddy_vision_max_rounds() -> i64 {
    std::env::var("CODEBUDDY_VISION_MAX_ROUNDS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(CODEBUDDY_VISION_MAX_ROUNDS_DEFAULT)
}

/// 最近一次向前端推送统计变更事件的时间戳（毫秒），用于防抖：
/// 同一请求的 request_completed + usage 事件几乎同时到达，500ms 内去重。
static LAST_STATS_EMIT_MS: AtomicI64 = AtomicI64::new(0);

/// 统计变更事件名（前端 `listen` 监听）。
pub const CODEBUDDY_STATS_CHANGED_EVENT: &str = "codebuddy-local-access-stats-changed";

/// 通知前端统计已更新（事件驱动刷新）。带 500ms 防抖，避免高频 emit；
/// 应用尚未就绪（AppHandle 为 None）时静默跳过。
fn emit_stats_changed() {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let last = LAST_STATS_EMIT_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last) < 500 {
        return;
    }
    LAST_STATS_EMIT_MS.store(now_ms, Ordering::Relaxed);
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit(CODEBUDDY_STATS_CHANGED_EVENT, ());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarLaunchConfig {
    config_path: PathBuf,
    manifest_path: PathBuf,
    fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessAccountOption {
    pub id: String,
    pub email: String,
    pub region: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// 计划徽章 CSS 类（K12 / TEAM / FREE / ...），供前端卡片渲染。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_class: Option<String>,
    /// 剩余 credits（来自 quota_raw.CapacityRemain / CapacityRemainPrecise）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_remain: Option<f64>,
    /// 总容量 credits（来自 quota_raw.CapacityNum，登录时刷新）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_total: Option<f64>,
    /// 计划过期时间戳（毫秒），来自 CycleEndTime。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_expiry_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodebuddyLocalAccessState {
    pub collection: CodebuddyLocalAccessCollection,
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub intl_accounts: Vec<CodebuddyLocalAccessAccountOption>,
    pub cn_accounts: Vec<CodebuddyLocalAccessAccountOption>,
    pub base_url: String,
    /// 局域网模式下供外部设备使用的 URL（如 http://192.168.1.10:11435）。
    /// 仅在 scope=lan 且能解析到本机局域网 IP 时返回。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lan_base_url: Option<String>,
    /// 选中账号的图片生成能力健康状态（镜像 codex 的 image_generation_status）。
    pub account_health: Vec<CodebuddyLocalAccessAccountHealth>,
}

/// 运行时账号健康跟踪状态（由 auth_result / auth_selected 事件驱动更新，
/// 对齐 Codex 反代的 AccountHealth 语义：连续失败 / 冷却 / 可用性）。
#[derive(Debug, Clone)]
struct AccountHealthState {
    available: bool,
    consecutive_failures: u32,
    last_failure_at: Option<i64>,
    last_failure_category: Option<String>,
    cooldowns: Vec<CodebuddyLocalAccessAccountCooldown>,
}

impl Default for AccountHealthState {
    fn default() -> Self {
        Self {
            available: true,
            consecutive_failures: 0,
            last_failure_at: None,
            last_failure_category: None,
            cooldowns: Vec::new(),
        }
    }
}

/// 事件映射表的容量上限（requestId → accountId / 流结束原因）。
/// 请求完成后即清理；超限时整体重置，防止泄漏。
const MAX_EVENT_MAP_ENTRIES: usize = 4096;

#[derive(Default)]
struct CodebuddyGatewayRuntime {
    loaded: bool,
    collection: CodebuddyLocalAccessCollection,
    running: bool,
    actual_port: Option<u16>,
    child: Option<Child>,
    fingerprint: Option<String>,
    last_error: Option<String>,
    /// 捕获自 sidecar stdout 的请求日志（按 requestId 索引，保留顺序）。
    request_logs: Vec<CodebuddyLocalAccessRequestLog>,
    /// 请求日志的 requestId 索引，用于 usage 事件回填 token 统计。
    request_index: HashMap<String, usize>,
    /// 统计起始时间戳（最近一次清空统计）。
    stats_since: i64,
    /// 账号健康跟踪状态（accountId → 状态）。
    account_health: HashMap<String, AccountHealthState>,
    /// requestId → 处理请求的账号 ID（auth_selected / auth_result 提供，
    /// 供失败请求的账号归因回填）。
    request_account: HashMap<String, String>,
    /// requestId → 流结束原因（stream_completed 提供；通常先于
    /// request_completed 到达，用于标记 stream_incomplete / client_canceled）。
    request_stream_end: HashMap<String, String>,
}

const MAX_REQUEST_LOGS: usize = 500;

static GATEWAY_RUNTIME: OnceLock<TokioMutex<CodebuddyGatewayRuntime>> = OnceLock::new();

fn gateway_runtime() -> &'static TokioMutex<CodebuddyGatewayRuntime> {
    GATEWAY_RUNTIME.get_or_init(|| TokioMutex::new(CodebuddyGatewayRuntime::default()))
}

fn state_file_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEBUDDY_LOCAL_ACCESS_FILE))
}

fn sidecar_base_dir() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(CODEBUDDY_LOCAL_ACCESS_SIDECAR_DIR))
}

fn sidecar_config_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEBUDDY_SIDECAR_CONFIG_FILE)
}

fn sidecar_manifest_path(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEBUDDY_SIDECAR_MANIFEST_FILE)
}

fn sidecar_auths_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(CODEBUDDY_SIDECAR_AUTHS_DIR)
}

fn sidecar_auth_file_name(account_id: &str) -> String {
    let safe: String = account_id
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let safe = if safe.trim_matches('_').is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        safe
    };
    format!("{safe}.json")
}

fn load_collection() -> CodebuddyLocalAccessCollection {
    let path = match state_file_path() {
        Ok(path) => path,
        Err(_) => return CodebuddyLocalAccessCollection::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => CodebuddyLocalAccessCollection::default(),
    }
}

fn save_collection(collection: &CodebuddyLocalAccessCollection) -> Result<(), String> {
    let path = state_file_path()?;
    let content = serde_json::to_string_pretty(collection)
        .map_err(|e| format!("序列化 CodeBuddy 反代配置失败: {e}"))?;
    write_string_atomic(&path, &content)
}

fn effective_bind_host(collection: &CodebuddyLocalAccessCollection) -> String {
    if collection.scope == CodebuddyLocalAccessScope::Lan {
        "0.0.0.0".to_string()
    } else {
        let host = collection.bind_host.trim();
        if host.is_empty() {
            "127.0.0.1".to_string()
        } else {
            host.to_string()
        }
    }
}

fn account_option(account: &CodebuddyAccount, region: &str) -> CodebuddyLocalAccessAccountOption {
    let meta = account_routing_metadata(account);
    let plan_class = Some(plan_type_class(account.payment_type.as_deref()));
    let (quota_remain, quota_total) = account
        .quota_raw
        .as_ref()
        .and_then(|q| q.get("userResource")?.get("data")?.get("Response")?.get("Data")?.get("Accounts")?.as_array()?.first().cloned())
        .map(|first| {
            let remain = first
                .get("CapacityRemain")
                .and_then(|v| v.as_f64())
                .or_else(|| first.get("CapacityRemainPrecise").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()));
            let total = first.get("CapacityNum").and_then(|v| v.as_f64());
            (remain, total)
        })
        .unwrap_or((None, None));
    CodebuddyLocalAccessAccountOption {
        id: account.id.clone(),
        email: account.email.clone(),
        region: region.to_string(),
        uid: account.uid.clone(),
        enterprise_id: account.enterprise_id.clone(),
        plan_type: account.plan_type.clone(),
        plan_class,
        quota_remain,
        quota_total,
        subscription_expiry_ms: meta.subscription_expiry_ms,
    }
}

/// `payment_type` → CSS 类名（与 Codex 风格对齐：K12 / TEAM / FREE / PLUS / PRO / ENTERPRISE）。
fn plan_type_class(payment_type: Option<&str>) -> String {
    let raw = payment_type.unwrap_or("").trim().to_ascii_lowercase();
    if raw.is_empty() {
        return "free".to_string();
    }
    match raw.as_str() {
        "free" => "free".to_string(),
        "plus" | "会员" => "plus".to_string(),
        "team" | "pro" | "专业版" => "team".to_string(),
        "k12" => "k12".to_string(),
        "enterprise" | "团队版" | "企业版" => "enterprise".to_string(),
        _ => "free".to_string(),
    }
}

fn list_intl_accounts() -> Vec<CodebuddyLocalAccessAccountOption> {
    codebuddy_account::list_accounts()
        .iter()
        .map(|a| account_option(a, REGION_INTL))
        .collect()
}

fn list_cn_accounts() -> Vec<CodebuddyLocalAccessAccountOption> {
    codebuddy_cn_account::list_accounts()
        .iter()
        .map(|a| account_option(a, REGION_CN))
        .collect()
}

fn auth_json_for_account(account: &CodebuddyAccount, region: &str, custom_rules: &[CodebuddyLocalAccessCustomRoutingRule]) -> Value {
    let base_url = if region == REGION_CN {
        BASE_URL_CN
    } else {
        BASE_URL_INTL
    };
    let meta = account_routing_metadata(account);
    let mut obj = json!({
        "type": "codebuddy",
        "access_token": account.access_token.clone(),
        "refresh_token": account.refresh_token.clone().unwrap_or_default(),
        "uid": account.uid.clone().unwrap_or_default(),
        "enterprise_id": account.enterprise_id.clone().unwrap_or_default(),
        "domain": account.domain.clone().unwrap_or_default(),
        "base_url": base_url,
        "region": region,
        "email": account.email.clone(),
    });
    if let Some(map) = obj.as_object_mut() {
        if let Some(v) = meta.quota_remain {
            map.insert("quota_remain".to_string(), json!(v));
        }
        if let Some(v) = meta.plan_rank {
            map.insert("plan_rank".to_string(), json!(v));
        }
        if let Some(v) = meta.subscription_expiry_ms {
            map.insert("subscription_expiry_ms".to_string(), json!(v));
        }
        if let Some(v) = meta.payment_type {
            map.insert("payment_type".to_string(), json!(v));
        }
        // 自定义路由规则（custom 策略）：注入到 auth 顶层，Go 侧 customSelector 读取。
        if let Some(rule) = custom_rules.iter().find(|r| r.account_id == account.id) {
            map.insert("routing_priority".to_string(), json!(rule.priority));
            map.insert("routing_weight".to_string(), json!(rule.weight.max(1)));
            map.insert("routing_is_backup".to_string(), json!(rule.is_backup));
            map.insert("routing_is_preferred".to_string(), json!(rule.is_preferred));
        }
    }
    obj
}

/// 从账号元数据提取调度策略所需的配额/订阅/到期信息。
/// 配额来源：`quota_raw.userResource.data.Response.Data.Accounts[0]`。
struct AccountRoutingMetadata {
    quota_remain: Option<f64>,
    plan_rank: Option<i64>,
    subscription_expiry_ms: Option<i64>,
    payment_type: Option<String>,
}

fn account_routing_metadata(account: &CodebuddyAccount) -> AccountRoutingMetadata {
    let payment_type = account.payment_type.clone();
    let plan_rank = payment_type
        .as_deref()
        .map(|p| plan_type_rank(p))
        .unwrap_or(0);

    let (quota_remain, subscription_expiry_ms) = account
        .quota_raw
        .as_ref()
        .and_then(|q| q.get("userResource")?.get("data")?.get("Response")?.get("Data")?.get("Accounts")?.as_array()?.first().cloned())
        .map(|first| {
            let remain = first
                .get("CapacityRemain")
                .and_then(|v| v.as_f64())
                .or_else(|| first.get("CapacityRemainPrecise").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok()));
            let expiry = first
                .get("CycleEndTime")
                .and_then(|v| v.as_str())
                .and_then(parse_cycle_end_time_ms);
            (remain, expiry)
        })
        .unwrap_or((None, None));

    AccountRoutingMetadata {
        quota_remain,
        plan_rank: Some(plan_rank),
        subscription_expiry_ms,
        payment_type,
    }
}

/// 订阅类型 → 等级数值（越大越优先，供 plan_high_first / plan_low_first 使用）。
fn plan_type_rank(payment_type: &str) -> i64 {
    match payment_type.trim().to_ascii_lowercase().as_str() {
        "enterprise" | "团队版" | "企业版" => 3,
        "team" | "pro" | "专业版" => 2,
        "plus" | "会员" => 1,
        _ => 0, // free 及未知类型
    }
}

/// 解析 `CycleEndTime`（格式 `2026-08-31 23:59:59`）为 Unix 毫秒时间戳。
fn parse_cycle_end_time_ms(s: &str) -> Option<i64> {
    let normalized = s.trim().replace(' ', "T");
    chrono::NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp() * 1000)
}

fn sidecar_binary_path() -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("读取当前程序路径失败: {e}"))?;
    let parent = exe
        .parent()
        .ok_or_else(|| format!("当前程序路径缺少父目录: {}", exe.display()))?;

    let mut names: Vec<String> = if cfg!(target_os = "windows") {
        vec![
            format!("{CODEBUDDY_SIDECAR_BIN_NAME}.exe"),
            format!("{CODEBUDDY_SIDECAR_BIN_NAME}-{}.exe", env!("COCKPIT_RUST_TARGET")),
        ]
    } else {
        vec![
            CODEBUDDY_SIDECAR_BIN_NAME.to_string(),
            format!("{CODEBUDDY_SIDECAR_BIN_NAME}-{}", env!("COCKPIT_RUST_TARGET")),
        ]
    };
    if !cfg!(target_os = "windows") {
        names.push(format!("{CODEBUDDY_SIDECAR_BIN_NAME}.exe"));
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dev_sidecar_dir = manifest_dir.join("../sidecars/cockpit-cliproxy/bin");

    for dir in [dev_sidecar_dir, parent.to_path_buf()] {
        for name in &names {
            let path = dir.join(name);
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }
    if let Some(contents_dir) = parent.parent() {
        for name in &names {
            let path = contents_dir.join("Resources").join(name);
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }

    candidates
        .iter()
        .find(|path| path.exists())
        .cloned()
        .ok_or_else(|| {
            format!(
                "CodeBuddy 反代 sidecar 二进制不存在，已检查: {}。请重新构建应用。",
                candidates
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn config_fingerprint(config: &str, manifest: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(config.as_bytes());
    hasher.update(b"\0");
    hasher.update(manifest.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn prepare_sidecar_launch_config(
    collection: &CodebuddyLocalAccessCollection,
    base_dir: &Path,
) -> Result<SidecarLaunchConfig, String> {
    let auths_dir = sidecar_auths_dir(base_dir);
    std::fs::create_dir_all(&auths_dir)
        .map_err(|e| format!("创建 CodeBuddy 反代认证目录失败: {e}"))?;

    let mut expected_auth_files = HashSet::new();
    let mut manifest_accounts = Vec::new();

    // 国际站账号
    for account_id in &collection.intl_account_ids {
        let Some(account) = codebuddy_account::load_account(account_id) else {
            logger::log_warn(&format!(
                "[CodeBuddyLocalAccess] 跳过不存在的国际站账号: account_id={account_id}"
            ));
            continue;
        };
        write_auth_file(&auths_dir, &account, REGION_INTL, &collection.custom_routing_rules, &mut expected_auth_files)?;
        manifest_accounts.push(json!({
            "id": account.id.clone(),
            "email": account.email.clone(),
            "authId": sidecar_auth_file_name(&account.id),
            "authKind": "oauth",
        }));
    }

    // 中国站账号
    for account_id in &collection.cn_account_ids {
        let Some(account) = codebuddy_cn_account::load_account(account_id) else {
            logger::log_warn(&format!(
                "[CodeBuddyLocalAccess] 跳过不存在的中国站账号: account_id={account_id}"
            ));
            continue;
        };
        write_auth_file(&auths_dir, &account, REGION_CN, &collection.custom_routing_rules, &mut expected_auth_files)?;
        manifest_accounts.push(json!({
            "id": account.id.clone(),
            "email": account.email.clone(),
            "authId": sidecar_auth_file_name(&account.id),
            "authKind": "oauth",
        }));
    }

    remove_stale_auth_files(&auths_dir, &expected_auth_files)?;

    // 图片生成关闭时，从模型清单中隐藏图片模型（镜像 codex 的 /v1/models 可见性规则）。
    let image_models_enabled =
        collection.image_generation_mode != CodebuddyLocalAccessImageGenerationMode::Disabled;
    let model_ids: Vec<String> = CODEBUDDY_MODEL_IDS
        .iter()
        .map(|id| id.to_string())
        .filter(|id| !collection.excluded_models.contains(id))
        .filter(|id| {
            image_models_enabled || *id != CODEBUDDY_IMAGE_MODEL_ID
        })
        .collect();

    let manifest = json!({
        "apiKeys": manifest_api_keys(collection),
        "accounts": manifest_accounts,
        "modelIds": model_ids,
        "modelAliases": collection.model_aliases.iter().map(|alias| json!({
            "sourceModel": alias.source_model.clone(),
            "alias": alias.alias.clone(),
            "fork": alias.fork,
        })).collect::<Vec<_>>(),
        "excludedModels": collection.excluded_models.clone(),
        "routingStrategy": routing_strategy_name(collection.routing_strategy),
        "debugLogs": collection.debug_logs,
        "immediateSseResponse": collection.immediate_sse_response,
        "imageGenerationMode": match collection.image_generation_mode {
            CodebuddyLocalAccessImageGenerationMode::Disabled => "disabled",
            CodebuddyLocalAccessImageGenerationMode::ImagesOnly => "images_only",
            CodebuddyLocalAccessImageGenerationMode::Enabled => "enabled",
        },
        "imageModels": [CODEBUDDY_IMAGE_MODEL_ID],
        "visionMode": codebuddy_vision_mode(collection.vision_tool_enabled),
    });

    let mut config = serde_json::Map::new();
    config.insert("host".to_string(), json!(effective_bind_host(collection)));
    config.insert("port".to_string(), json!(collection.port));
    config.insert(
        "auth-dir".to_string(),
        json!(auths_dir.to_string_lossy().to_string()),
    );
    config.insert("debug".to_string(), json!(collection.debug_logs));
    config.insert(
        "api-keys".to_string(),
        json!(client_api_keys(collection)),
    );
    config.insert("request-log".to_string(), json!(false));
    config.insert("logging-to-file".to_string(), json!(false));
    config.insert("commercial-mode".to_string(), json!(true));
    config.insert("ws-auth".to_string(), json!(true));
    // CodeBuddy token 由 Go 侧 executor 自动刷新，因此不禁用 auth-auto-refresh。
    config.insert("disable-auth-auto-refresh".to_string(), json!(false));
    config.insert(
        "routing".to_string(),
        json!({
            // 策略名直接使用 snake_case 传入，Go 侧 normalizeStrategy 负责映射到
            // 具体 selector（round-robin / fill-first / random / quota_* / plan_* /
            // expiry_soon_first）。
            "strategy": routing_strategy_name(collection.routing_strategy),
            "session-affinity": collection.session_affinity,
            "session-affinity-ttl": collection.session_affinity_ttl_ms,
        }),
    );
    config.insert(
        "max-retry-credentials".to_string(),
        json!(collection.max_retry_credentials as i32),
    );
    config.insert(
        "max-retry-interval".to_string(),
        json!(((collection.max_retry_interval_ms + 999) / 1000) as i32),
    );
    config.insert(
        "disable-cooling".to_string(),
        json!(collection.disable_cooling),
    );
    config.insert(
        "image-generation-mode".to_string(),
        json!(match collection.image_generation_mode {
            CodebuddyLocalAccessImageGenerationMode::Disabled => "disabled",
            CodebuddyLocalAccessImageGenerationMode::ImagesOnly => "images_only",
            CodebuddyLocalAccessImageGenerationMode::Enabled => "enabled",
        }),
    );
    config.insert(
        "max-concurrent-image-requests".to_string(),
        json!(collection.max_concurrent_image_requests as i32),
    );
    if !collection.excluded_models.is_empty() {
        config.insert(
            "oauth-excluded-models".to_string(),
            json!({ "codebuddy": collection.excluded_models.clone() }),
        );
    }
    if !collection.model_aliases.is_empty() {
        config.insert(
            "oauth-model-alias".to_string(),
            json!({ "codebuddy": collection.model_aliases.iter().map(|alias| json!({
                "sourceModel": alias.source_model.clone(),
                "alias": alias.alias.clone(),
                "fork": alias.fork,
            })).collect::<Vec<_>>() }),
        );
    }

    // 视觉代理层配置。模式由 UI 开关或环境变量决定：
    //   CODEBUDDY_VISION_MODE    off | routing | preprocess | agentic（显式覆盖优先）
    //   CODEBUDDY_VISION_MODEL   视觉引擎模型（默认 hy3-preview）
    //   CODEBUDDY_VISION_MAX_ROUNDS   agentic 模式最大迭代轮次（默认 3）
    //   CODEBUDDY_VISION_PREPROCESS_PROMPT  预处理模式的自定义提示词（可选）
    let vision_mode = codebuddy_vision_mode(collection.vision_tool_enabled);
    let vision_model =
        std::env::var("CODEBUDDY_VISION_MODEL").unwrap_or_else(|_| "hy3-preview".to_string());
    let vision_preprocess_prompt =
        std::env::var("CODEBUDDY_VISION_PREPROCESS_PROMPT").unwrap_or_default();
    let mut vision_cfg = json!({
        "mode": vision_mode,
        "model": vision_model,
        "max-tool-rounds": codebuddy_vision_max_rounds(),
    });
    if !vision_preprocess_prompt.is_empty() {
        vision_cfg["preprocess-prompt"] = json!(vision_preprocess_prompt);
    }
    config.insert("codebuddy-vision".to_string(), vision_cfg);

    let config_path = sidecar_config_path(base_dir);
    let manifest_path = sidecar_manifest_path(base_dir);
    let config_content = serde_json::to_string_pretty(&Value::Object(config))
        .map_err(|e| format!("序列化 CodeBuddy sidecar 配置失败: {e}"))?;
    let manifest_content = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("序列化 CodeBuddy sidecar manifest 失败: {e}"))?;
    let fingerprint = config_fingerprint(&config_content, &manifest_content);

    write_string_atomic(&config_path, &config_content)?;
    write_string_atomic(&manifest_path, &manifest_content)?;

    Ok(SidecarLaunchConfig {
        config_path,
        manifest_path,
        fingerprint,
    })
}

fn write_auth_file(
    auths_dir: &Path,
    account: &CodebuddyAccount,
    region: &str,
    custom_rules: &[CodebuddyLocalAccessCustomRoutingRule],
    expected: &mut HashSet<String>,
) -> Result<(), String> {
    let file_name = sidecar_auth_file_name(&account.id);
    let auth_path = auths_dir.join(&file_name);
    expected.insert(file_name);
    let auth_json = auth_json_for_account(account, region, custom_rules);
    let content = serde_json::to_string_pretty(&auth_json)
        .map_err(|e| format!("序列化 CodeBuddy 认证失败: {e}"))?;
    write_string_atomic(&auth_path, &content)
}

fn remove_stale_auth_files(auths_dir: &Path, expected: &HashSet<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(auths_dir)
        .map_err(|e| format!("读取 CodeBuddy 认证目录失败: {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !expected.contains(name) {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

fn manifest_api_keys(collection: &CodebuddyLocalAccessCollection) -> Vec<Value> {
    collection
        .api_keys
        .iter()
        .map(|key| {
            let mut entry = json!({
                "id": key.id.clone(),
                "label": key.name.clone(),
                "key": key.key.clone(),
                "enabled": key.enabled,
                "responsesWebsockets": collection.responses_websockets_enabled,
            });
            if let Some(account_ids) = &key.account_ids {
                if !account_ids.is_empty() {
                    entry["accountIds"] = json!(account_ids);
                }
            }
            entry
        })
        .collect()
}

fn client_api_keys(collection: &CodebuddyLocalAccessCollection) -> Vec<String> {
    collection
        .api_keys
        .iter()
        .filter(|key| key.enabled)
        .map(|key| key.key.clone())
        .collect()
}

fn client_base_url(collection: &CodebuddyLocalAccessCollection) -> String {
    let host = if collection.bind_host.is_empty() {
        "127.0.0.1"
    } else {
        collection.bind_host.as_str()
    };
    // LAN 模式下本机访问仍走 127.0.0.1（避免误导用户复制 0.0.0.0）
    let display_host = if host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host
    };
    format!("http://{}:{}", display_host, collection.port)
}

/// 局域网模式下供外部设备使用的 URL。
/// 解析本机非环回、非虚拟网卡的 IPv4 地址，返回形如 `http://192.168.1.10:11435` 的字符串。
/// 若解析失败则返回 None。
fn lan_base_url(collection: &CodebuddyLocalAccessCollection) -> Option<String> {
    if !matches!(collection.scope, CodebuddyLocalAccessScope::Lan) {
        return None;
    }
    let lan_ip = resolve_lan_ipv4()?;
    Some(format!("http://{}:{}", lan_ip, collection.port))
}

/// 选择本机局域网 IPv4：优先非虚拟网卡、非环回、非链路本地地址。
fn resolve_lan_ipv4() -> Option<String> {
    use std::net::IpAddr;

    // local_ip_address::local_ip() 已经筛选出用于外部网络通信的本机 IP
    let ip = local_ip_address::local_ip().ok()?;
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return None;
            }
            let octets = v4.octets();
            // 链路本地 169.254.x.x
            if octets[0] == 169 && octets[1] == 254 {
                return None;
            }
            Some(v4.to_string())
        }
        IpAddr::V6(_) => None,
    }
}

async fn start_sidecar_inner(collection: &CodebuddyLocalAccessCollection) -> Result<SidecarLaunchConfig, String> {
    let base_dir = sidecar_base_dir()?;
    let launch_config = prepare_sidecar_launch_config(collection, &base_dir)?;

    let binary = sidecar_binary_path()?;
    let mut command = TokioCommand::new(&binary);
    command
        .arg("--config")
        .arg(&launch_config.config_path)
        .arg("--manifest")
        .arg(&launch_config.manifest_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(&base_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("启动 CodeBuddy 反代 sidecar 失败: {e}"))?;

    let stdout = child.stdout.take();
    {
        let mut runtime = gateway_runtime().lock().await;
        // 注意：此处【不清空 request_logs】。request_logs 是历史统计数据的
        // 唯一来源（用量统计/按账号统计均由此累计），sidecar 因配置变更
        // （调度策略/路由规则/模型映射等）重启时统计必须保留，只有用户
        // 主动点击「清除统计」（codebuddy_local_access_clear_stats）才能清零。
        //
        // 仅清空与「旧 sidecar 进程」绑定的 requestId 映射表：
        //  - request_index：旧进程 requestId → request_logs 索引，usage 事件回填用
        //  - request_account / request_stream_end：旧进程 requestId → 账号/流状态
        // 这些映射随旧进程退出而永久失效，新进程会产生全新 requestId。
        // （账号健康状态 account_health 亦跨重启保留。）
        runtime.request_index.clear();
        runtime.request_account.clear();
        runtime.request_stream_end.clear();
        runtime.child = Some(child);
        runtime.actual_port = Some(collection.port);
        runtime.fingerprint = Some(launch_config.fingerprint.clone());
        runtime.running = true;
        runtime.last_error = None;
    }

    if let Some(stdout) = stdout {
        tokio::spawn(read_sidecar_stdout(stdout));
    }

    Ok(launch_config)
}

async fn read_sidecar_stdout(stdout: tokio::process::ChildStdout) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            ingest_sidecar_log_event(&value).await;
        }
    }
}

fn event_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn request_kind_from_str(raw: &str) -> CodebuddyLocalAccessRequestKind {
    match raw.trim() {
        "text" => CodebuddyLocalAccessRequestKind::Text,
        "image_generation" => CodebuddyLocalAccessRequestKind::ImageGeneration,
        "image_edit" => CodebuddyLocalAccessRequestKind::ImageEdit,
        _ => CodebuddyLocalAccessRequestKind::Other,
    }
}

/// 失败分类优先级：数值越小优先级越高。
/// client_canceled > stream_incomplete > upstream_response_failed。
fn error_category_priority(category: &str) -> i32 {
    match category {
        CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED => 0,
        CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE => 1,
        CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED => 2,
        _ => 3,
    }
}

/// 将失败分类应用到日志条目（仅在优先级更高时覆盖），并同步 success 语义：
/// 被分类的请求（取消 / 流未完成 / 上游失败）计入失败而非成功。
fn apply_error_category(log: &mut CodebuddyLocalAccessRequestLog, category: &str) {
    let new_priority = error_category_priority(category);
    let should_apply = match log.error_category.as_deref() {
        Some(existing) => new_priority < error_category_priority(existing),
        None => true,
    };
    if should_apply {
        log.error_category = Some(category.to_string());
        log.success = false;
    }
}

/// stream_completed 的 reason → 失败分类。
/// "done" 为正常结束；"client_gone" 为客户端断开；其余（idle 超时 / 写失败 / 流错误）
/// 归为流未完成。
fn stream_end_category(reason: &str) -> Option<&'static str> {
    match reason.trim() {
        "client_gone" => Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED),
        "stream_idle_timeout" | "write_failed" | "stream_error" => {
            Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE)
        }
        _ => None,
    }
}

/// 将 auth_result / stream_completed 携带的账号归因与流结束状态回填到已存在的日志。
fn backfill_log_context(runtime: &mut CodebuddyGatewayRuntime, request_id: &str) {
    let Some(idx) = runtime.request_index.get(request_id).copied() else {
        return;
    };
    let Some(log) = runtime.request_logs.get_mut(idx) else {
        return;
    };
    if let Some(account_id) = runtime.request_account.get(request_id) {
        if !account_id.is_empty() && log.account_id.is_empty() {
            log.account_id = account_id.clone();
        }
    }
}

/// 将 auth_result 事件应用到账号健康状态（纯函数，便于单测）。
///
/// 成功：连续失败清零、恢复可用。
/// 失败：连续失败 +1、记录失败时间与分类；nextRetryAtMs 在未来时登记冷却；
/// authAvailable 已知时同步可用性。
fn apply_auth_result_to_health(
    state: &mut AccountHealthState,
    success: bool,
    error_code: &str,
    next_retry_at_ms: i64,
    auth_available: Option<bool>,
    model: &str,
    now_ms: i64,
) {
    if success {
        state.consecutive_failures = 0;
        state.available = true;
        return;
    }
    state.consecutive_failures = state.consecutive_failures.saturating_add(1);
    state.last_failure_at = Some(now_ms);
    if !error_code.is_empty() {
        state.last_failure_category = Some(error_code.to_string());
    }
    if next_retry_at_ms > now_ms {
        let cooldown = CodebuddyLocalAccessAccountCooldown {
            model_id: model.to_string(),
            next_retry_at: next_retry_at_ms,
            remaining_ms: next_retry_at_ms - now_ms,
            reason: if error_code.is_empty() {
                "upstream_failure".to_string()
            } else {
                error_code.to_string()
            },
        };
        // 去重：同模型旧冷却直接替换。
        state
            .cooldowns
            .retain(|item| item.model_id != cooldown.model_id);
        state.cooldowns.push(cooldown);
    }
    if let Some(available) = auth_available {
        state.available = available;
    }
}

async fn ingest_sidecar_log_event(value: &Value) {
    let event_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "request_completed" => {
            let request_id = event_str(value, "requestId");
            if request_id.is_empty() {
                return;
            }
            let status = value.get("status").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            let aborted = value
                .get("aborted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut log_entry = CodebuddyLocalAccessRequestLog {
                request_id: request_id.clone(),
                timestamp: value
                    .get("completedAtMs")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| {
                        value
                            .get("startedAtMs")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                    }),
                model: event_str(value, "model"),
                api_key_id: event_str(value, "apiKeyId"),
                account_id: String::new(),
                status,
                success: status > 0 && status < 400,
                latency_ms: value.get("latencyMs").and_then(|v| v.as_u64()).unwrap_or(0),
                input_tokens: 0,
                output_tokens: 0,
                credit: 0.0,
                prompt_cache_hit_tokens: 0,
                prompt_cache_miss_tokens: 0,
                prompt_cache_write_tokens: 0,
                request_kind: request_kind_from_str(&event_str(value, "requestKind")),
                error_category: None,
                error_message: value
                    .get("errorMessage")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            };

            let mut runtime = gateway_runtime().lock().await;

            // 账号归因：usage 事件尚未到达时，用 auth_selected / auth_result 的映射回填。
            if let Some(account_id) = runtime.request_account.get(&request_id) {
                if !account_id.is_empty() {
                    log_entry.account_id = account_id.clone();
                }
            }

            // 失败分类（优先级：客户端取消 > 流未完成 > 上游失败）。
            if aborted {
                apply_error_category(&mut log_entry, CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED);
            }
            if let Some(reason) = runtime.request_stream_end.remove(&request_id) {
                if let Some(category) = stream_end_category(&reason) {
                    apply_error_category(&mut log_entry, category);
                }
            }
            if log_entry.error_category.is_none() && status >= 400 {
                apply_error_category(
                    &mut log_entry,
                    CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED,
                );
            }

            runtime.request_account.remove(&request_id);
            if runtime.request_account.len() > MAX_EVENT_MAP_ENTRIES {
                runtime.request_account.clear();
            }
            if runtime.request_stream_end.len() > MAX_EVENT_MAP_ENTRIES {
                runtime.request_stream_end.clear();
            }

            let next_idx = runtime.request_logs.len();
            runtime.request_index.insert(request_id, next_idx);
            runtime.request_logs.push(log_entry);
            if runtime.request_logs.len() > MAX_REQUEST_LOGS {
                let removed = runtime.request_logs.len() - MAX_REQUEST_LOGS;
                runtime.request_logs.drain(0..removed);
                let rebuild: Vec<(String, usize)> = runtime
                    .request_logs
                    .iter()
                    .enumerate()
                    .map(|(idx, log)| (log.request_id.clone(), idx))
                    .collect();
                runtime.request_index.clear();
                for (rid, idx) in rebuild {
                    runtime.request_index.insert(rid, idx);
                }
            }
            // 释放 runtime 锁后通知前端刷新统计（避免持锁 emit）。
            drop(runtime);
            emit_stats_changed();
        }
        "usage" => {
            let request_id = event_str(value, "requestId");
            if request_id.is_empty() {
                return;
            }
            let input_tokens = value
                .get("usage")
                .and_then(|v| v.get("tokenBreakdown"))
                .and_then(|v| v.get("input"))
                .and_then(|v| v.get("total_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output_tokens = value
                .get("usage")
                .and_then(|v| v.get("tokenBreakdown"))
                .and_then(|v| v.get("output"))
                .and_then(|v| v.get("total_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let credit = value
                .get("usage")
                .and_then(|v| v.get("credit"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            // prompt cache token 回传（Go 侧 usage.tokenBreakdown.input 下的
            // cache_read_tokens / uncached_tokens / cache_write_tokens）。
            let cache_hit = value
                .get("usage")
                .and_then(|v| v.get("tokenBreakdown"))
                .and_then(|v| v.get("input"))
                .and_then(|v| v.get("cache_read_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cache_miss = value
                .get("usage")
                .and_then(|v| v.get("tokenBreakdown"))
                .and_then(|v| v.get("input"))
                .and_then(|v| v.get("uncached_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let cache_write = value
                .get("usage")
                .and_then(|v| v.get("tokenBreakdown"))
                .and_then(|v| v.get("input"))
                .and_then(|v| v.get("cache_write_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let account_id = event_str(value, "accountId");
            let mut runtime = gateway_runtime().lock().await;
            if let Some(idx) = runtime.request_index.get(&request_id).copied() {
                if let Some(log) = runtime.request_logs.get_mut(idx) {
                    log.input_tokens = input_tokens;
                    log.output_tokens = output_tokens;
                    log.credit = credit;
                    log.prompt_cache_hit_tokens = cache_hit;
                    log.prompt_cache_miss_tokens = cache_miss;
                    log.prompt_cache_write_tokens = cache_write;
                    if !account_id.is_empty() {
                        log.account_id = account_id;
                    }
                }
            }
            // 释放 runtime 锁后通知前端刷新统计。
            drop(runtime);
            emit_stats_changed();
        }
        "auth_selected" => {
            // 路由选定账号：记录 requestId → accountId，供失败请求账号归因。
            let request_id = event_str(value, "requestId");
            let account_id = event_str(value, "accountId");
            if request_id.is_empty() || account_id.is_empty() {
                return;
            }
            let mut runtime = gateway_runtime().lock().await;
            runtime.request_account.insert(request_id, account_id);
            if runtime.request_account.len() > MAX_EVENT_MAP_ENTRIES {
                runtime.request_account.clear();
            }
        }
        "auth_result" => {
            // 上游鉴权 / 响应结果：驱动账号健康跟踪（连续失败 / 冷却 / 可用性）。
            let request_id = event_str(value, "requestId");
            let account_id = event_str(value, "accountId");
            if request_id.is_empty() || account_id.is_empty() {
                return;
            }
            let success = value
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let error_code = event_str(value, "errorCode");
            let next_retry_at_ms = value
                .get("nextRetryAtMs")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let auth_available = value.get("authAvailable").and_then(|v| v.as_bool());
            let model = event_str(value, "model");
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let mut runtime = gateway_runtime().lock().await;
            // 失败重试切换账号时，auth_result 提供最新的账号归因。
            runtime
                .request_account
                .insert(request_id.clone(), account_id.clone());
            if runtime.request_account.len() > MAX_EVENT_MAP_ENTRIES {
                runtime.request_account.clear();
            }
            backfill_log_context(&mut runtime, &request_id);

            let state = runtime
                .account_health
                .entry(account_id)
                .or_default();
            apply_auth_result_to_health(
                state,
                success,
                &error_code,
                next_retry_at_ms,
                auth_available,
                &model,
                now_ms,
            );
        }
        "stream_completed" => {
            // 流结束：reason 标记客户端断开 / 流未完成。
            // 通常先于 request_completed 到达（defer 在中间件 emit 之前执行），
            // 此时日志尚未创建，先暂存；若日志已存在（乱序）则直接回填。
            let request_id = event_str(value, "requestId");
            if request_id.is_empty() {
                return;
            }
            // reason 编码在 errorMessage：`reason=%s received=%d`。
            let reason = event_str(value, "errorMessage")
                .split_whitespace()
                .next()
                .and_then(|token| token.strip_prefix("reason="))
                .unwrap_or("")
                .to_string();

            let mut runtime = gateway_runtime().lock().await;
            if let Some(idx) = runtime.request_index.get(&request_id).copied() {
                if let Some(category) = stream_end_category(&reason) {
                    // 先取账号归因，避免与日志可变借用冲突。
                    let attributed_account = runtime
                        .request_account
                        .get(&request_id)
                        .filter(|id| !id.is_empty())
                        .cloned();
                    if let Some(log) = runtime.request_logs.get_mut(idx) {
                        apply_error_category(log, category);
                        if let Some(account_id) = attributed_account {
                            if log.account_id.is_empty() {
                                log.account_id = account_id;
                            }
                        }
                    }
                }
                runtime.request_stream_end.remove(&request_id);
            } else if !reason.is_empty() {
                runtime.request_stream_end.insert(request_id, reason);
                if runtime.request_stream_end.len() > MAX_EVENT_MAP_ENTRIES {
                    runtime.request_stream_end.clear();
                }
            }
        }
        "codebuddy_debug_body" => {
            // Go 侧 CODEBUDDY_DEBUG_BODY=1 时的脱敏请求/响应体 dump。仅用于诊断，
            // 直接落日志，不进统计。
            let phase = event_str(value, "phase");
            let body = event_str(value, "body");
            if !phase.is_empty() {
                logger::log_info(&format!("[CodeBuddyDebug] {phase}: {body}"));
            }
        }
        _ => {}
    }
}

async fn stop_sidecar_inner() -> Result<(), String> {
    let mut runtime = gateway_runtime().lock().await;
    if let Some(mut child) = runtime.child.take() {
        let _ = child.kill().await;
    }
    runtime.child = None;
    runtime.running = false;
    runtime.actual_port = None;
    runtime.fingerprint = None;
    runtime.last_error = None;
    Ok(())
}

async fn reconcile_sidecar(collection: &CodebuddyLocalAccessCollection) -> Result<(), String> {
    if !collection.enabled {
        return stop_sidecar_inner().await;
    }

    let base_dir = sidecar_base_dir()?;
    let launch_config = prepare_sidecar_launch_config(collection, &base_dir)?;

    let runtime = gateway_runtime().lock().await;
    let needs_restart = runtime.running != true
        || runtime.fingerprint.as_deref() != Some(launch_config.fingerprint.as_str())
        || runtime.actual_port != Some(collection.port);
    drop(runtime);

    if !needs_restart {
        return Ok(());
    }

    stop_sidecar_inner().await?;
    start_sidecar_inner(collection).await?;
    Ok(())
}

async fn restore_on_startup() {
    let collection = load_collection();
    let enabled = collection.enabled;
    {
        let mut runtime = gateway_runtime().lock().await;
        runtime.collection = collection.clone();
        runtime.loaded = true;
    }
    if enabled {
        if let Err(error) = reconcile_sidecar(&collection).await {
            logger::log_error(&format!("[CodeBuddyLocalAccess] 启动时恢复 sidecar 失败: {error}"));
            let mut runtime = gateway_runtime().lock().await;
            runtime.last_error = Some(error);
        }
    }
}

#[tauri::command]
pub async fn codebuddy_local_access_get_state() -> Result<CodebuddyLocalAccessState, String> {
    let mut runtime = gateway_runtime().lock().await;
    if !runtime.loaded {
        runtime.collection = load_collection();
        runtime.loaded = true;
    }
    let collection = runtime.collection.clone();
    let running = runtime.running;
    let actual_port = runtime.actual_port;
    let last_error = runtime.last_error.clone();
    let health_state = runtime.account_health.clone();
    drop(runtime);

    Ok(CodebuddyLocalAccessState {
        base_url: client_base_url(&collection),
        lan_base_url: lan_base_url(&collection),
        collection: collection.clone(),
        running,
        actual_port,
        last_error,
        intl_accounts: list_intl_accounts(),
        cn_accounts: list_cn_accounts(),
        account_health: build_account_health(&collection, &health_state),
    })
}

/// 由运行时健康状态解析可用性与有效冷却列表（纯函数，便于单测）：
/// 修剪已过期冷却项；仍存在未到期冷却时账号暂不可用。
fn resolve_runtime_health(
    state: &AccountHealthState,
    now_ms: i64,
) -> (bool, Vec<CodebuddyLocalAccessAccountCooldown>) {
    let cooldowns: Vec<CodebuddyLocalAccessAccountCooldown> = state
        .cooldowns
        .iter()
        .filter(|item| item.next_retry_at > now_ms)
        .cloned()
        .collect();
    let available = state.available && cooldowns.is_empty();
    (available, cooldowns)
}

/// 构建账号健康状态（图片生成能力 + 运行时调度健康度）。
///
/// 图片生成能力：因 CodeBuddy 上游图片协议未实测，不发起真实上游调用：
/// - 图片生成模式为 Disabled 时，所有账号标记为 Disabled；
/// - 否则标记为 Unknown（待上游图片协议确认后，可接入真实探测）。
///
/// 调度健康度：合并 runtime 中由 auth_result 事件驱动的连续失败 / 冷却 / 可用性
/// 跟踪状态（无数据账号视为可用、连续失败 0）。
fn build_account_health(
    collection: &CodebuddyLocalAccessCollection,
    health_state: &HashMap<String, AccountHealthState>,
) -> Vec<CodebuddyLocalAccessAccountHealth> {
    let status = match collection.image_generation_mode {
        CodebuddyLocalAccessImageGenerationMode::Disabled => {
            CodebuddyLocalAccessImageGenerationStatus::Disabled
        }
        CodebuddyLocalAccessImageGenerationMode::ImagesOnly
        | CodebuddyLocalAccessImageGenerationMode::Enabled => {
            CodebuddyLocalAccessImageGenerationStatus::Unknown
        }
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let build_entry =
        |account: &CodebuddyLocalAccessAccountOption| -> CodebuddyLocalAccessAccountHealth {
            let state = health_state
                .get(&account.id)
                .cloned()
                .unwrap_or_default();
            let (available, cooldowns) = resolve_runtime_health(&state, now_ms);
            CodebuddyLocalAccessAccountHealth {
                account_id: account.id.clone(),
                email: account.email.clone(),
                available,
                consecutive_failures: state.consecutive_failures,
                last_failure_at: state.last_failure_at,
                last_failure_category: state.last_failure_category,
                cooldowns,
                image_generation_status: status,
                image_generation_checked_at: None,
            }
        };

    let mut health: Vec<CodebuddyLocalAccessAccountHealth> = Vec::new();
    let mut seen = HashSet::new();
    for account in list_intl_accounts() {
        if seen.insert(account.id.clone()) {
            health.push(build_entry(&account));
        }
    }
    for account in list_cn_accounts() {
        if seen.insert(account.id.clone()) {
            health.push(build_entry(&account));
        }
    }
    health
}

#[tauri::command]
pub async fn codebuddy_local_access_save_collection(
    collection: CodebuddyLocalAccessCollection,
) -> Result<CodebuddyLocalAccessState, String> {
    save_collection(&collection)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        runtime.collection = collection.clone();
    }
    reconcile_sidecar(&collection).await?;
    codebuddy_local_access_get_state().await
}

#[tauri::command]
pub async fn codebuddy_local_access_set_enabled(enabled: bool) -> Result<CodebuddyLocalAccessState, String> {
    let mut collection = {
        let mut runtime = gateway_runtime().lock().await;
        if !runtime.loaded {
            runtime.collection = load_collection();
            runtime.loaded = true;
        }
        runtime.collection.clone()
    };
    collection.enabled = enabled;
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_start() -> Result<CodebuddyLocalAccessState, String> {
    let mut collection = gateway_runtime().lock().await.collection.clone();
    collection.enabled = true;
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_stop() -> Result<CodebuddyLocalAccessState, String> {
    let mut collection = gateway_runtime().lock().await.collection.clone();
    collection.enabled = false;
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_test() -> Result<String, String> {
    let runtime = gateway_runtime().lock().await;
    if !runtime.running {
        return Err("CodeBuddy 反代服务未运行".to_string());
    }
    let port = runtime.actual_port.ok_or_else(|| "CodeBuddy 反代服务端口未知".to_string())?;
    drop(runtime);

    let url = format!("http://127.0.0.1:{port}/v1/models");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("构建测试客户端失败: {e}"))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let status = response.status();
    if status.is_success() {
        Ok(format!("联通正常 (HTTP {status})"))
    } else {
        Err(format!("服务返回异常状态码: HTTP {status}"))
    }
}

/// 强制回收占用指定端口的进程。
///
/// 用途：当 sidecar 进程异常退出、端口仍被占用导致重启失败时，
/// 前端可调用此命令清理残留进程。
#[tauri::command]
pub async fn codebuddy_local_access_kill_port(port: u16) -> Result<usize, String> {
    let killed = tokio::task::spawn_blocking(move || crate::modules::process::kill_port_processes(port))
        .await
        .map_err(|e| format!("执行 kill_port 任务失败: {e}"))??;
    Ok(killed)
}

fn now_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn merge_usage(target: &mut CodebuddyLocalAccessUsageStats, log: &CodebuddyLocalAccessRequestLog) {
    target.request_count += 1;
    if log.success {
        target.success_count += 1;
    } else {
        target.failure_count += 1;
    }
    // 失败分类子计数（与 failure_count 互斥子集，对齐 Codex 反代语义）。
    match log.error_category.as_deref() {
        Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED) => {
            target.client_canceled_count += 1;
        }
        Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE) => {
            target.stream_incomplete_count += 1;
        }
        Some(CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED) => {
            target.upstream_response_failed_count += 1;
        }
        _ => {}
    }
    target.total_latency_ms += log.latency_ms;
    target.input_tokens += log.input_tokens;
    target.output_tokens += log.output_tokens;
    target.total_tokens += log.input_tokens + log.output_tokens;
    target.total_credit += log.credit;
    target.prompt_cache_hit_tokens += log.prompt_cache_hit_tokens;
    target.prompt_cache_miss_tokens += log.prompt_cache_miss_tokens;
    target.prompt_cache_write_tokens += log.prompt_cache_write_tokens;
    match log.request_kind {
        CodebuddyLocalAccessRequestKind::Text => {
            target.text_request_count += 1;
        }
        CodebuddyLocalAccessRequestKind::ImageGeneration => {
            target.image_request_count += 1;
            target.image_generation_request_count += 1;
            if !log.success {
                target.image_generation_capability_failure_count += 1;
            }
        }
        CodebuddyLocalAccessRequestKind::ImageEdit => {
            target.image_request_count += 1;
            target.image_edit_request_count += 1;
            if !log.success {
                target.image_generation_capability_failure_count += 1;
            }
        }
        CodebuddyLocalAccessRequestKind::Other => {}
    }
}

/// 调度策略的 snake_case 名称（用于 manifest 展示）。
fn routing_strategy_name(strategy: CodebuddyLocalAccessRoutingStrategy) -> &'static str {
    match strategy {
        CodebuddyLocalAccessRoutingStrategy::Auto => "auto",
        CodebuddyLocalAccessRoutingStrategy::Random => "random",
        CodebuddyLocalAccessRoutingStrategy::SingleAccount => "single_account",
        CodebuddyLocalAccessRoutingStrategy::QuotaHighFirst => "quota_high_first",
        CodebuddyLocalAccessRoutingStrategy::QuotaLowFirst => "quota_low_first",
        CodebuddyLocalAccessRoutingStrategy::PlanHighFirst => "plan_high_first",
        CodebuddyLocalAccessRoutingStrategy::PlanLowFirst => "plan_low_first",
        CodebuddyLocalAccessRoutingStrategy::ExpirySoonFirst => "expiry_soon_first",
        CodebuddyLocalAccessRoutingStrategy::Custom => "custom",
    }
}

fn build_stats(runtime: &CodebuddyGatewayRuntime) -> CodebuddyLocalAccessStats {
    // stats_since 以秒存储；输出毫秒以匹配前端时间显示（new Date(ms)）。
    // 首次启动（尚未清空统计）时为 0，回退到当前时间，保证前端可显示统计起点。
    let since_ms = if runtime.stats_since > 0 {
        runtime.stats_since.saturating_mul(1000)
    } else {
        now_epoch_secs().saturating_mul(1000)
    };
    let mut stats = CodebuddyLocalAccessStats {
        since: since_ms,
        ..Default::default()
    };
    let mut by_model: HashMap<String, CodebuddyLocalAccessUsageStats> = HashMap::new();
    let mut by_api_key: HashMap<String, CodebuddyLocalAccessUsageStats> = HashMap::new();
    let mut by_account: HashMap<String, CodebuddyLocalAccessUsageStats> = HashMap::new();

    for log in &runtime.request_logs {
        merge_usage(&mut stats.totals, log);
        let model_entry = by_model.entry(log.model.clone()).or_default();
        merge_usage(model_entry, log);
        let key_entry = by_api_key.entry(log.api_key_id.clone()).or_default();
        merge_usage(key_entry, log);
        if !log.account_id.is_empty() {
            let account_entry = by_account.entry(log.account_id.clone()).or_default();
            merge_usage(account_entry, log);
        }
    }

    let mut models: Vec<_> = by_model
        .into_iter()
        .map(|(model_id, usage)| crate::models::codebuddy_local_access::CodebuddyLocalAccessModelStats {
            model_id,
            usage,
        })
        .collect();
    models.sort_by(|a, b| b.usage.request_count.cmp(&a.usage.request_count));

    let mut keys: Vec<_> = by_api_key
        .into_iter()
        .map(|(api_key_id, usage)| crate::models::codebuddy_local_access::CodebuddyLocalAccessApiKeyStats {
            api_key_id,
            usage,
        })
        .collect();
    keys.sort_by(|a, b| b.usage.request_count.cmp(&a.usage.request_count));

    let mut accounts: Vec<_> = by_account
        .into_iter()
        .map(|(account_id, usage)| crate::models::codebuddy_local_access::CodebuddyLocalAccessAccountStats {
            account_id,
            usage,
        })
        .collect();
    accounts.sort_by(|a, b| b.usage.request_count.cmp(&a.usage.request_count));

    stats.by_model = models;
    stats.by_api_key = keys;
    stats.by_account = accounts;
    stats.recent_logs = runtime.request_logs.iter().rev().take(200).cloned().collect();
    stats
}

#[tauri::command]
pub async fn codebuddy_local_access_get_stats() -> Result<CodebuddyLocalAccessStats, String> {
    let runtime = gateway_runtime().lock().await;
    Ok(build_stats(&runtime))
}

#[tauri::command]
pub async fn codebuddy_local_access_clear_stats() -> Result<CodebuddyLocalAccessStats, String> {
    let mut runtime = gateway_runtime().lock().await;
    runtime.request_logs.clear();
    runtime.request_index.clear();
    runtime.stats_since = now_epoch_secs();
    Ok(build_stats(&runtime))
}

#[tauri::command]
pub async fn codebuddy_local_access_get_logs(
    page: u32,
    page_size: u32,
    model_filter: Option<String>,
    api_key_filter: Option<String>,
    success_filter: Option<bool>,
) -> Result<Value, String> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 200);
    let runtime = gateway_runtime().lock().await;

    let filtered: Vec<&CodebuddyLocalAccessRequestLog> = runtime
        .request_logs
        .iter()
        .filter(|log| {
            if let Some(model) = &model_filter {
                if !model.is_empty() && !log.model.contains(model) {
                    return false;
                }
            }
            if let Some(key) = &api_key_filter {
                if !key.is_empty() && !log.api_key_id.contains(key) {
                    return false;
                }
            }
            if let Some(success) = success_filter {
                if log.success != success {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len() as u64;
    let total_pages = if total == 0 {
        1
    } else {
        ((total + page_size as u64 - 1) / page_size as u64)
    };
    let start = ((page - 1) as usize * page_size as usize).min(filtered.len());
    let end = (start + page_size as usize).min(filtered.len());
    let logs: Vec<&CodebuddyLocalAccessRequestLog> = filtered[start..end].to_vec();

    Ok(json!({
        "logs": logs,
        "total": total,
        "page": page,
        "pageSize": page_size,
        "totalPages": total_pages,
    }))
}

#[tauri::command]
pub async fn codebuddy_local_access_create_api_key(
    name: String,
    account_ids: Option<Vec<String>>,
) -> Result<CodebuddyLocalAccessState, String> {
    let now = now_epoch_secs();
    let key = CodebuddyLocalAccessApiKey {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        key: format!("sk-cockpit-{}", uuid::Uuid::new_v4().simple()),
        enabled: true,
        account_ids,
        created_at: now,
        updated_at: now,
    };
    let collection = {
        let mut runtime = gateway_runtime().lock().await;
        runtime.collection.api_keys.push(key);
        runtime.collection.clone()
    };
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_update_api_key(
    id: String,
    name: Option<String>,
    enabled: Option<bool>,
    account_ids: Option<Vec<String>>,
) -> Result<CodebuddyLocalAccessState, String> {
    let collection = {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_epoch_secs();
        let mut updated = false;
        for key in runtime.collection.api_keys.iter_mut() {
            if key.id == id {
                if let Some(name) = &name {
                    key.name = name.clone();
                }
                if let Some(enabled) = enabled {
                    key.enabled = enabled;
                }
                if let Some(account_ids) = &account_ids {
                    key.account_ids = Some(account_ids.clone());
                }
                key.updated_at = now;
                updated = true;
                break;
            }
        }
        if !updated {
            return Err(format!("未找到 Key: {id}"));
        }
        runtime.collection.clone()
    };
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_rotate_api_key(
    id: String,
) -> Result<CodebuddyLocalAccessState, String> {
    let collection = {
        let mut runtime = gateway_runtime().lock().await;
        let now = now_epoch_secs();
        let mut rotated = false;
        for key in runtime.collection.api_keys.iter_mut() {
            if key.id == id {
                key.key = format!("sk-cockpit-{}", uuid::Uuid::new_v4().simple());
                key.updated_at = now;
                rotated = true;
                break;
            }
        }
        if !rotated {
            return Err(format!("未找到 Key: {id}"));
        }
        runtime.collection.clone()
    };
    codebuddy_local_access_save_collection(collection).await
}

#[tauri::command]
pub async fn codebuddy_local_access_delete_api_key(
    id: String,
) -> Result<CodebuddyLocalAccessState, String> {
    let collection = {
        let mut runtime = gateway_runtime().lock().await;
        let before = runtime.collection.api_keys.len();
        runtime.collection.api_keys.retain(|key| key.id != id);
        if runtime.collection.api_keys.len() == before {
            return Err(format!("未找到 Key: {id}"));
        }
        runtime.collection.clone()
    };
    codebuddy_local_access_save_collection(collection).await
}

/// 通过本地网关发起一轮对话测试（非流式）。
#[tauri::command]
pub async fn codebuddy_local_access_chat_test(
    model: String,
    messages: Vec<Value>,
) -> Result<Value, String> {
    let (running, port) = {
        let runtime = gateway_runtime().lock().await;
        (runtime.running, runtime.actual_port)
    };
    if !running {
        return Err("CodeBuddy 反代服务未运行".to_string());
    }
    let port = port.ok_or_else(|| "CodeBuddy 反代服务端口未知".to_string())?;

    let api_key = {
        let runtime = gateway_runtime().lock().await;
        runtime
            .collection
            .api_keys
            .iter()
            .find(|k| k.enabled)
            .map(|k| k.key.clone())
            .unwrap_or_default()
    };

    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("构建测试客户端失败: {e}"))?;
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 {url} 失败: {e}"))?;
    let status = response.status();
    let payload = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let value: Value = serde_json::from_str(&payload).unwrap_or_else(|_| json!({ "raw": payload }));
    if status.is_success() {
        Ok(value)
    } else {
        Err(format!("服务返回异常 (HTTP {status}): {payload}"))
    }
}

/// 应用启动时调用，恢复反代服务状态。
pub async fn restore_codebuddy_local_access() {
    restore_on_startup().await;
}

/// 应用退出时停止 sidecar（sidecar 亦会因 parent-pid 失效而退出）。
pub async fn shutdown_codebuddy_local_access() {
    let _ = stop_sidecar_inner().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::codebuddy::CodebuddyAccount;

    fn sample_account() -> CodebuddyAccount {
        CodebuddyAccount {
            id: "acc-123".to_string(),
            email: "user@example.com".to_string(),
            uid: Some("uid-456".to_string()),
            nickname: None,
            enterprise_id: Some("ent-789".to_string()),
            enterprise_name: None,
            tags: None,
            access_token: "at-secret".to_string(),
            refresh_token: Some("rt-secret".to_string()),
            token_type: None,
            expires_at: None,
            domain: Some("www.codebuddy.cn".to_string()),
            plan_type: Some("pro".to_string()),
            dosage_notify_code: None,
            dosage_notify_zh: None,
            dosage_notify_en: None,
            payment_type: None,
            quota_raw: None,
            auth_raw: None,
            profile_raw: None,
            usage_raw: None,
            status: None,
            status_reason: None,
            last_checkin_time: None,
            checkin_streak: 0,
            checkin_rewards: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn test_sidecar_auth_file_name_sanitizes() {
        assert_eq!(sidecar_auth_file_name("acc-123"), "acc-123.json");
        assert_eq!(sidecar_auth_file_name("a b/c"), "a_b_c.json");
        assert!(sidecar_auth_file_name("").ends_with(".json"));
    }

    #[test]
    fn test_auth_json_for_account_intl() {
        let account = sample_account();
        let value = auth_json_for_account(&account, REGION_INTL, &[]);
        assert_eq!(value["type"].as_str(), Some("codebuddy"));
        assert_eq!(value["access_token"].as_str(), Some("at-secret"));
        assert_eq!(value["refresh_token"].as_str(), Some("rt-secret"));
        assert_eq!(value["region"].as_str(), Some(REGION_INTL));
        assert_eq!(value["base_url"].as_str(), Some(BASE_URL_INTL));
        assert_eq!(value["uid"].as_str(), Some("uid-456"));
        assert_eq!(value["enterprise_id"].as_str(), Some("ent-789"));
        assert_eq!(value["domain"].as_str(), Some("www.codebuddy.cn"));
    }

    #[test]
    fn test_auth_json_for_account_cn() {
        let account = sample_account();
        let value = auth_json_for_account(&account, REGION_CN, &[]);
        assert_eq!(value["base_url"].as_str(), Some(BASE_URL_CN));
        assert_eq!(value["region"].as_str(), Some(REGION_CN));
    }

    #[test]
    fn test_auth_json_injects_custom_routing_metadata() {
        let account = sample_account();
        let rules = vec![CodebuddyLocalAccessCustomRoutingRule {
            account_id: account.id.clone(),
            priority: 7,
            weight: 3,
            is_backup: false,
            is_preferred: true,
        }];
        let value = auth_json_for_account(&account, REGION_CN, &rules);
        assert_eq!(value["routing_priority"], json!(7));
        assert_eq!(value["routing_weight"], json!(3));
        assert_eq!(value["routing_is_backup"], json!(false));
        assert_eq!(value["routing_is_preferred"], json!(true));
    }

    #[test]
    fn test_config_fingerprint_deterministic() {
        let a = config_fingerprint("config-a", "manifest-a");
        let b = config_fingerprint("config-a", "manifest-a");
        let c = config_fingerprint("config-b", "manifest-a");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_effective_bind_host() {
        let mut collection = CodebuddyLocalAccessCollection::default();
        collection.scope = CodebuddyLocalAccessScope::Localhost;
        collection.bind_host = "127.0.0.1".to_string();
        assert_eq!(effective_bind_host(&collection), "127.0.0.1");

        collection.scope = CodebuddyLocalAccessScope::Lan;
        assert_eq!(effective_bind_host(&collection), "0.0.0.0");

        collection.scope = CodebuddyLocalAccessScope::Localhost;
        collection.bind_host = "".to_string();
        assert_eq!(effective_bind_host(&collection), "127.0.0.1");
    }

    fn sample_api_key(enabled: bool) -> CodebuddyLocalAccessApiKey {
        CodebuddyLocalAccessApiKey {
            id: "key-1".to_string(),
            name: "codex-cli".to_string(),
            key: "sk-cockpit-abc".to_string(),
            enabled,
            account_ids: Some(vec!["acc-1".to_string()]),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn test_manifest_api_keys_maps_go_fields() {
        let mut collection = CodebuddyLocalAccessCollection::default();
        collection.api_keys = vec![sample_api_key(true)];
        let keys = manifest_api_keys(&collection);
        assert_eq!(keys.len(), 1);
        let entry = &keys[0];
        assert_eq!(entry["id"].as_str(), Some("key-1"));
        assert_eq!(entry["label"].as_str(), Some("codex-cli"));
        assert_eq!(entry["key"].as_str(), Some("sk-cockpit-abc"));
        assert_eq!(entry["enabled"].as_bool(), Some(true));
        assert_eq!(entry["accountIds"][0].as_str(), Some("acc-1"));
    }

    #[test]
    fn test_client_api_keys_filters_disabled() {
        let mut collection = CodebuddyLocalAccessCollection::default();
        collection.api_keys = vec![sample_api_key(true), sample_api_key(false)];
        let keys = client_api_keys(&collection);
        assert_eq!(keys, vec!["sk-cockpit-abc".to_string()]);
    }

    fn sample_log(success: bool, model: &str) -> CodebuddyLocalAccessRequestLog {
        CodebuddyLocalAccessRequestLog {
            request_id: format!("req-{model}-{success}"),
            timestamp: 0,
            model: model.to_string(),
            api_key_id: "key-1".to_string(),
            account_id: "acc-1".to_string(),
            status: if success { 200 } else { 503 },
            success,
            latency_ms: 100,
            input_tokens: 10,
            output_tokens: 20,
            credit: 0.5,
            prompt_cache_hit_tokens: 6,
            prompt_cache_miss_tokens: 4,
            prompt_cache_write_tokens: 0,
            request_kind: CodebuddyLocalAccessRequestKind::Other,
            error_category: None,
            error_message: None,
        }
    }

    #[test]
    fn test_merge_usage_accumulates() {
        let mut target = CodebuddyLocalAccessUsageStats::default();
        merge_usage(&mut target, &sample_log(true, "auto"));
        merge_usage(&mut target, &sample_log(false, "auto"));
        assert_eq!(target.request_count, 2);
        assert_eq!(target.success_count, 1);
        assert_eq!(target.failure_count, 1);
        assert_eq!(target.total_latency_ms, 200);
        assert_eq!(target.input_tokens, 20);
        assert_eq!(target.output_tokens, 40);
        assert_eq!(target.total_tokens, 60);
        assert!((target.total_credit - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_merge_usage_counts_error_categories() {
        let mut target = CodebuddyLocalAccessUsageStats::default();
        let mut canceled = sample_log(false, "auto");
        canceled.error_category = Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED.to_string());
        let mut incomplete = sample_log(false, "auto");
        incomplete.error_category = Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE.to_string());
        let mut upstream = sample_log(false, "auto");
        upstream.error_category = Some(CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED.to_string());
        let plain_failure = sample_log(false, "auto");
        let success = sample_log(true, "auto");
        for log in [&canceled, &incomplete, &upstream, &plain_failure, &success] {
            merge_usage(&mut target, log);
        }
        assert_eq!(target.request_count, 5);
        assert_eq!(target.failure_count, 4);
        assert_eq!(target.success_count, 1);
        assert_eq!(target.client_canceled_count, 1);
        assert_eq!(target.stream_incomplete_count, 1);
        assert_eq!(target.upstream_response_failed_count, 1);
    }

    #[test]
    fn test_apply_error_category_priority() {
        let mut log = sample_log(true, "auto");
        apply_error_category(&mut log, CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED);
        assert_eq!(
            log.error_category.as_deref(),
            Some(CODEBUDDY_ERROR_CATEGORY_UPSTREAM_FAILED)
        );
        assert!(!log.success);

        // 低优先级不覆盖高优先级。
        apply_error_category(&mut log, CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED);
        assert_eq!(
            log.error_category.as_deref(),
            Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED)
        );
        // 高优先级已存在，低优先级不覆盖。
        apply_error_category(&mut log, CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE);
        assert_eq!(
            log.error_category.as_deref(),
            Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED)
        );
    }

    #[test]
    fn test_stream_end_category_mapping() {
        assert_eq!(
            stream_end_category("client_gone"),
            Some(CODEBUDDY_ERROR_CATEGORY_CLIENT_CANCELED)
        );
        assert_eq!(
            stream_end_category("stream_idle_timeout"),
            Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE)
        );
        assert_eq!(
            stream_end_category("write_failed"),
            Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE)
        );
        assert_eq!(
            stream_end_category("stream_error"),
            Some(CODEBUDDY_ERROR_CATEGORY_STREAM_INCOMPLETE)
        );
        assert_eq!(stream_end_category("done"), None);
        assert_eq!(stream_end_category(""), None);
    }

    #[test]
    fn test_apply_auth_result_tracks_failures_and_cooldowns() {
        let now_ms: i64 = 1_000_000;
        let mut state = AccountHealthState::default();
        assert!(state.available);

        // 失败：连续失败 +1，登记冷却，authAvailable=false 生效。
        apply_auth_result_to_health(
            &mut state,
            false,
            "upstream_timeout",
            now_ms + 30_000,
            Some(false),
            "auto",
            now_ms,
        );
        assert_eq!(state.consecutive_failures, 1);
        assert_eq!(state.last_failure_category.as_deref(), Some("upstream_timeout"));
        assert_eq!(state.cooldowns.len(), 1);
        assert_eq!(state.cooldowns[0].model_id, "auto");
        assert_eq!(state.cooldowns[0].reason, "upstream_timeout");
        assert!(!state.available);

        // 再次失败（同模型）：冷却去重替换。
        apply_auth_result_to_health(
            &mut state,
            false,
            "rate_limited",
            now_ms + 60_000,
            None,
            "auto",
            now_ms,
        );
        assert_eq!(state.consecutive_failures, 2);
        assert_eq!(state.cooldowns.len(), 1);
        assert_eq!(state.cooldowns[0].reason, "rate_limited");

        // 成功：连续失败清零、恢复可用（冷却保留至过期）。
        apply_auth_result_to_health(
            &mut state,
            true,
            "",
            0,
            None,
            "auto",
            now_ms,
        );
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.available);
        assert_eq!(state.cooldowns.len(), 1);
    }

    #[test]
    fn test_build_account_health_prunes_expired_cooldowns() {
        let now_ms: i64 = 1_000_000;
        let mut state = AccountHealthState::default();
        state.cooldowns.push(CodebuddyLocalAccessAccountCooldown {
            model_id: "auto".to_string(),
            next_retry_at: now_ms - 1_000, // 已过期
            remaining_ms: 0,
            reason: "old".to_string(),
        });
        state.cooldowns.push(CodebuddyLocalAccessAccountCooldown {
            model_id: "hy3".to_string(),
            next_retry_at: now_ms + 30_000, // 仍有效
            remaining_ms: 30_000,
            reason: "rate_limited".to_string(),
        });

        let (available, cooldowns) = resolve_runtime_health(&state, now_ms);
        // 有效冷却未清空 → 暂不可用；过期冷却被修剪。
        assert!(!available);
        assert_eq!(cooldowns.len(), 1);
        assert_eq!(cooldowns[0].model_id, "hy3");

        // 冷却全部过期后恢复可用。
        let (available, cooldowns) = resolve_runtime_health(&state, now_ms + 60_000);
        assert!(available);
        assert!(cooldowns.is_empty());

        // 无事件数据的账号默认可用。
        let (available, cooldowns) =
            resolve_runtime_health(&AccountHealthState::default(), now_ms);
        assert!(available);
        assert!(cooldowns.is_empty());
    }

    #[test]
    fn test_manifest_api_keys_includes_new_flags() {
        let mut collection = CodebuddyLocalAccessCollection::default();
        collection.api_keys = vec![sample_api_key(true)];
        collection.immediate_sse_response = true;
        collection.responses_websockets_enabled = true;
        let keys = manifest_api_keys(&collection);
        assert_eq!(keys[0]["responsesWebsockets"].as_bool(), Some(true));
    }

    #[test]
    fn test_vision_mode_derives_from_switch() {
        // 开关开 → agentic；开关关 → off。
        // 注意：此测试假设环境变量 CODEBUDDY_VISION_MODE 未设置。
        // 若 CI 环境设置了该变量，测试仍会因显式覆盖而通过（不脆断），
        // 但为确定性起见在断言前临时清除。
        unsafe {
            std::env::remove_var("CODEBUDDY_VISION_MODE");
        }
        assert_eq!(codebuddy_vision_mode(true), "agentic");
        assert_eq!(codebuddy_vision_mode(false), "off");
    }

    #[test]
    fn test_vision_mode_env_override() {
        unsafe {
            std::env::set_var("CODEBUDDY_VISION_MODE", "preprocess");
        }
        // 环境变量显式设置时优先于开关。
        assert_eq!(codebuddy_vision_mode(true), "preprocess");
        assert_eq!(codebuddy_vision_mode(false), "preprocess");
        unsafe {
            std::env::remove_var("CODEBUDDY_VISION_MODE");
        }
    }

    #[test]
    fn test_vision_max_rounds_default() {
        unsafe {
            std::env::remove_var("CODEBUDDY_VISION_MAX_ROUNDS");
        }
        assert_eq!(codebuddy_vision_max_rounds(), 3);
    }

    #[test]
    fn test_build_stats_groups_by_model() {
        let mut runtime = CodebuddyGatewayRuntime::default();
        runtime.request_logs = vec![
            sample_log(true, "auto"),
            sample_log(true, "auto"),
            sample_log(false, "glm-5.2"),
        ];
        let stats = build_stats(&runtime);
        assert_eq!(stats.totals.request_count, 3);
        assert_eq!(stats.by_model.len(), 2);
        let auto_entry = stats
            .by_model
            .iter()
            .find(|m| m.model_id == "auto")
            .expect("auto model group");
        assert_eq!(auto_entry.usage.request_count, 2);
        let glm_entry = stats
            .by_model
            .iter()
            .find(|m| m.model_id == "glm-5.2")
            .expect("glm model group");
        assert_eq!(glm_entry.usage.failure_count, 1);
    }

    #[test]
    fn test_build_stats_groups_by_account() {
        let mut runtime = CodebuddyGatewayRuntime::default();
        let mut log_a = sample_log(true, "auto");
        log_a.account_id = "acc-a".to_string();
        let mut log_b1 = sample_log(true, "auto");
        log_b1.account_id = "acc-b".to_string();
        let mut log_b2 = sample_log(false, "glm-5.2");
        log_b2.account_id = "acc-b".to_string();
        runtime.request_logs = vec![log_a, log_b1, log_b2];

        let stats = build_stats(&runtime);
        assert_eq!(stats.by_account.len(), 2);

        let acc_a = stats
            .by_account
            .iter()
            .find(|a| a.account_id == "acc-a")
            .expect("acc-a group");
        assert_eq!(acc_a.usage.request_count, 1);

        let acc_b = stats
            .by_account
            .iter()
            .find(|a| a.account_id == "acc-b")
            .expect("acc-b group");
        assert_eq!(acc_b.usage.request_count, 2);
        assert_eq!(acc_b.usage.prompt_cache_hit_tokens, 12);
    }
}
